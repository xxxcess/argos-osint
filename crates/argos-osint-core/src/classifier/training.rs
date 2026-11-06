//! Training DTOs, corpus validation, deterministic family representatives,
//! and candidate training orchestration for Argos classifier models.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::ops::Deref;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::artifact::{DomainClassCount, DomainFamilyCount, ModelId};
use super::calibration::{
    CalibrationPolicy, DomainAcceptance, EvaluationMetrics, GateSet, ScoredFamily,
};
use crate::atlas_taxonomy::DomainId;

use super::{
    BridgeOperation, ClassifierError, ClassifierWorker, Provenance, RawDomainScore, WorkControl,
    PREPROCESSING_VERSION,
};

/// Batch size for bridge training calls (32 representative rows per batch).
pub const TRAINING_BATCH_SIZE: usize = 32;

/// Maximum documents fitted per finish step in the bridge worker (16 documents).
pub const FIT_DOCUMENTS_PER_STEP: usize = 16;

/// Maximum number of rows allowed in a training corpus (50,000).
pub const MAX_TRAINING_ROWS: usize = 50_000;

/// Maximum combined UTF-8 text bytes across an entire training corpus (64 MiB).
pub const MAX_TRAINING_TEXT_BYTES: usize = 64 * 1024 * 1024;

/// Maximum combined UTF-8 text bytes (title + description) for a single row (32 KiB).
pub const MAX_ROW_TEXT_BYTES: usize = 32 * 1024;

/// Maximum size in bytes for a training envelope wire payload (2 MiB).
pub const MAX_TRAINING_ENVELOPE_BYTES: usize = 2 * 1024 * 1024;

/// Maximum unique retained token storage across training (32 MiB).
pub const MAX_RETAINED_TOKEN_BYTES: usize = 32 * 1024 * 1024;

/// Maximum training features allowed per domain model (100,000).
pub const MAX_TRAINING_FEATURES_PER_DOMAIN: usize = 100_000;

/// Maximum retained documents allowed per domain (50,000).
pub const MAX_RETAINED_DOCUMENTS_PER_DOMAIN: usize = 50_000;

/// Schema version for classifier training reports.
pub const TRAINING_REPORT_SCHEMA_VERSION: &str = "classifier-training-report-v2";

/// Policy version for deterministic family representative deduplication.
pub const REPRESENTATIVE_POLICY_VERSION: &str = "family-representative-v1";

/// Canonical split partition assignment for a dataset row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Split {
    Train,
    Calibration,
    Development,
    Test,
}

impl Split {
    /// Returns the static snake_case string representation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Train => "train",
            Self::Calibration => "calibration",
            Self::Development => "development",
            Self::Test => "test",
        }
    }
}

impl fmt::Display for Split {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Binary class membership annotation for a specific domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum LabelMembership {
    Present,
    Absent,
    Unknown,
}

impl LabelMembership {
    /// Returns the static string representation matching JS bridge wire format.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Present => "Present",
            Self::Absent => "Absent",
            Self::Unknown => "Unknown",
        }
    }
}

impl fmt::Display for LabelMembership {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A validated, non-empty logical identifier for a news story family.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct FamilyId(String);

impl FamilyId {
    /// Parses and validates a family identifier.
    ///
    /// # Errors
    /// Returns [`ClassifierError::InvalidTrainingData`] if `value` is empty,
    /// longer than 256 bytes, or contains Unicode control characters.
    pub fn parse(value: &str) -> Result<Self, ClassifierError> {
        if value.is_empty() {
            return Err(ClassifierError::InvalidTrainingData {
                row_id: None,
                reason: "family_id cannot be empty".into(),
            });
        }
        if value.len() > 256 {
            return Err(ClassifierError::InvalidTrainingData {
                row_id: None,
                reason: format!("family_id exceeds 256 bytes (len={})", value.len()),
            });
        }
        for (idx, character) in value.char_indices() {
            if character.is_control() {
                return Err(ClassifierError::InvalidTrainingData {
                    row_id: None,
                    reason: format!("family_id contains a control character at byte {idx}"),
                });
            }
        }
        Ok(Self(value.to_string()))
    }

    /// Accesses the underlying string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Deref for FamilyId {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AsRef<str> for FamilyId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for FamilyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<FamilyId> for String {
    fn from(id: FamilyId) -> Self {
        id.0
    }
}

/// A normalized dataset row for training, calibration, or evaluation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatasetRow {
    pub sample_id: String,
    pub family_id: FamilyId,
    pub split: Split,
    pub title: String,
    pub description: String,
    pub language: String,
    pub membership: [LabelMembership; 6],
    pub labels_complete: bool,
    pub provenance: Provenance,
}

/// Canonical length-framed split and family assignment hashes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CorpusHashes {
    pub train_split_sha256: String,
    pub calibration_split_sha256: String,
    pub development_split_sha256: String,
    pub test_split_sha256: String,
    pub family_assignment_sha256: String,
}

/// A complete, typed training corpus ready for validation and candidate fitting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrainingCorpus {
    pub rows: Vec<DatasetRow>,
    pub dataset_sha256: String,
    pub train_split_sha256: String,
    pub calibration_split_sha256: String,
    pub development_split_sha256: String,
    pub test_split_sha256: String,
    pub family_assignment_sha256: String,
}

/// Consent setting governing synthetic candidate activation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyntheticActivation {
    #[default]
    RetainPrior,
    ExplicitOptIn,
}

/// Request parameters to train a candidate model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrainRequest {
    pub request_id: String,
    pub model_id: ModelId,
    pub provenance: Provenance,
    pub synthetic_activation: SyntheticActivation,
}

/// Lifecycle stage of a candidate training execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrainingStage {
    Validating,
    Collecting,
    Fitting,
    Calibrating,
    ValidatingRestore,
    CandidateWritten,
    Finished,
}

impl TrainingStage {
    /// Returns the static snake_case string representation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Validating => "validating",
            Self::Collecting => "collecting",
            Self::Fitting => "fitting",
            Self::Calibrating => "calibrating",
            Self::ValidatingRestore => "validating_restore",
            Self::CandidateWritten => "candidate_written",
            Self::Finished => "finished",
        }
    }
}

impl fmt::Display for TrainingStage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Monotonic progress telemetry emitted during candidate training.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrainingProgress {
    pub request_id: String,
    pub stage: TrainingStage,
    pub round: u64,
    pub rows_seen: u64,
    pub rows_fitted: u64,
    pub rows_total: u64,
    pub fitted_memberships: [u64; 6],
    pub token_bytes: u64,
    pub vocabulary_sizes: [u64; 6],
}

/// Output of a completed candidate training run.
#[derive(Debug, Clone, PartialEq)]
pub struct TrainedCandidate {
    pub model_id: ModelId,
    pub payload: String,
    pub report: TrainingReport,
}

/// Detailed evaluation and calibration report for a candidate model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrainingReport {
    #[serde(default)]
    pub calibrator_comparison: Vec<CalibratorComparison>,
    pub schema_version: String,
    pub request_id: String,
    pub model_id: ModelId,
    pub provenance: Provenance,
    pub class_counts: [DomainClassCount; 6],
    pub family_counts: [DomainFamilyCount; 6],
    #[serde(rename = "calibration")]
    pub gate_set: GateSet,
    /// Calibration-fit diagnostics are not independent quality evidence.
    #[serde(default)]
    pub fit_diagnostics: Option<EvaluationMetrics>,
    #[serde(default = "default_evaluation_split")]
    pub evaluation_split: Split,
    pub evaluation: EvaluationMetrics,
    pub domain_decisions: [DomainAcceptance; 6],
}

/// Development comparison; never derived from Test or reported as real review.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CalibratorComparison {
    pub kind: String,
    pub development_brier: Option<f64>,
    pub development_macro_f1: Option<f64>,
    pub selected: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<String>,
}

fn default_evaluation_split() -> Split {
    Split::Calibration
}

/// High-level training outcome combining report and activation authorization result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrainingOutcome {
    pub report: TrainingReport,
    pub activation: super::calibration::ActivationOutcome,
}

// -----------------------------------------------------------------------------
// Wire Bridge DTOs (Serde JSON representation for QuickJS worker)
// -----------------------------------------------------------------------------

/// Preprocessing policy sent to the QuickJS bridge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreprocessingPolicy {
    pub stemmer: String,
    #[serde(rename = "keepStops")]
    pub keep_stops: bool,
    pub policy_version: String,
}

/// A single row sent in a `trainBatch` bridge call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrainingRow {
    pub row_id: String,
    #[serde(default)]
    pub feature_input_hash: String,
    pub title: String,
    pub description: String,
    pub membership: [LabelMembership; 6],
}

/// Request envelope for `BridgeOperation::TrainBatch`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrainBatchRequest {
    pub version: String,
    pub request_id: String,
    pub model_id: String,
    pub rows: Vec<TrainingRow>,
    pub preprocessing: PreprocessingPolicy,
    pub batch_limit: usize,
}

/// Failure diagnostic from a bridge operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeFailure {
    pub code: String,
    pub message: String,
    pub row_id: Option<String>,
    pub domain: Option<DomainId>,
}

/// Response payload from `BridgeOperation::TrainBatch`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrainBatchResponse {
    pub version: String,
    pub request_id: String,
    pub accepted: u64,
    pub skipped: u64,
    pub token_bytes: u64,
    pub vocabulary_new: u64,
    pub progress: f64,
    pub round: u64,
    pub feature_order_revisions: [u64; 6],
    pub fitted_memberships: [u64; 6],
    pub vocabulary_sizes: [u64; 6],
    pub errors: Vec<BridgeFailure>,
}

/// Request envelope for `BridgeOperation::FinishTraining`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FinishTrainingRequest {
    pub version: String,
    pub request_id: String,
    pub model_id: String,
    pub freeze_feature_order: bool,
    pub max_documents_per_step: usize,
    pub done_when: u64,
}

/// Response payload from `BridgeOperation::FinishTraining`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FinishTrainingResponse {
    pub version: String,
    pub request_id: String,
    pub done: bool,
    pub frozen_order_length: u64,
    pub fit_step: u64,
    pub round: u64,
    pub fitted_memberships: [u64; 6],
    pub vocabulary_sizes: [u64; 6],
    pub feature_order_revisions: [u64; 6],
    pub errors: Vec<BridgeFailure>,
}

// -----------------------------------------------------------------------------
// Canonical Hashing & Validation
// -----------------------------------------------------------------------------

/// Computes canonical length-framed SHA-256 hashes for dataset splits and
/// permanent family assignments.
///
/// # Errors
/// Returns [`ClassifierError::InvalidTrainingData`] if any sample ID or family ID
/// is invalid, or if conflicting split assignments are detected for the same family.
pub fn corpus_hashes(rows: &[DatasetRow]) -> Result<CorpusHashes, ClassifierError> {
    let mut train_rows = Vec::new();
    let mut calib_rows = Vec::new();
    let mut test_rows = Vec::new();
    let mut development_rows = Vec::new();
    let mut family_splits: BTreeMap<&FamilyId, Split> = BTreeMap::new();

    for row in rows {
        if let Some(existing_split) = family_splits.get(&row.family_id) {
            if *existing_split != row.split {
                return Err(ClassifierError::InvalidTrainingData {
                    row_id: Some(row.sample_id.clone()),
                    reason: format!(
                        "conflicting split for family `{}`: previously `{}`, got `{}`",
                        row.family_id,
                        existing_split.as_str(),
                        row.split.as_str()
                    ),
                });
            }
        } else {
            family_splits.insert(&row.family_id, row.split);
        }

        match row.split {
            Split::Train => train_rows.push(row),
            Split::Calibration => calib_rows.push(row),
            Split::Development => development_rows.push(row),
            Split::Test => test_rows.push(row),
        }
    }

    // Sort stably by sample_id for deterministic hash calculation
    train_rows.sort_by(|a, b| a.sample_id.cmp(&b.sample_id));
    calib_rows.sort_by(|a, b| a.sample_id.cmp(&b.sample_id));
    test_rows.sort_by(|a, b| a.sample_id.cmp(&b.sample_id));

    let train_split_sha256 = hash_split_rows("train", &train_rows);
    let calibration_split_sha256 = hash_split_rows("calibration", &calib_rows);
    development_rows.sort_by(|a, b| a.sample_id.cmp(&b.sample_id));
    let development_split_sha256 = hash_split_rows("development", &development_rows);
    let test_split_sha256 = hash_split_rows("test", &test_rows);

    // Family assignment hash: sorted distinct (family_id, split) pairs
    let mut hasher = Sha256::new();
    hasher.update(b"classifier-family-assignment-v1\0");
    for (family_id, split) in family_splits {
        let fam_bytes = family_id.as_bytes();
        hasher.update((fam_bytes.len() as u64).to_le_bytes());
        hasher.update(fam_bytes);
        let split_bytes = split.as_str().as_bytes();
        hasher.update((split_bytes.len() as u64).to_le_bytes());
        hasher.update(split_bytes);
    }
    let family_assignment_sha256 = format!("{:x}", hasher.finalize());

    Ok(CorpusHashes {
        train_split_sha256,
        calibration_split_sha256,
        development_split_sha256,
        test_split_sha256,
        family_assignment_sha256,
    })
}

fn hash_split_rows(split_name: &str, rows: &[&DatasetRow]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"classifier-split-rows-v1\0");
    hasher.update((split_name.len() as u64).to_le_bytes());
    hasher.update(split_name.as_bytes());
    hasher.update((rows.len() as u64).to_le_bytes());

    for row in rows {
        // Length-frame sample_id
        let s_bytes = row.sample_id.as_bytes();
        hasher.update((s_bytes.len() as u64).to_le_bytes());
        hasher.update(s_bytes);

        // Length-frame family_id
        let f_bytes = row.family_id.as_bytes();
        hasher.update((f_bytes.len() as u64).to_le_bytes());
        hasher.update(f_bytes);

        // Length-frame title
        let t_bytes = row.title.as_bytes();
        hasher.update((t_bytes.len() as u64).to_le_bytes());
        hasher.update(t_bytes);

        // Length-frame description
        let d_bytes = row.description.as_bytes();
        hasher.update((d_bytes.len() as u64).to_le_bytes());
        hasher.update(d_bytes);

        // Length-frame language
        let l_bytes = row.language.as_bytes();
        hasher.update((l_bytes.len() as u64).to_le_bytes());
        hasher.update(l_bytes);

        // Membership for 6 domains (1 byte each: 'P', 'A', 'U')
        for m in &row.membership {
            let m_byte = match m {
                LabelMembership::Present => b'P',
                LabelMembership::Absent => b'A',
                LabelMembership::Unknown => b'U',
            };
            hasher.update([m_byte]);
        }

        // labels_complete flag (1 byte: 1 or 0)
        hasher.update([if row.labels_complete { 1 } else { 0 }]);
    }

    format!("{:x}", hasher.finalize())
}

/// Validates source attribution metadata before it is used for model or quality claims.
fn valid_source_hash(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}
fn invalid_source() -> ClassifierError {
    ClassifierError::InvalidTrainingData {
        row_id: None,
        reason: "invalid_source_provenance".into(),
    }
}

pub(crate) fn validate_provenance(provenance: &Provenance) -> Result<(), ClassifierError> {
    let (version, hash, review) = match provenance {
        Provenance::SyntheticBaseline {
            dataset_version,
            dataset_sha256,
        } => (dataset_version, dataset_sha256, None),
        Provenance::ReviewedReal {
            dataset_version,
            dataset_sha256,
            review_reference,
        } => (dataset_version, dataset_sha256, Some(review_reference)),
        Provenance::ApprovedProvider {
            dataset_version,
            dataset_sha256,
            mapping_approval_sha256,
        } => {
            if !valid_source_hash(mapping_approval_sha256) {
                return Err(invalid_source());
            }
            (dataset_version, dataset_sha256, None)
        }
        Provenance::MixedTraining {
            dataset_version,
            dataset_sha256,
            seed_dataset_sha256,
            reviewed_corpus_sha256,
            review_reference,
            mapping_approval_sha256,
        } => {
            if ![
                seed_dataset_sha256,
                reviewed_corpus_sha256,
                mapping_approval_sha256,
            ]
            .into_iter()
            .all(|hash| valid_source_hash(hash))
            {
                return Err(invalid_source());
            }
            (dataset_version, dataset_sha256, Some(review_reference))
        }
    };
    if version.trim().is_empty()
        || version.len() > 256
        || version.chars().any(char::is_control)
        || hash.len() != 64
        || !hash.bytes().all(|byte| byte.is_ascii_hexdigit())
        || review.is_some_and(|reference| {
            reference.trim().is_empty()
                || reference.len() > 16 * 1024
                || reference.chars().any(char::is_control)
        })
    {
        return Err(ClassifierError::InvalidTrainingData {
            row_id: None,
            reason: "invalid_source_provenance".into(),
        });
    }
    Ok(())
}

/// Validates corpus bounds, source provenance, complete labels and split/family integrity.
///
/// # Errors
/// Returns [`ClassifierError::InvalidTrainingData`] on malformed or inconsistent data.
pub fn validate_corpus(corpus: &TrainingCorpus) -> Result<(), ClassifierError> {
    if corpus.rows.is_empty() {
        return Err(ClassifierError::InvalidTrainingData {
            row_id: None,
            reason: "training corpus contains 0 rows".into(),
        });
    }

    if corpus.rows.len() > MAX_TRAINING_ROWS {
        return Err(ClassifierError::InvalidTrainingData {
            row_id: None,
            reason: format!(
                "corpus row count {} exceeds maximum {}",
                corpus.rows.len(),
                MAX_TRAINING_ROWS
            ),
        });
    }

    // Verify dataset_sha256 format
    if corpus.dataset_sha256.len() != 64
        || !corpus.dataset_sha256.chars().all(|c| c.is_ascii_hexdigit())
    {
        return Err(ClassifierError::InvalidTrainingData {
            row_id: None,
            reason: "invalid dataset_sha256 hex string".into(),
        });
    }

    let mut total_text_bytes: usize = 0;
    let mut seen_samples = BTreeSet::new();
    let mut family_representatives: BTreeMap<&FamilyId, &DatasetRow> = BTreeMap::new();
    let mut family_memberships: BTreeMap<&FamilyId, [LabelMembership; 6]> = BTreeMap::new();
    let mut train_row_count = 0;

    for row in &corpus.rows {
        validate_provenance(&row.provenance)?;
        // Validate sample_id bounds
        if row.sample_id.is_empty()
            || row.sample_id.len() > 256
            || row.sample_id.chars().any(char::is_control)
        {
            return Err(ClassifierError::InvalidTrainingData {
                row_id: Some(row.sample_id.clone()),
                reason: "sample_id must be between 1 and 256 bytes".into(),
            });
        }
        if !seen_samples.insert(&row.sample_id) {
            return Err(ClassifierError::InvalidTrainingData {
                row_id: Some(row.sample_id.clone()),
                reason: "duplicate_sample_id".into(),
            });
        }

        // Validate text length and non-blankness
        let title_bytes = row.title.len();
        let desc_bytes = row.description.len();
        let combined_bytes = title_bytes.saturating_add(desc_bytes);

        if combined_bytes > MAX_ROW_TEXT_BYTES {
            return Err(ClassifierError::InvalidTrainingData {
                row_id: Some(row.sample_id.clone()),
                reason: "row_text_exceeds_32k".into(),
            });
        }

        let is_blank = row.title.trim().is_empty() && row.description.trim().is_empty();
        if is_blank {
            return Err(ClassifierError::InvalidTrainingData {
                row_id: Some(row.sample_id.clone()),
                reason: "blank_text".into(),
            });
        }

        total_text_bytes = total_text_bytes.saturating_add(combined_bytes);
        if total_text_bytes > MAX_TRAINING_TEXT_BYTES {
            return Err(ClassifierError::InvalidTrainingData {
                row_id: Some(row.sample_id.clone()),
                reason: "corpus_text_exceeds_64mb".into(),
            });
        }

        // Validate English language requirement ("en" or "en-*")
        let lang_lower = row.language.trim().to_ascii_lowercase();
        if lang_lower != "en" && !lang_lower.starts_with("en-") {
            return Err(ClassifierError::InvalidTrainingData {
                row_id: Some(row.sample_id.clone()),
                reason: format!("unsupported_language: `{}`", row.language),
            });
        }

        // Validate completeness consistency:
        // labels_complete is true iff NONE of the 6 memberships is Unknown
        let has_unknown = row.membership.contains(&LabelMembership::Unknown);
        if row.labels_complete && has_unknown {
            return Err(ClassifierError::InvalidTrainingData {
                row_id: Some(row.sample_id.clone()),
                reason: "incomplete_row_cannot_claim_complete".into(),
            });
        }
        if !row.labels_complete && !has_unknown {
            return Err(ClassifierError::InvalidTrainingData {
                row_id: Some(row.sample_id.clone()),
                reason: "complete_row_must_set_labels_complete".into(),
            });
        }

        // Validate family membership consistency
        if let Some(existing_membership) = family_memberships.get(&row.family_id) {
            for (idx, (m_new, m_old)) in row
                .membership
                .iter()
                .zip(existing_membership.iter())
                .enumerate()
            {
                if m_new != m_old {
                    let domain = DomainId::ALL[idx];
                    return Err(ClassifierError::InvalidTrainingData {
                        row_id: Some(row.sample_id.clone()),
                        reason: format!(
                            "conflicting_family_membership: family `{}` has conflicting annotation for domain `{}` (`{}` vs `{}`)",
                            row.family_id,
                            domain.as_str(),
                            m_old.as_str(),
                            m_new.as_str()
                        ),
                    });
                }
            }
        } else {
            family_memberships.insert(&row.family_id, row.membership);
        }

        // Track deterministic representative per family (lexicographically smallest sample_id)
        if let Some(current_rep) = family_representatives.get(&row.family_id) {
            if row.sample_id < current_rep.sample_id {
                family_representatives.insert(&row.family_id, row);
            }
        } else {
            family_representatives.insert(&row.family_id, row);
        }

        if row.split == Split::Train {
            train_row_count += 1;
        }
    }

    if train_row_count == 0 {
        return Err(ClassifierError::InvalidTrainingData {
            row_id: None,
            reason: "corpus contains 0 train split rows".into(),
        });
    }

    // Verify canonical split and family assignment hashes
    let computed_hashes = corpus_hashes(&corpus.rows)?;
    if computed_hashes.train_split_sha256 != corpus.train_split_sha256 {
        return Err(ClassifierError::InvalidTrainingData {
            row_id: None,
            reason: format!(
                "train_split_sha256 mismatch: expected `{}`, computed `{}`",
                corpus.train_split_sha256, computed_hashes.train_split_sha256
            ),
        });
    }
    if computed_hashes.calibration_split_sha256 != corpus.calibration_split_sha256 {
        return Err(ClassifierError::InvalidTrainingData {
            row_id: None,
            reason: format!(
                "calibration_split_sha256 mismatch: expected `{}`, computed `{}`",
                corpus.calibration_split_sha256, computed_hashes.calibration_split_sha256
            ),
        });
    }
    if computed_hashes.development_split_sha256 != corpus.development_split_sha256 {
        return Err(ClassifierError::InvalidTrainingData {
            row_id: None,
            reason: format!(
                "development_split_sha256 mismatch: expected `{}`, computed `{}`",
                corpus.development_split_sha256, computed_hashes.development_split_sha256
            ),
        });
    }
    if computed_hashes.test_split_sha256 != corpus.test_split_sha256 {
        return Err(ClassifierError::InvalidTrainingData {
            row_id: None,
            reason: format!(
                "test_split_sha256 mismatch: expected `{}`, computed `{}`",
                corpus.test_split_sha256, computed_hashes.test_split_sha256
            ),
        });
    }
    if computed_hashes.family_assignment_sha256 != corpus.family_assignment_sha256 {
        return Err(ClassifierError::InvalidTrainingData {
            row_id: None,
            reason: format!(
                "family_assignment_sha256 mismatch: expected `{}`, computed `{}`",
                corpus.family_assignment_sha256, computed_hashes.family_assignment_sha256
            ),
        });
    }

    Ok(())
}

// -----------------------------------------------------------------------------
// Training Orchestration via ClassifierWorker
// -----------------------------------------------------------------------------

fn operation_id(request: &TrainRequest, operation: &str, round: u64, scope: &str) -> String {
    let mut digest = Sha256::new();
    let round = round.to_string();
    for part in [
        "classifier-operation-id-v1",
        request.request_id.as_str(),
        request.model_id.as_str(),
        operation,
        round.as_str(),
        scope,
    ] {
        digest.update((part.len() as u64).to_le_bytes());
        digest.update(part.as_bytes());
    }
    format!("{operation}:{:x}", digest.finalize())
}

fn training_batch_request(
    request: &TrainRequest,
    round: u64,
    rows: Vec<TrainingRow>,
) -> TrainBatchRequest {
    TrainBatchRequest {
        version: "1".into(),
        request_id: operation_id(request, "trainBatch", round, "0"),
        model_id: request.model_id.as_str().into(),
        rows,
        preprocessing: PreprocessingPolicy {
            stemmer: "porter".into(),
            keep_stops: true,
            policy_version: PREPROCESSING_VERSION.into(),
        },
        batch_limit: TRAINING_BATCH_SIZE,
    }
}

/// Orchestrates candidate training rounds across a `ClassifierWorker`.
///
/// # Errors
/// Returns [`ClassifierError`] on validation errors, timeout, cancellation, or
/// worker protocol failures.
pub fn train_candidate<W: ClassifierWorker + ?Sized>(
    worker: &mut W,
    request: &TrainRequest,
    corpus: &TrainingCorpus,
    control: &WorkControl,
    progress: &mut dyn FnMut(&TrainingProgress),
) -> Result<TrainedCandidate, ClassifierError> {
    control.check_active()?;

    let mut prog = TrainingProgress {
        request_id: request.request_id.clone(),
        stage: TrainingStage::Validating,
        round: 0,
        rows_seen: 0,
        rows_fitted: 0,
        rows_total: 0,
        fitted_memberships: [0; 6],
        token_bytes: 0,
        vocabulary_sizes: [0; 6],
    };
    progress(&prog);

    // 1. Validate corpus end-to-end
    validate_corpus(corpus)?;
    validate_provenance(&request.provenance)?;
    let request_hash = match &request.provenance {
        Provenance::SyntheticBaseline { dataset_sha256, .. }
        | Provenance::ReviewedReal { dataset_sha256, .. }
        | Provenance::MixedTraining { dataset_sha256, .. } => dataset_sha256,
        Provenance::ApprovedProvider { .. } => return Err(invalid_source()),
    };
    if request_hash != &corpus.dataset_sha256
        || (matches!(request.provenance, Provenance::ReviewedReal { .. })
            && corpus.rows.iter().any(|row| {
                row.split == Split::Train
                    && !matches!(row.provenance, Provenance::ReviewedReal { .. })
            }))
    {
        return Err(ClassifierError::InvalidTrainingData {
            row_id: None,
            reason: "training_source_provenance_mismatch".into(),
        });
    }
    if request.request_id.is_empty()
        || request.request_id.len() > 256
        || request.request_id.chars().any(char::is_control)
    {
        return Err(ClassifierError::InvalidTrainingData {
            row_id: None,
            reason: "invalid_request_id".into(),
        });
    }

    // 2. Select Train split family representatives (lexicographically smallest sample_id per family)
    prog.stage = TrainingStage::Collecting;
    let mut train_families: BTreeMap<&FamilyId, &DatasetRow> = BTreeMap::new();
    let mut calib_families: BTreeMap<&FamilyId, &DatasetRow> = BTreeMap::new();
    let mut development_families: BTreeMap<&FamilyId, &DatasetRow> = BTreeMap::new();

    for row in &corpus.rows {
        match row.split {
            Split::Train => {
                if let Some(rep) = train_families.get(&row.family_id) {
                    if row.sample_id < rep.sample_id {
                        train_families.insert(&row.family_id, row);
                    }
                } else {
                    train_families.insert(&row.family_id, row);
                }
            }
            Split::Calibration => {
                if let Some(rep) = calib_families.get(&row.family_id) {
                    if row.sample_id < rep.sample_id {
                        calib_families.insert(&row.family_id, row);
                    }
                } else {
                    calib_families.insert(&row.family_id, row);
                }
            }
            Split::Development => {
                let representative = development_families.entry(&row.family_id).or_insert(row);
                if row.sample_id < representative.sample_id {
                    *representative = row;
                }
            }
            Split::Test => {}
        }
    }

    let mut train_reps: Vec<&DatasetRow> = train_families.into_values().collect();
    train_reps.sort_by(|a, b| a.sample_id.cmp(&b.sample_id));

    prog.rows_total = train_reps.len() as u64;
    progress(&prog);

    if !train_reps.iter().any(|row| {
        row.membership
            .iter()
            .any(|label| *label != LabelMembership::Unknown)
    }) {
        return Err(ClassifierError::InvalidTrainingData {
            row_id: None,
            reason: "no_supervised_training_families".into(),
        });
    }
    // 3. Batch training rounds
    prog.stage = TrainingStage::Fitting;
    // Vocabulary sees Train-family representatives only. Retain their encoded
    // features in the bounded worker until fit, avoiding a second tokenization.
    let mut prepared_offset = 0;
    let mut prepared_round = 0u64;
    while prepared_offset < train_reps.len() {
        control.check_active()?;
        let mut count = (train_reps.len() - prepared_offset).min(TRAINING_BATCH_SIZE);
        let (request_json, prepared_count) = loop {
            let rows: Vec<_> = train_reps[prepared_offset..prepared_offset + count]
                .iter()
                .filter(|row| {
                    row.membership
                        .iter()
                        .any(|label| *label != LabelMembership::Unknown)
                })
                .map(|row| TrainingRow {
                    row_id: row.sample_id.clone(),
                    feature_input_hash: super::ClassifyInput::new(
                        "vocabulary",
                        &row.title,
                        &row.description,
                        None,
                    )
                    .input_hash,
                    title: row.title.clone(),
                    description: row.description.clone(),
                    membership: row.membership,
                })
                .collect();
            let length = rows.len();
            let envelope = serde_json::json!({"version":"1","request_id":operation_id(request,"prepareVocabulary",prepared_round,"0"),"model_id":request.model_id,"phase":"vocabulary","rows":rows});
            let encoded = serde_json::to_string(&envelope)
                .map_err(|_| ClassifierError::InvalidArtifact("vocabulary_envelope".into()))?;
            if encoded.len() <= MAX_TRAINING_ENVELOPE_BYTES {
                break (encoded, length);
            }
            if count == 1 {
                return Err(ClassifierError::ModelTooLarge);
            }
            count = count.div_ceil(2);
        };
        if prepared_count > 0 {
            let response = worker.call(BridgeOperation::TrainBatch, &request_json, control)?;
            let response: serde_json::Value = serde_json::from_str(&response)
                .map_err(|_| ClassifierError::InvalidArtifact("vocabulary_response".into()))?;
            if response["prepared"].as_u64() != Some(prepared_count as u64) {
                return Err(ClassifierError::BridgeProtocol {
                    operation: "trainBatch".into(),
                    code: "vocabulary_preparation_failed".into(),
                });
            }
        }
        prepared_offset += count;
        prepared_round += 1;
    }
    let freeze = serde_json::json!({"version":"1","request_id":operation_id(request,"freezeVocabulary",0,"0"),"model_id":request.model_id,"phase":"freeze_vocabulary","rows":[]});
    let frozen = worker.call(BridgeOperation::TrainBatch, &freeze.to_string(), control)?;
    let frozen: serde_json::Value = serde_json::from_str(&frozen)
        .map_err(|_| ClassifierError::InvalidArtifact("vocabulary_freeze_response".into()))?;
    if frozen["vocabulary_frozen"] != true {
        return Err(ClassifierError::BridgeProtocol {
            operation: "trainBatch".into(),
            code: "vocabulary_freeze_failed".into(),
        });
    }

    let mut next_row = 0usize;
    let mut round: u64 = 0;
    let mut rows_seen: u64 = 0;
    let mut rows_fitted: u64 = 0;
    let mut total_token_bytes: u64 = 0;
    let mut current_memberships = [0u64; 6];
    let mut current_vocab_sizes = [0u64; 6];
    let mut current_revisions = [0u64; 6];
    let mut bridge_round = 0u64;
    let mut fit_step = 0u64;

    while next_row < train_reps.len() {
        control.check_active()?;
        let pending_round = round + 1;
        let mut wire_bytes =
            serde_json::to_vec(&training_batch_request(request, pending_round, Vec::new()))
                .map_err(|error| ClassifierError::InvalidTrainingData {
                    row_id: None,
                    reason: format!("serialize training envelope: {error}"),
                })?
                .len();
        let mut training_rows = Vec::with_capacity(TRAINING_BATCH_SIZE);
        while next_row < train_reps.len() && training_rows.len() < TRAINING_BATCH_SIZE {
            let row = train_reps[next_row];
            if row
                .membership
                .iter()
                .all(|membership| *membership == LabelMembership::Unknown)
            {
                next_row += 1;
                rows_seen += 1;
                continue;
            }
            let training_row = TrainingRow {
                row_id: row.sample_id.clone(),
                feature_input_hash: super::ClassifyInput::new(
                    "vocabulary",
                    &row.title,
                    &row.description,
                    None,
                )
                .input_hash,
                title: row.title.clone(),
                description: row.description.clone(),
                membership: row.membership,
            };
            let row_bytes = serde_json::to_vec(&training_row)
                .map_err(|error| ClassifierError::InvalidTrainingData {
                    row_id: Some(row.sample_id.clone()),
                    reason: format!("serialize training row: {error}"),
                })?
                .len();
            let proposed = wire_bytes
                .checked_add(row_bytes)
                .and_then(|bytes| bytes.checked_add(usize::from(!training_rows.is_empty())))
                .ok_or(ClassifierError::ModelTooLarge)?;
            if proposed > MAX_TRAINING_ENVELOPE_BYTES {
                if training_rows.is_empty() {
                    return Err(ClassifierError::InvalidTrainingData {
                        row_id: Some(row.sample_id.clone()),
                        reason: "single_training_row_exceeds_envelope_bound".into(),
                    });
                }
                break;
            }
            wire_bytes = proposed;
            training_rows.push(training_row);
            next_row += 1;
            rows_seen += 1;
        }
        if !training_rows.is_empty() {
            round += 1;
            prog.round = round;
            bridge_round += 1;
            let requested_memberships: [u64; 6] = std::array::from_fn(|domain| {
                training_rows
                    .iter()
                    .filter(|row| row.membership[domain] != LabelMembership::Unknown)
                    .count() as u64
            });
            let before_round = current_memberships;
            let batch_req = training_batch_request(request, round, training_rows);

            let req_str = serde_json::to_string(&batch_req).map_err(|e| {
                ClassifierError::InvalidTrainingData {
                    row_id: None,
                    reason: format!("failed to serialize TrainBatchRequest: {e}"),
                }
            })?;

            if req_str.len() > MAX_TRAINING_ENVELOPE_BYTES || req_str.len() != wire_bytes {
                return Err(ClassifierError::BridgeProtocol {
                    operation: "trainBatch".into(),
                    code: "host_training_envelope_bound_mismatch".into(),
                });
            }
            let resp_str = worker.call(BridgeOperation::TrainBatch, &req_str, control)?;
            let batch_resp: TrainBatchResponse =
                serde_json::from_str(&resp_str).map_err(|e| ClassifierError::BridgeProtocol {
                    operation: "TrainBatch".into(),
                    code: format!("invalid TrainBatchResponse JSON: {e}"),
                })?;

            if !batch_resp.errors.is_empty() {
                return Err(ClassifierError::BridgeProtocol {
                    operation: "TrainBatch".into(),
                    code: batch_resp.errors[0].code.clone(),
                });
            }

            if batch_resp.version != "1"
                || batch_resp.request_id != batch_req.request_id
                || batch_resp.accepted.checked_add(batch_resp.skipped)
                    != Some(batch_req.rows.len() as u64)
                || batch_resp.round != bridge_round
                || batch_resp.fitted_memberships != current_memberships
                || batch_resp.progress != 1.0
                || batch_resp
                    .vocabulary_sizes
                    .iter()
                    .zip(current_vocab_sizes)
                    .any(|(next, prior)| *next < prior)
                || batch_resp
                    .feature_order_revisions
                    .iter()
                    .zip(current_revisions)
                    .any(|(next, prior)| *next < prior)
            {
                return Err(ClassifierError::BridgeProtocol {
                    operation: "trainBatch".into(),
                    code: "invalid_batch_response_identity_or_counts".into(),
                });
            }
            if batch_resp.skipped != 0 {
                return Err(ClassifierError::InvalidTrainingData {
                    row_id: None,
                    reason: "supervised_training_rows_have_no_usable_tokens".into(),
                });
            }
            total_token_bytes = total_token_bytes
                .checked_add(batch_resp.token_bytes)
                .ok_or(ClassifierError::InvalidTrainingData {
                    row_id: None,
                    reason: "token_bytes_overflow".into(),
                })?;
            if total_token_bytes > MAX_RETAINED_TOKEN_BYTES as u64
                || batch_resp
                    .vocabulary_sizes
                    .iter()
                    .any(|n| *n > MAX_TRAINING_FEATURES_PER_DOMAIN as u64)
            {
                return Err(ClassifierError::InvalidTrainingData {
                    row_id: None,
                    reason: "retained_training_data_exceeds_bound".into(),
                });
            }
            current_vocab_sizes = batch_resp.vocabulary_sizes;
            current_revisions = batch_resp.feature_order_revisions;
            let mut round_memberships = 0u64;

            // Advance finish cursor until done
            let mut step: u64 = 0;
            loop {
                control.check_active()?;
                step += 1;
                let finish_req = FinishTrainingRequest {
                    version: "1".into(),
                    request_id: operation_id(request, "finishTraining", round, &step.to_string()),
                    model_id: request.model_id.as_str().into(),
                    freeze_feature_order: true,
                    max_documents_per_step: FIT_DOCUMENTS_PER_STEP,
                    done_when: 0,
                };

                let finish_req_str = serde_json::to_string(&finish_req).map_err(|e| {
                    ClassifierError::InvalidTrainingData {
                        row_id: None,
                        reason: format!("failed to serialize FinishTrainingRequest: {e}"),
                    }
                })?;

                let finish_resp_str =
                    worker.call(BridgeOperation::FinishTraining, &finish_req_str, control)?;
                let finish_resp: FinishTrainingResponse = serde_json::from_str(&finish_resp_str)
                    .map_err(|e| ClassifierError::BridgeProtocol {
                        operation: "FinishTraining".into(),
                        code: format!("invalid FinishTrainingResponse JSON: {e}"),
                    })?;

                if !finish_resp.errors.is_empty() {
                    return Err(ClassifierError::BridgeProtocol {
                        operation: "FinishTraining".into(),
                        code: finish_resp.errors[0].code.clone(),
                    });
                }

                let delta: Option<u64> = finish_resp
                    .fitted_memberships
                    .iter()
                    .zip(current_memberships)
                    .try_fold(0u64, |total, (next, prior)| {
                        total.checked_add(next.checked_sub(prior)?)
                    });
                let valid = delta.is_some_and(|count| count <= FIT_DOCUMENTS_PER_STEP as u64)
                    && finish_resp.version == "1"
                    && finish_resp.request_id == finish_req.request_id
                    && finish_resp.round == bridge_round
                    && finish_resp.vocabulary_sizes == current_vocab_sizes
                    && finish_resp.feature_order_revisions == current_revisions
                    && finish_resp.frozen_order_length == current_vocab_sizes.iter().sum::<u64>()
                    && finish_resp.fit_step
                        == fit_step + u64::from(delta.is_some_and(|count| count > 0))
                    && finish_resp
                        .fitted_memberships
                        .iter()
                        .enumerate()
                        .all(|(domain, next)| {
                            *next <= MAX_RETAINED_DOCUMENTS_PER_DOMAIN as u64
                                && *next <= before_round[domain] + requested_memberships[domain]
                        });
                if !valid || (!finish_resp.done && delta == Some(0)) {
                    return Err(ClassifierError::BridgeProtocol {
                        operation: "finishTraining".into(),
                        code: "invalid_fit_response_identity_or_progress".into(),
                    });
                }
                round_memberships += delta.expect("validated fit delta");
                current_memberships = finish_resp.fitted_memberships;
                fit_step = finish_resp.fit_step;
                if finish_resp.done {
                    let fully_accepted = batch_resp.skipped == 0;
                    if round_memberships < batch_resp.accepted
                        || (fully_accepted
                            && current_memberships
                                .iter()
                                .enumerate()
                                .any(|(domain, next)| {
                                    *next != before_round[domain] + requested_memberships[domain]
                                }))
                    {
                        return Err(ClassifierError::BridgeProtocol {
                            operation: "finishTraining".into(),
                            code: "premature_fit_completion".into(),
                        });
                    }
                    rows_fitted += batch_resp.accepted;
                    break;
                }
            }
        }

        prog.rows_seen = rows_seen;
        prog.rows_fitted = rows_fitted;
        prog.token_bytes = total_token_bytes;
        prog.fitted_memberships = current_memberships;
        prog.vocabulary_sizes = current_vocab_sizes;
        progress(&prog);
    }

    if round == 0 {
        return Err(ClassifierError::InvalidTrainingData {
            row_id: None,
            reason: "no_supervised_training_rows".into(),
        });
    }

    // 4. Serialize Candidate Payload
    control.check_active()?;
    let serialize_req = serde_json::json!({
        "version": "1",
        "request_id": operation_id(request, "serializeModel", 0, "0"),
        "model_id": request.model_id.as_str(),
    });
    let mut artifact_control = control.clone();
    artifact_control.execution_timeout = control.artifact_execution_timeout;
    let payload = worker.call(
        BridgeOperation::SerializeModel,
        &serialize_req.to_string(),
        &artifact_control,
    )?;

    // 5. Calibration & Evaluation on Calibration split
    control.check_active()?;
    prog.stage = TrainingStage::Calibrating;
    progress(&prog);

    let mut calib_reps: Vec<&DatasetRow> = calib_families
        .into_values()
        .chain(development_families.into_values())
        .collect();
    calib_reps.sort_by(|a, b| a.sample_id.cmp(&b.sample_id));

    let mut scored_families = Vec::with_capacity(calib_reps.len());
    for rep in &calib_reps {
        control.check_active()?;
        // Classify calibration representative
        let classify_req = serde_json::json!({
            "version": "1",
            "request_id": operation_id(request, "classifyBatch", 0, &rep.sample_id),
            "model_id": request.model_id.as_str(),
            "items": [{
                "id": rep.sample_id,
                "title": rep.title,
                "description": rep.description,
            }],
        });

        let classify_resp_str = worker.call(
            BridgeOperation::ClassifyBatch,
            &classify_req.to_string(),
            control,
        )?;
        let parsed_resp: serde_json::Value =
            serde_json::from_str(&classify_resp_str).map_err(|e| {
                ClassifierError::BridgeProtocol {
                    operation: "ClassifyBatch".into(),
                    code: format!("failed to parse classify response: {e}"),
                }
            })?;

        let scores_array = parsed_resp["items"][0]["results"]
            .as_array()
            .ok_or_else(|| ClassifierError::BridgeProtocol {
                operation: "ClassifyBatch".into(),
                code: "missing scores array in response".into(),
            })?;

        if scores_array.len() != 6 {
            return Err(ClassifierError::BridgeProtocol {
                operation: "ClassifyBatch".into(),
                code: format!("expected 6 scores, got {}", scores_array.len()),
            });
        }

        let mut raw_scores = core::array::from_fn(|idx| RawDomainScore {
            domain: DomainId::ALL[idx],
            present_log_score: 0.0,
            absent_log_score: 0.0,
            raw_log_odds: 0.0,
            known_feature_count: 0,
            oov_count: 0,
        });

        let protocol_error = |field: &str| ClassifierError::BridgeProtocol {
            operation: "classifyBatch".into(),
            code: format!("invalid_or_missing_{field}"),
        };
        if parsed_resp["items"].as_array().map(Vec::len) != Some(1)
            || parsed_resp["items"][0]["id"].as_str() != Some(rep.sample_id.as_str())
            || parsed_resp["request_id"].as_str() != classify_req["request_id"].as_str()
            || parsed_resp["version"].as_str() != Some("1")
        {
            return Err(protocol_error("response_identity"));
        }
        for (idx, score_val) in scores_array.iter().enumerate() {
            if score_val["domain"].as_str() != Some(DomainId::ALL[idx].as_str()) {
                return Err(protocol_error("domain"));
            }
            let finite = |field: &str| {
                score_val[field]
                    .as_f64()
                    .filter(|n| n.is_finite())
                    .ok_or_else(|| protocol_error(field))
            };
            let count = |field: &str| {
                score_val[field]
                    .as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                    .ok_or_else(|| protocol_error(field))
            };
            raw_scores[idx] = RawDomainScore {
                domain: DomainId::ALL[idx],
                present_log_score: finite("present_log_score")?,
                absent_log_score: finite("absent_log_score")?,
                raw_log_odds: finite("raw_log_odds")?,
                known_feature_count: count("known_feature_count")?,
                oov_count: count("oov_count")?,
            };
        }

        scored_families.push(ScoredFamily {
            family_id: rep.family_id.clone(),
            representative_id: rep.sample_id.clone(),
            split: rep.split,
            provenance: rep.provenance.clone(),
            membership: rep.membership,
            raw: raw_scores,
        });
    }

    // Fit isotonic transform and gate set
    let calibration_rows: Vec<_> = scored_families
        .iter()
        .filter(|row| row.split == Split::Calibration)
        .cloned()
        .collect();
    let development_rows: Vec<_> = scored_families
        .iter()
        .filter(|row| row.split == Split::Development)
        .cloned()
        .collect();
    let mut calibration_fit = super::calibration::fit(&calibration_rows, &CalibrationPolicy)?;
    let isotonic_brier =
        super::calibration::brier_score(&development_rows, &calibration_fit.gates)?;
    let mut calibrator_comparison = vec![CalibratorComparison {
        kind: "isotonic".into(),
        failure: None,
        development_brier: isotonic_brier,
        development_macro_f1: None,
        selected: true,
    }];
    if !development_rows.is_empty() {
        let baseline = super::calibration::evaluate(&development_rows, &calibration_fit.gates)?;
        calibrator_comparison[0].development_macro_f1 = Some(baseline.macro_f1);
        match super::calibration::fit_sigmoid(&calibration_rows) {
            Ok(candidate) => {
                let candidate_metrics =
                    super::calibration::evaluate(&development_rows, &candidate.gates)?;
                let candidate_brier =
                    super::calibration::brier_score(&development_rows, &candidate.gates)?;
                let choose = candidate_brier
                    .zip(isotonic_brier)
                    .is_some_and(|(a, b)| a < b)
                    && candidate_metrics.macro_f1 >= baseline.macro_f1
                    && super::calibration::selection_nonregression(&candidate_metrics, &baseline)
                    && candidate_metrics.false_oos_admissions <= baseline.false_oos_admissions
                    && candidate_metrics.invalid_selected_pairs <= baseline.invalid_selected_pairs
                    && candidate_metrics
                        .domains
                        .iter()
                        .zip(&baseline.domains)
                        .all(|(a, b)| !b.accepted || a.accepted);
                calibrator_comparison.push(CalibratorComparison {
                    kind: "regularized_monotone_sigmoid".into(),
                    failure: None,
                    development_brier: candidate_brier,
                    development_macro_f1: Some(candidate_metrics.macro_f1),
                    selected: choose,
                });
                if choose {
                    calibrator_comparison[0].selected = false;
                    calibration_fit = candidate;
                }
            }
            Err(ClassifierError::InvalidCalibrationData { reason, .. }) => {
                calibrator_comparison.push(CalibratorComparison {
                    kind: "regularized_monotone_sigmoid".into(),
                    development_brier: None,
                    development_macro_f1: None,
                    selected: false,
                    failure: Some(reason),
                });
            }
            Err(error) => return Err(error),
        }
    }
    // Legacy corpora have no Development partition: identify their evaluation as
    // fit-only instead of claiming independent support. Never open Test here.
    let evaluation_split = if development_rows.is_empty() {
        Split::Calibration
    } else {
        Split::Development
    };
    let evaluation_rows = if development_rows.is_empty() {
        &calibration_rows
    } else {
        &development_rows
    };
    let evaluation_metrics = super::calibration::evaluate(evaluation_rows, &calibration_fit.gates)?;

    // 6. Restore smoke check
    control.check_active()?;
    prog.stage = TrainingStage::ValidatingRestore;
    progress(&prog);

    worker.restore_completed_candidate(&request.model_id, &payload, &artifact_control)?;

    // Compute class and family counts
    let mut class_counts = core::array::from_fn(|idx| DomainClassCount {
        domain: DomainId::ALL[idx],
        present: 0,
        absent: 0,
    });
    let mut family_counts = core::array::from_fn(|idx| DomainFamilyCount {
        domain: DomainId::ALL[idx],
        accepted_families: 0,
    });

    for row in &train_reps {
        for (idx, m) in row.membership.iter().enumerate() {
            match m {
                LabelMembership::Present => class_counts[idx].present += 1,
                LabelMembership::Absent => class_counts[idx].absent += 1,
                LabelMembership::Unknown => {}
            }
        }
    }
    for (idx, acceptance) in evaluation_metrics.domains.iter().enumerate() {
        family_counts[idx].accepted_families = acceptance.accepted_families;
    }

    // 7. Finished Report Construction
    prog.stage = TrainingStage::Finished;
    progress(&prog);

    let report = TrainingReport {
        calibrator_comparison,
        schema_version: TRAINING_REPORT_SCHEMA_VERSION.into(),
        request_id: request.request_id.clone(),
        model_id: request.model_id.clone(),
        provenance: request.provenance.clone(),
        class_counts,
        family_counts,
        gate_set: calibration_fit.gates,
        fit_diagnostics: Some(calibration_fit.metrics),
        evaluation_split,
        evaluation: evaluation_metrics.clone(),
        domain_decisions: evaluation_metrics.domains,
    };

    Ok(TrainedCandidate {
        model_id: request.model_id.clone(),
        payload,
        report,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_row(
        sample_id: &str,
        family_id: &str,
        split: Split,
        memberships: [LabelMembership; 6],
    ) -> DatasetRow {
        let labels_complete = memberships.iter().all(|m| *m != LabelMembership::Unknown);
        DatasetRow {
            sample_id: sample_id.to_string(),
            family_id: FamilyId::parse(family_id).unwrap(),
            split,
            title: format!("Title for {sample_id}"),
            description: format!("Description for {sample_id}"),
            language: "en".to_string(),
            membership: memberships,
            labels_complete,
            provenance: Provenance::synthetic_baseline("v1", "0".repeat(64)),
        }
    }

    #[test]
    fn test_family_id_validation() {
        assert!(FamilyId::parse("fam-123").is_ok());
        assert!(FamilyId::parse("").is_err());
        assert!(FamilyId::parse(&"a".repeat(257)).is_err());
        assert!(FamilyId::parse("fam\0bad").is_err());
        assert!(FamilyId::parse("fam\nbad").is_err());
    }

    #[test]
    fn test_corpus_hashes_deterministic() {
        let rows = vec![
            sample_row("s1", "f1", Split::Train, [LabelMembership::Present; 6]),
            sample_row("s2", "f2", Split::Calibration, [LabelMembership::Absent; 6]),
            sample_row("s3", "f3", Split::Test, [LabelMembership::Present; 6]),
        ];

        let h1 = corpus_hashes(&rows).unwrap();
        let h2 = corpus_hashes(&rows).unwrap();
        assert_eq!(h1, h2);
        assert_eq!(h1.train_split_sha256.len(), 64);
        assert_eq!(h1.calibration_split_sha256.len(), 64);
        assert_eq!(h1.test_split_sha256.len(), 64);
        assert_eq!(h1.family_assignment_sha256.len(), 64);
    }

    #[test]
    fn test_validate_corpus_conflict_rejection() {
        let rows = vec![
            sample_row("s1", "f1", Split::Train, [LabelMembership::Present; 6]),
            sample_row("s2", "f1", Split::Train, [LabelMembership::Absent; 6]),
        ];

        let h = corpus_hashes(&rows).unwrap_or(CorpusHashes {
            train_split_sha256: "0".repeat(64),
            calibration_split_sha256: "0".repeat(64),
            development_split_sha256: "0".repeat(64),
            test_split_sha256: "0".repeat(64),
            family_assignment_sha256: "0".repeat(64),
        });

        let corpus = TrainingCorpus {
            rows,
            dataset_sha256: "a".repeat(64),
            train_split_sha256: h.train_split_sha256,
            calibration_split_sha256: h.calibration_split_sha256,
            development_split_sha256: h.development_split_sha256,
            test_split_sha256: h.test_split_sha256,
            family_assignment_sha256: h.family_assignment_sha256,
        };

        match validate_corpus(&corpus) {
            Err(ClassifierError::InvalidTrainingData { reason, .. }) => {
                assert!(reason.contains("conflicting_family_membership"));
            }
            other => panic!("expected InvalidTrainingData, got {:?}", other),
        }
    }
}

#[cfg(test)]
mod packing_bound_tests {
    use super::*;
    #[test]
    fn closed_training_header_has_the_bounded_frame_used_by_replay_proof() {
        let request = TrainRequest {
            request_id: "\"\\".repeat(128),
            model_id: ModelId::parse(&"m".repeat(64)).unwrap(),
            provenance: Provenance::synthetic_baseline("frame-fixture", "0".repeat(64)),
            synthetic_activation: SyntheticActivation::RetainPrior,
        };
        for round in [1, u64::MAX] {
            let header = training_batch_request(&request, round, Vec::new());
            assert!(serde_json::to_vec(&header).unwrap().len() <= 1024);
            assert_eq!(header.request_id.len(), "trainBatch:".len() + 64);
        }
    }
}

/// Builds the immutable manifest identically for manual and durable candidates.
pub fn candidate_manifest(
    corpus: &TrainingCorpus,
    request: &TrainRequest,
    candidate: &TrainedCandidate,
) -> Result<super::artifact::ModelManifest, ClassifierError> {
    let identity = super::compiled_identity()?;
    let report_bytes = super::artifact::bounded_json_bytes(&candidate.report)?;
    let manifest = super::artifact::ModelManifest {
        schema_version: super::artifact::MANIFEST_SCHEMA_VERSION.into(),
        activation_generation: 0,
        model_id: request.model_id.clone(),
        previous_model_id: None,
        activated_at: String::new(),
        model_sha256: super::artifact::sha256_hex(candidate.payload.as_bytes()),
        manifest_sha256: String::new(),
        report_sha256: super::artifact::sha256_hex(&report_bytes),
        dataset_sha256: corpus.dataset_sha256.clone(),
        train_split_sha256: corpus.train_split_sha256.clone(),
        calibration_split_sha256: corpus.calibration_split_sha256.clone(),
        development_split_sha256: corpus.development_split_sha256.clone(),
        test_split_sha256: corpus.test_split_sha256.clone(),
        family_assignment_sha256: corpus.family_assignment_sha256.clone(),
        taxonomy_version: super::TAXONOMY_VERSION.into(),
        domains: super::DomainId::ALL.to_vec(),
        algorithm_version: super::ALGORITHM_VERSION.into(),
        preprocessing_version: super::PREPROCESSING_VERSION.into(),
        feature_schema_version: super::artifact::FEATURE_SCHEMA_VERSION.into(),
        admission_policy_version: super::ADMISSION_POLICY_VERSION.into(),
        natural_version: identity.natural_version,
        apparatus_version: identity.apparatus_version,
        bundle_sha256: identity.bundle_sha256,
        bridge_version: identity.bridge_version,
        bridge_sha256: identity.bridge_sha256,
        adapter_sha256: identity.adapter_sha256,
        source_manifest_sha256: identity.source_manifest_sha256,
        smoothing: 1.0,
        thresholds: candidate
            .report
            .gate_set
            .domains
            .iter()
            .map(|d| super::artifact::DomainThreshold {
                domain: d.domain,
                threshold: d.threshold,
            })
            .collect(),
        class_counts: candidate.report.class_counts.to_vec(),
        family_counts: candidate.report.family_counts.to_vec(),
        metrics: serde_json::to_value(&candidate.report).map_err(|_| {
            ClassifierError::InvalidArtifact("training_report_serialization_failed".into())
        })?,
        provenance: request.provenance.clone(),
        activation_reason: super::artifact::ActivationReason::ManualSeed,
    };
    Ok(manifest)
}
