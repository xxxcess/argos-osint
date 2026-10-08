use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::store::AtlasArticleRow;

pub const EXTRACT_CONTRACT: &str = "extract-v2";
pub const STAGE_EXTRACT: i32 = 4;

pub const DISPOSITION_SUCCESS: &str = "success";
pub const DISPOSITION_EMPTY: &str = "empty";
pub const DISPOSITION_REJECTED: &str = "rejected";
pub const DISPOSITION_FAILED: &str = "failed";
pub const DISPOSITION_INCOMPLETE: &str = "incomplete";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AttemptRecord {
    #[serde(default)]
    pub at: String,
    #[serde(default)]
    pub route_index: u32,
    #[serde(default)]
    pub attempt: u32,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub dispatched: bool,
    #[serde(default)]
    pub outcome: String,
    #[serde(default)]
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnitManifest {
    pub run_id: String,
    pub unit_id: String,
    pub stage: i32,
    pub input_ids: Vec<String>,
    pub input_rev: String,
    pub contract_version: String,
    pub dependency_ids: Vec<String>,
    pub is_required: bool,
    pub output_refs: Vec<String>,
    pub effective_model: String,
    pub attempt_history: Vec<String>,
    pub next_eligible_at: Option<String>,
    pub terminal_reason: Option<String>,
    #[serde(default)]
    pub output_json: String,
    #[serde(default)]
    pub disposition: String,
    #[serde(default)]
    pub receipt_json: String,
}

impl UnitManifest {
    /// Empty output references are not proof of reusable success.
    pub fn reusable_success(&self) -> bool {
        self.terminal_reason.is_none()
            && !self.output_json.trim().is_empty()
            && (self.disposition == DISPOSITION_SUCCESS || self.disposition == DISPOSITION_EMPTY)
    }

    pub fn exhausted_required_failure(&self) -> bool {
        self.is_required
            && self.terminal_reason.is_some()
            && (self.disposition == DISPOSITION_FAILED
                || self.disposition == DISPOSITION_INCOMPLETE)
    }
}

#[derive(Debug, Clone)]
pub struct DependencyCoverage {
    pub met: bool,
    pub missing_units: Vec<String>,
}

pub fn check_dependency_coverage(
    manifest: &UnitManifest,
    completed_units: &[String],
) -> DependencyCoverage {
    let mut missing_units = Vec::new();
    for dep in &manifest.dependency_ids {
        if !completed_units.contains(dep) {
            missing_units.push(dep.clone());
        }
    }
    DependencyCoverage {
        met: missing_units.is_empty(),
        missing_units,
    }
}

pub fn reduce_cycle_outcome(manifests: &[UnitManifest]) -> CycleOutcome {
    if manifests.is_empty() {
        return CycleOutcome::Pending;
    }

    let mut has_pending = false;
    let mut has_failed_required = false;
    let mut has_useful = false;
    let mut warnings = Vec::new();

    for m in manifests {
        if m.reusable_success() {
            has_useful = true;
            continue;
        }
        if let Some(ref reason) = m.terminal_reason {
            if m.is_required {
                has_failed_required = true;
            } else {
                warnings.push(format!("{}: {}", m.unit_id, reason));
            }
        } else if m.next_eligible_at.is_some() || m.attempt_history.is_empty() {
            has_pending = true;
        }
    }

    if has_pending {
        return CycleOutcome::Waiting;
    }

    if has_failed_required {
        if has_useful {
            return CycleOutcome::Partial;
        }
        return CycleOutcome::Failed;
    }

    if !warnings.is_empty() {
        return CycleOutcome::CompletedWithWarnings(warnings);
    }

    CycleOutcome::Completed
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CycleOutcome {
    Completed,
    CompletedWithWarnings(Vec<String>),
    Waiting,
    Blocked,
    Partial,
    Failed,
    Cancelled,
    Pending,
}

pub fn hex12(bytes: &[u8]) -> String {
    bytes.iter().take(12).map(|b| format!("{b:02x}")).collect()
}

/// Stable unit identity = stage kind + canonical input IDs/revisions + contract.
pub fn packet_identity(kind: &str, input_ids: &[String], input_rev: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(kind.as_bytes());
    hasher.update(b"\0");
    let mut ids = input_ids.to_vec();
    ids.sort();
    for id in &ids {
        hasher.update(id.as_bytes());
        hasher.update(b"\0");
    }
    hasher.update(input_rev.as_bytes());
    hasher.update(b"\0");
    hasher.update(EXTRACT_CONTRACT.as_bytes());
    format!("{kind}-{}", hex12(&hasher.finalize()))
}

pub fn articles_rev(articles: &[AtlasArticleRow]) -> String {
    let mut hasher = Sha256::new();
    for article in articles {
        hasher.update(article.id.as_bytes());
        hasher.update(b"\0");
        hasher.update(article.title.as_bytes());
        hasher.update(b"\0");
        hasher.update(article.description.as_bytes());
        hasher.update(b"\0");
        hasher.update(article.published_at.as_bytes());
        hasher.update(b"\n");
    }
    hex12(&hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(id: &str, required: bool, disposition: &str, reason: Option<&str>) -> UnitManifest {
        UnitManifest {
            run_id: "r".into(),
            unit_id: id.into(),
            stage: 4,
            input_ids: vec!["a".into()],
            input_rev: "1".into(),
            contract_version: EXTRACT_CONTRACT.into(),
            dependency_ids: Vec::new(),
            is_required: required,
            output_refs: Vec::new(),
            effective_model: "m".into(),
            attempt_history: vec!["t".into()],
            next_eligible_at: None,
            terminal_reason: reason.map(str::to_string),
            output_json: if disposition == DISPOSITION_SUCCESS {
                "[]".into()
            } else if disposition == DISPOSITION_EMPTY {
                "[]".into()
            } else {
                String::new()
            },
            disposition: disposition.into(),
            receipt_json: String::new(),
        }
    }

    #[test]
    fn empty_output_refs_are_not_reusable() {
        let mut m = unit("u", true, DISPOSITION_SUCCESS, None);
        m.output_json.clear();
        assert!(!m.reusable_success());
    }

    #[test]
    fn optional_failure_is_completed_with_warnings() {
        let manifests = vec![
            unit("lead", true, DISPOSITION_SUCCESS, None),
            unit(
                "ctx",
                false,
                DISPOSITION_FAILED,
                Some("context packet failed"),
            ),
        ];
        match reduce_cycle_outcome(&manifests) {
            CycleOutcome::CompletedWithWarnings(w) => {
                assert!(w[0].contains("ctx"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn required_failure_with_useful_output_is_partial() {
        let manifests = vec![
            unit("lead-1", true, DISPOSITION_SUCCESS, None),
            unit("lead-2", true, DISPOSITION_FAILED, Some("timeout")),
        ];
        assert_eq!(reduce_cycle_outcome(&manifests), CycleOutcome::Partial);
    }

    #[test]
    fn identity_is_stable_for_the_same_inputs() {
        let a = packet_identity("lead", &["b".into(), "a".into()], "rev");
        let b = packet_identity("lead", &["a".into(), "b".into()], "rev");
        assert_eq!(a, b);
        assert_ne!(
            packet_identity("lead", &["a".into()], "rev"),
            packet_identity("ctx", &["a".into()], "rev")
        );
    }
}
