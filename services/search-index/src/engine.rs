//! One serialized writer, immutable serving snapshots and durable fences.

use std::fs;
use std::fs::File;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::RwLock;

use fleetworks_contracts::actor_index::ActorIndexBatch;
use fleetworks_contracts::actor_index::ActorIndexGeneration;
use fleetworks_contracts::actor_index::ActorIndexPromote;
use fleetworks_contracts::actor_index::ActorIndexQuery;
use fleetworks_contracts::actor_index::ActorIndexQueryResponse;
use fleetworks_contracts::actor_index::ActorIndexRebuildStart;
use fleetworks_contracts::actor_index::ActorIndexReceipt;
use fleetworks_contracts::actor_index::ActorIndexStatus;
use fleetworks_contracts::actor_index::ACTOR_INDEX_ANALYZER;
use fleetworks_contracts::actor_index::ACTOR_INDEX_PROTOCOL;
use fleetworks_contracts::actor_index::ACTOR_INDEX_SCHEMA;
use serde::Deserialize;
use serde::Serialize;
use sha2::Digest;
use tantivy::directory::Directory;
use tantivy::directory::DirectoryLock;
use tantivy::directory::Lock;
use tantivy::directory::MmapDirectory;
use tantivy::indexer::IndexWriterOptions;
use tantivy::Index;
use tantivy::IndexReader;
use tantivy::IndexWriter;
use tantivy::ReloadPolicy;
use tantivy::Searcher;
use tantivy::TantivyDocument;
use tantivy::Term;

use crate::query;
use crate::schema::Fields;
use crate::IndexError;

const RECEIPT_WINDOW: usize = 64;
const META_FORMAT: u16 = 1;

#[derive(Clone, Debug)]
pub struct EngineConfig {
    pub directory: PathBuf,
    pub deployment_id: String,
    pub source_epoch: String,
    pub writer_memory_bytes: usize,
    pub writer_threads: usize,
    pub merge_threads: usize,
}

impl EngineConfig {
    pub fn validate(&self) -> Result<(), IndexError> {
        if !self.directory.is_absolute()
            || self.deployment_id.trim().is_empty()
            || self.source_epoch.trim().is_empty()
            || self.deployment_id.len() > 256
            || self.source_epoch.len() > 256
            || self.deployment_id.chars().any(char::is_control)
            || self.source_epoch.chars().any(char::is_control)
        {
            return Err(IndexError::invalid("invalid_index_identity_or_root"));
        }
        if !(1..=2).contains(&self.writer_threads)
            || self.merge_threads != 1
            || self.writer_memory_bytes < 32 * 1024 * 1024 * self.writer_threads
            || self.writer_memory_bytes > 256 * 1024 * 1024
        {
            return Err(IndexError::invalid("invalid_writer_budget"));
        }
        Ok(())
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RootManifest {
    format: u16,
    deployment_id: String,
    source_epoch: String,
    schema_version: u16,
    analyzer_version: u16,
    tantivy_version: String,
    index_format_version: u32,
    active: String,
    pending: Option<String>,
    pending_request: Option<String>,
    pending_checksum: Option<String>,
    #[serde(default)]
    pending_checkpoint: Option<u64>,
    #[serde(default)]
    pending_writer_epoch: Option<u64>,
    retained: Vec<String>,
    next_generation: u64,
    #[serde(default)]
    repair_request: Option<String>,
    #[serde(default)]
    repair_checksum: Option<String>,
    #[serde(default)]
    retiring: Option<String>,
    #[serde(default)]
    retired_requests: Vec<String>,
}

#[path = "recovery.rs"]
mod recovery;

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RecordedReceipt {
    batch_id: String,
    checksum: String,
    checkpoint: u64,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CommitMeta {
    format: u16,
    deployment_id: String,
    source_epoch: String,
    generation: String,
    schema_version: u16,
    analyzer_version: u16,
    tantivy_version: String,
    index_format_version: u32,
    checkpoint: u64,
    source_complete: bool,
    writer_epoch: u64,
    receipts: Vec<RecordedReceipt>,
}

impl CommitMeta {
    fn new(config: &EngineConfig, generation: String, checkpoint: u64, writer_epoch: u64) -> Self {
        Self {
            format: META_FORMAT,
            deployment_id: config.deployment_id.clone(),
            source_epoch: config.source_epoch.clone(),
            generation,
            schema_version: ACTOR_INDEX_SCHEMA,
            analyzer_version: ACTOR_INDEX_ANALYZER,
            tantivy_version: "0.27.0".into(),
            index_format_version: tantivy::INDEX_FORMAT_VERSION,
            checkpoint,
            source_complete: false,
            writer_epoch,
            receipts: Vec::new(),
        }
    }
    fn view(&self) -> ActorIndexGeneration {
        ActorIndexGeneration {
            generation: self.generation.clone(),
            committed_checkpoint: self.checkpoint,
            visible_checkpoint: self.checkpoint,
            source_complete: self.source_complete,
            writer_epoch: self.writer_epoch,
        }
    }
}

struct Generation {
    index: Index,
    reader: IndexReader,
    writer: Option<IndexWriter<TantivyDocument>>,
    committed: CommitMeta,
    visible: CommitMeta,
    recover: bool,
}

impl Generation {
    fn open(
        config: &EngineConfig,
        path: &Path,
        initial: CommitMeta,
        create: bool,
        writable: bool,
    ) -> Result<Self, IndexError> {
        reject_symlinks(path)?;
        let (schema, _) = Fields::schema();
        let index = if create {
            fs::create_dir(path).map_err(IndexError::storage)?;
            Index::create_in_dir(path, schema.clone()).map_err(IndexError::storage)?
        } else {
            Index::open_in_dir(path).map_err(IndexError::storage)?
        };
        if index.schema() != schema {
            return Err(IndexError::unavailable("index_schema_incompatible"));
        }
        let mut writer = if writable || create {
            Some(writer(&index, config)?)
        } else {
            None
        };
        if create {
            let metadata = serde_json::to_string(&initial).map_err(IndexError::storage)?;
            let mut prepared = writer
                .as_mut()
                .ok_or_else(|| IndexError::unavailable("writer_inactive"))?
                .prepare_commit()
                .map_err(IndexError::storage)?;
            prepared.set_payload(&metadata);
            prepared.commit().map_err(IndexError::storage)?;
        }
        let reader: IndexReader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()
            .map_err(IndexError::storage)?;
        reader.reload().map_err(IndexError::storage)?;
        let committed = metadata(&index, config, &initial.generation)?;
        if !writable {
            writer = None;
        }
        Ok(Self {
            index,
            reader,
            writer,
            visible: committed.clone(),
            committed,
            recover: false,
        })
    }

    fn reconcile(&mut self, config: &EngineConfig) -> Result<(), IndexError> {
        // Reopen the writer from the durable commit authority. Uncommitted
        // queue contents after a failed prepare/commit must not leak into a
        // different batch; never guess whether an ambiguous commit succeeded.
        self.writer = None;
        let committed = metadata(&self.index, config, &self.committed.generation)?;
        self.reader.reload().map_err(IndexError::storage)?;
        self.writer = Some(writer(&self.index, config)?);
        self.committed = committed.clone();
        self.visible = committed;
        self.recover = false;
        Ok(())
    }

    fn commit(&mut self, next: CommitMeta) -> Result<(), IndexError> {
        let encoded = serde_json::to_string(&next).map_err(IndexError::storage)?;
        let writer = self
            .writer
            .as_mut()
            .ok_or_else(|| IndexError::unavailable("writer_inactive"))?;
        let mut prepared = match writer.prepare_commit() {
            Ok(prepared) => prepared,
            Err(error) => {
                self.recover = true;
                return Err(IndexError::storage(error));
            }
        };
        prepared.set_payload(&encoded);
        if let Err(error) = prepared.commit() {
            self.recover = true;
            return Err(IndexError::storage(error));
        }
        self.committed = next;
        if let Err(error) = self.reader.reload() {
            self.recover = true;
            return Err(IndexError::storage(error));
        }
        self.visible = self.committed.clone();
        Ok(())
    }

    fn view(&self) -> ActorIndexGeneration {
        let mut view = self.visible.view();
        view.committed_checkpoint = self.committed.checkpoint;
        view
    }
}

struct Snapshot {
    searcher: Searcher,
    metadata: CommitMeta,
}
struct Control {
    manifest: RootManifest,
    active: Generation,
    pending: Option<Generation>,
    error: Option<&'static str>,
}

/// Blocking methods must run outside Tokio executors. HTTP wraps them in bounded
/// blocking slots; readers never wait on this writer/control mutex.
pub struct Engine {
    config: EngineConfig,
    root: PathBuf,
    fields: Fields,
    serving: RwLock<Arc<Snapshot>>,
    control: Mutex<Control>,
    _root_lock: DirectoryLock,
}

impl Engine {
    pub fn open(config: EngineConfig) -> Result<Self, IndexError> {
        config.validate()?;
        reject_symlinks(&config.directory)?;
        fs::create_dir_all(&config.directory).map_err(IndexError::storage)?;
        let scope = format!(
            "{:x}",
            sha2::Sha256::digest(config.deployment_id.as_bytes())
        );
        let root = config.directory.join(scope);
        fs::create_dir_all(&root).map_err(IndexError::storage)?;
        reject_symlinks(&root)?;
        let directory = MmapDirectory::open(&root).map_err(IndexError::storage)?;
        let root_lock = directory
            .acquire_lock(&Lock {
                filepath: "fleetworks-writer.lock".into(),
                is_blocking: false,
            })
            .map_err(|_| IndexError::unavailable("index_writer_locked"))?;
        let manifest_path = root.join("manifest.json");
        let (manifest, create) = if manifest_path.exists() {
            (
                serde_json::from_slice::<RootManifest>(&recovery::manifest_bytes(&root)?)
                    .map_err(IndexError::storage)?,
                false,
            )
        } else {
            (
                RootManifest {
                    format: META_FORMAT,
                    deployment_id: config.deployment_id.clone(),
                    source_epoch: config.source_epoch.clone(),
                    schema_version: ACTOR_INDEX_SCHEMA,
                    analyzer_version: ACTOR_INDEX_ANALYZER,
                    tantivy_version: "0.27.0".into(),
                    index_format_version: tantivy::INDEX_FORMAT_VERSION,
                    active: "g1".into(),
                    pending: None,
                    pending_request: None,
                    pending_checksum: None,
                    pending_checkpoint: None,
                    pending_writer_epoch: None,
                    retained: Vec::new(),
                    next_generation: 2,
                    repair_request: None,
                    repair_checksum: None,
                    retiring: None,
                    retired_requests: Vec::new(),
                },
                true,
            )
        };
        validate_manifest(&manifest, &config)?;
        let active = Generation::open(
            &config,
            &root.join(&manifest.active),
            CommitMeta::new(&config, manifest.active.clone(), 0, 0),
            create,
            manifest.pending.is_none(),
        )?;
        let pending_result = manifest
            .pending
            .as_ref()
            .map(|id| {
                let create_pending = !root.join(id).try_exists().map_err(IndexError::storage)?;
                if create_pending
                    && (manifest.pending_checkpoint.is_none()
                        || manifest.pending_writer_epoch.is_none())
                {
                    return Err(IndexError::unavailable("shadow_creation_evidence_missing"));
                }
                Generation::open(
                    &config,
                    &root.join(id),
                    CommitMeta::new(
                        &config,
                        id.clone(),
                        manifest.pending_checkpoint.unwrap_or_default(),
                        manifest.pending_writer_epoch.unwrap_or_default(),
                    ),
                    create_pending,
                    true,
                )
            })
            .transpose();
        let (pending, error) = match pending_result {
            Ok(pending) => (pending, None),
            Err(_) => (None, Some("shadow_generation_unavailable")),
        };
        if create {
            persist_manifest(&root, &manifest)?;
        }
        let snapshot = Snapshot {
            searcher: active.reader.searcher(),
            metadata: active.visible.clone(),
        };
        let (_, fields) = Fields::schema();
        Ok(Self {
            config,
            root,
            fields,
            serving: RwLock::new(Arc::new(snapshot)),
            control: Mutex::new(Control {
                manifest,
                active,
                pending,
                error,
            }),
            _root_lock: root_lock,
        })
    }

    pub fn status(&self) -> Result<ActorIndexStatus, IndexError> {
        let state = self
            .control
            .try_lock()
            .map_err(|_| IndexError::unavailable("writer_status_temporarily_unavailable"))?;
        let status = if state.error.is_some() {
            "degraded"
        } else if state.pending.is_some() {
            "rebuilding"
        } else if state.active.visible.source_complete {
            "ready"
        } else if state.active.visible.checkpoint == 0 {
            "not_built"
        } else {
            "indexing"
        };
        Ok(ActorIndexStatus {
            protocol: ACTOR_INDEX_PROTOCOL,
            deployment_id: self.config.deployment_id.clone(),
            source_epoch: self.config.source_epoch.clone(),
            schema_version: ACTOR_INDEX_SCHEMA,
            analyzer_version: ACTOR_INDEX_ANALYZER,
            tantivy_version: "0.27.0".into(),
            index_format_version: tantivy::INDEX_FORMAT_VERSION,
            active: Some(state.active.view()),
            rebuilding: state.pending.as_ref().map(Generation::view),
            retained_generations: state.manifest.retained.len(),
            retained_generation_ids: state.manifest.retained.clone(),
            manifest_checksum: Some(recovery::manifest_checksum(&self.root)?),
            recovery_available: state.error.is_some() && state.manifest.pending.is_some(),
            limits: fleetworks_contracts::actor_index::ActorIndexLimits {
                writer_memory_bytes: self.config.writer_memory_bytes as u64,
                writer_threads: self.config.writer_threads as u8,
                merge_threads: self.config.merge_threads as u8,
                ..Default::default()
            },
            pressure: None,
            state: status.into(),
            reason_code: state.error.map(str::to_owned),
        })
    }

    pub fn query(&self, request: &ActorIndexQuery) -> Result<ActorIndexQueryResponse, IndexError> {
        request.validate().map_err(IndexError::protocol)?;
        let snapshot = {
            let serving = self
                .serving
                .read()
                .map_err(|_| IndexError::unavailable("reader_state_poisoned"))?;
            Arc::clone(&serving)
        };
        if snapshot.metadata.checkpoint == 0 && !snapshot.metadata.source_complete {
            return Err(IndexError::unavailable("actor_index_not_built"));
        }
        let (hits, truncated) = query::search(&snapshot.searcher, &self.fields, request)?;
        let serving_state = match self.control.try_lock() {
            Ok(control) if control.error.is_some() => "degraded",
            Ok(control) if control.pending.is_some() => "rebuilding",
            Ok(control)
                if control.active.visible.generation != snapshot.metadata.generation
                    || control.active.visible.checkpoint != snapshot.metadata.checkpoint =>
            {
                "stale"
            }
            Ok(_) if snapshot.metadata.source_complete => "ready",
            Ok(_) => "indexing",
            Err(std::sync::TryLockError::WouldBlock) => "stale",
            Err(std::sync::TryLockError::Poisoned(_)) => {
                return Err(IndexError::unavailable("writer_state_poisoned"))
            }
        };
        Ok(ActorIndexQueryResponse {
            deployment_id: self.config.deployment_id.clone(),
            source_epoch: self.config.source_epoch.clone(),
            index_generation: snapshot.metadata.generation.clone(),
            visible_checkpoint: snapshot.metadata.checkpoint,
            source_complete: snapshot.metadata.source_complete,
            serving_state: serving_state.into(),
            hits,
            truncated,
        })
    }

    fn publish(&self, generation: &Generation) -> Result<(), IndexError> {
        let next = Snapshot {
            searcher: generation.reader.searcher(),
            metadata: generation.visible.clone(),
        };
        *self
            .serving
            .write()
            .map_err(|_| IndexError::unavailable("reader_state_poisoned"))? = Arc::new(next);
        Ok(())
    }

    pub fn apply(&self, batch: &ActorIndexBatch) -> Result<ActorIndexReceipt, IndexError> {
        batch.validate().map_err(IndexError::protocol)?;
        if batch.deployment_id != self.config.deployment_id
            || batch.source_epoch != self.config.source_epoch
        {
            return Err(IndexError::conflict("source_identity_mismatch"));
        }
        let checksum = batch.content_checksum().map_err(IndexError::protocol)?;
        let mut state = self
            .control
            .lock()
            .map_err(|_| IndexError::unavailable("writer_state_poisoned"))?;
        let Control {
            active,
            pending,
            error,
            ..
        } = &mut *state;
        let targets_pending = pending.is_some();
        let generation = if let Some(pending) = pending.as_mut() {
            pending
        } else {
            active
        };
        if generation.committed.generation != batch.index_generation {
            return Err(IndexError::conflict("target_generation_mismatch"));
        }
        if generation.writer.is_none() && !generation.recover {
            *error = Some("writer_inactive");
            return Err(IndexError::unavailable("writer_inactive"));
        }
        if generation.recover {
            generation.reconcile(&self.config)?;
        }
        if batch.writer_epoch < generation.committed.writer_epoch {
            return Err(IndexError::conflict("stale_writer_epoch"));
        }
        let replay = generation
            .committed
            .receipts
            .iter()
            .find(|receipt| receipt.batch_id == batch.batch_id);
        let replayed = if let Some(receipt) = replay {
            if receipt.checksum != checksum {
                return Err(IndexError::conflict("batch_identity_conflict"));
            }
            true
        } else {
            if batch.from_checkpoint != generation.committed.checkpoint {
                return Err(IndexError::conflict("checkpoint_gap_or_expired_receipt"));
            }
            let searcher = generation.reader.searcher();
            let mut documents = Vec::new();
            for mutation in &batch.mutations {
                if let Some(existing) = query::record(&searcher, &self.fields, &mutation.actor_id)?
                {
                    if mutation.revision < existing.revision {
                        return Err(IndexError::conflict("obsolete_projection_revision"));
                    }
                    if mutation.revision == existing.revision
                        && mutation.checksum != existing.checksum
                    {
                        return Err(IndexError::conflict("projection_revision_conflict"));
                    }
                }
                documents.push(self.fields.document(mutation)?);
            }
            let writer = generation
                .writer
                .as_mut()
                .ok_or_else(|| IndexError::unavailable("writer_inactive"))?;
            for (mutation, document) in batch.mutations.iter().zip(documents) {
                writer.delete_term(Term::from_field_text(
                    self.fields.actor_id,
                    &mutation.actor_id,
                ));
                if let Err(error) = writer.add_document(document) {
                    let _rollback = writer.rollback().map_err(IndexError::storage)?;
                    return Err(IndexError::storage(error));
                }
            }
            false
        };
        if !replayed || batch.writer_epoch > generation.committed.writer_epoch {
            let mut next = generation.committed.clone();
            next.writer_epoch = batch.writer_epoch;
            if !replayed {
                next.checkpoint = batch.through_checkpoint;
                next.source_complete = batch.source_complete;
                next.receipts.push(RecordedReceipt {
                    batch_id: batch.batch_id.clone(),
                    checksum: checksum.clone(),
                    checkpoint: batch.through_checkpoint,
                });
                if next.receipts.len() > RECEIPT_WINDOW {
                    next.receipts.remove(0);
                }
            }
            if let Err(failure) = generation.commit(next) {
                *error = Some(failure.code);
                return Err(failure);
            }
        }
        if !targets_pending {
            self.publish(generation)?;
        }
        let receipt = ActorIndexReceipt {
            batch_id: batch.batch_id.clone(),
            checksum,
            index_generation: batch.index_generation.clone(),
            committed_checkpoint: batch.through_checkpoint,
            visible_checkpoint: batch.through_checkpoint,
            writer_epoch: generation.visible.writer_epoch,
            replayed,
        };
        *error = None;
        Ok(receipt)
    }

    pub fn start_rebuild(
        &self,
        request: &ActorIndexRebuildStart,
    ) -> Result<ActorIndexGeneration, IndexError> {
        if request.deployment_id != self.config.deployment_id
            || request.source_epoch != self.config.source_epoch
        {
            return Err(IndexError::conflict("source_identity_mismatch"));
        }
        if request.request_id.trim().is_empty()
            || request.request_id.len() > 256
            || request.writer_epoch == 0
        {
            return Err(IndexError::invalid("invalid_rebuild_identity"));
        }
        let mut state = self
            .control
            .lock()
            .map_err(|_| IndexError::unavailable("writer_state_poisoned"))?;
        let request_bytes = serde_json::to_vec(&(
            &request.request_id,
            &request.expected_generation,
            request.source_checkpoint,
        ))
        .map_err(IndexError::storage)?;
        let request_checksum = format!("{:x}", sha2::Sha256::digest(request_bytes));
        if request.expected_generation != state.manifest.active {
            return Err(IndexError::conflict("active_generation_changed"));
        }
        if let Some(pending) = &state.pending {
            let same = state.manifest.pending_request.as_deref()
                == Some(request.request_id.as_str())
                && state.manifest.pending_checksum.as_deref() == Some(request_checksum.as_str())
                && request.writer_epoch >= pending.committed.writer_epoch;
            if !same {
                return Err(IndexError::conflict("rebuild_already_in_progress"));
            }
            let pending = state
                .pending
                .as_mut()
                .ok_or_else(|| IndexError::unavailable("shadow_generation_unavailable"))?;
            if request.writer_epoch > pending.committed.writer_epoch {
                let mut meta = pending.committed.clone();
                meta.writer_epoch = request.writer_epoch;
                pending.commit(meta)?;
            }
            return Ok(pending.view());
        }
        if state.manifest.pending.is_some() {
            return Err(IndexError::conflict("shadow_requires_explicit_repair"));
        }
        if state.manifest.retained.len() >= 64 {
            return Err(IndexError::conflict(
                "retained_generation_limit_requires_review",
            ));
        }
        if request.writer_epoch < state.active.committed.writer_epoch {
            return Err(IndexError::conflict("stale_writer_epoch"));
        }
        // Only one generation owns writer/merge memory. The previous reader
        // remains serving while all source changes catch up in the shadow.
        state.active.writer = None;
        let id = format!("g{}", state.manifest.next_generation);
        let mut manifest = state.manifest.clone();
        manifest.next_generation = manifest
            .next_generation
            .checked_add(1)
            .ok_or_else(|| IndexError::unavailable("generation_exhausted"))?;
        manifest.pending = Some(id.clone());
        manifest.pending_request = Some(request.request_id.clone());
        manifest.pending_checksum = Some(request_checksum);
        manifest.pending_checkpoint = Some(request.source_checkpoint);
        manifest.pending_writer_epoch = Some(request.writer_epoch);
        if let Err(error) = persist_manifest(&self.root, &manifest) {
            state.active.writer = writer(&state.active.index, &self.config).ok();
            return Err(error);
        }
        state.manifest = manifest.clone();
        let pending = match Generation::open(
            &self.config,
            &self.root.join(&id),
            CommitMeta::new(
                &self.config,
                id.clone(),
                request.source_checkpoint,
                request.writer_epoch,
            ),
            true,
            true,
        ) {
            Ok(pending) => pending,
            Err(error) => {
                state.error = Some("shadow_generation_unavailable");
                return Err(error);
            }
        };
        let view = pending.view();
        state.manifest = manifest;
        state.pending = Some(pending);
        Ok(view)
    }

    pub fn promote(&self, request: &ActorIndexPromote) -> Result<ActorIndexGeneration, IndexError> {
        if request.deployment_id != self.config.deployment_id
            || request.source_epoch != self.config.source_epoch
        {
            return Err(IndexError::conflict("source_identity_mismatch"));
        }
        if request.writer_epoch == 0 {
            return Err(IndexError::invalid("writer_epoch_required"));
        }
        let mut state = self
            .control
            .lock()
            .map_err(|_| IndexError::unavailable("writer_state_poisoned"))?;
        if state.manifest.active == request.replacement_generation
            && state.pending.is_none()
            && state.active.visible.checkpoint >= request.catchup_checkpoint
        {
            if request.writer_epoch < state.active.visible.writer_epoch {
                return Err(IndexError::conflict("stale_writer_epoch"));
            }
            if request.writer_epoch > state.active.visible.writer_epoch {
                let mut next = state.active.committed.clone();
                next.writer_epoch = request.writer_epoch;
                state.active.commit(next)?;
                self.publish(&state.active)?;
            }
            return Ok(state.active.view());
        }
        if state.manifest.active != request.expected_generation {
            return Err(IndexError::conflict("active_generation_changed"));
        }
        let pending = state
            .pending
            .as_ref()
            .ok_or_else(|| IndexError::conflict("no_replacement_generation"))?;
        if pending.visible.generation != request.replacement_generation
            || !pending.visible.source_complete
            || pending.recover
            || pending.visible.checkpoint != request.catchup_checkpoint
            || pending.visible.checkpoint < state.active.visible.checkpoint
            || request.writer_epoch < pending.visible.writer_epoch
        {
            return Err(IndexError::conflict("replacement_not_verified_at_barrier"));
        }
        let pending = state
            .pending
            .as_mut()
            .ok_or_else(|| IndexError::unavailable("replacement_state_missing"))?;
        if request.writer_epoch > pending.visible.writer_epoch {
            let mut next = pending.committed.clone();
            next.writer_epoch = request.writer_epoch;
            pending.commit(next)?;
        }
        let mut manifest = state.manifest.clone();
        manifest.retained.push(manifest.active.clone());
        manifest.active = request.replacement_generation.clone();
        manifest.pending = None;
        manifest.pending_request = None;
        manifest.pending_checksum = None;
        manifest.pending_checkpoint = None;
        manifest.pending_writer_epoch = None;
        persist_manifest(&self.root, &manifest)?;
        let replacement = state
            .pending
            .take()
            .ok_or_else(|| IndexError::unavailable("replacement_state_missing"))?;
        state.active = replacement;
        state.manifest = manifest;
        self.publish(&state.active)?;
        Ok(state.active.view())
    }
}

fn writer(
    index: &Index,
    config: &EngineConfig,
) -> Result<IndexWriter<TantivyDocument>, IndexError> {
    let options = IndexWriterOptions::builder()
        .num_worker_threads(config.writer_threads)
        .num_merge_threads(config.merge_threads)
        .memory_budget_per_thread(config.writer_memory_bytes / config.writer_threads)
        .build();
    index
        .writer_with_options(options)
        .map_err(IndexError::storage)
}

fn metadata(
    index: &Index,
    config: &EngineConfig,
    generation: &str,
) -> Result<CommitMeta, IndexError> {
    let payload = index
        .load_metas()
        .map_err(IndexError::storage)?
        .payload
        .ok_or_else(|| IndexError::unavailable("missing_commit_fence"))?;
    let metadata: CommitMeta = serde_json::from_str(&payload).map_err(IndexError::storage)?;
    if metadata.format != META_FORMAT
        || metadata.deployment_id != config.deployment_id
        || metadata.source_epoch != config.source_epoch
        || metadata.generation != generation
        || metadata.schema_version != ACTOR_INDEX_SCHEMA
        || metadata.analyzer_version != ACTOR_INDEX_ANALYZER
        || metadata.tantivy_version != "0.27.0"
        || metadata.index_format_version != tantivy::INDEX_FORMAT_VERSION
        || metadata.receipts.len() > RECEIPT_WINDOW
    {
        return Err(IndexError::unavailable("commit_identity_incompatible"));
    }
    Ok(metadata)
}

fn validate_manifest(manifest: &RootManifest, config: &EngineConfig) -> Result<(), IndexError> {
    if manifest.format != META_FORMAT
        || manifest.deployment_id != config.deployment_id
        || manifest.source_epoch != config.source_epoch
        || manifest.schema_version != ACTOR_INDEX_SCHEMA
        || manifest.analyzer_version != ACTOR_INDEX_ANALYZER
        || manifest.retained.len() > 64
        || manifest.retired_requests.len() > 64
    {
        return Err(IndexError::unavailable("index_identity_incompatible"));
    }
    if manifest.tantivy_version != "0.27.0"
        || manifest.index_format_version != tantivy::INDEX_FORMAT_VERSION
    {
        return Err(IndexError::unavailable("index_engine_format_incompatible"));
    }
    let mut ids = std::collections::BTreeSet::new();
    for id in std::iter::once(&manifest.active)
        .chain(manifest.pending.iter())
        .chain(manifest.retained.iter())
    {
        if !id.starts_with('g')
            || id.len() > 24
            || id[1..].parse::<u64>().is_err()
            || !ids.insert(id)
        {
            return Err(IndexError::unavailable("invalid_generation_manifest"));
        }
    }
    Ok(())
}

fn persist_manifest(root: &Path, manifest: &RootManifest) -> Result<(), IndexError> {
    let temporary = root.join(format!(".manifest-{}.tmp", std::process::id()));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .map_err(IndexError::storage)?;
    file.write_all(&serde_json::to_vec(manifest).map_err(IndexError::storage)?)
        .map_err(IndexError::storage)?;
    file.sync_all().map_err(IndexError::storage)?;
    fs::rename(&temporary, root.join("manifest.json")).map_err(IndexError::storage)?;
    File::open(root)
        .map_err(IndexError::storage)?
        .sync_all()
        .map_err(IndexError::storage)?;
    Ok(())
}

fn reject_symlinks(path: &Path) -> Result<(), IndexError> {
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(IndexError::invalid("index_root_symlink"))
            }
            Ok(_) => (),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => return Err(IndexError::storage(error)),
        }
    }
    Ok(())
}
