//! Brain index stand-in for builds compiled without LanceDB.
//!
//! SQLite stays the source of truth. Vector search is unavailable, and recall
//! uses Jaccard. [`block_on`] stays so embedding downloads and summary upgrades
//! still have a runtime bridge.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::{anyhow, Result};
use rusqlite::{params, Connection, OptionalExtension};

use crate::embed;

/// Lance table name kept so callers can record the same label.
pub const TABLE: &str = "brain_memories";
/// Cosine similarity at or above which two memories count as the same statement.
pub const DUPLICATE_THRESHOLD: f32 = 0.92;
/// Key of the fingerprint row in `memory_embed_meta`.
pub const META_KEY: &str = "brain_vectors";
const LAYOUT: &str = "lance:brain_memories:v1";

fn unavailable() -> anyhow::Error {
    anyhow!("LanceDB support was not compiled into this build")
}

fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("argos-lance")
            .enable_all()
            .build()
            .expect("start the Brain index runtime")
    })
}

/// Runs `future` to completion on a process-local runtime.
pub fn block_on<F>(future: F) -> F::Output
where
    F: Future + Send,
    F::Output: Send,
{
    use tokio::runtime::{Handle, RuntimeFlavor};
    match Handle::try_current() {
        Err(_) => runtime().block_on(future),
        Ok(handle) if handle.runtime_flavor() == RuntimeFlavor::MultiThread => {
            tokio::task::block_in_place(|| runtime().block_on(future))
        }
        Ok(_) => {
            std::thread::scope(
                |scope| match scope.spawn(|| runtime().block_on(future)).join() {
                    Ok(output) => output,
                    Err(panic) => std::panic::resume_unwind(panic),
                },
            )
        }
    }
}

static LAST_ERROR: Mutex<Option<String>> = Mutex::new(None);

pub(crate) fn note_error(err: &anyhow::Error) {
    if let Ok(mut slot) = LAST_ERROR.lock() {
        *slot = Some(format!("{err:#}"));
    }
}

/// The most recent vector index or embedding failure in this process, if any.
pub fn last_error() -> Option<String> {
    LAST_ERROR.lock().ok().and_then(|slot| slot.clone())
}

/// Model + table layout identity stored in `memory_embed_meta`.
pub fn current_fingerprint() -> String {
    format!("{};{LAYOUT}", embed::fingerprint())
}

pub fn fingerprint_matches(conn: &Connection) -> Result<bool> {
    let stored: Option<String> = conn
        .query_row(
            "SELECT value FROM memory_embed_meta WHERE key=?1",
            [META_KEY],
            |row| row.get(0),
        )
        .optional()?;
    Ok(stored.as_deref() == Some(current_fingerprint().as_str()))
}

pub fn write_fingerprint(conn: &Connection) -> Result<()> {
    conn.execute(
        "INSERT INTO memory_embed_meta(key,value,updated_at) VALUES (?1,?2,?3)
         ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_at=excluded.updated_at",
        params![
            META_KEY,
            current_fingerprint(),
            chrono::Utc::now().to_rfc3339()
        ],
    )?;
    Ok(())
}

pub fn clear_fingerprint(conn: &Connection) -> Result<()> {
    conn.execute("DELETE FROM memory_embed_meta WHERE key=?1", [META_KEY])?;
    Ok(())
}

/// Vector index handle. This build has no LanceDB client, so every read and
/// write of vectors fails and callers fall back to Jaccard.
pub struct BrainIndex {
    uri: PathBuf,
    serving_name: Mutex<String>,
}

impl BrainIndex {
    pub fn shared(uri: &Path) -> Arc<BrainIndex> {
        Arc::new(Self::new(uri))
    }

    pub fn new(uri: &Path) -> Self {
        Self {
            uri: uri.to_path_buf(),
            serving_name: Mutex::new(TABLE.to_string()),
        }
    }

    pub fn uri(&self) -> &Path {
        &self.uri
    }

    pub fn is_ready(&self) -> bool {
        false
    }

    pub fn mark_ready(&self) {}

    pub fn mark_stale(&self) {}

    pub fn exists(&self) -> bool {
        false
    }

    pub fn serving_table_name(&self) -> String {
        self.serving_name
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
    }

    pub fn set_serving_table(&self, name: &str) {
        *self
            .serving_name
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = name.to_string();
    }

    pub fn upsert(&self, _id: &str, _text: &str) -> Result<()> {
        Err(unavailable())
    }

    pub fn upsert_texts(&self, _rows: &[(String, String)]) -> Result<()> {
        Err(unavailable())
    }

    pub fn upsert_vectors(&self, _rows: &[(String, Vec<f32>)]) -> Result<()> {
        Err(unavailable())
    }

    pub fn remove(&self, _id: &str) -> Result<()> {
        Err(unavailable())
    }

    pub fn remove_many(&self, _ids: &[String]) -> Result<()> {
        Err(unavailable())
    }

    pub fn search(&self, _query: &str, _k: usize) -> Result<Vec<(String, f32)>> {
        Err(unavailable())
    }

    pub fn search_vector(&self, _vector: &[f32], _k: usize) -> Result<Vec<(String, f32)>> {
        Err(unavailable())
    }

    pub fn find_similar(&self, _text: &str, _threshold: f32) -> Result<Option<(String, f32)>> {
        Err(unavailable())
    }

    pub fn ids(&self) -> Result<Vec<String>> {
        Err(unavailable())
    }

    pub fn present_ids(&self, _ids: &[String]) -> Result<std::collections::HashSet<String>> {
        Err(unavailable())
    }

    pub fn count(&self) -> Result<usize> {
        Err(unavailable())
    }

    pub fn rebuild(&self, _memories: &[(String, String)]) -> Result<usize> {
        Err(unavailable())
    }

    pub fn rebuild_vectors(&self, _rows: &[(String, Vec<f32>)]) -> Result<usize> {
        Err(unavailable())
    }
}

/// Durable generation metadata. The table is created so later opens stay valid;
/// filling it requires LanceDB.
pub fn migrate_generations(conn: &rusqlite::Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS argos_index_generations (
            id TEXT PRIMARY KEY,
            fingerprint TEXT NOT NULL,
            state TEXT NOT NULL,
            serving INTEGER NOT NULL DEFAULT 0,
            source_count INTEGER NOT NULL DEFAULT 0,
            batch_cursor INTEGER NOT NULL DEFAULT 0,
            table_name TEXT NOT NULL DEFAULT '',
            error TEXT NOT NULL DEFAULT '',
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );",
    )?;
    let cols: Vec<String> = {
        let mut stmt = conn.prepare("PRAGMA table_info(argos_index_generations)")?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    if !cols.iter().any(|c| c == "table_name") {
        conn.execute(
            "ALTER TABLE argos_index_generations ADD COLUMN table_name TEXT NOT NULL DEFAULT ''",
            [],
        )?;
    }
    Ok(())
}

pub fn begin_generation(
    conn: &rusqlite::Connection,
    source_count: usize,
) -> anyhow::Result<String> {
    migrate_generations(conn)?;
    let id = format!("gen-{}", chrono::Utc::now().timestamp_millis());
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO argos_index_generations(id,fingerprint,state,serving,source_count,batch_cursor,table_name,created_at,updated_at)
         VALUES (?1,?2,'building',0,?3,0,'',?4,?4)",
        rusqlite::params![id, current_fingerprint(), source_count as i64, now],
    )?;
    Ok(id)
}

pub fn serving_table_name(conn: &rusqlite::Connection) -> anyhow::Result<Option<String>> {
    migrate_generations(conn)?;
    let row: Option<String> = conn
        .query_row(
            "SELECT table_name FROM argos_index_generations WHERE serving=1 LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    Ok(row.filter(|name| !name.is_empty()))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GenerationProgress {
    pub generation_id: String,
    pub cursor: usize,
    pub total: usize,
    pub activated: bool,
}

pub fn rebuild_generation_batched(
    _conn: &rusqlite::Connection,
    _index: &BrainIndex,
    _generation_id: &str,
    _memories: &[(String, String)],
) -> anyhow::Result<GenerationProgress> {
    Err(unavailable())
}
