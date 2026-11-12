//! Explicit, manifest-fenced recovery/retirement of this deployment's owned
//! generations. No request supplies a directory, URL or unverified filename.

use std::fs;
use std::fs::File;
use std::io::Read;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

use fleetworks_contracts::actor_index::ActorIndexLimits;
use fleetworks_contracts::actor_index::ActorIndexRepair;
use fleetworks_contracts::actor_index::ActorIndexRetire;
use fleetworks_contracts::actor_index::ActorIndexStatus;
use fleetworks_contracts::actor_index::ACTOR_INDEX_ANALYZER;
use fleetworks_contracts::actor_index::ACTOR_INDEX_PROTOCOL;
use fleetworks_contracts::actor_index::ACTOR_INDEX_SCHEMA;
use sha2::Digest;
use tantivy::directory::Directory;
use tantivy::directory::Lock;
use tantivy::directory::MmapDirectory;

use super::persist_manifest;
use super::reject_symlinks;
use super::Engine;
use super::EngineConfig;
use super::RootManifest;
use super::META_FORMAT;
use crate::IndexError;

pub(super) fn manifest_bytes(root: &Path) -> Result<Vec<u8>, IndexError> {
    let path = root.join("manifest.json");
    reject_symlinks(&path)?;
    let metadata = fs::symlink_metadata(&path).map_err(IndexError::storage)?;
    if !metadata.is_file() || metadata.len() > 64 * 1024 {
        return Err(IndexError::unavailable(
            "manifest_ownership_evidence_unavailable",
        ));
    }
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(IndexError::storage)?
        .take(64 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(IndexError::storage)?;
    if bytes.len() > 64 * 1024 {
        return Err(IndexError::unavailable(
            "manifest_ownership_evidence_unavailable",
        ));
    }
    Ok(bytes)
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", sha2::Sha256::digest(bytes))
}

pub(super) fn manifest_checksum(root: &Path) -> Result<String, IndexError> {
    Ok(digest(&manifest_bytes(root)?))
}

fn root(config: &EngineConfig) -> PathBuf {
    config
        .directory
        .join(digest(config.deployment_id.as_bytes()))
}

fn owned_manifest(config: &EngineConfig) -> Result<(RootManifest, Vec<u8>), IndexError> {
    let root = root(config);
    reject_symlinks(&root)?;
    let bytes = manifest_bytes(&root)?;
    let manifest: RootManifest = serde_json::from_slice(&bytes)
        .map_err(|_| IndexError::unavailable("manifest_ownership_evidence_unavailable"))?;
    if manifest.deployment_id != config.deployment_id
        || manifest.source_epoch != config.source_epoch
        || manifest.format != META_FORMAT
    {
        return Err(IndexError::conflict("recovery_source_identity_mismatch"));
    }
    for generation in std::iter::once(&manifest.active)
        .chain(manifest.pending.iter())
        .chain(manifest.retained.iter())
    {
        generation_id(generation)?;
    }
    if manifest.retained.len() > 64 {
        return Err(IndexError::unavailable(
            "retained_generation_limit_requires_review",
        ));
    }
    Ok((manifest, bytes))
}

fn generation_id(value: &str) -> Result<(), IndexError> {
    if value.len() > 32
        || value
            .strip_prefix('g')
            .is_none_or(|tail| tail.is_empty() || tail.parse::<u64>().is_err())
    {
        return Err(IndexError::invalid("invalid_generation_identity"));
    }
    Ok(())
}

impl Engine {
    /// Restart a damaged shadow while retaining the last verified serving
    /// reader. The failed directory is retained for explicit later review.
    pub fn repair_shadow(
        &self,
        request: &ActorIndexRepair,
    ) -> Result<ActorIndexStatus, IndexError> {
        request.validate().map_err(IndexError::protocol)?;
        if request.deployment_id != self.config.deployment_id
            || request.source_epoch != self.config.source_epoch
        {
            return Err(IndexError::conflict("recovery_source_identity_mismatch"));
        }
        let mut state = self
            .control
            .lock()
            .map_err(|_| IndexError::unavailable("writer_state_poisoned"))?;
        if state.manifest.repair_request.as_deref() == Some(request.request_id.as_str())
            && state.manifest.repair_checksum.as_deref()
                == Some(request.expected_manifest_checksum.as_str())
        {
            drop(state);
            return self.status();
        }
        if state.error.is_none() || state.manifest.pending.is_none() {
            return Err(IndexError::conflict("healthy_shadow_repair_not_allowed"));
        }
        if manifest_checksum(&self.root)? != request.expected_manifest_checksum
            || request.writer_epoch
                < state
                    .active
                    .committed
                    .writer_epoch
                    .max(state.manifest.pending_writer_epoch.unwrap_or_default())
        {
            return Err(IndexError::conflict("recovery_manifest_or_epoch_changed"));
        }
        let mut manifest = state.manifest.clone();
        let failed = manifest
            .pending
            .take()
            .ok_or_else(|| IndexError::conflict("shadow_missing"))?;
        if !manifest.retained.contains(&failed) {
            manifest.retained.push(failed);
        }
        if manifest.retained.len() > 64 {
            return Err(IndexError::unavailable(
                "retained_generation_limit_requires_review",
            ));
        }
        manifest.pending_request = None;
        manifest.pending_checksum = None;
        manifest.pending_checkpoint = None;
        manifest.pending_writer_epoch = None;
        manifest.repair_request = Some(request.request_id.clone());
        manifest.repair_checksum = Some(request.expected_manifest_checksum.clone());
        drop(state.pending.take());
        drop(state.active.writer.take());
        state.active.writer = Some(super::writer(&state.active.index, &self.config)?);
        let mut meta = state.active.committed.clone();
        meta.writer_epoch = request.writer_epoch;
        if let Err(error) = state.active.commit(meta) {
            state.active.writer = None;
            state.error = Some("shadow_generation_unavailable");
            return Err(error);
        }
        if let Err(error) = persist_manifest(&self.root, &manifest) {
            state.active.writer = None;
            state.error = Some("shadow_generation_unavailable");
            return Err(error);
        }
        state.manifest = manifest;
        state.error = None;
        self.publish(&state.active)?;
        drop(state);
        self.status()
    }

    /// Read-only diagnosis after normal open fails. A damaged/foreign manifest
    /// cannot prove ownership and therefore never enables online destruction.
    pub fn recovery_status(config: &EngineConfig, reason: &'static str) -> ActorIndexStatus {
        let evidence = owned_manifest(config).ok();
        ActorIndexStatus {
            protocol: ACTOR_INDEX_PROTOCOL,
            deployment_id: config.deployment_id.clone(),
            source_epoch: config.source_epoch.clone(),
            schema_version: ACTOR_INDEX_SCHEMA,
            analyzer_version: ACTOR_INDEX_ANALYZER,
            tantivy_version: "0.27.0".into(),
            index_format_version: tantivy::INDEX_FORMAT_VERSION,
            active: None,
            rebuilding: None,
            retained_generations: evidence
                .as_ref()
                .map(|(manifest, _)| manifest.retained.len())
                .unwrap_or_default(),
            retained_generation_ids: evidence
                .as_ref()
                .map(|(manifest, _)| manifest.retained.clone())
                .unwrap_or_default(),
            manifest_checksum: evidence.as_ref().map(|(_, bytes)| digest(bytes)),
            recovery_available: evidence.is_some(),
            limits: ActorIndexLimits::default(),
            pressure: None,
            state: "unavailable".into(),
            reason_code: Some(reason.into()),
        }
    }

    /// Only the private transport calls this when no healthy Engine exists.
    /// Preserve old generations and original manifest bytes, then create an
    /// empty current-schema baseline. It serves no successful empty queries;
    /// the separately admitted DB rebuild must reach its visible barrier.
    pub fn repair(config: &EngineConfig, request: &ActorIndexRepair) -> Result<Self, IndexError> {
        request.validate().map_err(IndexError::protocol)?;
        if request.deployment_id != config.deployment_id
            || request.source_epoch != config.source_epoch
        {
            return Err(IndexError::conflict("recovery_source_identity_mismatch"));
        }
        let root = root(config);
        reject_symlinks(&root)?;
        let directory = MmapDirectory::open(&root).map_err(IndexError::storage)?;
        let ownership = directory
            .acquire_lock(&Lock {
                filepath: "fleetworks-writer.lock".into(),
                is_blocking: false,
            })
            .map_err(|_| IndexError::unavailable("index_writer_locked"))?;
        let (previous, bytes) = owned_manifest(config)?;
        if previous.repair_request.as_deref() == Some(request.request_id.as_str())
            && previous.repair_checksum.as_deref()
                == Some(request.expected_manifest_checksum.as_str())
        {
            drop(ownership);
            return Self::open(config.clone());
        }
        if digest(&bytes) != request.expected_manifest_checksum {
            return Err(IndexError::conflict("recovery_manifest_changed"));
        }
        let mut retained = previous.retained.clone();
        retained.push(previous.active.clone());
        retained.extend(previous.pending.clone());
        retained.sort();
        retained.dedup();
        if retained.len() > 64 {
            return Err(IndexError::unavailable(
                "retained_generation_limit_requires_review",
            ));
        }
        // A prior interrupted repair may leave its exact empty candidate. It
        // is never removed/reused by guessing; advancing the unused identity
        // retains the orphan for explicit offline review.
        let mut next = previous.next_generation;
        let active = loop {
            let id = format!("g{next}");
            next = next
                .checked_add(1)
                .ok_or_else(|| IndexError::unavailable("generation_exhausted"))?;
            if !root.join(&id).try_exists().map_err(IndexError::storage)? {
                break id;
            }
            if next.saturating_sub(previous.next_generation) > 64 {
                return Err(IndexError::unavailable("recovery_orphan_review_required"));
            }
        };
        let proof = root.join(format!(
            "recovery-proof-{}.json",
            request.expected_manifest_checksum
        ));
        if !proof.try_exists().map_err(IndexError::storage)? {
            let mut file = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&proof)
                .map_err(IndexError::storage)?;
            file.write_all(&bytes).map_err(IndexError::storage)?;
            file.sync_all().map_err(IndexError::storage)?;
        } else {
            reject_symlinks(&proof)?;
            let metadata = fs::symlink_metadata(&proof).map_err(IndexError::storage)?;
            if !metadata.is_file()
                || metadata.len() > 64 * 1024
                || fs::read(&proof).map_err(IndexError::storage)? != bytes
            {
                return Err(IndexError::conflict("recovery_proof_conflict"));
            }
        }
        let generation = super::Generation::open(
            config,
            &root.join(&active),
            super::CommitMeta::new(config, active.clone(), 0, request.writer_epoch),
            true,
            true,
        )?;
        drop(generation);
        let manifest = RootManifest {
            format: META_FORMAT,
            deployment_id: config.deployment_id.clone(),
            source_epoch: config.source_epoch.clone(),
            schema_version: ACTOR_INDEX_SCHEMA,
            analyzer_version: ACTOR_INDEX_ANALYZER,
            tantivy_version: "0.27.0".into(),
            index_format_version: tantivy::INDEX_FORMAT_VERSION,
            active,
            pending: None,
            pending_request: None,
            pending_checksum: None,
            pending_checkpoint: None,
            pending_writer_epoch: None,
            retained,
            next_generation: next,
            repair_request: Some(request.request_id.clone()),
            repair_checksum: Some(request.expected_manifest_checksum.clone()),
            retiring: None,
            retired_requests: Vec::new(),
        };
        persist_manifest(&root, &manifest)?;
        drop(ownership);
        Self::open(config.clone())
    }

    /// Private transport reserves all query slots during explicit retirement.
    /// Only a manifest-listed inactive generation is eligible; file validation
    /// refuses links/subdirectories/unbounded entries before any unlink occurs.
    pub fn retire(&self, request: &ActorIndexRetire) -> Result<ActorIndexStatus, IndexError> {
        request.validate().map_err(IndexError::protocol)?;
        if request.deployment_id != self.config.deployment_id
            || request.source_epoch != self.config.source_epoch
        {
            return Err(IndexError::conflict("retirement_source_identity_mismatch"));
        }
        let mut state = self
            .control
            .lock()
            .map_err(|_| IndexError::unavailable("writer_state_poisoned"))?;
        let identity = (
            &request.request_id,
            &request.generation,
            &request.expected_manifest_checksum,
        );
        let checksum = digest(&serde_json::to_vec(&identity).map_err(IndexError::storage)?);
        if state
            .manifest
            .retired_requests
            .iter()
            .any(|receipt| receipt == &checksum)
        {
            drop(state);
            return self.status();
        }
        if state.manifest.active != request.expected_generation
            || state.manifest.active == request.generation
            || state.manifest.pending.as_deref() == Some(request.generation.as_str())
            || request.writer_epoch < state.active.visible.writer_epoch
            || !state.manifest.retained.contains(&request.generation)
        {
            return Err(IndexError::conflict("retirement_generation_changed"));
        }
        let manifest_digest = digest(&manifest_bytes(&self.root)?);
        if state.manifest.retiring.as_deref() != Some(checksum.as_str())
            && manifest_digest != request.expected_manifest_checksum
        {
            return Err(IndexError::conflict("retirement_manifest_changed"));
        }
        let target = self.root.join(&request.generation);
        reject_symlinks(&target)?;
        let mut files = Vec::new();
        if target.try_exists().map_err(IndexError::storage)? {
            for entry in fs::read_dir(&target).map_err(IndexError::storage)? {
                let entry = entry.map_err(IndexError::storage)?;
                let metadata = fs::symlink_metadata(entry.path()).map_err(IndexError::storage)?;
                if !metadata.is_file() || metadata.file_type().is_symlink() || files.len() >= 8192 {
                    return Err(IndexError::unavailable(
                        "retirement_unexpected_generation_entries",
                    ));
                }
                files.push(entry.path());
            }
        }
        let mut manifest = state.manifest.clone();
        manifest.retiring = Some(checksum.clone());
        persist_manifest(&self.root, &manifest)?;
        state.manifest = manifest.clone();
        for path in files {
            fs::remove_file(path).map_err(IndexError::storage)?;
        }
        if target.try_exists().map_err(IndexError::storage)? {
            fs::remove_dir(&target).map_err(IndexError::storage)?;
        }
        manifest
            .retained
            .retain(|generation| generation != &request.generation);
        manifest.retiring = None;
        manifest.retired_requests.push(checksum);
        if manifest.retired_requests.len() > 64 {
            manifest.retired_requests.remove(0);
        }
        persist_manifest(&self.root, &manifest)?;
        state.manifest = manifest;
        drop(state);
        self.status()
    }
}
