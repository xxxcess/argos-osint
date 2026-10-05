//! Persistent Brain memories and their conversation provenance.

use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

use crate::brain::{
    hybrid_recall, normalize_category, recall, Memory, MemorySource, ScoredMemory,
};
use crate::brain_lance::{self, BrainIndex};

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

/// Schema version this build writes. 18 adds durable task/index-change tables
/// (`argos_jobs`, `argos_tasks`, `argos_attempts`, `argos_index_changes`,
/// `argos_derived_summaries`). 17 added `memory_embed_meta` and dropped sqlite-vec.
pub const SCHEMA_VERSION: i64 = 18;

/// Soft hint only: sync rebuild above this size is skipped in favor of an
/// asynchronous `argos_index_changes` rebuild enqueue (no manual reindex required).
pub const AUTO_REBUILD_HINT: usize = 64;

pub struct Store {
    pub(crate) conn: Connection,
    /// Lance vector index beside the database. None for in-memory stores and when
    /// `ARGOS_EMBED=0`.
    vectors: Option<Arc<BrainIndex>>,
}

/// Result of [`Store::commit_article_insight_replacement`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArticleInsightCommit {
    pub claims: usize,
    pub preserved_user_edits: usize,
}

/// What `Store::reindex_memory_vectors` did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReindexReport {
    pub memories: usize,
    pub lance_dir: std::path::PathBuf,
    pub fingerprint: String,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path).with_context(|| format!("open {}", path.display()))?;
        let vectors = crate::embed::enabled()
            .then(|| BrainIndex::shared(&crate::paths::lancedb_dir_for(path)));
        let store = Self { conn, vectors };
        store.ensure_schema()?;
        Ok(store)
    }

    pub fn memory() -> Result<Self> {
        let store = Self {
            conn: Connection::open_in_memory()?,
            vectors: None,
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
                version <= SCHEMA_VERSION,
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
            if version < 11 {
                self.conn.execute_batch(
                    "CREATE TABLE IF NOT EXISTS atlas_runs (
                       id TEXT PRIMARY KEY,
                       state TEXT NOT NULL,
                       phase INTEGER NOT NULL DEFAULT 1,
                       cursor_json TEXT NOT NULL DEFAULT '',
                       stats_json TEXT NOT NULL DEFAULT '',
                       note TEXT NOT NULL DEFAULT '',
                       started_at TEXT NOT NULL,
                       finished_at TEXT NOT NULL DEFAULT ''
                     );
                     CREATE TABLE IF NOT EXISTS atlas_quota (
                       provider TEXT NOT NULL,
                       day TEXT NOT NULL,
                       used INTEGER NOT NULL,
                       PRIMARY KEY (provider, day)
                     );",
                )?;
                self.conn.pragma_update(None, "user_version", 11)?;
            }
            if version < 12 {
                self.conn.execute_batch(
                    "CREATE TABLE IF NOT EXISTS atlas_articles (
                       run_id TEXT NOT NULL,
                       article_id TEXT NOT NULL,
                       title TEXT NOT NULL,
                       description TEXT NOT NULL,
                       url TEXT NOT NULL,
                       country TEXT NOT NULL,
                       source_name TEXT NOT NULL,
                       source_domain TEXT NOT NULL,
                       published_at TEXT NOT NULL,
                       provider TEXT NOT NULL,
                       temperature REAL NOT NULL,
                       category TEXT NOT NULL DEFAULT 'unk',
                       seen_at TEXT NOT NULL,
                       PRIMARY KEY (run_id, article_id)
                     );",
                )?;
                self.conn.pragma_update(None, "user_version", 12)?;
            }
            if version < 13 {
                let columns: Vec<String> = {
                    let mut stmt = self.conn.prepare("PRAGMA table_info(atlas_articles)")?;
                    let columns = stmt
                        .query_map([], |row| row.get(1))?
                        .collect::<rusqlite::Result<_>>()?;
                    columns
                };
                if !columns.iter().any(|name| name == "author") {
                    self.conn.execute_batch(
                        "ALTER TABLE atlas_articles ADD COLUMN author TEXT NOT NULL DEFAULT ''",
                    )?;
                }
                if !columns.iter().any(|name| name == "image_url") {
                    self.conn.execute_batch(
                        "ALTER TABLE atlas_articles ADD COLUMN image_url TEXT NOT NULL DEFAULT ''",
                    )?;
                }
                self.conn.pragma_update(None, "user_version", 13)?;
            }
            if version < 14 {
                let thread_columns: Vec<String> = {
                    let mut stmt = self.conn.prepare("PRAGMA table_info(recon_threads)")?;
                    let columns = stmt
                        .query_map([], |row| row.get(1))?
                        .collect::<rusqlite::Result<_>>()?;
                    columns
                };
                if !thread_columns.iter().any(|name| name == "recall_insights") {
                    self.conn.execute_batch(
                        "ALTER TABLE recon_threads ADD COLUMN recall_insights INTEGER NOT NULL DEFAULT 0",
                    )?;
                }
                let source_columns: Vec<String> = {
                    let mut stmt = self.conn.prepare("PRAGMA table_info(insight_sources)")?;
                    let columns = stmt
                        .query_map([], |row| row.get(1))?
                        .collect::<rusqlite::Result<_>>()?;
                    columns
                };
                if !source_columns.iter().any(|name| name == "published_at") {
                    self.conn.execute_batch(
                        "ALTER TABLE insight_sources ADD COLUMN published_at TEXT NOT NULL DEFAULT ''",
                    )?;
                }
                self.conn.pragma_update(None, "user_version", 14)?;
            }
            if version < 15 {
                let claim_columns: Vec<String> = {
                    let mut stmt = self.conn.prepare("PRAGMA table_info(insight_claims)")?;
                    let columns = stmt
                        .query_map([], |row| row.get(1))?
                        .collect::<rusqlite::Result<_>>()?;
                    columns
                };
                if !claim_columns
                    .iter()
                    .any(|name| name == "source_reliability")
                {
                    self.conn.execute_batch(
                        "ALTER TABLE insight_claims ADD COLUMN source_reliability TEXT NOT NULL DEFAULT '';
                         ALTER TABLE insight_claims ADD COLUMN info_credibility INTEGER NOT NULL DEFAULT 0;
                         ALTER TABLE insight_claims ADD COLUMN admiralty TEXT NOT NULL DEFAULT '';
                         ALTER TABLE insight_claims ADD COLUMN rsp_status TEXT NOT NULL DEFAULT '';",
                    )?;
                }
                self.conn.pragma_update(None, "user_version", 15)?;
            }
            if version < 16 {
                self.conn
                    .execute_batch(include_str!("schema_intel_recon.sql"))?;
                self.conn.pragma_update(None, "user_version", 16)?;
            }
            // v17: Brain vectors moved to LanceDB. Runs on every open as well, because
            // unreleased local builds already used user_version 17 for sqlite-vec.
            repair_embed_tables(&self.conn)?;
            if version < 17 {
                self.conn.pragma_update(None, "user_version", 17)?;
            }
            // v18: durable jobs/tasks/attempts, index change queue, derived summaries.
            if version < 18 {
                crate::tasks::migrate_tables(&self.conn)?;
                self.conn.pragma_update(None, "user_version", 18)?;
            } else {
                // Idempotent ensure for databases already at 18+.
                crate::tasks::migrate_tables(&self.conn)?;
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

    /// Substring search across memory text, category, claim fields, and source JSON.
    pub fn search_memories(&self, query: &str) -> Result<Vec<Memory>> {
        let needle = like_needle(query);
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT m.id,m.text,m.category,m.pinned,m.created_at,m.source_json
             FROM memories m
             LEFT JOIN insight_claims c ON c.memory_id = m.id
             WHERE m.text LIKE ?1 ESCAPE '\\'
                OR m.category LIKE ?1 ESCAPE '\\'
                OR IFNULL(c.entity_id,'') LIKE ?1 ESCAPE '\\'
                OR IFNULL(c.predicate,'') LIKE ?1 ESCAPE '\\'
                OR IFNULL(c.object_value,'') LIKE ?1 ESCAPE '\\'
                OR m.source_json LIKE ?1 ESCAPE '\\'
             ORDER BY m.pinned DESC, m.created_at DESC",
        )?;
        let rows = stmt.query_map([needle], memory_row)?;
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
        self.index_upsert(std::slice::from_ref(&memory.id));
        Ok(memory)
    }

    /// Hybrid recall: Lance vector search blended with Jaccard ([`hybrid_recall`]).
    /// Falls back to Jaccard alone when embedding is off or the index fails; an
    /// index problem never fails the caller.
    pub fn recall(&self, query: &str, top_k: usize) -> Result<Vec<ScoredMemory>> {
        let memories = self.list_memories()?;
        if query.trim().is_empty() || memories.is_empty() || top_k == 0 {
            return Ok(Vec::new());
        }
        if let Some(index) = self.vector_index() {
            match index.search(query, (top_k * 3).max(16)) {
                Ok(hits) => return Ok(hybrid_recall(&memories, query, &hits, top_k)),
                Err(err) => {
                    brain_lance::note_error(&err);
                    index.mark_stale();
                }
            }
        }
        Ok(recall(&memories, query, top_k))
    }

    /// True when recall will use the vector index (embedding on, file-backed store).
    pub fn vectors_enabled(&self) -> bool {
        self.vectors.is_some()
    }

    /// The closest existing memory to `text` at or above `threshold` cosine
    /// similarity (use [`brain_lance::DUPLICATE_THRESHOLD`] for near-duplicates).
    /// None when embedding is off or the index is unavailable.
    pub fn find_similar_memory(&self, text: &str, threshold: f32) -> Option<(String, f32)> {
        let index = self.vector_index()?;
        match index.find_similar(text, threshold) {
            Ok(hit) => hit,
            Err(err) => {
                brain_lance::note_error(&err);
                None
            }
        }
    }

    fn memory_texts(&self, ids: Option<&[String]>) -> Result<Vec<(String, String)>> {
        let mut out = Vec::new();
        match ids {
            None => {
                let mut stmt = self.conn.prepare("SELECT id,text FROM memories")?;
                let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
                for row in rows {
                    out.push(row?);
                }
            }
            Some(ids) => {
                let mut stmt = self.conn.prepare("SELECT id,text FROM memories WHERE id=?1")?;
                for id in ids {
                    if let Some(row) = stmt
                        .query_row([id], |row| Ok((row.get(0)?, row.get(1)?)))
                        .optional()?
                    {
                        out.push(row);
                    }
                }
            }
        }
        Ok(out)
    }

    /// The index when it is usable, checking it against SQLite once per process.
    fn vector_index(&self) -> Option<&BrainIndex> {
        let index = self.vectors.as_deref()?;
        if index.is_ready() {
            return Some(index);
        }
        match self.ensure_memory_vectors() {
            Ok(()) => Some(index),
            Err(err) => {
                brain_lance::note_error(&err);
                None
            }
        }
    }

    /// Brings the Lance index in line with SQLite. Small missing/mismatched indexes
    /// rebuild inline; larger ones enqueue an automatic generation rebuild via
    /// `argos_index_changes` so foreground recall is never blocked on a 3,000-row
    /// sync barrier. Incremental id reconciliation still runs when the serving
    /// generation is compatible.
    pub fn ensure_memory_vectors(&self) -> Result<()> {
        let index = self
            .vectors
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("Brain vectors are off (ARGOS_EMBED=0 or in-memory store)"))?;
        crate::embed::warm_up()?;
        let rows = self.memory_texts(None)?;
        if !index.exists() || !brain_lance::fingerprint_matches(&self.conn)? {
            if rows.len() <= AUTO_REBUILD_HINT {
                brain_lance::clear_fingerprint(&self.conn)?;
                index.rebuild(&rows)?;
                brain_lance::write_fingerprint(&self.conn)?;
            } else {
                // Do not block recall. Queue a durable rebuild and keep Jaccard until ready.
                let now = chrono::Utc::now().to_rfc3339();
                let _ = crate::tasks::enqueue_index_change(
                    &self.conn,
                    "memory_index",
                    "generation",
                    &format!("count={}", rows.len()),
                    "rebuild",
                    &now,
                );
                return Ok(());
            }
        } else {
            let have: HashSet<String> = index.ids()?.into_iter().collect();
            let want: HashSet<&str> = rows.iter().map(|(id, _)| id.as_str()).collect();
            let missing: Vec<(String, String)> = rows
                .iter()
                .filter(|(id, _)| !have.contains(id))
                .cloned()
                .collect();
            let stale: Vec<String> = have
                .iter()
                .filter(|id| !want.contains(id.as_str()))
                .cloned()
                .collect();
            index.upsert_texts(&missing)?;
            index.remove_many(&stale)?;
        }
        index.mark_ready();
        Ok(())
    }

    /// Count user-edited insight memories linked to one Atlas article.
    pub fn article_insight_user_edit_count(&self, run_id: &str, article_id: &str) -> Result<usize> {
        let answer_id = atlas_answer_id(run_id);
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(DISTINCT c.memory_id)
             FROM insight_sources s
             JOIN insight_claims c ON c.fingerprint = s.fingerprint
             JOIN insight_user_edits e ON e.memory_id = c.memory_id
             WHERE s.run_id=?1 AND s.answer_id=?2 AND s.call_id=?3",
            params![run_id, answer_id, article_id],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    /// Atomically replace one article's insight sources with already-validated claims.
    /// Prior insights stay live until this commits. User-edited memories are retained
    /// even when this article was their only source. Vector hooks run after COMMIT.
    pub fn commit_article_insight_replacement(
        &self,
        run_id: &str,
        article_id: &str,
        claims: &[AtlasInsightClaim],
        relations: &[(String, String, String)],
    ) -> Result<ArticleInsightCommit> {
        let answer_id = atlas_answer_id(run_id);
        let now = chrono::Utc::now().to_rfc3339();
        let fingerprints: Vec<String> = {
            let mut stmt = self.conn.prepare(
                "SELECT DISTINCT fingerprint FROM insight_sources                  WHERE run_id=?1 AND answer_id=?2 AND call_id=?3",
            )?;
            let rows = stmt.query_map(params![run_id, answer_id, article_id], |row| row.get(0))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        let mut doomed: Vec<String> = Vec::new();
        let mut written: Vec<String> = Vec::new();
        let mut preserved = 0usize;
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| -> Result<ArticleInsightCommit> {
            self.conn.execute(
                "DELETE FROM insight_sources WHERE run_id=?1 AND answer_id=?2 AND call_id=?3",
                params![run_id, answer_id, article_id],
            )?;
            for fingerprint in &fingerprints {
                let remaining: i64 = self.conn.query_row(
                    "SELECT COUNT(*) FROM insight_sources WHERE fingerprint=?1",
                    [fingerprint],
                    |row| row.get(0),
                )?;
                if remaining > 0 {
                    continue;
                }
                let memory_id: Option<String> = self
                    .conn
                    .query_row(
                        "SELECT memory_id FROM insight_claims WHERE fingerprint=?1",
                        [fingerprint],
                        |row| row.get(0),
                    )
                    .optional()?;
                let Some(memory_id) = memory_id else {
                    self.conn.execute(
                        "DELETE FROM insight_relations WHERE left_fingerprint=?1 OR right_fingerprint=?1",
                        [fingerprint],
                    )?;
                    self.conn.execute(
                        "DELETE FROM insight_claims WHERE fingerprint=?1",
                        [fingerprint],
                    )?;
                    continue;
                };
                let user_edited: i64 = self.conn.query_row(
                    "SELECT COUNT(*) FROM insight_user_edits WHERE memory_id=?1",
                    [&memory_id],
                    |row| row.get(0),
                )?;
                if user_edited > 0 {
                    preserved += 1;
                    continue;
                }
                self.conn.execute(
                    "DELETE FROM insight_relations WHERE left_fingerprint=?1 OR right_fingerprint=?1",
                    [fingerprint],
                )?;
                self.conn.execute(
                    "DELETE FROM insight_claims WHERE fingerprint=?1",
                    [fingerprint],
                )?;
                self.conn.execute(
                    "DELETE FROM memory_graph_summaries WHERE memory_id=?1",
                    [&memory_id],
                )?;
                self.conn
                    .execute("DELETE FROM memories WHERE id=?1", [&memory_id])?;
                doomed.push(memory_id);
            }

            let mut committed = 0usize;
            for claim in claims {
                let entity = claim.entity.trim().to_ascii_lowercase();
                let namespace = claim.namespace.trim().to_ascii_lowercase();
                let predicate = claim.predicate.trim().to_ascii_lowercase();
                let object = claim.object.trim().to_ascii_lowercase();
                let sentence = claim.claim.trim();
                anyhow::ensure!(
                    !entity.is_empty()
                        && !namespace.is_empty()
                        && !predicate.is_empty()
                        && !object.is_empty()
                        && !sentence.is_empty()
                        && claim.article_id.trim() == article_id,
                    "invalid staged claim during commit"
                );
                let fingerprint = insight_fingerprint(&namespace, &entity, &predicate, &object);
                let existing: Option<String> = self
                    .conn
                    .query_row(
                        "SELECT memory_id FROM insight_claims WHERE fingerprint=?1",
                        [&fingerprint],
                        |row| row.get(0),
                    )
                    .optional()?;
                if existing.is_none() {
                    let memory_id = new_id();
                    let source = MemorySource {
                        app: "atlas".into(),
                        conversation_id: run_id.into(),
                        message_id: None,
                        reference: Some(run_id.into()),
                    };
                    self.conn.execute(
                        "INSERT INTO memories(id,text,category,pinned,created_at,source_json) VALUES (?1,?2,'investigation',0,?3,?4)",
                        params![memory_id, sentence, now, serde_json::to_string(&source)?],
                    )?;
                    written.push(memory_id.clone());
                    self.conn.execute(
                        "INSERT INTO insight_claims(fingerprint,memory_id,entity_id,predicate,object_value,topic,classification,confidence,created_at,updated_at,source_reliability,info_credibility,admiralty,rsp_status) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?9,?10,?11,?12,?13)",
                        params![
                            fingerprint,
                            memory_id,
                            entity,
                            predicate,
                            object,
                            claim.topic.trim(),
                            claim.classification.trim(),
                            claim.confidence,
                            now,
                            claim.reliability.trim(),
                            claim.info_credibility as i64,
                            claim.admiralty.trim(),
                            claim.rsp_status.trim(),
                        ],
                    )?;
                }
                self.conn.execute(
                    "INSERT OR IGNORE INTO insight_sources(fingerprint,thread_id,run_id,answer_id,call_id,source_url,published_at) VALUES (?1,NULL,?2,?3,?4,?5,?6)",
                    params![
                        fingerprint,
                        run_id,
                        answer_id,
                        article_id,
                        claim.source_url.trim(),
                        claim.published_at.trim(),
                    ],
                )?;
                self.conn.execute(
                    "UPDATE insight_claims SET confidence=?1, source_reliability=?2, info_credibility=?3, admiralty=?4, rsp_status=?5, updated_at=?6 WHERE fingerprint=?7",
                    params![
                        claim.confidence,
                        claim.reliability.trim(),
                        claim.info_credibility as i64,
                        claim.admiralty.trim(),
                        claim.rsp_status.trim(),
                        now,
                        fingerprint,
                    ],
                )?;
                committed += 1;
            }
            for (left, right, relation) in relations {
                if left.is_empty() || right.is_empty() || relation.is_empty() || left == right {
                    continue;
                }
                self.conn.execute(
                    "INSERT OR IGNORE INTO insight_relations(left_fingerprint,right_fingerprint,relation) VALUES (?1,?2,?3)",
                    params![left, right, relation],
                )?;
            }
            Ok(ArticleInsightCommit {
                claims: committed,
                preserved_user_edits: preserved,
            })
        })();
        match result {
            Ok(outcome) => {
                self.conn.execute_batch("COMMIT")?;
                self.index_remove_missing(&doomed);
                self.index_upsert(&written);
                Ok(outcome)
            }
            Err(err) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(err)
            }
        }
    }

    /// Drops and rebuilds the Lance table from every memory, then writes the
    /// fingerprint. Backs `argos memories reindex`.
    pub fn reindex_memory_vectors(&self) -> Result<ReindexReport> {
        let index = self
            .vectors
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("Brain vectors are off (ARGOS_EMBED=0); nothing to reindex"))?;
        crate::embed::warm_up()?;
        index.mark_stale();
        let rows = self.memory_texts(None)?;
        brain_lance::clear_fingerprint(&self.conn)?;
        let memories = index.rebuild(&rows)?;
        brain_lance::write_fingerprint(&self.conn)?;
        index.mark_ready();
        Ok(ReindexReport {
            memories,
            lance_dir: index.uri().to_path_buf(),
            fingerprint: brain_lance::current_fingerprint(),
        })
    }

    /// Index hook after memories were written or their text changed. Best effort.
    pub(crate) fn index_upsert(&self, ids: &[String]) {
        if ids.is_empty() || self.vectors.is_none() {
            return;
        }
        let Some(index) = self.vector_index() else {
            return;
        };
        let result = self
            .memory_texts(Some(ids))
            .and_then(|rows| index.upsert_texts(&rows));
        if let Err(err) = result {
            brain_lance::note_error(&err);
            index.mark_stale();
        }
    }

    /// Index hook after memories may have been deleted: drops vectors for the ids
    /// that are gone from SQLite. Best effort.
    pub(crate) fn index_remove_missing(&self, ids: &[String]) {
        if ids.is_empty() || self.vectors.is_none() {
            return;
        }
        let Some(index) = self.vector_index() else {
            return;
        };
        let result = (|| -> Result<()> {
            let present: HashSet<String> = self
                .memory_texts(Some(ids))?
                .into_iter()
                .map(|(id, _)| id)
                .collect();
            let gone: Vec<String> = ids
                .iter()
                .filter(|id| !present.contains(*id))
                .cloned()
                .collect();
            index.remove_many(&gone)
        })();
        if let Err(err) = result {
            brain_lance::note_error(&err);
            index.mark_stale();
        }
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
            self.index_upsert(&[id.to_string()]);
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
                if deleted {
                    self.index_remove_missing(&[id.to_string()]);
                }
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

    pub fn atlas_insert_run(&self, id: &str, cursor_json: &str, stats_json: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO atlas_runs (id, state, phase, cursor_json, stats_json, note, started_at, finished_at)
             VALUES (?1, 'running', 1, ?2, ?3, '', ?4, '')",
            params![id, cursor_json, stats_json, chrono::Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    pub fn atlas_save(&self, id: &str, cursor_json: &str, stats_json: &str) -> Result<()> {
        let phase = serde_json::from_str::<serde_json::Value>(cursor_json)
            .ok()
            .and_then(|value| value.get("phase").and_then(|phase| phase.as_i64()))
            .unwrap_or(1);
        self.conn.execute(
            "UPDATE atlas_runs SET phase=?2, cursor_json=?3, stats_json=?4 WHERE id=?1",
            params![id, phase, cursor_json, stats_json],
        )?;
        Ok(())
    }

    pub fn atlas_set_state(&self, id: &str, state: &str, note: &str, finished: bool) -> Result<()> {
        let finished_at = if finished {
            chrono::Utc::now().to_rfc3339()
        } else {
            String::new()
        };
        self.conn.execute(
            "UPDATE atlas_runs SET state=?2, note=?3, finished_at=CASE WHEN ?4 = '' THEN finished_at ELSE ?4 END WHERE id=?1",
            params![id, state, note, finished_at],
        )?;
        Ok(())
    }

    pub fn atlas_latest_run(&self) -> Result<Option<AtlasRunRow>> {
        Ok(self.atlas_list_runs()?.into_iter().next())
    }

    pub fn atlas_list_runs(&self) -> Result<Vec<AtlasRunRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, state, phase, cursor_json, stats_json, note, started_at, finished_at
             FROM atlas_runs ORDER BY started_at DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(AtlasRunRow {
                id: row.get(0)?,
                state: row.get(1)?,
                phase: row.get(2)?,
                cursor_json: row.get(3)?,
                stats_json: row.get(4)?,
                note: row.get(5)?,
                started_at: row.get(6)?,
                finished_at: row.get(7)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn atlas_delete_run(&self, id: &str) -> Result<bool> {
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| -> Result<bool> {
            self.delete_cycle_memories(id)?;
            self.conn
                .execute("DELETE FROM atlas_articles WHERE run_id=?1", [id])?;
            Ok(self
                .conn
                .execute("DELETE FROM atlas_runs WHERE id=?1", [id])?
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

    /// Drops insight source rows for one Atlas article and orphans Brain memories that
    /// no longer have any source. Fingerprints still linked from other articles/cycles keep
    /// their claim and memory.
    pub fn delete_article_insights(&self, run_id: &str, article_id: &str) -> Result<usize> {
        let answer_id = atlas_answer_id(run_id);
        let fingerprints: Vec<String> = {
            let mut stmt = self.conn.prepare(
                "SELECT DISTINCT fingerprint FROM insight_sources \
                 WHERE run_id=?1 AND answer_id=?2 AND call_id=?3",
            )?;
            let rows = stmt.query_map(params![run_id, answer_id, article_id], |row| row.get(0))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        if fingerprints.is_empty() {
            return Ok(0);
        }
        let mut doomed: Vec<String> = Vec::new();
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| -> Result<usize> {
            let removed = self.conn.execute(
                "DELETE FROM insight_sources WHERE run_id=?1 AND answer_id=?2 AND call_id=?3",
                params![run_id, answer_id, article_id],
            )?;
            let mut orphaned = 0usize;
            for fingerprint in &fingerprints {
                let remaining: i64 = self.conn.query_row(
                    "SELECT COUNT(*) FROM insight_sources WHERE fingerprint=?1",
                    [fingerprint],
                    |row| row.get(0),
                )?;
                if remaining > 0 {
                    continue;
                }
                let memory_id: Option<String> = self
                    .conn
                    .query_row(
                        "SELECT memory_id FROM insight_claims WHERE fingerprint=?1",
                        [fingerprint],
                        |row| row.get(0),
                    )
                    .optional()?;
                self.conn.execute(
                    "DELETE FROM insight_relations WHERE left_fingerprint=?1 OR right_fingerprint=?1",
                    [fingerprint],
                )?;
                self.conn.execute(
                    "DELETE FROM insight_claims WHERE fingerprint=?1",
                    [fingerprint],
                )?;
                if let Some(memory_id) = memory_id {
                    self.conn.execute(
                        "DELETE FROM insight_user_edits WHERE memory_id=?1",
                        [&memory_id],
                    )?;
                    self.conn.execute(
                        "DELETE FROM memory_graph_summaries WHERE memory_id=?1",
                        [&memory_id],
                    )?;
                    self.conn
                        .execute("DELETE FROM memories WHERE id=?1", [&memory_id])?;
                    doomed.push(memory_id);
                    orphaned += 1;
                }
            }
            let _ = removed;
            Ok(orphaned)
        })();
        match result {
            Ok(orphaned) => {
                self.conn.execute_batch("COMMIT")?;
                self.index_remove_missing(&doomed);
                Ok(orphaned)
            }
            Err(err) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(err)
            }
        }
    }

    /// Drops Brain memories written from one Atlas cycle.
    /// A claim that another cycle also sourced keeps its memory and loses only this cycle's source row.
    pub fn delete_cycle_memories(&self, run_id: &str) -> Result<()> {
        let answer_id = atlas_answer_id(run_id);
        let fingerprints: Vec<String> = {
            let mut stmt = self.conn.prepare(
                "SELECT DISTINCT fingerprint FROM insight_sources WHERE run_id=?1 AND answer_id=?2",
            )?;
            let rows = stmt.query_map(params![run_id, answer_id], |row| row.get(0))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        self.conn.execute(
            "DELETE FROM insight_sources WHERE run_id=?1 AND answer_id=?2",
            params![run_id, answer_id],
        )?;
        let mut doomed: Vec<String> = Vec::new();
        for fingerprint in fingerprints {
            let remaining: i64 = self.conn.query_row(
                "SELECT COUNT(*) FROM insight_sources WHERE fingerprint=?1",
                [&fingerprint],
                |row| row.get(0),
            )?;
            if remaining > 0 {
                continue;
            }
            let memory_id: Option<String> = self
                .conn
                .query_row(
                    "SELECT memory_id FROM insight_claims WHERE fingerprint=?1",
                    [&fingerprint],
                    |row| row.get(0),
                )
                .optional()?;
            self.conn.execute(
                "DELETE FROM insight_relations WHERE left_fingerprint=?1 OR right_fingerprint=?1",
                [&fingerprint],
            )?;
            self.conn.execute(
                "DELETE FROM insight_claims WHERE fingerprint=?1",
                [&fingerprint],
            )?;
            if let Some(memory_id) = memory_id {
                self.conn.execute(
                    "DELETE FROM memory_graph_summaries WHERE memory_id=?1",
                    [&memory_id],
                )?;
                self.conn
                    .execute("DELETE FROM memories WHERE id=?1", [&memory_id])?;
                doomed.push(memory_id);
            }
        }
        if let Some(memory_id) = atlas_brief_id(&self.conn, run_id)? {
            self.conn.execute(
                "DELETE FROM memory_graph_summaries WHERE memory_id=?1",
                [&memory_id],
            )?;
            self.conn
                .execute("DELETE FROM memories WHERE id=?1", [&memory_id])?;
            doomed.push(memory_id);
        }
        self.index_remove_missing(&doomed);
        Ok(())
    }

    pub fn atlas_upsert_article(&self, row: &AtlasArticleRow) -> Result<()> {
        self.conn.execute(
            "INSERT INTO atlas_articles (
                run_id, article_id, title, description, url, country, source_name, source_domain,
                published_at, provider, temperature, category, seen_at, author, image_url
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)
             ON CONFLICT(run_id, article_id) DO UPDATE SET
                title=excluded.title,
                description=excluded.description,
                url=excluded.url,
                country=excluded.country,
                source_name=excluded.source_name,
                source_domain=excluded.source_domain,
                published_at=excluded.published_at,
                provider=excluded.provider,
                temperature=excluded.temperature,
                category=excluded.category,
                seen_at=excluded.seen_at,
                author=excluded.author,
                image_url=excluded.image_url",
            params![
                row.run_id,
                row.id,
                row.title,
                row.description,
                row.url,
                row.country,
                row.source_name,
                row.source_domain,
                row.published_at,
                row.provider,
                row.temperature,
                row.category,
                row.seen_at,
                row.author,
                row.image_url,
            ],
        )?;
        Ok(())
    }

    pub fn atlas_delete_article(&self, run_id: &str, article_id: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM atlas_articles WHERE run_id=?1 AND article_id=?2",
            params![run_id, article_id],
        )?;
        Ok(())
    }

    pub fn atlas_set_category(&self, run_id: &str, article_id: &str, category: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE atlas_articles SET category=?3 WHERE run_id=?1 AND article_id=?2",
            params![run_id, article_id, category],
        )?;
        Ok(())
    }

    pub fn atlas_list_articles(&self, run_id: &str) -> Result<Vec<AtlasArticleRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT run_id, article_id, title, description, url, country, source_name, source_domain,
                    published_at, provider, temperature, category, seen_at, author, image_url
             FROM atlas_articles WHERE run_id=?1 ORDER BY seen_at, article_id",
        )?;
        let rows = stmt.query_map([run_id], |row| atlas_article_from_row(row))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Every stored article still within the retention window (prune drops older runs).
    /// Used to seed Atlas dedup so a new cycle skips headlines already processed.
    pub fn atlas_recent_articles(&self) -> Result<Vec<AtlasArticleRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT run_id, article_id, title, description, url, country, source_name, source_domain,
                    published_at, provider, temperature, category, seen_at, author, image_url
             FROM atlas_articles
             ORDER BY seen_at DESC, article_id",
        )?;
        let rows = stmt.query_map([], |row| atlas_article_from_row(row))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn atlas_article(&self, run_id: &str, article_id: &str) -> Result<Option<AtlasArticleRow>> {
        Ok(self
            .atlas_list_articles(run_id)?
            .into_iter()
            .find(|row| row.id == article_id))
    }

    /// Distinct UTC calendar days (`YYYY-MM-DD`) that have an Atlas run, newest first.
    pub fn atlas_run_days(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT substr(started_at, 1, 10) AS day
             FROM atlas_runs
             WHERE length(started_at) >= 10
             ORDER BY day DESC",
        )?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Articles for the Intel bulletin: one OSINT category, one run day, optional search.
    pub fn atlas_articles_for_intel(
        &self,
        category: &str,
        day: &str,
        query: &str,
    ) -> Result<Vec<AtlasArticleRow>> {
        let q = query.trim();
        let like = if q.is_empty() {
            String::new()
        } else {
            format!("%{}%", q.to_ascii_lowercase())
        };
        let sql = if like.is_empty() {
            "SELECT a.run_id, a.article_id, a.title, a.description, a.url, a.country,
                    a.source_name, a.source_domain, a.published_at, a.provider, a.temperature,
                    a.category, a.seen_at, a.author, a.image_url
             FROM atlas_articles a
             JOIN atlas_runs r ON r.id = a.run_id
             WHERE a.category = ?1 AND substr(r.started_at, 1, 10) = ?2
             ORDER BY a.published_at DESC, a.seen_at DESC, a.article_id"
        } else {
            "SELECT a.run_id, a.article_id, a.title, a.description, a.url, a.country,
                    a.source_name, a.source_domain, a.published_at, a.provider, a.temperature,
                    a.category, a.seen_at, a.author, a.image_url
             FROM atlas_articles a
             JOIN atlas_runs r ON r.id = a.run_id
             WHERE a.category = ?1 AND substr(r.started_at, 1, 10) = ?2
               AND (
                 lower(a.title) LIKE ?3 OR
                 lower(a.description) LIKE ?3 OR
                 lower(a.source_name) LIKE ?3 OR
                 lower(a.source_domain) LIKE ?3 OR
                 lower(a.url) LIKE ?3
               )
             ORDER BY a.published_at DESC, a.seen_at DESC, a.article_id"
        };
        let mut stmt = self.conn.prepare(sql)?;
        let rows = if like.is_empty() {
            stmt.query_map(params![category, day], atlas_article_from_row)?
        } else {
            stmt.query_map(params![category, day, like], atlas_article_from_row)?
        };
        // Same URL (article_id) can appear in multiple runs on one day; keep the newest.
        Ok(dedupe_articles_by_id(
            rows.collect::<rusqlite::Result<_>>()?,
        ))
    }

    /// Claims linked to one Atlas article (`insight_sources.call_id` stores article_id).
    pub fn atlas_claims_for_article(
        &self,
        run_id: &str,
        article_id: &str,
    ) -> Result<Vec<AtlasArticleClaim>> {
        let answer_id = atlas_answer_id(run_id);
        let mut stmt = self.conn.prepare(
            "SELECT c.fingerprint, c.entity_id, c.predicate, c.object_value, c.topic,
                    c.classification, c.confidence, m.text, s.source_url, s.published_at, s.call_id,
                    c.source_reliability, c.info_credibility, c.admiralty, c.rsp_status
             FROM insight_claims c
             JOIN insight_sources s ON s.fingerprint = c.fingerprint
             JOIN memories m ON m.id = c.memory_id
             WHERE s.run_id=?1 AND s.answer_id=?2 AND s.call_id=?3
             GROUP BY c.fingerprint
             ORDER BY c.created_at",
        )?;
        let rows = stmt.query_map(params![run_id, answer_id, article_id], |row| {
            Ok(AtlasArticleClaim {
                fingerprint: row.get(0)?,
                entity: row.get(1)?,
                predicate: row.get(2)?,
                object: row.get(3)?,
                topic: row.get(4)?,
                classification: row.get(5)?,
                confidence: row.get(6)?,
                claim: row.get(7)?,
                source_url: row.get(8)?,
                published_at: row.get(9)?,
                article_id: row.get(10)?,
                reliability: row.get(11)?,
                info_credibility: row.get::<_, i64>(12)? as u8,
                admiralty: row.get(13)?,
                rsp_status: row.get(14)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// A run left `running` by a closed process can be resumed.
    pub fn atlas_park_running(&self) -> Result<()> {
        self.conn.execute(
            "UPDATE atlas_runs SET state='paused', note='Paused' WHERE state='running'",
            [],
        )?;
        Ok(())
    }

    pub fn atlas_quota_used(&self, provider: &str, day: &str) -> Result<u32> {
        let used: Option<i64> = self
            .conn
            .query_row(
                "SELECT used FROM atlas_quota WHERE provider=?1 AND day=?2",
                params![provider, day],
                |row| row.get(0),
            )
            .optional()?;
        Ok(used.unwrap_or(0) as u32)
    }

    pub fn atlas_quota_bump(&self, provider: &str, day: &str) -> Result<u32> {
        self.conn.execute(
            "INSERT INTO atlas_quota (provider, day, used) VALUES (?1, ?2, 1)
             ON CONFLICT(provider, day) DO UPDATE SET used = used + 1",
            params![provider, day],
        )?;
        self.atlas_quota_used(provider, day)
    }

    pub fn app_state_get(&self, key: &str) -> Result<Option<String>> {
        let value: Option<String> = self
            .conn
            .query_row("SELECT value FROM app_state WHERE key=?1", [key], |row| {
                row.get(0)
            })
            .optional()?;
        Ok(value)
    }

    pub fn app_state_set(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO app_state(key,value) VALUES (?1,?2)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    /// Unix time of the next automatic Atlas run. Absent means auto run is off.
    pub fn atlas_auto_next(&self) -> Result<Option<u64>> {
        let value: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM app_state WHERE key='atlas_auto_next'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        Ok(value.and_then(|text| text.parse().ok()))
    }

    pub fn set_atlas_auto_next(&self, when: Option<u64>) -> Result<()> {
        match when {
            Some(secs) => {
                self.conn.execute(
                    "INSERT INTO app_state(key,value) VALUES ('atlas_auto_next',?1)
                     ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                    [secs.to_string()],
                )?;
            }
            None => {
                self.conn
                    .execute("DELETE FROM app_state WHERE key='atlas_auto_next'", [])?;
            }
        }
        Ok(())
    }

    /// Drops finished runs that started more than 36 hours ago, including their
    /// statistics and articles. Runs that are still running or paused stay so they can resume.
    pub fn atlas_prune_expired(&self) -> Result<Vec<String>> {
        let cutoff = (chrono::Utc::now() - chrono::Duration::hours(36)).to_rfc3339();
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| -> Result<Vec<String>> {
            let mut stmt = self.conn.prepare(
                "SELECT id FROM atlas_runs
                 WHERE started_at < ?1 AND state NOT IN ('running', 'paused')",
            )?;
            let ids = stmt
                .query_map([cutoff.as_str()], |row| row.get(0))?
                .collect::<rusqlite::Result<Vec<String>>>()?;
            drop(stmt);
            // Retain articles linked to Intel Recon investigations/reports.
            let protected: std::collections::HashSet<String> = {
                let mut stmt = self
                    .conn
                    .prepare("SELECT DISTINCT article_id FROM intel_investigations")?;
                let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
                rows.collect::<rusqlite::Result<_>>()?
            };
            for id in &ids {
                self.delete_cycle_memories(id)?;
                if protected.is_empty() {
                    self.conn
                        .execute("DELETE FROM atlas_articles WHERE run_id=?1", [id])?;
                } else {
                    let mut stmt = self
                        .conn
                        .prepare("SELECT article_id FROM atlas_articles WHERE run_id=?1")?;
                    let article_ids: Vec<String> = stmt
                        .query_map([id], |row| row.get(0))?
                        .collect::<rusqlite::Result<_>>()?;
                    drop(stmt);
                    for article_id in article_ids {
                        if protected.contains(&article_id) {
                            continue;
                        }
                        self.conn.execute(
                            "DELETE FROM atlas_articles WHERE run_id=?1 AND article_id=?2",
                            params![id, article_id],
                        )?;
                    }
                }
                // Keep the run row only when it still owns protected articles.
                let remaining: i64 = self.conn.query_row(
                    "SELECT COUNT(*) FROM atlas_articles WHERE run_id=?1",
                    [id],
                    |row| row.get(0),
                )?;
                if remaining == 0 {
                    self.conn
                        .execute("DELETE FROM atlas_runs WHERE id=?1", [id])?;
                }
            }
            Ok(ids)
        })();
        match result {
            Ok(ids) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(ids)
            }
            Err(err) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(err)
            }
        }
    }

    /// True when this Atlas run already wrote insight sources. A resume skips the model.
    pub fn atlas_has_insights(&self, run_id: &str) -> Result<bool> {
        let answer_id = atlas_answer_id(run_id);
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM insight_sources WHERE run_id=?1 AND answer_id=?2",
            params![run_id, answer_id],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Claims whose sources point at this Atlas run.
    pub fn atlas_stored_claims(&self, run_id: &str) -> Result<Vec<AtlasStoredClaim>> {
        let answer_id = atlas_answer_id(run_id);
        let mut stmt = self.conn.prepare(
            "SELECT c.fingerprint, c.entity_id, c.predicate, c.object_value, c.topic, c.classification,
                    c.source_reliability, c.info_credibility, c.admiralty
             FROM insight_claims c
             JOIN insight_sources s ON s.fingerprint = c.fingerprint
             WHERE s.run_id=?1 AND s.answer_id=?2
             GROUP BY c.fingerprint
             ORDER BY c.created_at",
        )?;
        let rows = stmt.query_map(params![run_id, answer_id], |row| {
            Ok(AtlasStoredClaim {
                fingerprint: row.get(0)?,
                entity: row.get(1)?,
                predicate: row.get(2)?,
                object: row.get(3)?,
                topic: row.get(4)?,
                classification: row.get(5)?,
                reliability: row.get(6)?,
                info_credibility: row.get::<_, i64>(7)? as u8,
                admiralty: row.get(8)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Relations whose endpoints are both in `fingerprints`.
    pub fn insight_relations_among(
        &self,
        fingerprints: &[String],
    ) -> Result<Vec<(String, String, String)>> {
        if fingerprints.is_empty() {
            return Ok(Vec::new());
        }
        let mut stmt = self.conn.prepare(
            "SELECT left_fingerprint, right_fingerprint, relation FROM insight_relations",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        let known: std::collections::HashSet<&str> =
            fingerprints.iter().map(String::as_str).collect();
        let mut kept = Vec::new();
        for row in rows {
            let (left, right, relation) = row?;
            if known.contains(left.as_str()) && known.contains(right.as_str()) {
                kept.push((left, right, relation));
            }
        }
        Ok(kept)
    }

    /// Writes Atlas claims into the same Brain tables Recon reads.
    /// `answer_id` is `atlas-{run_id}` and is not a Recon message.
    pub fn persist_atlas_insights(
        &self,
        run_id: &str,
        claims: &[AtlasInsightClaim],
        relations: &[(String, String, String)],
        brief: &str,
        entity_path: &str,
    ) -> Result<()> {
        if claims.is_empty() {
            return Ok(());
        }
        let answer_id = atlas_answer_id(run_id);
        let now = chrono::Utc::now().to_rfc3339();
        let mut written: Vec<String> = Vec::new();
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| -> Result<()> {
            for claim in claims {
                let entity = claim.entity.trim().to_ascii_lowercase();
                let namespace = claim.namespace.trim().to_ascii_lowercase();
                let predicate = claim.predicate.trim().to_ascii_lowercase();
                let object = claim.object.trim().to_ascii_lowercase();
                let sentence = claim.claim.trim();
                if entity.is_empty()
                    || namespace.is_empty()
                    || predicate.is_empty()
                    || object.is_empty()
                    || sentence.is_empty()
                    || claim.article_id.trim().is_empty()
                {
                    continue;
                }
                let fingerprint = insight_fingerprint(&namespace, &entity, &predicate, &object);
                let existing: Option<String> = self
                    .conn
                    .query_row(
                        "SELECT memory_id FROM insight_claims WHERE fingerprint=?1",
                        [&fingerprint],
                        |row| row.get(0),
                    )
                    .optional()?;
                if existing.is_none() {
                    let memory_id = new_id();
                    let source = MemorySource {
                        app: "atlas".into(),
                        conversation_id: run_id.into(),
                        message_id: None,
                        reference: Some(run_id.into()),
                    };
                    self.conn.execute(
                        "INSERT INTO memories(id,text,category,pinned,created_at,source_json) VALUES (?1,?2,'investigation',0,?3,?4)",
                        params![memory_id, sentence, now, serde_json::to_string(&source)?],
                    )?;
                    written.push(memory_id.clone());
                    self.conn.execute(
                        "INSERT INTO insight_claims(fingerprint,memory_id,entity_id,predicate,object_value,topic,classification,confidence,created_at,updated_at,source_reliability,info_credibility,admiralty,rsp_status) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?9,?10,?11,?12,?13)",
                        params![
                            fingerprint,
                            memory_id,
                            entity,
                            predicate,
                            object,
                            claim.topic.trim(),
                            claim.classification.trim(),
                            claim.confidence,
                            now,
                            claim.reliability.trim(),
                            claim.info_credibility as i64,
                            claim.admiralty.trim(),
                            claim.rsp_status.trim(),
                        ],
                    )?;
                    let mut stmt = self.conn.prepare(
                        "SELECT fingerprint FROM insight_claims WHERE entity_id=?1 AND predicate=?2 AND fingerprint<>?3",
                    )?;
                    let others: Vec<String> = stmt
                        .query_map(params![entity, predicate, fingerprint], |row| row.get(0))?
                        .collect::<rusqlite::Result<_>>()?;
                    drop(stmt);
                    for old in others {
                        self.conn.execute(
                            "INSERT OR IGNORE INTO insight_relations(left_fingerprint,right_fingerprint,relation) VALUES (?1,?2,'conflict_or_revision')",
                            params![old, fingerprint],
                        )?;
                    }
                }
                self.conn.execute(
                    "INSERT OR IGNORE INTO insight_sources(fingerprint,thread_id,run_id,answer_id,call_id,source_url,published_at) VALUES (?1,NULL,?2,?3,?4,?5,?6)",
                    params![
                        fingerprint,
                        run_id,
                        answer_id,
                        claim.article_id.trim(),
                        claim.source_url.trim(),
                        claim.published_at.trim(),
                    ],
                )?;
                self.conn.execute(
                    "UPDATE insight_sources SET published_at=?1 WHERE fingerprint=?2 AND answer_id=?3 AND call_id=?4",
                    params![
                        claim.published_at.trim(),
                        fingerprint,
                        answer_id,
                        claim.article_id.trim(),
                    ],
                )?;
                self.conn.execute(
                    "UPDATE insight_claims SET confidence=?1, source_reliability=?2, info_credibility=?3, admiralty=?4, rsp_status=?5, updated_at=?6 WHERE fingerprint=?7",
                    params![
                        claim.confidence,
                        claim.reliability.trim(),
                        claim.info_credibility as i64,
                        claim.admiralty.trim(),
                        claim.rsp_status.trim(),
                        now,
                        fingerprint,
                    ],
                )?;
            }
            for (left, right, relation) in relations {
                if left.is_empty() || right.is_empty() || relation.is_empty() || left == right {
                    continue;
                }
                self.conn.execute(
                    "INSERT OR IGNORE INTO insight_relations(left_fingerprint,right_fingerprint,relation) VALUES (?1,?2,?3)",
                    params![left, right, relation],
                )?;
            }
            let _ = entity_path;
            let brief = brief.trim();
            if !brief.is_empty() {
                if let Some(memory_id) = atlas_brief_id(&self.conn, run_id)? {
                    self.conn.execute(
                        "UPDATE memories SET text=?1 WHERE id=?2",
                        params![brief, memory_id],
                    )?;
                    written.push(memory_id);
                } else {
                    let memory_id = new_id();
                    let source = MemorySource {
                        app: "atlas".into(),
                        conversation_id: run_id.into(),
                        message_id: None,
                        reference: Some(run_id.into()),
                    };
                    self.conn.execute(
                        "INSERT INTO memories(id,text,category,pinned,created_at,source_json) VALUES (?1,?2,'investigation',0,?3,?4)",
                        params![memory_id, brief, now, serde_json::to_string(&source)?],
                    )?;
                    written.push(memory_id);
                }
            }
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.conn.execute_batch("COMMIT")?;
                self.index_upsert(&written);
                Ok(())
            }
            Err(err) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(err)
            }
        }
    }
}

/// Idempotent v17 step: creates `memory_embed_meta` (the Brain vector fingerprint)
/// and removes the sqlite-vec `memory_vec` table plus its shadow tables. Safe on a
/// database that unreleased local builds already moved to user_version 17 with
/// their own `memory_embed_meta`/`memory_vec`: a meta table of another shape is
/// recreated (it only caches a fingerprint, so the Lance index just rebuilds).
fn repair_embed_tables(conn: &Connection) -> Result<()> {
    let meta_cols: Vec<String> = {
        let mut stmt = conn.prepare("PRAGMA table_info(memory_embed_meta)")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let fits = ["key", "value", "updated_at"]
        .iter()
        .all(|col| meta_cols.iter().any(|have| have == col));
    if !meta_cols.is_empty() && !fits {
        conn.execute_batch("DROP TABLE memory_embed_meta")?;
    }
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS memory_embed_meta (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )",
    )?;
    let has_vec: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE name='memory_vec'",
        [],
        |row| row.get(0),
    )?;
    if has_vec > 0 && conn.execute_batch("DROP TABLE IF EXISTS memory_vec").is_err() {
        // A vec0 virtual table cannot be dropped without the sqlite-vec module
        // ("no such module: vec0"). Remove its schema row directly instead.
        conn.execute_batch(
            "PRAGMA writable_schema=ON;
             DELETE FROM sqlite_master WHERE name='memory_vec';
             PRAGMA writable_schema=OFF;
             PRAGMA writable_schema=RESET;",
        )?;
    }
    let shadows: Vec<String> = {
        let mut stmt = conn.prepare(
            "SELECT name FROM sqlite_master WHERE type='table' AND name LIKE 'memory\\_vec\\_%' ESCAPE '\\'",
        )?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    for name in shadows {
        conn.execute_batch(&format!("DROP TABLE IF EXISTS \"{}\"", name.replace('"', "\"\"")))?;
    }
    Ok(())
}

fn atlas_answer_id(run_id: &str) -> String {
    format!("atlas-{run_id}")
}

fn like_needle(query: &str) -> String {
    let escaped = query
        .trim()
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    format!("%{escaped}%")
}

fn memory_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<crate::brain::Memory> {
    let source: String = row.get(5)?;
    let source = serde_json::from_str(&source).map_err(|err| {
        rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, Box::new(err))
    })?;
    Ok(crate::brain::Memory {
        id: row.get(0)?,
        text: row.get(1)?,
        category: row.get(2)?,
        pinned: row.get::<_, i64>(3)? != 0,
        created_at: row.get(4)?,
        source,
    })
}

/// Fingerprint Recon uses: the JSON array of namespace, entity, predicate, and object.
pub fn insight_fingerprint(namespace: &str, entity: &str, predicate: &str, object: &str) -> String {
    serde_json::to_string(&(namespace, entity, predicate, object)).unwrap_or_default()
}

fn atlas_brief_id(conn: &Connection, run_id: &str) -> Result<Option<String>> {
    let mut stmt = conn.prepare(
        "SELECT m.id, m.source_json FROM memories m
         WHERE m.id NOT IN (SELECT memory_id FROM insight_claims)",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    for row in rows {
        let (id, source_json) = row?;
        let source: MemorySource = match serde_json::from_str(&source_json) {
            Ok(source) => source,
            Err(_) => continue,
        };
        if source.app == "atlas" && source.conversation_id == run_id {
            return Ok(Some(id));
        }
    }
    Ok(None)
}

fn atlas_article_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AtlasArticleRow> {
    Ok(AtlasArticleRow {
        run_id: row.get(0)?,
        id: row.get(1)?,
        title: row.get(2)?,
        description: row.get(3)?,
        url: row.get(4)?,
        country: row.get(5)?,
        source_name: row.get(6)?,
        source_domain: row.get(7)?,
        published_at: row.get(8)?,
        provider: row.get(9)?,
        temperature: row.get(10)?,
        category: row.get(11)?,
        seen_at: row.get(12)?,
        author: row.get(13)?,
        image_url: row.get(14)?,
    })
}

/// Keep the first row for each `article_id` (caller orders newest first).
fn dedupe_articles_by_id(rows: Vec<AtlasArticleRow>) -> Vec<AtlasArticleRow> {
    let mut seen = std::collections::HashSet::new();
    rows.into_iter()
        .filter(|row| seen.insert(row.id.clone()))
        .collect()
}

/// One Atlas pipeline run. Statistics live in `stats_json`.
#[derive(Clone, Debug)]
pub struct AtlasRunRow {
    pub id: String,
    pub state: String,
    pub phase: i64,
    pub cursor_json: String,
    pub stats_json: String,
    pub note: String,
    pub started_at: String,
    pub finished_at: String,
}

/// One headline saved for an Atlas run. `category` is an OSINT tag or `unk`.
#[derive(Clone, Debug, PartialEq)]
pub struct AtlasArticleRow {
    pub run_id: String,
    pub id: String,
    pub title: String,
    pub description: String,
    pub url: String,
    pub country: String,
    pub source_name: String,
    pub source_domain: String,
    pub published_at: String,
    pub provider: String,
    pub temperature: f64,
    pub category: String,
    pub seen_at: String,
    pub author: String,
    pub image_url: String,
}

/// One span-checked claim the Atlas cycle writes into Brain.
#[derive(Clone, Debug, PartialEq)]
pub struct AtlasInsightClaim {
    pub fingerprint: String,
    pub entity: String,
    pub namespace: String,
    pub predicate: String,
    pub object: String,
    pub topic: String,
    pub claim: String,
    pub classification: String,
    pub confidence: f64,
    pub article_id: String,
    pub source_url: String,
    pub published_at: String,
    pub reliability: String,
    pub info_credibility: u8,
    pub admiralty: String,
    pub rsp_status: String,
}

/// A claim already stored for an Atlas run, used when a resume skips the model.
#[derive(Clone, Debug, PartialEq)]
pub struct AtlasStoredClaim {
    pub fingerprint: String,
    pub entity: String,
    pub predicate: String,
    pub object: String,
    pub topic: String,
    pub classification: String,
    pub reliability: String,
    pub info_credibility: u8,
    pub admiralty: String,
}

/// One Atlas claim scoped to a single article, for the Intel briefing view.
#[derive(Clone, Debug, PartialEq)]
pub struct AtlasArticleClaim {
    pub fingerprint: String,
    pub entity: String,
    pub predicate: String,
    pub object: String,
    pub topic: String,
    pub classification: String,
    pub confidence: f64,
    pub claim: String,
    pub source_url: String,
    pub published_at: String,
    pub article_id: String,
    pub reliability: String,
    pub info_credibility: u8,
    pub admiralty: String,
    pub rsp_status: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    mod vectors {
        use super::*;
        use crate::embed::testing;

        fn src() -> MemorySource {
            MemorySource {
                app: "test".into(),
                conversation_id: "c".into(),
                message_id: None,
                reference: None,
            }
        }

        fn version(conn: &Connection) -> i64 {
            conn.pragma_query_value(None, "user_version", |row| row.get(0))
                .unwrap()
        }

        fn has_table(conn: &Connection, name: &str) -> bool {
            conn.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name=?1",
                [name],
                |row| row.get::<_, i64>(0),
            )
            .unwrap()
                > 0
        }

        #[test]
        fn schema_17_has_embed_meta_and_no_memory_vec() {
            let store = Store::memory().unwrap();
            assert_eq!(version(&store.conn), SCHEMA_VERSION);
            assert!(has_table(&store.conn, "memory_embed_meta"));
            assert!(!has_table(&store.conn, "memory_vec"));
        }

        /// x3cess's unreleased local build used user_version 17 for sqlite-vec. Opening
        /// such a DB (vec0 table we cannot load, shadow tables, a differently shaped
        /// meta table) must clean up and keep the memories.
        #[test]
        fn local_sqlite_vec_v17_database_is_repaired_on_open() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("argos.db");
            Store::open(&path)
                .unwrap()
                .add_memory("keep me", "fact", false, src())
                .unwrap();
            {
                let conn = Connection::open(&path).unwrap();
                conn.execute_batch(
                    "DROP TABLE memory_embed_meta;
                     CREATE TABLE memory_embed_meta(id INTEGER PRIMARY KEY, model TEXT, dim INTEGER);
                     CREATE TABLE memory_vec_info(key TEXT PRIMARY KEY, value ANY);
                     CREATE TABLE memory_vec_chunks(chunk_id INTEGER PRIMARY KEY, size INTEGER);
                     CREATE TABLE memory_vec_rowids(rowid INTEGER PRIMARY KEY, id TEXT);
                     PRAGMA writable_schema=ON;
                     INSERT INTO sqlite_master(type,name,tbl_name,rootpage,sql) VALUES ('table','memory_vec','memory_vec',0,'CREATE VIRTUAL TABLE memory_vec USING vec0(memory_id TEXT PRIMARY KEY, embedding float[384])');
                     PRAGMA writable_schema=OFF;
                     PRAGMA user_version=17;",
                )
                .unwrap();
            }
            {
                let conn = Connection::open(&path).unwrap();
                assert!(
                    conn.execute_batch("DROP TABLE memory_vec").is_err(),
                    "the planted vec0 table needs the missing module"
                );
            }
            let store = Store::open(&path).unwrap();
            assert_eq!(version(&store.conn), SCHEMA_VERSION);
            assert!(!has_table(&store.conn, "memory_vec"));
            assert!(has_table(&store.conn, "argos_tasks"));
            for shadow in ["memory_vec_info", "memory_vec_chunks", "memory_vec_rowids"] {
                assert!(!has_table(&store.conn, shadow), "{shadow} left behind");
            }
            brain_lance::write_fingerprint(&store.conn).unwrap();
            assert!(brain_lance::fingerprint_matches(&store.conn).unwrap());
            assert_eq!(store.list_memories().unwrap()[0].text, "keep me");
            let ok: String = store
                .conn
                .query_row("PRAGMA integrity_check", [], |row| row.get(0))
                .unwrap();
            assert_eq!(ok, "ok");
            drop(store);
            // Idempotent: reopening a repaired DB keeps the fingerprint row.
            let again = Store::open(&path).unwrap();
            assert!(brain_lance::fingerprint_matches(&again.conn).unwrap());
        }

        #[test]
        fn newer_schema_is_refused() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("argos.db");
            drop(Store::open(&path).unwrap());
            Connection::open(&path)
                .unwrap()
                .execute_batch("PRAGMA user_version=19")
                .unwrap();
            assert!(Store::open(&path).is_err());
        }

        #[test]
        fn embedding_disabled_degrades_to_jaccard() {
            // Unit tests run as if ARGOS_EMBED=0 unless ARGOS_EMBED=1 is set.
            if crate::embed::enabled() {
                eprintln!("skipped: ARGOS_EMBED is enabled");
                return;
            }
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("argos.db");
            let store = Store::open(&path).unwrap();
            assert!(!store.vectors_enabled());
            store
                .add_memory("Prefers markdown documents with source urls", "fact", false, src())
                .unwrap();
            store
                .add_memory("The kettle is in the galley", "fact", false, src())
                .unwrap();
            let hits = store.recall("markdown source documents", 2).unwrap();
            assert_eq!(hits, recall(&store.list_memories().unwrap(), "markdown source documents", 2));
            assert!(!dir.path().join("memory_lancedb").exists(), "no Lance dir without embedding");
            assert!(store.reindex_memory_vectors().is_err());
        }

        #[test]
        fn embedding_failure_mid_session_degrades_to_jaccard() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("argos.db");
            let store = {
                let _fake = testing::fake();
                let store = Store::open(&path).unwrap();
                store
                    .add_memory("Harbor tanker manifests", "fact", false, src())
                    .unwrap();
                store
            };
            // Embedder now unavailable: recall must still answer from SQLite.
            let hits = store.recall("harbor manifests", 3).unwrap();
            assert_eq!(hits.len(), 1);
            if crate::embed::enabled() {
                return;
            }
            assert_eq!(hits[0].score, recall(&store.list_memories().unwrap(), "harbor manifests", 3)[0].score);
        }

        #[test]
        fn reindex_creates_lance_dir_and_hooks_track_writes() {
            let _fake = testing::fake();
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("argos.db");
            let store = Store::open(&path).unwrap();
            let lance = dir.path().join("memory_lancedb");
            assert!(!lance.exists(), "opening a store does not touch Lance");
            let a = store.add_memory("Harbor tanker manifests list cargo", "fact", false, src()).unwrap();
            let b = store.add_memory("Night desk shift starts at nine", "fact", false, src()).unwrap();
            let report = store.reindex_memory_vectors().unwrap();
            assert_eq!(report.memories, 2);
            assert_eq!(report.lance_dir, lance);
            assert!(lance.join(format!("{}.lance", brain_lance::TABLE)).is_dir());
            assert!(brain_lance::fingerprint_matches(&store.conn).unwrap());
            let index = BrainIndex::shared(&lance);
            let mut ids = index.ids().unwrap();
            ids.sort();
            let mut want = vec![a.id.clone(), b.id.clone()];
            want.sort();
            assert_eq!(ids, want);

            // Delete hook.
            assert!(store.delete_memory(&b.id).unwrap());
            assert_eq!(index.ids().unwrap(), vec![a.id.clone()]);
            // Write hook through a reopened store (shared index handle).
            let reopened = Store::open(&path).unwrap();
            let c = reopened.add_memory("Galley kettle inventory", "fact", false, src()).unwrap();
            assert_eq!(index.count().unwrap(), 2);
            // Update hook re-embeds the new text.
            reopened.update_memory(&c.id, "Northwind ferry timetable", "fact", false).unwrap();
            let hit = index.search("northwind ferry timetable", 1).unwrap();
            assert_eq!(hit[0].0, c.id);
            assert!(hit[0].1 > 0.99);
            assert_eq!(
                reopened.find_similar_memory("Northwind ferry timetable", brain_lance::DUPLICATE_THRESHOLD).map(|h| h.0),
                Some(c.id.clone())
            );
            // Hybrid recall through the Store.
            let hits = reopened.recall("ferry timetable", 3).unwrap();
            assert_eq!(hits[0].memory.id, c.id);
        }

        #[test]
        fn fingerprint_mismatch_rebuilds_and_drift_is_reconciled() {
            let _fake = testing::fake();
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("argos.db");
            let store = Store::open(&path).unwrap();
            let a = store.add_memory("Harbor tanker manifests", "fact", false, src()).unwrap();
            let lance = dir.path().join("memory_lancedb");
            let index = BrainIndex::shared(&lance);
            assert_eq!(index.ids().unwrap(), vec![a.id.clone()], "first write builds the index");
            // Simulate a model change plus a stray vector.
            index
                .upsert_vectors(&[("ghost".into(), crate::embed::embed_one("ghost").unwrap())])
                .unwrap();
            store
                .conn
                .execute("UPDATE memory_embed_meta SET value='old-model'", [])
                .unwrap();
            index.mark_stale();
            store.ensure_memory_vectors().unwrap();
            assert_eq!(index.ids().unwrap(), vec![a.id.clone()], "rebuilt from SQLite");
            assert!(brain_lance::fingerprint_matches(&store.conn).unwrap());
            // Drift with a matching fingerprint: a memory written behind the index's back.
            store
                .conn
                .execute(
                    "INSERT INTO memories(id,text,category,pinned,created_at,source_json) VALUES ('raw','Ferry timetable','fact',0,'now','{}')",
                    [],
                )
                .unwrap();
            index.mark_stale();
            store.ensure_memory_vectors().unwrap();
            assert_eq!(index.count().unwrap(), 2);
        }

        /// Recon calls the sync `Store::recall` from inside its async turn.
        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn recall_works_inside_a_multi_thread_runtime() {
            let _fake = testing::fake();
            let dir = tempfile::tempdir().unwrap();
            let store = Store::open(&dir.path().join("argos.db")).unwrap();
            let m = store.add_memory("Harbor tanker manifests", "fact", false, src()).unwrap();
            let hits = store.recall("tanker manifests", 3).unwrap();
            assert_eq!(hits[0].memory.id, m.id);
            assert!(store.vectors.as_ref().unwrap().is_ready());
        }

        #[tokio::test(flavor = "current_thread")]
        async fn recall_works_inside_a_current_thread_runtime() {
            let _fake = testing::fake();
            let dir = tempfile::tempdir().unwrap();
            let store = Store::open(&dir.path().join("argos.db")).unwrap();
            let m = store.add_memory("Harbor tanker manifests", "fact", false, src()).unwrap();
            assert_eq!(store.recall("tanker manifests", 3).unwrap()[0].memory.id, m.id);
            assert!(store.vectors.as_ref().unwrap().is_ready());
        }

        /// Real MiniLM + LanceDB. Downloads the model on first run:
        /// `ARGOS_EMBED=1 cargo test -p argos-osint-core -- --ignored minilm`
        #[test]
        #[ignore = "downloads all-MiniLM-L6-v2; set ARGOS_EMBED=1 and pass --ignored"]
        fn minilm_lance_upsert_then_search_returns_same_id() {
            if !crate::embed::enabled() {
                eprintln!("skipped: ARGOS_EMBED is not enabled");
                return;
            }
            let dir = tempfile::tempdir().unwrap();
            let index = BrainIndex::new(&dir.path().join("memory_lancedb"));
            index.upsert("m-ship", "The vessel docked at the harbor at dawn").unwrap();
            index.upsert("m-tax", "Quarterly tax filings are due in April").unwrap();
            let hits = index.search("ship arrived in port this morning", 2).unwrap();
            assert_eq!(hits[0].0, "m-ship", "{hits:?}");
            assert!(hits[0].1 > hits[1].1);
            assert_eq!(
                index.find_similar("The vessel docked at the harbor at dawn", brain_lance::DUPLICATE_THRESHOLD).unwrap().map(|h| h.0),
                Some("m-ship".to_string())
            );

            // Through the Store: a paraphrase with no shared words is recalled.
            let store = Store::open(&dir.path().join("store").join("argos.db")).unwrap();
            let ship = store.add_memory("The vessel docked at the harbor at dawn", "fact", false, src()).unwrap();
            store.add_memory("Quarterly tax filings are due in April", "fact", false, src()).unwrap();
            let hits = store.recall("ship arrived in port this morning", 3).unwrap();
            assert_eq!(hits[0].memory.id, ship.id, "{hits:?}");
            assert!(
                recall(&store.list_memories().unwrap(), "ship arrived in port this morning", 3)
                    .iter()
                    .all(|hit| hit.memory.id != ship.id),
                "Jaccard alone cannot find the paraphrase"
            );
        }
    }

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
            .save_graph_summary(
                &memory.id,
                "The recon path ends at the launch window.",
                "d1"
            )
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
        assert_eq!(version, SCHEMA_VERSION);
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

    fn article(run_id: &str, id: &str) -> AtlasArticleRow {
        AtlasArticleRow {
            run_id: run_id.into(),
            id: id.into(),
            title: "Headline".into(),
            description: String::new(),
            url: "https://example.com".into(),
            country: "us".into(),
            source_name: "Wire".into(),
            source_domain: "example.com".into(),
            published_at: "2000-01-01T00:00:00+00:00".into(),
            provider: "newsapi".into(),
            temperature: 1.0,
            category: "unk".into(),
            seen_at: "2000-01-01T00:00:00+00:00".into(),
            author: String::new(),
            image_url: String::new(),
        }
    }

    #[test]
    fn atlas_auto_next_round_trips_through_app_state() {
        let store = Store::memory().unwrap();
        assert!(store.atlas_auto_next().unwrap().is_none());
        store.set_atlas_auto_next(Some(1_700_000_000)).unwrap();
        assert_eq!(store.atlas_auto_next().unwrap(), Some(1_700_000_000));
        store.set_atlas_auto_next(None).unwrap();
        assert!(store.atlas_auto_next().unwrap().is_none());
    }

    #[test]
    fn atlas_prune_drops_finished_runs_older_than_the_cutoff() {
        let store = Store::memory().unwrap();
        store
            .atlas_insert_run("old", "{}", r#"{"scored":true}"#)
            .unwrap();
        store.atlas_insert_run("fresh", "{}", "{}").unwrap();
        store.atlas_insert_run("held", "{}", "{}").unwrap();
        store.atlas_insert_run("active", "{}", "{}").unwrap();
        store
            .conn
            .execute(
                "UPDATE atlas_runs SET state='completed', started_at=?1 WHERE id='old'",
                ["2000-01-01T00:00:00+00:00"],
            )
            .unwrap();
        store
            .conn
            .execute(
                "UPDATE atlas_runs SET state='completed' WHERE id='fresh'",
                [],
            )
            .unwrap();
        store
            .conn
            .execute(
                "UPDATE atlas_runs SET state='paused', started_at=?1 WHERE id='held'",
                ["2000-01-01T00:00:00+00:00"],
            )
            .unwrap();
        store
            .conn
            .execute(
                "UPDATE atlas_runs SET started_at=?1 WHERE id='active'",
                ["2000-01-01T00:00:00+00:00"],
            )
            .unwrap();
        store.atlas_upsert_article(&article("old", "a1")).unwrap();
        store.atlas_upsert_article(&article("fresh", "a2")).unwrap();
        store.atlas_upsert_article(&article("held", "a3")).unwrap();
        store.atlas_quota_bump("newsapi", "2000-01-01").unwrap();
        let mut removed = store.atlas_prune_expired().unwrap();
        removed.sort();
        assert_eq!(removed, vec!["old".to_string()]);
        assert!(store.atlas_list_articles("old").unwrap().is_empty());
        assert_eq!(store.atlas_list_articles("fresh").unwrap().len(), 1);
        assert_eq!(store.atlas_list_articles("held").unwrap().len(), 1);
        let mut ids: Vec<_> = store
            .atlas_list_runs()
            .unwrap()
            .into_iter()
            .map(|run| run.id)
            .collect();
        ids.sort();
        assert_eq!(
            ids,
            vec!["active".to_string(), "fresh".into(), "held".into()]
        );
        assert_eq!(store.atlas_quota_used("newsapi", "2000-01-01").unwrap(), 1);
    }

    #[test]
    fn atlas_run_days_lists_distinct_days_newest_first() {
        let store = Store::memory().unwrap();
        store.atlas_insert_run("r1", "{}", "{}").unwrap();
        store.atlas_insert_run("r2", "{}", "{}").unwrap();
        store.atlas_insert_run("r3", "{}", "{}").unwrap();
        store
            .conn
            .execute(
                "UPDATE atlas_runs SET started_at=?1 WHERE id='r1'",
                ["2026-10-01T12:00:00+00:00"],
            )
            .unwrap();
        store
            .conn
            .execute(
                "UPDATE atlas_runs SET started_at=?1 WHERE id='r2'",
                ["2026-10-03T08:00:00+00:00"],
            )
            .unwrap();
        store
            .conn
            .execute(
                "UPDATE atlas_runs SET started_at=?1 WHERE id='r3'",
                ["2026-10-03T18:00:00+00:00"],
            )
            .unwrap();
        assert_eq!(
            store.atlas_run_days().unwrap(),
            vec!["2026-10-03".to_string(), "2026-10-01".into()]
        );
    }

    #[test]
    fn atlas_recent_articles_lists_retained_rows() {
        let store = Store::memory().unwrap();
        store.atlas_insert_run("r1", "{}", "{}").unwrap();
        store.atlas_upsert_article(&article("r1", "a1")).unwrap();
        let rows = store.atlas_recent_articles().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "a1");
    }

    #[test]
    fn atlas_articles_for_intel_dedupes_same_article_id_across_runs() {
        let store = Store::memory().unwrap();
        store.atlas_insert_run("day-a", "{}", "{}").unwrap();
        store.atlas_insert_run("day-a2", "{}", "{}").unwrap();
        store
            .conn
            .execute(
                "UPDATE atlas_runs SET started_at=?1 WHERE id IN ('day-a','day-a2')",
                ["2026-10-04T10:00:00+00:00"],
            )
            .unwrap();
        let mut first = article("day-a", "shared");
        first.category = "geopolitical".into();
        first.title = "Border talks stall".into();
        first.published_at = "2026-10-04T11:00:00+00:00".into();
        let mut second = article("day-a2", "shared");
        second.category = "geopolitical".into();
        second.title = "Border talks stall".into();
        second.published_at = "2026-10-04T12:00:00+00:00".into();
        second.description = "newer copy".into();
        store.atlas_upsert_article(&first).unwrap();
        store.atlas_upsert_article(&second).unwrap();
        let rows = store
            .atlas_articles_for_intel("geopolitical", "2026-10-04", "")
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].run_id, "day-a2");
        assert_eq!(rows[0].description, "newer copy");
    }

    #[test]
    fn atlas_articles_for_intel_filters_category_day_and_query() {
        let store = Store::memory().unwrap();
        store.atlas_insert_run("day-a", "{}", "{}").unwrap();
        store.atlas_insert_run("day-b", "{}", "{}").unwrap();
        store
            .conn
            .execute(
                "UPDATE atlas_runs SET started_at=?1 WHERE id='day-a'",
                ["2026-10-04T10:00:00+00:00"],
            )
            .unwrap();
        store
            .conn
            .execute(
                "UPDATE atlas_runs SET started_at=?1 WHERE id='day-b'",
                ["2026-10-03T10:00:00+00:00"],
            )
            .unwrap();
        let mut geo = article("day-a", "g1");
        geo.category = "geopolitical".into();
        geo.title = "Border talks stall".into();
        geo.description = "Diplomats meet in Geneva".into();
        geo.published_at = "2026-10-04T12:00:00+00:00".into();
        let mut econ = article("day-a", "e1");
        econ.category = "economic".into();
        econ.title = "Oil markets jump".into();
        econ.published_at = "2026-10-04T13:00:00+00:00".into();
        let mut older = article("day-b", "g2");
        older.category = "geopolitical".into();
        older.title = "Border talks resume".into();
        older.published_at = "2026-10-03T12:00:00+00:00".into();
        store.atlas_upsert_article(&geo).unwrap();
        store.atlas_upsert_article(&econ).unwrap();
        store.atlas_upsert_article(&older).unwrap();

        let day_a = store
            .atlas_articles_for_intel("geopolitical", "2026-10-04", "")
            .unwrap();
        assert_eq!(day_a.len(), 1);
        assert_eq!(day_a[0].id, "g1");

        let searched = store
            .atlas_articles_for_intel("geopolitical", "2026-10-04", "geneva")
            .unwrap();
        assert_eq!(searched.len(), 1);
        assert_eq!(searched[0].id, "g1");

        let miss = store
            .atlas_articles_for_intel("geopolitical", "2026-10-04", "oil")
            .unwrap();
        assert!(miss.is_empty());
    }

    #[test]
    fn atlas_claims_for_article_returns_linked_claims() {
        let store = Store::memory().unwrap();
        store.atlas_insert_run("run-1", "{}", "{}").unwrap();
        let claim = AtlasInsightClaim {
            fingerprint: String::new(),
            entity: "geneva".into(),
            namespace: "place".into(),
            predicate: "hosts".into(),
            object: "talks".into(),
            topic: "geopolitical".into(),
            claim: "Geneva hosts border talks.".into(),
            classification: "fact".into(),
            confidence: 0.91,
            article_id: "art-1".into(),
            source_url: "https://example.com/a".into(),
            published_at: "2026-10-04T12:00:00+00:00".into(),
            reliability: "B".into(),
            info_credibility: 2,
            admiralty: "B2".into(),
            rsp_status: "gr".into(),
        };
        store
            .persist_atlas_insights("run-1", &[claim], &[], "brief", "")
            .unwrap();
        let rows = store.atlas_claims_for_article("run-1", "art-1").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].entity, "geneva");
        assert!((rows[0].confidence - 0.91).abs() < f64::EPSILON);
        assert_eq!(rows[0].claim, "Geneva hosts border talks.");
        assert_eq!(rows[0].admiralty, "B2");
        assert_eq!(rows[0].reliability, "B");
        assert_eq!(rows[0].info_credibility, 2);
        assert!(store
            .atlas_claims_for_article("run-1", "missing")
            .unwrap()
            .is_empty());
    }

    #[test]
    fn delete_article_insights_orphans_brain_and_keeps_shared() {
        let store = Store::memory().unwrap();
        store.atlas_insert_run("run-1", "{}", "{}").unwrap();
        let shared = AtlasInsightClaim {
            fingerprint: String::new(),
            entity: "fleet".into(),
            namespace: "org".into(),
            predicate: "owned_by".into(),
            object: "acme".into(),
            topic: "geopolitical".into(),
            claim: "Fleet owned by Acme.".into(),
            classification: "fact".into(),
            confidence: 0.8,
            article_id: "art-1".into(),
            source_url: "https://example.com/a".into(),
            published_at: "2026-10-04T12:00:00+00:00".into(),
            reliability: "B".into(),
            info_credibility: 2,
            admiralty: "B2".into(),
            rsp_status: "gr".into(),
        };
        let only = AtlasInsightClaim {
            fingerprint: String::new(),
            entity: "acme".into(),
            namespace: "org".into(),
            predicate: "sanctioned".into(),
            object: "eu".into(),
            topic: "geopolitical".into(),
            claim: "Acme sanctioned by EU.".into(),
            classification: "fact".into(),
            confidence: 0.7,
            article_id: "art-1".into(),
            source_url: "https://example.com/a".into(),
            published_at: "2026-10-04T12:00:00+00:00".into(),
            reliability: "B".into(),
            info_credibility: 2,
            admiralty: "B2".into(),
            rsp_status: "gr".into(),
        };
        store
            .persist_atlas_insights("run-1", &[shared.clone(), only], &[], "", "")
            .unwrap();
        // Second article also sources the shared claim fingerprint.
        let mut shared_peer = shared;
        shared_peer.article_id = "art-2".into();
        shared_peer.source_url = "https://example.com/b".into();
        store
            .persist_atlas_insights("run-1", &[shared_peer], &[], "", "")
            .unwrap();
        let before = store.list_memories().unwrap().len();
        let orphaned = store.delete_article_insights("run-1", "art-1").unwrap();
        assert_eq!(orphaned, 1);
        assert!(store
            .atlas_claims_for_article("run-1", "art-1")
            .unwrap()
            .is_empty());
        let shared_left = store.atlas_claims_for_article("run-1", "art-2").unwrap();
        assert_eq!(shared_left.len(), 1);
        assert_eq!(shared_left[0].entity, "fleet");
        assert_eq!(store.list_memories().unwrap().len(), before - 1);
    }
}
