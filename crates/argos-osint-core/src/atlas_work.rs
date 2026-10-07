use serde::{Deserialize, Serialize};

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
}

#[derive(Debug, Clone)]
pub struct DependencyCoverage {
    pub met: bool,
    pub missing_units: Vec<String>,
}

pub fn check_dependency_coverage(manifest: &UnitManifest, completed_units: &[String]) -> DependencyCoverage {
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
    let mut warnings = Vec::new();

    for m in manifests {
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

    if has_failed_required {
        return CycleOutcome::Failed;
    }

    if has_pending {
        return CycleOutcome::Pending;
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
