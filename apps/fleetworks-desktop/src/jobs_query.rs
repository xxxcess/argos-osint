//! Applied Jobs filters and bounded, continuous cursor loading.
//!
//! The first page is the head and is the only part a background poll refreshes. Later pages are
//! appended at the tail through the server's opaque cursor, so rows, selection and scroll
//! position survive both fetches and refreshes. Work is bounded by [`HISTORY_LIMIT`] pages and
//! [`MAX_ROWS`] rows.
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
/// Most pages one scope will load: the head plus appended tail pages.
pub const HISTORY_LIMIT: usize = 64;
/// Most rows kept for one applied query, so memory and the list stay bounded.
pub const MAX_ROWS: usize = 500;

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

impl JobsDashboardCounts {
    /// Server-authoritative unfiltered jobs currently in progress
    /// (`running + queued + retry_scheduled`).
    #[must_use]
    pub fn in_progress(&self) -> u64 {
        self.active
            .saturating_add(self.queued)
            .saturating_add(self.retrying)
    }
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

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum JobsEvent {
    Apply,
    Clear,
    /// Append the next page at the tail.
    More,
    Poll,
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
    is_poll: bool,
    is_tail: bool,
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
    /// Rows appended after the head page, oldest page last.
    more: Vec<ActivityJob>,
    /// Opaque cursor for the next tail page, when the server has more.
    tail_cursor: Option<String>,
    /// Tail pages appended so far.
    tail_pages: usize,
    /// A tail fetch failed; every loaded row stays and the user can retry.
    tail_error: Option<String>,
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
    /// Number of reads begun so far, polls included.
    #[cfg(test)]
    pub(crate) fn reads_begun(&self) -> u64 {
        self.sequence
    }
    pub fn revision(&self) -> u64 {
        self.generation
    }
    fn row_iter(&self) -> impl Iterator<Item = &ActivityJob> {
        let mut seen = std::collections::HashSet::new();
        self.page
            .iter()
            .flat_map(|page| page.rows.iter())
            .chain(self.more.iter())
            .filter(move |row| seen.insert(row.id.as_str()))
            .take(MAX_ROWS)
    }
    /// Head rows followed by appended rows, without duplicates, at most [`MAX_ROWS`].
    pub fn rows(&self) -> Vec<ActivityJob> {
        self.row_iter().cloned().collect()
    }
    /// How many rows [`rows`](Self::rows) holds, without copying them.
    pub fn row_count(&self) -> usize {
        self.row_iter().count()
    }
    /// Pages loaded for the applied query: the head plus the tail.
    pub fn loaded_pages(&self) -> usize {
        usize::from(self.page.is_some()) + self.tail_pages
    }
    /// Whether the user can ask for more rows right now.
    pub fn can_load_more(&self) -> bool {
        self.status == JobsLoad::Ready
            && !self.busy()
            && !self.dirty
            && self.tail_cursor.is_some()
            && self.loaded_pages() < HISTORY_LIMIT
            && self.row_count() < MAX_ROWS
    }
    /// A tail fetch is in flight; the list stays as it is.
    pub fn loading_more(&self) -> bool {
        self.active.as_ref().is_some_and(|ticket| ticket.is_tail)
    }
    /// The last tail fetch failed, with its short reason.
    pub fn tail_error(&self) -> Option<&str> {
        self.tail_error.as_deref()
    }
    /// The server has more rows but this scope has reached its bound.
    pub fn at_bound(&self) -> bool {
        self.tail_cursor.is_some()
            && (self.loaded_pages() >= HISTORY_LIMIT || self.row_count() >= MAX_ROWS)
    }
    /// Server-authoritative unfiltered count of jobs in progress, or `None` when unknown.
    #[must_use]
    pub fn in_progress_count(&self) -> Option<u64> {
        self.page
            .as_ref()
            .and_then(|page| page.counts.as_ref())
            .map(JobsDashboardCounts::in_progress)
    }

    pub fn origin_label(&self) -> Option<String> {
        self.binding
            .as_ref()
            .map(|binding| binding.context.connect_id.to_string())
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
                Some("Register this workstation with activity permission to view Jobs.".into());
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
                    // Registration fixes the workstation scope independently of filters.
                    self.draft = JobsDraft::default();
                }
                self.reset();
                let query = self.first_query(now);
                let read = match query {
                    Ok(query) => {
                        self.applied = Some(query.clone());
                        self.dirty = false;
                        self.begin_ticket(query, true, false)
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
            JobsEvent::More if self.can_load_more() => {
                // A tail fetch keeps every row, the selection and the scroll position.
                let read = self
                    .tail_cursor
                    .clone()
                    .and_then(|cursor| self.begin_tail(cursor));
                JobsEffect {
                    read,
                    clear_selection: false,
                }
            }
            JobsEvent::Poll => {
                if self.loading_more() {
                    // Wait for an active tail continuation rather than cancelling it.
                    JobsEffect::default()
                } else if self.page.is_none() {
                    let query = self.first_query(now);
                    let read = match query {
                        Ok(query) => {
                            self.applied = Some(query.clone());
                            self.dirty = false;
                            self.begin_ticket(query, true, false)
                        }
                        Err(message) => {
                            self.status = JobsLoad::Failed;
                            self.message = Some(message);
                            None
                        }
                    };
                    JobsEffect {
                        read,
                        clear_selection: false,
                    }
                } else {
                    let query = self.applied.clone().or_else(|| self.first_query(now).ok());
                    let read = query.and_then(|q| self.begin_ticket(q, false, true));
                    JobsEffect {
                        read,
                        clear_selection: false,
                    }
                }
            }
            _ => JobsEffect::default(),
        }
    }

    fn first_query(&self, now: DateTime<Utc>) -> Result<JobsQuery, String> {
        if self.binding.is_none() {
            return Err("Register this workstation with activity permission first.".into());
        }
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
            app: field(&self.draft.app)?,
            operation: field(&self.draft.operation)?,
            kind: field(&self.draft.kind)?,
            correlation_id: field(&self.draft.correlation_id)?,
            text: field(&self.draft.text)?,
            since,
            ..Default::default()
        })
    }

    fn begin_tail(&mut self, cursor: String) -> Option<JobsTicket> {
        let mut ticket = self.begin_ticket(
            JobsQuery {
                cursor: Some(cursor),
                ..Default::default()
            },
            false,
            false,
        )?;
        ticket.is_tail = true;
        self.active = Some(ticket.clone());
        self.tail_error = None;
        Some(ticket)
    }

    fn begin_ticket(
        &mut self,
        query: JobsQuery,
        clear_page: bool,
        is_poll: bool,
    ) -> Option<JobsTicket> {
        let binding = self.binding.clone()?;
        self.request = None;
        self.sequence = self.sequence.wrapping_add(1);
        let ticket = JobsTicket {
            binding,
            query,
            generation: self.generation,
            sequence: self.sequence,
            is_poll,
            is_tail: false,
        };
        self.active = Some(ticket.clone());
        if clear_page {
            self.page = None;
            self.message = None;
            self.status = JobsLoad::Loading;
        }
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
            Ok(page) if self.valid_page(&page, ticket) && ticket.is_tail => {
                // Append at the tail; the head page and its totals stay as observed.
                let room = MAX_ROWS.saturating_sub(self.row_count());
                self.more.extend(page.rows.into_iter().take(room));
                self.tail_cursor = page.next_cursor;
                self.tail_pages += 1;
                self.tail_error = None;
            }
            Ok(page) if self.valid_page(&page, ticket) => {
                // Ignore an older poll observation so newer head state is never regressed.
                if ticket.is_poll
                    && self.page.as_ref().is_some_and(|current| {
                        observed_older_than(&page.observed_at, &current.observed_at)
                    })
                {
                    return false;
                }
                // A head refresh keeps appended rows and reconciles overlapping IDs without
                // resetting the tail cursor. Before any tail exists it also refreshes the cursor.
                if self.tail_pages == 0 {
                    self.tail_cursor = page.next_cursor.clone();
                } else if !self.more.is_empty() {
                    let head_ids: std::collections::HashSet<&str> =
                        page.rows.iter().map(|row| row.id.as_str()).collect();
                    self.more.retain(|row| !head_ids.contains(row.id.as_str()));
                }
                self.page = Some(page);
                self.status = JobsLoad::Ready;
                self.message = None;
            }
            Ok(_) => {
                self.reset();
                self.status = JobsLoad::Failed;
                self.message =
                    Some("The gateway returned an invalid or different scoped Jobs page.".into());
            }
            Err(error)
                if ticket.is_tail && !matches!(error, ClientError::Status(401 | 403 | 409)) =>
            {
                self.tail_error = Some(error.to_string());
            }
            Err(error)
                if ticket.is_poll
                    && self.page.is_some()
                    && !matches!(error, ClientError::Status(401 | 403 | 409)) =>
            {
                self.status = JobsLoad::Ready;
                self.message = Some(error.to_string());
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
        page.activity_scope.deployment_id == context.deployment_id
            && page.activity_scope.workstation_id == context.workstation_id
            && page.activity_scope.scope_revision == context.scope_revision
            && page.rows.len() <= usize::from(PAGE_SIZE)
            && page.apps.len() <= 32
            && page.dashboard_counts_scope == "unfiltered_top_level_workstation_activity"
            && cursor_valid(&page.page_cursor)
            && page.next_cursor.as_deref().is_none_or(cursor_valid)
            && page.has_more == page.next_cursor.is_some()
            && DateTime::parse_from_rfc3339(&page.observed_at).is_ok()
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
        self.more.clear();
        self.tail_cursor = None;
        self.tail_pages = 0;
        self.tail_error = None;
        self.applied = None;
        self.status = JobsLoad::Idle;
        self.message = None;
    }
}

fn observed_older_than(candidate: &str, current: &str) -> bool {
    DateTime::parse_from_rfc3339(candidate)
        .ok()
        .zip(DateTime::parse_from_rfc3339(current).ok())
        .is_some_and(|(candidate, current)| candidate < current)
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
