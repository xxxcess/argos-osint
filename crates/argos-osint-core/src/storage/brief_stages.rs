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
        mirror_runs(repo, &inputs.article_id).await?;
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

/// Mirror the article's chain runs into `intel_brief_run`, the fence the
/// retained Intel display seeds read (`storage::brief::record_run`).
async fn mirror_runs(repo: &Repository, article_id: &str) -> Result<()> {
    #[derive(SurrealValue)]
    struct Row {
        job_id: String,
        generation: i64,
        state: String,
    }
    let mut r = repo
        .db()
        .query("SELECT record::id(job) AS job_id, generation, state FROM brief_run WHERE article_id = $a")
        .bind(("a", article_id.to_string()))
        .await?
        .check()?;
    let rows: Vec<Row> = r.take(0)?;
    for row in rows {
        super::brief::record_run(repo, &row.job_id, article_id, row.generation, &row.state).await?;
    }
    Ok(())
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
    let run = run_of(repo, &leased.job_id).await?;
    if last {
        super::brief::record_run(
            repo,
            &leased.job_id,
            &run.article_id,
            run.generation,
            "completed",
        )
        .await?;
    }
    publish_stage(repo, &leased.job_id, &leased.operation, &run).await?;
    Ok(true)
}

/// `brief_pipeline::publish_completed_stage`: merge a completed Claims,
/// Inferences or Links stage into the displayed brief while its run is the
/// article's newest live one, replace the links-stage leads, and bind the
/// stage's canonical index state.
///
/// Canonical Brain publication runs through [`super::brief_publish`]; its
/// failure is recorded in the bound state and never fails the stage.
async fn publish_stage(repo: &Repository, job_id: &str, operation: &str, run: &Run) -> Result<()> {
    let stage = super::brief::Stage::from_operation(operation);
    if stage == super::brief::Stage::Other {
        return Ok(());
    }
    let mut r = repo
        .db()
        .query(
            "RETURN count(SELECT 1 FROM brief_run WHERE article_id = $a AND generation > $g);
             SELECT VALUE state FROM brief_run WHERE job = $j;",
        )
        .bind(("a", run.article_id.clone()))
        .bind(("g", run.generation))
        .bind(("j", rid("job", job_id)))
        .await?
        .check()?;
    let newer: Option<i64> = r.take(0)?;
    let states: Vec<String> = r.take(1)?;
    let live = states
        .first()
        .is_some_and(|s| matches!(s.as_str(), "active" | "completed" | "failed"));
    if newer.unwrap_or(0) > 0 || !live {
        return Ok(());
    }
    let stages = SurrealStages::new(repo);
    let output = |op: &'static str| {
        let stages = &stages;
        async move {
            stages
                .stage_output(job_id, op)
                .await
                .unwrap_or_else(|_| json!({}))
        }
    };
    let body = output("intel_brief_body").await;
    let claims = output("intel_brief_claims").await;
    let inferences = output("intel_brief_inferences").await;
    let links = output("intel_brief_links").await;
    let own = stages.stage_output(job_id, operation).await?;
    let snapshot_hash = crate::nlp::cache::hash(&json!([operation, own]).to_string());
    let written = super::brief::publish(
        repo,
        &super::brief::Publication {
            article_id: run.article_id.clone(),
            job_id: job_id.to_string(),
            generation: run.generation,
            contract: brief_pipeline::CONTRACT.to_string(),
            body_hash: body["content_hash"].as_str().unwrap_or("").to_string(),
            snapshot_hash: snapshot_hash.clone(),
            claims: claims.clone(),
            inferences: inferences.clone(),
            leads: links["leads"].clone(),
            stage,
        },
    )
    .await?;
    if !written {
        return Ok(());
    }
    if stage == super::brief::Stage::Links {
        let texts: std::collections::HashMap<&str, &str> = inferences["inferences"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|i| Some((i["id"].as_str()?, i["text"].as_str()?)))
            .collect();
        let leads: Vec<super::brief::Lead> = links["leads"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|lead| {
                let inference = lead["inference_id"].as_str().unwrap_or("");
                let candidate = lead["article_id"].as_str().unwrap_or("");
                super::brief::Lead {
                    id: format!(
                        "{}:{}:{inference}:{candidate}",
                        run.article_id, run.generation
                    ),
                    inference_id: inference.to_string(),
                    inference_text: texts.get(inference).copied().unwrap_or("").to_string(),
                    candidate_article_id: candidate.to_string(),
                    candidate_run_id: lead["article_run_id"].as_str().unwrap_or("").to_string(),
                    candidate_metadata_revision: lead["metadata_revision"]
                        .as_str()
                        .unwrap_or("")
                        .to_string(),
                    interpretation: lead["why"].as_str().unwrap_or("").to_string(),
                    verification_needed: lead["verification_needed"]
                        .as_str()
                        .unwrap_or("")
                        .to_string(),
                    citations_json: lead["metadata_evidence"].to_string(),
                }
            })
            .collect();
        super::brief::replace_leads(repo, &run.article_id, job_id, run.generation, &leads).await?;
    }
    let prepared = super::brief_publish::prepare(
        repo,
        &run.article_id,
        &body,
        &claims,
        &inferences,
        operation,
    )
    .await
    .unwrap_or_else(|error| Err(format!("canonical inputs not prepared: {error}")));
    let state = super::brief_publish::publish_stage(
        repo,
        &run.article_id,
        job_id,
        operation,
        &prepared,
        &snapshot_hash,
    )
    .await?;
    super::brief::put_stage_receipt(repo, job_id, operation, &state).await?;
    super::brief::put_index_state(repo, &run.article_id, &state).await?;
    Ok(())
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
    struct HeartbeatGuard {
        done: Arc<AtomicBool>,
        handle: Option<tokio::task::JoinHandle<()>>,
    }

    impl Drop for HeartbeatGuard {
        fn drop(&mut self) {
            self.done.store(true, Ordering::Release);
            if let Some(handle) = self.handle.take() {
                handle.abort();
            }
        }
    }

    let mut heartbeat_guard = HeartbeatGuard {
        done: done.clone(),
        handle: Some(heartbeat),
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
    if let Some(h) = heartbeat_guard.handle.take() {
        let _ = h.await;
    }
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

/// Production stage backend for the SurrealDB drainer: the configured
/// provider routes of [`RuntimeBackend`](crate::intel_recon::brief_stages::RuntimeBackend)
/// without its SQLite file (no telemetry rows), with the body and related
/// reads served by the repository.
///
/// Body acquisition, cleanup and the retrieval-attempt ledger use BodyDb::Surreal.
pub struct SurrealRuntimeBackend {
    inner: crate::intel_recon::brief_stages::RuntimeBackend,
    repo: Repository,
    handle: tokio::runtime::Handle,
}

impl SurrealRuntimeBackend {
    /// Configured settings and keys; must be built inside the runtime that
    /// drives `repo`.
    pub fn load(repo: Repository) -> Self {
        Self {
            inner: crate::intel_recon::brief_stages::RuntimeBackend::load(std::path::Path::new("")),
            repo,
            handle: tokio::runtime::Handle::current(),
        }
    }
}

/// Clean artifact stored for an article's newest body version.
pub async fn clean_artifact(
    repo: &Repository,
    article_id: &str,
) -> Result<Option<crate::intel_recon::clean_body::CleanArtifact>> {
    let Some(body) = super::pipeline_inputs::article_body_for_article(repo, article_id).await?
    else {
        return Ok(None);
    };
    let key = format!("intel-clean-body:{article_id}:{}", body.body_version);
    Ok(super::pipeline_inputs::app_state_get(repo, &key)
        .await?
        .and_then(|json| serde_json::from_str(&json).ok()))
}

/// Up to 50 articles of the same run paired with `article` by Related,
/// strongest first.
pub async fn related(repo: &Repository, article: &AtlasArticleRow) -> Result<Vec<AtlasArticleRow>> {
    #[derive(SurrealValue)]
    struct Pair {
        left_id: String,
        right_id: String,
    }
    let mut r = repo
        .db()
        .query(
            "SELECT left_id, right_id, rarity, shared_count FROM atlas_related_article
               WHERE run_id = $run AND (left_id = $a OR right_id = $a)
               ORDER BY rarity DESC, shared_count DESC LIMIT 50",
        )
        .bind(("run", article.run_id.clone()))
        .bind(("a", article.id.clone()))
        .await?
        .check()?;
    let pairs: Vec<Pair> = r.take(0)?;
    let mut out = Vec::new();
    for p in pairs {
        let other = if p.left_id == article.id {
            p.right_id
        } else {
            p.left_id
        };
        if let Some(row) = super::atlas::article(repo.db(), &article.run_id, &other)
            .await
            .map_err(|e| anyhow!("related article: {e}"))?
        {
            out.push(row);
        }
    }
    Ok(out)
}

impl StageBackend for SurrealRuntimeBackend {
    fn prepare_body<'a>(
        &'a mut self,
        ctx: &'a StageContext,
        article: &'a AtlasArticleRow,
        force: bool,
        cancel: &'a Arc<AtomicBool>,
    ) -> BoxFut<'a, Result<crate::intel_recon::clean_body::CleanArtifact>> {
        Box::pin(async move {
            self.inner
                .prepare_body_on(
                    crate::intel_recon::body_db::BodyDb::Surreal(&self.repo),
                    ctx,
                    article,
                    force,
                    cancel,
                )
                .await
        })
    }

    fn assess<'a>(
        &'a mut self,
        ctx: &'a StageContext,
        items: Vec<crate::intel_recon::brief_fidelity::FidelityItem>,
        cancel: &'a Arc<AtomicBool>,
    ) -> BoxFut<'a, Result<std::collections::HashMap<String, String>>> {
        self.inner.assess(ctx, items, cancel)
    }

    fn classify_context<'a>(
        &mut self,
        ctx: &'a StageContext,
        batch: crate::intel_recon::brief_context::ContextBatch,
        cancel: &'a Arc<AtomicBool>,
    ) -> BoxFut<'a, Result<std::collections::HashMap<String, String>>> {
        self.inner.classify_context(ctx, batch, cancel)
    }

    fn related(&mut self, article: &AtlasArticleRow) -> Result<Vec<AtlasArticleRow>> {
        // Sync boundary inside an async stage: run the read on a scoped
        // thread driven by the repository's runtime.
        let (repo, handle) = (self.repo.clone(), self.handle.clone());
        std::thread::scope(|s| {
            s.spawn(|| handle.block_on(related(&repo, article)))
                .join()
                .map_err(|_| anyhow!("related read panicked"))?
        })
    }

    fn focused_oversized(&self) -> bool {
        self.inner.focused_oversized()
    }

    fn generate<'a>(
        &mut self,
        request: crate::intel_recon::brief_stages::StageRequest,
        cancel: &'a Arc<AtomicBool>,
    ) -> BoxFut<'a, Result<String>> {
        self.inner.generate(request, cancel)
    }
}

/// Up to `slots` stages at once, each with its own backend, until no
/// stage is ready or `cancel` is set.
pub async fn drain_parallel(
    repo: &Repository,
    owner: &str,
    make_backend: impl Fn() -> Box<dyn StageBackend + Send>,
    cancel: &Arc<AtomicBool>,
    config: &ExecConfig,
    slots: usize,
) -> Result<usize> {
    let mut ran = 0;
    loop {
        let mut set = tokio::task::JoinSet::new();
        for _ in 0..slots.max(1) {
            if cancel.load(Ordering::Acquire) {
                break;
            }
            let Some((leased, claimed)) = claim(repo, owner, config).await? else {
                break;
            };
            let (repo, cancel, config) = (repo.clone(), cancel.clone(), config.clone());
            let mut backend = make_backend();
            set.spawn(async move {
                run_claimed(&repo, leased, claimed, backend.as_mut(), &cancel, &config).await
            });
        }
        if set.is_empty() {
            return Ok(ran);
        }
        let mut first_err = None;
        while let Some(joined) = set.join_next().await {
            match joined
                .map_err(|e| anyhow!("brief stage task: {e}"))
                .and_then(|r| r)
            {
                Ok(_) => ran += 1,
                Err(e) => {
                    first_err.get_or_insert(e);
                }
            }
        }
        if let Some(e) = first_err {
            return Err(e);
        }
    }
}

/// `ensure_intel_brief_pipeline` on the repository: the article's chain job
/// for its current inputs (attached or newly admitted).
pub async fn ensure_for_article(
    repo: &Repository,
    article: &AtlasArticleRow,
    settings: &crate::provider::SettingsFile,
) -> Result<String> {
    use crate::intel_recon::brief_stages as bs;
    let inputs = BriefInputs {
        article_id: article.id.clone(),
        input_revision: bs::input_revision(article, settings),
        role_snapshot: serde_json::to_string(&settings.defaults).unwrap_or_default(),
        title: article.title.clone(),
        stage_revisions: bs::stage_revisions(article, settings),
        metadata: bs::metadata_snapshot(article),
    };
    Ok(ensure(repo, &inputs).await?.job_id)
}

/// `brief_pipeline::stage_views` for a repository chain.
pub async fn stage_views(
    repo: &Repository,
    job_id: &str,
) -> Result<Vec<brief_pipeline::StageView>> {
    #[derive(SurrealValue)]
    struct TaskRow {
        operation: String,
        state: String,
        error_message: String,
    }
    let mut r = repo
        .db()
        .query("SELECT operation, state, error_message FROM task WHERE job = $j")
        .bind(("j", rid("job", job_id)))
        .await?
        .check()?;
    let tasks: Vec<TaskRow> = r.take(0)?;
    let by_op: std::collections::HashMap<String, (String, String)> = tasks
        .into_iter()
        .map(|t| (t.operation, (t.state, t.error_message)))
        .collect();
    let mut rows = std::collections::HashMap::new();
    for (op, (state, error)) in &by_op {
        let id = task_id(job_id, op);
        let deps = brief_pipeline::prerequisites(op)
            .iter()
            .filter_map(|d| by_op.get(*d).map(|(s, _)| (d.to_string(), s.clone())))
            .collect();
        let output = if state == "completed" {
            stage_meta(repo, &id).await?
        } else {
            None
        };
        let activity = if state == "running" {
            activity(repo, &id).await?
        } else {
            None
        };
        rows.insert(
            op.clone(),
            brief_pipeline::StageRow {
                state: state.clone(),
                error: error.clone(),
                deps,
                output,
                activity,
            },
        );
    }
    brief_pipeline::stage_views_from(|op| Ok(rows.remove(op)))
}

/// `brief_pipeline::stage_outputs` for a repository chain.
pub async fn stage_outputs(
    repo: &Repository,
    job_id: &str,
) -> Result<std::collections::HashMap<String, Value>> {
    #[derive(SurrealValue)]
    struct Out {
        stage: String,
        output_json: String,
    }
    let mut r = repo
        .db()
        .query("SELECT stage, output_json FROM stage_output WHERE task.job = $j")
        .bind(("j", rid("job", job_id)))
        .await?
        .check()?;
    let rows: Vec<Out> = r.take(0)?;
    Ok(rows
        .into_iter()
        .filter_map(|o| Some((o.stage, serde_json::from_str(&o.output_json).ok()?)))
        .collect())
}
