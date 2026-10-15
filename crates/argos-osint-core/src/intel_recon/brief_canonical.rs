//! Canonical Brain/index publication from a completed intel brief.
//!
//! The brief lane reuses the shared canonical publisher
//! ([`insight_persist::publish_snapshot_on`] + [`insight_persist::write_derivations`]);
//! only its fence (the fenced Stage-4 commit) and its readiness differ from the
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
}

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

/// Brief readiness: a complete retained acquisition story, Synthesis over every
/// body window, and every published item faithful with resolvable citations.
/// The Claim assessor is scoped to the cited passages (spec §4), so story-range
/// coverage is required of Synthesis, not of the assessor.
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
    if !body.iter().any(|e| e.complete_story && !e.known_truncation) {
        out.gaps
            .push("complete retained acquisition story is unavailable".into());
    }
    if !body.iter().all(|e| {
        coverage
            .iter()
            .any(|c| c.stage == "synthesis" && c.covers(e))
    }) {
        out.gaps
            .push("Synthesis has not analyzed every retained story range".into());
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
    let generation = insight_persist::desired_generation(&store.conn, "article", &article.id)?;
    let mut input =
        insight_persist::article_input(store, article, generation, "intel brief", false)?;
    input.surface = "article".into();
    input.required_tasks_complete = true;
    let markdown = body["markdown"].as_str().unwrap_or("");
    let Some(body_ev) = input
        .evidence
        .iter()
        .find(|e| e.reference.source_id.starts_with("body:"))
        .cloned()
    else {
        return Ok(Err("retained article body unavailable".into()));
    };
    if body_ev.text != markdown {
        return Ok(Err(
            "displayed body differs from the retained clean artifact".into(),
        ));
    }
    // Evidence spans come from the artifact's sentence index; a persisted
    // index must describe exactly the displayed artifact.
    if !body["sentence_index"].is_null()
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
    for i in inferences["inferences"].as_array().into_iter().flatten() {
        if i["fidelity"] != "faithful" {
            continue;
        }
        output.inferences.push(Inference {
            id: i["id"].as_str().unwrap_or("").into(),
            text: i["text"].as_str().unwrap_or("").into(),
            entity_refs: vec![],
            premise_claim_ids: strings(&i["premise_claim_ids"]),
            evidence_refs: span_refs(i, "evidence", &body_ev.reference, |_| 0),
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
    // Stage 3 sends every ordered body window; Stage 2 reads all metadata.
    input.snapshot.coverage_by_stage = vec![
        full("synthesis", &body_ev),
        full("claim_assessor", &meta_ev),
    ];
    output.readiness = readiness(&input.evidence, &input.snapshot.coverage_by_stage, &output);
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

/// Inside the fenced Stage-4 commit transaction. Never returns an error for a
/// canonical failure: the savepoint is rolled back and the footer records it.
pub fn publish_on(
    tx: &Connection,
    article_id: &str,
    job_id: &str,
    prepared: Option<&std::result::Result<CanonicalBrief, String>>,
    snapshot_hash: &str,
    now: &str,
) -> Result<IndexState> {
    let mut state = IndexState {
        publication_version: 3,
        job_id: job_id.into(),
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
            let attempt = (|| -> Result<Option<String>> {
                let mut input = brief.input.clone();
                let mut output = brief.output.clone();
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
                insight_persist::publish_snapshot_on(
                    tx,
                    &input.snapshot,
                    &key,
                    "active",
                    now,
                    |tx, snapshot_id| {
                        insight_persist::write_derivations(tx, snapshot_id, &input, &output, now)?;
                        publish_leads(tx, article_id, snapshot_id, now)
                    },
                )
            })();
            match attempt {
                Ok(Some(id)) => {
                    tx.execute_batch("RELEASE brief_canonical")?;
                    state.state = "indexed".into();
                    state.snapshot_id = id;
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
    record(tx, article_id, &state)?;
    Ok(state)
}

/// Revalidate and publish cached artifacts on the next visit after the
/// publication-policy change. Extraction keys and fidelity receipts stay valid.
pub fn refresh_cached_publication(
    store: &Store,
    article: &AtlasArticleRow,
    job: &str,
) -> Result<()> {
    if index_state(&store.conn, &article.id)?
        .is_some_and(|s| s.job_id == job && s.publication_version >= 3)
    {
        return Ok(());
    }
    let hash: Option<String> = store.conn.query_row("SELECT p.snapshot_hash FROM intel_brief_publications p JOIN intel_brief_runs r ON r.job_id=p.job_id WHERE p.article_id=?1 AND p.job_id=?2 AND r.state='completed' AND NOT EXISTS(SELECT 1 FROM intel_brief_runs n WHERE n.article_id=r.article_id AND n.generation>r.generation)",params![article.id,job],|r|r.get(0)).optional()?;
    let Some(hash) = hash else { return Ok(()) };
    let outputs = super::brief_pipeline::stage_outputs(&store.conn, job)?;
    let (Some(body), Some(claims), Some(inferences)) = (
        outputs.get("intel_brief_body"),
        outputs.get("intel_brief_claims"),
        outputs.get("intel_brief_inferences"),
    ) else {
        return Ok(());
    };
    let prepared = prepare(store, article, body, claims, inferences)?;
    let tx = rusqlite::Transaction::new_unchecked(
        &store.conn,
        rusqlite::TransactionBehavior::Immediate,
    )?;
    let current:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM intel_brief_publications p JOIN intel_brief_runs r ON r.job_id=p.job_id WHERE p.article_id=?1 AND p.job_id=?2 AND p.snapshot_hash=?3 AND NOT EXISTS(SELECT 1 FROM intel_brief_runs n WHERE n.article_id=r.article_id AND n.generation>r.generation))",params![article.id,job,hash],|r|r.get(0))?;
    if current {
        publish_on(
            &tx,
            &article.id,
            job,
            Some(&prepared),
            &hash,
            &chrono::Utc::now().to_rfc3339(),
        )?;
    }
    tx.commit()?;
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
            "Pending · after Links".into()
        });
    }
    let state = index_state(conn, article_id)?.filter(|s| s.job_id == job_id);
    let index = match state {
        None => "Index pending".to_string(),
        Some(s) => match s.state.as_str() {
            "indexed" => {
                let pending: bool = conn.query_row(
                    "SELECT EXISTS(SELECT 1 FROM argos_index_changes WHERE state<>'completed' AND record_id IN (SELECT memory_id FROM insight_derivations WHERE snapshot_id=?1))",
                    [&s.snapshot_id],
                    |r| r.get(0),
                )?;
                if pending {
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
