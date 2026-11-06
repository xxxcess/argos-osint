//! Independent, checkpointed Atlas derivative work. Collection IDs are provenance,
//! never authority. Each task owns one occurrence/revision and its own live lease.
//! @tag:atlas-collection @tag:classifier
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use surrealdb::types::SurrealValue;

use crate::atlas_classify::OccurrenceKey;
use crate::storage::classifier_admission::{
    self as admission, AdmissionOwner, DerivativeFence, DerivativeStage, DerivativeWork,
};
use crate::storage::jobs::{self, ClaimedTask, NewJob, NewTask};
use crate::storage::repo::Repository;
use crate::storage::tx::{RetryPolicy, Tx, TxError};

pub const POOL: &str = "atlas_derivatives";
pub const OPERATION: &str = "atlas_derivative";
pub const LEASE_SECONDS: i64 = 60;
pub const MAX_ACTIVATION_UNITS: usize = 16;
pub const DISPATCH_BUDGET: Duration = Duration::from_secs(2);
pub const SCHEMA: &str = "
DEFINE TABLE IF NOT EXISTS atlas_derivative_job SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS run_id ON atlas_derivative_job TYPE string;
DEFINE FIELD IF NOT EXISTS article_id ON atlas_derivative_job TYPE string;
DEFINE FIELD IF NOT EXISTS receipt_key ON atlas_derivative_job TYPE string;
DEFINE FIELD IF NOT EXISTS activation ON atlas_derivative_job TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS cleanup_only ON atlas_derivative_job TYPE bool DEFAULT false;
DEFINE FIELD IF NOT EXISTS pages ON atlas_derivative_job TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS last_error ON atlas_derivative_job TYPE string DEFAULT '';
DEFINE INDEX IF NOT EXISTS atlas_derivative_job_run ON atlas_derivative_job FIELDS run_id,article_id;
";

fn invalid(code: &str) -> TxError {
    TxError::Abort(code.into())
}

#[derive(Clone, Debug, Serialize, Deserialize, SurrealValue)]
struct WorkRow {
    run_id: String,
    article_id: String,
    receipt_key: String,
    activation: i64,
    cleanup_only: bool,
}

/// Idempotent durable handoff in the admission/source publication transaction.
/// No live notification or parent-cycle lease is required after this commits.
pub(crate) async fn enqueue_in(
    tx: &Tx,
    occurrence: &OccurrenceKey,
    receipt_key: &str,
    now: DateTime<Utc>,
) -> Result<(), TxError> {
    enqueue_revision_in(tx, occurrence, receipt_key, "admission", now).await
}

pub(crate) async fn enqueue_revision_in(
    tx: &Tx,
    occurrence: &OccurrenceKey,
    receipt_key: &str,
    trigger: &str,
    now: DateTime<Utc>,
) -> Result<(), TxError> {
    let mut current = tx
        .query("RETURN (SELECT VALUE receipt_key FROM ONLY $projection)=$receipt;")
        .bind((
            "projection",
            jobs::rid("atlas_classification_projection", &occurrence.record_id()),
        ))
        .bind(("receipt", receipt_key.to_owned()))
        .await?
        .check()?;
    if current.take::<Option<bool>>(0)? != Some(true) {
        return Ok(());
    }
    enqueue_job_in(tx, occurrence, receipt_key, trigger, false, now).await
}

async fn enqueue_job_in(
    tx: &Tx,
    occurrence: &OccurrenceKey,
    receipt_key: &str,
    trigger: &str,
    cleanup_only: bool,
    now: DateTime<Utc>,
) -> Result<(), TxError> {
    let identity = crate::classifier::artifact::sha256_hex(
        format!(
            "atlas-derivative-job-v2\0{}\0{receipt_key}\0{trigger}",
            occurrence.record_id()
        )
        .as_bytes(),
    );
    let id = format!("atlas-derivative-{identity}");
    jobs::bump(tx, "job_guard", &id).await?;
    let mut response = tx
        .query("RETURN record::exists($row);")
        .bind(("row", jobs::rid("atlas_derivative_job", &id)))
        .await?
        .check()?;
    if response.take::<Option<bool>>(0)? == Some(true) {
        return Ok(());
    }
    jobs::create_job_in(
        tx,
        &NewJob {
            id: &id,
            kind: "maintenance",
            operation: OPERATION,
            parent: None,
            created_at: now,
        },
    )
    .await?;
    let task = format!("{id}:activation:0");
    jobs::create_task_in(
        tx,
        &NewTask {
            id: &task,
            job: &id,
            pool: POOL,
            operation: OPERATION,
            priority: 30,
            deps: Vec::new(),
            input_hash: &identity,
            created_at: now,
        },
    )
    .await?;
    tx.query("CREATE $row SET run_id=$run,article_id=$article,receipt_key=$receipt,cleanup_only=$cleanup,activation=0,pages=0,last_error='' RETURN NONE; UPDATE $job SET run_ref=$run,resource_ref=$run,phase=IF $cleanup {'Published cleanup queued'} ELSE {'Derivatives queued'},progress_total=IF $cleanup {1} ELSE {4},progress_done=0 RETURN NONE;")
        .bind(("row",jobs::rid("atlas_derivative_job",&id))).bind(("job",jobs::rid("job",&id)))
        .bind(("run",occurrence.run_id.clone())).bind(("article",occurrence.article_id.clone())).bind(("receipt",receipt_key.to_owned())).bind(("cleanup",cleanup_only)).await?.check()?;
    Ok(())
}

/// A source replacement retains any already published cleanup even when the
/// old job was paused/cancelled and the new classification rejects its input.
/// Fresh rejected occurrences have no owed checkpoint and create no job.
pub(crate) async fn enqueue_cleanup_in(
    tx: &Tx,
    occurrence: &OccurrenceKey,
    now: DateTime<Utc>,
) -> Result<(), TxError> {
    let mut response = tx.query("SELECT receipt_key FROM atlas_classification_derivative WITH INDEX atlas_derivative_occurrence_stage WHERE run_id=$run AND article_id=$article AND stage='reconcile' AND state!='complete' AND (checkpoint_json CONTAINS '\"frontier_kind\":\"reconcile\"' OR checkpoint_json CONTAINS '\"frontier_kind\":\"reconcile_restart\"') ORDER BY input_revision,receipt_key LIMIT 1; RETURN (SELECT VALUE input_revision FROM ONLY $projection) ?? 0;")
        .bind(("run",occurrence.run_id.clone())).bind(("article",occurrence.article_id.clone()))
        .bind(("projection",jobs::rid("atlas_classification_projection",&occurrence.record_id()))).await?.check()?;
    #[derive(SurrealValue)]
    struct Owed {
        receipt_key: String,
    }
    let rows: Vec<Owed> = response.take(0)?;
    let Some(row) = rows.first() else {
        return Ok(());
    };
    let revision: Option<i64> = response.take(1)?;
    enqueue_job_in(
        tx,
        occurrence,
        &row.receipt_key,
        &format!("cleanup:{}", revision.unwrap_or_default()),
        true,
        now,
    )
    .await
}

pub(crate) async fn has_owed_cleanup_in(
    tx: &Tx,
    occurrence: &OccurrenceKey,
) -> Result<bool, TxError> {
    let mut response = tx.query("RETURN count(SELECT id FROM atlas_classification_derivative WITH INDEX atlas_derivative_occurrence_stage WHERE run_id=$run AND article_id=$article AND stage='reconcile' AND state!='complete' AND (checkpoint_json CONTAINS '\"frontier_kind\":\"reconcile\"' OR checkpoint_json CONTAINS '\"frontier_kind\":\"reconcile_restart\"') LIMIT 1)>0;")
        .bind(("run",occurrence.run_id.clone())).bind(("article",occurrence.article_id.clone())).await?.check()?;
    Ok(response.take::<Option<bool>>(0)? == Some(true))
}

/// Also fences a Reconcile preparation lease whose checkpoint is still empty:
/// publication must be able to hand off its new cursor without losing it.
pub(crate) async fn publication_blocked_in(
    tx: &Tx,
    occurrence: &OccurrenceKey,
) -> Result<bool, TxError> {
    let mut response = tx.query("RETURN count(SELECT id FROM atlas_classification_derivative WITH INDEX atlas_derivative_occurrence_stage WHERE run_id=$run AND article_id=$article AND stage='reconcile' AND ((state!='complete' AND (checkpoint_json CONTAINS '\"frontier_kind\":\"reconcile\"' OR checkpoint_json CONTAINS '\"frontier_kind\":\"reconcile_restart\"')) OR (owner!='' AND lease_until!=NONE AND lease_until>time::now())) LIMIT 1)>0;")
        .bind(("run",occurrence.run_id.clone())).bind(("article",occurrence.article_id.clone())).await?.check()?;
    Ok(response.take::<Option<bool>>(0)? == Some(true))
}

pub(crate) struct CleanupProgress {
    pub changed_memory_ids: Vec<String>,
    pub pending: bool,
}

/// Caller must hold a live current capability and the occurrence source guard
/// in this publisher transaction. This can only advance saved published work.
pub(crate) async fn cleanup_one_in(
    tx: &Tx,
    occurrence: &OccurrenceKey,
) -> Result<Option<CleanupProgress>, TxError> {
    admission::derivative_source_guards_in(tx, std::slice::from_ref(occurrence)).await?;
    let mut response = tx.query("SELECT receipt_key,owner,epoch,(owner!='' AND lease_until!=NONE AND lease_until>time::now()) AS busy,IF bytes::len(type::bytes(checkpoint_json))<=16384 {checkpoint_json} ELSE {NONE} AS checkpoint_json FROM atlas_classification_derivative WITH INDEX atlas_derivative_occurrence_stage WHERE run_id=$run AND article_id=$article AND stage='reconcile' AND state!='complete' AND (checkpoint_json CONTAINS '\"frontier_kind\":\"reconcile\"' OR checkpoint_json CONTAINS '\"frontier_kind\":\"reconcile_restart\"') ORDER BY input_revision,receipt_key LIMIT 1;")
        .bind(("run",occurrence.run_id.clone())).bind(("article",occurrence.article_id.clone())).await?.check()?;
    #[derive(SurrealValue)]
    struct Owed {
        receipt_key: String,
        owner: String,
        epoch: i64,
        busy: bool,
        checkpoint_json: Option<String>,
    }
    let rows: Vec<Owed> = response.take(0)?;
    let Some(row) = rows.into_iter().next() else {
        return Ok(None);
    };
    if row.busy {
        return Ok(Some(CleanupProgress {
            changed_memory_ids: Vec::new(),
            pending: true,
        }));
    }
    let checkpoint = row
        .checkpoint_json
        .ok_or_else(|| invalid("cleanup_checkpoint_too_large"))?;
    let cursor = serde_json::from_str::<crate::storage::lifecycle::ReconcileCursor>(&checkpoint)
        .map_err(|_| invalid("cleanup_checkpoint_invalid"))?;
    let scope = crate::storage::lifecycle::ReconcileScope {
        keys: vec![crate::storage::lifecycle::ReconcileKey::Source {
            run_id: occurrence.run_id.clone(),
            article_id: occurrence.article_id.clone(),
        }],
    };
    let progress = crate::storage::lifecycle::resume_published_in(tx, &scope, &cursor).await?;
    let next = progress
        .next
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|_| invalid("cleanup_checkpoint_encode"))?
        .unwrap_or_default();
    let pending = !next.is_empty();
    let disposition = if pending {
        admission::DerivativeDisposition::Retryable {
            code: "published_cleanup_continuation".into(),
        }
    } else {
        admission::DerivativeDisposition::Current
    };
    let mut response = tx.query("UPDATE $stage SET state=IF $pending {'pending'} ELSE {'complete'},checkpoint_json=$next,disposition_json=$disposition,owner='',epoch+=1,lease_until=NONE,last_error=IF $pending {'published_cleanup_continuation'} ELSE {''},updated_at=time::now() WHERE epoch=$epoch AND owner=$owner AND checkpoint_json=$checkpoint RETURN AFTER;")
        .bind(("stage",admission::derivative_record(&row.receipt_key,DerivativeStage::Reconcile)))
        .bind(("pending",pending)).bind(("next",next)).bind(("epoch",row.epoch)).bind(("owner",row.owner)).bind(("checkpoint",checkpoint))
        .bind(("disposition",serde_json::to_string(&disposition).map_err(|_|invalid("cleanup_disposition_encode"))?)).await?.check()?;
    #[derive(SurrealValue)]
    struct Updated {
        receipt_key: String,
    }
    let updated: Vec<Updated> = response.take(0)?;
    if updated.len() != 1 || updated[0].receipt_key != row.receipt_key {
        return Err(invalid("cleanup_stage_changed"));
    }
    // A previous build may have left several receipts outstanding. Keep this
    // selective wakeup runnable until every saved published cursor is drained.
    let remaining = has_owed_cleanup_in(tx, occurrence).await?;
    Ok(Some(CleanupProgress {
        changed_memory_ids: progress.changed_memory_ids,
        pending: remaining,
    }))
}

/// Independent task capability. All source/metadata/body checks still run in the
/// existing derivative processor, in the publication/checkpoint transaction.
#[derive(Debug, Clone)]
pub struct JobFence {
    pub(crate) task: ClaimedTask,
    pub(crate) occurrence: OccurrenceKey,
    pub(crate) receipt_key: String,
}
impl JobFence {
    pub(crate) fn identity(&self) -> (&str, u64) {
        (&self.task.id, self.task.lease_epoch as u64)
    }
    pub(crate) fn contains(&self, key: &OccurrenceKey) -> bool {
        &self.occurrence == key
    }
    pub(crate) async fn check_in(
        &self,
        tx: &Tx,
        now: DateTime<Utc>,
        renew: bool,
    ) -> Result<bool, TxError> {
        if self.task.pool != POOL || self.task.operation != OPERATION || self.task.lease_epoch < 1 {
            return Ok(false);
        }
        jobs::bump(tx, "job_guard", &self.task.root_id).await?;
        let Some(fence) = jobs::fence(tx, &self.task, &now.to_rfc3339()).await? else {
            return Ok(false);
        };
        if !jobs::holds(&fence, &self.task) || fence.cancelled {
            return Ok(false);
        }
        let mut reply=tx.query("RETURN (SELECT VALUE receipt_key FROM ONLY $projection)=$receipt AND (SELECT VALUE state FROM ONLY $job)='running' AND !((SELECT VALUE stopped FROM ONLY $job) ?? false);")
            .bind(("job",jobs::rid("job",&self.task.job_id))).bind(("projection",jobs::rid("atlas_classification_projection",&self.occurrence.record_id()))).bind(("receipt",self.receipt_key.clone())).await?.check()?;
        if reply.take::<Option<bool>>(0)? != Some(true) {
            return Ok(false);
        }
        if renew && !jobs::renew_owned_in(tx, &self.task, LEASE_SECONDS, now).await? {
            return Ok(false);
        }
        Ok(true)
    }
}

/// One activation dispatches one occurrence and at most one continuation page.
/// This intentionally starts below the 16-unit ceiling; no preparation pool is
/// introduced until measured preparation cost warrants it. In-flight database
/// work is cooperative and can overrun the dispatch budget.
pub async fn drain_one(
    repo: &Repository,
    worker: &str,
    cancel: Arc<AtomicBool>,
) -> anyhow::Result<usize> {
    static FOREGROUND_TURNS: AtomicU64 = AtomicU64::new(0);
    let started = Instant::now();
    if cancel.load(Ordering::Acquire) {
        return Ok(0);
    }
    // Foreground gets priority, but an older derivative receives a fair turn.
    if admission::foreground_requested(repo, Utc::now()).await?
        && !FOREGROUND_TURNS
            .fetch_add(1, Ordering::Relaxed)
            .is_multiple_of(8)
    {
        return Ok(0);
    }
    let owner = AdmissionOwner::new(repo.clone());
    let outcome=owner.scope(async {
        // A slow foreground probe must also prevent a new discovery page.
        if started.elapsed()>=DISPATCH_BUDGET {return Ok(0);}
        static DISCOVERY_TURNS: AtomicU64 = AtomicU64::new(0);
        if DISCOVERY_TURNS.fetch_add(1,Ordering::Relaxed).is_multiple_of(2) {
            let discovered=crate::storage::atlas_derivative_invalidation::discover_one(&owner,Utc::now()).await?;
            if discovered>0 {tracing::info!(target:"argos::atlas_derivatives",stage="invalidation",units=discovered as u64,activation_ms=started.elapsed().as_millis() as u64,budget_overrun=started.elapsed()>DISPATCH_BUDGET,"derivative_activation");return Ok(discovered);}
        }
        if started.elapsed()>=DISPATCH_BUDGET {return Ok(0);}
        let name=worker.to_owned();
        let task=admission::owned_tx(&owner,RetryPolicy::default(),move |tx| {
            let name=name.clone();async move {
                let now=Utc::now();jobs::interrupt_expired_pool_in(&tx,POOL,now).await?;
                jobs::claim_in(&tx,POOL,None,&name,LEASE_SECONDS,now,&format!("derivative:{name}:{}",now.timestamp_nanos_opt().unwrap_or_default()),None).await
            }
        }).await?.value;
        let Some(task)=task else {return Ok::<usize,anyhow::Error>(0);};
        let mut response=owner.query("SELECT run_id,article_id,receipt_key,activation,(cleanup_only ?? false) AS cleanup_only FROM ONLY $row;").bind(("row",jobs::rid("atlas_derivative_job",&task.job_id))).await?.check()?;
        let row:WorkRow=response.take::<Option<WorkRow>>(0)?.ok_or_else(||invalid("derivative_job_missing"))?;
        if row.cleanup_only {
            let dispatch_allowed = started.elapsed() < DISPATCH_BUDGET && !cancel.load(Ordering::Acquire);
            let dispatched_at = Instant::now();
            drain_cleanup_task(&owner, &task, &row, &cancel, dispatch_allowed).await?;
            tracing::info!(target:"argos::atlas_derivatives",job_id=task.job_id,input_sha256=task.input_hash,epoch=task.lease_epoch,stage="cleanup",elapsed_ms=dispatched_at.elapsed().as_millis() as u64,activation_ms=started.elapsed().as_millis() as u64,budget_overrun=started.elapsed()>DISPATCH_BUDGET,units=usize::from(dispatch_allowed) as u64,"derivative_activation");
            return Ok(1);
        }
        if settle_superseded(&owner,&task,&row).await? {return Ok(1);}
        let occurrence=OccurrenceKey::new(&row.run_id,&row.article_id);
        let work=DerivativeWork::new(occurrence.clone(),row.receipt_key.clone());
        let fence=DerivativeFence::Job(Box::new(JobFence {task:task.clone(),occurrence,receipt_key:row.receipt_key.clone()}));
        // Prerequisites are settled in order using the same processors as the
        // synchronous path. No dependency is dropped to make a job runnable.
        let stage=next_stage(&owner,&row,&task).await?;
        let dispatched_at=Instant::now();
        let report=if let Some(stage)=stage.filter(|_|started.elapsed()<DISPATCH_BUDGET && !cancel.load(Ordering::Acquire)) {
            match admission::process_derivative_stage(&owner,&fence,std::slice::from_ref(&work),stage,&cancel).await {
                Ok(report)=>Some(report),
                Err(error)=>{if settle_superseded(&owner,&task,&row).await? {return Ok(1);}return Err(error.into());}
            }
        } else {None};
        let remaining=next_stage(&owner,&row,&task).await?;
        let last_error=report.as_ref().and_then(|r|r.items.iter().find_map(|item| match &item.disposition {
            admission::DerivativeDisposition::Retryable {code}=>Some(code.clone()),admission::DerivativeDisposition::StaleInput=>Some("stale_input".into()),_=>None
        })).unwrap_or_default();
        let terminal=remaining.is_none();
        let progress=remaining.map_or(4,|stage|match stage {DerivativeStage::Metadata=>0,DerivativeStage::Related=>1,DerivativeStage::Actors=>2,DerivativeStage::Reconcile=>3});
        let useful_units=report.as_ref().map_or(0,|report|report.items.len());
        let phase=remaining.map(|stage|format!("{} pending",stage.as_str())).unwrap_or_else(||"Derivatives ready".into());
        let trace_job=task.job_id.clone();
        let trace_input=task.input_hash.clone();
        let trace_epoch=task.lease_epoch;
        let captured=(task,row,terminal,last_error,phase);
        admission::owned_tx(&owner,RetryPolicy::default(),move |tx| {
            let (task,row,terminal,last_error,phase)=captured.clone();async move {
                let now=Utc::now();jobs::bump(&tx,"job_guard",&task.root_id).await?;
                let Some(fence)=jobs::fence(&tx,&task,&now.to_rfc3339()).await? else {return Err(invalid("derivative_task_lost"));};
                if !jobs::holds(&fence,&task) {return Err(invalid("derivative_task_lost"));}
                if fence.cancelled {return Err(invalid("derivative_cancelled"));}
                if terminal {jobs::complete_in(&tx,&task,&row.receipt_key,&now.to_rfc3339()).await?;} else {
                    let next=format!("{}:activation:{}",task.job_id,row.activation+1);
                    let delay=if last_error.is_empty() || last_error.ends_with("continuation") {1} else {30};
                    if jobs::yield_owned_in(&tx,&task,&NewTask {id:&next,job:&task.job_id,pool:POOL,operation:OPERATION,priority:30,deps:vec![&task.id],input_hash:&task.input_hash,created_at:now},now+chrono::Duration::seconds(delay),"derivative_checkpoint",now).await?!=jobs::Publish::Committed {return Err(invalid("derivative_yield_lost"));}
                }
                tx.query("UPDATE $row SET activation+=1,pages+=$units,last_error=$error RETURN NONE; UPDATE $job SET state=$state,phase=$phase,progress_done=$progress,error_summary=$error,worker_owner='',finished_at=IF $terminal {<datetime>$now} ELSE {NONE},updated_at=<datetime>$now RETURN NONE;")
                    .bind(("row",jobs::rid("atlas_derivative_job",&task.job_id))).bind(("job",jobs::rid("job",&task.job_id)))
                    .bind(("error",last_error)).bind(("state",if terminal {"completed"} else {"running"})).bind(("phase",phase)).bind(("progress",progress)).bind(("units",useful_units as i64)).bind(("terminal",terminal)).bind(("now",now.to_rfc3339())).await?.check()?;
                Ok(())
            }
        }).await?;
        tracing::info!(target:"argos::atlas_derivatives", job_id=trace_job,input_sha256=trace_input,epoch=trace_epoch,stage=stage.map(DerivativeStage::as_str).unwrap_or("ready"), elapsed_ms=dispatched_at.elapsed().as_millis() as u64, activation_ms=started.elapsed().as_millis() as u64, budget_overrun=started.elapsed()>DISPATCH_BUDGET, units=useful_units as u64, "derivative_activation");
        Ok(1)
    }).await;
    let cleanup = owner.shutdown().await;
    let value = outcome?;
    cleanup?;
    Ok(value)
}

async fn drain_cleanup_task(
    owner: &AdmissionOwner,
    task: &ClaimedTask,
    row: &WorkRow,
    cancel: &Arc<AtomicBool>,
    dispatch_allowed: bool,
) -> Result<(), admission::AdmissionError> {
    let (task, row, cancel) = (task.clone(), row.clone(), cancel.clone());
    let repo = owner.repository().clone();
    let write: crate::storage::memory::MemoryWrite = Arc::new(move |tx| {
        let (task, row, cancel) = (task.clone(), row.clone(), cancel.clone());
        Box::pin(async move {
            let now = Utc::now();
            if task.pool != POOL || task.operation != OPERATION {
                return Err(invalid("cleanup_task_invalid"));
            }
            jobs::bump(&tx, "job_guard", &task.root_id).await?;
            let Some(fence) = jobs::fence(&tx, &task, &now.to_rfc3339()).await? else {
                return Err(invalid("cleanup_task_lost"));
            };
            if !jobs::holds(&fence, &task) || fence.cancelled {
                return Err(invalid("cleanup_task_lost"));
            }
            let mut active = tx.query("RETURN (SELECT VALUE state FROM ONLY $job)='running' AND !((SELECT VALUE stopped FROM ONLY $job) ?? false) AND (SELECT VALUE cleanup_only FROM ONLY $row)=true AND (SELECT VALUE run_id FROM ONLY $row)=$run AND (SELECT VALUE article_id FROM ONLY $row)=$article;")
                .bind(("job",jobs::rid("job",&task.job_id))).bind(("row",jobs::rid("atlas_derivative_job",&task.job_id)))
                .bind(("run",row.run_id.clone())).bind(("article",row.article_id.clone())).await?.check()?;
            if active.take::<Option<bool>>(0)? != Some(true)
                || !jobs::renew_owned_in(&tx, &task, LEASE_SECONDS, now).await?
            {
                return Err(invalid("cleanup_task_lost"));
            }
            let occurrence = OccurrenceKey::new(&row.run_id, &row.article_id);
            let dispatched = dispatch_allowed && !cancel.load(Ordering::Acquire);
            let progress = if dispatched {
                cleanup_one_in(&tx, &occurrence).await?
            } else {
                None
            };
            let pending = !dispatched || progress.as_ref().is_some_and(|progress| progress.pending);
            if dispatched && cancel.load(Ordering::Acquire) {
                return Err(invalid("cleanup_cancelled"));
            }
            if pending {
                let next = format!("{}:activation:{}", task.job_id, row.activation + 1);
                if jobs::yield_owned_in(
                    &tx,
                    &task,
                    &NewTask {
                        id: &next,
                        job: &task.job_id,
                        pool: POOL,
                        operation: OPERATION,
                        priority: 30,
                        deps: vec![&task.id],
                        input_hash: &task.input_hash,
                        created_at: now,
                    },
                    now + chrono::Duration::seconds(1),
                    "published_cleanup",
                    now,
                )
                .await?
                    != jobs::Publish::Committed
                {
                    return Err(invalid("cleanup_yield_lost"));
                }
            } else {
                jobs::complete_in(&tx, &task, "published_cleanup_complete", &now.to_rfc3339())
                    .await?;
            }
            tx.query("UPDATE $row SET activation+=1,pages+=1,last_error='' RETURN NONE; UPDATE $job SET state=IF $pending {'running'} ELSE {'completed'},phase=IF $pending {'Published cleanup pending'} ELSE {'Published cleanup complete'},progress_done=IF $pending {0} ELSE {1},worker_owner='',finished_at=IF $pending {NONE} ELSE {<datetime>$now},updated_at=<datetime>$now RETURN NONE;")
                .bind(("row",jobs::rid("atlas_derivative_job",&task.job_id))).bind(("job",jobs::rid("job",&task.job_id))).bind(("pending",pending)).bind(("now",now.to_rfc3339())).await?.check()?;
            Ok(progress.map_or_else(Vec::new, |progress| progress.changed_memory_ids))
        })
    });
    crate::storage::memory::write_then_index(repo.publisher(), write).await?;
    Ok(())
}

async fn settle_superseded(
    owner: &AdmissionOwner,
    task: &ClaimedTask,
    row: &WorkRow,
) -> Result<bool, TxError> {
    let key = OccurrenceKey::new(&row.run_id, &row.article_id).record_id();
    let mut current = owner
        .query("RETURN (SELECT VALUE receipt_key FROM ONLY $projection)=$receipt;")
        .bind((
            "projection",
            jobs::rid("atlas_classification_projection", &key),
        ))
        .bind(("receipt", row.receipt_key.clone()))
        .await?
        .check()?;
    if current.take::<Option<bool>>(0)? == Some(true) {
        return Ok(false);
    }
    let (task, receipt) = (task.clone(), row.receipt_key.clone());
    admission::owned_tx(owner,RetryPolicy::default(),move|tx|{let(task,receipt,key)=(task.clone(),receipt.clone(),key.clone());async move {
        let now=Utc::now();jobs::bump(&tx,"job_guard",&task.root_id).await?;
        let Some(fence)=jobs::fence(&tx,&task,&now.to_rfc3339()).await? else {return Err(invalid("derivative_task_lost"));};
        if !jobs::holds(&fence,&task) || fence.cancelled {return Err(invalid("derivative_task_lost"));}
        let mut current=tx.query("RETURN (SELECT VALUE receipt_key FROM ONLY $projection)=$receipt;").bind(("projection",jobs::rid("atlas_classification_projection",&key))).bind(("receipt",receipt.clone())).await?.check()?;
        if current.take::<Option<bool>>(0)?==Some(true) {return Ok(false);}
        jobs::complete_in(&tx,&task,&format!("superseded:{receipt}"),&now.to_rfc3339()).await?;
        tx.query("UPDATE $job SET state='completed',phase='Superseded source receipt',error_summary='',worker_owner='',finished_at=time::now(),updated_at=time::now() RETURN NONE; UPDATE $row SET last_error='superseded_receipt' RETURN NONE;").bind(("job",jobs::rid("job",&task.job_id))).bind(("row",jobs::rid("atlas_derivative_job",&task.job_id))).await?.check()?;
        Ok(true)
    }}).await.map(|value|value.value).map_err(|_|invalid("derivative_supersede_failed"))
}

async fn next_stage(
    owner: &AdmissionOwner,
    row: &WorkRow,
    task: &ClaimedTask,
) -> Result<Option<DerivativeStage>, TxError> {
    let occurrence = OccurrenceKey::new(&row.run_id, &row.article_id);
    let work = DerivativeWork::new(occurrence.clone(), row.receipt_key.clone());
    let fence = DerivativeFence::Job(Box::new(JobFence {
        task: task.clone(),
        occurrence,
        receipt_key: row.receipt_key.clone(),
    }));
    admission::owned_tx(owner, RetryPolicy::default(), move |tx| {
        let (fence, work) = (fence.clone(), work.clone());
        async move {
            let states = admission::derivative_states_in(&tx, &fence, &[work], Utc::now()).await?;
            for row in &states {
                for stage in &row.stages {
                    if stage.disposition == Some(admission::DerivativeDisposition::Current) {
                        crate::storage::atlas_derivative_status::audit_current_in(
                            &tx,
                            &row.work.occurrence,
                            &row.work.receipt_key,
                            stage.stage,
                        )
                        .await?;
                    }
                }
            }
            Ok(states
                .into_iter()
                .flat_map(|row| row.stages)
                .find(|state| {
                    !state
                        .disposition
                        .as_ref()
                        .is_some_and(admission::DerivativeDisposition::terminal)
                })
                .map(|state| state.stage))
        }
    })
    .await
    .map(|value| value.value)
    .map_err(|_| invalid("derivative_current_state_failed"))
}

/// Separate job controls never pause/cancel collection or another derivative job.
pub async fn control(
    repo: &Repository,
    id: &str,
    action: &str,
) -> anyhow::Result<serde_json::Value> {
    if !id.starts_with("atlas-derivative-")
        || id.len() > 256
        || !matches!(action, "status" | "pause" | "resume" | "cancel")
    {
        anyhow::bail!("invalid_derivative_job_control");
    }
    let id = id.to_owned();
    let action = action.to_owned();
    crate::storage::tx::run_tx(repo.db(),RetryPolicy::default(),move|tx| {
        let(id,action)=(id.clone(),action.clone());async move {
            jobs::bump(&tx,"job_guard",&id).await?;
            let mut reply=tx.query("RETURN (SELECT VALUE operation FROM ONLY $job)=$operation;").bind(("job",jobs::rid("job",&id))).bind(("operation",OPERATION)).await?.check()?;
            if reply.take::<Option<bool>>(0)?!=Some(true) {return Err(invalid("derivative_job_not_found"));}
            if action=="resume" {
                let mut reply=tx.query("RETURN (SELECT VALUE cancel_requested FROM ONLY $job) ?? false;").bind(("job",jobs::rid("job",&id))).await?.check()?;
                if reply.take::<Option<bool>>(0)?==Some(true) {return Err(invalid("derivative_job_cancelled"));}
            }
            if action!="status" {
                tx.query("UPDATE $job SET stopped=$paused,cancel_requested=IF $cancel {true} ELSE {cancel_requested},state=IF $cancel {'cancelled'} ELSE IF state='completed' {'completed'} ELSE IF $paused {'paused'} ELSE {'running'},updated_at=time::now() WHERE state!='completed' RETURN NONE;")
                    .bind(("job",jobs::rid("job",&id))).bind(("paused",action=="pause")).bind(("cancel",action=="cancel")).await?.check()?;
                jobs::log_transition_in(&tx,&id,&format!("{action}:{}",Utc::now().timestamp_nanos_opt().unwrap_or_default()),"atlas.derivative.control",&action,&Utc::now().to_rfc3339()).await?;
            }
            let mut reply=tx.query("SELECT state,stopped,cancel_requested,phase,error_summary,progress_done,progress_total FROM ONLY $job; SELECT run_id,article_id,activation,pages,last_error FROM ONLY $row;")
                .bind(("job",jobs::rid("job",&id))).bind(("row",jobs::rid("atlas_derivative_job",&id))).await?.check()?;
            // Use bounded string JSON at the SDK boundary, not dynamic SDK values.
            #[derive(SurrealValue,Serialize)] struct Status {state:String,stopped:bool,cancel_requested:bool,phase:String,error_summary:String,progress_done:Option<i64>,progress_total:Option<i64>}
            #[derive(SurrealValue,Serialize)] struct Progress {run_id:String,article_id:String,activation:i64,pages:i64,last_error:String}
            let status:Option<Status>=reply.take(0)?;let progress:Option<Progress>=reply.take(1)?;
            Ok(serde_json::json!({"schema_version":"atlas-derivative-status-v1","job_id":id,"status":status,"progress":progress}))
        }
    }).await.map(|value|value.value).map_err(Into::into)
}
