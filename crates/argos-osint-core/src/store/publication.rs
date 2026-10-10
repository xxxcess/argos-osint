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

use std::collections::{BTreeSet, HashSet};

use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::{
    atlas_answer_id, atlas_brief_id, insight_fingerprint, new_id, AtlasInsightClaim, Store,
};
use crate::brain::MemorySource;
use crate::brain_lance;
use crate::tasks::{self, IndexEnqueue, IndexOutcome};

/// Record kind used for memory rows in the index outbox.
pub const MEMORY_RECORD: &str = "memory";
/// Revision recorded for removal work of a deleted memory.
pub const DELETED_REVISION: &str = "deleted";
const INLINE_LEASE_SECS: i64 = 60;

/// Test-only fault injection: runs right after the Lance write and before the
/// compare-and-record step of [`Store::try_index_memory`] on this thread.
#[cfg(test)]
pub(crate) mod fault {
    use std::cell::RefCell;

    type Hook = Box<dyn Fn(&rusqlite::Connection)>;

    thread_local! {
        static AFTER_WRITE: RefCell<Option<Hook>> = const { RefCell::new(None) };
    }

    pub(crate) fn set_after_write(hook: impl Fn(&rusqlite::Connection) + 'static) {
        AFTER_WRITE.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
    }

    pub(crate) fn clear() {
        AFTER_WRITE.with(|slot| *slot.borrow_mut() = None);
    }

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

    pub(crate) fn fire(conn: &rusqlite::Connection) {
        AFTER_WRITE.with(|slot| {
            if let Some(hook) = slot.borrow().as_ref() {
                hook(conn);
            }
        });
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

/// Drop a run's checkpoints, publication receipts and repair status (run
/// deletion and retention). Plain statements; joins the caller's transaction.
pub(crate) fn delete_run_memory_state(conn: &Connection, run_id: &str) -> Result<()> {
    for table in [
        "argos_atlas_checkpoints",
        "argos_atlas_publications",
        "argos_atlas_repairs",
    ] {
        conn.execute(&format!("DELETE FROM {table} WHERE run_id=?1"), [run_id])?;
    }
    Ok(())
}

/// Current revision recorded as indexed for a memory.
pub(crate) fn indexed_revision(conn: &Connection, memory_id: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT revision FROM argos_memory_index_state WHERE memory_id=?1",
            [memory_id],
            |r| r.get(0),
        )
        .optional()?)
}

pub(crate) fn record_index_state(
    conn: &Connection,
    memory_id: &str,
    revision: &str,
    fingerprint: &str,
    generation: &str,
) -> Result<()> {
    conn.execute(
        "INSERT INTO argos_memory_index_state(memory_id,revision,fingerprint,generation,indexed_at)
         VALUES (?1,?2,?3,?4,?5)
         ON CONFLICT(memory_id) DO UPDATE SET revision=excluded.revision, fingerprint=excluded.fingerprint,
            generation=excluded.generation, indexed_at=excluded.indexed_at",
        params![memory_id, revision, fingerprint, generation, chrono::Utc::now().to_rfc3339()],
    )?;
    Ok(())
}

/// Record index state for many rows (rebuilds). Wraps its own transaction only
/// when the connection is not already inside one.
pub(crate) fn record_index_states(
    conn: &Connection,
    rows: &[(String, String)],
    generation: &str,
) -> Result<()> {
    let own_tx = conn.is_autocommit();
    if own_tx {
        conn.execute_batch("BEGIN IMMEDIATE")?;
    }
    let result = (|| -> Result<()> {
        let fp = brain_lance::current_fingerprint();
        for (id, text) in rows {
            record_index_state(conn, id, &memory_revision(text), &fp, generation)?;
        }
        Ok(())
    })();
    if own_tx {
        if result.is_ok() {
            conn.execute_batch("COMMIT")?;
        } else {
            let _ = conn.execute_batch("ROLLBACK");
        }
    }
    result
}

pub(crate) fn clear_index_state(conn: &Connection, memory_id: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM argos_memory_index_state WHERE memory_id=?1",
        [memory_id],
    )?;
    Ok(())
}

/// Bump the durable memories-changed counter (lets other processes/sessions
/// detect externally committed memory changes cheaply).
pub(crate) fn bump_memories_changed(conn: &Connection) -> Result<()> {
    conn.execute(
        "INSERT INTO app_state(key,value) VALUES ('memories_changed_seq','1')
         ON CONFLICT(key) DO UPDATE SET value=CAST(CAST(value AS INTEGER)+1 AS TEXT)",
        [],
    )?;
    Ok(())
}

fn memory_text(conn: &Connection, id: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row("SELECT text FROM memories WHERE id=?1", [id], |r| r.get(0))
        .optional()?)
}

/// Tombstoned fingerprint → run ids it was deleted from (`None` = any run).
pub(crate) fn tombstone_runs(conn: &Connection, fingerprint: &str) -> Result<Option<Vec<String>>> {
    let rows: Vec<String> = {
        let mut stmt = conn.prepare(
            "SELECT run_ids FROM argos_memory_tombstones WHERE fingerprint=?1 AND fingerprint<>''",
        )?;
        let rows = stmt
            .query_map([fingerprint], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    if rows.is_empty() {
        return Ok(None);
    }
    let mut runs = Vec::new();
    for json in rows {
        runs.extend(serde_json::from_str::<Vec<String>>(&json).unwrap_or_default());
    }
    Ok(Some(runs))
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

fn reject_reason(claim: &AtlasInsightClaim) -> Option<String> {
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

/// Queue index work for every memory whose current revision is not recorded as
/// indexed, and removal work for ids that no longer exist. Plain statements
/// only, so it joins the caller's transaction (including a
/// `rusqlite::Transaction`, which derefs to `Connection`).
pub(crate) fn enqueue_memory_index_on(
    conn: &Connection,
    ids: &[String],
    parent_job: Option<&str>,
) -> Result<(Vec<IndexEnqueue>, usize)> {
    let now = chrono::Utc::now().to_rfc3339();
    let mut queued = Vec::new();
    let mut already = 0usize;
    let mut seen = HashSet::new();
    for id in ids {
        if !seen.insert(id.as_str()) {
            continue;
        }
        match memory_text(conn, id)? {
            Some(text) => {
                let rev = memory_revision(&text);
                if indexed_revision(conn, id)?.as_deref() == Some(rev.as_str()) {
                    already += 1;
                    continue;
                }
                queued.push(tasks::enqueue_index_work(
                    conn,
                    MEMORY_RECORD,
                    id,
                    &rev,
                    &rev,
                    "index_upsert",
                    parent_job,
                    &now,
                )?);
            }
            None => {
                queued.push(tasks::enqueue_index_work(
                    conn,
                    MEMORY_RECORD,
                    id,
                    DELETED_REVISION,
                    "",
                    "index_remove",
                    parent_job,
                    &now,
                )?);
            }
        }
    }
    Ok((queued, already))
}

impl Store {
    /// Store-connection form of [`enqueue_memory_index_on`].
    pub(crate) fn enqueue_memory_index(
        &self,
        ids: &[String],
        parent_job: Option<&str>,
    ) -> Result<(Vec<IndexEnqueue>, usize)> {
        enqueue_memory_index_on(&self.conn, ids, parent_job)
    }

    /// Durable memories-changed counter (committed changes only).
    pub fn memories_changed_seq(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row(
                "SELECT CAST(value AS INTEGER) FROM app_state WHERE key='memories_changed_seq'",
                [],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(0))
    }

    /// Optional immediate indexing after commit: claims exactly the queued
    /// outbox tasks and acknowledges them through the same typed path. Returns
    /// the outcomes of tasks this call could claim (others are left to the pool).
    pub fn index_now(&self, queued: &[IndexEnqueue]) -> Vec<IndexOutcome> {
        if queued.is_empty() || self.vectors.is_none() || !self.conn.is_autocommit() {
            return Vec::new();
        }
        let owner = format!("inline-{}", crate::scheduler::process_owner());
        let mut outcomes = Vec::new();
        let mut seen = HashSet::new();
        for q in queued {
            if !seen.insert(q.task_id.clone()) {
                continue;
            }
            let now = chrono::Utc::now().to_rfc3339();
            let Ok(Some(claimed)) =
                tasks::claim_task(&self.conn, &q.task_id, &owner, INLINE_LEASE_SECS, &now)
            else {
                continue;
            };
            let work = match self.conn.query_row(
                "SELECT seq, task_id, record_kind, record_id, revision, content_hash, operation
                 FROM argos_index_changes WHERE task_id=?1",
                [&claimed.id],
                |r| {
                    Ok(tasks::IndexWork {
                        seq: r.get(0)?,
                        task_id: r.get(1)?,
                        record_kind: r.get(2)?,
                        record_id: r.get(3)?,
                        revision: r.get(4)?,
                        content_hash: r.get(5)?,
                        operation: r.get(6)?,
                    })
                },
            ) {
                Ok(work) => work,
                Err(_) => continue,
            };
            let _ = self.conn.execute(
                "UPDATE argos_index_changes SET state='running', updated_at=?1 WHERE seq=?2",
                params![now, work.seq],
            );
            let outcome = self.apply_index_work(&work);
            let done = chrono::Utc::now().to_rfc3339();
            let _ = tasks::finish_index_work(&self.conn, &claimed, &work, &outcome, &done);
            outcomes.push(outcome);
        }
        outcomes
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
    pub fn try_index_memory(&self, memory_id: &str, expected: &str) -> IndexOutcome {
        let _ = expected;
        if let Some(outcome) = self.index_unavailable() {
            return outcome;
        }
        if let Some(reason) = self.rebuild_in_progress() {
            return IndexOutcome::Pending { reason };
        }
        let Some(index) = self.vector_index() else {
            return IndexOutcome::RetryableFailure {
                message: brain_lance::last_error()
                    .unwrap_or_else(|| "vector index is not ready".into()),
            };
        };
        let fail = |err: anyhow::Error| {
            brain_lance::note_error(&err);
            index.mark_stale();
            IndexOutcome::RetryableFailure {
                message: format!("{err:#}"),
            }
        };
        let ids = [memory_id.to_string()];
        let text = match memory_text(&self.conn, memory_id) {
            Ok(text) => text,
            Err(err) => return fail(err),
        };
        let Some(text) = text else {
            return match index.remove_many(&ids) {
                Ok(()) => {
                    let _ = clear_index_state(&self.conn, memory_id);
                    IndexOutcome::Ready {
                        revision: DELETED_REVISION.into(),
                        fingerprint: brain_lance::current_fingerprint(),
                        generation: index.serving_table_name(),
                    }
                }
                Err(err) => fail(err),
            };
        };
        let rev = memory_revision(&text);
        let ready = |rev: String| IndexOutcome::Ready {
            revision: rev,
            fingerprint: brain_lance::current_fingerprint(),
            generation: index.serving_table_name(),
        };
        let present = match index.present_ids(&ids) {
            Ok(present) => present,
            Err(err) => return fail(err),
        };
        if present.contains(memory_id)
            && indexed_revision(&self.conn, memory_id)
                .ok()
                .flatten()
                .as_deref()
                == Some(rev.as_str())
        {
            return ready(rev);
        }
        if let Err(err) = index.upsert_texts(&[(memory_id.to_string(), text)]) {
            return fail(err);
        }
        #[cfg(test)]
        fault::fire(&self.conn);
        match index.present_ids(&ids) {
            Ok(present) if present.contains(memory_id) => {}
            Ok(_) => {
                return IndexOutcome::RetryableFailure {
                    message: format!(
                        "verification failed: {memory_id} absent from serving index after write"
                    ),
                }
            }
            Err(err) => return fail(err),
        }
        // Compare-and-record: only acknowledge the revision that was written.
        match memory_text(&self.conn, memory_id) {
            Ok(Some(now_text)) if memory_revision(&now_text) == rev => {
                match record_index_state(
                    &self.conn,
                    memory_id,
                    &rev,
                    &brain_lance::current_fingerprint(),
                    &index.serving_table_name(),
                ) {
                    Ok(()) => ready(rev),
                    Err(err) => fail(err),
                }
            }
            Ok(Some(_)) => IndexOutcome::Pending {
                reason: "memory changed during indexing; re-running on the newer revision".into(),
            },
            Ok(None) => match index.remove_many(&ids) {
                Ok(()) => {
                    let _ = clear_index_state(&self.conn, memory_id);
                    ready(DELETED_REVISION.into())
                }
                Err(err) => fail(err),
            },
            Err(err) => fail(err),
        }
    }

    /// Exact coverage for required memories: the current revision must be
    /// recorded as indexed **and** the id must be present in the serving index.
    /// Never inferred from top-k similarity.
    pub fn verify_memory_coverage(&self, ids: &[String]) -> Result<CoverageReport> {
        let unique: Vec<String> = ids
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let mut report = CoverageReport {
            required: unique.len(),
            ..Default::default()
        };
        let mut candidates = Vec::new();
        for id in &unique {
            match memory_text(&self.conn, id)? {
                None => report.missing_rows.push(id.clone()),
                Some(text) => candidates.push((id.clone(), memory_revision(&text))),
            }
        }
        let Some(index) = self.vectors.as_deref() else {
            report.disabled = true;
            report.pending = candidates
                .into_iter()
                .map(|(id, _)| (id, "semantic indexing disabled".to_string()))
                .collect();
            return Ok(report);
        };
        let present = index.present_ids(
            &candidates
                .iter()
                .map(|(id, _)| id.clone())
                .collect::<Vec<_>>(),
        )?;
        for (id, rev) in candidates {
            let recorded = indexed_revision(&self.conn, &id)?;
            match (
                recorded.as_deref() == Some(rev.as_str()),
                present.contains(&id),
            ) {
                (true, true) => report.indexed.push(id),
                (true, false) => report.pending.push((
                    id,
                    "recorded as indexed but absent from the serving index".into(),
                )),
                (false, _) if recorded.is_some() => report
                    .pending
                    .push((id, "an older revision is indexed".into())),
                _ => report.pending.push((id, "not indexed yet".into())),
            }
        }
        Ok(report)
    }

    /// Re-queue index work for every pending/absent required memory (e.g. index
    /// rows lost behind the metadata). Returns the queued outbox rows.
    pub fn requeue_uncovered(
        &self,
        report: &CoverageReport,
        parent_job: Option<&str>,
    ) -> Result<Vec<IndexEnqueue>> {
        let ids: Vec<String> = report.pending.iter().map(|(id, _)| id.clone()).collect();
        if ids.is_empty() || report.disabled {
            return Ok(Vec::new());
        }
        for id in &ids {
            clear_index_state(&self.conn, id)?;
        }
        Ok(self.enqueue_memory_index(&ids, parent_job)?.0)
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
        let revision = options
            .revision
            .clone()
            .unwrap_or_else(|| payload_revision(claims, brief));
        let answer_id = atlas_answer_id(run_id);
        let now = chrono::Utc::now().to_rfc3339();
        let mut receipt = PublicationReceipt {
            run_id: run_id.into(),
            revision: revision.clone(),
            input_claims: claims.len(),
            ..Default::default()
        };
        let mut queued_all: Vec<IndexEnqueue> = Vec::new();
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| -> Result<()> {
            let mut source = MemorySource {
                app: "atlas".into(),
                conversation_id: run_id.into(),
                message_id: None,
                reference: Some(run_id.into()),
            };
            if let Some(app) = &options.retag_app {
                source = retag_source(source, app, run_id);
            }
            let source_json = serde_json::to_string(&source)?;
            let mut retagged = 0usize;
            for (index, claim) in claims.iter().enumerate() {
                if let Some(reason) = reject_reason(claim) {
                    receipt.rejected.push(RejectedClaim {
                        index,
                        article_id: claim.article_id.trim().into(),
                        reason,
                    });
                    continue;
                }
                let entity = claim.entity.trim().to_ascii_lowercase();
                let namespace = claim.namespace.trim().to_ascii_lowercase();
                let predicate = claim.predicate.trim().to_ascii_lowercase();
                let object = claim.object.trim().to_ascii_lowercase();
                let sentence = claim.claim.trim();
                let fingerprint = insight_fingerprint(&namespace, &entity, &predicate, &object);
                let existing: Option<String> = self
                    .conn
                    .query_row(
                        "SELECT memory_id FROM insight_claims WHERE fingerprint=?1",
                        [&fingerprint],
                        |row| row.get(0),
                    )
                    .optional()?;
                if existing.is_none() {
                    if let Some(runs) = tombstone_runs(&self.conn, &fingerprint)? {
                        if options.repair || runs.iter().any(|r| r == run_id) {
                            receipt.rejected.push(RejectedClaim {
                                index,
                                article_id: claim.article_id.trim().into(),
                                reason: "deleted by user (tombstoned); not recreated".into(),
                            });
                            continue;
                        }
                    }
                }
                let memory_id = match existing {
                    Some(memory_id) => {
                        let row_exists = memory_text(&self.conn, &memory_id)?.is_some();
                        if row_exists {
                            if let Some(app) = &options.retag_app {
                                let current: String = self.conn.query_row(
                                    "SELECT source_json FROM memories WHERE id=?1",
                                    [&memory_id],
                                    |r| r.get(0),
                                )?;
                                let parsed: MemorySource = serde_json::from_str(&current)
                                    .unwrap_or_else(|_| MemorySource {
                                        app: app.clone(),
                                        conversation_id: run_id.into(),
                                        message_id: None,
                                        reference: Some(run_id.into()),
                                    });
                                let tagged =
                                    serde_json::to_string(&retag_source(parsed, app, run_id))?;
                                if tagged != current {
                                    self.conn.execute(
                                        "UPDATE memories SET source_json=?1 WHERE id=?2",
                                        params![tagged, memory_id],
                                    )?;
                                    retagged += 1;
                                }
                            }
                            receipt.reused += 1;
                            if !receipt.reused_memory_ids.contains(&memory_id)
                                && !receipt.created_memory_ids.contains(&memory_id)
                            {
                                receipt.reused_memory_ids.push(memory_id.clone());
                            }
                        } else {
                            // Damaged link from an earlier failure: restore the
                            // canonical row from this authoritative claim text.
                            self.conn.execute(
                                "INSERT INTO memories(id,text,category,pinned,created_at,source_json) VALUES (?1,?2,'investigation',0,?3,?4)",
                                params![memory_id, sentence, now, source_json],
                            )?;
                            receipt.repaired += 1;
                            receipt.repaired_memory_ids.push(memory_id.clone());
                        }
                        memory_id
                    }
                    None => {
                        let memory_id = new_id();
                        self.conn.execute(
                            "INSERT INTO memories(id,text,category,pinned,created_at,source_json) VALUES (?1,?2,'investigation',0,?3,?4)",
                            params![memory_id, sentence, now, source_json],
                        )?;
                        self.conn.execute("INSERT INTO memory_metadata (memory_id, memory_kind) VALUES (?1, 'atomic_claim') ON CONFLICT(memory_id) DO UPDATE SET memory_kind=excluded.memory_kind", params![memory_id])?;
                        self.conn.execute(
                            "INSERT INTO insight_claims(fingerprint,memory_id,entity_id,predicate,object_value,topic,classification,confidence,created_at,updated_at,source_reliability,info_credibility,admiralty,rsp_status) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?9,?10,?11,?12,?13)",
                            params![
                                fingerprint,
                                memory_id,
                                entity,
                                predicate,
                                object,
                                claim.topic.trim(),
                                claim.classification.trim(),
                                claim.confidence,
                                now,
                                claim.reliability.trim(),
                                claim.info_credibility as i64,
                                claim.admiralty.trim(),
                                claim.rsp_status.trim(),
                            ],
                        )?;
                        let others: Vec<String> = {
                            let mut stmt = self.conn.prepare(
                                "SELECT fingerprint FROM insight_claims WHERE entity_id=?1 AND predicate=?2 AND fingerprint<>?3",
                            )?;
                            let rows = stmt
                                .query_map(params![entity, predicate, fingerprint], |row| {
                                    row.get(0)
                                })?
                                .collect::<rusqlite::Result<_>>()?;
                            rows
                        };
                        for old in others {
                            self.conn.execute(
                                "INSERT OR IGNORE INTO insight_relations(left_fingerprint,right_fingerprint,relation) VALUES (?1,?2,'conflict_or_revision')",
                                params![old, fingerprint],
                            )?;
                        }
                        receipt.created += 1;
                        receipt.created_memory_ids.push(memory_id.clone());
                        memory_id
                    }
                };
                self.conn.execute(
                    "INSERT OR IGNORE INTO insight_sources(fingerprint,thread_id,run_id,answer_id,call_id,source_url,published_at) VALUES (?1,NULL,?2,?3,?4,?5,?6)",
                    params![
                        fingerprint,
                        run_id,
                        answer_id,
                        claim.article_id.trim(),
                        claim.source_url.trim(),
                        claim.published_at.trim(),
                    ],
                )?;
                self.conn.execute(
                    "UPDATE insight_sources SET published_at=?1 WHERE fingerprint=?2 AND answer_id=?3 AND call_id=?4",
                    params![claim.published_at.trim(), fingerprint, answer_id, claim.article_id.trim()],
                )?;
                // Preserve classification; refresh reliability/confidence only.
                self.conn.execute(
                    "UPDATE insight_claims SET confidence=?1, source_reliability=?2, info_credibility=?3, admiralty=?4, rsp_status=?5, updated_at=?6 WHERE fingerprint=?7",
                    params![
                        claim.confidence,
                        claim.reliability.trim(),
                        claim.info_credibility as i64,
                        claim.admiralty.trim(),
                        claim.rsp_status.trim(),
                        now,
                        fingerprint,
                    ],
                )?;
                receipt.accepted_claims += 1;
                receipt.claim_memory_ids.push((fingerprint, memory_id));
            }
            for (left, right, relation) in relations {
                if left.is_empty() || right.is_empty() || relation.is_empty() || left == right {
                    continue;
                }
                self.conn.execute(
                    "INSERT OR IGNORE INTO insight_relations(left_fingerprint,right_fingerprint,relation) VALUES (?1,?2,?3)",
                    params![left, right, relation],
                )?;
            }
            let brief = brief.trim();
            if !brief.is_empty() && receipt.accepted_claims > 0 {
                if let Some(memory_id) = atlas_brief_id(&self.conn, run_id)? {
                    let changed = self.conn.execute(
                        "UPDATE memories SET text=?1 WHERE id=?2 AND text<>?1",
                        params![brief, memory_id],
                    )?;
                    if changed > 0 {
                        receipt.updated_memory_ids.push(memory_id.clone());
                    }
                    receipt.brief_memory_id = Some(memory_id);
                } else {
                    let memory_id = new_id();
                    self.conn.execute(
                        "INSERT INTO memories(id,text,category,pinned,created_at,source_json) VALUES (?1,?2,'investigation',0,?3,?4)",
                        params![memory_id, brief, now, source_json],
                    )?;
                    receipt.brief_memory_id = Some(memory_id);
                }
            }
            let mut affected: Vec<String> = Vec::new();
            for (_, id) in &receipt.claim_memory_ids {
                if !affected.contains(id) {
                    affected.push(id.clone());
                }
            }
            if let Some(id) = &receipt.brief_memory_id {
                if !affected.contains(id) {
                    affected.push(id.clone());
                }
            }
            let (queued, already) =
                self.enqueue_memory_index(&affected, options.parent_job.as_deref())?;
            receipt.affected_memory_ids = affected;
            receipt.already_indexed = already;
            receipt.queued_index_changes = queued.iter().map(|q| q.seq).collect();
            receipt.index_tasks = queued.iter().map(|q| q.task_id.clone()).collect();
            queued_all = queued;
            if !options.skip_receipt {
                self.conn.execute(
                    "INSERT INTO argos_atlas_publications(run_id,revision,state,receipt_json,job_id,created_at,updated_at)
                     VALUES (?1,?2,'published',?3,?4,?5,?5)
                     ON CONFLICT(run_id,revision) DO UPDATE SET state='published', receipt_json=excluded.receipt_json,
                        job_id=excluded.job_id, updated_at=excluded.updated_at",
                    params![
                        run_id,
                        revision,
                        serde_json::to_string(&receipt)?,
                        options.parent_job.clone().unwrap_or_default(),
                        now
                    ],
                )?;
            }
            if retagged > 0
                || receipt.created + receipt.repaired > 0
                || !receipt.updated_memory_ids.is_empty()
                || receipt.brief_memory_id.is_some()
            {
                bump_memories_changed(&self.conn)?;
            }
            #[cfg(test)]
            if let Some(message) = fault::publish_fault() {
                anyhow::bail!(message);
            }
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.conn.execute_batch("COMMIT")?;
                if options.index_now {
                    let _ = self.index_now(&queued_all);
                }
                Ok(receipt)
            }
            Err(err) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(err)
            }
        }
    }

    /// Latest stored receipt for a run (resume after restart).
    pub fn atlas_publication_receipt(&self, run_id: &str) -> Result<Option<PublicationReceipt>> {
        let json: Option<String> = self
            .conn
            .query_row(
                "SELECT receipt_json FROM argos_atlas_publications WHERE run_id=?1 ORDER BY updated_at DESC LIMIT 1",
                [run_id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(match json {
            Some(json) => Some(serde_json::from_str(&json)?),
            None => None,
        })
    }

    /// Phase-completion verification of one receipt (spec §3.3).
    pub fn verify_atlas_publication(
        &self,
        receipt: &PublicationReceipt,
    ) -> Result<PublicationVerification> {
        let answer_id = atlas_answer_id(&receipt.run_id);
        let mut out = PublicationVerification {
            brief_ok: true,
            reconciled: receipt.reconciles(),
            ..Default::default()
        };
        for (fingerprint, memory_id) in &receipt.claim_memory_ids {
            let claim_memory: Option<String> = self
                .conn
                .query_row(
                    "SELECT memory_id FROM insight_claims WHERE fingerprint=?1",
                    [fingerprint],
                    |r| r.get(0),
                )
                .optional()?;
            let row_ok = memory_text(&self.conn, memory_id)?.is_some();
            if (claim_memory.as_deref() != Some(memory_id.as_str()) || !row_ok)
                && !out.missing_claim_memories.contains(memory_id)
            {
                out.missing_claim_memories.push(memory_id.clone());
            }
            let linked: i64 = self.conn.query_row(
                "SELECT COUNT(*) FROM insight_sources WHERE fingerprint=?1 AND run_id=?2 AND answer_id=?3",
                params![fingerprint, receipt.run_id, answer_id],
                |r| r.get(0),
            )?;
            if linked == 0 && !out.broken_source_links.contains(fingerprint) {
                out.broken_source_links.push(fingerprint.clone());
            }
        }
        let orphan_links: Vec<String> = {
            let mut stmt = self.conn.prepare(
                "SELECT DISTINCT s.fingerprint FROM insight_sources s
                 LEFT JOIN insight_claims c ON c.fingerprint = s.fingerprint
                 WHERE s.run_id=?1 AND s.answer_id=?2 AND c.fingerprint IS NULL",
            )?;
            let rows = stmt
                .query_map(params![receipt.run_id, answer_id], |r| r.get(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        for fp in orphan_links {
            if !out.broken_source_links.contains(&fp) {
                out.broken_source_links.push(fp);
            }
        }
        if let Some(brief) = &receipt.brief_memory_id {
            out.brief_ok = memory_text(&self.conn, brief)?.is_some();
        }
        out.coverage = self.verify_memory_coverage(&receipt.affected_memory_ids)?;
        Ok(out)
    }

    /// Record a user-deletion tombstone for a memory (and its claim fingerprint,
    /// with the cycles it was linked to) inside the caller's transaction.
    pub(crate) fn tombstone_memory(&self, memory_id: &str, reason: &str) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        let fingerprints: Vec<String> = {
            let mut stmt = self
                .conn
                .prepare("SELECT fingerprint FROM insight_claims WHERE memory_id=?1")?;
            let rows = stmt
                .query_map([memory_id], |r| r.get(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        if fingerprints.is_empty() {
            self.conn.execute(
                "INSERT OR REPLACE INTO argos_memory_tombstones(memory_id,fingerprint,reason,created_at,run_ids) VALUES (?1,'',?2,?3,'[]')",
                params![memory_id, reason, now],
            )?;
        }
        for fp in fingerprints {
            let runs: Vec<String> = {
                let mut stmt = self.conn.prepare(
                    "SELECT DISTINCT run_id FROM insight_sources WHERE fingerprint=?1 AND run_id IS NOT NULL",
                )?;
                let rows = stmt
                    .query_map([&fp], |r| r.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                rows
            };
            self.conn.execute(
                "INSERT OR REPLACE INTO argos_memory_tombstones(memory_id,fingerprint,reason,created_at,run_ids) VALUES (?1,?2,?3,?4,?5)",
                params![memory_id, fp, reason, now, serde_json::to_string(&runs)?],
            )?;
        }
        Ok(())
    }

    /// True when a tombstone suppresses this memory id.
    pub fn is_tombstoned(&self, memory_id: &str) -> Result<bool> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM argos_memory_tombstones WHERE memory_id=?1",
            [memory_id],
            |r| r.get(0),
        )?;
        Ok(n > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embed::testing;

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

    fn disk_store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("argos.db")).unwrap();
        (dir, store)
    }

    fn active_index_rows(store: &Store) -> i64 {
        store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM argos_index_changes WHERE state IN ('pending','running','blocked')",
                [],
                |r| r.get(0),
            )
            .unwrap()
    }

    #[test]
    fn publication_receipt_reconciles_and_reports_rejections() {
        let store = Store::memory().unwrap();
        let mut bad = claim("acme", "berlin", "a2");
        bad.predicate = "  ".into();
        let claims = vec![
            claim("acme", "paris", "a1"),
            bad,
            claim("acme", "paris", "a3"), // duplicate fingerprint from another article
        ];
        let receipt = store
            .publish_atlas_insights(
                "run-1",
                &claims,
                &[],
                "Cycle brief.",
                &PublishOptions::default(),
            )
            .unwrap();
        assert_eq!(receipt.input_claims, 3);
        assert_eq!(receipt.accepted_claims, 2);
        assert_eq!(receipt.rejected.len(), 1);
        assert_eq!(receipt.rejected[0].index, 1);
        assert!(
            receipt.rejected[0].reason.contains("predicate"),
            "{:?}",
            receipt.rejected
        );
        assert_eq!(
            (receipt.created, receipt.reused, receipt.repaired),
            (1, 1, 0)
        );
        assert!(receipt.reconciles());
        assert!(receipt.brief_memory_id.is_some());
        assert_eq!(receipt.affected_memory_ids.len(), 2, "claim memory + brief");
        assert_eq!(receipt.queued_index_changes.len(), 2);
        assert_eq!(store.list_memories().unwrap().len(), 2);
        // Both article links are kept for the deduplicated claim.
        let links: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM insight_sources WHERE run_id='run-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(links, 2);
        let stored = store.atlas_publication_receipt("run-1").unwrap().unwrap();
        assert_eq!(stored, receipt);
        let verification = store.verify_atlas_publication(&receipt).unwrap();
        assert!(verification.memories_ok(), "{verification:?}");
        assert!(
            verification.coverage.disabled,
            "in-memory store has no vectors"
        );
        assert!(!verification.coverage.complete());
    }

    #[test]
    fn retrying_the_same_payload_is_idempotent() {
        let store = Store::memory().unwrap();
        let claims = vec![claim("acme", "paris", "a1"), claim("globex", "rome", "a2")];
        let first = store
            .publish_atlas_insights("run-1", &claims, &[], "Brief.", &PublishOptions::default())
            .unwrap();
        let second = store
            .publish_atlas_insights("run-1", &claims, &[], "Brief.", &PublishOptions::default())
            .unwrap();
        assert_eq!(first.revision, second.revision);
        assert_eq!((second.created, second.reused), (0, 2));
        assert_eq!(store.list_memories().unwrap().len(), 3);
        assert_eq!(
            active_index_rows(&store),
            3,
            "no duplicate active index requests"
        );
        let receipts: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM argos_atlas_publications WHERE run_id='run-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(receipts, 1);
    }

    #[test]
    fn failed_publication_rolls_back_memories_links_and_outbox() {
        let store = Store::memory().unwrap();
        store
            .conn
            .execute_batch(
                "CREATE TRIGGER fail_receipt BEFORE INSERT ON argos_atlas_publications
                 BEGIN SELECT RAISE(ABORT, 'injected publication failure'); END;",
            )
            .unwrap();
        let err = store
            .publish_atlas_insights(
                "run-1",
                &[claim("acme", "paris", "a1")],
                &[],
                "Brief.",
                &PublishOptions::default(),
            )
            .unwrap_err();
        assert!(format!("{err:#}").contains("injected publication failure"));
        assert!(store.list_memories().unwrap().is_empty());
        assert_eq!(active_index_rows(&store), 0);
        let claims: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM insight_claims", [], |r| r.get(0))
            .unwrap();
        assert_eq!(claims, 0);
    }

    #[cfg(feature = "lancedb")]
    #[test]
    fn indexing_is_verified_by_exact_id_and_revision() {
        let _fake = testing::fake();
        let (_dir, store) = disk_store();
        let receipt = store
            .publish_atlas_insights(
                "run-1",
                &[claim("acme", "paris", "a1"), claim("globex", "rome", "a2")],
                &[],
                "Brief text.",
                &PublishOptions {
                    index_now: true,
                    ..Default::default()
                },
            )
            .unwrap();
        let report = store
            .verify_memory_coverage(&receipt.affected_memory_ids)
            .unwrap();
        assert!(report.complete(), "{report:?}");
        assert_eq!(report.indexed.len(), 3);
        // Lose a vector behind the metadata: exact lookup notices, top-k would not.
        let lost = receipt.claim_memory_ids[0].1.clone();
        store
            .vectors
            .as_deref()
            .unwrap()
            .remove_many(std::slice::from_ref(&lost))
            .unwrap();
        let report = store
            .verify_memory_coverage(&receipt.affected_memory_ids)
            .unwrap();
        assert_eq!(report.pending.len(), 1);
        assert_eq!(report.pending[0].0, lost);
        let queued = store.requeue_uncovered(&report, None).unwrap();
        assert_eq!(queued.len(), 1);
        store.index_now(&queued);
        assert!(store
            .verify_memory_coverage(&receipt.affected_memory_ids)
            .unwrap()
            .complete());
    }

    #[cfg(feature = "lancedb")]
    #[test]
    fn embedding_failure_keeps_memories_visible_and_retry_reaches_verified() {
        let _fake = testing::fake();
        let (_dir, store) = disk_store();
        let receipt = {
            let _fail = testing::fail();
            store
                .publish_atlas_insights(
                    "run-1",
                    &[claim("acme", "paris", "a1")],
                    &[],
                    "",
                    &PublishOptions {
                        index_now: true,
                        ..Default::default()
                    },
                )
                .unwrap()
        };
        assert_eq!(
            store.list_memories().unwrap().len(),
            1,
            "memories stay visible"
        );
        let report = store
            .verify_memory_coverage(&receipt.affected_memory_ids)
            .unwrap();
        assert!(!report.complete());
        let (state, category): (String, String) = store
            .conn
            .query_row(
                "SELECT state, error_category FROM argos_tasks WHERE id=?1",
                [&receipt.index_tasks[0]],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            (state.as_str(), category.as_str()),
            ("retry_scheduled", "index_failure")
        );
        store
            .conn
            .execute("UPDATE argos_tasks SET next_eligible_at=''", [])
            .unwrap();
        let n = crate::scheduler::drain_index_once(&store.conn, &store, "worker", 8).unwrap();
        assert!(n >= 1);
        assert!(store
            .verify_memory_coverage(&receipt.affected_memory_ids)
            .unwrap()
            .complete());
    }

    #[cfg(feature = "lancedb")]
    #[test]
    fn stale_work_cannot_overwrite_a_newer_revision() {
        let _fake = testing::fake();
        let (_dir, store) = disk_store();
        let memory = store
            .add_memory(
                "Harbor tanker manifests list cargo",
                "fact",
                false,
                crate::brain::MemorySource {
                    app: "test".into(),
                    conversation_id: "c".into(),
                    message_id: None,
                    reference: None,
                },
            )
            .unwrap();
        let old_rev = memory_revision(&memory.text);
        store
            .update_memory(&memory.id, "Northwind ferry timetable", "fact", false)
            .unwrap();
        // A stale worker now runs the old revision's work.
        let outcome = store.try_index_memory(&memory.id, &old_rev);
        match &outcome {
            IndexOutcome::Ready { revision, .. } => {
                assert_eq!(revision, &memory_revision("Northwind ferry timetable"))
            }
            other => panic!("{other:?}"),
        }
        let index = store.vectors.as_deref().unwrap();
        let hit = index.search("northwind ferry timetable", 1).unwrap();
        assert_eq!(hit[0].0, memory.id);
        assert!(hit[0].1 > 0.99, "newest text stays indexed: {hit:?}");
    }

    #[cfg(feature = "lancedb")]
    #[test]
    fn deletion_during_indexing_is_not_resurrected() {
        let _fake = testing::fake();
        let (_dir, store) = disk_store();
        let claims = vec![claim("acme", "paris", "a1")];
        let receipt = store
            .publish_atlas_insights(
                "run-1",
                &claims,
                &[],
                "",
                &PublishOptions {
                    index_now: true,
                    ..Default::default()
                },
            )
            .unwrap();
        let id = receipt.claim_memory_ids[0].1.clone();
        let rev = memory_revision("acme is located in paris.");
        assert!(store.delete_memory(&id).unwrap());
        assert!(store.is_tombstoned(&id).unwrap());
        // A late upsert for the deleted memory removes rather than resurrects.
        let outcome = store.try_index_memory(&id, &rev);
        assert!(
            matches!(outcome, IndexOutcome::Ready { ref revision, .. } if revision == DELETED_REVISION),
            "{outcome:?}"
        );
        assert!(!store
            .vectors
            .as_deref()
            .unwrap()
            .present_ids(std::slice::from_ref(&id))
            .unwrap()
            .contains(&id));
        // Retrying the same run cannot recreate the deleted claim.
        let retry = store
            .publish_atlas_insights("run-1", &claims, &[], "", &PublishOptions::default())
            .unwrap();
        assert_eq!(retry.accepted_claims, 0);
        assert_eq!(retry.rejected.len(), 1);
        assert!(retry.rejected[0].reason.contains("tombstoned"));
        assert!(store.list_memories().unwrap().is_empty());
        // A different, later cycle with new evidence may publish it again.
        let fresh = store
            .publish_atlas_insights("run-2", &claims, &[], "", &PublishOptions::default())
            .unwrap();
        assert_eq!(fresh.created, 1);
        // Repair mode suppresses every tombstoned fingerprint.
        store.delete_memory(&fresh.claim_memory_ids[0].1).unwrap();
        let repair = store
            .publish_atlas_insights(
                "run-3",
                &claims,
                &[],
                "",
                &PublishOptions {
                    repair: true,
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(repair.accepted_claims, 0);
    }

    #[cfg(feature = "lancedb")]
    #[test]
    fn reused_claims_with_damaged_rows_are_repaired_and_reindexed() {
        let _fake = testing::fake();
        let (_dir, store) = disk_store();
        let claims = vec![claim("acme", "paris", "a1")];
        let first = store
            .publish_atlas_insights(
                "run-1",
                &claims,
                &[],
                "",
                &PublishOptions {
                    index_now: true,
                    ..Default::default()
                },
            )
            .unwrap();
        let id = first.claim_memory_ids[0].1.clone();
        // Simulate an earlier failure (pre-FK legacy write) that lost the memory
        // row and its vector state while the claim survived.
        store.conn.execute_batch("PRAGMA foreign_keys=OFF").unwrap();
        store
            .conn
            .execute("DELETE FROM memories WHERE id=?1", [&id])
            .unwrap();
        store.conn.execute_batch("PRAGMA foreign_keys=ON").unwrap();
        store
            .conn
            .execute(
                "DELETE FROM argos_memory_index_state WHERE memory_id=?1",
                [&id],
            )
            .unwrap();
        let second = store
            .publish_atlas_insights(
                "run-2",
                &claims,
                &[],
                "",
                &PublishOptions {
                    index_now: true,
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!((second.created, second.reused, second.repaired), (0, 0, 1));
        assert_eq!(second.claim_memory_ids[0].1, id, "same canonical memory id");
        assert!(second.reconciles());
        let verification = store.verify_atlas_publication(&second).unwrap();
        assert!(verification.memories_ok(), "{verification:?}");
        assert!(
            verification.coverage.complete(),
            "{:?}",
            verification.coverage
        );
        // Both cycles keep their source links.
        let runs: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(DISTINCT run_id) FROM insight_sources",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(runs, 2);
    }

    #[cfg(feature = "lancedb")]
    #[test]
    fn text_changed_mid_index_write_reruns_on_the_new_revision() {
        let _fake = testing::fake();
        let (_dir, store) = disk_store();
        let memory = store
            .add_memory(
                "Harbor tanker manifests list cargo",
                "fact",
                false,
                crate::brain::MemorySource {
                    app: "test".into(),
                    conversation_id: "c".into(),
                    message_id: None,
                    reference: None,
                },
            )
            .unwrap();
        // Force a fresh write of the current revision.
        store
            .conn
            .execute(
                "DELETE FROM argos_memory_index_state WHERE memory_id=?1",
                [&memory.id],
            )
            .unwrap();
        let old_rev = memory_revision(&memory.text);
        let id = memory.id.clone();
        fault::set_after_write(move |conn| {
            conn.execute(
                "UPDATE memories SET text='Northwind ferry timetable' WHERE id=?1",
                [&id],
            )
            .unwrap();
        });
        let first = store.try_index_memory(&memory.id, &old_rev);
        fault::clear();
        assert!(matches!(first, IndexOutcome::Pending { .. }), "{first:?}");
        assert_eq!(
            indexed_revision(&store.conn, &memory.id).unwrap(),
            None,
            "the stale revision written mid-change is not acknowledged"
        );
        let new_rev = memory_revision("Northwind ferry timetable");
        let second = store.try_index_memory(&memory.id, &old_rev);
        assert!(
            matches!(second, IndexOutcome::Ready { ref revision, .. } if *revision == new_rev),
            "{second:?}"
        );
        assert_eq!(
            indexed_revision(&store.conn, &memory.id).unwrap(),
            Some(new_rev)
        );
        let hit = store
            .vectors
            .as_deref()
            .unwrap()
            .search("northwind ferry timetable", 1)
            .unwrap();
        assert_eq!(hit[0].0, memory.id);
        assert!(hit[0].1 > 0.99, "{hit:?}");
        // Through the durable path the pending outcome requeues the same task.
        assert!(store
            .verify_memory_coverage(std::slice::from_ref(&memory.id))
            .unwrap()
            .complete());
    }

    fn source() -> crate::brain::MemorySource {
        crate::brain::MemorySource {
            app: "test".into(),
            conversation_id: "c".into(),
            message_id: None,
            reference: None,
        }
    }

    #[test]
    fn memory_writes_and_their_outbox_rows_commit_together() {
        let _off = testing::disable();
        let store = Store::memory().unwrap();
        let before = store.memories_changed_seq().unwrap();
        let memory = store
            .add_memory(
                "Harbor tanker manifests list cargo",
                "fact",
                false,
                source(),
            )
            .unwrap();
        let active = |store: &Store, id: &str| -> i64 {
            store
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM argos_index_changes
                     WHERE record_id=?1 AND state IN ('pending','running','blocked')",
                    [id],
                    |r| r.get(0),
                )
                .unwrap()
        };
        assert_eq!(
            active(&store, &memory.id),
            1,
            "add queues durable index work"
        );
        assert_eq!(store.memories_changed_seq().unwrap(), before + 1);
        store
            .update_memory(&memory.id, "Northwind ferry timetable", "fact", false)
            .unwrap();
        assert!(
            active(&store, &memory.id) >= 1,
            "edit queues the new revision"
        );
        assert_eq!(store.memories_changed_seq().unwrap(), before + 2);

        // When the outbox insert fails, the memory write rolls back with it.
        store
            .conn
            .execute_batch(
                "CREATE TRIGGER outbox_down BEFORE INSERT ON argos_index_changes
                 BEGIN SELECT RAISE(ABORT, 'outbox down'); END;",
            )
            .unwrap();
        assert!(store
            .add_memory("Orphan without an outbox row", "fact", false, source())
            .is_err());
        assert!(store
            .update_memory(&memory.id, "Edited without an outbox row", "fact", false)
            .is_err());
        let texts: Vec<String> = store
            .list_memories()
            .unwrap()
            .into_iter()
            .map(|m| m.text)
            .collect();
        assert_eq!(texts, vec!["Northwind ferry timetable".to_string()]);
        assert_eq!(store.memories_changed_seq().unwrap(), before + 2);
        assert!(store.conn.is_autocommit(), "no transaction left open");
    }

    #[test]
    fn change_watcher_sees_only_committed_changes_and_coalesces() {
        let _off = testing::disable();
        let (dir, store) = disk_store();
        let other = Store::open(&dir.path().join("argos.db")).unwrap();
        let mut watch = MemoryChangeWatcher::new(&store);
        assert_eq!(watch.poll(&store), None);
        // An uncommitted write in another connection is invisible.
        other.conn.execute_batch("BEGIN IMMEDIATE").unwrap();
        let memory = other
            .add_memory(
                "Harbor tanker manifests list cargo",
                "fact",
                false,
                source(),
            )
            .unwrap();
        assert_eq!(watch.poll(&store), None);
        other
            .update_memory(&memory.id, "Northwind ferry timetable", "fact", false)
            .unwrap();
        other.conn.execute_batch("COMMIT").unwrap();
        // Two commits from another process coalesce into one notification.
        let changed = watch.poll(&store).expect("committed change observed");
        assert_eq!(changed.seq, store.memories_changed_seq().unwrap());
        assert_eq!(watch.poll(&store), None);
        other.delete_memory(&memory.id).unwrap();
        assert!(watch.poll(&store).is_some(), "deletes notify too");
    }

    #[cfg(feature = "lancedb")]
    #[test]
    fn worker_killed_after_the_lance_write_is_recovered_by_revision() {
        let _fake = testing::fake();
        let (_dir, store) = disk_store();
        let memory = store
            .add_memory(
                "Harbor tanker manifests list cargo",
                "fact",
                false,
                source(),
            )
            .unwrap();
        // An external edit commits with its outbox row; no inline indexing.
        let edit = |store: &Store, text: &str| {
            store
                .conn
                .execute(
                    "UPDATE memories SET text=?1 WHERE id=?2",
                    rusqlite::params![text, memory.id],
                )
                .unwrap();
            store
                .enqueue_memory_index(std::slice::from_ref(&memory.id), None)
                .unwrap();
        };
        edit(&store, "Northwind ferry timetable");
        let now = chrono::Utc::now().to_rfc3339();
        let (claimed, work) = tasks::claim_index_work(&store.conn, "worker-a", 30, &now)
            .unwrap()
            .expect("index work queued");
        assert_eq!(work.record_id, memory.id);
        // Worker A writes Lance, then the process dies before recording the
        // revision or acknowledging the task.
        store
            .vectors
            .as_deref()
            .unwrap()
            .upsert_texts(&[(memory.id.clone(), "Northwind ferry timetable".into())])
            .unwrap();
        // While A is dead, the memory changes again.
        edit(&store, "Granite quarry shipping ledger");
        assert_eq!(
            indexed_revision(&store.conn, &memory.id).unwrap(),
            Some(memory_revision("Harbor tanker manifests list cargo")),
            "the unacknowledged write recorded nothing"
        );
        let later = (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339();
        assert!(tasks::interrupt_expired_leases(&store.conn, &later).unwrap() >= 1);
        crate::scheduler::drain_index_once(&store.conn, &store, "worker-b", 16).unwrap();

        let current = memory_revision("Granite quarry shipping ledger");
        assert_eq!(
            indexed_revision(&store.conn, &memory.id).unwrap(),
            Some(current)
        );
        assert!(store
            .verify_memory_coverage(std::slice::from_ref(&memory.id))
            .unwrap()
            .complete());
        assert_eq!(active_index_rows(&store), 0);
        let index = store.vectors.as_deref().unwrap();
        let copies = index
            .ids()
            .unwrap()
            .into_iter()
            .filter(|id| *id == memory.id)
            .count();
        assert_eq!(copies, 1, "re-running the write does not duplicate vectors");
        let hit = index.search("granite quarry shipping ledger", 1).unwrap();
        assert_eq!(hit[0].0, memory.id);
        assert!(hit[0].1 > 0.99, "{hit:?}");
        // The dead worker's late acknowledgement is rejected by owner/epoch.
        let stale = IndexOutcome::Ready {
            revision: memory_revision("Northwind ferry timetable"),
            fingerprint: String::new(),
            generation: String::new(),
        };
        assert!(!tasks::finish_index_work(&store.conn, &claimed, &work, &stale, &later).unwrap());
        assert_eq!(
            indexed_revision(&store.conn, &memory.id).unwrap(),
            Some(memory_revision("Granite quarry shipping ledger"))
        );
    }
}
