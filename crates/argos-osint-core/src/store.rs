//! SQLite case desk. Tool-call inspection is test-only.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result};
use rusqlite::{params, Connection};

use crate::brain::Memory;
use crate::report::ReportMeta;
use crate::session::Case;
use crate::tna::TnaSnapshot;

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
            CREATE TABLE IF NOT EXISTS tna_graphs (
                key TEXT PRIMARY KEY,
                snapshot_json TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            "#,
        )?;
        self.ensure_memory_columns()?;
        Ok(())
    }

    fn ensure_memory_columns(&self) -> Result<()> {
        let mut stmt = self.conn.prepare("PRAGMA table_info(memories)")?;
        let cols = stmt.query_map([], |row| row.get::<_, String>(1))?;
        let cols: Vec<String> = cols.collect::<Result<Vec<_>, _>>()?;
        if !cols.iter().any(|col| col == "category") {
            self.conn.execute(
                "ALTER TABLE memories ADD COLUMN category TEXT NOT NULL DEFAULT 'fact'",
                [],
            )?;
        }
        if !cols.iter().any(|col| col == "pinned") {
            self.conn.execute(
                "ALTER TABLE memories ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
        }
        if !cols.iter().any(|col| col == "report_id") {
            self.conn
                .execute("ALTER TABLE memories ADD COLUMN report_id TEXT", [])?;
        }
        self.conn
            .execute("UPDATE memories SET category = 'fact'", [])?;
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

    pub fn delete_case(&self, id: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM messages WHERE session_id = ?1", params![id])?;
        self.conn.execute(
            "DELETE FROM sessions WHERE id = ?1 AND kind = 'case'",
            params![id],
        )?;
        Ok(())
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

    /// Replace the newest message of `role` in the session. Inserts one when none exists.
    pub fn update_last_message(&self, session_id: &str, role: &str, body: &str) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE messages SET body = ?1 WHERE id = (
                SELECT id FROM messages WHERE session_id = ?2 AND role = ?3 ORDER BY id DESC LIMIT 1
            )",
            params![body, session_id, role],
        )?;
        if changed == 0 {
            self.append_message(session_id, role, body)?;
        }
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
        let mut stmt = self.conn.prepare(
            "SELECT id, text, created_at, category, pinned, report_id FROM memories ORDER BY pinned DESC, created_at DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            let pinned: i64 = row.get(4)?;
            Ok(Memory {
                id: row.get(0)?,
                text: row.get(1)?,
                created_at: row.get(2)?,
                category: "fact".into(),
                pinned: pinned != 0,
                report_id: row.get(5)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn add_memory(&self, text: &str) -> Result<Memory> {
        self.add_memory_typed(text, "fact", false)
    }

    pub fn add_memory_typed(&self, text: &str, category: &str, pinned: bool) -> Result<Memory> {
        let memory = Memory {
            id: new_id("mem"),
            text: text.trim().to_string(),
            category: "fact".into(),
            pinned,
            created_at: stamp(),
            report_id: None,
        };
        let _ = category;
        if memory.text.is_empty() {
            anyhow::bail!("memory text is empty");
        }
        self.conn.execute(
            "INSERT INTO memories (id, text, created_at, category, pinned, report_id) VALUES (?1, ?2, ?3, 'fact', ?4, ?5)",
            params![
                memory.id,
                memory.text,
                memory.created_at,
                i64::from(memory.pinned),
                memory.report_id
            ],
        )?;
        Ok(memory)
    }

    pub fn add_report_fact(&self, text: &str, report_id: &str) -> Result<Memory> {
        let memory = Memory {
            id: new_id("mem"),
            text: text.trim().to_string(),
            category: "fact".into(),
            pinned: false,
            created_at: stamp(),
            report_id: Some(report_id.to_string()),
        };
        if memory.text.is_empty() {
            anyhow::bail!("memory text is empty");
        }
        self.conn.execute(
            "INSERT INTO memories (id, text, created_at, category, pinned, report_id) VALUES (?1, ?2, ?3, 'fact', 0, ?4)",
            params![memory.id, memory.text, memory.created_at, report_id],
        )?;
        Ok(memory)
    }

    pub fn update_memory(
        &self,
        id: &str,
        text: &str,
        category: &str,
        pinned: bool,
    ) -> Result<bool> {
        let text = text.trim();
        if text.is_empty() {
            anyhow::bail!("memory text is empty");
        }
        let _ = category;
        let n = self.conn.execute(
            "UPDATE memories SET text = ?1, category = 'fact', pinned = ?2 WHERE id = ?3",
            params![text, i64::from(pinned), id],
        )?;
        Ok(n > 0)
    }

    pub fn delete_report_bundle(&self, id: &str) -> Result<()> {
        let session = format!("report:{id}");
        self.conn
            .execute("DELETE FROM memories WHERE report_id = ?1", params![id])?;
        self.conn.execute(
            "DELETE FROM messages WHERE session_id = ?1",
            params![session],
        )?;
        self.conn
            .execute("DELETE FROM sessions WHERE id = ?1", params![session])?;
        self.conn
            .execute("DELETE FROM reports WHERE id = ?1", params![id])?;
        Ok(())
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

    pub fn get_tna_graph(&self, key: &str) -> Result<Option<TnaSnapshot>> {
        let mut stmt = self
            .conn
            .prepare("SELECT snapshot_json FROM tna_graphs WHERE key = ?1")?;
        let mut rows = stmt.query(params![key])?;
        if let Some(row) = rows.next()? {
            let json: String = row.get(0)?;
            let snap: TnaSnapshot = serde_json::from_str(&json)
                .with_context(|| format!("decode tna graph {key}"))?;
            Ok(Some(snap))
        } else {
            Ok(None)
        }
    }

    pub fn upsert_tna_graph(&self, key: &str, snapshot: &TnaSnapshot) -> Result<()> {
        let json = serde_json::to_string(snapshot).context("encode tna graph")?;
        self.conn.execute(
            "INSERT INTO tna_graphs (key, snapshot_json, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET snapshot_json = excluded.snapshot_json, updated_at = excluded.updated_at",
            params![key, json, stamp()],
        )?;
        Ok(())
    }

    pub fn delete_tna_graph(&self, key: &str) -> Result<bool> {
        let n = self
            .conn
            .execute("DELETE FROM tna_graphs WHERE key = ?1", params![key])?;
        Ok(n > 0)
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
            .update_last_message(&case.id, "assistant", "the tide turned")
            .unwrap();
        let saved = store.load_messages(&case.id).unwrap();
        assert_eq!(saved.len(), 2);
        assert_eq!(saved[1].body, "the tide turned");
        store
            .log_call(&case.id, "web_search", "ok", "3 hits")
            .unwrap();
        assert_eq!(
            store.call_states().unwrap(),
            vec![("web_search".into(), "ok".into())]
        );
        let mem = store
            .add_memory_typed("Night desk case", "project", true)
            .unwrap();
        let listed = store.list_memories().unwrap();
        assert_eq!(listed[0].id, mem.id);
        assert_eq!(listed[0].category, "fact");
        assert!(listed[0].pinned);
        assert!(store
            .update_memory(&mem.id, "Night desk, renamed", "goal", false)
            .unwrap());
        let updated = store.list_memories().unwrap();
        assert_eq!(updated[0].text, "Night desk, renamed");
        assert_eq!(updated[0].category, "fact");
        assert!(!updated[0].pinned);
        assert!(store.delete_memory(&mem.id).unwrap());
        assert!(store.list_memories().unwrap().is_empty());
    }
    #[test]
    fn tna_graph_persist_roundtrip() {
        use crate::tna::{TnaScope, TnaSnapshot};
        let store = Store::memory().unwrap();
        let snap = TnaSnapshot::empty(TnaScope::Collection);
        store.upsert_tna_graph("desk", &snap).unwrap();
        let loaded = store.get_tna_graph("desk").unwrap().unwrap();
        assert_eq!(loaded.title, "TNA · collection");
        assert!(store.delete_tna_graph("desk").unwrap());
        assert!(store.get_tna_graph("desk").unwrap().is_none());
    }

}