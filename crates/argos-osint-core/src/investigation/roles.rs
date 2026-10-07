//! Logical model roles, prompts, and inheritance profiles.

use serde::{Deserialize, Serialize};

use crate::provider::{ModelAssignment, RoleDefaults};

/// The 9 logical model roles specified in the unified investigation harness.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LogicalRole {
    Classifier,
    Planner,
    ToolPicker,
    EvidenceCurator,
    EntityResolver,
    ClaimAssessor,
    Controller,
    Synthesis,
    Summarization,
}

impl LogicalRole {
    pub const ALL: [LogicalRole; 9] = [
        LogicalRole::Classifier,
        LogicalRole::Planner,
        LogicalRole::ToolPicker,
        LogicalRole::EvidenceCurator,
        LogicalRole::EntityResolver,
        LogicalRole::ClaimAssessor,
        LogicalRole::Controller,
        LogicalRole::Synthesis,
        LogicalRole::Summarization,
    ];

    pub fn key(&self) -> &'static str {
        match self {
            Self::Classifier => "classifier",
            Self::Planner => "recon",
            Self::ToolPicker => "tool_picker",
            Self::EvidenceCurator => "evidence_curator",
            Self::EntityResolver => "entity_resolver",
            Self::ClaimAssessor => "claim_assessor",
            Self::Controller => "investigation_controller",
            Self::Synthesis => "synthesis",
            Self::Summarization => "summarization",
        }
    }

    pub fn title(&self) -> &'static str {
        match self {
            Self::Classifier => "Classifier",
            Self::Planner => "Planner (Recon)",
            Self::ToolPicker => "Tool Picker",
            Self::EvidenceCurator => "Evidence Curator",
            Self::EntityResolver => "Entity Resolver",
            Self::ClaimAssessor => "Claim Assessor",
            Self::Controller => "Investigation Controller",
            Self::Synthesis => "Synthesis",
            Self::Summarization => "Summarization",
        }
    }

    pub fn purpose(&self) -> &'static str {
        match self {
            Self::Classifier => "Mode, intent, subject types, ambiguity, freshness; semantic gates",
            Self::Planner => {
                "Directives, dependent tasks, evidence requirements, stopping conditions"
            }
            Self::ToolPicker => "Best eligible tool and grounded call proposal for one task",
            Self::EvidenceCurator => {
                "Relevant source-linked passages, observations, dates, candidate bindings"
            }
            Self::EntityResolver => "Confirmed/candidate/rejected/ambiguous identity bindings",
            Self::ClaimAssessor => {
                "Per-claim supported/disputed/unresolved assessment with cited passages"
            }
            Self::Controller => "Continue, redirect, defer or finish; gap and next-action decision",
            Self::Synthesis => "Incremental sections and final answer from assessed findings",
            Self::Summarization => {
                "Compact context preserving evidence IDs, contradictions, unfinished tasks"
            }
        }
    }

    pub fn default_inheritance(&self) -> Option<&'static str> {
        match self {
            Self::EvidenceCurator => Some("recon"),
            Self::EntityResolver => Some("classifier"),
            Self::ClaimAssessor => Some("synthesis"),
            Self::Controller => Some("recon"),
            Self::Summarization => Some("synthesis"),
            _ => None,
        }
    }

    pub fn resolve_assignment(
        &self,
        defaults: &RoleDefaults,
    ) -> (ModelAssignment, Option<&'static str>) {
        defaults.resolve_role(self.key())
    }
}

/// Prompt builder for bounded Controller decisions.
pub fn controller_prompt(
    objective: &str,
    directives_status: &str,
    assessed_claims: &str,
    unmet_gaps: &str,
) -> String {
    format!(
        "You are the Argos Investigation Controller.\n\
Objective: {objective}\n\
Directive Status:\n{directives_status}\n\
Assessed Findings:\n{assessed_claims}\n\
Identified Gaps:\n{unmet_gaps}\n\n\
Decide the next action:\n\
1. Continue with next task\n\
2. Material scope redirect needed\n\
3. Ready for synthesis\n\
4. Defer / terminate due to insufficient sources\n\n\
Respond with concise JSON: {{\"action\": \"continue\"|\"redirect\"|\"synthesize\"|\"finish\", \"reason\": \"...\"}}"
    )
}

/// Prompt builder for Evidence Curation.
pub fn evidence_curation_prompt(subject: &str, raw_observation: &str) -> String {
    format!(
        "Extract relevant factual passages regarding '{subject}' from the following observation.\n\
Observation:\n{raw_observation}\n\n\
Return JSON array of passages with source URL, excerpt, and observed date."
    )
}

/// Prompt builder for Claim Assessment.
pub fn claim_assessment_prompt(claim: &str, passages: &str) -> String {
    format!(
        "Evaluate the following claim strictly based on the cited evidence passages.\n\
Claim: {claim}\n\n\
Passages:\n{passages}\n\n\
Determine if the claim is supported, disputed, or unresolved. Return JSON: {{\"stance\": \"supported\"|\"disputed\"|\"unresolved\", \"rationale\": \"...\", \"passage_ids\": [...]}}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_nine_roles_have_valid_keys_and_purposes() {
        assert_eq!(LogicalRole::ALL.len(), 9);
        for role in LogicalRole::ALL {
            assert!(!role.key().is_empty());
            assert!(!role.title().is_empty());
            assert!(!role.purpose().is_empty());
        }
    }

    #[test]
    fn inheritance_defaults_match_spec() {
        assert_eq!(
            LogicalRole::EvidenceCurator.default_inheritance(),
            Some("recon")
        );
        assert_eq!(
            LogicalRole::EntityResolver.default_inheritance(),
            Some("classifier")
        );
        assert_eq!(
            LogicalRole::ClaimAssessor.default_inheritance(),
            Some("synthesis")
        );
        assert_eq!(LogicalRole::Controller.default_inheritance(), Some("recon"));
        assert_eq!(
            LogicalRole::Summarization.default_inheritance(),
            Some("synthesis")
        );
        assert_eq!(LogicalRole::Planner.default_inheritance(), None);
    }
}
