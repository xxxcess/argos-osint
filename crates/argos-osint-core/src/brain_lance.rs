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

// ---------------------------------------------------------------------------
// The index

pub struct BrainIndex {
    uri: PathBuf,
    table: Mutex<Option<lancedb::Table>>,
    /// Lance table name currently used for search/upsert (serving generation).
    serving_name: Mutex<String>,
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
            serving_name: Mutex::new(TABLE.to_string()),
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
        let name = self.serving_table_name();
        self.uri.join(format!("{name}.lance")).is_dir()
    }

    async fn connect(uri: PathBuf) -> Result<lancedb::Connection> {
        std::fs::create_dir_all(&uri)?;
        let uri = uri
            .to_str()
            .ok_or_else(|| anyhow!("Lance path {} is not UTF-8", uri.display()))?
            .to_string();
        Ok(lancedb::connect(&uri).execute().await?)
    }

    pub fn serving_table_name(&self) -> String {
        self.serving_name
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// Point search/live upserts at `name` and drop the cached handle.
    pub fn set_serving_table(&self, name: &str) {
        let mut slot = self.serving_name.lock().unwrap_or_else(|p| p.into_inner());
        *slot = name.to_string();
        *self.table.lock().unwrap_or_else(|p| p.into_inner()) = None;
        self.mark_stale();
    }

    /// Opens the serving table, creating an empty one the first time.
    fn table(&self) -> Result<lancedb::Table> {
        let name = self.serving_table_name();
        self.table_named(&name)
    }

    /// Open or create a named Lance table in this directory (shadow generations).
    pub fn table_named(&self, name: &str) -> Result<lancedb::Table> {
        let serving = self.serving_table_name();
        if name == serving {
            let mut slot = self.table.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(table) = slot.as_ref() {
                return Ok(table.clone());
            }
            let table = self.open_or_create(name)?;
            *slot = Some(table.clone());
            return Ok(table);
        }
        self.open_or_create(name)
    }

    fn open_or_create(&self, name: &str) -> Result<lancedb::Table> {
        let uri = self.uri.clone();
        let name = name.to_string();
        block_on(async move {
            let db = Self::connect(uri).await?;
            match db.open_table(&name).execute().await {
                Ok(table) => anyhow::Ok(table),
                Err(lancedb::Error::TableNotFound { .. }) => {
                    Ok(db.create_empty_table(&name, schema()).execute().await?)
                }
                Err(err) => Err(err.into()),
            }
        })
    }

    /// Upsert into a shadow building table (does not touch the serving table).
    pub fn upsert_texts_into(&self, table_name: &str, rows: &[(String, String)]) -> Result<()> {
        for chunk in rows.chunks(EMBED_CHUNK) {
            let texts: Vec<&str> = chunk.iter().map(|(_, text)| text.as_str()).collect();
            let vectors = embed::embed_batch(&texts)?;
            let pairs: Vec<(String, Vec<f32>)> = chunk
                .iter()
                .map(|(id, _)| id.clone())
                .zip(vectors)
                .collect();
            self.upsert_vectors_into(table_name, &pairs)?;
        }
        Ok(())
    }

    pub fn upsert_vectors_into(&self, table_name: &str, rows: &[(String, Vec<f32>)]) -> Result<()> {
        if rows.is_empty() {
            return Ok(());
        }
        let data = batch(rows)?;
        let filter = id_filter(&rows.iter().map(|(id, _)| id.clone()).collect::<Vec<_>>());
        let table = self.table_named(table_name)?;
        block_on(async move {
            let _ = table.delete(filter.as_str()).await;
            table.add(data).execute().await?;
            anyhow::Ok(())
        })
    }

    /// Drop a named table if it exists (best effort).
    pub fn drop_table(&self, name: &str) -> Result<()> {
        if name == TABLE || name.is_empty() {
            return Ok(());
        }
        let uri = self.uri.clone();
        let name = name.to_string();
        block_on(async move {
            let db = Self::connect(uri).await?;
            match db.drop_table(&name, &[]).await {
                Ok(()) => Ok(()),
                Err(lancedb::Error::TableNotFound { .. }) => Ok(()),
                Err(err) => Err(err.into()),
            }
        })
    }

    pub fn table_exists_named(&self, name: &str) -> bool {
        self.uri.join(format!("{name}.lance")).is_dir()
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

    /// Exact id lookup in the serving table (a filter scan, not similarity):
    /// which of `ids` currently have a vector row.
    pub fn present_ids(&self, ids: &[String]) -> Result<std::collections::HashSet<String>> {
        let mut out = std::collections::HashSet::new();
        if ids.is_empty() || !self.exists() {
            return Ok(out);
        }
        let table = self.table()?;
        for chunk in ids.chunks(256) {
            let filter = id_filter(chunk);
            let table = table.clone();
            let batches: Vec<RecordBatch> = block_on(async move {
                Ok::<_, anyhow::Error>(
                    table
                        .query()
                        .only_if(filter)
                        .select(Select::columns(&["memory_id"]))
                        .execute()
                        .await?
                        .try_collect()
                        .await?,
                )
            })?;
            for batch in &batches {
                let col = batch
                    .column_by_name("memory_id")
                    .and_then(|col| col.as_any().downcast_ref::<StringArray>())
                    .ok_or_else(|| anyhow!("lookup result has no memory_id"))?;
                out.extend((0..batch.num_rows()).map(|row| col.value(row).to_string()));
            }
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

    /// Whether a vector ANN index currently exists on the serving table.
    pub fn has_vector_ann_index(&self) -> bool {
        if !self.exists() {
            return false;
        }
        let table = match self.table() {
            Ok(t) => t,
            Err(_) => return false,
        };
        block_on(async move {
            match table.list_indices().await {
                Ok(indices) => indices.iter().any(|idx| {
                    matches!(
                        idx.index_type,
                        lancedb::index::IndexType::IvfFlat
                            | lancedb::index::IndexType::IvfPq
                            | lancedb::index::IndexType::IvfSq
                            | lancedb::index::IndexType::IvfHnswFlat
                            | lancedb::index::IndexType::IvfHnswPq
                            | lancedb::index::IndexType::IvfHnswSq
                    ) || idx.columns.iter().any(|c| c == "vector")
                }),
                Err(_) => false,
            }
        })
    }

    /// Create an IVF-Flat ANN index when `policy` is [`crate::evidence::AnnPolicy::AnnEnabled`].
    /// Uses cosine distance to match [`Self::search_vector`]. Returns whether an index
    /// was created (false when policy forbids it or the table is empty).
    pub fn ensure_ann_index(&self, policy: crate::evidence::AnnPolicy) -> Result<bool> {
        if !policy.uses_ann() {
            return Ok(false);
        }
        if !self.exists() || self.count()? == 0 {
            return Ok(false);
        }
        if self.has_vector_ann_index() {
            return Ok(true);
        }
        let rows = self.count()?;
        // Keep partitions small enough to train on modest corpora.
        let partitions = ((rows as f64).sqrt() as u32).clamp(2, 64);
        let table = self.table()?;
        block_on(async move {
            use lancedb::index::vector::IvfFlatIndexBuilder;
            use lancedb::index::Index;
            use lancedb::DistanceType;
            let builder = IvfFlatIndexBuilder::default()
                .distance_type(DistanceType::Cosine)
                .num_partitions(partitions);
            table
                .create_index(&["vector"], Index::IvfFlat(builder))
                .execute()
                .await?;
            anyhow::Ok(())
        })?;
        Ok(true)
    }

    /// Offline measurement helper: exact search IDs for synthetic query vectors.
    pub fn exact_top_ids(&self, query: &[f32], k: usize) -> Result<Vec<String>> {
        Ok(self
            .search_vector(query, k)?
            .into_iter()
            .map(|(id, _)| id)
            .collect())
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

/// Start a new generation without dropping the serving table.
pub fn begin_generation(
    conn: &rusqlite::Connection,
    source_count: usize,
) -> anyhow::Result<String> {
    migrate_generations(conn)?;
    let id = format!("gen-{}", chrono::Utc::now().timestamp_millis());
    let now = chrono::Utc::now().to_rfc3339();
    let table_name = shadow_table_name(&id);
    conn.execute(
        "INSERT INTO argos_index_generations(id,fingerprint,state,serving,source_count,batch_cursor,table_name,created_at,updated_at)
         VALUES (?1,?2,'building',0,?3,0,?4,?5,?5)",
        rusqlite::params![id, current_fingerprint(), source_count as i64, table_name, now],
    )?;
    Ok(id)
}

pub fn shadow_table_name(generation_id: &str) -> String {
    let safe: String = generation_id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("{TABLE}__{safe}")
}

pub fn generation_table_name(conn: &rusqlite::Connection, id: &str) -> anyhow::Result<String> {
    let name: String = conn.query_row(
        "SELECT table_name FROM argos_index_generations WHERE id=?1",
        [id],
        |row| row.get(0),
    )?;
    if name.is_empty() {
        Ok(shadow_table_name(id))
    } else {
        Ok(name)
    }
}

pub fn serving_table_name(conn: &rusqlite::Connection) -> anyhow::Result<Option<String>> {
    migrate_generations(conn)?;
    let row: Option<(String, String)> = conn
        .query_row(
            "SELECT id, table_name FROM argos_index_generations WHERE serving=1 LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    Ok(row.map(|(id, name)| {
        if name.is_empty() {
            shadow_table_name(&id)
        } else {
            name
        }
    }))
}

/// Mark `id` as the serving generation and point the Lance index at its shadow table.
pub fn activate_generation(
    conn: &rusqlite::Connection,
    id: &str,
    index: Option<&BrainIndex>,
) -> anyhow::Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    let previous_table = serving_table_name(conn)?;
    let table_name = generation_table_name(conn, id)?;
    // Ensure empty shadow generations still have a table handle before promote.
    if let Some(index) = index {
        let _ = index.table_named(&table_name)?;
    }
    conn.execute(
        "UPDATE argos_index_generations SET serving=0, updated_at=?1 WHERE serving=1",
        rusqlite::params![now],
    )?;
    conn.execute(
        "UPDATE argos_index_generations SET state='active', serving=1, updated_at=?1 WHERE id=?2",
        rusqlite::params![now, id],
    )?;
    write_fingerprint(conn)?;
    if let Some(index) = index {
        let old = index.serving_table_name();
        index.set_serving_table(&table_name);
        // Drop prior shadow (never drop the legacy default table name used as bootstrap).
        if let Some(ref prev) = previous_table {
            if prev.as_str() != table_name && prev.as_str() != TABLE {
                let _ = index.drop_table(prev);
            }
        }
        if old != table_name && old != TABLE && Some(old.as_str()) != previous_table.as_deref() {
            let _ = index.drop_table(&old);
        }
        index.mark_ready();
    }
    Ok(())
}

const GENERATION_BATCH: usize = 64;

/// Advance a building generation by embedding up to `GENERATION_BATCH` memories,
/// checkpointing the cursor. Does not drop the serving table. Refuses to activate
/// when the generation fingerprint no longer matches the process fingerprint
/// (embedding-space mix guard).
pub fn rebuild_generation_batched(
    conn: &rusqlite::Connection,
    index: &BrainIndex,
    generation_id: &str,
    memories: &[(String, String)],
) -> anyhow::Result<GenerationProgress> {
    migrate_generations(conn)?;
    let (state, fingerprint, cursor, source_count, table_name): (String, String, i64, i64, String) =
        conn.query_row(
            "SELECT state, fingerprint, batch_cursor, source_count, table_name FROM argos_index_generations WHERE id=?1",
            [generation_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )?;
    anyhow::ensure!(
        state == "building" || state == "replaying",
        "generation {generation_id} is not buildable (state={state})"
    );
    anyhow::ensure!(
        fingerprint == current_fingerprint(),
        "generation fingerprint mismatch; refusing to mix embedding spaces"
    );
    let building = if table_name.is_empty() {
        let name = shadow_table_name(generation_id);
        conn.execute(
            "UPDATE argos_index_generations SET table_name=?1 WHERE id=?2",
            rusqlite::params![&name, generation_id],
        )?;
        name
    } else {
        table_name
    };
    let cursor = cursor as usize;
    if cursor >= memories.len() {
        activate_generation(conn, generation_id, Some(index))?;
        return Ok(GenerationProgress {
            generation_id: generation_id.into(),
            cursor: memories.len(),
            total: memories.len(),
            activated: true,
        });
    }
    let end = (cursor + GENERATION_BATCH).min(memories.len());
    let slice = &memories[cursor..end];
    // Build into the shadow generation table — serving table stays readable until activate.
    index.upsert_texts_into(&building, slice)?;
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE argos_index_generations SET batch_cursor=?1, state='building', updated_at=?2 WHERE id=?3",
        rusqlite::params![end as i64, now, generation_id],
    )?;
    let activated = end >= memories.len() || end as i64 >= source_count;
    if activated {
        activate_generation(conn, generation_id, Some(index))?;
    }
    Ok(GenerationProgress {
        generation_id: generation_id.into(),
        cursor: end,
        total: memories.len(),
        activated,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GenerationProgress {
    pub generation_id: String,
    pub cursor: usize,
    pub total: usize,
    pub activated: bool,
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
    fn rebuild_generation_batched_respects_fingerprint() {
        let dir = tempfile::tempdir().unwrap();
        let lance = dir.path().join("lance");
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE memory_embed_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL, updated_at TEXT NOT NULL);",
        )
        .unwrap();
        let index = BrainIndex::shared(&lance);
        let id = begin_generation(&conn, 2).unwrap();
        let memories = vec![
            ("m1".into(), "Harbor tanker manifests".into()),
            ("m2".into(), "Night desk shift".into()),
        ];
        // Force fingerprint mismatch
        conn.execute(
            "UPDATE argos_index_generations SET fingerprint='other' WHERE id=?1",
            [&id],
        )
        .unwrap();
        assert!(rebuild_generation_batched(&conn, &index, &id, &memories).is_err());
    }

    #[test]
    fn begin_generation_then_activate() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE memory_embed_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL, updated_at TEXT NOT NULL);").unwrap();
        let id = begin_generation(&conn, 10).unwrap();
        assert!(serving_generation(&conn).unwrap().is_none());
        activate_generation(&conn, &id, None).unwrap();
        assert_eq!(
            serving_generation(&conn).unwrap().as_deref(),
            Some(id.as_str())
        );
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
        conn.execute("UPDATE memory_embed_meta SET value='old'", [])
            .unwrap();
        assert!(!fingerprint_matches(&conn).unwrap());
    }
    #[test]
    fn rebuild_writes_shadow_then_activates_serving_pointer() {
        let dir = tempfile::tempdir().unwrap();
        let lance = dir.path().join("lance");
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE memory_embed_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL, updated_at TEXT NOT NULL);",
        )
        .unwrap();
        let index = BrainIndex::shared(&lance);
        index.upsert_vectors(&[("legacy".into(), unit(1))]).unwrap();
        assert_eq!(index.serving_table_name(), TABLE);
        let id = begin_generation(&conn, 2).unwrap();
        let building = generation_table_name(&conn, &id).unwrap();
        assert!(building.starts_with(&format!("{TABLE}__")));
        // Offline: write vectors into the shadow table the same way batched rebuild does.
        index
            .upsert_vectors_into(&building, &[("m1".into(), unit(3)), ("m2".into(), unit(4))])
            .unwrap();
        assert_eq!(
            index.serving_table_name(),
            TABLE,
            "serving unchanged while building"
        );
        assert!(index.table_exists_named(&building) || index.ids().is_ok());
        activate_generation(&conn, &id, Some(&index)).unwrap();
        assert_eq!(index.serving_table_name(), building);
        assert_eq!(
            serving_generation(&conn).unwrap().as_deref(),
            Some(id.as_str())
        );
        let ids = index.ids().unwrap();
        assert!(
            ids.contains(&"m1".to_string()) && ids.contains(&"m2".to_string()),
            "{ids:?}"
        );
        assert!(!ids.contains(&"legacy".to_string()));
    }

    #[test]
    fn ensure_ann_index_respects_exact_policy() {
        let dir = tempfile::tempdir().unwrap();
        let index = BrainIndex::new(&dir.path().join("lance"));
        for i in 0..40 {
            index.upsert_vectors(&[(format!("m{i}"), unit(i))]).unwrap();
        }
        assert!(!index
            .ensure_ann_index(crate::evidence::AnnPolicy::ExactSearch)
            .unwrap());
        assert!(!index.has_vector_ann_index());
    }

    #[test]
    fn measured_ann_can_build_ivf_when_enabled() {
        let dir = tempfile::tempdir().unwrap();
        let index = BrainIndex::new(&dir.path().join("lance"));
        let n = 80usize;
        for i in 0..n {
            index
                .upsert_vectors(&[(format!("m{i}"), unit(i * 3 + 1))])
                .unwrap();
        }
        // Build under AnnEnabled (criteria already decided by caller/harness).
        assert!(index
            .ensure_ann_index(crate::evidence::AnnPolicy::AnnEnabled)
            .unwrap());
        assert!(index.has_vector_ann_index());
        let q = unit(1);
        let hits = index.search_vector(&q, 5).unwrap();
        assert!(!hits.is_empty());
    }
}
