//! Elected single-owner scheduler for a shared SQLite state root (spec §4).
//!
//! One process holds the `argos_scheduler_lease` row. Others observe only.
//! Worker pools are separated by kind: LLM, network collect, local index.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

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
fn report_drainer<T>(db_path: &std::path::Path, name: &str, result: Result<T>) {
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

/// Ensure the scheduler lease table exists (additive).
pub fn migrate_scheduler(conn: &Connection) -> Result<()> {
    // Lease renewal calls this helper too. Existing pools need a read-only
    // existence check, not a CREATE statement on every polling interval.
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='argos_scheduler_lease')",
        [],
        |row| row.get(0),
    )?;
    if exists {
        return Ok(());
    }
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS argos_scheduler_lease (
            id TEXT PRIMARY KEY,
            owner TEXT NOT NULL,
            lease_until TEXT NOT NULL,
            epoch INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL
        );",
    )?;
    Ok(())
}

/// Try to become (or renew) the elected scheduler owner for this state root.
pub fn try_elect(conn: &Connection, owner: &str, lease_secs: i64) -> Result<bool> {
    migrate_scheduler(conn)?;
    let now = chrono::Utc::now();
    let now_s = now.to_rfc3339();
    let until = (now + chrono::Duration::seconds(lease_secs)).to_rfc3339();
    let existing: Option<(String, String, i64)> = conn
        .query_row(
            "SELECT owner, lease_until, epoch FROM argos_scheduler_lease WHERE id=?1",
            [SCHEDULER_LEASE_KEY],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    match existing {
        None => {
            conn.execute(
                "INSERT INTO argos_scheduler_lease(id,owner,lease_until,epoch,updated_at)
                 VALUES (?1,?2,?3,1,?4)",
                params![SCHEDULER_LEASE_KEY, owner, until, now_s],
            )?;
            Ok(true)
        }
        Some((cur_owner, lease_until, epoch)) => {
            let expired = lease_until.as_str() < now_s.as_str();
            if cur_owner == owner || expired {
                let n = conn.execute(
                    "UPDATE argos_scheduler_lease SET owner=?1, lease_until=?2, epoch=?3, updated_at=?4
                     WHERE id=?5 AND (owner=?1 OR lease_until<?4)",
                    params![
                        owner,
                        until,
                        epoch + if expired && cur_owner != owner { 1 } else { 0 },
                        now_s,
                        SCHEDULER_LEASE_KEY
                    ],
                )?;
                Ok(n > 0)
            } else {
                Ok(false)
            }
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

/// Claim and apply up to `max` leased index tasks. Embedding/Lance work runs
/// outside any SQLite write transaction; acknowledgement is owner/epoch guarded.
pub fn drain_index_once(
    conn: &Connection,
    store: &crate::store::Store,
    owner: &str,
    max: usize,
) -> Result<usize> {
    let now = chrono::Utc::now().to_rfc3339();
    let _ = tasks::adopt_untracked_index_changes(conn, &now);
    if store.vectors_enabled() {
        // Enablement resumes work parked while embeddings were disabled.
        let _ = tasks::resume_blocked(conn, tasks::EMBEDDINGS_DISABLED, &now);
    }
    let mut done = 0usize;
    for _ in 0..max {
        let now = chrono::Utc::now().to_rfc3339();
        let Some((claimed, work)) = tasks::claim_index_work(conn, owner, DEFAULT_LEASE_SECS, &now)?
        else {
            break;
        };
        let mut outcome = store.apply_index_work(&work);
        if let tasks::IndexOutcome::Ready { revision, .. } = &mut outcome {
            if revision.is_empty() {
                *revision = work.revision.clone();
            }
        }
        let finished = chrono::Utc::now().to_rfc3339();
        tasks::finish_index_work(conn, &claimed, &work, &outcome, &finished)?;
        done += 1;
    }
    Ok(done)
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

/// Drain pending summarization flush tasks from the summary pool only.
///
/// When `secret` is `Some`, runs [`crate::summarization::try_live_summary_upgrade`]
/// (2-attempt `complete_summary`). When `None`, keeps deterministic cache and
/// records `cached_deterministic_no_secret` — never invents network success.
/// Errors are recorded as typed failures under the shared retry policy, never
/// as a completed result string. Topology neighborhood tasks need the whole
/// role chain; use [`drain_summary_flush_with_runtime`].
pub fn drain_summary_flush(
    conn: &Connection,
    owner: &str,
    secret: Option<&crate::secrets::ProviderSecret>,
) -> Result<usize> {
    Ok(
        drain_summary_flush_with_runtime(conn, owner, secret, None, None, DEFAULT_LEASE_SECS)?
            .len(),
    )
}

/// Role-aware drain. Topology neighborhood tasks receive the worker's full
/// `RoleRuntime` (every route of the Summarization chain) and are generated,
/// validated, published and completed under the lease fence. Other modes keep
/// the single-secret path. Returns each task's terminal outcome.
pub fn drain_summary_flush_with_runtime(
    conn: &Connection,
    owner: &str,
    secret: Option<&crate::secrets::ProviderSecret>,
    runtime: Option<&crate::recon::model_exec::RoleRuntime>,
    db_path: Option<&std::path::Path>,
    lease_secs: i64,
) -> Result<Vec<FlushOutcome>> {
    drain_summary_flush_lazy(
        conn,
        owner,
        Lazy::given(secret),
        Lazy::given(runtime),
        db_path,
        lease_secs,
    )
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

pub(crate) fn drain_summary_flush_lazy(
    conn: &Connection,
    owner: &str,
    secret: Lazy<crate::secrets::ProviderSecret>,
    runtime: Lazy<crate::recon::model_exec::RoleRuntime>,
    db_path: Option<&std::path::Path>,
    lease_secs: i64,
) -> Result<Vec<FlushOutcome>> {
    let mut outcomes = Vec::new();
    for _ in 0..8 {
        let now = chrono::Utc::now().to_rfc3339();
        let Some(claimed) =
            tasks::claim_next_in(conn, &[tasks::POOL_SUMMARY], owner, lease_secs, &now)?
        else {
            break;
        };
        let outcome = |tag: &str, generated: bool, stale: bool| FlushOutcome {
            task_id: claimed.id.clone(),
            operation: claimed.operation.clone(),
            cache_key: claimed.input_hash.clone(),
            tag: tag.to_string(),
            generated,
            stale,
        };
        if claimed.operation == crate::brain_neighborhood_brief_exec::OPERATION {
            match crate::brain_neighborhood_brief_exec::run_claimed(
                conn,
                &claimed,
                runtime.get(),
                db_path,
                lease_secs,
            ) {
                Ok(o) => outcomes.push(outcome(&o.tag, o.generated, o.stale)),
                Err(error) => {
                    let finished = chrono::Utc::now().to_rfc3339();
                    tasks::fail_claimed(
                        conn,
                        &claimed,
                        OperationKind::Summarization,
                        &error,
                        &finished,
                    )?;
                    outcomes.push(outcome("storage_failed", false, false));
                }
            }
            continue;
        }
        // Typed routing of failures: a missing request is permanent (nothing to
        // upgrade); storage errors are not provider errors and are not retried
        // blindly. No error text is ever stored as a completion result.
        let result = match crate::summarization::load_flush_request(conn, &claimed.input_hash) {
            Ok(Some(_)) => crate::summarization::try_live_summary_upgrade(
                conn,
                secret.get(),
                &claimed.input_hash,
            )
            .map_err(|err| {
                tasks::TaskError::new(tasks::ErrorCategory::Unknown, format!("{err:#}"))
            }),
            Ok(None) => Err(tasks::TaskError::new(
                tasks::ErrorCategory::Unknown,
                format!("flush request missing for {}", claimed.input_hash),
            )),
            Err(err) => Err(tasks::TaskError::new(
                tasks::ErrorCategory::Unknown,
                format!("{err:#}"),
            )),
        };
        let finished = chrono::Utc::now().to_rfc3339();
        match result {
            Ok((summary, tag)) => {
                tasks::complete_claimed(conn, &claimed, tag, &finished)?;
                outcomes.push(outcome(tag, !summary.fallback, false));
            }
            Err(error) => {
                tasks::fail_claimed(
                    conn,
                    &claimed,
                    OperationKind::Summarization,
                    &error,
                    &finished,
                )?;
                outcomes.push(outcome("failed", false, false));
            }
        }
    }
    Ok(outcomes)
}

/// Drain without a provider secret (deterministic cache only).
pub fn drain_summary_flush_cached(conn: &Connection, owner: &str) -> Result<usize> {
    drain_summary_flush(conn, owner, None)
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
        if crate::recon::backend::sqlite_path(&db_path).is_none() {
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
            return Self { stop };
        }
        std::thread::Builder::new()
            .name("argos-index-pool".into())
            .spawn(move || {
                let mut idle_loops = 0u32;
                let mut schema_ready = false;
                while !flag.load(Ordering::Relaxed) {
                    let Ok(conn) = Connection::open(&db_path) else {
                        std::thread::sleep(Duration::from_secs(2));
                        continue;
                    };
                    let _ = conn.busy_timeout(Duration::from_secs(5));
                    if !schema_ready {
                        schema_ready = migrate_scheduler(&conn)
                            .and_then(|_| tasks::migrate_tables(&conn))
                            .is_ok();
                    }
                    if !try_elect(&conn, &owner, DEFAULT_LEASE_SECS).unwrap_or(false) {
                        std::thread::sleep(Duration::from_secs(2));
                        continue;
                    }
                    let now = chrono::Utc::now().to_rfc3339();
                    if idle_loops.is_multiple_of(60) {
                        if let Ok(service) = tasks::ensure_service_job(
                            &conn,
                            tasks::INDEX_SERVICE_JOB,
                            "brain",
                            "Local index worker",
                            &now,
                        ) {
                            let _ = tasks::refresh_job_state(&conn, &service, &now);
                        }
                    }
                    let _ = tasks::retire_atlas_insight_work(&conn, &now);
                    let _ = tasks::interrupt_expired_leases(&conn, &now);
                    if let Ok(store) = crate::store::Store::open_index_worker(&db_path) {
                        let drained = drain_index_once(&conn, &store, &owner, 16).unwrap_or(0);
                        // Catch up generation rebuilds even without an explicit change row.
                        let _ = store.process_pending_vector_rebuild(2);
                        // Atlas runs left partial/blocked by indexing reach their
                        // verified status once this pool finishes their work.
                        idle_loops += 1;
                        if drained > 0 || idle_loops >= 60 {
                            idle_loops = 0;
                            let _ = crate::atlas_memory::refresh_incomplete_runs(&store, 8);
                        }
                    }
                    std::thread::sleep(Duration::from_millis(500));
                }
            })
            .ok();
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
                let mut loops = 0u32;
                let mut schema_ready = false;
                while !flag.load(Ordering::Relaxed) {
                    {
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
                        continue;
                    }
                    let Ok(conn) = Connection::open(&db_path) else {
                        std::thread::sleep(Duration::from_secs(2));
                        continue;
                    };
                    let _ = conn.busy_timeout(Duration::from_secs(5));
                    if !schema_ready {
                        schema_ready = tasks::migrate_tables(&conn).is_ok();
                    }
                    if !try_elect(&conn, &owner, DEFAULT_LEASE_SECS).unwrap_or(false) {
                        std::thread::sleep(Duration::from_secs(2));
                        continue;
                    }
                    let _ =
                        tasks::retire_atlas_insight_work(&conn, &chrono::Utc::now().to_rfc3339());
                    // Best-effort: load summarization role secret when configured.
                    // Never invents success when auth/settings/secret are missing.
                    // Loaded lazily, only once a summary task is claimed.
                    // Service-health row; batches/tasks supply the job history.
                    if loops.is_multiple_of(40) {
                        let now = chrono::Utc::now().to_rfc3339();
                        let service = tasks::ensure_service_job(
                            &conn,
                            SUMMARY_SERVICE_JOB,
                            "brain",
                            "Summary flush worker",
                            &now,
                        );
                        if let Ok(service) = service {
                            let _ = tasks::refresh_job_state(&conn, &service, &now);
                        }
                    }
                    loops = loops.wrapping_add(1);
                    let _ = drain_summary_flush_lazy(
                        &conn,
                        &owner,
                        Lazy::loaded_on_demand(load_summarization_secret),
                        Lazy::loaded_on_demand(load_role_runtime),
                        Some(&db_path),
                        DEFAULT_LEASE_SECS,
                    );
                    std::thread::sleep(Duration::from_millis(750));
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
        if crate::recon::backend::surreal_selected() {
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
            return vec![Self::spawn_insight_refresh_drainer(db_path, owner)];
        }
        vec![
            Self::spawn_index_drainer(db_path.clone(), owner.clone()),
            Self::spawn_summary_flush_drainer(db_path.clone(), owner.clone()),
            Self::spawn_insight_refresh_drainer(db_path.clone(), owner.clone()),
            Self::spawn_brain_backfill_drainer(db_path.clone(), owner.clone()),
            Self::spawn_actor_dedup_drainer(db_path.clone(), owner.clone()),
            Self::spawn_intel_brief_drainer(db_path.clone(), owner.clone()),
            Self::spawn_brain_scope_drainer(db_path, owner),
        ]
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
                let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                else {
                    return;
                };
                while !flag.load(Ordering::Acquire) {
                    {
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
                        continue;
                    }
                    let elected = Connection::open(&db_path).ok().is_some_and(|conn| {
                        try_elect(&conn, &owner, DEFAULT_LEASE_SECS).unwrap_or(false)
                    });
                    if elected {
                        report_drainer(
                            &db_path,
                            "actor-dedup",
                            runtime
                                .block_on(crate::brain_actor_dedup::drain(&db_path, &owner, &flag)),
                        );
                    }
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
                    {
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
                        continue;
                    }
                    let elected = Connection::open(&db_path).ok().is_some_and(|conn| {
                        try_elect(&conn, &owner, DEFAULT_LEASE_SECS).unwrap_or(false)
                    });
                    let complete = elected
                        && crate::store::Store::open_light(&db_path)
                            .and_then(|s| s.brain_backfill(crate::brain_backfill::BATCH))
                            .map(|r| r.complete)
                            .unwrap_or(false);
                    let pause = if !elected || complete { 30 } else { 1 };
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
        make_backend: IntelBackendFactory,
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let _ = std::thread::Builder::new()
            .name("argos-intel-brief".into())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_multi_thread()
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
                                        {
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
                        continue;
                    }
                    // Two stage slots: the independent v2 roots really run
                    // together, each with its own backend and connections.
                    let factory = make_backend.clone();
                    report_drainer(
                        &db_path,
                        "intel-brief",
                        runtime.block_on(crate::intel_recon::brief_stages::drain_parallel(
                            &db_path,
                            &format!("{owner}:intel-brief"),
                            move || factory(),
                            &flag,
                            &config,
                            crate::intel_recon::brief_stages::STAGE_SLOTS,
                        )),
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
                let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                else {
                    return;
                };
                while !flag.load(Ordering::Acquire) {
                    // SurrealDB: elect on the repository's scheduler lease and
                    // drive the shared refresh tasks there (startup recovery
                    // included); never the SQLite file.
                    {
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
                                    crate::storage::refresh::retire_atlas_insight_work(repo.db())
                                        .await;
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
                        continue;
                    }
                    let elected = Connection::open(&db_path).ok().is_some_and(|conn| {
                        try_elect(&conn, &owner, DEFAULT_LEASE_SECS).unwrap_or(false)
                    });
                    if elected {
                        let auth = crate::secrets::AuthFile::load().unwrap_or_default();
                        let settings = crate::provider::SettingsFile::load().unwrap_or_default();
                        report_drainer(
                            &db_path,
                            "insight-refresh",
                            runtime.block_on(
                                crate::investigation::insight_persist::recover_refreshes(
                                    &db_path, &auth, &settings, &flag,
                                ),
                            ),
                        );
                    }
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
                let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                else {
                    return;
                };
                while !flag.load(Ordering::Acquire) {
                    {
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
                        continue;
                    }
                    let elected = Connection::open(&db_path).ok().is_some_and(|conn| {
                        try_elect(&conn, &owner, DEFAULT_LEASE_SECS).unwrap_or(false)
                    });
                    if elected {
                        report_drainer(
                            &db_path,
                            "brain-scope",
                            runtime.block_on(crate::brain_scope::drain_scope_tasks(
                                &db_path, &owner, &flag,
                            )),
                        );
                    }
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
    use tempfile::tempdir;

    #[test]
    fn election_is_exclusive_until_expiry() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("t.db");
        let conn = Connection::open(&path).unwrap();
        assert!(try_elect(&conn, "a", 60).unwrap());
        assert!(!try_elect(&conn, "b", 60).unwrap());
        assert!(try_elect(&conn, "a", 60).unwrap(), "owner can renew");
    }

    #[test]
    fn operation_kind_maps_summarization_modes() {
        assert_eq!(
            operation_kind("page_evidence"),
            OperationKind::Summarization
        );
        assert_eq!(operation_kind("synthesis"), OperationKind::OtherLlm);
    }

    #[test]
    fn apply_index_change_rebuild_reports_outcome() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("argos.db");
        let store = crate::store::Store::open(&path).unwrap();
        drop(store);
        let outcome = apply_index_change(&path, "memory_index", "generation", "rebuild");
        // ARGOS_EMBED=0 in unit tests: a typed Disabled outcome, never "upserted".
        if !crate::embed::enabled() {
            assert!(
                matches!(outcome, tasks::IndexOutcome::Disabled { .. }),
                "{outcome:?}"
            );
        }
    }

    fn summary_task(conn: &Connection, input_hash: &str) {
        let now = chrono::Utc::now().to_rfc3339();
        tasks::enqueue_job(
            conn,
            &tasks::NewJob {
                id: "job-s".into(),
                kind: "summarization_flush".into(),
                owner_scope: "atlas_brief".into(),
                input_revision: "1".into(),
                deadline_at: String::new(),
            },
            &now,
        )
        .unwrap();
        assert!(tasks::enqueue_task(
            conn,
            &tasks::NewTask {
                id: "task-s".into(),
                job_id: "job-s".into(),
                operation: "atlas_brief".into(),
                dedupe_key: "flush:test".into(),
                priority: 50,
                input_ref: "atlas".into(),
                input_hash: input_hash.into(),
                source_revision: "1".into(),
                role_snapshot: "summarization".into(),
                max_attempts: 2,
            },
            &now,
        )
        .unwrap());
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

    #[test]
    fn an_idle_summary_tick_loads_no_secret_or_role_runtime() {
        let conn = Connection::open_in_memory().unwrap();
        tasks::migrate_tables(&conn).unwrap();
        LOADS.store(0, std::sync::atomic::Ordering::SeqCst);
        let out = drain_summary_flush_lazy(
            &conn,
            "worker",
            Lazy::loaded_on_demand(counting_secret),
            Lazy::loaded_on_demand(counting_runtime),
            None,
            DEFAULT_LEASE_SECS,
        )
        .unwrap();
        assert!(out.is_empty());
        assert_eq!(
            LOADS.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "idle ticks must not read auth/settings"
        );
    }

    #[test]
    fn drain_summary_flush_completes_cached_tasks() {
        let conn = Connection::open_in_memory().unwrap();
        tasks::migrate_tables(&conn).unwrap();
        let req = crate::summarization::flush_request(
            crate::summarization::SummarizationMode::AtlasBrief,
            "atlas-brief-r1",
            "r1",
            "Brief text about the cycle.",
            "atlas",
            800,
        );
        let det = crate::summarization::SummaryResult {
            content: "Brief text about the cycle.".into(),
            source_refs: vec!["atlas-brief-r1".into()],
            source_hash: String::new(),
            model: "deterministic".into(),
            prompt_version: req.prompt_version.clone(),
            coverage: Default::default(),
            source_hashes: Vec::new(),
            provider: "local".into(),
            preprocessing: serde_json::Value::Null,
            fallback: true,
        };
        crate::summarization::cache_put(&conn, &req, &det).unwrap();
        crate::summarization::persist_flush_request(&conn, &req).unwrap();
        summary_task(&conn, &req.cache_key());
        let n = drain_summary_flush_cached(&conn, "worker").unwrap();
        assert_eq!(n, 1);
        let (state, result_ref): (String, String) = conn
            .query_row(
                "SELECT state, result_ref FROM argos_tasks WHERE id='task-s'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(state, "completed");
        assert_eq!(result_ref, "cached_deterministic_no_secret");
    }

    #[test]
    fn a_leftover_graph_explanation_task_fails_cleanly_and_the_fresh_task_still_runs() {
        let conn = Connection::open_in_memory().unwrap();
        tasks::migrate_tables(&conn).unwrap();
        let now = chrono::Utc::now().to_rfc3339();
        // An upgraded database: the removed graph-explanation operation was queued
        // before the view was removed.
        tasks::enqueue_job(
            &conn,
            &tasks::NewJob {
                id: "job-old".into(),
                kind: "graph_summary".into(),
                owner_scope: String::new(),
                input_revision: "1".into(),
                deadline_at: String::new(),
            },
            &now,
        )
        .unwrap();
        assert!(tasks::enqueue_task(
            &conn,
            &tasks::NewTask {
                id: "task-old".into(),
                job_id: "job-old".into(),
                operation: "graph_explanation".into(),
                dedupe_key: "legacy-graph".into(),
                priority: 99,
                input_ref: "memory-1".into(),
                input_hash: "legacy-graph-key".into(),
                source_revision: "1".into(),
                role_snapshot: "summarization".into(),
                max_attempts: 2,
            },
            &now,
        )
        .unwrap());
        let req = crate::summarization::flush_request(
            crate::summarization::SummarizationMode::AtlasBrief,
            "atlas-brief-r1",
            "r1",
            "Brief text about the cycle.",
            "atlas",
            800,
        );
        let det = crate::summarization::SummaryResult {
            content: "Brief text about the cycle.".into(),
            source_refs: vec!["atlas-brief-r1".into()],
            source_hash: String::new(),
            model: "deterministic".into(),
            prompt_version: req.prompt_version.clone(),
            coverage: Default::default(),
            source_hashes: Vec::new(),
            provider: "local".into(),
            preprocessing: serde_json::Value::Null,
            fallback: true,
        };
        crate::summarization::cache_put(&conn, &req, &det).unwrap();
        crate::summarization::persist_flush_request(&conn, &req).unwrap();
        summary_task(&conn, &req.cache_key());
        let n = drain_summary_flush_cached(&conn, "worker").unwrap();
        assert_eq!(n, 2, "the drainer handles both tasks without stalling");
        let state = |id: &str| -> (String, String) {
            conn.query_row(
                "SELECT state, error_message FROM argos_tasks WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap()
        };
        assert_eq!(state("task-s").0, "completed");
        let (old_state, message) = state("task-old");
        assert_eq!(old_state, "failed", "{message}");
        assert!(message.contains("flush request missing"), "{message}");
        let open: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM argos_tasks WHERE state IN ('queued','running','retry_scheduled')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(open, 0);
    }

    #[test]
    fn summary_worker_errors_are_typed_failures_not_completions() {
        let conn = Connection::open_in_memory().unwrap();
        tasks::migrate_tables(&conn).unwrap();
        summary_task(&conn, "missing-request");
        drain_summary_flush_cached(&conn, "worker").unwrap();
        let (state, result_ref, message): (String, String, String) = conn
            .query_row(
                "SELECT state, result_ref, error_message FROM argos_tasks WHERE id='task-s'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(state, "failed");
        assert!(result_ref.is_empty(), "{result_ref}");
        assert!(message.contains("flush request missing"), "{message}");
    }

    #[test]
    fn summary_worker_leaves_index_and_atlas_tasks_alone() {
        let conn = Connection::open_in_memory().unwrap();
        tasks::migrate_tables(&conn).unwrap();
        let now = chrono::Utc::now().to_rfc3339();
        tasks::enqueue_index_change(&conn, "memory", "m1", "r1", "upsert", &now).unwrap();
        tasks::enqueue_job(
            &conn,
            &tasks::NewJob {
                id: "job-a".into(),
                kind: "atlas_cycle".into(),
                owner_scope: String::new(),
                input_revision: String::new(),
                deadline_at: String::new(),
            },
            &now,
        )
        .unwrap();
        tasks::enqueue_task(
            &conn,
            &tasks::NewTask {
                id: "atlas-pub".into(),
                job_id: "job-a".into(),
                operation: "atlas_publish".into(),
                dedupe_key: String::new(),
                priority: 1,
                input_ref: String::new(),
                input_hash: String::new(),
                source_revision: String::new(),
                role_snapshot: String::new(),
                max_attempts: 3,
            },
            &now,
        )
        .unwrap();
        assert_eq!(drain_summary_flush_cached(&conn, "worker").unwrap(), 0);
        let states: Vec<(String, String)> = conn
            .prepare("SELECT state, result_ref FROM argos_tasks")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert!(
            states.iter().all(|(s, r)| s == "queued" && r.is_empty()),
            "{states:?}"
        );
    }

    #[test]
    fn index_drain_on_store_without_vectors_blocks_honestly() {
        let store = crate::store::Store::memory().unwrap();
        let now = chrono::Utc::now().to_rfc3339();
        tasks::enqueue_index_change(store.conn_for_tests(), "memory", "m1", "r1", "upsert", &now)
            .unwrap();
        let n = drain_index_once(store.conn_for_tests(), &store, "w", 4).unwrap();
        assert_eq!(n, 1);
        let (state, outcome): (String, String) = store
            .conn_for_tests()
            .query_row("SELECT state, outcome FROM argos_index_changes", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(state, "blocked");
        assert!(outcome.contains("disabled"), "{outcome}");
    }
}
