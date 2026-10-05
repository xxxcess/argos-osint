//! Brain vector index in LanceDB, beside `argos.db`.
//!
//! SQLite stays the source of truth for memories, claims, and provenance. This module
//! keeps one Lance table, `brain_memories` (`memory_id` Utf8 as the logical key,
//! `vector` FixedSizeList<Float32, 384>), under `memory_lancedb/` next to the
//! database (`ARGOS_HOME/memory_lancedb` for the default DB, see
//! [`crate::paths::lancedb_dir`]). The index can always be thrown away and rebuilt
//! from SQLite: `argos memories reindex` does that, and so does a fingerprint
//! mismatch in the SQLite `memory_embed_meta` table (model or layout change).
//!
//! LanceDB is async. Everything here is sync and goes through [`block_on`], which is
//! safe to call from plain threads and from inside a Tokio runtime (Recon turns call
//! `Store::recall` from async code).

use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::{anyhow, Result};
use futures_util::TryStreamExt;
use lancedb::arrow::arrow::array::{
    Array, FixedSizeListArray, Float32Array, RecordBatch, StringArray,
};
use lancedb::arrow::arrow::datatypes::{DataType, Field, Float32Type, Schema, SchemaRef};
use lancedb::query::{ExecutableQuery, QueryBase, Select};
use lancedb::DistanceType;
use rusqlite::{params, Connection, OptionalExtension};

use crate::embed;

/// Lance table holding one vector per Brain memory.
pub const TABLE: &str = "brain_memories";
/// Cosine similarity at or above which two memories count as the same statement.
pub const DUPLICATE_THRESHOLD: f32 = 0.92;
/// Key of the fingerprint row in `memory_embed_meta`.
pub const META_KEY: &str = "brain_vectors";
/// Bumped when the Lance table layout changes.
const LAYOUT: &str = "lance:brain_memories:v1";
const EMBED_CHUNK: usize = 64;

// ---------------------------------------------------------------------------
// Sync bridge over the async LanceDB API

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
///
/// * No runtime on this thread: plain `block_on`.
/// * Inside a multi-thread Tokio runtime (Recon turns): `block_in_place`, so the
///   worker hands its other tasks off while we wait, then `block_on` there.
/// * Inside a current-thread runtime, where `block_in_place` would panic: run the
///   future from a scoped helper thread and join it.
///
/// Calling `Runtime::block_on` directly inside a runtime panics; this never does.
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
        Ok(_) => std::thread::scope(|scope| {
            match scope.spawn(|| runtime().block_on(future)).join() {
                Ok(output) => output,
                Err(panic) => std::panic::resume_unwind(panic),
            }
        }),
    }
}

// ---------------------------------------------------------------------------
// Diagnostics: index failures never fail a turn, so remember the last one.

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

// ---------------------------------------------------------------------------
// Fingerprint in SQLite

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
        params![META_KEY, current_fingerprint(), chrono::Utc::now().to_rfc3339()],
    )?;
    Ok(())
}

pub fn clear_fingerprint(conn: &Connection) -> Result<()> {
    conn.execute("DELETE FROM memory_embed_meta WHERE key=?1", [META_KEY])?;
    Ok(())
}

// ---------------------------------------------------------------------------
// The index

pub struct BrainIndex {
    uri: PathBuf,
    table: Mutex<Option<lancedb::Table>>,
    /// Set once this process has checked the fingerprint and reconciled ids.
    ready: AtomicBool,
}

fn schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("memory_id", DataType::Utf8, false),
        Field::new(
            "vector",
            DataType::FixedSizeList(
                Arc::new(Field::new("item", DataType::Float32, true)),
                embed::DIM as i32,
            ),
            false,
        ),
    ]))
}

fn batch(rows: &[(String, Vec<f32>)]) -> Result<RecordBatch> {
    for (id, vector) in rows {
        anyhow::ensure!(
            vector.len() == embed::DIM,
            "vector for {id} has {} dims, expected {}",
            vector.len(),
            embed::DIM
        );
    }
    let ids = StringArray::from_iter_values(rows.iter().map(|(id, _)| id.as_str()));
    let vectors = FixedSizeListArray::from_iter_primitive::<Float32Type, _, _>(
        rows.iter()
            .map(|(_, vector)| Some(vector.iter().copied().map(Some))),
        embed::DIM as i32,
    );
    Ok(RecordBatch::try_new(
        schema(),
        vec![Arc::new(ids), Arc::new(vectors)],
    )?)
}

fn quote(id: &str) -> String {
    format!("'{}'", id.replace('\'', "''"))
}

fn id_filter(ids: &[String]) -> String {
    let list: Vec<String> = ids.iter().map(|id| quote(id)).collect();
    format!("memory_id IN ({})", list.join(","))
}

impl BrainIndex {
    /// The process-wide handle for one Lance directory. Each `Store::open` reuses it,
    /// so the fingerprint check and the open table survive store reopen.
    pub fn shared(uri: &Path) -> Arc<BrainIndex> {
        static REGISTRY: OnceLock<Mutex<HashMap<PathBuf, Arc<BrainIndex>>>> = OnceLock::new();
        let registry = REGISTRY.get_or_init(|| Mutex::new(HashMap::new()));
        let mut map = registry.lock().unwrap_or_else(|poison| poison.into_inner());
        map.entry(uri.to_path_buf())
            .or_insert_with(|| Arc::new(BrainIndex::new(uri)))
            .clone()
    }

    pub fn new(uri: &Path) -> Self {
        Self {
            uri: uri.to_path_buf(),
            table: Mutex::new(None),
            ready: AtomicBool::new(false),
        }
    }

    pub fn uri(&self) -> &Path {
        &self.uri
    }

    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::Acquire)
    }

    pub fn mark_ready(&self) {
        self.ready.store(true, Ordering::Release);
    }

    /// Forces the next use to re-check the fingerprint and reconcile ids.
    pub fn mark_stale(&self) {
        self.ready.store(false, Ordering::Release);
    }

    /// True once the Lance table exists on disk.
    pub fn exists(&self) -> bool {
        self.uri.join(format!("{TABLE}.lance")).is_dir()
    }

    async fn connect(uri: PathBuf) -> Result<lancedb::Connection> {
        std::fs::create_dir_all(&uri)?;
        let uri = uri
            .to_str()
            .ok_or_else(|| anyhow!("Lance path {} is not UTF-8", uri.display()))?
            .to_string();
        Ok(lancedb::connect(&uri).execute().await?)
    }

    /// Opens the table, creating an empty one the first time.
    fn table(&self) -> Result<lancedb::Table> {
        let mut slot = self.table.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(table) = slot.as_ref() {
            return Ok(table.clone());
        }
        let uri = self.uri.clone();
        let table = block_on(async move {
            let db = Self::connect(uri).await?;
            match db.open_table(TABLE).execute().await {
                Ok(table) => anyhow::Ok(table),
                Err(lancedb::Error::TableNotFound { .. }) => {
                    Ok(db.create_empty_table(TABLE, schema()).execute().await?)
                }
                Err(err) => Err(err.into()),
            }
        })?;
        *slot = Some(table.clone());
        Ok(table)
    }

    pub fn upsert(&self, id: &str, text: &str) -> Result<()> {
        let vector = embed::embed_one(text)?;
        self.upsert_vectors(&[(id.to_string(), vector)])
    }

    /// Embeds and writes `(memory_id, text)` pairs, replacing earlier rows for those ids.
    pub fn upsert_texts(&self, rows: &[(String, String)]) -> Result<()> {
        for chunk in rows.chunks(EMBED_CHUNK) {
            let texts: Vec<&str> = chunk.iter().map(|(_, text)| text.as_str()).collect();
            let vectors = embed::embed_batch(&texts)?;
            let pairs: Vec<(String, Vec<f32>)> = chunk
                .iter()
                .map(|(id, _)| id.clone())
                .zip(vectors)
                .collect();
            self.upsert_vectors(&pairs)?;
        }
        Ok(())
    }

    /// Deletes rows with these ids, then appends one Arrow batch.
    pub fn upsert_vectors(&self, rows: &[(String, Vec<f32>)]) -> Result<()> {
        if rows.is_empty() {
            return Ok(());
        }
        let data = batch(rows)?;
        let filter = id_filter(&rows.iter().map(|(id, _)| id.clone()).collect::<Vec<_>>());
        let table = self.table()?;
        block_on(async move {
            table.delete(filter.as_str()).await?;
            table.add(data).execute().await?;
            anyhow::Ok(())
        })
    }

    pub fn remove(&self, id: &str) -> Result<()> {
        self.remove_many(&[id.to_string()])
    }

    pub fn remove_many(&self, ids: &[String]) -> Result<()> {
        if ids.is_empty() || !self.exists() {
            return Ok(());
        }
        let table = self.table()?;
        let filter = id_filter(ids);
        block_on(async move {
            table.delete(filter.as_str()).await?;
            anyhow::Ok(())
        })
    }

    /// Top `k` memories by cosine similarity (`1 - cosine distance`) to `query`.
    pub fn search(&self, query: &str, k: usize) -> Result<Vec<(String, f32)>> {
        if query.trim().is_empty() || k == 0 {
            return Ok(Vec::new());
        }
        let vector = embed::embed_one(query)?;
        self.search_vector(&vector, k)
    }

    pub fn search_vector(&self, vector: &[f32], k: usize) -> Result<Vec<(String, f32)>> {
        anyhow::ensure!(vector.len() == embed::DIM, "query vector has wrong width");
        if k == 0 {
            return Ok(Vec::new());
        }
        let table = self.table()?;
        let vector = vector.to_vec();
        let batches: Vec<RecordBatch> = block_on(async move {
            if table.count_rows(None).await? == 0 {
                return anyhow::Ok(Vec::new());
            }
            Ok(table
                .query()
                .nearest_to(vector)?
                .distance_type(DistanceType::Cosine)
                .select(Select::columns(&["memory_id"]))
                .limit(k)
                .execute()
                .await?
                .try_collect()
                .await?)
        })?;
        let mut hits = Vec::new();
        for batch in &batches {
            let ids = batch
                .column_by_name("memory_id")
                .and_then(|col| col.as_any().downcast_ref::<StringArray>())
                .ok_or_else(|| anyhow!("search result has no memory_id"))?;
            let distances = batch
                .column_by_name("_distance")
                .and_then(|col| col.as_any().downcast_ref::<Float32Array>())
                .ok_or_else(|| anyhow!("search result has no _distance"))?;
            for row in 0..batch.num_rows() {
                if ids.is_null(row) {
                    continue;
                }
                hits.push((ids.value(row).to_string(), 1.0 - distances.value(row)));
            }
        }
        hits.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        hits.truncate(k);
        Ok(hits)
    }

    /// The closest memory when its similarity reaches `threshold` (see
    /// [`DUPLICATE_THRESHOLD`]).
    pub fn find_similar(&self, text: &str, threshold: f32) -> Result<Option<(String, f32)>> {
        Ok(self
            .search(text, 1)?
            .into_iter()
            .next()
            .filter(|(_, score)| *score >= threshold))
    }

    /// Every memory id in the table.
    pub fn ids(&self) -> Result<Vec<String>> {
        if !self.exists() {
            return Ok(Vec::new());
        }
        let table = self.table()?;
        let batches: Vec<RecordBatch> = block_on(async move {
            Ok::<_, anyhow::Error>(
                table
                    .query()
                    .select(Select::columns(&["memory_id"]))
                    .execute()
                    .await?
                    .try_collect()
                    .await?,
            )
        })?;
        let mut out = Vec::new();
        for batch in &batches {
            let ids = batch
                .column_by_name("memory_id")
                .and_then(|col| col.as_any().downcast_ref::<StringArray>())
                .ok_or_else(|| anyhow!("scan result has no memory_id"))?;
            out.extend((0..batch.num_rows()).map(|row| ids.value(row).to_string()));
        }
        Ok(out)
    }

    pub fn count(&self) -> Result<usize> {
        if !self.exists() {
            return Ok(0);
        }
        let table = self.table()?;
        block_on(async move { Ok(table.count_rows(None).await?) })
    }

    /// Drops and recreates the table from `(memory_id, text)` pairs.
    pub fn rebuild(&self, memories: &[(String, String)]) -> Result<usize> {
        let mut rows = Vec::with_capacity(memories.len());
        for chunk in memories.chunks(EMBED_CHUNK) {
            let texts: Vec<&str> = chunk.iter().map(|(_, text)| text.as_str()).collect();
            let vectors = embed::embed_batch(&texts)?;
            rows.extend(chunk.iter().map(|(id, _)| id.clone()).zip(vectors));
        }
        self.rebuild_vectors(&rows)
    }

    pub fn rebuild_vectors(&self, rows: &[(String, Vec<f32>)]) -> Result<usize> {
        let mut slot = self.table.lock().unwrap_or_else(|p| p.into_inner());
        *slot = None;
        let uri = self.uri.clone();
        let data = if rows.is_empty() {
            None
        } else {
            Some(batch(rows)?)
        };
        let table = block_on(async move {
            let db = Self::connect(uri).await?;
            match db.drop_table(TABLE, &[]).await {
                Ok(()) | Err(lancedb::Error::TableNotFound { .. }) => {}
                Err(err) => return Err(err.into()),
            }
            let table = match data {
                Some(data) => db.create_table(TABLE, data).execute().await?,
                None => db.create_empty_table(TABLE, schema()).execute().await?,
            };
            anyhow::Ok(table)
        })?;
        *slot = Some(table);
        Ok(rows.len())
    }
}


/// Durable generation metadata for automatic rebuilds (spec §9.2).
pub fn migrate_generations(conn: &rusqlite::Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS argos_index_generations (
            id TEXT PRIMARY KEY,
            fingerprint TEXT NOT NULL,
            state TEXT NOT NULL,
            serving INTEGER NOT NULL DEFAULT 0,
            source_count INTEGER NOT NULL DEFAULT 0,
            batch_cursor INTEGER NOT NULL DEFAULT 0,
            error TEXT NOT NULL DEFAULT '',
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );",
    )?;
    Ok(())
}

/// Start a new generation without dropping the serving table.
pub fn begin_generation(conn: &rusqlite::Connection, source_count: usize) -> anyhow::Result<String> {
    migrate_generations(conn)?;
    let id = format!("gen-{}", chrono::Utc::now().timestamp_millis());
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO argos_index_generations(id,fingerprint,state,serving,source_count,batch_cursor,created_at,updated_at)
         VALUES (?1,?2,'building',0,?3,0,?4,?4)",
        rusqlite::params![id, current_fingerprint(), source_count as i64, now],
    )?;
    Ok(id)
}

pub fn activate_generation(conn: &rusqlite::Connection, id: &str) -> anyhow::Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE argos_index_generations SET serving=0, updated_at=?1 WHERE serving=1",
        rusqlite::params![now],
    )?;
    conn.execute(
        "UPDATE argos_index_generations SET state='active', serving=1, updated_at=?1 WHERE id=?2",
        rusqlite::params![now, id],
    )?;
    write_fingerprint(conn)?;
    Ok(())
}

pub fn serving_generation(conn: &rusqlite::Connection) -> anyhow::Result<Option<String>> {
    migrate_generations(conn)?;
    let id = conn
        .query_row(
            "SELECT id FROM argos_index_generations WHERE serving=1 LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    Ok(id)
}


#[cfg(test)]
mod tests {
    use super::*;

    fn unit(seed: usize) -> Vec<f32> {
        let mut v = vec![0.0f32; embed::DIM];
        v[seed % embed::DIM] = 1.0;
        v[(seed * 7 + 3) % embed::DIM] = 0.5;
        embed::normalize(v)
    }

    fn roundtrip(index: &BrainIndex) {
        index
            .upsert_vectors(&[("a".into(), unit(1)), ("b'q".into(), unit(2))])
            .unwrap();
        let hits = index.search_vector(&unit(2), 2).unwrap();
        assert_eq!(hits[0].0, "b'q");
        assert!((hits[0].1 - 1.0).abs() < 1e-4, "{hits:?}");
        // Upsert replaces, it does not duplicate.
        index.upsert_vectors(&[("a".into(), unit(5))]).unwrap();
        assert_eq!(index.count().unwrap(), 2);
        assert_eq!(index.search_vector(&unit(5), 1).unwrap()[0].0, "a");
        index.remove("a").unwrap();
        assert_eq!(index.ids().unwrap(), vec!["b'q".to_string()]);
    }

    #[test]
    fn upsert_search_remove_round_trip_outside_a_runtime() {
        let dir = tempfile::tempdir().unwrap();
        let index = BrainIndex::new(&dir.path().join("memory_lancedb"));
        assert!(!index.exists());
        assert!(index.search_vector(&unit(1), 3).unwrap().is_empty());
        roundtrip(&index);
        assert!(index.exists());
        assert_eq!(index.rebuild_vectors(&[("z".into(), unit(9))]).unwrap(), 1);
        assert_eq!(index.ids().unwrap(), vec!["z".to_string()]);
    }

    /// Recon calls `Store::recall` from inside its async turn on a multi-thread runtime.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn sync_wrapper_works_inside_a_multi_thread_runtime() {
        let dir = tempfile::tempdir().unwrap();
        let index = BrainIndex::new(&dir.path().join("memory_lancedb"));
        roundtrip(&index);
    }

    /// `block_in_place` panics on a current-thread runtime; the wrapper must not.
    #[tokio::test(flavor = "current_thread")]
    async fn sync_wrapper_works_inside_a_current_thread_runtime() {
        let dir = tempfile::tempdir().unwrap();
        let index = BrainIndex::new(&dir.path().join("memory_lancedb"));
        roundtrip(&index);
    }

    #[test]
    fn begin_generation_then_activate() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE memory_embed_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL, updated_at TEXT NOT NULL);").unwrap();
        let id = begin_generation(&conn, 10).unwrap();
        assert!(serving_generation(&conn).unwrap().is_none());
        activate_generation(&conn, &id).unwrap();
        assert_eq!(serving_generation(&conn).unwrap().as_deref(), Some(id.as_str()));
        assert!(fingerprint_matches(&conn).unwrap());
    }

    #[test]
    fn fingerprint_round_trips_through_memory_embed_meta() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE memory_embed_meta(key TEXT PRIMARY KEY, value TEXT NOT NULL, updated_at TEXT NOT NULL)",
        )
        .unwrap();
        assert!(!fingerprint_matches(&conn).unwrap());
        write_fingerprint(&conn).unwrap();
        assert!(fingerprint_matches(&conn).unwrap());
        conn.execute("UPDATE memory_embed_meta SET value='old'", []).unwrap();
        assert!(!fingerprint_matches(&conn).unwrap());
    }
}
