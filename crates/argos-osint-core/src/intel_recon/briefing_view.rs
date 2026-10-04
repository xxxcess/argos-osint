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

/// Partition claims and relations into briefing buckets.
pub fn bucket_extracted(
    claims: &[AtlasArticleClaim],
    relations: &[(String, String, String)],
) -> ExtractedBuckets {
    let context_fps: std::collections::HashSet<&str> = relations
        .iter()
        .filter(|(_, _, rel)| rel == "context_for")
        .map(|(left, _, _)| left.as_str())
        .collect();

    let by_fp: std::collections::HashMap<&str, &AtlasArticleClaim> = claims
        .iter()
        .map(|c| (c.fingerprint.as_str(), c))
        .collect();

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
        if entity.is_empty() || actors.iter().any(|item| item == entity) {
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
        let left_label = by_fp
            .get(left.as_str())
            .map(|c| short_claim(c))
            .unwrap_or_else(|| left.chars().take(24).collect());
        let right_label = by_fp
            .get(right.as_str())
            .map(|c| short_claim(c))
            .unwrap_or_else(|| right.chars().take(24).collect());
        links.push(format!("{left_label} —{rel}→ {right_label}"));
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
            claim("f1", "nato", "announces", "aid", "fact", 0.9, "NATO announces aid."),
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
        ];
        let relations = vec![("f3".into(), "f1".into(), "context_for".into())];
        let buckets = bucket_extracted(&claims, &relations);
        assert_eq!(buckets.facts.len(), 1);
        assert!(buckets.facts[0].text.contains("NATO announces"));
        assert_eq!(buckets.inferences.len(), 1);
        assert!(buckets.inferences[0].text.contains("signals"));
        assert_eq!(buckets.context.len(), 1);
        assert!(buckets.context[0].text.contains("1949"));
        assert!(buckets.actors.iter().any(|a| a == "nato"));
        assert_eq!(buckets.links.len(), 1);
        assert!(buckets.links[0].contains("context_for"));
    }
}
