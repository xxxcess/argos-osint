//! Profile dashboard: the Overview and System tabs.
//!
//! Overview renders all 35 stable metrics as aligned cards and scrollable reports.
//! System exposes Host and Paths; Configs retains its existing import/export flow.
//! Rendering consumes immutable snapshots delivered by one background read at a
//! time. Missing history, measured zero, empty windows and stale errors remain
//! distinct, and exact values take precedence over decorative plot labels.

use std::collections::HashMap;
use std::time::Instant;

use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use argos_osint_core::profile_stats::{self, ProfileSnapshot, StatFilters};
#[cfg(test)]
use argos_osint_core::store::Store;

use super::app::App;
use super::components::{self, ScrollPane};
use super::profile_charts as charts;
use super::profile_layout::{AnalyticsLayout, ReportLayout};
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
    pub renderer: RendererKind,
    pub unit: &'static str,
    pub series_keys: &'static [&'static str],
    pub applicable_filters: &'static [&'static str],
    pub detail_columns: &'static [&'static str],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RendererKind {
    Time,
    Ranked,
    Comparison,
    Meter,
    Table,
}
const fn same_id(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}
const fn renderer_kind(id: &str) -> RendererKind {
    if same_id(id, "intel.volume") {
        return RendererKind::Time;
    }
    if same_id(id, "intel.confidence") {
        return RendererKind::Comparison;
    }
    if same_id(id, "intel.origins") {
        return RendererKind::Ranked;
    }
    if same_id(id, "intel.enrichment") {
        return RendererKind::Table;
    }
    if same_id(id, "intel.reports") {
        return RendererKind::Table;
    }
    if same_id(id, "intel.publishers") {
        return RendererKind::Ranked;
    }
    if same_id(id, "intel.freshness") {
        return RendererKind::Ranked;
    }
    if same_id(id, "recon.outcomes") {
        return RendererKind::Time;
    }
    if same_id(id, "recon.stages") {
        return RendererKind::Comparison;
    }
    if same_id(id, "recon.recall") {
        return RendererKind::Meter;
    }
    if same_id(id, "recon.workload") {
        return RendererKind::Table;
    }
    if same_id(id, "recon.diversity") {
        return RendererKind::Meter;
    }
    if same_id(id, "recon.directives") {
        return RendererKind::Meter;
    }
    if same_id(id, "recon.unresolved") {
        return RendererKind::Table;
    }
    if same_id(id, "atlas.cycles") {
        return RendererKind::Time;
    }
    if same_id(id, "atlas.hot_zones") {
        return RendererKind::Ranked;
    }
    if same_id(id, "atlas.temperature") {
        return RendererKind::Comparison;
    }
    if same_id(id, "atlas.cycle_time") {
        return RendererKind::Time;
    }
    if same_id(id, "atlas.discovery") {
        return RendererKind::Time;
    }
    if same_id(id, "atlas.backlog") {
        return RendererKind::Table;
    }
    if same_id(id, "models.capacity") {
        return RendererKind::Meter;
    }
    if same_id(id, "models.by_role") {
        return RendererKind::Time;
    }
    if same_id(id, "models.latency") {
        return RendererKind::Time;
    }
    if same_id(id, "models.queue") {
        return RendererKind::Time;
    }
    if same_id(id, "models.performance") {
        return RendererKind::Table;
    }
    if same_id(id, "models.fallback") {
        return RendererKind::Table;
    }
    if same_id(id, "models.amplification") {
        return RendererKind::Time;
    }
    if same_id(id, "models.failures") {
        return RendererKind::Time;
    }
    if same_id(id, "tools.usage") {
        return RendererKind::Ranked;
    }
    if same_id(id, "tools.attribution") {
        return RendererKind::Ranked;
    }
    if same_id(id, "tools.outcomes") {
        return RendererKind::Time;
    }
    if same_id(id, "tools.reliability") {
        return RendererKind::Table;
    }
    if same_id(id, "tools.search_health") {
        return RendererKind::Table;
    }
    if same_id(id, "tools.evidence") {
        return RendererKind::Meter;
    }
    if same_id(id, "tools.failure_causes") {
        return RendererKind::Ranked;
    }
    RendererKind::Table
}
const fn widget_unit(id: &str) -> &'static str {
    if same_id(id, "intel.volume") {
        return "articles";
    }
    if same_id(id, "intel.confidence") {
        return "confidence (0–1)";
    }
    if same_id(id, "intel.origins") {
        return "articles";
    }
    if same_id(id, "intel.enrichment") {
        return "articles / coverage";
    }
    if same_id(id, "intel.reports") {
        return "reports / ms";
    }
    if same_id(id, "intel.publishers") {
        return "articles";
    }
    if same_id(id, "intel.freshness") {
        return "articles / delay";
    }
    if same_id(id, "recon.outcomes") {
        return "terminal runs";
    }
    if same_id(id, "recon.stages") {
        return "ms";
    }
    if same_id(id, "recon.recall") {
        return "queries / memories";
    }
    if same_id(id, "recon.workload") {
        return "runs / ms";
    }
    if same_id(id, "recon.diversity") {
        return "scopes";
    }
    if same_id(id, "recon.directives") {
        return "assessments";
    }
    if same_id(id, "recon.unresolved") {
        return "directives";
    }
    if same_id(id, "atlas.cycles") {
        return "terminal cycles";
    }
    if same_id(id, "atlas.hot_zones") {
        return "articles / temperature";
    }
    if same_id(id, "atlas.temperature") {
        return "temperature change";
    }
    if same_id(id, "atlas.cycle_time") {
        return "ms";
    }
    if same_id(id, "atlas.discovery") {
        return "candidate occurrences";
    }
    if same_id(id, "atlas.backlog") {
        return "Live units";
    }
    if same_id(id, "models.capacity") {
        return "Live requests / minute";
    }
    if same_id(id, "models.by_role") {
        return "wire sends";
    }
    if same_id(id, "models.latency") {
        return "ms";
    }
    if same_id(id, "models.queue") {
        return "ms";
    }
    if same_id(id, "models.performance") {
        return "attempts / operations";
    }
    if same_id(id, "models.fallback") {
        return "operations / ms";
    }
    if same_id(id, "models.amplification") {
        return "sends / terminal operation";
    }
    if same_id(id, "models.failures") {
        return "failed wire attempts";
    }
    if same_id(id, "tools.usage") {
        return "invocations";
    }
    if same_id(id, "tools.attribution") {
        return "invocations";
    }
    if same_id(id, "tools.outcomes") {
        return "invocations";
    }
    if same_id(id, "tools.reliability") {
        return "invocations / requests / ms";
    }
    if same_id(id, "tools.search_health") {
        return "fetches / results";
    }
    if same_id(id, "tools.evidence") {
        return "invocations / evidence";
    }
    if same_id(id, "tools.failure_causes") {
        return "failed invocations";
    }
    "values"
}
const fn series_keys(id: &str) -> &'static [&'static str] {
    if same_id(id, "intel.volume") {
        return &["total"];
    }
    if same_id(id, "intel.confidence") {
        return &[];
    }
    if same_id(id, "intel.origins") {
        return &[];
    }
    if same_id(id, "intel.enrichment") {
        return &[];
    }
    if same_id(id, "intel.reports") {
        return &[];
    }
    if same_id(id, "intel.publishers") {
        return &[];
    }
    if same_id(id, "intel.freshness") {
        return &[];
    }
    if same_id(id, "recon.outcomes") {
        return &[
            "completed_with_evidence",
            "completed_zero_evidence",
            "partial",
            "failed",
            "cancelled",
        ];
    }
    if same_id(id, "recon.stages") {
        return &[];
    }
    if same_id(id, "recon.recall") {
        return &[];
    }
    if same_id(id, "recon.workload") {
        return &[];
    }
    if same_id(id, "recon.diversity") {
        return &[];
    }
    if same_id(id, "recon.directives") {
        return &[];
    }
    if same_id(id, "recon.unresolved") {
        return &[];
    }
    if same_id(id, "atlas.cycles") {
        return &["completed", "partial", "failed", "cancelled"];
    }
    if same_id(id, "atlas.hot_zones") {
        return &[];
    }
    if same_id(id, "atlas.temperature") {
        return &[];
    }
    if same_id(id, "atlas.cycle_time") {
        return &[];
    }
    if same_id(id, "atlas.discovery") {
        return &[
            "retained_new",
            "retained_existing",
            "duplicate",
            "rejected",
            "pending",
        ];
    }
    if same_id(id, "atlas.backlog") {
        return &[];
    }
    if same_id(id, "models.capacity") {
        return &[];
    }
    if same_id(id, "models.by_role") {
        return &["role"];
    }
    if same_id(id, "models.latency") {
        return &[];
    }
    if same_id(id, "models.queue") {
        return &[];
    }
    if same_id(id, "models.performance") {
        return &[];
    }
    if same_id(id, "models.fallback") {
        return &[];
    }
    if same_id(id, "models.amplification") {
        return &[];
    }
    if same_id(id, "models.failures") {
        return &["failure_category"];
    }
    if same_id(id, "tools.usage") {
        return &[];
    }
    if same_id(id, "tools.attribution") {
        return &[];
    }
    if same_id(id, "tools.outcomes") {
        return &[
            "completed_nonempty",
            "verified_zero",
            "partial",
            "failed",
            "blocked",
        ];
    }
    if same_id(id, "tools.reliability") {
        return &[];
    }
    if same_id(id, "tools.search_health") {
        return &[];
    }
    if same_id(id, "tools.evidence") {
        return &[];
    }
    if same_id(id, "tools.failure_causes") {
        return &[];
    }
    &[]
}
const fn applicable_filters(id: &str) -> &'static [&'static str] {
    if same_id(id, "models.capacity") || same_id(id, "atlas.backlog") {
        return &[];
    }
    &["app", "provider", "role", "mode", "tool", "category"]
}

const fn detail_columns(id: &str) -> &'static [&'static str] {
    if same_id(id, "intel.volume") {
        return &["bucket", "total", "by_tag", "untagged"];
    }
    if same_id(id, "intel.confidence") {
        return &[
            "bands",
            "paired_n",
            "mean_initial",
            "mean_current",
            "median_initial",
            "median_current",
            "coverage_note",
        ];
    }
    if same_id(id, "intel.origins") {
        return &["origin", "articles", "share", "is_other", "is_unknown"];
    }
    if same_id(id, "intel.enrichment") {
        return &[
            "tag",
            "articles",
            "body_pct",
            "claims_pct",
            "report_pct",
            "mean_initial_confidence",
            "initial_n",
            "mean_brief_rating",
            "brief_n",
        ];
    }
    if same_id(id, "intel.reports") {
        return &[
            "mode",
            "completed",
            "partial",
            "failed",
            "waiting",
            "blocked",
            "mean_wall_ms",
            "p95_wall_ms",
            "mean_active_ms",
            "n",
        ];
    }
    if same_id(id, "intel.publishers") {
        return &[
            "domain",
            "articles",
            "share",
            "is_other",
            "is_unknown",
            "top3_concentration",
        ];
    }
    if same_id(id, "intel.freshness") {
        return &["buckets", "missing", "future"];
    }
    if same_id(id, "recon.outcomes") {
        return &[
            "bucket",
            "mode",
            "completed_with_evidence",
            "completed_zero_evidence",
            "partial",
            "failed",
            "cancelled",
        ];
    }
    if same_id(id, "recon.stages") {
        return &["stage", "mode", "mean_exec_ms", "mean_wait_ms", "n"];
    }
    if same_id(id, "recon.recall") {
        return &[
            "mode",
            "queries",
            "with_candidates",
            "with_accepted",
            "retention_pct",
            "accepted_per_run",
            "rejection_reason",
        ];
    }
    if same_id(id, "recon.workload") {
        return &[
            "mode",
            "runs",
            "directives_per_run",
            "calls_per_run",
            "categories_per_run",
            "accepted_memories_per_run",
            "median_wall_ms",
            "p95_wall_ms",
        ];
    }
    if same_id(id, "recon.diversity") {
        return &[
            "category",
            "eligible_scopes",
            "scopes_with_two_attempted",
            "scopes_with_two_successful",
            "source_groups_per_scope",
            "top_shortfall_reason",
            "eligible",
        ];
    }
    if same_id(id, "recon.directives") {
        return &[
            "mode",
            "answered",
            "partial",
            "unresolved",
            "blocked",
            "unknown",
            "n",
        ];
    }
    if same_id(id, "recon.unresolved") {
        return &[
            "run_id",
            "mode",
            "label",
            "reason",
            "evidence_count",
            "last_progress",
            "next_action",
        ];
    }
    if same_id(id, "atlas.cycles") {
        return &["bucket", "completed", "partial", "failed", "cancelled"];
    }
    if same_id(id, "atlas.hot_zones") {
        return &[
            "origin",
            "articles",
            "latest_temperature",
            "mean_temperature",
            "latest_tier",
            "tier1_share",
            "tier2_share",
            "tier3_share",
            "snapshots",
            "latest_at",
        ];
    }
    if same_id(id, "atlas.temperature") {
        return &[
            "origin",
            "previous_temperature",
            "current_temperature",
            "delta",
            "latest_tier",
            "articles",
            "comparable",
            "label",
        ];
    }
    if same_id(id, "atlas.cycle_time") {
        return &["bucket", "mean_ms", "p95_ms", "n", "mean_queue_ms"];
    }
    if same_id(id, "atlas.discovery") {
        return &[
            "bucket",
            "fetched",
            "retained_new",
            "retained_existing",
            "duplicate_in_cycle",
            "rejected",
            "pending",
        ];
    }
    if same_id(id, "atlas.backlog") {
        return &[
            "stage",
            "unit",
            "queued",
            "running",
            "waiting",
            "blocked",
            "oldest_pending_ms",
            "completed_in_period",
            "latest_error_category",
        ];
    }
    if same_id(id, "models.capacity") {
        return &[
            "capacity",
            "capacity_available",
            "by_role",
            "latency",
            "queue_delay",
            "performance",
            "fallback",
            "amplification",
            "failures",
            "attempts",
            "note",
        ];
    }
    if same_id(id, "models.by_role") {
        return &["bucket", "by_role", "total"];
    }
    if same_id(id, "models.latency") {
        return &[
            "bucket",
            "p50_ms",
            "p95_ms",
            "n",
            "p50_first_header_ms",
            "p50_first_content_ms",
        ];
    }
    if same_id(id, "models.queue") {
        return &["bucket", "p50_ms", "p95_ms", "n"];
    }
    if same_id(id, "models.performance") {
        return &[
            "provider",
            "model",
            "role",
            "sends",
            "completed_attempts",
            "attempt_error_pct",
            "http_429",
            "final_operation_failure_pct",
            "mean_exec_ms",
            "p95_exec_ms",
            "n",
            "is_model_row",
        ];
    }
    if same_id(id, "models.fallback") {
        return &[
            "role",
            "primary_provider",
            "primary_model",
            "effective_provider",
            "effective_model",
            "trigger_reason",
            "triggered_operations",
            "recovered_operations",
            "recovery_pct",
            "median_ttoutcome_ms",
            "p95_ttoutcome_ms",
            "recovered_n",
            "failed_n",
        ];
    }
    if same_id(id, "models.amplification") {
        return &[
            "bucket",
            "sends",
            "terminal_operations",
            "ratio",
            "in_flight_operations",
        ];
    }
    if same_id(id, "models.failures") {
        return &["bucket", "by_category", "total_failed", "pct_of_finished"];
    }
    if same_id(id, "tools.usage") {
        return &[
            "tool_id",
            "category",
            "invocations",
            "remote",
            "local",
            "cache",
        ];
    }
    if same_id(id, "tools.outcomes") {
        return &[
            "bucket",
            "completed_nonempty",
            "verified_zero",
            "partial",
            "failed",
            "blocked",
        ];
    }
    if same_id(id, "tools.reliability") {
        return &[
            "tool_id",
            "category",
            "invocations",
            "wire_requests",
            "cache_hit_pct",
            "verified_zero_pct",
            "error_pct",
            "mean_ms",
            "p95_ms",
            "dominant_trigger",
            "dominant_mode",
        ];
    }
    if same_id(id, "tools.search_health") {
        return &[
            "engine",
            "fetches",
            "valid_serps",
            "verified_zero",
            "challenge",
            "parser_mismatch",
            "transport_failure",
            "usable_per_fetch",
            "cache_hits",
            "last_success",
            "parser_version",
        ];
    }
    if same_id(id, "tools.evidence") {
        return &[
            "tool_id",
            "category",
            "successful_nonempty",
            "with_evidence",
            "acceptance_pct",
            "distinct_evidence",
            "cited_by_completed_reports",
            "coverage_n",
            "nonadditive",
        ];
    }
    if same_id(id, "tools.failure_causes") {
        return &["cause", "invocations", "share", "rank"];
    }
    &[]
}

macro_rules! widget {
    ($id:expr, $section:expr, $title:expr, $priority:expr) => {
        WidgetSpec {
            id: $id,
            section: $section,
            title: $title,
            priority: $priority,
            renderer: renderer_kind($id),
            unit: widget_unit($id),
            series_keys: series_keys($id),
            applicable_filters: applicable_filters($id),
            detail_columns: detail_columns($id),
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
    pub all_apps: bool,
    pub report: Option<&'static str>,
    pub grid_scroll: ScrollPane,
    pub report_scroll: ScrollPane,
    pub system_scroll: [ScrollPane; 2],
    pub selected_bucket: HashMap<&'static str, usize>,
    pub period_popup: bool,
    pub picker_selection: usize,
    pub generation: u64,
    pub loading: bool,
    pub snapshot_filters: Option<StatFilters>,
    pub section: Section,
    /// Focused widget index inside the focused section.
    pub focus: usize,
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
            all_apps: true,
            report: None,
            grid_scroll: ScrollPane::default(),
            report_scroll: ScrollPane::default(),
            system_scroll: [ScrollPane::default(), ScrollPane::default()],
            selected_bucket: HashMap::new(),
            period_popup: false,
            picker_selection: 0,
            generation: 0,
            loading: false,
            snapshot_filters: None,
            section: Section::Intel,
            focus: 0,
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
        if self.all_apps {
            [
                "intel.volume",
                "recon.outcomes",
                "atlas.cycles",
                "models.by_role",
                "tools.outcomes",
            ]
            .iter()
            .filter_map(|id| WIDGETS.iter().find(|w| w.id == *id))
            .collect()
        } else {
            self.section.widgets()
        }
    }

    pub fn focused_widget(&self) -> Option<&'static WidgetSpec> {
        let widgets = self.focused_widgets();
        widgets
            .get(self.focus.min(widgets.len().saturating_sub(1)))
            .copied()
    }

    pub fn accept_snapshot(&mut self, snapshot: ProfileSnapshot) {
        for (id, index) in &mut self.selected_bucket {
            if let Some(widget) = WIDGETS.iter().find(|widget| widget.id == *id) {
                let stamps = |snapshot: &ProfileSnapshot| {
                    let buckets = count_buckets(widget, snapshot);
                    if buckets.is_empty() {
                        series_points(widget, snapshot)
                            .into_iter()
                            .map(|point| point.start)
                            .collect::<Vec<_>>()
                    } else {
                        buckets.into_iter().map(|bucket| bucket.start).collect()
                    }
                };
                if let Some(previous) = self
                    .snapshot
                    .as_ref()
                    .and_then(|old| stamps(old).get(*index).cloned())
                {
                    let next = stamps(&snapshot);
                    *index = next
                        .iter()
                        .position(|stamp| stamp == &previous)
                        .unwrap_or_else(|| (*index).min(next.len().saturating_sub(1)));
                }
            }
        }
        self.snapshot_filters = Some(snapshot.filters.clone());
        self.snapshot = Some(snapshot);
        self.error = None;
    }

    pub fn invalidate(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.loaded_at = None;
    }

    pub fn picker_choices(&self) -> Vec<String> {
        if self.period_popup {
            return profile_stats::Period::ALL
                .iter()
                .map(|p| p.label().to_owned())
                .collect();
        }
        let mut choices = vec![String::new()];
        if let Some(dimension) = self.filter_popup {
            if let Some((_, values)) = self.options.iter().find(|(name, _)| name == dimension) {
                choices.extend(
                    values
                        .iter()
                        .filter(|value| {
                            value
                                .to_lowercase()
                                .contains(&self.filter_edit.to_lowercase())
                        })
                        .cloned(),
                );
            }
        }
        choices
    }

    /// Reads one snapshot. Read-only: the writer is never locked.
    #[cfg(test)]
    pub fn reload(&mut self, store: &Store) {
        match store.profile_snapshot(&self.filters.period, &self.filters) {
            Ok(snapshot) => {
                self.accept_snapshot(snapshot);
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
        self.all_apps = false;
        self.grid_scroll.offset = 0;
        self.section = all[next];
        self.focus = 0;
    }

    pub fn prev_section(&mut self) {
        let all = Section::all();
        let prev = (self.section_index() + all.len() - 1) % all.len();
        self.all_apps = false;
        self.grid_scroll.offset = 0;
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
        self.invalidate();
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// Draws the Profile module: the tab strip, then the active tab's body.
pub fn draw_profile(frame: &mut Frame, app: &App, area: Rect) {
    let layout = AnalyticsLayout::new(area);
    draw_tab_strip(frame, app, layout.tabs);
    match app.profile.tab {
        SystemTab::Overview => {
            draw_filter_strip(frame, app, layout.controls);
            draw_section_navigator(frame, app, layout.apps);
            draw_section_body(frame, app, layout.content);
        }
        SystemTab::System => {
            draw_system_tab(frame, app, strip(area, 1, area.height.saturating_sub(2)))
        }
    }
    draw_status_strip(frame, app, layout.status);
    if app.profile.filter_popup.is_some() || app.profile.period_popup {
        draw_picker(frame, app, area);
    }
}

/// The System tab: Host and Paths, the panes this module always had. Logs stays
/// its own module with its own clear action and retention label; the two tabs
/// split the System pane, they do not take Logs over.
fn system_lines(app: &App, pane: usize, width: u16) -> Vec<Line<'static>> {
    let lines = if pane == 0 {
        super::ui::system_host_lines(&app.hardware)
    } else {
        super::ui::system_path_lines()
    };
    components::measured_lines(lines.into_iter().map(Line::raw).collect(), width)
}
fn draw_system_tab(frame: &mut Frame, app: &App, area: Rect) {
    let (content, actions) = super::ui::system_areas(area);
    super::ui::draw_button(
        frame,
        app,
        super::app::ButtonId::RefreshHardware,
        "Refresh hardware",
        super::ui::system_button(actions),
    );
    let panes = if content.height < 27 {
        super::ui::pane_tabs(
            frame,
            app,
            Rect::new(content.x, content.y, content.width, 1),
            &["Host", "Paths"],
        );
        let selected = app.compact_pages[super::app::ModuleId::System.index()] % 2;
        vec![(
            selected,
            Rect::new(
                content.x,
                content.y + 1,
                content.width,
                content.height.saturating_sub(1),
            ),
        )]
    } else {
        let host_h = (system_lines(app, 0, content.width.saturating_sub(2)).len() + 2)
            .min(content.height as usize / 2) as u16;
        let areas = rows(content, &[host_h, 0]);
        vec![(0, areas[0]), (1, areas[1])]
    };
    for (index, area) in panes {
        let block = theme::panel(if index == 0 { " host " } else { " paths " });
        let inner = block.inner(area);
        frame.render_widget(block, area);
        app.layout
            .borrow_mut()
            .register(Target::ProfileSystem(index), area);
        let lines = system_lines(app, index, inner.width);
        let mut pane = app.profile.system_scroll[index].clone();
        pane.scroll(0, lines.len(), inner.height as usize);
        frame.render_widget(
            Paragraph::new(pane.visible(&lines, inner.height as usize).to_vec()),
            inner,
        );
    }
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

fn draw_actions(frame: &mut Frame, app: &App, area: Rect, actions: &[(Target, String)]) {
    let labels: Vec<&str> = actions.iter().map(|(_, label)| label.as_str()).collect();
    for ((target, label), rect) in actions.iter().zip(components::action_rects(area, &labels)) {
        if rect.width == 0 {
            continue;
        }
        app.layout.borrow_mut().register(*target, rect);
        let active = match target {
            Target::ProfileTab(index) => {
                *index == usize::from(app.profile.tab == SystemTab::System)
            }
            Target::ProfileApp(index) => {
                *index
                    == if app.profile.all_apps {
                        0
                    } else {
                        app.profile.section_index() + 1
                    }
            }
            _ => false,
        };
        frame.render_widget(
            Paragraph::new(label.as_str()).style(if app.focus == *target || active {
                theme::selected()
            } else {
                theme::dim()
            }),
            rect,
        );
    }
}
fn draw_tab_strip(frame: &mut Frame, app: &App, area: Rect) {
    draw_actions(
        frame,
        app,
        area,
        &[
            (
                Target::ProfileTab(0),
                format!(" {} ", SystemTab::all()[0].label()),
            ),
            (
                Target::ProfileTab(1),
                format!(" {} ", SystemTab::all()[1].label()),
            ),
            (Target::ProfileAction(3), " Configs [x] ".into()),
        ],
    );
}

fn draw_status_strip(frame: &mut Frame, app: &App, area: Rect) {
    if area.height == 0 {
        return;
    }
    let line = if let Some(error) = &app.profile.error {
        Line::from(Span::styled(format!("Stale · {error}"), theme::error()))
    } else if app
        .profile
        .snapshot_filters
        .as_ref()
        .is_some_and(|filters| filters != &app.profile.filters)
    {
        Line::from(Span::styled(
            "Stale · filters changed · awaiting refresh",
            theme::warn(),
        ))
    } else if app.profile.loading {
        Line::from(Span::styled(
            if app.profile.snapshot.is_some() {
                "Refreshing · showing previous snapshot"
            } else {
                "Loading statistics"
            },
            theme::warn(),
        ))
    } else {
        match app.profile.snapshot.as_ref() {
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
        }
    };
    frame.render_widget(Paragraph::new(line), area);
}

fn draw_filter_strip(frame: &mut Frame, app: &App, area: Rect) {
    let active: Vec<String> = ["app", "provider", "role", "mode", "tool", "category"]
        .iter()
        .filter_map(|name| {
            let value = app.profile.filter_value(name);
            (!value.is_empty()).then(|| format!("{name}={value}"))
        })
        .collect();
    let compact = area.width < 55;
    let labels = if compact {
        vec![
            (
                Target::ProfileAction(0),
                format!(" [p]{} ", app.profile.filters.period.label()),
            ),
            (Target::ProfileAction(1), format!(" [f]{} ", active.len())),
            (Target::ProfileAction(2), " [c] ".into()),
            (Target::ProfileAction(4), " [r] ".into()),
        ]
    } else {
        vec![
            (
                Target::ProfileAction(0),
                format!(" [p] {} ", app.profile.filters.period.label()),
            ),
            (
                Target::ProfileAction(1),
                format!(
                    " [f] filters{} ",
                    if active.is_empty() {
                        String::new()
                    } else {
                        format!(" ({})", active.len())
                    }
                ),
            ),
            (Target::ProfileAction(2), " [c] clear ".into()),
            (Target::ProfileAction(4), " [r] ".into()),
        ]
    };
    let used = labels
        .iter()
        .map(|(_, label)| components::text_width(label))
        .sum::<usize>() as u16;
    draw_actions(frame, app, area, &labels);
    if !active.is_empty() {
        let x = area.x.saturating_add(used).min(area.right());
        frame.render_widget(
            Paragraph::new(active.join(" · ")).style(theme::accent()),
            Rect::new(x, area.y, area.right() - x, area.height),
        );
    }
}
fn draw_section_navigator(frame: &mut Frame, app: &App, area: Rect) {
    // On compact screens expose a measured current-app selector; activation cycles all six choices.
    if area.width < 66 {
        let label = if app.profile.all_apps {
            "All apps"
        } else {
            app.profile.section.label()
        };
        draw_actions(
            frame,
            app,
            area,
            &[(Target::ProfileApp(6), format!(" {label} [0–5, [, ]] "))],
        );
    } else {
        let mut actions = vec![(Target::ProfileApp(0), " All apps ".into())];
        actions.extend(
            Section::all()
                .iter()
                .enumerate()
                .map(|(i, section)| (Target::ProfileApp(i + 1), format!(" {} ", section.label()))),
        );
        draw_actions(frame, app, area, &actions);
    }
}
fn content_layout(app: &App) -> AnalyticsLayout {
    AnalyticsLayout::new(super::ui::body_rect(app))
}
fn grid_offset(app: &App, layout: &AnalyticsLayout) -> usize {
    let mut pane = app.profile.grid_scroll.clone();
    pane.reveal(
        layout.focus_start(app.profile.focus),
        if layout.compact {
            1
        } else {
            layout.card_height
        },
        layout.content.height as usize,
    );
    pane.offset
}
fn draw_section_body(frame: &mut Frame, app: &App, area: Rect) {
    let widgets = app.profile.focused_widgets();
    if let Some(id) = app.profile.report {
        if let Some(widget) = WIDGETS.iter().find(|w| w.id == id) {
            draw_report(frame, app, widget, area);
        }
        return;
    }
    let layout = content_layout(app);
    let offset = grid_offset(app, &layout);
    for (index, widget) in widgets.iter().enumerate() {
        if let Some(rect) = layout.card(index, offset) {
            app.layout
                .borrow_mut()
                .register(Target::ProfileCard(index), rect);
            if layout.compact {
                frame.render_widget(
                    Paragraph::new(format!(
                        "{} {} · {}",
                        if index == app.profile.focus {
                            "▸"
                        } else {
                            " "
                        },
                        widget.section.label(),
                        widget.title
                    ))
                    .style(if index == app.profile.focus {
                        theme::accent()
                    } else {
                        theme::text()
                    }),
                    rect,
                );
            } else {
                draw_widget_panel(frame, app, widget, rect);
            }
        }
    }
}
fn widget_data(widget: &WidgetSpec, snapshot: &ProfileSnapshot) -> serde_json::Value {
    match widget.id {
        "intel.volume" => serde_json::to_value(&snapshot.intel.volume).unwrap_or_default(),
        "intel.confidence" => serde_json::to_value(&snapshot.intel.confidence).unwrap_or_default(),
        "intel.origins" => serde_json::to_value(&snapshot.intel.origins).unwrap_or_default(),
        "intel.enrichment" => serde_json::to_value(&snapshot.intel.enrichment).unwrap_or_default(),
        "intel.reports" => serde_json::to_value(&snapshot.intel.reports).unwrap_or_default(),
        "intel.publishers" => serde_json::to_value(&snapshot.intel.publishers).unwrap_or_default(),
        "intel.freshness" => serde_json::to_value(&snapshot.intel.freshness).unwrap_or_default(),
        "recon.outcomes" => serde_json::to_value(&snapshot.recon.outcomes).unwrap_or_default(),
        "recon.stages" => serde_json::to_value(&snapshot.recon.stages).unwrap_or_default(),
        "recon.recall" => serde_json::to_value(&snapshot.recon.recall).unwrap_or_default(),
        "recon.workload" => serde_json::to_value(&snapshot.recon.workload).unwrap_or_default(),
        "recon.diversity" => serde_json::to_value(&snapshot.recon.diversity).unwrap_or_default(),
        "recon.directives" => serde_json::to_value(&snapshot.recon.directives).unwrap_or_default(),
        "recon.unresolved" => serde_json::to_value(&snapshot.recon.unresolved).unwrap_or_default(),
        "atlas.cycles" => serde_json::to_value(&snapshot.atlas.cycle_outcomes).unwrap_or_default(),
        "atlas.hot_zones" => serde_json::to_value(&snapshot.atlas.origins).unwrap_or_default(),
        "atlas.temperature" => {
            serde_json::to_value(&snapshot.atlas.temperature_changes).unwrap_or_default()
        }
        "atlas.cycle_time" => serde_json::to_value(&snapshot.atlas.cycle_times).unwrap_or_default(),
        "atlas.discovery" => serde_json::to_value(&snapshot.atlas.discovery).unwrap_or_default(),
        "atlas.backlog" => serde_json::to_value(&snapshot.atlas.backlog).unwrap_or_default(),
        "models.capacity" => serde_json::to_value(&snapshot.models.capacity).unwrap_or_default(),
        "models.by_role" => serde_json::to_value(&snapshot.models.by_role).unwrap_or_default(),
        "models.latency" => serde_json::to_value(&snapshot.models.latency).unwrap_or_default(),
        "models.queue" => serde_json::to_value(&snapshot.models.queue_delay).unwrap_or_default(),
        "models.performance" => {
            serde_json::to_value(&snapshot.models.performance).unwrap_or_default()
        }
        "models.fallback" => serde_json::to_value(&snapshot.models.fallback).unwrap_or_default(),
        "models.amplification" => {
            serde_json::to_value(&snapshot.models.amplification).unwrap_or_default()
        }
        "models.failures" => serde_json::to_value(&snapshot.models.failures).unwrap_or_default(),
        "tools.usage" => serde_json::to_value(&snapshot.tools.top_tools).unwrap_or_default(),
        "tools.attribution" => {
            serde_json::to_value(&snapshot.tools.categories_by_trigger).unwrap_or_default()
        }
        "tools.outcomes" => serde_json::to_value(&snapshot.tools.outcomes).unwrap_or_default(),
        "tools.reliability" => {
            serde_json::to_value(&snapshot.tools.reliability).unwrap_or_default()
        }
        "tools.search_health" => {
            serde_json::to_value(&snapshot.tools.engine_health).unwrap_or_default()
        }
        "tools.evidence" => serde_json::to_value(&snapshot.tools.evidence).unwrap_or_default(),
        "tools.failure_causes" => {
            serde_json::to_value(&snapshot.tools.failure_causes).unwrap_or_default()
        }
        _ => serde_json::Value::Null,
    }
}
fn count_buckets(widget: &WidgetSpec, snapshot: &ProfileSnapshot) -> Vec<charts::TimeBucket> {
    let data = widget_data(widget, snapshot);
    let Some(rows) = data.as_array() else {
        return Vec::new();
    };
    let _stable_keys = widget.series_keys;
    let keys: &[&str] = match widget.id {
        "intel.volume" => &["total"],
        "recon.outcomes" => &[
            "completed_with_evidence",
            "completed_zero_evidence",
            "partial",
            "failed",
            "cancelled",
        ],
        "atlas.cycles" => &["completed", "partial", "failed", "cancelled"],
        "tools.outcomes" => &[
            "completed_nonempty",
            "verified_zero",
            "partial",
            "failed",
            "blocked",
        ],
        "models.by_role" | "models.failures" => &[],
        "atlas.discovery" => &[
            "retained_new",
            "retained_existing",
            "duplicate_in_cycle",
            "rejected",
            "pending",
        ],
        _ => return Vec::new(),
    };
    let mut merged =
        std::collections::BTreeMap::<String, std::collections::BTreeMap<String, u64>>::new();
    for row in rows {
        let start = row["bucket"].as_str().unwrap_or("").to_owned();
        let series = merged.entry(start).or_default();
        for key in keys {
            *series.entry((*key).to_owned()).or_default() += row[*key].as_u64().unwrap_or(0);
        }
        if widget.id == "models.by_role" || widget.id == "models.failures" {
            if let Some(roles) = row[if widget.id == "models.by_role" {
                "by_role"
            } else {
                "by_category"
            }]
            .as_array()
            {
                for role in roles {
                    *series
                        .entry(role[0].as_str().unwrap_or("unknown").to_owned())
                        .or_default() += role[1].as_u64().unwrap_or(0);
                }
            }
        }
    }
    let captured = chrono::DateTime::parse_from_rfc3339(&snapshot.captured_at)
        .map(|time| time.with_timezone(&chrono::Utc))
        .ok();
    let Some(now) = captured else {
        return merged
            .into_iter()
            .map(|(start, series)| charts::TimeBucket {
                end: bucket_end(&start, snapshot.filters.period.bucket_seconds()),
                start: bucket_start(&start),
                series: series.into_iter().collect(),
                available: true,
            })
            .collect();
    };
    let period = &snapshot.filters.period;
    let seconds = period.bucket_seconds();
    let first = period.start(now).timestamp().div_euclid(seconds) * seconds;
    let last = period.end(now).timestamp();
    let observed = chrono::DateTime::parse_from_rfc3339(&snapshot.observed_since)
        .ok()
        .map(|time| time.timestamp());
    let retained = chrono::DateTime::parse_from_rfc3339(&snapshot.retention.oldest_rollup)
        .ok()
        .or_else(|| chrono::DateTime::parse_from_rfc3339(&snapshot.retention.oldest_raw).ok())
        .map(|time| time.timestamp());
    let mut buckets = Vec::new();
    let mut timestamp = first;
    while timestamp < last {
        let Some(time) = chrono::DateTime::from_timestamp(timestamp, 0) else {
            break;
        };
        let format = if seconds < 3600 {
            "%Y-%m-%dT%H:%M"
        } else if seconds < 86400 {
            "%Y-%m-%dT%H"
        } else {
            "%Y-%m-%d"
        };
        let start = time.format(format).to_string();
        let values = merged.remove(&start);
        let available = values
            .as_ref()
            .is_some_and(|series| series.values().any(|value| *value > 0))
            || observed.is_some_and(|since| timestamp + seconds > since)
                && retained.is_none_or(|since| timestamp + seconds > since);
        buckets.push(charts::TimeBucket {
            end: bucket_end(&start, seconds),
            start: bucket_start(&start),
            series: values
                .map(|values| values.into_iter().collect())
                .unwrap_or_else(|| {
                    widget
                        .series_keys
                        .iter()
                        .map(|key| ((*key).to_owned(), 0))
                        .collect()
                }),
            available,
        });
        timestamp = timestamp.saturating_add(seconds);
    }
    buckets
}
fn series_points(widget: &WidgetSpec, snapshot: &ProfileSnapshot) -> Vec<charts::SeriesPoint> {
    let keys: &[&str] = match widget.id {
        "atlas.cycle_time" => &["mean_ms", "p95_ms", "mean_queue_ms"],
        "models.latency" => &[
            "p50_ms",
            "p95_ms",
            "p50_first_header_ms",
            "p50_first_content_ms",
        ],
        "models.queue" => &["p50_ms", "p95_ms"],
        "models.amplification" => &["ratio"],
        _ => return Vec::new(),
    };
    let data = widget_data(widget, snapshot);
    data.as_array()
        .into_iter()
        .flatten()
        .map(|row| charts::SeriesPoint {
            start: bucket_start(row["bucket"].as_str().unwrap_or("")),
            end: bucket_end(
                row["bucket"].as_str().unwrap_or(""),
                snapshot.filters.period.bucket_seconds(),
            ),
            series: keys
                .iter()
                .map(|key| ((*key).to_owned(), row[*key].as_f64()))
                .collect(),
            n: row["n"]
                .as_u64()
                .or_else(|| row["terminal_operations"].as_u64())
                .unwrap_or(0),
        })
        .collect()
}
fn point_count(widget: &WidgetSpec, snapshot: &ProfileSnapshot) -> usize {
    let buckets = count_buckets(widget, snapshot);
    if buckets.is_empty() {
        series_points(widget, snapshot).len()
    } else {
        buckets.len()
    }
}
fn bucket_start(start: &str) -> String {
    match start.len() {
        10 => format!("{start}T00:00:00Z"),
        13 => format!("{start}:00:00Z"),
        16 => format!("{start}:00Z"),
        _ => start.to_owned(),
    }
}
fn bucket_end(start: &str, seconds: i64) -> String {
    let padded = match start.len() {
        10 => format!("{start}T00:00:00Z"),
        13 => format!("{start}:00:00Z"),
        16 => format!("{start}:00Z"),
        _ => start.to_owned(),
    };
    chrono::DateTime::parse_from_rfc3339(&padded)
        .map(|time| (time + chrono::Duration::seconds(seconds)).to_rfc3339())
        .unwrap_or_else(|_| start.to_owned())
}
fn analytics_lines(
    app: &App,
    widget: &WidgetSpec,
    width: u16,
    height: usize,
    report: bool,
) -> Vec<Line<'static>> {
    let Some(snapshot) = &app.profile.snapshot else {
        return vec![Line::raw(if app.profile.error.is_some() {
            "Unavailable: snapshot read failed"
        } else {
            "Loading statistics"
        })];
    };
    let buckets = if widget.renderer == RendererKind::Time {
        count_buckets(widget, snapshot)
    } else {
        Vec::new()
    };
    let selected = app
        .profile
        .selected_bucket
        .get(widget.id)
        .copied()
        .unwrap_or(buckets.len().saturating_sub(1))
        .min(buckets.len().saturating_sub(1));
    let mut lines = vec![Line::from(Span::styled(
        format!(
            "{} · {} buckets · {}",
            snapshot.filters.period.label(),
            snapshot.filters.period.bucket_label(),
            widget.unit
        ),
        theme::dim(),
    ))];
    if report {
        lines.extend(components::measured_lines(
            vec![Line::raw(format!(
                "Observed since {} · retained raw {} days / rollups {} days",
                if snapshot.observed_since.is_empty() {
                    "unavailable"
                } else {
                    &snapshot.observed_since
                },
                snapshot.retention.raw_days,
                snapshot.retention.rollup_days
            ))],
            width,
        ));
    }
    if !buckets.is_empty() {
        let total: u64 = buckets.iter().map(charts::TimeBucket::total).sum();
        let headline = if buckets.iter().all(|bucket| !bucket.available) {
            "Unavailable history".into()
        } else if total == 0 {
            format!("Empty window · 0 {}", widget.unit)
        } else {
            format!("{} {}", charts::count(total), widget.title)
        };
        lines.push(Line::from(Span::styled(headline, theme::accent())));
        // Exact selected values take precedence over decorative plot labels.
        let bucket = &buckets[selected];
        lines.extend(components::measured_lines(
            vec![
                Line::raw(format!(
                    "Selected {} → {} · N={}{}",
                    bucket.start,
                    bucket.end,
                    bucket.total(),
                    if bucket.available {
                        ""
                    } else {
                        " · unavailable history"
                    }
                )),
                Line::from(
                    bucket
                        .series
                        .iter()
                        .map(|(key, value)| {
                            Span::styled(
                                format!("{key}={}  ", charts::count(*value)),
                                charts::series_style(key),
                            )
                        })
                        .collect::<Vec<_>>(),
                ),
            ],
            width,
        ));
        let available = height.saturating_sub(lines.len() + 2);
        let plot = charts::time_plot(&buckets, width as usize, available.min(10), selected);
        lines.extend(components::measured_lines(plot, width));
        if widget.id == "intel.volume" {
            lines.extend(components::measured_lines(
                vec![Line::raw(
                    "Distinct articles; tags overlap and are shown separately in the report",
                )],
                width,
            ));
        }
    } else if !report && width < 70 {
        lines.extend(components::detail_table(
            &widget_data(widget, snapshot),
            widget.detail_columns,
            width,
        ));
    } else if !report {
        lines.extend(components::measured_lines(
            widget_lines(widget, snapshot, width as usize, 6),
            width,
        ));
    }
    if report {
        lines.push(Line::raw(format!(
            "Details · {}",
            widget.detail_columns.join(" / ")
        )));
        lines.extend(components::detail_table(
            &widget_data(widget, snapshot),
            widget.detail_columns,
            width,
        ));
    }
    if widget.applicable_filters.is_empty() {
        lines.insert(0, Line::raw("Live · historical filters do not apply"));
    }
    if lines.len() <= 2 {
        lines.push(Line::raw("Empty window / unavailable history"));
    }
    lines
}
fn draw_widget_panel(frame: &mut Frame, app: &App, widget: &WidgetSpec, area: Rect) {
    let focused = app
        .profile
        .focused_widget()
        .is_some_and(|w| w.id == widget.id);
    let inner = components::analytics_card(
        frame,
        area,
        &format!(" {} · {} ", widget.section.label(), widget.title),
        focused,
    );
    let lines = analytics_lines(app, widget, inner.width, inner.height as usize, false);
    frame.render_widget(
        Paragraph::new(
            lines
                .into_iter()
                .take(inner.height as usize)
                .collect::<Vec<_>>(),
        ),
        inner,
    );
}
fn report_layout(app: &App, widget: &WidgetSpec, inner: Rect) -> ReportLayout {
    let mut layout = ReportLayout::new(inner, selected_lines(app, widget, inner.width).len());
    if widget.renderer == RendererKind::Table {
        layout.details.y = layout.plot.y.min(layout.details.y);
        layout.details.x = inner.x;
        layout.details.width = inner.width;
        layout.details.height = layout.position.y.saturating_sub(layout.details.y);
        layout.plot = Rect::default();
    }
    layout
}
fn categorical_plot(
    widget: &WidgetSpec,
    snapshot: &ProfileSnapshot,
    width: u16,
    height: usize,
) -> Vec<Line<'static>> {
    let (label, value) = match widget.id {
        "intel.origins" | "atlas.hot_zones" => ("origin", "articles"),
        "intel.publishers" => ("domain", "articles"),
        "tools.usage" => ("tool_id", "invocations"),
        "tools.attribution" => ("category", "total"),
        "tools.failure_causes" => ("cause", "invocations"),
        _ => {
            return components::measured_lines(
                widget_lines(widget, snapshot, width as usize, height.max(1)),
                width,
            )
            .into_iter()
            .take(height)
            .collect();
        }
    };
    let data = widget_data(widget, snapshot);
    let Some(rows) = data.as_array() else {
        return Vec::new();
    };
    let peak = rows
        .iter()
        .filter_map(|row| row[value].as_u64())
        .max()
        .unwrap_or(0);
    let mut lines = Vec::new();
    for row in rows.iter().take(height / 2) {
        let count = row[value].as_u64().unwrap_or(0);
        lines.push(Line::raw(format!(
            "{} · {value}={count}",
            components::clip_text(
                row[label].as_str().unwrap_or("unknown"),
                usize::from(width).saturating_sub(value.len() + count.to_string().len() + 5)
            ),
        )));
        lines.push(charts::rank_bar(count, peak, width as usize));
    }
    lines
}
fn report_detail_lines(app: &App, widget: &WidgetSpec, width: u16) -> Vec<Line<'static>> {
    let Some(snapshot) = &app.profile.snapshot else {
        return vec![Line::raw("Loading statistics")];
    };
    let mut lines =
        components::detail_table(&widget_data(widget, snapshot), widget.detail_columns, width);
    if lines.is_empty() {
        lines.push(Line::raw("Empty window · no recorded rows"));
    }
    if widget.id == "models.capacity" && !snapshot.models.capacity_available {
        lines.insert(
            0,
            Line::raw("Unavailable · provider companion has not published capacity"),
        );
    }
    let note = match widget.section {
        Section::Intel => &snapshot.intel.note,
        Section::Recon => &snapshot.recon.note,
        Section::Atlas => &snapshot.atlas.note,
        Section::Models => &snapshot.models.note,
        Section::Tools => &snapshot.tools.note,
    };
    lines.extend(components::measured_lines(
        vec![
            Line::raw(note.clone()),
            Line::raw(format!(
                "Observed since {} · raw {} days / rollups {} days",
                if snapshot.observed_since.is_empty() {
                    "unavailable"
                } else {
                    &snapshot.observed_since
                },
                snapshot.retention.raw_days,
                snapshot.retention.rollup_days
            )),
        ],
        width,
    ));
    lines
}
fn selected_lines(app: &App, widget: &WidgetSpec, width: u16) -> Vec<Line<'static>> {
    let Some(snapshot) = &app.profile.snapshot else {
        return vec![Line::raw("Loading statistics")];
    };
    let buckets = count_buckets(widget, snapshot);
    let selected = app
        .profile
        .selected_bucket
        .get(widget.id)
        .copied()
        .unwrap_or(buckets.len().saturating_sub(1))
        .min(buckets.len().saturating_sub(1));
    let mut lines = vec![Line::raw(format!(
        "{} · {} buckets · {}",
        snapshot.filters.period.label(),
        snapshot.filters.period.bucket_label(),
        widget.unit
    ))];
    if let Some(bucket) = buckets.get(selected) {
        lines.push(Line::raw(format!(
            "{} → {} · N={}{}",
            bucket.start,
            bucket.end,
            bucket.total(),
            if bucket.available {
                ""
            } else {
                " · unavailable history"
            }
        )));
        lines.push(Line::from(
            bucket
                .series
                .iter()
                .map(|(key, value)| {
                    Span::styled(
                        format!("{key}={}  ", charts::count(*value)),
                        charts::series_style(key),
                    )
                })
                .collect::<Vec<_>>(),
        ));
    }
    if widget.id == "intel.volume" {
        if let Some(bucket) = buckets.get(selected) {
            if let Some(row) = snapshot
                .intel
                .volume
                .iter()
                .find(|row| bucket_start(&row.bucket) == bucket.start)
            {
                lines.push(Line::raw(format!(
                    "Tags (overlap): {} · untagged={}",
                    row.by_tag
                        .iter()
                        .map(|(key, value)| format!("{key}={value}"))
                        .collect::<Vec<_>>()
                        .join(" · "),
                    row.untagged
                )));
            }
        }
    }
    if buckets.is_empty() {
        let points = series_points(widget, snapshot);
        let selected = app
            .profile
            .selected_bucket
            .get(widget.id)
            .copied()
            .unwrap_or(points.len().saturating_sub(1))
            .min(points.len().saturating_sub(1));
        if let Some(point) = points.get(selected) {
            lines.push(Line::raw(format!(
                "{} → {} · N={}{}",
                point.start,
                point.end,
                point.n,
                small_sample(point.n)
                    .map(|_| " (small sample; p95 suppressed)")
                    .unwrap_or("")
            )));
            lines.push(Line::from(
                point
                    .series
                    .iter()
                    .map(|(key, value)| {
                        Span::styled(
                            format!(
                                "{key}={}  ",
                                value
                                    .map(|value| value.to_string())
                                    .unwrap_or_else(|| "N/A".into())
                            ),
                            charts::series_style(key),
                        )
                    })
                    .collect::<Vec<_>>(),
            ));
        }
    }
    components::measured_lines(lines, width)
}
fn draw_report(frame: &mut Frame, app: &App, widget: &WidgetSpec, area: Rect) {
    let block = theme::panel(&format!(" {} · Esc grid · ←/→ bucket ", widget.title));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let selected = selected_lines(app, widget, inner.width);
    let layout = report_layout(app, widget, inner);
    frame.render_widget(Paragraph::new(selected), layout.selected);
    app.layout
        .borrow_mut()
        .register(Target::ProfileReport, layout.details);
    if let Some(snapshot) = &app.profile.snapshot {
        let buckets = count_buckets(widget, snapshot);
        let points = series_points(widget, snapshot);
        let selected = app
            .profile
            .selected_bucket
            .get(widget.id)
            .copied()
            .unwrap_or(point_count(widget, snapshot).saturating_sub(1));
        let lines = if widget.renderer != RendererKind::Time {
            categorical_plot(
                widget,
                snapshot,
                layout.plot.width,
                layout.plot.height as usize,
            )
        } else if buckets.is_empty() {
            charts::series_plot(
                &points,
                layout.plot.width as usize,
                layout.plot.height as usize,
                selected,
            )
        } else {
            charts::time_plot(
                &buckets,
                layout.plot.width as usize,
                layout.plot.height as usize,
                selected,
            )
        };
        frame.render_widget(
            Paragraph::new(components::measured_lines(lines, layout.plot.width)),
            layout.plot,
        );
        if !points.is_empty() && layout.plot.width >= 14 && layout.plot.height >= 5 {
            let capacity = ((layout.plot.width as usize - 7) / 3).max(1);
            let start = selected.min(points.len() - 1) / capacity * capacity;
            for index in start..(start + capacity).min(points.len()) {
                app.layout.borrow_mut().register(
                    Target::ProfileBucket(index),
                    Rect::new(
                        layout.plot.x + 7 + (index - start) as u16 * 3,
                        layout.plot.y,
                        2,
                        layout.plot.height,
                    ),
                );
            }
        }
        if !buckets.is_empty() && layout.plot.width >= 14 && layout.plot.height >= 4 {
            let capacity = (usize::from(layout.plot.width) - 7) / 3;
            let group = buckets.len().div_ceil(capacity.max(1)).max(1);
            for (i, _) in buckets.chunks(group).enumerate() {
                app.layout.borrow_mut().register(
                    Target::ProfileBucket(i * group),
                    Rect::new(
                        layout.plot.x + 7 + i as u16 * 3,
                        layout.plot.y,
                        2,
                        layout.plot.height,
                    ),
                );
            }
        }
    }
    let lines = report_detail_lines(app, widget, layout.details.width);
    let mut pane = app.profile.report_scroll.clone();
    pane.scroll(0, lines.len(), layout.details.height as usize);
    frame.render_widget(
        Paragraph::new(
            pane.visible(&lines, layout.details.height as usize)
                .to_vec(),
        ),
        layout.details,
    );
    frame.render_widget(
        Paragraph::new(format!(
            "Rows {}–{} / {} · PgUp/PgDn · wheel",
            pane.offset + 1,
            (pane.offset + layout.details.height as usize).min(lines.len()),
            lines.len()
        ))
        .style(theme::dim()),
        layout.position,
    );
}
fn draw_picker(frame: &mut Frame, app: &App, area: Rect) {
    use ratatui::widgets::Clear;
    let popup = Rect::new(
        area.x + 2,
        area.y + 2,
        area.width.saturating_sub(4),
        area.height.saturating_sub(4),
    );
    app.layout.borrow_mut().push_scope(popup);
    frame.render_widget(Clear, popup);
    let dimension = app.profile.filter_popup.unwrap_or("period");
    let block = theme::panel(&format!(" {dimension} · Tab dimension · Esc close "));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    frame.render_widget(
        Paragraph::new(format!("Search: {}", app.profile.filter_edit)),
        Rect::new(inner.x, inner.y, inner.width, 1),
    );
    let choices = app.profile.picker_choices();
    let room = inner.height.saturating_sub(2) as usize;
    let start = app
        .profile
        .picker_selection
        .saturating_sub(room.saturating_sub(1));
    for (i, label) in choices.iter().enumerate().skip(start).take(room) {
        let rect = Rect::new(inner.x, inner.y + 1 + (i - start) as u16, inner.width, 1);
        app.layout
            .borrow_mut()
            .register(Target::ProfileChoice(i), rect);
        frame.render_widget(
            Paragraph::new(if label.is_empty() {
                "any"
            } else {
                label.as_str()
            })
            .style(if i == app.profile.picker_selection {
                theme::selected()
            } else {
                theme::text()
            }),
            rect,
        );
    }
}

/// The line list for one widget. Every widget renders through this single
/// entry point, so a new widget cannot smuggle in its own drawing rules.
fn widget_lines(
    widget: &WidgetSpec,
    snapshot: &ProfileSnapshot,
    width: usize,
    page: usize,
) -> Vec<Line<'static>> {
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
pub fn activate(app: &mut App, target: Target) {
    match target {
        Target::ProfileTab(index) => {
            app.profile.tab = if index == 0 {
                SystemTab::Overview
            } else {
                SystemTab::System
            }
        }
        Target::ProfileApp(index) => {
            let index = if index == 6 {
                if app.profile.all_apps {
                    1
                } else {
                    (app.profile.section_index() + 2) % 6
                }
            } else {
                index
            };
            app.profile.all_apps = index == 0;
            if index > 0 {
                app.profile.section = Section::all()[(index - 1).min(4)];
            }
            app.profile.focus = 0;
            app.profile.grid_scroll.offset = 0;
            app.profile.report = None;
        }
        Target::ProfileCard(index) => {
            app.profile.focus = index;
            if let Some(widget) = app.profile.focused_widget() {
                app.profile.report = Some(widget.id);
                app.profile.report_scroll.offset = 0;
            }
        }
        Target::ProfileAction(0) => {
            app.profile.period_popup = true;
            app.profile.picker_selection = 0;
        }
        Target::ProfileAction(1) => {
            app.profile.filter_popup = Some("app");
            app.profile.filter_edit.clear();
            app.profile.picker_selection = 0;
        }
        Target::ProfileAction(2) => {
            app.profile.filters.clear();
            app.profile.invalidate();
        }
        Target::ProfileAction(3) => {
            app.profile_config.open();
            app.profile.config_open = true;
            app.overlay = super::app::Overlay::Configs;
            app.set_focus(Target::Field(super::app::FieldId::ProfileExportPath));
        }
        Target::ProfileAction(4) => app.profile.loaded_at = None,
        Target::ProfileBucket(index) => {
            if let Some(id) = app.profile.report {
                app.profile.selected_bucket.insert(id, index);
            }
        }
        Target::ProfileChoice(index) => {
            if app.profile.period_popup {
                app.profile.filters.period = profile_stats::Period::ALL[index.min(3)].clone();
                app.profile.invalidate();
            } else if let Some(dimension) = app.profile.filter_popup {
                if let Some(value) = app.profile.picker_choices().get(index).cloned() {
                    app.profile.set_filter(dimension, &value);
                }
            }
            app.profile.period_popup = false;
            app.profile.filter_popup = None;
            app.set_focus(Target::ProfileAction(1));
        }
        _ => {}
    }
}
pub fn scroll(app: &mut App, delta: isize) {
    let layout = content_layout(app);
    if app.profile.tab == SystemTab::System {
        let index = match app.focus {
            Target::ProfileSystem(index) => index,
            _ => app.compact_pages[super::app::ModuleId::System.index()] % 2,
        };
        let lines = system_lines(app, index, layout.content.width.saturating_sub(2));
        app.profile.system_scroll[index].scroll(
            delta,
            lines.len(),
            layout.content.height.saturating_sub(3) as usize,
        );
        return;
    }
    if let Some(id) = app.profile.report {
        if let Some(widget) = WIDGETS.iter().find(|w| w.id == id) {
            let inner = Rect::new(
                layout.content.x + 1,
                layout.content.y + 1,
                layout.content.width.saturating_sub(2),
                layout.content.height.saturating_sub(2),
            );
            let report = report_layout(app, widget, inner);
            let lines = report_detail_lines(app, widget, report.details.width);
            app.profile
                .report_scroll
                .scroll(delta, lines.len(), report.details.height as usize);
        }
    } else {
        let offset = grid_offset(app, &layout);
        app.profile.grid_scroll.offset = offset;
        app.profile.grid_scroll.scroll(
            delta,
            layout.extent(app.profile.focused_widgets().len()),
            layout.content.height as usize,
        );
        let stride = if layout.compact {
            1
        } else {
            layout.card_height + 1
        };
        app.profile.focus = (app.profile.grid_scroll.offset.div_ceil(stride) * layout.columns)
            .min(app.profile.focused_widgets().len().saturating_sub(1));
    }
}
pub fn handle_key(app: &mut App, key: KeyEvent) -> bool {
    if key
        .modifiers
        .intersects(crossterm::event::KeyModifiers::CONTROL | crossterm::event::KeyModifiers::ALT)
    {
        return false;
    }
    if app.profile.filter_popup.is_some() || app.profile.period_popup {
        match key.code {
            KeyCode::Esc => {
                app.profile.filter_popup = None;
                app.profile.period_popup = false;
                app.set_focus(Target::ProfileAction(1));
            }
            KeyCode::Tab | KeyCode::BackTab if !app.profile.period_popup => {
                let dimensions = ["app", "provider", "role", "mode", "tool", "category"];
                let index = dimensions
                    .iter()
                    .position(|d| Some(*d) == app.profile.filter_popup)
                    .unwrap_or(0);
                app.profile.filter_popup =
                    Some(dimensions[(index + if key.code == KeyCode::Tab { 1 } else { 5 }) % 6]);
                app.profile.filter_edit.clear();
                app.profile.picker_selection = 0;
            }
            KeyCode::Down => {
                app.profile.picker_selection = (app.profile.picker_selection + 1)
                    .min(app.profile.picker_choices().len().saturating_sub(1))
            }
            KeyCode::Up => {
                app.profile.picker_selection = app.profile.picker_selection.saturating_sub(1)
            }
            KeyCode::Enter => activate(app, Target::ProfileChoice(app.profile.picker_selection)),
            KeyCode::Char(ch) if !app.profile.period_popup => {
                app.profile.filter_edit.push(ch);
                app.profile.picker_selection = 0;
            }
            KeyCode::Backspace => {
                app.profile.filter_edit.pop();
                app.profile.picker_selection = 0;
            }
            _ => {}
        }
        return true;
    }
    if matches!(app.focus, Target::Field(_)) {
        return false;
    }
    match key.code {
        KeyCode::Char('t') => {
            activate(
                app,
                Target::ProfileTab(usize::from(app.profile.tab == SystemTab::Overview)),
            );
            true
        }
        KeyCode::Char('x') => {
            activate(app, Target::ProfileAction(3));
            true
        }
        KeyCode::Char('r') => {
            if app.profile.tab == SystemTab::System {
                app.hardware = argos_osint_core::hardware::profile_cached(true);
            } else {
                activate(app, Target::ProfileAction(4));
            }
            true
        }
        KeyCode::PageUp | KeyCode::PageDown | KeyCode::Up | KeyCode::Down
            if app.profile.tab == SystemTab::System =>
        {
            scroll(
                app,
                match key.code {
                    KeyCode::PageUp => -10,
                    KeyCode::PageDown => 10,
                    KeyCode::Up => -1,
                    _ => 1,
                },
            );
            true
        }
        _ if app.profile.tab == SystemTab::System => false,
        KeyCode::Char('0'..='5') => {
            if let KeyCode::Char(ch) = key.code {
                activate(app, Target::ProfileApp(ch as usize - '0' as usize));
            }
            true
        }
        KeyCode::Char('[') => {
            app.profile.prev_section();
            true
        }
        KeyCode::Char(']') => {
            app.profile.next_section();
            true
        }
        KeyCode::Char('p') => {
            activate(app, Target::ProfileAction(0));
            true
        }
        KeyCode::Char('f') => {
            activate(app, Target::ProfileAction(1));
            true
        }
        KeyCode::Char('c') => {
            activate(app, Target::ProfileAction(2));
            true
        }
        KeyCode::Esc if app.profile.report.is_some() => {
            app.profile.report = None;
            app.set_focus(Target::ProfileCard(app.profile.focus));
            true
        }
        KeyCode::Left | KeyCode::Right if app.profile.report.is_some() => {
            let id = app.profile.report.unwrap();
            let count = app
                .profile
                .snapshot
                .as_ref()
                .and_then(|snapshot| {
                    WIDGETS
                        .iter()
                        .find(|w| w.id == id)
                        .map(|w| point_count(w, snapshot))
                })
                .unwrap_or(0);
            let selected = app
                .profile
                .selected_bucket
                .entry(id)
                .or_insert(count.saturating_sub(1));
            *selected = if key.code == KeyCode::Left {
                selected.saturating_sub(1)
            } else {
                selected.saturating_add(1).min(count.saturating_sub(1))
            };
            true
        }
        KeyCode::PageUp | KeyCode::PageDown => {
            scroll(
                app,
                if key.code == KeyCode::PageUp {
                    -(content_layout(app).content.height as isize)
                } else {
                    content_layout(app).content.height as isize
                },
            );
            true
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if app.profile.report.is_some() {
                scroll(app, 1);
            } else {
                app.profile.next_widget();
                app.set_focus(Target::ProfileCard(app.profile.focus));
            }
            true
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if app.profile.report.is_some() {
                scroll(app, -1);
            } else {
                app.profile.prev_widget();
                app.set_focus(Target::ProfileCard(app.profile.focus));
            }
            true
        }
        KeyCode::Left | KeyCode::Right => {
            if key.code == KeyCode::Left {
                app.profile.prev_widget();
            } else {
                app.profile.next_widget();
            }
            app.set_focus(Target::ProfileCard(app.profile.focus));
            true
        }
        KeyCode::Enter => match app.focus {
            Target::ProfileTab(_)
            | Target::ProfileApp(_)
            | Target::ProfileAction(_)
            | Target::ProfileCard(_) => {
                activate(app, app.focus);
                true
            }
            _ => false,
        },
        KeyCode::Char('m') => {
            activate(app, Target::ProfileCard(app.profile.focus));
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
    let width = width.saturating_sub(4);
    out.extend(charts::trend_columns(&totals, width, 4));
    for bucket in shown {
        let series = vec![("total".to_owned(), bucket.total as u64)];
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
    let max = distribution
        .bands
        .iter()
        .map(|band| band.initial.max(band.current) as u64)
        .max()
        .unwrap_or(0);
    for band in &distribution.bands {
        for (key, value) in [("initial", band.initial), ("current", band.current)] {
            let label = format!("{} {key}={value}", band.label);
            out.push(Line::raw(label));
            let mut bar = charts::rank_bar(value as u64, max, width);
            for span in &mut bar.spans {
                span.style = charts::series_style(key);
            }
            out.push(bar);
        }
    }
    out.push(Line::from(vec![
        Span::styled("mean  ", theme::dim()),
        Span::styled(
            format!(
                "{}  {}",
                charts::percent(distribution.mean_initial),
                charts::percent(distribution.mean_current)
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
