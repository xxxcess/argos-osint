//! Applied Jobs filters and bounded server-owned page navigation.
//!
//! Draft edits never rewrite an in-flight request. Read cancellation affects
//! local reads only; accepted backend work and the separate Logs state survive.

use chrono::DateTime;
use chrono::Duration;
use chrono::SecondsFormat;
use chrono::Utc;
use fleetworks_contracts::context::ActivityScope;
use fleetworks_contracts::context::RegisteredContext;
use fleetworks_contracts::Client;
use fleetworks_contracts::ClientError;
use rune::TextEdit;
use serde::Deserialize;
use serde::Serialize;

use crate::backend::ActivityJob;
use crate::connection::RequestTask;

pub const PAGE_SIZE: u16 = 50;
pub const HISTORY_LIMIT: usize = 64;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JobsStatusFilter {
    #[default]
    All,
    Active,
    Retrying,
    Failed,
    Completed,
    Stopped,
}

impl JobsStatusFilter {
    pub const ALL: [Self; 6] = [
        Self::All,
        Self::Active,
        Self::Retrying,
        Self::Failed,
        Self::Completed,
        Self::Stopped,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Active => "Active",
            Self::Retrying => "Retrying",
            Self::Failed => "Failed",
            Self::Completed => "Completed",
            Self::Stopped => "Stopped",
        }
    }
    pub fn key(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Active => "active",
            Self::Retrying => "retrying",
            Self::Failed => "failed",
            Self::Completed => "completed",
            Self::Stopped => "stopped",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum JobsTime {
    #[default]
    Available,
    Hour,
    Day,
}

impl JobsTime {
    pub const ALL: [Self; 3] = [Self::Available, Self::Hour, Self::Day];
    pub fn label(self) -> &'static str {
        match self {
            Self::Available => "Available 72 hours",
            Self::Hour => "Last hour",
            Self::Day => "Last 24 hours",
        }
    }
    pub fn key(self) -> &'static str {
        match self {
            Self::Available => "available",
            Self::Hour => "hour",
            Self::Day => "day",
        }
    }
}

#[derive(Default)]
pub struct JobsDraft {
    pub status: JobsStatusFilter,
    pub app: TextEdit,
    pub operation: TextEdit,
    pub kind: TextEdit,
    pub correlation_id: TextEdit,
    pub text: TextEdit,
    pub time: JobsTime,
    pub selected_workspace_only: bool,
}

/// Native always requests the additive paged route behavior explicitly.
#[derive(Clone, Copy, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JobsSort {
    #[default]
    Newest,
}

/// Client-safe wire request. A continuation sends only sort and opaque cursor.
#[derive(Clone, Default, Serialize)]
pub struct JobsQuery {
    pub sort: JobsSort,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<JobsStatusFilter>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
}

#[derive(Clone, Deserialize)]
pub struct JobsDashboardCounts {
    pub active: u64,
    pub queued: u64,
    pub retrying: u64,
    pub failed: u64,
    pub completed_recent: u64,
}

/// Match totals are a current observation, not an immutable snapshot or the
/// sum of the legacy scope-wide dashboard buckets.
#[derive(Clone, Deserialize)]
pub struct JobsPage {
    pub rows: Vec<ActivityJob>,
    pub activity_scope: ActivityScope,
    pub relation_counts_available: bool,
    pub activity_actions_available: bool,
    pub counts: Option<JobsDashboardCounts>,
    pub dashboard_counts_scope: String,
    pub matched_total: Option<u64>,
    pub observed_at: String,
    pub apps: Vec<String>,
    pub apps_complete: bool,
    pub has_more: bool,
    pub page_cursor: String,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JobsBinding {
    pub context: RegisteredContext,
    pub connection_epoch: u64,
    pub origin_revision: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum JobsLoad {
    #[default]
    Idle,
    Loading,
    Ready,
    Failed,
    RestartRequired,
}

#[derive(Clone, Copy)]
pub enum JobsEvent {
    Apply,
    Clear,
    Next,
    Previous,
}

/// The adapter clears the selected job/detail/action preview before issuing a
/// changed query or page. No durable job cancellation is part of this effect.
#[derive(Default)]
pub struct JobsEffect {
    pub read: Option<JobsTicket>,
    pub clear_selection: bool,
}

#[derive(Clone)]
pub struct JobsTicket {
    pub binding: JobsBinding,
    pub query: JobsQuery,
    generation: u64,
    sequence: u64,
    page_index: usize,
}

#[derive(Default)]
pub struct JobsQueryState {
    pub draft: JobsDraft,
    pub status: JobsLoad,
    pub page: Option<JobsPage>,
    pub message: Option<String>,
    pub dirty: bool,
    binding: Option<JobsBinding>,
    blocked_binding: Option<JobsBinding>,
    applied: Option<JobsQuery>,
    history: Vec<String>,
    page_index: usize,
    generation: u64,
    sequence: u64,
    active: Option<JobsTicket>,
    pub(crate) request: Option<RequestTask>,
}

impl JobsQueryState {
    pub fn connected(&self) -> bool {
        self.binding.is_some()
    }
    pub fn busy(&self) -> bool {
        self.active.is_some()
    }
    pub fn revision(&self) -> u64 {
        self.generation
    }
    pub fn page_number(&self) -> usize {
        self.page_index + 1
    }
    pub fn origin_label(&self) -> Option<String> {
        self.binding
            .as_ref()
            .map(|binding| binding.context.connect_id.to_string())
    }
    pub fn can_next(&self) -> bool {
        self.status == JobsLoad::Ready
            && !self.busy()
            && !self.dirty
            && self.page_index + 1 < HISTORY_LIMIT
            && self.page.as_ref().is_some_and(|page| page.has_more)
    }
    pub fn can_previous(&self) -> bool {
        self.status == JobsLoad::Ready && !self.busy() && !self.dirty && self.page_index > 0
    }
    pub fn active_ticket(&self) -> Option<&JobsTicket> {
        self.active.as_ref()
    }
    pub fn applied_query(&self) -> Option<&JobsQuery> {
        self.applied.as_ref()
    }

    /// Call on connection, registration, activity permission and origin changes.
    pub fn bind(&mut self, binding: Option<JobsBinding>, permitted: bool) {
        let binding =
            binding.filter(|binding| permitted && self.blocked_binding.as_ref() != Some(binding));
        if self.binding != binding {
            self.reset();
            self.draft = JobsDraft::default();
            self.dirty = false;
            self.binding = binding;
        }
        if self.binding.is_none() && self.message.is_none() {
            self.message =
                Some("Register a workspace with activity permission to view Jobs.".into());
        }
    }

    pub fn invalidate_origin(&mut self) {
        self.reset();
        self.binding = None;
        self.draft = JobsDraft::default();
        self.dirty = false;
    }

    pub fn cancel_reads(&mut self) {
        self.request = None;
        self.active = None;
        self.generation = self.generation.wrapping_add(1);
        if self.status == JobsLoad::Loading {
            self.status = JobsLoad::Idle;
        }
    }

    pub fn draft_edited(&mut self) {
        self.dirty = true;
    }

    pub fn handle(&mut self, event: JobsEvent, now: DateTime<Utc>) -> JobsEffect {
        match event {
            JobsEvent::Apply | JobsEvent::Clear => {
                if matches!(event, JobsEvent::Clear) {
                    // Clearing filters must not widen an explicitly narrowed scope.
                    self.draft = JobsDraft {
                        selected_workspace_only: self.draft.selected_workspace_only,
                        ..Default::default()
                    };
                }
                self.reset();
                let query = self.first_query(now);
                let read = match query {
                    Ok(query) => {
                        self.applied = Some(query.clone());
                        self.dirty = false;
                        self.begin(query, 0)
                    }
                    Err(message) => {
                        self.status = JobsLoad::Failed;
                        self.message = Some(message);
                        None
                    }
                };
                JobsEffect {
                    read,
                    clear_selection: true,
                }
            }
            JobsEvent::Next if self.can_next() => {
                let cursor = self.page.as_ref().and_then(|page| page.next_cursor.clone());
                JobsEffect {
                    read: cursor.and_then(|cursor| self.begin_cursor(cursor, self.page_index + 1)),
                    clear_selection: true,
                }
            }
            JobsEvent::Previous if self.can_previous() => {
                let index = self.page_index - 1;
                JobsEffect {
                    read: self
                        .history
                        .get(index)
                        .cloned()
                        .and_then(|cursor| self.begin_cursor(cursor, index)),
                    clear_selection: true,
                }
            }
            _ => JobsEffect::default(),
        }
    }

    fn first_query(&self, now: DateTime<Utc>) -> Result<JobsQuery, String> {
        let Some(binding) = &self.binding else {
            return Err("Register a workspace with activity permission first.".into());
        };
        let field = |edit: &TextEdit| -> Result<Option<String>, String> {
            let value = edit.text.trim();
            if edit.text.len() > 256 || edit.text.chars().any(char::is_control) {
                return Err(
                    "Filters must be at most 256 bytes and contain no control characters.".into(),
                );
            }
            Ok((!value.is_empty()).then(|| value.to_owned()))
        };
        let since = match self.draft.time {
            JobsTime::Available => None,
            JobsTime::Hour => Some(now - Duration::hours(1)),
            JobsTime::Day => Some(now - Duration::hours(24)),
        }
        .map(|time| time.to_rfc3339_opts(SecondsFormat::Secs, true));
        Ok(JobsQuery {
            limit: Some(PAGE_SIZE),
            status: Some(self.draft.status),
            workspace_id: self
                .draft
                .selected_workspace_only
                .then(|| binding.context.workspace_id.clone()),
            app: field(&self.draft.app)?,
            operation: field(&self.draft.operation)?,
            kind: field(&self.draft.kind)?,
            correlation_id: field(&self.draft.correlation_id)?,
            text: field(&self.draft.text)?,
            since,
            ..Default::default()
        })
    }

    fn begin_cursor(&mut self, cursor: String, index: usize) -> Option<JobsTicket> {
        self.begin(
            JobsQuery {
                cursor: Some(cursor),
                ..Default::default()
            },
            index,
        )
    }

    fn begin(&mut self, query: JobsQuery, index: usize) -> Option<JobsTicket> {
        let binding = self.binding.clone()?;
        self.request = None;
        self.sequence = self.sequence.wrapping_add(1);
        let ticket = JobsTicket {
            binding,
            query,
            generation: self.generation,
            sequence: self.sequence,
            page_index: index,
        };
        self.active = Some(ticket.clone());
        self.page = None;
        self.message = None;
        self.status = JobsLoad::Loading;
        Some(ticket)
    }

    /// Ignore stale completions without changing a newer query or context.
    pub fn complete(&mut self, ticket: &JobsTicket, result: Result<JobsPage, ClientError>) -> bool {
        if self.binding.as_ref() != Some(&ticket.binding)
            || self.generation != ticket.generation
            || self
                .active
                .as_ref()
                .is_none_or(|active| active.sequence != ticket.sequence)
        {
            return false;
        }
        self.active = None;
        self.request = None;
        match result {
            Ok(page) if self.valid_page(&page, ticket) => {
                self.history.truncate(ticket.page_index + 1);
                if ticket.page_index == self.history.len() {
                    self.history.push(page.page_cursor.clone());
                } else {
                    self.history[ticket.page_index] = page.page_cursor.clone();
                }
                self.page_index = ticket.page_index;
                self.page = Some(page);
                self.status = JobsLoad::Ready;
            }
            Ok(_) => {
                self.reset();
                self.status = JobsLoad::Failed;
                self.message =
                    Some("The gateway returned an invalid or different scoped Jobs page.".into());
            }
            Err(error) => {
                self.reset();
                self.status = match error {
                    ClientError::Status(409) => JobsLoad::RestartRequired,
                    _ => JobsLoad::Failed,
                };
                if matches!(error, ClientError::Status(401 | 403)) {
                    self.blocked_binding = self.binding.take();
                    self.message =
                        Some("Jobs access changed. Reconnect with current credentials.".into());
                } else if self.status == JobsLoad::RestartRequired {
                    self.message = Some(
                        "This page expired or its scope changed. Apply filters to restart Jobs."
                            .into(),
                    );
                } else {
                    self.message = Some(error.to_string());
                }
            }
        }
        true
    }

    fn valid_page(&self, page: &JobsPage, ticket: &JobsTicket) -> bool {
        let context = &ticket.binding.context;
        let workspace = self
            .applied
            .as_ref()
            .and_then(|query| query.workspace_id.as_ref());
        page.activity_scope.deployment_id == context.deployment_id
            && page.activity_scope.workstation_id == context.workstation_id
            && page.activity_scope.scope_revision == context.scope_revision
            && page.activity_scope.workspace_id.as_ref() == workspace
            && page.rows.len() <= usize::from(PAGE_SIZE)
            && page.apps.len() <= 32
            && page.dashboard_counts_scope == "unfiltered_top_level_workstation_activity"
            && cursor_valid(&page.page_cursor)
            && page.next_cursor.as_deref().is_none_or(cursor_valid)
            && page.has_more == page.next_cursor.is_some()
            && DateTime::parse_from_rfc3339(&page.observed_at).is_ok()
            && ticket.page_index < HISTORY_LIMIT
            && ticket.page_index <= self.history.len()
            && page.rows.iter().all(|row| {
                row.private_details_available != row.status_only
                    && (!(row.can_cancel || row.can_retry)
                        || row.private_details_available && row.activity_actions_available)
                    && (!row.status_only
                        || row.title.is_empty()
                            && row.error_summary.is_empty()
                            && row.phase.is_empty()
                            && row.progress_done.is_none()
                            && row.progress_total.is_none()
                            && !row.activity_actions_available)
            })
    }

    fn reset(&mut self) {
        self.cancel_reads();
        self.page = None;
        self.history.clear();
        self.page_index = 0;
        self.applied = None;
        self.status = JobsLoad::Idle;
        self.message = None;
    }
}

fn cursor_valid(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Use the shared origin-bound SDK and only the ticket's registered proof.
pub async fn read(client: &Client, ticket: &JobsTicket) -> Result<JobsPage, ClientError> {
    client
        .with_registered_context(&ticket.binding.context)?
        .get_with_query("/api/v1/jobs", &ticket.query)
        .await
}

#[cfg(test)]
#[path = "jobs_query_tests.rs"]
mod tests;
