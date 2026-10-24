//! Retry admission for insight refreshes on SurrealDB: the counterpart of
//! `insight_persist::validate_retry` (live evidence plus the owner's desired
//! generation) and of the `exhausted` checkpoint check `retry_extraction`
//! and `owner_progress` use, so a SurrealDB retry never reopens SQLite.
//!
//! No new tables: reads `insight_owner_state` (through
//! `insight::desired_generation`), the live-evidence tables (through
//! `live_evidence::validate_in`) and `insight_stage_checkpoint`.

use anyhow::Result;
use surrealdb::engine::any::Any;
use surrealdb::types::SurrealValue;
use surrealdb::Surreal;

use super::jobs::{run, take};
use super::tx::{run_tx, RetryPolicy, Tx, TxError};
use crate::investigation::insights::{EvidenceSnapshot, PipelineInput};

/// `validate_retry` inside `tx`: every evidence source is still live and
/// the snapshot's generation is still the owner's desired one.
pub async fn validate_retry_in(tx: &Tx, snapshot: &EvidenceSnapshot) -> Result<(), TxError> {
    super::live_evidence::validate_in(tx, snapshot).await?;
    let desired =
        super::insight::desired_generation(tx, &snapshot.owner_kind, &snapshot.owner_id).await?;
    if desired != i64::from(snapshot.generation) {
        return Err(TxError::Abort("refresh superseded; cannot retry".into()));
    }
    Ok(())
}

/// [`validate_retry_in`] in its own read transaction.
pub async fn validate_retry(db: &Surreal<Any>, input: &PipelineInput) -> Result<()> {
    let snapshot = input.snapshot.clone();
    run_tx(db, RetryPolicy::default(), |tx| {
        let snapshot = snapshot.clone();
        async move { validate_retry_in(&tx, &snapshot).await }
    })
    .await
    .map_err(|e| match e {
        TxError::Abort(message) => anyhow::anyhow!(message),
        other => anyhow::anyhow!("insight retry validation: {other}"),
    })?;
    Ok(())
}

/// Whether the owner generation has an `exhausted` stage checkpoint, the
/// only state an explicit retry admits.
pub async fn has_exhausted_stage(
    db: &Surreal<Any>,
    kind: &str,
    owner: &str,
    generation: u32,
) -> Result<bool> {
    let (kind, owner) = (kind.to_string(), owner.to_string());
    let committed = run_tx(db, RetryPolicy::default(), |tx| {
        let (kind, owner) = (kind.clone(), owner.clone());
        async move {
            let mut r = run(
                &tx,
                tx.query(
                    "RETURN count(SELECT VALUE id FROM insight_stage_checkpoint
                       WHERE owner_kind = $k AND owner_id = $o AND generation = $g
                         AND state = 'exhausted' LIMIT 1);",
                )
                .bind(("k", kind))
                .bind(("o", owner))
                .bind(("g", i64::from(generation))),
            )
            .await?;
            Ok::<bool, TxError>(take::<Option<i64>>(&mut r, 0)?.unwrap_or(0) > 0)
        }
    })
    .await
    .map_err(|e| anyhow::anyhow!("insight retry checkpoint: {e}"))?;
    Ok(committed.value)
}

/// `owner_progress`'s `retryable`: a failed/completed refresh with an
/// exhausted stage whose retry would still be admitted.
pub async fn retryable(db: &Surreal<Any>, state: &str, input: &PipelineInput) -> Result<bool> {
    if !matches!(state, "failed" | "completed") {
        return Ok(false);
    }
    let s = &input.snapshot;
    if !has_exhausted_stage(db, &s.owner_kind, &s.owner_id, s.generation).await? {
        return Ok(false);
    }
    Ok(validate_retry(db, input).await.is_ok())
}

/// `insight_execution::retry_extraction` on SurrealDB: an explicit retry of
/// one failed or completed `insight_refresh` task whose owner generation has
/// an `exhausted` stage. In one transaction it re-checks the admission
/// (`validate_retry_in`), requeues the task with one more attempt, marks the
/// exhausted/cancelled checkpoints `retry_admitted` (completed ones are
/// kept), bumps the execution's `retry_epoch`, sets the owner back to
/// `pending`, reopens the job and records an `insight.retry` event. Returns
/// false when there is nothing to retry (no such task, a cancel request, no
/// exhausted stage, or another writer changed the task first).
pub async fn retry_extraction(db: &Surreal<Any>, task_id: &str) -> Result<bool> {
    let task_id = task_id.to_string();
    let committed = run_tx(db, RetryPolicy::default(), |tx| {
        let task_id = task_id.clone();
        async move { retry_extraction_in(&tx, &task_id).await }
    })
    .await
    .map_err(|e| match e {
        TxError::Abort(message) => anyhow::anyhow!(message),
        other => anyhow::anyhow!("insight retry: {other}"),
    })?;
    Ok(committed.value)
}

#[derive(Debug, SurrealValue)]
struct RetryHead {
    state: String,
    job: String,
    cancel_requested: Option<bool>,
    input_ref: Option<String>,
    role_snapshot: Option<String>,
}

async fn retry_extraction_in(tx: &Tx, task_id: &str) -> Result<bool, TxError> {
    let mut r = run(
        tx,
        tx.query(
            "SELECT state, record::id(job) AS job, job.cancel_requested AS cancel_requested,
               type::record('insight_refresh_task', $t).input_ref AS input_ref,
               type::record('insight_refresh_task', $t).role_snapshot AS role_snapshot
             FROM ONLY type::record('task', $t) WHERE operation = 'insight_refresh';",
        )
        .bind(("t", task_id.to_string())),
    )
    .await?;
    let Some(head): Option<RetryHead> = take(&mut r, 0)? else {
        return Ok(false);
    };
    let (Some(input_json), Some(captured_json)) = (head.input_ref, head.role_snapshot) else {
        return Ok(false);
    };
    if head.cancel_requested == Some(true) || !matches!(head.state.as_str(), "failed" | "completed")
    {
        return Ok(false);
    }
    let input: PipelineInput = serde_json::from_str(&input_json)
        .map_err(|e| TxError::Abort(format!("insight retry input: {e}")))?;
    validate_retry_in(tx, &input.snapshot).await?;
    let s = &input.snapshot;
    let mut r = run(
        tx,
        tx.query(
            "RETURN count(SELECT VALUE id FROM insight_stage_checkpoint
               WHERE owner_kind = $k AND owner_id = $o AND generation = $g
                 AND state = 'exhausted' LIMIT 1);",
        )
        .bind(("k", s.owner_kind.clone()))
        .bind(("o", s.owner_id.clone()))
        .bind(("g", i64::from(s.generation))),
    )
    .await?;
    if take::<Option<i64>>(&mut r, 0)?.unwrap_or(0) == 0 {
        return Ok(false);
    }
    let captured: crate::provider::SettingsFile = serde_json::from_str(&captured_json)
        .map_err(|e| TxError::Abort(format!("insight retry settings: {e}")))?;
    let live_key = crate::investigation::insights::input_key(&input, &captured);
    let now = chrono::Utc::now();
    let mut r = run(
        tx,
        tx.query(
            "UPDATE type::record('task', $t) SET state = 'queued', max_attempts = attempts + 1,
               next_eligible_at = <datetime>$now, lease_owner = '', process_owner = '', lease_until = NONE,
               error_category = '', error_message = '', finished_at = NONE, updated_at = <datetime>$now
             WHERE state = $state RETURN VALUE record::id(id);",
        )
        .bind(("t", task_id.to_string()))
        .bind(("state", head.state.clone()))
        .bind(("now", now.to_rfc3339())),
    )
    .await?;
    if take::<Vec<String>>(&mut r, 0)?.is_empty() {
        return Ok(false);
    }
    let stamp = now.to_rfc3339();
    run(
        tx,
        tx.query(
            "UPDATE insight_stage_checkpoint SET state = 'retry_admitted', updated_at = $stamp
               WHERE owner_kind = $k AND owner_id = $o AND generation = $g
                 AND state IN ['exhausted', 'cancelled'] RETURN NONE;
             UPSERT type::record('insight_refresh_execution', $t)
               SET retry_epoch = (retry_epoch ?? 0) + 1, stage = 'queued', updated_at = $stamp
               RETURN NONE;
             UPDATE type::record('insight_refresh_task', $t) SET blocked_reason = '' RETURN NONE;
             UPDATE insight_owner_state SET lifecycle = 'pending', last_error = ''
               WHERE id = type::record('insight_owner_state', [$k, $o])
                 AND desired_generation = $g RETURN NONE;
             UPDATE type::record('job', $j) SET state = 'running', finished_at = NONE,
               updated_at = <datetime>$now WHERE state IN ['failed', 'partial', 'completed'] RETURN NONE;
             CREATE job_event SET job_id = $j, app = $k, event_type = 'insight.retry',
               severity = 'info', message = $msg, ts = <datetime>$now RETURN NONE;",
        )
        .bind(("t", task_id.to_string()))
        .bind(("j", head.job.clone()))
        .bind(("k", s.owner_kind.clone()))
        .bind(("o", s.owner_id.clone()))
        .bind(("g", i64::from(s.generation)))
        .bind(("stamp", stamp))
        .bind(("now", now.to_rfc3339()))
        .bind((
            "msg",
            format!("Retry extraction admitted for {live_key}; completed checkpoints retained"),
        )),
    )
    .await?;
    Ok(true)
}
