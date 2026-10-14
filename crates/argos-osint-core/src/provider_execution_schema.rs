//! Migration 41 (spec §9): per-attempt usage/timing counters, policy
//! diagnostics and task epochs. Purely additive and idempotent: every column
//! is checked before it is added, inside `ensure_schema`'s transaction. New
//! counters stay NULL (unknown) for pre-41 rows — never zero.

use anyhow::Result;
use rusqlite::Connection;

const ATTEMPT_COLUMNS: [(&str, &str); 8] = [
    ("reasoning_tokens", "reasoning_tokens INTEGER"),
    ("visible_tokens", "visible_tokens INTEGER"),
    ("reported_cost_usd", "reported_cost_usd REAL"),
    ("response_headers_ms", "response_headers_ms INTEGER"),
    ("first_reasoning_ms", "first_reasoning_ms INTEGER"),
    ("first_output_ms", "first_output_ms INTEGER"),
    ("stream_completion_ms", "stream_completion_ms INTEGER"),
    (
        "policy_diagnostics_json",
        "policy_diagnostics_json TEXT NOT NULL DEFAULT '[]'",
    ),
];

fn has_column(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT COUNT(*) > 0 FROM pragma_table_info(?1) WHERE name=?2",
        [table, column],
        |r| r.get(0),
    )?)
}

fn has_table(conn: &Connection, table: &str) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name=?1",
        [table],
        |r| r.get(0),
    )?)
}

fn add(conn: &Connection, table: &str, column: &str, ddl: &str) -> Result<()> {
    if has_table(conn, table)? && !has_column(conn, table, column)? {
        conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {ddl}"))?;
    }
    Ok(())
}

pub fn migrate(conn: &Connection) -> Result<()> {
    for (column, ddl) in ATTEMPT_COLUMNS {
        add(conn, "recon_model_attempts", column, ddl)?;
    }
    add(
        conn,
        "recon_model_operations",
        "task_epoch",
        "task_epoch INTEGER",
    )?;
    add(
        conn,
        "intel_brief_stage_timings",
        "task_epoch",
        "task_epoch INTEGER",
    )?;
    if has_table(conn, "recon_model_operations")? {
        conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS recon_operations_task_epoch
               ON recon_model_operations(task_id, task_epoch)",
        )?;
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn columns(conn: &Connection, table: &str) -> Vec<String> {
    let mut stmt = conn
        .prepare("SELECT name FROM pragma_table_info(?1)")
        .unwrap();
    stmt.query_map([table], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}
