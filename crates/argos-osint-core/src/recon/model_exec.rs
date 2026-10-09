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
use serde_json::Value;

use crate::provider::{self, ChatMessage, SettingsFile};
use crate::provider_attempt::{self, Deadlines, Transport};
use crate::provider_chain::{self, ChainReport, DispatchError, ExecuteOptions, Route};
use crate::provider_diag::{Category, ProviderFailure, Stage};
use crate::secrets::{AuthFile, ProviderSecret};
use crate::store::Store;

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
) -> Result<String> {
    let attempt_id = format!(
        "{}-{}-{}-{}",
        operation_id, generation, route_index, attempt
    );
    let Some(db_path) = db_path else {
        return Ok(attempt_id);
    };
    let store = Store::open(db_path)?;
    let now = now_rfc3339();
    store.conn.execute(
        "INSERT INTO recon_model_attempts (
           id, operation_id, generation, route_index, attempt,
           provider, account, model, transport, dispatched,
           outcome, started_at, finished_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 1, 'running', ?10, '')
         ON CONFLICT(operation_id, generation, route_index, attempt) DO UPDATE SET
           outcome = 'running', started_at = excluded.started_at, finished_at = ''",
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
            now,
        ],
    )?;
    Ok(attempt_id)
}

/// Record the completion of an attempt.
fn record_attempt_finish(
    db_path: Option<&Path>,
    attempt_id: &str,
    outcome: &str,
    category: Option<&str>,
    http_status: Option<u16>,
    request_id: Option<&str>,
    finish_reason: Option<&str>,
    char_count: u64,
    wait_ms: u64,
    error_message: Option<&str>,
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
           finished_at = ?9
         WHERE id = ?10",
        rusqlite::params![
            outcome,
            category.unwrap_or(""),
            http_status,
            request_id.unwrap_or(""),
            finish_reason.unwrap_or(""),
            char_count,
            wait_ms,
            error_message.unwrap_or(""),
            now,
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

        async move {
            let attempt_db_id = record_attempt_start(
                db_path_buf.as_deref(),
                &op_id,
                generation,
                route_idx as u32,
                attempt_num,
                &route_cloned,
                transport.as_str(),
            )
            .unwrap_or_default();

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

            let rep = provider_attempt::attempt_with_observer(
                &secret,
                &messages_vec,
                transport,
                Deadlines::default(),
                obs_wrapper
                    .as_ref()
                    .map(|o| o.as_ref() as &(dyn Fn(&str) + Send + Sync)),
            )
            .await;

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
                        let _ = record_attempt_finish(
                            db_path_buf.as_deref(),
                            &attempt_db_id,
                            "fail",
                            Some("empty"),
                            None,
                            None,
                            finish_reason.as_deref(),
                            chars,
                            elapsed_ms,
                            Some("empty completion"),
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
                        let _ = record_attempt_finish(
                            db_path_buf.as_deref(),
                            &attempt_db_id,
                            "fail",
                            Some("invalid_result"),
                            None,
                            None,
                            finish_reason.as_deref(),
                            chars,
                            elapsed_ms,
                            Some(&val_err),
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
                    let _ = record_attempt_finish(
                        db_path_buf.as_deref(),
                        &attempt_db_id,
                        "ok",
                        None,
                        None,
                        None,
                        finish_reason.as_deref(),
                        chars,
                        elapsed_ms,
                        None,
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

                    let _ = record_attempt_finish(
                        db_path_buf.as_deref(),
                        &attempt_db_id,
                        if err.cancelled { "cancel" } else { "fail" },
                        Some(&cat),
                        failure.http_status,
                        failure.request_id.as_deref(),
                        finish_r.as_deref(),
                        chars,
                        elapsed_ms,
                        Some(&msg),
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

        async move {
            let attempt_db_id = record_attempt_start(
                db_path_buf.as_deref(),
                &op_id,
                generation,
                route_idx as u32,
                attempt_num,
                &route_cloned,
                transport,
            )
            .unwrap_or_default();

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

            if is_native {
                let (native_state, native_questions) = adapter.compile_native();
                match provider::decide(&secret, &native_state, &native_questions).await {
                    Ok(resp) => {
                        let elapsed_ms = start_t.elapsed().as_millis() as u64;
                        match adapter.parse_native(&resp) {
                            Ok(val) => {
                                let _ = record_attempt_finish(
                                    db_path_buf.as_deref(),
                                    &attempt_db_id,
                                    "ok",
                                    None,
                                    None,
                                    None,
                                    None,
                                    0,
                                    elapsed_ms,
                                    None,
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
                                let _ = record_attempt_finish(
                                    db_path_buf.as_deref(),
                                    &attempt_db_id,
                                    "fail",
                                    Some("malformed_payload"),
                                    None,
                                    None,
                                    None,
                                    0,
                                    elapsed_ms,
                                    Some(&parse_err),
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
                        let _ = record_attempt_finish(
                            db_path_buf.as_deref(),
                            &attempt_db_id,
                            if err.cancelled { "cancel" } else { "fail" },
                            Some(f.category.as_str()),
                            f.http_status,
                            f.request_id.as_deref(),
                            None,
                            0,
                            elapsed_ms,
                            Some(&f.summary()),
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
                                let _ = record_attempt_finish(
                                    db_path_buf.as_deref(),
                                    &attempt_db_id,
                                    "ok",
                                    None,
                                    None,
                                    None,
                                    completion.finish_reason.as_deref(),
                                    chars,
                                    elapsed_ms,
                                    None,
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
                                let _ = record_attempt_finish(
                                    db_path_buf.as_deref(),
                                    &attempt_db_id,
                                    "fail",
                                    Some("malformed_payload"),
                                    None,
                                    None,
                                    completion.finish_reason.as_deref(),
                                    chars,
                                    elapsed_ms,
                                    Some(&parse_err),
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

                        let _ = record_attempt_finish(
                            db_path_buf.as_deref(),
                            &attempt_db_id,
                            if err.cancelled { "cancel" } else { "fail" },
                            Some(&cat),
                            failure.http_status,
                            failure.request_id.as_deref(),
                            finish_r.as_deref(),
                            chars,
                            elapsed_ms,
                            Some(&msg),
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
