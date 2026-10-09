//! Bucket Atlas article insights for the Intel briefing extracted pane.

use crate::store::AtlasArticleClaim;

/// One line in the extracted pane.
#[derive(Clone, Debug, PartialEq)]
pub struct ExtractedLine {
    pub text: String,
    pub confidence: Option<f64>,
}

/// Grouped insight elements for the left briefing column.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExtractedBuckets {
    pub facts: Vec<ExtractedLine>,
    pub inferences: Vec<ExtractedLine>,
    pub context: Vec<ExtractedLine>,
    pub actors: Vec<String>,
    pub links: Vec<String>,
}

const MAX_FACTS: usize = 8;
const MAX_INFERENCES: usize = 8;
const MAX_CONTEXT: usize = 6;
const MAX_ACTORS: usize = 8;
const MAX_LINKS: usize = 8;

/// Deterministic meaningful explanation for a relation link between two claims.
pub fn explain_relation_link(rel: &str, left: &str, right: &str) -> String {
    let clean = rel.trim().to_ascii_lowercase();
    match clean.as_str() {
        "context_for" | "context" => format!("{left} provides context for: {right}"),
        "supports" | "corroborates" => format!("{left} corroborates: {right}"),
        "contradicts" | "conflicts" => format!("{left} contradicts: {right}"),
        "revision_of" | "revises" => format!("{left} updates: {right}"),
        "causes" | "leads_to" => format!("{left} leads to: {right}"),
        _ => {
            let readable = rel.replace('_', " ");
            format!("{left} ({readable}) {right}")
        }
    }
}

/// Partition claims and relations into briefing buckets.
pub fn bucket_extracted(
    claims: &[AtlasArticleClaim],
    relations: &[(String, String, String)],
) -> ExtractedBuckets {
    bucket_extracted_with_explanations(claims, relations, &std::collections::HashMap::new())
}

/// Partition claims and relations into briefing buckets with optional model-rewritten link explanations.
pub fn bucket_extracted_with_explanations(
    claims: &[AtlasArticleClaim],
    relations: &[(String, String, String)],
    explanations: &std::collections::HashMap<(String, String), String>,
) -> ExtractedBuckets {
    let context_fps: std::collections::HashSet<&str> = relations
        .iter()
        .filter(|(_, _, rel)| rel == "context_for")
        .map(|(left, _, _)| left.as_str())
        .collect();

    let by_fp: std::collections::HashMap<&str, &AtlasArticleClaim> =
        claims.iter().map(|c| (c.fingerprint.as_str(), c)).collect();

    let mut facts = Vec::new();
    let mut inferences = Vec::new();
    let mut context = Vec::new();

    for claim in claims {
        let line = ExtractedLine {
            text: claim_label(claim),
            confidence: Some(claim.confidence),
        };
        if context_fps.contains(claim.fingerprint.as_str()) {
            if context.len() < MAX_CONTEXT {
                context.push(line);
            }
            continue;
        }
        if claim.classification == "fact" {
            if facts.len() < MAX_FACTS {
                facts.push(line);
            }
        } else if inferences.len() < MAX_INFERENCES {
            inferences.push(line);
        }
    }

    let mut actors = Vec::new();
    for claim in claims {
        let entity = claim.entity.trim();
        if entity.is_empty()
            || !super::is_meaningful_actor(entity)
            || actors.iter().any(|item| item == entity)
        {
            continue;
        }
        actors.push(entity.to_string());
        if actors.len() >= MAX_ACTORS {
            break;
        }
    }

    let mut links = Vec::new();
    for (left, right, rel) in relations {
        if links.len() >= MAX_LINKS {
            break;
        }
        if let Some(explained) = explanations.get(&(left.clone(), right.clone())) {
            links.push(explained.clone());
            continue;
        }
        let left_label = by_fp
            .get(left.as_str())
            .map(|c| short_claim(c))
            .unwrap_or_else(|| left.chars().take(24).collect());
        let right_label = by_fp
            .get(right.as_str())
            .map(|c| short_claim(c))
            .unwrap_or_else(|| right.chars().take(24).collect());
        links.push(explain_relation_link(rel, &left_label, &right_label));
    }

    ExtractedBuckets {
        facts,
        inferences,
        context,
        actors,
        links,
    }
}

fn claim_label(claim: &AtlasArticleClaim) -> String {
    let text = claim.claim.trim();
    if !text.is_empty() {
        return text.to_string();
    }
    format!("{} → {} → {}", claim.entity, claim.predicate, claim.object)
}

fn short_claim(claim: &AtlasArticleClaim) -> String {
    let base = if claim.entity.trim().is_empty() {
        claim.claim.clone()
    } else {
        format!("{}·{}", claim.entity, claim.predicate)
    };
    base.chars().take(28).collect()
}

use crate::provider::{self, ChatMessage};
use crate::secrets::ProviderSecret;

/// Rewrites briefing links to explain them meaningfully using the summarization / synthesis model role.
/// Returns a map of (left_fingerprint, right_fingerprint) -> meaningful explanation.
pub async fn explain_intel_links_with_model(
    secret: Option<&ProviderSecret>,
    article_title: &str,
    claims: &[AtlasArticleClaim],
    relations: &[(String, String, String)],
) -> std::collections::HashMap<(String, String), String> {
    let mut result = std::collections::HashMap::new();
    if relations.is_empty() {
        return result;
    }

    let by_fp: std::collections::HashMap<&str, &AtlasArticleClaim> =
        claims.iter().map(|c| (c.fingerprint.as_str(), c)).collect();

    let links_to_explain: Vec<((String, String), String, String, String)> = relations
        .iter()
        .take(MAX_LINKS)
        .map(|(left, right, rel)| {
            let left_label = by_fp
                .get(left.as_str())
                .map(|c| short_claim(c))
                .unwrap_or_else(|| left.chars().take(24).collect());
            let right_label = by_fp
                .get(right.as_str())
                .map(|c| short_claim(c))
                .unwrap_or_else(|| right.chars().take(24).collect());
            (
                (left.clone(), right.clone()),
                left_label,
                right_label,
                rel.clone(),
            )
        })
        .collect();

    if let Some(secret) = secret {
        if let Ok(model_explanations) =
            query_model_link_explanations(secret, article_title, &links_to_explain).await
        {
            for (((left, right), _, _, _), explanation) in
                links_to_explain.iter().zip(model_explanations)
            {
                if !explanation.trim().is_empty() {
                    result.insert((left.clone(), right.clone()), explanation);
                }
            }
            if !result.is_empty() {
                return result;
            }
        }
    }

    // Deterministic fallback
    for ((left, right), left_label, right_label, rel) in &links_to_explain {
        result.insert(
            (left.clone(), right.clone()),
            explain_relation_link(rel, left_label, right_label),
        );
    }
    result
}

async fn query_model_link_explanations(
    secret: &ProviderSecret,
    article_title: &str,
    links: &[((String, String), String, String, String)],
) -> anyhow::Result<Vec<String>> {
    #[derive(serde::Deserialize)]
    struct ExplanationResp {
        explanations: Option<Vec<String>>,
    }

    let mut links_text = String::new();
    for (i, (_, left, right, rel)) in links.iter().enumerate() {
        links_text.push_str(&format!("{}. [{left}] —{rel}→ [{right}]\n", i + 1));
    }

    let system = "You are an OSINT intelligence analyst explaining relationship links between claims in an intelligence briefing.\n\
For each link, write a single concise, clear, and informative sentence explaining how they connect meaningfully.\n\
Avoid generic jargon. Be direct and readable.\n\
Return JSON only: {\"explanations\": [\"...\"]}";

    let user = format!(
        "Article: {article_title}\n\nLinks to explain:\n{links_text}\n\nReturn JSON only with 'explanations' array containing exactly {} strings in order.",
        links.len()
    );

    let messages = [
        ChatMessage {
            role: "system".into(),
            content: system.into(),
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

    let done = provider::complete(secret, &messages, &[], |_| {}).await?;
    let cleaned = strip_json_fences(&done.content);
    let parsed: ExplanationResp = serde_json::from_str(&cleaned)?;
    let mut explanations = parsed.explanations.unwrap_or_default();
    if explanations.len() < links.len() {
        for (_, left, right, rel) in links.iter().skip(explanations.len()) {
            explanations.push(explain_relation_link(rel, left, right));
        }
    }
    Ok(explanations)
}

fn strip_json_fences(text: &str) -> String {
    let trimmed = text.trim();
    if let Some(rest) = trimmed.strip_prefix("```") {
        let rest = rest.strip_prefix("json").unwrap_or(rest);
        let rest = rest.trim_start_matches('\n');
        if let Some(end) = rest.rfind("```") {
            return rest[..end].trim().to_string();
        }
    }
    trimmed.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claim(
        fp: &str,
        entity: &str,
        predicate: &str,
        object: &str,
        classification: &str,
        confidence: f64,
        text: &str,
    ) -> AtlasArticleClaim {
        AtlasArticleClaim {
            fingerprint: fp.into(),
            entity: entity.into(),
            predicate: predicate.into(),
            object: object.into(),
            topic: "geopolitical".into(),
            classification: classification.into(),
            confidence,
            claim: text.into(),
            source_url: "".into(),
            published_at: "".into(),
            article_id: "a".into(),
            reliability: "B".into(),
            info_credibility: 2,
            admiralty: "B2".into(),
            rsp_status: "".into(),
        }
    }

    #[test]
    fn buckets_split_facts_inferences_context_and_links() {
        let claims = vec![
            claim(
                "f1",
                "nato",
                "announces",
                "aid",
                "fact",
                0.9,
                "NATO announces aid.",
            ),
            claim(
                "f2",
                "nato",
                "signals",
                "escalation",
                "inference",
                0.5,
                "NATO signals escalation.",
            ),
            claim(
                "f3",
                "nato",
                "founded",
                "1949",
                "inference",
                0.6,
                "NATO was founded in 1949.",
            ),
            claim(
                "f4",
                "he",
                "said",
                "statement",
                "fact",
                0.8,
                "He issued a statement.",
            ),
        ];
        let relations = vec![("f3".into(), "f1".into(), "context_for".into())];
        let buckets = bucket_extracted(&claims, &relations);
        assert_eq!(buckets.facts.len(), 2);
        assert!(buckets.facts[0].text.contains("NATO announces"));
        assert_eq!(buckets.inferences.len(), 1);
        assert!(buckets.inferences[0].text.contains("signals"));
        assert_eq!(buckets.context.len(), 1);
        assert!(buckets.context[0].text.contains("1949"));
        assert!(buckets.actors.iter().any(|a| a == "nato"));
        // "he" should be filtered out from actors
        assert!(!buckets.actors.iter().any(|a| a == "he"));
        assert_eq!(buckets.links.len(), 1);
        assert!(buckets.links[0].contains("provides context for"));
    }
}
