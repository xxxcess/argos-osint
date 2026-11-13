//! Registered workstation log reads. Local cancellation only stops read effects;
//! it never cancels accepted backend work. No global/embedded fallback exists.

use std::collections::BTreeSet;
use std::time::Duration;

use fleetworks_contracts::context::RegisteredContext;
use fleetworks_contracts::operational_logs::LogCounts;
use fleetworks_contracts::operational_logs::LogEntry;
use fleetworks_contracts::operational_logs::LogPage;
use fleetworks_contracts::operational_logs::LogQuery;
use fleetworks_contracts::Client;
use fleetworks_contracts::ClientError;
use rune::TextEdit;
use rune_kit::TableState;

use crate::connection::RequestTask;
/// Finite safe-log intents, independent of the retired toolbar presentation.
#[derive(Clone)]
pub enum LogEvent {
    Select(LogTarget),
    Refresh,
    Older,
    ToggleFollow,
    CopyLoaded,
    PrepareExport,
    ContinueExport,
    SaveExport { allow_partial: bool },
}

pub const DISPLAY_LIMIT: usize = 500;
const EXPORT_LIMIT: usize = 2 * 1024 * 1024;
const EXPORT_PAGES: usize = 16;
const PAGE_SIZE: usize = 100;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ActivityTab {
    #[default]
    Jobs,
    Logs,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LogBinding {
    pub context: RegisteredContext,
    pub connection_epoch: u64,
    pub origin_revision: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum LogTarget {
    #[default]
    Workstation,
    SelectedWorkspace,
    Job {
        id: String,
        descendants: bool,
    },
}

#[derive(Default)]
pub struct LogDraft {
    pub text: TextEdit,
    pub app: TextEdit,
    pub since: TextEdit,
    pub min_rank: Option<i64>,
    pub selected_workspace_only: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum LogStatus {
    #[default]
    Idle,
    Loading,
    Ready,
    Unavailable,
    RestartRequired,
    DisplayLimit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LogAction {
    Refresh,
    Older,
    Follow,
    Export,
}

#[derive(Clone)]
pub struct LogTicket {
    binding: LogBinding,
    generation: u64,
    sequence: u64,
    pub action: LogAction,
    pub query: LogQuery,
}

pub enum LogReply {
    Page(LogPage),
    Export {
        bytes: Vec<u8>,
        next_cursor: Option<String>,
        apps_complete: bool,
    },
}

/// Safe summaries only. This is private operational data and is deliberately
/// absent from Debug, trace intents and persistent connection profiles.
#[derive(Default)]
pub struct LogExport {
    bytes: Vec<u8>,
    rows: usize,
    pages: usize,
    next_cursor: Option<String>,
    started: bool,
    pub complete: bool,
    pub bounded: bool,
    pub apps_complete: bool,
}

impl LogExport {
    pub fn rows(&self) -> usize {
        self.rows
    }
    pub fn prepared(&self) -> bool {
        self.started && !self.bytes.is_empty()
    }
    pub fn has_more(&self) -> bool {
        self.next_cursor.is_some() && !self.bounded
    }

    /// A partial download is never silently presented as a complete export.
    /// The caller must show a separate explicit partial-save control.
    pub fn bytes_for_save(&self, allow_partial: bool) -> Option<&[u8]> {
        (self.started && (self.complete || allow_partial)).then_some(self.bytes.as_slice())
    }
}

/// A copy operation always says which loaded subset it contains, even if the
/// currently observed eligible private scope happens to fit on screen.
pub struct LogCopy {
    pub text: String,
    pub rows: usize,
    pub complete_eligible_scope: bool,
    pub excludes_shared_private: bool,
}

#[derive(Default)]
pub struct LogState {
    pub draft: LogDraft,
    pub target: LogTarget,
    pub status: LogStatus,
    pub rows: Vec<LogEntry>,
    pub table: TableState,
    pub counts: Option<LogCounts>,
    pub apps: Vec<String>,
    pub apps_complete: bool,
    pub shared_private_logs_available: bool,
    pub message: Option<String>,
    pub following: bool,
    pub visible: bool,
    pub window_focused: bool,
    pub export: LogExport,
    binding: Option<LogBinding>,
    blocked_binding: Option<LogBinding>,
    query: Option<LogQuery>,
    next_cursor: Option<String>,
    follow_cursor: Option<String>,
    available: bool,
    generation: u64,
    sequence: u64,
    active: Option<LogTicket>,
    pub(crate) request: Option<RequestTask>,
}

impl LogState {
    pub fn connected(&self) -> bool {
        self.binding.is_some()
    }
    pub fn busy(&self) -> bool {
        self.active.is_some() || self.request.is_some()
    }
    pub fn available(&self) -> bool {
        self.available
    }
    pub fn revision(&self) -> u64 {
        self.generation
    }
    pub fn request_revision(&self) -> u64 {
        self.sequence
    }
    pub fn active_ticket(&self) -> Option<&LogTicket> {
        self.active.as_ref()
    }
    pub fn can_copy_loaded(&self) -> bool {
        self.available && !self.busy() && !self.rows.is_empty()
    }
    pub fn can_load_older(&self) -> bool {
        self.available
            && !self.busy()
            && self.status != LogStatus::DisplayLimit
            && self.rows.len() < DISPLAY_LIMIT
            && self.next_cursor.is_some()
    }
    pub fn can_export(&self) -> bool {
        self.available && !self.busy()
    }
    pub fn can_follow(&self) -> bool {
        self.available
            && self.visible
            && !self.busy()
            && self.status != LogStatus::DisplayLimit
            && self.rows.len() < DISPLAY_LIMIT
            && self.follow_cursor.is_some()
    }
    pub fn origin_label(&self) -> Option<String> {
        self.binding
            .as_ref()
            .map(|binding| binding.context.connect_id.to_string())
    }

    /// Call on every connection, registration, workspace or permission update.
    /// Reusing an epoch after a 401/403 cannot resurrect the rejected binding.
    pub fn bind(&mut self, binding: Option<LogBinding>, permitted: bool) {
        let binding =
            binding.filter(|binding| permitted && self.blocked_binding.as_ref() != Some(binding));
        if self.binding != binding {
            self.reset();
            self.draft = LogDraft::default();
            self.target = LogTarget::Workstation;
            self.binding = binding;
            if self.binding.is_none() {
                self.message =
                    Some("Register this workstation with activity permission to view logs.".into());
            }
        }
    }

    pub fn invalidate_origin(&mut self) {
        self.reset();
        self.binding = None;
        self.draft = LogDraft::default();
        self.target = LogTarget::Workstation;
    }

    pub fn cancel_requests(&mut self) {
        self.request = None;
        self.active = None;
        self.generation = self.generation.wrapping_add(1);
        self.following = false;
        if self.status == LogStatus::Loading {
            self.status = LogStatus::Idle;
        }
    }

    fn reset(&mut self) {
        self.cancel_requests();
        self.rows.clear();
        self.table = TableState::new().key("scoped-logs-table");
        self.counts = None;
        self.apps.clear();
        self.apps_complete = false;
        self.shared_private_logs_available = false;
        self.export = LogExport::default();
        self.query = None;
        self.next_cursor = None;
        self.follow_cursor = None;
        self.available = false;
        self.message = None;
        self.status = LogStatus::Idle;
    }

    pub fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
        if !visible {
            self.following = false;
            if self.busy() {
                self.cancel_requests();
                self.status = if self.available {
                    LogStatus::Ready
                } else {
                    LogStatus::Idle
                };
            }
        }
    }

    pub fn set_following(&mut self, following: bool) {
        self.following = following && self.visible && self.available;
        if !self.following
            && self
                .active
                .as_ref()
                .is_some_and(|ticket| ticket.action == LogAction::Follow)
        {
            self.cancel_requests();
            self.status = LogStatus::Ready;
        }
    }

    pub fn select_target(&mut self, target: LogTarget) {
        self.reset();
        match &target {
            LogTarget::Workstation => self.draft.selected_workspace_only = false,
            LogTarget::SelectedWorkspace => self.draft.selected_workspace_only = false,
            _ => {}
        }
        self.target = target;
    }

    /// Edits invalidate loaded/private data immediately. Explicit Apply starts
    /// the new query; no per-keystroke network loop is started.
    pub fn draft_edited(&mut self) {
        self.reset();
    }

    fn draft_query(&self) -> Option<LogQuery> {
        self.binding.as_ref()?;
        let text = self.draft.text.text.trim();
        let app = self.draft.app.text.trim();
        let since = self.draft.since.text.trim();
        if text.len() > 256
            || app.len() > 64
            || since.len() > 64
            || [text, app, since]
                .iter()
                .any(|value| value.chars().any(char::is_control))
            || self
                .draft
                .min_rank
                .is_some_and(|rank| !(0..=3).contains(&rank))
        {
            return None;
        }
        let since = if since.is_empty() {
            None
        } else {
            Some(
                chrono::DateTime::parse_from_rfc3339(since)
                    .ok()?
                    .with_timezone(&chrono::Utc)
                    .to_rfc3339(),
            )
        };
        let mut query = LogQuery::default();
        query.limit = Some(PAGE_SIZE);
        query.text = (!text.is_empty()).then(|| text.into());
        query.app = (!app.is_empty()).then(|| app.into());
        query.since = since;
        query.min_rank = self.draft.min_rank;
        if let LogTarget::Job { id, descendants } = &self.target {
            if id.is_empty() || id.len() > 512 || id.chars().any(char::is_control) {
                return None;
            }
            query.job_id = Some(id.clone());
            query.descendants = Some(*descendants);
        }
        Some(query)
    }

    fn ticket(&mut self, action: LogAction, query: LogQuery) -> Option<LogTicket> {
        if self.busy() {
            return None;
        }
        self.sequence = self.sequence.wrapping_add(1);
        let ticket = LogTicket {
            binding: self.binding.clone()?,
            generation: self.generation,
            sequence: self.sequence,
            action,
            query,
        };
        self.active = Some(ticket.clone());
        self.status = LogStatus::Loading;
        Some(ticket)
    }

    pub fn refresh(&mut self) -> Option<LogTicket> {
        let query = match self.draft_query() {
            Some(query) => query,
            None => {
                self.message =
                    Some("Use bounded filters and a valid UTC/RFC3339 since timestamp.".into());
                return None;
            }
        };
        self.reset();
        self.query = Some(query.clone());
        self.ticket(LogAction::Refresh, query)
    }

    pub fn older(&mut self) -> Option<LogTicket> {
        if !self.can_load_older() {
            return None;
        }
        let mut query = LogQuery::default();
        query.cursor = self.next_cursor.clone();
        self.ticket(LogAction::Older, query)
    }

    /// The UI scheduler calls this only while the Logs panel/window is visible.
    /// Each request waits at most three seconds on the five-second server path.
    pub fn follow(&mut self) -> Option<LogTicket> {
        if !self.following || !self.can_follow() {
            return None;
        }
        let mut query = LogQuery::default();
        query.cursor = self.follow_cursor.clone();
        query.wait_ms = Some(3000);
        self.ticket(LogAction::Follow, query)
    }

    pub fn prepare_export(&mut self) -> Option<LogTicket> {
        if !self.can_export() {
            return None;
        }
        self.set_following(false);
        self.export = LogExport::default();
        self.ticket(LogAction::Export, self.query.clone()?)
    }

    pub fn continue_export(&mut self) -> Option<LogTicket> {
        if !self.can_export() || !self.export.has_more() {
            return None;
        }
        let mut query = LogQuery::default();
        query.cursor = self.export.next_cursor.clone();
        self.ticket(LogAction::Export, query)
    }

    fn current(&self, ticket: &LogTicket) -> bool {
        self.binding.as_ref() == Some(&ticket.binding)
            && self.generation == ticket.generation
            && self.active.as_ref().is_some_and(|active| {
                active.sequence == ticket.sequence && active.action == ticket.action
            })
    }

    /// Call only from a completion already bound to the app's current epoch.
    /// A queued response from a previous scope is ignored without side effects.
    pub fn complete(&mut self, ticket: &LogTicket, result: Result<LogReply, ClientError>) -> bool {
        if !self.current(ticket) {
            return false;
        }
        self.request = None;
        self.active = None;
        let result = match result {
            Ok(LogReply::Page(page)) if ticket.action != LogAction::Export => {
                self.complete_page(ticket.action, page)
            }
            Ok(LogReply::Export {
                bytes,
                next_cursor,
                apps_complete,
            }) if ticket.action == LogAction::Export => {
                self.complete_export(bytes, next_cursor, apps_complete)
            }
            Ok(_) => Err(ClientError::InvalidResponse),
            Err(error) => Err(error),
        };
        if let Err(error) = result {
            let rejected = matches!(error, ClientError::Status(401 | 403));
            self.reset();
            if rejected {
                self.blocked_binding = self.binding.take();
            }
            self.status = if matches!(error, ClientError::Status(409)) {
                LogStatus::RestartRequired
            } else {
                LogStatus::Unavailable
            };
            self.message = Some(match error {
                ClientError::Status(401 | 403) => "Log authority changed. Reconnect and register before continuing.".into(),
                ClientError::Status(409) => "Log scope or cursor changed. Refresh this scoped view to restart.".into(),
                _ => "Authorized operational logs are unavailable. Refresh this scope when the gateway is ready.".into(),
            });
        }
        true
    }

    fn complete_page(&mut self, action: LogAction, page: LogPage) -> Result<(), ClientError> {
        page.validate().map_err(|_| ClientError::InvalidResponse)?;
        if !page.available {
            self.reset();
            self.status = LogStatus::Unavailable;
            self.message = Some(match page.reason_code.as_deref() {
                Some("scope_exceeds_complete_log_cohort_limit") => {
                    "This scope exceeds 512 complete eligible jobs. Select an attributed job."
                        .into()
                }
                Some("shared_private_logs_unavailable") => {
                    "Private logs for this shared job are unavailable.".into()
                }
                _ => "A complete authorized operational log view is unavailable.".into(),
            });
            return Ok(());
        }
        if !page.private_logs_only
            || page.counts.is_none()
            || page.rows.len() > PAGE_SIZE
            || page.apps.len() > 32
            || [&page.next_cursor, &page.follow_cursor]
                .iter()
                .any(|cursor| cursor.as_ref().is_some_and(|value| !valid_cursor(value)))
            || (action != LogAction::Follow && page.has_more != page.next_cursor.is_some())
            || (action == LogAction::Follow && page.next_cursor.is_none())
        {
            return Err(ClientError::InvalidResponse);
        }
        let mut ids = BTreeSet::new();
        let mut fresh = Vec::new();
        for row in page.rows {
            validate_row(&row)?;
            if !ids.insert(row.id.clone()) {
                return Err(ClientError::InvalidResponse);
            }
            if let Some(existing) = self.rows.iter().find(|existing| existing.id == row.id) {
                if existing != &row {
                    return Err(ClientError::InvalidResponse);
                }
            } else {
                fresh.push(row);
            }
        }
        self.counts = page.counts;
        self.apps = page.apps;
        self.apps_complete = page.apps_complete;
        self.shared_private_logs_available = page.shared_private_logs_available;
        self.available = true;
        if self.rows.len() + fresh.len() > DISPLAY_LIMIT {
            self.following = false;
            self.status = LogStatus::DisplayLimit;
            self.message = Some("The received page exceeds the 500-summary display bound. Narrow filters or prepare a separate export; its cursor was not advanced.".into());
            return Ok(());
        }
        match action {
            LogAction::Follow => {
                fresh.reverse();
                fresh.append(&mut self.rows);
                self.rows = fresh;
                self.follow_cursor = page.next_cursor;
            }
            LogAction::Refresh | LogAction::Older => {
                self.rows.extend(fresh);
                self.next_cursor = page.next_cursor;
                if action == LogAction::Refresh {
                    self.follow_cursor = page.follow_cursor;
                }
            }
            _ => return Err(ClientError::InvalidResponse),
        }
        if self.rows.len() >= DISPLAY_LIMIT {
            self.following = false;
            self.status = LogStatus::DisplayLimit;
            self.message = Some("The display reached 500 safe summaries. Follow is paused; narrow filters or prepare a separate export.".into());
        } else {
            self.status = LogStatus::Ready;
            self.message = None;
        }
        Ok(())
    }

    fn complete_export(
        &mut self,
        bytes: Vec<u8>,
        next_cursor: Option<String>,
        apps_complete: bool,
    ) -> Result<(), ClientError> {
        if next_cursor
            .as_ref()
            .is_some_and(|value| !valid_cursor(value))
            || bytes.len() > EXPORT_LIMIT
        {
            return Err(ClientError::InvalidResponse);
        }
        let source = std::str::from_utf8(&bytes).map_err(|_| ClientError::InvalidResponse)?;
        let mut canonical = Vec::new();
        let mut count = 0;
        for line in source.lines() {
            let row: LogEntry =
                serde_json::from_str(line).map_err(|_| ClientError::InvalidResponse)?;
            validate_row(&row)?;
            serde_json::to_writer(&mut canonical, &row)
                .map_err(|_| ClientError::InvalidResponse)?;
            canonical.push(b'\n');
            count += 1;
            if count > PAGE_SIZE {
                return Err(ClientError::InvalidResponse);
            }
        }
        if next_cursor.is_some() && count == 0 {
            return Err(ClientError::InvalidResponse);
        }
        if self.export.bytes.len().saturating_add(canonical.len()) > EXPORT_LIMIT {
            self.export.bounded = true;
            self.export.complete = false;
            self.message = Some("The export reached its 2 MiB bound. Only an explicitly labelled partial export can be saved.".into());
        } else {
            self.export.bytes.extend(canonical);
            self.export.rows += count;
            self.export.pages += 1;
            self.export.started = true;
            self.export.apps_complete = if self.export.pages == 1 {
                apps_complete
            } else {
                self.export.apps_complete && apps_complete
            };
            self.export.complete = next_cursor.is_none();
            self.export.next_cursor = next_cursor;
            self.export.bounded = self.export.pages >= EXPORT_PAGES && !self.export.complete;
            self.message = if self.export.bounded {
                Some("The export reached its 16-page bound. Save it only as a labelled partial export or narrow the scope.".into())
            } else {
                None
            };
        }
        self.status = LogStatus::Ready;
        Ok(())
    }

    pub fn copy_loaded(&self) -> Option<LogCopy> {
        if !self.can_copy_loaded() {
            return None;
        }
        let mut text = String::new();
        for row in &self.rows {
            text.push_str(&serde_json::to_string(row).ok()?);
            text.push('\n');
        }
        Some(LogCopy {
            text,
            rows: self.rows.len(),
            complete_eligible_scope: self.next_cursor.is_none()
                && self
                    .counts
                    .as_ref()
                    .is_some_and(|counts| counts.total == self.rows.len() as u64),
            excludes_shared_private: !self.shared_private_logs_available,
        })
    }
}

fn valid_cursor(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn validate_row(row: &LogEntry) -> Result<(), ClientError> {
    row.validate().map_err(|_| ClientError::InvalidResponse)
}

/// This finite effect never starts a durable operation or follows redirects.
pub async fn execute(client: Client, ticket: &LogTicket) -> Result<LogReply, ClientError> {
    let client = client.with_registered_context(&ticket.binding.context)?;
    match ticket.action {
        LogAction::Export => {
            let page = client.export_logs(&ticket.query).await?;
            Ok(LogReply::Export {
                bytes: page.bytes,
                next_cursor: page.next_cursor,
                apps_complete: page.apps_complete,
            })
        }
        LogAction::Follow => tokio::time::timeout(
            Duration::from_secs(6),
            client.get_with_query("/api/v1/logs/follow", &ticket.query),
        )
        .await
        .map_err(|_| ClientError::Timeout)?
        .map(LogReply::Page),
        _ => client
            .get_with_query("/api/v1/logs", &ticket.query)
            .await
            .map(LogReply::Page),
    }
}

#[cfg(test)]
#[path = "log_tests.rs"]
mod tests;
