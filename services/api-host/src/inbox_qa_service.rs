//! Dedicated durable Jobs consumer for Inbox QA in either supported runtime.

use std::future::Future;
use std::time::Duration;

use anyhow::{anyhow, Result};
use argos_osint_api::inbox_qa::{CancellationToken, QaInference};
use argos_osint_core::storage::inbox_qa::{self as store, FrozenEvidence, POOL};
use argos_osint_core::storage::job_dispatch::{
    self, DeliveryClaim, DeliveryFinish, DeliveryLease, DeliveryOutcome, Envelope,
};
use argos_osint_core::storage::repo::Repository;
use chrono::Utc;
use fleetworks_contracts::chat::{QaFinding, QaTurnState};
use fleetworks_contracts::qa_worker::{QaWorkerEvent, QaWorkerOutcome, QaWorkerSequence};
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;
use tokio::time::{sleep, timeout, Instant};

use crate::qa_worker::QaWorkerSupervisor;
use crate::queue_client::{Client, Disposition, Received};

pub(crate) const QUEUE: &str = "fleetworks.qa";
const DELIVERY_SECS: i64 = 12;
const POLL: Duration = Duration::from_millis(250);
const CHECK: Duration = Duration::from_secs(2);
const AUTH_TIMEOUT: Duration = Duration::from_secs(2);
const WARM_TIMEOUT: Duration = Duration::from_secs(20);
const STOP_TIMEOUT: Duration = Duration::from_secs(8);
const TURN_TIMEOUT: Duration = Duration::from_secs(30);
const WARM_RETRY: Duration = Duration::from_secs(10);

pub(crate) async fn run_distributed(
    repo: Repository,
    signal: impl Future<Output = ()>,
) -> Result<()> {
    let Some(model) = QaWorkerSupervisor::distributed_from_environment()? else {
        signal.await;
        return Ok(());
    };
    let broker = tokio::task::spawn_blocking(|| {
        Client::new(
            &std::env::var("FLEETWORKS_JOBS_ORIGIN")?,
            &crate::fleetworks::read_secret("FLEETWORKS_JOBS_TOKEN")?,
            QUEUE.to_owned(),
        )
    })
    .await??;
    run(repo, broker, model, signal).await
}

struct Executor {
    repo: Repository,
    broker: Client,
    model: QaWorkerSupervisor,
    owner: String,
}

pub(crate) async fn run(
    repo: Repository,
    broker: Client,
    model: QaWorkerSupervisor,
    signal: impl Future<Output = ()>,
) -> Result<()> {
    let executor = Executor {
        repo,
        broker,
        model,
        owner: format!("assistant-qa-{}", argos_osint_core::store::new_id()),
    };
    let stop = CancellationToken::new();
    let session = executor.serve(&stop);
    tokio::pin!(session, signal);
    let result = tokio::select! {
        result = &mut session => result,
        () = &mut signal => {
            stop.cancel();
            session.await
        }
    };
    executor.unload().await;
    result
}

impl Executor {
    async fn serve(&self, stop: &CancellationToken) -> Result<()> {
        let mut loaded = None;
        let mut next_warm = Instant::now();
        let mut next_check = Instant::now();
        let mut next_dispatch = Instant::now();
        let mut next_cleanup = Instant::now();
        let mut initialized = false;
        while !stop.is_cancelled() {
            let setting = match timeout(AUTH_TIMEOUT, store::setting(&self.repo)).await {
                Ok(Ok(setting)) => setting,
                _ => {
                    if loaded.take().is_some() {
                        self.unload().await;
                    }
                    self.pause(stop).await;
                    continue;
                }
            };
            if loaded.is_some_and(|generation| !setting.enabled || setting.generation != generation)
            {
                self.unload().await;
                loaded = None;
                next_warm = Instant::now();
            }
            if Instant::now() >= next_cleanup {
                next_cleanup = Instant::now() + Duration::from_secs(15);
                let _ = timeout(AUTH_TIMEOUT, store::cleanup(&self.repo)).await;
            }
            if !setting.enabled {
                self.pause(stop).await;
                continue;
            }
            if !initialized {
                let initialize = async {
                    job_dispatch::ensure_queue_transport(self.repo.db(), POOL).await?;
                    job_dispatch::enable_pool(self.repo.db(), POOL).await
                };
                if !matches!(timeout(AUTH_TIMEOUT, initialize).await, Ok(Ok(()))) {
                    self.pause(stop).await;
                    continue;
                }
                initialized = true;
            }
            if loaded.is_none() && Instant::now() >= next_warm {
                if self.warm(setting.generation, stop).await {
                    loaded = Some(setting.generation);
                } else {
                    self.unload().await;
                    next_warm = Instant::now() + WARM_RETRY;
                }
            }
            let Some(generation) = loaded else {
                self.pause(stop).await;
                continue;
            };
            if Instant::now() >= next_check {
                next_check = Instant::now() + CHECK;
                if !self.heartbeat(generation).await {
                    self.unload().await;
                    loaded = None;
                    next_warm = Instant::now() + WARM_RETRY;
                    continue;
                }
            }
            if Instant::now() >= next_dispatch {
                next_dispatch = Instant::now() + CHECK;
                if !matches!(timeout(AUTH_TIMEOUT, self.relay()).await, Ok(Ok(()))) {
                    tracing::debug!("Inbox QA wake-up relay exceeded its deadline");
                }
            }
            match timeout(AUTH_TIMEOUT, self.broker.claim(&self.owner)).await {
                Ok(Ok(Some(delivery))) => {
                    let result = self.handle(delivery, generation, stop).await;
                    if !matches!(result, Ok(true)) {
                        tracing::debug!("Inbox QA execution session ended");
                        self.unload().await;
                        loaded = None;
                        next_warm = Instant::now() + WARM_RETRY;
                    }
                }
                Ok(Ok(None)) => {}
                _ => {
                    self.unload().await;
                    loaded = None;
                    next_warm = Instant::now() + WARM_RETRY;
                }
            }
            self.pause(stop).await;
        }
        Ok(())
    }

    async fn pause(&self, stop: &CancellationToken) {
        tokio::select! {
            () = stop.cancelled() => {}
            () = sleep(POLL) => {}
        }
    }

    async fn warm(&self, generation: i64, stop: &CancellationToken) -> bool {
        let lease = || {
            timeout(
                AUTH_TIMEOUT,
                store::beat(&self.repo, &self.owner, generation, false),
            )
        };
        if !matches!(lease().await, Ok(Ok(true))) {
            return false;
        }
        let warm = timeout(WARM_TIMEOUT, self.model.warm());
        tokio::pin!(warm);
        let mut next = Instant::now() + CHECK;
        loop {
            tokio::select! {
                () = stop.cancelled() => return false,
                result = &mut warm => {
                    return matches!(result, Ok(Ok(())))
                        && matches!(timeout(AUTH_TIMEOUT, store::beat(&self.repo, &self.owner, generation, true)).await, Ok(Ok(true)));
                }
                () = tokio::time::sleep_until(next) => {
                    if !matches!(lease().await, Ok(Ok(true))) {
                        return false;
                    }
                    next = Instant::now() + CHECK;
                }
            }
        }
    }

    async fn heartbeat(&self, generation: i64) -> bool {
        // Refresh the datastore permit before touching the independently supervised model.
        matches!(
            timeout(
                AUTH_TIMEOUT,
                store::beat(&self.repo, &self.owner, generation, true)
            )
            .await,
            Ok(Ok(true))
        ) && matches!(timeout(AUTH_TIMEOUT, self.model.warm()).await, Ok(Ok(())))
    }

    async fn unload(&self) {
        if matches!(
            timeout(STOP_TIMEOUT, self.model.shutdown()).await,
            Ok(Ok(()))
        ) {
            let _ = timeout(AUTH_TIMEOUT, store::forget_replica(&self.repo, &self.owner)).await;
        }
    }

    async fn relay(&self) -> Result<()> {
        let now = Utc::now();
        job_dispatch::recover_expired(self.repo.db(), POOL, now).await?;
        if let Some(claim) =
            job_dispatch::claim_ready(self.repo.db(), POOL, &self.owner, now).await?
        {
            let envelope = &claim.envelope;
            let identity = serde_json::to_vec(&(
                &envelope.workspace_generation,
                &envelope.task_id,
                envelope.revision,
            ))?;
            let message_id = format!("{:x}", Sha256::digest(identity));
            self.broker
                .publish(message_id, serde_json::to_vec(envelope)?)
                .await?;
            job_dispatch::mark_published(self.repo.db(), &claim, Utc::now()).await?;
        }
        Ok(())
    }

    async fn handle(
        &self,
        delivery: Received,
        generation: i64,
        stop: &CancellationToken,
    ) -> Result<bool> {
        let envelope = match Envelope::decode(delivery.payload()) {
            Ok(envelope) if envelope.pool == POOL => envelope,
            _ => {
                delivery.settle(Disposition::DeadLetter).await?;
                return Ok(true);
            }
        };
        let op_key = argos_osint_core::store::new_id();
        let claim = timeout(
            AUTH_TIMEOUT,
            job_dispatch::claim_delivery(
                self.repo.db(),
                &envelope,
                &self.owner,
                DELIVERY_SECS,
                Utc::now(),
                &op_key,
            ),
        )
        .await??;
        let lease = match claim {
            DeliveryClaim::Stale => {
                delivery.settle(Disposition::Ack).await?;
                return Ok(true);
            }
            DeliveryClaim::Deferred => {
                delivery.settle(Disposition::Requeue).await?;
                return Ok(true);
            }
            DeliveryClaim::Claimed(lease) => lease,
        };
        let Some(evidence) = timeout(AUTH_TIMEOUT, store::start(&self.repo, &lease)).await?? else {
            self.finish(
                delivery,
                &lease,
                DeliveryOutcome::Failed {
                    reason: "qa_stale_turn",
                },
            )
            .await?;
            return Ok(true);
        };
        let result = self
            .infer(&evidence, &lease, &delivery, generation, stop)
            .await;
        let (outcome, keep_warm) = match result {
            RunResult::Completed => (
                DeliveryOutcome::Complete {
                    result_ref: "qa_terminal",
                },
                true,
            ),
            RunResult::Stopped => (
                DeliveryOutcome::Relinquish {
                    reason: "qa_worker_shutdown",
                },
                false,
            ),
            RunResult::Fenced => (
                DeliveryOutcome::Failed {
                    reason: "qa_fenced",
                },
                false,
            ),
            RunResult::More | RunResult::Error => {
                let _ = timeout(
                    AUTH_TIMEOUT,
                    store::publish(&self.repo, &lease, QaTurnState::Failed, None),
                )
                .await;
                (
                    DeliveryOutcome::Failed {
                        reason: "qa_inference_failed",
                    },
                    false,
                )
            }
        };
        self.finish(delivery, &lease, outcome).await?;
        Ok(keep_warm)
    }

    async fn finish(
        &self,
        delivery: Received,
        lease: &DeliveryLease,
        outcome: DeliveryOutcome<'_>,
    ) -> Result<()> {
        let finish = timeout(
            AUTH_TIMEOUT,
            job_dispatch::finish_delivery(self.repo.db(), lease, outcome, Utc::now()),
        )
        .await??;
        let disposition = match finish {
            DeliveryFinish::Completed | DeliveryFinish::Requeued | DeliveryFinish::Stale => {
                Disposition::Ack
            }
            DeliveryFinish::Relinquished => Disposition::Requeue,
            DeliveryFinish::Failed => Disposition::DeadLetter,
        };
        timeout(AUTH_TIMEOUT, delivery.settle(disposition)).await??;
        Ok(())
    }

    async fn infer(
        &self,
        evidence: &FrozenEvidence,
        lease: &DeliveryLease,
        delivery: &Received,
        generation: i64,
        stop: &CancellationToken,
    ) -> RunResult {
        let (sender, mut receiver) = mpsc::channel(4);
        let cancel = CancellationToken::new();
        let inference = self
            .model
            .ask(evidence.request.clone(), sender, cancel.clone());
        tokio::pin!(inference);
        let mut next_check = Instant::now() + CHECK;
        let deadline = Instant::now() + TURN_TIMEOUT;
        let mut sequence = QaWorkerSequence::default();
        let mut model_finished = false;
        let result = loop {
            tokio::select! {
                biased;
                () = stop.cancelled() => break RunResult::Stopped,
                () = tokio::time::sleep_until(deadline) => break RunResult::Error,
                () = tokio::time::sleep_until(next_check) => {
                    next_check = Instant::now() + CHECK;
                    if !matches!(timeout(AUTH_TIMEOUT, self.renew(lease, delivery, generation)).await, Ok(Ok(true))) {
                        break RunResult::Fenced;
                    }
                }
                result = &mut inference, if !model_finished => {
                    model_finished = true;
                    if result.is_err() {
                        break RunResult::Error;
                    }
                }
                event = receiver.recv() => {
                    let Some(event) = event else { break RunResult::Error };
                    match timeout(AUTH_TIMEOUT, publish(&self.repo, lease, evidence, event, &mut sequence)).await {
                        Ok(Ok(RunResult::More)) => {}
                        Ok(Ok(other)) => break other,
                        _ => break RunResult::Error,
                    }
                }
            }
        };
        if !model_finished {
            if !matches!(result, RunResult::Completed) {
                cancel.cancel();
            }
            let _ = timeout(STOP_TIMEOUT, inference).await;
        }
        result
    }

    async fn renew(
        &self,
        lease: &DeliveryLease,
        delivery: &Received,
        generation: i64,
    ) -> Result<bool> {
        let (permitted, live, dispatch, queue) = tokio::join!(
            store::beat(&self.repo, &self.owner, generation, true),
            store::is_live(&self.repo, lease),
            job_dispatch::renew_delivery(self.repo.db(), lease, DELIVERY_SECS, Utc::now()),
            delivery.renew(),
        );
        Ok(permitted? && live? && dispatch? && queue?)
    }
}

enum RunResult {
    More,
    Completed,
    Stopped,
    Fenced,
    Error,
}

async fn publish(
    repo: &Repository,
    lease: &DeliveryLease,
    evidence: &FrozenEvidence,
    event: QaWorkerEvent,
    sequence: &mut QaWorkerSequence,
) -> Result<RunResult> {
    sequence.accept(&evidence.request, &event)?;
    let (state, finding) = match event {
        QaWorkerEvent::Finding {
            source_id,
            start,
            end,
            text,
            ..
        } => {
            let source = evidence
                .sources
                .iter()
                .find(|source| source.opaque_id == source_id)
                .ok_or_else(|| anyhow!("unknown Inbox QA source"))?;
            let finding = QaFinding::new(
                source.reference.clone(),
                source.label.clone(),
                text,
                start,
                end,
                source.observed_at.clone(),
            );
            (QaTurnState::Running, Some(finding))
        }
        QaWorkerEvent::Complete { outcome, .. } => {
            let state = match outcome {
                QaWorkerOutcome::Answered => QaTurnState::Answered,
                QaWorkerOutcome::InsufficientContext => QaTurnState::InsufficientContext,
                QaWorkerOutcome::Failed => QaTurnState::Failed,
                QaWorkerOutcome::Cancelled => QaTurnState::Canceled,
                _ => return Err(anyhow!("unsupported Inbox QA outcome")),
            };
            (state, None)
        }
        _ => return Err(anyhow!("unsupported Inbox QA event")),
    };
    if !store::publish(repo, lease, state, finding).await? {
        return Ok(RunResult::Fenced);
    }
    Ok(if state.is_terminal() {
        RunResult::Completed
    } else {
        RunResult::More
    })
}
