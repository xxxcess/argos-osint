//! Insight refresh execution and recovery on SurrealDB (migration phase
//! 4d): `insight_persist::run_refresh_slice` and `recover_refreshes`.
//!
//! The slice claims exactly its `insight_refresh` task, admits it (live
//! evidence, generation, article details, captured input key), runs the
//! pipeline under a 30 s lease heartbeat that drops superseded work, and
//! ends in one of: slice continuation (requeued), supersession, cancelled
//! (paused, evidence dirty), failure (owner state `previous`) or fenced
//! publication with derivations (`insight::publish_snapshot`).
//!
//! SurrealDB `task` rows have no message or blocked-reason columns, so the
//! terminal message and `blocked_reason` live on the `insight_refresh_task`
//! sidecar. Caller-supplied, because their inputs are still SQLite: the
//! pipeline run (`execute`), live-evidence validation (`validate`) and the
//! derivation context (the live Recon opt-in). `insight.recovery_error`
//! events are returned rather than recorded (`events` is SQLite).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{anyhow, ensure, Result};
use chrono::Utc;
use futures_util::future::BoxFuture;
use surrealdb::types::SurrealValue;

use super::derivations::{derivation_step, DerivationContext};
use super::insight::{desired_generation, publish_snapshot, SnapshotStep};
use super::jobs::ClaimedTask;
use super::jobs::{bump, claim_task, fence, holds, interrupt_expired_leases, rid, run, ts};
use super::repo::Repository;
use super::tx::{run_tx, RetryPolicy, Tx, TxError};
use crate::investigation::insight_persist::article_details_eligible;
use crate::investigation::insights::{input_key, PipelineInput, PipelineOutput};
use crate::provider::SettingsFile;

const LEASE_SECS: i64 = 120;
const OPERATION: &str = "insight_refresh";

/// Runs the pipeline for a claimed refresh input, returning the input as
/// the pipeline left it with the output.
pub type Execute = Box<
    dyn FnOnce(PipelineInput) -> BoxFuture<'static, Result<(PipelineInput, PipelineOutput)>> + Send,
>;

/// How a claimed refresh ends without publication.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ending {
    /// Slice yield: back to `queued`, attempt closed as `slice_complete`.
    Continue,
    Superseded,
    /// Cancelled with evidence dirty: `paused`, attempt refunded.
    Blocked,
    Failed,
}

#[derive(SurrealValue)]
struct Sidecar {
    input_ref: String,
    input_hash: String,
    role_snapshot: String,
}

async fn sidecar(repo: &Repository, task_id: &str) -> Result<Option<Sidecar>> {
    let mut r = repo
        .db()
        .query(
            "SELECT input_ref, input_hash, role_snapshot FROM insight_refresh_task
               WHERE id = type::record('insight_refresh_task', $t)",
        )
        .bind(("t", task_id.to_string()))
        .await?
        .check()?;
    Ok(r.take::<Vec<Sidecar>>(0)?.into_iter().next())
}

/// End a claimed refresh under its lease fence. `false` when the lease was
/// lost (nothing written).
pub async fn end_claimed(
    repo: &Repository,
    claimed: &ClaimedTask,
    ending: Ending,
    message: &str,
) -> Result<bool> {
    let now = ts(Utc::now());
    let message = message.to_string();
    let done = run_tx(repo.db(), RetryPolicy::default(), |tx| {
        let (now, message) = (now.clone(), message.clone());
        async move {
            bump(&tx, "job_guard", &claimed.root_id).await?;
            match fence(&tx, claimed, &now).await? {
                Some(f) if holds(&f, claimed) => {}
                _ => return Ok(false),
            }
            let (state, outcome, blocked) = match ending {
                Ending::Continue => ("queued", "slice_complete", ""),
                Ending::Superseded => ("superseded", "superseded", ""),
                Ending::Blocked => ("paused", "cancelled", message.as_str()),
                Ending::Failed => ("failed", "failed", ""),
            };
            let terminal = matches!(ending, Ending::Superseded | Ending::Failed);
            run(
                &tx,
                tx.query(
                    "UPDATE $t SET state = $state, lease_owner = '', process_owner = '',
                       lease_until = NONE, next_eligible_at = <datetime>$now,
                       attempts = IF $refund THEN math::max([attempts - 1, 0]) ELSE attempts END,
                       finished_at = IF $terminal THEN <datetime>$now ELSE NONE END,
                       updated_at = <datetime>$now RETURN NONE;
                     LET $open = SELECT VALUE id FROM attempt WHERE task = $t AND epoch = $e
                       AND finished_at = NONE;
                     UPDATE $open SET finished_at = <datetime>$now, outcome = $outcome RETURN NONE;
                     UPDATE type::record('insight_refresh_task', record::id($t))
                       SET message = $msg, blocked_reason = $blocked RETURN NONE;",
                )
                .bind(("t", rid("task", &claimed.id)))
                .bind(("state", state))
                .bind(("refund", ending == Ending::Blocked))
                .bind(("terminal", terminal))
                .bind(("now", now.clone()))
                .bind(("e", claimed.lease_epoch))
                .bind(("outcome", outcome))
                .bind(("msg", message.clone()))
                .bind(("blocked", blocked.to_string())),
            )
            .await?;
            Ok(true)
        }
    })
    .await?;
    Ok(done.value)
}

/// `record_terminal_failure`: the owner falls back to its previous snapshot
/// when this generation is still desired.
pub async fn record_terminal_failure_in(
    tx: &Tx,
    input: &PipelineInput,
    error: &str,
    lifecycle: &str,
    now: &str,
) -> Result<(), TxError> {
    run(
        tx,
        tx.query(
            "UPDATE insight_owner_state SET lifecycle = $life, last_error = $err, updated_at = $now
               WHERE id = type::record('insight_owner_state', [$k, $o]) AND desired_generation = $g
               RETURN NONE;",
        )
        .bind(("k", input.snapshot.owner_kind.clone()))
        .bind(("o", input.snapshot.owner_id.clone()))
        .bind(("g", i64::from(input.snapshot.generation)))
        .bind(("life", lifecycle.to_string()))
        .bind(("err", error.to_string()))
        .bind(("now", now.to_string())),
    )
    .await?;
    Ok(())
}

async fn owner_state(
    repo: &Repository,
    input: &PipelineInput,
    error: &str,
    lifecycle: &str,
) -> Result<()> {
    let (input, error, lifecycle) = (input.clone(), error.to_string(), lifecycle.to_string());
    run_tx(repo.db(), RetryPolicy::default(), |tx| {
        let (input, error, lifecycle) = (input.clone(), error.clone(), lifecycle.clone());
        async move {
            record_terminal_failure_in(&tx, &input, &error, &lifecycle, &ts(Utc::now())).await
        }
    })
    .await?;
    Ok(())
}

async fn generation_of(repo: &Repository, kind: &str, owner: &str) -> Result<i64> {
    let (kind, owner) = (kind.to_string(), owner.to_string());
    Ok(run_tx(repo.db(), RetryPolicy::default(), |tx| {
        let (kind, owner) = (kind.clone(), owner.clone());
        async move { desired_generation(&tx, &kind, &owner).await }
    })
    .await?
    .value)
}

async fn fail(
    repo: &Repository,
    claimed: &ClaimedTask,
    input: &PipelineInput,
    error: &str,
) -> Result<()> {
    // Only a refresh still holding its lease records the failure.
    if end_claimed(repo, claimed, Ending::Failed, error).await? {
        owner_state(repo, input, error, "previous").await?;
    }
    Ok(())
}

async fn block(repo: &Repository, claimed: &ClaimedTask, input: &PipelineInput) -> Result<()> {
    owner_state(repo, input, "cancelled; evidence dirty", "pending").await?;
    end_claimed(repo, claimed, Ending::Blocked, "cancelled; evidence dirty").await?;
    Ok(())
}

/// `run_refresh_slice` on SurrealDB. `Ok(None)` when the task was not
/// claimable, yielded, or was superseded.
pub async fn run_refresh_slice(
    repo: &Repository,
    task_id: &str,
    settings: &SettingsFile,
    cancel: &Arc<AtomicBool>,
    validate: SnapshotStep,
    ctx: DerivationContext,
    execute: Execute,
) -> Result<Option<PipelineOutput>> {
    let db = repo.db();
    let now = Utc::now();
    let Some(side) = sidecar(repo, task_id).await? else {
        anyhow::bail!("not an insight refresh task");
    };
    let owner = format!("insight-worker:{}", std::process::id());
    let op_key = format!(
        "refresh-claim:{task_id}:{}",
        now.timestamp_nanos_opt().unwrap_or(0)
    );
    let Some(claimed) = claim_task(
        db,
        crate::tasks::pool_for_operation(OPERATION),
        task_id,
        &owner,
        LEASE_SECS,
        now,
        &op_key,
    )
    .await?
    .value
    else {
        return Ok(None);
    };
    ensure!(
        claimed.operation == OPERATION,
        "not an insight refresh task"
    );
    let input: PipelineInput = serde_json::from_str(&side.input_ref)?;
    ensure!(
        input.surface != "atlas" && input.snapshot.owner_kind != "atlas",
        "Atlas insight refresh retired"
    );
    let (kind, owner_id, generation) = (
        input.snapshot.owner_kind.clone(),
        input.snapshot.owner_id.clone(),
        i64::from(input.snapshot.generation),
    );
    let admission: Result<()> = async {
        run_tx(db, RetryPolicy::default(), |tx| {
            let validate = validate.clone();
            async move { validate(tx, String::new()).await }
        })
        .await?;
        ensure!(
            generation_of(repo, &kind, &owner_id).await? == generation,
            "refresh superseded"
        );
        ensure!(
            input.surface != "article" || article_details_eligible(&input),
            "waiting for complete article details"
        );
        ensure!(
            input_key(&input, settings) == side.input_hash,
            "captured route/preparation contract changed"
        );
        Ok(())
    }
    .await;
    let result = match admission {
        Err(error) => Err(error),
        Ok(()) => {
            let refresh = crate::investigation::insight_execution::slice(execute(input.clone()));
            tokio::pin!(refresh);
            let mut heartbeat = tokio::time::interval(std::time::Duration::from_secs(30));
            heartbeat.tick().await;
            loop {
                tokio::select! {
                    result = &mut refresh => break result,
                    _ = heartbeat.tick() => {
                        let renewed = super::jobs::renew_lease(db, &claimed, LEASE_SECS, Utc::now()).await?.value;
                        ensure!(renewed, "refresh lease lost");
                        if generation_of(repo, &kind, &owner_id).await? != generation {
                            break Err(anyhow!("refresh superseded"));
                        }
                    }
                }
            }
        }
    };
    let (mut input, mut output) = match result {
        Ok(done) => done,
        Err(error) => {
            if error.is::<crate::investigation::insight_execution::SliceYield>() {
                ensure!(
                    end_claimed(repo, &claimed, Ending::Continue, "").await?,
                    "refresh lease lost before continuation"
                );
                return Ok(None);
            }
            if generation_of(repo, &kind, &owner_id).await? != generation {
                let message = match super::refresh::supersession(db, &claimed.id).await? {
                    Some((job, reason)) => format!("Superseded · replaced by {job} · {reason}"),
                    None => "Superseded · replaced by a newer generation".into(),
                };
                end_claimed(repo, &claimed, Ending::Superseded, &message).await?;
                return Ok(None);
            }
            if cancel.load(Ordering::Acquire) {
                block(repo, &claimed, &input).await?;
            } else {
                fail(repo, &claimed, &input, &error.to_string()).await?;
            }
            return Err(error);
        }
    };
    // Reduced and repaired snapshots get distinct publication identities.
    input.snapshot.key = crate::nlp::cache::key(&serde_json::json!([input.snapshot.key, output]));
    for inference in &mut output.inferences {
        inference.snapshot_key = input.snapshot.key.clone();
    }
    for link in &mut output.links {
        link.snapshot_key = input.snapshot.key.clone();
    }
    if cancel.load(Ordering::Acquire) {
        block(repo, &claimed, &input).await?;
        anyhow::bail!("cancelled before insight publication");
    }
    if !output.readiness.publication_eligible {
        let reason = output.readiness.gaps.join("; ");
        fail(repo, &claimed, &input, &reason).await?;
        anyhow::bail!("replacement coverage incomplete: {reason}");
    }
    let input = Arc::new(input);
    let shared = Arc::new(output.clone());
    let publish = derivation_step(input.clone(), shared, None, ctx);
    let published = publish_snapshot(
        db,
        &claimed,
        &input.snapshot,
        &input.snapshot.key,
        "active",
        Utc::now(),
        validate,
        publish,
    )
    .await;
    match published {
        Ok(true) => Ok(Some(output)),
        failure => {
            let error = match failure {
                Err(error) => anyhow!(error.to_string()),
                _ => anyhow!("stale insight snapshot; previous history retained"),
            };
            fail(repo, &claimed, &input, &error.to_string()).await?;
            Err(error)
        }
    }
}

/// One recovery error (`insight.recovery_error`), for the caller to record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryError {
    pub resource: String,
    pub message: String,
}

/// Queued and retry-scheduled refresh task ids, oldest first.
pub async fn queued_refreshes(repo: &Repository) -> Result<Vec<String>> {
    let mut r = repo
        .db()
        .query(
            "SELECT VALUE record::id(id) FROM (SELECT id, created_at FROM task
               WHERE operation = $op AND state IN ['queued', 'retry_scheduled']
               ORDER BY created_at, id)",
        )
        .bind(("op", OPERATION))
        .await?
        .check()?;
    Ok(r.take(0)?)
}

/// The settings a refresh task captured; `fallback` when unreadable.
pub async fn captured_settings(
    repo: &Repository,
    task_id: &str,
    fallback: &SettingsFile,
) -> Result<SettingsFile> {
    Ok(sidecar(repo, task_id)
        .await?
        .and_then(|s| serde_json::from_str(&s.role_snapshot).ok())
        .unwrap_or_else(|| fallback.clone()))
}

/// The admission half of `recover_refreshes`: reschedule each durable
/// article intent from its retained input, then reap expired leases.
/// Returns the queued refresh ids to run (via [`run_refresh_slice`] with
/// [`captured_settings`]) and the per-owner errors.
pub async fn recover_refreshes(
    repo: &Repository,
    settings: &SettingsFile,
) -> Result<(Vec<String>, Vec<RecoveryError>)> {
    let db = repo.db();
    let mut r = db
        .query("SELECT VALUE record::id(id) FROM article_refresh_intent")
        .await?
        .check()?;
    let owners: Vec<String> = r.take(0)?;
    let mut errors = Vec::new();
    for owner in owners {
        let Some(article) = super::atlas::latest_article_by_id(db, &owner).await? else {
            continue;
        };
        let scheduled = async {
            let input = super::pipeline_inputs::article_input(repo, &article, 0, "").await?;
            super::refresh::schedule_article_refresh(db, &owner, Some(&input), settings, false)
                .await?;
            Ok::<_, anyhow::Error>(())
        }
        .await;
        if let Err(error) = scheduled {
            errors.push(RecoveryError {
                resource: owner,
                message: error.to_string(),
            });
        }
    }
    interrupt_expired_leases(db, Utc::now()).await?;
    Ok((queued_refreshes(repo).await?, errors))
}
