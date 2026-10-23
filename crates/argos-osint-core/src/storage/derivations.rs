//! Canonical derivation writes on SurrealDB (migration phase 4b, part 1):
//! `insight_persist::write_derivations` and the article association
//! retirement it starts with, as the `publish` step of
//! [`super::insight::publish_snapshot_on`].
//!
//! Per item: tombstone check (including legacy fingerprints of the
//! subject's original forms), the canonical memory and `insight_claim`
//! (insert-if-missing), its `insight_source` rows, the derivation and its
//! evidence (premise and memory-ref evidence for inferences), then the
//! snapshot readiness and assessed links. Index work and `memories_changed`
//! join the same transaction through the outbox.
//!
//! Differences from SQLite, by design or pending:
//! - The caller passes the live Recon opt-in in [`DerivationContext`]
//!   (`save_insights`, already rechecked) and the owner run's thread;
//!   `recon_runs` is not on SurrealDB.
//! - Claim subjects and referenced actors bind through
//!   [`super::actor_bind`]; claims and inferences get typed payloads.
//! - [`write_article_derivations_for_kind_in`] replaces one article kind and
//!   carries the other kind's derivations forward.
//! - No `recon_entities` rows; `model_versions_json` stays `{}` (no stage
//!   checkpoints on SurrealDB yet); the publisher prior is read from any
//!   run's metadata of the article; the job retention sweep runs after
//!   commit (`jobs::cleanup_batch`), not inside the transaction.

use std::collections::{BTreeSet, HashSet};

use serde_json::Value;
use std::sync::Arc;

use surrealdb::types::SurrealValue;

use super::insight::{
    tombstone_runs, SnapshotStep, DELETE_RELATIONS_LEFT_SURQL, DELETE_RELATIONS_RIGHT_SURQL,
};
use super::jobs::{rid, run, take};
use super::outbox::{self, NewChange};
use super::publisher::CHANGED_COUNTER;
use super::tx::{Tx, TxError};
use crate::investigation::insights::{
    claim_fingerprint, inference_fingerprint, Assessment, EvidenceRef, PipelineInput,
    PipelineOutput,
};

fn json_err(e: serde_json::Error) -> TxError {
    TxError::Abort(format!("derivation json: {e}"))
}

/// Live facts the caller resolved under its own reads.
#[derive(Clone, Debug, Default)]
pub struct DerivationContext {
    /// `input.save_insights` with the live Recon opt-in rechecked.
    pub save_insights: bool,
    /// For a Recon owner: the run's thread (sources carry it).
    pub recon_thread: Option<String>,
}

struct Item {
    id: String,
    kind: &'static str,
    fingerprint: String,
    text: String,
    item_json: String,
    assessment: String,
    refs: Vec<EvidenceRef>,
    subject: String,
    predicate: String,
    object: String,
}

fn items(output: &PipelineOutput, kind: Option<&str>) -> Result<Vec<Item>, TxError> {
    let mut out = Vec::new();
    for c in output.claims.iter().filter(|_| kind != Some("inference")) {
        out.push(Item {
            id: c.id.clone(),
            kind: "claim",
            fingerprint: claim_fingerprint(c),
            text: c.text.clone(),
            item_json: serde_json::to_string(c).map_err(json_err)?,
            assessment: serde_json::to_string(&c.assessment).map_err(json_err)?,
            refs: c.evidence_refs.clone(),
            subject: c.subject_ref.clone(),
            predicate: c.predicate.clone(),
            object: c.object.clone(),
        });
    }
    for i in output.inferences.iter().filter(|_| kind != Some("claim")) {
        out.push(Item {
            id: i.id.clone(),
            kind: "inference",
            fingerprint: inference_fingerprint(i),
            text: i.text.clone(),
            item_json: serde_json::to_string(i).map_err(json_err)?,
            assessment: serde_json::to_string(&i.assessment).map_err(json_err)?,
            refs: i.evidence_refs.clone(),
            subject: i.entity_refs.first().cloned().unwrap_or_default(),
            predicate: "derived".into(),
            object: i.text.clone(),
        });
    }
    Ok(out)
}

/// Write the snapshot's derivations; returns the memory ids whose index
/// state may have changed (created, retired or deleted).
pub async fn write_derivations_in(
    tx: &Tx,
    snapshot_id: &str,
    input: &PipelineInput,
    output: &PipelineOutput,
    now: &str,
    kind: Option<&str>,
    ctx: &DerivationContext,
) -> Result<Vec<String>, TxError> {
    let owner_kind = input.snapshot.owner_kind.as_str();
    let owner = input.snapshot.owner_id.as_str();
    let save = input.save_insights && ctx.save_insights;
    let mut touched = Vec::new();
    if owner_kind == "article" {
        touched.extend(retire_article_associations_in(tx, owner, output, kind).await?);
    }
    let source = crate::brain::MemorySource {
        app: owner_kind.to_string(),
        conversation_id: owner.to_string(),
        message_id: None,
        reference: Some(snapshot_id.to_string()),
    };
    let source_json = serde_json::to_string(&source).map_err(json_err)?;
    for item in items(output, kind)? {
        let assessment: Assessment = serde_json::from_str(&item.assessment).map_err(json_err)?;
        let faithful = assessment.faithful() && !item.refs.is_empty();
        let mut memory_id = None;
        if save && faithful && !tombstoned(tx, &item, output, owner).await? {
            let mid = canonical_memory_in(tx, &item, owner_kind, &source_json, now).await?;
            touched.extend(bind_item_in(tx, snapshot_id, input, output, &item, &mid, now).await?);
            for r in &item.refs {
                let call = r.source_id.strip_prefix("call:").unwrap_or(&r.source_id);
                let key = vec![
                    item.fingerprint.clone(),
                    owner.to_string(),
                    snapshot_id.to_string(),
                    call.to_string(),
                ];
                run(
                    tx,
                    tx.query(
                        "IF !record::exists(type::record('insight_source', $key)) {
                           CREATE type::record('insight_source', $key) SET fingerprint = $fp,
                             thread_id = $thread, run_id = $run, answer_id = $answer,
                             call_id = $call, source_url = $url, published_at = $published
                             RETURN NONE;
                         };",
                    )
                    .bind(("key", key))
                    .bind(("fp", item.fingerprint.clone()))
                    .bind((
                        "thread",
                        ctx.recon_thread.clone().filter(|_| owner_kind == "recon"),
                    ))
                    .bind(("run", owner.to_string()))
                    .bind(("answer", snapshot_id.to_string()))
                    .bind(("call", call.to_string()))
                    .bind(("url", r.url.clone()))
                    .bind(("published", r.published_at.clone().unwrap_or_default())),
                )
                .await?;
            }
            touched.push(mid.clone());
            memory_id = Some(mid);
        }
        let derivation_id = format!("{snapshot_id}:{}", item.id);
        let mut q = String::from(
            "CREATE type::record('insight_derivation', $d) SET snapshot_id = $s, memory_id = $m,
               insight_kind = $kind, fingerprint = $fp, item_json = $item,
               assessment_json = $assessment, status = $status, revision = $rev RETURN NONE;",
        );
        for i in 0..item.refs.len() {
            q.push_str(&format!(
                "CREATE insight_derivation_evidence SET derivation_id = $d, source_id = $src{i},
                   source_revision = $srev{i}, evidence_json = $ev{i} RETURN NONE;"
            ));
        }
        let mut query = tx
            .query(q)
            .bind(("d", derivation_id))
            .bind(("s", snapshot_id.to_string()))
            .bind(("m", memory_id))
            .bind(("kind", item.kind.to_string()))
            .bind(("fp", item.fingerprint.clone()))
            .bind(("item", item.item_json.clone()))
            .bind(("assessment", item.assessment.clone()))
            .bind(("status", if faithful { "assessed" } else { "rejected" }))
            .bind(("rev", crate::nlp::cache::hash(&item.text)));
        for (i, r) in item.refs.iter().enumerate() {
            query = query
                .bind((format!("src{i}"), r.source_id.clone()))
                .bind((format!("srev{i}"), r.revision.clone()))
                .bind((
                    format!("ev{i}"),
                    serde_json::to_string(r).map_err(json_err)?,
                ));
        }
        run(tx, query).await?;
    }
    inference_evidence_in(tx, snapshot_id, input, output, kind).await?;
    readiness_in(tx, snapshot_id, input, output).await?;
    if kind.is_none() {
        for link in &output.links {
            let [left, right] = &link.typed_endpoints[..] else {
                return Err(TxError::Abort(format!(
                    "link {} needs two endpoints",
                    link.id
                )));
            };
            run(
                tx,
                tx.query(
                    "CREATE type::record('insight_derivation_link', $id) SET snapshot_id = $s,
                       left_kind = $lk, left_id = $li, right_kind = $rk, right_id = $ri,
                       relation = $rel, rationale = $why, assessment_json = $a,
                       evidence_json = $e, status = 'assessed', revision = $rev RETURN NONE;",
                )
                .bind(("id", format!("{snapshot_id}:{}", link.id)))
                .bind(("s", snapshot_id.to_string()))
                .bind(("lk", left.kind.clone()))
                .bind(("li", left.id.clone()))
                .bind(("rk", right.kind.clone()))
                .bind(("ri", right.id.clone()))
                .bind(("rel", link.relation.clone()))
                .bind(("why", link.rationale.clone()))
                .bind((
                    "a",
                    serde_json::to_string(&link.assessment).map_err(json_err)?,
                ))
                .bind((
                    "e",
                    serde_json::to_string(&link.evidence_refs).map_err(json_err)?,
                ))
                .bind((
                    "rev",
                    crate::nlp::cache::hash(&serde_json::to_string(link).map_err(json_err)?),
                )),
            )
            .await?;
        }
    }
    Ok(touched)
}

/// A deletion tombstone owned by this owner (or unscoped) suppresses the
/// item; claims also check the legacy fingerprints of the subject's forms.
async fn tombstoned(
    tx: &Tx,
    item: &Item,
    output: &PipelineOutput,
    owner: &str,
) -> Result<bool, TxError> {
    let owned = |runs: Vec<String>| runs.is_empty() || runs.iter().any(|r| r == owner);
    if tombstone_runs(tx, &item.fingerprint)
        .await?
        .is_some_and(owned)
    {
        return Ok(true);
    }
    if item.kind != "claim" {
        return Ok(false);
    }
    let Some(entity) = output.entities.iter().find(|e| e.id == item.subject) else {
        return Ok(false);
    };
    let namespace = match entity.type_hint.as_str() {
        "person" => "person",
        "place" => "place",
        _ => "org",
    };
    for form in &entity.original_forms {
        let legacy = crate::store::insight_fingerprint(
            namespace,
            &form.trim().to_lowercase(),
            &item.predicate.trim().to_lowercase(),
            &item.object.trim().to_lowercase(),
        );
        if tombstone_runs(tx, &legacy).await?.is_some_and(owned) {
            return Ok(true);
        }
    }
    Ok(false)
}

#[derive(SurrealValue)]
struct Existing {
    source_json: String,
}

/// The item's memory (the claim's, or `mem:<fingerprint>`) and its
/// `insight_claim`, both insert-if-missing. An article refresh rewrites an
/// article-owned memory that no source backs yet, dropping its user edit.
async fn canonical_memory_in(
    tx: &Tx,
    item: &Item,
    owner_kind: &str,
    source_json: &str,
    now: &str,
) -> Result<String, TxError> {
    let mut r = run(
        tx,
        tx.query("SELECT VALUE memory_id FROM ONLY type::record('insight_claim', $fp)")
            .bind(("fp", item.fingerprint.clone())),
    )
    .await?;
    let mid =
        take::<Option<String>>(&mut r, 0)?.unwrap_or_else(|| format!("mem:{}", item.fingerprint));
    let mut r = run(
        tx,
        tx.query(
            "IF !record::exists($m) {
               CREATE $m SET text = $text, category = 'investigation', pinned = false,
                 created_at = <datetime>$now, source_json = $src, memory_kind = $kind
                 RETURN NONE;
             };
             SELECT source_json FROM ONLY $m;
             RETURN count(SELECT id FROM insight_source WHERE fingerprint = $fp LIMIT 1) > 0;",
        )
        .bind(("m", rid("memory", &mid)))
        .bind(("text", item.text.clone()))
        .bind(("now", now.to_string()))
        .bind(("src", source_json.to_string()))
        .bind(("kind", item.kind.to_string()))
        .bind(("fp", item.fingerprint.clone())),
    )
    .await?;
    let existing: Option<Existing> = take(&mut r, 1)?;
    let sourced = take::<Option<bool>>(&mut r, 2)? == Some(true);
    if owner_kind == "article" && !sourced {
        let app = existing
            .and_then(|e| serde_json::from_str::<serde_json::Value>(&e.source_json).ok())
            .and_then(|v| v["app"].as_str().map(str::to_string));
        if matches!(app.as_deref(), Some("article" | "atlas")) {
            run(
                tx,
                tx.query(
                    "UPDATE $m SET text = $text, source_json = $src RETURN NONE;
                     DELETE type::record('insight_user_edit', $id) RETURN NONE;",
                )
                .bind(("m", rid("memory", &mid)))
                .bind(("id", mid.clone()))
                .bind(("text", item.text.clone()))
                .bind(("src", source_json.to_string())),
            )
            .await?;
        }
    }
    run(
        tx,
        tx.query(
            "IF !record::exists(type::record('insight_claim', $fp)) {
               CREATE type::record('insight_claim', $fp) SET memory_id = $m, entity_id = $subject,
                 predicate = $predicate, object_value = $object, topic = 'investigation',
                 classification = $kind, confidence = 0.0, created_at = $now, updated_at = $now
                 RETURN NONE;
             };",
        )
        .bind(("fp", item.fingerprint.clone()))
        .bind(("m", mid.clone()))
        .bind(("subject", item.subject.clone()))
        .bind(("predicate", item.predicate.clone()))
        .bind(("object", item.object.clone()))
        .bind(("kind", item.kind.to_string()))
        .bind(("now", now.to_string())),
    )
    .await?;
    Ok(mid)
}

/// Inference evidence inherited from premise claims and from the memory
/// refs the snapshot read, idempotent per evidence identity.
async fn inference_evidence_in(
    tx: &Tx,
    snapshot_id: &str,
    input: &PipelineInput,
    output: &PipelineOutput,
    kind: Option<&str>,
) -> Result<(), TxError> {
    if kind == Some("claim") {
        return Ok(());
    }
    for inference in &output.inferences {
        let derivation = format!("{snapshot_id}:{}", inference.id);
        for premise in &inference.premise_claim_ids {
            let Some(claim) = output.claims.iter().find(|c| &c.id == premise) else {
                continue;
            };
            let mut r = run(
                tx,
                tx.query(
                    "SELECT VALUE memory_id FROM insight_derivation
                       WHERE id = type::record('insight_derivation', $d) AND snapshot_id = $s AND insight_kind = 'claim' AND status = 'assessed'",
                )
                .bind(("d", format!("{snapshot_id}:{premise}")))
                .bind(("s", snapshot_id.to_string())),
            )
            .await?;
            let memory: Option<String> = take::<Vec<Option<String>>>(&mut r, 0)?
                .into_iter()
                .flatten()
                .next();
            for reference in &claim.evidence_refs {
                let ev = serde_json::to_string(reference).map_err(json_err)?;
                evidence_upsert_in(
                    tx,
                    &derivation,
                    reference,
                    memory.clone(),
                    premise,
                    &ev,
                    true,
                )
                .await?;
            }
        }
        for (memory, revision) in &input.snapshot.memory_refs {
            let premise = format!("memory:{memory}@{revision}");
            for reference in &inference.evidence_refs {
                let ev = serde_json::to_string(reference).map_err(json_err)?;
                evidence_upsert_in(
                    tx,
                    &derivation,
                    reference,
                    Some(memory.clone()),
                    &premise,
                    &ev,
                    false,
                )
                .await?;
            }
        }
    }
    Ok(())
}

/// Evidence keyed by `(derivation, source, revision, premise, evidence)`;
/// `update` refreshes `memory_id` on conflict, otherwise it is ignored.
async fn evidence_upsert_in(
    tx: &Tx,
    derivation: &str,
    r: &EvidenceRef,
    memory: Option<String>,
    premise: &str,
    evidence_json: &str,
    update: bool,
) -> Result<(), TxError> {
    let key = crate::nlp::cache::hash(
        &serde_json::to_string(&(
            derivation,
            &r.source_id,
            &r.revision,
            premise,
            evidence_json,
        ))
        .map_err(json_err)?,
    );
    let sql = if update {
        "UPSERT type::record('insight_derivation_evidence', $k) SET derivation_id = $d,
           source_id = $src, source_revision = $rev, memory_id = $m, premise_id = $p,
           evidence_json = $ev RETURN NONE;"
    } else {
        "IF !record::exists(type::record('insight_derivation_evidence', $k)) {
           CREATE type::record('insight_derivation_evidence', $k) SET derivation_id = $d,
             source_id = $src, source_revision = $rev, memory_id = $m, premise_id = $p,
             evidence_json = $ev RETURN NONE;
         };"
    };
    run(
        tx,
        tx.query(sql)
            .bind(("k", key))
            .bind(("d", derivation.to_string()))
            .bind(("src", r.source_id.clone()))
            .bind(("rev", r.revision.clone()))
            .bind(("m", memory))
            .bind(("p", premise.to_string()))
            .bind(("ev", evidence_json.to_string())),
    )
    .await?;
    Ok(())
}

/// Snapshot readiness with the brief rating (publisher prior from the
/// article's metadata, `F` when unknown).
async fn readiness_in(
    tx: &Tx,
    snapshot_id: &str,
    input: &PipelineInput,
    output: &PipelineOutput,
) -> Result<(), TxError> {
    let mut r = run(
        tx,
        tx.query(
            "SELECT VALUE metadata_json FROM atlas_article_metadata WHERE article_id = $a
               ORDER BY run_id DESC LIMIT 1",
        )
        .bind(("a", input.snapshot.owner_id.clone())),
    )
    .await?;
    let publisher = take::<Vec<String>>(&mut r, 0)?
        .into_iter()
        .next()
        .and_then(|j| serde_json::from_str::<crate::atlas_metadata::ArticleMetadata>(&j).ok())
        .and_then(|m| m.publisher_prior)
        .and_then(|p| crate::osint::SourceReliability::parse(&p.observation.reliability))
        .unwrap_or(crate::osint::SourceReliability::F);
    let rating = crate::intel_recon::EvidenceConfidence::estimate_for_source(
        output,
        output.readiness.publication_eligible,
        publisher,
        input.surface == "recon_report",
    );
    let readiness = serde_json::json!({
        "readiness": output.readiness,
        "limit_reached": output.limit_reached,
        "partial_enumeration": output.partial_enumeration,
        "item_gaps": output.item_gaps,
        "withheld_count": output.withheld_count,
        "rating": {
            "policy": "brief-rating-v2",
            "input_mode": input.surface,
            "source_hash": crate::nlp::cache::key(&serde_json::json!(input.snapshot.source_refs)),
            "estimate": rating,
        },
    });
    run(
        tx,
        tx.query("UPDATE type::record('insight_snapshot', $s) SET readiness_json = $r RETURN NONE")
            .bind(("s", snapshot_id.to_string()))
            .bind(("r", readiness.to_string())),
    )
    .await?;
    Ok(())
}

#[derive(SurrealValue)]
struct ClaimRow {
    fp: String,
    memory_id: String,
}

/// Retire an article's associations, not its historical snapshots. Only
/// exclusively owned canonical memories are deleted; memories another
/// owner, an inference premise or a user edit keeps stay. Returns deleted
/// memory ids.
pub async fn retire_article_associations_in(
    tx: &Tx,
    article: &str,
    output: &PipelineOutput,
    kind: Option<&str>,
) -> Result<Vec<String>, TxError> {
    let retained: HashSet<String> = output
        .claims
        .iter()
        .filter(|c| c.assessment.faithful())
        .map(claim_fingerprint)
        .chain(output.inferences.iter().map(inference_fingerprint))
        .collect();
    let mut r = run(
        tx,
        tx.query(
            "LET $snaps = (SELECT VALUE meta::id(id) FROM insight_snapshot
               WHERE owner_kind INSIDE ['article', 'atlas'] AND owner_id = $a);
             LET $dfps = (SELECT VALUE fingerprint FROM insight_derivation WHERE snapshot_id INSIDE $snaps);
             LET $sfps = (SELECT VALUE fingerprint FROM insight_source WHERE call_id = $a);
             SELECT meta::id(id) AS fp, memory_id FROM insight_claim
               WHERE ($k = NONE OR classification = $k)
                 AND (meta::id(id) INSIDE $dfps OR meta::id(id) INSIDE $sfps);
             DELETE insight_source WHERE (answer_id INSIDE $snaps OR call_id = $a)
               AND ($k = NONE OR type::record('insight_claim', fingerprint).classification = $k)
               RETURN NONE;
             IF $k = NONE {
               UPDATE insight_derivation_link SET status = 'retired' WHERE snapshot_id INSIDE $snaps
                 RETURN NONE;
             };
             UPDATE insight_derivation SET status = 'retired'
               WHERE snapshot_id INSIDE $snaps AND ($k = NONE OR insight_kind = $k) RETURN NONE;",
        )
        .bind(("a", article.to_string()))
        .bind(("k", kind.map(str::to_string))),
    )
    .await?;
    let rows: Vec<ClaimRow> = take(&mut r, 3)?;
    let mut deleted = Vec::new();
    let mut seen = BTreeSet::new();
    for row in rows {
        if !seen.insert((row.fp.clone(), row.memory_id.clone())) {
            continue;
        }
        let mut r = run(
            tx,
            tx.query(
                "RETURN count(SELECT id FROM insight_source WHERE fingerprint = $fp LIMIT 1) > 0;
                 LET $live = (SELECT VALUE snapshot_id FROM insight_derivation
                   WHERE memory_id = $m AND status = 'assessed');
                 RETURN count(SELECT id FROM insight_owner_state
                   WHERE active_snapshot_id INSIDE $live AND owner_id != $a LIMIT 1) > 0;
                 SELECT VALUE payload_json FROM memory WHERE memory_kind = 'inference'
                   AND lifecycle = 'active' AND string::contains(payload_json, $m);
                 RETURN record::exists(type::record('insight_user_edit', $m));
                 SELECT VALUE source_json FROM ONLY type::record('memory', $m);",
            )
            .bind(("fp", row.fp.clone()))
            .bind(("m", row.memory_id.clone()))
            .bind(("a", article.to_string())),
        )
        .await?;
        let sourced = take::<Option<bool>>(&mut r, 0)? == Some(true);
        let elsewhere = take::<Option<bool>>(&mut r, 2)? == Some(true);
        let premise = kind == Some("claim")
            && take::<Vec<String>>(&mut r, 3)?.iter().any(|raw| {
                serde_json::from_str::<serde_json::Value>(raw)
                    .ok()
                    .and_then(|v| v["payload"]["premise_claims"].as_array().cloned())
                    .is_some_and(|p| p.iter().any(|x| x.as_str() == Some(&row.memory_id)))
            });
        if sourced || elsewhere || premise {
            continue;
        }
        run(
            tx,
            tx.query(format!(
                "{DELETE_RELATIONS_LEFT_SURQL} RETURN NONE; {DELETE_RELATIONS_RIGHT_SURQL} RETURN NONE;"
            ))
            .bind(("fp", row.fp.clone())),
        )
        .await?;
        if retained.contains(&row.fp) || take::<Option<bool>>(&mut r, 4)? == Some(true) {
            continue;
        }
        let owned = take::<Option<String>>(&mut r, 5)?
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .and_then(|v| v["app"].as_str().map(str::to_string))
            .is_some_and(|app| app == "article" || app == "atlas");
        if owned {
            run(
                tx,
                tx.query(
                    "DELETE type::record('insight_user_edit', $m) RETURN NONE;
                     DELETE type::record('insight_claim', $fp) RETURN NONE;
                     DELETE type::record('memory_graph_summary', $m) RETURN NONE;
                     DELETE type::record('memory', $m) RETURN NONE;",
                )
                .bind(("m", row.memory_id.clone()))
                .bind(("fp", row.fp.clone())),
            )
            .await?;
            deleted.push(row.memory_id);
        }
    }
    Ok(deleted)
}

/// The `publish` step for [`super::insight::publish_snapshot_on`]: write
/// the derivations, then queue index work for touched memories and bump
/// `memories_changed` (when insights are saved) in the same transaction.
pub fn derivation_step(
    input: Arc<PipelineInput>,
    output: Arc<PipelineOutput>,
    kind: Option<&'static str>,
    ctx: DerivationContext,
) -> SnapshotStep {
    Arc::new(move |tx: Tx, snapshot_id: String| {
        let (input, output, ctx) = (input.clone(), output.clone(), ctx.clone());
        Box::pin(async move {
            let now = chrono::Utc::now();
            let now_s = super::jobs::ts(now);
            let ids = match kind {
                Some(kind) => {
                    write_article_derivations_for_kind_in(
                        &tx,
                        &snapshot_id,
                        &input,
                        &output,
                        &now_s,
                        kind,
                        &ctx,
                    )
                    .await?
                }
                None => {
                    write_derivations_in(&tx, &snapshot_id, &input, &output, &now_s, None, &ctx)
                        .await?
                }
            };
            let mut ids = ids;
            ids.sort();
            ids.dedup();
            let nonce = format!("deriv:{snapshot_id}:{}", kind.unwrap_or("all"));
            let changes = super::memory::index_changes_in(&tx, &ids, &nonce).await?;
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
            let bump = (input.save_insights && ctx.save_insights).then_some(CHANGED_COUNTER);
            outbox::enqueue_many_in(&tx, &new, bump, now).await?;
            Ok(())
        })
    })
}

/// Typed payloads and actor refs for one written item: a claim binds its
/// subject and the actors its cited passages reference; an inference
/// records its premise memories and their actors (role `premise`).
async fn bind_item_in(
    tx: &Tx,
    snapshot_id: &str,
    input: &PipelineInput,
    output: &PipelineOutput,
    item: &Item,
    mid: &str,
    now: &str,
) -> Result<Vec<String>, TxError> {
    use super::actor_bind::{
        add_memory_actor_ref_in, bind_claim_subject_in, bind_in, referenced, resolve_in,
        update_claim_payload_in,
    };
    use crate::memory_metadata::{ActorRef, InferencePayload, MemoryKind, MemoryPayload};
    let owner_kind = input.snapshot.owner_kind.as_str();
    let scope = format!("{owner_kind}:{}", input.snapshot.owner_id);
    let mut touched = Vec::new();
    if item.kind == "claim" {
        let label = output
            .entities
            .iter()
            .find(|e| e.id == item.subject)
            .map(|e| {
                e.original_forms
                    .first()
                    .filter(|f| !f.trim().is_empty())
                    .unwrap_or(&e.comparison_key)
                    .clone()
            });
        touched.extend(bind_claim_subject_in(tx, &item.fingerprint, label.as_deref(), now).await?);
        let provenance = serde_json::json!({"snapshot": snapshot_id, "claim": item.id});
        for entity in &output.entities {
            let article_entity = |id: &str| {
                owner_kind == "article" && input.snapshot.entities.iter().any(|e| e.id == id)
            };
            if !referenced(entity, &item.subject, &item.refs, article_entity) {
                continue;
            }
            let name = entity
                .original_forms
                .first()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(&entity.comparison_key);
            let (bound, changed) = bind_in(
                tx,
                &crate::brain_actors::BindRequest {
                    name,
                    subtype: crate::brain_actors::parse_subtype(&entity.type_hint),
                    scope: &scope,
                    identifiers: &[],
                    external: Some(("recon_entities", &entity.id)),
                    provenance: provenance.clone(),
                },
                now,
            )
            .await?;
            touched.extend(changed);
            let role = if entity.id == item.subject {
                "subject"
            } else {
                "mentioned"
            };
            add_memory_actor_ref_in(tx, mid, &bound.actor_id, role, bound.state, &provenance)
                .await?;
        }
        let modality = output
            .claims
            .iter()
            .find(|c| c.id == item.id)
            .map(|c| c.modality.clone());
        update_claim_payload_in(tx, mid, |p| {
            p.predicate = item.predicate.clone();
            p.object = item.object.clone();
            p.assessment = item.assessment.clone();
            if let Some(m) = modality {
                p.modality = m;
            }
            p.passages = item.refs.iter().map(|r| r.source_id.clone()).collect();
        })
        .await?;
    } else {
        let inference = output
            .inferences
            .iter()
            .find(|i| i.id == item.id)
            .ok_or_else(|| TxError::Abort(format!("inference {} not in output", item.id)))?;
        let mut premises = Vec::new();
        for premise in &inference.premise_claim_ids {
            let mut r = run(
                tx,
                tx.query(
                    "SELECT VALUE memory_id FROM insight_derivation
                       WHERE id = type::record('insight_derivation', $d) AND status = 'assessed'",
                )
                .bind(("d", format!("{snapshot_id}:{premise}"))),
            )
            .await?;
            premises.extend(
                take::<Vec<Option<String>>>(&mut r, 0)?
                    .into_iter()
                    .flatten(),
            );
        }
        let mut r = run(
            tx,
            tx.query("SELECT VALUE payload_json FROM ONLY $m")
                .bind(("m", rid("memory", mid))),
        )
        .await?;
        let mut actors: Vec<ActorRef> = take::<Option<String>>(&mut r, 0)?
            .and_then(|raw| serde_json::from_str::<MemoryPayload>(&raw).ok())
            .and_then(|p| match p {
                MemoryPayload::Inference(p) => Some(p.actors),
                _ => None,
            })
            .unwrap_or_default();
        actors.retain(|a| a.role != "premise");
        // Explicit premise associations, not independent support.
        for premise in &premises {
            let mut r = run(
                tx,
                tx.query(
                    "SELECT VALUE payload_json FROM memory WHERE id = $m
                       AND memory_kind INSIDE ['claim', 'atomic_claim']",
                )
                .bind(("m", rid("memory", premise))),
            )
            .await?;
            let raw = take::<Vec<String>>(&mut r, 0)?.into_iter().next();
            if let Some(MemoryPayload::Claim(claim)) =
                raw.and_then(|raw| serde_json::from_str::<MemoryPayload>(&raw).ok())
            {
                for actor in claim.actors {
                    actors.push(ActorRef {
                        actor_id: resolve_in(tx, &actor.actor_id).await?,
                        role: "premise".into(),
                        binding_state: actor.binding_state,
                    });
                }
            }
        }
        actors.sort_by(|l, r| (&l.actor_id, &l.role).cmp(&(&r.actor_id, &r.role)));
        actors.dedup_by(|l, r| l.actor_id == r.actor_id && l.role == r.role);
        let payload = MemoryPayload::Inference(InferencePayload {
            premise_claims: premises,
            actors: actors.clone(),
            reasoning: inference.rationale.clone(),
            limitations: inference.limitations.join("; "),
            support_state: inference.assessment.fidelity.clone(),
            coverage_receipt: snapshot_id.into(),
        });
        let mut r = run(
            tx,
            tx.query("SELECT VALUE text FROM ONLY $m")
                .bind(("m", rid("memory", mid))),
        )
        .await?;
        let text = take::<Option<String>>(&mut r, 0)?.unwrap_or_else(|| item.text.clone());
        super::memory::put_typed_in(
            tx,
            &super::memory::TypedMemory {
                id: mid.to_string(),
                text,
                kind: MemoryKind::Inference,
                payload: Some(payload),
                lifecycle: "active".into(),
            },
        )
        .await?;
        run(
            tx,
            tx.query("DELETE brain_memory_actor_ref WHERE memory_id = $m AND role = 'premise' RETURN NONE")
                .bind(("m", mid.to_string())),
        )
        .await?;
        let provenance = serde_json::json!({"snapshot": snapshot_id, "inference": item.id,
            "premise_claims": inference.premise_claim_ids});
        for actor in actors {
            add_memory_actor_ref_in(
                tx,
                mid,
                &actor.actor_id,
                &actor.role,
                actor.binding_state,
                &provenance,
            )
            .await?;
        }
    }
    let mut r = run(
        tx,
        tx.query(
            "SELECT VALUE actor_id FROM brain_memory_actor_ref WHERE memory_id = $m ORDER BY actor_id",
        )
        .bind(("m", mid.to_string())),
    )
    .await?;
    let mut actors: Vec<String> = take(&mut r, 0)?;
    actors.dedup();
    touched.extend(actors);
    touched.push(mid.to_string());
    Ok(touched)
}

/// Replace one article insight kind, carrying the other kind's immutable
/// derivations (and their evidence) forward from the active snapshot. Only
/// the completed kind is written; an inference needs its premise claims
/// published at their current revision.
pub async fn write_article_derivations_for_kind_in(
    tx: &Tx,
    snapshot_id: &str,
    input: &PipelineInput,
    output: &PipelineOutput,
    now: &str,
    kind: &str,
    ctx: &DerivationContext,
) -> Result<Vec<String>, TxError> {
    if input.snapshot.owner_kind != "article" {
        return Err(TxError::Abort(
            "scoped derivations require an article".into(),
        ));
    }
    if !["claim", "inference"].contains(&kind) {
        return Err(TxError::Abort("invalid derivation kind".into()));
    }
    let carried_kind = if kind == "claim" {
        "inference"
    } else {
        "claim"
    };
    let mut r = run(
        tx,
        tx.query(
            "SELECT VALUE active_snapshot_id FROM insight_owner_state
               WHERE owner_kind = 'article' AND owner_id = $o",
        )
        .bind(("o", input.snapshot.owner_id.clone())),
    )
    .await?;
    let previous = take::<Vec<String>>(&mut r, 0)?
        .into_iter()
        .find(|id| !id.is_empty() && id != snapshot_id);
    let mut carried = Vec::new();
    if let Some(previous) = previous {
        #[derive(SurrealValue)]
        struct Prev {
            inputs_json: String,
            entities_json: String,
            coverage_json: String,
            readiness_json: String,
        }
        #[derive(SurrealValue)]
        struct Row {
            key: String,
            memory_id: Option<String>,
            insight_kind: String,
            fingerprint: String,
            item_json: String,
            assessment_json: String,
            status: String,
            revision: String,
        }
        #[derive(SurrealValue)]
        struct Ev {
            derivation_id: String,
            source_id: String,
            source_revision: String,
            memory_id: Option<String>,
            premise_id: String,
            evidence_json: String,
        }
        let mut r = run(
            tx,
            tx.query(
                "RETURN count(SELECT id FROM insight_derivation WHERE snapshot_id = $p
                   AND insight_kind = $ck AND status = 'assessed' LIMIT 1) > 0;
                 SELECT inputs_json, entities_json, coverage_json, readiness_json
                   FROM ONLY type::record('insight_snapshot', $p);
                 SELECT meta::id(id) AS key, memory_id, insight_kind, fingerprint, item_json,
                   assessment_json, status, revision FROM insight_derivation
                   WHERE snapshot_id = $p AND insight_kind != $k;
                 LET $ids = (SELECT VALUE meta::id(id) FROM insight_derivation
                   WHERE snapshot_id = $p AND insight_kind != $k);
                 SELECT derivation_id, source_id, source_revision, memory_id, premise_id,
                   evidence_json FROM insight_derivation_evidence WHERE derivation_id INSIDE $ids;",
            )
            .bind(("p", previous.clone()))
            .bind(("ck", carried_kind))
            .bind(("k", kind.to_string())),
        )
        .await?;
        let exists = take::<Option<bool>>(&mut r, 0)? == Some(true);
        let prev: Option<Prev> = take(&mut r, 1)?;
        let rows: Vec<Row> = take(&mut r, 2)?;
        let evidence: Vec<Ev> = take(&mut r, 4)?;
        if let (true, Some(prev)) = (exists, prev) {
            let mut readiness: Value =
                serde_json::from_str(&prev.readiness_json).map_err(json_err)?;
            let original = readiness["carried_kinds"]
                .as_array()
                .and_then(|rows| rows.iter().find(|row| row["kind"] == carried_kind))
                .cloned();
            if let Some(o) = readiness.as_object_mut() {
                o.remove("carried_kinds");
            }
            let parse = |s: &str| serde_json::from_str::<Value>(s).map_err(json_err);
            carried.push(match original {
                Some(o) => o,
                None => serde_json::json!({
                    "kind": carried_kind, "source_snapshot_id": previous,
                    "inputs": parse(&prev.inputs_json)?,
                    "entities": parse(&prev.entities_json)?,
                    "coverage": parse(&prev.coverage_json)?,
                    "readiness": readiness,
                }),
            });
        }
        let rekey = |id: &str| format!("{snapshot_id}{}", &id[previous.len().min(id.len())..]);
        for row in rows {
            run(
                tx,
                tx.query(
                    "CREATE type::record('insight_derivation', $d) SET snapshot_id = $s,
                       memory_id = $m, insight_kind = $kind, fingerprint = $fp, item_json = $item,
                       assessment_json = $a, status = $status, revision = $rev RETURN NONE;",
                )
                .bind(("d", rekey(&row.key)))
                .bind(("s", snapshot_id.to_string()))
                .bind(("m", row.memory_id))
                .bind(("kind", row.insight_kind))
                .bind(("fp", row.fingerprint))
                .bind(("item", row.item_json))
                .bind(("a", row.assessment_json))
                .bind(("status", row.status))
                .bind(("rev", row.revision)),
            )
            .await?;
        }
        for ev in evidence {
            run(
                tx,
                tx.query(
                    "CREATE insight_derivation_evidence SET derivation_id = $d, source_id = $src,
                       source_revision = $rev, memory_id = $m, premise_id = $p,
                       evidence_json = $ev RETURN NONE;",
                )
                .bind(("d", rekey(&ev.derivation_id)))
                .bind(("src", ev.source_id))
                .bind(("rev", ev.source_revision))
                .bind(("m", ev.memory_id))
                .bind(("p", ev.premise_id))
                .bind(("ev", ev.evidence_json)),
            )
            .await?;
        }
    }
    if kind == "inference" {
        let premises: BTreeSet<&str> = output
            .inferences
            .iter()
            .flat_map(|i| i.premise_claim_ids.iter().map(String::as_str))
            .collect();
        for premise in premises {
            let claim = output
                .claims
                .iter()
                .find(|c| c.id == premise)
                .ok_or_else(|| {
                    TxError::Abort(format!(
                        "canonical premise claim {premise} is not in the current extraction"
                    ))
                })?;
            let mut r = run(
                tx,
                tx.query(
                    "LET $d = (SELECT memory_id FROM insight_derivation
                       WHERE id = type::record('insight_derivation', $id) AND fingerprint = $fp
                         AND status = 'assessed')[0];
                     RETURN $d != NONE AND $d.memory_id != NONE
                       AND type::record('memory', $d.memory_id).memory_kind INSIDE ['claim', 'atomic_claim']
                       AND type::record('memory', $d.memory_id).lifecycle = 'active';",
                )
                .bind(("id", format!("{snapshot_id}:{}", claim.id)))
                .bind(("fp", claim_fingerprint(claim))),
            )
            .await?;
            if take::<Option<bool>>(&mut r, 1)? != Some(true) {
                return Err(TxError::Abort(format!(
                    "canonical premise claim {} is not published at its current revision",
                    claim.id
                )));
            }
        }
    }
    let touched =
        write_derivations_in(tx, snapshot_id, input, output, now, Some(kind), ctx).await?;
    let mut r = run(
        tx,
        tx.query("SELECT VALUE readiness_json FROM ONLY type::record('insight_snapshot', $s)")
            .bind(("s", snapshot_id.to_string())),
    )
    .await?;
    let mut readiness: Value = take::<Option<String>>(&mut r, 0)?
        .map(|raw| serde_json::from_str(&raw).map_err(json_err))
        .transpose()?
        .unwrap_or_else(|| serde_json::json!({}));
    readiness["carried_kinds"] = Value::Array(carried);
    run(
        tx,
        tx.query("UPDATE type::record('insight_snapshot', $s) SET readiness_json = $r RETURN NONE")
            .bind(("s", snapshot_id.to_string()))
            .bind(("r", readiness.to_string())),
    )
    .await?;
    Ok(touched)
}
