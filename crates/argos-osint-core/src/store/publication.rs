//! Transactional Atlas publication, durable index outbox, revision-aware
//! indexing and exact coverage verification (spec §3.2–§3.3).
//!
//! One SQLite transaction upserts canonical memories and claim/source links,
//! writes index-outbox rows for every missing or stale memory revision and
//! persists a structured [`PublicationReceipt`]. Embedding and Lance writes run
//! only after commit, through leased index tasks; [`Store::try_index_memory`]
//! verifies the exact memory id/revision in the serving index before it
//! acknowledges work. Deletions leave tombstones so retries and repair cannot
//! resurrect intentionally removed memories.

use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::{AtlasInsightClaim, Store};
use crate::brain::MemorySource;
use crate::tasks::{self, IndexEnqueue, IndexOutcome};

/// Record kind used for memory rows in the index outbox.
pub const MEMORY_RECORD: &str = "memory";
/// Revision recorded for removal work of a deleted memory.
pub const DELETED_REVISION: &str = "deleted";

/// Test-only fault injection for the publication rollback path.
#[cfg(test)]
pub(crate) mod fault {
    use std::cell::RefCell;

    thread_local! {
        static FAIL_PUBLISH: RefCell<Option<String>> = const { RefCell::new(None) };
    }

    /// Make the next publications on this thread fail just before COMMIT
    /// (after rows were written), exercising the rollback path.
    pub(crate) fn fail_publish(message: Option<&str>) {
        FAIL_PUBLISH.with(|slot| *slot.borrow_mut() = message.map(str::to_string));
    }

    pub(crate) fn publish_fault() -> Option<String> {
        FAIL_PUBLISH.with(|slot| slot.borrow().clone())
    }
}

/// Stable revision of a memory's indexed content (SHA-256 of the text).
pub fn memory_revision(text: &str) -> String {
    crate::evidence::content_hash(text)
}

/// Why an extracted claim was not published. Invalid model output is reported,
/// never silently dropped.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RejectedClaim {
    /// Position in the extraction input.
    pub index: usize,
    pub article_id: String,
    pub reason: String,
}

/// Structured result of one Atlas publication. Extraction counts live elsewhere;
/// these are persistence counts.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicationReceipt {
    pub run_id: String,
    pub revision: String,
    /// Claims handed to the publisher (validated extraction output).
    pub input_claims: usize,
    /// Claims accepted for publication (input minus rejected).
    pub accepted_claims: usize,
    /// Accepted claims that created a new canonical memory.
    pub created: usize,
    /// Accepted claims that reused an existing canonical memory (deduplicated).
    pub reused: usize,
    /// Accepted claims whose canonical memory row was missing and was restored
    /// from the authoritative claim text in this input.
    pub repaired: usize,
    pub created_memory_ids: Vec<String>,
    pub reused_memory_ids: Vec<String>,
    pub repaired_memory_ids: Vec<String>,
    /// Memories whose text changed (e.g. a regenerated cycle brief).
    pub updated_memory_ids: Vec<String>,
    pub rejected: Vec<RejectedClaim>,
    pub brief_memory_id: Option<String>,
    /// `(fingerprint, canonical memory id)` per accepted claim, input order.
    pub claim_memory_ids: Vec<(String, String)>,
    /// Every memory this run requires (claims + brief), deduplicated.
    pub affected_memory_ids: Vec<String>,
    /// Outbox rows queued (new or coalesced) for missing/stale index revisions.
    pub queued_index_changes: Vec<i64>,
    pub index_tasks: Vec<String>,
    /// Required memories whose current revision was already recorded as indexed.
    pub already_indexed: usize,
}

impl PublicationReceipt {
    /// Created + reused + repaired accepted claims account for the accepted set
    /// (the brief is counted separately).
    pub fn reconciles(&self) -> bool {
        self.created + self.reused + self.repaired == self.accepted_claims
            && self.claim_memory_ids.len() == self.accepted_claims
            && self.accepted_claims + self.rejected.len() == self.input_claims
    }
}

/// Options for [`Store::publish_atlas_insights`].
#[derive(Clone, Debug, Default)]
pub struct PublishOptions {
    /// Extraction revision/checkpoint id. Defaults to a hash of the payload, so
    /// a retry of the same payload is idempotent.
    pub revision: Option<String>,
    /// Parent job for queued index tasks (Jobs tree). None uses the index service job.
    pub parent_job: Option<String>,
    /// Repair/reconciliation: suppress every tombstoned fingerprint, not only
    /// those deleted from this run.
    pub repair: bool,
    /// Attempt the queued index work right after commit (still acknowledged
    /// through the same leased outbox tasks).
    pub index_now: bool,
    /// Provenance tag for a non-Atlas publisher (e.g. `intel-recon`): new
    /// memories and reused ones are tagged in the same transaction (see
    /// [`retag_source`]).
    pub retag_app: Option<String>,
    /// Do not persist an Atlas publication receipt (publishers that reuse the
    /// Atlas tables for another run kind must not shadow the cycle's receipt).
    pub skip_receipt: bool,
}

/// Tag a memory's provenance as touched by another app/run. Atlas provenance is
/// kept (with a reference suffix); anything else is re-attributed.
pub fn retag_source(mut source: MemorySource, app: &str, run_id: &str) -> MemorySource {
    if source.app == "atlas" {
        source.reference = Some(format!("atlas+{app}:{run_id}"));
    } else {
        source.app = app.into();
        source.conversation_id = run_id.into();
        source.reference = Some(run_id.into());
    }
    source
}

/// Typed "memories changed" notification derived from the durable commit
/// counter, so it is emitted only for committed changes and also observes
/// commits from other processes. Coalesces any number of commits per poll.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoriesChanged {
    pub seq: i64,
}

/// Polls [`Store::memories_changed_seq`] and yields [`MemoriesChanged`] when it
/// moved since the last poll.
#[derive(Clone, Debug, Default)]
pub struct MemoryChangeWatcher {
    seq: i64,
}

impl MemoryChangeWatcher {
    /// Start watching from the current committed sequence.
    pub fn new(store: &Store) -> Self {
        Self {
            seq: store.memories_changed_seq().unwrap_or(0),
        }
    }

    pub fn poll(&mut self, store: &Store) -> Option<MemoriesChanged> {
        let seq = store.memories_changed_seq().ok()?;
        if seq == self.seq {
            return None;
        }
        self.seq = seq;
        Some(MemoriesChanged { seq })
    }
}

/// Exact index coverage for a set of required memories.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoverageReport {
    pub required: usize,
    /// Required memory ids with no SQLite row.
    pub missing_rows: Vec<String>,
    /// Current revision recorded as indexed **and** id present in the serving index.
    pub indexed: Vec<String>,
    /// `(memory id, reason)` not yet verified.
    pub pending: Vec<(String, String)>,
    /// Semantic indexing is disabled for this store; nothing can be verified.
    pub disabled: bool,
}

impl CoverageReport {
    pub fn complete(&self) -> bool {
        !self.disabled && self.missing_rows.is_empty() && self.pending.is_empty()
    }
}

/// Phase-completion checks for one publication receipt (spec §3.3).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicationVerification {
    /// Accepted claims whose claim row or canonical memory row is missing/mismatched.
    pub missing_claim_memories: Vec<String>,
    /// Accepted fingerprints with no source link to this run, or run source links
    /// that resolve to no claim.
    pub broken_source_links: Vec<String>,
    /// False when a brief memory was produced but its row is gone.
    pub brief_ok: bool,
    pub reconciled: bool,
    pub coverage: CoverageReport,
}

impl PublicationVerification {
    /// Memories and links are all present (indexing may still be incomplete).
    pub fn memories_ok(&self) -> bool {
        self.missing_claim_memories.is_empty()
            && self.broken_source_links.is_empty()
            && self.brief_ok
            && self.reconciled
    }
}

/// Default publication revision: hash of the validated payload.
pub fn payload_revision(claims: &[AtlasInsightClaim], brief: &str) -> String {
    let mut parts: Vec<String> = claims
        .iter()
        .map(|c| {
            format!(
                "{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}",
                c.namespace.trim().to_ascii_lowercase(),
                c.entity.trim().to_ascii_lowercase(),
                c.predicate.trim().to_ascii_lowercase(),
                c.object.trim().to_ascii_lowercase(),
                c.article_id.trim(),
                c.claim.trim()
            )
        })
        .collect();
    parts.push(format!("brief\u{1f}{}", brief.trim()));
    crate::evidence::content_hash(&parts.join("\n"))[..24].to_string()
}

pub(crate) fn reject_reason(claim: &AtlasInsightClaim) -> Option<String> {
    let missing: Vec<&str> = [
        ("entity", claim.entity.trim().is_empty()),
        ("namespace", claim.namespace.trim().is_empty()),
        ("predicate", claim.predicate.trim().is_empty()),
        ("object", claim.object.trim().is_empty()),
        ("claim text", claim.claim.trim().is_empty()),
        ("article id", claim.article_id.trim().is_empty()),
    ]
    .iter()
    .filter(|(_, empty)| *empty)
    .map(|(name, _)| *name)
    .collect();
    if missing.is_empty() {
        None
    } else {
        Some(format!("missing {}", missing.join(", ")))
    }
}

impl Store {
    /// Durable memories-changed counter (committed changes only).
    pub fn memories_changed_seq(&self) -> Result<i64> {
        crate::storage::store_bridge::memories_changed_seq()
    }

    /// Optional immediate indexing after commit: claims exactly the queued
    /// outbox tasks and acknowledges them through the same typed path. Returns
    /// the outcomes of tasks this call could claim (others are left to the pool).
    pub fn index_now(&self, queued: &[IndexEnqueue]) -> Vec<IndexOutcome> {
        crate::storage::store_bridge::index_now(queued)
    }

    /// Apply one outbox row with a typed, revision-aware outcome.
    pub fn apply_index_work(&self, work: &tasks::IndexWork) -> IndexOutcome {
        match work.operation.as_str() {
            "index_upsert" | "upsert" => self.try_index_memory(&work.record_id, &work.revision),
            "index_remove" | "remove" => {
                self.try_index_remove_missing(std::slice::from_ref(&work.record_id))
            }
            "index_rebuild" | "rebuild" | "generation_rebuild" => {
                self.try_process_vector_rebuild(4)
            }
            other => IndexOutcome::PermanentFailure {
                message: format!("unsupported index operation `{other}`"),
            },
        }
    }

    /// Revision-aware upsert of one memory. Reads text + revision, embeds and
    /// writes outside any SQLite write lock, verifies the exact id in the serving
    /// index, then records the revision only if the memory still has it. A
    /// deleted memory has its vector removed (never resurrected); a revision that
    /// changed during the write returns `Pending` so the task re-runs on the
    /// newest text instead of acknowledging stale content. `expected` may be
    /// older than the current revision: a newer indexed revision supersedes it.
    pub fn try_index_memory(&self, memory_id: &str, _expected: &str) -> IndexOutcome {
        crate::storage::store_bridge::try_index_upsert(&[memory_id.to_string()])
    }

    /// Exact coverage for required memories: the current revision must be
    /// recorded as indexed **and** the id must be present in the serving index.
    /// Never inferred from top-k similarity.
    pub fn verify_memory_coverage(&self, ids: &[String]) -> Result<CoverageReport> {
        crate::storage::store_bridge::verify_memory_coverage(ids)
    }

    /// Re-queue index work for every pending/absent required memory (e.g. index
    /// rows lost behind the metadata). Returns the queued outbox rows.
    pub fn requeue_uncovered(
        &self,
        report: &CoverageReport,
        _parent_job: Option<&str>,
    ) -> Result<Vec<IndexEnqueue>> {
        crate::storage::store_bridge::requeue_uncovered(report)
    }

    /// Transaction-safe Atlas publisher (spec §3.1–§3.2).
    ///
    /// In one transaction: validate, reject (with reasons) invalid or
    /// tombstoned claims, upsert canonical memories and claim/source links by
    /// fingerprint, restore damaged memory rows from authoritative claim text,
    /// publish the cycle brief, queue index work for every missing or stale
    /// required revision (including reused claims), persist the receipt and bump
    /// the memories-changed counter. Embeddings/Lance run only after commit.
    pub fn publish_atlas_insights(
        &self,
        run_id: &str,
        claims: &[AtlasInsightClaim],
        relations: &[(String, String, String)],
        brief: &str,
        options: &PublishOptions,
    ) -> Result<PublicationReceipt> {
        crate::storage::store_bridge::publish_atlas_insights(
            run_id, claims, relations, brief, options,
        )
    }

    /// Latest stored receipt for a run (resume after restart).
    pub fn atlas_publication_receipt(&self, run_id: &str) -> Result<Option<PublicationReceipt>> {
        crate::storage::store_bridge::atlas_publication_receipt(run_id)
    }

    /// Phase-completion verification of one receipt (spec §3.3).
    pub fn verify_atlas_publication(
        &self,
        receipt: &PublicationReceipt,
    ) -> Result<PublicationVerification> {
        crate::storage::store_bridge::verify_atlas_publication(receipt)
    }

    /// Record a user-deletion tombstone for a memory (and its claim fingerprint,
    /// with the cycles it was linked to) inside the caller's transaction.
    pub(crate) fn tombstone_memory(&self, memory_id: &str, reason: &str) -> Result<()> {
        crate::storage::store_bridge::tombstone_memory(memory_id, reason)
    }

    /// True when a tombstone suppresses this memory id.
    pub fn is_tombstoned(&self, memory_id: &str) -> Result<bool> {
        crate::storage::store_bridge::is_tombstoned(memory_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claim(entity: &str, object: &str, article: &str) -> AtlasInsightClaim {
        AtlasInsightClaim {
            fingerprint: String::new(),
            entity: entity.into(),
            namespace: "org".into(),
            predicate: "located_in".into(),
            object: object.into(),
            topic: "geo".into(),
            claim: format!("{entity} is located in {object}."),
            classification: "fact".into(),
            confidence: 0.8,
            article_id: article.into(),
            source_url: format!("https://news.example/{article}"),
            published_at: "2026-10-05".into(),
            reliability: "B".into(),
            info_credibility: 2,
            admiralty: "B2".into(),
            rsp_status: String::new(),
        }
    }

    fn source() -> crate::brain::MemorySource {
        crate::brain::MemorySource {
            app: "test".into(),
            conversation_id: "c".into(),
            message_id: None,
            reference: None,
        }
    }
}
