//! Versioned, length-delimited typed IPC framing and multiplexed transport.
//!
//! This module provides local pipe/stream communication between a native
//! workstation client and an attached backend process without HTTP headers,
//! URLs, or bearer tokens on the wire.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::fmt;
use std::net::IpAddr;
use std::process::Stdio;
use std::str::FromStr;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use serde::de;
use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use tokio::io::AsyncRead;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWrite;
use tokio::io::AsyncWriteExt;
use tokio::process::Child;
use tokio::process::Command;
use tokio::sync::broadcast;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio::sync::Semaphore;
use tokio::task::JoinHandle;

use crate::capabilities::Capabilities;
use crate::classifier_activation::ActivationRequest;
use crate::classifier_activation::ACTIVATION_PATH;
use crate::classifier_activation::ACTIVATION_RESULT_PATH;
use crate::classifier_activation::CANDIDATE_DETAIL_PATH;
use crate::client::validate_route;
use crate::client::ClientError;
use crate::client::DatasetUpload;
use crate::client::MAX_REQUEST_BYTES;
use crate::client::MAX_RESPONSE_BYTES;
use crate::connect::RegistrationEnsureRequest;
use crate::connect::WebSessionCreateRequest;
use crate::connect::REGISTRATION_ENSURE_PATH;
use crate::connect::WEB_SESSION_CREATE_PATH;
use crate::context::RegisteredContext;
use crate::context::WorkstationRequestDetailQuery;
use crate::context::WORKSTATION_REQUEST_DETAIL_PATH;
use crate::cycle_history::DeleteCycleRequest;
use crate::cycle_history::CYCLE_HISTORY_DELETE_PATH;
use crate::cycle_history::CYCLE_HISTORY_PATH;
use crate::dataset_upload::DatasetUploadRequest;
use crate::dataset_upload::DATASET_UPLOADS_PATH;
use crate::operational_logs::LogQuery;
use crate::readiness::MaintenanceRequest;
use crate::readiness::ACTOR_SEED_PATH;
use crate::readiness::CLASSIFIER_TRAIN_PATH;
use crate::readiness::WORKSTATION_READINESS_PATH;
use crate::search::ActorSearchQuery;
use crate::search::ACTOR_DETAIL_PATH;
use crate::search::ACTOR_REINDEX_PATH;
use crate::search::ACTOR_SEARCH_PATH;
use crate::AtlasCycleRequest;
use crate::Ready;
use crate::ToolRunRequest;
use crate::ATLAS_CYCLES_PATH;
use crate::CAPABILITIES_PATH;
use crate::JOBS_PATH;
use crate::JOB_CANCEL_PATH;
use crate::JOB_DETAIL_PATH;
use crate::JOB_RETRY_PATH;
use crate::READY_PATH;
use crate::TOOL_RESULT_PATH;
use crate::TOOL_RUN_PATH;

/// Current local IPC wire protocol version.
pub const IPC_PROTOCOL_VERSION: u16 = 1;
/// Minimum supported local IPC wire protocol version.
pub const IPC_MIN_PROTOCOL_VERSION: u16 = 1;
/// Maximum serialized JSON payload bytes permitted in a single IPC frame.
pub const MAX_IPC_FRAME_BYTES: usize = 4 * 1024 * 1024;
/// Maximum raw binary chunk size per dataset upload chunk frame.
pub const MAX_IPC_CHUNK_BYTES: usize = 256 * 1024;
/// Maximum concurrent in-flight IPC requests multiplexed on one connection.
pub const MAX_IPC_INFLIGHT: usize = 64;
/// Maximum queued outbound frames awaiting the IPC writer task.
pub const MAX_IPC_QUEUE_DEPTH: usize = 64;
/// Maximum retained stderr bytes captured from an attached child process.
pub const MAX_IPC_STDERR_BYTES: usize = 64 * 1024;

const DEFAULT_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(50);
const DEFAULT_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);
const EVENT_CHANNEL_CAPACITY: usize = 64;

const DATASET_UPLOAD_CHUNK_PATH: &str = "/api/v1/workstation/datasets/uploads/chunk";
const DATASET_UPLOAD_FINISH_PATH: &str = "/api/v1/workstation/datasets/uploads/finish";
const HOST_METRICS_PATH: &str = "/api/v1/host/metrics";
const WORKSTATION_DATASETS_PATH: &str = "/api/v1/workstation/datasets";
const TOOLS_PATH: &str = "/api/v1/tools";
const MODELS_DEFAULTS_PATH: &str = "/api/v1/models/defaults";
const MODELS_DEFAULTS_SET_PATH: &str = "/api/v1/models/defaults/set";
const PROFILE_PATH: &str = "/api/v1/profile";
const PROFILE_ACCOUNTS_UPSERT_PATH: &str = "/api/v1/profile/accounts/upsert";
const PROFILE_ACCOUNTS_DELETE_PATH: &str = "/api/v1/profile/accounts/delete";
const PROFILE_TOOLS_SET_PATH: &str = "/api/v1/profile/tools/set";
const PROFILE_EXPORT_PATH: &str = "/api/v1/profile/export";
const PROFILE_IMPORT_PATH: &str = "/api/v1/profile/import";
const ARTICLES_PATH: &str = "/api/v1/articles";
const LOGS_PATH: &str = "/api/v1/logs";
const LOGS_FOLLOW_PATH: &str = "/api/v1/logs/follow";
const LOGS_EXPORT_PATH: &str = "/api/v1/logs/export";
const REGISTRATION_CONTEXT_PATH: &str = "/api/v1/connect/registrations/context";
const REGISTRATION_REBIND_PATH: &str = "/api/v1/connect/registrations/rebind";
const REGISTRATION_RETIRE_PATH: &str = "/api/v1/connect/registrations/retire";

/// Failures on the local IPC framing, handshake, or multiplexed pipe transport.
///
/// Diagnostics never echo raw payloads, credentials, or filesystem paths.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[must_use]
#[non_exhaustive]
pub enum IpcError {
    ZeroLengthFrame,
    FrameTooLarge,
    Truncated,
    InvalidPayload,
    PipeClosed,
    UnsupportedVersion(u16),
    HandshakeFailed,
    UnexpectedFrame,
    QueueFull,
    InflightFull,
    Timeout,
    Io(std::io::ErrorKind),
}

impl fmt::Display for IpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::ZeroLengthFrame => write!(f, "IPC frame length prefix was zero"),
            Self::FrameTooLarge => write!(f, "IPC frame exceeds maximum allowed size"),
            Self::Truncated => write!(f, "IPC frame was truncated before completion"),
            Self::InvalidPayload => write!(f, "IPC frame payload is malformed"),
            Self::PipeClosed => write!(f, "local IPC pipe is closed"),
            Self::UnsupportedVersion(version) => {
                write!(f, "unsupported local IPC protocol version {version}")
            }
            Self::HandshakeFailed => write!(f, "local IPC handshake was rejected"),
            Self::UnexpectedFrame => write!(f, "unexpected local IPC frame"),
            Self::QueueFull => write!(f, "local IPC outbound queue is saturated"),
            Self::InflightFull => write!(f, "local IPC in-flight request limit reached"),
            Self::Timeout => write!(f, "local IPC operation timed out"),
            Self::Io(_) => write!(f, "local IPC stream I/O failed"),
        }
    }
}

impl std::error::Error for IpcError {}

impl From<IpcError> for ClientError {
    fn from(error: IpcError) -> Self {
        match error {
            IpcError::UnsupportedVersion(version) => Self::UnsupportedVersion(version),
            IpcError::FrameTooLarge => Self::ResponseTooLarge,
            IpcError::ZeroLengthFrame | IpcError::InvalidPayload | IpcError::UnexpectedFrame => {
                Self::InvalidResponse
            }
            IpcError::QueueFull | IpcError::InflightFull => Self::Status(503),
            IpcError::Timeout => Self::Timeout,
            IpcError::Truncated
            | IpcError::PipeClosed
            | IpcError::HandshakeFailed
            | IpcError::Io(_) => Self::Transport,
        }
    }
}

/// Behavior when `MAX_IPC_INFLIGHT` or `MAX_IPC_QUEUE_DEPTH` is saturated.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum BackpressurePolicy {
    /// Return `IpcError::InflightFull` / `IpcError::QueueFull` immediately.
    #[default]
    FailFast,
    /// Wait up to the given duration for capacity before failing.
    Wait(Duration),
}

/// Client greeting sent immediately after attaching to the local IPC stream.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct IpcHello {
    pub protocol_version: u16,
    pub min_protocol_version: u16,
    pub client_name: String,
    pub client_version: String,
}

impl IpcHello {
    /// Construct a protocol-v1 greeting.
    #[must_use]
    pub fn new(client_name: String, client_version: String) -> Self {
        Self {
            protocol_version: IPC_PROTOCOL_VERSION,
            min_protocol_version: IPC_MIN_PROTOCOL_VERSION,
            client_name,
            client_version,
        }
    }
}

/// Server handshake response carrying version negotiation and initial metadata.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct IpcHelloAck {
    pub protocol_version: u16,
    pub min_protocol_version: u16,
    pub compatible: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<Capabilities>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ready: Option<Ready>,
}

impl IpcHelloAck {
    /// Construct a compatible protocol-v1 acknowledgment.
    #[must_use]
    pub fn compatible(capabilities: Option<Capabilities>, ready: Option<Ready>) -> Self {
        Self {
            protocol_version: IPC_PROTOCOL_VERSION,
            min_protocol_version: IPC_MIN_PROTOCOL_VERSION,
            compatible: true,
            reason: None,
            capabilities,
            ready,
        }
    }

    /// Construct an incompatible handshake acknowledgment.
    #[must_use]
    pub fn incompatible(protocol_version: u16, reason: Option<String>) -> Self {
        Self {
            protocol_version,
            min_protocol_version: IPC_MIN_PROTOCOL_VERSION,
            compatible: false,
            reason,
            capabilities: None,
            ready: None,
        }
    }
}

/// Graceful shutdown completion reported by the attached backend process.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ShutdownAck {
    pub checkpointed: bool,
    pub relinquished_leases: usize,
}

/// Unsolicited backend lifecycle and progress events pushed over the IPC stream.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
#[non_exhaustive]
pub enum IpcEvent {
    JobProgress {
        job_id: String,
        phase: String,
        summary: String,
    },
    WorkstationReadinessChanged,
    PressurePaused {
        reason_code: String,
    },
    ShutdownNotice {
        checkpointed: bool,
        relinquished_leases: usize,
    },
}

/// Top-level versioned wire frame serialized inside each length-delimited frame.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "frame", content = "body", rename_all = "snake_case")]
#[non_exhaustive]
pub enum IpcEnvelope {
    Hello(IpcHello),
    HelloAck(IpcHelloAck),
    Request {
        request_id: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        registration: Option<RegisteredContext>,
        op: IpcOperation,
    },
    Response {
        request_id: u64,
        outcome: IpcOutcome,
    },
    Event(IpcEvent),
    Shutdown {
        reason: String,
    },
    ShutdownAck {
        checkpointed: bool,
        relinquished_leases: usize,
    },
}

/// Logical method classification for route mapping helpers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum IpcMethod {
    Get,
    Post,
}

/// Query for `/api/v1/actors/detail`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ActorDetailQuery {
    pub id: String,
}

/// Query for `/api/v1/articles`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ArticlesListQuery {
    pub day: String,
    pub category: String,
    pub search: String,
    pub sort: String,
    #[serde(deserialize_with = "deserialize_num_or_str")]
    pub limit: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

/// Query for `/api/v1/jobs`.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct JobsListQuery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(
        default,
        deserialize_with = "deserialize_opt_num_or_str",
        skip_serializing_if = "Option::is_none"
    )]
    pub limit: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
}

/// Query for `/api/v1/jobs/detail`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct JobDetailQuery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(
        default,
        deserialize_with = "deserialize_opt_num_or_str",
        skip_serializing_if = "Option::is_none"
    )]
    pub limit: Option<u16>,
}

/// Query for `/api/v1/tools/result`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ToolResultQuery {
    pub id: String,
}

/// Query for `/api/v1/logs` when reading job-scoped activity logs.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct JobLogsQuery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(deserialize_with = "deserialize_num_or_str")]
    pub limit: u16,
}

/// Query for `/api/v1/atlas/cycles`.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct CycleHistoryQuery {
    #[serde(
        default,
        deserialize_with = "deserialize_opt_num_or_str",
        skip_serializing_if = "Option::is_none"
    )]
    pub limit: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

/// Query for `/api/v1/classifier/candidates/detail`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct CandidateDetailQuery {
    pub training_job_id: String,
    pub model_id: String,
}

/// Query for `/api/v1/classifier/activation/result`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ActivationResultQuery {
    pub job_id: String,
}

/// Request payload for `/api/v1/connect/registrations/context`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct RegistrationContextRequest {
    pub registration_id: String,
    pub expected_revision: u64,
}

/// Request payload for `/api/v1/connect/registrations/rebind`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct RegistrationRebindRequest {
    pub request_id: String,
    pub registration_id: String,
    pub expected_revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workstation_ip: Option<IpAddr>,
    pub installation_marker: String,
}

/// Request payload for `/api/v1/connect/registrations/retire`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct RegistrationRetireRequest {
    pub registration_id: String,
    pub expected_revision: u64,
}

/// Request payload for `/api/v1/jobs/retry` and `/api/v1/jobs/cancel`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct JobActionRequest {
    pub id: String,
    pub request_id: String,
}

/// Request payload for `/api/v1/models/defaults/set`.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ModelRoleSetRequest {
    pub expected_revision: u64,
    pub role: String,
    pub primary: Option<serde_json::Value>,
    pub fallbacks: Vec<serde_json::Value>,
    pub variant_updates: BTreeMap<String, Option<String>>,
}

/// Secret-bearing request payload for `/api/v1/profile/accounts/upsert`.
#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ProfileAccountUpsertRequest {
    pub expected_revision: u64,
    pub account_id: String,
    pub provider: String,
    pub base_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
}

impl fmt::Debug for ProfileAccountUpsertRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProfileAccountUpsertRequest")
            .field("expected_revision", &self.expected_revision)
            .field("account_id", &self.account_id)
            .field("provider", &self.provider)
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field("has_api_key", &self.api_key.is_some())
            .finish()
    }
}

/// Request payload for `/api/v1/profile/accounts/delete`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ProfileAccountDeleteRequest {
    pub expected_revision: u64,
    pub account_id: String,
}

/// Secret-bearing request payload for `/api/v1/profile/tools/set`.
#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ProfileToolsSetRequest {
    pub expected_revision: u64,
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<String>,
}

impl fmt::Debug for ProfileToolsSetRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProfileToolsSetRequest")
            .field("expected_revision", &self.expected_revision)
            .field("provider", &self.provider)
            .field("has_primary", &self.primary.is_some())
            .field("has_fallback", &self.fallback.is_some())
            .finish()
    }
}

/// Request payload for `/api/v1/profile/export`.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ConfigurationExportRequest {
    pub expected_revision: u64,
    #[serde(default)]
    pub include_credentials: bool,
}

/// Strongly typed RPC operation covering every supported Fleetworks API action.
#[derive(Clone, Deserialize, PartialEq, Serialize)]
#[serde(tag = "op", content = "args", rename_all = "snake_case")]
#[non_exhaustive]
pub enum IpcOperation {
    Ready,
    Capabilities,
    WorkstationReadiness,
    HostMetrics,
    DatasetsList,
    ToolsList,
    ModelsDefaults,
    ProfileRead,
    ActorsSearch(ActorSearchQuery),
    ActorDetail(ActorDetailQuery),
    ArticlesList(ArticlesListQuery),
    JobsList(JobsListQuery),
    JobDetail(JobDetailQuery),
    ToolResult(ToolResultQuery),
    LogsPage(LogQuery),
    JobLogs(JobLogsQuery),
    LogsFollow(LogQuery),
    LogsExport(LogQuery),
    WorkstationRequestDetail(WorkstationRequestDetailQuery),
    CycleHistory(CycleHistoryQuery),
    ClassifierCandidateDetail(CandidateDetailQuery),
    ClassifierActivationResult(ActivationResultQuery),
    RegistrationEnsure(RegistrationEnsureRequest),
    RegistrationContext(RegistrationContextRequest),
    RegistrationRebind(RegistrationRebindRequest),
    RegistrationRetire(RegistrationRetireRequest),
    WebSessionCreate(WebSessionCreateRequest),
    AtlasCycleSubmit(AtlasCycleRequest),
    CycleDelete(DeleteCycleRequest),
    ToolRun(ToolRunRequest),
    JobRetry(JobActionRequest),
    JobCancel(JobActionRequest),
    DatasetUploadStart(DatasetUploadRequest),
    DatasetUploadChunk {
        upload_id: String,
        offset: u64,
        data_b64: String,
    },
    DatasetUploadFinish {
        upload_id: String,
    },
    SeedActors(MaintenanceRequest),
    TrainClassifier(MaintenanceRequest),
    ReindexActors(MaintenanceRequest),
    ClassifierActivate(ActivationRequest),
    ModelRoleSet(ModelRoleSetRequest),
    ProfileAccountUpsert(ProfileAccountUpsertRequest),
    ProfileAccountDelete(ProfileAccountDeleteRequest),
    ProfileToolsSet(ProfileToolsSetRequest),
    ConfigurationExport {
        expected_revision: u64,
    },
    ConfigurationImport {
        expected_revision: u64,
        bytes_b64: String,
    },
}

impl fmt::Debug for IpcOperation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ready => f.write_str("Ready"),
            Self::Capabilities => f.write_str("Capabilities"),
            Self::WorkstationReadiness => f.write_str("WorkstationReadiness"),
            Self::HostMetrics => f.write_str("HostMetrics"),
            Self::DatasetsList => f.write_str("DatasetsList"),
            Self::ToolsList => f.write_str("ToolsList"),
            Self::ModelsDefaults => f.write_str("ModelsDefaults"),
            Self::ProfileRead => f.write_str("ProfileRead"),
            Self::ActorsSearch(q) => f.debug_tuple("ActorsSearch").field(q).finish(),
            Self::ActorDetail(q) => f.debug_tuple("ActorDetail").field(q).finish(),
            Self::ArticlesList(q) => f.debug_tuple("ArticlesList").field(q).finish(),
            Self::JobsList(q) => f.debug_tuple("JobsList").field(q).finish(),
            Self::JobDetail(q) => f.debug_tuple("JobDetail").field(q).finish(),
            Self::ToolResult(q) => f.debug_tuple("ToolResult").field(q).finish(),
            Self::LogsPage(q) => f.debug_tuple("LogsPage").field(q).finish(),
            Self::JobLogs(q) => f.debug_tuple("JobLogs").field(q).finish(),
            Self::LogsFollow(q) => f.debug_tuple("LogsFollow").field(q).finish(),
            Self::LogsExport(q) => f.debug_tuple("LogsExport").field(q).finish(),
            Self::WorkstationRequestDetail(q) => {
                f.debug_tuple("WorkstationRequestDetail").field(q).finish()
            }
            Self::CycleHistory(q) => f.debug_tuple("CycleHistory").field(q).finish(),
            Self::ClassifierCandidateDetail(q) => {
                f.debug_tuple("ClassifierCandidateDetail").field(q).finish()
            }
            Self::ClassifierActivationResult(q) => f
                .debug_tuple("ClassifierActivationResult")
                .field(q)
                .finish(),
            Self::RegistrationEnsure(r) => f.debug_tuple("RegistrationEnsure").field(r).finish(),
            Self::RegistrationContext(r) => f.debug_tuple("RegistrationContext").field(r).finish(),
            Self::RegistrationRebind(r) => f.debug_tuple("RegistrationRebind").field(r).finish(),
            Self::RegistrationRetire(r) => f.debug_tuple("RegistrationRetire").field(r).finish(),
            Self::WebSessionCreate(r) => f.debug_tuple("WebSessionCreate").field(r).finish(),
            Self::AtlasCycleSubmit(r) => f.debug_tuple("AtlasCycleSubmit").field(r).finish(),
            Self::CycleDelete(r) => f.debug_tuple("CycleDelete").field(r).finish(),
            Self::ToolRun(r) => f.debug_tuple("ToolRun").field(r).finish(),
            Self::JobRetry(r) => f.debug_tuple("JobRetry").field(r).finish(),
            Self::JobCancel(r) => f.debug_tuple("JobCancel").field(r).finish(),
            Self::DatasetUploadStart(r) => f.debug_tuple("DatasetUploadStart").field(r).finish(),
            Self::DatasetUploadChunk {
                upload_id,
                offset,
                data_b64,
            } => f
                .debug_struct("DatasetUploadChunk")
                .field("upload_id", upload_id)
                .field("offset", offset)
                .field("encoded_len", &data_b64.len())
                .finish(),
            Self::DatasetUploadFinish { upload_id } => f
                .debug_struct("DatasetUploadFinish")
                .field("upload_id", upload_id)
                .finish(),
            Self::SeedActors(r) => f.debug_tuple("SeedActors").field(r).finish(),
            Self::TrainClassifier(r) => f.debug_tuple("TrainClassifier").field(r).finish(),
            Self::ReindexActors(r) => f.debug_tuple("ReindexActors").field(r).finish(),
            Self::ClassifierActivate(r) => f.debug_tuple("ClassifierActivate").field(r).finish(),
            Self::ModelRoleSet(r) => f.debug_tuple("ModelRoleSet").field(r).finish(),
            Self::ProfileAccountUpsert(r) => {
                f.debug_tuple("ProfileAccountUpsert").field(r).finish()
            }
            Self::ProfileAccountDelete(r) => {
                f.debug_tuple("ProfileAccountDelete").field(r).finish()
            }
            Self::ProfileToolsSet(r) => f.debug_tuple("ProfileToolsSet").field(r).finish(),
            Self::ConfigurationExport { expected_revision } => f
                .debug_struct("ConfigurationExport")
                .field("expected_revision", expected_revision)
                .finish(),
            Self::ConfigurationImport {
                expected_revision,
                bytes_b64,
            } => f
                .debug_struct("ConfigurationImport")
                .field("expected_revision", expected_revision)
                .field("encoded_len", &bytes_b64.len())
                .finish(),
        }
    }
}

impl IpcOperation {
    /// Map this typed IPC operation to its corresponding logical API method and route.
    #[must_use]
    pub fn into_typed_route(&self) -> (IpcMethod, &'static str) {
        match self {
            Self::Ready => (IpcMethod::Get, READY_PATH),
            Self::Capabilities => (IpcMethod::Get, CAPABILITIES_PATH),
            Self::WorkstationReadiness => (IpcMethod::Get, WORKSTATION_READINESS_PATH),
            Self::HostMetrics => (IpcMethod::Get, HOST_METRICS_PATH),
            Self::DatasetsList => (IpcMethod::Get, WORKSTATION_DATASETS_PATH),
            Self::ToolsList => (IpcMethod::Get, TOOLS_PATH),
            Self::ModelsDefaults => (IpcMethod::Get, MODELS_DEFAULTS_PATH),
            Self::ProfileRead => (IpcMethod::Get, PROFILE_PATH),
            Self::ActorsSearch(_) => (IpcMethod::Get, ACTOR_SEARCH_PATH),
            Self::ActorDetail(_) => (IpcMethod::Get, ACTOR_DETAIL_PATH),
            Self::ArticlesList(_) => (IpcMethod::Get, ARTICLES_PATH),
            Self::JobsList(_) => (IpcMethod::Get, JOBS_PATH),
            Self::JobDetail(_) => (IpcMethod::Get, JOB_DETAIL_PATH),
            Self::ToolResult(_) => (IpcMethod::Get, TOOL_RESULT_PATH),
            Self::LogsPage(_) | Self::JobLogs(_) => (IpcMethod::Get, LOGS_PATH),
            Self::LogsFollow(_) => (IpcMethod::Get, LOGS_FOLLOW_PATH),
            Self::LogsExport(_) => (IpcMethod::Get, LOGS_EXPORT_PATH),
            Self::WorkstationRequestDetail(_) => (IpcMethod::Get, WORKSTATION_REQUEST_DETAIL_PATH),
            Self::CycleHistory(_) => (IpcMethod::Get, CYCLE_HISTORY_PATH),
            Self::ClassifierCandidateDetail(_) => (IpcMethod::Get, CANDIDATE_DETAIL_PATH),
            Self::ClassifierActivationResult(_) => (IpcMethod::Get, ACTIVATION_RESULT_PATH),
            Self::RegistrationEnsure(_) => (IpcMethod::Post, REGISTRATION_ENSURE_PATH),
            Self::RegistrationContext(_) => (IpcMethod::Post, REGISTRATION_CONTEXT_PATH),
            Self::RegistrationRebind(_) => (IpcMethod::Post, REGISTRATION_REBIND_PATH),
            Self::RegistrationRetire(_) => (IpcMethod::Post, REGISTRATION_RETIRE_PATH),
            Self::WebSessionCreate(_) => (IpcMethod::Post, WEB_SESSION_CREATE_PATH),
            Self::AtlasCycleSubmit(_) => (IpcMethod::Post, ATLAS_CYCLES_PATH),
            Self::CycleDelete(_) => (IpcMethod::Post, CYCLE_HISTORY_DELETE_PATH),
            Self::ToolRun(_) => (IpcMethod::Post, TOOL_RUN_PATH),
            Self::JobRetry(_) => (IpcMethod::Post, JOB_RETRY_PATH),
            Self::JobCancel(_) => (IpcMethod::Post, JOB_CANCEL_PATH),
            Self::DatasetUploadStart(_) => (IpcMethod::Post, DATASET_UPLOADS_PATH),
            Self::DatasetUploadChunk { .. } => (IpcMethod::Post, DATASET_UPLOAD_CHUNK_PATH),
            Self::DatasetUploadFinish { .. } => (IpcMethod::Post, DATASET_UPLOAD_FINISH_PATH),
            Self::SeedActors(_) => (IpcMethod::Post, ACTOR_SEED_PATH),
            Self::TrainClassifier(_) => (IpcMethod::Post, CLASSIFIER_TRAIN_PATH),
            Self::ReindexActors(_) => (IpcMethod::Post, ACTOR_REINDEX_PATH),
            Self::ClassifierActivate(_) => (IpcMethod::Post, ACTIVATION_PATH),
            Self::ModelRoleSet(_) => (IpcMethod::Post, MODELS_DEFAULTS_SET_PATH),
            Self::ProfileAccountUpsert(_) => (IpcMethod::Post, PROFILE_ACCOUNTS_UPSERT_PATH),
            Self::ProfileAccountDelete(_) => (IpcMethod::Post, PROFILE_ACCOUNTS_DELETE_PATH),
            Self::ProfileToolsSet(_) => (IpcMethod::Post, PROFILE_TOOLS_SET_PATH),
            Self::ConfigurationExport { .. } => (IpcMethod::Post, PROFILE_EXPORT_PATH),
            Self::ConfigurationImport { .. } => (IpcMethod::Post, PROFILE_IMPORT_PATH),
        }
    }

    /// Translate a parameterless GET route into a typed `IpcOperation`, failing closed on unknown routes.
    pub fn from_get(route: &str) -> Result<Self, ClientError> {
        validate_route(route)?;
        match route {
            READY_PATH => Ok(Self::Ready),
            CAPABILITIES_PATH => Ok(Self::Capabilities),
            WORKSTATION_READINESS_PATH => Ok(Self::WorkstationReadiness),
            HOST_METRICS_PATH => Ok(Self::HostMetrics),
            WORKSTATION_DATASETS_PATH => Ok(Self::DatasetsList),
            TOOLS_PATH => Ok(Self::ToolsList),
            MODELS_DEFAULTS_PATH => Ok(Self::ModelsDefaults),
            PROFILE_PATH => Ok(Self::ProfileRead),
            JOBS_PATH => Ok(Self::JobsList(JobsListQuery::default())),
            CYCLE_HISTORY_PATH => Ok(Self::CycleHistory(CycleHistoryQuery::default())),
            LOGS_PATH => Ok(Self::LogsPage(LogQuery::default())),
            _ => Err(ClientError::InvalidRoute),
        }
    }

    /// Translate a GET route and query value into a typed `IpcOperation`, failing closed on unknown routes.
    pub fn from_get_with_query(
        route: &str,
        query: &(impl Serialize + ?Sized),
    ) -> Result<Self, ClientError> {
        validate_route(route)?;
        let value = query_to_json_object(query)?;
        match route {
            ACTOR_SEARCH_PATH => decode_query(value).map(Self::ActorsSearch),
            ACTOR_DETAIL_PATH => decode_query(value).map(Self::ActorDetail),
            ARTICLES_PATH => decode_query(value).map(Self::ArticlesList),
            JOBS_PATH => decode_query(value).map(Self::JobsList),
            JOB_DETAIL_PATH => decode_query(value).map(Self::JobDetail),
            TOOL_RESULT_PATH => decode_query(value).map(Self::ToolResult),
            LOGS_PATH => {
                if value.as_object().is_some_and(|map| map.contains_key("id")) {
                    decode_query(value).map(Self::JobLogs)
                } else {
                    decode_query(value).map(Self::LogsPage)
                }
            }
            LOGS_FOLLOW_PATH => decode_query(value).map(Self::LogsFollow),
            LOGS_EXPORT_PATH => decode_query(value).map(Self::LogsExport),
            WORKSTATION_REQUEST_DETAIL_PATH => {
                decode_query(value).map(Self::WorkstationRequestDetail)
            }
            CYCLE_HISTORY_PATH => decode_query(value).map(Self::CycleHistory),
            CANDIDATE_DETAIL_PATH => decode_query(value).map(Self::ClassifierCandidateDetail),
            ACTIVATION_RESULT_PATH => decode_query(value).map(Self::ClassifierActivationResult),
            _ => Err(ClientError::InvalidRoute),
        }
    }

    /// Translate a POST route and serialized JSON body into a typed `IpcOperation`, failing closed on unknown routes.
    pub fn from_post_bytes(route: &str, body_bytes: &[u8]) -> Result<Self, ClientError> {
        validate_route(route)?;
        if body_bytes.len() > MAX_REQUEST_BYTES {
            return Err(ClientError::RequestTooLarge);
        }
        match route {
            REGISTRATION_ENSURE_PATH => decode_body(body_bytes).map(Self::RegistrationEnsure),
            REGISTRATION_CONTEXT_PATH => decode_body(body_bytes).map(Self::RegistrationContext),
            REGISTRATION_REBIND_PATH => decode_body(body_bytes).map(Self::RegistrationRebind),
            REGISTRATION_RETIRE_PATH => decode_body(body_bytes).map(Self::RegistrationRetire),
            WEB_SESSION_CREATE_PATH => decode_body(body_bytes).map(Self::WebSessionCreate),
            ATLAS_CYCLES_PATH => decode_body(body_bytes).map(Self::AtlasCycleSubmit),
            CYCLE_HISTORY_DELETE_PATH => decode_body(body_bytes).map(Self::CycleDelete),
            TOOL_RUN_PATH => decode_body(body_bytes).map(Self::ToolRun),
            JOB_RETRY_PATH => decode_body(body_bytes).map(Self::JobRetry),
            JOB_CANCEL_PATH => decode_body(body_bytes).map(Self::JobCancel),
            DATASET_UPLOADS_PATH => decode_body(body_bytes).map(Self::DatasetUploadStart),
            ACTOR_SEED_PATH => decode_body(body_bytes).map(Self::SeedActors),
            CLASSIFIER_TRAIN_PATH => decode_body(body_bytes).map(Self::TrainClassifier),
            ACTOR_REINDEX_PATH => decode_body(body_bytes).map(Self::ReindexActors),
            ACTIVATION_PATH => decode_body(body_bytes).map(Self::ClassifierActivate),
            MODELS_DEFAULTS_SET_PATH => decode_body(body_bytes).map(Self::ModelRoleSet),
            PROFILE_ACCOUNTS_UPSERT_PATH => decode_body(body_bytes).map(Self::ProfileAccountUpsert),
            PROFILE_ACCOUNTS_DELETE_PATH => decode_body(body_bytes).map(Self::ProfileAccountDelete),
            PROFILE_TOOLS_SET_PATH => decode_body(body_bytes).map(Self::ProfileToolsSet),
            PROFILE_EXPORT_PATH => {
                let req: ConfigurationExportRequest = decode_body(body_bytes)?;
                Ok(Self::ConfigurationExport {
                    expected_revision: req.expected_revision,
                })
            }
            _ => Err(ClientError::InvalidRoute),
        }
    }

    /// Translate a POST route and serializable body into a typed `IpcOperation`.
    pub fn from_post(route: &str, body: &(impl Serialize + ?Sized)) -> Result<Self, ClientError> {
        validate_route(route)?;
        let body_bytes = serde_json::to_vec(body).map_err(|_| ClientError::InvalidRequest)?;
        Self::from_post_bytes(route, &body_bytes)
    }
}

/// Typed payload returned inside `IpcOutcome::Ok`.
#[derive(Clone, Deserialize, PartialEq, Serialize)]
#[serde(tag = "payload", content = "value", rename_all = "snake_case")]
#[non_exhaustive]
pub enum IpcPayload {
    Empty,
    Json(serde_json::Value),
    DatasetUpload(DatasetUpload),
    ConfigurationExport {
        bytes_b64: String,
    },
    LogExport {
        bytes_b64: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        next_cursor: Option<String>,
        apps_complete: bool,
    },
}

impl fmt::Debug for IpcPayload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("Empty"),
            Self::Json(value) => f.debug_tuple("Json").field(value).finish(),
            Self::DatasetUpload(upload) => f.debug_tuple("DatasetUpload").field(upload).finish(),
            Self::ConfigurationExport { bytes_b64 } => f
                .debug_struct("ConfigurationExport")
                .field("encoded_len", &bytes_b64.len())
                .finish(),
            Self::LogExport {
                bytes_b64,
                next_cursor,
                apps_complete,
            } => f
                .debug_struct("LogExport")
                .field("encoded_len", &bytes_b64.len())
                .field("has_continuation", &next_cursor.is_some())
                .field("apps_complete", apps_complete)
                .finish(),
        }
    }
}

/// Outcome of a single multiplexed IPC request.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "outcome", content = "data", rename_all = "snake_case")]
#[non_exhaustive]
pub enum IpcOutcome {
    Ok(IpcPayload),
    Err {
        status: u16,
        code: String,
        message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        retry_after_seconds: Option<u64>,
    },
}

/// Write a single 4-byte big-endian length-prefixed UTF-8 JSON envelope frame.
pub async fn write_frame<W>(writer: &mut W, envelope: &IpcEnvelope) -> Result<(), IpcError>
where
    W: AsyncWrite + Unpin,
{
    let payload = serde_json::to_vec(envelope).map_err(|_| IpcError::InvalidPayload)?;
    if payload.is_empty() || payload.len() > MAX_IPC_FRAME_BYTES {
        return Err(IpcError::FrameTooLarge);
    }
    let len = u32::try_from(payload.len()).map_err(|_| IpcError::FrameTooLarge)?;
    writer
        .write_all(&len.to_be_bytes())
        .await
        .map_err(map_write_io_error)?;
    writer
        .write_all(&payload)
        .await
        .map_err(map_write_io_error)?;
    writer.flush().await.map_err(map_write_io_error)?;
    Ok(())
}

/// Read a single 4-byte big-endian length-prefixed UTF-8 JSON envelope frame.
///
/// Returns `Ok(None)` only on clean EOF at the frame boundary. Truncated length
/// prefixes or truncated payloads return `Err(IpcError::Truncated)`. Zero-length
/// and oversized frames fail closed before payload buffer allocation.
pub async fn read_frame<R>(reader: &mut R) -> Result<Option<IpcEnvelope>, IpcError>
where
    R: AsyncRead + Unpin,
{
    let mut header = [0_u8; 4];
    let mut filled = 0;
    while filled < header.len() {
        let count = reader
            .read(&mut header[filled..])
            .await
            .map_err(|error| IpcError::Io(error.kind()))?;
        if count == 0 {
            if filled == 0 {
                return Ok(None);
            }
            return Err(IpcError::Truncated);
        }
        filled += count;
    }

    let len = u32::from_be_bytes(header) as usize;
    if len == 0 {
        return Err(IpcError::ZeroLengthFrame);
    }
    if len > MAX_IPC_FRAME_BYTES {
        return Err(IpcError::FrameTooLarge);
    }

    let mut payload = vec![0_u8; len];
    reader.read_exact(&mut payload).await.map_err(|error| {
        if error.kind() == std::io::ErrorKind::UnexpectedEof {
            IpcError::Truncated
        } else {
            IpcError::Io(error.kind())
        }
    })?;

    serde_json::from_slice(&payload)
        .map(Some)
        .map_err(|_| IpcError::InvalidPayload)
}

/// Encode binary bytes as standard RFC 4648 base64 without line breaks.
#[must_use]
pub(crate) fn base64_encode(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    if input.is_empty() {
        return String::new();
    }
    let mut out = Vec::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0];
        let b1 = if chunk.len() > 1 { chunk[1] } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] } else { 0 };
        let n = ((b0 as u32) << 16) | ((b1 as u32) << 8) | (b2 as u32);
        out.push(ALPHABET[((n >> 18) & 0x3f) as usize]);
        out.push(ALPHABET[((n >> 12) & 0x3f) as usize]);
        out.push(if chunk.len() > 1 {
            ALPHABET[((n >> 6) & 0x3f) as usize]
        } else {
            b'='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[(n & 0x3f) as usize]
        } else {
            b'='
        });
    }
    String::from_utf8(out).expect("base64 alphabet is valid ASCII")
}

/// Decode standard RFC 4648 base64 while enforcing `max_decoded_len` before allocation.
pub(crate) fn base64_decode(input: &str, max_decoded_len: usize) -> Result<Vec<u8>, IpcError> {
    fn sextet(byte: u8) -> Result<u8, IpcError> {
        match byte {
            b'A'..=b'Z' => Ok(byte - b'A'),
            b'a'..=b'z' => Ok(byte - b'a' + 26),
            b'0'..=b'9' => Ok(byte - b'0' + 52),
            b'+' => Ok(62),
            b'/' => Ok(63),
            _ => Err(IpcError::InvalidPayload),
        }
    }

    let bytes = input.as_bytes();
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    if !bytes.len().is_multiple_of(4) {
        return Err(IpcError::InvalidPayload);
    }
    let pad = if bytes.ends_with(b"==") {
        2
    } else if bytes.ends_with(b"=") {
        1
    } else {
        0
    };
    let decoded_len = (bytes.len() / 4) * 3 - pad;
    if decoded_len > max_decoded_len {
        return Err(IpcError::FrameTooLarge);
    }

    let mut out = Vec::with_capacity(decoded_len);
    for (idx, chunk) in bytes.chunks_exact(4).enumerate() {
        let is_last = (idx + 1) * 4 == bytes.len();
        let c0 = sextet(chunk[0])?;
        let c1 = sextet(chunk[1])?;
        if is_last && chunk[2] == b'=' {
            if chunk[3] != b'=' || (c1 & 0x0f) != 0 {
                return Err(IpcError::InvalidPayload);
            }
            out.push((c0 << 2) | (c1 >> 4));
        } else if is_last && chunk[3] == b'=' {
            let c2 = sextet(chunk[2])?;
            if (c2 & 0x03) != 0 {
                return Err(IpcError::InvalidPayload);
            }
            let n = ((c0 as u32) << 18) | ((c1 as u32) << 12) | ((c2 as u32) << 6);
            out.push((n >> 16) as u8);
            out.push((n >> 8) as u8);
        } else {
            let c2 = sextet(chunk[2])?;
            let c3 = sextet(chunk[3])?;
            let n = ((c0 as u32) << 18) | ((c1 as u32) << 12) | ((c2 as u32) << 6) | (c3 as u32);
            out.push((n >> 16) as u8);
            out.push((n >> 8) as u8);
            out.push(n as u8);
        }
    }
    Ok(out)
}

/// Alias for callers referring to the multiplexed local IPC transport type.
pub type LocalIpcTransport = LocalIpcClient;

type PendingMap = HashMap<u64, oneshot::Sender<Result<IpcOutcome, IpcError>>>;
type ShutdownWaiter = Arc<Mutex<Option<oneshot::Sender<Result<ShutdownAck, IpcError>>>>>;

struct LocalIpcInner {
    outbound_tx: mpsc::Sender<IpcEnvelope>,
    inflight: Arc<Semaphore>,
    pending: Arc<Mutex<PendingMap>>,
    shutdown_waiter: ShutdownWaiter,
    events_tx: broadcast::Sender<IpcEvent>,
    next_request_id: AtomicU64,
    closed: AtomicBool,
    capabilities: Option<Capabilities>,
    ready: Option<Ready>,
    stderr_buffer: Arc<Mutex<Vec<u8>>>,
    reader_task: Mutex<Option<JoinHandle<()>>>,
    writer_task: Mutex<Option<JoinHandle<()>>>,
    stderr_task: Mutex<Option<JoinHandle<()>>>,
    child: Mutex<Option<Child>>,
}

impl Drop for LocalIpcInner {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::Release);
        if let Ok(mut guard) = self.reader_task.lock() {
            if let Some(handle) = guard.take() {
                handle.abort();
            }
        }
        if let Ok(mut guard) = self.writer_task.lock() {
            if let Some(handle) = guard.take() {
                handle.abort();
            }
        }
        if let Ok(mut guard) = self.stderr_task.lock() {
            if let Some(handle) = guard.take() {
                handle.abort();
            }
        }
        if let Ok(mut guard) = self.child.lock() {
            if let Some(mut child) = guard.take() {
                let _ = child.start_kill();
            }
        }
    }
}

/// Multiplexed, bounded local IPC client over an inherited child pipe or async stream.
#[derive(Clone)]
pub struct LocalIpcClient {
    inner: Arc<LocalIpcInner>,
    request_timeout: Duration,
    backpressure: BackpressurePolicy,
}

impl fmt::Debug for LocalIpcClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LocalIpcClient")
            .field("closed", &self.inner.closed.load(Ordering::Acquire))
            .field("has_capabilities", &self.inner.capabilities.is_some())
            .field("has_ready", &self.inner.ready.is_some())
            .finish_non_exhaustive()
    }
}

impl LocalIpcClient {
    /// Connect over a single bidirectional async stream (for example, `tokio::io::DuplexStream`).
    pub async fn connect_stream<S>(
        stream: S,
        client_name: impl Into<String>,
        client_version: impl Into<String>,
    ) -> Result<Self, IpcError>
    where
        S: AsyncRead + AsyncWrite + Send + 'static,
    {
        let (reader, writer) = tokio::io::split(stream);
        Self::connect_streams(reader, writer, client_name, client_version).await
    }

    /// Connect over an async reader/writer pair and complete the `Hello`/`HelloAck` handshake.
    pub async fn connect_streams<R, W>(
        reader: R,
        writer: W,
        client_name: impl Into<String>,
        client_version: impl Into<String>,
    ) -> Result<Self, IpcError>
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        Self::connect_with_child(
            reader,
            writer,
            None::<tokio::io::Empty>,
            None,
            client_name.into(),
            client_version.into(),
        )
        .await
    }

    /// Attach to an already-spawned child process using its piped `stdin`, `stdout`, and optional `stderr`.
    pub async fn from_child(
        mut child: Child,
        client_name: impl Into<String>,
        client_version: impl Into<String>,
    ) -> Result<Self, IpcError> {
        let stdin = child.stdin.take().ok_or(IpcError::PipeClosed)?;
        let stdout = child.stdout.take().ok_or(IpcError::PipeClosed)?;
        let stderr = child.stderr.take();
        Self::connect_with_child(
            stdout,
            stdin,
            stderr,
            Some(child),
            client_name.into(),
            client_version.into(),
        )
        .await
    }

    /// Spawn a child command with piped stdio (`kill_on_drop(true)`) and complete the handshake.
    pub async fn spawn_command(
        command: &mut Command,
        client_name: impl Into<String>,
        client_version: impl Into<String>,
    ) -> Result<Self, IpcError> {
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let child = command
            .spawn()
            .map_err(|error| IpcError::Io(error.kind()))?;
        Self::from_child(child, client_name, client_version).await
    }

    async fn connect_with_child<R, W, E>(
        mut reader: R,
        mut writer: W,
        stderr: Option<E>,
        child: Option<Child>,
        client_name: String,
        client_version: String,
    ) -> Result<Self, IpcError>
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
        E: AsyncRead + Unpin + Send + 'static,
    {
        let stderr_buffer = Arc::new(Mutex::new(Vec::new()));
        let stderr_task = stderr.map(|err_stream| {
            let buffer = Arc::clone(&stderr_buffer);
            tokio::spawn(drain_bounded_stderr(err_stream, buffer))
        });

        let hello = IpcEnvelope::Hello(IpcHello::new(client_name, client_version));
        let handshake = async {
            write_frame(&mut writer, &hello).await?;
            match read_frame(&mut reader).await? {
                Some(IpcEnvelope::HelloAck(ack)) => Ok(ack),
                Some(_) => Err(IpcError::UnexpectedFrame),
                None => Err(IpcError::PipeClosed),
            }
        };
        let ack = tokio::time::timeout(DEFAULT_HANDSHAKE_TIMEOUT, handshake)
            .await
            .map_err(|_| IpcError::Timeout)??;

        if ack.protocol_version < IPC_MIN_PROTOCOL_VERSION
            || ack.protocol_version > IPC_PROTOCOL_VERSION
        {
            return Err(IpcError::UnsupportedVersion(ack.protocol_version));
        }
        if !ack.compatible {
            if ack.protocol_version != IPC_PROTOCOL_VERSION {
                return Err(IpcError::UnsupportedVersion(ack.protocol_version));
            }
            return Err(IpcError::HandshakeFailed);
        }

        let (outbound_tx, outbound_rx) = mpsc::channel(MAX_IPC_QUEUE_DEPTH);
        let (events_tx, _) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let shutdown_waiter = Arc::new(Mutex::new(None));

        let inner = Arc::new(LocalIpcInner {
            outbound_tx,
            inflight: Arc::new(Semaphore::new(MAX_IPC_INFLIGHT)),
            pending: Arc::clone(&pending),
            shutdown_waiter: Arc::clone(&shutdown_waiter),
            events_tx: events_tx.clone(),
            next_request_id: AtomicU64::new(1),
            closed: AtomicBool::new(false),
            capabilities: ack.capabilities,
            ready: ack.ready,
            stderr_buffer,
            reader_task: Mutex::new(None),
            writer_task: Mutex::new(None),
            stderr_task: Mutex::new(stderr_task),
            child: Mutex::new(child),
        });

        let writer_inner = Arc::clone(&inner);
        let writer_handle = tokio::spawn(run_writer(writer, outbound_rx, writer_inner));
        let reader_inner = Arc::clone(&inner);
        let reader_handle = tokio::spawn(run_reader(reader, reader_inner));

        *inner.writer_task.lock().expect("writer task lock") = Some(writer_handle);
        *inner.reader_task.lock().expect("reader task lock") = Some(reader_handle);

        Ok(Self {
            inner,
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
            backpressure: BackpressurePolicy::FailFast,
        })
    }

    /// Return a clone with a custom per-request timeout.
    #[must_use]
    pub fn with_request_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = timeout;
        self
    }

    /// Return a clone with a custom backpressure policy when capacity is full.
    #[must_use]
    pub fn with_backpressure_policy(mut self, policy: BackpressurePolicy) -> Self {
        self.backpressure = policy;
        self
    }

    /// Capabilities advertised during the `HelloAck` handshake, if provided.
    #[must_use]
    pub fn capabilities(&self) -> Option<&Capabilities> {
        self.inner.capabilities.as_ref()
    }

    /// Initial readiness advertised during the `HelloAck` handshake, if provided.
    #[must_use]
    pub fn ready_snapshot(&self) -> Option<&Ready> {
        self.inner.ready.as_ref()
    }

    /// Subscribe to unsolicited backend `IpcEvent` notifications.
    #[must_use]
    pub fn subscribe_events(&self) -> broadcast::Receiver<IpcEvent> {
        self.inner.events_tx.subscribe()
    }

    /// Return true if the underlying pipe has closed or shutdown has completed.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.inner.closed.load(Ordering::Acquire)
    }

    /// Return the bounded stderr bytes captured from an attached child process.
    #[must_use]
    pub fn captured_stderr(&self) -> Vec<u8> {
        self.inner
            .stderr_buffer
            .lock()
            .expect("stderr buffer lock")
            .clone()
    }

    /// Send a typed request and await its `IpcOutcome`.
    pub async fn execute(
        &self,
        registration: Option<RegisteredContext>,
        op: IpcOperation,
    ) -> Result<IpcOutcome, IpcError> {
        if self.is_closed() {
            return Err(IpcError::PipeClosed);
        }

        let _permit = match self.backpressure {
            BackpressurePolicy::FailFast => self
                .inner
                .inflight
                .clone()
                .try_acquire_owned()
                .map_err(|_| IpcError::InflightFull)?,
            BackpressurePolicy::Wait(limit) => {
                tokio::time::timeout(limit, self.inner.inflight.clone().acquire_owned())
                    .await
                    .map_err(|_| IpcError::InflightFull)?
                    .map_err(|_| IpcError::PipeClosed)?
            }
        };

        if self.is_closed() {
            return Err(IpcError::PipeClosed);
        }

        let request_id = self.inner.next_request_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.inner.pending.lock().expect("pending map lock");
            if self.is_closed() {
                return Err(IpcError::PipeClosed);
            }
            pending.insert(request_id, tx);
        }

        let frame = IpcEnvelope::Request {
            request_id,
            registration,
            op,
        };
        let enqueue_result = match self.backpressure {
            BackpressurePolicy::FailFast => {
                self.inner
                    .outbound_tx
                    .try_send(frame)
                    .map_err(|err| match err {
                        mpsc::error::TrySendError::Full(_) => IpcError::QueueFull,
                        mpsc::error::TrySendError::Closed(_) => IpcError::PipeClosed,
                    })
            }
            BackpressurePolicy::Wait(limit) => {
                match tokio::time::timeout(limit, self.inner.outbound_tx.send(frame)).await {
                    Ok(Ok(())) => Ok(()),
                    Ok(Err(_)) => Err(IpcError::PipeClosed),
                    Err(_) => Err(IpcError::QueueFull),
                }
            }
        };

        if let Err(err) = enqueue_result {
            self.inner
                .pending
                .lock()
                .expect("pending map lock")
                .remove(&request_id);
            return Err(err);
        }

        match tokio::time::timeout(self.request_timeout, rx).await {
            Ok(Ok(outcome)) => outcome,
            Ok(Err(_)) => Err(IpcError::PipeClosed),
            Err(_) => {
                self.inner
                    .pending
                    .lock()
                    .expect("pending map lock")
                    .remove(&request_id);
                Err(IpcError::Timeout)
            }
        }
    }

    /// Execute a typed operation and translate `IpcOutcome` into `IpcPayload` or `ClientError`.
    pub async fn request_payload(
        &self,
        registration: Option<RegisteredContext>,
        op: IpcOperation,
    ) -> Result<IpcPayload, ClientError> {
        let outcome = self
            .execute(registration, op)
            .await
            .map_err(ClientError::from)?;
        match outcome {
            IpcOutcome::Ok(payload) => Ok(payload),
            IpcOutcome::Err { status, .. } => Err(ClientError::Status(status)),
        }
    }

    /// Execute a typed operation and deserialize its JSON payload into `T`.
    pub async fn request_json<T: de::DeserializeOwned>(
        &self,
        registration: Option<RegisteredContext>,
        op: IpcOperation,
    ) -> Result<T, ClientError> {
        let payload = self.request_payload(registration, op).await?;
        let value = match payload {
            IpcPayload::Json(value) => value,
            IpcPayload::DatasetUpload(upload) => {
                serde_json::to_value(upload).map_err(|_| ClientError::InvalidResponse)?
            }
            IpcPayload::Empty => serde_json::Value::Null,
            IpcPayload::ConfigurationExport { .. } | IpcPayload::LogExport { .. } => {
                return Err(ClientError::InvalidResponse);
            }
        };
        if let Ok(encoded) = serde_json::to_vec(&value) {
            if encoded.len() > MAX_RESPONSE_BYTES {
                return Err(ClientError::ResponseTooLarge);
            }
        }
        serde_json::from_value(value).map_err(|_| ClientError::InvalidResponse)
    }

    /// Send a graceful `Shutdown` frame and await `ShutdownAck`.
    pub async fn shutdown(&self, reason: &str) -> Result<ShutdownAck, IpcError> {
        if self.is_closed() {
            return Err(IpcError::PipeClosed);
        }
        let (tx, rx) = oneshot::channel();
        {
            let mut waiter = self
                .inner
                .shutdown_waiter
                .lock()
                .expect("shutdown waiter lock");
            if waiter.is_some() {
                return Err(IpcError::UnexpectedFrame);
            }
            *waiter = Some(tx);
        }

        let frame = IpcEnvelope::Shutdown {
            reason: reason.to_owned(),
        };
        if let Err(err) = self.inner.outbound_tx.try_send(frame) {
            *self
                .inner
                .shutdown_waiter
                .lock()
                .expect("shutdown waiter lock") = None;
            return Err(match err {
                mpsc::error::TrySendError::Full(_) => IpcError::QueueFull,
                mpsc::error::TrySendError::Closed(_) => IpcError::PipeClosed,
            });
        }

        let result = match tokio::time::timeout(DEFAULT_SHUTDOWN_TIMEOUT, rx).await {
            Ok(Ok(res)) => res,
            Ok(Err(_)) => Err(IpcError::PipeClosed),
            Err(_) => {
                *self
                    .inner
                    .shutdown_waiter
                    .lock()
                    .expect("shutdown waiter lock") = None;
                Err(IpcError::Timeout)
            }
        };
        if result.is_ok() {
            close_inner(&self.inner, IpcError::PipeClosed);
        }
        result
    }
}

async fn run_writer<W>(
    mut writer: W,
    mut outbound_rx: mpsc::Receiver<IpcEnvelope>,
    inner: Arc<LocalIpcInner>,
) where
    W: AsyncWrite + Unpin + Send + 'static,
{
    while let Some(frame) = outbound_rx.recv().await {
        if let Err(err) = write_frame(&mut writer, &frame).await {
            close_inner(&inner, err);
            return;
        }
    }
    close_inner(&inner, IpcError::PipeClosed);
}

async fn run_reader<R>(mut reader: R, inner: Arc<LocalIpcInner>)
where
    R: AsyncRead + Unpin + Send + 'static,
{
    loop {
        match read_frame(&mut reader).await {
            Ok(Some(IpcEnvelope::Response {
                request_id,
                outcome,
            })) => {
                let tx = inner
                    .pending
                    .lock()
                    .expect("pending map lock")
                    .remove(&request_id);
                if let Some(tx) = tx {
                    let _ = tx.send(Ok(outcome));
                }
            }
            Ok(Some(IpcEnvelope::Event(event))) => {
                let _ = inner.events_tx.send(event);
            }
            Ok(Some(IpcEnvelope::ShutdownAck {
                checkpointed,
                relinquished_leases,
            })) => {
                let waiter = inner
                    .shutdown_waiter
                    .lock()
                    .expect("shutdown waiter lock")
                    .take();
                if let Some(tx) = waiter {
                    let _ = tx.send(Ok(ShutdownAck {
                        checkpointed,
                        relinquished_leases,
                    }));
                }
            }
            Ok(Some(_)) => {
                close_inner(&inner, IpcError::UnexpectedFrame);
                return;
            }
            Ok(None) => {
                close_inner(&inner, IpcError::PipeClosed);
                return;
            }
            Err(err) => {
                close_inner(&inner, err);
                return;
            }
        }
    }
}

fn close_inner(inner: &LocalIpcInner, reason: IpcError) {
    inner.closed.store(true, Ordering::Release);
    let pending: Vec<_> = {
        let mut guard = inner.pending.lock().expect("pending map lock");
        guard.drain().map(|(_, tx)| tx).collect()
    };
    for tx in pending {
        let _ = tx.send(Err(reason));
    }
    if let Some(tx) = inner
        .shutdown_waiter
        .lock()
        .expect("shutdown waiter lock")
        .take()
    {
        let _ = tx.send(Err(reason));
    }
}

async fn drain_bounded_stderr<R>(mut stderr: R, buffer: Arc<Mutex<Vec<u8>>>)
where
    R: AsyncRead + Unpin + Send + 'static,
{
    let mut chunk = [0_u8; 4096];
    loop {
        match stderr.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let mut guard = buffer.lock().expect("stderr buffer lock");
                if guard.len() < MAX_IPC_STDERR_BYTES {
                    let remaining = MAX_IPC_STDERR_BYTES - guard.len();
                    let keep = n.min(remaining);
                    guard.extend_from_slice(&chunk[..keep]);
                }
            }
        }
    }
}

fn map_write_io_error(error: std::io::Error) -> IpcError {
    if matches!(
        error.kind(),
        std::io::ErrorKind::BrokenPipe
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::UnexpectedEof
    ) {
        IpcError::PipeClosed
    } else {
        IpcError::Io(error.kind())
    }
}

fn query_to_json_object(
    query: &(impl Serialize + ?Sized),
) -> Result<serde_json::Value, ClientError> {
    let value = serde_json::to_value(query).map_err(|_| ClientError::InvalidRequest)?;
    match value {
        serde_json::Value::Object(_) => Ok(value),
        serde_json::Value::Null => Ok(serde_json::Value::Object(serde_json::Map::new())),
        serde_json::Value::Array(items) => {
            let mut map = serde_json::Map::new();
            for item in items {
                let serde_json::Value::Array(mut pair) = item else {
                    return Err(ClientError::InvalidRequest);
                };
                if pair.len() != 2 {
                    return Err(ClientError::InvalidRequest);
                }
                let val = pair.pop().expect("pair has 2 items");
                let serde_json::Value::String(key) = pair.pop().expect("pair has 2 items") else {
                    return Err(ClientError::InvalidRequest);
                };
                map.insert(key, val);
            }
            Ok(serde_json::Value::Object(map))
        }
        _ => Err(ClientError::InvalidRequest),
    }
}

fn decode_query<T: de::DeserializeOwned>(value: serde_json::Value) -> Result<T, ClientError> {
    serde_json::from_value(value).map_err(|_| ClientError::InvalidRequest)
}

fn decode_body<T: de::DeserializeOwned>(bytes: &[u8]) -> Result<T, ClientError> {
    serde_json::from_slice(bytes).map_err(|_| ClientError::InvalidRequest)
}

fn deserialize_num_or_str<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: FromStr + TryFrom<u64>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    parse_num_or_str::<T, D::Error>(value)
}

fn deserialize_opt_num_or_str<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: FromStr + TryFrom<u64>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    match value {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(other) => parse_num_or_str::<T, D::Error>(other).map(Some),
    }
}

fn parse_num_or_str<T, E>(value: serde_json::Value) -> Result<T, E>
where
    T: FromStr + TryFrom<u64>,
    E: de::Error,
{
    match value {
        serde_json::Value::Number(num) => num
            .as_u64()
            .and_then(|n| T::try_from(n).ok())
            .ok_or_else(|| de::Error::custom("integer out of range")),
        serde_json::Value::String(s) => s
            .parse::<T>()
            .map_err(|_| de::Error::custom("invalid integer string")),
        _ => Err(de::Error::custom("expected integer or integer string")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connect::RegistrationReceipt;
    use crate::readiness::DatasetKind;
    use crate::Client;

    #[test]
    fn base64_codec_round_trips_and_enforces_bounds() {
        for raw in [
            b"".as_slice(),
            b"f",
            b"fo",
            b"foo",
            b"hello world!",
            &[0_u8, 1, 2, 254, 255],
        ] {
            let encoded = base64_encode(raw);
            let decoded = base64_decode(&encoded, raw.len()).unwrap();
            assert_eq!(decoded, raw);
        }
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");

        let large = base64_encode(&vec![0xab; MAX_IPC_CHUNK_BYTES + 1]);
        assert_eq!(
            base64_decode(&large, MAX_IPC_CHUNK_BYTES),
            Err(IpcError::FrameTooLarge)
        );
        for malformed in ["abc", "Zg=", "Zg=a", "Zh==", "!!!!"] {
            assert_eq!(
                base64_decode(malformed, 1024),
                Err(IpcError::InvalidPayload),
                "{malformed} must fail"
            );
        }
    }

    #[tokio::test]
    async fn frame_codec_round_trips_and_rejects_zero_oversize_and_truncated_frames() {
        let (mut client_side, mut server_side) = tokio::io::duplex(4096);
        let envelope = IpcEnvelope::Hello(IpcHello::new("desktop".into(), "0.2.1".into()));
        write_frame(&mut client_side, &envelope).await.unwrap();
        let decoded = read_frame(&mut server_side).await.unwrap().unwrap();
        assert_eq!(decoded, envelope);

        drop(client_side);
        assert_eq!(read_frame(&mut server_side).await.unwrap(), None);

        // Zero-length frame rejected.
        let (mut w, mut r) = tokio::io::duplex(64);
        w.write_all(&0_u32.to_be_bytes()).await.unwrap();
        drop(w);
        assert_eq!(read_frame(&mut r).await, Err(IpcError::ZeroLengthFrame));

        // Oversize frame rejected before allocation (we only wrote the 4-byte header!).
        let (mut w, mut r) = tokio::io::duplex(64);
        let oversize = (MAX_IPC_FRAME_BYTES as u32) + 1;
        w.write_all(&oversize.to_be_bytes()).await.unwrap();
        drop(w);
        assert_eq!(read_frame(&mut r).await, Err(IpcError::FrameTooLarge));

        // Truncated 4-byte length prefix.
        let (mut w, mut r) = tokio::io::duplex(64);
        w.write_all(&[0, 0, 1]).await.unwrap();
        drop(w);
        assert_eq!(read_frame(&mut r).await, Err(IpcError::Truncated));

        // Truncated payload.
        let (mut w, mut r) = tokio::io::duplex(64);
        w.write_all(&10_u32.to_be_bytes()).await.unwrap();
        w.write_all(b"{\"a\":").await.unwrap();
        drop(w);
        assert_eq!(read_frame(&mut r).await, Err(IpcError::Truncated));
    }

    #[tokio::test]
    async fn handshake_accepts_compatible_and_rejects_version_mismatch() {
        let (client_stream, mut server_stream) = tokio::io::duplex(64 * 1024);
        let server = tokio::spawn(async move {
            let hello = read_frame(&mut server_stream).await.unwrap().unwrap();
            assert!(matches!(hello, IpcEnvelope::Hello(_)));
            let ack = IpcEnvelope::HelloAck(IpcHelloAck::compatible(
                Some(Capabilities::default()),
                Some(Ready {
                    service: "argos-api".into(),
                    api_version: crate::API_VERSION,
                    ready: true,
                    storage: true,
                    instance_id: "local-1".into(),
                }),
            ));
            write_frame(&mut server_stream, &ack).await.unwrap();
            server_stream
        });
        let client = LocalIpcClient::connect_stream(client_stream, "desktop", "0.2.1")
            .await
            .unwrap();
        assert_eq!(client.ready_snapshot().unwrap().instance_id, "local-1");
        let _server_stream = server.await.unwrap();

        let (client_stream, mut server_stream) = tokio::io::duplex(64 * 1024);
        let server = tokio::spawn(async move {
            let _ = read_frame(&mut server_stream).await.unwrap().unwrap();
            let ack = IpcEnvelope::HelloAck(IpcHelloAck::incompatible(99, Some("too new".into())));
            write_frame(&mut server_stream, &ack).await.unwrap();
        });
        let err = LocalIpcClient::connect_stream(client_stream, "desktop", "0.2.1")
            .await
            .unwrap_err();
        assert_eq!(err, IpcError::UnsupportedVersion(99));
        assert_eq!(ClientError::from(err), ClientError::UnsupportedVersion(99));
        server.await.unwrap();
    }

    async fn connected_pair() -> (LocalIpcClient, tokio::io::DuplexStream) {
        let (client_stream, mut server_stream) = tokio::io::duplex(512 * 1024);
        let handshake = tokio::spawn(async move {
            let _ = read_frame(&mut server_stream).await.unwrap().unwrap();
            write_frame(
                &mut server_stream,
                &IpcEnvelope::HelloAck(IpcHelloAck::compatible(None, None)),
            )
            .await
            .unwrap();
            server_stream
        });
        let client = LocalIpcClient::connect_stream(client_stream, "desktop", "0.2.1")
            .await
            .unwrap();
        let server_stream = handshake.await.unwrap();
        (client, server_stream)
    }

    #[tokio::test]
    async fn multiplexes_concurrent_requests_delivers_events_and_shuts_down() {
        let (client, mut server) = connected_pair().await;
        let mut events = client.subscribe_events();

        let server_task = tokio::spawn(async move {
            let mut requests = Vec::new();
            for _ in 0..3 {
                match read_frame(&mut server).await.unwrap().unwrap() {
                    IpcEnvelope::Request {
                        request_id,
                        registration,
                        op,
                    } => requests.push((request_id, registration, op)),
                    other => panic!("unexpected frame: {other:?}"),
                }
            }
            // Push an unsolicited event before out-of-order responses.
            write_frame(
                &mut server,
                &IpcEnvelope::Event(IpcEvent::JobProgress {
                    job_id: "job-1".into(),
                    phase: "collect".into(),
                    summary: "running".into(),
                }),
            )
            .await
            .unwrap();

            // Respond in reverse order to verify request_id correlation.
            for (request_id, _reg, op) in requests.into_iter().rev() {
                let payload = match op {
                    IpcOperation::Ready => IpcPayload::Json(serde_json::json!({
                        "service": "argos",
                        "api_version": 1,
                        "ready": true,
                        "storage": true,
                        "instance_id": "inst-1"
                    })),
                    IpcOperation::HostMetrics => {
                        IpcPayload::Json(serde_json::json!({"cpu_pct": 12}))
                    }
                    IpcOperation::ToolsList => IpcPayload::Json(serde_json::json!({"tools": []})),
                    other => panic!("unexpected op: {other:?}"),
                };
                write_frame(
                    &mut server,
                    &IpcEnvelope::Response {
                        request_id,
                        outcome: IpcOutcome::Ok(payload),
                    },
                )
                .await
                .unwrap();
            }

            match read_frame(&mut server).await.unwrap().unwrap() {
                IpcEnvelope::Shutdown { reason } => {
                    assert_eq!(reason, "user_quit");
                    write_frame(
                        &mut server,
                        &IpcEnvelope::ShutdownAck {
                            checkpointed: true,
                            relinquished_leases: 2,
                        },
                    )
                    .await
                    .unwrap();
                }
                other => panic!("expected Shutdown, got {other:?}"),
            }
        });

        let (r1, r2, r3) = tokio::join!(
            client.execute(None, IpcOperation::Ready),
            client.execute(None, IpcOperation::HostMetrics),
            client.execute(None, IpcOperation::ToolsList),
        );
        assert!(matches!(r1, Ok(IpcOutcome::Ok(IpcPayload::Json(_)))));
        assert!(matches!(r2, Ok(IpcOutcome::Ok(IpcPayload::Json(_)))));
        assert!(matches!(r3, Ok(IpcOutcome::Ok(IpcPayload::Json(_)))));

        let ev = events.recv().await.unwrap();
        assert_eq!(
            ev,
            IpcEvent::JobProgress {
                job_id: "job-1".into(),
                phase: "collect".into(),
                summary: "running".into(),
            }
        );

        let ack = client.shutdown("user_quit").await.unwrap();
        assert_eq!(
            ack,
            ShutdownAck {
                checkpointed: true,
                relinquished_leases: 2,
            }
        );
        assert!(client.is_closed());
        // Dead pipe fails closed without fallback.
        assert_eq!(
            client.execute(None, IpcOperation::Ready).await,
            Err(IpcError::PipeClosed)
        );
        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn enforces_inflight_saturation_backpressure_and_dead_pipe_error() {
        let (client, server) = connected_pair().await;
        // Hold server without reading/responding so MAX_IPC_INFLIGHT permits fill up.
        let mut handles = Vec::new();
        for _ in 0..MAX_IPC_INFLIGHT {
            let c = client.clone();
            handles.push(tokio::spawn(async move {
                c.execute(None, IpcOperation::Ready).await
            }));
        }
        // Wait until all 64 permits are acquired.
        while client.inner.inflight.available_permits() > 0 {
            tokio::task::yield_now().await;
        }
        let saturated = client.execute(None, IpcOperation::Ready).await;
        assert_eq!(saturated, Err(IpcError::InflightFull));
        assert_eq!(
            ClientError::from(IpcError::InflightFull),
            ClientError::Status(503)
        );

        // Dropping server closes the pipe and wakes all in-flight waiters with PipeClosed.
        drop(server);
        for handle in handles {
            let result = handle.await.unwrap();
            assert_eq!(result, Err(IpcError::PipeClosed));
        }
    }

    #[tokio::test]
    async fn client_from_local_ipc_routes_context_and_chunked_upload_and_rejects_unknown_routes() {
        use sha2::Digest;

        let (ipc, mut server) = connected_pair().await;
        let ipc = Arc::new(ipc);
        let receipt =
            RegistrationReceipt::new("dep-1".into(), "node-1".into(), "reg-1".into(), None, 3);
        let context =
            RegisteredContext::new(receipt, 4, 5, "cfg-1".into(), "acc-1".into()).unwrap();

        let client = Client::from_local_ipc(ipc.clone())
            .with_registered_context(&context)
            .unwrap();
        assert!(client.is_local_ipc());
        assert_eq!(
            client.get::<serde_json::Value>("/api/v1/unknown").await,
            Err(ClientError::InvalidRoute)
        );

        // Create a temporary CSV larger than MAX_IPC_CHUNK_BYTES (256 KiB) to verify chunking.
        let mut payload = crate::dataset_upload::ACTOR_CSV_COLUMNS.join(",") + "\n";
        while payload.len() <= MAX_IPC_CHUNK_BYTES + 1024 {
            payload.push_str("1,Actor Name,person,[],actor name\n");
        }
        let payload = payload.into_bytes();
        let sha256 = format!("{:x}", sha2::Sha256::digest(&payload));
        let expected_size = payload.len() as u64;
        let path = std::env::temp_dir().join(format!(
            "fleetworks-ipc-upload-test-{}.csv",
            std::process::id()
        ));
        tokio::fs::write(&path, &payload).await.unwrap();
        let file = tokio::fs::File::open(&path).await.unwrap();

        let expected_hash = sha256.clone();
        let expected_ctx = context.clone();
        let server_task = tokio::spawn(async move {
            let mut assembled = Vec::new();
            let mut chunk_count = 0usize;
            loop {
                let frame = read_frame(&mut server).await.unwrap().unwrap();
                let IpcEnvelope::Request {
                    request_id,
                    registration,
                    op,
                } = frame
                else {
                    panic!("expected request");
                };
                assert_eq!(registration.as_ref(), Some(&expected_ctx));
                match op {
                    IpcOperation::DatasetUploadStart(req) => {
                        assert_eq!(req.size_bytes, expected_size);
                        write_frame(
                            &mut server,
                            &IpcEnvelope::Response {
                                request_id,
                                outcome: IpcOutcome::Ok(IpcPayload::Json(serde_json::json!({
                                    "upload_path": "/api/v1/workstation/datasets/uploads/ipc-up-1",
                                    "expires_in_seconds": 300
                                }))),
                            },
                        )
                        .await
                        .unwrap();
                    }
                    IpcOperation::DatasetUploadChunk {
                        upload_id,
                        offset,
                        data_b64,
                    } => {
                        assert_eq!(upload_id, "ipc-up-1");
                        assert_eq!(offset as usize, assembled.len());
                        let bytes = base64_decode(&data_b64, MAX_IPC_CHUNK_BYTES).unwrap();
                        assert!(!bytes.is_empty() && bytes.len() <= MAX_IPC_CHUNK_BYTES);
                        assembled.extend_from_slice(&bytes);
                        chunk_count += 1;
                        write_frame(
                            &mut server,
                            &IpcEnvelope::Response {
                                request_id,
                                outcome: IpcOutcome::Ok(IpcPayload::Empty),
                            },
                        )
                        .await
                        .unwrap();
                    }
                    IpcOperation::DatasetUploadFinish { upload_id } => {
                        assert_eq!(upload_id, "ipc-up-1");
                        assert!(chunk_count >= 2);
                        assert_eq!(assembled.len() as u64, expected_size);
                        write_frame(
                            &mut server,
                            &IpcEnvelope::Response {
                                request_id,
                                outcome: IpcOutcome::Ok(IpcPayload::Json(serde_json::json!({
                                    "dataset": {
                                        "id": "dataset-ipc-1",
                                        "kind": "actor_catalog",
                                        "sha256": expected_hash,
                                        "size_bytes": expected_size
                                    },
                                    "expires_in_seconds": 86400
                                }))),
                            },
                        )
                        .await
                        .unwrap();
                        break;
                    }
                    other => panic!("unexpected op: {other:?}"),
                }
            }
        });

        let uploaded = client
            .upload_dataset(DatasetKind::ActorCatalog, &sha256, file, expected_size)
            .await
            .unwrap();
        assert_eq!(uploaded.dataset.id, "dataset-ipc-1");
        server_task.await.unwrap();
        tokio::fs::remove_file(&path).await.unwrap();
    }

    #[tokio::test]
    async fn stderr_capture_never_buffers_beyond_limit() {
        let (mut w, r) = tokio::io::duplex(4096);
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let task = tokio::spawn(drain_bounded_stderr(r, Arc::clone(&buffer)));
        let total = MAX_IPC_STDERR_BYTES + 16 * 1024;
        let payload = vec![b'e'; total];
        w.write_all(&payload).await.unwrap();
        drop(w);
        task.await.unwrap();
        assert_eq!(buffer.lock().unwrap().len(), MAX_IPC_STDERR_BYTES);
    }

    #[test]
    fn route_mappings_cover_supported_routes_and_reject_unknown_routes() {
        for route in [
            READY_PATH,
            CAPABILITIES_PATH,
            WORKSTATION_READINESS_PATH,
            HOST_METRICS_PATH,
            WORKSTATION_DATASETS_PATH,
            TOOLS_PATH,
            MODELS_DEFAULTS_PATH,
            PROFILE_PATH,
        ] {
            let op = IpcOperation::from_get(route).unwrap();
            assert_eq!(op.into_typed_route(), (IpcMethod::Get, route));
        }

        let op =
            IpcOperation::from_get_with_query(ACTOR_DETAIL_PATH, &[("id", "actor-1")]).unwrap();
        assert_eq!(op.into_typed_route(), (IpcMethod::Get, ACTOR_DETAIL_PATH));

        let op = IpcOperation::from_get_with_query(
            LOGS_PATH,
            &serde_json::json!({"id": "job-1", "limit": 50}),
        )
        .unwrap();
        assert!(matches!(op, IpcOperation::JobLogs(_)));

        let op = IpcOperation::from_get_with_query(LOGS_PATH, &LogQuery::default()).unwrap();
        assert!(matches!(op, IpcOperation::LogsPage(_)));

        assert_eq!(
            IpcOperation::from_get("/api/v1/not-a-route"),
            Err(ClientError::InvalidRoute)
        );
        assert_eq!(
            IpcOperation::from_post("/api/v1/not-a-route", &serde_json::json!({})),
            Err(ClientError::InvalidRoute)
        );
    }

    #[tokio::test]
    async fn client_from_local_ipc_covers_transfers_cycle_history_maintenance_and_shutdown() {
        let (client_stream, mut server_stream) = tokio::io::duplex(512 * 1024);
        let handshake = tokio::spawn(async move {
            let _ = read_frame(&mut server_stream).await.unwrap().unwrap();
            let caps = Capabilities {
                auth: true,
                ..Capabilities::default()
            };
            write_frame(
                &mut server_stream,
                &IpcEnvelope::HelloAck(IpcHelloAck::compatible(Some(caps), None)),
            )
            .await
            .unwrap();
            server_stream
        });
        let base = Client::from_local_ipc(Arc::new(
            LocalIpcClient::connect_stream(client_stream, "desktop", "0.2.1")
                .await
                .unwrap(),
        ));
        let mut server = handshake.await.unwrap();
        assert!(base.capabilities().is_some_and(|c| c.auth));

        let receipt =
            RegistrationReceipt::new("dep-1".into(), "node-1".into(), "reg-1".into(), None, 7);
        let context =
            RegisteredContext::new(receipt, 11, 12, "cfg-1".into(), "acc-1".into()).unwrap();
        let client = base.with_registered_context(&context).unwrap();
        let mutation = crate::context::MutationContext::new(&context);
        let origin = crate::context::JobOrigin::new(
            &context,
            crate::context::OriginSource::Native,
            "req-reindex-1".into(),
            "2026-11-14T12:00:00Z".into(),
            4,
            vec![],
        );

        let exported_cfg = serde_json::to_vec(&serde_json::json!({
            "schema_version": 4,
            "name": "local"
        }))
        .unwrap();
        let exported_cfg_clone = exported_cfg.clone();
        let log_line = serde_json::to_string(&crate::operational_logs::LogEntry {
            id: "log-1".into(),
            ts: "2026-11-14T12:00:00Z".into(),
            severity: "info".into(),
            app: "atlas".into(),
            event_type: "cycle".into(),
            message: "ok".into(),
            job_id: "job-1".into(),
            task_id: "task-1".into(),
            attempt_id: "att-1".into(),
            run_id: "run-1".into(),
        })
        .unwrap();
        let log_bytes = format!("{log_line}\n").into_bytes();
        let log_bytes_clone = log_bytes.clone();

        let server_task = tokio::spawn(async move {
            loop {
                match read_frame(&mut server).await.unwrap() {
                    Some(IpcEnvelope::Request {
                        request_id,
                        registration: _,
                        op,
                    }) => {
                        let outcome = match op {
                            IpcOperation::Ready => {
                                IpcOutcome::Ok(IpcPayload::Json(serde_json::json!({
                                    "service": "argos",
                                    "api_version": crate::API_VERSION,
                                    "ready": true,
                                    "storage": true,
                                    "instance_id": "local-2"
                                })))
                            }
                            IpcOperation::CycleHistory(q) => {
                                assert_eq!(q.limit, Some(25));
                                assert_eq!(q.cursor.as_deref(), Some("cur-1"));
                                IpcOutcome::Ok(IpcPayload::Json(
                                    serde_json::to_value(crate::cycle_history::CycleHistory::new(
                                        vec![],
                                    ))
                                    .unwrap(),
                                ))
                            }
                            IpcOperation::CycleDelete(req) => {
                                assert_eq!(req.job_id, "job-1");
                                IpcOutcome::Ok(IpcPayload::Json(
                                    serde_json::to_value(
                                        crate::cycle_history::DeleteCycleReceipt::hidden(
                                            "job-1".into(),
                                        ),
                                    )
                                    .unwrap(),
                                ))
                            }
                            IpcOperation::ReindexActors(req) => {
                                assert_eq!(req.request_id, "req-reindex-1");
                                let receipt = crate::readiness::MaintenanceReceipt::new(
                                    "job-reindex-1".into(),
                                    crate::readiness::MaintenanceOperation::ReindexActors,
                                    origin.clone(),
                                    None,
                                    false,
                                );
                                IpcOutcome::Ok(IpcPayload::Json(
                                    serde_json::to_value(receipt).unwrap(),
                                ))
                            }
                            IpcOperation::WorkstationRequestDetail(q) => {
                                assert_eq!(q.request_id, "req-reindex-1");
                                let assoc = crate::context::JobRequestAssociation::new(
                                    "job-reindex-1".into(),
                                    origin.clone(),
                                    "2026-11-14T12:00:00Z".into(),
                                );
                                IpcOutcome::Ok(IpcPayload::Json(
                                    serde_json::to_value(assoc).unwrap(),
                                ))
                            }
                            IpcOperation::ConfigurationExport { expected_revision } => {
                                assert_eq!(expected_revision, 4);
                                IpcOutcome::Ok(IpcPayload::ConfigurationExport {
                                    bytes_b64: base64_encode(&exported_cfg_clone),
                                })
                            }
                            IpcOperation::ConfigurationImport {
                                expected_revision,
                                bytes_b64,
                            } => {
                                assert_eq!(expected_revision, 4);
                                assert_eq!(
                                    base64_decode(&bytes_b64, 1024).unwrap(),
                                    exported_cfg_clone
                                );
                                IpcOutcome::Ok(IpcPayload::Json(serde_json::json!({
                                    "saved": true,
                                    "configuration_revision": 5
                                })))
                            }
                            IpcOperation::LogsExport(q) => {
                                assert_eq!(q.limit, Some(10));
                                IpcOutcome::Ok(IpcPayload::LogExport {
                                    bytes_b64: base64_encode(&log_bytes_clone),
                                    next_cursor: None,
                                    apps_complete: true,
                                })
                            }
                            other => panic!("unexpected op: {other:?}"),
                        };
                        write_frame(
                            &mut server,
                            &IpcEnvelope::Response {
                                request_id,
                                outcome,
                            },
                        )
                        .await
                        .unwrap();
                    }
                    Some(IpcEnvelope::Shutdown { reason }) => {
                        assert_eq!(reason, "test_done");
                        write_frame(
                            &mut server,
                            &IpcEnvelope::ShutdownAck {
                                checkpointed: true,
                                relinquished_leases: 0,
                            },
                        )
                        .await
                        .unwrap();
                        break;
                    }
                    other => panic!("unexpected frame: {other:?}"),
                }
            }
        });

        let ready = client.ready().await.unwrap();
        assert_eq!(ready.instance_id, "local-2");
        let history = client.cycle_history_page(25, Some("cur-1")).await.unwrap();
        assert!(history.rows.is_empty());
        let del = client
            .delete_cycle(&DeleteCycleRequest::new(
                "req-del-1".into(),
                mutation.clone(),
                "job-1".into(),
                11,
            ))
            .await
            .unwrap();
        assert!(del.hidden);
        let maint = client
            .reindex_actors(
                &MaintenanceRequest::for_reindex("req-reindex-1".into(), mutation).unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(maint.job_id, "job-reindex-1");
        let assoc = client
            .request_detail(&WorkstationRequestDetailQuery::new(
                "req-reindex-1".into(),
                "reindex_actors".into(),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(assoc.job_id, "job-reindex-1");
        let exported = client.export_configuration(4).await.unwrap();
        assert_eq!(exported, exported_cfg);
        let import_receipt = client.import_configuration(4, exported).await.unwrap();
        assert_eq!(import_receipt.configuration_revision, 5);
        let log_query = LogQuery {
            limit: Some(10),
            ..LogQuery::default()
        };
        let log_page = client.export_logs(&log_query).await.unwrap();
        assert_eq!(log_page.bytes, log_bytes);
        assert!(log_page.apps_complete);

        let ack = client.shutdown_local("test_done").await.unwrap();
        assert!(ack.checkpointed);
        server_task.await.unwrap();
    }
}
