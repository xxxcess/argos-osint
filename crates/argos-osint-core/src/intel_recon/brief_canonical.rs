//! Canonical Brain/index publication after each completed intel brief kind.
//!
//! The brief lane reuses the shared canonical publisher
//! ([`insight_persist::publish_snapshot_on`] + [`insight_persist::write_derivations`]);
//! only its fence (the completed stage commit) and its readiness differ from the
//! insight-refresh lane. Only items the Claim assessor judged faithful to their
//! own cited passages are published; citations alone never make an item
//! faithful. Historical reports do not prevent brief publication. Canonical failure never
//! fails a completed stage: it rolls back to a savepoint and is reported as the
//! separate Index footer state.

use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::investigation::insight_persist;
use crate::investigation::insights::{
    Assessment, EvidenceLocation, EvidenceRef, Inference, PipelineInput, PipelineOutput, Readiness,
    RetainedEvidence, SourceClaim, StageCoverage,
};
use crate::store::{AtlasArticleRow, Store};

/// Prepared canonical inputs for one brief, built before the Stage-4 commit.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CanonicalBrief {
    pub input: PipelineInput,
    pub output: PipelineOutput,
}

/// Footer state of canonical publication, stored per article.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexState {
    #[serde(default)]
    pub publication_version: u32,
    /// `indexed` | `not_indexed` | `failed`
    pub state: String,
    pub snapshot_id: String,
    pub reason: String,
    pub job_id: String,
    /// Exact id/revision acknowledgements required by this publication.
    #[serde(default)]
    pub required: Vec<IndexReceipt>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexReceipt {
    pub memory_id: String,
    pub revision: String,
}

const PUBLICATION_VERSION: u32 = 4;

pub fn state_key(article_id: &str) -> String {
    format!("intel-brief-canonical:{article_id}")
}

pub fn index_state(conn: &Connection, article_id: &str) -> Result<Option<IndexState>> {
    Ok(conn
        .query_row(
            "SELECT value FROM app_state WHERE key=?1",
            [state_key(article_id)],
            |r| r.get::<_, String>(0),
        )
        .optional()?
        .and_then(|json| serde_json::from_str(&json).ok()))
}

/// Brief Links are metadata leads, so Brain stores them as tentative Related
/// memories, never as assessed corroboration. Run inside the publication
/// savepoint after the canonical inference memories exist.
fn publish_leads(conn: &Connection, article_id: &str, snapshot: &str, now: &str) -> Result<()> {
    use crate::memory_metadata::{EndpointRef, MemoryKind, RelatedPayload};
    let key = format!("intel-brief-brain-leads:{article_id}");
    let previous: Vec<String> = conn
        .query_row("SELECT value FROM app_state WHERE key=?1", [&key], |r| {
            r.get::<_, String>(0)
        })
        .optional()?
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    let mut statement = conn.prepare(
        "SELECT d.memory_id,l.candidate_run_id,l.candidate_article_id,l.candidate_metadata_revision,l.interpretation,l.verification_needed,l.id
         FROM intel_inference_article_leads l JOIN insight_derivations d
         ON d.snapshot_id=?2 AND d.insight_kind='inference' AND d.status='assessed'
         AND json_extract(d.item_json,'$.id')=l.inference_id
         JOIN atlas_articles a ON a.run_id=l.candidate_run_id AND a.article_id=l.candidate_article_id
         WHERE l.article_id=?1 AND d.memory_id IS NOT NULL ORDER BY l.id")?;
    let rows = statement
        .query_map(params![article_id, snapshot], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut current = Vec::new();
    for (memory_id, run_id, candidate, revision, interpretation, gap, lead_id) in rows {
        let payload = RelatedPayload {
            source: EndpointRef::Memory {
                memory_id,
                kind: MemoryKind::Inference,
            },
            target: EndpointRef::Article {
                run_id,
                article_id: candidate,
                revision: revision.clone(),
            },
            shared_actors: vec![],
            basis: format!("intel-brief:{article_id}:metadata-lead"),
            interpretation,
            verification_gap: gap,
            source_revisions: vec![revision, format!("intel-brief-lead:{lead_id}")],
        };
        let (id, _) = crate::brain_actors::write_related(conn, &payload, now)?;
        crate::store::enqueue_memory_index_on(conn, std::slice::from_ref(&id), None)?;
        current.push(id);
    }
    for id in previous.iter().filter(|id| !current.contains(id)) {
        conn.execute(
            "UPDATE memory_metadata SET lifecycle='retired',updated_at=?2 WHERE memory_id=?1",
            params![id, now],
        )?;
        crate::store::enqueue_memory_index_on(conn, std::slice::from_ref(id), None)?;
    }
    conn.execute("INSERT INTO app_state(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![key,serde_json::to_string(&current)?])?;
    Ok(())
}

/// A shared inference can survive its originating article. Retire that
/// article's tentative leads independently of the inference endpoint's life.
pub(crate) fn retire_deleted_article_leads(conn: &Connection, now: &str) -> Result<()> {
    let mut statement=conn.prepare("SELECT DISTINCT m.memory_id FROM app_state s,json_each(CASE WHEN s.key LIKE 'intel-brief-brain-leads:%' THEN s.value ELSE '[]' END) j JOIN memory_metadata m ON m.memory_id=j.value WHERE s.key LIKE 'intel-brief-brain-leads:%' AND m.lifecycle='active' AND NOT EXISTS(SELECT 1 FROM atlas_articles a WHERE a.article_id=substr(s.key,length('intel-brief-brain-leads:')+1))")?;
    let ids = statement
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for id in ids {
        conn.execute(
            "UPDATE memory_metadata SET lifecycle='retired',updated_at=?2 WHERE memory_id=?1",
            params![id, now],
        )?;
        crate::store::enqueue_memory_index_on(conn, std::slice::from_ref(&id), None)?;
    }
    Ok(())
}

fn faithful() -> Assessment {
    Assessment {
        fidelity: "faithful".into(),
        relation: "supports".into(),
        threshold_passed: true,
        rationale: "Claim assessor fidelity batch over the cited passages".into(),
        gaps: vec![],
    }
}

fn with_range(base: &EvidenceRef, start: usize, end: usize) -> EvidenceRef {
    let mut r = base.clone();
    r.location = EvidenceLocation::Range(crate::nlp::Span {
        start_byte: start,
        end_byte: end,
    });
    r
}

fn span_refs(
    item: &Value,
    key: &str,
    base: &EvidenceRef,
    offset_for: impl Fn(&str) -> usize,
) -> Vec<EvidenceRef> {
    item[key]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| {
            let id = s["id"].as_str()?;
            let off = offset_for(id);
            Some(with_range(
                base,
                off + s["start_byte"].as_u64()? as usize,
                off + s["end_byte"].as_u64()? as usize,
            ))
        })
        .collect()
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| s.as_str().map(str::to_string))
        .filter(|s| !s.trim().is_empty())
        .collect()
}

/// Metadata claims publish independently. Inferences require a complete retained
/// article plus Synthesis coverage of their cited context, rather than a claim
/// that the model read the whole story.
pub fn readiness(
    evidence: &[RetainedEvidence],
    coverage: &[StageCoverage],
    output: &PipelineOutput,
) -> Readiness {
    let mut out = Readiness::default();
    let body: Vec<_> = evidence
        .iter()
        .filter(|e| e.reference.source_id.starts_with("body:"))
        .collect();
    if !output.inferences.is_empty()
        && !body.iter().any(|e| e.complete_story && !e.known_truncation)
    {
        out.gaps
            .push("complete retained acquisition story is unavailable".into());
    }
    if !output.inferences.iter().all(|i| {
        i.evidence_refs.iter().all(|r| {
            coverage.iter().any(|c| {
                c.stage == "synthesis"
                    && c.source_id == r.source_id
                    && c.artifact_hash == r.artifact_hash
                    && match &r.location {
                        EvidenceLocation::Range(span) => c.ranges.iter().any(|range| {
                            range.start_byte <= span.start_byte && range.end_byte >= span.end_byte
                        }),
                        _ => false,
                    }
            })
        })
    }) {
        out.gaps
            .push("Synthesis has not analyzed a cited inference passage".into());
    }
    let resolvable = |refs: &[EvidenceRef]| {
        !refs.is_empty()
            && refs
                .iter()
                .all(|r| evidence.iter().any(|e| e.validate_ref(r)))
    };
    if !output
        .claims
        .iter()
        .all(|c| c.assessment.faithful() && resolvable(&c.evidence_refs))
        || !output
            .inferences
            .iter()
            .all(|i| i.assessment.faithful() && resolvable(&i.evidence_refs))
    {
        out.gaps
            .push("a published item lacks a faithful cited passage".into());
    }
    if output.claims.is_empty() && output.inferences.is_empty() {
        out.gaps.push("no faithful claims or inferences".into());
    }
    out.input_eligible = out.gaps.is_empty();
    out.publication_eligible = out.input_eligible;
    out
}

/// Build canonical inputs from the retained evidence and the committed stage
/// outputs. Returns Err(reason) as `Ok(Err)` when the brief cannot map onto
/// retained evidence (the Index footer then names it).
pub fn prepare(
    store: &Store,
    article: &AtlasArticleRow,
    body: &Value,
    claims: &Value,
    inferences: &Value,
) -> Result<std::result::Result<CanonicalBrief, String>> {
    prepare_for_stage(
        store,
        article,
        body,
        claims,
        inferences,
        "intel_brief_inferences",
    )
}

/// Prepare the completed kind without requiring an unfinished sibling branch.
pub fn prepare_for_stage(
    store: &Store,
    article: &AtlasArticleRow,
    body: &Value,
    claims: &Value,
    inferences: &Value,
    operation: &str,
) -> Result<std::result::Result<CanonicalBrief, String>> {
    let claims_only = operation == "intel_brief_claims";
    let generation = insight_persist::desired_generation(&store.conn, "article", &article.id)?;
    let mut input =
        insight_persist::article_input(store, article, generation, "intel brief", false)?;
    input.surface = "article".into();
    input.required_tasks_complete = true;
    let markdown = body["markdown"].as_str().unwrap_or("");
    let body_ev = input
        .evidence
        .iter()
        .find(|e| e.reference.source_id.starts_with("body:"))
        .cloned();
    if !claims_only && body_ev.is_none() {
        return Ok(Err("retained article body unavailable".into()));
    }
    if !claims_only && body_ev.as_ref().is_some_and(|e| e.text != markdown) {
        return Ok(Err(
            "displayed body differs from the retained clean artifact".into(),
        ));
    }
    // Evidence spans come from the artifact's sentence index; a persisted
    // index must describe exactly the displayed artifact.
    if !claims_only
        && !body["sentence_index"].is_null()
        && !serde_json::from_value::<super::sentence_index::SentenceIndex>(
            body["sentence_index"].clone(),
        )
        .is_ok_and(|i| i.validates(markdown))
    {
        return Ok(Err(
            "sentence index does not match the displayed artifact".into()
        ));
    }
    let meta_ev = input
        .evidence
        .iter()
        .find(|e| e.reference.source_id.starts_with("article:"))
        .cloned()
        .expect("article_input always retains metadata");
    let title_offset = article.title.len() + 1;
    let meta_offset = |id: &str| if id.starts_with('d') { title_offset } else { 0 };
    let mut output = PipelineOutput {
        entities: input.snapshot.entities.clone(),
        ..Default::default()
    };
    for c in claims["claims"].as_array().into_iter().flatten() {
        if c["fidelity"] != "faithful" {
            continue;
        }
        output.claims.push(SourceClaim {
            id: c["id"].as_str().unwrap_or("").into(),
            subject_ref: c["subject_id"].as_str().unwrap_or("").into(),
            predicate: c["predicate"].as_str().unwrap_or("").into(),
            object: c["object"].as_str().unwrap_or("").into(),
            text: c["text"].as_str().unwrap_or("").into(),
            polarity: c["polarity"].as_str().unwrap_or("positive").into(),
            modality: c["modality"].as_str().unwrap_or("asserted").into(),
            attributed_to: c["attributed_to"].as_str().unwrap_or("").into(),
            temporal_scope: c["time"].as_str().unwrap_or("").into(),
            evidence_refs: span_refs(c, "evidence", &meta_ev.reference, meta_offset),
            extraction_scope: format!("{}:metadata", super::brief_pipeline::CANONICAL_CONTRACT),
            assessment: faithful(),
            support_receipts: vec![],
        });
    }
    for i in inferences["inferences"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|_| !claims_only)
    {
        if i["fidelity"] != "faithful" {
            continue;
        }
        let body_reference = &body_ev
            .as_ref()
            .expect("inference body checked above")
            .reference;
        let mut evidence_refs = span_refs(i, "evidence", body_reference, |_| 0);
        for reference in span_refs(i, "counterevidence", body_reference, |_| 0) {
            if !evidence_refs.contains(&reference) {
                evidence_refs.push(reference);
            }
        }
        output.inferences.push(Inference {
            id: i["id"].as_str().unwrap_or("").into(),
            text: i["text"].as_str().unwrap_or("").into(),
            entity_refs: vec![],
            premise_claim_ids: strings(&i["premise_claim_ids"]),
            // References retain supporting and contrary passages. Their
            // presence records provenance; it does not assert corroboration.
            evidence_refs,
            rationale: i["why"].as_str().unwrap_or("").into(),
            limitations: strings(&i["limitations"]),
            assessment: faithful(),
            snapshot_key: String::new(),
        });
    }
    output.withheld_count = claims["withheld"].as_u64().unwrap_or(0) as usize
        + inferences["withheld"].as_u64().unwrap_or(0) as usize;
    let full = |stage: &str, e: &RetainedEvidence| StageCoverage {
        stage: stage.into(),
        source_id: e.reference.source_id.clone(),
        artifact_hash: e.reference.artifact_hash.clone(),
        ranges: vec![crate::nlp::Span {
            start_byte: 0,
            end_byte: e.text.len(),
        }],
        native_complete: true,
    };
    input.snapshot.coverage_by_stage = vec![full("claim_assessor", &meta_ev)];
    if claims_only {
        input
            .evidence
            .retain(|e| e.reference.source_id.starts_with("article:"));
        input
            .snapshot
            .source_refs
            .retain(|e| e.source_id.starts_with("article:"));
    } else if let Some(body_ev) = &body_ev {
        let ranges = if inferences["path"] == "claim_context" {
            let ids = strings(&inferences["analysis_sentence_ids"]);
            super::sentence_index::body_sentences(body)
                .0
                .into_iter()
                .filter(|s| ids.contains(&s.id))
                .map(|s| crate::nlp::Span {
                    start_byte: s.start_byte,
                    end_byte: s.end_byte,
                })
                .collect()
        } else {
            // Cached legacy extraction receipts predate per-request sentence ids.
            output
                .inferences
                .iter()
                .flat_map(|i| &i.evidence_refs)
                .filter_map(|r| match &r.location {
                    EvidenceLocation::Range(span) => Some(*span),
                    _ => None,
                })
                .collect()
        };
        input.snapshot.coverage_by_stage.push(StageCoverage {
            stage: "synthesis".into(),
            source_id: body_ev.reference.source_id.clone(),
            artifact_hash: body_ev.reference.artifact_hash.clone(),
            ranges,
            native_complete: false,
        });
    }
    output.readiness = readiness(&input.evidence, &input.snapshot.coverage_by_stage, &output);
    let completed_items = if claims_only {
        &claims["claims"]
    } else {
        &inferences["inferences"]
    };
    if completed_items.is_array() {
        // A validated empty extraction replaces the previous same-kind
        // association set. It is not a missing/unfinished stage.
        output
            .readiness
            .gaps
            .retain(|gap| gap != "no faithful claims or inferences");
        output.readiness.input_eligible = output.readiness.gaps.is_empty();
        output.readiness.publication_eligible = output.readiness.input_eligible;
    }
    input.snapshot.assessed_claim_refs = output.claims.iter().map(|c| c.id.clone()).collect();
    Ok(Ok(CanonicalBrief { input, output }))
}

fn record(conn: &Connection, article_id: &str, state: &IndexState) -> Result<()> {
    conn.execute(
        "INSERT INTO app_state(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![state_key(article_id), serde_json::to_string(state)?],
    )?;
    Ok(())
}

/// Compatibility entry point for callers holding a complete brief. The
/// automatic lane calls [`publish_stage_on`] as each kind completes.
pub fn publish_on(
    tx: &Connection,
    article_id: &str,
    job_id: &str,
    prepared: Option<&std::result::Result<CanonicalBrief, String>>,
    snapshot_hash: &str,
    now: &str,
) -> Result<IndexState> {
    let claims = prepared.cloned().map(|result| {
        result.map(|mut brief| {
            let completed = brief.output.readiness.publication_eligible;
            brief.output.inferences.clear();
            brief
                .input
                .evidence
                .retain(|e| e.reference.source_id.starts_with("article:"));
            brief
                .input
                .snapshot
                .source_refs
                .retain(|r| r.source_id.starts_with("article:"));
            brief
                .input
                .snapshot
                .coverage_by_stage
                .retain(|c| c.stage != "synthesis");
            brief.output.readiness = readiness(
                &brief.input.evidence,
                &brief.input.snapshot.coverage_by_stage,
                &brief.output,
            );
            if completed {
                brief
                    .output
                    .readiness
                    .gaps
                    .retain(|gap| gap != "no faithful claims or inferences");
                brief.output.readiness.input_eligible = brief.output.readiness.gaps.is_empty();
                brief.output.readiness.publication_eligible = brief.output.readiness.input_eligible;
            }
            brief
        })
    });
    publish_stage_on(
        tx,
        article_id,
        job_id,
        "intel_brief_claims",
        claims.as_ref(),
        snapshot_hash,
        now,
    )?;
    publish_stage_on(
        tx,
        article_id,
        job_id,
        "intel_brief_inferences",
        prepared,
        snapshot_hash,
        now,
    )?;
    publish_stage_on(
        tx,
        article_id,
        job_id,
        "intel_brief_links",
        prepared,
        snapshot_hash,
        now,
    )
}

/// Inside the fenced completed-kind transaction. Never returns an error for a
/// canonical failure: the savepoint is rolled back and the footer records it.
pub fn publish_stage_on(
    tx: &Connection,
    article_id: &str,
    job_id: &str,
    operation: &str,
    prepared: Option<&std::result::Result<CanonicalBrief, String>>,
    snapshot_hash: &str,
    now: &str,
) -> Result<IndexState> {
    let receipt_key = format!("intel-brief-canonical-stage:{job_id}:{operation}");
    let existing: Option<String> = tx
        .query_row(
            "SELECT value FROM app_state WHERE key=?1",
            [&receipt_key],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(receipt) = existing.and_then(|s| serde_json::from_str::<Value>(&s).ok()) {
        if receipt["snapshot_hash"] == snapshot_hash {
            if let Ok(state) = serde_json::from_value::<IndexState>(receipt["state"].clone()) {
                if state.publication_version == PUBLICATION_VERSION && state.state == "indexed" {
                    record(tx, article_id, &state)?;
                    return Ok(state);
                }
            }
        }
    }
    let previous = index_state(tx, article_id)?.filter(|s| s.job_id == job_id);
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
    match prepared {
        None => {
            state.state = "not_indexed".into();
            state.reason = "canonical inputs not prepared".into();
        }
        Some(Err(reason)) => {
            state.state = "not_indexed".into();
            state.reason = reason.clone();
        }
        Some(Ok(brief)) if !brief.output.readiness.publication_eligible => {
            state.state = "not_indexed".into();
            state.reason = brief.output.readiness.gaps.join("; ");
        }
        Some(Ok(brief)) => {
            tx.execute_batch("SAVEPOINT brief_canonical")?;
            let attempt = (|| -> Result<Option<(String, Vec<IndexReceipt>)>> {
                let mut input = brief.input.clone();
                let mut output = brief.output.clone();
                if operation == "intel_brief_links" {
                    let inference_receipt: Option<String> = tx
                        .query_row(
                            "SELECT value FROM app_state WHERE key=?1",
                            [format!(
                                "intel-brief-canonical-stage:{job_id}:intel_brief_inferences"
                            )],
                            |r| r.get(0),
                        )
                        .optional()?;
                    let inference_receipt = inference_receipt
                        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
                        .unwrap_or(Value::Null);
                    anyhow::ensure!(
                        inference_receipt["state"]["state"] == "indexed",
                        "canonical inference publication unavailable"
                    );
                    let raw: Option<String> = tx.query_row("SELECT s.inputs_json FROM insight_snapshots s JOIN insight_owner_state h ON h.active_snapshot_id=s.id WHERE h.owner_kind='article' AND h.owner_id=?1", [article_id], |r|r.get(0)).optional()?;
                    let raw = raw.ok_or_else(|| {
                        anyhow::anyhow!("canonical inference publication unavailable")
                    })?;
                    let snapshot: crate::investigation::insights::EvidenceSnapshot =
                        serde_json::from_str(&raw)?;
                    insight_persist::validate_live_evidence(tx, &snapshot)?;
                    let id: String = tx.query_row("SELECT active_snapshot_id FROM insight_owner_state WHERE owner_kind='article' AND owner_id=?1", [article_id], |r|r.get(0))?;
                    anyhow::ensure!(
                        inference_receipt["state"]["snapshot_id"] == id,
                        "canonical inference snapshot was replaced before Links publication"
                    );
                    let missing:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM intel_inference_article_leads l LEFT JOIN insight_derivations d ON d.snapshot_id=?2 AND d.insight_kind='inference' AND d.status='assessed' AND json_extract(d.item_json,'$.id')=l.inference_id LEFT JOIN memory_metadata m ON m.memory_id=d.memory_id AND m.memory_kind='inference' AND m.lifecycle='active' WHERE l.article_id=?1 AND l.job_id=?3 AND (d.memory_id IS NULL OR m.memory_id IS NULL))",params![article_id,id,job_id],|r|r.get(0))?;
                    anyhow::ensure!(
                        !missing,
                        "a Link endpoint lacks its current canonical inference"
                    );
                    publish_leads(tx, article_id, &id, now)?;
                    let required = required_index_receipts(tx, article_id, &id)?;
                    return Ok(Some((id, required)));
                }
                input.snapshot.generation =
                    insight_persist::advance_generation(tx, "article", article_id, now)?;
                input.snapshot.key = crate::nlp::cache::key(&serde_json::json!([
                    super::brief_pipeline::CANONICAL_CONTRACT,
                    input.snapshot.source_refs,
                    snapshot_hash
                ]));
                for inference in &mut output.inferences {
                    inference.snapshot_key = input.snapshot.key.clone();
                }
                let key = input.snapshot.key.clone();
                let snapshot = insight_persist::publish_snapshot_on(
                    tx,
                    &input.snapshot,
                    &key,
                    "active",
                    now,
                    |tx, snapshot_id| {
                        insight_persist::write_article_derivations_for_kind(
                            tx,
                            snapshot_id,
                            &input,
                            &output,
                            now,
                            if operation == "intel_brief_claims" {
                                "claim"
                            } else {
                                "inference"
                            },
                        )
                    },
                )?;
                snapshot
                    .map(|id| {
                        let required = required_index_receipts(tx, article_id, &id)?;
                        Ok((id, required))
                    })
                    .transpose()
            })();
            match attempt {
                Ok(Some((id, required))) => {
                    tx.execute_batch("RELEASE brief_canonical")?;
                    state.state = "indexed".into();
                    state.snapshot_id = id;
                    state.required = required;
                }
                Ok(None) => {
                    tx.execute_batch("ROLLBACK TO brief_canonical; RELEASE brief_canonical")?;
                    state.state = "not_indexed".into();
                    state.reason = "stale canonical generation".into();
                }
                Err(error) => {
                    tx.execute_batch("ROLLBACK TO brief_canonical; RELEASE brief_canonical")?;
                    state.state = "failed".into();
                    state.reason = error.to_string();
                }
            }
        }
    }
    tx.execute("INSERT INTO app_state(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![receipt_key,serde_json::json!({"snapshot_hash":snapshot_hash,"state":state}).to_string()])?;
    record(tx, article_id, &state)?;
    Ok(state)
}

fn required_index_receipts(
    conn: &Connection,
    article_id: &str,
    snapshot: &str,
) -> Result<Vec<IndexReceipt>> {
    let mut statement = conn.prepare("SELECT DISTINCT m.id,m.text FROM memories m WHERE m.id IN (
        SELECT memory_id FROM insight_derivations WHERE snapshot_id=?1 AND status='assessed'
        UNION SELECT actor_id FROM brain_memory_actor_refs WHERE memory_id IN (SELECT memory_id FROM insight_derivations WHERE snapshot_id=?1 AND status='assessed')
        UNION SELECT value FROM json_each(COALESCE((SELECT value FROM app_state WHERE key=?2),'[]')))")?;
    let mut receipts = statement
        .query_map(
            params![snapshot, format!("intel-brief-brain-leads:{article_id}")],
            |r| {
                let text: String = r.get(1)?;
                Ok(IndexReceipt {
                    memory_id: r.get(0)?,
                    revision: crate::store::memory_revision(&text),
                })
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut removed = conn.prepare("SELECT DISTINCT d.memory_id FROM insight_derivations d JOIN insight_snapshots s ON s.id=d.snapshot_id WHERE s.owner_kind='article' AND s.owner_id=?1 AND d.memory_id IS NOT NULL AND NOT EXISTS(SELECT 1 FROM memories m WHERE m.id=d.memory_id) AND EXISTS(SELECT 1 FROM argos_index_changes c WHERE c.record_id=d.memory_id AND c.revision=?2 AND c.operation='index_remove')")?;
    for id in removed.query_map(params![article_id, crate::store::DELETED_REVISION], |r| {
        r.get::<_, String>(0)
    })? {
        receipts.push(IndexReceipt {
            memory_id: id?,
            revision: crate::store::DELETED_REVISION.into(),
        });
    }
    Ok(receipts)
}

/// Revalidate and publish cached artifacts on the next visit after the
/// publication-policy change. Extraction keys and fidelity receipts stay valid.
pub fn refresh_cached_publication(
    store: &Store,
    article: &AtlasArticleRow,
    job: &str,
) -> Result<()> {
    // Rebuild exactly the metadata revision that extraction saw. The live
    // publication fence then rejects edits rather than remapping old offsets.
    let mut article = article.clone();
    let captured: Option<String> = store
        .conn
        .query_row(
            "SELECT input_ref FROM argos_tasks WHERE id=?1",
            [super::brief_pipeline::task_id(job, "intel_brief_claims")],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(metadata) = captured
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .map(|s| s["metadata"].clone())
    {
        if let Some(title) = metadata["title"].as_str() {
            article.title = title.into();
        }
        if let Some(description) = metadata["description"].as_str() {
            article.description = description.into();
        }
        if let Some(published) = metadata["published_at"].as_str() {
            article.published_at = published.into();
        }
    }
    let outputs = super::brief_pipeline::stage_outputs(&store.conn, job)?;
    for operation in [
        "intel_brief_claims",
        "intel_brief_inferences",
        "intel_brief_links",
    ] {
        if !outputs.contains_key(operation) {
            continue;
        }
        let prepared = prepare_for_stage(
            store,
            &article,
            outputs.get("intel_brief_body").unwrap_or(&Value::Null),
            outputs.get("intel_brief_claims").unwrap_or(&Value::Null),
            outputs
                .get("intel_brief_inferences")
                .unwrap_or(&Value::Null),
            operation,
        )?;
        let tx = rusqlite::Transaction::new_unchecked(
            &store.conn,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        super::brief_pipeline::publish_completed_stage(
            &tx,
            job,
            operation,
            Some(&prepared),
            &chrono::Utc::now().to_rfc3339(),
        )?;
        tx.commit()?;
    }
    Ok(())
}

/// Publication / Index footer text for one chain. The index part is separate
/// from the stage rows: an index failure never fails a completed stage.
pub fn footer(
    conn: &Connection,
    article_id: &str,
    job_id: &str,
    any_failed: bool,
) -> Result<String> {
    let published: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM intel_brief_publications WHERE article_id=?1 AND job_id=?2)",
        params![article_id, job_id],
        |r| r.get(0),
    )?;
    if !published {
        return Ok(if any_failed {
            "Not published · previous snapshot kept".into()
        } else {
            "Pending · after Claims".into()
        });
    }
    let state = index_state(conn, article_id)?.filter(|s| s.job_id == job_id);
    let snapshot = state
        .as_ref()
        .map(|state| state.snapshot_id.as_str())
        .unwrap_or("");
    let canonical_published:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM insight_snapshots WHERE id=?1 AND owner_kind='article' AND owner_id=?2) OR EXISTS(SELECT 1 FROM app_state receipt JOIN insight_snapshots snapshot ON snapshot.id=json_extract(CASE WHEN json_valid(receipt.value) THEN receipt.value ELSE '{}' END,'$.state.snapshot_id') WHERE substr(receipt.key,1,length(?3))=?3 AND json_extract(CASE WHEN json_valid(receipt.value) THEN receipt.value ELSE '{}' END,'$.state.state')='indexed' AND snapshot.owner_kind='article' AND snapshot.owner_id=?2)",params![snapshot,article_id,format!("intel-brief-canonical-stage:{job_id}:")],|r|r.get(0))?;
    if !canonical_published {
        let reason = state
            .as_ref()
            .map(|state| state.reason.as_str())
            .filter(|reason| !reason.trim().is_empty())
            .unwrap_or("canonical publication pending");
        return Ok(format!("Not published · {reason}"));
    }
    let index = match state {
        None => "Index pending".to_string(),
        Some(s) => match s.state.as_str() {
            "indexed" => {
                let mut pending = false;
                let mut failed = None;
                let mut blocked = None;
                for receipt in &s.required {
                    let ready = if receipt.revision == crate::store::DELETED_REVISION {
                        conn.query_row("SELECT NOT EXISTS(SELECT 1 FROM memories WHERE id=?1) AND NOT EXISTS(SELECT 1 FROM argos_memory_index_state WHERE memory_id=?1) AND COALESCE((SELECT state='completed' FROM argos_index_changes WHERE record_id=?1 AND revision=?2 AND operation='index_remove' AND state<>'superseded' ORDER BY seq DESC LIMIT 1),0)",params![receipt.memory_id,receipt.revision],|r|r.get::<_,bool>(0))?
                    } else {
                        let acknowledged: bool = conn.query_row(
                            "SELECT EXISTS(SELECT 1 FROM argos_memory_index_state WHERE memory_id=?1 AND revision=?2)",
                            params![receipt.memory_id,receipt.revision],
                            |r|r.get(0),
                        )?;
                        let live_text: Option<String> = conn
                            .query_row(
                                "SELECT text FROM memories WHERE id=?1",
                                [&receipt.memory_id],
                                |r| r.get(0),
                            )
                            .optional()?;
                        acknowledged
                            && live_text
                                .as_deref()
                                .map(crate::store::memory_revision)
                                .as_deref()
                                == Some(receipt.revision.as_str())
                    };
                    // An exact live acknowledgement wins over old failures,
                    // superseded work and blocked rows from another revision.
                    if ready {
                        continue;
                    }
                    pending = true;
                    let work:Option<(String,String,String,String)>=conn.query_row("SELECT state,outcome,error_category,error_message FROM argos_index_changes WHERE record_kind='memory' AND record_id=?1 AND revision=?2 AND state<>'superseded' ORDER BY seq DESC LIMIT 1",params![receipt.memory_id,receipt.revision],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
                    if let Some((state, outcome, category, message)) = work {
                        let outcome: Value = serde_json::from_str(&outcome).unwrap_or(Value::Null);
                        let reason = if !message.trim().is_empty() {
                            message
                        } else {
                            outcome["reason"]
                                .as_str()
                                .or_else(|| outcome["message"].as_str())
                                .filter(|s| !s.trim().is_empty())
                                .map(str::to_string)
                                .unwrap_or_else(|| {
                                    if category.is_empty() {
                                        format!("index work {state}")
                                    } else {
                                        category
                                    }
                                })
                        };
                        if state == "failed" {
                            failed.get_or_insert(reason);
                        } else if ["blocked", "deferred"].contains(&state.as_str())
                            || outcome["status"] == "disabled"
                            || outcome["status"] == "deferred"
                        {
                            blocked.get_or_insert(reason);
                        }
                    }
                }
                if let Some(reason) = failed {
                    format!("Index failed · {reason}")
                } else if let Some(reason) = blocked {
                    format!("Not indexed · {reason}")
                } else if pending {
                    "Index pending".into()
                } else {
                    "Indexed".into()
                }
            }

            "failed" => format!("Index failed · {}", s.reason),
            _ => format!("Not indexed · {}", s.reason),
        },
    };
    Ok(format!("Published · {index}"))
}

#[cfg(test)]
#[path = "brief_canonical_tests.rs"]
mod tests;
