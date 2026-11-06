//! Human review exchange. A review binds immutable captured source and permanent
//! family partition. CSV flags and model agreement never mint this provenance.
use super::provider_labels::{self, ProviderObservation};
use super::training::{FamilyId, LabelMembership, Split};
use super::{ClassifierError, Provenance};
use crate::atlas_taxonomy::DomainId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const REVIEW_SCHEMA: &str = "classifier-review-v1";
pub const MAX_REVIEW_ITEMS: usize = 512;
pub const MAX_REVIEW_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewDecision {
    Unreviewed,
    Complete,
    Abstain,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewItem {
    pub observation: ProviderObservation,
    pub payload_sha256: String,
    pub family_id: FamilyId,
    pub split: Split,
    pub revision: u64,
    pub decision: ReviewDecision,
    /// Complete membership, not an editorial pair. Empty can be a reviewed OOS.
    pub domains: Vec<DomainId>,
    pub review_note: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewExchange {
    pub schema_version: String,
    pub run_id: String,
    pub sampling_bias: String,
    pub items: Vec<ReviewItem>,
}
impl ReviewExchange {
    /// Validates human input and its exact captured observation. Storage must
    /// additionally compare current source/review revisions under its guard.
    pub fn validate(&self, reference: &str) -> Result<(), ClassifierError> {
        let invalid = || ClassifierError::InvalidTrainingData {
            row_id: None,
            reason: "invalid_review_exchange".into(),
        };
        if self.schema_version != REVIEW_SCHEMA
            || self.run_id.is_empty()
            || self.run_id.len() > 512
            || self.items.len() > MAX_REVIEW_ITEMS
            || reference.trim().is_empty()
            || reference.len() > 1024
            || reference.chars().any(char::is_control)
        {
            return Err(invalid());
        }
        struct Bound(usize);
        impl std::io::Write for Bound {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0 = self
                    .0
                    .checked_sub(bytes.len())
                    .ok_or_else(|| std::io::Error::other("review byte bound"))?;
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        serde_json::to_writer(&mut Bound(MAX_REVIEW_BYTES), self).map_err(|_| invalid())?;
        let mut seen = BTreeSet::new();
        for item in &self.items {
            provider_labels::validate_observation(&item.observation)?;
            let actual = super::artifact::sha256_hex(
                serde_json::to_string(&item.observation)
                    .map_err(|_| invalid())?
                    .as_bytes(),
            );
            if actual != item.payload_sha256
                || item.revision == 0
                || item.revision > i64::MAX as u64
                || !seen.insert(&item.observation.observation_id)
                || item.review_note.len() > 4096
                || item.domains.len() > 6
                || item.domains.iter().collect::<BTreeSet<_>>().len() != item.domains.len()
                || (item.decision != ReviewDecision::Complete && !item.domains.is_empty())
            {
                return Err(invalid());
            }
        }
        Ok(())
    }
    /// Exact review evidence identity; source observations and assignments are
    /// part of the digest. Persist this and the actual external review reference.
    pub fn sha256(&self) -> Result<String, ClassifierError> {
        let bytes = serde_json::to_vec(self)
            .map_err(|_| ClassifierError::InvalidArtifact("review_serialization".into()))?;
        Ok(super::artifact::sha256_hex(&bytes))
    }
}
impl ReviewItem {
    pub fn membership(&self) -> [LabelMembership; 6] {
        DomainId::ALL.map(|domain| {
            if self.decision != ReviewDecision::Complete {
                LabelMembership::Unknown
            } else if self.domains.contains(&domain) {
                LabelMembership::Present
            } else {
                LabelMembership::Absent
            }
        })
    }
    pub fn provenance(&self, corpus_sha256: &str, reference: &str) -> Provenance {
        Provenance::ReviewedReal {
            dataset_version: REVIEW_SCHEMA.into(),
            dataset_sha256: corpus_sha256.into(),
            review_reference: reference.into(),
        }
    }
}
