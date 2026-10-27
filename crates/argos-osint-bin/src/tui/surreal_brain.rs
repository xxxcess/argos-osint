//! The TUI Brain on SurrealDB. Under a storage config the Brain list,
//! preview, delete and the Brain topology run only on the repository
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
        // The TUI unit tests drive the SQLite store directly.
        if cfg!(test) {
            return false;
        }
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

/// A retained Intel report job and the run of its article's latest
/// sighting, for Profile → Attention "Open report" (`intel_report_job`,
/// `atlas_article`). A short keyed read, like the Brain preview.
#[cfg(feature = "surreal")]
pub(crate) fn report_job_with_run(
    id: &str,
) -> Result<
    Option<(
        argos_osint_core::intel_recon::IntelReportJobRow,
        Option<String>,
    )>,
> {
    use argos_osint_core::storage::{atlas, recon_jobs, runtime};
    runtime::blocking(|repo| async move {
        let Some(job) = recon_jobs::report_job(&repo, id).await? else {
            return Ok(None);
        };
        let run = atlas::latest_article_by_id(repo.db(), &job.article_id)
            .await?
            .map(|a| a.run_id);
        Ok(Some((job, run)))
    })
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

/// The memory check before Enter opens a memory's topology: its kind, or
/// `None` when it no longer exists.
#[cfg(feature = "surreal")]
pub(crate) fn memory_kind(
    id: &str,
) -> Result<Option<argos_osint_core::memory_metadata::MemoryKind>> {
    use argos_osint_core::storage::{runtime, topology};
    runtime::blocking(|repo| async move { topology::memory_seed_kind(&repo, id).await })
}

#[cfg(not(feature = "surreal"))]
pub(crate) fn memory_kind(
    _id: &str,
) -> Result<Option<argos_osint_core::memory_metadata::MemoryKind>> {
    unreachable!("SurrealDB Brain is off without the surreal feature")
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
    use argos_osint_core::events::{NewEvent, Severity};
    use argos_osint_core::job_registry::CancelRequest;
    use argos_osint_core::storage::{event_log, jobs_view, runtime};
    match action {
        JobAction::Retry(id) => {
            let job_id = id.clone();
            let n = runtime::blocking(move |repo| async move {
                let n = jobs_view::retry_failed_tasks(repo.db(), &id).await?;
                if n > 0 {
                    let event = NewEvent {
                        severity: Some(Severity::Info),
                        app: "jobs".into(),
                        event_type: "jobs.retry".into(),
                        message: format!("Retrying {n} failed task(s)"),
                        job_id,
                        ..Default::default()
                    };
                    let _ = event_log::record_event(repo.db(), &event).await;
                }
                Ok(n)
            })?;
            Ok(if n == 0 {
                "Nothing to retry".into()
            } else {
                format!("Retrying {n} failed task(s)")
            })
        }
        JobAction::Cancel(id) => {
            let job_id = id.clone();
            let log_job_id = id.clone();
            let outcome = runtime::blocking(move |repo| async move {
                let outcome = jobs_view::cancel_job(repo.db(), &id).await?;
                if outcome == CancelRequest::Requested {
                    let event = NewEvent {
                        severity: Some(Severity::Info),
                        app: "jobs".into(),
                        event_type: "jobs.cancel".into(),
                        message: "Stop requested · the job stops at its next safe point".into(),
                        job_id: log_job_id,
                        ..Default::default()
                    };
                    let _ = event_log::record_event(repo.db(), &event).await;
                }
                Ok(outcome)
            })?;
            if outcome == CancelRequest::Requested {
                argos_osint_core::job_registry::raise_local_cancel(&job_id);
            }
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
            let label = status.label().to_lowercase();
            let n = runtime::blocking(move |repo| async move {
                let n = jobs_view::clear_jobs(repo.db(), status).await?;
                let event = NewEvent {
                    severity: Some(Severity::Info),
                    app: "jobs".into(),
                    event_type: "jobs.clear".into(),
                    message: format!("Cleared {n} {label} job(s)"),
                    ..Default::default()
                };
                let _ = event_log::record_event(repo.db(), &event).await;
                Ok(n)
            })?;
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

/// Batches per job retention sweep (each is one bounded transaction).
#[cfg(feature = "surreal")]
const SWEEP_BATCHES: usize = 64;

/// SurrealDB job retention: one sweep now, then hourly, on its own plain
/// thread for the life of the TUI (the SQLite `clear_old_jobs` never runs
/// under a storage config).
#[cfg(feature = "surreal")]
pub(crate) fn spawn_job_sweeper() {
    use argos_osint_core::storage::{jobs_view, runtime};
    std::thread::spawn(|| loop {
        // A failed sweep retries next hour; the Jobs pane already hides
        // expired jobs, so nothing is shown stale meanwhile.
        let _ = runtime::blocking(|repo| async move {
            jobs_view::sweep_expired(repo.db(), chrono::Utc::now(), SWEEP_BATCHES).await
        });
        std::thread::sleep(std::time::Duration::from_secs(3600));
    });
}

#[cfg(not(feature = "surreal"))]
pub(crate) fn spawn_job_sweeper() {
    unreachable!("SurrealDB Jobs is off without the surreal feature")
}

/// Intel "Retry extraction" on the repository: the article's shared refresh
/// is retried by `refresh_driver::retry_owner` (queued task runs, ended task
/// re-enqueued at a new generation) on a plain worker thread, so the draw
/// loop never waits on the model run. Errors go to the event log.
#[cfg(feature = "surreal")]
pub(crate) fn retry_article_extraction(
    article_id: String,
    auth: argos_osint_core::secrets::AuthFile,
    settings: argos_osint_core::provider::SettingsFile,
) {
    use argos_osint_core::storage::{refresh_driver, runtime};
    let _ = std::thread::Builder::new()
        .name("argos-intel-retry".into())
        .spawn(move || {
            let result = runtime::blocking(move |repo| async move {
                let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                let execute =
                    refresh_driver::pipeline(&auth, &settings, &cancel, std::path::PathBuf::new());
                refresh_driver::retry_owner(
                    &repo,
                    "article",
                    &article_id,
                    &settings,
                    &cancel,
                    None,
                    execute,
                )
                .await
            });
            if let Err(error) = result {
                let event = argos_osint_core::events::NewEvent {
                    severity: Some(argos_osint_core::events::Severity::Error),
                    app: "intel".into(),
                    event_type: "insight.retry_error".into(),
                    message: format!("{error:#}"),
                    ..Default::default()
                };
                let _ = runtime::blocking(move |repo| async move {
                    argos_osint_core::storage::event_log::record_event(repo.db(), &event)
                        .await
                        .map_err(|e| anyhow::anyhow!("{e}"))
                });
            }
        });
}

#[cfg(not(feature = "surreal"))]
pub(crate) fn retry_article_extraction(
    _article_id: String,
    _auth: argos_osint_core::secrets::AuthFile,
    _settings: argos_osint_core::provider::SettingsFile,
) {
}

/// The Logs pane on the repository's `argos_event`. Plain worker threads
/// only.
#[cfg(feature = "surreal")]
pub(crate) struct SurrealLogs;

#[cfg(feature = "surreal")]
impl super::logs::LogsSource for SurrealLogs {
    fn list(
        &self,
        filter: &argos_osint_core::events::EventFilter,
        limit: usize,
    ) -> Result<Vec<argos_osint_core::events::EventRow>> {
        use argos_osint_core::storage::{event_log, runtime};
        runtime::blocking(
            |repo| async move { event_log::list_events(repo.db(), filter, limit).await },
        )
    }
    fn counts(&self) -> Result<argos_osint_core::jobs_view::EventCounts> {
        use argos_osint_core::storage::{event_log, runtime};
        runtime::blocking(|repo| async move { event_log::event_counts(repo.db()).await })
    }
    fn apps(&self) -> Result<Vec<String>> {
        use argos_osint_core::storage::{event_log, runtime};
        runtime::blocking(|repo| async move { event_log::event_apps(repo.db()).await })
    }
}

/// Reload a Logs snapshot from the repository. Plain worker threads only.
#[cfg(feature = "surreal")]
pub(crate) fn load_logs(logs: &mut super::logs::LogsView) {
    logs.reload(&SurrealLogs);
}

#[cfg(not(feature = "surreal"))]
pub(crate) fn load_logs(_logs: &mut super::logs::LogsView) {
    unreachable!("SurrealDB Logs is off without the surreal feature")
}

/// The Logs header counts from the repository. Plain worker threads only.
#[cfg(feature = "surreal")]
pub(crate) fn log_counts() -> Result<argos_osint_core::jobs_view::EventCounts> {
    use super::logs::LogsSource;
    SurrealLogs.counts()
}

#[cfg(not(feature = "surreal"))]
pub(crate) fn log_counts() -> Result<argos_osint_core::jobs_view::EventCounts> {
    unreachable!("SurrealDB Logs is off without the surreal feature")
}

/// Record one session log line in `argos_event` without blocking the UI
/// thread (ordered, so lines keep their order; drained before exit).
/// False when the write could not be started.
#[cfg(feature = "surreal")]
pub(crate) fn record_log(event: argos_osint_core::events::NewEvent) -> bool {
    use argos_osint_core::storage::{event_log, runtime};
    runtime::spawn_ordered(async move {
        if let Ok(repo) = runtime::repository().await {
            let _ = event_log::record_event(repo.db(), &event).await;
        }
    })
}

#[cfg(not(feature = "surreal"))]
pub(crate) fn record_log(_event: argos_osint_core::events::NewEvent) -> bool {
    unreachable!("SurrealDB Logs is off without the surreal feature")
}

/// Logs retention on the repository: `argos_event` past `hours` and the
/// rate-limited telemetry prune. Plain worker threads only.
#[cfg(feature = "surreal")]
pub(crate) fn prune_logs(hours: i64) -> Result<usize> {
    use argos_osint_core::storage::{event_log, runtime, telemetry};
    runtime::blocking(|repo| async move {
        let pruned = event_log::prune_events(repo.db(), hours).await?;
        telemetry::prune(repo.db(), chrono::Utc::now()).await?;
        Ok(pruned)
    })
}

#[cfg(not(feature = "surreal"))]
pub(crate) fn prune_logs(_hours: i64) -> Result<usize> {
    unreachable!("SurrealDB Logs is off without the surreal feature")
}

/// Logs "Clear log" on the repository (`argos_event` only). Plain worker
/// threads only. Records the post-clear session event in the same worker
/// call so the subsequent `reload_logs` snapshot sees it deterministically.
#[cfg(feature = "surreal")]
pub(crate) fn clear_logs() -> Result<usize> {
    use argos_osint_core::storage::{event_log, runtime};
    let cleared =
        runtime::blocking(|repo| async move { event_log::clear_events(repo.db()).await })?;
    let event = super::logs::session_event(
        "info",
        "Events cleared (jobs, results, and memories kept)",
        "",
    );
    let _ =
        runtime::blocking(
            move |repo| async move { event_log::record_event(repo.db(), &event).await },
        );
    Ok(cleared)
}

#[cfg(not(feature = "surreal"))]
pub(crate) fn clear_logs() -> Result<usize> {
    unreachable!("SurrealDB Logs is off without the surreal feature")
}

/// Intel Related leads on the repository (`related_articles_global`).
#[cfg(feature = "surreal")]
pub(crate) fn related_global(
    run_id: &str,
    article_id: &str,
    scope: argos_osint_core::related_global::RelatedScope,
    cursor: Option<&str>,
    limit: usize,
) -> Result<argos_osint_core::related_global::GlobalRelatedPage> {
    use argos_osint_core::related_global::related_articles_global_on;
    use argos_osint_core::storage::runtime;
    runtime::blocking(|repo| async move {
        related_articles_global_on(&repo, run_id, article_id, scope, cursor, limit).await
    })
}

#[cfg(not(feature = "surreal"))]
pub(crate) fn related_global(
    _run_id: &str,
    _article_id: &str,
    _scope: argos_osint_core::related_global::RelatedScope,
    _cursor: Option<&str>,
    _limit: usize,
) -> Result<argos_osint_core::related_global::GlobalRelatedPage> {
    unreachable!("SurrealDB Related is off without the surreal feature")
}

/// The Intel brief's repository-backed parts for one article.
#[cfg(feature = "surreal")]
pub(crate) fn brief_parts(
    article: &argos_osint_core::store::AtlasArticleRow,
) -> Result<argos_osint_core::storage::intel_brief_view::BriefParts> {
    use argos_osint_core::storage::{intel_brief_view, runtime};
    runtime::blocking(|repo| async move { intel_brief_view::brief_parts(&repo, article).await })
}
