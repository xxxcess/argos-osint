//! The Brain neighbourhood summary on the repository: the
//! `brain_neighborhood_brief_exec` publish / poll / retry / run on the
//! SurrealDB summary queue (`storage::summary_flush`). The deterministic
//! brief is cached as the fallback and one `topology_neighborhood` task per
//! request identity upgrades it with the full role chain. Publication and
//! completion are one transaction fenced by the task lease.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use chrono::Utc;
use surrealdb::types::SurrealValue;

use super::jobs::{self, rid, run, take, ClaimedTask};
use super::repo::Repository;
use super::summary_flush;
use super::tx::{run_tx, RetryPolicy};
use crate::brain_neighborhood_brief::{NeighborhoodBrief, NeighborhoodInput};
use crate::brain_neighborhood_brief_exec::{
    build_request, delivered, fallback_result, generate, generated_result, Delivery, FailureKind,
    GenerationFailure, Outcome,
};
use crate::recon::model_exec::RoleRuntime;
use crate::summarization::SummaryRequest;

fn task_id(req: &SummaryRequest) -> String {
    format!("sum-task-{}", req.cache_key())
}

/// `exec::publish`: the generated result when one is cached, else cache the
/// fallback, queue one upgrade and report its state.
pub async fn publish(
    repo: &Repository,
    input: &NeighborhoodInput,
    fallback: &NeighborhoodBrief,
    fingerprint: &str,
) -> Result<(SummaryRequest, Delivery)> {
    let req = build_request(input, fingerprint);
    if !input.worth_generating() {
        return Ok((req, Delivery::Idle));
    }
    if let Some(g) = summary_flush::cache_get(repo, &req)
        .await?
        .as_ref()
        .and_then(delivered)
    {
        return Ok((req, Delivery::Generated(Box::new(g))));
    }
    summary_flush::publish(repo, &req, fallback_result(&req, fallback)).await?;
    let delivery = poll(repo, &req).await?;
    Ok((req, delivery))
}

#[derive(SurrealValue)]
struct TaskRow {
    state: String,
    output_hash: Option<String>,
    error_message: Option<String>,
}

/// `exec::poll`: read-only; never runs a model.
pub async fn poll(repo: &Repository, req: &SummaryRequest) -> Result<Delivery> {
    if let Some(g) = summary_flush::cache_get(repo, req)
        .await?
        .as_ref()
        .and_then(delivered)
    {
        return Ok(Delivery::Generated(Box::new(g)));
    }
    let mut r = repo
        .db()
        .query("SELECT state, output_hash, error_message FROM ONLY $t")
        .bind(("t", rid("task", &task_id(req))))
        .await?
        .check()?;
    let row: Option<TaskRow> = r.take(0usize)?;
    Ok(match row {
        None => Delivery::Idle,
        Some(t) => match t.state.as_str() {
            "queued" | "running" | "retry_scheduled" | "paused" | "blocked" => Delivery::Pending,
            "completed" => match t
                .output_hash
                .as_deref()
                .and_then(GenerationFailure::from_tag)
            {
                Some(f) => Delivery::Failed(f),
                None => Delivery::Failed(GenerationFailure::new(
                    FailureKind::Provider,
                    "generation finished without a stored result",
                )),
            },
            _ => Delivery::Failed(GenerationFailure::new(
                FailureKind::Provider,
                t.error_message.filter(|e| !e.is_empty()).unwrap_or(t.state),
            )),
        },
    })
}

/// `exec::retry`: reset a finished task of the same identity to queued
/// once; queued or running work is left alone. `false` when nothing was
/// published for the request or it is still in flight.
pub async fn retry(repo: &Repository, req: &SummaryRequest) -> Result<bool> {
    let key = req.cache_key();
    if summary_flush::load_request(repo, &key).await?.is_none() {
        return Ok(false);
    }
    let (task, job) = (task_id(req), format!("sum-job-{key}"));
    let now = jobs::ts(Utc::now());
    let reset = run_tx(repo.db(), RetryPolicy::default(), |tx| {
        let (task, job, now) = (task.clone(), job.clone(), now.clone());
        async move {
            jobs::bump(&tx, "job_guard", &job).await?;
            let mut r = run(
                &tx,
                tx.query(
                    "UPDATE $t SET state = 'queued', attempts = 0, output_hash = NONE,
                       error_category = '', error_message = '', lease_owner = '',
                       process_owner = '', lease_until = NONE, finished_at = NONE,
                       next_eligible_at = <datetime>$now, updated_at = <datetime>$now
                     WHERE state IN ['completed', 'failed', 'cancelled']
                     RETURN VALUE record::id(id);",
                )
                .bind(("t", rid("task", &task)))
                .bind(("now", now.clone())),
            )
            .await?;
            let ids: Vec<String> = take(&mut r, 0)?;
            if ids.is_empty() {
                return Ok(false);
            }
            run(
                &tx,
                tx.query(
                    "UPDATE $j SET state = 'running', cancel_requested = false,
                       finished_at = NONE, updated_at = <datetime>$now RETURN NONE;",
                )
                .bind(("j", rid("job", &job)))
                .bind(("now", now)),
            )
            .await?;
            Ok(true)
        }
    })
    .await?;
    Ok(reset.value)
}

/// `exec::run_claimed` on the repository: the lease is renewed while the
/// model runs; the cache write and completion are one fenced transaction.
pub async fn run_claimed(
    repo: &Repository,
    claimed: &ClaimedTask,
    req: &SummaryRequest,
    runtime: Option<&RoleRuntime>,
    lease_secs: i64,
) -> Result<Outcome> {
    if summary_flush::cache_get(repo, req)
        .await?
        .as_ref()
        .and_then(delivered)
        .is_some()
    {
        return finish(repo, claimed, req, None, "already_upgraded".into(), true).await;
    }
    let Some(runtime) = runtime else {
        let f = GenerationFailure::new(
            FailureKind::NotConfigured,
            "no Summarization model runtime is available",
        );
        return finish(repo, claimed, req, None, f.tag(), false).await;
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let lost = Arc::new(AtomicBool::new(false));
    let stop = Arc::new(AtomicBool::new(false));
    let beat = {
        let (repo, claimed) = (repo.clone(), claimed.clone());
        let (cancel, lost, stop) = (cancel.clone(), lost.clone(), stop.clone());
        let every = Duration::from_millis((lease_secs.max(1) as u64 * 1000 / 3).max(50));
        tokio::spawn(async move {
            while !stop.load(Ordering::SeqCst) {
                tokio::time::sleep(every).await;
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                let renewed = jobs::renew_lease(repo.db(), &claimed, lease_secs, Utc::now())
                    .await
                    .map(|c| c.value)
                    .unwrap_or(false);
                if !renewed {
                    lost.store(true, Ordering::SeqCst);
                    cancel.store(true, Ordering::SeqCst);
                    break;
                }
            }
        })
    };
    struct HeartbeatGuard {
        stop: Arc<AtomicBool>,
        handle: Option<tokio::task::JoinHandle<()>>,
    }

    impl Drop for HeartbeatGuard {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            if let Some(handle) = self.handle.take() {
                handle.abort();
            }
        }
    }

    let mut beat_guard = HeartbeatGuard {
        stop: stop.clone(),
        handle: Some(beat),
    };
    let operation_id = format!("{}:e{}:topology", claimed.id, claimed.lease_epoch);
    let result = generate(
        runtime,
        None,
        &operation_id,
        Some((&claimed.id, claimed.lease_epoch)),
        req,
        &cancel,
    )
    .await;
    stop.store(true, Ordering::SeqCst);
    if let Some(h) = beat_guard.handle.take() {
        let _ = h.await;
    }
    if lost.load(Ordering::SeqCst) {
        return Ok(lease_lost());
    }
    match result {
        Ok(g) => {
            let cached = generated_result(req, &g)?;
            finish(
                repo,
                claimed,
                req,
                Some(cached),
                format!("generated:{}", g.route),
                true,
            )
            .await
        }
        Err(f) => finish(repo, claimed, req, None, f.tag(), false).await,
    }
}

fn lease_lost() -> Outcome {
    Outcome {
        tag: "lease_lost".into(),
        generated: false,
        stale: true,
    }
}

async fn finish(
    repo: &Repository,
    claimed: &ClaimedTask,
    req: &SummaryRequest,
    cached: Option<crate::summarization::SummaryResult>,
    tag: String,
    generated: bool,
) -> Result<Outcome> {
    let now = jobs::ts(Utc::now());
    let json = cached.as_ref().map(serde_json::to_string).transpose()?;
    let key = format!("sum-{}", req.cache_key());
    let mode = req.mode.as_str().to_string();
    let done = run_tx(repo.db(), RetryPolicy::default(), |tx| {
        let (now, json, key, mode, tag) = (
            now.clone(),
            json.clone(),
            key.clone(),
            mode.clone(),
            tag.clone(),
        );
        async move {
            jobs::bump(&tx, "job_guard", &claimed.root_id).await?;
            match jobs::fence(&tx, claimed, &now).await? {
                Some(f) if jobs::holds(&f, claimed) => {}
                _ => return Ok(false),
            }
            if let Some(json) = json {
                run(
                    &tx,
                    tx.query(
                        "UPSERT $r SET mode = $mode, result_json = $json, fallback = false,
                           updated_at = $now RETURN NONE;",
                    )
                    .bind(("r", rid("derived_summary", &key)))
                    .bind(("mode", mode))
                    .bind(("json", json))
                    .bind(("now", now.clone())),
                )
                .await?;
            }
            jobs::complete_in(&tx, claimed, &tag, &now).await?;
            Ok(true)
        }
    })
    .await?;
    if !done.value {
        return Ok(lease_lost());
    }
    Ok(Outcome {
        tag,
        generated,
        stale: false,
    })
}
