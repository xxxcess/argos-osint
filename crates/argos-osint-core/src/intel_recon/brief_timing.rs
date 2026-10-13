//! Per-stage Intel brief timing (spec §2) and live substep/role/model labels.
//!
//! Local phases (retrieval/NLP/selection, model wall time, fidelity,
//! persistence) are measured by the orchestrator; provider admission, first
//! meaningful output, generation and backoff are aggregated from the existing
//! per-attempt rows (`recon_model_attempts`) of operations owned by the task.
//! Only counts and durations are stored or logged — never source/model text,
//! prompts or secrets.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::recon::model_exec::ModelExecEvent;
use crate::store::Store;
use crate::tasks::ClaimedTask;

/// Locally measured phase of one stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Phase {
    /// Retrieval, cleanup, NLP, windowing, shortlist and prompt assembly.
    Local,
    /// Wall time inside a generation request chain (includes admission/backoff).
    Model,
    /// Wall time inside the fidelity assessment chain.
    Fidelity,
    /// Dispatch → first meaningful answer/reasoning delta (first one only).
    FirstOutput,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageTiming {
    pub task_id: String,
    pub job_id: String,
    pub operation: String,
    /// Ready (dependencies met, eligible) → claimed.
    pub queue_ms: i64,
    pub local_ms: i64,
    /// Sum of provider admission waits (queued → sent) across attempts.
    pub admission_ms: i64,
    /// Dispatch → first meaningful answer/reasoning delta (first attempt that
    /// produced any). Header-only/heartbeat activity does not count.
    pub first_output_ms: i64,
    /// Wire activity: sent → response headers of the first successful attempt.
    pub headers_ms: i64,
    /// Sum of wire attempt durations.
    pub generation_ms: i64,
    pub fidelity_ms: i64,
    /// Sum of retry/backoff waits between attempts.
    pub backoff_ms: i64,
    pub persistence_ms: i64,
    /// Claim → commit/failure wall time.
    pub total_ms: i64,
    pub attempts: i64,
    /// Distinct `provider/model` routes that were dispatched.
    pub routes: Vec<String>,
    pub outcome: String,
}

/// Per-stage phase accumulator. Scoped to the stage's future with
/// [`with_clock`], so concurrent stages (and databases) never share one.
pub type StageClock = Arc<Mutex<HashMap<Phase, Duration>>>;

tokio::task_local! {
    static CLOCK: StageClock;
}

/// Run `fut` with `clock` as the current stage clock.
pub async fn with_clock<F: std::future::Future>(clock: StageClock, fut: F) -> F::Output {
    CLOCK.scope(clock, fut).await
}

fn add(map: &mut HashMap<Phase, Duration>, phase: Phase, elapsed: Duration) {
    if phase == Phase::FirstOutput {
        map.entry(phase).or_insert(elapsed);
    } else {
        *map.entry(phase).or_default() += elapsed;
    }
}

/// Add `elapsed` to `phase` on the current stage clock (no-op outside one).
pub fn note(phase: Phase, elapsed: Duration) {
    let _ = CLOCK.try_with(|clock| {
        if let Ok(mut map) = clock.lock() {
            add(&mut map, phase, elapsed);
        }
    });
}

/// Add directly to a clock.
pub fn note_on(clock: &StageClock, phase: Phase, elapsed: Duration) {
    if let Ok(mut map) = clock.lock() {
        add(&mut map, phase, elapsed);
    }
}

/// Accumulated time for `phase` on `clock`.
pub fn peek(clock: &StageClock, phase: Phase) -> Duration {
    clock
        .lock()
        .ok()
        .and_then(|m| m.get(&phase).copied())
        .unwrap_or_default()
}

/// Run `fut`, attributing its wall time to `phase`.
pub async fn timed<F: std::future::Future>(phase: Phase, fut: F) -> F::Output {
    let start = std::time::Instant::now();
    let out = fut.await;
    note(phase, start.elapsed());
    out
}

fn ms(d: Option<&Duration>) -> i64 {
    d.map_or(0, |d| d.as_millis() as i64)
}

fn ms_between(a: &str, b: &str) -> i64 {
    match (
        chrono::DateTime::parse_from_rfc3339(a),
        chrono::DateTime::parse_from_rfc3339(b),
    ) {
        (Ok(a), Ok(b)) => (b - a).num_milliseconds().max(0),
        _ => 0,
    }
}

/// When the task became runnable: created/eligible and all dependencies done.
fn ready_at(conn: &Connection, task: &str) -> Result<String> {
    let (created, eligible): (String, String) = conn.query_row(
        "SELECT created_at, next_eligible_at FROM argos_tasks WHERE id=?1",
        [task],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let deps: Option<String> = conn
        .query_row(
            "SELECT MAX(t.updated_at) FROM argos_task_dependencies d
             JOIN argos_tasks t ON t.id=d.depends_on WHERE d.task_id=?1",
            [task],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    Ok([created, eligible, deps.unwrap_or_default()]
        .into_iter()
        .max()
        .unwrap_or_default())
}

/// Build the timing record for a finished stage.
pub fn collect(
    conn: &Connection,
    claimed: &ClaimedTask,
    clock: &StageClock,
    total: Duration,
    persistence: Duration,
    outcome: &str,
) -> Result<StageTiming> {
    let local = clock.lock().map(|m| m.clone()).unwrap_or_default();
    let ready = ready_at(conn, &claimed.id).unwrap_or_default();
    let mut t = StageTiming {
        task_id: claimed.id.clone(),
        job_id: claimed.job_id.clone(),
        operation: claimed.operation.clone(),
        queue_ms: if ready.is_empty() {
            0
        } else {
            ms_between(&ready, &claimed.started_at)
        },
        local_ms: ms(local.get(&Phase::Local)),
        first_output_ms: ms(local.get(&Phase::FirstOutput)),
        fidelity_ms: ms(local.get(&Phase::Fidelity)),
        persistence_ms: persistence.as_millis() as i64,
        total_ms: total.as_millis() as i64,
        outcome: outcome.to_string(),
        ..Default::default()
    };
    let mut stmt = conn.prepare(
        "SELECT a.provider, a.model, a.dispatched, a.outcome, a.queue_ms, a.wait_ms,
                a.elapsed_ms, a.ttfb_ms
         FROM recon_model_attempts a JOIN recon_model_operations o ON o.id=a.operation_id
         WHERE o.task_id=?1 AND a.started_at >= ?2
         ORDER BY a.started_at, a.attempt",
    )?;
    let rows = stmt.query_map(params![claimed.id, claimed.started_at], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, i64>(4)?,
            r.get::<_, i64>(5)?,
            r.get::<_, i64>(6)?,
            r.get::<_, i64>(7)?,
        ))
    })?;
    let mut first_set = false;
    for row in rows {
        let (provider, model, dispatched, outcome, queue, wait, elapsed, ttfb) = row?;
        t.admission_ms += queue;
        t.backoff_ms += wait;
        if dispatched != 0 {
            t.attempts += 1;
            t.generation_ms += elapsed;
            let route = format!("{provider}/{model}");
            if !t.routes.contains(&route) {
                t.routes.push(route);
            }
        }
        if !first_set && outcome == "ok" {
            t.headers_ms = ttfb;
            first_set = true;
        }
    }
    // Requests the orchestrator timed but which wrote no attempt rows (test
    // doubles, cache hits) still report their wall time as generation.
    if t.generation_ms == 0 {
        t.generation_ms = ms(local.get(&Phase::Model));
    }
    Ok(t)
}

pub fn persist(conn: &Connection, t: &StageTiming, at: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO intel_brief_stage_timings
         (task_id, job_id, operation, queue_ms, local_ms, admission_ms, first_output_ms,
          headers_ms, generation_ms, fidelity_ms, backoff_ms, persistence_ms, total_ms, attempts,
          routes_json, outcome, recorded_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)
         ON CONFLICT(task_id) DO UPDATE SET
          queue_ms=excluded.queue_ms, local_ms=excluded.local_ms,
          admission_ms=excluded.admission_ms, first_output_ms=excluded.first_output_ms,
          headers_ms=excluded.headers_ms,
          generation_ms=excluded.generation_ms, fidelity_ms=excluded.fidelity_ms,
          backoff_ms=excluded.backoff_ms, persistence_ms=excluded.persistence_ms,
          total_ms=excluded.total_ms, attempts=excluded.attempts,
          routes_json=excluded.routes_json, outcome=excluded.outcome,
          recorded_at=excluded.recorded_at",
        params![
            t.task_id,
            t.job_id,
            t.operation,
            t.queue_ms,
            t.local_ms,
            t.admission_ms,
            t.first_output_ms,
            t.headers_ms,
            t.generation_ms,
            t.fidelity_ms,
            t.backoff_ms,
            t.persistence_ms,
            t.total_ms,
            t.attempts,
            serde_json::to_string(&t.routes)?,
            t.outcome,
            at
        ],
    )?;
    Ok(())
}

/// Read stored timings for a chain, stage order.
pub fn for_job(conn: &Connection, job_id: &str) -> Result<Vec<StageTiming>> {
    let mut stmt = conn.prepare(
        "SELECT task_id, job_id, operation, queue_ms, local_ms, admission_ms, first_output_ms,
                headers_ms, generation_ms, fidelity_ms, backoff_ms, persistence_ms, total_ms, attempts,
                routes_json, outcome
         FROM intel_brief_stage_timings WHERE job_id=?1 ORDER BY recorded_at",
    )?;
    let rows = stmt.query_map([job_id], |r| {
        Ok(StageTiming {
            task_id: r.get(0)?,
            job_id: r.get(1)?,
            operation: r.get(2)?,
            queue_ms: r.get(3)?,
            local_ms: r.get(4)?,
            admission_ms: r.get(5)?,
            first_output_ms: r.get(6)?,
            headers_ms: r.get(7)?,
            generation_ms: r.get(8)?,
            fidelity_ms: r.get(9)?,
            backoff_ms: r.get(10)?,
            persistence_ms: r.get(11)?,
            total_ms: r.get(12)?,
            attempts: r.get(13)?,
            routes: serde_json::from_str(&r.get::<_, String>(14)?).unwrap_or_default(),
            outcome: r.get(15)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Compact counts-only summary for Logs.
pub fn summary(t: &StageTiming) -> String {
    let routes = if t.routes.is_empty() {
        "local".to_string()
    } else {
        t.routes.join(", ")
    };
    format!(
        "total {}ms · queue {} · local {} · admission {} · headers {} · first output {} · generation {} · fidelity {} · backoff {} · persistence {} · {} attempt(s) · {routes}",
        t.total_ms,
        t.queue_ms,
        t.local_ms,
        t.admission_ms,
        t.headers_ms,
        t.first_output_ms,
        t.generation_ms,
        t.fidelity_ms,
        t.backoff_ms,
        t.persistence_ms,
        t.attempts,
    )
}

/// Human substep for a stage operation key.
pub fn substep_label(stage: &str) -> &str {
    match stage {
        "intel_brief_body" => "retrieving / cleaning",
        "intel_brief_claims" => "extracting claims",
        "intel_brief_inferences" => "generating inferences",
        "intel_brief_links" => "selecting links",
        other => other,
    }
}

/// Display label for a logical role key (`claim_assessor` → `Claim Assessor`).
pub fn role_label(role: &str) -> String {
    role.split('_')
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            c.next()
                .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Live activity sink for a stage's model chain: writes the actual substep,
/// active role and model to the stage activity label on state changes
/// (dispatch, first meaningful output, backoff), never on every delta.
pub fn activity_sink(
    db_path: &Path,
    task_id: &str,
    substep: &str,
    role: &str,
) -> Arc<dyn Fn(ModelExecEvent) + Send + Sync> {
    let db: PathBuf = db_path.to_path_buf();
    let task = task_id.to_string();
    let prefix = format!("{} · {}", substep_label(substep), role_label(role));
    let model = Arc::new(Mutex::new(String::new()));
    let dispatched = Arc::new(Mutex::new(None::<std::time::Instant>));
    Arc::new(move |event: ModelExecEvent| {
        use crate::provider_attempt::AttemptProgress as P;
        let label = match event {
            ModelExecEvent::AttemptStart {
                model: m,
                attempt,
                provider,
                ..
            } => {
                let route = format!("{provider}/{m}");
                if let Ok(mut g) = model.lock() {
                    *g = route.clone();
                }
                Some(if attempt > 1 {
                    format!("{prefix} · {route} · attempt {attempt}")
                } else {
                    format!("{prefix} · {route} · awaiting admission")
                })
            }
            ModelExecEvent::Progress { progress, .. } => {
                let route = model.lock().map(|g| g.clone()).unwrap_or_default();
                match progress {
                    P::Dispatched => {
                        if let Ok(mut d) = dispatched.lock() {
                            *d = Some(std::time::Instant::now());
                        }
                        Some(format!("{prefix} · {route} · awaiting first output"))
                    }
                    P::FirstModelOutput => {
                        if let Some(at) = dispatched.lock().ok().and_then(|d| *d) {
                            note(Phase::FirstOutput, at.elapsed());
                        }
                        Some(format!("{prefix} · {route} · generating"))
                    }
                    P::StreamComplete { .. } => Some(format!("{prefix} · {route} · validating")),
                    _ => None,
                }
            }
            ModelExecEvent::RetryStatus { wait, .. } => {
                Some(format!("{prefix} · backing off {}s", wait.as_secs().max(1)))
            }
            _ => None,
        };
        if let Some(label) = label {
            if let Ok(store) = Store::open_light(&db) {
                let _ = super::brief_pipeline::set_activity(&store.conn, &task, &label);
            }
        }
    })
}

#[cfg(test)]
#[path = "brief_timing_tests.rs"]
mod tests;
