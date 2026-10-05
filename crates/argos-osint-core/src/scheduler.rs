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
                    params![owner, until, epoch + if expired && cur_owner != owner { 1 } else { 0 }, now_s, SCHEDULER_LEASE_KEY],
                )?;
                Ok(n > 0)
            } else {
                Ok(false)
            }
        }
    }
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
                            let outcome = format!("{kind}/{id}/{op}");
                            let _ = tasks::complete_index_change(&conn, seq, &outcome);
                        }
                    }
                    // Also recover expired task leases while we own the scheduler.
                    let now = chrono::Utc::now().to_rfc3339();
                    let _ = tasks::interrupt_expired_leases(&conn, &now);
                    std::thread::sleep(Duration::from_millis(500));
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
}
