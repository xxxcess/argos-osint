#[cfg(feature = "surreal")]
use std::time::Duration;

#[cfg(feature = "surreal")]
const DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

/// Tokio worker-stack size for the shared multi-thread runtime (8 MiB).
///
/// The default 2 MiB worker stack is not enough for the embedded SurrealDB
/// engine's poll depth: lldb backtraces of the TUI composer crash showed
/// ~2.1 MB of `Future::poll` frames — `App::send_recon_query` ->
/// `recon::Service::ask` -> `orchestrate::run_turn` -> `derive_directives` ->
/// `nlp::prepare_mode` -> `nlp::surreal_put` -> `storage::nlp_cache::put` ->
/// `Transaction::commit` (SurrealDB `IndexOperation::compute` /
/// `Document::upsert_create` / `Executor`) — for one ordinary cache UPSERT.
/// 8 MiB keeps every deep storage call (recon, Atlas, NLP cache) on the same
/// worker stack instead of localizing the fix to one NLP cache write.
const WORKER_STACK_BYTES: usize = 8 * 1024 * 1024;

// Compile-time guard on the chosen floor: the measured poll chain on the
// failing composer launch was ~2.08 MB on the default 2 MiB worker stack
// (lldb frame span 0x1f9c30), and the 8 MiB floor keeps headroom for the
// other deep storage calls (recon, Atlas, NLP cache).
const _: () = assert!(
    WORKER_STACK_BYTES >= 8 * 1024 * 1024,
    "worker stack must stay at the 8 MiB floor for the measured ~2.08 MB SurrealDB commit poll chain"
);

/// Build the runtime that owns CLI dispatch, the TUI and the storage handle.
fn build_runtime() -> anyhow::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(WORKER_STACK_BYTES)
        .build()
        .map_err(|err| anyhow::anyhow!("failed to start the Tokio runtime: {err:#}"))
}

fn main() {
    argos_osint_bin::logging::init_tracing();
    match build_runtime() {
        Ok(runtime) => runtime.block_on(serve()),
        Err(err) => {
            eprintln!("argos: {err:#}");
            std::process::exit(1);
        }
    }
}

async fn serve() {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_builds_with_the_configured_worker_stack() {
        let runtime = build_runtime().expect("runtime with an explicit worker stack builds");
        runtime.block_on(async {
            tokio::spawn(async {
                // The spawn must succeed on the built runtime, i.e. the
                // accepted multi-thread flavour is in place.
            })
            .await
            .expect("spawn on the configured runtime")
        });
        runtime.shutdown_timeout(std::time::Duration::from_secs(2));
    }
}
