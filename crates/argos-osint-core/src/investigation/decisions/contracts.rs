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
    /// Ordinal scale: 2–10 ordered `levels` (zero-based on the wire) mapped
    /// linearly onto the application scale `min..=max`.
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
    /// Score only: ordered level descriptions, worst/lowest first (2–10).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub levels: Vec<String>,
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
            levels: Vec::new(),
        }
    }

    pub fn noul(id: impl Into<String>, instructions: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            question_type: DecisionQuestionType::Noul,
            instructions: instructions.into(),
            criteria: BTreeMap::new(),
            abstention_label: "insufficient".into(),
            levels: Vec::new(),
        }
    }

    /// Noul with both sides described (Jev `criteria: {true, false}`).
    pub fn noul_with(
        id: impl Into<String>,
        instructions: impl Into<String>,
        when_true: impl Into<String>,
        when_false: impl Into<String>,
    ) -> Self {
        let mut q = Self::noul(id, instructions);
        q.criteria.insert("true".into(), when_true.into());
        q.criteria.insert("false".into(), when_false.into());
        q
    }

    /// Score over ordered `levels` (lowest first) mapped onto `min..=max`.
    pub fn score<V: Into<String>>(
        id: impl Into<String>,
        instructions: impl Into<String>,
        min: i64,
        max: i64,
        levels: impl IntoIterator<Item = V>,
    ) -> Self {
        Self {
            id: id.into(),
            question_type: DecisionQuestionType::Score { min, max },
            instructions: instructions.into(),
            criteria: BTreeMap::new(),
            abstention_label: "insufficient".into(),
            levels: levels.into_iter().map(Into::into).collect(),
        }
    }

    /// Translate a zero-based (possibly fractional) level onto the
    /// application scale. `None` for non-Score questions.
    pub fn scale_value(&self, level: f64) -> Option<f64> {
        let DecisionQuestionType::Score { min, max } = self.question_type else {
            return None;
        };
        let top = self.levels.len().saturating_sub(1).max(1) as f64;
        Some(min as f64 + level * (max - min) as f64 / top)
    }

    /// Structural validation; a malformed contract never reaches the network.
    pub fn validate(&self) -> Result<(), String> {
        if self.instructions.trim().is_empty() {
            return Err(format!("question `{}` has no instructions", self.id));
        }
        match &self.question_type {
            DecisionQuestionType::Choice => {
                if self.criteria.len() < 2 || self.criteria.len() > 255 {
                    return Err(format!(
                        "choice `{}` needs 2–255 options, got {}",
                        self.id,
                        self.criteria.len()
                    ));
                }
            }
            DecisionQuestionType::Noul => {
                if !self.criteria.is_empty()
                    && self.criteria.keys().any(|k| k != "true" && k != "false")
                {
                    return Err(format!("noul `{}` criteria must be true/false", self.id));
                }
            }
            DecisionQuestionType::Score { min, max } => {
                if !(2..=10).contains(&self.levels.len()) {
                    return Err(format!(
                        "score `{}` needs 2–10 ordered levels, got {}",
                        self.id,
                        self.levels.len()
                    ));
                }
                if min >= max {
                    return Err(format!("score `{}` scale {min}..={max} is empty", self.id));
                }
                if self.levels.iter().any(|l| l.trim().is_empty()) {
                    return Err(format!("score `{}` has an empty level", self.id));
                }
            }
        }
        Ok(())
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

    /// Validate every question before any dispatch.
    pub fn validate(&self) -> Result<(), String> {
        if self.questions.is_empty() {
            return Err("decision contract has no questions".into());
        }
        for (id, q) in &self.questions {
            if id != &q.id {
                return Err(format!("question key `{id}` does not match id `{}`", q.id));
            }
            q.validate()?;
        }
        Ok(())
    }
}

/// Normalized single answer returned from either Jev or a general model.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct NormalizedAnswer {
    pub label: String,
    /// Legacy rounded application-scale score (display only).
    pub score: Option<i64>,
    /// Raw probability-weighted application-scale value (Score).
    #[serde(default)]
    pub score_value: Option<f64>,
    /// Native P(yes) for Noul.
    pub noul: Option<f64>,
    /// Native winning probability (Choice: P(choice); Noul: p or 1−p;
    /// Score: P(nearest level)). Never substituted by confidence.
    pub probability: Option<f64>,
    /// Native distribution-shape confidence, kept separately.
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
