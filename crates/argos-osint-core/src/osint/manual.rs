//! Repository-backed manual calls. Admission claims a request once and the
//! API process completes it independently of the caller's connection. New
//! network calls use the durable dispatch adapter; embedded callers retain
//! their existing process-local entry point until its retirement gate.

mod durable;

pub use durable::status as durable_status;
pub use durable::DurableToolOutcome;
pub use durable::TOOLS_OPERATION;
pub use durable::TOOLS_POOL;

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use chrono::{Duration, Utc};
use serde::Serialize;
use serde_json::Value;
use surrealdb::types::SurrealValue;
use tokio::sync::{oneshot, Semaphore};

use super::tool_runner::{ToolAttribution, ToolRunner};
use super::{canonical_tool_id, ProviderKeys, ToolResult};
use crate::provider::SettingsFile;
use crate::storage::jobs::{bump, rid, run, take};
use crate::storage::repo::Repository;
use crate::storage::tx::{run_tx, RetryPolicy};

pub(crate) const ORIGIN: &str = "fleetworks-manual";
pub(crate) const PREFIX: &str = "fleet-tool-";
const MAX_PENDING: usize = 16;
const INTERRUPTED_AFTER_MINUTES: i64 = 15;

#[derive(Debug)]
#[non_exhaustive]
pub enum SubmitError {
    Invalid,
    Conflict,
    Busy,
    ContextChanged,
    Unavailable(anyhow::Error),
}

/// The durable identity of a manual invocation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct Receipt {
    pub call_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
}

/// The safe projection of a manual invocation, without provider wire data.
#[derive(Debug, Serialize)]
#[non_exhaustive]
pub struct Status {
    pub call_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    pub tool_id: String,
    pub status: String,
    pub result: Option<ToolResult>,
}

#[derive(SurrealValue)]
struct CallRow {
    origin: String,
    tool_id: String,
    inputs_json: String,
    status: String,
    started_at: String,
    result_json: Option<String>,
}

impl CallRow {
    fn matches(&self, tool: &str, inputs: &Value) -> bool {
        self.origin == ORIGIN
            && self.tool_id == tool
            && serde_json::from_str::<Value>(&self.inputs_json).is_ok_and(|saved| saved == *inputs)
    }
}

/// Bounded manual tool execution for a single API router.
#[derive(Clone)]
pub struct Tools {
    capacity: Arc<Semaphore>,
    runner: ToolRunner,
}

impl Default for Tools {
    fn default() -> Self {
        Self {
            capacity: Arc::new(Semaphore::new(MAX_PENDING)),
            runner: ToolRunner::new("", SettingsFile::default(), ProviderKeys::default()),
        }
    }
}

fn valid_request_id(value: &str) -> bool {
    (1..=64).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

/// The server only reads calls created through its own manual route.
pub fn valid_call_id(id: &str) -> bool {
    id.strip_prefix(PREFIX).is_some_and(valid_request_id)
}

impl Tools {
    /// Admit a manual call and run it independently of the request lifetime.
    /// A repeated ID with the same input returns the original receipt.
    pub async fn submit(
        &self,
        repo: Repository,
        request_id: &str,
        tool_id: &str,
        inputs: Value,
    ) -> Result<Receipt, SubmitError> {
        let tool = canonical_tool_id(tool_id);
        // Atlas-only calls must go through their collection admission, which
        // reserves those providers' shared daily request buckets.
        if !valid_request_id(request_id)
            || super::atlas_pipeline_tool(tool)
            || super::validate(tool, &inputs).is_err()
        {
            return Err(SubmitError::Invalid);
        }
        let call_id = format!("{PREFIX}{request_id}");
        let Ok(slot) = self.capacity.clone().try_acquire_owned() else {
            return match row(&repo, &call_id)
                .await
                .map_err(SubmitError::Unavailable)?
            {
                Some(saved) if saved.matches(tool, &inputs) => Ok(Receipt {
                    call_id,
                    job_id: None,
                }),
                Some(_) => Err(SubmitError::Conflict),
                None => Err(SubmitError::Busy),
            };
        };
        let tool = tool.to_owned();
        let (reply, accepted) = oneshot::channel();
        let runner = self.runner.clone();
        tokio::spawn(async move {
            let _slot = slot;
            match admit(&repo, &call_id, &tool, &inputs).await {
                Ok(should_run) => {
                    let _ = reply.send(Ok(Receipt {
                        call_id: call_id.clone(),
                        job_id: None,
                    }));
                    if should_run {
                        execute(&repo, &runner, &call_id, &tool, inputs).await;
                    }
                }
                Err(error) => {
                    let _ = reply.send(Err(error));
                }
            }
        });
        accepted
            .await
            .map_err(|error| SubmitError::Unavailable(error.into()))?
    }
}

async fn admit(
    repo: &Repository,
    call_id: &str,
    tool: &str,
    inputs: &Value,
) -> Result<bool, SubmitError> {
    let saved_inputs =
        serde_json::to_string(inputs).map_err(|error| SubmitError::Unavailable(error.into()))?;
    let now = Utc::now().to_rfc3339();
    run_tx(repo.db(), RetryPolicy::default(), |tx| {
        let saved_inputs = &saved_inputs;
        let now = &now;
        async move {
            // The guard serializes the absent-row case across API replicas.
            bump(&tx, "job_guard", call_id).await?;
            let key = rid("osint_call", call_id);
            let mut response = run(
                &tx,
                tx.query(
                    "SELECT origin, tool_id, inputs_json, status, started_at, result_json FROM ONLY $call",
                )
                .bind(("call", key.clone())),
            )
            .await?;
            let existing: Option<CallRow> = take(&mut response, 0)?;
            if let Some(existing) = existing {
                return Ok(if existing.matches(tool, inputs) {
                    Ok(false)
                } else {
                    Err(SubmitError::Conflict)
                });
            }
            run(
                &tx,
                tx.query(
                    "CREATE $call SET tool_id=$tool, origin=$origin, inputs_json=$input,
                     status='running', attempts=1, started_at=$at RETURN NONE",
                )
                .bind(("call", key))
                .bind(("tool", tool.to_owned()))
                .bind(("origin", ORIGIN))
                .bind(("input", saved_inputs.clone()))
                .bind(("at", now.clone())),
            )
            .await?;
            Ok(Ok(true))
        }
    })
    .await
    .map_err(|error| SubmitError::Unavailable(error.into()))?
    .value
}

async fn execute(repo: &Repository, runner: &ToolRunner, call_id: &str, tool: &str, inputs: Value) {
    let settings = tokio::task::spawn_blocking(SettingsFile::load).await;
    let keys = settings
        .as_ref()
        .ok()
        .and_then(|result| result.as_ref().ok())
        .map(ProviderKeys::from_settings)
        .unwrap_or_default();
    let result = match settings {
        Ok(Ok(settings)) => run_tool(repo, runner, call_id, tool, &inputs, settings).await,
        _ => Err(anyhow::anyhow!("Could not read provider configuration")),
    };
    let mut result = result.unwrap_or_else(|error| ToolResult {
        tool_id: tool.to_owned(),
        inputs,
        status: "failed".to_owned(),
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
    super::redact_manual_result(&mut result, tool, &keys);
    if let Err(error) = finish(repo, call_id, &result).await {
        tracing::warn!(error = %crate::provider_diag::sanitize(&error.to_string()),
            "Could not persist manual tool result");
    }
}

async fn run_tool(
    repo: &Repository,
    runner: &ToolRunner,
    call_id: &str,
    tool: &str,
    inputs: &Value,
    settings: SettingsFile,
) -> anyhow::Result<ToolResult> {
    let cancel = Arc::new(AtomicBool::new(false));
    run_tool_with_cancel(repo, runner, call_id, tool, inputs, settings, &cancel, None).await
}

async fn run_tool_with_cancel(
    repo: &Repository,
    runner: &ToolRunner,
    call_id: &str,
    tool: &str,
    inputs: &Value,
    settings: SettingsFile,
    cancel: &Arc<AtomicBool>,
    keys: Option<ProviderKeys>,
) -> anyhow::Result<ToolResult> {
    anyhow::ensure!(
        crate::storage::tool_state::tool_enabled(repo, tool).await?,
        "Tool disabled in catalog"
    );
    let runner = match keys {
        Some(keys) => runner.for_repository_with_keys(repo.clone(), settings, keys),
        None => runner.for_repository(repo.clone(), settings),
    };
    let attribution = ToolAttribution::manual().with_call_id(call_id);
    runner
        .execute_tool_attributed(tool, inputs.clone(), false, cancel, &attribution)
        .await
}

async fn finish(repo: &Repository, call_id: &str, result: &ToolResult) -> anyhow::Result<()> {
    repo.db()
        .query(
            "UPDATE $call SET status=$status, result_json=$result, completed_at=$now
             WHERE origin=$origin AND status='running' RETURN NONE",
        )
        .bind(("call", rid("osint_call", call_id)))
        .bind(("status", result.status.clone()))
        .bind(("result", serde_json::to_string(result)?))
        .bind(("now", Utc::now().to_rfc3339()))
        .bind(("origin", ORIGIN))
        .await?
        .check()?;
    Ok(())
}

async fn row(repo: &Repository, call_id: &str) -> anyhow::Result<Option<CallRow>> {
    let mut reply = repo
        .db()
        .query(
            "SELECT origin, tool_id, inputs_json, status, started_at, result_json FROM ONLY $call WHERE origin=$origin",
        )
        .bind(("call", rid("osint_call", call_id)))
        .bind(("origin", ORIGIN))
        .await?
        .check()?;
    Ok(reply.take(0usize)?)
}

/// Read a saved result. An abandoned execution is marked interrupted and
/// never repeated automatically because an external provider may have run it.
pub async fn status(repo: &Repository, call_id: &str) -> anyhow::Result<Option<Status>> {
    if !valid_call_id(call_id) {
        return Ok(None);
    }
    let Some(mut call) = row(repo, call_id).await? else {
        return Ok(None);
    };
    let now = Utc::now();
    let cutoff = (now - Duration::minutes(INTERRUPTED_AFTER_MINUTES)).to_rfc3339();
    if call.status == "running" && call.started_at < cutoff {
        run_tx(repo.db(), RetryPolicy::default(), |tx| {
            let cutoff = &cutoff;
            async move {
                bump(&tx, "job_guard", call_id).await?;
                run(
                    &tx,
                    tx.query(
                        "UPDATE $call SET status='interrupted', completed_at=$now
                         WHERE origin=$origin AND status='running' AND started_at < $cutoff RETURN NONE",
                    )
                    .bind(("call", rid("osint_call", call_id)))
                    .bind(("origin", ORIGIN))
                    .bind(("now", now.to_rfc3339()))
                    .bind(("cutoff", cutoff.clone())),
                )
                .await?;
                Ok(())
            }
        })
        .await?;
        let Some(current) = row(repo, call_id).await? else {
            return Ok(None);
        };
        call = current;
    }
    // A transport may have reached the provider before interruption or before
    // a local settlement failed. Never return any remaining estimate to quota.
    if call.status != "running" {
        crate::storage::credits::settle_abandoned_manual_call(repo.db(), call_id).await?;
    }
    Ok(Some(Status {
        call_id: call_id.to_owned(),
        job_id: None,
        tool_id: call.tool_id,
        status: call.status,
        result: call
            .result_json
            .map(|raw| serde_json::from_str(&raw))
            .transpose()?,
    }))
}
