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
    /// Per-stage dependency hashes in [`STAGES`] order (spec §3). A new
    /// generation reuses a predecessor's committed output for each leading
    /// stage whose hash is unchanged; empty disables reuse.
    pub stage_revisions: Vec<String>,
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
        // A Reload revision (`<base>:reload:<at>`) is the same inputs plus a
        // forced re-acquisition; ordinary visits attach to it.
        if *revision == inputs.input_revision
            || revision.starts_with(&format!("{}:reload:", inputs.input_revision))
        {
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
                    serde_json::json!({"article_id":inputs.article_id,"force_body":inputs.input_revision.contains(":reload:")}).to_string()
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
        "INSERT INTO intel_brief_runs(job_id,article_id,generation,contract,input_revision,state,created_at,updated_at,stage_revisions_json) VALUES(?1,?2,?3,?4,?5,'active',?6,?6,?7)",
        params![job_id, inputs.article_id, generation, CONTRACT, inputs.input_revision, now, serde_json::to_string(&inputs.stage_revisions)?],
    )?;
    if !inputs.input_revision.contains(":reload:") {
        reuse_checkpoints(&tx, inputs, &job_id, generation, now)?;
    }
    tasks::refresh_job_state(&tx, &job_id, now)?;
    tx.commit()?;
    Ok(job_id)
}

/// Reuse leading stage checkpoints whose dependency hash is unchanged from
/// an earlier generation. Links (the last stage) always runs in a new
/// generation. Returns the number of reused stages.
fn reuse_checkpoints(
    tx: &Connection,
    inputs: &BriefInputs,
    job_id: &str,
    generation: i64,
    now: &str,
) -> Result<usize> {
    let mut reused = 0;
    for (index, (operation, _)) in STAGES.iter().enumerate().take(STAGES.len() - 1) {
        let Some(revision) = inputs.stage_revisions.get(index).filter(|r| !r.is_empty()) else {
            break;
        };
        let prior: Option<(String, String, i64, String)> = tx
            .query_row(
                "SELECT o.output_json,o.output_hash,o.item_count,o.empty_reason
                 FROM intel_brief_runs r
                 JOIN intel_brief_stage_outputs o ON o.task_id = r.job_id || ':' || ?4
                 JOIN argos_tasks t ON t.id = o.task_id AND t.state='completed'
                 WHERE r.article_id=?1 AND r.generation<?2 AND r.contract=?5
                   AND json_extract(r.stage_revisions_json, '$[' || ?3 || ']') = ?6
                 ORDER BY r.generation DESC LIMIT 1",
                params![
                    inputs.article_id,
                    generation,
                    index as i64,
                    operation,
                    CONTRACT,
                    revision
                ],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        let Some((output, hash, items, empty)) = prior else {
            break;
        };
        let id = task_id(job_id, operation);
        tx.execute(
            "INSERT OR REPLACE INTO intel_brief_stage_outputs(task_id,job_id,stage,output_hash,output_json,item_count,empty_reason,committed_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![id, job_id, operation, hash, output, items, empty, now],
        )?;
        tx.execute(
            "UPDATE argos_tasks SET state='completed', result_ref=?2, error_message='Reused checkpoint · unchanged inputs', updated_at=?3, finished_at=?3 WHERE id=?1 AND state='queued'",
            params![id, hash, now],
        )?;
        if let Some((next_op, _)) = STAGES.get(index + 1) {
            tx.execute(
                "UPDATE argos_tasks SET input_ref=?2,input_hash=?3,updated_at=?4 WHERE id=?1 AND state='queued'",
                params![task_id(job_id, next_op), serde_json::json!({"predecessor":id,"output_hash":hash,"reused":true}).to_string(), hash, now],
            )?;
        }
        reused += 1;
    }
    Ok(reused)
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
    commit_stage_with(
        conn,
        claimed,
        output_json,
        item_count,
        empty_reason,
        now,
        None,
    )
}

/// [`commit_stage`] carrying the prepared canonical inputs for the final stage,
/// so brief publication, canonical Brain derivations and the index outbox share
/// the one fenced transaction.
pub fn commit_stage_with(
    conn: &Connection,
    claimed: &ClaimedTask,
    output_json: &str,
    item_count: usize,
    empty_reason: &str,
    now: &str,
    canonical: Option<&std::result::Result<super::brief_canonical::CanonicalBrief, String>>,
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
        if let Some((article_id, snapshot_hash)) = publish(&tx, &claimed.job_id, output_json, now)?
        {
            super::brief_canonical::publish_on(
                &tx,
                &article_id,
                &claimed.job_id,
                canonical,
                &snapshot_hash,
                now,
            )?;
        }
    }
    tasks::refresh_job_state(&tx, &claimed.job_id, now)?;
    tx.commit()?;
    Ok(true)
}

/// Final Stage-4 publication, inside the fenced commit transaction: the
/// combined validated snapshot (claims, inferences, leads) and its leads rows.
/// The previous publication stays until this succeeds; an older generation
/// never overwrites a newer one. Separate from canonical report snapshots, so
/// a completed Recon report keeps precedence.
fn publish(
    tx: &Connection,
    job_id: &str,
    links_json: &str,
    now: &str,
) -> Result<Option<(String, String)>> {
    let (article_id, generation): (String, i64) = tx.query_row(
        "SELECT article_id,generation FROM intel_brief_runs WHERE job_id=?1",
        [job_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let output = |op: &str| -> Result<serde_json::Value> {
        let raw: String = tx.query_row(
            "SELECT output_json FROM intel_brief_stage_outputs WHERE task_id=?1",
            [task_id(job_id, op)],
            |r| r.get(0),
        )?;
        Ok(serde_json::from_str(&raw)?)
    };
    let body = output("intel_brief_body")?;
    let claims = output("intel_brief_claims")?;
    let inferences = output("intel_brief_inferences")?;
    let links: serde_json::Value = serde_json::from_str(links_json)?;
    let snapshot = serde_json::json!([body["content_hash"], claims, inferences, links]).to_string();
    let snapshot_hash = crate::nlp::cache::hash(&snapshot);
    let written = tx.execute(
        "INSERT INTO intel_brief_publications(article_id,job_id,generation,contract,body_hash,snapshot_hash,claims_json,inferences_json,leads_json,published_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
         ON CONFLICT(article_id) DO UPDATE SET job_id=excluded.job_id,generation=excluded.generation,contract=excluded.contract,body_hash=excluded.body_hash,snapshot_hash=excluded.snapshot_hash,claims_json=excluded.claims_json,inferences_json=excluded.inferences_json,leads_json=excluded.leads_json,published_at=excluded.published_at
         WHERE excluded.generation>=intel_brief_publications.generation",
        params![
            article_id, job_id, generation, CONTRACT,
            body["content_hash"].as_str().unwrap_or(""),
            snapshot_hash,
            claims.to_string(), inferences.to_string(), links["leads"].to_string(), now
        ],
    )?;
    if written == 0 {
        return Ok(None);
    }
    tx.execute(
        "DELETE FROM intel_inference_article_leads WHERE article_id=?1 AND source_generation<=?2",
        params![article_id, generation],
    )?;
    let texts: std::collections::HashMap<String, String> = inferences["inferences"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|i| {
            Some((
                i["id"].as_str()?.to_string(),
                i["text"].as_str()?.to_string(),
            ))
        })
        .collect();
    for lead in links["leads"].as_array().into_iter().flatten() {
        let inference = lead["inference_id"].as_str().unwrap_or("");
        let candidate = lead["article_id"].as_str().unwrap_or("");
        tx.execute(
            "INSERT OR REPLACE INTO intel_inference_article_leads(id,article_id,source_generation,job_id,inference_id,inference_text,candidate_article_id,candidate_run_id,candidate_metadata_revision,interpretation,verification_needed,citations_json,created_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
            params![
                format!("{article_id}:{generation}:{inference}:{candidate}"),
                article_id, generation, job_id, inference,
                texts.get(inference).cloned().unwrap_or_default(),
                candidate,
                lead["article_run_id"].as_str().unwrap_or(""),
                lead["metadata_revision"].as_str().unwrap_or(""),
                lead["why"].as_str().unwrap_or(""),
                lead["verification_needed"].as_str().unwrap_or(""),
                lead["metadata_evidence"].to_string(),
                now
            ],
        )?;
    }
    Ok(Some((article_id, snapshot_hash)))
}

/// One published lead row for presentation ("Potential corroboration").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublishedLead {
    pub inference_id: String,
    pub inference_text: String,
    pub candidate_article_id: String,
    pub candidate_run_id: String,
    pub interpretation: String,
    pub verification_needed: String,
}

pub fn published_leads(conn: &Connection, article_id: &str) -> Result<Vec<PublishedLead>> {
    let mut stmt = conn.prepare(
        "SELECT inference_id,inference_text,candidate_article_id,candidate_run_id,interpretation,verification_needed FROM intel_inference_article_leads WHERE article_id=?1 ORDER BY source_generation DESC, rowid",
    )?;
    let rows = stmt
        .query_map([article_id], |r| {
            Ok(PublishedLead {
                inference_id: r.get(0)?,
                inference_text: r.get(1)?,
                candidate_article_id: r.get(2)?,
                candidate_run_id: r.get(3)?,
                interpretation: r.get(4)?,
                verification_needed: r.get(5)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Published brief (claims/inferences JSON) for an article, if any.
pub fn published_brief(
    conn: &Connection,
    article_id: &str,
) -> Result<Option<(i64, serde_json::Value, serde_json::Value)>> {
    let row: Option<(i64, String, String)> = conn
        .query_row(
            "SELECT generation,claims_json,inferences_json FROM intel_brief_publications WHERE article_id=?1",
            [article_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    Ok(match row {
        Some((g, c, i)) => Some((g, serde_json::from_str(&c)?, serde_json::from_str(&i)?)),
        None => None,
    })
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
            "running" => {
                let activity: Option<String> = conn
                    .query_row(
                        "SELECT value FROM app_state WHERE key=?1",
                        [format!(
                            "intel-brief-activity:{}",
                            task_id(job_id, operation)
                        )],
                        |r| r.get(0),
                    )
                    .optional()
                    .unwrap_or(None);
                match activity.filter(|a| !a.is_empty()) {
                    Some(a) => format!("Running · {a}"),
                    None => "Running".into(),
                }
            }
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

/// Committed stage outputs of a chain, keyed by operation.
pub fn stage_outputs(
    conn: &Connection,
    job_id: &str,
) -> Result<std::collections::HashMap<String, serde_json::Value>> {
    let mut stmt =
        conn.prepare("SELECT stage,output_json FROM intel_brief_stage_outputs WHERE job_id=?1")?;
    let rows = stmt
        .query_map([job_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows
        .into_iter()
        .filter_map(|(stage, json)| Some((stage, serde_json::from_str(&json).ok()?)))
        .collect())
}

/// Record the running stage's current activity for every surface.
pub fn set_activity(conn: &Connection, task: &str, activity: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO app_state(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![format!("intel-brief-activity:{task}"), activity],
    )?;
    Ok(())
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
