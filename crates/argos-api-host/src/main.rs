mod atlas_worker;
mod classifier_train;
mod fleetworks;
mod nlp_service;

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
    /// Private workspace CSV snapshot selected by Venture for one-shot training.
    #[arg(long, required_if_eq("role", "train-classifier"))]
    file: Option<std::path::PathBuf>,
    #[arg(long, required_if_eq("role", "train-classifier"))]
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
    /// Fleetworks background domain workers.
    Worker,
    /// Claim and execute durable Atlas dispatches.
    AtlasWorker,
    /// Warm, private Compromise inference without a database session.
    Nlp,
    /// Train the optional Natural classifier from a workspace dataset.
    TrainClassifier,
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
    ensure!(
        file.is_none() && expected_sha256.is_none() && !activate,
        "classifier training options require --role train-classifier"
    );
    if role == Role::Nlp {
        let (bind, policy, router) = tokio::task::spawn_blocking(|| {
            let (bind, policy) = fleetworks::internal_access("NLP")?;
            Ok::<_, anyhow::Error>((bind, policy, nlp_service::router()?))
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
    let (layout, network, access) = tokio::task::spawn_blocking(move || {
        let access = if matches!(role, Role::Combined | Role::Api) {
            Some(fleetworks::ApiAccess::from_environment(role, &origins)?)
        } else {
            None
        };
        if role == Role::Combined {
            Ok::<_, anyhow::Error>((
                Layout::new(argos_osint_core::paths::ensure_home()?),
                None,
                access,
            ))
        } else {
            let network = fleetworks::NetworkStorage::from_environment(role)?;
            Ok((
                Layout::new(network.workspace.clone()),
                Some(network),
                access,
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
        storage::job_dispatch::configure_pool(
            repo.db(),
            storage::job_dispatch::ATLAS_POOL,
            atlas_dispatch,
        )
        .await?;
        println!("Fleetworks storage ready");
        return Ok(());
    }
    tokio::task::spawn_blocking(argos_osint_core::provider::SettingsFile::load).await??;
    if role == Role::AtlasWorker {
        let _beat = start_process_beat().await?;
        let result = atlas_worker::run(repo, shutdown_signal()).await;
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
            argos_osint_api::network_router(state, policy)
        } else {
            ensure!(
                api.bind == IpAddr::V4(Ipv4Addr::LOCALHOST),
                "legacy API must bind to loopback"
            );
            argos_osint_api::router(state)
        };
        println!("Argos API http://{}:{port}", api.bind);
        Some((listener, router))
    } else {
        None
    };
    let workers = if matches!(role, Role::Combined | Role::Worker) {
        Some(start_workers().await?)
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

async fn start_workers() -> Result<(
    argos_osint_core::job_registry::ProcessBeat,
    Vec<argos_osint_core::scheduler::WorkerPool>,
)> {
    let beat = start_process_beat().await?;
    let workers =
        argos_osint_core::scheduler::WorkerPool::spawn_default(argos_osint_core::paths::db_path());
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
    let server = axum::serve(listener, app)
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
