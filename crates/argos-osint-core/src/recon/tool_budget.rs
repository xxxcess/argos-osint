//! Budgeted batch execution for legacy Recon turns.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{anyhow, Result};
use chrono::Utc;
use serde_json::Value;

use crate::osint::tool_runner::{ToolAttribution, ToolRunner};
use crate::osint::{self, CreditHold, ToolResult};
use crate::telemetry::Trigger;

use super::backend::ReconStore;
use super::{BudgetedOutcome, PlanCall, Run};

impl ToolAttribution {
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
}

impl ToolRunner {
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
        let store = ReconStore::open(&self.db_path).await?;
        let mut affordable = Vec::new();
        let mut skipped = Vec::new();
        let mut holds: Vec<(String, CreditHold)> = Vec::new();

        for call in calls {
            if cancel.load(Ordering::Relaxed) {
                return Err(anyhow!("cancelled"));
            }
            if !store.tool_enabled(&call.tool_id).await? {
                skipped.push(format!("{} — disabled in the catalog", call.tool_id));
                continue;
            }
            if store
                .inflight_duplicate(&call.tool_id, &call.arguments)
                .await?
            {
                skipped.push(format!(
                    "{} — a duplicate call is already queued or running",
                    call.tool_id
                ));
                continue;
            }
            let cache_key = osint::cache_identity(&call.tool_id, &call.arguments);
            let cached = store.cache_get(&cache_key).await?.is_some();
            if let Some((provider_name, cost)) =
                limits.configured_cost_for(&call.tool_id, &call.arguments)
            {
                if !cached && cost > 0 {
                    match store.reserve_credits(provider_name, cost, limits).await? {
                        Some(hold) => {
                            holds.push((format!("{}:{}", call.tool_id, call.arguments), hold));
                        }
                        None => {
                            let available = store.credits_available(provider_name, limits).await?;
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
                self.release_holds(&holds).await?;
                return Err(anyhow!("cancelled"));
            }
            let store = ReconStore::open(&self.db_path).await?;
            for value in call
                .arguments
                .as_object()
                .into_iter()
                .flat_map(|m| m.values())
                .filter_map(Value::as_str)
            {
                for (kind, canonical) in crate::recon::explicit_entities(value) {
                    let _ = store
                        .link_entity(&run.thread_id, &kind, &canonical, None)
                        .await;
                }
            }
            let call_id = store
                .queue_call(
                    &call.tool_id,
                    &call.arguments,
                    "recon",
                    Some(&run.id),
                    Some(&run.thread_id),
                    Some(&run.turn_id),
                )
                .await?;
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
            let store = ReconStore::open(&self.db_path).await?;
            store.finish_call(&call_id, &result).await?;
            if let Some(hosts) = result
                .observations
                .get("hostnames")
                .and_then(Value::as_array)
            {
                for host in hosts.iter().filter_map(Value::as_str).take(100) {
                    for (kind, canonical) in crate::recon::explicit_entities(host) {
                        let _ = store
                            .link_entity(&run.thread_id, &kind, &canonical, Some(&call_id))
                            .await;
                    }
                }
            }
            results.push((call_id, result));
        }

        self.settle_holds(&results, holds).await?;
        Ok(BudgetedOutcome { results, skipped })
    }

    async fn release_holds(&self, holds: &[(String, CreditHold)]) -> Result<()> {
        let store = ReconStore::open(&self.db_path).await?;
        for (_, hold) in holds {
            store.release_credits(hold).await?;
        }
        Ok(())
    }

    pub async fn settle_holds(
        &self,
        results: &[(String, ToolResult)],
        mut holds: Vec<(String, CreditHold)>,
    ) -> Result<()> {
        let store = ReconStore::open(&self.db_path).await?;
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
                store.reconcile_credits(&hold, actual).await?;
            } else {
                store.release_credits(&hold).await?;
            }
        }
        for (_, hold) in holds {
            store.release_credits(&hold).await?;
        }
        Ok(())
    }
}
