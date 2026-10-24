//! The Intel brief stage chain on SurrealDB (`intel_recon::brief_stages`
//! claim, run and commit, and `brief_pipeline::ensure_chain` admission).
//!
//! The chain itself is `jobs::ensure_chain` (one `intel_brief` job, four
//! tasks with their dependencies) and stages commit through the fenced
//! `jobs::commit_stage`. This module keeps what the SQLite chain stores in
//! its own tables: the captured stage inputs (`brief_stage_input`, the root
//! tasks' `input_ref` and every task's role snapshot), per-item checkpoints
//! (`brief_checkpoint`), fidelity receipts (`brief_receipt`), the live
//! activity label (`brief_activity`) and the committed item count and
//! reason (`brief_stage_meta`). [`SurrealStages`] is the executor's
//! [`StageStore`] over them.
//!
//! A stage runs once per lease: the heartbeat renews the lease while the
//! model works, a lost renewal cancels the stage, and the commit is fenced
//! by owner and epoch and receipted per task and epoch, so a delayed or
//! retried commit never publishes twice and a second worker never runs a
//! stage that is still leased.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use chrono::Utc;
use serde_json::{json, Value};
use surrealdb::types::SurrealValue;

use super::codec;
use super::jobs::{self, rid, ClaimedTask as Leased, Publish, StageSpec};
use super::repo::Repository;
use super::tx::{run_tx, RetryPolicy};
use crate::intel_recon::brief_context::ContextActor;
use crate::intel_recon::brief_pipeline::{self, BriefInputs};
use crate::intel_recon::brief_stages::{
    execute, stage_error_category, BoxFut, ExecConfig, StageBackend, StageContext, StageOutcome,
    StageStore,
};
use crate::store::AtlasArticleRow;
use crate::tasks::ClaimedTask;

pub const SCHEMA: &str = "
DEFINE TABLE IF NOT EXISTS brief_stage_input SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS input_json ON brief_stage_input TYPE string;
DEFINE FIELD IF NOT EXISTS role_snapshot ON brief_stage_input TYPE string;
DEFINE TABLE IF NOT EXISTS brief_checkpoint SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS job_id ON brief_checkpoint TYPE string;
DEFINE FIELD IF NOT EXISTS stage ON brief_checkpoint TYPE string;
DEFINE FIELD IF NOT EXISTS input_hash ON brief_checkpoint TYPE string;
DEFINE FIELD IF NOT EXISTS output_json ON brief_checkpoint TYPE string;
DEFINE INDEX IF NOT EXISTS brief_checkpoint_stage ON brief_checkpoint FIELDS job_id, stage;
DEFINE TABLE IF NOT EXISTS brief_receipt SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS value ON brief_receipt TYPE string;
DEFINE TABLE IF NOT EXISTS brief_activity SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS label ON brief_activity TYPE string;
DEFINE FIELD IF NOT EXISTS updated_at ON brief_activity TYPE string;
DEFINE TABLE IF NOT EXISTS brief_stage_meta SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS item_count ON brief_stage_meta TYPE int;
DEFINE FIELD IF NOT EXISTS empty_reason ON brief_stage_meta TYPE string;
";

/// Task id of one stage in a SurrealDB chain (`jobs::ensure_chain`).
pub fn task_id(job_id: &str, operation: &str) -> String {
    codec::encode(&[job_id, operation])
}

/// `brief_pipeline::ensure_chain` on the repository: attach to the
/// article's chain for unchanged inputs, otherwise supersede it and admit
/// the next generation with its captured stage inputs.
pub async fn ensure(repo: &Repository, inputs: &BriefInputs) -> Result<jobs::Chain> {
    let specs: Vec<StageSpec<'_>> = brief_pipeline::STAGES
        .iter()
        .map(|(op, _)| StageSpec {
            operation: op,
            deps: brief_pipeline::prerequisites(op)
                .iter()
                .filter_map(|d| brief_pipeline::STAGES.iter().position(|(s, _)| s == d))
                .collect(),
        })
        .collect();
    let chain = jobs::ensure_chain(
        repo.db(),
        &inputs.article_id,
        &inputs.input_revision,
        &specs,
        Utc::now(),
    )
    .await
    .map_err(|e| anyhow!("ensure brief chain {}: {e}", inputs.article_id))?
    .value;
    if chain.created {
        let force_body = inputs.input_revision.contains(":reload:");
        for (index, (operation, _)) in brief_pipeline::STAGES.iter().enumerate() {
            let input = if brief_pipeline::prerequisites(operation).is_empty() {
                json!({
                    "article_id": inputs.article_id,
                    "force_body": *operation == "intel_brief_body" && force_body,
                    "metadata": inputs.metadata,
                    "node_revision": inputs.stage_revisions.get(index).cloned().unwrap_or_default(),
                })
                .to_string()
            } else {
                String::new()
            };
            repo.db()
                .query("UPSERT $r SET input_json = $i, role_snapshot = $s RETURN NONE")
                .bind((
                    "r",
                    rid("brief_stage_input", &task_id(&chain.job_id, operation)),
                ))
                .bind(("i", input))
                .bind(("s", inputs.role_snapshot.clone()))
                .await?
                .check()?;
        }
    }
    Ok(chain)
}

#[derive(SurrealValue)]
struct Input {
    input_json: String,
    role_snapshot: String,
}

async fn input(repo: &Repository, task: &str) -> Result<Option<Input>> {
    let mut r = repo
        .db()
        .query("SELECT input_json, role_snapshot FROM ONLY $r")
        .bind(("r", rid("brief_stage_input", task)))
        .await?
        .check()?;
    Ok(r.take(0)?)
}

/// The executor's view of a leased task (its own captured input as
/// `input_ref`).
async fn executor_task(repo: &Repository, leased: &Leased) -> Result<ClaimedTask> {
    let own = input(repo, &leased.id).await?;
    Ok(ClaimedTask {
        id: leased.id.clone(),
        job_id: leased.job_id.clone(),
        operation: leased.operation.clone(),
        owner: leased.lease_owner.clone(),
        epoch: leased.lease_epoch,
        attempt: u32::try_from(leased.attempts).unwrap_or(0),
        max_attempts: 1,
        input_ref: own.map(|i| i.input_json).unwrap_or_default(),
        input_hash: leased.input_hash.clone(),
        source_revision: String::new(),
        attempt_id: String::new(),
        started_at: Utc::now().to_rfc3339(),
    })
}

#[derive(SurrealValue)]
struct Run {
    article_id: String,
    generation: i64,
}

async fn run_of(repo: &Repository, job_id: &str) -> Result<Run> {
    let mut r = repo
        .db()
        .query("SELECT article_id, generation FROM brief_run WHERE job = $j LIMIT 1")
        .bind(("j", rid("job", job_id)))
        .await?
        .check()?;
    let rows: Vec<Run> = r.take(0)?;
    rows.into_iter()
        .next()
        .ok_or_else(|| anyhow!("brief run for {job_id} missing"))
}

/// [`StageStore`] on the repository.
pub struct SurrealStages {
    repo: Repository,
}

impl SurrealStages {
    pub fn new(repo: &Repository) -> Self {
        Self { repo: repo.clone() }
    }
}

impl StageStore for SurrealStages {
    fn load<'a>(
        &'a self,
        claimed: &'a ClaimedTask,
    ) -> BoxFut<'a, Result<(StageContext, AtlasArticleRow)>> {
        Box::pin(async move {
            let run = run_of(&self.repo, &claimed.job_id).await?;
            let route_snapshot = input(&self.repo, &claimed.id)
                .await?
                .map(|i| i.role_snapshot)
                .unwrap_or_default();
            // Roots carry the captured metadata; descendants read the Claims root's.
            let own: Value = serde_json::from_str(&claimed.input_ref).unwrap_or(Value::Null);
            let metadata_snapshot = if own["metadata"].is_object() {
                own["metadata"].clone()
            } else {
                input(&self.repo, &task_id(&claimed.job_id, "intel_brief_claims"))
                    .await?
                    .and_then(|i| serde_json::from_str::<Value>(&i.input_json).ok())
                    .map(|v| v["metadata"].clone())
                    .unwrap_or(Value::Null)
            };
            let article = super::atlas::latest_article_by_id(self.repo.db(), &run.article_id)
                .await?
                .ok_or_else(|| anyhow!("article {} no longer exists", run.article_id))?;
            Ok((
                StageContext {
                    job_id: claimed.job_id.clone(),
                    task_id: claimed.id.clone(),
                    job_generation: run.generation,
                    lease_epoch: claimed.epoch,
                    metadata_snapshot,
                    route_snapshot,
                    ledger: Arc::default(),
                },
                article,
            ))
        })
    }

    fn stage_output<'a>(
        &'a self,
        job_id: &'a str,
        operation: &'a str,
    ) -> BoxFut<'a, Result<Value>> {
        Box::pin(async move {
            let mut r = self
                .repo
                .db()
                .query("SELECT VALUE output_json FROM ONLY $o")
                .bind(("o", rid("stage_output", &task_id(job_id, operation))))
                .await?
                .check()?;
            let raw: Option<String> = r.take(0)?;
            let raw = raw.ok_or_else(|| anyhow!("{operation} output missing"))?;
            Ok(serde_json::from_str(&raw)?)
        })
    }

    fn article_actors<'a>(
        &'a self,
        article: &'a AtlasArticleRow,
    ) -> BoxFut<'a, Result<Vec<ContextActor>>> {
        Box::pin(async move {
            Ok(
                super::atlas_metadata::metadata(self.repo.db(), &article.run_id, &article.id)
                    .await?
                    .map(|metadata| {
                        crate::atlas_metadata::article_actors(article, &metadata)
                            .iter()
                            .map(ContextActor::from)
                            .collect()
                    })
                    .unwrap_or_default(),
            )
        })
    }

    fn checkpoint_get<'a>(
        &'a self,
        job_id: &'a str,
        stage: &'a str,
        item: &'a str,
        input_hash: &'a str,
    ) -> BoxFut<'a, Result<Option<Value>>> {
        Box::pin(async move {
            let mut r = self
                .repo
                .db()
                .query("SELECT VALUE output_json FROM ONLY $c WHERE input_hash = $h")
                .bind((
                    "c",
                    rid("brief_checkpoint", &codec::encode(&[job_id, stage, item])),
                ))
                .bind(("h", input_hash.to_string()))
                .await?
                .check()?;
            let raw: Option<String> = r.take(0)?;
            Ok(raw.and_then(|s| serde_json::from_str(&s).ok()))
        })
    }

    fn checkpoint_put<'a>(
        &'a self,
        job_id: &'a str,
        stage: &'a str,
        item: &'a str,
        input_hash: &'a str,
        output: &'a Value,
    ) -> BoxFut<'a, Result<()>> {
        Box::pin(async move {
            self.repo
                .db()
                .query(
                    "UPSERT $c SET job_id = $j, stage = $s, input_hash = $h, output_json = $o \
                     RETURN NONE",
                )
                .bind((
                    "c",
                    rid("brief_checkpoint", &codec::encode(&[job_id, stage, item])),
                ))
                .bind(("j", job_id.to_string()))
                .bind(("s", stage.to_string()))
                .bind(("h", input_hash.to_string()))
                .bind(("o", output.to_string()))
                .await?
                .check()?;
            Ok(())
        })
    }

    fn receipt_get<'a>(&'a self, key: &'a str) -> BoxFut<'a, Result<Option<String>>> {
        Box::pin(async move {
            let mut r = self
                .repo
                .db()
                .query("SELECT VALUE value FROM ONLY $r")
                .bind(("r", rid("brief_receipt", key)))
                .await?
                .check()?;
            Ok(r.take(0)?)
        })
    }

    fn receipt_set<'a>(&'a self, key: &'a str, value: &'a str) -> BoxFut<'a, Result<()>> {
        Box::pin(async move {
            self.repo
                .db()
                .query("UPSERT $r SET value = $v RETURN NONE")
                .bind(("r", rid("brief_receipt", key)))
                .bind(("v", value.to_string()))
                .await?
                .check()?;
            Ok(())
        })
    }

    fn activity(&self, task: &str, label: &str) {
        let (repo, task, label) = (self.repo.clone(), task.to_string(), label.to_string());
        super::runtime::spawn(async move {
            let _ = set_activity(&repo, &task, &label).await;
        });
    }
}

async fn set_activity(repo: &Repository, task: &str, label: &str) -> Result<()> {
    repo.db()
        .query("UPSERT $r SET label = $l, updated_at = $now RETURN NONE")
        .bind(("r", rid("brief_activity", task)))
        .bind(("l", label.to_string()))
        .bind(("now", Utc::now().to_rfc3339()))
        .await?
        .check()?;
    Ok(())
}

/// The live activity label of a stage task (`brief_pipeline::set_activity`).
pub async fn activity(repo: &Repository, task: &str) -> Result<Option<String>> {
    let mut r = repo
        .db()
        .query("SELECT VALUE label FROM ONLY $r")
        .bind(("r", rid("brief_activity", task)))
        .await?
        .check()?;
    Ok(r.take(0)?)
}

/// Reap expired leases and claim one ready stage.
pub async fn claim(
    repo: &Repository,
    owner: &str,
    config: &ExecConfig,
) -> Result<Option<(Leased, ClaimedTask)>> {
    jobs::interrupt_expired_leases(repo.db(), Utc::now())
        .await
        .map_err(|e| anyhow!("reap brief leases: {e}"))?;
    let op_key = format!(
        "brief-claim:{owner}:{}",
        Utc::now().timestamp_nanos_opt().unwrap_or_default()
    );
    let leased = jobs::claim_next(
        repo.db(),
        jobs::POOL_INTEL_BRIEF,
        owner,
        config.lease_secs,
        Utc::now(),
        &op_key,
    )
    .await
    .map_err(|e| anyhow!("claim brief stage: {e}"))?
    .value;
    match leased {
        Some(leased) => {
            let task = executor_task(repo, &leased).await?;
            Ok(Some((leased, task)))
        }
        None => Ok(None),
    }
}

/// Fenced stage commit: output, completion and the stage's item count, once
/// per task and epoch. False (nothing written) when the lease is lost or the
/// chain was cancelled.
pub async fn commit(
    repo: &Repository,
    leased: &Leased,
    output: &Value,
    items: usize,
    reason: &str,
) -> Result<bool> {
    let json = output.to_string();
    let hash = crate::nlp::cache::hash(&json);
    let op_key = format!("brief-commit:{}:{}", leased.id, leased.lease_epoch);
    let published = jobs::commit_stage(repo.db(), leased, &json, &hash, Utc::now(), &op_key)
        .await
        .map_err(|e| anyhow!("commit {}: {e}", leased.id))?
        .value;
    if published != Publish::Committed {
        return Ok(false);
    }
    let last = brief_pipeline::STAGES.last().map(|(op, _)| *op) == Some(leased.operation.as_str());
    // A completed-with-warnings stage keeps its checkpoints so Retry re-asks
    // only the items that failed.
    repo.db()
        .query(
            "UPSERT $m SET item_count = $n, empty_reason = $reason RETURN NONE;
             IF !string::starts_with($reason, 'warning:') {
                 DELETE brief_checkpoint WHERE job_id = $job AND stage = $stage;
             };
             IF $last {
                 UPDATE brief_run SET state = 'completed', updated_at = time::now() WHERE job = $j;
                 UPDATE $j SET state = 'completed', finished_at = time::now(),
                   updated_at = time::now() WHERE state = 'active' OR state = 'queued';
             };",
        )
        .bind(("m", rid("brief_stage_meta", &leased.id)))
        .bind(("n", items as i64))
        .bind(("reason", reason.to_string()))
        .bind(("job", leased.job_id.clone()))
        .bind(("stage", leased.operation.clone()))
        .bind(("last", last))
        .bind(("j", rid("job", &leased.job_id)))
        .await?
        .check()?;
    Ok(true)
}

/// Fenced stage failure. The model chain is the single retry owner, so the
/// stage waits for an explicit Retry. False when the lease was lost.
pub async fn fail(repo: &Repository, leased: &Leased, error: &anyhow::Error) -> Result<bool> {
    let now = jobs::ts(Utc::now());
    let (category, message) = (
        stage_error_category(error).as_str().to_string(),
        error.to_string(),
    );
    let done = run_tx(repo.db(), RetryPolicy::default(), |tx| {
        let (now, category, message) = (now.clone(), category.clone(), message.clone());
        async move {
            jobs::bump(&tx, "job_guard", &leased.root_id).await?;
            match jobs::fence(&tx, leased, &now).await? {
                Some(f) if jobs::holds(&f, leased) => {}
                _ => return Ok(false),
            }
            jobs::run(
                &tx,
                tx.query(
                    "UPDATE $t SET state = 'failed', lease_owner = '', process_owner = '',
                       lease_until = NONE, finished_at = <datetime>$now,
                       error_category = $cat, error_message = $msg,
                       updated_at = <datetime>$now RETURN NONE;
                     LET $open = SELECT VALUE id FROM attempt WHERE task = $t AND epoch = $e
                       AND finished_at = NONE;
                     UPDATE $open SET finished_at = <datetime>$now, outcome = 'failed' RETURN NONE;",
                )
                .bind(("t", rid("task", &leased.id)))
                .bind(("now", now.clone()))
                .bind(("e", leased.lease_epoch))
                .bind(("cat", category))
                .bind(("msg", message)),
            )
            .await?;
            Ok(true)
        }
    })
    .await
    .map_err(|e| anyhow!("fail {}: {e}", leased.id))?;
    Ok(done.value)
}

async fn renew(repo: &Repository, leased: &Leased, lease: i64) -> bool {
    for _ in 0..5 {
        match jobs::renew_lease(repo.db(), leased, lease, Utc::now()).await {
            Ok(ok) => return ok.value,
            Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
        }
    }
    false
}

/// Run one claimed stage through its fenced commit or failure, renewing the
/// lease while it runs (`brief_stages::run_claimed`).
pub async fn run_claimed(
    repo: &Repository,
    leased: Leased,
    claimed: ClaimedTask,
    backend: &mut dyn StageBackend,
    parent_cancel: &Arc<AtomicBool>,
    config: &ExecConfig,
) -> Result<StageOutcome> {
    let operation = claimed.operation.clone();
    let cancel = Arc::new(AtomicBool::new(false));
    let lost = Arc::new(AtomicBool::new(false));
    let done = Arc::new(AtomicBool::new(false));
    let heartbeat = {
        let (repo, l, cancel, lost, done, parent) = (
            repo.clone(),
            leased.clone(),
            cancel.clone(),
            lost.clone(),
            done.clone(),
            parent_cancel.clone(),
        );
        let (interval, lease) = (config.heartbeat, config.lease_secs);
        tokio::spawn(async move {
            while !done.load(Ordering::Acquire) {
                let step = Duration::from_millis(50).min(interval);
                let mut waited = Duration::ZERO;
                while waited < interval && !done.load(Ordering::Acquire) {
                    if parent.load(Ordering::Acquire) {
                        cancel.store(true, Ordering::Release);
                    }
                    tokio::time::sleep(step).await;
                    waited += step;
                }
                if done.load(Ordering::Acquire) {
                    break;
                }
                let mut renewed = renew(&repo, &l, lease).await;
                if !renewed {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    renewed = renew(&repo, &l, lease).await;
                }
                if !renewed {
                    lost.store(true, Ordering::Release);
                    cancel.store(true, Ordering::Release);
                    break;
                }
            }
        })
    };
    let stages = SurrealStages::new(repo);
    let watch = cancel.clone();
    let result = tokio::select! {
        r = execute(&stages, &claimed, backend, &cancel) => r,
        _ = async move {
            while !watch.load(Ordering::Acquire) {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        } => Err(anyhow!("cancelled")),
    };
    // Commit while the heartbeat still covers the lease, then stop it.
    let outcome = match result {
        _ if lost.load(Ordering::Acquire) || cancel.load(Ordering::Acquire) => {
            StageOutcome::Stale { operation }
        }
        Ok((value, items, reason)) => {
            if commit(repo, &leased, &value, items, &reason).await? {
                StageOutcome::Committed { operation, items }
            } else {
                StageOutcome::Stale { operation }
            }
        }
        Err(error) => {
            if fail(repo, &leased, &error).await? {
                StageOutcome::Failed {
                    operation,
                    reason: error.to_string(),
                }
            } else {
                StageOutcome::Stale { operation }
            }
        }
    };
    done.store(true, Ordering::Release);
    let _ = heartbeat.await;
    let _ = set_activity(repo, &claimed.id, "").await;
    Ok(outcome)
}

/// Serial drain with one backend (`brief_stages::drain`): run ready stages
/// until none is claimable; stops after a stale outcome.
pub async fn drain(
    repo: &Repository,
    owner: &str,
    backend: &mut dyn StageBackend,
    cancel: &Arc<AtomicBool>,
    config: &ExecConfig,
) -> Result<Vec<StageOutcome>> {
    let mut outcomes = Vec::new();
    while !cancel.load(Ordering::Acquire) {
        let Some((leased, claimed)) = claim(repo, owner, config).await? else {
            break;
        };
        let outcome = run_claimed(repo, leased, claimed, backend, cancel, config).await?;
        let stop = matches!(outcome, StageOutcome::Stale { .. });
        outcomes.push(outcome);
        if stop {
            break;
        }
    }
    Ok(outcomes)
}

/// Committed item count and reason of a stage task.
pub async fn stage_meta(repo: &Repository, task: &str) -> Result<Option<(i64, String)>> {
    #[derive(SurrealValue)]
    struct Meta {
        item_count: i64,
        empty_reason: String,
    }
    let mut r = repo
        .db()
        .query("SELECT item_count, empty_reason FROM ONLY $m")
        .bind(("m", rid("brief_stage_meta", task)))
        .await?
        .check()?;
    let meta: Option<Meta> = r.take(0)?;
    Ok(meta.map(|m| (m.item_count, m.empty_reason)))
}
