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

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::investigation::insights::{
    Assessment, EvidenceLocation, EvidenceRef, Inference, PipelineInput, PipelineOutput, Readiness,
    RetainedEvidence, SourceClaim, StageCoverage,
};
use crate::store::AtlasArticleRow;

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

pub const PUBLICATION_VERSION: u32 = 4;

pub fn state_key(article_id: &str) -> String {
    format!("intel-brief-canonical:{article_id}")
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

/// Build canonical inputs for one completed brief kind from the retained
/// article input (`storage::pipeline_inputs::article_input`) and the committed
/// stage outputs. `Err(reason)` when the brief cannot map onto retained
/// evidence (the Index footer then names it).
pub fn prepare_for_stage(
    mut input: PipelineInput,
    article: &AtlasArticleRow,
    body: &Value,
    claims: &Value,
    inferences: &Value,
    operation: &str,
) -> std::result::Result<CanonicalBrief, String> {
    let claims_only = operation == "intel_brief_claims";
    input.surface = "article".into();
    input.required_tasks_complete = true;
    let markdown = body["markdown"].as_str().unwrap_or("");
    let body_ev = input
        .evidence
        .iter()
        .find(|e| e.reference.source_id.starts_with("body:"))
        .cloned();
    if !claims_only && body_ev.is_none() {
        return Err("retained article body unavailable".into());
    }
    if !claims_only && body_ev.as_ref().is_some_and(|e| e.text != markdown) {
        return Err("displayed body differs from the retained clean artifact".into());
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
        return Err("sentence index does not match the displayed artifact".into());
    }
    let meta_ev = input
        .evidence
        .iter()
        .find(|e| e.reference.source_id.starts_with("article:"))
        .cloned()
        .ok_or_else(|| "retained article metadata unavailable".to_string())?;
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
    Ok(CanonicalBrief { input, output })
}

#[cfg(test)]
#[path = "brief_canonical_tests.rs"]
mod tests;
