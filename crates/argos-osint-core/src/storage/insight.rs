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
use surrealdb::types::SurrealValue;
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
DEFINE FIELD IF NOT EXISTS created_at ON memory_graph_summary TYPE string;
-- argos_memory_tombstones, keyed by [memory_id, fingerprint].
DEFINE TABLE IF NOT EXISTS memory_tombstone SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS memory_id ON memory_tombstone TYPE string;
DEFINE FIELD IF NOT EXISTS fingerprint ON memory_tombstone TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS reason ON memory_tombstone TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS run_ids ON memory_tombstone TYPE array<string> DEFAULT [];
DEFINE FIELD IF NOT EXISTS created_at ON memory_tombstone TYPE string;
DEFINE INDEX IF NOT EXISTS memory_tombstone_fp ON memory_tombstone FIELDS fingerprint;
DEFINE INDEX IF NOT EXISTS memory_tombstone_memory ON memory_tombstone FIELDS memory_id;
-- argos_atlas_publications, keyed by [run_id, revision].
DEFINE TABLE IF NOT EXISTS atlas_publication SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS run_id ON atlas_publication TYPE string;
DEFINE FIELD IF NOT EXISTS revision ON atlas_publication TYPE string;
DEFINE FIELD IF NOT EXISTS state ON atlas_publication TYPE string;
DEFINE FIELD IF NOT EXISTS receipt_json ON atlas_publication TYPE string DEFAULT '{}';
DEFINE FIELD IF NOT EXISTS job_id ON atlas_publication TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS created_at ON atlas_publication TYPE string;
DEFINE FIELD IF NOT EXISTS updated_at ON atlas_publication TYPE string;
DEFINE INDEX IF NOT EXISTS atlas_publication_run ON atlas_publication FIELDS run_id;
-- Schema 29 provenance: owner state keyed by [owner_kind, owner_id],
-- snapshots by snapshot id; derivations, evidence and links reference a
-- snapshot and are deleted with it.
DEFINE TABLE IF NOT EXISTS insight_owner_state SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS owner_kind ON insight_owner_state TYPE string;
DEFINE FIELD IF NOT EXISTS owner_id ON insight_owner_state TYPE string;
DEFINE FIELD IF NOT EXISTS desired_generation ON insight_owner_state TYPE int DEFAULT 1;
DEFINE FIELD IF NOT EXISTS active_snapshot_id ON insight_owner_state TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS refresh_job_id ON insight_owner_state TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS lifecycle ON insight_owner_state TYPE string DEFAULT 'pending';
DEFINE FIELD IF NOT EXISTS last_error ON insight_owner_state TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS updated_at ON insight_owner_state TYPE string;
DEFINE INDEX IF NOT EXISTS insight_owner_active ON insight_owner_state FIELDS active_snapshot_id;
DEFINE TABLE IF NOT EXISTS insight_snapshot SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS owner_kind ON insight_snapshot TYPE string;
DEFINE FIELD IF NOT EXISTS owner_id ON insight_snapshot TYPE string;
DEFINE FIELD IF NOT EXISTS generation ON insight_snapshot TYPE int;
DEFINE FIELD IF NOT EXISTS input_key ON insight_snapshot TYPE string;
DEFINE FIELD IF NOT EXISTS inputs_json ON insight_snapshot TYPE string;
DEFINE FIELD IF NOT EXISTS entities_json ON insight_snapshot TYPE string;
DEFINE FIELD IF NOT EXISTS coverage_json ON insight_snapshot TYPE string;
DEFINE FIELD IF NOT EXISTS readiness_json ON insight_snapshot TYPE string;
DEFINE FIELD IF NOT EXISTS contract_version ON insight_snapshot TYPE string;
DEFINE FIELD IF NOT EXISTS policy_version ON insight_snapshot TYPE string;
DEFINE FIELD IF NOT EXISTS model_versions_json ON insight_snapshot TYPE string DEFAULT '{}';
DEFINE FIELD IF NOT EXISTS created_at ON insight_snapshot TYPE string;
DEFINE INDEX IF NOT EXISTS insight_snapshot_identity ON insight_snapshot
  FIELDS owner_kind, owner_id, generation, input_key UNIQUE;
DEFINE TABLE IF NOT EXISTS insight_derivation SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS snapshot_id ON insight_derivation TYPE string;
DEFINE FIELD IF NOT EXISTS memory_id ON insight_derivation TYPE option<string>;
DEFINE FIELD IF NOT EXISTS insight_kind ON insight_derivation TYPE string
  ASSERT $value INSIDE ['claim', 'inference'];
DEFINE FIELD IF NOT EXISTS fingerprint ON insight_derivation TYPE string;
DEFINE FIELD IF NOT EXISTS item_json ON insight_derivation TYPE string;
DEFINE FIELD IF NOT EXISTS assessment_json ON insight_derivation TYPE string;
DEFINE FIELD IF NOT EXISTS status ON insight_derivation TYPE string;
DEFINE FIELD IF NOT EXISTS revision ON insight_derivation TYPE string;
DEFINE INDEX IF NOT EXISTS insight_derivation_memory ON insight_derivation FIELDS memory_id;
DEFINE INDEX IF NOT EXISTS insight_derivation_snapshot ON insight_derivation FIELDS snapshot_id;
DEFINE INDEX IF NOT EXISTS insight_derivation_status ON insight_derivation FIELDS status;
DEFINE TABLE IF NOT EXISTS insight_derivation_evidence SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS derivation_id ON insight_derivation_evidence TYPE string;
DEFINE FIELD IF NOT EXISTS source_id ON insight_derivation_evidence TYPE string;
DEFINE FIELD IF NOT EXISTS source_revision ON insight_derivation_evidence TYPE string;
DEFINE FIELD IF NOT EXISTS memory_id ON insight_derivation_evidence TYPE option<string>;
DEFINE FIELD IF NOT EXISTS premise_id ON insight_derivation_evidence TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS evidence_json ON insight_derivation_evidence TYPE string;
DEFINE INDEX IF NOT EXISTS insight_evidence_derivation ON insight_derivation_evidence FIELDS derivation_id;
DEFINE INDEX IF NOT EXISTS insight_evidence_source ON insight_derivation_evidence FIELDS source_id, source_revision;
DEFINE INDEX IF NOT EXISTS insight_evidence_memory ON insight_derivation_evidence FIELDS memory_id;
DEFINE INDEX IF NOT EXISTS insight_evidence_premise ON insight_derivation_evidence FIELDS premise_id;
DEFINE TABLE IF NOT EXISTS insight_derivation_link SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS snapshot_id ON insight_derivation_link TYPE string;
DEFINE FIELD IF NOT EXISTS left_kind ON insight_derivation_link TYPE string;
DEFINE FIELD IF NOT EXISTS left_id ON insight_derivation_link TYPE string;
DEFINE FIELD IF NOT EXISTS right_kind ON insight_derivation_link TYPE string;
DEFINE FIELD IF NOT EXISTS right_id ON insight_derivation_link TYPE string;
DEFINE FIELD IF NOT EXISTS relation ON insight_derivation_link TYPE string;
DEFINE FIELD IF NOT EXISTS rationale ON insight_derivation_link TYPE string;
DEFINE FIELD IF NOT EXISTS assessment_json ON insight_derivation_link TYPE string;
DEFINE FIELD IF NOT EXISTS evidence_json ON insight_derivation_link TYPE string;
DEFINE FIELD IF NOT EXISTS status ON insight_derivation_link TYPE string;
DEFINE FIELD IF NOT EXISTS revision ON insight_derivation_link TYPE string;
DEFINE INDEX IF NOT EXISTS insight_links_snapshot ON insight_derivation_link FIELDS snapshot_id;
DEFINE INDEX IF NOT EXISTS insight_links_left ON insight_derivation_link FIELDS left_kind, left_id;
DEFINE INDEX IF NOT EXISTS insight_links_right ON insight_derivation_link FIELDS right_kind, right_id;
DEFINE INDEX IF NOT EXISTS insight_links_left_id ON insight_derivation_link FIELDS left_id;
DEFINE INDEX IF NOT EXISTS insight_links_right_id ON insight_derivation_link FIELDS right_id;";

/// An article's source rows, through the `insight_source_call` index.
pub const SOURCES_BY_CALL_SURQL: &str = "SELECT VALUE fingerprint FROM insight_source
    WHERE run_id = $run AND answer_id = $answer AND call_id = $call";
/// Relations on either side of `$fp`, as two deletes so each uses its own
/// index (`insight_relation_left`, `insight_relation_right`).
pub const DELETE_RELATIONS_LEFT_SURQL: &str =
    "DELETE insight_relation WHERE left_fingerprint = $fp";
pub const DELETE_RELATIONS_RIGHT_SURQL: &str =
    "DELETE insight_relation WHERE right_fingerprint = $fp";

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
        tx.query(format!(
            "{DELETE_RELATIONS_LEFT_SURQL} RETURN NONE;
             {DELETE_RELATIONS_RIGHT_SURQL} RETURN NONE;
             DELETE type::record('insight_claim', $fp) RETURN NONE;
             IF $mem != NONE {{
               DELETE type::record('memory_graph_summary', $mem) RETURN NONE;
               DELETE type::record('memory', $mem) RETURN NONE;
               DELETE brain_graph_edge WHERE owner = $mem RETURN NONE;
             }};"
        ))
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
        tx.query(format!(
            "{SOURCES_BY_CALL_SURQL};
             DELETE insight_source
                WHERE run_id = $run AND answer_id = $answer AND call_id = $call RETURN NONE;"
        ))
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
    for (claim, [entity, _ns, predicate, object], fp) in staged {
        let mut r = run(
            &tx,
            tx.query("RETURN record::exists(type::record('insight_claim', $fp));")
                .bind(("fp", fp.clone())),
        )
        .await?;
        if take::<Option<bool>>(&mut r, 0)? != Some(true) {
            let memory_id = crate::store::new_id();
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
    upsert_relations(&tx, &relations).await?;
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
    let slot = outcome.clone();
    let write: MemoryWrite = Arc::new(move |tx| {
        let (run_id, article_id) = (run_id.clone(), article_id.clone());
        let (claims, relations, slot) = (claims.clone(), relations.clone(), slot.clone());
        Box::pin(async move {
            let (ids, out) = replace_in(tx, run_id, article_id, claims, relations).await?;
            // The last attempt to run is the one that committed.
            *slot.lock().expect("outcome lock") = out;
            Ok(ids)
        })
    });
    write_then_index(publisher, write).await?;
    let out = outcome.lock().expect("outcome lock").clone();
    Ok(out)
}

// ---------------------------------------------------------------------------
// publish_atlas_insights

use crate::brain::MemorySource;
use crate::store::{
    payload_revision, reject_reason, retag_source, PublicationReceipt, PublishOptions,
    RejectedClaim,
};

/// Tombstoned fingerprint -> run ids it was deleted from (`None`: none).
pub async fn tombstone_runs(tx: &Tx, fingerprint: &str) -> Result<Option<Vec<String>>, TxError> {
    let mut r = run(
        tx,
        tx.query("SELECT VALUE run_ids FROM memory_tombstone WHERE fingerprint = $fp AND fingerprint != '';")
            .bind(("fp", fingerprint.to_string())),
    )
    .await?;
    let rows: Vec<Vec<String>> = take(&mut r, 0)?;
    Ok((!rows.is_empty()).then(|| rows.into_iter().flatten().collect()))
}

#[derive(Debug, Clone, SurrealValue)]
struct MemoryHead {
    lifecycle: String,
    source_json: String,
}

async fn memory_head(tx: &Tx, id: &str) -> Result<Option<MemoryHead>, TxError> {
    let mut r = run(
        tx,
        tx.query(
            "SELECT lifecycle, source_json FROM memory WHERE id = type::record('memory', $id);",
        )
        .bind(("id", id.to_string())),
    )
    .await?;
    Ok(take::<Vec<MemoryHead>>(&mut r, 0)?.into_iter().next())
}

async fn create_memory(
    tx: &Tx,
    id: &str,
    text: &str,
    source_json: &str,
    kind: Option<&str>,
) -> Result<(), TxError> {
    run(
        tx,
        tx.query(
            "UPSERT type::record('memory', $id) SET text = $text, category = 'investigation',
               pinned = false, lifecycle = 'active', source_json = $src,
               memory_kind = $kind ?? memory_kind ?? 'claim' RETURN NONE;",
        )
        .bind(("id", id.to_string()))
        .bind(("text", text.to_string()))
        .bind(("src", source_json.to_string()))
        .bind(("kind", kind.map(str::to_string))),
    )
    .await?;
    Ok(())
}

/// `atlas_brief_id`: the run's brief memory (an Atlas memory of the run that
/// backs no claim).
pub(crate) async fn atlas_brief_id(tx: &Tx, run_id: &str) -> Result<Option<String>, TxError> {
    #[derive(Debug, Clone, SurrealValue)]
    struct Row {
        id: String,
        source_json: String,
    }
    let mut r = run(
        tx,
        tx.query(
            "LET $claimed = (SELECT VALUE memory_id FROM insight_claim);
             SELECT record::id(id) AS id, source_json FROM memory
               WHERE category = 'investigation' AND record::id(id) NOTINSIDE $claimed
               AND string::contains(source_json, $run) ORDER BY id;",
        )
        .bind(("run", run_id.to_string())),
    )
    .await?;
    let rows: Vec<Row> = take(&mut r, 1)?;
    Ok(rows.into_iter().find_map(|row| {
        let source: MemorySource = serde_json::from_str(&row.source_json).ok()?;
        (source.app == "atlas" && source.conversation_id == run_id).then_some(row.id)
    }))
}

fn json_err(e: serde_json::Error) -> TxError {
    TxError::Abort(format!("receipt serialization: {e}"))
}

async fn publish_in(
    tx: &Tx,
    run_id: &str,
    claims: &[AtlasInsightClaim],
    relations: &[(String, String, String)],
    brief: &str,
    options: &PublishOptions,
    revision: &str,
) -> Result<(PublicationReceipt, usize), TxError> {
    let answer_id = atlas_answer_id(run_id);
    let mut retagged = 0usize;
    let now = chrono::Utc::now().to_rfc3339();
    let mut receipt = PublicationReceipt {
        run_id: run_id.into(),
        revision: revision.into(),
        input_claims: claims.len(),
        ..Default::default()
    };
    let mut source = MemorySource {
        app: "atlas".into(),
        conversation_id: run_id.into(),
        message_id: None,
        reference: Some(run_id.into()),
    };
    if let Some(app) = &options.retag_app {
        source = retag_source(source, app, run_id);
    }
    let source_json = serde_json::to_string(&source).map_err(json_err)?;

    for (index, claim) in claims.iter().enumerate() {
        if let Some(reason) = reject_reason(claim) {
            receipt.rejected.push(RejectedClaim {
                index,
                article_id: claim.article_id.trim().into(),
                reason,
            });
            continue;
        }
        let entity = claim.entity.trim().to_ascii_lowercase();
        let predicate = claim.predicate.trim().to_ascii_lowercase();
        let object = claim.object.trim().to_ascii_lowercase();
        let sentence = claim.claim.trim();
        let fingerprint = atlas_claim_identity(claim);
        let mut r = run(
            tx,
            tx.query("SELECT VALUE memory_id FROM insight_claim WHERE id = type::record('insight_claim', $fp);")
                .bind(("fp", fingerprint.clone())),
        )
        .await?;
        let existing: Option<String> = take::<Vec<String>>(&mut r, 0)?.into_iter().next();
        if existing.is_none() {
            if let Some(runs) = tombstone_runs(tx, &fingerprint).await? {
                if options.repair || runs.iter().any(|r| r == run_id) {
                    receipt.rejected.push(RejectedClaim {
                        index,
                        article_id: claim.article_id.trim().into(),
                        reason: "deleted by user (tombstoned); not recreated".into(),
                    });
                    continue;
                }
            }
        }
        let memory_id = match existing {
            Some(memory_id) => {
                match memory_head(tx, &memory_id)
                    .await?
                    .filter(|m| m.lifecycle == "active")
                {
                    Some(head) => {
                        let staged_here = fingerprint.starts_with("v2:")
                            && !options.repair
                            && serde_json::from_str::<MemorySource>(&head.source_json).is_ok_and(
                                |s| {
                                    s.app == "atlas"
                                        && s.conversation_id == run_id
                                        && s.reference
                                            .as_deref()
                                            .is_some_and(|r| r.starts_with("isnap:atlas:"))
                                },
                            );
                        if let Some(app) = &options.retag_app {
                            let parsed: MemorySource = serde_json::from_str(&head.source_json)
                                .unwrap_or_else(|_| MemorySource {
                                    app: app.clone(),
                                    conversation_id: run_id.into(),
                                    message_id: None,
                                    reference: Some(run_id.into()),
                                });
                            let tagged = serde_json::to_string(&retag_source(parsed, app, run_id))
                                .map_err(json_err)?;
                            if tagged != head.source_json {
                                retagged += 1;
                                run(
                                    tx,
                                    tx.query("UPDATE type::record('memory', $id) SET source_json = $s RETURN NONE;")
                                        .bind(("id", memory_id.clone()))
                                        .bind(("s", tagged)),
                                )
                                .await?;
                            }
                        }
                        if staged_here && !receipt.created_memory_ids.contains(&memory_id) {
                            receipt.created += 1;
                            receipt.created_memory_ids.push(memory_id.clone());
                        } else {
                            receipt.reused += 1;
                        }
                        if !staged_here
                            && !receipt.reused_memory_ids.contains(&memory_id)
                            && !receipt.created_memory_ids.contains(&memory_id)
                        {
                            receipt.reused_memory_ids.push(memory_id.clone());
                        }
                    }
                    None => {
                        // Damaged link from an earlier failure: restore the
                        // canonical row from this authoritative claim text.
                        create_memory(tx, &memory_id, sentence, &source_json, None).await?;
                        receipt.repaired += 1;
                        receipt.repaired_memory_ids.push(memory_id.clone());
                    }
                }
                memory_id
            }
            None => {
                let memory_id = crate::store::new_id();
                let kind =
                    crate::memory_metadata::MemoryKind::from_classification(&claim.classification);
                create_memory(tx, &memory_id, sentence, &source_json, Some(kind.as_str())).await?;
                let mut r = run(
                    tx,
                    tx.query(
                        "CREATE type::record('insight_claim', $fp) SET memory_id = $mem,
                           entity_id = $entity, predicate = $pred, object_value = $obj,
                           topic = $topic, classification = $class, confidence = $conf,
                           created_at = $now, updated_at = $now RETURN NONE;
                         SELECT VALUE record::id(id) FROM insight_claim
                           WHERE entity_id = $entity AND predicate = $pred
                           AND id != type::record('insight_claim', $fp);",
                    )
                    .bind(("fp", fingerprint.clone()))
                    .bind(("mem", memory_id.clone()))
                    .bind(("entity", entity))
                    .bind(("pred", predicate))
                    .bind(("obj", object))
                    .bind(("topic", claim.topic.trim().to_string()))
                    .bind(("class", claim.classification.trim().to_string()))
                    .bind(("conf", claim.confidence))
                    .bind(("now", now.clone())),
                )
                .await?;
                let others: Vec<String> = take(&mut r, 1)?;
                let rels: Vec<_> = others
                    .into_iter()
                    .map(|old| (old, fingerprint.clone(), "conflict_or_revision".to_string()))
                    .collect();
                upsert_relations(tx, &rels).await?;
                receipt.created += 1;
                receipt.created_memory_ids.push(memory_id.clone());
                memory_id
            }
        };
        let article = claim.article_id.trim().to_string();
        run(
            tx,
            tx.query(
                "UPSERT type::record('insight_source', $key) MERGE {
                   fingerprint: $fp, run_id: $run, answer_id: $answer, call_id: $call,
                   source_url: source_url ?? $url, published_at: $pub } RETURN NONE;
                 UPDATE type::record('insight_claim', $fp) SET confidence = $conf,
                   source_reliability = $rel, info_credibility = $cred,
                   admiralty = $adm, rsp_status = $rsp, updated_at = $now RETURN NONE;",
            )
            .bind(("key", encode(&[&fingerprint, &answer_id, &article])))
            .bind(("fp", fingerprint.clone()))
            .bind(("run", run_id.to_string()))
            .bind(("answer", answer_id.clone()))
            .bind(("call", article))
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
        receipt.accepted_claims += 1;
        receipt.claim_memory_ids.push((fingerprint, memory_id));
    }
    upsert_relations(tx, relations).await?;
    let brief = brief.trim();
    if !brief.is_empty() && receipt.accepted_claims > 0 {
        if let Some(memory_id) = atlas_brief_id(tx, run_id).await? {
            let mut r = run(
                tx,
                tx.query(
                    "RETURN (UPDATE type::record('memory', $id) SET text = $text
                       WHERE text != $text RETURN VALUE id) != [];",
                )
                .bind(("id", memory_id.clone()))
                .bind(("text", brief.to_string())),
            )
            .await?;
            if take::<Option<bool>>(&mut r, 0)? == Some(true) {
                receipt.updated_memory_ids.push(memory_id.clone());
            }
            receipt.brief_memory_id = Some(memory_id);
        } else {
            let memory_id = crate::store::new_id();
            create_memory(tx, &memory_id, brief, &source_json, None).await?;
            receipt.brief_memory_id = Some(memory_id);
        }
    }
    let mut affected: Vec<String> = Vec::new();
    for id in receipt
        .claim_memory_ids
        .iter()
        .map(|(_, id)| id)
        .chain(receipt.brief_memory_id.iter())
    {
        if !affected.contains(id) {
            affected.push(id.clone());
        }
    }
    receipt.affected_memory_ids = affected;
    Ok((receipt, retagged))
}

async fn upsert_relations(tx: &Tx, relations: &[(String, String, String)]) -> Result<(), TxError> {
    for (left, right, relation) in relations {
        if left.is_empty() || right.is_empty() || relation.is_empty() || left == right {
            continue;
        }
        run(
            tx,
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
    Ok(())
}

/// `Store::publish_atlas_insights` on SurrealDB: claims, sources, relations,
/// the brief, the receipt row and the index work for every affected memory
/// commit in one publisher transaction. `queued_index_changes` and
/// `index_tasks` stay empty: SurrealDB index work is keyed by memory id
/// (`index_work`), not by outbox sequence or task.
pub async fn publish_atlas_insights(
    publisher: &Publisher,
    run_id: &str,
    claims: Vec<AtlasInsightClaim>,
    relations: Vec<(String, String, String)>,
    brief: &str,
    options: PublishOptions,
) -> Result<PublicationReceipt, TxError> {
    let revision = options
        .revision
        .clone()
        .unwrap_or_else(|| payload_revision(&claims, brief));
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let slot: Arc<Mutex<PublicationReceipt>> = Arc::default();
    let args = Arc::new((
        run_id.to_string(),
        claims,
        relations,
        brief.to_string(),
        options,
        revision,
    ));
    let nonce = format!("pub-{seq}-{}", chrono::Utc::now().timestamp_micros());
    let out = slot.clone();
    let w: super::publisher::Write = Arc::new(move |tx: Tx| {
        let (args, out, nonce) = (args.clone(), out.clone(), nonce.clone());
        Box::pin(async move {
            let (run_id, claims, relations, brief, options, revision) = &*args;
            let (mut receipt, retagged) =
                publish_in(&tx, run_id, claims, relations, brief, options, revision).await?;
            let changes =
                super::memory::index_changes_in(&tx, &receipt.affected_memory_ids, &nonce).await?;
            receipt.already_indexed = receipt
                .affected_memory_ids
                .len()
                .saturating_sub(changes.len());
            if !options.skip_receipt {
                run(
                    &tx,
                    tx.query(
                        "UPSERT type::record('atlas_publication', [$run, $rev]) SET run_id = $run,
                           revision = $rev, state = 'published', receipt_json = $json,
                           job_id = $job, created_at = created_at ?? $now, updated_at = $now RETURN NONE;",
                    )
                    .bind(("run", run_id.clone()))
                    .bind(("rev", revision.clone()))
                    .bind(("json", serde_json::to_string(&receipt).map_err(json_err)?))
                    .bind(("job", options.parent_job.clone().unwrap_or_default()))
                    .bind(("now", chrono::Utc::now().to_rfc3339())),
                )
                .await?;
            }
            // SQLite bumps only for a retag, a created or repaired claim, an
            // updated brief, or a brief at all.
            let changed = retagged > 0
                || receipt.created + receipt.repaired > 0
                || !receipt.updated_memory_ids.is_empty()
                || receipt.brief_memory_id.is_some();
            *out.lock().expect("receipt lock") = receipt;
            Ok(super::publisher::Wrote { changes, changed })
        })
    });
    publisher.publish(w).await?;
    let receipt = slot.lock().expect("receipt lock").clone();
    Ok(receipt)
}

// ---------------------------------------------------------------------------
// publish_snapshot

use crate::investigation::insights::{EvidenceSnapshot, CONTRACT, READINESS_POLICY};
use futures_util::future::BoxFuture;

/// Writes derivations for a newly inserted snapshot, or checks live
/// evidence, inside the publication transaction.
pub type SnapshotStep =
    Arc<dyn Fn(Tx, String) -> BoxFuture<'static, Result<(), TxError>> + Send + Sync>;

/// `desired_generation`: 0 when the owner has no state.
pub async fn desired_generation(tx: &Tx, kind: &str, owner: &str) -> Result<i64, TxError> {
    let mut r = run(
        tx,
        tx.query("SELECT VALUE desired_generation FROM insight_owner_state WHERE id = type::record('insight_owner_state', [$k, $o]);")
            .bind(("k", kind.to_string()))
            .bind(("o", owner.to_string())),
    )
    .await?;
    Ok(take::<Vec<i64>>(&mut r, 0)?.into_iter().next().unwrap_or(0))
}

/// `advance_generation`: start the owner's next generation (pending).
pub async fn advance_generation(
    tx: &Tx,
    kind: &str,
    owner: &str,
    now: &str,
) -> Result<i64, TxError> {
    let mut r = run(
        tx,
        tx.query(
            "UPSERT type::record('insight_owner_state', [$k, $o]) SET owner_kind = $k, owner_id = $o,
               desired_generation = IF desired_generation = NONE THEN 1 ELSE desired_generation + 1 END,
               lifecycle = 'pending', updated_at = $now RETURN VALUE desired_generation;",
        )
        .bind(("k", kind.to_string()))
        .bind(("o", owner.to_string()))
        .bind(("now", now.to_string())),
    )
    .await?;
    take::<Vec<i64>>(&mut r, 0)?
        .into_iter()
        .next()
        .ok_or_else(|| TxError::Abort("owner state upsert returned nothing".into()))
}

/// `publish_snapshot_on`: lane-independent canonical publication inside the
/// caller's transaction. `None` when the generation or input key moved on.
/// `validate` stands in for `validate_live_evidence`, whose owner and
/// source tables (articles, recon runs, intel reports) are still SQLite.
pub async fn publish_snapshot_on(
    tx: &Tx,
    snapshot: &EvidenceSnapshot,
    live_input_key: &str,
    lifecycle: &str,
    now: &str,
    validate: &SnapshotStep,
    publish: &SnapshotStep,
) -> Result<Option<String>, TxError> {
    if snapshot.owner_kind == "atlas" {
        return Err(TxError::Abort("Atlas insight publication retired".into()));
    }
    if desired_generation(tx, &snapshot.owner_kind, &snapshot.owner_id).await?
        != i64::from(snapshot.generation)
        || snapshot.key != live_input_key
    {
        return Ok(None);
    }
    if !["active", "insufficient_evidence", "blocked"].contains(&lifecycle) {
        return Err(TxError::Abort("invalid snapshot lifecycle".into()));
    }
    validate(tx.clone(), String::new()).await?;
    let snapshot_id = format!(
        "isnap:{}:{}:{}:{}",
        snapshot.owner_kind, snapshot.owner_id, snapshot.generation, snapshot.key
    );
    let mut r = run(
        tx,
        tx.query(
            "RETURN record::exists(type::record('insight_snapshot', $id))
               OR (SELECT VALUE id FROM insight_snapshot WHERE owner_kind = $k AND owner_id = $o
                   AND generation = $g AND input_key = $key LIMIT 1) != [];",
        )
        .bind(("id", snapshot_id.clone()))
        .bind(("k", snapshot.owner_kind.clone()))
        .bind(("o", snapshot.owner_id.clone()))
        .bind(("g", i64::from(snapshot.generation)))
        .bind(("key", snapshot.key.clone())),
    )
    .await?;
    if take::<Option<bool>>(&mut r, 0)? != Some(true) {
        run(
            tx,
            tx.query(
                "CREATE type::record('insight_snapshot', $id) SET owner_kind = $k, owner_id = $o,
                   generation = $g, input_key = $key, inputs_json = $inputs,
                   entities_json = $entities, coverage_json = $coverage,
                   readiness_json = $readiness, contract_version = $contract,
                   policy_version = $policy, created_at = $now RETURN NONE;",
            )
            .bind(("id", snapshot_id.clone()))
            .bind(("k", snapshot.owner_kind.clone()))
            .bind(("o", snapshot.owner_id.clone()))
            .bind(("g", i64::from(snapshot.generation)))
            .bind(("key", snapshot.key.clone()))
            .bind(("inputs", serde_json::to_string(snapshot).map_err(json_err)?))
            .bind((
                "entities",
                serde_json::to_string(&snapshot.entities).map_err(json_err)?,
            ))
            .bind((
                "coverage",
                serde_json::to_string(&snapshot.coverage_by_stage).map_err(json_err)?,
            ))
            .bind((
                "readiness",
                serde_json::json!({ "lifecycle": lifecycle }).to_string(),
            ))
            .bind(("contract", CONTRACT.to_string()))
            .bind(("policy", READINESS_POLICY.to_string()))
            .bind(("now", now.to_string())),
        )
        .await?;
        publish(tx.clone(), snapshot_id.clone()).await?;
    }
    run(
        tx,
        tx.query(
            "UPDATE insight_owner_state SET active_snapshot_id = $id, lifecycle = $life,
               last_error = '', updated_at = $now
             WHERE id = type::record('insight_owner_state', [$k, $o]) AND desired_generation = $g
             RETURN NONE;",
        )
        .bind(("id", snapshot_id.clone()))
        .bind(("life", lifecycle.to_string()))
        .bind(("now", now.to_string()))
        .bind(("k", snapshot.owner_kind.clone()))
        .bind(("o", snapshot.owner_id.clone()))
        .bind(("g", i64::from(snapshot.generation))),
    )
    .await?;
    Ok(Some(snapshot_id))
}

/// `publish_snapshot`: the insight-refresh lease fence around
/// [`publish_snapshot_on`], with task completion in the same transaction.
/// `Ok(false)` when the lease was lost, the job was cancelled or the
/// generation moved on; nothing is written then.
#[allow(clippy::too_many_arguments)]
pub async fn publish_snapshot(
    db: &Surreal<Any>,
    claimed: &super::jobs::ClaimedTask,
    snapshot: &EvidenceSnapshot,
    live_input_key: &str,
    lifecycle: &str,
    now: chrono::DateTime<chrono::Utc>,
    validate: SnapshotStep,
    publish: SnapshotStep,
) -> Result<bool, TxError> {
    use super::tx::{run_tx, RetryPolicy};
    let now_s = super::jobs::ts(now);
    let committed = run_tx(db, RetryPolicy::default(), |tx| {
        let (now_s, validate, publish) = (now_s.clone(), validate.clone(), publish.clone());
        async move {
            if claimed.operation != "insight_refresh" {
                return Ok(false);
            }
            super::jobs::bump(&tx, "job_guard", &claimed.root_id).await?;
            let Some(f) = super::jobs::fence(&tx, claimed, &now_s).await? else {
                return Ok(false);
            };
            if !super::jobs::holds(&f, claimed) || f.cancelled {
                return Ok(false);
            }
            let Some(id) = publish_snapshot_on(
                &tx,
                snapshot,
                live_input_key,
                lifecycle,
                &now_s,
                &validate,
                &publish,
            )
            .await?
            else {
                return Ok(false);
            };
            super::jobs::complete_in(&tx, claimed, &id, &now_s).await?;
            Ok(true)
        }
    })
    .await?;
    Ok(committed.value)
}

// ---------------------------------------------------------------------------
// delete_memory

/// `tombstone_memory`: record a user-deletion tombstone for a memory and each
/// claim fingerprint it backs, with the runs that claim came from.
pub async fn tombstone_memory(tx: &Tx, memory_id: &str, reason: &str) -> Result<(), TxError> {
    run(
        tx,
        tx.query(
            "LET $fps = (SELECT VALUE record::id(id) FROM insight_claim WHERE memory_id = $mem);
             IF $fps = [] {
               UPSERT type::record('memory_tombstone', [$mem, '']) SET memory_id = $mem,
                 fingerprint = '', reason = $reason, run_ids = [], created_at = $now RETURN NONE;
             };
             FOR $fp IN $fps {
               UPSERT type::record('memory_tombstone', [$mem, $fp]) SET memory_id = $mem,
                 fingerprint = $fp, reason = $reason, created_at = $now,
                 run_ids = array::distinct((SELECT VALUE run_id FROM insight_source
                   WHERE fingerprint = $fp AND run_id != NONE)) RETURN NONE;
             };",
        )
        .bind(("mem", memory_id.to_string()))
        .bind(("reason", reason.to_string()))
        .bind(("now", chrono::Utc::now().to_rfc3339())),
    )
    .await?;
    Ok(())
}

/// Item ids (`item_json.id`) of the derivations backed by `memory_id`.
async fn derivation_item_ids(tx: &Tx, memory_id: &str) -> Result<Vec<String>, TxError> {
    let mut r = run(
        tx,
        tx.query("SELECT VALUE item_json FROM insight_derivation WHERE memory_id = $mem;")
            .bind(("mem", memory_id.to_string())),
    )
    .await?;
    let items: Vec<String> = take(&mut r, 0)?;
    Ok(items
        .iter()
        .filter_map(|j| serde_json::from_str::<serde_json::Value>(j).ok())
        .filter_map(|v| v.get("id").and_then(|id| id.as_str()).map(str::to_string))
        .collect())
}

async fn delete_memory_in(tx: &Tx, id: &str) -> Result<bool, TxError> {
    let mut r = run(
        tx,
        tx.query("RETURN record::exists(type::record('memory', $id));")
            .bind(("id", id.to_string())),
    )
    .await?;
    let exists = take::<Option<bool>>(&mut r, 0)? == Some(true);
    if exists {
        tombstone_memory(tx, id, "user_delete").await?;
    }
    let items = derivation_item_ids(tx, id).await?;
    let mut r = run(
        tx,
        tx.query(
            "LET $derived = (SELECT VALUE derivation_id FROM insight_derivation_evidence WHERE memory_id = $mem);
             -- Record ids, so these are direct fetches rather than a scan.
             LET $drids = array::map($derived, |$d| type::record('insight_derivation', $d));
             LET $snaps = (SELECT VALUE snapshot_id FROM $drids);
             FOR $s IN $snaps {
               UPDATE insight_owner_state SET lifecycle = 'previous', last_error = 'memory premise deleted'
                 WHERE active_snapshot_id = $s RETURN NONE;
             };
             -- One equality per item keeps each statement on its index.
             FOR $item IN $items {
               LET $premised = (SELECT VALUE derivation_id FROM insight_derivation_evidence WHERE premise_id = $item);
               UPDATE array::map($premised, |$d| type::record('insight_derivation', $d))
                 SET status = 'blocked_premise' RETURN NONE;
               UPDATE insight_derivation_link SET status = 'deleted_endpoint' WHERE left_id = $item RETURN NONE;
               UPDATE insight_derivation_link SET status = 'deleted_endpoint' WHERE right_id = $item RETURN NONE;
             };
             LET $blocked = (SELECT VALUE snapshot_id FROM insight_derivation WHERE status = 'blocked_premise');
             FOR $s IN $blocked {
               UPDATE insight_owner_state SET lifecycle = 'previous', last_error = 'claim premise deleted'
                 WHERE active_snapshot_id = $s RETURN NONE;
             };
             UPDATE insight_derivation SET memory_id = NONE, status = 'deleted'
               WHERE memory_id = $mem RETURN NONE;
             UPDATE insight_derivation_evidence SET memory_id = NONE WHERE memory_id = $mem RETURN NONE;
             LET $fps = (SELECT VALUE record::id(id) FROM insight_claim WHERE memory_id = $mem);
             -- One equality per fingerprint keeps each delete on its index.
             FOR $fp IN $fps {
               DELETE insight_source WHERE fingerprint = $fp RETURN NONE;
               DELETE insight_relation WHERE left_fingerprint = $fp RETURN NONE;
               DELETE insight_relation WHERE right_fingerprint = $fp RETURN NONE;
             };
             DELETE insight_claim WHERE memory_id = $mem RETURN NONE;
             DELETE type::record('memory_graph_summary', $mem) RETURN NONE;
             DELETE brain_graph_edge WHERE owner = $mem RETURN NONE;
             RETURN (DELETE type::record('memory', $mem) RETURN BEFORE) != [];",
        )
        .bind(("mem", id.to_string()))
        .bind(("items", items)),
    )
    .await?;
    let last = r.num_statements() - 1;
    Ok(take::<Option<bool>>(&mut r, last)? == Some(true))
}

/// `Store::delete_memory` on SurrealDB: tombstone, provenance downgrade,
/// insight cleanup, the memory delete and its removal index work in one
/// publisher transaction. The NLP cache (still SQLite) and the job
/// retention sweep (`jobs::cleanup_batch`, its own idempotent transaction)
/// stay with the caller.
pub async fn delete_memory(publisher: &Publisher, id: &str) -> Result<bool, TxError> {
    let deleted = Arc::new(Mutex::new(false));
    let (id, slot) = (id.to_string(), deleted.clone());
    let write: MemoryWrite = Arc::new(move |tx| {
        let (id, slot) = (id.clone(), slot.clone());
        Box::pin(async move {
            let gone = delete_memory_in(&tx, &id).await?;
            *slot.lock().expect("delete lock") = gone;
            Ok(if gone { vec![id] } else { Vec::new() })
        })
    });
    write_then_index(publisher, write).await?;
    let gone = *deleted.lock().expect("delete lock");
    Ok(gone)
}
