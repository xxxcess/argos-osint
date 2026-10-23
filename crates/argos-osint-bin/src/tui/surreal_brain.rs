//! The TUI Brain on SurrealDB. Under a storage config the Brain list,
//! preview, delete and the Actor-group topology run only on the repository
//! (never mixed with the SQLite store), through `storage::runtime::blocking`. Callers are the
//! Brain's plain worker threads, never async code or the draw loop.

use anyhow::Result;
use argos_osint_core::brain::Memory;
use argos_osint_core::recon::InsightView;

#[cfg(feature = "surreal")]
pub(crate) fn on() -> bool {
    use argos_osint_core::storage::home;
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        // An unreadable config is configured: its errors surface on the
        // SurrealDB path rather than silently falling back to SQLite.
        argos_osint_core::paths::ensure_home()
            .and_then(|h| home::load_config(&home::Layout::new(h)))
            .map_or(true, |config| config.is_some())
    })
}

#[cfg(not(feature = "surreal"))]
pub(crate) fn on() -> bool {
    false
}

#[cfg(feature = "surreal")]
pub(crate) fn list() -> Result<Vec<Memory>> {
    use argos_osint_core::storage::{browse_assoc, runtime};
    runtime::blocking(
        |repo| async move { browse_assoc::brain_browse_memories(&repo, None, "").await },
    )
}

#[cfg(feature = "surreal")]
pub(crate) fn preview(id: &str) -> Result<Option<InsightView>> {
    use argos_osint_core::storage::{insight_views, runtime};
    runtime::blocking(|repo| async move { insight_views::insight_for_memory(&repo, id).await })
}

#[cfg(feature = "surreal")]
pub(crate) fn delete(id: &str) -> Result<bool> {
    use argos_osint_core::storage::{browse, runtime};
    runtime::blocking(|repo| async move { browse::delete_memory(&repo, id).await })
}

#[cfg(feature = "surreal")]
pub(crate) fn topology(
    seed: &argos_osint_core::brain_topology::TopologySeed,
    options: &argos_osint_core::brain_topology::TopologyOptions,
) -> Result<argos_osint_core::brain_topology::TopologySnapshot> {
    use argos_osint_core::storage::{runtime, topology};
    runtime::blocking(|repo| async move { topology::topology_snapshot(&repo, seed, options).await })
}

#[cfg(not(feature = "surreal"))]
pub(crate) fn topology(
    _seed: &argos_osint_core::brain_topology::TopologySeed,
    _options: &argos_osint_core::brain_topology::TopologyOptions,
) -> Result<argos_osint_core::brain_topology::TopologySnapshot> {
    unreachable!("SurrealDB Brain is off without the surreal feature")
}

#[cfg(not(feature = "surreal"))]
pub(crate) fn list() -> Result<Vec<Memory>> {
    unreachable!("SurrealDB Brain is off without the surreal feature")
}

#[cfg(not(feature = "surreal"))]
pub(crate) fn preview(_id: &str) -> Result<Option<InsightView>> {
    unreachable!("SurrealDB Brain is off without the surreal feature")
}

#[cfg(not(feature = "surreal"))]
pub(crate) fn delete(_id: &str) -> Result<bool> {
    unreachable!("SurrealDB Brain is off without the surreal feature")
}

/// The Jobs pane on the repository. Plain worker threads only.
#[cfg(feature = "surreal")]
pub(crate) struct SurrealJobs;

#[cfg(feature = "surreal")]
impl super::jobs::JobsSource for SurrealJobs {
    fn list(
        &self,
        filter: &argos_osint_core::jobs_view::JobFilter,
        limit: usize,
    ) -> Result<Vec<argos_osint_core::jobs_view::JobRow>> {
        use argos_osint_core::storage::{jobs_view, runtime};
        runtime::blocking(
            |repo| async move { jobs_view::list_jobs(repo.db(), filter, limit).await },
        )
    }
    fn counts(&self) -> Result<argos_osint_core::jobs_view::JobCounts> {
        use argos_osint_core::storage::{jobs_view, runtime};
        runtime::blocking(|repo| async move { jobs_view::job_counts(repo.db()).await })
    }
    fn apps(&self) -> Result<Vec<String>> {
        use argos_osint_core::storage::{jobs_view, runtime};
        runtime::blocking(|repo| async move { jobs_view::job_apps(repo.db()).await })
    }
    fn detail(
        &self,
        id: &str,
        tasks: usize,
    ) -> Result<Option<argos_osint_core::jobs_view::JobDetail>> {
        use argos_osint_core::storage::{jobs_view, runtime};
        runtime::blocking(|repo| async move { jobs_view::job_detail(repo.db(), id, tasks).await })
    }
    fn job(&self, id: &str) -> Result<Option<argos_osint_core::jobs_view::JobRow>> {
        use argos_osint_core::storage::{jobs_view, runtime};
        runtime::blocking(|repo| async move { jobs_view::get_job(repo.db(), id).await })
    }
}

/// Jobs pane actions on the repository (Retry, Stop, Clear history), as the
/// status line text. Plain worker threads only.
#[cfg(feature = "surreal")]
pub(crate) fn job_action(action: JobAction) -> Result<String> {
    use argos_osint_core::job_registry::CancelRequest;
    use argos_osint_core::storage::{jobs_view, runtime};
    match action {
        JobAction::Retry(id) => {
            let n = runtime::blocking(|repo| async move {
                jobs_view::retry_failed_tasks(repo.db(), &id).await
            })?;
            Ok(if n == 0 {
                "Nothing to retry".into()
            } else {
                format!("Retrying {n} failed task(s)")
            })
        }
        JobAction::Cancel(id) => {
            let outcome =
                runtime::blocking(
                    |repo| async move { jobs_view::cancel_job(repo.db(), &id).await },
                )?;
            Ok(match outcome {
                CancelRequest::Requested => {
                    "Stop requested · the job stops at its next safe point".into()
                }
                CancelRequest::NotCancellable => "This job cannot be cancelled safely".into(),
                CancelRequest::NotRunning => "This job is not running".into(),
                CancelRequest::Missing => "Job history expired".into(),
            })
        }
        JobAction::Clear(status) => {
            let n =
                runtime::blocking(
                    |repo| async move { jobs_view::clear_jobs(repo.db(), status).await },
                )?;
            Ok(format!("Cleared {n} jobs"))
        }
    }
}

#[cfg(not(feature = "surreal"))]
pub(crate) fn job_action(action: JobAction) -> Result<String> {
    let target = match action {
        JobAction::Retry(id) | JobAction::Cancel(id) => id,
        JobAction::Clear(status) => status.label().to_string(),
    };
    unreachable!("SurrealDB Jobs is off without the surreal feature ({target})")
}

/// A Jobs pane action for [`job_action`].
pub(crate) enum JobAction {
    Retry(String),
    Cancel(String),
    Clear(argos_osint_core::jobs_view::JobStatusFilter),
}

/// Load a Jobs snapshot from the repository (focus `pending` when set).
/// Plain worker threads only.
#[cfg(feature = "surreal")]
pub(crate) fn load_jobs(jobs: &mut super::jobs::JobsView, pending: Option<&str>) {
    match pending {
        Some(id) => {
            if !jobs.focus_job(&SurrealJobs, id) {
                jobs.error = Some("Job history expired".into());
            }
        }
        None => jobs.reload(&SurrealJobs),
    }
}

#[cfg(not(feature = "surreal"))]
pub(crate) fn load_jobs(_jobs: &mut super::jobs::JobsView, _pending: Option<&str>) {
    unreachable!("SurrealDB Jobs is off without the surreal feature")
}
