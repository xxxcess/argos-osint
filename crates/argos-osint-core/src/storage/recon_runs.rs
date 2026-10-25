//! Recon runs, tool calls, answer evidence and the tool cache on SurrealDB
//! (migration phase 5, Recon remainder part 2a): `recon_runs` →
//! `recon_run`, `osint_calls` → `osint_call`, `recon_message_evidence` →
//! `recon_message_evidence` and `osint_cache` → `osint_cache`, with the
//! `Store` operations the Recon callers use on them.
//!
//! JSON columns (`plan_json`, `inputs_json`, `result_json`) stay JSON
//! strings, as in SQLite, so the stored text is byte-identical and
//! `inflight_duplicate` can compare inputs as text. Times are RFC 3339 UTC
//! strings (they sort as text).
//!
//! Record keys: `recon_run`, `osint_call` by id, `osint_cache` by cache key,
//! `recon_message_evidence` by `[message, call]`. A run needs its thread;
//! deleting a thread deletes its runs (event), and deleting a message
//! deletes its evidence rows. Calls are not deleted with their thread
//! (SQLite keeps them too); `delete_thread` handles them.

use anyhow::{anyhow, ensure, Result};
use chrono::Utc;
use serde_json::Value;
use surrealdb::types::SurrealValue;

use super::jobs::{rid, run};
use super::recon::get_thread;
use super::repo::Repository;
use super::tx::{run_tx, RetryPolicy};
use crate::osint::{self, ToolResult};
use crate::recon::{Call, Plan, Run, RunLimits};

/// Schema (idempotent).
pub const SCHEMA: &str = "DEFINE TABLE IF NOT EXISTS recon_run SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS thread_id ON recon_run TYPE string
  ASSERT record::exists(type::record('recon_thread', $value));
DEFINE FIELD IF NOT EXISTS turn_id ON recon_run TYPE string;
DEFINE FIELD IF NOT EXISTS state ON recon_run TYPE string;
DEFINE FIELD IF NOT EXISTS stage ON recon_run TYPE string;
DEFINE FIELD IF NOT EXISTS recon_model ON recon_run TYPE string;
DEFINE FIELD IF NOT EXISTS synthesis_model ON recon_run TYPE string;
DEFINE FIELD IF NOT EXISTS tool_picker_model ON recon_run TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS max_rounds ON recon_run TYPE int;
DEFINE FIELD IF NOT EXISTS max_calls ON recon_run TYPE int;
DEFINE FIELD IF NOT EXISTS turn_seconds ON recon_run TYPE int;
DEFINE FIELD IF NOT EXISTS plan_json ON recon_run TYPE option<string>;
DEFINE FIELD IF NOT EXISTS error ON recon_run TYPE option<string>;
DEFINE FIELD IF NOT EXISTS created_at ON recon_run TYPE string;
DEFINE FIELD IF NOT EXISTS updated_at ON recon_run TYPE string;
DEFINE INDEX IF NOT EXISTS recon_run_thread ON recon_run FIELDS thread_id, updated_at;
DEFINE INDEX IF NOT EXISTS recon_run_state ON recon_run FIELDS state;

DEFINE TABLE IF NOT EXISTS osint_call SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS tool_id ON osint_call TYPE string;
DEFINE FIELD IF NOT EXISTS run_id ON osint_call TYPE option<string>;
DEFINE FIELD IF NOT EXISTS thread_id ON osint_call TYPE option<string>;
DEFINE FIELD IF NOT EXISTS turn_id ON osint_call TYPE option<string>;
DEFINE FIELD IF NOT EXISTS origin ON osint_call TYPE string;
DEFINE FIELD IF NOT EXISTS inputs_json ON osint_call TYPE string;
DEFINE FIELD IF NOT EXISTS status ON osint_call TYPE string;
DEFINE FIELD IF NOT EXISTS attempts ON osint_call TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS result_json ON osint_call TYPE option<string>;
DEFINE FIELD IF NOT EXISTS started_at ON osint_call TYPE string;
DEFINE FIELD IF NOT EXISTS completed_at ON osint_call TYPE option<string>;
DEFINE INDEX IF NOT EXISTS osint_call_thread ON osint_call FIELDS thread_id, started_at;
DEFINE INDEX IF NOT EXISTS osint_call_run ON osint_call FIELDS run_id;
DEFINE INDEX IF NOT EXISTS osint_call_status ON osint_call FIELDS status;

DEFINE TABLE IF NOT EXISTS recon_message_evidence SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS message_id ON recon_message_evidence TYPE string
  ASSERT record::exists(type::record('recon_message', $value));
DEFINE FIELD IF NOT EXISTS call_id ON recon_message_evidence TYPE string
  ASSERT record::exists(type::record('osint_call', $value));
DEFINE INDEX IF NOT EXISTS recon_message_evidence_message ON recon_message_evidence FIELDS message_id;

DEFINE TABLE IF NOT EXISTS osint_cache SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS result_json ON osint_cache TYPE string;
DEFINE FIELD IF NOT EXISTS expires_at ON osint_cache TYPE string;

DEFINE EVENT IF NOT EXISTS recon_run_cascade ON TABLE recon_thread
  WHEN $event = 'DELETE' THEN {
    DELETE recon_run WHERE thread_id = record::id($before.id) RETURN NONE;
  };
DEFINE EVENT IF NOT EXISTS recon_evidence_cascade ON TABLE recon_message
  WHEN $event = 'DELETE' THEN {
    DELETE recon_message_evidence WHERE message_id = record::id($before.id) RETURN NONE;
  };";

pub(crate) fn now() -> String {
    Utc::now().to_rfc3339()
}

const RUN_FIELDS: &str = "meta::id(id) AS id, thread_id, turn_id, state, stage, recon_model, \
     synthesis_model, tool_picker_model, max_rounds, max_calls, turn_seconds, plan_json, error, \
     created_at, updated_at";

#[derive(Debug, Clone, SurrealValue)]
struct RunRow {
    id: String,
    thread_id: String,
    turn_id: String,
    state: String,
    stage: String,
    recon_model: String,
    synthesis_model: String,
    tool_picker_model: String,
    max_rounds: i64,
    max_calls: i64,
    turn_seconds: i64,
    plan_json: Option<String>,
    error: Option<String>,
    created_at: String,
    updated_at: String,
}

impl From<RunRow> for Run {
    fn from(r: RunRow) -> Self {
        Run {
            id: r.id,
            thread_id: r.thread_id,
            turn_id: r.turn_id,
            state: r.state,
            stage: r.stage,
            recon_model: r.recon_model,
            synthesis_model: r.synthesis_model,
            tool_picker_model: r.tool_picker_model,
            max_rounds: r.max_rounds as u8,
            max_calls: r.max_calls as u8,
            turn_seconds: r.turn_seconds as u16,
            plan_json: r.plan_json,
            error: r.error,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

/// `Store::new_run_with_models`: `models` is `[recon, tool picker,
/// synthesis]`; the limits are checked as in SQLite.
pub async fn new_run(
    repo: &Repository,
    thread_id: &str,
    turn_id: &str,
    models: [&str; 3],
    limits: RunLimits,
) -> Result<Run> {
    let [recon_model, tool_picker_model, synthesis_model] = models;
    ensure!(
        get_thread(repo, thread_id).await?.is_some(),
        "thread not found"
    );
    ensure!(
        (1..=8).contains(&limits.max_rounds),
        "max_rounds must be 1..8"
    );
    ensure!(
        (1..=24).contains(&limits.max_calls),
        "max_calls must be 1..24"
    );
    ensure!(
        (300..=900).contains(&limits.turn_seconds),
        "turn_seconds must be 300..900"
    );
    let time = now();
    let run = Run {
        id: crate::recon::id("run"),
        thread_id: thread_id.into(),
        turn_id: turn_id.into(),
        state: "running".into(),
        stage: "extracting entities".into(),
        recon_model: recon_model.into(),
        synthesis_model: synthesis_model.into(),
        tool_picker_model: tool_picker_model.into(),
        max_rounds: limits.max_rounds,
        max_calls: limits.max_calls,
        turn_seconds: limits.turn_seconds,
        plan_json: None,
        error: None,
        created_at: time.clone(),
        updated_at: time,
    };
    repo.db()
        .query(
            "CREATE $r SET thread_id = $t, turn_id = $turn, state = $state, stage = $stage, \
             recon_model = $rm, synthesis_model = $sm, tool_picker_model = $tm, \
             max_rounds = $mr, max_calls = $mc, turn_seconds = $ts, plan_json = NONE, \
             error = NONE, created_at = $at, updated_at = $at RETURN NONE",
        )
        .bind(("r", rid("recon_run", &run.id)))
        .bind(("t", run.thread_id.clone()))
        .bind(("turn", run.turn_id.clone()))
        .bind(("state", run.state.clone()))
        .bind(("stage", run.stage.clone()))
        .bind(("rm", run.recon_model.clone()))
        .bind(("sm", run.synthesis_model.clone()))
        .bind(("tm", run.tool_picker_model.clone()))
        .bind(("mr", i64::from(run.max_rounds)))
        .bind(("mc", i64::from(run.max_calls)))
        .bind(("ts", i64::from(run.turn_seconds)))
        .bind(("at", run.created_at.clone()))
        .await?
        .check()?;
    Ok(run)
}

pub async fn get_run(repo: &Repository, id: &str) -> Result<Option<Run>> {
    let mut r = repo
        .db()
        .query(format!("SELECT {RUN_FIELDS} FROM ONLY $r"))
        .bind(("r", rid("recon_run", id)))
        .await?
        .check()?;
    let row: Option<RunRow> = r.take(0usize)?;
    Ok(row.map(Run::from))
}

/// Runs of a thread, oldest first.
pub async fn runs_for_thread(repo: &Repository, thread_id: &str) -> Result<Vec<Run>> {
    let mut r = repo
        .db()
        .query(format!(
            "SELECT {RUN_FIELDS} FROM recon_run WHERE thread_id = $t ORDER BY created_at, id"
        ))
        .bind(("t", thread_id.to_string()))
        .await?
        .check()?;
    let rows: Vec<RunRow> = r.take(0usize)?;
    Ok(rows.into_iter().map(Run::from).collect())
}

async fn latest_run(repo: &Repository, thread_id: &str) -> Result<Option<Run>> {
    let mut r = repo
        .db()
        .query(format!(
            "SELECT {RUN_FIELDS} FROM recon_run WHERE thread_id = $t \
             ORDER BY updated_at DESC LIMIT 1"
        ))
        .bind(("t", thread_id.to_string()))
        .await?
        .check()?;
    let rows: Vec<RunRow> = r.take(0usize)?;
    Ok(rows.into_iter().next().map(Run::from))
}

/// State of the thread's most recently updated run.
pub async fn latest_run_state(repo: &Repository, thread_id: &str) -> Result<Option<String>> {
    Ok(latest_run(repo, thread_id).await?.map(|r| r.state))
}

/// Newest run state for each of `thread_ids`, in one query that reads only
/// one newest row per thread (`LIMIT 1` per thread, as in [`latest_run`]). Threads with no
/// run are absent. Ties on `updated_at` keep whichever row the engine
/// returns first, the same as [`latest_run_state`]'s `LIMIT 1`.
pub async fn latest_run_states(
    repo: &Repository,
    thread_ids: &[String],
) -> Result<std::collections::HashMap<String, String>> {
    if thread_ids.is_empty() {
        return Ok(std::collections::HashMap::new());
    }
    #[derive(SurrealValue)]
    struct Row {
        thread_id: String,
        state: String,
    }
    let mut r = repo
        .db()
        .query(
            "RETURN $ids.map(|$t| (SELECT thread_id, state FROM recon_run \
             WHERE thread_id = $t ORDER BY updated_at DESC LIMIT 1)[0])",
        )
        .bind(("ids", thread_ids.to_vec()))
        .await?
        .check()?;
    let rows: Vec<Option<Row>> = r.take(0usize)?;
    Ok(rows
        .into_iter()
        .flatten()
        .map(|row| (row.thread_id, row.state))
        .collect())
}

/// The most recently updated run, when it was interrupted or failed.
pub async fn latest_resumable_run(repo: &Repository, thread_id: &str) -> Result<Option<Run>> {
    Ok(latest_run(repo, thread_id)
        .await?
        .filter(|r| r.state == "interrupted" || r.state == "failed"))
}

async fn changed(r: surrealdb::IndexedResults) -> Result<bool> {
    let mut r = r.check()?;
    let ids: Vec<String> = r.take(0usize)?;
    Ok(!ids.is_empty())
}

/// `Store::set_run`: a new plan replaces the stored one, no plan keeps it.
/// Cancelled and deleted runs are final.
pub async fn set_run(
    repo: &Repository,
    id: &str,
    state: &str,
    stage: &str,
    plan: Option<&Plan>,
    error: Option<&str>,
) -> Result<bool> {
    let plan = plan.map(serde_json::to_string).transpose()?;
    changed(
        repo.db()
            .query(
                "UPDATE $r SET state = $state, stage = $stage, plan_json = $plan ?? plan_json, \
                 error = $err, updated_at = $at WHERE state NOTINSIDE ['cancelled', 'deleted'] \
                 RETURN VALUE meta::id(id)",
            )
            .bind(("r", rid("recon_run", id)))
            .bind(("state", state.to_string()))
            .bind(("stage", stage.to_string()))
            .bind(("plan", plan))
            .bind(("err", error.map(str::to_string)))
            .bind(("at", now()))
            .await?,
    )
    .await
}

async fn transition(
    repo: &Repository,
    id: &str,
    from: &[&str],
    to: &str,
    stage: &str,
) -> Result<bool> {
    changed(
        repo.db()
            .query(
                "UPDATE $r SET state = $to, stage = $stage, updated_at = $at \
                 WHERE state INSIDE $from RETURN VALUE meta::id(id)",
            )
            .bind(("r", rid("recon_run", id)))
            .bind((
                "from",
                from.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            ))
            .bind(("to", to.to_string()))
            .bind(("stage", stage.to_string()))
            .bind(("at", now()))
            .await?,
    )
    .await
}

/// `Store::cancel_run` (running runs only).
pub async fn cancel_run(repo: &Repository, id: &str) -> Result<bool> {
    transition(repo, id, &["running"], "cancelled", "cancelled").await
}

/// `Store::restart_run` (interrupted or failed runs only).
pub async fn restart_run(repo: &Repository, id: &str) -> Result<bool> {
    transition(repo, id, &["interrupted", "failed"], "running", "resuming").await
}

const CALL_FIELDS: &str = "meta::id(id) AS id, tool_id, run_id, thread_id, turn_id, origin, \
     inputs_json, status, attempts, result_json, started_at, completed_at";

#[derive(Debug, Clone, SurrealValue)]
struct CallRow {
    id: String,
    tool_id: String,
    run_id: Option<String>,
    thread_id: Option<String>,
    turn_id: Option<String>,
    origin: String,
    inputs_json: String,
    status: String,
    attempts: i64,
    result_json: Option<String>,
    started_at: String,
    completed_at: Option<String>,
}

impl From<CallRow> for Call {
    fn from(c: CallRow) -> Self {
        Call {
            id: c.id,
            tool_id: c.tool_id,
            run_id: c.run_id,
            thread_id: c.thread_id,
            turn_id: c.turn_id,
            origin: c.origin,
            inputs: serde_json::from_str(&c.inputs_json).unwrap_or(Value::Null),
            status: c.status,
            attempts: c.attempts,
            result: c.result_json.and_then(|x| serde_json::from_str(&x).ok()),
            started_at: c.started_at,
            completed_at: c.completed_at,
        }
    }
}

/// Which calls [`calls`] returns.
#[derive(Debug, Clone, Copy)]
pub enum CallFilter<'a> {
    /// `calls_for_run`.
    Run(&'a str),
    /// `calls_for_thread`: completed calls only.
    ThreadCompleted(&'a str),
    /// `all_calls_for_thread`.
    Thread(&'a str),
    /// `manual_calls`.
    Manual,
}

/// Calls in start order (then id), as the SQLite `Store::calls`.
pub async fn calls(repo: &Repository, filter: CallFilter<'_>) -> Result<Vec<Call>> {
    let (cond, key) = match filter {
        CallFilter::Run(k) => ("run_id = $k", k),
        CallFilter::ThreadCompleted(k) => ("thread_id = $k AND status = 'completed'", k),
        CallFilter::Thread(k) => ("thread_id = $k", k),
        CallFilter::Manual => ("origin = 'manual'", ""),
    };
    let mut r = repo
        .db()
        .query(format!(
            "SELECT {CALL_FIELDS} FROM osint_call WHERE {cond} ORDER BY started_at, id"
        ))
        .bind(("k", key.to_string()))
        .await?
        .check()?;
    let rows: Vec<CallRow> = r.take(0usize)?;
    Ok(rows.into_iter().map(Call::from).collect())
}

/// `Store::queue_call`: the tool must exist, the thread be live and the run
/// still running on that thread; checked and written in one transaction.
pub async fn queue_call(
    repo: &Repository,
    tool_id: &str,
    inputs: &Value,
    origin: &str,
    run_id: Option<&str>,
    thread_id: Option<&str>,
    turn_id: Option<&str>,
) -> Result<String> {
    ensure!(osint::definition(tool_id).is_some(), "unknown tool");
    let call_id = crate::recon::id("call");
    let row = (
        call_id.clone(),
        tool_id.to_string(),
        serde_json::to_string(inputs)?,
        origin.to_string(),
        run_id.map(str::to_string),
        thread_id.map(str::to_string),
        turn_id.map(str::to_string),
    );
    run_tx(repo.db(), RetryPolicy::default(), |tx| {
        let (id, tool, inputs, origin, run_id, thread, turn) = row.clone();
        async move {
            run(
                &tx,
                tx.query(
                    "IF $thread != NONE AND (SELECT VALUE deleted FROM ONLY type::record('recon_thread', $thread)) != false {
                       THROW 'thread was deleted';
                     };
                     IF $run != NONE {
                       LET $r = (SELECT state, thread_id FROM ONLY type::record('recon_run', $run));
                       IF $r = NONE OR $r.state != 'running' OR $r.thread_id != $thread {
                         THROW 'run is no longer active';
                       };
                     };
                     CREATE $c SET tool_id = $tool, run_id = $run, thread_id = $thread,
                       turn_id = $turn, origin = $origin, inputs_json = $inputs,
                       status = 'queued', attempts = 0, started_at = $at RETURN NONE;",
                )
                .bind(("c", rid("osint_call", &id)))
                .bind(("tool", tool))
                .bind(("run", run_id))
                .bind(("thread", thread))
                .bind(("turn", turn))
                .bind(("origin", origin))
                .bind(("inputs", inputs))
                .bind(("at", now())),
            )
            .await?;
            Ok(())
        }
    })
    .await
    .map_err(|e| anyhow!("queue call: {e}"))?;
    Ok(call_id)
}

/// `Store::finish_call`: only an open call whose thread (if any) is live.
pub async fn finish_call(repo: &Repository, call_id: &str, result: &ToolResult) -> Result<bool> {
    changed(
        repo.db()
            .query(
                "UPDATE $c SET status = $status, attempts += 1, result_json = $res, completed_at = $at
                  WHERE status INSIDE ['queued', 'running']
                    AND (thread_id = NONE
                         OR type::record('recon_thread', thread_id).deleted = false)
                  RETURN VALUE meta::id(id)",
            )
            .bind(("c", rid("osint_call", call_id)))
            .bind(("status", result.status.clone()))
            .bind(("res", serde_json::to_string(result)?))
            .bind(("at", now()))
            .await?,
    )
    .await
}

/// `Store::attach_call`: file a finished manual call under a live thread.
pub async fn attach_call(repo: &Repository, call_id: &str, thread_id: &str) -> Result<bool> {
    ensure!(
        get_thread(repo, thread_id).await?.is_some(),
        "thread not found"
    );
    changed(
        repo.db()
            .query(
                "UPDATE $c SET thread_id = $t WHERE origin = 'manual' \
                 AND status INSIDE ['completed', 'no_results'] RETURN VALUE meta::id(id)",
            )
            .bind(("c", rid("osint_call", call_id)))
            .bind(("t", thread_id.to_string()))
            .await?,
    )
    .await
}

/// `Store::inflight_duplicate`: a queued or running call of the tool (or
/// its canonical id) with the same inputs.
pub async fn inflight_duplicate(repo: &Repository, tool_id: &str, inputs: &Value) -> Result<bool> {
    let mut r = repo
        .db()
        .query(
            "RETURN count(SELECT id FROM osint_call WHERE tool_id INSIDE $tools \
             AND inputs_json = $inputs AND status INSIDE ['queued', 'running'] LIMIT 1)",
        )
        .bind((
            "tools",
            vec![
                tool_id.to_string(),
                osint::canonical_tool_id(tool_id).to_string(),
            ],
        ))
        .bind(("inputs", serde_json::to_string(inputs)?))
        .await?
        .check()?;
    let n: Option<i64> = r.take(0usize)?;
    Ok(n.unwrap_or(0) > 0)
}

/// Link an answer to the calls it cites (each call must exist).
pub async fn link_evidence(repo: &Repository, message_id: &str, call_ids: &[String]) -> Result<()> {
    let (m, ids) = (message_id.to_string(), call_ids.to_vec());
    run_tx(repo.db(), RetryPolicy::default(), |tx| {
        let (m, ids) = (m.clone(), ids.clone());
        async move { link_evidence_in(&tx, &m, &ids).await }
    })
    .await
    .map_err(|e| anyhow!("link evidence: {e}"))?;
    Ok(())
}

/// [`link_evidence`] inside an open transaction.
pub(crate) async fn link_evidence_in(
    tx: &super::tx::Tx,
    message_id: &str,
    call_ids: &[String],
) -> Result<(), super::tx::TxError> {
    run(
        tx,
        tx.query(
            "FOR $c IN $ids {
               IF !record::exists(type::record('osint_call', $c)) { THROW 'evidence call missing'; };
               UPSERT type::record('recon_message_evidence', [$m, $c])
                 SET message_id = $m, call_id = $c RETURN NONE;
             };",
        )
        .bind(("m", message_id.to_string()))
        .bind(("ids", call_ids.to_vec())),
    )
    .await?;
    Ok(())
}

#[derive(Debug, Clone, SurrealValue)]
struct EvidenceRow {
    id: String,
    result_json: Option<String>,
}

/// `Store::answer_evidence`: the cited calls with a readable result, in
/// start order.
pub async fn answer_evidence(
    repo: &Repository,
    answer_id: &str,
) -> Result<Vec<(String, ToolResult)>> {
    let mut r = repo
        .db()
        .query(
            "LET $ids = (SELECT VALUE call_id FROM recon_message_evidence WHERE message_id = $m);
             SELECT meta::id(id) AS id, result_json, started_at FROM osint_call
              WHERE meta::id(id) INSIDE $ids ORDER BY started_at;",
        )
        .bind(("m", answer_id.to_string()))
        .await?
        .check()?;
    let rows: Vec<EvidenceRow> = r.take(1usize)?;
    Ok(rows
        .into_iter()
        .filter_map(|e| {
            let res = serde_json::from_str(e.result_json.as_deref()?).ok()?;
            Some((e.id, res))
        })
        .collect())
}

/// `Store::cache_put`: replace the entry, expiring `ttl` seconds from now.
pub async fn cache_put(repo: &Repository, key: &str, result: &ToolResult, ttl: u64) -> Result<()> {
    let expires = (Utc::now() + chrono::Duration::seconds(ttl as i64)).to_rfc3339();
    repo.db()
        .query("UPSERT $k SET result_json = $res, expires_at = $exp RETURN NONE")
        .bind(("k", rid("osint_cache", key)))
        .bind(("res", serde_json::to_string(result)?))
        .bind(("exp", expires))
        .await?
        .check()?;
    Ok(())
}

/// `Store::cache_get_fresh`: an unexpired entry, and (with a cutoff) one
/// retrieved no earlier than the cutoff. `None` cutoff is `cache_get`.
pub async fn cache_get_fresh(
    repo: &Repository,
    key: &str,
    freshness_cutoff: Option<&str>,
) -> Result<Option<ToolResult>> {
    let mut r = repo
        .db()
        .query("SELECT VALUE result_json FROM ONLY $k WHERE expires_at > $now")
        .bind(("k", rid("osint_cache", key)))
        .bind(("now", now()))
        .await?
        .check()?;
    let raw: Option<String> = r.take(0usize)?;
    let Some(res): Option<ToolResult> = raw.and_then(|r| serde_json::from_str(&r).ok()) else {
        return Ok(None);
    };
    if let Some(cutoff) = freshness_cutoff {
        if !cutoff.is_empty() && !res.retrieved_at.is_empty() && res.retrieved_at.as_str() < cutoff
        {
            return Ok(None);
        }
    }
    Ok(Some(res))
}

/// A run's still-open calls (`queued`/`running`) settled as `status`
/// (`cancelled` or `interrupted`) when the run ends early. Returns how many.
pub async fn settle_open_calls(repo: &Repository, run_id: &str, status: &str) -> Result<usize> {
    ensure!(
        matches!(status, "cancelled" | "interrupted"),
        "unexpected call settlement {status}"
    );
    let mut r = repo
        .db()
        .query(
            "UPDATE osint_call SET status = $s, completed_at = $at
              WHERE run_id = $rid AND status INSIDE ['queued', 'running']
              RETURN VALUE meta::id(id)",
        )
        .bind(("s", status.to_string()))
        .bind(("at", now()))
        .bind(("rid", run_id.to_string()))
        .await?
        .check()?;
    let ids: Vec<String> = r.take(0usize)?;
    Ok(ids.len())
}
