//! Where the Profile aggregation reads its rows from.
//!
//! [`ProfileSource`] is every read `profile_stats` performs, one method per
//! durable query. The SQLite [`Connection`] runs the original SQL; the
//! in-memory [`ProfileTables`] answers the same questions over rows another
//! store loaded (SurrealDB: `storage::profile_view`). `None` means the table
//! does not exist, which the aggregation renders as unavailable, never zero.

use std::collections::{BTreeMap, BTreeSet};

use crate::provider_metrics::{self, CapacitySnapshot, StoredCapacityRow};

/// One stored `telemetry_events` row.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TelemetryRow {
    pub id: String,
    pub event_type: String,
    pub occurred_at: String,
    pub app: String,
    pub trigger: String,
    pub tool_id: String,
    pub category: String,
    pub provider: String,
    pub role: String,
    pub model: String,
    pub mode: String,
    pub engine: String,
    pub outcome: String,
    pub reason: String,
    pub job_id: String,
    pub run_id: String,
    pub article_id: String,
    pub origin: String,
    pub call_id: String,
    pub canonical_ref: String,
    pub metric_ms: Option<i64>,
    pub count: i64,
    pub payload_json: String,
}

impl TelemetryRow {
    /// The value of one filterable column (`tool_id`, `app`, ...).
    pub fn column(&self, column: &str) -> &str {
        match column {
            "app" => &self.app,
            "trigger" => &self.trigger,
            "tool_id" => &self.tool_id,
            "category" => &self.category,
            "provider" => &self.provider,
            "role" => &self.role,
            "model" => &self.model,
            "mode" => &self.mode,
            "engine" => &self.engine,
            "outcome" => &self.outcome,
            "reason" => &self.reason,
            _ => "",
        }
    }
}

/// One `telemetry_hourly` / `telemetry_daily` bucket.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RollupRow {
    pub bucket: String,
    pub event_type: String,
    pub dim_key: String,
    pub events: i64,
    pub total_ms: i64,
    pub max_ms: i64,
    pub bins_json: String,
}

/// One Intel report job (`intel_report_jobs`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReportJobRow {
    pub id: String,
    pub article_id: String,
    pub state: String,
    pub stage: String,
    pub updated_at: String,
}

/// One Intel report task (`intel_report_tasks`); `article_id` is joined from
/// its job when read through [`ProfileSource::expired_report_tasks`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReportTaskRow {
    pub id: String,
    pub job_id: String,
    pub article_id: String,
    pub status: String,
    pub lease_until: String,
}

/// One `recon_coverage` row.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CoverageRow {
    pub scope: String,
    pub generation: i64,
    pub category: String,
    pub payload_json: String,
    pub updated_at: String,
}

/// One Atlas article joined to its run and (optional) metadata; missing
/// metadata states are `pending`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AtlasMetadataRow {
    pub run_state: String,
    pub stats_json: String,
    pub classification: String,
    pub entities: String,
    pub reliability: String,
}

/// One live `atlas_related` task.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RelatedTaskRow {
    pub state: String,
    pub created_at: String,
    pub error_category: String,
}

/// One `recon_model_operations` row.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModelOperationRow {
    pub id: String,
    pub role: String,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
}

/// One `recon_model_attempts` row with its operation's role.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModelAttemptRow {
    pub id: String,
    pub operation_id: String,
    pub provider: String,
    pub model: String,
    pub route_index: u32,
    pub dispatched: bool,
    pub outcome: String,
    pub failure_category: String,
    pub http_status: Option<u32>,
    pub elapsed_ms: Option<i64>,
    pub queue_ms: Option<i64>,
    pub ttfb_ms: Option<i64>,
    pub started_at: String,
    pub finished_at: String,
    pub sent_at: String,
    pub queued_at: String,
    pub first_response_at: String,
    pub role: String,
    pub transport: String,
}

/// Positional form the attempt reader consumes.
pub(crate) type AttemptTuple = (
    String,
    String,
    String,
    String,
    u32,
    bool,
    String,
    String,
    Option<u32>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    String,
    String,
    String,
    String,
    String,
    String,
    String,
);

impl ModelAttemptRow {
    pub(crate) fn into_tuple(self) -> AttemptTuple {
        (
            self.id,
            self.operation_id,
            self.provider,
            self.model,
            self.route_index,
            self.dispatched,
            self.outcome,
            self.failure_category,
            self.http_status,
            self.elapsed_ms,
            self.queue_ms,
            self.ttfb_ms,
            self.started_at,
            self.finished_at,
            self.sent_at,
            self.queued_at,
            self.first_response_at,
            self.role,
            self.transport,
        )
    }
}

/// One `article_bodies` row reduced to what Profile needs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ArticleBodyRow {
    pub article_id: String,
    pub quality: String,
    /// `length(trim(body_markdown)) > 0`.
    pub has_body: bool,
}

/// Every durable read the Profile aggregation performs.
pub trait ProfileSource {
    /// Raw events of one kind with `start <= occurred_at <= end` (RFC 3339
    /// text order), ordered by `occurred_at, id`.
    fn telemetry_events(
        &self,
        event_type: &str,
        start: &str,
        end: &str,
    ) -> Option<Vec<TelemetryRow>>;
    /// Rollup buckets of one kind from `telemetry_hourly` or `telemetry_daily`.
    fn telemetry_rollups(&self, table: &str, event_type: &str) -> Option<Vec<RollupRow>>;
    /// `MIN(column)` over values `>= floor` of a telemetry table, or empty.
    fn oldest_stored(&self, table: &str, column: &str, floor: &str) -> String;
    /// `MAX(column)` of a telemetry table; `None` when the table is missing.
    fn newest_stored(&self, table: &str, column: &str) -> Option<String>;
    /// Sorted distinct non-empty values of one telemetry column.
    fn distinct_values(&self, column: &str) -> Vec<String>;
    /// The cached provider capacity snapshot.
    fn capacity(&self) -> CapacitySnapshot;
    /// Live Recon runs, Intel report jobs and Atlas cycles.
    fn active_jobs(&self) -> i64;
    /// Blocked or waiting report jobs, oldest update first, at most 32.
    fn report_jobs_needing_attention(&self) -> Option<Vec<ReportJobRow>>;
    /// Running report tasks whose lease expired before `now`, at most 16.
    fn expired_report_tasks(&self, now: &str) -> Option<Vec<ReportTaskRow>>;
    fn has_article_bodies(&self) -> bool;
    /// A non-empty body whose quality is not `unavailable`.
    fn article_has_body(&self, article_id: &str) -> bool;
    fn article_has_report(&self, article_id: &str) -> bool;
    fn recon_coverage(&self) -> Option<Vec<CoverageRow>>;
    fn atlas_metadata(&self) -> Option<Vec<AtlasMetadataRow>>;
    /// Queued, running, waiting or blocked `atlas_related` tasks.
    fn atlas_related_tasks(&self) -> Option<Vec<RelatedTaskRow>>;
    /// Distinct Atlas articles per country, by country.
    fn atlas_articles_by_country(&self) -> Vec<(String, u32)>;
    /// Operations created at or before `end`.
    fn model_operations(&self, end: &str) -> Option<Vec<ModelOperationRow>>;
    /// Attempts started at or before `end` with a known operation, ordered by
    /// `started_at, id`.
    fn model_attempts(&self, end: &str) -> Option<Vec<ModelAttemptRow>>;
}

const RECON_TERMINAL: [&str; 7] = [
    "completed",
    "completed_with_evidence",
    "completed_zero_evidence",
    "partial",
    "failed",
    "cancelled",
    "canceled",
];
const REPORT_TERMINAL: [&str; 5] = ["completed", "partial", "failed", "cancelled", "canceled"];
// Atlas folds `paused` into a cancelled cycle, so it is terminal too.
const ATLAS_TERMINAL: [&str; 6] = [
    "completed",
    "partial",
    "failed",
    "cancelled",
    "canceled",
    "paused",
];
const LIVE_STATES: [&str; 4] = ["queued", "running", "waiting", "blocked"];

fn sql_list(values: &[&str]) -> String {
    let quoted: Vec<String> = values.iter().map(|v| format!("'{v}'")).collect();
    format!("({})", quoted.join(","))
}

/// Profile input rows held in memory. Each `None` is a table the store does
/// not have; the answers match the SQLite queries row for row.
#[derive(Clone, Debug, Default)]
pub struct ProfileTables {
    pub telemetry_events: Option<Vec<TelemetryRow>>,
    pub telemetry_hourly: Option<Vec<RollupRow>>,
    pub telemetry_daily: Option<Vec<RollupRow>>,
    pub provider_capacity: Option<Vec<StoredCapacityRow>>,
    /// `recon_runs.state` values.
    pub recon_run_states: Option<Vec<String>>,
    pub report_jobs: Option<Vec<ReportJobRow>>,
    pub report_tasks: Option<Vec<ReportTaskRow>>,
    pub article_bodies: Option<Vec<ArticleBodyRow>>,
    pub recon_coverage: Option<Vec<CoverageRow>>,
    /// `atlas_runs.state` values.
    pub atlas_run_states: Option<Vec<String>>,
    /// `(country, article_id)` per Atlas article row.
    pub atlas_articles: Option<Vec<(String, String)>>,
    pub atlas_metadata: Option<Vec<AtlasMetadataRow>>,
    pub related_tasks: Option<Vec<RelatedTaskRow>>,
    pub model_operations: Option<Vec<ModelOperationRow>>,
    /// Attempts; `role` is filled from the operation on read.
    pub model_attempts: Option<Vec<ModelAttemptRow>>,
}

impl ProfileTables {
    fn stamps(&self, table: &str, column: &str) -> Option<Vec<&str>> {
        match (table, column) {
            ("telemetry_events", "occurred_at") => self
                .telemetry_events
                .as_ref()
                .map(|rows| rows.iter().map(|r| r.occurred_at.as_str()).collect()),
            ("telemetry_hourly", "bucket") => self
                .telemetry_hourly
                .as_ref()
                .map(|rows| rows.iter().map(|r| r.bucket.as_str()).collect()),
            ("telemetry_daily", "bucket") => self
                .telemetry_daily
                .as_ref()
                .map(|rows| rows.iter().map(|r| r.bucket.as_str()).collect()),
            _ => None,
        }
    }
}

impl ProfileSource for ProfileTables {
    fn telemetry_events(
        &self,
        event_type: &str,
        start: &str,
        end: &str,
    ) -> Option<Vec<TelemetryRow>> {
        let mut out: Vec<TelemetryRow> = self
            .telemetry_events
            .as_ref()?
            .iter()
            .filter(|r| {
                r.event_type == event_type
                    && r.occurred_at.as_str() >= start
                    && r.occurred_at.as_str() <= end
            })
            .cloned()
            .collect();
        out.sort_by(|a, b| a.occurred_at.cmp(&b.occurred_at).then(a.id.cmp(&b.id)));
        Some(out)
    }

    fn telemetry_rollups(&self, table: &str, event_type: &str) -> Option<Vec<RollupRow>> {
        let rows = match table {
            "telemetry_hourly" => self.telemetry_hourly.as_ref()?,
            "telemetry_daily" => self.telemetry_daily.as_ref()?,
            _ => return None,
        };
        Some(
            rows.iter()
                .filter(|r| r.event_type == event_type)
                .cloned()
                .collect(),
        )
    }

    fn oldest_stored(&self, table: &str, column: &str, floor: &str) -> String {
        self.stamps(table, column)
            .and_then(|values| values.into_iter().filter(|v| *v >= floor).min())
            .unwrap_or_default()
            .to_string()
    }

    fn newest_stored(&self, table: &str, column: &str) -> Option<String> {
        self.stamps(table, column)?
            .into_iter()
            .max()
            .map(str::to_string)
    }

    fn distinct_values(&self, column: &str) -> Vec<String> {
        let Some(rows) = self.telemetry_events.as_ref() else {
            return Vec::new();
        };
        let values: BTreeSet<&str> = rows
            .iter()
            .map(|r| r.column(column))
            .filter(|v| !v.is_empty())
            .collect();
        values.into_iter().map(str::to_string).collect()
    }

    fn capacity(&self) -> CapacitySnapshot {
        let Some(rows) = self.provider_capacity.as_ref() else {
            return CapacitySnapshot::unavailable();
        };
        let mut rows = rows.clone();
        rows.sort_by(|a, b| {
            b.captured_at
                .cmp(&a.captured_at)
                .then(a.provider.cmp(&b.provider))
                .then(a.quota_group.cmp(&b.quota_group))
                .then(a.scope.cmp(&b.scope))
        });
        provider_metrics::capacity_from_rows(rows)
    }

    fn active_jobs(&self) -> i64 {
        let live = |states: Option<&Vec<String>>, terminal: &[&str]| {
            states.map_or(0, |states| {
                states
                    .iter()
                    .filter(|s| !terminal.contains(&s.as_str()))
                    .count() as i64
            })
        };
        let report_states: Option<Vec<String>> = self
            .report_jobs
            .as_ref()
            .map(|jobs| jobs.iter().map(|j| j.state.clone()).collect());
        live(self.recon_run_states.as_ref(), &RECON_TERMINAL)
            + live(report_states.as_ref(), &REPORT_TERMINAL)
            + live(self.atlas_run_states.as_ref(), &ATLAS_TERMINAL)
    }

    fn report_jobs_needing_attention(&self) -> Option<Vec<ReportJobRow>> {
        let mut out: Vec<ReportJobRow> = self
            .report_jobs
            .as_ref()?
            .iter()
            .filter(|j| matches!(j.state.as_str(), "blocked" | "waiting"))
            .cloned()
            .collect();
        out.sort_by(|a, b| a.updated_at.cmp(&b.updated_at).then(a.id.cmp(&b.id)));
        out.truncate(32);
        Some(out)
    }

    fn expired_report_tasks(&self, now: &str) -> Option<Vec<ReportTaskRow>> {
        let tasks = self.report_tasks.as_ref()?;
        let articles: BTreeMap<&str, &str> = self
            .report_jobs
            .as_ref()
            .map(|jobs| {
                jobs.iter()
                    .map(|j| (j.id.as_str(), j.article_id.as_str()))
                    .collect()
            })
            .unwrap_or_default();
        let mut out: Vec<ReportTaskRow> = tasks
            .iter()
            .filter(|t| {
                t.status == "running" && !t.lease_until.is_empty() && t.lease_until.as_str() < now
            })
            .filter_map(|t| {
                articles
                    .get(t.job_id.as_str())
                    .map(|article| ReportTaskRow {
                        article_id: (*article).to_string(),
                        ..t.clone()
                    })
            })
            .collect();
        out.sort_by(|a, b| a.lease_until.cmp(&b.lease_until).then(a.id.cmp(&b.id)));
        out.truncate(16);
        Some(out)
    }

    fn has_article_bodies(&self) -> bool {
        self.article_bodies.is_some()
    }

    fn article_has_body(&self, article_id: &str) -> bool {
        self.article_bodies.as_ref().is_some_and(|bodies| {
            bodies
                .iter()
                .any(|b| b.article_id == article_id && b.quality != "unavailable" && b.has_body)
        })
    }

    fn article_has_report(&self, article_id: &str) -> bool {
        self.report_jobs
            .as_ref()
            .is_some_and(|jobs| jobs.iter().any(|j| j.article_id == article_id))
    }

    fn recon_coverage(&self) -> Option<Vec<CoverageRow>> {
        self.recon_coverage.clone()
    }

    fn atlas_metadata(&self) -> Option<Vec<AtlasMetadataRow>> {
        self.atlas_metadata.clone()
    }

    fn atlas_related_tasks(&self) -> Option<Vec<RelatedTaskRow>> {
        Some(
            self.related_tasks
                .as_ref()?
                .iter()
                .filter(|t| LIVE_STATES.contains(&t.state.as_str()))
                .cloned()
                .collect(),
        )
    }

    fn atlas_articles_by_country(&self) -> Vec<(String, u32)> {
        let Some(rows) = self.atlas_articles.as_ref() else {
            return Vec::new();
        };
        let mut per: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        for (country, article) in rows {
            per.entry(country).or_default().insert(article);
        }
        per.into_iter()
            .map(|(country, articles)| (country.to_string(), articles.len() as u32))
            .collect()
    }

    fn model_operations(&self, end: &str) -> Option<Vec<ModelOperationRow>> {
        Some(
            self.model_operations
                .as_ref()?
                .iter()
                .filter(|o| o.created_at.as_str() <= end)
                .cloned()
                .collect(),
        )
    }

    fn model_attempts(&self, end: &str) -> Option<Vec<ModelAttemptRow>> {
        let attempts = self.model_attempts.as_ref()?;
        let roles: BTreeMap<&str, &str> = self
            .model_operations
            .as_ref()
            .map(|ops| {
                ops.iter()
                    .map(|o| (o.id.as_str(), o.role.as_str()))
                    .collect()
            })
            .unwrap_or_default();
        let mut out: Vec<ModelAttemptRow> = attempts
            .iter()
            .filter(|a| a.started_at.as_str() <= end)
            .filter_map(|a| {
                roles
                    .get(a.operation_id.as_str())
                    .map(|role| ModelAttemptRow {
                        role: (*role).to_string(),
                        ..a.clone()
                    })
            })
            .collect();
        out.sort_by(|a, b| a.started_at.cmp(&b.started_at).then(a.id.cmp(&b.id)));
        Some(out)
    }
}
