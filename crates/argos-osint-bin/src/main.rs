#[cfg(feature = "surreal")]
use std::time::Duration;

#[cfg(feature = "surreal")]
const DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

#[tokio::main]
async fn main() {
    argos_osint_bin::logging::init_tracing();
    // The one storage entry point: the CLI and TUI reach the SurrealDB
    // repository through this runtime's handle.
    #[cfg(feature = "surreal")]
    if let Err(err) = argos_osint_core::paths::ensure_home()
        .map_err(anyhow::Error::from)
        .and_then(|home| {
            argos_osint_core::storage::runtime::init(argos_osint_core::storage::home::Layout::new(
                home,
            ))
        })
    {
        eprintln!("argos: {err:#}");
        std::process::exit(1);
    }
    let outcome = argos_osint_bin::cli::dispatch().await;
    // Best-effort SurrealDB telemetry writes finish before the process exits.
    // Bounded by timeout so stuck background writes cannot prevent clean termination.
    #[cfg(feature = "surreal")]
    if tokio::time::timeout(DRAIN_TIMEOUT, argos_osint_core::storage::runtime::drain())
        .await
        .is_err()
    {
        tracing::warn!("storage runtime drain timed out after 5s during shutdown");
    }
    if let Err(err) = outcome {
        eprintln!("argos: {err:#}");
        std::process::exit(1);
    }
}
