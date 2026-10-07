//! Bounded decision state builder with untrusted evidence isolation.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// Bounded snapshot of task details.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct TaskStateSummary {
    pub id: String,
    pub revision: u32,
    pub objective: String,
    pub required_evidence: String,
    pub completion_criteria: String,
}

/// Bounded snapshot of subject bindings and known ambiguities.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct SubjectStateSummary {
    pub confirmed_bindings: BTreeMap<String, String>,
    pub known_ambiguities: Vec<String>,
}

/// Single original evidence passage reference.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct EvidencePassageState {
    pub source_id: String,
    pub url: String,
    pub domain: String,
    pub passage_text: String,
    pub qualification: Option<String>,
}

/// Complete bounded decision state passed to compilers.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct DecisionState {
    pub task: Option<TaskStateSummary>,
    pub subject: Option<SubjectStateSummary>,
    pub candidate: Option<Value>,
    pub evidence: Vec<EvidencePassageState>,
    pub computed_checks: BTreeMap<String, Value>,
    pub missing_context: Vec<String>,
    pub custom: BTreeMap<String, Value>,
}

impl DecisionState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_task(
        mut self,
        id: impl Into<String>,
        revision: u32,
        objective: impl Into<String>,
        required_evidence: impl Into<String>,
        completion_criteria: impl Into<String>,
    ) -> Self {
        self.task = Some(TaskStateSummary {
            id: id.into(),
            revision,
            objective: objective.into(),
            required_evidence: required_evidence.into(),
            completion_criteria: completion_criteria.into(),
        });
        self
    }

    pub fn with_subject(
        mut self,
        bindings: impl IntoIterator<Item = (String, String)>,
        ambiguities: impl IntoIterator<Item = String>,
    ) -> Self {
        let mut confirmed = BTreeMap::new();
        for (k, v) in bindings {
            confirmed.insert(k, v);
        }
        self.subject = Some(SubjectStateSummary {
            confirmed_bindings: confirmed,
            known_ambiguities: ambiguities.into_iter().collect(),
        });
        self
    }

    pub fn with_candidate(mut self, candidate: Value) -> Self {
        self.candidate = Some(candidate);
        self
    }

    pub fn with_evidence_passage(
        mut self,
        source_id: impl Into<String>,
        url: impl Into<String>,
        domain: impl Into<String>,
        passage_text: impl Into<String>,
        qualification: Option<String>,
    ) -> Self {
        self.evidence.push(EvidencePassageState {
            source_id: source_id.into(),
            url: url.into(),
            domain: domain.into(),
            passage_text: passage_text.into(),
            qualification,
        });
        self
    }

    pub fn with_computed_check(mut self, key: impl Into<String>, value: Value) -> Self {
        self.computed_checks.insert(key.into(), value);
        self
    }

    pub fn with_missing_context(mut self, note: impl Into<String>) -> Self {
        self.missing_context.push(note.into());
        self
    }

    pub fn with_custom(mut self, key: impl Into<String>, value: Value) -> Self {
        self.custom.insert(key.into(), value);
        self
    }

    /// Serialize into a clean JSON value for compilation into Jev native state or general model prompt.
    pub fn to_value(&self) -> Value {
        let mut map = serde_json::Map::new();

        if let Some(task) = &self.task {
            map.insert("task".into(), json!(task));
        }

        if let Some(subject) = &self.subject {
            map.insert("subject".into(), json!(subject));
        }

        if let Some(cand) = &self.candidate {
            map.insert("candidate".into(), cand.clone());
        }

        if !self.evidence.is_empty() {
            map.insert("evidence".into(), json!(self.evidence));
        }

        if !self.computed_checks.is_empty() {
            map.insert("computed_checks".into(), json!(self.computed_checks));
        }

        if !self.missing_context.is_empty() {
            map.insert("missing_context".into(), json!(self.missing_context));
        }

        for (k, v) in &self.custom {
            map.insert(k.clone(), v.clone());
        }

        Value::Object(map)
    }
}
