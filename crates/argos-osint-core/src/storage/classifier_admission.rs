//! Revision-fenced, run-scoped local Atlas admission. @tag:classifier @tag:atlas-collection
//!
//! The receipt and current projection commit with each occurrence outcome. A
//! run lease authorizes dispatch; an occurrence lease authorizes its result.
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use surrealdb::types::SurrealValue;

use super::codec;
use super::repo::Repository;
use super::tx::{run_tx, RetryPolicy, TxError};
use crate::atlas_classify::{OccurrenceKey, OccurrenceProvenance, StoredReceipt};
use crate::atlas_metadata::ArticleMetadata;
use crate::classifier::readiness::{CyclePermit, CycleRequirement};
use crate::classifier::ClassifierError;
use crate::store::AtlasArticleRow;

pub const PUBLICATION_VERSION: &str = "atlas-admission-publication-v4";
pub const PAGE_VERSION: &str = "atlas-admission-page-v1";
pub const FOREGROUND_PAGE_ROWS: usize = 512;
pub const COMMIT_BATCH_ROWS: usize = 64;
// Publishing each occurrence writes its receipt, source, projection and native
// stages. Shorter transactions leave the independent five-second heartbeat a
// commit window while hydration and inference retain their 64-row batches.
const PUBLICATION_TRANSACTION_ROWS: usize = 16;
pub const CLAIM_LEASE_SECS: i64 = 30;
pub const HEARTBEAT_SECS: u64 = 5;
pub const MAX_REVISION_ATTEMPTS_PER_ACTIVATION: u8 = 1;
pub const MAX_PAYLOAD_BYTES: usize = 64 * 1024;
pub const MAX_RECEIPT_BYTES: usize = 128 * 1024;
pub const MAX_PROVENANCE_BYTES: usize = 16 * 1024;
pub const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

pub const SCHEMA: &str = "DEFINE TABLE IF NOT EXISTS atlas_classification_receipt SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS run_id ON atlas_classification_receipt TYPE string;
DEFINE FIELD IF NOT EXISTS article_id ON atlas_classification_receipt TYPE string;
DEFINE FIELD IF NOT EXISTS input_revision ON atlas_classification_receipt TYPE int;
DEFINE FIELD IF NOT EXISTS source_revision ON atlas_classification_receipt TYPE string;
DEFINE FIELD IF NOT EXISTS classifier_input_hash ON atlas_classification_receipt TYPE string;
DEFINE FIELD IF NOT EXISTS model_id ON atlas_classification_receipt TYPE string;
DEFINE FIELD IF NOT EXISTS artifact_sha256 ON atlas_classification_receipt TYPE string;
DEFINE FIELD IF NOT EXISTS manifest_sha256 ON atlas_classification_receipt TYPE string;
DEFINE FIELD IF NOT EXISTS taxonomy_version ON atlas_classification_receipt TYPE string;
DEFINE FIELD IF NOT EXISTS preprocessing_version ON atlas_classification_receipt TYPE string;
DEFINE FIELD IF NOT EXISTS policy_version ON atlas_classification_receipt TYPE string;
DEFINE FIELD IF NOT EXISTS calibration_version ON atlas_classification_receipt TYPE string;
DEFINE FIELD IF NOT EXISTS receipt_schema_version ON atlas_classification_receipt TYPE string;
DEFINE FIELD IF NOT EXISTS publication_version ON atlas_classification_receipt TYPE string;
DEFINE FIELD IF NOT EXISTS receipt_json ON atlas_classification_receipt TYPE string;
DEFINE FIELD IF NOT EXISTS receipt_sha256 ON atlas_classification_receipt TYPE string;
DEFINE FIELD IF NOT EXISTS provenance_json ON atlas_classification_receipt TYPE string;
DEFINE FIELD IF NOT EXISTS evaluated_at ON atlas_classification_receipt TYPE string;
DEFINE INDEX IF NOT EXISTS atlas_receipt_occurrence ON atlas_classification_receipt FIELDS run_id, article_id, input_revision;
DEFINE INDEX IF NOT EXISTS atlas_receipt_model ON atlas_classification_receipt FIELDS model_id;
DEFINE TABLE IF NOT EXISTS atlas_classification_projection SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS run_id ON atlas_classification_projection TYPE string;
DEFINE FIELD IF NOT EXISTS article_id ON atlas_classification_projection TYPE string;
DEFINE FIELD IF NOT EXISTS receipt_key ON atlas_classification_projection TYPE string;
DEFINE FIELD IF NOT EXISTS taxonomy_version ON atlas_classification_projection TYPE string;
DEFINE FIELD IF NOT EXISTS admission_state ON atlas_classification_projection TYPE string;
DEFINE FIELD IF NOT EXISTS publication_version ON atlas_classification_projection TYPE string;
DEFINE FIELD IF NOT EXISTS input_revision ON atlas_classification_projection TYPE int;
DEFINE FIELD IF NOT EXISTS source_revision ON atlas_classification_projection TYPE string;
DEFINE FIELD IF NOT EXISTS updated_at ON atlas_classification_projection TYPE datetime;
DEFINE INDEX IF NOT EXISTS atlas_projection_run ON atlas_classification_projection FIELDS run_id, article_id;
DEFINE TABLE IF NOT EXISTS atlas_classification_derivative SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS run_id ON atlas_classification_derivative TYPE string;
DEFINE FIELD IF NOT EXISTS article_id ON atlas_classification_derivative TYPE string;
DEFINE FIELD IF NOT EXISTS stage ON atlas_classification_derivative TYPE string;
DEFINE FIELD IF NOT EXISTS input_revision ON atlas_classification_derivative TYPE int;
DEFINE FIELD IF NOT EXISTS receipt_key ON atlas_classification_derivative TYPE string;
DEFINE FIELD IF NOT EXISTS state ON atlas_classification_derivative TYPE string;
DEFINE FIELD IF NOT EXISTS checkpoint_json ON atlas_classification_derivative TYPE string;
DEFINE FIELD IF NOT EXISTS owner ON atlas_classification_derivative TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS epoch ON atlas_classification_derivative TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS lease_until ON atlas_classification_derivative TYPE option<datetime>;
DEFINE FIELD IF NOT EXISTS last_error ON atlas_classification_derivative TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS updated_at ON atlas_classification_derivative TYPE datetime;
DEFINE FIELD IF NOT EXISTS disposition_json ON atlas_classification_derivative TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS captured_json ON atlas_classification_derivative TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS output_revision ON atlas_classification_derivative TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS last_activation ON atlas_classification_derivative TYPE string DEFAULT '';
DEFINE INDEX IF NOT EXISTS atlas_derivative_work ON atlas_classification_derivative FIELDS state, run_id, article_id;
DEFINE INDEX IF NOT EXISTS atlas_derivative_occurrence_stage ON atlas_classification_derivative FIELDS run_id, article_id, stage, input_revision, receipt_key;
DEFINE TABLE IF NOT EXISTS atlas_classifier_run SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS owner ON atlas_classifier_run TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS epoch ON atlas_classifier_run TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS lease_until ON atlas_classifier_run TYPE option<datetime>;
DEFINE FIELD IF NOT EXISTS revision ON atlas_classifier_run TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS work_generation ON atlas_classifier_run TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS admission_generation ON atlas_classifier_run TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS closed_generation ON atlas_classifier_run TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS page_json ON atlas_classifier_run TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS pin_json ON atlas_classifier_run TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS manifest_sha256 ON atlas_classifier_run TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS terminal_receipt_json ON atlas_classifier_run TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS state ON atlas_classifier_run TYPE string DEFAULT 'pending';
DEFINE FIELD IF NOT EXISTS updated_at ON atlas_classifier_run TYPE datetime;
DEFINE INDEX IF NOT EXISTS atlas_classifier_running ON atlas_classifier_run FIELDS state, lease_until;
DEFINE TABLE IF NOT EXISTS atlas_classifier_guard SCHEMALESS;
DEFINE FIELD IF NOT EXISTS atlas_publication_guard ON nlp_feature TYPE int DEFAULT 0;
DEFINE TABLE IF NOT EXISTS atlas_classifier_intent SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS owner ON atlas_classifier_intent TYPE string;
DEFINE FIELD IF NOT EXISTS epoch ON atlas_classifier_intent TYPE int;
DEFINE FIELD IF NOT EXISTS lease_until ON atlas_classifier_intent TYPE datetime;
DEFINE FIELD IF NOT EXISTS updated_at ON atlas_classifier_intent TYPE datetime;
DEFINE INDEX IF NOT EXISTS atlas_classifier_intent_live ON atlas_classifier_intent FIELDS lease_until;";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RunClaim {
    pub run_id: String,
    pub owner: String,
    pub epoch: u64,
    pub revision: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct AdmissionClaim {
    pub occurrence: OccurrenceKey,
    pub owner: String,
    pub epoch: u64,
    pub input_revision: u64,
    pub source_revision: String,
    pub classifier_input_hash: String,
    pub work_seq: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct AdmissionPageToken {
    pub version: String,
    pub run_id: String,
    pub activation_id: String,
    pub admission_generation: u64,
    pub through_seq: u64,
    pub after_seq: u64,
    pub model_id: String,
    pub manifest_sha256: String,
}
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct AdmissionPage {
    pub inputs: Vec<ClaimedInput>,
    pub next: Option<AdmissionPageToken>,
    pub pending: u64,
    pub deferred: u64,
}
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ClaimedInput {
    pub claim: AdmissionClaim,
    pub article: AtlasArticleRow,
    pub metadata: Option<ArticleMetadata>,
    pub body_revision: Option<BodyRevision>,
    pub provenance: OccurrenceProvenance,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct BodyRevision {
    pub source_id: String,
    pub revision: String,
    pub artifact_hash: String,
    pub candidates_revision: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ItemCommit {
    Committed {
        receipt_key: String,
    },
    AlreadyCommitted {
        receipt_key: String,
    },
    StaleInput {
        occurrence: OccurrenceKey,
        current_revision: Option<u64>,
    },
    LeaseLost {
        occurrence: OccurrenceKey,
    },
    Cancelled {
        occurrence: OccurrenceKey,
    },
    Retryable {
        occurrence: OccurrenceKey,
        code: String,
    },
    InvalidReceipt {
        occurrence: OccurrenceKey,
        code: String,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct BatchCommit {
    pub items: Vec<ItemCommit>,
    pub retries: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ClassificationView {
    pub schema_version: String,
    pub occurrence: OccurrenceKey,
    pub taxonomy_version: String,
    pub admission_state: String,
    pub publication_version: String,
    pub receipt: Option<StoredReceipt>,
    pub legacy: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct TaxonomyCoverage {
    pub scope: String,
    pub retained_occurrences: u64,
    pub current_admitted: u64,
    pub current_rejected: u64,
    pub legacy_unmigrated: u64,
    pub missing_original_metadata: u64,
    pub conflicted: u64,
    pub taxonomy_version: String,
}
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum AdmissionError {
    Storage(TxError),
    CleanupUncertain {
        operation: Box<TxError>,
        cleanup: Box<TxError>,
    },
    Classifier(ClassifierError),
    UnsupportedVersion {
        kind: String,
        found: String,
    },
    InvalidRunState {
        code: String,
    },
    InvalidPage {
        code: String,
    },
    InvalidReceipt {
        code: String,
    },
    ReceiptConflict {
        key: String,
    },
    LeaseLost,
    Cancelled,
    PayloadTooLarge {
        kind: String,
        limit: usize,
    },
}
impl fmt::Display for AdmissionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Storage(_) => f.write_str("Atlas admission storage operation failed"),
            Self::CleanupUncertain { .. } => {
                f.write_str("Atlas database cleanup could not be confirmed; this owner has stopped")
            }
            Self::Classifier(_) => f.write_str("Atlas local classifier operation failed"),
            Self::UnsupportedVersion { kind, .. } => {
                write!(f, "unsupported Atlas admission {kind} version")
            }
            Self::InvalidRunState { code } => write!(f, "invalid Atlas admission run: {code}"),
            Self::InvalidPage { code } => write!(f, "invalid Atlas admission page: {code}"),
            Self::InvalidReceipt { code } => write!(f, "invalid Atlas admission receipt: {code}"),
            Self::ReceiptConflict { .. } => {
                f.write_str("Atlas admission receipt integrity conflict")
            }
            Self::LeaseLost => f.write_str("Atlas admission lease was lost"),
            Self::Cancelled => f.write_str("Atlas admission was cancelled"),
            Self::PayloadTooLarge { kind, limit } => {
                write!(f, "Atlas admission {kind} exceeds {limit} bytes")
            }
        }
    }
}
impl std::error::Error for AdmissionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Storage(error) => Some(error),
            Self::CleanupUncertain { cleanup, .. } => Some(cleanup.as_ref()),
            Self::Classifier(error) => Some(error),
            _ => None,
        }
    }
}
impl From<TxError> for AdmissionError {
    fn from(value: TxError) -> Self {
        Self::Storage(value)
    }
}
impl From<ClassifierError> for AdmissionError {
    fn from(value: ClassifierError) -> Self {
        Self::Classifier(value)
    }
}

/// Operation-local counters; never inferred by subtracting a process-global metric.
#[derive(Debug, Clone, Default)]
pub struct DbMetrics {
    round_trips: Arc<AtomicU64>,
    conflict_retries: Arc<AtomicU64>,
    begins: Arc<AtomicU64>,
    commits: Arc<AtomicU64>,
    rollbacks: Arc<AtomicU64>,
}
impl DbMetrics {
    #[must_use]
    pub fn round_trips(&self) -> u64 {
        self.round_trips.load(Ordering::Relaxed)
    }
    #[must_use]
    pub fn conflict_retries(&self) -> u64 {
        self.conflict_retries.load(Ordering::Relaxed)
    }
    pub fn query(&self) {
        self.round_trips.fetch_add(1, Ordering::Relaxed);
    }
    fn begin(&self) {
        self.begins.fetch_add(1, Ordering::Relaxed);
        self.query();
    }
    fn commit(&self) {
        self.commits.fetch_add(1, Ordering::Relaxed);
        self.query();
    }
    fn rollback(&self) {
        self.rollbacks.fetch_add(1, Ordering::Relaxed);
        self.query();
    }
    pub fn begins(&self) -> u64 {
        self.begins.load(Ordering::Relaxed)
    }
    pub fn commits(&self) -> u64 {
        self.commits.load(Ordering::Relaxed)
    }
    pub fn rollbacks(&self) -> u64 {
        self.rollbacks.load(Ordering::Relaxed)
    }
    pub fn committed(&self, retries: u32) {
        self.conflict_retries
            .fetch_add(u64::from(retries), Ordering::Relaxed);
    }
}

pub(crate) fn bounded_json(
    value: &impl Serialize,
    limit: usize,
    kind: &str,
) -> Result<String, AdmissionError> {
    struct Bound {
        data: Vec<u8>,
        limit: usize,
    }
    impl std::io::Write for Bound {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > self.limit.saturating_sub(self.data.len()) {
                return Err(std::io::Error::other("Atlas admission JSON limit"));
            }
            self.data.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut writer = Bound {
        data: Vec::new(),
        limit,
    };
    serde_json::to_writer(&mut writer, value).map_err(|_| AdmissionError::PayloadTooLarge {
        kind: kind.into(),
        limit,
    })?;
    String::from_utf8(writer.data).map_err(|_| AdmissionError::InvalidReceipt {
        code: "invalid_json_encoding".into(),
    })
}
fn checked_i64(value: u64) -> Result<i64, AdmissionError> {
    i64::try_from(value).map_err(|_| AdmissionError::InvalidRunState {
        code: "integer_overflow".into(),
    })
}

#[derive(SurrealValue)]
struct ProjectionRow {
    run_id: String,
    article_id: String,
    receipt_key: String,
    taxonomy_version: String,
    admission_state: String,
    publication_version: String,
    input_revision: i64,
    source_revision: String,
}
#[derive(SurrealValue)]
struct ReceiptRow {
    receipt_json: String,
    receipt_sha256: String,
}

/// Reads the authoritative admission alongside an unchanged historical article
/// projection. Legacy membership strings never authorize a current taxonomy.
pub async fn classification_view(
    repo: &Repository,
    key: &OccurrenceKey,
) -> Result<ClassificationView, AdmissionError> {
    classification_views_read(
        ClassificationReader::Db(repo.db()),
        std::slice::from_ref(key),
    )
    .await?
    .remove(key)
    .ok_or_else(|| AdmissionError::InvalidReceipt {
        code: "classification_view_missing".into(),
    })
}
fn finish_classification_view(
    key: &OccurrenceKey,
    projection: Option<ProjectionRow>,
    row: Option<ReceiptRow>,
) -> Result<ClassificationView, AdmissionError> {
    let Some(projection) = projection else {
        return Ok(ClassificationView {
            schema_version: "atlas-classification-view-v1".into(),
            occurrence: key.clone(),
            taxonomy_version: crate::atlas_taxonomy::LEGACY_TAXONOMY_VERSION.into(),
            admission_state: "legacy".into(),
            publication_version: String::new(),
            receipt: None,
            legacy: true,
        });
    };
    if projection.run_id != key.run_id
        || projection.article_id != key.article_id
        || projection.input_revision < 1
    {
        return Err(AdmissionError::InvalidReceipt {
            code: "projection_identity_mismatch".into(),
        });
    }
    if matches!(
        projection.publication_version.as_str(),
        "atlas-admission-publication-v1"
            | "atlas-admission-publication-v2"
            | "atlas-admission-publication-v3"
    ) {
        return Ok(ClassificationView {
            schema_version: "atlas-classification-view-v1".into(),
            occurrence: key.clone(),
            taxonomy_version: projection.taxonomy_version,
            admission_state: "legacy".into(),
            publication_version: projection.publication_version,
            receipt: None,
            legacy: true,
        });
    }
    if projection.taxonomy_version != crate::atlas_taxonomy::TAXONOMY_VERSION
        || projection.publication_version != PUBLICATION_VERSION
    {
        return Err(AdmissionError::UnsupportedVersion {
            kind: "classification_projection".into(),
            found: projection.publication_version,
        });
    }
    if projection.admission_state == "pending" {
        return Ok(ClassificationView {
            schema_version: "atlas-classification-view-v1".into(),
            occurrence: key.clone(),
            taxonomy_version: projection.taxonomy_version,
            admission_state: projection.admission_state,
            publication_version: projection.publication_version,
            receipt: None,
            legacy: false,
        });
    }
    if !matches!(projection.admission_state.as_str(), "admitted" | "rejected") {
        return Err(AdmissionError::InvalidReceipt {
            code: "invalid_projection_state".into(),
        });
    }
    let row = row.ok_or_else(|| AdmissionError::InvalidReceipt {
        code: "receipt_missing".into(),
    })?;
    if row.receipt_json.is_empty() || row.receipt_json.len() > MAX_RECEIPT_BYTES {
        return Err(AdmissionError::PayloadTooLarge {
            kind: "receipt".into(),
            limit: MAX_RECEIPT_BYTES,
        });
    }
    if crate::classifier::artifact::sha256_hex(row.receipt_json.as_bytes()) != row.receipt_sha256 {
        return Err(AdmissionError::InvalidReceipt {
            code: "receipt_checksum_mismatch".into(),
        });
    }
    let receipt: StoredReceipt =
        serde_json::from_str(&row.receipt_json).map_err(|_| AdmissionError::InvalidReceipt {
            code: "receipt_json_invalid".into(),
        })?;
    receipt.validate()?;
    if receipt.key.record_id() != projection.receipt_key
        || receipt.key.occurrence != *key
        || receipt.key.input_revision != projection.input_revision as u64
        || receipt.key.source_revision != projection.source_revision
        || (projection.admission_state == "admitted") != receipt.receipt.is_admitted()
    {
        return Err(AdmissionError::InvalidReceipt {
            code: "receipt_projection_mismatch".into(),
        });
    }
    Ok(ClassificationView {
        schema_version: "atlas-classification-view-v1".into(),
        occurrence: key.clone(),
        taxonomy_version: projection.taxonomy_version,
        admission_state: projection.admission_state,
        publication_version: projection.publication_version,
        receipt: Some(receipt),
        legacy: false,
    })
}

/// Canonical cycle states; only terminal outcomes may create calibration work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RunStop {
    Paused,
    Waiting,
    Blocked,
    Completed,
    Partial,
    Failed,
    Cancelled,
}
impl RunStop {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Paused => "paused",
            Self::Waiting => "waiting",
            Self::Blocked => "blocked",
            Self::Completed => "completed",
            Self::Partial => "partial",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
    #[must_use]
    pub fn terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Partial | Self::Failed | Self::Cancelled
        )
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct TerminalReceipt {
    pub run_id: String,
    pub terminal_revision: u64,
    pub observation_watermark: u64,
    pub calibration_job_id: Option<String>,
    pub replayed: bool,
}

#[derive(SurrealValue)]
struct RunHead {
    revision: i64,
    work_generation: i64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct RunRevision {
    pub revision: u64,
    pub work_generation: u64,
}
/// Reads the generations later compared by start_run's guard transaction.
pub async fn run_revision(repo: &Repository, run_id: &str) -> Result<RunRevision, AdmissionError> {
    let mut response = db_query(repo.db(),"SELECT (revision ?? 0) AS revision, (work_generation ?? 0) AS work_generation FROM ONLY type::record('atlas_classifier_run', $key)")
        .bind(("key", run_id.to_string())).await.map_err(TxError::from)?.check().map_err(TxError::from)?;
    let row: Option<RunHead> = response.take(0).map_err(TxError::from)?;
    let row = row.unwrap_or(RunHead {
        revision: 0,
        work_generation: 0,
    });
    Ok(RunRevision {
        revision: u64::try_from(row.revision).map_err(|_| AdmissionError::InvalidRunState {
            code: "negative_run_revision".into(),
        })?,
        work_generation: u64::try_from(row.work_generation).map_err(|_| {
            AdmissionError::InvalidRunState {
                code: "negative_work_generation".into(),
            }
        })?,
    })
}

#[derive(SurrealValue)]
struct StartReply {
    status: String,
    epoch: i64,
    revision: i64,
}
/// Atomically publishes the preflighted pin and running state under a new run
/// lease. Stale preflight must be repeated, never reused across a generation.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct StartRunRequest<'a> {
    pub run_id: &'a str,
    pub resumed: bool,
    pub expected_run_revision: u64,
    pub expected_work_generation: u64,
    pub cursor_json: &'a str,
    pub stats_json: &'a str,
    pub owner: &'a str,
    pub now: DateTime<Utc>,
}
impl<'a> StartRunRequest<'a> {
    /// Constructs a fresh-run request without inventing a persisted revision.
    #[must_use]
    pub fn fresh(
        run_id: &'a str,
        owner: &'a str,
        cursor_json: &'a str,
        stats_json: &'a str,
        now: DateTime<Utc>,
    ) -> Self {
        Self {
            run_id,
            resumed: false,
            expected_run_revision: 0,
            expected_work_generation: 0,
            cursor_json,
            stats_json,
            owner,
            now,
        }
    }
    /// Binds the saved generation selected before the classifier preflight.
    #[must_use]
    pub fn resume(mut self, expected_run_revision: u64, expected_work_generation: u64) -> Self {
        self.resumed = true;
        self.expected_run_revision = expected_run_revision;
        self.expected_work_generation = expected_work_generation;
        self
    }
}

pub async fn start_run(
    db_owner: &AdmissionOwner,
    permit: &CyclePermit,
    request: &StartRunRequest<'_>,
) -> Result<RunClaim, AdmissionError> {
    let started = std::time::Instant::now();
    db_owner.ensure_open()?;
    let StartRunRequest {
        run_id,
        resumed,
        expected_run_revision,
        expected_work_generation,
        cursor_json,
        stats_json,
        owner,
        now,
    } = *request;
    if run_id.is_empty()
        || owner.is_empty()
        || owner.len() > 256
        || cursor_json.len() > 64 * 1024
        || stats_json.len() > 1024 * 1024
    {
        return Err(AdmissionError::InvalidRunState {
            code: "invalid_start_request".into(),
        });
    }
    let cursor: crate::atlas::Cursor =
        serde_json::from_str(cursor_json).map_err(|_| AdmissionError::InvalidRunState {
            code: "invalid_cursor".into(),
        })?;
    let stats: crate::atlas::RunStats =
        serde_json::from_str(stats_json).map_err(|_| AdmissionError::InvalidRunState {
            code: "invalid_stats".into(),
        })?;
    if !(1..=6).contains(&cursor.phase)
        || (!resumed && permit.requirement() != CycleRequirement::FreshAcquisition)
        || (resumed && permit.requirement() == CycleRequirement::FreshAcquisition)
        || (permit.requirement() == CycleRequirement::PostAdmissionOnly
            && (cursor.phase <= 3 || stats.acquisition_retry.is_some()))
        || permit
            .pinned()
            .is_some_and(|pin| stats.classifier_snapshot.as_ref() != Some(&pin.snapshot))
    {
        return Err(AdmissionError::InvalidRunState {
            code: "permit_snapshot_mismatch".into(),
        });
    }
    let pin = bounded_json(&stats.classifier_snapshot, 16 * 1024, "run_pin")?;
    let expected_revision = checked_i64(expected_run_revision)?;
    let expected_work = checked_i64(expected_work_generation)?;
    let captured = (
        run_id.to_string(),
        owner.to_string(),
        cursor_json.to_string(),
        stats_json.to_string(),
        pin.clone(),
        permit
            .pinned()
            .map(|pin| pin.manifest_sha256.clone())
            .unwrap_or_default(),
    );
    let post = permit.requirement() == CycleRequirement::PostAdmissionOnly;
    let committed=foreground_tx(db_owner,RetryPolicy::default(),move |tx| {
        let (run_id,owner,cursor_json,stats_json,pin,manifest)=captured.clone();
        async move {
            let now = lease_time(now, started)?;
            let mut result=tx_query(&tx,"{
                UPSERT type::record('atlas_classifier_guard',$guard) SET updated_at=<datetime>$now RETURN NONE;
                LET $r=(SELECT * FROM ONLY type::record('atlas_classifier_run',$run));
                LET $cycle=(SELECT state FROM ONLY type::record('atlas_run',$run));
                IF ($r.revision ?? 0)!=$revision OR ($r.work_generation ?? 0)!=$work { RETURN {status:'preflight_stale',epoch:0,revision:0}; };
                IF ($r.owner ?? '')!='' AND $r.lease_until> <datetime>$now { RETURN {status:'lease_lost',epoch:0,revision:0}; };
                IF $resumed AND $cycle=NONE { RETURN {status:'missing_run',epoch:0,revision:0}; };
                IF !$resumed AND $cycle!=NONE { RETURN {status:'already_exists',epoch:0,revision:0}; };
                IF $post AND array::len(SELECT VALUE article_id FROM atlas_admission WHERE run_id=$run LIMIT 1)>0 { RETURN {status:'preflight_stale',epoch:0,revision:0}; };
                IF ($r.pin_json ?? '')!='' AND $r.pin_json!=$pin { RETURN {status:'pinned_revision_changed',epoch:0,revision:0}; };
                IF !$post AND ($r.manifest_sha256 ?? '')!='' AND $r.manifest_sha256!=$manifest { RETURN {status:'pinned_revision_changed',epoch:0,revision:0}; };
                LET $epoch=($r.epoch ?? 0)+1; LET $next=($r.revision ?? 0)+1;
                UPSERT type::record('atlas_classifier_run',$run) SET owner=$owner,epoch=$epoch,lease_until=<datetime>$lease,revision=$next,
                    work_generation=($r.work_generation ?? 0),admission_generation=IF ($r.admission_generation ?? 0)=0 AND !$post {1} ELSE {$r.admission_generation ?? 0},
                    closed_generation=($r.closed_generation ?? 0),page_json=($r.page_json ?? ''),pin_json=$pin,manifest_sha256=IF $manifest!='' {$manifest} ELSE {$r.manifest_sha256 ?? ''},state='running',updated_at=<datetime>$now RETURN NONE;
                IF $resumed {
                    UPDATE type::record('atlas_run',$run) SET state='running',phase=$phase,cursor_json=$cursor,stats_json=$stats RETURN NONE;
                } ELSE {
                    CREATE type::record('atlas_run',$run) CONTENT {state:'running',phase:$phase,cursor_json:$cursor,stats_json:$stats,note:'',started_at:$now,finished_at:''} RETURN NONE;
                };
                RETURN {status:'ok',epoch:$epoch,revision:$next};
            }")
                .bind(("guard",codec::encode(&["run",&run_id]))).bind(("run",run_id)).bind(("owner",owner))
                .bind(("revision",expected_revision)).bind(("work",expected_work)).bind(("resumed",resumed))
                .bind(("post",post)).bind(("pin",pin)).bind(("manifest",manifest))
                .bind(("phase",i64::from(cursor.phase))).bind(("cursor",cursor_json)).bind(("stats",stats_json))
                .bind(("now",now.to_rfc3339())).bind(("lease",(now+Duration::seconds(CLAIM_LEASE_SECS)).to_rfc3339()))
                .await?.check()?;
            let reply:StartReply=result.take::<Option<StartReply>>(0)?.ok_or_else(||TxError::Abort("missing_start_run_reply".into()))?; Ok(reply)
        }
    }).await?;
    let reply = committed.value;
    if reply.status == "lease_lost" {
        return Err(AdmissionError::LeaseLost);
    }
    if reply.status != "ok" {
        return Err(AdmissionError::InvalidRunState { code: reply.status });
    }
    Ok(RunClaim {
        run_id: run_id.into(),
        owner: owner.into(),
        epoch: u64::try_from(reply.epoch).map_err(|_| AdmissionError::InvalidRunState {
            code: "negative_run_epoch".into(),
        })?,
        revision: u64::try_from(reply.revision).map_err(|_| AdmissionError::InvalidRunState {
            code: "negative_run_revision".into(),
        })?,
    })
}

/// Saves only under the current unexpired owner/epoch. Revision advances from
/// the latest owned row; it is not incorrectly compared to the initial claim.
pub async fn save_run(
    db_owner: &AdmissionOwner,
    claim: &RunClaim,
    cursor_json: &str,
    stats_json: &str,
    now: DateTime<Utc>,
) -> Result<(), AdmissionError> {
    save_checkpoint(db_owner, claim, cursor_json, stats_json, None, now).await
}
/// Saves a native acquisition receipt and its exact checkpoint atomically.
pub(crate) async fn save_run_with_receipt(
    db_owner: &AdmissionOwner,
    claim: &RunClaim,
    cursor_json: &str,
    stats_json: &str,
    receipt_key: &str,
    receipt_json: &str,
    now: DateTime<Utc>,
) -> Result<(), AdmissionError> {
    save_checkpoint(
        db_owner,
        claim,
        cursor_json,
        stats_json,
        Some((receipt_key, receipt_json)),
        now,
    )
    .await
}
async fn save_checkpoint(
    db_owner: &AdmissionOwner,
    claim: &RunClaim,
    cursor_json: &str,
    stats_json: &str,
    receipt: Option<(&str, &str)>,
    now: DateTime<Utc>,
) -> Result<(), AdmissionError> {
    let started = std::time::Instant::now();
    db_owner.ensure_open()?;
    if cursor_json.len() > 64 * 1024 || stats_json.len() > 1024 * 1024 {
        return Err(AdmissionError::PayloadTooLarge {
            kind: "run_checkpoint".into(),
            limit: 1024 * 1024,
        });
    }
    let cursor: crate::atlas::Cursor =
        serde_json::from_str(cursor_json).map_err(|_| AdmissionError::InvalidRunState {
            code: "invalid_cursor".into(),
        })?;
    let stats: crate::atlas::RunStats =
        serde_json::from_str(stats_json).map_err(|_| AdmissionError::InvalidRunState {
            code: "invalid_checkpoint_stats".into(),
        })?;
    crate::atlas::validate_resume_state(&cursor, &stats).map_err(|_| {
        AdmissionError::InvalidRunState {
            code: "invalid_checkpoint".into(),
        }
    })?;
    let pin = bounded_json(&stats.classifier_snapshot, 16 * 1024, "run_pin")?;
    if receipt
        .is_some_and(|(key, json)| key.is_empty() || key.len() > 4096 || json.len() > 64 * 1024)
    {
        return Err(AdmissionError::PayloadTooLarge {
            kind: "native_receipt".into(),
            limit: 64 * 1024,
        });
    }
    let receipt = receipt.map(|(key, json)| (key.to_owned(), json.to_owned()));
    let epoch = checked_i64(claim.epoch)?;
    let captured = (
        claim.clone(),
        cursor_json.to_string(),
        stats_json.to_string(),
        receipt,
        pin,
    );
    let result=foreground_tx(db_owner,RetryPolicy::default(),move |tx| {
        let (claim,cursor_json,stats_json,receipt,pin)=captured.clone();
        async move {
            let now = lease_time(now, started)?;
            let mut response=tx_query(&tx,"{
                UPSERT type::record('atlas_classifier_guard',$guard) SET updated_at=<datetime>$now RETURN NONE;
                LET $r=(SELECT * FROM ONLY type::record('atlas_classifier_run',$run));
                IF $r.owner!=$owner OR $r.epoch!=$epoch OR $r.lease_until<= <datetime>$now OR $r.state!='running' OR $r.pin_json!=$pin {RETURN false;};
                IF $has_receipt {UPSERT type::record('app_state',$receipt_key) SET value=$receipt_json RETURN NONE;};
                UPDATE type::record('atlas_run',$run) SET cursor_json=$cursor,stats_json=$stats,phase=$phase RETURN NONE;
                UPDATE type::record('atlas_classifier_run',$run) SET revision=$r.revision+1,updated_at=<datetime>$now RETURN NONE;
                RETURN true;
            }").bind(("guard",codec::encode(&["run",&claim.run_id]))).bind(("run",claim.run_id)).bind(("owner",claim.owner)).bind(("epoch",epoch))
                .bind(("pin",pin)).bind(("has_receipt",receipt.is_some())).bind(("receipt_key",receipt.as_ref().map(|(key,_)|key.clone()).unwrap_or_default())).bind(("receipt_json",receipt.map(|(_,json)|json).unwrap_or_default())).bind(("cursor",cursor_json)).bind(("stats",stats_json)).bind(("phase",i64::from(cursor.phase))).bind(("now",now.to_rfc3339()))
                .await?.check()?;
            response.take::<Option<bool>>(0).map_err(TxError::from)?.ok_or_else(|| TxError::Abort("missing_run_fence_reply".into()))
        }
    }).await?;
    if result.value {
        Ok(())
    } else {
        Err(AdmissionError::LeaseLost)
    }
}

/// Expired owners cannot resurrect their epoch by heartbeating late.
pub async fn heartbeat_run(
    db_owner: &AdmissionOwner,
    claim: &RunClaim,
    now: DateTime<Utc>,
) -> Result<bool, AdmissionError> {
    let started = std::time::Instant::now();
    db_owner.ensure_open()?;
    let epoch = checked_i64(claim.epoch)?;
    let captured = claim.clone();
    let committed=foreground_tx(db_owner,RetryPolicy::default(),move |tx| {
        let claim=captured.clone();
        async move {
            let now = lease_time(now, started)?;
            let mut response=tx_query(&tx,"{
                UPSERT type::record('atlas_classifier_guard',$guard) SET updated_at=<datetime>$now RETURN NONE;
                LET $r=(SELECT * FROM ONLY type::record('atlas_classifier_run',$run));
                IF $r.owner!=$owner OR $r.epoch!=$epoch OR $r.state!='running' OR $r.lease_until<= <datetime>$now {RETURN false;};
                UPDATE type::record('atlas_classifier_run',$run) SET lease_until=<datetime>$lease,updated_at=<datetime>$now RETURN NONE;
                UPDATE atlas_admission SET lease_until=<datetime>$lease WHERE run_id=$run AND owner=$owner AND lease_until> <datetime>$now RETURN NONE;
                UPDATE atlas_work_unit SET lease_until=<datetime>$lease WHERE run_id=$run AND owner=$owner AND stage=8 AND lease_until> <datetime>$now RETURN NONE;
                RETURN true;
            }").bind(("guard",codec::encode(&["run",&claim.run_id]))).bind(("run",claim.run_id)).bind(("owner",claim.owner)).bind(("epoch",epoch))
                .bind(("now",now.to_rfc3339())).bind(("lease",(now+Duration::seconds(CLAIM_LEASE_SECS)).to_rfc3339()))
                .await?.check()?;
            response.take::<Option<bool>>(0).map_err(TxError::from)?.ok_or_else(|| TxError::Abort("missing_run_fence_reply".into()))
        }
    }).await?;
    Ok(committed.value)
}

#[derive(SurrealValue)]
struct StopReply {
    revision: i64,
    replayed: bool,
    receipt_json: Option<String>,
}
/// Ends only the current owned attempt. A replay from that same epoch returns
/// its receipt; an old epoch cannot overwrite a later owner or terminal state.
pub async fn stop_run(
    db_owner: &AdmissionOwner,
    claim: &RunClaim,
    state: RunStop,
    cursor_json: &str,
    stats_json: &str,
    note: &str,
    now: DateTime<Utc>,
) -> Result<TerminalReceipt, AdmissionError> {
    stop_run_with_hook(
        db_owner,
        claim,
        state,
        cursor_json,
        stats_json,
        note,
        now,
        None,
    )
    .await
}

/// The hook joins the existing stop transaction; no terminal state is visible
/// before its durable enqueue and persisted replay receipt commit together.
pub async fn stop_run_with_hook(
    db_owner: &AdmissionOwner,
    claim: &RunClaim,
    state: RunStop,
    cursor_json: &str,
    stats_json: &str,
    note: &str,
    now: DateTime<Utc>,
    hook: Option<TerminalHook>,
) -> Result<TerminalReceipt, AdmissionError> {
    let started = std::time::Instant::now();
    db_owner.ensure_open()?;
    if cursor_json.len() > 64 * 1024 || stats_json.len() > 1024 * 1024 || note.len() > 16 * 1024 {
        return Err(AdmissionError::PayloadTooLarge {
            kind: "terminal_checkpoint".into(),
            limit: 1024 * 1024,
        });
    }
    let cursor: crate::atlas::Cursor =
        serde_json::from_str(cursor_json).map_err(|_| AdmissionError::InvalidRunState {
            code: "invalid_cursor".into(),
        })?;
    let stats: crate::atlas::RunStats =
        serde_json::from_str(stats_json).map_err(|_| AdmissionError::InvalidRunState {
            code: "invalid_terminal_stats".into(),
        })?;
    crate::atlas::validate_resume_state(&cursor, &stats).map_err(|_| {
        AdmissionError::InvalidRunState {
            code: "invalid_terminal_checkpoint".into(),
        }
    })?;
    let terminal_identity = crate::classifier::artifact::sha256_hex(
        codec::encode(&[
            "atlas-terminal-v1",
            &claim.run_id,
            &claim.owner,
            &claim.epoch.to_string(),
            state.as_str(),
        ])
        .as_bytes(),
    );
    let pin_json = serde_json::to_string(&stats.classifier_snapshot).map_err(|_| {
        AdmissionError::InvalidRunState {
            code: "invalid_terminal_pin".into(),
        }
    })?;
    let snapshot = stats.classifier_snapshot;
    let epoch = checked_i64(claim.epoch)?;
    let captured = (
        claim.clone(),
        cursor_json.to_string(),
        stats_json.to_string(),
        note.to_string(),
    );
    let committed=foreground_tx(db_owner,RetryPolicy::default(),move |tx| {
        let (claim,cursor_json,stats_json,note)=captured.clone();
        let hook=hook.clone();let snapshot=snapshot.clone();let terminal_identity=terminal_identity.clone();let pin_json=pin_json.clone();
        async move {
            let now = lease_time(now, started)?;
            let mut response=tx_query(&tx,"{
                UPSERT type::record('atlas_classifier_guard',$guard) SET updated_at=<datetime>$now RETURN NONE;
                LET $r=(SELECT * FROM ONLY type::record('atlas_classifier_run',$run));
                IF $r=NONE OR $r.owner!=$owner OR $r.epoch!=$epoch OR $r.pin_json!=$pin {RETURN NONE;};
                IF $r.state=$state {RETURN {revision:$r.revision,replayed:true,receipt_json:IF bytes::len(type::bytes($r.terminal_receipt_json ?? ''))<=16384 {$r.terminal_receipt_json} ELSE {NONE}};};
                IF $r.state!='running' OR $r.lease_until=NONE OR $r.lease_until<= <datetime>$now {RETURN NONE;};
                IF $state='completed' AND array::len(SELECT VALUE article_id FROM atlas_admission WHERE run_id=$run LIMIT 1)>0 { THROW 'pending_admission_cannot_complete'; };
                LET $next=$r.revision+1;
                UPDATE type::record('atlas_run',$run) SET state=$state,phase=$phase,cursor_json=$cursor,stats_json=$stats,note=$note,finished_at=IF $terminal {$now} ELSE {finished_at} RETURN NONE;
                UPDATE type::record('atlas_classifier_run',$run) SET state=$state,revision=$next,lease_until=NONE,
                    closed_generation=IF $state='completed' {$r.admission_generation} ELSE {$r.closed_generation},updated_at=<datetime>$now RETURN NONE;
                UPDATE atlas_admission SET state='pending',owner='',epoch=(epoch ?? 0)+1,lease_until=NONE,last_activation='',updated_at=$now WHERE run_id=$run AND owner=$owner AND state='evaluating' RETURN NONE;
                RETURN {revision:$next,replayed:false,receipt_json:NONE};
            }").bind(("guard",codec::encode(&["run",&claim.run_id]))).bind(("run",claim.run_id.clone())).bind(("owner",claim.owner.clone())).bind(("epoch",epoch))
                .bind(("pin",pin_json)).bind(("state",state.as_str())).bind(("terminal",state.terminal())).bind(("cursor",cursor_json)).bind(("stats",stats_json)).bind(("note",note))
                .bind(("phase",i64::from(cursor.phase))).bind(("now",now.to_rfc3339())).await?.check()?;
            let Some(reply)=response.take::<Option<StopReply>>(0)? else {return Ok(None);};
            if reply.replayed {
                if let Some(json)=reply.receipt_json.as_deref().filter(|json|!json.is_empty()) {
                    let mut receipt:TerminalReceipt=serde_json::from_str(json).map_err(|_|TxError::Abort("terminal_receipt_corrupt".into()))?;
                    if receipt.run_id!=claim.run_id || receipt.terminal_revision!=u64::try_from(reply.revision).map_err(|_|TxError::Abort("negative_terminal_revision".into()))? {return Err(TxError::Abort("terminal_receipt_identity".into()));}
                    receipt.replayed=true;
                    return Ok(Some(receipt));
                }
            }
            let revision=u64::try_from(reply.revision).map_err(|_|TxError::Abort("negative_terminal_revision".into()))?;
            let effect=if state.terminal() {
                if let Some(hook)=hook {
                    let mut head=tx_query(&tx,"SELECT VALUE page_json FROM ONLY type::record('atlas_classifier_run',$run)").bind(("run",claim.run_id.clone())).await?.check()?;
                    let page=head.take::<Option<String>>(0)?.and_then(|json|serde_json::from_str::<AdmissionPageToken>(&json).ok());
                    let mut saved=tx_query(&tx,"SELECT VALUE manifest_sha256 FROM ONLY type::record('atlas_classifier_run',$run)").bind(("run",claim.run_id.clone())).await?.check()?;
                    let manifest_sha256=saved.take::<Option<String>>(0)?.filter(|hash|!hash.is_empty()).or_else(||page.filter(|page|snapshot.as_ref().is_some_and(|pin|pin.model_id==page.model_id)).map(|page|page.manifest_sha256));
                    hook(tx.clone(),TerminalCycle {run_id:claim.run_id.clone(),state:state.as_str().into(),revision,identity:terminal_identity,snapshot,manifest_sha256},now).await?
                } else {TerminalEffect::default()}
            } else {TerminalEffect::default()};
            if effect.observation_watermark>i64::MAX as u64 || effect.calibration_job_id.as_ref().is_some_and(|id|!crate::intel_service::valid_id(id)) {return Err(TxError::Abort("terminal_effect_identity".into()));}
            let receipt=TerminalReceipt {run_id:claim.run_id.clone(),terminal_revision:revision,observation_watermark:effect.observation_watermark,calibration_job_id:effect.calibration_job_id,replayed:reply.replayed};
            let json=bounded_json(&receipt,16384,"terminal_receipt").map_err(|_|TxError::Abort("terminal_receipt_too_large".into()))?;
            tx_query(&tx,"UPDATE type::record('atlas_classifier_run',$run) SET terminal_receipt_json=$receipt RETURN NONE").bind(("run",claim.run_id)).bind(("receipt",json)).await?.check()?;
            Ok(Some(receipt))
        }
    }).await?;
    committed.value.ok_or(AdmissionError::LeaseLost)
}

#[derive(SurrealValue)]
struct StageReply {
    changed: bool,
}
/// Stages only under the current run owner; source changes revoke older item
/// claims and obtain a new run-local work sequence atomically.
pub async fn stage_article(
    db_owner: &AdmissionOwner,
    run: &RunClaim,
    row: &AtlasArticleRow,
    source_revision: &str,
    now: DateTime<Utc>,
) -> Result<bool, AdmissionError> {
    db_owner.ensure_open()?;
    let repo = db_owner.repository();
    if row.run_id != run.run_id {
        return Err(AdmissionError::InvalidRunState {
            code: "stage_run_mismatch".into(),
        });
    }
    stage_on_db(
        repo.db(),
        Some(db_owner),
        Some(run),
        row,
        source_revision,
        crate::atlas_taxonomy::TAXONOMY_VERSION,
        now,
    )
    .await
}
/// Legacy callers may stage only outside a live classifier-owned run. They
/// cannot reset another worker's claims or manufacture a current admission.
pub(crate) async fn stage_legacy(
    db: &surrealdb::Surreal<surrealdb::engine::any::Any>,
    row: &AtlasArticleRow,
    source_revision: &str,
    taxonomy: &str,
    now: DateTime<Utc>,
) -> Result<bool, AdmissionError> {
    stage_on_db(db, None, None, row, source_revision, taxonomy, now).await
}
#[derive(Clone)]
struct StageWrite {
    row: AtlasArticleRow,
    payload: String,
    provenance_json: String,
    source_revision: String,
    source_hash: String,
    classifier_hash: String,
    taxonomy: String,
    owner: String,
    epoch: i64,
    owned: bool,
    now: DateTime<Utc>,
}
fn prepare_stage(
    run: Option<&RunClaim>,
    row: &AtlasArticleRow,
    source_revision: &str,
    taxonomy: &str,
    provenance: &StageProvenance,
    now: DateTime<Utc>,
) -> Result<StageWrite, AdmissionError> {
    if row
        .title
        .len()
        .checked_add(row.description.len())
        .is_none_or(|length| length > crate::classifier::training::MAX_ROW_TEXT_BYTES)
    {
        return Err(AdmissionError::PayloadTooLarge {
            kind: "classifier_text".into(),
            limit: crate::classifier::training::MAX_ROW_TEXT_BYTES,
        });
    }
    if !crate::intel_service::valid_id(&row.run_id)
        || !crate::intel_service::valid_id(&row.id)
        || source_revision.is_empty()
        || source_revision.len() > 512
    {
        return Err(AdmissionError::InvalidRunState {
            code: "invalid_staging_identity".into(),
        });
    }
    provenance.validate()?;
    let provenance_json = bounded_json(provenance, MAX_PROVENANCE_BYTES, "stage_provenance")?;
    let payload = bounded_json(row, MAX_PAYLOAD_BYTES, "staged_payload")?;
    let source_hash = crate::atlas_metadata::input_hash(row);
    let classifier_hash =
        crate::classifier::ClassifyInput::new("stage", &row.title, &row.description, None)
            .input_hash;
    let owner = run.map(|claim| claim.owner.as_str()).unwrap_or("");
    let epoch = checked_i64(run.map(|claim| claim.epoch).unwrap_or(0))?;
    let owned = run.is_some();
    Ok(StageWrite {
        row: row.clone(),
        payload,
        provenance_json,
        source_revision: source_revision.to_owned(),
        source_hash,
        classifier_hash,
        taxonomy: taxonomy.to_owned(),
        owner: owner.to_owned(),
        epoch,
        owned,
        now,
    })
}
async fn stage_in(tx: &super::tx::Tx, input: StageWrite) -> Result<Option<StageReply>, TxError> {
    let StageWrite {
        row,
        payload,
        provenance_json,
        source_revision,
        source_hash,
        classifier_hash,
        taxonomy,
        owner,
        epoch,
        owned,
        now,
    } = input;
    let key = codec::encode(&[&row.run_id, &row.id]);
    let occurrence = OccurrenceKey::new(&row.run_id, &row.id);
    let mut response=tx_query(tx,"{
                UPSERT type::record('atlas_classifier_guard',$run_guard) SET updated_at=<datetime>$now RETURN NONE;
                UPSERT type::record('atlas_classifier_guard',$source_guard) SET updated_at=<datetime>$now RETURN NONE;
                LET $r=(SELECT * FROM ONLY type::record('atlas_classifier_run',$run));
                IF $owned AND ($r=NONE OR $r.owner!=$owner OR $r.epoch!=$epoch OR $r.state!='running' OR $r.lease_until=NONE OR $r.lease_until<= <datetime>$now) {RETURN NONE;};
                IF !$owned AND ($r.owner ?? '')!='' AND $r.lease_until> <datetime>$now {RETURN NONE;};
                LET $s=(SELECT * FROM ONLY type::record('atlas_admission',$key));
                IF $s!=NONE AND $s.input_hash=$hash AND $s.source_revision=$source_revision AND $s.taxonomy_version=$taxonomy AND $s.payload_json=$payload AND $s.classifier_input_hash=$classifier_hash AND ($s.provenance_json ?? '')=$provenance {RETURN {changed:false};};
                LET $prior=(SELECT input_revision FROM ONLY type::record('atlas_classification_projection',$key));
                LET $guard=(SELECT input_revision FROM ONLY type::record('atlas_classifier_guard',$source_guard));
                LET $sequence=($r.work_generation ?? 0)+1; LET $revision=math::max([$s.input_revision ?? 0,$prior.input_revision ?? 0,$guard.input_revision ?? 0])+1;
                UPDATE type::record('atlas_classifier_guard',$source_guard) SET input_revision=$revision RETURN NONE;
                UPSERT type::record('atlas_classifier_run',$run) SET work_generation=$sequence,updated_at=<datetime>$now RETURN NONE;
                UPSERT type::record('atlas_admission',$key) SET run_id=$run,article_id=$article,payload_json=$payload,provenance_json=$provenance,input_hash=$hash,classifier_input_hash=$classifier_hash,
                    source_revision=$source_revision,input_revision=$revision,work_seq=$sequence,taxonomy_version=$taxonomy,state='pending',attempts=0,last_error='',seen_at=$seen_at,
                    owner='',epoch=($s.epoch ?? 0)+1,lease_until=NONE,next_eligible_at=NONE,last_activation='',created_at=($s.created_at ?? $now),updated_at=$now RETURN NONE;
                UPDATE type::record('atlas_article',$key) SET admission_state='pending' RETURN NONE;
                UPSERT type::record('atlas_classification_projection',$key) SET run_id=$run,article_id=$article,receipt_key='',taxonomy_version=$taxonomy,
                    admission_state='pending',publication_version=$publication,input_revision=$revision,source_revision=$source_revision,updated_at=<datetime>$now RETURN NONE;
                RETURN {changed:true};
            }").bind(("run_guard",codec::encode(&["run",&row.run_id]))).bind(("source_guard",codec::encode(&["source",&row.run_id,&row.id])))
                .bind(("seen_at",row.seen_at.clone())).bind(("run",row.run_id)).bind(("article",row.id)).bind(("key",key)).bind(("payload",payload))
                .bind(("provenance",provenance_json)).bind(("hash",source_hash)).bind(("classifier_hash",classifier_hash)).bind(("source_revision",source_revision)).bind(("taxonomy",taxonomy)).bind(("publication",PUBLICATION_VERSION))
                .bind(("owned",owned)).bind(("owner",owner)).bind(("epoch",epoch)).bind(("now",now.to_rfc3339()))
                .await?.check()?;
    let reply = response.take::<Option<StageReply>>(0)?;
    if reply.as_ref().is_some_and(|reply| reply.changed) {
        crate::atlas_derivative_worker::enqueue_cleanup_in(tx, &occurrence, now).await?;
    }
    Ok(reply)
}
async fn stage_on_db(
    db: &surrealdb::Surreal<surrealdb::engine::any::Any>,
    db_owner: Option<&AdmissionOwner>,
    run: Option<&RunClaim>,
    row: &AtlasArticleRow,
    source_revision: &str,
    taxonomy: &str,
    now: DateTime<Utc>,
) -> Result<bool, AdmissionError> {
    let started = std::time::Instant::now();
    let input = prepare_stage(
        run,
        row,
        source_revision,
        taxonomy,
        &StageProvenance::default(),
        now,
    )?;
    let body = move |tx: super::tx::Tx| {
        let mut input = input.clone();
        async move {
            input.now = lease_time(input.now, started)?;
            stage_in(&tx, input).await
        }
    };
    let committed = if let Some(db_owner) = db_owner {
        foreground_tx(db_owner, RetryPolicy::default(), body).await?
    } else {
        run_tx(db, RetryPolicy::default(), body).await?
    };
    Ok(committed.value.ok_or(AdmissionError::LeaseLost)?.changed)
}
/// Public legacy correction shares the exact guarded staging transaction body;
/// it cannot reset a live foreground owner's work or publish its own tags.
pub(crate) async fn restage_correction_in(
    tx: &super::tx::Tx,
    row: &AtlasArticleRow,
    now: DateTime<Utc>,
) -> Result<(), TxError> {
    let source = crate::atlas_metadata::input_hash(row);
    let input = prepare_stage(
        None,
        row,
        &source,
        crate::atlas_taxonomy::TAXONOMY_VERSION,
        &StageProvenance::default(),
        now,
    )
    .map_err(|_| TxError::Abort("native_article_correction_exceeds_input_bounds".into()))?;
    stage_in(tx, input)
        .await?
        .ok_or_else(|| TxError::Abort("native_article_correction_blocked_by_live_run".into()))?;
    Ok(())
}

#[derive(Debug, Clone, Default)]
pub struct RunAdmissionSummary {
    pub retained: u64,
    pub admitted: u64,
    pub rejected: u64,
    pub staged: u64,
    pub failed: u64,
    pub legacy: u64,
}
/// SQL aggregates over this run only, without materializing staged/article payloads.
pub async fn run_summary(
    repo: &Repository,
    run: &str,
) -> Result<RunAdmissionSummary, AdmissionError> {
    let current = super::atlas::CURRENT_ADMISSION_FILTER;
    let sql=format!("SELECT count() AS total FROM atlas_article WHERE run_id=$run GROUP ALL; SELECT count() AS total FROM atlas_article WHERE run_id=$run AND ({current}) GROUP ALL; SELECT count() AS total FROM atlas_classification_projection WHERE run_id=$run AND taxonomy_version=$taxonomy AND publication_version=$publication AND admission_state='rejected' GROUP ALL; SELECT count() AS total FROM atlas_admission WHERE run_id=$run GROUP ALL;SELECT count() AS total FROM atlas_admission WHERE run_id=$run AND state='failed' GROUP ALL;SELECT count() AS total FROM atlas_article WHERE run_id=$run AND (taxonomy_version!=$taxonomy OR publication_version!=$publication OR admission_state='legacy') GROUP ALL;SELECT count() AS total FROM atlas_admission WHERE run_id=$run AND !record::exists(type::record('atlas_article',record::id(id))) GROUP ALL;SELECT count() AS total FROM atlas_classification_projection WHERE run_id=$run AND taxonomy_version=$taxonomy AND publication_version=$publication AND admission_state='rejected' AND !record::exists(type::record('atlas_article',record::id(id))) GROUP ALL;");
    let mut response = db_query(repo.db(), sql)
        .bind(("run", run.to_owned()))
        .bind(("taxonomy", crate::atlas_taxonomy::TAXONOMY_VERSION))
        .bind(("publication", PUBLICATION_VERSION))
        .await
        .map_err(TxError::from)?
        .check()
        .map_err(TxError::from)?;
    let mut counts = [0u64; 8];
    for (index, count) in counts.iter_mut().enumerate() {
        let rows: Vec<AggregateCount> = response.take(index).map_err(TxError::from)?;
        if rows.len() > 1 {
            return Err(AdmissionError::InvalidPage {
                code: "unexpected_scope_aggregate".into(),
            });
        }
        *count = rows
            .first()
            .map(|row| u64::try_from(row.total))
            .transpose()
            .map_err(|_| AdmissionError::InvalidPage {
                code: "negative_scope_aggregate".into(),
            })?
            .unwrap_or(0);
    }
    Ok(RunAdmissionSummary {
        retained: counts[0]
            .saturating_add(counts[6])
            .saturating_add(counts[7]),
        admitted: counts[1],
        rejected: counts[2],
        staged: counts[3],
        failed: counts[4],
        legacy: counts[5],
    })
}

/// Decodes server-produced base64 JSON under a byte bound without a new runtime
/// dependency. Base64 makes the SQL response byte count explicit and ASCII-only.
pub(crate) fn decode_wire_json(input: &str, limit: usize) -> Result<Vec<u8>, AdmissionError> {
    let invalid = || AdmissionError::InvalidReceipt {
        code: "invalid_wire_json_encoding".into(),
    };
    if !input.len().is_multiple_of(4) || input.len() > limit.div_ceil(3) * 4 {
        return Err(AdmissionError::PayloadTooLarge {
            kind: "wire_json".into(),
            limit,
        });
    }
    let decode = |byte: u8| -> Option<u8> {
        match byte {
            b'A'..=b'Z' => Some(byte - b'A'),
            b'a'..=b'z' => Some(byte - b'a' + 26),
            b'0'..=b'9' => Some(byte - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    };
    let mut output = Vec::with_capacity((input.len() / 4 * 3).min(limit));
    for (index, chunk) in input.as_bytes().chunks_exact(4).enumerate() {
        let a = decode(chunk[0]).ok_or_else(invalid)?;
        let b = decode(chunk[1]).ok_or_else(invalid)?;
        let padding = if chunk[2] == b'=' {
            if chunk[3] != b'=' {
                return Err(invalid());
            }
            2
        } else if chunk[3] == b'=' {
            1
        } else {
            0
        };
        if padding > 0 && index + 1 != input.len() / 4 {
            return Err(invalid());
        }
        let c = if padding == 2 {
            0
        } else {
            decode(chunk[2]).ok_or_else(invalid)?
        };
        let d = if padding > 0 {
            0
        } else {
            decode(chunk[3]).ok_or_else(invalid)?
        };
        if (padding == 2 && b & 15 != 0) || (padding == 1 && c & 3 != 0) {
            return Err(invalid());
        }
        if output
            .len()
            .checked_add(3 - padding)
            .is_none_or(|bytes| bytes > limit)
        {
            return Err(AdmissionError::PayloadTooLarge {
                kind: "wire_json".into(),
                limit,
            });
        }
        output.push((a << 2) | (b >> 4));
        if padding < 2 {
            output.push((b << 4) | (c >> 2));
        }
        if padding == 0 {
            output.push((c << 6) | d);
        }
    }
    Ok(output)
}

// Append-only draft for the next registered repository checkpoint.
#[derive(SurrealValue)]
struct LegacyInput {
    run_id: String,
    article_id: String,
    payload64: Option<String>,
    input_hash: String,
    source_revision: String,
}
/// Converts at most64 legacy headers/payloads under this run's fence. Repeat
/// bounded calls before capturing through_seq; never scan another run's backlog.
pub async fn upgrade_legacy_page(
    db_owner: &AdmissionOwner,
    run: &RunClaim,
    now: DateTime<Utc>,
) -> Result<usize, AdmissionError> {
    let started = std::time::Instant::now();
    db_owner.ensure_open()?;
    let mut response=db_owner.query("SELECT run_id,article_id,IF bytes::len(type::bytes(payload_json)) <= $limit {encoding::base64::encode(type::bytes(payload_json),true)} ELSE {NONE} AS payload64,input_hash,source_revision,created_at FROM atlas_admission WHERE run_id=$run AND state INSIDE ['pending','failed','evaluating'] AND ((input_revision ?? 0)=0 OR (work_seq ?? 0)=0 OR (classifier_input_hash ?? '')='') ORDER BY created_at,article_id LIMIT 64")
        .bind(("run",run.run_id.clone())).bind(("limit",MAX_PAYLOAD_BYTES as i64))
        .await.map_err(TxError::from)?.check().map_err(TxError::from)?;
    let rows: Vec<LegacyInput> = response.take(0).map_err(TxError::from)?;
    if rows.is_empty() {
        return Ok(0);
    }
    let mut prepared = Vec::with_capacity(rows.len());
    for row in rows {
        let payload = row
            .payload64
            .ok_or_else(|| AdmissionError::PayloadTooLarge {
                kind: "legacy_payload".into(),
                limit: MAX_PAYLOAD_BYTES,
            })?;
        let payload = decode_wire_json(&payload, MAX_PAYLOAD_BYTES)?;
        let article: AtlasArticleRow =
            serde_json::from_slice(&payload).map_err(|_| AdmissionError::InvalidPage {
                code: "legacy_payload_invalid".into(),
            })?;
        let text_bytes = article
            .title
            .len()
            .checked_add(article.description.len())
            .ok_or_else(|| AdmissionError::PayloadTooLarge {
                kind: "classifier_text".into(),
                limit: crate::classifier::training::MAX_ROW_TEXT_BYTES,
            })?;
        if text_bytes > crate::classifier::training::MAX_ROW_TEXT_BYTES {
            return Err(AdmissionError::PayloadTooLarge {
                kind: "classifier_text".into(),
                limit: crate::classifier::training::MAX_ROW_TEXT_BYTES,
            });
        }
        if row.run_id != run.run_id
            || article.run_id != run.run_id
            || article.id != row.article_id
            || crate::atlas_metadata::input_hash(&article) != row.input_hash
            || row.source_revision.is_empty()
        {
            return Err(AdmissionError::InvalidPage {
                code: "legacy_source_identity_mismatch".into(),
            });
        }
        let hash = crate::classifier::ClassifyInput::new(
            "legacy",
            &article.title,
            &article.description,
            None,
        )
        .input_hash;
        prepared.push((row.article_id, row.input_hash, row.source_revision, hash));
    }
    let epoch = checked_i64(run.epoch)?;
    let mut sql=String::from("UPSERT type::record('atlas_classifier_guard',$run_guard) SET updated_at=<datetime>$now RETURN NONE; LET $r=(SELECT * FROM ONLY type::record('atlas_classifier_run',$run)); LET $ok=$r!=NONE AND $r.owner=$owner AND $r.epoch=$epoch AND $r.state='running' AND $r.lease_until> <datetime>$now;");
    for index in 0..prepared.len() {
        sql.push_str(&format!("{{
            IF !$ok {{RETURN false;}};
            UPSERT type::record('atlas_classifier_guard',$guard{index}) SET updated_at=<datetime>$now RETURN NONE;
            LET $s=(SELECT * FROM ONLY type::record('atlas_admission',$key{index}));
            IF $s=NONE OR $s.input_hash!=$hash{index} OR $s.source_revision!=$source{index} {{RETURN false;}};
            IF ($s.owner ?? '')!='' AND $s.lease_until> <datetime>$now {{RETURN false;}};
            IF ($s.input_revision ?? 0)>0 AND ($s.work_seq ?? 0)>0 AND ($s.classifier_input_hash ?? '')!='' {{RETURN false;}};
            LET $sequence=((SELECT VALUE work_generation FROM ONLY type::record('atlas_classifier_run',$run)) ?? 0)+1;
            UPDATE type::record('atlas_classifier_run',$run) SET work_generation=$sequence,updated_at=<datetime>$now RETURN NONE;
            LET $prior=(SELECT input_revision FROM ONLY type::record('atlas_classification_projection',$key{index}));
            LET $guard_head=(SELECT input_revision FROM ONLY type::record('atlas_classifier_guard',$guard{index}));
            LET $revision=math::max([$s.input_revision ?? 0,$prior.input_revision ?? 0,$guard_head.input_revision ?? 0])+1;
            UPDATE type::record('atlas_classifier_guard',$guard{index}) SET input_revision=$revision RETURN NONE;
            UPDATE type::record('atlas_admission',$key{index}) SET input_revision=$revision,work_seq=$sequence,classifier_input_hash=$classifier{index},
                owner='',epoch=($s.epoch ?? 0)+1,lease_until=NONE,state='pending',last_activation='',next_eligible_at=NONE,updated_at=$now RETURN NONE;
            RETURN true;
        }};"));
    }
    sql.push_str("RETURN $ok;");
    let captured = (sql.clone(), run.clone(), prepared.clone());
    let committed = foreground_tx(db_owner, RetryPolicy::default(), move |tx| {
        let (sql, run, prepared) = captured.clone();
        async move {
            let now = lease_time(now, started)?;
            let mut query = tx_query(&tx, sql)
                .bind(("run_guard", codec::encode(&["run", &run.run_id])))
                .bind(("run", run.run_id.clone()))
                .bind(("owner", run.owner))
                .bind(("epoch", epoch))
                .bind(("now", now.to_rfc3339()));
            for (index, (article, hash, source, classifier)) in prepared.iter().enumerate() {
                query = query
                    .bind((
                        format!("key{index}"),
                        codec::encode(&[&run.run_id, article]),
                    ))
                    .bind((
                        format!("guard{index}"),
                        codec::encode(&["source", &run.run_id, article]),
                    ))
                    .bind((format!("hash{index}"), hash.clone()))
                    .bind((format!("source{index}"), source.clone()))
                    .bind((format!("classifier{index}"), classifier.clone()));
            }
            let mut response = query.await?.check()?;
            if response.take::<Option<bool>>(3 + prepared.len())? != Some(true) {
                return Err(TxError::Abort("legacy_upgrade_lease_lost".into()));
            }
            let mut upgraded = 0;
            for index in 0..prepared.len() {
                if response.take::<Option<bool>>(3 + index)? == Some(true) {
                    upgraded += 1;
                }
            }
            Ok(upgraded)
        }
    })
    .await?;
    Ok(committed.value)
}

fn validate_page_token(token: &AdmissionPageToken, run: &RunClaim) -> Result<(), AdmissionError> {
    if token.version != PAGE_VERSION {
        return Err(AdmissionError::UnsupportedVersion {
            kind: "page".into(),
            found: token.version.clone(),
        });
    }
    if token.run_id != run.run_id
        || token.activation_id.is_empty()
        || token.activation_id.len() > 256
        || token.after_seq > token.through_seq
        || token.admission_generation == 0
        || token.admission_generation > i64::MAX as u64
        || token.through_seq > i64::MAX as u64
        || token.model_id.is_empty()
        || token.manifest_sha256.len() != 64
        || !token
            .manifest_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(AdmissionError::InvalidPage {
            code: "page_identity_or_bounds".into(),
        });
    }
    bounded_json(token, 4096, "page_token")?;
    Ok(())
}
#[derive(SurrealValue)]
struct PageRun {
    page_json: String,
    work_generation: i64,
    admission_generation: i64,
    pin_json: String,
}
/// Starts a bounded cohort or resumes its committed cursor. New work after its
/// through_seq is picked up in a subsequent bounded cohort, never OFFSET.
pub async fn begin_admission(
    db_owner: &AdmissionOwner,
    run: &RunClaim,
    context: &crate::atlas_classify::AdmissionContext,
    retry_unresolved: bool,
    now: DateTime<Utc>,
) -> Result<AdmissionPageToken, AdmissionError> {
    let started = std::time::Instant::now();
    db_owner.ensure_open()?;
    let mut response=db_owner.query("SELECT IF bytes::len(type::bytes(page_json))<=4096 {page_json} ELSE {NONE} AS page_json,work_generation,admission_generation,IF bytes::len(type::bytes(pin_json))<=16384 {pin_json} ELSE {NONE} AS pin_json FROM ONLY type::record('atlas_classifier_run',$run)")
        .bind(("run",run.run_id.clone())).await.map_err(TxError::from)?.check().map_err(TxError::from)?;
    let head: PageRun = response
        .take::<Option<PageRun>>(0)
        .map_err(TxError::from)?
        .ok_or(AdmissionError::LeaseLost)?;
    if head.page_json.len() > 4096 || head.pin_json.len() > 16 * 1024 {
        return Err(AdmissionError::PayloadTooLarge {
            kind: "run_admission_identity".into(),
            limit: 16 * 1024,
        });
    }
    let snapshot: Option<crate::classifier::ModelSnapshot> =
        serde_json::from_str(&head.pin_json).map_err(|_| AdmissionError::InvalidPage {
            code: "run_pin_invalid".into(),
        })?;
    if snapshot.as_ref() != Some(&context.pin.snapshot) {
        return Err(AdmissionError::InvalidPage {
            code: "run_pin_mismatch".into(),
        });
    }
    let generation =
        u64::try_from(head.admission_generation).map_err(|_| AdmissionError::InvalidPage {
            code: "negative_admission_generation".into(),
        })?;
    let through_seq =
        u64::try_from(head.work_generation).map_err(|_| AdmissionError::InvalidPage {
            code: "negative_work_sequence".into(),
        })?;
    let existing = if head.page_json.is_empty() {
        None
    } else {
        Some(
            serde_json::from_str::<AdmissionPageToken>(&head.page_json).map_err(|_| {
                AdmissionError::InvalidPage {
                    code: "page_json_invalid".into(),
                }
            })?,
        )
    };
    if let Some(token) = &existing {
        validate_page_token(token, run)?;
        if token.model_id != context.pin.snapshot.model_id
            || token.manifest_sha256 != context.manifest_sha256
            || token.admission_generation != generation
        {
            return Err(AdmissionError::InvalidPage {
                code: "page_pin_changed".into(),
            });
        }
        if token.after_seq < token.through_seq && !retry_unresolved {
            return Ok(token.clone());
        }
    }
    let after_seq = if retry_unresolved {
        0
    } else {
        existing
            .as_ref()
            .map(|token| token.through_seq)
            .unwrap_or(0)
    };
    let token = AdmissionPageToken {
        version: PAGE_VERSION.into(),
        run_id: run.run_id.clone(),
        activation_id: crate::classifier::artifact::sha256_hex(
            codec::encode(&[
                PAGE_VERSION,
                &run.run_id,
                &run.owner,
                &run.epoch.to_string(),
                &now.to_rfc3339(),
            ])
            .as_bytes(),
        ),
        admission_generation: generation,
        through_seq,
        after_seq,
        model_id: context.pin.snapshot.model_id.clone(),
        manifest_sha256: context.manifest_sha256.clone(),
    };
    validate_page_token(&token, run)?;
    let token_json = bounded_json(&token, 4096, "page_token")?;
    let epoch = checked_i64(run.epoch)?;
    let captured = (run.clone(), head.page_json.clone(), token_json.clone());
    let committed=foreground_tx(db_owner,RetryPolicy::default(),move |tx| {
        let (run,old_page,token_json)=captured.clone();
        async move {
            let now = lease_time(now, started)?;
            let mut response=tx_query(&tx,"{
                UPSERT type::record('atlas_classifier_guard',$guard) SET updated_at=<datetime>$now RETURN NONE;
                LET $r=(SELECT * FROM ONLY type::record('atlas_classifier_run',$run));
                IF $r=NONE OR $r.owner!=$owner OR $r.epoch!=$epoch OR $r.lease_until<= <datetime>$now OR $r.state!='running' {RETURN false;};
                IF $r.page_json!=$old_page OR $r.work_generation!=$through {RETURN false;};
                UPDATE type::record('atlas_classifier_run',$run) SET page_json=$page,updated_at=<datetime>$now RETURN NONE;
                RETURN true;
            }").bind(("guard",codec::encode(&["run",&run.run_id]))).bind(("run",run.run_id)).bind(("owner",run.owner)).bind(("epoch",epoch))
                .bind(("old_page",old_page)).bind(("through",checked_i64(through_seq).map_err(|_|TxError::Abort("work_sequence_overflow".into()))?)).bind(("page",token_json)).bind(("now",now.to_rfc3339())).await?.check()?;
            response.take::<Option<bool>>(0).map_err(TxError::from)
        }
    }).await?;
    if committed.value != Some(true) {
        return Err(AdmissionError::InvalidPage {
            code: "page_start_changed".into(),
        });
    }
    Ok(token)
}

#[derive(Debug, Clone, SurrealValue)]
struct ClaimHeader {
    article_id: String,
    input_revision: i64,
    source_revision: String,
    classifier_input_hash: String,
    work_seq: i64,
    epoch: i64,
}
fn claim_from_header(
    run: &RunClaim,
    header: ClaimHeader,
) -> Result<AdmissionClaim, AdmissionError> {
    Ok(AdmissionClaim {
        occurrence: OccurrenceKey::new(&run.run_id, header.article_id),
        owner: run.owner.clone(),
        epoch: u64::try_from(header.epoch).map_err(|_| AdmissionError::InvalidPage {
            code: "negative_claim_epoch".into(),
        })?,
        input_revision: u64::try_from(header.input_revision).map_err(|_| {
            AdmissionError::InvalidPage {
                code: "negative_input_revision".into(),
            }
        })?,
        source_revision: header.source_revision,
        classifier_input_hash: header.classifier_input_hash,
        work_seq: u64::try_from(header.work_seq).map_err(|_| AdmissionError::InvalidPage {
            code: "negative_work_sequence".into(),
        })?,
    })
}
/// Claims a bounded block from the pinned run-local keyset. A page may be
/// smaller than its 512-row ceiling to keep payload and inference memory bounded.
async fn claim_headers_page_inner(
    db_owner: &AdmissionOwner,
    run: &RunClaim,
    token: &AdmissionPageToken,
    now: DateTime<Utc>,
    limit: usize,
    metadata_budget: bool,
) -> Result<AdmissionHeaderPage, AdmissionError> {
    let started = std::time::Instant::now();
    db_owner.ensure_open()?;
    validate_page_token(token, run)?;
    let mut response=db_owner.query("SELECT IF bytes::len(type::bytes(article_id))<=512 {article_id} ELSE {NONE} AS article_id,input_revision,IF bytes::len(type::bytes(source_revision))<=512 {source_revision} ELSE {NONE} AS source_revision,IF bytes::len(type::bytes(classifier_input_hash))<=64 {classifier_input_hash} ELSE {NONE} AS classifier_input_hash,work_seq,epoch FROM atlas_admission WHERE run_id=$run AND work_seq>$after AND work_seq<=$through AND state INSIDE ['pending','failed','evaluating'] AND (next_eligible_at=NONE OR next_eligible_at<= <datetime>$now) AND (lease_until=NONE OR lease_until<= <datetime>$now) AND last_activation!=$activation ORDER BY work_seq,article_id LIMIT $limit")
        .bind(("limit",limit as i64)).bind(("run",run.run_id.clone())).bind(("after",checked_i64(token.after_seq)?)).bind(("through",checked_i64(token.through_seq)?)).bind(("activation",token.activation_id.clone())).bind(("now",now.to_rfc3339())).await.map_err(TxError::from)?.check().map_err(TxError::from)?;
    let wire: Vec<BoundedClaimHeader> = response.take(0).map_err(TxError::from)?;
    let mut headers: Vec<ClaimHeader> = wire
        .into_iter()
        .map(BoundedClaimHeader::into_header)
        .collect::<Result<_, _>>()?;
    if metadata_budget {
        let keys: Vec<_> = headers
            .iter()
            .map(|header| OccurrenceKey::new(&run.run_id, &header.article_id))
            .collect();
        let bounds = db_owner
            .scope(super::atlas_metadata::metadata_wire_bounds(
                db_owner.repository().db(),
                &keys,
            ))
            .await?;
        let mut wire_bytes = 0usize;
        let mut take = 0usize;
        for header in &headers {
            let size = bounds
                .get(&OccurrenceKey::new(&run.run_id, &header.article_id))
                .copied()
                .unwrap_or(1024);
            if wire_bytes
                .checked_add(size)
                .is_none_or(|total| total > MAX_RESPONSE_BYTES)
            {
                break;
            }
            wire_bytes += size;
            take += 1;
        }
        if take == 0 && !headers.is_empty() {
            return Err(AdmissionError::PayloadTooLarge {
                kind: "claimed_metadata".into(),
                limit: MAX_RESPONSE_BYTES,
            });
        }
        headers.truncate(take);
    }
    let examined = headers.len();
    let after = headers
        .last()
        .map(|header| u64::try_from(header.work_seq))
        .transpose()
        .map_err(|_| AdmissionError::InvalidPage {
            code: "negative_work_sequence".into(),
        })?
        .unwrap_or(token.through_seq);
    let mut claimed_headers = Vec::with_capacity(headers.len());
    for batch in headers.chunks(COMMIT_BATCH_ROWS) {
        // A later failure leaves only durable, epoch-fenced leases behind. The
        // page cursor is untouched until publication, so expiry/new activation
        // recovers the reserved prefix just like a process crash after claiming.
        claimed_headers
            .extend(reserve_header_batch(db_owner, run, token, batch, now, started).await?);
    }
    let claims = claimed_headers
        .into_iter()
        .map(|header| claim_from_header(run, header))
        .collect::<Result<Vec<_>, _>>()?;
    let now = lease_time(now, started)?;
    let mut next = token.clone();
    next.after_seq = after;
    if claims.is_empty() {
        checkpoint_empty_page(db_owner, run, token, &next, now).await?;
    }
    let (pending, deferred) =
        pending_counts(db_owner, &run.run_id, &token.activation_id, now).await?;
    let unclaimed = examined.saturating_sub(claims.len());
    Ok(AdmissionHeaderPage {
        claims,
        examined,
        unclaimed,
        next: (after < token.through_seq).then_some(next),
        pending,
        deferred,
    })
}
/// Advances a supplied clock by elapsed wall time without losing its offset.
fn lease_time(
    initial: DateTime<Utc>,
    started: std::time::Instant,
) -> Result<DateTime<Utc>, TxError> {
    let elapsed = Duration::from_std(started.elapsed())
        .map_err(|_| TxError::Abort("lease_clock_overflow".into()))?;
    initial
        .checked_add_signed(elapsed)
        .ok_or_else(|| TxError::Abort("lease_clock_overflow".into()))
}

/// Reserves at most one publication-sized subgroup, keeping the run fence
/// short enough for its independent heartbeat to renew the lease.
async fn reserve_header_batch(
    db_owner: &AdmissionOwner,
    run: &RunClaim,
    token: &AdmissionPageToken,
    headers: &[ClaimHeader],
    now: DateTime<Utc>,
    started: std::time::Instant,
) -> Result<Vec<ClaimHeader>, AdmissionError> {
    if headers.len() > COMMIT_BATCH_ROWS {
        return Err(AdmissionError::InvalidPage {
            code: "claim_reservation_bound".into(),
        });
    }
    let mut sql=String::from("UPSERT type::record('atlas_classifier_guard',$run_guard) SET updated_at=<datetime>$now RETURN NONE; LET $r=(SELECT * FROM ONLY type::record('atlas_classifier_run',$run)); LET $ok=$r!=NONE AND $r.owner=$owner AND $r.epoch=$epoch AND $r.state='running' AND $r.lease_until> <datetime>$now AND $r.page_json=$page AND $r.admission_generation=$generation;");
    for index in 0..headers.len() {
        sql.push_str(&format!("{{
        IF !$ok {{RETURN NONE;}};
        UPSERT type::record('atlas_classifier_guard',$guard{index}) SET updated_at=<datetime>$now RETURN NONE;
        LET $s=(SELECT * FROM ONLY type::record('atlas_admission',$key{index}));
        IF $s=NONE OR $s.work_seq!=$seq{index} OR $s.input_revision!=$revision{index} OR $s.epoch!=$old_epoch{index} OR $s.source_revision!=$source{index} OR $s.classifier_input_hash!=$hash{index} {{RETURN NONE;}};
        IF $s.last_activation=$activation OR $s.state NOTINSIDE ['pending','failed','evaluating'] OR ($s.next_eligible_at!=NONE AND $s.next_eligible_at> <datetime>$now) OR ($s.lease_until!=NONE AND $s.lease_until> <datetime>$now) {{RETURN NONE;}};
        LET $claim_epoch=$s.epoch+1;
        LET $expiry=IF $r.lease_until< <datetime>$until {{$r.lease_until}} ELSE {{<datetime>$until}};
        UPDATE type::record('atlas_admission',$key{index}) SET owner=$owner,epoch=$claim_epoch,lease_until=$expiry,state='evaluating',attempts=attempts+1,last_activation=$activation,updated_at=$now RETURN NONE;
        UPSERT type::record('atlas_work_unit',$unit_key{index}) SET run_id=$run,unit_id=$unit{index},stage=$stage,owner=$owner,epoch=$claim_epoch,lease_until=$expiry,input_revision=$revision{index},source_revision=$source{index},manifest_json=$manifest{index} RETURN NONE;
        RETURN {{article_id:$s.article_id,input_revision:$s.input_revision,source_revision:$s.source_revision,classifier_input_hash:$s.classifier_input_hash,work_seq:$s.work_seq,epoch:$claim_epoch}};
    }};"));
    }
    sql.push_str("RETURN $ok;");
    let token_json = bounded_json(token, 4096, "page_token")?;
    let epoch = checked_i64(run.epoch)?;
    let captured = (
        run.clone(),
        token.clone(),
        headers.to_vec(),
        sql.clone(),
        token_json.clone(),
    );
    let committed = foreground_tx(db_owner, RetryPolicy::default(), move |tx| {
        let (run, token, headers, sql, token_json) = captured.clone();
        async move {
            // Retry attempts must fence against the current lease, including
            // elapsed time while earlier subgroups were being reserved.
            let now = lease_time(now, started)?;
            let mut query = tx_query(&tx, sql)
                .bind(("run_guard", codec::encode(&["run", &run.run_id])))
                .bind(("run", run.run_id.clone()))
                .bind(("owner", run.owner.clone()))
                .bind(("epoch", epoch))
                .bind(("page", token_json))
                .bind(("generation", token.admission_generation as i64))
                .bind(("activation", token.activation_id))
                .bind(("stage", i64::from(crate::atlas_work::STAGE_CLASSIFICATION)))
                .bind(("now", now.to_rfc3339()))
                .bind((
                    "until",
                    (now + Duration::seconds(CLAIM_LEASE_SECS)).to_rfc3339(),
                ));
            for (index, header) in headers.iter().enumerate() {
                let occurrence = OccurrenceKey::new(&run.run_id, &header.article_id);
                let unit = classification_unit_id(&occurrence);
                let manifest = classification_manifest(
                    &occurrence,
                    &header.source_revision,
                    "",
                    now,
                    "",
                    None,
                );
                query = query
                    .bind((
                        format!("guard{index}"),
                        codec::encode(&["source", &run.run_id, &header.article_id]),
                    ))
                    .bind((format!("key{index}"), occurrence.record_id()))
                    .bind((format!("seq{index}"), header.work_seq))
                    .bind((format!("revision{index}"), header.input_revision))
                    .bind((format!("old_epoch{index}"), header.epoch))
                    .bind((format!("source{index}"), header.source_revision.clone()))
                    .bind((format!("hash{index}"), header.classifier_input_hash.clone()))
                    .bind((
                        format!("unit_key{index}"),
                        codec::encode(&[&run.run_id, &unit]),
                    ))
                    .bind((format!("unit{index}"), unit))
                    .bind((
                        format!("manifest{index}"),
                        serde_json::to_string(&manifest)
                            .map_err(|_| TxError::Abort("classification_manifest_json".into()))?,
                    ));
            }
            let mut response = query.await?.check()?;
            if response.take::<Option<bool>>(3 + headers.len())? != Some(true) {
                return Err(TxError::Abort("claim_run_lease_lost".into()));
            }
            let mut claimed = Vec::new();
            for index in 0..headers.len() {
                if let Some(header) = response.take::<Option<ClaimHeader>>(3 + index)? {
                    claimed.push(header);
                }
            }
            Ok(claimed)
        }
    })
    .await?;
    Ok(committed.value)
}

/// A lightweight owned header page. `next` is transient; publication advances
/// the durable cursor only through each examined <=64 subgroup.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct AdmissionHeaderPage {
    pub claims: Vec<AdmissionClaim>,
    pub next: Option<AdmissionPageToken>,
    pub examined: usize,
    pub unclaimed: usize,
    pub pending: u64,
    pub deferred: u64,
}
pub async fn claim_header_page(
    owner: &AdmissionOwner,
    run: &RunClaim,
    token: &AdmissionPageToken,
    now: DateTime<Utc>,
) -> Result<AdmissionHeaderPage, AdmissionError> {
    claim_headers_page_inner(owner, run, token, now, FOREGROUND_PAGE_ROWS, false).await
}
/// Compatibility bounded hydration API. Streaming foreground callers use
/// claim_header_page and hydrate only one <=64 subgroup at a time.
pub async fn claim_page(
    owner: &AdmissionOwner,
    run: &RunClaim,
    token: &AdmissionPageToken,
    now: DateTime<Utc>,
) -> Result<AdmissionPage, AdmissionError> {
    let page = claim_headers_page_inner(owner, run, token, now, COMMIT_BATCH_ROWS, true).await?;
    Ok(AdmissionPage {
        inputs: bulk_inputs(owner, &page.claims).await?,
        next: page.next,
        pending: page.pending,
        deferred: page.deferred,
    })
}

fn classification_unit_id(occurrence: &OccurrenceKey) -> String {
    format!(
        "classification-{}",
        crate::classifier::artifact::sha256_hex(occurrence.record_id().as_bytes())
    )
}
fn classification_manifest(
    occurrence: &OccurrenceKey,
    source_revision: &str,
    model: &str,
    now: DateTime<Utc>,
    disposition: &str,
    receipt: Option<&StoredReceipt>,
) -> crate::atlas_work::UnitManifest {
    let json = receipt
        .map(serde_json::to_string)
        .transpose()
        .expect("validated receipt serializes")
        .unwrap_or_default();
    crate::atlas_work::UnitManifest {
        run_id: occurrence.run_id.clone(),
        unit_id: classification_unit_id(occurrence),
        stage: crate::atlas_work::STAGE_CLASSIFICATION,
        input_ids: vec![occurrence.article_id.clone()],
        input_rev: source_revision.into(),
        contract_version: PUBLICATION_VERSION.into(),
        dependency_ids: vec![],
        is_required: true,
        output_refs: receipt
            .map(|receipt| vec![receipt.key.record_id()])
            .unwrap_or_default(),
        effective_model: model.into(),
        attempt_history: vec![now.to_rfc3339()],
        next_eligible_at: None,
        terminal_reason: None,
        output_json: json.clone(),
        disposition: disposition.into(),
        receipt_json: json,
    }
}
async fn checkpoint_empty_page(
    db_owner: &AdmissionOwner,
    run: &RunClaim,
    prior: &AdmissionPageToken,
    next: &AdmissionPageToken,
    now: DateTime<Utc>,
) -> Result<(), AdmissionError> {
    let started = std::time::Instant::now();
    let prior = bounded_json(prior, 4096, "page_token")?;
    let next = bounded_json(next, 4096, "page_token")?;
    let epoch = checked_i64(run.epoch)?;
    let captured = (run.clone(), prior.clone(), next.clone());
    let committed=foreground_tx(db_owner,RetryPolicy::default(),move |tx| {let (run,prior,next)=captured.clone();async move {
            let now = lease_time(now, started)?;
        let mut response=tx_query(&tx,"{UPSERT type::record('atlas_classifier_guard',$guard) SET updated_at=<datetime>$now RETURN NONE; LET $r=(SELECT * FROM ONLY type::record('atlas_classifier_run',$run)); IF $r=NONE OR $r.owner!=$owner OR $r.epoch!=$epoch OR $r.state!='running' OR $r.lease_until<= <datetime>$now OR $r.page_json!=$prior {RETURN false;}; UPDATE type::record('atlas_classifier_run',$run) SET page_json=$next,updated_at=<datetime>$now RETURN NONE;RETURN true;}").bind(("guard",codec::encode(&["run",&run.run_id]))).bind(("run",run.run_id)).bind(("owner",run.owner)).bind(("epoch",epoch)).bind(("prior",prior)).bind(("next",next)).bind(("now",now.to_rfc3339())).await?.check()?;
        response.take::<Option<bool>>(0).map_err(TxError::from)
    }}).await?;
    if committed.value != Some(true) {
        return Err(AdmissionError::LeaseLost);
    }
    Ok(())
}
#[derive(SurrealValue)]
struct AggregateCount {
    total: i64,
}
async fn pending_counts(
    db_owner: &AdmissionOwner,
    run: &str,
    activation: &str,
    now: DateTime<Utc>,
) -> Result<(u64, u64), AdmissionError> {
    let mut response=db_owner.query("SELECT count() AS total FROM atlas_admission WHERE run_id=$run GROUP ALL;SELECT count() AS total FROM atlas_admission WHERE run_id=$run AND (last_activation=$activation OR lease_until> <datetime>$now OR next_eligible_at> <datetime>$now) GROUP ALL;").bind(("run",run.to_owned())).bind(("activation",activation.to_owned())).bind(("now",now.to_rfc3339())).await.map_err(TxError::from)?.check().map_err(TxError::from)?;
    let mut counts = [0; 2];
    for (index, count) in counts.iter_mut().enumerate() {
        let rows: Vec<AggregateCount> = response.take(index).map_err(TxError::from)?;
        if rows.len() > 1 {
            return Err(AdmissionError::InvalidPage {
                code: "unexpected_pending_aggregate".into(),
            });
        }
        *count = rows
            .first()
            .map(|row| u64::try_from(row.total))
            .transpose()
            .map_err(|_| AdmissionError::InvalidPage {
                code: "negative_pending_count".into(),
            })?
            .unwrap_or(0);
    }
    Ok((counts[0], counts[1]))
}
#[derive(SurrealValue)]
struct StagedInput {
    payload64: Option<String>,
    provenance64: Option<String>,
    live: Option<bool>,
    input_hash: String,
    classifier_input_hash: String,
    input_revision: i64,
    source_revision: String,
    work_seq: i64,
    owner: String,
    epoch: i64,
}
/// Reads only explicitly claimed occurrences. Metadata and body identities are
/// batched; clean body text is intentionally outside classifier input reads.
async fn bulk_inputs_inner(
    db_owner: &AdmissionOwner,
    claims: &[AdmissionClaim],
    allow_changed: bool,
) -> Result<Vec<ClaimedInput>, AdmissionError> {
    db_owner.ensure_open()?;
    if claims.len() > FOREGROUND_PAGE_ROWS {
        return Err(AdmissionError::InvalidPage {
            code: "bulk_claim_limit".into(),
        });
    }
    let mut result = Vec::new();
    for claims in claims.chunks(COMMIT_BATCH_ROWS) {
        let captured_claims = claims.to_vec();
        let committed=owned_tx(db_owner,RetryPolicy::default(),move |tx| {
            let claims=captured_claims.clone();
            async move {
                let mut sql=String::new();
                for index in 0..claims.len() {sql.push_str(&format!("SELECT IF bytes::len(type::bytes(payload_json))<=$payload_limit {{encoding::base64::encode(type::bytes(payload_json),true)}} ELSE {{NONE}} AS payload64,IF bytes::len(type::bytes(provenance_json ?? ''))<=16384 {{encoding::base64::encode(type::bytes(provenance_json ?? ''),true)}} ELSE {{NONE}} AS provenance64,(state='evaluating' AND lease_until> <datetime>$read_now) AS live,input_hash,classifier_input_hash,input_revision,source_revision,work_seq,owner,epoch FROM ONLY $key{index};"));}
                let mut query=tx_query(&tx,sql).bind(("read_now",Utc::now().to_rfc3339())).bind(("payload_limit",MAX_PAYLOAD_BYTES as i64));
                for (index,claim) in claims.iter().enumerate() {query=query.bind((format!("key{index}"),super::jobs::rid("atlas_admission",&claim.occurrence.record_id())));}
                let mut response=query.await?.check()?;
                let staged:Vec<Option<StagedInput>>=(0..claims.len()).map(|index|response.take(index)).collect::<Result<_,_>>()?;
                let keys:Vec<_>=claims.iter().zip(&staged).filter(|(claim,row)|row.as_ref().is_some_and(|row|staged_matches(row,claim) && (!allow_changed || row.live==Some(true)))).map(|(claim,_)|claim.occurrence.clone()).collect();
                let mut metadata=super::atlas_metadata::metadata_many_in(&tx,&keys).await?;
                let articles:Vec<_>=keys.iter().map(|key|key.article_id.clone()).collect();
                let mut body_heads=super::actor_catalog::clean_body_heads_many_in(&tx,&articles).await?;
                let mut inputs=Vec::new();
                for (claim,row) in claims.into_iter().zip(staged) {
                    let Some(row)=row else {if allow_changed {continue;}return Err(TxError::Abort("claimed_source_disappeared".into()));};
                    if !staged_matches(&row,&claim) || (allow_changed && row.live!=Some(true)) {if allow_changed {continue;}return Err(TxError::Abort("claimed_source_changed".into()));}
                    let payload=row.payload64.ok_or_else(||TxError::Abort("claimed_payload_too_large".into()))?;
                    let payload=decode_wire_json(&payload,MAX_PAYLOAD_BYTES).map_err(|_|TxError::Abort("claimed_payload_too_large".into()))?;
                    let article:AtlasArticleRow=serde_json::from_slice(&payload).map_err(|_|TxError::Abort("claimed_payload_invalid".into()))?;
                    if article.run_id!=claim.occurrence.run_id || article.id!=claim.occurrence.article_id || crate::atlas_metadata::input_hash(&article)!=row.input_hash || crate::classifier::ClassifyInput::new("verify",&article.title,&article.description,None).input_hash!=claim.classifier_input_hash {return Err(TxError::Abort("claimed_payload_identity".into()));}
                    let provenance=StageProvenance::from_wire(row.provenance64.as_deref())?.occurrence(&article);
                    bounded_json(&provenance,MAX_PROVENANCE_BYTES,"occurrence_provenance").map_err(|_|TxError::Abort("occurrence_provenance_too_large".into()))?;
                    inputs.push(ClaimedInput {metadata:metadata.remove(&claim.occurrence),body_revision:body_heads.remove(&article.id),claim,article,provenance});
                }
                Ok(inputs)
            }
        }).await?;
        result.extend(committed.value);
    }
    Ok(result)
}

pub async fn bulk_inputs(
    owner: &AdmissionOwner,
    claims: &[AdmissionClaim],
) -> Result<Vec<ClaimedInput>, AdmissionError> {
    bulk_inputs_inner(owner, claims, false).await
}
/// Changed sources are omitted, and the caller retains their original claims
/// for an explicit StaleInput/LeaseLost disposition in commit_outcomes.
pub async fn bulk_inputs_current(
    owner: &AdmissionOwner,
    claims: &[AdmissionClaim],
) -> Result<Vec<ClaimedInput>, AdmissionError> {
    bulk_inputs_inner(owner, claims, true).await
}
/// Releases only claims still owned by this epoch and source revision. It does
/// not clear last_activation, so an operational failure cannot spin this page.
pub async fn release_owned(
    db_owner: &AdmissionOwner,
    run: &RunClaim,
    claims: &[AdmissionClaim],
    now: DateTime<Utc>,
) -> Result<(), AdmissionError> {
    let started = std::time::Instant::now();
    db_owner.ensure_open()?;
    if claims.len() > FOREGROUND_PAGE_ROWS
        || claims
            .iter()
            .any(|claim| claim.occurrence.run_id != run.run_id || claim.owner != run.owner)
    {
        return Err(AdmissionError::InvalidPage {
            code: "release_claim_identity".into(),
        });
    }
    for claims in claims.chunks(COMMIT_BATCH_ROWS) {
        let mut sql=String::from("UPSERT type::record('atlas_classifier_guard',$run_guard) SET updated_at=<datetime>$now RETURN NONE; LET $r=(SELECT * FROM ONLY type::record('atlas_classifier_run',$run)); LET $ok=$r!=NONE AND $r.owner=$owner AND $r.epoch=$epoch AND $r.state='running' AND $r.lease_until> <datetime>$now;");
        for index in 0..claims.len() {
            sql.push_str(&format!("{{IF !$ok {{RETURN false;}};UPSERT type::record('atlas_classifier_guard',$guard{index}) SET updated_at=<datetime>$now RETURN NONE;LET $s=(SELECT * FROM ONLY type::record('atlas_admission',$key{index}));IF $s=NONE OR $s.owner!=$owner OR $s.epoch!=$claim_epoch{index} OR $s.input_revision!=$revision{index} OR $s.source_revision!=$source{index} {{RETURN false;}};UPDATE type::record('atlas_admission',$key{index}) SET state='failed',owner='',epoch=epoch+1,lease_until=NONE,last_error='owned_release',updated_at=$now RETURN NONE;UPDATE type::record('atlas_work_unit',$unit_key{index}) SET owner='',epoch=epoch+1,lease_until=NONE WHERE owner=$owner AND epoch=$claim_epoch{index} RETURN NONE;RETURN true;}};"));
        }
        let epoch = checked_i64(run.epoch)?;
        let captured = (run.clone(), claims.to_vec(), sql.clone());
        foreground_tx(db_owner, RetryPolicy::default(), move |tx| {
            let (run, claims, sql) = captured.clone();
            async move {
                let now = lease_time(now, started)?;
                let mut query = tx_query(&tx, sql)
                    .bind(("run_guard", codec::encode(&["run", &run.run_id])))
                    .bind(("run", run.run_id.clone()))
                    .bind(("owner", run.owner))
                    .bind(("epoch", epoch))
                    .bind(("now", now.to_rfc3339()));
                for (index, claim) in claims.iter().enumerate() {
                    query = query
                        .bind((
                            format!("guard{index}"),
                            codec::encode(&["source", &run.run_id, &claim.occurrence.article_id]),
                        ))
                        .bind((format!("key{index}"), claim.occurrence.record_id()))
                        .bind((format!("claim_epoch{index}"), claim.epoch as i64))
                        .bind((format!("revision{index}"), claim.input_revision as i64))
                        .bind((format!("source{index}"), claim.source_revision.clone()))
                        .bind((
                            format!("unit_key{index}"),
                            codec::encode(&[
                                &run.run_id,
                                &classification_unit_id(&claim.occurrence),
                            ]),
                        ));
                }
                query.await?.check()?;
                Ok(())
            }
        })
        .await?;
    }
    Ok(())
}

#[derive(Clone)]
struct PreparedOutcome {
    claim: AdmissionClaim,
    receipt: Option<StoredReceipt>,
    receipt_json: String,
    receipt_key: String,
    provenance_json: String,
    retry_code: String,
    invalid: Option<String>,
}
#[derive(SurrealValue)]
struct CommitReply {
    kind: String,
    revision: Option<i64>,
}
#[derive(SurrealValue)]
struct CommitRun {
    pin_json: String,
    page_json: String,
    admission_generation: i64,
}
/// Validates each supplied receipt before opening a transaction, then commits
/// independently conditional siblings in short transactions of at most 16.
/// Earlier committed subgroups remain durable if a later subgroup fails;
/// operational failures retain retry work and never become negative evidence.
pub async fn commit_outcomes(
    db_owner: &AdmissionOwner,
    run: &RunClaim,
    items: &[crate::atlas_classify::ItemClassification],
    now: DateTime<Utc>,
) -> Result<BatchCommit, AdmissionError> {
    let started = std::time::Instant::now();
    db_owner.ensure_open()?;
    use crate::atlas_classify::ItemClassification;
    if items.len() > COMMIT_BATCH_ROWS {
        return Err(AdmissionError::InvalidPage {
            code: "commit_batch_limit".into(),
        });
    }
    if items.is_empty() {
        return Ok(BatchCommit {
            items: vec![],
            retries: 0,
        });
    }
    let mut prepared = Vec::with_capacity(items.len());
    let mut seen = std::collections::BTreeSet::new();
    for item in items {
        let (claim, receipt, retry_code) = match item {
            ItemClassification::Evaluated { claim, receipt } => {
                (claim, Some((**receipt).clone()), String::new())
            }
            ItemClassification::Retryable { claim, error } => (
                claim,
                None,
                match error {
                    ClassifierError::Cancelled => "cancelled",
                    ClassifierError::Timeout => "timeout",
                    ClassifierError::Busy => "busy",
                    _ => "classifier_operation_failed",
                }
                .to_string(),
            ),
        };
        if claim.occurrence.run_id != run.run_id
            || claim.owner != run.owner
            || claim.input_revision == 0
            || claim.epoch == 0
            || claim.input_revision > i64::MAX as u64
            || claim.epoch > i64::MAX as u64
            || claim.work_seq > i64::MAX as u64
            || !seen.insert(claim.occurrence.clone())
        {
            return Err(AdmissionError::InvalidPage {
                code: "commit_claim_identity".into(),
            });
        }
        let mut value = PreparedOutcome {
            claim: claim.clone(),
            receipt,
            receipt_json: String::new(),
            receipt_key: String::new(),
            provenance_json: String::new(),
            retry_code,
            invalid: None,
        };
        if let Some(receipt) = &value.receipt {
            let valid = receipt.validate().and_then(|()| {
                if receipt.key.occurrence != claim.occurrence
                    || receipt.key.input_revision != claim.input_revision
                    || receipt.key.source_revision != claim.source_revision
                    || receipt.key.classifier_input_hash != claim.classifier_input_hash
                {
                    return Err(AdmissionError::InvalidReceipt {
                        code: "claim_receipt_identity".into(),
                    });
                }
                Ok(())
            });
            match valid.and_then(|()| {
                Ok((
                    bounded_json(receipt, MAX_RECEIPT_BYTES, "receipt")?,
                    bounded_json(&receipt.provenance, MAX_PROVENANCE_BYTES, "provenance")?,
                ))
            }) {
                Ok((json, provenance)) => {
                    value.receipt_key = receipt.key.record_id();
                    value.receipt_json = json;
                    value.provenance_json = provenance;
                }
                Err(_) => value.invalid = Some("invalid_receipt".into()),
            }
        }
        prepared.push(value);
    }
    let mut result = BatchCommit {
        items: Vec::with_capacity(prepared.len()),
        retries: 0,
    };
    for subgroup in prepared.chunks(PUBLICATION_TRANSACTION_ROWS) {
        // Earlier subgroups remain durably published if a later one fails.
        // Replaying the immutable receipts returns AlreadyCommitted in order.
        let committed = commit_prepared_outcomes(db_owner, run, subgroup, now, started).await?;
        result.items.extend(committed.items);
        result.retries += committed.retries;
    }
    Ok(result)
}

/// Publishes only one short, source-fenced subgroup with per-item receipt
/// atomicity. The public batch was fully validated before the first subgroup.
async fn commit_prepared_outcomes(
    db_owner: &AdmissionOwner,
    run: &RunClaim,
    prepared: &[PreparedOutcome],
    now: DateTime<Utc>,
    started: std::time::Instant,
) -> Result<BatchCommit, AdmissionError> {
    let epoch = checked_i64(run.epoch)?;
    let captured = (run.clone(), prepared.to_vec());
    let committed=foreground_tx(db_owner,RetryPolicy::default(),move |tx| {
        let (run,prepared)=captured.clone();
        async move {
            let now = lease_time(now, started)?;
            // All writers of an occurrence use the same guard. Acquire guards
            // before reading current metadata so native entity/publisher work
            // cannot be lost by classification's merge.
            let mut sql=String::from("UPSERT type::record('atlas_classifier_guard',$run_guard) SET updated_at=<datetime>$now RETURN NONE;");
            for index in 0..prepared.len() {sql.push_str(&format!("UPSERT type::record('atlas_classifier_guard',$guard{index}) SET updated_at=<datetime>$now RETURN NONE;"));}
            sql.push_str("SELECT IF bytes::len(type::bytes(pin_json))<=16384 {pin_json} ELSE {NONE} AS pin_json,IF bytes::len(type::bytes(page_json))<=4096 {page_json} ELSE {NONE} AS page_json,admission_generation FROM ONLY type::record('atlas_classifier_run',$run);");
            let mut query=tx_query(&tx,sql).bind(("run_guard",codec::encode(&["run",&run.run_id]))).bind(("run",run.run_id.clone())).bind(("now",now.to_rfc3339()));
            for (index,item) in prepared.iter().enumerate() {query=query.bind((format!("guard{index}"),codec::encode(&["source",&run.run_id,&item.claim.occurrence.article_id])));}
            let mut response=query.await?.check()?;
            let head=response.take::<Option<CommitRun>>(1+prepared.len())?.ok_or_else(||TxError::Abort("commit_run_missing".into()))?;
            let snapshot:Option<crate::classifier::ModelSnapshot>=serde_json::from_str(&head.pin_json).map_err(|_|TxError::Abort("commit_run_pin_invalid".into()))?;
            let mut page:AdmissionPageToken=serde_json::from_str(&head.page_json).map_err(|_|TxError::Abort("commit_page_invalid".into()))?;
            validate_page_token(&page,&run).map_err(|_|TxError::Abort("commit_page_invalid".into()))?;
            if head.admission_generation!=page.admission_generation as i64 {return Err(TxError::Abort("commit_admission_generation_changed".into()));}
            let keys:Vec<_>=prepared.iter().filter(|item|item.receipt.is_some()).map(|item|item.claim.occurrence.clone()).collect();
            let mut metadata=super::atlas_metadata::metadata_many_in(&tx,&keys).await?;
            let mut sql=String::new();
            for index in 0..prepared.len() {sql.push_str(&format!("SELECT IF bytes::len(type::bytes(payload_json))<=$payload_limit {{encoding::base64::encode(type::bytes(payload_json),true)}} ELSE {{NONE}} AS payload64,IF bytes::len(type::bytes(provenance_json ?? ''))<=16384 {{encoding::base64::encode(type::bytes(provenance_json ?? ''),true)}} ELSE {{NONE}} AS provenance64,input_hash,classifier_input_hash,input_revision,source_revision,work_seq,owner,epoch FROM ONLY $key{index};"));}
            let mut query=tx_query(&tx,sql).bind(("payload_limit",MAX_PAYLOAD_BYTES as i64));
            for (index,item) in prepared.iter().enumerate() {query=query.bind((format!("key{index}"),super::jobs::rid("atlas_admission",&item.claim.occurrence.record_id())));}
            let mut response=query.await?.check()?;
            let mut canonical=Vec::new();let mut merged=Vec::new();let mut invalid=Vec::new();let mut metadata_hashes=Vec::new();let mut metadata_bytes=0usize;
            for (index,item) in prepared.iter().enumerate() {
                let source=response.take::<Option<StagedInput>>(index)?;
                metadata_hashes.push(source.as_ref().map(|source|source.input_hash.clone()).unwrap_or_default());
                let current_matches=source.as_ref().is_some_and(|source|source.input_revision==item.claim.input_revision as i64 && source.source_revision==item.claim.source_revision && source.classifier_input_hash==item.claim.classifier_input_hash && source.work_seq==item.claim.work_seq as i64);
                let article=source.as_ref().filter(|_|current_matches).and_then(|source|source.payload64.as_ref()).and_then(|json|decode_wire_json(json,MAX_PAYLOAD_BYTES).ok()).and_then(|json|serde_json::from_slice::<AtlasArticleRow>(&json).ok());
                let mut invalid_code=item.invalid.clone();
                if current_matches && article.is_none() {invalid_code=Some("staged_payload_invalid".into());}
                let mut row=serde_json::Value::Null;let mut metadata_json=String::new();
                if let Some(receipt)=&item.receipt {
                    if invalid_code.is_none() && (snapshot.as_ref().is_none_or(|pin|pin.model_id!=receipt.key.model_id || pin.artifact_sha256!=receipt.key.artifact_sha256 || pin.taxonomy_version!=receipt.key.taxonomy_version || pin.preprocessing_version!=receipt.key.preprocessing_version || pin.policy_version!=receipt.key.policy_version) || receipt.key.model_id!=page.model_id || receipt.key.manifest_sha256!=page.manifest_sha256) {invalid_code=Some("receipt_pin_mismatch".into());}
                    if let Some(article)=article {
                        let provenance=StageProvenance::from_wire(source.as_ref().and_then(|source|source.provenance64.as_deref()))?.occurrence(&article);
                        if article.run_id!=item.claim.occurrence.run_id || article.id!=item.claim.occurrence.article_id || provenance!=receipt.provenance || crate::classifier::ClassifyInput::new("verify",&article.title,&article.description,None).input_hash!=item.claim.classifier_input_hash {invalid_code=Some("receipt_source_mismatch".into());}
                        let prepared=prepare_publication(article,receipt,metadata.remove(&item.claim.occurrence))?;
                        row=prepared.article;metadata_json=prepared.metadata_json;
                        metadata_hashes[index]=prepared.metadata_hash;

                    }
                }
                metadata_bytes=metadata_bytes.checked_add(metadata_json.len()).ok_or_else(||TxError::Abort("classification_metadata_too_large".into()))?;
                if metadata_bytes>2*MAX_RESPONSE_BYTES {return Err(TxError::Abort("classification_metadata_too_large".into()));}
                canonical.push(row);merged.push(metadata_json);invalid.push(invalid_code);
            }
            let mut sql=String::from("LET $r=(SELECT * FROM ONLY type::record('atlas_classifier_run',$run)); LET $run_ok=$r!=NONE AND $r.owner=$owner AND $r.epoch=$epoch AND $r.state='running' AND $r.lease_until> <datetime>$now AND $r.admission_generation=$generation AND $r.page_json=$old_page;");
            for index in 0..prepared.len() {let publication=publication_sql(index);sql.push_str(&format!("{{
                IF $invalid{index}!='' {{RETURN {{kind:'invalid',revision:NONE}};}};
                IF !$run_ok {{RETURN {{kind:'lease_lost',revision:NONE}};}};
                IF $evaluated{index} {{
                    LET $old=(SELECT receipt_json,receipt_sha256,provenance_json FROM ONLY type::record('atlas_classification_receipt',$receipt_key{index}));
                    IF $old!=NONE {{IF $old.receipt_json!=$receipt_json{index} OR $old.receipt_sha256!=$receipt_hash{index} OR $old.provenance_json!=$provenance{index} {{RETURN {{kind:'receipt_conflict',revision:NONE}};}};RETURN {{kind:'already',revision:NONE}};}};
                }};
                LET $s=(SELECT * FROM ONLY type::record('atlas_admission',$key{index}));
                IF $s=NONE OR $s.input_revision!=$revision{index} OR $s.source_revision!=$source{index} OR $s.classifier_input_hash!=$hash{index} OR $s.work_seq!=$seq{index} {{RETURN {{kind:'stale',revision:$s.input_revision}};}};
                LET $m=(SELECT owner,epoch,lease_until,input_revision,source_revision FROM ONLY type::record('atlas_work_unit',$unit_key{index}));
                IF $s.owner!=$owner OR $s.epoch!=$claim_epoch{index} OR $s.state!='evaluating' OR $s.lease_until=NONE OR $s.lease_until<= <datetime>$now OR $m.owner!=$owner OR $m.epoch!=$claim_epoch{index} OR $m.lease_until<= <datetime>$now OR $m.input_revision!=$revision{index} OR $m.source_revision!=$source{index} {{RETURN {{kind:'lease_lost',revision:$s.input_revision}};}};
                IF !$evaluated{index} {{
                    UPDATE type::record('atlas_admission',$key{index}) SET state='failed',owner='',lease_until=NONE,last_error=$retry{index},updated_at=$now RETURN NONE;
                    UPDATE type::record('atlas_work_unit',$unit_key{index}) SET owner='',lease_until=NONE,manifest_json=$manifest{index} RETURN NONE;
                    RETURN {{kind:'retry',revision:$s.input_revision}};
                }};
                {publication}
                UPDATE type::record('atlas_work_unit',$unit_key{index}) SET owner='',lease_until=NONE,manifest_json=$manifest{index} RETURN NONE;
                DELETE type::record('atlas_admission',$key{index}) RETURN NONE;
                RETURN {{kind:'committed',revision:$s.input_revision}};
            }};"));}
            page.after_seq=page.after_seq.max(prepared.iter().map(|item|item.claim.work_seq).max().unwrap_or(page.after_seq)).min(page.through_seq);
            sql.push_str("IF $run_ok {UPDATE type::record('atlas_classifier_run',$run) SET page_json=$page,updated_at=<datetime>$now RETURN NONE;}; RETURN $run_ok;");
            let mut query=tx_query(&tx,sql).bind(("run",run.run_id.clone())).bind(("owner",run.owner)).bind(("epoch",epoch)).bind(("generation",head.admission_generation)).bind(("old_page",head.page_json)).bind(("page",serde_json::to_string(&page).map_err(|_|TxError::Abort("commit_page_json".into()))?)).bind(("now",now.to_rfc3339())).bind(("taxonomy",crate::atlas_taxonomy::TAXONOMY_VERSION)).bind(("publication",PUBLICATION_VERSION));
            for (index,item) in prepared.iter().enumerate() {
                let receipt=item.receipt.as_ref();let admitted=receipt.is_some_and(|receipt|receipt.receipt.is_admitted());
                let receipt_row=receipt.map(|receipt|receipt_row(receipt,&item.receipt_json,&item.provenance_json)).unwrap_or(serde_json::Value::Null);
                let disposition=if receipt.is_none() {crate::atlas_work::DISPOSITION_INCOMPLETE} else if admitted {crate::atlas_work::DISPOSITION_SUCCESS} else {crate::atlas_work::DISPOSITION_REJECTED};
                let mut manifest=classification_manifest(&item.claim.occurrence,&item.claim.source_revision,receipt.map(|receipt|receipt.key.model_id.as_str()).unwrap_or(""),now,disposition,receipt);
                if receipt.is_none() {manifest.terminal_reason=Some(item.retry_code.clone());}
                let metadata=if merged[index].is_empty() {None} else {Some(serde_json::from_str::<ArticleMetadata>(&merged[index]).map_err(|_|TxError::Abort("commit_metadata_json".into()))?)};
                query=query.bind((format!("occurrence_run{index}"),item.claim.occurrence.run_id.clone())).bind((format!("metadata_hash{index}"),metadata_hashes[index].clone())).bind((format!("key{index}"),item.claim.occurrence.record_id())).bind((format!("article_id{index}"),item.claim.occurrence.article_id.clone())).bind((format!("revision{index}"),item.claim.input_revision as i64)).bind((format!("source{index}"),item.claim.source_revision.clone())).bind((format!("hash{index}"),item.claim.classifier_input_hash.clone())).bind((format!("seq{index}"),item.claim.work_seq as i64)).bind((format!("claim_epoch{index}"),item.claim.epoch as i64)).bind((format!("unit_key{index}"),codec::encode(&[&run.run_id,&classification_unit_id(&item.claim.occurrence)]))).bind((format!("invalid{index}"),invalid[index].clone().unwrap_or_default())).bind((format!("evaluated{index}"),receipt.is_some())).bind((format!("admitted{index}"),admitted)).bind((format!("receipt_key{index}"),item.receipt_key.clone())).bind((format!("receipt_json{index}"),item.receipt_json.clone())).bind((format!("receipt_hash{index}"),crate::classifier::artifact::sha256_hex(item.receipt_json.as_bytes()))).bind((format!("provenance{index}"),item.provenance_json.clone())).bind((format!("receipt_row{index}"),receipt_row)).bind((format!("article{index}"),canonical[index].clone())).bind((format!("metadata{index}"),merged[index].clone())).bind((format!("entity{index}"),metadata.as_ref().map(|metadata|metadata.entity_state.as_str()).unwrap_or("pending").to_owned())).bind((format!("reliability{index}"),metadata.as_ref().map(|metadata|metadata.reliability_state.as_str()).unwrap_or("pending").to_owned())).bind((format!("retry{index}"),item.retry_code.clone())).bind((format!("manifest{index}"),serde_json::to_string(&manifest).map_err(|_|TxError::Abort("commit_manifest_json".into()))?));
            }
            let mut response=query.await?.check()?;
            let mut result=Vec::with_capacity(prepared.len());
            for (index,item) in prepared.iter().enumerate() {
                let reply=response.take::<Option<CommitReply>>(2+index)?.ok_or_else(||TxError::Abort("missing_item_commit_reply".into()))?;
                let outcome=match reply.kind.as_str() {
                    "committed"=>ItemCommit::Committed {receipt_key:item.receipt_key.clone()},"already"=>ItemCommit::AlreadyCommitted {receipt_key:item.receipt_key.clone()},
                    "stale"=>ItemCommit::StaleInput {occurrence:item.claim.occurrence.clone(),current_revision:reply.revision.and_then(|revision|u64::try_from(revision).ok())},
                    "invalid"=>ItemCommit::InvalidReceipt {occurrence:item.claim.occurrence.clone(),code:invalid[index].clone().unwrap_or_else(||"invalid_receipt".into())},
                    "lease_lost"=>ItemCommit::LeaseLost {occurrence:item.claim.occurrence.clone()},
                    "retry"=>if item.retry_code=="cancelled" {ItemCommit::Cancelled {occurrence:item.claim.occurrence.clone()}} else {ItemCommit::Retryable {occurrence:item.claim.occurrence.clone(),code:item.retry_code.clone()}},
                    "receipt_conflict"=>return Err(TxError::Abort(format!("classifier_receipt_conflict:{index}"))),
                    _=>return Err(TxError::Abort("unknown_item_commit_reply".into())),
                };
                if matches!(outcome,ItemCommit::Committed {..}|ItemCommit::AlreadyCommitted {..}) {
                    if let Some(receipt)=&item.receipt { super::atlas_derivative_status::invalidate_in(&tx,&item.claim.occurrence,&item.receipt_key,receipt.receipt.is_admitted()).await?; }
                    crate::atlas_derivative_worker::enqueue_cleanup_in(&tx,&item.claim.occurrence,now).await?;
                }
                if matches!(outcome,ItemCommit::Committed {..}|ItemCommit::AlreadyCommitted {..}) && item.receipt.as_ref().is_some_and(|receipt|receipt.receipt.is_admitted()) {
                    crate::atlas_derivative_worker::enqueue_in(&tx,&item.claim.occurrence,&item.receipt_key,now).await?;
                }
                result.push(outcome);
            }
            Ok(result)
        }
    }).await;
    match committed {
        Ok(committed) => Ok(BatchCommit {
            items: committed.value,
            retries: committed.retries,
        }),
        Err(AdmissionError::Storage(TxError::Abort(code)))
            if code.starts_with("classifier_receipt_conflict:") =>
        {
            let index = code
                .trim_start_matches("classifier_receipt_conflict:")
                .parse::<usize>()
                .map_err(|_| AdmissionError::InvalidReceipt {
                    code: "receipt_conflict_correlation".into(),
                })?;
            Err(AdmissionError::ReceiptConflict {
                key: prepared
                    .get(index)
                    .map(|item| item.receipt_key.clone())
                    .unwrap_or_default(),
            })
        }
        Err(error) => Err(error),
    }
}

// Owned classifier-local transaction lifecycle. @tag:classifier
pub use owned::AdmissionOwner;
pub(crate) use owned::{
    db_query, foreground_tx, measured_query, owned_tx, tx_query, MeasuredQuery,
};
mod owned {
    use super::super::repo::Repository;
    use super::super::tx::{Committed, RetryPolicy, Tx, TxError};
    use super::{AdmissionError, DbMetrics};
    use std::borrow::Cow;
    use std::future::{Future, IntoFuture};
    use std::pin::Pin;
    use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use surrealdb::engine::any::Any;
    use surrealdb::method::{IntoVariables, Query};
    use surrealdb::{IndexedResults, Surreal};
    use tokio::sync::{oneshot, Semaphore};
    use tokio::task::JoinHandle;

    const ACTIVE_WORKERS: usize = 2;
    const RETAINED_HANDLES: usize = 8;
    const RUNNING: u8 = 0;
    const CANCEL_REQUESTED: u8 = 1;
    const COMMIT_ISSUED: u8 = 2;
    const SETTLED: u8 = 3;
    tokio::task_local! {static CURRENT_METRICS:DbMetrics;}

    #[derive(Default)]
    struct Control {
        phase: AtomicU8,
        cancel_requested: AtomicBool,
    }
    impl Control {
        fn cancel(&self) {
            // This CAS linearizes cancellation against beginning COMMIT. A
            // cancellation that loses to COMMIT must await its actual result.
            let _ = self.phase.compare_exchange(
                RUNNING,
                CANCEL_REQUESTED,
                Ordering::AcqRel,
                Ordering::Acquire,
            );
            self.cancel_requested.store(true, Ordering::Release);
        }
        fn cancelled(&self) -> bool {
            self.cancel_requested.load(Ordering::Acquire)
                || self.phase.load(Ordering::Acquire) == CANCEL_REQUESTED
        }
    }
    #[derive(Clone)]
    struct Worker {
        control: Arc<Control>,
        handle: Arc<tokio::sync::Mutex<Option<JoinHandle<()>>>>,
    }
    struct OwnerState {
        repo: Repository,
        metrics: DbMetrics,
        workers: Mutex<Vec<Worker>>,
        slots: Arc<Semaphore>,
        foreground_gate: Arc<tokio::sync::Mutex<()>>,
        closing: Arc<AtomicBool>,
        fatal: Arc<Mutex<Option<AdmissionError>>>,
    }
    impl Drop for OwnerState {
        fn drop(&mut self) {
            self.closing.store(true, Ordering::Release);
            if let Ok(workers) = self.workers.get_mut() {
                for worker in workers {
                    worker.control.cancel();
                }
            }
            // Dropping JoinHandle detaches; it never aborts the worker that
            // owns an in-flight BEGIN/CANCEL/COMMIT. Explicit shutdown joins.
        }
    }
    /// Owner of bounded database workers for one cycle or maintenance job.
    ///
    /// Call shutdown on every normal/cancel/lease-loss exit. Final Drop requests
    /// cleanup without aborting an owned transaction; process loss is outside
    /// this guarantee. Workers capture owned repository/inputs, never this Arc.
    #[derive(Clone)]
    pub struct AdmissionOwner {
        inner: Arc<OwnerState>,
    }
    impl AdmissionOwner {
        #[must_use]
        pub fn new(repo: Repository) -> Self {
            Self {
                inner: Arc::new(OwnerState {
                    repo,
                    metrics: DbMetrics::default(),
                    workers: Mutex::new(Vec::new()),
                    slots: Arc::new(Semaphore::new(ACTIVE_WORKERS)),
                    foreground_gate: Arc::new(tokio::sync::Mutex::new(())),
                    closing: Arc::new(AtomicBool::new(false)),
                    fatal: Arc::new(Mutex::new(None)),
                }),
            }
        }
        pub(crate) fn ensure_open(&self) -> Result<(), AdmissionError> {
            if let Some(error) = self
                .inner
                .fatal
                .lock()
                .map_err(|_| TxError::Abort("admission_owner_mutex_poisoned".into()))?
                .clone()
            {
                return Err(error);
            }
            if self.inner.closing.load(Ordering::Acquire) {
                return Err(AdmissionError::InvalidRunState {
                    code: "owner_stopped".into(),
                });
            }
            Ok(())
        }
        #[must_use]
        pub fn repository(&self) -> &Repository {
            &self.inner.repo
        }
        #[must_use]
        pub fn metrics(&self) -> DbMetrics {
            self.inner.metrics.clone()
        }
        pub(crate) fn query<'a>(&'a self, sql: impl Into<Cow<'a, str>>) -> MeasuredQuery<'a> {
            MeasuredQuery {
                query: self.inner.repo.db().query(sql),
                metrics: Some(self.inner.metrics.clone()),
            }
        }
        /// Runs read helpers with the same operation-local counters.
        pub fn scope<F: Future>(&self, future: F) -> impl Future<Output = F::Output> {
            // Keep large operations out of the task-local wrapper's poll frame
            // before entering deeply nested embedded queries.
            CURRENT_METRICS.scope(self.metrics(), Box::pin(future))
        }
        async fn reap(&self, all: bool) -> Result<(), AdmissionError> {
            // Retain every handle under the owner while awaiting it. Dropping
            // a join/reap future leaves its handle available to a later join.
            let candidates = {
                let workers = self
                    .inner
                    .workers
                    .lock()
                    .map_err(|_| TxError::Abort("admission_owner_mutex_poisoned".into()))?;
                workers
                    .iter()
                    .filter(|worker| all || worker.control.phase.load(Ordering::Acquire) == SETTLED)
                    .cloned()
                    .collect::<Vec<_>>()
            };
            for worker in candidates {
                let mut slot = worker.handle.lock().await;
                let result = if let Some(handle) = slot.as_mut() {
                    let result = handle.await;
                    slot.take();
                    result
                } else {
                    Ok(())
                };
                drop(slot);
                self.inner
                    .workers
                    .lock()
                    .map_err(|_| TxError::Abort("admission_owner_mutex_poisoned".into()))?
                    .retain(|candidate| !Arc::ptr_eq(&candidate.handle, &worker.handle));
                result.map_err(|_| TxError::Abort("admission_transaction_worker_failed".into()))?;
            }
            if let Some(error) = self
                .inner
                .fatal
                .lock()
                .map_err(|_| TxError::Abort("admission_owner_mutex_poisoned".into()))?
                .clone()
            {
                return Err(error);
            }
            Ok(())
        }
        /// Joins current workers. Callers must quiesce new work first.
        pub async fn join(&self) -> Result<(), AdmissionError> {
            self.reap(true).await
        }
        /// Stops dispatch, requests cancellation before COMMIT, and joins all
        /// owned workers, including transactions whose awaiting caller left.
        pub async fn shutdown(&self) -> Result<(), AdmissionError> {
            self.inner.closing.store(true, Ordering::Release);
            {
                let workers = self
                    .inner
                    .workers
                    .lock()
                    .map_err(|_| TxError::Abort("admission_owner_mutex_poisoned".into()))?;
                for worker in workers.iter() {
                    worker.control.cancel();
                }
            }
            self.join().await
        }
    }
    struct Awaiter {
        control: Arc<Control>,
        settled: bool,
    }
    impl Drop for Awaiter {
        fn drop(&mut self) {
            if !self.settled {
                self.control.cancel();
            }
        }
    }

    /// Bounded owned counterpart of storage::tx::run_tx. Keep its boundary
    /// flow coupled to that helper: default RetryPolicy, begin failures do not
    /// retry, closure errors cancel, typed conflicts retry, exhaustion is typed.
    /// The intentional difference is that dropping the awaiting caller cannot
    /// drop a transaction boundary future. Only database work belongs in body.
    pub(crate) async fn owned_tx<T, F, Fut>(
        owner: &AdmissionOwner,
        policy: RetryPolicy,
        body: F,
    ) -> Result<Committed<T>, AdmissionError>
    where
        T: Send + 'static,
        F: FnMut(Tx) -> Fut + Send + 'static,
        Fut: Future<Output = Result<T, TxError>> + Send + 'static,
    {
        let driver = SdkDriver {
            repo: owner.inner.repo.clone(),
        };
        owned_tx_with_driver(owner, policy, driver, body).await
    }
    /// Gives same-owner heartbeat and short foreground writes FIFO turns.
    /// The worker retains the guard through COMMIT/CANCEL even if its awaiting
    /// caller leaves. Database guards still fence other owners and processes.
    pub(crate) async fn foreground_tx<T, F, Fut>(
        owner: &AdmissionOwner,
        policy: RetryPolicy,
        mut body: F,
    ) -> Result<Committed<T>, AdmissionError>
    where
        T: Send + 'static,
        F: FnMut(Tx) -> Fut + Send + 'static,
        Fut: Future<Output = Result<T, TxError>> + Send + 'static,
    {
        owner.ensure_open()?;
        let gate = Arc::new(owner.inner.foreground_gate.clone().lock_owned().await);
        owned_tx(owner, policy, move |tx| {
            let gate = gate.clone();
            let future = body(tx);
            async move {
                let _gate = gate;
                future.await
            }
        })
        .await
    }

    async fn owned_tx_with_driver<D, T, F, Fut>(
        owner: &AdmissionOwner,
        policy: RetryPolicy,
        driver: D,
        body: F,
    ) -> Result<Committed<T>, AdmissionError>
    where
        D: Driver<Handle = Tx>,
        T: Send + 'static,
        F: FnMut(Tx) -> Fut + Send + 'static,
        Fut: Future<Output = Result<T, TxError>> + Send + 'static,
    {
        owner.ensure_open()?;
        let permit = owner
            .inner
            .slots
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| TxError::Abort("admission_owner_closed".into()))?;
        owner.reap(false).await?;
        // Cap even handles that finished between reaping and slot acquisition.
        if owner
            .inner
            .workers
            .lock()
            .map_err(|_| TxError::Abort("admission_owner_mutex_poisoned".into()))?
            .len()
            >= RETAINED_HANDLES
        {
            owner.join().await?;
        }
        let (sender, receiver) = oneshot::channel();
        let control = Arc::new(Control::default());
        let mut awaiter = Awaiter {
            control: control.clone(),
            settled: false,
        };
        let metrics = owner.metrics();
        let closing = owner.inner.closing.clone();
        let fatal = owner.inner.fatal.clone();
        {
            let mut workers = owner
                .inner
                .workers
                .lock()
                .map_err(|_| TxError::Abort("admission_owner_mutex_poisoned".into()))?;
            if owner.inner.closing.load(Ordering::Acquire) {
                return Err(AdmissionError::InvalidRunState {
                    code: "owner_stopped".into(),
                });
            }
            if workers.len() >= RETAINED_HANDLES {
                return Err(AdmissionError::InvalidRunState {
                    code: "worker_handle_limit".into(),
                });
            }
            let worker_control = control.clone();
            let handle = tokio::spawn(async move {
                let result = CURRENT_METRICS
                    .scope(
                        metrics.clone(),
                        drive(
                            driver,
                            metrics,
                            worker_control.clone(),
                            closing.clone(),
                            policy,
                            body,
                        ),
                    )
                    .await;
                if matches!(&result, Err(AdmissionError::CleanupUncertain { .. })) {
                    if let Ok(mut fatal) = fatal.lock() {
                        *fatal = result.as_ref().err().cloned();
                    }
                    closing.store(true, Ordering::Release);
                }
                worker_control.phase.store(SETTLED, Ordering::Release);
                let _ = sender.send(result);
                drop(permit);
            });
            workers.push(Worker {
                control,
                handle: Arc::new(tokio::sync::Mutex::new(Some(handle))),
            });
        }
        let result = receiver
            .await
            .map_err(|_| TxError::Abort("admission_transaction_worker_failed".into()))?;
        awaiter.settled = true;
        result
    }

    trait Driver: Send + Sync + 'static {
        type Handle: Clone + Send + Sync + 'static;
        fn begin(&self) -> impl Future<Output = Result<Self::Handle, TxError>> + Send;
        fn commit(&self, tx: Self::Handle) -> impl Future<Output = Result<(), TxError>> + Send;
        fn cancel(&self, tx: Self::Handle) -> impl Future<Output = Result<(), TxError>> + Send;
    }
    struct SdkDriver {
        repo: Repository,
    }
    impl Driver for SdkDriver {
        type Handle = Tx;
        async fn begin(&self) -> Result<Tx, TxError> {
            Ok(Arc::new(self.repo.db().clone().begin().await?))
        }
        async fn commit(&self, tx: Tx) -> Result<(), TxError> {
            Arc::try_unwrap(tx)
                .map_err(|_| TxError::Abort("transaction_body_retained_handle".into()))?
                .commit()
                .await?;
            Ok(())
        }
        async fn cancel(&self, tx: Tx) -> Result<(), TxError> {
            Arc::try_unwrap(tx)
                .map_err(|_| TxError::Abort("transaction_body_retained_handle".into()))?
                .cancel()
                .await?;
            Ok(())
        }
    }
    async fn drive<D, T, F, Fut>(
        driver: D,
        metrics: DbMetrics,
        control: Arc<Control>,
        closing: Arc<AtomicBool>,
        policy: RetryPolicy,
        mut body: F,
    ) -> Result<Committed<T>, AdmissionError>
    where
        D: Driver,
        F: FnMut(D::Handle) -> Fut,
        Fut: Future<Output = Result<T, TxError>>,
    {
        let attempts = policy.max_attempts.max(1);
        for attempt in 0..attempts {
            if attempt > 0 {
                tokio::time::sleep(backoff(policy, attempt)).await;
            }
            if control.cancelled() || closing.load(Ordering::Acquire) {
                return Err(AdmissionError::Cancelled);
            }
            if attempt > 0 {
                metrics.committed(1);
            }
            metrics.begin();
            let tx = driver.begin().await?;
            // BEGIN always resolves before cleanup. Never select it away.
            if control.cancelled() || closing.load(Ordering::Acquire) {
                cancel_attempt(
                    &driver,
                    tx,
                    &metrics,
                    &closing,
                    TxError::Abort("admission_transaction_cancelled".into()),
                )
                .await?;
                return Err(AdmissionError::Cancelled);
            }
            let result = body(tx.clone()).await;
            // The bounded pure-DB body also settles before its transaction is
            // cancelled; it cannot continue issuing requests after rollback.
            match result {
                Ok(value) => {
                    if closing.load(Ordering::Acquire) {
                        control.cancel();
                    }
                    if control
                        .phase
                        .compare_exchange(
                            RUNNING,
                            COMMIT_ISSUED,
                            Ordering::AcqRel,
                            Ordering::Acquire,
                        )
                        .is_err()
                    {
                        cancel_attempt(
                            &driver,
                            tx,
                            &metrics,
                            &closing,
                            TxError::Abort("admission_transaction_cancelled".into()),
                        )
                        .await?;
                        return Err(AdmissionError::Cancelled);
                    }
                    metrics.commit();
                    // Once issued, await the outcome even if the receiver left.
                    match driver.commit(tx).await {
                        Ok(()) => {
                            return Ok(Committed {
                                value,
                                retries: attempt,
                            })
                        }
                        Err(error) if error.is_conflict() => {
                            if control.cancelled() || closing.load(Ordering::Acquire) {
                                return Err(AdmissionError::Cancelled);
                            }
                            control.phase.store(RUNNING, Ordering::Release);
                            continue;
                        }
                        Err(error) => return Err(error.into()),
                    }
                }
                Err(error) => {
                    let error = cancel_attempt(&driver, tx, &metrics, &closing, error).await?;
                    if error.is_conflict() {
                        continue;
                    }
                    return Err(error.into());
                }
            }
        }
        Err(TxError::Contention { attempts }.into())
    }
    async fn cancel_attempt<D: Driver>(
        driver: &D,
        tx: D::Handle,
        metrics: &DbMetrics,
        closing: &AtomicBool,
        operation: TxError,
    ) -> Result<TxError, AdmissionError> {
        metrics.rollback();
        match driver.cancel(tx).await {
            Ok(()) => Ok(operation),
            Err(cleanup) => {
                closing.store(true, Ordering::Release);
                Err(AdmissionError::CleanupUncertain {
                    operation: Box::new(operation),
                    cleanup: Box::new(cleanup),
                })
            }
        }
    }
    // Intentionally the same bounded full-jitter policy as storage::tx::backoff.
    // Keep this coupled when that helper changes; no classifier-specific retry
    // cap or unbounded sleep is introduced.
    fn backoff(policy: RetryPolicy, retry: u32) -> Duration {
        use std::hash::{BuildHasher, Hasher};
        let ceiling = policy
            .base
            .saturating_mul(1u32 << (retry - 1).min(16))
            .min(policy.cap);
        let nanos = ceiling.as_nanos() as u64;
        if nanos == 0 {
            return Duration::ZERO;
        }
        let mut hash = std::collections::hash_map::RandomState::new().build_hasher();
        hash.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or_default(),
        );
        Duration::from_nanos(hash.finish() % (nanos + 1))
    }
    /// Query construction is inert. Count only when its future is polled/issued.
    pub(crate) struct MeasuredQuery<'a> {
        query: Query<'a, Any>,
        metrics: Option<DbMetrics>,
    }
    impl MeasuredQuery<'_> {
        pub(crate) fn bind(mut self, vars: impl IntoVariables) -> Self {
            self.query = self.query.bind(vars);
            self
        }
    }
    impl<'a> IntoFuture for MeasuredQuery<'a> {
        type Output = surrealdb::Result<IndexedResults>;
        type IntoFuture = Pin<Box<dyn Future<Output = Self::Output> + Send + 'a>>;
        fn into_future(self) -> Self::IntoFuture {
            Box::pin(async move {
                if let Some(metrics) = self.metrics {
                    metrics.query();
                }
                self.query.await
            })
        }
    }
    pub(crate) fn measured_query(query: Query<'_, Any>) -> MeasuredQuery<'_> {
        MeasuredQuery {
            query,
            metrics: CURRENT_METRICS.try_with(Clone::clone).ok(),
        }
    }
    pub(crate) fn db_query<'a>(
        db: &'a Surreal<Any>,
        sql: impl Into<Cow<'a, str>>,
    ) -> MeasuredQuery<'a> {
        measured_query(db.query(sql))
    }
    pub(crate) fn tx_query<'a>(tx: &'a Tx, sql: impl Into<Cow<'a, str>>) -> MeasuredQuery<'a> {
        measured_query(tx.query(sql))
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        use std::collections::VecDeque;
        use tokio::sync::Notify;
        async fn barrier(notify: &Notify) {
            tokio::time::timeout(Duration::from_secs(30), notify.notified())
                .await
                .expect("controlled transaction barrier timed out");
        }
        #[derive(Default)]
        struct Gate {
            entered: Notify,
            release: Notify,
        }
        impl Gate {
            async fn pass(&self) {
                self.entered.notify_one();
                barrier(&self.release).await;
            }
        }
        struct FakeDriver {
            trace: Arc<Mutex<Vec<&'static str>>>,
            begin_error: bool,
            cancel_error: bool,
            commits: Mutex<VecDeque<bool>>,
            begin_gate: Option<Arc<Gate>>,
            commit_gate: Option<Arc<Gate>>,
        }
        impl FakeDriver {
            fn new(commits: impl IntoIterator<Item = bool>) -> Self {
                Self {
                    trace: Arc::new(Mutex::new(vec![])),
                    begin_error: false,
                    cancel_error: false,
                    commits: Mutex::new(commits.into_iter().collect()),
                    begin_gate: None,
                    commit_gate: None,
                }
            }
            fn event(&self, event: &'static str) {
                self.trace.lock().unwrap().push(event);
            }
        }
        fn conflict() -> TxError {
            TxError::Db(surrealdb::Error::query(
                "controlled conflict".to_string(),
                surrealdb::types::QueryError::TransactionConflict,
            ))
        }
        impl Driver for FakeDriver {
            type Handle = Arc<()>;
            async fn begin(&self) -> Result<Self::Handle, TxError> {
                self.event("begin");
                if let Some(gate) = &self.begin_gate {
                    gate.pass().await;
                }
                if self.begin_error {
                    Err(TxError::Abort("begin_failed".into()))
                } else {
                    Ok(Arc::new(()))
                }
            }
            async fn commit(&self, _tx: Self::Handle) -> Result<(), TxError> {
                self.event("commit");
                if let Some(gate) = &self.commit_gate {
                    gate.pass().await;
                }
                if self.commits.lock().unwrap().pop_front().unwrap_or(false) {
                    Err(conflict())
                } else {
                    Ok(())
                }
            }
            async fn cancel(&self, _tx: Self::Handle) -> Result<(), TxError> {
                self.event("rollback");
                if self.cancel_error {
                    Err(TxError::Abort("controlled_cleanup_failure".into()))
                } else {
                    Ok(())
                }
            }
        }
        fn no_wait_policy() -> RetryPolicy {
            RetryPolicy {
                base: Duration::ZERO,
                cap: Duration::ZERO,
                ..RetryPolicy::default()
            }
        }
        #[tokio::test]
        async fn actual_boundary_traces_preserve_success_failure_and_retry_policy() {
            for (conflicts, expected, retries) in [
                (vec![false], vec!["begin", "commit"], 0),
                (
                    vec![true, false],
                    vec!["begin", "commit", "begin", "commit"],
                    1,
                ),
            ] {
                let driver = FakeDriver::new(conflicts);
                let trace = driver.trace.clone();
                let metrics = DbMetrics::default();
                let result = drive(
                    driver,
                    metrics.clone(),
                    Arc::new(Control::default()),
                    Arc::new(AtomicBool::new(false)),
                    no_wait_policy(),
                    |_| async { Ok(7) },
                )
                .await
                .unwrap();
                assert_eq!(*trace.lock().unwrap(), expected);
                assert_eq!(result.value, 7);
                assert_eq!(result.retries, retries);
                assert_eq!(metrics.conflict_retries(), u64::from(retries));
                assert_eq!(metrics.round_trips(), expected.len() as u64);
            }
            let driver = FakeDriver::new([]);
            let trace = driver.trace.clone();
            let metrics = DbMetrics::default();
            assert!(matches!(
                drive(
                    driver,
                    metrics.clone(),
                    Arc::new(Control::default()),
                    Arc::new(AtomicBool::new(false)),
                    no_wait_policy(),
                    |_| async { Err::<(), _>(TxError::Abort("lease_lost".into())) }
                )
                .await,
                Err(AdmissionError::Storage(TxError::Abort(_)))
            ));
            assert_eq!(*trace.lock().unwrap(), vec!["begin", "rollback"]);
            assert_eq!(
                (metrics.begins(), metrics.commits(), metrics.rollbacks()),
                (1, 0, 1)
            );
            let mut driver = FakeDriver::new([]);
            driver.begin_error = true;
            let trace = driver.trace.clone();
            let metrics = DbMetrics::default();
            let called = Arc::new(AtomicBool::new(false));
            let body_called = called.clone();
            assert!(drive(
                driver,
                metrics.clone(),
                Arc::new(Control::default()),
                Arc::new(AtomicBool::new(false)),
                no_wait_policy(),
                move |_| {
                    body_called.store(true, Ordering::Release);
                    async { Ok(()) }
                }
            )
            .await
            .is_err());
            assert!(!called.load(Ordering::Acquire));
            assert_eq!(*trace.lock().unwrap(), vec!["begin"]);
            assert_eq!(metrics.round_trips(), 1);
            let driver = FakeDriver::new([true; 6]);
            let trace = driver.trace.clone();
            let metrics = DbMetrics::default();
            assert!(matches!(
                drive(
                    driver,
                    metrics.clone(),
                    Arc::new(Control::default()),
                    Arc::new(AtomicBool::new(false)),
                    no_wait_policy(),
                    |_| async { Ok(()) }
                )
                .await,
                Err(AdmissionError::Storage(TxError::Contention { attempts: 6 }))
            ));
            assert_eq!(trace.lock().unwrap().len(), 12);
            assert_eq!(
                (
                    metrics.begins(),
                    metrics.commits(),
                    metrics.rollbacks(),
                    metrics.conflict_retries()
                ),
                (6, 6, 0, 5)
            );
        }
        #[tokio::test]
        async fn cancellation_during_begin_waits_for_resolution_then_rolls_back() {
            let gate = Arc::new(Gate::default());
            let mut driver = FakeDriver::new([]);
            driver.begin_gate = Some(gate.clone());
            let trace = driver.trace.clone();
            let metrics = DbMetrics::default();
            let control = Arc::new(Control::default());
            let called = Arc::new(AtomicBool::new(false));
            let body_called = called.clone();
            let task = tokio::spawn(drive(
                driver,
                metrics.clone(),
                control.clone(),
                Arc::new(AtomicBool::new(false)),
                no_wait_policy(),
                move |_| {
                    body_called.store(true, Ordering::Release);
                    async { Ok(()) }
                },
            ));
            barrier(&gate.entered).await;
            control.cancel();
            assert_eq!(*trace.lock().unwrap(), vec!["begin"]);
            gate.release.notify_one();
            assert!(task.await.unwrap().is_err());
            assert!(!called.load(Ordering::Acquire));
            assert_eq!(*trace.lock().unwrap(), vec!["begin", "rollback"]);
            assert_eq!(metrics.commits(), 0);
        }
        #[tokio::test]
        async fn cancellation_during_body_rolls_back_but_inflight_commit_resolves() {
            let gate = Arc::new(Gate::default());
            let driver = FakeDriver::new([]);
            let trace = driver.trace.clone();
            let control = Arc::new(Control::default());
            let body_gate = gate.clone();
            let task = tokio::spawn(drive(
                driver,
                DbMetrics::default(),
                control.clone(),
                Arc::new(AtomicBool::new(false)),
                no_wait_policy(),
                move |_| {
                    let gate = body_gate.clone();
                    async move {
                        gate.pass().await;
                        Ok(())
                    }
                },
            ));
            barrier(&gate.entered).await;
            control.cancel();
            gate.release.notify_one();
            assert!(task.await.unwrap().is_err());
            assert_eq!(*trace.lock().unwrap(), vec!["begin", "rollback"]);
            let gate = Arc::new(Gate::default());
            let mut driver = FakeDriver::new([]);
            driver.commit_gate = Some(gate.clone());
            let trace = driver.trace.clone();
            let metrics = DbMetrics::default();
            let control = Arc::new(Control::default());
            let task = tokio::spawn(drive(
                driver,
                metrics.clone(),
                control.clone(),
                Arc::new(AtomicBool::new(false)),
                no_wait_policy(),
                |_| async { Ok(17) },
            ));
            barrier(&gate.entered).await;
            control.cancel();
            assert!(!task.is_finished());
            gate.release.notify_one();
            assert_eq!(task.await.unwrap().unwrap().value, 17);
            assert_eq!(*trace.lock().unwrap(), vec!["begin", "commit"]);
            assert_eq!(metrics.rollbacks(), 0);
        }
        #[tokio::test]
        async fn failed_cancel_preserves_both_causes_and_prevents_retry() {
            let mut driver = FakeDriver::new([]);
            driver.cancel_error = true;
            let trace = driver.trace.clone();
            let metrics = DbMetrics::default();
            let closing = Arc::new(AtomicBool::new(false));
            let result = drive(
                driver,
                metrics.clone(),
                Arc::new(Control::default()),
                closing.clone(),
                no_wait_policy(),
                |_| async { Err::<(), _>(conflict()) },
            )
            .await;
            let AdmissionError::CleanupUncertain { operation, cleanup } = result.unwrap_err()
            else {
                panic!("cleanup failure must stay distinct")
            };
            assert!(operation.is_conflict());
            assert!(
                matches!(*cleanup,TxError::Abort(ref code) if code=="controlled_cleanup_failure")
            );
            assert_eq!(*trace.lock().unwrap(), vec!["begin", "rollback"]);
            assert!(closing.load(Ordering::Acquire));
            assert_eq!(metrics.conflict_retries(), 0);
        }
        #[cfg(feature = "embedded")]
        struct BarrierDriver {
            sdk: SdkDriver,
            begin_reply: Option<Arc<Gate>>,
            commit_reply: Option<Arc<Gate>>,
            cancelled: Arc<Notify>,
            cancel_error: bool,
        }
        #[cfg(feature = "embedded")]
        impl Driver for BarrierDriver {
            type Handle = Tx;
            async fn begin(&self) -> Result<Tx, TxError> {
                let tx = self.sdk.begin().await?;
                if let Some(gate) = &self.begin_reply {
                    gate.pass().await;
                }
                Ok(tx)
            }
            async fn commit(&self, tx: Tx) -> Result<(), TxError> {
                self.sdk.commit(tx).await?;
                if let Some(gate) = &self.commit_reply {
                    gate.pass().await;
                }
                Ok(())
            }
            async fn cancel(&self, tx: Tx) -> Result<(), TxError> {
                let result = self.sdk.cancel(tx).await;
                self.cancelled.notify_one();
                result?;
                if self.cancel_error {
                    Err(TxError::Abort("controlled_cleanup_failure".into()))
                } else {
                    Ok(())
                }
            }
        }
        #[cfg(feature = "embedded")]
        async fn probe_repo() -> (tempfile::TempDir, Repository) {
            let directory = tempfile::tempdir().unwrap();
            let repo = Repository::connect(
                &crate::storage::open::Target::Embedded {
                    path: directory.path().join("storage"),
                },
                "argos",
                "classifier_owned_transaction",
            )
            .await
            .unwrap();
            repo.db().query("DEFINE TABLE admission_cleanup_probe SCHEMAFULL;DEFINE FIELD value ON admission_cleanup_probe TYPE string;").await.unwrap().check().unwrap();
            (directory, repo)
        }
        #[cfg(feature = "embedded")]
        async fn probe_write(tx: Tx, body_gate: Option<Arc<Gate>>) -> Result<(), TxError> {
            tx_query(
                &tx,
                "UPSERT admission_cleanup_probe:marker SET value='owned' RETURN NONE;",
            )
            .await?
            .check()?;
            if let Some(gate) = body_gate {
                gate.pass().await;
            }
            Ok(())
        }
        #[cfg(feature = "embedded")]
        async fn probe_value(repo: &Repository) -> Option<String> {
            repo.db()
                .query("SELECT VALUE value FROM ONLY admission_cleanup_probe:marker")
                .await
                .unwrap()
                .check()
                .unwrap()
                .take(0)
                .unwrap()
        }
        #[cfg(feature = "embedded")]
        #[tokio::test]
        async fn aborting_awaiter_preserves_owner_handle_and_real_rollback_at_begin_and_body() {
            for phase in ["begin", "body"] {
                let (_directory, repo) = probe_repo().await;
                let owner = AdmissionOwner::new(repo.clone());
                let metrics = owner.metrics();
                let gate = Arc::new(Gate::default());
                let cancelled = Arc::new(Notify::new());
                let driver = BarrierDriver {
                    sdk: SdkDriver { repo: repo.clone() },
                    begin_reply: (phase == "begin").then(|| gate.clone()),
                    commit_reply: None,
                    cancelled: cancelled.clone(),
                    cancel_error: false,
                };
                let caller_owner = owner.clone();
                let body_gate = (phase == "body").then(|| gate.clone());
                let caller = tokio::spawn(async move {
                    owned_tx_with_driver(&caller_owner, no_wait_policy(), driver, move |tx| {
                        let gate = body_gate.clone();
                        async move { probe_write(tx, gate).await }
                    })
                    .await
                });
                barrier(&gate.entered).await;
                caller.abort();
                assert!(caller.await.unwrap_err().is_cancelled());
                // Poll and drop a join future too: it must leave the retained
                // JoinHandle available under the owner for the next join.
                let mut join = Box::pin(owner.join());
                assert!(futures_util::poll!(join.as_mut()).is_pending());
                drop(join);
                gate.release.notify_one();
                owner.join().await.unwrap();
                barrier(&cancelled).await;
                assert_eq!(probe_value(&repo).await, None);
                assert_eq!(
                    (metrics.begins(), metrics.commits(), metrics.rollbacks()),
                    (1, 0, 1)
                );
                owned_tx(&owner, no_wait_policy(), |tx| async move {
                    probe_write(tx, None).await
                })
                .await
                .unwrap();
                assert_eq!(probe_value(&repo).await.as_deref(), Some("owned"));
                owner.shutdown().await.unwrap();
            }
        }
        #[cfg(feature = "embedded")]
        #[tokio::test]
        async fn dropped_awaiter_after_actual_commit_awaits_durable_outcome() {
            let (_directory, repo) = probe_repo().await;
            let owner = AdmissionOwner::new(repo.clone());
            let metrics = owner.metrics();
            let gate = Arc::new(Gate::default());
            let driver = BarrierDriver {
                sdk: SdkDriver { repo: repo.clone() },
                begin_reply: None,
                commit_reply: Some(gate.clone()),
                cancelled: Arc::new(Notify::new()),
                cancel_error: false,
            };
            let caller_owner = owner.clone();
            let caller = tokio::spawn(async move {
                owned_tx_with_driver(&caller_owner, no_wait_policy(), driver, |tx| async move {
                    probe_write(tx, None).await
                })
                .await
            });
            barrier(&gate.entered).await;
            assert_eq!(probe_value(&repo).await.as_deref(), Some("owned"));
            caller.abort();
            assert!(caller.await.unwrap_err().is_cancelled());
            gate.release.notify_one();
            owner.join().await.unwrap();
            assert_eq!(
                (metrics.begins(), metrics.commits(), metrics.rollbacks()),
                (1, 1, 0)
            );
            assert_eq!(probe_value(&repo).await.as_deref(), Some("owned"));
            owner.shutdown().await.unwrap();
        }
        #[cfg(feature = "embedded")]
        #[tokio::test]
        async fn final_owner_drop_requests_cleanup_without_aborting_worker() {
            let (_directory, repo) = probe_repo().await;
            let owner = AdmissionOwner::new(repo.clone());
            let metrics = owner.metrics();
            let gate = Arc::new(Gate::default());
            let cancelled = Arc::new(Notify::new());
            let driver = BarrierDriver {
                sdk: SdkDriver { repo: repo.clone() },
                begin_reply: None,
                commit_reply: None,
                cancelled: cancelled.clone(),
                cancel_error: false,
            };
            let caller_owner = owner.clone();
            let body_gate = gate.clone();
            let caller = tokio::spawn(async move {
                owned_tx_with_driver(&caller_owner, no_wait_policy(), driver, move |tx| {
                    let gate = body_gate.clone();
                    async move { probe_write(tx, Some(gate)).await }
                })
                .await
            });
            barrier(&gate.entered).await;
            caller.abort();
            assert!(caller.await.unwrap_err().is_cancelled());
            drop(owner);
            gate.release.notify_one();
            barrier(&cancelled).await;
            assert_eq!(probe_value(&repo).await, None);
            assert_eq!(
                (metrics.begins(), metrics.commits(), metrics.rollbacks()),
                (1, 0, 1)
            );
        }
        #[cfg(feature = "embedded")]
        #[tokio::test]
        async fn cleanup_failure_closes_owner_and_remains_visible_after_receiver_loss() {
            let (_directory, repo) = probe_repo().await;
            let owner = AdmissionOwner::new(repo.clone());
            let metrics = owner.metrics();
            let gate = Arc::new(Gate::default());
            let driver = BarrierDriver {
                sdk: SdkDriver { repo: repo.clone() },
                begin_reply: Some(gate.clone()),
                commit_reply: None,
                cancelled: Arc::new(Notify::new()),
                cancel_error: true,
            };
            let caller_owner = owner.clone();
            let caller = tokio::spawn(async move {
                owned_tx_with_driver(&caller_owner, no_wait_policy(), driver, |_| async {
                    Ok(())
                })
                .await
            });
            barrier(&gate.entered).await;
            caller.abort();
            assert!(caller.await.unwrap_err().is_cancelled());
            gate.release.notify_one();
            assert!(matches!(
                owner.join().await,
                Err(AdmissionError::CleanupUncertain { .. })
            ));
            let before = metrics.round_trips();
            assert!(matches!(
                owned_tx(&owner, no_wait_policy(), |_| async { Ok(()) }).await,
                Err(AdmissionError::CleanupUncertain { .. })
            ));
            assert_eq!(metrics.round_trips(), before);
            assert_eq!(
                (
                    metrics.begins(),
                    metrics.commits(),
                    metrics.rollbacks(),
                    metrics.conflict_retries()
                ),
                (1, 0, 1, 0)
            );
            assert!(matches!(
                owner.shutdown().await,
                Err(AdmissionError::CleanupUncertain { .. })
            ));
        }
    }
}

/// Canonical retained-corpus scope before any domain filter. Receipt-only new
/// rejections have never been retained and are outside this denominator.
#[derive(Debug, Clone, Default)]
pub(crate) struct CoverageFilter {
    pub run: Option<String>,
    pub seen_day: Option<String>,
    pub run_day: Option<String>,
    pub source: Option<String>,
    pub text: Option<String>,
    pub indexed_text: Option<String>,
    pub cutoff: Option<String>,
    pub upper: Option<OccurrenceKey>,
    pub empty: bool,
}
impl CoverageFilter {
    pub(crate) fn predicate(&self) -> String {
        let mut sql = String::from(if self.empty { "false" } else { "true" });
        if self.cutoff.is_some() {
            sql.push_str(" AND seen_at<=$scope_cutoff");
        }
        if self.upper.is_some() {
            sql.push_str(" AND (run_id<$scope_upper_run OR (run_id=$scope_upper_run AND article_id<=$scope_upper_article))");
        }
        if self.run.is_some() {
            sql.push_str(" AND run_id=$scope_run");
        }
        if self.seen_day.is_some() {
            sql.push_str(" AND seen_at >= $scope_seen AND seen_at < $scope_seen_end");
        }
        if self.run_day.is_some() {
            sql.push_str(" AND run_id IN (SELECT VALUE record::id(id) FROM atlas_run WHERE started_at >= $scope_started AND started_at < $scope_started_end)");
        }
        if self.source.is_some() {
            sql.push_str(" AND source_domain=$scope_source");
        }
        if self.text.is_some() {
            sql.push_str(" AND string::contains(string::lowercase(string::concat(title,' ',description,' ',source_name,' ',source_domain,' ',url)), $scope_text)");
        }
        if self.indexed_text.is_some() {
            sql.push_str(" AND (title @1@ $scope_query OR description @2@ $scope_query)");
        }
        sql
    }
}

/// Coverage of the retained canonical corpus, optionally scoped to an exact
/// run and/or the occurrence's seen_at day. This does not hydrate receipts or
/// count discarded candidates that never entered canonical retention.
pub async fn coverage(
    repo: &Repository,
    run: Option<&str>,
    day: Option<&str>,
) -> Result<TaxonomyCoverage, AdmissionError> {
    coverage_filtered(
        repo,
        &CoverageFilter {
            run: run.map(str::to_owned),
            seen_day: day.map(str::to_owned),
            ..CoverageFilter::default()
        },
    )
    .await
}

pub(crate) async fn coverage_filtered(
    repo: &Repository,
    filter: &CoverageFilter,
) -> Result<TaxonomyCoverage, AdmissionError> {
    coverage_filtered_read(CoverageReader::Db(repo.db()), filter).await
}
enum CoverageReader<'a> {
    Db(&'a surrealdb::Surreal<surrealdb::engine::any::Any>),
    Tx(&'a super::tx::Tx),
}
impl CoverageReader<'_> {
    fn query(&self, sql: String) -> MeasuredQuery<'_> {
        match self {
            Self::Db(db) => db_query(db, sql),
            Self::Tx(tx) => tx_query(tx, sql),
        }
    }
}
async fn coverage_filtered_read(
    reader: CoverageReader<'_>,
    filter: &CoverageFilter,
) -> Result<TaxonomyCoverage, AdmissionError> {
    let valid_day =
        |day: &str| day.len() == 10 && chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d").is_ok();
    if filter
        .run
        .as_ref()
        .is_some_and(|run| !crate::intel_service::valid_id(run))
        || filter.seen_day.as_ref().is_some_and(|day| !valid_day(day))
        || filter.run_day.as_ref().is_some_and(|day| !valid_day(day))
        || filter
            .source
            .as_ref()
            .is_some_and(|source| source.len() > 253)
        || filter.text.as_ref().is_some_and(|text| text.len() > 256)
        || filter
            .indexed_text
            .as_ref()
            .is_some_and(|text| text.len() > 256)
        || filter.cutoff.as_ref().is_some_and(|cutoff| {
            cutoff.len() > 64 || DateTime::parse_from_rfc3339(cutoff).is_err()
        })
        || filter.upper.as_ref().is_some_and(|key| {
            !crate::intel_service::valid_id(&key.run_id)
                || !crate::intel_service::valid_id(&key.article_id)
        })
    {
        return Err(AdmissionError::InvalidPage {
            code: "invalid_coverage_scope".into(),
        });
    }
    let scope = filter.predicate();
    let admitted = super::atlas::CURRENT_ADMISSION_FILTER;
    let rejected="taxonomy_version=$taxonomy AND publication_version=$publication AND admission_state='rejected' AND classification_receipt_key!='' AND record::exists(type::record('atlas_classification_receipt',classification_receipt_key)) AND (SELECT VALUE receipt_key FROM ONLY type::record('atlas_classification_projection',record::id($parent.id)) WHERE admission_state='rejected' AND taxonomy_version=$taxonomy AND publication_version=$publication)=classification_receipt_key";
    let legacy="(taxonomy_version!=$taxonomy OR publication_version!=$publication OR admission_state='legacy')";
    let missing = "(string::trim(title)='' AND string::trim(description)='')";
    let conflicted=format!("taxonomy_version=$taxonomy AND publication_version=$publication AND !({admitted}) AND !({rejected})");
    let mut sql = String::new();
    for predicate in ["true", admitted, rejected, legacy, missing, &conflicted] {
        sql.push_str(&format!("SELECT count() AS total FROM atlas_article WHERE ({scope}) AND ({predicate}) GROUP ALL;"));
    }
    let mut query = reader
        .query(sql)
        .bind(("taxonomy", crate::atlas_taxonomy::TAXONOMY_VERSION))
        .bind(("publication", PUBLICATION_VERSION));
    if let Some(cutoff) = &filter.cutoff {
        query = query.bind(("scope_cutoff", cutoff.clone()));
    }
    if let Some(upper) = &filter.upper {
        query = query
            .bind(("scope_upper_run", upper.run_id.clone()))
            .bind(("scope_upper_article", upper.article_id.clone()));
    }
    if let Some(run) = &filter.run {
        query = query.bind(("scope_run", run.clone()));
    }
    if let Some(day) = &filter.seen_day {
        query = query
            .bind(("scope_seen", day.clone()))
            .bind(("scope_seen_end", format!("{day}~")));
    }
    if let Some(day) = &filter.run_day {
        query = query
            .bind(("scope_started", day.clone()))
            .bind(("scope_started_end", format!("{day}~")));
    }
    if let Some(source) = &filter.source {
        query = query.bind(("scope_source", source.clone()));
    }
    if let Some(text) = &filter.text {
        query = query.bind(("scope_text", text.clone()));
    }
    if let Some(text) = &filter.indexed_text {
        query = query.bind(("scope_query", text.clone()));
    }
    let mut response = query
        .await
        .map_err(TxError::from)?
        .check()
        .map_err(TxError::from)?;
    let mut counts = [0u64; 6];
    for (index, count) in counts.iter_mut().enumerate() {
        let rows: Vec<AggregateCount> = response.take(index).map_err(TxError::from)?;
        if rows.len() > 1 {
            return Err(AdmissionError::InvalidPage {
                code: "unexpected_coverage_aggregate".into(),
            });
        }
        *count = rows
            .first()
            .map(|row| u64::try_from(row.total))
            .transpose()
            .map_err(|_| AdmissionError::InvalidPage {
                code: "negative_coverage_aggregate".into(),
            })?
            .unwrap_or(0);
    }
    let scope_id = crate::classifier::artifact::sha256_hex(format!("{:?}", filter).as_bytes());
    let coverage = TaxonomyCoverage {
        scope: format!("retained-canonical:{scope_id}"),
        retained_occurrences: counts[0],
        current_admitted: counts[1],
        current_rejected: counts[2],
        legacy_unmigrated: counts[3],
        missing_original_metadata: counts[4],
        conflicted: counts[5],
        taxonomy_version: crate::atlas_taxonomy::TAXONOMY_VERSION.into(),
    };
    Ok(coverage)
}

/// The single mutation body shared by foreground and historical maintenance.
/// Caller-specific lease/source checks run first in the same transaction.
fn publication_sql(index: usize) -> String {
    format!(
        r#"                IF $admitted{index} {{
                    UPSERT type::record('atlas_article',$key{index}) MERGE $article{index} RETURN NONE;
                }} ELSE {{UPDATE type::record('atlas_article',$key{index}) SET admission_state='rejected',classification_receipt_key=$receipt_key{index},taxonomy_version=$taxonomy,publication_version=$publication,input_revision=$revision{index},source_revision=$source{index} RETURN NONE;}};
                IF $metadata{index}!='' {{UPSERT type::record('atlas_article_metadata',$key{index}) SET run_id=$occurrence_run{index},article_id=$article_id{index},input_hash=$metadata_hash{index},entity_state=$entity{index},reliability_state=$reliability{index},classification_status=IF $admitted{index} {{'classified'}} ELSE {{'rejected'}},metadata_json=$metadata{index} RETURN NONE;}};
                CREATE type::record('atlas_classification_receipt',$receipt_key{index}) CONTENT $receipt_row{index} RETURN NONE;
                UPSERT type::record('atlas_classification_projection',$key{index}) SET run_id=$occurrence_run{index},article_id=$article_id{index},receipt_key=$receipt_key{index},taxonomy_version=$taxonomy,admission_state=IF $admitted{index} {{'admitted'}} ELSE {{'rejected'}},publication_version=$publication,input_revision=$revision{index},source_revision=$source{index},updated_at=<datetime>$now RETURN NONE;
                FOR $stage IN ['metadata','actors','related','reconcile'] {{UPSERT type::record('atlas_classification_derivative',[$receipt_key{index},$stage]) SET run_id=$occurrence_run{index},article_id=$article_id{index},stage=$stage,input_revision=$revision{index},receipt_key=$receipt_key{index},state=IF $admitted{index} {{'pending'}} ELSE {{'complete'}},output_revision=IF $admitted{index} {{''}} ELSE {{string::concat('rejected:',$receipt_key{index})}},disposition_json=IF $admitted{index} {{''}} ELSE {{'{{"kind":"policy_ineligible","reason":"current_admission_rejected"}}'}},checkpoint_json='',owner='',epoch=0,lease_until=NONE,last_error='',updated_at=<datetime>$now RETURN NONE;}};
"#
    )
}

/// Immutable terminal identity passed only inside the owned stop transaction.
/// Capture counters are supplied by the installed enqueue hook's durable
/// run-scoped headers, never by a foreground payload scan.
#[derive(Debug, Clone)]
pub struct TerminalCycle {
    pub run_id: String,
    pub state: String,
    pub revision: u64,
    pub identity: String,
    pub snapshot: Option<crate::classifier::ModelSnapshot>,
    pub manifest_sha256: Option<String>,
}
#[derive(Debug, Clone, Default)]
pub struct TerminalEffect {
    pub observation_watermark: u64,
    pub calibration_job_id: Option<String>,
}
impl TerminalEffect {
    pub fn new(watermark: u64, job_id: Option<String>) -> Self {
        Self {
            observation_watermark: watermark,
            calibration_job_id: job_id,
        }
    }
}
pub type TerminalHook = Arc<
    dyn Fn(
            super::tx::Tx,
            TerminalCycle,
            DateTime<Utc>,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<TerminalEffect, TxError>> + Send>,
        > + Send
        + Sync,
>;

/// Explicit provider provenance retained alongside staged input. Legacy callers
/// use `None`/empty defaults; this never changes classifier text features.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StageProvenance {
    schema_version: String,
    language: Option<String>,
    observation_ids: Vec<String>,
}
impl Default for StageProvenance {
    fn default() -> Self {
        Self {
            schema_version: "atlas-staged-provenance-v1".into(),
            language: None,
            observation_ids: vec![],
        }
    }
}
impl StageProvenance {
    /// IDs are bounded durable observation references, not labels or features.
    pub fn new(
        language: Option<String>,
        mut observation_ids: Vec<String>,
    ) -> Result<Self, AdmissionError> {
        if language.as_ref().is_some_and(|value| {
            value.len() > 128 || value.is_empty() || value.chars().any(char::is_control)
        }) || observation_ids.len() > 64
            || observation_ids
                .iter()
                .any(|id| id.is_empty() || id.len() > 128 || id.chars().any(char::is_control))
        {
            return Err(AdmissionError::InvalidPage {
                code: "invalid_stage_provenance".into(),
            });
        }
        observation_ids.sort();
        observation_ids.dedup();
        Ok(Self {
            schema_version: "atlas-staged-provenance-v1".into(),
            language,
            observation_ids,
        })
    }
    fn validate(&self) -> Result<(), AdmissionError> {
        let canonical = Self::new(self.language.clone(), self.observation_ids.clone())?;
        if self != &canonical {
            return Err(AdmissionError::InvalidPage {
                code: "noncanonical_stage_provenance".into(),
            });
        }
        Ok(())
    }
    fn from_wire(wire: Option<&str>) -> Result<Self, TxError> {
        let wire =
            wire.ok_or_else(|| TxError::Abort("stage_provenance_exceeds_byte_bound".into()))?;
        let value = match Some(wire).filter(|wire| !wire.is_empty()) {
            Some(wire) => {
                let bytes = decode_wire_json(wire, MAX_PROVENANCE_BYTES)
                    .map_err(|_| TxError::Abort("stage_provenance_invalid".into()))?;
                serde_json::from_slice(&bytes)
                    .map_err(|_| TxError::Abort("stage_provenance_invalid".into()))?
            }
            None => Self::default(),
        };
        value
            .validate()
            .map_err(|_| TxError::Abort("stage_provenance_invalid".into()))?;
        Ok(value)
    }
    pub fn occurrence(&self, article: &AtlasArticleRow) -> OccurrenceProvenance {
        OccurrenceProvenance {
            provider: article.provider.clone(),
            url: article.url.clone(),
            source_domain: article.source_domain.clone(),
            source_name: article.source_name.clone(),
            published_at: article.published_at.clone(),
            seen_at: article.seen_at.clone(),
            language: self.language.clone(),
            observation_ids: self.observation_ids.clone(),
        }
    }
}

/// A provenance-only change obtains a fresh guarded input revision and revokes
/// prior item claims, while retaining the title/description feature hash.
pub async fn stage_article_with_provenance(
    db_owner: &AdmissionOwner,
    run: &RunClaim,
    row: &AtlasArticleRow,
    source_revision: &str,
    provenance: &StageProvenance,
    now: DateTime<Utc>,
) -> Result<bool, AdmissionError> {
    let started = std::time::Instant::now();
    db_owner.ensure_open()?;
    if row.run_id != run.run_id {
        return Err(AdmissionError::InvalidRunState {
            code: "stage_run_mismatch".into(),
        });
    }
    let input = prepare_stage(
        Some(run),
        row,
        source_revision,
        crate::atlas_taxonomy::TAXONOMY_VERSION,
        provenance,
        now,
    )?;
    let committed = foreground_tx(db_owner, RetryPolicy::default(), move |tx| {
        let mut input = input.clone();
        async move {
            input.now = lease_time(input.now, started)?;
            stage_in(&tx, input).await
        }
    })
    .await?;
    Ok(committed.value.ok_or(AdmissionError::LeaseLost)?.changed)
}

struct PreparedPublication {
    article: serde_json::Value,
    metadata_json: String,
    metadata_hash: String,
    entity_state: String,
    reliability_state: String,
}
/// Preserves historical native annotations for rejection. Only an admitted
/// source replacement resets annotations whose original text hash changed.
fn prepare_publication(
    mut article: AtlasArticleRow,
    receipt: &StoredReceipt,
    latest: Option<ArticleMetadata>,
) -> Result<PreparedPublication, TxError> {
    let mut row = serde_json::Value::Null;
    let admitted = receipt.receipt.is_admitted();
    let mut metadata = if admitted {
        Some(
            latest
                .filter(|metadata| {
                    metadata.input_hash == crate::atlas_metadata::input_hash(&article)
                })
                .unwrap_or_else(|| ArticleMetadata::pending(&article)),
        )
    } else {
        latest
    };
    if receipt.receipt.is_admitted() {
        let pair = &receipt.receipt.selected_domains;
        article.categories = pair
            .iter()
            .map(|domain| domain.as_str().to_owned())
            .collect();
        article.category = article.categories[0].clone();
        article.admitted_at = receipt.receipt.evaluated_at.clone();
        row = serde_json::to_value(&article)
            .map_err(|_| TxError::Abort("admission_article_json".into()))?;
        let object = row
            .as_object_mut()
            .ok_or_else(|| TxError::Abort("admission_article_object".into()))?;
        object.remove("id");
        object.insert("article_id".into(), article.id.into());
        object.insert(
            "taxonomy_version".into(),
            receipt.key.taxonomy_version.clone().into(),
        );
        object.insert("admission_state".into(), "admitted".into());
        object.insert(
            "classification_receipt_key".into(),
            receipt.key.record_id().into(),
        );
        object.insert("publication_version".into(), PUBLICATION_VERSION.into());
        object.insert("input_revision".into(), receipt.key.input_revision.into());
        object.insert(
            "source_revision".into(),
            receipt.key.source_revision.clone().into(),
        );
    }
    if let Some(metadata) = &mut metadata {
        metadata.classification_status = if admitted { "classified" } else { "rejected" }.into();
        metadata.classification_receipt = Some(
            serde_json::to_value(receipt)
                .map_err(|_| TxError::Abort("admission_metadata_receipt_json".into()))?,
        );
    }
    let metadata_json = metadata
        .as_ref()
        .map(|metadata| {
            bounded_json(metadata, MAX_RESPONSE_BYTES, "classification_metadata")
                .map_err(|_| TxError::Abort("classification_metadata_too_large".into()))
        })
        .transpose()?
        .unwrap_or_default();
    Ok(PreparedPublication {
        article: row,
        metadata_hash: metadata
            .as_ref()
            .map(|metadata| metadata.input_hash.clone())
            .unwrap_or_default(),
        entity_state: metadata
            .as_ref()
            .map(|metadata| metadata.entity_state.as_str().to_owned())
            .unwrap_or_default(),
        reliability_state: metadata
            .as_ref()
            .map(|metadata| metadata.reliability_state.as_str().to_owned())
            .unwrap_or_default(),
        metadata_json,
    })
}
fn receipt_row(receipt: &StoredReceipt, json: &str, provenance: &str) -> serde_json::Value {
    serde_json::json!({"run_id":receipt.key.occurrence.run_id,"article_id":receipt.key.occurrence.article_id,"input_revision":receipt.key.input_revision,"source_revision":receipt.key.source_revision,"classifier_input_hash":receipt.key.classifier_input_hash,"model_id":receipt.key.model_id,"artifact_sha256":receipt.key.artifact_sha256,"manifest_sha256":receipt.key.manifest_sha256,"taxonomy_version":receipt.key.taxonomy_version,"preprocessing_version":receipt.key.preprocessing_version,"policy_version":receipt.key.policy_version,"calibration_version":receipt.key.calibration_version,"receipt_schema_version":receipt.key.receipt_schema_version,"publication_version":PUBLICATION_VERSION,"receipt_json":json,"receipt_sha256":crate::classifier::artifact::sha256_hex(json.as_bytes()),"provenance_json":provenance,"evaluated_at":receipt.receipt.evaluated_at})
}

/// A short-lived, repository-local foreground request visible before runtime
/// pinning. It is not a run lease and cannot spend provider quota or admit work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForegroundIntent {
    owner: String,
    epoch: u64,
}
impl ForegroundIntent {
    pub fn owner(&self) -> &str {
        &self.owner
    }
    pub fn epoch(&self) -> u64 {
        self.epoch
    }
}
pub const FOREGROUND_INTENT_TTL_SECS: i64 = 5;
pub const FOREGROUND_HANDOFF_SECS: u64 = 10;

pub async fn announce_foreground_intent(
    owner: &AdmissionOwner,
    identity: &str,
    now: DateTime<Utc>,
) -> Result<ForegroundIntent, AdmissionError> {
    if !crate::intel_service::valid_id(identity) {
        return Err(AdmissionError::InvalidPage {
            code: "invalid_foreground_intent_identity".into(),
        });
    }
    let identity = identity.to_owned();
    let key = identity.clone();
    let committed=owned_tx(owner,RetryPolicy::default(),move |tx| {let key=key.clone();async move {
        let mut response=tx_query(&tx,"UPSERT type::record('atlas_classifier_intent',$owner) SET owner=$owner,epoch=(epoch ?? 0)+1,lease_until=<datetime>$until,updated_at=<datetime>$now RETURN VALUE epoch;")
            .bind(("owner",key)).bind(("now",now.to_rfc3339())).bind(("until",(now+Duration::seconds(FOREGROUND_INTENT_TTL_SECS)).to_rfc3339())).await?.check()?;
        let epochs:Vec<i64>=response.take(0)?;
        if epochs.len()!=1 {return Err(TxError::Abort("missing_foreground_intent_epoch".into()));}
        u64::try_from(epochs[0]).map_err(|_|TxError::Abort("negative_foreground_intent_epoch".into()))
    }}).await?;
    Ok(ForegroundIntent {
        owner: identity,
        epoch: committed.value,
    })
}

pub async fn renew_foreground_intent(
    owner: &AdmissionOwner,
    intent: &ForegroundIntent,
    now: DateTime<Utc>,
) -> Result<bool, AdmissionError> {
    let intent = intent.clone();
    let epoch = checked_i64(intent.epoch)?;
    let committed=owned_tx(owner,RetryPolicy::default(),move |tx| {let intent=intent.clone();async move {
        let mut response=tx_query(&tx,"{LET $current=(SELECT * FROM ONLY type::record('atlas_classifier_intent',$owner));IF $current=NONE OR $current.owner!=$owner OR $current.epoch!=$epoch OR $current.lease_until<= <datetime>$now {RETURN false;};UPDATE type::record('atlas_classifier_intent',$owner) SET lease_until=<datetime>$until,updated_at=<datetime>$now RETURN NONE;RETURN true;}")
            .bind(("owner",intent.owner)).bind(("epoch",epoch)).bind(("now",now.to_rfc3339())).bind(("until",(now+Duration::seconds(FOREGROUND_INTENT_TTL_SECS)).to_rfc3339())).await?.check()?;
        response.take::<Option<bool>>(0)?.ok_or_else(||TxError::Abort("missing_intent_renewal_reply".into()))
    }}).await?;
    Ok(committed.value)
}

pub async fn clear_foreground_intent(
    owner: &AdmissionOwner,
    intent: &ForegroundIntent,
) -> Result<(), AdmissionError> {
    let intent = intent.clone();
    let epoch = checked_i64(intent.epoch)?;
    owned_tx(owner,RetryPolicy::default(),move |tx| {let intent=intent.clone();async move {
        tx_query(&tx,"UPDATE type::record('atlas_classifier_intent',$owner) SET lease_until=d'1970-01-01T00:00:00Z' WHERE owner=$owner AND epoch=$epoch RETURN NONE;").bind(("owner",intent.owner)).bind(("epoch",epoch)).await?.check()?;
        Ok(())
    }}).await?;
    Ok(())
}

/// Background drainers poll this at most one second apart and cancel their
/// controlled candidate step before dispatching another chunk. Expired owners
/// are ignored, so a crashed preflight never permanently blocks maintenance.
pub async fn foreground_requested(
    repo: &Repository,
    now: DateTime<Utc>,
) -> Result<bool, AdmissionError> {
    let mut response=db_query(repo.db(),"RETURN count(SELECT id FROM atlas_classifier_intent WHERE lease_until> <datetime>$now LIMIT 1)>0 OR count(SELECT id FROM atlas_classifier_run WHERE state='running' AND lease_until> <datetime>$now LIMIT 1)>0;").bind(("now",now.to_rfc3339())).await.map_err(TxError::from)?.check().map_err(TxError::from)?;
    response
        .take::<Option<bool>>(0)
        .map_err(TxError::from)?
        .ok_or_else(|| AdmissionError::InvalidPage {
            code: "missing_foreground_priority_reply".into(),
        })
}

/// Captures every persisted input component that can invalidate a historical
/// result, including provenance excluded from the stable metadata hash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct RetainedSourceFence {
    pub occurrence: OccurrenceKey,
    pub native_input_revision: u64,
    pub source_hash: String,
    pub classifier_input_hash: String,
    pub provenance: OccurrenceProvenance,
}
impl RetainedSourceFence {
    /// Immutable origin/item identity. Every component is length-framed; a
    /// provenance-only change never reuses a captured origin or job item key.
    pub fn capture_revision(&self) -> String {
        let native = self.native_input_revision.to_string();
        let count = self.provenance.observation_ids.len().to_string();
        let language_kind = if self.provenance.language.is_some() {
            "some"
        } else {
            "none"
        };
        let mut parts = vec![
            "atlas-retained-capture-v1",
            self.occurrence.run_id.as_str(),
            self.occurrence.article_id.as_str(),
            native.as_str(),
            self.source_hash.as_str(),
            self.classifier_input_hash.as_str(),
            self.provenance.provider.as_str(),
            self.provenance.url.as_str(),
            self.provenance.source_domain.as_str(),
            self.provenance.source_name.as_str(),
            self.provenance.published_at.as_str(),
            self.provenance.seen_at.as_str(),
            language_kind,
            self.provenance.language.as_deref().unwrap_or(""),
            count.as_str(),
        ];
        parts.extend(self.provenance.observation_ids.iter().map(String::as_str));
        crate::classifier::artifact::sha256_hex(codec::encode(&parts).as_bytes())
    }
    pub fn new(
        article: &AtlasArticleRow,
        native_input_revision: u64,
        extras: &StageProvenance,
    ) -> Result<Self, AdmissionError> {
        extras.validate()?;
        if native_input_revision > i64::MAX as u64
            || !crate::intel_service::valid_id(&article.run_id)
            || !crate::intel_service::valid_id(&article.id)
        {
            return Err(AdmissionError::InvalidPage {
                code: "invalid_retained_source".into(),
            });
        }
        bounded_json(article, 65536, "original_article")?;
        Ok(Self {
            occurrence: OccurrenceKey::new(&article.run_id, &article.id),
            native_input_revision,
            source_hash: crate::atlas_metadata::input_hash(article),
            classifier_input_hash: crate::classifier::ClassifyInput::new(
                "retained",
                &article.title,
                &article.description,
                None,
            )
            .input_hash,
            provenance: extras.occurrence(article),
        })
    }
}
#[derive(SurrealValue)]
struct RetainedNativeHead {
    revision: i64,
    pending: bool,
    classifier_hash: Option<String>,
    provenance64: Option<String>,
}
/// Bound source headers/provenance are fetched as one SDK batch, never one
/// request per occurrence. Language/observation hints are used only when the
/// receipt describes the exact retained title/description.
pub(crate) async fn retained_source_fences_many_in(
    tx: &super::tx::Tx,
    articles: &[AtlasArticleRow],
) -> Result<std::collections::BTreeMap<OccurrenceKey, RetainedSourceFence>, TxError> {
    retained_source_fences_read_in(tx, articles, false).await
}
/// Read-only conflict audit includes pending canonical heads; it grants no
/// reservation, dispatch or publication authority.
pub(crate) async fn retained_source_fences_for_audit_in(
    tx: &super::tx::Tx,
    articles: &[AtlasArticleRow],
) -> Result<std::collections::BTreeMap<OccurrenceKey, RetainedSourceFence>, TxError> {
    retained_source_fences_read_in(tx, articles, true).await
}
async fn retained_source_fences_read_in(
    tx: &super::tx::Tx,
    articles: &[AtlasArticleRow],
    include_pending: bool,
) -> Result<std::collections::BTreeMap<OccurrenceKey, RetainedSourceFence>, TxError> {
    if articles.len() > 64 {
        return Err(TxError::Abort("retained_source_batch_limit".into()));
    }
    if articles.is_empty() {
        return Ok(std::collections::BTreeMap::new());
    }
    let mut sql = String::new();
    for index in 0..articles.len() {
        sql.push_str(&format!("{{LET $a=(SELECT input_revision,classification_receipt_key,admission_state FROM ONLY $key{index});IF $a=NONE {{RETURN NONE;}};LET $p=(SELECT admission_state FROM ONLY type::record('atlas_classification_projection',$identity{index}));LET $r=IF ($a.classification_receipt_key ?? '')!='' {{(SELECT classifier_input_hash,IF bytes::len(type::bytes(provenance_json))<=16384 {{encoding::base64::encode(type::bytes(provenance_json),true)}} ELSE {{NONE}} AS provenance64 FROM ONLY type::record('atlas_classification_receipt',$a.classification_receipt_key))}} ELSE {{NONE}};RETURN {{revision:($a.input_revision ?? 0),pending:($a.admission_state='pending' OR ($p.admission_state ?? '')='pending' OR record::exists(type::record('atlas_admission',$identity{index}))),classifier_hash:$r.classifier_input_hash,provenance64:$r.provenance64}};}};"));
    }
    let mut query = tx_query(tx, sql);
    for (index, article) in articles.iter().enumerate() {
        let key = OccurrenceKey::new(&article.run_id, &article.id);
        query = query
            .bind((
                format!("key{index}"),
                super::jobs::rid("atlas_article", &key.record_id()),
            ))
            .bind((format!("identity{index}"), key.record_id()));
    }
    let mut response = query.await?.check()?;
    let mut out = std::collections::BTreeMap::new();
    for (index, article) in articles.iter().enumerate() {
        let Some(head) = response.take::<Option<RetainedNativeHead>>(index)? else {
            continue;
        };
        if head.pending && !include_pending {
            continue;
        }
        let input = crate::classifier::ClassifyInput::new(
            "retained",
            &article.title,
            &article.description,
            None,
        );
        let mut provenance = StageProvenance::default().occurrence(article);
        if head.classifier_hash.as_deref() == Some(input.input_hash.as_str()) {
            let encoded = head
                .provenance64
                .ok_or_else(|| TxError::Abort("retained_provenance_missing_or_oversized".into()))?;
            let bytes = decode_wire_json(&encoded, MAX_PROVENANCE_BYTES)
                .map_err(|_| TxError::Abort("retained_provenance_invalid".into()))?;
            let original: OccurrenceProvenance = serde_json::from_slice(&bytes)
                .map_err(|_| TxError::Abort("retained_provenance_invalid".into()))?;
            let extras = StageProvenance::new(original.language, original.observation_ids)
                .map_err(|_| TxError::Abort("retained_provenance_invalid".into()))?;
            provenance = extras.occurrence(article);
        }
        let occurrence = OccurrenceKey::new(&article.run_id, &article.id);
        out.insert(
            occurrence.clone(),
            RetainedSourceFence {
                occurrence,
                native_input_revision: u64::try_from(head.revision)
                    .map_err(|_| TxError::Abort("negative_retained_revision".into()))?,
                source_hash: crate::atlas_metadata::input_hash(article),
                classifier_input_hash: input.input_hash,
                provenance,
            },
        );
    }
    Ok(out)
}

/// Scope authorized by a durable historical reclassification request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum RetainedPublicationScope {
    AllRetained,
    Run { run_id: String },
}
impl RetainedPublicationScope {
    pub fn key(&self) -> String {
        match self {
            Self::AllRetained => "all_retained".into(),
            Self::Run { run_id } => codec::encode(&["run", run_id]),
        }
    }
    fn contains(&self, key: &OccurrenceKey) -> bool {
        match self {
            Self::AllRetained => true,
            Self::Run { run_id } => run_id == &key.run_id,
        }
    }
}

/// A maintenance task is never a foreground RunClaim. Its request and actual
/// task lease are checked under the job guard in every publication transaction.
#[derive(Debug, Clone)]
pub struct MaintenancePublicationFence {
    task: super::jobs::ClaimedTask,
    request_hash: String,
    scope: RetainedPublicationScope,
    pin: crate::classifier::ModelSnapshot,
    manifest_sha256: String,
}
impl MaintenancePublicationFence {
    pub fn new(
        task: super::jobs::ClaimedTask,
        request_hash: String,
        scope: RetainedPublicationScope,
        pin: crate::classifier::ModelSnapshot,
        manifest_sha256: String,
    ) -> Result<Self, AdmissionError> {
        let digest =
            |value: &str| value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit());
        if task.operation != "atlas_admission_reclassify"
            || task.pool != "atlas_admission_maintenance"
            || task.lease_owner.is_empty()
            || task.lease_epoch <= 0
            || !digest(&request_hash)
            || !digest(&manifest_sha256)
            || !digest(&pin.artifact_sha256)
            || pin.taxonomy_version != crate::atlas_taxonomy::TAXONOMY_VERSION
            || pin.preprocessing_version != crate::classifier::PREPROCESSING_VERSION
            || pin.policy_version != crate::classifier::ADMISSION_POLICY_VERSION
            || matches!(&scope,RetainedPublicationScope::Run {run_id} if !crate::intel_service::valid_id(run_id))
        {
            return Err(AdmissionError::InvalidPage {
                code: "invalid_maintenance_publication_fence".into(),
            });
        }
        Ok(Self {
            task,
            request_hash,
            scope,
            pin,
            manifest_sha256,
        })
    }
    pub fn task(&self) -> &super::jobs::ClaimedTask {
        &self.task
    }
    pub fn scope(&self) -> &RetainedPublicationScope {
        &self.scope
    }
    pub fn snapshot(&self) -> &crate::classifier::ModelSnapshot {
        &self.pin
    }
    pub fn manifest_sha256(&self) -> &str {
        &self.manifest_sha256
    }
    /// Reused by U6b's item and derivative/checkpoint writers in the same owned
    /// transaction; this is an authority check, not an in-memory capability.
    pub(crate) async fn owns_in(
        &self,
        tx: &super::tx::Tx,
        now: DateTime<Utc>,
    ) -> Result<bool, TxError> {
        super::jobs::bump(tx, "job_guard", &self.task.root_id).await?;
        let mut response=tx_query(tx,"LET $actual=(SELECT * FROM ONLY $task);RETURN $actual!=NONE AND record::id($actual.job)=$job AND record::id($actual.root)=$root AND $actual.operation=$operation AND $actual.pool=$pool AND $actual.input_hash=$input_hash; RETURN count(SELECT id FROM atlas_reclassification_job WHERE id=type::record('atlas_reclassification_job',$job) AND request_hash=$request AND scope_key=$scope AND pin_json=$pin AND manifest_sha256=$manifest LIMIT 1)=1;")
            .bind(("task",super::jobs::rid("task",&self.task.id))).bind(("job",self.task.job_id.clone())).bind(("root",self.task.root_id.clone())).bind(("operation",self.task.operation.clone())).bind(("pool",self.task.pool.clone())).bind(("input_hash",self.task.input_hash.clone()))
            .bind(("request",self.request_hash.clone())).bind(("scope",self.scope.key())).bind(("pin",serde_json::to_string(&self.pin).map_err(|_|TxError::Abort("maintenance_pin_json".into()))?)).bind(("manifest",self.manifest_sha256.clone())).await?.check()?;
        let task_matches = response.take::<Option<bool>>(1)? == Some(true);
        let request_matches = response.take::<Option<bool>>(2)? == Some(true);
        let Some(actual) = super::jobs::fence(tx, &self.task, &now.to_rfc3339()).await? else {
            return Ok(false);
        };
        Ok(task_matches && request_matches && super::jobs::holds(&actual, &self.task))
    }
    pub(crate) async fn check_in(
        &self,
        tx: &super::tx::Tx,
        now: DateTime<Utc>,
    ) -> Result<bool, TxError> {
        if !self.owns_in(tx, now).await? {
            return Ok(false);
        }
        let mut response=tx_query(tx,"RETURN count(SELECT id FROM atlas_reclassification_job WHERE id=$job AND state INSIDE ['running','reconciling'] AND !cancel_requested AND !pause_requested LIMIT 1)=1;").bind(("job",super::jobs::rid("atlas_reclassification_job",&self.task.job_id))).await?.check()?;
        let permitted = response.take::<Option<bool>>(0)? == Some(true);
        let Some(actual) = super::jobs::fence(tx, &self.task, &now.to_rfc3339()).await? else {
            return Ok(false);
        };
        Ok(permitted && super::jobs::holds(&actual, &self.task) && !actual.cancelled)
    }
}

/// Stable record identity shared with the U6b item repository.
pub fn maintenance_item_key(job: &str, source: &RetainedSourceFence) -> String {
    codec::encode(&[
        job,
        &source.occurrence.run_id,
        &source.occurrence.article_id,
        &source.capture_revision(),
    ])
}

/// Allocates a non-reusable classifier input revision under the same source
/// guard used by foreground staging. The canonical row and RunStats remain
/// unchanged until the eventual conditional publication succeeds.
pub async fn reserve_retained_revisions_in(
    tx: &super::tx::Tx,
    fence: &MaintenancePublicationFence,
    expected: &[RetainedSourceFence],
    now: DateTime<Utc>,
) -> Result<std::collections::BTreeMap<OccurrenceKey, u64>, TxError> {
    if expected.len() > 64
        || expected
            .iter()
            .any(|source| !fence.scope.contains(&source.occurrence))
    {
        return Err(TxError::Abort("retained_reservation_scope".into()));
    }
    let keys: Vec<_> = expected
        .iter()
        .map(|source| source.occurrence.clone())
        .collect();
    if keys.iter().collect::<std::collections::BTreeSet<_>>().len() != keys.len() {
        return Err(TxError::Abort("duplicate_retained_reservation".into()));
    }
    if keys.is_empty() || !fence.check_in(tx, now).await? {
        return Ok(std::collections::BTreeMap::new());
    }
    let mut guards = String::new();
    for index in 0..keys.len() {
        guards.push_str(&format!("UPSERT type::record('atlas_classifier_guard',$guard{index}) SET updated_at=<datetime>$now RETURN NONE;"));
    }
    let mut query = tx_query(tx, guards).bind(("now", now.to_rfc3339()));
    for (index, key) in keys.iter().enumerate() {
        query = query.bind((
            format!("guard{index}"),
            codec::encode(&["source", &key.run_id, &key.article_id]),
        ));
    }
    query.await?.check()?;
    let originals = super::atlas::articles_many_in(tx, &keys).await?;
    let current =
        retained_source_fences_many_in(tx, &originals.into_values().collect::<Vec<_>>()).await?;
    let mut sql = String::new();
    for index in 0..keys.len() {
        sql.push_str(&format!("{{IF !$valid{index} {{RETURN NONE;}};LET $run=(SELECT * FROM ONLY type::record('atlas_classifier_run',$run{index}));IF $run.state='running' AND $run.lease_until> <datetime>$now {{RETURN NONE;}};LET $old=(SELECT input_revision FROM ONLY type::record('atlas_classification_projection',$key{index}));LET $head=(SELECT input_revision FROM ONLY type::record('atlas_classifier_guard',$guard{index}));LET $next=math::max([$old.input_revision ?? 0,$head.input_revision ?? 0,$native{index}])+1;UPDATE type::record('atlas_classifier_guard',$guard{index}) SET input_revision=$next RETURN NONE;RETURN $next;}};"));
    }
    let mut query = tx_query(tx, sql).bind(("now", now.to_rfc3339()));
    for (index, source) in expected.iter().enumerate() {
        let key = &source.occurrence;
        query = query
            .bind((format!("valid{index}"), current.get(key) == Some(source)))
            .bind((
                format!("guard{index}"),
                codec::encode(&["source", &key.run_id, &key.article_id]),
            ))
            .bind((format!("run{index}"), key.run_id.clone()))
            .bind((format!("key{index}"), key.record_id()))
            .bind((
                format!("native{index}"),
                i64::try_from(source.native_input_revision)
                    .map_err(|_| TxError::Abort("retained_revision_overflow".into()))?,
            ));
    }
    let mut response = query.await?.check()?;
    let mut out = std::collections::BTreeMap::new();
    for (index, key) in keys.into_iter().enumerate() {
        if let Some(revision) = response.take::<Option<i64>>(index)? {
            out.insert(
                key,
                u64::try_from(revision)
                    .map_err(|_| TxError::Abort("negative_retained_input_revision".into()))?,
            );
        }
    }
    Ok(out)
}

#[derive(Debug)]
pub struct MaintenanceOutcome {
    pub source: RetainedSourceFence,
    pub classification: crate::atlas_classify::ItemClassification,
}
impl MaintenanceOutcome {
    pub fn new(
        source: RetainedSourceFence,
        classification: crate::atlas_classify::ItemClassification,
    ) -> Self {
        Self {
            source,
            classification,
        }
    }
}
#[derive(Clone)]
struct PreparedMaintenance {
    source: RetainedSourceFence,
    claim: AdmissionClaim,
    receipt: Option<StoredReceipt>,
    json: String,
    provenance: String,
    key: String,
    retry: String,
}
/// Publishes bounded historical outcomes through the same mutation fragment as
/// foreground admission. It never updates an original cycle pin/checkpoint or
/// its classifier work manifest, and never fabricates a foreground claim.
#[derive(Clone)]
pub(crate) struct PreparedMaintenanceBatch {
    prepared: Vec<PreparedMaintenance>,
}
/// Validates and bounds immutable outcomes before opening the write transaction.
pub(crate) fn prepare_maintenance_outcomes(
    fence: &MaintenancePublicationFence,
    items: &[MaintenanceOutcome],
) -> Result<PreparedMaintenanceBatch, AdmissionError> {
    if items.len() > 64 {
        return Err(AdmissionError::InvalidPage {
            code: "maintenance_commit_batch_limit".into(),
        });
    }
    let mut prepared = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for item in items {
        let (claim, receipt, retry) = match &item.classification {
            crate::atlas_classify::ItemClassification::Evaluated { claim, receipt } => {
                (claim, Some((**receipt).clone()), String::new())
            }
            crate::atlas_classify::ItemClassification::Retryable { claim, error } => (
                claim,
                None,
                match error {
                    ClassifierError::Busy => "busy",
                    ClassifierError::Timeout => "timeout",
                    ClassifierError::Cancelled => "cancelled",
                    _ => "classifier_operation_failed",
                }
                .into(),
            ),
        };
        if claim.occurrence != item.source.occurrence
            || !fence.scope.contains(&claim.occurrence)
            || claim.owner != fence.task.lease_owner
            || claim.epoch
                != u64::try_from(fence.task.lease_epoch).map_err(|_| AdmissionError::LeaseLost)?
            || claim.work_seq != 0
            || claim.input_revision == 0
            || claim.input_revision > i64::MAX as u64
            || claim.source_revision != item.source.source_hash
            || claim.classifier_input_hash != item.source.classifier_input_hash
            || !seen.insert(claim.occurrence.clone())
        {
            return Err(AdmissionError::InvalidPage {
                code: "maintenance_claim_identity".into(),
            });
        }
        let (json, provenance, key) = if let Some(receipt) = &receipt {
            receipt.validate()?;
            if receipt.key.occurrence != claim.occurrence
                || receipt.key.input_revision != claim.input_revision
                || receipt.key.source_revision != claim.source_revision
                || receipt.key.classifier_input_hash != claim.classifier_input_hash
                || receipt.provenance != item.source.provenance
                || receipt.key.model_id != fence.pin.model_id
                || receipt.key.artifact_sha256 != fence.pin.artifact_sha256
                || receipt.key.manifest_sha256 != fence.manifest_sha256
                || receipt.key.taxonomy_version != fence.pin.taxonomy_version
                || receipt.key.preprocessing_version != fence.pin.preprocessing_version
                || receipt.key.policy_version != fence.pin.policy_version
            {
                return Err(AdmissionError::InvalidReceipt {
                    code: "maintenance_receipt_identity".into(),
                });
            }
            (
                bounded_json(receipt, MAX_RECEIPT_BYTES, "receipt")?,
                bounded_json(&receipt.provenance, MAX_PROVENANCE_BYTES, "provenance")?,
                receipt.key.record_id(),
            )
        } else {
            (String::new(), String::new(), String::new())
        };
        prepared.push(PreparedMaintenance {
            source: item.source.clone(),
            claim: claim.clone(),
            receipt,
            json,
            provenance,
            key,
            retry,
        });
    }
    Ok(PreparedMaintenanceBatch { prepared })
}
/// The exact shared publisher body, exposed for atomic U6b conflict accounting.
/// No classifier call or independently committed write occurs in this port.
pub(crate) async fn commit_prepared_maintenance_in(
    tx: &super::tx::Tx,
    fence: &MaintenancePublicationFence,
    batch: &PreparedMaintenanceBatch,
    now: DateTime<Utc>,
) -> Result<Vec<ItemCommit>, TxError> {
    let prepared = &batch.prepared;
    if prepared.is_empty() {
        return Ok(Vec::new());
    }
    if !fence.check_in(tx, now).await? {
        return Ok(prepared
            .iter()
            .map(|item| ItemCommit::LeaseLost {
                occurrence: item.claim.occurrence.clone(),
            })
            .collect());
    }
    let keys: Vec<_> = prepared
        .iter()
        .map(|item| item.claim.occurrence.clone())
        .collect();
    let mut guards = String::new();
    for index in 0..keys.len() {
        guards.push_str(&format!("UPSERT type::record('atlas_classifier_guard',$guard{index}) SET updated_at=<datetime>$now RETURN NONE;"));
    }
    let mut query = tx_query(tx, guards).bind(("now", now.to_rfc3339()));
    for (index, key) in keys.iter().enumerate() {
        query = query.bind((
            format!("guard{index}"),
            codec::encode(&["source", &key.run_id, &key.article_id]),
        ));
    }
    query.await?.check()?;
    let mut articles = super::atlas::articles_many_in(tx, &keys).await?;
    let current =
        retained_source_fences_many_in(tx, &articles.values().cloned().collect::<Vec<_>>()).await?;
    let mut metadata = super::atlas_metadata::metadata_many_in(tx, &keys).await?;
    let mut publications = Vec::new();
    let mut metadata_bytes = 0usize;
    for item in prepared {
        let publication = match (
            articles.remove(&item.claim.occurrence),
            item.receipt.as_ref(),
        ) {
            (Some(article), Some(receipt)) => {
                prepare_publication(article, receipt, metadata.remove(&item.claim.occurrence))?
            }
            _ => PreparedPublication {
                article: serde_json::Value::Null,
                metadata_json: String::new(),
                metadata_hash: String::new(),
                entity_state: String::new(),
                reliability_state: String::new(),
            },
        };
        metadata_bytes = metadata_bytes
            .checked_add(publication.metadata_json.len())
            .ok_or_else(|| TxError::Abort("maintenance_metadata_too_large".into()))?;
        if metadata_bytes > 2 * MAX_RESPONSE_BYTES {
            return Err(TxError::Abort("maintenance_metadata_too_large".into()));
        }
        publications.push(publication);
    }
    let mut sql = String::new();
    for index in 0..prepared.len() {
        let publication = publication_sql(index);
        sql.push_str(&format!("{{
            IF $evaluated{index} {{LET $old=(SELECT receipt_json,receipt_sha256,provenance_json FROM ONLY type::record('atlas_classification_receipt',$receipt_key{index}));IF $old!=NONE {{IF $old.receipt_json!=$receipt_json{index} OR $old.receipt_sha256!=$receipt_hash{index} OR $old.provenance_json!=$provenance{index} {{RETURN {{kind:'receipt_conflict',revision:NONE}};}};RETURN {{kind:'already',revision:NONE}};}};}};
            LET $item=(SELECT * FROM ONLY type::record('atlas_reclassification_item',$item_key{index}));
            IF $item=NONE OR $item.disposition!='evaluating' OR $item.owner!=$owner OR $item.epoch!=$epoch OR $item.lease_until=NONE OR $item.lease_until<= <datetime>$now OR $item.input_revision!=$revision{index} OR $item.source_revision!=$source{index} OR $item.original_input_revision!=$native{index} OR $item.original_provenance_json!=$original_provenance{index} {{RETURN {{kind:'lease_lost',revision:NONE}};}};
            LET $run=(SELECT state,lease_until FROM ONLY type::record('atlas_classifier_run',$occurrence_run{index}));
            LET $guard=(SELECT input_revision FROM ONLY type::record('atlas_classifier_guard',$guard{index}));
            IF !$current{index} OR $guard.input_revision!=$revision{index} OR ($run.state='running' AND $run.lease_until> <datetime>$now) {{UPDATE type::record('atlas_reclassification_item',$item_key{index}) SET disposition='conflicted',last_error='source_changed',owner='',lease_until=NONE RETURN NONE;RETURN {{kind:'stale',revision:NONE}};}};
            IF !$evaluated{index} {{UPDATE type::record('atlas_reclassification_item',$item_key{index}) SET disposition='retryable',last_error=$retry{index},owner='',lease_until=NONE RETURN NONE;RETURN {{kind:'retry',revision:NONE}};}};
            {publication}
            UPDATE type::record('atlas_reclassification_item',$item_key{index}) SET disposition=IF $admitted{index} {{'admitted'}} ELSE {{'rejected'}},receipt_key=$receipt_key{index},last_error='',owner='',lease_until=NONE RETURN NONE;
            RETURN {{kind:'committed',revision:NONE}};
        }};"));
    }
    let mut query = tx_query(tx, sql)
        .bind(("owner", fence.task.lease_owner.clone()))
        .bind(("epoch", fence.task.lease_epoch))
        .bind(("now", now.to_rfc3339()))
        .bind(("taxonomy", crate::atlas_taxonomy::TAXONOMY_VERSION))
        .bind(("publication", PUBLICATION_VERSION));
    for (index, (item, publication)) in prepared.iter().zip(&publications).enumerate() {
        let receipt = item.receipt.as_ref();
        let admitted = receipt.is_some_and(|receipt| receipt.receipt.is_admitted());
        query = query
            .bind((format!("evaluated{index}"), receipt.is_some()))
            .bind((
                format!("current{index}"),
                current.get(&item.claim.occurrence) == Some(&item.source),
            ))
            .bind((
                format!("guard{index}"),
                codec::encode(&[
                    "source",
                    &item.claim.occurrence.run_id,
                    &item.claim.occurrence.article_id,
                ]),
            ))
            .bind((
                format!("item_key{index}"),
                maintenance_item_key(&fence.task.job_id, &item.source),
            ))
            .bind((
                format!("native{index}"),
                i64::try_from(item.source.native_input_revision)
                    .map_err(|_| TxError::Abort("retained_revision_overflow".into()))?,
            ))
            .bind((
                format!("original_provenance{index}"),
                serde_json::to_string(&item.source.provenance)
                    .map_err(|_| TxError::Abort("retained_provenance_json".into()))?,
            ))
            .bind((format!("revision{index}"), item.claim.input_revision as i64))
            .bind((format!("source{index}"), item.claim.source_revision.clone()))
            .bind((
                format!("occurrence_run{index}"),
                item.claim.occurrence.run_id.clone(),
            ))
            .bind((format!("key{index}"), item.claim.occurrence.record_id()))
            .bind((
                format!("article_id{index}"),
                item.claim.occurrence.article_id.clone(),
            ))
            .bind((format!("receipt_key{index}"), item.key.clone()))
            .bind((format!("receipt_json{index}"), item.json.clone()))
            .bind((
                format!("receipt_hash{index}"),
                crate::classifier::artifact::sha256_hex(item.json.as_bytes()),
            ))
            .bind((format!("provenance{index}"), item.provenance.clone()))
            .bind((
                format!("receipt_row{index}"),
                receipt
                    .map(|receipt| receipt_row(receipt, &item.json, &item.provenance))
                    .unwrap_or(serde_json::Value::Null),
            ))
            .bind((format!("admitted{index}"), admitted))
            .bind((format!("article{index}"), publication.article.clone()))
            .bind((
                format!("metadata{index}"),
                publication.metadata_json.clone(),
            ))
            .bind((
                format!("metadata_hash{index}"),
                publication.metadata_hash.clone(),
            ))
            .bind((format!("entity{index}"), publication.entity_state.clone()))
            .bind((
                format!("reliability{index}"),
                publication.reliability_state.clone(),
            ))
            .bind((format!("retry{index}"), item.retry.clone()));
    }
    let mut response = query.await?.check()?;
    let mut results = Vec::new();
    for (index, item) in prepared.iter().enumerate() {
        let reply = response
            .take::<Option<CommitReply>>(index)?
            .ok_or_else(|| TxError::Abort("missing_maintenance_publication_reply".into()))?;
        results.push(match reply.kind.as_str() {
            "committed" => ItemCommit::Committed {
                receipt_key: item.key.clone(),
            },
            "already" => ItemCommit::AlreadyCommitted {
                receipt_key: item.key.clone(),
            },
            "stale" => ItemCommit::StaleInput {
                occurrence: item.claim.occurrence.clone(),
                current_revision: None,
            },
            "lease_lost" => ItemCommit::LeaseLost {
                occurrence: item.claim.occurrence.clone(),
            },
            "retry" => {
                if item.retry == "cancelled" {
                    ItemCommit::Cancelled {
                        occurrence: item.claim.occurrence.clone(),
                    }
                } else {
                    ItemCommit::Retryable {
                        occurrence: item.claim.occurrence.clone(),
                        code: item.retry.clone(),
                    }
                }
            }
            "receipt_conflict" => {
                return Err(TxError::Abort(format!(
                    "maintenance_receipt_conflict:{index}"
                )))
            }
            _ => {
                return Err(TxError::Abort(
                    "invalid_maintenance_publication_reply".into(),
                ))
            }
        });
    }
    for (item, result) in prepared.iter().zip(&results) {
        if matches!(
            result,
            ItemCommit::Committed { .. } | ItemCommit::AlreadyCommitted { .. }
        ) {
            let admitted = item
                .receipt
                .as_ref()
                .is_some_and(|receipt| receipt.receipt.is_admitted());
            super::atlas_derivative_status::invalidate_in(
                tx,
                &item.claim.occurrence,
                &item.key,
                admitted,
            )
            .await?;
            crate::atlas_derivative_worker::enqueue_in(tx, &item.claim.occurrence, &item.key, now)
                .await?;
            crate::atlas_derivative_worker::enqueue_cleanup_in(tx, &item.claim.occurrence, now)
                .await?;
        }
    }
    Ok(results)
}

pub async fn commit_maintenance_outcomes(
    owner: &AdmissionOwner,
    fence: &MaintenancePublicationFence,
    items: &[MaintenanceOutcome],
    now: DateTime<Utc>,
) -> Result<BatchCommit, AdmissionError> {
    owner.ensure_open()?;
    let batch = prepare_maintenance_outcomes(fence, items)?;
    let captured = (fence.clone(), batch.clone());
    let committed = owned_tx(owner, RetryPolicy::default(), move |tx| {
        let (fence, batch) = captured.clone();
        async move { commit_prepared_maintenance_in(&tx, &fence, &batch, now).await }
    })
    .await;
    match committed {
        Ok(committed) => Ok(BatchCommit {
            items: committed.value,
            retries: committed.retries,
        }),
        Err(AdmissionError::Storage(TxError::Abort(code)))
            if code.starts_with("maintenance_receipt_conflict:") =>
        {
            let index = code
                .trim_start_matches("maintenance_receipt_conflict:")
                .parse::<usize>()
                .map_err(|_| AdmissionError::InvalidReceipt {
                    code: "maintenance_receipt_conflict_correlation".into(),
                })?;
            let item = batch
                .prepared
                .get(index)
                .ok_or_else(|| AdmissionError::InvalidReceipt {
                    code: "maintenance_receipt_conflict_correlation".into(),
                })?;
            Err(AdmissionError::ReceiptConflict {
                key: item.key.clone(),
            })
        }
        Err(error) => Err(error),
    }
}

#[derive(SurrealValue)]
struct BoundedClaimHeader {
    article_id: Option<String>,
    input_revision: i64,
    source_revision: Option<String>,
    classifier_input_hash: Option<String>,
    work_seq: i64,
    epoch: i64,
}
impl BoundedClaimHeader {
    fn into_header(self) -> Result<ClaimHeader, AdmissionError> {
        let missing = || AdmissionError::PayloadTooLarge {
            kind: "claim_header".into(),
            limit: 512,
        };
        Ok(ClaimHeader {
            article_id: self.article_id.ok_or_else(missing)?,
            input_revision: self.input_revision,
            source_revision: self.source_revision.ok_or_else(missing)?,
            classifier_input_hash: self.classifier_input_hash.ok_or_else(missing)?,
            work_seq: self.work_seq,
            epoch: self.epoch,
        })
    }
}

#[derive(Debug, Clone)]
pub enum HydrationGroup {
    Ready(Vec<AdmissionClaim>),
    Unavailable(Vec<AdmissionClaim>),
}
/// Preserves source order, including individual oversized dispositions, so a
/// durable cursor cannot jump over an unexamined sibling after a failed read.
pub async fn hydration_groups(
    owner: &AdmissionOwner,
    claims: &[AdmissionClaim],
) -> Result<Vec<HydrationGroup>, AdmissionError> {
    if claims.len() > 512 {
        return Err(AdmissionError::InvalidPage {
            code: "hydration_claim_limit".into(),
        });
    }
    let keys: Vec<_> = claims
        .iter()
        .map(|claim| claim.occurrence.clone())
        .collect();
    let costs = owner
        .scope(super::atlas_metadata::metadata_wire_costs(
            owner.repository().db(),
            &keys,
        ))
        .await?;
    let mut out = Vec::new();
    let mut group = Vec::new();
    let mut bytes = 0usize;
    for claim in claims {
        let cost = costs.get(&claim.occurrence).copied().unwrap_or(Some(1024));
        match cost {
            None => {
                if !group.is_empty() {
                    out.push(HydrationGroup::Ready(std::mem::take(&mut group)));
                    bytes = 0;
                }
                match out.last_mut() {
                    Some(HydrationGroup::Unavailable(claims)) if claims.len() < 64 => {
                        claims.push(claim.clone())
                    }
                    _ => out.push(HydrationGroup::Unavailable(vec![claim.clone()])),
                };
            }
            Some(cost) => {
                if !group.is_empty()
                    && (group.len() == 64
                        || bytes
                            .checked_add(cost)
                            .is_none_or(|total| total > MAX_RESPONSE_BYTES))
                {
                    out.push(HydrationGroup::Ready(std::mem::take(&mut group)));
                    bytes = 0;
                }
                bytes += cost;
                group.push(claim.clone());
            }
        }
    }
    if !group.is_empty() {
        out.push(HydrationGroup::Ready(group));
    }
    Ok(out)
}

fn staged_matches(row: &StagedInput, claim: &AdmissionClaim) -> bool {
    row.owner == claim.owner
        && u64::try_from(row.epoch).ok() == Some(claim.epoch)
        && u64::try_from(row.input_revision).ok() == Some(claim.input_revision)
        && row.source_revision == claim.source_revision
        && row.classifier_input_hash == claim.classifier_input_hash
        && u64::try_from(row.work_seq).ok() == Some(claim.work_seq)
}

enum ClassificationReader<'a> {
    Db(&'a surrealdb::Surreal<surrealdb::engine::any::Any>),
    Tx(&'a super::tx::Tx),
}
impl ClassificationReader<'_> {
    fn query(&self, sql: String) -> MeasuredQuery<'_> {
        match self {
            Self::Db(db) => db_query(db, sql),
            Self::Tx(tx) => tx_query(tx, sql),
        }
    }
}
#[derive(SurrealValue)]
struct ProjectionWire {
    projection: Option<ProjectionRow>,
}
#[derive(SurrealValue)]
struct ReceiptWire {
    receipt64: Option<String>,
    receipt_sha256: Option<String>,
}
pub(crate) async fn classification_views_many_in(
    tx: &super::tx::Tx,
    keys: &[OccurrenceKey],
) -> Result<std::collections::BTreeMap<OccurrenceKey, ClassificationView>, AdmissionError> {
    classification_views_read(ClassificationReader::Tx(tx), keys).await
}
async fn classification_views_read(
    reader: ClassificationReader<'_>,
    keys: &[OccurrenceKey],
) -> Result<std::collections::BTreeMap<OccurrenceKey, ClassificationView>, AdmissionError> {
    if keys.len() > 64
        || keys.iter().any(|key| {
            !crate::intel_service::valid_id(&key.run_id)
                || !crate::intel_service::valid_id(&key.article_id)
        })
    {
        return Err(AdmissionError::InvalidPage {
            code: "classification_view_batch_limit".into(),
        });
    }
    if keys.is_empty() {
        return Ok(std::collections::BTreeMap::new());
    }
    let mut sql = String::new();
    for index in 0..keys.len() {
        sql.push_str(&format!("SELECT IF bytes::len(type::bytes(string::concat(run_id,article_id,receipt_key,taxonomy_version,admission_state,publication_version,source_revision)))<=8192 {{{{run_id:run_id,article_id:article_id,receipt_key:receipt_key,taxonomy_version:taxonomy_version,admission_state:admission_state,publication_version:publication_version,input_revision:input_revision,source_revision:source_revision}}}} ELSE {{NONE}} AS projection FROM ONLY $key{index};"));
    }
    let mut query = reader.query(sql);
    for (index, key) in keys.iter().enumerate() {
        query = query.bind((
            format!("key{index}"),
            super::jobs::rid("atlas_classification_projection", &key.record_id()),
        ));
    }
    let mut response = query
        .await
        .map_err(TxError::from)?
        .check()
        .map_err(TxError::from)?;
    let mut projections = Vec::new();
    let mut receipts = Vec::new();
    let mut sql = String::new();
    for index in 0..keys.len() {
        let projection = match response
            .take::<Option<ProjectionWire>>(index)
            .map_err(TxError::from)?
        {
            None => None,
            Some(wire) => Some(
                wire.projection
                    .ok_or_else(|| AdmissionError::PayloadTooLarge {
                        kind: "classification_projection".into(),
                        limit: 8192,
                    })?,
            ),
        };
        if let Some(projection) = projection.as_ref().filter(|projection| {
            matches!(projection.admission_state.as_str(), "admitted" | "rejected")
        }) {
            let position = receipts.len();
            receipts.push((index, projection.receipt_key.clone()));
            sql.push_str(&format!("SELECT IF bytes::len(type::bytes(receipt_json))<=131072 {{encoding::base64::encode(type::bytes(receipt_json),true)}} ELSE {{NONE}} AS receipt64,IF bytes::len(type::bytes(receipt_sha256))<=64 {{receipt_sha256}} ELSE {{NONE}} AS receipt_sha256 FROM ONLY $receipt{position};"));
        }
        projections.push(projection);
    }
    let mut read = std::collections::BTreeMap::new();
    if !receipts.is_empty() {
        let mut query = reader.query(sql);
        for (position, (_, receipt)) in receipts.iter().enumerate() {
            query = query.bind((
                format!("receipt{position}"),
                super::jobs::rid("atlas_classification_receipt", receipt),
            ));
        }
        let mut response = query
            .await
            .map_err(TxError::from)?
            .check()
            .map_err(TxError::from)?;
        for (position, (index, _)) in receipts.iter().enumerate() {
            let row = response
                .take::<Option<ReceiptWire>>(position)
                .map_err(TxError::from)?
                .ok_or_else(|| AdmissionError::InvalidReceipt {
                    code: "receipt_missing".into(),
                })?;
            let bytes = decode_wire_json(
                &row.receipt64
                    .ok_or_else(|| AdmissionError::PayloadTooLarge {
                        kind: "receipt".into(),
                        limit: MAX_RECEIPT_BYTES,
                    })?,
                MAX_RECEIPT_BYTES,
            )?;
            let receipt_json =
                String::from_utf8(bytes).map_err(|_| AdmissionError::InvalidReceipt {
                    code: "receipt_json_invalid".into(),
                })?;
            let receipt_sha256 =
                row.receipt_sha256
                    .ok_or_else(|| AdmissionError::InvalidReceipt {
                        code: "receipt_checksum_mismatch".into(),
                    })?;
            read.insert(
                *index,
                ReceiptRow {
                    receipt_json,
                    receipt_sha256,
                },
            );
        }
    }
    let mut out = std::collections::BTreeMap::new();
    for (index, (key, projection)) in keys.iter().zip(projections).enumerate() {
        out.insert(
            key.clone(),
            finish_classification_view(key, projection, read.remove(&index))?,
        );
    }
    Ok(out)
}

/// Durable publication identity, retained across maintenance restarts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct DerivativeWork {
    pub occurrence: OccurrenceKey,
    pub receipt_key: String,
}
impl DerivativeWork {
    pub fn new(occurrence: OccurrenceKey, receipt_key: String) -> Self {
        Self {
            occurrence,
            receipt_key,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum DerivativeStage {
    Metadata,
    Actors,
    Related,
    Reconcile,
}
impl DerivativeStage {
    pub const ALL: [Self; 4] = [Self::Metadata, Self::Actors, Self::Related, Self::Reconcile];
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Metadata => "metadata",
            Self::Actors => "actors",
            Self::Related => "related",
            Self::Reconcile => "reconcile",
        }
    }
}
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum DerivativeFence {
    Cycle(RunClaim),
    Job(Box<crate::atlas_derivative_worker::JobFence>),
    Maintenance(Box<MaintenancePublicationFence>),
    NativeRefresh(NativeRefreshFence),
}
impl DerivativeFence {
    fn identity(&self) -> (&str, u64) {
        match self {
            Self::Cycle(run) => (&run.owner, run.epoch),
            Self::Job(job) => job.identity(),
            Self::Maintenance(fence) => (&fence.task.lease_owner, fence.task.lease_epoch as u64),
            Self::NativeRefresh(fence) => (&fence.identity, 1),
        }
    }
    pub(crate) async fn check_in(
        &self,
        tx: &super::tx::Tx,
        now: DateTime<Utc>,
        dispatch: bool,
    ) -> Result<bool, TxError> {
        match self {
            Self::Job(job) => job.check_in(tx, now, !dispatch).await,
            Self::NativeRefresh(fence) => Ok(!fence.sources.is_empty()),
            Self::Maintenance(fence) => {
                if dispatch {
                    fence.check_in(tx, now).await
                } else {
                    fence.owns_in(tx, now).await
                }
            }
            Self::Cycle(run) => {
                let mut response=tx_query(tx,"UPSERT type::record('atlas_classifier_guard',$guard) SET updated_at=<datetime>$now RETURN NONE;RETURN count(SELECT id FROM atlas_classifier_run WHERE id=$run AND owner=$owner AND epoch=$epoch AND state='running' AND lease_until> <datetime>$now LIMIT 1)=1;")
                    .bind(("guard",codec::encode(&["run",&run.run_id]))).bind(("run",super::jobs::rid("atlas_classifier_run",&run.run_id))).bind(("owner",run.owner.clone())).bind(("epoch",i64::try_from(run.epoch).map_err(|_|TxError::Abort("run_epoch_overflow".into()))?)).bind(("now",now.to_rfc3339())).await?.check()?;
                Ok(response.take::<Option<bool>>(1)? == Some(true))
            }
        }
    }
    fn contains(&self, key: &OccurrenceKey) -> bool {
        match self {
            Self::Cycle(run) => run.run_id == key.run_id,
            Self::Job(job) => job.contains(key),
            Self::Maintenance(fence) => fence.scope.contains(key),
            Self::NativeRefresh(fence) => fence.sources.contains_key(key),
        }
    }
}

/// Immutable native inputs captured while holding the source and body guards.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct DerivativeSnapshot {
    pub source: RetainedSourceFence,
    pub receipt_key: String,
    pub admission_state: String,
    pub metadata_sha256: String,
    pub body: Option<BodyRevision>,
    #[serde(default)]
    pub catalog_epoch: i64,
}
impl DerivativeSnapshot {
    pub fn revision(&self, stage: DerivativeStage) -> Result<String, TxError> {
        let json = serde_json::to_vec(&(
            "atlas-native-input-v2",
            super::lifecycle::RECONCILE_VERSION,
            self.catalog_epoch,
            stage,
            crate::atlas_metadata::ENTITY_VERSION,
            crate::atlas_metadata::PRIOR_VERSION,
            crate::atlas_actor_index::INDEX_CONTRACT,
            crate::brain_actors::IDENTITY_INDEX_VERSION,
            crate::related_articles::MATCH_VERSION,
            &self.source,
            &self.receipt_key,
            &self.admission_state,
            &self.metadata_sha256,
            if stage == DerivativeStage::Actors {
                self.body.as_ref()
            } else {
                None
            },
        ))
        .map_err(|_| TxError::Abort("native_input_json".into()))?;
        Ok(crate::classifier::artifact::sha256_hex(&json))
    }
}
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct DerivativeClaim {
    pub work: DerivativeWork,
    pub stage: DerivativeStage,
    pub owner: String,
    pub epoch: u64,
    pub snapshot: DerivativeSnapshot,
    pub input_revision: String,
    pub checkpoint_json: String,
    pub article: AtlasArticleRow,
    pub metadata: Option<ArticleMetadata>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum DerivativeDisposition {
    Current,
    PolicyIneligible { reason: String },
    Disabled { reason: String },
    Retryable { code: String },
    StaleInput,
}
impl DerivativeDisposition {
    pub fn terminal(&self) -> bool {
        matches!(
            self,
            Self::Current | Self::PolicyIneligible { .. } | Self::Disabled { .. }
        )
    }
    fn state(&self) -> &'static str {
        if self.terminal() {
            "complete"
        } else {
            "pending"
        }
    }
    fn code(&self) -> &str {
        match self {
            Self::Current => "",
            Self::PolicyIneligible { reason } | Self::Disabled { reason } => reason,
            Self::Retryable { code } => code,
            Self::StaleInput => "stale_input",
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct DerivativeItemReport {
    pub work: DerivativeWork,
    pub stage: DerivativeStage,
    pub disposition: DerivativeDisposition,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[non_exhaustive]
pub struct DerivativeBatchReport {
    pub items: Vec<DerivativeItemReport>,
    pub remaining: u64,
    pub states: Vec<DerivativeWorkState>,
}

pub(crate) async fn derivative_source_guards_in(
    tx: &super::tx::Tx,
    keys: &[OccurrenceKey],
) -> Result<(), TxError> {
    let mut sql = String::new();
    for index in 0..keys.len() {
        sql.push_str(&format!("UPSERT type::record('atlas_classifier_guard',$g{index}) SET updated_at=time::now() RETURN NONE;"));
    }
    if sql.is_empty() {
        return Ok(());
    }
    let mut query = tx_query(tx, sql);
    for (index, key) in keys.iter().enumerate() {
        query = query.bind((
            format!("g{index}"),
            codec::encode(&["source", &key.run_id, &key.article_id]),
        ));
    }
    query.await?.check()?;
    Ok(())
}

pub(crate) async fn derivative_snapshots_in(
    tx: &super::tx::Tx,
    keys: &[OccurrenceKey],
    stage: DerivativeStage,
) -> Result<
    std::collections::BTreeMap<
        OccurrenceKey,
        (DerivativeSnapshot, AtlasArticleRow, Option<ArticleMetadata>),
    >,
    TxError,
> {
    derivative_source_guards_in(tx, keys).await?;
    super::jobs::bump(tx, "job_guard", "atlas-derivative-catalog-input").await?;
    let mut catalog = tx
        .query("RETURN (SELECT VALUE epoch FROM ONLY atlas_derivative_catalog:current) ?? 0;")
        .await?
        .check()?;
    let catalog_epoch = catalog
        .take::<Option<i64>>(0)?
        .ok_or_else(|| TxError::Abort("catalog_epoch_missing".into()))?;
    let articles = super::atlas::articles_many_in(tx, keys).await?;
    let rows: Vec<_> = articles.values().cloned().collect();
    let sources = retained_source_fences_many_in(tx, &rows).await?;
    let views = classification_views_many_in(tx, keys)
        .await
        .map_err(|_| TxError::Abort("native_classification_invalid".into()))?;
    let mut metadata = super::atlas_metadata::metadata_many_in(tx, keys).await?;
    let bodies = if stage == DerivativeStage::Actors {
        let ids: Vec<_> = keys.iter().map(|key| key.article_id.clone()).collect();
        super::pipeline_inputs::body_input_guard_in(tx, &ids).await?;
        super::actor_catalog::clean_body_heads_many_in(tx, &ids).await?
    } else {
        std::collections::BTreeMap::new()
    };
    let mut out = std::collections::BTreeMap::new();
    for (key, source) in sources {
        let Some(view) = views.get(&key) else {
            continue;
        };
        let Some(receipt) = view.receipt.as_ref().filter(|_| {
            !view.legacy && matches!(view.admission_state.as_str(), "admitted" | "rejected")
        }) else {
            continue;
        };
        let Some(article) = articles.get(&key) else {
            continue;
        };
        // Rejected historical rows intentionally retain their original source.
        // Admitted native work requires the exact current classifier input/pair.
        if view.admission_state == "admitted"
            && (receipt.key.classifier_input_hash != source.classifier_input_hash
                || receipt.key.input_revision != source.native_input_revision
                || receipt
                    .receipt
                    .selected_domains
                    .iter()
                    .map(|domain| domain.as_str().to_owned())
                    .collect::<Vec<_>>()
                    != article.categories)
        {
            continue;
        }
        let native = metadata.remove(&key);
        let metadata_json = serde_json::to_vec(&native)
            .map_err(|_| TxError::Abort("native_metadata_json".into()))?;
        let snapshot = DerivativeSnapshot {
            source,
            receipt_key: receipt.key.record_id(),
            admission_state: view.admission_state.clone(),
            metadata_sha256: crate::classifier::artifact::sha256_hex(&metadata_json),
            body: bodies.get(&key.article_id).cloned(),
            catalog_epoch,
        };
        out.insert(key, (snapshot, article.clone(), native));
    }
    Ok(out)
}

#[derive(SurrealValue)]
struct DerivativeLeaseReply {
    epoch: i64,
    checkpoint_json: Option<String>,
}

/// Claims only exact current receipt work. A failed stage is attempted once per
/// activation; reopening under a new run/task epoch can retry it safely.
pub async fn claim_derivatives(
    owner: &AdmissionOwner,
    fence: &DerivativeFence,
    work: &[DerivativeWork],
    stage: DerivativeStage,
    now: DateTime<Utc>,
) -> Result<Vec<DerivativeClaim>, AdmissionError> {
    validate_derivative_work(work)?;
    if work.len() > 64
        || work
            .iter()
            .any(|work| !fence.contains(&work.occurrence) || work.receipt_key.len() > 4096)
    {
        return Err(AdmissionError::InvalidPage {
            code: "derivative_work_scope".into(),
        });
    }
    if work.is_empty() {
        return Ok(Vec::new());
    }
    let fence = fence.clone();
    let work = work.to_vec();
    let committed=owned_tx(owner,RetryPolicy::default(),move|tx|{let fence=fence.clone();let work=work.clone();async move {
        if !fence.check_in(&tx,now,true).await? {return Err(TxError::Abort("derivative_authority_lost".into()));}
        let keys:Vec<_>=work.iter().map(|work|work.occurrence.clone()).collect();
        let mut snapshots=derivative_snapshots_in(&tx,&keys,stage).await?;
        let actor_outputs = if stage == DerivativeStage::Actors {
            crate::atlas_actor_index::surreal::receipts_many_in(&tx, &keys).await?
        } else { std::collections::BTreeMap::new() };
        let(identity,activation_epoch)=fence.identity();let activation=codec::encode(&[identity,&activation_epoch.to_string()]);
        let mut claims=Vec::new();
        for work in work {
            // The outside adoption probe is only an optimization. Another
            // publisher may have committed a cleanup cursor since that read;
            // never let the revision-reset claim discard its durable frontier.
            if stage == DerivativeStage::Reconcile
                && crate::atlas_derivative_worker::has_owed_cleanup_in(&tx, &work.occurrence).await? {
                continue;
            }
            let Some((snapshot,article,metadata))=snapshots.remove(&work.occurrence)else{continue;};
            if snapshot.receipt_key!=work.receipt_key {continue;}
            if let DerivativeFence::NativeRefresh(native)=&fence {if native.sources.get(&work.occurrence)!=Some(&(snapshot.source.clone(),snapshot.receipt_key.clone())) {continue;}}
            if let DerivativeFence::Maintenance(maintenance)=&fence {
                let mut response=tx_query(&tx,"RETURN count(SELECT id FROM atlas_reclassification_item WHERE job_id=$job AND run_id=$run AND article_id=$article AND receipt_key=$receipt AND disposition INSIDE ['admitted','rejected','already_current'] LIMIT 1)=1;")
                    .bind(("job",maintenance.task.job_id.clone())).bind(("run",work.occurrence.run_id.clone())).bind(("article",work.occurrence.article_id.clone())).bind(("receipt",work.receipt_key.clone())).await?.check()?;
                if response.take::<Option<bool>>(0)?!=Some(true){continue;}
            }
            let revision=snapshot.revision(stage)?;
            let output_current = stage != DerivativeStage::Actors
                || actor_outputs.get(&work.occurrence) == Some(&revision);
            let mut response=tx_query(&tx,"{LET $row=(SELECT * FROM ONLY $key);IF $row=NONE OR ($row.state='complete' AND $row.output_revision=$revision AND ($row.disposition_json!='{\"kind\":\"current\"}' OR $output_current)) OR ($row.owner!='' AND $row.lease_until> <datetime>$now) OR ($row.last_activation=$activation AND $row.output_revision=$revision AND $row.last_error!='' AND $row.checkpoint_json='') {RETURN NONE;};UPDATE $key SET owner=$owner,epoch=(epoch??0)+1,lease_until=<datetime>$until,state='evaluating',disposition_json='',checkpoint_json=IF $row.state='complete' OR $row.output_revision!=$revision {''} ELSE {$row.checkpoint_json ?? ''},captured_json=$snapshot,output_revision=$revision,last_activation=$activation,updated_at=<datetime>$now RETURN NONE;LET $new=(SELECT epoch,IF bytes::len(type::bytes(checkpoint_json))<=16384 {checkpoint_json} ELSE {NONE} AS checkpoint_json FROM ONLY $key);RETURN $new;}")
                .bind(("key",derivative_record(&work.receipt_key,stage))).bind(("owner",identity.to_owned())).bind(("activation",activation.clone())).bind(("revision",revision.clone())).bind(("output_current",output_current)).bind(("snapshot",serde_json::to_string(&snapshot).map_err(|_|TxError::Abort("derivative_snapshot_json".into()))?)).bind(("now",now.to_rfc3339())).bind(("until",(now+Duration::seconds(CLAIM_LEASE_SECS)).to_rfc3339())).await?.check()?;
            if let Some(reply)=response.take::<Option<DerivativeLeaseReply>>(0)? {
                let stale_stages=if stage==DerivativeStage::Reconcile {vec![DerivativeStage::Reconcile]}else{vec![DerivativeStage::Actors,DerivativeStage::Reconcile]};
                super::atlas_derivative_status::reset_in(&tx,&work.occurrence,&work.receipt_key,&stale_stages).await?;
                claims.push(DerivativeClaim {work,stage,owner:identity.into(),epoch:u64::try_from(reply.epoch).map_err(|_|TxError::Abort("negative_derivative_epoch".into()))?,snapshot,input_revision:revision,checkpoint_json:reply.checkpoint_json.ok_or_else(||TxError::Abort("derivative_checkpoint_oversized".into()))?,article,metadata});
            }
        }
        Ok(claims)
    }}).await?;
    Ok(committed.value)
}

/// Checks both the live authority and all captured native inputs. Called in
/// the SAME transaction before any native inventory/receipt mutation.
pub(crate) async fn check_derivative_in(
    tx: &super::tx::Tx,
    fence: &DerivativeFence,
    claim: &DerivativeClaim,
    now: DateTime<Utc>,
) -> Result<bool, TxError> {
    if !fence.contains(&claim.work.occurrence) || !fence.check_in(tx, now, true).await? {
        return Ok(false);
    }
    if let DerivativeFence::NativeRefresh(native) = fence {
        if native.sources.get(&claim.work.occurrence)
            != Some(&(
                claim.snapshot.source.clone(),
                claim.work.receipt_key.clone(),
            ))
        {
            return Ok(false);
        }
    }
    let mut snapshots = derivative_snapshots_in(
        tx,
        std::slice::from_ref(&claim.work.occurrence),
        claim.stage,
    )
    .await?;
    if snapshots
        .remove(&claim.work.occurrence)
        .map(|(snapshot, _, _)| snapshot.revision(claim.stage))
        .transpose()?
        .as_deref()
        != Some(claim.input_revision.as_str())
    {
        return Ok(false);
    }
    derivative_lease_in(tx, claim, now).await
}
async fn derivative_lease_in(
    tx: &super::tx::Tx,
    claim: &DerivativeClaim,
    now: DateTime<Utc>,
) -> Result<bool, TxError> {
    let mut response=tx_query(tx,"RETURN count(SELECT id FROM atlas_classification_derivative WHERE id=$key AND owner=$owner AND epoch=$epoch AND state='evaluating' AND output_revision=$revision AND receipt_key=$receipt AND lease_until> <datetime>$now LIMIT 1)=1;")
        .bind(("key",derivative_record(&claim.work.receipt_key,claim.stage))).bind(("owner",claim.owner.clone())).bind(("epoch",i64::try_from(claim.epoch).map_err(|_|TxError::Abort("derivative_epoch_overflow".into()))?)).bind(("revision",claim.input_revision.clone())).bind(("receipt",claim.work.receipt_key.clone())).bind(("now",now.to_rfc3339())).await?.check()?;
    Ok(response.take::<Option<bool>>(0)? == Some(true))
}
/// Seals an already checked native publication. Check BEFORE writing metadata;
/// completion itself rechecks lease authority without comparing its old output.
pub(crate) async fn complete_derivative_in(
    tx: &super::tx::Tx,
    fence: &DerivativeFence,
    claim: &DerivativeClaim,
    disposition: &DerivativeDisposition,
    checkpoint: &str,
    now: DateTime<Utc>,
) -> Result<bool, TxError> {
    if checkpoint.len() > 16384 || disposition.code().len() > 256 {
        return Err(TxError::Abort("derivative_completion_bound".into()));
    }
    if !fence.check_in(tx, now, disposition.terminal()).await?
        || !derivative_lease_in(tx, claim, now).await?
    {
        return Ok(false);
    }
    let revision = if !matches!(disposition, DerivativeDisposition::StaleInput) {
        derivative_snapshots_in(
            tx,
            std::slice::from_ref(&claim.work.occurrence),
            claim.stage,
        )
        .await?
        .remove(&claim.work.occurrence)
        .ok_or_else(|| TxError::Abort("derivative_source_changed_during_publication".into()))?
        .0
        .revision(claim.stage)?
    } else {
        claim.input_revision.clone()
    };
    tx_query(tx,"UPDATE $key SET state=$state,output_revision=$revision,checkpoint_json=$checkpoint,last_error=$error,disposition_json=$disposition,owner='',lease_until=NONE,updated_at=<datetime>$now RETURN NONE;")
        .bind(("key",derivative_record(&claim.work.receipt_key,claim.stage))).bind(("state",disposition.state().to_owned())).bind(("disposition",serde_json::to_string(disposition).map_err(|_|TxError::Abort("derivative_disposition_json".into()))?)).bind(("revision",revision)).bind(("checkpoint",checkpoint.to_owned())).bind(("error",disposition.code().to_owned())).bind(("now",now.to_rfc3339())).await?.check()?;
    super::atlas_derivative_status::publish_in(tx, claim, disposition).await?;
    Ok(true)
}

/// Loads resumable work from this job's published items, never from another
/// historical run's task or an inferred current source capture key.
pub async fn maintenance_derivative_work(
    owner: &AdmissionOwner,
    fence: &MaintenancePublicationFence,
    limit: usize,
) -> Result<Vec<DerivativeWork>, AdmissionError> {
    if limit == 0 || limit > 64 {
        return Err(AdmissionError::InvalidPage {
            code: "derivative_scan_bound".into(),
        });
    }
    #[derive(SurrealValue)]
    struct WorkRow {
        run_id: String,
        article_id: String,
        receipt_key: String,
    }
    let fence = fence.clone();
    let committed=owned_tx(owner,RetryPolicy::default(),move|tx|{let fence=fence.clone();async move {
        let now=Utc::now();if !fence.owns_in(&tx,now).await?{return Err(TxError::Abort("derivative_authority_lost".into()));}
        let mut response=tx_query(&tx,"SELECT run_id,article_id,receipt_key FROM atlas_reclassification_item WHERE job_id=$job AND disposition INSIDE ['admitted','rejected','already_current'] AND receipt_key!='' AND ((derivative_state ?? 'pending')='pending' OR count(SELECT id FROM atlas_classification_derivative WHERE receipt_key=$parent.receipt_key AND state!='complete' LIMIT 1)>0) ORDER BY run_id,article_id,receipt_key LIMIT $limit;").bind(("job",fence.task.job_id)).bind(("limit",limit as i64)).await?.check()?;
        let rows:Vec<WorkRow>=response.take(0)?;Ok(rows.into_iter().map(|row|DerivativeWork::new(OccurrenceKey::new(row.run_id,row.article_id),row.receipt_key)).collect::<Vec<_>>())
    }}).await?;
    Ok(committed.value)
}

/// Releases a failed/cancelled stage under the still-owned task fence. Cleanup
/// remains legal after pause/cancel, unlike new dispatch or successful writes.
pub(crate) async fn release_derivative(
    owner: &AdmissionOwner,
    fence: &DerivativeFence,
    claim: &DerivativeClaim,
    disposition: DerivativeDisposition,
) -> Result<DerivativeDisposition, AdmissionError> {
    if disposition.terminal() || disposition.code().len() > 256 {
        return Err(AdmissionError::InvalidPage {
            code: "derivative_release_disposition".into(),
        });
    }
    let fence = fence.clone();
    let claim = claim.clone();
    let output = disposition.clone();
    let committed = owned_tx(owner, RetryPolicy::default(), move |tx| {
        let fence = fence.clone();
        let claim = claim.clone();
        let disposition = disposition.clone();
        async move {
            let now = Utc::now();
            if complete_derivative_in(&tx, &fence, &claim, &disposition, "", now).await? {
                return Ok(disposition);
            }
            // Pausing/cancelling the root revokes publication authority, but
            // must not strand this worker's child claim until lease expiry.
            // An uncertain commit or a newer owner fails this exact CAS.
            if !derivative_lease_in(&tx, &claim, now).await? {
                return Ok(DerivativeDisposition::StaleInput);
            }
            tx_query(&tx, "UPDATE $key SET state='pending',owner='',lease_until=NONE,disposition_json=$disposition,last_error=$error,updated_at=<datetime>$now RETURN NONE;")
                .bind(("key", derivative_record(&claim.work.receipt_key, claim.stage)))
                .bind(("disposition", serde_json::to_string(&disposition).map_err(|_| TxError::Abort("derivative_disposition_json".into()))?))
                .bind(("error", disposition.code().to_owned()))
                .bind(("now", now.to_rfc3339()))
                .await?.check()?;
            Ok(disposition)
        }
    })
    .await?;
    if committed.value == output {
        Ok(output)
    } else {
        Ok(committed.value)
    }
}

/// Runs the same native processors used by foreground work. Every work and
/// stage remains leased/revision checked; unavailable native inputs stay queued.
pub async fn process_maintenance_derivatives(
    owner: &AdmissionOwner,
    fence: &MaintenancePublicationFence,
    work: &[DerivativeWork],
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<DerivativeBatchReport, AdmissionError> {
    process_derivatives(
        owner,
        &DerivativeFence::Maintenance(Box::new(fence.clone())),
        work,
        cancel,
    )
    .await
}

pub(crate) fn process_derivatives<'a>(
    owner: &'a AdmissionOwner,
    fence: &'a DerivativeFence,
    work: &'a [DerivativeWork],
    cancel: &'a std::sync::atomic::AtomicBool,
) -> Pin<Box<impl Future<Output = Result<DerivativeBatchReport, AdmissionError>> + Send + 'a>> {
    Box::pin(process_derivative_stages(
        owner,
        fence,
        work,
        cancel,
        [
            DerivativeStage::Metadata,
            DerivativeStage::Related,
            DerivativeStage::Actors,
            DerivativeStage::Reconcile,
        ],
    ))
}

pub(crate) fn process_derivative_stage<'a>(
    owner: &'a AdmissionOwner,
    fence: &'a DerivativeFence,
    work: &'a [DerivativeWork],
    stage: DerivativeStage,
    cancel: &'a std::sync::atomic::AtomicBool,
) -> Pin<Box<impl Future<Output = Result<DerivativeBatchReport, AdmissionError>> + Send + 'a>> {
    // Allocate before the caller starts polling; Atlas must not carry this
    // dispatcher inline while a native stage enters embedded database queries.
    Box::pin(process_derivative_stages(
        owner,
        fence,
        work,
        cancel,
        [stage],
    ))
}

async fn process_derivative_stages(
    owner: &AdmissionOwner,
    fence: &DerivativeFence,
    work: &[DerivativeWork],
    cancel: &std::sync::atomic::AtomicBool,
    stages: impl AsRef<[DerivativeStage]>,
) -> Result<DerivativeBatchReport, AdmissionError> {
    if work.len() > 64 {
        return Err(AdmissionError::InvalidPage {
            code: "derivative_process_bound".into(),
        });
    }
    let mut report = DerivativeBatchReport::default();
    let mut eligible: std::collections::BTreeSet<_> =
        work.iter().map(|work| work.occurrence.clone()).collect();
    // Related needs the captured metadata inventory; actor publication then
    // consumes both inventories before the final scoped fixed-point drain.
    for &stage in stages.as_ref() {
        if cancel.load(Ordering::Acquire) {
            break;
        }
        let selected: Vec<_> = work
            .iter()
            .filter(|work| eligible.contains(&work.occurrence))
            .cloned()
            .collect();
        let mut ready = Vec::with_capacity(selected.len());
        for item in selected {
            if matches!(stage, DerivativeStage::Actors | DerivativeStage::Reconcile)
                && process_owed_cleanup(owner, fence, &item, cancel).await?
            {
                eligible.remove(&item.occurrence);
                report.items.push(DerivativeItemReport {
                    work: item,
                    stage,
                    disposition: DerivativeDisposition::Retryable {
                        code: "published_cleanup_continuation".into(),
                    },
                });
            } else {
                ready.push(item);
            }
        }
        let claims = claim_derivatives(owner, fence, &ready, stage, Utc::now()).await?;
        let stage_cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        struct CancelStage(Arc<std::sync::atomic::AtomicBool>);
        impl Drop for CancelStage {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
            }
        }
        let _stop = CancelStage(stage_cancel.clone());
        let mut processing = Box::pin(process_claimed_derivatives(
            owner,
            fence,
            &claims,
            stage,
            &stage_cancel,
        ));
        let period = std::time::Duration::from_secs(HEARTBEAT_SECS);
        let mut heartbeat = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
        heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut authority_error = None;
        let items = loop {
            tokio::select! {
                result=&mut processing=>break result?,
                _=heartbeat.tick()=>{
                    if cancel.load(Ordering::Acquire){stage_cancel.store(true,Ordering::Release);}
                    if authority_error.is_none(){if let Err(error)=renew_derivative_claims(owner,fence,&claims,Utc::now()).await {stage_cancel.store(true,Ordering::Release);authority_error=Some(error);}}
                },
                _=tokio::time::sleep(std::time::Duration::from_millis(50)),if !stage_cancel.load(Ordering::Acquire)=>{if cancel.load(Ordering::Acquire){stage_cancel.store(true,Ordering::Release);}},
            }
        };
        if let Some(error) = authority_error {
            return Err(error);
        }
        for item in items {
            if !item.disposition.terminal() {
                eligible.remove(&item.work.occurrence);
            }
            report.items.push(item);
        }
    }
    let receipt_keys: Vec<_> = work.iter().map(|work| work.receipt_key.clone()).collect();
    let mut response=owner.query("RETURN count(SELECT id FROM atlas_classification_derivative WHERE receipt_key INSIDE $receipts AND state!='complete');").bind(("receipts",receipt_keys)).await.map_err(TxError::from)?.check().map_err(TxError::from)?;
    report.remaining = u64::try_from(
        response
            .take::<Option<i64>>(0)
            .map_err(TxError::from)?
            .ok_or_else(|| TxError::Abort("missing_derivative_remaining".into()))?,
    )
    .map_err(|_| TxError::Abort("negative_derivative_remaining".into()))?;
    report.states = derivative_states(owner, fence, work).await?;
    Ok(report)
}

/// Adopt owed published cleanup through this caller's live current capability.
/// Old receipt/stage ownership never authorizes new native material publication.
async fn process_owed_cleanup(
    owner: &AdmissionOwner,
    fence: &DerivativeFence,
    work: &DerivativeWork,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<bool, AdmissionError> {
    let mut response = owner.query("RETURN count(SELECT id FROM atlas_classification_derivative WITH INDEX atlas_derivative_occurrence_stage WHERE run_id=$run AND article_id=$article AND stage='reconcile' AND state!='complete' AND (checkpoint_json CONTAINS '\"frontier_kind\":\"reconcile\"' OR checkpoint_json CONTAINS '\"frontier_kind\":\"reconcile_restart\"') LIMIT 1)>0;")
        .bind(("run",work.occurrence.run_id.clone())).bind(("article",work.occurrence.article_id.clone())).await.map_err(TxError::from)?.check().map_err(TxError::from)?;
    if response.take::<Option<bool>>(0).map_err(TxError::from)? != Some(true) {
        return Ok(false);
    }
    if cancel.load(Ordering::Acquire) {
        return Ok(false);
    }
    let (fence, work) = (fence.clone(), work.clone());
    let output = Arc::new(std::sync::Mutex::new(None));
    let slot = output.clone();
    let cleanup_cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
    struct CancelCleanup(Arc<std::sync::atomic::AtomicBool>);
    impl Drop for CancelCleanup {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }
    let _stop = CancelCleanup(cleanup_cancel.clone());
    let transaction_cancel = cleanup_cancel.clone();
    let write: super::memory::MemoryWrite = Arc::new(move |tx| {
        let (fence, work, slot, cancel) = (
            fence.clone(),
            work.clone(),
            slot.clone(),
            transaction_cancel.clone(),
        );
        Box::pin(async move {
            if cancel.load(Ordering::Acquire) {
                return Err(TxError::Abort("cleanup_cancelled".into()));
            }
            if !fence.contains(&work.occurrence) || !fence.check_in(&tx, Utc::now(), false).await? {
                return Err(TxError::Abort("cleanup_current_authority_lost".into()));
            }
            let snapshots = derivative_snapshots_in(
                &tx,
                std::slice::from_ref(&work.occurrence),
                DerivativeStage::Actors,
            )
            .await?;
            let Some((snapshot, _, _)) = snapshots.get(&work.occurrence) else {
                return Err(TxError::Abort("cleanup_current_source_missing".into()));
            };
            if snapshot.receipt_key != work.receipt_key {
                return Err(TxError::Abort("cleanup_current_receipt_changed".into()));
            }
            if let DerivativeFence::NativeRefresh(native) = &fence {
                if native.sources.get(&work.occurrence)
                    != Some(&(snapshot.source.clone(), snapshot.receipt_key.clone()))
                {
                    return Err(TxError::Abort("cleanup_current_source_changed".into()));
                }
            }
            let progress =
                crate::atlas_derivative_worker::cleanup_one_in(&tx, &work.occurrence).await?;
            if cancel.load(Ordering::Acquire) {
                return Err(TxError::Abort("cleanup_cancelled".into()));
            }
            *slot
                .lock()
                .map_err(|_| TxError::Abort("cleanup_result_poisoned".into()))? =
                Some(progress.is_some());
            Ok(progress.map_or_else(Vec::new, |progress| progress.changed_memory_ids))
        })
    });
    let mut processing = Box::pin(super::memory::write_then_index(
        owner.repository().publisher(),
        write,
    ));
    loop {
        tokio::select! {
            result = &mut processing => { result?; break; },
            _ = tokio::time::sleep(std::time::Duration::from_millis(50)) => {
                if cancel.load(Ordering::Acquire) { cleanup_cancel.store(true, Ordering::Release); }
            }
        }
    }
    let result = output
        .lock()
        .map_err(|_| TxError::Abort("cleanup_result_poisoned".into()))?
        .take()
        .ok_or_else(|| TxError::Abort("cleanup_result_missing".into()))?;
    Ok(result)
}

async fn process_claimed_derivatives(
    owner: &AdmissionOwner,
    fence: &DerivativeFence,
    claims: &[DerivativeClaim],
    stage: DerivativeStage,
    cancel: &Arc<std::sync::atomic::AtomicBool>,
) -> Result<Vec<DerivativeItemReport>, AdmissionError> {
    let mut items = Vec::new();
    let actor_claims: Vec<_> = claims
        .iter()
        .filter(|claim| claim.snapshot.admission_state == "admitted")
        .cloned()
        .collect();
    let mut actor_results = if stage == DerivativeStage::Actors {
        Box::pin(crate::atlas_actor_index::process_derivative_claims(
            owner,
            fence,
            &actor_claims,
            cancel,
        ))
        .await
        .map_err(|_| TxError::Abort("native_actor_processing_failed".into()))?
    } else {
        std::collections::BTreeMap::new()
    };
    for claim in claims.iter().cloned() {
        let disposition = if claim.snapshot.admission_state == "rejected" {
            // Retained historical rejection preserves its original material
            // and graph. Explicit disposition prevents native synthesis.
            let disposition = DerivativeDisposition::PolicyIneligible {
                reason: "current_admission_rejected".into(),
            };
            let fence = fence.clone();
            let claim = claim.clone();
            let committed = owned_tx(owner, RetryPolicy::default(), move |tx| {
                let fence = fence.clone();
                let claim = claim.clone();
                let disposition = disposition.clone();
                async move {
                    let now = Utc::now();
                    if !check_derivative_in(&tx, &fence, &claim, now).await? {
                        return Ok(DerivativeDisposition::StaleInput);
                    }
                    if !complete_derivative_in(&tx, &fence, &claim, &disposition, "", now).await? {
                        return Err(TxError::Abort("native_rejection_lease_lost".into()));
                    }
                    Ok(disposition)
                }
            })
            .await?;
            committed.value
        } else if cancel.load(Ordering::Acquire) {
            release_derivative(
                owner,
                fence,
                &claim,
                DerivativeDisposition::Retryable {
                    code: "cancelled".into(),
                },
            )
            .await?
        } else {
            let result = match stage {
                DerivativeStage::Metadata => {
                    Box::pin(crate::atlas_metadata::process_derivative(
                        owner, fence, &claim, cancel,
                    ))
                    .await
                }
                DerivativeStage::Actors => Ok(actor_results
                    .remove(&claim.work.occurrence)
                    .unwrap_or(DerivativeDisposition::Retryable {
                        code: "actor_material_unavailable".into(),
                    })),
                DerivativeStage::Related => {
                    Box::pin(crate::related_articles::process_derivative(
                        owner, fence, &claim, cancel,
                    ))
                    .await
                }
                DerivativeStage::Reconcile => {
                    Box::pin(process_reconcile_derivative(owner, fence, &claim, cancel))
                        .await
                        .map_err(Into::into)
                }
            };
            match result {
                Ok(disposition) => disposition,
                Err(error) => {
                    let code = match error.downcast_ref::<TxError>() {
                        Some(TxError::Abort(code))
                            if code.len() <= 128
                                && code
                                    .bytes()
                                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_') =>
                        {
                            code.clone()
                        }
                        Some(TxError::Contention { .. }) => "native_publication_contention".into(),
                        Some(TxError::PublisherDropped) => "native_commit_uncertain".into(),
                        _ => "native_stage_failed".into(),
                    };
                    release_derivative(
                        owner,
                        fence,
                        &claim,
                        DerivativeDisposition::Retryable { code },
                    )
                    .await?
                }
            }
        };
        items.push(DerivativeItemReport {
            work: claim.work,
            stage,
            disposition,
        });
    }
    Ok(items)
}

async fn renew_derivative_claims(
    owner: &AdmissionOwner,
    fence: &DerivativeFence,
    claims: &[DerivativeClaim],
    now: DateTime<Utc>,
) -> Result<(), AdmissionError> {
    if claims.is_empty() {
        return Ok(());
    }
    let fence = fence.clone();
    let claims = claims.to_vec();
    owned_tx(owner,RetryPolicy::default(),move|tx|{let fence=fence.clone();let claims=claims.clone();async move {
        if !fence.check_in(&tx,now,false).await?{return Err(TxError::Abort("derivative_heartbeat_authority_lost".into()));}
        let mut sql=String::new();for index in 0..claims.len(){sql.push_str(&format!("UPDATE $key{index} SET lease_until=<datetime>$until WHERE state='evaluating' AND owner=$owner{index} AND epoch=$epoch{index} AND lease_until> <datetime>$now RETURN NONE;"));}
        let mut query=tx_query(&tx,sql).bind(("now",now.to_rfc3339())).bind(("until",(now+Duration::seconds(CLAIM_LEASE_SECS)).to_rfc3339()));
        for(index,claim)in claims.iter().enumerate(){query=query.bind((format!("key{index}"),derivative_record(&claim.work.receipt_key,claim.stage))).bind((format!("owner{index}"),claim.owner.clone())).bind((format!("epoch{index}"),i64::try_from(claim.epoch).map_err(|_|TxError::Abort("derivative_epoch_overflow".into()))?));}
        query.await?.check()?;Ok(())
    }}).await?;
    Ok(())
}

async fn process_reconcile_derivative(
    owner: &AdmissionOwner,
    fence: &DerivativeFence,
    claim: &DerivativeClaim,
    cancel: &Arc<std::sync::atomic::AtomicBool>,
) -> Result<DerivativeDisposition, AdmissionError> {
    let cancel = cancel.clone();
    let receipt_key = claim.work.receipt_key.clone();
    let stage = claim.stage;
    let fence = fence.clone();
    let claim = claim.clone();
    let repo = owner.repository().clone();
    let write: super::memory::MemoryWrite = Arc::new(move |tx| {
        let fence = fence.clone();
        let claim = claim.clone();
        let cancel = cancel.clone();
        Box::pin(async move {
            if cancel.load(Ordering::Acquire) {
                return Err(TxError::Abort("native_reconcile_cancelled".into()));
            }
            let now = Utc::now();
            if !check_derivative_in(&tx, &fence, &claim, now).await? {
                return Err(TxError::Abort("native_reconcile_stale".into()));
            }
            let cursor = if claim.checkpoint_json.is_empty() {
                None
            } else {
                Some(
                    serde_json::from_str::<super::lifecycle::ReconcileCursor>(
                        &claim.checkpoint_json,
                    )
                    .map_err(|_| TxError::Abort("native_reconcile_checkpoint".into()))?,
                )
            };
            let scope = super::lifecycle::ReconcileScope {
                keys: vec![super::lifecycle::ReconcileKey::Source {
                    run_id: claim.work.occurrence.run_id.clone(),
                    article_id: claim.work.occurrence.article_id.clone(),
                }],
            };
            let progress =
                super::lifecycle::reconcile_affected_in(&tx, &scope, cursor.as_ref()).await?;
            let checkpoint = progress
                .next
                .as_ref()
                .map(serde_json::to_string)
                .transpose()
                .map_err(|_| TxError::Abort("native_reconcile_checkpoint".into()))?
                .unwrap_or_default();
            let disposition = if progress.next.is_some() {
                DerivativeDisposition::Retryable {
                    code: "reconcile_continuation".into(),
                }
            } else {
                DerivativeDisposition::Current
            };
            if cancel.load(Ordering::Acquire) {
                return Err(TxError::Abort("native_reconcile_cancelled".into()));
            }
            if !complete_derivative_in(&tx, &fence, &claim, &disposition, &checkpoint, now).await? {
                return Err(TxError::Abort("native_reconcile_lease_lost".into()));
            }
            Ok(progress.changed_memory_ids)
        })
    });
    super::memory::write_then_index(repo.publisher(), write).await?;
    let mut response = owner
        .query("SELECT VALUE state FROM ONLY $key;")
        .bind(("key", derivative_record(&receipt_key, stage)))
        .await
        .map_err(TxError::from)?
        .check()
        .map_err(TxError::from)?;
    Ok(
        if response
            .take::<Option<String>>(0)
            .map_err(TxError::from)?
            .as_deref()
            == Some("complete")
        {
            DerivativeDisposition::Current
        } else {
            DerivativeDisposition::Retryable {
                code: "reconcile_continuation".into(),
            }
        },
    )
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct DerivativeStageState {
    pub stage: DerivativeStage,
    pub disposition: Option<DerivativeDisposition>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct DerivativeWorkState {
    pub work: DerivativeWork,
    pub stages: Vec<DerivativeStageState>,
    pub complete: bool,
}
/// Authoritative bounded restart audit. Missing/stale work cannot count as a
/// completed empty result; every stage must have a current terminal receipt.
pub async fn maintenance_derivative_states(
    owner: &AdmissionOwner,
    fence: &MaintenancePublicationFence,
    work: &[DerivativeWork],
) -> Result<Vec<DerivativeWorkState>, AdmissionError> {
    derivative_states(
        owner,
        &DerivativeFence::Maintenance(Box::new(fence.clone())),
        work,
    )
    .await
}
#[derive(SurrealValue)]
struct DerivativeStateRow {
    state: String,
    output_revision: String,
    disposition_json: Option<String>,
}
async fn derivative_states(
    owner: &AdmissionOwner,
    fence: &DerivativeFence,
    work: &[DerivativeWork],
) -> Result<Vec<DerivativeWorkState>, AdmissionError> {
    if work.len() > 64 || work.iter().any(|work| !fence.contains(&work.occurrence)) {
        return Err(AdmissionError::InvalidPage {
            code: "derivative_audit_scope".into(),
        });
    }
    let work = work.to_vec();
    let fence = fence.clone();
    Ok(owned_tx(owner, RetryPolicy::default(), move |tx| {
        let work = work.clone();
        let fence = fence.clone();
        async move {
            let states = derivative_states_in(&tx, &fence, &work, Utc::now()).await?;
            // Explicit compatibility refreshes also publish current readiness;
            // they do not pass through the independent worker's next-stage audit.
            for row in &states {
                for stage in &row.stages {
                    if matches!(
                        stage.stage,
                        DerivativeStage::Actors | DerivativeStage::Reconcile
                    ) && stage.disposition == Some(DerivativeDisposition::Current)
                    {
                        super::atlas_derivative_status::audit_current_in(
                            &tx,
                            &row.work.occurrence,
                            &row.work.receipt_key,
                            stage.stage,
                        )
                        .await?;
                    }
                }
            }
            Ok(states)
        }
    })
    .await?
    .value)
}

/// Joins the exact same authoritative native audit to U6b's item completion
/// transaction; no source/body writer can slip between audit and item CAS.
pub(crate) async fn maintenance_derivative_states_in(
    tx: &super::tx::Tx,
    fence: &MaintenancePublicationFence,
    work: &[DerivativeWork],
    now: DateTime<Utc>,
) -> Result<Vec<DerivativeWorkState>, TxError> {
    derivative_states_in(
        tx,
        &DerivativeFence::Maintenance(Box::new(fence.clone())),
        work,
        now,
    )
    .await
}
pub(crate) async fn derivative_states_in(
    tx: &super::tx::Tx,
    fence: &DerivativeFence,
    work: &[DerivativeWork],
    now: DateTime<Utc>,
) -> Result<Vec<DerivativeWorkState>, TxError> {
    validate_derivative_work(work).map_err(|_| TxError::Abort("derivative_audit_scope".into()))?;
    if work.len() > 64 || work.iter().any(|work| !fence.contains(&work.occurrence)) {
        return Err(TxError::Abort("derivative_audit_scope".into()));
    }
    if !fence.check_in(tx, now, false).await? {
        return Err(TxError::Abort("derivative_audit_lease_lost".into()));
    }
    let keys: Vec<_> = work.iter().map(|work| work.occurrence.clone()).collect();
    // All four stages share source, admission and metadata inputs. Capture
    // them once, including the body fence needed only by Actors; revision()
    // excludes that body from the other stages' identities.
    let current = derivative_snapshots_in(tx, &keys, DerivativeStage::Actors).await?;
    let actor_outputs = crate::atlas_actor_index::surreal::receipts_many_in(tx, &keys).await?;
    let mut sql = String::new();
    for index in 0..work.len() * 4 {
        sql.push_str(&format!("SELECT state,output_revision,IF bytes::len(type::bytes(disposition_json))<=1024 {{disposition_json}} ELSE {{NONE}} AS disposition_json FROM ONLY $key{index};"));
    }
    if sql.is_empty() {
        return Ok(Vec::new());
    }
    let mut query = tx_query(tx, sql);
    for (index, work) in work.iter().enumerate() {
        for (stage_index, stage) in DerivativeStage::ALL.iter().enumerate() {
            query = query.bind((
                format!("key{}", index * 4 + stage_index),
                derivative_record(&work.receipt_key, *stage),
            ));
        }
    }
    let mut response = query.await?.check()?;
    let mut out = Vec::new();
    for (index, work) in work.iter().cloned().enumerate() {
        let mut stages = Vec::new();
        let mut complete = true;
        for (stage_index, stage) in DerivativeStage::ALL.into_iter().enumerate() {
            let row: Option<DerivativeStateRow> = response.take(index * 4 + stage_index)?;
            let snapshot = current
                .get(&work.occurrence)
                .filter(|(snapshot, _, _)| snapshot.receipt_key == work.receipt_key);
            let revision = snapshot
                .map(|(snapshot, _, _)| snapshot.revision(stage))
                .transpose()?;
            let disposition = match row {
                None => None,
                Some(row) if revision.as_deref() != Some(row.output_revision.as_str()) => {
                    Some(DerivativeDisposition::StaleInput)
                }
                Some(row) => {
                    let json = row
                        .disposition_json
                        .ok_or_else(|| TxError::Abort("derivative_disposition_oversized".into()))?;
                    if json.is_empty() {
                        None
                    } else {
                        let disposition: DerivativeDisposition = serde_json::from_str(&json)
                            .map_err(|_| TxError::Abort("derivative_disposition_invalid".into()))?;
                        if row.state == "complete" && !disposition.terminal() {
                            return Err(TxError::Abort(
                                "derivative_disposition_state_mismatch".into(),
                            ));
                        }
                        if row.state != "complete" && disposition.terminal() {
                            Some(DerivativeDisposition::Retryable {
                                code: "native_work_pending".into(),
                            })
                        } else if stage == DerivativeStage::Actors
                            && disposition == DerivativeDisposition::Current
                            && actor_outputs.get(&work.occurrence) != revision.as_ref()
                        {
                            Some(DerivativeDisposition::Retryable {
                                code: "actor_output_missing_or_stale".into(),
                            })
                        } else {
                            Some(disposition)
                        }
                    }
                }
            };
            complete &= disposition
                .as_ref()
                .is_some_and(DerivativeDisposition::terminal);
            stages.push(DerivativeStageState { stage, disposition });
        }
        out.push(DerivativeWorkState {
            work,
            stages,
            complete,
        });
    }
    Ok(out)
}

/// Composite identity matches the original publication SQL array key.
pub(crate) fn derivative_record(
    receipt: &str,
    stage: DerivativeStage,
) -> surrealdb::types::RecordId {
    surrealdb::types::RecordId::new(
        "atlas_classification_derivative",
        surrealdb::types::Array::from(vec![receipt.to_owned(), stage.as_str().to_owned()]),
    )
}

#[derive(Debug, Clone, Default)]
pub(crate) struct NativeRunSummary {
    pub pending: u64,
    pub earliest_phase: Option<u8>,
    pub entities: u64,
    pub publishers: u64,
    pub actors: u64,
    pub related: u64,
}
pub(crate) async fn native_run_summary(
    repo: &Repository,
    run: &str,
) -> Result<NativeRunSummary, AdmissionError> {
    let current = super::atlas::CURRENT_ADMISSION_FILTER;
    let identity="string::concat(<string>bytes::len(type::bytes($parent.run_id)),':',$parent.run_id,'|',<string>bytes::len(type::bytes($parent.article_id)),':',$parent.article_id)";
    let sql=format!("SELECT stage,count() AS total FROM atlas_classification_derivative WHERE run_id=$run AND state!='complete' AND receipt_key=(SELECT VALUE receipt_key FROM ONLY type::record('atlas_classification_projection',{identity})) GROUP BY stage;SELECT count() AS total FROM atlas_article_metadata WHERE run_id=$run AND entity_state='complete' AND count(SELECT id FROM atlas_article WHERE id=type::record('atlas_article',{identity}) AND ({current}) LIMIT 1)>0 GROUP ALL;SELECT count() AS total FROM atlas_article_metadata WHERE run_id=$run AND reliability_state='complete' AND count(SELECT id FROM atlas_article WHERE id=type::record('atlas_article',{identity}) AND ({current}) LIMIT 1)>0 GROUP ALL;SELECT count() AS total FROM atlas_classification_derivative WHERE run_id=$run AND stage='actors' AND state='complete' AND disposition_json='{{\"kind\":\"current\"}}' GROUP ALL;SELECT count() AS total FROM atlas_related_article WHERE run_id=$run GROUP ALL;");
    #[derive(SurrealValue)]
    struct StageCount {
        stage: String,
        total: i64,
    }
    let mut response = db_query(repo.db(), sql)
        .bind(("run", run.to_owned()))
        .bind(("taxonomy", crate::atlas_taxonomy::TAXONOMY_VERSION))
        .bind(("publication", PUBLICATION_VERSION))
        .await
        .map_err(TxError::from)?
        .check()
        .map_err(TxError::from)?;
    let pending: Vec<StageCount> = response.take(0).map_err(TxError::from)?;
    let mut summary = NativeRunSummary::default();
    for row in pending {
        let count =
            u64::try_from(row.total).map_err(|_| TxError::Abort("negative_native_count".into()))?;
        summary.pending = summary.pending.saturating_add(count);
        let phase = match row.stage.as_str() {
            "metadata" => 4,
            "related" => 5,
            "actors" | "reconcile" => 6,
            _ => {
                return Err(AdmissionError::InvalidPage {
                    code: "invalid_native_stage".into(),
                })
            }
        };
        summary.earliest_phase = Some(summary.earliest_phase.map_or(phase, |old| old.min(phase)));
    }
    let mut counts = [0u64; 4];
    for (index, count) in counts.iter_mut().enumerate() {
        let rows: Vec<AggregateCount> = response.take(index + 1).map_err(TxError::from)?;
        if rows.len() > 1 {
            return Err(TxError::Abort("native_count_shape".into()).into());
        }
        *count = rows
            .first()
            .map(|row| u64::try_from(row.total))
            .transpose()
            .map_err(|_| TxError::Abort("negative_native_count".into()))?
            .unwrap_or(0);
    }
    summary.entities = counts[0];
    summary.publishers = counts[1];
    summary.actors = counts[2];
    summary.related = counts[3];
    Ok(summary)
}

/// Coverage uses the accepted immutable retained scan boundary. An absent upper
/// key means the accepted corpus was empty, never an unbounded live query.
pub(crate) async fn coverage_retained_scope(
    repo: &Repository,
    scope: &RetainedPublicationScope,
    cutoff: &str,
    upper: Option<&OccurrenceKey>,
) -> Result<TaxonomyCoverage, AdmissionError> {
    coverage_filtered(
        repo,
        &CoverageFilter {
            run: match scope {
                RetainedPublicationScope::AllRetained => None,
                RetainedPublicationScope::Run { run_id } => Some(run_id.clone()),
            },
            cutoff: Some(cutoff.into()),
            upper: upper.cloned(),
            empty: upper.is_none(),
            ..CoverageFilter::default()
        },
    )
    .await
}

/// Accepted-scope coverage in the same request/checkpoint transaction.
pub(crate) async fn coverage_retained_scope_in(
    tx: &super::tx::Tx,
    scope: &RetainedPublicationScope,
    cutoff: &str,
    upper: Option<&OccurrenceKey>,
) -> Result<TaxonomyCoverage, TxError> {
    coverage_filtered_read(
        CoverageReader::Tx(tx),
        &CoverageFilter {
            run: match scope {
                RetainedPublicationScope::AllRetained => None,
                RetainedPublicationScope::Run { run_id } => Some(run_id.clone()),
            },
            cutoff: Some(cutoff.into()),
            upper: upper.cloned(),
            empty: upper.is_none(),
            ..CoverageFilter::default()
        },
    )
    .await
    .map_err(|error| match error {
        AdmissionError::Storage(error) => error,
        _ => TxError::Abort("invalid_retained_coverage_scope".into()),
    })
}

/// Existing native refresh authority. Only the private captured constructor
/// creates it; it cannot authorize classifier, model, run or admission writes.
#[derive(Debug, Clone)]
pub struct NativeRefreshFence {
    identity: String,
    sources: std::collections::BTreeMap<OccurrenceKey, (RetainedSourceFence, String)>,
}
/// Captures actual current admitted receipts under the source guard before
/// acquiring bounded owned derivative leases. Historical/pending/rejected
/// inputs are omitted explicitly, never relabelled or indexed as current.
pub(crate) async fn native_refresh_work(
    owner: &AdmissionOwner,
    keys: &[OccurrenceKey],
) -> Result<(DerivativeFence, Vec<DerivativeWork>), AdmissionError> {
    if keys.len() > 64 {
        return Err(AdmissionError::InvalidPage {
            code: "native_refresh_key_bound".into(),
        });
    }
    let keys = keys.to_vec();
    let captured = owned_tx(owner, RetryPolicy::default(), move |tx| {
        let keys = keys.clone();
        async move { derivative_snapshots_in(&tx, &keys, DerivativeStage::Metadata).await }
    })
    .await?
    .value;
    let mut sources = std::collections::BTreeMap::new();
    let mut work = Vec::new();
    for (key, (snapshot, _, _)) in captured {
        if snapshot.admission_state != "admitted" {
            continue;
        }
        sources.insert(key.clone(), (snapshot.source, snapshot.receipt_key.clone()));
        work.push(DerivativeWork::new(key, snapshot.receipt_key));
    }
    Ok((
        DerivativeFence::NativeRefresh(NativeRefreshFence {
            identity: format!("native-refresh-{}", crate::store::new_id()),
            sources,
        }),
        work,
    ))
}

/// One explicitly requested compatibility refresh over current retained source.
/// All publication still flows through the same leased native stage processors.
pub(crate) async fn process_native_refresh_stage(
    owner: &AdmissionOwner,
    keys: &[OccurrenceKey],
    stage: DerivativeStage,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<DerivativeBatchReport, AdmissionError> {
    let (fence, work) = native_refresh_work(owner, keys).await?;
    if work.len() != keys.len() {
        return Err(AdmissionError::InvalidRunState {
            code: "native_refresh_current_admission_required".into(),
        });
    }
    let mut accumulated = Vec::new();
    loop {
        let mut report = process_derivative_stage(owner, &fence, &work, stage, cancel).await?;
        let continuing=report.items.iter().any(|item|matches!(&item.disposition,DerivativeDisposition::Retryable {code} if code.ends_with("continuation")));
        accumulated.append(&mut report.items);
        let complete = report.states.iter().all(|work| {
            work.stages
                .iter()
                .find(|state| state.stage == stage)
                .and_then(|state| state.disposition.as_ref())
                .is_some_and(DerivativeDisposition::terminal)
        });
        if complete || cancel.load(Ordering::Acquire) || !continuing {
            report.items = accumulated;
            return Ok(report);
        }
    }
}

fn validate_derivative_work(work: &[DerivativeWork]) -> Result<(), AdmissionError> {
    let mut seen = std::collections::BTreeSet::new();
    if work.len() > 64
        || work.iter().any(|work| {
            !crate::intel_service::valid_id(&work.occurrence.run_id)
                || !crate::intel_service::valid_id(&work.occurrence.article_id)
                || work.receipt_key.is_empty()
                || work.receipt_key.len() > 4096
                || work.receipt_key.chars().any(char::is_control)
                || !seen.insert(work.occurrence.clone())
        })
    {
        return Err(AdmissionError::InvalidPage {
            code: "invalid_or_duplicate_derivative_work".into(),
        });
    }
    Ok(())
}
/// One authoritative Related material fingerprint, shared by stored pair reads
/// and topology consumers. Both ordered domains and the immutable current
/// receipt/publication/source/native metadata identities are included.
pub(crate) async fn native_related_revisions_many_in(
    tx: &super::tx::Tx,
    keys: &[OccurrenceKey],
) -> Result<std::collections::BTreeMap<OccurrenceKey, String>, TxError> {
    let rows = derivative_snapshots_in(tx, keys, DerivativeStage::Related).await?;
    let mut out = std::collections::BTreeMap::new();
    for (key, (snapshot, _, _)) in rows {
        if snapshot.admission_state == "admitted" {
            out.insert(key, snapshot.revision(DerivativeStage::Related)?);
        }
    }
    Ok(out)
}

/// Bounded current-projection stage work. A durable continuation remains
/// eligible in this activation; an ordinary failed input waits for a new one.
pub(crate) async fn cycle_derivative_work(
    owner: &AdmissionOwner,
    run: &RunClaim,
    stage: DerivativeStage,
) -> Result<Vec<DerivativeWork>, AdmissionError> {
    #[derive(SurrealValue)]
    struct WorkRow {
        run_id: String,
        article_id: String,
        receipt_key: String,
    }
    let activation = codec::encode(&[&run.owner, &run.epoch.to_string()]);
    let mut response=owner.query("SELECT run_id,article_id,receipt_key FROM atlas_classification_derivative WHERE run_id=$run AND stage=$stage AND state!='complete' AND (owner='' OR lease_until=NONE OR lease_until<= <datetime>$now) AND ((last_activation ?? '')!=$activation OR last_error='' OR checkpoint_json!='') AND receipt_key=(SELECT VALUE receipt_key FROM ONLY type::record('atlas_classification_projection',string::concat(<string>bytes::len(type::bytes($parent.run_id)),':',$parent.run_id,'|',<string>bytes::len(type::bytes($parent.article_id)),':',$parent.article_id))) ORDER BY article_id LIMIT 64;")
        .bind(("run",run.run_id.clone())).bind(("stage",stage.as_str())).bind(("activation",activation)).bind(("now",Utc::now().to_rfc3339())).await.map_err(TxError::from)?.check().map_err(TxError::from)?;
    let rows: Vec<WorkRow> = response.take(0).map_err(TxError::from)?;
    Ok(rows
        .into_iter()
        .map(|row| {
            DerivativeWork::new(
                OccurrenceKey::new(row.run_id, row.article_id),
                row.receipt_key,
            )
        })
        .collect())
}

/// Bounded current-source handoff for later native input changes. Historical or
/// rejected occurrences cannot gain new admission authority through this path.
pub(crate) async fn refresh_handoff_in(
    tx: &super::tx::Tx,
    keys: &[OccurrenceKey],
    trigger: &str,
    now: DateTime<Utc>,
) -> Result<(), TxError> {
    if keys.len() > 8 {
        return Err(TxError::Abort("derivative_refresh_page_bound".into()));
    }
    let snapshots = derivative_snapshots_in(tx, keys, DerivativeStage::Actors).await?;
    for (key, (snapshot, _, _)) in snapshots {
        if snapshot.admission_state != "admitted" {
            continue;
        }
        super::atlas_derivative_status::reset_in(
            tx,
            &key,
            &snapshot.receipt_key,
            &[DerivativeStage::Actors, DerivativeStage::Reconcile],
        )
        .await?;
        crate::atlas_derivative_worker::enqueue_revision_in(
            tx,
            &key,
            &snapshot.receipt_key,
            trigger,
            now,
        )
        .await?;
    }
    Ok(())
}

/// A source-fenced actor publication generation, independent of deferred cleanup.
#[derive(Debug, Clone)]
pub(crate) struct NativeActorGeneration {
    pub input_revision: String,
    pub publication_batch: String,
}
/// Readers require both the exact current native revision and its publication
/// stamp. Old inventory can remain stored for bounded reconciliation without
/// remaining visible as a current native relationship.
pub(crate) async fn native_actor_generations_many_in(
    tx: &super::tx::Tx,
    keys: &[OccurrenceKey],
) -> Result<std::collections::BTreeMap<OccurrenceKey, NativeActorGeneration>, TxError> {
    if keys.len() > 64 {
        return Err(TxError::Abort("native_generation_read_bound".into()));
    }
    let snapshots = derivative_snapshots_in(tx, keys, DerivativeStage::Actors).await?;
    let mut result = std::collections::BTreeMap::new();
    if snapshots.is_empty() {
        return Ok(result);
    }
    let mut sql = String::new();
    for index in 0..keys.len() {
        sql.push_str(&format!("SELECT input_revision,index_checksum,actor_coverage,IF bytes::len(type::bytes(publication_batch ?? ''))<=256 {{publication_batch ?? ''}} ELSE {{''}} AS publication_batch FROM ONLY $index{index};"));
    }
    let mut query = tx_query(tx, sql);
    for (index, key) in keys.iter().enumerate() {
        query = query.bind((
            format!("index{index}"),
            super::jobs::rid("atlas_article_index", &key.record_id()),
        ));
    }
    let mut response = query.await?.check()?;
    #[derive(SurrealValue)]
    struct Output {
        input_revision: String,
        index_checksum: String,
        actor_coverage: String,
        publication_batch: String,
    }
    for (index, key) in keys.iter().enumerate() {
        let Some((snapshot, _, _)) = snapshots
            .get(key)
            .filter(|(snapshot, _, _)| snapshot.admission_state == "admitted")
        else {
            continue;
        };
        let Some(output) = response.take::<Option<Output>>(index)? else {
            continue;
        };
        if !output.publication_batch.is_empty()
            && output.input_revision == snapshot.revision(DerivativeStage::Actors)?
            && output.index_checksum
                == crate::atlas_actor_index::checksum(
                    &output.input_revision,
                    &output.actor_coverage,
                )
        {
            result.insert(
                key.clone(),
                NativeActorGeneration {
                    input_revision: output.input_revision,
                    publication_batch: output.publication_batch,
                },
            );
        }
    }
    Ok(result)
}

#[cfg(all(test, feature = "embedded"))]
mod derivative_release_tests {
    use super::*;
    #[tokio::test]
    async fn lost_root_releases_only_its_live_child_and_preserves_uncertain_commit() {
        let directory = tempfile::tempdir().unwrap();
        let repo = super::super::repo::Repository::connect(
            &super::super::open::Target::Embedded {
                path: directory.path().join("storage"),
            },
            "argos",
            "derivative_release",
        )
        .await
        .unwrap();
        let owner = AdmissionOwner::new(repo.clone());
        let key = OccurrenceKey::new("revoked-root", "fixture");
        // Intentionally unadmitted: these rows test cleanup ownership, never
        // successful native publication or source authority.
        let snapshot = DerivativeSnapshot {
            source: RetainedSourceFence {
                occurrence: key.clone(),
                native_input_revision: 1,
                source_hash: "s".into(),
                classifier_input_hash: "i".into(),
                provenance: OccurrenceProvenance {
                    provider: "fixture".into(),
                    url: String::new(),
                    source_domain: String::new(),
                    source_name: String::new(),
                    published_at: String::new(),
                    seen_at: String::new(),
                    language: None,
                    observation_ids: Vec::new(),
                },
            },
            receipt_key: "fixture".into(),
            admission_state: "pending".into(),
            metadata_sha256: "m".into(),
            body: None,
            catalog_epoch: 0,
        };
        let fence = DerivativeFence::Cycle(RunClaim {
            run_id: key.run_id.clone(),
            owner: "old-owner".into(),
            epoch: 1,
            revision: 1,
        });
        for case in ["owned", "expired", "new-owner", "committed"] {
            let receipt = format!("cleanup-{case}");
            let claim = DerivativeClaim {
                work: DerivativeWork::new(key.clone(), receipt.clone()),
                stage: DerivativeStage::Actors,
                owner: "old-owner".into(),
                epoch: 1,
                snapshot: snapshot.clone(),
                input_revision: "revision".into(),
                checkpoint_json: "checkpoint".into(),
                article: AtlasArticleRow::default(),
                metadata: None,
            };
            repo.db().query("CREATE $key SET run_id=$run,article_id='fixture',stage='actors',input_revision=1,receipt_key=$receipt,state=$state,owner=$owner,epoch=$epoch,output_revision='revision',checkpoint_json='checkpoint',disposition_json=$disposition,lease_until=<datetime>$until,updated_at=time::now();")
                .bind(("key",derivative_record(&receipt,DerivativeStage::Actors))).bind(("run",key.run_id.clone())).bind(("receipt",receipt.clone()))
                .bind(("state",if case=="committed" {"complete"}else{"evaluating"})).bind(("owner",if case=="new-owner" {"new-owner"}else if case=="committed" {""}else{"old-owner"}))
                .bind(("epoch",if case=="new-owner" {2i64}else{1i64})).bind(("disposition",if case=="committed" {"{\"kind\":\"current\"}"}else{""}))
                .bind(("until",(Utc::now()+Duration::seconds(if case=="expired" {-60}else{60})).to_rfc3339())).await.unwrap().check().unwrap();
            let result = release_derivative(
                &owner,
                &fence,
                &claim,
                DerivativeDisposition::Retryable {
                    code: "cancelled".into(),
                },
            )
            .await
            .unwrap();
            assert_eq!(
                result,
                if case == "owned" {
                    DerivativeDisposition::Retryable {
                        code: "cancelled".into(),
                    }
                } else {
                    DerivativeDisposition::StaleInput
                }
            );
            #[derive(SurrealValue)]
            struct State {
                owner: String,
                state: String,
                checkpoint_json: String,
                disposition_json: String,
            }
            let state: State = repo
                .db()
                .query("SELECT owner,state,checkpoint_json,disposition_json FROM ONLY $key;")
                .bind(("key", derivative_record(&receipt, DerivativeStage::Actors)))
                .await
                .unwrap()
                .check()
                .unwrap()
                .take::<Option<State>>(0)
                .unwrap()
                .unwrap();
            assert_eq!(state.checkpoint_json, "checkpoint");
            assert_eq!(
                state.owner,
                if case == "owned" || case == "committed" {
                    ""
                } else if case == "new-owner" {
                    "new-owner"
                } else {
                    "old-owner"
                }
            );
            assert_eq!(
                state.state,
                if case == "owned" {
                    "pending"
                } else if case == "committed" {
                    "complete"
                } else {
                    "evaluating"
                }
            );
            if case == "committed" {
                assert_eq!(state.disposition_json, "{\"kind\":\"current\"}");
            }
        }
        owner.shutdown().await.unwrap();
        repo.publisher().shutdown().await;
    }
}
