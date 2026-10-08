//! Bounded summarization execution with a typed report.
//!
//! [`complete_summary_report`] spends at most [`MAX_REQUESTS`] outbound
//! requests on one summary, stream/non-stream fallbacks included. Waiting for
//! the shared provider admission slot never consumes an attempt. Every
//! attempt is reported with its typed [`ProviderFailure`], so callers can show
//! exactly why a summary is missing.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::{validate_result, SummarizationMode};
use crate::provider::ChatMessage;
use crate::provider_attempt::{self, Deadlines, Transport};
use crate::provider_diag::{Category, ProviderFailure, Stage};
use crate::provider_request::TimeoutProfile;
use crate::secrets::ProviderSecret;
use crate::tasks::{self, AdmissionGuard};

/// Outbound requests one summary execution may send on the primary route.
/// Shared chain policy: initial + 3 retries. Fallbacks add 3 each.
pub const MAX_REQUESTS: u32 = crate::provider_chain::PRIMARY_ATTEMPTS;

/// Execution limits.
#[derive(Clone, Debug)]
pub struct ExecOptions {
    pub deadlines: Deadlines,
    /// First transport; the default prefers a final-only non-streaming request.
    pub first_transport: Transport,
    pub admission_account: String,
    /// Longest wait for an admission slot before giving up (no request sent).
    pub admission_timeout: Duration,
    pub admission_poll: Duration,
    /// Cap on backoff / Retry-After waits between the two requests.
    pub max_backoff: Duration,
    pub cancel: Option<Arc<AtomicBool>>,
}

impl ExecOptions {
    pub fn for_secret(secret: &ProviderSecret) -> Self {
        let p = TimeoutProfile::SUMMARIZATION;
        Self {
            deadlines: Deadlines {
                connect: Duration::from_secs(p.connection),
                first_response: Duration::from_secs(p.first_response),
                idle: Duration::from_secs(p.stream_inactivity),
                total: Duration::from_secs(p.attempt_execution),
            },
            first_transport: Transport::NonStream,
            admission_account: admission_account(secret),
            admission_timeout: Duration::from_secs(120),
            admission_poll: Duration::from_millis(250),
            max_backoff: Duration::from_secs(60),
            cancel: None,
        }
    }
}

/// Shared admission key for a provider connection.
pub fn admission_account(secret: &ProviderSecret) -> String {
    let host = url::Url::parse(&secret.base_url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_default();
    format!("{}:{host}", crate::provider::effective_kind(secret))
}

/// One outbound request.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AttemptLog {
    pub number: u32,
    pub transport: String,
    pub elapsed_ms: u64,
    /// Time spent waiting for admission before this request (not an attempt).
    pub admission_wait_ms: u64,
    pub failure: Option<ProviderFailure>,
}

impl AttemptLog {
    pub fn line(&self) -> String {
        match &self.failure {
            None => format!(
                "Attempt {} · {} · {} ms · ok",
                self.number, self.transport, self.elapsed_ms
            ),
            Some(f) => format!(
                "Attempt {} · {} · {} ms · {} · {}",
                self.number,
                self.transport,
                self.elapsed_ms,
                f.category.as_str(),
                f.summary()
            ),
        }
    }
}

/// Progress callbacks (job phases, per-attempt records).
pub enum AttemptEvent<'a> {
    Started { number: u32, transport: Transport },
    Finished(&'a AttemptLog),
}

/// Typed result of one summary execution.
#[derive(Clone, Debug, Default)]
pub struct SummaryExecution {
    pub content: Option<String>,
    pub attempts: Vec<AttemptLog>,
    pub failure: Option<ProviderFailure>,
    pub admission_wait_ms: u64,
    /// Why the second request was sent (`transport_fallback`, `retry`), if any.
    pub fallback_reason: Option<String>,
}

impl SummaryExecution {
    /// Outbound requests sent.
    pub fn requests(&self) -> u32 {
        self.attempts.len() as u32
    }
}

fn cancelled(opts: &ExecOptions) -> bool {
    opts.cancel
        .as_ref()
        .is_some_and(|c| c.load(Ordering::Relaxed))
}

fn is_stream_fault(category: Category) -> bool {
    matches!(
        category,
        Category::StreamInterrupted
            | Category::PrematureEof
            | Category::SseError
            | Category::MalformedPayload
            | Category::Timeout
    )
}

/// Run one summary under the shared budget and return the full report.
pub async fn complete_summary_report(
    secret: &ProviderSecret,
    mode: SummarizationMode,
    messages: &[ChatMessage],
    known_ids: &[String],
    opts: &ExecOptions,
    mut observe: impl FnMut(AttemptEvent<'_>),
) -> SummaryExecution {
    let mut out = SummaryExecution::default();
    let kind = crate::provider::effective_kind(secret);
    let missing_model = secret.model.trim().is_empty();
    let missing_endpoint =
        !provider_attempt::is_subscription(secret) && secret.base_url.trim().is_empty();
    if missing_model || missing_endpoint {
        let what = if missing_model { "model" } else { "endpoint" };
        out.failure = Some(
            ProviderFailure::new(
                Stage::Configuration,
                Category::Configuration,
                format!("summarization {what} is not configured"),
            )
            .with_context(&kind, &secret.model, &secret.base_url, ""),
        );
        return out;
    }
    let cap = MAX_REQUESTS;
    let mut transport = opts.first_transport;
    let mut number = 0u32;
    while number < cap {
        if cancelled(opts) {
            out.failure = Some(ProviderFailure::new(
                Stage::Admission,
                Category::Cancelled,
                "cancelled before the request was sent",
            ));
            break;
        }
        // Admission: wait for a slot. Contention is not an attempt.
        let waited = Instant::now();
        let guard = loop {
            if let Some(g) = AdmissionGuard::try_enter(&opts.admission_account) {
                break Some(g);
            }
            if waited.elapsed() >= opts.admission_timeout || cancelled(opts) {
                break None;
            }
            tokio::time::sleep(opts.admission_poll).await;
        };
        let wait_ms = waited.elapsed().as_millis() as u64;
        out.admission_wait_ms += wait_ms;
        let Some(guard) = guard else {
            let category = if cancelled(opts) {
                Category::Cancelled
            } else {
                Category::Timeout
            };
            let mut f = ProviderFailure::new(
                Stage::Admission,
                category,
                format!(
                    "no provider slot after {}s; no request was sent",
                    wait_ms / 1000
                ),
            )
            .with_context(&kind, &secret.model, &secret.base_url, transport.as_str());
            f.retryable = category == Category::Timeout;
            out.failure = Some(f);
            break;
        };
        number += 1;
        observe(AttemptEvent::Started { number, transport });
        let report = provider_attempt::attempt(secret, messages, transport, opts.deadlines).await;
        drop(guard);
        let mut log = AttemptLog {
            number,
            transport: transport.as_str().into(),
            elapsed_ms: report.elapsed_ms,
            admission_wait_ms: wait_ms,
            failure: None,
        };
        let failure = match report.outcome {
            Ok(completion) => {
                let text = completion.content.trim().to_string();
                match validate_result(mode, &text, known_ids) {
                    Ok(()) => {
                        observe(AttemptEvent::Finished(&log));
                        out.attempts.push(log);
                        out.content = Some(text);
                        out.failure = None;
                        return out;
                    }
                    Err(why) => {
                        let mut f = ProviderFailure::new(
                            Stage::Validation,
                            Category::InvalidResult,
                            format!("summary rejected: {why}"),
                        )
                        .with_context(
                            &kind,
                            &secret.model,
                            &secret.base_url,
                            transport.as_str(),
                        );
                        f.elapsed_ms = report.elapsed_ms;
                        f
                    }
                }
            }
            Err(f) => f,
        };
        log.failure = Some(failure.clone());
        observe(AttemptEvent::Finished(&log));
        out.attempts.push(log);
        out.failure = Some(failure.clone());
        if number >= cap || cancelled(opts) {
            break;
        }
        // Transport switches consume an attempt; they are not a free extra request.
        if failure.category == Category::Unsupported
            || (transport == Transport::Stream && is_stream_fault(failure.category))
        {
            transport = transport.other();
            out.fallback_reason = Some(format!(
                "transport_fallback: retrying as {}",
                transport.as_str()
            ));
        } else {
            out.fallback_reason = Some(format!("retry: {}", failure.category.as_str()));
        }
        let delay = failure
            .retry_after_ms
            .map(Duration::from_millis)
            .or_else(|| crate::provider_chain::wait_before_retry(0, number))
            .unwrap_or(Duration::from_secs(10))
            .min(opts.max_backoff);
        if failure.category == Category::RateLimited {
            tasks::note_shared_rate_limit(&opts.admission_account, delay);
        }
        if !cfg!(test) {
            tokio::time::sleep(delay).await;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider_attempt::mock::*;

    fn msgs() -> Vec<ChatMessage> {
        vec![ChatMessage {
            role: "user".into(),
            content: "Explain the graph".into(),
            tool_call_id: None,
            tool_calls: Vec::new(),
        }]
    }

    fn opts(secret: &ProviderSecret, account: &str) -> ExecOptions {
        let mut o = ExecOptions::for_secret(secret);
        o.admission_account = account.into();
        o.max_backoff = Duration::from_millis(20);
        o.admission_poll = Duration::from_millis(10);
        o
    }

    const GOOD: &str = "## Northwind halted crossings\n\nTwo articles support the halt directly.";

    async fn exec(
        script: Vec<Reply>,
        account: &str,
        first: Transport,
    ) -> (SummaryExecution, usize) {
        let server = serve(script).await;
        let secret = secret(&server.base_url);
        let mut o = opts(&secret, account);
        o.first_transport = first;
        let out = complete_summary_report(
            &secret,
            SummarizationMode::GraphExplanation,
            &msgs(),
            &[],
            &o,
            |_| {},
        )
        .await;
        (out, server.hits())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn primary_budget_is_four_requests_including_transport_fallback() {
        // Persistent 503: primary budget is 4 requests.
        let (out, hits) = exec(
            vec![Reply::Json(503, "{}".into())],
            "x-503",
            Transport::NonStream,
        )
        .await;
        assert_eq!((out.requests(), hits), (4, 4));
        assert!(out.content.is_none());
        assert_eq!(out.failure.unwrap().category, Category::Server);

        // Stream drop then non-stream success: exactly two, final-only fallback.
        let (out, hits) = exec(
            vec![Reply::Sse(vec![delta("Partial")]), ok_json(GOOD, "stop")],
            "x-fallback",
            Transport::Stream,
        )
        .await;
        assert_eq!((out.requests(), hits), (2, 2));
        assert_eq!(out.content.as_deref(), Some(GOOD));
        assert_eq!(out.attempts[1].transport, "non_stream");
        assert!(out
            .fallback_reason
            .unwrap()
            .starts_with("transport_fallback"));

        // Unsupported non-stream then broken stream: primary budget of 4, partial never returned.
        let (out, hits) = exec(
            vec![
                Reply::Json(
                    400,
                    r#"{"error":{"message":"only stream mode is supported"}}"#.into(),
                ),
                Reply::Sse(vec![delta("Half an answer")]),
            ],
            "x-unsupported",
            Transport::NonStream,
        )
        .await;
        assert_eq!((out.requests(), hits), (4, 4));
        assert!(out.content.is_none());
        let f = out.failure.unwrap();
        assert!(
            matches!(
                f.category,
                Category::PrematureEof | Category::MalformedPayload | Category::StreamInterrupted
            ),
            "{:?}",
            f.category
        );
        assert!(
            out.attempts
                .iter()
                .any(|a| a.failure.as_ref().is_some_and(|f| f.stream.content_began)),
            "a broken stream began content and must not become the summary"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn configuration_errors_send_one_request_and_carry_guidance() {
        let (out, hits) = exec(
            vec![Reply::Json(
                404,
                r#"{"error":{"message":"model mock-model not found","code":"model_not_found"}}"#
                    .into(),
            )],
            "x-model",
            Transport::NonStream,
        )
        .await;
        assert_eq!(hits, 4);
        let f = out.failure.unwrap();
        assert_eq!(f.category, Category::InvalidModel);
        assert!(f.guidance().unwrap().contains("Models"));

        let (out, hits) = exec(
            vec![ok_json("Cut", "length")],
            "x-len",
            Transport::NonStream,
        )
        .await;
        assert_eq!(
            (hits, out.failure.unwrap().category),
            (4, Category::TokenLimit)
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn admission_contention_consumes_no_attempt() {
        let server = serve(vec![ok_json(GOOD, "stop")]).await;
        let secret = secret(&server.base_url);
        let account = "x-admission";
        // Saturate the account (default concurrency 2).
        let a = AdmissionGuard::try_enter(account).unwrap();
        let b = AdmissionGuard::try_enter(account).unwrap();
        let release = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(150)).await;
            drop(a);
            drop(b);
        });
        let out = complete_summary_report(
            &secret,
            SummarizationMode::GraphExplanation,
            &msgs(),
            &[],
            &opts(&secret, account),
            |_| {},
        )
        .await;
        release.await.unwrap();
        assert_eq!(out.content.as_deref(), Some(GOOD));
        assert_eq!((out.requests(), server.hits()), (1, 1));
        assert_eq!(out.attempts[0].number, 1);
        assert!(out.admission_wait_ms >= 100, "{}", out.admission_wait_ms);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn empty_then_valid_retries_once() {
        let (out, hits) = exec(
            vec![ok_json("", "stop"), ok_json(GOOD, "stop")],
            "x-empty",
            Transport::NonStream,
        )
        .await;
        assert_eq!((hits, out.content.as_deref()), (2, Some(GOOD)));
        assert_eq!(
            out.attempts[0].failure.as_ref().unwrap().category,
            Category::Empty
        );
    }
}
