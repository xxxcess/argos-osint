//! Insight owner state on SurrealDB (Recon switch, step 2): the
//! repository-level counterparts of `insight_persist::desired_generation`,
//! `advance_generation`, `enqueue_refresh`, `active_output` and
//! `reconcile_deleted_memories`, plus the live Recon opt-in that
//! `run_refresh_slice` needs for a Recon owner.
//!
//! Each runs in one transaction, as the SQLite versions run in one
//! statement (or one batch). The `_in` forms join a caller's transaction;
//! `reconcile_deleted_memories_in` is meant to run inside a deletion.
//!
//! Not ported: `enqueue_refresh`'s requeue of a paused task
//! (`blocked_reason='cancelled; evidence dirty'`), because SurrealDB tasks
//! have no `paused` state.

use anyhow::{anyhow, Result};
use surrealdb::types::SurrealValue;

use super::derivations::DerivationContext;
use super::jobs::{rid, run, take};
use super::repo::Repository;
use super::tx::{run_tx, RetryPolicy, Tx, TxError};
use crate::investigation::insights::{AssessedLink, PipelineInput, PipelineOutput, TypedEndpoint};
use crate::provider::SettingsFile;

/// `insight_persist::desired_generation`: 0 when the owner has no state.
pub async fn desired_generation(repo: &Repository, kind: &str, owner: &str) -> Result<u32> {
    let (kind, owner) = (kind.to_string(), owner.to_string());
    let g = run_tx(repo.db(), RetryPolicy::default(), |tx| {
        let (kind, owner) = (kind.clone(), owner.clone());
        async move { super::insight::desired_generation(&tx, &kind, &owner).await }
    })
    .await?
    .value;
    Ok(u32::try_from(g)?)
}

/// `insight_persist::advance_generation`: the owner's next generation, with
/// the active snapshot kept and the lifecycle set to `pending`.
pub async fn advance_generation(
    repo: &Repository,
    kind: &str,
    owner: &str,
    now: &str,
) -> Result<u32> {
    let (kind, owner, now) = (kind.to_string(), owner.to_string(), now.to_string());
    let g = run_tx(repo.db(), RetryPolicy::default(), |tx| {
        let (kind, owner, now) = (kind.clone(), owner.clone(), now.clone());
        async move { super::insight::advance_generation(&tx, &kind, &owner, &now).await }
    })
    .await?
    .value;
    Ok(u32::try_from(g)?)
}

/// `insight_persist::enqueue_refresh` in its own transaction: the refresh
/// job, task and sidecar, and the owner's `refresh_job_id` for this
/// generation. Sets `input.snapshot.key`; returns the task id.
pub async fn enqueue_refresh(
    repo: &Repository,
    input: &mut PipelineInput,
    settings: &SettingsFile,
) -> Result<String> {
    let now = chrono::Utc::now();
    let committed = run_tx(repo.db(), RetryPolicy::default(), |tx| {
        let mut attempt = input.clone();
        async move {
            let id = super::refresh::enqueue_refresh_in(&tx, &mut attempt, settings, now).await?;
            Ok((id, attempt))
        }
    })
    .await?
    .value;
    *input = committed.1;
    Ok(committed.0)
}

#[derive(SurrealValue)]
struct OptIn {
    thread: Option<String>,
    opted: bool,
}

/// The live Recon opt-in for a Recon owner, read in `tx`: saving needs the
/// input's `save_insights`, a live (not cancelled) run on an undeleted
/// thread, and the thread's `recall_insights` (SQLite's recheck in
/// `write_derivations_with_kind`). Carries the run's thread either way.
pub async fn recon_context_in(
    tx: &Tx,
    run_id: &str,
    save_insights: bool,
) -> Result<DerivationContext, TxError> {
    let mut r = run(
        tx,
        tx.query(
            "LET $run = (SELECT thread_id, state FROM ONLY $r);
             LET $thread = IF $run = NONE THEN NONE ELSE
               (SELECT deleted, recall_insights FROM ONLY type::record('recon_thread', $run.thread_id)) END;
             RETURN { thread: $run.thread_id, opted: $thread != NONE AND $thread.deleted = false
               AND $thread.recall_insights = true };",
        )
        .bind(("r", rid("recon_run", run_id))),
    )
    .await?;
    let last = r.num_statements() - 1;
    let row: Option<OptIn> = take(&mut r, last)?;
    let (thread, opted) = row.map(|o| (o.thread, o.opted)).unwrap_or((None, false));
    Ok(DerivationContext {
        save_insights: save_insights && opted,
        recon_thread: thread,
    })
}

/// [`recon_context_in`] in its own transaction.
pub async fn recon_context(
    repo: &Repository,
    run_id: &str,
    save_insights: bool,
) -> Result<DerivationContext> {
    let run_id = run_id.to_string();
    Ok(run_tx(repo.db(), RetryPolicy::default(), |tx| {
        let run_id = run_id.clone();
        async move { recon_context_in(&tx, &run_id, save_insights).await }
    })
    .await?
    .value)
}

#[derive(SurrealValue)]
struct Head {
    active_snapshot_id: String,
    lifecycle: String,
}

#[derive(SurrealValue)]
struct SnapshotRow {
    entities_json: String,
    readiness_json: String,
    input_key: String,
}

#[derive(SurrealValue)]
struct DerivationRow {
    insight_kind: String,
    item_json: String,
}

#[derive(SurrealValue)]
struct LinkRow {
    key: String,
    left_kind: String,
    left_id: String,
    right_kind: String,
    right_id: String,
    relation: String,
    rationale: String,
    assessment_json: String,
    evidence_json: String,
}

fn json<T: serde::de::DeserializeOwned>(s: &str) -> Result<T> {
    Ok(serde_json::from_str(s)?)
}

/// `insight_persist::active_output`: the owner's active snapshot as a
/// pipeline output (live derivations, assessed links), read in one
/// transaction. `None` without an active snapshot. A non-`active`
/// lifecycle makes the output ineligible and adds a readiness gap.
pub async fn active_output(
    repo: &Repository,
    kind: &str,
    owner: &str,
) -> Result<Option<PipelineOutput>> {
    let (kind, owner) = (kind.to_string(), owner.to_string());
    let rows = run_tx(repo.db(), RetryPolicy::default(), |tx| {
        let (kind, owner) = (kind.clone(), owner.clone());
        async move {
            let mut r = run(
                &tx,
                tx.query(
                    "SELECT active_snapshot_id, lifecycle FROM ONLY
                       type::record('insight_owner_state', [$k, $o]);",
                )
                .bind(("k", kind))
                .bind(("o", owner)),
            )
            .await?;
            let Some(head) =
                take::<Option<Head>>(&mut r, 0)?.filter(|h| !h.active_snapshot_id.is_empty())
            else {
                return Ok(None);
            };
            let mut r = run(
                &tx,
                tx.query(
                    "SELECT entities_json, readiness_json, input_key FROM ONLY
                       type::record('insight_snapshot', $s);
                     SELECT insight_kind, item_json, id FROM insight_derivation
                       WHERE snapshot_id = $s AND status != 'deleted' ORDER BY id;
                     SELECT record::id(id) AS key, left_kind, left_id, right_kind, right_id,
                       relation, rationale, assessment_json, evidence_json, id
                       FROM insight_derivation_link
                       WHERE snapshot_id = $s AND status = 'assessed' ORDER BY id;",
                )
                .bind(("s", head.active_snapshot_id.clone())),
            )
            .await?;
            let snapshot: Option<SnapshotRow> = take(&mut r, 0)?;
            let derivations: Vec<DerivationRow> = take(&mut r, 1)?;
            let links: Vec<LinkRow> = take(&mut r, 2)?;
            Ok(Some((head, snapshot, derivations, links)))
        }
    })
    .await?
    .value;
    let Some((head, snapshot, derivations, links)) = rows else {
        return Ok(None);
    };
    let snapshot = snapshot
        .ok_or_else(|| anyhow!("active snapshot {} is missing", head.active_snapshot_id))?;
    let mut output = PipelineOutput {
        entities: json(&snapshot.entities_json)?,
        ..Default::default()
    };
    let stored: serde_json::Value = json(&snapshot.readiness_json)?;
    output.limit_reached = stored["limit_reached"].as_bool().unwrap_or(false);
    output.partial_enumeration = stored["partial_enumeration"].as_bool().unwrap_or(false);
    output.item_gaps = serde_json::from_value(stored["item_gaps"].clone()).unwrap_or_default();
    output.withheld_count = stored["withheld_count"].as_u64().unwrap_or(0) as usize;
    if let Some(value) = stored.get("readiness") {
        output.readiness = serde_json::from_value(value.clone())?;
    }
    let active = head.lifecycle == "active";
    output.readiness.input_eligible = active;
    output.readiness.publication_eligible = active;
    if !active {
        output
            .readiness
            .gaps
            .push(format!("insight lifecycle: {}", head.lifecycle));
    }
    for row in derivations {
        if row.insight_kind == "claim" {
            output.claims.push(json(&row.item_json)?);
        } else if row.insight_kind == "inference" {
            output.inferences.push(json(&row.item_json)?);
        }
    }
    for l in links {
        output.links.push(AssessedLink {
            id: l.key,
            typed_endpoints: vec![
                TypedEndpoint {
                    kind: l.left_kind,
                    id: l.left_id,
                },
                TypedEndpoint {
                    kind: l.right_kind,
                    id: l.right_id,
                },
            ],
            relation: l.relation,
            rationale: l.rationale,
            assessment: json(&l.assessment_json)?,
            evidence_refs: json(&l.evidence_json)?,
            snapshot_key: snapshot.input_key.clone(),
        });
    }
    Ok(Some(output))
}

/// `insight_persist::reconcile_deleted_memories` inside a deletion
/// transaction: derivations whose memory is gone become `deleted` (memory
/// detached), links touching a deleted derivation's item become
/// `deleted_endpoint`, owners whose active snapshot holds a deleted
/// derivation drop to `previous`, and evidence loses dangling memory ids.
/// Historical evidence rows are kept.
pub async fn reconcile_deleted_memories_in(tx: &Tx) -> Result<(), TxError> {
    let mut r = run(
        tx,
        tx.query(
            "UPDATE insight_derivation SET status = 'deleted'
               WHERE memory_id != NONE AND !record::exists(type::record('memory', memory_id)) RETURN NONE;
             SELECT VALUE item_json FROM insight_derivation WHERE status = 'deleted';",
        ),
    )
    .await?;
    let items: Vec<String> = take::<Vec<String>>(&mut r, 1)?
        .iter()
        .filter_map(|j| serde_json::from_str::<serde_json::Value>(j).ok())
        .filter_map(|v| v.get("id").and_then(|id| id.as_str()).map(str::to_string))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    run(
        tx,
        tx.query(
            "-- One equality per item keeps each update on its index.
             FOR $item IN $items {
               UPDATE insight_derivation_link SET status = 'deleted_endpoint' WHERE left_id = $item RETURN NONE;
               UPDATE insight_derivation_link SET status = 'deleted_endpoint' WHERE right_id = $item RETURN NONE;
             };
             LET $snaps = array::distinct((SELECT VALUE snapshot_id FROM insight_derivation WHERE status = 'deleted'));
             FOR $s IN $snaps {
               UPDATE insight_owner_state SET lifecycle = 'previous', last_error = 'memory endpoint removed'
                 WHERE active_snapshot_id = $s RETURN NONE;
             };
             UPDATE insight_derivation SET memory_id = NONE WHERE status = 'deleted' RETURN NONE;
             UPDATE insight_derivation_evidence SET memory_id = NONE
               WHERE memory_id != NONE AND !record::exists(type::record('memory', memory_id)) RETURN NONE;",
        )
        .bind(("items", items)),
    )
    .await?;
    Ok(())
}

/// [`reconcile_deleted_memories_in`] in its own transaction.
pub async fn reconcile_deleted_memories(repo: &Repository) -> Result<()> {
    run_tx(repo.db(), RetryPolicy::default(), |tx| async move {
        reconcile_deleted_memories_in(&tx).await
    })
    .await?;
    Ok(())
}
