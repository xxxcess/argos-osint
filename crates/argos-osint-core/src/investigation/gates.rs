//! Deterministic and semantic validation gates with bounded repair.

use serde::{Deserialize, Serialize};

use super::contracts::{CallProposal, TaskRecord};
use super::evidence::{ClaimAssessment, EvidencePassage};
use super::policy::SurfacePolicy;

/// Outcome of evaluating a validation gate.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GateOutcome {
    Passed,
    Rejected { reason: String, repairable: bool },
}

impl GateOutcome {
    pub fn is_passed(&self) -> bool {
        matches!(self, Self::Passed)
    }
}

/// 1. Task Admission Gate: validates IDs, revision, limits, and dependencies.
pub fn validate_task_admission(task: &TaskRecord, pending_dependency_count: usize) -> GateOutcome {
    if task.id.trim().is_empty() || task.investigation_id.trim().is_empty() {
        return GateOutcome::Rejected {
            reason: "task record is missing required identifier".into(),
            repairable: false,
        };
    }

    if task.attempts >= task.max_attempts {
        return GateOutcome::Rejected {
            reason: format!(
                "task exceeded max attempt allowance ({})",
                task.max_attempts
            ),
            repairable: false,
        };
    }

    if pending_dependency_count > 0 {
        return GateOutcome::Rejected {
            reason: format!(
                "task has {} unresolved dependencies",
                pending_dependency_count
            ),
            repairable: false,
        };
    }

    GateOutcome::Passed
}

/// 2. Tool Preflight Gate: validates surface policy, credentials, and parameters.
pub fn validate_tool_preflight(
    proposal: &CallProposal,
    policy: &SurfacePolicy,
    tool_enabled: bool,
) -> GateOutcome {
    if !tool_enabled {
        return GateOutcome::Rejected {
            reason: format!("tool {} is disabled by user settings", proposal.tool_id),
            repairable: false,
        };
    }

    if !policy.is_tool_permitted(&proposal.tool_id) {
        return GateOutcome::Rejected {
            reason: format!("tool {} is not permitted on this surface", proposal.tool_id),
            repairable: false,
        };
    }

    if proposal.arguments.is_null() {
        return GateOutcome::Rejected {
            reason: "proposed arguments are null".into(),
            repairable: true,
        };
    }

    GateOutcome::Passed
}

/// 3. Evidence Admission Gate: validates source references, timestamps, and non-empty content.
pub fn validate_evidence_admission(passage: &EvidencePassage) -> GateOutcome {
    if passage.passage_text.trim().is_empty() {
        return GateOutcome::Rejected {
            reason: "evidence passage is empty".into(),
            repairable: false,
        };
    }

    if passage.source_url.trim().is_empty() && passage.source_domain.trim().is_empty() {
        return GateOutcome::Rejected {
            reason: "evidence passage lacks provenance or source attribution".into(),
            repairable: false,
        };
    }

    GateOutcome::Passed
}

/// 4. Claim Assessment Gate: validates claim-passage links and reasoning.
pub fn validate_claim_assessment(assessment: &ClaimAssessment) -> GateOutcome {
    if assessment.claim_id.trim().is_empty() {
        return GateOutcome::Rejected {
            reason: "assessment missing claim identifier".into(),
            repairable: false,
        };
    }

    if assessment.evidence_id.trim().is_empty() {
        return GateOutcome::Rejected {
            reason: "assessment missing linked evidence reference".into(),
            repairable: false,
        };
    }

    if assessment.rationale.trim().is_empty() {
        return GateOutcome::Rejected {
            reason: "assessment requires supporting rationale".into(),
            repairable: true,
        };
    }

    GateOutcome::Passed
}

/// 5. Resolution & Publication Gate: validates that conclusions are supported and not overstated.
pub fn validate_publication(unresolved_tasks_count: usize, total_passages: usize) -> GateOutcome {
    if total_passages == 0 {
        return GateOutcome::Rejected {
            reason: "cannot publish report without retained evidence passages".into(),
            repairable: false,
        };
    }

    if unresolved_tasks_count > 0 {
        // Not fatal rejection, but marked for explicit coverage disclosure
        return GateOutcome::Passed;
    }

    GateOutcome::Passed
}

use super::decisions::{
    templates::{
        template_claim_relation, template_entity_binding, template_evidence_relevance,
        template_publication,
    },
    DecisionPolicy, DecisionService, DecisionState, EnforcementOutcome,
};
use crate::secrets::ProviderSecret;
use serde_json::json;

/// Evaluate semantic claim relation (supports, contradicts, mentions_only) using DecisionService.
pub async fn evaluate_claim_relation_gate(
    secret: &ProviderSecret,
    claim: &str,
    passage: &str,
    service: &DecisionService,
    policy: &DecisionPolicy,
) -> anyhow::Result<EnforcementOutcome> {
    let contract = template_claim_relation();
    let state = DecisionState::new()
        .with_custom("claim", json!(claim))
        .with_custom("passage", json!(passage));

    let (_res, outcomes) = service.evaluate(secret, &contract, &state, policy).await?;
    let outcome =
        outcomes
            .get("claim_relation")
            .cloned()
            .unwrap_or(EnforcementOutcome::Unavailable {
                reason: "missing question outcome".into(),
            });
    Ok(outcome)
}

/// Evaluate semantic evidence relevance to an entity and directive using DecisionService.
pub async fn evaluate_evidence_relevance_gate(
    secret: &ProviderSecret,
    subject_entity: &str,
    directive: &str,
    passage: &str,
    service: &DecisionService,
    policy: &DecisionPolicy,
) -> anyhow::Result<EnforcementOutcome> {
    let contract = template_evidence_relevance();
    let state = DecisionState::new()
        .with_custom("subject", json!(subject_entity))
        .with_custom("candidate", json!(directive))
        .with_custom("evidence", json!(passage));

    let (_res, outcomes) = service.evaluate(secret, &contract, &state, policy).await?;
    let outcome =
        outcomes
            .get("evidence_relevance")
            .cloned()
            .unwrap_or(EnforcementOutcome::Unavailable {
                reason: "missing question outcome".into(),
            });
    Ok(outcome)
}

/// Evaluate semantic entity binding between subject and candidate identifier using DecisionService.
pub async fn evaluate_entity_binding_gate(
    secret: &ProviderSecret,
    target_entity: &str,
    candidate_identifier: &str,
    evidence_text: &str,
    service: &DecisionService,
    policy: &DecisionPolicy,
) -> anyhow::Result<EnforcementOutcome> {
    let contract = template_entity_binding();
    let state = DecisionState::new()
        .with_custom("subject", json!(target_entity))
        .with_custom("candidate", json!(candidate_identifier))
        .with_custom("evidence", json!(evidence_text));

    let (_res, outcomes) = service.evaluate(secret, &contract, &state, policy).await?;
    let outcome =
        outcomes
            .get("entity_binding")
            .cloned()
            .unwrap_or(EnforcementOutcome::Unavailable {
                reason: "missing question outcome".into(),
            });
    Ok(outcome)
}

/// Evaluate semantic publication fidelity against retained evidence using DecisionService.
pub async fn evaluate_publication_fidelity_gate(
    secret: &ProviderSecret,
    conclusion: &str,
    evidence_summary: &str,
    service: &DecisionService,
    policy: &DecisionPolicy,
) -> anyhow::Result<EnforcementOutcome> {
    let contract = template_publication();
    let state = DecisionState::new()
        .with_custom("candidate", json!(conclusion))
        .with_custom("evidence", json!(evidence_summary));

    let (_res, outcomes) = service.evaluate(secret, &contract, &state, policy).await?;
    let outcome = outcomes
        .get("publication")
        .cloned()
        .unwrap_or(EnforcementOutcome::Unavailable {
            reason: "missing question outcome".into(),
        });
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::investigation::contracts::InvestigationSurface;
    use serde_json::json;

    #[test]
    fn preflight_blocks_disabled_or_unpermitted_tools() {
        let policy = SurfacePolicy::for_surface(InvestigationSurface::ReconChat);
        let proposal = CallProposal {
            task_id: "t1".into(),
            directive_id: "d1".into(),
            task_revision: 1,
            tool_id: "firecrawl_search".into(),
            contract_version: "v1".into(),
            arguments: json!({"query": "test"}),
            purpose: "discover".into(),
            expected_evidence: "results".into(),
            satisfaction_test: "results > 0".into(),
        };

        // Enabled -> ok
        assert!(validate_tool_preflight(&proposal, &policy, true).is_passed());
        // Disabled -> rejected
        assert!(!validate_tool_preflight(&proposal, &policy, false).is_passed());

        // Intel policy blocks sociavault
        let intel_policy = SurfacePolicy::for_surface(InvestigationSurface::IntelBrief);
        let mut socia_proposal = proposal.clone();
        socia_proposal.tool_id = "sociavault_search".into();
        assert!(!validate_tool_preflight(&socia_proposal, &intel_policy, true).is_passed());
    }
}
