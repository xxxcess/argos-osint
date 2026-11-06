//! Bounded explanation DTOs for the shared Natural feature policy.
use super::ClassifierError;
use serde::{Deserialize, Serialize};

pub const VERSION: &str = "natural-en-phrases-v2";
pub const MAX_FEATURES: usize = 4096;
pub const MAX_EXPLANATIONS: usize = 8;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FeatureContribution {
    pub id: String,
    pub field: String,
    pub start_byte: usize,
    pub end_byte: usize,
    pub phrase: bool,
    pub contribution: f64,
}

/// Statistical model behavior, never proof of the article's claims.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DomainSupport {
    pub positive_spans: usize,
    pub negative_features: usize,
    pub vocabulary_matches: usize,
    pub oov_fraction: f64,
    pub active_phrases: usize,
    pub class_prior: f64,
    pub positive: Vec<FeatureContribution>,
    pub negative: Vec<FeatureContribution>,
}
impl DomainSupport {
    pub fn validate(&self) -> Result<(), ClassifierError> {
        if self.positive_spans > MAX_FEATURES
            || self.negative_features > MAX_FEATURES
            || self.vocabulary_matches > MAX_FEATURES
            || self.active_phrases > MAX_FEATURES
            || self.positive_spans > self.vocabulary_matches
            || self.negative_features > self.vocabulary_matches
            || self.active_phrases > self.vocabulary_matches
            || !self.class_prior.is_finite()
            || !self.oov_fraction.is_finite()
            || !(0.0..=1.0).contains(&self.oov_fraction)
            || self.positive.len() > MAX_EXPLANATIONS
            || self.negative.len() > MAX_EXPLANATIONS
            || self.positive.iter().chain(&self.negative).any(|feature| {
                feature.id.len() > 32768
                    || !matches!(feature.field.as_str(), "title" | "description")
                    || feature.start_byte >= feature.end_byte
                    || feature.end_byte > 32768
                    || !feature.contribution.is_finite()
            })
            || self.positive.iter().any(|f| f.contribution <= 0.0)
            || self.negative.iter().any(|f| f.contribution >= 0.0)
        {
            return Err(ClassifierError::InvalidArtifact(
                "feature_support_bound".into(),
            ));
        }
        Ok(())
    }
}

/// Frozen train-family vocabulary. TF-IDF selects/inspects features; these are
/// never fractional numerical inputs to Natural's binary Bayes classifier.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrozenVocabulary {
    pub version: String,
    pub min_document_frequency: usize,
    pub max_vocabulary: usize,
    pub include_bigrams: bool,
    pub field_namespaces: bool,
    pub training_documents: usize,
    pub terms: Vec<VocabularyTerm>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VocabularyTerm {
    pub id: String,
    pub document_frequency: usize,
    pub idf: f64,
    pub association: f64,
}
impl FrozenVocabulary {
    pub fn validate(&self) -> Result<(), ClassifierError> {
        let invalid = || ClassifierError::InvalidArtifact("invalid_frozen_vocabulary".into());
        if self.version != VERSION
            || self.training_documents == 0
            || self.training_documents > 50000
            || self.min_document_frequency == 0
            || self.min_document_frequency > self.training_documents
            || self.max_vocabulary == 0
            || self.max_vocabulary > 100000
            || self.terms.is_empty()
            || self.terms.len() > self.max_vocabulary
        {
            return Err(invalid());
        }
        let mut seen = std::collections::BTreeSet::new();
        for term in &self.terms {
            let id = if self.field_namespaces {
                term.id
                    .strip_prefix("title:")
                    .or_else(|| term.id.strip_prefix("description:"))
                    .ok_or_else(invalid)?
            } else {
                &term.id
            };
            if (!id.starts_with("u:") && !id.starts_with("b:"))
                || term.id.len() > 2 * super::MAX_INPUT_TEXT_BYTES
                || !seen.insert(&term.id)
                || term.document_frequency < self.min_document_frequency
                || term.document_frequency > self.training_documents
                || !term.idf.is_finite()
                || (term.idf
                    - (1.0
                        + (self.training_documents as f64 / (1 + term.document_frequency) as f64)
                            .ln()))
                .abs()
                    > 1e-12
                || !term.association.is_finite()
                || term.association <= 0.0
                || term.association > 32.0
            {
                return Err(invalid());
            }
        }
        Ok(())
    }
}
