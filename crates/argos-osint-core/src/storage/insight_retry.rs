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
