use anyhow::Result;
use argos_osint_core::storage::{self, home::Layout};
use clap::Parser;
use std::{
    future::IntoFuture,
    net::{Ipv4Addr, SocketAddrV4},
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
    let args = Args::parse();
    let origins = args
        .origins
        .iter()
        .map(|raw| argos_osint_api::loopback_url(raw).map(|url| url.origin().ascii_serialization()))
        .collect::<Result<Vec<_>>>()?;
    storage::runtime::init(Layout::new(argos_osint_core::paths::ensure_home()?))?;
    // Only this process opens storage; startup fails rather than taking a second lock.
    let repo = storage::runtime::repository().await?;
    argos_osint_core::provider::SettingsFile::load()?;
    let listener =
        tokio::net::TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, args.port)).await?;
    let port = listener.local_addr()?.port();
    let instance_id = format!("{}-{}", std::process::id(), chrono_id());
    // Start the existing elected core pools (brief, indexing, summaries and
    // durable recovery) rather than creating an HTTP-specific executor.
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
    let beat = beat_rx.await??;
    let workers =
        argos_osint_core::scheduler::WorkerPool::spawn_default(argos_osint_core::paths::db_path());
    let state = argos_osint_api::ApiState {
        repo,
        instance_id,
        port,
        origins: Arc::new(origins),
    };
    println!("Argos API http://127.0.0.1:{port}");
    serve_with_graceful_shutdown_timeout(
        listener,
        argos_osint_api::router(state),
        shutdown_signal(),
        Duration::from_secs(15),
    )
    .await?;
    argos_osint_core::scheduler::WorkerPool::shutdown_all(workers);
    drop(beat);
    if tokio::time::timeout(Duration::from_secs(6), storage::runtime::drain())
        .await
        .is_err()
    {
        eprintln!("Core worker shutdown deadline reached; durable leases will recover on restart");
    }
    Ok(())
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
