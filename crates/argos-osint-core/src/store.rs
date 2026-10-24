//! Persistent Brain memories and their conversation provenance.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result};

use crate::brain::{Memory, MemorySource, ScoredMemory};
use crate::profile_stats;

mod publication;

pub use publication::{
    memory_revision, payload_revision, retag_source, CoverageReport, MemoriesChanged,
    MemoryChangeWatcher, PublicationReceipt, PublicationVerification, PublishOptions,
    RejectedClaim, DELETED_REVISION, MEMORY_RECORD,
};

#[cfg(feature = "surreal")]
pub(crate) use publication::reject_reason;

static IDS: AtomicU64 = AtomicU64::new(1);

/// Memory id: time, a random per-process salt and a sequence, so two
/// processes writing one database in the same millisecond cannot collide.
pub fn new_id() -> String {
    static SALT: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    let salt = *SALT.get_or_init(|| {
        use std::hash::{BuildHasher, Hasher};
        // RandomState is seeded from the OS RNG; no extra dependency.
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u32(std::process::id());
        h.finish()
    });
    let n = IDS.fetch_add(1, Ordering::Relaxed);
    format!(
        "mem-{}-{:012x}-{n}",
        chrono::Utc::now().timestamp_millis(),
        salt & 0xffff_ffff_ffff
    )
}

/// A saved explanation of one memory, keyed by the directive it was written from.
pub struct GraphSummary {
    pub summary: String,
    /// Directive id. Empty for a summary written before the path was narrowed.
    pub focus: String,
}

/// Schema version this build writes. 26 adds model operations/attempts,
/// coverage ledger, and intel task lease epoch. 27 adds the telemetry and
/// activity-history tables plus additive timing columns on the model attempt
/// and tool call records. 28 adds immutable NLP artifacts/references/features and
/// complete summary-result envelopes; existing facts are preserved unchanged.
/// 37 adds the unified Brain graph (typed kinds, actors, endpoints) and brief
/// stage timings; `atomic_claim` memories migrate to claim/inference in place.
/// 43 drops the unused `argos_graph_explanations` table (the graph explanation
/// feature was removed); `memory_graph_summaries` is left alone.
pub const SCHEMA_VERSION: i64 = 43;

/// Soft hint only: sync rebuild above this size is skipped in favor of an
/// asynchronous `argos_index_changes` rebuild enqueue (no manual reindex required).
pub const AUTO_REBUILD_HINT: usize = 64;

/// Telemetry and activity-history tables, applied once at schema 27 and reused
/// by the in-memory stores used in tests.
pub const SCHEMA_TELEMETRY_SQL: &str = include_str!("schema_telemetry.sql");

/// Handle onto the process SurrealDB store (`storage::runtime`). Carries no
/// connection of its own; every method goes through `storage::{brain_store,
/// store_bridge}`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Store {}

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

/// Re-rank memory hits using passage hybrid scores for long texts.
fn boost_with_passage_hybrid(
    query: &str,
    memories: &[Memory],
    mut ranked: Vec<ScoredMemory>,
    top_k: usize,
) -> Vec<ScoredMemory> {
    use crate::evidence::{chunk_text, hybrid_passage_candidates, RecordKind};
    const LONG: usize = 360;
    let mut passages = Vec::new();
    for memory in memories {
        if memory.text.chars().count() < LONG {
            continue;
        }
        passages.extend(chunk_text(
            &memory.id,
            &memory.created_at,
            RecordKind::Memory,
            &memory.text,
            280,
            40,
        ));
    }
    if passages.is_empty() {
        ranked.truncate(top_k);
        return ranked;
    }
    let hits = hybrid_passage_candidates(query, &passages, &[], (top_k * 3).max(8));
    if hits.is_empty() {
        ranked.truncate(top_k);
        return ranked;
    }
    let mut bonus: std::collections::HashMap<String, f32> = std::collections::HashMap::new();
    for hit in hits {
        // passage id is `{memory_id}:p{n}`
        let mem_id = hit
            .passage_id
            .rsplit_once(":p")
            .map(|(id, _)| id.to_string())
            .unwrap_or_else(|| hit.passage_id.clone());
        let slot = bonus.entry(mem_id).or_insert(0.0);
        *slot = slot.max(hit.score * 0.15);
    }
    for item in &mut ranked {
        if let Some(b) = bonus.get(&item.memory.id) {
            item.score += *b;
        }
    }
    // Also surface long memories that only matched via passages.
    let have: std::collections::HashSet<String> =
        ranked.iter().map(|h| h.memory.id.clone()).collect();
    for memory in memories {
        if have.contains(&memory.id) {
            continue;
        }
        if let Some(b) = bonus.get(&memory.id) {
            if *b > 0.0 {
                ranked.push(ScoredMemory {
                    memory: memory.clone(),
                    score: *b,
                });
            }
        }
    }
    ranked.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    ranked.truncate(top_k);
    ranked
}

impl Store {
    /// TUI control handle. Every method dispatches to the process SurrealDB
    /// runtime, so this only makes sure the state folder exists.
    pub fn open_ui(path: &Path) -> Result<Self> {
        Self::open(path)
    }

    /// Typed Brain insight kind (`claim` = source claim, `inference`) from the
    /// canonical derivation row, newest first; `None` for untyped memories.
    pub fn memory_insight_kind(&self, memory_id: &str) -> Option<String> {
        crate::storage::store_bridge::memory_insight_kind(memory_id)
    }

    /// Analytics reader handle (SurrealDB-backed; see [`Store::open`]).
    pub fn open_profile_reader(path: &Path) -> Result<Self> {
        let _ = path;
        Ok(Self {})
    }

    /// Lightweight handle for lease heartbeats (SurrealDB-backed).
    pub fn open_light(path: &Path) -> Result<Self> {
        let _ = path;
        Ok(Self {})
    }

    /// Worker/cache handle (SurrealDB-backed; see [`Store::open`]).
    pub fn open_initialized_light(path: &Path) -> Result<Self> {
        Self::open(path)
    }

    /// A handle onto the process SurrealDB store. `path` is the legacy
    /// `argos.db` location; only its folder is created, no file is opened.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create {}", parent.display()))?;
        }
        Ok(Self {})
    }

    /// Index-pool handle; vectors live in SurrealDB and the worker drains
    /// them through `storage::store_bridge`.
    pub fn open_index_worker(path: &Path) -> Result<Self> {
        Self::open(path)
    }

    /// A handle with no state folder, for tests and tools.
    pub fn memory() -> Result<Self> {
        Ok(Self {})
    }

    pub fn list_memories(&self) -> Result<Vec<Memory>> {
        crate::storage::brain_store::list_memories()
    }

    /// Substring search across memory text, category, claim fields, and source JSON.
    pub fn search_memories(&self, query: &str) -> Result<Vec<Memory>> {
        crate::storage::brain_store::search_memories(query)
    }

    pub fn add_memory(
        &self,
        text: &str,
        category: &str,
        pinned: bool,
        source: MemorySource,
    ) -> Result<Memory> {
        crate::storage::brain_store::add_memory(text, category, pinned, source)
    }

    /// Recall: Jaccard on SQLite (SurrealDB serves vector recall),
    /// then passage-level [`crate::evidence::hybrid_passage_candidates`] for long
    /// memories so Brain/Recon share the same hybrid path (spec follow-up).
    /// Falls back to Jaccard alone when embedding is off or the index fails; an
    /// index problem never fails the caller.
    pub fn recall(&self, query: &str, top_k: usize) -> Result<Vec<ScoredMemory>> {
        crate::storage::brain_store::recall(query, top_k)
    }

    /// True when recall will use the vector index (embedding on, file-backed store).
    pub fn vectors_enabled(&self) -> bool {
        crate::embed::enabled()
    }

    /// The closest existing memory to `text` at or above `threshold` cosine
    /// similarity (see `storage::memories` for the duplicate threshold).
    /// None when embedding is off or the index is unavailable.
    pub fn find_similar_memory(&self, text: &str, threshold: f32) -> Option<(String, f32)> {
        crate::storage::brain_store::find_similar_memory(text, threshold)
    }

    /// Brings the Lance index in line with SQLite. Small missing/mismatched indexes
    /// rebuild inline; larger ones enqueue an automatic generation rebuild via
    /// `argos_index_changes` so foreground recall is never blocked on a 3,000-row
    /// sync barrier. Incremental id reconciliation still runs when the serving
    /// generation is compatible.
    pub fn ensure_memory_vectors(&self) -> Result<()> {
        crate::storage::store_bridge::ensure_memory_vectors()
    }

    /// Count user-edited insight memories linked to one Atlas article.
    pub fn article_insight_user_edit_count(&self, run_id: &str, article_id: &str) -> Result<usize> {
        crate::storage::store_bridge::article_insight_user_edit_count(run_id, article_id)
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
        crate::storage::store_bridge::commit_article_insight_replacement(
            run_id, article_id, claims, relations,
        )
    }

    /// Drops and rebuilds the Lance table from every memory, then writes the
    /// fingerprint. Backs `argos memories reindex`.
    pub fn reindex_memory_vectors(&self) -> Result<ReindexReport> {
        crate::storage::store_bridge::reindex_memory_vectors()
    }

    /// Drain pending generation rebuild work in bounded batches (off the TUI path).
    pub fn process_pending_vector_rebuild(&self, max_batches: usize) -> Result<usize> {
        crate::storage::store_bridge::process_pending_vector_rebuild(max_batches)
    }

    /// Typed index upsert used by the reliable (outbox) path. Distinguishes
    /// ready, pending, disabled and retryable/permanent failures; a missing
    /// memory row means the memory was deleted, so its vector is removed instead
    /// of resurrected.
    pub fn try_index_upsert(&self, ids: &[String]) -> crate::tasks::IndexOutcome {
        crate::storage::store_bridge::try_index_upsert(ids)
    }

    /// Typed removal of vectors whose memories are gone from SQLite.
    pub fn try_index_remove_missing(&self, ids: &[String]) -> crate::tasks::IndexOutcome {
        crate::storage::store_bridge::try_index_remove_missing(ids)
    }

    /// Drain generation rebuild batches with a typed outcome: a batch is
    /// progress (`Pending`), only an activated (or absent) generation is `Ready`.
    pub fn try_process_vector_rebuild(&self, max_batches: usize) -> crate::tasks::IndexOutcome {
        crate::storage::store_bridge::try_process_vector_rebuild(max_batches)
    }

    pub fn update_memory(
        &self,
        id: &str,
        text: &str,
        category: &str,
        pinned: bool,
    ) -> Result<bool> {
        crate::storage::brain_store::update_memory(id, text, category, pinned)
    }

    pub fn graph_summary(&self, memory_id: &str) -> Result<Option<GraphSummary>> {
        crate::storage::brain_store::graph_summary(memory_id)
    }

    /// Stores the synthesis explanation for one memory's recon path.
    /// `focus` is the directive id the summary was written from.
    /// Returns false when that memory is already gone.
    pub fn save_graph_summary(&self, memory_id: &str, summary: &str, focus: &str) -> Result<bool> {
        crate::storage::brain_store::save_graph_summary(memory_id, summary, focus)
    }

    /// User deletion. Leaves a tombstone so retries/repair never recreate the
    /// memory, and queues durable vector removal in the same transaction.
    pub fn delete_memory(&self, id: &str) -> Result<bool> {
        crate::storage::brain_store::delete_memory(id)
    }

    /// Fetch one memory by id directly (independent of any list filter).
    pub fn get_memory(&self, id: &str) -> Result<Option<Memory>> {
        crate::storage::brain_store::get_memory(id)
    }

    pub fn atlas_insert_run(&self, id: &str, cursor_json: &str, stats_json: &str) -> Result<()> {
        crate::storage::store_bridge::atlas_insert_run(id, cursor_json, stats_json)
    }

    pub fn atlas_save(&self, id: &str, cursor_json: &str, stats_json: &str) -> Result<()> {
        crate::storage::store_bridge::atlas_save(id, cursor_json, stats_json)
    }

    pub fn atlas_save_unit_manifest(
        &self,
        manifest: &crate::atlas_work::UnitManifest,
    ) -> Result<()> {
        crate::storage::store_bridge::atlas_save_unit_manifest(manifest)
    }

    pub fn atlas_get_unit_manifests(
        &self,
        run_id: &str,
        stage: i32,
    ) -> Result<Vec<crate::atlas_work::UnitManifest>> {
        crate::storage::store_bridge::atlas_get_unit_manifests(run_id, stage)
    }

    pub fn atlas_get_all_unit_manifests(
        &self,
        run_id: &str,
    ) -> Result<Vec<crate::atlas_work::UnitManifest>> {
        crate::storage::store_bridge::atlas_get_all_unit_manifests(run_id)
    }
    pub fn atlas_set_state(&self, id: &str, state: &str, note: &str, finished: bool) -> Result<()> {
        crate::storage::store_bridge::atlas_set_state(id, state, note, finished)
    }

    pub fn atlas_latest_run(&self) -> Result<Option<AtlasRunRow>> {
        crate::storage::store_bridge::atlas_latest_run()
    }

    pub fn atlas_list_runs(&self) -> Result<Vec<AtlasRunRow>> {
        crate::storage::store_bridge::atlas_list_runs()
    }

    /// One dashboard snapshot over this store.
    pub fn profile_snapshot(
        &self,
        period: &profile_stats::Period,
        filters: &profile_stats::StatFilters,
    ) -> anyhow::Result<profile_stats::ProfileSnapshot> {
        crate::storage::store_bridge::profile_snapshot(period, filters)
    }

    /// The bounded filter option lists for the Profile filter strip.
    pub fn profile_filter_options(&self) -> Vec<(String, Vec<String>)> {
        crate::storage::store_bridge::profile_filter_options()
    }

    pub fn atlas_delete_run(&self, id: &str) -> Result<bool> {
        crate::storage::store_bridge::atlas_delete_run(id)
    }

    /// Drops insight source rows for one Atlas article and orphans Brain memories that
    /// no longer have any source. Fingerprints still linked from other articles/cycles keep
    /// their claim and memory.
    pub fn delete_article_insights(&self, run_id: &str, article_id: &str) -> Result<usize> {
        crate::storage::store_bridge::delete_article_insights(run_id, article_id)
    }

    /// Drops Brain memories written from one Atlas cycle.
    /// A claim that another cycle also sourced keeps its memory and loses only this cycle's source row.
    pub fn delete_cycle_memories(&self, run_id: &str) -> Result<()> {
        crate::storage::store_bridge::delete_cycle_memories(run_id)
    }

    pub fn atlas_upsert_article(&self, row: &AtlasArticleRow) -> Result<()> {
        crate::storage::store_bridge::atlas_upsert_article(row)
    }

    pub fn atlas_delete_article(&self, run_id: &str, article_id: &str) -> Result<()> {
        crate::storage::store_bridge::atlas_delete_article(run_id, article_id)
    }

    pub fn atlas_set_category(&self, run_id: &str, article_id: &str, category: &str) -> Result<()> {
        crate::storage::store_bridge::atlas_set_category(run_id, article_id, category)
    }

    pub fn atlas_list_articles(&self, run_id: &str) -> Result<Vec<AtlasArticleRow>> {
        crate::storage::store_bridge::atlas_list_articles(run_id)
    }

    /// Every stored article still within the retention window (prune drops older runs).
    /// Used to seed Atlas dedup so a new cycle skips headlines already processed.
    pub fn atlas_recent_articles(&self) -> Result<Vec<AtlasArticleRow>> {
        crate::storage::store_bridge::atlas_recent_articles()
    }

    pub fn atlas_article(&self, run_id: &str, article_id: &str) -> Result<Option<AtlasArticleRow>> {
        crate::storage::store_bridge::atlas_article(run_id, article_id)
    }

    /// Latest retained occurrence of one article, without decoding the whole
    /// retained corpus just to open a headline.
    pub fn atlas_latest_article_by_id(&self, article_id: &str) -> Result<Option<AtlasArticleRow>> {
        crate::storage::store_bridge::atlas_latest_article_by_id(article_id)
    }

    /// Distinct UTC calendar days (`YYYY-MM-DD`) that have an Atlas run, newest first.
    pub fn atlas_run_days(&self) -> Result<Vec<String>> {
        crate::storage::store_bridge::atlas_run_days()
    }

    /// Articles for the Intel bulletin: one OSINT category, one run day, optional search.
    pub fn atlas_articles_for_intel(
        &self,
        category: &str,
        day: &str,
        query: &str,
    ) -> Result<Vec<AtlasArticleRow>> {
        crate::storage::store_bridge::atlas_articles_for_intel(category, day, query)
    }

    /// Claims linked to one Atlas article (`insight_sources.call_id` stores article_id).
    pub fn atlas_claims_for_article(
        &self,
        run_id: &str,
        article_id: &str,
    ) -> Result<Vec<AtlasArticleClaim>> {
        crate::storage::store_bridge::atlas_claims_for_article(run_id, article_id)
    }

    /// Persists view-only link explanations for an article on the Intel Brief page.
    pub fn save_intel_link_explanations(
        &self,
        article_id: &str,
        explanations: &HashMap<(String, String), String>,
    ) -> Result<()> {
        crate::storage::store_bridge::save_intel_link_explanations(article_id, explanations)
    }

    /// Fetches persisted view-only link explanations for an article on the Intel Brief page.
    pub fn get_intel_link_explanations(
        &self,
        article_id: &str,
    ) -> Result<HashMap<(String, String), String>> {
        crate::storage::store_bridge::get_intel_link_explanations(article_id)
    }

    /// Deletes persisted link explanations for an article.
    pub fn delete_intel_link_explanations(&self, article_id: &str) -> Result<()> {
        crate::storage::store_bridge::delete_intel_link_explanations(article_id)
    }

    /// A run left `running` by a closed process can be resumed.
    pub fn atlas_park_running(&self) -> Result<()> {
        crate::storage::store_bridge::atlas_park_running()
    }

    pub fn atlas_quota_used(&self, provider: &str, day: &str) -> Result<u32> {
        crate::storage::store_bridge::atlas_quota_used(provider, day)
    }

    pub fn atlas_quota_bump(&self, provider: &str, day: &str) -> Result<u32> {
        crate::storage::store_bridge::atlas_quota_bump(provider, day)
    }

    pub fn app_state_get(&self, key: &str) -> Result<Option<String>> {
        crate::storage::store_bridge::app_state_get(key)
    }

    pub fn app_state_set(&self, key: &str, value: &str) -> Result<()> {
        crate::storage::store_bridge::app_state_set(key, value)
    }

    /// Unix time of the next automatic Atlas run. Absent means auto run is off.
    pub fn atlas_auto_next(&self) -> Result<Option<u64>> {
        crate::storage::store_bridge::atlas_auto_next()
    }

    pub fn set_atlas_auto_next(&self, when: Option<u64>) -> Result<()> {
        crate::storage::store_bridge::set_atlas_auto_next(when)
    }

    /// Drops finished runs that started more than 36 hours ago, including their
    /// statistics and articles. Runs that are still running or paused stay so they can resume.
    pub fn atlas_prune_expired(&self) -> Result<Vec<String>> {
        crate::storage::store_bridge::atlas_prune_expired()
    }

    /// True when this Atlas run already wrote insight sources. A resume skips the model.
    pub fn atlas_has_insights(&self, run_id: &str) -> Result<bool> {
        crate::storage::store_bridge::atlas_has_insights(run_id)
    }

    /// Claims whose sources point at this Atlas run.
    pub fn atlas_stored_claims(&self, run_id: &str) -> Result<Vec<AtlasStoredClaim>> {
        crate::storage::store_bridge::atlas_stored_claims(run_id)
    }

    /// Relations whose endpoints are both in `fingerprints`.
    pub fn insight_relations_among(
        &self,
        fingerprints: &[String],
    ) -> Result<Vec<(String, String, String)>> {
        crate::storage::store_bridge::insight_relations_among(fingerprints)
    }

    /// Cache a deterministic summary and enqueue a background LLM flush task
    /// (summarization 2-attempt policy). Failures are ignored — callers already
    /// have usable deterministic text.
    pub(crate) fn enqueue_summary_flush_best_effort(
        &self,
        mode: crate::summarization::SummarizationMode,
        source_id: &str,
        revision: &str,
        text: &str,
        focus: &str,
        budget_chars: usize,
    ) {
        if text.trim().is_empty() {
            return;
        }
        let req = crate::summarization::flush_request(
            mode,
            source_id,
            revision,
            text,
            focus,
            budget_chars,
        );
        let deterministic = crate::summarization::SummaryResult {
            content: text.to_string(),
            source_refs: vec![source_id.into()],
            source_hash: req
                .sources
                .first()
                .map(|s| s.hash.clone())
                .unwrap_or_default(),
            model: "deterministic".into(),
            prompt_version: mode.prompt_version().into(),
            coverage: crate::summarization::CoverageMeta {
                partial: false,
                omitted: Vec::new(),
                notes: "queued_for_flush".into(),
            },
            source_hashes: Vec::new(),
            provider: "local".into(),
            preprocessing: serde_json::Value::Null,
            fallback: true,
        };
        let _ = crate::summarization::publish_selected(&req, deterministic);
    }

    /// Writes Atlas claims into the same Brain tables Recon reads, through the
    /// transactional publisher ([`Self::publish_atlas_insights`]), and returns its
    /// receipt. `answer_id` is `atlas-{run_id}` and is not a Recon message.
    pub fn persist_atlas_insights(
        &self,
        run_id: &str,
        claims: &[AtlasInsightClaim],
        relations: &[(String, String, String)],
        brief: &str,
        entity_path: &str,
    ) -> Result<PublicationReceipt> {
        let _ = entity_path;
        let receipt = self.publish_atlas_insights(
            run_id,
            claims,
            relations,
            brief,
            &PublishOptions {
                index_now: true,
                ..Default::default()
            },
        )?;
        if receipt.brief_memory_id.is_some() {
            self.enqueue_summary_flush_best_effort(
                crate::summarization::SummarizationMode::AtlasBrief,
                &format!("atlas-brief-{run_id}"),
                run_id,
                brief.trim(),
                "atlas",
                800,
            );
        }
        Ok(receipt)
    }
}

pub(crate) fn atlas_answer_id(run_id: &str) -> String {
    format!("atlas-{run_id}")
}

/// Fingerprint Recon uses: the JSON array of namespace, entity, predicate, and object.
pub fn insight_fingerprint(namespace: &str, entity: &str, predicate: &str, object: &str) -> String {
    serde_json::to_string(&(namespace, entity, predicate, object)).unwrap_or_default()
}

/// Qualified source assertions never merge with a legacy triple or with a
/// differently attributed, dated, negated or modal statement.
pub fn atlas_claim_identity(claim: &AtlasInsightClaim) -> String {
    if claim.fingerprint.starts_with("v2:") {
        return claim.fingerprint.clone();
    }
    if claim.classification != "claim" {
        return insight_fingerprint(
            &claim.namespace.trim().to_lowercase(),
            &claim.entity.trim().to_lowercase(),
            &claim.predicate.trim().to_lowercase(),
            &claim.object.trim().to_lowercase(),
        );
    }
    format!(
        "v2:{}",
        crate::nlp::cache::key(&serde_json::json!([
            "claim",
            claim.article_id,
            claim.namespace,
            claim.entity.to_lowercase(),
            claim.predicate.to_lowercase(),
            claim.object.to_lowercase(),
            claim.claim.split_whitespace().collect::<Vec<_>>().join(" "),
            claim.source_url,
            claim.published_at
        ]))
    )
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
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
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

/// One span-checked claim the Atlas cycle writes into Brain. Serializable so the
/// validated extraction can be checkpointed before publication.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
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
}
