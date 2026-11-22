//! Durable native maintenance/tool consumers and the independent index pump.

use std::future::Future;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use argos_osint_api::datasets::DatasetStore;
use argos_osint_core::events::Severity;
use argos_osint_core::osint::manual;
use argos_osint_core::osint::manual::DurableToolOutcome;
use argos_osint_core::storage::classifier_activation;
use argos_osint_core::storage::classifier_activation::ActivationWork;
use argos_osint_core::storage::job_dispatch;
use argos_osint_core::storage::job_dispatch::DeliveryClaim;
use argos_osint_core::storage::job_dispatch::DeliveryFinish;
use argos_osint_core::storage::job_dispatch::DeliveryOutcome;
use argos_osint_core::storage::job_dispatch::Envelope;
use argos_osint_core::storage::jobs;
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

/// In-process or remote service configuration for [`run_with_config`].
pub(crate) struct LocalWorkerConfig {
    pub(crate) broker: Client,
    pub(crate) store: DatasetStore,
    pub(crate) index: Option<ActorIndexWorker>,
    pub(crate) relinquish_on_shutdown: bool,
    pub(crate) classifier_enabled: bool,
}

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
    run_with_config(
        repo,
        workspace,
        LocalWorkerConfig {
            broker,
            store,
            index,
            relinquish_on_shutdown: false,
            classifier_enabled: true,
        },
        signal,
    )
    .await?;
    Ok(())
}

/// Run the durable native maintenance worker lanes until `signal` resolves,
/// returning the count of active leases cleanly relinquished on shutdown.
pub(crate) async fn run_with_config(
    repo: Repository,
    workspace: PathBuf,
    config: LocalWorkerConfig,
    signal: impl Future<Output = ()>,
) -> Result<usize> {
    let LocalWorkerConfig {
        broker,
        store,
        index,
        relinquish_on_shutdown,
        classifier_enabled,
    } = config;
    let reindex = index.is_some();
    let pools: Vec<&'static str> = lanes::Lane::ALL
        .into_iter()
        .filter(|lane| reindex || *lane != lanes::Lane::Reindex)
        .filter(|lane| {
            classifier_enabled || !matches!(lane, lanes::Lane::Train | lanes::Lane::Activate)
        })
        .map(lanes::Lane::pool)
        .collect();
    if !classifier_enabled {
        job_dispatch::configure_pool(repo.db(), setup_jobs::TRAIN_POOL, false).await?;
        classifier_activation::configure_consumer(&repo, false).await?;
    }
    for pool in &pools {
        job_dispatch::ensure_queue_transport(repo.db(), pool).await?;
        job_dispatch::enable_pool(repo.db(), pool).await?;
    }
    for operation in [
        MaintenanceOperation::SeedActors,
        MaintenanceOperation::TrainClassifier,
    ] {
        setup_jobs::enable_operation(
            repo.db(),
            operation,
            operation != MaintenanceOperation::TrainClassifier || classifier_enabled,
        )
        .await?;
    }
    if reindex {
        setup_jobs::enable_operation(repo.db(), MaintenanceOperation::ReindexActors, true).await?;
    }
    let (stop, receive) = watch::channel(false);
    let (routes, receivers) =
        lanes::bounded_for_capabilities::<Received>(reindex, classifier_enabled);
    let tools = Arc::new(manual::Tools::default());
    let relinquished = Arc::new(AtomicUsize::new(0));
    let mut tasks = JoinSet::new();
    let index_interval = if relinquish_on_shutdown {
        // Poll between the writer admission windows so local batches don't
        // miss a full window to scheduling jitter. Write pacing is unchanged.
        Duration::from_millis(250)
    } else {
        Duration::from_secs(1)
    };
    tasks.spawn(index_loop(
        repo.clone(),
        index,
        receive.clone(),
        index_interval,
    ));
    let runtime = LaneRuntime {
        repo: repo.clone(),
        workspace,
        store: store.clone(),
        tools,
        relinquish_on_shutdown,
        relinquished: Arc::clone(&relinquished),
    };
    for (lane, receiver) in receivers {
        tasks.spawn(consume_lane(
            runtime.clone(),
            lane,
            receiver,
            receive.clone(),
        ));
    }
    // Queue transport and the actual bounded activation lane are now owned.
    classifier_activation::configure_consumer(&repo, classifier_enabled).await?;
    tracing::info!(
        role = if relinquish_on_shutdown {
            "local-ipc-worker"
        } else {
            "native-worker"
        },
        worker_pools = pools.len(),
        actor_index_pump = reindex,
        storage = "connected",
        "Fleetworks startup summary: maintenance worker ready"
    );
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
    result.map(|()| relinquished.load(Ordering::Acquire))
}

async fn index_loop(
    repo: Repository,
    mut index: Option<ActorIndexWorker>,
    mut stop: watch::Receiver<bool>,
    interval: Duration,
) -> Result<()> {
    let mut tick = tokio::time::interval(interval);
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = stop.changed() => break,
            _ = tick.tick() => {
                if let Some(index) = index.as_mut() {
                    // The durable lease and persisted body reconcile an uncertain
                    // outcome. Never start another concurrent tick locally.
                    if index.tick(&repo).await.is_err() {
                        tracing::warn!(code = "actor_index_tick_failed", "Actor index coordinator will reconcile durable state");
                    }
                }
            }
        }
        if *stop.borrow() {
            break;
        }
    }
    if let Some(index) = index.as_ref() {
        let _ = index.release(&repo).await;
    }
    Ok(())
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
    let mut last_recovery = Option::<std::time::Instant>::None;
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
                let run_recovery = last_recovery.is_none_or(|at| at.elapsed() >= Duration::from_secs(5));
                let result=async {
                    // Publishing and routing remain independent of every long
                    // operation. Rotate the first pool; one source item per pool.
                    for offset in 0..pools.len() {
                        let pool=pools[(turn+offset)%pools.len()];
                        if run_recovery {
                            job_dispatch::recover_expired(repo.db(),pool,Utc::now()).await?;
                        }
                        if let Some(claim)=job_dispatch::claim_ready(repo.db(),pool,&owner,Utc::now()).await? {
                            let envelope=&claim.envelope;
                            let identity=serde_json::to_vec(&(&envelope.workspace_generation,&envelope.task_id,envelope.revision))?;
                            broker.publish(format!("{:x}",Sha256::digest(identity)),serde_json::to_vec(envelope)?).await?;
                            job_dispatch::mark_published(repo.db(),&claim,Utc::now()).await?;
                        }
                    }
                    if run_recovery {
                        last_recovery = Some(std::time::Instant::now());
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

async fn wait_stop(stop: &mut watch::Receiver<bool>) {
    if !*stop.borrow() {
        let _ = stop.changed().await;
    }
}

#[derive(Clone)]
struct LaneRuntime {
    repo: Repository,
    workspace: PathBuf,
    store: DatasetStore,
    tools: Arc<manual::Tools>,
    relinquish_on_shutdown: bool,
    relinquished: Arc<AtomicUsize>,
}

async fn consume_lane(
    runtime: LaneRuntime,
    lane: lanes::Lane,
    mut receiver: mpsc::Receiver<lanes::Routed<Received>>,
    mut stop: watch::Receiver<bool>,
) -> Result<()> {
    let LaneRuntime {
        repo,
        workspace,
        store,
        tools,
        relinquish_on_shutdown,
        relinquished,
    } = runtime;
    let owner = format!(
        "native-{}-{}",
        lane.pool(),
        argos_osint_core::store::new_id()
    );
    let pools = [lane.pool()];
    let dispatch = DeliveryLane {
        owner: &owner,
        pools: &pools,
    };
    loop {
        let routed = tokio::select! {
            biased;
            _ = wait_stop(&mut stop) => break,
            value = receiver.recv() => value,
        };
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
            &dispatch,
            received,
            &mut stop,
            relinquish_on_shutdown,
            &relinquished,
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
            break;
        }
    }
    // Requeue any buffered unstarted deliveries so shutdown never leaves them invisible.
    receiver.close();
    while let Ok(routed) = receiver.try_recv() {
        let (received, _slots) = routed.into_parts();
        let _ = received.settle(Disposition::Requeue).await;
    }
    Ok(())
}

struct DeliveryLane<'a> {
    owner: &'a str,
    pools: &'a [&'a str],
}

enum WorkerOutcome {
    Complete { result_ref: String },
    Failed { reason: &'static str },
    RetrySeedPressure { reason: &'static str },
    Relinquish { reason: &'static str },
    Stale,
}

#[allow(clippy::too_many_arguments)]
async fn handle(
    repo: &Repository,
    workspace: &std::path::Path,
    store: &DatasetStore,
    tools: &manual::Tools,
    lane: &DeliveryLane<'_>,
    received: Received,
    stop: &mut watch::Receiver<bool>,
    relinquish_on_shutdown: bool,
    relinquished: &AtomicUsize,
) -> Result<()> {
    let envelope = match Envelope::decode(received.payload()) {
        Ok(envelope) if lane.pools.contains(&envelope.pool.as_str()) => envelope,
        _ => {
            received.settle(Disposition::DeadLetter).await?;
            return Ok(());
        }
    };
    let claim = job_dispatch::claim_delivery(
        repo.db(),
        &envelope,
        lane.owner,
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
    // Seed and reindex have durable, idempotent checkpoints on both runtimes.
    // A planned Distributed restart must not turn those checkpoints into a
    // terminal failure or consume a failure retry.
    let shutdown_reason = if relinquish_on_shutdown {
        Some("local_backend_shutdown")
    } else if matches!(lease.task.pool.as_str(), setup_jobs::SEED_POOL | setup_jobs::REINDEX_POOL) {
        Some("native_worker_shutdown")
    } else {
        None
    };
    let shutdown_outcome = || match shutdown_reason {
        Some(reason) => WorkerOutcome::Relinquish { reason },
        None => WorkerOutcome::Failed {
            reason: "native_worker_shutdown",
        },
    };
    let outcome = if *stop.borrow() {
        shutdown_outcome()
    } else {
        crate::setup_jobs::record_job_event(
            repo,
            &lease.task,
            Severity::Info,
            "task.started",
            format!(
                "Worker claimed {} (attempt {})",
                jobs::job_title(&lease.task.operation),
                lease.task.attempts
            ),
        )
        .await;
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
                    relinquish_on_shutdown,
                )
                .await?
                {
                    crate::setup_jobs::SetupOutcome::Complete => Ok(DurableToolOutcome::Complete {
                        result_ref: format!("setup:{}", lease.task.job_id),
                    }),
                    crate::setup_jobs::SetupOutcome::Interrupted => {
                        Ok(DurableToolOutcome::Failed {
                            reason: "setup_interrupted",
                        })
                    }
                }
            }
        };
        tokio::pin!(operation);
        let mut heartbeat = tokio::time::interval(Duration::from_secs(5));
        heartbeat.set_missed_tick_behavior(MissedTickBehavior::Delay);
        heartbeat.tick().await;
        let mut last_renewed = tokio::time::Instant::now();
        let max_renew_gap = Duration::from_secs((LEASE_SECONDS as u64).saturating_sub(10));
        loop {
            tokio::select! {
                biased;
                _ = wait_stop(stop) => {
                    argos_osint_core::cancellation::cancel_with_notify(&cancel);
                    // Retain the operation until its cancellation is observed; a
                    // detached trainer/seed must not outlive its claimed lease.
                    let _ = operation.await;
                    break shutdown_outcome();
                }
                _ = heartbeat.tick() => {
                    let queue_live = received.renew().await;
                    let job_live =
                        job_dispatch::renew_delivery(repo.db(), &lease, LEASE_SECONDS, Utc::now())
                            .await;
                    let lost = matches!(queue_live, Ok(false)) || matches!(job_live, Ok(false));
                    let renewed = matches!(queue_live, Ok(true)) && matches!(job_live, Ok(true));
                    if renewed {
                        last_renewed = tokio::time::Instant::now();
                    } else if lost || last_renewed.elapsed() >= max_renew_gap {
                        argos_osint_core::cancellation::cancel_with_notify(&cancel);
                        let _ = operation.await;
                        break WorkerOutcome::Stale;
                    }
                }
                result = &mut operation => {
                    let durable = match result {
                        Ok(outcome) => outcome,
                        Err(error) => DurableToolOutcome::Failed {
                            reason: crate::setup_jobs::safe_failure_reason(&error),
                        },
                    };
                    break match durable {
                        DurableToolOutcome::Complete { result_ref } => {
                            WorkerOutcome::Complete { result_ref }
                        }
                        DurableToolOutcome::Failed { reason }
                            if shutdown_reason.is_some()
                                && (*stop.borrow()
                                    || matches!(
                                        reason,
                                        "setup_interrupted" | "actor_reindex_interrupted"
                                    )) =>
                        {
                            shutdown_outcome()
                        }
                        DurableToolOutcome::Failed { reason }
                            if relinquish_on_shutdown
                                && lease.task.pool == setup_jobs::SEED_POOL
                                && crate::setup_jobs::is_worker_pressure_failure(reason) =>
                        {
                            WorkerOutcome::RetrySeedPressure { reason }
                        }
                        DurableToolOutcome::Failed { reason } => WorkerOutcome::Failed { reason },
                        DurableToolOutcome::Stale => WorkerOutcome::Stale,
                        _ => return Err(anyhow::anyhow!("native_domain_outcome_unsupported")),
                    };
                }
            }
        }
    };
    let finish = match &outcome {
        WorkerOutcome::Complete { result_ref } => {
            crate::setup_jobs::record_job_event(
                repo,
                &lease.task,
                Severity::Info,
                "task.completed",
                format!("Completed {}", jobs::job_title(&lease.task.operation)),
            )
            .await;
            job_dispatch::finish_delivery(
                repo.db(),
                &lease,
                DeliveryOutcome::Complete { result_ref },
                Utc::now(),
            )
            .await?
        }
        WorkerOutcome::Failed { reason } => {
            crate::setup_jobs::record_job_event(
                repo,
                &lease.task,
                Severity::Error,
                "task.failed",
                format!(
                    "Failed {}: {}",
                    jobs::job_title(&lease.task.operation),
                    reason
                ),
            )
            .await;
            job_dispatch::finish_delivery(
                repo.db(),
                &lease,
                DeliveryOutcome::Failed { reason },
                Utc::now(),
            )
            .await?
        }
        WorkerOutcome::RetrySeedPressure { reason } => {
            // The dispatcher applies the existing finite attempt limit.
            job_dispatch::finish_delivery(
                repo.db(),
                &lease,
                DeliveryOutcome::Retry {
                    reason,
                    delay_secs: 60,
                },
                Utc::now(),
            )
            .await?
        }
        WorkerOutcome::Relinquish { reason } => {
            crate::setup_jobs::record_job_event(
                repo,
                &lease.task,
                Severity::Info,
                "task.relinquished",
                format!(
                    "Relinquished {} on worker shutdown",
                    jobs::job_title(&lease.task.operation)
                ),
            )
            .await;
            let finish = job_dispatch::finish_delivery(
                repo.db(),
                &lease,
                DeliveryOutcome::Relinquish { reason },
                Utc::now(),
            )
            .await?;
            if finish == DeliveryFinish::Relinquished {
                relinquished.fetch_add(1, Ordering::AcqRel);
            }
            finish
        }
        WorkerOutcome::Stale => {
            crate::setup_jobs::record_job_event(
                repo,
                &lease.task,
                Severity::Warn,
                "task.lease_lost",
                format!(
                    "Stopped {}: worker lease expired or job cancelled",
                    jobs::job_title(&lease.task.operation)
                ),
            )
            .await;
            received.settle(Disposition::Ack).await?;
            return Ok(());
        }
    };
    received
        .settle(match finish {
            DeliveryFinish::Failed => Disposition::DeadLetter,
            DeliveryFinish::Relinquished => Disposition::Requeue,
            _ => Disposition::Ack,
        })
        .await?;
    Ok(())
}

#[cfg(all(test, feature = "embedded"))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn relinquish_on_shutdown_preserves_retry_budget_and_resumes_same_job_id() {
        check_reindex_worker_shutdown(true).await;
    }

    #[tokio::test]
    async fn distributed_shutdown_requeues_reindex_under_the_same_job_id() {
        check_reindex_worker_shutdown(false).await;
    }

    // Exercise the worker's transport-specific shutdown policy using isolated
    // local storage and queue; the public HTTP contract is tested separately.
    async fn check_reindex_worker_shutdown(embedded: bool) {
        let _lifecycle_guard = crate::tests::EMBEDDED_LIFECYCLE_TEST_LOCK.lock().await;
        use argos_osint_core::storage::bootstrap;
        use argos_osint_core::storage::connect_registry;
        use argos_osint_core::storage::connect_registry::RegistryAuthority;
        use argos_osint_core::storage::open::Target;
        use argos_osint_core::storage::server;
        use argos_osint_core::storage::tx;
        use fleetworks_contracts::actor_index::ActorIndexLimits;
        use fleetworks_contracts::connect::RegistrationEnsureRequest;
        use fleetworks_contracts::context::JobOrigin;
        use fleetworks_contracts::context::MutationContext;
        use fleetworks_contracts::context::OriginSource;
        use fleetworks_contracts::readiness::MaintenanceRequest;
        use fleetworks_jobs::queue::Queue;
        use fleetworks_search_index::config::LocalEngineConfig;
        use fleetworks_search_index::engine::Engine;
        use fleetworks_search_index::pressure::Meter;
        use fleetworks_search_index::service::EngineSlot;
        use fleetworks_search_index::service::SearchService;

        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path().canonicalize().expect("canonical tempdir");
        let repo = Repository::connect(
            &Target::Embedded {
                path: base.join("surreal"),
            },
            server::NS,
            server::DB,
        )
        .await
        .expect("embedded repo");
        bootstrap::ensure_network_identity(repo.db())
            .await
            .expect("network identity");
        connect_registry::initialize_control(repo.db())
            .await
            .expect("control init");

        let deployment = connect_registry::deployment_state(&repo)
            .await
            .expect("deployment state");
        let queue_handle = Arc::new(Queue::open(base.join("jobs")).await.expect("open queue"));
        let store = DatasetStore::open(base.join("staged-datasets")).expect("dataset store");

        for pool in [
            job_dispatch::ATLAS_POOL,
            manual::TOOLS_POOL,
            setup_jobs::SEED_POOL,
            setup_jobs::TRAIN_POOL,
            setup_jobs::REINDEX_POOL,
            classifier_activation::POOL,
        ] {
            job_dispatch::ensure_queue_transport(repo.db(), pool)
                .await
                .expect("transport");
            job_dispatch::enable_pool(repo.db(), pool)
                .await
                .expect("enable pool");
        }
        setup_jobs::enable_operation(repo.db(), MaintenanceOperation::ReindexActors, true)
            .await
            .expect("enable reindex");

        let authority = RegistryAuthority::new("local-ipc-test", "cred-1", "epoch-1")
            .expect("registry authority");
        let ensured = connect_registry::ensure(
            &repo,
            &authority,
            &RegistrationEnsureRequest::new(
                "reg-relinquish-1".into(),
                None,
                "install-relinquish-1".into(),
            ),
        )
        .await
        .expect("registration");
        let context = connect_registry::registration_context(
            &repo,
            &authority,
            &ensured.receipt.registration_id,
            ensured.receipt.revision,
        )
        .await
        .expect("context");
        let request =
            MaintenanceRequest::for_reindex("reindex-req-1".into(), MutationContext::new(&context))
                .expect("reindex request");
        let origin = JobOrigin::new(
            &context,
            OriginSource::Native,
            "reindex-req-1".into(),
            Utc::now().to_rfc3339(),
            ensured.receipt.revision,
            Vec::new(),
        );
        let receipt = tx::run_tx(repo.db(), tx::RetryPolicy::default(), |transaction| {
            let (authority, context, origin, request) = (&authority, &context, &origin, &request);
            async move {
                setup_jobs::admit_in(
                    &transaction,
                    authority,
                    context,
                    origin,
                    MaintenanceOperation::ReindexActors,
                    request,
                )
                .await
            }
        })
        .await
        .expect("admit reindex")
        .value;

        let search_dir = base.join("search");
        std::fs::create_dir_all(&search_dir).expect("search dir");
        // Other unit tests in the shared `argos-api-host` binary keep embedded RocksDB/Tantivy/model
        // allocations resident; give this lease-relinquishment test headroom above harness RSS.
        let limits = ActorIndexLimits {
            memory_resume_bytes: 2 * 1024 * 1024 * 1024,
            memory_pause_bytes: 4 * 1024 * 1024 * 1024,
            ..Default::default()
        };
        let engine_config = LocalEngineConfig::for_identity(
            search_dir,
            deployment.deployment_id.clone(),
            "source-epoch-1".into(),
            limits.clone(),
        )
        .expect("engine config");

        // Run 1: SearchService runs with a pressure-paused Meter so ReindexActors claims and
        // waits cleanly until shutdown arrives, exercising clean lease relinquishment without
        // triggering error deferral.
        let engine1 = Engine::open(engine_config.engine.clone()).expect("open tantivy engine 1");
        let paused_meter = Arc::new(
            Meter::new(engine_config.engine.directory.clone(), limits).expect("paused meter"),
        );
        paused_meter
            .record_query(Duration::from_secs(10))
            .expect("record slow query pressure");
        let unready_search = Arc::new(
            SearchService::new(
                engine_config.clone(),
                Arc::new(EngineSlot::new(Some(Arc::new(engine1)))),
                "index_recovering",
            )
            .expect("unready search service")
            .with_meter(paused_meter),
        );
        let (stop1_tx, stop1_rx) = tokio::sync::oneshot::channel::<()>();
        let worker1 = tokio::spawn(run_with_config(
            repo.clone(),
            base.clone(),
            LocalWorkerConfig {
                broker: Client::local(Arc::clone(&queue_handle), "fleetworks.native".into()),
                store: store.clone(),
                index: Some(
                    ActorIndexWorker::local(unready_search, "index-owner-1".into())
                        .expect("local index worker"),
                ),
                relinquish_on_shutdown: embedded,
                classifier_enabled: true,
            },
            async move {
                let _ = stop1_rx.await;
            },
        ));

        // Wait until the ReindexActors task is actively running before triggering shutdown.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
        let initial_max_attempts: i64 = loop {
            let mut resp = repo
                .db()
                .query(
                    "SELECT VALUE state FROM ONLY task WHERE job = type::record('job', $job);
                     SELECT VALUE max_attempts FROM ONLY task WHERE job = type::record('job', $job);",
                )
                .bind(("job", receipt.job_id.clone()))
                .await
                .expect("query task")
                .check()
                .expect("check task");
            let state: Option<String> = resp.take(0).expect("take state");
            let max_attempts: Option<i64> = resp.take(1).expect("take max_attempts");
            if state.as_deref() == Some("running") {
                if let Some(max_attempts) = max_attempts {
                    break max_attempts;
                }
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "timed out waiting for task to start running"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        };

        let _ = stop1_tx.send(());
        let relinquished_count = worker1.await.expect("join worker1").expect("worker1 ok");
        assert_eq!(relinquished_count, 1);

        let mut check_resp = repo
            .db()
            .query(
                "SELECT VALUE state FROM ONLY task WHERE job = type::record('job', $job);
                 SELECT VALUE max_attempts FROM ONLY task WHERE job = type::record('job', $job);
                 SELECT VALUE outcome FROM attempt WHERE root = type::record('job', $job);",
            )
            .bind(("job", receipt.job_id.clone()))
            .await
            .expect("inspect relinquished task")
            .check()
            .expect("check relinquished");
        assert_eq!(
            check_resp
                .take::<Option<String>>(0)
                .expect("state")
                .as_deref(),
            Some("queued")
        );
        assert_eq!(
            check_resp.take::<Option<i64>>(1).expect("max_attempts"),
            Some(initial_max_attempts + 1)
        );
        let outcomes: Vec<String> = check_resp.take(2).expect("outcomes");
        assert_eq!(outcomes, vec!["relinquished".to_owned()]);

        // Run 2: restart with a live Tantivy Engine against the exact same stores and finish.
        let engine = Engine::open(engine_config.engine.clone()).expect("open tantivy engine");
        let ready_search = Arc::new(
            SearchService::new(
                engine_config,
                Arc::new(EngineSlot::new(Some(Arc::new(engine)))),
                "index_recovering",
            )
            .expect("ready search service"),
        );
        let (stop2_tx, stop2_rx) = tokio::sync::oneshot::channel::<()>();
        let worker2 = tokio::spawn(run_with_config(
            repo.clone(),
            base,
            LocalWorkerConfig {
                broker: Client::local(queue_handle, "fleetworks.native".into()),
                store,
                index: Some(
                    ActorIndexWorker::local(ready_search, "index-owner-2".into())
                        .expect("local index worker 2"),
                ),
                relinquish_on_shutdown: embedded,
                classifier_enabled: true,
            },
            async move {
                let _ = stop2_rx.await;
            },
        ));

        let complete_deadline = tokio::time::Instant::now() + Duration::from_secs(20);
        loop {
            let mut resp = repo
                .db()
                .query("SELECT VALUE state FROM ONLY type::record('job', $job)")
                .bind(("job", receipt.job_id.clone()))
                .await
                .expect("query job")
                .check()
                .expect("check job");
            if resp.take::<Option<String>>(0).expect("state").as_deref() == Some("completed") {
                break;
            }
            assert!(
                tokio::time::Instant::now() < complete_deadline,
                "timed out waiting for resumed job to complete"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let _ = stop2_tx.send(());
        let _ = worker2.await.expect("join worker2").expect("worker2 ok");

        let mut count_resp = repo
            .db()
            .query("RETURN count(SELECT VALUE id FROM job);")
            .await
            .expect("count jobs")
            .check()
            .expect("check count");
        assert_eq!(count_resp.take::<Option<i64>>(0).expect("count"), Some(1));
    }
}
