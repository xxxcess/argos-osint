//! Feature-independent domain context and evidence presentation contracts.
//! @tag:intel-brief @tag:classifier @tag:brain-memory
//!
//! This module never queries storage or invokes a model. Context is captured
//! from an already validated admission receipt; labels cannot establish facts.

use std::fmt;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::atlas_taxonomy::{DomainId, TAXONOMY_VERSION};
use crate::investigation::insights::EvidenceRef;
use crate::memory_metadata::EndpointRef;

/// Immutable input policy included in brief and summary revisions.
pub const DOMAIN_CONTEXT_VERSION: &str = "domain-context-v2";
/// Canonical typed relationship validation contract.
pub const LINK_VALIDATION_VERSION: &str = "domain-link-validation-v1";
/// Hard summary packet ceiling, separate from topology exploration limits.
pub const MAX_SUMMARY_NODES: usize = 100;
/// Hard summary packet edge ceiling, respecting stricter caller budgets.
pub const MAX_SUMMARY_EDGES: usize = 200;
/// Maximum canonical actors captured for a single article context.
pub const MAX_CONTEXT_ACTORS: usize = 64;
/// Maximum verified aliases captured for one actor.
pub const MAX_CONTEXT_ALIASES: usize = 16;

/// Analytical meaning, independent from whether its premises are supported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum InferenceKind {
    Implication,
    Mechanism,
    ConditionalOutlook,
    CrossDomainImpact,
}
impl InferenceKind {
    /// Exact wire identifier; no legacy string translation is implicit.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Implication => "implication",
            Self::Mechanism => "mechanism",
            Self::ConditionalOutlook => "conditional_outlook",
            Self::CrossDomainImpact => "cross_domain_impact",
        }
    }
    /// Parses only the current contract vocabulary.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        [
            Self::Implication,
            Self::Mechanism,
            Self::ConditionalOutlook,
            Self::CrossDomainImpact,
        ]
        .into_iter()
        .find(|kind| kind.as_str() == value)
    }
}
/// Validates current inference attributes independently from extraction fidelity.
/// Domain relevance never supplies a premise, condition or proof of support.
pub fn validate_inference_attributes(
    kind: InferenceKind,
    domains: &[DomainId],
    support: &str,
    conditions: &[String],
    limitations: &[String],
) -> Result<(), ContractError> {
    if !matches!(support, "supported" | "conditional") {
        return Err(ContractError::InvalidField("inference.support"));
    }
    let unique: std::collections::BTreeSet<_> = domains.iter().collect();
    if unique.len() != domains.len() || domains.len() > DomainId::ALL.len() {
        return Err(ContractError::InvalidField("inference.domains"));
    }
    if conditions.len() > 4 || limitations.len() > 4 {
        return Err(ContractError::TooLarge("inference.qualifiers"));
    }
    for value in conditions.iter().chain(limitations) {
        valid_text(value, 1024, "inference.qualifier")?;
    }
    if kind == InferenceKind::CrossDomainImpact && domains.len() < 2 {
        return Err(ContractError::InvalidField("inference.cross_domain_impact"));
    }
    if (kind == InferenceKind::ConditionalOutlook || support == "conditional")
        && (support != "conditional" || conditions.is_empty() || limitations.is_empty())
    {
        return Err(ContractError::InvalidConditionalImpact);
    }
    Ok(())
}

/// Directed `source relation target`; precedes is explicitly noncausal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum LinkKind {
    Supports,
    Contradicts,
    Updates,
    Precedes,
    Contextualizes,
    PotentialImpact,
}
impl LinkKind {
    /// Current supported relationship vocabulary.
    pub const ALL: [Self; 6] = [
        Self::Supports,
        Self::Contradicts,
        Self::Updates,
        Self::Precedes,
        Self::Contextualizes,
        Self::PotentialImpact,
    ];
    /// Exact wire identifier.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Supports => "supports",
            Self::Contradicts => "contradicts",
            Self::Updates => "updates",
            Self::Precedes => "precedes",
            Self::Contextualizes => "contextualizes",
            Self::PotentialImpact => "potential_impact",
        }
    }
    /// Legacy relation names remain legacy rather than silently changing meaning.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == value)
    }
}
/// Eight presentation facets; these are not model domains or graph node kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum BriefFacet {
    CurrentState,
    SignificantDevelopments,
    ActorRelationships,
    CrossDomainImplications,
    EmergingSignals,
    ConditionalOutlook,
    Evidence,
    Limitations,
}
impl BriefFacet {
    /// Stable reading order for fallback, generated and consumer projections.
    pub const ALL: [Self; 8] = [
        Self::CurrentState,
        Self::SignificantDevelopments,
        Self::ActorRelationships,
        Self::CrossDomainImplications,
        Self::EmergingSignals,
        Self::ConditionalOutlook,
        Self::Evidence,
        Self::Limitations,
    ];
    /// Exact serialization/model-section identifier.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CurrentState => "current_state",
            Self::SignificantDevelopments => "significant_developments",
            Self::ActorRelationships => "actor_relationships",
            Self::CrossDomainImplications => "cross_domain_implications",
            Self::EmergingSignals => "emerging_signals",
            Self::ConditionalOutlook => "conditional_outlook",
            Self::Evidence => "evidence",
            Self::Limitations => "limitations",
        }
    }
    /// Visible heading within the existing summary pane.
    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::CurrentState => "Current State",
            Self::SignificantDevelopments => "Significant Developments",
            Self::ActorRelationships => "Actor Relationships",
            Self::CrossDomainImplications => "Cross-Domain Implications",
            Self::EmergingSignals => "Emerging Signals",
            Self::ConditionalOutlook => "Conditional Outlook",
            Self::Evidence => "Evidence",
            Self::Limitations => "Limitations",
        }
    }
    /// Parses an exact current facet identifier.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|facet| facet.as_str() == value)
    }
}

/// One additive summary facet. Empty facets disclose missing support rather
/// than generating content to fill a fixed presentation inventory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct FacetBlock<T> {
    pub facet: BriefFacet,
    pub items: Vec<T>,
    pub empty_reason: Option<String>,
}
impl<T> FacetBlock<T> {
    /// Captures supported items, or an honest bounded-view empty state.
    #[must_use]
    pub fn new(facet: BriefFacet, items: Vec<T>) -> Self {
        let empty_reason = items
            .is_empty()
            .then(|| "No supported statement in the retained bounded view.".into());
        Self {
            facet,
            items,
            empty_reason,
        }
    }
}

/// Bounded contract error. Source text is never embedded in the diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ContractError {
    InvalidField(&'static str),
    TooLarge(&'static str),
    MissingEvidence,
    UnsupportedChronology,
    InvalidConditionalImpact,
}
impl fmt::Display for ContractError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidField(field) => write!(f, "invalid intelligence contract field: {field}"),
            Self::TooLarge(field) => write!(f, "intelligence contract bound exceeded: {field}"),
            Self::MissingEvidence => f.write_str("relationship lacks validated evidence"),
            Self::UnsupportedChronology => f.write_str("relationship chronology is unsupported"),
            Self::InvalidConditionalImpact => {
                f.write_str("potential impact requires explicit conditions and limitations")
            }
        }
    }
}
impl std::error::Error for ContractError {}

/// Canonical actor identity and verified spellings, never model-guessed aliases.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct DomainActor {
    pub id: String,
    pub name: String,
    pub verified_aliases: Vec<String>,
}
impl DomainActor {
    /// Captures bounded, exact verified aliases in canonical order.
    pub fn new(
        id: String,
        name: String,
        mut verified_aliases: Vec<String>,
    ) -> Result<Self, ContractError> {
        valid_identifier(&id, "actor.id")?;
        valid_text(&name, 256, "actor.name")?;
        if verified_aliases.len() > MAX_CONTEXT_ALIASES {
            return Err(ContractError::TooLarge("actor.aliases"));
        }
        for alias in &verified_aliases {
            valid_text(alias, 256, "actor.alias")?;
        }
        verified_aliases.sort();
        verified_aliases.dedup();
        Ok(Self {
            id,
            name,
            verified_aliases,
        })
    }
}
/// Frozen evidence identity; unknown time/place remain explicitly absent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct DomainSource {
    pub run_id: String,
    pub article_id: String,
    pub metadata_revision: String,
    pub sentence_index_version: String,
    pub body_revision: Option<String>,
    pub publisher: String,
    pub url: String,
    pub published_at: Option<String>,
    pub place: Option<String>,
}
impl DomainSource {
    /// Builds an occurrence-bound source without supplying invented time/place.
    pub fn new(
        run_id: String,
        article_id: String,
        metadata_revision: String,
        sentence_index_version: String,
    ) -> Result<Self, ContractError> {
        for (value, field) in [
            (&run_id, "source.run_id"),
            (&article_id, "source.article_id"),
            (&metadata_revision, "source.metadata_revision"),
            (&sentence_index_version, "source.sentence_index_version"),
        ] {
            valid_identifier(value, field)?;
        }
        Ok(Self {
            run_id,
            article_id,
            metadata_revision,
            sentence_index_version,
            body_revision: None,
            publisher: String::new(),
            url: String::new(),
            published_at: None,
            place: None,
        })
    }
}
/// Domain identity established by a current admission receipt, not category text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct DomainAdmission {
    pub taxonomy_version: String,
    pub selected_domains: Vec<DomainId>,
    pub receipt_sha256: String,
}
impl DomainAdmission {
    /// Requires current taxonomy, distinct domains and a bound receipt digest.
    pub fn new(
        taxonomy: &str,
        selected_domains: impl Into<Vec<DomainId>>,
        receipt_sha256: String,
    ) -> Result<Self, ContractError> {
        if taxonomy != TAXONOMY_VERSION {
            return Err(ContractError::InvalidField("admission.taxonomy_version"));
        }
        let selected_domains = selected_domains.into();
        if selected_domains.is_empty()
            || selected_domains.len() > 2
            || (selected_domains.len() == 2 && selected_domains[0] == selected_domains[1])
        {
            return Err(ContractError::InvalidField("admission.selected_domains"));
        }
        if receipt_sha256.len() != 64
            || !receipt_sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(ContractError::InvalidField("admission.receipt_sha256"));
        }
        Ok(Self {
            taxonomy_version: taxonomy.into(),
            selected_domains,
            receipt_sha256,
        })
    }
}
/// Human-readable domain scope captured with its stable identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct DomainDefinition {
    pub id: DomainId,
    pub name: String,
    pub definition: String,
}
/// Frozen prompt/contract/route versions; job/lease epochs are not semantic inputs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct DomainVersions {
    pub prompt: String,
    pub contract: String,
    pub route: String,
}
impl DomainVersions {
    /// Captures semantic versions only, without credentials or mutable role state.
    pub fn new(prompt: String, contract: String, route: String) -> Result<Self, ContractError> {
        for (value, field) in [
            (&prompt, "versions.prompt"),
            (&contract, "versions.contract"),
            (&route, "versions.route"),
        ] {
            valid_identifier(value, field)?;
        }
        Ok(Self {
            prompt,
            contract,
            route,
        })
    }
}
/// Immutable metadata/domain context shared by the parallel brief roots.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct DomainPacket {
    pub schema_version: String,
    pub taxonomy_version: String,
    pub admission: Option<DomainAdmission>,
    pub domains: Vec<DomainDefinition>,
    pub source: DomainSource,
    pub actors: Vec<DomainActor>,
    pub versions: DomainVersions,
    pub unavailable_reason: Option<String>,
}
impl DomainPacket {
    /// Captures current admission context or an explicit unavailable legacy view.
    pub fn new(
        source: DomainSource,
        admission: Option<DomainAdmission>,
        mut actors: Vec<DomainActor>,
        versions: DomainVersions,
    ) -> Result<Self, ContractError> {
        if actors.len() > MAX_CONTEXT_ACTORS {
            return Err(ContractError::TooLarge("actors"));
        }
        actors.sort_by(|a, b| a.id.cmp(&b.id));
        if actors.windows(2).any(|pair| pair[0].id == pair[1].id) {
            return Err(ContractError::InvalidField("actors.duplicate_id"));
        }
        let domains = admission
            .as_ref()
            .map(|a| {
                a.selected_domains
                    .iter()
                    .map(|id| DomainDefinition {
                        id: *id,
                        name: id.display_name().into(),
                        definition: id.definition().into(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let packet = Self {
            schema_version: DOMAIN_CONTEXT_VERSION.into(),
            taxonomy_version: TAXONOMY_VERSION.into(),
            unavailable_reason: admission
                .is_none()
                .then(|| "no_current_admission_receipt".into()),
            admission,
            domains,
            source,
            actors,
            versions,
        };
        packet.validate()?;
        Ok(packet)
    }
    /// Revalidates a persisted snapshot before supplying it to a model.
    pub fn validate(&self) -> Result<(), ContractError> {
        if self.schema_version != DOMAIN_CONTEXT_VERSION
            || self.taxonomy_version != TAXONOMY_VERSION
        {
            return Err(ContractError::InvalidField("packet.version"));
        }
        valid_identifier(&self.source.run_id, "source.run_id")?;
        valid_identifier(&self.source.article_id, "source.article_id")?;
        valid_identifier(&self.source.metadata_revision, "source.metadata_revision")?;
        valid_identifier(
            &self.source.sentence_index_version,
            "source.sentence_index_version",
        )?;
        if self.source.url.len() > 2048 || self.source.publisher.len() > 256 {
            return Err(ContractError::TooLarge("source.provenance"));
        }
        if let Some(time) = &self.source.published_at {
            if chrono::DateTime::parse_from_rfc3339(time).is_err() {
                return Err(ContractError::InvalidField("source.published_at"));
            }
        }
        if let Some(place) = &self.source.place {
            valid_text(place, 256, "source.place")?;
        }
        if let Some(body) = &self.source.body_revision {
            valid_identifier(body, "source.body_revision")?;
        }
        DomainVersions::new(
            self.versions.prompt.clone(),
            self.versions.contract.clone(),
            self.versions.route.clone(),
        )?;
        if self.actors.len() > MAX_CONTEXT_ACTORS {
            return Err(ContractError::TooLarge("actors"));
        }
        let mut previous: Option<&str> = None;
        for actor in &self.actors {
            let canonical = DomainActor::new(
                actor.id.clone(),
                actor.name.clone(),
                actor.verified_aliases.clone(),
            )?;
            if canonical.verified_aliases != actor.verified_aliases {
                return Err(ContractError::InvalidField("actor.alias_order"));
            }
            if previous.is_some_and(|id| id >= actor.id.as_str()) {
                return Err(ContractError::InvalidField("actors.order"));
            }
            previous = Some(&actor.id);
        }
        match &self.admission {
            Some(admission) => {
                DomainAdmission::new(
                    &admission.taxonomy_version,
                    admission.selected_domains.clone(),
                    admission.receipt_sha256.clone(),
                )?;
                if self.domains.len() != admission.selected_domains.len()
                    || self.unavailable_reason.is_some()
                {
                    return Err(ContractError::InvalidField("packet.domains"));
                }
                for (scope, id) in self
                    .domains
                    .iter()
                    .zip(admission.selected_domains.iter().copied())
                {
                    if scope.id != id
                        || scope.name != id.display_name()
                        || scope.definition != id.definition()
                    {
                        return Err(ContractError::InvalidField("packet.domain_definition"));
                    }
                }
            }
            None if !self.domains.is_empty()
                || self.unavailable_reason.as_deref() != Some("no_current_admission_receipt") =>
            {
                return Err(ContractError::InvalidField("packet.legacy_context"))
            }
            None => {}
        }
        Ok(())
    }
    /// Deterministic identity binding both domains, aliases and semantic versions.
    pub fn revision(&self) -> Result<String, ContractError> {
        self.validate()?;
        let bytes = serde_json::to_vec(self)
            .map_err(|_| ContractError::InvalidField("packet.serialization"))?;
        let mut hash = Sha256::new();
        hash.update(b"domain-context-revision-v1\0");
        hash.update(bytes);
        Ok(format!("{:x}", hash.finalize()))
    }
}

/// Chronological basis is explicit; publication order is not event causation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ChronologyBasis {
    Event,
    Publication,
}
/// Typed validation receipt persisted only after existing reference/liveness and
/// semantic evaluator gates pass. It does not itself publish a graph edge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct LinkValidationReceipt {
    pub schema_version: String,
    pub kind: LinkKind,
    pub source: EndpointRef,
    pub target: EndpointRef,
    pub source_revision: String,
    pub target_revision: String,
    pub evidence: Vec<EvidenceRef>,
    pub source_lineage: String,
    pub target_lineage: String,
    pub chronology_basis: Option<ChronologyBasis>,
    pub source_time: Option<String>,
    pub target_time: Option<String>,
    pub conditions: Vec<String>,
    pub limitations: Vec<String>,
    pub decision_receipt: String,
    /// Frozen source/domain/route policy context of the actual validation.
    #[serde(default)]
    pub context_revision: String,
}
impl LinkValidationReceipt {
    /// Necessary structural/chronological checks; publishers must also validate
    /// every evidence ref and endpoint against their live canonical snapshot.
    pub fn validate_structure(&self) -> Result<(), ContractError> {
        if self.schema_version != LINK_VALIDATION_VERSION || self.source == self.target {
            return Err(ContractError::InvalidField("link.identity"));
        }
        for (value, field) in [
            (&self.source_revision, "link.source_revision"),
            (&self.target_revision, "link.target_revision"),
            (&self.decision_receipt, "link.decision_receipt"),
            (&self.context_revision, "link.context_revision"),
        ] {
            valid_identifier(value, field)?;
        }
        if self.evidence.is_empty() || self.evidence.len() > 32 {
            return Err(ContractError::MissingEvidence);
        }
        for lineage in [&self.source_lineage, &self.target_lineage] {
            if !lineage.is_empty() {
                valid_identifier(lineage, "link.lineage")?;
            }
        }
        if matches!(self.kind, LinkKind::Supports | LinkKind::Contradicts) {
            let roots: std::collections::BTreeSet<_> = self
                .evidence
                .iter()
                .map(|evidence| evidence.origin_root.as_str())
                .filter(|root| !root.is_empty())
                .collect();
            if self.source_lineage.is_empty()
                || self.target_lineage.is_empty()
                || self.source_lineage == self.target_lineage
                || roots.len() < 2
            {
                return Err(ContractError::MissingEvidence);
            }
        }
        if self.conditions.len() > 4 || self.limitations.len() > 4 {
            return Err(ContractError::TooLarge("link.conditions"));
        }
        for value in self.conditions.iter().chain(&self.limitations) {
            valid_text(value, 1024, "link.qualifier")?;
        }
        if self.kind == LinkKind::PotentialImpact
            && (self.conditions.is_empty() || self.limitations.is_empty())
        {
            return Err(ContractError::InvalidConditionalImpact);
        }
        if matches!(self.kind, LinkKind::Updates | LinkKind::Precedes) {
            if self.chronology_basis.is_none() {
                return Err(ContractError::UnsupportedChronology);
            }
            let source = self
                .source_time
                .as_ref()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .ok_or(ContractError::UnsupportedChronology)?;
            let target = self
                .target_time
                .as_ref()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .ok_or(ContractError::UnsupportedChronology)?;
            if (self.kind == LinkKind::Updates && source <= target)
                || (self.kind == LinkKind::Precedes && source >= target)
            {
                return Err(ContractError::UnsupportedChronology);
            }
        }
        Ok(())
    }
}
fn valid_identifier(value: &str, field: &'static str) -> Result<(), ContractError> {
    valid_text(value, 256, field)?;
    if value.chars().any(char::is_control) {
        return Err(ContractError::InvalidField(field));
    }
    Ok(())
}
fn valid_text(value: &str, maximum: usize, field: &'static str) -> Result<(), ContractError> {
    if value.trim().is_empty() {
        return Err(ContractError::InvalidField(field));
    }
    if value.len() > maximum {
        return Err(ContractError::TooLarge(field));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn packet(pair: Option<[DomainId; 2]>) -> DomainPacket {
        DomainPacket::new(
            DomainSource::new(
                "run-a".into(),
                "article-a".into(),
                "source-r1".into(),
                "sentence-v1".into(),
            )
            .expect("source"),
            pair.map(|pair| {
                DomainAdmission::new(TAXONOMY_VERSION, pair, "a".repeat(64))
                    .expect("current receipt")
            }),
            vec![DomainActor::new(
                "actor-a".into(),
                "Synthetic Harbor Authority".into(),
                vec!["Authority".into()],
            )
            .expect("actor")],
            DomainVersions::new("prompt-v1".into(), "contract-v1".into(), "route-v1".into())
                .expect("versions"),
        )
        .expect("packet")
    }
    #[test]
    fn packet_hash_binds_secondary_domain_aliases_and_prompt() {
        let first = packet(Some([DomainId::Governance, DomainId::Security]));
        let mut changed = first.clone();
        changed.versions.prompt = "prompt-v2".into();
        assert_ne!(first.revision(), changed.revision());
        let other = packet(Some([DomainId::Governance, DomainId::Human]));
        assert_ne!(first.revision(), other.revision());
        let mut alias = first.clone();
        alias.actors[0]
            .verified_aliases
            .push("Verified spelling".into());
        assert_ne!(first.revision(), alias.revision());
    }
    #[test]
    fn legacy_context_does_not_map_category_strings_or_manufacture_domains() {
        let legacy = packet(None);
        assert!(legacy.domains.is_empty());
        assert!(legacy.admission.is_none());
        assert_eq!(
            legacy.unavailable_reason.as_deref(),
            Some("no_current_admission_receipt")
        );
        assert!(DomainAdmission::new(
            "atlas-taxonomy-v2",
            [DomainId::Economic, DomainId::Human],
            "a".repeat(64)
        )
        .is_err());
        assert!(DomainAdmission::new(
            TAXONOMY_VERSION,
            [DomainId::Human, DomainId::Human],
            "a".repeat(64)
        )
        .is_err());
    }
    #[test]
    fn verified_alias_order_is_canonical_and_bounds_fail_visibly() {
        let a = DomainActor::new(
            "actor".into(),
            "Synthetic Actor".into(),
            vec!["B".into(), "A".into()],
        )
        .expect("aliases");
        let b = DomainActor::new(
            "actor".into(),
            "Synthetic Actor".into(),
            vec!["A".into(), "B".into()],
        )
        .expect("aliases");
        assert_eq!(a, b);
        assert!(DomainActor::new(
            "actor".into(),
            "Synthetic Actor".into(),
            vec!["Alias".into(); MAX_CONTEXT_ALIASES + 1]
        )
        .is_err());
    }
    fn relationship(kind: LinkKind) -> LinkValidationReceipt {
        LinkValidationReceipt {
            schema_version: LINK_VALIDATION_VERSION.into(), kind,
            source: EndpointRef::Memory { memory_id: "claim-source".into(), kind: crate::memory_metadata::MemoryKind::Claim },
            target: EndpointRef::Memory { memory_id: "claim-target".into(), kind: crate::memory_metadata::MemoryKind::Claim },
            source_revision: "source-r1".into(), target_revision: "target-r1".into(),
            evidence: vec![serde_json::from_value(serde_json::json!({
                "source_id":"source-a","revision":"source-r1","artifact_hash":"a".repeat(64),
                "acquisition_id":"acq-a","origin_root":"root-a","url":"https://fictional.example/a",
                "published_at":null,"retrieved_at":"2026-01-01T00:00:00Z","location":{"json_pointer":""}
            })).expect("immutable reference fixture")],
            source_lineage: "root-a".into(), target_lineage: "root-b".into(),
            chronology_basis: Some(ChronologyBasis::Event),
            source_time: Some("2026-01-02T00:00:00Z".into()), target_time: Some("2026-01-01T00:00:00Z".into()),
            conditions: Vec::new(), limitations: Vec::new(), decision_receipt: "semantic-decision-passed-v1".into(), context_revision: "context-v1".into(),
        }
    }
    #[test]
    fn directed_chronology_rejects_reversal_equal_times_and_unknown_basis() {
        let mut update = relationship(LinkKind::Updates);
        assert!(update.validate_structure().is_ok());
        update.kind = LinkKind::Precedes;
        assert_eq!(
            update.validate_structure(),
            Err(ContractError::UnsupportedChronology)
        );
        std::mem::swap(&mut update.source_time, &mut update.target_time);
        assert!(update.validate_structure().is_ok());
        update.target_time = update.source_time.clone();
        assert!(update.validate_structure().is_err());
        update.chronology_basis = None;
        assert!(update.validate_structure().is_err());
    }
    #[test]
    fn conditional_impact_and_independent_support_are_required() {
        let mut impact = relationship(LinkKind::PotentialImpact);
        assert_eq!(
            impact.validate_structure(),
            Err(ContractError::InvalidConditionalImpact)
        );
        impact
            .conditions
            .push("If the observed closure persists".into());
        impact
            .limitations
            .push("No independent traffic measurements".into());
        assert!(impact.validate_structure().is_ok());
        let mut support = relationship(LinkKind::Supports);
        support.target_lineage = support.source_lineage.clone();
        assert_eq!(
            support.validate_structure(),
            Err(ContractError::MissingEvidence)
        );
        assert!(
            LinkKind::parse("revises").is_none(),
            "legacy strings are not promoted by translation"
        );
    }
}
