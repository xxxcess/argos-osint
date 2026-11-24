//! Authenticated Inbox QA admission and reads over private Embedded IPC.

mod metrics;
mod question;
mod readiness;

pub use readiness::validated_summary_finding;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use argos_osint_core::storage::inbox_qa as store;
use argos_osint_core::storage::repo::Repository;
use argos_osint_core::storage::tx::TxError;
use axum::http::StatusCode;
use fleetworks_contracts::chat::{
    ChatSourceKind, ChatSourceReference, QaAvailabilityReason, QaCancelRequest,
    QaConversationReply, QaEventPage, QaEventQuery, QaModelTarget, QaPhase, QaSettings,
    QaSettingsUpdate, QaStartupStage, QaTurn, QaTurnNotice, QaTurnRequest, QA_MAX_FINDINGS,
    QA_MAX_WORDS,
};
use fleetworks_contracts::context::{CredentialPurpose, MutationContext, Permission};
use fleetworks_contracts::qa_worker::{
    QaDialogueKind, QaPassage, QaWorkerDialogue, QaWorkerEvent, QaWorkerRequest,
};
use fleetworks_contracts::readiness::{ReadinessCheckId, ReadinessState};
use futures_util::StreamExt;
use tokio::sync::{mpsc, watch, Semaphore};
use tokio::time::Instant;

use self::question::Question;
use crate::actor_search::SearchClient;
use crate::host_metrics::SharedSampler;
use crate::request_context::{ContextFailure, ResolvedRequest};
use crate::workstation::{self, ReadinessOwners};
use crate::{ApiError, ApiState};

pub(crate) const FEATURE: &str = "inbox-qa-v2";

/// Cancellation remains observable to subscribers that join after it was raised.
#[derive(Clone)]
pub struct CancellationToken(watch::Sender<bool>);

impl CancellationToken {
    #[must_use]
    pub fn new() -> Self {
        Self(watch::channel(false).0)
    }

    pub fn cancel(&self) {
        self.0.send_replace(true);
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        *self.0.borrow()
    }

    pub async fn cancelled(&self) {
        let _ = self.0.subscribe().wait_for(|canceled| *canceled).await;
    }
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

/// The selected Embedded companion's redacted startup observation.
#[derive(Clone)]
pub struct QaRuntimeAvailability(watch::Sender<Option<GenerationAvailability>>);

#[derive(Clone, Copy)]
struct GenerationAvailability {
    generation: i64,
    observation: Option<StartupObservation>,
    model: Option<QaModelTarget>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StartupObservation {
    Progress(QaStartupStage),
    Failure(QaAvailabilityReason),
}

impl Default for QaRuntimeAvailability {
    fn default() -> Self {
        Self(watch::channel(None).0)
    }
}

impl QaRuntimeAvailability {
    /// Report a failure or clear it after successful startup for an enablement generation.
    pub fn record(&self, generation: i64, reason: Option<QaAvailabilityReason>) {
        self.publish(generation, reason.map(StartupObservation::Failure));
    }

    /// Report progress of the current supervised worker startup attempt.
    pub fn record_stage(&self, generation: i64, stage: QaStartupStage) {
        self.publish(generation, Some(StartupObservation::Progress(stage)));
    }

    /// Clear abandoned progress while preserving the latest startup diagnosis.
    pub fn clear_stage(&self, generation: i64) {
        self.0.send_modify(|current| {
            if let Some(current) = current.as_mut() {
                if current.generation == generation
                    && matches!(current.observation, Some(StartupObservation::Progress(_)))
                {
                    current.observation = None;
                }
            }
        });
    }

    /// Report the automatically selected model for the current generation.
    pub fn record_model(&self, generation: i64, model: QaModelTarget) {
        self.update(generation, |state| state.model = Some(model));
    }

    fn model(&self, generation: i64) -> Option<QaModelTarget> {
        self.0
            .borrow()
            .filter(|current| current.generation == generation)
            .and_then(|current| current.model)
    }

    fn publish(&self, generation: i64, observation: Option<StartupObservation>) {
        self.update(generation, |state| state.observation = observation);
    }

    fn update(&self, generation: i64, update: impl FnOnce(&mut GenerationAvailability)) {
        self.0.send_modify(|current| {
            if current.is_some_and(|previous| previous.generation > generation) {
                return;
            }
            if current.is_none_or(|previous| previous.generation < generation) {
                *current = Some(GenerationAvailability {
                    generation,
                    observation: None,
                    model: None,
                });
            }
            if let Some(state) = current.as_mut() {
                update(state);
            }
        });
    }

    fn observe(&self, generation: i64) -> Option<StartupObservation> {
        self.0
            .borrow()
            .filter(|current| current.generation == generation)
            .and_then(|current| current.observation)
    }
}

/// Model process operations used by the Embedded executor.
pub type QaInferenceFuture<'a> = Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>>;

/// An executor owns an isolated implementation of this stream.
pub trait QaInference: Send + Sync {
    fn ask(
        &self,
        request: QaWorkerRequest,
        events: mpsc::Sender<QaWorkerEvent>,
        cancel: CancellationToken,
    ) -> QaInferenceFuture<'_>;
    fn shutdown(&self) -> QaInferenceFuture<'_>;
}

/// Every active long poll on a target shares a single backend subscription.
struct EventWake {
    revision: watch::Sender<i64>,
    waiters: watch::Sender<EventWaitCounts>,
    observer: OnceLock<()>,
}

#[derive(Clone, Copy, Default)]
struct EventWaitCounts {
    polls: usize,
    refreshers: usize,
}

impl EventWaitCounts {
    fn active(self) -> bool {
        self.polls > 0 || self.refreshers > 0
    }

    fn counter(&mut self, kind: EventWaitKind) -> &mut usize {
        match kind {
            EventWaitKind::Poll => &mut self.polls,
            EventWaitKind::Refresh => &mut self.refreshers,
        }
    }
}

#[derive(Clone, Copy)]
enum EventWaitKind {
    Poll,
    Refresh,
}

impl EventWake {
    fn new() -> Self {
        Self {
            revision: watch::channel(0).0,
            waiters: watch::channel(EventWaitCounts::default()).0,
            observer: OnceLock::new(),
        }
    }

    fn subscribe(&self, repo: &Repository, kind: EventWaitKind) -> EventWaiter<'_> {
        let revision = self.revision.subscribe();
        self.waiters
            .send_modify(|counts| *counts.counter(kind) += 1);
        self.observer.get_or_init(|| {
            tokio::spawn(observe_events(
                repo.clone(),
                self.waiters.subscribe(),
                self.revision.clone(),
            ));
        });
        EventWaiter {
            wake: self,
            revision,
            kind,
        }
    }
}

struct EventWaiter<'a> {
    wake: &'a EventWake,
    revision: watch::Receiver<i64>,
    kind: EventWaitKind,
}

impl Drop for EventWaiter<'_> {
    fn drop(&mut self) {
        self.wake
            .waiters
            .send_modify(|counts| *counts.counter(self.kind) -= 1);
    }
}

async fn observe_events(
    repo: Repository,
    mut waiters: watch::Receiver<EventWaitCounts>,
    revision: watch::Sender<i64>,
) {
    const FALLBACK: Duration = Duration::from_millis(200);
    const RESAMPLE: Duration = Duration::from_secs(1);
    while waiters.wait_for(|counts| counts.active()).await.is_ok() {
        let mut live = tokio::select! {
            result = tokio::time::timeout(FALLBACK, store::event_changes(&repo)) => result.ok().and_then(Result::ok),
            _ = waiters.wait_for(|counts| !counts.active()) => continue,
        };
        let mut next_sample = Instant::now();
        loop {
            let counts = *waiters.borrow_and_update();
            if !counts.active() {
                break;
            }
            let delay = match (counts.polls > 0, live.is_some()) {
                (true, true) => RESAMPLE,
                (true, false) => FALLBACK,
                (false, true) => Duration::from_secs(5),
                (false, false) => Duration::from_secs(2),
            };
            let notice = async {
                if let Some(stream) = live.as_mut() {
                    stream.next().await
                } else {
                    std::future::pending().await
                }
            };
            tokio::select! {
                changed = waiters.changed() => {
                    if changed.is_err() { return; }
                    next_sample = next_sample.min(Instant::now() + FALLBACK);
                }
                _ = tokio::time::sleep_until(next_sample) => {
                    // Subscribe before the first read; periodically resample a quiet connection.
                    let sample = tokio::time::timeout(FALLBACK, store::event_revision(&repo)).await;
                    if let Ok(Ok(sample)) = sample {
                        wake_at(&revision, sample);
                    }
                    next_sample = Instant::now() + delay;
                }
                notice = notice => match notice {
                    Some(Ok(sample)) => wake_at(&revision, sample),
                    _ => {
                        live = None;
                        next_sample = Instant::now();
                    }
                },
            }
        }
    }
}

fn wake_at(revision: &watch::Sender<i64>, sample: i64) {
    revision.send_if_modified(|current| {
        if sample <= *current {
            return false;
        }
        *current = sample;
        true
    });
}

#[derive(Clone)]
pub(crate) struct QaTarget {
    readiness: Option<ReadinessOwners>,
    search: Option<SearchClient>,
    sampler: Option<SharedSampler>,
    subscribers: Arc<Semaphore>,
    events: Arc<EventWake>,
    probes: Arc<Semaphore>,
}

impl QaTarget {
    pub(crate) fn new(readiness: Option<ReadinessOwners>, search: Option<SearchClient>) -> Self {
        Self {
            readiness,
            search,
            sampler: None,
            subscribers: Arc::new(Semaphore::new(8)),
            events: Arc::new(EventWake::new()),
            probes: Arc::new(Semaphore::new(4)),
        }
    }

    pub(crate) fn with_metrics(mut self, sampler: SharedSampler) -> Self {
        self.sampler = Some(sampler);
        self
    }

    async fn observe(
        &self,
        state: &ApiState,
        generation: i64,
        boundary: i64,
        admission: bool,
    ) -> Result<Option<store::HealthSample>, ApiError> {
        if self.readiness.is_none() {
            return Err(current_health_unavailable());
        }
        let publisher = argos_osint_core::store::new_id();
        match store::claim_health(&state.repo, &publisher, generation, boundary, admission)
            .await
            .map_err(storage_error)?
        {
            store::HealthClaim::Ready(sample) => Ok(Some(sample)),
            store::HealthClaim::Pending => Ok(None),
            store::HealthClaim::Probe(boundary) => {
                // The durable claim outlives a disconnected or timed-out IPC reader.
                let target = self.clone();
                let state = state.clone();
                tokio::spawn(async move {
                    target
                        .probe_health(&state, &publisher, generation, boundary)
                        .await
                })
                .await
                .map_err(|_| current_health_unavailable())?
            }
        }
    }

    async fn probe_health(
        &self,
        state: &ApiState,
        publisher: &str,
        generation: i64,
        boundary: i64,
    ) -> Result<Option<store::HealthSample>, ApiError> {
        let observation = async {
            let readiness = self
                .readiness
                .as_ref()
                .ok_or_else(current_health_unavailable)?;
            let Some((at, facts)) =
                workstation::qa_snapshot(state, readiness, self.search.as_ref(), boundary).await
            else {
                return Ok(None);
            };
            let sample = store::HealthSample {
                sampled_at_ms: at.timestamp_millis(),
                facts: facts
                    .into_iter()
                    .map(|(id, label, status, reason)| store::HealthFact {
                        id: id.to_owned(),
                        label: label.to_owned(),
                        status: status.to_owned(),
                        explanation: String::new(),
                        reason: (!reason.is_empty()).then(|| reason.to_owned()),
                    })
                    .collect(),
            };
            let published = store::put_health(&state.repo, publisher, generation, &sample)
                .await
                .map_err(storage_error)?;
            Ok(published.then_some(sample))
        };
        let result = tokio::time::timeout(Duration::from_millis(3_500), observation)
            .await
            .unwrap_or_else(|_| Err(current_health_unavailable()));
        if !matches!(&result, Ok(Some(_))) {
            let _ = tokio::time::timeout(
                Duration::from_millis(500),
                store::abandon_health(&state.repo, publisher),
            )
            .await;
        }
        result
    }

    pub(crate) async fn refresh(&self, state: &ApiState, stop: &CancellationToken) {
        if self.readiness.is_none() {
            return;
        }
        let mut events = self.events.subscribe(&state.repo, EventWaitKind::Refresh);
        loop {
            let refresh = async {
                if let Ok(setting) = store::setting(&state.repo).await {
                    if setting.enabled {
                        let _ = self.observe(state, setting.generation, 0, false).await;
                    }
                }
            };
            tokio::select! {
                () = stop.cancelled() => break,
                _ = tokio::time::timeout(Duration::from_secs(4), refresh) => {}
            }
            tokio::select! {
                () = stop.cancelled() => break,
                _ = async {
                    // Coalesce provisional deltas. Terminal events invalidate the durable
                    // cache; the fallback also retries a failed refresh on a quiet backend.
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    let _ = tokio::time::timeout(Duration::from_secs(4), events.revision.changed()).await;
                } => {}
            }
        }
    }
}

fn storage_error(error: TxError) -> ApiError {
    if let TxError::Abort(reason) = error {
        match reason.as_str() {
            "inbox_qa:revision" | "inbox_qa:id_conflict" => {
                return ApiError(
                    StatusCode::CONFLICT,
                    "Inbox request or settings revision changed",
                );
            }
            "inbox_qa:busy" | "inbox_qa:active" => {
                return ApiError(StatusCode::TOO_MANY_REQUESTS, "Inbox inference is busy");
            }
            "inbox_qa:missing" => return crate::missing(),
            "inbox_qa:forbidden" => return ContextFailure::forbidden().into(),
            "inbox_qa:off" => {
                return ApiError(StatusCode::SERVICE_UNAVAILABLE, "Backend inference is off");
            }
            "inbox_qa:freshness" => return current_health_unavailable(),
            _ => {}
        }
    }
    ApiError(
        StatusCode::SERVICE_UNAVAILABLE,
        "Backend inference is unavailable",
    )
}

fn settings_storage_error(stage: &'static str, error: TxError) -> ApiError {
    let class = match &error {
        TxError::Db(_) => store::storage_error_kind(&error),
        TxError::Contention { .. } => "contention",
        TxError::Abort(reason)
            if matches!(reason.as_str(), "inbox_qa:forbidden" | "inbox_qa:revision") =>
        {
            return storage_error(error);
        }
        TxError::Abort(_) => "transaction_abort",
        TxError::PublisherStopped => "publisher_stopped",
        TxError::PublisherDropped => "publisher_dropped",
    };
    tracing::warn!(
        qa_settings_stage = stage,
        qa_settings_error = class,
        "Inbox settings storage error"
    );
    storage_error(error)
}

fn current_health_unavailable() -> ApiError {
    ApiError(
        StatusCode::SERVICE_UNAVAILABLE,
        "Current backend health could not be verified",
    )
}

fn require(
    context: &ResolvedRequest,
    permission: Permission,
    mutation: Option<&MutationContext>,
) -> Result<store::Actor, ApiError> {
    let authorized_purpose = context.purpose() == CredentialPurpose::Native
        || (context.purpose() == CredentialPurpose::Operator
            && permission == Permission::ManageInboxQa);
    if !authorized_purpose || !context.permits(permission) {
        return Err(ContextFailure::forbidden().into());
    }
    let registered = context.registered()?;
    if mutation.is_some_and(|expected| {
        expected.registration_id != registered.registration_id
            || expected.expected_registration_revision != registered.registration_revision
            || expected.expected_context_revision != registered.context_revision
    }) {
        return Err(ContextFailure::changed().into());
    }
    Ok(store::Actor::new(
        context.scope_id().to_owned(),
        context.credential_id().to_owned(),
        context.access_epoch().to_owned(),
        registered.clone(),
    ))
}

async fn settings_value(
    state: &ApiState,
    context: &ResolvedRequest,
    setting: store::Setting,
    availability: Option<&QaRuntimeAvailability>,
) -> Result<QaSettings, ApiError> {
    let observation =
        availability.and_then(|availability| availability.observe(setting.generation));
    let (startup_stage, reason) = match observation {
        Some(StartupObservation::Progress(stage)) => (Some(stage), None),
        Some(StartupObservation::Failure(reason)) => (None, Some(reason)),
        None => (None, None),
    };
    let phase = if setting.enabled {
        if store::replica_present(&state.repo, setting.generation)
            .await
            .map_err(storage_error)?
        {
            QaPhase::Ready
        } else if startup_stage.is_some()
            || reason.is_none()
                && chrono::Utc::now().timestamp_millis() - setting.changed_at_ms < 15_000
        {
            QaPhase::Starting
        } else {
            QaPhase::Unavailable
        }
    } else if store::replica_stopping(&state.repo)
        .await
        .map_err(|error| settings_storage_error("observe_stopping", error))?
    {
        QaPhase::Stopping
    } else {
        QaPhase::Off
    };
    let startup_stage = match phase {
        QaPhase::Starting if startup_stage.is_some() => startup_stage,
        QaPhase::Off | QaPhase::Starting | QaPhase::Unavailable if availability.is_some() => {
            match state.workspace_root.as_deref() {
                Some(root) => pack_progress(root).await,
                None => None,
            }
        }
        _ => None,
    };
    context.revalidate(&state.repo).await?;
    let mut settings = QaSettings::new(
        setting.enabled,
        phase,
        u64::try_from(setting.revision).unwrap_or_default(),
        u64::try_from(setting.generation).unwrap_or_default(),
        context.permits(Permission::ManageInboxQa),
    );
    if phase == QaPhase::Unavailable {
        settings.availability_reason = reason;
    }
    settings.startup_stage = startup_stage;
    if let Some(model) =
        availability.and_then(|availability| availability.model(setting.generation))
    {
        settings.model_target = model;
    }
    Ok(settings)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
async fn pack_progress(root: &Path) -> Option<QaStartupStage> {
    let root = root.to_path_buf();
    tokio::task::spawn_blocking(move || read_pack_progress(&root))
        .await
        .ok()
        .flatten()
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
async fn pack_progress(_root: &Path) -> Option<QaStartupStage> {
    None
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn read_pack_progress(root: &Path) -> Option<QaStartupStage> {
    use std::fs::{self, OpenOptions};
    use std::io::Read;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Marker {
        stage: QaStartupStage,
    }

    const MAX_BYTES: u64 = 128;
    const MAX_AGE: Duration = Duration::from_secs(15 * 60);

    // SAFETY: geteuid has no arguments or memory side effects.
    let uid = unsafe { libc::geteuid() };
    let directory = fs::symlink_metadata(root).ok()?;
    if !directory.is_dir() || directory.uid() != uid || directory.mode() & 0o777 != 0o700 {
        return None;
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(root.join("qa-pack-progress.json"))
        .ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file()
        || metadata.uid() != uid
        || metadata.mode() & 0o777 != 0o600
        || metadata.nlink() != 1
        || !(1..=MAX_BYTES).contains(&metadata.len())
        || metadata.modified().ok()?.elapsed().ok()? > MAX_AGE
    {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > MAX_BYTES {
        return None;
    }
    let marker: Marker = serde_json::from_slice(&bytes).ok()?;
    matches!(
        marker.stage,
        QaStartupStage::DownloadingPack | QaStartupStage::VerifyingPack
    )
    .then_some(marker.stage)
}

pub(crate) async fn settings_request(
    state: &ApiState,
    context: &ResolvedRequest,
    availability: Option<&QaRuntimeAvailability>,
) -> Result<QaSettings, ApiError> {
    require(context, Permission::ReadBackend, None)?;
    let settings = store::setting(&state.repo).await.map_err(storage_error)?;
    settings_value(state, context, settings, availability).await
}

pub(crate) async fn update_request(
    state: &ApiState,
    context: &ResolvedRequest,
    request: QaSettingsUpdate,
    availability: Option<&QaRuntimeAvailability>,
) -> Result<QaSettings, ApiError> {
    let actor = require(context, Permission::ManageInboxQa, Some(&request.context))?;
    let revision = i64::try_from(request.expected_revision).map_err(|_| crate::bad())?;
    context.revalidate(&state.repo).await?;
    let setting = store::set(&state.repo, &actor, revision, request.enabled)
        .await
        .map_err(|error| settings_storage_error("write_setting", error))?;
    settings_value(state, context, setting, availability).await
}

pub(crate) async fn submit_request(
    state: &ApiState,
    context: &ResolvedRequest,
    target: &QaTarget,
    availability: &QaRuntimeAvailability,
    request: QaTurnRequest,
) -> Result<QaTurn, ApiError> {
    let actor = require(
        context,
        Permission::SubmitInteractive,
        Some(&request.context),
    )?;
    if !context.permits(Permission::ReadBackend) {
        return Err(ContextFailure::forbidden().into());
    }
    request.validate().map_err(|_| crate::bad())?;
    context.revalidate(&state.repo).await?;
    if let Some(turn) = store::replay(&state.repo, &actor, &request)
        .await
        .map_err(storage_error)?
    {
        context.revalidate(&state.repo).await?;
        return Ok(turn);
    }
    let setting = store::setting(&state.repo).await.map_err(storage_error)?;
    if !setting.enabled {
        return Err(ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            "Backend inference is off",
        ));
    }
    if !store::replica_present(&state.repo, setting.generation)
        .await
        .map_err(storage_error)?
    {
        return Err(ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            "Backend inference has no ready worker",
        ));
    }
    let deadline = Instant::now() + Duration::from_secs(4);
    let follow_up = if question::classify(&request.question) == Question::FollowUp {
        tokio::time::timeout_at(deadline, store::last_answered_prompt(&state.repo, &actor))
            .await
            .map_err(|_| current_health_unavailable())?
            .map_err(storage_error)?
            .filter(|previous| {
                matches!(
                    question::classify(previous),
                    Question::Readiness(_)
                        | Question::ReadinessExplanation(_)
                        | Question::StatusChecks(_)
                        | Question::Metric(_)
                        | Question::HostResources
                        | Question::JobsSummary
                        | Question::Configuration
                        | Question::Summary
                )
            })
            .map(|question| {
                let mut resolved = request.clone();
                resolved.question = question;
                resolved
            })
    } else {
        None
    };
    let evidence_request = follow_up.as_ref().unwrap_or(&request);
    loop {
        let evidence = snapshot(
            state,
            context,
            target,
            &actor,
            evidence_request,
            setting.generation,
            availability,
            deadline,
        )
        .await?;
        context.revalidate(&state.repo).await?;
        match store::admit(&state.repo, &actor, &request, evidence, setting.generation).await {
            Ok(turn) => {
                context.revalidate(&state.repo).await?;
                return Ok(turn);
            }
            Err(TxError::Abort(reason))
                if reason == "inbox_qa:freshness" && Instant::now() < deadline => {}
            Err(error) => return Err(storage_error(error)),
        }
    }
}

async fn snapshot(
    state: &ApiState,
    context: &ResolvedRequest,
    target: &QaTarget,
    actor: &store::Actor,
    request: &QaTurnRequest,
    generation: i64,
    availability: &QaRuntimeAvailability,
    deadline: Instant,
) -> Result<store::FrozenEvidence, ApiError> {
    let _permit = target.probes.try_acquire().map_err(|_| {
        ApiError(
            StatusCode::TOO_MANY_REQUESTS,
            "Backend health observation is busy",
        )
    })?;
    let terminal_boundary = store::freshness_boundary(&state.repo, actor)
        .await
        .map_err(storage_error)?;
    let now = chrono::Utc::now().timestamp_millis();
    // Leave headroom for decoding within the five-second evidence budget.
    let health_boundary = terminal_boundary.max(now - 1_000);
    let boundary = terminal_boundary.max(now - 1);
    let intent = question::classify(&request.question);
    let source = request.source.as_ref();
    let mut selected_activity = None;
    if let Some(source) = source {
        match source.kind {
            ChatSourceKind::Job | ChatSourceKind::Cycle => {
                let observed_at = observation_start(boundary).await;
                let (label, status) =
                    tokio::time::timeout_at(deadline, activity_fact(state, context, source))
                        .await
                        .map_err(|_| current_health_unavailable())??;
                selected_activity = Some((label, status, observed_at));
            }
            ChatSourceKind::Readiness => {}
            _ => return Err(crate::missing()),
        }
    }

    let readiness_source = source.filter(|source| source.kind == ChatSourceKind::Readiness);
    let health_probe = async {
        if readiness_source.is_none()
            && !matches!(
                intent,
                Question::Readiness(_)
                    | Question::ReadinessExplanation(_)
                    | Question::StatusChecks(_)
                    | Question::Summary
            )
        {
            return Ok(None);
        }
        tokio::time::timeout_at(deadline, async {
            loop {
                if let Some(sample) = target
                    .observe(state, generation, health_boundary, true)
                    .await?
                {
                    return Ok::<_, ApiError>(Some(sample));
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .map_err(|_| current_health_unavailable())?
    };
    // Independent observations run together when a turn needs both.
    let (health, measurements) = tokio::join!(
        health_probe,
        metrics::collect_snapshot(
            state,
            context,
            target.sampler.as_ref(),
            intent,
            boundary,
            deadline
        ),
    );
    let health = health.ok().flatten();
    if let Some(source) = readiness_source {
        let sample = health.as_ref().ok_or_else(current_health_unavailable)?;
        if !sample.facts.iter().any(|fact| fact.id == source.id) {
            return Err(crate::missing());
        }
    }

    let mut passages = Vec::new();
    let mut sources = Vec::new();
    let mut earliest = None::<chrono::DateTime<chrono::Utc>>;
    let mut push = |reference,
                    label,
                    text,
                    observed_at: chrono::DateTime<chrono::Utc>,
                    measurement,
                    whole_passage| {
        earliest = Some(earliest.map_or(observed_at, |prior| prior.min(observed_at)));
        let opaque_id = format!("passage-{}", passages.len());
        passages.push(QaPassage::new(opaque_id.clone(), text));
        sources.push(store::Source {
            opaque_id,
            reference,
            label,
            observed_at: observed_at.to_rfc3339(),
            measurement,
            whole_passage,
        });
    };
    let mut answer_mode = store::AnswerMode::Grounded;
    let mut notice = None;
    let mut conversation = None;
    match intent {
        Question::Readiness(check) | Question::ReadinessExplanation(check) => {
            let explanation = matches!(intent, Question::ReadinessExplanation(_));
            if let Some((sample, fact)) = health.as_ref().and_then(|sample| {
                sample
                    .facts
                    .iter()
                    .find(|fact| {
                        fact.id == check.as_str()
                            && (!explanation || readiness::has_explanation(fact))
                    })
                    .map(|fact| (sample, fact))
            }) {
                let reference =
                    ChatSourceReference::new(ChatSourceKind::Readiness, fact.id.clone())
                        .map_err(|_| crate::bad())?;
                let at = chrono::DateTime::from_timestamp_millis(sample.sampled_at_ms)
                    .ok_or_else(current_health_unavailable)?;
                push(
                    reference,
                    readiness::label(fact).to_owned(),
                    if explanation {
                        readiness::explanation(fact)
                    } else {
                        readiness::answer(fact)
                    },
                    at,
                    None,
                    explanation,
                );
            }
        }
        Question::StatusChecks(selection) => {
            // This observation belongs to the validated caller, not the shared health sample.
            let registration = store::HealthFact {
                id: ReadinessCheckId::Registration.as_str().to_owned(),
                label: "Registration".into(),
                status: ReadinessState::Ready.as_str().to_owned(),
                reason: None,
                explanation: String::new(),
            };
            if let Some((sample, (summary, details))) = health.as_ref().and_then(|sample| {
                readiness::status_check_summary(
                    sample.facts.iter().chain([&registration]),
                    selection,
                )
                .map(|report| (sample, report))
            }) {
                let at = chrono::DateTime::from_timestamp_millis(sample.sampled_at_ms)
                    .ok_or_else(current_health_unavailable)?;
                let reference = ChatSourceReference::new(
                    ChatSourceKind::Readiness,
                    "workstation_status_checks".to_owned(),
                )
                .map_err(|_| crate::bad())?;
                let mut remaining_words =
                    QA_MAX_WORDS.saturating_sub(summary.split_whitespace().count());
                push(
                    reference,
                    "Workstation status checks".to_owned(),
                    summary,
                    at,
                    None,
                    false,
                );
                for fact in details.into_iter().take(QA_MAX_FINDINGS - 1) {
                    let text = readiness::answer(fact);
                    let words = text.split_whitespace().count();
                    if words > remaining_words {
                        continue;
                    }
                    remaining_words -= words;
                    let reference =
                        ChatSourceReference::new(ChatSourceKind::Readiness, fact.id.clone())
                            .map_err(|_| crate::bad())?;
                    push(
                        reference,
                        readiness::label(fact).to_owned(),
                        text,
                        at,
                        None,
                        false,
                    );
                }
            }
        }
        Question::Metric(_) | Question::HostResources | Question::JobsSummary => {
            for (_, fact) in measurements {
                let reference = ChatSourceReference::new(fact.kind, fact.id.into())
                    .map_err(|_| crate::bad())?;
                push(
                    reference,
                    fact.label.into(),
                    fact.text,
                    fact.observed_at,
                    Some(fact.measurement),
                    false,
                );
            }
        }
        Question::ContextStatus(kind) => {
            if let (Some(source), Some((label, status, at))) = (source, selected_activity) {
                if source.kind == kind && status != "unknown" && at.timestamp_millis() > boundary {
                    push(
                        source.clone(),
                        label.into(),
                        format!("{label} is {status}."),
                        at,
                        None,
                        false,
                    );
                }
            }
        }
        Question::ContextLogs(kind) => {
            if let (Some(source), Some((label, _, _))) = (
                source.filter(|source| source.kind == kind),
                selected_activity,
            ) {
                let at = observation_start(boundary).await;
                let counts = tokio::time::timeout_at(
                    deadline,
                    crate::operational_logs::qa_job_counts(state, context, &source.id, at),
                )
                .await
                .ok()
                .and_then(Result::ok)
                .flatten();
                if let Some(counts) = counts {
                    push(
                        source.clone(),
                        format!("{label} logs"),
                        format!(
                            "{label} recorded {} log entries over the last hour, including {} at error severity and {} at warning severity.",
                            counts.total, counts.error, counts.warn,
                        ),
                        at,
                        None,
                        false,
                    );
                }
            } else {
                notice = Some(QaTurnNotice::OutOfScope);
                conversation = Some(QaConversationReply::ContextNeeded);
            }
        }
        Question::Configuration => {
            // Admission atomically verifies that this Embedded generation is enabled and ready.
            let reference = ChatSourceReference::new(
                ChatSourceKind::Readiness,
                "embedded_inference".to_owned(),
            )
            .map_err(|_| crate::bad())?;
            let mut text = "This workstation is connected to the Embedded backend. Backend inference is on and the Assistant worker is ready.".to_owned();
            if let Some(model) = availability.model(generation) {
                use std::fmt::Write as _;
                let _ = write!(text, " The active model is {}.", model.label());
            }
            push(
                reference,
                "Backend inference".to_owned(),
                text,
                observation_start(boundary).await,
                None,
                false,
            );
        }
        Question::ExplainSelected => {
            if let Some(source) = source {
                if source.kind == ChatSourceKind::Readiness {
                    if let Some((sample, fact)) = health.as_ref().and_then(|sample| {
                        sample
                            .facts
                            .iter()
                            .find(|fact| fact.id == source.id)
                            .map(|fact| (sample, fact))
                    }) {
                        let observed_at =
                            chrono::DateTime::from_timestamp_millis(sample.sampled_at_ms)
                                .ok_or_else(current_health_unavailable)?;
                        push(
                            source.clone(),
                            readiness::label(fact).to_owned(),
                            readiness::explanation(fact),
                            observed_at,
                            None,
                            true,
                        );
                    }
                } else if let Some((label, status, at)) = selected_activity {
                    if status != "unknown" && at.timestamp_millis() > boundary {
                        push(
                            source.clone(),
                            label.into(),
                            format!("{label} is {status}."),
                            at,
                            None,
                            false,
                        );
                    }
                }
            } else {
                notice = Some(QaTurnNotice::OutOfScope);
                conversation = Some(QaConversationReply::ContextNeeded);
            }
        }
        Question::Summary => {
            if let Some(sample) = health.filter(|sample| !sample.facts.is_empty()) {
                answer_mode = store::AnswerMode::Model;
                let at = chrono::DateTime::from_timestamp_millis(sample.sampled_at_ms)
                    .ok_or_else(current_health_unavailable)?;
                let mut facts = sample
                    .facts
                    .iter()
                    .filter(|fact| {
                        !matches!(fact.status.as_str(), "not applicable" | "not_applicable")
                    })
                    .collect::<Vec<_>>();
                facts.sort_by_key(|fact| readiness::priority(fact));
                for fact in facts {
                    let reference =
                        ChatSourceReference::new(ChatSourceKind::Readiness, fact.id.clone())
                            .map_err(|_| crate::bad())?;
                    push(
                        reference,
                        readiness::label(fact).to_owned(),
                        readiness::answer(fact),
                        at,
                        None,
                        true,
                    );
                }
                if let (Some(source), Some((label, status, observed_at))) =
                    (source, selected_activity)
                {
                    push(
                        source.clone(),
                        label.into(),
                        format!("{label} is {status}."),
                        observed_at,
                        None,
                        true,
                    );
                }
            }
        }
        Question::Greeting => notice = Some(QaTurnNotice::Greeting),
        Question::Acknowledgement => {
            notice = Some(QaTurnNotice::Greeting);
            conversation = Some(QaConversationReply::Acknowledgement);
        }
        Question::Help => {
            notice = Some(QaTurnNotice::Greeting);
            conversation = Some(QaConversationReply::Help);
        }
        Question::FollowUp => {
            notice = Some(QaTurnNotice::OutOfScope);
            conversation = Some(QaConversationReply::FollowUpNeedsTopic);
        }
        Question::Recall => {
            notice = Some(QaTurnNotice::OutOfScope);
            conversation = Some(QaConversationReply::RecallNeedsHistory);
        }
        Question::Unsupported => {
            notice = Some(QaTurnNotice::OutOfScope);
            conversation = Some(QaConversationReply::ContextNeeded);
        }
    }
    let sampled_at_ms = match earliest {
        Some(observed_at) => observed_at,
        None => observation_start(boundary).await,
    }
    .timestamp_millis();
    let turn_id = store::accepted_turn_id(actor, request, generation).map_err(storage_error)?;
    let dialogue = match intent {
        Question::Greeting => Some(QaDialogueKind::Greeting),
        Question::Acknowledgement => Some(QaDialogueKind::Acknowledgement),
        Question::Help => Some(QaDialogueKind::Help),
        Question::Recall => Some(QaDialogueKind::Recall),
        Question::Summary | Question::ReadinessExplanation(_) if !passages.is_empty() => {
            Some(QaDialogueKind::Operational)
        }
        Question::ExplainSelected if readiness_source.is_some() && !passages.is_empty() => {
            Some(QaDialogueKind::Operational)
        }
        _ => None,
    };
    let mut worker_request =
        QaWorkerRequest::new(turn_id.clone(), turn_id, request.question.clone(), passages);
    if let Some(kind) = dialogue {
        answer_mode = store::AnswerMode::Model;
        worker_request.dialogue = Some(QaWorkerDialogue {
            kind,
            history: Vec::new(),
        });
    }
    Ok(store::FrozenEvidence {
        answer_mode,
        notice,
        conversation,
        request: worker_request,
        sources,
        sampled_at_ms,
    })
}

async fn observation_start(boundary: i64) -> chrono::DateTime<chrono::Utc> {
    let mut at = chrono::Utc::now();
    if at.timestamp_millis() <= boundary {
        tokio::time::sleep(Duration::from_millis(1)).await;
        at = chrono::Utc::now();
    }
    at
}

async fn activity_fact(
    state: &ApiState,
    context: &ResolvedRequest,
    source: &ChatSourceReference,
) -> Result<(&'static str, &'static str), ApiError> {
    let scope = crate::activity::scope_for_request(&state.repo, context, None).await?;
    let row = crate::activity::authorized_job(&state.repo, context, &scope, &source.id).await?;
    let label = if source.kind == ChatSourceKind::Cycle {
        if fleetworks_contracts::AtlasCycleReceipt::from_job_id(&row.job.id).is_none() {
            return Err(crate::missing());
        }
        "Selected cycle"
    } else {
        "Selected job"
    };
    let status = match row.job.state.as_str() {
        "queued" => "queued",
        "running" => "running",
        "reconciling" => "reconciling",
        "succeeded" | "completed" => "completed",
        "failed" => "failed",
        "cancelled" | "canceled" => "canceled",
        "partial" => "partially completed",
        "paused" => "paused",
        _ => "unknown",
    };
    Ok((label, status))
}

pub(crate) async fn latest_request(
    state: &ApiState,
    context: &ResolvedRequest,
) -> Result<Option<QaTurn>, ApiError> {
    let actor = require(context, Permission::ReadWorkstationActivity, None)?;
    let latest = store::latest(&state.repo, &actor)
        .await
        .map_err(storage_error)?;
    context.revalidate(&state.repo).await?;
    Ok(latest.map(|(turn, _)| turn))
}

pub(crate) async fn events_request(
    state: &ApiState,
    context: &ResolvedRequest,
    target: &QaTarget,
    query: QaEventQuery,
) -> Result<QaEventPage, ApiError> {
    let actor = require(context, Permission::ReadWorkstationActivity, None)?;
    query.validate().map_err(|_| crate::bad())?;
    let deadline = Instant::now() + Duration::from_secs(u64::from(query.wait_seconds));
    let long_poll = if query.wait_seconds == 0 {
        None
    } else {
        let permit = target.subscribers.try_acquire().map_err(|_| {
            ApiError(
                StatusCode::TOO_MANY_REQUESTS,
                "Inbox event subscribers are at capacity",
            )
        })?;
        Some((
            permit,
            target.events.subscribe(&state.repo, EventWaitKind::Poll),
        ))
    };
    let mut long_poll = long_poll;
    loop {
        let Some((turn, mut events)) = store::latest(&state.repo, &actor)
            .await
            .map_err(storage_error)?
            .filter(|(turn, _)| turn.turn_id == query.turn_id)
        else {
            return Err(crate::missing());
        };
        let gap = query.after > turn.cursor
            || events
                .first()
                .is_some_and(|event| event.sequence > query.after.saturating_add(1));
        if gap
            || turn.cursor > query.after
            || turn.state.is_terminal()
            || Instant::now() >= deadline
        {
            context.revalidate(&state.repo).await?;
            events.retain(|event| event.sequence > query.after && !gap);
            return Ok(QaEventPage::new(turn, events, gap));
        }
        if let Some((_, waiter)) = &mut long_poll {
            let _ = tokio::time::timeout_at(deadline, waiter.revision.changed()).await;
        }
    }
}

pub(crate) async fn cancel_request(
    state: &ApiState,
    context: &ResolvedRequest,
    request: QaCancelRequest,
) -> Result<QaTurn, ApiError> {
    let actor = require(
        context,
        Permission::SubmitInteractive,
        Some(&request.context),
    )?;
    request.validate().map_err(|_| crate::bad())?;
    context.revalidate(&state.repo).await?;
    let turn = store::cancel(&state.repo, &actor, &request.turn_id)
        .await
        .map_err(storage_error)?;
    context.revalidate(&state.repo).await?;
    Ok(turn)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_only_the_current_embedded_enablement_generation() {
        let availability = QaRuntimeAvailability::default();
        assert_eq!(availability.observe(1), None);

        let supervisor = availability.clone();
        supervisor.record(1, Some(QaAvailabilityReason::PackMissing));
        assert_eq!(
            availability.observe(1),
            Some(StartupObservation::Failure(
                QaAvailabilityReason::PackMissing
            ))
        );
        assert_eq!(availability.observe(2), None);

        supervisor.record_stage(2, QaStartupStage::LoadingModel);
        supervisor.record(1, Some(QaAvailabilityReason::WorkerStartFailed));
        assert_eq!(
            availability.observe(2),
            Some(StartupObservation::Progress(QaStartupStage::LoadingModel))
        );
        assert_eq!(availability.observe(1), None);
        supervisor.clear_stage(2);
        assert_eq!(availability.observe(2), None);

        supervisor.record(2, Some(QaAvailabilityReason::SandboxUnsupported));
        supervisor.clear_stage(2);
        assert_eq!(
            availability.observe(2),
            Some(StartupObservation::Failure(
                QaAvailabilityReason::SandboxUnsupported
            ))
        );
        supervisor.record(2, None);
        assert_eq!(availability.observe(2), None);
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn pack_progress_reads_only_private_fresh_installer_stages() {
        use std::fs;
        use std::os::unix::fs::{symlink, PermissionsExt};
        use std::time::SystemTime;

        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        fs::set_permissions(root, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(read_pack_progress(root), None);

        let marker = root.join("qa-pack-progress.json");
        fs::write(&marker, br#"{"stage":"downloading_pack"}"#).unwrap();
        fs::set_permissions(&marker, fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(read_pack_progress(root), None);
        fs::set_permissions(&marker, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(
            read_pack_progress(root),
            Some(QaStartupStage::DownloadingPack)
        );

        fs::write(&marker, br#"{"stage":"verifying_pack"}"#).unwrap();
        assert_eq!(
            read_pack_progress(root),
            Some(QaStartupStage::VerifyingPack)
        );
        fs::write(&marker, br#"{"stage":"loading_model"}"#).unwrap();
        assert_eq!(read_pack_progress(root), None);

        fs::write(&marker, br#"{"stage":"downloading_pack"}"#).unwrap();
        let previous = root.join("previous-progress.json");
        fs::rename(&marker, &previous).unwrap();
        symlink(&previous, &marker).unwrap();
        assert_eq!(read_pack_progress(root), None);
        fs::remove_file(&marker).unwrap();
        fs::rename(&previous, &marker).unwrap();

        fs::File::open(&marker)
            .unwrap()
            .set_modified(SystemTime::now() - Duration::from_secs(16 * 60))
            .unwrap();
        assert_eq!(read_pack_progress(root), None);
    }
}
