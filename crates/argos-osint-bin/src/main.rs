mod cli;
#[cfg(test)]
mod test_storage;
mod tui;

#[tokio::main]
async fn main() {
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
    let outcome = cli::dispatch().await;
    // Best-effort SurrealDB telemetry writes finish before the process exits.
    #[cfg(feature = "surreal")]
    argos_osint_core::storage::runtime::drain().await;
    if let Err(err) = outcome {
        eprintln!("argos: {err:#}");
        std::process::exit(1);
    }
}
