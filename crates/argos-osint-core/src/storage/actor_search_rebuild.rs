//! Durable shadow scan/catch-up commands. Included by actor_search_outbox only
//! after its integration checkpoint; there is no HTTP inside a transaction.

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use chrono::DateTime;
use chrono::Duration;
use chrono::Utc;
use fleetworks_contracts::actor_index::ActorIndexBatch;
use fleetworks_contracts::actor_index::ActorIndexGeneration;
use fleetworks_contracts::actor_index::ActorIndexMutation;
use fleetworks_contracts::actor_index::ActorIndexPromote;
use fleetworks_contracts::actor_index::ActorIndexRebuildStart;
use fleetworks_contracts::actor_index::ActorIndexReceipt;
use fleetworks_contracts::actor_index::ActorIndexRepair;
use fleetworks_contracts::actor_index::ActorIndexStatus;
use fleetworks_contracts::actor_index::ACTOR_INDEX_PROTOCOL;
use fleetworks_contracts::readiness::MaintenanceOperation;
use serde::Deserialize;
use serde::Serialize;
use surrealdb::types::SurrealValue;

use super::canonical_many_in;
use super::control_in;
use super::guard;
use super::hash;
use super::instant;
use super::invalid;
use super::jobs;
use super::root_guard_in;
use super::run_tx;
use super::Control;
use super::Event;
use super::Lease;
use super::Repository;
use super::RetryPolicy;
use super::Tx;
use super::TxError;

use crate::storage::jobs::ClaimedTask;
use crate::storage::setup_jobs;

#[derive(Clone, Debug, SurrealValue)]
pub struct RebuildProgress {
    pub job_id: String,
    pub original_job_id: String,
    pub request_id: String,
    pub phase: String,
    pub expected_generation: String,
    pub replacement_generation: String,
    pub base_checkpoint: i64,
    pub visible_checkpoint: i64,
    pub scan_after: String,
    pub scanned: i64,
    pub scan_complete: bool,
    pub scan_streak: i64,
    pub barrier: i64,
    pub pending_json: String,
    pub reason_code: String,
    pub repair_attempts: i64,
}

async fn row(tx: &Tx) -> Result<Option<RebuildProgress>, TxError> {
    let mut response=jobs::run(tx,tx.query("SELECT job_id,original_job_id,request_id,phase,expected_generation,replacement_generation,base_checkpoint,visible_checkpoint,scan_after,scanned,scan_complete,scan_streak,barrier,pending_json,reason_code,repair_attempts FROM ONLY actor_search_rebuild:catalog")).await?;
    jobs::take(&mut response, 0)
}

pub async fn observe_rebuild(
    repo: &Repository,
    job_id: &str,
) -> Result<Option<RebuildProgress>, TxError> {
    let mut response=repo.db().query("SELECT job_id,original_job_id,request_id,phase,expected_generation,replacement_generation,base_checkpoint,visible_checkpoint,scan_after,scanned,scan_complete,scan_streak,barrier,pending_json,reason_code,repair_attempts FROM ONLY actor_search_rebuild:catalog WHERE job_id=$job")
        .bind(("job",job_id.to_owned())).await?.check()?;
    Ok(response.take(0)?)
}

/// Called only by the actual claimed ReindexActors consumer. Durable acceptance
/// already captured provenance; this operation does not invent an origin.
pub async fn request_rebuild(repo: &Repository, claimed: &ClaimedTask) -> Result<(), TxError> {
    let payload = setup_jobs::payload(repo, &claimed.job_id, &claimed.input_hash).await?;
    if payload.operation != MaintenanceOperation::ReindexActors
        || payload.dataset.is_some()
        || claimed.pool != setup_jobs::REINDEX_POOL
    {
        return Err(invalid("actor_reindex_claim_contract"));
    }
    run_tx(repo.db(),RetryPolicy::default(),|tx|async move {
        guard(&tx).await?;root_guard_in(&tx,&claimed.job_id).await?;
        let now=Utc::now();
        let fence=jobs::fence(&tx,claimed,&now.to_rfc3339()).await?;
        if !fence.is_some_and(|fence|!fence.cancelled&&jobs::holds(&fence,claimed)) {return Err(invalid("actor_reindex_job_lease_lost"));}
        if let Some(previous)=row(&tx).await? {
            if previous.job_id==claimed.job_id {return Ok(());}
            let mut state=jobs::run(&tx,tx.query("RETURN (SELECT VALUE state FROM ONLY $job) IN ['queued','running','reconciling']").bind(("job",jobs::rid("job",&previous.job_id)))).await?;
            if previous.phase!="completed" && jobs::take::<Option<bool>>(&mut state,0)?==Some(true) {return Err(invalid("actor_reindex_resource_busy"));}
            if previous.phase!="completed" {
                // An explicit new admitted job may resume preserved work. Its
                // result retains the original job; creator metadata is never rewritten.
                jobs::run(&tx,tx.query("UPDATE actor_search_rebuild:catalog SET job_id=$job,reason_code='' RETURN NONE; UPDATE actor_search_control:catalog SET blocked=false,failures=0,retry_at=<datetime>$now RETURN NONE")
                    .bind(("job",claimed.job_id.clone())).bind(("now",now.to_rfc3339()))).await?;
                return Ok(());
            }
        }
        jobs::run(&tx,tx.query("UPSERT actor_search_rebuild:catalog SET job_id=$job,original_job_id=$job,request_id=$job,phase='requested',expected_generation='',replacement_generation='',base_checkpoint=0,visible_checkpoint=0,scan_after='',scanned=0,scan_complete=false,scan_streak=0,barrier=0,pending_json='',reason_code='',repair_attempts=0 RETURN NONE;
            UPDATE actor_search_control:catalog SET blocked=false,failures=0,retry_at=<datetime>$now RETURN NONE")
            .bind(("job",claimed.job_id.clone())).bind(("now",now.to_rfc3339()))).await?;
        Ok(())
    }).await?;
    Ok(())
}

#[derive(Clone, Debug)]
pub enum RebuildAction {
    Waiting,
    Start {
        lease: Lease,
        request: ActorIndexRebuildStart,
    },
    Repair {
        lease: Lease,
        request: ActorIndexRepair,
    },
    Deliver {
        lease: Lease,
        batch: ActorIndexBatch,
    },
    Promote {
        lease: Lease,
        request: ActorIndexPromote,
    },
}

impl RebuildAction {
    pub fn lease(&self) -> Option<&Lease> {
        match self {
            Self::Waiting => None,
            Self::Start { lease, .. }
            | Self::Repair { lease, .. }
            | Self::Deliver { lease, .. }
            | Self::Promote { lease, .. } => Some(lease),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct SavedBatch {
    batch: ActorIndexBatch,
    scan_after: Option<String>,
    scan_complete: bool,
    scanned: u64,
}

async fn bind_source(
    tx: &Tx,
    control: &mut Control,
    status: &ActorIndexStatus,
) -> Result<(), TxError> {
    let mut response = jobs::run(
        tx,
        tx.query("SELECT VALUE instance_id FROM ONLY storage_meta:identity"),
    )
    .await?;
    if jobs::take::<Option<String>>(&mut response, 0)?.as_deref()
        != Some(status.deployment_id.as_str())
    {
        return Err(invalid("actor_index_deployment_mismatch"));
    }
    if control.deployment_id.is_empty() {
        jobs::run(tx,tx.query("UPDATE actor_search_control:catalog SET deployment_id=$deployment,source_epoch=$source,index_generation=$generation RETURN NONE")
            .bind(("deployment",status.deployment_id.clone())).bind(("source",status.source_epoch.clone())).bind(("generation",status.active.as_ref().map(|active|active.generation.clone()).unwrap_or_default()))).await?;
        control.deployment_id = status.deployment_id.clone();
        control.source_epoch = status.source_epoch.clone();
        control.index_generation = status
            .active
            .as_ref()
            .map(|active| active.generation.clone())
            .unwrap_or_default();
    }
    if control.deployment_id != status.deployment_id || control.source_epoch != status.source_epoch
    {
        return Err(invalid("actor_index_source_identity_mismatch"));
    }
    Ok(())
}

async fn reserve(
    tx: &Tx,
    control: &Control,
    owner: &str,
    status: &ActorIndexStatus,
    now: DateTime<Utc>,
) -> Result<Lease, TxError> {
    let remote = status
        .active
        .iter()
        .chain(status.rebuilding.iter())
        .map(|generation| generation.writer_epoch)
        .max()
        .unwrap_or_default();
    let remote = i64::try_from(remote).map_err(|_| invalid("actor_index_epoch_invalid"))?;
    let epoch = if control.lease_owner == owner
        && instant(&control.lease_until)? > now
        && control.lease_epoch >= remote
    {
        control.lease_epoch
    } else {
        control
            .lease_epoch
            .max(remote)
            .checked_add(1)
            .ok_or_else(|| invalid("actor_index_epoch_exhausted"))?
    };
    jobs::run(tx,tx.query("UPDATE actor_search_control:catalog SET lease_owner=$owner,lease_epoch=$epoch,lease_until=<datetime>$until,next_start_at=<datetime>$next,effective_limits_json=$limits RETURN NONE")
        .bind(("owner",owner.to_owned())).bind(("epoch",epoch)).bind(("until",(now+Duration::seconds(45)).to_rfc3339()))
        .bind(("next",(now+Duration::milliseconds(i64::from(status.limits.min_start_ms))).to_rfc3339())).bind(("limits",serde_json::to_string(&status.limits).map_err(|_|invalid("actor_index_limits_encoding"))?))).await?;
    Ok(Lease {
        owner: owner.to_owned(),
        epoch: epoch as u64,
    })
}

fn request(
    rebuild: &RebuildProgress,
    status: &ActorIndexStatus,
    lease: &Lease,
) -> ActorIndexRebuildStart {
    ActorIndexRebuildStart {
        deployment_id: status.deployment_id.clone(),
        source_epoch: status.source_epoch.clone(),
        request_id: rebuild.request_id.clone(),
        expected_generation: rebuild.expected_generation.clone(),
        writer_epoch: lease.epoch,
        source_checkpoint: rebuild.base_checkpoint as u64,
    }
}

/// Returns None only when ordinary live delivery should run first. One shared
/// DB fence/rate owns all live, shadow, repair and promote starts.
pub async fn next_rebuild(
    repo: &Repository,
    owner: &str,
    status: &ActorIndexStatus,
    now: DateTime<Utc>,
) -> Result<Option<RebuildAction>, TxError> {
    status
        .limits
        .validate()
        .map_err(|_| invalid("actor_index_limits_invalid"))?;
    if status.protocol != ACTOR_INDEX_PROTOCOL {
        return Err(invalid("actor_index_protocol_mismatch"));
    }
    let mut existing = repo
        .db()
        .query("SELECT VALUE phase FROM ONLY actor_search_rebuild:catalog")
        .await?
        .check()?;
    let current_phase: Option<String> = existing.take(0)?;
    if current_phase.is_none() || current_phase.as_deref() == Some("completed") {
        return Ok(None);
    }
    let owner = owner.to_owned();
    let status = status.clone();
    Ok(run_tx(repo.db(),RetryPolicy::default(),|tx| {let owner=owner.clone();let status=status.clone();async move {
        guard(&tx).await?;let Some(mut rebuild)=row(&tx).await? else {return Ok(None);};
        if rebuild.phase=="completed" {return Ok(None);}
        let mut control=control_in(&tx).await?;bind_source(&tx,&mut control,&status).await?;
        if (instant(&control.lease_until)?>now&&control.lease_owner!=owner) || control.blocked || instant(&control.retry_at)?>now || instant(&control.next_start_at)?>now {return Ok(Some(RebuildAction::Waiting));}
        root_guard_in(&tx,&rebuild.job_id).await?;
        let mut active=jobs::run(&tx,tx.query("RETURN (SELECT VALUE (state IN ['queued','running','reconciling'] AND !cancel_requested AND !stopped AND !array::any(ancestors.cancel_requested)) FROM ONLY $job) ?? false").bind(("job",jobs::rid("job",&rebuild.job_id)))).await?;
        if jobs::take::<Option<bool>>(&mut active,0)?!=Some(true) {
            jobs::run(&tx,tx.query("UPDATE actor_search_rebuild:catalog SET reason_code='reindex_job_not_active' RETURN NONE")).await?;
            return Ok(Some(RebuildAction::Waiting));
        }
        // An explicitly admitted rebuild may supersede a rejected live body.
        // Preserve that body and every source event until verified promotion;
        // do not force an invalid/oversized projection to succeed first.
        let lease=reserve(&tx,&control,&owner,&status,now).await?;
        if rebuild.phase=="repairing"&&status.active.is_some()&&status.rebuilding.is_none()&&!status.recovery_available {
            // Reconcile an uncertain repair response from the authenticated
            // current-source status; the new reader remains unbuilt until scan.
            rebuild.phase="requested".into();
            reset_after_repair(&tx).await?;
        }
        if rebuild.phase=="repairing" || status.recovery_available || (rebuild.phase=="requested"&&status.active.is_none()) {
            if !status.recovery_available {return Err(invalid("actor_index_explicit_repair_unavailable"));}
            if rebuild.repair_attempts>=3 {return Err(invalid("actor_index_repair_attempt_limit"));}
            let checksum=status.manifest_checksum.clone().ok_or_else(||invalid("actor_index_manifest_evidence_missing"))?;
            jobs::run(&tx,tx.query("UPDATE actor_search_rebuild:catalog SET phase='repairing' RETURN NONE")).await?;
            return Ok(Some(RebuildAction::Repair {request:ActorIndexRepair {deployment_id:status.deployment_id.clone(),source_epoch:status.source_epoch.clone(),request_id:format!("repair-{}",hash(&[&rebuild.request_id,&checksum])),expected_manifest_checksum:checksum,writer_epoch:lease.epoch},lease}));
        }
        if rebuild.phase=="requested" {
            let active=status.active.as_ref().ok_or_else(||invalid("actor_index_service_unavailable"))?;
            if active.visible_checkpoint>control.next_sequence as u64 || status.rebuilding.is_some() {return Err(invalid("actor_index_unknown_remote_generation"));}
            if active.visible_checkpoint>control.acknowledged_sequence as u64 {
                let pending:super::PreparedBatch=serde_json::from_str(&control.pending_json).map_err(|_|invalid("actor_index_unknown_remote_checkpoint"))?;
                if pending.batch.index_generation!=active.generation||pending.batch.deployment_id!=control.deployment_id||pending.batch.source_epoch!=control.source_epoch
                    ||pending.batch.from_checkpoint!=control.acknowledged_sequence as u64||pending.batch.through_checkpoint!=active.visible_checkpoint {
                    return Err(invalid("actor_index_unknown_remote_checkpoint"));
                }
            }
            let base=control.acknowledged_sequence.max(active.visible_checkpoint as i64);
            rebuild.expected_generation=active.generation.clone();rebuild.base_checkpoint=base;rebuild.visible_checkpoint=base;rebuild.phase="starting".into();
            jobs::run(&tx,tx.query("UPDATE actor_search_rebuild:catalog SET phase='starting',expected_generation=$generation,base_checkpoint=$base,visible_checkpoint=$base RETURN NONE")
                .bind(("generation",rebuild.expected_generation.clone())).bind(("base",rebuild.base_checkpoint))).await?;
        }
        if rebuild.phase=="starting" {return Ok(Some(RebuildAction::Start {request:request(&rebuild,&status,&lease),lease}));}
        if rebuild.phase=="promoting" {
            return Ok(Some(RebuildAction::Promote {request:ActorIndexPromote {deployment_id:status.deployment_id.clone(),source_epoch:status.source_epoch.clone(),
                expected_generation:rebuild.expected_generation.clone(),replacement_generation:rebuild.replacement_generation.clone(),writer_epoch:lease.epoch,catchup_checkpoint:rebuild.barrier as u64},lease}));
        }
        let shadow=status.rebuilding.as_ref().filter(|shadow|shadow.generation==rebuild.replacement_generation).ok_or_else(||invalid("actor_index_shadow_missing"))?;
        if shadow.visible_checkpoint<rebuild.visible_checkpoint as u64 {return Err(invalid("actor_index_shadow_checkpoint_lost"));}
        if !rebuild.pending_json.is_empty() {
            let mut pending:SavedBatch=serde_json::from_str(&rebuild.pending_json).map_err(|_|invalid("actor_index_rebuild_pending_invalid"))?;
            pending.batch.writer_epoch=lease.epoch;
            jobs::run(&tx,tx.query("UPDATE actor_search_rebuild:catalog SET pending_json=$body RETURN NONE").bind(("body",serde_json::to_string(&pending).map_err(|_|invalid("actor_index_rebuild_encoding"))?))).await?;
            return Ok(Some(RebuildAction::Deliver {lease,batch:pending.batch}));
        }
        if shadow.visible_checkpoint!=rebuild.visible_checkpoint as u64 {return Err(invalid("actor_index_unknown_shadow_checkpoint"));}
        let has_delta=control.next_sequence>rebuild.visible_checkpoint;
        // Three bounded scan pages earn one delta turn; a hot delta tail gets
        // one turn at a time rather than starving scan progress indefinitely.
        let delta=has_delta&&(rebuild.scan_complete||rebuild.scan_streak>=3);
        let mut pending=if delta {delta_batch(&tx,&rebuild,&control,&status,&lease).await?}
            else if !rebuild.scan_complete {scan_batch(&tx,&rebuild,&status,&lease,now).await?}
            else {
                // This barrier covers the canonical DB snapshot, separately
                // from completion of a still-running CSV import. The gateway
                // also checks the seed receipt before declaring setup ready.
                let batch=ActorIndexBatch {protocol:ACTOR_INDEX_PROTOCOL,deployment_id:status.deployment_id.clone(),source_epoch:status.source_epoch.clone(),index_generation:rebuild.replacement_generation.clone(),
                    batch_id:format!("rebuild-barrier-{}-{}",hash(&[&rebuild.request_id]),rebuild.visible_checkpoint),writer_epoch:lease.epoch,from_checkpoint:rebuild.visible_checkpoint as u64,
                    through_checkpoint:rebuild.visible_checkpoint as u64,source_complete:true,mutations:Vec::new()};
                SavedBatch {batch,scan_after:None,scan_complete:true,scanned:0}
            };
        pending.batch.writer_epoch=lease.epoch;pending.batch.validate().map_err(|_|invalid("actor_index_rebuild_batch_invalid"))?;
        jobs::run(&tx,tx.query("UPDATE actor_search_rebuild:catalog SET pending_json=$body RETURN NONE").bind(("body",serde_json::to_string(&pending).map_err(|_|invalid("actor_index_rebuild_encoding"))?))).await?;
        Ok(Some(RebuildAction::Deliver {lease,batch:pending.batch}))
    }}).await?.value)
}

async fn scan_batch(
    tx: &Tx,
    rebuild: &RebuildProgress,
    status: &ActorIndexStatus,
    lease: &Lease,
    now: DateTime<Utc>,
) -> Result<SavedBatch, TxError> {
    let limit = usize::from(status.limits.batch_documents).min(128);
    let mut response = jobs::run(
        tx,
        tx.query(
            "SELECT VALUE record::id(id) FROM brain_actor WHERE id>$after ORDER BY id LIMIT $limit",
        )
        .bind(("after", jobs::rid("brain_actor", &rebuild.scan_after)))
        .bind(("limit", limit as i64)),
    )
    .await?;
    let ids: Vec<String> = jobs::take(&mut response, 0)?;
    // Adoption gives missing old source actors a fence without changing the
    // revision of already indexed actors just because a rebuild observed them.
    jobs::run(tx,tx.query("FOR $actor IN $actors { LET $head=type::record('actor_search_head',$actor); IF !record::exists($head) { CREATE $head SET actor_id=$actor,revision=1,updated_at=<datetime>$now RETURN NONE; }; };")
        .bind(("actors",ids.clone())).bind(("now",now.to_rfc3339()))).await?;
    let rows = canonical_many_in(tx, &ids).await?;
    let mut batch = ActorIndexBatch {
        protocol: ACTOR_INDEX_PROTOCOL,
        deployment_id: status.deployment_id.clone(),
        source_epoch: status.source_epoch.clone(),
        index_generation: rebuild.replacement_generation.clone(),
        batch_id: format!(
            "rebuild-scan-{}-{}",
            hash(&[&rebuild.request_id]),
            hash(&[&rebuild.scan_after])
        ),
        writer_epoch: lease.epoch,
        from_checkpoint: rebuild.visible_checkpoint as u64,
        through_checkpoint: rebuild.visible_checkpoint as u64,
        source_complete: false,
        mutations: Vec::new(),
    };
    let mut last = rebuild.scan_after.clone();
    for row in rows {
        let mutation = ActorIndexMutation::new(row.actor_id.clone(), row.revision, row.projection)
            .map_err(|_| invalid("actor_index_rebuild_projection_invalid"))?;
        batch.mutations.push(mutation);
        if serde_json::to_vec(&batch)
            .map_err(|_| invalid("actor_index_rebuild_encoding"))?
            .len()
            > status.limits.batch_bytes as usize
        {
            batch.mutations.pop();
            break;
        }
        last = row.actor_id;
    }
    if !ids.is_empty() && batch.mutations.is_empty() {
        return Err(invalid("actor_index_rebuild_projection_oversize"));
    }
    let complete = ids.len() < limit && batch.mutations.len() == ids.len();
    Ok(SavedBatch {
        scanned: batch.mutations.len() as u64,
        batch,
        scan_after: Some(last),
        scan_complete: complete,
    })
}

async fn delta_batch(
    tx: &Tx,
    rebuild: &RebuildProgress,
    control: &Control,
    status: &ActorIndexStatus,
    lease: &Lease,
) -> Result<SavedBatch, TxError> {
    let mut response=jobs::run(tx,tx.query("SELECT sequence,actor_id,cause_job FROM actor_search_event WITH INDEX actor_search_event_sequence WHERE sequence>$after ORDER BY sequence LIMIT $limit")
        .bind(("after",rebuild.visible_checkpoint)).bind(("limit",usize::from(status.limits.batch_documents).min(super::BACKFILL_PAGE) as i64))).await?;
    let events: Vec<Event> = jobs::take(&mut response, 0)?;
    let ids: Vec<String> = events
        .iter()
        .map(|event| event.actor_id.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let canonical = canonical_many_in(tx, &ids)
        .await?
        .into_iter()
        .map(|row| (row.actor_id.clone(), row))
        .collect::<BTreeMap<_, _>>();
    let mut batch = ActorIndexBatch {
        protocol: ACTOR_INDEX_PROTOCOL,
        deployment_id: status.deployment_id.clone(),
        source_epoch: status.source_epoch.clone(),
        index_generation: rebuild.replacement_generation.clone(),
        batch_id: String::new(),
        writer_epoch: lease.epoch,
        from_checkpoint: rebuild.visible_checkpoint as u64,
        through_checkpoint: rebuild.visible_checkpoint as u64,
        source_complete: false,
        mutations: Vec::new(),
    };
    let mut seen = BTreeSet::new();
    for (offset, event) in events.into_iter().enumerate() {
        if event.sequence != rebuild.visible_checkpoint + offset as i64 + 1 {
            return Err(invalid("actor_index_rebuild_outbox_gap"));
        }
        let mut next = batch.clone();
        if seen.insert(event.actor_id.clone()) {
            let row = canonical
                .get(&event.actor_id)
                .ok_or_else(|| invalid("actor_index_rebuild_projection_missing"))?;
            next.mutations.push(
                ActorIndexMutation::new(row.actor_id.clone(), row.revision, row.projection.clone())
                    .map_err(|_| invalid("actor_index_rebuild_projection_invalid"))?,
            );
        }
        next.through_checkpoint = event.sequence as u64;
        next.batch_id = format!(
            "rebuild-delta-{}-{}-{}",
            hash(&[&rebuild.request_id]),
            next.from_checkpoint,
            next.through_checkpoint
        );
        if serde_json::to_vec(&next)
            .map_err(|_| invalid("actor_index_rebuild_encoding"))?
            .len()
            > status.limits.batch_bytes as usize
        {
            break;
        }
        batch = next;
    }
    if batch.through_checkpoint == batch.from_checkpoint
        && control.next_sequence > rebuild.visible_checkpoint
    {
        return Err(invalid("actor_index_rebuild_projection_oversize"));
    }
    Ok(SavedBatch {
        batch,
        scan_after: None,
        scan_complete: rebuild.scan_complete,
        scanned: 0,
    })
}

async fn lease_in(tx: &Tx, lease: &Lease, now: DateTime<Utc>) -> Result<(), TxError> {
    let control = control_in(tx).await?;
    if control.lease_owner != lease.owner
        || control.lease_epoch != lease.epoch as i64
        || instant(&control.lease_until)? <= now
    {
        return Err(invalid("actor_index_lease_lost"));
    }
    Ok(())
}

pub async fn rebuild_started(
    repo: &Repository,
    lease: &Lease,
    accepted: &ActorIndexGeneration,
    now: DateTime<Utc>,
) -> Result<(), TxError> {
    run_tx(repo.db(),RetryPolicy::default(),|tx|async move {
        guard(&tx).await?;lease_in(&tx,lease,now).await?;
        let rebuild=row(&tx).await?.ok_or_else(||invalid("actor_index_rebuild_missing"))?;
        if rebuild.phase!="starting" || accepted.committed_checkpoint!=rebuild.base_checkpoint as u64 || accepted.visible_checkpoint!=accepted.committed_checkpoint || accepted.writer_epoch!=lease.epoch {return Err(invalid("actor_index_rebuild_start_receipt_mismatch"));}
        jobs::run(&tx,tx.query("UPDATE actor_search_rebuild:catalog SET phase='scanning',replacement_generation=$generation,reason_code='' RETURN NONE")
            .bind(("generation",accepted.generation.clone()))).await?;Ok(())
    }).await?;
    Ok(())
}

pub async fn rebuild_repaired(
    repo: &Repository,
    lease: &Lease,
    accepted: &ActorIndexStatus,
    now: DateTime<Utc>,
) -> Result<(), TxError> {
    run_tx(repo.db(), RetryPolicy::default(), |tx| async move {
        guard(&tx).await?;
        lease_in(&tx, lease, now).await?;
        let rebuild = row(&tx)
            .await?
            .ok_or_else(|| invalid("actor_index_rebuild_missing"))?;
        let control = control_in(&tx).await?;
        if rebuild.phase != "repairing"
            || accepted.deployment_id != control.deployment_id
            || accepted.source_epoch != control.source_epoch
            || accepted.active.is_none()
            || accepted.rebuilding.is_some()
            || accepted.recovery_available
        {
            return Err(invalid("actor_index_repair_receipt_mismatch"));
        }
        reset_after_repair(&tx).await?;
        Ok(())
    })
    .await?;
    Ok(())
}

async fn reset_after_repair(tx: &Tx) -> Result<(), TxError> {
    jobs::run(tx,tx.query("UPDATE actor_search_rebuild:catalog SET phase='requested',expected_generation='',replacement_generation='',base_checkpoint=0,visible_checkpoint=0,scan_after='',scanned=0,scan_complete=false,scan_streak=0,barrier=0,pending_json='',reason_code='',repair_attempts+=1 RETURN NONE")).await?;
    Ok(())
}

pub async fn rebuild_acknowledge(
    repo: &Repository,
    lease: &Lease,
    receipt: &ActorIndexReceipt,
    now: DateTime<Utc>,
) -> Result<(), TxError> {
    run_tx(repo.db(),RetryPolicy::default(),|tx|async move {
        guard(&tx).await?;lease_in(&tx,lease,now).await?;
        let rebuild=row(&tx).await?.ok_or_else(||invalid("actor_index_rebuild_missing"))?;
        let pending:SavedBatch=serde_json::from_str(&rebuild.pending_json).map_err(|_|invalid("actor_index_rebuild_pending_invalid"))?;
        if receipt.batch_id!=pending.batch.batch_id || receipt.checksum!=pending.batch.content_checksum().map_err(|_|invalid("actor_index_rebuild_checksum"))?
            || receipt.index_generation!=rebuild.replacement_generation || receipt.committed_checkpoint!=pending.batch.through_checkpoint || receipt.visible_checkpoint!=receipt.committed_checkpoint || receipt.writer_epoch!=lease.epoch {
            return Err(invalid("actor_index_rebuild_receipt_mismatch"));
        }
        let promoting=pending.batch.source_complete;
        let scan=pending.scan_after.is_some();
        jobs::run(&tx,tx.query("UPDATE actor_search_rebuild:catalog SET visible_checkpoint=$through,scan_after=$after,scanned+=$scanned,scan_complete=$complete,scan_streak=$streak,pending_json='',phase=$phase,barrier=$barrier RETURN NONE")
            .bind(("through",receipt.visible_checkpoint as i64)).bind(("after",pending.scan_after.unwrap_or(rebuild.scan_after))).bind(("scanned",pending.scanned as i64))
            .bind(("complete",pending.scan_complete)).bind(("streak",if scan {rebuild.scan_streak+1} else {0}))
            .bind(("phase",if promoting {"promoting"} else if pending.scan_complete {"catchup"} else {"scanning"})).bind(("barrier",if promoting {receipt.visible_checkpoint as i64} else {0}))).await?;
        Ok(())
    }).await?;
    Ok(())
}

pub async fn rebuild_promoted(
    repo: &Repository,
    lease: &Lease,
    accepted: &ActorIndexGeneration,
    now: DateTime<Utc>,
) -> Result<(), TxError> {
    run_tx(repo.db(),RetryPolicy::default(),|tx|async move {
        guard(&tx).await?;lease_in(&tx,lease,now).await?;
        let rebuild=row(&tx).await?.ok_or_else(||invalid("actor_index_rebuild_missing"))?;
        if rebuild.phase!="promoting" || accepted.generation!=rebuild.replacement_generation || accepted.visible_checkpoint!=rebuild.barrier as u64
            || accepted.committed_checkpoint!=accepted.visible_checkpoint || !accepted.source_complete || accepted.writer_epoch!=lease.epoch {return Err(invalid("actor_index_promote_receipt_mismatch"));}
        let control=control_in(&tx).await?;
        if rebuild.barrier<control.acknowledged_sequence || rebuild.barrier>control.next_sequence {return Err(invalid("actor_index_promote_source_checkpoint_invalid"));}
        jobs::run(&tx,tx.query("UPDATE actor_search_control:catalog SET index_generation=$generation,acknowledged_sequence=$through,backfill_complete=true,pending_json='',blocked=false,failures=0,last_error='',retry_at=<datetime>$now,retired_through=$through RETURN NONE;
            UPDATE actor_search_rebuild:catalog SET phase='completed',reason_code='' RETURN NONE")
            .bind(("generation",accepted.generation.clone())).bind(("through",rebuild.barrier)).bind(("now",now.to_rfc3339()))).await?;
        // Source causes may now complete because their contiguous range is in
        // the serving reader. Old event storage is pruned in bounded pages.
        super::finish_causes_in(&tx,accepted.visible_checkpoint,now).await?;Ok(())
    }).await?;
    Ok(())
}
