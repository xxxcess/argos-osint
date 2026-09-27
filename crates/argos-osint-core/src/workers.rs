//! Shared bounds for blocking SQLite/filesystem/parser/graph work.
use std::sync::OnceLock;
use tokio::{sync::Semaphore, task::JoinHandle};
pub fn spawn_blocking<F, R>(work: F) -> JoinHandle<R>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    tokio::spawn(async move {
        static LIMIT: OnceLock<Semaphore> = OnceLock::new();
        let _permit = LIMIT
            .get_or_init(|| Semaphore::new(4))
            .acquire()
            .await
            .expect("worker semaphore is never closed");
        match tokio::task::spawn_blocking(work).await {
            Ok(result) => result,
            Err(error) => std::panic::resume_unwind(Box::new(error)),
        }
    })
}
