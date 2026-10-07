//! Adapter-neutral decision contracts and normalized result types.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use super::adapters::DecisionAdapterKind;

/// Type of a decision question.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionQuestionType {
    /// Finite choice among discrete criteria outcomes.
    Choice,
    /// Strictly binary proposition (yes/no).
    Noul,
    /// Bounded integer rating on an ordinal scale.
    Score { min: i64, max: i64 },
}

/// Specification for a single question inside a decision contract.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct QuestionSpec {
    pub id: String,
    pub question_type: DecisionQuestionType,
    pub instructions: String,
    /// Criteria mapping candidate/outcome labels to descriptive definitions.
    pub criteria: BTreeMap<String, String>,
    /// Explicit label used when evidence is incomplete or ambiguous.
    pub abstention_label: String,
}

impl QuestionSpec {
    pub fn choice<K: Into<String>, V: Into<String>>(
        id: impl Into<String>,
        instructions: impl Into<String>,
        criteria: impl IntoIterator<Item = (K, V)>,
        abstention: impl Into<String>,
    ) -> Self {
        let mut map = BTreeMap::new();
        for (k, v) in criteria {
            map.insert(k.into(), v.into());
        }
        Self {
            id: id.into(),
            question_type: DecisionQuestionType::Choice,
            instructions: instructions.into(),
            criteria: map,
            abstention_label: abstention.into(),
        }
    }

    pub fn noul(id: impl Into<String>, instructions: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            question_type: DecisionQuestionType::Noul,
            instructions: instructions.into(),
            criteria: BTreeMap::new(),
            abstention_label: "insufficient".into(),
        }
    }

    pub fn score<V: Into<String>>(
        id: impl Into<String>,
        instructions: impl Into<String>,
        min: i64,
        max: i64,
        criteria: impl IntoIterator<Item = (i64, V)>,
    ) -> Self {
        let mut map = BTreeMap::new();
        for (k, v) in criteria {
            map.insert(k.to_string(), v.into());
        }
        Self {
            id: id.into(),
            question_type: DecisionQuestionType::Score { min, max },
            instructions: instructions.into(),
            criteria: map,
            abstention_label: "insufficient".into(),
        }
    }
}

/// Canonical decision contract compiled across both Jev-native and general-model adapters.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DecisionContract {
    pub role_id: String,
    pub template_name: String,
    pub template_version: String,
    pub required_state_fields: Vec<String>,
    pub questions: BTreeMap<String, QuestionSpec>,
}

impl DecisionContract {
    pub fn new(
        role_id: impl Into<String>,
        template_name: impl Into<String>,
        template_version: impl Into<String>,
        required_state_fields: impl IntoIterator<Item = &'static str>,
        questions: impl IntoIterator<Item = QuestionSpec>,
    ) -> Self {
        let mut q_map = BTreeMap::new();
        for q in questions {
            q_map.insert(q.id.clone(), q);
        }
        Self {
            role_id: role_id.into(),
            template_name: template_name.into(),
            template_version: template_version.into(),
            required_state_fields: required_state_fields
                .into_iter()
                .map(str::to_string)
                .collect(),
            questions: q_map,
        }
    }
}

/// Normalized single answer returned from either Jev or a general model.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct NormalizedAnswer {
    pub label: String,
    pub score: Option<i64>,
    pub noul: Option<f64>,
    pub probability: Option<f64>,
    pub confidence: Option<f64>,
    #[serde(default)]
    pub probabilities: BTreeMap<String, f64>,
}

/// Status of validation for a received model output against its contract.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DecisionValidationStatus {
    Valid,
    Rejected { reason: String },
}

/// Normalized result across adapters for an entire decision contract invocation.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct NormalizedDecisionResult {
    pub contract_role: String,
    pub template_name: String,
    pub template_version: String,
    pub model_used: String,
    pub adapter_used: DecisionAdapterKind,
    pub answers: BTreeMap<String, NormalizedAnswer>,
    pub cost: Option<f64>,
    pub latency_ms: u64,
    pub validation_status: DecisionValidationStatus,
}
