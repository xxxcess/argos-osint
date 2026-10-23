//! The Recon run executor's writes on SurrealDB (migration phase 5, Recon
//! remainder part 2b): the final answer (`Store::add_answer`), run
//! recovery (`Store::recover_runs`), investigation strategies and the
//! model operation ledger the executor streams drafts into.
//! `extraction_jobs` → `extraction_job` (keyed by answer id),
//! `recon_model_operations` → `recon_model_operation` (by operation id),
//! `investigation_strategies` → `investigation_strategy` (by id).
//!
//! Nothing calls these yet; the Recon callers switch in a later commit.

use anyhow::{anyhow, Result};
use surrealdb::types::SurrealValue;

use super::credits::release_held_credits;
use super::jobs::{rid, run, take};
use super::recon_runs::{link_evidence_in, now};
use super::recon_threads::sequence_counter;
use super::repo::Repository;
use super::tx::{run_tx, RetryPolicy};
use crate::recon::Message;

/// Schema (idempotent).
pub const SCHEMA: &str = "DEFINE TABLE IF NOT EXISTS extraction_job SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS run_id ON extraction_job TYPE string;
DEFINE FIELD IF NOT EXISTS state ON extraction_job TYPE string;
DEFINE FIELD IF NOT EXISTS error ON extraction_job TYPE option<string>;
DEFINE FIELD IF NOT EXISTS updated_at ON extraction_job TYPE string;
DEFINE INDEX IF NOT EXISTS extraction_job_state ON extraction_job FIELDS state;

DEFINE TABLE IF NOT EXISTS recon_model_operation SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS run_id ON recon_model_operation TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS task_id ON recon_model_operation TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS role ON recon_model_operation TYPE string;
DEFINE FIELD IF NOT EXISTS generation ON recon_model_operation TYPE int DEFAULT 1;
DEFINE FIELD IF NOT EXISTS status ON recon_model_operation TYPE string;
DEFINE FIELD IF NOT EXISTS draft ON recon_model_operation TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS final_message_id ON recon_model_operation TYPE option<string>;
DEFINE FIELD IF NOT EXISTS created_at ON recon_model_operation TYPE string;
DEFINE FIELD IF NOT EXISTS updated_at ON recon_model_operation TYPE string;
DEFINE INDEX IF NOT EXISTS recon_model_operation_run ON recon_model_operation FIELDS run_id, role;

DEFINE TABLE IF NOT EXISTS investigation_strategy SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS thread_id ON investigation_strategy TYPE string;
DEFINE FIELD IF NOT EXISTS run_id ON investigation_strategy TYPE string;
DEFINE FIELD IF NOT EXISTS kind ON investigation_strategy TYPE string;
DEFINE FIELD IF NOT EXISTS rationale ON investigation_strategy TYPE string;
DEFINE FIELD IF NOT EXISTS previous_kind ON investigation_strategy TYPE option<string>;
DEFINE FIELD IF NOT EXISTS change_reason ON investigation_strategy TYPE option<string>;
DEFINE FIELD IF NOT EXISTS created_at ON investigation_strategy TYPE string;
DEFINE INDEX IF NOT EXISTS investigation_strategy_thread ON investigation_strategy FIELDS thread_id, created_at;";

/// `Store::add_answer`: in one transaction, check the run is still
/// running on a live thread, append the assistant message, cite the
/// evidence calls (a missing call fails the answer) and the memories
/// that still exist (in order), queue its extraction job, finalize the
/// run's latest synthesis operation, touch the thread and mark the run's
/// stage complete.
pub async fn add_answer(
    repo: &Repository,
    thread_id: &str,
    run_id: &str,
    content: &str,
    evidence_ids: &[String],
    memory_ids: &[String],
) -> Result<Message> {
    let base = Message {
        id: crate::recon::id("msg"),
        thread_id: thread_id.into(),
        sequence: 0,
        role: "assistant".into(),
        content: content.into(),
        run_id: Some(run_id.into()),
        created_at: now(),
    };
    let (m, run_id) = (base.clone(), run_id.to_string());
    let (evidence, memories) = (evidence_ids.to_vec(), memory_ids.to_vec());
    let done = run_tx(repo.db(), RetryPolicy::default(), |tx| {
        let (m, run_id) = (m.clone(), run_id.clone());
        let (evidence, memories) = (evidence.clone(), memories.clone());
        async move {
            let mut r = run(
                &tx,
                tx.query(
                    "LET $run = (SELECT state, thread_id FROM ONLY type::record('recon_run', $rid));
                     IF $run.state != 'running' OR $run.thread_id != $t
                        OR (SELECT VALUE deleted FROM ONLY type::record('recon_thread', $t)) != false {
                       THROW 'run no longer active';
                     };
                     LET $top = math::max(SELECT VALUE sequence FROM recon_message WHERE thread_id = $t) ?? 0;
                     LET $n = (UPSERT $c SET n = math::max([n ?? 0, $top]) + 1 RETURN VALUE n)[0];
                     CREATE $m SET thread_id = $t, sequence = $n, role = 'assistant', content = $body,
                       run_id = $rid, created_at = $at RETURN NONE;
                     RETURN $n;",
                )
                .bind(("rid", run_id.clone()))
                .bind(("t", m.thread_id.clone()))
                .bind(("c", rid("counter", &sequence_counter(&m.thread_id))))
                .bind(("m", rid("recon_message", &m.id)))
                .bind(("body", m.content.clone()))
                .bind(("at", m.created_at.clone())),
            )
            .await?;
            let last = r.num_statements() - 1;
            let n: Option<i64> = take(&mut r, last)?;
            link_evidence_in(&tx, &m.id, &evidence).await?;
            run(
                &tx,
                tx.query(
                    "LET $live = array::filter($mems, |$x| record::exists(type::record('memory', $x)));
                     FOR $i IN array::range(0, array::len($mems)) {
                       IF $mems[$i] INSIDE $live {
                         UPSERT type::record('recon_message_memory', [$m, $mems[$i]])
                           SET message_id = $m, memory_id = $mems[$i], ordinal = $i RETURN NONE;
                       };
                     };
                     UPSERT type::record('extraction_job', $m)
                       SET run_id = $rid, state = 'queued', error = NONE, updated_at = $at RETURN NONE;
                     LET $op = (SELECT VALUE id FROM recon_model_operation
                       WHERE run_id = $rid AND role = 'synthesis' ORDER BY created_at DESC LIMIT 1)[0];
                     IF $op != NONE {
                       UPDATE $op SET status = 'final', final_message_id = $m, updated_at = $at RETURN NONE;
                     };
                     UPDATE type::record('recon_thread', $t) SET updated_at = $at RETURN NONE;
                     UPDATE type::record('recon_run', $rid) SET stage = 'complete', updated_at = $at
                       WHERE state = 'running' RETURN NONE;",
                )
                .bind(("mems", memories))
                .bind(("m", m.id.clone()))
                .bind(("rid", run_id))
                .bind(("t", m.thread_id.clone()))
                .bind(("at", m.created_at.clone())),
            )
            .await?;
            Ok(n)
        }
    })
    .await
    .map_err(|e| anyhow!("add answer: {e}"))?;
    let sequence = done
        .value
        .ok_or_else(|| anyhow!("add answer: no sequence"))?;
    Ok(Message { sequence, ..base })
}

/// `Store::has_final_answer_for_run`: a final synthesis operation exists.
pub async fn has_final_answer_for_run(repo: &Repository, run_id: &str) -> Result<bool> {
    let mut r = repo
        .db()
        .query(
            "RETURN count(SELECT id FROM recon_model_operation
               WHERE run_id = $rid AND role = 'synthesis' AND status = 'final') > 0",
        )
        .bind(("rid", run_id.to_string()))
        .await?
        .check()?;
    Ok(r.take::<Option<bool>>(0usize)?.unwrap_or(false))
}

/// `Store::recover_runs` (startup): running runs become interrupted,
/// queued/running calls interrupted, running extraction jobs requeued and
/// every held credit released. Returns the number of runs interrupted.
pub async fn recover_runs(repo: &Repository) -> Result<usize> {
    let mut r = repo
        .db()
        .query(
            "UPDATE recon_run SET state = 'interrupted', stage = 'interrupted', updated_at = $at
               WHERE state = 'running' RETURN VALUE meta::id(id);
             UPDATE osint_call SET status = 'interrupted', completed_at = $at
               WHERE status INSIDE ['queued', 'running'] RETURN NONE;
             UPDATE extraction_job SET state = 'queued', updated_at = $at
               WHERE state = 'running' RETURN NONE;",
        )
        .bind(("at", now()))
        .await?
        .check()?;
    let ids: Vec<String> = r.take(0usize)?;
    release_held_credits(repo.db()).await?;
    Ok(ids.len())
}

/// The model operation the executor streams into (`model_exec` upsert):
/// create it running, or move an existing one back to running at
/// `generation`, keeping its draft unless a new one is given.
pub async fn begin_operation(
    repo: &Repository,
    id: &str,
    run_id: Option<&str>,
    task_id: Option<&str>,
    role: &str,
    generation: i64,
    draft: &str,
) -> Result<()> {
    repo.db()
        .query(
            "UPSERT $o SET run_id = $run, task_id = $task, role = $role, generation = $g,
               status = 'running', draft = IF $draft != '' { $draft } ELSE { draft ?? '' },
               created_at = created_at ?? $at, updated_at = $at RETURN NONE",
        )
        .bind(("o", rid("recon_model_operation", id)))
        .bind(("run", run_id.unwrap_or("").to_string()))
        .bind(("task", task_id.unwrap_or("").to_string()))
        .bind(("role", role.to_string()))
        .bind(("g", generation))
        .bind(("draft", draft.to_string()))
        .bind(("at", now()))
        .await?
        .check()?;
    Ok(())
}

/// Replace an operation's streamed draft.
pub async fn set_operation_draft(repo: &Repository, id: &str, draft: &str) -> Result<()> {
    repo.db()
        .query("UPDATE $o SET draft = $d, updated_at = $at RETURN NONE")
        .bind(("o", rid("recon_model_operation", id)))
        .bind(("d", draft.to_string()))
        .bind(("at", now()))
        .await?
        .check()?;
    Ok(())
}

/// Close an operation with `status`; a given final message replaces the
/// stored one, none keeps it.
pub async fn finish_operation(
    repo: &Repository,
    id: &str,
    status: &str,
    final_message_id: Option<&str>,
) -> Result<()> {
    repo.db()
        .query(
            "UPDATE $o SET status = $s, final_message_id = $f ?? final_message_id,
               updated_at = $at RETURN NONE",
        )
        .bind(("o", rid("recon_model_operation", id)))
        .bind(("s", status.to_string()))
        .bind(("f", final_message_id.map(str::to_string)))
        .bind(("at", now()))
        .await?
        .check()?;
    Ok(())
}

/// One model operation, as the executor and tests read it back.
#[derive(Debug, Clone, PartialEq, SurrealValue)]
pub struct Operation {
    pub id: String,
    pub run_id: String,
    pub role: String,
    pub generation: i64,
    pub status: String,
    pub draft: String,
    pub final_message_id: Option<String>,
}

/// A run's operations for `role`, newest generation first.
pub async fn operations(repo: &Repository, run_id: &str, role: &str) -> Result<Vec<Operation>> {
    let mut r = repo
        .db()
        .query(
            "SELECT meta::id(id) AS id, run_id, role, generation, status, draft, final_message_id
             FROM recon_model_operation WHERE run_id = $rid AND role = $role
             ORDER BY generation DESC",
        )
        .bind(("rid", run_id.to_string()))
        .bind(("role", role.to_string()))
        .await?
        .check()?;
    Ok(r.take(0usize)?)
}

/// `Store::record_strategy`.
pub async fn record_strategy(
    repo: &Repository,
    thread_id: &str,
    run_id: &str,
    kind: &str,
    rationale: &str,
    previous_kind: Option<&str>,
    change_reason: Option<&str>,
) -> Result<()> {
    repo.db()
        .query(
            "CREATE $s SET thread_id = $t, run_id = $run, kind = $kind, rationale = $why,
               previous_kind = $prev, change_reason = $reason, created_at = $at RETURN NONE",
        )
        .bind((
            "s",
            rid("investigation_strategy", &crate::recon::id("strategy")),
        ))
        .bind(("t", thread_id.to_string()))
        .bind(("run", run_id.to_string()))
        .bind(("kind", kind.to_string()))
        .bind(("why", rationale.to_string()))
        .bind(("prev", previous_kind.map(str::to_string)))
        .bind(("reason", change_reason.map(str::to_string)))
        .bind(("at", now()))
        .await?
        .check()?;
    Ok(())
}

/// `Store::latest_strategy_kind`.
pub async fn latest_strategy_kind(repo: &Repository, thread_id: &str) -> Result<Option<String>> {
    let mut r = repo
        .db()
        .query(
            "SELECT VALUE kind FROM investigation_strategy WHERE thread_id = $t
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(("t", thread_id.to_string()))
        .await?
        .check()?;
    let kinds: Vec<String> = r.take(0usize)?;
    Ok(kinds.into_iter().next())
}

/// An answer's extraction job state (`queued`, `running`, ...).
pub async fn extraction_state(repo: &Repository, answer_id: &str) -> Result<Option<String>> {
    let mut r = repo
        .db()
        .query("SELECT VALUE state FROM ONLY $j")
        .bind(("j", rid("extraction_job", answer_id)))
        .await?
        .check()?;
    Ok(r.take::<Option<String>>(0usize)?)
}

/// Set an answer's extraction job state (the extraction worker's claim).
pub async fn set_extraction_state(repo: &Repository, answer_id: &str, state: &str) -> Result<()> {
    repo.db()
        .query("UPDATE $j SET state = $s, updated_at = $at RETURN NONE")
        .bind(("j", rid("extraction_job", answer_id)))
        .bind(("s", state.to_string()))
        .bind(("at", now()))
        .await?
        .check()?;
    Ok(())
}
