//! Logs dashboard: durable `argos_events` with independent filter, selection,
//! fold, scroll and live-follow state. Replaces the in-session System event log.

use std::collections::HashSet;

use argos_osint_core::events::{EventFilter, EventRow, NewEvent, Severity};
use argos_osint_core::jobs_view::EventCounts;
use argos_osint_core::store::Store;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use super::app::{App, ButtonId, FieldId, Target};
use super::theme;
use super::ui::{
    button_areas, contains, detail_rows, draw_button_state, draw_field, fit, inset, pane, ACTION_H,
    FIELD_H,
};

/// Bounded in-memory page; the table is the source of truth.
pub const PAGE: usize = 500;
/// Retention shown in the dashboard and applied by pruning.
pub const RETENTION_HOURS: i64 = argos_osint_core::events::DEFAULT_RETENTION_HOURS;

/// Minimum severity shown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LevelFilter {
    #[default]
    All,
    Info,
    Warn,
    Error,
}

impl LevelFilter {
    pub fn label(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Info => "info+",
            Self::Warn => "warn+",
            Self::Error => "errors",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::All => Self::Info,
            Self::Info => Self::Warn,
            Self::Warn => Self::Error,
            Self::Error => Self::All,
        }
    }

    fn severity(self) -> Option<Severity> {
        match self {
            Self::All => None,
            Self::Info => Some(Severity::Info),
            Self::Warn => Some(Severity::Warn),
            Self::Error => Some(Severity::Error),
        }
    }
}

#[derive(Debug)]
pub struct LogsView {
    /// Oldest first (newest at the bottom, where live follow keeps the selection).
    pub rows: Vec<EventRow>,
    pub sel: usize,
    sel_id: Option<String>,
    /// Expanded event ids.
    pub open: HashSet<String>,
    pub scroll: u16,
    pub follow: bool,
    pub level: LevelFilter,
    pub app: String,
    pub apps: Vec<String>,
    /// Job filter (matches the job and its descendants).
    pub job: String,
    pub search: String,
    pub counts: EventCounts,
    /// Last read failure. The previous rows stay visible.
    pub error: Option<String>,
    /// Return path when opened from Jobs ("View logs").
    pub back_to_job: Option<String>,
    pub loaded: bool,
}

impl Default for LogsView {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            sel: 0,
            sel_id: None,
            open: HashSet::new(),
            scroll: 0,
            follow: true,
            level: LevelFilter::All,
            app: String::new(),
            apps: Vec::new(),
            job: String::new(),
            search: String::new(),
            counts: EventCounts::default(),
            error: None,
            back_to_job: None,
            loaded: false,
        }
    }
}

impl LogsView {
    pub fn filter(&self) -> EventFilter {
        EventFilter {
            min_severity: self.level.severity(),
            app: self.app.clone(),
            job_id: self.job.clone(),
            text: self.search.trim().to_string(),
            ..Default::default()
        }
    }

    /// Reload the bounded page. Selection is kept by event id unless live
    /// follow is on; a read failure keeps the last good rows and says so.
    pub fn reload(&mut self, store: &Store) {
        match store.list_events(&self.filter(), PAGE) {
            Ok(mut rows) => {
                rows.reverse();
                self.rows = rows;
                self.error = None;
                self.loaded = true;
                let keep = self
                    .sel_id
                    .as_ref()
                    .and_then(|id| self.rows.iter().position(|row| &row.id == id));
                self.sel = match keep {
                    Some(index) if !self.follow => index,
                    _ if self.follow => self.rows.len().saturating_sub(1),
                    _ => self.sel.min(self.rows.len().saturating_sub(1)),
                };
                self.sel_id = self.rows.get(self.sel).map(|row| row.id.clone());
                let live: HashSet<&str> = self.rows.iter().map(|row| row.id.as_str()).collect();
                self.open.retain(|id| live.contains(id.as_str()));
            }
            Err(err) => self.error = Some(format!("Could not read events: {err:#}")),
        }
        self.refresh_counts(store);
        if let Ok(apps) = store.event_apps() {
            self.apps = apps;
        }
    }

    pub fn refresh_counts(&mut self, store: &Store) {
        if let Ok(counts) = store.event_counts() {
            self.counts = counts;
        }
    }

    pub fn selected(&self) -> Option<&EventRow> {
        self.rows.get(self.sel)
    }

    /// Manual selection. Moving off the newest row pauses live follow so
    /// incoming entries never move the user's place.
    pub fn select(&mut self, index: usize) {
        if self.rows.is_empty() {
            return;
        }
        self.sel = index.min(self.rows.len() - 1);
        self.sel_id = self.rows.get(self.sel).map(|row| row.id.clone());
        if self.sel + 1 < self.rows.len() {
            self.follow = false;
        }
    }

    pub fn move_by(&mut self, delta: i32) {
        if self.rows.is_empty() {
            return;
        }
        let last = self.rows.len() as i32 - 1;
        self.select((self.sel as i32 + delta).clamp(0, last) as usize);
    }

    pub fn toggle_open(&mut self) -> bool {
        let Some(row) = self.selected() else {
            return false;
        };
        if row.details.is_empty() {
            return false;
        }
        let id = row.id.clone();
        if !self.open.insert(id.clone()) {
            self.open.remove(&id);
        }
        true
    }

    pub fn cycle_app(&mut self) {
        let mut options = vec![String::new()];
        options.extend(self.apps.iter().cloned());
        let pos = options.iter().position(|a| *a == self.app).unwrap_or(0);
        self.app = options[(pos + 1) % options.len()].clone();
    }

    fn extra_rows(&self, row: &EventRow, width: usize) -> usize {
        if self.open.contains(&row.id) && !row.details.is_empty() {
            detail_rows(&row.details, width).len()
        } else {
            0
        }
    }

    pub fn line_count(&self, width: usize) -> usize {
        self.rows
            .iter()
            .map(|row| 1 + self.extra_rows(row, width))
            .sum()
    }

    pub fn entry_start(&self, index: usize, width: usize) -> usize {
        self.rows
            .iter()
            .take(index)
            .map(|row| 1 + self.extra_rows(row, width))
            .sum()
    }

    /// The entry painted at visual line `line` (after scroll).
    pub fn index_at_line(&self, line: usize, width: usize) -> Option<usize> {
        let mut cursor = 0usize;
        for (index, row) in self.rows.iter().enumerate() {
            let height = 1 + self.extra_rows(row, width);
            if line < cursor + height {
                return Some(index);
            }
            cursor += height;
        }
        None
    }

    /// Keep the selected entry (and its open detail) inside `room` lines.
    pub fn reveal(&mut self, room: usize, width: usize) {
        if self.rows.is_empty() {
            self.scroll = 0;
            return;
        }
        let start = self.entry_start(self.sel, width);
        let height = 1 + self.extra_rows(&self.rows[self.sel], width);
        let room = room.max(1);
        let top = self.scroll as usize;
        if start < top {
            self.scroll = start as u16;
        } else if start + height > top + room {
            self.scroll = (start + height).saturating_sub(room).min(start) as u16;
        }
    }

    pub fn scroll_max(&self, room: usize, width: usize) -> u16 {
        self.line_count(width).saturating_sub(room) as u16
    }
}

/// Which app wrote a session log line, from its wording. Background workers
/// set their app explicitly.
pub fn infer_app(text: &str) -> &'static str {
    let lower = text.trim().to_ascii_lowercase();
    let starts = |p: &str| lower.starts_with(p);
    if starts("atlas") {
        "atlas"
    } else if starts("recon") {
        "recon"
    } else if starts("intel") {
        "intel"
    } else if starts("osint") || starts("lookup") || starts("tool") {
        "tools"
    } else if starts("graph summary") || starts("memory") || starts("brain") {
        "brain"
    } else if starts("model") || starts("provider") || lower.contains("sign-in") {
        "models"
    } else {
        "argos"
    }
}

/// Session log line → durable event.
pub fn session_event(level: &str, text: &str, detail: &str) -> NewEvent {
    NewEvent {
        severity: Some(Severity::parse(level)),
        app: infer_app(text).into(),
        event_type: "tui.log".into(),
        message: text.into(),
        details: detail.into(),
        ..Default::default()
    }
}

pub fn local_time(ts: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(ts)
        .map(|t| {
            t.with_timezone(&chrono::Local)
                .format("%H:%M:%S")
                .to_string()
        })
        .unwrap_or_else(|_| ts.chars().take(8).collect())
}

pub struct LogsAreas {
    pub summary: Rect,
    pub search: Rect,
    pub actions: Rect,
    pub list: Rect,
}

pub fn areas(body: Rect) -> LogsAreas {
    let summary_h = 1.min(body.height);
    let search_h = FIELD_H.min(body.height.saturating_sub(summary_h));
    let actions_h = ACTION_H.min(body.height.saturating_sub(summary_h + search_h));
    let list_h = body.height.saturating_sub(summary_h + search_h + actions_h);
    let at = |offset: u16, height: u16| Rect {
        x: body.x,
        y: body.y.saturating_add(offset),
        width: body.width,
        height,
    };
    LogsAreas {
        summary: at(0, summary_h),
        search: at(summary_h, search_h),
        actions: at(summary_h + search_h, actions_h),
        list: at(summary_h + search_h + actions_h, list_h),
    }
}

/// Buttons on the action row, in paint and hit order.
pub fn buttons(view: &LogsView) -> Vec<ButtonId> {
    let mut out = vec![ButtonId::LogsLevel, ButtonId::LogsApp, ButtonId::LogsFollow];
    if view.selected().is_some_and(|row| !row.job_id.is_empty()) {
        out.push(ButtonId::LogsOpenJob);
    }
    if view.back_to_job.is_some() {
        out.push(ButtonId::LogsBack);
    }
    out.push(ButtonId::ClearLog);
    out
}

fn button_label(view: &LogsView, button: ButtonId) -> String {
    match button {
        ButtonId::LogsLevel => format!("Level: {}", view.level.label()),
        ButtonId::LogsApp => format!(
            "App: {}",
            if view.app.is_empty() {
                "all"
            } else {
                view.app.as_str()
            }
        ),
        ButtonId::LogsFollow => format!("Follow: {}", if view.follow { "on" } else { "off" }),
        ButtonId::LogsOpenJob => "Open job".into(),
        ButtonId::LogsBack => "Back to job".into(),
        ButtonId::ClearLog => "Clear events".into(),
        _ => String::new(),
    }
}

/// "1 error", "3 errors".
pub fn count(n: i64, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

pub fn summary_text(view: &LogsView) -> String {
    let c = &view.counts;
    let mut parts = vec![
        count(c.error, "error"),
        count(c.warn, "warning"),
        format!("{} info", c.info),
    ];
    if c.recent_failures > 0 {
        parts.push(format!(
            "{} in the last hour",
            count(c.recent_failures, "failure")
        ));
    }
    if !view.job.is_empty() {
        parts.push(format!("job {}", short_id(&view.job)));
    }
    parts.push(format!("kept {RETENTION_HOURS} h"));
    parts.join(" · ")
}

pub fn short_id(id: &str) -> String {
    let tail: String = id
        .chars()
        .rev()
        .take(8)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    if id.chars().count() > 8 {
        format!("…{tail}")
    } else {
        tail
    }
}

pub fn row_lines(view: &LogsView, width: usize) -> Vec<Line<'static>> {
    let width = width.max(1);
    let mut lines = Vec::new();
    for (index, row) in view.rows.iter().enumerate() {
        let style = if index == view.sel {
            theme::selected()
        } else {
            match row.severity.as_str() {
                "error" => theme::error(),
                "warn" => theme::warn(),
                _ => theme::dim(),
            }
        };
        let marker = if row.details.is_empty() {
            "  "
        } else if view.open.contains(&row.id) {
            "▾ "
        } else {
            "▸ "
        };
        let job = if row.job_id.is_empty() {
            String::new()
        } else {
            format!("[{}] ", short_id(&row.job_id))
        };
        let text = format!(
            "{marker}{} {:<5} {:<6} {job}{}",
            local_time(&row.ts),
            row.severity,
            row.app,
            row.message
        );
        lines.push(Line::from(Span::styled(fit(&text, width), style)));
        if view.open.contains(&row.id) && !row.details.is_empty() {
            for line in detail_rows(&row.details, width) {
                lines.push(Line::from(Span::styled(line, theme::text())));
            }
        }
    }
    lines
}

pub fn draw(frame: &mut Frame, app: &App, body: Rect) {
    let view = &app.logs;
    let a = areas(body);
    frame.render_widget(
        Paragraph::new(fit(&summary_text(view), a.summary.width as usize)).style(
            if view.counts.error > 0 {
                theme::warn()
            } else {
                theme::dim()
            },
        ),
        a.summary,
    );
    draw_field(frame, app, FieldId::LogsSearch, "filter", a.search);
    let ids = buttons(view);
    for (button, rect) in ids.iter().zip(button_areas(a.actions, ids.len())) {
        let active = *button == ButtonId::LogsFollow && view.follow;
        draw_button_state(
            frame,
            app,
            *button,
            &button_label(view, *button),
            rect,
            active,
        );
    }
    let width = inset(a.list).width as usize;
    let mut lines = Vec::new();
    if let Some(err) = &view.error {
        lines.push(Line::from(Span::styled(
            fit(&format!("{err} (showing the last page read)"), width),
            theme::error(),
        )));
    }
    if view.rows.is_empty() {
        let text = if !view.loaded && view.error.is_none() {
            "Loading events…".to_string()
        } else if view.filter().min_severity.is_some()
            || !view.app.is_empty()
            || !view.job.is_empty()
            || !view.search.trim().is_empty()
        {
            "No events match these filters.".to_string()
        } else {
            format!(
                "No events in the last {RETENTION_HOURS} hours. Failures and run stages from every app are recorded here."
            )
        };
        lines.push(Line::from(Span::styled(text, theme::dim())));
    } else {
        lines.extend(row_lines(view, width));
    }
    let title = if view.job.is_empty() {
        " events ".to_string()
    } else {
        format!(" events · job {} and descendants ", short_id(&view.job))
    };
    let scroll = if view.error.is_some() { 0 } else { view.scroll };
    frame.render_widget(
        Paragraph::new(lines)
            .block(pane(&title))
            .scroll((scroll, 0)),
        a.list,
    );
}

pub fn hit(app: &App, body: Rect, x: u16, y: u16) -> Option<Target> {
    let a = areas(body);
    if contains(a.search, x, y) {
        return Some(Target::Field(FieldId::LogsSearch));
    }
    if contains(a.actions, x, y) {
        let ids = buttons(&app.logs);
        return ids
            .iter()
            .zip(button_areas(a.actions, ids.len()))
            .find(|(_, rect)| contains(*rect, x, y))
            .map(|(button, _)| Target::Button(*button));
    }
    let inner = inset(a.list);
    if app.logs.error.is_some() || !contains(inner, x, y) {
        return None;
    }
    let line = (y - inner.y) as usize + app.logs.scroll as usize;
    app.logs
        .index_at_line(line, inner.width as usize)
        .map(Target::LogLine)
}

/// List room and width for the current screen (scroll/reveal maths).
pub fn list_geometry(body: Rect) -> (usize, usize) {
    let inner = inset(areas(body).list);
    (inner.height.max(1) as usize, inner.width.max(1) as usize)
}

pub fn in_list(body: Rect, x: u16, y: u16) -> bool {
    contains(areas(body).list, x, y)
}
