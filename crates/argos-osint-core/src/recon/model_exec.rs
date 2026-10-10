//! Shared model execution across Recon, Tool Picker, Synthesis, and Intel.
//!
//! Enforces:
//! - One role executor and unified route fallback chain.
//! - Per-route admission control (provider:host:account).
//! - Cancellable admission, dispatch, and backoff.
//! - Observability: attempt start/reset, streaming provisional deltas, retry status, final replacement.
//! - Durable operation state (pending/running/failed/cancelled/final).
//! - No credentials or reasoning text persisted.

use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::provider::{self, ChatMessage, SettingsFile};
use crate::provider_attempt::{self, Deadlines, Transport};
use crate::provider_chain::{self, ChainReport, DispatchError, ExecuteOptions, Route};
use crate::provider_diag::{Category, ProviderFailure, Stage};
use crate::secrets::{AuthFile, ProviderSecret};
use crate::store::Store;
use crate::telemetry::{self, EventKind, TelemetryEvent, Trigger};

/// App dimension on every telemetry row written from this module.
const TELEMETRY_APP: &str = "recon";
/// Model traffic dispatched here is Recon turn traffic; never inferred from text.
const TELEMETRY_TRIGGER: Trigger = Trigger::ReconPrompt;

/// Stable runtime for role execution.
#[derive(Clone, Debug)]
pub struct RoleRuntime {
    pub auth: AuthFile,
    pub settings: SettingsFile,
}

impl RoleRuntime {
    pub fn new(auth: AuthFile, settings: SettingsFile) -> Self {
        Self { auth, settings }
    }

    pub fn resolve_routes(&self, role: &str) -> (Vec<Route>, Vec<ProviderSecret>) {
        resolve_role_routes(&self.auth, &self.settings, role)
    }

    pub fn resolve_secret(&self, role: &str) -> Result<ProviderSecret> {
        provider::role_secret(&self.auth, &self.settings, role)
    }
}

/// Scope identifying a durable model operation.
#[derive(Clone, Debug)]
pub struct OperationScope<'a> {
    pub operation_id: &'a str,
    pub role: &'a str,
    pub run_id: Option<&'a str>,
    pub task_id: Option<&'a str>,
    pub generation: u32,
}

impl<'a> OperationScope<'a> {
    pub fn new(operation_id: &'a str, role: &'a str) -> Self {
        Self {
            operation_id,
            role,
            run_id: None,
            task_id: None,
            generation: 1,
        }
    }

    pub fn with_run(mut self, run_id: &'a str) -> Self {
        self.run_id = Some(run_id);
        self
    }

    pub fn with_task(mut self, task_id: &'a str) -> Self {
        self.task_id = Some(task_id);
        self
    }

    pub fn with_generation(mut self, generation: u32) -> Self {
        self.generation = generation;
        self
    }
}

/// Events emitted during model chain execution.
#[derive(Clone, Debug)]
pub enum ModelExecEvent {
    AttemptStart {
        operation_id: String,
        generation: u32,
        route_index: u32,
        attempt: u32,
        provider: String,
        account: String,
        model: String,
        transport: String,
    },
    AttemptReset {
        operation_id: String,
        attempt: u32,
    },
    ProvisionalDelta {
        operation_id: String,
        attempt: u32,
        delta: String,
    },
    AttemptFinish {
        operation_id: String,
        attempt: u32,
        outcome: String,
        category: Option<String>,
        finish_reason: Option<String>,
        chars: u64,
    },
    RetryStatus {
        operation_id: String,
        wait: Duration,
        next_attempt: u32,
        next_route: u32,
    },
    FinalReplacement {
        operation_id: String,
        text: String,
    },
}

/// Durable operation record in SQLite.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReconModelOperation {
    pub id: String,
    pub run_id: String,
    pub task_id: String,
    pub role: String,
    pub generation: u32,
    pub status: String,
    pub draft: String,
    pub final_message_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Durable attempt record in SQLite.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReconModelAttempt {
    pub id: String,
    pub operation_id: String,
    pub generation: u32,
    pub route_index: u32,
    pub attempt: u32,
    pub provider: String,
    pub account: String,
    pub model: String,
    pub transport: String,
    pub dispatched: bool,
    pub outcome: String,
    pub failure_category: String,
    pub http_status: Option<u16>,
    pub request_id: String,
    pub finish_reason: String,
    pub char_count: u64,
    pub wait_ms: u64,
    pub error_message: String,
    pub started_at: String,
    pub finished_at: String,
}

/// Resolve the effective role assignment and fallbacks into Routes and Secrets.
pub fn resolve_role_routes(
    auth: &AuthFile,
    settings: &SettingsFile,
    role: &str,
) -> (Vec<Route>, Vec<ProviderSecret>) {
    let canonical = provider::role_name(role).unwrap_or(role);
    let (assignment, _inherited) = settings.defaults.resolve_role(canonical);
    let chain = assignment.chain();
    let mut routes = Vec::with_capacity(chain.len());
    let mut secrets = Vec::with_capacity(chain.len());

    for (idx, model_route) in chain.iter().enumerate() {
        let mut secret = if idx == 0 && model_route.provider.is_empty() {
            // Legacy primary resolution
            provider::writer_secret(auth, settings)
        } else {
            provider::route_secret(auth, model_route)
        };
        if !model_route.model.is_empty() {
            secret.model = model_route.model.clone();
        }

        let provider_name = if model_route.provider.is_empty() {
            provider::effective_kind(&secret)
        } else {
            model_route.provider.clone()
        };
        let account_name = if model_route.account.is_empty() {
            provider_name.clone()
        } else {
            model_route.account.clone()
        };

        let host = url::Url::parse(&secret.base_url)
            .ok()
            .and_then(|u| {
                if let Some(port) = u.port() {
                    Some(format!("{}:{}", u.host_str().unwrap_or("default"), port))
                } else {
                    u.host_str().map(str::to_string)
                }
            })
            .unwrap_or_else(|| "default".into());
        let admission_key = format!("{}:{}:{}", provider_name, host, account_name);

        let missing_key = secret
            .api_key
            .as_deref()
            .map(str::trim)
            .unwrap_or("")
            .is_empty()
            && provider::effective_kind(&secret) != "local";
        let missing_model = secret.model.trim().is_empty();

        let skip_reason = if missing_model {
            Some("model is not configured".into())
        } else if missing_key {
            Some(format!(
                "API key missing; configure {account_name} in Providers"
            ))
        } else {
            None
        };

        routes.push(Route {
            provider: provider_name,
            account: account_name,
            model: secret.model.clone(),
            skip_reason,
            admission_key: Some(admission_key),
        });
        secrets.push(secret);
    }

    (routes, secrets)
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// Best-effort telemetry write. Instrumentation never fails a recon turn, so
/// every recorded event is discarded on error.
fn note(db_path: Option<&Path>, event: &TelemetryEvent) {
    if let Some(path) = db_path {
        let _ = telemetry::record(path, event);
    }
}

/// Canonical id for one wire attempt. Stable across replay, resume and retry so
/// telemetry cannot count the same attempt twice.
pub fn canonical_attempt_id(
    operation_id: &str,
    generation: u32,
    route_index: u32,
    attempt: u32,
) -> String {
    format!("{operation_id}-{generation}-{route_index}-{attempt}")
}

/// Measured per-attempt timing facts (the additive schema-27 columns).
///
/// The wall-clock stamps exist for the durable record; every *duration* is
/// measured on a monotonic clock, never by subtracting wall-clock stamps.
#[derive(Clone, Debug, Default)]
pub struct AttemptTimings {
    /// When the logical operation was enqueued, before the chain dispatched it.
    pub queued_at: String,
    /// When this attempt was admitted onto its route.
    pub admitted_at: String,
    /// When this attempt's request went on the wire.
    pub sent_at: String,
    /// First progress or first chunk, on success as well as on failure.
    pub first_response_at: String,
    /// Measured monotonic attempt duration.
    pub elapsed_ms: i64,
    /// Measured monotonic queue residence (`sent_at` − `queued_at`).
    pub queue_ms: i64,
    /// Measured monotonic time to first response (`first_response_at` − `sent_at`).
    pub ttfb_ms: i64,
    /// Prompt tokens when the adapter reports usage.
    pub prompt_tokens: Option<i64>,
    /// Completion tokens when the adapter reports usage.
    pub completion_tokens: Option<i64>,
    /// Whether the streamed response ran to its end.
    pub stream_completed: i64,
    queued_instant: Option<Instant>,
    sent_instant: Option<Instant>,
    first_response_instant: Option<Instant>,
    /// First-response latency reported by the adapter, used when no chunk leaked.
    reported_ttfb_ms: Option<i64>,
}

impl AttemptTimings {
    /// Timings for an attempt that inherits the operation's queue stamp.
    pub fn queued(queued_at: &str, queued_instant: Instant) -> Self {
        Self {
            queued_at: queued_at.to_string(),
            queued_instant: Some(queued_instant),
            ..Self::default()
        }
    }

    /// The chain admitted this attempt and handed it to the dispatch closure.
    pub fn admitted(&mut self) {
        self.admitted_at = now_rfc3339();
    }

    /// The request is about to be sent.
    pub fn sent(&mut self) {
        self.sent_at = now_rfc3339();
        self.sent_instant = Some(Instant::now());
    }

    /// First progress or first chunk, whichever arrived first. Keeps the first one.
    pub fn first_response(&mut self, at: &str, instant: Instant) {
        if self.first_response_instant.is_none() {
            self.first_response_at = at.to_string();
            self.first_response_instant = Some(instant);
        }
    }

    /// Adapter-reported first-response latency (failures carry one).
    pub fn reported_ttfb_ms(&mut self, ms: Option<u64>) {
        self.reported_ttfb_ms = ms.map(|value| value as i64);
    }

    /// Adapter-reported token usage. Adapters that report nothing leave the
    /// columns at their defaults rather than guessing.
    pub fn usage(&mut self, prompt_tokens: Option<i64>, completion_tokens: Option<i64>) {
        if let Some(prompt) = prompt_tokens {
            self.prompt_tokens = Some(prompt);
        }
        if let Some(completion) = completion_tokens {
            self.completion_tokens = Some(completion);
        }
    }

    /// Freeze the measured durations at the end of the attempt.
    pub fn finish(&mut self, elapsed_ms: u64, stream_completed: bool) {
        self.elapsed_ms = i64::try_from(elapsed_ms).unwrap_or(i64::MAX);
        if let (Some(sent), Some(queued)) = (self.sent_instant, self.queued_instant) {
            self.queue_ms = sent.saturating_duration_since(queued).as_millis() as i64;
        }
        self.ttfb_ms = match (self.first_response_instant, self.sent_instant) {
            (Some(first), Some(sent)) => first.saturating_duration_since(sent).as_millis() as i64,
            // No chunk arrived: keep whatever the adapter measured, else nothing.
            _ => self.reported_ttfb_ms.unwrap_or_default(),
        };
        self.stream_completed = i64::from(stream_completed);
    }
}

/// Terminal facts of one wire attempt, recorded on `recon_model_attempts`.
#[derive(Clone, Debug, Default)]
pub struct AttemptOutcome {
    /// `ok`, `fail` or `cancel`.
    pub outcome: String,
    /// Provider failure category, when the attempt failed.
    pub category: Option<String>,
    pub http_status: Option<u16>,
    pub request_id: Option<String>,
    pub finish_reason: Option<String>,
    pub char_count: u64,
    /// Retry/backoff wait charged to this attempt (policy, not measured latency).
    pub wait_ms: u64,
    pub error_message: Option<String>,
}

impl AttemptOutcome {
    /// A completed, validated attempt.
    pub fn ok(finish_reason: Option<&str>, char_count: u64, elapsed_ms: u64) -> Self {
        Self {
            outcome: "ok".into(),
            finish_reason: finish_reason.map(str::to_string),
            char_count,
            wait_ms: elapsed_ms,
            ..Self::default()
        }
    }

    /// A transport or protocol failure.
    pub fn failed(
        outcome: &str,
        category: &str,
        http_status: Option<u16>,
        request_id: Option<&str>,
        finish_reason: Option<&str>,
        char_count: u64,
        elapsed_ms: u64,
        error_message: &str,
    ) -> Self {
        Self {
            outcome: outcome.into(),
            category: Some(category.into()),
            http_status,
            request_id: request_id.map(str::to_string),
            finish_reason: finish_reason.map(str::to_string),
            char_count,
            wait_ms: elapsed_ms,
            error_message: Some(error_message.into()),
        }
    }
}

/// Facts shared by every telemetry row of one model operation. Owned so a
/// long-lived attribution never borrows values the attempt later moves.
#[derive(Clone, Debug)]
struct AttemptFacts {
    operation_id: String,
    generation: u32,
    role: String,
    run_id: String,
    provider: String,
    account: String,
    model: String,
    transport: String,
    route_index: u32,
    attempt: u32,
}

impl AttemptFacts {
    fn new(
        operation_id: &str,
        generation: u32,
        role: &str,
        run_id: &str,
        route: &Route,
        transport: &str,
        route_index: u32,
        attempt: u32,
    ) -> Self {
        Self {
            operation_id: operation_id.to_string(),
            generation,
            role: role.to_string(),
            run_id: run_id.to_string(),
            provider: route.provider.clone(),
            account: route.account.clone(),
            model: route.model.clone(),
            transport: transport.to_string(),
            route_index,
            attempt,
        }
    }
}

/// Exactly one `model_attempt` row per wire attempt, keyed by the canonical
/// attempt id so a replay or resume cannot double count it.
fn note_attempt(
    db_path: Option<&Path>,
    attempt_id: &str,
    facts: &AttemptFacts,
    outcome: &AttemptOutcome,
    timings: &AttemptTimings,
) {
    let Some(db_path) = db_path else {
        return;
    };
    let event = TelemetryEvent::new(EventKind::ModelAttempt)
        .with_id(attempt_id)
        .canonical(attempt_id)
        .at(now_rfc3339())
        .app(TELEMETRY_APP)
        .trigger(TELEMETRY_TRIGGER)
        .provider(&facts.provider)
        .role(&facts.role)
        .model(&facts.model)
        .mode(&facts.transport)
        .outcome(&outcome.outcome)
        .reason(outcome.category.as_deref().unwrap_or_default())
        .run(&facts.run_id)
        .duration_ms(Some(timings.elapsed_ms))
        .payload(json!({
            "operation_id": facts.operation_id,
            "generation": facts.generation,
            "route_index": facts.route_index,
            "attempt": facts.attempt,
            "account": facts.account,
            "queue_ms": timings.queue_ms,
            "ttfb_ms": timings.ttfb_ms,
            "stream_completed": timings.stream_completed,
            "wait_ms": outcome.wait_ms,
            "http_status": outcome.http_status,
            "chars": outcome.char_count,
        }));
    note(Some(db_path), &event);
}

/// Exactly one `model_operation` row per terminal logical operation. A logical
/// operation's many wire attempts never count as many jobs.
fn note_operation(
    db_path: Option<&Path>,
    scope: &OperationScope<'_>,
    status: &str,
    routes: &[Route],
    history: &[provider_chain::AttemptRecord],
    effective_route: Option<u32>,
    effective_model: &str,
) {
    let Some(db_path) = db_path else {
        return;
    };
    let effective_provider = effective_route
        .and_then(|index| routes.get(index as usize))
        .map(|route| route.provider.clone())
        .unwrap_or_default();
    let routes_tried: Vec<serde_json::Value> = history
        .iter()
        .filter(|record| record.dispatched)
        .map(|record| {
            json!({
                "route_index": record.route_index,
                "attempt": record.attempt,
                "model": record.model,
                "outcome": record.outcome,
            })
        })
        .collect();
    let skipped: Vec<serde_json::Value> = history
        .iter()
        .filter(|record| !record.dispatched)
        .map(|record| json!({"route_index": record.route_index, "reason": record.reason}))
        .collect();
    let event = TelemetryEvent::new(EventKind::ModelOperation)
        .with_id(format!(
            "model-op-{}-{}",
            scope.operation_id, scope.generation
        ))
        .canonical(scope.operation_id)
        .at(now_rfc3339())
        .app(TELEMETRY_APP)
        .trigger(TELEMETRY_TRIGGER)
        .provider(effective_provider)
        .role(scope.role)
        .model(effective_model)
        .outcome(status)
        .run(scope.run_id.unwrap_or_default())
        .count(1)
        .payload(json!({
            "routes_tried": routes_tried,
            "routes_skipped": skipped,
            "wire_attempts": routes_tried.len(),
            "effective_route": effective_route,
        }));
    note(Some(db_path), &event);
}

/// Freeze one attempt's measured timings, persist them on the durable attempt
/// row, and emit exactly one `model_attempt` telemetry row. Best-effort: a
/// telemetry failure never fails the attempt.
fn finish_attempt(
    db_path: Option<&Path>,
    attempt_id: &str,
    facts: &AttemptFacts,
    outcome: &AttemptOutcome,
    timings: &mut AttemptTimings,
    elapsed_ms: u64,
    stream_completed: bool,
) {
    // No adapter in this module reports token usage yet, so the columns stay at
    // their defaults rather than being guessed.
    timings.usage(None, None);
    timings.finish(elapsed_ms, stream_completed);
    let _ = record_attempt_finish(db_path, attempt_id, outcome, timings);
    note_attempt(db_path, attempt_id, facts, outcome, timings);
}

/// Persist an operation record as pending/running if db is present.
pub fn ensure_operation(
    db_path: Option<&Path>,
    scope: &OperationScope<'_>,
    draft: &str,
) -> Result<()> {
    let Some(db_path) = db_path else {
        return Ok(());
    };
    let store = Store::open(db_path)?;
    let now = now_rfc3339();
    store.conn.execute(
        "INSERT INTO recon_model_operations (id, run_id, task_id, role, generation, status, draft, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 'running', ?6, ?7, ?7)
         ON CONFLICT(id) DO UPDATE SET
           generation = excluded.generation,
           status = 'running',
           draft = CASE WHEN excluded.draft != '' THEN excluded.draft ELSE recon_model_operations.draft END,
           updated_at = excluded.updated_at",
        rusqlite::params![
            scope.operation_id,
            scope.run_id.unwrap_or(""),
            scope.task_id.unwrap_or(""),
            scope.role,
            scope.generation,
            draft,
            now,
        ],
    )?;
    Ok(())
}

/// Update durable draft for an operation.
pub fn save_operation_draft(db_path: Option<&Path>, operation_id: &str, draft: &str) -> Result<()> {
    let Some(db_path) = db_path else {
        return Ok(());
    };
    let store = Store::open(db_path)?;
    store.conn.execute(
        "UPDATE recon_model_operations SET draft = ?1, updated_at = ?2 WHERE id = ?3",
        rusqlite::params![draft, now_rfc3339(), operation_id],
    )?;
    Ok(())
}

/// Mark operation as terminal status (failed, cancelled, final).
pub fn update_operation_status(
    db_path: Option<&Path>,
    operation_id: &str,
    status: &str,
    final_message_id: Option<&str>,
) -> Result<()> {
    let Some(db_path) = db_path else {
        return Ok(());
    };
    let store = Store::open(db_path)?;
    store.conn.execute(
        "UPDATE recon_model_operations SET status = ?1, final_message_id = COALESCE(?2, final_message_id), updated_at = ?3 WHERE id = ?4",
        rusqlite::params![status, final_message_id, now_rfc3339(), operation_id],
    )?;
    Ok(())
}

/// Record the start of a network attempt.
fn record_attempt_start(
    db_path: Option<&Path>,
    operation_id: &str,
    generation: u32,
    route_index: u32,
    attempt: u32,
    route: &Route,
    transport: &str,
    queued_at: &str,
) -> Result<String> {
    let attempt_id = canonical_attempt_id(operation_id, generation, route_index, attempt);
    let Some(db_path) = db_path else {
        return Ok(attempt_id);
    };
    let store = Store::open(db_path)?;
    let now = now_rfc3339();
    store.conn.execute(
        "INSERT INTO recon_model_attempts (
           id, operation_id, generation, route_index, attempt,
           provider, account, model, transport, dispatched,
           outcome, queued_at, admitted_at, started_at, finished_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 1, 'running', ?10, ?11, ?11, '')
         ON CONFLICT(operation_id, generation, route_index, attempt) DO UPDATE SET
           outcome = 'running', queued_at = excluded.queued_at, admitted_at = excluded.admitted_at,
            started_at = excluded.started_at, finished_at = ''",
        rusqlite::params![
            attempt_id,
            operation_id,
            generation,
            route_index,
            attempt,
            route.provider,
            route.account,
            route.model,
            transport,
            queued_at,
            now,
        ],
    )?;
    Ok(attempt_id)
}

/// Record the completion of an attempt with its measured timings.
fn record_attempt_finish(
    db_path: Option<&Path>,
    attempt_id: &str,
    outcome: &AttemptOutcome,
    timings: &AttemptTimings,
) -> Result<()> {
    let Some(db_path) = db_path else {
        return Ok(());
    };
    let store = Store::open(db_path)?;
    let now = now_rfc3339();
    store.conn.execute(
        "UPDATE recon_model_attempts SET
           outcome = ?1,
           failure_category = ?2,
           http_status = ?3,
           request_id = ?4,
           finish_reason = ?5,
           char_count = ?6,
           wait_ms = ?7,
           error_message = ?8,
           finished_at = ?9,
           queued_at = ?10,
           admitted_at = ?11,
           sent_at = ?12,
           first_response_at = ?13,
           elapsed_ms = ?14,
           queue_ms = ?15,
           ttfb_ms = ?16,
           prompt_tokens = ?17,
           completion_tokens = ?18,
           stream_completed = ?19
         WHERE id = ?20",
        rusqlite::params![
            outcome.outcome,
            outcome.category.as_deref().unwrap_or(""),
            outcome.http_status,
            outcome.request_id.as_deref().unwrap_or(""),
            outcome.finish_reason.as_deref().unwrap_or(""),
            outcome.char_count,
            outcome.wait_ms,
            outcome.error_message.as_deref().unwrap_or(""),
            now,
            timings.queued_at,
            timings.admitted_at,
            timings.sent_at,
            timings.first_response_at,
            timings.elapsed_ms,
            timings.queue_ms,
            timings.ttfb_ms,
            timings.prompt_tokens,
            timings.completion_tokens,
            timings.stream_completed,
            attempt_id,
        ],
    )?;
    Ok(())
}

/// Execute a chat-based model role request across the configured route chain.
#[allow(clippy::too_many_arguments)]
pub async fn execute_chat<V>(
    auth: &AuthFile,
    settings: &SettingsFile,
    scope: &OperationScope<'_>,
    messages: &[ChatMessage],
    validator: V,
    cancel: &Arc<AtomicBool>,
    on_event: Option<Arc<dyn Fn(ModelExecEvent) + Send + Sync>>,
    text_observer: Option<Arc<dyn Fn(&str) + Send + Sync>>,
    db_path: Option<&Path>,
    stream: bool,
) -> Result<(String, ChainReport<String>)>
where
    V: Fn(&str) -> Result<(), String> + Send + Sync,
{
    let _ = ensure_operation(db_path, scope, "");
    let (routes, secrets) = resolve_role_routes(auth, settings, scope.role);

    let mut attempt_counter = 0u32;
    let mut last_route_idx = 0usize;
    let on_event_shared = on_event.clone();
    let op_id = scope.operation_id.to_string();
    let generation = scope.generation;
    let db_path_owned = db_path.map(Path::to_path_buf);
    let validator = Arc::new(validator);
    let role = scope.role.to_string();
    let run_id = scope.run_id.unwrap_or_default().to_string();
    let queued_at = now_rfc3339();
    let queued_instant = Instant::now();

    let dispatch = |route_idx: usize, route: &Route| {
        last_route_idx = route_idx;
        attempt_counter += 1;
        let attempt_num = attempt_counter;
        let secret = secrets[route_idx].clone();
        let transport = if stream && !provider_attempt::is_subscription(&secret) {
            Transport::Stream
        } else {
            Transport::NonStream
        };
        let messages_vec = messages.to_vec();
        let text_obs = text_observer.clone();
        let val = Arc::clone(&validator);
        let on_event_step = on_event_shared.clone();
        let route_cloned = route.clone();
        let op_id = op_id.clone();
        let db_path_buf = db_path_owned.clone();
        let role = role.clone();
        let run_id = run_id.clone();
        let queued_at = queued_at.clone();

        async move {
            let attempt_db_id = record_attempt_start(
                db_path_buf.as_deref(),
                &op_id,
                generation,
                route_idx as u32,
                attempt_num,
                &route_cloned,
                transport.as_str(),
                &queued_at,
            )
            .unwrap_or_default();
            let facts = AttemptFacts::new(
                &op_id,
                generation,
                &role,
                &run_id,
                &route_cloned,
                transport.as_str(),
                route_idx as u32,
                attempt_num,
            );

            // Notify attempt start
            if let Some(sink) = &on_event_step {
                sink(ModelExecEvent::AttemptStart {
                    operation_id: op_id.clone(),
                    generation,
                    route_index: route_idx as u32,
                    attempt: attempt_num,
                    provider: route_cloned.provider.clone(),
                    account: route_cloned.account.clone(),
                    model: route_cloned.model.clone(),
                    transport: transport.as_str().into(),
                });
            }

            let start_t = Instant::now();
            let mut timings = AttemptTimings::queued(&queued_at, queued_instant);
            timings.admitted();
            let streamed_acc = Arc::new(std::sync::Mutex::new(String::new()));
            let streamed_for_obs = streamed_acc.clone();
            let obs_db = db_path_buf.clone();
            let obs_op_id = op_id.clone();
            let obs_wrapper = text_obs.clone().map(|cb| {
                Arc::new(move |delta: &str| {
                    let text = if let Ok(mut acc) = streamed_for_obs.lock() {
                        acc.push_str(delta);
                        acc.clone()
                    } else {
                        String::new()
                    };
                    if !text.is_empty() {
                        let _ = save_operation_draft(obs_db.as_deref(), &obs_op_id, &text);
                    }
                    cb(delta);
                })
            });
            // First-response timing is captured on success as well as on failure:
            // the adapter only reports a latency when the attempt fails.
            let first_at: Arc<std::sync::Mutex<Option<(String, Instant)>>> =
                Arc::new(std::sync::Mutex::new(None));
            let observer = {
                let first_at = first_at.clone();
                let inner = obs_wrapper.clone();
                Arc::new(move |delta: &str| {
                    if let Ok(mut slot) = first_at.lock() {
                        if slot.is_none() {
                            *slot = Some((now_rfc3339(), Instant::now()));
                        }
                    }
                    if let Some(inner) = &inner {
                        inner(delta);
                    }
                }) as Arc<dyn Fn(&str) + Send + Sync>
            };

            timings.sent();
            let rep = provider_attempt::attempt_with_observer(
                &secret,
                &messages_vec,
                transport,
                Deadlines::default(),
                Some(observer.as_ref() as &(dyn Fn(&str) + Send + Sync)),
            )
            .await;

            if let Ok(slot) = first_at.lock() {
                if let Some((at, instant)) = slot.as_ref() {
                    timings.first_response(at, *instant);
                }
            }
            let elapsed_ms = start_t.elapsed().as_millis() as u64;
            let partial = streamed_acc.lock().map(|s| s.clone()).unwrap_or_default();
            if !partial.is_empty() {
                let _ = save_operation_draft(db_path_buf.as_deref(), &op_id, &partial);
            }

            match rep.outcome {
                Ok(completion) => {
                    let text = completion.content;
                    let finish_reason = completion.finish_reason.clone();
                    let chars = text.chars().count() as u64;

                    // Validate output
                    if text.trim().is_empty() {
                        let f = ProviderFailure::new(
                            Stage::Validation,
                            Category::Empty,
                            "empty completion output",
                        );
                        let outcome = AttemptOutcome::failed(
                            "fail",
                            "empty",
                            None,
                            None,
                            finish_reason.as_deref(),
                            chars,
                            elapsed_ms,
                            "empty completion",
                        );
                        finish_attempt(
                            db_path_buf.as_deref(),
                            &attempt_db_id,
                            &facts,
                            &outcome,
                            &mut timings,
                            elapsed_ms,
                            false,
                        );
                        if let Some(sink) = &on_event_step {
                            sink(ModelExecEvent::AttemptReset {
                                operation_id: op_id.clone(),
                                attempt: attempt_num,
                            });
                            sink(ModelExecEvent::AttemptFinish {
                                operation_id: op_id,
                                attempt: attempt_num,
                                outcome: "fail".into(),
                                category: Some("empty".into()),
                                finish_reason,
                                chars,
                            });
                        }
                        return Err(DispatchError::from_failure(f));
                    }

                    if let Err(val_err) = val(&text) {
                        let mut f = ProviderFailure::new(
                            Stage::Validation,
                            Category::InvalidResult,
                            format!("validation failed: {val_err}"),
                        );
                        f.causes.push(val_err.clone());
                        let outcome = AttemptOutcome::failed(
                            "fail",
                            "invalid_result",
                            None,
                            None,
                            finish_reason.as_deref(),
                            chars,
                            elapsed_ms,
                            &val_err,
                        );
                        finish_attempt(
                            db_path_buf.as_deref(),
                            &attempt_db_id,
                            &facts,
                            &outcome,
                            &mut timings,
                            elapsed_ms,
                            false,
                        );
                        if let Some(sink) = &on_event_step {
                            sink(ModelExecEvent::AttemptReset {
                                operation_id: op_id.clone(),
                                attempt: attempt_num,
                            });
                            sink(ModelExecEvent::AttemptFinish {
                                operation_id: op_id,
                                attempt: attempt_num,
                                outcome: "fail".into(),
                                category: Some("invalid_result".into()),
                                finish_reason,
                                chars,
                            });
                        }
                        return Err(DispatchError::from_failure(f));
                    }

                    // Success!
                    let outcome = AttemptOutcome::ok(finish_reason.as_deref(), chars, elapsed_ms);
                    finish_attempt(
                        db_path_buf.as_deref(),
                        &attempt_db_id,
                        &facts,
                        &outcome,
                        &mut timings,
                        elapsed_ms,
                        matches!(transport, Transport::Stream) && finish_reason.is_some(),
                    );
                    if let Some(sink) = &on_event_step {
                        sink(ModelExecEvent::AttemptFinish {
                            operation_id: op_id.clone(),
                            attempt: attempt_num,
                            outcome: "ok".into(),
                            category: None,
                            finish_reason,
                            chars,
                        });
                        sink(ModelExecEvent::FinalReplacement {
                            operation_id: op_id,
                            text: text.clone(),
                        });
                    }
                    Ok(text)
                }
                Err(failure) => {
                    let cat = failure.category.as_str().to_string();
                    let finish_r = failure.stream.finish_reason.clone();
                    let chars = failure.stream.partial_chars;
                    let msg = failure.summary();
                    let err = DispatchError::from_failure(failure.clone());
                    timings.reported_ttfb_ms(failure.first_response_ms);
                    let outcome = AttemptOutcome::failed(
                        if err.cancelled { "cancel" } else { "fail" },
                        &cat,
                        failure.http_status,
                        failure.request_id.as_deref(),
                        finish_r.as_deref(),
                        chars,
                        elapsed_ms,
                        &msg,
                    );
                    finish_attempt(
                        db_path_buf.as_deref(),
                        &attempt_db_id,
                        &facts,
                        &outcome,
                        &mut timings,
                        elapsed_ms,
                        failure.stream.done_marker,
                    );

                    if let Some(sink) = &on_event_step {
                        sink(ModelExecEvent::AttemptReset {
                            operation_id: op_id.clone(),
                            attempt: attempt_num,
                        });
                        sink(ModelExecEvent::AttemptFinish {
                            operation_id: op_id,
                            attempt: attempt_num,
                            outcome: if err.cancelled {
                                "cancel".into()
                            } else {
                                "fail".into()
                            },
                            category: Some(cat),
                            finish_reason: finish_r,
                            chars,
                        });
                    }

                    Err(err)
                }
            }
        }
    };

    let opts = ExecuteOptions {
        instant: cfg!(test),
        cancel: Some(cancel.clone()),
        recorded_waits: None,
        admission_account: String::new(),
    };

    let report = provider_chain::execute(&routes, dispatch, opts).await;

    // One terminal row per logical operation: its many wire attempts are
    // attempts, never extra jobs.
    let terminal_status = if report.value.is_some() {
        "final"
    } else if report.cancelled {
        "cancelled"
    } else {
        "failed"
    };
    note_operation(
        db_path,
        scope,
        terminal_status,
        &routes,
        &report.history,
        report.effective_route,
        &report.effective_model,
    );

    if let Some(value) = report.value.clone() {
        let _ = save_operation_draft(db_path, scope.operation_id, &value);
        Ok((value, report))
    } else if report.cancelled {
        let _ = update_operation_status(db_path, scope.operation_id, "cancelled", None);
        Err(anyhow!("operation cancelled"))
    } else if report.blocked {
        let _ = update_operation_status(db_path, scope.operation_id, "failed", None);
        Err(anyhow!("all routes blocked: {}", report.failure_message()))
    } else {
        let _ = update_operation_status(db_path, scope.operation_id, "failed", None);
        Err(anyhow!(
            "model chain exhausted: {}",
            report.failure_message()
        ))
    }
}

/// Decisions compiler abstraction for Jev or chat fallback.
pub trait DecisionsAdapter<T>: Send + Sync {
    fn compile_native(&self) -> (Value, Value);
    fn parse_native(&self, resp: &provider::DecisionsResponse) -> Result<T, String>;
    fn compile_chat(&self) -> Vec<ChatMessage>;
    fn parse_chat(&self, raw: &str) -> Result<T, String>;
}

/// Execute a decision or tool picker request that supports native Decisions (Jev) with Chat fallback.
pub async fn execute_decisions_or_chat<T, A>(
    auth: &AuthFile,
    settings: &SettingsFile,
    scope: &OperationScope<'_>,
    adapter: &A,
    cancel: &Arc<AtomicBool>,
    on_event: Option<Arc<dyn Fn(ModelExecEvent) + Send + Sync>>,
    db_path: Option<&Path>,
) -> Result<(T, ChainReport<T>)>
where
    T: Clone + Send + 'static,
    A: DecisionsAdapter<T>,
{
    let _ = ensure_operation(db_path, scope, "");
    let (routes, secrets) = resolve_role_routes(auth, settings, scope.role);

    let mut attempt_counter = 0u32;
    let on_event_shared = on_event.clone();
    let op_id = scope.operation_id.to_string();
    let generation = scope.generation;
    let db_path_owned = db_path.map(Path::to_path_buf);
    let role = scope.role.to_string();
    let run_id = scope.run_id.unwrap_or_default().to_string();
    let queued_at = now_rfc3339();
    let queued_instant = Instant::now();

    let dispatch = |route_idx: usize, route: &Route| {
        attempt_counter += 1;
        let attempt_num = attempt_counter;
        let secret = secrets[route_idx].clone();
        let is_native = provider::is_decisions_model(&secret.model)
            && provider::effective_kind(&secret) == "openrouter"
            && provider::resolved_key(&secret).is_some();
        let transport = if is_native { "decisions" } else { "chat" };
        let on_event_step = on_event_shared.clone();
        let route_cloned = route.clone();
        let op_id = op_id.clone();
        let db_path_buf = db_path_owned.clone();
        let role = role.clone();
        let run_id = run_id.clone();
        let queued_at = queued_at.clone();

        async move {
            let attempt_db_id = record_attempt_start(
                db_path_buf.as_deref(),
                &op_id,
                generation,
                route_idx as u32,
                attempt_num,
                &route_cloned,
                transport,
                &queued_at,
            )
            .unwrap_or_default();
            let facts = AttemptFacts::new(
                &op_id,
                generation,
                &role,
                &run_id,
                &route_cloned,
                transport,
                route_idx as u32,
                attempt_num,
            );

            if let Some(sink) = &on_event_step {
                sink(ModelExecEvent::AttemptStart {
                    operation_id: op_id.clone(),
                    generation,
                    route_index: route_idx as u32,
                    attempt: attempt_num,
                    provider: route_cloned.provider.clone(),
                    account: route_cloned.account.clone(),
                    model: route_cloned.model.clone(),
                    transport: transport.into(),
                });
            }

            let start_t = Instant::now();
            let mut timings = AttemptTimings::queued(&queued_at, queued_instant);
            timings.admitted();
            timings.sent();

            if is_native {
                let (native_state, native_questions) = adapter.compile_native();
                match provider::decide(&secret, &native_state, &native_questions).await {
                    Ok(resp) => {
                        let elapsed_ms = start_t.elapsed().as_millis() as u64;
                        match adapter.parse_native(&resp) {
                            Ok(val) => {
                                let outcome = AttemptOutcome::ok(None, 0, elapsed_ms);
                                finish_attempt(
                                    db_path_buf.as_deref(),
                                    &attempt_db_id,
                                    &facts,
                                    &outcome,
                                    &mut timings,
                                    elapsed_ms,
                                    false,
                                );
                                if let Some(sink) = &on_event_step {
                                    sink(ModelExecEvent::AttemptFinish {
                                        operation_id: op_id,
                                        attempt: attempt_num,
                                        outcome: "ok".into(),
                                        category: None,
                                        finish_reason: None,
                                        chars: 0,
                                    });
                                }
                                Ok(val)
                            }
                            Err(parse_err) => {
                                let f = ProviderFailure::new(
                                    Stage::Parse,
                                    Category::MalformedPayload,
                                    format!("failed to parse Jev decision: {parse_err}"),
                                );
                                let outcome = AttemptOutcome::failed(
                                    "fail",
                                    "malformed_payload",
                                    None,
                                    None,
                                    None,
                                    0,
                                    elapsed_ms,
                                    &parse_err,
                                );
                                finish_attempt(
                                    db_path_buf.as_deref(),
                                    &attempt_db_id,
                                    &facts,
                                    &outcome,
                                    &mut timings,
                                    elapsed_ms,
                                    false,
                                );
                                if let Some(sink) = &on_event_step {
                                    sink(ModelExecEvent::AttemptReset {
                                        operation_id: op_id.clone(),
                                        attempt: attempt_num,
                                    });
                                    sink(ModelExecEvent::AttemptFinish {
                                        operation_id: op_id,
                                        attempt: attempt_num,
                                        outcome: "fail".into(),
                                        category: Some("malformed_payload".into()),
                                        finish_reason: None,
                                        chars: 0,
                                    });
                                }
                                Err(DispatchError::from_failure(f))
                            }
                        }
                    }
                    Err(decide_err) => {
                        let elapsed_ms = start_t.elapsed().as_millis() as u64;
                        let f = ProviderFailure::from_anyhow(Stage::Response, &decide_err);
                        let err = DispatchError::from_failure(f.clone());
                        timings.reported_ttfb_ms(f.first_response_ms);
                        let outcome = AttemptOutcome::failed(
                            if err.cancelled { "cancel" } else { "fail" },
                            f.category.as_str(),
                            f.http_status,
                            f.request_id.as_deref(),
                            None,
                            0,
                            elapsed_ms,
                            &f.summary(),
                        );
                        finish_attempt(
                            db_path_buf.as_deref(),
                            &attempt_db_id,
                            &facts,
                            &outcome,
                            &mut timings,
                            elapsed_ms,
                            false,
                        );
                        if let Some(sink) = &on_event_step {
                            sink(ModelExecEvent::AttemptReset {
                                operation_id: op_id.clone(),
                                attempt: attempt_num,
                            });
                            sink(ModelExecEvent::AttemptFinish {
                                operation_id: op_id,
                                attempt: attempt_num,
                                outcome: if err.cancelled {
                                    "cancel".into()
                                } else {
                                    "fail".into()
                                },
                                category: Some(f.category.as_str().into()),
                                finish_reason: None,
                                chars: 0,
                            });
                        }
                        Err(err)
                    }
                }
            } else {
                // Chat fallback: re-compile for chat
                let chat_messages = adapter.compile_chat();
                let rep = provider_attempt::attempt(
                    &secret,
                    &chat_messages,
                    Transport::NonStream,
                    Deadlines::default(),
                )
                .await;

                let elapsed_ms = start_t.elapsed().as_millis() as u64;

                match rep.outcome {
                    Ok(completion) => {
                        let chars = completion.content.chars().count() as u64;
                        match adapter.parse_chat(&completion.content) {
                            Ok(val) => {
                                let outcome = AttemptOutcome::ok(
                                    completion.finish_reason.as_deref(),
                                    chars,
                                    elapsed_ms,
                                );
                                finish_attempt(
                                    db_path_buf.as_deref(),
                                    &attempt_db_id,
                                    &facts,
                                    &outcome,
                                    &mut timings,
                                    elapsed_ms,
                                    false,
                                );
                                if let Some(sink) = &on_event_step {
                                    sink(ModelExecEvent::AttemptFinish {
                                        operation_id: op_id,
                                        attempt: attempt_num,
                                        outcome: "ok".into(),
                                        category: None,
                                        finish_reason: completion.finish_reason,
                                        chars,
                                    });
                                }
                                Ok(val)
                            }
                            Err(parse_err) => {
                                let f = ProviderFailure::new(
                                    Stage::Parse,
                                    Category::MalformedPayload,
                                    format!("failed to parse chat decision: {parse_err}"),
                                );
                                let outcome = AttemptOutcome::failed(
                                    "fail",
                                    "malformed_payload",
                                    None,
                                    None,
                                    completion.finish_reason.as_deref(),
                                    chars,
                                    elapsed_ms,
                                    &parse_err,
                                );
                                finish_attempt(
                                    db_path_buf.as_deref(),
                                    &attempt_db_id,
                                    &facts,
                                    &outcome,
                                    &mut timings,
                                    elapsed_ms,
                                    false,
                                );
                                if let Some(sink) = &on_event_step {
                                    sink(ModelExecEvent::AttemptReset {
                                        operation_id: op_id.clone(),
                                        attempt: attempt_num,
                                    });
                                    sink(ModelExecEvent::AttemptFinish {
                                        operation_id: op_id,
                                        attempt: attempt_num,
                                        outcome: "fail".into(),
                                        category: Some("malformed_payload".into()),
                                        finish_reason: completion.finish_reason,
                                        chars,
                                    });
                                }
                                Err(DispatchError::from_failure(f))
                            }
                        }
                    }
                    Err(failure) => {
                        let cat = failure.category.as_str().to_string();
                        let finish_r = failure.stream.finish_reason.clone();
                        let chars = failure.stream.partial_chars;
                        let msg = failure.summary();
                        let err = DispatchError::from_failure(failure.clone());
                        timings.reported_ttfb_ms(failure.first_response_ms);

                        let outcome = AttemptOutcome::failed(
                            if err.cancelled { "cancel" } else { "fail" },
                            &cat,
                            failure.http_status,
                            failure.request_id.as_deref(),
                            finish_r.as_deref(),
                            chars,
                            elapsed_ms,
                            &msg,
                        );
                        finish_attempt(
                            db_path_buf.as_deref(),
                            &attempt_db_id,
                            &facts,
                            &outcome,
                            &mut timings,
                            elapsed_ms,
                            failure.stream.done_marker,
                        );

                        if let Some(sink) = &on_event_step {
                            sink(ModelExecEvent::AttemptReset {
                                operation_id: op_id.clone(),
                                attempt: attempt_num,
                            });
                            sink(ModelExecEvent::AttemptFinish {
                                operation_id: op_id,
                                attempt: attempt_num,
                                outcome: if err.cancelled {
                                    "cancel".into()
                                } else {
                                    "fail".into()
                                },
                                category: Some(cat),
                                finish_reason: finish_r,
                                chars,
                            });
                        }
                        Err(err)
                    }
                }
            }
        }
    };

    let opts = ExecuteOptions {
        instant: cfg!(test),
        cancel: Some(cancel.clone()),
        recorded_waits: None,
        admission_account: String::new(),
    };

    let report = provider_chain::execute(&routes, dispatch, opts).await;

    // One terminal row per logical operation: its many wire attempts are
    // attempts, never extra jobs.
    let terminal_status = if report.value.is_some() {
        "final"
    } else if report.cancelled {
        "cancelled"
    } else {
        "failed"
    };
    note_operation(
        db_path,
        scope,
        terminal_status,
        &routes,
        &report.history,
        report.effective_route,
        &report.effective_model,
    );

    if let Some(value) = report.value.clone() {
        Ok((value, report))
    } else if report.cancelled {
        let _ = update_operation_status(db_path, scope.operation_id, "cancelled", None);
        Err(anyhow!("operation cancelled"))
    } else if report.blocked {
        let _ = update_operation_status(db_path, scope.operation_id, "failed", None);
        Err(anyhow!("all routes blocked: {}", report.failure_message()))
    } else {
        let _ = update_operation_status(db_path, scope.operation_id, "failed", None);
        Err(anyhow!(
            "model chain exhausted: {}",
            report.failure_message()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_attempt_id_is_stable_across_replay() {
        let first = canonical_attempt_id("run-1-synthesis", 2, 1, 3);
        assert_eq!(first, canonical_attempt_id("run-1-synthesis", 2, 1, 3));
        assert_eq!(first, "run-1-synthesis-2-1-3");
        // A different route, attempt or generation is a different wire attempt.
        assert_ne!(first, canonical_attempt_id("run-1-synthesis", 2, 2, 3));
        assert_ne!(first, canonical_attempt_id("run-1-synthesis", 2, 1, 4));
        assert_ne!(first, canonical_attempt_id("run-1-synthesis", 3, 1, 3));
    }

    #[test]
    fn measured_timings_are_monotonic_and_never_negative() {
        let queued_at = now_rfc3339();
        let queued_instant = Instant::now();
        std::thread::sleep(Duration::from_millis(5));
        let mut timings = AttemptTimings::queued(&queued_at, queued_instant);
        timings.admitted();
        assert!(!timings.admitted_at.is_empty());
        std::thread::sleep(Duration::from_millis(5));
        timings.sent();
        assert!(!timings.sent_at.is_empty());
        // First response is captured on success as well as on failure.
        timings.first_response(&now_rfc3339(), Instant::now());
        std::thread::sleep(Duration::from_millis(2));
        timings.reported_ttfb_ms(Some(999));
        timings.finish(42, true);

        assert_eq!(timings.elapsed_ms, 42);
        assert_eq!(timings.stream_completed, 1);
        assert!(timings.queue_ms >= 10, "{timings:?}");
        assert!(
            timings.ttfb_ms < 999,
            "observed first response wins: {timings:?}"
        );
        assert!(!timings.first_response_at.is_empty());
    }

    #[test]
    fn adapter_reported_first_response_is_kept_when_no_chunk_arrived() {
        let mut timings = AttemptTimings::queued(&now_rfc3339(), Instant::now());
        timings.sent();
        timings.reported_ttfb_ms(Some(640));
        timings.finish(800, false);
        assert_eq!(timings.ttfb_ms, 640);
        assert_eq!(
            timings.queue_ms, 0,
            "no send stamp means no queue residence"
        );
        assert_eq!(timings.prompt_tokens, None);
        assert_eq!(timings.completion_tokens, None);
    }

    #[test]
    fn attempt_outcome_keeps_the_failure_category_for_the_reason_dimension() {
        let outcome = AttemptOutcome::failed(
            "fail",
            "rate_limit",
            Some(429),
            Some("req-1"),
            Some("stop"),
            12,
            30,
            "slow down",
        );
        assert_eq!(outcome.outcome, "fail");
        assert_eq!(outcome.category.as_deref(), Some("rate_limit"));
        assert_eq!(outcome.http_status, Some(429));
        assert_eq!(outcome.wait_ms, 30);
        assert_eq!(AttemptOutcome::ok(Some("stop"), 4, 7).outcome, "ok");
        assert!(AttemptOutcome::default().category.is_none());
    }
}
