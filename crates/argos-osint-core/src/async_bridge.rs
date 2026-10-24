//! Runtime bridge for sync callers that need to drive a future
//! (embedding downloads, summary upgrades, neighborhood briefs).

use std::future::Future;
use std::sync::OnceLock;

fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("argos-bridge")
            .enable_all()
            .build()
            .expect("start the async bridge runtime")
    })
}

/// Runs `future` to completion on a process-local runtime.
pub fn block_on<F>(future: F) -> F::Output
where
    F: Future + Send,
    F::Output: Send,
{
    use tokio::runtime::{Handle, RuntimeFlavor};
    match Handle::try_current() {
        Err(_) => runtime().block_on(future),
        Ok(handle) if handle.runtime_flavor() == RuntimeFlavor::MultiThread => {
            tokio::task::block_in_place(|| runtime().block_on(future))
        }
        Ok(_) => {
            std::thread::scope(
                |scope| match scope.spawn(|| runtime().block_on(future)).join() {
                    Ok(output) => output,
                    Err(panic) => std::panic::resume_unwind(panic),
                },
            )
        }
    }
}
