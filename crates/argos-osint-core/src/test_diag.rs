//! Failure diagnostics for flaky scheduler tests: a plain-text dump of the
//! job, task, lease and attempt tables, so a CI failure shows the state that
//! caused it instead of only the assertion that noticed it.

use std::fmt::Write as _;
use std::path::Path;

const TABLES: &[&str] = &[
    "argos_jobs",
    "argos_tasks",
    "argos_attempts",
    "insight_owner_state",
];

/// Dumps every row of the scheduler tables in the database at `path`, plus
/// SQLite's own `now`, through a fresh connection. Never panics: errors are
/// written into the dump.
pub(crate) fn dump_task_state(path: &Path) -> String {
    let mut out = String::from("\n==== scheduler state dump ====\n");
    let conn = match rusqlite::Connection::open(path) {
        Ok(conn) => conn,
        Err(e) => return format!("{out}open failed: {e}\n"),
    };
    let _ = conn.busy_timeout(std::time::Duration::from_secs(5));
    let now: Result<String, _> =
        conn.query_row("SELECT strftime('%Y-%m-%dT%H:%M:%fZ','now')", [], |r| {
            r.get(0)
        });
    let _ = writeln!(out, "sqlite now: {now:?}");
    for table in TABLES {
        let _ = writeln!(out, "-- {table}");
        let mut stmt = match conn.prepare(&format!("SELECT * FROM {table}")) {
            Ok(stmt) => stmt,
            Err(e) => {
                let _ = writeln!(out, "   ({e})");
                continue;
            }
        };
        let names: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
        let rows = stmt.query_map([], |row| {
            Ok((0..names.len())
                .map(|i| {
                    let v = match row.get_ref(i) {
                        Ok(rusqlite::types::ValueRef::Text(t)) => {
                            let t = String::from_utf8_lossy(t);
                            if t.len() > 200 {
                                format!("{}…", &t[..t.floor_char_boundary(200)])
                            } else {
                                t.into_owned()
                            }
                        }
                        Ok(rusqlite::types::ValueRef::Integer(n)) => n.to_string(),
                        Ok(rusqlite::types::ValueRef::Real(f)) => f.to_string(),
                        Ok(rusqlite::types::ValueRef::Null) => "NULL".into(),
                        Ok(rusqlite::types::ValueRef::Blob(b)) => format!("<{} bytes>", b.len()),
                        Err(e) => format!("<{e}>"),
                    };
                    format!("{}={v}", names[i])
                })
                .collect::<Vec<_>>()
                .join(" "))
        });
        match rows {
            Ok(rows) => {
                for row in rows {
                    let _ = writeln!(out, "   {}", row.unwrap_or_else(|e| e.to_string()));
                }
            }
            Err(e) => {
                let _ = writeln!(out, "   ({e})");
            }
        }
    }
    out
}
