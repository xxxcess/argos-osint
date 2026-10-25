//! Durable event logs on SurrealDB: the `argos_events` sink
//! (`crate::events`) and the per-thread `investigation_events` trace
//! (`investigation::persist`), which the Recon pane reads.
//!
//! `argos_event` is keyed by the event id and ordered by `seq`, allocated in
//! the write's transaction from the `argos_event_seq` counter record
//! (`storage::counters::next_in`, the outbox's `index_change` seq
//! precedent): every writer increments that one record, so concurrent writers
//! serialize on it and the loser retries the whole transaction
//! (`counters::HOT_RECORD_POLICY`). The increment commits with the row, so a
//! rolled-back write consumes no value; the numbers a reader misses come only
//! from prune/clear deletions, which the old read-max allocator left
//! non-contiguous too. Gaps are allowed: every consumer orders by `seq`
//! (never by insert or commit order) and none requires contiguous values. On
//! first use per process the counter is seeded to the highest stored `seq`
//! (read with ORDER BY/LIMIT, because `math::max` of no rows is -Infinity on
//! a server). Messages and details are redacted and bounded
//! before they are stored, as in SQLite. `investigation_event` is keyed by
//! the event id; a rewrite of an existing id only updates its status, summary
//! and payload, like SQLite's `ON CONFLICT(id) DO UPDATE`.
//!
//! The Logs pane reads only this table under a storage config: the page
//! ([`list_events`], whose job filter covers the job and its descendant
//! jobs like SQLite's recursive `argos_jobs.parent_id` walk), one event
//! ([`get_event`]), the header counts ([`event_counts`]), the app filter
//! ([`event_apps`]), the retention prune ([`prune_events`]) and Clear log
//! ([`clear_events`]). Prune and clear touch only `argos_event`.

use std::sync::OnceLock;

use anyhow::Result;
use surrealdb::engine::any::Any;
use surrealdb::types::SurrealValue;
use surrealdb::Surreal;

use super::counters;
use super::jobs::{rid, run};
use super::tx::{run_tx, Tx, TxError};
use crate::events::{
    bound, new_event_id, redact, severity_rank, EventFilter, EventRow, NewEvent, Severity,
    MAX_DETAIL_CHARS, MAX_MESSAGE_CHARS,
};
use crate::investigation::{InvestigationEvent, InvestigationSurface};
use crate::jobs_view::EventCounts;

/// Deepest job nesting the Logs job filter follows (SQLite's recursive CTE
/// has no bound; real trees are two or three levels).
const MAX_JOB_DEPTH: usize = 16;

/// Event log schema (idempotent).
pub const SCHEMA: &str = "DEFINE TABLE IF NOT EXISTS argos_event SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS seq ON argos_event TYPE int;
DEFINE FIELD IF NOT EXISTS ts ON argos_event TYPE string;
DEFINE FIELD IF NOT EXISTS severity ON argos_event TYPE string;
DEFINE FIELD IF NOT EXISTS rank ON argos_event TYPE int DEFAULT 1;
DEFINE FIELD IF NOT EXISTS app ON argos_event TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS event_type ON argos_event TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS message ON argos_event TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS details ON argos_event TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS job_id ON argos_event TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS task_id ON argos_event TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS attempt_id ON argos_event TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS run_id ON argos_event TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS resource_ref ON argos_event TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS correlation_id ON argos_event TYPE string DEFAULT '';
DEFINE INDEX IF NOT EXISTS argos_event_seq ON argos_event FIELDS seq;
DEFINE INDEX IF NOT EXISTS argos_event_ts ON argos_event FIELDS ts;
DEFINE INDEX IF NOT EXISTS argos_event_job ON argos_event FIELDS job_id;

DEFINE TABLE IF NOT EXISTS investigation_event SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS investigation_id ON investigation_event TYPE string;
DEFINE FIELD IF NOT EXISTS sequence ON investigation_event TYPE int;
DEFINE FIELD IF NOT EXISTS occurrence_time ON investigation_event TYPE string;
DEFINE FIELD IF NOT EXISTS recording_time ON investigation_event TYPE string;
DEFINE FIELD IF NOT EXISTS surface ON investigation_event TYPE string;
DEFINE FIELD IF NOT EXISTS run_id ON investigation_event TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS directive_id ON investigation_event TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS task_id ON investigation_event TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS revision ON investigation_event TYPE int DEFAULT 1;
DEFINE FIELD IF NOT EXISTS parent_event_id ON investigation_event TYPE option<string>;
DEFINE FIELD IF NOT EXISTS role ON investigation_event TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS model ON investigation_event TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS provider ON investigation_event TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS attempt_id ON investigation_event TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS event_type ON investigation_event TYPE string;
DEFINE FIELD IF NOT EXISTS status ON investigation_event TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS summary ON investigation_event TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS payload_json ON investigation_event TYPE string DEFAULT '{}';
DEFINE FIELD IF NOT EXISTS evidence_refs_json ON investigation_event TYPE string DEFAULT '[]';
DEFINE FIELD IF NOT EXISTS superseded_event_id ON investigation_event TYPE option<string>;
DEFINE INDEX IF NOT EXISTS investigation_event_seq ON investigation_event FIELDS investigation_id, sequence;";

#[derive(SurrealValue, Clone)]
struct EventContent {
    seq: i64,
    ts: String,
    severity: String,
    rank: i64,
    app: String,
    event_type: String,
    message: String,
    details: String,
    job_id: String,
    task_id: String,
    attempt_id: String,
    run_id: String,
    resource_ref: String,
    correlation_id: String,
}

#[derive(SurrealValue)]
struct StoredEvent {
    key: String,
    seq: i64,
    ts: String,
    severity: String,
    app: String,
    event_type: String,
    message: String,
    details: String,
    job_id: String,
    task_id: String,
    attempt_id: String,
    run_id: String,
    resource_ref: String,
    correlation_id: String,
}

const EVENT_COLS: &str = "record::id(id) AS key, seq, ts, severity, app, event_type, message,
    details, job_id, task_id, attempt_id, run_id, resource_ref, correlation_id";

impl From<StoredEvent> for EventRow {
    fn from(e: StoredEvent) -> Self {
        EventRow {
            id: e.key,
            seq: e.seq,
            ts: e.ts,
            severity: e.severity,
            app: e.app,
            event_type: e.event_type,
            message: e.message,
            details: e.details,
            job_id: e.job_id,
            task_id: e.task_id,
            attempt_id: e.attempt_id,
            run_id: e.run_id,
            resource_ref: e.resource_ref,
            correlation_id: e.correlation_id,
        }
    }
}

/// Counter record that orders `argos_event` rows (the outbox's
/// `index_change` counter names itself the same way).
const SEQ_COUNTER: &str = "argos_event_seq";

/// Per-process "the seq counter was seeded" marker, so the seed stays out of
/// the hot path. Assumption: a store's `argos_event` writers run in this
/// process (embedded, or this client against its server), so the first
/// [`record_event`] sees the store as it is; a foreign process that seeds
/// later only raises the counter with `math::max`, which still yields
/// strictly increasing, possibly gapped (never reused) seqs.
static SEQ_SEEDED: OnceLock<()> = OnceLock::new();

/// Raise [`SEQ_COUNTER`] to the highest stored `seq` when it is lower —
/// once per process, inside the first [`record_event`]'s transaction (see
/// [`SEQ_SEEDED`]), so the seed commits atomically with that event. The max
/// is read with ORDER BY/LIMIT because `math::max` of no rows is -Infinity
/// on a server; `math::max` keeps a counter that is already ahead (pruned or
/// cleared rows) from moving backwards.
async fn seed_seq(tx: &Tx) -> Result<(), TxError> {
    run(
        tx,
        tx.query(
            "LET $top = (SELECT VALUE seq FROM argos_event ORDER BY seq DESC LIMIT 1)[0] ?? 0;
             UPSERT $c SET n = math::max([n ?? 0, $top]) RETURN NONE;",
        )
        .bind(("c", rid("counter", SEQ_COUNTER))),
    )
    .await?;
    Ok(())
}

/// `events::record_event` on SurrealDB: persist one redacted event and
/// return its id. `seq` is allocated from the [`SEQ_COUNTER`] record in the
/// same transaction as the row, so the two commit (or roll back) together.
pub async fn record_event(db: &Surreal<Any>, event: &NewEvent) -> Result<String> {
    let now = chrono::Utc::now();
    let id = new_event_id(now.timestamp_millis());
    let severity = event.severity.unwrap_or(Severity::Info);
    let content = EventContent {
        seq: 0,
        ts: now.to_rfc3339(),
        severity: severity.as_str().to_string(),
        rank: severity_rank(severity),
        app: event.app.clone(),
        event_type: event.event_type.clone(),
        message: bound(&redact(&event.message), MAX_MESSAGE_CHARS),
        details: bound(&redact(&event.details), MAX_DETAIL_CHARS),
        job_id: event.job_id.clone(),
        task_id: event.task_id.clone(),
        attempt_id: event.attempt_id.clone(),
        run_id: event.run_id.clone(),
        resource_ref: event.resource_ref.clone(),
        correlation_id: event.correlation_id.clone(),
    };
    let key = id.clone();
    // Read before the first attempt; a failed transaction leaves the marker
    // unset so the next call seeds again.
    let seed = SEQ_SEEDED.get().is_none();
    run_tx(db, counters::HOT_RECORD_POLICY, |tx| {
        let content = content.clone();
        let key = key.clone();
        async move {
            if seed {
                seed_seq(&tx).await?;
            }
            let mut content = content;
            content.seq = counters::next_in(&tx, SEQ_COUNTER).await?;
            run(
                &tx,
                tx.query("CREATE $e CONTENT $row RETURN NONE")
                    .bind(("e", rid("argos_event", &key)))
                    .bind(("row", content)),
            )
            .await?;
            Ok(())
        }
    })
    .await?;
    if seed {
        // Losing the race to another task is fine: it seeded the same way.
        let _ = SEQ_SEEDED.set(());
    }
    Ok(id)
}

/// `events::list_events` on SurrealDB: newest-first bounded page. The job
/// filter matches the job and its descendant jobs.
pub async fn list_events(
    db: &Surreal<Any>,
    filter: &EventFilter,
    limit: usize,
) -> Result<Vec<EventRow>> {
    let mut sql = format!("SELECT {EVENT_COLS} FROM argos_event WHERE true");
    if filter.min_severity.is_some() {
        sql.push_str(" AND rank >= $rank");
    }
    if !filter.app.is_empty() {
        sql.push_str(" AND app = $app");
    }
    let jobs = if filter.job_id.is_empty() {
        Vec::new()
    } else {
        job_tree(db, &filter.job_id).await?
    };
    if !filter.job_id.is_empty() {
        sql.push_str(" AND job_id IN $jobs");
    }
    if !filter.text.is_empty() {
        sql.push_str(" AND (string::contains(message, $text) OR string::contains(details, $text))");
    }
    if !filter.since.is_empty() {
        sql.push_str(" AND ts >= $since");
    }
    if filter.before_seq.is_some() {
        sql.push_str(" AND seq < $before");
    }
    sql.push_str(" ORDER BY seq DESC LIMIT $limit");
    let mut r = db
        .query(sql)
        .bind(("rank", filter.min_severity.map(severity_rank).unwrap_or(0)))
        .bind(("app", filter.app.clone()))
        .bind(("jobs", jobs))
        .bind(("text", filter.text.replace('%', "")))
        .bind(("since", filter.since.clone()))
        .bind(("before", filter.before_seq.unwrap_or(0)))
        .bind(("limit", limit.max(1) as i64))
        .await?
        .check()?;
    let rows: Vec<StoredEvent> = r.take(0)?;
    Ok(rows.into_iter().map(EventRow::from).collect())
}

/// The job id and every descendant job id (breadth-first over
/// `job.parent`). An unknown job is just itself, so events recorded for a
/// job that was already cleared still match, as in SQLite.
pub async fn job_tree(db: &Surreal<Any>, root: &str) -> Result<Vec<String>> {
    let mut all = vec![root.to_string()];
    let mut frontier = vec![root.to_string()];
    for _ in 0..MAX_JOB_DEPTH {
        frontier.retain(|id| !id.is_empty());
        if frontier.is_empty() {
            break;
        }
        let parents: Vec<_> = frontier.iter().map(|id| rid("job", id)).collect();
        let mut r = db
            .query("SELECT VALUE record::id(id) FROM job WHERE parent IN $parents")
            .bind(("parents", parents))
            .await?
            .check()?;
        let children: Vec<String> = r.take(0)?;
        frontier = children
            .into_iter()
            .filter(|id| !all.contains(id))
            .collect();
        all.extend(frontier.iter().cloned());
    }
    Ok(all)
}

/// `events::get_event` on SurrealDB. `None` means it expired or never
/// existed.
pub async fn get_event(db: &Surreal<Any>, id: &str) -> Result<Option<EventRow>> {
    if id.is_empty() {
        return Ok(None);
    }
    let mut r = db
        .query(format!("SELECT {EVENT_COLS} FROM ONLY $e"))
        .bind(("e", rid("argos_event", id)))
        .await?
        .check()?;
    let row: Option<StoredEvent> = r.take(0)?;
    Ok(row.map(EventRow::from))
}

/// `events::prune_events` on SurrealDB: delete events older than `hours`.
/// Touches only `argos_event`.
pub async fn prune_events(db: &Surreal<Any>, hours: i64) -> Result<usize> {
    let cutoff = (chrono::Utc::now() - chrono::Duration::hours(hours)).to_rfc3339();
    let mut r = db
        .query(
            "LET $gone = (DELETE argos_event WHERE ts < $cutoff RETURN BEFORE);
             RETURN array::len($gone);",
        )
        .bind(("cutoff", cutoff))
        .await?
        .check()?;
    let n: Option<i64> = r.take(1)?;
    Ok(n.unwrap_or(0).max(0) as usize)
}

/// `events::clear_events` on SurrealDB (Logs "Clear log"). Touches only
/// `argos_event`: jobs, task results and memories are kept.
pub async fn clear_events(db: &Surreal<Any>) -> Result<usize> {
    let mut r = db
        .query(
            "LET $gone = (DELETE argos_event RETURN BEFORE);
             RETURN array::len($gone);",
        )
        .await?
        .check()?;
    let n: Option<i64> = r.take(1)?;
    Ok(n.unwrap_or(0).max(0) as usize)
}

#[derive(SurrealValue)]
struct SeverityCount {
    severity: String,
    n: i64,
}

/// `jobs_view::event_counts` on SurrealDB: severity counts for the Logs
/// header (unknown severities count as info) and errors in the last hour.
pub async fn event_counts(db: &Surreal<Any>) -> Result<EventCounts> {
    let hour = (chrono::Utc::now() - chrono::Duration::hours(1)).to_rfc3339();
    let mut r = db
        .query(
            "SELECT severity, count() AS n FROM argos_event GROUP BY severity;
             RETURN count((SELECT VALUE id FROM argos_event
               WHERE severity = 'error' AND ts >= $hour));",
        )
        .bind(("hour", hour))
        .await?
        .check()?;
    let rows: Vec<SeverityCount> = r.take(0)?;
    let recent: Option<i64> = r.take(1)?;
    let mut counts = EventCounts::default();
    for row in rows {
        match row.severity.as_str() {
            "error" => counts.error += row.n,
            "warn" => counts.warn += row.n,
            "debug" => counts.debug += row.n,
            _ => counts.info += row.n,
        }
    }
    counts.recent_failures = recent.unwrap_or(0);
    Ok(counts)
}

#[derive(SurrealValue)]
struct AppCount {
    app: String,
    n: i64,
}

/// `jobs_view::event_apps` on SurrealDB: distinct non-empty apps, sorted,
/// at most 32. A `GROUP BY app` aggregate replaces the whole-table
/// `array::distinct` + `array::sort` scan: the dedup happens in the
/// database (one row per app, `count()` kept for the aggregate shape shared
/// with [`event_counts`]), and the sort happens in Rust — byte order, which
/// `array::sort` also uses for strings (UTF-8 byte order is code-point
/// order, and neither side folds case), so the output order and contents are
/// unchanged.
pub async fn event_apps(db: &Surreal<Any>) -> Result<Vec<String>> {
    let mut r = db
        .query("SELECT app, count() AS n FROM argos_event WHERE app != '' GROUP BY app;")
        .await?
        .check()?;
    let rows: Vec<AppCount> = r.take(0)?;
    let mut apps: Vec<String> = rows.into_iter().map(|row| row.app).collect();
    apps.sort();
    apps.truncate(32);
    Ok(apps)
}

#[derive(SurrealValue, Clone)]
struct InvestigationContent {
    investigation_id: String,
    sequence: i64,
    occurrence_time: String,
    recording_time: String,
    surface: String,
    run_id: String,
    directive_id: String,
    task_id: String,
    revision: i64,
    parent_event_id: Option<String>,
    role: String,
    model: String,
    provider: String,
    attempt_id: String,
    event_type: String,
    status: String,
    summary: String,
    payload_json: String,
    evidence_refs_json: String,
    superseded_event_id: Option<String>,
}

#[derive(SurrealValue)]
struct StoredInvestigation {
    key: String,
    investigation_id: String,
    sequence: i64,
    occurrence_time: String,
    recording_time: String,
    surface: String,
    run_id: String,
    directive_id: String,
    task_id: String,
    revision: i64,
    parent_event_id: Option<String>,
    role: String,
    model: String,
    provider: String,
    attempt_id: String,
    event_type: String,
    status: String,
    summary: String,
    payload_json: String,
    evidence_refs_json: String,
    superseded_event_id: Option<String>,
}

/// `Store::insert_investigation_event` on SurrealDB.
pub async fn insert_investigation_event(db: &Surreal<Any>, ev: &InvestigationEvent) -> Result<()> {
    let content = InvestigationContent {
        investigation_id: ev.investigation_id.clone(),
        sequence: ev.sequence,
        occurrence_time: ev.occurrence_time.clone(),
        recording_time: ev.recording_time.clone(),
        surface: ev.surface.as_str().to_string(),
        run_id: ev.run_id.clone(),
        directive_id: ev.directive_id.clone(),
        task_id: ev.task_id.clone(),
        revision: ev.revision,
        parent_event_id: ev.parent_event_id.clone(),
        role: ev.role.clone(),
        model: ev.model.clone(),
        provider: ev.provider.clone(),
        attempt_id: ev.attempt_id.clone(),
        event_type: ev.event_type.clone(),
        status: ev.status.clone(),
        summary: ev.summary.clone(),
        payload_json: serde_json::to_string(&ev.payload)?,
        evidence_refs_json: serde_json::to_string(&ev.evidence_refs)?,
        superseded_event_id: ev.superseded_event_id.clone(),
    };
    db.query(
        "IF record::exists($e) {
           UPDATE $e SET status = $row.status, summary = $row.summary,
             payload_json = $row.payload_json RETURN NONE;
         } ELSE {
           CREATE $e CONTENT $row RETURN NONE;
         };",
    )
    .bind(("e", rid("investigation_event", &ev.id)))
    .bind(("row", content))
    .await?
    .check()?;
    Ok(())
}

/// `Store::next_event_sequence` on SurrealDB.
pub async fn next_event_sequence(db: &Surreal<Any>, investigation_id: &str) -> Result<i64> {
    let mut r = db
        .query(
            "RETURN (SELECT VALUE sequence FROM investigation_event
               WHERE investigation_id = $i ORDER BY sequence DESC LIMIT 1)[0] ?? 0;",
        )
        .bind(("i", investigation_id.to_string()))
        .await?
        .check()?;
    let top: Option<i64> = r.take(0)?;
    Ok(top.unwrap_or(0) + 1)
}

/// `Store::list_investigation_events` on SurrealDB: one thread's trace in
/// sequence order.
pub async fn list_investigation_events(
    db: &Surreal<Any>,
    investigation_id: &str,
) -> Result<Vec<InvestigationEvent>> {
    let mut r = db
        .query(
            "SELECT record::id(id) AS key, investigation_id, sequence, occurrence_time,
                recording_time, surface, run_id, directive_id, task_id, revision,
                parent_event_id, role, model, provider, attempt_id, event_type, status,
                summary, payload_json, evidence_refs_json, superseded_event_id
             FROM investigation_event WHERE investigation_id = $i ORDER BY sequence ASC",
        )
        .bind(("i", investigation_id.to_string()))
        .await?
        .check()?;
    let rows: Vec<StoredInvestigation> = r.take(0)?;
    Ok(rows
        .into_iter()
        .map(|e| InvestigationEvent {
            id: e.key,
            investigation_id: e.investigation_id,
            sequence: e.sequence,
            occurrence_time: e.occurrence_time,
            recording_time: e.recording_time,
            surface: InvestigationSurface::parse(&e.surface),
            run_id: e.run_id,
            directive_id: e.directive_id,
            task_id: e.task_id,
            revision: e.revision,
            parent_event_id: e.parent_event_id,
            role: e.role,
            model: e.model,
            provider: e.provider,
            attempt_id: e.attempt_id,
            event_type: e.event_type,
            status: e.status,
            summary: e.summary,
            payload: serde_json::from_str(&e.payload_json).unwrap_or_default(),
            evidence_refs: serde_json::from_str(&e.evidence_refs_json).unwrap_or_default(),
            superseded_event_id: e.superseded_event_id,
        })
        .collect())
}
