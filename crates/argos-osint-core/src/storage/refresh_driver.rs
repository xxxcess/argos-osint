//! The shared insight refresh driver on SurrealDB: the production caller of
//! `refresh_exec::run_refresh_slice`, and the shared-refresh retry
//! (`Service::retry_insights` for the `insights::CONTRACT` path, and
//! `insight_persist::recover_refreshes`).
//!
//! For each refresh task the driver reads the captured input and settings
//! from the task's `insight_refresh_task` sidecar, resolves the derivation
//! context (the live Recon opt-in for a Recon owner), validates the live
//! evidence through the publishing transaction (`live_evidence::step`) and
//! runs the insight pipeline (`insights::run_pipeline`) as the slice's
//! executor. [`retry_owner`] re-enqueues an owner whose newest refresh
//! ended (failed, cancelled or paused) at a new generation and runs it; a
//! still-queued one is just run. No new tables.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{anyhow, Result};
use surrealdb::types::SurrealValue;

use super::derivations::DerivationContext;
use super::insight::SnapshotStep;
use super::refresh::OWNER_TASKS_SURQL;
use super::refresh_exec::{
    captured_settings, recover_refreshes, run_refresh_slice, Execute, RecoveryError,
};
use super::repo::Repository;
use crate::investigation::insights::{PipelineInput, PipelineOutput};
use crate::provider::SettingsFile;
use crate::secrets::AuthFile;

/// The production executor: `insights::run_pipeline` on the claimed input.
pub fn pipeline(
    auth: &AuthFile,
    settings: &SettingsFile,
    cancel: &Arc<AtomicBool>,
    db_path: PathBuf,
) -> Execute {
    let (auth, settings, cancel) = (auth.clone(), settings.clone(), cancel.clone());
    Box::new(move |mut input: PipelineInput| {
        Box::pin(async move {
            let out = crate::investigation::insights::run_pipeline(
                &mut input, &auth, &settings, &cancel, &db_path,
            )
            .await?;
            Ok((input, out))
        })
    })
}

#[derive(SurrealValue)]
struct Sidecar {
    input_ref: String,
}

/// The input a refresh task captured.
pub async fn task_input(repo: &Repository, task_id: &str) -> Result<Option<PipelineInput>> {
    let mut r = repo
        .db()
        .query("SELECT input_ref FROM ONLY type::record('insight_refresh_task', $t)")
        .bind(("t", task_id.to_string()))
        .await?
        .check()?;
    let side: Option<Sidecar> = r.take(0usize)?;
    side.map(|s| serde_json::from_str(&s.input_ref).map_err(Into::into))
        .transpose()
}

/// The derivation context for `input`'s owner: the live Recon opt-in for a
/// Recon run, the input's own `save_insights` otherwise.
pub async fn context(repo: &Repository, input: &PipelineInput) -> Result<DerivationContext> {
    if input.snapshot.owner_kind == "recon" {
        return super::insight_owner::recon_context(
            repo,
            &input.snapshot.owner_id,
            input.save_insights,
        )
        .await;
    }
    Ok(DerivationContext {
        save_insights: input.save_insights,
        recon_thread: None,
    })
}

/// Run one refresh task with its captured settings. `validate` defaults to
/// the live-evidence check of the task's snapshot.
pub async fn run_one(
    repo: &Repository,
    task_id: &str,
    settings: &SettingsFile,
    cancel: &Arc<AtomicBool>,
    validate: Option<SnapshotStep>,
    execute: Execute,
) -> Result<Option<PipelineOutput>> {
    let input = task_input(repo, task_id)
        .await?
        .ok_or_else(|| anyhow!("not an insight refresh task"))?;
    let captured = captured_settings(repo, task_id, settings).await?;
    let ctx = context(repo, &input).await?;
    let validate = validate.unwrap_or_else(|| super::live_evidence::step(input.snapshot.clone()));
    run_refresh_slice(repo, task_id, &captured, cancel, validate, ctx, execute).await
}

/// `insight_persist::recover_refreshes` on SurrealDB: reschedule durable
/// article intents, reap expired leases, then run every queued refresh.
/// Returns how many published and the recovery errors (`insight.recovery_error`
/// events are the caller's to record).
pub async fn recover_and_run(
    repo: &Repository,
    settings: &SettingsFile,
    cancel: &Arc<AtomicBool>,
    executor: impl Fn() -> Execute,
) -> Result<(usize, Vec<RecoveryError>)> {
    let (ids, mut errors) = recover_refreshes(repo, settings).await?;
    let mut completed = 0;
    for id in ids {
        if cancel.load(Ordering::Acquire) {
            break;
        }
        match run_one(repo, &id, settings, cancel, None, executor()).await {
            Ok(Some(_)) => completed += 1,
            Ok(None) => {}
            Err(error) => errors.push(RecoveryError {
                resource: id,
                message: error.to_string(),
            }),
        }
    }
    Ok((completed, errors))
}

#[derive(SurrealValue)]
struct OwnerTask {
    id: String,
    input_ref: String,
    state: String,
}

/// What [`retry_owner`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Retried {
    /// The owner has no refresh task to retry.
    Nothing,
    /// The newest refresh already completed: nothing to retry, and the
    /// saved answer must not be mined instead.
    Completed { task_id: String },
    /// The task ran (it published when `published`).
    Ran { task_id: String, published: bool },
}

/// The shared-refresh retry for one owner (`kind`, `owner`): the newest
/// refresh task runs again if still queued; if it ended, its input is
/// re-enqueued at a new generation and that task runs.
pub async fn retry_owner(
    repo: &Repository,
    kind: &str,
    owner: &str,
    settings: &SettingsFile,
    cancel: &Arc<AtomicBool>,
    validate: Option<SnapshotStep>,
    execute: Execute,
) -> Result<Retried> {
    let mut r = repo
        .db()
        .query(OWNER_TASKS_SURQL)
        .bind(("scope", format!("{kind}:{owner}")))
        .await?
        .check()?;
    let tasks: Vec<OwnerTask> = r.take(0usize)?;
    let Some(newest) = tasks.into_iter().next() else {
        return Ok(Retried::Nothing);
    };
    let task_id = match newest.state.as_str() {
        "queued" | "retry_scheduled" => {
            // Same generation: admit it like `insight_persist::validate_retry`
            // (live evidence, still the desired generation).
            let input: PipelineInput = serde_json::from_str(&newest.input_ref)?;
            super::insight_retry::validate_retry(repo.db(), &input).await?;
            newest.id
        }
        "running" => anyhow::bail!("the insight refresh is already running"),
        "completed" => return Ok(Retried::Completed { task_id: newest.id }),
        "paused" => {
            // `enqueue_refresh` requeues a task paused with evidence dirty;
            // any other pause gets a new generation below.
            let captured = captured_settings(repo, &newest.id, settings).await?;
            let mut input: PipelineInput = serde_json::from_str(&newest.input_ref)?;
            let id = super::insight_owner::enqueue_refresh(repo, &mut input, &captured).await?;
            if state_of(repo, &id).await? == "queued" {
                id
            } else {
                reenqueue(repo, kind, owner, &newest, settings).await?
            }
        }
        _ => reenqueue(repo, kind, owner, &newest, settings).await?,
    };
    let published = run_one(repo, &task_id, settings, cancel, validate, execute)
        .await?
        .is_some();
    Ok(Retried::Ran { task_id, published })
}

async fn state_of(repo: &Repository, task_id: &str) -> Result<String> {
    let mut r = repo
        .db()
        .query("SELECT VALUE state FROM ONLY type::record('task', $t)")
        .bind(("t", task_id.to_string()))
        .await?
        .check()?;
    Ok(r.take::<Option<String>>(0usize)?.unwrap_or_default())
}

/// Re-enqueue `task`'s captured input at the owner's next generation.
async fn reenqueue(
    repo: &Repository,
    kind: &str,
    owner: &str,
    task: &OwnerTask,
    settings: &SettingsFile,
) -> Result<String> {
    let captured = captured_settings(repo, &task.id, settings).await?;
    let mut input: PipelineInput = serde_json::from_str(&task.input_ref)?;
    let now = chrono::Utc::now().to_rfc3339();
    input.snapshot.generation =
        super::insight_owner::advance_generation(repo, kind, owner, &now).await?;
    super::insight_owner::enqueue_refresh(repo, &mut input, &captured).await
}
