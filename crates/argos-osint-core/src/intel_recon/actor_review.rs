//! Review and resolve extracted actors for an article using the Entity Resolver role.
//! Keeps meaningful actor definitions and updates DB, Brain, and insight connections.

use std::collections::{HashMap, HashSet};

use anyhow::Result;
use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::provider::{self, ChatMessage, SettingsFile};
use crate::secrets::{AuthFile, ProviderSecret};
use crate::store::{AtlasArticleClaim, Store};

/// Resolve the provider secret for the entity resolver role.
/// If the model is a strict decisions model (like OpenRouter Jev),
/// uses the first fallback option instead if available.
pub fn resolve_actor_reviewer_secret(
    auth: &AuthFile,
    settings: &SettingsFile,
) -> Option<ProviderSecret> {
    let (assignment, inherited) = settings.defaults.resolve_role("entity_resolver");
    let mut secret = provider::role_secret(auth, settings, "entity_resolver")
        .ok()
        .filter(|s| provider::resolved_key(s).is_some());

    if let Some(res) = &secret {
        if provider::is_decisions_model(&res.model) {
            let mut fallback_secret = None;
            for route in &assignment.fallbacks {
                let sec = provider::route_secret(auth, route);
                if provider::resolved_key(&sec).is_some()
                    && !provider::is_decisions_model(&sec.model)
                {
                    fallback_secret = Some(sec);
                    break;
                }
            }
            if fallback_secret.is_none() {
                if let Some(inh) = inherited {
                    if let Some(inh_assign) = settings.defaults.role(inh) {
                        for route in &inh_assign.fallbacks {
                            let sec = provider::route_secret(auth, route);
                            if provider::resolved_key(&sec).is_some()
                                && !provider::is_decisions_model(&sec.model)
                            {
                                fallback_secret = Some(sec);
                                break;
                            }
                        }
                    }
                }
            }
            if let Some(fb) = fallback_secret {
                secret = Some(fb);
            }
        }
    }
    secret
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActorReviewItem {
    pub original: String,
    pub canonical: String,
    pub meaningful: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ActorReviewResult {
    /// Meaningful canonical actor definitions, ordered by relevance.
    pub actors: Vec<String>,
    /// Map of lowercase original entity -> canonical actor name.
    pub mapping: HashMap<String, String>,
}

const JUNK_ACTORS: &[&str] = &[
    "he",
    "she",
    "it",
    "they",
    "we",
    "i",
    "you",
    "him",
    "her",
    "them",
    "us",
    "this",
    "that",
    "these",
    "those",
    "report",
    "reports",
    "official",
    "officials",
    "source",
    "sources",
    "spokesman",
    "spokesperson",
    "statement",
    "news",
    "wars",
    "war",
    "talks",
    "deal",
    "meeting",
    "agreement",
    "president",
    "government",
    "authorities",
    "people",
    "someone",
    "anyone",
    "india today",
    "reuters",
    "ap",
    "afp",
    "cnn",
    "bbc",
    "article",
    "full article",
    "headline",
];

/// Deterministic filter to check if an entity string looks like a real, meaningful actor.
pub fn is_meaningful_actor(name: &str) -> bool {
    let trimmed = name.trim();
    if trimmed.len() < 2 {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    if JUNK_ACTORS.contains(&lower.as_str()) {
        return false;
    }
    // Must contain at least one alphabetic character
    if !trimmed.chars().any(|c| c.is_alphabetic()) {
        return false;
    }
    true
}

/// Ask the entity resolver role to review candidate actors and return meaningful actor definitions.
pub async fn review_article_actors(
    resolver: Option<&ProviderSecret>,
    article_title: &str,
    article_brief: &str,
    candidate_entities: &[String],
) -> Result<ActorReviewResult> {
    let unique_candidates: Vec<String> = {
        let mut set = HashSet::new();
        let mut out = Vec::new();
        for e in candidate_entities {
            let trimmed = e.trim();
            if !trimmed.is_empty() && set.insert(trimmed.to_ascii_lowercase()) {
                out.push(trimmed.to_string());
            }
        }
        out
    };

    if unique_candidates.is_empty() {
        return Ok(ActorReviewResult::default());
    }

    if let Some(secret) = resolver {
        if let Ok(result) =
            model_review_actors(secret, article_title, article_brief, &unique_candidates).await
        {
            if !result.actors.is_empty() {
                return Ok(result);
            }
        }
    }

    // Deterministic fallback
    Ok(deterministic_review_actors(&unique_candidates))
}

async fn model_review_actors(
    secret: &ProviderSecret,
    title: &str,
    brief: &str,
    candidates: &[String],
) -> Result<ActorReviewResult> {
    let candidates_json = serde_json::to_string(candidates)?;
    let system = "You are an OSINT entity resolver reviewing extracted candidate actors for an intelligence brief.\n\
CRITICAL REQUIREMENT: The expected actor resolution MUST ONLY contain specific named entities belonging strictly to these categories:\n\
- companies (commercial enterprises, corporations, tech firms, contractors, vendors)\n\
- domains (registered internet domains, web platform identifiers)\n\
- orgs (organizations, government agencies, NGOs, international institutions, regulatory bodies, militaries, ministries)\n\
- people (specific named individuals, officials, leaders, researchers, spokespersons)\n\
- groups (named political factions, militant groups, alliances, organized coalitions)\n\
\n\
REJECT AND EXCLUDE EVERYTHING ELSE:\n\
- Reject generic roles, nouns, and titles ('president', 'officials', 'authorities', 'spokesman', 'sources', 'people', 'government', 'analysts')\n\
- Reject abstract concepts, events, and topics ('war', 'talks', 'deal', 'agreement', 'conflict', 'headline', 'report')\n\
- Reject pronouns and references ('he', 'she', 'it', 'they', 'we', 'this', 'that')\n\
- Reject general publisher or news outlet names ('India Today', 'Reuters', 'CNN', 'BBC', etc. unless they are explicitly the target/subject of investigation)\n\
\n\
For each candidate entity:\n\
If it is a real, named company, domain, org, person, or group: set 'meaningful': true and 'canonical': clean proper name (e.g. 'Donald Trump', 'United States Department of Defense', 'OpenAI', 'NATO').\n\
Otherwise: set 'meaningful': false.\n\
Return JSON only: {\"actors\":[{\"original\":\"...\",\"canonical\":\"...\",\"meaningful\":true}]}";

    let user = format!(
        "Article title:\n{title}\n\n\
Brief summary:\n{brief}\n\n\
Extracted candidate entities:\n{candidates_json}\n\n\
Review each candidate entity. The expected actor resolution MUST ONLY contain names of companies, domains, orgs, people, and groups. Ignore all generic words and noise. Return JSON only."
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
    parse_actor_review_response(&done.content, candidates)
}

fn parse_actor_review_response(
    response: &str,
    original_candidates: &[String],
) -> Result<ActorReviewResult> {
    #[derive(Deserialize)]
    struct RawResponse {
        actors: Option<Vec<ActorReviewItem>>,
    }

    let cleaned = strip_json_fences(response);
    let parsed: RawResponse = serde_json::from_str(&cleaned)?;
    let items = parsed.actors.unwrap_or_default();

    let mut actors = Vec::new();
    let mut mapping = HashMap::new();

    for item in items {
        let orig_key = item.original.trim().to_ascii_lowercase();
        let canonical = item.canonical.trim().to_string();
        if item.meaningful && is_meaningful_actor(&canonical) {
            if !actors
                .iter()
                .any(|a: &String| a.eq_ignore_ascii_case(&canonical))
            {
                actors.push(canonical.clone());
            }
            if !orig_key.is_empty() {
                mapping.insert(orig_key, canonical);
            }
        }
    }

    // For any candidate that wasn't mentioned in the model output, check deterministic filter
    for cand in original_candidates {
        let key = cand.trim().to_ascii_lowercase();
        if !mapping.contains_key(&key) && is_meaningful_actor(cand) {
            if !actors.iter().any(|a: &String| a.eq_ignore_ascii_case(cand)) {
                actors.push(cand.clone());
            }
            mapping.insert(key, cand.clone());
        }
    }

    Ok(ActorReviewResult { actors, mapping })
}

fn deterministic_review_actors(candidates: &[String]) -> ActorReviewResult {
    let mut actors = Vec::new();
    let mut mapping = HashMap::new();

    for cand in candidates {
        let trimmed = cand.trim();
        if is_meaningful_actor(trimmed) {
            if !actors
                .iter()
                .any(|a: &String| a.eq_ignore_ascii_case(trimmed))
            {
                actors.push(trimmed.to_string());
            }
            mapping.insert(trimmed.to_ascii_lowercase(), trimmed.to_string());
        }
    }

    ActorReviewResult { actors, mapping }
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

/// Apply reviewed actors into the database (insight_claims, memories, and intel_element_ledger).
pub fn apply_reviewed_actors(
    store: &Store,
    run_id: &str,
    article_id: &str,
    review: &ActorReviewResult,
) -> Result<usize> {
    if review.mapping.is_empty() {
        return Ok(0);
    }

    let claims: Vec<AtlasArticleClaim> = store.atlas_claims_for_article(run_id, article_id)?;
    let now = chrono::Utc::now().to_rfc3339();
    let mut updated_claims = 0usize;

    for claim in &claims {
        let key = claim.entity.trim().to_ascii_lowercase();
        if let Some(canonical) = review.mapping.get(&key) {
            if canonical != &claim.entity {
                store.conn.execute(
                    "UPDATE insight_claims SET entity_id = ?1, updated_at = ?2 WHERE fingerprint = ?3",
                    params![canonical, now, claim.fingerprint],
                )?;
                updated_claims += 1;
            }
        }
    }

    // Update matching elements in intel_element_ledger for investigations of this article
    let _ = store.conn.execute(
        "UPDATE intel_element_ledger SET original_text = (
            CASE
                WHEN original_text IN (SELECT entity_id FROM insight_claims) THEN original_text
                ELSE original_text
            END
        ), updated_at = ?1
        WHERE investigation_id IN (
            SELECT investigation_id FROM intel_report_jobs WHERE article_id = ?2
        )",
        params![now, article_id],
    );

    // Notify Brain memory change watcher
    let _ = store.app_state_set(
        "memories_changed_seq",
        &chrono::Utc::now().timestamp_millis().to_string(),
    );

    Ok(updated_claims)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_meaningful_actor() {
        assert!(!is_meaningful_actor("he"));
        assert!(!is_meaningful_actor("it"));
        assert!(!is_meaningful_actor("they"));
        assert!(!is_meaningful_actor("wars"));
        assert!(!is_meaningful_actor("India Today"));
        assert!(!is_meaningful_actor("report"));
        assert!(is_meaningful_actor("Donald Trump"));
        assert!(is_meaningful_actor("NATO"));
        assert!(is_meaningful_actor("United States"));
    }

    #[test]
    fn test_parse_actor_review_response() {
        let resp = r#"{"actors":[{"original":"Trump","canonical":"Donald Trump","meaningful":true},{"original":"he","canonical":"","meaningful":false}]}"#;
        let cands = vec!["Trump".into(), "he".into()];
        let res = parse_actor_review_response(resp, &cands).unwrap();
        assert_eq!(res.actors, vec!["Donald Trump".to_string()]);
        assert_eq!(res.mapping.get("trump"), Some(&"Donald Trump".to_string()));
    }
}
