//! Dedicated durable Jobs consumer for the Embedded Assistant.

use std::collections::BTreeMap;
use std::future::poll_fn;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use argos_osint_api::inbox_qa::decision;
use argos_osint_api::inbox_qa::decision::DecisionRuntime;
use argos_osint_api::inbox_qa::{
    validated_summary_finding, CancellationToken, QaInference, QaRuntimeAvailability,
};
use argos_osint_core::storage::inbox_qa::{self as store, FrozenEvidence, POOL};
use argos_osint_core::storage::job_dispatch::{
    self, DeliveryClaim, DeliveryFinish, DeliveryLease, DeliveryOutcome, Envelope,
};
use argos_osint_core::storage::repo::Repository;
use argos_osint_core::storage::tx::TxError;
use chrono::Utc;
use fleetworks_contracts::chat::QaEvidenceProvenance;
use fleetworks_contracts::chat::QaGeneratorPreference;
use fleetworks_contracts::chat::QaModelTarget;
use fleetworks_contracts::chat::{
    QaAvailabilityReason, QaFinding, QaStartupStage, QaTurnLimitation, QaTurnState,
};
use fleetworks_contracts::qa_worker::{
    valid_uncited_reply, verified_operational_sources, QaContextUsage, QaDialogueKind,
    QaWorkerEvent, QaWorkerOutcome, QaWorkerSequence,
};
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;
use tokio::time::{sleep, timeout, Instant};

use crate::qa_worker::QaWorkerSupervisor;
use crate::queue_client::{Client, Disposition, Received};

const DELIVERY_SECS: i64 = 12;
const POLL: Duration = Duration::from_millis(250);
const CHECK: Duration = Duration::from_secs(2);
const AUTH_TIMEOUT: Duration = Duration::from_secs(2);
// The supervisor separately bounds pack verification and the model readiness handshake.
const WARM_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const STOP_TIMEOUT: Duration = Duration::from_secs(8);
const TURN_TIMEOUT: Duration = Duration::from_secs(30);
const SETTLED_TURN_DRAIN: Duration = Duration::from_secs(2);
const OPERATIONAL_COMMIT_RESERVE_MS: i64 = 500;
const WARM_RETRY: Duration = Duration::from_secs(10);

struct Executor {
    repo: Repository,
    broker: Client,
    model: QaWorkerSupervisor,
    owner: String,
    placement: String,
    availability: Option<QaRuntimeAvailability>,
    replica_lease: ReplicaLease,
    resident_generation: Option<i64>,
    decision: Arc<dyn DecisionRuntime>,
}

#[derive(Default)]
enum ReplicaLease {
    #[default]
    Unclaimed,
    Claimed,
    Reaped,
}

impl ReplicaLease {
    async fn release(
        &mut self,
        shutdown: impl Future<Output = Result<()>>,
        forget: impl Future<Output = Result<(), TxError>>,
    ) {
        if matches!(self, Self::Claimed) {
            if !matches!(timeout(STOP_TIMEOUT, shutdown).await, Ok(Ok(()))) {
                tracing::warn!(
                    qa_replica_stage = "reap_confirmation",
                    "Inbox QA model reap is unconfirmed"
                );
                return;
            }
            *self = Self::Reaped;
        }
        if matches!(self, Self::Reaped) {
            if matches!(timeout(AUTH_TIMEOUT, forget).await, Ok(Ok(()))) {
                *self = Self::Unclaimed;
            } else {
                tracing::warn!(
                    qa_replica_stage = "reap_record",
                    "Inbox QA reap confirmation could not be recorded"
                );
            }
        }
    }
}

pub(crate) async fn run(
    repo: Repository,
    broker: Client,
    model: QaWorkerSupervisor,
    placement: String,
    availability: Option<QaRuntimeAvailability>,
    decision: Arc<dyn DecisionRuntime>,
    signal: impl Future<Output = ()>,
) -> Result<()> {
    let mut executor = Executor {
        repo,
        broker,
        model,
        owner: format!("assistant-qa-{}", argos_osint_core::store::new_id()),
        placement,
        availability,
        replica_lease: ReplicaLease::default(),
        resident_generation: None,
        decision,
    };
    let stop = CancellationToken::new();
    let result = {
        let session = executor.serve(&stop);
        tokio::pin!(session, signal);
        tokio::select! {
            result = &mut session => result,
            () = &mut signal => {
                stop.cancel();
                session.await
            }
        }
    };
    executor.unload().await;
    result
}

impl Executor {
    async fn serve(&mut self, stop: &CancellationToken) -> Result<()> {
        let mut loaded = None;
        let mut observed_generation = None;
        let mut next_warm = Instant::now();
        let mut next_check = Instant::now();
        let mut next_dispatch = Instant::now();
        let mut next_cleanup = Instant::now();
        let mut initialized = false;
        let mut next_store_notice = Instant::now();
        let mut next_queue_notice = Instant::now();
        while !stop.is_cancelled() {
            if loaded.is_none() && !matches!(self.replica_lease, ReplicaLease::Unclaimed) {
                // Retry unfinished shutdown or record removal before warming another worker.
                self.unload().await;
                if !matches!(self.replica_lease, ReplicaLease::Unclaimed) {
                    self.pause(stop).await;
                    continue;
                }
            }
            let setting = match timeout(AUTH_TIMEOUT, store::setting(&self.repo)).await {
                Ok(Ok(setting)) => setting,
                _ => {
                    if Instant::now() >= next_store_notice {
                        next_store_notice = Instant::now() + WARM_RETRY;
                        tracing::warn!(
                            qa_replica_stage = "settings_read",
                            "Inbox QA settings unavailable"
                        );
                    }
                    if loaded.take().is_some() {
                        self.unload().await;
                    }
                    self.pause(stop).await;
                    continue;
                }
            };
            if observed_generation != Some(setting.generation) {
                observed_generation = Some(setting.generation);
                next_warm = Instant::now();
            }
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
            if setting.enabled && !initialized {
                let initialize = async {
                    job_dispatch::ensure_queue_transport(self.repo.db(), POOL)
                        .await
                        .map_err(|_| "queue_transport")?;
                    job_dispatch::enable_pool(self.repo.db(), POOL)
                        .await
                        .map_err(|_| "queue_pool")
                };
                let failure = match timeout(AUTH_TIMEOUT, initialize).await {
                    Ok(Ok(())) => None,
                    Ok(Err(stage)) => Some(stage),
                    Err(_) => Some("queue_setup_timeout"),
                };
                if let Some(stage) = failure {
                    if Instant::now() >= next_queue_notice {
                        next_queue_notice = Instant::now() + WARM_RETRY;
                        tracing::warn!(
                            qa_queue_stage = stage,
                            "Inbox QA queue initialization failed"
                        );
                    }
                    self.pause(stop).await;
                    continue;
                }
                initialized = true;
            }
            if setting.enabled
                && loaded.is_none()
                && matches!(self.replica_lease, ReplicaLease::Unclaimed)
                && Instant::now() >= next_warm
            {
                next_warm = Instant::now() + WARM_RETRY;
                let claim = timeout(
                    AUTH_TIMEOUT,
                    store::claim_replica(
                        &self.repo,
                        &self.placement,
                        &self.owner,
                        setting.generation,
                    ),
                )
                .await;
                match claim {
                    Ok(Ok(store::ReplicaClaim::Warm)) => {
                        self.replica_lease = ReplicaLease::Claimed;
                        if self
                            .warm(setting.generation, setting.generator_preference, stop)
                            .await
                        {
                            loaded = Some(setting.generation);
                            self.resident_generation = loaded;
                            self.record_availability(setting.generation, None);
                            if let Some(availability) = &self.availability {
                                availability.record_resident(setting.generation);
                            }
                            tracing::info!(qa_replica_stage = "ready", "Inbox QA replica ready");
                        } else {
                            if let Some(availability) = &self.availability {
                                availability.clear_stage(setting.generation);
                            }
                            self.unload().await;
                        }
                    }
                    Ok(Ok(store::ReplicaClaim::ConfirmReap)) => {
                        self.replica_lease = ReplicaLease::Claimed;
                        self.unload().await;
                    }
                    Ok(Ok(store::ReplicaClaim::Wait)) => {}
                    _ => {
                        // The claim may have committed before the reply was lost. No model
                        // was opened, so only the owner-fenced record needs cleanup.
                        self.replica_lease = ReplicaLease::Reaped;
                        tracing::warn!(
                            qa_replica_stage = "replica_claim",
                            "Inbox QA replica could not claim startup"
                        );
                    }
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
                    tracing::warn!(
                        qa_queue_stage = "relay",
                        "Inbox QA wake-up relay unavailable"
                    );
                }
            }
            match timeout(AUTH_TIMEOUT, self.broker.claim(&self.owner)).await {
                Ok(Ok(Some(delivery))) => {
                    let result = self.handle(delivery, generation, stop).await;
                    if result.is_err() {
                        tracing::warn!(
                            qa_queue_stage = "delivery",
                            "Inbox QA delivery did not settle"
                        );
                    }
                    if !matches!(result, Ok(true)) {
                        self.record_availability(
                            generation,
                            Some(QaAvailabilityReason::WorkerStartFailed),
                        );
                        self.unload().await;
                        loaded = None;
                        next_warm = Instant::now() + WARM_RETRY;
                    }
                }
                Ok(Ok(None)) => {}
                _ => {
                    // The replica and worker heartbeat still governs the resident model.
                    if Instant::now() >= next_queue_notice {
                        next_queue_notice = Instant::now() + WARM_RETRY;
                        tracing::warn!(
                            qa_queue_stage = "claim",
                            "Inbox QA queue claim unavailable"
                        );
                    }
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

    async fn warm(
        &self,
        generation: i64,
        preference: QaGeneratorPreference,
        stop: &CancellationToken,
    ) -> bool {
        if !self.renew_replica(generation, false, "warm_lease").await {
            return false;
        }
        let selected_model = |model| {
            if let Some(availability) = &self.availability {
                availability.record_model(generation, model);
            }
        };
        let progress = |stage| {
            if let Some(availability) = &self.availability {
                availability.record_stage(generation, stage);
            }
        };
        let warm = timeout(
            WARM_TIMEOUT,
            self.model
                .warm_with_progress(preference, &selected_model, &progress),
        );
        tokio::pin!(warm);
        let mut next = Instant::now() + CHECK;
        loop {
            tokio::select! {
                () = stop.cancelled() => return false,
                result = &mut warm => {
                    match result {
                        Ok(Ok(())) => {
                            progress(QaStartupStage::Activating);
                            return self.renew_replica(generation, true, "ready_lease").await;
                        }
                        Ok(Err(error)) => {
                            self.record_availability(generation, Some(crate::qa_worker::availability_reason(&error)));
                            tracing::warn!(qa_replica_stage = "warm_model", "Inbox QA model did not warm");
                        }
                        Err(_) => {
                            self.record_availability(generation, Some(QaAvailabilityReason::WorkerStartFailed));
                            tracing::warn!(qa_replica_stage = "warm_timeout", "Inbox QA model did not warm within its budget");
                        }
                    }
                    return false;
                }
                () = tokio::time::sleep_until(next) => {
                    if !self.renew_replica(generation, false, "warm_lease").await {
                        return false;
                    }
                    next = Instant::now() + CHECK;
                }
            }
        }
    }

    fn record_availability(&self, generation: i64, reason: Option<QaAvailabilityReason>) {
        if let Some(availability) = &self.availability {
            availability.record(generation, reason);
        }
    }

    async fn renew_replica(&self, generation: i64, ready: bool, stage: &'static str) -> bool {
        let result = timeout(
            AUTH_TIMEOUT,
            store::beat(&self.repo, &self.placement, &self.owner, generation, ready),
        )
        .await;
        match result {
            Ok(Ok(true)) => true,
            Ok(Ok(false)) => {
                tracing::info!(qa_replica_stage = stage, "Inbox QA replica permit ended");
                false
            }
            _ => {
                tracing::warn!(
                    qa_replica_stage = stage,
                    "Inbox QA replica permit could not be renewed"
                );
                false
            }
        }
    }

    async fn heartbeat(&self, generation: i64) -> bool {
        // Refresh the datastore permit before touching the independently supervised model.
        if !self
            .renew_replica(generation, true, "heartbeat_lease")
            .await
        {
            return false;
        }
        let reason = match timeout(AUTH_TIMEOUT, self.model.check_ready()).await {
            Ok(Ok(())) => return true,
            Ok(Err(error)) => crate::qa_worker::availability_reason(&error),
            Err(_) => QaAvailabilityReason::WorkerStartFailed,
        };
        self.record_availability(generation, Some(reason));
        tracing::warn!(
            qa_replica_stage = "heartbeat_model",
            "Inbox QA model heartbeat failed"
        );
        false
    }

    async fn unload(&mut self) {
        self.replica_lease
            .release(
                self.model.shutdown(),
                store::forget_replica(&self.repo, &self.placement, &self.owner),
            )
            .await;
        if matches!(self.replica_lease, ReplicaLease::Claimed) {
            return;
        }
        if let Some((generation, availability)) = self
            .resident_generation
            .take()
            .zip(self.availability.as_ref())
        {
            availability.record_unloaded(generation);
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
        let mut evidence = match timeout(AUTH_TIMEOUT, store::start(&self.repo, &lease)).await {
            Ok(Ok(Some(evidence))) => evidence,
            Ok(Ok(None)) => {
                tracing::debug!(
                    qa_turn_stage = "start_fenced",
                    "Inbox QA turn could not start"
                );
                self.finish(
                    delivery,
                    &lease,
                    DeliveryOutcome::Failed {
                        reason: "qa_stale_turn",
                    },
                )
                .await?;
                return Ok(true);
            }
            Ok(Err(error)) => {
                tracing::warn!(
                    qa_turn_stage = "start_store_error",
                    "Inbox QA turn could not start"
                );
                return Err(error.into());
            }
            Err(error) => {
                tracing::warn!(
                    qa_turn_stage = "start_store_timeout",
                    "Inbox QA turn could not start"
                );
                return Err(error.into());
            }
        };
        let result = match self.prepare(&mut evidence, &lease, stop).await {
            RunResult::More => {
                self.infer(&evidence, &lease, &delivery, generation, stop)
                    .await
            }
            result => result,
        };
        let (outcome, keep_warm) = match result {
            RunResult::Completed => (
                DeliveryOutcome::Complete {
                    result_ref: "qa_terminal",
                },
                true,
            ),
            RunResult::EvidenceExpired { keep_warm } => (
                DeliveryOutcome::Complete {
                    result_ref: "qa_terminal",
                },
                keep_warm,
            ),
            RunResult::FailedTerminal { keep_warm } => (
                DeliveryOutcome::Failed {
                    reason: "qa_inference_failed",
                },
                keep_warm,
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
                let terminal = timeout(
                    AUTH_TIMEOUT,
                    store::publish(&self.repo, &lease, QaTurnState::Failed, None),
                )
                .await;
                let stage = match terminal {
                    Ok(Ok(true)) => None,
                    Ok(Ok(false)) => Some("failure_terminal_fenced"),
                    Ok(Err(_)) => Some("failure_terminal_store_error"),
                    Err(_) => Some("failure_terminal_store_timeout"),
                };
                if let Some(stage) = stage {
                    tracing::warn!(
                        qa_turn_stage = stage,
                        "Inbox QA failure could not be recorded"
                    );
                }
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
        // Only this executor can replace the generator while it owns the delivery lease.
        if stop.is_cancelled() {
            return RunResult::Stopped;
        }
        let Some(model_target) = self.model.active_model().await else {
            return RunResult::Error;
        };
        let budget = inference_budget(evidence, Utc::now().timestamp_millis());
        if budget.is_zero() {
            return self.finish_expired_evidence(lease).await;
        }
        if evidence.request.dialogue.is_none() {
            return RunResult::Error;
        }
        let mut publication = Publication::new(model_target);
        let deadline = Instant::now() + budget;
        let (sender, mut receiver) = mpsc::channel(4);
        let cancel = CancellationToken::new();
        let inference = self
            .model
            .ask(evidence.request.clone(), sender, cancel.clone());
        tokio::pin!(inference);
        let mut next_check = Instant::now() + CHECK;
        let mut model_finished = false;
        let mut model_polled = false;
        let mut received_event = false;
        let mut result = loop {
            tokio::select! {
                biased;
                () = stop.cancelled() => break RunResult::Stopped,
                () = tokio::time::sleep_until(deadline) => {
                    if uses_fresh_evidence(evidence) {
                        break self.finish_expired_evidence(lease).await;
                    }
                    tracing::warn!(qa_turn_stage = "turn_deadline", "Inbox QA turn exceeded its budget");
                    break RunResult::Error;
                }
                () = tokio::time::sleep_until(next_check) => {
                    next_check = Instant::now() + CHECK;
                    let stage = match timeout(AUTH_TIMEOUT, self.renew(lease, delivery, generation)).await {
                        Ok(Ok(true)) => None,
                        Ok(Ok(false)) => Some("lease_fenced"),
                        Ok(Err(_)) => Some("lease_error"),
                        Err(_) => Some("lease_timeout"),
                    };
                    if let Some(stage) = stage {
                        tracing::warn!(qa_turn_stage = stage, "Inbox QA turn lease ended");
                        break RunResult::Fenced;
                    }
                }
                result = poll_fn(|context| {
                    model_polled = true;
                    inference.as_mut().poll(context)
                }), if !model_finished => {
                    model_finished = true;
                    if result.is_err() {
                        let stage = if received_event { "worker_inference_error" } else { "worker_error_before_event" };
                        tracing::warn!(qa_turn_stage = stage, "Inbox QA worker inference failed");
                        break RunResult::Error;
                    }
                }
                event = receiver.recv() => {
                    let Some(event) = event else {
                        let stage = if received_event { "event_stream_incomplete" } else { "event_stream_empty" };
                        tracing::warn!(qa_turn_stage = stage, "Inbox QA worker event stream ended");
                        break RunResult::Error;
                    };
                    received_event = true;
                    match timeout(AUTH_TIMEOUT, publication.publish(&self.repo, lease, evidence, event)).await {
                        Ok(Ok(RunResult::More)) => {}
                        Ok(Ok(other)) => {
                            if matches!(other, RunResult::Fenced) {
                                tracing::warn!(qa_turn_stage = "publication_fenced", "Inbox QA event publication was fenced");
                            }
                            break other;
                        }
                        Ok(Err(_)) => {
                            tracing::warn!(qa_turn_stage = "publication_error", "Inbox QA event publication failed");
                            break RunResult::Error;
                        }
                        Err(_) => {
                            tracing::warn!(qa_turn_stage = "publication_timeout", "Inbox QA event publication exceeded its budget");
                            break RunResult::Error;
                        }
                    }
                }
            }
        };
        if model_polled && !model_finished {
            match &mut result {
                RunResult::EvidenceExpired { keep_warm }
                | RunResult::FailedTerminal { keep_warm } => {
                    // The turn has settled. Keep the model only after its validated terminal frame.
                    *keep_warm =
                        drain_worker_terminal(inference.as_mut(), &mut receiver, stop).await;
                    if !*keep_warm {
                        tracing::warn!(
                            qa_turn_stage = "worker_terminal_drain",
                            "Inbox QA worker did not settle after the turn ended"
                        );
                    }
                }
                other => {
                    if !matches!(other, RunResult::Completed) {
                        cancel.cancel();
                    }
                    let _ = timeout(STOP_TIMEOUT, inference).await;
                }
            }
        }
        result
    }

    async fn finish_expired_evidence(&self, lease: &DeliveryLease) -> RunResult {
        match timeout(
            AUTH_TIMEOUT,
            store::publish(&self.repo, lease, QaTurnState::InsufficientContext, None),
        )
        .await
        {
            Ok(Ok(true)) => {
                tracing::info!(
                    qa_turn_stage = "evidence_expired",
                    "Inbox QA evidence expired"
                );
                RunResult::EvidenceExpired { keep_warm: true }
            }
            Ok(Ok(false)) => RunResult::Fenced,
            _ => {
                tracing::warn!(
                    qa_turn_stage = "evidence_expired_publication",
                    "Inbox QA evidence expiry could not be recorded"
                );
                RunResult::Error
            }
        }
    }

    async fn prepare(
        &self,
        evidence: &mut FrozenEvidence,
        lease: &DeliveryLease,
        stop: &CancellationToken,
    ) -> RunResult {
        let budget = inference_budget(evidence, Utc::now().timestamp_millis());
        if budget.is_zero() {
            return self.finish_expired_evidence(lease).await;
        }
        let prepare = async {
            let Some(context) = store::begin_planning(&self.repo, lease).await? else {
                return Ok(RunResult::Fenced);
            };
            let mut plan =
                decision::select_response_plan(Some(self.decision.as_ref()), evidence, &context)
                    .await;
            let needs_decoded_clarification = plan
                .worker
                .as_ref()
                .is_some_and(|request| request.clarification.is_some());
            let needs_general_dialogue = evidence
                .request
                .dialogue
                .as_ref()
                .is_some_and(|dialogue| dialogue.kind.needs_general_support());
            let (plan_supported, job_logs_supported) = self
                .model
                .response_plan_support(needs_decoded_clarification, needs_general_dialogue)
                .await;
            plan.negotiated = plan_supported;
            if !job_logs_supported {
                plan.ordered_passages.retain(|id| {
                    evidence
                        .request
                        .passages
                        .iter()
                        .any(|passage| &passage.source_id == id && !passage.job_log_tail)
                });
            }
            if (plan.requires_worker_support || needs_general_dialogue) && !plan.negotiated {
                return Ok(
                    if store::publish_unavailable(
                        &self.repo,
                        lease,
                        QaTurnLimitation::CapabilityUnavailable,
                    )
                    .await?
                    {
                        RunResult::Completed
                    } else {
                        RunResult::Fenced
                    },
                );
            }
            let request = plan
                .request(evidence)
                .ok_or_else(|| anyhow!("approved plan evidence is unavailable"))?;
            if !store::accept_plan(&self.repo, lease, &plan).await? {
                return Ok(RunResult::Fenced);
            }
            evidence.request = request;
            Ok::<_, anyhow::Error>(RunResult::More)
        };
        tokio::select! {
            biased;
            () = stop.cancelled() => RunResult::Stopped,
            result = timeout(budget.min(AUTH_TIMEOUT), prepare) => {
                match result {
                    Ok(Ok(result)) => result,
                    Ok(Err(_)) => {
                        tracing::warn!(qa_turn_stage = "plan_error", "Inbox QA response preparation failed");
                        RunResult::Error
                    }
                    Err(_) => {
                        if budget <= AUTH_TIMEOUT {
                            return self.finish_expired_evidence(lease).await;
                        }
                        tracing::warn!(qa_turn_stage = "plan_timeout", "Inbox QA response preparation exceeded its budget");
                        RunResult::Error
                    }
                }
            }
        }
    }

    async fn renew(
        &self,
        lease: &DeliveryLease,
        delivery: &Received,
        generation: i64,
    ) -> Result<bool> {
        let (permitted, live, dispatch, queue) = tokio::join!(
            store::beat(&self.repo, &self.placement, &self.owner, generation, true),
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
    EvidenceExpired { keep_warm: bool },
    FailedTerminal { keep_warm: bool },
    Stopped,
    Fenced,
    Error,
}

async fn drain_worker_terminal(
    inference: impl Future<Output = Result<()>>,
    receiver: &mut mpsc::Receiver<QaWorkerEvent>,
    stop: &CancellationToken,
) -> bool {
    let drain = async {
        tokio::pin!(inference);
        loop {
            tokio::select! {
                biased;
                result = &mut inference => return result.is_ok(),
                Some(_) = receiver.recv() => {}
            }
        }
    };
    tokio::select! {
        biased;
        () = stop.cancelled() => false,
        result = timeout(SETTLED_TURN_DRAIN, drain) => matches!(result, Ok(true)),
    }
}

fn cited_finding(
    evidence: &FrozenEvidence,
    source_id: &str,
    start: u32,
    end: u32,
    text: String,
    now_ms: i64,
) -> Result<Option<QaFinding>> {
    let source = evidence
        .sources
        .iter()
        .find(|source| source.opaque_id == source_id)
        .ok_or_else(|| anyhow!("unknown Inbox QA source"))?;
    if !observation_is_fresh(evidence.sampled_at_ms, now_ms)
        || !source.observation_is_valid(evidence.sampled_at_ms, now_ms)
    {
        return Ok(None);
    }
    if !source.whole_passage || evidence.request.plan.is_some() {
        return Ok(Some(source.finding(text, start, end)));
    }
    let passage = evidence
        .request
        .passages
        .iter()
        .find(|passage| passage.source_id == source_id)
        .ok_or_else(|| anyhow!("unknown Inbox QA passage"))?;
    let Some(complete) = validated_summary_finding(&passage.text, &source.label, start, end, &text)
    else {
        return Ok(None);
    };
    let end = u32::try_from(complete.chars().count())?;
    Ok(Some(source.finding(complete.to_owned(), 0, end)))
}

fn observation_is_fresh(observed_ms: i64, now_ms: i64) -> bool {
    (0..=store::MAX_EVIDENCE_AGE_MS).contains(&now_ms.saturating_sub(observed_ms))
}

fn is_operational(evidence: &FrozenEvidence) -> bool {
    evidence
        .request
        .dialogue
        .as_ref()
        .is_some_and(|dialogue| dialogue.kind == QaDialogueKind::Operational)
}

fn is_contextual(evidence: &FrozenEvidence) -> bool {
    evidence
        .request
        .dialogue
        .as_ref()
        .is_some_and(|dialogue| dialogue.kind == QaDialogueKind::Contextual)
}

fn uses_fresh_evidence(evidence: &FrozenEvidence) -> bool {
    evidence
        .request
        .dialogue
        .as_ref()
        .is_some_and(|dialogue| dialogue.kind.uses_evidence())
}

fn inference_budget(evidence: &FrozenEvidence, now_ms: i64) -> Duration {
    if !uses_fresh_evidence(evidence) {
        return TURN_TIMEOUT;
    }
    let deadline = evidence
        .sampled_at_ms
        .saturating_add(store::MAX_EVIDENCE_AGE_MS - OPERATIONAL_COMMIT_RESERVE_MS);
    Duration::from_millis(deadline.saturating_sub(now_ms).max(0) as u64)
}

fn requires_complete_snapshot(evidence: &FrozenEvidence) -> bool {
    is_operational(evidence)
        && evidence.request.plan.is_none()
        && evidence.sources.len() > 1
        && evidence.sources.iter().any(|source| !source.whole_passage)
}

fn defer_findings(evidence: &FrozenEvidence) -> bool {
    is_contextual(evidence)
        || requires_complete_snapshot(evidence)
        || evidence
            .request
            .plan
            .as_ref()
            .is_some_and(|plan| plan.required_passages.len() > 1)
        || evidence.sources.iter().any(|source| {
            matches!(
                source.provenance,
                Some(
                    QaEvidenceProvenance::PreviousObservation
                        | QaEvidenceProvenance::Historical { .. }
                )
            )
        })
}

struct Publication {
    model_target: QaModelTarget,
    sequence: QaWorkerSequence,
    cited: BTreeMap<String, QaFinding>,
}

impl Publication {
    fn new(model_target: QaModelTarget) -> Self {
        Self {
            model_target,
            sequence: QaWorkerSequence::default(),
            cited: BTreeMap::new(),
        }
    }

    fn accepted_reply(
        &self,
        evidence: &FrozenEvidence,
        context: Option<QaContextUsage>,
        reply: Option<String>,
        now_ms: i64,
    ) -> Option<(String, Vec<QaFinding>)> {
        if context.is_none_or(|usage| usage.model_windows == 0) {
            return None;
        }
        let reply = reply?;
        let contextual = is_contextual(evidence);
        if !is_operational(evidence) && !contextual {
            return Some((reply, Vec::new()));
        }
        if !observation_is_fresh(evidence.sampled_at_ms, now_ms) {
            return None;
        }
        let selected = if contextual {
            if !valid_uncited_reply(&evidence.request, &reply) {
                return None;
            }
            evidence.request.passages.iter().collect()
        } else {
            verified_operational_sources(&evidence.request, &reply)?
        };
        let complete = requires_complete_snapshot(evidence);
        if complete && selected.len() != evidence.sources.len() {
            return None;
        }
        let deferred = defer_findings(evidence);
        let mut findings = Vec::new();
        for passage in selected {
            let finding = self.cited.get(&passage.source_id)?;
            let source = evidence
                .sources
                .iter()
                .find(|source| source.opaque_id == passage.source_id)?;
            if !source.observation_is_valid(evidence.sampled_at_ms, now_ms) {
                return None;
            }
            if deferred {
                findings.push(finding.clone());
            }
        }
        Some((reply, findings))
    }

    async fn publish(
        &mut self,
        repo: &Repository,
        lease: &DeliveryLease,
        evidence: &FrozenEvidence,
        event: QaWorkerEvent,
    ) -> Result<RunResult> {
        self.sequence.accept(&evidence.request, &event)?;
        let (state, context, reply, findings) = match event {
            QaWorkerEvent::Delta { text, .. } => {
                if (is_operational(evidence) || is_contextual(evidence))
                    && (defer_findings(evidence)
                        || !observation_is_fresh(
                            evidence.sampled_at_ms,
                            Utc::now().timestamp_millis(),
                        ))
                {
                    return Ok(RunResult::More);
                }
                return Ok(if store::publish_draft(repo, lease, text).await? {
                    RunResult::More
                } else {
                    RunResult::Fenced
                });
            }
            QaWorkerEvent::Finding {
                source_id,
                start,
                end,
                text,
                ..
            } => {
                let Some(finding) = cited_finding(
                    evidence,
                    &source_id,
                    start,
                    end,
                    text,
                    Utc::now().timestamp_millis(),
                )?
                else {
                    return Ok(RunResult::More);
                };
                // Independent current facts can appear while the rest of the reply is pending.
                // Grouped and historical facts stay together under the terminal lease fence.
                if !defer_findings(evidence)
                    && !store::publish(repo, lease, QaTurnState::Running, Some(finding.clone()))
                        .await?
                {
                    return Ok(RunResult::Fenced);
                }
                self.cited.insert(source_id, finding);
                return Ok(RunResult::More);
            }
            QaWorkerEvent::Complete {
                outcome,
                context,
                reply,
                ..
            } => {
                let now_ms = Utc::now().timestamp_millis();
                let (state, reply, findings) = match outcome {
                    QaWorkerOutcome::Answered
                        if uses_fresh_evidence(evidence)
                            && !observation_is_fresh(evidence.sampled_at_ms, now_ms) =>
                    {
                        (QaTurnState::InsufficientContext, None, Vec::new())
                    }
                    QaWorkerOutcome::Answered => {
                        match self.accepted_reply(evidence, context, reply, now_ms) {
                            Some((reply, findings)) => {
                                (QaTurnState::Answered, Some(reply), findings)
                            }
                            None => (QaTurnState::Failed, None, Vec::new()),
                        }
                    }
                    QaWorkerOutcome::InsufficientContext => {
                        (QaTurnState::InsufficientContext, None, Vec::new())
                    }
                    QaWorkerOutcome::InputTooLarge => {
                        return Ok(
                            if store::publish_unavailable(
                                repo,
                                lease,
                                QaTurnLimitation::QuestionTooLong,
                            )
                            .await?
                            {
                                RunResult::Completed
                            } else {
                                RunResult::Fenced
                            },
                        );
                    }
                    QaWorkerOutcome::Failed => (QaTurnState::Failed, None, Vec::new()),
                    QaWorkerOutcome::Cancelled => (QaTurnState::Canceled, None, Vec::new()),
                    _ => return Err(anyhow!("unsupported Inbox QA outcome")),
                };
                (state, context, reply, findings)
            }
            _ => return Err(anyhow!("unsupported Inbox QA event")),
        };
        let result = store::ModelResult {
            context,
            reply,
            model_target: Some(self.model_target),
        };
        if !store::publish_findings_and_complete(repo, lease, state, &findings, result).await? {
            return Ok(RunResult::Fenced);
        }
        Ok(if state == QaTurnState::Failed {
            RunResult::FailedTerminal { keep_warm: true }
        } else {
            RunResult::Completed
        })
    }
}

#[cfg(test)]
mod tests {
    use chrono::DateTime;
    use fleetworks_contracts::chat::ChatSourceKind;
    use fleetworks_contracts::chat::ChatSourceReference;
    use fleetworks_contracts::chat::QaMeasurement;
    use fleetworks_contracts::chat::QaMeasurementUnit;
    use fleetworks_contracts::qa_worker::QaGuidance;
    use fleetworks_contracts::qa_worker::QaPassage;
    use fleetworks_contracts::qa_worker::QaWorkerDialogue;
    use fleetworks_contracts::qa_worker::QaWorkerPlan;
    use fleetworks_contracts::qa_worker::QaWorkerRequest;

    use super::*;

    #[tokio::test(start_paused = true)]
    async fn expired_turn_drain_consumes_late_frames_to_keep_the_worker() {
        let (sender, mut receiver) = mpsc::channel(1);
        let stop = CancellationToken::new();
        let inference = async move {
            sleep(Duration::from_millis(100)).await;
            sender
                .send(QaWorkerEvent::Delta {
                    turn_id: "expired-turn".into(),
                    sequence: 1,
                    text: "Late response".into(),
                })
                .await
                .unwrap();
            sender
                .send(QaWorkerEvent::Complete {
                    turn_id: "expired-turn".into(),
                    sequence: 2,
                    outcome: QaWorkerOutcome::InsufficientContext,
                    context: None,
                    reply: None,
                })
                .await
                .unwrap();
            Ok(())
        };

        assert!(drain_worker_terminal(inference, &mut receiver, &stop).await);
    }

    #[tokio::test(start_paused = true)]
    async fn expired_turn_requires_reap_if_drain_fails_times_out_or_stops() {
        let (_sender, mut receiver) = mpsc::channel(1);
        let stop = CancellationToken::new();
        assert!(
            !drain_worker_terminal(
                std::future::ready(Err(anyhow!("invalid worker terminal"))),
                &mut receiver,
                &stop,
            )
            .await
        );

        let started = Instant::now();
        assert!(!drain_worker_terminal(std::future::pending(), &mut receiver, &stop).await);
        assert!(started.elapsed() >= SETTLED_TURN_DRAIN);

        stop.cancel();
        assert!(
            !drain_worker_terminal(
                async { panic!("a stopped service must not resume the worker") },
                &mut receiver,
                &stop,
            )
            .await
        );
    }

    #[tokio::test(start_paused = true)]
    async fn retries_recording_confirmed_reap_without_repeating_shutdown() {
        let mut lease = ReplicaLease::Claimed;
        lease
            .release(std::future::ready(Ok(())), std::future::pending())
            .await;
        assert!(matches!(lease, ReplicaLease::Reaped));

        lease
            .release(
                async { panic!("confirmed reap must not shut down the worker again") },
                std::future::ready(Ok(())),
            )
            .await;
        assert!(matches!(lease, ReplicaLease::Unclaimed));
    }

    #[tokio::test(start_paused = true)]
    async fn retries_shutdown_after_an_error_and_timeout() {
        let mut lease = ReplicaLease::Claimed;
        lease
            .release(
                std::future::ready(Err(anyhow!("worker reap failed"))),
                async { panic!("an unconfirmed reap must not release the durable permit") },
            )
            .await;
        assert!(matches!(lease, ReplicaLease::Claimed));
        lease
            .release(std::future::pending(), async {
                panic!("a timed-out reap must not release the durable permit")
            })
            .await;
        assert!(matches!(lease, ReplicaLease::Claimed));

        lease
            .release(std::future::ready(Ok(())), std::future::ready(Ok(())))
            .await;
        assert!(matches!(lease, ReplicaLease::Unclaimed));
    }

    #[test]
    fn generated_comparison_requires_usage_freshness_and_both_verified_citations() {
        let now = 10_000;
        let source = |id: &str, label: &str, observed_at, value| store::Source {
            opaque_id: id.into(),
            reference: ChatSourceReference::new(ChatSourceKind::HostMetric, "memory".into())
                .unwrap(),
            label: label.into(),
            observed_at: DateTime::from_timestamp_millis(observed_at)
                .unwrap()
                .to_rfc3339(),
            provenance: (id == "prior").then_some(QaEvidenceProvenance::PreviousObservation),
            measurement: Some(QaMeasurement::new(
                value,
                QaMeasurementUnit::PercentTenths,
                "API host memory usage".into(),
            )),
            whole_passage: false,
        };
        let mut request = QaWorkerRequest::new(
            "turn".into(),
            "snapshot".into(),
            "Has that changed?".into(),
            vec![
                QaPassage::new(
                    "current".into(),
                    "The API host memory usage is 12.5%.".into(),
                ),
                QaPassage::new("prior".into(), "The previous observation was 10.0%.".into()),
            ],
        );
        request.dialogue = Some(QaWorkerDialogue {
            kind: QaDialogueKind::Operational,
            history: vec![],
        });
        let mut evidence = FrozenEvidence {
            request,
            sources: vec![
                source("current", "API host memory usage", now, 125),
                source("prior", store::PREVIOUS_OBSERVATION_LABEL, 1_000, 100),
            ],
            sampled_at_ms: now,
        };
        assert!(defer_findings(&evidence));
        let target = QaModelTarget::default();
        let context = Some(QaContextUsage::new(target.input_tokens(), 100, 1));
        let reply = || {
            Some("The API host memory usage is 12.5%. The previous observation was 10.0%.".into())
        };
        let mut publication = Publication::new(target);
        assert!(publication
            .accepted_reply(&evidence, context, reply(), now)
            .is_none());
        for passage in &evidence.request.passages {
            let end = u32::try_from(passage.text.chars().count()).unwrap();
            publication.cited.insert(
                passage.source_id.clone(),
                cited_finding(
                    &evidence,
                    &passage.source_id,
                    0,
                    end,
                    passage.text.clone(),
                    now,
                )
                .unwrap()
                .unwrap(),
            );
        }
        let (_, findings) = publication
            .accepted_reply(&evidence, context, reply(), now)
            .unwrap();
        assert_eq!(findings.len(), 2);
        assert_eq!(findings[0].measurement.as_ref().unwrap().value, 125);
        assert_eq!(findings[1].measurement.as_ref().unwrap().value, 100);
        assert_eq!(findings[1].label, store::PREVIOUS_OBSERVATION_LABEL);
        assert!(publication
            .accepted_reply(&evidence, None, reply(), now)
            .is_none());
        assert_eq!(
            inference_budget(&evidence, now),
            Duration::from_millis(4_500)
        );
        assert!(inference_budget(&evidence, 14_500).is_zero());
        assert!(publication
            .accepted_reply(&evidence, context, reply(), 15_001)
            .is_none());
        evidence.sources[1].provenance = Some(QaEvidenceProvenance::Current);
        assert!(publication
            .accepted_reply(&evidence, context, reply(), now)
            .is_none());
    }

    #[test]
    fn planned_history_retains_observation_time_and_excludes_guidance_from_citations() {
        let now = 20_000;
        let observed = 1_500;
        let observed_at = DateTime::from_timestamp_millis(observed)
            .unwrap()
            .to_rfc3339();
        let mut request = QaWorkerRequest::new(
            "turn".into(),
            "snapshot".into(),
            "When was memory last observed?".into(),
            vec![QaPassage::new("past".into(), "Memory was 56%.".into())],
        );
        request.dialogue = Some(QaWorkerDialogue {
            kind: QaDialogueKind::Operational,
            history: vec![],
        });
        let guidance = QaGuidance::CheckResourceUsage;
        request.plan = Some(QaWorkerPlan::new(
            "memory usage".into(),
            "Present the historical reading and offer the approved resource check.".into(),
            vec!["past".into()],
            Some(guidance),
        ));
        let provenance = QaEvidenceProvenance::Historical {
            sequence: 12,
            coverage_start_ms: 1_000,
            has_gaps: true,
        };
        let mut evidence = FrozenEvidence {
            request,
            sources: vec![store::Source {
                opaque_id: "past".into(),
                reference: ChatSourceReference::new(ChatSourceKind::HostMetric, "memory".into())
                    .unwrap(),
                label: "Historical memory reading".into(),
                observed_at: observed_at.clone(),
                provenance: Some(provenance),
                measurement: None,
                whole_passage: false,
            }],
            sampled_at_ms: now,
        };
        assert!(defer_findings(&evidence));
        let target = QaModelTarget::default();
        let mut publication = Publication::new(target);
        let text = evidence.request.passages[0].text.clone();
        let end = u32::try_from(text.chars().count()).unwrap();
        let citation = cited_finding(&evidence, "past", 0, end, text.clone(), now)
            .unwrap()
            .unwrap();
        publication.cited.insert("past".into(), citation);
        let usage = Some(QaContextUsage::new(target.input_tokens(), 75, 1));
        let reply = format!("{text} {}", guidance.text());
        let (_, findings) = publication
            .accepted_reply(&evidence, usage, Some(reply.clone()), now)
            .unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].text, text);
        assert_eq!(findings[0].observed_at, observed_at);
        assert_eq!(findings[0].provenance, Some(provenance));
        assert!(publication
            .accepted_reply(&evidence, usage, Some(text), now)
            .is_none());
        assert!(publication
            .accepted_reply(&evidence, usage, Some(reply), now + 5_001)
            .is_none());

        evidence.sources[0].provenance = Some(QaEvidenceProvenance::Current);
        evidence.sources[0].observed_at =
            DateTime::from_timestamp_millis(now).unwrap().to_rfc3339();
        assert!(!defer_findings(&evidence));
        evidence.request.plan = None;
        assert!(!defer_findings(&evidence));
    }
}
