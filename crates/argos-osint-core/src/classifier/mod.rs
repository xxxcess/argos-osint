//! Typed local classifier contracts for six-domain Atlas admission.
//!
//! This module is the public facade of the local classification service:
//! the domain vocabulary ([`DomainId`]), versioned constants, the input and
//! receipt data contracts, the [`InferenceEngine`] port, and the
//! [`ClassifierService`] entry point. It is unconditional in the crate;
//! when the `classifier` feature is off (or no engine is installed) the
//! public API still exists and reports
//! [`ClassifierStatus::Unavailable`] / [`ClassifierError::Unavailable`]
//! instead of panicking. It never depends on `nlp`.
//!
//! # Failures versus rejections
//!
//! Model, queue and storage faults are [`ClassifierError`] (blocked or
//! retryable). Blank, unsupported-language, wholly out-of-vocabulary,
//! insufficient-domain and out-of-scope inputs are explicit
//! [`RejectionReason`] values on an otherwise valid receipt — never `unk`
//! and never a hidden fallback to the remote classifier.
//!
//! # Service boundaries
//!
//! `engine` owns QuickJS inference and candidate workers. `artifact` persists
//! immutable learned JSON, checked pointers and activation history. `dataset`
//! resolves and validates CSV inputs; `training` and `calibration` own family
//! partitions, gates and promotion evidence. `readiness` coordinates the saved
//! home and manual jobs; `provider_capture` and `provider_labels` preserve raw
//! provider observations separately from reviewed training evidence.

pub mod artifact;
pub mod calibration;
pub mod dataset;
#[cfg(feature = "classifier")]
pub(crate) mod engine;
pub mod provider_capture;
pub mod provider_labels;
pub mod readiness;
pub mod training;

pub use crate::atlas_taxonomy::{DomainId, TAXONOMY_VERSION};
pub use artifact::{
    ActivatedModelIndex, ActivationAuthorization, ActivationAuthorizationDecision,
    ActivationOutcome, ActivationRecord, ModelId, ValidatedArtifact, WriterLease,
};
pub use calibration::{
    CalibrationFit, CalibrationPolicy, DomainCalibration, GateSet, ScoredFamily,
};
pub use provider_labels::{MaskedLabels, ProviderFormat, ProviderObservation};
pub use training::{
    CorpusHashes, DatasetRow, FamilyId, LabelMembership, Split, SyntheticActivation, TrainRequest,
    TrainedCandidate, TrainingCorpus, TrainingOutcome, TrainingProgress, TrainingReport,
    TrainingStage,
};

use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::Mutex;

/// Maximum number of inputs per inference call. Batches above this bound
/// are rejected with [`ClassifierError::BatchTooLarge`] instead of being
/// silently truncated.
pub const INFERENCE_BATCH_SIZE: usize = 64;

// One gate for every real classifier runtime fixture in this lib-test process.
// Integration binaries have independent process-local ledgers and their own gates.
#[cfg(all(test, feature = "classifier"))]
pub(crate) static TEST_RUNTIME_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Maximum title and description bytes accepted for one inference input.
pub const MAX_INPUT_TEXT_BYTES: usize = 32 * 1024;

/// QuickJS heap ceiling per classifier runtime, mirroring the NLP engine
/// precedent. This bounds the JS heap, not the process RSS.
pub const RUNTIME_MEMORY_LIMIT_BYTES: usize = 128 * 1024 * 1024;

/// QuickJS stack ceiling per classifier runtime.
pub const RUNTIME_STACK_LIMIT_BYTES: usize = 1024 * 1024;

/// Algorithm identity of the six-model engine: one binary Natural
/// `BayesClassifier` per domain, scored in log space. Artifacts are only
/// compatible with this algorithm.
pub const ALGORITHM_VERSION: &str = "natural-bayes-binary-ovr-v1";

/// Versioned preprocessing policy applied identically by the training
/// oracle and the runtime tokenizer: `keepStops=true` — meaningful
/// negation, acronyms and numbers are kept.
pub const PREPROCESSING_VERSION: &str = "natural-en-keepstops-v1";

/// Version of the admission policy: exactly two independently gated
/// domains (`MIN = MAX = 2`), each passing its own calibrated threshold and
/// informative-feature check.
pub const ADMISSION_POLICY_VERSION: &str = "exact-two-independent-gates-v1";

/// Version of the receipt schema produced by the classifier.
pub const RECEIPT_SCHEMA_VERSION: &str = "classifier-receipt-v1";

/// Version of the JavaScript bridge protocol.
pub const BRIDGE_VERSION: &str = "argos-classifier-bridge-v1";

/// Versioned scheme of [`ClassifyInput::input_hash`]: a SHA-256 over a
/// version tag plus length-delimited title and description bytes.
const INPUT_HASH_VERSION: &str = "classifier-input-v1";

/// Readiness vocabulary reported by the CLI and both launcher variants:
/// `missing`, `ready`, `training`, `invalid`, `incompatible`, `unavailable`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ClassifierStatus {
    /// No model is pinned for this home.
    Missing,
    /// A compatible model is pinned and its artifacts validate.
    Ready,
    /// Training or calibration work is in progress for this home.
    Training,
    /// A pinned model exists but its artifacts are missing or corrupt.
    Invalid,
    /// A pinned model exists but was built for other versions.
    Incompatible,
    /// The classifier is not available in this build or home.
    Unavailable,
}

impl ClassifierStatus {
    /// The snake_case status word used by CLI text output; matches the
    /// serialized serde form.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            ClassifierStatus::Missing => "missing",
            ClassifierStatus::Ready => "ready",
            ClassifierStatus::Training => "training",
            ClassifierStatus::Invalid => "invalid",
            ClassifierStatus::Incompatible => "incompatible",
            ClassifierStatus::Unavailable => "unavailable",
        }
    }
}

/// Faults raised by the classifier service and artifact store. These are
/// operational failures — blocked or retryable — never data rejections.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ClassifierError {
    /// No usable classifier: the `classifier` feature is disabled, or no
    /// engine is installed in this home.
    Unavailable,
    /// An artifact or manifest is missing, corrupt, or fails validation.
    InvalidArtifact(String),
    /// An artifact was built for other versions (algorithm, preprocessing,
    /// policy, taxonomy, feature schema or bundle).
    Incompatible(String),
    /// The service is temporarily busy (for example a training writer owns
    /// the home).
    Busy,
    /// The requested work was cancelled before completion.
    Cancelled,
    /// A queued call, inference or training deadline elapsed.
    Timeout,
    /// Filesystem access failed. The message carries the path and cause.
    Io(String),
    /// Storage access failed outside the artifact store.
    Storage(String),
    /// Training or calibration failed; the prior active model is untouched.
    Training(String),
    /// The training corpus or a queued input exceeds its configured bound.
    CorpusTooLarge,
    /// An inference batch exceeds [`INFERENCE_BATCH_SIZE`].
    BatchTooLarge,
    /// A manifest or model payload exceeds its size bound.
    ModelTooLarge,
    /// Process-wide memory reservation ceiling exceeded.
    RuntimeBudgetExceeded {
        requested: usize,
        reserved: usize,
        limit: usize,
    },
    /// Feature order mismatch during incremental training or restore.
    FeatureOrderMismatch { domain: DomainId },
    /// Input training dataset row fails validation or consistency check.
    InvalidTrainingData {
        row_id: Option<String>,
        reason: String,
    },
    /// Calibration scenario or parameter fails validity invariants.
    InvalidCalibrationData {
        family_id: Option<String>,
        reason: String,
    },
    /// Illegal lifecycle transition.
    InvalidTransition { from: String, event: String },
    /// JavaScript bridge returned unexpected or malformed envelope.
    BridgeProtocol { operation: String, code: String },
    /// Background worker thread stopped unexpectedly.
    WorkerStopped,
    /// Individual runtime resource limit exceeded.
    RuntimeLimitExceeded { resource: String },
    /// Single observation exceeds size limit.
    ObservationTooLarge,
    /// Observation batch size exceeds limit.
    ObservationBatchTooLarge,
    /// Calibration scenario family capacity exceeded.
    FamilyCapacityExceeded,
    /// Candidate model id already exists in storage.
    CandidateAlreadyExists,
    /// Candidate model activation denied by quality gate.
    ActivationDenied { reason: String },
    /// Active baseline changed during activation attempt.
    StaleActivation,
    /// Failure during post-rename sync creates indeterminate active state.
    ActivationCommitUncertain { model_id: String, generation: u64 },
}

impl fmt::Display for ClassifierError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClassifierError::Unavailable => {
                write!(f, "classifier engine unavailable in this build or home")
            }
            ClassifierError::InvalidArtifact(why) => {
                write!(f, "invalid classifier artifact: {why}")
            }
            ClassifierError::Incompatible(why) => {
                write!(f, "classifier artifact incompatible with this build: {why}")
            }
            ClassifierError::Busy => write!(f, "classifier is busy"),
            ClassifierError::Cancelled => write!(f, "classifier work was cancelled"),
            ClassifierError::Timeout => write!(f, "classifier deadline elapsed"),
            ClassifierError::Io(why) => write!(f, "classifier filesystem error: {why}"),
            ClassifierError::Storage(why) => write!(f, "classifier storage error: {why}"),
            ClassifierError::Training(why) => write!(f, "classifier training error: {why}"),
            ClassifierError::CorpusTooLarge => {
                write!(f, "classifier input exceeds the corpus bound")
            }
            ClassifierError::BatchTooLarge => {
                write!(f, "inference batch exceeds {} items", INFERENCE_BATCH_SIZE)
            }
            ClassifierError::ModelTooLarge => {
                write!(f, "classifier model artifact exceeds its size bound")
            }
            ClassifierError::RuntimeBudgetExceeded {
                requested,
                reserved,
                limit,
            } => {
                write!(
                    f,
                    "runtime memory budget exceeded: requested {requested} bytes, reserved {reserved} of {limit} byte limit"
                )
            }
            ClassifierError::FeatureOrderMismatch { domain } => {
                write!(f, "feature order mismatch for domain `{}`", domain.as_str())
            }
            ClassifierError::InvalidTrainingData { row_id, reason } => {
                write!(f, "invalid training data: {reason} (row: {row_id:?})")
            }
            ClassifierError::InvalidCalibrationData { family_id, reason } => {
                write!(
                    f,
                    "invalid calibration data: {reason} (family: {family_id:?})"
                )
            }
            ClassifierError::InvalidTransition { from, event } => {
                write!(
                    f,
                    "invalid classifier transition from `{from}` on `{event}`"
                )
            }
            ClassifierError::BridgeProtocol { operation, code } => {
                write!(
                    f,
                    "classifier bridge protocol error in `{operation}`: {code}"
                )
            }
            ClassifierError::WorkerStopped => write!(f, "classifier worker stopped"),
            ClassifierError::RuntimeLimitExceeded { resource } => {
                write!(f, "classifier runtime limit exceeded for `{resource}`")
            }
            ClassifierError::ObservationTooLarge => write!(f, "classifier observation too large"),
            ClassifierError::ObservationBatchTooLarge => {
                write!(f, "classifier observation batch too large")
            }
            ClassifierError::FamilyCapacityExceeded => {
                write!(f, "classifier calibration family capacity exceeded")
            }
            ClassifierError::CandidateAlreadyExists => {
                write!(f, "classifier candidate model already exists")
            }
            ClassifierError::ActivationDenied { reason } => {
                write!(f, "classifier model activation denied: {reason}")
            }
            ClassifierError::StaleActivation => {
                write!(f, "classifier model activation was stale")
            }
            ClassifierError::ActivationCommitUncertain {
                model_id,
                generation,
            } => {
                write!(
                    f,
                    "classifier activation commit uncertain for model `{model_id}` generation {generation}"
                )
            }
        }
    }
}

impl std::error::Error for ClassifierError {}

/// Why the classifier refused to admit an article. Rejections are explicit
/// receipt outcomes about the *input*, never model, queue or storage
/// faults (those are [`ClassifierError`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RejectionReason {
    /// Title and description are blank.
    Blank,
    /// The input language is outside the supported policy.
    UnsupportedLanguage,
    /// No known features were found in the input.
    WhollyOov,
    /// Fewer than two domains pass their independent gates.
    InsufficientDomains,
    /// The input is out of scope for strategic-domain admission.
    OutOfScope,
}

/// One article to classify. Text is English title + description only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassifyInput {
    /// Stable occurrence identifier carried onto the receipt.
    pub occurrence_id: String,
    pub title: String,
    pub description: String,
    /// Input language hint, if the provider supplied one. Provenance only:
    /// not part of [`ClassifyInput::input_hash`].
    pub language: Option<String>,
    /// Versioned SHA-256 over title and description: a
    /// `classifier-input-v1` frame (version tag, separator, length-prefixed
    /// title and description bytes) hashed with `sha256_hex`.
    /// Cache receipts and identical computations by this hash.
    pub input_hash: String,
}

impl ClassifyInput {
    /// Builds an input and computes [`ClassifyInput::input_hash`] over a
    /// version tag plus length-delimited title and description.
    pub fn new(
        occurrence_id: impl Into<String>,
        title: impl Into<String>,
        description: impl Into<String>,
        language: Option<String>,
    ) -> Self {
        let title = title.into();
        let description = description.into();
        // Frame = version tag + one separator byte + two u64 length headers.
        const FRAME_HEADERS: usize = INPUT_HASH_VERSION.len() + 1 + 2 * std::mem::size_of::<u64>();
        let mut framed = Vec::with_capacity(FRAME_HEADERS + title.len() + description.len());
        framed.extend_from_slice(INPUT_HASH_VERSION.as_bytes());
        framed.push(0);
        framed.extend_from_slice(&(title.len() as u64).to_le_bytes());
        framed.extend_from_slice(title.as_bytes());
        framed.extend_from_slice(&(description.len() as u64).to_le_bytes());
        framed.extend_from_slice(description.as_bytes());
        Self {
            occurrence_id: occurrence_id.into(),
            title,
            description,
            language,
            input_hash: artifact::sha256_hex(&framed),
        }
    }
}

/// Calibrated result for one domain inside a receipt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct DomainScore {
    pub domain: DomainId,
    /// Raw binary log-odds from the per-domain model. Raw values are never
    /// compared across domains; only the calibrated score is.
    pub raw_log_odds: f64,
    /// Independently calibrated relevance score for this domain.
    pub calibrated_score: f64,
    /// The admission threshold this domain was evaluated against.
    pub threshold: f64,
    /// Number of informative (known) features that fired.
    pub informative_features: usize,
    /// Number of out-of-vocabulary tokens, for OOV coverage reporting.
    pub oov_features: usize,
    /// Whether this domain passed its own gate.
    pub passes_gate: bool,
}

pub(crate) fn validate_inference_input(input: &ClassifyInput) -> Result<(), ClassifierError> {
    if input
        .title
        .len()
        .checked_add(input.description.len())
        .is_none_or(|size| size > MAX_INPUT_TEXT_BYTES)
        || input.occurrence_id.len() > 256
        || input
            .language
            .as_ref()
            .is_some_and(|language| language.len() > 128)
    {
        return Err(ClassifierError::CorpusTooLarge);
    }
    let expected = ClassifyInput::new(
        input.occurrence_id.clone(),
        input.title.clone(),
        input.description.clone(),
        input.language.clone(),
    );
    if input.input_hash != expected.input_hash {
        return Err(ClassifierError::InvalidArtifact(
            "inference input hash mismatch".into(),
        ));
    }
    Ok(())
}

/// Raw scoring output for one domain from the bridge.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RawDomainScore {
    pub domain: DomainId,
    pub present_log_score: f64,
    pub absent_log_score: f64,
    pub raw_log_odds: f64,
    pub known_feature_count: usize,
    pub oov_count: usize,
}

/// Raw scoring diagnostics attached to a [`ClassificationReceipt`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawDiagnostics {
    pub schema_version: String,
    pub calibration_version: String,
    pub domains: [RawDomainScore; 6],
    pub provider: Option<ProviderLabelDiagnostics>,
}

/// Diagnostics from upstream provider topic mapping.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderLabelDiagnostics {
    pub provider: String,
    pub endpoint: String,
    pub mapping_version: String,
    pub observation_id: String,
    pub raw_categories: serde_json::Value,
    pub filtered_labels: serde_json::Value,
    pub evidence: serde_json::Value,
    pub membership: [String; 6],
    pub training_membership: [String; 6],
    pub abstain_reason: Option<String>,
}

/// Durable outcome for one input: six ordered scores, the selected
/// two-domain pair or an explicit rejection reason, and every version the
/// decision depended on (schema [`RECEIPT_SCHEMA_VERSION`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ClassificationReceipt {
    #[serde(deserialize_with = "deserialize_receipt_schema")]
    pub receipt_schema_version: String,
    pub occurrence_id: String,
    pub input_hash: String,
    /// Exactly six scores, one per [`DomainId`], in [`DomainId::ALL`] order.
    /// Never normalized to sum to one.
    pub scores: Vec<DomainScore>,
    /// The exactly-two admitted domains, ranked first then second.
    pub selected_pair: Option<[DomainId; 2]>,
    /// Why the input was rejected, when it was not admitted.
    pub rejection_reason: Option<RejectionReason>,
    pub taxonomy_version: String,
    pub model_id: String,
    pub preprocessing_version: String,
    pub policy_version: String,
    /// RFC 3339 evaluation timestamp.
    pub evaluated_at: String,
    #[serde(default)]
    pub raw_diagnostics: Option<RawDiagnostics>,
}

fn deserialize_receipt_schema<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<String, D::Error> {
    let version = String::deserialize(deserializer)?;
    if version != RECEIPT_SCHEMA_VERSION {
        return Err(serde::de::Error::custom(
            "receipt schema requires migration",
        ));
    }
    Ok(version)
}

impl ClassificationReceipt {
    /// Validates a receipt before using persisted decisions. Legacy receipts require migration.
    pub fn validate(&self) -> Result<(), ClassifierError> {
        if self.receipt_schema_version != RECEIPT_SCHEMA_VERSION
            || self.taxonomy_version != TAXONOMY_VERSION
            || self.preprocessing_version != PREPROCESSING_VERSION
            || self.policy_version != ADMISSION_POLICY_VERSION
        {
            return Err(ClassifierError::Incompatible(
                "receipt versions require migration".into(),
            ));
        }
        let invalid =
            || ClassifierError::InvalidArtifact("invalid persisted classification receipt".into());
        if self.occurrence_id.is_empty()
            || ModelId::parse(&self.model_id).is_none()
            || self.input_hash.len() != 64
            || !self
                .input_hash
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            || chrono::DateTime::parse_from_rfc3339(&self.evaluated_at).is_err()
        {
            return Err(invalid());
        }
        if self.scores.len() != 6
            || !self
                .scores
                .iter()
                .map(|score| score.domain)
                .eq(DomainId::ALL)
            || self.scores.iter().any(|score| {
                !score.raw_log_odds.is_finite()
                    || !score.calibrated_score.is_finite()
                    || !(0.0..=1.0).contains(&score.calibrated_score)
                    || !score.threshold.is_finite()
                    || (!(0.0..=1.0).contains(&score.threshold) && score.threshold != 2.0)
                    || (score.passes_gate
                        && (score.calibrated_score < score.threshold
                            || score.informative_features == 0))
            })
        {
            return Err(invalid());
        }
        let mut ranked: Vec<&DomainScore> = self.scores.iter().collect();
        ranked.sort_by(|first, second| {
            second
                .calibrated_score
                .total_cmp(&first.calibrated_score)
                .then_with(|| first.domain.as_str().cmp(second.domain.as_str()))
        });
        match self.selected_pair {
            Some(pair)
                if self.rejection_reason.is_none()
                    && pair == [ranked[0].domain, ranked[1].domain]
                    && ranked[0].passes_gate
                    && ranked[1].passes_gate => {}
            None if self.rejection_reason.is_some() => {
                if ranked[0].passes_gate && ranked[1].passes_gate {
                    return Err(invalid());
                }
                if matches!(self.rejection_reason, Some(RejectionReason::WhollyOov))
                    && self
                        .scores
                        .iter()
                        .any(|score| score.informative_features != 0)
                {
                    return Err(invalid());
                }
            }
            _ => return Err(invalid()),
        }
        if let Some(diagnostics) = &self.raw_diagnostics {
            if diagnostics.schema_version != "classifier-raw-diagnostics-v1"
                || diagnostics.calibration_version != calibration::CALIBRATION_VERSION
            {
                return Err(invalid());
            }
            for (raw, score) in diagnostics.domains.iter().zip(&self.scores) {
                if raw.domain != score.domain
                    || !raw.present_log_score.is_finite()
                    || !raw.absent_log_score.is_finite()
                    || raw.raw_log_odds != score.raw_log_odds
                    || raw.known_feature_count != score.informative_features
                    || raw.oov_count != score.oov_features
                    || (raw.raw_log_odds - (raw.present_log_score - raw.absent_log_score)).abs()
                        > 1e-9
                {
                    return Err(invalid());
                }
            }
        }
        Ok(())
    }

    /// Builds a receipt with the canonical six zeroed scores in
    /// [`DomainId::ALL`] order and the current module versions, ready for
    /// the engine to fill in. Guarantees the six-score invariant.
    pub fn new(
        occurrence_id: impl Into<String>,
        input_hash: impl Into<String>,
        model_id: impl Into<String>,
    ) -> Self {
        let scores = DomainId::ALL
            .iter()
            .map(|domain| DomainScore {
                domain: *domain,
                raw_log_odds: 0.0,
                calibrated_score: 0.0,
                threshold: 0.0,
                informative_features: 0,
                oov_features: 0,
                passes_gate: false,
            })
            .collect();
        Self {
            receipt_schema_version: RECEIPT_SCHEMA_VERSION.into(),
            occurrence_id: occurrence_id.into(),
            input_hash: input_hash.into(),
            scores,
            selected_pair: None,
            rejection_reason: None,
            taxonomy_version: TAXONOMY_VERSION.into(),
            model_id: model_id.into(),
            preprocessing_version: PREPROCESSING_VERSION.into(),
            policy_version: ADMISSION_POLICY_VERSION.into(),
            evaluated_at: chrono::Utc::now().to_rfc3339(),
            raw_diagnostics: None,
        }
    }

    /// The score recorded for `domain`, if present.
    #[must_use]
    pub fn score(&self, domain: DomainId) -> Option<&DomainScore> {
        self.scores.iter().find(|score| score.domain == domain)
    }

    /// Whether the receipt admits exactly two domains and carries no
    /// rejection reason.
    #[must_use]
    pub fn is_admitted(&self) -> bool {
        self.selected_pair.is_some() && self.rejection_reason.is_none()
    }
}

/// Where a model's training data came from. A synthetic-only install is
/// labelled visibly and is never auto-promoted to production evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Provenance {
    SyntheticBaseline {
        dataset_version: String,
        dataset_sha256: String,
    },
    ReviewedReal {
        dataset_version: String,
        dataset_sha256: String,
        review_reference: String,
    },
}

impl Provenance {
    /// Builds synthetic-seed provenance. Activating a model with this
    /// provenance always shows the visible
    /// [`Provenance::label`](Self::label) for the seed.
    pub fn synthetic_baseline(
        dataset_version: impl Into<String>,
        dataset_sha256: impl Into<String>,
    ) -> Self {
        Provenance::SyntheticBaseline {
            dataset_version: dataset_version.into(),
            dataset_sha256: dataset_sha256.into(),
        }
    }

    /// Builds reviewed-real provenance; the review reference keeps the
    /// promotion decision auditable instead of trusting a bare hash.
    pub fn reviewed_real(
        dataset_version: impl Into<String>,
        dataset_sha256: impl Into<String>,
        review_reference: impl Into<String>,
    ) -> Self {
        Provenance::ReviewedReal {
            dataset_version: dataset_version.into(),
            dataset_sha256: dataset_sha256.into(),
            review_reference: review_reference.into(),
        }
    }

    /// Human-visible quality provenance label.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Provenance::SyntheticBaseline { .. } => "Synthetic baseline",
            Provenance::ReviewedReal { .. } => "Reviewed real",
        }
    }
}

/// The pinned model a cycle or job runs against, captured once and reused
/// for the whole run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ModelSnapshot {
    pub model_id: String,
    pub artifact_sha256: String,
    pub taxonomy_version: String,
    pub algorithm_version: String,
    pub preprocessing_version: String,
    pub policy_version: String,
    /// RFC 3339 activation timestamp.
    pub activated_at: String,
    pub provenance: Provenance,
}

impl ModelSnapshot {
    /// Builds a snapshot for an activated model, stamping this build's
    /// version constants and the activation time. Engine wiring, the CLI
    /// and the launchers use it to pin a cycle or job to exactly one model.
    pub fn new(
        model_id: impl Into<String>,
        artifact_sha256: impl Into<String>,
        provenance: Provenance,
    ) -> Self {
        Self {
            model_id: model_id.into(),
            artifact_sha256: artifact_sha256.into(),
            taxonomy_version: TAXONOMY_VERSION.into(),
            algorithm_version: ALGORITHM_VERSION.into(),
            preprocessing_version: PREPROCESSING_VERSION.into(),
            policy_version: ADMISSION_POLICY_VERSION.into(),
            activated_at: chrono::Utc::now().to_rfc3339(),
            provenance,
        }
    }
}

/// Actual worker observations for one long-lived inference runtime.
/// Execution samples and sizes are paired, ordered, and capped at the latest 4096 batches.
/// Heap peak is the maximum observed QuickJS memory_usage snapshot, not an allocator peak.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ClassifierPerformance {
    pub queue_micros: u64,
    pub batch_execution_micros: Vec<u64>,
    pub batch_sizes: Vec<usize>,
    pub js_heap_current_bytes: u64,
    pub js_heap_peak_bytes: u64,
    pub runtime_reserved_bytes: u64,
    pub stack_limit_bytes: u64,
}

/// Aggregate inference metrics. Contains counts and timings only — never
/// article text or credentials.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct InferenceMetrics {
    pub articles: u64,
    /// Reserved aggregate queue timing. Actual worker observations are exposed
    /// by [`ClassifierService::performance`]; this counter does not measure it.
    pub queue_micros: Option<u64>,
    pub inference_micros: u64,
    pub cache_hits: u64,
    pub rejections: u64,
}

/// Port implemented by the local inference engine (the later QuickJS/Natural
/// `engine` module). Object-safe so a service can own
/// `Box<dyn InferenceEngine>` on its worker.
///
/// Implementors may assume batches respect [`INFERENCE_BATCH_SIZE`] and that
/// the serialized payload is the validated data-only model produced by
/// [`artifact`].
pub trait InferenceEngine: Send {
    /// Classifies one bounded batch, one receipt per input in order.
    ///
    /// # Errors
    /// Returns [`ClassifierError`] for model, queue or engine faults;
    /// per-input data rejections are recorded on the receipt instead.
    fn classify_batch(
        &mut self,
        inputs: &[ClassifyInput],
    ) -> Result<Vec<ClassificationReceipt>, ClassifierError>;

    /// Runs an inference batch with caller-owned cancellation and independent budgets.
    /// Existing ports may keep synchronous behavior; the embedded worker overrides this.
    fn classify_batch_with_control(
        &mut self,
        inputs: &[ClassifyInput],
        control: &WorkControl,
    ) -> Result<Vec<ClassificationReceipt>, ClassifierError> {
        control.check_active()?;
        self.classify_batch(inputs)
    }

    /// Returns actual observations, or Unavailable for a port without instrumentation.
    fn performance(&self) -> Result<ClassifierPerformance, ClassifierError> {
        Err(ClassifierError::Unavailable)
    }

    /// Serializes the loaded model to its data-only JSON payload.
    ///
    /// # Errors
    /// Returns [`ClassifierError`] when no model is loaded or serialization
    /// fails.
    fn serialize_model(&mut self) -> Result<String, ClassifierError>;

    /// Restores a validated data-only payload produced by
    /// [`artifact::validate_artifact`].
    ///
    /// # Errors
    /// Returns [`ClassifierError`] when the payload cannot be restored.
    fn restore_model(&mut self, payload: &str) -> Result<(), ClassifierError>;
}

/// Entry-point service for local classification.
///
/// The facade owns the pinned [`ModelSnapshot`], the readiness state and the
/// aggregate [`InferenceMetrics`]; the engine worker (later unit) plugs in
/// behind [`InferenceEngine`], serialized by an internal lock that
/// stands in for the dedicated worker channel. Facade methods are
/// synchronous; engine, queue and file work runs on the owning worker
/// thread, never on a Tokio worker.
pub struct ClassifierService {
    status: ClassifierStatus,
    unavailable_reason: Option<String>,
    pinned: Option<ModelSnapshot>,
    engine: Mutex<Option<Box<dyn InferenceEngine>>>,
    metrics: Mutex<InferenceMetrics>,
}

impl Default for ClassifierService {
    /// A fresh service with no pinned model: [`ClassifierStatus::Missing`].
    fn default() -> Self {
        Self {
            status: ClassifierStatus::Missing,
            unavailable_reason: None,
            pinned: None,
            engine: Mutex::new(None),
            metrics: Mutex::new(InferenceMetrics::default()),
        }
    }
}

impl ClassifierService {
    /// A service that can never classify in this build or home (disabled
    /// `classifier` feature, unsupported platform, failed initialization).
    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            status: ClassifierStatus::Unavailable,
            unavailable_reason: Some(reason.into()),
            pinned: None,
            engine: Mutex::new(None),
            metrics: Mutex::new(InferenceMetrics::default()),
        }
    }

    /// Constructs a ready [`ClassifierService`] from a validated snapshot and artifact.
    pub fn from_snapshot(
        snapshot: ModelSnapshot,
        artifact: ValidatedArtifact,
        policy: Box<dyn ReceiptPolicy>,
    ) -> Result<Self, ClassifierError> {
        #[cfg(feature = "classifier")]
        {
            engine::from_snapshot(snapshot, artifact, policy)
        }
        #[cfg(not(feature = "classifier"))]
        {
            let _ = (snapshot, artifact, policy);
            Err(ClassifierError::Unavailable)
        }
    }

    /// Current readiness state.
    #[must_use]
    pub fn status(&self) -> ClassifierStatus {
        self.status
    }

    /// Why this service is [`ClassifierStatus::Unavailable`], if it is.
    #[must_use]
    pub fn unavailable_reason(&self) -> Option<&str> {
        self.unavailable_reason.as_deref()
    }

    /// The pinned model snapshot, if one is installed.
    #[must_use]
    pub fn pinned_model(&self) -> Option<&ModelSnapshot> {
        self.pinned.as_ref()
    }

    /// Snapshots best-effort successful inference counters and service wall time.
    /// External cache accounting and actual worker queue/heap observations have
    /// separate owners. Unmeasured queue time remains `None`.
    ///
    /// # Errors
    /// Returns [`ClassifierError::Busy`] if the accounting mutex is poisoned;
    /// unavailable counters must never appear as a fresh all-zero measurement.
    pub fn metrics(&self) -> Result<InferenceMetrics, ClassifierError> {
        self.metrics
            .lock()
            .map(|metrics| *metrics)
            .map_err(|_| ClassifierError::Busy)
    }

    /// Classifies one bounded batch.
    ///
    /// Checks availability first (a disabled feature reports
    /// [`ClassifierError::Unavailable`] even for an oversized batch), then
    /// the [`INFERENCE_BATCH_SIZE`] bound ([`ClassifierError::BatchTooLarge`]).
    /// With the `classifier` feature off — or with the feature on but no
    /// engine installed — this returns [`ClassifierError::Unavailable`]; it
    /// never panics and never falls back to the remote classifier. Metrics
    /// update only on success. Accounting failure does not block inference;
    /// dropped updates are traced and [`Self::metrics`] reports a typed error.
    ///
    /// # Errors
    /// See above; model and engine faults propagate as [`ClassifierError`].
    pub fn classify_batch(
        &self,
        inputs: Vec<ClassifyInput>,
    ) -> Result<Vec<ClassificationReceipt>, ClassifierError> {
        self.classify_batch_impl(inputs, None)
    }

    /// Runs inference with the owning cycle's cancellation and worker budgets.
    pub fn classify_batch_with_control(
        &self,
        inputs: Vec<ClassifyInput>,
        control: &WorkControl,
    ) -> Result<Vec<ClassificationReceipt>, ClassifierError> {
        self.classify_batch_impl(inputs, Some(control))
    }

    /// Snapshots actual bounded measurements without sending JavaScript values across threads.
    pub fn performance(&self) -> Result<ClassifierPerformance, ClassifierError> {
        let engine = self.engine.lock().map_err(|_| ClassifierError::Busy)?;
        engine
            .as_ref()
            .ok_or(ClassifierError::Unavailable)?
            .performance()
    }

    fn classify_batch_impl(
        &self,
        inputs: Vec<ClassifyInput>,
        control: Option<&WorkControl>,
    ) -> Result<Vec<ClassificationReceipt>, ClassifierError> {
        // A poisoned mutex means a previous engine call panicked; the Argos
        // mutex rule forbids observing or continuing through poisoned state,
        // so the service reports a retryable Busy instead of reading the
        // engine.
        let mut locked = self.engine.lock().map_err(|_| ClassifierError::Busy)?;
        let Some(engine) = locked.as_mut() else {
            return Err(ClassifierError::Unavailable);
        };
        // Availability precedes the batch bound: a disabled feature or a
        // missing engine reports Unavailable even when the batch is also
        // oversized.
        if inputs.len() > INFERENCE_BATCH_SIZE {
            return Err(ClassifierError::BatchTooLarge);
        }
        for input in &inputs {
            validate_inference_input(input)?;
        }
        let started = std::time::Instant::now();
        let receipts = if let Some(control) = control {
            engine.classify_batch_with_control(&inputs, control)?
        } else {
            engine.classify_batch(&inputs)?
        };
        drop(locked);
        let inference_micros = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
        let rejections = receipts
            .iter()
            .filter(|receipt| receipt.rejection_reason.is_some())
            .count() as u64;
        let articles = receipts.len() as u64;
        self.record_metrics(|metrics| {
            metrics.articles += articles;
            metrics.inference_micros += inference_micros;
            metrics.rejections += rejections;
        });
        Ok(receipts)
    }

    fn record_metrics(&self, update: impl FnOnce(&mut InferenceMetrics)) {
        // Accounting is independent of inference. Never inspect/recover a
        // poisoned guard; readers observe the failure through the fallible port.
        match self.metrics.lock() {
            Ok(mut metrics) => update(&mut metrics),
            Err(_) => tracing::warn!(
                code = "classifier_metrics_poisoned",
                "classifier aggregate metric update dropped; counters unavailable"
            ),
        }
    }
}

impl fmt::Debug for ClassifierService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClassifierService")
            .field("status", &self.status)
            .field("pinned", &self.pinned.as_ref().map(|m| &m.model_id))
            .field("metrics", &self.metrics())
            .finish_non_exhaustive()
    }
}

/// Pure readiness helper for the CLI and both launcher variants: computes
/// the probed status **without loading the dataset CSV** or any artifact
/// file. The live [`ClassifierStatus::Training`] state is owned by the
/// running service and is not derivable here.
///
/// ```
/// use argos_osint_core::classifier::{
///     status_for_model, ClassifierStatus, ModelSnapshot, Provenance,
/// };
///
/// // Feature disabled: unavailable regardless of artifacts.
/// assert_eq!(
///     status_for_model(None, false, false),
///     ClassifierStatus::Unavailable
/// );
/// // No pinned model: missing.
/// assert_eq!(status_for_model(None, true, true), ClassifierStatus::Missing);
/// // Pinned model whose artifact files are gone: invalid.
/// let snapshot =
///     ModelSnapshot::new("seed-m1", "0".repeat(64), Provenance::synthetic_baseline("seed-v1", "0".repeat(64)));
/// assert_eq!(
///     status_for_model(Some(&snapshot), false, true),
///     ClassifierStatus::Invalid
/// );
/// ```
///
/// A pinned model whose snapshot versions differ from this build reports
/// [`ClassifierStatus::Incompatible`]; otherwise [`ClassifierStatus::Ready`].
#[must_use]
pub fn status_for_model(
    snapshot: Option<&ModelSnapshot>,
    artifacts_present: bool,
    feature_enabled: bool,
) -> ClassifierStatus {
    if !feature_enabled {
        return ClassifierStatus::Unavailable;
    }
    let Some(snapshot) = snapshot else {
        return ClassifierStatus::Missing;
    };
    if !artifacts_present {
        return ClassifierStatus::Invalid;
    }
    if snapshot.taxonomy_version != TAXONOMY_VERSION
        || snapshot.algorithm_version != ALGORITHM_VERSION
        || snapshot.preprocessing_version != PREPROCESSING_VERSION
        || snapshot.policy_version != ADMISSION_POLICY_VERSION
    {
        return ClassifierStatus::Incompatible;
    }
    ClassifierStatus::Ready
}

/// Port for applying calibration gates and turning raw domain scores into
/// an admitted/rejected [`ClassificationReceipt`].
pub trait ReceiptPolicy: Send {
    fn receipt(
        &self,
        input: &ClassifyInput,
        raw: &[RawDomainScore; 6],
        model_id: &ModelId,
    ) -> Result<ClassificationReceipt, ClassifierError>;
}

/// Closed set of operations supported by the JavaScript bridge protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BridgeOperation {
    TrainBatch,
    FinishTraining,
    ClassifyBatch,
    SerializeModel,
    RestoreModel,
}

/// Control knobs and cancellation handle for a worker invocation.
#[derive(Debug, Clone)]
pub struct WorkControl {
    pub cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pub queue_timeout: std::time::Duration,
    pub execution_timeout: std::time::Duration,
    pub overall_deadline: std::time::Instant,
}

impl WorkControl {
    /// Builds a default work control with 5s queue timeout, 2s execution timeout,
    /// and 30-minute overall deadline from now.
    #[must_use]
    pub fn new() -> Self {
        Self {
            cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            queue_timeout: std::time::Duration::from_secs(5),
            execution_timeout: std::time::Duration::from_secs(2),
            overall_deadline: std::time::Instant::now() + std::time::Duration::from_secs(1800),
        }
    }

    /// Checks whether execution has been cancelled or timed out.
    ///
    /// # Errors
    /// Returns [`ClassifierError::Cancelled`] if cancelled, or [`ClassifierError::Timeout`]
    /// if the overall deadline has elapsed.
    pub fn check_active(&self) -> Result<(), ClassifierError> {
        if self.cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(ClassifierError::Cancelled);
        }
        if std::time::Instant::now() >= self.overall_deadline {
            return Err(ClassifierError::Timeout);
        }
        Ok(())
    }
}

impl Default for WorkControl {
    fn default() -> Self {
        Self::new()
    }
}

/// Generic worker interface for candidate training, evaluation, and bridge operations.
pub trait ClassifierWorker: Send {
    fn call(
        &mut self,
        operation: BridgeOperation,
        request: &str,
        control: &WorkControl,
    ) -> Result<String, ClassifierError>;

    /// Verifies a completed exported candidate, then permanently closes its worker.
    /// Real workers release their old runtime before the bounded startup restore.
    /// The default preserves small protocol fakes; forwarding wrappers must delegate.
    fn restore_completed_candidate(
        &mut self,
        model_id: &ModelId,
        payload: &str,
        control: &WorkControl,
    ) -> Result<(), ClassifierError> {
        control.check_active()?;
        if payload.len() as u64 > artifact::MAX_ARTIFACT_BYTES {
            return Err(ClassifierError::ModelTooLarge);
        }
        let request = serde_json::json!({"version":"1","request_id":"terminal-restore","model_id":model_id.as_str(),"artifact":payload});
        let response = self.call(BridgeOperation::RestoreModel, &request.to_string(), control)?;
        let response: serde_json::Value =
            serde_json::from_str(&response).map_err(|error| ClassifierError::BridgeProtocol {
                operation: "restoreModel".into(),
                code: error.to_string(),
            })?;
        if response["version"] != "1"
            || response["request_id"] != "terminal-restore"
            || response["errors"]
                .as_array()
                .is_none_or(|errors| !errors.is_empty())
            || response.get("error").is_some()
            || response["smoke_ok"] != true
            || response["restored_model_id"] != model_id.as_str()
        {
            return Err(ClassifierError::BridgeProtocol {
                operation: "restoreModel".into(),
                code: "restore_smoke_failed".into(),
            });
        }
        control.check_active()
    }
}

/// Starts a dedicated candidate classifier worker.
pub fn start_candidate_worker(
    model_id: &ModelId,
    lease: std::sync::Arc<WriterLease>,
    control: &WorkControl,
) -> Result<Box<dyn ClassifierWorker>, ClassifierError> {
    #[cfg(feature = "classifier")]
    {
        engine::start_candidate_worker(model_id, lease, control)
    }
    #[cfg(not(feature = "classifier"))]
    {
        let _ = (model_id, lease, control);
        Err(ClassifierError::Unavailable)
    }
}

/// Identity and version checksums of the compiled classifier assets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompiledIdentity {
    pub bundle_sha256: String,
    pub bridge_sha256: String,
    pub adapter_sha256: String,
    pub source_manifest_sha256: String,
    pub bridge_version: String,
    pub natural_version: String,
    pub apparatus_version: String,
}

/// Returns the trusted compiled identity of the classifier assets embedded
/// in this binary.
pub fn compiled_identity() -> Result<CompiledIdentity, ClassifierError> {
    #[cfg(feature = "classifier")]
    {
        engine::compiled_identity()
    }
    #[cfg(not(feature = "classifier"))]
    {
        Err(ClassifierError::Unavailable)
    }
}

#[cfg(test)]
mod fault_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    struct FaultEngine {
        fault: ClassifierError,
        calls: Arc<AtomicUsize>,
    }
    impl InferenceEngine for FaultEngine {
        fn classify_batch(
            &mut self,
            _: &[ClassifyInput],
        ) -> Result<Vec<ClassificationReceipt>, ClassifierError> {
            self.calls.fetch_add(1, Ordering::AcqRel);
            Err(self.fault.clone())
        }
        fn serialize_model(&mut self) -> Result<String, ClassifierError> {
            Err(self.fault.clone())
        }
        fn restore_model(&mut self, _: &str) -> Result<(), ClassifierError> {
            Err(self.fault.clone())
        }
    }
    fn service(fault: ClassifierError, calls: Arc<AtomicUsize>) -> ClassifierService {
        ClassifierService {
            status: ClassifierStatus::Ready,
            unavailable_reason: None,
            pinned: None,
            engine: Mutex::new(Some(Box::new(FaultEngine { fault, calls }))),
            metrics: Mutex::new(InferenceMetrics::default()),
        }
    }
    fn input() -> ClassifyInput {
        ClassifyInput::new(
            "fault-occurrence",
            "Synthetic policy",
            "Test description",
            Some("en".into()),
        )
    }

    #[test]
    fn operational_faults_never_become_rejection_receipts() {
        for fault in [
            ClassifierError::Busy,
            ClassifierError::Timeout,
            ClassifierError::Cancelled,
            ClassifierError::WorkerStopped,
            ClassifierError::Storage("injected-storage".into()),
            ClassifierError::Io("injected-io".into()),
            ClassifierError::RuntimeLimitExceeded {
                resource: "heap".into(),
            },
            ClassifierError::RuntimeBudgetExceeded {
                requested: 129,
                reserved: 387,
                limit: 512,
            },
        ] {
            let calls = Arc::new(AtomicUsize::new(0));
            let service = service(fault.clone(), calls.clone());
            assert_eq!(service.classify_batch(vec![input()]), Err(fault));
            assert_eq!(calls.load(Ordering::Acquire), 1);
            assert_eq!(service.metrics().unwrap().articles, 0);
            assert_eq!(service.performance(), Err(ClassifierError::Unavailable));
        }
        let unavailable = ClassifierService::default();
        assert_eq!(
            unavailable.classify_batch(vec![input(); 65]),
            Err(ClassifierError::Unavailable)
        );
    }

    #[test]
    fn poisoned_engine_stops_before_dispatch_without_recovering_the_guard() {
        let calls = Arc::new(AtomicUsize::new(0));
        let service = Arc::new(service(ClassifierError::WorkerStopped, calls.clone()));
        let owned = service.clone();
        let poisoning = std::thread::spawn(move || {
            let _guard = owned.engine.lock().unwrap();
            panic!("test-only engine poisoning");
        });
        assert!(
            poisoning.join().is_err(),
            "the injected worker panic must be observed"
        );
        assert_eq!(
            service.classify_batch(vec![input()]),
            Err(ClassifierError::Busy)
        );
        assert_eq!(service.performance(), Err(ClassifierError::Busy));
        assert_eq!(calls.load(Ordering::Acquire), 0);
        assert!(service.engine.is_poisoned());
    }

    #[test]
    fn poisoned_counters_report_error_without_reading_or_updating_the_guard() {
        let service = Arc::new(service(
            ClassifierError::WorkerStopped,
            Arc::new(AtomicUsize::new(0)),
        ));
        assert_eq!(service.metrics().unwrap().articles, 0);
        assert_eq!(service.metrics().unwrap().queue_micros, None);
        let owned = service.clone();
        let poisoning = std::thread::spawn(move || {
            let _guard = owned.metrics.lock().unwrap();
            panic!("test-only accounting poisoning");
        });
        assert!(
            poisoning.join().is_err(),
            "the injected panic must be observed"
        );
        assert_eq!(service.metrics(), Err(ClassifierError::Busy));
        let updates = AtomicUsize::new(0);
        service.record_metrics(|_| {
            updates.fetch_add(1, Ordering::AcqRel);
        });
        assert_eq!(updates.load(Ordering::Acquire), 0);
        assert!(service.metrics.is_poisoned());
        assert!(!service.engine.is_poisoned());
        assert!(format!("{service:?}").contains("Busy"));
    }

    #[test]
    fn invalid_or_oversized_inputs_fail_before_engine_dispatch() {
        let calls = Arc::new(AtomicUsize::new(0));
        let service = service(ClassifierError::WorkerStopped, calls.clone());
        let mut corrupt = input();
        corrupt.input_hash = "0".repeat(64);
        assert!(matches!(
            service.classify_batch(vec![corrupt]),
            Err(ClassifierError::InvalidArtifact(_))
        ));
        assert!(matches!(
            service.classify_batch(vec![ClassifyInput::new(
                "large",
                "x".repeat(MAX_INPUT_TEXT_BYTES + 1),
                "",
                None
            )]),
            Err(ClassifierError::CorpusTooLarge)
        ));
        assert_eq!(calls.load(Ordering::Acquire), 0);
        assert_eq!(
            service.classify_batch(vec![ClassifyInput::new(
                "inclusive",
                "x".repeat(MAX_INPUT_TEXT_BYTES),
                "",
                None
            )]),
            Err(ClassifierError::WorkerStopped)
        );
        assert_eq!(
            calls.load(Ordering::Acquire),
            1,
            "exact input limit must reach the engine"
        );
    }
}
