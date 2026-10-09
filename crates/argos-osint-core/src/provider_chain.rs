//! Shared model-request retry and fallback executor.
//!
//! Primary: 4 attempts (initial + 3 retries) with 10s, 20s, 30s waits *before*
//! each retry. Each fallback: 3 attempts (initial + 2 retries) with 10s, 20s.
//! Exhaust primary before fallback 1, then each fallback top to bottom. Delay
//! progression resets per route. Stop at the first contract-valid success.

use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::tasks::{self, AdmissionGuard};

/// Primary route: initial call plus three retries.
pub const PRIMARY_ATTEMPTS: u32 = 4;
/// Each fallback route: initial call plus two retries.
pub const FALLBACK_ATTEMPTS: u32 = 3;
/// Waits before primary retries (after attempts 1, 2, and 3).
pub const PRIMARY_WAITS: [Duration; 3] = [
    Duration::from_secs(10),
    Duration::from_secs(20),
    Duration::from_secs(30),
];
/// Waits before fallback retries (after attempts 1 and 2 on that route).
pub const FALLBACK_WAITS: [Duration; 2] = [Duration::from_secs(10), Duration::from_secs(20)];

/// One configured inference route (primary or a fallback).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Route {
    pub provider: String,
    pub account: String,
    pub model: String,
    /// When set, this route is not dispatched (missing key, invalid, incompatible).
    #[serde(default)]
    pub skip_reason: Option<String>,
    /// Unique admission slot key for concurrency and rate-limit cooldown.
    #[serde(default)]
    pub admission_key: Option<String>,
}

/// One recorded attempt or skip.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttemptRecord {
    pub at: String,
    pub route_index: u32,
    pub attempt: u32,
    pub model: String,
    pub dispatched: bool,
    pub outcome: String,
    pub reason: String,
}

/// Result of running the chain.
#[derive(Clone, Debug)]
pub struct ChainReport<T> {
    pub value: Option<T>,
    pub blocked: bool,
    pub cancelled: bool,
    pub requests: u32,
    pub waits: Vec<Duration>,
    pub history: Vec<AttemptRecord>,
    pub effective_route: Option<u32>,
    pub effective_model: String,
}

impl<T> ChainReport<T> {
    pub fn failure_message(&self) -> String {
        self.history
            .iter()
            .rev()
            .find(|r| r.outcome == "fail" || r.outcome == "skip")
            .map(|r| r.reason.clone())
            .unwrap_or_else(|| "every route failed".into())
    }
}

/// How a failed attempt should be handled by the chain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetryDisposition {
    /// Transient failure: retry on the same route up to attempt limit.
    RetryRoute,
    /// Non-retryable on this route (bad credentials, invalid model, unsupported, token limit):
    /// skip to the next configured route without spending remaining attempts on this route.
    NextRoute,
    /// Terminal failure (cancelled, invalid local input, refusal, persistence failure):
    /// stop the chain immediately.
    Stop,
}

/// Dispatch failure.
#[derive(Clone, Debug)]
pub struct DispatchError {
    pub message: String,
    pub retry_after: Option<Duration>,
    pub cancelled: bool,
    pub disposition: RetryDisposition,
    pub failure: Option<crate::provider_diag::ProviderFailure>,
}

impl DispatchError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retry_after: None,
            cancelled: false,
            disposition: RetryDisposition::RetryRoute,
            failure: None,
        }
    }

    pub fn with_retry_after(mut self, duration: Duration) -> Self {
        self.retry_after = Some(duration);
        self
    }

    pub fn cancelled() -> Self {
        Self {
            message: "cancelled".into(),
            retry_after: None,
            cancelled: true,
            disposition: RetryDisposition::Stop,
            failure: None,
        }
    }

    pub fn from_failure(f: crate::provider_diag::ProviderFailure) -> Self {
        use crate::provider_diag::Category;
        let disposition = match f.category {
            Category::Network
            | Category::Timeout
            | Category::RateLimited
            | Category::Server
            | Category::Empty
            | Category::MalformedPayload
            | Category::StreamInterrupted
            | Category::PrematureEof
            | Category::SseError => RetryDisposition::RetryRoute,

            Category::Auth
            | Category::Permission
            | Category::InvalidModel
            | Category::Configuration
            | Category::Unsupported
            | Category::TokenLimit => RetryDisposition::NextRoute,

            Category::MalformedRequest
            | Category::Refused
            | Category::InvalidResult
            | Category::Persistence
            | Category::Cancelled => RetryDisposition::Stop,
        };
        let cancelled = f.category == Category::Cancelled;
        let retry_after = f.retry_after_ms.map(Duration::from_millis);
        let message = f.summary();
        Self {
            message,
            retry_after,
            cancelled,
            disposition,
            failure: Some(f),
        }
    }
}

/// Clock / cancel / admission options.
#[derive(Clone, Default)]
pub struct ExecuteOptions {
    /// Skip actual timers (tests). Waits are still recorded.
    pub instant: bool,
    pub cancel: Option<Arc<AtomicBool>>,
    pub recorded_waits: Option<Arc<Mutex<Vec<Duration>>>>,
    /// Admission key; empty skips the shared slot.
    pub admission_account: String,
}

impl ExecuteOptions {
    pub fn for_tests() -> Self {
        Self {
            instant: true,
            ..Self::default()
        }
    }
}

pub fn route_attempt_limit(route_index: u32) -> u32 {
    if route_index == 0 {
        PRIMARY_ATTEMPTS
    } else {
        FALLBACK_ATTEMPTS
    }
}

/// Wait applied *after* a failed attempt `attempt` (1-based) on `route_index`
/// before the next attempt on the same route. `None` when the route is exhausted.
pub fn wait_before_retry(route_index: u32, attempt: u32) -> Option<Duration> {
    let idx = attempt.saturating_sub(1) as usize;
    if route_index == 0 {
        PRIMARY_WAITS.get(idx).copied()
    } else {
        FALLBACK_WAITS.get(idx).copied()
    }
}

pub fn max_requests(fallback_count: u32) -> u32 {
    PRIMARY_ATTEMPTS + FALLBACK_ATTEMPTS * fallback_count
}

pub fn all_failure_base_wait(fallback_count: u32) -> Duration {
    Duration::from_secs(60 + 30 * u64::from(fallback_count))
}

fn cancelled(opts: &ExecuteOptions) -> bool {
    opts.cancel
        .as_ref()
        .is_some_and(|c| c.load(Ordering::Relaxed))
}

async fn sleep_recorded(opts: &ExecuteOptions, wait: Duration) -> bool {
    if let Some(slot) = &opts.recorded_waits {
        if let Ok(mut v) = slot.lock() {
            v.push(wait);
        }
    }
    if opts.instant || wait.is_zero() {
        return cancelled(opts);
    }
    cancellable_sleep(wait, opts.cancel.as_ref()).await
}

pub async fn cancellable_sleep(wait: Duration, cancel: Option<&Arc<AtomicBool>>) -> bool {
    if let Some(c) = cancel {
        if c.load(Ordering::Relaxed) {
            return true;
        }
        let step = Duration::from_millis(50);
        let mut elapsed = Duration::ZERO;
        while elapsed < wait {
            let chunk = step.min(wait - elapsed);
            tokio::time::sleep(chunk).await;
            if c.load(Ordering::Relaxed) {
                return true;
            }
            elapsed += chunk;
        }
        false
    } else {
        tokio::time::sleep(wait).await;
        false
    }
}

async fn wait_cancelled(cancel: &Arc<AtomicBool>) {
    while !cancel.load(Ordering::Relaxed) {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn stamp() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// Run `dispatch` under the primary/fallback attempt policy.
///
/// `dispatch` performs exactly one provider call for the given route. Invalid
/// output is a dispatched failure (the caller validates inside `dispatch`).
pub async fn execute<T, F, Fut>(
    routes: &[Route],
    mut dispatch: F,
    opts: ExecuteOptions,
) -> ChainReport<T>
where
    F: FnMut(usize, &Route) -> Fut,
    Fut: Future<Output = Result<T, DispatchError>>,
{
    let mut report = ChainReport {
        value: None,
        blocked: false,
        cancelled: false,
        requests: 0,
        waits: Vec::new(),
        history: Vec::new(),
        effective_route: None,
        effective_model: String::new(),
    };
    if routes.is_empty() {
        report.blocked = true;
        report.history.push(AttemptRecord {
            at: stamp(),
            route_index: 0,
            attempt: 0,
            model: String::new(),
            dispatched: false,
            outcome: "skip".into(),
            reason: "no routes configured".into(),
        });
        return report;
    }

    let mut any_dispatchable = false;
    for (route_index, route) in routes.iter().enumerate() {
        if cancelled(&opts) {
            report.cancelled = true;
            break;
        }
        if let Some(reason) = &route.skip_reason {
            report.history.push(AttemptRecord {
                at: stamp(),
                route_index: route_index as u32,
                attempt: 0,
                model: route.model.clone(),
                dispatched: false,
                outcome: "skip".into(),
                reason: reason.clone(),
            });
            continue;
        }
        any_dispatchable = true;
        let limit = route_attempt_limit(route_index as u32);
        let mut attempt = 0u32;
        let admission_key = route
            .admission_key
            .as_deref()
            .filter(|k| !k.is_empty())
            .or_else(|| (!route.account.is_empty()).then_some(route.account.as_str()))
            .unwrap_or(opts.admission_account.as_str());

        while attempt < limit {
            if cancelled(&opts) {
                report.cancelled = true;
                break;
            }
            let guard = if admission_key.is_empty() {
                None
            } else {
                loop {
                    if cancelled(&opts) {
                        report.cancelled = true;
                        break None;
                    }
                    if let Some(g) = AdmissionGuard::try_enter(admission_key) {
                        break Some(g);
                    }
                    if sleep_recorded(&opts, Duration::from_millis(50)).await {
                        report.cancelled = true;
                        break None;
                    }
                }
            };
            if report.cancelled {
                break;
            }
            attempt += 1;
            report.requests += 1;
            let dispatch_fut = dispatch(route_index, route);
            let result = match &opts.cancel {
                None => dispatch_fut.await,
                Some(cancel_flag) => {
                    tokio::select! {
                        res = dispatch_fut => res,
                        _ = wait_cancelled(cancel_flag) => Err(DispatchError::cancelled()),
                    }
                }
            };
            drop(guard);
            match result {
                Ok(value) => {
                    report.history.push(AttemptRecord {
                        at: stamp(),
                        route_index: route_index as u32,
                        attempt,
                        model: route.model.clone(),
                        dispatched: true,
                        outcome: "ok".into(),
                        reason: String::new(),
                    });
                    report.value = Some(value);
                    report.effective_route = Some(route_index as u32);
                    report.effective_model = route.model.clone();
                    return report;
                }
                Err(err) if err.cancelled || err.disposition == RetryDisposition::Stop => {
                    let is_cancel = err.cancelled;
                    report.history.push(AttemptRecord {
                        at: stamp(),
                        route_index: route_index as u32,
                        attempt,
                        model: route.model.clone(),
                        dispatched: true,
                        outcome: if is_cancel {
                            "cancel".into()
                        } else {
                            "fail".into()
                        },
                        reason: err.message,
                    });
                    if is_cancel {
                        report.cancelled = true;
                    }
                    return report;
                }
                Err(err) if err.disposition == RetryDisposition::NextRoute => {
                    report.history.push(AttemptRecord {
                        at: stamp(),
                        route_index: route_index as u32,
                        attempt,
                        model: route.model.clone(),
                        dispatched: true,
                        outcome: "fail".into(),
                        reason: err.message,
                    });
                    break;
                }
                Err(err) => {
                    report.history.push(AttemptRecord {
                        at: stamp(),
                        route_index: route_index as u32,
                        attempt,
                        model: route.model.clone(),
                        dispatched: true,
                        outcome: "fail".into(),
                        reason: err.message,
                    });
                    let Some(base) = wait_before_retry(route_index as u32, attempt) else {
                        break;
                    };
                    let wait = err.retry_after.filter(|d| *d > base).unwrap_or(base);
                    if !opts.instant {
                        if let Some(extra) = err.retry_after {
                            tasks::note_shared_rate_limit(admission_key, extra);
                        }
                    }
                    report.waits.push(wait);
                    if sleep_recorded(&opts, wait).await {
                        report.cancelled = true;
                        break;
                    }
                }
            }
        }
        if report.cancelled {
            break;
        }
    }
    if !any_dispatchable {
        report.blocked = true;
    }
    if let Some(slot) = &opts.recorded_waits {
        if let Ok(v) = slot.lock() {
            if report.waits.is_empty() {
                report.waits = v.clone();
            }
        }
    }
    report
}

/// Build routes from a primary secret plus fallback secrets. Empty API keys
/// become preflight skips, not dispatched attempts.
pub fn routes_from_secrets(
    primary: &crate::secrets::ProviderSecret,
    fallbacks: &[crate::secrets::ProviderSecret],
) -> Vec<Route> {
    let mut routes = Vec::with_capacity(1 + fallbacks.len());
    routes.push(secret_route(primary));
    for secret in fallbacks {
        routes.push(secret_route(secret));
    }
    routes
}

fn secret_route(secret: &crate::secrets::ProviderSecret) -> Route {
    let host = url::Url::parse(&secret.base_url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_else(|| "default".into());
    let kind = crate::provider::effective_kind(secret);
    let account = if secret.kind.is_empty() {
        "default".to_string()
    } else {
        secret.kind.clone()
    };
    let admission_key = format!("{}:{}:{}", kind, host, account);
    let missing_key = secret
        .api_key
        .as_deref()
        .map(str::trim)
        .unwrap_or("")
        .is_empty()
        && crate::provider::effective_kind(secret) != "local";
    let missing_model = secret.model.trim().is_empty();
    let skip = if missing_model {
        Some("model is not configured".into())
    } else if missing_key {
        Some(format!(
            "API key missing; configure {} in Providers",
            crate::provider::effective_kind(secret)
        ))
    } else {
        None
    };
    Route {
        provider: crate::provider::effective_kind(secret),
        account,
        model: secret.model.clone(),
        skip_reason: skip,
        admission_key: Some(admission_key),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;

    fn routes(n: usize) -> Vec<Route> {
        (0..n)
            .map(|i| Route {
                provider: "openrouter".into(),
                account: format!("a{i}"),
                model: format!("m{i}"),
                skip_reason: None,
                admission_key: None,
            })
            .collect()
    }

    #[tokio::test]
    async fn primary_emits_four_attempts_with_10_20_30_waits() {
        let hits = AtomicU32::new(0);
        let recorded = Arc::new(Mutex::new(Vec::new()));
        let opts = ExecuteOptions {
            instant: true,
            recorded_waits: Some(recorded.clone()),
            ..ExecuteOptions::default()
        };
        let out = execute(
            &routes(1),
            |_i, _r| {
                hits.fetch_add(1, Ordering::SeqCst);
                async { Err(DispatchError::new("503")) as Result<(), _> }
            },
            opts,
        )
        .await;
        assert_eq!(out.requests, 4);
        assert_eq!(hits.load(Ordering::SeqCst), 4);
        assert_eq!(
            out.waits,
            vec![
                Duration::from_secs(10),
                Duration::from_secs(20),
                Duration::from_secs(30)
            ]
        );
        assert!(out.value.is_none());
        assert!(!out.blocked);
    }

    #[tokio::test]
    async fn fallback_emits_three_attempts_with_10_20_waits() {
        let hits = AtomicU32::new(0);
        let out = execute(
            &routes(2),
            |_i, _r| {
                hits.fetch_add(1, Ordering::SeqCst);
                async { Err(DispatchError::new("boom")) as Result<(), _> }
            },
            ExecuteOptions::for_tests(),
        )
        .await;
        assert_eq!(out.requests, 7);
        assert_eq!(
            out.waits,
            vec![
                Duration::from_secs(10),
                Duration::from_secs(20),
                Duration::from_secs(30),
                Duration::from_secs(10),
                Duration::from_secs(20),
            ]
        );
    }

    #[tokio::test]
    async fn two_exhausted_fallbacks_are_ten_requests_and_120s_base() {
        let out = execute(
            &routes(3),
            |_i, _r| async { Err(DispatchError::new("x")) as Result<(), _> },
            ExecuteOptions::for_tests(),
        )
        .await;
        assert_eq!(out.requests, max_requests(2));
        assert_eq!(out.requests, 10);
        let total: u64 = out.waits.iter().map(|d| d.as_secs()).sum();
        assert_eq!(Duration::from_secs(total), all_failure_base_wait(2));
        assert_eq!(total, 120);
    }

    #[tokio::test]
    async fn success_short_circuits() {
        let hits = AtomicU32::new(0);
        let out = execute(
            &routes(2),
            |_i, _r| {
                let n = hits.fetch_add(1, Ordering::SeqCst);
                async move {
                    if n == 1 {
                        Ok("ok")
                    } else {
                        Err(DispatchError::new("fail"))
                    }
                }
            },
            ExecuteOptions::for_tests(),
        )
        .await;
        assert_eq!(out.value, Some("ok"));
        assert_eq!(out.requests, 2);
        assert_eq!(out.effective_route, Some(0));
        assert_eq!(out.waits.len(), 1);
    }

    #[tokio::test]
    async fn http_stream_and_parse_failures_all_retry() {
        let kinds = ["400", "401", "403", "404", "429", "500", "timeout", "parse"];
        for kind in kinds {
            let hits = AtomicU32::new(0);
            let out = execute(
                &routes(1),
                |_i, _r| {
                    hits.fetch_add(1, Ordering::SeqCst);
                    let msg = kind.to_string();
                    async move { Err(DispatchError::new(msg)) as Result<(), _> }
                },
                ExecuteOptions::for_tests(),
            )
            .await;
            assert_eq!(out.requests, 4, "{kind} should consume the primary budget");
        }
    }

    #[tokio::test]
    async fn preflight_skip_does_not_dispatch_and_blocks_when_none_can() {
        let routes = vec![Route {
            provider: "google".into(),
            account: String::new(),
            model: "gemini".into(),
            skip_reason: Some("API key missing; configure google in Providers".into()),
            admission_key: None,
        }];
        let hits = AtomicU32::new(0);
        let out = execute(
            &routes,
            |_i, _r| {
                hits.fetch_add(1, Ordering::SeqCst);
                async { Ok(()) }
            },
            ExecuteOptions::for_tests(),
        )
        .await;
        assert_eq!(hits.load(Ordering::SeqCst), 0);
        assert_eq!(out.requests, 0);
        assert!(out.blocked);
        assert_eq!(out.history[0].outcome, "skip");
    }

    #[tokio::test]
    async fn cancellation_stops_the_chain() {
        let flag = Arc::new(AtomicBool::new(false));
        let hits = AtomicU32::new(0);
        let cancel = flag.clone();
        let out = execute(
            &routes(2),
            |_i, _r| {
                let n = hits.fetch_add(1, Ordering::SeqCst);
                if n >= 1 {
                    flag.store(true, Ordering::SeqCst);
                }
                async { Err(DispatchError::new("x")) as Result<(), _> }
            },
            ExecuteOptions {
                instant: true,
                cancel: Some(cancel),
                ..ExecuteOptions::default()
            },
        )
        .await;
        assert!(out.cancelled);
        assert!(out.requests < 10);
    }

    #[tokio::test]
    async fn retry_after_extends_base_wait() {
        let out = execute(
            &routes(1),
            |_i, _r| async {
                Err(DispatchError::new("429").with_retry_after(Duration::from_secs(45)))
                    as Result<(), _>
            },
            ExecuteOptions::for_tests(),
        )
        .await;
        assert_eq!(out.waits[0], Duration::from_secs(45));
        assert_eq!(out.waits[1], Duration::from_secs(45));
        assert_eq!(out.waits[2], Duration::from_secs(45));
    }
}
