//! Durable event logs on SurrealDB: the `argos_events` sink
//! (`crate::events`) and the per-thread `investigation_events` trace
//! (`investigation::persist`), which the Recon pane reads.
//!
//! `argos_event` is keyed by the event id and ordered by `seq`, assigned in
//! the write's transaction from the highest stored `seq` (a concurrent writer
//! conflicts and retries). Messages and details are redacted and bounded
//! before they are stored, as in SQLite. `investigation_event` is keyed by
//! the event id; a rewrite of an existing id only updates its status, summary
//! and payload, like SQLite's `ON CONFLICT(id) DO UPDATE`.

use anyhow::Result;
use surrealdb::engine::any::Any;
use surrealdb::types::SurrealValue;
use surrealdb::Surreal;

use super::jobs::{rid, run};
use super::tx::{run_tx, RetryPolicy};
use crate::events::{
    bound, new_event_id, redact, severity_rank, EventFilter, EventRow, NewEvent, Severity,
    MAX_DETAIL_CHARS, MAX_MESSAGE_CHARS,
};
use crate::investigation::{InvestigationEvent, InvestigationSurface};

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

/// `events::record_event` on SurrealDB: persist one redacted event and
/// return its id.
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
    run_tx(db, RetryPolicy::default(), |tx| {
        let content = content.clone();
        let key = key.clone();
        async move {
            run(
                &tx,
                tx.query(
                    "LET $top = math::max(SELECT VALUE seq FROM argos_event) ?? 0;
                     CREATE $e CONTENT $row RETURN NONE;
                     UPDATE $e SET seq = $top + 1 RETURN NONE;",
                )
                .bind(("e", rid("argos_event", &key)))
                .bind(("row", content)),
            )
            .await?;
            Ok(())
        }
    })
    .await?;
    Ok(id)
}

/// `events::list_events` on SurrealDB: newest-first bounded page. The job
/// filter matches the job id itself (child jobs are not expanded yet).
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
    if !filter.job_id.is_empty() {
        sql.push_str(" AND job_id = $job");
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
        .bind(("job", filter.job_id.clone()))
        .bind(("text", filter.text.replace('%', "")))
        .bind(("since", filter.since.clone()))
        .bind(("before", filter.before_seq.unwrap_or(0)))
        .bind(("limit", limit.max(1) as i64))
        .await?
        .check()?;
    let rows: Vec<StoredEvent> = r.take(0)?;
    Ok(rows.into_iter().map(EventRow::from).collect())
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
            "RETURN math::max(SELECT VALUE sequence FROM investigation_event
               WHERE investigation_id = $i) ?? 0;",
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
