//! Central six-domain Atlas taxonomy (`strategic-domains-v1`).
//!
//! This module is a dependency leaf: it must not import the classifier
//! engine, `nlp`, `storage`, or orchestration. Downstream consumers
//! (`classifier`, `atlas`, `related_articles`, `storage`, `intel_service`)
//! import the vocabulary from here so there is exactly one definition.
//!
//! # Semantics
//!
//! Domains are **attributes/facets of an article, not six permanent graph
//! nodes**. A domain assignment records that an article substantively
//! discusses that facet; it is never a source reliability grade, an actor
//! threat rating, an event probability, or proof of a relationship between
//! actors or events.
//!
//! # Legacy taxonomy
//!
//! The previous seven-label taxonomy (`atlas-taxonomy-v2`, including the
//! `unk` rejection marker) stays readable but is never mapped blindly onto
//! the new domains: old labels identify legacy rows so six-domain filters
//! can exclude them explicitly and report coverage. Note that `economic`
//! exists in **both** taxonomies — label membership alone therefore cannot
//! decide whether a row is legacy; the recorded taxonomy version decides.

use serde::{Deserialize, Serialize};

/// Version of the six-domain strategic taxonomy. Receipts, artifacts and
/// filters record this version; changing it is a compatibility break.
pub const TAXONOMY_VERSION: &str = "strategic-domains-v1";

/// Canonical wire IDs from the same ordered domain leaf used by policies.
pub const DOMAIN_IDS: [&str; 6] = [
    DomainId::ALL[0].as_str(),
    DomainId::ALL[1].as_str(),
    DomainId::ALL[2].as_str(),
    DomainId::ALL[3].as_str(),
    DomainId::ALL[4].as_str(),
    DomainId::ALL[5].as_str(),
];

/// Version of the previous seven-label admission taxonomy. Historical rows
/// recorded under this version are identified, never silently migrated.
pub const LEGACY_TAXONOMY_VERSION: &str = "atlas-taxonomy-v2";

/// A strategic domain: one of six article facets, not a graph node and not a
/// reliability, threat or probability judgement.
///
/// Ordering matches [`DomainId::ALL`]; serialized values are the canonical
/// lower-case ids from [`DomainId::as_str`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum DomainId {
    Governance,
    Security,
    Economic,
    Human,
    Environmental,
    Scientific,
}

impl DomainId {
    /// All six domains in canonical order. Receipt scores, per-domain
    /// thresholds and model arrays use this exact order.
    pub const ALL: [DomainId; 6] = [
        DomainId::Governance,
        DomainId::Security,
        DomainId::Economic,
        DomainId::Human,
        DomainId::Environmental,
        DomainId::Scientific,
    ];

    /// Canonical lower-case id: one of `governance`, `security`, `economic`,
    /// `human`, `environmental`, `scientific`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            DomainId::Governance => "governance",
            DomainId::Security => "security",
            DomainId::Economic => "economic",
            DomainId::Human => "human",
            DomainId::Environmental => "environmental",
            DomainId::Scientific => "scientific",
        }
    }

    /// Official category name, as agreed in the implementation spec.
    #[must_use]
    pub fn display_name(self) -> &'static str {
        match self {
            DomainId::Governance => "Governance & Political Stability",
            DomainId::Security => "Security & Threat Environment",
            DomainId::Economic => "Economic & Supply Chain Resilience",
            DomainId::Human => "Human Security & Public Welfare",
            DomainId::Environmental => "Environmental & Resource Security",
            DomainId::Scientific => "Scientific & Technological Capability",
        }
    }

    /// Versioned domain scope used to guide evidence selection, never to invent facts.
    #[must_use]
    pub const fn definition(self) -> &'static str {
        match self {
            Self::Governance => "Institutions, elections, authority and diplomacy.",
            Self::Security => "Armed threats, crime, cyber incidents and protective actions.",
            Self::Economic => "Finance, trade, production, logistics and supply resilience.",
            Self::Human => "Health, displacement, food access and essential services.",
            Self::Environmental => "Hazards, pollution, water, ecosystems and natural resources.",
            Self::Scientific => {
                "Research, technical performance and scientific or technological capability."
            }
        }
    }

    /// Parses an exact, case-sensitive lower-case id. There is no alias or
    /// normalization layer on purpose: stored values must round-trip.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|domain| domain.as_str() == value)
    }

    /// The domain at `index` in [`DomainId::ALL`] order, if in range.
    #[must_use]
    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }
}

/// The seven legacy admission labels plus the legacy `unk` rejection marker,
/// exact case-sensitive spellings from `atlas-taxonomy-v2`.
///
/// `economic` appears here **and** in the current taxonomy, so membership in
/// this set alone cannot identify a legacy row; the row's recorded taxonomy
/// version decides. Legacy labels are never mapped onto new domains.
///
/// For exclusion/identification only: mark rows as legacy so six-domain
/// filters exclude them explicitly and can report coverage.
pub fn legacy_label_set() -> &'static [&'static str] {
    &[
        "geopolitical",
        "economic",
        "military",
        "science",
        "health",
        "stability",
        "technology",
        "unk",
    ]
}

/// Whether `value` is a legacy `atlas-taxonomy-v2` category (including the
/// `unk` rejection marker). Exact, case-sensitive match. Identification
/// only — never a mapping hint.
#[must_use]
pub fn is_legacy_category(value: &str) -> bool {
    legacy_label_set().contains(&value)
}

/// Serializable description of the taxonomy for the CLI `--json` status and
/// the artifact manifest: version, the six canonical ids with official
/// names, and the legacy version/labels kept for identification.
#[must_use]
pub fn taxonomy_manifest() -> serde_json::Value {
    let domains: Vec<serde_json::Value> = DomainId::ALL
        .iter()
        .map(|domain| {
            serde_json::json!({
                "id": domain.as_str(),
                "name": domain.display_name(),
            })
        })
        .collect();
    serde_json::json!({
        "taxonomy_version": TAXONOMY_VERSION,
        "domains": domains,
        "legacy_taxonomy_version": LEGACY_TAXONOMY_VERSION,
        "legacy_labels": legacy_label_set(),
    })
}
