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
    Assessment, EvidenceLocation, EvidenceRef, PipelineInput, PipelineOutput, Readiness,
    RetainedEvidence, StageCoverage,
};

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

#[cfg(test)]
#[path = "brief_canonical_tests.rs"]
mod tests;
