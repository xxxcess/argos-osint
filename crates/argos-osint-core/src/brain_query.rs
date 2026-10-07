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

pub fn admit_memory(query: &BrainQuery, memory: &Memory, policy: &AdmissionPolicy) -> bool {
    let memory_text = memory.text.to_ascii_lowercase();
    
    if policy.strict_mode {
        if !query.confirmed_subjects.is_empty() {
            let mut match_found = false;
            for subject in &query.confirmed_subjects {
                if memory_text.contains(&subject.to_ascii_lowercase()) {
                    match_found = true;
                    break;
                }
            }
            if !match_found {
                return false;
            }
        }
    }
    true
}

pub fn pack_context(_admitted: &[Memory], _budget: usize) -> String {
    String::new()
}
