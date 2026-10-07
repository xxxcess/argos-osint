//! Curation, identity resolution interfaces, and claim-specific assessments.

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::osint::results::extract_search_results;
use crate::osint::ToolResult;

/// Stance of an individual evidence passage or assessed claim.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClaimStance {
    Supported,
    Disputed,
    Mention,
    Insufficient,
}

impl ClaimStance {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Supported => "supported",
            Self::Disputed => "disputed",
            Self::Mention => "mention",
            Self::Insufficient => "insufficient",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "supported" | "support" => Self::Supported,
            "disputed" | "contradict" | "refuted" => Self::Disputed,
            "mention" => Self::Mention,
            _ => Self::Insufficient,
        }
    }
}

/// A discrete evidence passage extracted from a tool call or document.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct EvidencePassage {
    pub id: String,
    pub investigation_id: String,
    pub task_id: String,
    pub call_id: String,
    pub source_url: String,
    pub source_domain: String,
    pub passage_text: String,
    pub observed_at: String,
    pub published_at: String,
    pub stance: ClaimStance,
    pub relevance_score: f64,
    pub created_at: String,
}

/// An individual assessment connecting one claim to one cited evidence passage.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ClaimAssessment {
    pub id: String,
    pub investigation_id: String,
    pub claim_id: String,
    pub evidence_id: String,
    pub stance: ClaimStance,
    pub rationale: String,
    pub created_at: String,
}

/// Status of an entity identity binding.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntityBindingStatus {
    Confirmed,
    Candidate,
    Rejected,
    Ambiguous,
}

/// An entity binding identified during investigation.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct EntityBinding {
    pub entity_kind: String,
    pub value: String,
    pub status: EntityBindingStatus,
    pub provenance: String,
    pub confidence: f64,
}

/// Overall outcome of assessing a claim across relevant passages.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ClaimAssessmentOutcome {
    pub claim_id: String,
    pub stance: ClaimStance,
    pub rationale: String,
    pub cited_passage_ids: Vec<String>,
}

/// Curates source-linked evidence passages from a tool result.
pub fn curate_passages_from_result(
    investigation_id: &str,
    task_id: &str,
    call_id: &str,
    result: &ToolResult,
) -> Vec<EvidencePassage> {
    let mut passages = Vec::new();
    let ts = Utc::now().to_rfc3339();

    // 1. Check for search results
    let search_results = extract_search_results(&result.observations);
    for (idx, item) in search_results.into_iter().enumerate() {
        if item.snippet.trim().is_empty() {
            continue;
        }

        let domain = url::Url::parse(&item.url)
            .ok()
            .and_then(|u| u.host_str().map(str::to_string))
            .unwrap_or_default();

        passages.push(EvidencePassage {
            id: format!(
                "pass:{}:{}:{}",
                call_id,
                idx,
                Utc::now().timestamp_subsec_millis()
            ),
            investigation_id: investigation_id.to_string(),
            task_id: task_id.to_string(),
            call_id: call_id.to_string(),
            source_url: item.url,
            source_domain: domain,
            passage_text: item.snippet,
            observed_at: result.retrieved_at.clone(),
            published_at: item.published_at,
            stance: ClaimStance::Mention,
            relevance_score: item.score.unwrap_or(1.0),
            created_at: ts.clone(),
        });
    }

    // 2. Check for markdown / body text
    if passages.is_empty() {
        let text = result
            .observations
            .get("markdown")
            .or_else(|| result.observations.get("text"))
            .or_else(|| result.observations.get("body"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");

        if !text.is_empty() {
            let domain = url::Url::parse(&result.source_url)
                .ok()
                .and_then(|u| u.host_str().map(str::to_string))
                .unwrap_or_default();

            passages.push(EvidencePassage {
                id: format!("pass:{}:body", call_id),
                investigation_id: investigation_id.to_string(),
                task_id: task_id.to_string(),
                call_id: call_id.to_string(),
                source_url: result.source_url.clone(),
                source_domain: domain,
                passage_text: text.chars().take(2000).collect(),
                observed_at: result.retrieved_at.clone(),
                published_at: String::new(),
                stance: ClaimStance::Mention,
                relevance_score: 1.0,
                created_at: ts,
            });
        }
    }

    passages
}

/// Assesses an individual claim against a list of evidence passages.
/// Ensures an unrelated supporting item never validates an unrelated claim.
pub fn assess_claim_against_passages(
    claim_id: &str,
    claim_text: &str,
    passages: &[EvidencePassage],
) -> ClaimAssessmentOutcome {
    let claim_lower = claim_text.to_ascii_lowercase();
    let keywords: Vec<&str> = claim_lower
        .split_whitespace()
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()))
        .filter(|w| w.len() >= 4)
        .collect();

    // Only passages relevant to this specific claim
    let relevant: Vec<&EvidencePassage> = passages
        .iter()
        .filter(|p| {
            if keywords.is_empty() {
                return false;
            }
            let passage_lower = p.passage_text.to_ascii_lowercase();
            let matches = keywords
                .iter()
                .filter(|&&kw| passage_lower.contains(kw))
                .count();
            matches >= 2.min(keywords.len())
        })
        .collect();

    if relevant.is_empty() {
        return ClaimAssessmentOutcome {
            claim_id: claim_id.to_string(),
            stance: ClaimStance::Insufficient,
            rationale: "No independent corroboration located for this specific claim; unresolved."
                .to_string(),
            cited_passage_ids: Vec::new(),
        };
    }

    let has_contradiction = relevant.iter().any(|p| p.stance == ClaimStance::Disputed);
    let has_support = relevant
        .iter()
        .any(|p| p.stance == ClaimStance::Supported || p.stance == ClaimStance::Mention);

    let (stance, rationale) = if has_contradiction {
        (
            ClaimStance::Disputed,
            "Contradicting evidence identified in cited passages.".to_string(),
        )
    } else if has_support {
        (
            ClaimStance::Supported,
            "Corroborating evidence present in cited source passages.".to_string(),
        )
    } else {
        (
            ClaimStance::Insufficient,
            "Cited sources mention subject without conclusive corroboration.".to_string(),
        )
    };

    ClaimAssessmentOutcome {
        claim_id: claim_id.to_string(),
        stance,
        rationale,
        cited_passage_ids: relevant.iter().map(|p| p.id.clone()).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unrelated_evidence_never_validates_claim() {
        let passages = vec![EvidencePassage {
            id: "pass1".into(),
            investigation_id: "inv1".into(),
            task_id: "task1".into(),
            call_id: "call1".into(),
            source_url: "https://example.com/alpha".into(),
            source_domain: "example.com".into(),
            passage_text: "Alpha Corp announced a major expansion into robotics hardware.".into(),
            observed_at: "2026-10-01".into(),
            published_at: "2026-10-01".into(),
            stance: ClaimStance::Supported,
            relevance_score: 1.0,
            created_at: "2026-10-01".into(),
        }];

        // Claim A matches passage1
        let outcome_a = assess_claim_against_passages(
            "c1",
            "Alpha Corp announced robotics expansion",
            &passages,
        );
        assert_eq!(outcome_a.stance, ClaimStance::Supported);
        assert_eq!(outcome_a.cited_passage_ids, vec!["pass1"]);

        // Claim B is unrelated to passage1
        let outcome_b =
            assess_claim_against_passages("c2", "Beta Ltd merged with Gamma Energy", &passages);
        assert_eq!(outcome_b.stance, ClaimStance::Insufficient);
        assert!(outcome_b.cited_passage_ids.is_empty());
    }
}
