//! Brain actor registry and typed relationship writer (spec §4).
//!
//! One identity authority for every app: Atlas, Intel, Recon, claims,
//! inferences, graphs and Related browsing bind through [`bind`]. Names are
//! search keys, never identities. An Actor is reused only through
//! (1) an existing scoped external reference, (2) a trusted stable identifier
//! (account handle, domain, IP …) with exactly one live owner, or (3) an
//! assessed merge (redirect). Otherwise a deterministic **source-scoped
//! provisional** Actor is created/reused and same-name actors elsewhere are
//! returned as labelled *candidates*, never merged. Reads follow acyclic
//! redirects; merge receipts preserve old IDs, source bindings and history.
//!
//! Actor, Link and Related memories are written by one transactional writer
//! with deterministic IDs, so repeated sightings and restarts never duplicate
//! identities or relationships. Endpoint existence/type/scope and relationship
//! vocabulary are validated before commit.

use serde_json::Value;

use crate::memory_metadata::{ActorSubtype, EndpointRef, IdentityState, MemoryKind, MemoryPayload};

/// Identity/index contract version (folded into recall-cache and Links
/// dependency identity).
pub const IDENTITY_INDEX_VERSION: &str = "brain-actors-v2-possessive";
pub const PAYLOAD_SCHEMA: i64 = 1;
/// Bounded materialized Related records per newly indexed article.
pub const RELATED_PER_ARTICLE: usize = 20;
/// Relationship vocabulary for assessed/observed Link memories.
pub const LINK_RELATIONS: &[&str] = &[
    "mentioned_in",
    "supports",
    "contradicts",
    "revises",
    "entity_connection",
    "derived_from",
];

/// Strip terminal English possessives, preserving internal apostrophes.
/// Original spellings and spans remain in source mention evidence.
pub fn normalize_name(text: &str) -> String {
    let name = text.trim().trim_matches(['"', '“', '”']);
    let lower = name.to_lowercase();
    let name = ["'s", "’s", "ʼs"]
        .iter()
        .find_map(|suffix| {
            lower
                .ends_with(suffix)
                .then(|| &name[..name.len() - suffix.len()])
        })
        .unwrap_or(name);
    name.trim_end_matches(['\'', '’', 'ʼ'])
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Candidate comparison key; equality across sources is not identity.
pub fn normalize_alias(text: &str) -> String {
    normalize_name(text)
        .chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_lowercase().next().unwrap_or(c)
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn hid(prefix: &str, parts: &[&str]) -> String {
    let h = crate::nlp::cache::hash(&parts.join("\u{1f}"));
    format!("{prefix}-{}", &h[..h.len().min(24)])
}

pub fn subtype_str(s: ActorSubtype) -> &'static str {
    match s {
        ActorSubtype::Person => "person",
        ActorSubtype::Organization => "organization",
        ActorSubtype::Place => "place",
        ActorSubtype::Account => "account",
        ActorSubtype::Domain => "domain",
        ActorSubtype::Ip => "ip",
        ActorSubtype::Unknown => "unknown",
    }
}

pub fn parse_subtype(raw: &str) -> ActorSubtype {
    match raw.trim().to_ascii_lowercase().as_str() {
        "person" | "people" | "per" => ActorSubtype::Person,
        "org" | "organization" | "organisation" | "company" => ActorSubtype::Organization,
        "place" | "location" | "gpe" | "loc" | "country" | "city" => ActorSubtype::Place,
        "account" | "username" | "handle" | "email" | "phone" => ActorSubtype::Account,
        "domain" | "website" | "url" => ActorSubtype::Domain,
        "ip" | "ip_address" | "ipaddress" => ActorSubtype::Ip,
        _ => ActorSubtype::Unknown,
    }
}

pub(crate) fn state_str(s: IdentityState) -> &'static str {
    match s {
        IdentityState::Provisional => "provisional",
        IdentityState::Ambiguous => "ambiguous",
        IdentityState::Resolved => "resolved",
    }
}

pub(crate) fn parse_state(raw: &str) -> IdentityState {
    match raw {
        "resolved" => IdentityState::Resolved,
        "ambiguous" => IdentityState::Ambiguous,
        _ => IdentityState::Provisional,
    }
}

/// Stable identifiers are trusted only for subtypes that are themselves
/// identifiers (an account handle, a domain, an IP).
fn trusted_identifier_subtype(s: ActorSubtype) -> bool {
    matches!(
        s,
        ActorSubtype::Account | ActorSubtype::Domain | ActorSubtype::Ip
    )
}

/// One bind/create request from any caller.
#[derive(Clone, Debug)]
pub struct BindRequest<'a> {
    pub name: &'a str,
    pub subtype: ActorSubtype,
    /// Deterministic source scope of a provisional actor, e.g.
    /// `atlas-article:<article_id>` or `recon-thread:<thread_id>`.
    pub scope: &'a str,
    /// Trusted stable identifiers (normalized; only honoured for identifier
    /// subtypes).
    pub identifiers: &'a [String],
    /// Scoped legacy/source-local ID mapped through `brain_actor_external_refs`.
    pub external: Option<(&'a str, &'a str)>,
    pub provenance: Value,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    /// Canonical (redirect-followed) Actor ID.
    pub actor_id: String,
    pub created: bool,
    pub state: IdentityState,
    /// How the actor was chosen: `external_ref`, `identifier`, `scoped`.
    pub basis: &'static str,
    /// Other live actors with the same normalized name/subtype. Candidates
    /// only — never "same actor" facts.
    pub candidates: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActorRow {
    pub actor_id: String,
    pub subtype: String,
    pub display_name: String,
    pub identity_state: IdentityState,
    pub scope_key: String,
    pub revision: i64,
}

pub(crate) fn text_of(payload: &MemoryPayload) -> String {
    match payload {
        MemoryPayload::Actor(a) => {
            format!("Actor: {} ({})", a.display_name, subtype_str(a.subtype))
        }
        MemoryPayload::Link(l) => format!(
            "Link: {} {} {}",
            l.source.index_key().1,
            l.relation.replace('_', " "),
            l.target.index_key().1
        ),
        MemoryPayload::Related(r) => format!(
            "Related (tentative): {} ↔ {} — {}",
            r.source.index_key().1,
            r.target.index_key().1,
            r.basis
        ),
        MemoryPayload::Claim(_) | MemoryPayload::Inference(_) => String::new(),
    }
}

pub(crate) fn endpoint_key(e: &EndpointRef) -> String {
    let (k, id, origin, rev) = e.index_key();
    format!("{k}:{origin}:{id}:{rev}")
}

fn actor_ids(e: &EndpointRef) -> Option<String> {
    match e {
        EndpointRef::Memory {
            memory_id,
            kind: MemoryKind::Actor,
        } => Some(memory_id.clone()),
        _ => None,
    }
}

#[cfg(test)]
#[path = "brain_actors_tests.rs"]
mod tests;
