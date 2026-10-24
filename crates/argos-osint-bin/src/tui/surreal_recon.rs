//! The TUI's own Recon reads and writes on SurrealDB (Recon switch step 4).
//!
//! Under a storage config ([`on`]) the thread list, the open thread's
//! messages, calls and runs, drafts, selection, rename, delete, recall and
//! tool toggles go through `recon::backend::ReconStore` on the repository
//! and never touch the SQLite store. Every function blocks on
//! `storage::runtime::blocking`, so callers are plain worker threads that
//! report back by `WorkEvent`, never async code or the draw loop (the one
//! exception is [`boot`], before the event loop starts).

use std::collections::HashMap;

use anyhow::Result;
use argos_osint_core::recon::Thread;

use super::db_reads::ReconSnapshot;

pub(crate) fn on() -> bool {
    super::surreal_brain::on()
}

/// What `App::boot` reads from the Recon store.
pub(crate) struct Boot {
    pub last_thread: Option<String>,
    pub threads: Vec<Thread>,
    pub tool_enabled: Vec<bool>,
}

/// After a new thread is created on the worker.
#[derive(Debug, Clone)]
pub(crate) enum Then {
    /// Open it in Recon.
    Open,
    /// Home launch: open it and ask `question`.
    Launch { question: String },
    /// A Tools result was attached; open it.
    Attached,
}

#[cfg(feature = "surreal")]
mod imp {
    use std::future::Future;
    use std::pin::Pin;

    use super::*;
    use argos_osint_core::recon::backend::ReconStore;
    use argos_osint_core::storage::runtime;

    type Fut<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + 'a>>;

    fn with<'b, T>(f: impl FnOnce(ReconStore) -> Fut<'b, T>) -> Result<T> {
        runtime::blocking(|repo| f(ReconStore::from_repository(repo)))
    }

    /// Runs on a short-lived thread so it never sits inside an async
    /// context; called once before the event loop.
    pub(crate) fn boot() -> Result<Boot> {
        std::thread::spawn(|| {
            with(|s| {
                Box::pin(async move {
                    s.recover_runs().await?;
                    let mut tool_enabled = Vec::new();
                    for tool in argos_osint_core::osint::registry() {
                        tool_enabled.push(s.tool_enabled(tool.id).await?);
                    }
                    Ok(Boot {
                        last_thread: s.last_thread().await?,
                        threads: s.list_threads("").await?,
                        tool_enabled,
                    })
                })
            })
        })
        .join()
        .map_err(|_| anyhow::anyhow!("Recon boot worker panicked"))?
    }

    pub(crate) fn threads(query: &str) -> Result<(Vec<Thread>, HashMap<String, String>)> {
        with(|s| {
            Box::pin(async move {
                let rows = s.list_threads(query).await?;
                let mut states = HashMap::new();
                for t in &rows {
                    states.insert(
                        t.id.clone(),
                        s.latest_run_state(&t.id).await?.unwrap_or_default(),
                    );
                }
                Ok((rows, states))
            })
        })
    }

    /// The open thread. Investigation events and call logs live in the
    /// SQLite event log and stay empty here.
    pub(crate) fn snapshot(id: &str) -> Result<ReconSnapshot> {
        let snapshot = with(|s| {
            Box::pin(async move {
                Ok(ReconSnapshot {
                    messages: s.list_messages(id).await?,
                    events: Vec::new(),
                    calls: s.all_calls_for_thread(id).await?,
                    runs: s.runs_for_thread(id).await?,
                    memories: s.answer_memories(id).await?,
                    memory_summaries: HashMap::new(),
                    logged_calls: Vec::new(),
                    log_error: None,
                })
            })
        })?;
        let mut snapshot = snapshot;
        let ids: std::collections::HashSet<String> = snapshot
            .memories
            .values()
            .flatten()
            .map(|m| m.id.clone())
            .collect();
        for memory_id in ids {
            if let Ok(Some(insight)) = super::super::surreal_brain::preview(&memory_id) {
                snapshot.memory_summaries.insert(
                    memory_id,
                    super::super::db_reads::format_insight_summary(&insight),
                );
            }
        }
        Ok(snapshot)
    }

    pub(crate) fn manual_calls() -> Result<Vec<argos_osint_core::recon::Call>> {
        with(|s| Box::pin(async move { s.manual_calls().await }))
    }

    pub(crate) fn save_draft(id: &str, draft: &str, scroll: i64) -> Result<()> {
        with(|s| Box::pin(async move { s.save_draft(id, draft, scroll).await }))
    }

    pub(crate) fn select_thread(id: &str) -> Result<()> {
        with(|s| Box::pin(async move { s.select_thread(id).await }))
    }

    pub(crate) fn set_recall_insights(id: &str, on: bool) -> Result<bool> {
        with(|s| Box::pin(async move { s.set_recall_insights(id, on).await }))
    }

    pub(crate) fn set_tool_enabled(id: &str, enabled: bool) -> Result<()> {
        with(|s| Box::pin(async move { s.set_tool_enabled(id, enabled).await }))
    }

    pub(crate) fn rename_thread(id: &str, title: &str) -> Result<bool> {
        with(|s| Box::pin(async move { s.rename_thread(id, title).await }))
    }

    /// Create a thread, attaching `call` (a Tools result) when given.
    pub(crate) fn new_thread(title: &str, call: Option<&str>) -> Result<Thread> {
        with(|s| {
            Box::pin(async move {
                let thread = s.new_thread(title).await?;
                if let Some(call) = call {
                    s.attach_call(call, &thread.id).await?;
                }
                Ok(thread)
            })
        })
    }

    pub(crate) fn attach_call(call: &str, thread: &str) -> Result<()> {
        with(|s| {
            Box::pin(async move {
                anyhow::ensure!(
                    s.attach_call(call, thread).await?,
                    "Call or thread not found"
                );
                Ok(())
            })
        })
    }

    pub(crate) fn delete_thread(id: &str, with_insights: bool) -> Result<bool> {
        with(|s| Box::pin(async move { s.delete_thread(id, with_insights).await }))
    }
}

#[cfg(not(feature = "surreal"))]
mod imp {
    use super::*;
    use argos_osint_core::recon::Call;

    fn off<T>() -> Result<T> {
        unreachable!("SurrealDB Recon is off without the surreal feature")
    }
    pub(crate) fn boot() -> Result<Boot> {
        off()
    }
    pub(crate) fn threads(_: &str) -> Result<(Vec<Thread>, HashMap<String, String>)> {
        off()
    }
    pub(crate) fn snapshot(_: &str) -> Result<ReconSnapshot> {
        off()
    }
    pub(crate) fn manual_calls() -> Result<Vec<Call>> {
        off()
    }
    pub(crate) fn save_draft(_: &str, _: &str, _: i64) -> Result<()> {
        off()
    }
    pub(crate) fn select_thread(_: &str) -> Result<()> {
        off()
    }
    pub(crate) fn set_recall_insights(_: &str, _: bool) -> Result<bool> {
        off()
    }
    pub(crate) fn set_tool_enabled(_: &str, _: bool) -> Result<()> {
        off()
    }
    pub(crate) fn rename_thread(_: &str, _: &str) -> Result<bool> {
        off()
    }
    pub(crate) fn new_thread(_: &str, _: Option<&str>) -> Result<Thread> {
        off()
    }
    pub(crate) fn attach_call(_: &str, _: &str) -> Result<()> {
        off()
    }
    pub(crate) fn delete_thread(_: &str, _: bool) -> Result<bool> {
        off()
    }
}

pub(crate) use imp::*;
