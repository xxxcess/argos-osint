//! Schema-37 Brain migration: typed columns on `memory_metadata` and the
//! claim/inference kind migration preserving memory IDs (spec §4).

use anyhow::Result;
use rusqlite::Connection;

/// Typed columns added to `memory_metadata` (additive, idempotent).
const METADATA_COLUMNS: [(&str, &str); 6] = [
    ("payload_json", "payload_json TEXT NOT NULL DEFAULT '{}'"),
    (
        "payload_schema",
        "payload_schema INTEGER NOT NULL DEFAULT 0",
    ),
    ("revision", "revision INTEGER NOT NULL DEFAULT 1"),
    ("lifecycle", "lifecycle TEXT NOT NULL DEFAULT 'active'"),
    ("created_at", "created_at TEXT NOT NULL DEFAULT ''"),
    ("updated_at", "updated_at TEXT NOT NULL DEFAULT ''"),
];

fn has_table(conn: &Connection, table: &str) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name=?1",
        [table],
        |r| r.get(0),
    )?)
}

pub fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(include_str!("schema_brain_graph.sql"))?;
    if !has_table(conn, "memory_metadata")? {
        return Ok(());
    }
    for (column, ddl) in METADATA_COLUMNS {
        let exists: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM pragma_table_info('memory_metadata') WHERE name=?1",
            [column],
            |r| r.get(0),
        )?;
        if !exists {
            conn.execute_batch(&format!("ALTER TABLE memory_metadata ADD COLUMN {ddl}"))?;
        }
    }
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS memory_metadata_kind ON memory_metadata(memory_kind)",
    )?;
    migrate_claim_kinds(conn)
}

/// `atomic_claim` → claim/inference by authoritative receipts, and type
/// untyped insight-backed memories. Manual/summary/profile/brief kinds are
/// left unchanged; memory IDs are never rewritten.
pub fn migrate_claim_kinds(conn: &Connection) -> Result<()> {
    let derivations = has_table(conn, "insight_derivations")?;
    let claims = has_table(conn, "insight_claims")?;
    // Inference evidence: derivation kind first, classification receipt second.
    let mut inference_sources = Vec::new();
    if derivations {
        inference_sources.push(
            "SELECT memory_id FROM insight_derivations WHERE insight_kind='inference' AND memory_id IS NOT NULL",
        );
    }
    if claims {
        inference_sources.push(
            "SELECT memory_id FROM insight_claims WHERE lower(trim(classification))='inference'",
        );
    }
    let at = chrono::Utc::now().to_rfc3339();
    if !inference_sources.is_empty() {
        let union = inference_sources.join(" UNION ");
        conn.execute(
            &format!(
                "UPDATE memory_metadata SET memory_kind='inference', updated_at=?1
                 WHERE memory_kind='atomic_claim' AND memory_id IN ({union})"
            ),
            [&at],
        )?;
    }
    conn.execute(
        "UPDATE memory_metadata SET memory_kind='claim', updated_at=?1 WHERE memory_kind='atomic_claim'",
        [&at],
    )?;
    if claims {
        // Insight-backed memories that predate memory_metadata.
        conn.execute(
            "INSERT OR IGNORE INTO memory_metadata (memory_id, memory_kind, created_at, updated_at)
             SELECT c.memory_id,
                    CASE WHEN lower(trim(c.classification))='inference' THEN 'inference' ELSE 'claim' END,
                    m.created_at, ?1
             FROM insight_claims c JOIN memories m ON m.id=c.memory_id",
            [&at],
        )?;
    }
    if derivations {
        conn.execute(
            "INSERT OR IGNORE INTO memory_metadata (memory_id, memory_kind, created_at, updated_at)
             SELECT d.memory_id, d.insight_kind, m.created_at, ?1
             FROM insight_derivations d JOIN memories m ON m.id=d.memory_id",
            [&at],
        )?;
    }
    if has_table(conn, "memory_brief_memberships")? {
        conn.execute_batch(
            "UPDATE memory_brief_memberships SET member_type='claim' WHERE member_type='atomic_claim'",
        )?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "brain_schema_tests.rs"]
mod tests;
