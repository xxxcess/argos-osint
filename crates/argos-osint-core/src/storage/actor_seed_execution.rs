//! Opaque execution identity for attributed seed source transactions.
//!
//! Included by actor_catalog after the integration owner releases its graph.
//! This type is never serialized or deserialized, and carries no client origin.

use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use chrono::Utc;
use fleetworks_contracts::readiness::DatasetKind;
use fleetworks_contracts::readiness::MaintenanceOperation;

use crate::storage::jobs;
use crate::storage::jobs::ClaimedTask;
use crate::storage::repo::Repository;
use crate::storage::setup_jobs;
use crate::storage::tx::run_tx;
use crate::storage::tx::RetryPolicy;
use crate::storage::tx::Tx;
use crate::storage::tx::TxError;

/// The actual leased seed task and its admitted immutable dataset identity.
/// Private fields prevent request DTOs from constructing execution authority.
#[derive(Clone)]
pub struct SeedExecution {
    task: ClaimedTask,
    fingerprint: String,
}

fn invalid(code: &str) -> TxError {
    TxError::Abort(code.to_owned())
}

impl SeedExecution {
    /// Verify an actual Jobs claim before acquiring the separate source lease.
    /// Every later source transaction must still call `check_in`.
    pub async fn from_claimed(
        repo: &Repository,
        task: &ClaimedTask,
        cancel: &AtomicBool,
    ) -> Result<Self, TxError> {
        if task.pool != setup_jobs::SEED_POOL
            || task.operation != setup_jobs::SEED_POOL
            || task.state != "running"
        {
            return Err(invalid("actor_seed_execution_contract"));
        }
        let payload = setup_jobs::payload(repo, &task.job_id, &task.input_hash).await?;
        if payload.operation != MaintenanceOperation::SeedActors {
            return Err(invalid("actor_seed_execution_contract"));
        }
        let dataset = payload
            .dataset
            .filter(|dataset| dataset.kind == DatasetKind::ActorCatalog)
            .ok_or_else(|| invalid("actor_seed_execution_dataset_missing"))?;
        let execution = Self {
            task: task.clone(),
            fingerprint: dataset.sha256,
        };
        run_tx(repo.db(), RetryPolicy::default(), |tx| {
            let execution = &execution;
            async move { execution.check_in(&tx, cancel).await }
        })
        .await?;
        Ok(execution)
    }

    pub(super) fn parent_job(&self) -> &str {
        &self.task.job_id
    }

    pub(super) fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    /// Serialize with cancellation/retry/retirement on the genuine parent guard
    /// and re-read current task owner, epoch, running state and lease expiry.
    /// Call before source writes and again before returning their transaction.
    pub(crate) async fn check_in(&self, tx: &Tx, cancel: &AtomicBool) -> Result<(), TxError> {
        if cancel.load(Ordering::Acquire) {
            return Err(invalid("actor_seed_execution_cancelled"));
        }
        jobs::bump(tx, "job_guard", &self.task.root_id).await?;
        let now = Utc::now().to_rfc3339();
        let fence = jobs::fence(tx, &self.task, &now).await?;
        if !fence.is_some_and(|fence| !fence.cancelled && jobs::holds(&fence, &self.task)) {
            return Err(invalid("actor_seed_execution_lease_lost"));
        }
        let mut response = jobs::run(
            tx,
            tx.query("RETURN (SELECT VALUE (state IN ['queued','running','reconciling'] AND !stopped AND !cancel_requested) FROM ONLY $job) ?? false;
                RETURN (SELECT VALUE (record::id(job)=$job_id AND record::id(root)=$root_id AND pool=$pool AND operation=$operation AND input_hash=$input
                    AND !record::exists(type::record('job_retirement',record::id(root)))) FROM ONLY $task) ?? false;")
                .bind(("job", jobs::rid("job", &self.task.job_id)))
                .bind(("job_id", self.task.job_id.clone()))
                .bind(("root_id", self.task.root_id.clone()))
                .bind(("pool", self.task.pool.clone()))
                .bind(("operation", self.task.operation.clone()))
                .bind(("input", self.task.input_hash.clone()))
                .bind(("task", jobs::rid("task", &self.task.id))),
        )
        .await?;
        if jobs::take::<Option<bool>>(&mut response, 0)? != Some(true)
            || jobs::take::<Option<bool>>(&mut response, 1)? != Some(true)
            || cancel.load(Ordering::Acquire)
        {
            return Err(invalid("actor_seed_execution_cancelled"));
        }
        Ok(())
    }
}
