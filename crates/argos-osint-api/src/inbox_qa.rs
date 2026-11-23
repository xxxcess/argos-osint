//! Authenticated V2 Inbox QA admission and reads shared by HTTP and private IPC.

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
use axum::extract::{Extension, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use fleetworks_contracts::chat::{
    ChatSourceKind, ChatSourceReference, QaAvailabilityReason, QaCancelRequest,
    QaConversationReply, QaEventPage, QaEventQuery, QaFinding, QaPhase, QaSettings,
    QaSettingsUpdate, QaStartupStage, QaTurn, QaTurnNotice, QaTurnRequest, QA_CANCEL_PATH,
    QA_EVENTS_PATH, QA_EVIDENCE_HEADER, QA_EVIDENCE_VERSION, QA_LATEST_PATH, QA_SETTINGS_PATH,
    QA_TURNS_PATH,
};
use fleetworks_contracts::context::{CredentialPurpose, MutationContext, Permission};
use fleetworks_contracts::qa_worker::{QaPassage, QaWorkerEvent, QaWorkerRequest};
use futures_util::StreamExt;
use tokio::sync::{mpsc, watch, Semaphore};
use tokio::time::Instant;

use self::question::{Metric, Question};
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

    fn publish(&self, generation: i64, observation: Option<StartupObservation>) {
        self.0.send_modify(|current| {
            if current.is_none_or(|previous| previous.generation <= generation) {
                *current = Some(GenerationAvailability {
                    generation,
                    observation,
                });
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

/// Model process operations used by either executor placement.
pub type QaInferenceFuture<'a> = Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>>;

/// An executor owns an isolated implementation of this stream.
pub trait QaInference: Send + Sync {
    fn warm(&self) -> QaInferenceFuture<'_>;
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
    waiters: watch::Sender<usize>,
    observer: OnceLock<()>,
}

impl EventWake {
    fn new() -> Self {
        Self {
            revision: watch::channel(0).0,
            waiters: watch::channel(0).0,
            observer: OnceLock::new(),
        }
    }

    fn subscribe(&self, repo: &Repository) -> EventWaiter<'_> {
        let revision = self.revision.subscribe();
        self.waiters.send_modify(|count| *count += 1);
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
        }
    }
}

struct EventWaiter<'a> {
    wake: &'a EventWake,
    revision: watch::Receiver<i64>,
}

impl Drop for EventWaiter<'_> {
    fn drop(&mut self) {
        self.wake.waiters.send_modify(|count| *count -= 1);
    }
}

async fn observe_events(
    repo: Repository,
    mut waiters: watch::Receiver<usize>,
    revision: watch::Sender<i64>,
) {
    const FALLBACK: Duration = Duration::from_millis(200);
    const RESAMPLE: Duration = Duration::from_secs(1);
    while waiters.wait_for(|count| *count > 0).await.is_ok() {
        let mut live = tokio::select! {
            result = tokio::time::timeout(FALLBACK, store::event_changes(&repo)) => result.ok().and_then(Result::ok),
            _ = waiters.wait_for(|count| *count == 0) => continue,
        };
        loop {
            // Subscribe before the first read so a change during setup is observed by at
            // least one mechanism. Resample even on a quiet live connection after a reconnect.
            let sample = tokio::time::timeout(FALLBACK, store::event_revision(&repo)).await;
            if let Ok(Ok(sample)) = sample {
                wake_at(&revision, sample);
            }
            let delay = if live.is_some() { RESAMPLE } else { FALLBACK };
            let notice = async {
                if let Some(stream) = live.as_mut() {
                    stream.next().await
                } else {
                    std::future::pending().await
                }
            };
            tokio::select! {
                _ = waiters.wait_for(|count| *count == 0) => break,
                _ = tokio::time::sleep(delay) => {},
                notice = notice => match notice {
                    Some(Ok(sample)) => wake_at(&revision, sample),
                    _ => live = None,
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
    embedded: bool,
    sampler: Option<SharedSampler>,
    subscribers: Arc<Semaphore>,
    events: Arc<EventWake>,
    probes: Arc<Semaphore>,
}

impl QaTarget {
    pub(crate) fn new(
        readiness: Option<ReadinessOwners>,
        search: Option<SearchClient>,
        embedded: bool,
    ) -> Self {
        Self {
            readiness,
            search,
            embedded,
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
                // The durable claim outlives a disconnected or timed-out HTTP/IPC reader.
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
            let Some((at, facts)) = workstation::qa_snapshot(
                state,
                readiness,
                self.search.as_ref(),
                boundary,
                self.embedded,
            )
            .await
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
        loop {
            tokio::select! {
                () = stop.cancelled() => break,
                () = tokio::time::sleep(Duration::from_secs(1)) => {}
            }
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
        }
    }
}

/// Refresh coalesced backend facts while enabled, independent of drawers or event readers.
pub async fn run_health_refresh(
    state: ApiState,
    services: crate::PrivateServices,
    stop: CancellationToken,
) {
    let target = QaTarget::new(services.readiness, services.search, false);
    target.refresh(&state, &stop).await;
}

pub(crate) fn routes() -> Router<ApiState> {
    Router::new()
        .route(QA_SETTINGS_PATH, get(read_settings))
        .route(QA_TURNS_PATH, post(submit))
        .route(QA_LATEST_PATH, get(latest))
        .route(QA_EVENTS_PATH, get(events))
        .route(QA_CANCEL_PATH, post(cancel))
}

pub(crate) fn control_routes() -> Router<ApiState> {
    Router::new().route(QA_SETTINGS_PATH, post(update))
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
                        | Question::Metric(_)
                        | Question::HostResources
                        | Question::JobsSummary
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
    deadline: Instant,
) -> Result<store::FrozenEvidence, ApiError> {
    let _permit = target.probes.try_acquire().map_err(|_| {
        ApiError(
            StatusCode::TOO_MANY_REQUESTS,
            "Backend health observation is busy",
        )
    })?;
    let boundary = store::freshness_boundary(&state.repo, actor)
        .await
        .map_err(storage_error)?
        .max(chrono::Utc::now().timestamp_millis() - 1);
    let intent = question::classify(&request.question);
    let source = request.source.as_ref();
    let mut selected_activity = None;
    if let Some(source) = source {
        match source.kind {
            ChatSourceKind::Job | ChatSourceKind::Cycle => {
                let observed_at = chrono::Utc::now();
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

    let health_probe = async {
        tokio::time::timeout_at(deadline, async {
            loop {
                if let Some(sample) = target.observe(state, generation, boundary, true).await? {
                    return Ok::<_, ApiError>(sample);
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .map_err(|_| current_health_unavailable())?
    };
    let subject = if target.embedded {
        metrics::HostSubject::Embedded
    } else {
        metrics::HostSubject::ConnectedGateway
    };
    // Keep one current, authorized projection for the turn. Select the relevant
    // facts only after the host, Jobs and readiness observations have settled.
    let (health, measurements) = tokio::join!(
        health_probe,
        metrics::collect_snapshot(
            state,
            context,
            target.sampler.as_ref(),
            boundary,
            deadline,
            subject,
        ),
    );
    if let Some(source) = source.filter(|source| source.kind == ChatSourceKind::Readiness) {
        let sample = health.as_ref().map_err(|_| current_health_unavailable())?;
        if !sample.facts.iter().any(|fact| fact.id == source.id) {
            return Err(crate::missing());
        }
    }
    let health = health.ok();

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
            if let Some((sample, fact)) = health.as_ref().and_then(|sample| {
                let explanation = matches!(intent, Question::ReadinessExplanation(_));
                sample
                    .facts
                    .iter()
                    .find(|fact| {
                        fact.id == check.as_str() && (!explanation || fact.reason.is_some())
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
                    readiness::answer(fact),
                    at,
                    None,
                    false,
                );
            }
        }
        Question::Metric(_) | Question::HostResources | Question::JobsSummary => {
            let relevant = |metric| match (intent, metric) {
                (Question::Metric(requested), metric) => requested == metric,
                (Question::HostResources, Metric::Cpu | Metric::Memory | Metric::Disk)
                | (
                    Question::JobsSummary,
                    Metric::QueuedJobs | Metric::RunningJobs | Metric::RetryingJobs,
                ) => true,
                _ => false,
            };
            for (_, fact) in measurements
                .into_iter()
                .filter(|(metric, _)| relevant(*metric))
            {
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
                            readiness::answer(fact),
                            observed_at,
                            None,
                            false,
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
                for fact in &sample.facts {
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
        Question::Unsupported => {
            notice = Some(QaTurnNotice::OutOfScope);
            conversation = Some(QaConversationReply::ContextNeeded);
        }
    }
    let sampled_at_ms = earliest.unwrap_or_else(chrono::Utc::now).timestamp_millis();
    let turn_id = store::accepted_turn_id(actor, request, generation).map_err(storage_error)?;
    Ok(store::FrozenEvidence {
        answer_mode,
        notice,
        conversation,
        request: QaWorkerRequest::new(turn_id.clone(), turn_id, request.question.clone(), passages),
        sources,
        sampled_at_ms,
    })
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
        Some((permit, target.events.subscribe(&state.repo)))
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

struct HttpError(ApiError);

impl From<ApiError> for HttpError {
    fn from(error: ApiError) -> Self {
        Self(error)
    }
}

impl IntoResponse for HttpError {
    fn into_response(self) -> Response {
        let mut response = self.0.into_response();
        if response.status() == StatusCode::TOO_MANY_REQUESTS {
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from_static("1"));
        }
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        response
    }
}

type HttpResult<T> = Result<([(header::HeaderName, &'static str); 2], Json<T>), HttpError>;

fn response<T>(value: T) -> HttpResult<T> {
    Ok((
        [
            (header::CACHE_CONTROL, "private, no-store"),
            (header::VARY, QA_EVIDENCE_HEADER),
        ],
        Json(value),
    ))
}

fn legacy_evidence(headers: &HeaderMap) -> bool {
    headers.get(QA_EVIDENCE_HEADER) != Some(&HeaderValue::from_static(QA_EVIDENCE_VERSION))
}

fn project_legacy_finding(finding: &mut QaFinding) {
    if matches!(
        finding.source.kind,
        ChatSourceKind::HostMetric | ChatSourceKind::JobCount
    ) {
        finding.source.kind = ChatSourceKind::Readiness;
        finding.measurement = None;
    }
}

fn project_legacy_turn(turn: &mut QaTurn) {
    turn.findings.iter_mut().for_each(project_legacy_finding);
}

fn project_legacy_page(page: &mut QaEventPage) {
    project_legacy_turn(&mut page.turn);
    for finding in page
        .events
        .iter_mut()
        .filter_map(|event| event.finding.as_mut())
    {
        project_legacy_finding(finding);
    }
}

async fn read_settings(
    State(state): State<ApiState>,
    Extension(context): Extension<ResolvedRequest>,
) -> HttpResult<QaSettings> {
    response(settings_request(&state, &context, None).await?)
}

async fn update(
    State(state): State<ApiState>,
    Extension(context): Extension<ResolvedRequest>,
    Json(request): Json<QaSettingsUpdate>,
) -> HttpResult<QaSettings> {
    response(update_request(&state, &context, request, None).await?)
}

async fn submit(
    State(state): State<ApiState>,
    Extension(context): Extension<ResolvedRequest>,
    Extension(target): Extension<QaTarget>,
    headers: HeaderMap,
    Json(request): Json<QaTurnRequest>,
) -> HttpResult<QaTurn> {
    let mut turn = submit_request(&state, &context, &target, request).await?;
    if legacy_evidence(&headers) {
        project_legacy_turn(&mut turn);
    }
    response(turn)
}

async fn latest(
    State(state): State<ApiState>,
    Extension(context): Extension<ResolvedRequest>,
    headers: HeaderMap,
) -> HttpResult<Option<QaTurn>> {
    let mut latest = latest_request(&state, &context).await?;
    if let Some(turn) = latest.as_mut().filter(|_| legacy_evidence(&headers)) {
        project_legacy_turn(turn);
    }
    response(latest)
}

async fn events(
    State(state): State<ApiState>,
    Extension(context): Extension<ResolvedRequest>,
    Extension(target): Extension<QaTarget>,
    headers: HeaderMap,
    Query(query): Query<QaEventQuery>,
) -> HttpResult<QaEventPage> {
    let mut page = events_request(&state, &context, &target, query).await?;
    if legacy_evidence(&headers) {
        project_legacy_page(&mut page);
    }
    response(page)
}

async fn cancel(
    State(state): State<ApiState>,
    Extension(context): Extension<ResolvedRequest>,
    headers: HeaderMap,
    Json(request): Json<QaCancelRequest>,
) -> HttpResult<QaTurn> {
    let mut turn = cancel_request(&state, &context, request).await?;
    if legacy_evidence(&headers) {
        project_legacy_turn(&mut turn);
    }
    response(turn)
}

#[cfg(test)]
mod tests {
    use fleetworks_contracts::chat::{QaEvent, QaMeasurement, QaMeasurementUnit, QaTurnState};
    use serde::Deserialize;

    use super::*;

    #[derive(Deserialize)]
    #[serde(rename_all = "snake_case")]
    enum LegacySourceKind {
        Actor,
        Job,
        Cycle,
        Readiness,
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct LegacySource {
        kind: LegacySourceKind,
        id: String,
    }

    #[derive(Deserialize)]
    struct LegacyFinding {
        source: LegacySource,
    }

    #[derive(Deserialize)]
    struct LegacyTurn {
        findings: Vec<LegacyFinding>,
        cursor: u64,
    }

    #[derive(Deserialize)]
    struct LegacyEvent {
        sequence: u64,
        state: QaTurnState,
        finding: Option<LegacyFinding>,
    }

    #[derive(Deserialize)]
    struct LegacyPage {
        turn: LegacyTurn,
        events: Vec<LegacyEvent>,
        gap: bool,
    }

    #[test]
    fn negotiated_legacy_view_decodes_metrics_without_changing_the_event_cursor() {
        let mut headers = HeaderMap::new();
        assert!(legacy_evidence(&headers));
        headers.insert(QA_EVIDENCE_HEADER, HeaderValue::from_static("2"));
        assert!(legacy_evidence(&headers));
        headers.insert(
            QA_EVIDENCE_HEADER,
            HeaderValue::from_static(QA_EVIDENCE_VERSION),
        );
        assert!(!legacy_evidence(&headers));

        for (kind, id, unit) in [
            (
                ChatSourceKind::HostMetric,
                "cpu",
                QaMeasurementUnit::PercentTenths,
            ),
            (
                ChatSourceKind::JobCount,
                "running_jobs",
                QaMeasurementUnit::Jobs,
            ),
        ] {
            let mut finding = QaFinding::new(
                ChatSourceReference::new(kind, id.to_owned()).unwrap(),
                "Registered evidence".to_owned(),
                "A frozen reading.".to_owned(),
                0,
                17,
                "2026-11-21T10:00:00+00:00".to_owned(),
            );
            finding.measurement = Some(QaMeasurement::new(0, unit, "fixture".to_owned()));
            let mut turn = QaTurn::queued(
                "turn".to_owned(),
                "request".to_owned(),
                "Question?".to_owned(),
            );
            turn.findings.push(finding.clone());
            turn.state = QaTurnState::Answered;
            turn.cursor = 2;
            let mut page = QaEventPage::new(
                turn.clone(),
                vec![
                    QaEvent::new(1, QaTurnState::Running, Some(finding.clone())),
                    QaEvent::new(2, QaTurnState::Answered, None),
                ],
                false,
            );
            assert!(
                serde_json::from_value::<LegacyPage>(serde_json::to_value(&page).unwrap()).is_err()
            );
            project_legacy_page(&mut page);
            let shape = serde_json::to_value(&page).unwrap();
            assert!(shape["turn"]["findings"][0].get("measurement").is_none());
            assert!(shape["events"][0]["finding"].get("measurement").is_none());
            let legacy = serde_json::from_value::<LegacyPage>(shape).unwrap();
            assert!(!legacy.gap);
            assert_eq!(legacy.turn.cursor, turn.cursor);
            assert_eq!(legacy.events[0].sequence, 1);
            assert_eq!(legacy.events[1].sequence, legacy.turn.cursor);
            assert_eq!(legacy.events[1].state, QaTurnState::Answered);
            assert!(legacy.events[1].finding.is_none());
            for read in [
                &legacy.turn.findings[0],
                legacy.events[0].finding.as_ref().unwrap(),
            ] {
                assert!(matches!(read.source.kind, LegacySourceKind::Readiness));
                assert_eq!(read.source.id, id);
            }
            finding.source.kind = ChatSourceKind::Readiness;
            finding.measurement = None;
            assert_eq!(page.turn.findings, [finding.clone()]);
            assert_eq!(page.events[0].finding, Some(finding));
            assert_eq!(turn.findings[0].source.kind, kind);
            assert_eq!(turn.findings[0].measurement.as_ref().unwrap().value, 0);
        }
    }

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
