//! Shared classifier home management for CLI, launchers and HTTP adapters.
//!
//! File work runs off Tokio workers. Readiness never trains, opens storage or
//! initializes JavaScript. Explicit operations always revalidate immutable data.

use std::collections::BTreeMap;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::task::JoinHandle;

use super::artifact::{self, ActivationReason, ArtifactStore, ModelId, ModelManifest};
use super::calibration::{
    self, ActivationAuthorizationDecision, ActivationOutcome, DomainAcceptance, EvaluationMetrics,
    ScoredFamily,
};
use super::dataset::{
    self, DatasetCandidate, DatasetError, DatasetRequest, DatasetResolution, DatasetSummary,
};
use super::training::{
    self, Split, SyntheticActivation, TrainRequest, TrainingOutcome, TrainingProgress,
    TrainingReport,
};
use super::{
    ClassifierError, ClassifierService, ClassifierStatus, ClassifyInput, ModelSnapshot, Provenance,
    WorkControl,
};

/// Safe diagnostic for presentation and wire responses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Diagnostic {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

impl Diagnostic {
    /// Constructs a safe presentation diagnostic.
    pub fn new(code: String, message: String, retryable: bool) -> Self {
        Self {
            code,
            message,
            retryable,
        }
    }
}

/// Orthogonal dataset status; missing data never invalidates a ready model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct DatasetReadiness {
    pub path: Option<String>,
    pub source: Option<DatasetCandidate>,
    pub availability: String,
    pub change: String,
    pub hash_check: String,
    pub sha256: Option<String>,
    pub diagnostic: Option<Diagnostic>,
}
/// Display adapter over the stored quality provenance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ProvenanceSummary {
    pub kind: String,
    pub label: String,
    pub dataset_version: String,
    pub dataset_sha256: String,
    pub review_reference: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed_dataset_sha256: Option<String>,
}
impl From<&Provenance> for ProvenanceSummary {
    fn from(value: &Provenance) -> Self {
        let (kind, version, hash, review) = match value {
            Provenance::ZeroShot => {
                return Self {
                    kind: "zero_shot".into(),
                    label: value.label().into(),
                    dataset_version: String::new(),
                    dataset_sha256: String::new(),
                    review_reference: None,
                    seed_dataset_sha256: None,
                }
            }
            Provenance::SyntheticBaseline {
                dataset_version,
                dataset_sha256,
            } => ("synthetic_baseline", dataset_version, dataset_sha256, None),
            Provenance::ReviewedReal {
                dataset_version,
                dataset_sha256,
                review_reference,
            } => (
                "reviewed_real",
                dataset_version,
                dataset_sha256,
                Some(review_reference.clone()),
            ),
            Provenance::ApprovedProvider {
                dataset_version,
                dataset_sha256,
                ..
            } => ("approved_provider", dataset_version, dataset_sha256, None),
            Provenance::MixedTraining {
                dataset_version,
                dataset_sha256,
                review_reference,
                ..
            } => (
                "mixed_training",
                dataset_version,
                dataset_sha256,
                Some(review_reference.clone()),
            ),
        };
        Self {
            kind: kind.into(),
            label: value.label().into(),
            dataset_version: version.clone(),
            dataset_sha256: hash.clone(),
            review_reference: review,
            seed_dataset_sha256: match value {
                Provenance::MixedTraining {
                    seed_dataset_sha256,
                    ..
                } => Some(seed_dataset_sha256.clone()),
                _ => None,
            },
        }
    }
}
/// Actual immutable evaluation decisions; no inferred production approval.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct QualitySummary {
    pub production_approved: bool,
    pub reason_codes: Vec<String>,
    pub domain_acceptance: Option<[DomainAcceptance; 6]>,
}
/// Verified model identity, separate from its readiness state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ModelSummary {
    pub model_id: String,
    pub generation: u64,
    pub activated_at: String,
    pub artifact_sha256: String,
    pub manifest_sha256: String,
    pub taxonomy_version: String,
    pub domains: Vec<super::DomainId>,
    pub algorithm_version: String,
    pub preprocessing_version: String,
    pub policy_version: String,
    pub receipt_schema_version: String,
    pub calibration_version: String,
    pub provenance: ProvenanceSummary,
    pub quality: QualitySummary,
}
/// Identical readiness DTO for every user interface.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Readiness {
    pub schema_version: String,
    pub state: ClassifierStatus,
    pub active_model_state: ClassifierStatus,
    pub feature_enabled: bool,
    pub can_infer: bool,
    pub can_train: bool,
    pub integrity: String,
    pub retraining_available: bool,
    pub dataset: DatasetReadiness,
    pub active_model: Option<ModelSummary>,
    pub training: Option<TrainingJobSummary>,
    pub diagnostic: Option<Diagnostic>,
}
/// Manual-job states, deliberately distinct from classifier readiness.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum TrainingJobState {
    Queued,
    Running,
    Cancelling,
    Completed,
    Failed,
    Cancelled,
    Interrupted,
}
impl TrainingJobState {
    fn terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Cancelled | Self::Interrupted
        )
    }
}
/// Prompt acceptance of an owned, persistent manual operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct JobAccepted {
    pub schema_version: String,
    pub job_id: String,
    pub request_id: String,
    pub state: TrainingJobState,
    pub reused: bool,
}
/// Latest progress only; no unbounded event log.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct TrainingJobSummary {
    pub job_id: String,
    pub request_id: String,
    pub state: TrainingJobState,
    pub revision: u64,
    pub progress: Option<TrainingProgress>,
    pub cancel_requested: bool,
}
/// Tagged presentation of the core activation decision. Stored training
/// contracts retain their existing enum representation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ActivationView {
    CandidateOnly {
        reason_code: String,
        reason: String,
        failures: Vec<calibration::EvidenceFailure>,
    },
    Activated {
        reason: ActivationReason,
        model_id: ModelId,
        generation: u64,
    },
}
impl From<ActivationOutcome> for ActivationView {
    fn from(value: ActivationOutcome) -> Self {
        match value {
            ActivationOutcome::CandidateOnly {
                reason_code,
                reason,
                failures,
            } => Self::CandidateOnly {
                reason_code,
                reason,
                failures,
            },
            ActivationOutcome::Activated {
                reason,
                model_id,
                generation,
            } => Self::Activated {
                reason,
                model_id,
                generation,
            },
        }
    }
}

/// Final training result, preserving the real core authorization outcome.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct TrainingOutcomeView {
    pub schema_version: String,
    pub job_id: String,
    pub state: String,
    pub candidate_model_id: String,
    pub active_model_id: Option<String>,
    pub activation: ActivationView,
    pub provenance: String,
    pub admission_thresholds_enforced: bool,
    pub report: TrainingReport,
}
/// Persistent job detail, including explicit operational failure.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct TrainingJob {
    pub schema_version: String,
    pub job_id: String,
    pub request_id: String,
    pub state: TrainingJobState,
    pub revision: u64,
    pub dataset_path: Option<String>,
    pub progress: Option<TrainingProgress>,
    pub active_model: Option<ModelSummary>,
    pub candidate_model_id: Option<String>,
    pub provenance: Option<ProvenanceSummary>,
    pub cancel_requested: bool,
    pub outcome: Option<TrainingOutcomeView>,
    pub diagnostic: Option<Diagnostic>,
}
impl TrainingJob {
    fn summary(&self) -> TrainingJobSummary {
        TrainingJobSummary {
            job_id: self.job_id.clone(),
            request_id: self.request_id.clone(),
            state: self.state,
            revision: self.revision,
            progress: self.progress.clone(),
            cancel_requested: self.cancel_requested,
        }
    }
    fn advance(&mut self) -> Result<(), HomeError> {
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or_else(|| invalid_artifact("manual_job_revision_overflow"))?;
        Ok(())
    }
}
/// Explicit consent is passed intact to the core authorization gate.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ManualTrainRequest {
    pub dataset: DatasetRequest,
    pub request_id: String,
    pub synthetic_activation: SyntheticActivation,
}
impl ManualTrainRequest {
    /// Constructs an operation with explicit typed activation consent.
    #[must_use]
    pub fn new(
        dataset: DatasetRequest,
        request_id: String,
        synthetic_activation: SyntheticActivation,
    ) -> Self {
        Self {
            dataset,
            request_id,
            synthetic_activation,
        }
    }
}
/// Evaluation reads the test partition and never changes the active model.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct EvaluateRequest {
    pub dataset: DatasetRequest,
}
impl EvaluateRequest {
    /// Selects the dataset to evaluate with the currently pinned model.
    #[must_use]
    pub fn new(dataset: DatasetRequest) -> Self {
        Self { dataset }
    }
}
/// Pinned inference service and immutable identity for a cycle/job.
#[derive(Clone)]
#[non_exhaustive]
pub struct PinnedClassifier {
    pub snapshot: ModelSnapshot,
    /// SHA-256 of the immutable manifest, including thresholds and report identity.
    pub manifest_sha256: String,
    pub service: Arc<ClassifierService>,
}
/// Evaluation report, never authorization for promotion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct EvaluationReport {
    pub schema_version: String,
    pub model: ModelSummary,
    pub dataset: DatasetSummary,
    pub split: String,
    pub representative_policy: String,
    pub metrics: EvaluationMetrics,
    pub admission_thresholds_enforced: bool,
}
/// Named rollback result from the core history-gated path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RollbackResult {
    pub schema_version: String,
    pub previous_model_id: Option<String>,
    pub active_model_id: String,
    pub generation: u64,
    pub reason: String,
    pub provenance: ProvenanceSummary,
}
/// Typed management failures; parse, operational and request errors stay distinct.
#[derive(Debug)]
#[non_exhaustive]
pub enum HomeError {
    Dataset(DatasetError),
    Classifier(ClassifierError),
    InvalidRequest { code: String },
    NotFound { job_id: String },
    RequestConflict,
}
impl fmt::Display for HomeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Dataset(e) => e.fmt(f),
            Self::Classifier(e) => e.fmt(f),
            Self::InvalidRequest { code } => write!(f, "invalid classifier request: {code}"),
            Self::NotFound { .. } => f.write_str("classifier job not found"),
            Self::RequestConflict => {
                f.write_str("classifier request ID already has different inputs")
            }
        }
    }
}
impl std::error::Error for HomeError {}
impl From<ClassifierError> for HomeError {
    fn from(value: ClassifierError) -> Self {
        Self::Classifier(value)
    }
}
impl From<DatasetError> for HomeError {
    fn from(value: DatasetError) -> Self {
        Self::Dataset(value)
    }
}
fn invalid_artifact(reason: &str) -> HomeError {
    ClassifierError::InvalidArtifact(reason.into()).into()
}
fn io_error(error: std::io::Error) -> HomeError {
    ClassifierError::Io(error.kind().to_string()).into()
}
fn diagnostic(code: &str, message: &str, retryable: bool) -> Diagnostic {
    Diagnostic {
        code: code.into(),
        message: message.into(),
        retryable,
    }
}

#[derive(Default)]
struct ProbeCache {
    key: String,
    readiness: Option<Readiness>,
    refreshing: bool,
}
struct OwnedJob {
    job_id: String,
    cancel: Arc<AtomicBool>,
    task: JoinHandle<Result<(), HomeError>>,
    #[cfg(test)]
    reap_entered: Option<Arc<tokio::sync::Notify>>,
}
struct HomeInner {
    home: PathBuf,
    cache: Arc<Mutex<ProbeCache>>,
    refresh: Mutex<Option<JoinHandle<()>>>,
    owner: tokio::sync::Mutex<Option<OwnedJob>>,
    operation: tokio::sync::Mutex<()>,
    pin_operation: tokio::sync::Mutex<()>,
    pins: Mutex<BTreeMap<String, Weak<ClassifierService>>>,
    active_pin: Mutex<Option<Arc<ClassifierService>>>,
    cancel: Arc<AtomicBool>,
    #[cfg(test)]
    dispatch_barrier: Mutex<Option<Arc<DispatchBarrier>>>,
}
#[cfg(test)]
struct DispatchBarrier {
    prepared: tokio::sync::Notify,
    proceed: tokio::sync::Notify,
    acquiring_owner: tokio::sync::Notify,
    dispatched: AtomicBool,
}
impl Drop for HomeInner {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
        if let Some(owner) = self.owner.get_mut() {
            owner.cancel.store(true, Ordering::Release);
        }
    }
}
/// Lazy reusable adapter. Construction performs no I/O, creates no directory,
/// and reserves no JavaScript runtime.
#[derive(Clone)]
pub struct ClassifierHome {
    inner: Arc<HomeInner>,
}
impl ClassifierHome {
    /// Revalidates an accepted immutable pin/report/manifest without restoring
    /// JavaScript. Used for already-current and derivative-only maintenance.
    pub(crate) async fn validate_saved_pin_integrity(
        &self,
        snapshot: &ModelSnapshot,
        expected_manifest_sha256: &str,
    ) -> Result<(), ClassifierError> {
        if !cfg!(feature = "classifier") {
            return Err(ClassifierError::Unavailable);
        }
        if expected_manifest_sha256.len() != 64
            || !expected_manifest_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(ClassifierError::InvalidArtifact(
                "invalid_saved_manifest_hash".into(),
            ));
        }
        let root = self.inner.home.clone();
        let snapshot = snapshot.clone();
        let expected = expected_manifest_sha256.to_owned();
        blocking(move || {
            #[cfg(feature = "classifier")]
            let actual = if snapshot.algorithm_version == super::ZERO_SHOT_ALGORITHM_VERSION {
                super::bert::Bundle::load(&root, Some(snapshot))?.manifest_sha256
            } else {
                load_pin(&root, Some(snapshot))?.3
            };
            #[cfg(not(feature = "classifier"))]
            let actual = load_pin(&root, Some(snapshot))?.3;
            if actual != expected {
                return Err(invalid_artifact("saved_manifest_hash_mismatch"));
            }
            Ok(())
        })
        .await
        .map_err(classifier_error)
    }

    /// Runtime home bound to this adapter. Reading it performs no I/O.
    #[must_use]
    pub fn home(&self) -> &Path {
        &self.inner.home
    }

    /// Constructs a lazy home adapter; runtime paths should already be absolute.
    #[must_use]
    pub fn new(home: PathBuf) -> Self {
        Self {
            inner: Arc::new(HomeInner {
                home,
                cache: Arc::new(Mutex::new(ProbeCache::default())),
                refresh: Mutex::new(None),
                owner: tokio::sync::Mutex::new(None),
                operation: tokio::sync::Mutex::new(()),
                pin_operation: tokio::sync::Mutex::new(()),
                pins: Mutex::new(BTreeMap::new()),
                active_pin: Mutex::new(None),
                cancel: Arc::new(AtomicBool::new(false)),
                #[cfg(test)]
                dispatch_barrier: Mutex::new(None),
            }),
        }
    }

    /// Fast cached readiness. Cold/changed artifacts return verification_pending
    /// while one retained background integrity refresh checks the actual bytes.
    pub async fn probe(&self, request: &DatasetRequest) -> Readiness {
        let home = self.inner.home.clone();
        let request = request.clone();
        let prepared = blocking(move || prepare_probe(&home, &request)).await;
        let (key, pending, selected) = match prepared {
            Ok(result) => result,
            Err(_) => return unavailable_readiness("probe_failed"),
        };
        let mut cache = match self.inner.cache.lock() {
            Ok(cache) => cache,
            Err(_) => return unavailable_readiness("probe_cache_poisoned"),
        };
        if cache.key == key {
            if let Some(value) = &cache.readiness {
                return value.clone();
            }
        }
        if !cache.refreshing {
            cache.refreshing = true;
            let home = self.inner.home.clone();
            let target = self.inner.cache.clone();
            let cancel = self.inner.cancel.clone();
            let task_key = key.clone();
            let task_pending = pending.clone();
            let task = tokio::spawn(async move {
                let result = blocking(move || refresh_probe(&home, selected, task_pending)).await;
                if !cancel.load(Ordering::Acquire) {
                    if let Ok(mut cache) = target.lock() {
                        cache.refreshing = false;
                        cache.key = task_key;
                        cache.readiness =
                            Some(result.unwrap_or_else(|_| {
                                unavailable_readiness("integrity_refresh_failed")
                            }));
                    }
                }
            });
            if let Ok(mut previous) = self.inner.refresh.lock() {
                *previous = Some(task);
            }
        }
        pending
    }

    /// Explicit full integrity refresh without CSV parsing or JavaScript startup.
    pub async fn refresh_integrity(
        &self,
        request: &DatasetRequest,
    ) -> Result<Readiness, HomeError> {
        let home = self.inner.home.clone();
        let request = request.clone();
        let (key, value) = blocking(move || {
            let (key, pending, selected) = prepare_probe(&home, &request)?;
            Ok((key, refresh_probe(&home, selected, pending)?))
        })
        .await?;
        let mut cache = self.inner.cache.lock().map_err(|_| ClassifierError::Busy)?;
        cache.key = key;
        cache.readiness = Some(value.clone());
        Ok(value)
    }

    /// Pins the current or exact saved immutable model. No fallback is allowed
    /// when a requested identity is absent or incompatible.
    pub async fn pin(
        &self,
        requested: Option<&ModelSnapshot>,
    ) -> Result<PinnedClassifier, ClassifierError> {
        if requested.is_some_and(|snapshot| {
            snapshot.algorithm_version == super::ZERO_SHOT_ALGORITHM_VERSION
        }) {
            return self.pin_atlas(requested).await;
        }
        self.pin_natural(requested).await
    }

    /// Pins live Atlas inference, or the exact saved backend for a resumed run.
    pub async fn pin_atlas(
        &self,
        requested: Option<&ModelSnapshot>,
    ) -> Result<PinnedClassifier, ClassifierError> {
        if requested.is_some_and(|snapshot| {
            snapshot.algorithm_version != super::ZERO_SHOT_ALGORITHM_VERSION
        }) {
            return self.pin_natural(requested).await;
        }
        #[cfg(not(feature = "classifier"))]
        return Err(ClassifierError::Unavailable);
        #[cfg(feature = "classifier")]
        {
            let _serial = self.inner.pin_operation.lock().await;
            let control = super::WorkControl {
                cancel: self.inner.cancel.clone(),
                ..super::WorkControl::new()
            };
            control.check_active()?;
            let home = self.inner.home.clone();
            let current = requested.is_none();
            let requested = requested.cloned();
            let bundle = blocking(move || Ok(super::bert::Bundle::load(&home, requested)?))
                .await
                .map_err(classifier_error)?;
            let snapshot = bundle.snapshot.clone();
            let manifest_sha256 = bundle.manifest_sha256.clone();
            let key = format!(
                "{}:{}:{}",
                snapshot.model_id, snapshot.artifact_sha256, manifest_sha256
            );
            {
                let mut pins = self.inner.pins.lock().map_err(|_| ClassifierError::Busy)?;
                if let Some(service) = pins.get(&key).and_then(Weak::upgrade) {
                    let snapshot = if current {
                        service
                            .pinned_model()
                            .cloned()
                            .ok_or(ClassifierError::Unavailable)?
                    } else {
                        snapshot
                    };
                    return Ok(PinnedClassifier {
                        snapshot,
                        manifest_sha256,
                        service,
                    });
                }
                pins.retain(|_, value| value.strong_count() > 0);
                if pins.len() >= 3 {
                    return Err(ClassifierError::Busy);
                }
            }
            let runtime = tokio::runtime::Handle::current();
            let service = blocking(move || Ok(Arc::new(bundle.start(runtime, &control)?)))
                .await
                .map_err(classifier_error)?;
            self.inner
                .pins
                .lock()
                .map_err(|_| ClassifierError::Busy)?
                .insert(key, Arc::downgrade(&service));
            *self
                .inner
                .active_pin
                .lock()
                .map_err(|_| ClassifierError::Busy)? = Some(service.clone());
            Ok(PinnedClassifier {
                snapshot,
                manifest_sha256,
                service,
            })
        }
    }

    async fn pin_natural(
        &self,
        requested: Option<&ModelSnapshot>,
    ) -> Result<PinnedClassifier, ClassifierError> {
        if !cfg!(feature = "classifier") {
            return Err(ClassifierError::Unavailable);
        }
        // Coalesce concurrent cold pins before constructing a native runtime.
        let _serial = self.inner.pin_operation.lock().await;
        if self.inner.cancel.load(Ordering::Acquire) {
            return Err(ClassifierError::Cancelled);
        }
        let home = self.inner.home.clone();
        let requested = requested.cloned();
        let (snapshot, artifact, report, manifest_sha256) =
            blocking(move || load_pin(&home, requested))
                .await
                .map_err(classifier_error)?;
        let key = format!(
            "{}:{}:{}",
            snapshot.model_id, snapshot.artifact_sha256, manifest_sha256
        );
        {
            let pins = self.inner.pins.lock().map_err(|_| ClassifierError::Busy)?;
            if let Some(service) = pins.get(&key).and_then(Weak::upgrade) {
                return Ok(PinnedClassifier {
                    snapshot,
                    manifest_sha256,
                    service,
                });
            }
        }
        let saved = snapshot.clone();
        let service = blocking(move || {
            Ok(Arc::new(ClassifierService::from_snapshot(
                saved,
                artifact,
                Box::new(report.gate_set),
            )?))
        })
        .await
        .map_err(classifier_error)?;
        let mut pins = self.inner.pins.lock().map_err(|_| ClassifierError::Busy)?;
        pins.retain(|_, value| value.strong_count() > 0);
        if pins.len() >= 3 && !pins.contains_key(&key) {
            return Err(ClassifierError::Busy);
        }
        pins.insert(key, Arc::downgrade(&service));
        *self
            .inner
            .active_pin
            .lock()
            .map_err(|_| ClassifierError::Busy)? = Some(service.clone());
        Ok(PinnedClassifier {
            snapshot,
            manifest_sha256,
            service,
        })
    }

    /// Accepts one manual training writer promptly and retains its owner task.
    /// The immutable request ID is replayed rather than retrained.
    pub async fn start_training(
        &self,
        request: ManualTrainRequest,
    ) -> Result<JobAccepted, HomeError> {
        if !cfg!(feature = "classifier") {
            return Err(ClassifierError::Unavailable.into());
        }
        if self.inner.cancel.load(Ordering::Acquire) {
            return Err(ClassifierError::Cancelled.into());
        }
        validate_request_id(&request.request_id)?;
        let _serial = self.inner.operation.lock().await;
        {
            let mut owner = self.inner.owner.lock().await;
            if let Some(existing) = owner.as_mut() {
                let terminal = if existing.task.is_finished() {
                    true
                } else {
                    let path = artifact::root_for(&self.inner.home)?
                        .join("manual-jobs")
                        .join(&existing.job_id);
                    let id = existing.job_id.clone();
                    blocking(move || Ok(read_job(&path, &id)?.state.terminal())).await?
                };
                if terminal {
                    // Retain ownership across cancellation and timeout. A
                    // terminal result may precede the monitor's final join.
                    #[cfg(test)]
                    if let Some(entered) = &existing.reap_entered {
                        entered.notify_one();
                    }
                    let joined = tokio::time::timeout(Duration::from_secs(5), &mut existing.task)
                        .await
                        .map_err(|_| ClassifierError::Timeout)?;
                    owner.take();
                    joined.map_err(join_error)??;
                } else if existing.job_id != job_id(&request.request_id) {
                    return Err(ClassifierError::Busy.into());
                }
            }
        }
        if self.inner.cancel.load(Ordering::Acquire) {
            return Err(ClassifierError::Cancelled.into());
        }
        let home = self.inner.home.clone();
        let prepared = blocking(move || prepare_training(&home, request)).await?;
        match prepared {
            PreparedTraining::Replay(job) => Ok(accepted(&job, true)),
            PreparedTraining::Start(prepared) => {
                let TrainingStart {
                    store,
                    lease,
                    owner_lock,
                    selected,
                    request,
                    job,
                } = *prepared;
                #[cfg(test)]
                let barrier = self
                    .inner
                    .dispatch_barrier
                    .lock()
                    .map_err(|_| ClassifierError::Busy)?
                    .clone();
                #[cfg(test)]
                if let Some(barrier) = &barrier {
                    barrier.prepared.notify_one();
                    barrier.proceed.notified().await;
                    barrier.acquiring_owner.notify_one();
                }
                // Acquire the handle slot before spawning. Cancellation at this
                // await drops the prepared lease without dispatching fitting.
                let mut owner = self.inner.owner.lock().await;
                if self.inner.cancel.load(Ordering::Acquire) {
                    return Err(ClassifierError::Cancelled.into());
                }
                let answer = accepted(&job, false);
                let cancel = Arc::new(AtomicBool::new(false));
                let worker_cancel = cancel.clone();
                let owner_cancel = cancel.clone();
                let home_cancel = self.inner.cancel.clone();
                let monitor_path = store.root().join("manual-jobs").join(&job.job_id);
                let monitor_id = job.job_id.clone();
                let owned_job_id = job.job_id.clone();
                #[cfg(test)]
                if let Some(barrier) = &barrier {
                    barrier.dispatched.store(true, Ordering::Release);
                }
                let task = tokio::spawn(async move {
                    let worker = tokio::task::spawn_blocking(move || {
                        run_training(
                            store,
                            lease,
                            owner_lock,
                            selected,
                            request,
                            job,
                            worker_cancel,
                        )
                    });
                    tokio::pin!(worker);
                    let mut interval = tokio::time::interval(Duration::from_millis(100));
                    loop {
                        tokio::select! {
                            result = &mut worker => return result.map_err(join_error)?,
                            _ = interval.tick() => {
                                if home_cancel.load(Ordering::Acquire) { cancel.store(true, Ordering::Release); }
                                let path = monitor_path.clone();
                                let id = monitor_id.clone();
                                let requested = blocking(move || cancel_requested(&path, &id)).await;
                                match requested { Ok(true) => cancel.store(true, Ordering::Release), Err(_) => cancel.store(true, Ordering::Release), _ => {} }
                            }
                        }
                    }
                });
                // Keep a separate token because the task owns its own clone.
                *owner = Some(OwnedJob {
                    job_id: owned_job_id,
                    cancel: owner_cancel,
                    task,
                    #[cfg(test)]
                    reap_entered: None,
                });
                Ok(answer)
            }
        }
    }

    /// Reads one exact job; orphan settlement requires acquiring the shared
    /// writer and never interprets PID or age as lock ownership.
    pub async fn job_detail(&self, job_id: &str) -> Result<TrainingJob, HomeError> {
        validate_job_id(job_id)?;
        let home = self.inner.home.clone();
        let id = job_id.to_string();
        blocking(move || job_detail_sync(&home, &id, true)).await
    }

    /// Requests cancellation of this home's exact manual job, idempotently.
    pub async fn cancel_training(&self, job_id: &str) -> Result<TrainingJob, HomeError> {
        validate_job_id(job_id)?;
        let home = self.inner.home.clone();
        let id = job_id.to_string();
        blocking(move || {
            let mut job = job_detail_sync(&home, &id, true)?;
            if !job.state.terminal() {
                let path = artifact::root_for(&home)?.join("manual-jobs").join(&id);
                write_json(
                    &path.join("cancel.json"),
                    &CancelMarker {
                        schema_version: "classifier-cancel-v1".into(),
                        job_id: id.clone(),
                    },
                    1024,
                )?;
                job.cancel_requested = true;
                job.state = TrainingJobState::Cancelling;
            }
            Ok(job)
        })
        .await
    }

    /// Evaluates test-family representatives with frozen current gates.
    pub async fn evaluate_file(
        &self,
        request: EvaluateRequest,
    ) -> Result<EvaluationReport, HomeError> {
        let pinned = self.pin(None).await?;
        let home = self.inner.home.clone();
        blocking(move || evaluate_sync(&home, request.dataset, pinned)).await
    }

    /// Rolls back only to a previously activated and revalidated named model.
    pub async fn rollback_to(&self, model: &ModelId) -> Result<RollbackResult, HomeError> {
        if !cfg!(feature = "classifier") {
            return Err(ClassifierError::Unavailable.into());
        }
        let home = self.inner.home.clone();
        let model = model.clone();
        let result = blocking(move || {
            let store = ArtifactStore::new(&home)?;
            let lease = store.try_writer()?;
            let previous = store.read_active()?;
            let active = store.rollback_to(&model, &lease)?;
            Ok(RollbackResult {
                schema_version: "classifier-rollback-v1".into(),
                previous_model_id: previous.map(|p| p.model_id.as_str().into()),
                active_model_id: active.model_id.as_str().into(),
                generation: active.activation_generation,
                reason: "manual_rollback".into(),
                provenance: (&active.provenance).into(),
            })
        })
        .await?;
        self.invalidate()?;
        Ok(result)
    }

    /// Cancels only owned work and joins its task. An unfinished handle is
    /// retained on timeout; no unrelated writer is killed or lock removed.
    pub async fn shutdown(&self) -> Result<(), HomeError> {
        self.inner.cancel.store(true, Ordering::Release);
        let mut owner = self.inner.owner.lock().await;
        if let Some(work) = owner.as_mut() {
            work.cancel.store(true, Ordering::Release);
            match tokio::time::timeout(Duration::from_secs(5), &mut work.task).await {
                Ok(result) => {
                    result.map_err(join_error)??;
                    owner.take();
                }
                Err(_) => return Err(ClassifierError::Timeout.into()),
            }
        }
        let refresh = self
            .inner
            .refresh
            .lock()
            .map_err(|_| ClassifierError::Busy)?
            .take();
        if let Some(mut task) = refresh {
            if tokio::time::timeout(Duration::from_secs(5), &mut task)
                .await
                .is_err()
            {
                *self
                    .inner
                    .refresh
                    .lock()
                    .map_err(|_| ClassifierError::Busy)? = Some(task);
                return Err(ClassifierError::Timeout.into());
            }
        }
        // Release only this home's retained cache. Externally pinned callers
        // keep their immutable service alive until their own work finishes.
        let _pins = tokio::time::timeout(Duration::from_secs(5), self.inner.pin_operation.lock())
            .await
            .map_err(|_| ClassifierError::Timeout)?;
        let retained = self
            .inner
            .active_pin
            .lock()
            .map_err(|_| ClassifierError::Busy)?
            .take();
        self.inner
            .pins
            .lock()
            .map_err(|_| ClassifierError::Busy)?
            .clear();
        // The last service reference joins its native worker on drop.
        blocking(move || {
            drop(retained);
            Ok(())
        })
        .await?;
        Ok(())
    }
    fn invalidate(&self) -> Result<(), HomeError> {
        self.inner
            .cache
            .lock()
            .map_err(|_| ClassifierError::Busy)?
            .readiness = None;
        Ok(())
    }
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, HomeError> + Send + 'static,
) -> Result<T, HomeError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(join_error)?
}
fn join_error(_: tokio::task::JoinError) -> HomeError {
    ClassifierError::WorkerStopped.into()
}
fn classifier_error(error: HomeError) -> ClassifierError {
    match error {
        HomeError::Classifier(error) => error,
        _ => ClassifierError::InvalidArtifact("classifier_home_operation_failed".into()),
    }
}
fn unavailable_readiness(code: &str) -> Readiness {
    Readiness {
        schema_version: "classifier-readiness-v1".into(),
        state: ClassifierStatus::Unavailable,
        active_model_state: ClassifierStatus::Unavailable,
        feature_enabled: cfg!(feature = "classifier"),
        can_infer: false,
        can_train: false,
        integrity: "pending".into(),
        retraining_available: false,
        dataset: DatasetReadiness {
            path: None,
            source: None,
            availability: "missing".into(),
            change: "unknown".into(),
            hash_check: "pending".into(),
            sha256: None,
            diagnostic: None,
        },
        active_model: None,
        training: None,
        diagnostic: Some(diagnostic(code, code, true)),
    }
}
fn prepare_probe(
    home: &Path,
    request: &DatasetRequest,
) -> Result<(String, Readiness, Option<DatasetResolution>), HomeError> {
    let mut value = unavailable_readiness("verification_pending");
    if !cfg!(feature = "classifier") {
        value.diagnostic = Some(diagnostic(
            "feature_disabled",
            "Classifier feature unavailable",
            false,
        ));
        return Ok(("disabled".into(), value, None));
    }
    let root = artifact::root_for(home)?;
    let selected = match dataset::resolve_dataset(request) {
        Ok(selected) => {
            value.dataset.path = Some(selected.path.to_string_lossy().into_owned());
            value.dataset.source = Some(selected.candidate);
            value.dataset.availability = "present".into();
            value.can_train = true;
            Some(selected)
        }
        Err(error) => {
            let missing = matches!(error, DatasetError::NotFound { .. });
            value.dataset.availability = if missing { "missing" } else { "invalid" }.into();
            value.dataset.diagnostic = Some(diagnostic(
                if missing {
                    "dataset_missing"
                } else {
                    "dataset_invalid"
                },
                &error.to_string(),
                !missing,
            ));
            None
        }
    };
    let active = root.join("active.json");
    let mut fingerprints = vec![file_signature(&active)];
    if !active.try_exists().map_err(io_error)? {
        value.state = ClassifierStatus::Missing;
        value.active_model_state = ClassifierStatus::Missing;
        value.integrity = "verified".into();
        value.diagnostic = None;
        value.retraining_available = selected.is_some();
    } else if let Ok(Some(manifest)) =
        read_optional_json::<ModelManifest>(&active, artifact::MAX_MANIFEST_BYTES)
    {
        for file in [
            artifact::MODEL_MANIFEST_FILE,
            artifact::MODEL_PAYLOAD_FILE,
            artifact::MODEL_REPORT_FILE,
        ] {
            let path = root
                .join("models")
                .join(manifest.model_id.as_str())
                .join(file);
            fingerprints.push(file_signature(&path));
        }
    }
    fingerprints.push(
        selected
            .as_ref()
            .map(|s| format!("{:?}:{:?}", s.path, s.fingerprint))
            .unwrap_or_else(|| value.dataset.availability.clone()),
    );
    let active_job_path = root.join("manual-active.json");
    fingerprints.push(file_signature(&active_job_path));
    if let Ok(Some(active_job)) = read_optional_json::<ActiveJob>(&active_job_path, 64 * 1024) {
        if validate_job_id(&active_job.job_id).is_ok() {
            fingerprints.push(file_signature(
                &root
                    .join("manual-jobs")
                    .join(&active_job.job_id)
                    .join("progress.json"),
            ));
        }
    }
    if let Ok(Some(job)) = read_live_job(home) {
        value.state = ClassifierStatus::Training;
        value.can_train = false;
        value.training = Some(job.summary());
    }
    // A durable terminal result precedes owner teardown by contract. The
    // writer can still be held after read_live_job stops returning that job.
    let writer_busy = lock_held(&root.join("writer.lock"))?;
    fingerprints.push(format!("writer-busy:{writer_busy}"));
    if writer_busy {
        value.can_train = false;
        value.diagnostic = Some(diagnostic(
            "writer_busy",
            "Classifier writer is completing owned work; check readiness again",
            true,
        ));
    }
    let key = artifact::sha256_hex(fingerprints.join("\0").as_bytes());
    Ok((key, value, selected))
}
fn file_signature(path: &Path) -> String {
    match fs::metadata(path) {
        Ok(meta) => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                format!(
                    "{}:{:?}:{}:{}:{}:{}",
                    meta.len(),
                    meta.modified().ok(),
                    meta.dev(),
                    meta.ino(),
                    meta.ctime(),
                    meta.ctime_nsec()
                )
            }
            #[cfg(not(unix))]
            {
                format!("{}:{:?}", meta.len(), meta.modified().ok())
            }
        }
        Err(error) => format!("{:?}", error.kind()),
    }
}
fn refresh_probe(
    home: &Path,
    selected: Option<DatasetResolution>,
    mut value: Readiness,
) -> Result<Readiness, HomeError> {
    if !cfg!(feature = "classifier") {
        return Ok(value);
    }
    let root = artifact::root_for(home)?;
    let dataset_available = selected.is_some();
    let active_path = root.join("active.json");
    if active_path.try_exists().map_err(io_error)? {
        let outcome = ArtifactStore::new(home).and_then(|store| {
            let manifest = store.read_active_integrity()?;
            manifest
                .map(|m| {
                    let report: TrainingReport =
                        serde_json::from_str(&store.read_report(&m.model_id)?).map_err(|_| {
                            ClassifierError::InvalidArtifact("invalid_training_report".into())
                        })?;
                    Ok((m, report))
                })
                .transpose()
        });
        match outcome {
            Ok(Some((manifest, report))) => {
                value.active_model = Some(model_summary(&manifest, Some(&report)));
                value.active_model_state = ClassifierStatus::Ready;
                value.can_infer = true;
                value.integrity = "verified".into();
                value.diagnostic = None;
            }
            Ok(None) => {
                value.active_model_state = ClassifierStatus::Missing;
                value.integrity = "verified".into();
                value.diagnostic = None;
            }
            Err(error) => {
                value.active_model_state = match error {
                    ClassifierError::Incompatible(_) => ClassifierStatus::Incompatible,
                    ClassifierError::Io(_) | ClassifierError::Storage(_) => {
                        ClassifierStatus::Unavailable
                    }
                    _ => ClassifierStatus::Invalid,
                };
                value.integrity = "failed".into();
                value.diagnostic = Some(diagnostic(
                    "artifact_validation_failed",
                    &error.to_string(),
                    true,
                ));
            }
        }
    }
    if let Some(selected) = selected {
        match hash_dataset(&selected) {
            Ok(hash) => {
                value.dataset.hash_check = "verified".into();
                value.dataset.change = match value.active_model.as_ref() {
                    Some(model)
                        if model
                            .provenance
                            .seed_dataset_sha256
                            .as_deref()
                            .unwrap_or(&model.provenance.dataset_sha256)
                            == hash =>
                    {
                        "unchanged"
                    }
                    Some(_) => "changed",
                    None => "unknown",
                }
                .into();
                value.dataset.sha256 = Some(hash);
                value.retraining_available =
                    value.active_model.is_none() || value.dataset.change == "changed";
            }
            Err(error) => {
                value.dataset.hash_check = "failed".into();
                value.dataset.diagnostic =
                    Some(diagnostic("dataset_hash_failed", &error.to_string(), true));
                value.retraining_available = false;
            }
        }
    }
    value.state = if value.training.is_some() {
        ClassifierStatus::Training
    } else {
        value.active_model_state
    };
    // Preserve a retryable teardown signal after active-artifact validation
    // replaced the preliminary diagnostic. This read never creates a lock.
    let writer_busy = lock_held(&root.join("writer.lock"))?;
    value.can_train = dataset_available && value.training.is_none() && !writer_busy;
    if writer_busy {
        value.diagnostic = Some(diagnostic(
            "writer_busy",
            "Classifier writer is completing owned work; check readiness again",
            true,
        ));
    }
    Ok(value)
}
fn hash_dataset(selected: &DatasetResolution) -> Result<String, HomeError> {
    let mut file = File::open(&selected.path).map_err(io_error)?;
    let before = file_signature(&selected.path);
    let mut hash = Sha256::new();
    let mut total = 0;
    let mut buffer = [0; 8192];
    loop {
        let n = file.read(&mut buffer).map_err(io_error)?;
        if n == 0 {
            break;
        }
        total += n as u64;
        if total > dataset::MAX_CSV_BYTES {
            return Err(DatasetError::TooLarge {
                path: selected.path.clone(),
                limit: dataset::MAX_CSV_BYTES,
                kind: "file_bytes".into(),
            }
            .into());
        }
        hash.update(&buffer[..n]);
    }
    if before != file_signature(&selected.path) {
        return Err(DatasetError::ChangedDuringRead {
            path: selected.path.clone(),
        }
        .into());
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn model_summary(manifest: &ModelManifest, report: Option<&TrainingReport>) -> ModelSummary {
    let approved = manifest.activation_reason == ActivationReason::AutomaticPromotion
        && matches!(manifest.provenance, Provenance::ReviewedReal { .. });
    ModelSummary {
        model_id: manifest.model_id.as_str().into(),
        generation: manifest.activation_generation,
        activated_at: manifest.activated_at.clone(),
        artifact_sha256: manifest.model_sha256.clone(),
        manifest_sha256: manifest.manifest_sha256.clone(),
        taxonomy_version: manifest.taxonomy_version.clone(),
        domains: manifest.domains.clone(),
        algorithm_version: manifest.algorithm_version.clone(),
        preprocessing_version: manifest.preprocessing_version.clone(),
        policy_version: manifest.admission_policy_version.clone(),
        receipt_schema_version: super::RECEIPT_SCHEMA_VERSION.into(),
        calibration_version: calibration::CALIBRATION_VERSION.into(),
        provenance: (&manifest.provenance).into(),
        quality: QualitySummary {
            production_approved: approved,
            reason_codes: if approved {
                Vec::new()
            } else {
                vec!["insufficient_quality_evidence".into()]
            },
            domain_acceptance: report.map(|r| r.domain_decisions.clone()),
        },
    }
}
fn snapshot_of(manifest: &ModelManifest) -> ModelSnapshot {
    ModelSnapshot {
        model_id: manifest.model_id.as_str().into(),
        artifact_sha256: manifest.model_sha256.clone(),
        taxonomy_version: manifest.taxonomy_version.clone(),
        algorithm_version: manifest.algorithm_version.clone(),
        preprocessing_version: manifest.preprocessing_version.clone(),
        policy_version: manifest.admission_policy_version.clone(),
        activated_at: manifest.activated_at.clone(),
        provenance: manifest.provenance.clone(),
    }
}
fn load_pin(
    home: &Path,
    requested: Option<ModelSnapshot>,
) -> Result<
    (
        ModelSnapshot,
        artifact::ValidatedArtifact,
        TrainingReport,
        String,
    ),
    HomeError,
> {
    if !artifact::root_for(home)?.is_dir() {
        return Err(ClassifierError::Unavailable.into());
    }
    let store = ArtifactStore::new(home)?;
    let snapshot = match requested {
        Some(snapshot) => snapshot,
        None => snapshot_of(&store.read_active()?.ok_or(ClassifierError::Unavailable)?),
    };
    let id = ModelId::parse(&snapshot.model_id)
        .ok_or_else(|| invalid_artifact("invalid_pinned_model_id"))?;
    let artifact = store.validate_model_integrity(&id)?;
    let manifest = &artifact.manifest;
    if snapshot.artifact_sha256 != manifest.model_sha256
        || snapshot.taxonomy_version != manifest.taxonomy_version
        || snapshot.algorithm_version != manifest.algorithm_version
        || snapshot.preprocessing_version != manifest.preprocessing_version
        || snapshot.policy_version != manifest.admission_policy_version
        || snapshot.provenance != manifest.provenance
        || chrono::DateTime::parse_from_rfc3339(&snapshot.activated_at).is_err()
    {
        return Err(invalid_artifact("pinned_identity_mismatch"));
    }
    let report = serde_json::from_str(&store.read_report(&id)?)
        .map_err(|_| invalid_artifact("invalid_training_report"))?;
    let manifest_sha256 = artifact::sha256_hex(&store.read_manifest_raw(&id)?);
    Ok((snapshot, artifact, report, manifest_sha256))
}

fn validate_request_id(id: &str) -> Result<(), HomeError> {
    if id.is_empty()
        || id.len() > 128
        || matches!(id, "." | "..")
        || !id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_.-".contains(&c))
    {
        return Err(HomeError::InvalidRequest {
            code: "invalid_request_id".into(),
        });
    }
    Ok(())
}
fn job_id(request: &str) -> String {
    format!(
        "classifier-{}",
        artifact::sha256_hex(format!("classifier-manual-job-v1\0{request}").as_bytes())
    )
}
fn validate_job_id(id: &str) -> Result<(), HomeError> {
    if id.len() != 75
        || !id.starts_with("classifier-")
        || !id[11..]
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        return Err(HomeError::NotFound { job_id: id.into() });
    }
    Ok(())
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SelectionIntent {
    explicit_file: Option<PathBuf>,
    environment_file: Option<PathBuf>,
    #[serde(default)]
    configured_file: Option<PathBuf>,
    cwd: PathBuf,
    executable: PathBuf,
    home: PathBuf,
}
impl From<&DatasetRequest> for SelectionIntent {
    fn from(value: &DatasetRequest) -> Self {
        Self {
            explicit_file: value.explicit_file.clone(),
            environment_file: value.environment_file.clone(),
            configured_file: value.configured_file.clone(),
            cwd: value.cwd.clone(),
            executable: value.executable.clone(),
            home: value.home.clone(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredRequest {
    schema_version: String,
    request_id: String,
    job_id: String,
    selected_path: PathBuf,
    selection_intent: SelectionIntent,
    synthetic_activation: SyntheticActivation,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CancelMarker {
    schema_version: String,
    job_id: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ActiveJob {
    schema_version: String,
    job_id: String,
    request_id: String,
}

enum PreparedTraining {
    Replay(Box<TrainingJob>),
    Start(Box<TrainingStart>),
}
struct TrainingStart {
    store: ArtifactStore,
    lease: Arc<artifact::WriterLease>,
    owner_lock: File,
    selected: DatasetResolution,
    request: StoredRequest,
    job: TrainingJob,
}
fn accepted(job: &TrainingJob, reused: bool) -> JobAccepted {
    JobAccepted {
        schema_version: "classifier-job-accepted-v1".into(),
        job_id: job.job_id.clone(),
        request_id: job.request_id.clone(),
        state: job.state,
        reused,
    }
}
fn prepare_training(
    home: &Path,
    request: ManualTrainRequest,
) -> Result<PreparedTraining, HomeError> {
    let id = job_id(&request.request_id);
    let path = artifact::root_for(home)?.join("manual-jobs").join(&id);
    let intent = SelectionIntent::from(&request.dataset);
    if let Some(stored) =
        read_optional_json::<StoredRequest>(&path.join("request.json"), 16 * 1024)?
    {
        if stored.schema_version != "classifier-manual-request-v1"
            || stored.job_id != id
            || stored.request_id != request.request_id
        {
            return Err(invalid_artifact("manual_request_identity_mismatch"));
        }
        if stored.synthetic_activation != request.synthetic_activation {
            return Err(HomeError::RequestConflict);
        }
        if stored.selection_intent == intent {
            // Replay the immutable operation even if the selected CSV has since
            // changed or disappeared. New contents require a new request ID.
            return Ok(PreparedTraining::Replay(Box::new(job_detail_sync(
                home, &id, true,
            )?)));
        }
        let selected = dataset::resolve_dataset(&request.dataset)?;
        if stored.selected_path != selected.path {
            return Err(HomeError::RequestConflict);
        }
        return Ok(PreparedTraining::Replay(Box::new(job_detail_sync(
            home, &id, true,
        )?)));
    }
    let selected = dataset::resolve_dataset(&request.dataset)?;
    let desired = StoredRequest {
        schema_version: "classifier-manual-request-v1".into(),
        request_id: request.request_id.clone(),
        job_id: id.clone(),
        selected_path: selected.path.clone(),
        selection_intent: intent,
        synthetic_activation: request.synthetic_activation,
    };
    let store = ArtifactStore::new(home)?;
    let lease = Arc::new(store.try_writer()?);
    if let Some(stored) =
        read_optional_json::<StoredRequest>(&path.join("request.json"), 16 * 1024)?
    {
        if stored != desired {
            return Err(HomeError::RequestConflict);
        }
        return Ok(PreparedTraining::Replay(Box::new(job_detail_sync(
            home, &id, false,
        )?)));
    }
    if let Some(old) =
        read_optional_json::<ActiveJob>(&store.root().join("manual-active.json"), 64 * 1024)?
    {
        let old_path = store.root().join("manual-jobs").join(&old.job_id);
        validate_job_id(&old.job_id)?;
        if let Some(mut job) =
            read_optional_json::<TrainingJob>(&old_path.join("progress.json"), 64 * 1024)?
        {
            if !job.state.terminal() {
                job.state = TrainingJobState::Interrupted;
                job.diagnostic = Some(diagnostic(
                    "worker_stopped",
                    "Prior training owner stopped; rebuild with a new request ID",
                    true,
                ));
                job.advance()?;
                write_job(&old_path, &job)?;
            }
        }
    }
    fs::create_dir_all(&path).map_err(io_error)?;
    let owner_lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path.join("owner.lock"))
        .map_err(io_error)?;
    owner_lock.try_lock().map_err(|_| ClassifierError::Busy)?;
    write_request_new(&path.join("request.json"), &desired)?;
    let job = TrainingJob {
        schema_version: "classifier-training-job-v1".into(),
        job_id: id.clone(),
        request_id: desired.request_id.clone(),
        state: TrainingJobState::Queued,
        revision: 1,
        dataset_path: Some(selected.path.to_string_lossy().into_owned()),
        progress: None,
        active_model: None,
        candidate_model_id: None,
        provenance: None,
        cancel_requested: false,
        outcome: None,
        diagnostic: None,
    };
    write_job(&path, &job)?;
    write_json(
        &store.root().join("manual-active.json"),
        &ActiveJob {
            schema_version: "classifier-manual-active-v1".into(),
            job_id: id,
            request_id: desired.request_id.clone(),
        },
        64 * 1024,
    )?;
    Ok(PreparedTraining::Start(Box::new(TrainingStart {
        store,
        lease,
        owner_lock,
        selected,
        request: desired,
        job,
    })))
}
fn run_training(
    store: ArtifactStore,
    lease: Arc<artifact::WriterLease>,
    _owner_lock: File,
    selected: DatasetResolution,
    request: StoredRequest,
    mut job: TrainingJob,
    cancel: Arc<AtomicBool>,
) -> Result<(), HomeError> {
    let path = store.root().join("manual-jobs").join(&job.job_id);
    let mut control = WorkControl::new();
    control.cancel = cancel;
    job.state = TrainingJobState::Running;
    job.advance()?;
    write_job(&path, &job)?;
    let result = train_sync(
        &store,
        lease.clone(),
        &selected,
        &request,
        &mut job,
        &control,
    );
    match result {
        Ok(outcome) => {
            job.state = TrainingJobState::Completed;
            job.candidate_model_id = Some(outcome.report.model_id.as_str().into());
            // Presentation cannot discard a valid candidate/report merely
            // because an existing pointer is invalid or from an older build.
            // This optional summary is never an activation authorization.
            let active = match store.read_active() {
                Ok(active) => active,
                Err(error) => {
                    if job.diagnostic.is_none() {
                        job.diagnostic = Some(prior_diagnostic(&error));
                    }
                    None
                }
            };
            job.active_model = active.as_ref().map(|m| model_summary(m, None));
            let state = match &outcome.activation {
                ActivationOutcome::CandidateOnly { .. } => "trained_not_activated",
                ActivationOutcome::Activated {
                    reason: ActivationReason::ManualSeed,
                    ..
                } => "manual_seed_activated",
                ActivationOutcome::Activated { .. } => "promoted",
            };
            job.outcome = Some(TrainingOutcomeView {
                schema_version: "classifier-training-outcome-v1".into(),
                job_id: job.job_id.clone(),
                state: state.into(),
                candidate_model_id: outcome.report.model_id.as_str().into(),
                active_model_id: active.map(|m| m.model_id.as_str().into()),
                activation: outcome.activation.into(),
                provenance: outcome.report.provenance.label().into(),
                admission_thresholds_enforced: true,
                report: outcome.report,
            });
        }
        Err(error) => {
            job.state = if matches!(error, HomeError::Classifier(ClassifierError::Cancelled)) {
                TrainingJobState::Cancelled
            } else {
                TrainingJobState::Failed
            };
            job.cancel_requested = control.cancel.load(Ordering::Acquire);
            job.diagnostic = Some(diagnostic("training_failed", &error.to_string(), true));
        }
    }
    job.advance()?;
    // Publish the full result before the terminal progress pointer, so readers
    // cannot observe Completed without its immutable outcome/report.
    write_json(&path.join("result.json"), &job, 1024 * 1024)?;
    write_job(&path, &job)?;
    Ok(())
}
fn prior_diagnostic(error: &ClassifierError) -> Diagnostic {
    let (code, message, retryable) = match error {
        ClassifierError::Incompatible(_) => ("prior_classifier_incompatible", "Candidate saved. The prior classifier is incompatible with this build; explicit activation must validate its repair identity.", false),
        ClassifierError::Io(_) | ClassifierError::Storage(_) => ("prior_classifier_unreadable", "Candidate saved. The prior classifier could not be read; its activation was preserved.", true),
        ClassifierError::StaleActivation => ("prior_classifier_changed", "Candidate saved. The prior classifier changed before activation; its current state was preserved.", true),
        _ => ("prior_classifier_invalid", "Candidate saved. The prior classifier identity is invalid and requires explicit recovery before activation.", false),
    };
    diagnostic(code, message, retryable)
}
fn prior_blocked_candidate(error: &ClassifierError) -> ActivationAuthorizationDecision {
    let problem = prior_diagnostic(error);
    ActivationAuthorizationDecision::RetainPrior(ActivationOutcome::CandidateOnly {
        reason_code: problem.code,
        reason: problem.message,
        failures: Vec::new(),
    })
}

fn train_sync(
    store: &ArtifactStore,
    lease: Arc<artifact::WriterLease>,
    selected: &DatasetResolution,
    request: &StoredRequest,
    job: &mut TrainingJob,
    control: &WorkControl,
) -> Result<TrainingOutcome, HomeError> {
    control.check_active()?;
    let parsed = dataset::parse_csv(selected)?;
    control.check_active()?;
    let provenance =
        Provenance::synthetic_baseline(&parsed.dataset_version, &parsed.summary.dataset_sha256);
    job.provenance = Some((&provenance).into());
    let temp = tempfile::Builder::new()
        .prefix("model-")
        .tempfile_in(store.root())
        .map_err(io_error)?;
    let model_name = temp
        .path()
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| invalid_artifact("model_id_generation_failed"))?;
    let model_id =
        ModelId::parse(model_name).ok_or_else(|| invalid_artifact("model_id_generation_failed"))?;
    let train_request = TrainRequest {
        request_id: request.request_id.clone(),
        model_id: model_id.clone(),
        provenance,
        synthetic_activation: request.synthetic_activation,
    };
    let mut worker = super::start_candidate_worker(&model_id, lease.clone(), control)?;
    let mut last = Instant::now();
    let mut last_stage = None;
    let mut progress_error = None;
    let path = store.root().join("manual-jobs").join(&job.job_id);
    let candidate = training::train_candidate(
        worker.as_mut(),
        &train_request,
        &parsed.corpus,
        control,
        &mut |progress| {
            if last_stage != Some(progress.stage) || last.elapsed() >= Duration::from_millis(250) {
                job.progress = Some(progress.clone());
                last_stage = Some(progress.stage);
                last = Instant::now();
                if let Err(error) = job.advance().and_then(|()| write_job(&path, job)) {
                    progress_error = Some(error);
                    control.cancel.store(true, Ordering::Release);
                }
            }
        },
    )?;
    drop(worker);
    if let Some(error) = progress_error {
        return Err(error);
    }
    control.check_active()?;
    let manifest = training::candidate_manifest(&parsed.corpus, &train_request, &candidate)?;
    let validated =
        store.write_candidate(&lease, &manifest, &candidate.payload, &candidate.report)?;
    job.candidate_model_id = Some(model_id.as_str().into());
    let prior = store.read_active();
    control.check_active()?;
    if cancel_requested(&path, &job.job_id)? {
        return Err(ClassifierError::Cancelled.into());
    }
    let decision = match prior {
        Ok(prior) => calibration::authorize_activation(
            &lease,
            &validated,
            &candidate.report,
            &train_request,
            None,
            prior.as_ref(),
        )?,
        Err(error) => {
            if train_request.synthetic_activation == SyntheticActivation::ExplicitOptIn {
                match store.authorize_manual_repair(
                    &lease,
                    &validated,
                    &candidate.report,
                    &train_request,
                ) {
                    Ok(decision) => decision,
                    Err(error) => {
                        job.diagnostic = Some(prior_diagnostic(&error));
                        prior_blocked_candidate(&error)
                    }
                }
            } else {
                job.diagnostic = Some(prior_diagnostic(&error));
                prior_blocked_candidate(&error)
            }
        }
    };
    let activation = match decision {
        ActivationAuthorizationDecision::RetainPrior(outcome) => outcome,
        ActivationAuthorizationDecision::Authorized(capability) => {
            control.check_active()?;
            let active = match store.activate(&model_id, &capability, &lease) {
                Ok(active) => active,
                Err(error @ ClassifierError::ActivationCommitUncertain { .. }) => {
                    job.active_model = store
                        .read_active_integrity()?
                        .as_ref()
                        .map(|m| model_summary(m, None));
                    return Err(error.into());
                }
                Err(error) => return Err(error.into()),
            };
            ActivationOutcome::Activated {
                reason: active.activation_reason,
                model_id: active.model_id,
                generation: active.activation_generation,
            }
        }
    };
    Ok(TrainingOutcome {
        report: candidate.report,
        activation,
    })
}
fn evaluate_sync(
    home: &Path,
    request: DatasetRequest,
    pinned: PinnedClassifier,
) -> Result<EvaluationReport, HomeError> {
    let selected = dataset::resolve_dataset(&request)?;
    let parsed = dataset::parse_csv(&selected)?;
    let store = ArtifactStore::new(home)?;
    let id = ModelId::parse(&pinned.snapshot.model_id)
        .ok_or_else(|| invalid_artifact("invalid_model_id"))?;
    let manifest = store.read_manifest(&id)?;
    let report: TrainingReport = serde_json::from_str(&store.read_report(&id)?)
        .map_err(|_| invalid_artifact("invalid_training_report"))?;
    let mut representatives = BTreeMap::new();
    for row in &parsed.corpus.rows {
        if row.split != Split::Test {
            continue;
        }
        representatives
            .entry(row.family_id.clone())
            .and_modify(|old: &mut &training::DatasetRow| {
                if row.sample_id < old.sample_id {
                    *old = row;
                }
            })
            .or_insert(row);
    }
    if representatives.is_empty() {
        return Err(ClassifierError::InvalidCalibrationData {
            family_id: None,
            reason: "missing_test_representatives".into(),
        }
        .into());
    }
    let rows: Vec<_> = representatives.into_values().collect();
    let mut scored = Vec::new();
    for batch in rows.chunks(super::INFERENCE_BATCH_SIZE) {
        let inputs = batch
            .iter()
            .map(|r| {
                ClassifyInput::new(
                    &r.sample_id,
                    &r.title,
                    &r.description,
                    Some(r.language.clone()),
                )
            })
            .collect();
        let receipts = pinned.service.classify_batch(inputs)?;
        if receipts.len() != batch.len() {
            return Err(invalid_artifact("evaluation_receipt_count_mismatch"));
        }
        for (row, receipt) in batch.iter().zip(receipts) {
            receipt.validate()?;
            let raw = receipt
                .raw_diagnostics
                .ok_or_else(|| invalid_artifact("missing_raw_diagnostics"))?;
            scored.push(ScoredFamily {
                family_id: row.family_id.clone(),
                representative_id: row.sample_id.clone(),
                split: row.split,
                provenance: row.provenance.clone(),
                membership: row.membership,
                raw: raw.domains,
            });
        }
    }
    let metrics = calibration::evaluate(&scored, &report.gate_set)?;
    let mut model = model_summary(&manifest, Some(&report));
    model.activated_at = pinned.snapshot.activated_at;
    Ok(EvaluationReport {
        schema_version: "classifier-evaluation-report-v1".into(),
        model,
        dataset: parsed.summary,
        split: "test".into(),
        representative_policy: training::REPRESENTATIVE_POLICY_VERSION.into(),
        metrics,
        admission_thresholds_enforced: true,
    })
}

fn cancel_requested(path: &Path, id: &str) -> Result<bool, HomeError> {
    let marker = read_optional_json::<CancelMarker>(&path.join("cancel.json"), 1024)?;
    match marker {
        None => Ok(false),
        Some(marker) if marker.schema_version == "classifier-cancel-v1" && marker.job_id == id => {
            Ok(true)
        }
        Some(_) => Err(invalid_artifact("invalid_cancel_marker")),
    }
}
fn read_live_job(home: &Path) -> Result<Option<TrainingJob>, HomeError> {
    let root = artifact::root_for(home)?;
    let Some(active) =
        read_optional_json::<ActiveJob>(&root.join("manual-active.json"), 64 * 1024)?
    else {
        return Ok(None);
    };
    validate_job_id(&active.job_id)?;
    if active.schema_version != "classifier-manual-active-v1" {
        return Err(invalid_artifact("manual_active_schema"));
    }
    let path = root.join("manual-jobs").join(&active.job_id);
    let job = read_job(&path, &active.job_id)?;
    if job.request_id != active.request_id || job.state.terminal() {
        return Ok(None);
    }
    if lock_held(&root.join("writer.lock"))? && lock_held(&path.join("owner.lock"))? {
        Ok(Some(job))
    } else {
        Ok(None)
    }
}
fn lock_held(path: &Path) -> Result<bool, HomeError> {
    match fs::metadata(path) {
        Ok(meta) if !meta.is_file() => return Err(invalid_artifact("nonregular_lock_file")),
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(io_error(e)),
    }
    let file = match File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(io_error(e)),
    };
    match file.try_lock_shared() {
        Ok(()) => Ok(false),
        Err(fs::TryLockError::WouldBlock) => Ok(true),
        Err(fs::TryLockError::Error(e)) => Err(io_error(e)),
    }
}
fn job_detail_sync(home: &Path, id: &str, settle: bool) -> Result<TrainingJob, HomeError> {
    validate_job_id(id)?;
    let path = artifact::root_for(home)?.join("manual-jobs").join(id);
    let mut job = read_job(&path, id)?;
    if job.state.terminal() {
        return Ok(job);
    }
    if read_live_job(home)?.is_some_and(|live| live.job_id == id) {
        return Ok(job);
    }
    if settle {
        let store = ArtifactStore::new(home)?;
        match store.try_writer() {
            Ok(_lease) => {
                job.state = TrainingJobState::Interrupted;
                job.advance()?;
                job.diagnostic = Some(diagnostic(
                    "worker_stopped",
                    "Training owner stopped; use a new request ID to rebuild",
                    true,
                ));
                write_job(&path, &job)?;
            }
            Err(ClassifierError::Busy) => {
                job.diagnostic = Some(diagnostic(
                    "ownership_unverified",
                    "Training ownership cannot currently be verified",
                    true,
                ));
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(job)
}
fn read_job(path: &Path, id: &str) -> Result<TrainingJob, HomeError> {
    let job = read_optional_json::<TrainingJob>(&path.join("result.json"), 1024 * 1024)?
        .or(read_optional_json::<TrainingJob>(
            &path.join("progress.json"),
            64 * 1024,
        )?)
        .ok_or_else(|| HomeError::NotFound { job_id: id.into() })?;
    if job.job_id != id
        || job.schema_version != "classifier-training-job-v1"
        || job_id(&job.request_id) != id
        || job.revision == 0
    {
        return Err(invalid_artifact("manual_job_identity_mismatch"));
    }
    Ok(job)
}
fn write_job(path: &Path, job: &TrainingJob) -> Result<(), HomeError> {
    let mut progress = job.clone();
    progress.outcome = None;
    write_json(&path.join("progress.json"), &progress, 64 * 1024)
}
fn read_optional_json<T: for<'a> Deserialize<'a>>(
    path: &Path,
    limit: u64,
) -> Result<Option<T>, HomeError> {
    match fs::metadata(path) {
        Ok(meta) if !meta.is_file() => return Err(invalid_artifact("nonregular_manual_file")),
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(io_error(e)),
    }
    let mut file = File::open(path).map_err(io_error)?;
    if !file.metadata().map_err(io_error)?.is_file() {
        return Err(invalid_artifact("nonregular_manual_file"));
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    if bytes.len() as u64 > limit {
        return Err(invalid_artifact("manual_file_too_large"));
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| invalid_artifact("invalid_manual_json"))
}
fn json_bytes(value: &impl Serialize, limit: usize) -> Result<Vec<u8>, HomeError> {
    struct Bounded {
        bytes: Vec<u8>,
        limit: usize,
    }
    impl Write for Bounded {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
                return Err(std::io::Error::other("manual JSON size limit"));
            }
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut writer = Bounded {
        bytes: Vec::new(),
        limit,
    };
    serde_json::to_writer(&mut writer, value)
        .map_err(|_| invalid_artifact("manual_json_size_or_serialization"))?;
    Ok(writer.bytes)
}
fn write_json(path: &Path, value: &impl Serialize, limit: usize) -> Result<(), HomeError> {
    let bytes = json_bytes(value, limit)?;
    let parent = path
        .parent()
        .ok_or_else(|| invalid_artifact("manual_path_missing_parent"))?;
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(io_error)?;
    file.write_all(&bytes).map_err(io_error)?;
    file.as_file().sync_all().map_err(io_error)?;
    file.persist(path).map_err(|e| io_error(e.error))?;
    File::open(parent)
        .and_then(|file| file.sync_all())
        .map_err(io_error)?;
    Ok(())
}
fn write_request_new(path: &Path, value: &StoredRequest) -> Result<(), HomeError> {
    let bytes = json_bytes(value, 16 * 1024)?;
    let parent = path
        .parent()
        .ok_or_else(|| invalid_artifact("manual_path_missing_parent"))?;
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(io_error)?;
    file.write_all(&bytes).map_err(io_error)?;
    file.as_file().sync_all().map_err(io_error)?;
    file.persist_noclobber(path)
        .map_err(|e| io_error(e.error))?;
    File::open(parent)
        .and_then(|file| file.sync_all())
        .map_err(io_error)?;
    Ok(())
}

/// Work a cycle is authorized to dispatch after its selected run is inspected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CycleRequirement {
    FreshAcquisition,
    AcquisitionAndAdmission,
    AdmissionOnly,
    PostAdmissionOnly,
}
/// Non-deserializable proof of successful classifier preflight. Callers cannot
/// manufacture a permit from a readiness boolean or an unrelated active model.
pub struct CyclePermit {
    requirement: CycleRequirement,
    pinned: Option<PinnedClassifier>,
}
impl CyclePermit {
    /// The exact phase requirement checked by this permit.
    #[must_use]
    pub fn requirement(&self) -> CycleRequirement {
        self.requirement
    }
    /// The retained model runtime, absent only for post-admission work.
    #[must_use]
    pub fn pinned(&self) -> Option<&PinnedClassifier> {
        self.pinned.as_ref()
    }
    /// Transfers the retained pin to the cycle without resolving active again.
    #[must_use]
    pub fn into_pinned(self) -> Option<PinnedClassifier> {
        self.pinned
    }
}
/// Typed blocked-cycle reason, distinct from article rejection.
#[derive(Debug)]
#[non_exhaustive]
pub enum CyclePreflightError {
    Classifier {
        status: ClassifierStatus,
        code: String,
        source: ClassifierError,
    },
    MissingPinnedRevision,
    InvalidRunState {
        code: String,
    },
}
impl fmt::Display for CyclePreflightError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Classifier { status, code, source } => {
                write!(f, "Atlas classifier blocked ({}; {code}): {source}.", status.as_str())?;
                match code.as_str() {
                    "classifier_unavailable" => f.write_str(" Run argos classifier status to check the local Rust-BERT setup."),
                    "pinned_classifier_unavailable" => f.write_str(" Restore the saved classifier revision to resume, or explicitly reclassify under a new revision."),
                    _ => Ok(()),
                }
            }
            Self::MissingPinnedRevision => f.write_str("Atlas classifier blocked: resumed acquisition/admission has no saved model revision. Explicitly re-evaluate under a new revision; the current active model cannot replace a missing pin."),
            Self::InvalidRunState { code } => write!(f, "Atlas classifier preflight cannot verify the saved run: {code}"),
        }
    }
}
impl std::error::Error for CyclePreflightError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Classifier { source, .. } => Some(source),
            _ => None,
        }
    }
}
/// Validates a real pin before acquisition/admission side effects. Post-admission
/// work needs no classifier, including in builds without the feature.
pub async fn preflight_cycle(
    home: &ClassifierHome,
    requirement: CycleRequirement,
    requested: Option<&ModelSnapshot>,
) -> Result<CyclePermit, CyclePreflightError> {
    if requirement == CycleRequirement::PostAdmissionOnly {
        return Ok(CyclePermit {
            requirement,
            pinned: None,
        });
    }
    let selected = match requirement {
        CycleRequirement::FreshAcquisition => None,
        CycleRequirement::AcquisitionAndAdmission | CycleRequirement::AdmissionOnly => {
            Some(requested.ok_or(CyclePreflightError::MissingPinnedRevision)?)
        }
        CycleRequirement::PostAdmissionOnly => None,
    };
    let pinned = home.pin_atlas(selected).await.map_err(|source| {
        let status = match &source {
            ClassifierError::Unavailable if cfg!(feature = "classifier") => {
                ClassifierStatus::Missing
            }
            ClassifierError::Incompatible(_) => ClassifierStatus::Incompatible,
            ClassifierError::InvalidArtifact(_) | ClassifierError::Io(_) => {
                ClassifierStatus::Invalid
            }
            _ => ClassifierStatus::Unavailable,
        };
        CyclePreflightError::Classifier {
            status,
            code: if selected.is_some() {
                "pinned_classifier_unavailable"
            } else {
                "classifier_unavailable"
            }
            .into(),
            source,
        }
    })?;
    Ok(CyclePermit {
        requirement,
        pinned: Some(pinned),
    })
}

#[cfg(all(test, feature = "classifier"))]
mod ownership_tests {
    use super::*;

    #[test]
    fn dropping_a_clone_preserves_the_home_and_only_the_final_owner_cancels() {
        let root = tempfile::tempdir().unwrap();
        let home = ClassifierHome::new(root.path().join("home"));
        let cancel = home.inner.cancel.clone();
        let weak = Arc::downgrade(&home.inner);
        let clone = home.clone();
        drop(clone);
        assert!(!cancel.load(Ordering::Acquire));
        assert!(weak.upgrade().is_some());
        drop(home);
        assert!(
            weak.upgrade().is_none(),
            "owned task storage must not retain the home"
        );
        assert!(cancel.load(Ordering::Acquire));
    }

    #[tokio::test]
    async fn a_completed_failed_job_does_not_cancel_a_clone_or_prevent_the_next_job() {
        let root = tempfile::tempdir().unwrap();
        let csv = root.path().join("invalid.csv");
        fs::write(&csv, b"deliberately invalid CSV; fitting must never begin").unwrap();
        let request = DatasetRequest {
            explicit_file: Some(csv),
            cwd: root.path().into(),
            executable: root.path().join("argos"),
            home: root.path().join("home"),
            ..Default::default()
        };
        let home = ClassifierHome::new(request.home.clone());
        for id in ["completed-first", "completed-second"] {
            let accepted = home
                .clone()
                .start_training(ManualTrainRequest::new(
                    request.clone(),
                    id.into(),
                    SyntheticActivation::RetainPrior,
                ))
                .await
                .unwrap();
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    if home
                        .inner
                        .owner
                        .lock()
                        .await
                        .as_ref()
                        .is_some_and(|owner| owner.task.is_finished())
                    {
                        break;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("bounded parser failure completes without any fitting runtime");
            assert_eq!(
                home.job_detail(&accepted.job_id).await.unwrap().state,
                TrainingJobState::Failed
            );
            assert!(
                !home.inner.cancel.load(Ordering::Acquire),
                "completion of a worker or temporary client clone cannot shut down the home"
            );
        }
        home.shutdown().await.unwrap();
        assert!(home.inner.owner.lock().await.is_none());
    }

    #[tokio::test]
    async fn cancelled_acceptance_waiting_for_owner_never_dispatches_fitting() {
        let root = tempfile::tempdir().expect("isolated classifier home");
        let csv = root.path().join("selected.csv");
        fs::write(&csv, b"Parsing must never begin in this cancellation case")
            .expect("selection fixture");
        let request = DatasetRequest {
            cwd: root.path().into(),
            executable: root.path().join("argos"),
            home: root.path().join("home"),
            explicit_file: Some(csv),
            ..Default::default()
        };
        let home = ClassifierHome::new(request.home.clone());
        let barrier = Arc::new(DispatchBarrier {
            prepared: tokio::sync::Notify::new(),
            proceed: tokio::sync::Notify::new(),
            acquiring_owner: tokio::sync::Notify::new(),
            dispatched: AtomicBool::new(false),
        });
        *home.inner.dispatch_barrier.lock().expect("test barrier") = Some(barrier.clone());
        let started_home = home.clone();
        let operation = tokio::spawn(async move {
            started_home
                .start_training(ManualTrainRequest::new(
                    request,
                    "cancel-before-owner".into(),
                    SyntheticActivation::RetainPrior,
                ))
                .await
        });
        barrier.prepared.notified().await;
        let held_slot = home.inner.owner.lock().await;
        barrier.proceed.notify_one();
        barrier.acquiring_owner.notified().await;
        operation.abort();
        assert!(operation
            .await
            .expect_err("operation cancelled at owned handle wait")
            .is_cancelled());
        assert!(
            !barrier.dispatched.load(Ordering::Acquire),
            "no task may dispatch before its handle can be retained"
        );
        drop(held_slot);
        let id = job_id("cancel-before-owner");
        let interrupted = home
            .job_detail(&id)
            .await
            .expect("durable prepared request remains inspectable");
        assert_eq!(interrupted.state, TrainingJobState::Interrupted);
        assert!(!home.home().join("classifier/active.json").exists());
        let store = ArtifactStore::new(home.home()).expect("existing home");
        let lease = store
            .try_writer()
            .expect("cancelled preparation released shared writer");
        drop(lease);
        home.shutdown().await.expect("no detached fitting owner");
    }

    struct TerminalMonitorFixture {
        root: tempfile::TempDir,
        home: ClassifierHome,
        request: DatasetRequest,
        release: Arc<tokio::sync::Notify>,
        entered: Arc<tokio::sync::Notify>,
        completed: Arc<AtomicBool>,
    }
    async fn terminal_monitor_fixture() -> TerminalMonitorFixture {
        let root = tempfile::tempdir().unwrap();
        let selected = root.path().join("invalid.csv");
        fs::write(&selected, "not a corpus").unwrap();
        let request = DatasetRequest {
            cwd: root.path().into(),
            executable: root.path().join("argos"),
            home: root.path().join("home"),
            explicit_file: Some(selected),
            ..Default::default()
        };
        let home = ClassifierHome::new(request.home.clone());
        let id = job_id("terminal-monitor-fixture");
        let path = request.home.join("classifier/manual-jobs").join(&id);
        fs::create_dir_all(&path).unwrap();
        let job = TrainingJob {
            schema_version: "classifier-training-job-v1".into(),
            job_id: id.clone(),
            request_id: "terminal-monitor-fixture".into(),
            state: TrainingJobState::Failed,
            revision: 2,
            dataset_path: None,
            progress: None,
            active_model: None,
            candidate_model_id: None,
            provenance: None,
            cancel_requested: false,
            outcome: None,
            diagnostic: Some(diagnostic(
                "fixture_parser_failed",
                "Isolated parser fixture",
                false,
            )),
        };
        write_json(&path.join("result.json"), &job, 1024 * 1024).unwrap();
        write_job(&path, &job).unwrap();
        let release = Arc::new(tokio::sync::Notify::new());
        let entered = Arc::new(tokio::sync::Notify::new());
        let completed = Arc::new(AtomicBool::new(false));
        let gate = release.clone();
        let done = completed.clone();
        let task = tokio::spawn(async move {
            gate.notified().await;
            done.store(true, Ordering::Release);
            Ok(())
        });
        *home.inner.owner.lock().await = Some(OwnedJob {
            job_id: id,
            cancel: Arc::new(AtomicBool::new(false)),
            task,
            reap_entered: Some(entered.clone()),
        });
        TerminalMonitorFixture {
            root,
            home,
            request,
            release,
            entered,
            completed,
        }
    }

    #[tokio::test]
    async fn terminal_monitor_is_joined_before_next_request_is_prepared() {
        let TerminalMonitorFixture {
            root: _root,
            home,
            request,
            release,
            entered,
            completed,
        } = terminal_monitor_fixture().await;
        let started = home.clone();
        let selected = request.clone();
        let operation = tokio::spawn(async move {
            started
                .start_training(ManualTrainRequest::new(
                    selected,
                    "after-terminal-owner".into(),
                    SyntheticActivation::RetainPrior,
                ))
                .await
        });
        entered.notified().await;
        assert!(
            !request
                .home
                .join("classifier/manual-jobs")
                .join(job_id("after-terminal-owner"))
                .exists(),
            "new request cannot precede owned monitor cleanup"
        );
        assert!(!completed.load(Ordering::Acquire));
        release.notify_one();
        operation.await.unwrap().unwrap();
        assert!(
            completed.load(Ordering::Acquire),
            "acceptance followed the actual monitor join"
        );
        home.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn cancelled_terminal_monitor_join_retains_handle_for_retry() {
        let TerminalMonitorFixture {
            root: _root,
            home,
            request,
            release,
            entered,
            completed,
        } = terminal_monitor_fixture().await;
        let started = home.clone();
        let selected = request.clone();
        let operation = tokio::spawn(async move {
            started
                .start_training(ManualTrainRequest::new(
                    selected,
                    "cancelled-terminal-join".into(),
                    SyntheticActivation::RetainPrior,
                ))
                .await
        });
        entered.notified().await;
        operation.abort();
        assert!(operation.await.unwrap_err().is_cancelled());
        {
            let owner = home.inner.owner.lock().await;
            let retained = owner
                .as_ref()
                .expect("caller cancellation retains old owned handle");
            assert_eq!(retained.job_id, job_id("terminal-monitor-fixture"));
            assert!(!retained.task.is_finished());
        }
        assert!(!request
            .home
            .join("classifier/manual-jobs")
            .join(job_id("cancelled-terminal-join"))
            .exists());
        let started = home.clone();
        let retry = tokio::spawn(async move {
            started
                .start_training(ManualTrainRequest::new(
                    request,
                    "cancelled-terminal-join".into(),
                    SyntheticActivation::RetainPrior,
                ))
                .await
        });
        entered.notified().await;
        release.notify_one();
        retry.await.unwrap().unwrap();
        assert!(completed.load(Ordering::Acquire));
        home.shutdown().await.unwrap();
    }
}
