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
    /// The stage-wide request budget ended the operation before it dispatched.
    pub budget_exhausted: bool,
    pub value: Option<T>,
    pub blocked: bool,
    pub cancelled: bool,
    pub requests: u32,
    pub waits: Vec<Duration>,
    pub history: Vec<AttemptRecord>,
    pub effective_route: Option<u32>,
    pub effective_model: String,
    pub terminal_failure: Option<crate::provider_diag::ProviderFailure>,
    pub stopped: bool,
    /// The operation wall-clock cap ended the chain (queue, pacing, backoff
    /// and dispatch time all count against it).
    pub wall_clock_exceeded: bool,
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
    /// Typed recovery proposal (spec §7): the dispatch may be recompiled and
    /// sent once more on the same route *without backoff* (unsupported-option
    /// recovery or one output repair). It overrides `disposition` but always
    /// consumes the operation wire budget.
    pub recovery: Option<String>,
}

impl DispatchError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retry_after: None,
            cancelled: false,
            disposition: RetryDisposition::RetryRoute,
            failure: None,
            recovery: None,
        }
    }

    pub fn with_retry_after(mut self, duration: Duration) -> Self {
        self.retry_after = Some(duration);
        self
    }

    /// Mark this failure as recoverable on the same route (see `recovery`).
    pub fn with_recovery(mut self, reason: impl Into<String>) -> Self {
        self.recovery = Some(reason.into());
        self
    }

    pub fn cancelled() -> Self {
        Self {
            message: "cancelled".into(),
            retry_after: None,
            cancelled: true,
            disposition: RetryDisposition::Stop,
            failure: None,
            recovery: None,
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
            recovery: None,
        }
    }
}

/// Operation-scoped wire budget (spec §7). Every route keeps its reserved
/// initial dispatch; extra dispatches (one primary transient retry, output
/// repair, unsupported-option recovery) draw from one shared allowance and
/// never exceed `per_route_max` on a route. A 4-route chain therefore sends
/// at most `4 + shared_extra` = 6 requests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WireBudget {
    pub per_route_max: u32,
    pub primary_transient_retries: u32,
    pub shared_extra: u32,
    /// A timed-out route (no headers, no first output, stalled stream) moves
    /// straight to the next dispatchable route instead of resending to the
    /// route that just stalled. A stall with no later dispatchable route keeps
    /// the normal retry rules.
    pub skip_stalled_route: bool,
    /// Operation wall-clock cap across every route, queue wait, provider
    /// pacing and backoff. `None` = uncapped.
    pub wall_clock: Option<Duration>,
}

impl WireBudget {
    /// Interactive brief operations.
    pub const INTERACTIVE: Self = Self {
        per_route_max: 2,
        primary_transient_retries: 1,
        shared_extra: 2,
        skip_stalled_route: true,
        wall_clock: Some(Duration::from_secs(600)),
    };

    /// Interactive operations on thinking models: per-attempt waits are
    /// thinking-sized, so the cap leaves room for a fallback after a stall.
    pub const INTERACTIVE_THINKING: Self = Self {
        wall_clock: Some(Duration::from_secs(900)),
        ..Self::INTERACTIVE
    };

    pub fn max_dispatches(&self, routes: u32) -> u32 {
        routes + self.shared_extra
    }
}

/// Per-dispatch context handed to contextual dispatchers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DispatchContext {
    pub route_index: usize,
    /// 1-based attempt on this route.
    pub attempt: u32,
    /// 1-based wire dispatch across the operation.
    pub wire_dispatch: u32,
    /// Set when this dispatch carries a recovery proposal from the previous one.
    pub recovery: Option<String>,
    /// Admission queue time for this dispatch (slot polling), never backoff.
    pub queue_ms: u64,
    /// Retry backoff actually slept before this dispatch.
    pub backoff_ms: u64,
    /// Wall-clock time left under the operation cap when this dispatch
    /// starts (`None` = uncapped). The dispatcher bounds the attempt by it.
    pub remaining: Option<Duration>,
    /// When this dispatch started waiting for admission.
    pub queued_at: String,
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
    /// Operation-specific bound. Ordinary callers retain the 4/3 policy.
    pub attempt_limit: Option<u32>,
    /// User-waiting work (e.g. an open Intel brief): takes freed slots ahead
    /// of background callers on the same account, within the same limit.
    pub interactive: bool,
    /// Operation wire budget; replaces `attempt_limit` when set.
    pub wire_budget: Option<WireBudget>,
    /// Stage-wide request budget and route health shared by every operation
    /// of one stage (`None` = operation-local behaviour).
    pub ledger: Option<Arc<StageLedger>>,
}

impl ExecuteOptions {
    pub fn for_tests() -> Self {
        Self {
            instant: true,
            ..Self::default()
        }
    }
}

/// Default stage-wide dispatch ceiling before a stage declares how much work
/// it planned (see [`StageLedger::plan`]).
pub const STAGE_REQUEST_CAP: u32 = 24;
/// How long a route that just failed transiently is passed over by later
/// operations of the same stage (first, second, third and later strike).
pub const ROUTE_COOLDOWNS: [Duration; 3] = [
    Duration::from_secs(120),
    Duration::from_secs(300),
    Duration::from_secs(600),
];

/// Extra overload retries granted to the last route that can still answer.
pub const LAST_ROUTE_RETRIES: u32 = 2;

/// Hold time for a route that hard-rejected (credentials, access, model
/// deployment missing): retrying inside a session cannot fix those.
pub const HARD_REJECT_COOLDOWN: Duration = Duration::from_secs(1800);

/// `nvidia/nemotron-3-ultra-550b-a55b:free` and the same id on another
/// gateway name one model.
fn model_family(model: &str) -> String {
    let id = model.split(':').next().unwrap_or(model);
    id.rsplit('/').next().unwrap_or(id).to_ascii_lowercase()
}

/// Request budget and route-health memory of ONE stage execution, shared by
/// all of its operations. A [`WireBudget`] bounds a single operation; a stage
/// of many operations (per-claim classification, generation, fidelity, dedup)
/// would otherwise restart at route 0 each time and re-pay the same dead
/// route, multiplying the wire count and the wall clock.
#[derive(Debug)]
pub struct StageLedger {
    cap: std::sync::atomic::AtomicU32,
    dispatched: std::sync::atomic::AtomicU32,
    health: Mutex<std::collections::HashMap<String, RouteStrikes>>,
    /// Model families whose deployment this account cannot reach (hard
    /// reject), with the recorded reason. Routes through another gateway to
    /// the same model are passed over too.
    dead: Mutex<std::collections::HashMap<String, String>>,
}

#[derive(Debug, Clone, Copy)]
struct RouteStrikes {
    until: std::time::Instant,
    strikes: usize,
}

impl Default for StageLedger {
    fn default() -> Self {
        Self::new(STAGE_REQUEST_CAP)
    }
}

impl StageLedger {
    pub fn new(cap: u32) -> Self {
        Self {
            cap: cap.into(),
            dispatched: 0.into(),
            health: Mutex::default(),
            dead: Mutex::default(),
        }
    }

    /// Declare the operations a stage plans to run. The ceiling becomes twice
    /// the planned wire count plus slack for repairs and one dead route, never
    /// less than the default.
    pub fn plan(&self, operations: u32) {
        let cap = operations.saturating_mul(2).saturating_add(8);
        self.cap.fetch_max(
            cap.max(STAGE_REQUEST_CAP),
            std::sync::atomic::Ordering::AcqRel,
        );
    }

    pub fn cap(&self) -> u32 {
        self.cap.load(std::sync::atomic::Ordering::Acquire)
    }

    pub fn dispatched(&self) -> u32 {
        self.dispatched.load(std::sync::atomic::Ordering::Acquire)
    }

    /// Claim one wire dispatch; `false` once the stage budget is spent.
    fn reserve(&self) -> bool {
        use std::sync::atomic::Ordering::{AcqRel, Acquire};
        let cap = self.cap();
        self.dispatched
            .fetch_update(AcqRel, Acquire, |n| (n < cap).then_some(n + 1))
            .is_ok()
    }

    fn key(route: &Route) -> String {
        format!(
            "{}|{}",
            route
                .admission_key
                .as_deref()
                .filter(|k| !k.is_empty())
                .unwrap_or(route.account.as_str()),
            route.model
        )
    }

    fn cooling(&self, route: &Route) -> Option<Duration> {
        let health = self.health.lock().ok()?;
        let entry = health.get(&Self::key(route))?;
        let left = entry
            .until
            .saturating_duration_since(std::time::Instant::now());
        (!left.is_zero()).then_some(left)
    }

    /// Pass over a route after a failure. Hard rejects (credentials, access,
    /// missing model deployment) hold for 30 minutes; transient failures back
    /// off 120/300/600s per strike.
    fn open(&self, route: &Route, failure: Option<&crate::provider_diag::ProviderFailure>) {
        use crate::provider_diag::Category;
        let category = failure.map(|f| f.category);
        let hard = matches!(
            category,
            Some(Category::Auth | Category::Permission | Category::InvalidModel)
        );
        if category == Some(Category::InvalidModel) {
            if let (Ok(mut dead), Some(f)) = (self.dead.lock(), failure) {
                dead.insert(model_family(&route.model), f.summary());
            }
        }
        if let Ok(mut health) = self.health.lock() {
            let entry = health.entry(Self::key(route)).or_insert(RouteStrikes {
                until: std::time::Instant::now(),
                strikes: 0,
            });
            let cooldown = if hard {
                HARD_REJECT_COOLDOWN
            } else {
                ROUTE_COOLDOWNS[entry.strikes.min(ROUTE_COOLDOWNS.len() - 1)]
            };
            entry.strikes += 1;
            entry.until = std::time::Instant::now() + cooldown;
        }
    }

    /// Why this route's model is known to be unreachable for the account.
    fn dead_reason(&self, route: &Route) -> Option<String> {
        self.dead
            .lock()
            .ok()?
            .get(&model_family(&route.model))
            .cloned()
    }

    fn close(&self, route: &Route) {
        if let Ok(mut health) = self.health.lock() {
            health.remove(&Self::key(route));
        }
    }
}

/// Failures that say the route itself is unwell right now, as opposed to this
/// request being bad (malformed output, refusal, oversize).
fn route_unwell(category: crate::provider_diag::Category) -> bool {
    use crate::provider_diag::Category::*;
    matches!(
        category,
        Network
            | Timeout
            | RateLimited
            | Server
            | StreamInterrupted
            | PrematureEof
            | SseError
            | Auth
            | Permission
            | InvalidModel
    )
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

/// Backoff sleep: recorded as a retry wait.
async fn sleep_recorded(opts: &ExecuteOptions, wait: Duration) -> bool {
    if let Some(slot) = &opts.recorded_waits {
        if let Ok(mut v) = slot.lock() {
            v.push(wait);
        }
    }
    sleep_unrecorded(opts, wait).await
}

/// Admission polling: queue time, never recorded as backoff.
async fn sleep_unrecorded(opts: &ExecuteOptions, wait: Duration) -> bool {
    if opts.instant || wait.is_zero() {
        if opts.instant && !wait.is_zero() {
            tokio::task::yield_now().await;
        }
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
    execute_with_context(routes, |ctx, route| dispatch(ctx.route_index, route), opts).await
}

/// Contextual executor: like [`execute`] but each dispatch receives its
/// [`DispatchContext`] (queue/backoff clocks, recovery proposal) and the
/// operation [`WireBudget`] bounds every wire request when set.
pub async fn execute_with_context<T, F, Fut>(
    routes: &[Route],
    mut dispatch: F,
    opts: ExecuteOptions,
) -> ChainReport<T>
where
    F: FnMut(DispatchContext, &Route) -> Fut,
    Fut: Future<Output = Result<T, DispatchError>>,
{
    let mut report = ChainReport {
        budget_exhausted: false,
        value: None,
        blocked: false,
        cancelled: false,
        requests: 0,
        waits: Vec::new(),
        history: Vec::new(),
        effective_route: None,
        effective_model: String::new(),
        terminal_failure: None,
        stopped: false,
        wall_clock_exceeded: false,
    };
    let wall_clock = opts.wire_budget.and_then(|b| b.wall_clock);
    let deadline = wall_clock.map(|d| std::time::Instant::now() + d);
    let remaining = || deadline.map(|at| at.saturating_duration_since(std::time::Instant::now()));
    // Ends the operation at the cap, keeping the last real failure in the text.
    fn cap_failure<T>(
        report: &mut ChainReport<T>,
        cap: Duration,
        last: Option<crate::provider_diag::ProviderFailure>,
    ) {
        use crate::provider_diag::{Category, ProviderFailure, Stage};
        let detail = last
            .map(|f| format!("; last failure: {}", f.summary()))
            .unwrap_or_default();
        report.wall_clock_exceeded = true;
        report.terminal_failure = Some(ProviderFailure::new(
            Stage::Admission,
            Category::Timeout,
            format!(
                "operation wall-clock cap of {}s reached after {} request(s){detail}",
                cap.as_secs(),
                report.requests
            ),
        ));
        report.history.push(AttemptRecord {
            at: stamp(),
            route_index: 0,
            attempt: 0,
            model: String::new(),
            dispatched: false,
            outcome: "skip".into(),
            reason: format!("operation wall-clock cap of {}s reached", cap.as_secs()),
        });
    }
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

    let mut extra_used = 0u32;
    let mut primary_transient_used = 0u32;
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
        if let Some(reason) = opts.ledger.as_deref().and_then(|l| l.dead_reason(route)) {
            report.history.push(AttemptRecord {
                at: stamp(),
                route_index: route_index as u32,
                attempt: 0,
                model: route.model.clone(),
                dispatched: false,
                outcome: "skip".into(),
                reason: format!("same model already rejected in this stage: {reason}"),
            });
            if report.terminal_failure.is_none() {
                report.terminal_failure = Some(crate::provider_diag::ProviderFailure::new(
                    crate::provider_diag::Stage::Admission,
                    crate::provider_diag::Category::InvalidModel,
                    reason,
                ));
            }
            continue;
        }
        if let Some(left) = opts.ledger.as_deref().and_then(|l| l.cooling(route)) {
            let ledger = opts.ledger.as_deref().expect("checked");
            if routes[route_index + 1..].iter().any(|r| {
                r.skip_reason.is_none()
                    && ledger.cooling(r).is_none()
                    && ledger.dead_reason(r).is_none()
            }) {
                report.history.push(AttemptRecord {
                    at: stamp(),
                    route_index: route_index as u32,
                    attempt: 0,
                    model: route.model.clone(),
                    dispatched: false,
                    outcome: "skip".into(),
                    reason: format!(
                        "route failed recently in this stage; passed over for {}s",
                        left.as_secs()
                    ),
                });
                continue;
            }
        }
        let mut route_failure: Option<crate::provider_diag::ProviderFailure> = None;
        let limit = match opts.wire_budget {
            Some(b) => b.per_route_max,
            None => opts
                .attempt_limit
                .unwrap_or_else(|| route_attempt_limit(route_index as u32)),
        };
        let mut attempt = 0u32;
        let mut recovery: Option<String> = None;
        let mut backoff = Duration::ZERO;
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
            if let (Some(cap), Some(0)) = (wall_clock, remaining().map(|r| r.as_millis())) {
                let last = report.terminal_failure.take();
                cap_failure(&mut report, cap, last);
                return report;
            }
            let queued_at = stamp();
            let queue_start = std::time::Instant::now();
            let guard = if admission_key.is_empty() {
                None
            } else {
                let _waiting = opts
                    .interactive
                    .then(|| tasks::InteractiveWait::register(admission_key));
                loop {
                    if cancelled(&opts) {
                        report.cancelled = true;
                        break None;
                    }
                    if let Some(g) =
                        AdmissionGuard::try_enter_class(admission_key, opts.interactive)
                    {
                        break Some(g);
                    }
                    if remaining().is_some_and(|r| r.is_zero()) {
                        break None;
                    }
                    if sleep_unrecorded(&opts, Duration::from_millis(50)).await {
                        report.cancelled = true;
                        break None;
                    }
                }
            };
            if report.cancelled {
                break;
            }
            if guard.is_none() && !admission_key.is_empty() {
                // Admission wait consumed the cap.
                if let Some(cap) = wall_clock {
                    let last = report.terminal_failure.take();
                    cap_failure(&mut report, cap, last);
                    return report;
                }
            }
            if let Some(ledger) = opts.ledger.as_deref() {
                if !ledger.reserve() {
                    drop(guard);
                    use crate::provider_diag::{Category, ProviderFailure, Stage};
                    let last = report.terminal_failure.take();
                    let detail = last
                        .map(|f| format!("; last failure: {}", f.summary()))
                        .unwrap_or_default();
                    report.budget_exhausted = true;
                    report.terminal_failure = Some(ProviderFailure::new(
                        Stage::Admission,
                        Category::Timeout,
                        format!(
                            "stage request budget of {} reached ({} dispatched across this stage's operations){detail}",
                            ledger.cap(),
                            ledger.dispatched()
                        ),
                    ));
                    report.history.push(AttemptRecord {
                        at: stamp(),
                        route_index: route_index as u32,
                        attempt: 0,
                        model: route.model.clone(),
                        dispatched: false,
                        outcome: "skip".into(),
                        reason: format!("stage request budget of {} reached", ledger.cap()),
                    });
                    return report;
                }
            }
            attempt += 1;
            report.requests += 1;
            let ctx = DispatchContext {
                route_index,
                attempt,
                wire_dispatch: report.requests,
                recovery: recovery.take(),
                queue_ms: queue_start.elapsed().as_millis() as u64,
                backoff_ms: backoff.as_millis() as u64,
                remaining: remaining(),
                queued_at,
            };
            backoff = Duration::ZERO;
            let dispatch_fut = dispatch(ctx, route);
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
            let err = match result {
                Ok(value) => {
                    report.terminal_failure = None;
                    report.history.push(AttemptRecord {
                        at: stamp(),
                        route_index: route_index as u32,
                        attempt,
                        model: route.model.clone(),
                        dispatched: true,
                        outcome: "ok".into(),
                        reason: String::new(),
                    });
                    if let Some(ledger) = opts.ledger.as_deref() {
                        ledger.close(route);
                    }
                    report.value = Some(value);
                    report.effective_route = Some(route_index as u32);
                    report.effective_model = route.model.clone();
                    return report;
                }
                Err(err) => err,
            };
            report.terminal_failure = err.failure.clone();
            route_failure = err.failure.clone();
            let stop = err.cancelled
                || (err.recovery.is_none() && err.disposition == RetryDisposition::Stop);
            report.history.push(AttemptRecord {
                at: stamp(),
                route_index: route_index as u32,
                attempt,
                model: route.model.clone(),
                dispatched: true,
                outcome: if err.cancelled {
                    "cancel".into()
                } else {
                    "fail".into()
                },
                reason: err.message.clone(),
            });
            if stop {
                report.stopped = true;
                if err.cancelled {
                    report.cancelled = true;
                }
                return report;
            }
            let is_recovery = err.recovery.is_some();
            if !is_recovery && err.disposition == RetryDisposition::NextRoute {
                break;
            }
            if let Some(cap) = wall_clock {
                if remaining().is_some_and(|r| r.is_zero()) {
                    let last = report.terminal_failure.take();
                    cap_failure(&mut report, cap, last);
                    return report;
                }
            }
            if !is_recovery
                && opts.wire_budget.is_some_and(|b| b.skip_stalled_route)
                && err
                    .failure
                    .as_ref()
                    .is_some_and(|f| f.category == crate::provider_diag::Category::Timeout)
                && routes[route_index + 1..]
                    .iter()
                    .any(|r| r.skip_reason.is_none())
            {
                // Stalled: a thinking model that went silent will not answer a
                // resend sooner. Fall through to the next route.
                break;
            }
            if attempt >= limit {
                break;
            }
            if let Some(b) = opts.wire_budget {
                // The last route that can still answer is the only hope: a
                // brief overload (503/429) there deserves a short backoff and
                // another try rather than ending the operation.
                let last_live = routes[route_index + 1..].iter().all(|r| {
                    r.skip_reason.is_some()
                        || opts
                            .ledger
                            .as_deref()
                            .is_some_and(|l| l.cooling(r).is_some() || l.dead_reason(r).is_some())
                });
                let overloaded = err.failure.as_ref().is_some_and(|f| {
                    matches!(
                        f.category,
                        crate::provider_diag::Category::Server
                            | crate::provider_diag::Category::RateLimited
                    )
                });
                let transient_ok = (route_index == 0
                    && primary_transient_used < b.primary_transient_retries)
                    || (last_live
                        && overloaded
                        && primary_transient_used
                            < b.primary_transient_retries + LAST_ROUTE_RETRIES);
                if extra_used >= b.shared_extra || !(is_recovery || transient_ok) {
                    break;
                }
                extra_used += 1;
                if !is_recovery {
                    primary_transient_used += 1;
                }
            }
            if is_recovery {
                // Recompiled resend: no backoff, the request itself changes.
                recovery = err.recovery;
                continue;
            }
            let Some(base) = wait_before_retry(route_index as u32, attempt) else {
                break;
            };
            let wait = err.retry_after.filter(|d| *d > base).unwrap_or(base);
            if remaining().is_some_and(|r| r <= wait) {
                // The wait alone would exhaust the cap: try the next route now.
                break;
            }
            if !opts.instant {
                if let Some(extra) = err.retry_after {
                    tasks::note_shared_rate_limit(admission_key, extra);
                }
            }
            let slept = std::time::Instant::now();
            let was_cancelled = sleep_recorded(&opts, wait).await;
            // Record what was actually waited (shorter when cancelled).
            let actual = if opts.instant {
                wait
            } else {
                slept.elapsed().min(wait)
            };
            report.waits.push(actual);
            backoff = actual;
            if was_cancelled {
                report.cancelled = true;
                break;
            }
        }
        if !report.cancelled {
            if let (Some(ledger), Some(failure)) = (opts.ledger.as_deref(), route_failure.as_ref())
            {
                if route_unwell(failure.category) {
                    ledger.open(route, Some(failure));
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

    fn budgeted() -> ExecuteOptions {
        ExecuteOptions {
            wire_budget: Some(WireBudget::INTERACTIVE),
            ..ExecuteOptions::for_tests()
        }
    }

    #[tokio::test]
    async fn wire_budget_bounds_a_four_route_chain_to_six_dispatches() {
        // Every failure proposes recovery: the shared allowance caps extras.
        let out = execute(
            &routes(4),
            |_i, _r| async {
                Err(DispatchError::new("bad").with_recovery("repair")) as Result<(), _>
            },
            budgeted(),
        )
        .await;
        assert_eq!(out.requests, 6);
        assert_eq!(WireBudget::INTERACTIVE.max_dispatches(4), 6);
        // Every route kept its reserved initial dispatch.
        for r in 0..4u32 {
            assert!(out
                .history
                .iter()
                .any(|h| h.route_index == r && h.dispatched));
        }
        assert!(out.waits.is_empty(), "recovery never sleeps");
    }

    #[tokio::test]
    async fn wire_budget_allows_one_primary_transient_retry_only() {
        let out = execute(
            &routes(2),
            |_i, _r| async { Err(DispatchError::new("503")) as Result<(), _> },
            budgeted(),
        )
        .await;
        // primary: initial + 1 transient retry; fallback: initial only.
        assert_eq!(out.requests, 3);
        assert_eq!(out.waits, vec![Duration::from_secs(10)]);
    }

    fn stall() -> DispatchError {
        use crate::provider_diag::{Category, ProviderFailure, Stage};
        DispatchError::from_failure(ProviderFailure::new(
            Stage::Stream,
            Category::Timeout,
            "no model output within 300s of response headers",
        ))
    }

    // A stalled thinking route is not resent; the chain falls through to the
    // next route (the route 0 repeat that cost ~6 minutes on main).
    #[tokio::test]
    async fn a_stalled_route_falls_through_instead_of_repeating() {
        let seen = Mutex::new(Vec::new());
        let out = execute_with_context(
            &routes(3),
            |ctx, _r| {
                seen.lock().unwrap().push((ctx.route_index, ctx.attempt));
                async move {
                    if ctx.route_index < 2 {
                        Err(stall())
                    } else {
                        Ok("answer")
                    }
                }
            },
            budgeted(),
        )
        .await;
        assert_eq!(out.value, Some("answer"));
        assert_eq!(*seen.lock().unwrap(), [(0, 1), (1, 1), (2, 1)]);
        assert!(out.waits.is_empty(), "no backoff before a fallback");
    }

    // Other transient failures still draw the primary's one retry, and a
    // primary stall with no fallback is retried because nothing else is left.
    #[tokio::test]
    async fn transient_failures_keep_their_retry_and_the_last_route_retries_a_stall() {
        let seen = Mutex::new(Vec::new());
        let out = execute_with_context(
            &routes(2),
            |ctx, _r| {
                seen.lock().unwrap().push((ctx.route_index, ctx.attempt));
                async move {
                    if ctx.route_index == 0 {
                        Err::<(), _>(DispatchError::new("503"))
                    } else {
                        Err(stall())
                    }
                }
            },
            budgeted(),
        )
        .await;
        assert!(out.value.is_none());
        assert_eq!(*seen.lock().unwrap(), [(0, 1), (0, 2), (1, 1)]);
        let lone = execute_with_context(
            &routes(1),
            |_ctx, _r| async { Err::<(), _>(stall()) },
            budgeted(),
        )
        .await;
        assert_eq!(
            lone.requests, 2,
            "no fallback: the stalled primary is retried"
        );
    }

    // The wall-clock cap ends the operation across routes and says so,
    // keeping the last real failure in the message and category `timeout`.
    #[tokio::test]
    async fn the_wall_clock_cap_ends_the_operation_with_a_timeout() {
        let mut opts = budgeted();
        opts.wire_budget = Some(WireBudget {
            wall_clock: Some(Duration::from_millis(150)),
            ..WireBudget::INTERACTIVE
        });
        let seen = Mutex::new(Vec::new());
        let out = execute_with_context(
            &routes(3),
            |ctx, _r| {
                seen.lock().unwrap().push(ctx.remaining);
                async move {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    Err::<(), _>(DispatchError::new("503 busy"))
                }
            },
            opts,
        )
        .await;
        assert!(out.wall_clock_exceeded && out.value.is_none());
        assert_eq!(out.requests, 1, "no dispatch after the cap");
        let failure = out.terminal_failure.unwrap();
        assert_eq!(failure.category, crate::provider_diag::Category::Timeout);
        assert!(
            failure.message.contains("wall-clock cap"),
            "{}",
            failure.message
        );
        let seen = seen.lock().unwrap();
        assert!(seen[0].unwrap() <= Duration::from_millis(150));
    }

    // Backoff that would outlast the cap is skipped rather than slept.
    #[tokio::test]
    async fn a_backoff_longer_than_the_remaining_cap_moves_on() {
        let mut opts = budgeted();
        opts.instant = false;
        opts.wire_budget = Some(WireBudget {
            wall_clock: Some(Duration::from_secs(5)),
            ..WireBudget::INTERACTIVE
        });
        let started = std::time::Instant::now();
        let out = execute_with_context(
            &routes(2),
            |ctx, _r| async move {
                if ctx.route_index == 0 {
                    Err(DispatchError::new("503"))
                } else {
                    Ok(())
                }
            },
            opts,
        )
        .await;
        // The 10s primary backoff would exceed the 5s cap: straight to route 1.
        assert!(out.value.is_some());
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(out.waits.is_empty());
    }

    #[tokio::test]
    async fn recovery_overrides_stop_and_reports_context() {
        let seen = Mutex::new(Vec::new());
        let out = execute_with_context(
            &routes(1),
            |ctx, _r| {
                seen.lock().unwrap().push(ctx.clone());
                async move {
                    if ctx.attempt == 1 {
                        let mut e = DispatchError::new("400 unsupported");
                        e.disposition = RetryDisposition::Stop;
                        Err(e.with_recovery("unsupported option"))
                    } else {
                        Ok(ctx.recovery.clone())
                    }
                }
            },
            budgeted(),
        )
        .await;
        assert_eq!(out.requests, 2);
        assert_eq!(out.value, Some(Some("unsupported option".to_string())));
        let seen = seen.lock().unwrap();
        assert_eq!(seen[1].wire_dispatch, 2);
        assert_eq!(seen[1].backoff_ms, 0);
    }

    #[tokio::test]
    async fn admission_polling_is_not_recorded_as_backoff() {
        let key = format!("queue-test-{}", std::process::id());
        let mut held = Vec::new();
        while let Some(g) = AdmissionGuard::try_enter_class(&key, false) {
            held.push(g);
        }
        let recorded = Arc::new(Mutex::new(Vec::new()));
        let mut rs = routes(1);
        rs[0].admission_key = Some(key.clone());
        let release = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            drop(held);
        });
        let out = execute(
            &rs,
            |_i, _r| async { Ok(()) },
            ExecuteOptions {
                recorded_waits: Some(recorded.clone()),
                ..ExecuteOptions::for_tests()
            },
        )
        .await;
        release.join().unwrap();
        assert_eq!(out.requests, 1);
        assert!(recorded.lock().unwrap().is_empty());
        assert!(out.waits.is_empty());
    }

    fn route(model: &str) -> Route {
        Route {
            provider: "p".into(),
            account: "a".into(),
            model: model.into(),
            skip_reason: None,
            admission_key: Some(format!("k:{model}")),
        }
    }

    fn timeout() -> DispatchError {
        DispatchError::from_failure(crate::provider_diag::ProviderFailure::new(
            crate::provider_diag::Stage::Stream,
            crate::provider_diag::Category::Timeout,
            "stalled",
        ))
    }

    async fn run(
        routes: &[Route],
        ledger: &Arc<StageLedger>,
        dead: &str,
    ) -> (ChainReport<()>, Vec<String>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        let dead = dead.to_string();
        let report = execute_with_context(
            routes,
            move |_, r| {
                log.lock().unwrap().push(r.model.clone());
                let fail = r.model == dead;
                async move {
                    if fail {
                        Err(timeout())
                    } else {
                        Ok(())
                    }
                }
            },
            ExecuteOptions {
                ledger: Some(ledger.clone()),
                ..budgeted()
            },
        )
        .await;
        let seen = seen.lock().unwrap().clone();
        (report, seen)
    }

    #[tokio::test]
    async fn a_route_that_just_stalled_is_passed_over_by_sibling_operations() {
        let routes = [route("a"), route("b")];
        let ledger = Arc::new(StageLedger::new(50));
        let (first, seen) = run(&routes, &ledger, "a").await;
        assert!(first.value.is_some());
        assert_eq!(seen, ["a", "b"]);
        let (second, seen) = run(&routes, &ledger, "a").await;
        assert!(second.value.is_some());
        assert_eq!(seen, ["b"], "route a is cooling");
        assert_eq!(ledger.dispatched(), 3);
    }

    #[tokio::test]
    async fn the_last_dispatchable_route_is_never_passed_over() {
        let routes = [route("a")];
        let ledger = Arc::new(StageLedger::new(50));
        let _ = run(&routes, &ledger, "a").await;
        let (_, seen) = run(&routes, &ledger, "a").await;
        assert!(!seen.is_empty());
    }

    #[tokio::test]
    async fn the_stage_budget_stops_dispatch_across_operations() {
        let routes = [route("a"), route("b")];
        let ledger = Arc::new(StageLedger::new(2));
        let (_, _) = run(&routes, &ledger, "none").await;
        let (_, _) = run(&routes, &ledger, "none").await;
        let (third, seen) = run(&routes, &ledger, "none").await;
        assert!(third.budget_exhausted && seen.is_empty());
        assert!(third.value.is_none());
    }

    fn failure(category: crate::provider_diag::Category, text: &str) -> DispatchError {
        DispatchError::from_failure(crate::provider_diag::ProviderFailure::new(
            crate::provider_diag::Stage::Response,
            category,
            text,
        ))
    }

    #[tokio::test]
    async fn a_missing_deployment_is_not_retried_on_another_route_to_the_same_model() {
        // Route 0: OpenRouter gateway id, route 2: same model direct.
        let routes = [
            route("nvidia/ultra:free"),
            route("models/gemini"),
            route("nvidia/ultra"),
        ];
        let ledger = Arc::new(StageLedger::new(50));
        let seen = Arc::new(Mutex::new(Vec::new()));
        for _ in 0..2 {
            let log = seen.clone();
            let report: ChainReport<()> = execute_with_context(
                &routes,
                move |_, r| {
                    log.lock().unwrap().push(r.model.clone());
                    let model = r.model.clone();
                    async move {
                        if model.contains("ultra") {
                            Err(failure(
                                crate::provider_diag::Category::InvalidModel,
                                "function not found",
                            ))
                        } else {
                            Err(failure(crate::provider_diag::Category::Server, "503"))
                        }
                    }
                },
                ExecuteOptions {
                    ledger: Some(ledger.clone()),
                    ..budgeted()
                },
            )
            .await;
            assert!(report.value.is_none());
        }
        let seen = seen.lock().unwrap().clone();
        // First op: ultra:free (reject), gemini x2 (overload retry on the last
        // live route), ultra skipped. Second op: ultra and gemini passed over
        // or retried only as the last live route; ultra never again.
        assert_eq!(
            seen.iter().filter(|m| m.contains("ultra")).count(),
            1,
            "{seen:?}"
        );
    }

    #[tokio::test]
    async fn the_last_live_route_gets_a_short_overload_retry() {
        let routes = [route("a"), route("b")];
        let ledger = Arc::new(StageLedger::new(50));
        let calls = Arc::new(Mutex::new(0u32));
        let counter = calls.clone();
        let report = execute_with_context(
            &routes,
            move |_, r| {
                let model = r.model.clone();
                let n = {
                    let mut c = counter.lock().unwrap();
                    *c += 1;
                    *c
                };
                async move {
                    match model.as_str() {
                        "a" => Err(failure(
                            crate::provider_diag::Category::InvalidModel,
                            "gone",
                        )),
                        _ if n < 3 => Err(failure(crate::provider_diag::Category::Server, "503")),
                        _ => Ok(()),
                    }
                }
            },
            ExecuteOptions {
                ledger: Some(ledger),
                ..budgeted()
            },
        )
        .await;
        assert!(report.value.is_some(), "{:?}", report.history);
    }

    #[tokio::test]
    async fn a_cooling_route_is_still_tried_when_every_later_route_is_a_dead_model() {
        // Live regression: [ultra:free, gemini, ultra] with ultra missing and
        // gemini recently timed out must still ask gemini.
        let routes = [
            route("nvidia/ultra:free"),
            route("models/gemini"),
            route("nvidia/ultra"),
        ];
        let ledger = Arc::new(StageLedger::new(50));
        ledger.open(
            &routes[1],
            Some(&crate::provider_diag::ProviderFailure::new(
                crate::provider_diag::Stage::Stream,
                crate::provider_diag::Category::Timeout,
                "stalled",
            )),
        );
        ledger.open(
            &routes[0],
            Some(&crate::provider_diag::ProviderFailure::new(
                crate::provider_diag::Stage::Response,
                crate::provider_diag::Category::InvalidModel,
                "function not found",
            )),
        );
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        let report: ChainReport<()> = execute_with_context(
            &routes,
            move |_, r| {
                log.lock().unwrap().push(r.model.clone());
                async { Ok(()) }
            },
            ExecuteOptions {
                ledger: Some(ledger),
                ..budgeted()
            },
        )
        .await;
        assert!(report.value.is_some());
        assert_eq!(seen.lock().unwrap().as_slice(), ["models/gemini"]);
    }
}
