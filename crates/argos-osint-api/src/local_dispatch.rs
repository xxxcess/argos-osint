//! In-process local IPC gateway dispatching typed operations to shared handlers.
//!
//! Avoids HTTP requests, loopback listeners, and bearer headers while preserving
//! the exact permission, registered-context, capability, workspace, and
//! configuration-reset security boundaries enforced by the HTTP gateway.

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use argos_osint_core::osint::manual::Tools;
use argos_osint_core::storage;
use argos_osint_core::storage::configuration_reset;
use argos_osint_core::storage::job_actions;
use argos_osint_core::storage::upload_ticket;
use argos_osint_core::storage::upload_ticket::UploadTicketBinding;
use axum::body::Bytes;
use axum::http::StatusCode;
use chrono::Utc;
use fleetworks_contracts::capabilities::Capabilities;
use fleetworks_contracts::capabilities::CONNECT_REGISTRY_FEATURE;
use fleetworks_contracts::chat::ChatSourceKind;
use fleetworks_contracts::chat::QA_GPT2_GENERATORS_FEATURE;
use fleetworks_contracts::chat::QA_HISTORY_FEATURE;
use fleetworks_contracts::chat::QA_MODEL_SETTINGS_FEATURE;
use fleetworks_contracts::chat::QA_TRIAGE_FEATURE;
use fleetworks_contracts::context::CredentialPurpose;
use fleetworks_contracts::context::Permission;
use fleetworks_contracts::context::RegisteredContext;
use fleetworks_contracts::dataset_upload::DATASET_UPLOADS_PATH;
use fleetworks_contracts::ipc::IpcOperation;
use fleetworks_contracts::ipc::IpcOutcome;
use fleetworks_contracts::ipc::IpcPayload;
use fleetworks_contracts::ipc::MAX_IPC_CHUNK_BYTES;
use fleetworks_contracts::readiness::DatasetKind;
use fleetworks_contracts::readiness::MaintenanceOperation;
use fleetworks_contracts::search::ActorSearchMode;
use fleetworks_contracts::Ready;
use fleetworks_contracts::API_VERSION;
use serde::Serialize;
use sha2::Digest;
use sha2::Sha256;
use tokio::sync::Mutex;

use crate::activity;
use crate::activity_actions;
use crate::activity_paging;
use crate::actor_dossier;
use crate::base64;
use crate::chat;
use crate::classifier_activation;
use crate::configuration;
use crate::connect;
use crate::cycle_history;
use crate::datasets;
use crate::host_metrics;
use crate::inbox_qa::context::GeneratorState;
use crate::inbox_qa::context::Observer;
use crate::maintenance;
use crate::operational_logs;
use crate::operations;
use crate::request_context;
use crate::request_context::AdmissionProof;
use crate::request_context::ContextFailure;
use crate::request_context::ContextRequirement;
use crate::request_context::ResolvedRequest;
use crate::request_reconciliation;
use crate::tools;
use crate::workstation;
use crate::ApiError;
use crate::ApiState;
use crate::AuthenticatedPrincipal;
use crate::PrivateServices;

const MAX_LOCAL_UPLOAD_SESSIONS: usize = 16;
const MAX_CONFIGURATION_IMPORT_BYTES: usize = 2 * 1024 * 1024;
const RETRY_AFTER_CAPACITY_SECONDS: u64 = 60;

/// Host capability flags exposed and enforced by [`LocalGateway`].
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LocalCapabilityConfig {
    pub nlp_enabled: bool,
    pub classifier_enabled: bool,
    pub search_enabled: bool,
    pub jobs_enabled: bool,
}

#[derive(Debug)]
struct DispatchError {
    status: StatusCode,
    message: String,
}

impl DispatchError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    fn bad_request(message: &'static str) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }

    fn unavailable(message: &'static str) -> Self {
        Self::new(StatusCode::SERVICE_UNAVAILABLE, message)
    }
}

impl From<ApiError> for DispatchError {
    fn from(error: ApiError) -> Self {
        Self::new(error.0, error.1)
    }
}

impl From<ContextFailure> for DispatchError {
    fn from(error: ContextFailure) -> Self {
        Self::from(ApiError::from(error))
    }
}

impl From<connect::ConnectError> for DispatchError {
    fn from(error: connect::ConnectError) -> Self {
        Self::new(error.0, error.1)
    }
}

impl From<(StatusCode, &'static str)> for DispatchError {
    fn from((status, message): (StatusCode, &'static str)) -> Self {
        Self::new(status, message)
    }
}

struct UploadSession {
    kind: DatasetKind,
    expected_sha256: String,
    expected_size_bytes: u64,
    written_bytes: u64,
    digest: Sha256,
    started_at: Instant,
    part: Option<datasets::LocalDatasetPart>,
}

#[derive(Clone)]
struct UploadSessionEntry {
    binding: UploadTicketBinding,
    cancelled: Arc<AtomicBool>,
    expires_at: Instant,
    session: Arc<Mutex<UploadSession>>,
}

/// Typed local dispatcher bridging [`IpcOperation`] values to backend core handlers.
#[derive(Clone)]
pub struct LocalGateway {
    state: ApiState,
    services: PrivateServices,
    qa: crate::inbox_qa::QaTarget,
    qa_availability: crate::inbox_qa::QaRuntimeAvailability,
    context: Option<Arc<Observer>>,
    capability_config: LocalCapabilityConfig,
    job_cursors: Arc<activity_paging::JobsCursors>,
    log_cursors: Arc<operational_logs::LogCursors>,
    host_sampler: host_metrics::SharedSampler,
    tools: Arc<Tools>,
    upload_sessions: Arc<Mutex<HashMap<String, UploadSessionEntry>>>,
}

impl LocalGateway {
    /// Cancel every partial upload when the companion's sole connection ends.
    pub async fn cancel_dataset_uploads(&self) {
        let uploads = {
            let mut sessions = self.upload_sessions.lock().await;
            for entry in sessions.values() {
                entry.cancelled.store(true, Ordering::Release);
            }
            std::mem::take(&mut *sessions)
        };
        drop(uploads);
    }

    /// Construct a local IPC gateway over initialized storage and service clients.
    pub fn new(
        state: ApiState,
        services: PrivateServices,
        capability_config: LocalCapabilityConfig,
    ) -> anyhow::Result<Self> {
        let host_sampler = host_metrics::new_sampler();
        let qa =
            crate::inbox_qa::QaTarget::new(services.readiness.clone(), services.search.clone())
                .with_metrics(host_sampler.clone());
        Ok(Self {
            state,
            qa,
            qa_availability: crate::inbox_qa::QaRuntimeAvailability::default(),
            context: None,
            services,
            capability_config,
            job_cursors: Arc::new(activity_paging::JobsCursors::new()),
            log_cursors: Arc::new(operational_logs::LogCursors::new()),
            host_sampler,
            tools: Arc::new(Tools::default()),
            upload_sessions: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// Attach the Embedded-only runtime for qualified decision models.
    #[must_use]
    pub fn with_qa_decision(
        mut self,
        runtime: Arc<dyn crate::inbox_qa::decision::DecisionRuntime>,
    ) -> Self {
        self.qa = self.qa.with_decision(runtime);
        self
    }

    /// Attach the optional Embedded observation journal shared with assistant admission.
    #[must_use]
    pub fn with_workstation_context(mut self, observer: Arc<Observer>) -> Self {
        self.qa = self.qa.with_context(Arc::clone(&observer));
        self.qa_availability = self.qa_availability.with_context(Arc::clone(&observer));
        self.context = Some(observer);
        self
    }

    /// Share the diagnosis reported by this companion's supervised model worker.
    #[must_use]
    pub fn qa_runtime_availability(&self) -> crate::inbox_qa::QaRuntimeAvailability {
        self.qa_availability.clone()
    }

    /// Refresh QA health observations for this companion while inference is enabled.
    pub async fn run_qa_health_refresh(&self, stop: &crate::inbox_qa::CancellationToken) {
        self.qa.refresh(&self.state, stop).await;
    }

    /// Return runtime-backed local capability metadata governed by `LocalCapabilityConfig`.
    pub async fn capabilities(
        &self,
        permissions: Vec<Permission>,
    ) -> Result<Capabilities, (StatusCode, &'static str)> {
        self.capabilities_inner(permissions)
            .await
            .map_err(|ApiError(status, message)| (status, message))
    }

    async fn capabilities_inner(
        &self,
        permissions: Vec<Permission>,
    ) -> Result<Capabilities, ApiError> {
        let deployment = storage::connect_registry::deployment_state(&self.state.repo)
            .await
            .map_err(crate::internal)?;
        let mut extra_features = Vec::new();
        if self.capability_config.jobs_enabled {
            for (pool, feature) in [
                (storage::job_dispatch::ATLAS_POOL, "atlas-cycles"),
                (
                    argos_osint_core::osint::manual::TOOLS_POOL,
                    "tool-execution",
                ),
            ] {
                let mut result = self
                    .state
                    .repo
                    .db()
                    .query("SELECT VALUE enabled FROM ONLY type::record('job_dispatch_pool',$pool)")
                    .bind(("pool", pool.to_owned()))
                    .await
                    .map_err(crate::internal)?
                    .check()
                    .map_err(crate::internal)?;
                if result.take::<Option<bool>>(0).map_err(crate::internal)? == Some(true) {
                    extra_features.push(feature.to_owned());
                }
            }
        }
        self.build_capabilities(
            Some(deployment.deployment_id),
            Some(deployment.configuration_epoch),
            permissions,
            extra_features,
        )
    }

    /// Probe datastore readiness for embedded or local health status.
    pub async fn ready(&self) -> Ready {
        let ready = crate::storage_ready(&self.state.repo).await.is_ok();
        build_ready_contract(ready, &self.state.instance_id)
    }

    /// Authorize and dispatch a single typed [`IpcOperation`] without HTTP framing.
    pub async fn dispatch(
        &self,
        principal: &AuthenticatedPrincipal,
        registration: Option<&RegisteredContext>,
        op: IpcOperation,
    ) -> IpcOutcome {
        match Box::pin(self.dispatch_inner(principal, registration, op)).await {
            Ok(payload) => IpcOutcome::Ok(payload),
            Err(error) => IpcOutcome::Err {
                status: error.status.as_u16(),
                code: error_code(error.status).to_owned(),
                message: error.message,
                retry_after_seconds: (error.status == StatusCode::TOO_MANY_REQUESTS)
                    .then_some(RETRY_AFTER_CAPACITY_SECONDS),
            },
        }
    }

    async fn dispatch_inner(
        &self,
        principal: &AuthenticatedPrincipal,
        registration: Option<&RegisteredContext>,
        op: IpcOperation,
    ) -> Result<IpcPayload, DispatchError> {
        self.enforce_reset_fence(principal).await?;

        match op {
            op @ (IpcOperation::QaSettings
            | IpcOperation::QaSettingsUpdate(_)
            | IpcOperation::QaTurnSubmit(_)
            | IpcOperation::QaRelatedActivity(_)
            | IpcOperation::QaLatest
            | IpcOperation::QaHistory
            | IpcOperation::QaEvents(_)
            | IpcOperation::QaCancel(_)) => {
                Box::pin(self.dispatch_qa_op(principal, registration, op)).await
            }
            op @ (IpcOperation::Ready
            | IpcOperation::Capabilities
            | IpcOperation::WorkstationReadiness
            | IpcOperation::HostMetrics
            | IpcOperation::DatasetsList
            | IpcOperation::ToolsList
            | IpcOperation::ModelsDefaults
            | IpcOperation::ProfileRead
            | IpcOperation::ActorsSearch(_)
            | IpcOperation::ActorsBrowse(_)
            | IpcOperation::ActorDetail(_)
            | IpcOperation::ArticlesList(_)
            | IpcOperation::ArticlesSearch(_)
            | IpcOperation::JobsList(_)
            | IpcOperation::JobDetail(_)
            | IpcOperation::ToolResult(_)
            | IpcOperation::LogsPage(_)
            | IpcOperation::JobLogs(_)
            | IpcOperation::LogsFollow(_)
            | IpcOperation::LogsExport(_)
            | IpcOperation::WorkstationRequestDetail(_)
            | IpcOperation::CycleHistory(_)
            | IpcOperation::ClassifierCandidateDetail(_)
            | IpcOperation::ClassifierActivationResult(_)) => {
                Box::pin(self.dispatch_read_op(principal, registration, op)).await
            }
            op => Box::pin(self.dispatch_write_op(principal, registration, op)).await,
        }
    }

    async fn dispatch_qa_op(
        &self,
        principal: &AuthenticatedPrincipal,
        registration: Option<&RegisteredContext>,
        op: IpcOperation,
    ) -> Result<IpcPayload, DispatchError> {
        use crate::inbox_qa;

        if principal.purpose() != CredentialPurpose::Native {
            return Err(ContextFailure::forbidden().into());
        }
        let permission = match &op {
            IpcOperation::QaSettings => Permission::ReadBackend,
            IpcOperation::QaSettingsUpdate(_) => Permission::ManageInboxQa,
            IpcOperation::QaRelatedActivity(request) => {
                request
                    .validate()
                    .map_err(|_| DispatchError::bad_request("Invalid Assistant source"))?;
                match request.source.kind {
                    ChatSourceKind::Job => Permission::ReadWorkstationActivity,
                    _ => Permission::ReadBackend,
                }
            }
            IpcOperation::QaLatest | IpcOperation::QaHistory | IpcOperation::QaEvents(_) => {
                Permission::ReadWorkstationActivity
            }
            IpcOperation::QaTurnSubmit(_) | IpcOperation::QaCancel(_) => {
                Permission::SubmitInteractive
            }
            _ => return Err(DispatchError::bad_request("Invalid Inbox operation")),
        };
        let context = self
            .resolve(
                principal,
                registration,
                permission,
                ContextRequirement::Registered,
            )
            .await?;
        match op {
            IpcOperation::QaSettings => json_payload(
                inbox_qa::settings_request(
                    &self.state,
                    &context,
                    &self.qa,
                    Some(&self.qa_availability),
                )
                .await?,
            ),
            IpcOperation::QaSettingsUpdate(request) => {
                let settings = inbox_qa::update_request(
                    &self.state,
                    &context,
                    &self.qa,
                    request,
                    Some(&self.qa_availability),
                )
                .await?;
                if !settings.enabled {
                    if let (Some(observer), Ok(generation)) =
                        (&self.context, i64::try_from(settings.generation))
                    {
                        observer.record_generator(generation, GeneratorState::Disabled, None);
                    }
                }
                json_payload(settings)
            }
            IpcOperation::QaTurnSubmit(request) => json_payload(
                inbox_qa::submit_request(
                    &self.state,
                    &context,
                    &self.qa,
                    &self.qa_availability,
                    request,
                )
                .await?,
            ),
            IpcOperation::QaRelatedActivity(request) => json_payload(
                inbox_qa::related_activity_request(&self.state, &context, &self.qa, request)
                    .await?,
            ),
            IpcOperation::QaLatest => {
                json_payload(inbox_qa::latest_request(&self.state, &context).await?)
            }
            IpcOperation::QaHistory => {
                json_payload(inbox_qa::history_request(&self.state, &context).await?)
            }
            IpcOperation::QaEvents(query) => json_payload(
                inbox_qa::events_request(&self.state, &context, &self.qa, query).await?,
            ),
            IpcOperation::QaCancel(request) => {
                json_payload(inbox_qa::cancel_request(&self.state, &context, request).await?)
            }
            _ => Err(DispatchError::bad_request("Invalid Inbox operation")),
        }
    }

    async fn dispatch_read_op(
        &self,
        principal: &AuthenticatedPrincipal,
        registration: Option<&RegisteredContext>,
        op: IpcOperation,
    ) -> Result<IpcPayload, DispatchError> {
        match op {
            IpcOperation::Ready => self.handle_ready().await,
            IpcOperation::Capabilities => self.handle_capabilities(principal).await,
            IpcOperation::WorkstationReadiness => {
                self.handle_workstation_readiness(principal, registration)
                    .await
            }
            IpcOperation::HostMetrics => {
                self.require_global_read(principal, registration).await?;
                let metrics = host_metrics::sample_current(self.host_sampler.clone()).await?;
                json_payload(metrics)
            }
            IpcOperation::DatasetsList => {
                self.require_global_read(principal, registration).await?;
                Ok(IpcPayload::Json(
                    datasets::list_value(&self.state, principal).await?,
                ))
            }
            IpcOperation::ToolsList => {
                self.require_global_read(principal, registration).await?;
                Ok(IpcPayload::Json(operations::tools_value()))
            }
            IpcOperation::ModelsDefaults => {
                self.require_global_read(principal, registration).await?;
                Ok(IpcPayload::Json(
                    configuration::models_value(&self.state).await?,
                ))
            }
            IpcOperation::ProfileRead => {
                self.require_global_read(principal, registration).await?;
                Ok(IpcPayload::Json(
                    configuration::profile_value(&self.state).await?,
                ))
            }
            IpcOperation::ActorsSearch(query) => {
                self.require_search_enabled()?;
                self.require_global_read(principal, registration).await?;
                let internal_query = actor_dossier::ActorSearchQuery {
                    query: query.query,
                    mode: match query.mode {
                        ActorSearchMode::Discovery => actor_dossier::SearchMode::Discovery,
                        _ => {
                            return Err(DispatchError::bad_request("Unsupported actor search mode"))
                        }
                    },
                    limit: Some(usize::from(query.limit)),
                };
                Ok(IpcPayload::Json(
                    actor_dossier::search_value(
                        &self.state,
                        internal_query,
                        self.services.search.as_ref(),
                        Some(principal),
                    )
                    .await?,
                ))
            }
            IpcOperation::ActorsBrowse(query) => {
                self.require_global_read(principal, registration).await?;
                Ok(IpcPayload::Json(
                    actor_dossier::browse_value(&self.state, &query).await?,
                ))
            }
            IpcOperation::ActorDetail(query) => {
                self.require_global_read(principal, registration).await?;
                Ok(IpcPayload::Json(
                    actor_dossier::read_value(&self.state, &query.id).await?,
                ))
            }
            IpcOperation::ArticlesList(query) => {
                self.require_global_read(principal, registration).await?;
                let article_query = argos_osint_core::intel_service::ArticleQuery {
                    category: query.category,
                    day: query.day,
                    search: query.search,
                    sort: query.sort,
                    cursor: query.cursor,
                    limit: Some(usize::from(query.limit)),
                };
                article_query
                    .validate()
                    .map_err(|_| DispatchError::from(crate::bad()))?;
                let page =
                    argos_osint_core::intel_service::articles(&self.state.repo, &article_query)
                        .await
                        .map_err(crate::internal)?;
                json_payload(page)
            }
            IpcOperation::ArticlesSearch(query) => {
                self.require_global_read(principal, registration).await?;
                let search = argos_osint_core::storage::article_discovery::ArticleSearchQuery {
                    query: query.query,
                    category: query.category,
                    day: query.day,
                    source: query.source,
                    run: query.run,
                    limit: query.limit.map(usize::from),
                    cursor: query.cursor,
                };
                search
                    .validate()
                    .map_err(|_| DispatchError::from(crate::bad()))?;
                let page =
                    argos_osint_core::intel_service::article_search(&self.state.repo, &search)
                        .await
                        .map_err(crate::internal)?;
                json_payload(page)
            }
            IpcOperation::JobsList(query) => {
                let context = self
                    .resolve(
                        principal,
                        registration,
                        Permission::ReadWorkstationActivity,
                        ContextRequirement::Registered,
                    )
                    .await?;
                let paging = activity_paging::JobsQuery {
                    id: query.id,
                    job_id: None,
                    workspace_id: None,
                    limit: query.limit.map(usize::from),
                    status: query.status,
                    app: query.app,
                    operation: query.operation,
                    kind: query.kind,
                    correlation_id: query.correlation_id,
                    text: query.text,
                    since: query.since,
                    sort: query.sort,
                    cursor: query.cursor,
                };
                let observed_at = Utc::now().timestamp_millis();
                let value =
                    activity_paging::jobs_value(&self.state, &context, &self.job_cursors, paging)
                        .await?;
                if let Some(observer) = &self.context {
                    observer.record_jobs(&context, &value, observed_at);
                }
                Ok(IpcPayload::Json(value))
            }
            IpcOperation::JobDetail(query) => {
                let context = self
                    .resolve(
                        principal,
                        registration,
                        Permission::ReadWorkstationActivity,
                        ContextRequirement::Registered,
                    )
                    .await?;
                let detail = activity::ActivityQuery {
                    id: query.id,
                    job_id: None,
                    workspace_id: None,
                    limit: query.limit.map(usize::from),
                    status: None,
                    app: None,
                    operation: None,
                    text: None,
                    since: None,
                };
                let observed_at = Utc::now().timestamp_millis();
                let value = activity::job_value(&self.state, &context, detail).await?;
                if let Some(observer) = &self.context {
                    observer.record_jobs(&context, &value, observed_at);
                }
                Ok(IpcPayload::Json(value))
            }
            IpcOperation::ToolResult(query) => {
                let context = self
                    .resolve(
                        principal,
                        registration,
                        Permission::ReadWorkstationActivity,
                        ContextRequirement::Registered,
                    )
                    .await?;
                json_payload(tools::result_request(&self.state, &context, &query.id).await?)
            }
            IpcOperation::LogsPage(query) => {
                let context = self
                    .resolve(
                        principal,
                        registration,
                        Permission::ReadWorkstationActivity,
                        ContextRequirement::Registered,
                    )
                    .await?;
                let internal_query = convert_log_query(query);
                json_payload(
                    operational_logs::page_value(
                        &self.state,
                        &context,
                        &self.log_cursors,
                        &internal_query,
                    )
                    .await?,
                )
            }
            IpcOperation::JobLogs(query) => {
                let context = self
                    .resolve(
                        principal,
                        registration,
                        Permission::ReadWorkstationActivity,
                        ContextRequirement::Registered,
                    )
                    .await?;
                let internal_query = operational_logs::LogQuery {
                    job_id: query.id,
                    workspace_id: None,
                    descendants: None,
                    min_rank: None,
                    app: None,
                    text: None,
                    since: None,
                    limit: Some(usize::from(query.limit)),
                    cursor: None,
                    wait_ms: None,
                };
                json_payload(
                    operational_logs::page_value(
                        &self.state,
                        &context,
                        &self.log_cursors,
                        &internal_query,
                    )
                    .await?,
                )
            }
            IpcOperation::LogsFollow(query) => {
                let context = self
                    .resolve(
                        principal,
                        registration,
                        Permission::ReadWorkstationActivity,
                        ContextRequirement::Registered,
                    )
                    .await?;
                let internal_query = convert_log_query(query);
                json_payload(
                    operational_logs::follow_value(
                        &self.state,
                        &context,
                        &self.log_cursors,
                        &internal_query,
                    )
                    .await?,
                )
            }
            IpcOperation::LogsExport(query) => {
                let context = self
                    .resolve(
                        principal,
                        registration,
                        Permission::ReadWorkstationActivity,
                        ContextRequirement::Registered,
                    )
                    .await?;
                let internal_query = convert_log_query(query);
                let payload = match operational_logs::export_payload(
                    &self.state,
                    &context,
                    &self.log_cursors,
                    &internal_query,
                )
                .await?
                {
                    Ok(payload) => payload,
                    Err(_page) => {
                        return Err(DispatchError::new(
                            StatusCode::CONFLICT,
                            "Scoped operational log cohort unavailable",
                        ))
                    }
                };
                Ok(IpcPayload::LogExport {
                    bytes_b64: base64::encode(&payload.bytes),
                    next_cursor: payload.next_cursor,
                    apps_complete: payload.apps_complete,
                })
            }
            IpcOperation::WorkstationRequestDetail(query) => {
                let context = self
                    .resolve(
                        principal,
                        registration,
                        Permission::ReadWorkstationActivity,
                        ContextRequirement::Registered,
                    )
                    .await?;
                Ok(IpcPayload::Json(
                    request_reconciliation::read_value(&self.state, &context, &query).await?,
                ))
            }
            IpcOperation::CycleHistory(query) => {
                let context = self
                    .resolve(
                        principal,
                        registration,
                        Permission::ReadWorkstationActivity,
                        ContextRequirement::Registered,
                    )
                    .await?;
                let history = cycle_history::list_value(
                    &self.state,
                    &context,
                    cycle_history::HistoryQuery {
                        limit: query.limit,
                        cursor: query.cursor,
                    },
                )
                .await?;
                json_payload(history)
            }
            IpcOperation::ClassifierCandidateDetail(query) => {
                let context = self
                    .resolve(
                        principal,
                        registration,
                        Permission::MaintainBackend,
                        ContextRequirement::Registered,
                    )
                    .await?;
                let owner = self.services.activation.as_ref().ok_or_else(|| {
                    DispatchError::unavailable("Classifier activation unavailable")
                })?;
                let preview = classifier_activation::actual_preview(
                    &self.state,
                    &context,
                    owner,
                    &classifier_activation::PreviewQuery {
                        training_job_id: query.training_job_id,
                        model_id: query.model_id,
                    },
                )
                .await?;
                json_payload(preview)
            }
            IpcOperation::ClassifierActivationResult(query) => {
                let context = self
                    .resolve(
                        principal,
                        registration,
                        Permission::MaintainBackend,
                        ContextRequirement::Registered,
                    )
                    .await?;
                let result =
                    classifier_activation::result_request(&self.state, &context, &query.job_id)
                        .await?;
                json_payload(result)
            }
            _ => Err(DispatchError::bad_request("Unsupported IPC operation")),
        }
    }

    async fn dispatch_write_op(
        &self,
        principal: &AuthenticatedPrincipal,
        registration: Option<&RegisteredContext>,
        op: IpcOperation,
    ) -> Result<IpcPayload, DispatchError> {
        match op {
            IpcOperation::RegistrationEnsure(input) => {
                let (_status, value) =
                    connect::ensure_request(&self.state, Some(principal), None, input).await?;
                Ok(IpcPayload::Json(value))
            }
            IpcOperation::RegistrationContext(input) => {
                let value = connect::registration_context_request(
                    &self.state,
                    Some(principal),
                    &input.registration_id,
                    input.expected_revision,
                )
                .await?;
                Ok(IpcPayload::Json(value))
            }
            IpcOperation::RegistrationRebind(input) => {
                let value = connect::rebind_request(
                    &self.state,
                    Some(principal),
                    connect::RebindRequest {
                        request_id: input.request_id,
                        registration_id: input.registration_id,
                        expected_revision: input.expected_revision,
                        workstation_ip: input.workstation_ip,
                        installation_marker: input.installation_marker,
                    },
                )
                .await?;
                Ok(IpcPayload::Json(value))
            }
            IpcOperation::RegistrationRetire(input) => {
                let (_status, value) = connect::retire_request(
                    &self.state,
                    Some(principal),
                    &input.registration_id,
                    input.expected_revision,
                )
                .await?;
                Ok(IpcPayload::Json(value))
            }
            IpcOperation::WebSessionCreate(input) => {
                let (_status, value) = connect::create_session_request(
                    &self.state,
                    Some(principal),
                    &input.connect_id,
                )
                .await?;
                Ok(IpcPayload::Json(value))
            }
            IpcOperation::AtlasCycleSubmit(request) => {
                self.require_jobs_enabled()?;
                let context = self
                    .resolve(
                        principal,
                        registration,
                        Permission::SubmitInteractive,
                        ContextRequirement::Registered,
                    )
                    .await?;
                let receipt =
                    operations::submit_atlas_cycle_request(&self.state, &context, request).await?;
                json_payload(receipt)
            }
            IpcOperation::CycleDelete(request) => {
                let context = self
                    .resolve(
                        principal,
                        registration,
                        Permission::ReadWorkstationActivity,
                        ContextRequirement::Registered,
                    )
                    .await?;
                let receipt = cycle_history::delete_request(&self.state, &context, request).await?;
                json_payload(receipt)
            }
            IpcOperation::ToolRun(request) => {
                self.require_jobs_enabled()?;
                let context = self
                    .resolve(
                        principal,
                        registration,
                        Permission::SubmitInteractive,
                        ContextRequirement::Registered,
                    )
                    .await?;
                let receipt =
                    tools::submit_request(&self.state, &self.tools, &context, request).await?;
                json_payload(receipt)
            }
            IpcOperation::JobRetry(request) => {
                let context = self
                    .resolve(
                        principal,
                        registration,
                        Permission::RetryJob,
                        ContextRequirement::Registered,
                    )
                    .await?;
                Ok(IpcPayload::Json(
                    activity_actions::perform_request(
                        &self.state,
                        &context,
                        activity_actions::ActionRequest {
                            id: request.id,
                            request_id: request.request_id,
                        },
                        job_actions::Action::Retry,
                    )
                    .await?,
                ))
            }
            IpcOperation::JobCancel(request) => {
                let context = self
                    .resolve(
                        principal,
                        registration,
                        Permission::CancelJob,
                        ContextRequirement::Registered,
                    )
                    .await?;
                Ok(IpcPayload::Json(
                    activity_actions::perform_request(
                        &self.state,
                        &context,
                        activity_actions::ActionRequest {
                            id: request.id,
                            request_id: request.request_id,
                        },
                        job_actions::Action::Cancel,
                    )
                    .await?,
                ))
            }
            IpcOperation::DatasetUploadStart(request) => {
                self.handle_dataset_upload_start(principal, registration, request)
                    .await
            }
            IpcOperation::DatasetUploadChunk {
                upload_id,
                offset,
                data,
            } => {
                self.handle_dataset_upload_chunk(principal, registration, upload_id, offset, data)
                    .await
            }
            IpcOperation::DatasetUploadFinish { upload_id } => {
                self.handle_dataset_upload_finish(principal, registration, upload_id)
                    .await
            }
            IpcOperation::DatasetUploadCancel { upload_id } => {
                self.handle_dataset_upload_cancel(principal, registration, upload_id)
                    .await
            }
            IpcOperation::SeedActors(request) => {
                self.require_jobs_enabled()?;
                let context = self
                    .resolve(
                        principal,
                        registration,
                        Permission::MaintainBackend,
                        ContextRequirement::Registered,
                    )
                    .await?;
                let receipt = maintenance::admit_request(
                    &self.state,
                    &context,
                    request,
                    MaintenanceOperation::SeedActors,
                    maintenance::SetupIngress::Embedded(None),
                )
                .await?;
                json_payload(receipt)
            }
            IpcOperation::TrainClassifier(request) => {
                let context = self
                    .resolve(
                        principal,
                        registration,
                        Permission::MaintainBackend,
                        ContextRequirement::Registered,
                    )
                    .await?;
                let receipt = maintenance::admit_request(
                    &self.state,
                    &context,
                    request,
                    MaintenanceOperation::TrainClassifier,
                    maintenance::SetupIngress::Embedded(None),
                )
                .await?;
                json_payload(receipt)
            }
            IpcOperation::ReindexActors(request) => {
                self.require_jobs_enabled()?;
                self.require_search_enabled()?;
                let context = self
                    .resolve(
                        principal,
                        registration,
                        Permission::MaintainBackend,
                        ContextRequirement::Registered,
                    )
                    .await?;
                // A failed status read never proves the current generation is reusable.
                let index_status = if let Some(search) = self.services.search.clone() {
                    crate::run_shielded_timeout(std::time::Duration::from_secs(2), async move {
                        search.status().await
                    })
                    .await
                    .and_then(Result::ok)
                } else {
                    None
                };
                let receipt = maintenance::admit_request(
                    &self.state,
                    &context,
                    request,
                    MaintenanceOperation::ReindexActors,
                    maintenance::SetupIngress::Embedded(index_status.as_ref()),
                )
                .await?;
                json_payload(receipt)
            }
            IpcOperation::ClassifierActivate(_) => {
                let context = self
                    .resolve(
                        principal,
                        registration,
                        Permission::MaintainBackend,
                        ContextRequirement::Registered,
                    )
                    .await?;
                classifier_activation::ensure_activation_authority(&context)?;
                Err(classifier_activation::retired_activation().into())
            }
            IpcOperation::ModelRoleSet(request) => {
                let context = self
                    .resolve(
                        principal,
                        registration,
                        Permission::ManageConfiguration,
                        ContextRequirement::GlobalRead,
                    )
                    .await?;
                let role_request: configuration::SetRole = serde_json::to_value(&request)
                    .and_then(serde_json::from_value)
                    .map_err(|_| DispatchError::bad_request("Invalid model role update"))?;
                Ok(IpcPayload::Json(
                    configuration::save_role_request(&self.state, Some(&context), role_request)
                        .await?,
                ))
            }
            IpcOperation::ProfileAccountUpsert(request) => {
                let context = self
                    .resolve(
                        principal,
                        registration,
                        Permission::ManageConfiguration,
                        ContextRequirement::GlobalRead,
                    )
                    .await?;
                let account = configuration::AccountInput {
                    expected_revision: request.expected_revision,
                    account_id: request.account_id,
                    provider: request.provider,
                    base_url: request.base_url,
                    model: request.model,
                    api_key: request.api_key,
                };
                Ok(IpcPayload::Json(
                    configuration::upsert_account_request(&self.state, Some(&context), account)
                        .await?,
                ))
            }
            IpcOperation::ProfileAccountDelete(request) => {
                let context = self
                    .resolve(
                        principal,
                        registration,
                        Permission::ManageConfiguration,
                        ContextRequirement::GlobalRead,
                    )
                    .await?;
                let delete = configuration::DeleteAccount {
                    expected_revision: request.expected_revision,
                    account_id: request.account_id,
                };
                Ok(IpcPayload::Json(
                    configuration::delete_account_request(&self.state, Some(&context), delete)
                        .await?,
                ))
            }
            IpcOperation::ChatTurnSubmit(request) => {
                let context = self
                    .resolve(
                        principal,
                        registration,
                        Permission::SubmitInteractive,
                        ContextRequirement::Registered,
                    )
                    .await?;
                json_payload(chat::submit_request(&self.state, &context, request).await?)
            }
            IpcOperation::ProfileToolsSet(request) => {
                let context = self
                    .resolve(
                        principal,
                        registration,
                        Permission::ManageConfiguration,
                        ContextRequirement::GlobalRead,
                    )
                    .await?;
                let credentials = configuration::ToolCredential {
                    expected_revision: request.expected_revision,
                    provider: request.provider,
                    primary: request.primary,
                    fallback: request.fallback,
                };
                Ok(IpcPayload::Json(
                    configuration::save_tool_credential_request(
                        &self.state,
                        Some(&context),
                        credentials,
                    )
                    .await?,
                ))
            }
            IpcOperation::ConfigurationExport { expected_revision } => {
                let _context = self
                    .resolve(
                        principal,
                        registration,
                        Permission::ManageConfiguration,
                        ContextRequirement::GlobalRead,
                    )
                    .await?;
                let bytes =
                    configuration::export_bytes(&self.state, expected_revision, true).await?;
                Ok(IpcPayload::ConfigurationExport {
                    bytes_b64: base64::encode(&bytes),
                })
            }
            IpcOperation::ConfigurationImport {
                expected_revision,
                bytes_b64,
            } => {
                let context = self
                    .resolve(
                        principal,
                        registration,
                        Permission::ManageConfiguration,
                        ContextRequirement::GlobalRead,
                    )
                    .await?;
                let bytes =
                    base64::decode(&bytes_b64, MAX_CONFIGURATION_IMPORT_BYTES).map_err(|_| {
                        DispatchError::bad_request("Invalid configuration import payload")
                    })?;
                Ok(IpcPayload::Json(
                    configuration::import_bytes(
                        &self.state,
                        Some(&context),
                        expected_revision,
                        true,
                        Bytes::from(bytes),
                    )
                    .await?,
                ))
            }
            _ => Err(DispatchError::bad_request("Unsupported IPC operation")),
        }
    }

    async fn enforce_reset_fence(
        &self,
        principal: &AuthenticatedPrincipal,
    ) -> Result<(), DispatchError> {
        if matches!(
            principal.purpose(),
            CredentialPurpose::Native | CredentialPurpose::WebRelay
        ) {
            if let Err(error) = configuration_reset::authorize_delegated_access(
                &self.state.repo,
                principal.scope_id(),
                principal.access_epoch(),
            )
            .await
            {
                let status = if error == configuration_reset::ResetError::Forbidden {
                    StatusCode::UNAUTHORIZED
                } else {
                    StatusCode::SERVICE_UNAVAILABLE
                };
                return Err(DispatchError::new(
                    status,
                    "Current user access is unavailable",
                ));
            }
        }
        Ok(())
    }

    async fn resolve(
        &self,
        principal: &AuthenticatedPrincipal,
        registration: Option<&RegisteredContext>,
        permission: Permission,
        requirement: ContextRequirement,
    ) -> Result<ResolvedRequest, DispatchError> {
        let proof = match (principal.purpose(), registration) {
            (CredentialPurpose::Native | CredentialPurpose::Operator, Some(reg)) => {
                AdmissionProof::Native {
                    registration_id: reg.registration_id.clone(),
                    expected_registration_revision: reg.registration_revision,
                }
            }
            _ => AdmissionProof::None,
        };
        request_context::resolve_local(&self.state.repo, principal, proof, permission, requirement)
            .await
            .map_err(DispatchError::from)
    }

    async fn require_global_read(
        &self,
        principal: &AuthenticatedPrincipal,
        registration: Option<&RegisteredContext>,
    ) -> Result<ResolvedRequest, DispatchError> {
        self.resolve(
            principal,
            registration,
            Permission::ReadBackend,
            ContextRequirement::GlobalRead,
        )
        .await
    }

    fn require_search_enabled(&self) -> Result<(), DispatchError> {
        if !self.capability_config.search_enabled {
            return Err(DispatchError::unavailable(
                "Actor search capability is disabled",
            ));
        }
        Ok(())
    }

    fn require_jobs_enabled(&self) -> Result<(), DispatchError> {
        if !self.capability_config.jobs_enabled {
            return Err(DispatchError::unavailable("Jobs capability is disabled"));
        }
        Ok(())
    }

    async fn handle_ready(&self) -> Result<IpcPayload, DispatchError> {
        crate::storage_ready(&self.state.repo).await?;
        json_payload(build_ready_contract(true, &self.state.instance_id))
    }

    async fn handle_capabilities(
        &self,
        principal: &AuthenticatedPrincipal,
    ) -> Result<IpcPayload, DispatchError> {
        let response = self
            .capabilities_inner(principal.permissions().collect())
            .await?;
        json_payload(response)
    }

    fn build_capabilities(
        &self,
        deployment_id: Option<String>,
        configuration_epoch: Option<String>,
        permissions: Vec<Permission>,
        extra_features: Vec<String>,
    ) -> Result<Capabilities, ApiError> {
        let mut features: Vec<String> = [
            "bulletin",
            "brief",
            "related",
            "topology",
            "live",
            "article-search",
            "logs",
            "tools",
            "model-defaults",
            "profile",
            CONNECT_REGISTRY_FEATURE,
            "workstation-readiness",
            "dataset-upload",
            "profile-management",
            "actor-discovery",
            "workstation-cycle-history-v1",
            crate::inbox_qa::FEATURE,
            QA_TRIAGE_FEATURE,
            QA_MODEL_SETTINGS_FEATURE,
            QA_HISTORY_FEATURE,
            QA_GPT2_GENERATORS_FEATURE,
            fleetworks_contracts::chat::QA_INBOX_NOTIFICATIONS_FEATURE,
            fleetworks_contracts::chat::QA_JOB_NOTIFICATION_LOGS_FEATURE,
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        if self.capability_config.jobs_enabled {
            features.push("jobs".to_owned());
        }
        if self.capability_config.nlp_enabled {
            features.push("nlp".to_owned());
        }
        if self.capability_config.classifier_enabled {
            features.push("classifier".to_owned());
        }
        features.extend(extra_features);
        crate::base_capabilities(
            &self.state.instance_id,
            features,
            self.capability_config.search_enabled,
            deployment_id,
            configuration_epoch,
            permissions,
        )
    }

    async fn handle_workstation_readiness(
        &self,
        principal: &AuthenticatedPrincipal,
        registration: Option<&RegisteredContext>,
    ) -> Result<IpcPayload, DispatchError> {
        let context = self.require_global_read(principal, registration).await?;
        let owners = self.services.readiness.as_ref().ok_or_else(|| {
            DispatchError::unavailable("Backend diagnostic owners are not configured")
        })?;
        let search = if self.capability_config.search_enabled {
            self.services.search.as_ref()
        } else {
            None
        };
        let report = workstation::read_report(
            &self.state,
            principal,
            owners,
            search,
            context.context().cloned(),
            true,
        )
        .await?;
        let payload = json_payload(&report)?;
        if let Some(observer) = &self.context {
            observer.record_readiness(&report.checks);
        }
        Ok(payload)
    }

    async fn handle_dataset_upload_start(
        &self,
        principal: &AuthenticatedPrincipal,
        registration: Option<&RegisteredContext>,
        request: fleetworks_contracts::dataset_upload::DatasetUploadRequest,
    ) -> Result<IpcPayload, DispatchError> {
        let context = self
            .resolve(
                principal,
                registration,
                Permission::MaintainBackend,
                ContextRequirement::Registered,
            )
            .await?;
        let store =
            self.services.datasets.as_ref().ok_or_else(|| {
                DispatchError::unavailable("Dataset staging store is unavailable")
            })?;
        let binding = datasets::upload_binding(&context).ok_or_else(|| {
            DispatchError::new(
                StatusCode::FORBIDDEN,
                "Native or Operator upload credential required",
            )
        })?;
        let ticket =
            datasets::create_upload_ticket(&self.state, &context, Some(store), &request).await?;
        let upload_id = ticket
            .upload_path
            .strip_prefix(DATASET_UPLOADS_PATH)
            .and_then(|suffix| suffix.strip_prefix('/'))
            .filter(|id| valid_upload_id(id))
            .ok_or_else(|| crate::internal("invalid upload ticket path"))?
            .to_owned();
        let part = store
            .begin_local_part(&self.state.repo, context.authority(), request.kind)
            .await
            .map_err(crate::internal)?;
        let started_at = Instant::now();
        let expires_at = started_at + Duration::from_secs(ticket.expires_in_seconds);
        let entry = UploadSessionEntry {
            binding,
            cancelled: Arc::new(AtomicBool::new(false)),
            expires_at,
            session: Arc::new(Mutex::new(UploadSession {
                kind: request.kind,
                expected_sha256: request.sha256.to_ascii_lowercase(),
                expected_size_bytes: request.size_bytes,
                written_bytes: 0,
                digest: Sha256::new(),
                started_at,
                part: Some(part),
            })),
        };
        let mut sessions = self.upload_sessions.lock().await;
        let now = Instant::now();
        sessions.retain(|_, existing| existing.expires_at > now);
        if sessions.len() >= MAX_LOCAL_UPLOAD_SESSIONS {
            return Err(DispatchError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "Dataset upload capacity reached",
            ));
        }
        sessions.insert(upload_id, entry);
        drop(sessions);
        json_payload(ticket)
    }

    async fn handle_dataset_upload_chunk(
        &self,
        principal: &AuthenticatedPrincipal,
        registration: Option<&RegisteredContext>,
        upload_id: String,
        offset: u64,
        data: Vec<u8>,
    ) -> Result<IpcPayload, DispatchError> {
        let context = self
            .resolve(
                principal,
                registration,
                Permission::MaintainBackend,
                ContextRequirement::Registered,
            )
            .await?;
        if !valid_upload_id(&upload_id) {
            return Err(DispatchError::bad_request("Invalid dataset upload ID"));
        }
        let _store =
            self.services.datasets.as_ref().ok_or_else(|| {
                DispatchError::unavailable("Dataset staging store is unavailable")
            })?;
        let binding = datasets::upload_binding(&context).ok_or_else(|| {
            DispatchError::new(
                StatusCode::FORBIDDEN,
                "Native or Operator upload credential required",
            )
        })?;
        if data.is_empty() {
            return Err(DispatchError::bad_request(
                "Dataset upload chunk must not be empty",
            ));
        }
        if data.len() > MAX_IPC_CHUNK_BYTES {
            return Err(DispatchError::bad_request(
                "Dataset upload chunk exceeds the size limit",
            ));
        }
        let entry = self.dataset_upload_session(&upload_id, &binding).await?;
        let mut session = entry.session.lock().await;
        check_upload_live(&entry)?;
        if offset != session.written_bytes {
            return Err(DispatchError::new(
                StatusCode::CONFLICT,
                "Dataset upload chunk offset mismatch",
            ));
        }
        let next_offset = offset
            .checked_add(data.len() as u64)
            .ok_or_else(|| DispatchError::bad_request("Dataset upload offset overflow"))?;
        if next_offset > session.expected_size_bytes {
            return Err(DispatchError::new(
                StatusCode::BAD_REQUEST,
                "Dataset upload exceeds declared size",
            ));
        }
        // A canceled write may leave partial bytes. Keeping ownership outside
        // the session during the write unlinks the file on failure or cancel.
        let mut part = session.part.take().ok_or_else(|| {
            DispatchError::new(StatusCode::NOT_FOUND, "Dataset upload destination closed")
        })?;
        part.write_chunk(&data).await.map_err(|error| {
            if error.kind() == std::io::ErrorKind::StorageFull {
                DispatchError::new(
                    StatusCode::INSUFFICIENT_STORAGE,
                    "Insufficient space within the Embedded disk allowance; start a new upload when space is available",
                )
            } else {
                crate::internal(error).into()
            }
        })?;
        check_upload_live(&entry)?;
        session.digest.update(&data);
        session.written_bytes = next_offset;
        session.part = Some(part);
        Ok(IpcPayload::DatasetUploadProgress {
            received_bytes: next_offset,
        })
    }

    async fn dataset_upload_session(
        &self,
        upload_id: &str,
        binding: &UploadTicketBinding,
    ) -> Result<UploadSessionEntry, DispatchError> {
        let mut sessions = self.upload_sessions.lock().await;
        let now = Instant::now();
        sessions.retain(|_, existing| existing.expires_at > now);
        let entry = sessions.get(upload_id).ok_or_else(|| {
            DispatchError::new(
                StatusCode::NOT_FOUND,
                "Dataset upload destination not found",
            )
        })?;
        if &entry.binding != binding {
            return Err(DispatchError::new(
                StatusCode::FORBIDDEN,
                "Dataset upload belongs to another credential",
            ));
        }
        Ok(entry.clone())
    }

    async fn handle_dataset_upload_cancel(
        &self,
        principal: &AuthenticatedPrincipal,
        registration: Option<&RegisteredContext>,
        upload_id: String,
    ) -> Result<IpcPayload, DispatchError> {
        if !valid_upload_id(&upload_id) {
            return Err(DispatchError::bad_request("Invalid dataset upload ID"));
        }
        let context = self
            .resolve(
                principal,
                registration,
                Permission::MaintainBackend,
                ContextRequirement::Registered,
            )
            .await?;
        context.revalidate(&self.state.repo).await?;
        let binding = datasets::upload_binding(&context).ok_or_else(|| {
            DispatchError::new(
                StatusCode::FORBIDDEN,
                "Native or Operator upload credential required",
            )
        })?;
        let mut sessions = self.upload_sessions.lock().await;
        if let Some(entry) = sessions.get(&upload_id) {
            if entry.binding != binding {
                return Err(DispatchError::new(
                    StatusCode::FORBIDDEN,
                    "Dataset upload belongs to another credential",
                ));
            }
            entry.cancelled.store(true, Ordering::Release);
        }
        let cancelled = sessions.remove(&upload_id);
        drop(sessions);
        drop(cancelled);
        Ok(IpcPayload::Empty)
    }

    async fn handle_dataset_upload_finish(
        &self,
        principal: &AuthenticatedPrincipal,
        registration: Option<&RegisteredContext>,
        upload_id: String,
    ) -> Result<IpcPayload, DispatchError> {
        let context = self
            .resolve(
                principal,
                registration,
                Permission::MaintainBackend,
                ContextRequirement::Registered,
            )
            .await?;
        if !valid_upload_id(&upload_id) {
            return Err(DispatchError::bad_request("Invalid dataset upload ID"));
        }
        let store =
            self.services.datasets.as_ref().ok_or_else(|| {
                DispatchError::unavailable("Dataset staging store is unavailable")
            })?;
        store.verify_root().await.map_err(crate::internal)?;
        let binding = datasets::upload_binding(&context).ok_or_else(|| {
            DispatchError::new(
                StatusCode::FORBIDDEN,
                "Native or Operator upload credential required",
            )
        })?;
        let entry = self.dataset_upload_session(&upload_id, &binding).await?;
        let mut session = entry.session.lock().await;
        check_upload_live(&entry)?;
        if session.written_bytes != session.expected_size_bytes {
            return Err(DispatchError::bad_request("Dataset upload is incomplete"));
        }
        let permit = store.uploads.clone().try_acquire_owned().map_err(|_| {
            DispatchError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "Dataset upload capacity reached",
            )
        })?;
        let part = session.part.take().ok_or_else(|| {
            DispatchError::new(StatusCode::NOT_FOUND, "Dataset upload destination closed")
        })?;
        let transfer_ms = session.started_at.elapsed().as_millis() as u64;
        let validation_started = Instant::now();
        let written_sha256 = format!("{:x}", std::mem::take(&mut session.digest).finalize());
        let expected_kind = session.kind;
        let expected_size = session.expected_size_bytes;
        let expected_sha256 = session.expected_sha256.clone();
        drop(session);
        let staged = async {
            let lease = upload_ticket::claim(&self.state.repo, &binding, &upload_id)
                .await
                .map_err(|error| {
                    datasets::ticket_failure_status_message(error, StatusCode::NOT_FOUND)
                })?;
            if lease.kind != expected_kind
                || lease.size_bytes != expected_size
                || lease.sha256 != expected_sha256
            {
                return Err(DispatchError::bad_request("Dataset upload metadata mismatch"));
            }
            if written_sha256 != lease.sha256 {
                return Err(DispatchError::new(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "Dataset SHA-256 mismatch; request a new destination",
                ));
            }
            store
                .finish_local_part(&self.state.repo, &context, part, lease, permit, entry.cancelled)
                .await
                .map_err(|_| DispatchError::new(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "Upload validation failed or staging capacity is unavailable; request a new destination",
                ))
        }
        .await;
        self.upload_sessions.lock().await.remove(&upload_id);
        let upload = staged?;
        if context.revalidate(&self.state.repo).await.is_err() {
            return Err(DispatchError::new(
                StatusCode::UNAUTHORIZED,
                "Upload authority changed; recover staged metadata after reconnecting",
            ));
        }
        tracing::info!(
            code = "dataset_upload_single_copy",
            bytes = upload.dataset.size_bytes,
            transfer_ms,
            validation_publish_ms = validation_started.elapsed().as_millis() as u64,
            "Embedded dataset received and published"
        );
        Ok(IpcPayload::DatasetUpload(upload))
    }
}

fn check_upload_live(entry: &UploadSessionEntry) -> Result<(), DispatchError> {
    if entry.cancelled.load(Ordering::Acquire) || Instant::now() >= entry.expires_at {
        return Err(DispatchError::new(
            StatusCode::NOT_FOUND,
            "Dataset upload destination closed",
        ));
    }
    Ok(())
}

fn valid_upload_id(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && id.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
        })
}

fn convert_log_query(
    query: fleetworks_contracts::operational_logs::LogQuery,
) -> operational_logs::LogQuery {
    operational_logs::LogQuery {
        job_id: query.job_id,
        workspace_id: None,
        descendants: query.descendants,
        min_rank: query.min_rank,
        app: query.app,
        text: query.text,
        since: query.since,
        limit: query.limit,
        cursor: query.cursor,
        wait_ms: query.wait_ms,
    }
}

fn build_ready_contract(ready: bool, instance_id: &str) -> Ready {
    serde_json::from_value(serde_json::json!({
        "service": "argos-intel",
        "api_version": API_VERSION,
        "ready": ready,
        "storage": ready,
        "instance_id": instance_id,
    }))
    .expect("valid Ready contract payload")
}

fn json_payload(value: impl Serialize) -> Result<IpcPayload, DispatchError> {
    serde_json::to_value(value)
        .map(IpcPayload::Json)
        .map_err(|error| DispatchError::from(crate::internal(error)))
}

fn error_code(status: StatusCode) -> &'static str {
    match status {
        StatusCode::BAD_REQUEST => "bad_request",
        StatusCode::UNAUTHORIZED => "unauthorized",
        StatusCode::FORBIDDEN => "forbidden",
        StatusCode::NOT_FOUND => "not_found",
        StatusCode::CONFLICT => "conflict",
        StatusCode::PAYLOAD_TOO_LARGE => "payload_too_large",
        StatusCode::UNPROCESSABLE_ENTITY => "unprocessable_entity",
        StatusCode::TOO_MANY_REQUESTS => "too_many_requests",
        StatusCode::SERVICE_UNAVAILABLE => "service_unavailable",
        _ => "request_failed",
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::atomic::AtomicUsize;
    use std::sync::atomic::Ordering;

    use argos_osint_core::config_commit::CommitFile;
    use argos_osint_core::config_commit::ConfigLock;
    use argos_osint_core::provider;
    use argos_osint_core::provider::SettingsFile;
    use argos_osint_core::storage::bootstrap;
    use argos_osint_core::storage::connect_registry;
    use argos_osint_core::storage::repo::Repository;
    use fleetworks_contracts::actor_index::ActorIndexGeneration;
    use fleetworks_contracts::actor_index::ActorIndexHit;
    use fleetworks_contracts::actor_index::ActorIndexLimits;
    use fleetworks_contracts::actor_index::ActorIndexQuery;
    use fleetworks_contracts::actor_index::ActorIndexQueryResponse;
    use fleetworks_contracts::actor_index::ActorIndexStatus;
    use fleetworks_contracts::actor_index::ActorProjection;
    use fleetworks_contracts::connect::RegistrationEnsureRequest;
    use fleetworks_contracts::dataset_upload::DatasetUploadRequest;
    use fleetworks_contracts::readiness::MaintenanceRequest;
    use fleetworks_contracts::search::ActorSearchQuery;
    use fleetworks_contracts::AtlasCycleRequest;
    use serde_json::json;
    use sha2::Digest;
    use sha2::Sha256;

    use super::*;
    use crate::ActorSearchBackend;
    use crate::SearchClient;
    use crate::SearchFuture;

    struct MockSearchBackend {
        status_calls: Arc<AtomicUsize>,
        query_calls: Arc<AtomicUsize>,
        deployment_id: String,
        source_epoch: String,
        index_generation: String,
    }

    impl ActorSearchBackend for MockSearchBackend {
        fn status(&self) -> SearchFuture<'_, ActorIndexStatus> {
            self.status_calls.fetch_add(1, Ordering::Relaxed);
            let deployment_id = self.deployment_id.clone();
            let source_epoch = self.source_epoch.clone();
            let index_generation = self.index_generation.clone();
            Box::pin(async move {
                Ok(ActorIndexStatus {
                    protocol: 1,
                    deployment_id,
                    source_epoch,
                    schema_version: fleetworks_contracts::actor_index::ACTOR_INDEX_SCHEMA,
                    analyzer_version: fleetworks_contracts::actor_index::ACTOR_INDEX_ANALYZER,
                    tantivy_version: "0.22".to_owned(),
                    index_format_version: 1,
                    active: Some(ActorIndexGeneration {
                        generation: index_generation.clone(),
                        committed_checkpoint: 1,
                        visible_checkpoint: 1,
                        source_complete: true,
                        writer_epoch: 1,
                    }),
                    rebuilding: None,
                    retained_generations: 1,
                    retained_generation_ids: vec![index_generation],
                    manifest_checksum: None,
                    recovery_available: false,
                    limits: ActorIndexLimits::default(),
                    pressure: None,
                    state: "ready".to_owned(),
                    reason_code: None,
                })
            })
        }

        fn query<'a>(
            &'a self,
            _query: &'a ActorIndexQuery,
        ) -> SearchFuture<'a, ActorIndexQueryResponse> {
            self.query_calls.fetch_add(1, Ordering::Relaxed);
            let deployment_id = self.deployment_id.clone();
            let source_epoch = self.source_epoch.clone();
            let index_generation = self.index_generation.clone();
            Box::pin(async move {
                Ok(ActorIndexQueryResponse {
                    deployment_id,
                    source_epoch,
                    index_generation,
                    visible_checkpoint: 1,
                    source_complete: true,
                    serving_state: "ready".to_owned(),
                    hits: vec![ActorIndexHit {
                        actor_id: "union".to_owned(),
                        revision: 1,
                        projection: ActorProjection {
                            display_name: "Local Union".to_owned(),
                            aliases: Vec::new(),
                            summary: None,
                            actor_type: Some("organization".to_owned()),
                            countries: Vec::new(),
                            source_id: None,
                            authority_scopes: vec!["backend".to_owned()],
                            eligible: true,
                        },
                        matched_field: "name".to_owned(),
                        score: 9.5,
                    }],
                    truncated: false,
                })
            })
        }
    }

    fn init_workspace_config(root: &Path) -> u64 {
        let mut settings = SettingsFile::default();
        for role in provider::CANONICAL_ROLE_KEYS {
            *settings.defaults.role_mut(role).expect("canonical role") = Default::default();
        }
        let mut value = toml::Value::try_from(settings).expect("settings");
        value
            .as_table_mut()
            .expect("table")
            .insert("configuration_format".into(), toml::Value::Integer(1));
        ConfigLock::acquire_at(root)
            .expect("config lock")
            .commit_files(&[
                CommitFile::new(
                    "current_settings",
                    root.join("config.toml"),
                    toml::to_string(&value).expect("toml"),
                ),
                CommitFile::new(
                    "auth",
                    root.join("auth.json"),
                    r#"{"configuration_format":1,"accounts":{}}"#.to_owned(),
                ),
                CommitFile::new(
                    "quota",
                    root.join("quota.json"),
                    r#"{"settings":[]}"#.to_owned(),
                ),
            ])
            .expect("initial config commit")
    }

    struct TestRepo {
        repo: Repository,
        _server: Option<crate::storage_test_support::Server>,
    }

    async fn open_test_repo(root: &Path, namespace: &str, db_name: &str) -> TestRepo {
        #[cfg(not(feature = "embedded"))]
        let (repo, _server) = {
            let datastore = crate::storage_test_support::Server::start()
                .expect("required pinned isolated SurrealDB harness");
            let repo = datastore.repo(namespace, db_name).await;
            (repo, Some(datastore))
        };
        #[cfg(feature = "embedded")]
        let (repo, _server) = (
            Repository::connect(
                &argos_osint_core::storage::open::Target::Embedded {
                    path: root.join("store"),
                },
                namespace,
                db_name,
            )
            .await
            .expect("embedded test repository"),
            None,
        );
        bootstrap::ensure_network_identity(repo.db())
            .await
            .expect("deployment identity");
        connect_registry::initialize_control(repo.db())
            .await
            .expect("connect control");
        TestRepo { repo, _server }
    }

    fn full_ipc_permissions() -> Vec<Permission> {
        vec![
            Permission::ReadBackend,
            Permission::RegisterWorkstation,
            Permission::RebindWorkstation,
            Permission::ReadWorkstationActivity,
            Permission::SubmitInteractive,
            Permission::RetryJob,
            Permission::CancelJob,
            Permission::MaintainBackend,
            Permission::ManageConfiguration,
        ]
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }

    async fn start_actor_upload(
        gateway: &LocalGateway,
        principal: &AuthenticatedPrincipal,
        registration: &RegisteredContext,
        bytes: &[u8],
        sha256: String,
    ) -> String {
        let request =
            DatasetUploadRequest::new(DatasetKind::ActorCatalog, sha256, bytes.len() as u64)
                .expect("upload request");
        let outcome = gateway
            .dispatch(
                principal,
                Some(registration),
                IpcOperation::DatasetUploadStart(request),
            )
            .await;
        let IpcOutcome::Ok(IpcPayload::Json(ticket)) = outcome else {
            panic!("expected upload ticket, got {outcome:?}");
        };
        ticket["upload_path"]
            .as_str()
            .expect("upload path")
            .rsplit('/')
            .next()
            .expect("upload ID")
            .to_owned()
    }

    fn only_staged_part(root: &Path) -> std::path::PathBuf {
        let parts: Vec<_> = std::fs::read_dir(root)
            .expect("staging files")
            .map(|entry| entry.expect("staging entry").path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "part"))
            .collect();
        assert_eq!(parts.len(), 1, "the upload owns one private file");
        parts.into_iter().next().expect("one staged part")
    }

    #[test]
    fn local_ipc_principal_validation_enforces_identity_and_permission_boundaries() {
        assert!(AuthenticatedPrincipal::for_local_ipc(
            "paired-stolen",
            "scope-1",
            "epoch-1",
            vec![Permission::ReadBackend],
        )
        .is_err());
        assert!(AuthenticatedPrincipal::for_local_ipc(
            "local-ipc-1",
            "scope-1",
            "epoch-1",
            vec![Permission::ResolveWebSession],
        )
        .is_err());
        assert!(
            AuthenticatedPrincipal::for_local_ipc("local-ipc-1", "scope-1", "epoch-1", vec![])
                .is_err()
        );
        assert!(AuthenticatedPrincipal::for_local_ipc(
            "local-ipc-1",
            "scope-1",
            "epoch-1",
            vec![Permission::ReadBackend, Permission::ReadBackend],
        )
        .is_err());
        assert!(AuthenticatedPrincipal::for_local_ipc(
            "bad/id",
            "scope-1",
            "epoch-1",
            vec![Permission::ReadBackend],
        )
        .is_err());

        let principal = AuthenticatedPrincipal::for_local_ipc(
            "local-ipc-1",
            "scope-1",
            "epoch-1",
            vec![Permission::ReadBackend, Permission::ManageConfiguration],
        )
        .expect("valid local ipc principal");
        assert_eq!(principal.purpose(), CredentialPurpose::Native);
        assert_eq!(principal.credential_id(), "local-ipc-1");
        assert_eq!(principal.scope_id(), "scope-1");
        assert_eq!(principal.access_epoch(), "epoch-1");
        assert!(principal.permits(Permission::ReadBackend));
        assert!(!principal.permits(Permission::ResolveWebSession));
    }

    #[tokio::test]
    async fn search_client_from_local_delegates_status_and_query() {
        let status_calls = Arc::new(AtomicUsize::new(0));
        let query_calls = Arc::new(AtomicUsize::new(0));
        let client = SearchClient::from_local(Arc::new(MockSearchBackend {
            status_calls: Arc::clone(&status_calls),
            query_calls: Arc::clone(&query_calls),
            deployment_id: "dep-1".to_owned(),
            source_epoch: "epoch-1".to_owned(),
            index_generation: "gen-1".to_owned(),
        }));

        let status = client.status().await.expect("local status");
        assert_eq!(status.state, "ready");
        assert_eq!(status_calls.load(Ordering::Relaxed), 1);

        let query = ActorIndexQuery {
            query: "union".to_owned(),
            limit: 5,
        };
        assert!(query.validate().is_ok());
        let response = client.query(&query).await.expect("local query");
        assert_eq!(response.hits.len(), 1);
        assert_eq!(response.hits[0].actor_id, "union");
        assert_eq!(query_calls.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn workspace_for_uses_explicit_workspace_root_without_env_vars() {
        let dir = tempfile::tempdir().expect("temp workspace");
        let root = dir.path().canonicalize().expect("canonical");
        let initial_revision = init_workspace_config(&root);
        let test_repo = open_test_repo(&root, "local_ws", "isolated_root").await;
        let state = ApiState::new(
            test_repo.repo,
            "inst-ws".to_owned(),
            0,
            Arc::new(Vec::new()),
        )
        .with_workspace_root(root.clone());

        assert_eq!(
            configuration::workspace_for(&state).expect("explicit workspace"),
            root
        );
        let profile = configuration::profile_value(&state)
            .await
            .expect("profile from explicit root");
        assert_eq!(profile["configuration_revision"], json!(initial_revision));
    }

    #[tokio::test]
    async fn local_gateway_capabilities_and_dispatch_enforce_security_and_operations() {
        let dir = tempfile::tempdir().expect("temp workspace");
        let root = dir.path().canonicalize().expect("canonical");
        let initial_revision = init_workspace_config(&root);
        let staging_dir = root.join("datasets");
        std::fs::create_dir_all(&staging_dir).expect("create dataset dir");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&staging_dir, std::fs::Permissions::from_mode(0o700))
                .expect("private dataset dir");
        }
        let dataset_store =
            datasets::DatasetStore::open(staging_dir.clone()).expect("dataset store");

        let test_repo = open_test_repo(&root, "local_dispatch", "gateway_test").await;
        let repo = test_repo.repo;
        let deployment = storage::connect_registry::deployment_state(&repo)
            .await
            .expect("deployment");

        let status_calls = Arc::new(AtomicUsize::new(0));
        let query_calls = Arc::new(AtomicUsize::new(0));
        let search_client = SearchClient::from_local(Arc::new(MockSearchBackend {
            status_calls,
            query_calls,
            deployment_id: deployment.deployment_id.clone(),
            source_epoch: deployment.configuration_epoch.clone(),
            index_generation: "gen-1".to_owned(),
        }));

        let state = ApiState::new(
            repo.clone(),
            "inst-local".to_owned(),
            0,
            Arc::new(Vec::new()),
        )
        .with_workspace_root(root.clone());
        let services = PrivateServices::new(None, None, None, None)
            .with_search(search_client)
            .with_datasets(dataset_store.clone());

        let disabled_gateway = LocalGateway::new(
            state.clone(),
            services.clone(),
            LocalCapabilityConfig {
                nlp_enabled: false,
                classifier_enabled: false,
                search_enabled: false,
                jobs_enabled: false,
            },
        )
        .expect("disabled gateway");
        let disabled_caps = disabled_gateway
            .capabilities(Vec::new())
            .await
            .expect("disabled capabilities");
        assert!(disabled_caps.search.is_none());
        assert!(!disabled_caps.features.iter().any(|f| f == "jobs"));
        assert!(!disabled_caps.features.iter().any(|f| f == "classifier"));
        assert!(!disabled_caps.features.iter().any(|f| f == "nlp"));
        disabled_caps
            .require_native_contract()
            .expect("disabled gateway still advertises deployment contract");
        assert_eq!(
            disabled_caps.deployment_id.as_deref(),
            Some(deployment.deployment_id.as_str())
        );
        assert_eq!(
            disabled_caps.configuration_epoch.as_deref(),
            Some(deployment.configuration_epoch.as_str())
        );

        let gateway = LocalGateway::new(
            state,
            services,
            LocalCapabilityConfig {
                nlp_enabled: true,
                classifier_enabled: true,
                search_enabled: true,
                jobs_enabled: true,
            },
        )
        .expect("enabled gateway");
        let enabled_caps = gateway
            .capabilities(full_ipc_permissions())
            .await
            .expect("enabled capabilities");
        assert!(enabled_caps.search.is_some());
        assert!(enabled_caps.features.iter().any(|f| f == "jobs"));
        assert!(enabled_caps.features.iter().any(|f| f == "classifier"));
        assert!(enabled_caps.features.iter().any(|f| f == "nlp"));
        enabled_caps
            .require_native_contract()
            .expect("enabled gateway satisfies native contract");
        assert_eq!(
            enabled_caps.deployment_id.as_deref(),
            Some(deployment.deployment_id.as_str())
        );
        assert_eq!(
            enabled_caps.configuration_epoch.as_deref(),
            Some(deployment.configuration_epoch.as_str())
        );
        assert!(gateway.ready().await.ready);

        let principal = AuthenticatedPrincipal::for_local_ipc(
            "local-client-1",
            "scope-local",
            "epoch-1",
            full_ipc_permissions(),
        )
        .expect("principal");
        let other_principal = AuthenticatedPrincipal::for_local_ipc(
            "local-client-2",
            "scope-other",
            "epoch-1",
            full_ipc_permissions(),
        )
        .expect("other principal");
        let read_only = AuthenticatedPrincipal::for_local_ipc(
            "local-ro-1",
            "scope-local",
            "epoch-1",
            vec![Permission::ReadBackend],
        )
        .expect("read only");

        // Ensure registration & resolve RegisteredContext.
        let ensured = gateway
            .dispatch(
                &principal,
                None,
                IpcOperation::RegistrationEnsure(RegistrationEnsureRequest::new(
                    "ensure-1".to_owned(),
                    None,
                    "install-marker-local-1".to_owned(),
                )),
            )
            .await;
        let IpcOutcome::Ok(IpcPayload::Json(ensured_json)) = ensured else {
            panic!("expected registration ensure success, got {ensured:?}");
        };
        let reg_id = ensured_json["registration_id"]
            .as_str()
            .expect("registration_id")
            .to_owned();
        let reg_rev = ensured_json["revision"].as_u64().expect("revision");
        let ctx_req: fleetworks_contracts::ipc::RegistrationContextRequest =
            serde_json::from_value(json!({
                "registration_id": reg_id,
                "expected_revision": reg_rev,
            }))
            .expect("context request");
        let ctx_outcome = gateway
            .dispatch(&principal, None, IpcOperation::RegistrationContext(ctx_req))
            .await;
        let IpcOutcome::Ok(IpcPayload::Json(ctx_json)) = ctx_outcome else {
            panic!("expected registration context success, got {ctx_outcome:?}");
        };
        let registration: RegisteredContext =
            serde_json::from_value(ctx_json["context"].clone()).expect("registered context");

        // Permission check: read_only cannot export configuration.
        let denied_export = gateway
            .dispatch(
                &read_only,
                None,
                IpcOperation::ConfigurationExport {
                    expected_revision: initial_revision,
                },
            )
            .await;
        assert!(matches!(denied_export, IpcOutcome::Err { status: 403, .. }));

        // Configuration export -> import roundtrip works using explicit workspace_root.
        let exported = gateway
            .dispatch(
                &principal,
                None,
                IpcOperation::ConfigurationExport {
                    expected_revision: initial_revision,
                },
            )
            .await;
        let IpcOutcome::Ok(IpcPayload::ConfigurationExport { bytes_b64 }) = exported else {
            panic!("expected configuration export payload, got {exported:?}");
        };
        let imported = gateway
            .dispatch(
                &principal,
                None,
                IpcOperation::ConfigurationImport {
                    expected_revision: initial_revision,
                    bytes_b64: bytes_b64.clone(),
                },
            )
            .await;
        let IpcOutcome::Ok(IpcPayload::Json(imported_json)) = imported else {
            panic!("expected configuration import ok, got {imported:?}");
        };
        assert_eq!(imported_json["saved"], json!(true));
        let updated_revision = imported_json["configuration_revision"]
            .as_u64()
            .expect("updated configuration revision");
        assert!(updated_revision > initial_revision);

        // Stale revision after import is rejected with 409 Conflict.
        let stale_export = gateway
            .dispatch(
                &principal,
                None,
                IpcOperation::ConfigurationExport {
                    expected_revision: initial_revision,
                },
            )
            .await;
        assert!(matches!(stale_export, IpcOutcome::Err { status: 409, .. }));
        let stale_import = gateway
            .dispatch(
                &principal,
                None,
                IpcOperation::ConfigurationImport {
                    expected_revision: initial_revision,
                    bytes_b64,
                },
            )
            .await;
        assert!(matches!(stale_import, IpcOutcome::Err { status: 409, .. }));

        // Registered context required on activity/maintenance routes.
        let jobs_query = fleetworks_contracts::ipc::JobsListQuery::default();
        let missing_ctx = gateway
            .dispatch(&principal, None, IpcOperation::JobsList(jobs_query.clone()))
            .await;
        assert!(matches!(
            missing_ctx,
            IpcOutcome::Err {
                status: 409 | 428 | 401 | 403,
                ..
            }
        ));

        // Registered context from another principal is rejected.
        for operation in [IpcOperation::JobsList(jobs_query), IpcOperation::QaHistory] {
            let wrong_ctx = gateway
                .dispatch(&other_principal, Some(&registration), operation)
                .await;
            assert!(matches!(
                wrong_ctx,
                IpcOutcome::Err {
                    status: 403 | 401 | 409,
                    ..
                }
            ));
        }

        let without_activity = AuthenticatedPrincipal::for_local_ipc(
            principal.credential_id(),
            principal.scope_id(),
            principal.access_epoch(),
            vec![Permission::ReadBackend],
        )
        .expect("same principal without activity permission");
        let denied_history = gateway
            .dispatch(
                &without_activity,
                Some(&registration),
                IpcOperation::QaHistory,
            )
            .await;
        assert!(matches!(
            denied_history,
            IpcOutcome::Err { status: 403, .. }
        ));

        // Capability gating rejects disabled operations with 503.
        let search_query = ActorSearchQuery::new("union".to_owned(), 5).expect("query");
        let disabled_search = disabled_gateway
            .dispatch(
                &principal,
                Some(&registration),
                IpcOperation::ActorsSearch(search_query.clone()),
            )
            .await;
        assert!(matches!(
            disabled_search,
            IpcOutcome::Err { status: 503, .. }
        ));

        let cycle_req = AtlasCycleRequest::new("cycle-disabled".to_owned()).expect("valid request");
        let disabled_cycle = disabled_gateway
            .dispatch(
                &principal,
                Some(&registration),
                IpcOperation::AtlasCycleSubmit(cycle_req),
            )
            .await;
        assert!(matches!(
            disabled_cycle,
            IpcOutcome::Err { status: 503, .. }
        ));

        let maint_req: MaintenanceRequest = serde_json::from_value(json!({
            "request_id": "maint-disabled",
            "context": {
                "registration_id": registration.registration_id,
                "expected_registration_revision": registration.registration_revision,
                "expected_context_revision": registration.context_revision,
            },
        }))
        .expect("maintenance request");
        let retired_train = disabled_gateway
            .dispatch(
                &principal,
                Some(&registration),
                IpcOperation::TrainClassifier(maint_req),
            )
            .await;
        assert!(matches!(retired_train, IpcOutcome::Err { status: 410, .. }));

        // Enabled search succeeds (returning NotBuilt before outbox indexing begins).
        let search_outcome = gateway
            .dispatch(&principal, None, IpcOperation::ActorsSearch(search_query))
            .await;
        assert!(matches!(
            search_outcome,
            IpcOutcome::Ok(IpcPayload::Json(_))
        ));

        // Dataset chunked upload roundtrip & tampered chunk/binding rejection.
        let csv_bytes = b"actor_id,actor_name,actor_type,aliases,name_normalized\nact-1,Acme Org,organization,[],acme org\n";
        let upload_id = start_actor_upload(
            &gateway,
            &principal,
            &registration,
            csv_bytes,
            sha256_hex(csv_bytes),
        )
        .await;
        let part = only_staged_part(&staging_dir);
        #[cfg(unix)]
        let initial_inode = {
            use std::os::unix::fs::MetadataExt;
            let metadata = std::fs::metadata(&part).expect("open upload inode");
            (metadata.dev(), metadata.ino())
        };

        // Wrong offset is rejected with 409 Conflict.
        let bad_offset = gateway
            .dispatch(
                &principal,
                Some(&registration),
                IpcOperation::DatasetUploadChunk {
                    upload_id: upload_id.clone(),
                    offset: 1,
                    data: csv_bytes.to_vec(),
                },
            )
            .await;
        assert!(matches!(bad_offset, IpcOutcome::Err { status: 409, .. }));

        for data in [Vec::new(), vec![0; MAX_IPC_CHUNK_BYTES + 1]] {
            let outcome = gateway
                .dispatch(
                    &principal,
                    Some(&registration),
                    IpcOperation::DatasetUploadChunk {
                        upload_id: upload_id.clone(),
                        offset: 0,
                        data,
                    },
                )
                .await;
            assert!(matches!(outcome, IpcOutcome::Err { status: 400, .. }));
        }

        // Valid chunk & finish succeed.
        for (offset, chunk) in [(0, &csv_bytes[..32]), (32, &csv_bytes[32..])] {
            let chunk_ok = gateway
                .dispatch(
                    &principal,
                    Some(&registration),
                    IpcOperation::DatasetUploadChunk {
                        upload_id: upload_id.clone(),
                        offset,
                        data: chunk.to_vec(),
                    },
                )
                .await;
            assert!(matches!(
                chunk_ok,
                IpcOutcome::Ok(IpcPayload::DatasetUploadProgress { received_bytes })
                    if received_bytes == offset + chunk.len() as u64
            ));
        }

        let finish_ok = gateway
            .dispatch(
                &principal,
                Some(&registration),
                IpcOperation::DatasetUploadFinish { upload_id },
            )
            .await;
        let IpcOutcome::Ok(IpcPayload::DatasetUpload(uploaded)) = finish_ok else {
            panic!("expected DatasetUpload finish payload, got {finish_ok:?}");
        };
        assert_eq!(uploaded.dataset.size_bytes, csv_bytes.len() as u64);
        assert_eq!(uploaded.dataset.sha256, sha256_hex(csv_bytes));
        assert_eq!(uploaded.row_count, Some(1));
        let published = staging_dir.join(format!("{}.csv", uploaded.dataset.id));
        assert!(
            !part.exists(),
            "publication removes the private partial name"
        );
        assert_eq!(
            std::fs::read(&published).expect("published bytes"),
            csv_bytes
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let metadata = std::fs::metadata(&published).expect("published inode");
            assert_eq!(
                (metadata.dev(), metadata.ino()),
                initial_inode,
                "publication reuses the originally received inode"
            );
        }

        let cancel_id = start_actor_upload(
            &gateway,
            &principal,
            &registration,
            csv_bytes,
            sha256_hex(csv_bytes),
        )
        .await;
        let cancelled_part = only_staged_part(&staging_dir);
        for _ in 0..2 {
            let outcome = gateway
                .dispatch(
                    &principal,
                    Some(&registration),
                    IpcOperation::DatasetUploadCancel {
                        upload_id: cancel_id.clone(),
                    },
                )
                .await;
            assert!(matches!(outcome, IpcOutcome::Ok(IpcPayload::Empty)));
        }
        assert!(
            !cancelled_part.exists(),
            "cancellation unlinks the partial file"
        );

        for (expected, collision) in [(sha256_hex(csv_bytes), true), ("0".repeat(64), false)] {
            let upload_id =
                start_actor_upload(&gateway, &principal, &registration, csv_bytes, expected).await;
            let part = only_staged_part(&staging_dir);
            let final_path = part.with_extension("csv");
            if collision {
                std::fs::write(&final_path, b"unrelated destination").expect("write collision");
            }
            let chunk = gateway
                .dispatch(
                    &principal,
                    Some(&registration),
                    IpcOperation::DatasetUploadChunk {
                        upload_id: upload_id.clone(),
                        offset: 0,
                        data: csv_bytes.to_vec(),
                    },
                )
                .await;
            assert!(matches!(
                chunk,
                IpcOutcome::Ok(IpcPayload::DatasetUploadProgress { received_bytes })
                    if received_bytes == csv_bytes.len() as u64
            ));
            let outcome = gateway
                .dispatch(
                    &principal,
                    Some(&registration),
                    IpcOperation::DatasetUploadFinish { upload_id },
                )
                .await;
            assert!(matches!(outcome, IpcOutcome::Err { status: 422, .. }));
            assert!(!part.exists(), "rejected upload removes its private name");
            if collision {
                let dataset_id = final_path.file_stem().unwrap().to_str().unwrap().to_owned();
                repo.db()
                    .query("UPDATE setup_upload SET expires_at=time::now()-1s WHERE dataset_id=$id")
                    .bind(("id", dataset_id))
                    .await
                    .expect("expire collision journal")
                    .check()
                    .expect("expire collision statement");
                dataset_store
                    .expire_abandoned(&repo)
                    .await
                    .expect("recover expired local upload");
                assert_eq!(
                    std::fs::read(final_path).expect("collision survives expiry"),
                    b"unrelated destination"
                );
            } else {
                assert!(!final_path.exists(), "wrong SHA cannot publish a file");
            }
        }

        let _abandoned = start_actor_upload(
            &gateway,
            &principal,
            &registration,
            csv_bytes,
            sha256_hex(csv_bytes),
        )
        .await;
        let disconnected_part = only_staged_part(&staging_dir);

        // Reset fence blocks subsequent local dispatch with 401.
        repo.db()
            .query("CREATE configuration_reset_scope:fence SET scope_id='scope-local', active_reset='r1', access_epoch='epoch-2', first_use_blocked=true, revoked_json='[]' RETURN NONE")
            .await
            .expect("insert reset fence")
            .check()
            .expect("reset fence check");
        let fenced = gateway
            .dispatch(&principal, None, IpcOperation::ProfileRead)
            .await;
        assert!(matches!(fenced, IpcOutcome::Err { status: 401, .. }));
        drop(gateway);
        assert!(
            !disconnected_part.exists(),
            "disconnect drops bounded partial uploads"
        );
    }
}
