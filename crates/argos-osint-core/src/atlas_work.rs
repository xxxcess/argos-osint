use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::store::AtlasArticleRow;
use crate::telemetry::{EventKind, TelemetryEvent, Trigger};

pub const EXTRACT_CONTRACT: &str = "extract-v2";
pub const STAGE_EXTRACT: i32 = 4;

/// Stage ids recorded on work units. Extraction is the only stage that writes
/// unit manifests today; the rest of the vocabulary exists so telemetry,
/// aggregation and the pipeline-backlog view spell every stage the same way.
pub const STAGE_COLLECTION: i32 = 1;
pub const STAGE_PUBLICATION: i32 = 6;
pub const STAGE_INDEXING: i32 = 7;

pub const DISPOSITION_SUCCESS: &str = "success";
pub const DISPOSITION_EMPTY: &str = "empty";
pub const DISPOSITION_REJECTED: &str = "rejected";
pub const DISPOSITION_FAILED: &str = "failed";
pub const DISPOSITION_INCOMPLETE: &str = "incomplete";

/// Exactly one disposition label applies to one work unit, so unit
/// dispositions sum to the unit total for a stage instead of overlapping.
/// An unrecognised or absent value stays visible as `pending` / `unknown`
/// rather than being silently merged into a success.
pub fn unit_disposition_label(disposition: &str) -> &'static str {
    match disposition {
        DISPOSITION_SUCCESS => "success",
        DISPOSITION_EMPTY => "empty",
        DISPOSITION_REJECTED => "rejected",
        DISPOSITION_FAILED => "failed",
        DISPOSITION_INCOMPLETE => "incomplete",
        "" => "pending",
        _ => "unknown",
    }
}

/// The unit a stage counts. Units from different stages are never added into
/// one grand total: articles, packets and memories are different work.
pub fn stage_unit(stage: i32) -> &'static str {
    match stage {
        STAGE_COLLECTION => "article",
        STAGE_EXTRACT => "packet",
        STAGE_PUBLICATION | STAGE_INDEXING => "memory",
        _ => "unit",
    }
}

/// Stage name used by every Atlas stage row, so the backlog view, the work-unit
/// ledger and the memory lifecycle all name a stage the same way.
pub fn stage_label(stage: i32) -> &'static str {
    match stage {
        STAGE_COLLECTION => "collection",
        STAGE_EXTRACT => "extraction",
        STAGE_PUBLICATION => "publication",
        STAGE_INDEXING => "indexing",
        _ => "unknown",
    }
}

/// Terminal cycle outcome vocabulary shared by the work-unit ledger, the memory
/// lifecycle and the Atlas cycle row.
pub fn cycle_outcome_label(state: &str) -> &'static str {
    match state {
        "completed" => "completed",
        "partial" => "partial",
        "failed" => "failed",
        "waiting" => "waiting",
        "blocked" => "blocked",
        "cancelled" | "paused" => "cancelled",
        _ => "unknown",
    }
}

/// Per-run, per-stage row counter. A resumed or replayed cycle appends the next
/// stage interval instead of overwriting or double counting the previous one.
/// Best-effort: a database that cannot be read returns zero.
pub fn next_stage_seq(db_path: &Path, run_id: &str, stage: &str) -> i64 {
    let prefix = format!("atlas-stage-{run_id}-{stage}-");
    rusqlite::Connection::open(db_path)
        .ok()
        .and_then(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM telemetry_events WHERE id GLOB ?1",
                rusqlite::params![format!("{prefix}*")],
                |row| row.get::<_, i64>(0),
            )
            .ok()
        })
        .unwrap_or(0)
        .max(0)
}

/// Terminal telemetry for one work unit. The event id is the unit key, so a
/// retried unit replaces its row and dispositions stay exactly one per unit.
/// Best-effort: a write failure never fails the pipeline.
pub fn record_unit_stage_telemetry(db_path: &Path, manifest: &UnitManifest) {
    if manifest.run_id.trim().is_empty() || manifest.unit_id.trim().is_empty() {
        return;
    }
    let label = unit_disposition_label(&manifest.disposition);
    let stage = stage_label(manifest.stage);
    let started = manifest
        .attempt_history
        .last()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());
    let event = TelemetryEvent::new(EventKind::AtlasStage)
        .at(started)
        .trigger(Trigger::AtlasCycle)
        .app("atlas")
        .run(&manifest.run_id)
        .outcome(label)
        .count(1)
        .payload(json!({
            "stage": stage,
            "unit": stage_unit(manifest.stage),
            "unit_id": manifest.unit_id,
            "required": manifest.is_required,
            "disposition": label,
            "contract_version": manifest.contract_version,
            "inputs": manifest.input_ids.len(),
            "terminal_reason": manifest.terminal_reason.clone().unwrap_or_default(),
        }));
    let event = event.with_id(format!(
        "atlas-unit-{}-{}",
        manifest.run_id, manifest.unit_id
    ));
    let _ = crate::telemetry::record(db_path, &event);
}

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

    #[test]
    fn disposition_labels_are_mutually_exclusive() {
        assert_eq!(unit_disposition_label(DISPOSITION_SUCCESS), "success");
        assert_eq!(unit_disposition_label(DISPOSITION_EMPTY), "empty");
        assert_eq!(unit_disposition_label(DISPOSITION_REJECTED), "rejected");
        assert_eq!(unit_disposition_label(DISPOSITION_FAILED), "failed");
        assert_eq!(unit_disposition_label(DISPOSITION_INCOMPLETE), "incomplete");
        // A unit that never ran stays visible instead of counting as work done.
        assert_eq!(unit_disposition_label(""), "pending");
        assert_eq!(unit_disposition_label("something_new"), "unknown");
    }

    #[test]
    fn each_stage_declares_one_unit() {
        assert_eq!(stage_unit(STAGE_EXTRACT), "packet");
        assert_eq!(stage_unit(STAGE_PUBLICATION), "memory");
        assert_eq!(stage_unit(STAGE_INDEXING), "memory");
        assert_eq!(stage_unit(STAGE_COLLECTION), "article");
        assert_eq!(stage_label(STAGE_EXTRACT), "extraction");
        assert_eq!(stage_label(STAGE_PUBLICATION), "publication");
        assert_eq!(stage_label(STAGE_INDEXING), "indexing");
        assert_eq!(stage_label(STAGE_COLLECTION), "collection");
        assert_eq!(cycle_outcome_label("paused"), "cancelled");
        assert_eq!(cycle_outcome_label("completed"), "completed");
    }

    #[test]
    fn unit_telemetry_key_is_stable_so_a_retry_replaces_its_row() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("argos.db");
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(crate::store::SCHEMA_TELEMETRY_SQL)
            .unwrap();
        drop(conn);
        let mut manifest = unit("u-1", true, DISPOSITION_FAILED, Some("timeout"));
        manifest.run_id = "atlas-run-1".into();
        record_unit_stage_telemetry(&path, &manifest);
        // A later retry of the same unit must not add a second row.
        manifest.disposition = DISPOSITION_SUCCESS.into();
        manifest.terminal_reason = None;
        record_unit_stage_telemetry(&path, &manifest);
        let conn = rusqlite::Connection::open(&path).unwrap();
        let mut stmt = conn
            .prepare("SELECT COUNT(*) FROM telemetry_events WHERE event_type='atlas_stage'")
            .unwrap();
        assert_eq!(stmt.query_row([], |r| r.get::<_, i64>(0)).unwrap(), 1);
        let mut stmt = conn
            .prepare("SELECT outcome FROM telemetry_events WHERE id='atlas-unit-atlas-run-1-u-1'")
            .unwrap();
        assert_eq!(
            stmt.query_row([], |r| r.get::<_, String>(0)).unwrap(),
            "success"
        );
    }
}
