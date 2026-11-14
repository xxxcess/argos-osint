//! Durable, revision-fenced publication of completed brief stages.

use anyhow::Result;
use serde_json::{json, Value};
use surrealdb::types::SurrealValue;

use super::{input, task_id, Run, SurrealStages};
use crate::intel_recon::brief_pipeline;
use crate::intel_recon::brief_stages::StageStore;
use crate::storage::repo::Repository;
use crate::storage::tx::{run_tx, RetryPolicy, Tx, TxError};
use crate::storage::{brief, brief_publish, jobs};

const TABLE: &str = "brief_publication_work";

#[derive(SurrealValue)]
struct Work {
    id: String,
    job_id: String,
    operation: String,
    epoch: i64,
}

const WORK_FIELDS: &str = "record::id(id) AS id, job_id, operation, epoch";

pub(super) async fn enqueue_in(tx: &Tx, task: &jobs::ClaimedTask) -> Result<(), TxError> {
    jobs::run(tx, tx.query("UPSERT $work SET job_id = $job, operation = $op, epoch = $epoch, due_at = time::now() RETURN NONE")
        .bind(("work", jobs::rid(TABLE, &task.id)))
        .bind(("job", task.job_id.clone())).bind(("op", task.operation.clone()))
        .bind(("epoch", task.lease_epoch))).await?;
    Ok(())
}

/// Process a bounded page of work committed by this or a previous worker.
/// The stage's immutable output is reused without a model dispatch.
pub async fn drain_publications(repo: &Repository, limit: usize) -> Result<usize> {
    let mut result = repo.db().query(format!(
        "SELECT {WORK_FIELDS}, due_at FROM brief_publication_work WHERE due_at <= time::now() ORDER BY due_at LIMIT $limit"
    )).bind(("limit", limit.min(64) as i64)).await?.check()?;
    let work: Vec<Work> = result.take(0)?;
    let mut published = 0;
    for item in &work {
        match process(repo, item).await {
            Ok(done) => published += usize::from(done),
            Err(error) => {
                defer_failure(repo, item, &error).await?;
                tracing::warn!(task = %item.id, error = %crate::provider_diag::sanitize(&error.to_string()), "brief publication deferred");
            }
        }
    }
    Ok(published)
}

pub(super) async fn publish_task(repo: &Repository, id: &str, epoch: i64) -> Result<()> {
    let mut result = repo
        .db()
        .query(format!("SELECT {WORK_FIELDS} FROM ONLY $work"))
        .bind(("work", jobs::rid(TABLE, id)))
        .await?
        .check()?;
    let row: Option<Work> = result.take(0)?;
    if let Some(work) = row.filter(|work| work.epoch == epoch) {
        if let Err(error) = process(repo, &work).await {
            defer_failure(repo, &work, &error).await?;
            return Err(error);
        }
    }
    Ok(())
}

async fn process(repo: &Repository, work: &Work) -> Result<bool> {
    let mut result = repo
        .db()
        .query("SELECT article_id, generation FROM brief_run WHERE job = $job LIMIT 1")
        .bind(("job", jobs::rid("job", &work.job_id)))
        .await?
        .check()?;
    let run = result.take::<Vec<Run>>(0)?.into_iter().next();
    let Some(run) = run else {
        return Ok(run_tx(repo.db(), RetryPolicy::default(), |tx| async move {
            acknowledge(&tx, work).await
        })
        .await?
        .value);
    };
    if work.operation == "intel_brief_body" {
        // Native actor indexing has its own source/revision receipts. If the
        // process stops before the acknowledgement, replay is safe.
        let live = run_tx(repo.db(), RetryPolicy::default(), |tx| {
            let run = &run;
            async move { current(&tx, work, run).await }
        })
        .await?
        .value;
        if live {
            crate::atlas_actor_index::surreal::index_article_occurrences(repo, &run.article_id)
                .await?;
        }
        return Ok(run_tx(repo.db(), RetryPolicy::default(), |tx| async move {
            acknowledge(&tx, work).await
        })
        .await?
        .value);
    }
    let stage = brief::Stage::from_operation(&work.operation);
    let stages = SurrealStages::new(repo);
    let output = |operation: &'static str| {
        let stages = &stages;
        async move {
            stages
                .stage_output(&work.job_id, operation)
                .await
                .unwrap_or(Value::Null)
        }
    };
    let body = output("intel_brief_body").await;
    let claims = output("intel_brief_claims").await;
    let inferences = output("intel_brief_inferences").await;
    let links = output("intel_brief_links").await;
    let own = stages.stage_output(&work.job_id, &work.operation).await?;
    let snapshot_hash = crate::nlp::cache::hash(&json!([work.operation, own]).to_string());
    let captured = input(repo, &task_id(&work.job_id, "intel_brief_claims"))
        .await?
        .and_then(|input| serde_json::from_str::<Value>(&input.input_json).ok())
        .map(|input| input["metadata"].clone())
        .unwrap_or(Value::Null);
    let prepared = brief_publish::prepare_captured(
        repo,
        &run.article_id,
        &captured,
        &body,
        &claims,
        &inferences,
        &work.operation,
    )
    .await?;
    let publication = brief::Publication {
        article_id: run.article_id.clone(),
        job_id: work.job_id.clone(),
        generation: run.generation,
        contract: brief_pipeline::CONTRACT.into(),
        body_hash: body["content_hash"].as_str().unwrap_or("").to_owned(),
        snapshot_hash,
        leads: links["leads"].clone(),
        claims,
        inferences,
        stage,
    };
    let leads = if stage == brief::Stage::Links {
        leads_for(&run, &publication.inferences, &links)
    } else {
        Vec::new()
    };
    let link_revision = format!("{}:{}", work.id, work.epoch);
    Ok(run_tx(repo.db(), RetryPolicy::default(), |tx| {
        let (run, publication, prepared, captured, leads, link_revision) = (
            &run,
            &publication,
            &prepared,
            &captured,
            &leads,
            &link_revision,
        );
        async move {
            if !current(&tx, work, run).await?
                || !current_source(&tx, captured, &run.article_id).await?
            {
                return acknowledge(&tx, work).await;
            }
            if prepared
                .as_ref()
                .err()
                .is_some_and(|reason| reason == brief_publish::CAPTURED_SOURCE_CHANGED)
            {
                return acknowledge(&tx, work).await;
            }
            if let Some(prerequisite) = match stage {
                brief::Stage::Inferences => Some("intel_brief_claims"),
                brief::Stage::Links => Some("intel_brief_inferences"),
                _ => None,
            } {
                let mut result = jobs::run(
                    &tx,
                    tx.query("RETURN record::exists($previous)").bind((
                        "previous",
                        jobs::rid(TABLE, &task_id(&work.job_id, prerequisite)),
                    )),
                )
                .await?;
                if jobs::take::<Option<bool>>(&mut result, 0)? == Some(true) {
                    defer_in(&tx, work, 1).await?;
                    return Ok(false);
                }
            }
            if !brief::publish_in(&tx, publication).await? {
                return acknowledge(&tx, work).await;
            }
            if stage == brief::Stage::Links {
                brief::replace_leads_in(&tx, &run.article_id, &work.job_id, run.generation, leads)
                    .await?;
            }
            let state = brief_publish::publish_stage_in(
                &tx,
                &run.article_id,
                &work.job_id,
                &work.operation,
                prepared,
                &publication.snapshot_hash,
                link_revision,
            )
            .await?;
            brief::put_publication_state_in(
                &tx,
                &run.article_id,
                &work.job_id,
                &work.operation,
                &state,
            )
            .await?;
            acknowledge(&tx, work).await
        }
    })
    .await?
    .value)
}

async fn current(tx: &Tx, work: &Work, run: &Run) -> Result<bool, TxError> {
    jobs::bump(tx, "job_guard", &work.job_id).await?;
    jobs::bump(tx, "brief_chain_guard", &run.article_id).await?;
    let mut result = jobs::run(tx, tx.query(
        "RETURN (SELECT VALUE epoch FROM ONLY $work) = $epoch
          AND (SELECT VALUE lease_epoch = $epoch AND state = 'completed' AND !job.cancel_requested FROM ONLY $task) = true
          AND (SELECT VALUE generation FROM ONLY $guard) = $generation;"
    ).bind(("work", jobs::rid(TABLE, &work.id)))
        .bind(("task", jobs::rid("task", &work.id)))
        .bind(("guard", jobs::rid("brief_chain_guard", &run.article_id)))
        .bind(("epoch", work.epoch)).bind(("generation", run.generation))).await?;
    Ok(jobs::take::<Option<bool>>(&mut result, 0)? == Some(true))
}

async fn current_source(tx: &Tx, captured: &Value, article_id: &str) -> Result<bool, TxError> {
    let Some(run_id) = captured["run_id"].as_str() else {
        return Ok(true);
    };
    crate::storage::atlas_delete::guard_occurrence_removal_in(tx, run_id).await?;
    crate::storage::classifier_admission::derivative_source_guards_in(
        tx,
        &[crate::atlas_classify::OccurrenceKey {
            run_id: run_id.to_owned(),
            article_id: article_id.to_owned(),
        }],
    )
    .await?;
    Ok(crate::storage::atlas::article_in(tx, run_id, article_id)
        .await?
        .is_some_and(|article| brief_publish::matches_captured(captured, &article)))
}

async fn acknowledge(tx: &Tx, work: &Work) -> Result<bool, TxError> {
    let mut result = jobs::run(
        tx,
        tx.query("DELETE $work WHERE epoch = $epoch RETURN BEFORE")
            .bind(("work", jobs::rid(TABLE, &work.id)))
            .bind(("epoch", work.epoch)),
    )
    .await?;
    Ok(!jobs::take::<Vec<surrealdb::types::Value>>(&mut result, 0)?.is_empty())
}

async fn defer_in(tx: &Tx, work: &Work, seconds: u64) -> Result<(), TxError> {
    jobs::run(tx, tx.query("UPDATE $work SET due_at = time::now() + <duration>($seconds + 's') WHERE epoch = $epoch RETURN NONE")
        .bind(("work", jobs::rid(TABLE, &work.id))).bind(("epoch", work.epoch))
        .bind(("seconds", seconds.to_string()))).await?;
    Ok(())
}

async fn defer_failure(repo: &Repository, work: &Work, error: &anyhow::Error) -> Result<()> {
    let reason = crate::provider_diag::sanitize(&error.to_string());
    run_tx(repo.db(), RetryPolicy::default(), |tx| {
        let reason = &reason;
        async move {
            defer_in(&tx, work, 30).await?;
            if work.operation == "intel_brief_body" {
                return Ok(());
            }
            let mut result = jobs::run(
                &tx,
                tx.query("SELECT article_id, generation FROM brief_run WHERE job = $job LIMIT 1")
                    .bind(("job", jobs::rid("job", &work.job_id))),
            )
            .await?;
            if let Some(run) = jobs::take::<Vec<Run>>(&mut result, 0)?.into_iter().next() {
                if current(&tx, work, &run).await? {
                    let state = brief_publish::failed_state(&work.job_id, reason);
                    brief::put_publication_state_in(
                        &tx,
                        &run.article_id,
                        &work.job_id,
                        &work.operation,
                        &state,
                    )
                    .await?;
                }
            }
            Ok(())
        }
    })
    .await?;
    Ok(())
}

fn leads_for(run: &Run, inferences: &Value, links: &Value) -> Vec<brief::Lead> {
    let texts: std::collections::HashMap<&str, &str> = inferences["inferences"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|i| Some((i["id"].as_str()?, i["text"].as_str()?)))
        .collect();
    links["leads"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|lead| {
            let inference = lead["inference_id"].as_str().unwrap_or("");
            let candidate = lead["article_id"].as_str().unwrap_or("");
            brief::Lead {
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
        .collect()
}
