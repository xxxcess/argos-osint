//! Canonical Brain publication of a completed intel brief kind on SurrealDB
//! (SQLite `brief_canonical::publish_stage_on`).
//!
//! Claims and Inferences publish an article insight snapshot through the
//! shared canonical publisher ([`super::insight::publish_snapshot_on`] with
//! [`super::derivations::derivation_step`] scoped to the completed kind), so
//! a brief lands as the same Claim/Inference memories, sources and index work
//! as an insight refresh. Links publish each metadata lead as a tentative
//! Related memory (inference → candidate article), never as corroboration.
//!
//! Canonical failure never fails a completed stage: the publication
//! transaction rolls back and the returned [`IndexState`] names the reason.

use std::sync::Arc;

use anyhow::Result;
use serde_json::Value;
use surrealdb::types::SurrealValue;

use super::jobs::{rid, run, take};
use super::outbox::{self, NewChange};
use super::repo::Repository;
use super::tx::{run_tx, RetryPolicy, Tx, TxError};
use crate::intel_recon::brief_canonical::{
    self, CanonicalBrief, IndexReceipt, IndexState, PUBLICATION_VERSION,
};
use crate::memory_metadata::{EndpointRef, MemoryKind, MemoryPayload, RelatedPayload};

fn abort(e: impl std::fmt::Display) -> TxError {
    TxError::Abort(e.to_string())
}

/// App-state key holding the Related lead memories of one article.
fn leads_key(article_id: &str) -> String {
    format!("intel-brief-brain-leads:{article_id}")
}

async fn stage_receipt(
    repo: &Repository,
    job_id: &str,
    operation: &str,
) -> Result<Option<IndexState>> {
    let mut r = repo
        .db()
        .query("RETURN type::record('intel_brief_stage_receipt', [$j, $op]).state_json;")
        .bind(("j", job_id.to_string()))
        .bind(("op", operation.to_string()))
        .await?
        .check()?;
    let raw: Option<String> = r.take(0usize)?;
    Ok(raw.and_then(|s| serde_json::from_str(&s).ok()))
}

async fn article_state(repo: &Repository, article_id: &str) -> Result<Option<IndexState>> {
    let mut r = repo
        .db()
        .query("RETURN type::record('intel_brief_index_state', $a).state_json;")
        .bind(("a", article_id.to_string()))
        .await?
        .check()?;
    let raw: Option<String> = r.take(0usize)?;
    Ok(raw.and_then(|s| serde_json::from_str(&s).ok()))
}

/// Prepare canonical inputs for `operation` from the retained article input.
pub async fn prepare(
    repo: &Repository,
    article_id: &str,
    body: &Value,
    claims: &Value,
    inferences: &Value,
    operation: &str,
) -> Result<std::result::Result<CanonicalBrief, String>> {
    let Some(article) = super::atlas::latest_article_by_id(repo.db(), article_id).await? else {
        return Ok(Err("article is no longer in Atlas".into()));
    };
    let generation = super::insight_owner::desired_generation(repo, "article", article_id).await?;
    let input =
        super::pipeline_inputs::article_input(repo, &article, generation, "intel brief").await?;
    Ok(brief_canonical::prepare_for_stage(
        input, &article, body, claims, inferences, operation,
    ))
}

/// Publish one completed kind and return its index state. The caller binds
/// the state as the stage receipt and the article's footer state.
pub async fn publish_stage(
    repo: &Repository,
    article_id: &str,
    job_id: &str,
    operation: &str,
    prepared: &std::result::Result<CanonicalBrief, String>,
    snapshot_hash: &str,
) -> Result<IndexState> {
    if let Some(existing) = stage_receipt(repo, job_id, operation).await? {
        if existing.publication_version == PUBLICATION_VERSION
            && existing.state == "indexed"
            && existing.job_id == job_id
        {
            return Ok(existing);
        }
    }
    let previous = article_state(repo, article_id)
        .await?
        .filter(|s| s.job_id == job_id);
    let mut state = IndexState {
        publication_version: PUBLICATION_VERSION,
        job_id: job_id.into(),
        snapshot_id: previous
            .as_ref()
            .map(|s| s.snapshot_id.clone())
            .unwrap_or_default(),
        required: previous.map(|s| s.required).unwrap_or_default(),
        ..Default::default()
    };
    let brief = match prepared {
        Err(reason) => {
            state.state = "not_indexed".into();
            state.reason = reason.clone();
            return Ok(state);
        }
        Ok(brief) if !brief.output.readiness.publication_eligible => {
            state.state = "not_indexed".into();
            state.reason = brief.output.readiness.gaps.join("; ");
            return Ok(state);
        }
        Ok(brief) => brief.clone(),
    };
    let attempt = if operation == "intel_brief_links" {
        publish_links(repo, article_id, job_id).await
    } else {
        publish_kind(repo, article_id, operation, brief, snapshot_hash).await
    };
    match attempt {
        Ok(Some((id, required))) => {
            state.state = "indexed".into();
            state.snapshot_id = id;
            state.required = required;
        }
        Ok(None) => {
            state.state = "not_indexed".into();
            state.reason = "stale canonical generation".into();
        }
        Err(error) => {
            state.state = "failed".into();
            state.reason = error.to_string();
        }
    }
    Ok(state)
}

type Published = Option<(String, Vec<IndexReceipt>)>;

async fn publish_kind(
    repo: &Repository,
    article_id: &str,
    operation: &str,
    brief: CanonicalBrief,
    snapshot_hash: &str,
) -> Result<Published> {
    let kind: &'static str = if operation == "intel_brief_claims" {
        "claim"
    } else {
        "inference"
    };
    let (brief, hash, article) = (
        Arc::new(brief),
        snapshot_hash.to_string(),
        article_id.to_string(),
    );
    let committed = run_tx(repo.db(), RetryPolicy::default(), |tx| {
        let (brief, hash, article) = (brief.clone(), hash.clone(), article.clone());
        async move {
            let now = super::jobs::ts(chrono::Utc::now());
            let mut input = brief.input.clone();
            let mut output = brief.output.clone();
            let generation =
                super::insight::advance_generation(&tx, "article", &article, &now).await?;
            input.snapshot.generation = u32::try_from(generation).map_err(abort)?;
            input.snapshot.key = crate::nlp::cache::key(&serde_json::json!([
                crate::intel_recon::brief_pipeline::CANONICAL_CONTRACT,
                input.snapshot.source_refs,
                hash
            ]));
            for inference in &mut output.inferences {
                inference.snapshot_key = input.snapshot.key.clone();
            }
            let validate = super::live_evidence::step(input.snapshot.clone());
            let save = input.save_insights;
            let key = input.snapshot.key.clone();
            let snapshot = input.snapshot.clone();
            let publish = super::derivations::derivation_step(
                Arc::new(input),
                Arc::new(output),
                Some(kind),
                super::derivations::DerivationContext {
                    save_insights: save,
                    recon_thread: None,
                },
            );
            let Some(id) = super::insight::publish_snapshot_on(
                &tx, &snapshot, &key, "active", &now, &validate, &publish,
            )
            .await?
            else {
                return Ok(None);
            };
            let required = required_receipts(&tx, &id, &article).await?;
            Ok(Some((id, required)))
        }
    })
    .await?;
    Ok(committed.value)
}

#[derive(SurrealValue)]
struct DerivationRow {
    memory_id: Option<String>,
    item_json: String,
}

/// Memory ids (with their content revisions) the publication requires the
/// index to acknowledge: the snapshot's derivations plus the article's
/// Related lead memories.
async fn required_receipts(
    tx: &Tx,
    snapshot_id: &str,
    article_id: &str,
) -> Result<Vec<IndexReceipt>, TxError> {
    let mut r = run(
        tx,
        tx.query(
            "SELECT memory_id, item_json FROM insight_derivation
               WHERE snapshot_id = $s AND memory_id != NONE;",
        )
        .bind(("s", snapshot_id.to_string())),
    )
    .await?;
    let rows: Vec<DerivationRow> = take(&mut r, 0)?;
    let mut ids: Vec<String> = rows.into_iter().filter_map(|d| d.memory_id).collect();
    ids.extend(lead_ids(tx, article_id).await?);
    ids.sort();
    ids.dedup();
    let mut out = Vec::new();
    for id in ids {
        let mut r = run(
            tx,
            tx.query("RETURN (SELECT VALUE text FROM ONLY $m);")
                .bind(("m", rid("memory", &id))),
        )
        .await?;
        if let Some(text) = take::<Option<String>>(&mut r, 0)? {
            out.push(IndexReceipt {
                revision: super::memory::memory_revision(&text),
                memory_id: id,
            });
        }
    }
    Ok(out)
}

async fn lead_ids(tx: &Tx, article_id: &str) -> Result<Vec<String>, TxError> {
    let mut r = run(
        tx,
        tx.query("RETURN (SELECT VALUE value FROM ONLY type::record('app_state', $k));")
            .bind(("k", leads_key(article_id))),
    )
    .await?;
    let raw: Option<String> = take(&mut r, 0)?;
    Ok(raw
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default())
}

#[derive(SurrealValue)]
struct LeadRow {
    id: String,
    inference_id: String,
    candidate_article_id: String,
    candidate_run_id: String,
    candidate_metadata_revision: String,
    interpretation: String,
    verification_needed: String,
}

/// Links: requires the job's Inference publication to be indexed and still
/// the article's active snapshot; every lead's inference must be a live
/// canonical memory. Each lead becomes a tentative Related memory; leads the
/// article no longer has are retired.
async fn publish_links(repo: &Repository, article_id: &str, job_id: &str) -> Result<Published> {
    let inference = stage_receipt(repo, job_id, "intel_brief_inferences")
        .await?
        .filter(|s| s.state == "indexed");
    let Some(inference) = inference else {
        anyhow::bail!("canonical inference publication unavailable");
    };
    let (article, job) = (article_id.to_string(), job_id.to_string());
    let snapshot = inference.snapshot_id.clone();
    let committed = run_tx(repo.db(), RetryPolicy::default(), |tx| {
        let (article, job, snapshot) = (article.clone(), job.clone(), snapshot.clone());
        async move {
            let now = chrono::Utc::now();
            let mut r = run(
                &tx,
                tx.query(
                    "SELECT VALUE active_snapshot_id FROM insight_owner_state
                       WHERE owner_kind = 'article' AND owner_id = $o;
                     SELECT memory_id, item_json FROM insight_derivation
                       WHERE snapshot_id = $s AND insight_kind = 'inference'
                         AND status = 'assessed' AND memory_id != NONE;
                     SELECT meta::id(id) AS id, inference_id, candidate_article_id,
                       candidate_run_id, candidate_metadata_revision, interpretation,
                       verification_needed FROM intel_inference_article_lead
                       WHERE article_id = $o AND job_id = $j ORDER BY id;",
                )
                .bind(("o", article.clone()))
                .bind(("s", snapshot.clone()))
                .bind(("j", job.clone())),
            )
            .await?;
            let active: Vec<Option<String>> = take(&mut r, 0)?;
            if active.into_iter().flatten().next().as_deref() != Some(snapshot.as_str()) {
                return Err(abort(
                    "canonical inference snapshot was replaced before Links publication",
                ));
            }
            let derivations: Vec<DerivationRow> = take(&mut r, 1)?;
            let leads: Vec<LeadRow> = take(&mut r, 2)?;
            let memory_of = |inference_id: &str| {
                derivations.iter().find_map(|d| {
                    let item: Value = serde_json::from_str(&d.item_json).ok()?;
                    (item["id"] == inference_id)
                        .then(|| d.memory_id.clone())
                        .flatten()
                })
            };
            let mut current = Vec::new();
            for lead in leads {
                let memory_id = memory_of(&lead.inference_id).ok_or_else(|| {
                    abort("a Link endpoint lacks its current canonical inference")
                })?;
                let mut r = run(
                    &tx,
                    tx.query(
                        "RETURN (SELECT VALUE memory_kind = 'inference' AND lifecycle = 'active'
                           FROM ONLY $m) ?? false;",
                    )
                    .bind(("m", rid("memory", &memory_id))),
                )
                .await?;
                if take::<Option<bool>>(&mut r, 0)? != Some(true) {
                    return Err(abort(
                        "a Link endpoint lacks its current canonical inference",
                    ));
                }
                if super::atlas::article_in(&tx, &lead.candidate_run_id, &lead.candidate_article_id)
                    .await?
                    .is_none()
                {
                    continue;
                }
                let payload = RelatedPayload {
                    source: EndpointRef::Memory {
                        memory_id,
                        kind: MemoryKind::Inference,
                    },
                    target: EndpointRef::Article {
                        run_id: lead.candidate_run_id,
                        article_id: lead.candidate_article_id,
                        revision: lead.candidate_metadata_revision.clone(),
                    },
                    shared_actors: vec![],
                    basis: format!("intel-brief:{article}:metadata-lead"),
                    interpretation: lead.interpretation,
                    verification_gap: lead.verification_needed,
                    source_revisions: vec![
                        lead.candidate_metadata_revision,
                        format!("intel-brief-lead:{}", lead.id),
                    ],
                };
                let mut keys = [
                    crate::brain_actors::endpoint_key(&payload.source),
                    crate::brain_actors::endpoint_key(&payload.target),
                ];
                keys.sort();
                let id = crate::brain_actors::hid("related", &[&keys[0], &keys[1], &payload.basis]);
                let payload = MemoryPayload::Related(payload);
                super::memory::put_typed_in(
                    &tx,
                    &super::memory::TypedMemory {
                        id: id.clone(),
                        text: crate::brain_actors::text_of(&payload),
                        kind: MemoryKind::Related,
                        payload: Some(payload),
                        lifecycle: "active".into(),
                    },
                )
                .await?;
                current.push(id);
            }
            let previous = lead_ids(&tx, &article).await?;
            let stale: Vec<String> = previous
                .into_iter()
                .filter(|id| !current.contains(id))
                .collect();
            let retired = super::memory::set_lifecycle_in(&tx, &stale, "retired").await?;
            run(
                &tx,
                tx.query("UPSERT type::record('app_state', $k) SET value = $v RETURN NONE;")
                    .bind(("k", leads_key(&article)))
                    .bind(("v", serde_json::to_string(&current).map_err(abort)?)),
            )
            .await?;
            let mut touched = current.clone();
            touched.extend(retired);
            enqueue_index(&tx, &touched, &format!("brief-leads:{job}"), now).await?;
            let required = required_receipts(&tx, &snapshot, &article).await?;
            Ok(Some((snapshot, required)))
        }
    })
    .await?;
    Ok(committed.value)
}

async fn enqueue_index(
    tx: &Tx,
    ids: &[String],
    nonce: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<(), TxError> {
    let changes = super::memory::index_changes_in(tx, ids, nonce).await?;
    let new: Vec<(NewChange<'_>, &str)> = changes
        .iter()
        .map(|c| {
            (
                NewChange {
                    record_kind: &c.record_kind,
                    record_id: &c.record_id,
                    revision: &c.revision,
                    operation: &c.operation,
                    generation: &c.generation,
                    content_hash: &c.content_hash,
                    task_id: "",
                    job_id: "",
                },
                c.op_key.as_str(),
            )
        })
        .collect();
    outbox::enqueue_many_in(tx, &new, None, now).await?;
    Ok(())
}
