//! Profile dashboard read model on SurrealDB (`profile_stats` parity).
//!
//! [`snapshot`] loads the Profile input rows into
//! [`ProfileTables`](crate::profile_stats::ProfileTables) and runs the same
//! aggregation as SQLite, so all 20 views (Intel 4, Recon 5, Atlas 3,
//! Models 4, Tools 4), the KPIs, Needs attention, retention and the status
//! strip come out identical for identical rows.
//!
//! Tables read here: `telemetry_event`, `telemetry_hourly`,
//! `telemetry_daily`, `telemetry_provider_capacity` (defined in [`SCHEMA`]),
//! `article_body`, `atlas_run`, `atlas_article`, `atlas_article_metadata`
//! and `task` (`atlas_related`). Recon runs, Recon coverage, Intel report
//! jobs/tasks and Recon model operations/attempts are not on SurrealDB yet;
//! they load as missing tables, which the views render as unavailable.

use anyhow::Result;
use chrono::{DateTime, Utc};
use surrealdb::engine::any::Any;
use surrealdb::types::SurrealValue;
use surrealdb::Surreal;

use crate::profile_stats::{
    self, ArticleBodyRow, AtlasMetadataRow, Period, ProfileSnapshot, ProfileTables, RelatedTaskRow,
    RollupRow, StatFilters, TelemetryRow,
};
use crate::provider_metrics::StoredCapacityRow;

/// Telemetry schema (idempotent): the SurrealDB side of `telemetry_events`,
/// `telemetry_hourly`, `telemetry_daily` and `telemetry_provider_capacity`.
/// Raw events are keyed by event id; rollups by `[bucket, event_type,
/// dim_key]`; capacity rows by `[provider, quota_group, scope, captured_at]`.
pub const SCHEMA: &str = "DEFINE TABLE IF NOT EXISTS telemetry_event SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS event_type ON telemetry_event TYPE string;
DEFINE FIELD IF NOT EXISTS occurred_at ON telemetry_event TYPE string;
DEFINE FIELD IF NOT EXISTS day ON telemetry_event TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS hour ON telemetry_event TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS app ON telemetry_event TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS trigger ON telemetry_event TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS tool_id ON telemetry_event TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS category ON telemetry_event TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS provider ON telemetry_event TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS role ON telemetry_event TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS model ON telemetry_event TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS mode ON telemetry_event TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS engine ON telemetry_event TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS outcome ON telemetry_event TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS reason ON telemetry_event TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS job_id ON telemetry_event TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS run_id ON telemetry_event TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS thread_id ON telemetry_event TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS turn_id ON telemetry_event TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS article_id ON telemetry_event TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS origin ON telemetry_event TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS call_id ON telemetry_event TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS canonical_ref ON telemetry_event TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS metric_ms ON telemetry_event TYPE option<int>;
DEFINE FIELD IF NOT EXISTS count ON telemetry_event TYPE int DEFAULT 1;
DEFINE FIELD IF NOT EXISTS payload_json ON telemetry_event TYPE string DEFAULT '{}';
DEFINE INDEX IF NOT EXISTS telemetry_event_type_time ON telemetry_event FIELDS event_type, occurred_at;
DEFINE INDEX IF NOT EXISTS telemetry_event_time ON telemetry_event FIELDS occurred_at;
DEFINE INDEX IF NOT EXISTS telemetry_event_canonical ON telemetry_event FIELDS canonical_ref;
DEFINE TABLE IF NOT EXISTS telemetry_hourly SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS bucket ON telemetry_hourly TYPE string;
DEFINE FIELD IF NOT EXISTS event_type ON telemetry_hourly TYPE string;
DEFINE FIELD IF NOT EXISTS dim_key ON telemetry_hourly TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS events ON telemetry_hourly TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS total_ms ON telemetry_hourly TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS max_ms ON telemetry_hourly TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS bins_json ON telemetry_hourly TYPE string DEFAULT '[]';
DEFINE INDEX IF NOT EXISTS telemetry_hourly_type ON telemetry_hourly FIELDS event_type, bucket;
DEFINE TABLE IF NOT EXISTS telemetry_daily SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS bucket ON telemetry_daily TYPE string;
DEFINE FIELD IF NOT EXISTS event_type ON telemetry_daily TYPE string;
DEFINE FIELD IF NOT EXISTS dim_key ON telemetry_daily TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS events ON telemetry_daily TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS total_ms ON telemetry_daily TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS max_ms ON telemetry_daily TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS bins_json ON telemetry_daily TYPE string DEFAULT '[]';
DEFINE INDEX IF NOT EXISTS telemetry_daily_type ON telemetry_daily FIELDS event_type, bucket;
DEFINE TABLE IF NOT EXISTS telemetry_provider_capacity SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS captured_at ON telemetry_provider_capacity TYPE string;
DEFINE FIELD IF NOT EXISTS provider ON telemetry_provider_capacity TYPE string;
DEFINE FIELD IF NOT EXISTS quota_group ON telemetry_provider_capacity TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS scope ON telemetry_provider_capacity TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS sends_60s ON telemetry_provider_capacity TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS rpm_limit ON telemetry_provider_capacity TYPE option<int>;
DEFINE FIELD IF NOT EXISTS pace_per_min ON telemetry_provider_capacity TYPE option<float>;
DEFINE FIELD IF NOT EXISTS active ON telemetry_provider_capacity TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS max_concurrency ON telemetry_provider_capacity TYPE option<int>;
DEFINE FIELD IF NOT EXISTS queued ON telemetry_provider_capacity TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS oldest_wait_ms ON telemetry_provider_capacity TYPE option<int>;
DEFINE FIELD IF NOT EXISTS cooldown_ms ON telemetry_provider_capacity TYPE option<int>;
DEFINE FIELD IF NOT EXISTS quota_source ON telemetry_provider_capacity TYPE string DEFAULT '';";

#[derive(SurrealValue)]
struct Event {
    key: String,
    event_type: String,
    occurred_at: String,
    app: String,
    trigger: String,
    tool_id: String,
    category: String,
    provider: String,
    role: String,
    model: String,
    mode: String,
    engine: String,
    outcome: String,
    reason: String,
    job_id: String,
    run_id: String,
    article_id: String,
    origin: String,
    call_id: String,
    canonical_ref: String,
    metric_ms: Option<i64>,
    count: i64,
    payload_json: String,
}

#[derive(SurrealValue)]
struct Rollup {
    bucket: String,
    event_type: String,
    dim_key: String,
    events: i64,
    total_ms: i64,
    max_ms: i64,
    bins_json: String,
}

#[derive(SurrealValue)]
struct Capacity {
    captured_at: String,
    provider: String,
    quota_group: String,
    scope: String,
    sends_60s: i64,
    rpm_limit: Option<i64>,
    pace_per_min: Option<f64>,
    active: i64,
    max_concurrency: Option<i64>,
    queued: i64,
    oldest_wait_ms: Option<i64>,
    cooldown_ms: Option<i64>,
    quota_source: String,
}

#[derive(SurrealValue)]
struct Body {
    article_id: String,
    quality: String,
    has_body: bool,
}

#[derive(SurrealValue)]
struct Run {
    key: String,
    state: String,
    stats_json: String,
}

#[derive(SurrealValue)]
struct Article {
    run_id: String,
    article_id: String,
    country: String,
}

#[derive(SurrealValue)]
struct Metadata {
    run_id: String,
    article_id: String,
    classification_status: String,
    entity_state: String,
    reliability_state: String,
}

#[derive(SurrealValue)]
struct Related {
    state: String,
    created_at: String,
    error_category: String,
}

const LOAD: &str = "SELECT record::id(id) AS key, event_type, occurred_at, app, trigger, tool_id,
    category, provider, role, model, mode, engine, outcome, reason, job_id, run_id, article_id,
    origin, call_id, canonical_ref, metric_ms, count, payload_json FROM telemetry_event;
SELECT bucket, event_type, dim_key, events, total_ms, max_ms, bins_json FROM telemetry_hourly;
SELECT bucket, event_type, dim_key, events, total_ms, max_ms, bins_json FROM telemetry_daily;
SELECT captured_at, provider, quota_group, scope, sends_60s, rpm_limit, pace_per_min, active,
    max_concurrency, queued, oldest_wait_ms, cooldown_ms, quota_source
    FROM telemetry_provider_capacity;
SELECT article_id, quality, string::len(string::trim(body_markdown)) > 0 AS has_body
    FROM article_body;
SELECT record::id(id) AS key, state, stats_json FROM atlas_run;
SELECT run_id, article_id, country FROM atlas_article;
SELECT run_id, article_id, classification_status, entity_state, reliability_state
    FROM atlas_article_metadata;
SELECT state, <string>created_at AS created_at, error_category FROM task
    WHERE operation = 'atlas_related' AND state IN ['queued', 'running', 'waiting', 'blocked'];";

fn u32_of(value: i64) -> u32 {
    u32::try_from(value.max(0)).unwrap_or(u32::MAX)
}

/// Every Profile input row on SurrealDB, read in one request.
pub async fn load_tables(db: &Surreal<Any>) -> Result<ProfileTables> {
    let mut r = db.query(LOAD).await?.check()?;
    let events: Vec<Event> = r.take(0)?;
    let hourly: Vec<Rollup> = r.take(1)?;
    let daily: Vec<Rollup> = r.take(2)?;
    let capacity: Vec<Capacity> = r.take(3)?;
    let bodies: Vec<Body> = r.take(4)?;
    let runs: Vec<Run> = r.take(5)?;
    let articles: Vec<Article> = r.take(6)?;
    let metadata: Vec<Metadata> = r.take(7)?;
    let related: Vec<Related> = r.take(8)?;

    let rollups = |rows: Vec<Rollup>| -> Vec<RollupRow> {
        rows.into_iter()
            .map(|r| RollupRow {
                bucket: r.bucket,
                event_type: r.event_type,
                dim_key: r.dim_key,
                events: r.events,
                total_ms: r.total_ms,
                max_ms: r.max_ms,
                bins_json: r.bins_json,
            })
            .collect()
    };
    // atlas_articles JOIN atlas_runs LEFT JOIN atlas_article_metadata.
    let run_by_id: std::collections::HashMap<&str, &Run> =
        runs.iter().map(|run| (run.key.as_str(), run)).collect();
    let meta_by_key: std::collections::HashMap<(&str, &str), &Metadata> = metadata
        .iter()
        .map(|m| ((m.run_id.as_str(), m.article_id.as_str()), m))
        .collect();
    let atlas_metadata: Vec<AtlasMetadataRow> = articles
        .iter()
        .filter_map(|article| {
            let run = run_by_id.get(article.run_id.as_str())?;
            let meta = meta_by_key.get(&(article.run_id.as_str(), article.article_id.as_str()));
            let state = |pick: fn(&Metadata) -> &str| {
                meta.map_or_else(|| "pending".to_string(), |m| pick(m).to_string())
            };
            Some(AtlasMetadataRow {
                run_state: run.state.clone(),
                stats_json: run.stats_json.clone(),
                classification: state(|m| &m.classification_status),
                entities: state(|m| &m.entity_state),
                reliability: state(|m| &m.reliability_state),
            })
        })
        .collect();

    let recon = super::recon_jobs::profile_rows(db).await?;
    Ok(ProfileTables {
        telemetry_events: Some(
            events
                .into_iter()
                .map(|e| TelemetryRow {
                    id: e.key,
                    event_type: e.event_type,
                    occurred_at: e.occurred_at,
                    app: e.app,
                    trigger: e.trigger,
                    tool_id: e.tool_id,
                    category: e.category,
                    provider: e.provider,
                    role: e.role,
                    model: e.model,
                    mode: e.mode,
                    engine: e.engine,
                    outcome: e.outcome,
                    reason: e.reason,
                    job_id: e.job_id,
                    run_id: e.run_id,
                    article_id: e.article_id,
                    origin: e.origin,
                    call_id: e.call_id,
                    canonical_ref: e.canonical_ref,
                    metric_ms: e.metric_ms,
                    count: e.count,
                    payload_json: e.payload_json,
                })
                .collect(),
        ),
        telemetry_hourly: Some(rollups(hourly)),
        telemetry_daily: Some(rollups(daily)),
        provider_capacity: Some(
            capacity
                .into_iter()
                .map(|c| StoredCapacityRow {
                    captured_at: c.captured_at,
                    provider: c.provider,
                    quota_group: c.quota_group,
                    scope: c.scope,
                    sends_60s: u32_of(c.sends_60s),
                    rpm_limit: c.rpm_limit.map(u32_of),
                    pace_per_min: c.pace_per_min,
                    active: u32_of(c.active),
                    max_concurrency: c.max_concurrency.map(u32_of),
                    queued: u32_of(c.queued),
                    oldest_wait_ms: c.oldest_wait_ms.map(|v| v.max(0) as u64),
                    cooldown_ms: c.cooldown_ms.map(|v| v.max(0) as u64),
                    quota_source: c.quota_source,
                })
                .collect(),
        ),
        article_bodies: Some(
            bodies
                .into_iter()
                .map(|b| ArticleBodyRow {
                    article_id: b.article_id,
                    quality: b.quality,
                    has_body: b.has_body,
                })
                .collect(),
        ),
        atlas_run_states: Some(runs.iter().map(|run| run.state.clone()).collect()),
        atlas_articles: Some(
            articles
                .into_iter()
                .map(|a| (a.country, a.article_id))
                .collect(),
        ),
        atlas_metadata: Some(atlas_metadata),
        related_tasks: Some(
            related
                .into_iter()
                .map(|t| RelatedTaskRow {
                    state: t.state,
                    created_at: t.created_at,
                    error_category: t.error_category,
                })
                .collect(),
        ),
        recon_run_states: Some(recon.recon_run_states),
        report_jobs: Some(recon.report_jobs),
        report_tasks: Some(recon.report_tasks),
        recon_coverage: Some(recon.recon_coverage),
        model_operations: Some(recon.model_operations),
        model_attempts: Some(recon.model_attempts),
    })
}

/// `Store::profile_snapshot` on SurrealDB: one dashboard snapshot at `now`.
pub async fn snapshot(
    db: &Surreal<Any>,
    period: &Period,
    filters: &StatFilters,
    now: DateTime<Utc>,
) -> Result<ProfileSnapshot> {
    let tables = load_tables(db).await?;
    profile_stats::snapshot_from(&tables, period, filters, now)
}

/// `Store::profile_filter_options` on SurrealDB.
pub async fn filter_options(db: &Surreal<Any>) -> Result<Vec<(String, Vec<String>)>> {
    let tables = load_tables(db).await?;
    Ok(profile_stats::filter_options_from(&tables))
}

/// A dashboard snapshot with the filter options for every dimension.
pub type Dashboard = (ProfileSnapshot, Vec<(String, Vec<String>)>);

/// The TUI dashboard read on SurrealDB: the snapshot and the filter options
/// from one load of the source tables.
pub async fn dashboard(
    db: &Surreal<Any>,
    period: &Period,
    filters: &StatFilters,
    now: DateTime<Utc>,
) -> Result<Dashboard> {
    let tables = load_tables(db).await?;
    let snapshot = profile_stats::snapshot_from(&tables, period, filters, now)?;
    Ok((snapshot, profile_stats::filter_options_from(&tables)))
}
