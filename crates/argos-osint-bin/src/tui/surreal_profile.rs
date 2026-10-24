//! The TUI Profile screen on SurrealDB. Under a storage config ([`on`]) the
//! dashboard snapshot, its filter options and the attention actions' run
//! lookups read only the repository (never mixed with the SQLite store),
//! through `storage::runtime::blocking`. Callers are plain worker threads
//! that report back by `WorkEvent` (the Jobs worker pattern), never async
//! code or the draw loop.

use anyhow::Result;
use argos_osint_core::profile_stats::{ProfileSnapshot, StatFilters};

pub(crate) fn on() -> bool {
    super::surreal_brain::on()
}

/// One dashboard snapshot for `filters` and the filter options, read from
/// one load of the Profile source tables.
#[cfg(feature = "surreal")]
pub(crate) fn dashboard(
    filters: &StatFilters,
) -> Result<(ProfileSnapshot, Vec<(String, Vec<String>)>)> {
    use argos_osint_core::storage::{profile_view, runtime};
    let filters = filters.clone();
    runtime::blocking(|repo| async move {
        profile_view::dashboard(repo.db(), &filters.period, &filters, chrono::Utc::now()).await
    })
}

#[cfg(not(feature = "surreal"))]
pub(crate) fn dashboard(
    _filters: &StatFilters,
) -> Result<(ProfileSnapshot, Vec<(String, Vec<String>)>)> {
    unreachable!("SurrealDB Profile is off without the surreal feature")
}

/// The thread of a Recon run an attention item points at, or `None` when the
/// run is no longer retained.
#[cfg(feature = "surreal")]
pub(crate) fn run_thread(run_id: &str) -> Result<Option<String>> {
    use argos_osint_core::storage::{recon_runs, runtime};
    runtime::blocking(|repo| async move {
        Ok(recon_runs::get_run(&repo, run_id)
            .await?
            .map(|run| run.thread_id))
    })
}

#[cfg(not(feature = "surreal"))]
pub(crate) fn run_thread(_run_id: &str) -> Result<Option<String>> {
    unreachable!("SurrealDB Profile is off without the surreal feature")
}
