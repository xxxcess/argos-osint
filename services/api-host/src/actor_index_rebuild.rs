//! Shadow lifecycle commands use the same leased, sequential HTTP owner.

use argos_osint_core::storage::actor_search_outbox;
use argos_osint_core::storage::actor_search_outbox::RebuildAction;
use argos_osint_core::storage::repo::Repository;
use argos_osint_core::storage::tx::TxError;
use chrono::Utc;
use fleetworks_contracts::actor_index::ActorIndexGeneration;
use fleetworks_contracts::actor_index::ActorIndexStatus;
use reqwest::header;
use serde::de::DeserializeOwned;
use serde::Serialize;

use super::decode;
use super::ActorIndexWorker;
use super::DeliveryFailure;
use super::TickReport;

impl ActorIndexWorker {
    async fn command<T: Serialize + ?Sized, R: DeserializeOwned + Send + 'static>(
        &self,
        path: &str,
        request: &T,
    ) -> Result<R, DeliveryFailure> {
        let response = self
            .http
            .post(self.endpoint(path))
            .header(header::AUTHORIZATION, self.writer.clone())
            .json(request)
            .send()
            .await
            .map_err(|_| DeliveryFailure::transient())?;
        decode(response).await
    }

    pub(super) async fn rebuild_tick(
        &mut self,
        repo: &Repository,
        action: RebuildAction,
    ) -> anyhow::Result<TickReport> {
        let Some(lease) = action.lease().cloned() else {
            return Ok(TickReport::Waiting);
        };
        let result = {
            let operation = async {
                match &action {
                    RebuildAction::Waiting => unreachable!("waiting handled before network"),
                    RebuildAction::Start { request, .. } => {
                        let generation: ActorIndexGeneration = self
                            .command("/internal/v1/actors/rebuild/start", request)
                            .await?;
                        Ok(RebuildReceipt::Started(generation))
                    }
                    RebuildAction::Repair { request, .. } => {
                        let status: ActorIndexStatus = self
                            .command("/internal/v1/actors/rebuild/repair", request)
                            .await?;
                        Ok(RebuildReceipt::Repaired(Box::new(status)))
                    }
                    RebuildAction::Deliver { batch, .. } => {
                        self.deliver(batch).await.map(RebuildReceipt::Batch)
                    }
                    RebuildAction::Promote { request, .. } => {
                        let generation: ActorIndexGeneration = self
                            .command("/internal/v1/actors/rebuild/promote", request)
                            .await?;
                        Ok(RebuildReceipt::Promoted(generation))
                    }
                }
            };
            tokio::pin!(operation);
            let mut heartbeat = tokio::time::interval(std::time::Duration::from_secs(10));
            heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            heartbeat.tick().await;
            loop {
                tokio::select! {
                    result=&mut operation=>break result,
                    _=heartbeat.tick()=>{
                        if let Err(error)=actor_search_outbox::renew(repo,&lease,Utc::now()).await {
                            tracing::warn!(stage="rebuild_lease_renew",code=%super::tx_error_code(&error),"Actor index lease renewal failed");
                            return Err(error.into());
                        }
                    },
                }
            }
        };
        match result {
            Ok(receipt) => {
                // Invalidate the coalesced status only on lifecycle changes.
                // Normal shadow batches update the existing immutable view.
                let acknowledgement = match receipt {
                    RebuildReceipt::Started(generation) => {
                        let result = actor_search_outbox::rebuild_started(
                            repo,
                            &lease,
                            &generation,
                            Utc::now(),
                        )
                        .await;
                        if let Some((_, status)) = &mut self.status {
                            status.rebuilding = Some(generation);
                        }
                        result
                    }
                    RebuildReceipt::Repaired(status) => {
                        let result = actor_search_outbox::rebuild_repaired(
                            repo,
                            &lease,
                            &status,
                            Utc::now(),
                        )
                        .await;
                        self.status = None;
                        result
                    }
                    RebuildReceipt::Batch(receipt) => {
                        let result = actor_search_outbox::rebuild_acknowledge(
                            repo,
                            &lease,
                            &receipt,
                            Utc::now(),
                        )
                        .await;
                        if let Some((_, status)) = &mut self.status {
                            if let Some(shadow) = &mut status.rebuilding {
                                shadow.visible_checkpoint = receipt.visible_checkpoint;
                                shadow.committed_checkpoint = receipt.committed_checkpoint;
                                shadow.writer_epoch = receipt.writer_epoch;
                                if let RebuildAction::Deliver { batch, .. } = &action {
                                    shadow.source_complete = batch.source_complete;
                                }
                            }
                        }
                        result
                    }
                    RebuildReceipt::Promoted(generation) => {
                        let result = actor_search_outbox::rebuild_promoted(
                            repo,
                            &lease,
                            &generation,
                            Utc::now(),
                        )
                        .await;
                        self.status = None;
                        result
                    }
                };
                if let Err(error) = acknowledgement {
                    self.status = None;
                    tracing::warn!(stage="rebuild_acknowledge",code=%super::tx_error_code(&error),"Actor index receipt acknowledgement failed");
                    // A lost lease or storage failure leaves the exact body
                    // pending for a newly fenced attempt. Validation failures
                    // still require a reviewed correction.
                    let lease_lost =
                        matches!(&error,TxError::Abort(reason) if reason=="actor_index_lease_lost");
                    if !lease_lost {
                        if let Err(defer_error) = actor_search_outbox::defer(
                            repo,
                            &lease,
                            None,
                            super::rebuild_ack_requires_correction(&error),
                            Utc::now(),
                        )
                        .await
                        {
                            tracing::warn!(stage="rebuild_defer",code=%super::tx_error_code(&defer_error),"Actor index retry scheduling failed");
                        }
                    }
                    return Err(error.into());
                }
                Ok(TickReport::Waiting)
            }
            Err(error) => {
                self.status = None;
                tracing::warn!(
                    stage = "rebuild_http",
                    status = error
                        .status
                        .map(|status| status.as_u16())
                        .unwrap_or_default(),
                    code = error.code.as_deref().unwrap_or("no_service_code"),
                    correction_required = error.permanent,
                    "Actor index request failed"
                );
                actor_search_outbox::defer(
                    repo,
                    &lease,
                    error.retry_after,
                    error.permanent,
                    Utc::now(),
                )
                .await?;
                Ok(TickReport::Deferred {
                    correction_required: error.permanent,
                })
            }
        }
    }
}

enum RebuildReceipt {
    Started(ActorIndexGeneration),
    Repaired(Box<ActorIndexStatus>),
    Batch(fleetworks_contracts::actor_index::ActorIndexReceipt),
    Promoted(ActorIndexGeneration),
}
