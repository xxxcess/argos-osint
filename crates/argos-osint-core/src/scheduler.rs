//! Elected single-owner scheduler for a shared SQLite state root (spec §4).
//!
//! One process holds the `argos_scheduler_lease` row. Others observe only.
//! Worker pools are separated by kind: LLM, network collect, local index.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;

use crate::tasks::{self, OperationKind};

pub const SCHEDULER_LEASE_KEY: &str = "argos_scheduler";
pub const DEFAULT_LEASE_SECS: i64 = 30;

/// Backend factory for the Intel brief drainer.
pub type IntelBackendFactory =
    Arc<dyn Fn() -> Box<dyn crate::intel_recon::brief_stages::StageBackend + Send> + Send + Sync>;

/// A given error from a given drainer is surfaced at most once per window.
const DRAINER_ERROR_REPEAT: Duration = Duration::from_secs(300);
/// Upper bound on remembered (drainer, message) pairs.
const DRAINER_ERROR_CAP: usize = 128;

type Seen = std::collections::HashMap<(String, String), std::time::Instant>;

/// Decide whether a drainer error should be surfaced now. Keyed by
/// (drainer name, redacted message), so alternating errors A, B, A each log
/// once per window. Entries older than [`DRAINER_ERROR_REPEAT`] are evicted and
/// the map never exceeds [`DRAINER_ERROR_CAP`] (oldest dropped first).
fn should_surface(seen: &mut Seen, name: &str, message: &str, at: std::time::Instant) -> bool {
    seen.retain(|_, when| at.saturating_duration_since(*when) < DRAINER_ERROR_REPEAT);
    let key = (name.to_string(), message.to_string());
    if seen.contains_key(&key) {
        return false;
    }
    while seen.len() >= DRAINER_ERROR_CAP {
        let Some(oldest) = seen
            .iter()
            .min_by_key(|(_, when)| **when)
            .map(|(k, _)| k.clone())
        else {
            break;
        };
        seen.remove(&oldest);
    }
    seen.insert(key, at);
    true
}

/// Replace local paths in `message`: the `ARGOS_HOME` override becomes
/// `$ARGOS_HOME`, and the user's home directory (`HOME` and the platform home)
/// becomes `~`. Longest path first, so an `ARGOS_HOME` inside the home
/// directory wins over `~`. `events::redact` does not cover paths.
fn redact_paths(message: &str, argos_home: Option<&str>, homes: &[&str]) -> String {
    let mut pairs: Vec<(&str, &str)> = Vec::new();
    if let Some(path) = argos_home {
        pairs.push((path, "$ARGOS_HOME"));
    }
    pairs.extend(homes.iter().map(|path| (*path, "~")));
    pairs.retain(|(path, _)| path.trim_end_matches('/').len() > 1);
    pairs.sort_by_key(|(path, _)| std::cmp::Reverse(path.len()));
    let mut out = message.to_string();
    for (path, replacement) in pairs {
        out = out.replace(path.trim_end_matches('/'), replacement);
    }
    out
}

fn redact_local_paths(message: &str) -> String {
    let argos_home = std::env::var("ARGOS_HOME")
        .ok()
        .filter(|v| !v.trim().is_empty());
    let env_home = std::env::var("HOME").ok().filter(|v| !v.trim().is_empty());
    let dirs_home = dirs::home_dir().map(|p| p.to_string_lossy().into_owned());
    let mut homes: Vec<&str> = Vec::new();
    homes.extend(env_home.as_deref());
    homes.extend(dirs_home.as_deref());
    redact_paths(message, argos_home.as_deref(), &homes)
}

/// Surface a drainer failure as a redacted (secrets and local paths),
/// rate-limited error event (Logs) instead of discarding it.
fn report_drainer<T>(_db_path: &std::path::Path, name: &str, result: Result<T>) {
    static SEEN: std::sync::OnceLock<std::sync::Mutex<Seen>> = std::sync::OnceLock::new();
    let Err(error) = result else {
        return;
    };
    let message = redact_local_paths(&format!("{error:#}"));
    let fresh = SEEN
        .get_or_init(Default::default)
        .lock()
        .map(|mut seen| should_surface(&mut seen, name, &message, std::time::Instant::now()))
        .unwrap_or(true);
    if !fresh {
        return;
    }
    let event = crate::events::NewEvent {
        severity: Some(crate::events::Severity::Error),
        app: "system".into(),
        event_type: "drainer.error".into(),
        message: format!("{name} worker error: {message}"),
        ..Default::default()
    };
    crate::storage::runtime::spawn_ordered(async move {
        if let Ok(repo) = crate::storage::runtime::repository().await {
            let _ = crate::storage::event_log::record_event(repo.db(), &event).await;
        }
    });
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PoolKind {
    Llm,
    Network,
    Index,
}

impl PoolKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Llm => "llm",
            Self::Network => "network",
            Self::Index => "index",
        }
    }
}

/// Apply one index change with a typed outcome. Opening the store or any index
/// error is a typed failure, never an "upserted" string.
pub fn apply_index_change(
    db_path: &std::path::Path,
    kind: &str,
    id: &str,
    operation: &str,
) -> tasks::IndexOutcome {
    match crate::store::Store::open(db_path) {
        Ok(store) => apply_index_with(&store, kind, id, operation),
        Err(err) => tasks::IndexOutcome::RetryableFailure {
            message: format!("open store: {err:#}"),
        },
    }
}

/// Apply one index change against an open store.
pub fn apply_index_with(
    store: &crate::store::Store,
    _kind: &str,
    id: &str,
    operation: &str,
) -> tasks::IndexOutcome {
    match operation {
        "rebuild" | "generation_rebuild" | "index_rebuild" => store.try_process_vector_rebuild(4),
        "upsert" | "index_upsert" => store.try_index_upsert(&[id.to_string()]),
        "remove" | "index_remove" => store.try_index_remove_missing(&[id.to_string()]),
        other => tasks::IndexOutcome::PermanentFailure {
            message: format!("unsupported index operation `{other}`"),
        },
    }
}

/// One finished summary flush task, with its truthful terminal outcome.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlushOutcome {
    pub task_id: String,
    pub operation: String,
    pub cache_key: String,
    pub tag: String,
    /// A complete generated (non-fallback) summary is cached. `llm_fallback`,
    /// rejections and failures are not success.
    pub generated: bool,
    /// The lease was lost; the task was neither published nor completed.
    pub stale: bool,
}

/// A dependency that is either supplied or loaded on first use. The idle
/// worker tick therefore reads no auth/settings files: they load only after a
/// summary task has actually been claimed, once per drain.
pub(crate) struct Lazy<'a, T> {
    given: Option<Option<&'a T>>,
    load: Option<fn() -> Option<T>>,
    cell: std::cell::OnceCell<Option<T>>,
}

impl<'a, T> Lazy<'a, T> {
    pub(crate) fn given(value: Option<&'a T>) -> Self {
        Self {
            given: Some(value),
            load: None,
            cell: std::cell::OnceCell::new(),
        }
    }

    pub(crate) fn loaded_on_demand(load: fn() -> Option<T>) -> Self {
        Self {
            given: None,
            load: Some(load),
            cell: std::cell::OnceCell::new(),
        }
    }

    fn get(&self) -> Option<&T> {
        if let Some(value) = self.given {
            return value;
        }
        self.cell
            .get_or_init(|| self.load.and_then(|load| load()))
            .as_ref()
    }
}

/// The worker's full role runtime (all routes), when auth and settings load.
fn load_role_runtime() -> Option<crate::recon::model_exec::RoleRuntime> {
    Some(crate::recon::model_exec::RoleRuntime::new(
        crate::secrets::AuthFile::load().ok()?,
        crate::provider::SettingsFile::load().ok()?,
    ))
}

fn load_summarization_secret() -> Option<crate::secrets::ProviderSecret> {
    let auth = crate::secrets::AuthFile::load().ok()?;
    let settings = crate::provider::SettingsFile::load().ok()?;
    // First prose-capable route in the complete Summarization chain.
    crate::recon::model_exec::select_role_secret(
        &auth,
        &settings,
        "summarization",
        crate::recon::model_exec::OperationFormat::Prose,
    )
}

/// Service-health job for the summary flush pool.
pub const SUMMARY_SERVICE_JOB: &str = "svc-summary-flush";

/// Stable worker-owner id for this process (shared by all its pools so
/// election stays coherent across processes sharing one state root).
pub fn process_owner() -> String {
    static OWNER: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    OWNER
        .get_or_init(|| {
            format!(
                "argos-{}-{}",
                std::process::id(),
                chrono::Utc::now().timestamp_millis()
            )
        })
        .clone()
}

/// Background worker handle. Dropping / cancelling stops the loop.
pub struct WorkerPool {
    stop: Arc<AtomicBool>,
}

impl WorkerPool {
    pub fn spawn_index_drainer(db_path: std::path::PathBuf, owner: String) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        // Under SurrealDB the index pool is the repository's; only the Atlas
        // memory-phase refresh runs here, never reopening SQLite.
        #[cfg(feature = "surreal")]
        crate::storage::runtime::spawn(async move {
            while !flag.load(Ordering::Relaxed) {
                if let Ok(store) = crate::atlas_store::AtlasStore::open(&db_path).await {
                    let _ = crate::atlas_memory::refresh_incomplete_runs_store(&store, 8).await;
                }
                for _ in 0..60 {
                    if flag.load(Ordering::Relaxed) {
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
            }
        });
        let _ = owner;
        Self { stop }
    }

    /// Drains summarization flush tasks (summary pool only) while this process
    /// holds the scheduler election.
    pub fn spawn_summary_flush_drainer(db_path: std::path::PathBuf, owner: String) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        std::thread::Builder::new()
            .name("argos-summary-pool".into())
            .spawn(move || {
                let _loops = 0u32;
                let _schema_ready = false;
                while !flag.load(Ordering::Relaxed) {
                    let owner = owner.clone();
                    let drained = crate::storage::runtime::blocking(move |repo| async move {
                        if !surreal_elected(&repo, &owner).await? {
                            return Ok(0);
                        }
                        let out = crate::storage::summary_flush::drain(
                            &repo,
                            &owner,
                            load_summarization_secret,
                            DEFAULT_LEASE_SECS,
                        )
                        .await?;
                        Ok(out.len())
                    });
                    let busy = matches!(drained, Ok(n) if n > 0);
                    report_drainer(&db_path, "summary", drained);
                    if !busy {
                        std::thread::sleep(Duration::from_millis(750));
                    }
                }
            })
            .ok();
        Self { stop }
    }

    /// Spawn the default local pools (index + summary) for one state root.
    pub fn spawn_default(db_path: std::path::PathBuf) -> Vec<WorkerPool> {
        let owner = process_owner();
        // Under a storage config nothing may open SQLite: each pool below
        // has a repository drainer.
        #[cfg(feature = "surreal")]
        return vec![
            Self::spawn_surreal_index_drainer(db_path.clone(), owner.clone()),
            Self::spawn_summary_flush_drainer(db_path.clone(), owner.clone()),
            Self::spawn_insight_refresh_drainer(db_path.clone(), owner.clone()),
            Self::spawn_brain_backfill_drainer(db_path.clone(), owner.clone()),
            Self::spawn_actor_dedup_drainer(db_path.clone(), owner.clone()),
            Self::spawn_intel_brief_drainer(db_path.clone(), owner.clone()),
            Self::spawn_brain_scope_drainer(db_path, owner),
        ];
        #[cfg(not(feature = "surreal"))]
        vec![Self::spawn_insight_refresh_drainer(db_path, owner)]
    }

    /// The index pool on SurrealDB: while elected on the repository's
    /// scheduler lease, embed pending memory changes into the serving vector
    /// generation (`storage::index_drain`). Off when local embedding is off.
    #[cfg(feature = "surreal")]
    pub fn spawn_surreal_index_drainer(db_path: std::path::PathBuf, owner: String) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let _ = std::thread::Builder::new()
            .name("argos-index".into())
            .spawn(move || {
                let Some(embedder) = crate::storage::vectors::Embedder::local() else {
                    return;
                };
                let embedder = Arc::new(embedder);
                while !flag.load(Ordering::Acquire) {
                    let (owner, embedder) = (owner.clone(), embedder.clone());
                    let drained = crate::storage::runtime::blocking(move |repo| async move {
                        let elected = crate::storage::jobs::try_elect(
                            repo.db(),
                            &owner,
                            DEFAULT_LEASE_SECS,
                            chrono::Utc::now(),
                        )
                        .await
                        .map_err(|e| anyhow::anyhow!("elect: {e}"))?
                        .value
                        .elected;
                        if !elected {
                            return Ok(0);
                        }
                        let d =
                            crate::storage::index_drain::drain_index(&repo, &embedder, 32).await?;
                        Ok(d.completed + d.failed)
                    });
                    let busy = matches!(drained, Ok(n) if n > 0);
                    report_drainer(&db_path, "index", drained);
                    let pause = if busy { 1 } else { 8 };
                    for _ in 0..pause {
                        if flag.load(Ordering::Acquire) {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(250));
                    }
                }
            });
        Self { stop }
    }

    /// Actor identity maintenance is separate from Atlas collection and never
    /// delays ingestion. Its durable pair leases fence competing processes.
    pub fn spawn_actor_dedup_drainer(db_path: std::path::PathBuf, owner: String) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let _ = std::thread::Builder::new()
            .name("argos-actor-dedup".into())
            .spawn(move || {
                let Ok(_runtime) = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                else {
                    return;
                };
                while !flag.load(Ordering::Acquire) {
                    let (owner, stop) = (owner.clone(), flag.clone());
                    report_drainer(
                        &db_path,
                        "actor-dedup",
                        crate::storage::runtime::blocking(move |repo| async move {
                            if !surreal_elected(&repo, &owner).await? {
                                return Ok(0);
                            }
                            let auth = crate::secrets::AuthFile::load().unwrap_or_default();
                            let settings =
                                crate::provider::SettingsFile::load().unwrap_or_default();
                            crate::storage::actor_dedup::drain(
                                &repo, &owner, &stop, &auth, &settings,
                            )
                            .await
                        }),
                    );
                    for _ in 0..120 {
                        if flag.load(Ordering::Acquire) {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(250));
                    }
                }
            });
        Self { stop }
    }

    /// Local, resumable Brain actor/relationship backfill over every retained
    /// cycle and legacy reference (spec §5). Bounded batches while elected;
    /// idles once coverage is complete. No model/network/embedding work.
    pub fn spawn_brain_backfill_drainer(db_path: std::path::PathBuf, owner: String) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let _ = std::thread::Builder::new()
            .name("argos-brain-backfill".into())
            .spawn(move || {
                while !flag.load(Ordering::Acquire) {
                    let owner = owner.clone();
                    let ran = crate::storage::runtime::blocking(move |repo| async move {
                        if !surreal_elected(&repo, &owner).await? {
                            return Ok(None);
                        }
                        crate::storage::brain_backfill::backfill(
                            &repo,
                            crate::brain_backfill::BATCH,
                        )
                        .await
                        .map(|r| Some(r.complete))
                    });
                    let busy = matches!(ran, Ok(Some(false)));
                    report_drainer(&db_path, "brain-backfill", ran);
                    let pause = if busy { 1 } else { 30 };
                    for _ in 0..pause * 4 {
                        if flag.load(Ordering::Acquire) {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(250));
                    }
                }
            });
        Self { stop }
    }

    /// Dedicated Intel brief stage runner: a bounded two-slot drainer
    /// dispatches ready stages onto separate Tokio tasks. The multi-thread
    /// runtime keeps lease heartbeats independent of blocking SQLite work.
    pub fn spawn_intel_brief_drainer(db_path: std::path::PathBuf, owner: String) -> Self {
        let db = db_path.clone();
        Self::spawn_intel_brief_drainer_with(
            db_path,
            owner,
            Arc::new(move || {
                Box::new(crate::intel_recon::brief_stages::RuntimeBackend::load(&db))
                    as Box<dyn crate::intel_recon::brief_stages::StageBackend + Send>
            }),
        )
    }

    /// [`Self::spawn_intel_brief_drainer`] with an injectable backend factory
    /// (production passes `RuntimeBackend::load`); same loop, same error reporting.
    pub fn spawn_intel_brief_drainer_with(
        db_path: std::path::PathBuf,
        owner: String,
        _make_backend: IntelBackendFactory,
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let _ = std::thread::Builder::new()
            .name("argos-intel-brief".into())
            .spawn(move || {
                let _runtime = match tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(e) => {
                        report_drainer::<()>(
                            &db_path,
                            "intel-brief",
                            Err(anyhow::anyhow!("runtime failed to start: {e}")),
                        );
                        return;
                    }
                };
                let config = crate::intel_recon::brief_stages::ExecConfig::default();
                while !flag.load(Ordering::Acquire) {
                    // SurrealDB: elect on the repository's scheduler lease and
                    // drain the repository's brief chains; never the SQLite file.
                    let (owner, flag, config) = (owner.clone(), flag.clone(), config.clone());
                    report_drainer(
                        &db_path,
                        "intel-brief",
                        crate::storage::runtime::blocking(move |repo| async move {
                            let elected = crate::storage::jobs::try_elect(
                                repo.db(),
                                &owner,
                                DEFAULT_LEASE_SECS,
                                chrono::Utc::now(),
                            )
                            .await
                            .map_err(|e| anyhow::anyhow!("elect: {e}"))?
                            .value
                            .elected;
                            if !elected {
                                return Ok(0);
                            }
                            let r = repo.clone();
                            crate::storage::brief_stages::drain_parallel(
                                &repo,
                                &format!("{owner}:intel-brief"),
                                move || {
                                    Box::new(
                                        crate::storage::brief_stages::SurrealRuntimeBackend::load(
                                            r.clone(),
                                        ),
                                    )
                                        as Box<
                                            dyn crate::intel_recon::brief_stages::StageBackend
                                                + Send,
                                        >
                                },
                                &flag,
                                &config,
                                crate::intel_recon::brief_stages::STAGE_SLOTS,
                            )
                            .await
                        }),
                    );
                    std::thread::sleep(Duration::from_millis(750));
                }
            });
        Self { stop }
    }

    pub fn spawn_insight_refresh_drainer(db_path: std::path::PathBuf, owner: String) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let _ = std::thread::Builder::new()
            .name("argos-insight-refresh".into())
            .spawn(move || {
                let Ok(_runtime) = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                else {
                    return;
                };
                while !flag.load(Ordering::Acquire) {
                    // SurrealDB: elect on the repository's scheduler lease and
                    // drive the shared refresh tasks there (startup recovery
                    // included); never the SQLite file.
                    let (owner, flag) = (owner.clone(), flag.clone());
                    report_drainer(
                        &db_path,
                        "insight-refresh",
                        crate::storage::runtime::blocking(move |repo| async move {
                            let elected = crate::storage::jobs::try_elect(
                                repo.db(),
                                &owner,
                                DEFAULT_LEASE_SECS,
                                chrono::Utc::now(),
                            )
                            .await
                            .map_err(|e| anyhow::anyhow!("elect: {e}"))?
                            .value
                            .elected;
                            if !elected {
                                return Ok(());
                            }
                            // Same fence as the SQLite pools' sweep.
                            let _ =
                                crate::storage::refresh::retire_atlas_insight_work(repo.db()).await;
                            let auth = crate::secrets::AuthFile::load().unwrap_or_default();
                            let settings =
                                crate::provider::SettingsFile::load().unwrap_or_default();
                            let (_, errors) = crate::storage::refresh_driver::recover_and_run(
                                &repo,
                                &settings,
                                &flag,
                                || {
                                    crate::storage::refresh_driver::pipeline(
                                        &auth,
                                        &settings,
                                        &flag,
                                        std::path::PathBuf::new(),
                                    )
                                },
                            )
                            .await?;
                            match errors.first() {
                                Some(e) => Err(anyhow::anyhow!(
                                    "insight refresh recovery: {}: {}",
                                    e.resource,
                                    e.message
                                )),
                                None => Ok(()),
                            }
                        }),
                    );
                    std::thread::sleep(Duration::from_secs(2));
                }
            });
        Self { stop }
    }

    /// Dedicated Brain topology scope analysis drainer: executes inferences
    /// and link generation for enqueued scope tasks while elected.
    pub fn spawn_brain_scope_drainer(db_path: std::path::PathBuf, owner: String) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let _ = std::thread::Builder::new()
            .name("argos-brain-scope".into())
            .spawn(move || {
                let Ok(_runtime) = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                else {
                    return;
                };
                while !flag.load(Ordering::Acquire) {
                    let (owner, stop) = (owner.clone(), flag.clone());
                    report_drainer(
                        &db_path,
                        "brain-scope",
                        crate::storage::runtime::blocking(move |repo| async move {
                            if !surreal_elected(&repo, &owner).await? {
                                return Ok(0);
                            }
                            crate::storage::brain_scope::drain(&repo, &owner, &stop).await
                        }),
                    );
                    std::thread::sleep(Duration::from_millis(750));
                }
            });
        Self { stop }
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Whether this process holds the repository's scheduler lease.
#[cfg(feature = "surreal")]
async fn surreal_elected(
    repo: &crate::storage::repo::Repository,
    owner: &str,
) -> anyhow::Result<bool> {
    Ok(
        crate::storage::jobs::try_elect(repo.db(), owner, DEFAULT_LEASE_SECS, chrono::Utc::now())
            .await
            .map_err(|e| anyhow::anyhow!("elect: {e}"))?
            .value
            .elected,
    )
}

impl Drop for WorkerPool {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Map operation name onto attempt policy kind (delegates to [`tasks::operation_kind`]).
pub fn operation_kind(name: &str) -> OperationKind {
    tasks::operation_kind(name)
}

#[cfg(test)]
mod tests {
    #[test]
    fn alternating_drainer_errors_each_surface_once_per_window_and_map_is_bounded() {
        use std::time::Instant;
        let mut seen = Seen::new();
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);
        assert!(super::should_surface(&mut seen, "intel-brief", "A", at(0)));
        assert!(super::should_surface(&mut seen, "intel-brief", "B", at(1)));
        assert!(!super::should_surface(&mut seen, "intel-brief", "A", at(2)));
        assert!(!super::should_surface(&mut seen, "intel-brief", "B", at(3)));
        // Same message from another drainer is its own key.
        assert!(super::should_surface(&mut seen, "brain-scope", "A", at(4)));
        // After the window both repeat once more, and stale keys are evicted.
        let later = super::DRAINER_ERROR_REPEAT.as_secs() + 10;
        assert!(super::should_surface(
            &mut seen,
            "intel-brief",
            "A",
            at(later)
        ));
        assert!(seen.len() <= 2, "old entries evicted: {}", seen.len());
        for n in 0..(super::DRAINER_ERROR_CAP * 2) {
            super::should_surface(&mut seen, "intel-brief", &format!("e{n}"), at(later + 1));
        }
        assert!(seen.len() <= super::DRAINER_ERROR_CAP);
    }

    #[test]
    fn drainer_messages_replace_argos_home_and_home_with_placeholders() {
        let msg = "open /home/x3/.argos/argos.db failed; cache /home/x3/.cache/a; root /";
        // ARGOS_HOME inside HOME: the longer path wins over `~`.
        let out = super::redact_paths(msg, Some("/home/x3/.argos"), &["/home/x3"]);
        assert_eq!(
            out,
            "open $ARGOS_HOME/argos.db failed; cache ~/.cache/a; root /"
        );
        // Trailing slashes and a bare `/` home never rewrite every slash.
        let out = super::redact_paths("a/b", None, &["/", "/home/x3/"]);
        assert_eq!(out, "a/b");
        // Unrelated text and no configured paths are untouched.
        assert_eq!(super::redact_paths("plain", None, &[]), "plain");
    }

    use super::*;

    #[test]
    fn operation_kind_maps_summarization_modes() {
        assert_eq!(
            operation_kind("page_evidence"),
            OperationKind::Summarization
        );
        assert_eq!(operation_kind("synthesis"), OperationKind::OtherLlm);
    }

    static LOADS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    fn counting_secret() -> Option<crate::secrets::ProviderSecret> {
        LOADS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        None
    }
    fn counting_runtime() -> Option<crate::recon::model_exec::RoleRuntime> {
        LOADS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        None
    }
}
