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

use anyhow::Result;
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
    /// Insight-only bounded repair/fallback policy; other operations retain their policy.
    pub insight_contract: bool,
    /// Summarization-role structured operation: honors `contract`, uses the
    /// Summarization attempt cap and timeout profile, and is never labeled
    /// insight extraction.
    pub summarization: bool,
    /// Optional explicit completion ceiling. Automatic operations leave it unset.
    pub completion_tokens: Option<u32>,
    /// Requested reasoning effort for this operation (provider default if unset).
    pub reasoning: crate::request_policy::ReasoningEffort,
    /// Operation key (e.g. `intel_brief_links.generate`) for route overrides.
    pub operation_key: Option<&'a str>,
    /// Declared response format of this dispatch.
    pub response_format: ResponseFormat,
    /// Input revision the dispatch was compiled from (cache/audit identity).
    pub input_revision: Option<&'a str>,
    /// A user is waiting on this result (admission priority, no preemption).
    pub interactive: bool,
    /// Legacy stage sizing hints; automatic stages use provider output defaults.
    pub budget: Option<crate::request_policy::StageBudget>,
    /// Authoritative response contract (schema for strict/native/output tool).
    pub contract: Option<crate::provider_adapters::ResponseContract>,
    /// Lease epoch of the owning task (schema 41); `None` outside tasks.
    pub task_epoch: Option<i64>,
    /// Stage-wide request budget and route health shared with sibling
    /// operations (`None` outside stages).
    pub ledger: Option<std::sync::Arc<crate::provider_chain::StageLedger>>,
}

/// Declared response format (spec §3). `BoundedDecision` may use native Jev
/// or constrained chat; `GeneratedJson`/`Prose` skip Decisions-only routes
/// without spending an attempt.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseFormat {
    BoundedDecision,
    #[default]
    GeneratedJson,
    Prose,
}

/// The declaration every dispatch carries, recorded for audit.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct DispatchDeclaration {
    pub logical_role: String,
    pub operation_key: String,
    pub response_format: ResponseFormat,
    pub input_revision: String,
    pub request_policy: crate::request_policy::RequestPolicy,
    /// True when an operation override supplied the route chain.
    pub overridden: bool,
    /// Capability digest per route, frozen at stage admission (spec §1).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capability_digests: Vec<String>,
}

impl<'a> OperationScope<'a> {
    pub fn new(operation_id: &'a str, role: &'a str) -> Self {
        Self {
            operation_id,
            role,
            run_id: None,
            task_id: None,
            generation: 1,
            insight_contract: false,
            summarization: false,
            completion_tokens: None,
            reasoning: crate::request_policy::ReasoningEffort::Default,
            operation_key: None,
            response_format: ResponseFormat::GeneratedJson,
            input_revision: None,
            interactive: false,
            budget: None,
            contract: None,
            task_epoch: None,
            ledger: None,
        }
    }

    /// Retain stage sizing metadata without imposing output/reasoning limits.
    pub fn with_budget(mut self, budget: crate::request_policy::StageBudget) -> Self {
        self.budget = Some(budget);
        self
    }

    pub fn with_contract(mut self, contract: crate::provider_adapters::ResponseContract) -> Self {
        self.contract = Some(contract);
        self
    }

    /// Per-attempt deadlines: thinking stages get thinking-sized waits.
    pub fn deadlines(&self, stream: bool) -> Deadlines {
        let thinks = self.budget.is_some_and(|b| b.thinks());
        match (stream, thinks) {
            (true, true) => Deadlines::insight_stream_thinking(),
            (true, false) => Deadlines::insight_stream(),
            (false, true) => Deadlines::insight_non_stream_thinking(),
            (false, false) => Deadlines::insight_non_stream(),
        }
    }

    /// The request policy for this operation's dispatches.
    pub fn request_policy(
        &self,
        json_object: bool,
        _default_tokens: u32,
    ) -> crate::request_policy::RequestPolicy {
        match self.budget {
            Some(budget) => budget.policy(json_object),
            None => crate::request_policy::RequestPolicy {
                max_output_tokens: self.completion_tokens,
                json_object,
                reasoning: self.reasoning,
                ..Default::default()
            },
        }
    }

    /// Wire budget: interactive bounded operations use the §7 operation
    /// budget; everyone else keeps the ordinary policy.
    pub fn wire_budget(&self) -> Option<provider_chain::WireBudget> {
        self.interactive
            .then_some(if self.budget.is_some_and(|b| b.thinks()) {
                provider_chain::WireBudget::INTERACTIVE_THINKING
            } else {
                provider_chain::WireBudget::INTERACTIVE
            })
    }

    pub fn with_interactive(mut self) -> Self {
        self.interactive = true;
        self
    }

    pub fn with_operation(mut self, key: &'a str) -> Self {
        self.operation_key = Some(key);
        self
    }

    pub fn with_format(mut self, format: ResponseFormat) -> Self {
        self.response_format = format;
        self
    }

    pub fn with_input_revision(mut self, revision: &'a str) -> Self {
        self.input_revision = Some(revision);
        self
    }

    /// The dispatch declaration for `policy`.
    pub fn declaration(
        &self,
        settings: &SettingsFile,
        policy: crate::request_policy::RequestPolicy,
    ) -> DispatchDeclaration {
        let canonical = provider::role_name(self.role).unwrap_or(self.role);
        DispatchDeclaration {
            logical_role: canonical.to_string(),
            operation_key: self.operation_key.unwrap_or("").to_string(),
            response_format: self.response_format,
            input_revision: self.input_revision.unwrap_or("").to_string(),
            request_policy: policy,
            overridden: settings
                .defaults
                .resolve_operation(self.operation_key, canonical)
                .1,
            capability_digests: Vec::new(),
        }
    }

    pub fn with_reasoning(mut self, effort: crate::request_policy::ReasoningEffort) -> Self {
        self.reasoning = effort;
        self
    }

    pub fn with_run(mut self, run_id: &'a str) -> Self {
        self.run_id = Some(run_id);
        self
    }

    pub fn with_task(mut self, task_id: &'a str) -> Self {
        self.task_id = Some(task_id);
        self
    }

    /// Bind the operation to a task lease epoch (ownership/telemetry). The
    /// caller also puts the epoch in the operation id
    /// (`{task_id}:e{epoch}:...`), so a retried execution writes new attempt
    /// rows and prior rows stay immutable.
    pub fn with_ledger(
        mut self,
        ledger: std::sync::Arc<crate::provider_chain::StageLedger>,
    ) -> Self {
        self.ledger = Some(ledger);
        self
    }

    pub fn with_task_epoch(mut self, epoch: i64) -> Self {
        self.task_epoch = Some(epoch);
        self
    }

    pub fn with_generation(mut self, generation: u32) -> Self {
        self.generation = generation;
        self
    }

    pub fn with_completion_tokens(mut self, tokens: u32) -> Self {
        self.completion_tokens = Some(tokens);
        self
    }

    /// Mark a structured Summarization operation (see [`OperationScope::summarization`]).
    pub fn for_summarization(
        mut self,
        contract: crate::provider_adapters::ResponseContract,
    ) -> Self {
        self.summarization = true;
        self.response_format = ResponseFormat::GeneratedJson;
        self.contract = Some(contract);
        self
    }

    pub fn with_insight_contract(mut self) -> Self {
        self.insight_contract = true;
        self
    }
}

/// An executor failure, distinct from local validation and persistence errors.
#[derive(Debug)]
pub struct ModelOperationError {
    pub category: Option<Category>,
    pub requests: u32,
    pub history: Vec<provider_chain::AttemptRecord>,
    pub message: String,
}

impl std::fmt::Display for ModelOperationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} ({} dispatched attempts): {}",
            self.message,
            self.requests,
            self.history
                .iter()
                .map(|r| format!("route {} {}: {}", r.route_index, r.outcome, r.reason))
                .collect::<Vec<_>>()
                .join("; ")
        )
    }
}
impl std::error::Error for ModelOperationError {}

impl ModelOperationError {
    pub fn can_withhold(&self) -> bool {
        !matches!(
            self.category,
            Some(
                Category::Cancelled
                    | Category::Refused
                    | Category::MalformedRequest
                    | Category::Persistence
            )
        )
    }
}

/// Bound one attempt by the operation's remaining wall-clock cap, covering
/// provider pacing and every per-attempt wait. Expiry is a typed timeout the
/// caller records like any other attempt failure.
async fn bounded_by_cap(
    remaining: Option<std::time::Duration>,
    secret: &crate::secrets::ProviderSecret,
    transport: Transport,
    attempt: impl std::future::Future<Output = provider_attempt::AttemptReport>,
) -> provider_attempt::AttemptReport {
    let Some(left) = remaining else {
        return attempt.await;
    };
    let started = Instant::now();
    match tokio::time::timeout(left, attempt).await {
        Ok(report) => report,
        Err(_) => provider_attempt::AttemptReport {
            transport,
            endpoint: provider::normalize_base(&secret.base_url),
            elapsed_ms: started.elapsed().as_millis() as u64,
            outcome: Err(ProviderFailure::new(
                Stage::Admission,
                Category::Timeout,
                format!(
                    "operation wall-clock cap reached while waiting on this route ({}s left of the operation budget)",
                    left.as_secs()
                ),
            )),
            policy_diagnostics: Vec::new(),
            effective: None,
            recovery: None,
        },
    }
}

fn chain_error<T>(report: &ChainReport<T>) -> anyhow::Error {
    let termination = if report.cancelled {
        "operation cancelled"
    } else if report.budget_exhausted {
        "stage request budget reached"
    } else if report.wall_clock_exceeded {
        "operation wall-clock cap reached"
    } else if report.blocked {
        "all routes blocked"
    } else if report.stopped {
        "model operation stopped"
    } else {
        "model routes exhausted"
    };
    let message = report
        .terminal_failure
        .as_ref()
        .map(|failure| {
            format!(
                "{termination}: {}/{}: {}{}",
                failure.stage.label(),
                failure.category.as_str(),
                failure.message,
                failure
                    .request_id
                    .as_deref()
                    .map(|id| format!("; request {id}"))
                    .unwrap_or_default()
            )
        })
        .unwrap_or_else(|| termination.into());
    ModelOperationError {
        category: report.terminal_failure.as_ref().map(|f| f.category),
        requests: report.requests,
        history: report.history.clone(),
        message,
    }
    .into()
}

/// One repair per compatible route, counted as a wire attempt by the chain.
#[derive(Default)]
struct OutputRepairs(std::sync::Mutex<std::collections::BTreeMap<usize, String>>);

impl OutputRepairs {
    fn messages(&self, route: usize, messages: &[ChatMessage]) -> Vec<ChatMessage> {
        let mut messages = messages.to_vec();
        if let Some(error) = self.0.lock().unwrap().get(&route) {
            messages.push(ChatMessage {
                role: "user".into(),
                content: format!("Repair the previous output contract failure: {error}. Return the complete valid answer JSON using only supplied IDs and evidence. No explanation."),
                tool_call_id: None,
                tool_calls: vec![],
            });
        }
        messages
    }

    fn failure(&self, route: usize, error: String, stage: Stage) -> DispatchError {
        let mut errors = self.0.lock().unwrap();
        let repaired = errors
            .insert(
                route,
                crate::events::redact(&error).chars().take(512).collect(),
            )
            .is_some();
        let mut failure = DispatchError::from_failure(ProviderFailure::new(
            stage,
            Category::InvalidResult,
            error,
        ));
        if repaired {
            failure.disposition = provider_chain::RetryDisposition::NextRoute;
            failure
        } else {
            // One output repair per route: a recovery dispatch, no backoff.
            failure.with_recovery("output repair")
        }
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
    /// Typed transport/model activity for the current attempt. Carries counts
    /// only; answer and reasoning text are never included.
    Progress {
        operation_id: String,
        attempt: u32,
        route_index: u32,
        model: String,
        progress: provider_attempt::AttemptProgress,
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

/// Output contract an operation requires, declared independently of the role
/// that executes it (spec §2). Capability selection runs against the actual
/// route on every attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationFormat {
    /// Finite typed judgments: native Jev decisions or constrained chat JSON.
    BoundedDecision,
    /// New structured content (claims, entities, proposals) via chat JSON.
    GeneratedJson,
    /// Text/Markdown via chat.
    Prose,
}

impl OperationFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BoundedDecision => "bounded_decision",
            Self::GeneratedJson => "generated_json",
            Self::Prose => "prose",
        }
    }

    /// Can this route serve the format? `Err` is an explicit capability skip.
    pub fn supports(self, secret: &ProviderSecret) -> std::result::Result<(), String> {
        let jev = provider::is_jev_family(&secret.model);
        match self {
            Self::BoundedDecision if jev => provider::native_decision_capability(secret),
            Self::BoundedDecision => Ok(()),
            _ if jev => Err(format!(
                "{} cannot generate {}; Jev returns typed decisions only",
                secret.model.trim(),
                self.as_str()
            )),
            _ => Ok(()),
        }
    }
}

/// First usable route in the role's complete effective chain (primary,
/// inheritance and ordered fallbacks) that supports `format`. Used by legacy
/// single-secret callers so a Jev primary never receives chat and a later
/// compatible fallback is not lost. Returns `None` when nothing qualifies.
pub fn select_role_secret(
    auth: &AuthFile,
    settings: &SettingsFile,
    role: &str,
    format: OperationFormat,
) -> Option<ProviderSecret> {
    let (routes, secrets) = resolve_role_routes(auth, settings, role);
    routes
        .iter()
        .zip(secrets)
        .find(|(route, secret)| route.skip_reason.is_none() && format.supports(secret).is_ok())
        .map(|(_, secret)| secret)
}

/// Resolve the effective role assignment and fallbacks into Routes and Secrets.
pub fn resolve_role_routes(
    auth: &AuthFile,
    settings: &SettingsFile,
    role: &str,
) -> (Vec<Route>, Vec<ProviderSecret>) {
    resolve_operation_routes(auth, settings, role, None)
}

/// Route chain for an operation: its override when set, else the role chain.
pub fn resolve_operation_routes(
    auth: &AuthFile,
    settings: &SettingsFile,
    role: &str,
    operation: Option<&str>,
) -> (Vec<Route>, Vec<ProviderSecret>) {
    let canonical = provider::role_name(role).unwrap_or(role);
    let (assignment, _overridden) = settings.defaults.resolve_operation(operation, canonical);
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
    /// Reported reasoning/visible tokens and USD cost (schema 41); `None`
    /// means the provider did not report it, never zero.
    pub reasoning_tokens: Option<i64>,
    pub visible_tokens: Option<i64>,
    pub reported_cost_usd: Option<f64>,
    /// Wire marks relative to `sent_at` (schema 41); `None` when not observed.
    pub response_headers_ms: Option<i64>,
    pub first_reasoning_ms: Option<i64>,
    pub first_output_ms: Option<i64>,
    pub stream_completion_ms: Option<i64>,
    /// Request-policy diagnostics (dropped/rewritten fields), JSON array.
    pub policy_diagnostics: Vec<String>,
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

    /// Reported reasoning/visible tokens and cost, never estimated.
    pub fn extended_usage(&mut self, usage: Option<&crate::provider::Usage>) {
        let Some(u) = usage else { return };
        self.reasoning_tokens = u
            .reasoning_tokens
            .map(|v| v as i64)
            .or(self.reasoning_tokens);
        self.visible_tokens = u.visible_tokens.map(|v| v as i64).or(self.visible_tokens);
        self.reported_cost_usd = u.cost_usd.or(self.reported_cost_usd);
    }

    /// Convert observed wire marks into offsets from the send instant.
    pub fn wire_marks(&mut self, marks: &WireMarks) {
        if let Some((at, instant)) = &marks.dispatched {
            self.sent_at = at.clone();
            self.sent_instant = Some(*instant);
        }
        let Some(sent) = self.sent_instant else {
            return;
        };
        let rel =
            |at: Option<Instant>| at.map(|a| a.saturating_duration_since(sent).as_millis() as i64);
        self.response_headers_ms = rel(marks.headers);
        self.first_reasoning_ms = rel(marks.first_reasoning);
        self.stream_completion_ms = rel(marks.stream_complete);
        self.first_output_ms = rel(marks.first_output);
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

/// Instants observed from `AttemptProgress` during one wire attempt.
#[derive(Clone, Debug, Default)]
pub struct WireMarks {
    pub dispatched: Option<(String, Instant)>,
    pub headers: Option<Instant>,
    pub first_reasoning: Option<Instant>,
    pub first_output: Option<Instant>,
    pub stream_complete: Option<Instant>,
}

impl WireMarks {
    pub fn observe(&mut self, progress: &provider_attempt::AttemptProgress, at: Instant) {
        use provider_attempt::AttemptProgress as P;
        match progress {
            P::Dispatched => {
                self.dispatched.get_or_insert_with(|| (now_rfc3339(), at));
            }
            P::HeadersReceived { .. } => {
                self.headers.get_or_insert(at);
            }
            P::Receiving {
                answer_chars,
                reasoning_chars,
            } => {
                if *reasoning_chars > 0 {
                    self.first_reasoning.get_or_insert(at);
                }
                if *answer_chars > 0 {
                    self.first_output.get_or_insert(at);
                }
            }
            P::StreamComplete { .. } => {
                self.stream_complete.get_or_insert(at);
            }
            _ => {}
        }
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
    /// Adapter-reported usage for this wire attempt, never estimated.
    pub usage: Option<crate::provider::Usage>,
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
            usage: None,
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
    strict_persistence: bool,
    /// Retry backoff actually slept before this dispatch (from the chain
    /// context); overrides the constructor's legacy elapsed value.
    backoff_ms: Option<u64>,
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
            strict_persistence: false,
            backoff_ms: None,
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
        .app(if facts.operation_id.starts_with("atlas-classify-") {
            "atlas"
        } else {
            TELEMETRY_APP
        })
        .trigger(if facts.operation_id.starts_with("atlas-classify-") {
            Trigger::AtlasCycle
        } else {
            TELEMETRY_TRIGGER
        })
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
            "prompt_tokens": outcome.usage.as_ref().and_then(|u| u.prompt_tokens),
            "completion_tokens": outcome.usage.as_ref().and_then(|u| u.completion_tokens),
            "reasoning_tokens": outcome.usage.as_ref().and_then(|u| u.reasoning_tokens),
            "visible_tokens": outcome.usage.as_ref().and_then(|u| u.visible_tokens),
            "cost_usd": outcome.usage.as_ref().and_then(|u| u.cost_usd),
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
        .app(if scope.operation_id.starts_with("atlas-classify-") {
            "atlas"
        } else {
            TELEMETRY_APP
        })
        .trigger(if scope.operation_id.starts_with("atlas-classify-") {
            Trigger::AtlasCycle
        } else {
            TELEMETRY_TRIGGER
        })
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
// The shared chain carries one complete typed provider diagnostic per failure.
#[allow(clippy::result_large_err)]
fn finish_attempt(
    db_path: Option<&Path>,
    attempt_id: &str,
    facts: &AttemptFacts,
    outcome: &AttemptOutcome,
    timings: &mut AttemptTimings,
    elapsed_ms: u64,
    stream_completed: bool,
) -> Result<(), DispatchError> {
    let owned;
    let outcome = match facts.backoff_ms {
        Some(ms) => {
            let mut charged = outcome.clone();
            charged.wait_ms = ms;
            owned = charged;
            &owned
        }
        None => outcome,
    };
    // Adapter-reported usage only; adapters that report nothing leave the
    // columns at their defaults rather than being guessed.
    let usage = outcome.usage.as_ref();
    timings.usage(
        usage.and_then(|u| u.prompt_tokens).map(|v| v as i64),
        usage.and_then(|u| u.completion_tokens).map(|v| v as i64),
    );
    timings.extended_usage(usage);
    timings.finish(elapsed_ms, stream_completed);
    if let Err(error) = record_attempt_finish(db_path, attempt_id, outcome, timings) {
        if facts.strict_persistence {
            return Err(DispatchError::from_failure(ProviderFailure::new(
                Stage::Persistence,
                Category::Persistence,
                error.to_string(),
            )));
        }
    }
    note_attempt(db_path, attempt_id, facts, outcome, timings);
    Ok(())
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
    let (id, run, task, role) = (
        scope.operation_id.to_string(),
        scope.run_id.unwrap_or("").to_string(),
        scope.task_id.unwrap_or("").to_string(),
        scope.role.to_string(),
    );
    let (generation, draft, epoch) = (
        i64::from(scope.generation),
        draft.to_string(),
        scope.task_epoch,
    );
    on_repository(move |repo| async move {
        crate::storage::recon_jobs::ensure_operation(
            &repo, &id, &run, &task, &role, generation, &draft, epoch,
        )
        .await
    });
    Ok(())
}

/// Update durable draft for an operation.
pub fn save_operation_draft(db_path: Option<&Path>, operation_id: &str, draft: &str) -> Result<()> {
    let Some(db_path) = db_path else {
        return Ok(());
    };
    let (id, draft) = (operation_id.to_string(), draft.to_string());
    on_repository(move |repo| async move {
        crate::storage::recon_exec::set_operation_draft(&repo, &id, &draft).await
    });
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
    let (id, status) = (operation_id.to_string(), status.to_string());
    let final_id = final_message_id.map(str::to_string);
    on_repository(move |repo| async move {
        crate::storage::recon_exec::finish_operation(&repo, &id, &status, final_id.as_deref()).await
    });
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
    declaration: &str,
) -> Result<String> {
    let attempt_id = canonical_attempt_id(operation_id, generation, route_index, attempt);
    let Some(db_path) = db_path else {
        return Ok(attempt_id);
    };
    let owned = [
        attempt_id.clone(),
        operation_id.to_string(),
        route.provider.to_string(),
        route.account.to_string(),
        route.model.to_string(),
        transport.to_string(),
        queued_at.to_string(),
        declaration.to_string(),
    ];
    on_repository(move |repo| async move {
        let [id, op, provider, account, model, transport, queued, decl] = &owned;
        let start = crate::storage::recon_jobs::AttemptStart {
            id,
            operation_id: op,
            generation,
            route_index,
            attempt,
            provider,
            account,
            model,
            transport,
            queued_at: queued,
            declaration_json: decl,
        };
        crate::storage::recon_jobs::record_attempt_start(&repo, &start).await
    });
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
    let id = attempt_id.to_string();
    let finish = attempt_finish(outcome, timings);
    on_repository(move |repo| async move {
        crate::storage::recon_jobs::record_attempt_finish(&repo, &id, &finish).await
    });
    Ok(())
}

/// Run one ledger write on the repository, in start order (an attempt's
/// finish never lands before its start). Best-effort, like telemetry.
#[cfg(feature = "surreal")]
fn on_repository<F, Fut>(write: F)
where
    F: FnOnce(crate::storage::repo::Repository) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<()>> + Send,
{
    crate::storage::runtime::spawn_ordered(async move {
        if let Ok(repo) = crate::storage::runtime::repository().await {
            let _ = write(repo).await;
        }
    });
}

/// The repository form of an attempt's finish (`record_attempt_finish`).
#[cfg(feature = "surreal")]
fn attempt_finish(
    outcome: &AttemptOutcome,
    timings: &AttemptTimings,
) -> crate::storage::recon_jobs::AttemptFinish {
    crate::storage::recon_jobs::AttemptFinish {
        outcome: outcome.outcome.clone(),
        failure_category: outcome.category.clone().unwrap_or_default(),
        http_status: outcome.http_status.map(i64::from),
        request_id: outcome.request_id.clone().unwrap_or_default(),
        finish_reason: outcome.finish_reason.clone().unwrap_or_default(),
        char_count: outcome.char_count as i64,
        wait_ms: outcome.wait_ms as i64,
        error_message: outcome.error_message.clone().unwrap_or_default(),
        queued_at: timings.queued_at.clone(),
        admitted_at: timings.admitted_at.clone(),
        sent_at: timings.sent_at.clone(),
        first_response_at: timings.first_response_at.clone(),
        elapsed_ms: timings.elapsed_ms,
        queue_ms: timings.queue_ms,
        ttfb_ms: timings.ttfb_ms,
        prompt_tokens: timings.prompt_tokens,
        completion_tokens: timings.completion_tokens,
        reasoning_tokens: timings.reasoning_tokens,
        visible_tokens: timings.visible_tokens,
        reported_cost_usd: timings.reported_cost_usd,
        response_headers_ms: timings.response_headers_ms,
        first_reasoning_ms: timings.first_reasoning_ms,
        first_output_ms: timings.first_output_ms,
        stream_completion_ms: timings.stream_completion_ms,
        stream_completed: timings.stream_completed != 0,
        policy_diagnostics_json: serde_json::to_string(&timings.policy_diagnostics)
            .unwrap_or_else(|_| "[]".into()),
    }
}

/// Operation label for the all-Jev generative-chain configuration error.
pub fn all_jev_operation_label(role: &str) -> String {
    match role {
        "claim_assessor" => "Claim Assessor extraction".into(),
        "entity_resolver" => "Entity Resolver generation".into(),
        other => {
            let name = provider::role_name(other).unwrap_or(other);
            let mut chars = name.replace('_', " ").chars().collect::<Vec<_>>();
            if let Some(c) = chars.first_mut() {
                *c = c.to_ascii_uppercase();
            }
            format!("{} generation", chars.into_iter().collect::<String>())
        }
    }
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
    if scope.insight_contract {
        ensure_operation(db_path, scope, "")?;
    } else {
        let _ = ensure_operation(db_path, scope, "");
    }
    let (mut routes, secrets) =
        resolve_operation_routes(auth, settings, scope.role, scope.operation_key);
    // Free-chat extraction cannot dispatch a Decisions-only model. Keep route
    // indices and declared fallback order so audit and secrets remain aligned.
    for (route, secret) in routes.iter_mut().zip(&secrets) {
        if provider::is_decisions_model(&secret.model) {
            route.skip_reason = Some("Decisions-only route does not support free chat".into());
        }
    }
    if !routes.is_empty()
        && secrets
            .iter()
            .all(|secret| provider::is_decisions_model(&secret.model))
    {
        anyhow::bail!(
            "{} requires a non-Jev chat fallback",
            all_jev_operation_label(scope.role)
        );
    }

    let repairs = Arc::new(OutputRepairs::default());
    let insight_contract = scope.insight_contract;
    let summarization = scope.summarization && !insight_contract;
    let insight_policy = scope.request_policy(true, 8192);
    // Stage admission: capture one capability snapshot per route (and kick a
    // coalesced background catalog refresh) so nothing changes mid-operation.
    let admitted: Arc<Vec<crate::provider_capabilities::ResolvedCapabilities>> = Arc::new(
        secrets
            .iter()
            .map(|secret| {
                // The capability cache is SQLite-only: never open it under SurrealDB.
                let cache = db_path.and_then(super::backend::sqlite_path);
                crate::provider_capabilities::admit(secret, cache)
            })
            .collect(),
    );
    let contract = scope.contract.clone();
    let declaration = {
        let mut declaration = scope.declaration(settings, insight_policy);
        declaration.capability_digests = admitted.iter().map(|c| c.digest()).collect();
        Arc::new(serde_json::to_string(&declaration).unwrap_or_default())
    };
    let mut attempt_counter = 0u32;
    let mut last_route_idx = 0usize;
    let on_event_shared = on_event.clone();
    let op_id = scope.operation_id.to_string();
    let generation = scope.generation;
    let db_path_owned = db_path.map(Path::to_path_buf);
    let validator = Arc::new(validator);
    let role = scope.role.to_string();
    let run_id = scope.run_id.unwrap_or_default().to_string();
    let dispatch = |ctx: provider_chain::DispatchContext, route: &Route| {
        let route_idx = ctx.route_index;
        last_route_idx = route_idx;
        attempt_counter += 1;
        let attempt_num = attempt_counter;
        let queued_at = ctx.queued_at.clone();
        let queued_instant = Instant::now()
            .checked_sub(std::time::Duration::from_millis(ctx.queue_ms))
            .unwrap_or_else(Instant::now);
        let backoff_ms = ctx.backoff_ms;
        let remaining = ctx.remaining;
        let admitted = admitted.clone();
        let contract = contract.clone();
        let secret = secrets[route_idx].clone();
        let transport = if stream && !provider_attempt::is_subscription(&secret) {
            Transport::Stream
        } else {
            Transport::NonStream
        };
        let messages_vec = if insight_contract {
            repairs.messages(route_idx, messages)
        } else {
            messages.to_vec()
        };
        let repairs = repairs.clone();
        let text_obs = text_observer.clone();
        let val = Arc::clone(&validator);
        let on_event_step = on_event_shared.clone();
        let route_cloned = route.clone();
        let op_id = op_id.clone();
        let db_path_buf = db_path_owned.clone();
        let role = role.clone();
        let run_id = run_id.clone();
        let declaration = declaration.clone();

        async move {
            if insight_contract
                && crate::nlp::message_bytes(&messages_vec, &Value::Null)
                    > crate::investigation::insights::REQUEST_BYTES
            {
                return Err(DispatchError::from_failure(ProviderFailure::new(
                    Stage::Configuration,
                    Category::MalformedRequest,
                    "compiled insight request exceeds byte budget",
                )));
            }
            let attempt_db_id = record_attempt_start(
                db_path_buf.as_deref(),
                &op_id,
                generation,
                route_idx as u32,
                attempt_num,
                &route_cloned,
                transport.as_str(),
                &queued_at,
                &declaration,
            )
            .map_err(|error| {
                DispatchError::from_failure(ProviderFailure::new(
                    Stage::Persistence,
                    Category::Persistence,
                    error.to_string(),
                ))
            });
            let attempt_db_id = if insight_contract {
                attempt_db_id?
            } else {
                attempt_db_id.unwrap_or_default()
            };
            let mut facts = AttemptFacts::new(
                &op_id,
                generation,
                &role,
                &run_id,
                &route_cloned,
                transport.as_str(),
                route_idx as u32,
                attempt_num,
            );
            facts.strict_persistence = insight_contract;
            facts.backoff_ms = Some(backoff_ms);

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
            let marks: Arc<std::sync::Mutex<WireMarks>> = Arc::default();
            let progress_sink = {
                let sink = on_event_step.clone();
                let marks = marks.clone();
                let op_id = op_id.clone();
                let model = route_cloned.model.clone();
                Some(
                    Arc::new(move |progress: provider_attempt::AttemptProgress| {
                        if let Ok(mut m) = marks.lock() {
                            m.observe(&progress, Instant::now());
                        }
                        if let Some(sink) = &sink {
                            sink(ModelExecEvent::Progress {
                                operation_id: op_id.clone(),
                                attempt: attempt_num,
                                route_index: route_idx as u32,
                                model: model.clone(),
                                progress,
                            })
                        }
                    }) as Arc<provider_attempt::ProgressFn>,
                )
            };
            let rep = if insight_contract || summarization {
                let deadlines = if summarization {
                    Deadlines::summarization()
                } else {
                    scope.deadlines(transport == Transport::Stream)
                };
                let attempt = provider_attempt::attempt_spec(
                    &secret,
                    &messages_vec,
                    transport,
                    deadlines,
                    &insight_policy,
                    provider_attempt::AttemptSpec {
                        observer: Some(observer.as_ref() as &(dyn Fn(&str) + Send + Sync)),
                        progress: progress_sink.as_deref(),
                        contract: contract.as_ref(),
                        caps: admitted.get(route_idx),
                    },
                );
                bounded_by_cap(remaining, &secret, transport, attempt).await
            } else {
                provider_attempt::attempt_full(
                    &secret,
                    &messages_vec,
                    transport,
                    Deadlines::default(),
                    Some(observer.as_ref() as &(dyn Fn(&str) + Send + Sync)),
                    progress_sink.as_deref(),
                    &crate::request_policy::RequestPolicy::default(),
                )
                .await
            };

            if let Ok(slot) = first_at.lock() {
                if let Some((at, instant)) = slot.as_ref() {
                    timings.first_response(at, *instant);
                }
            }
            if let Ok(mut m) = marks.lock() {
                // The observer sees the first answer delta exactly; the
                // throttled `Receiving` mark is only a fallback.
                if timings.first_response_instant.is_some() {
                    m.first_output = timings.first_response_instant;
                }
                timings.wire_marks(&m);
            }
            timings.policy_diagnostics = rep.policy_diagnostics.clone();
            let elapsed_ms = start_t.elapsed().as_millis() as u64;
            let partial = streamed_acc.lock().map(|s| s.clone()).unwrap_or_default();
            if !partial.is_empty() {
                let _ = save_operation_draft(db_path_buf.as_deref(), &op_id, &partial);
            }

            let recovery = rep.recovery.clone();
            match rep.outcome {
                Ok(completion) => {
                    let usage = completion.usage.clone();
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
                        )?;
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
                        )?;
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
                        return Err(if insight_contract {
                            repairs.failure(route_idx, val_err, Stage::Validation)
                        } else {
                            DispatchError::from_failure(f)
                        });
                    }

                    // Success!
                    let mut outcome =
                        AttemptOutcome::ok(finish_reason.as_deref(), chars, elapsed_ms);
                    outcome.usage = usage;
                    finish_attempt(
                        db_path_buf.as_deref(),
                        &attempt_db_id,
                        &facts,
                        &outcome,
                        &mut timings,
                        elapsed_ms,
                        matches!(transport, Transport::Stream) && finish_reason.is_some(),
                    )?;
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
                    let mut err = DispatchError::from_failure(failure.clone());
                    if let Some(reason) = recovery {
                        err = err.with_recovery(reason);
                    }
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
                    )?;

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
        attempt_limit: if insight_contract {
            Some(2)
        } else if summarization {
            Some(crate::tasks::OperationKind::Summarization.attempt_cap())
        } else {
            None
        },
        interactive: scope.interactive,
        wire_budget: scope.wire_budget(),
        ledger: scope.ledger.clone(),
    };

    let report = provider_chain::execute_with_context(&routes, dispatch, opts).await;

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
    } else {
        let _ = update_operation_status(
            db_path,
            scope.operation_id,
            if report.cancelled {
                "cancelled"
            } else {
                "failed"
            },
            None,
        );
        Err(chain_error(&report))
    }
}

/// Decisions compiler abstraction for Jev or chat fallback.
pub trait DecisionsAdapter<T>: Send + Sync {
    fn compile_native(&self) -> (Value, Value);
    fn parse_native(&self, resp: &provider::DecisionsResponse) -> Result<T, String>;
    fn compile_chat(&self) -> Vec<ChatMessage>;
    fn parse_chat(&self, raw: &str) -> Result<T, String>;
    /// Parse a chat answer knowing the structured mode actually compiled.
    fn parse_chat_as(
        &self,
        raw: &str,
        _kind: crate::investigation::decisions::DecisionAdapterKind,
    ) -> Result<T, String> {
        self.parse_chat(raw)
    }
    /// Contract validation before any network access.
    fn preflight(&self) -> Result<(), String> {
        Ok(())
    }
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
    if scope.insight_contract {
        ensure_operation(db_path, scope, "")?;
    } else {
        let _ = ensure_operation(db_path, scope, "");
    }
    adapter
        .preflight()
        .map_err(|reason| anyhow::anyhow!("malformed decision contract: {reason}"))?;
    let (mut routes, secrets) =
        resolve_operation_routes(auth, settings, scope.role, scope.operation_key);
    // Capability is resolved separately from endpoint availability: a Jev
    // route without a supported native connection is skipped (zero requests,
    // no attempt/RPM/retry consumed); it is never sent to chat.
    for (route, secret) in routes.iter_mut().zip(&secrets) {
        if route.skip_reason.is_none() && provider::is_jev_family(&secret.model) {
            if let Err(reason) = provider::native_decision_capability(secret) {
                route.skip_reason = Some(reason);
            }
        }
    }

    let repairs = Arc::new(OutputRepairs::default());
    let insight_contract = scope.insight_contract;
    let stage_policy = scope.completion_tokens.is_some() || scope.budget.is_some();
    let chat_policy = scope.request_policy(true, 2048);
    let admitted: Arc<Vec<crate::provider_capabilities::ResolvedCapabilities>> = Arc::new(
        secrets
            .iter()
            .map(|secret| {
                // The capability cache is SQLite-only: never open it under SurrealDB.
                let cache = db_path.and_then(super::backend::sqlite_path);
                crate::provider_capabilities::admit(secret, cache)
            })
            .collect(),
    );
    let declaration = {
        let mut declaration = scope.declaration(settings, chat_policy);
        declaration.capability_digests = admitted.iter().map(|c| c.digest()).collect();
        Arc::new(serde_json::to_string(&declaration).unwrap_or_default())
    };
    let mut attempt_counter = 0u32;
    let on_event_shared = on_event.clone();
    let op_id = scope.operation_id.to_string();
    let generation = scope.generation;
    let db_path_owned = db_path.map(Path::to_path_buf);
    let role = scope.role.to_string();
    let run_id = scope.run_id.unwrap_or_default().to_string();
    let dispatch = |ctx: provider_chain::DispatchContext, route: &Route| {
        let route_idx = ctx.route_index;
        attempt_counter += 1;
        let attempt_num = attempt_counter;
        let queued_at = ctx.queued_at.clone();
        let queued_instant = Instant::now()
            .checked_sub(std::time::Duration::from_millis(ctx.queue_ms))
            .unwrap_or_else(Instant::now);
        let backoff_ms = ctx.backoff_ms;
        let admitted = admitted.clone();
        let secret = secrets[route_idx].clone();
        // Skipped incompatible Jev routes never reach dispatch, so a Jev ID here
        // always has a native connection.
        let is_native = provider::native_decision_capability(&secret).is_ok();
        let transport = if is_native { "decisions" } else { "chat" };
        let on_event_step = on_event_shared.clone();
        let route_cloned = route.clone();
        let op_id = op_id.clone();
        let db_path_buf = db_path_owned.clone();
        let role = role.clone();
        let run_id = run_id.clone();
        let repairs = repairs.clone();
        let declaration = declaration.clone();

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
                &declaration,
            )
            .map_err(|error| {
                DispatchError::from_failure(ProviderFailure::new(
                    Stage::Persistence,
                    Category::Persistence,
                    error.to_string(),
                ))
            });
            let attempt_db_id = if insight_contract {
                attempt_db_id?
            } else {
                attempt_db_id.unwrap_or_default()
            };
            let mut facts = AttemptFacts::new(
                &op_id,
                generation,
                &role,
                &run_id,
                &route_cloned,
                transport,
                route_idx as u32,
                attempt_num,
            );
            facts.strict_persistence = insight_contract;
            facts.backoff_ms = Some(backoff_ms);

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
                let (mut native_state, native_questions) = adapter.compile_native();
                if insight_contract {
                    if let Some(error) = repairs.0.lock().unwrap().get(&route_idx) {
                        native_state["output_validation_error"] = Value::String(error.clone());
                    }
                    if serde_json::to_vec(&serde_json::json!([native_state, native_questions]))
                        .map_or(true, |b| {
                            b.len() > crate::investigation::insights::REQUEST_BYTES
                        })
                    {
                        return Err(DispatchError::from_failure(ProviderFailure::new(
                            Stage::Configuration,
                            Category::MalformedRequest,
                            "compiled native assessment exceeds byte budget",
                        )));
                    }
                }
                match provider::decide_with_dispatch(
                    &secret,
                    &native_state,
                    &native_questions,
                    || timings.sent(),
                )
                .await
                {
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
                                )?;
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
                                )?;
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
                                Err(if insight_contract {
                                    repairs.failure(route_idx, parse_err, Stage::Validation)
                                } else {
                                    DispatchError::from_failure(f)
                                })
                            }
                        }
                    }
                    Err(decide_err) => {
                        let elapsed_ms = start_t.elapsed().as_millis() as u64;
                        let f = ProviderFailure::from_anyhow(Stage::Response, &decide_err);
                        let err = if insight_contract
                            && matches!(
                                f.category,
                                Category::InvalidResult | Category::MalformedPayload
                            ) {
                            repairs.failure(route_idx, f.message.clone(), Stage::Validation)
                        } else {
                            DispatchError::from_failure(f.clone())
                        };
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
                        )?;
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
                let chat_messages = if insight_contract {
                    repairs.messages(route_idx, &adapter.compile_chat())
                } else {
                    adapter.compile_chat()
                };
                if insight_contract
                    && crate::nlp::message_bytes(&chat_messages, &Value::Null)
                        > crate::investigation::insights::REQUEST_BYTES
                {
                    return Err(DispatchError::from_failure(ProviderFailure::new(
                        Stage::Configuration,
                        Category::MalformedRequest,
                        "compiled assessment exceeds byte budget",
                    )));
                }
                let marks: Arc<std::sync::Mutex<WireMarks>> = Arc::default();
                let progress = {
                    let marks = marks.clone();
                    move |p: provider_attempt::AttemptProgress| {
                        if let Ok(mut marks) = marks.lock() {
                            marks.observe(&p, Instant::now());
                        }
                    }
                };
                let rep = if insight_contract || stage_policy {
                    // Preserve the bounded decision response contract without
                    // imposing the former per-stage output/thinking ceilings.
                    provider_attempt::attempt_spec(
                        &secret,
                        &chat_messages,
                        Transport::NonStream,
                        Deadlines::native_decision(),
                        &chat_policy,
                        provider_attempt::AttemptSpec {
                            progress: Some(&progress),
                            contract: Some(
                                &crate::provider_adapters::ResponseContract::BoundedDecision,
                            ),
                            caps: admitted.get(route_idx),
                            ..Default::default()
                        },
                    )
                    .await
                } else {
                    provider_attempt::attempt_full(
                        &secret,
                        &chat_messages,
                        Transport::NonStream,
                        Deadlines::default(),
                        None,
                        Some(&progress),
                        &crate::request_policy::RequestPolicy::default(),
                    )
                    .await
                };

                if let Ok(marks) = marks.lock() {
                    timings.wire_marks(&marks);
                }
                let elapsed_ms = start_t.elapsed().as_millis() as u64;
                let recovery = rep.recovery.clone();
                let kind = crate::investigation::decisions::adapters::kind_for_structured(
                    rep.effective.as_ref().map(|e| e.structured),
                );

                match rep.outcome {
                    Ok(completion) => {
                        let chars = completion.content.chars().count() as u64;
                        match adapter.parse_chat_as(&completion.content, kind) {
                            Ok(val) => {
                                let mut outcome = AttemptOutcome::ok(
                                    completion.finish_reason.as_deref(),
                                    chars,
                                    elapsed_ms,
                                );
                                outcome.usage = completion.usage.clone();
                                finish_attempt(
                                    db_path_buf.as_deref(),
                                    &attempt_db_id,
                                    &facts,
                                    &outcome,
                                    &mut timings,
                                    elapsed_ms,
                                    false,
                                )?;
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
                                )?;
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
                                Err(if insight_contract {
                                    repairs.failure(route_idx, parse_err, Stage::Validation)
                                } else {
                                    DispatchError::from_failure(f)
                                })
                            }
                        }
                    }
                    Err(failure) => {
                        let cat = failure.category.as_str().to_string();
                        let finish_r = failure.stream.finish_reason.clone();
                        let chars = failure.stream.partial_chars;
                        let msg = failure.summary();
                        let mut err = DispatchError::from_failure(failure.clone());
                        if let Some(reason) = recovery {
                            err = err.with_recovery(reason);
                        }
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
                        )?;

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
        attempt_limit: insight_contract.then_some(2),
        interactive: scope.interactive,
        wire_budget: scope.wire_budget(),
        ledger: scope.ledger.clone(),
    };

    let report = provider_chain::execute_with_context(&routes, dispatch, opts).await;

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
    } else {
        let _ = update_operation_status(
            db_path,
            scope.operation_id,
            if report.cancelled {
                "cancelled"
            } else {
                "failed"
            },
            None,
        );
        Err(chain_error(&report))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automatic_operations_have_no_output_caps_and_only_thinking_stages_send_effort() {
        use crate::request_policy::{ReasoningEffort, INFERENCES_BUDGET};
        for role in provider::CANONICAL_ROLE_KEYS {
            for (scope, effort) in [
                (OperationScope::new("op", role), ReasoningEffort::Default),
                (
                    OperationScope::new("op", role).with_budget(INFERENCES_BUDGET),
                    ReasoningEffort::Efficient,
                ),
                (
                    OperationScope::new("op", role)
                        .with_budget(crate::request_policy::CLAIMS_BUDGET),
                    ReasoningEffort::Default,
                ),
            ] {
                let policy = scope.request_policy(true, 2048);
                assert!(policy.max_output_tokens.is_none());
                assert!(policy.thinking_ceiling.is_none());
                assert!(policy.reasoning_budget.is_none());
                assert!(policy.budgeted_ceiling.is_none());
                assert_eq!(policy.reasoning, effort);
                assert!(policy.json_object);
            }
        }
    }

    #[test]
    fn interactive_operations_have_a_wall_clock_cap_sized_for_thinking() {
        use crate::request_policy::{CLAIMS_BUDGET, INFERENCES_BUDGET};
        let cap = |scope: OperationScope| scope.wire_budget().unwrap().wall_clock.unwrap();
        let think = cap(OperationScope::new("op", "synthesis")
            .with_budget(INFERENCES_BUDGET)
            .with_interactive());
        let plain = cap(OperationScope::new("op", "claims")
            .with_budget(CLAIMS_BUDGET)
            .with_interactive());
        assert_eq!(think.as_secs(), 900);
        assert_eq!(plain.as_secs(), 600);
        assert!(OperationScope::new("op", "claims").wire_budget().is_none());
        // Room for a fallback after one thinking-sized stall.
        assert!(think > Deadlines::insight_stream_thinking().first_output.unwrap() * 2);
    }

    #[tokio::test]
    async fn an_attempt_that_outlives_the_cap_becomes_a_typed_timeout() {
        let secret = crate::provider_attempt::mock::secret("http://127.0.0.1:9");
        let report = bounded_by_cap(
            Some(std::time::Duration::from_millis(50)),
            &secret,
            Transport::Stream,
            std::future::pending(),
        )
        .await;
        let failure = report.outcome.unwrap_err();
        assert_eq!(failure.category, Category::Timeout);
        assert!(
            failure.message.contains("wall-clock cap"),
            "{}",
            failure.message
        );
        let uncapped = bounded_by_cap(None, &secret, Transport::Stream, async {
            provider_attempt::AttemptReport {
                transport: Transport::Stream,
                endpoint: String::new(),
                elapsed_ms: 0,
                outcome: Err(ProviderFailure::new(Stage::Stream, Category::Empty, "x")),
                policy_diagnostics: vec![],
                effective: None,
                recovery: None,
            }
        })
        .await;
        assert_eq!(uncapped.outcome.unwrap_err().category, Category::Empty);
    }

    #[test]
    fn thinking_stages_get_thinking_sized_deadlines_and_others_keep_theirs() {
        use crate::request_policy::{CLAIMS_BUDGET, INFERENCES_BUDGET, LINKS_BUDGET};
        let think = OperationScope::new("op", "synthesis").with_budget(INFERENCES_BUDGET);
        let (s, n) = (think.deadlines(true), think.deadlines(false));
        assert_eq!(s.first_response, Deadlines::insight_stream().first_response);
        assert!(s.first_output > Deadlines::insight_stream().first_output);
        assert!(s.model_inactivity > Deadlines::insight_stream().model_inactivity);
        assert_eq!(s.total, Deadlines::insight_stream().total);
        assert!(n.first_response > Deadlines::insight_non_stream().first_response);
        assert!(n.first_response < n.total);
        for budget in [CLAIMS_BUDGET, LINKS_BUDGET] {
            let scope = OperationScope::new("op", "claims").with_budget(budget);
            assert_eq!(
                scope.deadlines(true).first_output,
                Deadlines::insight_stream().first_output
            );
            assert_eq!(
                scope.deadlines(false).first_response,
                Deadlines::insight_non_stream().first_response
            );
        }
    }

    #[test]
    fn provider_queue_residence_is_separate_from_wire_latency() {
        let queued = Instant::now() - Duration::from_secs(10);
        let sent = queued + Duration::from_secs(6);
        let mut timings = AttemptTimings::queued("queued", queued);
        timings.sent(); // Superseded by the actual transport send mark.
        let mut marks = WireMarks::default();
        marks.observe(&provider_attempt::AttemptProgress::Dispatched, sent);
        marks.observe(
            &provider_attempt::AttemptProgress::HeadersReceived { status: 200 },
            sent + Duration::from_secs(1),
        );
        timings.wire_marks(&marks);
        timings.finish(8000, false);
        assert_eq!(timings.queue_ms, 6000);
        assert_eq!(timings.response_headers_ms, Some(1000));
    }

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

#[cfg(test)]
#[path = "model_exec_routing_tests.rs"]
mod routing_tests;

#[cfg(test)]
#[path = "model_exec_format_tests.rs"]
mod format_tests;
