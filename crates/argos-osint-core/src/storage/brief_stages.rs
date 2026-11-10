//! The Intel brief stage chain on SurrealDB (`intel_recon::brief_stages`
//! claim, run and commit, and `brief_pipeline::ensure_chain` admission).
//!
//! The chain itself is `jobs::ensure_chain` (one `intel_brief` job, four
//! tasks with their dependencies) and stages commit through the fenced
//! `jobs::commit_stage`. This module keeps what the legacy store chain stores in
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
use super::tx::{run_tx, RetryPolicy, Tx, TxError};
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

async fn latest_run(repo: &Repository, article_id: &str) -> Result<Option<jobs::ChainRun>> {
    let mut response = repo
        .db()
        .query(
            "SELECT record::id(job) AS job_id, generation, state, input_revision FROM brief_run \
             WHERE article_id = $a ORDER BY generation DESC LIMIT 1",
        )
        .bind(("a", article_id.to_string()))
        .await?
        .check()?;
    let rows: Vec<jobs::ChainRun> = response.take(0)?;
    Ok(rows.into_iter().find(|run| run.state != "superseded"))
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
    let force_body = inputs.input_revision.contains(":reload:");
    let now = Utc::now();
    let committed = run_tx(repo.db(), RetryPolicy::default(), |tx| {
        let specs = &specs;
        async move {
            let admission = jobs::ensure_chain_in(
                &tx,
                &inputs.article_id,
                &inputs.input_revision,
                specs,
                now,
                |saved, requested| {
                    saved == requested || (!force_body && same_semantic_revision(saved, requested))
                },
            )
            .await?;
            let chain = admission.chain;
            if chain.created {
                if let Some(previous) = admission.superseded {
                    super::brief::record_run_in(
                        &tx,
                        &previous.job_id,
                        &inputs.article_id,
                        previous.generation,
                        "superseded",
                    )
                    .await?;
                }
                super::brief::record_run_in(
                    &tx,
                    &chain.job_id,
                    &inputs.article_id,
                    chain.generation,
                    "active",
                )
                .await?;
                capture_stage_inputs(&tx, &chain.job_id, inputs, force_body).await?;
            }
            Ok(chain)
        }
    })
    .await
    .map_err(|e| anyhow!("ensure brief chain {}: {e}", inputs.article_id))?;
    Ok(committed.value)
}

async fn capture_stage_inputs(
    tx: &Tx,
    job_id: &str,
    inputs: &BriefInputs,
    force_body: bool,
) -> Result<(), TxError> {
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
        jobs::run(
            tx,
            tx.query("CREATE $r SET input_json = $i, role_snapshot = $s RETURN NONE")
                .bind(("r", rid("brief_stage_input", &task_id(job_id, operation))))
                .bind(("i", input))
                .bind(("s", inputs.role_snapshot.clone())),
        )
        .await?;
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
            if let Some(actors) = super::actor_catalog::article_actors(&self.repo, article).await? {
                return Ok(actors.iter().map(ContextActor::from).collect());
            }
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
                 UPDATE brief_run SET state = 'completed', updated_at = time::now() WHERE job = $j AND state = 'active';
                 UPDATE $j SET state = 'completed', finished_at = time::now(),
                   updated_at = time::now() WHERE state IN ['active', 'queued', 'running'];
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
    if stage == super::brief::Stage::Other && operation != "intel_brief_body" {
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
    if operation == "intel_brief_body" {
        crate::atlas_actor_index::surreal::index_article_occurrences(repo, &run.article_id).await?;
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
    let captured = input(repo, &task_id(job_id, "intel_brief_claims"))
        .await?
        .and_then(|input| serde_json::from_str::<Value>(&input.input_json).ok())
        .map(|input| input["metadata"].clone())
        .unwrap_or(Value::Null);
    let prepared = super::brief_publish::prepare_captured(
        repo,
        &run.article_id,
        &captured,
        &body,
        &claims,
        &inferences,
        operation,
    )
    .await?;
    if prepared
        .as_ref()
        .err()
        .is_some_and(|reason| reason == super::brief_publish::CAPTURED_SOURCE_CHANGED)
    {
        return Ok(());
    }
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
                        "{}:{}:{inference}:{}:{candidate}:{}",
                        run.article_id,
                        run.generation,
                        lead["article_run_id"].as_str().unwrap_or(""),
                        lead["metadata_revision"].as_str().unwrap_or("")
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
    if done.value {
        // A failed stage leaves later stages queued. Those cannot start, so
        // close them and the job instead of leaving the chain `running`.
        let _ = jobs::settle_blocked_job(repo.db(), &leased.job_id).await;
    }
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
    let measured_at = std::time::Instant::now();
    let clock = crate::intel_recon::brief_timing::StageClock::default();
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
        r = crate::intel_recon::brief_timing::with_clock(clock.clone(), execute(&stages, &claimed, backend, &cancel)) => r,
        _ = async move {
            while !watch.load(Ordering::Acquire) {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        } => Err(anyhow!("cancelled")),
    };
    // Commit while the heartbeat still covers the lease, then stop it.
    let persist_at = std::time::Instant::now();
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
    crate::intel_recon::brief_timing::note_on(
        &clock,
        crate::intel_recon::brief_timing::Phase::Persistence,
        persist_at.elapsed(),
    );
    let (disposition, rows) = match &outcome {
        StageOutcome::Committed { items, .. } => ("committed", *items),
        StageOutcome::Stale { .. } => ("stale_or_cancelled", 0),
        StageOutcome::Failed { .. } => ("failed", 0),
    };
    let measurement = crate::intel_recon::brief_timing::observation(
        &clock,
        measured_at.elapsed(),
        disposition,
        rows,
    );
    crate::intel_recon::brief_timing::emit(
        &clock,
        measured_at.elapsed(),
        disposition,
        rows,
        &claimed.operation,
    );
    let mut event = crate::telemetry::TelemetryEvent::new(crate::telemetry::EventKind::IntelStage);
    event.id = format!("brief-timing:{}:e{}", claimed.id, claimed.epoch);
    event.app = "intel".into();
    event.mode = claimed.operation.clone();
    event.job_id = claimed.job_id.clone();
    event.outcome = disposition.into();
    event.metric_ms = Some(measured_at.elapsed().as_millis().min(i64::MAX as u128) as i64);
    event.payload_json = measurement.to_string();
    if let Err(error) = super::telemetry::record(repo.db(), &event).await {
        tracing::warn!(error = %error, "brief timing telemetry unavailable");
    }
    done.store(true, Ordering::Release);
    if let Some(h) = heartbeat_guard.handle.take() {
        h.abort();
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
/// without its legacy store file (no telemetry rows), with the body and related
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

/// Two independent indexed channels share one hard initial raw-row budget.
/// Hydration follows the actor cap; text metadata queries have their own SQL caps.
pub async fn discover_link_candidates(
    repo: &Repository,
    ctx: &StageContext,
    article: &AtlasArticleRow,
) -> Result<crate::intel_recon::brief_stages::LinkDiscovery> {
    discover_link_candidates_after(repo, ctx, article, None).await
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct LinkDiscoveryCursor {
    revision: String,
    actor_after: Option<[String; 3]>,
    actor_done: bool,
    text_after: Option<String>,
    text_done: bool,
}

/// Explicit continuation of bounded indexed discovery. Never queues model work.
pub async fn discover_link_candidates_after(
    repo: &Repository,
    ctx: &StageContext,
    article: &AtlasArticleRow,
    cursor: Option<&str>,
) -> Result<crate::intel_recon::brief_stages::LinkDiscovery> {
    let mut actors: Vec<String> = ctx.metadata_snapshot["actors"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|actor| actor["id"].as_str().map(str::to_owned))
        .collect();
    actors.sort();
    actors.dedup();
    actors.truncate(crate::intelligence_contracts::MAX_CONTEXT_ACTORS);
    let revision = crate::summarization::sha_hex(
        &json!([crate::atlas_metadata::input_hash(article), actors]).to_string(),
    );
    let cursor = match cursor {
        Some(raw) => {
            anyhow::ensure!(raw.len() <= 4096, "link discovery cursor exceeds bound");
            let cursor: LinkDiscoveryCursor = serde_json::from_str(raw)?;
            anyhow::ensure!(cursor.revision == revision, "stale link discovery cursor");
            cursor
        }
        None => LinkDiscoveryCursor {
            revision: revision.clone(),
            ..Default::default()
        },
    };
    let actor_page = if actors.is_empty() || cursor.actor_done {
        super::atlas::CurrentActorMentionPage::default()
    } else {
        super::atlas::current_actor_mentions_page(
            repo,
            &actors,
            &crate::atlas_classify::OccurrenceKey::new(&article.run_id, &article.id),
            50,
            cursor.actor_after.as_ref(),
        )
        .await?
    };
    let actor_raw = actor_page.fetched;
    let actor_after = actor_page.after;
    let actor_more = actor_page.has_more;
    let mentions = actor_page.rows;
    let filtered_actor_count = actor_raw.saturating_sub(mentions.len());
    let mut seen = std::collections::BTreeSet::new();
    let keys: Vec<_> = mentions
        .iter()
        .filter(|mention| seen.insert((mention.origin_scope.clone(), mention.origin_id.clone())))
        .map(|mention| {
            crate::atlas_classify::OccurrenceKey::new(&mention.origin_scope, &mention.origin_id)
        })
        .collect();
    let actor_unique = keys.len();
    let mut rows: Vec<AtlasArticleRow> = super::atlas::current_articles_many(repo.db(), &keys)
        .await?
        .into_values()
        .collect();
    rows.sort_by(|a, b| (&a.run_id, &a.id).cmp(&(&b.run_id, &b.id)));
    let excluded_or_missing = filtered_actor_count + actor_unique.saturating_sub(rows.len());
    // An excluded actor hit must not suppress a separately authorized text hit.
    seen = rows
        .iter()
        .map(|row| (row.run_id.clone(), row.id.clone()))
        .collect();
    let text = if cursor.text_done {
        super::article_discovery::BoundedRelatedCandidates::default()
    } else {
        super::article_discovery::related_from_seed_bounded_after(
            repo,
            article,
            100 - actor_raw,
            cursor.text_after.as_deref(),
        )
        .await?
    };
    for hit in &text.hits {
        if !seen.insert((hit.run_id.clone(), hit.article_id.clone())) {
            continue;
        }
        rows.push(AtlasArticleRow {
            run_id: hit.run_id.clone(),
            id: hit.article_id.clone(),
            title: hit.title.clone(),
            description: hit.description.clone(),
            url: hit.url.clone(),
            source_name: hit.source_name.clone(),
            source_domain: hit.source_domain.clone(),
            published_at: hit.published_at.clone(),
            seen_at: hit.seen_at.clone(),
            provider: hit.provider.clone(),
            categories: hit.categories.clone(),
            category: hit.categories.first().cloned().unwrap_or_default(),
            ..Default::default()
        });
    }
    let actor_done = cursor.actor_done || !actor_more;
    let text_done = cursor.text_done || !text.truncated;
    let next_cursor = if actor_done && text_done {
        None
    } else {
        Some(serde_json::to_string(&LinkDiscoveryCursor {
            revision,
            actor_after: actor_after.or(cursor.actor_after),
            actor_done,
            text_after: text.next_cursor.clone().or(cursor.text_after),
            text_done,
        })?)
    };
    let coverage = json!({"adapter":"indexed_all_retained","initial_limit":100,"actor_fetched":actor_raw,"text_fetched":text.fetched,"unique_candidates":rows.len(),"excluded_or_missing_actor_candidates":excluded_or_missing,"truncated":next_cursor.is_some(),"next_cursor":next_cursor,"text_revision":text.revision,"curator_limit":crate::intel_recon::brief_stages::MAX_CANDIDATES,"note":"Discovery overlap is tentative; current metadata and semantic publication gates decide confirmation."});
    Ok(crate::intel_recon::brief_stages::LinkDiscovery { rows, coverage })
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
        let clock = crate::intel_recon::brief_timing::current_clock().unwrap_or_default();
        std::thread::scope(|s| {
            s.spawn(|| {
                handle.block_on(crate::intel_recon::brief_timing::with_clock(
                    clock,
                    crate::intel_recon::brief_timing::timed(
                        crate::intel_recon::brief_timing::Phase::Local,
                        related(&repo, article),
                    ),
                ))
            })
            .join()
            .map_err(|_| anyhow!("related read panicked"))?
        })
    }

    fn related_discovery<'a>(
        &'a mut self,
        ctx: &'a StageContext,
        article: &'a AtlasArticleRow,
    ) -> BoxFut<'a, Result<crate::intel_recon::brief_stages::LinkDiscovery>> {
        Box::pin(async move { discover_link_candidates(&self.repo, ctx, article).await })
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
    let inputs = inputs_for_article(repo, article, settings).await?;
    Ok(ensure(repo, &inputs).await?.job_id)
}

/// Captures the metadata/domain/actor/route context once before either root
/// dispatches. Body text is never part of this metadata-only context.
pub async fn inputs_for_article(
    repo: &Repository,
    article: &AtlasArticleRow,
    settings: &crate::provider::SettingsFile,
) -> Result<BriefInputs> {
    let auth = tokio::task::spawn_blocking(crate::secrets::AuthFile::load).await??;
    inputs_for_article_with_auth(repo, article, settings, &auth).await
}

/// Captures context using an already loaded runtime auth snapshot. Hosts and
/// isolated offline fixtures can share their immutable route configuration.
pub async fn inputs_for_article_with_auth(
    repo: &Repository,
    article: &AtlasArticleRow,
    settings: &crate::provider::SettingsFile,
    auth: &crate::secrets::AuthFile,
) -> Result<BriefInputs> {
    use crate::intel_recon::brief_stages as bs;
    use crate::intelligence_contracts::{
        DomainActor, DomainPacket, DomainSource, DomainVersions, MAX_CONTEXT_ACTORS,
        MAX_CONTEXT_ALIASES,
    };

    let captured =
        if let Some(actors) = super::actor_catalog::metadata_article_actors(repo, article).await? {
            actors
        } else {
            super::atlas_metadata::metadata(repo.db(), &article.run_id, &article.id)
                .await?
                .map(|metadata| crate::atlas_metadata::article_actors(article, &metadata))
                .unwrap_or_default()
        };
    let mut captured: Vec<ContextActor> = captured.iter().map(ContextActor::from).collect();
    captured.sort_by(|left, right| left.id.cmp(&right.id));
    captured.dedup_by(|left, right| left.id == right.id);
    let mut actors_omitted = captured.len().saturating_sub(MAX_CONTEXT_ACTORS);
    captured.truncate(MAX_CONTEXT_ACTORS);
    let mut aliases_omitted = 0usize;
    let mut actors = Vec::with_capacity(captured.len());
    for actor in &mut captured {
        actor.aliases.sort();
        actor.aliases.dedup();
        aliases_omitted += actor.aliases.len().saturating_sub(MAX_CONTEXT_ALIASES);
        actor.aliases.truncate(MAX_CONTEXT_ALIASES);
        actors.push(DomainActor::new(
            actor.id.clone(),
            actor.name.clone(),
            actor.aliases.clone(),
        )?);
    }
    let source_revision = crate::atlas_metadata::input_hash(article);
    let mut source = DomainSource::new(
        article.run_id.clone(),
        article.id.clone(),
        source_revision,
        crate::intel_recon::sentence_index::INDEX_VERSION.into(),
    )?;
    source.publisher = article.source_name.clone();
    source.url = article.url.clone();
    source.published_at = chrono::DateTime::parse_from_rfc3339(&article.published_at)
        .ok()
        .map(|_| article.published_at.clone());
    // Country is known source metadata. More precise geography is not inferred.
    source.place = (!article.country.trim().is_empty()).then(|| article.country.clone());
    #[cfg(feature = "classifier")]
    let admission = {
        use crate::atlas_classify::OccurrenceKey;
        use crate::intelligence_contracts::DomainAdmission;
        let key = OccurrenceKey::new(article.run_id.clone(), article.id.clone());
        let view = super::classifier_admission::classification_view(repo, &key).await?;
        let current_input = crate::classifier::ClassifyInput::new(
            key.record_id(),
            article.title.clone(),
            article.description.clone(),
            None,
        );
        if !view.legacy && view.admission_state == "admitted" {
            let stored = view
                .receipt
                .ok_or_else(|| anyhow!("admitted domain context lacks its validated receipt"))?;
            if stored.receipt.input_hash != current_input.input_hash {
                return Err(anyhow!(
                    "admitted domain context belongs to a different metadata revision"
                ));
            }
            let pair = stored.receipt.selected_domains.clone();
            Some(DomainAdmission::new(
                &view.taxonomy_version,
                pair,
                crate::summarization::sha_hex(&serde_json::to_string(&stored)?),
            )?)
        } else {
            None
        }
    };
    #[cfg(not(feature = "classifier"))]
    let admission = None;
    let mut routes = Vec::new();
    for (role, operation) in [
        ("atlas_analyst", Some(crate::provider::OP_CLAIMS_GENERATE)),
        (
            "atlas_analyst",
            Some(crate::provider::OP_INFERENCES_GENERATE),
        ),
        ("atlas_analyst", Some(crate::provider::OP_LINKS_GENERATE)),
        ("atlas_evaluator", None),
        (
            "atlas_evaluator",
            Some(crate::provider::OP_INFERENCES_ASSESS),
        ),
        (
            "atlas_evaluator",
            Some(crate::provider::OP_INFERENCES_DEDUP),
        ),
    ] {
        let (resolved, secrets) =
            crate::recon::model_exec::resolve_operation_routes(auth, settings, role, operation);
        let projection: Vec<_> = resolved
            .iter()
            .zip(&secrets)
            .map(|(route, secret)| {
                let host = url::Url::parse(&secret.base_url)
                    .ok()
                    .and_then(|url| url.host_str().map(str::to_owned))
                    .unwrap_or_default();
                json!([
                    route.provider,
                    route.account,
                    secret.model,
                    host,
                    route.skip_reason.is_some()
                ])
            })
            .collect();
        routes.push(json!([role, operation, projection]));
    }
    let route = crate::summarization::sha_hex(&serde_json::to_string(&routes)?);
    let mut packet = DomainPacket::new(
        source,
        admission,
        actors,
        DomainVersions::new(
            "intel-brief-domain-context-v1".into(),
            crate::intel_recon::brief_contracts::BRIEF_CONTRACT_VERSION.into(),
            route,
        )?,
    )?;
    // Preserve exact known aliases, or explicitly omit whole alias/actor
    // entries. Source text is never abbreviated into a different identity.
    while serde_json::to_vec(&packet)?.len() > 8 * 1024 {
        if let Some(actor) = packet
            .actors
            .iter_mut()
            .rev()
            .find(|actor| !actor.verified_aliases.is_empty())
        {
            actor.verified_aliases.pop();
            aliases_omitted += 1;
        } else if packet.actors.pop().is_some() {
            actors_omitted += 1;
        } else {
            return Err(anyhow!("domain source context exceeds its bounded packet"));
        }
    }
    let captured: Vec<ContextActor> = packet
        .actors
        .iter()
        .map(|actor| ContextActor {
            id: actor.id.clone(),
            name: actor.name.clone(),
            aliases: actor.verified_aliases.clone(),
        })
        .collect();
    let packet_revision = packet.revision()?;
    let coverage =
        json!({"actors_omitted":actors_omitted,"verified_aliases_omitted":aliases_omitted});
    let domain_revision =
        crate::summarization::sha_hex(&json!([packet_revision, coverage]).to_string());
    let stage_revisions: Vec<_> = bs::stage_revisions(article, settings)
        .into_iter()
        .map(|revision| {
            crate::summarization::sha_hex(&json!([revision, domain_revision]).to_string())
        })
        .collect();
    let mut metadata = bs::metadata_snapshot(article);
    metadata["domain_context"] = serde_json::to_value(packet)?;
    metadata["domain_context_coverage"] = coverage;
    metadata["actors"] = serde_json::to_value(captured)?;
    let base_revision = crate::nlp::cache::hash(&stage_revisions.join(":"));
    let current_body = clean_artifact(repo, &article.id)
        .await?
        .map(|artifact| artifact.content_hash)
        .filter(|hash| !hash.is_empty());
    let latest = latest_run(repo, &article.id).await?;
    let completed_body = if let Some(latest) = &latest {
        SurrealStages::new(repo)
            .stage_output(&latest.job_id, "intel_brief_body")
            .await
            .ok()
            .and_then(|output| {
                output["content_hash"]
                    .as_str()
                    .filter(|hash| !hash.is_empty())
                    .map(str::to_owned)
            })
    } else {
        None
    };
    let revision = observed_body_revision(
        &base_revision,
        latest
            .as_ref()
            .map(|run| (run.input_revision.as_str(), run.state == "active")),
        current_body.as_deref(),
        completed_body.as_deref(),
    );
    let mut stage_revisions = stage_revisions;
    // Claims remain metadata-only. Body and its inference join must not reuse
    // outputs from an observed external body refresh.
    let body_basis = revision
        .split_once(":body:")
        .map(|(_, value)| value)
        .unwrap_or("initial");
    for index in [0, 2] {
        if let Some(stage) = stage_revisions.get_mut(index) {
            *stage = crate::summarization::sha_hex(&json!([stage, body_basis]).to_string());
        }
    }
    metadata["body_observation"] = json!(current_body);
    Ok(BriefInputs {
        article_id: article.id.clone(),
        input_revision: revision,
        role_snapshot: serde_json::to_string(&settings.defaults)?,
        title: article.title.clone(),
        stage_revisions,
        metadata,
    })
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

/// Compare only against a committed Body output. A cache head may appear
/// before that task commits; it must never supersede its own pending chain.
fn observed_body_revision(
    base: &str,
    previous: Option<(&str, bool)>,
    current: Option<&str>,
    completed: Option<&str>,
) -> String {
    if let Some((previous, pending)) = previous {
        let previous = previous
            .split_once(":reload:")
            .map_or(previous, |(base, _)| base);
        let previous_base = previous
            .split_once(":body:")
            .map_or(previous, |(base, _)| base);
        if previous_base == base && ((pending && completed.is_none()) || current == completed) {
            return previous.to_owned();
        }
    }
    let body_basis = if previous.is_some() && completed.is_some() {
        format!(
            "transition-{}",
            crate::summarization::sha_hex(&json!([current, completed]).to_string())
        )
    } else {
        current.unwrap_or("missing").to_owned()
    };
    format!("{base}:body:{body_basis}")
}

fn same_semantic_revision(saved: &str, requested: &str) -> bool {
    !requested.is_empty()
        && saved.split_once(":reload:").map_or(saved, |(base, _)| base) == requested
}

#[cfg(test)]
mod revision_tests {
    use super::{observed_body_revision, same_semantic_revision};

    #[test]
    fn body_completion_preserves_its_own_chain_but_external_refresh_changes_it() {
        let first = observed_body_revision("metadata-route", None, None, None);
        assert_eq!(first, "metadata-route:body:missing");
        assert_eq!(
            observed_body_revision("metadata-route", Some((&first, true)), Some("body-1"), None),
            first,
            "pending Body race cannot supersede itself"
        );
        assert_eq!(
            observed_body_revision(
                "metadata-route",
                Some((&first, false)),
                Some("body-1"),
                Some("body-1")
            ),
            first,
            "completion is not its own invalidation source"
        );
        let refreshed = observed_body_revision(
            "metadata-route",
            Some((&first, false)),
            Some("body-2"),
            Some("body-1"),
        );
        assert!(refreshed.starts_with("metadata-route:body:transition-"));
        assert_eq!(
            observed_body_revision(
                "metadata-route",
                Some((&refreshed, false)),
                Some("body-2"),
                Some("body-2")
            ),
            refreshed,
            "ordinary reopen reuses the new body generation"
        );
        let reload = "metadata-route:body:body-1:reload:explicit";
        let rollback = observed_body_revision(
            "metadata-route",
            Some((reload, false)),
            Some("body-1"),
            Some("body-2"),
        );
        assert_ne!(
            rollback, "metadata-route:body:body-1",
            "reload A->B then external head A must not reuse the B output"
        );
        assert_ne!(
            observed_body_revision(
                "changed-source",
                Some((&refreshed, true)),
                Some("body-2"),
                None
            ),
            refreshed
        );
    }

    #[test]
    fn ordinary_reopen_preserves_only_matching_semantic_inputs() {
        assert!(same_semantic_revision(
            "source-domain-prompt-route-v1",
            "source-domain-prompt-route-v1"
        ));
        assert!(same_semantic_revision(
            "source-domain-prompt-route-v1:reload:explicit-key",
            "source-domain-prompt-route-v1"
        ));
        assert!(!same_semantic_revision(
            "source-domain-prompt-route-v1:reload:explicit-key",
            "source-domain-prompt-route-v2"
        ));
        assert!(!same_semantic_revision(
            "legacy-domainless",
            "source-domain-prompt-route-v1"
        ));
        assert!(!same_semantic_revision("", ""));
    }
}
