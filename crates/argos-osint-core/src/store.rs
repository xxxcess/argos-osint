//! Persistent Brain memories and their conversation provenance.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

use crate::brain::{normalize_category, recall, Memory, MemorySource, ScoredMemory};

static IDS: AtomicU64 = AtomicU64::new(1);

fn new_id() -> String {
    let n = IDS.fetch_add(1, Ordering::Relaxed);
    format!("mem-{}-{n}", chrono::Utc::now().timestamp_millis())
}

/// A saved explanation of one memory, keyed by the directive it was written from.
pub struct GraphSummary {
    pub summary: String,
    /// Directive id. Empty for a summary written before the path was narrowed.
    pub focus: String,
}

pub struct Store {
    pub(crate) conn: Connection,
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
        let store = Self {
            conn: Connection::open_in_memory()?,
        };
        store.ensure_schema()?;
        Ok(store)
    }

    fn ensure_schema(&self) -> Result<()> {
        self.conn
            .execute_batch("PRAGMA foreign_keys=OFF; BEGIN IMMEDIATE")?;
        let result = (|| -> Result<()> {
            let version: i64 = self
                .conn
                .pragma_query_value(None, "user_version", |row| row.get(0))?;
            anyhow::ensure!(
                version <= 10,
                "database schema version {version} is newer than this Argos build"
            );
            let tables: Vec<String> = {
                let mut stmt = self.conn.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'")?;
                let rows = stmt
                    .query_map([], |row| row.get(0))?
                    .collect::<rusqlite::Result<_>>()?;
                rows
            };
            let old_memories = tables.iter().any(|name| name == "memories");
            let cols: Vec<String> = if old_memories {
                let mut stmt = self.conn.prepare("PRAGMA table_info(memories)")?;
                let rows = stmt
                    .query_map([], |row| row.get(1))?
                    .collect::<rusqlite::Result<_>>()?;
                rows
            } else {
                Vec::new()
            };
            if !old_memories {
                self.conn.execute_batch("CREATE TABLE memories (id TEXT PRIMARY KEY, text TEXT NOT NULL, category TEXT NOT NULL, pinned INTEGER NOT NULL, created_at TEXT NOT NULL, source_json TEXT NOT NULL)")?;
            } else if !cols.iter().any(|v| v == "source_json") {
                self.conn.execute_batch("CREATE TABLE memories_new (id TEXT PRIMARY KEY, text TEXT NOT NULL, category TEXT NOT NULL, pinned INTEGER NOT NULL, created_at TEXT NOT NULL, source_json TEXT NOT NULL)")?;
                let category = if cols.iter().any(|v| v == "category") {
                    "category"
                } else {
                    "'fact'"
                };
                let pinned = if cols.iter().any(|v| v == "pinned") {
                    "pinned"
                } else {
                    "0"
                };
                let source = if cols.iter().any(|v| v == "source_json") {
                    "source_json"
                } else {
                    "'{\"app\":\"argos-legacy\",\"conversation_id\":\"unknown\",\"message_id\":null,\"reference\":null}'"
                };
                let filter = if cols.iter().any(|v| v == "report_id") {
                    "WHERE report_id IS NULL"
                } else {
                    ""
                };
                self.conn.execute_batch(&format!("INSERT INTO memories_new SELECT id,text,{category},{pinned},created_at,{source} FROM memories {filter}"))?;
                self.conn.execute_batch(
                    "DROP TABLE memories; ALTER TABLE memories_new RENAME TO memories",
                )?;
            }
            self.conn.execute_batch(
                "CREATE INDEX IF NOT EXISTS memories_created ON memories(created_at DESC)",
            )?;
            if version < 4 {
                self.conn.execute_batch(include_str!("schema_recon.sql"))?;
                self.conn.pragma_update(None, "user_version", 4)?;
            }
            if version < 5 {
                let recon_schema_present: bool = self.conn.query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='recon_runs')",
                    [],
                    |row| row.get(0),
                )?;
                if !recon_schema_present {
                    self.conn.execute_batch(include_str!("schema_recon.sql"))?;
                }
                let run_columns: Vec<String> = {
                    let mut stmt = self.conn.prepare("PRAGMA table_info(recon_runs)")?;
                    let columns = stmt
                        .query_map([], |row| row.get(1))?
                        .collect::<rusqlite::Result<_>>()?;
                    columns
                };
                if !run_columns.iter().any(|name| name == "max_rounds") {
                    self.conn.execute_batch(
                        "ALTER TABLE recon_runs ADD COLUMN max_rounds INTEGER NOT NULL DEFAULT 6;
                         ALTER TABLE recon_runs ADD COLUMN max_calls INTEGER NOT NULL DEFAULT 12;
                         ALTER TABLE recon_runs ADD COLUMN turn_seconds INTEGER NOT NULL DEFAULT 300;",
                    )?;
                }
                self.conn.pragma_update(None, "user_version", 5)?;
            }
            if version < 6 {
                self.conn.execute_batch(
                    "CREATE TABLE IF NOT EXISTS recon_message_memories (
                       message_id TEXT NOT NULL REFERENCES recon_messages(id) ON DELETE CASCADE,
                       memory_id TEXT NOT NULL,
                       ordinal INTEGER NOT NULL,
                       PRIMARY KEY(message_id, memory_id)
                     );",
                )?;
                self.conn.pragma_update(None, "user_version", 6)?;
            }
            if version < 7 {
                self.conn
                    .execute_batch(include_str!("schema_investigation.sql"))?;
                self.conn.pragma_update(None, "user_version", 7)?;
            }
            if version < 8 {
                let run_columns: Vec<String> = {
                    let mut stmt = self.conn.prepare("PRAGMA table_info(recon_runs)")?;
                    let columns = stmt
                        .query_map([], |row| row.get(1))?
                        .collect::<rusqlite::Result<_>>()?;
                    columns
                };
                if !run_columns.iter().any(|name| name == "tool_picker_model") {
                    self.conn.execute_batch(
                        "ALTER TABLE recon_runs ADD COLUMN tool_picker_model TEXT NOT NULL DEFAULT ''",
                    )?;
                }
                self.conn.pragma_update(None, "user_version", 8)?;
            }
            if version < 9 {
                self.conn.execute_batch(
                    "CREATE TABLE IF NOT EXISTS memory_graph_summaries (
                       memory_id TEXT PRIMARY KEY,
                       summary TEXT NOT NULL,
                       created_at TEXT NOT NULL
                     );",
                )?;
                self.conn.pragma_update(None, "user_version", 9)?;
            }
            if version < 10 {
                let summary_columns: Vec<String> = {
                    let mut stmt = self
                        .conn
                        .prepare("PRAGMA table_info(memory_graph_summaries)")?;
                    let columns = stmt
                        .query_map([], |row| row.get(1))?
                        .collect::<rusqlite::Result<_>>()?;
                    columns
                };
                if !summary_columns.iter().any(|name| name == "focus") {
                    self.conn.execute_batch(
                        "ALTER TABLE memory_graph_summaries ADD COLUMN focus TEXT NOT NULL DEFAULT ''",
                    )?;
                }
                self.conn.pragma_update(None, "user_version", 10)?;
            }
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.conn.execute_batch("COMMIT; PRAGMA foreign_keys=ON")?;
                Ok(())
            }
            Err(err) => {
                let _ = self.conn.execute_batch("ROLLBACK; PRAGMA foreign_keys=ON");
                Err(err)
            }
        }
    }

    pub fn list_memories(&self) -> Result<Vec<Memory>> {
        let mut stmt = self.conn.prepare("SELECT id,text,category,pinned,created_at,source_json FROM memories ORDER BY pinned DESC,created_at DESC")?;
        let rows = stmt.query_map([], |row| {
            let source: String = row.get(5)?;
            let source = serde_json::from_str(&source).map_err(|err| {
                rusqlite::Error::FromSqlConversionFailure(
                    5,
                    rusqlite::types::Type::Text,
                    Box::new(err),
                )
            })?;
            Ok(Memory {
                id: row.get(0)?,
                text: row.get(1)?,
                category: row.get(2)?,
                pinned: row.get::<_, i64>(3)? != 0,
                created_at: row.get(4)?,
                source,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn add_memory(
        &self,
        text: &str,
        category: &str,
        pinned: bool,
        source: MemorySource,
    ) -> Result<Memory> {
        source.validate()?;
        let text = text.trim();
        anyhow::ensure!(!text.is_empty(), "memory text is empty");
        let memory = Memory {
            id: new_id(),
            text: text.into(),
            category: normalize_category(category).into(),
            pinned,
            created_at: chrono::Utc::now().to_rfc3339(),
            source,
        };
        self.conn.execute(
            "INSERT INTO memories(id,text,category,pinned,created_at,source_json) VALUES (?1,?2,?3,?4,?5,?6)",
            params![
                memory.id,
                memory.text,
                memory.category,
                i64::from(memory.pinned),
                memory.created_at,
                serde_json::to_string(&memory.source)?
            ],
        )?;
        Ok(memory)
    }

    pub fn recall(&self, query: &str, top_k: usize) -> Result<Vec<ScoredMemory>> {
        Ok(recall(&self.list_memories()?, query, top_k))
    }

    pub fn update_memory(
        &self,
        id: &str,
        text: &str,
        category: &str,
        pinned: bool,
    ) -> Result<bool> {
        let text = text.trim();
        anyhow::ensure!(!text.is_empty(), "memory text is empty");
        let changed = self.conn.execute(
            "UPDATE memories SET text=?1,category=?2,pinned=?3 WHERE id=?4",
            params![text, normalize_category(category), i64::from(pinned), id],
        )? > 0;
        if changed {
            self.conn.execute("INSERT OR IGNORE INTO insight_user_edits(memory_id) SELECT memory_id FROM insight_claims WHERE memory_id=?1",[id])?;
        }
        Ok(changed)
    }

    pub fn graph_summary(&self, memory_id: &str) -> Result<Option<GraphSummary>> {
        Ok(self
            .conn
            .query_row(
                "SELECT summary, focus FROM memory_graph_summaries WHERE memory_id=?1",
                [memory_id],
                |row| {
                    Ok(GraphSummary {
                        summary: row.get(0)?,
                        focus: row.get(1)?,
                    })
                },
            )
            .optional()?)
    }

    /// Stores the synthesis explanation for one memory's recon path.
    /// `focus` is the directive id the summary was written from.
    /// Returns false when that memory is already gone.
    pub fn save_graph_summary(&self, memory_id: &str, summary: &str, focus: &str) -> Result<bool> {
        let summary = summary.trim();
        anyhow::ensure!(!summary.is_empty(), "graph summary is empty");
        let exists: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM memories WHERE id=?1",
            [memory_id],
            |row| row.get(0),
        )?;
        if exists == 0 {
            return Ok(false);
        }
        self.conn.execute(
            "INSERT INTO memory_graph_summaries(memory_id,summary,created_at,focus) VALUES (?1,?2,?3,?4)
             ON CONFLICT(memory_id) DO UPDATE SET summary=excluded.summary, created_at=excluded.created_at, focus=excluded.focus",
            params![memory_id, summary, chrono::Utc::now().to_rfc3339(), focus],
        )?;
        Ok(true)
    }

    pub fn delete_memory(&self, id: &str) -> Result<bool> {
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| -> Result<bool> {
            self.conn.execute("DELETE FROM insight_sources WHERE fingerprint IN (SELECT fingerprint FROM insight_claims WHERE memory_id=?1)",[id])?;
            self.conn.execute("DELETE FROM insight_relations WHERE left_fingerprint IN (SELECT fingerprint FROM insight_claims WHERE memory_id=?1) OR right_fingerprint IN (SELECT fingerprint FROM insight_claims WHERE memory_id=?1)",[id])?;
            self.conn
                .execute("DELETE FROM insight_claims WHERE memory_id=?1", [id])?;
            self.conn.execute(
                "DELETE FROM memory_graph_summaries WHERE memory_id=?1",
                [id],
            )?;
            Ok(self
                .conn
                .execute("DELETE FROM memories WHERE id=?1", [id])?
                > 0)
        })();
        match result {
            Ok(deleted) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(deleted)
            }
            Err(err) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(err)
            }
        }
    }

    pub fn get_memory(&self, id: &str) -> Result<Option<Memory>> {
        Ok(self.list_memories()?.into_iter().find(|m| m.id == id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_memory_has_provenance_and_category() {
        let store = Store::memory().unwrap();
        let source = MemorySource {
            app: "chat".into(),
            conversation_id: "thread-1".into(),
            message_id: Some("msg-2".into()),
            reference: None,
        };
        assert!(store
            .add_memory(
                "test",
                "fact",
                false,
                MemorySource {
                    app: "".into(),
                    ..source.clone()
                }
            )
            .is_err());
        let memory = store
            .add_memory("My name is Ada", "identity", true, source.clone())
            .unwrap();
        assert_eq!(
            store.get_memory(&memory.id).unwrap().unwrap().source,
            source
        );
        assert_eq!(
            store.recall("who am I", 3).unwrap()[0].memory.category,
            "identity"
        );
    }

    #[test]
    fn reopening_keeps_memory_source() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let source = MemorySource {
            app: "chat".into(),
            conversation_id: "thread-9".into(),
            message_id: Some("message-3".into()),
            reference: Some("https://example.com/thread-9".into()),
        };
        Store::open(file.path())
            .unwrap()
            .add_memory("Atlas launch", "project", false, source.clone())
            .unwrap();
        let reopened = Store::open(file.path()).unwrap();
        assert_eq!(reopened.list_memories().unwrap()[0].source, source);
    }

    #[test]
    fn deleting_a_memory_removes_its_graph_summary() {
        let store = Store::memory().unwrap();
        let memory = store
            .add_memory(
                "Atlas launch",
                "project",
                false,
                MemorySource {
                    app: "chat".into(),
                    conversation_id: "thread-9".into(),
                    message_id: None,
                    reference: None,
                },
            )
            .unwrap();
        assert!(store
            .save_graph_summary(&memory.id, "The recon path ends at the launch window.", "d1")
            .unwrap());
        assert!(store
            .graph_summary(&memory.id)
            .unwrap()
            .unwrap()
            .summary
            .contains("launch"));
        assert!(store.delete_memory(&memory.id).unwrap());
        assert!(store.graph_summary(&memory.id).unwrap().is_none());
    }

    #[test]
    fn legacy_brain_is_preserved_without_dropping_unrelated_tables() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let conn = Connection::open(file.path()).unwrap();
        conn.execute_batch("CREATE TABLE memories(id TEXT PRIMARY KEY,text TEXT,created_at TEXT,category TEXT,pinned INTEGER,report_id TEXT); INSERT INTO memories VALUES ('a','keep','today','identity',1,NULL),('b','report','today','fact',0,'r1'); CREATE TABLE reports(id TEXT); CREATE TABLE sessions(id TEXT); CREATE VIRTUAL TABLE passage_fts USING fts5(body);").unwrap();
        drop(conn);
        let store = Store::open(file.path()).unwrap();
        assert_eq!(store.list_memories().unwrap().len(), 1);
        assert_eq!(store.list_memories().unwrap()[0].source.app, "argos-legacy");
        let count: i64 = store.conn.query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name NOT IN ('memories') AND name NOT LIKE 'sqlite_%'", [], |r| r.get(0)).unwrap();
        assert!(count > 0);
        let reports: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name='reports'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(reports, 1);
    }

    #[test]
    fn version_eight_adds_the_tool_picker_snapshot_column() {
        let columns = |store: &Store| -> Vec<String> {
            let mut stmt = store.conn.prepare("PRAGMA table_info(recon_runs)").unwrap();
            stmt.query_map([], |row| row.get(1))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap()
        };
        let fresh = Store::memory().unwrap();
        assert!(columns(&fresh)
            .iter()
            .any(|name| name == "tool_picker_model"));
        let version: i64 = fresh
            .conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, 10);
        // A version-7 database without the column gains it, keeping existing runs.
        let file = tempfile::NamedTempFile::new().unwrap();
        let store = Store::open(file.path()).unwrap();
        store.conn.execute_batch("INSERT INTO recon_threads(id,title,created_at,updated_at) VALUES ('t','T','now','now'); INSERT INTO recon_runs(id,thread_id,turn_id,state,stage,recon_model,synthesis_model,created_at,updated_at) VALUES ('r','t','m','completed','complete','grok / a','grok / b','now','now'); ALTER TABLE recon_runs DROP COLUMN tool_picker_model; PRAGMA user_version=7;").unwrap();
        assert!(!columns(&store)
            .iter()
            .any(|name| name == "tool_picker_model"));
        drop(store);
        let reopened = Store::open(file.path()).unwrap();
        assert!(columns(&reopened)
            .iter()
            .any(|name| name == "tool_picker_model"));
        let snapshot: String = reopened
            .conn
            .query_row(
                "SELECT tool_picker_model FROM recon_runs WHERE id='r'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(snapshot, "");
    }
}
