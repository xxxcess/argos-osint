//! The summary flush pool on SurrealDB (`summarization.rs` flush cache and
//! `scheduler::drain_summary_flush_lazy` on the repository).
//!
//! A caller publishes a deterministic summary at once ([`publish`]): the
//! local result and the full request are stored, and a `summary` pool task
//! keyed by the request's cache key is queued. An elected worker ([`drain`])
//! claims it and, when a summarization secret is configured, runs
//! [`complete_summary`] and upgrades the cached result. A model result never
//! gets replaced by a fallback.
//!
//! Topology-neighbourhood requests need the whole role chain and are
//! dispatched by the neighbourhood brief port; this pool leaves them queued.

use anyhow::Result;
use chrono::Utc;
use surrealdb::types::SurrealValue;

use super::jobs::{self, rid, run, ClaimedTask, NewJob, NewTask};
use super::repo::Repository;
use super::tx::{run_tx, RetryPolicy};
use crate::secrets::ProviderSecret;
use crate::summarization::{
    complete_summary, sha_hex, SummarizationMode, SummaryRequest, SummaryResult,
};

/// The summary pool (`tasks::POOL_SUMMARY`).
pub const POOL: &str = crate::tasks::POOL_SUMMARY;

pub const SCHEMA: &str = "
DEFINE TABLE IF NOT EXISTS derived_summary SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS mode ON derived_summary TYPE string;
DEFINE FIELD IF NOT EXISTS result_json ON derived_summary TYPE string;
DEFINE FIELD IF NOT EXISTS fallback ON derived_summary TYPE bool;
DEFINE FIELD IF NOT EXISTS updated_at ON derived_summary TYPE string;
DEFINE TABLE IF NOT EXISTS summary_flush_request SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS request_json ON summary_flush_request TYPE string;
DEFINE FIELD IF NOT EXISTS created_at ON summary_flush_request TYPE string;
";

fn local(req: &SummaryRequest) -> SummaryRequest {
    let mut l = req.clone();
    l.provider = "local".into();
    l.model = "deterministic".into();
    l
}

#[derive(Debug, SurrealValue)]
struct Row {
    result_json: String,
}

async fn get_exact(repo: &Repository, req: &SummaryRequest) -> Result<Option<SummaryResult>> {
    if req.sources.iter().any(|s| sha_hex(&s.text) != s.hash) {
        return Ok(None);
    }
    let mut r = repo
        .db()
        .query("SELECT result_json FROM ONLY $r")
        .bind((
            "r",
            rid("derived_summary", &format!("sum-{}", req.cache_key())),
        ))
        .await?
        .check()?;
    let row: Option<Row> = r.take(0usize)?;
    Ok(row.and_then(|r| serde_json::from_str(&r.result_json).ok()))
}

/// `cache_get`: the exact model result, else its local fallback.
pub async fn cache_get(repo: &Repository, req: &SummaryRequest) -> Result<Option<SummaryResult>> {
    if let Some(hit) = get_exact(repo, req).await? {
        return Ok(Some(hit));
    }
    Ok(get_exact(repo, &local(req)).await?.filter(|r| r.fallback))
}

/// `cache_put`: a fallback never replaces a model result.
pub async fn cache_put(
    repo: &Repository,
    req: &SummaryRequest,
    result: &SummaryResult,
) -> Result<()> {
    anyhow::ensure!(
        req.sources.iter().all(|s| sha_hex(&s.text) == s.hash),
        "summary source hash mismatch"
    );
    repo.db()
        .query(
            "IF !($r.fallback ?? true) AND $fallback { RETURN NONE } ELSE {
               UPSERT $r SET mode = $mode, result_json = $json, fallback = $fallback,
                 updated_at = $now RETURN NONE
             };",
        )
        .bind((
            "r",
            rid("derived_summary", &format!("sum-{}", req.cache_key())),
        ))
        .bind(("mode", req.mode.as_str().to_string()))
        .bind(("json", serde_json::to_string(result)?))
        .bind(("fallback", result.fallback))
        .bind(("now", Utc::now().to_rfc3339()))
        .await?
        .check()?;
    Ok(())
}

#[derive(Debug, SurrealValue)]
struct ReqRow {
    request_json: String,
}

/// `load_flush_request`.
pub async fn load_request(repo: &Repository, key: &str) -> Result<Option<SummaryRequest>> {
    let mut r = repo
        .db()
        .query("SELECT request_json FROM ONLY $r")
        .bind(("r", rid("summary_flush_request", key)))
        .await?
        .check()?;
    let row: Option<ReqRow> = r.take(0usize)?;
    Ok(match row {
        Some(row) => Some(serde_json::from_str(&row.request_json)?),
        None => None,
    })
}

/// `publish_deterministic_and_enqueue`: cache `deterministic`, store the
/// request and queue one flush task per cache key. Returns the cached model
/// result when one exists, else `deterministic`.
pub async fn publish(
    repo: &Repository,
    req: &SummaryRequest,
    deterministic: SummaryResult,
) -> Result<SummaryResult> {
    let mut req = req.clone();
    if req.provider.is_empty() {
        let (provider, model) = crate::summarization::resolved_summary_route();
        req.provider = provider;
        req.model = model;
    }
    if let Some(hit) = cache_get(repo, &req).await? {
        if !hit.fallback {
            return Ok(hit);
        }
    }
    let mut deterministic = deterministic;
    deterministic.source_hashes = req.sources.iter().map(|s| s.hash.clone()).collect();
    req.deterministic = Some(Box::new(deterministic.clone()));
    cache_put(repo, &local(&req), &deterministic).await?;
    let key = req.cache_key();
    let json = serde_json::to_string(&req)?;
    let (job_id, task_id) = (format!("sum-job-{key}"), format!("sum-task-{key}"));
    let op = req.mode.as_str().to_string();
    let now = Utc::now();
    repo.db()
        .query("UPSERT $r SET request_json = $json, created_at = $now RETURN NONE")
        .bind(("r", rid("summary_flush_request", &key)))
        .bind(("json", json))
        .bind(("now", now.to_rfc3339()))
        .await?
        .check()?;
    // The task is created once per cache key; a later publish refreshes the
    // stored request only.
    let exists = task_state(repo, &task_id).await?.is_some();
    if !exists {
        let created = jobs::create_job(
            repo.db(),
            &NewJob {
                id: &job_id,
                kind: "summarization_flush",
                operation: &op,
                parent: None,
                created_at: now,
            },
            &[NewTask {
                id: &task_id,
                job: &job_id,
                pool: POOL,
                operation: &op,
                priority: 50,
                deps: Vec::new(),
                input_hash: &key,
                created_at: now,
            }],
        )
        .await;
        if let Err(e) = created {
            // A concurrent publish of the same key created it first.
            if task_state(repo, &task_id).await?.is_none() {
                anyhow::bail!("summary flush task: {e}");
            }
        }
    }
    Ok(deterministic)
}

/// The flush task's state, `None` when it does not exist.
pub async fn task_state(repo: &Repository, task_id: &str) -> Result<Option<String>> {
    let mut r = repo
        .db()
        .query("SELECT VALUE state FROM ONLY $t")
        .bind(("t", rid("task", task_id)))
        .await?
        .check()?;
    Ok(r.take::<Option<String>>(0usize)?)
}

/// One drained task's outcome (`scheduler::FlushOutcome` tags).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub task_id: String,
    pub tag: String,
    pub generated: bool,
}

/// `try_live_summary_upgrade` on the repository.
async fn upgrade(
    repo: &Repository,
    secret: Option<&ProviderSecret>,
    req: &SummaryRequest,
) -> Result<(SummaryResult, &'static str)> {
    let Some(cached) = cache_get(repo, req)
        .await?
        .or_else(|| req.deterministic.as_deref().cloned())
    else {
        anyhow::bail!("no cached or deterministic summary for {}", req.cache_key());
    };
    if !cached.fallback {
        return Ok((cached, "already_upgraded"));
    }
    let Some(secret) = secret else {
        return Ok((cached, "cached_deterministic_no_secret"));
    };
    let upgraded = complete_summary(secret, req, cached).await;
    cache_put(repo, req, &upgraded).await?;
    Ok(if upgraded.fallback {
        (upgraded, "llm_fallback")
    } else {
        (upgraded, "llm_upgraded")
    })
}

/// End `claimed` under its lease fence: `completed`, `failed`, or back to
/// `queued` (attempt refunded). `false` when the lease was lost.
async fn end(repo: &Repository, claimed: &ClaimedTask, state: &str, hash: &str) -> Result<bool> {
    let now = jobs::ts(Utc::now());
    let (state, hash) = (state.to_string(), hash.to_string());
    let done = run_tx(repo.db(), RetryPolicy::default(), |tx| {
        let (now, state, hash) = (now.clone(), state.clone(), hash.clone());
        async move {
            jobs::bump(&tx, "job_guard", &claimed.root_id).await?;
            match jobs::fence(&tx, claimed, &now).await? {
                Some(f) if jobs::holds(&f, claimed) => {}
                _ => return Ok(false),
            }
            if state == "completed" {
                jobs::complete_in(&tx, claimed, &hash, &now).await?;
                return Ok(true);
            }
            run(
                &tx,
                tx.query(
                    "UPDATE $t SET state = $state, lease_owner = '', process_owner = '',
                       lease_until = NONE, next_eligible_at = <datetime>$now,
                       attempts = IF $refund THEN array::max([attempts - 1, 0]) ELSE attempts END,
                       finished_at = IF $refund THEN NONE ELSE <datetime>$now END,
                       updated_at = <datetime>$now RETURN NONE;
                     LET $open = SELECT VALUE id FROM attempt WHERE task = $t AND epoch = $e
                       AND finished_at = NONE;
                     UPDATE $open SET finished_at = <datetime>$now, outcome = $state RETURN NONE;",
                )
                .bind(("t", rid("task", &claimed.id)))
                .bind(("state", state.clone()))
                .bind(("refund", state == "queued"))
                .bind(("now", now))
                .bind(("e", claimed.lease_epoch)),
            )
            .await?;
            Ok(true)
        }
    })
    .await?;
    Ok(done.value)
}

/// Drain up to eight summary tasks for `owner` (the caller holds the
/// scheduler election). `secret` loads at most once, after a first claim.
pub async fn drain(
    repo: &Repository,
    owner: &str,
    secret: impl FnOnce() -> Option<ProviderSecret>,
    lease_secs: i64,
) -> Result<Vec<Outcome>> {
    let mut out = Vec::new();
    let mut load = Some(secret);
    let mut secret: Option<ProviderSecret> = None;
    for n in 0..8 {
        let now = Utc::now();
        let op_key = format!(
            "sum-claim-{owner}-{}-{n}",
            now.timestamp_nanos_opt().unwrap_or(0)
        );
        let Some(claimed) = jobs::claim_next(repo.db(), POOL, owner, lease_secs, now, &op_key)
            .await
            .map_err(|e| anyhow::anyhow!("summary claim: {e}"))?
            .value
        else {
            break;
        };
        let outcome = |tag: &str, generated: bool| Outcome {
            task_id: claimed.id.clone(),
            tag: tag.to_string(),
            generated,
        };
        let req = match load_request(repo, &claimed.input_hash).await {
            Ok(Some(req)) => req,
            Ok(None) | Err(_) => {
                end(repo, &claimed, "failed", "").await?;
                out.push(outcome("failed", false));
                continue;
            }
        };
        if req.mode == SummarizationMode::TopologyNeighborhood {
            end(repo, &claimed, "queued", "").await?;
            out.push(outcome("role_dispatch_required", false));
            break;
        }
        if let Some(load) = load.take() {
            secret = load();
        }
        match upgrade(repo, secret.as_ref(), &req).await {
            Ok((summary, tag)) => {
                end(repo, &claimed, "completed", tag).await?;
                out.push(outcome(tag, !summary.fallback));
            }
            Err(_) => {
                end(repo, &claimed, "failed", "").await?;
                out.push(outcome("failed", false));
            }
        }
    }
    Ok(out)
}
