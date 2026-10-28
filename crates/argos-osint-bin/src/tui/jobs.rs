//! Jobs dashboard: a bounded, filterable view over durable jobs/tasks/attempts
//! with its own selection, filter and scroll state.

use argos_osint_core::jobs_view::{
    format_duration, JobCounts, JobDetail, JobFilter, JobRow, JobStatusFilter,
};
use argos_osint_core::store::Store;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use super::app::{App, ButtonId, FieldId, Target};
use super::logs::local_time;
use super::theme;
use super::ui::{contains, draw_button_state, draw_field, fit, inset, pane, ACTION_H, FIELD_H};

/// Where the Jobs pane reads: the SQLite store, or the SurrealDB repository
/// under a storage config (see `surreal_brain::SurrealJobs`). One pane never
/// mixes the two.
pub trait JobsSource {
    fn list(&self, filter: &JobFilter, limit: usize) -> anyhow::Result<Vec<JobRow>>;
    fn counts(&self) -> anyhow::Result<JobCounts>;
    fn apps(&self) -> anyhow::Result<Vec<String>>;
    fn detail(&self, id: &str, tasks: usize) -> anyhow::Result<Option<JobDetail>>;
    fn job(&self, id: &str) -> anyhow::Result<Option<JobRow>>;
}

impl JobsSource for Store {
    fn list(&self, filter: &JobFilter, limit: usize) -> anyhow::Result<Vec<JobRow>> {
        self.list_jobs(filter, limit)
    }
    fn counts(&self) -> anyhow::Result<JobCounts> {
        self.job_counts()
    }
    fn apps(&self) -> anyhow::Result<Vec<String>> {
        self.job_apps()
    }
    fn detail(&self, id: &str, tasks: usize) -> anyhow::Result<Option<JobDetail>> {
        self.job_detail(id, tasks)
    }
    fn job(&self, id: &str) -> anyhow::Result<Option<JobRow>> {
        self.get_job(id)
    }
}

pub const PAGE: usize = 200;
/// Below this body width the table and detail share one panel.
pub const WIDE: u16 = 100;

#[derive(Clone, Debug, Default)]
pub struct JobsView {
    pub rows: Vec<JobRow>,
    pub sel: usize,
    pub sel_id: Option<String>,
    pub scroll: u16,
    pub detail: Option<JobDetail>,
    pub detail_scroll: u16,
    /// Narrow layouts: the detail replaces the table.
    pub detail_open: bool,
    pub status: JobStatusFilter,
    pub app: String,
    pub apps: Vec<String>,
    pub search: String,
    pub counts: JobCounts,
    /// Last read failure; the previous rows stay visible.
    pub error: Option<String>,
    pub loaded: bool,
}

impl JobsView {
    pub fn filter(&self) -> JobFilter {
        JobFilter {
            status: self.status,
            app: self.app.clone(),
            text: self.search.trim().to_string(),
            ..Default::default()
        }
    }

    /// A user changed the cohort. Old rows/detail must not masquerade as the
    /// new filter while an asynchronous read is still in flight.
    pub fn invalidate_filter(&mut self) {
        self.rows.clear();
        self.sel = 0;
        self.sel_id = None;
        self.scroll = 0;
        self.detail = None;
        self.detail_scroll = 0;
        self.detail_open = false;
        self.error = None;
        self.loaded = false;
    }

    /// Immediately filter existing rows in-place based on the current filter settings.
    /// Retains matching rows, preserves or clamps selection, synchronizes active detail,
    /// and keeps loaded state so the UI updates immediately without flashing "Loading jobs".
    pub fn apply_filter(&mut self) {
        let filter = self.filter();
        self.rows.retain(|row| filter.matches(row));

        if let Some(id) = self.sel_id.as_ref() {
            if let Some(index) = self.rows.iter().position(|row| &row.id == id) {
                self.sel = index;
            } else {
                self.sel = self.sel.min(self.rows.len().saturating_sub(1));
            }
        } else {
            self.sel = self.sel.min(self.rows.len().saturating_sub(1));
        }
        self.sel_id = self.rows.get(self.sel).map(|row| row.id.clone());

        if let Some(row) = self.selected() {
            if self.detail.as_ref().map(|d| &d.job.id) != Some(&row.id) {
                let retryable = if row.state == "failed" || row.state == "partial" {
                    1
                } else {
                    0
                };
                self.detail = Some(JobDetail {
                    job: row.clone(),
                    retryable,
                    ..Default::default()
                });
                self.detail_scroll = 0;
            }
        } else {
            self.detail = None;
            self.detail_scroll = 0;
            self.detail_open = false;
        }
    }

    /// Optimistically prune non-service rows matching a history clear filter
    /// so the table and detail pane reflect the action immediately while the
    /// authoritative repository reload completes.
    pub fn prune_status(&mut self, status: JobStatusFilter) {
        let states = status.states();
        if states.is_empty() {
            return;
        }
        let selected_id = self.selected().map(|row| row.id.clone());
        self.rows
            .retain(|row| row.kind == "service" || !states.contains(&row.state.as_str()));
        match status {
            JobStatusFilter::Completed => self.counts.completed_recent = 0,
            JobStatusFilter::Failed => self.counts.failed = 0,
            _ => {}
        }
        if let Some(id) = selected_id {
            if let Some(index) = self.rows.iter().position(|row| row.id == id) {
                self.sel = index;
            } else {
                self.sel = self.sel.min(self.rows.len().saturating_sub(1));
            }
        } else {
            self.sel = 0;
        }
        self.sel_id = self.rows.get(self.sel).map(|row| row.id.clone());

        if let Some(row) = self.selected() {
            if self.detail.as_ref().map(|d| &d.job.id) != Some(&row.id) {
                let retryable = if row.state == "failed" || row.state == "partial" {
                    1
                } else {
                    0
                };
                self.detail = Some(JobDetail {
                    job: row.clone(),
                    retryable,
                    ..Default::default()
                });
                self.detail_scroll = 0;
            }
        } else {
            self.detail = None;
            self.detail_scroll = 0;
            self.detail_open = false;
        }
    }

    /// Reload rows and the selected job's detail, keeping the selection by id.
    pub fn reload(&mut self, store: &impl JobsSource) {
        match store.list(&self.filter(), PAGE) {
            Ok(rows) => {
                self.rows = rows;
                self.error = None;
                self.loaded = true;
                if let Some(index) = self
                    .sel_id
                    .as_ref()
                    .and_then(|id| self.rows.iter().position(|row| &row.id == id))
                {
                    self.sel = index;
                } else {
                    self.sel = self.sel.min(self.rows.len().saturating_sub(1));
                }
                self.sel_id = self.rows.get(self.sel).map(|row| row.id.clone());
            }
            Err(err) => self.error = Some(format!("Could not read jobs: {err:#}")),
        }
        if let Ok(counts) = store.counts() {
            self.counts = counts;
        }
        if let Ok(apps) = store.apps() {
            self.apps = apps;
        }
        self.load_detail(store);
    }

    pub fn load_detail(&mut self, store: &impl JobsSource) {
        let Some(id) = self.selected().map(|row| row.id.clone()) else {
            self.detail = None;
            return;
        };
        match store.detail(&id, 40) {
            Ok(detail) => self.detail = detail,
            Err(err) => self.error = Some(format!("Could not read job {id}: {err:#}")),
        }
    }

    pub fn selected(&self) -> Option<&JobRow> {
        self.rows.get(self.sel)
    }

    pub fn select(&mut self, index: usize, store: &impl JobsSource) {
        self.select_cached(index);
        self.load_detail(store);
    }

    pub fn select_cached(&mut self, index: usize) {
        if self.rows.is_empty() {
            return;
        }
        let index = index.min(self.rows.len() - 1);
        if index != self.sel {
            self.detail_scroll = 0;
            self.detail = None;
        }
        self.sel = index;
        self.sel_id = self.rows.get(index).map(|row| row.id.clone());
    }

    pub fn accept(&mut self, snapshot: Self, select_snapshot: bool) {
        let selected = if select_snapshot {
            snapshot.selected().map(|j| j.id.clone())
        } else {
            self.selected().map(|j| j.id.clone())
        };
        self.rows = snapshot.rows;
        self.counts = snapshot.counts;
        self.apps = snapshot.apps;
        self.error = snapshot.error;
        self.loaded = snapshot.loaded;
        self.sel = selected
            .and_then(|id| self.rows.iter().position(|j| j.id == id))
            .unwrap_or(self.sel.min(self.rows.len().saturating_sub(1)));
        self.sel_id = self.rows.get(self.sel).map(|j| j.id.clone());
        if snapshot.detail.as_ref().map(|d| &d.job.id) == self.sel_id.as_ref() {
            self.detail = snapshot.detail;
        } else if let Some(row) = self.selected() {
            if self.detail.as_ref().map(|d| &d.job.id) != Some(&row.id) {
                let retryable = if row.state == "failed" || row.state == "partial" {
                    1
                } else {
                    0
                };
                self.detail = Some(JobDetail {
                    job: row.clone(),
                    retryable,
                    ..Default::default()
                });
            }
        } else {
            self.detail = None;
        }
    }

    pub fn move_by(&mut self, delta: i32, store: &impl JobsSource) {
        if self.rows.is_empty() {
            return;
        }
        let last = self.rows.len() as i32 - 1;
        self.select((self.sel as i32 + delta).clamp(0, last) as usize, store);
    }

    /// Log → job navigation: select the job's top-level ancestor, clearing
    /// filters only when they hide it. Returns false when it no longer exists.
    pub fn focus_job(&mut self, store: &impl JobsSource, id: &str) -> bool {
        let mut target = id.to_string();
        for _ in 0..8 {
            match store.job(&target) {
                Ok(Some(job)) if !job.parent_id.is_empty() => target = job.parent_id,
                Ok(Some(_)) => break,
                _ => return false,
            }
        }
        self.sel_id = Some(target.clone());
        self.reload(store);
        if self.selected().map(|row| row.id.as_str()) != Some(target.as_str()) {
            self.status = JobStatusFilter::All;
            self.app.clear();
            self.search.clear();
            self.sel_id = Some(target.clone());
            self.reload(store);
        }
        self.detail_scroll = 0;
        self.selected().map(|row| row.id.as_str()) == Some(target.as_str())
    }

    pub fn cycle_status(&mut self) {
        let all = JobStatusFilter::ALL;
        let pos = all.iter().position(|s| *s == self.status).unwrap_or(0);
        self.status = all[(pos + 1) % all.len()];
    }

    pub fn cycle_app(&mut self) {
        let mut options = vec![String::new()];
        options.extend(self.apps.iter().cloned());
        let pos = options.iter().position(|a| *a == self.app).unwrap_or(0);
        self.app = options[(pos + 1) % options.len()].clone();
    }

    pub fn can_retry(&self) -> bool {
        self.detail
            .as_ref()
            .is_some_and(|d| d.retryable > 0 && !d.job.cancel_requested)
    }

    /// Stop is offered for supported running, queued or retry-scheduled work
    /// that has not already been asked to stop.
    pub fn can_cancel(&self) -> bool {
        self.selected().is_some_and(|job| {
            job.cancellable
                && !job.cancel_requested
                && matches!(job.state.as_str(), "running" | "queued" | "retry_scheduled")
        })
    }
}

pub struct JobsAreas {
    pub summary: Rect,
    pub search: Rect,
    pub actions: Rect,
    pub action_height: u16,
    /// Zero-sized when the narrow detail replaces it.
    pub table: Rect,
    /// Zero-sized when narrow and the detail is closed.
    pub detail: Rect,
}

pub fn areas(body: Rect, view: &JobsView) -> JobsAreas {
    let action_height = if body.height < 24 { 1 } else { ACTION_H };
    let summary_h = 1.min(body.height);
    let search_h = FIELD_H.min(body.height.saturating_sub(summary_h));
    let ids = buttons(view, true);
    let labels = ids
        .iter()
        .map(|id| button_label(view, *id))
        .collect::<Vec<_>>();
    let refs = labels.iter().map(String::as_str).collect::<Vec<_>>();
    let virtual_rects = super::components::wrapped_action_rects(
        Rect::new(0, 0, body.width, u16::MAX),
        &refs,
        action_height,
    );
    let required = virtual_rects
        .last()
        .map_or(action_height, |r| r.y.saturating_add(r.height));
    let actions_h = required.min(body.height.saturating_sub(summary_h + search_h));
    let rest_y = body.y + summary_h + search_h + actions_h;
    let rest_h = body.height.saturating_sub(summary_h + search_h + actions_h);
    let at = |offset: u16, height: u16| Rect {
        x: body.x,
        y: body.y.saturating_add(offset),
        width: body.width,
        height,
    };
    let rest = Rect {
        x: body.x,
        y: rest_y,
        width: body.width,
        height: rest_h,
    };
    let (table, detail) = if body.width >= WIDE {
        let table_w = body.width * 62 / 100;
        (
            Rect {
                width: table_w,
                ..rest
            },
            Rect {
                x: rest.x + table_w,
                width: rest.width - table_w,
                ..rest
            },
        )
    } else if view.detail_open {
        (
            Rect {
                width: 0,
                height: 0,
                ..rest
            },
            rest,
        )
    } else {
        (
            rest,
            Rect {
                width: 0,
                height: 0,
                ..rest
            },
        )
    };
    JobsAreas {
        summary: at(0, summary_h),
        search: at(summary_h, search_h),
        actions: at(summary_h + search_h, actions_h),
        action_height,
        table,
        detail,
    }
}

/// Action buttons in paint/hit order. Retry and Open source appear only when
/// they are truthful for the selected job.
pub fn buttons(view: &JobsView, can_open_source: bool) -> Vec<ButtonId> {
    let mut out = vec![
        ButtonId::JobsStatus,
        ButtonId::JobsApp,
        ButtonId::JobsClearCompleted,
        ButtonId::JobsClearStopped,
        ButtonId::JobsClearFailed,
    ];
    if view.selected().is_some() {
        out.push(ButtonId::JobsViewLogs);
    }
    if view.can_retry() {
        out.push(ButtonId::JobsRetry);
    }
    if view.can_cancel() {
        out.push(ButtonId::JobsCancel);
    }
    if can_open_source {
        out.push(ButtonId::JobsOpenSource);
    }
    out
}

fn button_label(view: &JobsView, button: ButtonId) -> String {
    match button {
        ButtonId::JobsStatus => format!("Status: {}", view.status.label()),
        ButtonId::JobsApp => format!(
            "App: {}",
            if view.app.is_empty() {
                "all"
            } else {
                view.app.as_str()
            }
        ),
        ButtonId::JobsViewLogs => "View logs".into(),
        ButtonId::JobsRetry => if view
            .detail
            .as_ref()
            .is_some_and(|d| !d.insight_progress.is_empty())
        {
            "Retry extraction"
        } else if view.detail.as_ref().is_some_and(|d| {
            d.tasks
                .iter()
                .any(|t| argos_osint_core::tasks::INDEX_OPERATIONS.contains(&t.operation.as_str()))
        }) {
            "Retry index"
        } else {
            "Retry failed"
        }
        .into(),
        ButtonId::JobsCancel => "Stop job".into(),
        ButtonId::JobsClearCompleted => "Clear completed".into(),
        ButtonId::JobsClearStopped => "Clear stopped".into(),
        ButtonId::JobsClearFailed => "Clear failed".into(),
        ButtonId::JobsOpenSource => "Open source".into(),
        _ => String::new(),
    }
}

fn action_geometry(area: Rect, view: &JobsView, ids: &[ButtonId], height: u16) -> Vec<Rect> {
    let labels = ids
        .iter()
        .map(|id| button_label(view, *id))
        .collect::<Vec<_>>();
    super::components::wrapped_action_rects(
        area,
        &labels.iter().map(String::as_str).collect::<Vec<_>>(),
        height,
    )
}

pub fn summary_text(counts: &JobCounts) -> String {
    format!(
        "{} active · {} queued · {} retrying · {} failed · {} completed (24 h)",
        counts.active, counts.queued, counts.retrying, counts.failed, counts.completed_recent
    )
}

fn progress(job: &JobRow) -> String {
    let mut out = job.phase.clone();
    if let (Some(done), Some(total)) = (job.progress_done, job.progress_total) {
        if total > 0 {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(&format!("{done}/{total}"));
        }
    }
    out
}

fn title(job: &JobRow) -> String {
    if !job.title.is_empty() {
        job.title.clone()
    } else if !job.operation.is_empty() {
        job.operation.clone()
    } else {
        job.kind.clone()
    }
}

fn state_style(job: &JobRow) -> ratatui::style::Style {
    match job.state.as_str() {
        "failed" | "partial" => theme::error(),
        "retry_scheduled" | "paused" | "blocked" => theme::warn(),
        "running" | "queued" => theme::accent(),
        _ => theme::dim(),
    }
}

/// Column layout for the Jobs table. Optional columns drop on narrow widths
/// (logs first, then phase) so every visible header stays readable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TableColumns {
    pub title: usize,
    pub phase: bool,
    pub logs: bool,
}

const MIN_TITLE: usize = 16;
/// status 11 + app 7 + active 12 + elapsed 12 + try 5.
const BASE_COLUMNS: usize = 11 + 7 + 12 + 12 + 5;
const PHASE_COLUMN: usize = 12;
const LOGS_COLUMN: usize = 5;

impl TableColumns {
    pub fn for_width(width: usize) -> Self {
        let phase = width >= BASE_COLUMNS + PHASE_COLUMN + MIN_TITLE;
        let used = BASE_COLUMNS + if phase { PHASE_COLUMN } else { 0 };
        let logs = width >= used + LOGS_COLUMN + MIN_TITLE;
        let used = used + if logs { LOGS_COLUMN } else { 0 };
        Self {
            title: width.saturating_sub(used).max(8),
            phase,
            logs,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn row(
        &self,
        status: &str,
        job: &str,
        app: &str,
        phase: &str,
        active: &str,
        elapsed: &str,
        attempts: &str,
        logs: &str,
    ) -> String {
        let mut out = format!(
            "{:<11}{:<w$} {:<6} ",
            fit(status, 10),
            fit(job, self.title.saturating_sub(1)),
            fit(app, 6),
            w = self.title.saturating_sub(1)
        );
        if self.phase {
            out.push_str(&format!("{:<11} ", fit(phase, 11)));
        }
        out.push_str(&format!(
            "{:>11} {:>11} {:>4}",
            fit(active, 11),
            fit(elapsed, 11),
            fit(attempts, 4)
        ));
        if self.logs {
            out.push_str(&format!(" {:^4}", logs));
        }
        out
    }
}

/// One table line per job: status, title, app, phase/progress, active, elapsed,
/// attempts, log indicator.
pub fn table_lines(
    view: &JobsView,
    width: usize,
    now: chrono::DateTime<chrono::Utc>,
) -> Vec<Line<'static>> {
    let width = width.max(1);
    let cols = TableColumns::for_width(width);
    let mut lines = vec![Line::from(Span::styled(
        fit(
            &cols.row(
                "status", "job", "app", "phase", "active", "elapsed", "try", "logs",
            ),
            width,
        ),
        theme::muted(),
    ))];
    for (index, job) in view.rows.iter().enumerate() {
        if index > 0 {
            lines.push(Line::raw(""));
        }
        let attempts = if job.attempt_cap > 0 {
            format!("{}/{}", job.attempts_used, job.attempt_cap)
        } else {
            job.attempts_used.to_string()
        };
        let state = super::anim::job_state_label(&job.state, job.state_label());
        let text = cols.row(
            &state,
            &title(job),
            &job.app,
            &progress(job),
            &format_duration(job.active_now(now)),
            &format_duration(job.elapsed_ms(now)),
            &attempts,
            if job.events > 0 { "◆" } else { "" },
        );
        let style = if index == view.sel {
            theme::selected()
        } else {
            state_style(job)
        };
        lines.push(Line::from(Span::styled(fit(&text, width), style)));
    }
    lines
}

fn stamp(ts: &str) -> String {
    if ts.is_empty() {
        "Unavailable".into()
    } else {
        local_time(ts)
    }
}

pub fn detail_lines(detail: &JobDetail, now: chrono::DateTime<chrono::Utc>) -> Vec<String> {
    let job = &detail.job;
    let mut lines = vec![
        title(job),
        format!("{} · {} · {}", job.state_label(), job.app, job.id),
    ];
    if !job.operation.is_empty() {
        lines.push(format!("Operation: {}", job.operation));
    }
    if job.is_service() {
        lines.push(
            "Long-lived service · stays available while Argos runs, including when idle.".into(),
        );
        lines.push(
            "Elapsed is service age; individual tasks and attempts below show actual work.".into(),
        );
    }
    for progress in &detail.insight_progress {
        lines.extend(progress.lines());
    }
    if job.cancel_requested && matches!(job.state.as_str(), "running" | "queued") {
        lines.push("Stopping… (stops at the next safe point)".into());
    }
    let progress = progress(job);
    if !progress.is_empty() {
        lines.push(format!("Phase: {progress}"));
    }
    lines.push(String::new());
    lines.push(format!(
        "Created {} · started {} · finished {}",
        stamp(&job.created_at),
        stamp(&job.started_at),
        stamp(&job.finished_at)
    ));
    if !job.heartbeat_at.is_empty() && job.is_active() {
        lines.push(format!("Last heartbeat {}", stamp(&job.heartbeat_at)));
    }
    lines.push(format!(
        "Active {} · queued {} · retry wait {} · elapsed {}",
        format_duration(job.active_now(now)),
        format_duration(job.queue_ms),
        format_duration(job.retry_wait_ms),
        format_duration(job.elapsed_ms(now))
    ));
    // Task attempts (each task start) and provider requests (every dispatch,
    // including retries and fallback routes) are different counts.
    lines.push(format!(
        "Task attempts {}{} · provider requests {}",
        job.attempts_used,
        if job.attempt_cap > 0 {
            format!(" of {}", job.attempt_cap)
        } else {
            String::new()
        },
        detail.wire_requests
    ));
    let resource = [
        ("Run", &job.run_ref),
        ("Resource", &job.resource_ref),
        ("Result", &job.result_ref),
        ("Provider", &job.provider),
        ("Model", &job.model),
        ("Tool", &job.tool),
        ("Worker", &job.worker_owner),
    ];
    for (label, value) in resource {
        if !value.is_empty() {
            lines.push(format!("{label}: {value}"));
        }
    }
    if !job.error_summary.is_empty() {
        lines.push(String::new());
        lines.push(format!(
            "Latest error{}: {}",
            if job.error_category.is_empty() {
                String::new()
            } else {
                format!(" ({})", job.error_category)
            },
            job.error_summary
        ));
    }
    for warning in &detail.warnings {
        lines.push(format!("Warning · {warning}"));
    }
    if !detail.warnings.is_empty() {
        lines.push(
            "Partial result published · Retry in the brief re-asks only the failed items".into(),
        );
    }
    lines.push(String::new());
    if detail.events_expired {
        lines.push(format!(
            "Logs: event detail expired ({} h retention); the terminal summary above is kept",
            argos_osint_core::events::DEFAULT_RETENTION_HOURS
        ));
    } else if job.events > 0 {
        lines.push(format!(
            "Logs: {} events · View logs opens them",
            job.events
        ));
    } else {
        lines.push("Logs: no events recorded for this job".into());
    }
    if !detail.children.is_empty() {
        lines.push(String::new());
        lines.push(format!("Phases ({})", detail.children.len()));
        for (index, child) in detail.children.iter().enumerate() {
            if index > 0 {
                lines.push(String::new());
            }
            lines.push(format!(
                "  {} {} {}",
                child.state_label(),
                title(child),
                format_duration(child.active_now(now))
            ));
        }
    }
    if detail.task_total > 0 {
        lines.push(String::new());
        lines.push(format!(
            "Tasks ({} of {})",
            detail.tasks.len(),
            detail.task_total
        ));
        for (index, task) in detail.tasks.iter().enumerate() {
            if index > 0 {
                lines.push(String::new());
            }
            let mut line = if task.stage_label.is_empty() {
                format!(
                    "  {} {} · {}/{} attempts",
                    task.state, task.operation, task.attempts, task.max_attempts
                )
            } else {
                format!(
                    "  {} · {}/{} attempts",
                    task.stage_label, task.attempts, task.max_attempts
                )
            };
            if task.state == "retry_scheduled" && !task.next_eligible_at.is_empty() {
                line.push_str(&format!(
                    " · retry at {}",
                    local_time(&task.next_eligible_at)
                ));
            }
            if !task.error_message.is_empty() {
                line.push_str(&format!(" · {}", task.error_message));
            }
            lines.push(line);
        }
    }
    if !detail.attempts.is_empty() {
        lines.push(String::new());
        lines.push("Attempt history".into());
        for (index, attempt) in detail.attempts.iter().enumerate() {
            if index > 0 {
                lines.push(String::new());
            }
            let mut line = format!(
                "  {} #{} {} {} {}",
                if attempt.task_label.is_empty() {
                    "Task"
                } else {
                    attempt.task_label.as_str()
                },
                attempt.attempt_number,
                stamp(&attempt.started_at),
                if attempt.outcome.is_empty() {
                    "running"
                } else {
                    attempt.outcome.as_str()
                },
                format_duration(attempt.duration_ms)
            );
            if !attempt.error_category.is_empty() {
                line.push_str(&format!(" [{}]", attempt.error_category));
            }
            if !attempt.error_message.is_empty() {
                line.push_str(&format!(" · {}", attempt.error_message));
            }
            lines.push(line);
        }
    }
    lines
}

fn measured_detail(
    view: &JobsView,
    width: u16,
    now: chrono::DateTime<chrono::Utc>,
) -> Vec<Line<'static>> {
    let lines = match &view.detail {
        Some(detail) => detail_lines(detail, now),
        None => vec!["Select a job to see its timing, tasks, and attempts.".into()],
    };
    super::components::measured_lines(lines.into_iter().map(Line::raw).collect(), width)
}

pub fn detail_scroll_max(app: &App) -> u16 {
    let inner = inset(areas(super::ui::body_rect(app), &app.jobs).detail);
    measured_detail(&app.jobs, inner.width, chrono::Utc::now())
        .len()
        .saturating_sub(inner.height as usize)
        .min(u16::MAX as usize) as u16
}

pub fn draw(frame: &mut Frame, app: &App, body: Rect, can_open_source: bool) {
    let view = &app.jobs;
    let a = areas(body, view);
    let now = chrono::Utc::now();
    frame.render_widget(
        Paragraph::new(fit(&summary_text(&view.counts), a.summary.width as usize)).style(
            if view.counts.failed > 0 {
                theme::warn()
            } else {
                theme::dim()
            },
        ),
        a.summary,
    );
    draw_field(frame, app, FieldId::JobsSearch, "filter", a.search);
    let ids = buttons(view, can_open_source);
    for (button, rect) in ids
        .iter()
        .zip(action_geometry(a.actions, view, &ids, a.action_height))
    {
        draw_button_state(
            frame,
            app,
            *button,
            &button_label(view, *button),
            rect,
            false,
        );
    }
    if a.table.width > 0 && a.table.height > 0 {
        let width = inset(a.table).width as usize;
        let mut lines = Vec::new();
        if let Some(err) = &view.error {
            lines.push(Line::from(Span::styled(
                fit(&format!("{err} (showing the last list read)"), width),
                theme::error(),
            )));
        }
        let inner = inset(a.table);
        if view.rows.is_empty() && !view.loaded && view.error.is_none() {
            lines = super::anim::centered_lines(
                "Loading jobs",
                theme::dim(),
                inner.width,
                inner.height,
            );
        } else if view.rows.is_empty() {
            let text = if view.status != JobStatusFilter::All
                || !view.app.is_empty()
                || !view.search.trim().is_empty()
            {
                "No jobs match these filters.".to_string()
            } else {
                "No background work recorded yet.".to_string()
            };
            lines.push(Line::from(Span::styled(text, theme::dim())));
        } else {
            let mut table = table_lines(view, width, now);
            let data = table.split_off(1);
            lines.extend(table);
            lines.extend(data.into_iter().skip(view.scroll as usize * 2));
        }
        frame.render_widget(Paragraph::new(lines).block(pane(" jobs ")), a.table);
        let inner = inset(a.table);
        if view.error.is_none() && !view.rows.is_empty() {
            let start = view.scroll as usize;
            let count = super::components::ListGeometry::capacity(
                inner.height.saturating_sub(1) as usize,
                1,
            );
            for i in 0..count {
                let index = start + i;
                if index < view.rows.len() {
                    app.layout.borrow_mut().register(
                        Target::JobRow(index),
                        Rect {
                            x: inner.x,
                            y: inner.y + 1 + i as u16 * 2,
                            width: inner.width,
                            height: 1,
                        },
                    );
                }
            }
        }
    }
    if a.detail.width > 0 && a.detail.height > 0 {
        app.layout
            .borrow_mut()
            .register(Target::JobDetail, a.detail);
        let lines = measured_detail(view, inset(a.detail).width, now);
        let offset = view.detail_scroll.min(
            lines
                .len()
                .saturating_sub(inset(a.detail).height as usize)
                .min(u16::MAX as usize) as u16,
        );
        let focused = app.focus == Target::JobDetail;
        frame.render_widget(
            Paragraph::new(lines)
                .style(theme::text())
                .block(pane(if focused {
                    " detail · focused "
                } else {
                    " detail "
                }))
                .scroll((offset, 0)),
            a.detail,
        );
    }
}

/// Table row index under a point. The header occupies the first inner line.
pub fn row_at(view: &JobsView, table: Rect, x: u16, y: u16) -> Option<usize> {
    let inner = inset(table);
    if view.error.is_some() || !contains(inner, x, y) || y == inner.y {
        return None;
    }
    let row = (y - inner.y - 1) as usize;
    let index = super::components::ListGeometry::fixed(view.rows.len(), 1).index_at(row)?
        + view.scroll as usize;
    (index < view.rows.len()).then_some(index)
}

pub fn hit(app: &App, body: Rect, x: u16, y: u16, can_open_source: bool) -> Option<Target> {
    let view = &app.jobs;
    let a = areas(body, view);
    if contains(a.search, x, y) {
        return Some(Target::Field(FieldId::JobsSearch));
    }
    if contains(a.actions, x, y) {
        let ids = buttons(view, can_open_source);
        return ids
            .iter()
            .zip(action_geometry(a.actions, view, &ids, a.action_height))
            .find(|(_, rect)| contains(*rect, x, y))
            .map(|(button, _)| Target::Button(*button));
    }
    if contains(a.detail, x, y) {
        return Some(Target::JobDetail);
    }
    row_at(view, a.table, x, y).map(Target::JobRow)
}

/// Rows visible in the table (header excluded).
pub fn table_room(body: Rect, view: &JobsView) -> usize {
    super::components::ListGeometry::capacity(
        inset(areas(body, view).table).height.saturating_sub(1) as usize,
        1,
    )
    .max(1)
}

pub fn reveal(view: &mut JobsView, room: usize) {
    let top = view.scroll as usize;
    if view.sel < top {
        view.scroll = view.sel as u16;
    } else if view.sel >= top + room {
        view.scroll = (view.sel + 1 - room) as u16;
    }
}

#[cfg(test)]
mod tests {
    use super::TableColumns;

    #[test]
    fn optional_columns_drop_instead_of_truncating_headers() {
        let wide = TableColumns::for_width(120);
        assert!(wide.phase && wide.logs);
        let mid = TableColumns::for_width(78);
        assert!(mid.phase && !mid.logs, "{mid:?}");
        let narrow = TableColumns::for_width(60);
        assert!(!narrow.phase && !narrow.logs, "{narrow:?}");
        for width in [50usize, 60, 70, 80, 90, 120] {
            let cols = TableColumns::for_width(width);
            let header = cols.row(
                "status", "job", "app", "phase", "active", "elapsed", "try", "logs",
            );
            assert!(
                header.chars().count() <= width.max(cols.title + 47),
                "{width}: {header}"
            );
            if cols.logs {
                assert!(header.trim_end().ends_with("logs"), "{width}: {header}");
            }
        }
    }

    #[test]
    fn test_apply_filter_in_place_immediate() {
        use super::*;
        use argos_osint_core::jobs_view::{JobRow, JobStatusFilter};

        let mut view = JobsView {
            rows: vec![
                JobRow {
                    id: "j1".into(),
                    state: "running".into(),
                    app: "intel".into(),
                    title: "Intel gathering".into(),
                    ..Default::default()
                },
                JobRow {
                    id: "j2".into(),
                    state: "completed".into(),
                    app: "recon".into(),
                    title: "Recon scan".into(),
                    ..Default::default()
                },
            ],
            loaded: true,
            sel: 1,
            sel_id: Some("j2".into()),
            status: JobStatusFilter::Active,
            ..Default::default()
        };

        view.apply_filter();
        assert_eq!(view.rows.len(), 1);
        assert_eq!(view.rows[0].id, "j1");
        assert_eq!(view.sel, 0);
        assert_eq!(view.sel_id.as_deref(), Some("j1"));
        assert!(view.loaded);
        assert!(view.detail.is_some());
        assert_eq!(view.detail.as_ref().unwrap().job.id, "j1");
    }

    #[test]
    fn test_prune_status_preserves_active_detail_and_retry() {
        use super::*;
        use argos_osint_core::jobs_view::{JobDetail, JobRow, JobStatusFilter};

        let mut view = JobsView {
            rows: vec![
                JobRow {
                    id: "j-done".into(),
                    state: "completed".into(),
                    ..Default::default()
                },
                JobRow {
                    id: "j-failed".into(),
                    state: "failed".into(),
                    title: "Failed job".into(),
                    ..Default::default()
                },
            ],
            sel: 0,
            sel_id: Some("j-done".into()),
            detail: Some(JobDetail {
                job: JobRow {
                    id: "j-done".into(),
                    state: "completed".into(),
                    ..Default::default()
                },
                ..Default::default()
            }),
            ..Default::default()
        };

        view.prune_status(JobStatusFilter::Completed);
        assert_eq!(view.rows.len(), 1);
        assert_eq!(view.rows[0].id, "j-failed");
        assert_eq!(view.sel, 0);
        assert_eq!(view.sel_id.as_deref(), Some("j-failed"));
        assert!(view.detail.is_some());
        let detail = view.detail.as_ref().unwrap();
        assert_eq!(detail.job.id, "j-failed");
        assert_eq!(detail.retryable, 1);
        assert!(
            view.can_retry(),
            "Retry button preserved on active remaining failed job"
        );
    }

    #[test]
    fn test_accept_snapshot_guards_detail() {
        use super::*;
        use argos_osint_core::jobs_view::{JobDetail, JobRow};

        let mut view = JobsView {
            rows: vec![JobRow {
                id: "j1".into(),
                state: "running".into(),
                ..Default::default()
            }],
            sel: 0,
            sel_id: Some("j1".into()),
            ..Default::default()
        };

        // Snapshot has detail for a different job
        let snapshot = JobsView {
            rows: vec![JobRow {
                id: "j1".into(),
                state: "running".into(),
                ..Default::default()
            }],
            sel: 0,
            sel_id: Some("j1".into()),
            detail: Some(JobDetail {
                job: JobRow {
                    id: "other-job".into(),
                    ..Default::default()
                },
                ..Default::default()
            }),
            loaded: true,
            ..Default::default()
        };

        view.accept(snapshot, false);
        assert_eq!(view.sel_id.as_deref(), Some("j1"));
        assert!(view.detail.is_some());
        assert_eq!(view.detail.as_ref().unwrap().job.id, "j1");
    }
}
