//! Dedicated Summarization role: evidence compression and source-grounded views.
//!
//! Analytical conclusions stay with Synthesis. Summarization must not invent
//! connections, promote inferences to facts, call tools, or decide that a
//! directive is satisfied.

use serde::{Deserialize, Serialize};

/// Nine production modes. Each maps to a real call site.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SummarizationMode {
    PageEvidence,
    GraphExplanation,
    FollowUpContext,
    ToolObservation,
    InvestigationTitle,
    ReportContext,
    SectionDigest,
    AtlasBrief,
    ArticleDescription,
}

impl SummarizationMode {
    pub const ALL: [SummarizationMode; 9] = [
        Self::PageEvidence,
        Self::GraphExplanation,
        Self::FollowUpContext,
        Self::ToolObservation,
        Self::InvestigationTitle,
        Self::ReportContext,
        Self::SectionDigest,
        Self::AtlasBrief,
        Self::ArticleDescription,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::PageEvidence => "page_evidence",
            Self::GraphExplanation => "graph_explanation",
            Self::FollowUpContext => "follow_up_context",
            Self::ToolObservation => "tool_observation",
            Self::InvestigationTitle => "investigation_title",
            Self::ReportContext => "report_context",
            Self::SectionDigest => "section_digest",
            Self::AtlasBrief => "atlas_brief",
            Self::ArticleDescription => "article_description",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "page_evidence" | "page-evidence" => Some(Self::PageEvidence),
            "graph_explanation" | "graph-explanation" => Some(Self::GraphExplanation),
            "follow_up_context" | "follow-up-context" => Some(Self::FollowUpContext),
            "tool_observation" | "tool-observation" => Some(Self::ToolObservation),
            "investigation_title" | "investigation-title" => Some(Self::InvestigationTitle),
            "report_context" | "report-context" => Some(Self::ReportContext),
            "section_digest" | "section-digest" => Some(Self::SectionDigest),
            "atlas_brief" | "atlas-brief" => Some(Self::AtlasBrief),
            "article_description" | "article-description" => Some(Self::ArticleDescription),
            _ => None,
        }
    }

    /// Prompt version string included in cache keys.
    pub fn prompt_version(self) -> &'static str {
        "summarization.v1"
    }
}

/// One source record handed to Summarization.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SummarySource {
    pub id: String,
    pub revision: String,
    pub hash: String,
    pub text: String,
    #[serde(default)]
    pub meta: serde_json::Value,
}

/// Request envelope for the shared summarization service.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SummaryRequest {
    pub mode: SummarizationMode,
    pub sources: Vec<SummarySource>,
    pub focus: String,
    pub budget_chars: usize,
    pub required_fields: Vec<String>,
    pub model: String,
    pub provider: String,
    pub prompt_version: String,
}

impl SummaryRequest {
    pub fn cache_key(&self) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(self.mode.as_str().as_bytes());
        hasher.update(self.focus.as_bytes());
        hasher.update(self.budget_chars.to_string().as_bytes());
        hasher.update(self.model.as_bytes());
        hasher.update(self.prompt_version.as_bytes());
        for source in &self.sources {
            hasher.update(source.id.as_bytes());
            hasher.update(source.revision.as_bytes());
            hasher.update(source.hash.as_bytes());
        }
        format!("{:x}", hasher.finalize())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SummaryResult {
    pub content: String,
    pub source_refs: Vec<String>,
    pub source_hash: String,
    pub model: String,
    pub prompt_version: String,
    pub coverage: CoverageMeta,
    pub fallback: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CoverageMeta {
    pub partial: bool,
    pub omitted: Vec<String>,
    pub notes: String,
}

/// System prompt for a mode. Callers pass sources separately as data.
pub fn system_prompt(mode: SummarizationMode) -> &'static str {
    match mode {
        SummarizationMode::PageEvidence => {
            "Compress the supplied page evidence for investigation use. Preserve attribution, uncertainty, and exact identifiers. Do not invent facts, call tools, or decide directive completion. Write concise Markdown."
        }
        SummarizationMode::GraphExplanation => {
            "Explain only the selected claim path and its supplied support. Preserve timestamps, attribution, and fact-versus-inference labels. Do not invent sources or outcomes."
        }
        SummarizationMode::FollowUpContext => {
            "Summarize the prior investigation answer for the next turn. Copy supplied directive states exactly; do not reassess completion. Keep entities, identifiers, uncertainty, and unresolved questions."
        }
        SummarizationMode::ToolObservation => {
            "Digest the tool observation for the current question. Preserve tool name, call id, status, counts, errors, and selected record identifiers as structured facts. Do not invent bindings."
        }
        SummarizationMode::InvestigationTitle => {
            "Propose a short recognizable investigation title from the question. No quotes. No trailing punctuation. At most 8 words."
        }
        SummarizationMode::ReportContext => {
            "Compress source material for one report section. Preserve evidence references and disagreements. Do not write final analytical judgments or BLUF."
        }
        SummarizationMode::SectionDigest => {
            "Write a short digest of a completed analytical section for later BLUF use. Do not change the section's conclusions."
        }
        SummarizationMode::AtlasBrief => {
            "Render a concise natural-language brief from the supplied accepted claims. Preserve fact/inference distinctions. Do not create new claims."
        }
        SummarizationMode::ArticleDescription => {
            "Write a concise article description from the supplied title and body excerpt. Do not invent events absent from the source."
        }
    }
}

/// Reject empty or malformed model output for a mode.
pub fn validate_result(mode: SummarizationMode, content: &str, known_ids: &[String]) -> Result<(), String> {
    let text = content.trim();
    if text.is_empty() {
        return Err("empty summarization output".into());
    }
    match mode {
        SummarizationMode::InvestigationTitle => {
            if text.chars().count() > 80 {
                return Err("title exceeds length budget".into());
            }
        }
        SummarizationMode::GraphExplanation | SummarizationMode::PageEvidence => {
            if text.chars().count() < 20 {
                return Err("summary too short to be useful".into());
            }
        }
        _ => {}
    }
    // Unknown evidence IDs in bracket citations are rejected when known_ids is non-empty.
    if !known_ids.is_empty() {
        for token in text.split_whitespace() {
            let trimmed = token.trim_matches(|c: char| matches!(c, '[' | ']' | ',' | ';' | '.'));
            if let Some(id) = trimmed.strip_prefix("call-").or_else(|| {
                if trimmed.starts_with("brain:") {
                    Some(trimmed)
                } else {
                    None
                }
            }) {
                let full = if trimmed.starts_with("brain:") {
                    trimmed.to_string()
                } else {
                    format!("call-{id}")
                };
                if !known_ids.iter().any(|k| k == &full || k == trimmed) {
                    return Err(format!("unknown evidence id {trimmed}"));
                }
            }
        }
    }
    Ok(())
}

/// Deterministic title fallback when the model is unavailable.
pub fn fallback_title(question: &str) -> String {
    let flat: String = question
        .split_whitespace()
        .take(8)
        .collect::<Vec<_>>()
        .join(" ");
    let mut title = flat.chars().take(72).collect::<String>();
    if title.is_empty() {
        title = "Investigation".into();
    }
    title
}

/// Deterministic page excerpt used when Summarization cannot run.
pub fn fallback_excerpt(text: &str, budget: usize) -> String {
    let budget = budget.max(1);
    let trimmed = text.trim();
    if trimmed.chars().count() <= budget {
        return trimmed.to_string();
    }
    let mut out: String = trimmed.chars().take(budget.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_nine_modes_round_trip() {
        assert_eq!(SummarizationMode::ALL.len(), 9);
        for mode in SummarizationMode::ALL {
            assert_eq!(SummarizationMode::parse(mode.as_str()), Some(mode));
            assert!(!system_prompt(mode).is_empty());
        }
    }

    #[test]
    fn validate_rejects_empty_and_unknown_ids() {
        assert!(validate_result(SummarizationMode::AtlasBrief, "  ", &[]).is_err());
        assert!(validate_result(
            SummarizationMode::PageEvidence,
            "See [call-missing] for details that are long enough.",
            &["call-1".into()]
        )
        .is_err());
        assert!(validate_result(
            SummarizationMode::PageEvidence,
            "See [call-1] for details that are long enough.",
            &["call-1".into()]
        )
        .is_ok());
    }

    #[test]
    fn cache_key_changes_with_revision_and_focus() {
        let mut req = SummaryRequest {
            mode: SummarizationMode::GraphExplanation,
            sources: vec![SummarySource {
                id: "m1".into(),
                revision: "1".into(),
                hash: "h1".into(),
                text: "x".into(),
                meta: serde_json::json!({}),
            }],
            focus: "path-a".into(),
            budget_chars: 800,
            required_fields: vec![],
            model: "m".into(),
            provider: "p".into(),
            prompt_version: "summarization.v1".into(),
        };
        let a = req.cache_key();
        req.focus = "path-b".into();
        assert_ne!(a, req.cache_key());
        req.focus = "path-a".into();
        req.sources[0].revision = "2".into();
        assert_ne!(a, req.cache_key());
    }

    #[test]
    fn fallbacks_are_non_empty() {
        assert_eq!(fallback_title("who runs Example Org today?"), "who runs Example Org today?");
        assert!(fallback_excerpt("abcdefghij", 6).ends_with('…'));
    }
}
