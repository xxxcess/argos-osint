//! Authenticated Inbox QA admission and reads over private Embedded IPC.

mod comparison;
pub mod context;
mod conversation;
pub mod decision;
mod metrics;
mod question;
mod readiness;
mod triage;

pub(crate) use triage::related_activity_request;

pub use readiness::validated_summary_finding;

use std::collections::HashSet;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use argos_osint_core::nlp;
use argos_osint_core::nlp::conversation::Context as ConversationContext;
use argos_osint_core::nlp::conversation::Interval;
use argos_osint_core::storage::inbox_qa as store;
use argos_osint_core::storage::repo::Repository;
use argos_osint_core::storage::tx::TxError;
use axum::http::StatusCode;
use fleetworks_contracts::chat::QaEvidenceProvenance;
use fleetworks_contracts::chat::QaMeasurement;
use fleetworks_contracts::chat::QaMeasurementUnit;
use fleetworks_contracts::chat::{
    ChatSourceKind, ChatSourceReference, QaAvailabilityReason, QaCancelRequest, QaEventPage,
    QaEventQuery, QaGeneratorSettings, QaModelTarget, QaPhase, QaRelatedActivity, QaSettings,
    QaSettingsUpdate, QaStartupStage, QaTurn, QaTurnRequest, QA_MAX_FINDINGS, QA_MAX_WORDS,
};
use fleetworks_contracts::context::{CredentialPurpose, MutationContext, Permission};
use fleetworks_contracts::qa_worker::{
    QaDialogueKind, QaPassage, QaWorkerDialogue, QaWorkerEvent, QaWorkerRequest,
    QA_MAX_DIALOGUE_MESSAGE_BYTES, QA_MAX_VISIBLE_WORDS, QA_OPERATIONAL_OBSERVATIONS_INTRODUCTION,
};
use fleetworks_contracts::readiness::ReadinessCheckId;
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

const MAX_ADMISSION_FRESHNESS_RETRIES: usize = 4;
const ADMISSION_FRESHNESS_BACKOFF: Duration = Duration::from_millis(50);
const MAX_SUMMARY_PASSAGES: usize = 6;
const JOB_NOTIFICATION_LOG_DEADLINE: Duration = Duration::from_secs(1);

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
pub struct QaRuntimeAvailability(
    watch::Sender<Option<GenerationAvailability>>,
    Option<Arc<context::Observer>>,
);

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
        Self(watch::channel(None).0, None)
    }
}

impl QaRuntimeAvailability {
    /// Report a failure or clear it after successful startup for an enablement generation.
    pub fn record(&self, generation: i64, reason: Option<QaAvailabilityReason>) {
        self.publish(generation, reason.map(StartupObservation::Failure));
        if let Some(reason) = reason {
            self.record_generator(
                generation,
                context::GeneratorState::Unavailable,
                Some(reason),
            );
        }
    }

    pub(crate) fn with_context(mut self, observer: Arc<context::Observer>) -> Self {
        self.1 = Some(observer);
        self
    }

    /// Record a generator whose lease and ready handshake were confirmed.
    pub fn record_resident(&self, generation: i64) {
        self.record_generator(generation, context::GeneratorState::Ready, None);
    }

    /// Record a generator confirmed to have exited.
    pub fn record_unloaded(&self, generation: i64) {
        self.record_generator(generation, context::GeneratorState::Unloaded, None);
    }

    fn record_generator(
        &self,
        generation: i64,
        state: context::GeneratorState,
        reason: Option<QaAvailabilityReason>,
    ) {
        if self
            .0
            .borrow()
            .is_some_and(|current| current.generation > generation)
        {
            return;
        }
        if let Some(observer) = &self.1 {
            observer.record_generator(generation, state, reason);
        }
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

    fn ready_model(&self, generation: i64) -> Option<QaModelTarget> {
        self.0
            .borrow()
            .filter(|current| current.generation == generation && current.observation.is_none())
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
    decision: Option<Arc<dyn decision::DecisionRuntime>>,
    context: Option<Arc<context::Observer>>,
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
            decision: None,
            context: None,
        }
    }

    pub(crate) fn with_decision(mut self, runtime: Arc<dyn decision::DecisionRuntime>) -> Self {
        self.decision = Some(runtime);
        self
    }

    pub(crate) fn with_context(mut self, observer: Arc<context::Observer>) -> Self {
        self.context = Some(observer);
        self
    }

    fn decision_status(
        &self,
        setting: &store::Setting,
    ) -> fleetworks_contracts::chat::QaDecisionSettings {
        use fleetworks_contracts::chat::QaDecisionAvailabilityReason;
        use fleetworks_contracts::chat::QaDecisionPreference;
        let preference = setting.decision_preference;
        if !setting.enabled {
            return fleetworks_contracts::chat::QaDecisionSettings::new(
                preference,
                QaDecisionPreference::RulesOnly,
                fleetworks_contracts::chat::QaPhase::Off,
                None,
            );
        }
        if let Some(runtime) = &self.decision {
            return runtime.status(preference, setting.decision_generation);
        }
        decision::rules_status(
            preference,
            (preference != QaDecisionPreference::RulesOnly)
                .then_some(QaDecisionAvailabilityReason::AwaitingQualification),
        )
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
            let Some((at, facts, checks)) =
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
            if let Some(observer) = self.context.as_ref().filter(|_| published) {
                observer.record_readiness(&checks);
            }
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
            "inbox_qa:generator_unavailable" => return generator_unavailable(),
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

fn generator_unavailable() -> ApiError {
    ApiError(
        StatusCode::SERVICE_UNAVAILABLE,
        "Assistant generator is unavailable",
    )
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
    target: &QaTarget,
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
    let ready_model =
        availability.and_then(|availability| availability.ready_model(setting.generation));
    let phase = if setting.enabled {
        if ready_model.is_some()
            && store::replica_present(&state.repo, setting.generation)
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
    let model = availability.and_then(|availability| availability.model(setting.generation));
    settings.model_target = model
        .or_else(|| setting.generator_preference.explicit_target())
        .unwrap_or_default();
    settings.generator = Some(QaGeneratorSettings::new(
        setting.generator_preference,
        ready_model.filter(|_| phase == QaPhase::Ready),
        phase,
        settings.availability_reason,
    ));
    settings.decision = Some(target.decision_status(&setting));
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
    target: &QaTarget,
    availability: Option<&QaRuntimeAvailability>,
) -> Result<QaSettings, ApiError> {
    require(context, Permission::ReadBackend, None)?;
    let settings = store::setting(&state.repo).await.map_err(storage_error)?;
    settings_value(state, context, target, settings, availability).await
}

pub(crate) async fn update_request(
    state: &ApiState,
    context: &ResolvedRequest,
    target: &QaTarget,
    request: QaSettingsUpdate,
    availability: Option<&QaRuntimeAvailability>,
) -> Result<QaSettings, ApiError> {
    let actor = require(context, Permission::ManageInboxQa, Some(&request.context))?;
    let revision = i64::try_from(request.expected_revision).map_err(|_| crate::bad())?;
    context.revalidate(&state.repo).await?;
    let update = store::SettingUpdate {
        enabled: request.enabled,
        generator_preference: request.generator_preference,
        decision_preference: request.decision_preference,
    };
    let setting = store::update(&state.repo, &actor, revision, update)
        .await
        .map_err(|error| settings_storage_error("write_setting", error))?;
    settings_value(state, context, target, setting, availability).await
}

pub(crate) async fn submit_request(
    state: &ApiState,
    context: &ResolvedRequest,
    target: &QaTarget,
    availability: &QaRuntimeAvailability,
    request: QaTurnRequest,
) -> Result<QaTurn, ApiError> {
    let response_started_at_ms = chrono::Utc::now().timestamp_millis();
    let actor = require(
        context,
        Permission::SubmitInteractive,
        Some(&request.context),
    )?;
    request.validate().map_err(|_| crate::bad())?;
    context.revalidate(&state.repo).await?;
    let intent = question::classify(&request.question);
    let ignores_attachment = request.source.is_some()
        && conversation::ignores_selected_source(&request.question, intent);
    let selected = request.source.as_ref().filter(|_| !ignores_attachment);
    require_evidence_permission(context, intent, selected)?;
    if let Some(turn) = store::replay(&state.repo, &actor, &request)
        .await
        .map_err(storage_error)?
    {
        context.revalidate(&state.repo).await?;
        require_turn_evidence_permission(context, &turn)?;
        return Ok(turn);
    }
    let setting = store::setting(&state.repo).await.map_err(storage_error)?;
    if !setting.enabled {
        return Err(ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            "Backend inference is off",
        ));
    }
    if availability.ready_model(setting.generation).is_none()
        || !store::replica_present(&state.repo, setting.generation)
            .await
            .map_err(storage_error)?
    {
        return Err(generator_unavailable());
    }
    let deadline = Instant::now() + Duration::from_secs(4);
    let features = nlp::Service::parse_conversation(
        &request.question,
        ConversationContext::workstation(response_started_at_ms),
        deadline,
    )
    .await;
    let may_reuse = !ignores_attachment
        && matches!(
            intent,
            Question::FollowUp
                | Question::StatusChecksFollowUp(..)
                | Question::ChooseSubject
                | Question::ExplainSelected
                | Question::Unsupported,
        );
    let read_previous = async {
        if may_reuse {
            store::last_answered_prompt(&state.repo, &actor).await
        } else {
            Ok(None)
        }
    };
    let read_pending = async {
        if features.status == nlp::Status::Complete && may_reuse && request.source.is_none() {
            store::pending_clarification(&state.repo, &actor).await
        } else {
            Ok(None)
        }
    };
    let (previous, pending) = tokio::time::timeout_at(deadline, async {
        tokio::try_join!(read_previous, read_pending)
    })
    .await
    .map_err(|_| current_health_unavailable())?
    .map_err(storage_error)?;
    let resolved = conversation::resolve(
        &request.question,
        request.source.as_ref(),
        intent,
        &features,
        previous.as_ref().and_then(|prompt| prompt.focus.as_ref()),
        pending.as_ref(),
    );
    let mut routed_request = request.clone();
    routed_request.source = resolved.source;
    let contextual = resolved.general_clause.is_some();
    if let Some(general_clause) = resolved.general_clause {
        routed_request.question = general_clause;
    }
    let historical = resolved
        .focus
        .as_ref()
        .and_then(|focus| focus.temporal.as_ref());
    let (mut intent, mut used_previous, mut observation) =
        (resolved.intent, resolved.used_previous_exchange, None);
    let mut status_check_follow_up = None;
    if used_previous && historical.is_none() && resolved.pending.is_none() {
        if let Some(previous) = &previous {
            observation = previous.observation.clone();
            if matches!(
                intent,
                Question::ContextStatus(_) | Question::ContextLogs(_) | Question::ExplainSelected
            ) {
                routed_request.question = previous.question.clone();
            }
        }
    } else if matches!(
        intent,
        Question::FollowUp | Question::StatusChecksFollowUp(..)
    ) && resolved.pending.is_none()
    {
        if let Some(follow_up) = resolve_follow_up(&routed_request, intent, previous) {
            intent = follow_up.intent;
            routed_request = follow_up.request;
            observation = follow_up.observation;
            used_previous = follow_up.used_previous_exchange;
            status_check_follow_up = follow_up.status_check_follow_up;
        }
    }
    require_evidence_permission(context, intent, routed_request.source.as_ref())?;
    routed_request.job_notification &= intent == Question::ExplainSelected
        && !used_previous
        && !contextual
        && routed_request.source == request.source;
    let runtime = SnapshotRuntime {
        intent,
        contextual,
        generation: setting.generation,
        history_generation: setting.history_generation,
        availability,
        deadline,
        response_started_at_ms,
        used_previous_exchange: used_previous,
        status_check_follow_up,
        previous_observation: observation.as_ref(),
        historical,
        focus: resolved.focus.as_ref(),
        pending: resolved.pending.as_ref(),
        parser_status: features.status,
    };
    let mut freshness_retries = 0;
    loop {
        let snapshot = snapshot(state, context, target, &actor, &routed_request, &runtime).await?;
        context.revalidate(&state.repo).await?;
        match store::admit(
            &state.repo,
            &actor,
            &request,
            snapshot.evidence,
            snapshot.metadata,
            setting.generation,
        )
        .await
        {
            Ok(turn) => {
                context.revalidate(&state.repo).await?;
                require_turn_evidence_permission(context, &turn)?;
                return Ok(turn);
            }
            Err(TxError::Abort(reason))
                if reason == "inbox_qa:freshness"
                    && freshness_retries < MAX_ADMISSION_FRESHNESS_RETRIES
                    && Instant::now() + ADMISSION_FRESHNESS_BACKOFF < deadline =>
            {
                freshness_retries += 1;
                tokio::time::sleep(ADMISSION_FRESHNESS_BACKOFF).await;
            }
            Err(error) => return Err(storage_error(error)),
        }
    }
}

fn require_evidence_permission(
    context: &ResolvedRequest,
    intent: Question,
    source: Option<&ChatSourceReference>,
) -> Result<(), ApiError> {
    let selected_job = source
        .is_some_and(|source| matches!(source.kind, ChatSourceKind::Job | ChatSourceKind::Cycle));
    let selected_readiness = source.is_some_and(|source| source.kind == ChatSourceKind::Readiness);
    let activity = context.permits(Permission::ReadWorkstationActivity);
    let backend = context.permits(Permission::ReadBackend);
    let allowed = match intent {
        Question::JobsSummary
        | Question::RunningJobList
        | Question::ContextStatus(_)
        | Question::ContextLogs(_) => activity,
        Question::Metric(metric) => match metric {
            question::Metric::Cpu | question::Metric::Memory | question::Metric::Disk => backend,
            _ => activity,
        },
        Question::Readiness(_)
        | Question::ReadinessExplanation(_)
        | Question::StatusChecks(_)
        | Question::StatusChecksFollowUp(..)
        | Question::HostResources
        | Question::Configuration
        | Question::Summary => backend,
        _ => backend || activity,
    };
    if allowed && (!selected_job || activity) && (!selected_readiness || backend) {
        Ok(())
    } else {
        Err(ContextFailure::forbidden().into())
    }
}

fn require_turn_evidence_permission(
    context: &ResolvedRequest,
    turn: &QaTurn,
) -> Result<(), ApiError> {
    let visible = |source: &ChatSourceReference| match source.kind {
        ChatSourceKind::Job | ChatSourceKind::Cycle | ChatSourceKind::JobCount => {
            context.permits(Permission::ReadWorkstationActivity)
        }
        ChatSourceKind::Readiness | ChatSourceKind::HostMetric => {
            context.permits(Permission::ReadBackend)
        }
        _ => false,
    };
    if turn.findings.iter().all(|finding| visible(&finding.source))
        && match &turn.related_activity {
            Some(QaRelatedActivity::Found { target, .. }) => visible(target),
            _ => true,
        }
    {
        Ok(())
    } else {
        Err(ContextFailure::forbidden().into())
    }
}

fn resolve_follow_up(
    request: &QaTurnRequest,
    intent: Question,
    previous: Option<store::AnsweredPrompt>,
) -> Option<ResolvedFollowUp> {
    let mut resolved = request.clone();
    if resolved.source.is_some() {
        if intent != Question::FollowUp {
            return None;
        }
        resolved.question = "What happened?".into();
        return Some(ResolvedFollowUp {
            intent: question::classify(&resolved.question),
            request: resolved,
            observation: None,
            used_previous_exchange: false,
            status_check_follow_up: None,
        });
    }
    let previous = previous?;
    if let Question::StatusChecksFollowUp(selection, hinted_count) = intent {
        let summary = previous
            .status_check_summary
            .filter(|_| !previous.intervening_turn)?;
        resolved.question = previous.question;
        return Some(ResolvedFollowUp {
            intent: Question::StatusChecks(selection),
            request: resolved,
            observation: None,
            used_previous_exchange: true,
            status_check_follow_up: Some(readiness::StatusCheckFollowUp {
                hinted_count,
                previous_tally: previous
                    .status_check_tally
                    .or_else(|| readiness::retained_status_check_tally(&summary)),
            }),
        });
    }
    let previous_intent = question::classify(&previous.question);
    let supported = match previous_intent {
        Question::ContextStatus(_) | Question::ContextLogs(_) | Question::ExplainSelected => {
            previous.subject.is_some()
        }
        Question::Readiness(_)
        | Question::ReadinessExplanation(_)
        | Question::StatusChecks(_)
        | Question::Metric(_)
        | Question::HostResources
        | Question::JobsSummary
        | Question::RunningJobList
        | Question::Configuration
        | Question::Summary => true,
        _ => false,
    };
    if !supported {
        return None;
    }
    resolved.question = previous.question;
    resolved.source = previous.subject;
    Some(ResolvedFollowUp {
        intent: previous_intent,
        request: resolved,
        observation: previous.observation,
        used_previous_exchange: true,
        status_check_follow_up: None,
    })
}

struct ResolvedFollowUp {
    intent: Question,
    request: QaTurnRequest,
    observation: Option<store::Observation>,
    used_previous_exchange: bool,
    status_check_follow_up: Option<readiness::StatusCheckFollowUp>,
}

struct Snapshot {
    evidence: store::FrozenEvidence,
    metadata: store::AdmissionMetadata,
}

struct SnapshotRuntime<'a> {
    intent: Question,
    contextual: bool,
    generation: i64,
    history_generation: i64,
    availability: &'a QaRuntimeAvailability,
    deadline: Instant,
    response_started_at_ms: i64,
    used_previous_exchange: bool,
    status_check_follow_up: Option<readiness::StatusCheckFollowUp>,
    previous_observation: Option<&'a store::Observation>,
    historical: Option<&'a Interval>,
    focus: Option<&'a nlp::conversation::Focus>,
    pending: Option<&'a nlp::conversation::PendingClarification>,
    parser_status: nlp::Status,
}

fn dialogue_kind(
    intent: Question,
    has_evidence: bool,
    may_read_backend: bool,
    contextual: bool,
    may_answer_general: bool,
) -> QaDialogueKind {
    match intent {
        Question::Greeting => QaDialogueKind::Greeting,
        Question::Acknowledgement => QaDialogueKind::Acknowledgement,
        Question::Help => QaDialogueKind::Help,
        Question::Recall if may_read_backend => QaDialogueKind::Recall,
        _ if contextual && has_evidence => QaDialogueKind::Contextual,
        _ if has_evidence => QaDialogueKind::Operational,
        _ if contextual || intent == Question::Unsupported && may_answer_general => {
            QaDialogueKind::General
        }
        _ => QaDialogueKind::Clarification,
    }
}

fn limit_contextual_evidence(passages: &mut Vec<QaPassage>, sources: &mut Vec<store::Source>) {
    let mut words = 0;
    let mut bytes = 0;
    let retained = passages
        .iter()
        .take(QA_MAX_FINDINGS)
        .take_while(|passage| {
            words += passage.text.split_whitespace().count();
            bytes += passage.text.len();
            words <= QA_MAX_VISIBLE_WORDS && bytes <= QA_MAX_DIALOGUE_MESSAGE_BYTES
        })
        .count();
    passages.truncate(retained);
    sources.truncate(retained);
}

fn append_job_log_tail(
    passages: &mut Vec<QaPassage>,
    sources: &mut Vec<store::Source>,
    source: &ChatSourceReference,
    observed_at: chrono::DateTime<chrono::Utc>,
    excerpts: Vec<String>,
) {
    let introduction = QA_OPERATIONAL_OBSERVATIONS_INTRODUCTION;
    let mut words = introduction.split_whitespace().count();
    let mut bytes = introduction.len();
    for passage in passages.iter() {
        words += passage.text.split_whitespace().count();
        bytes += passage.text.len() + 1;
    }
    for excerpt in excerpts
        .into_iter()
        .take(QA_MAX_FINDINGS.saturating_sub(passages.len()))
    {
        words += excerpt.split_whitespace().count();
        bytes += excerpt.len() + 1;
        if words > QA_MAX_VISIBLE_WORDS || bytes > QA_MAX_DIALOGUE_MESSAGE_BYTES {
            break;
        }
        let opaque_id = format!("passage-{}", passages.len());
        let mut passage = QaPassage::new(opaque_id.clone(), excerpt);
        passage.job_log_tail = true;
        passages.push(passage);
        sources.push(store::Source {
            opaque_id,
            reference: source.clone(),
            label: "Recent job log".into(),
            observed_at: observed_at.to_rfc3339(),
            provenance: Some(QaEvidenceProvenance::Current),
            measurement: None,
            whole_passage: true,
        });
    }
}

async fn snapshot(
    state: &ApiState,
    context: &ResolvedRequest,
    target: &QaTarget,
    actor: &store::Actor,
    request: &QaTurnRequest,
    runtime: &SnapshotRuntime<'_>,
) -> Result<Snapshot, ApiError> {
    let deadline = runtime.deadline;
    let needs_probe = request.source.is_some()
        || runtime.historical.is_some()
        || question::is_operational(runtime.intent);
    let _permit = needs_probe
        .then(|| target.probes.try_acquire())
        .transpose()
        .map_err(|_| {
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
    let intent = runtime.intent;
    let source = request.source.as_ref();
    if runtime.historical.is_some() {
        return historical_snapshot(state, context, target, actor, request, runtime, boundary)
            .await;
    }
    let mut selected_activity = None;
    if let Some(source) = source {
        match source.kind {
            ChatSourceKind::Job | ChatSourceKind::Cycle => {
                let observed_at = observation_start(boundary).await;
                let activity =
                    tokio::time::timeout_at(deadline, triage::activity(state, context, source))
                        .await
                        .map_err(|_| current_health_unavailable())??;
                selected_activity = Some((activity, observed_at));
            }
            ChatSourceKind::Readiness => {}
            _ => return Err(crate::missing()),
        }
    }

    let readiness_source = source.filter(|source| source.kind == ChatSourceKind::Readiness);
    let registration = readiness_source
        .filter(|source| source.id == ReadinessCheckId::Registration.as_str())
        .map(|_| readiness::verified_registration());
    let health_probe = async {
        if (readiness_source.is_none() || registration.is_some())
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
                    .observe(state, runtime.generation, health_boundary, true)
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
    let selected_readiness = match (readiness_source, registration.as_ref()) {
        (Some(source), Some(fact)) => {
            // Registration belongs to this caller, outside the shared health sample.
            let observed_at = observation_start(boundary).await;
            context.revalidate(&state.repo).await?;
            Some((source, fact, observed_at))
        }
        (Some(source), None) => {
            let sample = health.as_ref().ok_or_else(current_health_unavailable)?;
            let fact = sample
                .facts
                .iter()
                .find(|fact| fact.id == source.id)
                .ok_or_else(crate::missing)?;
            let observed_at = chrono::DateTime::from_timestamp_millis(sample.sampled_at_ms)
                .ok_or_else(current_health_unavailable)?;
            Some((source, fact, observed_at))
        }
        (None, _) => None,
    };

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
            provenance: Some(QaEvidenceProvenance::Current),
            measurement,
            whole_passage,
        });
    };
    let mut related_activity = None;
    let mut observed_state = None;
    let mut guidance = None;
    let mut status_check_tally = None;
    let mut job_log_tail = None;
    match intent {
        Question::Readiness(check) | Question::ReadinessExplanation(check) => {
            let explanation = matches!(intent, Question::ReadinessExplanation(_));
            if let Some((sample, fact)) = health.as_ref().and_then(|sample| {
                sample
                    .facts
                    .iter()
                    .find(|fact| {
                        fact.id == check.as_str()
                            && (!explanation
                                || readiness::has_explanation(fact)
                                || runtime.previous_observation.is_some())
                    })
                    .map(|fact| (sample, fact))
            }) {
                let reference =
                    ChatSourceReference::new(ChatSourceKind::Readiness, fact.id.clone())
                        .map_err(|_| crate::bad())?;
                let at = chrono::DateTime::from_timestamp_millis(sample.sampled_at_ms)
                    .ok_or_else(current_health_unavailable)?;
                observed_state = readiness::comparison_state(fact);
                guidance = readiness::guidance(fact);
                push(
                    reference,
                    readiness::label(fact).to_owned(),
                    readiness::answer(fact),
                    at,
                    None,
                    explanation,
                );
            }
        }
        Question::StatusChecks(selection) => {
            // This observation belongs to the validated caller, not the shared health sample.
            let registration = readiness::verified_registration();
            if let Some((sample, report)) = health.as_ref().and_then(|sample| {
                readiness::status_check_summary(
                    sample.facts.iter().chain([&registration]),
                    selection,
                    runtime.status_check_follow_up,
                )
                .map(|report| (sample, report))
            }) {
                status_check_tally = Some(report.tally);
                let at = chrono::DateTime::from_timestamp_millis(sample.sampled_at_ms)
                    .ok_or_else(current_health_unavailable)?;
                let reference = ChatSourceReference::new(
                    ChatSourceKind::Readiness,
                    store::STATUS_CHECKS_SOURCE_ID.to_owned(),
                )
                .map_err(|_| crate::bad())?;
                let mut remaining_words =
                    QA_MAX_WORDS.saturating_sub(report.summary.split_whitespace().count());
                push(
                    reference,
                    "Workstation status checks".to_owned(),
                    report.summary,
                    at,
                    None,
                    false,
                );
                for fact in report.details.into_iter().take(QA_MAX_FINDINGS - 1) {
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
        Question::RunningJobList => {
            if let Some(running) = metrics::running_jobs(state, context, boundary, deadline).await {
                let count = running.count;
                let named = running.names.len();
                let noun = if count == 1 { "job" } else { "jobs" };
                let mut text = format!("This workstation has {count} running {noun}.");
                if count > named as u64 {
                    if named == 0 {
                        text.push_str(" Job names are unavailable.");
                    } else {
                        text.push_str(&format!(" Showing {named} with names."));
                    }
                }
                let label = "Running workstation jobs";
                let reference =
                    ChatSourceReference::new(ChatSourceKind::JobCount, "running".into())
                        .map_err(|_| crate::bad())?;
                push(
                    reference,
                    label.to_owned(),
                    text,
                    running.observed_at,
                    Some(QaMeasurement::new(
                        count,
                        QaMeasurementUnit::Jobs,
                        label.to_owned(),
                    )),
                    false,
                );
                for (reference, name) in running.names {
                    let text = format!("“{name}” is running.");
                    push(reference, name, text, running.observed_at, None, false);
                }
            }
        }
        Question::ContextStatus(kind) => {
            if let (Some(source), Some((activity, at))) = (source, &selected_activity) {
                if source.kind == kind
                    && activity.status != "unknown"
                    && at.timestamp_millis() > boundary
                {
                    observed_state = Some(activity.status);
                    push(
                        source.clone(),
                        activity.label.into(),
                        format!("{} is {}.", activity.label, activity.status),
                        *at,
                        None,
                        false,
                    );
                }
            }
        }
        Question::ContextLogs(kind) => {
            if let (Some(source), Some((activity, _))) = (
                source.filter(|source| source.kind == kind),
                &selected_activity,
            ) {
                let label = activity.label;
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
            }
        }
        Question::Configuration => {
            // Admission atomically verifies that this Embedded generation is enabled.
            let reference = ChatSourceReference::new(
                ChatSourceKind::Readiness,
                "embedded_inference".to_owned(),
            )
            .map_err(|_| crate::bad())?;
            let ready = store::replica_present(&state.repo, runtime.generation)
                .await
                .map_err(storage_error)?;
            let mut text =
                "This workstation is connected to the Embedded backend. Backend inference is on."
                    .to_owned();
            if ready {
                text.push_str(" The Assistant worker is ready.");
                if let Some(model) = runtime.availability.model(runtime.generation) {
                    use std::fmt::Write as _;
                    let _ = write!(text, " The active model is {}.", model.label());
                }
            } else {
                text.push_str(" The Assistant worker is unavailable.");
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
            if let Some((source, fact, observed_at)) = selected_readiness {
                observed_state = readiness::comparison_state(fact);
                guidance = readiness::guidance(fact);
                push(
                    source.clone(),
                    readiness::label(fact).to_owned(),
                    readiness::answer(fact),
                    observed_at,
                    None,
                    true,
                );
            } else if let Some((source, (activity, at))) = source.zip(selected_activity.as_ref()) {
                if activity.status != "unknown" && at.timestamp_millis() > boundary {
                    observed_state = Some(activity.status);
                    push(
                        source.clone(),
                        activity.label.into(),
                        activity.explanation(),
                        *at,
                        None,
                        false,
                    );
                }
                let collect_job_logs = async {
                    if !request.job_notification
                        || source.kind != ChatSourceKind::Job
                        || observed_state.is_none()
                    {
                        return None;
                    }
                    let model = runtime.availability.ready_model(runtime.generation)?;
                    let observed_at = observation_start(boundary).await;
                    let log_deadline = deadline.min(Instant::now() + JOB_NOTIFICATION_LOG_DEADLINE);
                    let read = crate::operational_logs::qa_job_tail(
                        state,
                        context,
                        &source.id,
                        model.input_tokens(),
                    );
                    let excerpts = tokio::time::timeout_at(log_deadline, read)
                        .await
                        .ok()?
                        .ok()?;
                    (!excerpts.is_empty()).then_some((source, observed_at, excerpts))
                };
                let related_status = async {
                    // The scoped job row describes active work. Waiting for a separate
                    // readiness observation can spend this row's inference budget.
                    let check = activity.check.filter(|_| {
                        activity.status != "running" && context.permits(Permission::ReadBackend)
                    })?;
                    triage::current_status(
                        target,
                        state,
                        runtime.generation,
                        check.as_str(),
                        deadline,
                    )
                    .await
                };
                (job_log_tail, related_activity) = tokio::join!(collect_job_logs, related_status);
            }
        }
        Question::Summary => {
            if let Some(sample) = health.as_ref().filter(|sample| !sample.facts.is_empty()) {
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
                for fact in facts.into_iter().take(MAX_SUMMARY_PASSAGES) {
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
            }
        }
        Question::Greeting
        | Question::Acknowledgement
        | Question::Help
        | Question::FollowUp
        | Question::StatusChecksFollowUp(..)
        | Question::ChooseSubject
        | Question::Recall
        | Question::Unsupported => {}
    }
    if runtime.contextual {
        limit_contextual_evidence(&mut passages, &mut sources);
    }
    let observation = if intent == Question::RunningJobList {
        None
    } else {
        comparison::single_observation(&sources, observed_state)
    };
    if let Some((previous, current)) = runtime.previous_observation.zip(observation.as_ref()) {
        comparison::append(previous, current, &mut passages, &mut sources);
        if runtime.contextual {
            limit_contextual_evidence(&mut passages, &mut sources);
        }
    }
    // Keep the lifecycle observation available for follow-ups. The supporting
    // logs are supplemental evidence only for this explicit notification turn.
    if let Some((source, observed_at, excerpts)) = job_log_tail {
        append_job_log_tail(&mut passages, &mut sources, source, observed_at, excerpts);
    }
    let sampled_at_ms = match earliest {
        Some(observed_at) => observed_at,
        None => observation_start(boundary).await,
    }
    .timestamp_millis();
    let turn_id = store::accepted_turn_id(actor, request, runtime.history_generation)
        .map_err(storage_error)?;
    let kind = dialogue_kind(
        intent,
        !passages.is_empty(),
        context.permits(Permission::ReadBackend),
        runtime.contextual,
        request.inbox_notifications.is_some()
            || !question::refers_to_workstation(&request.question),
    );
    let mut worker_request =
        QaWorkerRequest::new(turn_id.clone(), turn_id, request.question.clone(), passages);
    worker_request.dialogue = Some(QaWorkerDialogue {
        kind,
        history: Vec::new(),
    });
    Ok(Snapshot {
        evidence: store::FrozenEvidence {
            request: worker_request,
            sources,
            sampled_at_ms,
        },
        metadata: store::AdmissionMetadata {
            subject: source.cloned(),
            observation,
            related_activity,
            status_check_tally,
            response_started_at_ms: Some(runtime.response_started_at_ms),
            used_previous_exchange: runtime.used_previous_exchange,
            conversation: store::ConversationState {
                focus: runtime.focus.cloned(),
                pending: runtime.pending.cloned(),
                parser_status: Some(runtime.parser_status),
                guidance,
                history_requested: false,
                inbox_notifications: request.inbox_notifications.clone(),
            },
        },
    })
}

fn historical_resource(
    intent: Question,
    source: Option<&ChatSourceReference>,
) -> Option<context::Resource> {
    use context::Resource;
    if let Some(source) = source {
        if source.kind == ChatSourceKind::Job {
            return Some(Resource::Job(source.id.clone()));
        }
        if source.kind == ChatSourceKind::Readiness {
            return [
                ReadinessCheckId::Gateway,
                ReadinessCheckId::Datastore,
                ReadinessCheckId::Jobs,
                ReadinessCheckId::Brain,
                ReadinessCheckId::Control,
                ReadinessCheckId::Nlp,
                ReadinessCheckId::Classifier,
                ReadinessCheckId::ActorCatalog,
                ReadinessCheckId::ActorSearch,
                ReadinessCheckId::ClassifierInference,
                ReadinessCheckId::ClassifierTraining,
            ]
            .into_iter()
            .find(|check| check.as_str() == source.id)
            .map(Resource::Readiness);
        }
        return None;
    }
    match intent {
        Question::Readiness(check) | Question::ReadinessExplanation(check) => {
            Some(Resource::Readiness(check))
        }
        Question::JobsSummary | Question::RunningJobList => Some(Resource::Jobs),
        Question::Summary => Some(Resource::Backend),
        Question::Configuration => Some(Resource::Generator),
        _ => None,
    }
}

async fn historical_snapshot(
    state: &ApiState,
    request_context: &ResolvedRequest,
    target: &QaTarget,
    actor: &store::Actor,
    request: &QaTurnRequest,
    runtime: &SnapshotRuntime<'_>,
    boundary: i64,
) -> Result<Snapshot, ApiError> {
    let deadline = runtime.deadline;
    let interval = runtime.historical.ok_or_else(crate::bad)?;
    let mut passages = Vec::new();
    let mut sources = Vec::new();
    let mut sampled_at_ms = None;
    let mut guidance = None;
    if let Some((observer, resource)) = target
        .context
        .as_ref()
        .zip(historical_resource(runtime.intent, request.source.as_ref()))
    {
        let current_check = match &resource {
            context::Resource::Readiness(check)
                if interval.reference_ms.checked_add(1) == Some(interval.end_ms) =>
            {
                Some(*check)
            }
            _ => None,
        };
        let lookup = observer.history(
            state,
            request_context,
            resource,
            interval.start_ms,
            interval.end_ms,
        );
        // A busy/unavailable journal must not consume a turn's current-read or
        // generator budget.
        let lookup_deadline = deadline.min(Instant::now() + Duration::from_millis(350));
        if let Ok(Some(history)) = tokio::time::timeout_at(lookup_deadline, lookup).await {
            sampled_at_ms = Some(history.retrieved_at_ms);
            let index_pause = |entry: &&context::Entry| {
                matches!(
                    &entry.value,
                    context::ObservationValue::Readiness {
                        reason: Some(context::ReadinessReason::IndexIngestPaused),
                        ..
                    }
                )
            };
            let mut selected = history.entries.iter().collect::<Vec<_>>();
            if matches!(
                runtime.intent,
                Question::ReadinessExplanation(ReadinessCheckId::ActorSearch)
            ) {
                selected.sort_by_key(|entry| !index_pause(entry));
            }
            let mut distinct = HashSet::new();
            for entry in selected {
                if sources.len() >= 2 {
                    break;
                }
                let mut text = entry.passage();
                if !distinct.insert(text.clone()) {
                    continue;
                }
                let Some(at) = chrono::DateTime::from_timestamp_millis(entry.observed_at_ms) else {
                    continue;
                };
                let (reference, label) = match entry.value.resource() {
                    context::Resource::Readiness(check) => (
                        ChatSourceReference::new(ChatSourceKind::Readiness, check.as_str().into()),
                        "Recorded readiness check",
                    ),
                    context::Resource::Job(id) => (
                        ChatSourceReference::new(ChatSourceKind::Job, id),
                        "Recorded job status",
                    ),
                    context::Resource::Backend => (
                        ChatSourceReference::new(ChatSourceKind::Readiness, "gateway".into()),
                        "Recorded backend state",
                    ),
                    context::Resource::Generator => (
                        ChatSourceReference::new(
                            ChatSourceKind::Readiness,
                            "inbox_generator".into(),
                        ),
                        "Recorded generator state",
                    ),
                    _ => continue,
                };
                let (Ok(reference), Ok(sequence)) = (reference, u64::try_from(entry.sequence))
                else {
                    continue;
                };
                let opaque_id = format!("passage-{}", passages.len());
                if passages.is_empty() && history.incomplete {
                    text.push_str(" Local coverage for this interval is incomplete.");
                }
                passages.push(QaPassage::new(opaque_id.clone(), text));
                sources.push(store::Source {
                    opaque_id,
                    reference,
                    label: label.into(),
                    observed_at: at.to_rfc3339(),
                    provenance: Some(QaEvidenceProvenance::Historical {
                        sequence,
                        coverage_start_ms: history.coverage_started_at_ms,
                        has_gaps: history.incomplete,
                    }),
                    measurement: None,
                    whole_passage: false,
                });
                if index_pause(&entry) {
                    guidance =
                        Some(fleetworks_contracts::qa_worker::QaGuidance::CheckResourceUsage);
                }
            }
            // A since/until-now request can include a single fresh reading when
            // one is independently available for the same authorized subject.
            if let Some(check) = current_check.filter(|_| !sources.is_empty()) {
                let current = target.observe(
                    state,
                    runtime.generation,
                    boundary.max(interval.reference_ms - 1),
                    true,
                );
                let read_deadline = deadline.min(Instant::now() + Duration::from_millis(600));
                if let Ok(Ok(Some(sample))) = tokio::time::timeout_at(read_deadline, current).await
                {
                    if let Some((fact, at)) = sample
                        .facts
                        .iter()
                        .find(|fact| fact.id == check.as_str())
                        .zip(chrono::DateTime::from_timestamp_millis(
                            sample.sampled_at_ms,
                        ))
                    {
                        let opaque_id = format!("passage-{}", passages.len());
                        let reference = ChatSourceReference::new(
                            ChatSourceKind::Readiness,
                            check.as_str().into(),
                        )
                        .map_err(|_| crate::bad())?;
                        passages.push(QaPassage::new(opaque_id.clone(), readiness::answer(fact)));
                        sources.push(store::Source {
                            opaque_id,
                            reference,
                            label: readiness::label(fact).into(),
                            observed_at: at.to_rfc3339(),
                            provenance: Some(QaEvidenceProvenance::Current),
                            measurement: None,
                            whole_passage: false,
                        });
                    }
                }
            }
        }
    }
    let kind = dialogue_kind(
        runtime.intent,
        !passages.is_empty(),
        request_context.permits(Permission::ReadBackend),
        false,
        false,
    );
    let turn_id = store::accepted_turn_id(actor, request, runtime.history_generation)
        .map_err(storage_error)?;
    let mut worker_request =
        QaWorkerRequest::new(turn_id.clone(), turn_id, request.question.clone(), passages);
    worker_request.dialogue = Some(QaWorkerDialogue {
        kind,
        history: Vec::new(),
    });
    let sampled_at_ms = match sampled_at_ms.filter(|at| *at > boundary) {
        Some(at) => at,
        None => observation_start(boundary).await.timestamp_millis(),
    };
    Ok(Snapshot {
        evidence: store::FrozenEvidence {
            request: worker_request,
            sources,
            sampled_at_ms,
        },
        metadata: store::AdmissionMetadata {
            subject: request.source.clone(),
            observation: None,
            related_activity: None,
            status_check_tally: None,
            used_previous_exchange: runtime.used_previous_exchange,
            response_started_at_ms: Some(runtime.response_started_at_ms),
            conversation: store::ConversationState {
                focus: runtime.focus.cloned(),
                pending: runtime.pending.cloned(),
                parser_status: Some(runtime.parser_status),
                guidance,
                history_requested: true,
                inbox_notifications: None,
            },
        },
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

pub(crate) async fn latest_request(
    state: &ApiState,
    context: &ResolvedRequest,
) -> Result<Option<QaTurn>, ApiError> {
    let actor = require(context, Permission::ReadWorkstationActivity, None)?;
    let latest = store::latest(&state.repo, &actor)
        .await
        .map_err(storage_error)?;
    context.revalidate(&state.repo).await?;
    if let Some((turn, _)) = latest {
        require_turn_evidence_permission(context, &turn)?;
        Ok(Some(turn))
    } else {
        Ok(None)
    }
}

pub(crate) async fn history_request(
    state: &ApiState,
    context: &ResolvedRequest,
) -> Result<Vec<QaTurn>, ApiError> {
    let actor = require(context, Permission::ReadWorkstationActivity, None)?;
    let mut turns = store::history(&state.repo, &actor)
        .await
        .map_err(storage_error)?;
    context.revalidate(&state.repo).await?;
    turns.retain(|turn| require_turn_evidence_permission(context, turn).is_ok());
    Ok(turns)
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
        context.revalidate(&state.repo).await?;
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
        require_turn_evidence_permission(context, &turn)?;
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
    require_turn_evidence_permission(context, &turn)?;
    Ok(turn)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_supported_facts_use_the_generator_when_evidence_is_available() {
        for prompt in [
            "What is the current CPU utilization?",
            "How many jobs are running?",
            "What jobs are running?",
            "Does the catalog need seeding?",
            "What's our RAM consumption right now?",
            "Can you give me the big picture on this workstation?",
        ] {
            let intent = question::classify(prompt);
            assert_ne!(intent, Question::Unsupported, "{prompt}");
            assert!(matches!(
                dialogue_kind(intent, true, true, false, false),
                QaDialogueKind::Operational
            ));
            assert!(matches!(
                dialogue_kind(intent, true, false, false, false),
                QaDialogueKind::Operational
            ));
            assert!(matches!(
                dialogue_kind(intent, false, true, false, false),
                QaDialogueKind::Clarification
            ));
        }
    }

    #[test]
    fn unverified_workstation_questions_do_not_enter_open_generation() {
        for prompt in [
            "Can you give me the big picture on this workstation and reboot it?",
            "What is my processor's temperature?",
            "What are my most important notifications?",
        ] {
            let intent = question::classify(prompt);
            assert_eq!(intent, Question::Unsupported);
            assert!(question::refers_to_workstation(prompt));
            assert!(matches!(
                dialogue_kind(intent, false, true, false, false),
                QaDialogueKind::Clarification
            ));
        }
        for prompt in ["who is trump", "What is RAM?"] {
            let intent = question::classify(prompt);
            assert!(!question::refers_to_workstation(prompt));
            assert!(matches!(
                dialogue_kind(intent, false, true, false, true),
                QaDialogueKind::General
            ));
        }
    }

    #[test]
    fn mixed_questions_can_use_bounded_evidence_and_still_answer_without_it() {
        let intent = question::classify("What is the current CPU utilization?");
        assert!(matches!(
            dialogue_kind(intent, true, true, true, true),
            QaDialogueKind::Contextual
        ));
        assert!(matches!(
            dialogue_kind(intent, false, true, true, true),
            QaDialogueKind::General
        ));
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
        supervisor.record_model(2, QaModelTarget::default());
        assert_eq!(availability.ready_model(2), None);
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
        assert_eq!(availability.ready_model(2), None);
        supervisor.record(2, None);
        assert_eq!(availability.observe(2), None);
        assert_eq!(availability.ready_model(2), Some(QaModelTarget::default()));
        assert_eq!(availability.ready_model(1), None);
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
