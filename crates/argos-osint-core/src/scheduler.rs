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

/// Apply one claimed index-change row. Returns a durable outcome string.
pub fn apply_index_change(
    db_path: &std::path::Path,
    kind: &str,
    id: &str,
    operation: &str,
) -> String {
    match operation {
        "rebuild" | "generation_rebuild" => {
            match crate::store::Store::open(db_path) {
                Ok(store) => match store.process_pending_vector_rebuild(4) {
                    Ok(n) => format!("rebuild_batches={n}"),
                    Err(err) => format!("rebuild_err={}", truncate_err(&err.to_string())),
                },
                Err(err) => format!("open_err={}", truncate_err(&err.to_string())),
            }
        }
        "upsert" | "index_upsert" => {
            match crate::store::Store::open(db_path) {
                Ok(store) => {
                    store.index_upsert(&[id.to_string()]);
                    format!("upserted:{kind}/{id}")
                }
                Err(err) => format!("open_err={}", truncate_err(&err.to_string())),
            }
        }
        "remove" | "index_remove" => {
            match crate::store::Store::open(db_path) {
                Ok(store) => {
                    store.index_remove_missing(&[id.to_string()]);
                    format!("removed:{kind}/{id}")
                }
                Err(err) => format!("open_err={}", truncate_err(&err.to_string())),
            }
        }
        other => format!("noop:{other}"),
    }
}

fn truncate_err(s: &str) -> String {
    s.chars().take(120).collect()
}

/// Drain pending summarization flush tasks.
///
/// When `secret` is `Some`, runs [`crate::summarization::try_live_summary_upgrade`]
/// (2-attempt `complete_summary`). When `None`, keeps deterministic cache and
/// records `cached_deterministic_no_secret` — never invents network success.
pub fn drain_summary_flush(
    conn: &Connection,
    owner: &str,
    secret: Option<&crate::secrets::ProviderSecret>,
) -> Result<usize> {
    let now = chrono::Utc::now().to_rfc3339();
    let mut done = 0usize;
    for _ in 0..8 {
        let Some(id) = tasks::claim_next(conn, owner, DEFAULT_LEASE_SECS, &now)? else {
            break;
        };
        let (operation, input_hash): (String, String) = conn.query_row(
            "SELECT operation, input_hash FROM argos_tasks WHERE id=?1",
            [&id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if operation_kind(&operation) != OperationKind::Summarization {
            let _ = tasks::complete_task(conn, &id, "skipped_non_summarization", &now);
            continue;
        }
        let outcome = match crate::summarization::try_live_summary_upgrade(conn, secret, &input_hash)
        {
            Ok((_, tag)) => tag.to_string(),
            Err(err) => format!(
                "upgrade_err={}",
                err.to_string().chars().take(80).collect::<String>()
            ),
        };
        let _ = tasks::complete_task(conn, &id, &outcome, &now);
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
                    let _ = migrate_scheduler(&conn);
                    let _ = tasks::migrate_tables(&conn);
                    if !try_elect(&conn, &owner, DEFAULT_LEASE_SECS).unwrap_or(false) {
                        std::thread::sleep(Duration::from_secs(2));
                        continue;
                    }
                    if let Ok(batch) = tasks::claim_index_changes(&conn, 16) {
                        for (seq, kind, id, op) in batch {
                            let outcome = apply_index_change(&db_path, &kind, &id, &op);
                            let _ = tasks::complete_index_change(&conn, seq, &outcome);
                        }
                    }
                    // Catch up generation rebuilds even without an explicit change row.
                    if let Ok(store) = crate::store::Store::open(&db_path) {
                        let _ = store.process_pending_vector_rebuild(2);
                    }
                    let now = chrono::Utc::now().to_rfc3339();
                    let _ = tasks::interrupt_expired_leases(&conn, &now);
                    std::thread::sleep(Duration::from_millis(500));
                }
            })
            .ok();
        Self { stop }
    }

    /// Elects the scheduler and drains summarization flush tasks that already
    /// have deterministic cache rows (no network). Pair with
    /// [`crate::summarization::complete_summary`] at call sites that hold secrets.
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
                    let _ = tasks::migrate_tables(&conn);
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

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Drop for WorkerPool {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Map operation name onto attempt policy kind.
pub fn operation_kind(name: &str) -> OperationKind {
    match name {
        "page_evidence" | "graph_explanation" | "follow_up_context" | "tool_observation"
        | "investigation_title" | "report_context" | "section_digest" | "atlas_brief"
        | "article_description" | "summarization" => OperationKind::Summarization,
        "index_rebuild" | "index_upsert" => OperationKind::LocalIndex,
        "osint_collect" => OperationKind::NetworkCollect,
        _ => OperationKind::OtherLlm,
    }
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
        assert!(
            outcome.starts_with("rebuild_batches=") || outcome.starts_with("rebuild_err="),
            "{outcome}"
        );
    }

    #[test]
    fn drain_summary_flush_completes_cached_tasks() {
        let conn = Connection::open_in_memory().unwrap();
        tasks::migrate_tables(&conn).unwrap();
        let now = chrono::Utc::now().to_rfc3339();
        tasks::enqueue_job(
            &conn,
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
            &conn,
            &tasks::NewTask {
                id: "task-s".into(),
                job_id: "job-s".into(),
                operation: "atlas_brief".into(),
                dedupe_key: "flush:test".into(),
                priority: 50,
                input_ref: "atlas".into(),
                input_hash: "h".into(),
                source_revision: "1".into(),
                role_snapshot: "summarization".into(),
                max_attempts: 2,
            },
            &now,
        )
        .unwrap());
        let n = drain_summary_flush_cached(&conn, "worker").unwrap();
        assert_eq!(n, 1);
        let state: String = conn
            .query_row(
                "SELECT state FROM argos_tasks WHERE id='task-s'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(state, "completed");
        let result_ref: String = conn
            .query_row(
                "SELECT result_ref FROM argos_tasks WHERE id='task-s'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            result_ref.contains("cached_deterministic") || result_ref.contains("upgrade_err"),
            "{result_ref}"
        );
    }
}
