//! Insight publication on SurrealDB (P3 step 1): the insight table family
//! and `Store::commit_article_insight_replacement`.
//!
//! An article's replacement runs as one publisher transaction: its old
//! sources go, orphaned claims (and their memories, unless the user edited
//! them) go with their relations and graph summaries, the staged claims are
//! validated and written, and the index work for every touched memory is
//! queued before commit. A validation failure aborts the whole replacement.

use std::sync::{Arc, Mutex};

use surrealdb::engine::any::Any;
use surrealdb::Surreal;

use super::codec::encode;
use super::jobs::{run, take};
use super::memory::{write_then_index, MemoryWrite};
use super::publisher::Publisher;
use super::tx::{Tx, TxError};
use crate::store::{atlas_claim_identity, AtlasInsightClaim};

/// Insight schema (idempotent). Record keys: `insight_claim` by fingerprint,
/// `insight_source` by `[fingerprint, answer_id, call_id]`,
/// `insight_relation` by `[left, right, relation]`, `insight_user_edit` and
/// `memory_graph_summary` by memory id.
pub const SCHEMA: &str = "DEFINE TABLE IF NOT EXISTS insight_claim SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS memory_id ON insight_claim TYPE string;
DEFINE FIELD IF NOT EXISTS entity_id ON insight_claim TYPE string;
DEFINE FIELD IF NOT EXISTS predicate ON insight_claim TYPE string;
DEFINE FIELD IF NOT EXISTS object_value ON insight_claim TYPE string;
DEFINE FIELD IF NOT EXISTS topic ON insight_claim TYPE string;
DEFINE FIELD IF NOT EXISTS classification ON insight_claim TYPE string;
DEFINE FIELD IF NOT EXISTS confidence ON insight_claim TYPE float;
DEFINE FIELD IF NOT EXISTS source_reliability ON insight_claim TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS info_credibility ON insight_claim TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS admiralty ON insight_claim TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS rsp_status ON insight_claim TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS created_at ON insight_claim TYPE string;
DEFINE FIELD IF NOT EXISTS updated_at ON insight_claim TYPE string;
DEFINE INDEX IF NOT EXISTS insight_claim_memory ON insight_claim FIELDS memory_id;
DEFINE TABLE IF NOT EXISTS insight_source SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS fingerprint ON insight_source TYPE string;
DEFINE FIELD IF NOT EXISTS thread_id ON insight_source TYPE option<string>;
DEFINE FIELD IF NOT EXISTS run_id ON insight_source TYPE option<string>;
DEFINE FIELD IF NOT EXISTS answer_id ON insight_source TYPE string;
DEFINE FIELD IF NOT EXISTS call_id ON insight_source TYPE string;
DEFINE FIELD IF NOT EXISTS source_url ON insight_source TYPE option<string>;
DEFINE FIELD IF NOT EXISTS deleted_origin ON insight_source TYPE bool DEFAULT false;
DEFINE FIELD IF NOT EXISTS published_at ON insight_source TYPE string DEFAULT '';
DEFINE INDEX IF NOT EXISTS insight_source_call ON insight_source FIELDS run_id, answer_id, call_id;
DEFINE INDEX IF NOT EXISTS insight_source_fingerprint ON insight_source FIELDS fingerprint;
DEFINE TABLE IF NOT EXISTS insight_relation SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS left_fingerprint ON insight_relation TYPE string;
DEFINE FIELD IF NOT EXISTS right_fingerprint ON insight_relation TYPE string;
DEFINE FIELD IF NOT EXISTS relation ON insight_relation TYPE string;
DEFINE INDEX IF NOT EXISTS insight_relation_left ON insight_relation FIELDS left_fingerprint;
DEFINE INDEX IF NOT EXISTS insight_relation_right ON insight_relation FIELDS right_fingerprint;
DEFINE TABLE IF NOT EXISTS insight_user_edit SCHEMAFULL;
DEFINE TABLE IF NOT EXISTS memory_graph_summary SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS summary ON memory_graph_summary TYPE string;
DEFINE FIELD IF NOT EXISTS focus ON memory_graph_summary TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS created_at ON memory_graph_summary TYPE string;";

/// Apply the insight schema (idempotent).
pub async fn apply_schema(db: &Surreal<Any>) -> Result<(), TxError> {
    db.query(SCHEMA).await?.check()?;
    Ok(())
}

/// What one article replacement did (`store::ArticleInsightCommit`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ArticleReplacement {
    pub claims: usize,
    pub preserved_user_edits: usize,
    /// Fingerprints of claims created by this replacement; their subject
    /// binding (`brain_backfill::bind_claim_subject`) ports with the backfill
    /// receipts in step 4.
    pub created: Vec<String>,
}

/// `atlas_answer_id`: the answer id under which Atlas stores a run.
pub fn atlas_answer_id(run_id: &str) -> String {
    format!("atlas-{run_id}")
}

/// Validate one staged claim against its article; `None` when invalid.
fn normalized(claim: &AtlasInsightClaim, article_id: &str) -> Option<[String; 4]> {
    let parts = [
        claim.entity.trim().to_ascii_lowercase(),
        claim.namespace.trim().to_ascii_lowercase(),
        claim.predicate.trim().to_ascii_lowercase(),
        claim.object.trim().to_ascii_lowercase(),
    ];
    let ok = parts.iter().all(|p| !p.is_empty())
        && !claim.claim.trim().is_empty()
        && claim.article_id.trim() == article_id;
    ok.then_some(parts)
}

/// Delete one orphaned claim with its relations, summary and memory.
/// Returns the deleted memory id (if any) and whether the claim was instead
/// preserved because the user edited its memory.
async fn drop_orphan(tx: &Tx, fingerprint: &str) -> Result<(Option<String>, bool), TxError> {
    let mut res = run(
        tx,
        tx.query("SELECT VALUE memory_id FROM insight_claim WHERE id = type::record('insight_claim', $fp);")
            .bind(("fp", fingerprint.to_string())),
    )
    .await?;
    let memory: Option<String> = take::<Vec<String>>(&mut res, 0)?.into_iter().next();
    let edited = match &memory {
        Some(mem) => {
            let mut r = run(
                tx,
                tx.query("RETURN record::exists(type::record('insight_user_edit', $mem));")
                    .bind(("mem", mem.clone())),
            )
            .await?;
            take::<Option<bool>>(&mut r, 0)?
        }
        None => None,
    };
    if memory.is_some() && edited == Some(true) {
        return Ok((None, true));
    }
    run(
        tx,
        tx.query(
            "DELETE insight_relation WHERE left_fingerprint = $fp OR right_fingerprint = $fp RETURN NONE;
             DELETE type::record('insight_claim', $fp) RETURN NONE;
             IF $mem != NONE {
               DELETE type::record('memory_graph_summary', $mem) RETURN NONE;
               DELETE type::record('memory', $mem) RETURN NONE;
               DELETE brain_graph_edge WHERE owner = $mem RETURN NONE;
             };",
        )
        .bind(("fp", fingerprint.to_string()))
        .bind(("mem", memory.clone())),
    )
    .await?;
    Ok((memory, false))
}

async fn replace_in(
    tx: Tx,
    run_id: String,
    article_id: String,
    claims: Arc<Vec<AtlasInsightClaim>>,
    relations: Arc<Vec<(String, String, String)>>,
    ids_seq: u64,
) -> Result<(Vec<String>, ArticleReplacement), TxError> {
    // Validate first so an invalid claim aborts before anything is read.
    let mut staged = Vec::with_capacity(claims.len());
    for claim in claims.iter() {
        let Some(parts) = normalized(claim, &article_id) else {
            return Err(TxError::Abort("invalid staged claim during commit".into()));
        };
        staged.push((claim, parts, atlas_claim_identity(claim)));
    }
    let answer_id = atlas_answer_id(&run_id);
    let now = chrono::Utc::now().to_rfc3339();
    let mut res = run(
        &tx,
        tx.query(
            "SELECT VALUE fingerprint FROM insight_source
                WHERE run_id = $run AND answer_id = $answer AND call_id = $call;
             DELETE insight_source
                WHERE run_id = $run AND answer_id = $answer AND call_id = $call RETURN NONE;",
        )
        .bind(("run", run_id.clone()))
        .bind(("answer", answer_id.clone()))
        .bind(("call", article_id.clone())),
    )
    .await?;
    let mut old: Vec<String> = take(&mut res, 0)?;
    old.sort();
    old.dedup();
    let mut touched = Vec::new();
    let mut out = ArticleReplacement::default();
    for fp in &old {
        let mut r = run(
            &tx,
            tx.query("RETURN count(SELECT id FROM insight_source WHERE fingerprint = $fp);")
                .bind(("fp", fp.clone())),
        )
        .await?;
        if take::<Option<i64>>(&mut r, 0)?.unwrap_or(0) > 0 {
            continue;
        }
        match drop_orphan(&tx, fp).await? {
            (_, true) => out.preserved_user_edits += 1,
            (Some(mem), false) => touched.push(mem),
            (None, false) => {}
        }
    }
    for (i, (claim, [entity, _ns, predicate, object], fp)) in staged.into_iter().enumerate() {
        let mut r = run(
            &tx,
            tx.query("RETURN record::exists(type::record('insight_claim', $fp));")
                .bind(("fp", fp.clone())),
        )
        .await?;
        if take::<Option<bool>>(&mut r, 0)? != Some(true) {
            let memory_id = format!(
                "mem-{}-{ids_seq}-{i}",
                chrono::Utc::now().timestamp_millis()
            );
            let source = serde_json::json!({
                "app": "atlas", "conversation_id": run_id,
                "message_id": null, "reference": run_id,
            })
            .to_string();
            run(
                &tx,
                tx.query(
                    "CREATE type::record('memory', $mem) SET text = $text,
                       category = 'investigation', pinned = false, source_json = $src RETURN NONE;
                     CREATE type::record('insight_claim', $fp) SET memory_id = $mem,
                       entity_id = $entity, predicate = $pred, object_value = $obj,
                       topic = $topic, classification = $class, confidence = $conf,
                       created_at = $now, updated_at = $now RETURN NONE;",
                )
                .bind(("mem", memory_id.clone()))
                .bind(("text", claim.claim.trim().to_string()))
                .bind(("src", source))
                .bind(("fp", fp.clone()))
                .bind(("entity", entity))
                .bind(("pred", predicate))
                .bind(("obj", object))
                .bind(("topic", claim.topic.trim().to_string()))
                .bind(("class", claim.classification.trim().to_string()))
                .bind(("conf", claim.confidence))
                .bind(("now", now.clone())),
            )
            .await?;
            touched.push(memory_id);
            out.created.push(fp.clone());
        }
        run(
            &tx,
            tx.query(
                "UPSERT type::record('insight_source', $key) MERGE {
                   fingerprint: $fp, run_id: $run, answer_id: $answer, call_id: $call,
                   source_url: $url, published_at: $pub } RETURN NONE;
                 UPDATE type::record('insight_claim', $fp) SET confidence = $conf,
                   source_reliability = $rel, info_credibility = $cred,
                   admiralty = $adm, rsp_status = $rsp, updated_at = $now RETURN NONE;",
            )
            .bind(("key", encode(&[&fp, &answer_id, &article_id])))
            .bind(("fp", fp.clone()))
            .bind(("run", run_id.clone()))
            .bind(("answer", answer_id.clone()))
            .bind(("call", article_id.clone()))
            .bind(("url", claim.source_url.trim().to_string()))
            .bind(("pub", claim.published_at.trim().to_string()))
            .bind(("conf", claim.confidence))
            .bind(("rel", claim.reliability.trim().to_string()))
            .bind(("cred", claim.info_credibility as i64))
            .bind(("adm", claim.admiralty.trim().to_string()))
            .bind(("rsp", claim.rsp_status.trim().to_string()))
            .bind(("now", now.clone())),
        )
        .await?;
        out.claims += 1;
    }
    for (left, right, relation) in relations.iter() {
        if left.is_empty() || right.is_empty() || relation.is_empty() || left == right {
            continue;
        }
        run(
            &tx,
            tx.query(
                "UPSERT type::record('insight_relation', $key) SET left_fingerprint = $l,
                   right_fingerprint = $r, relation = $rel RETURN NONE;",
            )
            .bind(("key", encode(&[left, right, relation])))
            .bind(("l", left.clone()))
            .bind(("r", right.clone()))
            .bind(("rel", relation.clone())),
        )
        .await?;
    }
    Ok((touched, out))
}

static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// `Store::commit_article_insight_replacement` on SurrealDB: one publisher
/// transaction covering sources, claims, relations, summaries, memories and
/// the index work for every created or deleted memory.
pub async fn commit_article_insight_replacement(
    publisher: &Publisher,
    run_id: &str,
    article_id: &str,
    claims: Vec<AtlasInsightClaim>,
    relations: Vec<(String, String, String)>,
) -> Result<ArticleReplacement, TxError> {
    let outcome: Arc<Mutex<ArticleReplacement>> = Arc::default();
    let (run_id, article_id) = (run_id.to_string(), article_id.to_string());
    let (claims, relations) = (Arc::new(claims), Arc::new(relations));
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let slot = outcome.clone();
    let write: MemoryWrite = Arc::new(move |tx| {
        let (run_id, article_id) = (run_id.clone(), article_id.clone());
        let (claims, relations, slot) = (claims.clone(), relations.clone(), slot.clone());
        Box::pin(async move {
            let (ids, out) = replace_in(tx, run_id, article_id, claims, relations, seq).await?;
            // The last attempt to run is the one that committed.
            *slot.lock().expect("outcome lock") = out;
            Ok(ids)
        })
    });
    write_then_index(publisher, write).await?;
    let out = outcome.lock().expect("outcome lock").clone();
    Ok(out)
}
