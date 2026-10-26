use anyhow::Result;
use argos_osint_core::storage::{self, home::Layout};
use clap::Parser;
use std::{
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
#[tokio::main]
async fn main() -> Result<()> {
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
    axum::serve(listener, argos_osint_api::router(state))
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    drop(workers);
    drop(beat);
    if tokio::time::timeout(Duration::from_secs(6), storage::runtime::drain())
        .await
        .is_err()
    {
        eprintln!("Core worker shutdown deadline reached; durable leases will recover on restart");
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
