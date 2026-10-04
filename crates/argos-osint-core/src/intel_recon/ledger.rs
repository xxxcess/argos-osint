//! Coverage ledger over Atlas claims and body-derived elements.

use anyhow::Result;

use crate::store::{AtlasArticleClaim, Store};

use super::persist::IntelElementRow;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ElementStatus {
    Pending,
    InProgress,
    Assessed,
    Unresolved,
    Superseded,
    Excluded,
}

impl ElementStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Assessed => "assessed",
            Self::Unresolved => "unresolved",
            Self::Superseded => "superseded",
            Self::Excluded => "excluded",
        }
    }
}

/// Seed the ledger from Atlas claims (and optional body-derived candidates).
pub fn seed_element_ledger(
    store: &Store,
    investigation_id: &str,
    claims: &[AtlasArticleClaim],
    body_candidates: &[(String, String)],
) -> Result<Vec<IntelElementRow>> {
    let mut out = Vec::new();
    for claim in claims {
        let key = format!("atlas:{}", claim.fingerprint);
        let element = store.upsert_element(
            investigation_id,
            &key,
            &claim.fingerprint,
            &claim.classification,
            &claim.claim,
            "atlas",
            ElementStatus::Pending.as_str(),
        )?;
        // Preserve Atlas original assessment separately.
        let original = serde_json::json!({
            "confidence": claim.confidence,
            "reliability": claim.reliability,
            "info_credibility": claim.info_credibility,
            "admiralty": claim.admiralty,
            "classification": claim.classification,
            "entity": claim.entity,
            "predicate": claim.predicate,
            "object": claim.object,
        });
        let _ = store.insert_assessment(
            investigation_id,
            &element.id,
            "atlas",
            "",
            "Atlas original assessment",
            claim.confidence,
            "[]",
            &original.to_string(),
            "{}",
        );
        out.push(element);
    }

    for (idx, (text, kind)) in body_candidates.iter().enumerate() {
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        // Skip duplicates that already match claim text.
        if claims.iter().any(|c| c.claim.eq_ignore_ascii_case(text)) {
            continue;
        }
        let key = format!("body:{idx}:{}", hash_key(text));
        let element = store.upsert_element(
            investigation_id,
            &key,
            "",
            kind,
            text,
            "article_body",
            ElementStatus::Pending.as_str(),
        )?;
        out.push(element);
    }

    Ok(out)
}

fn hash_key(text: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    format!("{:x}", hasher.finalize())
        .chars()
        .take(12)
        .collect()
}

/// Extract simple assertion candidates from article body when Atlas claims are absent.
pub fn body_assertion_candidates(body_markdown: &str, limit: usize) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for para in body_markdown.split("\n\n") {
        let para = para.trim();
        if para.len() < 40 || para.starts_with('#') {
            continue;
        }
        // Split on sentence boundaries lightly.
        for sentence in para.split(['.', '!', '?']) {
            let s = sentence.trim();
            if s.split_whitespace().count() < 6 {
                continue;
            }
            if s.chars().count() > 280 {
                continue;
            }
            out.push((format!("{s}."), "assertion".into()));
            if out.len() >= limit {
                return out;
            }
        }
    }
    out
}

pub fn coverage_complete(elements: &[IntelElementRow]) -> bool {
    !elements.is_empty()
        && elements.iter().all(|e| {
            matches!(
                e.status.as_str(),
                "assessed" | "unresolved" | "superseded" | "excluded"
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;

    #[test]
    fn seeds_atlas_claims_and_body_candidates() {
        let store = Store::memory().unwrap();
        let inv = store
            .ensure_intel_investigation("a1", "r1", "https://ex.com", "{}")
            .unwrap();
        let claim = AtlasArticleClaim {
            fingerprint: r#"["news","nato","announces","aid"]"#.into(),
            entity: "nato".into(),
            predicate: "announces".into(),
            object: "aid".into(),
            topic: "military".into(),
            classification: "fact".into(),
            confidence: 0.8,
            claim: "NATO announces aid".into(),
            source_url: "https://ex.com".into(),
            published_at: "2026-10-01".into(),
            article_id: "a1".into(),
            reliability: "B".into(),
            info_credibility: 2,
            admiralty: "B2".into(),
            rsp_status: "gr".into(),
        };
        let rows = seed_element_ledger(
            &store,
            &inv.id,
            &[claim],
            &[("Extra body assertion about logistics.".into(), "assertion".into())],
        )
        .unwrap();
        assert_eq!(rows.len(), 2);
        assert!(coverage_complete(&rows) == false);
    }
}
