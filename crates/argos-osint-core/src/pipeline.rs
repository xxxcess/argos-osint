//! Pipeline adoption helpers (spec §12–14): directive coverage, Atlas event
//! grouping, claim comparison, and Intel selective refresh markers.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectiveCoverage {
    pub directive_id: String,
    pub goal: String,
    /// Evidence call IDs that support this directive (similarity is not enough).
    pub supporting_call_ids: Vec<String>,
    pub covered: bool,
    pub notes: String,
}

/// Score directive coverage from explicit evidence citations only — never from
/// embedding similarity alone.
pub fn directive_coverage(
    directives: &[(String, String)],
    evidence_by_directive: &[(String, Vec<String>)],
) -> Vec<DirectiveCoverage> {
    let mut out = Vec::with_capacity(directives.len());
    for (id, goal) in directives {
        let supporting = evidence_by_directive
            .iter()
            .find(|(d, _)| d == id)
            .map(|(_, ids)| ids.clone())
            .unwrap_or_default();
        let covered = !supporting.is_empty();
        out.push(DirectiveCoverage {
            directive_id: id.clone(),
            goal: goal.clone(),
            supporting_call_ids: supporting,
            covered,
            notes: if covered {
                "supported by cited evidence".into()
            } else {
                "gap: no cited evidence yet; similarity alone is not completion".into()
            },
        });
    }
    out
}

/// Remaining gaps after coverage scoring (for gap-directed collection).
pub fn uncovered_directives(coverage: &[DirectiveCoverage]) -> Vec<&DirectiveCoverage> {
    coverage.iter().filter(|c| !c.covered).collect()
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventGroup {
    pub event_key: String,
    pub article_ids: Vec<String>,
    pub title_fingerprint: String,
}

/// Group paraphrased news about one event; keep recurring/separate events apart.
/// Uses normalized title tokens — not embeddings — so syndicated copies do not
/// inflate corroboration.
pub fn group_atlas_events(articles: &[(String, String, String)]) -> Vec<EventGroup> {
    // (id, title, published_day) — same calendar day + high title-token overlap = one event.
    let mut groups: Vec<(EventGroup, std::collections::HashSet<String>)> = Vec::new();
    for (id, title, day) in articles {
        let tokens = title_tokens(title);
        let fp = title_fingerprint(&tokens);
        let mut placed = false;
        for (g, existing) in groups.iter_mut() {
            if !g.event_key.starts_with(&format!("{day}:")) {
                continue;
            }
            if jaccard(existing, &tokens) >= 0.5 {
                if !g.article_ids.contains(id) {
                    g.article_ids.push(id.clone());
                }
                existing.extend(tokens.iter().cloned());
                placed = true;
                break;
            }
        }
        if !placed {
            let key = format!("{day}:{fp}");
            groups.push((
                EventGroup {
                    event_key: key,
                    article_ids: vec![id.clone()],
                    title_fingerprint: fp,
                },
                tokens,
            ));
        }
    }
    groups.into_iter().map(|(g, _)| g).collect()
}

fn title_tokens(title: &str) -> std::collections::HashSet<String> {
    title
        .to_ascii_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() > 2)
        .map(|t| t.trim_end_matches('s').to_string())
        .collect()
}

fn jaccard(a: &std::collections::HashSet<String>, b: &std::collections::HashSet<String>) -> f32 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let inter = a.intersection(b).count() as f32;
    let union = a.union(b).count() as f32;
    if union == 0.0 {
        0.0
    } else {
        inter / union
    }
}

fn title_fingerprint(tokens: &std::collections::HashSet<String>) -> String {
    let mut ordered: Vec<&String> = tokens.iter().collect();
    ordered.sort();
    let joined = ordered
        .into_iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(" ");
    let mut hasher = Sha256::new();
    hasher.update(joined.as_bytes());
    format!("{:x}", hasher.finalize())[..16].to_string()
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClaimRelation {
    Equivalent,
    Negation,
    PlannedVsCompleted,
    ChangedQuantity,
    AmbiguousEntity,
    Unrelated,
}

/// Lightweight claim comparison for insight consolidation fixtures.
pub fn compare_claims(a: &str, b: &str) -> ClaimRelation {
    let na = normalize_claim(a);
    let nb = normalize_claim(b);
    if na == nb {
        return ClaimRelation::Equivalent;
    }
    let neg = |s: &str| {
        let padded = format!(" {s} ");
        padded.contains(" not ")
            || padded.contains(" never ")
            || padded.contains(" no ")
            || padded.contains(" does not ")
            || padded.contains(" did not ")
    };
    if neg(&na) != neg(&nb) && share_content_tokens(&na, &nb) {
        return ClaimRelation::Negation;
    }
    let planned = |s: &str| s.contains("will ") || s.contains(" plan") || s.contains("scheduled");
    let done = |s: &str| s.contains(" completed") || s.contains(" finished") || s.contains(" signed");
    if (planned(&na) && done(&nb)) || (planned(&nb) && done(&na)) {
        return ClaimRelation::PlannedVsCompleted;
    }
    if quantity_tokens(&na) != quantity_tokens(&nb) && share_content_tokens(&na, &nb) {
        return ClaimRelation::ChangedQuantity;
    }
    if share_content_tokens(&na, &nb) {
        return ClaimRelation::AmbiguousEntity;
    }
    ClaimRelation::Unrelated
}

fn normalize_claim(s: &str) -> String {
    s.to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() || c.is_whitespace() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn share_content_tokens(a: &str, b: &str) -> bool {
    let stem = |t: &str| t.trim_end_matches('s').to_string();
    let ta: std::collections::HashSet<String> = a
        .split_whitespace()
        .map(stem)
        .filter(|t| t.len() > 2)
        .collect();
    let tb: std::collections::HashSet<String> = b
        .split_whitespace()
        .map(stem)
        .filter(|t| t.len() > 2)
        .collect();
    ta.intersection(&tb).count() >= 2
}

fn quantity_tokens(s: &str) -> Vec<String> {
    s.split_whitespace()
        .filter(|t| t.chars().any(|c| c.is_ascii_digit()))
        .map(str::to_string)
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StaleSection {
    pub section_key: String,
    pub reason: String,
}

/// Mark only sections that depend on changed evidence IDs as stale.
pub fn selective_refresh_targets(
    section_evidence: &[(String, Vec<String>)],
    changed_evidence_ids: &[String],
) -> Vec<StaleSection> {
    let changed: std::collections::HashSet<&str> =
        changed_evidence_ids.iter().map(String::as_str).collect();
    section_evidence
        .iter()
        .filter(|(_, ids)| ids.iter().any(|id| changed.contains(id.as_str())))
        .map(|(key, _)| StaleSection {
            section_key: key.clone(),
            reason: "evidence revision changed".into(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coverage_requires_cited_evidence_not_similarity() {
        let dirs = vec![("D1".into(), "find CEO".into()), ("D2".into(), "find HQ".into())];
        let ev = vec![("D1".into(), vec!["call-1".into()])];
        let cov = directive_coverage(&dirs, &ev);
        assert!(cov[0].covered);
        assert!(!cov[1].covered);
        assert_eq!(uncovered_directives(&cov).len(), 1);
    }

    #[test]
    fn event_grouping_keeps_separate_days_apart() {
        let articles = vec![
            ("a1".into(), "Geneva hosts peace talks".into(), "2026-10-01".into()),
            ("a2".into(), "Peace talks hosted in Geneva".into(), "2026-10-01".into()),
            ("a3".into(), "Geneva hosts peace talks".into(), "2026-10-05".into()),
        ];
        let groups = group_atlas_events(&articles);
        assert_eq!(groups.len(), 2);
        let same_day = groups.iter().find(|g| g.event_key.starts_with("2026-10-01")).unwrap();
        assert_eq!(same_day.article_ids.len(), 2);
    }

    #[test]
    fn claim_compare_detects_negation_and_quantity() {
        assert_eq!(
            compare_claims("Ada leads Example Org", "Ada leads Example Org"),
            ClaimRelation::Equivalent
        );
        assert_eq!(
            compare_claims("Ada leads Example Org", "Ada does not lead Example Org"),
            ClaimRelation::Negation
        );
        assert_eq!(
            compare_claims("raised 10 million dollars", "raised 25 million dollars"),
            ClaimRelation::ChangedQuantity
        );
    }

    #[test]
    fn selective_refresh_only_marks_affected_sections() {
        let sections = vec![
            ("bluf".into(), vec!["iev-1".into(), "iev-2".into()]),
            ("background".into(), vec!["iev-9".into()]),
        ];
        let stale = selective_refresh_targets(&sections, &["iev-1".into()]);
        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].section_key, "bluf");
    }
}
