//! Typed global read models and registered workstation activity.

use std::collections::BTreeMap;

use fleetworks_contracts::Client;
use fleetworks_contracts::context::ActivityScope;
use fleetworks_contracts::context::Permission;
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
    Jobs {
        workspace_id: Option<String>,
    },
    JobDetail {
        id: String,
        workspace_id: Option<String>,
    },
    Logs {
        id: String,
        workspace_id: Option<String>,
    },
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

enum ReadResult {
    Articles(ArticlePage),
    Tools(ToolCatalog),
    Models(ModelDefaults),
    Profile(ProfileSnapshot),
    Jobs(ActivityJobs),
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
    revisions: BTreeMap<ReadKind, u64>,
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

impl Backend {
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
            ReadKind::Tools => self.tools = Load::Loading,
            ReadKind::Models => self.models = Load::Loading,
            ReadKind::Profile => self.profile = Load::Loading,
            ReadKind::Jobs => self.jobs = Load::Loading,
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
        let request = match self.page {
            Page::Atlas => {
                let day = self.backend.article_day.text.trim();
                if day.len() != 10 || chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d").is_err() {
                    self.backend.articles =
                        Load::Failed("Enter a calendar day as YYYY-MM-DD".into());
                    return;
                }
                ReadRequest::Articles(ArticleQuery {
                    day: day.to_owned(),
                    category: self.backend.article_category.text.clone(),
                    search: self.backend.article_search.text.clone(),
                    sort: "latest",
                    limit: 30,
                    cursor: None,
                })
            }
            Page::Tools => ReadRequest::Tools,
            Page::Models => ReadRequest::Models,
            Page::Profile => ReadRequest::Profile,
            Page::Jobs => ReadRequest::Jobs {
                workspace_id: self.activity_workspace_filter(),
            },
            Page::Workstation => {
                self.refresh_readiness();
                self.backend_read(ReadRequest::Profile);
                return;
            }
            _ => return,
        };
        self.backend_read(request);
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
        if self.backend.selected_job.as_deref() != Some(id) {
            self.job_actions.confirmation = None;
        }
        let Load::Ready(jobs) = &self.backend.jobs else {
            return;
        };
        if !jobs.rows.iter().any(|job| job.id == id) {
            return;
        }
        self.backend.selected_job = Some(id.to_owned());
        self.backend.logs = Load::Idle;
        self.backend_read(ReadRequest::JobDetail {
            id: id.to_owned(),
            workspace_id: self.activity_workspace_filter(),
        });
    }

    /// Cross-page links still resolve through the current registered node; the
    /// gateway may deny a job that is outside this workstation's associations.
    pub fn open_job(&mut self, id: &str) {
        if self.backend.selected_job.as_deref() != Some(id) {
            self.job_actions.confirmation = None;
        }
        if self.registration.context.is_none() {
            return;
        }
        self.page = Page::Jobs;
        self.backend.selected_job = Some(id.to_owned());
        self.backend.logs = Load::Idle;
        self.backend_read(ReadRequest::Jobs {
            workspace_id: self.activity_workspace_filter(),
        });
        self.backend_read(ReadRequest::JobDetail {
            id: id.to_owned(),
            workspace_id: self.activity_workspace_filter(),
        });
    }

    pub fn refresh_job_logs(&mut self) {
        let Some(id) = &self.backend.selected_job else {
            return;
        };
        self.backend_read(ReadRequest::Logs {
            id: id.clone(),
            workspace_id: self.activity_workspace_filter(),
        });
    }

    pub fn set_activity_workspace_filter(&mut self, narrow: bool) {
        self.backend.selected_workspace_only = narrow;
        self.backend.invalidate_activity();
        self.refresh_page();
    }

    fn activity_workspace_filter(&self) -> Option<String> {
        self.backend
            .selected_workspace_only
            .then(|| {
                self.registration
                    .context
                    .as_ref()
                    .map(|context| context.workspace_id.clone())
            })
            .flatten()
    }

    pub fn backend_read(&mut self, request: ReadRequest) {
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
                "Register an owned workspace before loading workstation activity".into(),
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

    fn complete_backend_read(
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
            Ok(ReadResult::Jobs(jobs)) => {
                let valid = self.registration.context.as_ref().is_some_and(|context| {
                    jobs.activity_scope.deployment_id == context.deployment_id
                        && jobs.activity_scope.workstation_id == context.workstation_id
                        && jobs.activity_scope.scope_revision == context.scope_revision
                        && jobs.activity_scope.workspace_id == self.activity_workspace_filter()
                });
                if valid {
                    self.backend.jobs = Load::Ready(jobs);
                } else {
                    self.backend.failed(
                        kind,
                        "Gateway returned activity for a different registered scope".into(),
                    );
                }
            }
            Ok(ReadResult::JobDetail(detail)) => {
                if self.backend.selected_job.as_deref() == Some(&detail.job.id) {
                    self.backend.detail = Load::Ready(*detail);
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
        #[serde(skip_serializing_if = "Option::is_none")]
        workspace_id: Option<&'a str>,
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
        ReadRequest::Jobs { workspace_id } => client
            .get_with_query(
                "/api/v1/jobs",
                &ActivityQuery {
                    id: None,
                    workspace_id: workspace_id.as_deref(),
                    limit: 100,
                },
            )
            .await
            .map(ReadResult::Jobs),
        ReadRequest::JobDetail { id, workspace_id } => client
            .get_with_query(
                "/api/v1/jobs/detail",
                &ActivityQuery {
                    id: Some(&id),
                    workspace_id: workspace_id.as_deref(),
                    limit: 100,
                },
            )
            .await
            .map(ReadResult::JobDetail),
        ReadRequest::Logs { id, workspace_id } => client
            .get_with_query(
                "/api/v1/logs",
                &ActivityQuery {
                    id: Some(&id),
                    workspace_id: workspace_id.as_deref(),
                    limit: 100,
                },
            )
            .await
            .map(ReadResult::Logs),
    }
}
