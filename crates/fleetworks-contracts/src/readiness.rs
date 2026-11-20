//! Typed read-only backend readiness and durable maintenance admission.
//!
//! Readiness GETs do not seed, train, load models or rebuild indexes. Dataset
//! references identify staged hash-verified backend bytes, never a Mac path or
//! arbitrary server path. Receipt acceptance is not successful job completion.

use std::fmt;

use serde::Deserialize;
use serde::Serialize;

use crate::connect::RegistrationReceipt;
use crate::context::JobOrigin;
use crate::context::MutationContext;
use crate::context::Permission;
use crate::context::ResourceScope;

/// Bounded authenticated readiness aggregate, separate from load-balancer health.
pub const WORKSTATION_READINESS_PATH: &str = "/api/v1/workstation/readiness";
/// Durable actor catalog seed admission.
pub const ACTOR_SEED_PATH: &str = "/api/v1/workstation/maintenance/seed-actors";
/// Durable optional Natural training admission; no automatic activation.
pub const CLASSIFIER_TRAIN_PATH: &str = "/api/v1/workstation/maintenance/train-classifier";
/// Explicit candidate activation, separate from successful training.
pub const CLASSIFIER_ACTIVATE_PATH: &str = "/api/v1/classifier/activate";
/// Expected `operation` tag for `ActorReindexResult`.
pub const ACTOR_REINDEX_OPERATION: &str = "actor_reindex";

/// Safe, bounded job result emitted by the actor reindex worker and projected in activity.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ActorReindexResult {
    pub operation: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_job_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible_checkpoint: Option<u64>,
    pub scanned: u64,
    pub reader_visible: bool,
}

impl ActorReindexResult {
    /// Construct a validated actor reindex job result without worker paths or raw reports.
    #[must_use]
    pub fn new(
        original_job_id: Option<String>,
        generation: Option<String>,
        visible_checkpoint: Option<u64>,
        scanned: u64,
        reader_visible: bool,
    ) -> Self {
        Self {
            operation: ACTOR_REINDEX_OPERATION.to_owned(),
            original_job_id,
            generation,
            visible_checkpoint,
            scanned,
            reader_visible,
        }
    }
}

/// Server-owned readiness result, never inferred from the presence of a process.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ReadinessState {
    Ready,
    NotReady,
    Running,
    Degraded,
    Unknown,
    Unavailable,
    NotApplicable,
}

impl ReadinessState {
    /// Stable status label for accessible text and diagnostics.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::NotReady => "not_ready",
            Self::Running => "running",
            Self::Degraded => "degraded",
            Self::Unknown => "unknown",
            Self::Unavailable => "unavailable",
            Self::NotApplicable => "not_applicable",
        }
    }
}

impl fmt::Display for ReadinessState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Stable readiness check identity; seed, search, inference and training differ.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ReadinessCheckId {
    Gateway,
    Datastore,
    Jobs,
    Brain,
    Control,
    Nlp,
    Classifier,
    Registration,
    ActorCatalog,
    ActorSearch,
    ClassifierInference,
    ClassifierTraining,
}

impl ReadinessCheckId {
    /// Stable check spelling shared by aggregate and presentation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Gateway => "gateway",
            Self::Datastore => "datastore",
            Self::Jobs => "jobs",
            Self::Brain => "brain",
            Self::Control => "control",
            Self::Nlp => "nlp",
            Self::Classifier => "classifier",
            Self::Registration => "registration",
            Self::ActorCatalog => "actor_catalog",
            Self::ActorSearch => "actor_search",
            Self::ClassifierInference => "classifier_inference",
            Self::ClassifierTraining => "classifier_training",
        }
    }
}

impl fmt::Display for ReadinessCheckId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Supported page action; no generic command or infrastructure controls exist.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ReadinessActionId {
    Refresh,
    SeedActors,
    TrainClassifier,
    ActivateClassifier,
    RebuildActorIndex,
}

/// Safe action availability metadata; the server enforces it again at admission.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct ReadinessAction {
    pub id: ReadinessActionId,
    pub permission: Permission,
    pub allowed: bool,
    pub preconditions: Vec<String>,
    pub disabled_reason: Option<String>,
}

impl ReadinessAction {
    /// Describe an action and its explicit grant/preconditions.
    #[must_use]
    pub fn new(id: ReadinessActionId, permission: Permission, allowed: bool) -> Self {
        Self {
            id,
            permission,
            allowed,
            preconditions: Vec::new(),
            disabled_reason: None,
        }
    }
}

/// One timestamped safe check, including explicit unavailable/unknown state.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct ReadinessCheck {
    pub id: ReadinessCheckId,
    pub scope: ResourceScope,
    pub state: ReadinessState,
    pub reason_code: String,
    pub explanation: String,
    pub observed_at: String,
    pub fresh_for_ms: u64,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub job_ids: Vec<String>,
    #[serde(default)]
    pub artifact_ids: Vec<String>,
    #[serde(default)]
    pub actions: Vec<ReadinessAction>,
}

impl ReadinessCheck {
    /// Build a backend check; zero freshness means immediately stale evidence.
    #[must_use]
    pub fn new(
        id: ReadinessCheckId,
        state: ReadinessState,
        reason_code: String,
        explanation: String,
        observed_at: String,
    ) -> Self {
        Self {
            id,
            scope: if id == ReadinessCheckId::Registration {
                ResourceScope::WorkstationTriggeredActivity
            } else {
                ResourceScope::Backend
            },
            state,
            reason_code,
            explanation,
            observed_at,
            fresh_for_ms: 0,
            capabilities: Vec::new(),
            job_ids: Vec::new(),
            artifact_ids: Vec::new(),
            actions: Vec::new(),
        }
    }
}

/// Partial readiness results from configured owners, never arbitrary check URLs.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct WorkstationReadiness {
    pub deployment_id: String,
    pub observed_at: String,
    pub checks: Vec<ReadinessCheck>,
    pub registration: Option<RegistrationReceipt>,
    #[serde(default)]
    pub configured_inputs: Vec<ConfiguredMaintenanceInput>,
    /// The operator-chosen backend label, present only when one was configured at bootstrap.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform_identity: Option<PlatformIdentity>,
}

/// An operator-chosen backend label with the deployment it names. It is never derived from the
/// host OS, chipset, hostname, IP or Connect ID, and a rename keeps the deployment ID.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlatformIdentity {
    pub deployment_id: String,
    pub display_name: String,
}

impl WorkstationReadiness {
    /// Collect a bounded aggregate before or after workstation registration.
    #[must_use]
    pub fn new(deployment_id: String, observed_at: String, checks: Vec<ReadinessCheck>) -> Self {
        Self {
            deployment_id,
            observed_at,
            checks,
            registration: None,
            configured_inputs: Vec::new(),
            platform_identity: None,
        }
    }

    /// Attach the backend label for this deployment.
    #[must_use]
    pub fn with_platform_identity(mut self, display_name: String) -> Self {
        self.platform_identity = Some(PlatformIdentity {
            deployment_id: self.deployment_id.clone(),
            display_name,
        });
        self
    }
}

/// Operator-configured immutable inputs. They contain no host paths or credentials.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ConfiguredMaintenanceInput {
    pub operation: MaintenanceOperation,
    pub dataset: Option<DatasetRef>,
    pub configuration_revision: u64,
}

/// Dataset purpose is validated independently of its name or selected folder.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum DatasetKind {
    ActorCatalog,
    ClassifierTraining,
}

/// An opaque reference to immutable, validated backend-staged bytes.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct DatasetRef {
    pub id: String,
    pub kind: DatasetKind,
    pub sha256: String,
    pub size_bytes: u64,
}

impl DatasetRef {
    /// Construct a syntactically bounded reference, without claiming authorization.
    pub fn new(
        id: String,
        kind: DatasetKind,
        sha256: String,
        size_bytes: u64,
    ) -> Result<Self, MaintenanceInputError> {
        let reference = Self {
            id,
            kind,
            sha256: sha256.to_ascii_lowercase(),
            size_bytes,
        };
        reference.validate()?;
        Ok(reference)
    }

    /// Recheck a decoded reference before looking up authoritative upload metadata.
    pub fn validate(&self) -> Result<(), MaintenanceInputError> {
        if !(1..=128).contains(&self.id.len())
            || !self
                .id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(MaintenanceInputError::InvalidDatasetId);
        }
        if self.sha256.len() != 64 || !self.sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(MaintenanceInputError::InvalidDatasetHash);
        }
        if self.size_bytes == 0 {
            return Err(MaintenanceInputError::EmptyDataset);
        }
        Ok(())
    }
}

/// Existing supported trainable engine; this is not Rust-BERT retraining.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum TrainableEngine {
    Natural,
}

/// Resource-level durable operation, independent of the requesting workspace.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum MaintenanceOperation {
    SeedActors,
    TrainClassifier,
    ReindexActors,
}

/// Small maintenance JSON referring to uploaded bytes and confirmed context.
///
/// There is intentionally no server path, shell command, source/owner assignment,
/// or activation flag. Training and model activation require separate consent.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct MaintenanceRequest {
    pub request_id: String,
    pub context: MutationContext,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dataset: Option<DatasetRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<TrainableEngine>,
}

impl MaintenanceRequest {
    /// Request seeding from an actor catalog reference.
    pub fn for_seed(
        request_id: String,
        context: MutationContext,
        dataset: DatasetRef,
    ) -> Result<Self, MaintenanceInputError> {
        let request = Self {
            request_id,
            context,
            dataset: Some(dataset),
            engine: None,
        };
        request.validate(MaintenanceOperation::SeedActors)?;
        Ok(request)
    }

    /// Request optional Natural training with activation absent by construction.
    pub fn for_training(
        request_id: String,
        context: MutationContext,
        dataset: DatasetRef,
    ) -> Result<Self, MaintenanceInputError> {
        let request = Self {
            request_id,
            context,
            dataset: Some(dataset),
            engine: Some(TrainableEngine::Natural),
        };
        request.validate(MaintenanceOperation::TrainClassifier)?;
        Ok(request)
    }

    /// Request rebuilding from canonical backend actors, not uploaded CSV bytes.
    pub fn for_reindex(
        request_id: String,
        context: MutationContext,
    ) -> Result<Self, MaintenanceInputError> {
        let request = Self {
            request_id,
            context,
            dataset: None,
            engine: None,
        };
        request.validate(MaintenanceOperation::ReindexActors)?;
        Ok(request)
    }

    /// Validate route-specific shape; server permission/metadata checks follow.
    pub fn validate(&self, operation: MaintenanceOperation) -> Result<(), MaintenanceInputError> {
        if !(1..=64).contains(&self.request_id.len())
            || !self
                .request_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(MaintenanceInputError::InvalidRequestId);
        }
        if let Some(dataset) = &self.dataset {
            dataset.validate()?;
        }
        let valid = match operation {
            MaintenanceOperation::SeedActors => {
                self.dataset
                    .as_ref()
                    .is_some_and(|dataset| dataset.kind == DatasetKind::ActorCatalog)
                    && self.engine.is_none()
            }
            MaintenanceOperation::TrainClassifier => {
                self.dataset
                    .as_ref()
                    .is_some_and(|dataset| dataset.kind == DatasetKind::ClassifierTraining)
                    && self.engine == Some(TrainableEngine::Natural)
            }
            MaintenanceOperation::ReindexActors => self.dataset.is_none() && self.engine.is_none(),
        };
        if !valid {
            return Err(MaintenanceInputError::WrongDatasetOrEngine);
        }
        Ok(())
    }
}

/// A durable maintenance admission or a confirmed no-op for completed work.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct MaintenanceReceipt {
    pub job_id: String,
    pub operation: MaintenanceOperation,
    pub resource_scope: ResourceScope,
    pub origin: JobOrigin,
    pub dataset: Option<DatasetRef>,
    pub reused: bool,
    /// Already-completed work accepted without scheduling another job. The job ID
    /// is empty when no existing job is attached to the request.
    #[serde(default)]
    pub already_completed: bool,
}

impl MaintenanceReceipt {
    /// Construct a receipt only after authoritative durable acceptance.
    #[must_use]
    pub fn new(
        job_id: String,
        operation: MaintenanceOperation,
        origin: JobOrigin,
        dataset: Option<DatasetRef>,
        reused: bool,
    ) -> Self {
        Self {
            job_id,
            operation,
            resource_scope: ResourceScope::Backend,
            origin,
            dataset,
            reused,
            already_completed: false,
        }
    }

    /// Construct a successful no-op without claiming a new job or changing its origin.
    #[must_use]
    pub fn completed(
        operation: MaintenanceOperation,
        origin: JobOrigin,
        dataset: Option<DatasetRef>,
    ) -> Self {
        Self {
            already_completed: true,
            ..Self::new(String::new(), operation, origin, dataset, true)
        }
    }
}

/// Safe bounded-input error; raw paths, hashes and uploaded content are not echoed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum MaintenanceInputError {
    InvalidRequestId,
    InvalidDatasetId,
    InvalidDatasetHash,
    EmptyDataset,
    WrongDatasetOrEngine,
}

impl fmt::Display for MaintenanceInputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidRequestId => {
                "maintenance request ID must contain 1–64 identifier characters"
            }
            Self::InvalidDatasetId => "invalid opaque dataset ID",
            Self::InvalidDatasetHash => "dataset requires a SHA-256 digest",
            Self::EmptyDataset => "dataset bytes are empty",
            Self::WrongDatasetOrEngine => "dataset purpose or engine does not match this operation",
        })
    }
}

impl std::error::Error for MaintenanceInputError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::RegisteredContext;

    fn mutation() -> MutationContext {
        let registration = RegistrationReceipt::new(
            "deployment-a".into(),
            "node-a".into(),
            "registration-a".into(),
            None,
            1,
        );
        let context =
            RegisteredContext::new(registration, 1, 1, "reset-1".into(), "access-1".into())
                .expect("context");
        MutationContext::new(&context)
    }

    fn dataset(kind: DatasetKind) -> DatasetRef {
        DatasetRef::new("dataset-1".into(), kind, "a".repeat(64), 1024).expect("dataset reference")
    }

    #[test]
    fn seeding_training_and_reindex_have_distinct_dataset_contracts() {
        assert!(MaintenanceRequest::for_seed(
            "request-1".into(),
            mutation(),
            dataset(DatasetKind::ActorCatalog)
        )
        .is_ok());
        assert!(MaintenanceRequest::for_training(
            "request-1".into(),
            mutation(),
            dataset(DatasetKind::ClassifierTraining)
        )
        .is_ok());
        assert!(MaintenanceRequest::for_reindex("request-1".into(), mutation()).is_ok());
        assert_eq!(
            MaintenanceRequest::for_seed(
                "request-1".into(),
                mutation(),
                dataset(DatasetKind::ClassifierTraining)
            ),
            Err(MaintenanceInputError::WrongDatasetOrEngine)
        );
        assert_eq!(
            MaintenanceRequest::for_training(
                "request-1".into(),
                mutation(),
                dataset(DatasetKind::ActorCatalog)
            ),
            Err(MaintenanceInputError::WrongDatasetOrEngine)
        );
        assert_eq!(
            MaintenanceRequest::for_reindex("../request".into(), mutation()),
            Err(MaintenanceInputError::InvalidRequestId)
        );
    }

    #[test]
    fn training_never_silently_activates_or_accepts_local_paths() {
        let request = MaintenanceRequest::for_training(
            "request-1".into(),
            mutation(),
            dataset(DatasetKind::ClassifierTraining),
        )
        .expect("training request");
        let body = serde_json::to_value(request).expect("serialize training");
        assert_eq!(body["engine"], "natural");
        assert!(body.get("activate").is_none());
        for field in [
            "activate",
            "path",
            "server_path",
            "command",
            "origin",
            "source",
        ] {
            let mut forged = body.clone();
            forged[field] = serde_json::json!("forged");
            assert!(
                serde_json::from_value::<MaintenanceRequest>(forged).is_err(),
                "field {field}"
            );
        }
        assert!(DatasetRef::new(
            "/Users/example/catalog.csv".into(),
            DatasetKind::ActorCatalog,
            "a".repeat(64),
            1
        )
        .is_err());
        assert!(DatasetRef::new(
            "dataset-1".into(),
            DatasetKind::ActorCatalog,
            "not-a-hash".into(),
            1
        )
        .is_err());
        assert!(DatasetRef::new(
            "dataset-1".into(),
            DatasetKind::ActorCatalog,
            "a".repeat(64),
            0
        )
        .is_err());
    }

    #[test]
    fn unseeded_catalog_and_unavailable_search_remain_separate_checks() {
        let at = "2026-01-01T00:00:00Z".to_owned();
        let mut catalog = ReadinessCheck::new(
            ReadinessCheckId::ActorCatalog,
            ReadinessState::NotReady,
            "needs_seed".into(),
            "No seed receipt".into(),
            at.clone(),
        );
        catalog.actions.push(ReadinessAction::new(
            ReadinessActionId::SeedActors,
            Permission::MaintainBackend,
            true,
        ));
        let search = ReadinessCheck::new(
            ReadinessCheckId::ActorSearch,
            ReadinessState::Unavailable,
            "search_unavailable".into(),
            "Search owner unavailable".into(),
            at.clone(),
        );
        let aggregate = WorkstationReadiness::new("deployment-a".into(), at, vec![catalog, search]);
        assert_eq!(aggregate.checks[0].state, ReadinessState::NotReady);
        assert!(
            aggregate.checks[0].actions[0].allowed,
            "unseeded state must permit the repair action"
        );
        assert_eq!(aggregate.checks[1].state, ReadinessState::Unavailable);
        assert_eq!(
            aggregate.checks[0].fresh_for_ms, 0,
            "missing freshness must not manufacture a green result"
        );
    }

    #[test]
    fn actor_reindex_result_round_trips_safe_fields_only() {
        let result = ActorReindexResult::new(
            Some("job-orig-1".into()),
            Some("gen-7".into()),
            Some(99),
            250,
            true,
        );
        let value = serde_json::to_value(&result).expect("serialize");
        assert_eq!(value["operation"], ACTOR_REINDEX_OPERATION);
        let decoded: ActorReindexResult = serde_json::from_value(value).expect("deserialize");
        assert_eq!(decoded, result);

        let mut forged = serde_json::to_value(&result).expect("serialize");
        forged["server_path"] = serde_json::json!("/private/index");
        assert!(serde_json::from_value::<ActorReindexResult>(forged).is_err());
    }

    #[test]
    fn platform_identity_is_omitted_when_unset_and_keeps_the_deployment_id() {
        let plain = WorkstationReadiness::new(
            "deployment-a".into(),
            "2026-11-16T00:00:00Z".into(),
            Vec::new(),
        );
        let value = serde_json::to_value(&plain).expect("serialize");
        assert!(value.get("platform_identity").is_none());
        let older: WorkstationReadiness = serde_json::from_value(value).expect("older shape");
        assert!(older.platform_identity.is_none());

        let named = plain.with_platform_identity("Studio rack".into());
        let value = serde_json::to_value(&named).expect("serialize");
        assert_eq!(value["platform_identity"]["deployment_id"], "deployment-a");
        assert_eq!(value["platform_identity"]["display_name"], "Studio rack");
        let decoded: WorkstationReadiness =
            serde_json::from_value(value.clone()).expect("round trip");
        assert_eq!(decoded, named);

        let mut forged = value;
        forged["platform_identity"]["hostname"] = serde_json::json!("host.local");
        assert!(serde_json::from_value::<WorkstationReadiness>(forged).is_err());
    }
}
