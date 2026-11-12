mod actor_index_worker;
mod actor_seed;
mod atlas_worker;
mod classifier_service;
mod classifier_train;
mod execution_service;
mod fleetworks;
mod jobs_service;
mod native_worker;
mod nlp_service;
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
    /// Private workspace CSV snapshot selected by Venture.
    #[arg(long, required_if_eq_any([("role", "train-classifier"), ("role", "seed-actors")]))]
    file: Option<std::path::PathBuf>,
    #[arg(long, required_if_eq_any([("role", "train-classifier"), ("role", "seed-actors")]))]
    expected_sha256: Option<String>,
    /// Activate the synthetic Natural baseline after successful training.
    #[arg(long)]
    activate: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Role {
    /// Existing local API host with its background workers.
    Combined,
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
    /// Consume registered setup/tools Jobs and pump the private actor index.
    NativeWorker,
    /// Warm, private Compromise inference without a database session.
    Nlp,
    /// Serve pinned Rust-BERT inference without a database session.
    Classifier,
    /// Train the optional Natural classifier from a workspace dataset.
    TrainClassifier,
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

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();
    let Args {
        role,
        port,
        origins,
        file,
        expected_sha256,
        activate,
    } = Args::parse();
    if role == Role::TrainClassifier {
        return classifier_train::run(
            fleetworks::workspace_path()?,
            file.expect("required for classifier training"),
            expected_sha256.expect("required for classifier training"),
            activate,
            shutdown_signal(),
        )
        .await;
    }
    if role == Role::SeedActors {
        ensure!(!activate, "--activate requires --role train-classifier");
        return actor_seed::run(
            file.expect("required for actor catalog seeding"),
            expected_sha256.expect("required for actor catalog seeding"),
            shutdown_signal(),
        )
        .await;
    }
    ensure!(
        file.is_none() && expected_sha256.is_none() && !activate,
        "CSV worker options require --role train-classifier or --role seed-actors"
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
        let port = listener.local_addr()?.port();
        let router = argos_osint_api::guard_network(router, policy, Arc::new(Vec::new()), port);
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
    if role == Role::Bootstrap {
        fleetworks::provision_user(
            &repo,
            runtime_user.expect("bootstrap always supplies a runtime user"),
        )
        .await?;
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
        fleetworks::export_deployment_identity(&repo).await?;
        println!("Fleetworks storage ready");
        return Ok(());
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
        let port = listener.local_addr()?.port();
        let owner_state = argos_osint_api::ApiState {
            repo: repo.clone(),
            instance_id: format!("execution-{}-{}", std::process::id(), chrono_id()),
            port,
            origins: Arc::new(Vec::new()),
        };
        let router = argos_osint_api::guard_network(
            execution_service::router(workspace),
            policy.clone(),
            Arc::new(Vec::new()),
            port,
        );
        let router = reset_services::attach_http_owner(router, &owner_state, policy, role).await?;
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
        let state = argos_osint_api::ApiState {
            repo,
            instance_id: format!("{}-{}", std::process::id(), chrono_id()),
            port,
            origins: Arc::new(api.origins),
        };
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
                    let internal = argos_osint_api::guard_network(
                        jobs_service::router(store, queue),
                        policy.clone(),
                        Arc::new(Vec::new()),
                        port,
                    );
                    argos_osint_api::jobs_router(state, policy).merge(internal)
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
        let service = match role {
            Role::Brain => "Brain",
            Role::Tools => "Tools",
            Role::Control => "Control",
            Role::Jobs => "Jobs",
            _ => "API",
        };
        println!("Argos {service} http://{}:{port}", api.bind);
        Some((listener, router))
    } else {
        None
    };
    let workers = if role == Role::Combined {
        Some(start_workers(role).await?)
    } else {
        None
    };
    let served = if let Some((listener, router)) = http {
        serve_with_graceful_shutdown_timeout(
            listener,
            router,
            shutdown_signal(),
            Duration::from_secs(15),
        )
        .await
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
}
