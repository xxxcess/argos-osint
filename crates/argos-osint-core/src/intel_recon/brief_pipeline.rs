//! Intel brief preparation: one durable job per article generation with four
//! dependent stages (Full article → Claims → Inferences → Links).
//!
//! Admission is idempotent and atomic: one active chain per article, all four
//! tasks persisted immediately, only the first claimable. Dependencies and the
//! one-active-stage guard live in the claim SQL (`tasks::claim_with`); stages are
//! routed to the dedicated [`tasks::POOL_INTEL_BRIEF`] pool so unrelated drainers
//! never take them. A stage commit stores its immutable output/hash, binds the
//! next stage's exact input, and acknowledges completion in one transaction.

use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

use crate::tasks::{self, ClaimedTask};

/// Article-preparation contract version (distinct from the Recon/report
/// extraction contract).
pub const CONTRACT: &str = "intel-brief-v1";

/// (operation, display label) in dependency order.
pub const STAGES: [(&str, &str); 4] = [
    ("intel_brief_body", "Full article"),
    ("intel_brief_claims", "Claims"),
    ("intel_brief_inferences", "Inferences"),
    ("intel_brief_links", "Links"),
];

/// Stable identity of one preparation run's inputs.
#[derive(Clone, Debug, Default)]
pub struct BriefInputs {
    pub article_id: String,
    /// Article metadata/acquisition revision, related metadata revisions and
    /// captured role chain folded into one fingerprint by the caller.
    pub input_revision: String,
    /// Captured role settings (JSON) for every stage.
    pub role_snapshot: String,
    pub title: String,
}

/// Task id of one stage in a chain.
pub fn task_id(job_id: &str, operation: &str) -> String {
    format!("{job_id}:{operation}")
}

/// Ensure one active chain for this article's inputs; returns its job id.
///
/// Visits, body-ready events and recovery all call this and attach to the
/// existing chain. A changed input revision supersedes the old chain (queued
/// stages are retired; a running stage is fenced by the cancelled job). Failed
/// unchanged work is returned as-is and waits for explicit [`retry_stage`].
pub fn ensure_chain(conn: &Connection, inputs: &BriefInputs, now: &str) -> Result<String> {
    let tx = rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)?;
    let active: Option<(String, String, String)> = tx
        .query_row(
            "SELECT job_id,input_revision,state FROM intel_brief_runs WHERE article_id=?1 AND state IN ('active','completed') ORDER BY generation DESC LIMIT 1",
            [&inputs.article_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let mut generation = tx.query_row(
        "SELECT COALESCE(MAX(generation),0) FROM intel_brief_runs WHERE article_id=?1",
        [&inputs.article_id],
        |r| r.get::<_, i64>(0),
    )?;
    if let Some((job, revision, state)) = &active {
        // Unchanged inputs: attach (completed chains issue no new requests).
        if *revision == inputs.input_revision {
            tx.commit()?;
            return Ok(job.clone());
        }
        if state == "active" {
            supersede_run(&tx, job, now)?;
        }
    }
    generation += 1;
    let job_id = format!("intel-brief:{}:{generation}", inputs.article_id);
    tasks::enqueue_job_with(
        &tx,
        &tasks::NewJob {
            id: job_id.clone(),
            kind: "intel_brief".into(),
            owner_scope: format!("article:{}", inputs.article_id),
            input_revision: inputs.input_revision.clone(),
            deadline_at: String::new(),
        },
        &tasks::JobMeta {
            app: "intel".into(),
            operation: "intel_brief".into(),
            title: if inputs.title.is_empty() {
                "Intel brief preparation".into()
            } else {
                format!("Intel brief preparation · {}", inputs.title)
            },
            resource_ref: inputs.article_id.clone(),
            ..Default::default()
        },
        now,
    )?;
    let mut previous: Option<String> = None;
    for (index, (operation, _)) in STAGES.iter().enumerate() {
        let id = task_id(&job_id, operation);
        tasks::enqueue_task(
            &tx,
            &tasks::NewTask {
                id: id.clone(),
                job_id: job_id.clone(),
                operation: (*operation).into(),
                dedupe_key: String::new(),
                priority: index as i64,
                input_ref: if index == 0 {
                    serde_json::json!({"article_id":inputs.article_id}).to_string()
                } else {
                    String::new()
                },
                input_hash: if index == 0 {
                    inputs.input_revision.clone()
                } else {
                    String::new()
                },
                source_revision: generation.to_string(),
                role_snapshot: inputs.role_snapshot.clone(),
                max_attempts: tasks::DEFAULT_LLM_ATTEMPTS,
            },
            now,
        )?;
        if let Some(prev) = previous.replace(id.clone()) {
            tx.execute(
                "INSERT OR IGNORE INTO argos_task_dependencies(task_id,depends_on) VALUES(?1,?2)",
                params![id, prev],
            )?;
        }
    }
    tx.execute(
        "INSERT INTO intel_brief_runs(job_id,article_id,generation,contract,input_revision,state,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,'active',?6,?6)",
        params![job_id, inputs.article_id, generation, CONTRACT, inputs.input_revision, now],
    )?;
    tx.commit()?;
    Ok(job_id)
}

fn supersede_run(conn: &Connection, job_id: &str, now: &str) -> Result<()> {
    conn.execute(
        "UPDATE intel_brief_runs SET state='superseded',updated_at=?2 WHERE job_id=?1",
        params![job_id, now],
    )?;
    conn.execute(
        "UPDATE argos_tasks SET state='superseded',error_message='Superseded · new article inputs',updated_at=?2,lease_epoch=lease_epoch+1,lease_owner='',lease_until='' WHERE job_id=?1 AND state IN ('queued','retry_scheduled','paused','running')",
        params![job_id, now],
    )?;
    tasks::refresh_job_state(conn, job_id, now)?;
    Ok(())
}

/// Claim the next eligible brief stage (dedicated pool only).
pub fn claim_stage(
    conn: &Connection,
    owner: &str,
    lease_secs: i64,
    now: &str,
) -> Result<Option<ClaimedTask>> {
    tasks::claim_next_in(conn, &[tasks::POOL_INTEL_BRIEF], owner, lease_secs, now)
}

/// Commit a stage: immutable output + hash, bind the next stage's exact input,
/// acknowledge completion and unblock the successor — one transaction, fenced
/// by the claim's owner/epoch. Returns false (and writes nothing) when stale.
pub fn commit_stage(
    conn: &Connection,
    claimed: &ClaimedTask,
    output_json: &str,
    item_count: usize,
    empty_reason: &str,
    now: &str,
) -> Result<bool> {
    let tx = rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)?;
    let hash = crate::nlp::cache::hash(output_json);
    if !tasks::complete_claimed(&tx, claimed, &hash, now)? {
        tx.rollback()?;
        return Ok(false);
    }
    tx.execute(
        "INSERT OR REPLACE INTO intel_brief_stage_outputs(task_id,job_id,stage,output_hash,output_json,item_count,empty_reason,committed_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
        params![claimed.id, claimed.job_id, claimed.operation, hash, output_json, item_count as i64, empty_reason, now],
    )?;
    let next = STAGES
        .iter()
        .position(|(op, _)| *op == claimed.operation)
        .and_then(|i| STAGES.get(i + 1));
    if let Some((next_op, _)) = next {
        tx.execute(
            "UPDATE argos_tasks SET input_ref=?2,input_hash=?3,updated_at=?4 WHERE id=?1 AND state IN ('queued','retry_scheduled')",
            params![task_id(&claimed.job_id, next_op), serde_json::json!({"predecessor":claimed.id,"output_hash":hash}).to_string(), hash, now],
        )?;
    } else {
        tx.execute(
            "UPDATE intel_brief_runs SET state='completed',updated_at=?2 WHERE job_id=?1",
            params![claimed.job_id, now],
        )?;
    }
    tasks::refresh_job_state(&tx, &claimed.job_id, now)?;
    tx.commit()?;
    Ok(true)
}

/// Explicit Retry: resume the failed stage only, preserving completed
/// predecessors; dependents unblock as it commits.
pub fn retry_stage(conn: &Connection, job_id: &str, now: &str) -> Result<Option<String>> {
    let failed: Option<String> = conn
        .query_row(
            "SELECT id FROM argos_tasks WHERE job_id=?1 AND state='failed' ORDER BY priority LIMIT 1",
            [job_id],
            |r| r.get(0),
        )
        .optional()?;
    let Some(id) = failed else {
        return Ok(None);
    };
    conn.execute(
        "UPDATE argos_tasks SET state='queued',attempts=0,recoveries=0,error_category='',error_message='',finished_at='',next_eligible_at='',updated_at=?2 WHERE id=?1 AND state='failed'",
        params![id, now],
    )?;
    tasks::refresh_job_state(conn, job_id, now)?;
    Ok(Some(id))
}

/// Cancellation/deletion: stop the owned chain and prevent publication.
pub fn cancel_chain(conn: &Connection, job_id: &str, now: &str) -> Result<()> {
    conn.execute(
        "UPDATE argos_jobs SET cancel_requested=1,updated_at=?2 WHERE id=?1",
        params![job_id, now],
    )?;
    conn.execute(
        "UPDATE argos_tasks SET state='cancelled',lease_epoch=lease_epoch+1,lease_owner='',lease_until='',updated_at=?2 WHERE job_id=?1 AND state IN ('queued','retry_scheduled','paused','running')",
        params![job_id, now],
    )?;
    conn.execute(
        "UPDATE intel_brief_runs SET state='cancelled',updated_at=?2 WHERE job_id=?1",
        params![job_id, now],
    )?;
    tasks::refresh_job_state(conn, job_id, now)?;
    Ok(())
}

/// One immutable stage row for every surface (brief Jobs panel, section
/// headers, global Jobs, Logs).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StageView {
    pub operation: String,
    pub label: &'static str,
    pub state: String,
    /// Display row, e.g. `Queued · after Claims`, `Completed · 3 items`.
    pub row: String,
    pub active: bool,
}

fn short(reason: &str) -> String {
    let first = reason.split(['\n', ';']).next().unwrap_or("").trim();
    let mut out: String = first.chars().take(60).collect();
    if first.chars().count() > 60 {
        out.push('…');
    }
    if out.is_empty() {
        "unknown error".into()
    } else {
        out
    }
}

/// Stage views for a chain, in order.
pub fn stage_views(conn: &Connection, job_id: &str) -> Result<Vec<StageView>> {
    let mut out: Vec<StageView> = Vec::new();
    let mut blocked_by: Option<&'static str> = None;
    for (operation, label) in STAGES {
        let row: Option<(String, String, String)> = conn
            .query_row(
                "SELECT state,error_message,next_eligible_at FROM argos_tasks WHERE id=?1",
                [task_id(job_id, operation)],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let Some((state, error, _eligible)) = row else {
            continue;
        };
        let output: Option<(i64, String)> = conn
            .query_row(
                "SELECT item_count,empty_reason FROM intel_brief_stage_outputs WHERE task_id=?1",
                [task_id(job_id, operation)],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let prev = out.last();
        let text = match state.as_str() {
            "completed" => match output {
                Some((0, reason)) if !reason.is_empty() => format!("Completed · {reason}"),
                Some((n, _)) => format!("Completed · {n} item{}", if n == 1 { "" } else { "s" }),
                None => "Completed".into(),
            },
            "running" => "Running".into(),
            "retry_scheduled" => "Running · retry scheduled".into(),
            "failed" => format!("Failed · {}", short(&error)),
            "cancelled" => "Cancelled".into(),
            "superseded" => "Superseded".into(),
            _ => match (blocked_by, prev) {
                (Some(blocker), _) => format!("Queued · blocked by {blocker}"),
                (None, Some(p)) if p.state != "completed" => format!("Queued · after {}", p.label),
                _ => "Queued".into(),
            },
        };
        if state == "failed" && blocked_by.is_none() {
            blocked_by = Some(label);
        }
        out.push(StageView {
            operation: operation.into(),
            label,
            active: state == "running",
            state,
            row: text,
        });
    }
    Ok(out)
}

/// Active chain job for an article, if any (completed chains count as current).
pub fn current_chain(conn: &Connection, article_id: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT job_id FROM intel_brief_runs WHERE article_id=?1 AND state IN ('active','completed') ORDER BY generation DESC LIMIT 1",
            [article_id],
            |r| r.get(0),
        )
        .optional()?)
}

#[cfg(test)]
#[path = "brief_pipeline_tests.rs"]
mod tests;
