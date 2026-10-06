//! Jobs dashboard: a bounded, filterable view over durable jobs/tasks/attempts
//! with its own selection, filter and scroll state.

use argos_osint_core::jobs_view::{
    format_duration, JobCounts, JobDetail, JobFilter, JobRow, JobStatusFilter,
};
use argos_osint_core::store::Store;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

use super::app::{App, ButtonId, FieldId, Target};
use super::logs::local_time;
use super::theme;
use super::ui::{
    button_areas, contains, draw_button_state, draw_field, fit, inset, pane, ACTION_H, FIELD_H,
};

pub const PAGE: usize = 200;
/// Below this body width the table and detail share one panel.
pub const WIDE: u16 = 100;

#[derive(Debug, Default)]
pub struct JobsView {
    pub rows: Vec<JobRow>,
    pub sel: usize,
    sel_id: Option<String>,
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

    /// Reload rows and the selected job's detail, keeping the selection by id.
    pub fn reload(&mut self, store: &Store) {
        match store.list_jobs(&self.filter(), PAGE) {
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
        if let Ok(counts) = store.job_counts() {
            self.counts = counts;
        }
        if let Ok(apps) = store.job_apps() {
            self.apps = apps;
        }
        self.load_detail(store);
    }

    pub fn load_detail(&mut self, store: &Store) {
        let Some(id) = self.selected().map(|row| row.id.clone()) else {
            self.detail = None;
            return;
        };
        match store.job_detail(&id, 40) {
            Ok(detail) => self.detail = detail,
            Err(err) => self.error = Some(format!("Could not read job {id}: {err:#}")),
        }
    }

    pub fn selected(&self) -> Option<&JobRow> {
        self.rows.get(self.sel)
    }

    pub fn select(&mut self, index: usize, store: &Store) {
        if self.rows.is_empty() {
            return;
        }
        let index = index.min(self.rows.len() - 1);
        if index != self.sel {
            self.detail_scroll = 0;
        }
        self.sel = index;
        self.sel_id = self.rows.get(index).map(|row| row.id.clone());
        self.load_detail(store);
    }

    pub fn move_by(&mut self, delta: i32, store: &Store) {
        if self.rows.is_empty() {
            return;
        }
        let last = self.rows.len() as i32 - 1;
        self.select((self.sel as i32 + delta).clamp(0, last) as usize, store);
    }

    /// Log → job navigation: select the job's top-level ancestor, clearing
    /// filters only when they hide it. Returns false when it no longer exists.
    pub fn focus_job(&mut self, store: &Store, id: &str) -> bool {
        let mut target = id.to_string();
        for _ in 0..8 {
            match store.get_job(&target) {
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
        self.detail.as_ref().is_some_and(|d| d.retryable > 0)
    }
}

pub struct JobsAreas {
    pub summary: Rect,
    pub search: Rect,
    pub actions: Rect,
    /// Zero-sized when the narrow detail replaces it.
    pub table: Rect,
    /// Zero-sized when narrow and the detail is closed.
    pub detail: Rect,
}

pub fn areas(body: Rect, view: &JobsView) -> JobsAreas {
    let summary_h = 1.min(body.height);
    let search_h = FIELD_H.min(body.height.saturating_sub(summary_h));
    let actions_h = ACTION_H.min(body.height.saturating_sub(summary_h + search_h));
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
        table,
        detail,
    }
}

/// Action buttons in paint/hit order. Retry and Open source appear only when
/// they are truthful for the selected job.
pub fn buttons(view: &JobsView, can_open_source: bool) -> Vec<ButtonId> {
    let mut out = vec![ButtonId::JobsStatus, ButtonId::JobsApp];
    if view.selected().is_some() {
        out.push(ButtonId::JobsViewLogs);
    }
    if view.can_retry() {
        out.push(ButtonId::JobsRetry);
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
        ButtonId::JobsRetry => "Retry failed".into(),
        ButtonId::JobsOpenSource => "Open source".into(),
        _ => String::new(),
    }
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
        let attempts = if job.attempt_cap > 0 {
            format!("{}/{}", job.attempts_used, job.attempt_cap)
        } else {
            job.attempts_used.to_string()
        };
        let text = cols.row(
            job.state_label(),
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
    lines.push(format!(
        "Attempts {}{}",
        job.attempts_used,
        if job.attempt_cap > 0 {
            format!(" of {}", job.attempt_cap)
        } else {
            String::new()
        }
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
        for child in &detail.children {
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
        for task in &detail.tasks {
            let mut line = format!(
                "  {} {} · {}/{} attempts",
                task.state, task.operation, task.attempts, task.max_attempts
            );
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
        for attempt in &detail.attempts {
            let mut line = format!(
                "  #{} {} {} {}",
                attempt.attempt_number,
                stamp(&attempt.started_at),
                if attempt.outcome.is_empty() {
                    "running"
                } else {
                    attempt.outcome.as_str()
                },
                format_duration(attempt.duration_ms)
            );
            if !attempt.error_message.is_empty() {
                line.push_str(&format!(" · {}", attempt.error_message));
            }
            lines.push(line);
        }
    }
    lines
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
    for (button, rect) in ids.iter().zip(button_areas(a.actions, ids.len())) {
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
        if view.rows.is_empty() {
            let text = if !view.loaded && view.error.is_none() {
                "Loading jobs…"
            } else if view.status != JobStatusFilter::All
                || !view.app.is_empty()
                || !view.search.trim().is_empty()
            {
                "No jobs match these filters."
            } else {
                "No background work recorded yet."
            };
            lines.push(Line::from(Span::styled(text, theme::dim())));
        } else {
            lines.extend(table_lines(view, width, now));
        }
        frame.render_widget(
            Paragraph::new(lines)
                .block(pane(" jobs "))
                .scroll((view.scroll, 0)),
            a.table,
        );
    }
    if a.detail.width > 0 && a.detail.height > 0 {
        let text = match &view.detail {
            Some(detail) => detail_lines(detail, now).join("\n"),
            None => "Select a job to see its timing, tasks, and attempts.".into(),
        };
        let focused = app.focus == Target::JobDetail;
        frame.render_widget(
            Paragraph::new(text)
                .style(theme::text())
                .block(pane(if focused {
                    " detail · focused "
                } else {
                    " detail "
                }))
                .wrap(Wrap { trim: false })
                .scroll((view.detail_scroll, 0)),
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
    let index = (y - inner.y - 1) as usize + view.scroll as usize;
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
            .zip(button_areas(a.actions, ids.len()))
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
    inset(areas(body, view).table)
        .height
        .saturating_sub(1)
        .max(1) as usize
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
}
