//! Classifier picks the default Intel Recon report mode for a briefing article
//! or a chat Recon user prompt.

use anyhow::Result;
use serde_json::{json, Value};

use crate::provider::{self, ChatMessage};
use crate::secrets::ProviderSecret;
use crate::store::{AtlasArticleClaim, AtlasArticleRow};

use super::modes::ReportMode;

/// Inputs the classifier uses to choose a recon mode for an Atlas article.
#[derive(Clone, Debug)]
pub struct ModeClassifyInput {
    pub category: String,
    pub title: String,
    pub description: String,
    pub claim_count: usize,
    pub fact_count: usize,
    pub inference_count: usize,
    pub mean_confidence: f64,
    pub high_confidence: f64,
    pub admiralty_sample: String,
}

impl ModeClassifyInput {
    pub fn from_article(article: &AtlasArticleRow, claims: &[AtlasArticleClaim]) -> Self {
        let fact_count = claims.iter().filter(|c| c.classification == "fact").count();
        let inference_count = claims
            .iter()
            .filter(|c| c.classification == "inference")
            .count();
        let (high_confidence, mean_confidence) = if claims.is_empty() {
            (0.0, 0.0)
        } else {
            let high = claims.iter().map(|c| c.confidence).fold(0.0_f64, f64::max);
            let mean = claims.iter().map(|c| c.confidence).sum::<f64>() / claims.len() as f64;
            (high, mean)
        };
        let admiralty_sample = claims
            .iter()
            .find(|c| !c.admiralty.trim().is_empty())
            .map(|c| c.admiralty.clone())
            .unwrap_or_default();
        Self {
            category: article.category.clone(),
            title: article.title.clone(),
            description: article.description.clone(),
            claim_count: claims.len(),
            fact_count,
            inference_count,
            mean_confidence,
            high_confidence,
            admiralty_sample,
        }
    }
}

/// Inputs the classifier uses to choose a recon mode for a chat Recon prompt.
#[derive(Clone, Debug, Default)]
pub struct PromptModeClassifyInput {
    pub question: String,
    pub prior: String,
    pub known_facts: Vec<String>,
}

/// Classifiable defaults: Verify, Explain, Assess Outlook (never Full Assessment).
pub fn classifiable_modes() -> [ReportMode; 3] {
    [
        ReportMode::Verify,
        ReportMode::Explain,
        ReportMode::AssessOutlook,
    ]
}

/// Fallback when the classifier is unavailable or fails.
pub fn default_recon_mode() -> ReportMode {
    ReportMode::Verify
}

/// Parse a classifiable mode id. Full Assessment and unknowns fall back to Verify.
pub fn parse_mode_choice(raw: &str) -> ReportMode {
    match ReportMode::parse(raw.trim()) {
        Some(mode) if classifiable_modes().contains(&mode) => mode,
        _ => default_recon_mode(),
    }
}

/// Cheap keyword fallback when the classifier role is missing or fails.
pub fn heuristic_prompt_mode(question: &str) -> ReportMode {
    let q = question.to_ascii_lowercase();
    let outlook = [
        "what happens next",
        "what might happen",
        "outlook",
        "scenario",
        "forecast",
        "escalate",
        "escalation",
        "risk of",
        "will iran",
        "early warning",
    ];
    let explain = [
        "what is going on",
        "what's going on",
        "whats going on",
        "what happened",
        "what's happening",
        "whats happening",
        "explain",
        "context",
        "background",
        "how does",
        "situation",
        "developments",
        "why is",
        "who is involved",
    ];
    let verify = [
        "is it true",
        "is this true",
        "did they",
        "did he",
        "did she",
        "verify",
        "confirm",
        "allegation",
        "claim that",
        "false flag",
        "debunk",
        "corroborat",
    ];
    if outlook.iter().any(|needle| q.contains(needle)) {
        return ReportMode::AssessOutlook;
    }
    if verify.iter().any(|needle| q.contains(needle)) {
        return ReportMode::Verify;
    }
    if explain.iter().any(|needle| q.contains(needle)) {
        return ReportMode::Explain;
    }
    default_recon_mode()
}

fn mode_catalog_text() -> &'static str {
    "Modes and best use (pick exactly one; Full Assessment is not a classifier option):\n\
- verify: Breaking news, allegations, conflicting reporting, or consequential claims. \
Question: What can we establish, and what remains uncertain? \
Investigate assertions, primary evidence, corroboration, corrections, and contradictions. \
Tradeoff: defensible foundation, little on wider implications.\n\
- explain: Geopolitical developments, corporate networks, conflicts, policy changes, interconnected incidents. \
Question: How does this development fit into a larger situation? \
Resolve actors, location/timing, preceding events, evidence-backed relationships. \
Example: sanctions on a shipping company → ownership, vessels, trading relationships, prior restrictions, routes. \
Tradeoff: can expand indefinitely; stay bounded by stated questions.\n\
- assess_outlook: Early warning, escalation, operational disruption, strategic planning. \
Question: What might happen next, and what evidence would change that? \
Form competing scenarios, seek supporting and disconfirming evidence, conditional outlook with indicators. \
Example military deployment: routine exercise vs coercive signaling vs sustained operations. \
Tradeoff: most vulnerable to speculation; missing info lowers confidence."
}

/// Build the Decisions API state/questions for article mode selection.
pub fn mode_decisions_request(input: &ModeClassifyInput) -> (Value, Value) {
    let state = json!({
        "category": input.category,
        "title": input.title,
        "description": input.description,
        "claim_count": input.claim_count,
        "fact_count": input.fact_count,
        "inference_count": input.inference_count,
        "mean_confidence": input.mean_confidence,
        "high_confidence": input.high_confidence,
        "admiralty": input.admiralty_sample,
        "guidance": mode_catalog_text(),
    });
    let questions = json!([{
        "id": "mode",
        "type": "choice",
        "question": "Which Intel Recon report mode best fits this article?",
        "options": {
            "verify": "Verify — evidence-led claim investigation",
            "explain": "Explain — actors, timeline, relationships",
            "assess_outlook": "Assess Outlook — scenarios and indicators",
        }
    }]);
    (state, questions)
}

/// Build the Decisions API state/questions for chat Recon prompt mode selection.
pub fn prompt_mode_decisions_request(input: &PromptModeClassifyInput) -> (Value, Value) {
    let facts: Vec<String> = input
        .known_facts
        .iter()
        .take(6)
        .map(|fact| fact.chars().take(160).collect())
        .collect();
    let state = json!({
        "question": input.question,
        "prior_synthesis": input.prior.chars().take(400).collect::<String>(),
        "known_facts": facts,
        "guidance": mode_catalog_text(),
    });
    let questions = json!([{
        "id": "mode",
        "type": "choice",
        "question": "Which Recon report mode best fits this user investigation prompt?",
        "options": {
            "verify": "Verify — evidence-led claim investigation",
            "explain": "Explain — actors, timeline, relationships",
            "assess_outlook": "Assess Outlook — scenarios and indicators",
        }
    }]);
    (state, questions)
}

/// Ask the classifier which recon mode to default to for an article. Falls back to Verify.
pub async fn classify_recon_mode(
    classifier: Option<&ProviderSecret>,
    input: &ModeClassifyInput,
) -> ReportMode {
    let Some(classifier) = classifier else {
        return default_recon_mode();
    };
    let result = if provider::is_decisions_model(&classifier.model) {
        classify_mode_decisions(classifier, input).await
    } else {
        classify_mode_chat(classifier, input).await
    };
    result.unwrap_or_else(|_| default_recon_mode())
}

/// Ask the classifier which recon mode fits a chat Recon user prompt.
/// Falls back to [`heuristic_prompt_mode`] when the classifier is missing or fails.
pub async fn classify_prompt_mode(
    classifier: Option<&ProviderSecret>,
    input: &PromptModeClassifyInput,
) -> ReportMode {
    let fallback = heuristic_prompt_mode(&input.question);
    let Some(classifier) = classifier else {
        return fallback;
    };
    let result = if provider::is_decisions_model(&classifier.model) {
        classify_prompt_mode_decisions(classifier, input).await
    } else {
        classify_prompt_mode_chat(classifier, input).await
    };
    result.unwrap_or(fallback)
}

async fn classify_mode_decisions(
    classifier: &ProviderSecret,
    input: &ModeClassifyInput,
) -> Result<ReportMode> {
    let (state, questions) = mode_decisions_request(input);
    let response = provider::decide(classifier, &state, &questions).await?;
    let choice = response
        .answers
        .get("mode")
        .and_then(|answer| answer.choice.as_deref())
        .unwrap_or("");
    Ok(parse_mode_choice(choice))
}

async fn classify_prompt_mode_decisions(
    classifier: &ProviderSecret,
    input: &PromptModeClassifyInput,
) -> Result<ReportMode> {
    let (state, questions) = prompt_mode_decisions_request(input);
    let response = provider::decide(classifier, &state, &questions).await?;
    let choice = response
        .answers
        .get("mode")
        .and_then(|answer| answer.choice.as_deref())
        .unwrap_or("");
    Ok(parse_mode_choice(choice))
}

async fn classify_mode_chat(
    classifier: &ProviderSecret,
    input: &ModeClassifyInput,
) -> Result<ReportMode> {
    let system = format!(
        "Choose the single best Intel Recon report mode for this article. \
Do not choose full_assessment. \
Return JSON only: {{\"mode\":\"verify\"|\"explain\"|\"assess_outlook\"}}.\n\n{}",
        mode_catalog_text()
    );
    let user = json!({
        "category": input.category,
        "title": input.title,
        "description": input.description,
        "claim_count": input.claim_count,
        "fact_count": input.fact_count,
        "inference_count": input.inference_count,
        "mean_confidence": input.mean_confidence,
        "high_confidence": input.high_confidence,
        "admiralty": input.admiralty_sample,
    })
    .to_string();
    let messages = [
        ChatMessage {
            role: "system".into(),
            content: system,
            tool_call_id: None,
            tool_calls: Vec::new(),
        },
        ChatMessage {
            role: "user".into(),
            content: user,
            tool_call_id: None,
            tool_calls: Vec::new(),
        },
    ];
    let done = provider::complete(classifier, &messages, &[], |_| {}).await?;
    Ok(parse_mode_from_chat(&done.content))
}

async fn classify_prompt_mode_chat(
    classifier: &ProviderSecret,
    input: &PromptModeClassifyInput,
) -> Result<ReportMode> {
    let system = format!(
        "Choose the single best Recon report mode for this user investigation prompt. \
Do not choose full_assessment. \
Return JSON only: {{\"mode\":\"verify\"|\"explain\"|\"assess_outlook\"}}.\n\n{}",
        mode_catalog_text()
    );
    let facts: Vec<String> = input
        .known_facts
        .iter()
        .take(6)
        .map(|fact| fact.chars().take(160).collect())
        .collect();
    let user = json!({
        "question": input.question,
        "prior_synthesis": input.prior.chars().take(400).collect::<String>(),
        "known_facts": facts,
    })
    .to_string();
    let messages = [
        ChatMessage {
            role: "system".into(),
            content: system,
            tool_call_id: None,
            tool_calls: Vec::new(),
        },
        ChatMessage {
            role: "user".into(),
            content: user,
            tool_call_id: None,
            tool_calls: Vec::new(),
        },
    ];
    let done = provider::complete(classifier, &messages, &[], |_| {}).await?;
    Ok(parse_mode_from_chat(&done.content))
}

/// Extract mode from chat JSON or bare token.
pub fn parse_mode_from_chat(content: &str) -> ReportMode {
    let trimmed = content.trim();
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        if let Some(mode) = value.get("mode").and_then(Value::as_str) {
            return parse_mode_choice(mode);
        }
    }
    // Tolerate fenced JSON.
    if let Some(start) = trimmed.find('{') {
        if let Some(end) = trimmed.rfind('}') {
            if let Ok(value) = serde_json::from_str::<Value>(&trimmed[start..=end]) {
                if let Some(mode) = value.get("mode").and_then(Value::as_str) {
                    return parse_mode_choice(mode);
                }
            }
        }
    }
    for token in ["assess_outlook", "verify", "explain"] {
        if trimmed.to_ascii_lowercase().contains(token) {
            return parse_mode_choice(token);
        }
    }
    default_recon_mode()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_mode_choice_accepts_classifiable_only() {
        assert_eq!(parse_mode_choice("verify"), ReportMode::Verify);
        assert_eq!(parse_mode_choice("outlook"), ReportMode::AssessOutlook);
        assert_eq!(parse_mode_choice("full"), ReportMode::Verify);
        assert_eq!(parse_mode_choice("full_assessment"), ReportMode::Verify);
        assert_eq!(parse_mode_choice("nope"), ReportMode::Verify);
        assert!(!classifiable_modes().contains(&ReportMode::FullAssessment));
    }

    #[test]
    fn parse_mode_from_chat_reads_json() {
        assert_eq!(
            parse_mode_from_chat(r#"{"mode":"explain"}"#),
            ReportMode::Explain
        );
        assert_eq!(
            parse_mode_from_chat("```json\n{\"mode\":\"assess_outlook\"}\n```"),
            ReportMode::AssessOutlook
        );
        assert_eq!(
            parse_mode_from_chat(r#"{"mode":"full_assessment"}"#),
            ReportMode::Verify
        );
    }

    #[test]
    fn heuristic_prompt_mode_matches_common_prompts() {
        assert_eq!(
            heuristic_prompt_mode("what is going on with iran?"),
            ReportMode::Explain
        );
        assert_eq!(
            heuristic_prompt_mode("is it true that Iran transferred uranium?"),
            ReportMode::Verify
        );
        assert_eq!(
            heuristic_prompt_mode("what might happen next in the Strait?"),
            ReportMode::AssessOutlook
        );
        assert_eq!(
            heuristic_prompt_mode("who runs example.org?"),
            ReportMode::Verify
        );
    }

    #[test]
    fn from_article_counts_claims() {
        let article = AtlasArticleRow {
            run_id: "r".into(),
            id: "a".into(),
            title: "Sanctions hit fleet".into(),
            description: "Owners face new restrictions.".into(),
            url: "https://ex.com".into(),
            country: "US".into(),
            source_name: "Ex".into(),
            source_domain: "ex.com".into(),
            published_at: "2026-10-01".into(),
            provider: "news".into(),
            temperature: 0.5,
            category: "geopolitical".into(),
            seen_at: "".into(),
            author: "".into(),
            image_url: "".into(),
        };
        let claims = vec![
            AtlasArticleClaim {
                fingerprint: "f1".into(),
                entity: "acme".into(),
                predicate: "sanctioned".into(),
                object: "fleet".into(),
                topic: "geopolitical".into(),
                classification: "fact".into(),
                confidence: 0.8,
                claim: "Acme fleet sanctioned.".into(),
                source_url: "".into(),
                published_at: "".into(),
                article_id: "a".into(),
                reliability: "B".into(),
                info_credibility: 2,
                admiralty: "B2".into(),
                rsp_status: "gr".into(),
            },
            AtlasArticleClaim {
                fingerprint: "f2".into(),
                entity: "acme".into(),
                predicate: "may_reroute".into(),
                object: "trade".into(),
                topic: "geopolitical".into(),
                classification: "inference".into(),
                confidence: 0.4,
                claim: "Acme may reroute trade.".into(),
                source_url: "".into(),
                published_at: "".into(),
                article_id: "a".into(),
                reliability: "B".into(),
                info_credibility: 4,
                admiralty: "B4".into(),
                rsp_status: "gr".into(),
            },
        ];
        let input = ModeClassifyInput::from_article(&article, &claims);
        assert_eq!(input.fact_count, 1);
        assert_eq!(input.inference_count, 1);
        assert!((input.mean_confidence - 0.6).abs() < 1e-9);
        assert_eq!(input.admiralty_sample, "B2");
    }
}
