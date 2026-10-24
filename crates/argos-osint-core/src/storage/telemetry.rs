//! Telemetry writes on SurrealDB (`crate::telemetry::insert` parity).
//!
//! [`record`] writes one `telemetry_event` (keyed by the event id) and, the
//! first time that id is seen, folds it into `telemetry_hourly` and
//! `telemetry_daily` in the same transaction, exactly like SQLite: a replayed
//! id rewrites the row and never counts twice. The tables are defined by
//! `storage::profile_view::SCHEMA`, which the Profile views read.
//!
//! Under a storage config, `crate::telemetry::record` routes here (through
//! [`crate::storage::runtime::spawn`]) and never opens SQLite.

use anyhow::Result;
use surrealdb::engine::any::Any;
use surrealdb::types::SurrealValue;
use surrealdb::Surreal;

use super::jobs::{rid, run, take};
use super::tx::{run_tx, RetryPolicy};
use crate::telemetry::{event_bins, next_id, TelemetryEvent};

#[derive(SurrealValue, Clone)]
struct Row {
    event_type: String,
    occurred_at: String,
    day: String,
    hour: String,
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
    thread_id: String,
    turn_id: String,
    article_id: String,
    origin: String,
    call_id: String,
    canonical_ref: String,
    metric_ms: Option<i64>,
    count: i64,
    payload_json: String,
}

fn rollup_id(bucket: &str, event_type: &str, dim_key: &str) -> String {
    format!("{bucket}\u{1e}{event_type}\u{1e}{dim_key}")
}

/// Persist one event and refresh its rollups (first sighting only).
pub async fn record(db: &Surreal<Any>, event: &TelemetryEvent) -> Result<()> {
    let id = if event.id.is_empty() {
        next_id()
    } else {
        event.id.clone()
    };
    let occurred = event.occurred_at.clone();
    let day: String = occurred.chars().take(10).collect();
    let hour: String = occurred.chars().take(13).collect();
    let row = Row {
        event_type: event.event_type.clone(),
        occurred_at: occurred,
        day: day.clone(),
        hour: hour.clone(),
        app: event.app.clone(),
        trigger: event.trigger.clone(),
        tool_id: event.tool_id.clone(),
        category: event.category.clone(),
        provider: event.provider.clone(),
        role: event.role.clone(),
        model: event.model.clone(),
        mode: event.mode.clone(),
        engine: event.engine.clone(),
        outcome: event.outcome.clone(),
        reason: event.reason.clone(),
        job_id: event.job_id.clone(),
        run_id: event.run_id.clone(),
        thread_id: event.thread_id.clone(),
        turn_id: event.turn_id.clone(),
        article_id: event.article_id.clone(),
        origin: event.origin.clone(),
        call_id: event.call_id.clone(),
        canonical_ref: event.canonical_ref.clone(),
        metric_ms: event.metric_ms,
        count: event.count,
        payload_json: event.payload_json.clone(),
    };
    let metric = event.metric_ms.unwrap_or(0).max(0);
    let bins_json =
        serde_json::to_string(&event_bins(metric as u64)).unwrap_or_else(|_| "[]".into());
    let dim_key = event.rollup_key();
    let event_type = event.event_type.clone();
    let events = event.count.max(0);
    run_tx(db, RetryPolicy::default(), |tx| {
        let row = row.clone();
        let (id, day, hour) = (id.clone(), day.clone(), hour.clone());
        let (bins_json, dim_key, event_type) =
            (bins_json.clone(), dim_key.clone(), event_type.clone());
        async move {
            let mut r = run(
                &tx,
                tx.query(
                    "LET $seen = record::exists($e);
                     UPSERT $e CONTENT $row RETURN NONE;
                     IF !$seen {
                       UPSERT $h SET bucket = $hour, event_type = $type, dim_key = $dim,
                         events = (events ?? 0) + $n, total_ms = (total_ms ?? 0) + $ms,
                         max_ms = math::max([max_ms ?? 0, $ms]), bins_json = $bins RETURN NONE;
                       UPSERT $d SET bucket = $day, event_type = $type, dim_key = $dim,
                         events = (events ?? 0) + $n, total_ms = (total_ms ?? 0) + $ms,
                         max_ms = math::max([max_ms ?? 0, $ms]), bins_json = $bins RETURN NONE;
                     };
                     RETURN $seen;",
                )
                .bind(("e", rid("telemetry_event", &id)))
                .bind(("row", row))
                .bind((
                    "h",
                    rid("telemetry_hourly", &rollup_id(&hour, &event_type, &dim_key)),
                ))
                .bind((
                    "d",
                    rid("telemetry_daily", &rollup_id(&day, &event_type, &dim_key)),
                ))
                .bind(("hour", hour))
                .bind(("day", day))
                .bind(("type", event_type))
                .bind(("dim", dim_key))
                .bind(("n", events))
                .bind(("ms", metric))
                .bind(("bins", bins_json)),
            )
            .await?;
            let last = r.num_statements() - 1;
            let _: Option<bool> = take(&mut r, last)?;
            Ok(())
        }
    })
    .await?;
    Ok(())
}

/// The next `recon_stage` attempt ordinal for a run and stage: the number of
/// distinct stage rows already written (ids `run:stage:n`) plus one.
pub async fn stage_attempt(db: &Surreal<Any>, run_id: &str, stage: &str) -> Result<i64> {
    let mut r = db
        .query(
            "RETURN count(SELECT VALUE id FROM telemetry_event
               WHERE event_type = 'recon_stage' AND string::starts_with(record::id(id), $p));",
        )
        .bind(("p", format!("{run_id}:{stage}:")))
        .await?
        .check()?;
    let n: Option<i64> = r.take(0)?;
    Ok(n.unwrap_or(0) + 1)
}

/// Every stored event of `event_type`, oldest first (contract tests and
/// diagnostics; Profile reads through `profile_view`).
pub async fn events_of_type(db: &Surreal<Any>, event_type: &str) -> Result<Vec<TelemetryEvent>> {
    #[derive(SurrealValue)]
    struct Stored {
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
        thread_id: String,
        turn_id: String,
        article_id: String,
        origin: String,
        call_id: String,
        canonical_ref: String,
        metric_ms: Option<i64>,
        count: i64,
        payload_json: String,
    }
    let mut r = db
        .query(
            "SELECT record::id(id) AS key, event_type, occurred_at, app, trigger, tool_id, category, provider, role, model, mode, engine, outcome, reason, job_id, run_id, thread_id, turn_id, article_id, origin, call_id, canonical_ref, metric_ms, count, payload_json
             FROM telemetry_event WHERE event_type = $t ORDER BY occurred_at, key",
        )
        .bind(("t", event_type.to_string()))
        .await?
        .check()?;
    let rows: Vec<Stored> = r.take(0)?;
    Ok(rows
        .into_iter()
        .map(|s| TelemetryEvent {
            id: s.key,
            event_type: s.event_type,
            occurred_at: s.occurred_at,
            app: s.app,
            trigger: s.trigger,
            tool_id: s.tool_id,
            category: s.category,
            provider: s.provider,
            role: s.role,
            model: s.model,
            mode: s.mode,
            engine: s.engine,
            outcome: s.outcome,
            reason: s.reason,
            job_id: s.job_id,
            run_id: s.run_id,
            thread_id: s.thread_id,
            turn_id: s.turn_id,
            article_id: s.article_id,
            origin: s.origin,
            call_id: s.call_id,
            canonical_ref: s.canonical_ref,
            metric_ms: s.metric_ms,
            count: s.count,
            payload_json: s.payload_json,
        })
        .collect())
}
