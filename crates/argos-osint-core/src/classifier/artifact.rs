//! Immutable classifier artifact store: manifests, checksums, validation and
//! the atomic active pointer.
//!
//! Layout under `<ARGOS_HOME>/classifier/`:
//!
//! ```text
//! classifier/
//! ├── active.json                       the atomic active pointer (a ModelManifest)
//! └── models/<model_id>/
//!     ├── manifest.json                 immutable identity manifest
//!     ├── model.json                    data-only model payload
//!     └── metrics.json                  training/evaluation report
//! ```
//!
//! Self-checksum rule (no recursion): the immutable `manifest.json` hashes
//! its payloads but never itself; the active pointer records
//! [`ModelManifest::manifest_sha256`] — the SHA-256 of the exact stored
//! `manifest.json` bytes. Model ids are validated path segments, and the
//! payload is data-only JSON: prototype-pollution keys are rejected and no
//! executable code is ever restored.
//!
//! Failure and cancellation during activation leave the prior active
//! artifact intact: candidate files are written and synced first, then the
//! pointer is replaced by temp-file + fsync + rename with a parent-directory
//! sync. Prior models are always retained.

pub use super::calibration::{
    authorize_activation, ActivationAuthorization, ActivationAuthorizationDecision,
    ActivationOutcome,
};
use super::training::TrainingReport;
use super::{
    ClassifierError, Provenance, ADMISSION_POLICY_VERSION, ALGORITHM_VERSION, PREPROCESSING_VERSION,
};
use crate::atlas_taxonomy::{DomainId, TAXONOMY_VERSION};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use tempfile::NamedTempFile;

/// Version of the manifest schema written by this contract. Artifacts
/// declaring another schema are [`ClassifierError::Incompatible`].
pub const MANIFEST_SCHEMA_VERSION: &str = "classifier-manifest-v1";

/// Version of the data-only model payload envelope checked by
/// [`validate_artifact`].
pub const PAYLOAD_SCHEMA_VERSION: &str = "classifier-payload-v1";

/// Version of the per-domain feature-schema contract (explicit feature
/// order) shared by the payload and this manifest.
pub const FEATURE_SCHEMA_VERSION: &str = "classifier-features-v2";

/// Upper bound for `manifest.json` / `active.json` files.
pub const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;

/// Upper bound for the `model.json` payload.
pub const MAX_ARTIFACT_BYTES: u64 = 64 * 1024 * 1024;

/// Upper bound for a single model id (safe path segment).
pub const MAX_MODEL_ID_LEN: usize = 64;

/// Upper bound for one domain's feature-order array (vocabulary bound).
pub const MAX_FEATURES_PER_DOMAIN: usize = 1_000_000;

/// Legacy placeholder constant retained for compatibility.
pub const NO_BUNDLE_SHA256: &str = "no-classifier-bundle-compiled";

/// Real SHA-256 of the compiled Natural classifier bundle.
pub const COMPILED_BUNDLE_SHA256: &str =
    "7aae167cdb6a27473be18f89ba88e5ede39a27b2252f3a216026a770447a047a";

/// Returns the SHA-256 of the Natural classifier bundle compiled into this build.
#[must_use]
pub fn compiled_bundle_sha256() -> &'static str {
    COMPILED_BUNDLE_SHA256
}

/// File name of the immutable identity manifest inside a model directory.
pub const MODEL_MANIFEST_FILE: &str = "manifest.json";
/// File name of the data-only model payload inside a model directory.
pub const MODEL_PAYLOAD_FILE: &str = "model.json";
/// File name of the training/evaluation report inside a model directory.
pub const MODEL_REPORT_FILE: &str = "metrics.json";

const ACTIVE_FILE: &str = "active.json";

/// Top-level keys a model payload may carry. Anything else is rejected:
/// the payload is data only.
const PAYLOAD_TOP_LEVEL_KEYS: [&str; 7] = [
    "schema_version",
    "feature_schema_version",
    "bridge_version",
    "taxonomy_version",
    "algorithm_version",
    "preprocessing_version",
    "domains",
];

/// Object keys that would enable prototype pollution or code smuggling when
/// consumed by JavaScript. Rejected anywhere inside the payload.
const FORBIDDEN_KEYS: [&str; 3] = ["__proto__", "prototype", "constructor"];

/// Validated model id: a safe path segment (ASCII alphanumerics, `-`, `_`
/// and `.`, no `.`/`..`, bounded length) so it can never escape
/// `<classifier_dir>/models/`. Serialized as its string form.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ModelId(String);

impl ModelId {
    /// Parses a model id, enforcing the safe path-segment rules.
    #[must_use]
    pub fn parse(value: &str) -> Option<ModelId> {
        if value.is_empty()
            || value.len() > MAX_MODEL_ID_LEN
            || value == "."
            || value == ".."
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            return None;
        }
        Some(ModelId(value.to_string()))
    }

    /// The validated id as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ModelId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for ModelId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ModelId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        ModelId::parse(&value)
            .ok_or_else(|| serde::de::Error::custom(format!("invalid model id `{value}`")))
    }
}

/// Why a model became active. Manual seed activation is explicit and always
/// carries visible synthetic provenance; automatic promotion never happens
/// on insufficient evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ActivationReason {
    /// Explicit operator activation of the synthetic seed model.
    ManualSeed,
    /// Explicit rollback to the previously active model.
    ManualRollback,
    /// Automatic promotion after independent reviewed-real calibration met
    /// every quality gate.
    AutomaticPromotion,
}

/// Per-domain admission threshold (calibrated score scale).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct DomainThreshold {
    pub domain: DomainId,
    pub threshold: f64,
}

/// Per-domain training class support for the binary `present`/`absent`
/// model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct DomainClassCount {
    pub domain: DomainId,
    pub present: u64,
    pub absent: u64,
}

/// Per-domain count of independent accepted calibration scenario families.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct DomainFamilyCount {
    pub domain: DomainId,
    pub accepted_families: u64,
}

/// Full manifest for one classifier model. One type serves both copies:
/// the immutable `models/<id>/manifest.json` (activation fields are
/// placeholders there) and the atomic `active.json` pointer (authoritative
/// activation fields, including the checksum of the immutable manifest
/// bytes).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ModelManifest {
    /// Manifest schema version; see [`MANIFEST_SCHEMA_VERSION`].
    pub schema_version: String,
    /// Active-pointer generation, starting at 1; immutable copies carry 0.
    pub activation_generation: u64,
    pub model_id: ModelId,
    /// Model active before this one, recorded for [`ArtifactStore::rollback`].
    pub previous_model_id: Option<ModelId>,
    /// RFC 3339 activation timestamp; empty in immutable copies.
    pub activated_at: String,
    /// SHA-256 of `model.json` (the payload).
    pub model_sha256: String,
    /// SHA-256 of the immutable `manifest.json` bytes, recorded by the
    /// active pointer. Empty in the immutable copy itself (self-reference).
    pub manifest_sha256: String,
    /// SHA-256 of `metrics.json`.
    pub report_sha256: String,
    /// SHA-256 of the full training dataset file.
    pub dataset_sha256: String,
    /// SHA-256 of the train split (family-grouped).
    pub train_split_sha256: String,
    /// SHA-256 of the calibration split.
    pub calibration_split_sha256: String,
    /// SHA-256 of the test split.
    pub test_split_sha256: String,
    /// SHA-256 of the family-assignment table (permanent partitions).
    pub family_assignment_sha256: String,
    /// Taxonomy identity; see `crate::atlas_taxonomy::TAXONOMY_VERSION`.
    pub taxonomy_version: String,
    /// Exactly the six canonical domains, in [`DomainId::ALL`] order.
    pub domains: Vec<DomainId>,
    /// Algorithm identity; see `super::ALGORITHM_VERSION`.
    pub algorithm_version: String,
    /// Preprocessing identity; see `super::PREPROCESSING_VERSION`.
    pub preprocessing_version: String,
    /// Feature-schema identity; see [`FEATURE_SCHEMA_VERSION`].
    pub feature_schema_version: String,
    /// Admission-policy identity; see `super::ADMISSION_POLICY_VERSION`.
    pub admission_policy_version: String,
    /// Pinned Natural version the bundle was built from.
    pub natural_version: String,
    /// Pinned apparatus version the bundle was built from.
    pub apparatus_version: String,
    /// SHA-256 of the compiled JS bundle; must match
    /// [`compiled_bundle_sha256`].
    pub bundle_sha256: String,
    /// Version of the fixed JavaScript bridge protocol.
    pub bridge_version: String,
    /// SHA-256 of the fixed JavaScript bridge.
    pub bridge_sha256: String,
    /// SHA-256 of the adaptation source.
    pub adapter_sha256: String,
    /// SHA-256 of the pinned upstream source manifest.
    pub source_manifest_sha256: String,
    /// Additive smoothing fixed at1.0 by the compiled Bayes algorithm.
    pub smoothing: f64,
    /// Six per-domain thresholds in canonical domain order.
    pub thresholds: Vec<DomainThreshold>,
    /// Six per-domain class counts in canonical domain order.
    pub class_counts: Vec<DomainClassCount>,
    /// Six per-domain independent family counts in canonical domain order.
    pub family_counts: Vec<DomainFamilyCount>,
    /// Training/evaluation metrics object. The training unit owns the
    /// shape; it must be a JSON object and is hashed via
    /// [`ModelManifest::report_sha256`].
    pub metrics: serde_json::Value,
    /// Visible quality provenance (`Synthetic baseline` / `Reviewed real`).
    pub provenance: Provenance,
    /// Why this model was activated.
    pub activation_reason: ActivationReason,
}

/// A payload that passed [`validate_artifact`], ready for engine restore.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ValidatedArtifact {
    pub model_id: ModelId,
    pub manifest: ModelManifest,
    /// The data-only JSON payload, byte-identical to the stored file.
    pub payload: String,
}

/// Verified persisted identity for explicit replacement of an incompatible prior.
/// This contains no executable model or quality evidence and cannot authorize
/// automatic promotion. Only the store can construct it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PriorActivationIdentity {
    model_id: ModelId,
    activation_generation: u64,
    manifest_sha256: String,
}

impl PriorActivationIdentity {
    /// The verified prior model identifier.
    pub fn model_id(&self) -> &ModelId {
        &self.model_id
    }
    /// The committed activation generation, never reset during repair.
    pub fn generation(&self) -> u64 {
        self.activation_generation
    }
    /// SHA-256 of the exact immutable manifest bytes.
    pub fn manifest_sha256(&self) -> &str {
        &self.manifest_sha256
    }
}

/// An exclusive advisory OS lock on `<classifier_dir>/writer.lock`.
#[derive(Debug)]
pub struct WriterLease {
    _file: File,
    path: PathBuf,
    identity: u64,
    operation: Mutex<()>,
}

impl WriterLease {
    /// Identifies this acquisition, including after releasing and reacquiring the same lock.
    #[must_use]
    pub fn identity(&self) -> u64 {
        self.identity
    }

    /// The path to the lock file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Schema version for activation history records under `<classifier_dir>/activations/<gen>.json`.
pub const ACTIVATION_RECORD_SCHEMA_VERSION: &str = "classifier-activation-record-v1";

/// Schema version for activated model indices under `<classifier_dir>/activated-models/<id>.json`.
pub const ACTIVATED_MODEL_INDEX_SCHEMA_VERSION: &str = "classifier-activated-model-v1";

/// Durable record of an atomic activation event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivationRecord {
    pub schema_version: String,
    pub generation: u64,
    pub model_id: ModelId,
    pub model_sha256: String,
    pub immutable_manifest_sha256: String,
    pub previous_model_id: Option<ModelId>,
    pub previous_generation: Option<u64>,
    pub reason: ActivationReason,
    pub activated_at: String,
    pub committed: bool,
}

/// Durable index recording that a candidate model has been previously activated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivatedModelIndex {
    pub schema_version: String,
    pub model_id: ModelId,
    pub generation: u64,
    pub activation_record_sha256: String,
}

#[derive(Clone, Copy)]
enum ActiveWriteFault {
    None,
    #[cfg(test)]
    BeforeRename,
    #[cfg(test)]
    AfterRename,
}
impl ActiveWriteFault {
    fn before_rename(self) -> bool {
        match self {
            #[cfg(test)]
            Self::BeforeRename => true,
            _ => false,
        }
    }
    fn after_rename(self) -> bool {
        match self {
            #[cfg(test)]
            Self::AfterRename => true,
            _ => false,
        }
    }
}

/// Filesystem store for immutable classifier artifacts under

/// `<classifier_dir>/`.
#[derive(Debug, Clone)]
pub struct ArtifactStore {
    root: PathBuf,
}

/// Resolves the configured artifact directory without creating it.
pub fn root_for(home: &Path) -> Result<PathBuf, ClassifierError> {
    crate::data_paths::DataPaths::load_from_home(home)
        .map(|paths| paths.classifier_root(home))
        .map_err(|error| {
            ClassifierError::Io(format!("read classifier path configuration: {error}"))
        })
}

impl ArtifactStore {
    fn require_empty_activation_history(&self) -> Result<(), ClassifierError> {
        for directory in ["activations", "activated-models"] {
            let path = self.root.join(directory);
            let mut entries = match fs::read_dir(&path) {
                Ok(entries) => entries,
                Err(error) if error.kind() == ErrorKind::NotFound => continue,
                Err(error) => {
                    return Err(ClassifierError::Io(format!(
                        "read {}: {error}",
                        path.display()
                    )))
                }
            };
            if let Some(entry) = entries.next() {
                entry.map_err(|error| {
                    ClassifierError::Io(format!("read {}: {error}", path.display()))
                })?;
                return Err(ClassifierError::InvalidArtifact("active pointer is absent but activation history exists; explicit recovery_required".into()));
            }
        }
        Ok(())
    }

    fn require_unused_history_slot(&self, generation: u64) -> Result<(), ClassifierError> {
        let path = self
            .root
            .join("activations")
            .join(format!("{generation}.json"));
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(error) => Err(ClassifierError::Io(format!(
                "inspect {}: {error}",
                path.display()
            ))),
            Ok(_) => Err(ClassifierError::InvalidArtifact(
                "next activation history generation is occupied; explicit recovery_required".into(),
            )),
        }
    }

    /// Verifies persisted prior identity and committed lineage without executing it.
    /// This permits explicit replacement of a known manifest format from another
    /// build; it never supplies compatible inference or automatic quality evidence.
    pub fn read_active_repair_identity(
        &self,
    ) -> Result<Option<PriorActivationIdentity>, ClassifierError> {
        let path = self.active_manifest_path();
        let file = match File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                self.require_empty_activation_history()?;
                return Ok(None);
            }
            Err(error) => {
                return Err(ClassifierError::Io(format!(
                    "open {}: {error}",
                    path.display()
                )))
            }
        };
        let active_bytes = read_bounded_file(file, &path, MAX_MANIFEST_BYTES)?;
        let active = parse_repair_manifest(&active_bytes, false)?;
        let immutable_bytes = self.read_manifest_raw(&active.header.model_id)?;
        let immutable = parse_repair_manifest(&immutable_bytes, true)?;
        let immutable_sha256 = sha256_hex(&immutable_bytes);
        if active.header.manifest_sha256 != immutable_sha256
            || active.header.model_id != immutable.header.model_id
        {
            return Err(ClassifierError::InvalidArtifact(
                "repair prior immutable model_id/checksum mismatch".into(),
            ));
        }
        if active.immutable_fields != immutable.immutable_fields {
            return Err(ClassifierError::InvalidArtifact(
                "repair prior active/immutable identity fields differ".into(),
            ));
        }
        if sha256_hex(self.read_payload(&active.header.model_id)?.as_bytes())
            != active.header.model_sha256
            || sha256_hex(&self.read_report_raw(&active.header.model_id)?)
                != active.header.report_sha256
        {
            return Err(ClassifierError::InvalidArtifact(
                "repair prior payload/report checksum mismatch".into(),
            ));
        }
        let record_bytes = read_bounded(
            &self
                .root
                .join("activations")
                .join(format!("{}.json", active.header.activation_generation)),
            MAX_MANIFEST_BYTES,
        )?;
        let record: ActivationRecord = parse_repair_record(&record_bytes)?;
        let index_bytes = read_bounded(
            &self
                .root
                .join("activated-models")
                .join(format!("{}.json", active.header.model_id)),
            MAX_MANIFEST_BYTES,
        )?;
        let index_value = parse_strict_json(&index_bytes).map_err(|error| {
            ClassifierError::InvalidArtifact(format!("repair index JSON: {error}"))
        })?;
        assert_data_only(&index_value)?;
        let index: ActivatedModelIndex = serde_json::from_value(index_value).map_err(|error| {
            ClassifierError::InvalidArtifact(format!("repair index schema: {error}"))
        })?;
        if index.schema_version != ACTIVATED_MODEL_INDEX_SCHEMA_VERSION
            || index.model_id != active.header.model_id
            || index.generation != active.header.activation_generation
            || index.activation_record_sha256 != sha256_hex(&record_bytes)
            || record.model_id != active.header.model_id
            || record.generation != active.header.activation_generation
            || record.model_sha256 != active.header.model_sha256
            || record.immutable_manifest_sha256 != immutable_sha256
            || record.previous_model_id != active.header.previous_model_id
            || record.reason != active.header.activation_reason
            || record.activated_at != active.header.activated_at
        {
            return Err(ClassifierError::InvalidArtifact(
                "repair prior activation history/index identity mismatch".into(),
            ));
        }
        match (
            record.generation,
            record.previous_generation,
            record.previous_model_id.as_ref(),
        ) {
            (1, None, None) => {}
            (generation, Some(previous), Some(previous_id))
                if previous.checked_add(1) == Some(generation) =>
            {
                let previous_bytes = read_bounded(
                    &self
                        .root
                        .join("activations")
                        .join(format!("{previous}.json")),
                    MAX_MANIFEST_BYTES,
                )?;
                let previous_record: ActivationRecord = parse_repair_record(&previous_bytes)?;
                if previous_record.generation != previous
                    || &previous_record.model_id != previous_id
                {
                    return Err(ClassifierError::InvalidArtifact(
                        "repair prior previous activation lineage mismatch".into(),
                    ));
                }
            }
            _ => {
                return Err(ClassifierError::InvalidArtifact(
                    "repair prior activation generation/lineage mismatch".into(),
                ))
            }
        }
        Ok(Some(PriorActivationIdentity {
            model_id: active.header.model_id,
            activation_generation: active.header.activation_generation,
            manifest_sha256: immutable_sha256,
        }))
    }

    /// Issues an identity-bound capability only for explicitly consented synthetic repair.
    pub fn authorize_manual_repair(
        &self,
        lease: &WriterLease,
        candidate: &ValidatedArtifact,
        report: &TrainingReport,
        request: &super::training::TrainRequest,
    ) -> Result<ActivationAuthorizationDecision, ClassifierError> {
        self.validate_lease(lease)?;
        if !matches!(request.provenance, Provenance::SyntheticBaseline { .. })
            || request.synthetic_activation != super::training::SyntheticActivation::ExplicitOptIn
        {
            return Err(ClassifierError::ActivationDenied {
                reason: "manual repair requires explicit synthetic baseline opt-in".into(),
            });
        }
        let prior = self.read_active_repair_identity()?;
        let mut decision = authorize_activation(lease, candidate, report, request, None, None)?;
        if let ActivationAuthorizationDecision::Authorized(authorization) = &mut decision {
            authorization.expected_prior_id =
                prior.as_ref().map(|prior| prior.model_id.as_str().into());
            authorization.expected_prior_generation =
                prior.as_ref().map(|prior| prior.activation_generation);
            authorization.manual_repair_prior_sha256 = prior.map(|prior| prior.manifest_sha256);
        }
        Ok(decision)
    }

    /// Creates the store rooted at `<classifier_dir>`, creating the
    /// directory (and parents) if needed.
    ///
    /// # Errors
    /// [`ClassifierError::Io`] when the directory cannot be created.
    pub fn new(home: &Path) -> Result<Self, ClassifierError> {
        let root = root_for(home)?;
        fs::create_dir_all(&root)
            .map_err(|error| ClassifierError::Io(format!("create {}: {error}", root.display())))?;
        let root = fs::canonicalize(&root).map_err(|error| {
            ClassifierError::Io(format!("canonicalize {}: {error}", root.display()))
        })?;
        Ok(Self { root })
    }

    /// Acquires an exclusive writer lease on `<classifier_dir>/writer.lock`
    /// without waiting.
    ///
    /// # Errors
    /// [`ClassifierError::Busy`] if another writer currently holds the lease;
    /// [`ClassifierError::Io`] on filesystem faults.
    pub fn try_writer(&self) -> Result<WriterLease, ClassifierError> {
        let path = self.root.join("writer.lock");
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| ClassifierError::Io(format!("create {}: {e}", parent.display())))?;
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|e| ClassifierError::Io(format!("open {}: {e}", path.display())))?;
        match file.try_lock() {
            Ok(()) => {
                static NEXT_LEASE: AtomicU64 = AtomicU64::new(1);
                let identity = NEXT_LEASE
                    .fetch_update(Ordering::AcqRel, Ordering::Acquire, |id| id.checked_add(1))
                    .map_err(|_| {
                        ClassifierError::InvalidArtifact("writer_lease_identity_overflow".into())
                    })?;
                Ok(WriterLease {
                    _file: file,
                    path,
                    identity,
                    operation: Mutex::new(()),
                })
            }
            Err(fs::TryLockError::WouldBlock) => Err(ClassifierError::Busy),
            Err(fs::TryLockError::Error(e)) => {
                Err(ClassifierError::Io(format!("lock {}: {e}", path.display())))
            }
        }
    }

    fn validate_lease(&self, lease: &WriterLease) -> Result<(), ClassifierError> {
        if lease.path != self.root.join("writer.lock") {
            return Err(ClassifierError::ActivationDenied {
                reason: "writer lease belongs to another home".into(),
            });
        }
        Ok(())
    }

    /// The store root (`<classifier_dir>`).
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// `<classifier_dir>/models`.
    #[must_use]
    pub fn models_dir(&self) -> PathBuf {
        self.root.join("models")
    }

    /// `<classifier_dir>/models/<model_id>`.
    #[must_use]
    pub fn model_dir(&self, id: &ModelId) -> PathBuf {
        self.models_dir().join(id.as_str())
    }

    /// `<classifier_dir>/active.json` — the atomic active pointer.
    #[must_use]
    pub fn active_manifest_path(&self) -> PathBuf {
        self.root.join(ACTIVE_FILE)
    }

    /// Reads and verifies the active manifest. `Ok(None)` when no model has
    /// been activated yet.
    ///
    /// # Errors
    /// [`ClassifierError::InvalidArtifact`] for a corrupt pointer, missing
    /// required active fields, or checksum/identity mismatch against stored
    /// immutable manifest; [`ClassifierError::Incompatible`] for foreign versions.
    pub fn read_active(&self) -> Result<Option<ModelManifest>, ClassifierError> {
        let path = self.active_manifest_path();
        let file = match File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                self.require_empty_activation_history()?;
                return Ok(None);
            }
            Err(error) => {
                return Err(ClassifierError::Io(format!(
                    "open {}: {error}",
                    path.display()
                )))
            }
        };
        let bytes = read_bounded_file(file, &path, MAX_MANIFEST_BYTES)?;
        let active = parse_manifest_bytes(&bytes)?;

        // R3: active pointer validation
        if active.activation_generation == 0 {
            return Err(ClassifierError::InvalidArtifact(
                "active manifest must have activation_generation > 0".into(),
            ));
        }
        if active.activated_at.is_empty()
            || chrono::DateTime::parse_from_rfc3339(&active.activated_at).is_err()
        {
            return Err(ClassifierError::InvalidArtifact(
                "active manifest must have non-empty activated_at timestamp".into(),
            ));
        }
        if active.manifest_sha256.is_empty() {
            return Err(ClassifierError::InvalidArtifact(
                "active manifest must carry non-empty manifest_sha256".into(),
            ));
        }

        let raw = self.read_manifest_raw(&active.model_id)?;
        let stored_sha = sha256_hex(&raw);
        if active.manifest_sha256 != stored_sha {
            return Err(ClassifierError::InvalidArtifact(format!(
                "active manifest checksum does not match the stored manifest.json for model `{}`",
                active.model_id
            )));
        }

        let stored = parse_manifest_bytes(&raw)?;
        if stored.model_id != active.model_id
            || stored.schema_version != active.schema_version
            || stored.model_sha256 != active.model_sha256
            || stored.report_sha256 != active.report_sha256
            || stored.dataset_sha256 != active.dataset_sha256
            || stored.train_split_sha256 != active.train_split_sha256
            || stored.calibration_split_sha256 != active.calibration_split_sha256
            || stored.test_split_sha256 != active.test_split_sha256
            || stored.family_assignment_sha256 != active.family_assignment_sha256
            || stored.taxonomy_version != active.taxonomy_version
            || stored.domains != active.domains
            || stored.algorithm_version != active.algorithm_version
            || stored.preprocessing_version != active.preprocessing_version
            || stored.feature_schema_version != active.feature_schema_version
            || stored.admission_policy_version != active.admission_policy_version
            || stored.natural_version != active.natural_version
            || stored.apparatus_version != active.apparatus_version
            || stored.bridge_version != active.bridge_version
            || stored.bridge_sha256 != active.bridge_sha256
            || stored.bundle_sha256 != active.bundle_sha256
            || stored.adapter_sha256 != active.adapter_sha256
            || stored.source_manifest_sha256 != active.source_manifest_sha256
            || stored.smoothing != active.smoothing
            || stored.thresholds != active.thresholds
            || stored.class_counts != active.class_counts
            || stored.family_counts != active.family_counts
            || stored.metrics != active.metrics
            || stored.provenance != active.provenance
        {
            return Err(ClassifierError::InvalidArtifact(format!(
                "active manifest immutable fields do not match the stored manifest for model `{}`",
                active.model_id
            )));
        }

        Ok(Some(active))
    }

    /// Checks active pointer and immutable artifact integrity without initializing JavaScript.
    /// Suitable for launcher readiness probes; activation still performs restore verification.
    pub fn read_active_integrity(&self) -> Result<Option<ModelManifest>, ClassifierError> {
        let Some(manifest) = self.read_active()? else {
            return Ok(None);
        };
        let payload = self.read_payload(&manifest.model_id)?;
        let report = self.read_report_raw(&manifest.model_id)?;
        if sha256_hex(&report) != manifest.report_sha256 {
            return Err(ClassifierError::InvalidArtifact(
                "active report checksum mismatch".into(),
            ));
        }
        validate_artifact(&manifest, &payload, compiled_bundle_sha256())?;
        Ok(Some(manifest))
    }

    /// Reads the immutable manifest of `id`.
    ///
    /// # Errors
    /// [`ClassifierError::InvalidArtifact`] when the file is missing or
    /// corrupt; [`ClassifierError::Incompatible`] for foreign versions.
    pub fn read_manifest(&self, id: &ModelId) -> Result<ModelManifest, ClassifierError> {
        let raw = self.read_manifest_raw(id)?;
        parse_manifest_bytes(&raw)
    }

    /// Every stored model id, sorted. Directory names that are not valid
    /// model ids (for example staging temp dirs) are skipped.
    ///
    /// # Errors
    /// [`ClassifierError::Io`] when the models directory cannot be read.
    pub fn list_models(&self) -> Result<Vec<ModelId>, ClassifierError> {
        let dir = self.models_dir();
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(ClassifierError::Io(format!(
                    "read {}: {error}",
                    dir.display()
                )))
            }
        };
        let mut ids = Vec::new();
        for entry in entries {
            let entry = entry
                .map_err(|error| ClassifierError::Io(format!("read {}: {error}", dir.display())))?;
            if let Some(id) = ModelId::parse(&entry.file_name().to_string_lossy()) {
                ids.push(id);
            }
        }
        ids.sort();
        Ok(ids)
    }

    /// Activates `id` atomically: fully validates the candidate (manifest,
    /// report, payload, bundle compatibility), then replaces the active
    /// pointer via temp file + fsync + rename. The prior model is retained
    /// and recorded as [`ModelManifest::previous_model_id`] for rollback.
    /// Failure or cancellation leaves the prior active artifact intact.
    ///
    /// # Errors
    /// [`ClassifierError::InvalidArtifact`] / [`ClassifierError::Incompatible`]
    /// when validation fails; [`ClassifierError::Io`] on filesystem faults.
    /// Writes an immutable candidate artifact to `<classifier_dir>/models/<id>/`.
    ///
    /// # Errors
    /// Returns [`ClassifierError::CandidateAlreadyExists`] if the model directory already exists,
    /// [`ClassifierError::InvalidArtifact`] if validation fails, or [`ClassifierError::Io`].
    pub fn write_candidate(
        &self,
        lease: &WriterLease,
        manifest: &ModelManifest,
        payload: &str,
        report: &TrainingReport,
    ) -> Result<ValidatedArtifact, ClassifierError> {
        self.validate_lease(lease)?;
        let _operation = lease
            .operation
            .try_lock()
            .map_err(|_| ClassifierError::Busy)?;
        let model_dir = self.model_dir(&manifest.model_id);
        if model_dir.exists() {
            return Err(ClassifierError::CandidateAlreadyExists);
        }

        let report_bytes = bounded_json_bytes(report)?;
        let manifest_bytes = bounded_json_bytes(manifest)?;
        if manifest.report_sha256 != sha256_hex(&report_bytes) {
            return Err(ClassifierError::InvalidArtifact(
                "report checksum does not match candidate report".into(),
            ));
        }

        validate_artifact(manifest, payload, compiled_bundle_sha256())?;

        let models = self.models_dir();
        fs::create_dir_all(&models).map_err(|error| {
            ClassifierError::Io(format!("create {}: {error}", models.display()))
        })?;
        let staging = tempfile::Builder::new()
            .prefix(".candidate~")
            .tempdir_in(&models)
            .map_err(|error| ClassifierError::Io(format!("stage candidate: {error}")))?;
        write_atomic(&staging.path().join(MODEL_REPORT_FILE), &report_bytes)?;
        write_atomic(&staging.path().join(MODEL_PAYLOAD_FILE), payload.as_bytes())?;
        write_atomic(&staging.path().join(MODEL_MANIFEST_FILE), &manifest_bytes)?;
        // The per-home OS lease and acquisition-local operation lock exclude competing publishers.
        // Completed candidates appear as one directory rename; interrupted staging is never listed.
        if model_dir.exists() {
            return Err(ClassifierError::CandidateAlreadyExists);
        }
        fs::rename(staging.path(), &model_dir)
            .map_err(|error| ClassifierError::Io(format!("publish candidate: {error}")))?;
        #[cfg(unix)]
        File::open(&models)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| ClassifierError::Io(format!("sync candidate directory: {error}")))?;

        Ok(ValidatedArtifact {
            model_id: manifest.model_id.clone(),
            manifest: manifest.clone(),
            payload: payload.to_string(),
        })
    }

    /// Activates `id` atomically under an acquired `WriterLease` and validated
    /// `ActivationAuthorization`.
    ///
    /// # Errors
    /// Returns [`ClassifierError::ActivationDenied`] if authorization checks fail,
    /// [`ClassifierError::StaleActivation`] if expected prior baseline changed, or
    /// [`ClassifierError::ActivationCommitUncertain`] if post-rename sync fails.
    pub fn activate(
        &self,
        id: &ModelId,
        authorization: &ActivationAuthorization,
        lease: &WriterLease,
    ) -> Result<ModelManifest, ClassifierError> {
        self.validate_lease(lease)?;
        let _operation = lease
            .operation
            .try_lock()
            .map_err(|_| ClassifierError::Busy)?;
        if authorization.writer_lease_id != lease.identity() {
            return Err(ClassifierError::ActivationDenied {
                reason: "authorization belongs to another writer acquisition".into(),
            });
        }
        if authorization.candidate_model_id != *id {
            return Err(ClassifierError::ActivationDenied {
                reason: format!(
                    "model_id `{id}` does not match authorization target `{}`",
                    authorization.candidate_model_id
                ),
            });
        }

        let current = if let Some(expected_sha256) = &authorization.manual_repair_prior_sha256 {
            if authorization.reason != ActivationReason::ManualSeed {
                return Err(ClassifierError::ActivationDenied {
                    reason: "identity-only prior is restricted to explicit manual repair".into(),
                });
            }
            let current = self.read_active_repair_identity()?;
            if current.as_ref().map(|prior| prior.manifest_sha256.as_str())
                != Some(expected_sha256.as_str())
            {
                return Err(ClassifierError::StaleActivation);
            }
            current
        } else {
            self.read_active()?.map(|prior| PriorActivationIdentity {
                model_id: prior.model_id,
                activation_generation: prior.activation_generation,
                manifest_sha256: prior.manifest_sha256,
            })
        };
        if let Some(cur) = &current {
            if authorization.expected_prior_id.as_deref() != Some(cur.model_id.as_str())
                || authorization.expected_prior_generation != Some(cur.activation_generation)
            {
                return Err(ClassifierError::StaleActivation);
            }
        } else if authorization.expected_prior_id.is_some()
            || authorization.expected_prior_generation.is_some()
        {
            return Err(ClassifierError::StaleActivation);
        }

        let (manifest, manifest_sha256) = self.validated_manifest(id)?;
        if manifest_sha256 != authorization.manifest_sha256 {
            return Err(ClassifierError::ActivationDenied {
                reason: "immutable manifest checksum mismatch".into(),
            });
        }

        let payload = self.read_payload(id)?;
        let payload_sha = sha256_hex(payload.as_bytes());
        if payload_sha != authorization.payload_sha256 {
            return Err(ClassifierError::ActivationDenied {
                reason: "model payload checksum mismatch".into(),
            });
        }

        let report_bytes = self.read_report_raw(id)?;
        let report_sha = sha256_hex(&report_bytes);
        if report_sha != authorization.report_sha256 {
            return Err(ClassifierError::ActivationDenied {
                reason: "training report checksum mismatch".into(),
            });
        }

        let generation = current
            .as_ref()
            .map_or(Some(1), |m| m.activation_generation.checked_add(1))
            .ok_or_else(|| {
                ClassifierError::InvalidArtifact("activation_generation_overflow".into())
            })?;
        self.require_unused_history_slot(generation)?;

        let mut active = manifest;
        active.activation_generation = generation;
        active.previous_model_id = current.as_ref().map(|m| m.model_id.clone());
        active.activated_at = chrono::Utc::now().to_rfc3339();
        active.activation_reason = authorization.reason;
        active.manifest_sha256 = manifest_sha256.clone();

        self.write_active_with_commit_qualification(&active, id, generation)?;

        // Record activation history
        let record = ActivationRecord {
            schema_version: ACTIVATION_RECORD_SCHEMA_VERSION.into(),
            generation,
            model_id: id.clone(),
            model_sha256: active.model_sha256.clone(),
            immutable_manifest_sha256: manifest_sha256,
            previous_model_id: active.previous_model_id.clone(),
            previous_generation: current.as_ref().map(|m| m.activation_generation),
            reason: authorization.reason,
            activated_at: active.activated_at.clone(),
            committed: true,
        };
        let mut record_bytes = serde_json::to_vec_pretty(&record)
            .map_err(|e| ClassifierError::Storage(format!("serialize activation record: {e}")))?;
        record_bytes.push(b'\n');
        let record_sha = sha256_hex(&record_bytes);

        let activations_dir = self.root.join("activations");
        fs::create_dir_all(&activations_dir).map_err(|_| {
            ClassifierError::ActivationCommitUncertain {
                model_id: id.to_string(),
                generation,
            }
        })?;
        let record_path = activations_dir.join(format!("{generation}.json"));
        write_atomic_policy(&record_path, &record_bytes, false).map_err(|_| {
            ClassifierError::ActivationCommitUncertain {
                model_id: id.to_string(),
                generation,
            }
        })?;

        let index = ActivatedModelIndex {
            schema_version: ACTIVATED_MODEL_INDEX_SCHEMA_VERSION.into(),
            model_id: id.clone(),
            generation,
            activation_record_sha256: record_sha,
        };
        let mut index_bytes = serde_json::to_vec_pretty(&index).map_err(|e| {
            ClassifierError::Storage(format!("serialize activated model index: {e}"))
        })?;

        index_bytes.push(b'\n');

        let activated_models_dir = self.root.join("activated-models");
        fs::create_dir_all(&activated_models_dir).map_err(|_| {
            ClassifierError::ActivationCommitUncertain {
                model_id: id.to_string(),
                generation,
            }
        })?;
        let index_path = activated_models_dir.join(format!("{}.json", id.as_str()));
        write_atomic(&index_path, &index_bytes).map_err(|_| {
            ClassifierError::ActivationCommitUncertain {
                model_id: id.to_string(),
                generation,
            }
        })?;

        Ok(active)
    }

    /// Validates `id` completely and returns a `ValidatedArtifact`.
    ///
    /// # Errors
    /// Returns [`ClassifierError::InvalidArtifact`] or [`ClassifierError::Incompatible`].
    pub fn validate_model(&self, id: &ModelId) -> Result<ValidatedArtifact, ClassifierError> {
        let artifact = self.validate_model_integrity(id)?;
        #[cfg(feature = "classifier")]
        super::engine::restore_smoke_check(&artifact.payload)?;
        Ok(artifact)
    }

    /// Revalidates immutable checksums and compiled identity without starting JavaScript.
    /// Callers may reuse this for an already-live exact pin; new services still restore.
    pub fn validate_model_integrity(
        &self,
        id: &ModelId,
    ) -> Result<ValidatedArtifact, ClassifierError> {
        self.validated_artifact_integrity(id)
            .map(|(artifact, _)| artifact)
    }

    /// Rolls back active pointer to a target model previously activated in history.
    ///
    /// # Errors
    /// Returns [`ClassifierError::InvalidArtifact`] if `id` was not previously activated.
    pub fn rollback_to(
        &self,
        id: &ModelId,
        lease: &WriterLease,
    ) -> Result<ModelManifest, ClassifierError> {
        self.validate_lease(lease)?;
        let _operation = lease
            .operation
            .try_lock()
            .map_err(|_| ClassifierError::Busy)?;
        let index_path = self
            .root
            .join("activated-models")
            .join(format!("{}.json", id.as_str()));
        let index_bytes = read_bounded(&index_path, MAX_MANIFEST_BYTES)?;
        let index: ActivatedModelIndex = serde_json::from_slice(&index_bytes).map_err(|error| {
            ClassifierError::InvalidArtifact(format!("activation index JSON: {error}"))
        })?;
        if index.schema_version != ACTIVATED_MODEL_INDEX_SCHEMA_VERSION
            || index.model_id != *id
            || index.generation == 0
        {
            return Err(ClassifierError::InvalidArtifact(
                "invalid activated-model index".into(),
            ));
        }
        let record_bytes = read_bounded(
            &self
                .root
                .join("activations")
                .join(format!("{}.json", index.generation)),
            MAX_MANIFEST_BYTES,
        )?;
        let record: ActivationRecord = serde_json::from_slice(&record_bytes).map_err(|error| {
            ClassifierError::InvalidArtifact(format!("activation record JSON: {error}"))
        })?;
        if sha256_hex(&record_bytes) != index.activation_record_sha256
            || !record.committed
            || record.schema_version != ACTIVATION_RECORD_SCHEMA_VERSION
            || record.model_id != *id
            || record.generation != index.generation
        {
            return Err(ClassifierError::InvalidArtifact(
                "activation index does not identify a committed history record".into(),
            ));
        }

        let current = self.read_active()?.ok_or_else(|| {
            ClassifierError::InvalidArtifact("no active classifier model to roll back".into())
        })?;

        let (manifest, manifest_sha256) = self.validated_manifest(id)?;
        if record.model_sha256 != manifest.model_sha256
            || record.immutable_manifest_sha256 != manifest_sha256
        {
            return Err(ClassifierError::InvalidArtifact(
                "rollback history does not match immutable model".into(),
            ));
        }

        let generation = current
            .activation_generation
            .checked_add(1)
            .ok_or_else(|| {
                ClassifierError::InvalidArtifact("activation_generation_overflow".into())
            })?;
        self.require_unused_history_slot(generation)?;

        let mut active = manifest;
        active.activation_generation = generation;
        active.previous_model_id = Some(current.model_id.clone());
        active.activated_at = chrono::Utc::now().to_rfc3339();
        active.activation_reason = ActivationReason::ManualRollback;
        active.manifest_sha256 = manifest_sha256.clone();

        self.write_active_with_commit_qualification(&active, id, generation)?;

        // Record activation history
        let record = ActivationRecord {
            schema_version: ACTIVATION_RECORD_SCHEMA_VERSION.into(),
            generation,
            model_id: id.clone(),
            model_sha256: active.model_sha256.clone(),
            immutable_manifest_sha256: manifest_sha256,
            previous_model_id: active.previous_model_id.clone(),
            previous_generation: Some(current.activation_generation),
            reason: ActivationReason::ManualRollback,
            activated_at: active.activated_at.clone(),
            committed: true,
        };
        let mut record_bytes = serde_json::to_vec_pretty(&record).map_err(|e| {
            ClassifierError::Storage(format!("serialize rollback activation record: {e}"))
        })?;
        record_bytes.push(b'\n');
        let record_sha = sha256_hex(&record_bytes);

        let activations_dir = self.root.join("activations");
        fs::create_dir_all(&activations_dir).map_err(|_| {
            ClassifierError::ActivationCommitUncertain {
                model_id: id.to_string(),
                generation,
            }
        })?;
        let record_path = activations_dir.join(format!("{generation}.json"));
        write_atomic_policy(&record_path, &record_bytes, false).map_err(|_| {
            ClassifierError::ActivationCommitUncertain {
                model_id: id.to_string(),
                generation,
            }
        })?;

        let index = ActivatedModelIndex {
            schema_version: ACTIVATED_MODEL_INDEX_SCHEMA_VERSION.into(),
            model_id: id.clone(),
            generation,
            activation_record_sha256: record_sha,
        };
        let mut index_bytes = serde_json::to_vec_pretty(&index).map_err(|e| {
            ClassifierError::Storage(format!("serialize rollback activated model index: {e}"))
        })?;

        index_bytes.push(b'\n');

        let activated_models_dir = self.root.join("activated-models");
        fs::create_dir_all(&activated_models_dir).map_err(|_| {
            ClassifierError::ActivationCommitUncertain {
                model_id: id.to_string(),
                generation,
            }
        })?;
        let index_path = activated_models_dir.join(format!("{}.json", id.as_str()));
        write_atomic(&index_path, &index_bytes).map_err(|_| {
            ClassifierError::ActivationCommitUncertain {
                model_id: id.to_string(),
                generation,
            }
        })?;

        Ok(active)
    }

    /// Rolls back to the model recorded as
    /// [`ModelManifest::previous_model_id`] on the active manifest. The
    /// previous candidate is fully validated again — rollback is never a
    /// blind pointer replacement — and the just-superseded model becomes
    /// the new previous, so rollback can be undone.
    ///
    /// # Errors
    /// [`ClassifierError::InvalidArtifact`] when there is no active model,
    /// no recorded previous model, or validation of the previous model
    /// fails.
    pub fn rollback(&self) -> Result<ModelManifest, ClassifierError> {
        let lease = self.try_writer()?;
        let current = self.read_active()?.ok_or_else(|| {
            ClassifierError::InvalidArtifact("no active classifier model to roll back".into())
        })?;
        let previous_id = current.previous_model_id.clone().ok_or_else(|| {
            ClassifierError::InvalidArtifact(format!(
                "active model `{}` has no previous model to roll back to",
                current.model_id
            ))
        })?;
        self.rollback_to(&previous_id, &lease)
    }

    /// Reads the data-only model payload of `id`.
    ///
    /// # Errors
    /// [`ClassifierError::InvalidArtifact`] when the payload is missing or
    /// not UTF-8; [`ClassifierError::ModelTooLarge`] beyond
    /// [`MAX_ARTIFACT_BYTES`]; [`ClassifierError::Io`] on filesystem faults.
    pub fn read_payload(&self, id: &ModelId) -> Result<String, ClassifierError> {
        let path = self.model_dir(id).join(MODEL_PAYLOAD_FILE);
        let bytes = read_bounded(&path, MAX_ARTIFACT_BYTES)?;
        String::from_utf8(bytes).map_err(|_| {
            ClassifierError::InvalidArtifact(format!(
                "model payload {} is not valid UTF-8 JSON",
                path.display()
            ))
        })
    }

    /// Reads the raw report bytes of `id`.
    ///
    /// # Errors
    /// Returns [`ClassifierError::InvalidArtifact`] if the report is missing.
    pub fn read_report_raw(&self, id: &ModelId) -> Result<Vec<u8>, ClassifierError> {
        let path = self.model_dir(id).join(MODEL_REPORT_FILE);
        read_bounded(&path, MAX_MANIFEST_BYTES)
    }

    /// Reads the report string of `id`.
    ///
    /// # Errors
    /// Returns [`ClassifierError::InvalidArtifact`] if the report is missing or not valid UTF-8.
    pub fn read_report(&self, id: &ModelId) -> Result<String, ClassifierError> {
        let bytes = self.read_report_raw(id)?;
        String::from_utf8(bytes).map_err(|_| {
            ClassifierError::InvalidArtifact(format!(
                "model report {} is not valid UTF-8 JSON",
                self.model_dir(id).join(MODEL_REPORT_FILE).display()
            ))
        })
    }

    /// Validates a candidate end to end: immutable manifest bytes, report
    /// checksum, and payload validation against the compiled bundle.
    /// Returns the manifest plus the SHA-256 of the immutable manifest
    /// bytes for the active pointer.
    fn validated_manifest(&self, id: &ModelId) -> Result<(ModelManifest, String), ClassifierError> {
        let (artifact, checksum) = self.validated_artifact_integrity(id)?;
        #[cfg(feature = "classifier")]
        super::engine::restore_smoke_check(&artifact.payload)?;
        Ok((artifact.manifest, checksum))
    }

    fn validated_artifact_integrity(
        &self,
        id: &ModelId,
    ) -> Result<(ValidatedArtifact, String), ClassifierError> {
        let raw = self.read_manifest_raw(id)?;
        let manifest = parse_manifest_bytes(&raw)?;
        if manifest.model_id != *id {
            return Err(ClassifierError::InvalidArtifact(format!(
                "manifest in {} describes model `{}`, not `{}`",
                self.model_dir(id).join(MODEL_MANIFEST_FILE).display(),
                manifest.model_id,
                id
            )));
        }
        let report_path = self.model_dir(id).join(MODEL_REPORT_FILE);
        let report = read_bounded(&report_path, MAX_MANIFEST_BYTES)?;
        if manifest.report_sha256 != sha256_hex(&report) {
            return Err(ClassifierError::InvalidArtifact(format!(
                "report checksum does not match {}",
                report_path.display()
            )));
        }
        let payload = self.read_payload(id)?;
        let artifact = validate_artifact(&manifest, &payload, compiled_bundle_sha256())?;
        Ok((artifact, sha256_hex(&raw)))
    }

    pub(crate) fn read_manifest_raw(&self, id: &ModelId) -> Result<Vec<u8>, ClassifierError> {
        let path = self.model_dir(id).join(MODEL_MANIFEST_FILE);
        read_bounded(&path, MAX_MANIFEST_BYTES)
    }

    fn write_active_with_commit_qualification(
        &self,
        manifest: &ModelManifest,
        model_id: &ModelId,
        generation: u64,
    ) -> Result<(), ClassifierError> {
        self.write_active_observed(manifest, model_id, generation, ActiveWriteFault::None)
    }

    fn write_active_observed(
        &self,
        manifest: &ModelManifest,
        model_id: &ModelId,
        generation: u64,
        fault: ActiveWriteFault,
    ) -> Result<(), ClassifierError> {
        let path = self.active_manifest_path();
        let bytes = bounded_json_bytes(manifest)?;

        let dir = path.parent().ok_or_else(|| {
            ClassifierError::Io(format!("{} has no parent directory", path.display()))
        })?;
        fs::create_dir_all(dir)
            .map_err(|error| ClassifierError::Io(format!("create {}: {error}", dir.display())))?;
        let mut temp = NamedTempFile::new_in(dir)
            .map_err(|error| ClassifierError::Io(format!("stage {}: {error}", path.display())))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            temp.as_file()
                .set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(|error| {
                    ClassifierError::Io(format!("chmod {}: {error}", path.display()))
                })?;
        }
        use std::io::Write as _;
        temp.write_all(&bytes)
            .map_err(|error| ClassifierError::Io(format!("write {}: {error}", path.display())))?;
        temp.as_file()
            .sync_all()
            .map_err(|error| ClassifierError::Io(format!("sync {}: {error}", path.display())))?;
        if fault.before_rename() {
            return Err(ClassifierError::Io("injected pre-rename fault".into()));
        }
        temp.persist(&path).map_err(|error| {
            ClassifierError::Io(format!("replace {}: {}", path.display(), error.error))
        })?;
        if fault.after_rename() {
            return Err(ClassifierError::ActivationCommitUncertain {
                model_id: model_id.to_string(),
                generation,
            });
        }
        #[cfg(unix)]
        if File::open(dir).and_then(|d| d.sync_all()).is_err() {
            return Err(ClassifierError::ActivationCommitUncertain {
                model_id: model_id.to_string(),
                generation,
            });
        }
        Ok(())
    }
}

/// Validates a model manifest against its payload per the artifact
/// checklist: bounded sizes; schema, algorithm and compatibility versions;
/// exactly the six canonical domains with aligned thresholds, class counts
/// and family counts; finite smoothing and thresholds; positive class
/// support; a data-only JSON payload with the six per-domain sections,
/// explicit unique feature order, and a bundle hash matching
/// `expected_bundle_sha256`. The payload checksum must match
/// [`ModelManifest::model_sha256`].
///
/// The per-domain model objects must each carry a `feature_order` array of
/// unique token strings (numeric tokens appear as strings); their remaining
/// fields are the engine unit's typed Natural schema. No executable code
/// can hide in the payload: it is data-only JSON and pollution keys
/// (`__proto__`, `prototype`, `constructor`) are rejected anywhere.
///
/// # Errors
/// [`ClassifierError::ModelTooLarge`], [`ClassifierError::InvalidArtifact`]
/// or [`ClassifierError::Incompatible`] per the failing checklist item.
pub fn validate_artifact(
    manifest: &ModelManifest,
    payload: &str,
    expected_bundle_sha256: &str,
) -> Result<ValidatedArtifact, ClassifierError> {
    validate_manifest_shape(manifest)?;
    if payload.len() as u64 > MAX_ARTIFACT_BYTES {
        return Err(ClassifierError::ModelTooLarge);
    }
    if manifest.model_sha256 != sha256_hex(payload.as_bytes()) {
        return Err(ClassifierError::InvalidArtifact(
            "model checksum does not match the stored payload".into(),
        ));
    }
    if manifest.bundle_sha256 != expected_bundle_sha256 {
        return Err(ClassifierError::Incompatible(format!(
            "artifact was built for bundle `{}`, this build bundles `{}`",
            manifest.bundle_sha256, expected_bundle_sha256
        )));
    }
    #[cfg(feature = "classifier")]
    {
        let identity = super::compiled_identity()?;
        if manifest.bundle_sha256 != identity.bundle_sha256
            || manifest.bridge_version != identity.bridge_version
            || manifest.bridge_sha256 != identity.bridge_sha256
            || manifest.adapter_sha256 != identity.adapter_sha256
            || manifest.source_manifest_sha256 != identity.source_manifest_sha256
            || manifest.natural_version != identity.natural_version
            || manifest.apparatus_version != identity.apparatus_version
        {
            return Err(ClassifierError::Incompatible(
                "artifact compiled identity differs from this binary".into(),
            ));
        }
    }
    let payload_json = parse_strict_json(payload.as_bytes()).map_err(|error| {
        ClassifierError::InvalidArtifact(format!("model payload JSON: {error}"))
    })?;
    let top = payload_json.as_object().ok_or_else(|| {
        ClassifierError::InvalidArtifact("model payload must be a JSON object".into())
    })?;
    for key in top.keys() {
        if !PAYLOAD_TOP_LEVEL_KEYS.contains(&key.as_str()) {
            return Err(ClassifierError::InvalidArtifact(format!(
                "model payload has unexpected top-level key `{key}`"
            )));
        }
    }
    assert_data_only(&payload_json)?;
    require_payload_field(top, "schema_version", PAYLOAD_SCHEMA_VERSION)?;
    require_payload_field(top, "bridge_version", &manifest.bridge_version)?;
    require_payload_field(
        top,
        "feature_schema_version",
        &manifest.feature_schema_version,
    )?;
    require_payload_field(top, "taxonomy_version", &manifest.taxonomy_version)?;
    require_payload_field(top, "algorithm_version", &manifest.algorithm_version)?;
    require_payload_field(
        top,
        "preprocessing_version",
        &manifest.preprocessing_version,
    )?;
    let domains = top
        .get("domains")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| {
            ClassifierError::InvalidArtifact("model payload must carry a `domains` object".into())
        })?;

    // R4: Exact six canonical domains (reject extra or missing keys)
    if domains.len() != 6 {
        return Err(ClassifierError::InvalidArtifact(format!(
            "model payload `domains` must contain exactly the 6 canonical domains, found {}",
            domains.len()
        )));
    }

    for domain in DomainId::ALL {
        let model = domains
            .get(domain.as_str())
            .and_then(serde_json::Value::as_object)
            .ok_or_else(|| {
                ClassifierError::InvalidArtifact(format!(
                    "model payload domain `{}` is missing or not a JSON object",
                    domain.as_str()
                ))
            })?;
        let feature_order = model
            .get("feature_order")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| {
                ClassifierError::InvalidArtifact(format!(
                    "domain `{}` is missing its explicit `feature_order` array",
                    domain.as_str()
                ))
            })?;
        if feature_order.len() > MAX_FEATURES_PER_DOMAIN {
            return Err(ClassifierError::ModelTooLarge);
        }
        let mut seen = HashSet::with_capacity(feature_order.len());
        for entry in feature_order {
            let token = entry.as_str().ok_or_else(|| {
                ClassifierError::InvalidArtifact(format!(
                    "domain `{}` has a non-string feature-order entry",
                    domain.as_str()
                ))
            })?;
            if !seen.insert(token) {
                return Err(ClassifierError::InvalidArtifact(format!(
                    "domain `{}` has a duplicate feature-order entry",
                    domain.as_str()
                )));
            }
        }

        let feature_index = model
            .get("feature_index")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| {
                ClassifierError::InvalidArtifact(format!(
                    "domain `{}`: missing feature_index binding",
                    domain.as_str()
                ))
            })?;
        if feature_index.len() != feature_order.len()
            || feature_index.iter().zip(feature_order).enumerate().any(
                |(index, (binding, token))| {
                    binding.as_object().is_none_or(|entry| entry.len() != 2)
                        || binding.get("token") != Some(token)
                        || binding.get("index").and_then(serde_json::Value::as_u64)
                            != Some(index as u64)
                },
            )
        {
            return Err(ClassifierError::InvalidArtifact(format!(
                "domain `{}`: feature_index does not bind feature_order",
                domain.as_str()
            )));
        }

        // R2: Exact {present, absent} label set and nonzero support validation
        let class_totals = model
            .get("class_totals")
            .and_then(serde_json::Value::as_object)
            .ok_or_else(|| {
                ClassifierError::InvalidArtifact(format!(
                    "domain `{}` is missing its `class_totals` object",
                    domain.as_str()
                ))
            })?;
        let mut labels: Vec<&str> = class_totals.keys().map(String::as_str).collect();
        labels.sort_unstable();
        if labels != ["absent", "present"] {
            return Err(ClassifierError::InvalidArtifact(format!(
                "domain `{}`: invalid class set, expected exactly {{present, absent}}, found {:?}",
                domain.as_str(),
                labels
            )));
        }
        let present_count = class_totals
            .get("present")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| {
                ClassifierError::InvalidArtifact(format!(
                    "domain `{}`: class_totals must contain unsigned present and absent counts",
                    domain.as_str()
                ))
            })?;
        let absent_count = class_totals
            .get("absent")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| {
                ClassifierError::InvalidArtifact(format!(
                    "domain `{}`: class_totals must contain unsigned present and absent counts",
                    domain.as_str()
                ))
            })?;
        if present_count <= 1 {
            return Err(ClassifierError::InvalidArtifact(format!(
                "domain `{}`: class `present` must have nonzero training support (got {present_count})",
                domain.as_str()
            )));
        }
        if absent_count <= 1 {
            return Err(ClassifierError::InvalidArtifact(format!(
                "domain `{}`: class `absent` must have nonzero training support (got {absent_count})",
                domain.as_str()
            )));
        }

        let count = manifest
            .class_counts
            .iter()
            .find(|count| count.domain == domain)
            .ok_or_else(|| {
                ClassifierError::InvalidArtifact("missing manifest class counts".into())
            })?;
        // Apparatus includes one prior pseudo-observation per class.
        if count.present.checked_add(1) != Some(present_count)
            || count.absent.checked_add(1) != Some(absent_count)
        {
            return Err(ClassifierError::InvalidArtifact(format!(
                "domain `{}`: manifest class counts do not match payload support",
                domain.as_str()
            )));
        }

        let invalid_parameters = || {
            ClassifierError::InvalidArtifact(format!(
                "domain `{}`: invalid class_features, total_examples or smoothing",
                domain.as_str()
            ))
        };
        let smoothing = model
            .get("smoothing")
            .and_then(serde_json::Value::as_f64)
            .filter(|value| value.is_finite() && *value >= 0.0 && *value == manifest.smoothing)
            .ok_or_else(invalid_parameters)?;
        let expected_total = present_count
            .checked_add(absent_count)
            .and_then(|value| value.checked_sub(1))
            .ok_or_else(invalid_parameters)?;
        if model
            .get("total_examples")
            .and_then(serde_json::Value::as_u64)
            != Some(expected_total)
        {
            return Err(invalid_parameters());
        }
        let classes = model
            .get("class_features")
            .and_then(serde_json::Value::as_object)
            .ok_or_else(invalid_parameters)?;
        if classes.len() != 2 {
            return Err(invalid_parameters());
        }
        for (label, total) in [("present", present_count), ("absent", absent_count)] {
            let weights = classes
                .get(label)
                .and_then(serde_json::Value::as_object)
                .ok_or_else(invalid_parameters)?;
            for (index, weight) in weights {
                let position = index.parse::<usize>().map_err(|_| invalid_parameters())?;
                if position >= feature_order.len() || position.to_string() != *index {
                    return Err(invalid_parameters());
                }
                let weight = weight
                    .as_f64()
                    .filter(|value| value.is_finite() && *value > 0.0)
                    .ok_or_else(invalid_parameters)?;
                if (weight - smoothing).fract() != 0.0 {
                    return Err(ClassifierError::InvalidArtifact(format!(
                        "domain `{}`: fractional_feature_count",
                        domain.as_str()
                    )));
                }
                if weight < 1.0 + smoothing || weight > (total - 1) as f64 + smoothing {
                    return Err(invalid_parameters());
                }
            }
            if smoothing == 0.0 && weights.len() != feature_order.len() {
                return Err(invalid_parameters());
            }
        }

        // If retained example ID sets exist, enforce non-empty, unique, disjoint
        if let Some(example_ids) = model
            .get("example_ids")
            .and_then(serde_json::Value::as_object)
        {
            let present_ids = example_ids
                .get("present")
                .and_then(serde_json::Value::as_array);
            let absent_ids = example_ids
                .get("absent")
                .and_then(serde_json::Value::as_array);
            if let (Some(p_ids), Some(a_ids)) = (present_ids, absent_ids) {
                if p_ids.is_empty() {
                    return Err(ClassifierError::InvalidArtifact(format!(
                        "domain `{}`: retained present example IDs must not be empty",
                        domain.as_str()
                    )));
                }
                if a_ids.is_empty() {
                    return Err(ClassifierError::InvalidArtifact(format!(
                        "domain `{}`: retained absent example IDs must not be empty",
                        domain.as_str()
                    )));
                }
                let mut p_set = HashSet::with_capacity(p_ids.len());
                for id in p_ids {
                    let id_str = id.as_str().ok_or_else(|| {
                        ClassifierError::InvalidArtifact(format!(
                            "domain `{}`: non-string present example ID",
                            domain.as_str()
                        ))
                    })?;
                    if !p_set.insert(id_str) {
                        return Err(ClassifierError::InvalidArtifact(format!(
                            "domain `{}`: duplicate present example ID `{id_str}`",
                            domain.as_str()
                        )));
                    }
                }
                let mut a_set = HashSet::with_capacity(a_ids.len());
                for id in a_ids {
                    let id_str = id.as_str().ok_or_else(|| {
                        ClassifierError::InvalidArtifact(format!(
                            "domain `{}`: non-string absent example ID",
                            domain.as_str()
                        ))
                    })?;
                    if !a_set.insert(id_str) {
                        return Err(ClassifierError::InvalidArtifact(format!(
                            "domain `{}`: duplicate absent example ID `{id_str}`",
                            domain.as_str()
                        )));
                    }
                    if p_set.contains(id_str) {
                        return Err(ClassifierError::InvalidArtifact(format!(
                            "domain `{}`: example ID `{id_str}` belongs to both present and absent sets",
                            domain.as_str()
                        )));
                    }
                }
            }
        }
    }
    Ok(ValidatedArtifact {
        model_id: manifest.model_id.clone(),
        manifest: manifest.clone(),
        payload: payload.to_string(),
    })
}

/// Lower-case hex SHA-256 of `bytes`.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(2 * digest.len());
    for byte in digest {
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

/// Parses and structurally validates manifest bytes: bounded size, JSON
/// schema/compatibility versions, six canonical domains with aligned
/// per-domain arrays, finite parameters and well-formed checksums.
fn parse_manifest_bytes(bytes: &[u8]) -> Result<ModelManifest, ClassifierError> {
    if bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err(ClassifierError::ModelTooLarge);
    }
    let value = parse_strict_json(bytes)
        .map_err(|error| ClassifierError::InvalidArtifact(format!("manifest JSON: {error}")))?;
    let manifest: ModelManifest = serde_json::from_value(value)
        .map_err(|error| ClassifierError::InvalidArtifact(format!("manifest JSON: {error}")))?;
    validate_manifest_shape(&manifest)?;
    Ok(manifest)
}

#[derive(Deserialize)]
struct RepairManifestHeader {
    schema_version: String,
    model_id: ModelId,
    activation_generation: u64,
    previous_model_id: Option<ModelId>,
    activated_at: String,
    manifest_sha256: String,
    model_sha256: String,
    report_sha256: String,
    activation_reason: ActivationReason,
}
struct RepairManifest {
    header: RepairManifestHeader,
    immutable_fields: serde_json::Map<String, serde_json::Value>,
}

// Parse the known persisted layout without treating foreign algorithm/build
// identities as compatible. These checks establish identity, never model quality.
fn parse_repair_manifest(bytes: &[u8], immutable: bool) -> Result<RepairManifest, ClassifierError> {
    let value = parse_strict_json(bytes).map_err(|error| {
        ClassifierError::InvalidArtifact(format!("repair manifest JSON: {error}"))
    })?;
    assert_data_only(&value)?;
    let mut fields = value.as_object().cloned().ok_or_else(|| {
        ClassifierError::InvalidArtifact("repair manifest must be an object".into())
    })?;
    const KNOWN_FIELDS: &[&str] = &[
        "schema_version",
        "activation_generation",
        "model_id",
        "previous_model_id",
        "activated_at",
        "manifest_sha256",
        "model_sha256",
        "report_sha256",
        "dataset_sha256",
        "train_split_sha256",
        "calibration_split_sha256",
        "test_split_sha256",
        "family_assignment_sha256",
        "taxonomy_version",
        "domains",
        "algorithm_version",
        "preprocessing_version",
        "feature_schema_version",
        "admission_policy_version",
        "natural_version",
        "apparatus_version",
        "bundle_sha256",
        "bridge_version",
        "bridge_sha256",
        "adapter_sha256",
        "source_manifest_sha256",
        "smoothing",
        "thresholds",
        "class_counts",
        "family_counts",
        "metrics",
        "provenance",
        "activation_reason",
    ];
    if fields
        .keys()
        .any(|key| !KNOWN_FIELDS.contains(&key.as_str()))
    {
        return Err(ClassifierError::InvalidArtifact(
            "unknown repair manifest structural field".into(),
        ));
    }
    let header: RepairManifestHeader = serde_json::from_value(value.clone()).map_err(|error| {
        ClassifierError::InvalidArtifact(format!("repair manifest identity: {error}"))
    })?;
    if header.schema_version != MANIFEST_SCHEMA_VERSION {
        return Err(ClassifierError::Incompatible(
            "unknown repair manifest structural schema".into(),
        ));
    }
    let invalid =
        || ClassifierError::InvalidArtifact("malformed repair manifest identity/structure".into());
    if immutable {
        if header.activation_generation != 0
            || header.previous_model_id.is_some()
            || !header.activated_at.is_empty()
            || !header.manifest_sha256.is_empty()
        {
            return Err(invalid());
        }
    } else if header.activation_generation == 0
        || chrono::DateTime::parse_from_rfc3339(&header.activated_at).is_err()
    {
        return Err(invalid());
    } else {
        require_sha256_hex(&header.manifest_sha256, "manifest_sha256")?;
    }
    for key in [
        "model_sha256",
        "report_sha256",
        "dataset_sha256",
        "train_split_sha256",
        "calibration_split_sha256",
        "test_split_sha256",
        "family_assignment_sha256",
        "bundle_sha256",
    ] {
        let text = value[key].as_str().ok_or_else(invalid)?;
        require_sha256_hex(text, key)?;
    }
    for key in [
        "taxonomy_version",
        "algorithm_version",
        "preprocessing_version",
        "feature_schema_version",
        "admission_policy_version",
        "natural_version",
        "apparatus_version",
    ] {
        if value[key].as_str().is_none_or(str::is_empty) {
            return Err(invalid());
        }
    }
    for key in ["bridge_sha256", "adapter_sha256", "source_manifest_sha256"] {
        if let Some(field) = value.get(key) {
            require_sha256_hex(field.as_str().ok_or_else(invalid)?, key)?;
        } else if value["feature_schema_version"] != "classifier-features-v1" {
            return Err(invalid());
        }
    }
    if value.get("bridge_version").map_or(
        value["feature_schema_version"] != "classifier-features-v1",
        |field| field.as_str().is_none_or(str::is_empty),
    ) {
        return Err(invalid());
    }
    let domains: Vec<DomainId> =
        serde_json::from_value(value["domains"].clone()).map_err(|_| invalid())?;
    if domains.as_slice() != DomainId::ALL {
        return Err(invalid());
    }
    let thresholds: Vec<DomainThreshold> =
        serde_json::from_value(value["thresholds"].clone()).map_err(|_| invalid())?;
    let class_counts: Vec<DomainClassCount> =
        serde_json::from_value(value["class_counts"].clone()).map_err(|_| invalid())?;
    let family_counts: Vec<DomainFamilyCount> =
        serde_json::from_value(value["family_counts"].clone()).map_err(|_| invalid())?;
    if !thresholds
        .iter()
        .map(|value| value.domain)
        .eq(DomainId::ALL)
        || thresholds.iter().any(|value| !value.threshold.is_finite())
        || !class_counts
            .iter()
            .map(|value| value.domain)
            .eq(DomainId::ALL)
        || class_counts
            .iter()
            .any(|value| value.present == 0 || value.absent == 0)
        || !family_counts
            .iter()
            .map(|value| value.domain)
            .eq(DomainId::ALL)
        || value["smoothing"]
            .as_f64()
            .is_none_or(|value| !value.is_finite() || value < 0.0)
        || !value["metrics"].is_object()
    {
        return Err(invalid());
    }
    let _: Provenance =
        serde_json::from_value(value["provenance"].clone()).map_err(|_| invalid())?;
    for field in [
        "activation_generation",
        "previous_model_id",
        "activated_at",
        "manifest_sha256",
        "activation_reason",
    ] {
        fields.remove(field);
    }
    Ok(RepairManifest {
        header,
        immutable_fields: fields,
    })
}

fn parse_repair_record(bytes: &[u8]) -> Result<ActivationRecord, ClassifierError> {
    let value = parse_strict_json(bytes).map_err(|error| {
        ClassifierError::InvalidArtifact(format!("repair history JSON: {error}"))
    })?;
    assert_data_only(&value)?;
    let record: ActivationRecord = serde_json::from_value(value).map_err(|error| {
        ClassifierError::InvalidArtifact(format!("repair history schema: {error}"))
    })?;
    if record.schema_version != ACTIVATION_RECORD_SCHEMA_VERSION
        || !record.committed
        || record.generation == 0
        || chrono::DateTime::parse_from_rfc3339(&record.activated_at).is_err()
    {
        return Err(ClassifierError::InvalidArtifact(
            "repair activation history is not committed/known".into(),
        ));
    }
    require_sha256_hex(&record.model_sha256, "history model_sha256")?;
    require_sha256_hex(
        &record.immutable_manifest_sha256,
        "history immutable_manifest_sha256",
    )?;
    Ok(record)
}

/// Structural and compatibility checks shared by every manifest read.
fn validate_manifest_shape(manifest: &ModelManifest) -> Result<(), ClassifierError> {
    let versions = [
        (
            manifest.schema_version.as_str(),
            MANIFEST_SCHEMA_VERSION,
            "manifest schema",
        ),
        (
            manifest.taxonomy_version.as_str(),
            TAXONOMY_VERSION,
            "taxonomy version",
        ),
        (
            manifest.algorithm_version.as_str(),
            ALGORITHM_VERSION,
            "algorithm version",
        ),
        (
            manifest.preprocessing_version.as_str(),
            PREPROCESSING_VERSION,
            "preprocessing version",
        ),
        (
            manifest.feature_schema_version.as_str(),
            FEATURE_SCHEMA_VERSION,
            "feature schema version",
        ),
        (
            manifest.admission_policy_version.as_str(),
            ADMISSION_POLICY_VERSION,
            "admission policy version",
        ),
    ];
    for (actual, expected, what) in versions {
        if actual != expected {
            return Err(ClassifierError::Incompatible(format!(
                "{what} `{actual}` is not supported by this build (`{expected}`)"
            )));
        }
    }
    if manifest.domains != DomainId::ALL.to_vec() {
        return Err(ClassifierError::InvalidArtifact(
            "manifest must list exactly the six canonical domains in canonical order".into(),
        ));
    }
    require_canonical_order(manifest.thresholds.iter().map(|t| t.domain), "thresholds")?;
    require_canonical_order(
        manifest.class_counts.iter().map(|c| c.domain),
        "class counts",
    )?;
    require_canonical_order(
        manifest.family_counts.iter().map(|c| c.domain),
        "family counts",
    )?;
    if manifest.thresholds.iter().any(|t| !t.threshold.is_finite()) {
        return Err(ClassifierError::InvalidArtifact(
            "all six thresholds must be finite".into(),
        ));
    }
    if manifest.smoothing != 1.0 {
        return Err(ClassifierError::InvalidArtifact(
            "smoothing must equal1.0 for the compiled Bayes algorithm".into(),
        ));
    }
    for count in &manifest.class_counts {
        if count.present == 0 || count.absent == 0 {
            return Err(ClassifierError::InvalidArtifact(format!(
                "domain `{}` needs both classes trained (present={}, absent={})",
                count.domain.as_str(),
                count.present,
                count.absent
            )));
        }
    }
    if !manifest.metrics.is_object() {
        return Err(ClassifierError::InvalidArtifact(
            "manifest metrics must be a JSON object".into(),
        ));
    }
    if manifest.activation_generation > 0 {
        if manifest.activated_at.is_empty()
            || chrono::DateTime::parse_from_rfc3339(&manifest.activated_at).is_err()
        {
            return Err(ClassifierError::InvalidArtifact(
                "an activated manifest must carry an activation timestamp".into(),
            ));
        }
        if manifest.manifest_sha256.is_empty() {
            return Err(ClassifierError::InvalidArtifact(
                "an activated manifest must carry non-empty manifest_sha256".into(),
            ));
        }
        require_sha256_hex(&manifest.manifest_sha256, "manifest_sha256")?;
    } else if !manifest.manifest_sha256.is_empty() {
        return Err(ClassifierError::InvalidArtifact(
            "an immutable stored manifest must carry an empty manifest_sha256".into(),
        ));
    }
    require_sha256_hex(&manifest.model_sha256, "model_sha256")?;
    require_sha256_hex(&manifest.report_sha256, "report_sha256")?;
    require_sha256_hex(&manifest.dataset_sha256, "dataset_sha256")?;
    require_sha256_hex(&manifest.train_split_sha256, "train_split_sha256")?;
    require_sha256_hex(
        &manifest.calibration_split_sha256,
        "calibration_split_sha256",
    )?;
    require_sha256_hex(&manifest.test_split_sha256, "test_split_sha256")?;
    require_sha256_hex(
        &manifest.family_assignment_sha256,
        "family_assignment_sha256",
    )?;
    require_sha256_hex(&manifest.bundle_sha256, "bundle_sha256")?;
    require_sha256_hex(&manifest.bridge_sha256, "bridge_sha256")?;
    require_sha256_hex(&manifest.adapter_sha256, "adapter_sha256")?;
    require_sha256_hex(&manifest.source_manifest_sha256, "source_manifest_sha256")?;
    if manifest.natural_version.is_empty()
        || manifest.apparatus_version.is_empty()
        || manifest.bridge_version.is_empty()
    {
        return Err(ClassifierError::InvalidArtifact(
            "manifest must record the pinned Natural and apparatus versions".into(),
        ));
    }
    Ok(())
}

fn require_canonical_order(
    domains: impl Iterator<Item = DomainId>,
    what: &str,
) -> Result<(), ClassifierError> {
    if !domains.eq(DomainId::ALL) {
        return Err(ClassifierError::InvalidArtifact(format!(
            "{what} must list the six domains in canonical order"
        )));
    }
    Ok(())
}

fn require_sha256_hex(value: &str, field: &str) -> Result<(), ClassifierError> {
    let valid = value.len() == 64
        && value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'));
    if valid {
        Ok(())
    } else {
        Err(ClassifierError::InvalidArtifact(format!(
            "`{field}` must be 64 lower-case hex characters"
        )))
    }
}

fn require_payload_field(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &str,
    expected: &str,
) -> Result<(), ClassifierError> {
    let actual = object
        .get(key)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            ClassifierError::InvalidArtifact(format!(
                "model payload field `{key}` must be a string"
            ))
        })?;
    if actual != expected {
        return Err(ClassifierError::Incompatible(format!(
            "model payload {key} `{actual}` does not match the artifact manifest `{expected}`"
        )));
    }
    Ok(())
}

/// Rejects pollution/code-carrier keys anywhere in a JSON value. Depth is
/// already bounded by the parser's recursion limit.
fn assert_data_only(value: &serde_json::Value) -> Result<(), ClassifierError> {
    match value {
        serde_json::Value::Object(map) => {
            for (key, child) in map {
                if FORBIDDEN_KEYS.contains(&key.as_str()) {
                    return Err(ClassifierError::InvalidArtifact(format!(
                        "model payload contains forbidden key `{key}`"
                    )));
                }
                assert_data_only(child)?;
            }
            Ok(())
        }
        serde_json::Value::Array(items) => {
            for item in items {
                assert_data_only(item)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Reads a file with a hard size bound, consuming at most `max_bytes + 1` bytes.
fn read_bounded(path: &Path, max_bytes: u64) -> Result<Vec<u8>, ClassifierError> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Err(ClassifierError::InvalidArtifact(format!(
                "required artifact file {} is missing",
                path.display()
            )));
        }
        Err(error) => {
            return Err(ClassifierError::Io(format!(
                "open {}: {error}",
                path.display()
            )));
        }
    };
    read_bounded_file(file, path, max_bytes)
}

fn read_bounded_file(
    mut file: impl std::io::Read,
    path: &Path,
    max_bytes: u64,
) -> Result<Vec<u8>, ClassifierError> {
    use std::io::Read as _;
    let mut buf = Vec::new();
    let read_result = (&mut file)
        .take(max_bytes.saturating_add(1))
        .read_to_end(&mut buf);
    match read_result {
        Ok(_) => {
            if buf.len() as u64 > max_bytes {
                return Err(ClassifierError::ModelTooLarge);
            }
            Ok(buf)
        }
        Err(error) => Err(ClassifierError::Io(format!(
            "read {}: {error}",
            path.display()
        ))),
    }
}

/// Writes `bytes` to `path` atomically: temp file in the same directory,
/// fsync, rename, then fsync the directory. Mode 0600 on Unix. Mirrors the
/// storage home writer without importing it (the classifier must not depend
/// on the storage module).
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), ClassifierError> {
    write_atomic_policy(path, bytes, true)
}

fn write_atomic_policy(path: &Path, bytes: &[u8], replace: bool) -> Result<(), ClassifierError> {
    use std::io::Write as _;
    let dir = path.parent().ok_or_else(|| {
        ClassifierError::Io(format!("{} has no parent directory", path.display()))
    })?;
    fs::create_dir_all(dir)
        .map_err(|error| ClassifierError::Io(format!("create {}: {error}", dir.display())))?;
    let mut temp = NamedTempFile::new_in(dir)
        .map_err(|error| ClassifierError::Io(format!("stage {}: {error}", path.display())))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temp.as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|error| ClassifierError::Io(format!("chmod {}: {error}", path.display())))?;
    }
    temp.write_all(bytes)
        .map_err(|error| ClassifierError::Io(format!("write {}: {error}", path.display())))?;
    temp.as_file()
        .sync_all()
        .map_err(|error| ClassifierError::Io(format!("sync {}: {error}", path.display())))?;
    if replace {
        temp.persist(path).map_err(|error| {
            ClassifierError::Io(format!("replace {}: {}", path.display(), error.error))
        })?;
    } else {
        temp.persist_noclobber(path).map_err(|error| {
            ClassifierError::Io(format!(
                "install immutable {}: {}",
                path.display(),
                error.error
            ))
        })?;
    }
    #[cfg(unix)]
    File::open(dir)
        .and_then(|dir| dir.sync_all())
        .map_err(|error| ClassifierError::Io(format!("sync {}: {error}", dir.display())))?;
    Ok(())
}

/// Serializes bounded metadata without allocating an unbounded intermediate string.
pub(crate) fn bounded_json_bytes(value: &impl Serialize) -> Result<Vec<u8>, ClassifierError> {
    struct BoundedWriter(Vec<u8>);
    impl std::io::Write for BoundedWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > (MAX_MANIFEST_BYTES as usize - 1).saturating_sub(self.0.len()) {
                return Err(std::io::Error::other(
                    "classifier metadata exceeds size bound",
                ));
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut writer = BoundedWriter(Vec::new());
    serde_json::to_writer_pretty(&mut writer, value).map_err(|error| {
        ClassifierError::InvalidArtifact(format!("serialize bounded metadata: {error}"))
    })?;
    writer.0.push(b'\n');
    Ok(writer.0)
}

// Parse recursively before conversion to Value so duplicate keys cannot be collapsed.
fn parse_strict_json(bytes: &[u8]) -> Result<serde_json::Value, serde_json::Error> {
    struct StrictValue(serde_json::Value);
    impl<'de> Deserialize<'de> for StrictValue {
        fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            struct Visitor;
            impl<'de> serde::de::Visitor<'de> for Visitor {
                type Value = StrictValue;
                fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                    formatter.write_str("JSON without duplicate keys")
                }
                fn visit_bool<E: serde::de::Error>(self, value: bool) -> Result<Self::Value, E> {
                    Ok(StrictValue(value.into()))
                }
                fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<Self::Value, E> {
                    Ok(StrictValue(value.into()))
                }
                fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<Self::Value, E> {
                    Ok(StrictValue(value.into()))
                }
                fn visit_f64<E: serde::de::Error>(self, value: f64) -> Result<Self::Value, E> {
                    let number = serde_json::Number::from_f64(value)
                        .ok_or_else(|| E::custom("non-finite JSON number"))?;
                    Ok(StrictValue(serde_json::Value::Number(number)))
                }
                fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
                    Ok(StrictValue(value.into()))
                }
                fn visit_string<E: serde::de::Error>(
                    self,
                    value: String,
                ) -> Result<Self::Value, E> {
                    Ok(StrictValue(value.into()))
                }
                fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                    Ok(StrictValue(serde_json::Value::Null))
                }
                fn visit_seq<A: serde::de::SeqAccess<'de>>(
                    self,
                    mut seq: A,
                ) -> Result<Self::Value, A::Error> {
                    let mut values = Vec::new();
                    while let Some(StrictValue(value)) = seq.next_element()? {
                        values.push(value);
                    }
                    Ok(StrictValue(serde_json::Value::Array(values)))
                }
                fn visit_map<A: serde::de::MapAccess<'de>>(
                    self,
                    mut map: A,
                ) -> Result<Self::Value, A::Error> {
                    let mut values = serde_json::Map::new();
                    while let Some((key, StrictValue(value))) =
                        map.next_entry::<String, StrictValue>()?
                    {
                        if values.insert(key.clone(), value).is_some() {
                            return Err(serde::de::Error::custom(format!(
                                "duplicate JSON key `{key}`"
                            )));
                        }
                    }
                    Ok(StrictValue(serde_json::Value::Object(values)))
                }
            }
            deserializer.deserialize_any(Visitor)
        }
    }
    serde_json::from_slice::<StrictValue>(bytes).map(|value| value.0)
}

#[cfg(test)]
mod bounded_io_tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn bounded_reader_never_consumes_beyond_limit_plus_sentinel() {
        struct Instrumented<'a>(&'a Cell<usize>);
        impl std::io::Read for Instrumented<'_> {
            fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
                buffer.fill(b'x');
                self.0.set(self.0.get() + buffer.len());
                Ok(buffer.len())
            }
        }
        let consumed = Cell::new(0);
        assert!(matches!(
            read_bounded_file(Instrumented(&consumed), Path::new("instrumented"), 16),
            Err(ClassifierError::ModelTooLarge)
        ));
        assert_eq!(consumed.get(), 17);
    }

    #[test]
    fn bounded_reader_accepts_exact_limit_and_preserves_stream_errors() {
        use std::io::{Cursor, Read};
        for length in [0usize, 15, 16] {
            let bytes = vec![b'x'; length];
            assert_eq!(
                read_bounded_file(Cursor::new(&bytes), Path::new("exact"), 16).unwrap(),
                bytes
            );
        }
        struct Growing {
            returned: usize,
        }
        impl Read for Growing {
            fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
                // The open stream first supplies exactly the limit, then grows.
                let count = buffer.len().min(if self.returned < 16 {
                    16 - self.returned
                } else {
                    1
                });
                buffer[..count].fill(b'x');
                self.returned += count;
                Ok(count)
            }
        }
        assert!(matches!(
            read_bounded_file(Growing { returned: 0 }, Path::new("growing"), 16),
            Err(ClassifierError::ModelTooLarge)
        ));
        struct Failing;
        impl Read for Failing {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("read-fault-sentinel"))
            }
        }
        assert!(
            matches!(read_bounded_file(Failing,Path::new("fault-stream"),16),Err(ClassifierError::Io(message)) if message.contains("fault-stream")&&message.contains("read-fault-sentinel"))
        );
    }

    #[test]
    fn bounded_serializer_rejects_large_metadata_before_installation() {
        let metadata = serde_json::json!({ "large": "x".repeat(MAX_MANIFEST_BYTES as usize) });
        assert!(bounded_json_bytes(&metadata).is_err());
    }
}

#[cfg(test)]
mod commit_fault_tests {
    use super::*;

    fn manifest() -> ModelManifest {
        serde_json::from_value(serde_json::json!({
            "schema_version":MANIFEST_SCHEMA_VERSION,"activation_generation":1,"model_id":"commit-fixture",
            "previous_model_id":null,"activated_at":"2026-01-01T00:00:00Z","model_sha256":"0".repeat(64),
            "manifest_sha256":"1".repeat(64),"report_sha256":"2".repeat(64),"dataset_sha256":"3".repeat(64),
            "train_split_sha256":"4".repeat(64),"calibration_split_sha256":"5".repeat(64),"test_split_sha256":"6".repeat(64),
            "family_assignment_sha256":"7".repeat(64),"taxonomy_version":TAXONOMY_VERSION,"domains":DomainId::ALL,
            "algorithm_version":ALGORITHM_VERSION,"preprocessing_version":PREPROCESSING_VERSION,"feature_schema_version":FEATURE_SCHEMA_VERSION,
            "admission_policy_version":ADMISSION_POLICY_VERSION,"natural_version":"8.2.6","apparatus_version":"0.0.10",
            "bundle_sha256":COMPILED_BUNDLE_SHA256,"bridge_version":super::super::BRIDGE_VERSION,"bridge_sha256":"8".repeat(64),
            "adapter_sha256":"9".repeat(64),"source_manifest_sha256":"a".repeat(64),"smoothing":1.0,
            "thresholds":DomainId::ALL.map(|domain|serde_json::json!({"domain":domain,"threshold":2.0})),
            "class_counts":DomainId::ALL.map(|domain|serde_json::json!({"domain":domain,"present":1,"absent":1})),
            "family_counts":DomainId::ALL.map(|domain|serde_json::json!({"domain":domain,"accepted_families":0})),
            "metrics":{},"provenance":Provenance::synthetic_baseline("fixture","3".repeat(64)),"activation_reason":ActivationReason::ManualSeed
        })).unwrap()
    }

    #[test]
    fn pre_rename_failure_retains_prior_bytes_and_post_rename_failure_is_uncertain() {
        let home = tempfile::tempdir().unwrap();
        let store = ArtifactStore::new(home.path()).unwrap();
        let manifest = manifest();
        let prior = b"prior active pointer";
        fs::write(store.active_manifest_path(), prior).unwrap();
        assert!(matches!(
            store.write_active_observed(
                &manifest,
                &manifest.model_id,
                1,
                ActiveWriteFault::BeforeRename
            ),
            Err(ClassifierError::Io(_))
        ));
        assert_eq!(fs::read(store.active_manifest_path()).unwrap(), prior);
        assert!(matches!(
            store.write_active_observed(
                &manifest,
                &manifest.model_id,
                1,
                ActiveWriteFault::AfterRename
            ),
            Err(ClassifierError::ActivationCommitUncertain { generation: 1, .. })
        ));
        assert_eq!(
            fs::read(store.active_manifest_path()).unwrap(),
            bounded_json_bytes(&manifest).unwrap()
        );
    }
}
