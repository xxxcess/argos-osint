mod actor_index_worker;
mod actor_seed;
mod atlas_bert_admin;
mod atlas_worker;
mod classifier_service;
mod execution_service;
mod fleetworks;
mod inbox_qa_service;
mod jobs_service;
mod local_ipc;
mod native_worker;
mod nlp_service;
mod qa_worker;
mod queue_client;
mod reset_services;
mod setup_jobs;

use anyhow::{ensure, Result};
use argos_osint_core::storage::{self, home::Layout};
use clap::{Parser, ValueEnum};
use std::{
    future::IntoFuture,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Arc,
    time::Duration,
};

#[derive(Parser)]
struct Args {
    #[arg(long, default_value_t = 8080)]
    port: u16,
    #[arg(
        long,
        default_value = "http://localhost:3000,http://127.0.0.1:3000",
        value_delimiter = ','
    )]
    origins: Vec<String>,
    #[arg(long, value_enum, default_value_t = Role::Combined)]
    role: Role,
    /// Private workspace actor CSV snapshot.
    #[arg(long, required_if_eq("role", "seed-actors"))]
    file: Option<std::path::PathBuf>,
    #[arg(long, required_if_eq("role", "seed-actors"))]
    expected_sha256: Option<String>,
    /// Digest of the frozen Atlas BERT candidate in the classifier workspace.
    #[arg(long, required_if_eq("role", "atlas-bert-activate"))]
    candidate_sha256: Option<String>,
    /// Digest of the candidate's qualified held-out evaluation.
    #[arg(long, required_if_eq("role", "atlas-bert-activate"))]
    evaluation_sha256: Option<String>,
    /// Exact prior revision returned by --role atlas-bert-revision.
    #[arg(long, required_if_eq("role", "atlas-bert-activate"))]
    expected_bert_revision: Option<String>,
    /// Digest returned by --role atlas-bert-provider-mapping after operator review.
    #[arg(long, required_if_eq("role", "atlas-bert-recalibrate"))]
    reviewed_mapping_sha256: Option<String>,
    /// Completed Atlas cycle ID. Repeat to calibrate on several cycles.
    #[arg(long = "review-cycle")]
    review_cycles: Vec<String>,
    /// Bounded JSON file from an Atlas BERT review export, labeled by a reviewer.
    #[arg(long, required_if_eq("role", "atlas-bert-review-import"))]
    review_file: Option<std::path::PathBuf>,
    /// Trusted operator's external review reference.
    #[arg(long, required_if_eq("role", "atlas-bert-review-import"))]
    review_reference: Option<String>,
    /// Private state directory for `--role local-ipc` (enforced `0o700`).
    #[arg(long, required_if_eq("role", "local-ipc"))]
    state_dir: Option<std::path::PathBuf>,
    /// Optional canonical absolute path to a supervised NLP helper command.
    #[arg(long)]
    nlp_cmd: Option<std::path::PathBuf>,
    /// Optional canonical absolute path to a supervised classifier helper command.
    #[arg(long)]
    classifier_cmd: Option<std::path::PathBuf>,
    /// Operator-chosen label for this backend, shown to operators. Bootstrap only. Leaving it out
    /// keeps any stored label. Up to 64 printable characters; never derived from the host.
    #[arg(long)]
    platform_display_name: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum Role {
    /// Existing local API host with its background workers.
    Combined,
    /// Run the single-process embedded backend over stdin/stdout IPC without listeners.
    LocalIpc,
    /// Authenticated Fleetworks API process.
    Api,
    /// Serve retained Brain and Intel HTTP operations on the private network.
    Brain,
    /// Execute manual tools on the private network.
    Tools,
    /// Read and update workspace configuration on the private network.
    Control,
    /// Serve Jobs and Logs REST operations on the private network.
    Jobs,
    /// Fleetworks background domain workers.
    Worker,
    /// Execute and publish durable Focus Brief stages.
    BriefWorker,
    /// Run private single-attempt model execution for the Brief worker.
    Execution,
    /// Claim and execute durable Atlas dispatches.
    AtlasWorker,
    /// Execute private, durable Inbox QA work on the selected deployment.
    AssistantQa,
    /// Consume registered setup/tools Jobs and pump the private actor index.
    NativeWorker,
    /// Warm, private Compromise inference without a database session.
    Nlp,
    /// Serve pinned Rust-BERT inference without a database session.
    Classifier,
    /// Print the exact active revision from the Atlas classifier workspace.
    AtlasBertRevision,
    /// Explicitly activate a qualified, staged Atlas BERT candidate.
    AtlasBertActivate,
    /// Print the news-provider category mapping rules for operator review.
    AtlasBertProviderMapping,
    /// Export a completed Atlas cycle for independent human review.
    AtlasBertReviewExport,
    /// Import human labels for a completed Atlas cycle.
    AtlasBertReviewImport,
    /// Recalibrate the exact active Atlas BERT revision using reviewed provider mappings.
    AtlasBertRecalibrate,
    /// Seed the Brain actor catalog from a pinned workspace CSV.
    SeedActors,
    /// Initialize storage and provision the runtime database account.
    Bootstrap,
}

fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        tracing_subscriber::EnvFilter::new(
            "warn,argos_api_host=info,argos_osint_api=info,argos_osint_core=warn",
        )
    });

    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(true)
        .try_init();
}

fn ensure_platform_name_is_bootstrap_only(args: &Args) -> Result<()> {
    ensure!(
        args.platform_display_name.is_none() || args.role == Role::Bootstrap,
        "--platform-display-name requires --role bootstrap"
    );
    Ok(())
}

fn configure_embedded_allocator() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        // Share arenas across the short-lived import and search allocations.
        // Keep large buffers separately mapped so dropping them returns their
        // pages even when smaller, long-lived allocations share an arena.
        const ARENA_LIMIT: libc::c_int = 2;
        const MMAP_THRESHOLD_BYTES: libc::c_int = 128 * 1024;
        // SAFETY: Called once on the main thread before the runtime starts any workers.
        let (arenas, mappings) = unsafe {
            (
                libc::mallopt(libc::M_ARENA_MAX, ARENA_LIMIT),
                libc::mallopt(libc::M_MMAP_THRESHOLD, MMAP_THRESHOLD_BYTES),
            )
        };
        if arenas == 0 || mappings == 0 {
            tracing::warn!(
                code = "embedded_allocator_options_unavailable",
                "Could not set every Embedded allocator option"
            );
        }
    }
}

fn main() -> Result<()> {
    init_tracing();
    let args = Args::parse();
    if args.role == Role::LocalIpc {
        configure_embedded_allocator();
    }
    let runtime = tokio::runtime::Runtime::new()?;
    ensure_platform_name_is_bootstrap_only(&args)?;
    ensure!(
        args.role == Role::AtlasBertActivate
            || (args.candidate_sha256.is_none() && args.evaluation_sha256.is_none()),
        "Atlas BERT activation flags require --role atlas-bert-activate"
    );
    ensure!(
        matches!(
            args.role,
            Role::AtlasBertActivate | Role::AtlasBertRecalibrate
        ) || args.expected_bert_revision.is_none(),
        "--expected-bert-revision requires Atlas BERT activation or recalibration"
    );
    ensure!(
        args.role == Role::AtlasBertReviewImport
            || (args.review_file.is_none() && args.review_reference.is_none()),
        "Review import flags require --role atlas-bert-review-import"
    );
    ensure!(
        args.role == Role::AtlasBertRecalibrate || args.reviewed_mapping_sha256.is_none(),
        "--reviewed-mapping-sha256 requires --role atlas-bert-recalibrate"
    );
    match args.role {
        Role::AtlasBertReviewExport => {
            ensure!(
                args.review_cycles.len() == 1,
                "Export requires exactly one --review-cycle"
            );
        }
        Role::AtlasBertRecalibrate => {
            ensure!(
                !args.review_cycles.is_empty() && args.expected_bert_revision.is_some(),
                "Recalibration requires --review-cycle and --expected-bert-revision"
            );
        }
        _ => ensure!(
            args.review_cycles.is_empty(),
            "--review-cycle requires Atlas BERT review export or recalibration"
        ),
    }
    if args.role == Role::LocalIpc {
        ensure!(
            args.file.is_none() && args.expected_sha256.is_none(),
            "CSV worker options are incompatible with --role local-ipc"
        );
        let result = runtime.block_on(local_ipc::run(
            args.state_dir.expect("required for local-ipc"),
            args.nlp_cmd,
            args.classifier_cmd,
            shutdown_signal(),
        ));
        // `local_ipc::run` drops storage/index/lock/helpers before returning; bound teardown so
        // an external parent that waits for child exit before closing stdin cannot hang on `read(0)`.
        tracing::debug!(
            target: "argos_api_host::local_ipc::embedded",
            code = "embedded_backend_shutdown_phase",
            phase = "runtime_shutdown_started",
        );
        runtime.shutdown_timeout(Duration::from_millis(100));
        tracing::debug!(
            target: "argos_api_host::local_ipc::embedded",
            code = "embedded_backend_shutdown_phase",
            phase = "runtime_shutdown_finished",
        );
        return result;
    }
    runtime.block_on(run_non_ipc(args))
}

async fn run_non_ipc(args: Args) -> Result<()> {
    let Args {
        role,
        port,
        origins,
        file,
        expected_sha256,
        candidate_sha256,
        evaluation_sha256,
        expected_bert_revision,
        reviewed_mapping_sha256,
        review_cycles,
        review_file,
        review_reference,
        state_dir,
        nlp_cmd,
        classifier_cmd,
        platform_display_name,
    } = args;
    ensure!(
        state_dir.is_none() && nlp_cmd.is_none() && classifier_cmd.is_none(),
        "--state-dir, --nlp-cmd, and --classifier-cmd require --role local-ipc"
    );
    if role == Role::SeedActors {
        return actor_seed::run(
            file.expect("required for actor catalog seeding"),
            expected_sha256.expect("required for actor catalog seeding"),
            shutdown_signal(),
        )
        .await;
    }
    ensure!(
        file.is_none() && expected_sha256.is_none(),
        "CSV worker options require --role seed-actors"
    );
    if matches!(role, Role::AtlasBertRevision | Role::AtlasBertActivate) {
        let command = if role == Role::AtlasBertActivate {
            atlas_bert_admin::Command::Activate {
                frozen_sha256: candidate_sha256.expect("required for BERT activation"),
                evaluation_sha256: evaluation_sha256.expect("required for BERT activation"),
                expected_revision: expected_bert_revision.expect("required for BERT activation"),
            }
        } else {
            atlas_bert_admin::Command::Revision
        };
        return atlas_bert_admin::run(fleetworks::workspace_path()?, command, shutdown_signal())
            .await;
    }
    if role == Role::AtlasBertProviderMapping {
        return atlas_bert_admin::report_provider_mapping();
    }
    let review_command = match role {
        Role::AtlasBertReviewExport => Some(atlas_bert_admin::ReviewCommand::Export {
            cycle: review_cycles.into_iter().next().expect("one export cycle"),
        }),
        Role::AtlasBertReviewImport => Some(atlas_bert_admin::ReviewCommand::Import {
            file: review_file.expect("required for review import"),
            reference: review_reference.expect("required for review import"),
        }),
        Role::AtlasBertRecalibrate => Some(atlas_bert_admin::ReviewCommand::Recalibrate {
            cycles: review_cycles,
            expected_revision: expected_bert_revision.expect("required for BERT recalibration"),
            reviewed_mapping_sha256: reviewed_mapping_sha256
                .expect("required for BERT recalibration"),
        }),
        _ => None,
    };
    ensure!(
        review_command.is_none() || cfg!(feature = "classifier"),
        "this build does not include Atlas BERT support"
    );
    if matches!(role, Role::Nlp | Role::Classifier) {
        let (bind, policy, router) = tokio::task::spawn_blocking(move || {
            let (engine, router) = if role == Role::Nlp {
                ("NLP", nlp_service::router()?)
            } else {
                ensure!(
                    cfg!(feature = "classifier"),
                    "this build does not include the classifier engine"
                );
                let home = argos_osint_core::classifier::readiness::ClassifierHome::new(
                    fleetworks::workspace_path()?,
                );
                ("CLASSIFIER", classifier_service::router(home))
            };
            let (bind, policy) = fleetworks::internal_access(engine)?;
            Ok::<_, anyhow::Error>((bind, policy, router))
        })
        .await??;
        let listener = tokio::net::TcpListener::bind(SocketAddr::new(bind, port)).await?;
        let listen = listener.local_addr()?;
        let port = listen.port();
        let router = argos_osint_api::guard_network(router, policy, Arc::new(Vec::new()), port);
        tracing::info!(
            role = ?role,
            %listen,
            "Fleetworks startup summary: private HTTP listener ready"
        );
        return serve_with_graceful_shutdown_timeout(
            listener,
            router,
            shutdown_signal(),
            Duration::from_secs(15),
        )
        .await;
    }
    let (layout, network, access, mut services) = tokio::task::spawn_blocking(move || {
        let access = if matches!(
            role,
            Role::Combined | Role::Api | Role::Brain | Role::Tools | Role::Control | Role::Jobs
        ) {
            Some(fleetworks::ApiAccess::from_environment(role, &origins)?)
        } else {
            None
        };
        let services = if role == Role::Api {
            fleetworks::private_services()?
        } else {
            argos_osint_api::PrivateServices::default()
        };
        if role == Role::Combined {
            Ok::<_, anyhow::Error>((
                Layout::new(argos_osint_core::paths::ensure_home()?),
                None,
                access,
                services,
            ))
        } else {
            let network = fleetworks::NetworkStorage::from_environment(role)?;
            Ok((
                Layout::new(network.workspace.clone()),
                Some(network),
                access,
                services,
            ))
        }
    })
    .await??;
    let runtime_user = if let Some(network) = network {
        storage::runtime::init_target(layout, network.target, network.namespace, network.database)?;
        network.runtime_user
    } else {
        storage::runtime::init(layout)?;
        None
    };
    let repo = storage::runtime::repository().await?;
    if let Some(command) = review_command {
        let result = atlas_bert_admin::run_review(
            fleetworks::workspace_path()?,
            repo,
            command,
            shutdown_signal(),
        )
        .await;
        drain_storage().await;
        return result;
    }
    if role == Role::Bootstrap {
        fleetworks::provision_user(
            &repo,
            runtime_user.expect("bootstrap always supplies a runtime user"),
        )
        .await?;
        actor_seed::seed_bootstrap(&repo).await?;
        let atlas_dispatch = match std::env::var("FLEETWORKS_ATLAS_DISPATCH").as_deref() {
            Ok("1") => true,
            Ok("0") | Err(std::env::VarError::NotPresent) => false,
            _ => anyhow::bail!("FLEETWORKS_ATLAS_DISPATCH must be 0 or 1"),
        };
        if atlas_dispatch {
            storage::job_dispatch::ensure_queue_transport(
                repo.db(),
                storage::job_dispatch::ATLAS_POOL,
            )
            .await?;
        }
        storage::job_dispatch::configure_pool(
            repo.db(),
            storage::job_dispatch::ATLAS_POOL,
            atlas_dispatch,
        )
        .await?;
        if let Some(name) = &platform_display_name {
            storage::connect_registry::set_platform_display_name(&repo, name)
                .await
                .map_err(|_| anyhow::anyhow!("invalid --platform-display-name"))?;
        }
        fleetworks::export_deployment_identity(&repo).await?;
        tracing::info!(
            role = ?role,
            storage = "ready",
            "Fleetworks startup summary: bootstrap complete"
        );
        return Ok(());
    }
    // QA owns a queue-only Jobs credential and has no shared Jobs client.
    if role == Role::AssistantQa {
        let result = inbox_qa_service::run_distributed(repo, shutdown_signal()).await;
        drain_storage().await;
        return result;
    }
    let jobs_store = if role == Role::Jobs {
        let workspace = fleetworks::workspace_path()?;
        let store = jobs_service::open(workspace.clone()).await?;
        let queue = jobs_service::open_queue(workspace).await?;
        storage::jobs_client::install_local(store.clone())?;
        storage::jobs_client::install_queue(queue.clone())?;
        Some((store, queue))
    } else {
        if role != Role::Combined {
            tokio::task::spawn_blocking(storage::jobs_client::initialize_remote_from_environment)
                .await??;
        }
        None
    };
    if role == Role::Execution {
        let (bind, policy, workspace) = tokio::task::spawn_blocking(|| {
            let (bind, policy) =
                fleetworks::internal_access_with_token("EXECUTION", "FLEETWORKS_EXECUTION_TOKEN")?;
            Ok::<_, anyhow::Error>((bind, policy, fleetworks::workspace_path()?))
        })
        .await??;
        let listener = tokio::net::TcpListener::bind(SocketAddr::new(bind, port)).await?;
        let listen = listener.local_addr()?;
        let port = listen.port();
        let owner_state = argos_osint_api::ApiState::new(
            repo.clone(),
            format!("execution-{}-{}", std::process::id(), chrono_id()),
            port,
            Arc::new(Vec::new()),
        );
        let router = argos_osint_api::guard_network(
            execution_service::router(workspace),
            policy.clone(),
            Arc::new(Vec::new()),
            port,
        );
        let router = reset_services::attach_http_owner(router, &owner_state, policy, role).await?;
        tracing::info!(
            role = ?role,
            %listen,
            storage = "connected",
            "Fleetworks startup summary: private HTTP listener ready"
        );
        let result = serve_with_graceful_shutdown_timeout(
            listener,
            router,
            shutdown_signal(),
            Duration::from_secs(15),
        )
        .await;
        drain_storage().await;
        return result;
    }
    if role == Role::NativeWorker {
        let workspace = fleetworks::workspace_path()?;
        // Native setup and reindex tasks stamp this process owner on their
        // leases, so orphan recovery must see its heartbeat while they run.
        let _beat = start_process_beat().await?;
        let result = reset_services::run_headless(
            repo.clone(),
            role,
            move |stop| native_worker::run(repo, workspace, reset_services::stopped(stop)),
            shutdown_signal(),
        )
        .await;
        drain_storage().await;
        return result;
    }
    if role == Role::Api {
        services = fleetworks::workstation_services(&repo, services).await?;
    }
    // Retained operator/worker readers remain until their explicit retirement.
    if matches!(
        role,
        Role::Combined | Role::Worker | Role::BriefWorker | Role::AtlasWorker
    ) {
        tokio::task::spawn_blocking(argos_osint_core::provider::SettingsFile::load).await??;
    }
    if role == Role::AtlasWorker {
        let _beat = start_process_beat().await?;
        let result = reset_services::run_headless(
            repo.clone(),
            role,
            move |stop| atlas_worker::run(repo, reset_services::stopped(stop)),
            shutdown_signal(),
        )
        .await;
        drain_storage().await;
        return result;
    }
    if matches!(role, Role::Worker | Role::BriefWorker) {
        let result = reset_services::run_headless(
            repo.clone(),
            role,
            move |stop| async move {
                let (beat, workers) = start_workers(role).await?;
                tracing::info!(
                    role = ?role,
                    worker_pools = workers.len(),
                    storage = "connected",
                    "Fleetworks startup summary: worker pools ready"
                );
                reset_services::stopped(stop).await;
                tokio::task::spawn_blocking(move || {
                    argos_osint_core::scheduler::WorkerPool::shutdown_all(workers);
                    drop(beat);
                })
                .await?;
                Ok(())
            },
            shutdown_signal(),
        )
        .await;
        drain_storage().await;
        return result;
    }
    let http = if let Some(api) = access {
        let listener = tokio::net::TcpListener::bind(SocketAddr::new(api.bind, port)).await?;
        let port = listener.local_addr()?.port();
        let state = argos_osint_api::ApiState::new(
            repo,
            format!("{}-{}", std::process::id(), chrono_id()),
            port,
            Arc::new(api.origins),
        );
        let qa_stop = argos_osint_api::inbox_qa::CancellationToken::new();
        let qa_refresh = (role == Role::Api).then(|| {
            tokio::spawn(argos_osint_api::inbox_qa::run_health_refresh(
                state.clone(),
                services.clone(),
                qa_stop.clone(),
            ))
        });
        let router = if let Some(policy) = api.policy {
            let reset = if role == Role::Api {
                reset_services::gateway(&state, policy.clone()).await?
            } else {
                None
            };
            let owner_state = state.clone();
            let owner_policy = policy.clone();
            let router = match role {
                Role::Brain => argos_osint_api::brain_router(state, policy),
                Role::Tools => argos_osint_api::tools_router(state, policy),
                Role::Control => argos_osint_api::control_router(state, policy),
                Role::Jobs => {
                    let (store, queue) = jobs_store.expect("Jobs owns the operational datastores");
                    let qa_queue = if let Some(qa_policy) =
                        tokio::task::spawn_blocking(fleetworks::qa_jobs_access).await??
                    {
                        argos_osint_api::guard_network(
                            jobs_service::qa_router(queue.clone()),
                            qa_policy,
                            Arc::new(Vec::new()),
                            port,
                        )
                    } else {
                        axum::Router::new()
                    };
                    let internal = argos_osint_api::guard_network(
                        jobs_service::router(store, queue),
                        policy.clone(),
                        Arc::new(Vec::new()),
                        port,
                    );
                    argos_osint_api::jobs_router(state, policy)
                        .merge(internal)
                        .merge(qa_queue)
                }
                _ => argos_osint_api::network_router_with_private_services(state, policy, services),
            };
            let router = if let Some(reset) = reset {
                router.layer(axum::Extension(reset))
            } else {
                router
            };
            if matches!(role, Role::Brain | Role::Tools | Role::Control | Role::Jobs) {
                reset_services::attach_http_owner(router, &owner_state, owner_policy, role).await?
            } else {
                router
            }
        } else {
            ensure!(
                api.bind == IpAddr::V4(Ipv4Addr::LOCALHOST),
                "legacy API must bind to loopback"
            );
            argos_osint_api::router(state)
        };
        Some((listener, router, qa_stop, qa_refresh))
    } else {
        None
    };
    let workers = if role == Role::Combined {
        Some(start_workers(role).await?)
    } else {
        None
    };
    let served = if let Some((listener, router, qa_stop, qa_refresh)) = http {
        let listen = listener.local_addr()?;
        tracing::info!(
            role = ?role,
            %listen,
            storage = "connected",
            "Fleetworks startup summary: HTTP listener ready"
        );
        let served = serve_with_graceful_shutdown_timeout(
            listener,
            router,
            shutdown_signal(),
            Duration::from_secs(15),
        )
        .await;
        qa_stop.cancel();
        if let Some(refresh) = qa_refresh {
            let _ = refresh.await;
        }
        served
    } else {
        shutdown_signal().await;
        Ok(())
    };
    if let Some((beat, workers)) = workers {
        tokio::task::spawn_blocking(move || {
            argos_osint_core::scheduler::WorkerPool::shutdown_all(workers);
            drop(beat);
        })
        .await?;
    }
    drain_storage().await;
    served
}

async fn drain_storage() {
    if tokio::time::timeout(Duration::from_secs(6), storage::runtime::drain())
        .await
        .is_err()
    {
        eprintln!("Core worker shutdown deadline reached; durable leases will recover on restart");
    }
}

async fn start_workers(
    role: Role,
) -> Result<(
    argos_osint_core::job_registry::ProcessBeat,
    Vec<argos_osint_core::scheduler::WorkerPool>,
)> {
    let beat = start_process_beat().await?;
    use argos_osint_core::scheduler::{process_owner, WorkerPool};
    let path = argos_osint_core::paths::db_path();
    let workers = match role {
        Role::BriefWorker => vec![WorkerPool::spawn_intel_brief_drainer(path, process_owner())],
        Role::Worker => WorkerPool::spawn_network_pools(path),
        _ => WorkerPool::spawn_default(path),
    };
    Ok((beat, workers))
}

async fn start_process_beat() -> Result<argos_osint_core::job_registry::ProcessBeat> {
    // runtime::blocking requires a plain thread outside Tokio's entered
    // context; spawn_blocking also enters Tokio and would be rejected.
    let (beat_tx, beat_rx) = tokio::sync::oneshot::channel();
    let _beat_init = std::thread::Builder::new()
        .name("argos-api-init".into())
        .spawn(move || {
            let result = argos_osint_core::job_registry::start_process_beat(
                &argos_osint_core::paths::db_path(),
            );
            let _ = beat_tx.send(result);
        })?;
    beat_rx.await?
}

async fn serve_with_graceful_shutdown_timeout<F>(
    listener: tokio::net::TcpListener,
    app: axum::Router,
    signal: F,
    timeout_duration: Duration,
) -> Result<()>
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let server = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        let _ = shutdown_rx.await;
    })
    .into_future();
    tokio::pin!(server);

    tokio::select! {
        res = server.as_mut() => {
            res?;
        }
        _ = signal => {
            let _ = shutdown_tx.send(());
            match tokio::time::timeout(timeout_duration, server.as_mut()).await {
                Ok(Ok(())) => {}
                Ok(Err(err)) => eprintln!("Server error during graceful shutdown: {err}"),
                Err(_) => {
                    eprintln!(
                        "Graceful shutdown timeout reached ({:?}); forcing server termination",
                        timeout_duration
                    );
                }
            }
        }
    }
    Ok(())
}

fn chrono_id() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("SIGTERM handler");
        tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = term.recv() => {} }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::routing::get;

    #[test]
    fn recalibration_requires_reviewed_provider_mapping_digest() {
        let recalibration = [
            "argos-api-host",
            "--role",
            "atlas-bert-recalibrate",
            "--review-cycle",
            "cycle-1",
            "--expected-bert-revision",
            "model-revision",
        ];
        assert!(Args::try_parse_from(recalibration).is_err());
        let args = Args::try_parse_from(
            recalibration
                .into_iter()
                .chain(["--reviewed-mapping-sha256", "reviewed-rules"]),
        )
        .unwrap();
        assert_eq!(
            args.reviewed_mapping_sha256.as_deref(),
            Some("reviewed-rules")
        );
    }

    #[tokio::test]
    async fn chrono_id_is_non_zero() {
        assert!(chrono_id() > 0);
    }

    #[tokio::test]
    async fn graceful_shutdown_exits_cleanly_on_signal() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let app = axum::Router::new().route("/ping", get(|| async { "pong" }));
        let (signal_tx, signal_rx) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(serve_with_graceful_shutdown_timeout(
            listener,
            app,
            async move {
                let _ = signal_rx.await;
            },
            Duration::from_secs(5),
        ));

        let _ = signal_tx.send(());
        let res = tokio::time::timeout(Duration::from_secs(2), handle).await;
        assert!(res.is_ok(), "server did not shutdown in time");
        assert!(res.unwrap().unwrap().is_ok());
    }

    #[tokio::test]
    async fn graceful_shutdown_times_out_when_client_hangs() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = axum::Router::new().route(
            "/hang",
            get(|| async {
                tokio::time::sleep(Duration::from_secs(10)).await;
                "done"
            }),
        );
        let (signal_tx, signal_rx) = tokio::sync::oneshot::channel();
        let server_handle = tokio::spawn(serve_with_graceful_shutdown_timeout(
            listener,
            app,
            async move {
                let _ = signal_rx.await;
            },
            Duration::from_millis(100),
        ));

        let client_handle = tokio::spawn(async move {
            let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
            use tokio::io::AsyncWriteExt;
            stream
                .write_all(b"GET /hang HTTP/1.1\r\nHost: localhost\r\n\r\n")
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_secs(5)).await;
        });

        tokio::time::sleep(Duration::from_millis(50)).await;
        let _ = signal_tx.send(());

        let started = std::time::Instant::now();
        let server_res = tokio::time::timeout(Duration::from_secs(2), server_handle).await;
        let elapsed = started.elapsed();
        assert!(
            server_res.is_ok(),
            "server did not force terminate on timeout"
        );
        assert!(server_res.unwrap().unwrap().is_ok());
        assert!(
            elapsed < Duration::from_secs(2),
            "expected shutdown timeout around 100ms, took {:?}",
            elapsed
        );

        client_handle.abort();
    }

    #[cfg(feature = "embedded")]
    pub(crate) static EMBEDDED_LIFECYCLE_TEST_LOCK: tokio::sync::Mutex<()> =
        tokio::sync::Mutex::const_new(());

    #[test]
    fn local_ipc_cli_flags_parse_with_required_state_dir() {
        let parsed = Args::try_parse_from([
            "argos-api-host",
            "--role",
            "local-ipc",
            "--state-dir",
            "/tmp/fleetworks-state",
            "--nlp-cmd",
            "/usr/local/bin/nlp-helper",
            "--classifier-cmd",
            "/usr/local/bin/classifier-helper",
        ])
        .expect("local-ipc CLI parses");
        assert_eq!(parsed.role, Role::LocalIpc);
        assert_eq!(
            parsed.state_dir,
            Some(std::path::PathBuf::from("/tmp/fleetworks-state"))
        );
        assert_eq!(
            parsed.nlp_cmd,
            Some(std::path::PathBuf::from("/usr/local/bin/nlp-helper"))
        );
        assert_eq!(
            parsed.classifier_cmd,
            Some(std::path::PathBuf::from("/usr/local/bin/classifier-helper"))
        );
        assert!(Args::try_parse_from(["argos-api-host", "--role", "local-ipc"]).is_err());
    }

    #[test]
    fn platform_display_name_is_accepted_only_for_bootstrap() {
        let bootstrap = Args::try_parse_from([
            "argos-api-host",
            "--role",
            "bootstrap",
            "--platform-display-name",
            "Studio rack",
        ])
        .expect("bootstrap CLI parses");
        assert_eq!(
            bootstrap.platform_display_name.as_deref(),
            Some("Studio rack")
        );
        assert!(ensure_platform_name_is_bootstrap_only(&bootstrap).is_ok());
        let combined =
            Args::try_parse_from(["argos-api-host", "--platform-display-name", "Studio rack"])
                .expect("flag parses before the role rule");
        assert!(ensure_platform_name_is_bootstrap_only(&combined).is_err());
        let absent =
            Args::try_parse_from(["argos-api-host", "--role", "bootstrap"]).expect("parses");
        assert!(absent.platform_display_name.is_none());
        assert!(ensure_platform_name_is_bootstrap_only(&absent).is_ok());
    }
}
