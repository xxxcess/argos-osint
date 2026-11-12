//! Bounded live batches share the shadow pump's source lease and rate fence.

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use chrono::DateTime;
use chrono::Duration;
use chrono::Utc;
use fleetworks_contracts::actor_index::ActorIndexBatch;
use fleetworks_contracts::actor_index::ActorIndexMutation;
use fleetworks_contracts::actor_index::ActorIndexStatus;
use fleetworks_contracts::actor_index::ACTOR_INDEX_PROTOCOL;

use super::backfill_in;
use super::canonical_many_in;
use super::control_in;
use super::finish_causes_in;
use super::guard;
use super::hash;
use super::instant;
use super::invalid;
use super::jobs;
use super::resources;
use super::root_guard_in;
use super::run_tx;
use super::valid;
use super::Event;
use super::Lease;
use super::Preparation;
use super::PreparedBatch;
use super::Repository;
use super::RetryPolicy;
use super::TxError;

pub(super) async fn prepare(
    repo: &Repository,
    owner: &str,
    status: &ActorIndexStatus,
    now: DateTime<Utc>,
) -> Result<Preparation, TxError> {
    if !valid(owner) || status.protocol != ACTOR_INDEX_PROTOCOL {
        return Err(invalid("actor_index_worker_contract"));
    }
    status
        .limits
        .validate()
        .map_err(|_| invalid("actor_index_limits_invalid"))?;
    let active = status
        .active
        .clone()
        .ok_or_else(|| invalid("actor_index_service_unavailable"))?;
    if status.rebuilding.is_some() {
        return Err(invalid("actor_index_rebuild_requires_coordinated_scan"));
    }
    let owner = owner.to_owned();
    // Bound decoded alias provenance separately from serialized batch bytes.
    // Never fetch a speculative second source page while preparing this one.
    let page = usize::from(status.limits.batch_documents).min(super::BACKFILL_PAGE);
    Ok(run_tx(repo.db(),RetryPolicy::default(),|tx| {let owner=owner.clone();let active=active.clone();async move {
        guard(&tx).await?;
        let mut response=jobs::run(&tx,tx.query("SELECT VALUE instance_id FROM ONLY storage_meta:identity")).await?;
        if jobs::take::<Option<String>>(&mut response,0)?.as_deref()!=Some(status.deployment_id.as_str()) {return Err(invalid("actor_index_deployment_mismatch"));}
        let mut control=control_in(&tx).await?;
        finish_causes_in(&tx,control.acknowledged_sequence as u64,now).await?;
        resources::prune_in(&tx,control.acknowledged_sequence).await?;
        if control.deployment_id.is_empty() {
            jobs::run(&tx,tx.query("UPDATE actor_search_control:catalog SET deployment_id=$deployment,source_epoch=$source,index_generation=$generation RETURN NONE")
                .bind(("deployment",status.deployment_id.clone())).bind(("source",status.source_epoch.clone())).bind(("generation",active.generation.clone()))).await?;
            control.deployment_id=status.deployment_id.clone();control.source_epoch=status.source_epoch.clone();control.index_generation=active.generation.clone();
        }
        if control.deployment_id!=status.deployment_id||control.source_epoch!=status.source_epoch||control.index_generation!=active.generation {return Err(invalid("actor_index_source_or_generation_mismatch"));}
        if active.visible_checkpoint<control.acknowledged_sequence as u64 {return Err(invalid("actor_index_rebuild_required"));}
        if instant(&control.lease_until)?>now&&control.lease_owner!=owner {return Ok(Preparation::Waiting);}
        if control.blocked||instant(&control.retry_at)?>now||instant(&control.next_start_at)?>now {return Ok(Preparation::Waiting);}
        let remote=i64::try_from(active.writer_epoch).map_err(|_|invalid("actor_index_epoch_invalid"))?;
        let epoch=if control.lease_owner==owner&&instant(&control.lease_until)?>now&&control.lease_epoch>=remote {control.lease_epoch}
            else {control.lease_epoch.max(remote).checked_add(1).ok_or_else(||invalid("actor_index_epoch_exhausted"))?};
        jobs::run(&tx,tx.query("UPDATE actor_search_control:catalog SET lease_owner=$owner,lease_epoch=$epoch,lease_until=<datetime>$until,effective_limits_json=$limits RETURN NONE")
            .bind(("owner",owner.clone())).bind(("epoch",epoch)).bind(("until",(now+Duration::seconds(45)).to_rfc3339()))
            .bind(("limits",serde_json::to_string(&status.limits).map_err(|_|invalid("actor_index_limits_encoding"))?))).await?;
        let lease=Lease {owner:owner.clone(),epoch:epoch as u64};
        let mut prepared=if !control.pending_json.is_empty() {
            let prepared:PreparedBatch=serde_json::from_str(&control.pending_json).map_err(|_|invalid("actor_index_pending_invalid"))?;
            if prepared.batch.mutations.len()>usize::from(status.limits.batch_documents)
                || serde_json::to_vec(&prepared.batch).map_err(|_|invalid("actor_index_encoding"))?.len()>status.limits.batch_bytes as usize {
                // Policy tightening must not silently split an uncertain commit.
                return Err(invalid("actor_index_pending_exceeds_current_limits"));
            }
            prepared
        } else {
            if active.visible_checkpoint!=control.acknowledged_sequence as u64 {return Err(invalid("actor_index_unknown_remote_checkpoint"));}
            backfill_in(&tx,&control,now).await?;control=control_in(&tx).await?;
            let mut response=jobs::run(&tx,tx.query("SELECT sequence,actor_id,cause_job FROM actor_search_event WITH INDEX actor_search_event_sequence WHERE sequence>$after ORDER BY sequence LIMIT $limit")
                .bind(("after",control.acknowledged_sequence)).bind(("limit",page as i64))).await?;
            let events:Vec<Event>=jobs::take(&mut response,0)?;
            let mut seed=jobs::run(&tx,tx.query("RETURN (SELECT VALUE status FROM ONLY brain_actor_seed:catalog)='complete'")).await?;
            let caught_up=events.last().map(|event|event.sequence).unwrap_or(control.acknowledged_sequence)==control.next_sequence;
            let source_complete=control.backfill_complete&&caught_up&&jobs::take::<Option<bool>>(&mut seed,0)?==Some(true);
            if events.is_empty()&&(!source_complete||active.source_complete) {return Ok(Preparation::Waiting);}
            // Flush a partial live tail within the configured bound. The first
            // observation is persisted so replicas/restarts cannot extend it.
            if !events.is_empty()&&!source_complete&&events.len()<page {
                if control.tail_since.is_empty() {
                    jobs::run(&tx,tx.query("UPDATE actor_search_control:catalog SET tail_since=$now RETURN NONE").bind(("now",now.to_rfc3339()))).await?;
                    return Ok(Preparation::Waiting);
                }
                if now<instant(&control.tail_since)?+Duration::milliseconds(i64::from(status.limits.partial_tail_ms)) {return Ok(Preparation::Waiting);}
            }
            let ids:Vec<String>=events.iter().map(|event|event.actor_id.clone()).collect::<BTreeSet<_>>().into_iter().collect();
            let canonical=canonical_many_in(&tx,&ids).await?.into_iter().map(|row|(row.actor_id.clone(),row)).collect::<BTreeMap<_,_>>();
            let mut candidate=ActorIndexBatch {protocol:ACTOR_INDEX_PROTOCOL,deployment_id:status.deployment_id.clone(),source_epoch:status.source_epoch.clone(),index_generation:active.generation.clone(),
                batch_id:String::new(),writer_epoch:epoch as u64,from_checkpoint:control.acknowledged_sequence as u64,through_checkpoint:control.acknowledged_sequence as u64,source_complete:false,mutations:Vec::new()};
            let mut counts=BTreeMap::<String,u64>::new();let mut seen=BTreeSet::new();
            for (offset,event) in events.into_iter().enumerate() {
                if event.sequence!=control.acknowledged_sequence+offset as i64+1 {return Err(invalid("actor_index_outbox_gap"));}
                let mut next=candidate.clone();
                if seen.insert(event.actor_id.clone()) {
                    let row=canonical.get(&event.actor_id).ok_or_else(||invalid("actor_projection_missing"))?;
                    next.mutations.push(ActorIndexMutation::new(row.actor_id.clone(),row.revision,row.projection.clone()).map_err(|_|invalid("actor_projection_invalid"))?);
                }
                next.through_checkpoint=event.sequence as u64;
                next.batch_id=format!("actor-{}-{}-{}",hash(&[&status.source_epoch]),next.from_checkpoint,next.through_checkpoint);
                if serde_json::to_vec(&next).map_err(|_|invalid("actor_index_encoding"))?.len()>status.limits.batch_bytes as usize {
                    if candidate.mutations.is_empty() {return Err(invalid("actor_projection_oversize"));}break;
                }
                if let Some(cause)=event.cause_job {*counts.entry(cause).or_default()+=1;}
                candidate=next;
            }
            candidate.source_complete=source_complete&&candidate.through_checkpoint==control.next_sequence as u64;
            if candidate.through_checkpoint==candidate.from_checkpoint {candidate.batch_id=format!("actor-barrier-{}-{}",hash(&[&status.source_epoch]),candidate.through_checkpoint);}
            PreparedBatch {batch:candidate,cause_counts:counts}
        };
        prepared.batch.writer_epoch=lease.epoch;prepared.batch.validate().map_err(|_|invalid("actor_projection_invalid"))?;
        jobs::run(&tx,tx.query("UPDATE actor_search_control:catalog SET pending_json=$pending,next_start_at=<datetime>$next,tail_since='' RETURN NONE")
            .bind(("pending",serde_json::to_string(&prepared).map_err(|_|invalid("actor_index_encoding"))?)).bind(("next",(now+Duration::milliseconds(i64::from(status.limits.min_start_ms))).to_rfc3339()))).await?;
        for cause in prepared.cause_counts.keys() {
            root_guard_in(&tx,cause).await?;
            jobs::run(&tx,tx.query("UPDATE $job SET state='running',worker_owner=$owner,heartbeat_at=<datetime>$now WHERE state='queued' RETURN NONE")
                .bind(("job",jobs::rid("job",cause))).bind(("owner",owner.clone())).bind(("now",now.to_rfc3339()))).await?;
        }
        Ok(Preparation::Batch {lease,prepared:Box::new(prepared)})
    }}).await?.value)
}
