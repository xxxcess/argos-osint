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
