//! One outbound provider request, final-only.
//!
//! [`attempt`] sends exactly one HTTP request (or one subscription call) and
//! returns either a complete answer or a typed [`ProviderFailure`]. It never
//! retries or switches transport by itself; the caller owns the request
//! budget. A streamed answer is buffered and only returned when the stream
//! carried an explicit completion indicator (`[DONE]` or a finish reason);
//! partial text from a broken stream is measured and dropped.

// `ProviderFailure` is a full diagnostic returned at most once per request;
// boxing it would only add noise at every construction site.
#![allow(clippy::result_large_err)]

use std::time::{Duration, Instant};

use futures_util::StreamExt;
use serde_json::Value;

use crate::provider::{self, ChatMessage, Completion};
use crate::provider_diag::{
    classify_status, provider_error, request_id, retry_after_uncapped, Category, ProviderFailure,
    Stage, StreamState,
};
use crate::request_policy::RequestPolicy;
use crate::secrets::ProviderSecret;

/// How the single request is made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    NonStream,
    Stream,
}

impl Transport {
    pub fn as_str(self) -> &'static str {
        match self {
            Transport::NonStream => "non_stream",
            Transport::Stream => "stream",
        }
    }

    pub fn other(self) -> Self {
        match self {
            Transport::NonStream => Transport::Stream,
            Transport::Stream => Transport::NonStream,
        }
    }
}

/// Per-attempt deadlines.
///
/// `connect`/`first_response` bound the transport up to response headers.
/// `idle` bounds silence on the wire (any bytes, including heartbeats).
/// `first_output`/`model_inactivity`, when set, bound *meaningful model
/// output* (answer or reasoning deltas): heartbeat-only traffic never extends
/// them. `total` caps the whole attempt; receiving data does not extend it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Deadlines {
    pub connect: Duration,
    pub first_response: Duration,
    pub idle: Duration,
    pub total: Duration,
    pub first_output: Option<Duration>,
    pub model_inactivity: Option<Duration>,
}

impl Default for Deadlines {
    fn default() -> Self {
        Self::ordinary()
    }
}

impl Deadlines {
    /// Ordinary/small operations: unchanged historical policy.
    pub const fn ordinary() -> Self {
        Self {
            connect: Duration::from_secs(10),
            first_response: Duration::from_secs(60),
            idle: Duration::from_secs(30),
            total: Duration::from_secs(120),
            first_output: None,
            model_inactivity: None,
        }
    }

    /// Streamed generated-JSON insight operations (extraction, proposal,
    /// reconciliation). Initial settings to validate with timings, not
    /// measured optima: connect 10s, headers 60s, first model output 180s,
    /// model inactivity 90s, total 600s.
    pub const fn insight_stream() -> Self {
        Self {
            connect: Duration::from_secs(10),
            first_response: Duration::from_secs(60),
            idle: Duration::from_secs(90),
            total: Duration::from_secs(600),
            first_output: Some(Duration::from_secs(180)),
            model_inactivity: Some(Duration::from_secs(90)),
        }
    }

    /// Insight operations that cannot stream: one bounded wait for the whole body.
    pub const fn insight_non_stream() -> Self {
        Self {
            connect: Duration::from_secs(10),
            first_response: Duration::from_secs(240),
            idle: Duration::from_secs(90),
            total: Duration::from_secs(300),
            first_output: None,
            model_inactivity: None,
        }
    }

    /// Native bounded decisions keep their own short policy.
    pub const fn native_decision() -> Self {
        Self::ordinary()
    }
}

/// Typed per-attempt progress. Never carries answer or reasoning text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttemptProgress {
    /// Request handed to the transport.
    Dispatched,
    /// Response headers arrived (not proof of model activity).
    HeadersReceived { status: u16 },
    /// First meaningful answer/reasoning delta.
    FirstModelOutput,
    /// Throttled (~1/s) receiving update with character counts.
    Receiving {
        answer_chars: u64,
        reasoning_chars: u64,
    },
    /// Transport heartbeat/comment with no model content.
    Heartbeat,
    /// Terminal marker received; the caller validates next.
    StreamComplete { finish_reason: Option<String> },
}

/// Progress sink threaded through `attempt_insight → attempt_options → read_stream`.
pub type ProgressFn = dyn Fn(AttemptProgress) + Send + Sync;

/// Minimum interval between `Receiving` updates.
pub const PROGRESS_THROTTLE: Duration = Duration::from_secs(1);

/// Outcome and timing of one request.
#[derive(Clone, Debug)]
pub struct AttemptReport {
    pub transport: Transport,
    pub endpoint: String,
    pub elapsed_ms: u64,
    pub outcome: Result<Completion, ProviderFailure>,
    /// Request-policy diagnostics (unsupported/learned options). Never text.
    pub policy_diagnostics: Vec<String>,
}

/// Is this connection served by a subprocess rather than HTTP?
pub fn is_subscription(secret: &ProviderSecret) -> bool {
    provider::effective_kind(secret) == "openai-chatgpt"
}

/// Send exactly one request.
pub async fn attempt(
    secret: &ProviderSecret,
    messages: &[ChatMessage],
    transport: Transport,
    deadlines: Deadlines,
) -> AttemptReport {
    attempt_with_observer(secret, messages, transport, deadlines, None).await
}

/// Send exactly one request with an optional text observer for streamed answer deltas.
pub async fn attempt_with_observer(
    secret: &ProviderSecret,
    messages: &[ChatMessage],
    transport: Transport,
    deadlines: Deadlines,
    observer: Option<&(dyn Fn(&str) + Send + Sync)>,
) -> AttemptReport {
    attempt_options(
        secret,
        messages,
        transport,
        deadlines,
        observer,
        None,
        &RequestPolicy::default(),
    )
    .await
}

/// Only the insight contract lane requests structured JSON and completion bounds.
/// The observer and typed progress sink are preserved (they used to be dropped).
pub async fn attempt_insight(
    secret: &ProviderSecret,
    messages: &[ChatMessage],
    transport: Transport,
    policy: &RequestPolicy,
    observer: Option<&(dyn Fn(&str) + Send + Sync)>,
    progress: Option<&ProgressFn>,
) -> AttemptReport {
    let deadlines = if transport == Transport::Stream {
        Deadlines::insight_stream()
    } else {
        Deadlines::insight_non_stream()
    };
    attempt_full(
        secret, messages, transport, deadlines, observer, progress, policy,
    )
    .await
}

/// Fully parameterized single attempt.
pub async fn attempt_full(
    secret: &ProviderSecret,
    messages: &[ChatMessage],
    transport: Transport,
    deadlines: Deadlines,
    observer: Option<&(dyn Fn(&str) + Send + Sync)>,
    progress: Option<&ProgressFn>,
    policy: &RequestPolicy,
) -> AttemptReport {
    attempt_options(
        secret, messages, transport, deadlines, observer, progress, policy,
    )
    .await
}

async fn attempt_options(
    secret: &ProviderSecret,
    messages: &[ChatMessage],
    transport: Transport,
    deadlines: Deadlines,
    observer: Option<&(dyn Fn(&str) + Send + Sync)>,
    progress: Option<&ProgressFn>,
    policy: &RequestPolicy,
) -> AttemptReport {
    let started = Instant::now();
    let kind = provider::effective_kind(secret);
    let endpoint = format!(
        "{}/chat/completions",
        provider::normalize_base(&secret.base_url)
    );
    if is_subscription(secret) {
        let outcome = match tokio::time::timeout(
            deadlines.total,
            crate::subscription::complete(secret, messages, &[], |delta| {
                if let Some(obs) = observer {
                    obs(delta);
                }
            }),
        )
        .await
        {
            Ok(Ok(c)) => check_final(c, None),
            Ok(Err(err)) => Err(ProviderFailure::from_anyhow(Stage::Response, &err)),
            Err(_) => Err(ProviderFailure::new(
                Stage::Response,
                Category::Timeout,
                format!(
                    "subscription completion exceeded {}s",
                    deadlines.total.as_secs()
                ),
            )),
        };
        return AttemptReport {
            transport,
            endpoint: "subscription".into(),
            elapsed_ms: started.elapsed().as_millis() as u64,
            outcome: outcome.map_err(|f| {
                let mut f = f.with_context(&kind, &secret.model, "", "subscription");
                f.elapsed_ms = started.elapsed().as_millis() as u64;
                f
            }),
            policy_diagnostics: if policy.is_empty() {
                vec![]
            } else {
                vec!["unsupported-policy:subscription transport".into()]
            },
        };
    }
    let mut first_ms = None;
    let mut stream_state = StreamState::default();
    let mut policy_diagnostics = Vec::new();
    let outcome = run(
        secret,
        messages,
        transport,
        deadlines,
        &endpoint,
        started,
        &mut first_ms,
        &mut stream_state,
        observer,
        progress,
        policy,
        &mut policy_diagnostics,
    )
    .await;
    let elapsed_ms = started.elapsed().as_millis() as u64;
    AttemptReport {
        transport,
        endpoint: crate::provider_diag::sanitize_endpoint(&endpoint),
        elapsed_ms,
        outcome: outcome.map_err(|f| {
            let mut f = f.with_context(&kind, &secret.model, &endpoint, transport.as_str());
            f.elapsed_ms = elapsed_ms;
            f.first_response_ms = first_ms;
            if transport == Transport::Stream {
                f.stream = stream_state;
            }
            f
        }),
        policy_diagnostics,
    }
}

#[allow(clippy::too_many_arguments)]
async fn run(
    secret: &ProviderSecret,
    messages: &[ChatMessage],
    transport: Transport,
    deadlines: Deadlines,
    endpoint: &str,
    started: Instant,
    first_ms: &mut Option<u64>,
    state: &mut StreamState,
    observer: Option<&(dyn Fn(&str) + Send + Sync)>,
    progress: Option<&ProgressFn>,
    policy: &RequestPolicy,
    diagnostics: &mut Vec<String>,
) -> Result<Completion, ProviderFailure> {
    let client = reqwest::Client::builder()
        .connect_timeout(deadlines.connect)
        .build()
        .map_err(|e| {
            ProviderFailure::from_error(Stage::Configuration, Category::Configuration, &e)
        })?;
    let kind = provider::effective_kind(secret);
    // One recovery for options the route rejects: learn them and resend
    // without them, so later dispatches skip them before sending.
    let mut resent = false;
    loop {
        let mut body = provider::chat_body(secret, messages, &[], transport == Transport::Stream);
        diagnostics.extend(crate::request_policy::apply(secret, &mut body, policy));
        match send_once(
            &client, secret, &body, transport, deadlines, endpoint, started, first_ms, state,
            observer, progress,
        )
        .await
        {
            Err(f) if !resent && matches!(f.http_status, Some(400 | 422)) && !policy.is_empty() => {
                let message = format!(
                    "{} {} {}",
                    f.message,
                    f.provider_message.as_deref().unwrap_or(""),
                    f.provider_code.as_deref().unwrap_or("")
                );
                let fields = crate::request_policy::rejected_fields(&body, &message);
                if fields.is_empty() {
                    return Err(f);
                }
                for field in fields {
                    crate::request_policy::learn_unsupported(&kind, &secret.model, field);
                    diagnostics.push(format!(
                        "unsupported-policy:{field} rejected; resent without it"
                    ));
                }
                resent = true;
            }
            other => return other,
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn send_once(
    client: &reqwest::Client,
    secret: &ProviderSecret,
    body: &Value,
    transport: Transport,
    deadlines: Deadlines,
    endpoint: &str,
    started: Instant,
    first_ms: &mut Option<u64>,
    state: &mut StreamState,
    observer: Option<&(dyn Fn(&str) + Send + Sync)>,
    progress: Option<&ProgressFn>,
) -> Result<Completion, ProviderFailure> {
    let req = provider::authorize(client.post(endpoint).json(body), secret)
        .await
        .map_err(|err| {
            let mut f = ProviderFailure::from_anyhow(Stage::Configuration, &err);
            if !f.category.needs_configuration() {
                f.category = Category::Configuration;
                f.retryable = false;
            }
            f
        })?;
    let first_deadline = deadlines.first_response.min(deadlines.total);
    if let Some(p) = progress {
        p(AttemptProgress::Dispatched);
    }
    let resp = match tokio::time::timeout(first_deadline, req.send()).await {
        Err(_) => {
            return Err(ProviderFailure::new(
                Stage::FirstResponse,
                Category::Timeout,
                format!("no response headers within {}s", first_deadline.as_secs()),
            ))
        }
        Ok(Err(err)) => {
            let (stage, category) = if err.is_timeout() {
                (Stage::Connect, Category::Timeout)
            } else {
                (Stage::Connect, Category::Network)
            };
            return Err(ProviderFailure::from_error(stage, category, &err));
        }
        Ok(Ok(resp)) => resp,
    };
    *first_ms = Some(started.elapsed().as_millis() as u64);
    let status = resp.status();
    if let Some(p) = progress {
        p(AttemptProgress::HeadersReceived {
            status: status.as_u16(),
        });
    }
    let req_id = request_id(resp.headers());
    let wait = retry_after_uncapped(resp.headers());
    let remaining = |started: Instant| deadlines.total.saturating_sub(started.elapsed());
    if !status.is_success() {
        let text = tokio::time::timeout(remaining(started), resp.text())
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or_default();
        let (code, message) = provider_error(&text);
        let category = classify_status(status.as_u16(), code.as_deref(), message.as_deref());
        let mut f = ProviderFailure::new(
            Stage::Response,
            category,
            format!("provider returned HTTP {}", status.as_u16()),
        );
        if let Some(m) = &message {
            f.causes.push(m.clone());
        }
        f.http_status = Some(status.as_u16());
        f.provider_code = code;
        f.provider_message = message;
        f.request_id = req_id;
        f.retry_after_ms = wait.map(|d| d.as_millis() as u64);
        return Err(f);
    }
    let attach = |mut f: ProviderFailure| {
        f.http_status = Some(status.as_u16());
        f.request_id = req_id.clone();
        f
    };
    let is_event_stream = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|ct| ct.starts_with("text/event-stream"))
        .unwrap_or(transport == Transport::Stream);

    if is_event_stream {
        read_stream(
            resp.bytes_stream(),
            deadlines,
            started.elapsed(),
            state,
            observer,
            progress,
        )
        .await
        .map_err(attach)
    } else {
        let text = match tokio::time::timeout(remaining(started), resp.text()).await {
            Err(_) => {
                return Err(attach(ProviderFailure::new(
                    Stage::Stream,
                    Category::Timeout,
                    "response body exceeded the total deadline",
                )))
            }
            Ok(Err(err)) => {
                return Err(attach(ProviderFailure::from_error(
                    Stage::Stream,
                    Category::StreamInterrupted,
                    &err,
                )))
            }
            Ok(Ok(text)) => text,
        };
        parse_final_json(&text).map_err(attach)
    }
}

/// Decode a non-streaming body into a final answer.
pub fn parse_final_json(text: &str) -> Result<Completion, ProviderFailure> {
    let v: Value = serde_json::from_str(text)
        .map_err(|e| ProviderFailure::from_error(Stage::Parse, Category::MalformedPayload, &e))?;
    if let Some(err) = v.get("error").filter(|e| !e.is_null()) {
        let (code, message) = provider_error(&err.to_string());
        let mut f = ProviderFailure::new(
            Stage::Parse,
            Category::SseError,
            "provider returned an error payload with HTTP 200",
        );
        f.provider_code = code;
        f.provider_message = message;
        return Err(f);
    }
    if v.pointer("/choices/0").is_none() {
        return Err(ProviderFailure::new(
            Stage::Parse,
            Category::MalformedPayload,
            "response JSON has no choices",
        ));
    }
    let msg = v
        .pointer("/choices/0/message")
        .cloned()
        .unwrap_or(Value::Null);
    let completion = Completion {
        content: provider::message_text(&msg),
        reasoning: provider::message_reasoning(&msg),
        tool_calls: Vec::new(),
        finish_reason: v
            .pointer("/choices/0/finish_reason")
            .and_then(Value::as_str)
            .map(str::to_string),
        refusal: msg
            .get("refusal")
            .and_then(Value::as_str)
            .map(str::to_string),
    };
    check_final(completion, None)
}

/// Reject truncated, refused or empty answers.
fn check_final(c: Completion, state: Option<&StreamState>) -> Result<Completion, ProviderFailure> {
    let partial = c.content.chars().count() as u64;
    let stage = if state.is_some() {
        Stage::Stream
    } else {
        Stage::Parse
    };
    if c.finish_reason.as_deref() == Some("length") {
        let mut f = ProviderFailure::new(
            stage,
            Category::TokenLimit,
            format!("finish_reason=length after {partial} characters"),
        );
        f.stream.partial_chars = partial;
        f.stream.finish_reason = c.finish_reason.clone();
        return Err(f);
    }
    if c.content.trim().is_empty() {
        if let Some(refusal) = c.refusal.as_deref().filter(|r| !r.trim().is_empty()) {
            let mut f = ProviderFailure::new(stage, Category::Refused, "model refused");
            f.provider_message = Some(crate::provider_diag::sanitize(refusal));
            return Err(f);
        }
        if !c.reasoning.trim().is_empty() {
            let mut f = ProviderFailure::new(
                stage,
                Category::Empty,
                format!(
                    "reasoning-only completion without answer text (finish_reason={})",
                    c.finish_reason.as_deref().unwrap_or("none")
                ),
            );
            f.stream.finish_reason = c.finish_reason.clone();
            return Err(f);
        }
        let mut f = ProviderFailure::new(
            stage,
            Category::Empty,
            format!(
                "empty answer (finish_reason={})",
                c.finish_reason.as_deref().unwrap_or("none")
            ),
        );
        f.stream.finish_reason = c.finish_reason.clone();
        return Err(f);
    }
    Ok(c)
}

#[derive(Default)]
struct Acc {
    content: String,
    reasoning: String,
    refusal: Option<String>,
    event: String,
}

/// Read an SSE chat stream.
///
/// Generic over the byte stream so tests can drive it with a mock stream and a
/// paused tokio clock. Bytes are buffered until a full line arrives, so UTF-8
/// sequences split across network chunks decode intact. Clocks use
/// `tokio::time::Instant`; `already` is time spent before the body.
pub(crate) async fn read_stream<S, B, E>(
    stream: S,
    deadlines: Deadlines,
    already: Duration,
    state: &mut StreamState,
    observer: Option<&(dyn Fn(&str) + Send + Sync)>,
    progress: Option<&ProgressFn>,
) -> Result<Completion, ProviderFailure>
where
    S: futures_util::Stream<Item = Result<B, E>>,
    B: AsRef<[u8]>,
    E: std::error::Error + 'static,
{
    use tokio::time::Instant as TInstant;
    let mut stream = std::pin::pin!(stream);
    let mut acc = Acc::default();
    let mut buf: Vec<u8> = Vec::new();
    let body_start = TInstant::now();
    let total_left = deadlines.total.saturating_sub(already);
    let mut last_model: Option<TInstant> = None;
    let mut last_progress: Option<TInstant> = None;
    loop {
        let now = TInstant::now();
        let remaining = total_left.saturating_sub(now - body_start);
        if remaining.is_zero() {
            return Err(stream_fail(
                state,
                &acc,
                Category::Timeout,
                "stream exceeded the total deadline",
            ));
        }
        // Model-output clock: first output, then inactivity since the last delta.
        let model_left = match (
            last_model,
            deadlines.first_output,
            deadlines.model_inactivity,
        ) {
            (None, Some(first), _) => Some((first.saturating_sub(now - body_start), true)),
            (Some(at), _, Some(idle)) => Some((idle.saturating_sub(now - at), false)),
            _ => None,
        };
        if let Some((left, first)) = model_left {
            if left.is_zero() {
                return Err(model_timeout(state, &acc, first, deadlines));
            }
        }
        let mut wait = deadlines.idle.min(remaining);
        if let Some((left, _)) = model_left {
            wait = wait.min(left);
        }
        let next = match tokio::time::timeout(wait, stream.next()).await {
            Err(_) => {
                let now = TInstant::now();
                if total_left.saturating_sub(now - body_start).is_zero() {
                    return Err(stream_fail(
                        state,
                        &acc,
                        Category::Timeout,
                        "stream exceeded the total deadline",
                    ));
                }
                if let Some((left, first)) = model_left {
                    if left <= wait {
                        return Err(model_timeout(state, &acc, first, deadlines));
                    }
                }
                return Err(stream_fail(
                    state,
                    &acc,
                    Category::Timeout,
                    &format!("no stream data for {}s", wait.as_secs()),
                ));
            }
            Ok(next) => next,
        };
        let chunk = match next {
            None => break,
            Some(Ok(chunk)) => chunk,
            Some(Err(err)) => {
                let mut f =
                    ProviderFailure::from_error(Stage::Stream, Category::StreamInterrupted, &err);
                f.message = format!(
                    "stream interrupted after {} chunks{}",
                    state.chunks,
                    if state.content_began {
                        " (content had begun)"
                    } else {
                        " (before content)"
                    }
                );
                f.causes.insert(0, f.message.clone());
                fill(state, &acc);
                f.stream = state.clone();
                return Err(f);
            }
        };
        state.chunks += 1;
        let chunk = chunk.as_ref();
        state.bytes += chunk.len() as u64;
        buf.extend_from_slice(chunk);
        let before = (acc.content.len(), acc.reasoning.len());
        let mut heartbeat = false;
        while let Some(idx) = buf.iter().position(|b| *b == b'\n') {
            let line_bytes: Vec<u8> = buf.drain(..=idx).collect();
            let line = String::from_utf8_lossy(&line_bytes).trim().to_string();
            if line.starts_with(':') {
                heartbeat = true;
            }
            line_event(&line, state, &mut acc, observer)?;
            if state.done_marker {
                break;
            }
        }
        let grew = (acc.content.len(), acc.reasoning.len()) != before;
        let now = TInstant::now();
        if grew {
            if last_model.is_none() {
                if let Some(p) = progress {
                    p(AttemptProgress::FirstModelOutput);
                }
            }
            last_model = Some(now);
            if let Some(p) = progress {
                if last_progress.is_none_or(|t| now - t >= PROGRESS_THROTTLE) {
                    last_progress = Some(now);
                    p(AttemptProgress::Receiving {
                        answer_chars: acc.content.chars().count() as u64,
                        reasoning_chars: acc.reasoning.chars().count() as u64,
                    });
                }
            }
        } else if heartbeat {
            if let Some(p) = progress {
                p(AttemptProgress::Heartbeat);
            }
        }
        // Honor the explicit terminal marker promptly instead of waiting for EOF.
        if state.done_marker {
            buf.clear();
            break;
        }
    }
    if !buf.is_empty() {
        let line = String::from_utf8_lossy(&buf).trim().to_string();
        if !line.is_empty() {
            line_event(&line, state, &mut acc, observer)?;
        }
    }
    if let Some(p) = progress {
        if state.done_marker || state.finish_reason.is_some() {
            p(AttemptProgress::StreamComplete {
                finish_reason: state.finish_reason.clone(),
            });
        }
    }
    fill(state, &acc);
    if !state.done_marker && state.finish_reason.is_none() {
        return Err(stream_fail(
            state,
            &acc,
            Category::PrematureEof,
            &format!(
                "stream closed without [DONE] or finish_reason after {} chunks{}",
                state.chunks,
                if state.content_began {
                    " (content had begun)"
                } else {
                    " (before content)"
                }
            ),
        ));
    }
    let completion = Completion {
        content: acc.content,
        reasoning: acc.reasoning,
        tool_calls: Vec::new(),
        finish_reason: state.finish_reason.clone(),
        refusal: acc.refusal.clone(),
    };
    check_final(completion, Some(state)).map_err(|mut f| {
        f.stream = state.clone();
        f
    })
}

fn model_timeout(
    state: &mut StreamState,
    acc: &Acc,
    first: bool,
    deadlines: Deadlines,
) -> ProviderFailure {
    let msg = if first {
        format!(
            "no model output within {}s of response headers",
            deadlines.first_output.unwrap_or_default().as_secs()
        )
    } else {
        format!(
            "model output inactive for {}s",
            deadlines.model_inactivity.unwrap_or_default().as_secs()
        )
    };
    stream_fail(state, acc, Category::Timeout, &msg)
}

fn fill(state: &mut StreamState, acc: &Acc) {
    state.partial_chars = (acc
        .content
        .chars()
        .count()
        .max(acc.reasoning.chars().count())) as u64;
    state.content_began = state.partial_chars > 0;
}

fn stream_fail(
    state: &mut StreamState,
    acc: &Acc,
    category: Category,
    message: &str,
) -> ProviderFailure {
    fill(state, acc);
    let mut f = ProviderFailure::new(Stage::Stream, category, message);
    f.stream = state.clone();
    f
}

fn line_event(
    line: &str,
    state: &mut StreamState,
    acc: &mut Acc,
    observer: Option<&(dyn Fn(&str) + Send + Sync)>,
) -> Result<(), ProviderFailure> {
    if line.is_empty() {
        acc.event.clear();
        return Ok(());
    }
    if line.starts_with(':') {
        return Ok(());
    }
    if let Some(name) = line.strip_prefix("event:") {
        acc.event = name.trim().to_string();
        return Ok(());
    }
    let Some(data) = line.strip_prefix("data:") else {
        // id:/retry: and unknown fields are ignored per the SSE spec.
        return Ok(());
    };
    let data = data.trim();
    if data.is_empty() {
        return Ok(());
    }
    state.events += 1;
    if data == "[DONE]" {
        state.done_marker = true;
        return Ok(());
    }
    let v: Value = match serde_json::from_str(data) {
        Ok(v) => v,
        Err(e) => {
            if acc.event == "error" {
                return Err(sse_error(state, acc, None, Some(data.to_string())));
            }
            let mut f = ProviderFailure::from_error(Stage::Stream, Category::MalformedPayload, &e);
            f.message = format!("unreadable stream event #{}", state.events);
            f.causes.insert(0, f.message.clone());
            fill(state, acc);
            f.stream = state.clone();
            return Err(f);
        }
    };
    if acc.event == "error" || v.get("error").is_some_and(|e| !e.is_null()) {
        let err = v.get("error").cloned().unwrap_or(v.clone());
        let (code, message) = provider_error(&serde_json::json!({ "error": err }).to_string());
        return Err(sse_error(state, acc, code, message));
    }
    if let Some(reason) = v
        .pointer("/choices/0/finish_reason")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        state.finish_reason = Some(reason.to_string());
    }
    if let Some(delta) = v.pointer("/choices/0/delta") {
        if let Some(t) = delta.get("content").and_then(Value::as_str) {
            acc.content.push_str(t);
            if let Some(obs) = observer {
                obs(t);
            }
        }
        for key in ["reasoning_content", "reasoning"] {
            if let Some(t) = delta.get(key).and_then(Value::as_str) {
                acc.reasoning.push_str(t);
            }
        }
        if let Some(r) = delta
            .get("refusal")
            .and_then(Value::as_str)
            .filter(|r| !r.trim().is_empty())
        {
            acc.refusal = Some(r.to_string());
        }
    }
    fill(state, acc);
    Ok(())
}

fn sse_error(
    state: &mut StreamState,
    acc: &Acc,
    code: Option<String>,
    message: Option<String>,
) -> ProviderFailure {
    let mut f = stream_fail(
        state,
        acc,
        Category::SseError,
        "provider sent an error event in the stream",
    );
    let lower = format!(
        "{} {}",
        code.as_deref().unwrap_or(""),
        message.as_deref().unwrap_or("")
    )
    .to_ascii_lowercase();
    if lower.contains("invalid_api_key") || lower.contains("unauthorized") {
        f.category = Category::Auth;
        f.retryable = false;
    } else if lower.contains("context_length") || lower.contains("maximum context") {
        f.category = Category::MalformedRequest;
        f.retryable = false;
    }
    if let Some(m) = &message {
        f.causes.push(crate::provider_diag::sanitize(m));
    }
    f.provider_code = code;
    f.provider_message = message.map(|m| crate::provider_diag::sanitize(&m));
    f
}

/// Scripted local provider for fault-injection tests.
#[cfg(any(test, feature = "fixtures"))]
pub mod mock {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// One scripted reply. `Raw` is written verbatim and the socket closed.
    #[derive(Clone, Debug)]
    pub enum Reply {
        Json(u16, String),
        Sse(Vec<String>),
        Raw(String),
        /// Headers for a chunked/SSE body, then `body` and an abrupt close.
        Truncated(String),
    }

    pub struct Server {
        pub base_url: String,
        pub hits: Arc<AtomicUsize>,
        pub requests: Arc<Mutex<Vec<String>>>,
    }

    impl Server {
        pub fn hits(&self) -> usize {
            self.hits.load(Ordering::SeqCst)
        }
    }

    pub fn sse_reply(chunks: &[&str]) -> Reply {
        Reply::Sse(chunks.iter().map(|s| s.to_string()).collect())
    }

    pub fn delta(text: &str) -> String {
        format!(
            "data: {}\n\n",
            serde_json::json!({"choices":[{"delta":{"content":text}}]})
        )
    }

    pub fn finish(reason: &str) -> String {
        format!(
            "data: {}\n\n",
            serde_json::json!({"choices":[{"delta":{},"finish_reason":reason}]})
        )
    }

    pub fn ok_json(content: &str, finish: &str) -> Reply {
        Reply::Json(
            200,
            serde_json::json!({"choices":[{"finish_reason":finish,"message":{"role":"assistant","content":content}}]})
                .to_string(),
        )
    }

    /// Serve `script` in order (the last reply repeats).
    pub async fn serve(script: Vec<Reply>) -> Server {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let hits = Arc::new(AtomicUsize::new(0));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let (h, r) = (hits.clone(), requests.clone());
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                let n = h.fetch_add(1, Ordering::SeqCst);
                let reply = script.get(n).or(script.last()).cloned();
                let r = r.clone();
                tokio::spawn(async move {
                    let request = read_request(&mut socket).await;
                    r.lock().unwrap().push(request);
                    let Some(reply) = reply else { return };
                    let bytes = match reply {
                        Reply::Json(status, body) => format!(
                            "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                            body.len()
                        ),
                        Reply::Sse(chunks) => {
                            let body: String = chunks.concat();
                            format!(
                                "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                                body.len()
                            )
                        }
                        Reply::Raw(raw) => raw,
                        Reply::Truncated(body) => format!(
                            "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                            body.len() + 4096
                        ),
                    };
                    let _ = socket.write_all(bytes.as_bytes()).await;
                    let _ = socket.shutdown().await;
                });
            }
        });
        Server {
            base_url: format!("http://127.0.0.1:{port}/v1"),
            hits,
            requests,
        }
    }

    async fn read_request(socket: &mut tokio::net::TcpStream) -> String {
        let mut buffer = vec![0u8; 65536];
        let mut request = String::new();
        loop {
            let n = socket.read(&mut buffer).await.unwrap_or(0);
            if n == 0 {
                break;
            }
            request.push_str(&String::from_utf8_lossy(&buffer[..n]));
            let Some(end) = request.find("\r\n\r\n") else {
                continue;
            };
            let length = request[..end]
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|v| v.trim().parse::<usize>().ok())
                })
                .unwrap_or(0);
            if request.len() >= end + 4 + length {
                break;
            }
        }
        request
    }

    pub fn secret(base_url: &str) -> crate::secrets::ProviderSecret {
        crate::secrets::ProviderSecret {
            kind: "openrouter".into(),
            base_url: base_url.to_string(),
            model: "mock-model".into(),
            api_key: Some("sk-mocksecretvalue12345".into()),
            stt_model: None,
            device: None,
        }
    }
}

#[cfg(test)]
#[path = "provider_attempt_policy_tests.rs"]
mod policy_tests;

#[cfg(test)]
mod tests {
    use super::mock::*;
    use super::*;

    fn msgs() -> Vec<ChatMessage> {
        vec![ChatMessage {
            role: "user".into(),
            content: "Explain".into(),
            tool_call_id: None,
            tool_calls: Vec::new(),
        }]
    }

    async fn run(reply: Reply, transport: Transport) -> (AttemptReport, usize) {
        let server = serve(vec![reply]).await;
        let report = attempt(
            &secret(&server.base_url),
            &msgs(),
            transport,
            Deadlines::default(),
        )
        .await;
        (report, server.hits())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn premature_eof_before_and_after_content_is_typed() {
        let (r, hits) = run(sse_reply(&[]), Transport::Stream).await;
        let f = r.outcome.unwrap_err();
        assert_eq!(
            (f.category, f.stage, hits),
            (Category::PrematureEof, Stage::Stream, 1)
        );
        assert!(!f.stream.content_began);

        let body = format!("{}{}", delta("Partial "), delta("answer"));
        let (r, _) = run(Reply::Sse(vec![body]), Transport::Stream).await;
        let f = r.outcome.unwrap_err();
        assert_eq!(f.category, Category::PrematureEof);
        assert!(f.stream.content_began);
        assert_eq!(f.stream.partial_chars, "Partial answer".len() as u64);
        assert!(f.detail_lines().join("\n").contains("discarded"));

        // Connection closed short of the announced length: interrupted stream.
        let (r, _) = run(Reply::Truncated(delta("Half")), Transport::Stream).await;
        let f = r.outcome.unwrap_err();
        assert!(
            matches!(
                f.category,
                Category::StreamInterrupted | Category::PrematureEof
            ),
            "{f:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn sse_error_malformed_and_token_limit_are_typed() {
        let err = "event: error\ndata: {\"error\":{\"message\":\"overloaded\",\"code\":\"server_busy\"}}\n\n".to_string();
        let (r, _) = run(Reply::Sse(vec![delta("Some"), err]), Transport::Stream).await;
        let f = r.outcome.unwrap_err();
        assert_eq!(f.category, Category::SseError);
        assert_eq!(f.provider_code.as_deref(), Some("server_busy"));
        assert!(f.retryable);

        let (r, _) = run(
            Reply::Sse(vec!["data: {not json\n\n".into()]),
            Transport::Stream,
        )
        .await;
        assert_eq!(r.outcome.unwrap_err().category, Category::MalformedPayload);

        let (r, _) = run(
            Reply::Sse(vec![
                delta("Cut"),
                finish("length"),
                "data: [DONE]\n\n".into(),
            ]),
            Transport::Stream,
        )
        .await;
        let f = r.outcome.unwrap_err();
        assert_eq!(f.category, Category::TokenLimit);
        assert!(!f.retryable);

        let (r, _) = run(
            Reply::Json(200, "{\"choices\":[".into()),
            Transport::NonStream,
        )
        .await;
        assert_eq!(r.outcome.unwrap_err().category, Category::MalformedPayload);

        let (r, _) = run(ok_json("", "stop"), Transport::NonStream).await;
        assert_eq!(r.outcome.unwrap_err().category, Category::Empty);

        let (r, _) = run(ok_json("Cut off", "length"), Transport::NonStream).await;
        assert_eq!(r.outcome.unwrap_err().category, Category::TokenLimit);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn complete_stream_and_json_answers_succeed() {
        let (r, _) = run(
            Reply::Sse(vec![
                delta("Hello "),
                delta("graph"),
                finish("stop"),
                "data: [DONE]\n\n".into(),
            ]),
            Transport::Stream,
        )
        .await;
        assert_eq!(r.outcome.unwrap().content, "Hello graph");
        let (r, hits) = run(ok_json("Final", "stop"), Transport::NonStream).await;
        assert_eq!((r.outcome.unwrap().content.as_str(), hits), ("Final", 1));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn reasoning_only_stream_and_json_do_not_promote_to_answer() {
        let json_reasoning = serde_json::json!({
            "choices": [{
                "finish_reason": "stop",
                "message": {
                    "role": "assistant",
                    "content": "",
                    "reasoning_content": "Internal deliberation"
                }
            }]
        })
        .to_string();
        let (r, _) = run(Reply::Json(200, json_reasoning), Transport::NonStream).await;
        let err = r.outcome.unwrap_err();
        assert_eq!(err.category, Category::Empty);
        assert!(err.message.contains("reasoning-only completion"));

        let stream_reasoning = vec![
            "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"Deliberating\"}}]}\n\n"
                .to_string(),
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n".to_string(),
            "data: [DONE]\n\n".to_string(),
        ];
        let (r, _) = run(Reply::Sse(stream_reasoning), Transport::Stream).await;
        let err = r.outcome.unwrap_err();
        assert_eq!(err.category, Category::Empty);
        assert!(err.message.contains("reasoning-only completion"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn http_errors_carry_status_code_and_redacted_message() {
        let body = r#"{"error":{"message":"Incorrect API key provided: sk-mocksecretvalue12345","code":"invalid_api_key"}}"#;
        let (r, _) = run(Reply::Json(401, body.into()), Transport::NonStream).await;
        let f = r.outcome.unwrap_err();
        assert_eq!((f.category, f.http_status), (Category::Auth, Some(401)));
        assert!(f.guidance().unwrap().contains("Providers"));
        let all = serde_json::to_string(&f).unwrap();
        assert!(!all.contains("sk-mocksecretvalue12345"), "{all}");
        let raw = "HTTP/1.1 429 Too Many\r\nretry-after: 3\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}".to_string();
        let (r, _) = run(Reply::Raw(raw), Transport::NonStream).await;
        let f = r.outcome.unwrap_err();
        assert_eq!(
            (f.category, f.retry_after_ms),
            (Category::RateLimited, Some(3000))
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn other_complete_callers_get_typed_errors_without_extra_requests() {
        let body = r#"{"error":{"message":"slow down","code":"rate_limit"}}"#;
        let server = serve(vec![Reply::Json(429, body.into())]).await;
        let err = crate::provider::complete(&secret(&server.base_url), &msgs(), &[], |_| {})
            .await
            .unwrap_err();
        let f = err
            .downcast_ref::<ProviderFailure>()
            .expect("typed failure");
        assert_eq!(
            (f.category, f.http_status),
            (Category::RateLimited, Some(429))
        );
        assert!(err.to_string().contains("429"), "{err}");
        assert_eq!(server.hits(), 1, "no new retries for existing callers");
    }
}

#[cfg(test)]
#[path = "provider_attempt_stream_tests.rs"]
mod stream_tests;
