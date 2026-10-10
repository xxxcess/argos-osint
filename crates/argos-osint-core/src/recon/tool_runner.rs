//! Shared OSINT tool execution runner and credit accounting (§4).
//!
//! Provides concurrency limiting (4), per-request timeouts (30s),
//! atomic in-flight deduplication, credit reservation/reconciliation,
//! and centralized cache projection across Chat and Intel investigations.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use chrono::Utc;
use serde_json::{json, Value};
use tokio::sync::{watch, Mutex, Semaphore};

use crate::osint::contracts::IntelligenceCategory;
use crate::osint::{self, ProviderKeys, ToolDefinition, ToolResult};
use crate::provider::SettingsFile;
use crate::recon::orchestrate::BudgetedOutcome;
use crate::recon::{CreditHold, PlanCall, Run, Store};
use crate::telemetry::{self, EventKind, TelemetryEvent, ToolOutcome, Trigger};

/// Who asked for a tool run. Attribution is passed down by the caller and is
/// never inferred from prompt text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolAttribution {
    /// Initiating app (`recon`, `cli`, `intel`, `atlas`).
    pub app: String,
    /// Why the run started.
    pub trigger: Trigger,
    /// Detected or executed mode, when the caller knows it.
    pub mode: String,
    /// Parent recon run, when the call belongs to one.
    pub run_id: String,
    pub thread_id: String,
    pub turn_id: String,
    /// Durable call id when the caller queued one.
    pub call_id: String,
}

/// An unattributed run is Recon turn traffic: the runner's own default.
impl Default for ToolAttribution {
    fn default() -> Self {
        Self::recon()
    }
}

impl ToolAttribution {
    /// Default runner attribution: an unattributed Recon turn call.
    pub fn recon() -> Self {
        Self {
            app: "recon".into(),
            trigger: Trigger::ReconPrompt,
            mode: String::new(),
            run_id: String::new(),
            thread_id: String::new(),
            turn_id: String::new(),
            call_id: String::new(),
        }
    }

    /// Attribution for the calls of one recon run.
    pub fn for_run(run: &Run, mode: &str) -> Self {
        Self {
            app: "recon".into(),
            trigger: Trigger::ReconPrompt,
            mode: mode.to_string(),
            run_id: run.id.clone(),
            thread_id: run.thread_id.clone(),
            turn_id: run.turn_id.clone(),
            call_id: String::new(),
        }
    }

    /// Attribution for a manual CLI tool run.
    pub fn manual() -> Self {
        Self {
            app: "cli".into(),
            trigger: Trigger::ManualTool,
            mode: String::new(),
            run_id: String::new(),
            thread_id: String::new(),
            turn_id: String::new(),
            call_id: String::new(),
        }
    }

    /// Bind the durable call id this invocation is recorded under.
    pub fn with_call_id(mut self, call_id: &str) -> Self {
        self.call_id = call_id.to_string();
        self
    }

    /// Override the mode label.
    pub fn with_mode(mut self, mode: &str) -> Self {
        self.mode = mode.to_string();
        self
    }
}

/// Shared tool execution runner with concurrency bounding and deduplication.
#[derive(Clone)]
pub struct ToolRunner {
    pub db_path: PathBuf,
    pub settings: SettingsFile,
    pub keys: ProviderKeys,
    concurrency: Arc<Semaphore>,
    inflight: Arc<Mutex<HashMap<String, watch::Receiver<Option<ToolResult>>>>>,
    /// Default attribution for calls that do not carry their own.
    attribution: ToolAttribution,
}

impl ToolRunner {
    /// Maximum concurrent OSINT network requests across all tools.
    pub const MAX_CONCURRENCY: usize = 4;
    /// Per-request timeout for tool execution.
    pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
    /// Cache lifetime for a named-SERP result with usable rows (spec §9 fix 7).
    pub const SERP_VALID_CACHE_SECONDS: u64 = 15 * 60;
    /// Cache lifetime for a named-SERP verified zero (spec §9 fix 7).
    pub const SERP_ZERO_CACHE_SECONDS: u64 = 2 * 60;

    pub fn new(db_path: impl AsRef<Path>, settings: SettingsFile, keys: ProviderKeys) -> Self {
        Self {
            db_path: db_path.as_ref().to_path_buf(),
            settings,
            keys,
            concurrency: Arc::new(Semaphore::new(Self::MAX_CONCURRENCY)),
            inflight: Arc::new(Mutex::new(HashMap::new())),
            attribution: ToolAttribution::recon(),
        }
    }

    /// Replace the default attribution for calls that do not carry their own.
    pub fn with_attribution(mut self, attribution: ToolAttribution) -> Self {
        self.attribution = attribution;
        self
    }

    /// Execute a single tool with caching, in-flight dedup, and credit accounting.
    pub async fn execute_tool(
        &self,
        tool_id: &str,
        inputs: Value,
        refresh: bool,
        cancel: &Arc<AtomicBool>,
    ) -> Result<ToolResult> {
        self.execute_tool_inner(tool_id, inputs, refresh, cancel, true, &self.attribution)
            .await
    }

    /// Execute a single tool under an explicit attribution context.
    pub async fn execute_tool_attributed(
        &self,
        tool_id: &str,
        inputs: Value,
        refresh: bool,
        cancel: &Arc<AtomicBool>,
        attribution: &ToolAttribution,
    ) -> Result<ToolResult> {
        self.execute_tool_inner(tool_id, inputs, refresh, cancel, true, attribution)
            .await
    }

    pub async fn execute_tool_without_reservation(
        &self,
        tool_id: &str,
        inputs: Value,
        refresh: bool,
        cancel: &Arc<AtomicBool>,
    ) -> Result<ToolResult> {
        self.execute_tool_inner(tool_id, inputs, refresh, cancel, false, &self.attribution)
            .await
    }

    /// Execute a single tool without reservation under an explicit attribution.
    pub async fn execute_tool_without_reservation_attributed(
        &self,
        tool_id: &str,
        inputs: Value,
        refresh: bool,
        cancel: &Arc<AtomicBool>,
        attribution: &ToolAttribution,
    ) -> Result<ToolResult> {
        self.execute_tool_inner(tool_id, inputs, refresh, cancel, false, attribution)
            .await
    }

    async fn execute_tool_inner(
        &self,
        tool_id: &str,
        inputs: Value,
        refresh: bool,
        cancel: &Arc<AtomicBool>,
        reserve_credits: bool,
        attribution: &ToolAttribution,
    ) -> Result<ToolResult> {
        let clock = telemetry::Measured::start();
        if cancel.load(Ordering::Relaxed) {
            // A cancelled call is a cancelled invocation, never zero results.
            self.note_cancelled(tool_id, attribution, &clock);
            return Err(anyhow!("cancelled"));
        }

        let canonical = osint::canonical_tool_id(tool_id);
        let def =
            osint::definition(canonical).ok_or_else(|| anyhow!("unknown tool: {canonical}"))?;
        let cache_key = osint::cache_identity(canonical, &inputs);

        // 1. Cache hit check
        if !refresh {
            if let Ok(store) = Store::open(&self.db_path) {
                if let Ok(Some(cached)) = store.cache_get(&cache_key) {
                    let projected = osint::project_cached_result(canonical, &inputs, &cached);
                    // A cache hit is served locally: it is not a remote request and
                    // a cache miss shape never counts as zero results.
                    self.note_invocation(InvocationFact {
                        canonical,
                        def,
                        attribution,
                        cache_key: &cache_key,
                        mode: "cache",
                        result: Some(&projected),
                        timed_out: false,
                        clock: &clock,
                    });
                    return Ok(projected);
                }
            }
        }

        // 2. In-flight deduplication
        let (tx, mut rx, is_leader) = {
            let mut inflight = self.inflight.lock().await;
            if let Some(existing_rx) = inflight.get(&cache_key) {
                (None, existing_rx.clone(), false)
            } else {
                let (new_tx, new_rx) = watch::channel(None);
                inflight.insert(cache_key.clone(), new_rx.clone());
                (Some(new_tx), new_rx, true)
            }
        };

        if !is_leader {
            // Follower waits for leader's result
            let wait_res = tokio::select! {
                res = rx.changed() => res,
                _ = wait_cancellation(cancel) => {
                    self.note_invocation(InvocationFact {
                        canonical,
                        def,
                        attribution,
                        cache_key: &cache_key,
                        mode: "local",
                        result: None,
                        timed_out: false,
                        clock: &clock,
                    });
                    return Err(anyhow!("cancelled"));
                }
            };
            if wait_res.is_ok() {
                if let Some(res) = rx.borrow().as_ref() {
                    let projected = osint::project_cached_result(canonical, &inputs, res);
                    // Waiting on a sibling is a local resolution, not a remote request.
                    self.note_invocation(InvocationFact {
                        canonical,
                        def,
                        attribution,
                        cache_key: &cache_key,
                        mode: "local",
                        result: Some(&projected),
                        timed_out: false,
                        clock: &clock,
                    });
                    return Ok(projected);
                }
            }
            // If leader dropped or failed unexpectedly, fallback to direct run below
        }

        // RAII cleanup guard to remove cache_key from inflight map
        struct InflightGuard {
            key: String,
            map: Arc<Mutex<HashMap<String, watch::Receiver<Option<ToolResult>>>>>,
        }
        impl Drop for InflightGuard {
            fn drop(&mut self) {
                let key = self.key.clone();
                let map = self.map.clone();
                tokio::spawn(async move {
                    let mut lock = map.lock().await;
                    lock.remove(&key);
                });
            }
        }
        let _guard = InflightGuard {
            key: cache_key.clone(),
            map: self.inflight.clone(),
        };

        // 3. Check enabled status
        {
            let store = Store::open(&self.db_path)?;
            if !store.tool_enabled(canonical)? {
                self.note_failure(
                    canonical,
                    def,
                    attribution,
                    &cache_key,
                    "tool_disabled",
                    &clock,
                );
                return Err(anyhow!("tool disabled in catalog: {canonical}"));
            }
        }

        // 4. Reserve credits if needed
        let limits = &self.settings.recon_limits;
        let mut credit_hold: Option<CreditHold> = None;
        let cost_info = limits.configured_cost_for(canonical, &inputs);
        if reserve_credits {
            if let Some((provider_name, cost)) = cost_info {
                if cost > 0 {
                    let store = Store::open(&self.db_path)?;
                    credit_hold = store.reserve_credits(provider_name, cost, limits)?;
                    if credit_hold.is_none() {
                        let available = store.credits_available(provider_name, limits)?;
                        self.note_failure(
                            canonical,
                            def,
                            attribution,
                            &cache_key,
                            "credit_allowance",
                            &clock,
                        );
                        return Err(anyhow!(
                            "{canonical} — Argos {provider_name} credit allowance is exhausted ({available} left, needs {cost}; local monthly cap, not the provider dashboard)"
                        ));
                    }
                }
            }
        }

        // 5. Concurrency limit permit & timeout execution
        let permit = tokio::select! {
            p = self.concurrency.acquire() => p.map_err(|e| anyhow!("{e}"))?,
            _ = wait_cancellation(cancel) => {
                if reserve_credits {
                    if let (Some((_provider, _)), Some(hold)) = (cost_info, &credit_hold) {
                        let _ = Store::open(&self.db_path).map(|s| s.release_credits(hold));
                    }
                }
                self.note_cancelled(canonical, attribution, &clock);
                return Err(anyhow!("cancelled"));
            }
        };

        let executor = crate::osint::Executor::new()?;
        let ua = if self.settings.osint_user_agent.trim().is_empty() {
            None
        } else {
            Some(self.settings.osint_user_agent.as_str())
        };

        // Each tool runs under its own configured timeout, which stays below the
        // turn's outer deadline (spec §9 fix 6).
        let request_timeout = tool_timeout(def);
        let exec_fut = executor.run_configured(canonical, inputs.clone(), ua, &self.keys);
        let mut timed_out = false;
        let result_outcome = tokio::select! {
            res = tokio::time::timeout(request_timeout, exec_fut) => match res {
                Ok(r) => r,
                Err(_) => {
                    timed_out = true;
                    Ok(ToolResult {
                        tool_id: canonical.to_string(),
                        inputs: inputs.clone(),
                        status: "failed".into(),
                        source_url: String::new(),
                        retrieved_at: Utc::now().to_rfc3339(),
                        observations: Value::Null,
                        raw: String::new(),
                        error: Some(format!("tool execution timed out after {}s", request_timeout.as_secs())),
                        cached: false,
                        truncated: false,
                        credits_charged: 0,
                        credits_reported: None,
                    })
                }
            },
            _ = wait_cancellation(cancel) => {
                if reserve_credits {
                    if let (Some((_provider, _)), Some(hold)) = (cost_info, &credit_hold) {
                        let _ = Store::open(&self.db_path).map(|s| s.release_credits(hold));
                    }
                }
                self.note_cancelled(canonical, attribution, &clock);
                return Err(anyhow!("cancelled"));
            }
        };
        drop(permit);

        let result = match result_outcome {
            Ok(res) => res,
            Err(err) => {
                if reserve_credits {
                    if let (Some((_provider, _)), Some(hold)) = (cost_info, &credit_hold) {
                        let _ = Store::open(&self.db_path).map(|s| s.release_credits(hold));
                    }
                }
                self.note_failure(canonical, def, attribution, &cache_key, "transport", &clock);
                return Err(err);
            }
        };

        // 6. Settle or release credit hold
        if reserve_credits {
            if let (Some((_provider_name, _)), Some(hold)) = (cost_info, &credit_hold) {
                if let Ok(store) = Store::open(&self.db_path) {
                    let spend = matches!(result.status.as_str(), "completed" | "no_results")
                        && !result.cached;
                    let uncertain_whoxy = canonical == crate::osint::whoxy::TOOL_ID
                        && result.status == "failed"
                        && result.error.as_deref().is_some_and(|err| {
                            let lower = err.to_ascii_lowercase();
                            lower.contains("timeout")
                                || lower.contains("timed out")
                                || lower.contains("connection")
                                || lower.contains("error sending")
                        });
                    if spend || uncertain_whoxy {
                        let estimate = hold.trial_credits + hold.allowance_credits;
                        let actual = result.credits_reported.unwrap_or(estimate);
                        let _ = store.reconcile_credits(hold, actual);
                    } else {
                        let _ = store.release_credits(hold);
                    }
                }
            }
        }

        // 7. Cache put. Named SERP tools use the semantic policy below; every
        // other tool keeps its catalog cache lifetime.
        let outcome = tool_outcome(&result, timed_out);
        if osint_cacheable(&result) {
            if let Some(ttl) = cache_ttl_for(def, &outcome) {
                if let Ok(store) = Store::open(&self.db_path) {
                    let _ = store.cache_put(&cache_key, &result, ttl);
                }
            }
        }

        // 8. NewsAPI quota charge
        if !result.cached && canonical.starts_with("newsapi_") {
            if let Ok(store) = Store::open(&self.db_path) {
                let bucket = if osint::key_exhausted(&self.keys.newsapi)
                    && !self.keys.newsapi_fallback.trim().is_empty()
                {
                    "newsapi:fallback"
                } else {
                    "newsapi"
                };
                crate::atlas::charge_quota(&store, bucket);
            }
        }

        // 9. Notify in-flight waiters
        if let Some(sender) = tx {
            let _ = sender.send(Some(result.clone()));
        }

        // One terminal row for this logical invocation, plus the remote attempts
        // it actually made. A cache hit never reaches here.
        self.note_invocation(InvocationFact {
            canonical,
            def,
            attribution,
            cache_key: &cache_key,
            mode: "remote",
            result: Some(&result),
            timed_out,
            clock: &clock,
        });

        Ok(result)
    }

    /// Execute a budgeted batch of plan calls with reservation and reconciliation.
    pub async fn execute_budgeted_calls(
        &self,
        run: &Run,
        calls: &[PlanCall],
        cancel: &Arc<AtomicBool>,
    ) -> Result<BudgetedOutcome> {
        if calls.is_empty() {
            return Ok(BudgetedOutcome {
                results: Vec::new(),
                skipped: Vec::new(),
            });
        }
        let limits = &self.settings.recon_limits;
        let store = Store::open(&self.db_path)?;
        let mut affordable = Vec::new();
        let mut skipped = Vec::new();
        let mut holds: Vec<(String, CreditHold)> = Vec::new();

        for call in calls {
            if cancel.load(Ordering::Relaxed) {
                return Err(anyhow!("cancelled"));
            }
            if !store.tool_enabled(&call.tool_id)? {
                skipped.push(format!("{} — disabled in the catalog", call.tool_id));
                continue;
            }
            if store.inflight_duplicate(&call.tool_id, &call.arguments)? {
                skipped.push(format!(
                    "{} — a duplicate call is already queued or running",
                    call.tool_id
                ));
                continue;
            }
            let cache_key = osint::cache_identity(&call.tool_id, &call.arguments);
            let cached = store.cache_get(&cache_key)?.is_some();
            if let Some((provider_name, cost)) =
                limits.configured_cost_for(&call.tool_id, &call.arguments)
            {
                if !cached && cost > 0 {
                    match store.reserve_credits(provider_name, cost, limits)? {
                        Some(hold) => {
                            holds.push((format!("{}:{}", call.tool_id, call.arguments), hold));
                        }
                        None => {
                            let available = store.credits_available(provider_name, limits)?;
                            skipped.push(format!(
                                "{} — Argos {provider_name} credit allowance is exhausted ({available} left, needs {cost}; local monthly cap, not the provider dashboard)",
                                call.tool_id
                            ));
                            continue;
                        }
                    }
                }
            }
            affordable.push(PlanCall {
                depends_on: Vec::new(),
                ..call.clone()
            });
        }
        drop(store);

        if affordable.is_empty() {
            return Ok(BudgetedOutcome {
                results: Vec::new(),
                skipped,
            });
        }

        let mut results = Vec::new();
        for call in affordable {
            if cancel.load(Ordering::Relaxed) {
                self.release_holds(&holds)?;
                return Err(anyhow!("cancelled"));
            }
            let store = Store::open(&self.db_path)?;
            for value in call
                .arguments
                .as_object()
                .into_iter()
                .flat_map(|m| m.values())
                .filter_map(Value::as_str)
            {
                for (kind, canonical) in crate::recon::explicit_entities(value) {
                    let _ = store.link_entity(&run.thread_id, &kind, &canonical, None);
                }
            }
            let call_id = store.queue_call(
                &call.tool_id,
                &call.arguments,
                "recon",
                Some(&run.id),
                Some(&run.thread_id),
                Some(&run.turn_id),
            )?;
            drop(store);

            // Every call is attributed to the run it belongs to, and to the durable
            // call id that carries the same invocation.
            let attribution = ToolAttribution::for_run(run, "").with_call_id(&call_id);
            let res = self
                .execute_tool_without_reservation_attributed(
                    &call.tool_id,
                    call.arguments.clone(),
                    false,
                    cancel,
                    &attribution,
                )
                .await;
            let result = match res {
                Ok(r) => r,
                Err(e) if e.to_string() == "cancelled" || e.to_string().contains("cancelled") => {
                    ToolResult {
                        tool_id: call.tool_id.clone(),
                        inputs: call.arguments.clone(),
                        status: "cancelled".into(),
                        source_url: String::new(),
                        retrieved_at: Utc::now().to_rfc3339(),
                        observations: Value::Null,
                        raw: String::new(),
                        error: None,
                        cached: false,
                        truncated: false,
                        credits_charged: 0,
                        credits_reported: None,
                    }
                }
                Err(e) => ToolResult {
                    tool_id: call.tool_id.clone(),
                    inputs: call.arguments.clone(),
                    status: "failed".into(),
                    source_url: String::new(),
                    retrieved_at: Utc::now().to_rfc3339(),
                    observations: Value::Null,
                    raw: String::new(),
                    error: Some(e.to_string()),
                    cached: false,
                    truncated: false,
                    credits_charged: 0,
                    credits_reported: None,
                },
            };
            let store = Store::open(&self.db_path)?;
            store.finish_call(&call_id, &result)?;
            if let Some(hosts) = result
                .observations
                .get("hostnames")
                .and_then(Value::as_array)
            {
                for host in hosts.iter().filter_map(Value::as_str).take(100) {
                    for (kind, canonical) in crate::recon::explicit_entities(host) {
                        let _ =
                            store.link_entity(&run.thread_id, &kind, &canonical, Some(&call_id));
                    }
                }
            }
            results.push((call_id, result));
        }

        self.settle_holds(&results, holds)?;
        Ok(BudgetedOutcome { results, skipped })
    }

    fn release_holds(&self, holds: &[(String, CreditHold)]) -> Result<()> {
        let store = Store::open(&self.db_path)?;
        for (_, hold) in holds {
            store.release_credits(hold)?;
        }
        Ok(())
    }

    pub fn settle_holds(
        &self,
        results: &[(String, ToolResult)],
        mut holds: Vec<(String, CreditHold)>,
    ) -> Result<()> {
        let store = Store::open(&self.db_path)?;
        for (_, result) in results {
            let signature = format!("{}:{}", result.tool_id, result.inputs);
            let Some(index) = holds.iter().position(|(sig, _)| sig == &signature) else {
                continue;
            };
            let (_, hold) = holds.swap_remove(index);
            let spend =
                matches!(result.status.as_str(), "completed" | "no_results") && !result.cached;
            if spend {
                let estimate = hold.trial_credits + hold.allowance_credits;
                let actual = result.credits_reported.unwrap_or(estimate);
                store.reconcile_credits(&hold, actual)?;
            } else {
                store.release_credits(&hold)?;
            }
        }
        for (_, hold) in holds {
            store.release_credits(&hold)?;
        }
        Ok(())
    }
}

async fn wait_cancellation(cancel: &Arc<AtomicBool>) {
    while !cancel.load(Ordering::Relaxed) {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Named search engines whose SERP pages Argos parses itself. Named engines are
/// the only tools with a semantic cache policy (spec §9 fix 7).
pub const NAMED_SERP_TOOLS: [&str; 3] = [
    "firecrawl_google_search",
    "firecrawl_yandex_search",
    "firecrawl_mojeek_search",
];

/// Named-SERP cache classes. The rewritten `search_engines` module will publish
/// this vocabulary directly; until then the local shape is derived here so the
/// cache policy has a single owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SerpCacheClass {
    /// Usable result rows.
    Valid,
    /// The engine reported zero results and Argos verified that.
    VerifiedZero,
    /// Challenge, consent, block, parser mismatch or transport failure.
    Failure,
}

impl SerpCacheClass {
    fn from_outcome(outcome: ToolOutcome) -> Self {
        match outcome {
            ToolOutcome::CompletedNonEmpty | ToolOutcome::CompletedPartial => Self::Valid,
            ToolOutcome::CompletedVerifiedZero => Self::VerifiedZero,
            ToolOutcome::Failed
            | ToolOutcome::Blocked
            | ToolOutcome::ParserMismatch
            | ToolOutcome::Cancelled => Self::Failure,
        }
    }

    /// Cache lifetime for this class on a named engine.
    pub fn cache_seconds(self) -> Option<u64> {
        match self {
            Self::Valid => Some(ToolRunner::SERP_VALID_CACHE_SECONDS),
            Self::VerifiedZero => Some(ToolRunner::SERP_ZERO_CACHE_SECONDS),
            // Challenge, mismatch and transport errors are never cached.
            Self::Failure => None,
        }
    }
}

/// Named engine identity for a tool id, when it has one.
pub fn named_engine(tool_id: &str) -> Option<&'static str> {
    match tool_id {
        "firecrawl_google_search" => Some("google"),
        "firecrawl_yandex_search" => Some("yandex"),
        "firecrawl_mojeek_search" => Some("mojeek"),
        _ => None,
    }
}

/// True for the named search engines Argos parses itself.
pub fn is_named_serp(tool_id: &str) -> bool {
    NAMED_SERP_TOOLS.contains(&tool_id)
}

/// The tool's own configured timeout, which stays below the turn's outer
/// deadline. Falls back to the shared request timeout when a definition sets
/// none (spec §9 fix 6).
pub fn tool_timeout(def: &ToolDefinition) -> Duration {
    if def.timeout_seconds == 0 {
        ToolRunner::REQUEST_TIMEOUT
    } else {
        Duration::from_secs(def.timeout_seconds)
    }
}

/// Cache lifetime for a terminal tool outcome. Named SERP tools use the semantic
/// policy (valid 15m, verified zero 2m, nothing cached for challenge, mismatch
/// or transport errors); every other tool keeps its catalog lifetime.
pub fn cache_ttl_for(tool: &ToolDefinition, outcome: &ToolOutcome) -> Option<u64> {
    if is_named_serp(tool.id) {
        return SerpCacheClass::from_outcome(*outcome).cache_seconds();
    }
    Some(tool.cache_seconds)
}

/// Terminal outcome of one tool invocation. A failed transport is never zero
/// results, and a verified zero is only what the source actually reported.
pub fn tool_outcome(result: &ToolResult, timed_out: bool) -> ToolOutcome {
    if timed_out {
        return ToolOutcome::Failed;
    }
    match result.status.as_str() {
        "cancelled" => ToolOutcome::Cancelled,
        "completed" | "no_results" => {
            if result.status == "no_results" {
                return ToolOutcome::CompletedVerifiedZero;
            }
            let rows = osint::extract_observation_items(&result.observations).len();
            if rows == 0 {
                ToolOutcome::CompletedVerifiedZero
            } else if result.truncated {
                ToolOutcome::CompletedPartial
            } else {
                ToolOutcome::CompletedNonEmpty
            }
        }
        _ => failure_outcome(result.error.as_deref().unwrap_or_default()),
    }
}

/// Bounded machine reason for a failed invocation.
fn failure_reason(error: &str, timed_out: bool) -> &'static str {
    if timed_out {
        return "tool_timeout";
    }
    let lower = error.to_ascii_lowercase();
    if lower.contains("rate limit") || lower.contains("rate_limited") || lower.contains("429") {
        "rate_limited"
    } else if lower.contains("timed out") || lower.contains("timeout") {
        "timeout"
    } else if lower.contains("unauthor")
        || lower.contains("forbidden")
        || lower.contains("api key")
        || lower.contains("credential")
    {
        "auth"
    } else if lower.contains("quota") || lower.contains("credit") {
        "quota"
    } else if lower.is_empty() {
        "unknown"
    } else {
        "transport"
    }
}

/// Challenge, consent and block pages are not zero results and not plain
/// transport failures; parser mismatches stay their own class.
fn failure_outcome(error: &str) -> ToolOutcome {
    let lower = error.to_ascii_lowercase();
    if lower.contains("captcha")
        || lower.contains("challenge")
        || lower.contains("consent")
        || lower.contains("unusual traffic")
        || lower.contains("blocked")
    {
        return ToolOutcome::Blocked;
    }
    if lower.contains("parser mismatch") || lower.contains("could not classify") {
        return ToolOutcome::ParserMismatch;
    }
    ToolOutcome::Failed
}

/// Remote attempts the executor reports on a result. The executor does not
/// publish this counter yet; when it does, it is read here so a logical
/// invocation and its remote attempts are never the same number.
fn remote_attempts(result: &ToolResult) -> u32 {
    result
        .observations
        .get("remote_requests")
        .and_then(Value::as_u64)
        .unwrap_or(0)
        .min(u32::MAX as u64) as u32
}

/// Everything one terminal telemetry row needs.
struct InvocationFact<'a> {
    canonical: &'a str,
    def: &'a ToolDefinition,
    attribution: &'a ToolAttribution,
    cache_key: &'a str,
    /// `cache`, `local` (dedup wait) or `remote`.
    mode: &'static str,
    result: Option<&'a ToolResult>,
    timed_out: bool,
    clock: &'a telemetry::Measured,
}

impl ToolRunner {
    /// Best-effort telemetry write; never fails a tool run.
    fn note(&self, event: &TelemetryEvent) {
        let _ = telemetry::record(&self.db_path, event);
    }

    /// Stable id for one logical invocation. The durable call id wins so a
    /// replayed or resumed call cannot be counted twice.
    fn invocation_id(canonical: &str, cache_key: &str, call_id: &str) -> String {
        if call_id.is_empty() {
            format!(
                "tool-inv-{canonical}-{}",
                crate::evidence::content_hash(cache_key)
            )
        } else {
            call_id.to_string()
        }
    }

    /// One `ToolInvocation` row per logical invocation, plus the remote
    /// attempts and named-engine queries it actually made.
    fn note_invocation(&self, fact: InvocationFact<'_>) {
        let outcome = match fact.result {
            Some(result) => tool_outcome(result, fact.timed_out),
            // No result means the invocation was cancelled before dispatch.
            None => ToolOutcome::Cancelled,
        };
        let reason = match fact.result {
            Some(result) => {
                let error = result.error.as_deref().unwrap_or_default();
                if outcome.reached_source() {
                    ""
                } else {
                    failure_reason(error, fact.timed_out)
                }
            }
            None => "cancelled",
        };
        let result_count = fact
            .result
            .map(|result| osint::extract_observation_items(&result.observations).len())
            .unwrap_or(0);
        let attempts = fact.result.map(remote_attempts).unwrap_or(0);
        let id = Self::invocation_id(fact.canonical, fact.cache_key, &fact.attribution.call_id);
        let category = IntelligenceCategory::for_tool(fact.canonical)
            .map(|category| category.as_str().to_string())
            .unwrap_or_default();
        let event = TelemetryEvent::new(EventKind::ToolInvocation)
            .with_id(id.clone())
            .canonical(fact.cache_key)
            .at(chrono::Utc::now().to_rfc3339())
            .app(&fact.attribution.app)
            .trigger(fact.attribution.trigger)
            .tool(fact.canonical)
            .category(category)
            .mode(fact.mode)
            .outcome(outcome.as_str())
            .reason(reason)
            .provider(osint::primary_provider(fact.canonical).unwrap_or("public"))
            .run(&fact.attribution.run_id)
            .thread(&fact.attribution.thread_id)
            .turn(&fact.attribution.turn_id)
            .call(&fact.attribution.call_id)
            .duration_ms(Some(fact.clock.elapsed_ms()))
            .count(1)
            .payload(json!({
                "cache_mode": fact.mode,
                "requested_mode": fact.attribution.mode,
                "cached": fact.result.is_some_and(|result| result.cached),
                "result_count": result_count,
                "remote_requests": attempts,
                "tool_timeout_seconds": tool_timeout(fact.def).as_secs(),
                "engine": named_engine(fact.canonical).unwrap_or_default(),
                "truncated": fact.result.is_some_and(|result| result.truncated),
                "credits_charged": fact.result.map(|result| result.credits_charged).unwrap_or(0),
            }));
        self.note(&event);

        // Remote attempts are counted separately from the logical invocation, and
        // only for invocations that actually reached a provider.
        if fact.mode == "remote" && attempts > 0 {
            let wire = TelemetryEvent::new(EventKind::ToolWireRequest)
                .with_id(format!("{id}-wire"))
                .canonical(id.clone())
                .at(chrono::Utc::now().to_rfc3339())
                .app(&fact.attribution.app)
                .trigger(fact.attribution.trigger)
                .tool(fact.canonical)
                .category(
                    IntelligenceCategory::for_tool(fact.canonical)
                        .map(|c| c.as_str().to_string())
                        .unwrap_or_default(),
                )
                .mode("remote")
                .outcome(outcome.as_str())
                .provider(osint::primary_provider(fact.canonical).unwrap_or("public"))
                .engine(named_engine(fact.canonical).unwrap_or_default())
                .run(&fact.attribution.run_id)
                .thread(&fact.attribution.thread_id)
                .call(&fact.attribution.call_id)
                .duration_ms(Some(fact.clock.elapsed_ms()))
                .count(i64::from(attempts));
            self.note(&wire);
        }

        // Named engines keep their own query identity, separate from the
        // scraping transport that fetched the page.
        let engine = if fact.mode == "remote" {
            named_engine(fact.canonical)
        } else {
            None
        };
        if let Some(engine) = engine {
            let query = TelemetryEvent::new(EventKind::ToolEngineQuery)
                .with_id(format!("{id}-engine"))
                .canonical(id)
                .at(chrono::Utc::now().to_rfc3339())
                .app(&fact.attribution.app)
                .trigger(fact.attribution.trigger)
                .tool(fact.canonical)
                .engine(engine)
                .mode("remote")
                .outcome(outcome.as_str())
                .reason(reason)
                .provider(osint::primary_provider(fact.canonical).unwrap_or("public"))
                .run(&fact.attribution.run_id)
                .thread(&fact.attribution.thread_id)
                .call(&fact.attribution.call_id)
                .duration_ms(Some(fact.clock.elapsed_ms()))
                .count(1)
                .payload(json!({"result_count": result_count}));
            self.note(&query);
        }
    }

    /// Terminal failure recorded before the call is dispatched.
    fn note_failure(
        &self,
        canonical: &str,
        def: &ToolDefinition,
        attribution: &ToolAttribution,
        cache_key: &str,
        reason: &str,
        clock: &telemetry::Measured,
    ) {
        let id = Self::invocation_id(canonical, cache_key, &attribution.call_id);
        let event = TelemetryEvent::new(EventKind::ToolInvocation)
            .with_id(id)
            .canonical(cache_key)
            .at(chrono::Utc::now().to_rfc3339())
            .app(&attribution.app)
            .trigger(attribution.trigger)
            .tool(canonical)
            .category(
                IntelligenceCategory::for_tool(canonical)
                    .map(|c| c.as_str().to_string())
                    .unwrap_or_default(),
            )
            .provider(osint::primary_provider(canonical).unwrap_or("public"))
            .outcome(ToolOutcome::Failed.as_str())
            .reason(reason)
            .mode("remote")
            .run(&attribution.run_id)
            .thread(&attribution.thread_id)
            .turn(&attribution.turn_id)
            .call(&attribution.call_id)
            .duration_ms(Some(clock.elapsed_ms()))
            .count(1)
            .payload(json!({"tool_timeout_seconds": tool_timeout(def).as_secs()}));
        self.note(&event);
    }

    /// A cancelled invocation. Cancellation is never a zero result.
    fn note_cancelled(
        &self,
        tool_id: &str,
        attribution: &ToolAttribution,
        clock: &telemetry::Measured,
    ) {
        let canonical = osint::canonical_tool_id(tool_id);
        let id = Self::invocation_id(canonical, "", &attribution.call_id);
        let event = TelemetryEvent::new(EventKind::ToolInvocation)
            .with_id(id)
            .canonical(canonical)
            .at(chrono::Utc::now().to_rfc3339())
            .app(&attribution.app)
            .trigger(attribution.trigger)
            .tool(canonical)
            .category(
                IntelligenceCategory::for_tool(canonical)
                    .map(|c| c.as_str().to_string())
                    .unwrap_or_default(),
            )
            .provider(osint::primary_provider(canonical).unwrap_or("public"))
            .outcome(ToolOutcome::Cancelled.as_str())
            .reason("cancelled")
            .run(&attribution.run_id)
            .thread(&attribution.thread_id)
            .turn(&attribution.turn_id)
            .call(&attribution.call_id)
            .duration_ms(Some(clock.elapsed_ms()))
            .count(1);
        self.note(&event);
    }
}

fn osint_cacheable(result: &ToolResult) -> bool {
    matches!(result.status.as_str(), "completed" | "no_results")
        && result.error.as_deref().unwrap_or("").is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value as JsonValue;

    fn result(
        tool_id: &str,
        status: &str,
        observations: JsonValue,
        error: Option<&str>,
    ) -> ToolResult {
        ToolResult {
            tool_id: tool_id.into(),
            inputs: json!({"query": "x"}),
            status: status.into(),
            source_url: String::new(),
            retrieved_at: Utc::now().to_rfc3339(),
            observations,
            raw: String::new(),
            error: error.map(str::to_string),
            cached: false,
            truncated: false,
            credits_charged: 0,
            credits_reported: None,
        }
    }

    fn definition(id: &str) -> &'static ToolDefinition {
        osint::definition(id).expect("catalog definition")
    }

    #[test]
    fn named_serp_cache_policy_is_semantic() {
        let serp = definition("firecrawl_google_search");
        assert_eq!(
            cache_ttl_for(serp, &ToolOutcome::CompletedNonEmpty),
            Some(ToolRunner::SERP_VALID_CACHE_SECONDS)
        );
        assert_eq!(
            cache_ttl_for(serp, &ToolOutcome::CompletedPartial),
            Some(ToolRunner::SERP_VALID_CACHE_SECONDS)
        );
        assert_eq!(
            cache_ttl_for(serp, &ToolOutcome::CompletedVerifiedZero),
            Some(ToolRunner::SERP_ZERO_CACHE_SECONDS)
        );
        // Challenge, consent, parser mismatch and transport errors cache nothing.
        for outcome in [
            ToolOutcome::Blocked,
            ToolOutcome::ParserMismatch,
            ToolOutcome::Failed,
            ToolOutcome::Cancelled,
        ] {
            assert_eq!(cache_ttl_for(serp, &outcome), None, "{outcome:?}");
        }
        assert_eq!(SerpCacheClass::Valid.cache_seconds(), Some(15 * 60));
        assert_eq!(SerpCacheClass::VerifiedZero.cache_seconds(), Some(2 * 60));
        assert_eq!(SerpCacheClass::Failure.cache_seconds(), None);
    }

    #[test]
    fn every_other_tool_keeps_its_catalog_cache_lifetime() {
        let crtsh = definition("crtsh_certificates");
        assert_eq!(
            cache_ttl_for(crtsh, &ToolOutcome::CompletedNonEmpty),
            Some(crtsh.cache_seconds)
        );
        assert_eq!(
            cache_ttl_for(crtsh, &ToolOutcome::Failed),
            Some(crtsh.cache_seconds)
        );
    }

    #[test]
    fn serp_outcome_classes_separate_valid_zero_and_failure() {
        let valid = tool_outcome(
            &result(
                "firecrawl_yandex_search",
                "completed",
                json!({"results": [{"url": "https://a.example"}]}),
                None,
            ),
            false,
        );
        assert_eq!(valid, ToolOutcome::CompletedNonEmpty);
        let zero = tool_outcome(
            &result(
                "firecrawl_yandex_search",
                "no_results",
                json!({"results": []}),
                None,
            ),
            false,
        );
        assert_eq!(zero, ToolOutcome::CompletedVerifiedZero);
        let blocked = tool_outcome(
            &result(
                "firecrawl_yandex_search",
                "failed",
                JsonValue::Null,
                Some("captcha challenge served"),
            ),
            false,
        );
        assert_eq!(blocked, ToolOutcome::Blocked);
        // A failed transport is never zero results.
        assert!(!blocked.reached_source());
    }

    #[test]
    fn a_synthesized_timeout_is_a_failure_with_a_tool_timeout_reason() {
        let timed_out = result(
            "firecrawl_mojeek_search",
            "failed",
            JsonValue::Null,
            Some("tool execution timed out after 20s"),
        );
        assert_eq!(tool_outcome(&timed_out, true), ToolOutcome::Failed);
        assert_eq!(
            failure_reason(timed_out.error.as_deref().unwrap(), true),
            "tool_timeout"
        );
        assert_eq!(
            cache_ttl_for(
                definition("firecrawl_mojeek_search"),
                &tool_outcome(&timed_out, true)
            ),
            None
        );
    }

    #[test]
    fn cancelled_and_cache_modes_are_not_remote_requests() {
        let cancelled = result("crtsh_certificates", "cancelled", JsonValue::Null, None);
        assert_eq!(tool_outcome(&cancelled, false), ToolOutcome::Cancelled);
        let cached = ToolResult {
            cached: true,
            ..result(
                "crtsh_certificates",
                "completed",
                json!({"results": [1, 2]}),
                None,
            )
        };
        assert_eq!(tool_outcome(&cached, false), ToolOutcome::CompletedNonEmpty);
        // No executor counter yet: zero remote attempts are reported, not guessed.
        assert_eq!(remote_attempts(&cached), 0);
    }

    #[test]
    fn tool_timeout_prefers_the_catalog_timeout() {
        let crtsh = definition("crtsh_certificates");
        assert_eq!(
            tool_timeout(crtsh),
            Duration::from_secs(crtsh.timeout_seconds)
        );
        let mut no_timeout = crtsh.clone();
        no_timeout.timeout_seconds = 0;
        assert_eq!(tool_timeout(&no_timeout), ToolRunner::REQUEST_TIMEOUT);
        // A catalog timeout is honoured, never clamped down to the shared
        // fallback: the flat 30s runner clamp was the defect that failed large
        // SERPs and slow rendering (spec §9 fix 6).
        let serp = definition("firecrawl_google_search");
        assert!(serp.timeout_seconds > ToolRunner::REQUEST_TIMEOUT.as_secs());
        assert_eq!(
            tool_timeout(serp),
            Duration::from_secs(serp.timeout_seconds)
        );
        // The request timeout still leaves room under the turn's outer deadline.
        assert!(
            tool_timeout(serp)
                < Duration::from_secs(crate::provider::DEFAULT_MAX_TURN_SECONDS as u64)
        );
    }

    #[test]
    fn named_engine_identity_is_separate_from_the_transport_provider() {
        assert_eq!(named_engine("firecrawl_google_search"), Some("google"));
        assert_eq!(named_engine("firecrawl_yandex_search"), Some("yandex"));
        assert_eq!(named_engine("firecrawl_mojeek_search"), Some("mojeek"));
        assert_eq!(named_engine("sociavault_google_search"), None);
        assert!(is_named_serp("firecrawl_google_search"));
        assert!(!is_named_serp("firecrawl_search"));
        assert_eq!(SerpCacheClass::VerifiedZero.cache_seconds(), Some(120));
        assert_eq!(SerpCacheClass::Failure.cache_seconds(), None);
    }

    #[test]
    fn attribution_is_explicit_never_inferred_from_the_prompt() {
        assert_eq!(ToolAttribution::recon().trigger, Trigger::ReconPrompt);
        assert_eq!(ToolAttribution::manual().trigger, Trigger::ManualTool);
        assert_eq!(ToolAttribution::recon().app, "recon");
        assert_eq!(ToolAttribution::manual().app, "cli");
        let bound = ToolAttribution::recon()
            .with_call_id("call-1")
            .with_mode("verify");
        assert_eq!(bound.call_id, "call-1");
        assert_eq!(bound.mode, "verify");
    }

    #[test]
    fn invocation_id_is_stable_so_replay_cannot_double_count() {
        assert_eq!(
            ToolRunner::invocation_id("crtsh_certificates", "key", "call-9"),
            "call-9"
        );
        assert_eq!(
            ToolRunner::invocation_id("crtsh_certificates", "key", ""),
            ToolRunner::invocation_id("crtsh_certificates", "key", "")
        );
        assert_ne!(
            ToolRunner::invocation_id("crtsh_certificates", "key-a", ""),
            ToolRunner::invocation_id("crtsh_certificates", "key-b", "")
        );
    }

    #[tokio::test]
    async fn test_tool_runner_cache_hit() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        let store = Store::open(temp.path()).unwrap();
        let runner = ToolRunner::new(
            temp.path(),
            SettingsFile::default(),
            ProviderKeys::default(),
        );

        let fake_res = ToolResult {
            tool_id: "crtsh_certificates".into(),
            inputs: json!({"domain": "example.com"}),
            status: "completed".into(),
            source_url: "https://crt.sh".into(),
            retrieved_at: Utc::now().to_rfc3339(),
            observations: json!({"test": true}),
            raw: "{}".into(),
            error: None,
            cached: false,
            truncated: false,
            credits_charged: 0,
            credits_reported: None,
        };
        let key = osint::cache_identity("crtsh_certificates", &json!({"domain": "example.com"}));
        store.cache_put(&key, &fake_res, 3600).unwrap();

        let cancel = Arc::new(AtomicBool::new(false));
        let res = runner
            .execute_tool(
                "crtsh_certificates",
                json!({"domain": "example.com"}),
                false,
                &cancel,
            )
            .await
            .unwrap();

        assert!(res.cached);
        assert_eq!(res.status, "completed");
    }

    #[tokio::test]
    async fn test_tool_runner_concurrency_and_dedup() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        let runner = ToolRunner::new(
            temp.path(),
            SettingsFile::default(),
            ProviderKeys::default(),
        );

        // Both request the same cached result or run
        let temp_path = temp.path().to_path_buf();
        let store = Store::open(&temp_path).unwrap();
        let fake_res = ToolResult {
            tool_id: "crtsh_certificates".into(),
            inputs: json!({"domain": "dedup.com"}),
            status: "completed".into(),
            source_url: "https://crt.sh".into(),
            retrieved_at: Utc::now().to_rfc3339(),
            observations: json!({"dedup": true}),
            raw: "{}".into(),
            error: None,
            cached: false,
            truncated: false,
            credits_charged: 0,
            credits_reported: None,
        };
        let key = osint::cache_identity("crtsh_certificates", &json!({"domain": "dedup.com"}));
        store.cache_put(&key, &fake_res, 3600).unwrap();

        let runner2 = runner.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let c1 = cancel.clone();
        let c2 = cancel.clone();

        let (res1, res2) = tokio::join!(
            runner.execute_tool(
                "crtsh_certificates",
                json!({"domain": "dedup.com"}),
                false,
                &c1
            ),
            runner2.execute_tool(
                "crtsh_certificates",
                json!({"domain": "dedup.com"}),
                false,
                &c2
            )
        );

        assert!(res1.unwrap().cached);
        assert!(res2.unwrap().cached);
    }
}
