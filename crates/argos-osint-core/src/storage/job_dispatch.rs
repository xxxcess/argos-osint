//! Transactional AMQP outbox and exact delivery leases for opted-in job pools.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use surrealdb::engine::any::Any;
use surrealdb::types::{Datetime, RecordId, SurrealValue};
use surrealdb::Surreal;

use super::jobs::{self, ClaimedTask, NewJob, NewTask};
use super::tx::{run_tx, RetryPolicy, Tx, TxError};

/// Apply after the job schema and storage metadata.
pub const SCHEMA: &str = r#"
DEFINE TABLE IF NOT EXISTS job_dispatch_pool SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS enabled ON job_dispatch_pool TYPE bool;
DEFINE FIELD IF NOT EXISTS transport ON job_dispatch_pool TYPE option<string>;
DEFINE FIELD IF NOT EXISTS transport_revision ON job_dispatch_pool TYPE int DEFAULT 0;
DEFINE TABLE IF NOT EXISTS job_dispatch_outbox SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS generation ON job_dispatch_outbox TYPE string;
DEFINE FIELD IF NOT EXISTS task ON job_dispatch_outbox TYPE record<task>;
DEFINE FIELD IF NOT EXISTS job ON job_dispatch_outbox TYPE record<job>;
DEFINE FIELD IF NOT EXISTS pool ON job_dispatch_outbox TYPE string;
DEFINE FIELD IF NOT EXISTS revision ON job_dispatch_outbox TYPE int;
DEFINE FIELD IF NOT EXISTS created_at ON job_dispatch_outbox TYPE datetime;
DEFINE FIELD IF NOT EXISTS published_at ON job_dispatch_outbox TYPE option<datetime>;
DEFINE FIELD IF NOT EXISTS lease_owner ON job_dispatch_outbox TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS lease_epoch ON job_dispatch_outbox TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS lease_until ON job_dispatch_outbox TYPE option<datetime>;
DEFINE INDEX IF NOT EXISTS job_dispatch_ready ON job_dispatch_outbox FIELDS pool, generation, published_at, created_at;
DEFINE INDEX IF NOT EXISTS job_dispatch_task ON job_dispatch_outbox FIELDS task;
DEFINE EVENT IF NOT EXISTS job_dispatch_cleanup ON task WHEN $event='DELETE' THEN {
    DELETE job_dispatch_outbox WHERE task=$before.id RETURN NONE;
};
DEFINE EVENT IF NOT EXISTS job_dispatch_queued ON task
WHEN $event IN ['CREATE','UPDATE'] AND $after.state = 'queued'
    AND ($event = 'CREATE' OR $before.state != 'queued' OR $before.lease_epoch != $after.lease_epoch)
    AND record::exists(type::record('job_dispatch_pool', $after.pool))
THEN {
    LET $generation = (SELECT VALUE instance_id FROM ONLY storage_meta:identity);
    CREATE type::record('job_dispatch_outbox', [$generation, record::id($after.id), $after.lease_epoch])
        SET generation=$generation, task=$after.id, job=$after.job, pool=$after.pool,
            revision=$after.lease_epoch, created_at=$after.updated_at RETURN NONE;
};
"#;

/// Broker pool for collecting Atlas cycles.
pub const ATLAS_POOL: &str = "atlas_cycle";
const VERSION: u16 = 1;
const MAX_MESSAGE_BYTES: usize = 16 * 1024;
const RELAY_LEASE_SECS: i64 = 30;

/// Durable dispatch identity. Article content and credentials remain in the store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Envelope {
    pub version: u16,
    pub workspace_generation: String,
    pub job_id: String,
    pub task_id: String,
    pub pool: String,
    pub revision: i64,
    pub correlation_id: String,
}

impl Envelope {
    /// Decode a bounded, versioned broker delivery.
    pub fn decode(bytes: &[u8]) -> Result<Self, TxError> {
        if bytes.is_empty() || bytes.len() > MAX_MESSAGE_BYTES {
            return Err(invalid("invalid_dispatch_size"));
        }
        let envelope: Self =
            serde_json::from_slice(bytes).map_err(|_| invalid("invalid_dispatch_json"))?;
        envelope.validate()?;
        Ok(envelope)
    }

    fn validate(&self) -> Result<(), TxError> {
        if self.version != VERSION
            || self.revision < 0
            || !valid_pool(&self.pool)
            || [
                &self.workspace_generation,
                &self.job_id,
                &self.task_id,
                &self.correlation_id,
            ]
            .iter()
            .any(|id| !valid_identity(id))
        {
            return Err(invalid("invalid_dispatch_identity"));
        }
        Ok(())
    }
}

fn valid_identity(value: &str) -> bool {
    (1..=256).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':'))
}

fn valid_pool(value: &str) -> bool {
    (1..=64).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

fn invalid(code: &str) -> TxError {
    TxError::Abort(code.to_owned())
}

/// Opt a pool into transactional dispatch after bootstrap establishes the
/// database identity. Legacy pools retain their existing claim paths.
pub async fn enable_pool(db: &Surreal<Any>, pool: &str) -> Result<(), TxError> {
    configure_pool(db, pool, true).await
}

/// Records the embedded queue transport only after the previous transport has
/// drained. Published Rabbit deliveries are not visible to the new queue, so
/// switching while any task remains unfinished would strand work.
pub async fn ensure_queue_transport(db: &Surreal<Any>, pool: &str) -> Result<(), TxError> {
    if !valid_pool(pool) {
        return Err(invalid("invalid_dispatch_pool"));
    }
    run_tx(db, RetryPolicy::default(), |tx| async move {
        jobs::run(&tx, tx.query("UPSERT $p SET enabled=enabled ?? false, transport_revision=(transport_revision ?? 0)+1 RETURN NONE")
            .bind(("p", jobs::rid("job_dispatch_pool", pool)))).await?;
        let mut reply = jobs::run(&tx, tx.query(
            "RETURN (SELECT VALUE transport FROM ONLY $p);
             RETURN count(SELECT VALUE id FROM task WHERE pool=$pool
               AND (state NOT IN ['completed','failed','cancelled','superseded']
                    OR job.state NOT IN ['completed','succeeded','failed','partial','cancelled','paused','superseded']) LIMIT 1)>0;"
        ).bind(("p", jobs::rid("job_dispatch_pool", pool))).bind(("pool", pool.to_owned()))).await?;
        let transport: Option<String> = jobs::take(&mut reply, 0)?;
        match transport.as_deref() {
            Some("jobs_rocksdb_v1") => return Ok(()),
            Some(_) => return Err(invalid("unsupported_dispatch_transport")),
            None => {}
        }
        if jobs::take::<Option<bool>>(&mut reply, 1)? == Some(true) {
            return Err(invalid("drain_existing_atlas_jobs_before_queue_cutover"));
        }
        jobs::run(&tx, tx.query("UPDATE $p SET transport='jobs_rocksdb_v1' RETURN NONE")
            .bind(("p", jobs::rid("job_dispatch_pool", pool)))).await?;
        Ok(())
    }).await?;
    Ok(())
}

/// Allow or pause new submissions, retries and relay claims for this pool.
/// Existing outbox rows remain durable while dispatch is paused.
pub async fn configure_pool(db: &Surreal<Any>, pool: &str, enabled: bool) -> Result<(), TxError> {
    if !valid_pool(pool) {
        return Err(invalid("invalid_dispatch_pool"));
    }
    run_tx(db, RetryPolicy::default(), |tx| async move {
        let mut reply = jobs::run(
            &tx,
            tx.query("SELECT VALUE instance_id FROM ONLY storage_meta:identity"),
        )
        .await?;
        let generation: Option<String> = jobs::take(&mut reply, 0)?;
        if !generation.as_deref().is_some_and(valid_identity) {
            return Err(invalid("dispatch_generation_missing"));
        }
        jobs::run(
            &tx,
            tx.query("UPSERT $pool SET enabled=$enabled RETURN NONE")
                .bind(("pool", jobs::rid("job_dispatch_pool", pool)))
                .bind(("enabled", enabled)),
        )
        .await?;
        Ok(())
    })
    .await?;
    Ok(())
}

/// Atomically create the job, task and outbox revision. Repeating the same
/// input returns false and keeps the first submission's state and timestamps.
pub async fn submit(
    db: &Surreal<Any>,
    job: &NewJob<'_>,
    task: &NewTask<'_>,
) -> Result<bool, TxError> {
    if task.job != job.id
        || !valid_identity(job.id)
        || !valid_identity(task.id)
        || !valid_pool(task.pool)
        || job.parent.is_some_and(|id| !valid_identity(id))
        || task.deps.iter().any(|id| !valid_identity(id))
    {
        return Err(invalid("invalid_dispatch_submission"));
    }
    Ok(run_tx(db, RetryPolicy::default(), |tx| async move {
        let job_key = jobs::rid("job", job.id);
        let mut reply = jobs::run(
            &tx,
            tx.query(
                "LET $existing = (SELECT * FROM ONLY $job);
                 LET $queued = (SELECT * FROM ONLY $task);
                 RETURN (SELECT VALUE enabled FROM ONLY $pool)=true;
                 RETURN $existing!=NONE OR $queued!=NONE;
                 RETURN $existing.operation=$job_op AND $existing.kind=$kind
                    AND $existing.parent=$parent AND $queued.job=$job
                    AND $queued.pool=$pool_name AND $queued.operation=$task_op
                    AND $queued.input_hash=$input AND $queued.priority=$priority AND $queued.deps=$deps;
                 RETURN record::exists($retired);",
            )
            .bind(("job", job_key.clone()))
            .bind(("task", jobs::rid("task", task.id)))
            .bind(("pool", jobs::rid("job_dispatch_pool", task.pool)))
            .bind(("job_op", job.operation.to_owned()))
            .bind(("kind", job.kind.to_owned()))
            .bind(("parent", job.parent.map(|id| jobs::rid("job", id))))
            .bind(("pool_name", task.pool.to_owned()))
            .bind(("task_op", task.operation.to_owned()))
            .bind(("input", task.input_hash.to_owned()))
            .bind(("priority", task.priority))
            .bind(("deps", task.deps.iter().map(|id| jobs::rid("task", id)).collect::<Vec<_>>()))
            .bind(("retired", jobs::rid("job_retirement", job.id))),
        )
        .await?;
        if jobs::take::<Option<bool>>(&mut reply, 2)? != Some(true) {
            return Err(invalid("dispatch_pool_disabled"));
        }
        if jobs::take::<Option<bool>>(&mut reply, 3)? == Some(true) {
            return if jobs::take::<Option<bool>>(&mut reply, 4)? == Some(true) {
                Ok(false)
            } else {
                Err(invalid("dispatch_submission_conflict"))
            };
        }
        if jobs::take::<Option<bool>>(&mut reply, 5)? == Some(true) {
            return Err(invalid("dispatch_submission_retired"));
        }
        jobs::create_job_in(&tx, job).await?;
        jobs::create_task_in(&tx, task).await?;
        jobs::run(
            &tx,
            tx.query("UPDATE $job SET state='queued', phase='Queued', cancellable=true RETURN NONE")
                .bind(("job", job_key)),
        )
        .await?;
        // Reuse the original event key from create_job_in to publish the
        // queued state without retaining a premature started event.
        jobs::log_transition_in(
            &tx,
            job.id,
            &format!("{}:started", job.id),
            "job.queued",
            &format!("Queued: {}", jobs::job_title(job.operation)),
            &jobs::ts(job.created_at),
        )
        .await?;
        Ok(true)
    })
    .await?
    .value)
}

#[derive(SurrealValue)]
struct OutboxRow {
    key: RecordId,
    generation: String,
    job_id: String,
    task_id: String,
    pool: String,
    revision: i64,
    correlation_id: String,
    lease_epoch: i64,
    #[surreal(rename = "created_at")]
    _created_at: Datetime,
}

/// Lease for one publish. The same envelope can be re-published after an
/// uncertain confirmation. Only the task claim grants execution authority.
#[derive(Debug)]
pub struct DispatchClaim {
    pub envelope: Envelope,
    key: RecordId,
    owner: String,
    lease_epoch: i64,
}

/// Lease the oldest eligible unpublished entry for this pool.
pub async fn claim_ready(
    db: &Surreal<Any>,
    pool: &str,
    owner: &str,
    now: DateTime<Utc>,
) -> Result<Option<DispatchClaim>, TxError> {
    if !valid_pool(pool) || !valid_identity(owner) {
        return Err(invalid("invalid_dispatch_relay"));
    }
    Ok(run_tx(db, RetryPolicy::default(), |tx| async move {
        let mut reply = jobs::run(
            &tx,
            tx.query(
                "LET $generation=(SELECT VALUE instance_id FROM ONLY storage_meta:identity);
                 LET $enabled=(SELECT VALUE enabled FROM ONLY $pool_key);
                 SELECT id AS key, generation, record::id(job) AS job_id,
                     record::id(task) AS task_id, pool, revision,
                     (IF job.correlation_id='' {record::id(job)} ELSE {job.correlation_id}) AS correlation_id,
                     lease_epoch, created_at
                 FROM job_dispatch_outbox
                 WHERE $enabled=true AND pool=$pool AND generation=$generation AND published_at=NONE
                     AND (lease_until=NONE OR lease_until <= <datetime>$now)
                     AND task.state='queued' AND task.lease_epoch=revision
                     AND task.next_eligible_at <= <datetime>$now
                     AND job.state IN ['queued','running','reconciling'] AND !job.stopped
                     AND job.created_at >= <datetime>$fresh_after
                     AND !job.cancel_requested AND !array::any(job.ancestors.cancel_requested)
                     AND !record::exists(type::record('job_retirement', record::id(job.root)))
                     AND array::len(task.deps)=array::len(task.deps.state)
                     AND array::all(task.deps.state,'completed')
                 ORDER BY created_at LIMIT 1;",
            )
            .bind(("pool", pool.to_owned()))
            .bind(("pool_key", jobs::rid("job_dispatch_pool", pool)))
            .bind(("now", jobs::ts(now)))
            .bind(("fresh_after", jobs::ts(now - Duration::hours(jobs::EXECUTION_WINDOW_HOURS)))),
        )
        .await?;
        let rows: Vec<OutboxRow> = jobs::take(&mut reply, 2)?;
        let Some(row) = rows.into_iter().next() else {
            return Ok(None);
        };
        let lease_epoch = row
            .lease_epoch
            .checked_add(1)
            .ok_or_else(|| invalid("dispatch_relay_epoch_overflow"))?;
        let envelope = Envelope {
            version: VERSION,
            workspace_generation: row.generation,
            job_id: row.job_id,
            task_id: row.task_id,
            pool: row.pool,
            revision: row.revision,
            correlation_id: row.correlation_id,
        };
        envelope.validate()?;
        jobs::run(
            &tx,
            tx.query("UPDATE $key SET lease_owner=$owner, lease_epoch=$epoch, lease_until=<datetime>$until RETURN NONE")
                .bind(("key", row.key.clone()))
                .bind(("owner", owner.to_owned()))
                .bind(("epoch", lease_epoch))
                .bind(("until", jobs::ts(now + Duration::seconds(RELAY_LEASE_SECS)))),
        )
        .await?;
        Ok(Some(DispatchClaim {
            envelope,
            key: row.key,
            owner: owner.to_owned(),
            lease_epoch,
        }))
    })
    .await?
    .value)
}

/// Record a mandatory persistent publish only after its broker confirmation.
/// A stale relay cannot mark a successor's lease.
pub async fn mark_published(
    db: &Surreal<Any>,
    claim: &DispatchClaim,
    now: DateTime<Utc>,
) -> Result<bool, TxError> {
    let mut reply = db
        .query(
            "UPDATE $key SET published_at=<datetime>$now, lease_until=NONE
             WHERE lease_epoch=$epoch AND lease_owner=$owner AND published_at=NONE
                 AND lease_until > <datetime>$now RETURN VALUE lease_epoch",
        )
        .bind(("key", claim.key.clone()))
        .bind(("epoch", claim.lease_epoch))
        .bind(("owner", claim.owner.clone()))
        .bind(("now", jobs::ts(now)))
        .await?
        .check()?;
    Ok(!reply.take::<Vec<i64>>(0)?.is_empty())
}

async fn registered_in(tx: &Tx, envelope: &Envelope) -> Result<bool, TxError> {
    let mut reply = jobs::run(
        tx,
        tx.query(
            "LET $generation=(SELECT VALUE instance_id FROM ONLY storage_meta:identity);
             LET $dispatch=(SELECT job,pool FROM ONLY
                 type::record('job_dispatch_outbox', [$generation,$task,$revision]));
             RETURN $generation=$expected AND $dispatch!=NONE
                 AND $dispatch.job=$job AND $dispatch.pool=$pool;",
        )
        .bind(("job", jobs::rid("job", &envelope.job_id)))
        .bind(("task", envelope.task_id.clone()))
        .bind(("pool", envelope.pool.clone()))
        .bind(("revision", envelope.revision))
        .bind(("expected", envelope.workspace_generation.clone())),
    )
    .await?;
    Ok(jobs::take::<Option<bool>>(&mut reply, 2)? == Some(true))
}

/// A delivery lease and the original database generation that granted it.
#[derive(Debug)]
pub struct DeliveryLease {
    pub task: ClaimedTask,
    envelope: Envelope,
}

/// Delivery disposition before any service work runs.
#[derive(Debug)]
pub enum DeliveryClaim {
    Claimed(Box<DeliveryLease>),
    /// No work remains for this revision. The consumer may acknowledge it.
    Stale,
    /// The same revision is still queued but not eligible. Requeue delivery.
    Deferred,
}

#[derive(SurrealValue)]
struct TaskRevision {
    job_id: String,
    pool: String,
    revision: i64,
    state: String,
    lease_owner: String,
    live: bool,
    due: bool,
    dependencies_complete: bool,
    cancelled: bool,
}

/// Claim exactly the advertised job, task, pool, generation and queued
/// revision. Reuse `op_key` only to resolve an uncertain result for this claim.
pub async fn claim_delivery(
    db: &Surreal<Any>,
    envelope: &Envelope,
    owner: &str,
    lease_secs: i64,
    now: DateTime<Utc>,
    op_key: &str,
) -> Result<DeliveryClaim, TxError> {
    envelope.validate()?;
    if !valid_identity(owner) || !valid_identity(op_key) || !(1..=300).contains(&lease_secs) {
        return Err(invalid("invalid_dispatch_claim"));
    }
    Ok(run_tx(db, RetryPolicy::default(), |tx| async move {
        if !registered_in(&tx, envelope).await? {
            return Ok(DeliveryClaim::Stale);
        }
        let at = jobs::ts(now);
        let mut reply = jobs::run(
            &tx,
            tx.query(
                "SELECT record::id(job) AS job_id, pool, lease_epoch AS revision, state, lease_owner,
                 (lease_until!=NONE AND lease_until > <datetime>$now) AS live,
                 (next_eligible_at <= <datetime>$now) AS due,
                 (array::len(deps)=array::len(deps.state)
                   AND array::all(deps.state,'completed')) AS dependencies_complete,
                 (job.cancel_requested OR job.stopped OR array::any(job.ancestors.cancel_requested)
                   OR job.state NOTINSIDE ['queued','running','reconciling']
                   OR job.created_at < <datetime>$fresh_after
                   OR record::exists(type::record('job_retirement', record::id(root)))) AS cancelled
                 FROM ONLY $task",
            )
            .bind(("task", jobs::rid("task", &envelope.task_id)))
            .bind(("now", at.clone()))
            .bind(("fresh_after", jobs::ts(now - Duration::hours(jobs::EXECUTION_WINDOW_HOURS)))),
        )
        .await?;
        let Some(row) = jobs::take::<Option<TaskRevision>>(&mut reply, 0)? else {
            return Ok(DeliveryClaim::Stale);
        };
        if row.job_id != envelope.job_id || row.pool != envelope.pool || row.cancelled {
            return Ok(DeliveryClaim::Stale);
        }
        let queued = row.state == "queued" && row.revision == envelope.revision;
        let same_lease = row.state == "running"
            && row.live
            && row.lease_owner == owner
            && row.revision.checked_sub(1) == Some(envelope.revision);
        if !queued && !same_lease {
            return Ok(DeliveryClaim::Stale);
        }
        if queued && (!row.due || !row.dependencies_complete) {
            return Ok(DeliveryClaim::Deferred);
        }
        let claim = jobs::claim_in(
            &tx,
            &envelope.pool,
            Some(&envelope.task_id),
            owner,
            lease_secs,
            now,
            op_key,
            None,
        )
        .await?;
        match claim {
            Some(task)
                if task.id == envelope.task_id
                    && task.job_id == envelope.job_id
                    && task.pool == envelope.pool
                    && task.lease_owner == owner
                    && task.state == "running"
                    && task.lease_epoch.checked_sub(1) == Some(envelope.revision) =>
            {
                jobs::run(
                    &tx,
                    tx.query(
                        "UPDATE $job SET state='running', phase='Running', updated_at=<datetime>$now
                         WHERE state='queued' RETURN NONE",
                    )
                    .bind(("job", jobs::rid("job", &task.job_id)))
                    .bind(("now", at)),
                )
                .await?;
                Ok(DeliveryClaim::Claimed(Box::new(DeliveryLease {
                    task,
                    envelope: envelope.clone(),
                })))
            }
            _ if queued => Ok(DeliveryClaim::Deferred),
            _ => Ok(DeliveryClaim::Stale),
        }
    })
    .await?
    .value)
}

async fn holds_delivery(tx: &Tx, lease: &DeliveryLease, now: &str) -> Result<bool, TxError> {
    let task = &lease.task;
    let envelope = &lease.envelope;
    if task.id != envelope.task_id
        || task.job_id != envelope.job_id
        || task.pool != envelope.pool
        || task.lease_epoch.checked_sub(1) != Some(envelope.revision)
        || !registered_in(tx, envelope).await?
    {
        return Ok(false);
    }
    let fence = jobs::fence(tx, task, now).await?;
    Ok(fence.is_some_and(|fence| !fence.cancelled && jobs::holds(&fence, task)))
}

/// Renew only a live, uncancelled lease in the original store generation.
pub async fn renew_delivery(
    db: &Surreal<Any>,
    lease: &DeliveryLease,
    lease_secs: i64,
    now: DateTime<Utc>,
) -> Result<bool, TxError> {
    if !(1..=300).contains(&lease_secs) {
        return Err(invalid("invalid_dispatch_lease"));
    }
    Ok(run_tx(db, RetryPolicy::default(), |tx| async move {
        if !holds_delivery(&tx, lease, &jobs::ts(now)).await? {
            return Ok(false);
        }
        jobs::renew_owned_in(&tx, &lease.task, lease_secs, now).await
    })
    .await?
    .value)
}

/// The safe result of service work. Failure reasons are machine-readable codes.
#[derive(Debug, Clone, Copy)]
pub enum DeliveryOutcome<'a> {
    Complete { result_ref: &'a str },
    Retry { reason: &'a str, delay_secs: i64 },
    Failed { reason: &'a str },
}

/// State durably recorded before the consumer acknowledges the delivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryFinish {
    Completed,
    Requeued,
    Failed,
    Stale,
}

/// Close the attempt and settle or requeue the job under the live fence.
/// Requeueing emits a new outbox revision in the same transaction.
pub async fn finish_delivery(
    db: &Surreal<Any>,
    lease: &DeliveryLease,
    outcome: DeliveryOutcome<'_>,
    now: DateTime<Utc>,
) -> Result<DeliveryFinish, TxError> {
    match outcome {
        DeliveryOutcome::Complete { result_ref } if !valid_identity(result_ref) => {
            return Err(invalid("invalid_dispatch_result"));
        }
        DeliveryOutcome::Retry { reason, delay_secs }
            if !valid_pool(reason) || !(0..=3600).contains(&delay_secs) =>
        {
            return Err(invalid("invalid_dispatch_retry"));
        }
        DeliveryOutcome::Failed { reason } if !valid_pool(reason) => {
            return Err(invalid("invalid_dispatch_failure"));
        }
        _ => {}
    }
    Ok(run_tx(db, RetryPolicy::default(), |tx| async move {
        let task = &lease.task;
        let at = jobs::ts(now);
        if !holds_delivery(&tx, lease, &at).await? {
            return Ok(DeliveryFinish::Stale);
        }
        match outcome {
            DeliveryOutcome::Complete { result_ref } => {
                jobs::complete_in(&tx, task, result_ref, &at).await?;
                jobs::settle_finished_job_in(&tx, &task.job_id).await?;
                jobs::run(
                    &tx,
                    tx.query(
                        "UPDATE $job SET result_ref=$result, phase='Completed', error_category='',
                         error_summary='' WHERE state='completed' RETURN NONE",
                    )
                    .bind(("job", jobs::rid("job", &task.job_id)))
                    .bind(("result", result_ref.to_owned())),
                )
                .await?;
                Ok(DeliveryFinish::Completed)
            }
            DeliveryOutcome::Retry { reason, delay_secs } => {
                finish_error(&tx, task, reason, now, Some(delay_secs)).await
            }
            DeliveryOutcome::Failed { reason } => finish_error(&tx, task, reason, now, None).await,
        }
    })
    .await?
    .value)
}

async fn finish_error(
    tx: &Tx,
    task: &ClaimedTask,
    reason: &str,
    now: DateTime<Utc>,
    retry_delay: Option<i64>,
) -> Result<DeliveryFinish, TxError> {
    let task_key = jobs::rid("task", &task.id);
    let job_key = jobs::rid("job", &task.job_id);
    let retry = if let Some(delay) = retry_delay {
        let mut reply = jobs::run(
            tx,
            tx.query("SELECT VALUE attempts < max_attempts FROM ONLY $task")
                .bind(("task", task_key.clone())),
        )
        .await?;
        (jobs::take::<Option<bool>>(&mut reply, 0)? == Some(true)).then_some(delay)
    } else {
        None
    };
    let requeued = retry.is_some();
    let at = jobs::ts(now);
    jobs::run(
        tx,
        tx.query(
            "UPDATE $task SET state=IF $retry {'queued'} ELSE {'failed'},
                 lease_epoch+=1, lease_owner='', process_owner='', lease_until=NONE,
                 finished_at=IF $retry {NONE} ELSE {<datetime>$now},
                 next_eligible_at=<datetime>$next, updated_at=<datetime>$now,
                 error_category='dispatch', error_message=$reason RETURN NONE;
             LET $open=SELECT VALUE id FROM attempt
                 WHERE task=$task AND epoch=$epoch AND finished_at=NONE;
             UPDATE $open SET finished_at=<datetime>$now,
                 duration_ms=duration::millis(<datetime>$now-started_at),
                 outcome=IF $retry {'retry_scheduled'} ELSE {'failed'},
                 error_category='dispatch', error_message=$reason RETURN NONE;",
        )
        .bind(("task", task_key))
        .bind(("epoch", task.lease_epoch))
        .bind(("reason", reason.to_owned()))
        .bind(("retry", requeued))
        .bind((
            "next",
            jobs::ts(now + Duration::seconds(retry.unwrap_or_default())),
        ))
        .bind(("now", at.clone())),
    )
    .await?;
    let (event, disposition) = if requeued {
        ("task.retry_scheduled", DeliveryFinish::Requeued)
    } else {
        jobs::settle_finished_job_in(tx, &task.job_id).await?;
        ("task.failed", DeliveryFinish::Failed)
    };
    // Keep the bounded reason after the generic terminal rollup.
    jobs::run(
        tx,
        tx.query(
            "UPDATE $job SET state=IF $retry {'queued'} ELSE {state},
             phase=IF $retry {'Retry scheduled'} ELSE {'Failed'},
             error_category='dispatch', error_summary=$reason, updated_at=<datetime>$now RETURN NONE",
        )
        .bind(("job", job_key))
        .bind(("retry", requeued))
        .bind(("reason", reason.to_owned()))
        .bind(("now", at.clone())),
    )
    .await?;
    jobs::log_transition_in(
        tx,
        &task.job_id,
        &format!("{}:{}:{event}", task.id, task.lease_epoch),
        event,
        reason,
        &at,
    )
    .await?;
    Ok(disposition)
}

/// Requeue one failed Atlas collection task without reusing an attempt number.
///
/// An explicit retry extends the cap only if all prior attempts used it.
/// The new queued epoch and outbox record commit with the reopened job.
pub async fn retry_failed_atlas(
    db: &Surreal<Any>,
    job_id: &str,
    task_id: &str,
    now: DateTime<Utc>,
) -> Result<bool, TxError> {
    if !valid_identity(job_id) || !valid_identity(task_id) {
        return Err(invalid("invalid_dispatch_retry"));
    }
    Ok(run_tx(db, RetryPolicy::default(), |tx| async move {
        let job = jobs::rid("job", job_id);
        let at = jobs::ts(now);
        let mut reply = jobs::run(
            &tx,
            tx.query(
                "LET $eligible=(SELECT VALUE id FROM ONLY $job
                    WHERE state='failed' AND operation=$pool_name AND parent=NONE
                    AND !cancel_requested AND !stopped AND created_at>=<datetime>$fresh_after
                    AND !record::exists($retired));
                 LET $enabled=(SELECT VALUE enabled FROM ONLY $pool);
                 UPDATE $task SET state='queued', lease_epoch+=1, recoveries=0,
                    max_attempts=math::max([max_attempts, attempts+1]),
                    finished_at=NONE, updated_at=<datetime>$now, next_eligible_at=<datetime>$now,
                    lease_owner='', process_owner='', lease_until=NONE,
                    error_category='', error_message=''
                 WHERE job=$job AND pool=$pool_name AND operation=$pool_name AND state='failed'
                    AND $eligible=$job AND $enabled=true RETURN VALUE lease_epoch;",
            )
            .bind(("job", job.clone()))
            .bind(("task", jobs::rid("task", task_id)))
            .bind(("pool", jobs::rid("job_dispatch_pool", ATLAS_POOL)))
            .bind(("pool_name", ATLAS_POOL))
            .bind((
                "fresh_after",
                jobs::ts(now - Duration::hours(jobs::EXECUTION_WINDOW_HOURS)),
            ))
            .bind(("retired", jobs::rid("job_retirement", job_id)))
            .bind(("now", at.clone())),
        )
        .await?;
        let revisions: Vec<i64> = jobs::take(&mut reply, 2)?;
        let Some(revision) = revisions.into_iter().next() else {
            return Ok(false);
        };
        jobs::bump(&tx, "job_guard", job_id).await?;
        jobs::run(
            &tx,
            tx.query(
                "UPDATE $job SET state='queued', phase='Queued for retry', finished_at=NONE,
                 error_category='', error_summary='', updated_at=<datetime>$now RETURN NONE",
            )
            .bind(("job", job))
            .bind(("now", at.clone())),
        )
        .await?;
        jobs::log_transition_in(
            &tx,
            job_id,
            &format!("{task_id}:{revision}:retry"),
            "job.retry",
            "Queued for retry",
            &at,
        )
        .await?;
        Ok(true)
    })
    .await?
    .value)
}

/// Recover one bounded page of expired task leases for a pool. The task
/// event makes each requeue's next revision visible to the relay.
pub async fn recover_expired(
    db: &Surreal<Any>,
    pool: &str,
    now: DateTime<Utc>,
) -> Result<usize, TxError> {
    if !valid_pool(pool) {
        return Err(invalid("invalid_dispatch_pool"));
    }
    Ok(run_tx(db, RetryPolicy::default(), |tx| async move {
        let recovered = jobs::interrupt_expired_pool_in(&tx, pool, now).await?;
        let mut reply = jobs::run(
            &tx,
            tx.query(
                "SELECT VALUE record::id(job) FROM task WHERE pool=$pool
                 AND state='failed' AND job.state IN ['queued','running','active'] LIMIT $limit",
            )
            .bind(("pool", pool.to_owned()))
            .bind(("limit", jobs::SWEEP_BATCH)),
        )
        .await?;
        for job_id in jobs::take::<Vec<String>>(&mut reply, 0)? {
            jobs::settle_finished_job_in(&tx, &job_id).await?;
        }
        Ok(recovered)
    })
    .await?
    .value)
}

#[cfg(test)]
mod tests;
