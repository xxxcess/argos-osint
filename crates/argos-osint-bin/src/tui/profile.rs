//! Profile dashboard: the Overview and System tabs.
//!
//! Overview renders the reviewed 35 widgets from [`ProfileSnapshot`], one section
//! at a time, through the single shared chart renderer in
//! [`crate::tui::profile_charts`]. System keeps Host, Paths and Logs exactly
//! where they were — the two tabs replace the old single System pane rather
//! than growing a second pane next to it.
//!
//! Nothing here renders a number it was not given: an unavailable metric is
//! `N/A`, never zero, and an empty window is an empty section rather than a
//! fabricated baseline. The snapshot is read from the store on a 1 Hz cadence
//! through its own connection, so a dashboard refresh never blocks recording.

use std::collections::HashMap;
use std::time::Instant;

use ratatui::layout::{Constraint, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

use argos_osint_core::profile_stats::{self, ProfileSnapshot, StatFilters};
use argos_osint_core::store::Store;

use super::app::App;
use super::profile_charts as charts;
use super::theme;

/// Which Profile tab is on screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SystemTab {
    /// Activity: the 35 telemetry widgets.
    #[default]
    Overview,
    /// Host, Paths, Logs.
    System,
}
impl SystemTab {
    pub fn label(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::System => "System",
        }
    }

    pub fn all() -> [Self; 2] {
        [Self::Overview, Self::System]
    }
}

/// One Overview section. Presentation priority order follows the spec: Intel,
/// Recon, Atlas, Models, Tools.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Section {
    Intel,
    Recon,
    Atlas,
    Models,
    Tools,
}

impl Section {
    pub fn all() -> [Self; 5] {
        [
            Self::Intel,
            Self::Recon,
            Self::Atlas,
            Self::Models,
            Self::Tools,
        ]
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Intel => "Intel",
            Self::Recon => "Recon",
            Self::Atlas => "Atlas",
            Self::Models => "Models",
            Self::Tools => "Tools",
        }
    }

    /// Widgets in this section, in the presentation priority order the spec
    /// fixes: lower priority first, and the registry order breaks a tie.
    pub fn widgets(self) -> Vec<&'static WidgetSpec> {
        let mut widgets: Vec<&'static WidgetSpec> =
            WIDGETS.iter().filter(|w| w.section == self).collect();
        widgets.sort_by_key(|widget| (widget.priority, widget.id));
        widgets
    }
}

/// One registered widget. The inventory is a single table so the count is
/// asserted, not estimated.
#[derive(Clone, Copy, Debug)]
pub struct WidgetSpec {
    /// Stable id, also the `see more` key.
    pub id: &'static str,
    pub section: Section,
    pub title: &'static str,
    /// Lower renders first inside a section.
    pub priority: u8,
}

macro_rules! widget {
    ($id:expr, $section:expr, $title:expr, $priority:expr) => {
        WidgetSpec {
            id: $id,
            section: $section,
            title: $title,
            priority: $priority,
        }
    };
}

/// Exactly the reviewed 35 widgets. A test asserts the count and the absence of
/// a duplicate id, so a new widget cannot slip in unannounced.
pub const WIDGETS: &[WidgetSpec] = &[
    // ---- Intel (7) ----
    widget!("intel.volume", Section::Intel, "Ingestion volume", 1),
    widget!(
        "intel.confidence",
        Section::Intel,
        "Initial vs current confidence",
        2
    ),
    widget!("intel.origins", Section::Intel, "Country / origin mix", 3),
    widget!("intel.enrichment", Section::Intel, "Tag enrichment", 4),
    widget!("intel.reports", Section::Intel, "Report outcomes", 5),
    widget!("intel.publishers", Section::Intel, "Publishers", 6),
    widget!("intel.freshness", Section::Intel, "Article freshness", 7),
    // ---- Recon (7) ----
    widget!("recon.outcomes", Section::Recon, "Run outcomes", 1),
    widget!("recon.stages", Section::Recon, "Stage durations", 2),
    widget!("recon.recall", Section::Recon, "Memory recall", 3),
    widget!("recon.workload", Section::Recon, "Workload per run", 4),
    widget!("recon.diversity", Section::Recon, "Tool diversity", 5),
    widget!(
        "recon.directives",
        Section::Recon,
        "Directive resolution",
        6
    ),
    widget!(
        "recon.unresolved",
        Section::Recon,
        "Unresolved directives",
        7
    ),
    // ---- Atlas (6) ----
    widget!("atlas.cycles", Section::Atlas, "Cycle outcomes", 1),
    widget!("atlas.hot_zones", Section::Atlas, "Hot zones", 2),
    widget!("atlas.temperature", Section::Atlas, "Temperature shifts", 3),
    widget!("atlas.cycle_time", Section::Atlas, "Cycle time", 4),
    widget!("atlas.discovery", Section::Atlas, "Discovery mix", 5),
    widget!("atlas.backlog", Section::Atlas, "Backlog", 6),
    // ---- Models (8) ----
    widget!("models.capacity", Section::Models, "Provider capacity", 1),
    widget!("models.by_role", Section::Models, "Requests by role", 2),
    widget!("models.latency", Section::Models, "Latency", 3),
    widget!("models.queue", Section::Models, "Queue delay", 4),
    widget!(
        "models.performance",
        Section::Models,
        "Provider performance",
        5
    ),
    widget!("models.fallback", Section::Models, "Fallback triggers", 6),
    widget!(
        "models.amplification",
        Section::Models,
        "Retry amplification",
        7
    ),
    widget!(
        "models.failures",
        Section::Models,
        "Model failure causes",
        8
    ),
    // ---- Tools (7) ----
    widget!("tools.usage", Section::Tools, "Tool usage", 1),
    widget!("tools.attribution", Section::Tools, "Attribution", 2),
    widget!("tools.outcomes", Section::Tools, "Tool outcomes", 3),
    widget!("tools.reliability", Section::Tools, "Reliability", 4),
    widget!("tools.search_health", Section::Tools, "Search health", 5),
    widget!("tools.evidence", Section::Tools, "Evidence contribution", 6),
    widget!("tools.failure_causes", Section::Tools, "Failure causes", 7),
];

/// The reviewed inventory size. Changing it is a spec change, not a tweak.
pub const WIDGET_COUNT: usize = 35;

#[cfg(test)]
mod registry_tests {
    use super::*;

    /// The inventory is a table, so the count is asserted rather than estimated.
    #[test]
    fn exactly_the_reviewed_widgets_are_registered() {
        assert_eq!(WIDGETS.len(), WIDGET_COUNT);
        assert_eq!(WIDGET_COUNT, 35);
    }

    #[test]
    fn every_widget_has_a_unique_stable_id_and_a_known_section() {
        let mut ids: Vec<&str> = WIDGETS.iter().map(|w| w.id).collect();
        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), before, "a duplicate widget id was registered");
        for widget in WIDGETS {
            assert!(!widget.title.is_empty(), "{} has no title", widget.id);
            assert!(
                Section::all().contains(&widget.section),
                "{} names an unknown section",
                widget.id
            );
        }
    }

    #[test]
    fn every_section_registers_its_reviewed_widget_count() {
        let counts: Vec<usize> = Section::all().iter().map(|s| s.widgets().len()).collect();
        // Intel 7, Recon 7, Atlas 6, Models 8, Tools 7.
        assert_eq!(counts, vec![7, 7, 6, 8, 7]);
        assert_eq!(counts.iter().sum::<usize>(), WIDGET_COUNT);
    }

    #[test]
    fn widgets_render_in_the_specs_presentation_priority_order() {
        for section in Section::all() {
            let mut last = 0u8;
            for widget in section.widgets() {
                assert!(
                    widget.priority >= last,
                    "{} breaks the priority order",
                    widget.id
                );
                last = widget.priority;
            }
        }
    }

    #[test]
    fn no_legacy_widget_survives_under_a_second_name() {
        for stale in [
            "system.hardware",
            "system.paths",
            "profile.hardware",
            "system.storage",
        ] {
            assert!(
                !WIDGETS.iter().any(|w| w.id == stale),
                "legacy widget {stale} is still registered"
            );
        }
    }
}

/// Dashboard state. One struct on `App`, mirroring `JobsView` and `LogsView`.
#[derive(Clone, Debug)]
pub struct ProfileView {
    pub tab: SystemTab,
    pub section: Section,
    /// Focused widget index inside the focused section.
    pub focus: usize,
    /// Rendered rows for the focused widget (`see more` grows it).
    pub pages: HashMap<&'static str, usize>,
    pub filters: StatFilters,
    pub snapshot: Option<ProfileSnapshot>,
    pub error: Option<String>,
    pub loaded_at: Option<Instant>,
    /// Filter popup: the dimension being edited, when the viewport is narrow.
    pub filter_popup: Option<&'static str>,
    pub filter_edit: String,
    /// The bounded option lists for every dimension.
    pub options: Vec<(String, Vec<String>)>,
    /// True while the Configs popup is open, so the module keys stand down.
    pub config_open: bool,
}

impl Default for ProfileView {
    fn default() -> Self {
        Self {
            tab: SystemTab::Overview,
            section: Section::Intel,
            focus: 0,
            pages: HashMap::new(),
            filters: StatFilters::default(),
            snapshot: None,
            error: None,
            loaded_at: None,
            filter_popup: None,
            filter_edit: String::new(),
            options: Vec::new(),
            config_open: false,
        }
    }
}

impl ProfileView {
    /// Focused section index, for the navigator.
    pub fn section_index(&self) -> usize {
        Section::all()
            .iter()
            .position(|section| *section == self.section)
            .unwrap_or(0)
    }

    pub fn focused_widgets(&self) -> Vec<&'static WidgetSpec> {
        self.section.widgets()
    }

    pub fn focused_widget(&self) -> Option<&'static WidgetSpec> {
        let widgets = self.focused_widgets();
        widgets
            .get(self.focus.min(widgets.len().saturating_sub(1)))
            .copied()
    }

    /// Page size for one widget; `see more` raises it.
    pub fn page(&self, id: &'static str) -> usize {
        self.pages.get(id).copied().unwrap_or(6)
    }

    pub fn see_more(&mut self, id: &'static str) {
        let next = (self.page(id) + 8).min(64);
        self.pages.insert(id, next);
    }

    /// Reads one snapshot. Read-only: the writer is never locked.
    pub fn reload(&mut self, store: &Store) {
        match store.profile_snapshot(&self.filters.period, &self.filters) {
            Ok(snapshot) => {
                self.snapshot = Some(snapshot);
                self.error = None;
                self.loaded_at = Some(Instant::now());
            }
            Err(err) => {
                // A failed read keeps the last good snapshot and says so.
                self.error = Some(format!("Could not read profile: {err:#}"));
            }
        }
        if self.options.is_empty() {
            self.options = store.profile_filter_options();
        }
    }

    /// 1 Hz cadence, throttled by the caller.
    pub fn due(&self) -> bool {
        match self.loaded_at {
            None => true,
            Some(at) => at.elapsed() >= std::time::Duration::from_secs(1),
        }
    }

    pub fn next_section(&mut self) {
        let all = Section::all();
        let next = (self.section_index() + 1) % all.len();
        self.section = all[next];
        self.focus = 0;
    }

    pub fn prev_section(&mut self) {
        let all = Section::all();
        let prev = (self.section_index() + all.len() - 1) % all.len();
        self.section = all[prev];
        self.focus = 0;
    }

    pub fn next_widget(&mut self) {
        let count = self.focused_widgets().len();
        if count > 0 {
            self.focus = (self.focus + 1) % count;
        }
    }

    pub fn prev_widget(&mut self) {
        let count = self.focused_widgets().len();
        if count > 0 {
            self.focus = (self.focus + count - 1) % count;
        }
    }

    /// Cycles a bounded dimension in the filter strip to its next value.
    pub fn cycle_filter(&mut self, dimension: &str) {
        let options = self
            .options
            .iter()
            .find(|(name, _)| name == dimension)
            .map(|(_, values)| values.clone())
            .unwrap_or_default();
        let current = self.filter_value(dimension);
        let mut choices = vec![String::new()];
        choices.extend(options);
        let next = choices
            .iter()
            .position(|value| value == &current)
            .map(|index| (index + 1) % choices.len())
            .unwrap_or(0);
        self.set_filter(dimension, &choices[next]);
    }

    pub fn filter_value(&self, dimension: &str) -> String {
        match dimension {
            "app" => self.filters.app.clone(),
            "provider" => self.filters.provider.clone(),
            "role" => self.filters.role.clone(),
            "mode" => self.filters.mode.clone(),
            "tool" => self.filters.tool.clone(),
            "category" => self.filters.category.clone(),
            _ => String::new(),
        }
    }

    pub fn set_filter(&mut self, dimension: &str, value: &str) {
        let value = value.trim().to_string();
        match dimension {
            "app" => self.filters.app = value,
            "provider" => self.filters.provider = value,
            "role" => self.filters.role = value,
            "mode" => self.filters.mode = value,
            "tool" => self.filters.tool = value,
            "category" => self.filters.category = value,
            _ => return,
        }
        // A changed filter invalidates the snapshot and the see-more pages.
        self.pages.clear();
        self.loaded_at = None;
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// Draws the Profile module: the tab strip, then the active tab's body.
pub fn draw_profile(frame: &mut Frame, app: &App, area: Rect) {
    let tab_area = strip(area, 0, 1);
    let body = strip(area, 1, area.height.saturating_sub(3));
    draw_tab_strip(frame, app, tab_area);
    match app.profile.tab {
        SystemTab::Overview => draw_overview(frame, app, body),
        SystemTab::System => draw_system_tab(frame, app, body),
    }
    draw_status_strip(frame, app, strip(area, area.height.saturating_sub(1), 1));
}

/// The System tab: Host and Paths, the panes this module always had. Logs stays
/// its own module with its own clear action and retention label; the two tabs
/// split the System pane, they do not take Logs over.
fn draw_system_tab(frame: &mut Frame, app: &App, area: Rect) {
    if area.height < 3 {
        return;
    }
    let (content, actions) = super::ui::system_areas(area);
    super::ui::draw_button(
        frame,
        app,
        super::app::ButtonId::RefreshHardware,
        "Refresh hardware",
        super::ui::system_button(actions),
    );
    let host = super::ui::system_host_lines(&app.hardware);
    let host_h = (host.len() as u16 + 2).min(content.height);
    let rows = rows(content, &[host_h, 0]);
    frame.render_widget(
        Paragraph::new(host.join("\n"))
            .style(theme::text())
            .block(super::ui::pane(" host "))
            .wrap(Wrap { trim: true }),
        rows[0],
    );
    frame.render_widget(
        Paragraph::new(super::ui::system_path_lines().join("\n"))
            .style(theme::text())
            .block(super::ui::pane(" paths "))
            .wrap(Wrap { trim: true }),
        rows[1],
    );
}

/// One row inside a frame body.
fn strip(area: Rect, top: u16, height: u16) -> Rect {
    Rect {
        x: area.x,
        y: area.y + top,
        width: area.width,
        height: height.min(area.height.saturating_sub(top)),
    }
}

/// Splits a body into rows of the given heights; the last height is a minimum,
/// so the body always fills the area.
fn rows(area: Rect, heights: &[u16]) -> Vec<Rect> {
    let total = area.height;
    let fixed: u16 = heights.iter().take(heights.len().saturating_sub(1)).sum();
    let mut out = Vec::with_capacity(heights.len());
    let mut y = area.y;
    for (index, height) in heights.iter().enumerate() {
        let height = if index + 1 == heights.len() {
            total.saturating_sub(fixed)
        } else {
            *height
        };
        out.push(Rect {
            x: area.x,
            y,
            width: area.width,
            height,
        });
        y += height;
    }
    out
}

fn draw_tab_strip(frame: &mut Frame, app: &App, area: Rect) {
    let mut spans = Vec::new();
    for (index, tab) in SystemTab::all().iter().enumerate() {
        let selected = app.profile.tab == *tab;
        let style = if selected {
            theme::selected()
        } else {
            theme::dim()
        };
        let marker = if selected { "▸" } else { " " };
        spans.push(Span::styled(format!("{marker} {} ", tab.label()), style));
        if index + 1 < SystemTab::all().len() {
            spans.push(Span::styled("│ ", theme::muted()));
        }
    }
    spans.push(Span::styled("   [Tab] switch  ", theme::muted()));
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_status_strip(frame: &mut Frame, app: &App, area: Rect) {
    if area.height == 0 {
        return;
    }
    let line = match app.profile.snapshot.as_ref() {
        Some(snapshot) => {
            let health = snapshot.status.collection_health.as_str();
            let health_style = match health {
                "ok" => theme::accent(),
                "stale" => theme::warn(),
                _ => theme::muted(),
            };
            let jobs = snapshot.status.active_jobs;
            let queued = if snapshot.status.provider_capacity_available {
                charts::count(snapshot.status.queued_requests as u64)
            } else {
                charts::unavailable().to_string()
            };
            Line::from(vec![
                Span::styled(health, health_style),
                Span::styled(" · jobs ", theme::muted()),
                Span::styled(charts::count(jobs as u64), theme::text()),
                Span::styled(" · queued ", theme::muted()),
                Span::styled(queued, theme::text()),
                Span::styled(" · widgets ", theme::muted()),
                Span::styled(charts::count(WIDGET_COUNT as u64), theme::text()),
                Span::styled(" · updated ", theme::muted()),
                Span::styled(snapshot.status.last_updated.clone(), theme::dim()),
            ])
        }
        None => Line::from(Span::styled(
            app.profile
                .error
                .clone()
                .unwrap_or_else(|| "collecting".to_string()),
            theme::muted(),
        )),
    };
    frame.render_widget(Paragraph::new(line), area);
}

fn draw_overview(frame: &mut Frame, app: &App, area: Rect) {
    if area.height < 4 || area.width < 20 {
        frame.render_widget(
            Paragraph::new("Profile needs a wider terminal")
                .style(theme::muted())
                .wrap(Wrap { trim: true }),
            area,
        );
        return;
    }
    let split = rows(area, &[1, 1, area.height.saturating_sub(2).max(3)]);
    let (filter_area, nav_area, body) = (split[0], split[1], split[2]);
    draw_filter_strip(frame, app, filter_area);
    draw_section_navigator(frame, app, nav_area);
    draw_section_body(frame, app, body);
}

fn draw_filter_strip(frame: &mut Frame, app: &App, area: Rect) {
    let mut spans = vec![
        Span::styled(" period ", theme::muted()),
        Span::styled(app.profile.filters.period.label(), theme::accent()),
    ];
    let mut narrow = area.width < 72;
    let dimensions = ["app", "provider", "role", "mode"];
    for dimension in dimensions {
        let value = app.profile.filter_value(dimension);
        let label = if value.trim().is_empty() {
            "any".to_string()
        } else {
            value.clone()
        };
        spans.push(Span::styled(format!(" · {dimension} "), theme::muted()));
        spans.push(Span::styled(label, theme::text()));
        if !narrow && value.trim().is_empty() {
            narrow = false;
        }
    }
    if narrow {
        // A narrow viewport offers the popup filters instead of the full strip.
        spans.push(Span::styled(" · [f] filters", theme::muted()));
    } else {
        spans.push(Span::styled(" · [c] clear", theme::muted()));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_section_navigator(frame: &mut Frame, app: &App, area: Rect) {
    let mut spans = Vec::new();
    for section in Section::all() {
        let selected = app.profile.section == section;
        let style = if selected {
            theme::selected()
        } else {
            theme::dim()
        };
        spans.push(Span::styled(format!(" {} ", section.label()), style));
        spans.push(Span::styled(" ", theme::muted()));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_section_body(frame: &mut Frame, app: &App, area: Rect) {
    let widgets = app.profile.focused_widgets();
    // Lazily rendered: only the focused widget's section is laid out, and the
    // unfocused widgets collapse to a one-line summary.
    let constraints: Vec<Constraint> = widgets
        .iter()
        .map(|widget| {
            let focused = app.profile.focused_widget().map(|f| f.id) == Some(widget.id);
            if focused {
                Constraint::Min(6)
            } else {
                Constraint::Length(1)
            }
        })
        .collect();
    let rows = ratatui::layout::Layout::default()
        .constraints(constraints)
        .split(area);
    for (index, widget) in widgets.iter().enumerate() {
        let row = rows[index];
        if row.height == 0 {
            continue;
        }
        let focused = app.profile.focused_widget().map(|f| f.id) == Some(widget.id);
        if focused {
            draw_widget_panel(frame, app, widget, row);
        } else {
            let marker = if index == app.profile.focus {
                "▸"
            } else {
                " "
            };
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(format!("{marker} "), theme::muted()),
                    Span::styled(widget.title.to_string(), theme::dim()),
                ])),
                row,
            );
        }
    }
}

fn draw_widget_panel(frame: &mut Frame, app: &App, widget: &WidgetSpec, area: Rect) {
    let block = theme::panel(&format!(" {} ", widget.title));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 || inner.width == 0 {
        return;
    }
    let lines = match app.profile.snapshot.as_ref() {
        Some(snapshot) => {
            let page = app.profile.page(widget.id);
            widget_lines(widget, snapshot, inner.width as usize, page)
        }
        None => match app.profile.error.as_ref() {
            Some(err) => vec![Line::from(Span::styled(err.clone(), theme::error()))],
            None => vec![Line::from(Span::styled("collecting", theme::muted()))],
        },
    };
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
}

/// The line list for one widget. Every widget renders through this single
/// entry point, so a new widget cannot smuggle in its own drawing rules.
fn widget_lines(
    widget: &WidgetSpec,
    snapshot: &ProfileSnapshot,
    width: usize,
    page: usize,
) -> Vec<Line<'static>> {
    let width = charts::bounded(width, 20, 240);
    let page = page.max(3);
    match widget.id {
        // ---- Intel ----
        "intel.volume" => volume_lines(&snapshot.intel.volume, width, page),
        "intel.confidence" => confidence_lines(&snapshot.intel.confidence, width),
        "intel.origins" => origin_lines(&snapshot.intel.origins, width, page),
        "intel.enrichment" => enrichment_lines(&snapshot.intel.enrichment, width, page),
        "intel.reports" => report_lines(&snapshot.intel.reports, width, page),
        "intel.publishers" => publisher_lines(&snapshot.intel.publishers, width, page),
        "intel.freshness" => freshness_lines(&snapshot.intel.freshness, width, page),
        // ---- Recon ----
        "recon.outcomes" => outcome_lines(&snapshot.recon.outcomes, width, page),
        "recon.stages" => stage_lines(&snapshot.recon.stages, width, page),
        "recon.recall" => recall_lines(&snapshot.recon.recall, width, page),
        "recon.workload" => workload_lines(&snapshot.recon.workload, width, page),
        "recon.diversity" => diversity_lines(&snapshot.recon.diversity, width, page),
        "recon.directives" => directive_lines(&snapshot.recon.directives, width, page),
        "recon.unresolved" => unresolved_lines(&snapshot.recon.unresolved, width, page),
        // ---- Atlas ----
        "atlas.cycles" => cycle_lines(&snapshot.atlas.cycle_outcomes, width, page),
        "atlas.hot_zones" => hot_zone_lines(&snapshot.atlas.origins, width, page),
        "atlas.temperature" => temperature_lines(&snapshot.atlas.temperature_changes, width, page),
        "atlas.cycle_time" => cycle_time_lines(&snapshot.atlas.cycle_times, width, page),
        "atlas.discovery" => discovery_lines(&snapshot.atlas.discovery, width, page),
        "atlas.backlog" => backlog_lines(&snapshot.atlas.backlog, width, page),
        // ---- Models ----
        "models.capacity" => capacity_lines(&snapshot.models, width, page),
        "models.by_role" => role_request_lines(&snapshot.models.by_role, width, page),
        "models.latency" => latency_lines(&snapshot.models.latency, width, page),
        "models.queue" => queue_lines(&snapshot.models.queue_delay, width, page),
        "models.performance" => performance_lines(&snapshot.models.performance, width, page),
        "models.fallback" => fallback_lines(&snapshot.models.fallback, width, page),
        "models.amplification" => amplification_lines(&snapshot.models.amplification, width, page),
        "models.failures" => model_failure_lines(&snapshot.models.failures, width, page),
        // ---- Tools ----
        "tools.usage" => tool_usage_lines(&snapshot.tools.top_tools, width, page),
        "tools.attribution" => {
            attribution_lines(&snapshot.tools.categories_by_trigger, width, page)
        }
        "tools.outcomes" => tool_outcome_lines(&snapshot.tools.outcomes, width, page),
        "tools.reliability" => reliability_lines(&snapshot.tools.reliability, width, page),
        "tools.search_health" => engine_lines(&snapshot.tools.engine_health, width, page),
        "tools.evidence" => evidence_lines(&snapshot.tools.evidence, width, page),
        "tools.failure_causes" => cause_lines(&snapshot.tools.failure_causes, width, page),
        other => vec![Line::from(Span::styled(
            format!("no widget {other}"),
            theme::muted(),
        ))],
    }
}

// ---------------------------------------------------------------------------
// Keys
// ---------------------------------------------------------------------------

use super::app::Target;
use crossterm::event::{KeyCode, KeyEvent};

/// Profile keys when no overlay is open. Returns true when the key was consumed.
pub fn handle_key(app: &mut App, key: KeyEvent) -> bool {
    let in_field = matches!(app.focus, Target::Field(_));
    match key.code {
        // Tab switches the Overview/System tabs, but only for a bare Tab: the
        // module-cycling Ctrl+Tab and the field-traversal Tab keep their meaning.
        KeyCode::Tab if key.modifiers.is_empty() && !in_field => {
            app.profile.tab = match app.profile.tab {
                SystemTab::Overview => SystemTab::System,
                SystemTab::System => SystemTab::Overview,
            };
            true
        }
        KeyCode::Char('1'..='5') if app.profile.tab == SystemTab::Overview && !in_field => {
            let index = match key.code {
                KeyCode::Char('1') => 0,
                KeyCode::Char('2') => 1,
                KeyCode::Char('3') => 2,
                KeyCode::Char('4') => 3,
                _ => 4,
            };
            app.profile.section = Section::all()[index];
            app.profile.focus = 0;
            true
        }
        KeyCode::Char('[') if app.profile.tab == SystemTab::Overview && !in_field => {
            app.profile.prev_section();
            true
        }
        KeyCode::Char(']') if app.profile.tab == SystemTab::Overview && !in_field => {
            app.profile.next_section();
            true
        }
        KeyCode::Char('j') | KeyCode::Down
            if app.profile.tab == SystemTab::Overview && !in_field =>
        {
            app.profile.next_widget();
            true
        }
        KeyCode::Char('k') | KeyCode::Up if app.profile.tab == SystemTab::Overview && !in_field => {
            app.profile.prev_widget();
            true
        }
        // `see more` grows the focused widget's page.
        KeyCode::Char('m') if app.profile.tab == SystemTab::Overview && !in_field => {
            if let Some(widget) = app.profile.focused_widget() {
                let id = widget.id;
                app.profile.see_more(id);
            }
            true
        }
        KeyCode::Char('r') if !in_field => {
            app.hardware = argos_osint_core::hardware::profile_cached(true);
            app.profile.loaded_at = None;
            app.status = "Hardware refreshed".into();
            true
        }
        KeyCode::Char('c') if !in_field => {
            app.profile.filters.clear();
            app.profile.loaded_at = None;
            true
        }
        KeyCode::Char('f') if !in_field => {
            // A narrow viewport offers the popup filters instead of the strip.
            let dimensions = ["app", "provider", "role", "mode", "tool", "category"];
            let section = app.profile.section_index().min(5);
            let current = app.profile.filter_popup;
            let next = match current {
                None => Some(dimensions[section]),
                Some(value) if value == dimensions[section] => None,
                Some(_) => Some("app"),
            };
            app.profile.filter_popup = next;
            if let Some(dimension) = next {
                app.profile.filter_edit = app.profile.filter_value(dimension);
            }
            true
        }
        // The filter popup cycles its dimension to the next observed value, so a
        // narrow viewport still reaches every bounded filter.
        KeyCode::Char('n') if app.profile.filter_popup.is_some() && !in_field => {
            if let Some(dimension) = app.profile.filter_popup {
                app.profile.cycle_filter(dimension);
                app.profile.filter_edit = app.profile.filter_value(dimension);
                app.profile.loaded_at = None;
            }
            true
        }
        KeyCode::Char('x') if !in_field => {
            app.profile_config.open();
            app.profile.config_open = true;
            app.overlay = super::app::Overlay::Configs;
            // The open tab owns its field from the first frame, so typing never
            // leaks into the module shortcuts underneath.
            app.set_focus(Target::Field(super::app::FieldId::ProfileExportPath));
            true
        }
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Widget line builders. Every one renders through `charts` — the single shared
// renderer — so no widget draws its own bars.
// ---------------------------------------------------------------------------

/// A labelled row with a trailing note column, used by every table widget.
fn table_row(cols: &[String], note: &str) -> Line<'static> {
    let mut spans = vec![Span::styled(cols.join("  "), theme::text())];
    if !note.is_empty() {
        spans.push(Span::styled(format!("  {note}"), theme::dim()));
    }
    Line::from(spans)
}

fn empty_note(note: &str) -> Vec<Line<'static>> {
    vec![Line::from(Span::styled(note.to_string(), theme::muted()))]
}

/// "n=12 (small sample)" when the denominator is too small for a percentile.
fn small_sample(n: u64) -> Option<String> {
    (n > 0 && n < profile_stats::SMALL_SAMPLE_MIN).then(|| format!("n={n} (small sample)"))
}

fn volume_lines(
    volume: &[profile_stats::VolumeBucket],
    width: usize,
    page: usize,
) -> Vec<Line<'static>> {
    if volume.is_empty() {
        return empty_note("no articles in this window");
    }
    let shown = &volume[volume.len().saturating_sub(page)..];
    let mut out = Vec::new();
    // The trend strip is the same shared renderer the other charts use.
    let totals: Vec<u64> = volume.iter().map(|b| b.total as u64).collect();
    let width = charts::bounded(width.saturating_sub(4), 8, 48);
    out.extend(charts::trend_columns(&totals, width, 4));
    for bucket in shown {
        let mut series: Vec<(String, u64)> = bucket
            .by_tag
            .iter()
            .map(|(tag, count)| (tag.clone(), *count as u64))
            .collect();
        if bucket.untagged > 0 {
            series.push(("untagged".to_string(), bucket.untagged as u64));
        }
        out.push(charts::stacked_bar(
            &bucket.bucket,
            10,
            &series,
            width.saturating_sub(14),
            bucket.total as u64,
        ));
        out.push(Line::from(Span::styled(
            format!("  {}", charts::count(bucket.total as u64)),
            theme::dim(),
        )));
    }
    out.push(Line::from(Span::styled(
        "tag totals may exceed distinct articles: a multi-tag article counts once per tag"
            .to_string(),
        theme::muted(),
    )));
    out
}

fn confidence_lines(
    distribution: &profile_stats::ConfidenceDistribution,
    width: usize,
) -> Vec<Line<'static>> {
    if distribution.bands.is_empty() {
        return empty_note("no paired confidence in this window");
    }
    let mut out = vec![Line::from(Span::styled(
        "band          initial  current".to_string(),
        theme::dim(),
    ))];
    for band in &distribution.bands {
        let initial = charts::rank_bar(band.initial as u64, 16, width.saturating_sub(34));
        let current = charts::rank_bar(band.current as u64, 16, width.saturating_sub(34));
        let mut row = vec![Span::styled(format!("{:<12}", band.label), theme::text())];
        row.extend(initial.spans);
        row.extend(current.spans);
        out.push(Line::from(row));
    }
    let mean = |value: Option<f64>| match value {
        Some(v) => format!("{v:.2}"),
        None => charts::unavailable().to_string(),
    };
    out.push(Line::from(vec![
        Span::styled("mean  ", theme::dim()),
        Span::styled(
            format!(
                "{}  {}",
                mean(distribution.mean_initial),
                mean(distribution.mean_current)
            ),
            theme::text(),
        ),
    ]));
    out.push(Line::from(Span::styled(
        format!(
            "{}  {}",
            charts::sample_size(distribution.paired_n),
            distribution.coverage_note
        ),
        theme::muted(),
    )));
    out
}

fn origin_lines(
    origins: &[profile_stats::OriginCount],
    width: usize,
    page: usize,
) -> Vec<Line<'static>> {
    if origins.is_empty() {
        return empty_note("no origins in this window");
    }
    let max = origins.first().map(|o| o.articles).unwrap_or(0).max(1);
    let mut out = Vec::new();
    for origin in origins.iter().take(page) {
        let label = if origin.is_unknown {
            format!("{} (unknown)", origin.origin)
        } else if origin.is_other {
            format!("{} (other)", origin.origin)
        } else {
            origin.origin.clone()
        };
        let mut row = vec![Span::styled(format!("{:<22}", label), theme::text())];
        row.extend(
            charts::rank_bar(origin.articles as u64, max as u64, width.saturating_sub(34)).spans,
        );
        row.push(Span::styled(
            format!(
                "  {}  {}",
                charts::count(origin.articles as u64),
                charts::percent(Some(origin.share))
            ),
            theme::dim(),
        ));
        out.push(Line::from(row));
    }
    out.push(Line::from(Span::styled(
        "source/collection origin, not subject geography".to_string(),
        theme::muted(),
    )));
    out
}

fn enrichment_lines(
    rows: &[profile_stats::EnrichmentRow],
    _width: usize,
    page: usize,
) -> Vec<Line<'static>> {
    if rows.is_empty() {
        return empty_note("no enriched tags in this window");
    }
    let mut out = vec![Line::from(Span::styled(
        "tag              articles  body  claims  reports  conf  rating".to_string(),
        theme::dim(),
    ))];
    for row in rows.iter().take(page) {
        out.push(table_row(
            &[
                format!("{:<16}", row.tag),
                format!("{:>8}", charts::count(row.articles as u64)),
                format!("{:>5}", charts::percent(row.body_pct)),
                format!("{:>7}", charts::percent(row.claims_pct)),
                format!("{:>8}", charts::percent(row.report_pct)),
                format!("{:>5}", charts::percent(row.mean_initial_confidence)),
                format!("{:>6}", charts::percent(row.mean_brief_rating)),
            ],
            "",
        ));
    }
    out
}

fn report_lines(
    rows: &[profile_stats::ReportModeRow],
    _width: usize,
    page: usize,
) -> Vec<Line<'static>> {
    if rows.is_empty() {
        return empty_note("no reports in this window");
    }
    let mut out = vec![Line::from(Span::styled(
        "mode      done partial failed waiting blocked   wall(mean/p95)       active".to_string(),
        theme::dim(),
    ))];
    for row in rows.iter().take(page) {
        out.push(table_row(
            &[
                format!("{:<9}", row.mode),
                format!("{:>4}", row.completed),
                format!("{:>7}", row.partial),
                format!("{:>6}", row.failed),
                format!("{:>7}", row.waiting),
                format!("{:>7}", row.blocked),
                format!(
                    "{:>9} /{:>8}",
                    charts::duration_ms(row.mean_wall_ms),
                    charts::duration_ms(row.p95_wall_ms)
                ),
                format!("{:>10}", charts::duration_ms(row.mean_active_ms)),
            ],
            charts::sample_size(row.n as u64).as_str(),
        ));
    }
    out.push(Line::from(Span::styled(
        "latest terminal revision per (article, mode)".to_string(),
        theme::muted(),
    )));
    out
}

fn publisher_lines(
    rows: &[profile_stats::PublisherCount],
    width: usize,
    page: usize,
) -> Vec<Line<'static>> {
    if rows.is_empty() {
        return empty_note("no publishers in this window");
    }
    let max = rows.first().map(|p| p.articles).unwrap_or(0).max(1);
    let mut out = Vec::new();
    for row in rows.iter().take(page) {
        let label = if row.is_unknown {
            format!("{} (unknown)", row.domain)
        } else if row.is_other {
            format!("{} (other)", row.domain)
        } else {
            row.domain.clone()
        };
        let mut spans = vec![Span::styled(format!("{:<24}", label), theme::text())];
        spans.extend(
            charts::rank_bar(row.articles as u64, max as u64, width.saturating_sub(40)).spans,
        );
        spans.push(Span::styled(
            format!(
                "  {}  {}",
                charts::count(row.articles as u64),
                charts::percent(Some(row.share))
            ),
            theme::dim(),
        ));
        out.push(Line::from(spans));
    }
    let concentration = rows.first().map(|r| r.top3_concentration).unwrap_or(0.0);
    out.push(Line::from(Span::styled(
        format!(
            "top-3 concentration {}",
            charts::percent(Some(concentration))
        ),
        theme::muted(),
    )));
    out
}

fn freshness_lines(
    histogram: &profile_stats::FreshnessHistogram,
    _width: usize,
    page: usize,
) -> Vec<Line<'static>> {
    if histogram.buckets.is_empty() {
        return empty_note("no published dates in this window");
    }
    let max = histogram
        .buckets
        .iter()
        .map(|(_, n)| *n)
        .max()
        .unwrap_or(0)
        .max(1);
    let mut out = Vec::new();
    for (label, count) in histogram.buckets.iter().take(page) {
        let mut row = vec![Span::styled(format!("{:<18}", label), theme::text())];
        row.extend(charts::rank_bar(*count as u64, max as u64, 20).spans);
        row.push(Span::styled(
            format!("  {}", charts::count(*count as u64)),
            theme::dim(),
        ));
        out.push(Line::from(row));
    }
    let missing = charts::count(histogram.missing as u64);
    out.push(Line::from(Span::styled(
        format!(
            "missing date {missing} · future {future}",
            future = histogram.future
        ),
        theme::muted(),
    )));
    out
}

fn outcome_lines(
    rows: &[profile_stats::ReconOutcomeBucket],
    width: usize,
    page: usize,
) -> Vec<Line<'static>> {
    if rows.is_empty() {
        return empty_note("no runs in this window");
    }
    let mut out = Vec::new();
    for row in rows.iter().take(page) {
        let series = vec![
            (
                "with evidence".to_string(),
                row.completed_with_evidence as u64,
            ),
            (
                "zero evidence".to_string(),
                row.completed_zero_evidence as u64,
            ),
            ("partial".to_string(), row.partial as u64),
            ("failed".to_string(), row.failed as u64),
            ("cancelled".to_string(), row.cancelled as u64),
        ];
        let total: u64 = series.iter().map(|(_, n)| *n).sum();
        out.push(charts::stacked_bar(
            &format!("{} {}", row.bucket, row.mode),
            14,
            &series,
            width.saturating_sub(18),
            total,
        ));
    }
    out.push(Line::from(Span::styled(
        "verse evidence is a separate outcome from a failed run".to_string(),
        theme::muted(),
    )));
    out
}

fn stage_lines(
    rows: &[profile_stats::StageDurationRow],
    _width: usize,
    page: usize,
) -> Vec<Line<'static>> {
    if rows.is_empty() {
        return empty_note("no stages in this window");
    }
    let mut out = vec![Line::from(Span::styled(
        "stage            mode    exec(mean)   wait(mean)      n".to_string(),
        theme::dim(),
    ))];
    for row in rows.iter().take(page) {
        out.push(table_row(
            &[
                format!("{:<15}", row.stage),
                format!("{:<7}", row.mode),
                format!("{:>12}", charts::duration_ms(row.mean_exec_ms)),
                format!("{:>12}", charts::duration_ms(row.mean_wait_ms)),
                format!("{:>6}", charts::sample_size(row.n as u64)),
            ],
            small_sample(row.n as u64).as_deref().unwrap_or(""),
        ));
    }
    out.push(Line::from(Span::styled(
        "stage attempts are separate attempts, not extra runs".to_string(),
        theme::muted(),
    )));
    out
}

fn recall_lines(
    rows: &[profile_stats::RecallRow],
    _width: usize,
    page: usize,
) -> Vec<Line<'static>> {
    if rows.is_empty() {
        return empty_note("no recall queries in this window");
    }
    let mut out = vec![Line::from(Span::styled(
        "mode     queries candidates accepted  hit rate    accepted/run  rejection".to_string(),
        theme::dim(),
    ))];
    for row in rows.iter().take(page) {
        out.push(table_row(
            &[
                format!("{:<8}", row.mode),
                format!("{:>7}", charts::count(row.queries as u64)),
                format!("{:>11}", charts::count(row.with_candidates as u64)),
                format!("{:>9}", charts::count(row.with_accepted as u64)),
                format!("{:>9}", charts::percent(row.retention_pct)),
                format!(
                    "{:>14}",
                    match row.accepted_per_run {
                        Some(v) => format!("{v:.2}"),
                        None => charts::unavailable().to_string(),
                    }
                ),
                format!("  {}", row.rejection_reason),
            ],
            "",
        ));
    }
    out.push(Line::from(Span::styled(
        "hit rate = queries with at least one accepted hit".to_string(),
        theme::muted(),
    )));
    out
}

fn workload_lines(
    rows: &[profile_stats::WorkloadRow],
    _width: usize,
    page: usize,
) -> Vec<Line<'static>> {
    if rows.is_empty() {
        return empty_note("no workload in this window");
    }
    let mut out = vec![Line::from(Span::styled(
        "mode      runs  directives/run  calls/run  categories/run  memories/run  wall(p50/p95)"
            .to_string(),
        theme::dim(),
    ))];
    for row in rows.iter().take(page) {
        out.push(table_row(
            &[
                format!("{:<9}", row.mode),
                format!("{:>5}", charts::count(row.runs as u64)),
                format!("{:>14}", format!("{:.2}", row.directives_per_run)),
                format!("{:>10}", format!("{:.2}", row.calls_per_run)),
                format!("{:>15}", format!("{:.2}", row.categories_per_run)),
                format!("{:>13}", format!("{:.2}", row.accepted_memories_per_run)),
                format!(
                    "{:>9} /{:>8}",
                    charts::duration_ms(row.median_wall_ms),
                    charts::duration_ms(row.p95_wall_ms)
                ),
            ],
            small_sample(row.runs as u64).as_deref().unwrap_or(""),
        ));
    }
    out
}

fn diversity_lines(
    rows: &[profile_stats::DiversityRow],
    _width: usize,
    page: usize,
) -> Vec<Line<'static>> {
    if rows.is_empty() {
        return empty_note("no coverage in this window");
    }
    let mut out = vec![Line::from(Span::styled(
        "category              eligible  ≥2 attempted  ≥2 succeeded   groups/scope  shortfall"
            .to_string(),
        theme::dim(),
    ))];
    for row in rows.iter().take(page) {
        out.push(table_row(
            &[
                format!("{:<20}", row.category),
                format!("{:>9}", charts::count(row.eligible_scopes as u64)),
                format!(
                    "{:>13}",
                    charts::count(row.scopes_with_two_attempted as u64)
                ),
                format!(
                    "{:>13}",
                    charts::count(row.scopes_with_two_successful as u64)
                ),
                format!(
                    "{:>14}",
                    match row.source_groups_per_scope {
                        Some(v) => format!("{v:.2}"),
                        None => charts::unavailable().to_string(),
                    }
                ),
                format!("  {}", row.top_shortfall_reason),
            ],
            if row.eligible { "eligible" } else { "" },
        ));
    }
    out.push(Line::from(Span::styled(
        "categories are categorical and cannot be averaged".to_string(),
        theme::muted(),
    )));
    out
}

fn directive_lines(
    rows: &[profile_stats::DirectiveResolution],
    _width: usize,
    page: usize,
) -> Vec<Line<'static>> {
    if rows.is_empty() {
        return empty_note("no directives in this window");
    }
    let mut out = vec![Line::from(Span::styled(
        "mode     answered partial unresolved blocked unknown      n".to_string(),
        theme::dim(),
    ))];
    for row in rows.iter().take(page) {
        out.push(table_row(
            &[
                format!("{:<8}", row.mode),
                format!("{:>8}", charts::count(row.answered as u64)),
                format!("{:>7}", charts::count(row.partial as u64)),
                format!("{:>10}", charts::count(row.unresolved as u64)),
                format!("{:>7}", charts::count(row.blocked as u64)),
                format!("{:>7}", charts::count(row.unknown as u64)),
                format!("{:>6}", charts::sample_size(row.n as u64)),
            ],
            "",
        ));
    }
    out.push(Line::from(Span::styled(
        "an assessment is never inferred from run completion".to_string(),
        theme::muted(),
    )));
    out
}

fn unresolved_lines(
    rows: &[profile_stats::UnresolvedDirective],
    _width: usize,
    page: usize,
) -> Vec<Line<'static>> {
    if rows.is_empty() {
        return empty_note("nothing unresolved in this window");
    }
    rows.iter()
        .take(page)
        .map(|row| {
            Line::from(Span::styled(
                format!(
                    "{} {} {} — {} ({} evidence) → {}",
                    row.run_id,
                    row.mode,
                    row.label,
                    row.reason,
                    row.evidence_count,
                    row.next_action
                ),
                theme::text(),
            ))
        })
        .collect()
}

fn cycle_lines(
    rows: &[profile_stats::CycleOutcomeBucket],
    width: usize,
    page: usize,
) -> Vec<Line<'static>> {
    if rows.is_empty() {
        return empty_note("no cycles in this window");
    }
    let mut out = Vec::new();
    for row in rows.iter().take(page) {
        let series = vec![
            ("completed".to_string(), row.completed as u64),
            ("partial".to_string(), row.partial as u64),
            ("failed".to_string(), row.failed as u64),
            ("cancelled".to_string(), row.cancelled as u64),
        ];
        let total: u64 = series.iter().map(|(_, n)| *n).sum();
        out.push(charts::stacked_bar(
            &row.bucket,
            10,
            &series,
            width.saturating_sub(14),
            total,
        ));
    }
    out.push(Line::from(Span::styled(
        "one disposition per candidate occurrence".to_string(),
        theme::muted(),
    )));
    out
}

fn hot_zone_lines(
    rows: &[profile_stats::OriginTierRow],
    _width: usize,
    page: usize,
) -> Vec<Line<'static>> {
    if rows.is_empty() {
        return empty_note("no origins in this window");
    }
    let mut out = vec![Line::from(Span::styled(
        "origin              articles  temp(latest)  temp(mean)  tier  t1   t2   t3".to_string(),
        theme::dim(),
    ))];
    for row in rows.iter().take(page) {
        out.push(table_row(
            &[
                format!("{:<18}", row.origin),
                format!("{:>8}", charts::count(row.articles as u64)),
                format!("{:>13}", charts::percent(row.latest_temperature)),
                format!("{:>11}", charts::percent(row.mean_temperature)),
                format!(
                    "{:>5}",
                    match row.latest_tier {
                        Some(tier) => tier.to_string(),
                        None => charts::unavailable().to_string(),
                    }
                ),
                format!("{:>4}", charts::percent(Some(row.tier1_share))),
                format!("{:>4}", charts::percent(Some(row.tier2_share))),
                format!("{:>4}", charts::percent(Some(row.tier3_share))),
            ],
            &if row.latest_at.is_empty() {
                String::new()
            } else {
                format!("snapshots {} · latest {}", row.snapshots, row.latest_at)
            },
        ));
    }
    out.push(Line::from(Span::styled(
        "temperature is Argos's score, not weather; tier is ordinal".to_string(),
        theme::muted(),
    )));
    out
}

fn temperature_lines(
    rows: &[profile_stats::TemperatureChange],
    _width: usize,
    page: usize,
) -> Vec<Line<'static>> {
    if rows.is_empty() {
        return empty_note("no comparable origins in this window");
    }
    let mut out = vec![Line::from(Span::styled(
        "origin              previous  current   delta     articles  movement".to_string(),
        theme::dim(),
    ))];
    for row in rows.iter().take(page) {
        out.push(table_row(
            &[
                format!("{:<18}", row.origin),
                format!("{:>9}", charts::percent(row.previous_temperature)),
                format!("{:>8}", charts::percent(row.current_temperature)),
                format!("{:>8}", charts::percent(row.delta)),
                format!("{:>9}", charts::count(row.articles as u64)),
                format!("  {}", row.label),
            ],
            if row.comparable { "" } else { "not comparable" },
        ));
    }
    out
}

fn cycle_time_lines(
    rows: &[profile_stats::CycleTimeBucket],
    _width: usize,
    page: usize,
) -> Vec<Line<'static>> {
    if rows.is_empty() {
        return empty_note("no cycle times in this window");
    }
    let mut out = vec![Line::from(Span::styled(
        "bucket      mean          p95           queue(mean)      n".to_string(),
        theme::dim(),
    ))];
    for row in rows.iter().take(page) {
        out.push(table_row(
            &[
                format!("{:<11}", row.bucket),
                format!("{:>13}", charts::duration_ms(row.mean_ms)),
                format!("{:>13}", charts::duration_ms(row.p95_ms)),
                format!("{:>13}", charts::duration_ms(row.mean_queue_ms)),
                format!("{:>6}", charts::sample_size(row.n as u64)),
            ],
            small_sample(row.n as u64).as_deref().unwrap_or(""),
        ));
    }
    out
}

fn discovery_lines(
    rows: &[profile_stats::DiscoveryBucket],
    _width: usize,
    page: usize,
) -> Vec<Line<'static>> {
    if rows.is_empty() {
        return empty_note("no discovery in this window");
    }
    let mut out = vec![Line::from(Span::styled(
        "bucket      fetched  new     existing  duplicates  rejected  pending".to_string(),
        theme::dim(),
    ))];
    for row in rows.iter().take(page) {
        out.push(table_row(
            &[
                format!("{:<11}", row.bucket),
                format!("{:>7}", charts::count(row.fetched as u64)),
                format!("{:>6}", charts::count(row.retained_new as u64)),
                format!("{:>9}", charts::count(row.retained_existing as u64)),
                format!("{:>11}", charts::count(row.duplicate_in_cycle as u64)),
                format!("{:>9}", charts::count(row.rejected as u64)),
                format!("{:>8}", charts::count(row.pending as u64)),
            ],
            "",
        ));
    }
    out
}

fn backlog_lines(
    rows: &[profile_stats::BacklogRow],
    _width: usize,
    page: usize,
) -> Vec<Line<'static>> {
    if rows.is_empty() {
        return empty_note("no backlog in this window");
    }
    let mut out = vec![Line::from(Span::styled(
        "stage      unit      queued  running  waiting  blocked  oldest wait  done  latest error"
            .to_string(),
        theme::dim(),
    ))];
    for row in rows.iter().take(page) {
        out.push(table_row(
            &[
                format!("{:<10}", row.stage),
                format!("{:<9}", row.unit),
                format!("{:>6}", charts::count(row.queued as u64)),
                format!("{:>8}", charts::count(row.running as u64)),
                format!("{:>8}", charts::count(row.waiting as u64)),
                format!("{:>8}", charts::count(row.blocked as u64)),
                format!("{:>13}", charts::duration_ms(row.oldest_pending_ms)),
                format!("{:>5}", charts::count(row.completed_in_period as u64)),
            ],
            &row.latest_error_category,
        ));
    }
    out
}

// ---------- Models (8) ----------

fn capacity_lines(
    models: &profile_stats::ModelStats,
    width: usize,
    page: usize,
) -> Vec<Line<'static>> {
    if !models.capacity_available {
        return empty_note("live provider capacity unavailable — the orchestration companion has not published a snapshot");
    }
    if models.capacity.is_empty() {
        return empty_note("no capacity rows in this window");
    }
    let mut out = vec![Line::from(Span::styled(
        "provider  group         scope     sends/60s  effective rpm  active  queued  cooldown"
            .to_string(),
        theme::dim(),
    ))];
    for row in models.capacity.iter().take(page) {
        let rpm = match row.effective_rpm {
            Some(value) => charts::count(value as u64),
            None => charts::unavailable().to_string(),
        };
        out.push(table_row(
            &[
                format!("{:<9}", row.provider),
                format!("{:<13}", row.quota_group),
                format!("{:<9}", row.scope),
                format!("{:>9}", charts::count(row.sends_60s as u64)),
                format!("{:>14}", rpm),
                format!("{:>7}", charts::count(row.active as u64)),
                format!("{:>7}", charts::count(row.queued as u64)),
                format!(
                    "{:>9}",
                    charts::duration_ms(row.cooldown_ms.map(|v| v as i64))
                ),
            ],
            &row.quota_source,
        ));
    }
    // The pace meter is the shared ratio renderer; an unknown pace is N/A.
    for row in models.capacity.iter().take(page) {
        let pace = row.pace_per_min.filter(|value| value.is_finite());
        let mut spans = vec![Span::styled(format!("{:<10}", row.provider), theme::text())];
        spans.extend(charts::meter(pace, width.saturating_sub(16)).spans);
        spans.push(Span::styled(
            format!("  {}", charts::percent(pace)),
            theme::dim(),
        ));
        out.push(Line::from(spans));
    }
    out.push(Line::from(Span::styled(
        "capacity ignores the historical filters: it is live state".to_string(),
        theme::muted(),
    )));
    out
}

fn role_request_lines(
    rows: &[profile_stats::RoleRequestBucket],
    width: usize,
    page: usize,
) -> Vec<Line<'static>> {
    if rows.is_empty() {
        return empty_note("no model requests in this window");
    }
    let mut out = Vec::new();
    for row in rows.iter().take(page) {
        out.push(charts::stacked_bar(
            &row.bucket,
            10,
            &row.by_role,
            width.saturating_sub(14),
            row.total,
        ));
    }
    out.push(Line::from(Span::styled(
        "sends are actual wire attempts; a logical operation may hold several".to_string(),
        theme::muted(),
    )));
    out
}

fn latency_lines(
    rows: &[profile_stats::LatencyBucket],
    _width: usize,
    page: usize,
) -> Vec<Line<'static>> {
    if rows.is_empty() {
        return empty_note("no finished attempts in this window");
    }
    let mut out = vec![Line::from(Span::styled(
        "bucket      p50(exec)  p95(exec)      n   p50 first header   p50 first content"
            .to_string(),
        theme::dim(),
    ))];
    for row in rows.iter().take(page) {
        out.push(table_row(
            &[
                format!("{:<11}", row.bucket),
                format!("{:>10}", charts::duration_ms(row.p50_ms)),
                format!("{:>10}", charts::duration_ms(row.p95_ms)),
                format!("{:>6}", charts::sample_size(row.n)),
                format!("{:>17}", charts::duration_ms(row.p50_first_header_ms)),
                format!("{:>19}", charts::duration_ms(row.p50_first_content_ms)),
            ],
            small_sample(row.n).as_deref().unwrap_or(""),
        ));
    }
    out
}

fn queue_lines(
    rows: &[profile_stats::QueueBucket],
    _width: usize,
    page: usize,
) -> Vec<Line<'static>> {
    if rows.is_empty() {
        return empty_note(
            "queue delay unavailable — the orchestration companion has not published queue timing",
        );
    }
    let mut out = vec![Line::from(Span::styled(
        "bucket      p50(queue)  p95(queue)      n".to_string(),
        theme::dim(),
    ))];
    for row in rows.iter().take(page) {
        out.push(table_row(
            &[
                format!("{:<11}", row.bucket),
                format!("{:>10}", charts::duration_ms(row.p50_ms)),
                format!("{:>12}", charts::duration_ms(row.p95_ms)),
                format!("{:>6}", charts::sample_size(row.n)),
            ],
            small_sample(row.n).as_deref().unwrap_or(""),
        ));
    }
    let missing = rows.iter().map(|row| row.n).sum::<u64>();
    let _ = missing;
    out.push(Line::from(Span::styled(
        "`wait_ms` is not queue residence: only measured queue timing counts".to_string(),
        theme::muted(),
    )));
    out
}

fn performance_lines(
    rows: &[profile_stats::ProviderModelRow],
    _width: usize,
    page: usize,
) -> Vec<Line<'static>> {
    if rows.is_empty() {
        return empty_note("no provider sends in this window");
    }
    let mut out = vec![Line::from(Span::styled(
        "provider  model               role       sends  done  attempt err  429  final op err  mean  p95".to_string(),
        theme::dim(),
    ))];
    for row in rows.iter().take(page) {
        let indent = if row.is_model_row { "  " } else { "" };
        out.push(table_row(
            &[
                format!("{indent}{:<9}", row.provider),
                format!("{:<18}", row.model),
                format!("{:<10}", row.role),
                format!("{:>6}", charts::count(row.sends)),
                format!("{:>5}", charts::count(row.completed_attempts)),
                format!("{:>12}", charts::percent(row.attempt_error_pct)),
                format!("{:>5}", charts::count(row.http_429)),
                format!("{:>13}", charts::percent(row.final_operation_failure_pct)),
                format!("{:>12}", charts::duration_ms(row.mean_exec_ms)),
                format!("{:>10}", charts::duration_ms(row.p95_exec_ms)),
            ],
            "",
        ));
    }
    out.push(Line::from(Span::styled(
        "attempt errors and final operation failures are different denominators".to_string(),
        theme::muted(),
    )));
    out
}

fn fallback_lines(
    rows: &[profile_stats::FallbackRow],
    _width: usize,
    page: usize,
) -> Vec<Line<'static>> {
    if rows.is_empty() {
        return empty_note("no fallback triggers in this window");
    }
    let mut out = vec![Line::from(Span::styled(
        "role      primary                 effective               reason  triggered  recovered  recovery".to_string(),
        theme::dim(),
    ))];
    for row in rows.iter().take(page) {
        out.push(table_row(
            &[
                format!("{:<9}", row.role),
                format!(
                    "{:<22}",
                    format!("{} {}", row.primary_provider, row.primary_model)
                ),
                format!(
                    "{:<22}",
                    format!("{} {}", row.effective_provider, row.effective_model)
                ),
                format!("{:<8}", row.trigger_reason),
                format!("{:>10}", charts::count(row.triggered_operations)),
                format!("{:>10}", charts::count(row.recovered_operations)),
                format!("{:>9}", charts::percent(row.recovery_pct)),
            ],
            &format!("recovered n={} failed n={}", row.recovered_n, row.failed_n),
        ));
    }
    out.push(Line::from(Span::styled(
        "a trigger is not a recovery: they are counted separately".to_string(),
        theme::muted(),
    )));
    out
}

fn amplification_lines(
    rows: &[profile_stats::AmplificationBucket],
    _width: usize,
    page: usize,
) -> Vec<Line<'static>> {
    if rows.is_empty() {
        return empty_note("no amplification in this window");
    }
    let mut out = vec![Line::from(Span::styled(
        "bucket      sends  terminal ops  ratio      in flight".to_string(),
        theme::dim(),
    ))];
    for row in rows.iter().take(page) {
        out.push(table_row(
            &[
                format!("{:<11}", row.bucket),
                format!("{:>6}", charts::count(row.sends)),
                format!("{:>13}", charts::count(row.terminal_operations)),
                format!("{:>10}", charts::percent(row.ratio)),
                format!("{:>10}", charts::count(row.in_flight_operations)),
            ],
            "",
        ));
    }
    out.push(Line::from(Span::styled(
        "sends per terminal operation; a retry raises it, it is not an error rate".to_string(),
        theme::muted(),
    )));
    out
}

fn model_failure_lines(
    rows: &[profile_stats::FailureBucket],
    width: usize,
    page: usize,
) -> Vec<Line<'static>> {
    if rows.is_empty() {
        return empty_note("no failed attempts in this window");
    }
    let mut out = Vec::new();
    for row in rows.iter().take(page) {
        let total: u64 = row
            .by_category
            .iter()
            .map(|(_, n)| *n)
            .sum::<u64>()
            .max(row.total_failed);
        out.push(charts::stacked_bar(
            &row.bucket,
            10,
            &row.by_category,
            width.saturating_sub(14),
            total,
        ));
        out.push(Line::from(Span::styled(
            format!(
                "  failed {} · {} of finished",
                charts::count(row.total_failed),
                charts::percent(row.pct_of_finished)
            ),
            theme::dim(),
        )));
    }
    out.push(Line::from(Span::styled(
        "an unfinished send is never a failure".to_string(),
        theme::muted(),
    )));
    out
}

// ---------- Tools (7) ----------

fn tool_usage_lines(
    rows: &[profile_stats::ToolCount],
    _width: usize,
    page: usize,
) -> Vec<Line<'static>> {
    if rows.is_empty() {
        return empty_note("no tool calls in this window");
    }
    let max = rows.iter().map(|r| r.invocations).max().unwrap_or(0).max(1);
    let mut out = vec![Line::from(Span::styled(
        "tool                       category           invocations   remote     local      cache"
            .to_string(),
        theme::dim(),
    ))];
    for row in rows.iter().take(page) {
        let mut spans = Vec::new();
        spans.push(Span::styled(format!("{:<26}", row.tool_id), theme::text()));
        spans.push(Span::styled(format!("{:<18}", row.category), theme::dim()));
        spans.push(Span::styled(
            format!("{:>12}", charts::count(row.invocations)),
            theme::text(),
        ));
        out.push(Line::from(spans));
        let _ = max;
        out.push(Line::from(Span::styled(
            format!(
                "  {}  remote {}  local {}  cache {}",
                charts::count(row.invocations),
                charts::count(row.remote),
                charts::count(row.local),
                charts::count(row.cache)
            ),
            theme::dim(),
        )));
    }
    out.push(Line::from(Span::styled(
        "a cache hit is not a remote request".to_string(),
        theme::muted(),
    )));
    out
}

fn attribution_lines(
    rows: &[profile_stats::CategoryTrigger],
    width: usize,
    page: usize,
) -> Vec<Line<'static>> {
    if rows.is_empty() {
        return empty_note("no attribution in this window");
    }
    let mut out = Vec::new();
    for row in rows.iter().take(page) {
        out.push(charts::stacked_bar(
            &row.category,
            14,
            &row.by_trigger,
            width.saturating_sub(18),
            row.total,
        ));
    }
    out.push(Line::from(Span::styled(
        "trigger is captured at the call, never inferred from prompt text".to_string(),
        theme::muted(),
    )));
    out
}

fn tool_outcome_lines(
    rows: &[profile_stats::ToolOutcomeBucket],
    width: usize,
    page: usize,
) -> Vec<Line<'static>> {
    if rows.is_empty() {
        return empty_note("no tool outcomes in this window");
    }
    let mut out = Vec::new();
    for row in rows.iter().take(page) {
        let series = vec![
            ("completed".to_string(), row.completed_nonempty),
            ("verified zero".to_string(), row.verified_zero),
            ("partial".to_string(), row.partial),
            ("failed".to_string(), row.failed),
            ("blocked".to_string(), row.blocked),
        ];
        let total: u64 = series.iter().map(|(_, n)| *n).sum();
        out.push(charts::stacked_bar(
            &row.bucket,
            10,
            &series,
            width.saturating_sub(14),
            total,
        ));
    }
    out.push(Line::from(Span::styled(
        "a failed transport is not zero results".to_string(),
        theme::muted(),
    )));
    out
}

fn reliability_lines(
    rows: &[profile_stats::ReliabilityRow],
    _width: usize,
    page: usize,
) -> Vec<Line<'static>> {
    if rows.is_empty() {
        return empty_note("no reliability data in this window");
    }
    let mut out = vec![Line::from(Span::styled(
        "tool                       invocations   requests   cache%   zero%   error%   mean     p95    trigger        mode".to_string(),
        theme::dim(),
    ))];
    for row in rows.iter().take(page) {
        out.push(table_row(
            &[
                format!("{:<26}", row.tool_id),
                format!("{:>12}", charts::count(row.invocations)),
                format!("{:>10}", charts::count(row.wire_requests)),
                format!("{:>8}", charts::percent(row.cache_hit_pct)),
                format!("{:>7}", charts::percent(row.verified_zero_pct)),
                format!("{:>8}", charts::percent(row.error_pct)),
                format!("{:>13}", charts::duration_ms(row.mean_ms)),
                format!("{:>13}", charts::duration_ms(row.p95_ms)),
                format!("{:>14}", row.dominant_trigger),
                format!("{:>9}", row.dominant_mode),
            ],
            "",
        ));
    }
    out.push(Line::from(Span::styled(
        "the remote error denominator excludes cache and local-only executions".to_string(),
        theme::muted(),
    )));
    out
}

fn engine_lines(
    rows: &[profile_stats::EngineHealthRow],
    _width: usize,
    page: usize,
) -> Vec<Line<'static>> {
    if rows.is_empty() {
        return empty_note("no named-engine fetches in this window");
    }
    let mut out = vec![Line::from(Span::styled(
        "engine     fetches  valid  zero  challenge  mismatch  transport  usable/fetch  cache  parser".to_string(),
        theme::dim(),
    ))];
    for row in rows.iter().take(page) {
        out.push(table_row(
            &[
                format!("{:<10}", row.engine),
                format!("{:>7}", charts::count(row.fetches)),
                format!("{:>6}", charts::count(row.valid_serps)),
                format!("{:>5}", charts::count(row.verified_zero)),
                format!("{:>9}", charts::count(row.challenge)),
                format!("{:>9}", charts::count(row.parser_mismatch)),
                format!("{:>10}", charts::count(row.transport_failure)),
                format!("{:>14}", charts::percent(row.usable_per_fetch)),
                format!("{:>6}", charts::count(row.cache_hits)),
                format!("{:>8}", row.parser_version),
            ],
            &if row.last_success.is_empty() {
                "no success yet".to_string()
            } else {
                format!("last success {}", row.last_success)
            },
        ));
    }
    out.push(Line::from(Span::styled(
        "engine identity is separate from the transport provider that fetches the SERP".to_string(),
        theme::muted(),
    )));
    out
}

fn evidence_lines(
    rows: &[profile_stats::EvidenceRow],
    _width: usize,
    page: usize,
) -> Vec<Line<'static>> {
    if rows.is_empty() {
        return empty_note("no evidence contribution in this window");
    }
    let mut out = vec![Line::from(Span::styled(
        "tool                       successful  with evidence  acceptance  distinct  cited"
            .to_string(),
        theme::dim(),
    ))];
    for row in rows.iter().take(page) {
        out.push(table_row(
            &[
                format!("{:<26}", row.tool_id),
                format!("{:>11}", charts::count(row.successful_nonempty)),
                format!("{:>14}", charts::count(row.with_evidence)),
                format!("{:>11}", charts::percent(row.acceptance_pct)),
                format!("{:>9}", charts::count(row.distinct_evidence)),
                format!("{:>6}", charts::count(row.cited_by_completed_reports)),
            ],
            if row.nonadditive {
                "one evidence item may credit more than one tool"
            } else {
                ""
            },
        ));
    }
    out
}

fn cause_lines(
    rows: &[profile_stats::FailureCauseRow],
    width: usize,
    page: usize,
) -> Vec<Line<'static>> {
    if rows.is_empty() {
        return empty_note("no failures in this window");
    }
    let max = rows.iter().map(|r| r.invocations).max().unwrap_or(0).max(1);
    let mut out = Vec::new();
    for row in rows.iter().take(page) {
        let mut spans = vec![Span::styled(
            format!("{rank:<2} ", rank = row.rank),
            theme::muted(),
        )];
        spans.push(Span::styled(format!("{:<26}", row.cause), theme::text()));
        spans.extend(charts::rank_bar(row.invocations, max, width.saturating_sub(44)).spans);
        spans.push(Span::styled(
            format!(
                "  {}  {}",
                charts::count(row.invocations),
                charts::percent(Some(row.share))
            ),
            theme::dim(),
        ));
        out.push(Line::from(spans));
    }
    out
}
