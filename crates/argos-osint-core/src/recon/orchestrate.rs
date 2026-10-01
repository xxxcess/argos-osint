//! One Recon turn: choose a strategy, discover, then spend credits one useful step at a time.
use std::{
    collections::{HashMap, HashSet},
    sync::{atomic::Ordering, Arc},
};

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use super::{
    investigation, AnswerContext, CreditHold, EntityView, HypothesisView, Plan, PlanCall, Run,
    Store,
};
use crate::{
    osint::ToolResult,
    provider::{self, SettingsFile},
    secrets::ProviderSecret,
};
use std::sync::atomic::AtomicBool;

pub async fn execute_budgeted(
    service: &super::Service,
    run: &Run,
    calls: &[PlanCall],
    cancel: &Arc<AtomicBool>,
) -> Result<Vec<(String, ToolResult)>> {
    if calls.is_empty() {
        return Ok(Vec::new());
    }
    let limits = &service.settings.recon_limits;
    let store = Store::open(&service.db_path)?;
    let mut affordable = Vec::new();
    let mut holds: Vec<(String, CreditHold)> = Vec::new();
    for call in calls {
        if cancel.load(Ordering::Relaxed) {
            return Err(anyhow!("cancelled"));
        }
        if !store.tool_enabled(&call.tool_id)? {
            continue;
        }
        if store.inflight_duplicate(&call.tool_id, &call.arguments)? {
            continue;
        }
        let cache_key = format!(
            "{}:v1:{}",
            call.tool_id,
            serde_json::to_string(&call.arguments)?
        );
        let cached = store.cache_get(&cache_key)?.is_some();
        if let Some((provider_name, cost)) = limits.configured_cost(&call.tool_id) {
            if !cached && cost > 0 {
                match store.reserve_credits(provider_name, cost, limits)? {
                    Some(hold) => {
                        holds.push((format!("{}:{}", call.tool_id, call.arguments), hold));
                    }
                    None => continue,
                }
            }
        }
        affordable.push(call.clone());
    }
    drop(store);
    if affordable.is_empty() {
        return Ok(Vec::new());
    }
    let plan = Plan {
        calls: affordable,
        planning_mode: "budgeted".into(),
        ..Plan::default()
    };
    let outcome = service.execute_plan(run, &plan, cancel).await;
    let results = match outcome {
        Ok(results) => results,
        Err(err) => {
            let store = Store::open(&service.db_path)?;
            for (_, hold) in &holds {
                store.release_credits(hold)?;
            }
            return Err(err);
        }
    };
    settle_holds(service, &results, holds)?;
    Ok(results)
}

fn settle_holds(
    service: &super::Service,
    results: &[(String, ToolResult)],
    mut holds: Vec<(String, CreditHold)>,
) -> Result<()> {
    let store = Store::open(&service.db_path)?;
    for (_, result) in results {
        let signature = format!("{}:{}", result.tool_id, result.inputs);
        let Some(index) = holds.iter().position(|(sig, _)| sig == &signature) else {
            continue;
        };
        let (_, hold) = holds.swap_remove(index);
        let spend = matches!(result.status.as_str(), "completed" | "no_results") && !result.cached;
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

pub async fn run_turn(
    service: &super::Service,
    run: &Run,
    question: &str,
    recon_secret: &ProviderSecret,
    synthesis_secret: &ProviderSecret,
    cancel: &Arc<AtomicBool>,
    progress: &mut (impl FnMut(&str) + Send),
) -> Result<()> {
    progress("selecting a strategy");
    let store = Store::open(&service.db_path)?;
    store.set_run(&run.id, "running", "selecting a strategy", None, None)?;
    for (kind, value) in super::explicit_entities(question) {
        store.link_entity(&run.thread_id, &kind, &value, None)?;
    }
    let history = store.list_messages(&run.thread_id)?;
    let opening = !history.iter().any(|message| message.role == "assistant");
    let text_hits = store.recall(question, 8)?;
    let unfamiliar = super::brain_is_thin(&text_hits);
    let mut recalled = store.recon_recall(&store.thread_entities(&run.thread_id)?)?;
    let subject = super::question_subject(question);
    for hit in &text_hits {
        if recalled.iter().any(|item| item.memory_id == hit.memory.id) {
            continue;
        }
        recalled.push(super::RecallInsight {
            memory_id: hit.memory.id.clone(),
            text: hit.memory.text.clone(),
            entity: subject.clone(),
            predicate: "memory".into(),
            updated_at: hit.memory.created_at.clone(),
            evidence_count: 0,
        });
    }
    let prior: Vec<_> = store
        .calls_for_thread(&run.thread_id)?
        .into_iter()
        .rev()
        .take(12)
        .filter_map(|call| call.result.map(|result| (call.id, result)))
        .collect();
    let useful = prior.iter().any(|(_, result)| result.status == "completed");
    let previous = store.latest_strategy_kind(&run.thread_id)?.unwrap_or_default();
    let fallback = investigation::select_strategy(question, opening, useful, unfamiliar);
    let choice = match model_strategy(
        recon_secret,
        StrategyPrompt {
            question,
            opening,
            useful,
            unfamiliar,
            previous: &previous,
            settings: &service.settings,
        },
        cancel,
    )
    .await
    {
        Ok(Some(choice)) => choice,
        Ok(None) => fallback,
        Err(err) if cancelled(&err) => return Err(err),
        Err(_) => fallback,
    };
    let change = investigation::strategy_change_reason(&previous, &choice);
    store.record_strategy(
        &run.thread_id,
        &run.id,
        &choice.kind,
        &choice.rationale,
        if previous.is_empty() { None } else { Some(previous.as_str()) },
        change.as_deref(),
    )?;
    let known: Vec<String> = recalled.iter().map(|item| item.text.clone()).collect();
    let frame = investigation::investigation_frame(question, &known);
    let mut plan = Plan {
        objective: frame.objective.clone(),
        strategy: choice.kind.clone(),
        strategy_rationale: choice.rationale.clone(),
        strategy_change: change.unwrap_or_default(),
        planning_mode: "strategy".into(),
        stop_condition: "Stop when the question is answered, an identity needs clarification, no useful lookup remains, or a budget is reached.".into(),
        ..Plan::default()
    };
    store.set_run(&run.id, "running", "selecting a strategy", Some(&plan), None)?;
    drop(store);

    let mut results = prior;
    let mut discovery_hits = Vec::new();
    if opening {
        progress("searching the web");
        let (note, complete, searches, hits) = opening_discovery(
            service,
            run,
            question,
            &choice.kind,
            recon_secret,
            cancel,
        )
        .await?;
        plan.discovery_note = note;
        let store = Store::open(&service.db_path)?;
        store.save_discovery(
            &run.thread_id,
            if complete { "complete" } else { "incomplete" },
            &plan.discovery_note,
            &frame,
        )?;
        drop(store);
        for (id, result) in &searches {
            plan.calls.push(search_call(id, result));
        }
        results.extend(searches);
        discovery_hits = hits;
    }

    progress("selecting entities");
    let mut entities = if opening {
        investigation::select_entities(question, &discovery_hits)
    } else {
        let mut loaded = Store::open(&service.db_path)?.load_investigation_entities(&run.thread_id)?;
        investigation::focus_entities(question, &mut loaded);
        loaded
    };
    if entities.iter().all(|entity| !entity.selected) && !opening {
        entities = investigation::select_entities(question, &hits_from_results(&results));
    }
    Store::open(&service.db_path)?.save_investigation_entities(&run.thread_id, &entities)?;
    plan.selected_entities = entity_views(&entities);

    let mut hypotheses = if choice.kind == investigation::HYPOTHESIS {
        Some(investigation::draft_hypotheses(question))
    } else {
        None
    };
    let gaps = investigation::gaps_for(question, &choice.kind, &entities, hypotheses.as_ref());
    plan.gaps = gaps.iter().map(|gap| gap.question.clone()).collect();
    {
        let store = Store::open(&service.db_path)?;
        store.save_gaps(&run.thread_id, &run.id, &gaps)?;
        if let Some(record) = &hypotheses {
            store.save_hypotheses(&run.thread_id, &run.id, record)?;
        }
    }

    let enabled = enabled_tools(&service.db_path)?;
    let mut already = signatures(&results);
    let cached = HashSet::new();
    let costs = cost_map(&service.settings);
    let limits = &service.settings.recon_limits;
    let missing = missing_keys(service);
    let max_calls = usize::from(run.max_calls);
    let turn_hunter_cap = if opening {
        usize::from(limits.opening_hunter_calls)
    } else {
        4
    };
    let turn_sociavault_cap = if opening {
        usize::from(limits.opening_sociavault_calls)
    } else {
        4
    };
    let mut hunter_cap = turn_hunter_cap;
    let mut sociavault_cap = turn_sociavault_cap;
    let mut executed_actions = Vec::new();
    let mut isolation_skipped = Vec::new();
    if opening {
        // Tool isolation: the catalog tools that would otherwise only be listed as
        // additional context run as plan steps with subject-derived inputs. It never
        // calls Firecrawl.
        progress("isolating tools");
        let mut used: HashSet<String> = results
            .iter()
            .map(|(_, result)| result.tool_id.clone())
            .collect();
        used.extend(["firecrawl_search".to_string(), "firecrawl_scrape".to_string()]);
        let suggestions = investigation::additional_tools(question, &used, &enabled);
        let credits = credit_map(service)?;
        let isolation = investigation::isolate_tools(
            &suggestions,
            &investigation::SelectionInput {
                question,
                strategy: &choice.kind,
                opening,
                entities: &entities,
                gaps: &gaps,
                enabled: &enabled,
                already: &already,
                cached: &cached,
                hunter_cap,
                sociavault_cap,
                credits_left: &credits,
                costs: &costs,
                hits: &discovery_hits,
            },
            &missing,
            ISOLATED_TOOLS,
        );
        let wave = run_wave(
            service,
            run,
            &isolation.actions,
            Wave {
                plan: &mut plan,
                already: &mut already,
                executed: &mut executed_actions,
                results: &mut results,
                max_calls,
            },
            cancel,
        )
        .await?;
        hunter_cap = hunter_cap.saturating_sub(count_tools(&wave.ran, |id| id.starts_with("hunter_")));
        sociavault_cap =
            sociavault_cap.saturating_sub(count_tools(&wave.ran, |id| id == "sociavault_profile"));
        plan.isolated_tools = isolation_lines(&wave, &isolation.skipped);
        isolation_skipped = isolation.skipped;
    }
    let credits = credit_map(service)?;
    let mut ranked = investigation::rank_actions(&investigation::SelectionInput {
        question,
        strategy: &choice.kind,
        opening,
        entities: &entities,
        gaps: &gaps,
        enabled: &enabled,
        already: &already,
        cached: &cached,
        hunter_cap,
        sociavault_cap,
        credits_left: &credits,
        costs: &costs,
        hits: &discovery_hits,
    });
    note_cache(service, &mut ranked.actions)?;
    if ranked.considered == 0 && !enabled.is_empty() {
        plan.unresolved_inputs
            .push("No enabled tool was available to consider.".into());
    }
    let mut chosen = match model_subset(recon_secret, &ranked.actions, cancel).await {
        Ok(Some(subset)) => subset,
        Ok(None) => ranked.actions.clone(),
        Err(err) if cancelled(&err) => return Err(err),
        Err(_) => ranked.actions.clone(),
    };
    let mut deferred = ranked.deferred;
    let remaining = max_calls.saturating_sub(plan.calls.len());
    if choice.kind == investigation::ADAPTIVE {
        let mut rounds = 0usize;
        let mut blocked = HashSet::new();
        while rounds < usize::from(run.max_rounds) {
            if cancel.load(Ordering::Relaxed) {
                return Err(anyhow!("cancelled"));
            }
            let room = max_calls.saturating_sub(plan.calls.len());
            if room == 0 {
                plan.unresolved_inputs.push("Call budget reached.".into());
                break;
            }
            chosen.retain(|action| !blocked.contains(&action.signature()));
            let (free, scarce) = investigation::adaptive_step(&chosen);
            if free.is_empty() && scarce.is_none() {
                break;
            }
            let mut wave = Vec::new();
            if let Some(scarce_action) = scarce {
                let free_room = room.saturating_sub(1);
                if room == 1 {
                    wave.push(scarce_action);
                } else {
                    wave.extend(free.into_iter().take(free_room));
                    wave.push(scarce_action);
                }
            } else {
                wave.extend(free.into_iter().take(room));
            }
            wave.truncate(room);
            if wave.is_empty() {
                break;
            }
            progress("running lookups");
            let calls = wave
                .iter()
                .enumerate()
                .map(|(index, action)| action_call(action, plan.calls.len() + index))
                .collect::<Vec<_>>();
            let executed = execute_budgeted(service, run, &calls, cancel).await?;
            let ran: HashSet<String> = executed
                .iter()
                .map(|(_, result)| format!("{}:{}", result.tool_id, result.inputs))
                .collect();
            for (action, call) in wave.iter().zip(&calls) {
                if ran.contains(&action.signature()) {
                    plan.calls.push(call.clone());
                    already.insert(action.signature());
                    executed_actions.push(action.clone());
                } else {
                    blocked.insert(action.signature());
                    deferred.push(action.clone());
                }
            }
            let spent = wave.iter().any(|action| action.spends());
            results.extend(executed);
            if spent {
                progress("checking the result");
            }
            rounds += 1;
            let credits = credit_map(service)?;
            let mut ranked = investigation::rank_actions(&investigation::SelectionInput {
                question,
                strategy: &choice.kind,
                opening,
                entities: &entities,
                gaps: &gaps,
                enabled: &enabled,
                already: &already,
                cached: &cached,
                hunter_cap,
                sociavault_cap,
                credits_left: &credits,
                costs: &costs,
                hits: &discovery_hits,
            });
            note_cache(service, &mut ranked.actions)?;
            chosen = ranked.actions;
            deferred.extend(ranked.deferred);
        }
    } else {
        let (execute, later) = if choice.kind == investigation::HYPOTHESIS {
            investigation::hypothesis_batch(&chosen)
        } else {
            investigation::discovery_batch(&chosen)
        };
        deferred.extend(later);
        let execute: Vec<_> = execute.into_iter().take(remaining).collect();
        if !execute.is_empty() {
            progress("running lookups");
            let calls = execute
                .iter()
                .enumerate()
                .map(|(index, action)| action_call(action, plan.calls.len() + index))
                .collect::<Vec<_>>();
            let executed = execute_budgeted(service, run, &calls, cancel).await?;
            let ran: HashSet<String> = executed
                .iter()
                .map(|(_, result)| format!("{}:{}", result.tool_id, result.inputs))
                .collect();
            for (action, call) in execute.iter().zip(&calls) {
                if ran.contains(&action.signature()) {
                    plan.calls.push(call.clone());
                    executed_actions.push(action.clone());
                } else {
                    deferred.push(action.clone());
                }
            }
            results.extend(executed);
        }
    }

    if let Some(record) = hypotheses.as_mut() {
        let notes = evidence_notes(&results);
        investigation::classify_hypothesis(record, &notes);
        plan.hypotheses = vec![hypothesis_view(record)];
        Store::open(&service.db_path)?.save_hypotheses(&run.thread_id, &run.id, record)?;
    }
    plan.deferred = deferred
        .iter()
        .take(8)
        .map(|action| format!("{} — {}", action.tool_id, action.purpose))
        .collect();
    let used_tools: HashSet<String> = results.iter().map(|(_, result)| result.tool_id.clone()).collect();
    let fallback_tools = investigation::additional_tools(question, &used_tools, &enabled);
    let assessment = match model_assessment(
        recon_secret,
        question,
        &results,
        &fallback_tools,
        &enabled,
        cancel,
    )
    .await
    {
        Ok(Some(assessment)) => assessment,
        Ok(None) => investigation::AnswerAssessment {
            answered: false,
            tools: fallback_tools,
        },
        Err(err) if cancelled(&err) => return Err(err),
        Err(_) => investigation::AnswerAssessment {
            answered: false,
            tools: fallback_tools,
        },
    };
    plan.question_answered = assessment.answered;
    plan.additional_tools = Vec::new();
    if !assessment.answered {
        // Suggested tools are executed, not only listed. Whatever still cannot run
        // stays under Additional context with its reason.
        let mut skipped = isolation_skipped;
        let room = max_calls.saturating_sub(plan.calls.len());
        if room > 0 && !assessment.tools.is_empty() {
            progress("running suggested tools");
            hunter_cap = turn_hunter_cap
                .saturating_sub(count_tools(&executed_actions, |id| id.starts_with("hunter_")));
            sociavault_cap = turn_sociavault_cap
                .saturating_sub(count_tools(&executed_actions, |id| id == "sociavault_profile"));
            let credits = credit_map(service)?;
            let isolation = investigation::isolate_tools(
                &assessment.tools,
                &investigation::SelectionInput {
                    question,
                    strategy: &choice.kind,
                    opening,
                    entities: &entities,
                    gaps: &gaps,
                    enabled: &enabled,
                    already: &already,
                    cached: &cached,
                    hunter_cap,
                    sociavault_cap,
                    credits_left: &credits,
                    costs: &costs,
                    hits: &discovery_hits,
                },
                &missing,
                room.min(ISOLATED_TOOLS),
            );
            let wave = run_wave(
                service,
                run,
                &isolation.actions,
                Wave {
                    plan: &mut plan,
                    already: &mut already,
                    executed: &mut executed_actions,
                    results: &mut results,
                    max_calls,
                },
                cancel,
            )
            .await?;
            for action in &wave.blocked {
                skipped.push(format!(
                    "{} — not run: the call budget, credit budget, or a duplicate call stopped it.",
                    action.tool_id
                ));
            }
            skipped.extend(isolation.skipped);
        }
        let ran: HashSet<String> = results
            .iter()
            .map(|(_, result)| result.tool_id.clone())
            .collect();
        for tool in &assessment.tools {
            if ran.contains(&tool.tool_id) {
                continue;
            }
            let prefix = format!("{} — ", tool.tool_id);
            let line = skipped
                .iter()
                .find(|line| line.starts_with(&prefix))
                .cloned()
                .unwrap_or_else(|| format!("{} — {}", tool.tool_id, tool.reason));
            plan.additional_tools.push(line);
        }
    }
    {
        let store = Store::open(&service.db_path)?;
        let mut seen_deferred = HashSet::new();
        deferred.retain(|action| seen_deferred.insert(action.signature()));
        store.save_actions(&run.thread_id, &run.id, &executed_actions, "executed")?;
        store.save_actions(&run.thread_id, &run.id, &deferred, "deferred")?;
        store.set_run(&run.id, "running", "synthesizing", Some(&plan), None)?;
    }
    service
        .finish_answer(
            AnswerContext {
                run,
                question,
                plan: &plan,
                results: &results,
                recalled: &recalled,
                max_calls,
                opening,
                synthesis_secret,
                cancel,
            },
            progress,
        )
        .await
}

async fn opening_discovery(
    service: &super::Service,
    run: &Run,
    question: &str,
    strategy: &str,
    secret: &ProviderSecret,
    cancel: &Arc<AtomicBool>,
) -> Result<(String, bool, Vec<(String, ToolResult)>, Vec<investigation::SearchHit>)> {
    let store = Store::open(&service.db_path)?;
    let enabled = store.tool_enabled("firecrawl_search")?;
    drop(store);
    if !enabled || service.provider_keys().firecrawl.trim().is_empty() {
        return Ok((
            "Firecrawl search is unavailable, so discovery is not complete.".into(),
            false,
            Vec::new(),
            Vec::new(),
        ));
    }
    let cost = service
        .settings
        .recon_limits
        .configured_cost("firecrawl_search")
        .map(|(_, credits)| credits)
        .unwrap_or(2);
    let available = Store::open(&service.db_path)?
        .credits_available("firecrawl", &service.settings.recon_limits)?;
    if cost > 0 && available < cost.saturating_mul(2) {
        return Ok((
            "The Firecrawl credit budget cannot cover two opening searches, so discovery is not complete.".into(),
            false,
            Vec::new(),
            Vec::new(),
        ));
    }
    let mut queries = investigation::complementary_queries(question, strategy);
    match model_queries(secret, question, strategy, &queries, cancel).await {
        Ok(Some(replacement)) => queries = replacement,
        Err(err) if cancelled(&err) => return Err(err),
        _ => {}
    }
    let calls = vec![
        PlanCall {
            step_id: "search-identity".into(),
            tool_id: "firecrawl_search".into(),
            arguments: json!({"query": queries[0].query, "limit": 5}),
            reason: queries[0].angle.clone(),
            gap: "gap-identity".into(),
            expected: "Authoritative identifiers for the subject.".into(),
            credit_cost: cost,
            ..PlanCall::default()
        },
        PlanCall {
            step_id: "search-investigative".into(),
            tool_id: "firecrawl_search".into(),
            arguments: json!({"query": queries[1].query, "limit": 5}),
            reason: queries[1].angle.clone(),
            gap: "gap-question".into(),
            expected: "Evidence on the requested relationship, activity, or competing explanation.".into(),
            credit_cost: cost,
            ..PlanCall::default()
        },
    ];
    let executed = execute_budgeted(service, run, &calls, cancel).await?;
    let hits = hits_from_searches(&queries, &executed);
    let ok = |result: &ToolResult| matches!(result.status.as_str(), "completed" | "no_results");
    let both = executed.len() == 2 && executed.iter().all(|(_, result)| ok(result));
    let note = if both {
        "Two complementary Firecrawl searches ran: one for identity and one for the investigative question.".into()
    } else if executed.is_empty() {
        "Firecrawl search did not return, so discovery is not complete.".into()
    } else {
        "One opening Firecrawl search did not succeed, so discovery is not complete.".into()
    };
    Ok((note, both, executed, hits))
}

fn hits_from_searches(
    queries: &[investigation::DiscoveryQuery; 2],
    executed: &[(String, ToolResult)],
) -> Vec<investigation::SearchHit> {
    let mut hits = Vec::new();
    for (id, result) in executed {
        if result.tool_id != "firecrawl_search" || result.status != "completed" {
            continue;
        }
        let query = result
            .inputs
            .get("query")
            .and_then(Value::as_str)
            .unwrap_or("");
        let role = queries
            .iter()
            .find(|item| item.query == query)
            .map(|item| item.role.as_str())
            .unwrap_or("investigative");
        let Some(rows) = result.observations.get("results").and_then(Value::as_array) else {
            continue;
        };
        for row in rows {
            hits.push(investigation::SearchHit {
                evidence_id: id.clone(),
                title: row.get("title").and_then(Value::as_str).unwrap_or("").into(),
                url: row.get("url").and_then(Value::as_str).unwrap_or("").into(),
                snippet: row.get("snippet").and_then(Value::as_str).unwrap_or("").into(),
                retrieved_at: result.retrieved_at.clone(),
                query_role: role.into(),
            });
        }
    }
    investigation::dedupe_hits(hits)
}

fn hits_from_results(results: &[(String, ToolResult)]) -> Vec<investigation::SearchHit> {
    let queries = [
        investigation::DiscoveryQuery {
            role: "identity".into(),
            query: String::new(),
            angle: String::new(),
        },
        investigation::DiscoveryQuery {
            role: "investigative".into(),
            query: String::new(),
            angle: String::new(),
        },
    ];
    hits_from_searches(&queries, results)
}

fn search_call(_id: &str, result: &ToolResult) -> PlanCall {
    PlanCall {
        step_id: format!("search-{}", result.inputs.get("query").and_then(Value::as_str).unwrap_or("web")),
        tool_id: result.tool_id.clone(),
        arguments: result.inputs.clone(),
        reason: "Opening Firecrawl discovery search.".into(),
        credit_cost: result.credits_reported.unwrap_or(0),
        ..PlanCall::default()
    }
}

fn action_call(action: &investigation::ProposedAction, index: usize) -> PlanCall {
    PlanCall {
        step_id: format!("act-{index}"),
        tool_id: action.tool_id.clone(),
        arguments: action.arguments.clone(),
        reason: action.purpose.clone(),
        gap: action.gap_id.clone(),
        expected: action.expected.clone(),
        credit_cost: action.credit_cost,
        evidence_ids: action.evidence_ids.clone(),
        ..PlanCall::default()
    }
}

/// Tools chosen by the isolation step, and the most suggested tools run in one follow-up wave.
const ISOLATED_TOOLS: usize = 3;

struct Wave<'a> {
    plan: &'a mut Plan,
    already: &'a mut HashSet<String>,
    executed: &'a mut Vec<investigation::ProposedAction>,
    results: &'a mut Vec<(String, ToolResult)>,
    max_calls: usize,
}

struct WaveOutcome {
    ran: Vec<investigation::ProposedAction>,
    blocked: Vec<investigation::ProposedAction>,
}

/// Runs proposed actions as plan steps through the budgeted executor, so credits,
/// the call budget, disabled tools, and duplicate calls are handled as for any step.
async fn run_wave(
    service: &super::Service,
    run: &Run,
    actions: &[investigation::ProposedAction],
    wave: Wave<'_>,
    cancel: &Arc<AtomicBool>,
) -> Result<WaveOutcome> {
    let room = wave.max_calls.saturating_sub(wave.plan.calls.len());
    let (batch, over) = actions.split_at(actions.len().min(room));
    let mut outcome = WaveOutcome {
        ran: Vec::new(),
        blocked: over.to_vec(),
    };
    if batch.is_empty() {
        return Ok(outcome);
    }
    let calls = batch
        .iter()
        .enumerate()
        .map(|(index, action)| PlanCall {
            step_id: format!("isolate-{}", wave.plan.calls.len() + index),
            ..action_call(action, wave.plan.calls.len() + index)
        })
        .collect::<Vec<_>>();
    let executed = execute_budgeted(service, run, &calls, cancel).await?;
    let ran: HashSet<String> = executed
        .iter()
        .map(|(_, result)| format!("{}:{}", result.tool_id, result.inputs))
        .collect();
    for (action, call) in batch.iter().zip(&calls) {
        if ran.contains(&action.signature()) {
            wave.plan.calls.push(call.clone());
            wave.already.insert(action.signature());
            wave.executed.push(action.clone());
            outcome.ran.push(action.clone());
        } else {
            outcome.blocked.push(action.clone());
        }
    }
    wave.results.extend(executed);
    Ok(outcome)
}

fn count_tools(actions: &[investigation::ProposedAction], wanted: impl Fn(&str) -> bool) -> usize {
    actions.iter().filter(|action| wanted(&action.tool_id)).count()
}

fn isolation_lines(wave: &WaveOutcome, skipped: &[String]) -> Vec<String> {
    let mut lines: Vec<String> = wave
        .ran
        .iter()
        .map(|action| format!("{} — ran with {}", action.tool_id, action.arguments))
        .collect();
    lines.extend(wave.blocked.iter().map(|action| {
        format!(
            "{} — not run: the call budget, credit budget, or a duplicate call stopped it.",
            action.tool_id
        )
    }));
    lines.extend(skipped.iter().cloned());
    lines
}

fn missing_keys(service: &super::Service) -> HashSet<String> {
    let keys = service.provider_keys();
    [
        ("firecrawl", keys.firecrawl),
        ("hunter", keys.hunter),
        ("sociavault", keys.sociavault),
    ]
    .into_iter()
    .filter(|(_, key)| key.trim().is_empty())
    .map(|(provider_name, _)| provider_name.to_string())
    .collect()
}

fn signatures(results: &[(String, ToolResult)]) -> HashSet<String> {
    results
        .iter()
        .map(|(_, result)| format!("{}:{}", result.tool_id, result.inputs))
        .collect()
}

fn evidence_notes(results: &[(String, ToolResult)]) -> Vec<(String, String)> {
    results
        .iter()
        .map(|(id, result)| (id.clone(), result.observations.to_string()))
        .collect()
}

fn entity_views(entities: &[investigation::SelectedEntity]) -> Vec<EntityView> {
    entities
        .iter()
        .filter(|entity| entity.selected)
        .map(|entity| EntityView {
            name: entity.canonical_name.clone(),
            entity_type: entity.entity_type.clone(),
            identifiers: entity
                .identifiers
                .iter()
                .map(|identifier| format!("{} {}", identifier.kind, identifier.value))
                .collect::<Vec<_>>()
                .join(", "),
            certainty: entity.certainty.clone(),
            why: entity.why.clone(),
        })
        .collect()
}

fn hypothesis_view(record: &investigation::HypothesisRecord) -> HypothesisView {
    HypothesisView {
        question: record.question.clone(),
        status: record.status.clone(),
        lines: record
            .alternatives
            .iter()
            .map(|alternative| {
                let label = if !alternative.supporting.is_empty() && alternative.contradicting.is_empty()
                {
                    "supported"
                } else if !alternative.contradicting.is_empty() && alternative.supporting.is_empty() {
                    "contradicted"
                } else if alternative.supporting.is_empty() && alternative.contradicting.is_empty() {
                    "missing"
                } else {
                    "mixed"
                };
                format!(
                    "{} — {label}; supporting {}, contradicting {}, missing {}",
                    alternative.statement,
                    alternative.supporting.len(),
                    alternative.contradicting.len(),
                    alternative.missing.len()
                )
            })
            .collect(),
    }
}

fn enabled_tools(db: &std::path::Path) -> Result<HashSet<String>> {
    let store = Store::open(db)?;
    let mut ids = HashSet::new();
    for tool in crate::osint::registry() {
        if store.tool_enabled(tool.id)? {
            ids.insert(tool.id.to_string());
        }
    }
    Ok(ids)
}

fn note_cache(
    service: &super::Service,
    actions: &mut [investigation::ProposedAction],
) -> Result<()> {
    let store = Store::open(&service.db_path)?;
    for action in actions.iter_mut() {
        let key = format!(
            "{}:v1:{}",
            action.tool_id,
            serde_json::to_string(&action.arguments)?
        );
        if store.cache_get(&key)?.is_some() {
            action.cache_available = true;
            action.credit_cost = 0;
            if !action.rank_reason.contains("cached") {
                action
                    .rank_reason
                    .push_str(" A cached result is available.");
            }
        }
    }
    actions.sort_by_key(|action| {
        (
            u8::from(!action.cache_available),
            u8::from(action.spends()),
            action.credit_cost,
        )
    });
    Ok(())
}

fn credit_map(service: &super::Service) -> Result<HashMap<String, u32>> {
    let store = Store::open(&service.db_path)?;
    let limits = &service.settings.recon_limits;
    let mut map = HashMap::new();
    for provider_name in ["firecrawl", "hunter", "sociavault"] {
        map.insert(
            provider_name.into(),
            store.credits_available(provider_name, limits)?,
        );
    }
    Ok(map)
}

fn cost_map(settings: &SettingsFile) -> HashMap<String, u32> {
    let mut map = HashMap::new();
    for tool in crate::osint::registry() {
        if let Some((_, cost)) = settings.recon_limits.configured_cost(tool.id) {
            map.insert(tool.id.to_string(), cost);
        }
    }
    map
}

fn cancelled(err: &anyhow::Error) -> bool {
    err.to_string() == "cancelled"
}

struct StrategyPrompt<'a> {
    question: &'a str,
    opening: bool,
    useful: bool,
    unfamiliar: bool,
    previous: &'a str,
    settings: &'a SettingsFile,
}

async fn model_strategy(
    secret: &ProviderSecret,
    prompt: StrategyPrompt<'_>,
    cancel: &Arc<AtomicBool>,
) -> Result<Option<investigation::StrategyChoice>> {
    if secret.model.trim().is_empty() {
        return Ok(None);
    }
    let limits = &prompt.settings.recon_limits;
    let user = format!(
        "Question: {}\nOpening turn: {}\nUseful evidence already stored: {}\nBrain context thin: {}\nPrevious strategy: {}\nBudgets: firecrawl {}, hunter {}, sociavault {}.\nReturn JSON {{\"strategy\":\"discovery\"|\"hypothesis\"|\"adaptive\",\"rationale\":\"one or two sentences\"}}.",
        prompt.question,
        prompt.opening,
        prompt.useful,
        prompt.unfamiliar,
        prompt.previous,
        limits.firecrawl_credits,
        limits.hunter_credits,
        limits.sociavault_credits
    );
    let value = model_json(
        secret,
        "Choose one investigative strategy for this turn. discovery: broad questions or unfamiliar subjects. hypothesis: ambiguous identity, relationship, ownership, or conflicting claims. adaptive: a focused question or useful evidence already in hand. Do not call tools.",
        &user,
        cancel,
    )
    .await?;
    let kind = value.get("strategy").and_then(Value::as_str).unwrap_or("");
    let rationale = value
        .get("rationale")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if !matches!(kind, "discovery" | "hypothesis" | "adaptive")
        || rationale.is_empty()
        || rationale.chars().count() > 400
    {
        return Ok(None);
    }
    Ok(Some(investigation::StrategyChoice {
        kind: kind.into(),
        rationale: rationale.into(),
    }))
}

async fn model_queries(
    secret: &ProviderSecret,
    question: &str,
    strategy: &str,
    fallback: &[investigation::DiscoveryQuery; 2],
    cancel: &Arc<AtomicBool>,
) -> Result<Option<[investigation::DiscoveryQuery; 2]>> {
    if secret.model.trim().is_empty() {
        return Ok(None);
    }
    let user = format!(
        "Question: {question}\nStrategy: {strategy}\nDraft identity query: {}\nDraft investigative query: {}\nReturn JSON {{\"identity_query\":string,\"investigative_query\":string}}. The two queries must explore different angles.",
        fallback[0].query, fallback[1].query
    );
    let value = model_json(
        secret,
        "Write two Firecrawl search queries. The identity query establishes the subject and its authoritative identifiers. The investigative query addresses the requested relationship, activity, event, or competing explanation. Do not rephrase one query as the other.",
        &user,
        cancel,
    )
    .await?;
    let identity = value
        .get("identity_query")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let investigative = value
        .get("investigative_query")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if identity.chars().count() > 180
        || investigative.chars().count() > 180
        || !investigation::distinct_queries(identity, investigative)
    {
        return Ok(None);
    }
    Ok(Some([
        investigation::DiscoveryQuery {
            role: "identity".into(),
            query: identity.into(),
            angle: fallback[0].angle.clone(),
        },
        investigation::DiscoveryQuery {
            role: "investigative".into(),
            query: investigative.into(),
            angle: fallback[1].angle.clone(),
        },
    ]))
}

async fn model_subset(
    secret: &ProviderSecret,
    actions: &[investigation::ProposedAction],
    cancel: &Arc<AtomicBool>,
) -> Result<Option<Vec<investigation::ProposedAction>>> {
    if secret.model.trim().is_empty() || actions.is_empty() {
        return Ok(None);
    }
    let listing: Vec<_> = actions
        .iter()
        .map(|action| {
            json!({
                "id": action.id,
                "tool": action.tool_id,
                "purpose": action.purpose,
                "cost": action.credit_cost,
                "gap": action.gap_id,
                "cache": action.cache_available,
            })
        })
        .collect();
    let value = model_json(
        secret,
        "These lookups are already grounded and ranked. Return JSON {\"selected\":[id,...]} using only the given ids. Return an empty list when none should run. Do not invent tools or inputs.",
        &serde_json::to_string(&listing)?,
        cancel,
    )
    .await?;
    let Some(selected) = value.get("selected").and_then(Value::as_array) else {
        return Ok(None);
    };
    let ids: HashSet<&str> = selected.iter().filter_map(Value::as_str).collect();
    Ok(Some(
        actions
            .iter()
            .filter(|action| ids.contains(action.id.as_str()))
            .cloned()
            .collect(),
    ))
}

async fn model_assessment(
    secret: &ProviderSecret,
    question: &str,
    results: &[(String, ToolResult)],
    fallback: &[investigation::ToolSuggestion],
    enabled: &HashSet<String>,
    cancel: &Arc<AtomicBool>,
) -> Result<Option<investigation::AnswerAssessment>> {
    if secret.model.trim().is_empty() {
        return Ok(None);
    }
    let evidence: Vec<_> = results
        .iter()
        .rev()
        .take(12)
        .map(|(id, result)| {
            json!({
                "id": id,
                "tool": result.tool_id,
                "status": result.status,
                "observations": super::packet_observation(&result.observations),
            })
        })
        .collect();
    let catalog: Vec<_> = crate::osint::registry()
        .iter()
        .filter(|tool| enabled.contains(tool.id))
        .map(|tool| json!({"id": tool.id, "description": tool.description}))
        .collect();
    let user = format!(
        "Question: {question}\nTool results: {}\nEnabled tools: {}\nReturn JSON {{\"answered\":boolean,\"tools\":[{{\"tool_id\":string,\"reason\":string}}]}}. If answered is true, tools must be empty. If answered is false, name at least 3 enabled tools that were not already used and that could supply missing context.",
        serde_json::to_string(&evidence)?,
        serde_json::to_string(&catalog)?
    );
    let value = model_json(
        secret,
        "Decide whether the tool results sufficiently answer the user's question. Do not write a reply to the user. When they do not, suggest only real tool ids from the enabled list.",
        &user,
        cancel,
    )
    .await?;
    Ok(investigation::assessment_from_model(&value, fallback))
}

async fn model_json(
    secret: &ProviderSecret,
    system: &str,
    user: &str,
    cancel: &Arc<AtomicBool>,
) -> Result<Value> {
    let messages = [
        super::chat("system", system.into()),
        super::chat("user", user.into()),
    ];
    let response = tokio::select! {
        result = provider::complete(secret, &messages, &[], |_| {}) => result?,
        _ = super::wait_cancel(cancel.clone()) => return Err(anyhow!("cancelled")),
    };
    super::parse_json(&response.content)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action(tool_id: &str, arguments: Value) -> investigation::ProposedAction {
        investigation::ProposedAction {
            id: String::new(),
            tool_id: tool_id.into(),
            arguments,
            gap_id: "gap-profile".into(),
            purpose: "Tool isolation".into(),
            evidence_ids: vec!["question".into()],
            expected: "profile".into(),
            credit_cost: 0,
            provider: String::new(),
            cache_available: false,
            scarce: false,
            rank_reason: String::new(),
            alternative_id: String::new(),
        }
    }

    #[test]
    fn isolation_lines_show_what_ran_what_was_held_and_why() {
        let wave = WaveOutcome {
            ran: vec![action("stackexchange_users", json!({"name": "Donald Trump"}))],
            blocked: vec![action("keybase_identity", json!({"username": "realDonaldTrump"}))],
        };
        let lines = isolation_lines(
            &wave,
            &["sociavault_profile — skipped: no SociaVault API key is configured.".into()],
        );
        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with("stackexchange_users — ran with"));
        assert!(lines[0].contains("Donald Trump"));
        assert!(lines[1].starts_with("keybase_identity — not run"));
        assert!(lines[2].contains("SociaVault API key"));
        let step = PlanCall {
            step_id: "isolate-2".into(),
            ..action_call(&wave.ran[0], 2)
        };
        assert_eq!(step.tool_id, "stackexchange_users");
        assert_eq!(step.arguments, json!({"name": "Donald Trump"}));
        assert_eq!(step.reason, "Tool isolation");
        assert_eq!(count_tools(&wave.ran, |id| id == "stackexchange_users"), 1);
    }
}
