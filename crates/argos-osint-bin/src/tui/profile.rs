//! Profile dashboard: the Overview and System tabs.
//!
//! Overview renders 20 primary analytics views as aligned cards and scrollable reports.
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
use super::profile_components::{self as panels, Kpi};
use super::profile_layout::{DashboardPage, LayoutResult, ReportLayout};
use super::theme;

/// Which Profile tab is on screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SystemTab {
    /// Activity: the 20 primary analytics views.
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
        return RendererKind::Ranked;
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
    if same_id(id, "models.capacity")
        || same_id(id, "atlas.backlog")
        || same_id(id, "summary.attention")
        || same_id(id, "recon.unresolved")
    {
        return &[];
    }
    if same_id(id, "intel.reports")
        || same_id(id, "recon.outcomes")
        || same_id(id, "recon.directives")
        || same_id(id, "atlas.cycles")
    {
        return &["app", "mode"];
    }
    if same_id(id, "intel.enrichment")
        || same_id(id, "intel.publishers")
        || same_id(id, "intel.freshness")
    {
        return &["app", "category"];
    }
    if same_id(id, "recon.stages") {
        return &["app", "mode", "category"];
    }
    if same_id(id, "recon.diversity") {
        return &["category"];
    }
    if same_id(id, "atlas.temperature") {
        return &["app"];
    }
    if same_id(id, "models.performance") {
        return &["app", "provider", "role", "mode"];
    }
    if same_id(id, "models.latency") || same_id(id, "models.queue") {
        return &["app", "provider", "role", "mode"];
    }
    if same_id(id, "tools.search_health") || same_id(id, "tools.evidence") {
        return &["app", "provider", "mode", "tool"];
    }
    &["app", "provider", "mode", "tool", "category"]
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
            "provider",
            "quota_group",
            "scope",
            "sends_60s",
            "effective_rpm",
            "pace_per_min",
            "active",
            "max_concurrency",
            "queued",
            "oldest_wait_ms",
            "cooldown_ms",
            "quota_source",
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

/// Primary inventory. Summary reuses these views; merged details are not entries.
pub const WIDGETS: &[WidgetSpec] = &[
    widget!("intel.enrichment", Section::Intel, "Enrichment", 1),
    widget!("intel.reports", Section::Intel, "Report outcomes", 2),
    widget!("intel.publishers", Section::Intel, "Publishers", 3),
    widget!("intel.freshness", Section::Intel, "Article freshness", 4),
    widget!("recon.outcomes", Section::Recon, "Recon outcomes", 1),
    widget!(
        "recon.directives",
        Section::Recon,
        "Directive resolution",
        2
    ),
    widget!("recon.stages", Section::Recon, "Stage durations", 3),
    widget!(
        "recon.diversity",
        Section::Recon,
        "Corroboration diversity",
        4
    ),
    widget!(
        "recon.unresolved",
        Section::Recon,
        "Unresolved directives · Live",
        5
    ),
    widget!("atlas.backlog", Section::Atlas, "Atlas backlog · Live", 1),
    widget!("atlas.cycles", Section::Atlas, "Cycle outcomes", 2),
    widget!("atlas.temperature", Section::Atlas, "Temperature shifts", 3),
    widget!(
        "models.capacity",
        Section::Models,
        "Provider capacity · Live",
        1
    ),
    widget!(
        "models.performance",
        Section::Models,
        "Provider / model performance",
        2
    ),
    widget!(
        "models.latency",
        Section::Models,
        "Successful model latency",
        3
    ),
    widget!("models.queue", Section::Models, "Enqueue-to-send delay", 4),
    widget!("tools.search_health", Section::Tools, "Search health", 1),
    widget!("tools.reliability", Section::Tools, "Tool reliability", 2),
    widget!("tools.evidence", Section::Tools, "Evidence yield", 3),
    widget!(
        "tools.failure_causes",
        Section::Tools,
        "Terminal failure causes",
        4
    ),
];
pub const WIDGET_COUNT: usize = 20;
/// A Summary-only composition, deliberately outside the primary inventory.
const ATTENTION: WidgetSpec = WidgetSpec {
    detail_columns: &["app", "owner_id", "item_id", "reason", "age_ms", "action"],
    applicable_filters: &[],
    ..widget!(
        "summary.attention",
        Section::Recon,
        "Needs attention · Live",
        0
    )
};

#[cfg(test)]
mod registry_tests {
    use super::*;

    #[test]
    fn capacity_preserves_unknown_disabled_and_overflow_values() {
        let mut models = profile_stats::ModelStats {
            capacity_available: true,
            ..Default::default()
        };
        models
            .capacity
            .push(argos_osint_core::provider_metrics::CapacityRow {
                sends_60s: 125,
                active: 7,
                effective_rpm: Some(100),
                max_concurrency: Some(4),
                pace_per_min: Some(32.5),
                ..Default::default()
            });
        let text = |models: &profile_stats::ModelStats| {
            capacity_lines(models, 80, 4)
                .iter()
                .map(|line| {
                    line.spans
                        .iter()
                        .map(|span| span.content.as_ref())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        let overflow = text(&models);
        assert!(overflow.contains("125/100 sends/60s !"));
        assert!(overflow.contains("7/4 active !"));
        assert!(overflow.contains("pace 32.5 req/min"));
        models.capacity[0].effective_rpm = None;
        models.capacity[0].max_concurrency = Some(0);
        let unavailable = text(&models);
        assert!(unavailable.contains("125/N/A sends/60s"));
        assert!(unavailable.contains("7/N/A active"));
        assert!(!unavailable.contains(" !"));
    }

    /// The inventory is a table, so the count is asserted rather than estimated.
    #[test]
    fn exactly_the_reviewed_widgets_are_registered() {
        assert_eq!(WIDGETS.len(), WIDGET_COUNT);
        assert_eq!(WIDGET_COUNT, 20);
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
        // Intel 4, Recon 5, Atlas 3, Models 4, Tools 4.
        assert_eq!(counts, vec![4, 5, 3, 4, 4]);
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
            "intel.volume",
            "intel.confidence",
            "intel.origins",
            "recon.recall",
            "recon.workload",
            "atlas.hot_zones",
            "atlas.cycle_time",
            "atlas.discovery",
            "models.by_role",
            "models.fallback",
            "models.amplification",
            "models.failures",
            "tools.usage",
            "tools.attribution",
            "tools.outcomes",
        ] {
            assert!(
                !WIDGETS.iter().any(|w| w.id == stale),
                "legacy widget {stale} is still registered"
            );
        }
    }
}

/// Dashboard state. One struct on `App`, mirroring `JobsView` and `LogsView`.
type DetailCacheKey = (&'static str, u16, usize, usize, bool);
type DetailCache = std::cell::RefCell<HashMap<DetailCacheKey, Vec<Line<'static>>>>;
#[derive(Clone, Debug)]
pub struct ProfileView {
    pub tab: SystemTab,
    pub all_apps: bool,
    pub report: Option<&'static str>,
    pub grid_scroll: ScrollPane,
    pub report_scroll: ScrollPane,
    pub panel_scroll: HashMap<&'static str, ScrollPane>,
    pub table_view: bool,
    pub sort_descending: bool,
    pub sort_column: usize,
    pub collapsed_providers: HashMap<String, bool>,
    pub report_origin: Option<(usize, usize, Target)>,
    pub report_offsets: HashMap<&'static str, usize>,
    pub detail_cache: DetailCache,
    pub selected_row_keys: HashMap<&'static str, String>,
    pub selected_row: HashMap<&'static str, usize>,
    pub custom_period: bool,
    pub picker_error: Option<String>,
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
            panel_scroll: HashMap::new(),
            table_view: false,
            sort_descending: false,
            sort_column: 0,
            collapsed_providers: HashMap::new(),
            report_origin: None,
            report_offsets: HashMap::new(),
            detail_cache: std::cell::RefCell::new(HashMap::new()),
            selected_row_keys: HashMap::new(),
            selected_row: HashMap::new(),
            custom_period: false,
            picker_error: None,
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
                "recon.outcomes",
                "atlas.backlog",
                "summary.attention",
                "models.latency",
            ]
            .iter()
            .filter_map(|id| view_spec(id))
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
        self.detail_cache.borrow_mut().clear();
        for (id, key) in &self.selected_row_keys {
            if let Some(w) = view_spec(id) {
                let rows = ordered_rows(self, w, &snapshot);
                let index = rows
                    .iter()
                    .position(|r| row_identity(r) == *key)
                    .unwrap_or(0);
                self.selected_row.insert(id, index);
            }
        }
        for (id, index) in &mut self.selected_bucket {
            if let Some(widget) = view_spec(id) {
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
        self.detail_cache.borrow_mut().clear();
        self.generation = self.generation.wrapping_add(1);
        self.loaded_at = None;
    }

    pub fn picker_choices(&self) -> Vec<String> {
        if self.period_popup {
            let mut choices: Vec<_> = profile_stats::Period::ALL
                .iter()
                .map(|p| p.label().to_owned())
                .collect();
            choices.push("Custom range (RFC3339 with timezone)".into());
            return choices;
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

    /// Automatic snapshots refresh every 20 seconds, throttled by the caller.
    pub fn due(&self) -> bool {
        match self.loaded_at {
            None => true,
            Some(at) => at.elapsed() >= std::time::Duration::from_secs(20),
        }
    }

    pub fn next_section(&mut self) {
        let all = Section::all();
        let next = if self.all_apps {
            0
        } else {
            (self.section_index() + 1) % all.len()
        };
        let summary = !self.all_apps && self.section_index() == all.len() - 1;
        self.all_apps = summary;
        self.report = None;
        self.grid_scroll.offset = 0;
        self.section = all[next];
        self.focus = 0;
    }

    pub fn prev_section(&mut self) {
        let all = Section::all();
        let prev = if self.all_apps {
            all.len() - 1
        } else {
            (self.section_index() + all.len() - 1) % all.len()
        };
        let summary = !self.all_apps && self.section_index() == 0;
        self.all_apps = summary;
        self.report = None;
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
    let layout = dashboard_layout(app, area, app.profile.grid_scroll.offset);
    draw_tab_strip(frame, app, layout.tabs);
    match app.profile.tab {
        SystemTab::Overview => {
            draw_section_navigator(frame, app, layout.apps);
            draw_filter_strip(frame, app, layout.controls);
            if !layout.too_small {
                for (metric, rect) in kpis(app).iter().zip(&layout.kpis) {
                    panels::kpi(frame, *rect, metric);
                }
            }
            if layout.too_small {
                let y = layout.controls.bottom();
                frame.render_widget(
                    Paragraph::new("Profile needs 60×18 · resize to view analytics")
                        .style(theme::warn()),
                    Rect::new(area.x, y, area.width, area.bottom().saturating_sub(y)),
                );
            } else {
                draw_section_body(frame, app, layout.content);
            }
            if let Some(id) = app.profile.report {
                if let Some(widget) = view_spec(id) {
                    let popup = expanded_area(area);
                    frame.render_widget(ratatui::widgets::Clear, popup);
                    draw_report(frame, app, widget, popup);
                }
            }
        }
        SystemTab::System => {
            draw_system_tab(frame, app, strip(area, 1, area.height.saturating_sub(2)))
        }
    }
    if app.profile.filter_popup.is_some() || app.profile.period_popup {
        draw_picker(frame, app, area);
    }
}
fn expanded_area(area: Rect) -> Rect {
    let w = area.width * 9 / 10;
    let h = area.height * 9 / 10;
    Rect::new(
        area.x + (area.width - w) / 2,
        area.y + (area.height - h) / 2,
        w,
        h,
    )
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
fn system_panes(app: &App, area: Rect) -> Vec<(usize, Rect)> {
    let (content, _) = super::ui::system_areas(area);
    if content.height < 27 {
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
    }
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
    if content.height < 27 {
        super::ui::pane_tabs(
            frame,
            app,
            Rect::new(content.x, content.y, content.width, 1),
            &["Host", "Paths"],
        );
    }
    let panes = system_panes(app, area);
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
                ratatui::style::Style::default()
                    .fg(theme::BG)
                    .bg(theme::ACCENT)
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
                    Span::styled(" · views ", theme::muted()),
                    Span::styled(charts::count(WIDGET_COUNT as u64), theme::text()),
                    Span::styled(" · updated ", theme::muted()),
                    Span::styled(
                        format!("{} UTC", snapshot.captured_at.trim_end_matches('Z')),
                        theme::dim(),
                    ),
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
    let x = area.x.saturating_add(used).min(area.right());
    if active.is_empty() {
        draw_status_strip(frame, app, Rect::new(x, area.y, area.right() - x, 1));
    } else {
        let x = area.x.saturating_add(used).min(area.right());
        frame.render_widget(
            Paragraph::new(format!(
                "{}{}",
                if let Some(error) = &app.profile.error {
                    format!("Stale · {error} · ")
                } else if app.profile.loading {
                    "Refreshing · ".into()
                } else {
                    String::new()
                },
                active.join(" · ")
            ))
            .style(theme::accent()),
            Rect::new(x, area.y, area.right() - x, area.height),
        );
    }
}
fn draw_section_navigator(frame: &mut Frame, app: &App, area: Rect) {
    // On compact screens expose a measured current-app selector; activation cycles all six choices.
    if area.width < 66 {
        let label = if app.profile.all_apps {
            "Summary"
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
        let mut actions = vec![(Target::ProfileApp(0), " Summary ".into())];
        actions.extend(
            Section::all()
                .iter()
                .enumerate()
                .map(|(i, section)| (Target::ProfileApp(i + 1), format!(" {} ", section.label()))),
        );
        draw_actions(frame, app, area, &actions);
    }
}
pub fn reveal_focus(app: &mut App) {
    let layout = content_layout(app);
    app.profile.grid_scroll.reveal(
        layout.focus_start(app.profile.focus),
        layout
            .panel_height(app.profile.focus)
            .min(layout.content.height as usize),
        layout.content.height as usize,
    );
    app.profile
        .grid_scroll
        .scroll(0, layout.extent(), layout.content.height as usize);
}
fn row_identity(row: &serde_json::Value) -> String {
    [
        "app",
        "owner_id",
        "item_id",
        "run_id",
        "label",
        "tool_id",
        "engine",
        "provider",
        "model",
        "role",
        "scope",
        "quota_group",
        "stage",
        "unit",
        "mode",
        "tag",
        "domain",
        "origin",
        "bucket",
        "cause",
    ]
    .iter()
    .filter_map(|key| row.get(*key).map(|v| format!("{key}:{v}")))
    .collect::<Vec<_>>()
    .join("|")
}
fn compare_cells(a: &serde_json::Value, b: &serde_json::Value) -> std::cmp::Ordering {
    match (a.as_f64(), b.as_f64()) {
        (Some(a), Some(b)) => a.total_cmp(&b),
        _ => a.to_string().cmp(&b.to_string()),
    }
}
fn sorted_rows(app: &App, w: &WidgetSpec, s: &ProfileSnapshot) -> Vec<serde_json::Value> {
    ordered_rows(&app.profile, w, s)
}
fn ordered_rows(
    profile: &ProfileView,
    w: &WidgetSpec,
    s: &ProfileSnapshot,
) -> Vec<serde_json::Value> {
    let data = widget_data(w, s);
    let mut rows = data.as_array().cloned().unwrap_or_default();
    if let Some(key) = w.detail_columns.get(profile.sort_column) {
        if profile.sort_column > 0 || profile.sort_descending {
            rows.sort_by(|a, b| {
                compare_cells(&a[*key], &b[*key])
                    .then_with(|| row_identity(a).cmp(&row_identity(b)))
            });
            if profile.sort_descending {
                rows.reverse();
            }
        }
    }
    if w.id == "models.performance" {
        rows.retain(|row| {
            !row["is_model_row"].as_bool().unwrap_or(false)
                || !profile
                    .collapsed_providers
                    .get(row["provider"].as_str().unwrap_or(""))
                    .copied()
                    .unwrap_or(false)
        });
        rows.sort_by(|a, b| {
            a["provider"]
                .to_string()
                .cmp(&b["provider"].to_string())
                .then_with(|| {
                    a["is_model_row"]
                        .as_bool()
                        .unwrap_or(false)
                        .cmp(&b["is_model_row"].as_bool().unwrap_or(false))
                })
        });
    }
    rows
}
fn remember_row(app: &mut App, id: &'static str, index: usize) {
    app.profile.selected_row.insert(id, index);
    if let (Some(w), Some(snapshot)) = (view_spec(id), app.profile.snapshot.as_ref()) {
        if let Some(row) = sorted_rows(app, w, snapshot).get(index) {
            app.profile.selected_row_keys.insert(id, row_identity(row));
        }
    }
    app.profile.detail_cache.borrow_mut().clear();
}
fn select_row(app: &mut App, delta: isize) {
    let id = app
        .profile
        .report
        .or_else(|| app.profile.focused_widget().map(|w| w.id));
    let Some(w) = id.and_then(view_spec) else {
        return;
    };
    let Some(s) = app.profile.snapshot.as_ref() else {
        return;
    };
    let rows = sorted_rows(app, w, s);
    let n = rows.len();
    let index = app.profile.selected_row.entry(w.id).or_default();
    *index = index.saturating_add_signed(delta).min(n.saturating_sub(1));
    if let Some(row) = rows.get(*index) {
        app.profile
            .selected_row_keys
            .insert(w.id, row_identity(row));
    }
    let selected = *index;
    let layout = content_layout(app);
    if app.profile.report.is_none() {
        app.profile.panel_scroll.entry(w.id).or_default().reveal(
            selected + 1,
            1,
            layout.panel_height(app.profile.focus).saturating_sub(2),
        );
    }
}
fn open_selected_owner(app: &mut App) {
    let Some(id) = app.profile.report else {
        return;
    };
    let Some(s) = app.profile.snapshot.as_ref() else {
        return;
    };
    let Some(w) = view_spec(id) else {
        return;
    };
    let rows = sorted_rows(app, w, s);
    let index = app.profile.selected_row.get(id).copied().unwrap_or(0);
    let Some(row) = rows.get(index) else {
        return;
    };
    if id == "recon.unresolved" {
        if let Some(run) = row["run_id"].as_str() {
            open_run_owner(app, run);
        }
        return;
    }
    if id != ATTENTION.id {
        return;
    }
    let owner = row["owner_id"].as_str().unwrap_or("").to_owned();
    match row["owner_kind"].as_str().unwrap_or("") {
        "run" => open_run_owner(app, &owner),
        "intel_job" => {
            app.profile.report = None;
            if let Err(error) = app.open_profile_intel_job(&owner) {
                app.status = format!("Could not open report: {error}");
            }
        }
        "atlas_run" => {
            app.profile.report = None;
            app.select(super::app::ModuleId::Atlas.index());
            if let Some(index) = app.atlas_runs.iter().position(|run| run.id == owner) {
                app.activate_target(Target::AtlasHistory(index));
            } else {
                app.status = "Atlas cycle is no longer retained".into();
            }
        }
        "provider_scope" => {
            let capacity = view_spec("models.capacity").unwrap();
            let group = row["item_id"].as_str().unwrap_or("");
            let index = sorted_rows(app, capacity, s).iter().position(|r| {
                format!(
                    "{} {}",
                    r["provider"].as_str().unwrap_or(""),
                    r["scope"].as_str().unwrap_or("")
                ) == owner
                    && r["quota_group"].as_str().unwrap_or("") == group
            });
            let Some(index) = index else {
                app.status = "Capacity scope is no longer retained".into();
                return;
            };
            remember_row(app, capacity.id, index);
            app.profile.report = Some("models.capacity");
            app.profile.report_scroll.offset = index.saturating_sub(2);
            app.profile.table_view = true;
            app.set_focus(Target::ProfileReport);
        }
        _ => app.status = "No retained owner for this attention item".into(),
    }
}
fn open_run_owner(app: &mut App, run_id: &str) {
    match app.store.get_run(run_id) {
        Ok(Some(run)) => {
            app.profile.report = None;
            app.select(super::app::ModuleId::Recon.index());
            if let Err(error) = app.open_thread(&run.thread_id) {
                app.status = format!("Could not open investigation: {error}");
            }
        }
        Ok(None) => app.status = "Investigation is no longer retained".into(),
        Err(error) => app.status = format!("Could not load investigation: {error}"),
    }
}

fn w_is_time(widget: &WidgetSpec) -> bool {
    widget.renderer == RendererKind::Time
}
fn view_spec(id: &str) -> Option<&'static WidgetSpec> {
    if id == ATTENTION.id {
        Some(&ATTENTION)
    } else {
        WIDGETS.iter().find(|w| w.id == id)
    }
}
fn content_layout(app: &App) -> LayoutResult {
    dashboard_layout(
        app,
        super::ui::body_rect(app),
        app.profile.grid_scroll.offset,
    )
}
fn dashboard_layout(app: &App, area: Rect, offset: usize) -> LayoutResult {
    let page = if app.profile.all_apps {
        DashboardPage::Summary
    } else {
        match app.profile.section {
            Section::Recon => DashboardPage::Recon,
            Section::Atlas => DashboardPage::Atlas,
            _ => DashboardPage::Other,
        }
    };
    LayoutResult::new(area, page, app.profile.focused_widgets().len(), offset)
}
fn grid_offset(app: &App, layout: &LayoutResult) -> usize {
    app.profile.grid_scroll.offset.min(
        layout
            .extent()
            .saturating_sub(layout.content.height as usize),
    )
}
fn draw_section_body(frame: &mut Frame, app: &App, area: Rect) {
    let widgets = app.profile.focused_widgets();
    let initial = content_layout(app);
    let layout = dashboard_layout(app, super::ui::body_rect(app), grid_offset(app, &initial));
    if layout.too_small {
        frame.render_widget(
            Paragraph::new("Profile needs 60×18 · resize to view analytics").style(theme::warn()),
            area,
        );
        return;
    }
    for geometry in &layout.panels {
        let widget = widgets[geometry.index];
        app.layout
            .borrow_mut()
            .register(Target::ProfileCard(geometry.index), geometry.rect);
        let width = geometry.rect.width.saturating_sub(4) as usize;
        let height = geometry.height.saturating_sub(2);
        let mut lines = dashboard_lines(app, widget, width, height);
        let logical_rows = app
            .profile
            .snapshot
            .as_ref()
            .map(|s| sorted_rows(app, widget, s).len())
            .unwrap_or(0);
        let content_offset = app
            .profile
            .panel_scroll
            .get(widget.id)
            .map(|p| p.offset)
            .unwrap_or(0)
            .min(lines.len().saturating_sub(height));
        if widget.renderer != RendererKind::Time {
            let offset = app
                .profile
                .panel_scroll
                .get(widget.id)
                .map(|p| p.offset)
                .unwrap_or(0)
                .min(lines.len().saturating_sub(height));
            let hidden = offset + height < lines.len();
            lines = lines.into_iter().skip(offset).take(height).collect();
            if hidden && height > 0 {
                lines.truncate(height - 1);
                lines.push(Line::styled("See more · Enter", theme::accent()));
            }
        }
        if widget.renderer == RendererKind::Table || widget.id == ATTENTION.id {
            let prefix = usize::from(
                widget.applicable_filters.is_empty() || widget.id == "recon.unresolved",
            ) + usize::from(
                !widget.applicable_filters.is_empty()
                    && ["app", "provider", "role", "mode", "tool", "category"]
                        .iter()
                        .any(|d| {
                            !app.profile.filter_value(d).is_empty()
                                && !widget.applicable_filters.contains(d)
                        }),
            );
            for row in 0..logical_rows {
                let line = prefix + 1 + row;
                if line < content_offset {
                    continue;
                }
                let virtual_y = 1 + line - content_offset;
                if virtual_y >= geometry.source_offset
                    && virtual_y < geometry.source_offset + geometry.rect.height as usize
                    && virtual_y < geometry.height - 1
                {
                    app.layout.borrow_mut().register(
                        Target::ProfilePanelRow(geometry.index, row),
                        Rect::new(
                            geometry.rect.x + 2,
                            geometry.rect.y + (virtual_y - geometry.source_offset) as u16,
                            width as u16,
                            1,
                        ),
                    );
                }
            }
        }
        if w_is_time(widget) {
            if let Some(snapshot) = &app.profile.snapshot {
                let buckets = count_buckets(widget, snapshot);
                let series = buckets.is_empty();
                let count = if series {
                    series_points(widget, snapshot).len()
                } else {
                    buckets.len()
                };
                let selected = app
                    .profile
                    .selected_bucket
                    .get(widget.id)
                    .copied()
                    .unwrap_or(count.saturating_sub(1));
                let first = lines
                    .iter()
                    .position(|line| line.to_string().contains('│'))
                    .unwrap_or(0);
                let rows = lines
                    .iter()
                    .skip(first)
                    .take_while(|line| line.to_string().contains('│'))
                    .count();
                let source_top = geometry.source_offset;
                let top = (first + 1).max(source_top);
                let bottom = (first + 1 + rows).min(source_top + geometry.rect.height as usize);
                if bottom > top {
                    for (index, x, slot) in charts::plot_targets(count, width, selected, series) {
                        app.layout.borrow_mut().register(
                            Target::ProfilePanelBucket(geometry.index, index),
                            Rect::new(
                                geometry.rect.x + 2 + x as u16,
                                geometry.rect.y + (top - source_top) as u16,
                                slot as u16,
                                (bottom - top) as u16,
                            ),
                        );
                    }
                }
            }
        }
        panels::panel(
            frame,
            geometry,
            widget.title,
            app.focus == Target::ProfileCard(geometry.index),
            panels::clipped(lines, width),
        );
    }
}
fn rate_kpi(label: &'static str, numerator: u64, denominator: u64, note: &str) -> Kpi {
    Kpi {
        label,
        value: if denominator == 0 {
            "N/A".into()
        } else {
            format!("{:.1}%", numerator as f64 * 100.0 / denominator as f64)
        },
        coverage: format!("{numerator}/{denominator} · {note}"),
    }
}
fn count_kpi(label: &'static str, value: Option<u64>, note: String) -> Kpi {
    Kpi {
        label,
        value: value.map(charts::count).unwrap_or_else(|| "N/A".into()),
        coverage: note,
    }
}
fn kpis(app: &App) -> Vec<Kpi> {
    let Some(s) = app.profile.snapshot.as_ref() else {
        let labels = if app.profile.all_apps {
            [
                "Report completion",
                "Answered directives",
                "Atlas completion",
                "Successful model p95",
            ]
        } else {
            match app.profile.section {
                Section::Intel => [
                    "Distinct new articles",
                    "Body availability",
                    "Report completion",
                    "Top-three publisher share",
                ],
                Section::Recon => [
                    "Terminal runs",
                    "Answered directives",
                    "Unresolved · Live",
                    "Corroboration coverage",
                ],
                Section::Atlas => [
                    "Terminal cycles",
                    "Completed share",
                    "Blocked · Live",
                    "Oldest pending · Live",
                ],
                Section::Models => [
                    "Sends",
                    "Final operation failures",
                    "Queue p95",
                    "Active / max · Live",
                ],
                Section::Tools => [
                    "Logical calls",
                    "Usable search fetches",
                    "Remote errors",
                    "Evidence yield",
                ],
            }
        };
        return labels
            .into_iter()
            .map(|label| Kpi {
                label,
                value: "N/A".into(),
                coverage: "Loading statistics".into(),
            })
            .collect();
    };
    let report_complete = s.intel.reports.iter().map(|r| r.completed as u64).sum();
    let report_n = s
        .intel
        .reports
        .iter()
        .map(|r| (r.completed + r.partial + r.failed) as u64)
        .sum();
    let answered = s.recon.directives.iter().map(|r| r.answered as u64).sum();
    let directive_n = s.recon.directives.iter().map(|r| r.n as u64).sum();
    let cycle_complete = s
        .atlas
        .cycle_outcomes
        .iter()
        .map(|r| r.completed as u64)
        .sum();
    let cycle_n = s
        .atlas
        .cycle_outcomes
        .iter()
        .map(|r| (r.completed + r.partial + r.failed + r.cancelled) as u64)
        .sum();
    let report = || {
        rate_kpi(
            "Report completion",
            report_complete,
            report_n,
            "latest revisions",
        )
    };
    let directives = || {
        rate_kpi(
            "Answered directives",
            answered,
            directive_n,
            "assessments; unknown included",
        )
    };
    let atlas = || {
        rate_kpi(
            "Atlas completion",
            cycle_complete,
            cycle_n,
            "cancelled included",
        )
    };
    let latency = || Kpi {
        label: "Successful model p95",
        value: charts::duration_ms(s.models.period_summary.successful_execution_p95_ms),
        coverage: format!(
            "N={} · successful executions",
            s.models.period_summary.successful_execution_n
        ),
    };
    if app.profile.all_apps {
        return vec![report(), directives(), atlas(), latency()];
    }
    match app.profile.section {
        Section::Intel => vec![
            count_kpi(
                "Distinct new articles",
                s.intel.distinct_new_articles,
                "Distinct cohort".into(),
            ),
            match (
                s.intel.body_available_articles,
                s.intel.body_eligible_articles,
            ) {
                (Some(a), Some(n)) => rate_kpi("Body availability", a, n, "distinct articles"),
                _ => Kpi {
                    label: "Body availability",
                    value: "N/A".into(),
                    coverage: "Distinct body coverage unavailable".into(),
                },
            },
            report(),
            Kpi {
                label: "Top-three publisher share",
                value: charts::percent(s.intel.publishers.first().map(|r| r.top3_concentration)),
                coverage: format!(
                    "N={} · publisher cohort",
                    s.intel
                        .publishers
                        .iter()
                        .map(|r| r.articles as u64)
                        .sum::<u64>()
                ),
            },
        ],
        Section::Recon => {
            let runs = s
                .recon
                .outcomes
                .iter()
                .map(|r| {
                    (r.completed_with_evidence
                        + r.completed_zero_evidence
                        + r.partial
                        + r.failed
                        + r.cancelled) as u64
                })
                .sum();
            let eligible = s
                .recon
                .diversity
                .iter()
                .filter(|r| r.eligible)
                .map(|r| r.eligible_scopes as u64)
                .sum();
            let corroborated = s
                .recon
                .diversity
                .iter()
                .filter(|r| r.eligible)
                .map(|r| r.scopes_with_two_successful as u64)
                .sum();
            vec![
                count_kpi("Terminal runs", Some(runs), "Cancelled included".into()),
                directives(),
                count_kpi(
                    "Unresolved · Live",
                    Some(s.recon.unresolved_live_total),
                    "Live · ignores period".into(),
                ),
                rate_kpi(
                    "Corroboration coverage",
                    corroborated,
                    eligible,
                    "eligible category scopes",
                ),
            ]
        }
        Section::Atlas => {
            // Never add unlike backlog units. Each stage/unit is visible verbatim.
            let blocked = s
                .atlas
                .backlog
                .iter()
                .filter(|r| r.blocked > 0)
                .map(|r| format!("{} {} {}", r.blocked, r.unit, r.stage))
                .collect::<Vec<_>>()
                .join("; ");
            vec![
                count_kpi(
                    "Terminal cycles",
                    Some(cycle_n),
                    "Cancelled included".into(),
                ),
                rate_kpi(
                    "Completed share",
                    cycle_complete,
                    cycle_n,
                    "cancelled included",
                ),
                Kpi {
                    label: "Blocked · Live",
                    value: if blocked.is_empty() {
                        "0".into()
                    } else {
                        blocked
                    },
                    coverage: "By stage / unit · Live".into(),
                },
                Kpi {
                    label: "Oldest pending · Live",
                    value: charts::duration_ms(
                        s.atlas
                            .backlog
                            .iter()
                            .filter_map(|r| r.oldest_pending_ms)
                            .max(),
                    ),
                    coverage: "Live · ignores period".into(),
                },
            ]
        }
        Section::Models => {
            let rows: Vec<_> = s
                .models
                .capacity
                .iter()
                .filter(|r| {
                    (s.filters.provider.is_empty() || s.filters.provider == r.provider)
                        && (s.filters.role.is_empty() || s.filters.role == r.scope)
                })
                .collect();
            let capacity = if rows.len() == 1 {
                format!(
                    "{} / {}",
                    rows[0].active,
                    rows[0]
                        .max_concurrency
                        .filter(|n| *n > 0)
                        .map(|n| n.to_string())
                        .unwrap_or_else(|| "N/A".into())
                )
            } else {
                "N/A".into()
            };
            vec![
                count_kpi(
                    "Sends",
                    Some(s.models.period_summary.sends),
                    "Actual wire sends".into(),
                ),
                count_kpi(
                    "Final operation failures",
                    Some(s.models.period_summary.final_failures),
                    format!(
                        "N={} terminal · cancelled included",
                        s.models.period_summary.terminal_operations
                    ),
                ),
                Kpi {
                    label: "Queue p95",
                    value: charts::duration_ms(s.models.period_summary.queue_p95_ms),
                    coverage: format!(
                        "N={} · measured enqueue-to-send",
                        s.models.period_summary.queue_n
                    ),
                },
                Kpi {
                    label: "Active / max · Live",
                    value: capacity,
                    coverage: if rows.len() == 1 {
                        format!("{} / {} · Live", rows[0].provider, rows[0].scope)
                    } else {
                        "Select one quota scope · Live".into()
                    },
                },
            ]
        }
        Section::Tools => {
            let calls = s.tools.logical_calls;
            let usable = s
                .tools
                .engine_health
                .iter()
                .map(|r| r.valid_serps + r.verified_zero)
                .sum();
            let fetches = s.tools.engine_health.iter().map(|r| r.fetches).sum();
            // Remote error percentages cannot reconstruct exact eligible counts.
            let errors = rate_kpi(
                "Remote errors",
                s.tools.remote_errors,
                s.tools.eligible_remote_calls,
                "eligible remote calls",
            );
            let evidence = s.tools.evidence.iter().map(|r| r.with_evidence).sum();
            let eligible = s.tools.evidence.iter().map(|r| r.successful_nonempty).sum();
            vec![
                count_kpi("Logical calls", Some(calls), "Logical invocations".into()),
                rate_kpi(
                    "Usable search fetches",
                    usable,
                    fetches,
                    "valid + verified zero",
                ),
                errors,
                if s.tools
                    .evidence
                    .iter()
                    .any(|r| r.successful_nonempty > 0 && r.acceptance_pct.is_none())
                {
                    Kpi {
                        label: "Evidence yield",
                        value: "N/A".into(),
                        coverage: format!("Linked {evidence}/{eligible} · incomplete provenance"),
                    }
                } else {
                    rate_kpi(
                        "Evidence yield",
                        evidence,
                        eligible,
                        "logical calls; identities nonadditive",
                    )
                },
            ]
        }
    }
}
fn attention_lines(app: &App, width: usize) -> Vec<Line<'static>> {
    let Some(s) = &app.profile.snapshot else {
        return vec![Line::raw("Loading statistics")];
    };
    if s.attention.is_empty() {
        return vec![
            Line::raw("No recorded blockers in available telemetry."),
            Line::styled(format!("Coverage since {}", s.observed_since), theme::dim()),
        ];
    }
    let selected = app
        .profile
        .selected_row
        .get(ATTENTION.id)
        .copied()
        .unwrap_or(0);
    let mut lines = vec![panels::table_row(
        &[
            "App / item".into(),
            "Reason".into(),
            "Age".into(),
            "Action".into(),
        ],
        &[30, 35, 10, 25],
        width,
        false,
    )];
    for (i, row) in sorted_rows(app, &ATTENTION, s).into_iter().enumerate() {
        let Ok(r) = serde_json::from_value::<profile_stats::AttentionRow>(row) else {
            continue;
        };
        lines.push(panels::table_row(
            &[
                format!(
                    "{} / {}",
                    r.app,
                    if r.item_id.is_empty() {
                        &r.owner_id
                    } else {
                        &r.item_id
                    }
                ),
                r.reason.clone(),
                charts::duration_ms(r.age_ms),
                r.action.clone(),
            ],
            &[30, 35, 10, 25],
            width,
            i == selected,
        ));
    }
    lines
}
fn dashboard_lines(app: &App, w: &WidgetSpec, width: usize, height: usize) -> Vec<Line<'static>> {
    let Some(s) = app.profile.snapshot.as_ref() else {
        return vec![Line::raw(if app.profile.error.is_some() {
            "Unavailable statistics"
        } else {
            "Loading statistics"
        })];
    };
    let mut lines = Vec::new();
    if w.applicable_filters.is_empty() {
        lines.push(Line::styled(
            "Live · ignores historical period / dimensions",
            theme::dim(),
        ));
    }
    if !w.applicable_filters.is_empty() {
        let unused = ["app", "provider", "role", "mode", "tool", "category"]
            .into_iter()
            .filter(|d| {
                !app.profile.filter_value(d).is_empty() && !w.applicable_filters.contains(d)
            })
            .collect::<Vec<_>>();
        if !unused.is_empty() {
            lines.push(Line::styled(
                format!("Not applicable: {}", unused.join(", ")),
                theme::muted(),
            ));
        }
    }
    let selected = app
        .profile
        .selected_bucket
        .get(w.id)
        .copied()
        .unwrap_or(point_count(w, s).saturating_sub(1));
    match w.id {
        "summary.attention" => lines.extend(attention_lines(app, width)),
        "recon.outcomes" | "atlas.cycles" => {
            let buckets = count_buckets(w, s);
            let n = buckets.iter().map(charts::TimeBucket::total).sum::<u64>();
            lines.push(Line::styled(
                format!("{} · N={n} · cancelled included", s.filters.period.label()),
                theme::dim(),
            ));
            if let Some(error) = &app.profile.error {
                lines.insert(0, Line::styled(format!("Stale · {error}"), theme::warn()));
            }
            if app.profile.loading {
                lines.insert(
                    0,
                    Line::styled("Refreshing · previous snapshot", theme::warn()),
                );
            }
            if let Some(bucket) = buckets.get(selected) {
                lines.push(Line::styled(
                    format!("Selected {} · N={}", bucket.start, bucket.total()),
                    theme::text(),
                ));
            }
            lines.extend(charts::time_plot(
                &buckets,
                width,
                height.saturating_sub(lines.len() + 1),
                selected,
            ));
            let keys: &[(&str, &str)] = if w.id == "recon.outcomes" {
                &[
                    ("completed_with_evidence", "evidence"),
                    ("completed_zero_evidence", "zero"),
                    ("partial", "partial"),
                    ("failed", "failed"),
                    ("cancelled", "cancelled"),
                ]
            } else {
                &[
                    ("completed", "completed"),
                    ("partial", "partial"),
                    ("failed", "failed"),
                    ("cancelled", "cancelled"),
                ]
            };
            lines.push(Line::from(
                keys.iter()
                    .map(|(key, label)| {
                        Span::styled(format!("{label}  "), charts::series_style(key))
                    })
                    .collect::<Vec<_>>(),
            ));
        }
        "models.latency" | "models.queue" => {
            lines.push(Line::styled(
                format!(
                    "{} · seconds · measured N={}",
                    s.filters.period.label(),
                    series_points(w, s).iter().map(|p| p.n).sum::<u64>()
                ),
                theme::dim(),
            ));
            lines.extend(charts::series_plot(
                &series_points(w, s),
                width,
                height.saturating_sub(lines.len()),
                selected,
            ));
        }
        "intel.reports" => {
            let maximum = s
                .intel
                .reports
                .iter()
                .map(|r| (r.completed + r.partial + r.failed) as u64)
                .max()
                .unwrap_or(0);
            lines.push(Line::styled(
                format!("Latest revisions · reports · scale 0–{maximum}"),
                theme::dim(),
            ));
            for r in &s.intel.reports {
                let series = vec![
                    ("completed".into(), r.completed as u64),
                    ("partial".into(), r.partial as u64),
                    ("failed".into(), r.failed as u64),
                ];
                let label = width.min(18);
                let mut line = charts::stacked_bar(
                    &r.mode,
                    label,
                    &series,
                    width.saturating_sub(label + 10),
                    maximum,
                );
                line.spans.push(Span::raw(format!(
                    " N={}",
                    r.completed + r.partial + r.failed
                )));
                lines.push(line);
            }
            lines.push(legend(&["completed", "partial", "failed"]));
            lines.push(Line::styled(
                format!(
                    "Live waiting {} · blocked {}",
                    s.intel
                        .reports
                        .iter()
                        .map(|r| r.waiting as u64)
                        .sum::<u64>(),
                    s.intel
                        .reports
                        .iter()
                        .map(|r| r.blocked as u64)
                        .sum::<u64>()
                ),
                theme::dim(),
            ));
        }
        "recon.directives" => {
            lines.push(Line::styled(
                "Assessment share · 0–100% · actual mode",
                theme::dim(),
            ));
            for r in &s.recon.directives {
                let series = vec![
                    ("answered".into(), r.answered as u64),
                    ("partial".into(), r.partial as u64),
                    ("unresolved".into(), r.unresolved as u64),
                    ("blocked".into(), r.blocked as u64),
                    ("unknown".into(), r.unknown as u64),
                ];
                let label = width.min(16);
                let mut line = charts::stacked_bar(
                    &r.mode,
                    label,
                    &series,
                    width.saturating_sub(label + 10),
                    r.n as u64,
                );
                line.spans.push(Span::raw(format!(" N={}", r.n)));
                lines.push(line);
            }
            lines.push(legend(&[
                "answered",
                "partial",
                "unresolved",
                "blocked",
                "unknown",
            ]));
        }
        "recon.stages" => {
            let max = s
                .recon
                .stages
                .iter()
                .flat_map(|r| [r.mean_exec_ms, r.mean_wait_ms])
                .flatten()
                .max()
                .unwrap_or(0);
            lines.push(Line::styled(
                format!("Mean seconds · shared scale 0–{:.1}s", max as f64 / 1000.0),
                theme::dim(),
            ));
            for r in &s.recon.stages {
                let label = format!("{} N={}", r.stage, r.n);
                lines.extend(charts::paired_bars(
                    &label,
                    width.min(28),
                    r.mean_exec_ms,
                    r.mean_wait_ms,
                    max,
                    width.saturating_sub(39),
                ));
            }
            lines.push(Line::from(vec![
                Span::styled("execution", theme::accent()),
                Span::styled("  wait", theme::warn()),
            ]));
        }
        "atlas.temperature" => {
            let maximum = s
                .atlas
                .temperature_changes
                .iter()
                .filter(|r| r.comparable)
                .filter_map(|r| r.delta)
                .map(f64::abs)
                .fold(0.0, f64::max);
            lines.push(Line::styled(
                format!("Signed score points · zero centre · ±{maximum:.1}"),
                theme::dim(),
            ));
            for r in &s.atlas.temperature_changes {
                let label_width = width.min(18);
                let mut line = Line::raw(format!(
                    "{:<label_width$} ",
                    components::clip_text(&r.origin, label_width)
                ));
                line.spans.extend(
                    charts::diverging_bar(
                        if r.comparable { r.delta } else { None },
                        maximum,
                        width.saturating_sub(label_width + 12),
                    )
                    .spans,
                );
                line.spans.push(Span::raw(
                    r.delta
                        .filter(|_| r.comparable)
                        .map(|v| format!(" {v:+.1} pt"))
                        .unwrap_or_else(|| " N/A".into()),
                ));
                lines.push(line);
            }
        }
        "intel.publishers" | "tools.failure_causes" => {
            lines.extend(categorical_plot(w, s, width as u16, usize::MAX))
        }
        "intel.freshness" => {
            let max = s
                .intel
                .freshness
                .buckets
                .iter()
                .map(|(_, n)| *n)
                .max()
                .unwrap_or(0);
            lines.push(Line::styled(
                format!("Categorical delay bins · articles · 0–{max}"),
                theme::dim(),
            ));
            for (label, n) in &s.intel.freshness.buckets {
                let lw = width.min(16);
                let mut line = Line::raw(format!("{:<lw$} ", components::clip_text(label, lw)));
                line.spans.extend(
                    charts::rank_bar(*n as u64, max as u64, width.saturating_sub(lw + 10)).spans,
                );
                line.spans.push(Span::raw(format!(" {n}")));
                lines.push(line);
            }
            lines.push(Line::styled(
                format!(
                    "Missing {} · future {} · unequal bins",
                    s.intel.freshness.missing, s.intel.freshness.future
                ),
                theme::dim(),
            ));
        }
        "recon.diversity" => {
            lines.push(panels::table_row(
                &[
                    "Category".into(),
                    "Eligible".into(),
                    "Corroborated".into(),
                    "Shortfall".into(),
                ],
                &[24, 14, 20, 42],
                width,
                false,
            ));
            for r in &s.recon.diversity {
                lines.push(panels::table_row(
                    &[
                        r.category.clone(),
                        if r.eligible {
                            r.eligible_scopes.to_string()
                        } else {
                            "N/A".into()
                        },
                        r.scopes_with_two_successful.to_string(),
                        r.top_shortfall_reason.clone(),
                    ],
                    &[24, 14, 20, 42],
                    width,
                    false,
                ));
                let mut meter = charts::meter(
                    if r.eligible && r.eligible_scopes > 0 {
                        Some(r.scopes_with_two_successful as f64 / r.eligible_scopes as f64)
                    } else {
                        None
                    },
                    width.saturating_sub(10),
                );
                meter.spans.push(Span::raw(format!(
                    " {}/{}",
                    r.scopes_with_two_successful, r.eligible_scopes
                )));
                lines.push(meter);
            }
        }
        "models.capacity" => lines.extend(capacity_lines(&s.models, width, usize::MAX)),
        "tools.evidence" => {
            lines.push(Line::styled(
                "Yield per successful nonempty invocation · 0–100%",
                theme::dim(),
            ));
            for r in &s.tools.evidence {
                let lw = width.min(22);
                let mut line =
                    Line::raw(format!("{:<lw$} ", components::clip_text(&r.tool_id, lw)));
                line.spans.extend(
                    charts::meter(
                        r.acceptance_pct.map(|p| p / 100.0),
                        width.saturating_sub(lw + 14),
                    )
                    .spans,
                );
                line.spans.push(Span::raw(format!(
                    " {}/{}",
                    r.with_evidence, r.successful_nonempty
                )));
                lines.push(line);
            }
            lines.push(Line::styled(
                "Identity credit may overlap; do not sum tool identities",
                theme::dim(),
            ));
        }
        _ => lines.extend(primary_table(app, w, s, width)),
    }
    if lines.is_empty() {
        lines.push(Line::raw("Empty window · no recorded observations"));
    }
    lines
}
fn legend(keys: &[&str]) -> Line<'static> {
    Line::from(
        keys.iter()
            .map(|key| Span::styled(format!("{key}  "), charts::series_style(key)))
            .collect::<Vec<_>>(),
    )
}
fn primary_table(
    app: &App,
    w: &WidgetSpec,
    s: &ProfileSnapshot,
    width: usize,
) -> Vec<Line<'static>> {
    let (keys, headers, proportions): (&[&str], &[&str], &[usize]) = match w.id {
        "intel.enrichment" => (
            &["tag", "articles", "body_pct", "claims_pct", "report_pct"],
            &["Tag", "Articles", "Body", "Claims", "Reports"],
            &[28, 18, 18, 18, 18],
        ),
        "recon.unresolved" => (
            &["run_id", "label", "reason", "evidence_count", "next_action"],
            &["Run", "Directive", "Reason", "Evidence", "Next action"],
            &[8, 30, 23, 14, 25],
        ),
        "atlas.backlog" => (
            &["stage", "unit", "queued", "blocked", "oldest_pending_ms"],
            &["Stage", "Unit", "Queued", "Blocked", "Oldest"],
            &[30, 24, 14, 14, 18],
        ),
        "models.performance" => (
            &[
                "provider",
                "model",
                "role",
                "sends",
                "attempt_error_pct",
                "final_operation_failure_pct",
                "p95_exec_ms",
            ],
            &[
                "Provider",
                "Model",
                "Role",
                "Sends",
                "Errors",
                "Final fail",
                "p95",
            ],
            &[15, 25, 15, 10, 10, 13, 12],
        ),
        "tools.reliability" => (
            &[
                "tool_id",
                "invocations",
                "wire_requests",
                "cache_hit_pct",
                "verified_zero_pct",
                "error_pct",
                "p95_ms",
            ],
            &["Tool", "Calls", "Wire", "Cache", "Zero", "Errors", "p95"],
            &[30, 12, 12, 12, 12, 12, 10],
        ),
        "tools.search_health" => (
            &[
                "engine",
                "fetches",
                "valid_serps",
                "verified_zero",
                "challenge",
                "parser_mismatch",
                "transport_failure",
            ],
            &[
                "Engine",
                "Fetches",
                "Valid",
                "Zero",
                "Challenge",
                "Parser",
                "Transport",
            ],
            &[24, 14, 12, 10, 14, 12, 14],
        ),
        _ => (w.detail_columns, w.detail_columns, &[]),
    };
    let weights = if proportions.is_empty() {
        vec![1; keys.len()]
    } else {
        proportions.to_vec()
    };
    let rows = sorted_rows(app, w, s);
    let mut lines = vec![panels::table_row(
        &headers.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        &weights,
        width,
        false,
    )];
    let selected = app.profile.selected_row.get(w.id).copied().unwrap_or(0);
    for (i, row) in rows.iter().enumerate() {
        let mut cells = keys
            .iter()
            .map(|key| format_cell(key, &row[*key]))
            .collect::<Vec<_>>();
        if w.id == "models.performance" {
            let is_model = row["is_model_row"].as_bool().unwrap_or(false);
            let collapsed = app
                .profile
                .collapsed_providers
                .get(row["provider"].as_str().unwrap_or(""))
                .copied()
                .unwrap_or(false);
            cells[0] = format!(
                "{} {}",
                if is_model {
                    "  "
                } else if collapsed {
                    "▸"
                } else {
                    "▾"
                },
                cells[0]
            );
        }
        lines.push(panels::table_row(&cells, &weights, width, i == selected));
    }
    if rows.is_empty() {
        lines.push(Line::raw("Empty window · no recorded rows"));
    }
    if w.id == "tools.search_health" {
        for row in rows {
            let usable = row["valid_serps"].as_u64().unwrap_or(0)
                + row["verified_zero"].as_u64().unwrap_or(0);
            let n = row["fetches"].as_u64().unwrap_or(0);
            let mut meter = charts::meter(
                if n > 0 {
                    Some(usable as f64 / n as f64)
                } else {
                    None
                },
                width.saturating_sub(16),
            );
            meter.spans.push(Span::raw(format!(" {usable}/{n} usable")));
            lines.push(meter);
        }
    }
    lines
}
fn format_cell(key: &str, value: &serde_json::Value) -> String {
    if key.ends_with("_ms") {
        return charts::duration_ms(value.as_i64());
    }
    if key.ends_with("_pct") || key == "share" {
        return charts::percent(value.as_f64());
    }
    if key == "ratio" {
        return charts::ratio(value.as_f64());
    }
    match value {
        serde_json::Value::Null => "N/A".into(),
        serde_json::Value::String(s) => s.clone(),
        _ => value.to_string(),
    }
}

fn widget_data(widget: &WidgetSpec, snapshot: &ProfileSnapshot) -> serde_json::Value {
    match widget.id {
        "summary.attention" => serde_json::to_value(&snapshot.attention).unwrap_or_default(),
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
        "models.latency" => &["p50_ms", "p95_ms"],
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
fn report_layout(app: &App, widget: &WidgetSpec, inner: Rect) -> ReportLayout {
    let mut layout = ReportLayout::new(inner, selected_lines(app, widget, inner.width).len());
    if widget.renderer == RendererKind::Table || app.profile.table_view || widget.id == ATTENTION.id
    {
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
    let mut lines = vec![Line::styled(
        format!("{value} · shared scale 0–{peak}"),
        theme::dim(),
    )];
    for row in rows.iter().take(height.saturating_sub(1) / 2) {
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
    let key = (
        widget.id,
        width,
        app.profile
            .selected_row
            .get(widget.id)
            .copied()
            .unwrap_or(0),
        app.profile.sort_column,
        app.profile.sort_descending,
    );
    if let Some(lines) = app.profile.detail_cache.borrow().get(&key) {
        return lines.clone();
    }
    let lines = build_report_detail_lines(app, widget, width);
    let mut cache = app.profile.detail_cache.borrow_mut();
    if cache.len() >= 8 {
        cache.clear();
    }
    cache.insert(key, lines.clone());
    lines
}
fn build_report_detail_lines(app: &App, widget: &WidgetSpec, width: u16) -> Vec<Line<'static>> {
    let Some(snapshot) = &app.profile.snapshot else {
        return vec![Line::raw("Loading statistics")];
    };
    let rows = sorted_rows(app, widget, snapshot);
    let keys = if widget.id == ATTENTION.id {
        vec!["app", "owner_id", "item_id", "reason", "age_ms", "action"]
    } else {
        widget.detail_columns.to_vec()
    };
    let proportions = vec![1; keys.len()];
    let mut lines = Vec::new();
    if !rows.is_empty() {
        lines.push(panels::table_row(
            &keys.iter().map(|k| k.to_string()).collect::<Vec<_>>(),
            &proportions,
            width as usize,
            false,
        ));
        let selected = app
            .profile
            .selected_row
            .get(widget.id)
            .copied()
            .unwrap_or(0);
        for (i, row) in rows.iter().enumerate() {
            lines.push(panels::table_row(
                &keys
                    .iter()
                    .map(|k| format_cell(k, &row[*k]))
                    .collect::<Vec<_>>(),
                &proportions,
                width as usize,
                i == selected,
            ));
        }
        if let Some(row) = rows.get(selected) {
            lines.push(Line::styled(
                "Selected row · full values / prose",
                theme::accent(),
            ));
            lines.extend(components::detail_records(row, width));
        }
    } else {
        lines.extend(components::detail_records(
            &widget_data(widget, snapshot),
            width,
        ));
    }
    let merged: Vec<(&str, serde_json::Value)> = match widget.id {
        "intel.enrichment" => vec![
            (
                "Distinct ingestion",
                serde_json::to_value(&snapshot.intel.volume).unwrap_or_default(),
            ),
            (
                "Paired confidence coverage",
                serde_json::json!({"initial":snapshot.intel.confidence.mean_initial,"current":snapshot.intel.confidence.mean_current,"paired_n":snapshot.intel.confidence.paired_n,"coverage":snapshot.intel.confidence.coverage_note}),
            ),
        ],
        "recon.outcomes" => vec![
            (
                "Workload",
                serde_json::to_value(&snapshot.recon.workload).unwrap_or_default(),
            ),
            (
                "Recall / retention",
                serde_json::to_value(&snapshot.recon.recall).unwrap_or_default(),
            ),
        ],
        "atlas.cycles" => vec![(
            "Cycle durations",
            serde_json::to_value(&snapshot.atlas.cycle_times).unwrap_or_default(),
        )],
        "atlas.backlog" => vec![(
            "Discovery dispositions",
            serde_json::to_value(&snapshot.atlas.discovery).unwrap_or_default(),
        )],
        "models.performance" => vec![
            (
                "Fallback recovery",
                serde_json::to_value(&snapshot.models.fallback).unwrap_or_default(),
            ),
            (
                "Role breakdown",
                serde_json::to_value(&snapshot.models.by_role).unwrap_or_default(),
            ),
            (
                "Failure causes",
                serde_json::to_value(&snapshot.models.failures).unwrap_or_default(),
            ),
            (
                "Amplification (sends / terminal operation, ×)",
                serde_json::to_value(&snapshot.models.amplification).unwrap_or_default(),
            ),
        ],
        "tools.evidence" => vec![(
            "Accepted / cited identities and provenance · retained raw coverage",
            serde_json::to_value(&snapshot.tools.evidence_details).unwrap_or_default(),
        )],
        "tools.reliability" => vec![
            (
                "Usage",
                serde_json::to_value(&snapshot.tools.top_tools).unwrap_or_default(),
            ),
            (
                "Trigger attribution",
                serde_json::to_value(&snapshot.tools.categories_by_trigger).unwrap_or_default(),
            ),
            (
                "Outcomes",
                serde_json::to_value(&snapshot.tools.outcomes).unwrap_or_default(),
            ),
        ],
        _ => Vec::new(),
    };
    for (label, data) in merged {
        lines.push(Line::styled(label.to_owned(), theme::accent()));
        lines.extend(components::detail_table(&data, &[], width));
    }
    let retained_plot = match widget.id {
        "atlas.cycles" => Some(widget!(
            "atlas.cycle_time",
            Section::Atlas,
            "Cycle duration · seconds",
            0
        )),
        "models.performance" => Some(widget!(
            "models.amplification",
            Section::Models,
            "Amplification · sends / terminal operation (×)",
            0
        )),
        _ => None,
    };
    if let Some(plot) = retained_plot {
        let points = series_points(&plot, snapshot);
        if !points.is_empty() {
            lines.push(Line::styled(plot.title, theme::accent()));
            lines.extend(charts::series_plot(
                &points,
                width as usize,
                11,
                points.len() - 1,
            ));
            lines.push(Line::styled(
                "Exact bucket values in the retained table above",
                theme::dim(),
            ));
        }
    }
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
    if let Some(error) = &app.profile.error {
        lines.push(Line::styled(format!("Stale · {error}"), theme::warn()));
    } else if app.profile.loading {
        lines.push(Line::styled(
            "Refreshing · showing previous snapshot",
            theme::dim(),
        ));
    }
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
        if widget.renderer != RendererKind::Time {
            let rows = sorted_rows(app, widget, snapshot);
            let index = app
                .profile
                .selected_row
                .get(widget.id)
                .copied()
                .unwrap_or(0);
            if let Some(row) = rows.get(index) {
                lines.push(Line::raw(format!(
                    "Selected row {} / {} · {}",
                    index + 1,
                    rows.len(),
                    row_identity(row)
                )));
            }
        }
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
                        let value = if key.starts_with("p95") && small_sample(point.n).is_some() {
                            None
                        } else {
                            *value
                        };
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
    app.layout.borrow_mut().push_scope(area);
    let block = theme::panel(&format!(" {} · Esc grid · ←/→ bucket ", widget.title));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if widget.id == "models.performance" && area.width >= 45 {
        let fold = Rect::new(area.right() - 21, area.y, 19, 1);
        frame.render_widget(
            Paragraph::new(" [g] fold provider ").style(theme::accent()),
            fold,
        );
        app.layout
            .borrow_mut()
            .register(Target::ProfileAction(5), fold);
    }
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
        let lines = if matches!(
            widget.id,
            "intel.reports"
                | "recon.stages"
                | "recon.directives"
                | "recon.diversity"
                | "atlas.temperature"
                | "tools.evidence"
        ) {
            dashboard_lines(
                app,
                widget,
                layout.plot.width as usize,
                layout.plot.height as usize,
            )
        } else if widget.renderer != RendererKind::Time {
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
            Paragraph::new(panels::clipped(lines, layout.plot.width as usize)),
            layout.plot,
        );
        let series = buckets.is_empty();
        let count = if series { points.len() } else { buckets.len() };
        if layout.plot.width >= 14 && layout.plot.height >= 6 {
            for (index, x, slot) in
                charts::plot_targets(count, layout.plot.width as usize, selected, series)
            {
                app.layout.borrow_mut().register(
                    Target::ProfileBucket(index),
                    Rect::new(
                        layout.plot.x + x as u16,
                        layout.plot.y,
                        slot as u16,
                        layout.plot.height,
                    ),
                );
            }
        }
    }
    let lines = report_detail_lines(app, widget, layout.details.width);
    let mut pane = app.profile.report_scroll.clone();
    pane.scroll(0, lines.len(), layout.details.height as usize);
    if let Some(snapshot) = &app.profile.snapshot {
        let rows = sorted_rows(app, widget, snapshot);
        for i in 0..rows.len() {
            let row = i + 1;
            if row >= pane.offset && row < pane.offset + layout.details.height as usize {
                app.layout.borrow_mut().register(
                    Target::ProfileRow(i),
                    Rect::new(
                        layout.details.x,
                        layout.details.y + (row - pane.offset) as u16,
                        layout.details.width,
                        1,
                    ),
                );
            }
        }
    }
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
    if app.profile.custom_period {
        frame.render_widget(
            Paragraph::new(vec![
                Line::raw("Enter from | to, both RFC3339 with explicit timezone"),
                Line::raw(app.profile.filter_edit.clone()),
                Line::styled(
                    app.profile.picker_error.clone().unwrap_or_default(),
                    theme::warn(),
                ),
            ]),
            inner,
        );
        return;
    }
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
            app.profile.report_origin = Some((index, app.profile.grid_scroll.offset, app.focus));
            if let Some(widget) = app.profile.focused_widget() {
                app.profile.report = Some(widget.id);
                app.profile.table_view = false;
                app.profile.report_scroll.offset = app
                    .profile
                    .report_offsets
                    .get(widget.id)
                    .copied()
                    .unwrap_or(0);
                app.set_focus(Target::ProfileReport);
            }
        }
        Target::ProfileAction(0) => {
            app.profile.period_popup = true;
            app.profile.custom_period = false;
            app.profile.picker_error = None;
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
        Target::ProfileAction(5) => {
            if let Some(snapshot) = app.profile.snapshot.as_ref() {
                let w = view_spec("models.performance").unwrap();
                let rows = sorted_rows(app, w, snapshot);
                let selected = app.profile.selected_row.get(w.id).copied().unwrap_or(0);
                if let Some(row) = rows.get(selected) {
                    let provider = row["provider"].as_str().unwrap_or("").to_string();
                    let collapsed = app
                        .profile
                        .collapsed_providers
                        .entry(provider.clone())
                        .or_default();
                    *collapsed = !*collapsed;
                    let rows = sorted_rows(app, w, snapshot);
                    let index = rows
                        .iter()
                        .position(|r| {
                            r["provider"].as_str().unwrap_or("") == provider
                                && !r["is_model_row"].as_bool().unwrap_or(false)
                        })
                        .unwrap_or(0);
                    remember_row(app, w.id, index);
                }
            }
        }
        Target::ProfilePanelRow(card, index) => {
            app.profile.focus = card;
            if let Some(w) = app.profile.focused_widget() {
                remember_row(app, w.id, index);
            }
            app.set_focus(Target::ProfileCard(card));
        }
        Target::ProfilePanelBucket(card, index) => {
            app.profile.focus = card;
            if let Some(w) = app.profile.focused_widget() {
                app.profile.selected_bucket.insert(w.id, index);
            }
            app.set_focus(Target::ProfileCard(card));
        }
        Target::ProfileBucket(index) => {
            if let Some(id) = app.profile.report {
                app.profile.selected_bucket.insert(id, index);
            }
        }
        Target::ProfileChoice(index) => {
            if app.profile.period_popup && index == 4 {
                app.profile.custom_period = true;
                app.profile.filter_edit.clear();
                return;
            }
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
            app.set_focus(if app.profile.report.is_some() {
                Target::ProfileReport
            } else {
                Target::ProfileAction(1)
            });
        }
        Target::ProfileRow(index) => {
            if let Some(id) = app.profile.report {
                remember_row(app, id, index);
                open_selected_owner(app);
            }
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
        let body = super::ui::body_rect(app);
        let area = strip(body, 1, body.height.saturating_sub(2));
        if let Some((_, rect)) = system_panes(app, area)
            .into_iter()
            .find(|(i, _)| *i == index)
        {
            let lines = system_lines(app, index, rect.width.saturating_sub(2));
            app.profile.system_scroll[index].scroll(
                delta,
                lines.len(),
                rect.height.saturating_sub(2) as usize,
            );
        }
        return;
    }
    if let Some(id) = app.profile.report {
        if let Some(widget) = view_spec(id) {
            let popup = expanded_area(super::ui::body_rect(app));
            let inner = Rect::new(
                popup.x + 1,
                popup.y + 1,
                popup.width.saturating_sub(2),
                popup.height.saturating_sub(2),
            );
            let report = report_layout(app, widget, inner);
            let lines = report_detail_lines(app, widget, report.details.width);
            app.profile
                .report_scroll
                .scroll(delta, lines.len(), report.details.height as usize);
        }
    } else {
        if let Some(w) = app.profile.focused_widget() {
            let width = layout
                .panels
                .iter()
                .find(|p| p.index == app.profile.focus)
                .map(|p| p.rect.width.saturating_sub(4))
                .unwrap_or(layout.content.width.saturating_sub(4));
            let extent = dashboard_lines(
                app,
                w,
                width as usize,
                layout.panel_height(app.profile.focus).saturating_sub(2),
            )
            .len();
            let viewport = layout.panel_height(app.profile.focus).saturating_sub(2);
            let pane = app.profile.panel_scroll.entry(w.id).or_default();
            let previous = pane.offset;
            pane.scroll(delta, extent, viewport);
            if pane.offset != previous {
                return;
            }
        }
        app.profile.grid_scroll.offset = grid_offset(app, &layout);
        app.profile
            .grid_scroll
            .scroll(delta, layout.extent(), layout.content.height as usize);
        let offset = app.profile.grid_scroll.offset;
        let column = app.profile.focus % layout.columns;
        app.profile.focus = (0..app.profile.focused_widgets().len())
            .find(|i| layout.focus_start(*i) + layout.panel_height(*i) > offset)
            .unwrap_or(0);
        app.profile.focus =
            (app.profile.focus + column).min(app.profile.focused_widgets().len().saturating_sub(1));
        app.focus = Target::ProfileCard(app.profile.focus);
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
        if app.profile.custom_period {
            match key.code {
                KeyCode::Esc => {
                    app.profile.custom_period = false;
                    app.profile.period_popup = false;
                }
                KeyCode::Char(ch) => {
                    app.profile.filter_edit.push(ch);
                }
                KeyCode::Backspace => {
                    app.profile.filter_edit.pop();
                }
                KeyCode::Enter => {
                    let input = app.profile.filter_edit.clone();
                    if let Some((from, to)) = input.split_once('|') {
                        match (
                            chrono::DateTime::parse_from_rfc3339(from.trim()),
                            chrono::DateTime::parse_from_rfc3339(to.trim()),
                        ) {
                            (Ok(a), Ok(b)) if b > a => {
                                app.profile.filters.period = profile_stats::Period::Custom {
                                    from: a.to_rfc3339(),
                                    to: b.to_rfc3339(),
                                };
                                app.profile.invalidate();
                                app.profile.custom_period = false;
                                app.profile.period_popup = false;
                            }
                            _ => {
                                app.profile.picker_error =
                                    Some("Valid timestamps and end after start required".into())
                            }
                        }
                    } else {
                        app.profile.picker_error = Some("Separate start and end with |".into());
                    }
                }
                _ => {}
            }
            return true;
        }
        match key.code {
            KeyCode::Esc => {
                app.profile.filter_popup = None;
                app.profile.period_popup = false;
                app.set_focus(if app.profile.report.is_some() {
                    Target::ProfileReport
                } else {
                    Target::ProfileAction(1)
                });
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
            app.set_focus(Target::ProfileCard(0));
            true
        }
        KeyCode::Char(']') => {
            app.profile.next_section();
            app.set_focus(Target::ProfileCard(0));
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
            if let Some(id) = app.profile.report {
                app.profile
                    .report_offsets
                    .insert(id, app.profile.report_scroll.offset);
            }
            app.profile.report = None;
            if let Some((index, offset, focus)) = app.profile.report_origin.take() {
                app.profile.focus = index;
                app.set_focus(focus);
                app.profile.grid_scroll.offset = offset;
            } else {
                app.set_focus(Target::ProfileCard(app.profile.focus));
            }
            true
        }
        KeyCode::Char('s') if app.profile.report.is_some() => {
            let columns = app
                .profile
                .report
                .and_then(view_spec)
                .map(|w| w.detail_columns.len())
                .unwrap_or(1)
                .max(1);
            app.profile.sort_column = (app.profile.sort_column + 1) % columns;
            app.profile.sort_descending = false;
            if let Some(snapshot) = app.profile.snapshot.clone() {
                app.profile.accept_snapshot(snapshot);
            }
            true
        }
        KeyCode::Char('g')
            if app.profile.report == Some("models.performance")
                || app
                    .profile
                    .focused_widget()
                    .is_some_and(|w| w.id == "models.performance") =>
        {
            activate(app, Target::ProfileAction(5));
            true
        }
        KeyCode::Enter if app.profile.report.is_some() => {
            if app.focus == Target::ProfileAction(5) {
                activate(app, Target::ProfileAction(5));
            } else {
                open_selected_owner(app);
            }
            true
        }
        KeyCode::Char('v') if app.profile.report.is_some() => {
            app.profile.table_view = !app.profile.table_view;
            true
        }
        KeyCode::Left | KeyCode::Right
            if app.profile.report.is_some()
                || app.profile.focused_widget().is_some_and(w_is_time) =>
        {
            let id = app
                .profile
                .report
                .or_else(|| app.profile.focused_widget().map(|w| w.id))
                .unwrap();
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
            select_row(app, 1);
            if app.profile.report.is_some() {
                scroll(app, 1);
            }
            true
        }
        KeyCode::Up | KeyCode::Char('k') => {
            select_row(app, -1);
            if app.profile.report.is_some() {
                scroll(app, -1);
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
        KeyCode::Enter if app.profile.all_apps && app.focus == Target::ProfileCard(2) => {
            app.profile.report = Some(ATTENTION.id);
            open_selected_owner(app);
            if app.profile.report == Some(ATTENTION.id) {
                app.profile.report = None;
            }
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
                distribution
                    .mean_initial
                    .map(|v| format!("{v:.3} score"))
                    .unwrap_or_else(|| "N/A".into()),
                distribution
                    .mean_current
                    .map(|v| format!("{v:.3} score"))
                    .unwrap_or_else(|| "N/A".into())
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
                format!(
                    "{:>9}",
                    row.previous_temperature
                        .map(|v| format!("{v:.1}"))
                        .unwrap_or_else(|| "N/A".into())
                ),
                format!(
                    "{:>8}",
                    row.current_temperature
                        .map(|v| format!("{v:.1}"))
                        .unwrap_or_else(|| "N/A".into())
                ),
                format!(
                    "{:>8}",
                    row.delta
                        .filter(|_| row.comparable)
                        .map(|v| format!("{v:+.1} pt"))
                        .unwrap_or_else(|| "N/A".into())
                ),
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
        return empty_note("Live capacity unavailable · no companion snapshot");
    }
    let mut lines = Vec::new();
    for r in models.capacity.iter().take(page) {
        lines.push(Line::styled(
            format!(
                "{} / {} / {} · {}",
                r.provider, r.quota_group, r.scope, r.quota_source
            ),
            theme::dim(),
        ));
        let limit = r.effective_rpm.filter(|n| *n > 0);
        let ratio = limit.map(|n| r.sends_60s as f64 / n as f64);
        let mut meter = charts::meter(ratio, width.saturating_sub(25));
        meter.spans.push(Span::styled(
            format!(
                " {}/{} sends/60s{}",
                r.sends_60s,
                limit.map(|n| n.to_string()).unwrap_or_else(|| "N/A".into()),
                if ratio.is_some_and(|v| v > 1.0) {
                    " !"
                } else {
                    ""
                }
            ),
            if ratio.is_some_and(|v| v > 1.0) {
                theme::warn()
            } else {
                theme::text()
            },
        ));
        lines.push(meter);
        let concurrency = r.max_concurrency.filter(|n| *n > 0);
        let ratio = concurrency.map(|n| r.active as f64 / n as f64);
        let mut meter = charts::meter(ratio, width.saturating_sub(25));
        meter.spans.push(Span::styled(
            format!(
                " {}/{} active{}",
                r.active,
                concurrency
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "N/A".into()),
                if ratio.is_some_and(|v| v > 1.0) {
                    " !"
                } else {
                    ""
                }
            ),
            if ratio.is_some_and(|v| v > 1.0) {
                theme::warn()
            } else {
                theme::text()
            },
        ));
        lines.push(meter);
        lines.push(Line::raw(format!(
            "pace {} req/min · queued {} · oldest {} · cooldown {}",
            r.pace_per_min
                .filter(|v| v.is_finite())
                .map(|v| format!("{v:.1}"))
                .unwrap_or_else(|| "N/A".into()),
            r.queued,
            charts::duration_ms(r.oldest_wait_ms.map(|v| v as i64)),
            charts::duration_ms(r.cooldown_ms.map(|v| v as i64))
        )));
    }
    if lines.is_empty() {
        lines.push(Line::raw("No live capacity rows"));
    }
    lines
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
                format!("{:>10}", charts::ratio(row.ratio)),
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
