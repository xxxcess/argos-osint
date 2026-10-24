//! Synchronous `Store` Atlas, insight, vector, publication and app-state entry
//! points on SurrealDB (chunk 3, track 1).
//!
//! Same contract as [`super::brain_store`]: when the process selected
//! SurrealDB ([`on`]) the frozen `Store` methods dispatch here instead of the
//! SQLite connection. Every function blocks on [`runtime::blocking`], so the
//! callers must be plain threads, never async code.

use std::collections::HashMap;

use anyhow::{anyhow, Result};
use surrealdb::types::SurrealValue;

use super::insight::atlas_answer_id;
use super::runtime;
use super::tx::TxError;
use crate::atlas_work::UnitManifest;
use crate::store::{
    AtlasArticleClaim, AtlasArticleRow, AtlasInsightClaim, AtlasRunRow, AtlasStoredClaim,
    CoverageReport, PublicationReceipt, PublicationVerification, PublishOptions, ReindexReport,
};
use crate::tasks::{IndexEnqueue, IndexOutcome};

/// Whether the `Store` methods in this module run on SurrealDB.
pub fn on() -> bool {
    super::brain_store::on()
}

fn tx_err(e: TxError) -> anyhow::Error {
    match e {
        TxError::Abort(m) => anyhow!(m),
        e => e.into(),
    }
}

fn s(v: &str) -> String {
    v.to_string()
}

// ---- Atlas runs, manifests and articles ----

pub fn atlas_insert_run(id: &str, cursor_json: &str, stats_json: &str) -> Result<()> {
    let (id, c, st) = (s(id), s(cursor_json), s(stats_json));
    runtime::blocking(|repo| async move {
        super::atlas::insert_run(repo.db(), &id, &c, &st)
            .await
            .map_err(tx_err)
    })
}

pub fn atlas_save(id: &str, cursor_json: &str, stats_json: &str) -> Result<()> {
    let (id, c, st) = (s(id), s(cursor_json), s(stats_json));
    runtime::blocking(|repo| async move {
        super::atlas::save(repo.db(), &id, &c, &st)
            .await
            .map_err(tx_err)
    })
}

pub fn atlas_save_unit_manifest(manifest: &UnitManifest) -> Result<()> {
    let manifest = manifest.clone();
    runtime::blocking(|repo| async move {
        super::atlas_work::save_unit_manifest(repo.db(), &manifest).await
    })
}

pub fn atlas_get_unit_manifests(run_id: &str, stage: i32) -> Result<Vec<UnitManifest>> {
    let run_id = s(run_id);
    runtime::blocking(|repo| async move {
        super::atlas_work::unit_manifests(repo.db(), &run_id, stage).await
    })
}

pub fn atlas_get_all_unit_manifests(run_id: &str) -> Result<Vec<UnitManifest>> {
    let run_id = s(run_id);
    runtime::blocking(|repo| async move {
        super::atlas_work::all_unit_manifests(repo.db(), &run_id).await
    })
}

pub fn atlas_set_state(id: &str, state: &str, note: &str, finished: bool) -> Result<()> {
    let (id, state, note) = (s(id), s(state), s(note));
    runtime::blocking(|repo| async move {
        super::atlas::set_state(repo.db(), &id, &state, &note, finished)
            .await
            .map_err(tx_err)
    })
}

pub fn atlas_latest_run() -> Result<Option<AtlasRunRow>> {
    runtime::blocking(
        |repo| async move { super::atlas::latest_run(repo.db()).await.map_err(tx_err) },
    )
}

pub fn atlas_list_runs() -> Result<Vec<AtlasRunRow>> {
    runtime::blocking(
        |repo| async move { super::atlas::list_runs(repo.db()).await.map_err(tx_err) },
    )
}

pub fn atlas_delete_run(id: &str) -> Result<bool> {
    let id = s(id);
    runtime::blocking(|repo| async move {
        super::atlas_delete::delete_run(repo.publisher(), &id)
            .await
            .map_err(tx_err)
    })
}

pub fn delete_article_insights(run_id: &str, article_id: &str) -> Result<usize> {
    let (run_id, article_id) = (s(run_id), s(article_id));
    runtime::blocking(|repo| async move {
        super::atlas_delete::delete_article_insights(repo.publisher(), &run_id, &article_id)
            .await
            .map_err(tx_err)
    })
}

/// `Store::delete_cycle_memories`: drop one cycle's sources and the memories
/// left without one, reconcile what they backed, and queue their vector
/// removal through `write_then_index`.
pub fn delete_cycle_memories(run_id: &str) -> Result<()> {
    let run_id = s(run_id);
    runtime::blocking(|repo| async move {
        let w: super::memory::MemoryWrite = std::sync::Arc::new(move |tx| {
            let run_id = run_id.clone();
            Box::pin(async move {
                let ids = super::atlas_delete::delete_cycle_memories_in(&tx, &run_id).await?;
                super::atlas_delete::reconcile_deleted_memories_in(&tx, &ids).await?;
                Ok(ids)
            })
        });
        super::memory::write_then_index(repo.publisher(), w)
            .await
            .map_err(tx_err)?;
        Ok(())
    })
}

pub fn atlas_upsert_article(row: &AtlasArticleRow) -> Result<()> {
    let row = row.clone();
    runtime::blocking(|repo| async move {
        super::atlas::upsert_article(repo.db(), &row)
            .await
            .map_err(tx_err)
    })
}

pub fn atlas_delete_article(run_id: &str, article_id: &str) -> Result<()> {
    let (run_id, article_id) = (s(run_id), s(article_id));
    runtime::blocking(|repo| async move {
        super::atlas_delete::delete_article(repo.publisher(), &run_id, &article_id)
            .await
            .map_err(tx_err)
    })
}

pub fn atlas_set_category(run_id: &str, article_id: &str, category: &str) -> Result<()> {
    let (run_id, article_id, category) = (s(run_id), s(article_id), s(category));
    runtime::blocking(|repo| async move {
        super::atlas::set_category(repo.db(), &run_id, &article_id, &category)
            .await
            .map_err(tx_err)
    })
}

pub fn atlas_list_articles(run_id: &str) -> Result<Vec<AtlasArticleRow>> {
    let run_id = s(run_id);
    runtime::blocking(|repo| async move {
        super::atlas::list_articles(repo.db(), &run_id)
            .await
            .map_err(tx_err)
    })
}

pub fn atlas_recent_articles() -> Result<Vec<AtlasArticleRow>> {
    runtime::blocking(|repo| async move {
        super::atlas::recent_articles(repo.db())
            .await
            .map_err(tx_err)
    })
}

pub fn atlas_article(run_id: &str, article_id: &str) -> Result<Option<AtlasArticleRow>> {
    let (run_id, article_id) = (s(run_id), s(article_id));
    runtime::blocking(|repo| async move {
        super::atlas::article(repo.db(), &run_id, &article_id)
            .await
            .map_err(tx_err)
    })
}

pub fn atlas_latest_article_by_id(article_id: &str) -> Result<Option<AtlasArticleRow>> {
    let article_id = s(article_id);
    runtime::blocking(|repo| async move {
        super::atlas::latest_article_by_id(repo.db(), &article_id)
            .await
            .map_err(tx_err)
    })
}

pub fn atlas_run_days() -> Result<Vec<String>> {
    runtime::blocking(|repo| async move { super::atlas::run_days(repo.db()).await.map_err(tx_err) })
}

pub fn atlas_articles_for_intel(
    category: &str,
    day: &str,
    query: &str,
) -> Result<Vec<AtlasArticleRow>> {
    let (category, day, query) = (s(category), s(day), s(query));
    runtime::blocking(|repo| async move {
        super::atlas::articles_for_intel(repo.db(), &category, &day, &query)
            .await
            .map_err(tx_err)
    })
}

pub fn save_intel_link_explanations(
    article_id: &str,
    explanations: &HashMap<(String, String), String>,
) -> Result<()> {
    let (article_id, explanations) = (s(article_id), explanations.clone());
    runtime::blocking(|repo| async move {
        super::intel::save_link_explanations(repo.db(), &article_id, &explanations)
            .await
            .map_err(tx_err)
    })
}

pub fn get_intel_link_explanations(article_id: &str) -> Result<HashMap<(String, String), String>> {
    let article_id = s(article_id);
    runtime::blocking(|repo| async move {
        super::intel::link_explanations(repo.db(), &article_id)
            .await
            .map_err(tx_err)
    })
}

pub fn delete_intel_link_explanations(article_id: &str) -> Result<()> {
    let article_id = s(article_id);
    runtime::blocking(|repo| async move {
        super::intel::delete_link_explanations(repo.db(), &article_id)
            .await
            .map_err(tx_err)
    })
}

pub fn atlas_park_running() -> Result<()> {
    runtime::blocking(
        |repo| async move { super::atlas::park_running(repo.db()).await.map_err(tx_err) },
    )
}

pub fn atlas_quota_used(provider: &str, day: &str) -> Result<u32> {
    let (provider, day) = (s(provider), s(day));
    runtime::blocking(|repo| async move {
        super::atlas::quota_used(repo.db(), &provider, &day)
            .await
            .map_err(tx_err)
    })
}

pub fn atlas_quota_bump(provider: &str, day: &str) -> Result<u32> {
    let (provider, day) = (s(provider), s(day));
    runtime::blocking(|repo| async move {
        super::atlas::quota_bump(repo.db(), &provider, &day)
            .await
            .map_err(tx_err)
    })
}

pub fn atlas_prune_expired() -> Result<Vec<String>> {
    runtime::blocking(|repo| async move {
        super::atlas_delete::prune_expired(repo.publisher(), chrono::Utc::now())
            .await
            .map_err(tx_err)
    })
}

// ---- app_state ----

pub fn app_state_get(key: &str) -> Result<Option<String>> {
    let key = s(key);
    runtime::blocking(
        |repo| async move { super::pipeline_inputs::app_state_get(&repo, &key).await },
    )
}

pub fn app_state_set(key: &str, value: &str) -> Result<()> {
    let (key, value) = (s(key), s(value));
    runtime::blocking(|repo| async move {
        super::pipeline_inputs::app_state_set(&repo, &key, &value).await
    })
}

const AUTO_NEXT: &str = "atlas_auto_next";

pub fn atlas_auto_next() -> Result<Option<u64>> {
    Ok(app_state_get(AUTO_NEXT)?.and_then(|text| text.parse().ok()))
}

pub fn set_atlas_auto_next(when: Option<u64>) -> Result<()> {
    match when {
        Some(secs) => app_state_set(AUTO_NEXT, &secs.to_string()),
        None => runtime::blocking(|repo| async move {
            super::pipeline_inputs::app_state_delete(&repo, AUTO_NEXT).await
        }),
    }
}

// ---- Profile dashboard ----

pub fn profile_snapshot(
    period: &crate::profile_stats::Period,
    filters: &crate::profile_stats::StatFilters,
) -> Result<crate::profile_stats::ProfileSnapshot> {
    let (period, filters) = (period.clone(), filters.clone());
    runtime::blocking(|repo| async move {
        super::profile_view::snapshot(repo.db(), &period, &filters, chrono::Utc::now()).await
    })
}

pub fn profile_filter_options() -> Vec<(String, Vec<String>)> {
    runtime::blocking(|repo| async move { super::profile_view::filter_options(repo.db()).await })
        .unwrap_or_else(|err| {
            tracing::warn!("profile_filter_options: {err:#}");
            Vec::new()
        })
}

// ---- Insight reads and the article replacement ----

#[derive(SurrealValue)]
struct StoredClaimRow {
    fingerprint: String,
    memory_id: String,
    entity_id: String,
    predicate: String,
    object_value: String,
    topic: String,
    classification: String,
    confidence: f64,
    source_reliability: String,
    info_credibility: i64,
    admiralty: String,
    rsp_status: String,
    #[allow(dead_code)]
    created_at: String,
}

#[derive(SurrealValue)]
struct SourceRow {
    fingerprint: String,
    call_id: String,
    source_url: Option<String>,
    published_at: String,
}

const CLAIM_FIELDS: &str = "record::id(id) AS fingerprint, memory_id, entity_id, predicate, \
     object_value, topic, classification, confidence, source_reliability, info_credibility, \
     admiralty, rsp_status, created_at";

/// `Store::article_insight_user_edit_count`.
pub fn article_insight_user_edit_count(run_id: &str, article_id: &str) -> Result<usize> {
    let (run, call) = (s(run_id), s(article_id));
    runtime::blocking(|repo| async move {
        let mut r = repo
            .db()
            .query(format!(
                "LET $fps = ({});
                 LET $mems = array::distinct(SELECT VALUE memory_id FROM insight_claim
                     WHERE record::id(id) INSIDE $fps);
                 RETURN count(SELECT VALUE id FROM insight_user_edit
                     WHERE record::id(id) INSIDE $mems);",
                super::insight::SOURCES_BY_CALL_SURQL
            ))
            .bind(("answer", atlas_answer_id(&run)))
            .bind(("run", run))
            .bind(("call", call))
            .await?
            .check()?;
        let n: Option<i64> = r.take(2usize)?;
        Ok(n.unwrap_or(0).max(0) as usize)
    })
}

/// `Store::commit_article_insight_replacement`.
pub fn commit_article_insight_replacement(
    run_id: &str,
    article_id: &str,
    claims: &[AtlasInsightClaim],
    relations: &[(String, String, String)],
) -> Result<crate::store::ArticleInsightCommit> {
    let (run_id, article_id) = (s(run_id), s(article_id));
    let (claims, relations) = (claims.to_vec(), relations.to_vec());
    let done = runtime::blocking(|repo| async move {
        super::insight::commit_article_insight_replacement(
            repo.publisher(),
            &run_id,
            &article_id,
            claims,
            relations,
        )
        .await
        .map_err(tx_err)
    })?;
    Ok(crate::store::ArticleInsightCommit {
        claims: done.claims,
        preserved_user_edits: done.preserved_user_edits,
    })
}

/// `Store::atlas_has_insights`.
pub fn atlas_has_insights(run_id: &str) -> Result<bool> {
    let run_id = s(run_id);
    runtime::blocking(
        |repo| async move { super::atlas_memory::has_insights(repo.db(), &run_id).await },
    )
}

async fn run_claims(
    db: &surrealdb::Surreal<surrealdb::engine::any::Any>,
    run: &str,
    call: Option<&str>,
) -> Result<(Vec<StoredClaimRow>, Vec<SourceRow>)> {
    let filter = if call.is_some() {
        "run_id = $run AND answer_id = $answer AND call_id = $call"
    } else {
        "run_id = $run AND answer_id = $answer"
    };
    let mut r = db
        .query(format!(
            "LET $src = (SELECT fingerprint, call_id, source_url, published_at
                 FROM insight_source WHERE {filter});
             LET $fps = array::distinct($src.fingerprint);
             $src;
             SELECT {CLAIM_FIELDS} FROM insight_claim
                 WHERE record::id(id) INSIDE $fps ORDER BY created_at;"
        ))
        .bind(("run", s(run)))
        .bind(("answer", atlas_answer_id(run)))
        .bind(("call", call.unwrap_or_default().to_string()))
        .await?
        .check()?;
    let sources: Vec<SourceRow> = r.take(2usize)?;
    let claims: Vec<StoredClaimRow> = r.take(3usize)?;
    Ok((claims, sources))
}

/// `Store::atlas_stored_claims`.
pub fn atlas_stored_claims(run_id: &str) -> Result<Vec<AtlasStoredClaim>> {
    let run = s(run_id);
    let (claims, _) =
        runtime::blocking(|repo| async move { run_claims(repo.db(), &run, None).await })?;
    Ok(claims
        .into_iter()
        .map(|c| AtlasStoredClaim {
            fingerprint: c.fingerprint,
            entity: c.entity_id,
            predicate: c.predicate,
            object: c.object_value,
            topic: c.topic,
            classification: c.classification,
            reliability: c.source_reliability,
            info_credibility: c.info_credibility.clamp(0, 255) as u8,
            admiralty: c.admiralty,
        })
        .collect())
}

/// `Store::atlas_claims_for_article`: the article's legacy source claims
/// with their memory text. The provenance-snapshot rows the SQLite path
/// tries first (`insight_persist::article_claim_rows`) are served by
/// `storage::insight_views` on SurrealDB and are not consulted here.
pub fn atlas_claims_for_article(run_id: &str, article_id: &str) -> Result<Vec<AtlasArticleClaim>> {
    let (run, call) = (s(run_id), s(article_id));
    runtime::blocking(|repo| async move {
        let (claims, sources) = run_claims(repo.db(), &run, Some(&call)).await?;
        let mem_ids: Vec<String> = claims.iter().map(|c| c.memory_id.clone()).collect();
        let mut r = repo
            .db()
            .query(
                "SELECT record::id(id) AS id, text FROM memory
                 WHERE record::id(id) INSIDE $ids",
            )
            .bind(("ids", mem_ids))
            .await?
            .check()?;
        #[derive(SurrealValue)]
        struct TextRow {
            id: String,
            text: String,
        }
        let texts: HashMap<String, String> = r
            .take::<Vec<TextRow>>(0usize)?
            .into_iter()
            .map(|t| (t.id, t.text))
            .collect();
        let by_fp: HashMap<&str, &SourceRow> = sources
            .iter()
            .map(|src| (src.fingerprint.as_str(), src))
            .collect();
        Ok(claims
            .into_iter()
            .filter_map(|c| {
                let claim = texts.get(&c.memory_id)?.clone();
                let src = by_fp.get(c.fingerprint.as_str())?;
                Some(AtlasArticleClaim {
                    claim,
                    source_url: src.source_url.clone().unwrap_or_default(),
                    published_at: src.published_at.clone(),
                    article_id: src.call_id.clone(),
                    fingerprint: c.fingerprint,
                    entity: c.entity_id,
                    predicate: c.predicate,
                    object: c.object_value,
                    topic: c.topic,
                    classification: c.classification,
                    confidence: c.confidence,
                    reliability: c.source_reliability,
                    info_credibility: c.info_credibility.clamp(0, 255) as u8,
                    admiralty: c.admiralty,
                    rsp_status: c.rsp_status,
                })
            })
            .collect())
    })
}

/// `Store::insight_relations_among`.
pub fn insight_relations_among(fingerprints: &[String]) -> Result<Vec<(String, String, String)>> {
    if fingerprints.is_empty() {
        return Ok(Vec::new());
    }
    let fps = fingerprints.to_vec();
    #[derive(SurrealValue)]
    struct RelRow {
        left_fingerprint: String,
        right_fingerprint: String,
        relation: String,
    }
    let rows: Vec<RelRow> = runtime::blocking(|repo| async move {
        let mut r = repo
            .db()
            .query(
                "SELECT left_fingerprint, right_fingerprint, relation FROM insight_relation
                 WHERE left_fingerprint INSIDE $fps AND right_fingerprint INSIDE $fps",
            )
            .bind(("fps", fps))
            .await?
            .check()?;
        Ok(r.take(0usize)?)
    })?;
    Ok(rows
        .into_iter()
        .map(|r| (r.left_fingerprint, r.right_fingerprint, r.relation))
        .collect())
}

/// `Store::memory_insight_kind`.
pub fn memory_insight_kind(memory_id: &str) -> Option<String> {
    let m = s(memory_id);
    runtime::blocking(|repo| async move {
        let mut r = repo
            .db()
            .query(
                "SELECT VALUE insight_kind FROM insight_derivation
                 WHERE memory_id = $m ORDER BY id DESC LIMIT 1",
            )
            .bind(("m", m))
            .await?
            .check()?;
        let kinds: Vec<String> = r.take(0usize)?;
        Ok(kinds.into_iter().next())
    })
    .ok()
    .flatten()
}

// ---- Vectors (SurrealDB HNSW generations replace the Lance index) ----

fn disabled() -> IndexOutcome {
    IndexOutcome::Disabled {
        reason: "semantic indexing disabled (ARGOS_EMBED=0)".into(),
    }
}

fn embedder() -> Result<super::vectors::Embedder> {
    super::vectors::Embedder::local()
        .ok_or_else(|| anyhow!("Brain vectors are off (ARGOS_EMBED=0); nothing to reindex"))
}

const DRAIN_LIMIT: i64 = 512;

/// `Store::ensure_memory_vectors`: serve a generation (reindex when none
/// serves this model), then drain pending index changes.
pub fn ensure_memory_vectors() -> Result<()> {
    let e = embedder()?;
    runtime::blocking(|repo| async move {
        let serving = super::vectors::state(&repo).await?;
        if serving.as_ref().map(|st| st.model.as_str()) != Some(e.model.as_str()) {
            super::vectors::reindex(&repo, &e).await?;
        }
        super::index_drain::drain_index(&repo, &e, DRAIN_LIMIT).await?;
        Ok(())
    })
}

/// `Store::reindex_memory_vectors`. `lance_dir` is empty: vectors live in
/// SurrealDB.
pub fn reindex_memory_vectors() -> Result<ReindexReport> {
    let e = embedder()?;
    crate::embed::warm_up()?;
    let report = runtime::blocking(|repo| async move { super::vectors::reindex(&repo, &e).await })?;
    Ok(ReindexReport {
        memories: report.memories,
        lance_dir: std::path::PathBuf::new(),
        fingerprint: report.fingerprint,
    })
}

/// `Store::process_pending_vector_rebuild`: one drain pass per batch.
pub fn process_pending_vector_rebuild(max_batches: usize) -> Result<usize> {
    let e = embedder()?;
    runtime::blocking(|repo| async move {
        let mut done = 0usize;
        for _ in 0..max_batches {
            let d = super::index_drain::drain_index(&repo, &e, DRAIN_LIMIT).await?;
            if d.completed + d.failed == 0 {
                break;
            }
            done += 1;
        }
        Ok(done)
    })
}

fn ready(generation: String) -> IndexOutcome {
    IndexOutcome::Ready {
        revision: String::new(),
        fingerprint: crate::embed::fingerprint(),
        generation,
    }
}

/// `Store::try_index_upsert` / `try_index_memory`: embed `ids` into the
/// serving generation.
pub fn try_index_upsert(ids: &[String]) -> IndexOutcome {
    let Some(e) = super::vectors::Embedder::local() else {
        return disabled();
    };
    let ids = ids.to_vec();
    match runtime::blocking(|repo| async move {
        super::vectors::index_new(&repo, &e, &ids).await?;
        Ok(super::vectors::state(&repo).await?.map(|st| st.generation))
    }) {
        Ok(generation) => ready(generation.map(|g| g.to_string()).unwrap_or_default()),
        Err(err) => IndexOutcome::RetryableFailure {
            message: format!("{err:#}"),
        },
    }
}

/// `Store::try_index_remove_missing`: removals are queued by the publisher
/// as `index_change` rows, so draining them is the removal.
pub fn try_index_remove_missing(_ids: &[String]) -> IndexOutcome {
    try_process_vector_rebuild(1)
}

/// `Store::try_process_vector_rebuild`.
pub fn try_process_vector_rebuild(max_batches: usize) -> IndexOutcome {
    if super::vectors::Embedder::local().is_none() {
        return disabled();
    }
    match process_pending_vector_rebuild(max_batches) {
        Ok(_) => ready(String::new()),
        Err(err) => IndexOutcome::RetryableFailure {
            message: format!("{err:#}"),
        },
    }
}

// ---- Publication (`store/publication.rs`) ----

/// `Store::memories_changed_seq`.
pub fn memories_changed_seq() -> Result<i64> {
    runtime::blocking(|repo| async move {
        super::counters::current(repo.db(), super::publisher::CHANGED_COUNTER)
            .await
            .map_err(tx_err)
    })
}

/// `Store::index_now`: drain the queued work; one outcome per request.
pub fn index_now(queued: &[IndexEnqueue]) -> Vec<IndexOutcome> {
    let outcome = try_process_vector_rebuild(1);
    queued.iter().map(|_| outcome.clone()).collect()
}

/// `Store::verify_memory_coverage`.
pub fn verify_memory_coverage(ids: &[String]) -> Result<CoverageReport> {
    let ids = ids.to_vec();
    runtime::blocking(|repo| async move {
        super::atlas_memory::verify_memory_coverage(repo.db(), &ids).await
    })
}

/// `Store::requeue_uncovered`. SurrealDB queues `index_change` rows rather
/// than leased tasks, so no `IndexEnqueue` handles come back.
pub fn requeue_uncovered(report: &CoverageReport) -> Result<Vec<IndexEnqueue>> {
    let report = report.clone();
    runtime::blocking(|repo| async move {
        super::atlas_memory::requeue_uncovered(repo.publisher(), &report).await
    })?;
    Ok(Vec::new())
}

/// `Store::publish_atlas_insights`.
pub fn publish_atlas_insights(
    run_id: &str,
    claims: &[AtlasInsightClaim],
    relations: &[(String, String, String)],
    brief: &str,
    options: &PublishOptions,
) -> Result<PublicationReceipt> {
    let (run_id, brief, options) = (s(run_id), s(brief), options.clone());
    let (claims, relations) = (claims.to_vec(), relations.to_vec());
    runtime::blocking(|repo| async move {
        super::insight::publish_atlas_insights(
            repo.publisher(),
            &run_id,
            claims,
            relations,
            &brief,
            options,
        )
        .await
        .map_err(tx_err)
    })
}

/// `Store::atlas_publication_receipt`.
pub fn atlas_publication_receipt(run_id: &str) -> Result<Option<PublicationReceipt>> {
    let run_id = s(run_id);
    runtime::blocking(|repo| async move {
        super::atlas_memory::publication_receipt(repo.db(), &run_id).await
    })
}

/// `Store::verify_atlas_publication`.
pub fn verify_atlas_publication(receipt: &PublicationReceipt) -> Result<PublicationVerification> {
    let receipt = receipt.clone();
    runtime::blocking(|repo| async move {
        super::atlas_memory::verify_publication(repo.db(), &receipt).await
    })
}

/// `Store::tombstone_memory`.
pub fn tombstone_memory(memory_id: &str, reason: &str) -> Result<()> {
    let (memory_id, reason) = (s(memory_id), s(reason));
    runtime::blocking(|repo| async move {
        let w: super::publisher::Write = std::sync::Arc::new(move |tx| {
            let (memory_id, reason) = (memory_id.clone(), reason.clone());
            Box::pin(async move {
                super::insight::tombstone_memory(&tx, &memory_id, &reason).await?;
                Ok(super::publisher::Wrote {
                    changes: Vec::new(),
                    changed: false,
                })
            })
        });
        repo.publisher().publish(w).await.map_err(tx_err)?;
        Ok(())
    })
}

/// `Store::is_tombstoned`.
pub fn is_tombstoned(memory_id: &str) -> Result<bool> {
    let m = s(memory_id);
    runtime::blocking(|repo| async move { super::atlas_memory::is_tombstoned(repo.db(), &m).await })
}

// ---- Brain kinds and actors (`brain_kinds.rs`, `related_global.rs`) ----

/// `brain_kinds::memory_kind`: a missing or unknown kind reads as a Claim.
pub fn memory_kind(memory_id: &str) -> Result<crate::memory_metadata::MemoryKind> {
    let id = s(memory_id);
    Ok(runtime::blocking(
        |repo| async move { super::topology::memory_seed_kind(&repo, &id).await },
    )?
    .unwrap_or(crate::memory_metadata::MemoryKind::Claim))
}

/// `brain_actors::resolve`: follow merge redirects to the canonical actor.
pub fn actor_resolve(actor_id: &str) -> Result<String> {
    let id = s(actor_id);
    runtime::blocking(|repo| async move {
        let mut id = id;
        for _ in 0..64 {
            let mut r = repo
                .db()
                .query("SELECT VALUE to_actor FROM ONLY $r")
                .bind(("r", super::jobs::rid("brain_actor_redirect", &id)))
                .await?
                .check()?;
            match r.take::<Option<String>>(0usize)? {
                Some(next) => id = next,
                None => return Ok(id),
            }
        }
        Err(anyhow!("actor redirect chain does not terminate"))
    })
}

/// The canonical actor's display name (`brain_actors::actor`).
pub fn actor_display_name(actor_id: &str) -> Result<Option<String>> {
    let id = actor_resolve(actor_id)?;
    runtime::blocking(|repo| async move {
        let mut r = repo
            .db()
            .query("SELECT VALUE display_name FROM ONLY $r")
            .bind(("r", super::jobs::rid("brain_actor", &id)))
            .await?
            .check()?;
        Ok(r.take::<Option<String>>(0usize)?)
    })
}

/// `brain_actors::mentions`: `(origin_kind, origin_scope, origin_id,
/// origin_revision)` of an actor and everything merged into it.
pub fn actor_mentions(actor_id: &str) -> Result<Vec<(String, String, String, String)>> {
    let root = actor_resolve(actor_id)?;
    #[derive(SurrealValue)]
    struct MentionRow {
        origin_kind: String,
        origin_scope: String,
        origin_id: String,
        origin_revision: String,
    }
    runtime::blocking(|repo| async move {
        let mut family = vec![root.clone()];
        let mut frontier = vec![root];
        while let Some(id) = frontier.pop() {
            let mut r = repo
                .db()
                .query("SELECT VALUE record::id(id) FROM brain_actor_redirect WHERE to_actor = $to")
                .bind(("to", id))
                .await?
                .check()?;
            for from in r.take::<Vec<String>>(0usize)? {
                if !family.contains(&from) {
                    family.push(from.clone());
                    frontier.push(from);
                }
            }
        }
        let mut r = repo
            .db()
            .query(
                "SELECT origin_kind, origin_scope, origin_id, origin_revision
                 FROM brain_actor_mention WHERE actor_id INSIDE $fam",
            )
            .bind(("fam", family))
            .await?
            .check()?;
        let mut out: Vec<_> = r
            .take::<Vec<MentionRow>>(0usize)?
            .into_iter()
            .map(|m| {
                (
                    m.origin_kind,
                    m.origin_scope,
                    m.origin_id,
                    m.origin_revision,
                )
            })
            .collect();
        out.sort();
        out.dedup();
        Ok(out)
    })
}

// ---- Brain backfill (`brain_backfill.rs`) ----

pub fn brain_backfill(max_units: usize) -> Result<crate::brain_backfill::BackfillReport> {
    runtime::blocking(|repo| async move { super::brain_backfill::backfill(&repo, max_units).await })
}

/// Coverage only: a zero-budget backfill pass processes nothing.
pub fn brain_backfill_coverage() -> Result<Vec<crate::brain_backfill::SourceCoverage>> {
    Ok(brain_backfill(0)?.coverage)
}

// ---- Related articles (`related_articles.rs`, `related_global.rs`) ----

pub fn related_count(run_id: &str) -> Result<usize> {
    let run_id = s(run_id);
    runtime::blocking(|repo| async move {
        super::atlas_metadata::related_count(repo.db(), &run_id)
            .await
            .map_err(tx_err)
    })
}

pub fn replace_related(
    run_id: &str,
    articles: &[AtlasArticleRow],
    planned: &[crate::related_articles::PlannedPair],
) -> Result<usize> {
    let (run_id, articles, planned) = (s(run_id), articles.to_vec(), planned.to_vec());
    runtime::blocking(|repo| async move {
        super::atlas_metadata::replace_related(repo.db(), &run_id, &articles, &planned)
            .await
            .map_err(tx_err)
    })
}

pub fn related_pairs(
    run_id: &str,
    article_id: &str,
) -> Result<Vec<super::atlas_metadata::RelatedPairRow>> {
    let (run_id, article_id) = (s(run_id), s(article_id));
    runtime::blocking(|repo| async move {
        super::atlas_metadata::related_pairs(repo.db(), &run_id, &article_id)
            .await
            .map_err(tx_err)
    })
}

pub fn article_origins_for_memory(
    memory_id: &str,
) -> Result<Vec<crate::related_articles::ArticleOrigin>> {
    let id = s(memory_id);
    runtime::blocking(|repo| async move {
        super::browse_assoc::article_origins_for_memory(&repo, &id).await
    })
}

#[derive(SurrealValue)]
struct EndpointRow {
    memory_id: String,
    side: String,
    origin: String,
    ref_id: String,
}

/// `(run, article, memory)` for Related memories whose other article
/// endpoint is not this occurrence. `active_only` keeps lifecycle `active`;
/// otherwise anything not `retired`.
pub fn related_endpoint_candidates(
    run_id: &str,
    article_id: &str,
    active_only: bool,
) -> Result<Vec<(String, String, String)>> {
    let (run, art) = (s(run_id), s(article_id));
    runtime::blocking(|repo| async move {
        let mut r = repo
            .db()
            .query(
                "LET $mine = (SELECT memory_id, side, origin, ref_id FROM brain_memory_endpoint
                     WHERE endpoint_kind = 'article' AND ref_id = $art AND origin = $run);
                 LET $mids = array::distinct($mine.memory_id);
                 $mine;
                 SELECT memory_id, side, origin, ref_id FROM brain_memory_endpoint
                     WHERE endpoint_kind = 'article' AND memory_id INSIDE $mids;
                 SELECT VALUE record::id(id) FROM memory
                     WHERE record::id(id) INSIDE $mids AND memory_kind = 'related'
                     AND (IF $active THEN lifecycle = 'active' ELSE lifecycle != 'retired' END);",
            )
            .bind(("art", art))
            .bind(("run", run))
            .bind(("active", active_only))
            .await?
            .check()?;
        let mine: Vec<EndpointRow> = r.take(2usize)?;
        let all: Vec<EndpointRow> = r.take(3usize)?;
        let keep: std::collections::HashSet<String> =
            r.take::<Vec<String>>(4usize)?.into_iter().collect();
        let mut out = Vec::new();
        for e in &mine {
            if !keep.contains(&e.memory_id) {
                continue;
            }
            for o in all
                .iter()
                .filter(|o| o.memory_id == e.memory_id && o.side != e.side)
            {
                out.push((o.origin.clone(), o.ref_id.clone(), o.memory_id.clone()));
            }
        }
        out.sort();
        out.dedup();
        Ok(out)
    })
}

/// `(run, article)` article endpoints of one memory.
pub fn memory_article_endpoints(
    memory_id: &str,
) -> Result<Vec<crate::related_articles::ArticleOrigin>> {
    let id = s(memory_id);
    let rows: Vec<EndpointRow> = runtime::blocking(|repo| async move {
        let mut r = repo
            .db()
            .query(
                "SELECT memory_id, side, origin, ref_id FROM brain_memory_endpoint
                 WHERE memory_id = $m AND endpoint_kind = 'article'",
            )
            .bind(("m", id))
            .await?
            .check()?;
        Ok(r.take(0usize)?)
    })?;
    Ok(rows
        .into_iter()
        .map(|e| crate::related_articles::ArticleOrigin {
            run_id: e.origin,
            article_id: e.ref_id,
        })
        .collect())
}

/// `(run, article, url)` for every stored article with a URL.
pub fn article_urls() -> Result<Vec<(String, String, String)>> {
    #[derive(SurrealValue)]
    struct UrlRow {
        run_id: String,
        article_id: String,
        url: String,
    }
    let rows: Vec<UrlRow> = runtime::blocking(|repo| async move {
        let mut r = repo
            .db()
            .query("SELECT run_id, article_id, url FROM atlas_article WHERE url != ''")
            .await?
            .check()?;
        Ok(r.take(0usize)?)
    })?;
    Ok(rows
        .into_iter()
        .map(|u| (u.run_id, u.article_id, u.url))
        .collect())
}
