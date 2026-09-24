//! SQLite case desk. Tool-call inspection is test-only.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result};
use rusqlite::{params, Connection};

use crate::brain::Memory;
use crate::report::ReportMeta;
use crate::session::Case;

static IDS: AtomicU64 = AtomicU64::new(1);

pub fn new_id(prefix: &str) -> String {
    let n = IDS.fetch_add(1, Ordering::Relaxed);
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("{prefix}-{millis:x}-{n}")
}

#[derive(Clone, Debug)]
pub struct ChatLine {
    pub role: String,
    pub body: String,
    pub created_at: String,
}

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path).with_context(|| format!("open {}", path.display()))?;
        let store = Self { conn };
        store.ensure_schema()?;
        Ok(store)
    }

    pub fn memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let store = Self { conn };
        store.ensure_schema()?;
        Ok(store)
    }

    fn ensure_schema(&self) -> Result<()> {
        self.conn.execute_batch(
            r#"
            PRAGMA journal_mode = WAL;
            CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                kind TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                role TEXT NOT NULL,
                body TEXT NOT NULL,
                created_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS memories (
                id TEXT PRIMARY KEY,
                text TEXT NOT NULL,
                created_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS reports (
                id TEXT PRIMARY KEY,
                case_id TEXT,
                title TEXT NOT NULL,
                path TEXT NOT NULL,
                created_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS tool_calls (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                name TEXT NOT NULL,
                status TEXT NOT NULL,
                detail TEXT NOT NULL,
                created_at TEXT NOT NULL
            );
            "#,
        )?;
        Ok(())
    }

    pub fn ensure_session(&self, id: &str, title: &str, kind: &str) -> Result<()> {
        let now = stamp();
        self.conn.execute(
            "INSERT INTO sessions (id, title, kind, updated_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET title = excluded.title, updated_at = excluded.updated_at",
            params![id, title, kind, now],
        )?;
        Ok(())
    }

    pub fn list_cases(&self) -> Result<Vec<Case>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title FROM sessions WHERE kind = 'case' ORDER BY updated_at DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(Case {
                id: row.get(0)?,
                title: row.get(1)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn create_case(&self, title: &str) -> Result<Case> {
        let case = Case {
            id: new_id("case"),
            title: title.trim().to_string(),
        };
        self.ensure_session(&case.id, &case.title, "case")?;
        Ok(case)
    }

    pub fn touch(&self, id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
            params![stamp(), id],
        )?;
        Ok(())
    }

    pub fn append_message(&self, session_id: &str, role: &str, body: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO messages (session_id, role, body, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![session_id, role, body, stamp()],
        )?;
        self.touch(session_id)?;
        Ok(())
    }

    pub fn load_messages(&self, session_id: &str) -> Result<Vec<ChatLine>> {
        let mut stmt = self.conn.prepare(
            "SELECT role, body, created_at FROM messages WHERE session_id = ?1 ORDER BY id ASC",
        )?;
        let rows = stmt.query_map(params![session_id], |row| {
            Ok(ChatLine {
                role: row.get(0)?,
                body: row.get(1)?,
                created_at: row.get(2)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn clear_messages(&self, session_id: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM messages WHERE session_id = ?1",
            params![session_id],
        )?;
        Ok(())
    }

    pub fn list_memories(&self) -> Result<Vec<Memory>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, text, created_at FROM memories ORDER BY created_at DESC")?;
        let rows = stmt.query_map([], |row| {
            Ok(Memory {
                id: row.get(0)?,
                text: row.get(1)?,
                created_at: row.get(2)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn add_memory(&self, text: &str) -> Result<Memory> {
        let memory = Memory {
            id: new_id("mem"),
            text: text.trim().to_string(),
            created_at: stamp(),
        };
        self.conn.execute(
            "INSERT INTO memories (id, text, created_at) VALUES (?1, ?2, ?3)",
            params![memory.id, memory.text, memory.created_at],
        )?;
        Ok(memory)
    }

    pub fn delete_memory(&self, id: &str) -> Result<bool> {
        let n = self
            .conn
            .execute("DELETE FROM memories WHERE id = ?1", params![id])?;
        Ok(n > 0)
    }

    pub fn add_report(&self, report: &ReportMeta) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO reports (id, case_id, title, path, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![report.id, report.case_id, report.title, report.path, report.created_at],
        )?;
        Ok(())
    }

    pub fn list_reports(&self) -> Result<Vec<ReportMeta>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, case_id, title, path, created_at FROM reports ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(ReportMeta {
                id: row.get(0)?,
                case_id: row.get(1)?,
                title: row.get(2)?,
                path: row.get(3)?,
                created_at: row.get(4)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn log_call(&self, session_id: &str, name: &str, status: &str, detail: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO tool_calls (session_id, name, status, detail, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![session_id, name, status, detail, stamp()],
        )?;
        Ok(())
    }

    /// Tool-call rows for tests. Not part of the runtime API.
    #[cfg(test)]
    pub fn call_states(&self) -> Result<Vec<(String, String)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT name, status FROM tool_calls ORDER BY id ASC")?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }
}

fn stamp() -> String {
    chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cases_messages_and_hidden_call_states() {
        let store = Store::memory().unwrap();
        let case = store.create_case("Harbor").unwrap();
        store
            .append_message(&case.id, "user", "look up the port")
            .unwrap();
        store
            .append_message(&case.id, "assistant", "searching")
            .unwrap();
        assert_eq!(store.load_messages(&case.id).unwrap().len(), 2);
        store
            .log_call(&case.id, "web_search", "ok", "3 hits")
            .unwrap();
        assert_eq!(
            store.call_states().unwrap(),
            vec![("web_search".into(), "ok".into())]
        );
        let mem = store.add_memory("My name is Ada").unwrap();
        assert_eq!(store.list_memories().unwrap()[0].id, mem.id);
    }
}
