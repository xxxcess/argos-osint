//! Dedicated durable Jobs consumer for the Embedded Assistant.

use std::collections::BTreeSet;
use std::future::Future;
use std::time::Duration;

use anyhow::{anyhow, Result};
use argos_osint_api::inbox_qa::{
    validated_summary_finding, CancellationToken, QaInference, QaRuntimeAvailability,
};
use argos_osint_core::storage::inbox_qa::{self as store, AnswerMode, FrozenEvidence, POOL};
use argos_osint_core::storage::job_dispatch::{
    self, DeliveryClaim, DeliveryFinish, DeliveryLease, DeliveryOutcome, Envelope,
};
use argos_osint_core::storage::repo::Repository;
use chrono::DateTime;
use chrono::Utc;
use fleetworks_contracts::chat::ChatSourceKind;
use fleetworks_contracts::chat::QaGeneratorPreference;
use fleetworks_contracts::chat::QaModelTarget;
use fleetworks_contracts::chat::QA_MAX_FINDINGS;
use fleetworks_contracts::chat::QA_MAX_WORDS;
use fleetworks_contracts::chat::{QaAvailabilityReason, QaFinding, QaStartupStage, QaTurnState};
use fleetworks_contracts::qa_worker::{
    verified_operational_sources, QaContextUsage, QaDialogueKind, QaWorkerEvent, QaWorkerOutcome,
    QaWorkerSequence, QA_CONTEXT_WINDOW_TOKENS,
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
const WARM_RETRY: Duration = Duration::from_secs(10);

struct Executor {
    repo: Repository,
    broker: Client,
    model: QaWorkerSupervisor,
    owner: String,
    placement: String,
    availability: Option<QaRuntimeAvailability>,
    lease_held: bool,
}

pub(crate) async fn run(
    repo: Repository,
    broker: Client,
    model: QaWorkerSupervisor,
    placement: String,
    availability: Option<QaRuntimeAvailability>,
    signal: impl Future<Output = ()>,
) -> Result<()> {
    let mut executor = Executor {
        repo,
        broker,
        model,
        owner: format!("assistant-qa-{}", argos_osint_core::store::new_id()),
        placement,
        availability,
        lease_held: false,
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
            if setting.enabled && loaded.is_none() && Instant::now() >= next_warm {
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
                        self.lease_held = true;
                        if self
                            .warm(setting.generation, setting.generator_preference, stop)
                            .await
                        {
                            loaded = Some(setting.generation);
                            self.record_availability(setting.generation, None);
                            tracing::info!(qa_replica_stage = "ready", "Inbox QA replica ready");
                        } else {
                            if let Some(availability) = &self.availability {
                                availability.clear_stage(setting.generation);
                            }
                            self.unload().await;
                        }
                    }
                    Ok(Ok(store::ReplicaClaim::ConfirmReap)) => {
                        self.lease_held = true;
                        self.unload().await;
                    }
                    Ok(Ok(store::ReplicaClaim::Wait)) => {}
                    _ => tracing::warn!(
                        qa_replica_stage = "replica_claim",
                        "Inbox QA replica could not claim startup"
                    ),
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
                        self.unload().await;
                        loaded = None;
                        next_warm = Instant::now() + WARM_RETRY;
                    }
                }
                Ok(Ok(None)) => {}
                _ => {
                    tracing::warn!(qa_queue_stage = "claim", "Inbox QA queue claim unavailable");
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
        if !std::mem::take(&mut self.lease_held) {
            return;
        }
        if !matches!(
            timeout(STOP_TIMEOUT, self.model.shutdown()).await,
            Ok(Ok(()))
        ) {
            tracing::warn!(
                qa_replica_stage = "reap_confirmation",
                "Inbox QA model reap is unconfirmed"
            );
            return;
        }
        if !matches!(
            timeout(
                AUTH_TIMEOUT,
                store::forget_replica(&self.repo, &self.placement, &self.owner),
            )
            .await,
            Ok(Ok(()))
        ) {
            tracing::warn!(
                qa_replica_stage = "reap_record",
                "Inbox QA reap confirmation could not be recorded"
            );
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
        let evidence = match timeout(AUTH_TIMEOUT, store::start(&self.repo, &lease)).await {
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
        let result = self
            .infer(&evidence, &lease, &delivery, generation, stop)
            .await;
        let (outcome, keep_warm) = match result {
            RunResult::Completed | RunResult::CompletedReset => (
                DeliveryOutcome::Complete {
                    result_ref: "qa_terminal",
                },
                matches!(result, RunResult::Completed),
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
        let (sender, mut receiver) = mpsc::channel(4);
        let cancel = CancellationToken::new();
        let inference: argos_osint_api::inbox_qa::QaInferenceFuture<'_> = match evidence.answer_mode
        {
            AnswerMode::Model => self
                .model
                .ask(evidence.request.clone(), sender, cancel.clone()),
            AnswerMode::Grounded => Box::pin(grounded_events(
                evidence,
                sender,
                Utc::now().timestamp_millis(),
            )),
        };
        tokio::pin!(inference);
        let mut next_check = Instant::now() + CHECK;
        let deadline = Instant::now() + TURN_TIMEOUT;
        // Only this executor can replace the generator while it owns the delivery lease.
        let model_target = match evidence.answer_mode {
            AnswerMode::Model => self.model.active_model().await,
            AnswerMode::Grounded => None,
        };
        let mut publication = Publication {
            model_target,
            ..Publication::default()
        };
        let mut model_finished = false;
        let mut received_event = false;
        let result = loop {
            tokio::select! {
                biased;
                () = stop.cancelled() => break RunResult::Stopped,
                () = tokio::time::sleep_until(deadline) => {
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
                result = &mut inference, if !model_finished => {
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
        if !model_finished {
            if !matches!(result, RunResult::Completed) {
                cancel.cancel();
            }
            let _ = timeout(STOP_TIMEOUT, inference).await;
        }
        if matches!(result, RunResult::Error)
            && evidence.request.dialogue.is_some()
            && !stop.is_cancelled()
            && matches!(
                timeout(
                    AUTH_TIMEOUT,
                    store::publish(&self.repo, lease, QaTurnState::InsufficientContext, None),
                )
                .await,
                Ok(Ok(true))
            )
        {
            return RunResult::CompletedReset;
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
            store::beat(&self.repo, &self.placement, &self.owner, generation, true),
            store::is_live(&self.repo, lease),
            job_dispatch::renew_delivery(self.repo.db(), lease, DELIVERY_SECS, Utc::now()),
            delivery.renew(),
        );
        Ok(permitted? && live? && dispatch? && queue?)
    }
}

/// Publish the exact same validated event protocol used by the inference worker.
async fn grounded_events(
    evidence: &FrozenEvidence,
    sender: mpsc::Sender<QaWorkerEvent>,
    now_ms: i64,
) -> Result<()> {
    let turn_id = &evidence.request.turn_id;
    let (sequence, outcome) = match evidence.request.passages.as_slice() {
        [passage] if observation_is_fresh(evidence.sampled_at_ms, now_ms) => {
            sender
                .send(QaWorkerEvent::Finding {
                    turn_id: turn_id.clone(),
                    sequence: 1,
                    source_id: passage.source_id.clone(),
                    start: 0,
                    end: u32::try_from(passage.text.chars().count())?,
                    text: passage.text.clone(),
                })
                .await?;
            (2, QaWorkerOutcome::Answered)
        }
        [] | [_] => (1, QaWorkerOutcome::InsufficientContext),
        _ => return Err(anyhow!("grounded Inbox QA turn has multiple passages")),
    };
    sender
        .send(QaWorkerEvent::Complete {
            turn_id: turn_id.clone(),
            sequence,
            outcome,
            context: Some(QaContextUsage::new(QA_CONTEXT_WINDOW_TOKENS, 0, 0)),
            reply: None,
        })
        .await?;
    Ok(())
}

enum RunResult {
    More,
    Completed,
    CompletedReset,
    Stopped,
    Fenced,
    Error,
}

fn cited_finding(
    evidence: &FrozenEvidence,
    source_id: &str,
    start: u32,
    end: u32,
    text: String,
) -> Result<Option<QaFinding>> {
    let source = evidence
        .sources
        .iter()
        .find(|source| source.opaque_id == source_id)
        .ok_or_else(|| anyhow!("unknown Inbox QA source"))?;
    let now = Utc::now().timestamp_millis();
    if !observation_is_fresh(evidence.sampled_at_ms, now)
        || !DateTime::parse_from_rfc3339(&source.observed_at)
            .is_ok_and(|at| observation_is_fresh(at.timestamp_millis(), now))
    {
        return Ok(None);
    }
    if !source.whole_passage {
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

fn readiness_fallback(evidence: &FrozenEvidence, now_ms: i64) -> Vec<QaFinding> {
    if !matches!(evidence.answer_mode, AnswerMode::Model)
        || !observation_is_fresh(evidence.sampled_at_ms, now_ms)
    {
        return Vec::new();
    }

    let mut candidates = Vec::new();
    for source in &evidence.sources {
        if source.reference.kind != ChatSourceKind::Readiness || !source.whole_passage {
            continue;
        }
        let Ok(at) = DateTime::parse_from_rfc3339(&source.observed_at) else {
            continue;
        };
        if !observation_is_fresh(at.timestamp_millis(), now_ms) {
            continue;
        }
        let Some(passage) = evidence
            .request
            .passages
            .iter()
            .find(|passage| passage.source_id == source.opaque_id)
        else {
            continue;
        };
        let Ok(end) = u32::try_from(passage.text.chars().count()) else {
            continue;
        };
        let Some(complete) =
            validated_summary_finding(&passage.text, &source.label, 0, end, &passage.text)
        else {
            continue;
        };
        let Some(state) = complete.strip_prefix(source.label.as_str()) else {
            continue;
        };
        let priority = if state.starts_with(" is ready.") {
            1
        } else if state.starts_with(" readiness is not applicable.") {
            2
        } else {
            0
        };
        candidates.push((priority, source, complete, end));
    }
    candidates.sort_by_key(|(priority, _, _, _)| *priority);

    let mut remaining_words = QA_MAX_WORDS;
    candidates
        .into_iter()
        .filter_map(|(_, source, complete, end)| {
            remaining_words = remaining_words.checked_sub(complete.split_whitespace().count())?;
            Some(source.finding(complete.to_owned(), 0, end))
        })
        .take(QA_MAX_FINDINGS)
        .collect()
}

#[derive(Default)]
struct Publication {
    model_target: Option<QaModelTarget>,
    sequence: QaWorkerSequence,
    has_finding: bool,
    cited_sources: BTreeSet<String>,
}

impl Publication {
    async fn publish(
        &mut self,
        repo: &Repository,
        lease: &DeliveryLease,
        evidence: &FrozenEvidence,
        event: QaWorkerEvent,
    ) -> Result<RunResult> {
        self.sequence.accept(&evidence.request, &event)?;
        let (state, finding, context, reply) = match event {
            QaWorkerEvent::Delta { text, .. } => {
                if matches!(evidence.request.dialogue.as_ref(), Some(dialogue) if dialogue.kind == QaDialogueKind::Operational)
                    && !observation_is_fresh(evidence.sampled_at_ms, Utc::now().timestamp_millis())
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
                let Some(finding) = cited_finding(evidence, &source_id, start, end, text)? else {
                    return Ok(RunResult::More);
                };
                self.cited_sources.insert(source_id);
                (QaTurnState::Running, Some(finding), None, None)
            }
            QaWorkerEvent::Complete {
                outcome,
                context,
                reply,
                ..
            } => {
                let now = Utc::now().timestamp_millis();
                let reply = reply.filter(|text| {
                    if !evidence
                        .request
                        .dialogue
                        .as_ref()
                        .is_some_and(|dialogue| dialogue.kind == QaDialogueKind::Operational)
                    {
                        return true;
                    }
                    observation_is_fresh(evidence.sampled_at_ms, now)
                        && verified_operational_sources(&evidence.request, text).is_some_and(
                            |passages| {
                                passages.iter().all(|passage| {
                                    self.cited_sources.contains(&passage.source_id)
                                        && evidence.sources.iter().any(|source| {
                                            source.opaque_id == passage.source_id
                                                && DateTime::parse_from_rfc3339(&source.observed_at)
                                                    .is_ok_and(|at| {
                                                        observation_is_fresh(
                                                            at.timestamp_millis(),
                                                            now,
                                                        )
                                                    })
                                        })
                                })
                            },
                        )
                });
                let had_finding = self.has_finding;
                if !had_finding
                    && matches!(
                        outcome,
                        QaWorkerOutcome::Answered | QaWorkerOutcome::InsufficientContext
                    )
                {
                    // Keep the worker sequence intact and publish complete observations
                    // through the same lease checks as streamed findings.
                    for finding in readiness_fallback(evidence, Utc::now().timestamp_millis()) {
                        let now_ms = Utc::now().timestamp_millis();
                        if !observation_is_fresh(evidence.sampled_at_ms, now_ms) {
                            break;
                        }
                        if !DateTime::parse_from_rfc3339(&finding.observed_at)
                            .is_ok_and(|at| observation_is_fresh(at.timestamp_millis(), now_ms))
                        {
                            continue;
                        }
                        if !store::publish(repo, lease, QaTurnState::Running, Some(finding)).await?
                        {
                            return Ok(RunResult::Fenced);
                        }
                        self.has_finding = true;
                    }
                }
                let state = match outcome {
                    QaWorkerOutcome::Answered if self.has_finding || reply.is_some() => {
                        QaTurnState::Answered
                    }
                    QaWorkerOutcome::InsufficientContext if !had_finding && self.has_finding => {
                        QaTurnState::Answered
                    }
                    QaWorkerOutcome::Answered | QaWorkerOutcome::InsufficientContext => {
                        QaTurnState::InsufficientContext
                    }
                    QaWorkerOutcome::Failed => QaTurnState::Failed,
                    QaWorkerOutcome::Cancelled => QaTurnState::Canceled,
                    _ => return Err(anyhow!("unsupported Inbox QA outcome")),
                };
                (state, None, context, reply)
            }
            _ => return Err(anyhow!("unsupported Inbox QA event")),
        };
        self.has_finding |= finding.is_some();
        let result = store::ModelResult {
            context,
            reply,
            model_target: self.model_target,
        };
        if !store::publish_with_model_result(repo, lease, state, finding, result).await? {
            return Ok(RunResult::Fenced);
        }
        Ok(if state.is_terminal() {
            RunResult::Completed
        } else {
            RunResult::More
        })
    }
}

#[cfg(test)]
mod tests {
    use fleetworks_contracts::chat::ChatSourceReference;
    use fleetworks_contracts::qa_worker::QaPassage;
    use fleetworks_contracts::qa_worker::QaWorkerRequest;

    use super::*;

    #[test]
    fn summary_fallback_keeps_complete_fresh_readiness_facts_in_priority_order() {
        let now = 10_000;
        let rows = [
            (
                "Gateway",
                "Gateway is ready.",
                "gateway",
                ChatSourceKind::Readiness,
                now,
            ),
            (
                "Actor catalog",
                "Actor catalog is not ready. No seed was verified — check setup.",
                "actor_catalog",
                ChatSourceKind::Readiness,
                now,
            ),
            (
                "Classifier service",
                "Classifier service readiness is unknown.",
                "classifier",
                ChatSourceKind::Readiness,
                now,
            ),
            (
                "Brain service",
                "Brain service readiness is unknown.",
                "brain",
                ChatSourceKind::Readiness,
                now - store::MAX_EVIDENCE_AGE_MS - 1,
            ),
            (
                "Actor search",
                "Actor search is degraded.",
                "actor_search",
                ChatSourceKind::Readiness,
                now,
            ),
            (
                "API host CPU usage",
                "API host CPU usage is 11.0%.",
                "cpu",
                ChatSourceKind::HostMetric,
                now,
            ),
        ];
        let sources = rows
            .iter()
            .map(|(label, _, id, kind, observed_at)| store::Source {
                opaque_id: (*id).into(),
                reference: ChatSourceReference::new(*kind, (*id).into()).unwrap(),
                label: (*label).into(),
                observed_at: DateTime::from_timestamp_millis(*observed_at)
                    .unwrap()
                    .to_rfc3339(),
                measurement: None,
                whole_passage: true,
            })
            .collect();
        let passages = rows
            .iter()
            .map(|(_, text, id, _, _)| QaPassage::new((*id).into(), (*text).into()))
            .collect();
        let evidence = FrozenEvidence {
            answer_mode: AnswerMode::Model,
            notice: None,
            conversation: None,
            request: QaWorkerRequest::new(
                "turn".into(),
                "snapshot".into(),
                "Summarize workstation health.".into(),
                passages,
            ),
            sources,
            sampled_at_ms: now,
        };

        let findings = readiness_fallback(&evidence, now);
        assert_eq!(findings.len(), QA_MAX_FINDINGS);
        for (finding, index) in findings.iter().zip([1, 2, 4]) {
            assert_eq!(finding.source.id, rows[index].2);
            assert_eq!(finding.text, rows[index].1);
            assert_eq!(finding.start, 0);
            assert_eq!(finding.end as usize, finding.text.chars().count());
        }
        assert!(readiness_fallback(&evidence, now + store::MAX_EVIDENCE_AGE_MS + 1).is_empty());
    }

    #[tokio::test]
    async fn grounded_turn_publishes_fresh_facts_and_withholds_stale_ones() {
        let evidence = FrozenEvidence {
            answer_mode: AnswerMode::Grounded,
            notice: None,
            conversation: None,
            request: QaWorkerRequest::new(
                "turn".into(),
                "snapshot".into(),
                "Is the actor catalog ready?".into(),
                vec![QaPassage::new(
                    "catalog".into(),
                    "Actor catalog is not ready.".into(),
                )],
            ),
            sources: Vec::new(),
            sampled_at_ms: 10_000,
        };
        let (sender, mut receiver) = mpsc::channel(2);
        grounded_events(&evidence, sender, 10_001).await.unwrap();
        let mut sequence = QaWorkerSequence::default();
        let finding = receiver.recv().await.unwrap();
        assert!(
            matches!(&finding, QaWorkerEvent::Finding { text, .. } if text == "Actor catalog is not ready.")
        );
        assert!(!sequence.accept(&evidence.request, &finding).unwrap());
        let terminal = receiver.recv().await.unwrap();
        assert!(sequence.accept(&evidence.request, &terminal).unwrap());
        assert!(matches!(
            terminal,
            QaWorkerEvent::Complete {
                outcome: QaWorkerOutcome::Answered,
                context: Some(context), ..
            } if context == QaContextUsage::new(QA_CONTEXT_WINDOW_TOKENS, 0, 0)
        ));

        let (sender, mut receiver) = mpsc::channel(2);
        grounded_events(&evidence, sender, 15_001).await.unwrap();
        let terminal = receiver.recv().await.unwrap();
        assert!(QaWorkerSequence::default()
            .accept(&evidence.request, &terminal)
            .unwrap());
        assert!(matches!(
            terminal,
            QaWorkerEvent::Complete {
                outcome: QaWorkerOutcome::InsufficientContext,
                context: Some(context), ..
            } if context == QaContextUsage::new(QA_CONTEXT_WINDOW_TOKENS, 0, 0)
        ));
        assert!(receiver.recv().await.is_none());
    }
}
