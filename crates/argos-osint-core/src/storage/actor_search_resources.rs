//! Deployment-wide source backpressure and bounded retired-event collection.

use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use chrono::DateTime;
use chrono::Duration;
use chrono::Utc;
use fleetworks_contracts::actor_index::ActorIndexLimits;
use fleetworks_contracts::actor_index::ActorIndexStatus;

use super::control_in;
use super::guard;
use super::instant;
use super::invalid;
use super::jobs;
use super::run_tx;
use super::Lease;
use super::Repository;
use super::RetryPolicy;
use super::Tx;
use super::TxError;

/// The elected coordinator publishes measured service evidence once per status
/// observation. A source producer cannot multiply an index budget by workspace.
pub async fn observe_resources(
    repo: &Repository,
    lease: &Lease,
    status: &ActorIndexStatus,
    now: DateTime<Utc>,
) -> Result<(), TxError> {
    status
        .limits
        .validate()
        .map_err(|_| invalid("actor_index_limits_invalid"))?;
    let limits = serde_json::to_string(&status.limits)
        .map_err(|_| invalid("actor_index_limits_encoding"))?;
    let pressure = status.pressure.as_ref();
    // A missing OS reading is explicit missing evidence, never guessed free
    // capacity. The service still makes the final write admission decision.
    let reason = match pressure {
        Some(value) if value.resident_bytes.is_some() && value.disk_free_bytes.is_some() => {
            if value.paused {
                value
                    .reason_code
                    .clone()
                    .unwrap_or_else(|| "resource_pressure".into())
            } else {
                String::new()
            }
        }
        _ => "resource_measurement_unavailable".into(),
    };
    let pressure_json = serde_json::to_string(&status.pressure)
        .map_err(|_| invalid("actor_index_pressure_encoding"))?;
    run_tx(repo.db(), RetryPolicy::default(), |tx| {
        let limits=limits.clone();let reason=reason.clone();let pressure_json=pressure_json.clone();
        async move {
            guard(&tx).await?;let control=control_in(&tx).await?;
            if control.lease_owner!=lease.owner||control.lease_epoch!=lease.epoch as i64||instant(&control.lease_until)?<=now {return Err(invalid("actor_index_lease_lost"));}
            if !control.deployment_id.is_empty()&&(control.deployment_id!=status.deployment_id||control.source_epoch!=status.source_epoch) {return Err(invalid("actor_index_source_identity_mismatch"));}
            jobs::run(&tx,tx.query("UPDATE actor_search_control:catalog SET effective_limits_json=$limits,pressure_json=$pressure,pressure_paused=$paused,pressure_reason=$reason,pressure_observed_at=$now RETURN NONE")
                .bind(("limits",limits)).bind(("pressure",pressure_json)).bind(("paused",!reason.is_empty())).bind(("reason",reason)).bind(("now",now.to_rfc3339()))).await?;
            Ok(())
        }
    }).await?;
    Ok(())
}

pub(super) fn limits(control: &super::Control) -> Result<ActorIndexLimits, TxError> {
    let limits = if control.effective_limits_json.is_empty() {
        ActorIndexLimits::default()
    } else {
        serde_json::from_str(&control.effective_limits_json)
            .map_err(|_| invalid("actor_index_limits_invalid"))?
    };
    limits
        .validate()
        .map_err(|_| invalid("actor_index_limits_invalid"))?;
    Ok(limits)
}

/// Wait only at a committed seed chunk boundary. Keep the existing source
/// ownership lease alive while pressure pauses work; cancellation remains real.
pub async fn await_bulk_capacity(
    repo: &Repository,
    owner: &str,
    cancel: &AtomicBool,
) -> Result<(), TxError> {
    await_bulk_capacity_in(repo, owner, cancel, None).await
}

pub(crate) async fn await_bulk_capacity_claimed(
    repo: &Repository,
    owner: &str,
    cancel: &AtomicBool,
    execution: &crate::storage::actor_catalog::SeedExecution,
) -> Result<(), TxError> {
    await_bulk_capacity_in(repo, owner, cancel, Some(execution)).await
}

async fn await_bulk_capacity_in(
    repo: &Repository,
    owner: &str,
    cancel: &AtomicBool,
    execution: Option<&crate::storage::actor_catalog::SeedExecution>,
) -> Result<(), TxError> {
    loop {
        if cancel.load(Ordering::Acquire) {
            return Err(invalid("actor_seed_cancelled"));
        }
        let now = Utc::now();
        let waiting=run_tx(repo.db(),RetryPolicy::default(),|tx|async move {
            if let Some(execution)=execution {execution.check_in(&tx,cancel).await?;}
            guard(&tx).await?;let control=control_in(&tx).await?;let limits=limits(&control)?;
            let backlog=(control.next_sequence-control.acknowledged_sequence) as u64;
            let backlog_paused=if control.backlog_paused {backlog>limits.backlog_resume_events} else {backlog>=limits.backlog_pause_events};
            let stale=!control.pressure_observed_at.is_empty()&&(now-instant(&control.pressure_observed_at)?).num_seconds()>30;
            let reason=if backlog_paused {"actor_index_backlog"} else if stale {"actor_index_pressure_stale"} else if control.pressure_paused {&control.pressure_reason} else {""};
            jobs::run(&tx,tx.query("IF (SELECT VALUE owner FROM ONLY brain_actor_seed:catalog)!=$owner OR (SELECT VALUE status FROM ONLY brain_actor_seed:catalog)!='running' {THROW 'actor seed lease changed';};
                UPDATE brain_actor_seed:catalog SET index_pause_reason=$reason,lease_until=$until,updated_at=$now RETURN NONE;
                UPDATE actor_search_control:catalog SET backlog_paused=$backlog RETURN NONE")
                .bind(("owner",owner.to_owned())).bind(("reason",reason.to_owned())).bind(("until",(now+Duration::minutes(5)).to_rfc3339())).bind(("now",now.to_rfc3339())).bind(("backlog",backlog_paused))).await?;
            if let Some(execution)=execution {execution.check_in(&tx,cancel).await?;}
            Ok(!reason.is_empty())
        }).await?.value;
        if !waiting {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
}

/// A shadow promotion may acknowledge millions of rows at once. Delete at most
/// one bounded keyset page per coordinator turn, never an unbounded range.
pub(super) async fn prune_in(tx: &Tx, through: i64) -> Result<(), TxError> {
    jobs::run(tx,tx.query("LET $expired=(SELECT VALUE id FROM actor_search_event WITH INDEX actor_search_event_sequence WHERE sequence<=$through ORDER BY sequence LIMIT 500); DELETE $expired RETURN NONE;")
        .bind(("through",through))).await?;
    Ok(())
}
