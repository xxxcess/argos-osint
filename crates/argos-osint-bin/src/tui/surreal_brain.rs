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
