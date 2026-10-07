//! Brain semantic exploration and tool-selection aids (spec §15).

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphEdgeKind {
    Evidenced,
    Inferred,
    ContradictionOrUpdate,
    /// Semantic suggestion only — never silently treated as a factual edge.
    SemanticSuggestion,
}

impl GraphEdgeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Evidenced => "evidenced",
            Self::Inferred => "inferred",
            Self::ContradictionOrUpdate => "contradiction_or_update",
            Self::SemanticSuggestion => "semantic_suggestion",
        }
    }

    pub fn is_factual(self) -> bool {
        !matches!(self, Self::SemanticSuggestion)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RelatedEvidenceHit {
    pub memory_id: String,
    pub label: String,
    pub score: f32,
    pub edge_kind: GraphEdgeKind,
    pub why: String,
}

/// Bound related-evidence expansion: prefer factual edges; cap semantic suggestions.
pub fn related_evidence_view(
    factual: Vec<(String, String, f32)>,
    semantic: Vec<(String, String, f32)>,
    max_semantic: usize,
) -> Vec<RelatedEvidenceHit> {
    let mut out = Vec::new();
    for (id, label, score) in factual {
        out.push(RelatedEvidenceHit {
            memory_id: id,
            label,
            score,
            edge_kind: GraphEdgeKind::Evidenced,
            why: "sourced relationship".into(),
        });
    }
    for (id, label, score) in semantic.into_iter().take(max_semantic) {
        out.push(RelatedEvidenceHit {
            memory_id: id,
            label,
            score,
            edge_kind: GraphEdgeKind::SemanticSuggestion,
            why: "semantic similarity (not proof)".into(),
        });
    }
    out
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolCandidate {
    pub tool_id: String,
    pub score: f32,
    pub reason: String,
}

/// Rank specialized tool candidates, then ensure full-catalog fallback so a poor
/// embedding match cannot hide an eligible specialized tool.
pub fn tool_candidates_with_fallback(
    ranked: Vec<ToolCandidate>,
    full_eligible_catalog: &[String],
    limit: usize,
) -> Vec<ToolCandidate> {
    let mut out = ranked;
    out.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out.truncate(limit.max(1));
    let have: std::collections::HashSet<String> = out.iter().map(|c| c.tool_id.clone()).collect();
    for tool_id in full_eligible_catalog {
        if have.contains(tool_id) {
            continue;
        }
        out.push(ToolCandidate {
            tool_id: tool_id.clone(),
            score: 0.0,
            reason: "full-catalog fallback".into(),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semantic_edges_are_not_factual() {
        assert!(!GraphEdgeKind::SemanticSuggestion.is_factual());
        assert!(GraphEdgeKind::Evidenced.is_factual());
        let view = related_evidence_view(
            vec![("m1".into(), "Ada → leads → Org".into(), 1.0)],
            vec![
                ("m2".into(), "similar org".into(), 0.8),
                ("m3".into(), "other".into(), 0.7),
            ],
            1,
        );
        assert_eq!(view.len(), 2);
        assert_eq!(view[1].edge_kind, GraphEdgeKind::SemanticSuggestion);
    }

    #[test]
    fn tool_fallback_preserves_missed_specialized_tools() {
        let ranked = vec![ToolCandidate {
            tool_id: "newsapi_search".into(),
            score: 0.9,
            reason: "semantic".into(),
        }];
        let catalog = vec![
            "newsapi_search".into(),
            "courtlistener_search".into(),
            "hunter_domain_search".into(),
        ];
        let out = tool_candidates_with_fallback(ranked, &catalog, 1);
        assert!(out.iter().any(|c| c.tool_id == "courtlistener_search"));
        assert!(out.iter().any(|c| c.reason == "full-catalog fallback"));
    }
}
