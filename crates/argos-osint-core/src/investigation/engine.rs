//! Shared task execution engine, controller checkpoints, and coordinator.

use anyhow::Result;
use chrono::Utc;
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::contracts::{InvestigationSurface, TaskStatus};
use super::evidence::{
    assess_claim_against_passages, curate_passages_from_result, ClaimAssessment,
};
use super::gates::{validate_task_admission, validate_tool_preflight};
use super::patterns::InvestigationPattern;
use super::policy::SurfacePolicy;
use super::trace::InvestigationEvent;
use crate::intel_recon::ReportMode;
use crate::osint::{Executor, ProviderKeys};
use crate::provider::SettingsFile;
use crate::store::Store;

/// Execution runtime configuration for the shared harness.
#[derive(Clone)]
pub struct InvestigationRuntime {
    pub investigation_id: String,
    pub run_id: String,
    pub surface: InvestigationSurface,
    pub report_mode: ReportMode,
    pub query: String,
    pub keys: ProviderKeys,
    pub settings: SettingsFile,
    pub cancel: Arc<AtomicBool>,
}

/// Orchestrates an incremental execution step in the shared investigation harness.
pub async fn execute_harness_step(store: &Store, runtime: &InvestigationRuntime) -> Result<bool> {
    if runtime.cancel.load(Ordering::Relaxed) {
        return Ok(false);
    }

    // 1. Fetch pending tasks
    let tasks = store.list_investigation_tasks(&runtime.investigation_id)?;
    let ready_task = tasks
        .into_iter()
        .find(|t| t.status == TaskStatus::Planned || t.status == TaskStatus::Ready);

    let Some(mut task) = ready_task else {
        // No ready tasks remaining; check controller / completion
        return Ok(false);
    };

    // 2. Task admission check
    let deps = store.get_task_dependencies(&task.id)?;
    let pending_deps = deps
        .iter()
        .filter(|dep_id| {
            if let Ok(Some(dep_task)) = store.get_investigation_task(dep_id) {
                !dep_task.status.is_terminal()
            } else {
                false
            }
        })
        .count();

    let admission = validate_task_admission(&task, pending_deps);
    if !admission.is_passed() {
        task.status = TaskStatus::Deferred;
        task.terminal_reason = "dependencies pending or limits reached".into();
        store.insert_investigation_task(&task)?;
        return Ok(false);
    }

    // 3. Mark task running
    task.status = TaskStatus::Running;
    task.attempts += 1;
    store.insert_investigation_task(&task)?;

    // Record task started event
    let seq = store.next_event_sequence(&runtime.investigation_id)?;
    let ev = InvestigationEvent::new(
        &runtime.investigation_id,
        seq,
        runtime.surface,
        "task_started",
        format!("Started task {}: {}", task.task_id, task.objective),
    )
    .with_task(
        &runtime.run_id,
        &task.directive_id,
        &task.task_id,
        task.revision,
    );
    store.insert_investigation_event(&ev)?;

    // 4. Execute tool if applicable
    let preferred = InvestigationPattern::detect(&task.objective, &[]).preferred_tools();
    let tool_id = preferred.first().copied().unwrap_or("firecrawl_search");

    let policy = SurfacePolicy::for_surface(runtime.surface);
    let enabled = store.tool_enabled(tool_id).unwrap_or(true);
    let proposal = super::contracts::CallProposal {
        task_id: task.task_id.clone(),
        directive_id: task.directive_id.clone(),
        task_revision: task.revision,
        tool_id: tool_id.to_string(),
        contract_version: "v1".into(),
        arguments: json!({"query": runtime.query}),
        purpose: task.objective.clone(),
        expected_evidence: "results".into(),
        satisfaction_test: "results > 0".into(),
    };

    let preflight = validate_tool_preflight(&proposal, &policy, enabled);
    if !preflight.is_passed() {
        task.status = TaskStatus::Failed;
        task.terminal_reason = format!("Preflight rejected tool {tool_id}");
        store.insert_investigation_task(&task)?;
        return Ok(true);
    }

    // Execute with executor
    let executor = Executor::new()?;
    let call_res = executor
        .run_configured(
            tool_id,
            json!({"query": runtime.query, "limit": 5}),
            Some(&runtime.settings.osint_user_agent),
            &runtime.keys,
        )
        .await;

    match call_res {
        Ok(tool_result) => {
            // 5. Curate evidence passages
            let passages = curate_passages_from_result(
                &runtime.investigation_id,
                &task.task_id,
                "call_harness",
                &tool_result,
            );

            for passage in &passages {
                store.insert_evidence_passage(passage)?;
            }

            // 6. Assess claims
            let outcome = assess_claim_against_passages(&task.task_id, &task.objective, &passages);
            let assessment = ClaimAssessment {
                id: format!(
                    "ass:{}:{}",
                    task.task_id,
                    Utc::now().timestamp_subsec_millis()
                ),
                investigation_id: runtime.investigation_id.clone(),
                claim_id: task.task_id.clone(),
                evidence_id: outcome
                    .cited_passage_ids
                    .first()
                    .cloned()
                    .unwrap_or_default(),
                stance: outcome.stance,
                rationale: outcome.rationale,
                created_at: Utc::now().to_rfc3339(),
            };
            store.insert_claim_assessment(&assessment)?;

            // 7. Complete task
            task.status = TaskStatus::Completed;
            task.output_ref = format!("passages:{}", passages.len());
            store.insert_investigation_task(&task)?;

            let seq2 = store.next_event_sequence(&runtime.investigation_id)?;
            let ev2 = InvestigationEvent::new(
                &runtime.investigation_id,
                seq2,
                runtime.surface,
                "task_completed",
                format!(
                    "Completed task {} with {} passages",
                    task.task_id,
                    passages.len()
                ),
            )
            .with_task(
                &runtime.run_id,
                &task.directive_id,
                &task.task_id,
                task.revision,
            );
            store.insert_investigation_event(&ev2)?;

            Ok(true)
        }
        Err(err) => {
            task.status = TaskStatus::Failed;
            task.terminal_reason = err.to_string();
            store.insert_investigation_task(&task)?;
            Ok(true)
        }
    }
}
