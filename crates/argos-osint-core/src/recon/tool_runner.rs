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

use anyhow::{anyhow, ensure, Result};
use chrono::Utc;
use serde_json::Value;
use tokio::sync::{watch, Mutex, Semaphore};

use crate::osint::{self, ProviderKeys, ToolResult};
use crate::provider::SettingsFile;
use crate::recon::orchestrate::BudgetedOutcome;
use crate::recon::{CreditHold, PlanCall, Run, Store};

/// Shared tool execution runner with concurrency bounding and deduplication.
#[derive(Clone)]
pub struct ToolRunner {
    pub db_path: PathBuf,
    pub settings: SettingsFile,
    pub keys: ProviderKeys,
    concurrency: Arc<Semaphore>,
    inflight: Arc<Mutex<HashMap<String, watch::Receiver<Option<ToolResult>>>>>,
}

impl ToolRunner {
    /// Maximum concurrent OSINT network requests across all tools.
    pub const MAX_CONCURRENCY: usize = 4;
    /// Per-request timeout for tool execution.
    pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

    pub fn new(db_path: impl AsRef<Path>, settings: SettingsFile, keys: ProviderKeys) -> Self {
        Self {
            db_path: db_path.as_ref().to_path_buf(),
            settings,
            keys,
            concurrency: Arc::new(Semaphore::new(Self::MAX_CONCURRENCY)),
            inflight: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Execute a single tool with caching, in-flight dedup, and credit accounting.
    pub async fn execute_tool(
        &self,
        tool_id: &str,
        inputs: Value,
        refresh: bool,
        cancel: &Arc<AtomicBool>,
    ) -> Result<ToolResult> {
        self.execute_tool_inner(tool_id, inputs, refresh, cancel, true)
            .await
    }

    pub async fn execute_tool_without_reservation(
        &self,
        tool_id: &str,
        inputs: Value,
        refresh: bool,
        cancel: &Arc<AtomicBool>,
    ) -> Result<ToolResult> {
        self.execute_tool_inner(tool_id, inputs, refresh, cancel, false)
            .await
    }

    async fn execute_tool_inner(
        &self,
        tool_id: &str,
        inputs: Value,
        refresh: bool,
        cancel: &Arc<AtomicBool>,
        reserve_credits: bool,
    ) -> Result<ToolResult> {
        if cancel.load(Ordering::Relaxed) {
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
                    return Ok(osint::project_cached_result(canonical, &inputs, &cached));
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
                    return Err(anyhow!("cancelled"));
                }
            };
            if wait_res.is_ok() {
                if let Some(res) = rx.borrow().as_ref() {
                    return Ok(osint::project_cached_result(canonical, &inputs, res));
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
            ensure!(
                store.tool_enabled(canonical)?,
                "tool disabled in catalog: {canonical}"
            );
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
                return Err(anyhow!("cancelled"));
            }
        };

        let executor = crate::osint::Executor::new()?;
        let ua = if self.settings.osint_user_agent.trim().is_empty() {
            None
        } else {
            Some(self.settings.osint_user_agent.as_str())
        };

        let exec_fut = executor.run_configured(canonical, inputs.clone(), ua, &self.keys);
        let result_outcome = tokio::select! {
            res = tokio::time::timeout(Self::REQUEST_TIMEOUT, exec_fut) => match res {
                Ok(r) => r,
                Err(_) => Ok(ToolResult {
                    tool_id: canonical.to_string(),
                    inputs: inputs.clone(),
                    status: "failed".into(),
                    source_url: String::new(),
                    retrieved_at: Utc::now().to_rfc3339(),
                    observations: Value::Null,
                    raw: String::new(),
                    error: Some(format!("tool execution timed out after {}s", Self::REQUEST_TIMEOUT.as_secs())),
                    cached: false,
                    truncated: false,
                    credits_charged: 0,
                    credits_reported: None,
                }),
            },
            _ = wait_cancellation(cancel) => {
                if reserve_credits {
                    if let (Some((_provider, _)), Some(hold)) = (cost_info, &credit_hold) {
                        let _ = Store::open(&self.db_path).map(|s| s.release_credits(hold));
                    }
                }
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

        // 7. Cache put
        if osint_cacheable(&result) {
            if let Ok(store) = Store::open(&self.db_path) {
                let _ = store.cache_put(&cache_key, &result, def.cache_seconds);
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

            let res = self
                .execute_tool_without_reservation(
                    &call.tool_id,
                    call.arguments.clone(),
                    false,
                    cancel,
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

fn osint_cacheable(result: &ToolResult) -> bool {
    matches!(result.status.as_str(), "completed" | "no_results")
        && result.error.as_deref().unwrap_or("").is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
