use std::sync::Arc;

use anyhow::Context;
use anyhow::Result;
use fleetworks_search_index::config::Config;
use fleetworks_search_index::engine::Engine;
use fleetworks_search_index::http;

fn main() -> Result<()> {
    anyhow::ensure!(
        cfg!(target_os = "linux"),
        "the private search service is a Linux backend product"
    );
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let config = Config::from_environment()?;
    // Opening/dropping mmap writers and reading protected config happen on the
    // plain main thread, never on a Tokio worker or in a GUI process.
    let (engine, reason) = match Engine::open(config.engine.clone()) {
        Ok(engine) => (Some(Arc::new(engine)), ""),
        Err(error) => {
            tracing::error!(
                reason = error.code,
                "actor index unavailable; explicit operator repair is required"
            );
            (None, error.code)
        }
    };
    let engine = Arc::new(http::EngineSlot::new(engine));
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("cannot create search transport runtime")?;
    runtime.block_on(async {
        let listener = tokio::net::TcpListener::bind(config.bind)
            .await
            .context("cannot bind private search listener")?;
        axum::serve(
            listener,
            http::router_with_slot(config, engine.clone(), reason)?,
        )
        .with_graceful_shutdown(shutdown())
        .await
        .context("private search server failed")
    })?;
    drop(runtime);
    drop(engine);
    Ok(())
}

async fn shutdown() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::SignalKind;
        if let Ok(mut terminate) = tokio::signal::unix::signal(SignalKind::terminate()) {
            tokio::select! { _ = tokio::signal::ctrl_c() => (), _ = terminate.recv() => () }
            return;
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}
