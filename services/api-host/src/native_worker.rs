//! Durable native maintenance/tool consumers and the independent index pump.

use std::future::Future;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use argos_osint_api::datasets::DatasetStore;
use argos_osint_core::osint::manual;
use argos_osint_core::osint::manual::DurableToolOutcome;
use argos_osint_core::storage::classifier_activation;
use argos_osint_core::storage::classifier_activation::ActivationWork;
use argos_osint_core::storage::job_dispatch;
use argos_osint_core::storage::job_dispatch::DeliveryClaim;
use argos_osint_core::storage::job_dispatch::DeliveryFinish;
use argos_osint_core::storage::job_dispatch::DeliveryOutcome;
use argos_osint_core::storage::job_dispatch::Envelope;
use argos_osint_core::storage::repo::Repository;
use argos_osint_core::storage::setup_jobs;
use chrono::Utc;
use fleetworks_contracts::readiness::MaintenanceOperation;
use sha2::Digest;
use sha2::Sha256;
use tokio::sync::mpsc;
use tokio::sync::watch;
use tokio::task::JoinSet;
use tokio::time::MissedTickBehavior;

use crate::actor_index_worker::ActorIndexWorker;
use crate::queue_client::Client;
use crate::queue_client::Disposition;
use crate::queue_client::Received;

const LEASE_SECONDS: i64 = 45;

#[path = "native_pool.rs"]
mod lanes;
#[path = "actor_reindex_job.rs"]
mod reindex_job;

pub(crate) async fn run(
    repo: Repository,
    workspace: PathBuf,
    signal: impl Future<Output = ()>,
) -> Result<()> {
    let (broker, store, index) = tokio::task::spawn_blocking({
        let workspace = workspace.clone();
        move || {
            let queue = std::env::var("FLEETWORKS_NATIVE_QUEUE_NAME")
                .unwrap_or_else(|_| "fleetworks.native".into());
            anyhow::ensure!(
                queue != "fleetworks.atlas" && !queue.trim().is_empty() && queue.len() <= 128,
                "native queue must be separate from Atlas"
            );
            let broker = Client::new(
                &std::env::var("FLEETWORKS_JOBS_ORIGIN")?,
                &crate::fleetworks::read_secret("FLEETWORKS_JOBS_TOKEN")?,
                queue,
            )?;
            let store = DatasetStore::open(workspace.join("staged-datasets"))?;
            let index = match std::env::var("FLEETWORKS_SEARCH_ORIGIN") {
                Ok(origin) => Some(ActorIndexWorker::new(
                    &origin,
                    &crate::fleetworks::read_secret("FLEETWORKS_SEARCH_TOKEN")?,
                    &crate::fleetworks::read_secret("FLEETWORKS_SEARCH_WRITE_TOKEN")?,
                    format!("actor-index-{}", argos_osint_core::store::new_id()),
                )?),
                Err(std::env::VarError::NotPresent) => None,
                Err(_) => anyhow::bail!("invalid search origin encoding"),
            };
            Ok::<_, anyhow::Error>((broker, store, index))
        }
    })
    .await??;
    let reindex = index.is_some();
    let pools: Vec<&'static str> = lanes::Lane::ALL
        .into_iter()
        .filter(|lane| reindex || *lane != lanes::Lane::Reindex)
        .map(lanes::Lane::pool)
        .collect();
    for pool in &pools {
        job_dispatch::ensure_queue_transport(repo.db(), pool).await?;
        job_dispatch::enable_pool(repo.db(), pool).await?;
    }
    for operation in [
        MaintenanceOperation::SeedActors,
        MaintenanceOperation::TrainClassifier,
    ] {
        setup_jobs::enable_operation(repo.db(), operation, true).await?;
    }
    if reindex {
        setup_jobs::enable_operation(repo.db(), MaintenanceOperation::ReindexActors, true).await?;
    }
    let (stop, receive) = watch::channel(false);
    let (routes, receivers) = lanes::bounded::<Received>(reindex);
    let tools = Arc::new(manual::Tools::default());
    let mut tasks = JoinSet::new();
    tasks.spawn(index_loop(repo.clone(), index, receive.clone()));
    for (lane, receiver) in receivers {
        tasks.spawn(consume_lane(
            repo.clone(),
            workspace.clone(),
            store.clone(),
            Arc::clone(&tools),
            lane,
            receiver,
            receive.clone(),
        ));
    }
    // Queue transport and the actual bounded activation lane are now owned.
    classifier_activation::configure_consumer(&repo, true).await?;
    let relay_repo = repo.clone();
    let relay_stop = receive.clone();
    tasks.spawn(
        async move { relay(&relay_repo, &store, &broker, &pools, &routes, relay_stop).await },
    );
    tokio::pin!(signal);
    let mut result = tokio::select! {
        _=&mut signal=>Ok(()),
        ended=tasks.join_next()=>match ended {
            Some(Ok(Err(error)))=>Err(error),Some(Err(error))=>Err(error.into()),
            _=>Err(anyhow::anyhow!("native worker lane ended before shutdown")),
        },
    };
    let _ = stop.send(true);
    while let Some(ended) = tasks.join_next().await {
        let ended = ended.map_err(anyhow::Error::from).and_then(|result| result);
        if result.is_ok() {
            result = ended;
        }
    }
    result
}

async fn index_loop(
    repo: Repository,
    mut index: Option<ActorIndexWorker>,
    mut stop: watch::Receiver<bool>,
) -> Result<()> {
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _=stop.changed()=>return Ok(()),
            _=tick.tick()=>{
                if let Some(index)=index.as_mut() {
                    // The durable lease and persisted body reconcile an uncertain
                    // HTTP outcome. Never start another concurrent tick locally.
                    if index.tick(&repo).await.is_err() {tracing::warn!(code="actor_index_tick_failed","Actor index coordinator will reconcile durable state");}
                }
            }
        }
        if *stop.borrow() {
            return Ok(());
        }
    }
}

async fn relay(
    repo: &Repository,
    store: &DatasetStore,
    broker: &Client,
    pools: &[&str],
    routes: &lanes::Routes<Received>,
    mut stop: watch::Receiver<bool>,
) -> Result<()> {
    let owner = format!("native-worker-{}", argos_osint_core::store::new_id());
    let mut turn = 0;
    let mut poll = tokio::time::interval(Duration::from_millis(500));
    poll.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut housekeeping = tokio::time::interval(Duration::from_secs(60));
    housekeeping.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            biased;
            _=stop.changed()=>return Ok(()),
            _=housekeeping.tick()=>{if store.expire_abandoned(repo).await.is_err(){tracing::warn!(code="dataset_expiry_pending","Dataset expiry cleanup remains pending");}},
            _=poll.tick()=>{
                let result=async {
                    // Publishing and routing remain independent of every long
                    // operation. Rotate the first pool; one source item per pool.
                    for offset in 0..pools.len() {
                        let pool=pools[(turn+offset)%pools.len()];
                        job_dispatch::recover_expired(repo.db(),pool,Utc::now()).await?;
                        if let Some(claim)=job_dispatch::claim_ready(repo.db(),pool,&owner,Utc::now()).await? {
                            let envelope=&claim.envelope;
                            let identity=serde_json::to_vec(&(&envelope.workspace_generation,&envelope.task_id,envelope.revision))?;
                            broker.publish(format!("{:x}",Sha256::digest(identity)),serde_json::to_vec(envelope)?).await?;
                            job_dispatch::mark_published(repo.db(),&claim,Utc::now()).await?;
                        }
                    }
                    turn=(turn+1)%pools.len();
                    if let Some(received)=broker.claim(&owner).await? {
                        let pool=match Envelope::decode(received.payload()) {Ok(envelope)=>envelope.pool,Err(_)=>{received.settle(Disposition::DeadLetter).await?;return Ok::<_,anyhow::Error>(());}};
                        match routes.try_send(&pool,received) {
                            Ok(())=>(),
                            Err(lanes::RouteError::Full(received))=>received.settle(Disposition::Requeue).await?,
                            Err(lanes::RouteError::Unknown(received))=>{
                                // A replica without configured search must not
                                // destroy a Reindex delivery for another replica.
                                let known=lanes::Lane::ALL.iter().any(|lane|lane.pool()==pool);
                                received.settle(if known {Disposition::Requeue} else {Disposition::DeadLetter}).await?;
                            },
                        }
                    }
                    Ok::<_,anyhow::Error>(())
                }.await;
                if result.is_err(){tracing::warn!(code="native_dispatch_retry","Durable native dispatch will reconcile after service interruption");}
            }
        }
        if *stop.borrow() {
            return Ok(());
        }
    }
}

async fn consume_lane(
    repo: Repository,
    workspace: PathBuf,
    store: DatasetStore,
    tools: Arc<manual::Tools>,
    lane: lanes::Lane,
    mut receiver: mpsc::Receiver<lanes::Routed<Received>>,
    mut stop: watch::Receiver<bool>,
) -> Result<()> {
    let owner = format!(
        "native-{}-{}",
        lane.pool(),
        argos_osint_core::store::new_id()
    );
    loop {
        let routed =
            tokio::select! {biased;_=stop.changed()=>return Ok(()),value=receiver.recv()=>value};
        let Some(routed) = routed else {
            return Err(anyhow::anyhow!("native lane channel closed"));
        };
        let (received, _slots) = routed.into_parts();
        // Box at the task boundary: seed/train futures retain bounded heap
        // state rather than increasing the executor's normal thread stack.
        if Box::pin(handle(
            &repo,
            &workspace,
            &store,
            &tools,
            &owner,
            received,
            &[lane.pool()],
            &mut stop,
        ))
        .await
        .is_err()
        {
            tracing::warn!(
                code = "native_lane_delivery_retry",
                pool = lane.pool(),
                "Durable native delivery will reconcile its database and queue leases"
            );
        }
        if *stop.borrow() {
            return Ok(());
        }
    }
}

async fn handle(
    repo: &Repository,
    workspace: &std::path::Path,
    store: &DatasetStore,
    tools: &manual::Tools,
    owner: &str,
    received: Received,
    pools: &[&str],
    stop: &mut watch::Receiver<bool>,
) -> Result<()> {
    let envelope = match Envelope::decode(received.payload()) {
        Ok(envelope) if pools.contains(&envelope.pool.as_str()) => envelope,
        _ => {
            received.settle(Disposition::DeadLetter).await?;
            return Ok(());
        }
    };
    let claim = job_dispatch::claim_delivery(
        repo.db(),
        &envelope,
        owner,
        LEASE_SECONDS,
        Utc::now(),
        &argos_osint_core::store::new_id(),
    )
    .await?;
    let lease = match claim {
        DeliveryClaim::Stale => {
            received.settle(Disposition::Ack).await?;
            return Ok(());
        }
        DeliveryClaim::Deferred => {
            received.settle(Disposition::Requeue).await?;
            return Ok(());
        }
        DeliveryClaim::Claimed(lease) => lease,
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let operation = async {
        if lease.task.pool == manual::TOOLS_POOL {
            tools
                .execute_claimed(repo, &lease.task, cancel.clone())
                .await
        } else if lease.task.pool == classifier_activation::POOL {
            match classifier_activation::execute_claimed(
                repo,
                workspace.to_path_buf(),
                &lease.task,
                cancel.clone(),
            )
            .await?
            {
                ActivationWork::Complete { result_ref } => {
                    Ok(DurableToolOutcome::Complete { result_ref })
                }
                ActivationWork::Failed { reason } => Ok(DurableToolOutcome::Failed { reason }),
                ActivationWork::Stale => Ok(DurableToolOutcome::Stale),
                _ => Err(anyhow::anyhow!("activation_outcome_unsupported")),
            }
        } else if lease.task.pool == setup_jobs::REINDEX_POOL {
            reindex_job::execute(repo, &lease.task, &cancel).await
        } else {
            match crate::setup_jobs::execute(
                repo,
                workspace.to_path_buf(),
                store,
                &lease.task,
                cancel.clone(),
            )
            .await?
            {
                crate::setup_jobs::SetupOutcome::Complete => Ok(DurableToolOutcome::Complete {
                    result_ref: format!("setup:{}", lease.task.job_id),
                }),
                crate::setup_jobs::SetupOutcome::Interrupted => Ok(DurableToolOutcome::Failed {
                    reason: "setup_interrupted",
                }),
            }
        }
    };
    tokio::pin!(operation);
    let mut heartbeat = tokio::time::interval(Duration::from_secs(5));
    heartbeat.set_missed_tick_behavior(MissedTickBehavior::Delay);
    heartbeat.tick().await;
    let outcome = loop {
        tokio::select! {
            biased;
            _=stop.changed()=>{
                argos_osint_core::cancellation::cancel_with_notify(&cancel);
                // Retain the operation until its cancellation is observed; a
                // detached trainer/seed must not outlive its claimed lease.
                let _=operation.await;
                break DurableToolOutcome::Failed{reason:"native_worker_shutdown"};
            }
            _=heartbeat.tick()=>{
                let queue_live=received.renew().await;
                let job_live=job_dispatch::renew_delivery(repo.db(),&lease,LEASE_SECONDS,Utc::now()).await;
                if !matches!(queue_live,Ok(true))||!matches!(job_live,Ok(true)) {
                    argos_osint_core::cancellation::cancel_with_notify(&cancel);
                    let _=operation.await;
                    break DurableToolOutcome::Stale;
                }
            }
            result=&mut operation=>break match result {
                Ok(outcome)=>outcome,
                Err(error)=>DurableToolOutcome::Failed{reason:crate::setup_jobs::safe_failure_reason(&error)},
            },
        }
    };
    let finish = match &outcome {
        DurableToolOutcome::Complete { result_ref } => {
            job_dispatch::finish_delivery(
                repo.db(),
                &lease,
                DeliveryOutcome::Complete { result_ref },
                Utc::now(),
            )
            .await?
        }
        DurableToolOutcome::Failed { reason } => {
            job_dispatch::finish_delivery(
                repo.db(),
                &lease,
                DeliveryOutcome::Failed { reason },
                Utc::now(),
            )
            .await?
        }
        DurableToolOutcome::Stale => {
            received.settle(Disposition::Ack).await?;
            return Ok(());
        }
        _ => return Err(anyhow::anyhow!("native_domain_outcome_unsupported")),
    };
    received
        .settle(if finish == DeliveryFinish::Failed {
            Disposition::DeadLetter
        } else {
            Disposition::Ack
        })
        .await?;
    Ok(())
}
