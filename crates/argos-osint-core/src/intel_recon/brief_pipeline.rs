//! Intel brief preparation (`intel-brief-v2`): one durable job per article
//! generation with four operations and explicit dependencies
//! (`body=[]; claims=[]; inferences=[body,claims]; links=[inferences]`).
//!
//! Admission is idempotent and atomic: one active chain per article, all four
//! tasks persisted immediately. Full article and Claims are independent roots
//! and may run together; dependencies live in the claim SQL
//! (`tasks::claim_with`), and stages are routed to the dedicated
//! [`tasks::POOL_INTEL_BRIEF`] pool so unrelated drainers never take them. A
//! stage commit stores its immutable output/hash and, in the same fenced
//! transaction, runs the durable join ([`bind_ready`]): a descendant is bound
//! once, by compare-and-set, only when every prerequisite has committed.

use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

use crate::tasks::{self, ClaimedTask};

/// Article-preparation contract version (distinct from the Recon/report
/// extraction contract).
pub const CONTRACT: &str = "intel-brief-v2";
/// The legacy serial contract. Completed v1 history stays intact; active v1
/// chains are fenced at cutover and replaced by a v2 generation.
pub const CONTRACT_V1: &str = "intel-brief-v1";
/// Brain canonical extraction scope. Unchanged by the v2 cutover so Brain
/// Claim/Inference identities stay stable.
pub const CANONICAL_CONTRACT: &str = "intel-brief-v1";

/// Explicit v2 dependencies: (operation, prerequisites).
pub const DEPENDENCIES: [(&str, &[&str]); 4] = [
    ("intel_brief_body", &[]),
    ("intel_brief_claims", &[]),
    (
        "intel_brief_inferences",
        &["intel_brief_body", "intel_brief_claims"],
    ),
    ("intel_brief_links", &["intel_brief_inferences"]),
];

/// Declared prerequisites of an operation.
pub fn prerequisites(operation: &str) -> &'static [&'static str] {
    DEPENDENCIES
        .iter()
        .find(|(op, _)| *op == operation)
        .map(|(_, deps)| *deps)
        .unwrap_or(&[])
}

fn label_of(operation: &str) -> &'static str {
    STAGES
        .iter()
        .find(|(op, _)| *op == operation)
        .map(|(_, l)| *l)
        .unwrap_or("stage")
}

/// Short key used in a bound join input (`{body:{..},claims:{..}}`).
fn short_key(operation: &str) -> &str {
    operation.strip_prefix("intel_brief_").unwrap_or(operation)
}

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
    /// Per-node semantic revisions in [`STAGES`] order (spec §6). Each node
    /// is keyed independently: its own source revision, contract/prompt
    /// version and resolved route/policy. Empty disables reuse.
    pub stage_revisions: Vec<String>,
    /// Immutable metadata snapshot (title/description/published/source
    /// revision) persisted in the root task inputs, so both branches read
    /// the same captured values instead of mutable article rows.
    pub metadata: serde_json::Value,
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
    let active: Option<(String, String, String, String)> = tx
        .query_row(
            "SELECT job_id,input_revision,state,contract FROM intel_brief_runs WHERE article_id=?1 AND state IN ('active','completed','failed','cancelled') ORDER BY generation DESC LIMIT 1",
            [&inputs.article_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    let mut generation = tx.query_row(
        "SELECT COALESCE(MAX(generation),0) FROM intel_brief_runs WHERE article_id=?1",
        [&inputs.article_id],
        |r| r.get::<_, i64>(0),
    )?;
    if let Some((job, revision, state, contract)) = &active {
        // Unchanged inputs: attach (completed chains issue no new requests;
        // a failed chain is attached until explicit Retry). A Reload
        // revision (`<base>:reload:<at>`) is the same inputs plus a forced
        // re-acquisition; ordinary visits attach to it.
        let same = *revision == inputs.input_revision
            || revision.starts_with(&format!("{}:reload:", inputs.input_revision));
        if same && contract == CONTRACT {
            tx.commit()?;
            return Ok(job.clone());
        }
        if state == "active" {
            // v1 cutover and changed inputs both fence the active chain.
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
    let force_body = inputs.input_revision.contains(":reload:");
    for (index, (operation, _)) in STAGES.iter().enumerate() {
        let id = task_id(&job_id, operation);
        let root = prerequisites(operation).is_empty();
        let revision = inputs
            .stage_revisions
            .get(index)
            .cloned()
            .unwrap_or_default();
        // Roots are bound at admission with their own captured inputs;
        // descendants stay unbound until the join.
        let (input_ref, input_hash) = if !root {
            (String::new(), String::new())
        } else {
            let input = serde_json::json!({
                "article_id": inputs.article_id,
                "force_body": *operation == "intel_brief_body" && force_body,
                "metadata": inputs.metadata,
                "node_revision": revision,
            });
            let hash =
                crate::nlp::cache::hash(&serde_json::json!([operation, revision]).to_string());
            (input.to_string(), hash)
        };
        tasks::enqueue_task(
            &tx,
            &tasks::NewTask {
                id: id.clone(),
                job_id: job_id.clone(),
                operation: (*operation).into(),
                dedupe_key: String::new(),
                priority: index as i64,
                input_ref,
                input_hash,
                source_revision: generation.to_string(),
                role_snapshot: inputs.role_snapshot.clone(),
                max_attempts: tasks::DEFAULT_LLM_ATTEMPTS,
            },
            now,
        )?;
        for dep in prerequisites(operation) {
            tx.execute(
                "INSERT OR IGNORE INTO argos_task_dependencies(task_id,depends_on) VALUES(?1,?2)",
                params![id, task_id(&job_id, dep)],
            )?;
        }
    }
    tx.execute(
        "INSERT INTO intel_brief_runs(job_id,article_id,generation,contract,input_revision,state,created_at,updated_at,stage_revisions_json) VALUES(?1,?2,?3,?4,?5,'active',?6,?6,?7)",
        params![job_id, inputs.article_id, generation, CONTRACT, inputs.input_revision, now, serde_json::to_string(&inputs.stage_revisions)?],
    )?;
    reuse_checkpoints(&tx, inputs, &job_id, generation, force_body, now)?;
    tasks::refresh_job_state(&tx, &job_id, now)?;
    tx.commit()?;
    Ok(job_id)
}

/// Per-node reuse (spec §6). Roots reuse a prior v2 output whose own node
/// revision is unchanged (a forced body reload skips only the body). Reused
/// roots then enter the same join as live commits, and a descendant reuses a
/// prior output whose bound input hash (prerequisite output hashes + node
/// revision) matches, repeating until no further node completes. Links is
/// never reused: its candidate set is read live. Returns reused nodes.
fn reuse_checkpoints(
    tx: &Connection,
    inputs: &BriefInputs,
    job_id: &str,
    generation: i64,
    force_body: bool,
    now: &str,
) -> Result<usize> {
    let mut reused = 0;
    for (index, (operation, _)) in STAGES.iter().enumerate() {
        if !prerequisites(operation).is_empty() || (force_body && *operation == "intel_brief_body")
        {
            continue;
        }
        let Some(revision) = inputs.stage_revisions.get(index).filter(|r| !r.is_empty()) else {
            continue;
        };
        let prior: Option<(String, String, i64, String)> = tx
            .query_row(
                "SELECT o.output_json,o.output_hash,o.item_count,o.empty_reason
                 FROM intel_brief_runs r
                 JOIN intel_brief_stage_outputs o ON o.task_id = r.job_id || ':' || ?4
                 JOIN argos_tasks t ON t.id = o.task_id AND t.state='completed'
                 WHERE r.article_id=?1 AND r.generation<?2 AND r.contract=?5
                   AND json_extract(r.stage_revisions_json, '$[' || ?3 || ']') = ?6
                   AND o.empty_reason NOT LIKE 'warning:%'
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
        if let Some(prior) = prior {
            reused += adopt(tx, job_id, operation, prior, now)?;
        }
    }
    loop {
        bind_ready(tx, job_id, &inputs.stage_revisions, now)?;
        let mut progressed = 0;
        for (operation, _) in STAGES.iter() {
            if prerequisites(operation).is_empty() || *operation == "intel_brief_links" {
                continue;
            }
            let id = task_id(job_id, operation);
            let bound: Option<String> = tx
                .query_row(
                    "SELECT input_hash FROM argos_tasks WHERE id=?1 AND state='queued' AND input_hash<>''",
                    [&id],
                    |r| r.get(0),
                )
                .optional()?;
            let Some(bound) = bound else { continue };
            let prior: Option<(String, String, i64, String)> = tx
                .query_row(
                    "SELECT o.output_json,o.output_hash,o.item_count,o.empty_reason
                     FROM argos_tasks t
                     JOIN intel_brief_stage_outputs o ON o.task_id=t.id
                     JOIN intel_brief_runs r ON r.job_id=t.job_id
                     WHERE t.operation=?1 AND t.state='completed' AND t.input_hash=?2
                       AND t.job_id<>?3 AND r.article_id=?4 AND r.contract=?5
                       AND o.empty_reason NOT LIKE 'warning:%'
                     ORDER BY r.generation DESC LIMIT 1",
                    params![operation, bound, job_id, inputs.article_id, CONTRACT],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
                .optional()?;
            if let Some(prior) = prior {
                progressed += adopt(tx, job_id, operation, prior, now)?;
            }
        }
        reused += progressed;
        if progressed == 0 {
            break;
        }
    }
    Ok(reused)
}

/// Mark a queued node completed from a prior committed output.
fn adopt(
    tx: &Connection,
    job_id: &str,
    operation: &str,
    (output, hash, items, empty): (String, String, i64, String),
    now: &str,
) -> Result<usize> {
    let id = task_id(job_id, operation);
    let changed = tx.execute(
        "UPDATE argos_tasks SET state='completed', result_ref=?2, error_message='Reused checkpoint · unchanged inputs', updated_at=?3, finished_at=?3 WHERE id=?1 AND state='queued'",
        params![id, hash, now],
    )?;
    if changed == 0 {
        return Ok(0);
    }
    tx.execute(
        "INSERT OR REPLACE INTO intel_brief_stage_outputs(task_id,job_id,stage,output_hash,output_json,item_count,empty_reason,committed_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
        params![id, job_id, operation, hash, output, items, empty, now],
    )?;
    Ok(1)
}

/// The durable join: bind every unbound descendant whose prerequisites have
/// all committed in this generation. The bound input carries each
/// prerequisite's `{task_id, output_hash}`; the input hash covers only the
/// prerequisite output hashes plus the node's semantic revision, so task
/// ownership IDs never defeat reuse across generations. Compare-and-set on
/// an empty `input_hash` makes the bind succeed exactly once, in either
/// commit order. Returns the number of nodes bound.
pub fn bind_ready(tx: &Connection, job_id: &str, revisions: &[String], now: &str) -> Result<usize> {
    let mut bound = 0;
    for (index, (operation, _)) in STAGES.iter().enumerate() {
        let deps = prerequisites(operation);
        if deps.is_empty() {
            continue;
        }
        let mut input = serde_json::Map::new();
        let mut hashes = Vec::new();
        let mut ready = true;
        for dep in deps {
            let dep_id = task_id(job_id, dep);
            let hash: Option<String> = tx
                .query_row(
                    "SELECT o.output_hash FROM intel_brief_stage_outputs o JOIN argos_tasks t ON t.id=o.task_id
                     WHERE o.task_id=?1 AND t.state='completed'",
                    [&dep_id],
                    |r| r.get(0),
                )
                .optional()?;
            let Some(hash) = hash else {
                ready = false;
                break;
            };
            input.insert(
                short_key(dep).to_string(),
                serde_json::json!({"task_id": dep_id, "output_hash": hash}),
            );
            hashes.push(hash);
        }
        if !ready {
            continue;
        }
        let revision = revisions.get(index).cloned().unwrap_or_default();
        let key =
            crate::nlp::cache::hash(&serde_json::json!([operation, revision, hashes]).to_string());
        bound += tx.execute(
            "UPDATE argos_tasks SET input_ref=?2,input_hash=?3,updated_at=?4
             WHERE id=?1 AND input_hash='' AND state IN ('queued','retry_scheduled')",
            params![
                task_id(job_id, operation),
                serde_json::Value::Object(input).to_string(),
                key,
                now
            ],
        )?;
    }
    Ok(bound)
}

fn run_revisions(conn: &Connection, job_id: &str) -> Result<Vec<String>> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT stage_revisions_json FROM intel_brief_runs WHERE job_id=?1",
            [job_id],
            |r| r.get(0),
        )
        .optional()?;
    Ok(raw
        .and_then(|r| serde_json::from_str(&r).ok())
        .unwrap_or_default())
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

/// [`commit_stage`] carrying the completed kind's prepared canonical inputs,
/// so publication and the index outbox share the fenced stage transaction.
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
    // A completed-with-warnings stage keeps its checkpoints so Retry re-asks
    // only the items that failed.
    if !empty_reason.starts_with("warning:") {
        checkpoint_clear(&tx, &claimed.job_id, &claimed.operation)?;
    }
    let last = STAGES.last().map(|(op, _)| *op) == Some(claimed.operation.as_str());
    if !last {
        let revisions = run_revisions(&tx, &claimed.job_id)?;
        bind_ready(&tx, &claimed.job_id, &revisions, now)?;
    } else {
        tx.execute(
            "UPDATE intel_brief_runs SET state='completed',updated_at=?2 WHERE job_id=?1",
            params![claimed.job_id, now],
        )?;
    }
    publish_completed_stage(&tx, &claimed.job_id, &claimed.operation, canonical, now)?;
    tasks::refresh_job_state(&tx, &claimed.job_id, now)?;
    tx.commit()?;
    Ok(true)
}

/// Publish an immutable completed stage. Normal callers already hold the
/// stage's lease fence; cache repair additionally rechecks the latest run.
pub(crate) fn publish_completed_stage(
    tx: &Connection,
    job_id: &str,
    operation: &str,
    canonical: Option<&std::result::Result<super::brief_canonical::CanonicalBrief, String>>,
    now: &str,
) -> Result<()> {
    if ![
        "intel_brief_claims",
        "intel_brief_inferences",
        "intel_brief_links",
    ]
    .contains(&operation)
    {
        return Ok(());
    }
    let current: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM intel_brief_runs r JOIN argos_tasks t ON t.job_id=r.job_id
         WHERE r.job_id=?1 AND t.operation=?2 AND t.state='completed' AND r.state IN ('active','completed','failed')
         AND NOT EXISTS(SELECT 1 FROM intel_brief_runs n WHERE n.article_id=r.article_id AND n.generation>r.generation))",
        params![job_id,operation], |r| r.get(0),
    )?;
    if current {
        if let Some((article_id, snapshot_hash)) = publish(tx, job_id, operation, now)? {
            super::brief_canonical::publish_stage_on(
                tx,
                &article_id,
                job_id,
                operation,
                canonical,
                &snapshot_hash,
                now,
            )?;
        }
    }
    Ok(())
}

/// Merge only the completed kind into the displayed brief. New generations
/// cannot be overwritten by an old worker, and claims need no body receipt.
fn publish(
    tx: &Connection,
    job_id: &str,
    operation: &str,
    now: &str,
) -> Result<Option<(String, String)>> {
    let (article_id, generation): (String, i64) = tx.query_row(
        "SELECT article_id,generation FROM intel_brief_runs WHERE job_id=?1",
        [job_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let output = |op: &str| -> Result<serde_json::Value> {
        let raw: Option<String> = tx
            .query_row(
                "SELECT output_json FROM intel_brief_stage_outputs WHERE task_id=?1",
                [task_id(job_id, op)],
                |r| r.get(0),
            )
            .optional()?;
        Ok(raw
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_else(|| serde_json::json!({})))
    };
    let body = output("intel_brief_body")?;
    let claims = output("intel_brief_claims")?;
    let inferences = output("intel_brief_inferences")?;
    let links = output("intel_brief_links")?;
    let snapshot = serde_json::json!([operation, output(operation)?]).to_string();
    let snapshot_hash = crate::nlp::cache::hash(&snapshot);
    let written = tx.execute(
        "INSERT INTO intel_brief_publications(article_id,job_id,generation,contract,body_hash,snapshot_hash,claims_json,inferences_json,leads_json,published_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
         ON CONFLICT(article_id) DO UPDATE SET job_id=excluded.job_id,generation=excluded.generation,contract=excluded.contract,body_hash=excluded.body_hash,snapshot_hash=excluded.snapshot_hash,
         claims_json=CASE WHEN ?11='intel_brief_claims' THEN excluded.claims_json ELSE intel_brief_publications.claims_json END,
         inferences_json=CASE WHEN ?11='intel_brief_inferences' THEN excluded.inferences_json ELSE intel_brief_publications.inferences_json END,
         leads_json=CASE WHEN ?11='intel_brief_links' THEN excluded.leads_json ELSE intel_brief_publications.leads_json END,published_at=excluded.published_at
         WHERE excluded.generation>=intel_brief_publications.generation",
        params![
            article_id, job_id, generation, CONTRACT,
            body["content_hash"].as_str().unwrap_or(""),
            snapshot_hash,
            claims.to_string(), inferences.to_string(), links["leads"].to_string(), now, operation
        ],
    )?;
    if written == 0 {
        return Ok(None);
    }
    if operation != "intel_brief_links" {
        return Ok(Some((article_id, snapshot_hash)));
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

/// Explicit Retry: resume every failed branch only, preserving completed
/// siblings and predecessors; blocked descendants unblock as they commit.
/// Returns the first resumed task id.
pub fn retry_stage(conn: &Connection, job_id: &str, now: &str) -> Result<Option<String>> {
    let failed: Vec<String> = {
        let mut stmt = conn.prepare(
            "SELECT id FROM argos_tasks WHERE job_id=?1 AND state='failed' ORDER BY priority",
        )?;
        let rows = stmt
            .query_map([job_id], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        rows
    };
    if failed.is_empty() {
        return retry_warnings(conn, job_id, now);
    }
    for id in &failed {
        conn.execute(
            "UPDATE argos_tasks SET state='queued',attempts=0,recoveries=0,error_category='',error_message='',finished_at='',next_eligible_at='',updated_at=?2 WHERE id=?1 AND state='failed'",
            params![id, now],
        )?;
    }
    conn.execute(
        "UPDATE intel_brief_runs SET state='active',updated_at=?2 WHERE job_id=?1 AND state='failed'",
        params![job_id, now],
    )?;
    tasks::refresh_job_state(conn, job_id, now)?;
    Ok(failed.into_iter().next())
}

fn checkpoint_key(job_id: &str, stage: &str, item: &str) -> String {
    format!("intel-brief-ckpt:{job_id}:{stage}:{item}")
}

/// A per-item checkpoint of a stage that fans out (one model request per
/// claim). Valid only for the exact input hash it was written for.
pub fn checkpoint_get(
    conn: &Connection,
    job_id: &str,
    stage: &str,
    item: &str,
    input_hash: &str,
) -> Result<Option<serde_json::Value>> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT value FROM app_state WHERE key=?1",
            [checkpoint_key(job_id, stage, item)],
            |r| r.get(0),
        )
        .optional()?;
    Ok(raw
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .filter(|v| v["hash"] == input_hash)
        .map(|v| v["output"].clone()))
}

pub fn checkpoint_put(
    conn: &Connection,
    job_id: &str,
    stage: &str,
    item: &str,
    input_hash: &str,
    output: &serde_json::Value,
) -> Result<()> {
    conn.execute(
        "INSERT INTO app_state(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![
            checkpoint_key(job_id, stage, item),
            serde_json::json!({"hash": input_hash, "output": output}).to_string()
        ],
    )?;
    Ok(())
}

/// Drop a stage's checkpoints once its output is committed.
pub fn checkpoint_clear(conn: &Connection, job_id: &str, stage: &str) -> Result<()> {
    let prefix = checkpoint_key(job_id, stage, "");
    conn.execute(
        "DELETE FROM app_state WHERE substr(key,1,?2)=?1",
        params![prefix, prefix.chars().count() as i64],
    )?;
    Ok(())
}

/// Retry for a stage that completed with warnings (some items failed): reopen
/// it and everything downstream of it. Its per-item checkpoints survive, so
/// only the failed items are asked again; descendants rebind to the new
/// output when it commits. Nothing is reopened while any stage is running.
fn retry_warnings(conn: &Connection, job_id: &str, now: &str) -> Result<Option<String>> {
    let running: i64 = conn.query_row(
        "SELECT COUNT(*) FROM argos_tasks WHERE job_id=?1 AND state IN ('running','retry_scheduled')",
        [job_id],
        |r| r.get(0),
    )?;
    if running > 0 {
        return Ok(None);
    }
    let mut reopened: Vec<&str> = Vec::new();
    for (operation, _) in STAGES {
        let warned: bool = conn
            .query_row(
                "SELECT 1 FROM intel_brief_stage_outputs WHERE task_id=?1 AND empty_reason LIKE 'warning:%'",
                [task_id(job_id, operation)],
                |_| Ok(true),
            )
            .optional()?
            .unwrap_or(false);
        if warned
            || prerequisites(operation)
                .iter()
                .any(|p| reopened.contains(p))
        {
            reopened.push(operation);
        }
    }
    let Some(first) = reopened.first().copied() else {
        return Ok(None);
    };
    for operation in &reopened {
        let id = task_id(job_id, operation);
        let root_of_retry = *operation == first;
        conn.execute(
            "DELETE FROM intel_brief_stage_outputs WHERE task_id=?1",
            [&id],
        )?;
        conn.execute(
            "UPDATE argos_tasks SET state='queued',attempts=0,recoveries=0,error_category='',error_message='',result_ref='',finished_at='',next_eligible_at='',
                    input_hash=CASE WHEN ?3 THEN input_hash ELSE '' END,
                    input_ref=CASE WHEN ?3 THEN input_ref ELSE '' END,updated_at=?2
             WHERE id=?1",
            params![id, now, root_of_retry],
        )?;
    }
    conn.execute(
        "UPDATE intel_brief_runs SET state='active',updated_at=?2 WHERE job_id=?1",
        params![job_id, now],
    )?;
    tasks::refresh_job_state(conn, job_id, now)?;
    Ok(Some(task_id(job_id, first)))
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
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StageView {
    pub operation: String,
    pub label: &'static str,
    pub state: String,
    /// Display row, e.g. `Queued · after Claims`, `Completed · 3 items`.
    pub row: String,
    pub active: bool,
    /// Extra lines for a failed stage or one blocked by a failed predecessor:
    /// the failure, a Providers hint for a missing key, and how to retry.
    /// Empty when there is nothing to say.
    pub detail: String,
}

/// Longest failure text shown in a stage pane.
const DETAIL_ERROR_CHARS: usize = 220;

/// First line of a stage error, bounded for display.
fn detail_error(reason: &str) -> String {
    let first = reason.lines().next().unwrap_or("").trim();
    let mut out: String = first.chars().take(DETAIL_ERROR_CHARS).collect();
    if first.chars().count() > DETAIL_ERROR_CHARS {
        out.push('…');
    }
    out
}

/// Pane lines for a failed stage (`own`) or one blocked by `subject`'s failure.
/// Compact by design: a failed stage adds its full error only when the row
/// truncated it, plus one recovery line; a blocked stage adds one line.
fn failure_detail(subject: &str, own: bool, error: &str) -> String {
    let error = detail_error(error);
    if error.is_empty() {
        return String::new();
    }
    let lower = error.to_lowercase();
    let recovery = if lower.contains("api key missing") || lower.contains("all routes blocked") {
        "Configure a provider in Providers, then Retry article retrieval."
    } else {
        "Retry article retrieval resumes the failed stage."
    };
    if !own {
        // The cause sits in the failed stage's own pane; only recovery repeats.
        return format!("{subject} failed · {recovery}");
    }
    if short(&error) != error {
        format!("{error}\n{recovery}")
    } else {
        recovery.to_string()
    }
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

/// Stage views for a chain, in order, computed from the actual dependency
/// rows: a queued node names whichever prerequisites are still pending, or
/// the failed prerequisite (transitively) that blocks it.
pub fn stage_views(conn: &Connection, job_id: &str) -> Result<Vec<StageView>> {
    stage_views_from(|operation| {
        let id = task_id(job_id, operation);
        let row: Option<(String, String, String)> = conn
            .query_row(
                "SELECT state,error_message,next_eligible_at FROM argos_tasks WHERE id=?1",
                [&id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let Some((state, error, _eligible)) = row else {
            return Ok(None);
        };
        let deps: Vec<(String, String)> = {
            let mut stmt = conn.prepare(
                "SELECT p.operation,p.state FROM argos_task_dependencies d JOIN argos_tasks p ON p.id=d.depends_on
                 WHERE d.task_id=?1 ORDER BY p.priority",
            )?;
            let rows = stmt
                .query_map([&id], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        let output: Option<(i64, String)> = conn
            .query_row(
                "SELECT item_count,empty_reason FROM intel_brief_stage_outputs WHERE task_id=?1",
                [&id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let activity = if state == "running" {
            conn.query_row(
                "SELECT value FROM app_state WHERE key=?1",
                [format!("intel-brief-activity:{id}")],
                |r| r.get(0),
            )
            .optional()
            .unwrap_or(None)
        } else {
            None
        };
        Ok(Some(StageRow {
            state,
            error,
            deps,
            output,
            activity,
        }))
    })
}

/// One stage task as the views need it, from either store.
#[derive(Debug, Clone, Default)]
pub struct StageRow {
    pub state: String,
    pub error: String,
    /// Prerequisites `(operation, state)` in priority order.
    pub deps: Vec<(String, String)>,
    /// Committed `(item_count, empty_reason)`.
    pub output: Option<(i64, String)>,
    /// Live activity label of a running stage.
    pub activity: Option<String>,
}

/// [`stage_views`] over rows from `fetch` (by operation; `None` skips it).
pub fn stage_views_from(
    mut fetch: impl FnMut(&str) -> Result<Option<StageRow>>,
) -> Result<Vec<StageView>> {
    let mut out: Vec<StageView> = Vec::new();
    let mut failed_by: std::collections::HashMap<String, (&'static str, String)> =
        Default::default();
    for (operation, label) in STAGES {
        let Some(StageRow {
            state,
            error,
            deps,
            output,
            activity,
        }) = fetch(operation)?
        else {
            continue;
        };
        // A failed prerequisite registers itself in `failed_by` below, so one
        // lookup covers direct and transitive blockers.
        let blocker_full = deps.iter().find_map(|(op, _)| failed_by.get(op).cloned());
        let blocker = blocker_full.as_ref().map(|(label, _)| *label);
        let text = match state.as_str() {
            "completed" => match output {
                Some((0, reason)) if !reason.is_empty() => format!("Completed · {reason}"),
                Some((n, reason)) if reason.contains("warning") => {
                    format!(
                        "Completed with warnings · {n} item{}",
                        if n == 1 { "" } else { "s" }
                    )
                }
                Some((n, _)) => format!("Completed · {n} item{}", if n == 1 { "" } else { "s" }),
                None => "Completed".into(),
            },
            "running" => match activity.filter(|a| !a.is_empty()) {
                Some(a) => format!("Running · {a}"),
                None => "Running".into(),
            },
            "retry_scheduled" => "Running · retry scheduled".into(),
            "failed" => format!("Failed · {}", short(&error)),
            "cancelled" => "Cancelled".into(),
            "superseded" => "Superseded".into(),
            _ => match blocker {
                Some(b) => format!("Queued · blocked by {b}"),
                None => {
                    let pending: Vec<&str> = deps
                        .iter()
                        .filter(|(_, st)| st != "completed")
                        .map(|(op, _)| label_of(op))
                        .collect();
                    if pending.is_empty() {
                        "Queued".into()
                    } else {
                        format!("Queued · after {}", pending.join(", "))
                    }
                }
            },
        };
        let detail = if state == "failed" {
            failure_detail(label, true, &error)
        } else if state == "queued" {
            blocker_full
                .as_ref()
                .map(|(who, err)| failure_detail(who, false, err))
                .unwrap_or_default()
        } else {
            String::new()
        };
        if state == "failed" {
            failed_by.insert(operation.to_string(), (label, error.clone()));
        } else if state != "completed" {
            if let Some(b) = blocker_full {
                failed_by.insert(operation.to_string(), b);
            }
        }
        out.push(StageView {
            detail,
            operation: operation.into(),
            label,
            active: state == "running",
            state,
            row: text,
        });
    }
    Ok(out)
}

/// Truthful chain summary for headers and Jobs: `2 running · 0/4 completed`.
pub fn chain_summary(views: &[StageView]) -> String {
    let running = views.iter().filter(|v| v.state == "running").count();
    let completed = views.iter().filter(|v| v.state == "completed").count();
    let failed = views.iter().filter(|v| v.state == "failed").count();
    let mut parts = Vec::new();
    if running > 0 {
        parts.push(format!("{running} running"));
    }
    if failed > 0 {
        parts.push(format!("{failed} failed"));
    }
    parts.push(format!("{completed}/{} completed", views.len()));
    parts.join(" · ")
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
