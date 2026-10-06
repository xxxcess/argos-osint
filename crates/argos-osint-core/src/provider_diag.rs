//! Typed provider diagnostics.
//!
//! A [`ProviderFailure`] keeps everything needed to explain one failed
//! provider exchange without guessing: the stage that failed, a stable
//! category, HTTP status and provider error code, the full sanitized cause
//! chain (nested causes included), stream state and timing. Every string that
//! can carry provider text, endpoint URLs or headers passes through
//! [`crate::events::redact`] before it is stored.

use std::fmt;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Where in the exchange the failure happened.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    /// Credentials, model or connection settings (before any request).
    Configuration,
    /// Waiting for the shared provider admission slot.
    Admission,
    /// Opening the connection or sending the request.
    Connect,
    /// Waiting for response headers.
    FirstResponse,
    /// Non-success HTTP status.
    Response,
    /// Reading the response body or event stream.
    Stream,
    /// Decoding the provider payload.
    Parse,
    /// Checking the answer is a usable result.
    Validation,
    /// Saving the result.
    Persistence,
}

impl Stage {
    pub fn label(self) -> &'static str {
        match self {
            Stage::Configuration => "configuration",
            Stage::Admission => "admission",
            Stage::Connect => "connect",
            Stage::FirstResponse => "first response",
            Stage::Response => "response",
            Stage::Stream => "stream",
            Stage::Parse => "parse",
            Stage::Validation => "validation",
            Stage::Persistence => "persistence",
        }
    }
}

/// Stable failure category. Drives retry decisions and user guidance.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Auth,
    Permission,
    InvalidModel,
    Configuration,
    MalformedRequest,
    /// The endpoint rejected this transport (for example streaming).
    Unsupported,
    RateLimited,
    Server,
    Timeout,
    Network,
    /// The connection dropped while reading the body.
    StreamInterrupted,
    /// The stream ended without a completion marker or finish reason.
    PrematureEof,
    /// The provider sent an error event inside a successful stream.
    SseError,
    MalformedPayload,
    /// Output stopped at the token limit (`finish_reason=length`).
    TokenLimit,
    Refused,
    Empty,
    InvalidResult,
    Persistence,
    Cancelled,
}

impl Category {
    pub fn as_str(self) -> &'static str {
        match self {
            Category::Auth => "auth",
            Category::Permission => "permission",
            Category::InvalidModel => "invalid_model",
            Category::Configuration => "configuration",
            Category::MalformedRequest => "malformed_request",
            Category::Unsupported => "unsupported",
            Category::RateLimited => "rate_limited",
            Category::Server => "server",
            Category::Timeout => "timeout",
            Category::Network => "network",
            Category::StreamInterrupted => "stream_interrupted",
            Category::PrematureEof => "premature_eof",
            Category::SseError => "sse_error",
            Category::MalformedPayload => "malformed_payload",
            Category::TokenLimit => "token_limit",
            Category::Refused => "refused",
            Category::Empty => "empty",
            Category::InvalidResult => "invalid_result",
            Category::Persistence => "persistence",
            Category::Cancelled => "cancelled",
        }
    }

    /// Another request with the same settings may succeed.
    pub fn retryable(self) -> bool {
        matches!(
            self,
            Category::RateLimited
                | Category::Server
                | Category::Timeout
                | Category::Network
                | Category::StreamInterrupted
                | Category::PrematureEof
                | Category::SseError
                | Category::MalformedPayload
                | Category::Empty
                | Category::InvalidResult
        )
    }

    /// The user has to change a setting; retrying is pointless.
    pub fn needs_configuration(self) -> bool {
        matches!(
            self,
            Category::Auth
                | Category::Permission
                | Category::InvalidModel
                | Category::Configuration
        )
    }

    /// Short human phrase.
    pub fn reason(self) -> &'static str {
        match self {
            Category::Auth => "provider rejected the credentials",
            Category::Permission => "the account cannot use this model or endpoint",
            Category::InvalidModel => "the configured model was not found",
            Category::Configuration => "the summarization provider is not configured",
            Category::MalformedRequest => "the provider rejected the request",
            Category::Unsupported => "the endpoint does not support this request mode",
            Category::RateLimited => "the provider is rate limiting requests",
            Category::Server => "the provider had a server error",
            Category::Timeout => "the provider did not answer in time",
            Category::Network => "could not reach the provider",
            Category::StreamInterrupted => "the connection dropped while the answer was streaming",
            Category::PrematureEof => "the answer stream ended early",
            Category::SseError => "the provider sent an error event mid-stream",
            Category::MalformedPayload => "the provider sent an unreadable response",
            Category::TokenLimit => "the answer was cut off at the token limit",
            Category::Refused => "the model declined to answer",
            Category::Empty => "the provider returned an empty answer",
            Category::InvalidResult => "the answer was not a usable summary",
            Category::Persistence => "the summary could not be saved",
            Category::Cancelled => "the request was cancelled",
        }
    }

    /// Mapping onto the durable task error categories.
    pub fn task_category(self) -> crate::tasks::ErrorCategory {
        use crate::tasks::ErrorCategory as E;
        match self {
            Category::Auth | Category::Permission => E::AuthOrQuota,
            Category::InvalidModel | Category::Configuration => E::ConfigurationMissing,
            Category::Unsupported => E::UnsupportedOption,
            Category::RateLimited => E::RateLimit,
            Category::Server | Category::Network => E::TemporaryNetwork,
            Category::Timeout => E::Timeout,
            Category::StreamInterrupted | Category::PrematureEof | Category::SseError => {
                E::InterruptedStream
            }
            Category::TokenLimit => E::ContextLimit,
            Category::Cancelled => E::CancelledOrStale,
            Category::Persistence => E::Unknown,
            _ => E::InvalidResult,
        }
    }
}

/// What a streamed answer looked like when it stopped.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamState {
    pub chunks: u64,
    pub bytes: u64,
    pub events: u64,
    pub content_began: bool,
    pub done_marker: bool,
    pub finish_reason: Option<String>,
    /// Characters of answer text received before the failure. The text itself
    /// is never kept.
    pub partial_chars: u64,
}

/// One failed provider exchange.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProviderFailure {
    pub stage: Stage,
    pub category: Category,
    pub retryable: bool,
    /// Concise sanitized message (top of the chain).
    pub message: String,
    /// Full sanitized cause chain, outermost first.
    pub causes: Vec<String>,
    pub http_status: Option<u16>,
    pub provider_code: Option<String>,
    pub provider_message: Option<String>,
    pub request_id: Option<String>,
    /// Endpoint with credentials and query string removed.
    pub endpoint: String,
    pub provider: String,
    pub model: String,
    /// `stream`, `non_stream` or `subscription`.
    pub transport: String,
    pub retry_after_ms: Option<u64>,
    pub elapsed_ms: u64,
    pub first_response_ms: Option<u64>,
    pub stream: StreamState,
}

impl ProviderFailure {
    pub fn new(stage: Stage, category: Category, message: impl AsRef<str>) -> Self {
        let message = sanitize(message.as_ref());
        Self {
            stage,
            category,
            retryable: category.retryable(),
            causes: vec![message.clone()],
            message,
            http_status: None,
            provider_code: None,
            provider_message: None,
            request_id: None,
            endpoint: String::new(),
            provider: String::new(),
            model: String::new(),
            transport: String::new(),
            retry_after_ms: None,
            elapsed_ms: 0,
            first_response_ms: None,
            stream: StreamState::default(),
        }
    }

    /// Build from any error, keeping every nested cause (sanitized).
    pub fn from_error(
        stage: Stage,
        category: Category,
        err: &(dyn std::error::Error + 'static),
    ) -> Self {
        let causes = cause_chain(err);
        let mut failure = Self::new(stage, category, causes.first().cloned().unwrap_or_default());
        failure.causes = causes;
        failure
    }

    /// Build from an anyhow error, classifying it from its text when no typed
    /// failure is inside.
    pub fn from_anyhow(stage: Stage, err: &anyhow::Error) -> Self {
        if let Some(inner) = err.downcast_ref::<ProviderFailure>() {
            return inner.clone();
        }
        let causes: Vec<String> = err.chain().map(|c| sanitize(&c.to_string())).collect();
        let joined = causes.join(": ");
        let category = classify_text(&joined);
        let mut failure = Self::new(stage, category, causes.first().cloned().unwrap_or_default());
        failure.causes = causes;
        failure
    }

    pub fn with_context(
        mut self,
        provider: &str,
        model: &str,
        endpoint: &str,
        transport: &str,
    ) -> Self {
        self.provider = provider.to_string();
        self.model = model.to_string();
        self.endpoint = sanitize_endpoint(endpoint);
        self.transport = transport.to_string();
        self
    }

    pub fn needs_configuration(&self) -> bool {
        self.category.needs_configuration()
    }

    /// One-line reason for cards and job rows.
    pub fn summary(&self) -> String {
        let mut out = self.category.reason().to_string();
        if let Some(status) = self.http_status {
            out.push_str(&format!(" (HTTP {status})"));
        }
        if let Some(msg) = self
            .provider_message
            .as_deref()
            .map(str::trim)
            .filter(|m| !m.is_empty())
        {
            out.push_str(&format!(": {}", bounded(msg, 160)));
        }
        out
    }

    /// What the user should do, when a setting is wrong.
    pub fn guidance(&self) -> Option<String> {
        let model = if self.model.is_empty() {
            "the configured model".to_string()
        } else {
            format!("`{}`", self.model)
        };
        let provider = if self.provider.is_empty() {
            "the provider".to_string()
        } else {
            self.provider.clone()
        };
        match self.category {
            Category::Auth => Some(format!(
                "Reconnect {provider} in Providers or replace its API key, then retry."
            )),
            Category::Permission => Some(format!(
                "This {provider} account cannot use {model}. Choose another Summarization model in Models."
            )),
            Category::InvalidModel => Some(format!(
                "{model} is not available from {provider}. Choose another Summarization model in Models."
            )),
            Category::Configuration => Some(
                "Set a Summarization provider and model in Models, then retry.".to_string(),
            ),
            Category::TokenLimit => Some(
                "The answer hit the model's output limit. Retry, or pick a model with a larger output budget."
                    .to_string(),
            ),
            _ => None,
        }
    }

    /// Detail lines for "View details" (all sanitized).
    pub fn detail_lines(&self) -> Vec<String> {
        let mut lines = vec![
            format!("Reason: {}", self.summary()),
            format!(
                "Stage: {} · category: {}{}",
                self.stage.label(),
                self.category.as_str(),
                if self.retryable { " · retryable" } else { "" }
            ),
        ];
        if !self.provider.is_empty() || !self.model.is_empty() {
            lines.push(format!(
                "Provider: {} · model: {}",
                self.provider, self.model
            ));
        }
        if !self.endpoint.is_empty() {
            lines.push(format!("Endpoint: {} ({})", self.endpoint, self.transport));
        }
        let mut http = Vec::new();
        if let Some(status) = self.http_status {
            http.push(format!("HTTP {status}"));
        }
        if let Some(code) = &self.provider_code {
            http.push(format!("provider code {code}"));
        }
        if let Some(id) = &self.request_id {
            http.push(format!("request id {id}"));
        }
        if let Some(ms) = self.retry_after_ms {
            http.push(format!("retry after {}s", ms.div_ceil(1000)));
        }
        if !http.is_empty() {
            lines.push(http.join(" · "));
        }
        let mut timing = format!("Elapsed: {} ms", self.elapsed_ms);
        if let Some(ms) = self.first_response_ms {
            timing.push_str(&format!(" · first response {ms} ms"));
        }
        lines.push(timing);
        if self.transport == "stream" {
            let s = &self.stream;
            lines.push(format!(
                "Stream: {} chunks · {} bytes · {} events · content began: {} · done marker: {} · finish: {} · partial: {} chars (discarded)",
                s.chunks,
                s.bytes,
                s.events,
                if s.content_began { "yes" } else { "no" },
                if s.done_marker { "yes" } else { "no" },
                s.finish_reason.as_deref().unwrap_or("none"),
                s.partial_chars
            ));
        }
        for (i, cause) in self.causes.iter().enumerate() {
            let label = if i == 0 { "Error" } else { "Caused by" };
            lines.push(format!("{label}: {cause}"));
        }
        lines
    }

    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

impl fmt::Display for ProviderFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.summary())?;
        if f.alternate() {
            for cause in &self.causes {
                write!(f, ": {cause}")?;
            }
        }
        Ok(())
    }
}

impl std::error::Error for ProviderFailure {}

/// Every cause of an error, outermost first, sanitized.
pub fn cause_chain(err: &(dyn std::error::Error + 'static)) -> Vec<String> {
    let mut out = Vec::new();
    let mut current: Option<&(dyn std::error::Error + 'static)> = Some(err);
    while let Some(e) = current {
        let text = sanitize(&e.to_string());
        if out.last() != Some(&text) {
            out.push(text);
        }
        current = e.source();
        if out.len() > 16 {
            break;
        }
    }
    out
}

/// Redact secrets and strip URL query strings / credentials from free text.
pub fn sanitize(text: &str) -> String {
    let redacted = crate::events::redact(text);
    let mut out = String::with_capacity(redacted.len());
    let mut rest = redacted.as_str();
    while let Some(pos) = find_url(rest) {
        out.push_str(&rest[..pos]);
        let tail = &rest[pos..];
        let end = tail
            .find(|c: char| c.is_whitespace() || matches!(c, ')' | '"' | '\'' | '>' | ','))
            .unwrap_or(tail.len());
        out.push_str(&sanitize_endpoint(&tail[..end]));
        rest = &tail[end..];
    }
    out.push_str(rest);
    bounded(&out, 2000)
}

fn find_url(text: &str) -> Option<usize> {
    match (text.find("http://"), text.find("https://")) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// Endpoint shown in diagnostics: scheme, host, port and path only.
pub fn sanitize_endpoint(endpoint: &str) -> String {
    match url::Url::parse(endpoint) {
        Ok(mut url) => {
            let had_query = url.query().is_some();
            let _ = url.set_username("");
            let _ = url.set_password(None);
            url.set_query(None);
            url.set_fragment(None);
            let mut s = url.to_string();
            if had_query {
                s.push_str("?[redacted]");
            }
            s
        }
        Err(_) => crate::events::redact(endpoint),
    }
}

fn bounded(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut s: String = text.chars().take(max).collect();
    s.push('…');
    s
}

/// Classify an untyped error message.
pub fn classify_text(text: &str) -> Category {
    let t = text.to_ascii_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|n| t.contains(n));
    if has(&[
        "not configured",
        "no grok subscription login",
        "no summarization",
        "not signed in",
        "login required",
    ]) {
        Category::Configuration
    } else if has(&[
        "401",
        "unauthorized",
        "invalid api key",
        "invalid_api_key",
        "authentication",
    ]) {
        Category::Auth
    } else if has(&["403", "forbidden", "permission"]) {
        Category::Permission
    } else if has(&[
        "model_not_found",
        "model not found",
        "no such model",
        "unknown model",
    ]) {
        Category::InvalidModel
    } else if has(&["429", "rate limit", "too many requests"]) {
        Category::RateLimited
    } else if has(&["timed out", "timeout", "deadline"]) {
        Category::Timeout
    } else if has(&[
        " 500",
        " 502",
        " 503",
        " 504",
        "server error",
        "bad gateway",
        "unavailable",
    ]) {
        Category::Server
    } else if has(&[
        "connection reset",
        "connection refused",
        "error sending request",
        "dns",
        "connect",
    ]) {
        Category::Network
    } else if has(&["empty completion", "empty answer"]) {
        Category::Empty
    } else if has(&["json", "decode", "parse"]) {
        Category::MalformedPayload
    } else {
        Category::Server
    }
}

/// Classify a non-success HTTP status with the provider's error body.
pub fn classify_status(status: u16, code: Option<&str>, message: Option<&str>) -> Category {
    let text = format!("{} {}", code.unwrap_or(""), message.unwrap_or("")).to_ascii_lowercase();
    let mentions_model = text.contains("model");
    let mentions_stream = text.contains("stream");
    match status {
        401 => Category::Auth,
        402 | 403 => Category::Permission,
        404 if mentions_model => Category::InvalidModel,
        404 => Category::Unsupported,
        400 | 422 if mentions_stream => Category::Unsupported,
        400 | 422
            if mentions_model
                && (text.contains("not found")
                    || text.contains("invalid")
                    || text.contains("does not exist")
                    || text.contains("unknown")) =>
        {
            Category::InvalidModel
        }
        400 | 413 | 422 => Category::MalformedRequest,
        408 => Category::Timeout,
        409 | 425 | 429 => Category::RateLimited,
        500..=599 => Category::Server,
        _ => Category::MalformedRequest,
    }
}

/// Provider error JSON (`{"error":{"message","code","type"}}`, `{"message"}`, …).
pub fn provider_error(body: &str) -> (Option<String>, Option<String>) {
    let Ok(v) = serde_json::from_str::<Value>(body) else {
        let trimmed = body.trim();
        return (
            None,
            (!trimmed.is_empty()).then(|| sanitize(&bounded(trimmed, 300))),
        );
    };
    let err = v.get("error").unwrap_or(&v);
    let text = |key: &str| -> Option<String> {
        match err.get(key) {
            Some(Value::String(s)) if !s.trim().is_empty() => Some(s.trim().to_string()),
            Some(Value::Number(n)) => Some(n.to_string()),
            _ => None,
        }
    };
    let code = text("code")
        .or_else(|| text("type"))
        .or_else(|| text("status"));
    let message = text("message").or_else(|| match err {
        Value::String(s) => Some(s.clone()),
        _ => None,
    });
    (
        code.map(|c| sanitize(&c)),
        message.map(|m| sanitize(&bounded(&m, 300))),
    )
}

/// `Retry-After` in seconds (HTTP-date values are ignored), capped.
pub fn retry_after(headers: &reqwest::header::HeaderMap, cap: Duration) -> Option<Duration> {
    let value = headers.get(reqwest::header::RETRY_AFTER)?.to_str().ok()?;
    let secs: f64 = value.trim().parse().ok()?;
    if !secs.is_finite() || secs < 0.0 {
        return None;
    }
    Some(Duration::from_secs_f64(secs).min(cap))
}

/// Request id header from common providers.
pub fn request_id(headers: &reqwest::header::HeaderMap) -> Option<String> {
    for name in [
        "x-request-id",
        "request-id",
        "x-amzn-requestid",
        "x-generation-id",
        "cf-ray",
    ] {
        if let Some(v) = headers.get(name).and_then(|v| v.to_str().ok()) {
            if !v.trim().is_empty() {
                return Some(sanitize(&bounded(v.trim(), 120)));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct Leaf(String);
    impl fmt::Display for Leaf {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(&self.0)
        }
    }
    impl std::error::Error for Leaf {}

    #[test]
    fn nested_causes_and_endpoints_are_kept_and_redacted() {
        let err = anyhow::Error::new(Leaf(
            "connection reset by peer at https://user:pw@api.example.com/v1/chat?key=SECRETVALUE123".into(),
        ))
        .context("read body with Authorization: Bearer sk-abcdefghijklmnop1234")
        .context("POST https://api.example.com/v1/chat/completions?api_key=topsecret999");
        let failure = ProviderFailure::from_anyhow(Stage::Stream, &err);
        assert_eq!(failure.causes.len(), 3, "{:?}", failure.causes);
        let all = failure.causes.join("\n");
        for secret in [
            "SECRETVALUE123",
            "topsecret999",
            "sk-abcdefghijklmnop1234",
            "user:pw",
        ] {
            assert!(!all.contains(secret), "leaked {secret}: {all}");
        }
        assert!(all.contains("api.example.com/v1/chat/completions"));
        assert!(all.contains("connection reset"));
        assert_eq!(failure.category, Category::Network);
        let detail = failure.detail_lines().join("\n");
        assert!(detail.contains("Caused by: read body"));
    }

    #[test]
    fn statuses_map_to_configuration_or_retryable_categories() {
        assert_eq!(classify_status(401, None, None), Category::Auth);
        assert!(Category::Auth.needs_configuration());
        assert_eq!(
            classify_status(
                404,
                Some("model_not_found"),
                Some("The model `x` does not exist")
            ),
            Category::InvalidModel
        );
        assert_eq!(
            classify_status(400, None, Some("stream not supported")),
            Category::Unsupported
        );
        assert_eq!(classify_status(429, None, None), Category::RateLimited);
        assert!(classify_status(503, None, None).retryable());
        assert!(!Category::TokenLimit.retryable());
        let (code, msg) = provider_error(
            r#"{"error":{"message":"bad key sk-abcdefghijklmnop99","code":"invalid_api_key"}}"#,
        );
        assert_eq!(code.as_deref(), Some("invalid_api_key"));
        assert!(!msg.unwrap().contains("sk-abcdefghijklmnop99"));
    }

    #[test]
    fn endpoint_strips_query_and_userinfo() {
        let s = sanitize_endpoint("https://u:p@host.test:8443/v1/chat?key=abc#frag");
        assert_eq!(s, "https://host.test:8443/v1/chat?[redacted]");
    }
}
