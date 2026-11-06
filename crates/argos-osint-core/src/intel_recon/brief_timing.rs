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

use serde::{Deserialize, Serialize};

use crate::recon::model_exec::ModelExecEvent;

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
    /// Cache lookup wall time.
    Cache,
    /// Frozen packet construction.
    Packet,
    /// Host validation after generation.
    Validation,
    /// Fenced storage publication.
    Persistence,
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
    /// Lease epoch of the execution this row describes (schema 41).
    #[serde(default)]
    pub task_epoch: Option<i64>,
}

/// Per-stage phase accumulator. Scoped to the stage's future with
/// [`with_clock`], so concurrent stages (and databases) never share one.
pub type StageClock = Arc<Mutex<HashMap<Phase, Duration>>>;

tokio::task_local! {
    static CLOCK: StageClock;
}

/// Capture the current stage clock before crossing a thread/task boundary.
/// A spawned future must use `with_clock`; Tokio does not inherit task locals.
pub fn current_clock() -> Option<StageClock> {
    CLOCK.try_with(Arc::clone).ok()
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
        // Stage activity labels were a legacy-store-only side channel.
        let _ = (label, &db, &task);
    })
}

#[cfg(test)]
#[path = "brief_timing_tests.rs"]
mod tests;

/// Counts-only observation. Missing phases stay null, never a fabricated zero.
/// This payload is operational telemetry and must never enter evidence/cache keys.
pub fn observation(
    clock: &StageClock,
    total: Duration,
    outcome: &str,
    rows: usize,
) -> serde_json::Value {
    let value = |phase| {
        clock.lock().ok().and_then(|map| {
            map.get(&phase)
                .map(|d| d.as_micros().min(u64::MAX as u128) as u64)
        })
    };
    serde_json::json!({"schema":"intel-stage-timing-v1","outcome":outcome,"total_us":total.as_micros().min(u64::MAX as u128) as u64,
        "local_us":value(Phase::Local),"model_us":value(Phase::Model),"fidelity_us":value(Phase::Fidelity),"first_output_us":value(Phase::FirstOutput),
        "cache_us":value(Phase::Cache),"packet_us":value(Phase::Packet),"validation_us":value(Phase::Validation),"persistence_us":value(Phase::Persistence),"rows":rows,
        "queue_us":null,"db_round_trips":null,"rss_bytes":null,"unavailable_reason":"queue admission, driver round trips and process RSS are not measured by this stage clock; provider attempts remain in model-operation telemetry"})
}

/// Bounded optional observation sink for isolated benchmarks and host diagnostics.
/// It carries operational measurements only, outside all evidence/cache identities.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ObservationLog {
    pub events: Vec<serde_json::Value>,
    pub omitted: usize,
}

pub type ObservationSink = Arc<Mutex<ObservationLog>>;

tokio::task_local! {
    static OBSERVATIONS: ObservationSink;
}

/// Capture the scope's emitted measurements without a global subscriber.
/// Spawned tasks must explicitly carry a captured sink, like the stage clock.
pub fn with_observations<F: std::future::Future>(
    sink: ObservationSink,
    fut: F,
) -> impl std::future::Future<Output = F::Output> {
    // Keep the measured operation out of the task-local wrapper's inline state.
    // Deep embedded queries otherwise share its large poll frame with every
    // caller, even when the operation's later model branch is not executing.
    OBSERVATIONS.scope(sink, Box::pin(fut))
}

/// Emit bounded numeric/count metadata through the existing tracing subscriber.
/// The optional scope receives at most 64 events; omitted counts stay explicit.
pub fn emit_observation(operation: &str, measurement: serde_json::Value) {
    let observation = serde_json::json!({"operation":operation,"measurement":measurement});
    tracing::info!(target: "argos::intel_performance", observation = %observation, "Intel stage measured");
    let _ = OBSERVATIONS.try_with(|sink| {
        if let Ok(mut log) = sink.lock() {
            if log.events.len() < 64 && observation.to_string().len() <= 4096 {
                log.events.push(observation);
            } else {
                log.omitted = log.omitted.saturating_add(1);
            }
        }
    });
}

/// Emit a structured operational event without writing during read navigation.
pub fn emit(clock: &StageClock, total: Duration, outcome: &str, rows: usize, operation: &str) {
    emit_observation(operation, observation(clock, total, outcome, rows));
}
