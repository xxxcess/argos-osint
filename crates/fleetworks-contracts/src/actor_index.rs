//! Private actor projection protocol; contains no search-engine implementation.
//!
//! Only authenticated backend producers submit these records. They are derived
//! from committed canonical actors/aliases, never an uploaded CSV or a native
//! workspace. The public gateway still checks current canonical eligibility
//! and permissions before exposing any candidate label or dossier.

use std::collections::BTreeSet;
use std::fmt;

use serde::Deserialize;
use serde::Serialize;
use sha2::Digest;

pub const ACTOR_INDEX_PROTOCOL: u16 = 1;
pub const ACTOR_INDEX_SCHEMA: u16 = 1;
pub const ACTOR_INDEX_ANALYZER: u16 = 1;
pub const MAX_BATCH_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_BATCH_DOCUMENTS: usize = 500;
pub const MAX_QUERY_BYTES: usize = 256;
pub const MAX_QUERY_TOKENS: usize = 16;
pub const MAX_QUERY_RESULTS: u16 = 25;
pub const MAX_PROJECTION_BYTES: usize = 32 * 1024;

/// One deployment-resource policy shared by live changes and shadow rebuilds.
/// Values are operator-owned limits, not measured production capacity claims.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActorIndexLimits {
    pub batch_documents: u16,
    pub batch_bytes: u32,
    pub write_inflight: u8,
    pub prepared_batches: u8,
    pub min_start_ms: u32,
    pub partial_tail_ms: u32,
    pub query_capacity: u8,
    pub writer_memory_bytes: u64,
    pub writer_threads: u8,
    pub merge_threads: u8,
    pub memory_pause_bytes: u64,
    pub memory_resume_bytes: u64,
    pub disk_pause_free_bytes: u64,
    pub disk_resume_free_bytes: u64,
    pub backlog_pause_events: u64,
    pub backlog_resume_events: u64,
    pub query_pause_ms: u32,
    pub query_resume_ms: u32,
    pub commit_pause_ms: u32,
    pub commit_resume_ms: u32,
    pub backoff_initial_ms: u32,
    pub backoff_cap_ms: u32,
    pub cooldown_after: u8,
    pub cooldown_ms: u32,
}

impl Default for ActorIndexLimits {
    fn default() -> Self {
        Self {
            batch_documents: 500,
            batch_bytes: MAX_BATCH_BYTES as u32,
            write_inflight: 1,
            prepared_batches: 2,
            min_start_ms: 500,
            partial_tail_ms: 1000,
            query_capacity: 4,
            writer_memory_bytes: 64 * 1024 * 1024,
            writer_threads: 1,
            merge_threads: 1,
            memory_pause_bytes: 384 * 1024 * 1024,
            memory_resume_bytes: 320 * 1024 * 1024,
            disk_pause_free_bytes: 256 * 1024 * 1024,
            disk_resume_free_bytes: 512 * 1024 * 1024,
            backlog_pause_events: 500_000,
            backlog_resume_events: 250_000,
            query_pause_ms: 500,
            query_resume_ms: 250,
            commit_pause_ms: 5000,
            commit_resume_ms: 2000,
            backoff_initial_ms: 1000,
            backoff_cap_ms: 30_000,
            cooldown_after: 5,
            cooldown_ms: 60_000,
        }
    }
}

impl ActorIndexLimits {
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if !(1..=MAX_BATCH_DOCUMENTS as u16).contains(&self.batch_documents)
            || !(64 * 1024..=MAX_BATCH_BYTES as u32).contains(&self.batch_bytes)
            || self.write_inflight != 1
            || !(1..=2).contains(&self.prepared_batches)
            || !(500..=60_000).contains(&self.min_start_ms)
            || !(self.min_start_ms..=60_000).contains(&self.partial_tail_ms)
            || !(1..=8).contains(&self.query_capacity)
            || !(1..=2).contains(&self.writer_threads)
            || self.merge_threads != 1
            || !(32 * 1024 * 1024 * u64::from(self.writer_threads)..=256 * 1024 * 1024)
                .contains(&self.writer_memory_bytes)
            || self.memory_resume_bytes < self.writer_memory_bytes
            || self.memory_resume_bytes >= self.memory_pause_bytes
            || self.disk_pause_free_bytes < 64 * 1024 * 1024
            || self.disk_pause_free_bytes >= self.disk_resume_free_bytes
            || self.backlog_resume_events < 500
            || self.backlog_resume_events >= self.backlog_pause_events
            || self.query_resume_ms == 0
            || self.query_resume_ms >= self.query_pause_ms
            || self.commit_resume_ms == 0
            || self.commit_resume_ms >= self.commit_pause_ms
            || self.backoff_initial_ms < 1000
            || self.backoff_initial_ms > self.backoff_cap_ms
            || self.backoff_cap_ms > 30_000
            || self.cooldown_after == 0
            || self.cooldown_after > 5
            || self.cooldown_ms < 60_000
        {
            return Err(ProtocolError("invalid_actor_index_limits"));
        }
        Ok(())
    }
}

/// Actual latest resource measurements; absent evidence is never a healthy zero.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ActorIndexPressure {
    pub resident_bytes: Option<u64>,
    pub disk_free_bytes: Option<u64>,
    pub query_ewma_ms: u64,
    pub commit_ewma_ms: u64,
    pub paused: bool,
    pub reason_code: Option<String>,
    pub observed_at_unix_ms: u64,
}

/// The existing actor alias authority supplies strength and provenance.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActorIndexAlias {
    pub value: String,
    pub strength: String,
    pub provenance: Option<String>,
}

/// Bounded discovery data, not a dossier, key store or identity-resolution result.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActorProjection {
    pub display_name: String,
    pub aliases: Vec<ActorIndexAlias>,
    pub summary: Option<String>,
    pub actor_type: Option<String>,
    pub countries: Vec<String>,
    pub source_id: Option<String>,
    pub authority_scopes: Vec<String>,
    pub eligible: bool,
}

/// A monotonic source revision covers alias, visibility and lifecycle changes.
/// `projection: None` is a persisted tombstone, not an instruction to forget its fence.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActorIndexMutation {
    pub actor_id: String,
    pub revision: u64,
    pub projection: Option<ActorProjection>,
    pub checksum: String,
}

impl ActorIndexMutation {
    pub fn new(
        actor_id: String,
        revision: u64,
        projection: Option<ActorProjection>,
    ) -> Result<Self, ProtocolError> {
        let mut mutation = Self {
            actor_id,
            revision,
            projection,
            checksum: String::new(),
        };
        mutation.checksum = mutation.content_checksum()?;
        mutation.validate()?;
        Ok(mutation)
    }

    /// SHA-256 of the compact JSON tuple `(actor_id, revision, projection)`.
    /// The concrete DTO's field order is part of protocol version 1.
    pub fn content_checksum(&self) -> Result<String, ProtocolError> {
        digest(&(&self.actor_id, self.revision, &self.projection))
    }

    pub fn validate(&self) -> Result<(), ProtocolError> {
        bounded(&self.actor_id, 256)?;
        if self.revision == 0 {
            return Err(ProtocolError("projection_revision_required"));
        }
        if let Some(projection) = &self.projection {
            bounded(&projection.display_name, 1024)?;
            if projection.aliases.len() > 32
                || projection.countries.len() > 32
                || projection.authority_scopes.len() > 32
            {
                return Err(ProtocolError("projection_list_limit"));
            }
            for alias in &projection.aliases {
                bounded(&alias.value, 256)?;
                bounded(&alias.strength, 64)?;
                optional(alias.provenance.as_deref(), 256)?;
            }
            for country in &projection.countries {
                bounded(country, 64)?;
            }
            for scope in &projection.authority_scopes {
                bounded(scope, 256)?;
            }
            optional(projection.summary.as_deref(), 2048)?;
            optional(projection.actor_type.as_deref(), 128)?;
            optional(projection.source_id.as_deref(), 256)?;
            if serde_json::to_vec(projection)
                .map_err(|_| ProtocolError("projection_encoding"))?
                .len()
                > MAX_PROJECTION_BYTES
            {
                return Err(ProtocolError("projection_byte_limit"));
            }
        }
        if self.checksum != self.content_checksum()? {
            return Err(ProtocolError("projection_checksum_mismatch"));
        }
        Ok(())
    }
}

/// One leased coordinator sends ordered batches. Source epoch fences database
/// replacement; it is not the user-configuration reset epoch.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActorIndexBatch {
    pub protocol: u16,
    pub deployment_id: String,
    pub source_epoch: String,
    pub index_generation: String,
    pub batch_id: String,
    pub writer_epoch: u64,
    pub from_checkpoint: u64,
    pub through_checkpoint: u64,
    pub source_complete: bool,
    pub mutations: Vec<ActorIndexMutation>,
}

impl ActorIndexBatch {
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if self.protocol != ACTOR_INDEX_PROTOCOL {
            return Err(ProtocolError("unsupported_index_protocol"));
        }
        for value in [
            &self.deployment_id,
            &self.source_epoch,
            &self.index_generation,
            &self.batch_id,
        ] {
            bounded(value, 256)?;
        }
        if self.writer_epoch == 0 {
            return Err(ProtocolError("writer_epoch_required"));
        }
        if self.from_checkpoint > self.through_checkpoint {
            return Err(ProtocolError("checkpoint_order"));
        }
        if self.mutations.len() > MAX_BATCH_DOCUMENTS {
            return Err(ProtocolError("batch_document_limit"));
        }
        let mut ids = BTreeSet::new();
        for mutation in &self.mutations {
            mutation.validate()?;
            if !ids.insert(&mutation.actor_id) {
                return Err(ProtocolError("duplicate_batch_actor"));
            }
        }
        if serde_json::to_vec(self)
            .map_err(|_| ProtocolError("batch_encoding"))?
            .len()
            > MAX_BATCH_BYTES
        {
            return Err(ProtocolError("batch_byte_limit"));
        }
        Ok(())
    }

    /// Stable batch content excludes the coordinator lease epoch so an exact
    /// retry may reconcile under a newer lease without changing its identity.
    pub fn content_checksum(&self) -> Result<String, ProtocolError> {
        digest(&(
            self.protocol,
            &self.deployment_id,
            &self.source_epoch,
            &self.index_generation,
            &self.batch_id,
            self.from_checkpoint,
            self.through_checkpoint,
            self.source_complete,
            &self.mutations,
        ))
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActorIndexReceipt {
    pub batch_id: String,
    pub checksum: String,
    pub index_generation: String,
    pub committed_checkpoint: u64,
    pub visible_checkpoint: u64,
    pub writer_epoch: u64,
    pub replayed: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActorIndexQuery {
    pub query: String,
    pub limit: u16,
}

impl ActorIndexQuery {
    pub fn validate(&self) -> Result<(), ProtocolError> {
        bounded(&self.query, MAX_QUERY_BYTES)?;
        if !(1..=MAX_QUERY_RESULTS).contains(&self.limit) {
            return Err(ProtocolError("query_result_limit"));
        }
        if self.query.split_whitespace().count() > MAX_QUERY_TOKENS {
            return Err(ProtocolError("query_token_limit"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ActorIndexHit {
    pub actor_id: String,
    pub revision: u64,
    pub projection: ActorProjection,
    pub matched_field: String,
    /// Retrieval score only; never identity confidence or corroboration.
    pub score: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ActorIndexQueryResponse {
    pub deployment_id: String,
    pub source_epoch: String,
    pub index_generation: String,
    pub visible_checkpoint: u64,
    pub source_complete: bool,
    /// `rebuilding`/`stale` keeps a readable old snapshot distinct from current.
    pub serving_state: String,
    pub hits: Vec<ActorIndexHit>,
    pub truncated: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActorIndexRebuildStart {
    pub deployment_id: String,
    pub source_epoch: String,
    pub request_id: String,
    pub expected_generation: String,
    pub writer_epoch: u64,
    pub source_checkpoint: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActorIndexPromote {
    pub deployment_id: String,
    pub source_epoch: String,
    pub expected_generation: String,
    pub replacement_generation: String,
    pub writer_epoch: u64,
    pub catchup_checkpoint: u64,
}

/// Explicit private administration of one known inactive generation. Never a path.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActorIndexRetire {
    pub deployment_id: String,
    pub source_epoch: String,
    pub request_id: String,
    pub expected_generation: String,
    pub generation: String,
    pub expected_manifest_checksum: String,
    pub writer_epoch: u64,
}

/// Explicit corruption recovery, fenced by observed manifest bytes and source.
/// A service may refuse when ownership or safe retained-generation evidence is absent.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActorIndexRepair {
    pub deployment_id: String,
    pub source_epoch: String,
    pub request_id: String,
    pub expected_manifest_checksum: String,
    pub writer_epoch: u64,
}

impl ActorIndexRepair {
    pub fn validate(&self) -> Result<(), ProtocolError> {
        for value in [&self.deployment_id, &self.source_epoch, &self.request_id] {
            bounded(value, 256)?;
        }
        if self.writer_epoch == 0
            || self.expected_manifest_checksum.len() != 64
            || !self
                .expected_manifest_checksum
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(ProtocolError("invalid_repair_fence"));
        }
        Ok(())
    }
}

impl ActorIndexRetire {
    pub fn validate(&self) -> Result<(), ProtocolError> {
        ActorIndexRepair {
            deployment_id: self.deployment_id.clone(),
            source_epoch: self.source_epoch.clone(),
            request_id: self.request_id.clone(),
            expected_manifest_checksum: self.expected_manifest_checksum.clone(),
            writer_epoch: self.writer_epoch,
        }
        .validate()?;
        for generation in [&self.expected_generation, &self.generation] {
            if generation.len() > 32
                || generation.strip_prefix('g').is_none_or(|value| {
                    value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit())
                })
            {
                return Err(ProtocolError("invalid_generation_identity"));
            }
        }
        if self.expected_generation == self.generation {
            return Err(ProtocolError("cannot_retire_serving_generation"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ActorIndexGeneration {
    pub generation: String,
    pub committed_checkpoint: u64,
    pub visible_checkpoint: u64,
    pub source_complete: bool,
    pub writer_epoch: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ActorIndexStatus {
    pub protocol: u16,
    pub deployment_id: String,
    pub source_epoch: String,
    pub schema_version: u16,
    pub analyzer_version: u16,
    pub tantivy_version: String,
    pub index_format_version: u32,
    pub active: Option<ActorIndexGeneration>,
    pub rebuilding: Option<ActorIndexGeneration>,
    pub retained_generations: usize,
    #[serde(default)]
    pub retained_generation_ids: Vec<String>,
    #[serde(default)]
    pub manifest_checksum: Option<String>,
    #[serde(default)]
    pub recovery_available: bool,
    #[serde(default)]
    pub limits: ActorIndexLimits,
    #[serde(default)]
    pub pressure: Option<ActorIndexPressure>,
    pub state: String,
    pub reason_code: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProtocolError(pub &'static str);

impl fmt::Display for ProtocolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}
impl std::error::Error for ProtocolError {}

fn digest(value: &impl Serialize) -> Result<String, ProtocolError> {
    let bytes = serde_json::to_vec(value).map_err(|_| ProtocolError("checksum_encoding"))?;
    Ok(format!("{:x}", sha2::Sha256::digest(bytes)))
}

fn bounded(value: &str, limit: usize) -> Result<(), ProtocolError> {
    if value.trim().is_empty() || value.len() > limit || value.chars().any(char::is_control) {
        return Err(ProtocolError("invalid_bounded_text"));
    }
    Ok(())
}

fn optional(value: Option<&str>, limit: usize) -> Result<(), ProtocolError> {
    if let Some(value) = value {
        bounded(value, limit)?;
    }
    Ok(())
}

#[cfg(test)]
mod policy_tests {
    use super::*;

    #[test]
    fn deployment_limits_reject_amplification_and_invalid_hysteresis() {
        let defaults = ActorIndexLimits::default();
        assert!(defaults.validate().is_ok());
        for changed in [
            ActorIndexLimits {
                write_inflight: 2,
                ..defaults.clone()
            },
            ActorIndexLimits {
                min_start_ms: 499,
                ..defaults.clone()
            },
            ActorIndexLimits {
                batch_bytes: MAX_BATCH_BYTES as u32 + 1,
                ..defaults.clone()
            },
            ActorIndexLimits {
                prepared_batches: 3,
                ..defaults.clone()
            },
            ActorIndexLimits {
                memory_resume_bytes: defaults.memory_pause_bytes,
                ..defaults.clone()
            },
            ActorIndexLimits {
                disk_resume_free_bytes: defaults.disk_pause_free_bytes,
                ..defaults.clone()
            },
            ActorIndexLimits {
                cooldown_ms: 59_999,
                ..defaults.clone()
            },
        ] {
            assert!(changed.validate().is_err());
        }
    }

    #[test]
    fn generation_cleanup_is_identity_fenced_and_never_accepts_paths() {
        let request = ActorIndexRetire {
            deployment_id: "fixture-deployment".into(),
            source_epoch: "fixture-source".into(),
            request_id: "fixture-retire".into(),
            expected_generation: "g2".into(),
            generation: "g1".into(),
            expected_manifest_checksum: "a".repeat(64),
            writer_epoch: 1,
        };
        assert!(request.validate().is_ok());
        for generation in ["g2", "../g1", "/g1", "g1/other", ""] {
            assert!(ActorIndexRetire {
                generation: generation.into(),
                ..request.clone()
            }
            .validate()
            .is_err());
        }
    }
}
