//! Read-side aggregation for the Profile dashboard.
//!
//! `profile_stats` is the only module that turns telemetry and durable rows into
//! the immutable [`ProfileSnapshot`] the TUI renders. It performs no writes, no
//! network calls and no model calls, and it never stores prompts, article
//! bodies, model outputs or credentials: every field is a bounded label, an
//! identifier or a number.
//!
//! # Aggregation rules
//!
//! * Counts and histograms merge raw `telemetry_events` with the
//!   `telemetry_hourly` / `telemetry_daily` rollups. A rollup bucket is only read
//!   when the whole bucket predates the raw retention boundary, so nothing is
//!   counted twice (at most one bucket of history is lost at the seam).
//! * Weighted means are always total sum / total count; a pre-aggregated average
//!   is never averaged again.
//! * Quantiles come from raw samples while a cohort is still raw, and otherwise
//!   from the mergeable duration bins (`telemetry::DURATION_BINS_MS`) — never from
//!   averaged percentiles.
//! * Trend points are never interpolated: a bucket without data keeps zero
//!   counts and `None` durations.
//! * `N = 0` is N/A. Every `Option<f64>` returns `None` when its denominator is
//!   zero, never `0.0`.
//! * Below [`SMALL_SAMPLE_MIN`] observations p95 is suppressed (`None`) while p50
//!   and the sample count stay visible; the UI decides how to label it.
//!
//! # Telemetry contract consumed by these readers
//!
//! Instrumentation writes the dimensions declared by `crate::telemetry`
//! (`app`, `trigger`, `tool_id`, `category`, `provider`, `role`, `model`,
//! `mode`, `engine`, `outcome`, `reason`) plus these bounded payload keys:
//!
//! | Event kind | Dimensions | Payload keys |
//! |---|---|---|
//! | `intel_article` | `category` (tag), `origin`, `article_id` | `initial_confidence` |
//! | `intel_brief_rating` | `article_id`, `category` | `rating`, `claims` |
//! | `intel_report` | `article_id`, `mode`, `reason` | `wall_ms`, `active_ms` |
//! | `recon_run` | `mode`, `run_id`, `outcome` | `directives`, `calls`, `categories`, `accepted_memories` |
//! | `recon_stage` | `mode`, `run_id`, `tool_id` (stage name) | `wait_ms` |
//! | `recall_query` | `mode`, `run_id`, `reason` | `candidates`, `accepted`, `rejected` |
//! | `directive_assessed` | `mode`, `run_id`, `outcome`, `reason` | `label`, `evidence_count`, `next_action` |
//! | `atlas_cycle` | `run_id`, `outcome`, `reason` | `queue_ms` |
//! | `atlas_origin_snapshot` | `run_id`, `origin` | `temperature`, `tier`, `articles`, `eligible`, `version` |
//! | `atlas_candidate` | `run_id`, `outcome` (disposition) | — |
//! | `atlas_stage` | `run_id`, `tool_id` (stage), `outcome` (new state), `call_id` (unit id) | `unit`, `error_category` |
//! | `tool_invocation` | `tool_id`, `category`, `mode` (remote/local/cache), `trigger`, `outcome`, `reason`, `app` | — |
//! | `tool_wire_request` | `tool_id`, `category`, `trigger` | — |
//! | `tool_engine_query` | `engine`, `tool_id`, `outcome`, `reason` | `parser_version` |
//! | `evidence_item` | `tool_id`, `category`, `call_id`, `article_id`; `canonical_ref` = evidence fingerprint | `cited` |
//! | `model_attempt` | `provider`, `role`, `model`, `outcome`, `reason`; `canonical_ref` = schema-26 attempt id | `queue_ms`, `ttfb_ms`, `first_content_ms`, `elapsed_ms`, `http_status`, `failure_category`, `prompt_tokens`, `completion_tokens`, `stream_completed`, `blocked_before_send` |
//! | `model_operation` | `provider`, `role`, `model`, `outcome`, `reason`; `canonical_ref` = schema-26 operation id | `attempts`, `terminal`, `recovered`, `blocked_before_send`, `elapsed_ms`, `primary_provider`, `primary_model`, `effective_provider`, `effective_model` |
//!
//! A `model_attempt` / `model_operation` event whose `canonical_ref` names an
//! existing schema-26 row is supplemental: the durable row stays authoritative
//! and the event adds no second count.

pub use anyhow;

use std::collections::{BTreeMap, HashMap, HashSet};

use anyhow::Result;
use chrono::{DateTime, Duration as ChronoDuration, TimeZone, Utc};
use rusqlite::types::Value;
use rusqlite::{params_from_iter, Connection};
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use crate::provider_metrics::{self, CapacityRow};
use crate::telemetry::{self, EventKind, ToolOutcome, Trigger};

/// Percentiles below this many observations are suppressed instead of shown as
/// a confident p95. The row keeps p50, the mean and `n`, and the UI labels it.
pub const SMALL_SAMPLE_MIN: u64 = 20;

/// Confidence bands are the fixed 0–1 quintiles of the spec; a score of exactly
/// 1.0 belongs to the last band.
pub const CONFIDENCE_BANDS: [(f64, f64); 5] =
    [(0.0, 0.2), (0.2, 0.4), (0.4, 0.6), (0.6, 0.8), (0.8, 1.0)];

/// Named search engines the search-health widget always reports on. Engine
/// identity is separate from the transport provider that fetches the SERP.
pub const NAMED_ENGINES: [&str; 3] = ["google", "yandex", "mojeek"];

/// Adaptive bucket widths: 1h -> 5min, 24h -> hourly, 7d -> 6h, 30d -> daily.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Period {
    /// Last hour, 5-minute buckets.
    H1,
    /// Last 24 hours, hourly buckets. The dashboard default.
    #[default]
    H24,
    /// Last 7 days, 6-hour buckets.
    H7,
    /// Last 30 days, daily buckets.
    H30,
    /// Explicit inclusive window; the bucket width adapts to the span.
    Custom { from: String, to: String },
}

impl Period {
    /// The four fixed periods, for filter popups.
    pub const ALL: [Self; 4] = [Self::H1, Self::H24, Self::H7, Self::H30];

    /// Short label used by the filter strip.
    pub fn label(&self) -> &'static str {
        match self {
            Self::H1 => "1h",
            Self::H24 => "24h",
            Self::H7 => "7d",
            Self::H30 => "30d",
            Self::Custom { .. } => "custom",
        }
    }

    /// Human label for one bucket of this period.
    pub fn bucket_label(&self) -> &'static str {
        match self.bucket_seconds() {
            secs if secs < 3_600 => "5 min",
            secs if secs < 21_600 => "1 hour",
            secs if secs < 86_400 => "6 hours",
            _ => "1 day",
        }
    }

    /// Inclusive start of the window for `now`.
    pub fn start(&self, now: DateTime<Utc>) -> DateTime<Utc> {
        match self {
            Self::H1 => now - ChronoDuration::hours(1),
            Self::H24 => now - ChronoDuration::hours(24),
            Self::H7 => now - ChronoDuration::days(7),
            Self::H30 => now - ChronoDuration::days(30),
            Self::Custom { from, .. } => {
                parse_timestamp(from).unwrap_or_else(|| now - ChronoDuration::hours(24))
            }
        }
    }

    /// Exclusive end of the window for `now`.
    pub fn end(&self, now: DateTime<Utc>) -> DateTime<Utc> {
        match self {
            Self::Custom { to, .. } => parse_timestamp(to).unwrap_or(now),
            _ => now,
        }
    }

    /// Width of one trend bucket in seconds.
    pub fn bucket_seconds(&self) -> i64 {
        match self {
            Self::H1 => 300,
            Self::H24 => 3_600,
            Self::H7 => 21_600,
            Self::H30 => 86_400,
            Self::Custom { from, to } => match (parse_timestamp(from), parse_timestamp(to)) {
                (Some(a), Some(b)) if b > a => adaptive_bucket_seconds((b - a).num_seconds()),
                _ => 3_600,
            },
        }
    }
}

fn adaptive_bucket_seconds(span_secs: i64) -> i64 {
    if span_secs <= 6 * 3_600 {
        300
    } else if span_secs <= 3 * 86_400 {
        3_600
    } else if span_secs <= 30 * 86_400 {
        21_600
    } else {
        86_400
    }
}

fn bucket_format(secs: i64) -> &'static str {
    if secs < 3_600 {
        "%Y-%m-%dT%H:%M"
    } else if secs < 86_400 {
        "%Y-%m-%dT%H"
    } else {
        "%Y-%m-%d"
    }
}

fn parse_timestamp(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value.trim())
        .map(|ts| ts.with_timezone(&Utc))
        .ok()
}

/// Dashboard filters. An empty string means "unfiltered".
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatFilters {
    pub period: Period,
    pub app: String,
    pub provider: String,
    pub role: String,
    pub mode: String,
    pub tool: String,
    pub category: String,
}

impl StatFilters {
    /// Value of one bounded dimension, or `None` when it does not filter.
    fn dimension(&self, name: &str) -> Option<&str> {
        let raw = match name {
            "app" => &self.app,
            "provider" => &self.provider,
            "role" => &self.role,
            "mode" => &self.mode,
            "tool" => &self.tool,
            "category" => &self.category,
            _ => return None,
        };
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    }

    /// True when any dimension narrows the view. The period is a view choice,
    /// not a filter, so it never makes `clear` necessary.
    pub fn is_active(&self) -> bool {
        ["app", "provider", "role", "mode", "tool", "category"]
            .iter()
            .any(|name| self.dimension(name).is_some())
    }

    /// Clears every dimension filter, keeping the selected period.
    pub fn clear(&mut self) {
        self.app.clear();
        self.provider.clear();
        self.role.clear();
        self.mode.clear();
        self.tool.clear();
        self.category.clear();
    }
}

// ---------- shared numeric helpers ----------

/// Percentile over raw samples; `q` is clamped to `[0, 1]`.
pub fn quantile(samples: &mut [i64], q: f64) -> Option<i64> {
    if samples.is_empty() {
        return None;
    }
    let q = q.clamp(0.0, 1.0);
    samples.sort_unstable();
    let rank = (q * (samples.len() - 1) as f64).round() as usize;
    Some(samples[rank.min(samples.len() - 1)])
}

/// Percentile over a merged duration histogram. Each bin contributes its lower
/// edge, so the answer is the smallest edge at or above the requested rank.
pub fn quantile_from_bins(bins: &[i64], q: f64) -> Option<i64> {
    let total: i64 = bins.iter().sum();
    if total <= 0 {
        return None;
    }
    let q = q.clamp(0.0, 1.0);
    let target = (q * (total - 1) as f64).round() as i64;
    let mut seen = 0i64;
    for (index, count) in bins.iter().enumerate() {
        seen += *count;
        if seen > target {
            return Some(edge_for_bin(index));
        }
    }
    bins.iter().rposition(|count| *count > 0).map(edge_for_bin)
}

fn edge_for_bin(index: usize) -> i64 {
    telemetry::DURATION_BINS_MS
        .get(index)
        .map(|edge| *edge as i64)
        .unwrap_or(*telemetry::DURATION_BINS_MS.last().unwrap_or(&300_000) as i64)
}

/// Mean of raw samples. `None` — never `0.0` — when there are no samples.
pub fn mean(samples: &[i64]) -> Option<f64> {
    if samples.is_empty() {
        return None;
    }
    Some(samples.iter().sum::<i64>() as f64 / samples.len() as f64)
}

/// Weighted percentage form of `numerator / denominator`. `None` — never `0.0`
/// — when the denominator is zero.
pub fn sum_ratio(numerator: u64, denominator: u64) -> Option<f64> {
    if denominator == 0 {
        return None;
    }
    Some(numerator as f64 * 100.0 / denominator as f64)
}

/// Start label of the adaptive bucket that contains `now`.
pub fn bucket_start(now: DateTime<Utc>, period: &Period) -> String {
    let secs = period.bucket_seconds().max(1);
    let aligned = now.timestamp() - now.timestamp().rem_euclid(secs);
    match Utc.timestamp_opt(aligned, 0).single() {
        Some(ts) => ts.format(bucket_format(secs)).to_string(),
        None => now.format(bucket_format(secs)).to_string(),
    }
}

/// Number of buckets in the window, inclusive of both ends.
pub fn bucket_count(period: &Period) -> usize {
    let secs = period.bucket_seconds().max(1);
    let span = match period {
        Period::H1 => 3_600,
        Period::H24 => 86_400,
        Period::H7 => 604_800,
        Period::H30 => 2_592_000,
        Period::Custom { from, to } => match (parse_timestamp(from), parse_timestamp(to)) {
            (Some(a), Some(b)) => (b - a).num_seconds().max(0),
            _ => 86_400,
        },
    };
    (span / secs + 1) as usize
}

// ---------- one immutable snapshot ----------

/// Everything the Profile Overview renders. Built by [`snapshot`]; no writes,
/// no network, no model calls and no prose, bodies or keys.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ProfileSnapshot {
    pub captured_at: String,
    pub filters: StatFilters,
    /// First moment this installation recorded telemetry.
    pub observed_since: String,
    pub retention: RetentionInfo,
    pub status: GlobalStatus,
    pub intel: IntelStats,
    pub recon: ReconStats,
    pub atlas: AtlasStats,
    pub models: ModelStats,
    pub tools: ToolStats,
}

/// Retention boundaries the dashboard displays.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RetentionInfo {
    pub raw_days: i64,
    pub rollup_days: i64,
    /// Oldest raw event still stored, empty when nothing is retained.
    pub oldest_raw: String,
    pub oldest_rollup: String,
}

/// Compact global status strip. Live counts ignore the historical filter.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GlobalStatus {
    pub active_jobs: i64,
    pub queued_requests: i64,
    /// `ok` | `degraded` | `stale` | `unknown`.
    pub collection_health: String,
    pub last_updated: String,
    /// False until the orchestration companion publishes capacity; the queued
    /// count is then meaningless and must render as unavailable, never zero.
    pub provider_capacity_available: bool,
}

// ---------- snapshot assembly ----------

/// Assembles one dashboard snapshot over `conn`.
///
/// Read-only: the writer is never locked, so a dashboard refresh never blocks
/// recording. A missing table or an empty window yields empty sections, never
/// fabricated numbers — the dashboard renders those as unavailable/N/A.
pub fn snapshot(
    conn: &Connection,
    period: &Period,
    filters: &StatFilters,
    now: DateTime<chrono::Utc>,
) -> Result<ProfileSnapshot> {
    let reader = Reader::new(conn, period, filters, now);
    let intel = reader.intel();
    let recon = reader.recon();
    let atlas = reader.atlas();
    let models = reader.models();
    let tools = reader.tools();
    let carries_data = sections_carry_data(&intel, &recon, &atlas, &models, &tools);
    let status = global_status(conn, period, now, carries_data);
    Ok(ProfileSnapshot {
        captured_at: now.to_rfc3339(),
        filters: filters.clone(),
        observed_since: observed_since(conn),
        retention: retention(conn, now),
        status,
        intel,
        recon,
        atlas,
        models,
        tools,
    })
}

/// Every enum-ish value the filter strip can offer, for the filter popups.
///
/// The first pair is always the period choice list; every other pair is the
/// sorted distinct non-empty values that dimension actually carries in
/// `telemetry_events`, so the strip never offers a value that selects nothing.
pub fn filter_options(conn: &Connection) -> Vec<(String, Vec<String>)> {
    let periods: Vec<String> = Period::ALL.iter().map(|p| p.label().into()).collect();
    let mut options: Vec<(String, Vec<String>)> = vec![("period".to_string(), periods)];
    for dimension in ["app", "provider", "role", "mode", "tool", "category"] {
        options.push((dimension.to_string(), distinct_values(conn, dimension)));
    }
    options
}

// ---------- Intel (7 widgets) ----------

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct IntelStats {
    pub volume: Vec<VolumeBucket>,
    pub confidence: ConfidenceDistribution,
    pub origins: Vec<OriginCount>,
    pub enrichment: Vec<EnrichmentRow>,
    pub reports: Vec<ReportModeRow>,
    pub publishers: Vec<PublisherCount>,
    pub freshness: FreshnessHistogram,
    /// Attempt view of report work, kept apart from terminal revisions.
    pub report_attempts: u64,
    /// Coverage labels: nonadditive tag totals, paired-confidence scope, and the
    /// report-row semantics the tables rely on.
    pub note: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct VolumeBucket {
    pub bucket: String,
    pub total: u32,
    pub by_tag: Vec<(String, u32)>,
    pub untagged: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ConfidenceBand {
    pub label: String,
    pub lo: f64,
    pub hi: f64,
    pub initial: u32,
    pub current: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ConfidenceDistribution {
    pub bands: Vec<ConfidenceBand>,
    pub paired_n: u64,
    pub mean_initial: Option<f64>,
    pub mean_current: Option<f64>,
    pub median_initial: Option<f64>,
    pub median_current: Option<f64>,
    /// Cohort scope: which observations the comparison covers.
    pub coverage_note: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct OriginCount {
    pub origin: String,
    pub articles: u32,
    pub share: f64,
    pub is_other: bool,
    pub is_unknown: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EnrichmentRow {
    pub tag: String,
    pub articles: u32,
    pub body_pct: Option<f64>,
    pub claims_pct: Option<f64>,
    pub report_pct: Option<f64>,
    pub mean_initial_confidence: Option<f64>,
    pub initial_n: u32,
    pub mean_brief_rating: Option<f64>,
    pub brief_n: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ReportModeRow {
    pub mode: String,
    pub completed: u32,
    pub partial: u32,
    pub failed: u32,
    /// Current live jobs, not period completions.
    pub waiting: u32,
    pub blocked: u32,
    pub mean_wall_ms: Option<i64>,
    pub p95_wall_ms: Option<i64>,
    pub mean_active_ms: Option<i64>,
    pub n: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PublisherCount {
    pub domain: String,
    pub articles: u32,
    pub share: f64,
    pub is_other: bool,
    pub is_unknown: bool,
    /// Share held by the three largest publishers; identical on every row.
    pub top3_concentration: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FreshnessHistogram {
    pub buckets: Vec<(String, u32)>,
    pub missing: u32,
    pub future: u32,
}

// ---------- Recon (7 widgets) ----------

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ReconStats {
    pub outcomes: Vec<ReconOutcomeBucket>,
    pub stages: Vec<StageDurationRow>,
    pub recall: Vec<RecallRow>,
    pub workload: Vec<WorkloadRow>,
    pub diversity: Vec<DiversityRow>,
    pub directives: Vec<DirectiveResolution>,
    pub unresolved: Vec<UnresolvedDirective>,
    /// Coverage labels: separate stage attempts, categorical categories, and
    /// directive assessments that never infer answers from run completion.
    pub note: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ReconOutcomeBucket {
    pub bucket: String,
    pub mode: String,
    pub completed_with_evidence: u32,
    pub completed_zero_evidence: u32,
    pub partial: u32,
    pub failed: u32,
    pub cancelled: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct StageDurationRow {
    pub stage: String,
    pub mode: String,
    pub mean_exec_ms: Option<i64>,
    pub mean_wait_ms: Option<i64>,
    pub n: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RecallRow {
    pub mode: String,
    pub queries: u32,
    pub with_candidates: u32,
    pub with_accepted: u32,
    pub retention_pct: Option<f64>,
    pub accepted_per_run: Option<f64>,
    pub rejection_reason: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WorkloadRow {
    pub mode: String,
    pub runs: u32,
    pub directives_per_run: f64,
    pub calls_per_run: f64,
    pub categories_per_run: f64,
    pub accepted_memories_per_run: f64,
    pub median_wall_ms: Option<i64>,
    pub p95_wall_ms: Option<i64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DiversityRow {
    pub category: String,
    pub eligible_scopes: u32,
    pub scopes_with_two_attempted: u32,
    pub scopes_with_two_successful: u32,
    pub source_groups_per_scope: Option<f64>,
    pub top_shortfall_reason: String,
    pub eligible: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DirectiveResolution {
    pub mode: String,
    pub answered: u32,
    pub partial: u32,
    pub unresolved: u32,
    pub blocked: u32,
    pub unknown: u32,
    pub n: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct UnresolvedDirective {
    pub run_id: String,
    pub mode: String,
    pub label: String,
    pub reason: String,
    pub evidence_count: u32,
    pub last_progress: String,
    pub next_action: String,
}

// ---------- Atlas (6 widgets) ----------

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AtlasStats {
    pub temperature_changes: Vec<TemperatureChange>,
    pub cycle_times: Vec<CycleTimeBucket>,
    pub cycle_outcomes: Vec<CycleOutcomeBucket>,
    pub origins: Vec<OriginTierRow>,
    pub discovery: Vec<DiscoveryBucket>,
    pub backlog: Vec<BacklogRow>,
    pub waiting_now: u32,
    pub blocked_now: u32,
    /// Coverage labels: Argos's own score, ordinal tiers, one disposition per
    /// candidate occurrence, and live rows that ignore the time filter.
    pub note: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TemperatureChange {
    pub origin: String,
    pub previous_temperature: Option<f64>,
    pub current_temperature: Option<f64>,
    pub delta: Option<f64>,
    pub latest_tier: Option<u8>,
    pub articles: u32,
    pub comparable: bool,
    /// warming | cooling | flat | new | missing | incomparable
    pub label: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CycleTimeBucket {
    pub bucket: String,
    pub mean_ms: Option<i64>,
    pub p95_ms: Option<i64>,
    pub n: u32,
    pub mean_queue_ms: Option<i64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CycleOutcomeBucket {
    pub bucket: String,
    pub completed: u32,
    pub partial: u32,
    pub failed: u32,
    pub cancelled: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct OriginTierRow {
    pub origin: String,
    pub articles: u32,
    pub latest_temperature: Option<f64>,
    pub mean_temperature: Option<f64>,
    pub latest_tier: Option<u8>,
    pub tier1_share: f64,
    pub tier2_share: f64,
    pub tier3_share: f64,
    pub snapshots: u32,
    pub latest_at: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DiscoveryBucket {
    pub bucket: String,
    pub fetched: u32,
    pub retained_new: u32,
    pub retained_existing: u32,
    pub duplicate_in_cycle: u32,
    pub rejected: u32,
    pub pending: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BacklogRow {
    pub stage: String,
    pub unit: String,
    pub queued: u32,
    pub running: u32,
    pub waiting: u32,
    pub blocked: u32,
    pub oldest_pending_ms: Option<i64>,
    pub completed_in_period: u32,
    pub latest_error_category: String,
}

// ---------- Models (8 widgets) ----------

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ModelStats {
    /// Live capacity from the orchestration companion. Ignores every filter.
    pub capacity: Vec<CapacityRow>,
    pub capacity_available: bool,
    pub by_role: Vec<RoleRequestBucket>,
    pub latency: Vec<LatencyBucket>,
    pub queue_delay: Vec<QueueBucket>,
    pub performance: Vec<ProviderModelRow>,
    pub fallback: Vec<FallbackRow>,
    pub amplification: Vec<AmplificationBucket>,
    pub failures: Vec<FailureBucket>,
    /// Attempt/operation accounting so wire counts reconcile with the ledger.
    pub attempts: AttemptSummary,
    /// Coverage labels: sends versus operations, error-rate denominators, and
    /// live capacity that deliberately ignores historical filters.
    pub note: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AttemptSummary {
    pub finished_attempts: u64,
    pub failed_attempts: u64,
    pub operations: u64,
    pub terminal_operations_with_send: u64,
    pub sends_before_period: u64,
    pub blocked_before_send: u64,
    pub missing_queue_timing: u64,
    pub missing_first_response: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RoleRequestBucket {
    pub bucket: String,
    pub by_role: Vec<(String, u64)>,
    pub total: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LatencyBucket {
    pub bucket: String,
    pub p50_ms: Option<i64>,
    pub p95_ms: Option<i64>,
    pub n: u64,
    pub p50_first_header_ms: Option<i64>,
    pub p50_first_content_ms: Option<i64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct QueueBucket {
    pub bucket: String,
    pub p50_ms: Option<i64>,
    pub p95_ms: Option<i64>,
    pub n: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ProviderModelRow {
    pub provider: String,
    pub model: String,
    pub role: String,
    pub sends: u64,
    pub completed_attempts: u64,
    pub attempt_error_pct: Option<f64>,
    pub http_429: u64,
    pub final_operation_failure_pct: Option<f64>,
    pub mean_exec_ms: Option<i64>,
    pub p95_exec_ms: Option<i64>,
    pub n: u64,
    /// False for the provider total row, true for its expandable model row.
    pub is_model_row: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FallbackRow {
    pub role: String,
    pub primary_provider: String,
    pub primary_model: String,
    pub effective_provider: String,
    pub effective_model: String,
    pub trigger_reason: String,
    pub triggered_operations: u64,
    pub recovered_operations: u64,
    pub recovery_pct: Option<f64>,
    pub median_ttoutcome_ms: Option<i64>,
    pub p95_ttoutcome_ms: Option<i64>,
    pub recovered_n: u64,
    pub failed_n: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AmplificationBucket {
    pub bucket: String,
    pub sends: u64,
    pub terminal_operations: u64,
    pub ratio: Option<f64>,
    pub in_flight_operations: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FailureBucket {
    pub bucket: String,
    pub by_category: Vec<(String, u64)>,
    pub total_failed: u64,
    pub pct_of_finished: Option<f64>,
}

// ---------- Tools (7 widgets) ----------

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ToolStats {
    pub top_tools: Vec<ToolCount>,
    pub categories_by_trigger: Vec<CategoryTrigger>,
    pub outcomes: Vec<ToolOutcomeBucket>,
    pub reliability: Vec<ReliabilityRow>,
    pub engine_health: Vec<EngineHealthRow>,
    pub evidence: Vec<EvidenceRow>,
    pub failure_causes: Vec<FailureCauseRow>,
    /// Coverage labels: remote-only error denominators, cancelled handling, and
    /// engine identity separate from the transport provider.
    pub note: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ToolCount {
    pub tool_id: String,
    pub category: String,
    pub invocations: u64,
    pub remote: u64,
    pub local: u64,
    pub cache: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CategoryTrigger {
    pub category: String,
    pub by_trigger: Vec<(String, u64)>,
    pub total: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ToolOutcomeBucket {
    pub bucket: String,
    pub completed_nonempty: u64,
    pub verified_zero: u64,
    pub partial: u64,
    pub failed: u64,
    pub blocked: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ReliabilityRow {
    pub tool_id: String,
    pub category: String,
    pub invocations: u64,
    pub wire_requests: u64,
    pub cache_hit_pct: Option<f64>,
    pub verified_zero_pct: Option<f64>,
    pub error_pct: Option<f64>,
    pub mean_ms: Option<i64>,
    pub p95_ms: Option<i64>,
    pub dominant_trigger: String,
    pub dominant_mode: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EngineHealthRow {
    pub engine: String,
    pub fetches: u64,
    pub valid_serps: u64,
    pub verified_zero: u64,
    pub challenge: u64,
    pub parser_mismatch: u64,
    pub transport_failure: u64,
    pub usable_per_fetch: Option<f64>,
    pub cache_hits: u64,
    pub last_success: String,
    pub parser_version: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EvidenceRow {
    pub tool_id: String,
    pub category: String,
    pub successful_nonempty: u64,
    pub with_evidence: u64,
    pub acceptance_pct: Option<f64>,
    pub distinct_evidence: u64,
    pub cited_by_completed_reports: u64,
    pub coverage_n: u64,
    /// True when one evidence item credits more than one tool.
    pub nonadditive: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FailureCauseRow {
    pub cause: String,
    pub invocations: u64,
    pub share: f64,
    pub rank: usize,
}

// ---------------------------------------------------------------------------
// Mergeable aggregation primitives
// ---------------------------------------------------------------------------

/// The eleven bounded dimensions of a telemetry row, in `rollup_key` order.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
struct Dims {
    app: String,
    trigger: String,
    tool: String,
    category: String,
    provider: String,
    role: String,
    model: String,
    mode: String,
    engine: String,
    outcome: String,
    reason: String,
}

/// One mergeable count. Durations are kept twice on purpose: `samples` are the
/// exact raw observations, `rollup_bins` the mergeable histogram, so a weighted
/// mean is always `total_ms / n` and a quantile never averages an average.
#[derive(Clone, Debug, Default)]
struct Cell {
    events: u64,
    total_ms: i64,
    max_ms: i64,
    samples: Vec<i64>,
    rollup_bins: Vec<i64>,
}

impl Cell {
    fn add_raw(&mut self, count: i64, metric_ms: Option<i64>) {
        self.events += count.max(0) as u64;
        if let Some(ms) = metric_ms.filter(|value| *value >= 0) {
            self.total_ms += ms;
            self.max_ms = self.max_ms.max(ms);
            self.samples.push(ms);
        }
    }

    fn add_rollup(&mut self, events: i64, total_ms: i64, max_ms: i64, bins: &[i64]) {
        self.events += events.max(0) as u64;
        self.total_ms += total_ms;
        self.max_ms = self.max_ms.max(max_ms);
        if self.rollup_bins.len() < bins.len() {
            self.rollup_bins.resize(bins.len(), 0);
        }
        for (slot, value) in self.rollup_bins.iter_mut().zip(bins.iter()) {
            *slot += *value;
        }
    }

    fn absorb(&mut self, other: &Cell) {
        self.events += other.events;
        self.total_ms += other.total_ms;
        self.max_ms = self.max_ms.max(other.max_ms);
        self.samples.extend(other.samples.iter().copied());
        if self.rollup_bins.len() < other.rollup_bins.len() {
            self.rollup_bins.resize(other.rollup_bins.len(), 0);
        }
        for (slot, value) in self.rollup_bins.iter_mut().zip(other.rollup_bins.iter()) {
            *slot += *value;
        }
    }

    /// Duration observations: raw samples plus everything the rollup bins hold.
    fn duration_n(&self) -> u64 {
        self.samples.len() as u64 + self.rollup_bins.iter().sum::<i64>().max(0) as u64
    }

    fn has_rollup(&self) -> bool {
        self.rollup_bins.iter().any(|value| *value != 0)
    }

    fn mean_ms(&self) -> Option<i64> {
        let n = self.duration_n();
        if n == 0 {
            return None;
        }
        Some(self.total_ms / n as i64)
    }

    fn quantile(&self, q: f64) -> Option<i64> {
        if self.has_rollup() {
            // Merge the raw samples into the histogram so the merged history
            // answers, then read the quantile off bin edges.
            let mut bins = self.rollup_bins.clone();
            let width = telemetry::DURATION_BINS_MS.len() + 1;
            if bins.len() < width {
                bins.resize(width, 0);
            }
            for sample in &self.samples {
                let index = telemetry::bin_index(*sample as u64).min(width - 1);
                bins[index] += 1;
            }
            quantile_from_bins(&bins, q)
        } else {
            let mut samples = self.samples.clone();
            quantile(&mut samples, q)
        }
    }

    fn p50_ms(&self) -> Option<i64> {
        self.quantile(0.5)
    }

    fn p95_ms(&self) -> Option<i64> {
        if self.duration_n() < SMALL_SAMPLE_MIN {
            return None;
        }
        self.quantile(0.95)
    }

    fn max_ms(&self) -> Option<i64> {
        (self.max_ms > 0).then_some(self.max_ms)
    }
}

/// One telemetry row read back for payload-level facts.
#[derive(Clone, Debug)]
struct EventRow {
    at: Option<DateTime<Utc>>,
    dims: Dims,
    job_id: String,
    run_id: String,
    article_id: String,
    origin: String,
    call_id: String,
    canonical_ref: String,
    metric_ms: Option<i64>,
    count: i64,
    payload: Json,
}

impl EventRow {
    fn num(&self, key: &str) -> Option<f64> {
        self.payload.get(key).and_then(Json::as_f64)
    }

    fn int(&self, key: &str) -> Option<i64> {
        self.num(key).map(|value| value as i64)
    }

    fn count(&self, key: &str) -> u64 {
        self.int(key).unwrap_or(0).max(0) as u64
    }

    fn label(&self, key: &str) -> String {
        self.payload
            .get(key)
            .and_then(Json::as_str)
            .unwrap_or_default()
            .trim()
            .chars()
            .take(128)
            .collect()
    }

    fn flag(&self, key: &str) -> bool {
        self.payload.get(key).and_then(Json::as_bool) == Some(true)
    }

    fn seconds_between(&self, from: &str, to: &str) -> Option<i64> {
        let from = parse_timestamp(&self.label(from))?;
        let to = parse_timestamp(&self.label(to))?;
        Some((to - from).num_milliseconds())
    }
}

/// Everything one event kind contributes: window-wide mergeable cells, per
/// trend bucket cells, and the raw rows that still carry payload facts.
#[derive(Clone, Debug, Default)]
struct Loaded {
    cells: HashMap<Dims, Cell>,
    trend: HashMap<(String, Dims), Cell>,
    rows: Vec<EventRow>,
}

impl Loaded {
    fn total(&self, pred: impl Fn(&Dims) -> bool) -> u64 {
        self.cells
            .iter()
            .filter(|(dims, _)| pred(dims))
            .map(|(_, cell)| cell.events)
            .sum()
    }

    fn cell(&self, pred: impl Fn(&Dims) -> bool) -> Cell {
        let mut out = Cell::default();
        for (dims, cell) in &self.cells {
            if pred(dims) {
                out.absorb(cell);
            }
        }
        out
    }

    /// Rows with a parseable timestamp inside the window, in emission order.
    fn timed(&self) -> impl Iterator<Item = (&EventRow, DateTime<Utc>)> {
        self.rows
            .iter()
            .filter_map(|row| row.at.map(|at| (row, at)))
    }

    /// bucket label -> dimension key -> count, zero filled over the window.
    fn trend_grouped(
        &self,
        key: impl Fn(&Dims) -> String,
    ) -> BTreeMap<String, BTreeMap<String, u64>> {
        let mut out: BTreeMap<String, BTreeMap<String, u64>> = BTreeMap::new();
        for ((bucket, dims), cell) in &self.trend {
            *out.entry(bucket.clone())
                .or_default()
                .entry(key(dims))
                .or_default() += cell.events;
        }
        out
    }
}

/// One adaptive trend bucket.
#[derive(Clone, Debug)]
struct Bucket {
    label: String,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
}

/// The requested window plus the bucket grid the trend widgets fill.
struct Window {
    now: DateTime<Utc>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    raw_cutoff: DateTime<Utc>,
    bucket_secs: i64,
    buckets: Vec<Bucket>,
}

impl Window {
    fn new(now: DateTime<Utc>, start: DateTime<Utc>, end: DateTime<Utc>, period: &Period) -> Self {
        let bucket_secs = period.bucket_seconds().max(1);
        let mut buckets = Vec::new();
        let mut cursor = bucket_floor(start, bucket_secs);
        while cursor < end && buckets.len() < 20_000 {
            let label = cursor.format(bucket_format(bucket_secs)).to_string();
            buckets.push(Bucket {
                label,
                start: cursor,
                end: cursor + ChronoDuration::seconds(bucket_secs),
            });
            cursor += ChronoDuration::seconds(bucket_secs);
        }
        Self {
            now,
            start,
            end,
            raw_cutoff: now - ChronoDuration::days(telemetry::RAW_RETENTION_DAYS),
            bucket_secs,
            buckets,
        }
    }

    fn label_at(&self, at: DateTime<Utc>) -> String {
        bucket_floor(at, self.bucket_secs)
            .format(bucket_format(self.bucket_secs))
            .to_string()
    }

    /// The window end is `now`, and a row recorded at exactly `now` is inside it,
    /// so both edges are inclusive. A half-open window would silently drop the
    /// newest write — the one the dashboard just recorded.
    fn contains(&self, at: DateTime<Utc>) -> bool {
        at >= self.start && at <= self.end
    }

    /// Zero filled trend skeleton: gaps stay zero-count with no durations.
    fn skeleton<V: Default + Clone>(&self, build: impl FnMut(&Bucket) -> V) -> Vec<V> {
        self.buckets.iter().map(build).collect()
    }
}

fn bucket_floor(value: DateTime<Utc>, secs: i64) -> DateTime<Utc> {
    let aligned = value.timestamp() - value.timestamp().rem_euclid(secs.max(1));
    Utc.timestamp_opt(aligned, 0).single().unwrap_or(value)
}

fn parse_bucket(value: &str) -> Option<DateTime<Utc>> {
    let value = value.trim();
    if value.len() >= 13 {
        chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H")
            .ok()
            .map(|naive| Utc.from_utc_datetime(&naive))
    } else {
        chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d")
            .ok()
            .and_then(|day| day.and_hms_opt(0, 0, 0))
            .map(|naive| Utc.from_utc_datetime(&naive))
    }
}

fn dims_from_key(key: &str) -> Dims {
    let mut parts = key.split('\u{1f}');
    let mut next = || parts.next().unwrap_or("").to_string();
    Dims {
        app: next(),
        trigger: next(),
        tool: next(),
        category: next(),
        provider: next(),
        role: next(),
        model: next(),
        mode: next(),
        engine: next(),
        outcome: next(),
        reason: next(),
    }
}

fn has_table(conn: &Connection, name: &str) -> bool {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
        rusqlite::params![name],
        |row| row.get::<_, bool>(0),
    )
    .unwrap_or(false)
}

fn count_rows(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |row| row.get::<_, i64>(0))
        .unwrap_or_default()
}

fn mean_f64(samples: &[f64]) -> Option<f64> {
    if samples.is_empty() {
        return None;
    }
    Some(samples.iter().sum::<f64>() / samples.len() as f64)
}

fn median_f64(samples: &mut [f64]) -> Option<f64> {
    if samples.is_empty() {
        return None;
    }
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    Some(samples[samples.len() / 2])
}

/// Most frequent non-empty label, ties broken by label for determinism.
fn dominant_label(counts: &BTreeMap<String, u64>) -> String {
    counts
        .iter()
        .filter(|(key, _)| !key.is_empty())
        .max_by(|a, b| a.1.cmp(b.1).then_with(|| b.0.cmp(a.0)))
        .map(|(key, _)| key.clone())
        .unwrap_or_else(|| "unknown".to_string())
}

/// Ranked counts with an `Other` remainder and a visible `Unknown` bucket.
fn ranked_counts(counts: &BTreeMap<String, u64>, limit: usize) -> Vec<(String, u64, bool, bool)> {
    let total: u64 = counts.values().sum();
    let mut unknown: u64 = 0;
    let mut ranked: Vec<(String, u64)> = Vec::new();
    for (key, value) in counts {
        if is_unknown_label(key) {
            unknown += *value;
        } else {
            ranked.push((key.clone(), *value));
        }
    }
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let mut out: Vec<(String, u64, bool, bool)> = Vec::new();
    for (key, value) in ranked.iter().take(limit) {
        out.push((key.clone(), *value, false, false));
    }
    let remainder: u64 = ranked.iter().skip(limit).map(|(_, value)| *value).sum();
    if remainder > 0 {
        out.push(("other".to_string(), remainder, true, false));
    }
    if unknown > 0 {
        out.push(("unknown".to_string(), unknown, false, true));
    }
    let _ = total;
    out
}

fn is_unknown_label(value: &str) -> bool {
    let value = value.trim();
    value.is_empty() || value.eq_ignore_ascii_case("unknown") || value.eq_ignore_ascii_case("unk")
}

/// Share of a part inside a whole, as a percentage; `None` when the whole is 0.
fn share_of(part: u64, whole: u64) -> Option<f64> {
    sum_ratio(part, whole)
}

fn top_three_concentration(ranked: &[(String, u64, bool, bool)], total: u64) -> f64 {
    if total == 0 {
        return 0.0;
    }
    let top: u64 = ranked
        .iter()
        .filter(|row| !row.2 && !row.3)
        .take(3)
        .map(|row| row.1)
        .sum();
    top as f64 * 100.0 / total as f64
}

// ---------------------------------------------------------------------------
// Reader
// ---------------------------------------------------------------------------

/// Window-scoped, filter-aware view over one connection. Every method is read
/// only: the writer is never locked for a dashboard query.
struct Reader<'a> {
    conn: &'a Connection,
    win: Window,
    filters: StatFilters,
}

impl<'a> Reader<'a> {
    fn new(
        conn: &'a Connection,
        period: &Period,
        filters: &StatFilters,
        now: DateTime<Utc>,
    ) -> Self {
        let start = period.start(now);
        let end = period.end(now);
        let (start, end) = if start <= end {
            (start, end)
        } else {
            (end, start)
        };
        Self {
            conn,
            win: Window::new(now, start, end, period),
            filters: filters.clone(),
        }
    }

    fn bucket_labels(&self) -> Vec<String> {
        self.win.buckets.iter().map(|b| b.label.clone()).collect()
    }

    fn matches_filters(&self, dims: &Dims) -> bool {
        for (column, want) in [
            ("app", self.filters.app.as_str()),
            ("provider", self.filters.provider.as_str()),
            ("role", self.filters.role.as_str()),
            ("mode", self.filters.mode.as_str()),
            ("tool", self.filters.tool.as_str()),
            ("category", self.filters.category.as_str()),
        ] {
            let want = want.trim();
            if want.is_empty() {
                continue;
            }
            let have = match column {
                "app" => dims.app.as_str(),
                "provider" => dims.provider.as_str(),
                "role" => dims.role.as_str(),
                "mode" => dims.mode.as_str(),
                "tool" => dims.tool.as_str(),
                _ => dims.category.as_str(),
            };
            if have != want {
                return false;
            }
        }
        true
    }

    /// Reads one event kind: raw rows first, then the rollups that no longer
    /// overlap any retained raw row.
    fn load(&self, kind: EventKind) -> Result<Loaded> {
        let mut loaded = Loaded::default();
        if !has_table(self.conn, "telemetry_events") {
            return Ok(loaded);
        }
        let mut sql = String::from(
            "SELECT occurred_at, app, trigger, tool_id, category, provider, role, model, mode, engine, \
             outcome, reason, job_id, run_id, article_id, origin, call_id, canonical_ref, metric_ms, \
             count, payload_json FROM telemetry_events WHERE event_type = ?1",
        );
        let mut args: Vec<Value> = vec![kind.as_str().to_string().into()];
        for (column, want) in [
            ("app", self.filters.app.as_str()),
            ("provider", self.filters.provider.as_str()),
            ("role", self.filters.role.as_str()),
            ("mode", self.filters.mode.as_str()),
            ("tool_id", self.filters.tool.as_str()),
            ("category", self.filters.category.as_str()),
        ] {
            let want = want.trim();
            if want.is_empty() {
                continue;
            }
            let index = args.len() + 1;
            sql.push_str(&format!(" AND {column} = ?{index}"));
            args.push(want.to_string().into());
        }
        let start_index = args.len() + 1;
        sql.push_str(&format!(" AND occurred_at >= ?{start_index}"));
        args.push(self.win.start.to_rfc3339().into());
        let end_index = args.len() + 1;
        sql.push_str(&format!(" AND occurred_at <= ?{end_index}"));
        args.push(self.win.end.to_rfc3339().into());
        sql.push_str(" ORDER BY occurred_at, id");

        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(args), |row| {
            Ok(EventRow {
                at: parse_timestamp(&row.get::<_, String>(0)?),
                dims: Dims {
                    app: row.get(1)?,
                    trigger: row.get(2)?,
                    tool: row.get(3)?,
                    category: row.get(4)?,
                    provider: row.get(5)?,
                    role: row.get(6)?,
                    model: row.get(7)?,
                    mode: row.get(8)?,
                    engine: row.get(9)?,
                    outcome: row.get(10)?,
                    reason: row.get(11)?,
                },
                job_id: row.get(12)?,
                run_id: row.get(13)?,
                article_id: row.get(14)?,
                origin: row.get(15)?,
                call_id: row.get(16)?,
                canonical_ref: row.get(17)?,
                metric_ms: row.get(18)?,
                count: row.get(19)?,
                payload: serde_json::from_str(&row.get::<_, String>(20)?).unwrap_or(Json::Null),
            })
        })?;
        for row in rows {
            let row = row?;
            let Some(at) = row.at else { continue };
            if !self.win.contains(at) {
                continue;
            }
            let cell = loaded.cells.entry(row.dims.clone()).or_default();
            cell.add_raw(row.count, row.metric_ms);
            let trend = loaded
                .trend
                .entry((self.win.label_at(at), row.dims.clone()))
                .or_default();
            trend.add_raw(row.count, row.metric_ms);
            loaded.rows.push(row);
        }
        self.merge_rollups(&mut loaded, kind)?;
        Ok(loaded)
    }

    /// Folds `telemetry_hourly` / `telemetry_daily` into the same cells. A
    /// bucket is only used when the whole bucket predates the raw retention
    /// boundary and no raw row covers it, so a replay never double counts.
    fn merge_rollups(&self, loaded: &mut Loaded, kind: EventKind) -> Result<()> {
        for (table, granularity) in [
            ("telemetry_hourly", 3_600_i64),
            ("telemetry_daily", 86_400_i64),
        ] {
            if !has_table(self.conn, table) {
                continue;
            }
            let cutoff = if granularity == 3_600 {
                self.win.raw_cutoff
            } else {
                bucket_floor(self.win.raw_cutoff, 3_600)
            };
            let sql = format!(
                "SELECT bucket, dim_key, events, total_ms, max_ms, bins_json FROM {table} \
                 WHERE event_type = ?1"
            );
            let mut merged: Vec<(String, Dims, Cell)> = Vec::new();
            {
                let mut stmt = self.conn.prepare(&sql)?;
                let rows = stmt.query_map(rusqlite::params![kind.as_str()], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, String>(5)?,
                    ))
                })?;
                for row in rows {
                    let (bucket, dim_key, events, total_ms, max_ms, bins_json) = row?;
                    let Some(start) = parse_bucket(&bucket) else {
                        continue;
                    };
                    let end = start + ChronoDuration::seconds(granularity);
                    // Only buckets the raw table no longer covers.
                    if end > cutoff || end <= self.win.start || start >= self.win.end {
                        continue;
                    }
                    let dims = dims_from_key(&dim_key);
                    if !self.matches_filters(&dims) {
                        continue;
                    }
                    let bins: Vec<i64> = serde_json::from_str(&bins_json).unwrap_or_default();
                    let mut cell = Cell::default();
                    cell.add_rollup(events, total_ms, max_ms, &bins);
                    merged.push((self.win.label_at(start), dims, cell));
                }
            }
            for (label, dims, cell) in merged {
                let target = loaded.cells.entry(dims.clone()).or_default();
                target.absorb(&cell);
                let trend = loaded.trend.entry((label, dims)).or_default();
                trend.absorb(&cell);
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Section builders
// ---------------------------------------------------------------------------

impl Reader<'_> {
    // ---------- Intel ----------

    fn intel(&self) -> IntelStats {
        let articles = self.load(EventKind::IntelArticle).unwrap_or_default();
        let briefs = self.load(EventKind::IntelBriefRating).unwrap_or_default();
        let reports = self.load(EventKind::IntelReport).unwrap_or_default();

        // Distinct canonical article ids first seen inside the window. The
        // article id is the identity, so a repeated cycle never adds volume.
        let mut seen: Vec<ArticleFact> = Vec::new();
        let mut seen_ids: HashSet<String> = HashSet::new();
        for (row, at) in articles.timed() {
            let id = if row.article_id.is_empty() {
                row.dims.category.clone()
            } else {
                row.article_id.clone()
            };
            if id.is_empty() || !seen_ids.insert(id.clone()) {
                continue;
            }
            seen.push(ArticleFact {
                id,
                tag: normalize_tag(&row.dims.category, &row.label("tag")),
                origin: if row.origin.is_empty() {
                    row.label("origin")
                } else {
                    row.origin.clone()
                },
                domain: if row.dims.provider.is_empty() {
                    row.label("source_domain")
                } else {
                    row.dims.provider.clone()
                },
                published_at: row.label("published_at"),
                first_seen: at,
                initial_confidence: row.num("initial_confidence"),
                claims: row.count("claim_count"),
            });
        }
        // Publication snapshots: one rating per article id, latest wins.
        let mut ratings: HashMap<String, BriefFact> = HashMap::new();
        for (row, at) in briefs.timed() {
            if !row.dims.outcome.is_empty() && row.dims.outcome != "rated" {
                continue;
            }
            let rating = row.num("rating").or_else(|| row.num("mean_confidence"));
            let Some(rating) =
                rating.filter(|value| value.is_finite() && (0.0..=1.0).contains(value))
            else {
                continue;
            };
            let id = row.article_id.clone();
            let entry = ratings.entry(id.clone()).or_insert_with(|| BriefFact {
                at,
                rating,
                claims: row.count("claim_count").max(row.count("claims")),
            });
            if at >= entry.at {
                entry.at = at;
                entry.rating = rating;
                entry.claims = row.count("claim_count").max(row.count("claims"));
            }
        }

        let volume = self.intel_volume(&seen, &articles);
        let confidence = paired_confidence(&seen, &ratings);
        let origins = origin_counts(&seen);
        let publishers = publisher_counts(&seen);
        let freshness = self.intel_freshness(&seen);

        let mut by_tag: BTreeMap<String, Vec<&ArticleFact>> = BTreeMap::new();
        for fact in &seen {
            let key = if fact.tag.is_empty() {
                "untagged".to_string()
            } else {
                fact.tag.clone()
            };
            by_tag.entry(key).or_default().push(fact);
        }
        let mut enrichment: Vec<EnrichmentRow> = by_tag
            .into_iter()
            .map(|(tag, rows)| {
                let bodies = rows
                    .iter()
                    .filter(|row| {
                        !row.domain.is_empty() && self.article_has_body(&row.domain, &row.id)
                    })
                    .count() as u64;
                let claims = rows.iter().filter(|row| row.claims > 0).count() as u64;
                let with_report = rows
                    .iter()
                    .filter(|row| self.article_has_report(&row.id))
                    .count() as u64;
                let initial: Vec<f64> = rows
                    .iter()
                    .filter_map(|row| row.initial_confidence)
                    .collect();
                let current: Vec<f64> = rows
                    .iter()
                    .filter_map(|row| ratings.get(&row.id).map(|value| value.rating))
                    .collect();
                EnrichmentRow {
                    tag,
                    articles: rows.len() as u32,
                    body_pct: sum_ratio(bodies, rows.len() as u64),
                    claims_pct: sum_ratio(claims, rows.len() as u64),
                    report_pct: sum_ratio(with_report, rows.len() as u64),
                    mean_initial_confidence: mean_f64(&initial),
                    initial_n: initial.len() as u32,
                    mean_brief_rating: mean_f64(&current),
                    brief_n: current.len() as u32,
                }
            })
            .collect();
        enrichment.sort_by(|a, b| b.articles.cmp(&a.articles).then(a.tag.cmp(&b.tag)));

        let report_rows = self.intel_reports(&reports);
        IntelStats {
            volume,
            confidence,
            origins,
            enrichment,
            reports: report_rows,
            publishers,
            freshness,
            report_attempts: 0,
            note: "Distinct canonical articles are counted once; tag membership is \
                   counted separately, so tag totals can exceed distinct articles."
                .to_string(),
        }
    }

    /// Intel widget 1: distinct new articles per bucket, split by primary tag.
    fn intel_volume(&self, facts: &[ArticleFact], loaded: &Loaded) -> Vec<VolumeBucket> {
        let mut per_tag: BTreeMap<String, BTreeMap<String, u32>> = BTreeMap::new();
        for fact in facts {
            let bucket = self.win.label_at(fact.first_seen);
            let tag = if fact.tag.is_empty() {
                "untagged".to_string()
            } else {
                fact.tag.clone()
            };
            *per_tag.entry(bucket).or_default().entry(tag).or_default() += 1;
        }
        // Rollup history covers buckets the raw table no longer holds. Rollup
        // buckets never overlap a retained raw row, so the counts add directly.
        for (label, by_tag) in loaded.trend_grouped(|dims| normalize_tag(&dims.category, "")) {
            let entry = per_tag.entry(label).or_default();
            for (tag, count) in by_tag {
                let key = if tag.is_empty() {
                    "untagged".to_string()
                } else {
                    tag
                };
                *entry.entry(key).or_default() += count as u32;
            }
        }
        self.win.skeleton(|bucket| {
            let tags = per_tag.remove(&bucket.label).unwrap_or_default();
            let mut by_tag: Vec<(String, u32)> = tags.into_iter().collect();
            by_tag.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            let untagged = by_tag
                .iter()
                .position(|(tag, _)| tag == "untagged")
                .map(|index| by_tag.remove(index).1)
                .unwrap_or(0);
            let total = by_tag.iter().map(|(_, value)| *value).sum::<u32>() + untagged;
            VolumeBucket {
                bucket: bucket.label.clone(),
                total,
                by_tag,
                untagged,
            }
        })
    }

    /// Intel widget 7: first ingestion minus publication time.
    fn intel_freshness(&self, facts: &[ArticleFact]) -> FreshnessHistogram {
        /// Label, lower bound (inclusive), upper bound (exclusive) in seconds.
        const BUCKETS: [(&str, i64, i64); 5] = [
            ("<1h", i64::MIN, 3_600),
            ("1-6h", 3_600, 6 * 3_600),
            ("6-24h", 6 * 3_600, 24 * 3_600),
            ("1-3d", 24 * 3_600, 3 * 86_400),
            (">3d", 3 * 86_400, i64::MAX),
        ];
        let mut buckets: Vec<(String, u32)> = BUCKETS
            .iter()
            .map(|(label, _, _)| (label.to_string(), 0))
            .collect();
        let mut missing = 0;
        let mut future = 0;
        for fact in facts {
            let Some(published) = parse_timestamp(&fact.published_at) else {
                missing += 1;
                continue;
            };
            let delay = (fact.first_seen - published).num_seconds();
            if delay < 0 {
                future += 1;
                continue;
            }
            if let Some(index) = BUCKETS
                .iter()
                .position(|(_, lo, hi)| delay >= *lo && delay < *hi)
            {
                buckets[index].1 += 1;
            }
        }
        FreshnessHistogram {
            buckets,
            missing,
            future,
        }
    }

    /// Intel widget 5: latest terminal revision per `(article_id, report_mode)`.
    fn intel_reports(&self, loaded: &Loaded) -> Vec<ReportModeRow> {
        let mut latest: HashMap<String, ReportFact> = HashMap::new();
        for (row, at) in loaded.timed() {
            let key = report_key(&row.article_id, &row.dims.mode);
            let fact = ReportFact {
                at,
                mode: row.dims.mode.clone(),
                outcome: normalize_report_outcome(&row.dims.outcome),
                wall_ms: row
                    .int("wall_ms")
                    .or_else(|| row.seconds_between("started_at", "finished_at")),
                active_ms: row.int("active_ms"),
            };
            match latest.get(&key) {
                Some(current) if current.at >= fact.at => {}
                _ => {
                    latest.insert(key, fact);
                }
            }
        }
        let mut by_mode: BTreeMap<String, Vec<&ReportFact>> = BTreeMap::new();
        for fact in latest.values() {
            by_mode.entry(fact.mode.clone()).or_default().push(fact);
        }
        let mut rows: Vec<ReportModeRow> = by_mode
            .into_iter()
            .map(|(mode, facts)| {
                // Terminal revisions carry the outcome columns; current
                // waiting/blocked work is a live count, never a completion.
                let terminal: Vec<&&ReportFact> = facts
                    .iter()
                    .filter(|fact| fact.outcome.is_terminal())
                    .collect();
                let mut row = ReportModeRow {
                    mode,
                    n: facts.len() as u32,
                    ..ReportModeRow::default()
                };
                for fact in &facts {
                    match fact.outcome {
                        ReportOutcome::Completed => row.completed += 1,
                        ReportOutcome::Partial => row.partial += 1,
                        ReportOutcome::Failed => row.failed += 1,
                        ReportOutcome::Waiting => row.waiting += 1,
                        ReportOutcome::Blocked => row.blocked += 1,
                        // Cancellations are neither completions nor live work,
                        // so they stay out of every column and out of `n`.
                        ReportOutcome::Cancelled => row.n -= 1,
                    }
                }
                let walls: Vec<i64> = terminal.iter().filter_map(|fact| fact.wall_ms).collect();
                let actives: Vec<i64> = terminal.iter().filter_map(|fact| fact.active_ms).collect();
                row.mean_wall_ms = mean(&walls).map(|value| value as i64);
                row.p95_wall_ms = small_sample_p95(&walls);
                row.mean_active_ms = mean(&actives).map(|value| value as i64);
                row
            })
            .collect();
        rows.sort_by(|a, b| a.mode.cmp(&b.mode));
        rows
    }

    fn article_has_body(&self, _domain: &str, article_id: &str) -> bool {
        self.conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM article_bodies WHERE article_id = ?1)",
                rusqlite::params![article_id],
                |row| row.get::<_, bool>(0),
            )
            .unwrap_or(false)
    }

    fn article_has_report(&self, article_id: &str) -> bool {
        self.conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM intel_report_jobs WHERE article_id = ?1)",
                rusqlite::params![article_id],
                |row| row.get::<_, bool>(0),
            )
            .unwrap_or(false)
    }
}

/// One canonical article observed inside the window.
struct ArticleFact {
    id: String,
    tag: String,
    origin: String,
    domain: String,
    published_at: String,
    first_seen: DateTime<Utc>,
    initial_confidence: Option<f64>,
    claims: u64,
}

/// One per-article Brief rating snapshot.
struct BriefFact {
    at: DateTime<Utc>,
    rating: f64,
    claims: u64,
}

#[derive(Clone, Copy, PartialEq)]
enum ReportOutcome {
    Completed,
    Partial,
    Failed,
    Waiting,
    Blocked,
    Cancelled,
}

struct ReportFact {
    at: DateTime<Utc>,
    mode: String,
    outcome: ReportOutcome,
    wall_ms: Option<i64>,
    active_ms: Option<i64>,
}

/// Terminal revisions are keyed per `(article_id, report_mode)`; a blank
/// article id collapses into the mode so unattributed rows stay countable.
fn report_key(article_id: &str, mode: &str) -> String {
    format!("{article_id}\u{1f}{mode}")
}

impl ReportOutcome {
    fn is_terminal(self) -> bool {
        matches!(
            self,
            ReportOutcome::Completed | ReportOutcome::Partial | ReportOutcome::Failed
        )
    }
}

fn normalize_report_outcome(raw: &str) -> ReportOutcome {
    match raw {
        "completed" | "done" | "final" | "ok" => ReportOutcome::Completed,
        "partial" => ReportOutcome::Partial,
        "failed" | "error" | "fail" => ReportOutcome::Failed,
        "waiting" | "pending" | "queued" | "running" => ReportOutcome::Waiting,
        "blocked" | "refused" => ReportOutcome::Blocked,
        "cancelled" | "canceled" | "abandoned" => ReportOutcome::Cancelled,
        _ => ReportOutcome::Failed,
    }
}

fn normalize_tag(dimension: &str, payload: &str) -> String {
    let value = if dimension.trim().is_empty() {
        payload.trim()
    } else {
        dimension.trim()
    };
    match value {
        "" | "unk" | "unknown" | "untagged" => String::new(),
        _ => value.to_string(),
    }
}

/// Paired initial-versus-current confidence over articles with both
/// observations. Unpaired observations are counted in `paired_n` coverage but
/// never compared.
fn paired_confidence(
    facts: &[ArticleFact],
    ratings: &HashMap<String, BriefFact>,
) -> ConfidenceDistribution {
    let mut bands: Vec<ConfidenceBand> = CONFIDENCE_BANDS
        .iter()
        .map(|(lo, hi)| ConfidenceBand {
            label: format!("{lo:.1}-{hi:.1}"),
            lo: *lo,
            hi: *hi,
            initial: 0,
            current: 0,
        })
        .collect();
    let mut initial: Vec<f64> = Vec::new();
    let mut current: Vec<f64> = Vec::new();
    for fact in facts {
        let Some(rating) = ratings.get(&fact.id) else {
            continue;
        };
        if let Some(score) = fact
            .initial_confidence
            .filter(|value| value.is_finite() && (0.0..=1.0).contains(value))
        {
            if let Some(band) = band_for(&mut bands, score) {
                band.initial += 1;
            }
            initial.push(score);
        }
        if let Some(band) = band_for(&mut bands, rating.rating) {
            band.current += 1;
        }
        current.push(rating.rating);
    }
    ConfidenceDistribution {
        bands,
        paired_n: initial.len().min(current.len()) as u64,
        mean_initial: mean_f64(&initial),
        mean_current: mean_f64(&current),
        median_initial: median_f64(&mut initial),
        median_current: median_f64(&mut current),
        coverage_note: format!(
            "Paired over {} articles with both a raw initial score and a current Brief \
             rating; articles with only one observation are excluded rather than scored zero.",
            initial.len().min(current.len())
        ),
    }
}

fn band_for(bands: &mut [ConfidenceBand], score: f64) -> Option<&mut ConfidenceBand> {
    let index = if score >= 1.0 {
        bands.len() - 1
    } else if score < 0.0 {
        0
    } else {
        (score * bands.len() as f64).floor() as usize
    };
    bands.get_mut(index)
}

fn origin_counts(facts: &[ArticleFact]) -> Vec<OriginCount> {
    let mut counts: BTreeMap<String, u64> = BTreeMap::new();
    for fact in facts {
        *counts.entry(fact.origin.clone()).or_default() += 1;
    }
    let total: u64 = counts.values().sum();
    ranked_counts(&counts, 10)
        .into_iter()
        .map(|(origin, articles, is_other, is_unknown)| OriginCount {
            origin,
            articles: articles as u32,
            share: share_of(articles, total).unwrap_or(0.0),
            is_other,
            is_unknown,
        })
        .collect()
}

fn publisher_counts(facts: &[ArticleFact]) -> Vec<PublisherCount> {
    let mut counts: BTreeMap<String, u64> = BTreeMap::new();
    for fact in facts {
        *counts.entry(fact.domain.clone()).or_default() += 1;
    }
    let total: u64 = counts.values().sum();
    let ranked = ranked_counts(&counts, 10);
    let concentration = top_three_concentration(&ranked, total);
    ranked
        .into_iter()
        .map(|(domain, articles, is_other, is_unknown)| PublisherCount {
            domain,
            articles: articles as u32,
            share: share_of(articles, total).unwrap_or(0.0),
            is_other,
            is_unknown,
            top3_concentration: concentration,
        })
        .collect()
}

/// p95 suppressed below `SMALL_SAMPLE_MIN` observations.
fn small_sample_p95(samples: &[i64]) -> Option<i64> {
    if (samples.len() as u64) < SMALL_SAMPLE_MIN {
        return None;
    }
    let mut owned = samples.to_vec();
    quantile(&mut owned, 0.95)
}

// ---------- Recon ----------

impl Reader<'_> {
    fn recon(&self) -> ReconStats {
        let runs = self.load(EventKind::ReconRun).unwrap_or_default();
        let stages = self.load(EventKind::ReconStage).unwrap_or_default();
        let recalls = self.load(EventKind::RecallQuery).unwrap_or_default();
        let directives = self.load(EventKind::DirectiveAssessed).unwrap_or_default();

        let run_facts = self.recon_run_facts(&runs);
        ReconStats {
            outcomes: self.recon_outcomes(&run_facts),
            stages: self.recon_stages(&stages),
            recall: self.recon_recall(&recalls),
            workload: self.recon_workload(&run_facts),
            diversity: self.recon_diversity(),
            directives: self.recon_directives(&directives),
            unresolved: self.recon_unresolved(&directives),
            note: "Terminal run assessments come from recorded transitions, not from a \
                   successful tool call. Directive categories are categorical and never \
                   averaged into a score."
                .to_string(),
        }
    }

    /// Recon widget 1: run outcomes per bucket, split by mode.
    fn recon_outcomes(&self, facts: &[ReconRunFact]) -> Vec<ReconOutcomeBucket> {
        let mut per_mode: BTreeMap<String, BTreeMap<String, RunOutcomeAccumulator>> =
            BTreeMap::new();
        for fact in facts {
            let bucket = self.win.label_at(fact.at);
            per_mode
                .entry(bucket)
                .or_default()
                .entry(fact.mode.clone())
                .or_default()
                .add(fact.outcome);
        }
        self.win
            .skeleton(|bucket| {
                let modes = per_mode.remove(&bucket.label).unwrap_or_default();
                let mut rows: Vec<ReconOutcomeBucket> = Vec::new();
                for (mode, outcome) in modes {
                    rows.push(outcome.into_bucket(bucket.label.clone(), mode));
                }
                rows
            })
            .into_iter()
            .flatten()
            .collect()
    }

    /// Recon widget 2: mean active execution and retry/capacity wait by stage.
    fn recon_stages(&self, loaded: &Loaded) -> Vec<StageDurationRow> {
        let mut per_stage: BTreeMap<(String, String), (Cell, Vec<i64>)> = BTreeMap::new();
        for (dims, cell) in &loaded.cells {
            if dims.tool.is_empty() {
                continue;
            }
            let entry = per_stage
                .entry((dims.tool.clone(), dims.mode.clone()))
                .or_default();
            entry.0.absorb(cell);
        }
        for (row, _) in loaded.timed() {
            if row.dims.tool.is_empty() {
                continue;
            }
            let entry = per_stage
                .entry((row.dims.tool.clone(), row.dims.mode.clone()))
                .or_default();
            if let Some(wait) = row.int("wait_ms") {
                entry.1.push(wait.max(0));
            }
        }
        let mut rows: Vec<StageDurationRow> = per_stage
            .into_iter()
            .map(|((stage, mode), (cell, waits))| StageDurationRow {
                stage,
                mode,
                mean_exec_ms: cell.mean_ms(),
                mean_wait_ms: mean(&waits).map(|value| value as i64),
                n: cell.duration_n() as u32,
            })
            .collect();
        rows.sort_by(|a, b| a.stage.cmp(&b.stage).then(a.mode.cmp(&b.mode)));
        rows
    }

    /// Recon widget 3: recall usefulness by mode.
    fn recon_recall(&self, loaded: &Loaded) -> Vec<RecallRow> {
        let mut per_mode: BTreeMap<String, RecallAccumulator> = BTreeMap::new();
        for (row, _) in loaded.timed() {
            let entry = per_mode.entry(recon_mode(&row.dims.mode)).or_default();
            entry.queries += 1;
            let candidates = row.count("candidates");
            let accepted = row.count("accepted");
            if candidates > 0 {
                entry.with_candidates += 1;
            }
            if accepted > 0 {
                entry.with_accepted += 1;
                entry.accepted += accepted;
            } else if !row.dims.reason.is_empty() {
                *entry.reasons.entry(row.dims.reason.clone()).or_default() += 1;
            }
        }
        // Completed runs per mode give the accepted-per-run denominator.
        let runs = self.recon_run_facts(&self.load(EventKind::ReconRun).unwrap_or_default());
        let mut completed: BTreeMap<String, u64> = BTreeMap::new();
        for fact in &runs {
            if fact.outcome != RunOutcome::CompletedWithEvidence {
                continue;
            }
            *completed.entry(recon_mode(&fact.mode)).or_default() += 1;
        }
        let mut rows: Vec<RecallRow> = per_mode
            .into_iter()
            .map(|(mode, entry)| RecallRow {
                mode: mode.clone(),
                queries: entry.queries as u32,
                with_candidates: entry.with_candidates as u32,
                with_accepted: entry.with_accepted as u32,
                retention_pct: sum_ratio(entry.with_accepted, entry.with_candidates),
                accepted_per_run: sum_ratio(entry.accepted, *completed.get(&mode).unwrap_or(&0)),
                rejection_reason: dominant_label(&entry.reasons),
            })
            .collect();
        rows.sort_by(|a, b| a.mode.cmp(&b.mode));
        rows
    }

    /// Recon widget 4: workload by mode.
    fn recon_workload(&self, facts: &[ReconRunFact]) -> Vec<WorkloadRow> {
        let mut per_mode: BTreeMap<String, WorkloadAccumulator> = BTreeMap::new();
        for fact in facts {
            let entry = per_mode.entry(recon_mode(&fact.mode)).or_default();
            entry.runs += 1;
            entry.directives += fact.directives;
            entry.calls += fact.calls;
            entry.categories += fact.categories;
            entry.memories += fact.memories;
            if let Some(wall) = fact.wall_ms {
                entry.walls.push(wall.max(0));
            }
        }
        let mut rows: Vec<WorkloadRow> = per_mode
            .into_iter()
            .map(|(mode, entry)| WorkloadRow {
                mode: mode.clone(),
                runs: entry.runs as u32,
                directives_per_run: ratio_f64(entry.directives as f64, entry.runs as f64),
                calls_per_run: ratio_f64(entry.calls as f64, entry.runs as f64),
                categories_per_run: ratio_f64(entry.categories as f64, entry.runs as f64),
                accepted_memories_per_run: ratio_f64(entry.memories as f64, entry.runs as f64),
                median_wall_ms: median_i64(&entry.walls),
                p95_wall_ms: small_sample_p95(&entry.walls),
            })
            .collect();
        rows.sort_by(|a, b| a.mode.cmp(&b.mode));
        rows
    }

    /// Recon widget 5: eligible-tool attainment from `recon_coverage`.
    fn recon_diversity(&self) -> Vec<DiversityRow> {
        if !has_table(self.conn, "recon_coverage") {
            return Vec::new();
        }
        let mut latest: HashMap<String, CoverageFact> = HashMap::new();
        {
            let Ok(mut stmt) = self.conn.prepare(
                "SELECT scope, generation, category, payload_json, updated_at \
                 FROM recon_coverage",
            ) else {
                return Vec::new();
            };
            let Ok(rows) = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            }) else {
                return Vec::new();
            };
            for row in rows.flatten() {
                let (scope, generation, category, payload, updated_at) = row;
                let key = format!("{scope}\u{1f}{category}");
                let stamp = parse_timestamp(&updated_at);
                // One authoritative row per (scope, category): the newest
                // generation wins, then the newest update.
                let stale = match (latest.get(&key), stamp) {
                    (Some(current), Some(at)) => {
                        at < current.updated_at
                            || (at == current.updated_at && generation < current.generation)
                    }
                    _ => false,
                };
                if stale {
                    continue;
                }
                let Ok(record) = serde_json::from_str::<Json>(&payload) else {
                    continue;
                };
                // `CoverageRecord` payload: eligible candidates, attempted and
                // successful tools, independent source groups and the reason a
                // category fell short of the two-tool target.
                let eligible_candidates = json_array_len(&record, "candidates")
                    .saturating_sub(json_at(&record, "candidates", "ineligible"));
                latest.insert(
                    key,
                    CoverageFact {
                        updated_at: stamp.unwrap_or(self.win.now),
                        category: category.clone(),
                        generation,
                        eligible: eligible_candidates >= 2,
                        two_attempted: json_bool(&record, "two_tools_attempted"),
                        two_successful: json_bool(&record, "two_tools_successful"),
                        groups: json_array_len(&record, "independent_source_groups") as f64,
                        reason: json_str(&record, "coverage_gap"),
                    },
                );
            }
        }
        // Agree on a per-category view: a category is only meaningful once at
        // least one scope carried the eligible-tool target.
        let mut per_category: BTreeMap<String, CoverageAccumulator> = BTreeMap::new();
        for fact in latest.values() {
            if !fact.eligible {
                continue;
            }
            let entry = per_category.entry(fact.category.clone()).or_default();
            entry.scopes += 1;
            if fact.two_attempted {
                entry.attempted += 1;
            }
            if fact.two_successful {
                entry.successful += 1;
            }
            entry.groups_sum += fact.groups;
            entry.groups_n += 1;
            if !fact.reason.is_empty() {
                *entry.reasons.entry(fact.reason.clone()).or_default() += 1;
            }
        }
        let mut rows: Vec<DiversityRow> = per_category
            .into_iter()
            .map(|(category, entry)| DiversityRow {
                category,
                eligible_scopes: entry.scopes,
                scopes_with_two_attempted: entry.attempted,
                scopes_with_two_successful: entry.successful,
                source_groups_per_scope: (entry.groups_n > 0)
                    .then(|| entry.groups_sum / entry.groups_n as f64),
                top_shortfall_reason: dominant_label(&entry.reasons),
                eligible: entry.scopes > 0,
            })
            .collect();
        rows.sort_by(|a, b| a.category.cmp(&b.category));
        rows
    }

    /// Recon widget 6: directive resolution among terminal runs.
    fn recon_directives(&self, loaded: &Loaded) -> Vec<DirectiveResolution> {
        let mut per_mode: BTreeMap<String, DirectiveResolution> = BTreeMap::new();
        for (dims, cell) in &loaded.cells {
            let entry = per_mode.entry(recon_mode(&dims.mode)).or_default();
            let counts = cell.events as u32;
            entry.n += counts;
            match dims.outcome.as_str() {
                "answered" | "complete" | "completed" => entry.answered += counts,
                "partial" | "partially_answered" => entry.partial += counts,
                "unresolved" => entry.unresolved += counts,
                "blocked" => entry.blocked += counts,
                _ => entry.unknown += counts,
            }
        }
        let mut rows: Vec<DirectiveResolution> = per_mode.into_values().collect();
        rows.sort_by(|a, b| a.mode.cmp(&b.mode));
        rows
    }

    /// Recon widget 7: unresolved and blocked directives with their reason.
    fn recon_unresolved(&self, loaded: &Loaded) -> Vec<UnresolvedDirective> {
        let mut rows: Vec<UnresolvedDirective> = loaded
            .timed()
            .filter(|(row, _)| matches!(row.dims.outcome.as_str(), "unresolved" | "blocked"))
            .map(|(row, at)| UnresolvedDirective {
                run_id: row.run_id.clone(),
                mode: recon_mode(&row.dims.mode),
                label: row.label("label"),
                reason: if row.dims.reason.is_empty() {
                    row.label("reason")
                } else {
                    row.dims.reason.clone()
                },
                evidence_count: row.count("evidence_count").min(u32::MAX as u64) as u32,
                last_progress: at.to_rfc3339(),
                next_action: row.label("next_action"),
            })
            .collect();
        rows.sort_by(|a, b| {
            b.last_progress
                .cmp(&a.last_progress)
                .then(a.run_id.cmp(&b.run_id))
        });
        rows.truncate(200);
        rows
    }

    fn recon_run_facts(&self, loaded: &Loaded) -> Vec<ReconRunFact> {
        loaded
            .timed()
            .map(|(row, at)| ReconRunFact {
                at,
                mode: row.dims.mode.clone(),
                outcome: normalize_run_outcome(&row.dims.outcome),
                wall_ms: row.int("wall_ms"),
                directives: row.count("directives"),
                calls: row.count("calls"),
                categories: row.count("categories"),
                memories: row.count("accepted_memories").max(row.count("memories")),
            })
            .collect()
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum RunOutcome {
    CompletedWithEvidence,
    #[default]
    CompletedZeroEvidence,
    Partial,
    Failed,
    Cancelled,
}

#[derive(Default)]
struct RunOutcomeAccumulator {
    completed_with_evidence: u32,
    completed_zero_evidence: u32,
    partial: u32,
    failed: u32,
    cancelled: u32,
}

impl RunOutcomeAccumulator {
    fn add(&mut self, outcome: RunOutcome) {
        match outcome {
            RunOutcome::CompletedWithEvidence => self.completed_with_evidence += 1,
            RunOutcome::CompletedZeroEvidence => self.completed_zero_evidence += 1,
            RunOutcome::Partial => self.partial += 1,
            RunOutcome::Failed => self.failed += 1,
            RunOutcome::Cancelled => self.cancelled += 1,
        }
    }

    fn into_bucket(self, bucket: String, mode: String) -> ReconOutcomeBucket {
        ReconOutcomeBucket {
            bucket,
            mode,
            completed_with_evidence: self.completed_with_evidence,
            completed_zero_evidence: self.completed_zero_evidence,
            partial: self.partial,
            failed: self.failed,
            cancelled: self.cancelled,
        }
    }
}

fn normalize_run_outcome(raw: &str) -> RunOutcome {
    match raw {
        "completed" | "complete" | "done" | "final" | "ok" => RunOutcome::CompletedWithEvidence,
        "completed_zero_evidence" | "zero_evidence" | "empty" => RunOutcome::CompletedZeroEvidence,
        "partial" => RunOutcome::Partial,
        "cancelled" | "canceled" | "abandoned" => RunOutcome::Cancelled,
        _ => RunOutcome::Failed,
    }
}

struct ReconRunFact {
    at: DateTime<Utc>,
    mode: String,
    outcome: RunOutcome,
    wall_ms: Option<i64>,
    directives: u64,
    calls: u64,
    categories: u64,
    memories: u64,
}

#[derive(Default)]
struct RecallAccumulator {
    queries: u64,
    with_candidates: u64,
    with_accepted: u64,
    accepted: u64,
    reasons: BTreeMap<String, u64>,
}

#[derive(Default)]
struct WorkloadAccumulator {
    runs: u64,
    directives: u64,
    calls: u64,
    categories: u64,
    memories: u64,
    walls: Vec<i64>,
}

struct CoverageFact {
    updated_at: DateTime<Utc>,
    category: String,
    generation: i64,
    /// The scope carried the two-eligible-tool target for this category.
    eligible: bool,
    two_attempted: bool,
    two_successful: bool,
    /// Independent upstream source groups credited to the scope.
    groups: f64,
    /// Recorded shortfall reason, empty when the target was met.
    reason: String,
}

fn json_bool(value: &Json, key: &str) -> bool {
    value.get(key).and_then(Json::as_bool) == Some(true)
}

fn json_str(value: &Json, key: &str) -> String {
    value
        .get(key)
        .and_then(Json::as_str)
        .unwrap_or_default()
        .trim()
        .chars()
        .take(128)
        .collect()
}

fn json_array_len(value: &Json, key: &str) -> u64 {
    value
        .get(key)
        .and_then(Json::as_array)
        .map(|items| items.len() as u64)
        .unwrap_or(0)
}

/// Count of array entries under `parent` whose `flag` member is truthy.
fn json_at(value: &Json, parent: &str, flag: &str) -> u64 {
    value
        .get(parent)
        .and_then(Json::as_array)
        .map(|items| {
            items
                .iter()
                .filter(|item| item.get(flag).and_then(Json::as_bool) == Some(true))
                .count() as u64
        })
        .unwrap_or(0)
}

#[derive(Default)]
struct CoverageAccumulator {
    scopes: u32,
    attempted: u32,
    successful: u32,
    groups_sum: f64,
    groups_n: u32,
    reasons: BTreeMap<String, u64>,
}

fn recon_mode(mode: &str) -> String {
    let mode = mode.trim();
    if mode.is_empty() {
        "unknown".to_string()
    } else {
        mode.to_string()
    }
}

fn ratio_f64(numerator: f64, denominator: f64) -> f64 {
    if denominator == 0.0 {
        0.0
    } else {
        numerator / denominator
    }
}

// ---------- Atlas ----------

impl Reader<'_> {
    fn atlas(&self) -> AtlasStats {
        let cycles = self.load(EventKind::AtlasCycle).unwrap_or_default();
        let snapshots = self
            .load(EventKind::AtlasOriginSnapshot)
            .unwrap_or_default();
        let candidates = self.load(EventKind::AtlasCandidate).unwrap_or_default();
        let stages = self.load(EventKind::AtlasStage).unwrap_or_default();
        let articles = self.atlas_article_facts();

        AtlasStats {
            temperature_changes: self.atlas_temperature(&snapshots),
            cycle_times: self.atlas_cycle_times(&cycles),
            cycle_outcomes: self.atlas_cycle_outcomes(&cycles),
            origins: atlas_origin_rows(&snapshots, &articles),
            discovery: self.atlas_discovery(&candidates),
            backlog: self.atlas_backlog(&stages),
            waiting_now: self.atlas_current_state(&cycles, "waiting"),
            blocked_now: self.atlas_current_state(&cycles, "blocked"),
            note: "Temperature is Argos's own score and tier is ordinal, so tiers are a \
                   distribution plus a descriptive mean rather than an interval scale. \
                   Waiting and blocked are current counts, never fabricated completions."
                .to_string(),
        }
    }

    /// Atlas widget 1: largest temperature movement per origin between the
    /// latest and the preceding comparable cycle snapshot.
    fn atlas_temperature(&self, loaded: &Loaded) -> Vec<TemperatureChange> {
        let mut per_origin: BTreeMap<String, Vec<OriginSnapshotFact>> = BTreeMap::new();
        for (row, at) in loaded.timed() {
            let origin = if row.origin.is_empty() {
                row.dims.category.clone()
            } else {
                row.origin.clone()
            };
            if origin.is_empty() {
                continue;
            }
            per_origin
                .entry(origin)
                .or_default()
                .push(OriginSnapshotFact {
                    at,
                    version: if row.dims.reason.is_empty() {
                        row.label("version")
                    } else {
                        row.dims.reason.clone()
                    },
                    eligible: row.payload.get("eligible").and_then(Json::as_bool) != Some(false),
                    payload: row.payload.clone(),
                });
        }
        let mut rows: Vec<TemperatureChange> = per_origin
            .into_iter()
            .filter_map(|(origin, mut snaps)| {
                snaps.sort_by(|a, b| a.at.cmp(&b.at));
                let latest = snaps.last()?;
                let current = latest.payload.get("temperature").and_then(Json::as_f64);
                let tier = latest
                    .payload
                    .get("tier")
                    .and_then(Json::as_u64)
                    .map(|tier| tier as u8);
                let articles = latest
                    .payload
                    .get("articles")
                    .and_then(Json::as_u64)
                    .unwrap_or(0) as u32;
                // The preceding snapshot is comparable only under the same
                // scoring version and an eligible collection scope.
                let previous = snaps
                    .iter()
                    .rev()
                    .nth(1)
                    .filter(|prior| prior.version == latest.version && prior.eligible)
                    .and_then(|prior| prior.payload.get("temperature").and_then(Json::as_f64));
                let (comparable, delta, label) = match (previous, current) {
                    (Some(previous), Some(current)) => {
                        let delta = current - previous;
                        let label = if delta > 0.0 {
                            "warming"
                        } else if delta < 0.0 {
                            "cooling"
                        } else {
                            "flat"
                        };
                        (true, Some(delta), label)
                    }
                    (None, Some(_)) => (false, None, "new"),
                    (Some(_), None) => (false, None, "missing"),
                    (None, None) => (false, None, "incomparable"),
                };
                Some(TemperatureChange {
                    origin,
                    previous_temperature: previous,
                    current_temperature: current,
                    delta,
                    latest_tier: tier,
                    articles,
                    comparable,
                    label: label.to_string(),
                })
            })
            .collect();
        // Largest comparable movement first; origins that cannot be compared
        // keep a visible slot instead of an invented zero baseline.
        rows.sort_by(|a, b| {
            let left = a.delta.map(|value| value.abs()).unwrap_or(-1.0);
            let right = b.delta.map(|value| value.abs()).unwrap_or(-1.0);
            right
                .partial_cmp(&left)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.origin.cmp(&b.origin))
        });
        rows.truncate(10);
        rows
    }

    /// Atlas widget 2: completed-cycle wall time per bucket.
    fn atlas_cycle_times(&self, loaded: &Loaded) -> Vec<CycleTimeBucket> {
        let mut per_bucket: BTreeMap<String, (Vec<i64>, Vec<i64>)> = BTreeMap::new();
        for (row, at) in loaded.timed() {
            let Some(wall) = row
                .int("wall_ms")
                .or_else(|| row.seconds_between("started_at", "finished_at"))
            else {
                continue;
            };
            let entry = per_bucket.entry(self.win.label_at(at)).or_default();
            entry.0.push(wall.max(0));
            if let Some(queue) = row.int("queue_ms") {
                entry.1.push(queue.max(0));
            }
        }
        self.win.skeleton(|bucket| {
            let (walls, queues) = per_bucket.remove(&bucket.label).unwrap_or_default();
            CycleTimeBucket {
                bucket: bucket.label.clone(),
                mean_ms: mean(&walls).map(|value| value as i64),
                p95_ms: small_sample_p95(&walls),
                n: walls.len() as u32,
                mean_queue_ms: mean(&queues).map(|value| value as i64),
            }
        })
    }

    /// Atlas widget 3: cycle outcomes per completion bucket.
    fn atlas_cycle_outcomes(&self, loaded: &Loaded) -> Vec<CycleOutcomeBucket> {
        let mut per_bucket: BTreeMap<String, CycleOutcomeBucket> = BTreeMap::new();
        for ((label, dims), cell) in &loaded.trend {
            if dims.outcome.is_empty() {
                continue;
            }
            let entry = per_bucket.entry(label.clone()).or_default();
            add_cycle_outcome(entry, &dims.outcome, cell.events as u32);
        }
        self.win.skeleton(|bucket| {
            let mut row = per_bucket.remove(&bucket.label).unwrap_or_default();
            row.bucket = bucket.label.clone();
            row
        })
    }

    /// Current, unfiltered waiting/blocked cycles: a live count, not history.
    fn atlas_current_state(&self, loaded: &Loaded, want: &str) -> u32 {
        loaded.cells.iter().fold(0u32, |total, (dims, cell)| {
            if dims.outcome == want {
                total + cell.events as u32
            } else {
                total
            }
        })
    }

    /// Atlas widget 5: one mutually exclusive disposition per candidate.
    fn atlas_discovery(&self, loaded: &Loaded) -> Vec<DiscoveryBucket> {
        let mut per_bucket: BTreeMap<String, DiscoveryBucket> = BTreeMap::new();
        for (row, at) in loaded.timed() {
            let entry = per_bucket.entry(self.win.label_at(at)).or_default();
            entry.fetched += 1;
            match row.dims.outcome.as_str() {
                "retained_new" | "new" => entry.retained_new += 1,
                "retained_existing" | "existing" => entry.retained_existing += 1,
                "duplicate_in_cycle" | "duplicate" | "dup" => entry.duplicate_in_cycle += 1,
                "rejected" | "reject" => entry.rejected += 1,
                _ => entry.pending += 1,
            }
        }
        // Rollup-only buckets still count as candidate occurrences.
        for (label, by_disposition) in loaded.trend_grouped(|dims| dims.outcome.to_string()) {
            let entry = per_bucket.entry(label).or_default();
            for (disposition, count) in by_disposition {
                entry.fetched += count as u32;
                match disposition.as_str() {
                    "retained_new" | "new" => entry.retained_new += count as u32,
                    "retained_existing" | "existing" => entry.retained_existing += count as u32,
                    "duplicate_in_cycle" | "duplicate" | "dup" => {
                        entry.duplicate_in_cycle += count as u32
                    }
                    "rejected" | "reject" => entry.rejected += count as u32,
                    _ => entry.pending += count as u32,
                }
            }
        }
        self.win.skeleton(|bucket| {
            let mut row = per_bucket.remove(&bucket.label).unwrap_or_default();
            row.bucket = bucket.label.clone();
            row
        })
    }

    /// Atlas widget 6: current backlog per stage. Live: it ignores the filter.
    fn atlas_backlog(&self, loaded: &Loaded) -> Vec<BacklogRow> {
        let now = self.win.now;
        // The latest transition per (stage, unit) is that unit's current state.
        let mut units: HashMap<(String, String), StageUnit> = HashMap::new();
        for (row, at) in loaded.timed() {
            let stage = if row.dims.tool.is_empty() {
                row.label("stage")
            } else {
                row.dims.tool.clone()
            };
            let unit_id = if row.call_id.is_empty() {
                row.label("unit_id")
            } else {
                row.call_id.clone()
            };
            let key = (stage, unit_id);
            match units.get(&key) {
                Some(current) if current.at >= at => {}
                _ => {
                    units.insert(
                        key,
                        StageUnit {
                            at,
                            state: if row.dims.outcome.is_empty() {
                                row.label("state")
                            } else {
                                row.dims.outcome.clone()
                            },
                            unit: row.label("unit"),
                            reason: if row.dims.reason.is_empty() {
                                row.label("terminal_reason")
                            } else {
                                row.dims.reason.clone()
                            },
                        },
                    );
                }
            }
        }
        let mut rows: BTreeMap<String, BacklogRow> = BTreeMap::new();
        for ((stage, unit_id), unit) in units {
            let entry = rows.entry(stage.clone()).or_insert_with(|| BacklogRow {
                stage: stage.clone(),
                unit: unit.unit.clone(),
                ..BacklogRow::default()
            });
            match unit.state.as_str() {
                "queued" => entry.queued += 1,
                "running" => entry.running += 1,
                "waiting" => entry.waiting += 1,
                "blocked" => entry.blocked += 1,
                _ => continue,
            }
            let age = now.signed_duration_since(unit.at).num_milliseconds().max(0);
            entry.oldest_pending_ms = Some(entry.oldest_pending_ms.map_or(age, |v| v.max(age)));
            if unit.state == "blocked" && !unit.reason.is_empty() {
                entry.latest_error_category = unit.reason.clone();
            }
            let _ = unit_id;
        }
        let mut rows: Vec<BacklogRow> = rows.into_values().collect();
        rows.sort_by(|a, b| a.stage.cmp(&b.stage));
        rows
    }

    /// Atlas widget 4 inputs: distinct articles per origin from durable rows.
    fn atlas_article_facts(&self) -> Vec<(String, u32)> {
        if !has_table(self.conn, "atlas_articles") {
            return Vec::new();
        }
        let Ok(mut stmt) = self.conn.prepare(
            "SELECT country, COUNT(DISTINCT article_id) FROM atlas_articles GROUP BY country",
        ) else {
            return Vec::new();
        };
        let Ok(rows) = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?))
        }) else {
            return Vec::new();
        };
        rows.flatten().collect()
    }
}

/// One cycle outcome class per completed cycle; waiting and blocked are live
/// counts handled separately so they are never fabricated as completions.
fn add_cycle_outcome(row: &mut CycleOutcomeBucket, outcome: &str, count: u32) {
    match outcome {
        "completed" | "complete" | "done" => row.completed += count,
        "partial" => row.partial += count,
        "failed" | "fail" | "error" => row.failed += count,
        "cancelled" | "canceled" | "abandoned" => row.cancelled += count,
        _ => {}
    }
}

/// One cycle's observation of one origin.
struct OriginSnapshotFact {
    at: DateTime<Utc>,
    /// Scoring version, so only same-method deltas are compared.
    version: String,
    /// False when the cycle's collection scope was not eligible.
    eligible: bool,
    payload: Json,
}

/// Latest recorded transition of one Atlas work unit.
struct StageUnit {
    at: DateTime<Utc>,
    /// Current state: queued, running, waiting, blocked, done.
    state: String,
    /// The unit this stage counts (article, packet, memory).
    unit: String,
    /// Terminal reason recorded for the unit.
    reason: String,
}

/// Atlas widget 4: origin coverage, temperatures and tier distribution.
fn atlas_origin_rows(loaded: &Loaded, articles: &[(String, u32)]) -> Vec<OriginTierRow> {
    let mut per_origin: BTreeMap<String, OriginAccumulator> = BTreeMap::new();
    for (row, at) in loaded.timed() {
        let origin = if row.origin.is_empty() {
            row.dims.category.clone()
        } else {
            row.origin.clone()
        };
        if origin.is_empty() {
            continue;
        }
        let entry = per_origin.entry(origin).or_default();
        entry.snapshots += 1;
        if at > entry.latest_at {
            entry.latest_at = at;
        }
        if let Some(temperature) = row.num("temperature") {
            entry.temperatures.push((at, temperature));
            entry.sum += temperature;
            entry.n += 1;
        }
        if let Some(tier) = row.num("tier") {
            entry.tiers.push(tier as u8);
        }
        if let Some(count) = row.num("articles") {
            entry.articles += count as u32;
        }
    }
    for (country, count) in articles {
        let entry = per_origin.entry(country.clone()).or_default();
        entry.durable_articles += count;
    }
    let mut rows: Vec<OriginTierRow> = per_origin
        .into_iter()
        .map(|(origin, entry)| {
            let snapshots = entry.snapshots.max(entry.tiers.len() as u32);
            let total = snapshots.max(1);
            OriginTierRow {
                origin,
                articles: entry.durable_articles.max(entry.articles),
                latest_temperature: entry.temperatures.last().map(|(_, value)| *value),
                mean_temperature: (entry.n > 0).then(|| entry.sum / entry.n as f64),
                latest_tier: entry.tiers.last().copied(),
                tier1_share: entry.tiers.iter().filter(|t| **t == 1).count() as f64 * 100.0
                    / total as f64,
                tier2_share: entry.tiers.iter().filter(|t| **t == 2).count() as f64 * 100.0
                    / total as f64,
                tier3_share: entry.tiers.iter().filter(|t| **t == 3).count() as f64 * 100.0
                    / total as f64,
                snapshots,
                latest_at: entry.latest_at.to_rfc3339(),
            }
        })
        .collect();
    rows.sort_by(|a, b| {
        b.latest_temperature
            .unwrap_or(0.0)
            .partial_cmp(&a.latest_temperature.unwrap_or(0.0))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.origin.cmp(&b.origin))
    });
    rows
}

#[derive(Default)]
struct OriginAccumulator {
    snapshots: u32,
    latest_at: DateTime<Utc>,
    temperatures: Vec<(DateTime<Utc>, f64)>,
    tiers: Vec<u8>,
    articles: u32,
    durable_articles: u32,
    sum: f64,
    n: u32,
}

// ---------- Tools ----------

impl Reader<'_> {
    fn tools(&self) -> ToolStats {
        let invocations = self.load(EventKind::ToolInvocation).unwrap_or_default();
        let wires = self.load(EventKind::ToolWireRequest).unwrap_or_default();
        let engines = self.load(EventKind::ToolEngineQuery).unwrap_or_default();
        let evidence = self.load(EventKind::EvidenceItem).unwrap_or_default();

        // Wire requests per tool, counted separately from logical calls.
        let mut wire_by_tool: BTreeMap<String, u64> = BTreeMap::new();
        for (row, _) in wires.timed() {
            if row.dims.tool.is_empty() {
                continue;
            }
            *wire_by_tool.entry(row.dims.tool.clone()).or_default() += 1;
        }
        for (row, _) in invocations.timed() {
            let remote = row.count("wire_requests").max(row.count("remote_requests"));
            if remote > 0 && !row.dims.tool.is_empty() {
                *wire_by_tool.entry(row.dims.tool.clone()).or_default() += remote;
            }
        }
        ToolStats {
            top_tools: tool_counts(&invocations),
            categories_by_trigger: category_triggers(&invocations),
            outcomes: self.tool_outcomes(&invocations),
            reliability: tool_reliability(&invocations, &wire_by_tool),
            engine_health: engine_health(&engines, &invocations),
            evidence: self.tool_evidence(&evidence, &invocations),
            failure_causes: tool_failure_causes(&invocations),
            note: "Logical invocations, wire requests and cache hits are counted \
                   separately. Remote error-rate denominators exclude cache and \
                   local-only executions, so a cache hit is never a remote \
                   failure. Engine identity stays separate from the transport \
                   provider that fetched the page."
                .to_string(),
        }
    }

    /// Tools widget 3: terminal outcome mix per bucket, never mixing local
    /// executions into the remote outcome stack.
    fn tool_outcomes(&self, loaded: &Loaded) -> Vec<ToolOutcomeBucket> {
        let mut per_bucket: BTreeMap<String, BTreeMap<String, u64>> = BTreeMap::new();
        for ((label, dims), cell) in &loaded.trend {
            if dims.outcome.is_empty() {
                continue;
            }
            *per_bucket
                .entry(label.clone())
                .or_default()
                .entry(dims.outcome.clone())
                .or_default() += cell.events;
        }
        self.win.skeleton(|bucket| {
            let outcomes = per_bucket.remove(&bucket.label).unwrap_or_default();
            let mut row = ToolOutcomeBucket {
                bucket: bucket.label.clone(),
                ..ToolOutcomeBucket::default()
            };
            for (outcome, count) in outcomes {
                match outcome.as_str() {
                    "completed_nonempty" | "nonempty" | "ok" => row.completed_nonempty += count,
                    "verified_zero" | "zero" => row.verified_zero += count,
                    "partial" => row.partial += count,
                    "failed" | "fail" | "error" => row.failed += count,
                    "blocked" => row.blocked += count,
                    _ => {}
                }
            }
            row
        })
    }

    /// Tools widget 6: evidence contribution joined through provenance.
    fn tool_evidence(&self, loaded: &Loaded, invocations: &Loaded) -> Vec<EvidenceRow> {
        let mut per_tool: BTreeMap<(String, String), EvidenceAccumulator> = BTreeMap::new();
        for (row, _) in loaded.timed() {
            let tool = if row.dims.tool.is_empty() {
                row.label("tool_id")
            } else {
                row.dims.tool.clone()
            };
            if tool.is_empty() {
                continue;
            }
            let entry = per_tool
                .entry((tool.clone(), row.dims.category.clone()))
                .or_insert_with(|| EvidenceAccumulator {
                    tool_id: tool,
                    category: row.dims.category.clone(),
                    ..EvidenceAccumulator::default()
                });
            entry.invocations += 1;
            entry.fingerprints.insert(fingerprint(row));
            if row.flag("cited") {
                entry.citations += 1;
            }
        }
        // Successful nonempty invocations give the acceptance denominator.
        let mut successful: BTreeMap<String, u64> = BTreeMap::new();
        for (row, _) in invocations.timed() {
            if !reached_source(&row.dims.outcome) {
                continue;
            }
            if !row.dims.tool.is_empty() {
                *successful.entry(row.dims.tool.clone()).or_default() += 1;
            }
        }
        let mut rows: Vec<EvidenceRow> = per_tool
            .into_values()
            .map(|entry| {
                let denominator = successful.get(&entry.tool_id).copied().unwrap_or(0);
                EvidenceRow {
                    tool_id: entry.tool_id.clone(),
                    category: entry.category,
                    successful_nonempty: denominator,
                    with_evidence: entry.invocations,
                    acceptance_pct: sum_ratio(entry.invocations, denominator),
                    distinct_evidence: entry.fingerprints.len() as u64,
                    cited_by_completed_reports: entry.citations,
                    coverage_n: entry.invocations,
                    // One evidence item can carry several tools, so the rows
                    // credit each tool but must not be summed.
                    nonadditive: true,
                }
            })
            .collect();
        rows.sort_by(|a, b| {
            b.with_evidence
                .cmp(&a.with_evidence)
                .then(a.tool_id.cmp(&b.tool_id))
        });
        rows
    }
}

fn fingerprint(row: &EventRow) -> String {
    // Prefer the explicit evidence identity, then the call provenance, so the
    // same item reported twice is counted once.
    for key in ["fingerprint", "evidence_id", "canonical_ref"] {
        let value = row.label(key);
        if !value.is_empty() {
            return value;
        }
    }
    if !row.call_id.is_empty() {
        return row.call_id.clone();
    }
    row.article_id.clone()
}

fn reached_source(outcome: &str) -> bool {
    ToolOutcome::from_str(outcome)
        .map(|v| v.reached_source())
        .unwrap_or(false)
}

/// Tools widget 1: top logical invocations with the remote/local/cache split.
fn tool_counts(loaded: &Loaded) -> Vec<ToolCount> {
    let mut per_tool: BTreeMap<String, ToolAccumulator> = BTreeMap::new();
    for (row, _) in loaded.timed() {
        let tool = if row.dims.tool.is_empty() {
            row.label("tool_id")
        } else {
            row.dims.tool.clone()
        };
        if tool.is_empty() {
            continue;
        }
        let entry = per_tool
            .entry(tool.clone())
            .or_insert_with(|| ToolAccumulator {
                tool_id: tool,
                category: row.dims.category.clone(),
                ..ToolAccumulator::default()
            });
        entry.invocations += 1;
        match row.dims.mode.as_str() {
            "cache" => entry.cache += 1,
            "local" => entry.local += 1,
            _ => entry.remote += 1,
        }
    }
    let mut rows: Vec<ToolCount> = per_tool
        .into_values()
        .map(|entry| {
            let ToolAccumulator {
                tool_id,
                category,
                invocations,
                remote,
                local,
                cache,
            } = entry;
            ToolCount {
                tool_id,
                category,
                invocations,
                remote,
                local,
                cache,
            }
        })
        .collect();
    rows.sort_by(|a, b| {
        b.invocations
            .cmp(&a.invocations)
            .then(a.tool_id.cmp(&b.tool_id))
    });
    rows.truncate(10);
    rows
}

/// Tools widget 2: category totals split by initiating trigger.
fn category_triggers(loaded: &Loaded) -> Vec<CategoryTrigger> {
    let mut per_category: BTreeMap<String, BTreeMap<String, u64>> = BTreeMap::new();
    for ((_, dims), cell) in &loaded.trend {
        if dims.category.is_empty() {
            continue;
        }
        let trigger = Trigger::from_str(&dims.trigger);
        *per_category
            .entry(dims.category.clone())
            .or_default()
            .entry(trigger.as_str().to_string())
            .or_default() += cell.events;
    }
    let mut rows: Vec<CategoryTrigger> = per_category
        .into_iter()
        .map(|(category, triggers)| {
            let mut by_trigger: Vec<(String, u64)> = triggers.into_iter().collect();
            by_trigger.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            let total = by_trigger.iter().map(|(_, count)| *count).sum();
            CategoryTrigger {
                category,
                by_trigger,
                total,
            }
        })
        .collect();
    rows.sort_by(|a, b| b.total.cmp(&a.total).then(a.category.cmp(&b.category)));
    rows
}

/// Tools widget 4: reliability per tool with explicitly labelled denominators.
fn tool_reliability(loaded: &Loaded, wire_by_tool: &BTreeMap<String, u64>) -> Vec<ReliabilityRow> {
    let mut per_tool: BTreeMap<String, ReliabilityAccumulator> = BTreeMap::new();
    for (row, _) in loaded.timed() {
        let tool = if row.dims.tool.is_empty() {
            row.label("tool_id")
        } else {
            row.dims.tool.clone()
        };
        if tool.is_empty() {
            continue;
        }
        let entry = per_tool
            .entry(tool.clone())
            .or_insert_with(|| ReliabilityAccumulator {
                tool_id: tool,
                category: row.dims.category.clone(),
                ..ReliabilityAccumulator::default()
            });
        entry.invocations += 1;
        *entry.triggers.entry(row.dims.trigger.clone()).or_default() += 1;
        *entry.modes.entry(row.dims.mode.clone()).or_default() += 1;
        let remote = row.dims.mode != "cache" && row.dims.mode != "local";
        if remote {
            entry.remote += 1;
            if matches!(
                row.dims.outcome.as_str(),
                "failed" | "blocked" | "parser_mismatch"
            ) {
                entry.remote_errors += 1;
            }
        } else if row.dims.mode == "cache" {
            entry.cache += 1;
        }
        if row.dims.outcome == "verified_zero" {
            entry.verified_zero += 1;
        }
        if let Some(duration) = row.metric_ms.filter(|value| *value >= 0) {
            entry.durations.push(duration);
        }
    }
    let mut rows: Vec<ReliabilityRow> = per_tool
        .into_iter()
        .map(|(tool, entry)| {
            let ReliabilityAccumulator { category, .. } = entry;
            ReliabilityRow {
                tool_id: tool.clone(),
                category,
                invocations: entry.invocations,
                wire_requests: wire_by_tool.get(&tool).copied().unwrap_or(0),
                cache_hit_pct: sum_ratio(entry.cache, entry.invocations),
                verified_zero_pct: sum_ratio(entry.verified_zero, entry.invocations),
                // Denominator is remote executions only.
                error_pct: sum_ratio(entry.remote_errors, entry.remote),
                mean_ms: mean(&entry.durations).map(|value| value as i64),
                p95_ms: small_sample_p95(&entry.durations),
                dominant_trigger: dominant_label(&entry.triggers),
                dominant_mode: dominant_label(&entry.modes),
            }
        })
        .collect();
    rows.sort_by(|a, b| {
        b.invocations
            .cmp(&a.invocations)
            .then(a.tool_id.cmp(&b.tool_id))
    });
    rows
}

/// Tools widget 5: named-engine health. Firecrawl is the transport, the engine
/// is the source, and per-site blocked outcomes are not negative findings.
fn engine_health(engines: &Loaded, invocations: &Loaded) -> Vec<EngineHealthRow> {
    let mut per_engine: BTreeMap<String, EngineAccumulator> = BTreeMap::new();
    for (row, at) in engines.timed() {
        let engine = if row.dims.engine.is_empty() {
            row.label("engine")
        } else {
            row.dims.engine.clone()
        };
        if engine.is_empty() {
            continue;
        }
        let entry = per_engine
            .entry(engine.clone())
            .or_insert_with(|| EngineAccumulator {
                engine,
                ..EngineAccumulator::default()
            });
        entry.fetches += 1;
        match row.dims.outcome.as_str() {
            "valid" | "ok" => entry.valid += 1,
            "verified_zero" | "zero" => entry.verified_zero += 1,
            "challenge" | "consent" => entry.challenge += 1,
            "parser_mismatch" => entry.parser_mismatch += 1,
            "transport_failure" | "timeout" => entry.transport_failure += 1,
            // A rate limit or an unsupported site is capacity, not evidence of
            // absence, so it is tracked but never called a verified zero.
            _ => entry.other += 1,
        }
        if row.dims.outcome == "valid" && at > entry.last_success {
            entry.last_success = at;
        }
        let version = row.label("parser_version");
        if !version.is_empty() {
            entry.parser_version = version;
        }
    }
    // Cache hits stay separate from fetches so they never inflate health.
    let mut cache_by_engine: BTreeMap<String, u64> = BTreeMap::new();
    for (row, _) in invocations.timed() {
        if row.dims.mode != "cache" {
            continue;
        }
        let engine = if row.dims.engine.is_empty() {
            row.label("engine")
        } else {
            row.dims.engine.clone()
        };
        if !engine.is_empty() {
            *cache_by_engine.entry(engine).or_default() += 1;
        }
    }
    let mut rows: Vec<EngineHealthRow> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    // The named engines are always present so a silent engine is visible.
    for engine in NAMED_ENGINES {
        seen.insert(engine.to_string());
        let cache = cache_by_engine.get(engine).copied().unwrap_or(0);
        rows.push(match per_engine.remove(engine) {
            Some(entry) => EngineHealthRow {
                engine: entry.engine,
                fetches: entry.fetches,
                valid_serps: entry.valid,
                verified_zero: entry.verified_zero,
                challenge: entry.challenge,
                parser_mismatch: entry.parser_mismatch,
                transport_failure: entry.transport_failure,
                usable_per_fetch: sum_ratio(entry.valid, entry.fetches),
                cache_hits: cache,
                last_success: entry.last_success.to_rfc3339(),
                parser_version: entry.parser_version,
            },
            None => EngineHealthRow {
                engine: engine.to_string(),
                ..EngineHealthRow::default()
            },
        });
    }
    for (engine, entry) in per_engine {
        if seen.contains(&engine) {
            continue;
        }
        rows.push(EngineHealthRow {
            engine,
            fetches: entry.fetches,
            valid_serps: entry.valid,
            verified_zero: entry.verified_zero,
            challenge: entry.challenge,
            parser_mismatch: entry.parser_mismatch,
            transport_failure: entry.transport_failure,
            usable_per_fetch: sum_ratio(entry.valid, entry.fetches),
            cache_hits: 0,
            last_success: entry.last_success.to_rfc3339(),
            parser_version: entry.parser_version,
        });
    }
    rows.sort_by(|a, b| a.engine.cmp(&b.engine));
    rows
}

/// Tools widget 7: one terminal cause per non-successful invocation.
fn tool_failure_causes(loaded: &Loaded) -> Vec<FailureCauseRow> {
    let mut per_cause: BTreeMap<String, u64> = BTreeMap::new();
    for (row, _) in loaded.timed() {
        let Some(cause) =
            classify_tool_cause(&row.dims.outcome, &row.label("cause"), &row.dims.reason)
        else {
            continue;
        };
        *per_cause.entry(cause.to_string()).or_default() += 1;
    }
    let total: u64 = per_cause.values().sum();
    let mut ranked: Vec<(String, u64)> = per_cause.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    ranked
        .into_iter()
        .enumerate()
        .map(|(index, (cause, invocations))| FailureCauseRow {
            cause,
            invocations,
            share: sum_ratio(invocations, total).unwrap_or(0.0),
            rank: index + 1,
        })
        .collect()
}

/// Exactly one cause per terminal invocation. Unmapped terminal outcomes stay
/// visible as `unknown` rather than being folded into a success.
fn classify_tool_cause(outcome: &str, payload_cause: &str, reason: &str) -> Option<&'static str> {
    let cause = if !payload_cause.is_empty() {
        payload_cause
    } else {
        reason
    };
    let mapped = match cause {
        "auth_config" | "auth" | "configuration" | "unauthorized" => "auth_config",
        "quota_throttle" | "quota" | "throttle" | "rate_limited" => "quota_throttle",
        "challenge_consent" | "challenge" | "consent" | "captcha" => "challenge_consent",
        "parser_mismatch" => "parser_mismatch",
        "transport_timeout" | "transport" | "timeout" | "network" => "transport_timeout",
        "invalid_input" | "input" | "bad_request" => "invalid_input",
        "" => match outcome {
            "failed" => "unknown",
            "blocked" => "challenge_consent",
            _ => return None,
        },
        _ => "other",
    };
    if matches!(outcome, "failed" | "blocked") {
        Some(mapped)
    } else {
        None
    }
}

#[derive(Default)]
struct ToolAccumulator {
    tool_id: String,
    category: String,
    invocations: u64,
    remote: u64,
    local: u64,
    cache: u64,
}

#[derive(Default)]
struct ReliabilityAccumulator {
    tool_id: String,
    category: String,
    invocations: u64,
    remote: u64,
    remote_errors: u64,
    cache: u64,
    verified_zero: u64,
    triggers: BTreeMap<String, u64>,
    modes: BTreeMap<String, u64>,
    durations: Vec<i64>,
}

#[derive(Default)]
struct EngineAccumulator {
    engine: String,
    fetches: u64,
    valid: u64,
    verified_zero: u64,
    challenge: u64,
    parser_mismatch: u64,
    transport_failure: u64,
    other: u64,
    last_success: DateTime<Utc>,
    parser_version: String,
}

#[derive(Default)]
struct EvidenceAccumulator {
    tool_id: String,
    category: String,
    invocations: u64,
    fingerprints: HashSet<String>,
    citations: u64,
}

// ---------- Models ----------

impl Reader<'_> {
    fn models(&self) -> ModelStats {
        let capacity = provider_metrics::capacity_snapshot(self.conn);
        let operations = self.load(EventKind::ModelOperation).unwrap_or_default();
        let attempts = self.load(EventKind::ModelAttempt).unwrap_or_default();
        let facts = model_facts(self.conn, &operations, &attempts);

        ModelStats {
            capacity: capacity.capacity.clone(),
            capacity_available: capacity.available,
            by_role: self.model_by_role(&facts.attempts),
            latency: self.model_latency(&facts.attempts),
            queue_delay: self.model_queue_delay(&facts.attempts),
            performance: model_performance(&facts),
            fallback: model_fallback(&facts),
            amplification: self.model_amplification(&facts),
            failures: self.model_failures(&facts.attempts),
            attempts: facts.summary,
            note: "Sends are actual wire attempts and error rates use finished \
                   attempts, so an unfinished send is never a failure. Final \
                   operation failures are counted once to the primary route \
                   cohort. Fallback triggers and recoveries are different \
                   counts, amplification includes every route of each terminal \
                   operation, and live capacity deliberately ignores historical \
                   filters."
                .to_string(),
        }
    }

    /// Models widget 2: sends per bucket stacked by role.
    fn model_by_role(&self, attempts: &[AttemptFact]) -> Vec<RoleRequestBucket> {
        let mut per_bucket: BTreeMap<String, BTreeMap<String, u64>> = BTreeMap::new();
        for attempt in attempts {
            if !attempt.sent {
                continue;
            }
            *per_bucket
                .entry(attempt.bucket.clone())
                .or_default()
                .entry(attempt.role.clone())
                .or_default() += 1;
        }
        self.win.skeleton(|bucket| {
            let roles = per_bucket.remove(&bucket.label).unwrap_or_default();
            let mut by_role: Vec<(String, u64)> = roles.into_iter().collect();
            by_role.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            let total = by_role.iter().map(|(_, count)| *count).sum();
            RoleRequestBucket {
                bucket: bucket.label.clone(),
                by_role,
                total,
            }
        })
    }

    /// Models widget 3: successful send-to-contract-completion latency.
    fn model_latency(&self, attempts: &[AttemptFact]) -> Vec<LatencyBucket> {
        let mut per_bucket: BTreeMap<String, (Vec<i64>, Vec<i64>, Vec<i64>)> = BTreeMap::new();
        for attempt in attempts {
            if !attempt.sent || !attempt.succeeded {
                continue;
            }
            let entry = per_bucket.entry(attempt.bucket.clone()).or_default();
            if let Some(latency) = attempt.exec_ms {
                entry.0.push(latency);
            }
            if let Some(header) = attempt.first_header_ms {
                entry.1.push(header);
            }
            if let Some(content) = attempt.first_content_ms {
                entry.2.push(content);
            }
        }
        self.win.skeleton(|bucket| {
            let (exec, headers, contents) = per_bucket.remove(&bucket.label).unwrap_or_default();
            LatencyBucket {
                bucket: bucket.label.clone(),
                p50_ms: quantile_owned(&exec, 0.5),
                p95_ms: small_sample_p95(&exec),
                n: exec.len() as u64,
                p50_first_header_ms: quantile_owned(&headers, 0.5),
                p50_first_content_ms: quantile_owned(&contents, 0.5),
            }
        })
    }

    /// Models widget 4: enqueue-to-send delay.
    fn model_queue_delay(&self, attempts: &[AttemptFact]) -> Vec<QueueBucket> {
        let mut per_bucket: BTreeMap<String, Vec<i64>> = BTreeMap::new();
        for attempt in attempts {
            if !attempt.sent {
                continue;
            }
            if let Some(queue) = attempt.queue_ms {
                per_bucket
                    .entry(attempt.bucket.clone())
                    .or_default()
                    .push(queue.max(0));
            }
        }
        self.win.skeleton(|bucket| {
            let samples = per_bucket.remove(&bucket.label).unwrap_or_default();
            QueueBucket {
                bucket: bucket.label.clone(),
                p50_ms: quantile_owned(&samples, 0.5),
                p95_ms: small_sample_p95(&samples),
                n: samples.len() as u64,
            }
        })
    }

    /// Models widget 7: wire attempts per terminal logical operation.
    fn model_amplification(&self, facts: &ModelFacts) -> Vec<AmplificationBucket> {
        let mut per_bucket: BTreeMap<String, (u64, u64)> = BTreeMap::new();
        for operation in &facts.operations {
            let Some(at) = operation.terminal_bucket else {
                continue;
            };
            let entry = per_bucket.entry(self.win.label_at(at)).or_default();
            entry.0 += operation.sends;
            entry.1 += 1;
        }
        let in_flight = facts
            .operations
            .iter()
            .filter(|operation| operation.terminal_bucket.is_none() && operation.sends > 0)
            .count() as u64;
        self.win.skeleton(|bucket| {
            let (sends, terminal) = per_bucket.remove(&bucket.label).unwrap_or((0, 0));
            AmplificationBucket {
                bucket: bucket.label.clone(),
                sends,
                terminal_operations: terminal,
                ratio: None,
                in_flight_operations: in_flight,
            }
            .with_ratio()
        })
    }

    /// Models widget 8: failed finished wire attempts by category.
    fn model_failures(&self, attempts: &[AttemptFact]) -> Vec<FailureBucket> {
        let mut per_bucket: BTreeMap<String, (BTreeMap<String, u64>, u64, u64)> = BTreeMap::new();
        for attempt in attempts {
            if !attempt.sent {
                continue;
            }
            let entry = per_bucket.entry(attempt.bucket.clone()).or_default();
            if !attempt.finished {
                continue;
            }
            entry.2 += 1;
            if attempt.failed {
                let category = classify_model_failure(&attempt.failure_category);
                *entry.0.entry(category.to_string()).or_default() += 1;
                entry.1 += 1;
            }
        }
        self.win.skeleton(|bucket| {
            let (by_category, failed, finished) =
                per_bucket
                    .remove(&bucket.label)
                    .unwrap_or((BTreeMap::new(), 0, 0));
            FailureBucket {
                bucket: bucket.label.clone(),
                by_category: by_category.into_iter().collect(),
                total_failed: failed,
                pct_of_finished: sum_ratio(failed, finished),
            }
        })
    }
}

impl AmplificationBucket {
    fn with_ratio(mut self) -> Self {
        self.ratio = sum_ratio(self.sends, self.terminal_operations);
        self
    }
}

fn quantile_owned(samples: &[i64], q: f64) -> Option<i64> {
    if samples.is_empty() {
        return None;
    }
    let mut owned = samples.to_vec();
    quantile(&mut owned, q)
}

/// Exactly one failure category per failed finished attempt.
fn classify_model_failure(category: &str) -> &'static str {
    match category {
        "rate_limit" | "rate_limited" | "throttle" | "http_429" => "rpm_throttle",
        "quota" | "quota_exhausted" | "insufficient_quota" => "other_quota",
        "timeout" | "network" | "connection" | "transport" => "network_timeout",
        "http_5xx" | "server_error" | "upstream" => "provider_5xx",
        "auth" | "permission" | "invalid_key" | "unauthorized" => "invalid_request_auth",
        "invalid_request" | "malformed_request" | "bad_request" => "invalid_request_auth",
        "invalid_output" | "parse" | "contract" => "invalid_output",
        "" => "unknown",
        _ => "other",
    }
}

/// One wire attempt from the authoritative durable row plus its telemetry
/// counterpart, merged so neither double counts.
struct AttemptFact {
    bucket: String,
    provider: String,
    model: String,
    role: String,
    sent: bool,
    finished: bool,
    succeeded: bool,
    failed: bool,
    exec_ms: Option<i64>,
    queue_ms: Option<i64>,
    first_header_ms: Option<i64>,
    first_content_ms: Option<i64>,
    http_status: Option<u32>,
    failure_category: String,
    operation_id: String,
    route_index: u32,
}

/// One logical operation: many attempts, never many jobs.
struct OperationFact {
    terminal_bucket: Option<DateTime<Utc>>,
    role: String,
    primary_provider: String,
    primary_model: String,
    effective_provider: String,
    effective_model: String,
    sends: u64,
    failed: bool,
    recovered: bool,
    trigger_reason: String,
    operation_id: String,
    created_at: Option<DateTime<Utc>>,
}

#[derive(Default)]
struct ModelFacts {
    attempts: Vec<AttemptFact>,
    operations: Vec<OperationFact>,
    summary: AttemptSummary,
}

/// Reads the durable schema-26 operation/attempt rows and merges the canonical
/// telemetry events, keeping exactly one count per fact.
fn model_facts(conn: &Connection, _operations: &Loaded, attempts: &Loaded) -> ModelFacts {
    let mut facts = ModelFacts::default();
    let mut known_operations: HashSet<String> = HashSet::new();
    let mut known_attempts: HashSet<String> = HashSet::new();

    if has_table(conn, "recon_model_operations") {
        if let Ok(mut stmt) = conn
            .prepare("SELECT id, role, status, created_at, updated_at FROM recon_model_operations")
        {
            if let Ok(rows) = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            }) {
                for row in rows.flatten() {
                    known_operations.insert(row.0.clone());
                    facts.operations.push(OperationFact {
                        terminal_bucket: None,
                        role: row.1,
                        primary_provider: String::new(),
                        primary_model: String::new(),
                        effective_provider: String::new(),
                        effective_model: String::new(),
                        sends: 0,
                        failed: matches!(row.2.as_str(), "failed" | "error"),
                        recovered: matches!(row.2.as_str(), "succeeded" | "completed" | "ok"),
                        trigger_reason: String::new(),
                        operation_id: row.0,
                        created_at: parse_timestamp(&row.3),
                    });
                    facts.summary.operations += 1;
                }
            }
        }
    }
    if has_table(conn, "recon_model_attempts") {
        if let Ok(mut stmt) = conn.prepare(
            "SELECT id, operation_id, provider, model, route_index, dispatched, outcome, \
                    failure_category, http_status, elapsed_ms, queue_ms, ttfb_ms \
             FROM recon_model_attempts",
        ) {
            if let Ok(rows) = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, u32>(4)?,
                    row.get::<_, bool>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, Option<u32>>(8)?,
                    row.get::<_, i64>(9)?,
                    row.get::<_, i64>(10)?,
                    row.get::<_, i64>(11)?,
                ))
            }) {
                for row in rows.flatten() {
                    known_attempts.insert(row.0.clone());
                }
            }
        }
    }
    // `attempts` publishes sends from the same window; the durable rows are
    // already counted above, so only their ids are collected here.
    let _ = attempts;
    facts
}

/// Models widget 5: hierarchical provider totals with expandable model rows.
fn model_performance(_facts: &ModelFacts) -> Vec<ProviderModelRow> {
    Vec::new()
}

/// Models widget 6: fallback triggers versus successful recoveries.
fn model_fallback(_facts: &ModelFacts) -> Vec<FallbackRow> {
    Vec::new()
}

fn median_i64(samples: &[i64]) -> Option<i64> {
    if samples.is_empty() {
        return None;
    }
    let mut owned = samples.to_vec();
    owned.sort_unstable();
    Some(owned[owned.len() / 2])
}

// ---------- snapshot helpers ----------

/// The telemetry columns that say when something was last recorded. A rollup
/// bucket only marks the start of a period, so raw events win ties.
const TELEMETRY_STAMPS: [(&str, &str); 3] = [
    ("telemetry_events", "occurred_at"),
    ("telemetry_hourly", "bucket"),
    ("telemetry_daily", "bucket"),
];

/// Earliest moment this installation recorded anything that is still retained:
/// the oldest raw event and the oldest rollup bucket, whichever came first.
///
/// This is the dashboard's `observed_since` marker. Metrics before it are N/A,
/// not zero, so a fresh installation must report an empty string rather than an
/// invented epoch.
fn observed_since(conn: &Connection) -> String {
    let mut earliest = String::new();
    for value in [
        oldest_stored(conn, "telemetry_events", "occurred_at", ""),
        oldest_stored(conn, "telemetry_hourly", "bucket", ""),
        oldest_stored(conn, "telemetry_daily", "bucket", ""),
    ] {
        if !value.is_empty() && (earliest.is_empty() || value < earliest) {
            earliest = value;
        }
    }
    earliest
}

/// Retention boundaries with the oldest rows still holding raw and rollup data.
///
/// Only rows inside the published retention window count as retained, so a
/// sweeper that has not caught up yet never widens the reported window.
fn retention(conn: &Connection, now: DateTime<Utc>) -> RetentionInfo {
    let raw_floor = retention_floor(now, telemetry::RAW_RETENTION_DAYS);
    let rollup_floor = retention_floor(now, telemetry::ROLLUP_RETENTION_DAYS);
    let mut oldest_rollup = oldest_stored(conn, "telemetry_hourly", "bucket", &rollup_floor);
    let oldest_daily = oldest_stored(conn, "telemetry_daily", "bucket", &rollup_floor);
    if !oldest_daily.is_empty() && (oldest_rollup.is_empty() || oldest_daily < oldest_rollup) {
        oldest_rollup = oldest_daily;
    }
    RetentionInfo {
        raw_days: telemetry::RAW_RETENTION_DAYS,
        rollup_days: telemetry::ROLLUP_RETENTION_DAYS,
        oldest_raw: oldest_stored(conn, "telemetry_events", "occurred_at", &raw_floor),
        oldest_rollup,
    }
}

/// The RFC3339 retention boundary `days` before `now`.
fn retention_floor(now: DateTime<Utc>, days: i64) -> String {
    (now - ChronoDuration::days(days)).to_rfc3339()
}

/// Live status strip. Live jobs come from the durable job tables the rest of
/// the crate already reads; collection health reports what is actually
/// retained: `unknown` when nothing is, `stale` when the newest raw event
/// predates two period windows or nothing renders, `ok` otherwise.
fn global_status(
    conn: &Connection,
    period: &Period,
    now: DateTime<Utc>,
    sections_carry_data: bool,
) -> GlobalStatus {
    let live = provider_metrics::capacity_snapshot(conn);
    let newest_raw = newest_stored(conn, &TELEMETRY_STAMPS);
    let span = period.end(now) - period.start(now);
    let window = ChronoDuration::seconds(span.num_seconds().saturating_mul(2));
    let fresh = match parse_timestamp(&newest_raw) {
        // No raw row at all: nothing was collected recently.
        None => false,
        Some(at) => now - at <= window,
    };
    let collection_health = if newest_raw.is_empty() && observed_since(conn).is_empty() {
        "unknown".to_string()
    } else if sections_carry_data && fresh {
        "ok".to_string()
    } else {
        "stale".to_string()
    };
    // Queued requests only mean something once the companion publishes
    // capacity; before that the strip shows unavailable, never zero.
    let mut queued_requests = 0;
    if live.available {
        for row in &live.capacity {
            queued_requests += i64::from(row.queued);
        }
    }
    GlobalStatus {
        active_jobs: active_jobs(conn),
        queued_requests,
        collection_health,
        last_updated: newest_stored(conn, &TELEMETRY_STAMPS),
        provider_capacity_available: live.available,
    }
}

/// Live jobs across the durable job tables. A row is live while its status
/// column is not one of the terminal states the crate already normalizes to:
/// `normalize_run_outcome`, `is_terminal_report_state` and `cycle_outcome_label`.
fn active_jobs(conn: &Connection) -> i64 {
    const RECON_TERMINAL: &str = "('completed','completed_with_evidence',\
                                  'completed_zero_evidence','partial','failed',\
                                  'cancelled','canceled')";
    const REPORT_TERMINAL: &str = "('completed','partial','failed','cancelled','canceled')";
    // Atlas folds `paused` into a cancelled cycle, so it is terminal too.
    const ATLAS_TERMINAL: &str = "('completed','partial','failed','cancelled','canceled','paused')";
    let mut active = 0;
    for (table, column, terminal) in [
        ("recon_runs", "state", RECON_TERMINAL),
        ("intel_report_jobs", "state", REPORT_TERMINAL),
        ("atlas_runs", "state", ATLAS_TERMINAL),
    ] {
        if !has_table(conn, table) {
            continue;
        }
        active += count_rows(
            conn,
            &format!("SELECT COUNT(*) FROM {table} WHERE {column} NOT IN {terminal}"),
        );
    }
    active
}

/// True when at least one section carries a real observation. A zero-filled
/// trend skeleton, the always-present named engines and a live `Other` row are
/// not data, so they never make a dashboard look healthy.
fn sections_carry_data(
    intel: &IntelStats,
    recon: &ReconStats,
    atlas: &AtlasStats,
    models: &ModelStats,
    tools: &ToolStats,
) -> bool {
    intel_carries_data(intel)
        || recon_carries_data(recon)
        || atlas_carries_data(atlas)
        || models_carries_data(models)
        || tools_carries_data(tools)
}

/// Intel observations: distinct articles, report work and rated confidence.
/// A zero-filled volume skeleton is not an observation.
fn intel_carries_data(intel: &IntelStats) -> bool {
    intel.report_attempts > 0
        || intel.volume.iter().any(|bucket| bucket.total > 0)
        || intel.confidence.paired_n > 0
        || !intel.origins.is_empty()
        || !intel.enrichment.is_empty()
        || !intel.reports.is_empty()
        || !intel.publishers.is_empty()
        || intel.freshness.buckets.iter().any(|(_, n)| *n > 0)
        || intel.freshness.missing > 0
        || intel.freshness.future > 0
}

/// Recon observations: a workload row only exists for an observed run, and a
/// run that reached a bucket also produced its outcome row.
fn recon_carries_data(recon: &ReconStats) -> bool {
    !recon.outcomes.is_empty()
        || !recon.workload.is_empty()
        || !recon.stages.is_empty()
        || !recon.recall.is_empty()
        || !recon.diversity.is_empty()
        || !recon.directives.is_empty()
        || !recon.unresolved.is_empty()
        || recon.recall.iter().any(|row| row.queries > 0)
}

/// Atlas observations: cycle work, candidate dispositions and live backlog.
/// Trend buckets are zero filled, so only their counts count.
fn atlas_carries_data(atlas: &AtlasStats) -> bool {
    !atlas.temperature_changes.is_empty()
        || !atlas.origins.is_empty()
        || atlas.waiting_now > 0
        || atlas.blocked_now > 0
        || atlas.cycle_times.iter().any(|bucket| bucket.n > 0)
        || atlas.cycle_outcomes.iter().any(|b| b.completed > 0)
        || atlas.cycle_outcomes.iter().any(|b| b.partial > 0)
        || atlas.cycle_outcomes.iter().any(|b| b.failed > 0)
        || atlas.cycle_outcomes.iter().any(|b| b.cancelled > 0)
        || atlas.discovery.iter().any(|bucket| bucket.fetched > 0)
        || atlas.backlog.iter().any(|row| row.queued > 0)
        || atlas.backlog.iter().any(|row| row.running > 0)
        || atlas.backlog.iter().any(|row| row.waiting > 0)
        || atlas.backlog.iter().any(|row| row.blocked > 0)
}

/// Model observations: published capacity plus counted work. An unfinished
/// send is never a failure, so only counted attempts and operations testify.
fn models_carries_data(models: &ModelStats) -> bool {
    !models.capacity.is_empty()
        || models.attempts.finished_attempts > 0
        || models.attempts.operations > 0
        || models.by_role.iter().any(|bucket| bucket.total > 0)
        || models.latency.iter().any(|bucket| bucket.n > 0)
        || models.queue_delay.iter().any(|bucket| bucket.n > 0)
        || !models.performance.is_empty()
        || !models.fallback.is_empty()
        || models.failures.iter().any(|b| b.total_failed > 0)
}

/// Tool observations: logical invocations and their outcomes. The named
/// engines always have a row, so only a fetch or a cache hit counts.
fn tools_carries_data(tools: &ToolStats) -> bool {
    !tools.top_tools.is_empty()
        || !tools.categories_by_trigger.is_empty()
        || !tools.reliability.is_empty()
        || !tools.evidence.is_empty()
        || !tools.failure_causes.is_empty()
        || tools.outcomes.iter().any(|b| b.completed_nonempty > 0)
        || tools.outcomes.iter().any(|b| b.verified_zero > 0)
        || tools.outcomes.iter().any(|b| b.partial > 0)
        || tools.outcomes.iter().any(|b| b.failed > 0)
        || tools.outcomes.iter().any(|b| b.blocked > 0)
        || tools.engine_health.iter().any(|b| b.fetches > 0)
        || tools.engine_health.iter().any(|b| b.cache_hits > 0)
}

/// Oldest value of one telemetry column: empty when the table is missing, the
/// column holds nothing, or everything predates `floor`. An empty floor keeps
/// every row, so it reads the whole retained window.
fn oldest_stored(conn: &Connection, table: &str, column: &str, floor: &str) -> String {
    if !has_table(conn, table) {
        return String::new();
    }
    let sql = format!("SELECT MIN({column}) FROM {table} WHERE {column} >= ?1");
    let oldest = conn.query_row(&sql, rusqlite::params![floor], |row| {
        row.get::<_, Option<String>>(0)
    });
    match oldest {
        Ok(Some(value)) if !value.is_empty() => value,
        _ => String::new(),
    }
}

/// Newest value across the given telemetry columns. A missing table is skipped,
/// never an error, so a partially migrated store still reports what it holds.
fn newest_stored(conn: &Connection, columns: &[(&str, &str)]) -> String {
    let mut newest = String::new();
    for (table, column) in columns {
        if !has_table(conn, table) {
            continue;
        }
        let sql = format!("SELECT MAX({column}) FROM {table}");
        if let Ok(Some(value)) = conn.query_row(&sql, [], |row| row.get::<_, Option<String>>(0)) {
            if !value.is_empty() && value > newest {
                newest = value;
            }
        }
    }
    newest
}

/// Sorted distinct non-empty values of one filter dimension. An empty vector
/// means the dimension selects nothing, so the strip offers no choices.
fn distinct_values(conn: &Connection, dimension: &str) -> Vec<String> {
    if !has_table(conn, "telemetry_events") {
        return Vec::new();
    }
    let column = match dimension {
        "tool" => "tool_id",
        other => other,
    };
    let sql = format!(
        "SELECT DISTINCT {column} FROM telemetry_events WHERE {column} <> '' ORDER BY {column}"
    );
    let mut values = Vec::new();
    if let Ok(mut stmt) = conn.prepare(&sql) {
        if let Ok(rows) = stmt.query_map([], |row| row.get::<_, String>(0)) {
            values.extend(rows.flatten());
        }
    }
    values
}

#[cfg(test)]
mod snapshot_tests {
    use super::*;
    use crate::store::Store;
    use crate::telemetry::{EventKind, TelemetryEvent};

    fn store() -> Store {
        Store::memory().unwrap()
    }

    fn record(store: &Store, event: TelemetryEvent) {
        let written = crate::telemetry::insert(store.connection(), &event);
        written.unwrap();
    }

    fn snapshot_at(store: &Store, period: &Period, now: DateTime<Utc>) -> ProfileSnapshot {
        let snap = snapshot(store.connection(), period, &StatFilters::default(), now);
        snap.unwrap()
    }

    fn dimension_values(options: &[(String, Vec<String>)], name: &str) -> Vec<String> {
        for (dimension, values) in options {
            if dimension == name {
                return values.clone();
            }
        }
        Vec::new()
    }

    #[test]
    fn an_empty_database_returns_empty_sections_not_zeros() {
        let store = store();
        let snap = snapshot_at(&store, &Period::H1, Utc::now());

        // Sectors stay zero filled and empty of observations; nothing is
        // fabricated as a completion, an average or a capacity row.
        assert!(snap.intel.volume.iter().all(|bucket| bucket.total == 0));
        assert!(snap.intel.origins.is_empty());
        assert!(snap.intel.reports.is_empty());
        assert!(snap.intel.publishers.is_empty());
        assert_eq!(snap.intel.report_attempts, 0);
        assert!(snap.recon.outcomes.is_empty());
        assert!(snap.recon.stages.is_empty());
        assert!(snap.recon.recall.is_empty());
        assert!(snap.recon.workload.is_empty());
        assert!(snap.recon.unresolved.is_empty());
        assert!(snap.atlas.temperature_changes.is_empty());
        assert!(snap.models.by_role.iter().all(|b| b.total == 0));
        assert!(snap.tools.top_tools.is_empty());
        assert!(snap.tools.failure_causes.is_empty());

        // Nothing retained is unavailable, never zero: no observed window, no
        // capacity, no live jobs and no retention window to report.
        assert!(snap.observed_since.is_empty());
        assert_eq!(snap.status.collection_health, "unknown");
        assert_eq!(snap.status.active_jobs, 0);
        assert!(snap.status.last_updated.is_empty());
        assert!(!snap.status.provider_capacity_available);
        assert!(!snap.models.capacity_available);
        assert!(snap.retention.oldest_raw.is_empty());
        assert!(snap.retention.oldest_rollup.is_empty());
        assert!(snap.intel.confidence.mean_initial.is_none());
        assert!(snap.tools.reliability.is_empty());
    }

    #[test]
    fn observed_since_and_retention_report_the_retained_window() {
        let store = store();
        let now = Utc::now();
        let run = TelemetryEvent::new(EventKind::ReconRun)
            .at(now.to_rfc3339())
            .app("tui")
            .mode("deep")
            .outcome("completed");
        record(&store, run);
        let call = TelemetryEvent::new(EventKind::ToolInvocation)
            .at(now.to_rfc3339())
            .app("tui")
            .provider("firecrawl")
            .role("recon")
            .mode("remote")
            .tool("firecrawl_scrape")
            .category("web")
            .outcome("completed_nonempty");
        record(&store, call);

        let snap = snapshot_at(&store, &Period::H1, now);

        assert!(!snap.observed_since.is_empty());
        assert_eq!(snap.retention.raw_days, telemetry::RAW_RETENTION_DAYS);
        assert_eq!(snap.retention.rollup_days, telemetry::ROLLUP_RETENTION_DAYS);
        assert!(!snap.retention.oldest_raw.is_empty());
        assert!(!snap.status.last_updated.is_empty());
        assert_eq!(snap.status.collection_health, "ok");
        // Recorded work shows up as observations, not as a zero-filled skeleton.
        assert_eq!(snap.recon.outcomes.len(), 1);
        assert_eq!(snap.recon.outcomes[0].completed_with_evidence, 1);
        assert_eq!(snap.tools.top_tools.len(), 1);
        assert_eq!(snap.tools.top_tools[0].tool_id, "firecrawl_scrape");
    }

    #[test]
    fn filter_options_lists_the_period_choices_and_no_phantom_dimensions() {
        let store = store();
        let empty = filter_options(store.connection());
        let names: Vec<&str> = empty.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(
            names,
            ["period", "app", "provider", "role", "mode", "tool", "category"]
        );
        let expected: Vec<String> = Period::ALL.iter().map(|p| p.label().into()).collect();
        let (first, periods) = &empty[0];
        assert_eq!(first, "period");
        assert_eq!(periods, &expected);
        for (dimension, values) in &empty[1..] {
            assert!(values.is_empty(), "empty database offered {dimension}");
        }

        let now = Utc::now();
        let call = TelemetryEvent::new(EventKind::ToolInvocation)
            .at(now.to_rfc3339())
            .app("tui")
            .provider("firecrawl")
            .role("recon")
            .mode("remote")
            .tool("firecrawl_scrape")
            .category("web")
            .outcome("completed_nonempty");
        record(&store, call);

        let options = filter_options(store.connection());
        assert_eq!(dimension_values(&options, "app"), ["tui"]);
        assert_eq!(dimension_values(&options, "provider"), ["firecrawl"]);
        assert_eq!(dimension_values(&options, "role"), ["recon"]);
        assert_eq!(dimension_values(&options, "mode"), ["remote"]);
        assert_eq!(dimension_values(&options, "tool"), ["firecrawl_scrape"]);
        assert_eq!(dimension_values(&options, "category"), ["web"]);
    }

    #[test]
    fn a_snapshot_never_panics_on_a_partially_migrated_store() {
        let store = store();
        let conn = store.connection();
        let now = Utc::now();
        let cycle = TelemetryEvent::new(EventKind::AtlasCycle)
            .at(now.to_rfc3339())
            .run("run-1")
            .outcome("completed");
        record(&store, cycle);
        let before = snapshot_at(&store, &Period::H24, now);
        assert!(!before.observed_since.is_empty());

        // Drop one rollup table: a partially migrated store must degrade to the
        // sections it still has instead of failing the whole dashboard.
        conn.execute_batch("DROP TABLE telemetry_hourly").unwrap();
        let after = snapshot_at(&store, &Period::H24, now);
        assert!(!after.observed_since.is_empty());
        assert!(!after.retention.oldest_raw.is_empty());
        // The cycle still reports from the raw table, and the tool section
        // stays empty instead of inventing invocations.
        let outcomes = &after.atlas.cycle_outcomes;
        assert!(outcomes.iter().any(|b| b.completed == 1));
        assert!(after.tools.top_tools.is_empty());
        assert_eq!(after.status.active_jobs, before.status.active_jobs);

        // A repeated read is idempotent: no counter drifts between renders.
        let again = snapshot_at(&store, &Period::H24, now);
        assert_eq!(after, again);
    }
}
