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
