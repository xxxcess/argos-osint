//! Logical model roles, prompts, and inheritance profiles.

use serde::{Deserialize, Serialize};

use crate::provider::{ModelAssignment, RoleDefaults};

/// The six current pipeline roles and retained legacy serialized variants.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LogicalRole {
    ReconAgent,
    ReconDecisions,
    ReconAnalyst,
    AtlasEvaluator,
    AtlasAnalyst,
    BrainSummary,
    // Legacy serialized variants remain readable; new callers use ALL below.
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
    pub const ALL: [LogicalRole; 6] = [
        Self::ReconAgent,
        Self::ReconDecisions,
        Self::ReconAnalyst,
        Self::AtlasEvaluator,
        Self::AtlasAnalyst,
        Self::BrainSummary,
    ];

    pub fn key(&self) -> &'static str {
        match self {
            Self::ReconAgent => "recon_agent",
            Self::ReconDecisions => "recon_decisions",
            Self::ReconAnalyst => "recon_analyst",
            Self::AtlasEvaluator => "atlas_evaluator",
            Self::AtlasAnalyst => "atlas_analyst",
            Self::BrainSummary => "brain_summary",
            Self::Classifier => "classifier",
            Self::Planner => "recon",
            Self::ToolPicker => "tool_picker",
            Self::EvidenceCurator => "evidence_curator",
            Self::EntityResolver => "entity_resolver",
            Self::ClaimAssessor => "claim_assessor",
            Self::Controller => "recon_decisions",
            Self::Synthesis => "synthesis",
            Self::Summarization => "summarization",
        }
    }

    pub fn title(&self) -> &'static str {
        match self {
            Self::ReconAgent => "Recon Agent",
            Self::ReconDecisions => "Recon Decisions",
            Self::ReconAnalyst => "Recon Analyst",
            Self::AtlasEvaluator => "Atlas Evaluator",
            Self::AtlasAnalyst => "Atlas Analyst",
            Self::BrainSummary => "Brain Summary",
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
            Self::ReconAgent => "Recon directives, grounded queries and exceptional extraction",
            Self::ReconDecisions => "Bounded Recon tool selection and assessment gates",
            Self::ReconAnalyst => "Recon findings, inferences and reports",
            Self::AtlasEvaluator => "Bounded Atlas admission and support decisions",
            Self::AtlasAnalyst => "Atlas claims, inferences, related leads and prose",
            Self::BrainSummary => "Shared readable knowledge summaries; never primary evidence",
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_nine_roles_have_valid_keys_and_purposes() {
        assert_eq!(LogicalRole::ALL.len(), 6);
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
