//! Canonical Brain memory kinds and typed payload contract (spec §4).
//!
//! The five core kinds — Actor, Claim, Inference, Link, Related — carry
//! discriminated payloads; legacy/manual/profile/summary kinds are preserved.
//! `atomic_claim` (schema ≤36) decodes as [`MemoryKind::Claim`].

use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MemoryKind {
    Actor,
    Claim,
    Inference,
    Link,
    Related,
    CycleBrief,
    InvestigationDigest,
    SourceSummary,
    ManualNote,
    UserProfile,
}

impl MemoryKind {
    /// The five canonical typed kinds.
    pub const CORE: [MemoryKind; 5] = [
        MemoryKind::Actor,
        MemoryKind::Claim,
        MemoryKind::Inference,
        MemoryKind::Link,
        MemoryKind::Related,
    ];

    /// Every kind, canonical and legacy-derived.
    pub const ALL: [MemoryKind; 10] = [
        MemoryKind::Actor,
        MemoryKind::Claim,
        MemoryKind::Inference,
        MemoryKind::Link,
        MemoryKind::Related,
        MemoryKind::CycleBrief,
        MemoryKind::InvestigationDigest,
        MemoryKind::SourceSummary,
        MemoryKind::ManualNote,
        MemoryKind::UserProfile,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            MemoryKind::Actor => "actor",
            MemoryKind::Claim => "claim",
            MemoryKind::Inference => "inference",
            MemoryKind::Link => "link",
            MemoryKind::Related => "related",
            MemoryKind::CycleBrief => "cycle_brief",
            MemoryKind::InvestigationDigest => "investigation_digest",
            MemoryKind::SourceSummary => "source_summary",
            MemoryKind::ManualNote => "manual_note",
            MemoryKind::UserProfile => "user_profile",
        }
    }

    /// Decode a stored kind, accepting legacy spellings.
    pub fn parse(raw: &str) -> Option<Self> {
        Some(match raw.trim() {
            "actor" => MemoryKind::Actor,
            "claim" | "atomic_claim" => MemoryKind::Claim,
            "inference" => MemoryKind::Inference,
            "link" => MemoryKind::Link,
            "related" => MemoryKind::Related,
            "cycle_brief" => MemoryKind::CycleBrief,
            "investigation_digest" => MemoryKind::InvestigationDigest,
            "source_summary" => MemoryKind::SourceSummary,
            "manual_note" => MemoryKind::ManualNote,
            "user_profile" => MemoryKind::UserProfile,
            _ => return None,
        })
    }

    pub fn is_core(self) -> bool {
        Self::CORE.contains(&self)
    }

    /// Stored spellings that decode to this kind (for SQL prefilters).
    pub fn stored_spellings(self) -> &'static [&'static str] {
        match self {
            MemoryKind::Claim => &["claim", "atomic_claim"],
            other => match other {
                MemoryKind::Actor => &["actor"],
                MemoryKind::Inference => &["inference"],
                MemoryKind::Link => &["link"],
                MemoryKind::Related => &["related"],
                MemoryKind::CycleBrief => &["cycle_brief"],
                MemoryKind::InvestigationDigest => &["investigation_digest"],
                MemoryKind::SourceSummary => &["source_summary"],
                MemoryKind::ManualNote => &["manual_note"],
                MemoryKind::UserProfile => &["user_profile"],
                MemoryKind::Claim => unreachable!(),
            },
        }
    }

    /// Kind for an insight classification receipt (`fact`/`claim`/`inference`).
    pub fn from_classification(classification: &str) -> Self {
        if classification.trim().eq_ignore_ascii_case("inference") {
            MemoryKind::Inference
        } else {
            MemoryKind::Claim
        }
    }
}

impl std::fmt::Display for MemoryKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for MemoryKind {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for MemoryKind {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(d)?;
        MemoryKind::parse(&raw)
            .ok_or_else(|| serde::de::Error::custom(format!("unknown memory kind `{raw}`")))
    }
}

/// Current typed payload schema version.
pub const PAYLOAD_SCHEMA_VERSION: u32 = 1;

/// Memory lifecycle (separate from category, origin app and assessment).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lifecycle {
    #[default]
    Active,
    Superseded,
    Redirected,
    Retired,
}

/// Actor subtype (initial set).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorSubtype {
    Person,
    Organization,
    Place,
    Account,
    Domain,
    Ip,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdentityState {
    #[default]
    Provisional,
    Ambiguous,
    Resolved,
}

impl IdentityState {
    pub fn as_str(self) -> &'static str {
        match self {
            IdentityState::Provisional => "provisional",
            IdentityState::Ambiguous => "ambiguous",
            IdentityState::Resolved => "resolved",
        }
    }
}

/// Endpoint of a Link/Related memory: a canonical Brain memory, or an exact
/// Article/Source record (articles remain source records, not memories).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EndpointRef {
    Memory {
        memory_id: String,
        kind: MemoryKind,
    },
    Article {
        run_id: String,
        article_id: String,
        revision: String,
    },
    Source {
        origin: String,
        source_id: String,
        revision: String,
    },
}

impl EndpointRef {
    /// `(endpoint_kind, ref_id, origin, revision)` for the endpoint index.
    pub fn index_key(&self) -> (&'static str, String, String, String) {
        match self {
            EndpointRef::Memory { memory_id, kind } => (
                "memory",
                memory_id.clone(),
                kind.as_str().into(),
                String::new(),
            ),
            EndpointRef::Article {
                run_id,
                article_id,
                revision,
            } => (
                "article",
                article_id.clone(),
                run_id.clone(),
                revision.clone(),
            ),
            EndpointRef::Source {
                origin,
                source_id,
                revision,
            } => (
                "source",
                source_id.clone(),
                origin.clone(),
                revision.clone(),
            ),
        }
    }
}

/// Actor reference from another memory's payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActorRef {
    pub actor_id: String,
    /// e.g. `subject`, `object`, `mentioned`, `relevant`.
    pub role: String,
    #[serde(default)]
    pub binding_state: IdentityState,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActorPayload {
    pub subtype: ActorSubtype,
    pub display_name: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub identifiers: Vec<String>,
    #[serde(default)]
    pub identity_state: IdentityState,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ClaimPayload {
    #[serde(default)]
    pub actors: Vec<ActorRef>,
    #[serde(default)]
    pub predicate: String,
    #[serde(default)]
    pub object: String,
    #[serde(default)]
    pub modality: String,
    #[serde(default)]
    pub passages: Vec<String>,
    /// Extraction fidelity / support assessment (not real-world verification).
    #[serde(default)]
    pub assessment: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct InferencePayload {
    #[serde(default)]
    pub premise_claims: Vec<String>,
    #[serde(default)]
    pub actors: Vec<ActorRef>,
    #[serde(default)]
    pub reasoning: String,
    #[serde(default)]
    pub limitations: String,
    #[serde(default)]
    pub support_state: String,
    #[serde(default)]
    pub coverage_receipt: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LinkPayload {
    pub source: EndpointRef,
    pub target: EndpointRef,
    /// e.g. `mentioned_in`, `supports`, `contradicts`, `derived_from`.
    pub relation: String,
    #[serde(default)]
    pub rationale: String,
    #[serde(default)]
    pub receipts: Vec<String>,
    /// Original Related record when promoted from a tentative lead.
    #[serde(default)]
    pub promoted_from: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RelatedPayload {
    pub source: EndpointRef,
    pub target: EndpointRef,
    #[serde(default)]
    pub shared_actors: Vec<String>,
    /// Explicit basis when no shared Actor exists (candidate name/similarity).
    #[serde(default)]
    pub basis: String,
    #[serde(default)]
    pub interpretation: String,
    #[serde(default)]
    pub verification_gap: String,
    #[serde(default)]
    pub source_revisions: Vec<String>,
}

/// Discriminated payload of a core memory.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
pub enum MemoryPayload {
    Actor(ActorPayload),
    Claim(ClaimPayload),
    Inference(InferencePayload),
    Link(LinkPayload),
    Related(RelatedPayload),
}

impl MemoryPayload {
    pub fn kind(&self) -> MemoryKind {
        match self {
            MemoryPayload::Actor(_) => MemoryKind::Actor,
            MemoryPayload::Claim(_) => MemoryKind::Claim,
            MemoryPayload::Inference(_) => MemoryKind::Inference,
            MemoryPayload::Link(_) => MemoryKind::Link,
            MemoryPayload::Related(_) => MemoryKind::Related,
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

#[cfg(test)]
#[path = "memory_metadata_tests.rs"]
mod tests;
