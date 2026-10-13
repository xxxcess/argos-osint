//! Evidence confidence v1: an explainable score, not a calibrated truth probability.
use std::collections::BTreeSet;

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension};

use crate::investigation::insights::{claim_fingerprint, EvidenceRef, PipelineOutput};

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct EvidenceConfidence {
    pub score: Option<u8>,
    pub claims: usize,
    pub inferences: usize,
    pub information_grade: u8,
    pub item_scores: Vec<(String, f64)>,
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
                || (!refs[i].url.is_empty()
                    && crate::atlas::canonical_url(&refs[i].url)
                        == crate::atlas::canonical_url(&refs[j].url))
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

pub const RATING_POLICY: &str = "brief-rating-v2";

pub fn initial_rating(
    source: crate::osint::source_eval::SourceReliability,
    info: crate::osint::source_eval::InformationCredibility,
) -> (Option<u8>, bool) {
    let s = source.brief_score();
    let i = info.brief_score();
    (
        match (s, i) {
            (Some(s), Some(i)) => Some(((s + i) / 2.0).round() as u8),
            (Some(s), None) => Some(s.min(60.0).round() as u8),
            (None, Some(i)) => Some(i.round() as u8),
            _ => None,
        },
        i.is_none(),
    )
}

impl EvidenceConfidence {
    pub fn estimate(output: &PipelineOutput, complete_coverage: bool) -> Self {
        Self::estimate_for_source(
            output,
            complete_coverage,
            crate::osint::source_eval::SourceReliability::F,
            false,
        )
    }

    pub fn estimate_for_source(
        output: &PipelineOutput,
        complete_coverage: bool,
        publisher: crate::osint::source_eval::SourceReliability,
        report: bool,
    ) -> Self {
        use crate::osint::source_eval::InformationCredibility;
        let mut result = Self {
            complete_coverage,
            information_grade: 6,
            provisional: !complete_coverage || output.partial_enumeration || output.limit_reached,
            ..Default::default()
        };
        let mut propositions = std::collections::BTreeMap::<
            String,
            Vec<&crate::investigation::insights::SourceClaim>,
        >::new();
        for claim in &output.claims {
            propositions
                .entry(claim_fingerprint(claim))
                .or_default()
                .push(claim);
        }
        let mut scores = std::collections::HashMap::new();
        let mut grades = Vec::new();
        let mut supported = 0;
        let mut unresolved = false;
        for (fp, claims) in propositions {
            result.claims += 1;
            let faithful = claims
                .iter()
                .any(|c| c.assessment.faithful() && !c.evidence_refs.is_empty());
            result.faithful += usize::from(faithful);
            if claims.iter().any(|c| {
                c.assessment.faithful()
                    && c.support_receipts
                        .iter()
                        .any(|r| r.threshold_passed && r.relation == "contradicts")
            }) {
                result.disputed += 1;
                continue;
            }
            if !faithful {
                unresolved = true;
                continue;
            }
            let refs: Vec<_> = claims
                .iter()
                .filter(|c| c.assessment.faithful())
                .flat_map(|c| &c.support_receipts)
                .filter(|r| {
                    r.threshold_passed
                        && r.relation == "supports"
                        && !r.lineage.is_empty()
                        && !r.source.source_id.starts_with("report:")
                })
                .map(|r| r.source.clone())
                .collect();
            let d = roots(&refs).min(3);
            let (score, grade) = if d == 0 {
                unresolved = true;
                if report {
                    (40.0, 6)
                } else {
                    continue;
                }
            } else {
                let grade = match d {
                    1 => 3,
                    2 => 2,
                    _ => 1,
                };
                let info = InformationCredibility::from_u8(grade)
                    .unwrap()
                    .brief_score()
                    .unwrap();
                let p = publisher.brief_score().map_or(info, |s| (s + info) / 2.0);
                let w = d as f64 / (d as f64 + 2.0);
                let e = match d {
                    1 => 60.0,
                    2 => 80.0,
                    _ => 95.0,
                };
                (((1.0 - w) * p + w * e).clamp(0.0, 95.0), grade)
            };
            result.corroborated += usize::from(d >= 2);
            supported += 1;
            grades.push(grade);
            result.item_scores.push((fp, score));
            for c in claims {
                scores.insert(c.id.clone(), (score, grade));
            }
        }
        let mut seen = std::collections::HashSet::new();
        for inference in &output.inferences {
            if !seen.insert(crate::investigation::insights::inference_fingerprint(
                inference,
            )) {
                continue;
            }
            if !inference.assessment.faithful() || inference.premise_claim_ids.is_empty() {
                unresolved = true;
                continue;
            }
            let premises: Option<Vec<_>> = inference
                .premise_claim_ids
                .iter()
                .map(|id| scores.get(id).copied())
                .collect();
            let Some(premises) = premises else {
                unresolved = true;
                continue;
            };
            let score = premises.iter().map(|(s, _)| *s).fold(85.0, f64::min);
            let grade = premises.iter().map(|(_, g)| *g).max().unwrap_or(6).max(2);
            result.inferences += 1;
            grades.push(grade);
            result.item_scores.push((
                crate::investigation::insights::inference_fingerprint(inference),
                score,
            ));
        }
        let q = if supported + result.disputed > 0 {
            supported as f64 / (supported + result.disputed) as f64
        } else {
            1.0
        };
        result.score = if result.item_scores.is_empty() {
            if result.disputed > 0 && result.disputed == result.claims && complete_coverage {
                Some(0)
            } else {
                None
            }
        } else {
            Some(
                (result.item_scores.iter().map(|(_, s)| q * s).sum::<f64>()
                    / result.item_scores.len() as f64)
                    .round() as u8,
            )
        };
        result.information_grade = if !complete_coverage
            || unresolved
            || output.partial_enumeration
            || output.limit_reached
        {
            6
        } else if result.disputed == result.claims && result.claims > 0 {
            5
        } else if result.disputed > 0 {
            4
        } else {
            grades.into_iter().max().unwrap_or(6)
        };
        result.provisional |= result.information_grade == 6;
        result
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
    let publisher = conn.query_row("SELECT metadata_json FROM atlas_article_metadata WHERE article_id=?1 ORDER BY rowid DESC LIMIT 1", [article_id], |r| r.get::<_,String>(0)).optional()?
        .and_then(|s| serde_json::from_str::<crate::atlas_metadata::ArticleMetadata>(&s).ok()).and_then(|m| m.publisher_prior)
        .and_then(|p| crate::osint::source_eval::SourceReliability::parse(&p.observation.reliability)).unwrap_or(crate::osint::source_eval::SourceReliability::F);
    let report: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM insight_owner_state h JOIN insight_snapshots s ON s.id=h.active_snapshot_id WHERE h.owner_id=?1 AND s.inputs_json LIKE '%report:%')",[article_id],|r|r.get(0))?;
    Ok(Some(EvidenceConfidence::estimate_for_source(
        &output, complete, publisher, report,
    )))
}

/// Publisher metadata commits update the persisted current rating without
/// launching models or touching pending replacement generations.
pub(crate) fn persist_article_rating(conn: &Connection, article_id: &str) -> Result<()> {
    let Some(rating) = article_confidence(conn, article_id)? else {
        return Ok(());
    };
    conn.execute("UPDATE insight_snapshots SET readiness_json=json_set(readiness_json,'$.rating.estimate',json(?2),'$.rating.policy','brief-rating-v2') WHERE id IN (SELECT active_snapshot_id FROM insight_owner_state WHERE owner_kind='article' AND owner_id=?1) AND contract_version=?3",rusqlite::params![article_id,serde_json::to_string(&rating)?,crate::investigation::insights::CONTRACT])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::investigation::insights::{
        Assessment, EvidenceLocation, Inference, SourceClaim, SupportReceipt,
    };
    use crate::osint::source_eval::{InformationCredibility as I, SourceReliability as S};

    fn reference(root: &str) -> EvidenceRef {
        EvidenceRef {
            source_id: root.into(),
            revision: "1".into(),
            artifact_hash: root.into(),
            acquisition_id: root.into(),
            origin_root: root.into(),
            url: format!("https://{root}.test/story"),
            published_at: None,
            retrieved_at: String::new(),
            location: EvidenceLocation::Range(crate::nlp::Span {
                start_byte: 0,
                end_byte: 4,
            }),
        }
    }
    fn claim(id: &str, roots: &[&str]) -> SourceClaim {
        SourceClaim {
            id: id.into(),
            subject_ref: "actor".into(),
            predicate: id.into(),
            object: "item".into(),
            text: format!("Report says {id}"),
            polarity: "positive".into(),
            modality: "asserted".into(),
            attributed_to: "report".into(),
            temporal_scope: "current".into(),
            evidence_refs: vec![reference("selected")],
            extraction_scope: "complete".into(),
            assessment: Assessment {
                fidelity: "faithful".into(),
                threshold_passed: true,
                ..Default::default()
            },
            support_receipts: roots
                .iter()
                .map(|r| SupportReceipt {
                    source: reference(r),
                    relation: "supports".into(),
                    threshold_passed: true,
                    lineage: "verified-decision".into(),
                })
                .collect(),
        }
    }
    #[test]
    fn rating_gold_cases_round_only_the_final_mean() {
        assert_eq!(initial_rating(S::B, I::CannotBeJudged), (Some(60), true));
        assert_eq!(initial_rating(S::F, I::CannotBeJudged), (None, true));
        let mut output = PipelineOutput {
            claims: vec![claim("one", &["a"]), claim("two", &["b", "c"])],
            ..Default::default()
        };
        output.inferences.push(Inference {
            id: "inference".into(),
            text: "Qualified conclusion".into(),
            entity_refs: vec!["actor".into()],
            premise_claim_ids: vec!["one".into(), "two".into()],
            evidence_refs: vec![reference("selected")],
            rationale: "reasoning".into(),
            limitations: vec![],
            assessment: Assessment {
                fidelity: "faithful".into(),
                threshold_passed: true,
                ..Default::default()
            },
            snapshot_key: String::new(),
        });
        let rating = EvidenceConfidence::estimate_for_source(&output, true, S::B, false);
        assert!((rating.item_scores[0].1 - 67.6666666667).abs() < 1e-8);
        assert!((rating.item_scores[1].1 - 81.75).abs() < 1e-8);
        assert_eq!(rating.score, Some(72));
        assert_eq!(rating.information_grade, 3);
        let mut disputed = claim("disputed", &["d"]);
        disputed.support_receipts[0].relation = "contradicts".into();
        output.claims.push(disputed.clone());
        assert_eq!(
            EvidenceConfidence::estimate_for_source(&output, true, S::B, false).score,
            Some(48)
        );
        assert_eq!(
            EvidenceConfidence::estimate_for_source(&output, true, S::B, false).information_grade,
            4
        );
        output.claims = vec![disputed];
        output.inferences.clear();
        let rating = EvidenceConfidence::estimate_for_source(&output, true, S::B, false);
        assert_eq!((rating.score, rating.information_grade), (Some(0), 5));
        output.claims.clear();
        assert_eq!(
            EvidenceConfidence::estimate_for_source(&output, true, S::B, false).score,
            None
        );
    }
    #[test]
    fn reports_require_embedded_verified_roots_and_legacy_is_unassessed() {
        let mut output = PipelineOutput::default();
        output.claims.push(claim("assertion", &[]));
        let report = EvidenceConfidence::estimate_for_source(&output, true, S::B, true);
        assert_eq!(
            (report.score, report.information_grade, report.provisional),
            (Some(40), 6, true)
        );
        assert_eq!(
            EvidenceConfidence::estimate_for_source(&output, true, S::B, false).score,
            None
        );
        let mut a = reference("a");
        let mut b = reference("b");
        b.artifact_hash = a.artifact_hash.clone();
        let mut c = reference("c");
        c.origin_root = b.origin_root.clone();
        assert_eq!(roots(&[a.clone(), b, c]), 1, "transitive copied roots");
        a.origin_root.clear();
        assert_eq!(roots(&[a]), 0);
    }
}
