//! Evidence confidence v1: an explainable score, not a calibrated truth probability.
use std::collections::BTreeSet;

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension};

use crate::investigation::insights::{claim_fingerprint, EvidenceRef, PipelineOutput};

#[derive(Clone, Debug, Default)]
pub struct EvidenceConfidence {
    pub score: Option<u8>,
    pub claims: usize,
    pub faithful: usize,
    pub corroborated: usize,
    pub disputed: usize,
    pub complete_coverage: bool,
    pub provisional: bool,
}

fn roots(refs: &[EvidenceRef]) -> usize {
    let refs: Vec<_> = refs
        .iter()
        .filter(|r| !r.origin_root.is_empty() && !r.artifact_hash.is_empty())
        .collect();
    let mut groups: Vec<_> = (0..refs.len()).collect();
    for i in 0..refs.len() {
        for j in 0..i {
            if refs[i].origin_root == refs[j].origin_root
                || refs[i].artifact_hash == refs[j].artifact_hash
                || (!refs[i].url.is_empty() && refs[i].url == refs[j].url)
            {
                let from = groups[i];
                let to = groups[j];
                for group in &mut groups {
                    if *group == from {
                        *group = to;
                    }
                }
            }
        }
    }
    groups.into_iter().collect::<BTreeSet<_>>().len()
}

impl EvidenceConfidence {
    pub fn estimate(output: &PipelineOutput, complete_coverage: bool) -> Self {
        let claims = output.claims.len();
        if claims == 0 {
            return Self::default();
        }
        let faithful = output
            .claims
            .iter()
            .filter(|c| c.assessment.faithful())
            .count();
        let mut corroborated = 0;
        let mut strength = 0.0;
        let mut disputed = 0;
        let mut gaps = 0;
        for claim in &output.claims {
            gaps += claim.assessment.gaps.len();
            if !claim.assessment.faithful() {
                continue;
            }
            match claim.assessment.relation.as_str() {
                "supports" => {
                    let extra = roots(&claim.evidence_refs).saturating_sub(1).min(2);
                    corroborated += usize::from(extra > 0);
                    strength += extra as f64 / 2.0;
                }
                "contradicts" => disputed += 1,
                _ => {}
            }
        }
        let n = claims as f64;
        // Source material 20; faithful extraction 20; complete analysis 20;
        // independent support 40. Disputes subtract up to 40; gaps up to 15.
        let mut score = 20.0
            + 20.0 * faithful as f64 / n
            + if complete_coverage { 20.0 } else { 0.0 }
            + 40.0 * strength / n
            - 40.0 * disputed as f64 / n
            - (5.0 * gaps as f64 / n).min(15.0);
        let provisional = !complete_coverage
            || corroborated < claims
            || faithful < claims
            || disputed > 0
            || gaps > 0
            || output.partial_enumeration
            || output.limit_reached;
        if output.partial_enumeration || output.limit_reached {
            score = score.min(85.0);
        }
        Self {
            score: Some(score.clamp(0.0, 95.0).round() as u8),
            claims,
            faithful,
            corroborated,
            disputed,
            complete_coverage,
            provisional,
        }
    }
}

/// Loaded with the briefing snapshot, never from drawing. Pending owners keep
/// their previous score; additional evidence affects it only after assessment.
pub fn article_confidence(
    conn: &Connection,
    article_id: &str,
) -> Result<Option<EvidenceConfidence>> {
    let head: Option<(String,String,String)> = conn.query_row(
        "SELECT h.owner_kind,s.readiness_json,h.owner_id FROM insight_owner_state h JOIN insight_snapshots s ON s.id=h.active_snapshot_id WHERE h.owner_id=?1 AND h.owner_kind IN ('article','atlas') ORDER BY CASE h.owner_kind WHEN 'article' THEN 0 ELSE 1 END LIMIT 1",
        [article_id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
    let Some((kind, readiness, owner)) = head else {
        return Ok(None);
    };
    let Some(mut output) =
        crate::investigation::insight_persist::active_output(conn, &kind, &owner)?
    else {
        return Ok(None);
    };
    for claim in &mut output.claims {
        let text: Option<String> = conn.query_row("SELECT m.text FROM memories m JOIN insight_claims c ON c.memory_id=m.id WHERE c.fingerprint=?1", [claim_fingerprint(claim)], |r| r.get(0)).optional()?;
        if text.as_ref().is_some_and(|text| text != &claim.text) {
            claim.assessment.fidelity = "user_edited".into();
            claim.assessment.threshold_passed = false;
        }
    }
    let stored: serde_json::Value = serde_json::from_str(&readiness)?;
    let complete = stored
        .pointer("/readiness/input_eligible")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    Ok(Some(EvidenceConfidence::estimate(&output, complete)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::investigation::insights::{Assessment, EvidenceLocation, SourceClaim};
    fn evidence(root: &str, artifact: &str) -> EvidenceRef {
        EvidenceRef {
            source_id: root.into(),
            revision: "1".into(),
            artifact_hash: artifact.into(),
            acquisition_id: "1".into(),
            origin_root: root.into(),
            url: String::new(),
            published_at: None,
            retrieved_at: String::new(),
            location: EvidenceLocation::JsonPointer("/text".into()),
        }
    }
    #[test]
    fn confidence_progresses_with_assessed_evidence_and_falls_with_disputes() {
        let mut output = PipelineOutput::default();
        assert_eq!(EvidenceConfidence::estimate(&output, false).score, None);
        output.claims.push(SourceClaim {
            id: "c".into(),
            subject_ref: "e".into(),
            predicate: "p".into(),
            object: "o".into(),
            text: "source assertion".into(),
            polarity: "positive".into(),
            modality: "asserted".into(),
            attributed_to: "source".into(),
            temporal_scope: "unknown".into(),
            evidence_refs: vec![evidence("a", "1")],
            extraction_scope: String::new(),
            assessment: Assessment::default(),
        });
        assert_eq!(EvidenceConfidence::estimate(&output, false).score, Some(20));
        output.claims[0].assessment.fidelity = "faithful".into();
        output.claims[0].assessment.threshold_passed = true;
        assert_eq!(EvidenceConfidence::estimate(&output, false).score, Some(40));
        assert_eq!(EvidenceConfidence::estimate(&output, true).score, Some(60));
        output.claims[0].assessment.relation = "supports".into();
        output.claims[0].evidence_refs.push(evidence("b", "1"));
        assert_eq!(
            EvidenceConfidence::estimate(&output, true).score,
            Some(60),
            "syndication is not corroboration"
        );
        output.claims[0].evidence_refs.push(evidence("b", "2"));
        assert_eq!(
            EvidenceConfidence::estimate(&output, true).score,
            Some(60),
            "transitive common roots collapse"
        );
        output.claims[0].evidence_refs = vec![evidence("a", "1"), evidence("b", "2")];
        assert_eq!(EvidenceConfidence::estimate(&output, true).score, Some(80));
        output.claims[0].evidence_refs[0].url = "https://example.com/story".into();
        output.claims[0].evidence_refs[1].url = "https://example.com/story".into();
        assert_eq!(
            EvidenceConfidence::estimate(&output, true).score,
            Some(60),
            "updated copies of the same article are not independent"
        );
        output.claims[0].evidence_refs[0].url.clear();
        output.claims[0].evidence_refs[1].url.clear();
        output.claims[0].evidence_refs.push(evidence("c", "3"));
        assert_eq!(EvidenceConfidence::estimate(&output, true).score, Some(95));
        output.claims[0].assessment.relation = "mentions_only".into();
        assert_eq!(EvidenceConfidence::estimate(&output, true).score, Some(60));
        output.claims[0].assessment.relation = "contradicts".into();
        assert_eq!(EvidenceConfidence::estimate(&output, true).score, Some(20));
        output.claims[0].assessment.threshold_passed = false;
        assert_eq!(EvidenceConfidence::estimate(&output, true).score, Some(40));
    }
}
