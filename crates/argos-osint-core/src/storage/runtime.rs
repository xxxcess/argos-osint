//! The single storage entry point for the CLI and TUI (P4, Option B).
//!
//! [`init`] is called once from inside the process's one Tokio runtime (the
//! binary's `main`); it records that runtime's handle and the home. Async
//! code calls [`repository`], which opens the home's store once and hands
//! out clones. Synchronous code on a plain thread (outside the runtime)
//! calls [`blocking`], which drives the work on that same handle. Calling
//! [`blocking`] from inside an async context is an error, never a nested
//! `block_on`.

use std::future::Future;
use std::sync::OnceLock;

use anyhow::{anyhow, bail, Result};
use tokio::runtime::Handle;
use tokio::sync::OnceCell;

use super::home::Layout;
use super::repo::Repository;

struct Entry {
    handle: Handle,
    layout: Layout,
    repo: OnceCell<Repository>,
}

static ENTRY: OnceLock<Entry> = OnceLock::new();

/// Record the runtime and home. Must run inside the runtime; a second call
/// with the same home is a no-op, with another home an error.
pub fn init(layout: Layout) -> Result<()> {
    let handle = Handle::try_current()
        .map_err(|_| anyhow!("storage::runtime::init needs the Tokio runtime"))?;
    let entry = ENTRY.get_or_init(|| Entry {
        handle,
        layout: layout.clone(),
        repo: OnceCell::new(),
    });
    if entry.layout.home != layout.home {
        bail!(
            "storage runtime already initialized for {}",
            entry.layout.home.display()
        );
    }
    Ok(())
}

/// Whether the process selected SurrealDB: [`init`] ran and the home has a
/// storage config (an unreadable config counts as configured, so a broken
/// file fails loudly instead of silently falling back to SQLite). Cached
/// after the first call. Without [`init`] (core tests, embedders) this is
/// `false` and callers stay on SQLite.
pub fn configured() -> bool {
    static CONFIGURED: OnceLock<bool> = OnceLock::new();
    let Some(entry) = ENTRY.get() else {
        return false;
    };
    *CONFIGURED.get_or_init(|| !matches!(super::home::load_config(&entry.layout), Ok(None)))
}

fn entry() -> Result<&'static Entry> {
    ENTRY
        .get()
        .ok_or_else(|| anyhow!("storage runtime not initialized"))
}

/// The process's repository, opened on first use.
pub async fn repository() -> Result<Repository> {
    let entry = entry()?;
    entry
        .repo
        .get_or_try_init(|| Repository::open_home(&entry.layout))
        .await
        .cloned()
}

/// Run `f` with the repository from synchronous code on a thread outside
/// the runtime. Inside an async context use [`repository`] instead.
pub fn blocking<T, F, Fut>(f: F) -> Result<T>
where
    F: FnOnce(Repository) -> Fut,
    Fut: Future<Output = Result<T>>,
{
    if Handle::try_current().is_ok() {
        bail!("storage::runtime::blocking called inside an async context; await storage::runtime::repository() instead");
    }
    let entry = entry()?;
    entry
        .handle
        .block_on(async { f(repository().await?).await })
}

static PENDING: std::sync::Mutex<Vec<tokio::task::JoinHandle<()>>> =
    std::sync::Mutex::new(Vec::new());

/// Run a best-effort write (telemetry) on the process runtime without
/// waiting, from synchronous or async code. `false` without [`init`]. The
/// task is tracked so [`drain`] can wait for it before the process exits.
pub fn spawn<F>(work: F) -> bool
where
    F: Future<Output = ()> + Send + 'static,
{
    let Some(entry) = ENTRY.get() else {
        return false;
    };
    let handle = entry.handle.spawn(work);
    let mut pending = PENDING.lock().unwrap_or_else(|e| e.into_inner());
    pending.retain(|h| !h.is_finished());
    pending.push(handle);
    true
}

type Queued = (
    std::pin::Pin<Box<dyn Future<Output = ()> + Send>>,
    tokio::sync::oneshot::Sender<()>,
);

/// Like [`spawn`], but writes started here run one at a time in the order
/// they were started, so a later write (an attempt's finish, a job's
/// phase) never lands before an earlier one (its start). [`drain`] waits
/// for them too. `false` without [`init`].
pub fn spawn_ordered<F>(work: F) -> bool
where
    F: Future<Output = ()> + Send + 'static,
{
    static QUEUE: OnceLock<tokio::sync::mpsc::UnboundedSender<Queued>> = OnceLock::new();
    let Some(entry) = ENTRY.get() else {
        return false;
    };
    let queue = QUEUE.get_or_init(|| {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Queued>();
        entry.handle.spawn(async move {
            while let Some((work, done)) = rx.recv().await {
                work.await;
                let _ = done.send(());
            }
        });
        tx
    });
    let (done, finished) = tokio::sync::oneshot::channel();
    if queue.send((Box::pin(work), done)).is_err() {
        return false;
    }
    spawn(async move {
        let _ = finished.await;
    })
}

/// Wait for every write started by [`spawn`] (call before the process exits).
pub async fn drain() {
    loop {
        let batch: Vec<_> = {
            let mut pending = PENDING.lock().unwrap_or_else(|e| e.into_inner());
            std::mem::take(&mut *pending)
        };
        if batch.is_empty() {
            return;
        }
        for handle in batch {
            let _ = handle.await;
        }
    }
}
