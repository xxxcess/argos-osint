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
        let Some((claimed, work)) = tasks::claim_index_work(conn, owner, DEFAULT_LEASE_SECS, &now)? else {
            break;
        };
        let mut outcome = apply_index_with(store, &work.record_kind, &work.record_id, &work.operation);
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

/// Drain pending summarization flush tasks from the summary pool only.
///
/// When `secret` is `Some`, runs [`crate::summarization::try_live_summary_upgrade`]
/// (2-attempt `complete_summary`). When `None`, keeps deterministic cache and
/// records `cached_deterministic_no_secret` — never invents network success.
/// Errors are recorded as typed failures under the shared retry policy, never
/// as a completed result string.
pub fn drain_summary_flush(
    conn: &Connection,
    owner: &str,
    secret: Option<&crate::secrets::ProviderSecret>,
) -> Result<usize> {
    let mut done = 0usize;
    for _ in 0..8 {
        let now = chrono::Utc::now().to_rfc3339();
        let Some(claimed) =
            tasks::claim_next_in(conn, &[tasks::POOL_SUMMARY], owner, DEFAULT_LEASE_SECS, &now)?
        else {
            break;
        };
        // Typed routing of failures: a missing request is permanent (nothing to
        // upgrade); storage errors are not provider errors and are not retried
        // blindly. No error text is ever stored as a completion result.
        let result = match crate::summarization::load_flush_request(conn, &claimed.input_hash) {
            Ok(Some(_)) => crate::summarization::try_live_summary_upgrade(conn, secret, &claimed.input_hash)
                .map_err(|err| tasks::TaskError::new(tasks::ErrorCategory::Unknown, format!("{err:#}"))),
            Ok(None) => Err(tasks::TaskError::new(
                tasks::ErrorCategory::Unknown,
                format!("flush request missing for {}", claimed.input_hash),
            )),
            Err(err) => Err(tasks::TaskError::new(tasks::ErrorCategory::Unknown, format!("{err:#}"))),
        };
        let finished = chrono::Utc::now().to_rfc3339();
        match result {
            Ok((_, tag)) => {
                tasks::complete_claimed(conn, &claimed, tag, &finished)?;
            }
            Err(error) => {
                tasks::fail_claimed(conn, &claimed, OperationKind::Summarization, &error, &finished)?;
            }
        }
        done += 1;
    }
    Ok(done)
}

/// Drain without a provider secret (deterministic cache only).
pub fn drain_summary_flush_cached(conn: &Connection, owner: &str) -> Result<usize> {
    drain_summary_flush(conn, owner, None)
}


fn load_summarization_secret() -> Option<crate::secrets::ProviderSecret> {
    let auth = crate::secrets::AuthFile::load().ok()?;
    let settings = crate::provider::SettingsFile::load().ok()?;
    crate::provider::role_secret(&auth, &settings, "summarization").ok()
}

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
        std::thread::Builder::new()
            .name("argos-index-pool".into())
            .spawn(move || {
                while !flag.load(Ordering::Relaxed) {
                    let Ok(conn) = Connection::open(&db_path) else {
                        std::thread::sleep(Duration::from_secs(2));
                        continue;
                    };
                    let _ = conn.busy_timeout(Duration::from_secs(5));
                    let _ = migrate_scheduler(&conn);
                    let _ = tasks::migrate_tables(&conn);
                    if !try_elect(&conn, &owner, DEFAULT_LEASE_SECS).unwrap_or(false) {
                        std::thread::sleep(Duration::from_secs(2));
                        continue;
                    }
                    let now = chrono::Utc::now().to_rfc3339();
                    let _ = tasks::interrupt_expired_leases(&conn, &now);
                    if let Ok(store) = crate::store::Store::open(&db_path) {
                        let _ = drain_index_once(&conn, &store, &owner, 16);
                        // Catch up generation rebuilds even without an explicit change row.
                        let _ = store.process_pending_vector_rebuild(2);
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
                while !flag.load(Ordering::Relaxed) {
                    let Ok(conn) = Connection::open(&db_path) else {
                        std::thread::sleep(Duration::from_secs(2));
                        continue;
                    };
                    let _ = conn.busy_timeout(Duration::from_secs(5));
                    let _ = tasks::migrate_tables(&conn);
                    if !try_elect(&conn, &owner, DEFAULT_LEASE_SECS).unwrap_or(false) {
                        std::thread::sleep(Duration::from_secs(2));
                        continue;
                    }
                    // Best-effort: load summarization role secret when configured.
                    // Never invents success when auth/settings/secret are missing.
                    let secret = load_summarization_secret();
                    let _ = drain_summary_flush(&conn, &owner, secret.as_ref());
                    std::thread::sleep(Duration::from_millis(750));
                }
            })
            .ok();
        Self { stop }
    }

    /// Spawn the default local pools (index + summary) for one state root.
    pub fn spawn_default(db_path: std::path::PathBuf) -> Vec<WorkerPool> {
        let owner = process_owner();
        vec![
            Self::spawn_index_drainer(db_path.clone(), owner.clone()),
            Self::spawn_summary_flush_drainer(db_path, owner),
        ]
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
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
        assert_eq!(operation_kind("page_evidence"), OperationKind::Summarization);
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
        tasks::enqueue_index_change(store.conn_for_tests(), "memory", "m1", "r1", "upsert", &now).unwrap();
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
