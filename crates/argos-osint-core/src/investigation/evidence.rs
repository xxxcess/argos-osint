//! Curation, identity resolution interfaces, and claim-specific assessments.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::Path;

use crate::osint::results::extract_search_results;
use crate::osint::ToolResult;
use crate::telemetry::{EventKind, TelemetryEvent, Trigger};

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

/// Provenance for one accepted or cited evidence item: the tool and call that
/// produced it, plus the report revision it is credited to.
///
/// Evidence contribution is joined through these fields. A fetched result is not
/// accepted evidence, so only accepted/cited items are ever recorded.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EvidenceAttribution {
    /// Tool that produced the item (`firecrawl_search`, a fetch tool id, …).
    pub tool_id: String,
    /// Tool call that produced it.
    pub call_id: String,
    /// Final-report revision identity, e.g. `ijob-123:2`. Empty when unknown.
    pub report_revision: String,
    /// Report mode the revision belongs to (`verify`, `explain`, …).
    pub report_mode: String,
    /// Article or investigation the report belongs to.
    pub article_id: String,
    /// Durable report/job id the item is credited to.
    pub job_id: String,
    /// How many items the same citation credited. A multi-source citation
    /// credits several tools and must never be summed into one contribution.
    pub citations: usize,
}

/// Builds the telemetry row for one accepted/cited evidence item.
///
/// The event id is the item's own id, so replaying a report never double counts
/// an evidence contribution, and `canonical_ref` points at the owning report
/// revision so contribution can be joined without equating fetched results with
/// accepted evidence.
///
/// The builder carries no wall clock: it is a pure function of the item's
/// identity, so replaying the same report rebuilds the same event (and replaces
/// the same row) instead of adding a second one. Callers stamp the write time
/// with [`stamped`] before persisting.
pub fn evidence_item_event(
    item_id: &str,
    investigation_id: &str,
    source_domain: &str,
    stance: ClaimStance,
    attribution: &EvidenceAttribution,
) -> TelemetryEvent {
    let mut event = TelemetryEvent::new(EventKind::EvidenceItem)
        .trigger(Trigger::IntelBrief)
        .app("intel")
        .tool(&attribution.tool_id)
        .call(if attribution.call_id.is_empty() {
            item_id
        } else {
            attribution.call_id.as_str()
        })
        .article(&attribution.article_id)
        .mode(&attribution.report_mode)
        .provider(source_domain)
        .outcome(stance.as_str())
        .canonical(&attribution.report_revision)
        .job(&attribution.job_id)
        .count(1)
        .payload(json!({
            "item_id": item_id,
            "investigation_id": investigation_id,
            "tool_id": attribution.tool_id,
            "call_id": attribution.call_id,
            "report_revision": attribution.report_revision,
            "report_mode": attribution.report_mode,
            "source_domain": source_domain,
            "stance": stance.as_str(),
            // Multi-source citations stay nonadditive: several items share one
            // citation and each is credited once.
            "citations": attribution.citations,
        }))
        .with_id(format!("intel-evidence-{}", item_id));
    event.occurred_at = String::new();
    event
}

/// Stamps the write time onto an event whose builder left it blank.
///
/// A blank stamp must never reach the database: a row without `occurred_at`
/// falls outside every period filter, so replayed evidence would silently
/// disappear from the dashboard.
pub fn stamped(event: TelemetryEvent) -> TelemetryEvent {
    if event.occurred_at.trim().is_empty() {
        event.at(Utc::now().to_rfc3339())
    } else {
        event
    }
}

/// Best-effort write for one accepted/cited evidence item. Never fails the
/// caller: telemetry must not be able to break a report.
pub fn record_accepted_evidence(
    db_path: &Path,
    item_id: &str,
    investigation_id: &str,
    source_domain: &str,
    stance: ClaimStance,
    attribution: &EvidenceAttribution,
) {
    let built = evidence_item_event(
        item_id,
        investigation_id,
        source_domain,
        stance,
        attribution,
    );
    let event = stamped(built);
    let _ = crate::telemetry::record(db_path, &event);
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

    #[test]
    fn accepted_evidence_events_are_stable_and_keep_provenance() {
        let attribution = EvidenceAttribution {
            tool_id: "firecrawl_search".into(),
            call_id: "call-42".into(),
            report_revision: "ijob-9:2".into(),
            report_mode: "verify".into(),
            article_id: "art-1".into(),
            job_id: "ijob-9".into(),
            citations: 3,
        };
        let first = evidence_item_event(
            "iev-1",
            "iinv-1",
            "example.com",
            ClaimStance::Supported,
            &attribution,
        );
        let replay = evidence_item_event(
            "iev-1",
            "iinv-1",
            "example.com",
            ClaimStance::Supported,
            &attribution,
        );
        // Replaying the same item replaces its row instead of adding one.
        assert_eq!(first.id, "intel-evidence-iev-1");
        assert_eq!(first, replay);
        assert_eq!(first.event_type, "evidence_item");
        assert_eq!(first.tool_id, "firecrawl_search");
        assert_eq!(first.call_id, "call-42");
        assert_eq!(first.canonical_ref, "ijob-9:2");
        assert_eq!(first.mode, "verify");
        assert_eq!(first.article_id, "art-1");
        // A different item is a different row.
        let other = first.clone().with_id("intel-evidence-iev-2");
        assert_ne!(other.id, first.id);
    }

    #[test]
    fn multi_source_citations_stay_nonadditive() {
        let shared = EvidenceAttribution {
            tool_id: "firecrawl_search".into(),
            call_id: "call-7".into(),
            report_revision: "ijob-3:1".into(),
            report_mode: "explain".into(),
            article_id: "art-2".into(),
            job_id: "ijob-3".into(),
            citations: 2,
        };
        let a = evidence_item_event(
            "iev-a",
            "iinv-2",
            "a.example",
            ClaimStance::Supported,
            &shared,
        );
        let b = evidence_item_event(
            "iev-b",
            "iinv-2",
            "b.example",
            ClaimStance::Supported,
            &shared,
        );
        // Each item is credited exactly once; neither row claims a total.
        assert_eq!(a.count, 1);
        assert_eq!(b.count, 1);
        assert_eq!(a.canonical_ref, b.canonical_ref);
        let payload_a: serde_json::Value = serde_json::from_str(&a.payload_json).unwrap();
        assert_eq!(payload_a["citations"], 2);
        assert_eq!(payload_a["item_id"], "iev-a");
    }
}
