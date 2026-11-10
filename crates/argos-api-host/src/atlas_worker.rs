//! RabbitMQ is a wake-up transport. SurrealDB owns dispatch revisions,
//! execution leases and terminal outcomes.

use std::future::Future;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, ensure, Result};
use argos_osint_core::atlas::{self, LiveRun, Stop};
use argos_osint_core::classifier::readiness::CyclePreflightError;
use argos_osint_core::classifier::ClassifierError;
use argos_osint_core::osint::{effective_user_agent, ProviderKeys};
use argos_osint_core::provider::SettingsFile;
use argos_osint_core::storage::classifier_admission::AdmissionError;
use argos_osint_core::storage::job_dispatch::{
    self, DeliveryClaim, DeliveryFinish, DeliveryLease, DeliveryOutcome, Envelope, ATLAS_POOL,
};
use argos_osint_core::storage::repo::Repository;
use chrono::Utc;
use fleetworks_broker::{Broker, Disposition, Received};
use tokio::time::{interval, sleep, timeout, MissedTickBehavior};

const EXECUTION_LEASE_SECS: i64 = 45;
const LEASE_HEARTBEAT: Duration = Duration::from_secs(5);
const RECONNECT_DELAY: Duration = Duration::from_secs(2);
const PAUSE_LIMIT: Duration = Duration::from_secs(10);

pub(crate) async fn run(repo: Repository, signal: impl Future<Output = ()>) -> Result<()> {
    let endpoint = crate::fleetworks::read_secret("FLEETWORKS_BROKER_URL")?;
    let queue =
        std::env::var("FLEETWORKS_BROKER_QUEUE").unwrap_or_else(|_| "fleetworks.atlas".into());
    let owner = format!("atlas-worker-{}", argos_osint_core::store::new_id());
    tokio::pin!(signal);
    loop {
        let broker = tokio::select! {
            _ = &mut signal => return Ok(()),
            result = Broker::connect(&endpoint, &queue) => result,
        };
        let result = match broker {
            Ok(broker) => consume(&repo, &broker, &owner, signal.as_mut()).await,
            Err(error) => Err(error.into()),
        };
        match result {
            Ok(()) => return Ok(()),
            Err(error) => tracing::warn!(
                error = %argos_osint_core::provider_diag::sanitize(&error.to_string()),
                "Atlas broker session interrupted"
            ),
        }
        tokio::select! {
            _ = &mut signal => return Ok(()),
            _ = sleep(RECONNECT_DELAY) => {}
        }
    }
}

async fn consume(
    repo: &Repository,
    broker: &Broker,
    owner: &str,
    mut signal: std::pin::Pin<&mut impl Future<Output = ()>>,
) -> Result<()> {
    let mut consumer = broker.consumer(1).await?;
    let mut relay = interval(Duration::from_secs(1));
    relay.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            biased;
            _ = &mut signal => return Ok(()),
            _ = relay.tick() => {
                let now = Utc::now();
                job_dispatch::recover_expired(repo.db(), ATLAS_POOL, now).await?;
                if let Some(claim) = job_dispatch::claim_ready(repo.db(), ATLAS_POOL, owner, now).await? {
                    broker.publish(&serde_json::to_vec(&claim.envelope)?).await?;
                    job_dispatch::mark_published(repo.db(), &claim, Utc::now()).await?;
                }
            }
            delivery = consumer.next() => {
                let received = delivery?.ok_or_else(|| anyhow!("Atlas subscription ended"))?;
                if handle(repo, owner, received, signal.as_mut()).await? {
                    return Ok(());
                }
            }
        }
    }
}

async fn handle(
    repo: &Repository,
    owner: &str,
    received: Received,
    mut signal: std::pin::Pin<&mut impl Future<Output = ()>>,
) -> Result<bool> {
    let envelope = match Envelope::decode(received.payload()) {
        Ok(envelope) if envelope.pool == ATLAS_POOL => envelope,
        _ => {
            received.settle(Disposition::DeadLetter).await?;
            return Ok(false);
        }
    };
    let op_key = argos_osint_core::store::new_id();
    let claim = job_dispatch::claim_delivery(
        repo.db(),
        &envelope,
        owner,
        EXECUTION_LEASE_SECS,
        Utc::now(),
        &op_key,
    )
    .await?;
    let lease = match claim {
        DeliveryClaim::Stale => {
            received.settle(Disposition::Ack).await?;
            return Ok(false);
        }
        DeliveryClaim::Deferred => {
            let stopping = tokio::select! {
                _ = &mut signal => true,
                _ = sleep(RECONNECT_DELAY) => false,
            };
            received.settle(Disposition::Requeue).await?;
            return Ok(stopping);
        }
        DeliveryClaim::Claimed(lease) => lease,
    };
    let pause = Arc::new(AtomicBool::new(false));
    let cycle = run_cycle(repo, &lease, &pause);
    tokio::pin!(cycle);
    let mut heartbeat = interval(LEASE_HEARTBEAT);
    heartbeat.set_missed_tick_behavior(MissedTickBehavior::Delay);
    heartbeat.tick().await;
    let (outcome, stopping) = loop {
        tokio::select! {
            biased;
            _ = &mut signal => {
                argos_osint_core::recon::cancel_with_notify(&pause);
                let _ = timeout(PAUSE_LIMIT, &mut cycle).await;
                break (retry("atlas_worker_shutdown"), true);
            }
            _ = heartbeat.tick() => {
                let renewal = job_dispatch::renew_delivery(
                    repo.db(), &lease, EXECUTION_LEASE_SECS, Utc::now(),
                ).await;
                if !matches!(renewal, Ok(true)) {
                    argos_osint_core::recon::cancel_with_notify(&pause);
                    let _ = timeout(PAUSE_LIMIT, &mut cycle).await;
                    renewal?;
                    // A cancelled or replaced lease cannot settle domain output.
                    received.settle(Disposition::Ack).await?;
                    return Ok(false);
                }
            }
            result = &mut cycle => break (outcome(&lease, result), false),
        }
    };
    let disposition =
        match job_dispatch::finish_delivery(repo.db(), &lease, outcome, Utc::now()).await? {
            DeliveryFinish::Completed | DeliveryFinish::Requeued | DeliveryFinish::Stale => {
                Disposition::Ack
            }
            DeliveryFinish::Failed => Disposition::DeadLetter,
        };
    received.settle(disposition).await?;
    Ok(stopping)
}

enum CycleResult {
    Finished,
    Retry(&'static str),
    Failed(&'static str),
}

async fn run_cycle(
    repo: &Repository,
    lease: &DeliveryLease,
    pause: &AtomicBool,
) -> Result<CycleResult> {
    ensure!(
        lease.task.operation == "atlas_cycle",
        "unexpected Atlas dispatch operation"
    );
    let settings = tokio::task::spawn_blocking(SettingsFile::load).await??;
    let stop = atlas::run_live(
        &argos_osint_core::paths::db_path(),
        pause,
        &ProviderKeys::from_settings(&settings),
        effective_user_agent(Some(&settings.osint_user_agent)),
        LiveRun::Dispatch(lease.task.input_hash.clone()),
        &[],
        None,
        None,
        &[],
        |_| {},
    )
    .await?;
    Ok(match stop {
        Stop::Paused => CycleResult::Retry("atlas_cycle_paused"),
        Stop::Failed(_) => CycleResult::Failed("atlas_cycle_failed"),
        Stop::Finished => match atlas::dispatch_state(repo, &lease.task.input_hash)
            .await?
            .as_deref()
        {
            Some("completed") => CycleResult::Finished,
            Some("waiting" | "partial" | "paused") => CycleResult::Retry("atlas_cycle_incomplete"),
            Some("blocked") => CycleResult::Failed("atlas_cycle_blocked"),
            Some("failed") => CycleResult::Failed("atlas_cycle_failed"),
            _ => CycleResult::Retry("atlas_checkpoint_unavailable"),
        },
    })
}

fn retry(reason: &'static str) -> DeliveryOutcome<'static> {
    DeliveryOutcome::Retry {
        reason,
        delay_secs: 15,
    }
}

fn outcome(lease: &DeliveryLease, result: Result<CycleResult>) -> DeliveryOutcome<'_> {
    match result {
        Ok(CycleResult::Finished) => DeliveryOutcome::Complete {
            result_ref: &lease.task.input_hash,
        },
        Ok(CycleResult::Retry(reason)) => retry(reason),
        Ok(CycleResult::Failed(reason)) => DeliveryOutcome::Failed { reason },
        Err(error) => match error.downcast_ref::<CyclePreflightError>() {
            Some(CyclePreflightError::Classifier {
                source: ClassifierError::Busy,
                ..
            }) => retry("atlas_classifier_busy"),
            Some(
                CyclePreflightError::Classifier { .. } | CyclePreflightError::MissingPinnedRevision,
            ) => DeliveryOutcome::Failed {
                reason: "atlas_classifier_unavailable",
            },
            Some(_) => DeliveryOutcome::Failed {
                reason: "atlas_checkpoint_invalid",
            },
            None if matches!(
                error.downcast_ref::<AdmissionError>(),
                Some(AdmissionError::LeaseLost)
            ) =>
            {
                retry("atlas_cycle_lease_busy")
            }
            None => {
                tracing::warn!(
                    error = %argos_osint_core::provider_diag::sanitize(&error.to_string()),
                    job_id = %lease.task.job_id,
                    "Atlas cycle failed"
                );
                retry("atlas_cycle_error")
            }
        },
    }
}
