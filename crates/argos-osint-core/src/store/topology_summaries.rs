//! Saved topology scope summaries (schema 43).
//!
//! One row per exact digest (scope + graph version + mode + model + prompt), so
//! recentering away and back reuses the saved text. The older
//! `memory_graph_summaries` holds a single row per memory and cannot do this.
//! Rows are pruned per seed and deleted with their seed memory.

use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

use super::Store;

/// Rows kept per seed (least recently used pruned first).
pub const MAX_PER_SEED: usize = 50;

/// A saved scope summary.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TopologySummaryRow {
    pub digest: String,
    pub scope_key: String,
    pub seed_key: String,
    /// Memory id when the seed is a stored memory, else empty.
    pub seed_ref: String,
    pub graph_version: String,
    pub mode: String,
    pub summary: String,
    pub provider: String,
    pub model: String,
    pub prompt_version: String,
    pub node_count: i64,
    pub edge_count: i64,
    /// `memory_changed_seq` the summary was written at.
    pub changed_seq: i64,
    pub created_at: String,
    pub last_used_at: String,
}

pub(super) fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS topology_scope_summaries (
           digest TEXT PRIMARY KEY,
           scope_key TEXT NOT NULL,
           seed_key TEXT NOT NULL,
           seed_ref TEXT NOT NULL DEFAULT '',
           graph_version TEXT NOT NULL,
           mode TEXT NOT NULL,
           summary TEXT NOT NULL,
           provider TEXT NOT NULL DEFAULT '',
           model TEXT NOT NULL DEFAULT '',
           prompt_version TEXT NOT NULL DEFAULT '',
           node_count INTEGER NOT NULL DEFAULT 0,
           edge_count INTEGER NOT NULL DEFAULT 0,
           changed_seq INTEGER NOT NULL DEFAULT 0,
           created_at TEXT NOT NULL,
           last_used_at TEXT NOT NULL
         );
         CREATE INDEX IF NOT EXISTS topology_scope_summaries_scope
           ON topology_scope_summaries(scope_key, mode, created_at);
         CREATE INDEX IF NOT EXISTS topology_scope_summaries_seed
           ON topology_scope_summaries(seed_key, last_used_at);
         CREATE INDEX IF NOT EXISTS topology_scope_summaries_ref
           ON topology_scope_summaries(seed_ref);",
    )?;
    Ok(())
}

const COLUMNS: &str = "digest, scope_key, seed_key, seed_ref, graph_version, mode, summary,
    provider, model, prompt_version, node_count, edge_count, changed_seq, created_at, last_used_at";

fn row_of(r: &rusqlite::Row<'_>) -> rusqlite::Result<TopologySummaryRow> {
    Ok(TopologySummaryRow {
        digest: r.get(0)?,
        scope_key: r.get(1)?,
        seed_key: r.get(2)?,
        seed_ref: r.get(3)?,
        graph_version: r.get(4)?,
        mode: r.get(5)?,
        summary: r.get(6)?,
        provider: r.get(7)?,
        model: r.get(8)?,
        prompt_version: r.get(9)?,
        node_count: r.get(10)?,
        edge_count: r.get(11)?,
        changed_seq: r.get(12)?,
        created_at: r.get(13)?,
        last_used_at: r.get(14)?,
    })
}

impl Store {
    /// Exact digest hit. Marks the row used.
    pub fn topology_summary(&self, digest: &str) -> Result<Option<TopologySummaryRow>> {
        let row = self
            .conn
            .query_row(
                &format!("SELECT {COLUMNS} FROM topology_scope_summaries WHERE digest=?1"),
                [digest],
                row_of,
            )
            .optional()?;
        if row.is_some() {
            self.conn.execute(
                "UPDATE topology_scope_summaries SET last_used_at=?2 WHERE digest=?1",
                params![digest, chrono::Utc::now().to_rfc3339()],
            )?;
        }
        Ok(row)
    }

    /// Newest saved summary for this scope in `mode`, whatever its graph version
    /// (shown as an earlier result and used by the freshness rule).
    pub fn topology_summary_for_scope(
        &self,
        scope_key: &str,
        mode: &str,
    ) -> Result<Option<TopologySummaryRow>> {
        Ok(self
            .conn
            .query_row(
                &format!(
                    "SELECT {COLUMNS} FROM topology_scope_summaries
                     WHERE scope_key=?1 AND mode=?2 ORDER BY created_at DESC, rowid DESC LIMIT 1"
                ),
                params![scope_key, mode],
                row_of,
            )
            .optional()?)
    }

    /// Saves a summary under its digest and prunes the seed to [`MAX_PER_SEED`].
    pub fn save_topology_summary(&self, row: &TopologySummaryRow) -> Result<()> {
        let summary = row.summary.trim();
        anyhow::ensure!(!summary.is_empty(), "topology summary is empty");
        let now = chrono::Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO topology_scope_summaries
               (digest, scope_key, seed_key, seed_ref, graph_version, mode, summary, provider,
                model, prompt_version, node_count, edge_count, changed_seq, created_at, last_used_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?14)
             ON CONFLICT(digest) DO UPDATE SET summary=excluded.summary,
               node_count=excluded.node_count, edge_count=excluded.edge_count,
               changed_seq=excluded.changed_seq, created_at=excluded.created_at,
               last_used_at=excluded.last_used_at",
            params![
                row.digest,
                row.scope_key,
                row.seed_key,
                row.seed_ref,
                row.graph_version,
                row.mode,
                summary,
                row.provider,
                row.model,
                row.prompt_version,
                row.node_count,
                row.edge_count,
                row.changed_seq,
                now
            ],
        )?;
        self.conn.execute(
            "DELETE FROM topology_scope_summaries WHERE seed_key=?1 AND digest NOT IN (
               SELECT digest FROM topology_scope_summaries WHERE seed_key=?1
               ORDER BY last_used_at DESC, rowid DESC LIMIT ?2)",
            params![row.seed_key, MAX_PER_SEED as i64],
        )?;
        Ok(())
    }

    /// Deletes every saved summary whose seed is this memory.
    pub fn delete_topology_summaries_for_memory(&self, memory_id: &str) -> Result<usize> {
        Ok(self.conn.execute(
            "DELETE FROM topology_scope_summaries WHERE seed_ref=?1",
            [memory_id],
        )?)
    }

    #[cfg(test)]
    pub(crate) fn topology_summary_count(&self, seed_key: &str) -> usize {
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM topology_scope_summaries WHERE seed_key=?1",
                [seed_key],
                |r| r.get::<_, i64>(0),
            )
            .unwrap_or(0) as usize
    }
}
