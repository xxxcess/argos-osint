//! Canonical Brain publication of a completed intel brief kind on SurrealDB
//! (legacy store `brief_canonical::publish_stage_on`).
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

/// Whether a committed stage receipt already indexes exactly this
/// publication: the retry path reuses it and never re-extracts.
pub fn publication_current(existing: &IndexState, job_id: &str) -> bool {
    existing.publication_version == PUBLICATION_VERSION
        && existing.state == "indexed"
        && existing.job_id == job_id
}

#[cfg(test)]
mod publication_retry_tests {
    use super::*;

    fn receipt(state: &str, job: &str, version: u32) -> IndexState {
        IndexState {
            publication_version: version,
            state: state.into(),
            job_id: job.into(),
            ..Default::default()
        }
    }

    /// A retry after a committed publication reuses the receipt; a failed
    /// state, another job's receipt or an old publication version re-runs.
    #[test]
    fn retry_reuses_only_the_current_indexed_receipt() {
        assert!(publication_current(
            &receipt("indexed", "job-1", PUBLICATION_VERSION),
            "job-1"
        ));
        assert!(!publication_current(
            &receipt("failed", "job-1", PUBLICATION_VERSION),
            "job-1"
        ));
        assert!(!publication_current(
            &receipt("not_indexed", "job-1", PUBLICATION_VERSION),
            "job-1"
        ));
        assert!(!publication_current(
            &receipt("indexed", "job-2", PUBLICATION_VERSION),
            "job-1"
        ));
        assert!(!publication_current(
            &receipt("indexed", "job-1", PUBLICATION_VERSION - 1),
            "job-1"
        ));
    }
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

pub const CAPTURED_SOURCE_CHANGED: &str = "captured source changed before publication";

/// Current publication uses the exact root occurrence/metadata snapshot rather
/// than rebinding old faithful spans to a newer mutable article row.
pub async fn prepare_captured(
    repo: &Repository,
    article_id: &str,
    metadata: &Value,
    body: &Value,
    claims: &Value,
    inferences: &Value,
    operation: &str,
) -> Result<std::result::Result<CanonicalBrief, String>> {
    if !metadata.is_object() {
        return prepare(repo, article_id, body, claims, inferences, operation).await;
    }
    let run_id = metadata["run_id"].as_str();
    let article = match run_id {
        Some(run) => super::atlas::article(repo.db(), run, article_id).await?,
        None => super::atlas::latest_article_by_id(repo.db(), article_id).await?,
    };
    let Some(article) = article else {
        return Ok(Err(CAPTURED_SOURCE_CHANGED.into()));
    };
    let current = crate::intel_recon::brief_stages::metadata_snapshot(&article);
    for field in [
        "title",
        "description",
        "published_at",
        "url",
        "source_name",
        "source_domain",
        "country",
        "run_id",
        "article_id",
        "source_revision",
    ] {
        if metadata.get(field).is_some() && metadata[field] != current[field] {
            return Ok(Err(CAPTURED_SOURCE_CHANGED.into()));
        }
    }
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
        if publication_current(&existing, job_id) {
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
        publish_links(repo, article_id, job_id, brief).await
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
                PUBLICATION_VERSION,
                input.snapshot.source_refs,
                input.snapshot.entities,
                hash
            ]));
            for inference in &mut output.inferences {
                inference.snapshot_key = input.snapshot.key.clone();
            }
            let validate = super::live_evidence::publication_step(input.snapshot.clone());
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
    ids.extend(curated_ids(tx, article_id).await?);
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
async fn publish_links(
    repo: &Repository,
    article_id: &str,
    job_id: &str,
    prepared: CanonicalBrief,
) -> Result<Published> {
    let inference = stage_receipt(repo, job_id, "intel_brief_inferences")
        .await?
        .filter(|s| s.state == "indexed");
    let Some(inference) = inference else {
        anyhow::bail!("canonical inference publication unavailable");
    };
    let (article, job) = (article_id.to_string(), job_id.to_string());
    let snapshot = inference.snapshot_id.clone();
    let prepared = Arc::new(prepared);
    // Unique per publication, fixed across transaction retries: a later
    // Links publication of the same job must not reuse an op_key.
    let nonce = format!(
        "brief-leads:{job_id}:{}",
        chrono::Utc::now().timestamp_micros()
    );
    let committed = run_tx(repo.db(), RetryPolicy::default(), |tx| {
        let (article, job, snapshot, nonce, prepared) = (
            article.clone(),
            job.clone(),
            snapshot.clone(),
            nonce.clone(),
            prepared.clone(),
        );
        async move {
            // Recheck the retained original source inside the same transaction
            // as all tentative/confirmed writes and lifecycle/index changes.
            super::live_evidence::publication_step(prepared.input.snapshot.clone())(
                tx.clone(),
                String::new(),
            )
            .await?;
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
                let Some(target) = super::atlas::article_in(
                    &tx,
                    &lead.candidate_run_id,
                    &lead.candidate_article_id,
                )
                .await?
                else {
                    continue;
                };
                if crate::atlas_metadata::input_hash(&target) != lead.candidate_metadata_revision {
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
            let confirmed =
                publish_curated_links_in(&tx, &article, &job, &derivations, &prepared).await?;
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
            touched.extend(confirmed);
            touched.extend(retired);
            enqueue_index(&tx, &touched, &nonce, now).await?;
            let required = required_receipts(&tx, &snapshot, &article).await?;
            Ok(Some((snapshot, required)))
        }
    })
    .await?;
    Ok(committed.value)
}

fn curated_key(article: &str) -> String {
    format!("intel-brief-curated-links:{article}")
}
async fn curated_ids(tx: &Tx, article: &str) -> Result<Vec<String>, TxError> {
    let mut response = run(
        tx,
        tx.query("SELECT VALUE value FROM ONLY type::record('app_state', $key)")
            .bind(("key", curated_key(article))),
    )
    .await?;
    let raw: Option<String> = take(&mut response, 0)?;
    let Some(raw) = raw else {
        return Ok(Vec::new());
    };
    if raw.len() > 64 * 1024 {
        return Err(abort("curated Link owner receipt exceeds bound"));
    }
    let ids: Vec<String> = serde_json::from_str(&raw).map_err(abort)?;
    if ids.len() > 200 {
        return Err(abort("curated Link owner exceeds200 records"));
    }
    Ok(ids)
}
#[derive(SurrealValue)]
struct CuratedMemory {
    text: String,
    payload_json: String,
    memory_kind: String,
    lifecycle: String,
}
async fn curated_memory(tx: &Tx, id: &str) -> Result<Option<CuratedMemory>, TxError> {
    let mut result = run(
        tx,
        tx.query("SELECT text,payload_json,memory_kind,lifecycle FROM ONLY $memory")
            .bind(("memory", rid("memory", id))),
    )
    .await?;
    take(&mut result, 0)
}
fn endpoint_revision(memory: &CuratedMemory) -> Result<String, TxError> {
    let payload: MemoryPayload = serde_json::from_str(&memory.payload_json).map_err(abort)?;
    crate::memory_metadata::assertion_semantic_revision(&memory.text, &payload).map_err(abort)
}

async fn still_live_curated(
    tx: &Tx,
    payload: &crate::memory_metadata::LinkPayload,
    current: &[DerivationRow],
    prepared: &CanonicalBrief,
) -> Result<bool, TxError> {
    let Some(receipt) = &payload.validation else {
        return Ok(false);
    };
    if receipt.validate_structure().is_err()
        || payload.relation != receipt.kind.as_str()
        || payload.source != receipt.source
        || payload.target != receipt.target
    {
        return Ok(false);
    }
    let EndpointRef::Memory {
        memory_id,
        kind: MemoryKind::Inference,
    } = &payload.source
    else {
        return Ok(false);
    };
    if !current
        .iter()
        .any(|row| row.memory_id.as_ref() == Some(memory_id))
    {
        return Ok(false);
    }
    let Some(source) = curated_memory(tx, memory_id).await? else {
        return Ok(false);
    };
    if source.lifecycle != "active"
        || source.memory_kind != "inference"
        || endpoint_revision(&source)? != receipt.source_revision
    {
        return Ok(false);
    }
    let EndpointRef::Article {
        run_id,
        article_id,
        revision,
    } = &payload.target
    else {
        return Ok(false);
    };
    let Some(target) = super::atlas::article_in(tx, run_id, article_id).await? else {
        return Ok(false);
    };
    if crate::atlas_metadata::input_hash(&target) != *revision
        || receipt.target_revision != *revision
    {
        return Ok(false);
    }
    let published = chrono::DateTime::parse_from_rfc3339(&target.published_at)
        .ok()
        .map(|_| target.published_at.clone());
    if receipt.target_time != published {
        return Ok(false);
    }
    let target_source = format!("article:{run_id}/{article_id}");
    let sources: Vec<_> = receipt
        .evidence
        .iter()
        .filter(|reference| reference.source_id != target_source)
        .collect();
    if sources.is_empty()
        || !sources.iter().all(|reference| {
            prepared
                .input
                .evidence
                .iter()
                .any(|evidence| evidence.validate_ref(reference))
        })
    {
        return Ok(false);
    }
    let mut result = run(
        tx,
        tx.query("SELECT VALUE value FROM ONLY $receipt")
            .bind(("receipt", rid("brief_receipt", &receipt.decision_receipt))),
    )
    .await?;
    Ok(take::<Option<String>>(&mut result, 0)?.as_deref() == Some("faithful"))
}

/// Every current typed Link write passes this live transaction gate. Failed
/// proposals do not replace or retire unrelated still-valid confirmed records.
async fn publish_curated_links_in(
    tx: &Tx,
    article: &str,
    job: &str,
    derivations: &[DerivationRow],
    prepared: &CanonicalBrief,
) -> Result<Vec<String>, TxError> {
    use crate::intel_recon::brief_stages::{self as stages, CuratedLinkProof, StageContext};
    use crate::intelligence_contracts::{LinkValidationReceipt, LINK_VALIDATION_VERSION};
    use crate::investigation::insights::Inference;
    use crate::memory_metadata::LinkPayload;
    let mut response = run(tx, tx.query("SELECT VALUE output_json FROM ONLY $links; SELECT VALUE output_json FROM ONLY $body; SELECT VALUE output_json FROM ONLY $inferences; SELECT input_json,role_snapshot FROM ONLY $root;")
        .bind(("links",rid("stage_output",&super::brief_stages::task_id(job,"intel_brief_links"))))
        .bind(("body",rid("stage_output",&super::brief_stages::task_id(job,"intel_brief_body"))))
        .bind(("inferences",rid("stage_output",&super::brief_stages::task_id(job,"intel_brief_inferences"))))
        .bind(("root",rid("brief_stage_input",&super::brief_stages::task_id(job,"intel_brief_claims"))))).await?;
    let links: Value = take::<Option<String>>(&mut response, 0)?
        .map(|value| serde_json::from_str(&value))
        .transpose()
        .map_err(abort)?
        .unwrap_or(Value::Null);
    let body: Value = take::<Option<String>>(&mut response, 1)?
        .map(|value| serde_json::from_str(&value))
        .transpose()
        .map_err(abort)?
        .unwrap_or(Value::Null);
    let raw_inferences: Value = take::<Option<String>>(&mut response, 2)?
        .map(|value| serde_json::from_str(&value))
        .transpose()
        .map_err(abort)?
        .unwrap_or(Value::Null);
    #[derive(SurrealValue)]
    struct Root {
        input_json: String,
        role_snapshot: String,
    }
    let root: Option<Root> = take(&mut response, 3)?;
    let context = root.map(|root| StageContext {
        job_id: job.into(),
        task_id: String::new(),
        job_generation: 0,
        lease_epoch: 0,
        metadata_snapshot: serde_json::from_str::<Value>(&root.input_json)
            .map(|value| value["metadata"].clone())
            .unwrap_or(Value::Null),
        route_snapshot: root.role_snapshot,
        ledger: Arc::default(),
    });
    let context = context
        .map(|context| stages::body_bound_context(&context, body["content_hash"].as_str()))
        .transpose()
        .map_err(abort)?;
    let mut current = Vec::new();
    let previous = curated_ids(tx, article).await?;
    for id in &previous {
        if let Some(memory) = curated_memory(tx, id).await? {
            if let Ok(MemoryPayload::Link(payload)) = serde_json::from_str(&memory.payload_json) {
                if memory.lifecycle == "active"
                    && still_live_curated(tx, &payload, derivations, prepared).await?
                {
                    current.push(id.clone());
                }
            }
        }
    }
    let proofs: Vec<CuratedLinkProof> = if links["curated_links"].is_null() {
        Vec::new()
    } else {
        serde_json::from_value(links["curated_links"].clone()).map_err(abort)?
    };
    if proofs.len() > stages::MAX_LEADS {
        return Err(abort("curated Link proposal bound exceeded"));
    }
    for proof in proofs {
        if proof.schema_version != LINK_VALIDATION_VERSION || proof.source_article_id != article {
            continue;
        }
        let Some(context) = &context else {
            continue;
        };
        if stages::curation_revision(context).map_err(abort)? != proof.context_revision {
            continue;
        }
        let Some(source_article) =
            super::atlas::article_in(tx, &proof.source_run_id, &proof.source_article_id).await?
        else {
            continue;
        };
        let published = chrono::DateTime::parse_from_rfc3339(&source_article.published_at)
            .ok()
            .map(|_| source_article.published_at.clone());
        if crate::atlas_metadata::input_hash(&source_article) != proof.source_metadata_revision
            || published != proof.source_published_at
        {
            continue;
        }
        let Some(target) =
            super::atlas::article_in(tx, &proof.candidate_run_id, &proof.candidate_article_id)
                .await?
        else {
            continue;
        };
        let Some(raw_inference) = raw_inferences["inferences"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|inference| inference["id"] == proof.inference_id)
        else {
            continue;
        };
        let Some((memory_id, inference)) = derivations.iter().find_map(|row| {
            let inference: Inference = serde_json::from_str(&row.item_json).ok()?;
            if inference.id == proof.inference_id {
                Some((row.memory_id.clone()?, inference))
            } else {
                None
            }
        }) else {
            continue;
        };
        if inference.text != proof.source_assertion
            || inference.evidence_refs.is_empty()
            || !inference.evidence_refs.iter().all(|reference| {
                prepared
                    .input
                    .evidence
                    .iter()
                    .any(|evidence| evidence.validate_ref(reference))
            })
        {
            continue;
        }
        let Ok(item) = stages::curated_semantic_item(&proof, raw_inference, &body, &target) else {
            continue;
        };
        let route = context.route_key(
            Some(crate::provider::OP_INFERENCES_ASSESS),
            "atlas_evaluator",
        );
        if proof.decision_receipt != format!("intel-fidelity:{}:{route}", item.receipt_key()) {
            continue;
        }
        let mut response = run(
            tx,
            tx.query("SELECT VALUE value FROM ONLY $receipt")
                .bind(("receipt", rid("brief_receipt", &proof.decision_receipt))),
        )
        .await?;
        if take::<Option<String>>(&mut response, 0)?.as_deref() != Some("faithful") {
            continue;
        }
        let Some(source) = curated_memory(tx, &memory_id).await? else {
            continue;
        };
        if source.lifecycle != "active" || source.memory_kind != "inference" {
            continue;
        }
        let Ok((target_evidence, _)) = stages::curated_candidate_evidence(&proof, &target) else {
            continue;
        };
        let source_endpoint = EndpointRef::Memory {
            memory_id,
            kind: MemoryKind::Inference,
        };
        let target_endpoint = EndpointRef::Article {
            run_id: target.run_id.clone(),
            article_id: target.id.clone(),
            revision: proof.candidate_metadata_revision.clone(),
        };
        let roots: std::collections::BTreeSet<_> = inference
            .evidence_refs
            .iter()
            .map(|reference| reference.origin_root.as_str())
            .filter(|root| !root.is_empty())
            .collect();
        let source_lineage = if roots.len() == 1 {
            roots.iter().next().copied().unwrap_or("").to_owned()
        } else if roots.is_empty() {
            String::new()
        } else {
            crate::nlp::cache::hash(&roots.into_iter().collect::<Vec<_>>().join("|"))
        };
        let mut evidence = inference.evidence_refs;
        evidence.extend(target_evidence);
        let receipt = LinkValidationReceipt {
            schema_version: LINK_VALIDATION_VERSION.into(),
            kind: proof.kind,
            source: source_endpoint.clone(),
            target: target_endpoint.clone(),
            source_revision: endpoint_revision(&source)?,
            target_revision: proof.candidate_metadata_revision,
            evidence,
            source_lineage,
            target_lineage: String::new(),
            chronology_basis: proof.chronology_basis,
            source_time: proof.source_published_at,
            target_time: proof.target_published_at,
            conditions: proof.conditions,
            limitations: proof.limitations,
            decision_receipt: proof.decision_receipt,
            context_revision: proof.context_revision,
        };
        if receipt.validate_structure().is_err() {
            continue;
        }
        let id = crate::brain_actors::hid(
            "link",
            &[
                proof.kind.as_str(),
                &crate::brain_actors::endpoint_key(&source_endpoint),
                &crate::brain_actors::endpoint_key(&target_endpoint),
                &crate::nlp::cache::hash(&serde_json::to_string(&receipt).map_err(abort)?),
            ],
        );
        let payload = MemoryPayload::Link(Box::new(LinkPayload {
            validation: Some(receipt),
            source: source_endpoint,
            target: target_endpoint,
            relation: proof.kind.as_str().into(),
            rationale: proof.rationale,
            receipts: Vec::new(),
            promoted_from: None,
        }));
        super::memory::put_typed_in(
            tx,
            &super::memory::TypedMemory {
                id: id.clone(),
                text: crate::brain_actors::text_of(&payload),
                kind: MemoryKind::Link,
                payload: Some(payload),
                lifecycle: "active".into(),
            },
        )
        .await?;
        if !current.contains(&id) {
            current.push(id);
        }
    }
    if current.len() > 200 {
        return Err(abort(
            "curated Link owner bound reached; prior confirmed records retained",
        ));
    }
    let stale: Vec<_> = previous
        .into_iter()
        .filter(|id| !current.contains(id))
        .collect();
    let retired = super::memory::set_lifecycle_in(tx, &stale, "retired").await?;
    run(
        tx,
        tx.query("UPSERT type::record('app_state',$key) SET value=$value RETURN NONE")
            .bind(("key", curated_key(article)))
            .bind(("value", serde_json::to_string(&current).map_err(abort)?)),
    )
    .await?;
    current.extend(retired);
    Ok(current)
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
