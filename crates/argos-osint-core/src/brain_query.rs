use crate::brain::Memory;
use crate::memory_metadata::MemoryKind;

#[derive(Debug, Clone)]
pub struct BrainQuery {
    pub purpose: String,
    pub active_directive_ids: Vec<String>,
    pub current_question: String,
    pub confirmed_subjects: Vec<String>,
    pub requested_predicates: Vec<String>,
    pub allowed_kinds: Vec<MemoryKind>,
    pub candidate_limit: usize,
    pub token_budget: usize,
}

impl Default for BrainQuery {
    fn default() -> Self {
        Self {
            purpose: String::new(),
            active_directive_ids: Vec::new(),
            current_question: String::new(),
            confirmed_subjects: Vec::new(),
            requested_predicates: Vec::new(),
            allowed_kinds: Vec::new(),
            candidate_limit: 20,
            token_budget: 1500,
        }
    }
}

pub struct AdmissionPolicy {
    pub strict_mode: bool,
    pub allow_background: bool,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct InsightMetadata {
    pub insight_kind: String,
    pub assessment: crate::investigation::insights::Assessment,
    pub lifecycle: String,
    pub snapshot_id: String,
    pub revision: String,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct MemoryEvidence {
    pub memory_id: String,
    pub revision: String,
    pub kind: String,
    pub assessment: String,
    pub owner_kind: String,
    pub owner_id: String,
    #[serde(default)]
    pub entities: Vec<crate::nlp::EntityCandidate>,
    pub evidence: Vec<crate::investigation::insights::RetainedEvidence>,
}

pub fn admit_memory(query: &BrainQuery, memory: &Memory, policy: &AdmissionPolicy) -> bool {
    admit_text(query, &memory.text, policy)
}

/// [`admit_memory`] on a memory's text alone (the SurrealDB recall hits
/// carry no `Memory`).
pub fn admit_text(query: &BrainQuery, text: &str, policy: &AdmissionPolicy) -> bool {
    let memory_text: Vec<_> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(str::to_lowercase)
        .collect();

    if policy.strict_mode && !query.confirmed_subjects.is_empty() {
        let mut match_found = false;
        for subject in &query.confirmed_subjects {
            let subject: Vec<_> = subject
                .split(|c: char| !c.is_alphanumeric())
                .filter(|s| !s.is_empty())
                .map(str::to_lowercase)
                .collect();
            if !subject.is_empty() && memory_text.windows(subject.len()).any(|w| w == subject) {
                match_found = true;
                break;
            }
        }
        if !match_found {
            return false;
        }
    }
    true
}

pub fn pack_context(admitted: &[Memory], budget: usize) -> String {
    let mut out = String::new();
    for memory in admitted.iter().take(5) {
        let entry=serde_json::json!({"memory_id":memory.id,"revision":crate::store::memory_revision(&memory.text),"text":memory.text,"provenance":memory.source,"qualification":"lead; verify against original evidence"}).to_string();
        // Four bytes/token is an estimate; the serialized byte limit is enforced too.
        if out.len() + entry.len() + 1 > budget.saturating_mul(4).min(6000) {
            continue;
        }
        out.push_str(&entry);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {}
