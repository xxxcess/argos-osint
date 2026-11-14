//! Typed global read models and registered workstation activity.

use std::collections::BTreeMap;

use fleetworks_contracts::context::ActivityScope;
use fleetworks_contracts::context::Permission;
use fleetworks_contracts::Client;
use rune::TextEdit;
use serde::Deserialize;
use serde::Serialize;

use crate::app::Intent;
use crate::app::Load;
use crate::app::Page;
use crate::app::State;
use crate::connection::GatewayEffects;
use crate::connection::RequestTask;

#[derive(Clone, Debug, Deserialize)]
pub struct Article {
    pub id: String,
    pub title: String,
    pub description: String,
    pub source_name: String,
    pub published_at: String,
    pub category: String,
    #[serde(default)]
    pub categories: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ArticlePage {
    pub rows: Vec<Article>,
    pub next_cursor: Option<String>,
    pub revision: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct ArticleQuery {
    pub day: String,
    pub category: String,
    pub search: String,
    pub sort: &'static str,
    pub limit: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ToolDefinition {
    pub id: String,
    pub name: String,
    pub category: String,
    pub description: String,
    pub inputs: Vec<String>,
    pub restrictions: String,
    pub timeout_seconds: u64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ToolCatalog {
    pub tools: Vec<ToolDefinition>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ModelRoute {
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub account: String,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

impl ModelRoute {
    pub fn label(&self) -> String {
        if self.provider.is_empty() || self.model.is_empty() {
            "Unconfigured".into()
        } else if self.account.is_empty() {
            format!("{} · {}", self.provider, self.model)
        } else {
            format!("{} / {} · {}", self.provider, self.account, self.model)
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct ModelAssignment {
    #[serde(flatten)]
    pub primary: ModelRoute,
    #[serde(default)]
    pub fallbacks: Vec<ModelRoute>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ModelDefaults {
    pub roles: Vec<String>,
    pub defaults: BTreeMap<String, serde_json::Value>,
    pub configuration_revision: u64,
    #[serde(default)]
    pub catalog: Vec<ModelCatalogEntry>,
    #[serde(default)]
    pub model_variants: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ModelCatalogEntry {
    pub provider: String,
    pub account: String,
    pub model: String,
    pub efforts: Vec<String>,
}

impl ModelCatalogEntry {
    pub fn label(&self) -> String {
        format!("{} · {}", self.model, self.account)
    }
}

impl ModelDefaults {
    /// Only server-advertised canonical roles are active. Never infer role
    /// inheritance from archived or legacy objects in the configuration file.
    pub fn assignment(&self, role: &str) -> Option<ModelAssignment> {
        if !self.roles.iter().any(|candidate| candidate == role) {
            return None;
        }
        self.defaults
            .get(role)
            .and_then(|value| serde_json::from_value(value.clone()).ok())
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct AccountMetadata {
    pub id: String,
    pub provider: String,
    pub base_url: String,
    pub model: String,
    pub has_api_key: bool,
    pub credential_reference: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ToolCredentialMetadata {
    pub provider: String,
    pub primary_present: bool,
    pub fallback_present: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ProfileSnapshot {
    pub configuration_revision: u64,
    pub configuration_format: u16,
    pub accounts: Vec<AccountMetadata>,
    pub tool_credentials: Vec<ToolCredentialMetadata>,
    pub setup_required: bool,
    pub portable_schema_version: u16,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ActivityJob {
    pub id: String,
    pub app: String,
    pub title: String,
    pub operation: String,
    pub state: String,
    pub phase: String,
    pub progress_done: Option<i64>,
    pub progress_total: Option<i64>,
    pub error_summary: String,
    pub private_details_available: bool,
    pub activity_actions_available: bool,
    pub status_only: bool,
    #[serde(default)]
    pub can_cancel: bool,
    #[serde(default)]
    pub can_retry: bool,
    #[serde(default)]
    pub action_reason_code: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ActivityJobs {
    pub rows: Vec<ActivityJob>,
    pub activity_scope: ActivityScope,
    pub relation_counts_available: bool,
    pub activity_actions_available: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct JobStage {
    #[serde(default)]
    pub operation: String,
    #[serde(default)]
    pub state: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct JobDetail {
    pub job: ActivityJob,
    pub stages: Vec<JobStage>,
    pub private_details_available: bool,
    pub activity_actions_available: bool,
    #[serde(default)]
    pub maintenance_result: Option<crate::maintenance_result::MaintenanceResult>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct LogEntry {
    pub ts: String,
    pub severity: String,
    pub message: String,
    pub job_id: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct JobLogs {
    pub rows: Vec<LogEntry>,
    pub available: bool,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ReadKind {
    Articles,
    Tools,
    Models,
    Profile,
    Jobs,
    JobDetail,
    Logs,
}

impl ReadKind {
    pub fn is_activity(self) -> bool {
        matches!(self, Self::Jobs | Self::JobDetail | Self::Logs)
    }
}

#[derive(Clone, Debug)]
pub enum ReadRequest {
    Articles(ArticleQuery),
    Tools,
    Models,
    Profile,
    Jobs {},
    JobDetail { id: String },
    Logs { id: String },
}

impl ReadRequest {
    pub fn kind(&self) -> ReadKind {
        match self {
            Self::Articles(_) => ReadKind::Articles,
            Self::Tools => ReadKind::Tools,
            Self::Models => ReadKind::Models,
            Self::Profile => ReadKind::Profile,
            Self::Jobs { .. } => ReadKind::Jobs,
            Self::JobDetail { .. } => ReadKind::JobDetail,
            Self::Logs { .. } => ReadKind::Logs,
        }
    }
}

pub(crate) enum ReadResult {
    Articles(ArticlePage),
    Tools(ToolCatalog),
    Models(ModelDefaults),
    Profile(ProfileSnapshot),
    JobDetail(Box<JobDetail>),
    Logs(JobLogs),
}

pub struct Backend {
    pub article_day: TextEdit,
    pub article_category: TextEdit,
    pub article_search: TextEdit,
    pub articles: Load<ArticlePage>,
    pub tools: Load<ToolCatalog>,
    pub selected_tool: Option<String>,
    pub tool_inputs: TextEdit,
    pub models: Load<ModelDefaults>,
    pub profile: Load<ProfileSnapshot>,
    pub jobs: Load<ActivityJobs>,
    pub selected_job: Option<String>,
    pub detail: Load<JobDetail>,
    pub logs: Load<JobLogs>,
    pub selected_workspace_only: bool,
    pub message: Option<String>,
    pub(crate) revisions: BTreeMap<ReadKind, u64>,
    requests: BTreeMap<ReadKind, RequestTask>,
}

impl Default for Backend {
    fn default() -> Self {
        Self {
            article_day: TextEdit::new(""),
            article_category: TextEdit::new("all"),
            article_search: TextEdit::new(""),
            articles: Load::Idle,
            tools: Load::Idle,
            selected_tool: None,
            tool_inputs: TextEdit::new("{}"),
            models: Load::Idle,
            profile: Load::Idle,
            jobs: Load::Idle,
            selected_job: None,
            detail: Load::Idle,
            logs: Load::Idle,
            selected_workspace_only: false,
            message: None,
            revisions: BTreeMap::new(),
            requests: BTreeMap::new(),
        }
    }
}

pub(crate) fn job_is_active(state: &str) -> bool {
    matches!(
        state.to_ascii_lowercase().as_str(),
        "queued" | "pending" | "running" | "active" | "waiting" | "reconciling"
    )
}

impl Backend {
    pub(crate) fn busy(&self) -> bool {
        !self.requests.is_empty()
    }

    pub(crate) fn in_flight(&self, kind: ReadKind) -> bool {
        self.requests.contains_key(&kind)
    }

    pub(crate) fn cancel_reads(&mut self) {
        self.requests.clear();
    }

    pub(crate) fn invalidate_activity(&mut self) {
        self.requests.retain(|kind, _| !kind.is_activity());
        self.jobs = Load::Idle;
        self.detail = Load::Idle;
        self.logs = Load::Idle;
        self.selected_job = None;
        for kind in [ReadKind::Jobs, ReadKind::JobDetail, ReadKind::Logs] {
            let revision = self.revisions.entry(kind).or_default();
            *revision = revision.wrapping_add(1);
        }
    }

    fn loading(&mut self, kind: ReadKind) {
        match kind {
            ReadKind::Articles => self.articles = Load::Loading,
            ReadKind::Tools if matches!(self.tools, Load::Ready(_)) => {}
            ReadKind::Tools => self.tools = Load::Loading,
            ReadKind::Models if matches!(self.models, Load::Ready(_)) => {}
            ReadKind::Models => self.models = Load::Loading,
            ReadKind::Profile if matches!(self.profile, Load::Ready(_)) => {}
            ReadKind::Profile => self.profile = Load::Loading,
            ReadKind::Jobs if matches!(self.jobs, Load::Ready(_)) => {}
            ReadKind::Jobs => self.jobs = Load::Loading,
            ReadKind::JobDetail
                if matches!(
                    (&self.detail, &self.selected_job),
                    (Load::Ready(detail), Some(id)) if &detail.job.id == id
                ) => {}
            ReadKind::JobDetail => self.detail = Load::Loading,
            ReadKind::Logs => self.logs = Load::Loading,
        }
    }

    fn failed(&mut self, kind: ReadKind, error: String) {
        match kind {
            ReadKind::Articles => self.articles = Load::Failed(error),
            ReadKind::Tools => self.tools = Load::Failed(error),
            ReadKind::Models => self.models = Load::Failed(error),
            ReadKind::Profile => self.profile = Load::Failed(error),
            ReadKind::Jobs => self.jobs = Load::Failed(error),
            ReadKind::JobDetail
                if matches!(
                    (&self.detail, &self.selected_job),
                    (Load::Ready(detail), Some(id)) if &detail.job.id == id
                ) =>
            {
                self.message = Some(error);
            }
            ReadKind::JobDetail => self.detail = Load::Failed(error),
            ReadKind::Logs => self.logs = Load::Failed(error),
        }
    }
}

impl State {
    pub fn configuration_revision(&self) -> Option<u64> {
        let models = match &self.backend.models {
            Load::Ready(models) => Some(models.configuration_revision),
            _ => None,
        };
        let profile = match &self.backend.profile {
            Load::Ready(profile) => Some(profile.configuration_revision),
            _ => None,
        };
        models.into_iter().chain(profile).max()
    }

    pub fn refresh_page(&mut self) {
        match self.page {
            Page::Atlas => {
                self.backend_read(ReadRequest::Models);
                self.refresh_cycles();
            }
            Page::Seed => {
                self.refresh_readiness();
                self.backend_read(ReadRequest::Models);
                self.backend_read(ReadRequest::Profile);
            }
            Page::Tools => {
                self.backend_read(ReadRequest::Tools);
                self.backend_read(ReadRequest::Profile);
            }
            Page::Models => {
                self.backend_read(ReadRequest::Models);
                self.backend_read(ReadRequest::Profile);
            }
            Page::Profile => self.backend_read(ReadRequest::Profile),
            Page::Jobs => self.backend_read(ReadRequest::Jobs {}),
            Page::Workstation => {
                self.refresh_readiness();
                self.refresh_host_metrics();
            }
            _ => {}
        }
    }

    pub fn next_articles(&mut self) {
        let Load::Ready(page) = &self.backend.articles else {
            return;
        };
        let Some(cursor) = &page.next_cursor else {
            return;
        };
        self.backend_read(ReadRequest::Articles(ArticleQuery {
            day: self.backend.article_day.text.clone(),
            category: self.backend.article_category.text.clone(),
            search: self.backend.article_search.text.clone(),
            sort: "latest",
            limit: 30,
            cursor: Some(cursor.clone()),
        }));
    }

    pub fn select_job(&mut self, id: &str) {
        self.select_activity_tab(crate::logs::ActivityTab::Jobs);
        let switching = self.backend.selected_job.as_deref() != Some(id);
        if switching {
            self.job_actions.confirmation = None;
        }
        let Load::Ready(jobs) = &self.backend.jobs else {
            return;
        };
        if !jobs.rows.iter().any(|job| job.id == id) {
            return;
        }
        if switching {
            self.backend.detail = Load::Idle;
        }
        self.backend.selected_job = Some(id.to_owned());
        self.selected_log_job_changed(id);
        self.backend.logs = Load::Idle;
        self.backend_read(ReadRequest::JobDetail { id: id.to_owned() });
    }

    /// Cross-page links still resolve through the current registered node; the
    /// gateway may deny a job that is outside this workstation's associations.
    pub fn open_job(&mut self, id: &str) {
        let switching = self.backend.selected_job.as_deref() != Some(id);
        if switching {
            self.job_actions.confirmation = None;
            self.backend.detail = Load::Idle;
        }
        if self.registration.context.is_none() {
            return;
        }
        self.page = Page::Jobs;
        self.select_activity_tab(crate::logs::ActivityTab::Jobs);
        self.backend_read(ReadRequest::Jobs {});
        self.backend.selected_job = Some(id.to_owned());
        self.selected_log_job_changed(id);
        self.backend.logs = Load::Idle;
        self.backend_read(ReadRequest::JobDetail { id: id.to_owned() });
    }

    pub fn refresh_job_logs(&mut self) {
        let Some(id) = &self.backend.selected_job else {
            return;
        };
        self.backend_read(ReadRequest::Logs { id: id.clone() });
    }

    /// Legacy fixture entrypoint; current activity is always workstation-scoped.
    pub fn set_activity_workspace_filter(&mut self, _narrow: bool) {
        self.backend.selected_workspace_only = false;
        self.scoped_logs
            .select_target(crate::logs::LogTarget::Workstation);
        self.backend.invalidate_activity();
        self.refresh_page();
    }

    pub fn backend_read(&mut self, request: ReadRequest) {
        if matches!(request, ReadRequest::Jobs { .. }) {
            self.refresh_scoped_jobs();
            return;
        }
        let kind = request.kind();
        let permission = if kind.is_activity() {
            Permission::ReadWorkstationActivity
        } else {
            Permission::ReadBackend
        };
        if !self.permission(permission) {
            self.backend
                .failed(kind, "Connect with permission for this read".into());
            return;
        }
        if kind.is_activity() && self.registration.context.is_none() {
            self.backend.failed(
                kind,
                "Register this workstation before loading its activity".into(),
            );
            return;
        }
        self.record(Intent::BackendRead { kind });
        if self.fixture {
            return;
        }
        let Some(client) = &self.client else { return };
        let client = if kind.is_activity() {
            let Some(context) = &self.registration.context else {
                return;
            };
            match client.with_registered_context(context) {
                Ok(client) => client,
                Err(error) => {
                    self.backend.failed(kind, error.to_string());
                    return;
                }
            }
        } else {
            client.clone()
        };
        self.backend.loading(kind);
        let revision = self.backend.revisions.entry(kind).or_default();
        *revision = revision.wrapping_add(1);
        let revision = *revision;
        self.backend.requests.remove(&kind);
        if let Some(services) = &self.services {
            if let Some(task) = services.backend_read(
                client,
                request,
                self.connection.epoch,
                self.origin_revision,
                revision,
            ) {
                self.backend.requests.insert(kind, task);
            }
        }
    }

    pub(crate) fn complete_backend_read(
        &mut self,
        kind: ReadKind,
        revision: u64,
        result: Result<ReadResult, String>,
    ) {
        if self.backend.revisions.get(&kind) != Some(&revision) {
            return;
        }
        self.backend.requests.remove(&kind);
        let previous_configuration_revision = self.configuration_revision();
        match result {
            Ok(ReadResult::Articles(page)) => self.backend.articles = Load::Ready(page),
            Ok(ReadResult::Tools(catalog)) => self.backend.tools = Load::Ready(catalog),
            Ok(ReadResult::Models(defaults)) => self.backend.models = Load::Ready(defaults),
            Ok(ReadResult::Profile(profile)) => {
                self.backend.profile = if profile.configuration_format == 1
                    && profile.portable_schema_version == 4
                {
                    Load::Ready(profile)
                } else {
                    Load::Failed(
                        "Gateway profile requires the current explicit configuration format".into(),
                    )
                }
            }
            Ok(ReadResult::JobDetail(detail)) => {
                if self.backend.selected_job.as_deref() == Some(&detail.job.id) {
                    let transitioned = match &self.backend.detail {
                        Load::Ready(previous) if previous.job.id == detail.job.id => {
                            previous.job.state != detail.job.state
                                || previous.job.phase != detail.job.phase
                                || previous.job.progress_done != detail.job.progress_done
                                || previous.job.progress_total != detail.job.progress_total
                                || previous.job.error_summary != detail.job.error_summary
                        }
                        _ => false,
                    };
                    if let Load::Ready(jobs) = &mut self.backend.jobs {
                        if let Some(row) = jobs.rows.iter_mut().find(|row| row.id == detail.job.id)
                        {
                            *row = detail.job.clone();
                        }
                    }
                    if let Some(page) = &mut self.scoped_jobs.page {
                        if let Some(row) = page.rows.iter_mut().find(|row| row.id == detail.job.id)
                        {
                            *row = detail.job.clone();
                        }
                    }
                    self.backend.detail = Load::Ready(*detail);
                    self.sync_operational_logs_with_selected_job(transitioned);
                } else {
                    self.backend
                        .failed(kind, "Gateway returned a different job".into());
                }
            }
            Ok(ReadResult::Logs(logs)) => {
                if logs
                    .rows
                    .iter()
                    .all(|row| Some(row.job_id.as_str()) == self.backend.selected_job.as_deref())
                {
                    self.backend.logs = Load::Ready(logs);
                } else {
                    self.backend
                        .failed(kind, "Gateway returned logs for a different job".into());
                }
            }
            Err(error) => self.backend.failed(kind, error),
        }
        if self.configuration_revision() != previous_configuration_revision {
            self.classifier_activation.invalidate_confirmation();
            self.submissions.invalidate_confirmations();
            self.maintenance.confirmation = None;
            self.configuration.confirmation = None;
            self.transfer.confirmation = None;
        }
    }
}

impl GatewayEffects {
    fn backend_read(
        &self,
        client: Client,
        request: ReadRequest,
        epoch: u64,
        origin_revision: u64,
        revision: u64,
    ) -> Option<RequestTask> {
        let ui = self.ui.get()?.clone();
        let kind = request.kind();
        let task = self.runtime.spawn(async move {
            let result = read(&client, request)
                .await
                .map_err(|error| error.to_string());
            ui.update(move |state, _| {
                if state.connection.epoch == epoch
                    && (!kind.is_activity() || state.origin_revision == origin_revision)
                {
                    state.complete_backend_read(kind, revision, result);
                }
            });
        });
        Some(RequestTask(task.abort_handle()))
    }
}

async fn read(
    client: &Client,
    request: ReadRequest,
) -> Result<ReadResult, fleetworks_contracts::ClientError> {
    #[derive(Serialize)]
    struct ActivityQuery<'a> {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<&'a str>,
        limit: u16,
    }
    match request {
        ReadRequest::Articles(query) => client
            .get_with_query("/api/v1/articles", &query)
            .await
            .map(ReadResult::Articles),
        ReadRequest::Tools => client.get("/api/v1/tools").await.map(ReadResult::Tools),
        ReadRequest::Models => client
            .get("/api/v1/models/defaults")
            .await
            .map(ReadResult::Models),
        ReadRequest::Profile => client.get("/api/v1/profile").await.map(ReadResult::Profile),
        // Lists use the registered Jobs controller's explicit newest/cursor
        // contract; this generic detail adapter cannot issue a legacy list.
        ReadRequest::Jobs { .. } => Err(fleetworks_contracts::ClientError::InvalidRequest),
        ReadRequest::JobDetail { id } => client
            .get_with_query(
                "/api/v1/jobs/detail",
                &ActivityQuery {
                    id: Some(&id),
                    limit: 100,
                },
            )
            .await
            .map(ReadResult::JobDetail),
        ReadRequest::Logs { id } => client
            .get_with_query(
                "/api/v1/logs",
                &ActivityQuery {
                    id: Some(&id),
                    limit: 100,
                },
            )
            .await
            .map(ReadResult::Logs),
    }
}
