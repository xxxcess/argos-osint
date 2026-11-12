//! Queued network Tools calls. Provider execution never occurs in admission.

use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use chrono::Utc;
use serde_json::Value;
use sha2::Digest;
use sha2::Sha256;

use super::super::canonical_tool_id;
use super::super::ProviderKeys;
use super::super::ToolResult;
use super::CallRow;
use super::Receipt;
use super::Status;
use super::SubmitError;
use super::Tools;
use super::MAX_PENDING;
use super::ORIGIN;
use super::PREFIX;
use crate::storage::job_context::InteractiveAdmission;
use crate::storage::job_dispatch;
use crate::storage::jobs;
use crate::storage::jobs::ClaimedTask;
use crate::storage::jobs::NewJob;
use crate::storage::jobs::NewTask;
use crate::storage::repo::Repository;
use crate::storage::tx::run_tx;
use crate::storage::tx::RetryPolicy;
use crate::storage::tx::Tx;
use crate::storage::tx::TxError;

/// The host must explicitly enable and consume this durable dispatch pool.
pub const TOOLS_POOL: &str = "manual_tools";
/// One provider invocation, with conservative interruption recovery.
pub const TOOLS_OPERATION: &str = "manual_tool";
const MAX_INPUT_BYTES: usize = 16 * 1024;

/// The queue owner maps this result to its existing fenced delivery finish.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DurableToolOutcome {
    Complete { result_ref: String },
    Failed { reason: &'static str },
    Stale,
}

fn digest(parts: &[&str]) -> String {
    let mut digest = Sha256::new();
    for part in parts {
        digest.update((part.len() as u64).to_be_bytes());
        digest.update(part.as_bytes());
    }
    format!("{:x}", digest.finalize())
}

fn canonical(value: &Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.iter().map(canonical).collect()),
        Value::Object(values) => {
            let sorted: std::collections::BTreeMap<_, _> = values
                .iter()
                .map(|(key, value)| (key.clone(), canonical(value)))
                .collect();
            Value::Object(sorted.into_iter().collect())
        }
        other => other.clone(),
    }
}

fn aborted(code: &str) -> TxError {
    TxError::Abort(code.into())
}

fn submit_error(error: TxError) -> SubmitError {
    match &error {
        TxError::Abort(code)
            if matches!(
                code.as_str(),
                "manual_tool_conflict"
                    | "dispatch_submission_conflict"
                    | "dispatch_submission_retired"
                    | "request identity already accepted for different work"
            ) =>
        {
            SubmitError::Conflict
        }
        TxError::Abort(code) if code == "manual_tools_busy" => SubmitError::Busy,
        TxError::Abort(code)
            if code.contains("registered admission context")
                || code.contains("browser admission context")
                || code.starts_with("connect:") =>
        {
            SubmitError::ContextChanged
        }
        _ => SubmitError::Unavailable(error.into()),
    }
}

impl Tools {
    /// Commit invocation input, queued job/task/outbox and immutable origin in
    /// one transaction. No in-memory task is needed to retain acceptance.
    pub async fn submit_interactive(
        &self,
        repo: &Repository,
        request_id: &str,
        tool_id: &str,
        inputs: Value,
        admission: &InteractiveAdmission,
    ) -> Result<Receipt, SubmitError> {
        let tool = canonical_tool_id(tool_id);
        if !super::valid_request_id(request_id)
            || request_id != admission.origin().request_id
            || super::super::atlas_pipeline_tool(tool)
            || super::super::validate(tool, &inputs).is_err()
        {
            return Err(SubmitError::Invalid);
        }
        let saved_inputs = serde_json::to_string(&canonical(&inputs))
            .map_err(|error| SubmitError::Unavailable(error.into()))?;
        if saved_inputs.len() > MAX_INPUT_BYTES {
            return Err(SubmitError::Invalid);
        }
        let context = admission.context();
        let call_id = format!(
            "{PREFIX}{}",
            digest(&[&context.deployment_id, &context.registration_id, request_id])
        );
        let task_id = format!("{call_id}:execute");
        let input_hash = digest(&[tool, &saved_inputs]);
        let now = Utc::now();
        let job = NewJob {
            id: &call_id,
            kind: "task",
            operation: TOOLS_OPERATION,
            parent: None,
            created_at: now,
        };
        let task = NewTask {
            id: &task_id,
            job: &call_id,
            pool: TOOLS_POOL,
            operation: TOOLS_OPERATION,
            priority: 100,
            deps: Vec::new(),
            input_hash: &input_hash,
            created_at: now,
        };
        run_tx(repo.db(), RetryPolicy::default(), |tx| {
            let call_id = &call_id;
            let saved_inputs = &saved_inputs;
            let job = &job;
            let task = &task;
            let inputs = &inputs;
            let task_id = &task_id;
            async move {
                // Serialize the bounded shared pending count and duplicate
                // identity before any absent-row read or acceptance write.
                jobs::bump(&tx, "job_guard", "manual-tools-capacity").await?;
                jobs::bump(&tx, "job_guard", call_id).await?;
                let mut response = jobs::run(&tx, tx.query("SELECT origin,tool_id,inputs_json,status,started_at,result_json FROM ONLY $call")
                    .bind(("call", jobs::rid("osint_call", call_id)))).await?;
                let existing: Option<CallRow> = jobs::take(&mut response, 0)?;
                if let Some(existing) = existing {
                    if !existing.matches(tool, &inputs) { return Err(aborted("manual_tool_conflict")); }
                } else {
                    let mut response = jobs::run(&tx, tx.query("SELECT VALUE count() FROM osint_call WHERE origin=$origin AND status IN ['queued','running'] AND type::record('job',record::id(id)).operation=$operation AND type::record('job',record::id(id)).state IN ['queued','running','reconciling'] GROUP ALL")
                        .bind(("origin", ORIGIN)).bind(("operation", TOOLS_OPERATION))).await?;
                    let count: Vec<i64> = jobs::take(&mut response, 0)?;
                    if count.first().copied().unwrap_or(0) >= MAX_PENDING as i64 { return Err(aborted("manual_tools_busy")); }
                    jobs::run(&tx, tx.query("CREATE $call SET tool_id=$tool,origin=$origin,inputs_json=$input,status='queued',attempts=0,started_at=$at RETURN NONE")
                        .bind(("call", jobs::rid("osint_call", call_id))).bind(("tool", tool.to_owned()))
                        .bind(("origin", ORIGIN)).bind(("input", saved_inputs.clone())).bind(("at", now.to_rfc3339()))).await?;
                }
                let created = job_dispatch::submit_in(&tx, job, task).await?;
                admission.record_in(&tx, call_id, TOOLS_OPERATION, created).await?;
                if created {
                    jobs::run(&tx, tx.query("UPDATE $job SET app='tools',title=$title,tool=$tool,cancellable=false RETURN NONE; UPDATE $task SET max_attempts=1 RETURN NONE;")
                        .bind(("job", jobs::rid("job", call_id))).bind(("task", jobs::rid("task", &task_id)))
                        .bind(("title", format!("Tool: {tool}"))).bind(("tool", tool.to_owned()))).await?;
                }
                Ok(())
            }
        }).await.map_err(submit_error)?;
        Ok(Receipt {
            call_id: call_id.clone(),
            job_id: Some(call_id),
        })
    }

    /// Execute only a live claimed Tools task. The host owns queue/lease renewal
    /// and must retain this future until it stops after signalling cancellation.
    /// An ambiguous previous provider attempt is marked interrupted, never
    /// automatically issued again merely because the delivery was recovered.
    pub async fn execute_claimed(
        &self,
        repo: &Repository,
        task: &ClaimedTask,
        cancel: Arc<AtomicBool>,
    ) -> anyhow::Result<DurableToolOutcome> {
        anyhow::ensure!(
            task.pool == TOOLS_POOL
                && task.operation == TOOLS_OPERATION
                && task.id == format!("{}:execute", task.job_id)
                && super::valid_call_id(&task.job_id),
            "invalid_manual_tool_claim"
        );
        let start = begin_execution(repo, task).await?;
        let (tool, inputs) = match start {
            ExecutionStart::Run { tool, inputs } => (tool, inputs),
            ExecutionStart::Terminal(outcome) => return Ok(outcome),
        };
        if cancel.load(Ordering::Acquire) {
            return interrupt(repo, task, &tool, &inputs).await;
        }
        let origin = crate::storage::job_context::origin_for_job(repo, &task.job_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("manual_tool_origin_missing"))?;
        let settings = tokio::task::spawn_blocking(move || {
            let snapshot = crate::current_configuration::read(&crate::paths::home_dir())?;
            anyhow::ensure!(
                snapshot.revision == origin.configuration_revision,
                "manual_tool_configuration_changed_after_admission"
            );
            Ok::<_, anyhow::Error>(snapshot.settings)
        })
        .await;
        let keys = settings
            .as_ref()
            .ok()
            .and_then(|result| result.as_ref().ok())
            .map(ProviderKeys::from_saved_settings)
            .unwrap_or_default();
        let result = match settings {
            Ok(Ok(settings)) => {
                super::run_tool_with_cancel(
                    repo,
                    &self.runner,
                    &task.job_id,
                    &tool,
                    &inputs,
                    settings,
                    &cancel,
                    Some(keys.clone()),
                )
                .await
            }
            _ => Err(anyhow::anyhow!("Could not read provider configuration")),
        };
        if cancel.load(Ordering::Acquire) {
            return interrupt(repo, task, &tool, &inputs).await;
        }
        let mut result = result.unwrap_or_else(|error| ToolResult {
            tool_id: tool.clone(),
            inputs,
            status: "failed".into(),
            source_url: String::new(),
            retrieved_at: Utc::now().to_rfc3339(),
            observations: Value::Null,
            raw: String::new(),
            error: Some(error.to_string()),
            cached: false,
            truncated: false,
            credits_charged: 0,
            credits_reported: None,
        });
        super::super::redact_manual_result(&mut result, &tool, &keys);
        persist_result(repo, task, &result).await
    }
}

enum ExecutionStart {
    Run { tool: String, inputs: Value },
    Terminal(DurableToolOutcome),
}

async fn live_fence(tx: &Tx, task: &ClaimedTask, now: &str) -> Result<bool, TxError> {
    jobs::bump(tx, "job_guard", &task.root_id).await?;
    Ok(jobs::fence(tx, task, now)
        .await?
        .is_some_and(|fence| jobs::holds(&fence, task) && !fence.cancelled))
}

fn completed(call: &CallRow, call_id: &str) -> DurableToolOutcome {
    if matches!(call.status.as_str(), "completed" | "no_results") && call.result_json.is_some() {
        DurableToolOutcome::Complete {
            result_ref: call_id.to_owned(),
        }
    } else {
        DurableToolOutcome::Failed {
            reason: "manual_tool_interrupted_or_failed",
        }
    }
}

async fn begin_execution(repo: &Repository, task: &ClaimedTask) -> Result<ExecutionStart, TxError> {
    Ok(run_tx(repo.db(), RetryPolicy::default(), |tx| async move {
        let now = Utc::now().to_rfc3339();
        if !live_fence(&tx, task, &now).await? { return Ok(ExecutionStart::Terminal(DurableToolOutcome::Stale)); }
        let mut response = jobs::run(&tx, tx.query("SELECT origin,tool_id,inputs_json,status,started_at,result_json FROM ONLY $call")
            .bind(("call", jobs::rid("osint_call", &task.job_id)))).await?;
        let call: Option<CallRow> = jobs::take(&mut response, 0)?;
        let call = call.ok_or_else(|| aborted("manual_tool_payload_missing"))?;
        if call.origin != ORIGIN || digest(&[&call.tool_id, &call.inputs_json]) != task.input_hash { return Err(aborted("manual_tool_payload_changed")); }
        match call.status.as_str() {
            "queued" => {
                let inputs: Value = serde_json::from_str(&call.inputs_json).map_err(|_| aborted("manual_tool_input_invalid"))?;
                jobs::run(&tx, tx.query("UPDATE $call SET status='running',attempts+=1,started_at=$now RETURN NONE; UPDATE $job SET phase='Calling provider',progress_done=0,progress_total=1 RETURN NONE;")
                    .bind(("call", jobs::rid("osint_call", &task.job_id))).bind(("job", jobs::rid("job", &task.job_id))).bind(("now", now))).await?;
                Ok(ExecutionStart::Run { tool: call.tool_id, inputs })
            }
            "running" => {
                jobs::run(&tx, tx.query("UPDATE $call SET status='interrupted',completed_at=$now RETURN NONE")
                    .bind(("call", jobs::rid("osint_call", &task.job_id))).bind(("now", now))).await?;
                Ok(ExecutionStart::Terminal(DurableToolOutcome::Failed { reason: "manual_tool_previous_attempt_uncertain" }))
            }
            _ => Ok(ExecutionStart::Terminal(completed(&call, &task.job_id))),
        }
    }).await?.value)
}

async fn interrupt(
    repo: &Repository,
    task: &ClaimedTask,
    tool: &str,
    inputs: &Value,
) -> anyhow::Result<DurableToolOutcome> {
    let result = ToolResult {
        tool_id: tool.to_owned(),
        inputs: inputs.clone(),
        status: "interrupted".into(),
        source_url: String::new(),
        retrieved_at: Utc::now().to_rfc3339(),
        observations: Value::Null,
        raw: String::new(),
        error: Some("Tool execution interrupted; provider outcome may be uncertain".into()),
        cached: false,
        truncated: false,
        credits_charged: 0,
        credits_reported: None,
    };
    persist_result(repo, task, &result).await
}

async fn persist_result(
    repo: &Repository,
    task: &ClaimedTask,
    result: &ToolResult,
) -> anyhow::Result<DurableToolOutcome> {
    let json = serde_json::to_string(result)?;
    let saved = run_tx(repo.db(), RetryPolicy::default(), |tx| {
        let json = &json;
        async move {
            let now = Utc::now().to_rfc3339();
            if !live_fence(&tx, task, &now).await? { return Ok(false); }
            let mut response = jobs::run(&tx, tx.query("UPDATE $call SET status=$status,result_json=$result,completed_at=$now WHERE origin=$origin AND status='running' RETURN VALUE status; UPDATE $job SET phase='Result persisted',progress_done=1,progress_total=1 RETURN NONE;")
                .bind(("call", jobs::rid("osint_call", &task.job_id))).bind(("job", jobs::rid("job", &task.job_id)))
                .bind(("origin", ORIGIN)).bind(("status", result.status.clone())).bind(("result", json.clone())).bind(("now", now))).await?;
            Ok(!jobs::take::<Vec<String>>(&mut response, 0)?.is_empty())
        }
    }).await?.value;
    if !saved {
        return Ok(DurableToolOutcome::Stale);
    }
    Ok(
        if matches!(result.status.as_str(), "completed" | "no_results") {
            DurableToolOutcome::Complete {
                result_ref: task.job_id.clone(),
            }
        } else {
            DurableToolOutcome::Failed {
                reason: "manual_tool_execution_failed",
            }
        },
    )
}

/// Read-only durable status. Authorization belongs at the API boundary before
/// this payload read; no polling side effect may interrupt accepted work.
pub async fn status(repo: &Repository, call_id: &str) -> anyhow::Result<Option<Status>> {
    if !super::valid_call_id(call_id) {
        return Ok(None);
    }
    let mut response = repo
        .db()
        .query("RETURN (SELECT VALUE state FROM ONLY $job WHERE operation=$operation);")
        .bind(("job", jobs::rid("job", call_id)))
        .bind(("operation", TOOLS_OPERATION))
        .await?
        .check()?;
    let Some(job_state) = response.take::<Option<String>>(0)? else {
        return Ok(None);
    };
    let Some(call) = super::row(repo, call_id).await? else {
        return Ok(None);
    };
    let status = match (call.status.as_str(), job_state.as_str()) {
        ("queued" | "running", "cancelled" | "paused" | "superseded") => "interrupted".into(),
        ("running", "failed" | "partial") => "interrupted".into(),
        ("queued", "failed" | "partial") => "failed".into(),
        _ => call.status,
    };
    Ok(Some(Status {
        call_id: call_id.to_owned(),
        job_id: Some(call_id.to_owned()),
        tool_id: call.tool_id,
        status,
        result: call
            .result_json
            .map(|json| serde_json::from_str(&json))
            .transpose()?,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inputs_have_stable_identity_and_contexts_do_not_share_request_ids() {
        let a = serde_json::json!({"b":{"d":1,"c":2},"a":0});
        let b = serde_json::json!({"a":0,"b":{"c":2,"d":1}});
        assert_eq!(
            serde_json::to_string(&canonical(&a)).expect("canonical input"),
            serde_json::to_string(&canonical(&b)).expect("canonical input")
        );
        assert_ne!(
            digest(&["deployment-a", "registration-a", "request-a"]),
            digest(&["deployment-b", "registration-a", "request-a"])
        );
        assert_ne!(
            digest(&["deployment-a", "registration-a", "request-a"]),
            digest(&["deployment-a", "registration-b", "request-a"])
        );
    }
}
