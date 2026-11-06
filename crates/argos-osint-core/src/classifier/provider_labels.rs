//! Provider-label capture, category vocabulary filtering, sentence anchor evidence
//! predicates, and zero-approval weak-training masking.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::training::LabelMembership;
use super::{ClassifierError, ProviderLabelDiagnostics};
use crate::atlas_taxonomy::DomainId;

/// Mapping and predicate version string.
pub const MAPPING_VERSION: &str = "provider-categories-evidence-v1";

/// Maximum observation payload byte size before filtering (128 KiB).
pub const MAX_OBSERVATION_BYTES: usize = 128 * 1024;

/// Maximum number of raw category labels permitted per observation (64).
pub const MAX_RAW_LABELS: usize = 64;

/// Maximum byte length for a single raw label string (256).
pub const MAX_RAW_LABEL_BYTES: usize = 256;

/// Maximum evidence spans recorded per observation (64).
pub const MAX_EVIDENCE_SPANS: usize = 64;

/// Provider payload schema and API version format.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderFormat {
    NewsDataV1,
    CurrentsV1,
    Unsupported { provider: String, version: String },
}

/// An immutable provider article observation captured at ingest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderObservation {
    pub observation_id: String,
    pub provider: ProviderFormat,
    pub endpoint: String,
    pub publisher: Option<String>,
    pub url: String,
    pub article_id: Option<String>,
    pub published_at: Option<String>,
    pub fetched_at: String,
    pub cycle_ids: Vec<String>,
    pub input_hash: String,
    pub source_revision: String,
    pub title: String,
    pub description: String,
    pub language: Option<String>,
    pub raw_categories: serde_json::Value,
    pub raw_ai_tags: Option<serde_json::Value>,
    pub raw_metadata: serde_json::Value,
}

/// Filtering disposition for a raw category label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LabelDisposition {
    Eligible,
    Excluded,
    Unknown,
}

/// A parsed and normalized category label with disposition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilteredLabel {
    pub raw_index: usize,
    pub normalized: String,
    pub disposition: LabelDisposition,
}

/// Reasons why a provider observation abstains from weak training or monitoring.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AbstainReason {
    UnsupportedPayloadVersion,
    UnknownVocabulary,
    MalformedCategories,
    NoEligibleLabels,
    NoTextualEvidence,
    UnsupportedLanguage,
    UnreviewedMapping,
}

/// Field in which textual evidence was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceField {
    Title,
    Description,
}

/// Grounded sentence-level textual anchor match evidencing a domain topic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabelEvidence {
    pub domain: DomainId,
    pub raw_label_indices: Vec<usize>,
    pub predicate_id: String,
    pub field: EvidenceField,
    pub start_byte: usize,
    pub end_byte: usize,
    pub matched_text: String,
    pub input_hash: String,
}

/// The result of mapping and masking an observation's provider categories.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaskedLabels {
    pub mapping_version: String,
    pub membership: [LabelMembership; 6],
    pub training_membership: [LabelMembership; 6],
    pub filtered: Vec<FilteredLabel>,
    pub evidence: Vec<LabelEvidence>,
    pub abstain: Option<AbstainReason>,
    pub eligible_for_training: bool,
}

/// Independent precision validation for an approved mapping rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleValidation {
    pub domain: DomainId,
    pub true_positive_families: u64,
    pub false_positive_families: u64,
}

/// Signed certificate approving weak-training emission for a predicate set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MappingApproval {
    pub mapping_version: String,
    pub predicates_sha256: String,
    pub review_reference: String,
    pub reviewed_corpus_sha256: String,
    pub domains: [RuleValidation; 6],
}

// -----------------------------------------------------------------------------
// Validation & Category Filtering
// -----------------------------------------------------------------------------

/// Validates an incoming `ProviderObservation` against byte and count caps.
///
/// # Errors
/// Returns [`ClassifierError::ObservationTooLarge`] if text size, category count,
/// or label length exceeds bounded limits.
pub fn validate_observation(observation: &ProviderObservation) -> Result<(), ClassifierError> {
    struct SizeBound {
        remaining: usize,
    }
    impl std::io::Write for SizeBound {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.remaining = self
                .remaining
                .checked_sub(bytes.len())
                .ok_or_else(|| std::io::Error::other("observation exceeds 128 KiB"))?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    serde_json::to_writer(
        &mut SizeBound {
            remaining: MAX_OBSERVATION_BYTES,
        },
        observation,
    )
    .map_err(|_| ClassifierError::ObservationTooLarge)?;

    if let Some(categories) = observation.raw_categories.as_array() {
        if categories.len() > MAX_RAW_LABELS {
            return Err(ClassifierError::ObservationTooLarge);
        }
        for cat in categories {
            if let Some(s) = cat.as_str() {
                if s.len() > MAX_RAW_LABEL_BYTES {
                    return Err(ClassifierError::ObservationTooLarge);
                }
            }
        }
    }

    Ok(())
}

const NEWSDATA_ELIGIBLE: &[&str] = &[
    "business",
    "crime",
    "domestic",
    "environment",
    "food",
    "health",
    "politics",
    "science",
    "technology",
];

const CURRENTS_ELIGIBLE: &[&str] = &[
    "business",
    "food",
    "health",
    "programming",
    "science",
    "technology",
];

const EXCLUDED_LABELS: &[&str] = &[
    "education",
    "entertainment",
    "lifestyle",
    "other",
    "top",
    "tourism",
    "world",
    "regional",
    "general",
    "sports",
];

fn candidate_domains_for_label(label: &str) -> &'static [DomainId] {
    match label {
        "politics" | "domestic" => &[DomainId::Governance, DomainId::Security, DomainId::Human],
        "crime" => &[DomainId::Security, DomainId::Governance, DomainId::Human],
        "business" => &[
            DomainId::Economic,
            DomainId::Governance,
            DomainId::Security,
            DomainId::Human,
            DomainId::Environmental,
            DomainId::Scientific,
        ],
        "environment" => &[DomainId::Environmental, DomainId::Economic, DomainId::Human],
        "food" => &[DomainId::Human, DomainId::Economic, DomainId::Environmental],
        "health" => &[DomainId::Human, DomainId::Scientific],
        "science" | "technology" | "programming" => {
            &[DomainId::Scientific, DomainId::Security, DomainId::Economic]
        }
        _ => &[],
    }
}

const GOVERNANCE_ANCHORS: &[&str] = &[
    "election commission",
    "parliament",
    "constitutional court",
    "government coalition",
    "diplomatic treaty",
];

const SECURITY_ANCHORS: &[&str] = &[
    "armed attack",
    "military operation",
    "cyber attack",
    "ransomware attack",
    "criminal investigation",
    "police investigation",
];

const ECONOMIC_ANCHORS: &[&str] = &[
    "supply chain",
    "trade tariff",
    "export ban",
    "factory production",
    "freight shipment",
    "central bank",
    "food prices",
];

const HUMAN_ANCHORS: &[&str] = &[
    "public health",
    "hospital care",
    "food access",
    "food shortage",
    "displaced families",
    "drinking water",
    "essential services",
];

const ENVIRONMENTAL_ANCHORS: &[&str] = &[
    "water pollution",
    "forest fire",
    "drought",
    "flooding",
    "ecosystem",
    "resource depletion",
];

const SCIENTIFIC_ANCHORS: &[&str] = &[
    "scientific study",
    "clinical trial",
    "laboratory research",
    "technical performance",
    "research capability",
    "software engineering",
];

fn anchors_for_domain(domain: DomainId) -> &'static [&'static str] {
    match domain {
        DomainId::Governance => GOVERNANCE_ANCHORS,
        DomainId::Security => SECURITY_ANCHORS,
        DomainId::Economic => ECONOMIC_ANCHORS,
        DomainId::Human => HUMAN_ANCHORS,
        DomainId::Environmental => ENVIRONMENTAL_ANCHORS,
        DomainId::Scientific => SCIENTIFIC_ANCHORS,
    }
}

// -----------------------------------------------------------------------------
// Sentence Anchor Search
// -----------------------------------------------------------------------------

/// Hashes the exact v1 vocabulary, domain routing and textual evidence predicates.
#[must_use]
pub fn mapping_predicates_sha256() -> String {
    let mut digest = Sha256::new();
    let mut frame = |text: &str| {
        digest.update((text.len() as u64).to_le_bytes());
        digest.update(text.as_bytes());
    };
    frame(MAPPING_VERSION);
    frame("sentence-delimiters:.!?\n;ascii-casefold;unicode-word-boundaries;phrase-unicode-whitespace-v1");
    for (provider, labels) in [
        ("newsdata-v1", NEWSDATA_ELIGIBLE),
        ("currents-v1", CURRENTS_ELIGIBLE),
    ] {
        frame(provider);
        for label in labels {
            frame(label);
            for domain in candidate_domains_for_label(label) {
                frame(domain.as_str());
            }
        }
    }
    for label in EXCLUDED_LABELS {
        frame(label);
    }
    for domain in DomainId::ALL {
        frame(domain.as_str());
        for anchor in anchors_for_domain(domain) {
            frame(anchor);
        }
    }
    format!("{:x}", digest.finalize())
}

/// Validates an independent review certificate against the compiled predicate set.
pub fn validate_approval(approval: &MappingApproval) -> Result<(), ClassifierError> {
    let valid = approval.mapping_version == MAPPING_VERSION
        && approval.predicates_sha256 == mapping_predicates_sha256()
        && !approval.review_reference.trim().is_empty()
        && approval.reviewed_corpus_sha256.len() == 64
        && approval
            .reviewed_corpus_sha256
            .bytes()
            .all(|b| b.is_ascii_hexdigit())
        && approval
            .domains
            .iter()
            .zip(DomainId::ALL)
            .all(|(rule, domain)| {
                rule.domain == domain
                    && rule
                        .true_positive_families
                        .checked_add(rule.false_positive_families)
                        .is_some()
            });
    if !valid {
        return Err(ClassifierError::InvalidCalibrationData {
            family_id: None,
            reason: "invalid_mapping_approval".into(),
        });
    }
    Ok(())
}

fn find_anchor_matches(
    text: &str,
    field: EvidenceField,
    domain: DomainId,
    eligible_raw_indices: &[usize],
    input_hash: &str,
    evidence_out: &mut Vec<LabelEvidence>,
) -> Result<bool, ClassifierError> {
    let anchors = anchors_for_domain(domain);
    let mut matched_any = false;

    // Sentence splitting on delimiters '.', '!', '?', '\n'
    let mut sentence_start = 0;
    let bytes = text.as_bytes();

    for (idx, &b) in bytes.iter().enumerate() {
        if b == b'.' || b == b'!' || b == b'?' || b == b'\n' || idx == bytes.len() - 1 {
            let sentence_end = if idx == bytes.len() - 1 { idx + 1 } else { idx };
            let sentence = &text[sentence_start..sentence_end];

            for anchor in anchors {
                let anchor_lower = anchor.to_ascii_lowercase();
                let sentence_lower = sentence.to_ascii_lowercase();

                let words: Vec<_> = anchor_lower.split_ascii_whitespace().collect();
                let Some(first_word) = words.first() else {
                    continue;
                };
                let mut search_from = 0;
                while let Some(pos) = sentence_lower[search_from..].find(*first_word) {
                    let match_start_in_sentence = search_from + pos;
                    let mut match_end_in_sentence = match_start_in_sentence + first_word.len();
                    let mut phrase_matches = true;
                    for word in &words[1..] {
                        let suffix = &sentence_lower[match_end_in_sentence..];
                        let whitespace_bytes: usize = suffix
                            .chars()
                            .take_while(|ch| ch.is_whitespace())
                            .map(char::len_utf8)
                            .sum();
                        if whitespace_bytes == 0 || !suffix[whitespace_bytes..].starts_with(*word) {
                            phrase_matches = false;
                            break;
                        }
                        match_end_in_sentence += whitespace_bytes + word.len();
                    }
                    if !phrase_matches {
                        search_from = match_start_in_sentence + 1;
                        continue;
                    }

                    // Word boundary check: character immediately preceding and following
                    // must not be alphanumeric or underscore
                    let prev_ok = if match_start_in_sentence == 0 {
                        true
                    } else {
                        sentence[..match_start_in_sentence]
                            .chars()
                            .next_back()
                            .is_some_and(|c| !c.is_alphanumeric() && c != '_')
                    };

                    let next_ok = if match_end_in_sentence == sentence.len() {
                        true
                    } else {
                        sentence[match_end_in_sentence..]
                            .chars()
                            .next()
                            .is_some_and(|c| !c.is_alphanumeric() && c != '_')
                    };

                    if prev_ok && next_ok {
                        if evidence_out.len() >= MAX_EVIDENCE_SPANS {
                            return Err(ClassifierError::ObservationTooLarge);
                        }

                        let start_byte = sentence_start + match_start_in_sentence;
                        let end_byte = sentence_start + match_end_in_sentence;
                        let matched_text = text[start_byte..end_byte].to_string();

                        evidence_out.push(LabelEvidence {
                            domain,
                            raw_label_indices: eligible_raw_indices.to_vec(),
                            predicate_id: format!(
                                "{MAPPING_VERSION}:{}:anchor-v1",
                                domain.as_str()
                            ),
                            field,
                            start_byte,
                            end_byte,
                            matched_text,
                            input_hash: input_hash.to_string(),
                        });
                        matched_any = true;
                    }

                    search_from = match_start_in_sentence + 1;
                }
            }

            sentence_start = sentence_end + 1;
        }
    }

    Ok(matched_any)
}

// -----------------------------------------------------------------------------
// Mapping Function
// -----------------------------------------------------------------------------

/// Maps an observation's provider categories against eligible vocabulary,
/// sentences, and textual evidence anchors.
///
/// # Errors
/// Returns [`ClassifierError`] if validation fails or size bounds are exceeded.
pub fn map_labels(
    observation: &ProviderObservation,
    approval: Option<&MappingApproval>,
) -> Result<MaskedLabels, ClassifierError> {
    validate_observation(observation)?;
    if let Some(approval) = approval {
        validate_approval(approval)?;
    }

    // 1. Check supported provider format
    let eligible_vocab: &[&str] = match &observation.provider {
        ProviderFormat::NewsDataV1 => NEWSDATA_ELIGIBLE,
        ProviderFormat::CurrentsV1 => CURRENTS_ELIGIBLE,
        ProviderFormat::Unsupported { .. } => {
            return Ok(MaskedLabels {
                mapping_version: MAPPING_VERSION.into(),
                membership: [LabelMembership::Unknown; 6],
                training_membership: [LabelMembership::Unknown; 6],
                filtered: Vec::new(),
                evidence: Vec::new(),
                abstain: Some(AbstainReason::UnsupportedPayloadVersion),
                eligible_for_training: false,
            });
        }
    };

    // 2. Check categories array format
    let categories = match observation.raw_categories.as_array() {
        Some(arr) => arr,
        None => {
            return Ok(MaskedLabels {
                mapping_version: MAPPING_VERSION.into(),
                membership: [LabelMembership::Unknown; 6],
                training_membership: [LabelMembership::Unknown; 6],
                filtered: Vec::new(),
                evidence: Vec::new(),
                abstain: Some(AbstainReason::MalformedCategories),
                eligible_for_training: false,
            });
        }
    };

    if categories.is_empty() {
        return Ok(MaskedLabels {
            mapping_version: MAPPING_VERSION.into(),
            membership: [LabelMembership::Unknown; 6],
            training_membership: [LabelMembership::Unknown; 6],
            filtered: Vec::new(),
            evidence: Vec::new(),
            abstain: Some(AbstainReason::NoEligibleLabels),
            eligible_for_training: false,
        });
    }

    // 3. Filter category labels
    let mut filtered = Vec::with_capacity(categories.len());
    let mut has_unknown_vocab = false;
    let mut eligible_indices = Vec::new();
    let mut candidate_domains = BTreeSet::new();

    for (idx, cat_val) in categories.iter().enumerate() {
        let cat_str = match cat_val.as_str() {
            Some(s) => s,
            None => {
                has_unknown_vocab = true;
                filtered.push(FilteredLabel {
                    raw_index: idx,
                    normalized: String::new(),
                    disposition: LabelDisposition::Unknown,
                });
                continue;
            }
        };

        let normalized = cat_str.trim().to_ascii_lowercase();
        let disposition = if eligible_vocab.contains(&normalized.as_str()) {
            eligible_indices.push(idx);
            for d in candidate_domains_for_label(&normalized) {
                candidate_domains.insert(*d);
            }
            LabelDisposition::Eligible
        } else if EXCLUDED_LABELS.contains(&normalized.as_str()) {
            LabelDisposition::Excluded
        } else {
            has_unknown_vocab = true;
            LabelDisposition::Unknown
        };

        filtered.push(FilteredLabel {
            raw_index: idx,
            normalized,
            disposition,
        });
    }

    // Decision L1: Any unknown vocabulary causes whole-observation abstention
    if has_unknown_vocab {
        return Ok(MaskedLabels {
            mapping_version: MAPPING_VERSION.into(),
            membership: [LabelMembership::Unknown; 6],
            training_membership: [LabelMembership::Unknown; 6],
            filtered,
            evidence: Vec::new(),
            abstain: Some(AbstainReason::UnknownVocabulary),
            eligible_for_training: false,
        });
    }

    if eligible_indices.is_empty() {
        return Ok(MaskedLabels {
            mapping_version: MAPPING_VERSION.into(),
            membership: [LabelMembership::Unknown; 6],
            training_membership: [LabelMembership::Unknown; 6],
            filtered,
            evidence: Vec::new(),
            abstain: Some(AbstainReason::NoEligibleLabels),
            eligible_for_training: false,
        });
    }

    // 4. Textual evidence anchor matching (Decision L3)
    let mut evidence = Vec::new();
    let mut proposed_membership = [LabelMembership::Unknown; 6];

    for (domain_idx, domain) in DomainId::ALL.iter().enumerate() {
        if candidate_domains.contains(domain) {
            let matched_title = find_anchor_matches(
                &observation.title,
                EvidenceField::Title,
                *domain,
                &eligible_indices,
                &observation.input_hash,
                &mut evidence,
            )?;

            let matched_desc = find_anchor_matches(
                &observation.description,
                EvidenceField::Description,
                *domain,
                &eligible_indices,
                &observation.input_hash,
                &mut evidence,
            )?;

            if matched_title || matched_desc {
                proposed_membership[domain_idx] = LabelMembership::Present;
            }
        }
    }

    let has_any_evidence = evidence.iter().any(|e| {
        DomainId::ALL
            .iter()
            .position(|d| *d == e.domain)
            .is_some_and(|idx| proposed_membership[idx] == LabelMembership::Present)
    });

    if !has_any_evidence {
        return Ok(MaskedLabels {
            mapping_version: MAPPING_VERSION.into(),
            membership: [LabelMembership::Unknown; 6],
            training_membership: [LabelMembership::Unknown; 6],
            filtered,
            evidence,
            abstain: Some(AbstainReason::NoTextualEvidence),
            eligible_for_training: false,
        });
    }

    // 5. Zero-approval weak-training policy (Decision L4)
    // V1 starts with ZERO approved weak-training mappings.
    // If approval certificate is provided, verify matching hash and domain precision.
    let mut training_membership = [LabelMembership::Unknown; 6];
    let mut is_approved = false;

    let supported_language = observation.language.as_deref().is_some_and(|language| {
        let language = language.to_ascii_lowercase();
        language == "en" || language.starts_with("en-")
    });
    if let Some(appr) = approval.filter(|_| supported_language) {
        if appr.mapping_version == MAPPING_VERSION {
            for (idx, rule) in appr.domains.iter().enumerate() {
                let accepted = rule.true_positive_families + rule.false_positive_families;
                if accepted >= 20
                    && 10 * u128::from(rule.true_positive_families) >= 9 * u128::from(accepted)
                    && proposed_membership[idx] == LabelMembership::Present
                {
                    training_membership[idx] = LabelMembership::Present;
                    is_approved = true;
                }
            }
        }
    }

    let abstain = if is_approved {
        None
    } else if !supported_language {
        Some(AbstainReason::UnsupportedLanguage)
    } else {
        Some(AbstainReason::UnreviewedMapping)
    };

    Ok(MaskedLabels {
        mapping_version: MAPPING_VERSION.into(),
        membership: proposed_membership,
        training_membership,
        filtered,
        evidence,
        abstain,
        eligible_for_training: is_approved,
    })
}

/// Extracts structured provider diagnostics for receipt telemetry.
///
/// # Errors
/// Returns [`ClassifierError`] on serialization failure.
pub fn diagnostics(
    observation: &ProviderObservation,
    mapping: &MaskedLabels,
) -> Result<ProviderLabelDiagnostics, ClassifierError> {
    let filtered_labels_val = serde_json::to_value(&mapping.filtered)
        .map_err(|e| ClassifierError::Storage(format!("serialize filtered_labels: {e}")))?;
    let evidence_val = serde_json::to_value(&mapping.evidence)
        .map_err(|e| ClassifierError::Storage(format!("serialize evidence: {e}")))?;

    Ok(ProviderLabelDiagnostics {
        provider: format!("{:?}", observation.provider),
        endpoint: observation.endpoint.clone(),
        mapping_version: mapping.mapping_version.clone(),
        observation_id: observation.observation_id.clone(),
        raw_categories: observation.raw_categories.clone(),
        filtered_labels: filtered_labels_val,
        evidence: evidence_val,
        membership: mapping.membership.map(|m| m.as_str().to_string()),
        training_membership: mapping.training_membership.map(|m| m.as_str().to_string()),
        abstain_reason: mapping.abstain.map(|r| format!("{r:?}")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_observation(
        categories: serde_json::Value,
        title: &str,
        desc: &str,
    ) -> ProviderObservation {
        ProviderObservation {
            observation_id: "obs-1".into(),
            provider: ProviderFormat::NewsDataV1,
            endpoint: "https://newsdata.io/api/1/news".into(),
            publisher: Some("Test Publisher".into()),
            url: "https://example.com/story".into(),
            article_id: Some("art-1".into()),
            published_at: Some("2026-11-05T00:00:00Z".into()),
            fetched_at: "2026-11-05T01:00:00Z".into(),
            cycle_ids: vec!["cycle-1".into()],
            input_hash: "0".repeat(64),
            source_revision: "rev-1".into(),
            title: title.into(),
            description: desc.into(),
            language: Some("en".into()),
            raw_categories: categories,
            raw_ai_tags: None,
            raw_metadata: serde_json::json!({}),
        }
    }

    #[test]
    fn test_unknown_vocabulary_abstention() {
        let obs = dummy_observation(
            serde_json::json!(["politics", "unknown_category"]),
            "The parliament approved a treaty.",
            "Details follow.",
        );

        let masked = map_labels(&obs, None).unwrap();
        assert_eq!(masked.abstain, Some(AbstainReason::UnknownVocabulary));
        assert_eq!(masked.membership, [LabelMembership::Unknown; 6]);
    }

    #[test]
    fn test_anchor_matching_and_unreviewed_mapping() {
        let obs = dummy_observation(
            serde_json::json!(["politics"]),
            "The parliament voted on a constitutional court proposal.",
            "A government coalition has formed.",
        );

        let masked = map_labels(&obs, None).unwrap();
        // Proposed membership identifies Governance due to anchors
        assert_eq!(masked.membership[0], LabelMembership::Present); // Governance is index 0
                                                                    // But training_membership remains Unknown in v1 with no approval
        assert_eq!(masked.training_membership[0], LabelMembership::Unknown);
        assert!(!masked.eligible_for_training);
        assert_eq!(masked.abstain, Some(AbstainReason::UnreviewedMapping));
        assert!(!masked.evidence.is_empty());
    }
}
