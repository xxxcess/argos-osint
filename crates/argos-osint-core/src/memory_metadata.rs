use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    AtomicClaim,
    CycleBrief,
    InvestigationDigest,
    SourceSummary,
    ManualNote,
    UserProfile,
}

impl std::fmt::Display for MemoryKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MemoryKind::AtomicClaim => write!(f, "atomic_claim"),
            MemoryKind::CycleBrief => write!(f, "cycle_brief"),
            MemoryKind::InvestigationDigest => write!(f, "investigation_digest"),
            MemoryKind::SourceSummary => write!(f, "source_summary"),
            MemoryKind::ManualNote => write!(f, "manual_note"),
            MemoryKind::UserProfile => write!(f, "user_profile"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssessmentState {
    Supports,
    Contradicts,
    Mentions,
    Insufficient,
    Unassessed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    pub source_id: String,
    pub state: ProvenanceState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProvenanceState {
    Valid,
    Missing,
    Unavailable,
}
