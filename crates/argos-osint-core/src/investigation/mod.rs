//! Unified investigation harness shared across Recon chat, Home composer, and Intel briefing.

pub mod contracts;
pub mod decisions;
pub mod engine;
pub mod evidence;
pub mod gates;
pub mod patterns;
pub mod persist;
pub mod policy;
pub mod recovery;
pub mod roles;
pub mod trace;

#[cfg(test)]
pub mod tests_acceptance;

pub use contracts::{CallProposal, HandoffRecord, InvestigationSurface, TaskRecord, TaskStatus};
pub use engine::{execute_harness_step, InvestigationRuntime};
pub use evidence::{
    assess_claim_against_passages, curate_passages_from_result, ClaimAssessment,
    ClaimAssessmentOutcome, ClaimStance, EntityBinding, EntityBindingStatus, EvidencePassage,
};
pub use gates::{
    validate_claim_assessment, validate_evidence_admission, validate_publication,
    validate_task_admission, validate_tool_preflight, GateOutcome,
};
pub use patterns::InvestigationPattern;
pub use policy::{check_need_gate, is_observation_fresh, SurfacePolicy};
pub use recovery::{classify_recovery, RecoveryAction};
pub use roles::LogicalRole;
pub use trace::InvestigationEvent;
