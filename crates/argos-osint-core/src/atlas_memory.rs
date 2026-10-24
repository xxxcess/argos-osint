//! Atlas phase 5 "Index and verify memories" (spec §3) and the resumable
//! "Repair Atlas memories" job (spec §4).
//!
//! Lifecycle owned here, on top of the transactional publisher in
//! `store/publication.rs`:
//! 1. Phase 4 checkpoints the validated extraction ([`save_checkpoint`]) so a
//!    crash or retry never reruns a successful LLM extraction.
//! 2. The checkpoint is published (memories, claim/source links, durable index
//!    outbox, receipt) in one SQLite transaction ([`publish_checkpoint`]).
//! 3. Phase 5 drives the run's own outbox work and verifies exact coverage
//!    ([`index_and_verify`]); unrelated index backlog never blocks it.
//! 4. Extraction, publication and indexing keep independent states
//!    ([`MemoryPhase`]); the cycle outcome is derived from them, so a save
//!    failure can never be reported as completed.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::Result;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::atlas_insights::InsightStats;
use crate::events::{self, NewEvent, Severity};
use crate::store::{
    atlas_answer_id, atlas_brief_id, payload_revision, AtlasInsightClaim, PublicationReceipt,
    PublicationVerification, PublishOptions, RejectedClaim, Store,
};
use crate::tasks;
use crate::telemetry::{EventKind, TelemetryEvent, Trigger};

/// Revision used for receipts reconstructed from pre-checkpoint data.
pub const LEGACY_REVISION: &str = "legacy";
/// Shown when memories are saved but `ARGOS_EMBED=0` / no local index.
pub const INDEXING_DISABLED_LINE: &str = "Saved; semantic indexing disabled";
/// Shown when a historical cycle lost its claim payload entirely.
pub const MISSING_PAYLOAD_LINE: &str = "Extraction data missing; re-extraction required";

const REPAIR_PROGRESS_KEY: &str = "atlas_memory_repair_progress";

/// State of one independently tracked child step of the memory lifecycle.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepState {
    #[default]
    Pending,
    Running,
    Completed,
    /// Some work succeeded, some did not (results preserved).
    Partial,
    /// Required configuration is missing (including disabled embeddings).
    Blocked,
    Failed,
    /// Nothing to do (e.g. extraction genuinely found no insights).
    Skipped,
}

impl StepState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Partial => "partial",
            Self::Blocked => "blocked",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
        }
    }
}

/// Overall cycle outcome derived from durable child states.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CycleOutcome {
    Completed,
    Waiting,
    Partial,
    Blocked,
    Failed,
}

impl CycleOutcome {
    /// Value stored in `atlas_runs.state`.
    pub fn as_state(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Waiting => "waiting",
            Self::Partial => "partial",
            Self::Blocked => "blocked",
            Self::Failed => "failed",
        }
    }
}

/// Memory-lifecycle stage names. These are the stage vocabulary every Atlas
/// stage row uses, so the backlog view reads one spelling for the work-unit
/// ledger and the memory state machine.
pub const MEMORY_STAGE_EXTRACTION: &str = "extraction";
pub const MEMORY_STAGE_PUBLICATION: &str = "publication";
pub const MEMORY_STAGE_INDEXING: &str = "indexing";

/// The memory stages in the order the lifecycle runs them.
pub const MEMORY_STAGES: [&str; 3] = [
    MEMORY_STAGE_EXTRACTION,
    MEMORY_STAGE_PUBLICATION,
    MEMORY_STAGE_INDEXING,
];

/// Current state plus the counts one memory stage owns.
fn memory_step(phase: &MemoryPhase, stage: &str) -> Option<(StepState, serde_json::Value)> {
    let (state, counts) = match stage {
        MEMORY_STAGE_EXTRACTION => (
            phase.extraction,
            json!({
                "extracted": phase.extracted,
                "revision": phase.revision,
            }),
        ),
        MEMORY_STAGE_PUBLICATION => (
            phase.publication,
            json!({
                "accepted": phase.accepted,
                "created": phase.created,
                "reused": phase.reused,
                "repaired": phase.repaired,
                "rejected": phase.rejected,
                "brief": phase.brief,
            }),
        ),
        MEMORY_STAGE_INDEXING => (
            phase.indexing,
            json!({
                "required": phase.required,
                "indexed": phase.indexed,
                "pending": phase.pending,
            }),
        ),
        _ => return None,
    };
    Some((state, counts))
}

/// Stage telemetry for the memory lifecycle. Emits one row per child step whose
/// state changed since `previous`, so the pipeline-backlog view can show
/// queued / running / waiting / blocked memory units and the moment each one
/// entered that state. Exactly one row per step transition; a resumed cycle
/// appends the next transition instead of rewriting history.
/// Best-effort: a write failure never fails the cycle.
pub fn record_memory_stage_telemetry(
    db_path: &Path,
    run_id: &str,
    previous: &MemoryPhase,
    current: &MemoryPhase,
) {
    if run_id.trim().is_empty() {
        return;
    }
    for stage in MEMORY_STAGES {
        let Some((state, counts)) = memory_step(current, stage) else {
            continue;
        };
        let Some((before, _)) = memory_step(previous, stage) else {
            continue;
        };
        if before == state {
            continue;
        }
        let seq = crate::atlas_work::next_stage_seq(db_path, run_id, stage);
        let event = TelemetryEvent::new(EventKind::AtlasStage)
            .at(chrono::Utc::now().to_rfc3339())
            .trigger(Trigger::AtlasCycle)
            .app("atlas")
            .run(run_id)
            .outcome(state.label())
            .count(1)
            .payload(json!({
                "stage": stage,
                "unit": "memory",
                "state": state.label(),
                "revision": current.revision,
                "detail": current.detail,
                "counts": counts,
            }));
        let event = event.with_id(format!("atlas-stage-{run_id}-{stage}-{seq}"));
        let _ = crate::telemetry::record(db_path, &event);
    }
}

/// The memory contribution to a terminal cycle row. Durable child states are
/// written out verbatim, so a failed save can never be reported as completed.
pub fn memory_cycle_payload(phase: &MemoryPhase) -> serde_json::Value {
    json!({
        "extraction": phase.extraction.label(),
        "publication": phase.publication.label(),
        "indexing": phase.indexing.label(),
        "revision": phase.revision,
        "detail": phase.detail,
        "extracted": phase.extracted,
        "accepted": phase.accepted,
        "created": phase.created,
        "reused": phase.reused,
        "repaired": phase.repaired,
        "rejected": phase.rejected,
        "brief": phase.brief,
        "required": phase.required,
        "indexed": phase.indexed,
        "pending": phase.pending,
        "cycle_outcome": phase.outcome().as_state(),
    })
}

/// Phase-4/5 memory bookkeeping persisted inside `RunStats` (additive; absent
/// on runs saved before phase 5). Extraction counts and persistence counts are
/// separate fields and are never substituted for each other.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MemoryPhase {
    pub extraction: StepState,
    pub publication: StepState,
    pub indexing: StepState,
    /// Extraction checkpoint / publication revision.
    pub revision: String,
    /// Claims produced by extraction (not proof of persistence).
    pub extracted: u32,
    /// Accepted claims persisted as canonical memories.
    pub accepted: u32,
    pub created: u32,
    pub reused: u32,
    pub repaired: u32,
    pub rejected: u32,
    pub brief: bool,
    /// Memories this run requires to be indexed (claims + brief).
    pub required: u32,
    pub indexed: u32,
    pub pending: u32,
    /// Human-readable reason for a non-completed step.
    pub detail: String,
}

impl MemoryPhase {
    /// True once phase 4 has recorded anything for this run.
    pub fn started(&self) -> bool {
        self.extraction != StepState::Pending
    }

    pub fn outcome(&self) -> CycleOutcome {
        use StepState::*;
        if self.extraction == Failed || self.publication == Failed {
            return CycleOutcome::Failed;
        }
        if self.extraction == Blocked || self.indexing == Blocked {
            return CycleOutcome::Blocked;
        }
        if matches!(self.extraction, Pending | Running)
            || matches!(self.publication, Pending | Running)
            || matches!(self.indexing, Pending | Running)
        {
            return CycleOutcome::Waiting;
        }
        if self.extraction == Partial
            || self.publication == Partial
            || self.indexing == Partial
            || self.indexing == Failed
        {
            return CycleOutcome::Partial;
        }
        CycleOutcome::Completed
    }

    /// Dedicated fifth-phase progress row, e.g.
    /// "Memories: 18 new claims · 4 reused · 0 brief · 22/22 index ready".
    pub fn line(&self) -> String {
        use StepState::*;
        if !self.started() {
            return String::new();
        }
        match (self.extraction, self.publication) {
            (Failed | Blocked, _) => {
                return format!(
                    "Memories: none saved — {}",
                    self.detail_or("extraction did not run")
                )
            }
            (Skipped, _) => return "Memories: none (no insights to save)".into(),
            (_, Failed) => {
                return format!(
                    "Memories: not saved — {}",
                    self.detail_or("publication failed")
                )
            }
            (_, Pending | Running) => return "Memories: extracted, saving…".into(),
            _ => {}
        }
        if self.accepted == 0 && !self.brief {
            let mut line = "Memories: none (no accepted insights)".to_string();
            if self.rejected > 0 {
                line.push_str(&format!(" · {} rejected", self.rejected));
            }
            return line;
        }
        let mut parts = vec![format!("{} new claims", self.created)];
        if self.reused > 0 {
            parts.push(format!("{} reused", self.reused));
        }
        if self.repaired > 0 {
            parts.push(format!("{} repaired", self.repaired));
        }
        if self.rejected > 0 {
            parts.push(format!("{} rejected", self.rejected));
        }
        parts.push(if self.brief {
            "1 brief".into()
        } else {
            "0 brief".into()
        });
        match self.indexing {
            Blocked => parts.push(INDEXING_DISABLED_LINE.into()),
            Pending | Running => {
                parts.push(format!("{}/{} index ready", self.indexed, self.required))
            }
            _ => parts.push(format!("{}/{} index ready", self.indexed, self.required)),
        }
        format!("Memories: {}", parts.join(" · "))
    }

    /// Run note stored alongside the derived state.
    pub fn note(&self) -> String {
        let line = self.line();
        match self.outcome() {
            CycleOutcome::Completed => line,
            _ if self.detail.is_empty() || line.contains(&self.detail) => line,
            _ => format!("{line} ({})", self.detail),
        }
    }

    fn detail_or<'a>(&'a self, fallback: &'a str) -> &'a str {
        if self.detail.is_empty() {
            fallback
        } else {
            &self.detail
        }
    }

    pub fn apply_receipt(&mut self, receipt: &PublicationReceipt) {
        self.revision = receipt.revision.clone();
        self.accepted = receipt.accepted_claims as u32;
        self.created = receipt.created as u32;
        self.reused = receipt.reused as u32;
        self.repaired = receipt.repaired as u32;
        self.rejected = receipt.rejected.len() as u32;
        self.brief = receipt.brief_memory_id.is_some();
        self.required = receipt.affected_memory_ids.len() as u32;
        self.publication = if receipt.input_claims == 0 {
            StepState::Skipped
        } else if receipt.accepted_claims == 0 {
            // Every claim rejected: nothing usable was saved.
            StepState::Partial
        } else {
            StepState::Completed
        };
        if self.publication == StepState::Partial && self.detail.is_empty() {
            self.detail = "every extracted claim was rejected".into();
        }
        self.indexing = if self.required == 0 {
            StepState::Skipped
        } else {
            StepState::Pending
        };
    }

    pub fn apply_verification(&mut self, verification: &PublicationVerification) {
        let coverage = &verification.coverage;
        self.required = coverage.required as u32;
        self.indexed = coverage.indexed.len() as u32;
        self.pending = (coverage.pending.len() + coverage.missing_rows.len()) as u32;
        self.indexing = if coverage.required == 0 {
            StepState::Skipped
        } else if coverage.disabled {
            StepState::Blocked
        } else if coverage.complete() {
            StepState::Completed
        } else {
            // Eligible index work remains; background retries continue.
            StepState::Running
        };
        if !verification.memories_ok() {
            if matches!(self.publication, StepState::Completed | StepState::Skipped) {
                self.publication = StepState::Partial;
            }
            self.detail = format!(
                "{} claim memories missing, {} broken source links{}",
                verification.missing_claim_memories.len(),
                verification.broken_source_links.len(),
                if verification.brief_ok {
                    ""
                } else {
                    ", brief memory missing"
                }
            );
        } else if self.indexing == StepState::Blocked {
            self.detail = INDEXING_DISABLED_LINE.into();
        } else if matches!(self.indexing, StepState::Running | StepState::Pending) {
            self.detail = format!(
                "{} of {} memories not yet indexed; retries continue in the background",
                self.pending, self.required
            );
        } else if self.outcome() == CycleOutcome::Completed {
            self.detail.clear();
        }
    }
}

/// Durable, validated extraction payload for one run/revision.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Checkpoint {
    pub run_id: String,
    pub revision: String,
    pub claims: Vec<AtlasInsightClaim>,
    pub relations: Vec<(String, String, String)>,
    pub brief: String,
    #[serde(default)]
    pub entity_path: String,
    #[serde(default)]
    pub stats: InsightStats,
    /// Some extraction packets failed but kept claims survived.
    #[serde(default)]
    pub partial: bool,
    #[serde(default)]
    pub notes: Vec<String>,
    /// Raw pre-adjustment claim confidence by fingerprint, captured before
    /// `apply_admiralty_evaluation` scaled it. Kept beside the scaled
    /// `claims` so an "average initial confidence" can never reverse the
    /// adjusted score. Absent on checkpoints written before this field, and
    /// absent for claims recovered from an older published cycle, which stay
    /// N/A rather than being given a fabricated initial value.
    #[serde(default)]
    pub initial_confidences: std::collections::BTreeMap<String, f64>,
}

/// Inputs for [`save_checkpoint`].
pub struct CheckpointInput<'a> {
    pub claims: &'a [AtlasInsightClaim],
    pub relations: &'a [(String, String, String)],
    pub brief: &'a str,
    pub entity_path: &'a str,
    pub stats: &'a InsightStats,
    pub partial: bool,
    pub notes: Vec<String>,
    /// Raw pre-adjustment confidence by fingerprint; see [`Checkpoint`].
    pub initial_confidences: std::collections::BTreeMap<String, f64>,
}

fn checkpoint_of(run_id: &str, input: CheckpointInput<'_>) -> Checkpoint {
    Checkpoint {
        run_id: run_id.into(),
        revision: payload_revision(input.claims, input.brief),
        claims: input.claims.to_vec(),
        relations: input.relations.to_vec(),
        brief: input.brief.into(),
        entity_path: input.entity_path.into(),
        stats: input.stats.clone(),
        partial: input.partial,
        notes: input.notes,
        initial_confidences: input.initial_confidences.clone(),
    }
}

/// Persist the validated extraction before publication. Idempotent per
/// payload revision.
pub fn save_checkpoint(
    store: &Store,
    run_id: &str,
    input: CheckpointInput<'_>,
) -> Result<Checkpoint> {
    let checkpoint = checkpoint_of(run_id, input);
    store.conn.execute(
        "INSERT INTO argos_atlas_checkpoints(run_id,revision,payload_json,accepted,rejected,created_at)
         VALUES (?1,?2,?3,?4,0,?5)
         ON CONFLICT(run_id,revision) DO UPDATE SET payload_json=excluded.payload_json,
            accepted=excluded.accepted, created_at=excluded.created_at",
        params![
            run_id,
            checkpoint.revision,
            serde_json::to_string(&checkpoint)?,
            checkpoint.claims.len() as i64,
            chrono::Utc::now().to_rfc3339(),
        ],
    )?;
    Ok(checkpoint)
}

/// Latest checkpoint for a run.
pub fn load_checkpoint(store: &Store, run_id: &str) -> Result<Option<Checkpoint>> {
    let json: Option<String> = store
        .conn
        .query_row(
            "SELECT payload_json FROM argos_atlas_checkpoints WHERE run_id=?1
             ORDER BY created_at DESC, rowid DESC LIMIT 1",
            [run_id],
            |r| r.get(0),
        )
        .optional()?;
    Ok(match json {
        Some(json) => Some(serde_json::from_str(&json)?),
        None => None,
    })
}

/// Publish a checkpoint through the transactional publisher. Indexing happens
/// afterwards in phase 5 (never inside the SQLite transaction).
pub fn publish_checkpoint(
    store: &Store,
    checkpoint: &Checkpoint,
    parent_job: Option<&str>,
    repair: bool,
    brief_override: Option<&str>,
) -> Result<PublicationReceipt> {
    let brief = brief_override.unwrap_or(&checkpoint.brief);
    let receipt = store.publish_atlas_insights(
        &checkpoint.run_id,
        &checkpoint.claims,
        &checkpoint.relations,
        brief,
        &PublishOptions {
            revision: Some(checkpoint.revision.clone()),
            parent_job: parent_job.map(str::to_string),
            repair,
            index_now: false,
            ..Default::default()
        },
    )?;
    if receipt.brief_memory_id.is_some() && !repair {
        store.enqueue_summary_flush_best_effort(
            crate::summarization::SummarizationMode::AtlasBrief,
            &format!("atlas-brief-{}", checkpoint.run_id),
            &checkpoint.run_id,
            brief.trim(),
            "atlas",
            800,
        );
    }
    Ok(receipt)
}

/// Bounds for the phase-5 barrier: how many inline index rounds to attempt
/// before reporting an explicit incomplete outcome (the index pool keeps
/// retrying in the background).
#[derive(Clone, Copy, Debug)]
pub struct Barrier {
    pub rounds: u32,
    pub wait: Duration,
}

impl Default for Barrier {
    fn default() -> Self {
        Self {
            rounds: 3,
            wait: if cfg!(test) {
                Duration::from_millis(5)
            } else {
                Duration::from_millis(1500)
            },
        }
    }
}

/// Result of [`index_and_verify`].
#[derive(Clone, Debug)]
pub struct Phase5Report {
    pub verification: PublicationVerification,
    /// The pause flag was raised before coverage completed.
    pub paused: bool,
}

/// Phase 5: process pending index work for exactly this run's required
/// memories and verify memory + index coverage. Re-queues work for any
/// required memory whose revision is not verifiably served (including reused
/// claims damaged by earlier failures) and acknowledges it only through the
/// same leased outbox tasks.
pub fn index_and_verify(
    store: &Store,
    receipt: &PublicationReceipt,
    pause: Option<&AtomicBool>,
    barrier: Barrier,
    mut progress: impl FnMut(u32, u32),
) -> Result<Phase5Report> {
    ensure_historical_contract(store, &receipt.run_id)?;
    let mut round = 0u32;
    loop {
        let verification = store.verify_atlas_publication(receipt)?;
        let coverage = &verification.coverage;
        progress(coverage.indexed.len() as u32, coverage.required as u32);
        if coverage.disabled || coverage.complete() || round >= barrier.rounds {
            return Ok(Phase5Report {
                verification,
                paused: false,
            });
        }
        if pause.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
            return Ok(Phase5Report {
                verification,
                paused: true,
            });
        }
        // One inline attempt per required memory, acknowledged through the same
        // leased outbox task. Later rounds only wait for the index pool, so the
        // barrier never burns the task's retry budget (backoff stays with the pool).
        let mut all_ready = false;
        if round == 0 {
            let queued = store.requeue_uncovered(coverage, None)?;
            let outcomes = store.index_now(&queued);
            all_ready = !outcomes.is_empty() && outcomes.iter().all(|o| o.is_ready());
        }
        round += 1;
        if !all_ready && round < barrier.rounds {
            std::thread::sleep(barrier.wait);
        }
    }
}

/// Receipt for a run published before checkpoints existed, reconstructed from
/// its stored claim/source links. Unrecoverable links are reported as rejected
/// with a reason; nothing is fabricated from a fingerprint. Index work is
/// queued for every recoverable memory whose revision is missing or stale.
pub fn legacy_receipt(
    store: &Store,
    run_id: &str,
    parent_job: Option<&str>,
) -> Result<PublicationReceipt> {
    let answer_id = atlas_answer_id(run_id);
    let links: Vec<(String, String, Option<String>)> = {
        let mut stmt = store.conn.prepare(
            "SELECT s.fingerprint, MIN(IFNULL(s.call_id,'')), c.memory_id
             FROM insight_sources s LEFT JOIN insight_claims c ON c.fingerprint = s.fingerprint
             WHERE s.run_id=?1 AND s.answer_id=?2
             GROUP BY s.fingerprint ORDER BY MIN(s.rowid)",
        )?;
        let rows = stmt
            .query_map(params![run_id, answer_id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    let mut receipt = PublicationReceipt {
        run_id: run_id.into(),
        revision: LEGACY_REVISION.into(),
        input_claims: links.len(),
        ..Default::default()
    };
    for (index, (fingerprint, article_id, memory_id)) in links.into_iter().enumerate() {
        let tombstoned: i64 = store.conn.query_row(
            "SELECT COUNT(*) FROM argos_memory_tombstones WHERE (fingerprint=?1 AND fingerprint<>'') OR memory_id=?2",
            params![fingerprint, memory_id.clone().unwrap_or_default()],
            |r| r.get(0),
        )?;
        let row_exists = match &memory_id {
            Some(id) => store
                .conn
                .query_row("SELECT 1 FROM memories WHERE id=?1", [id], |_| Ok(()))
                .optional()?
                .is_some(),
            None => false,
        };
        let reason = match (&memory_id, row_exists) {
            (Some(_), true) => None,
            _ if tombstoned > 0 => Some("deleted by user (tombstoned); not recreated"),
            (None, _) => Some("source link has no claim; claim text lost — re-extraction required"),
            (Some(_), false) => {
                Some("canonical memory row missing; claim text lost — re-extraction required")
            }
        };
        match reason {
            None => {
                let id = memory_id.unwrap_or_default();
                receipt.reused += 1;
                if !receipt.reused_memory_ids.contains(&id) {
                    receipt.reused_memory_ids.push(id.clone());
                }
                receipt.accepted_claims += 1;
                receipt.claim_memory_ids.push((fingerprint, id));
            }
            Some(reason) => receipt.rejected.push(RejectedClaim {
                index,
                article_id,
                reason: reason.into(),
            }),
        }
    }
    receipt.brief_memory_id = atlas_brief_id(&store.conn, run_id)?;
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
    store.conn.execute_batch("BEGIN IMMEDIATE")?;
    let result = (|| -> Result<()> {
        let (queued, already) = store.enqueue_memory_index(&affected, parent_job)?;
        receipt.affected_memory_ids = affected;
        receipt.already_indexed = already;
        receipt.queued_index_changes = queued.iter().map(|q| q.seq).collect();
        receipt.index_tasks = queued.iter().map(|q| q.task_id.clone()).collect();
        let now = chrono::Utc::now().to_rfc3339();
        store.conn.execute(
            "INSERT INTO argos_atlas_publications(run_id,revision,state,receipt_json,job_id,created_at,updated_at)
             VALUES (?1,?2,'reconstructed',?3,?4,?5,?5)
             ON CONFLICT(run_id,revision) DO UPDATE SET receipt_json=excluded.receipt_json,
                job_id=excluded.job_id, updated_at=excluded.updated_at",
            params![
                run_id,
                LEGACY_REVISION,
                serde_json::to_string(&receipt)?,
                parent_job.unwrap_or_default(),
                now
            ],
        )?;
        Ok(())
    })();
    match result {
        Ok(()) => store.conn.execute_batch("COMMIT")?,
        Err(err) => {
            let _ = store.conn.execute_batch("ROLLBACK");
            return Err(err);
        }
    }
    Ok(receipt)
}

fn run_row(store: &Store, run_id: &str) -> Result<Option<crate::store::AtlasRunRow>> {
    Ok(store
        .atlas_list_runs()?
        .into_iter()
        .find(|run| run.id == run_id))
}

/// Explicit historical maintenance must never create cycle memories for the
/// classification-only contract. Read raw JSON to retain legacy compatibility.
pub fn ensure_historical_contract(store: &Store, run_id: &str) -> Result<()> {
    let Some(run) = run_row(store, run_id)? else {
        anyhow::bail!("Atlas run {run_id} not found");
    };
    let stats: serde_json::Value = serde_json::from_str(&run.stats_json).unwrap_or_default();
    anyhow::ensure!(
        !stats
            .get("contract_version")
            .and_then(serde_json::Value::as_str)
            .is_some_and(crate::atlas_work::is_collection_contract),
        "Repair/Verify are restricted to historical Atlas contracts"
    );
    Ok(())
}

/// Re-verify a finished run's stored receipt and update its memory phase.
/// Only the memory-derived state changes (e.g. `partial` → `completed` once the
/// background pool finished indexing); extraction dates/outcomes are untouched.
/// Returns the refreshed phase, or None for runs without phase-5 bookkeeping.
pub fn refresh_run_indexing(store: &Store, run_id: &str) -> Result<Option<MemoryPhase>> {
    let Some(run) = run_row(store, run_id)? else {
        return Ok(None);
    };
    if ensure_historical_contract(store, run_id).is_err() {
        return Ok(None);
    }
    if matches!(run.state.as_str(), "running" | "paused") {
        return Ok(None);
    }
    let mut stats: crate::atlas::RunStats =
        serde_json::from_str(&run.stats_json).unwrap_or_default();
    if !stats.memories.started() || stats.memories.publication == StepState::Failed {
        return Ok(None);
    }
    let Some(receipt) = store.atlas_publication_receipt(run_id)? else {
        return Ok(None);
    };
    let verification = store.verify_atlas_publication(&receipt)?;
    stats.memories.apply_verification(&verification);
    store.atlas_save(run_id, &run.cursor_json, &serde_json::to_string(&stats)?)?;
    let outcome = stats.memories.outcome();
    if outcome.as_state() != run.state {
        store.atlas_set_state(run_id, outcome.as_state(), &stats.memories.note(), false)?;
    }
    Ok(Some(stats.memories))
}

/// Re-verify finished runs whose memory phase is incomplete (partial or
/// blocked indexing) so they reach their verified status once the index pool
/// catches up or embeddings are enabled again. Bounded per call.
pub fn refresh_incomplete_runs(store: &Store, limit: usize) -> Result<usize> {
    let ids: Vec<String> = {
        let mut stmt = store.conn.prepare(
            "SELECT id FROM atlas_runs WHERE state IN ('partial','blocked','waiting') AND phase=5
             ORDER BY started_at DESC LIMIT ?1",
        )?;
        let rows = stmt
            .query_map([limit as i64], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        rows
    };
    let mut changed = 0usize;
    for id in ids {
        let before = run_row(store, &id)?.map(|run| run.state);
        refresh_run_indexing(store, &id)?;
        let after = run_row(store, &id)?.map(|run| run.state);
        if before != after {
            changed += 1;
        }
    }
    Ok(changed)
}

/// [`refresh_run_indexing`] on the selected store (SurrealDB included).
pub async fn refresh_run_indexing_store(
    store: &crate::atlas_store::AtlasStore,
    run: &crate::store::AtlasRunRow,
) -> Result<Option<MemoryPhase>> {
    let raw: serde_json::Value = serde_json::from_str(&run.stats_json).unwrap_or_default();
    if raw
        .get("contract_version")
        .and_then(serde_json::Value::as_str)
        .is_some_and(crate::atlas_work::is_collection_contract)
        || matches!(run.state.as_str(), "running" | "paused")
    {
        return Ok(None);
    }
    let mut stats: crate::atlas::RunStats =
        serde_json::from_str(&run.stats_json).unwrap_or_default();
    if !stats.memories.started() || stats.memories.publication == StepState::Failed {
        return Ok(None);
    }
    let Some(receipt) = store.publication_receipt(&run.id).await? else {
        return Ok(None);
    };
    let verification = store.verify_publication(&receipt).await?;
    stats.memories.apply_verification(&verification);
    store
        .save(&run.id, &run.cursor_json, &serde_json::to_string(&stats)?)
        .await?;
    let outcome = stats.memories.outcome();
    if outcome.as_state() != run.state {
        store
            .set_state(&run.id, outcome.as_state(), &stats.memories.note(), false)
            .await?;
    }
    Ok(Some(stats.memories))
}

/// [`refresh_incomplete_runs`] on the selected store. Returns the number
/// of runs whose state changed.
pub async fn refresh_incomplete_runs_store(
    store: &crate::atlas_store::AtlasStore,
    limit: usize,
) -> Result<usize> {
    let mut runs: Vec<crate::store::AtlasRunRow> = store
        .list_runs()
        .await?
        .into_iter()
        .filter(|run| {
            matches!(run.state.as_str(), "partial" | "blocked" | "waiting") && run.phase == 5
        })
        .collect();
    runs.sort_by(|a, b| b.started_at.cmp(&a.started_at));
    runs.truncate(limit);
    let mut changed = 0usize;
    for run in runs {
        if let Some(phase) = refresh_run_indexing_store(store, &run).await? {
            if phase.outcome().as_state() != run.state {
                changed += 1;
            }
        }
    }
    Ok(changed)
}

/// Per-run repair classification (stored separately from the run's own state).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepairStatus {
    /// Memories and index coverage verified; nothing to do.
    Verified,
    /// Missing canonical rows/links were restored from authoritative data.
    Repaired,
    /// Index work was (re)queued for missing or stale vectors.
    IndexQueued,
    /// Memories verified; semantic indexing is disabled.
    IndexingDisabled,
    /// Some links could not be recovered (reported, never fabricated).
    Partial,
    /// Claim payload lost entirely; re-extraction required.
    MissingPayload,
    /// The run produced no insights.
    NoInsights,
    /// Run is running/paused; repaired after it finishes.
    Active,
}

/// Outcome of repairing one run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunRepair {
    pub run_id: String,
    pub status: RepairStatus,
    pub restored_memory_ids: Vec<String>,
    pub requeued: usize,
    pub required: usize,
    pub indexed: usize,
    /// `(fingerprint or article id, reason)` that could not be recovered.
    pub unrecoverable: Vec<(String, String)>,
    /// Retained source articles allow targeted re-extraction.
    pub reextract_available: bool,
    pub detail: String,
}

/// Repair one retained run through the same phase-5 pipeline. Never rewrites
/// the run's historical extraction outcome; status lands in
/// `argos_atlas_repairs`.
pub fn repair_run(store: &Store, run_id: &str, job_id: Option<&str>) -> Result<RunRepair> {
    ensure_historical_contract(store, run_id)?;
    let mut out = RunRepair {
        run_id: run_id.into(),
        status: RepairStatus::Verified,
        restored_memory_ids: Vec::new(),
        requeued: 0,
        required: 0,
        indexed: 0,
        unrecoverable: Vec::new(),
        reextract_available: false,
        detail: String::new(),
    };
    let Some(run) = run_row(store, run_id)? else {
        anyhow::bail!("Atlas run {run_id} not found");
    };
    if matches!(run.state.as_str(), "running" | "paused") {
        out.status = RepairStatus::Active;
        out.detail = "run is still active".into();
        return Ok(out);
    }
    let stats: crate::atlas::RunStats = serde_json::from_str(&run.stats_json).unwrap_or_default();
    let prior = store.atlas_publication_receipt(run_id)?;
    let has_links = store.atlas_has_insights(run_id)?;
    let receipt = if let Some(checkpoint) = load_checkpoint(store, run_id)? {
        // Never recreate a brief the user deleted.
        let brief_deleted = match prior.as_ref().and_then(|r| r.brief_memory_id.clone()) {
            Some(id) => store.is_tombstoned(&id)?,
            None => false,
        };
        let receipt = publish_checkpoint(
            store,
            &checkpoint,
            job_id,
            true,
            brief_deleted.then_some(""),
        )?;
        out.restored_memory_ids
            .extend(receipt.created_memory_ids.iter().cloned());
        out.restored_memory_ids
            .extend(receipt.repaired_memory_ids.iter().cloned());
        receipt
    } else if has_links || prior.is_some() {
        legacy_receipt(store, run_id, job_id)?
    } else {
        out.reextract_available = !store.atlas_list_articles(run_id)?.is_empty();
        let produced = stats.insights.claims > 0
            || !stats.insights.rows.is_empty()
            || stats.memories.extracted > 0;
        out.status = if produced {
            out.detail = MISSING_PAYLOAD_LINE.into();
            RepairStatus::MissingPayload
        } else {
            RepairStatus::NoInsights
        };
        record_repair(store, &out, job_id)?;
        return Ok(out);
    };
    for rejected in &receipt.rejected {
        if !rejected.reason.contains("deleted by user") {
            out.unrecoverable
                .push((rejected.article_id.clone(), rejected.reason.clone()));
        }
    }
    let verification = store.verify_atlas_publication(&receipt)?;
    let requeued = store.requeue_uncovered(&verification.coverage, job_id)?;
    out.requeued = requeued.len() + receipt.queued_index_changes.len();
    out.required = verification.coverage.required;
    out.indexed = verification.coverage.indexed.len();
    out.status = if !out.unrecoverable.is_empty() {
        out.reextract_available = !store.atlas_list_articles(run_id)?.is_empty();
        out.detail = format!(
            "{} claims unrecoverable; {MISSING_PAYLOAD_LINE}",
            out.unrecoverable.len()
        );
        RepairStatus::Partial
    } else if !out.restored_memory_ids.is_empty() {
        RepairStatus::Repaired
    } else if verification.coverage.disabled {
        out.detail = INDEXING_DISABLED_LINE.into();
        RepairStatus::IndexingDisabled
    } else if out.requeued > 0 || !verification.coverage.complete() {
        RepairStatus::IndexQueued
    } else {
        RepairStatus::Verified
    };
    // New-style runs: refresh the memory phase from the repaired receipt.
    let _ = refresh_run_indexing(store, run_id);
    record_repair(store, &out, job_id)?;
    Ok(out)
}

fn record_repair(store: &Store, repair: &RunRepair, job_id: Option<&str>) -> Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    let status = serde_json::to_value(repair.status)?
        .as_str()
        .unwrap_or("verified")
        .to_string();
    store.conn.execute(
        "INSERT INTO argos_atlas_repairs(run_id,status,detail_json,job_id,updated_at) VALUES (?1,?2,?3,?4,?5)
         ON CONFLICT(run_id) DO UPDATE SET status=excluded.status, detail_json=excluded.detail_json,
            job_id=excluded.job_id, updated_at=excluded.updated_at",
        params![
            repair.run_id,
            status,
            serde_json::to_string(repair)?,
            job_id.unwrap_or_default(),
            now
        ],
    )?;
    if let Some(event) = repair_event(repair, &status, job_id)? {
        let _ = events::record_event(&store.conn, &event);
    }
    Ok(())
}

/// The `atlas.memory_repair` event of a non-trivial repair.
fn repair_event(
    repair: &RunRepair,
    status: &str,
    job_id: Option<&str>,
) -> Result<Option<NewEvent>> {
    if repair.status == RepairStatus::Verified || repair.status == RepairStatus::NoInsights {
        return Ok(None);
    }
    let severity = match repair.status {
        RepairStatus::Partial | RepairStatus::MissingPayload => Severity::Warn,
        _ => Severity::Info,
    };
    Ok(Some(NewEvent {
        severity: Some(severity),
        app: "atlas".into(),
        event_type: "atlas.memory_repair".into(),
        message: format!(
            "Repair Atlas memories: {} — {}",
            status,
            if repair.detail.is_empty() {
                format!(
                    "{} restored, {} index changes queued",
                    repair.restored_memory_ids.len(),
                    repair.requeued
                )
            } else {
                repair.detail.clone()
            }
        ),
        details: serde_json::to_string(repair)?,
        job_id: job_id.unwrap_or_default().into(),
        run_id: repair.run_id.clone(),
        ..Default::default()
    }))
}

/// Stored repair status for a run.
pub fn repair_status(store: &Store, run_id: &str) -> Result<Option<RunRepair>> {
    let json: Option<String> = store
        .conn
        .query_row(
            "SELECT detail_json FROM argos_atlas_repairs WHERE run_id=?1",
            [run_id],
            |r| r.get(0),
        )
        .optional()?;
    Ok(json.and_then(|json| serde_json::from_str(&json).ok()))
}

/// Resumable progress of the repair job (bounded pages over retained runs).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RepairProgress {
    pub job_id: String,
    /// Keyset cursor: last processed `(started_at, id)`.
    pub after_started_at: String,
    pub after_id: String,
    pub runs_checked: u32,
    pub runs_repaired: u32,
    pub memories_restored: u32,
    pub index_queued: u32,
    pub unrecoverable: u32,
    /// Runs needing targeted re-extraction.
    pub missing_payload: Vec<String>,
    pub done: bool,
}

fn load_progress(store: &Store) -> Result<RepairProgress> {
    Ok(store
        .app_state_get(REPAIR_PROGRESS_KEY)?
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default())
}

fn save_progress(store: &Store, progress: &RepairProgress) -> Result<()> {
    store.app_state_set(REPAIR_PROGRESS_KEY, &serde_json::to_string(progress)?)
}

/// Process one bounded page of retained runs (oldest first) and persist the
/// cursor. Returns the per-run results; an empty page marks the job done.
pub fn repair_page(
    store: &Store,
    progress: &mut RepairProgress,
    page: usize,
) -> Result<Vec<RunRepair>> {
    let ids: Vec<(String, String)> = {
        let mut stmt = store.conn.prepare(
            "SELECT started_at, id FROM atlas_runs
             WHERE (NOT json_valid(stats_json) OR COALESCE(json_extract(stats_json,'$.contract_version'),'') NOT IN ('atlas-collection-v1','atlas-collection-v2'))
             AND (started_at > ?1 OR (started_at = ?1 AND id > ?2))
             ORDER BY started_at, id LIMIT ?3",
        )?;
        let rows = stmt
            .query_map(
                params![progress.after_started_at, progress.after_id, page as i64],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    let mut results = Vec::new();
    tasks::retire_atlas_insight_work(&store.conn, &chrono::Utc::now().to_rfc3339())?;
    if !progress.job_id.is_empty() {
        register_repair_job(store, &progress.job_id)?;
    }
    if ids.is_empty() {
        progress.done = true;
        save_progress(store, progress)?;
        return Ok(results);
    }
    for (started_at, id) in ids {
        let repair = repair_run(
            store,
            &id,
            Some(progress.job_id.as_str()).filter(|j| !j.is_empty()),
        )?;
        progress.runs_checked += 1;
        if !matches!(
            repair.status,
            RepairStatus::Verified | RepairStatus::NoInsights | RepairStatus::Active
        ) {
            progress.runs_repaired += 1;
        }
        progress.memories_restored += repair.restored_memory_ids.len() as u32;
        progress.index_queued += repair.requeued as u32;
        progress.unrecoverable += repair.unrecoverable.len() as u32;
        if (repair.status == RepairStatus::MissingPayload || repair.status == RepairStatus::Partial)
            && !progress.missing_payload.contains(&id)
        {
            progress.missing_payload.push(id.clone());
        }
        progress.after_started_at = started_at;
        progress.after_id = id;
        results.push(repair);
    }
    // Progress commits with each page so an interrupted job resumes here.
    save_progress(store, progress)?;
    Ok(results)
}

/// Run (or resume) the "Repair Atlas memories" job to completion. `restart`
/// begins a new pass from the oldest run (user-triggered repair).
pub fn run_repair_job(store: &Store, page: usize, restart: bool) -> Result<RepairProgress> {
    let mut progress = load_progress(store)?;
    let now = chrono::Utc::now().to_rfc3339();
    if restart || progress.done || progress.job_id.is_empty() {
        progress = RepairProgress {
            job_id: format!("atlas-repair-{}", chrono::Utc::now().timestamp_millis()),
            ..Default::default()
        };
    }
    let _ = now;
    register_repair_job(store, &progress.job_id)?;
    let total: i64 = store
        .conn
        .query_row("SELECT COUNT(*) FROM atlas_runs", [], |r| r.get(0))?;
    save_progress(store, &progress)?;
    loop {
        let now = chrono::Utc::now().to_rfc3339();
        tasks::set_job_progress(
            &store.conn,
            &progress.job_id,
            "running",
            "repair",
            progress.runs_checked as i64,
            Some(total),
            "",
            &now,
        )?;
        let page_result = repair_page(store, &mut progress, page.max(1));
        match page_result {
            Ok(results) if results.is_empty() => break,
            Ok(_) => continue,
            Err(err) => {
                let now = chrono::Utc::now().to_rfc3339();
                let message = events::redact(&format!("{err:#}"));
                tasks::set_job_progress(
                    &store.conn,
                    &progress.job_id,
                    "failed",
                    "repair",
                    progress.runs_checked as i64,
                    Some(total),
                    &message,
                    &now,
                )?;
                return Err(err);
            }
        }
    }
    let now = chrono::Utc::now().to_rfc3339();
    let summary = if progress.missing_payload.is_empty() {
        String::new()
    } else {
        format!(
            "{} runs need re-extraction ({MISSING_PAYLOAD_LINE})",
            progress.missing_payload.len()
        )
    };
    tasks::set_job_progress(
        &store.conn,
        &progress.job_id,
        if progress.missing_payload.is_empty() {
            "completed"
        } else {
            "partial"
        },
        "repair",
        progress.runs_checked as i64,
        Some(total),
        &summary,
        &now,
    )?;
    if progress.memories_restored > 0 {
        let _ = store.conn.execute(
            "INSERT INTO app_state(key,value) VALUES ('memories_changed_seq','1')
             ON CONFLICT(key) DO UPDATE SET value=CAST(CAST(value AS INTEGER)+1 AS TEXT)",
            [],
        );
    }
    Ok(progress)
}

/// Register the repair job in the shared job registry (idempotent).
fn register_repair_job(store: &Store, job_id: &str) -> Result<()> {
    tasks::enqueue_job_with(
        &store.conn,
        &tasks::NewJob {
            id: job_id.into(),
            kind: "atlas_memory_repair".into(),
            owner_scope: "atlas".into(),
            input_revision: String::new(),
            deadline_at: String::new(),
        },
        &tasks::JobMeta {
            app: "atlas".into(),
            operation: "atlas_memory_repair".into(),
            title: "Repair Atlas memories".into(),
            ..Default::default()
        },
        &chrono::Utc::now().to_rfc3339(),
    )?;
    // Owned by this process: if it exits mid-repair, the next start marks the
    // job interrupted (and the resumable repair re-owns it when it continues).
    store.conn.execute(
        "UPDATE argos_jobs SET worker_owner=?2,
            active_since=CASE WHEN active_since='' AND state IN ('queued','running') THEN ?3 ELSE active_since END
         WHERE id=?1",
        params![
            job_id,
            crate::scheduler::process_owner(),
            chrono::Utc::now().to_rfc3339()
        ],
    )?;
    Ok(())
}

static REPAIR_RUNNING: AtomicBool = AtomicBool::new(false);

/// A repair that died mid-pass must not read "running" forever.
fn mark_repair_interrupted(store: &Store, reason: &str) {
    let Ok(progress) = load_progress(store) else {
        return;
    };
    if progress.job_id.is_empty() {
        return;
    }
    let (done, total): (i64, Option<i64>) = store
        .conn
        .query_row(
            "SELECT IFNULL(progress_done,0), progress_total FROM argos_jobs WHERE id=?1",
            [&progress.job_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap_or((0, None));
    let _ = tasks::set_job_progress(
        &store.conn,
        &progress.job_id,
        "failed",
        "repair",
        done,
        total,
        reason,
        &chrono::Utc::now().to_rfc3339(),
    );
}

/// Start the repair job on a background thread (no-op when one is already
/// running in this process). `restart` = user-triggered fresh pass.
pub fn spawn_repair(db_path: std::path::PathBuf, restart: bool) -> bool {
    if REPAIR_RUNNING.swap(true, Ordering::SeqCst) {
        return false;
    }
    // Under SurrealDB the repair runs on the process runtime, never SQLite.
    if crate::recon::backend::sqlite_path(&db_path).is_none() {
        let spawned = crate::storage::runtime::spawn(async move {
            if let Ok(store) = AtlasStore::open(&db_path).await {
                if let Err(e) = run_repair_job_store(&store, 16, restart).await {
                    tracing::warn!("atlas repair failed: {e:#}");
                }
            }
            REPAIR_RUNNING.store(false, Ordering::SeqCst);
        });
        if !spawned {
            REPAIR_RUNNING.store(false, Ordering::SeqCst);
        }
        return spawned;
    }
    let spawned = std::thread::Builder::new()
        .name("argos-atlas-repair".into())
        .spawn(move || {
            if let Ok(store) = Store::open(&db_path) {
                let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run_repair_job(&store, 16, restart)
                }));
                if ran.is_err() {
                    mark_repair_interrupted(&store, "repair stopped by a panic");
                }
            }
            REPAIR_RUNNING.store(false, Ordering::SeqCst);
        })
        .is_ok();
    if !spawned {
        REPAIR_RUNNING.store(false, Ordering::SeqCst);
    }
    spawned
}

/// Compatibility entry point: retire obsolete work without startup publication.
pub fn spawn_startup_reconciliation(db_path: std::path::PathBuf) -> bool {
    // The retired Atlas insight work lives in SQLite's task tables only.
    if crate::recon::backend::sqlite_path(&db_path).is_none() {
        return false;
    }
    if let Ok(store) = Store::open(&db_path) {
        let _ = tasks::retire_atlas_insight_work(&store.conn, &chrono::Utc::now().to_rfc3339());
    }
    false
}

/// Targeted re-extraction for a run whose payload was lost: rewinds it to the
/// phase-4 extraction step (retained articles are reused; no news is
/// re-fetched) and parks it so `run_atlas` can resume that run. Returns false
/// when no source articles are retained.
pub fn prepare_reextraction(store: &Store, run_id: &str) -> Result<bool> {
    ensure_historical_contract(store, run_id)?;
    let Some(run) = run_row(store, run_id)? else {
        return Ok(false);
    };
    if run.state == "running" || store.atlas_list_articles(run_id)?.is_empty() {
        return Ok(false);
    }
    let mut cursor: crate::atlas::Cursor =
        serde_json::from_str(&run.cursor_json).unwrap_or_default();
    cursor.phase = 4;
    cursor.leg = "insights".into();
    let mut stats: crate::atlas::RunStats =
        serde_json::from_str(&run.stats_json).unwrap_or_default();
    stats.memories = MemoryPhase::default();
    store.conn.execute(
        "DELETE FROM argos_atlas_checkpoints WHERE run_id=?1",
        [run_id],
    )?;
    store.atlas_save(
        run_id,
        &serde_json::to_string(&cursor)?,
        &serde_json::to_string(&stats)?,
    )?;
    store.atlas_set_state(run_id, "paused", "Re-extraction requested", false)?;
    Ok(true)
}

// ---------------------------------------------------------------------------
// Selected-store entry points. SQLite delegates to the functions above;
// SurrealDB runs the same steps against the repository (publication through
// `storage::insight::publish_atlas_insights`, reconstruction through
// `storage::provenance::legacy_receipt`).

use crate::atlas_store::AtlasStore;

/// [`save_checkpoint`] on the selected store.
pub async fn save_checkpoint_store(
    store: &AtlasStore,
    run_id: &str,
    input: CheckpointInput<'_>,
) -> Result<Checkpoint> {
    #[cfg(feature = "surreal")]
    if let Some(repo) = store.repository() {
        let checkpoint = checkpoint_of(run_id, input);
        crate::storage::atlas_memory::put_checkpoint(
            repo.db(),
            run_id,
            &checkpoint.revision,
            &serde_json::to_string(&checkpoint)?,
            checkpoint.claims.len() as i64,
        )
        .await?;
        return Ok(checkpoint);
    }
    store.sqlite("atlas save checkpoint", |s| {
        save_checkpoint(s, run_id, input)
    })
}

/// [`load_checkpoint`] on the selected store.
pub async fn load_checkpoint_store(store: &AtlasStore, run_id: &str) -> Result<Option<Checkpoint>> {
    #[cfg(feature = "surreal")]
    if let Some(repo) = store.repository() {
        return Ok(
            match crate::storage::atlas_memory::latest_checkpoint(repo.db(), run_id).await? {
                Some(json) => Some(serde_json::from_str(&json)?),
                None => None,
            },
        );
    }
    store.sqlite("atlas load checkpoint", |s| load_checkpoint(s, run_id))
}

/// [`publish_checkpoint`] on the selected store. Under SurrealDB the brief
/// summary flush (SQLite summarization queue) is not enqueued.
pub async fn publish_checkpoint_store(
    store: &AtlasStore,
    checkpoint: &Checkpoint,
    parent_job: Option<&str>,
    repair: bool,
    brief_override: Option<&str>,
) -> Result<PublicationReceipt> {
    #[cfg(feature = "surreal")]
    if let Some(repo) = store.repository() {
        let brief = brief_override.unwrap_or(&checkpoint.brief);
        return crate::storage::insight::publish_atlas_insights(
            repo.publisher(),
            &checkpoint.run_id,
            checkpoint.claims.clone(),
            checkpoint.relations.clone(),
            brief,
            PublishOptions {
                revision: Some(checkpoint.revision.clone()),
                parent_job: parent_job.map(str::to_string),
                repair,
                index_now: false,
                ..Default::default()
            },
        )
        .await
        .map_err(|e| anyhow::anyhow!("atlas publish checkpoint: {e}"));
    }
    store.sqlite("atlas publish checkpoint", |s| {
        publish_checkpoint(s, checkpoint, parent_job, repair, brief_override)
    })
}

/// [`legacy_receipt`] on the selected store.
pub async fn legacy_receipt_store(
    store: &AtlasStore,
    run_id: &str,
    parent_job: Option<&str>,
) -> Result<PublicationReceipt> {
    #[cfg(feature = "surreal")]
    if let Some(repo) = store.repository() {
        return crate::storage::provenance::legacy_receipt(repo.publisher(), run_id, parent_job)
            .await
            .map_err(|e| anyhow::anyhow!("atlas legacy receipt: {e}"));
    }
    store.sqlite("atlas legacy receipt", |s| {
        legacy_receipt(s, run_id, parent_job)
    })
}

async fn requeue_uncovered_store(
    store: &AtlasStore,
    coverage: &crate::store::CoverageReport,
    job_id: Option<&str>,
) -> Result<usize> {
    #[cfg(feature = "surreal")]
    if let Some(repo) = store.repository() {
        let _ = job_id;
        return crate::storage::atlas_memory::requeue_uncovered(repo.publisher(), coverage).await;
    }
    store.sqlite("atlas requeue", |s| {
        Ok(s.requeue_uncovered(coverage, job_id)?.len())
    })
}

async fn run_row_store(
    store: &AtlasStore,
    run_id: &str,
) -> Result<Option<crate::store::AtlasRunRow>> {
    Ok(store
        .list_runs()
        .await?
        .into_iter()
        .find(|run| run.id == run_id))
}

fn historical(run: &crate::store::AtlasRunRow) -> Result<()> {
    let stats: serde_json::Value = serde_json::from_str(&run.stats_json).unwrap_or_default();
    anyhow::ensure!(
        !stats
            .get("contract_version")
            .and_then(serde_json::Value::as_str)
            .is_some_and(crate::atlas_work::is_collection_contract),
        "Repair/Verify are restricted to historical Atlas contracts"
    );
    Ok(())
}

/// [`ensure_historical_contract`] on the selected store.
pub async fn ensure_historical_contract_store(store: &AtlasStore, run_id: &str) -> Result<()> {
    let Some(run) = run_row_store(store, run_id).await? else {
        anyhow::bail!("Atlas run {run_id} not found");
    };
    historical(&run)
}

/// [`index_and_verify`] on the selected store. Under SurrealDB the index
/// pool owns the work: round 0 re-queues uncovered memories, later rounds
/// only wait for it (there is no inline `index_now`).
pub async fn index_and_verify_store(
    store: &AtlasStore,
    receipt: &PublicationReceipt,
    pause: Option<&AtomicBool>,
    barrier: Barrier,
    mut progress: impl FnMut(u32, u32),
) -> Result<Phase5Report> {
    if !store.is_surreal() {
        return store.sqlite("atlas index and verify", |s| {
            index_and_verify(s, receipt, pause, barrier, progress)
        });
    }
    ensure_historical_contract_store(store, &receipt.run_id).await?;
    let mut round = 0u32;
    loop {
        let verification = store.verify_publication(receipt).await?;
        let coverage = &verification.coverage;
        progress(coverage.indexed.len() as u32, coverage.required as u32);
        if coverage.disabled || coverage.complete() || round >= barrier.rounds {
            return Ok(Phase5Report {
                verification,
                paused: false,
            });
        }
        if pause.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
            return Ok(Phase5Report {
                verification,
                paused: true,
            });
        }
        if round == 0 {
            requeue_uncovered_store(store, coverage, None).await?;
        }
        round += 1;
        if round < barrier.rounds {
            tokio::time::sleep(barrier.wait).await;
        }
    }
}

async fn record_repair_store(
    store: &AtlasStore,
    repair: &RunRepair,
    job_id: Option<&str>,
) -> Result<()> {
    #[cfg(feature = "surreal")]
    if let Some(repo) = store.repository() {
        let status = serde_json::to_value(repair.status)?
            .as_str()
            .unwrap_or("verified")
            .to_string();
        crate::storage::atlas::set_repair(
            repo.db(),
            &repair.run_id,
            &status,
            &serde_json::to_string(repair)?,
            job_id.unwrap_or_default(),
        )
        .await
        .map_err(|e| anyhow::anyhow!("atlas repair status: {e}"))?;
        if let Some(event) = repair_event(repair, &status, job_id)? {
            let _ = crate::storage::event_log::record_event(repo.db(), &event).await;
        }
        return Ok(());
    }
    store.sqlite("atlas record repair", |s| record_repair(s, repair, job_id))
}

/// [`repair_status`] on the selected store.
pub async fn repair_status_store(store: &AtlasStore, run_id: &str) -> Result<Option<RunRepair>> {
    #[cfg(feature = "surreal")]
    if let Some(repo) = store.repository() {
        let json = crate::storage::atlas_memory::repair_detail(repo.db(), run_id).await?;
        return Ok(json.and_then(|json| serde_json::from_str(&json).ok()));
    }
    store.sqlite("atlas repair status", |s| repair_status(s, run_id))
}

/// [`repair_run`] on the selected store: the CLI and the repair job use
/// this, so SurrealDB never reopens SQLite.
pub async fn repair_run_store(
    store: &AtlasStore,
    run_id: &str,
    job_id: Option<&str>,
) -> Result<RunRepair> {
    if !store.is_surreal() {
        return store.sqlite("atlas repair run", |s| repair_run(s, run_id, job_id));
    }
    #[cfg(feature = "surreal")]
    {
        let repo = store
            .repository()
            .ok_or_else(|| anyhow::anyhow!("atlas repair: no repository"))?;
        let db = repo.db();
        let Some(run) = run_row_store(store, run_id).await? else {
            anyhow::bail!("Atlas run {run_id} not found");
        };
        historical(&run)?;
        let mut out = RunRepair {
            run_id: run_id.into(),
            status: RepairStatus::Verified,
            restored_memory_ids: Vec::new(),
            requeued: 0,
            required: 0,
            indexed: 0,
            unrecoverable: Vec::new(),
            reextract_available: false,
            detail: String::new(),
        };
        if matches!(run.state.as_str(), "running" | "paused") {
            out.status = RepairStatus::Active;
            out.detail = "run is still active".into();
            return Ok(out);
        }
        let stats: crate::atlas::RunStats =
            serde_json::from_str(&run.stats_json).unwrap_or_default();
        let prior = store.publication_receipt(run_id).await?;
        let has_links = crate::storage::atlas_memory::has_insights(db, run_id).await?;
        let receipt = if let Some(checkpoint) = load_checkpoint_store(store, run_id).await? {
            let brief_deleted = match prior.as_ref().and_then(|r| r.brief_memory_id.clone()) {
                Some(id) => crate::storage::atlas_memory::is_tombstoned(db, &id).await?,
                None => false,
            };
            let receipt = publish_checkpoint_store(
                store,
                &checkpoint,
                job_id,
                true,
                brief_deleted.then_some(""),
            )
            .await?;
            out.restored_memory_ids
                .extend(receipt.created_memory_ids.iter().cloned());
            out.restored_memory_ids
                .extend(receipt.repaired_memory_ids.iter().cloned());
            receipt
        } else if has_links || prior.is_some() {
            legacy_receipt_store(store, run_id, job_id).await?
        } else {
            out.reextract_available = !store.list_articles(run_id).await?.is_empty();
            let produced = stats.insights.claims > 0
                || !stats.insights.rows.is_empty()
                || stats.memories.extracted > 0;
            out.status = if produced {
                out.detail = MISSING_PAYLOAD_LINE.into();
                RepairStatus::MissingPayload
            } else {
                RepairStatus::NoInsights
            };
            record_repair_store(store, &out, job_id).await?;
            return Ok(out);
        };
        for rejected in &receipt.rejected {
            if !rejected.reason.contains("deleted by user") {
                out.unrecoverable
                    .push((rejected.article_id.clone(), rejected.reason.clone()));
            }
        }
        let verification = store.verify_publication(&receipt).await?;
        let requeued = requeue_uncovered_store(store, &verification.coverage, job_id).await?;
        out.requeued = requeued + receipt.queued_index_changes.len();
        out.required = verification.coverage.required;
        out.indexed = verification.coverage.indexed.len();
        out.status = if !out.unrecoverable.is_empty() {
            out.reextract_available = !store.list_articles(run_id).await?.is_empty();
            out.detail = format!(
                "{} claims unrecoverable; {MISSING_PAYLOAD_LINE}",
                out.unrecoverable.len()
            );
            RepairStatus::Partial
        } else if !out.restored_memory_ids.is_empty() {
            RepairStatus::Repaired
        } else if verification.coverage.disabled {
            out.detail = INDEXING_DISABLED_LINE.into();
            RepairStatus::IndexingDisabled
        } else if out.requeued > 0 || !verification.coverage.complete() {
            RepairStatus::IndexQueued
        } else {
            RepairStatus::Verified
        };
        if let Some(run) = run_row_store(store, run_id).await? {
            let _ = refresh_run_indexing_store(store, &run).await;
        }
        record_repair_store(store, &out, job_id).await?;
        Ok(out)
    }
    #[cfg(not(feature = "surreal"))]
    unreachable!("SurrealDB store without the surreal feature")
}

async fn load_progress_store(store: &AtlasStore) -> Result<RepairProgress> {
    Ok(store
        .app_state_get(REPAIR_PROGRESS_KEY)
        .await?
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default())
}

async fn save_progress_store(store: &AtlasStore, progress: &RepairProgress) -> Result<()> {
    store
        .app_state_set(REPAIR_PROGRESS_KEY, &serde_json::to_string(progress)?)
        .await
}

/// [`repair_page`] on the selected store. Under SurrealDB the keyset page
/// is taken from `list_runs` (historical contracts, `(started_at, id)`
/// ascending); SQLite's `retire_atlas_insight_work` and job registration
/// have no SurrealDB counterpart here.
pub async fn repair_page_store(
    store: &AtlasStore,
    progress: &mut RepairProgress,
    page: usize,
) -> Result<Vec<RunRepair>> {
    if !store.is_surreal() {
        return store.sqlite("atlas repair page", |s| repair_page(s, progress, page));
    }
    let mut runs: Vec<(String, String)> = store
        .list_runs()
        .await?
        .into_iter()
        .filter(|run| historical(run).is_ok())
        .map(|run| (run.started_at, run.id))
        .filter(|(started, id)| {
            (started.as_str(), id.as_str())
                > (
                    progress.after_started_at.as_str(),
                    progress.after_id.as_str(),
                )
        })
        .collect();
    runs.sort();
    runs.truncate(page.max(1));
    let mut results = Vec::new();
    if runs.is_empty() {
        progress.done = true;
        save_progress_store(store, progress).await?;
        return Ok(results);
    }
    for (started_at, id) in runs {
        let job = Some(progress.job_id.clone()).filter(|j| !j.is_empty());
        let repair = repair_run_store(store, &id, job.as_deref()).await?;
        progress.runs_checked += 1;
        if !matches!(
            repair.status,
            RepairStatus::Verified | RepairStatus::NoInsights | RepairStatus::Active
        ) {
            progress.runs_repaired += 1;
        }
        progress.memories_restored += repair.restored_memory_ids.len() as u32;
        progress.index_queued += repair.requeued as u32;
        progress.unrecoverable += repair.unrecoverable.len() as u32;
        if (repair.status == RepairStatus::MissingPayload || repair.status == RepairStatus::Partial)
            && !progress.missing_payload.contains(&id)
        {
            progress.missing_payload.push(id.clone());
        }
        progress.after_started_at = started_at;
        progress.after_id = id;
        results.push(repair);
    }
    save_progress_store(store, progress).await?;
    Ok(results)
}

/// [`run_repair_job`] on the selected store. Under SurrealDB progress is
/// kept in `app_state` only; the job is not mirrored into the registry.
pub async fn run_repair_job_store(
    store: &AtlasStore,
    page: usize,
    restart: bool,
) -> Result<RepairProgress> {
    if !store.is_surreal() {
        return store.sqlite("atlas repair job", |s| run_repair_job(s, page, restart));
    }
    let mut progress = load_progress_store(store).await?;
    if restart || progress.done || progress.job_id.is_empty() {
        progress = RepairProgress {
            job_id: format!("atlas-repair-{}", chrono::Utc::now().timestamp_millis()),
            ..Default::default()
        };
    }
    save_progress_store(store, &progress).await?;
    while !repair_page_store(store, &mut progress, page.max(1))
        .await?
        .is_empty()
    {}
    #[cfg(feature = "surreal")]
    if progress.memories_restored > 0 {
        if let Some(repo) = store.repository() {
            let _ = crate::storage::atlas_memory::bump_memories_changed(repo.publisher()).await;
        }
    }
    Ok(progress)
}

/// [`prepare_reextraction`] on the selected store.
pub async fn prepare_reextraction_store(store: &AtlasStore, run_id: &str) -> Result<bool> {
    if !store.is_surreal() {
        return store.sqlite("atlas prepare reextraction", |s| {
            prepare_reextraction(s, run_id)
        });
    }
    let Some(run) = run_row_store(store, run_id).await? else {
        anyhow::bail!("Atlas run {run_id} not found");
    };
    historical(&run)?;
    if run.state == "running" || store.list_articles(run_id).await?.is_empty() {
        return Ok(false);
    }
    let mut cursor: crate::atlas::Cursor =
        serde_json::from_str(&run.cursor_json).unwrap_or_default();
    cursor.phase = 4;
    cursor.leg = "insights".into();
    let mut stats: crate::atlas::RunStats =
        serde_json::from_str(&run.stats_json).unwrap_or_default();
    stats.memories = MemoryPhase::default();
    #[cfg(feature = "surreal")]
    if let Some(repo) = store.repository() {
        crate::storage::atlas_memory::delete_checkpoints(repo.db(), run_id).await?;
    }
    store
        .save(
            run_id,
            &serde_json::to_string(&cursor)?,
            &serde_json::to_string(&stats)?,
        )
        .await?;
    store
        .set_state(run_id, "paused", "Re-extraction requested", false)
        .await?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atlas::{Cursor, RunStats};
    use crate::atlas_insights::{Extraction, Settled};
    use crate::embed::testing as embed_testing;
    use crate::store::AtlasArticleRow;

    fn claim(entity: &str, object: &str, article: &str) -> AtlasInsightClaim {
        AtlasInsightClaim {
            fingerprint: String::new(),
            entity: entity.into(),
            namespace: "org".into(),
            predicate: "located_in".into(),
            object: object.into(),
            topic: "geo".into(),
            claim: format!("{entity} is located in {object}."),
            classification: "inference".into(),
            confidence: 0.7,
            article_id: article.into(),
            source_url: format!("https://news.example/{article}"),
            published_at: "2026-10-05".into(),
            reliability: "B".into(),
            info_credibility: 2,
            admiralty: "B2".into(),
            rsp_status: String::new(),
        }
    }

    fn claims() -> Vec<AtlasInsightClaim> {
        vec![
            claim("acme", "lisbon", "art-1"),
            claim("globex", "porto", "art-1"),
        ]
    }

    fn extraction(claims: Vec<AtlasInsightClaim>, partial: bool) -> Extraction {
        let stats = InsightStats {
            claims: claims.len() as u32,
            inferences: claims.len() as u32,
            ..Default::default()
        };
        Extraction {
            settled: Settled {
                stats,
                claims,
                relations: Vec::new(),
                brief: "Cycle brief: Acme and Globex expand in Portugal.".into(),
                entity_path: String::new(),
                initial_confidences: std::collections::BTreeMap::new(),
            },
            extract_error: partial.then(|| "packet 2 timed out".to_string()),
            peer_error: None,
            context_error: None,
            warnings: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    // ---- Repair Atlas memories -------------------------------------------

    fn finished_run(store: &Store, id: &str, started: &str, stats: &RunStats) {
        let cursor = Cursor {
            phase: 4,
            leg: "insights_done".into(),
            ..Cursor::default()
        };
        store
            .atlas_insert_run(
                id,
                &serde_json::to_string(&cursor).unwrap(),
                &serde_json::to_string(stats).unwrap(),
            )
            .unwrap();
        store.atlas_set_state(id, "completed", "", true).unwrap();
        store
            .conn
            .execute(
                "UPDATE atlas_runs SET started_at=?1 WHERE id=?2",
                params![started, id],
            )
            .unwrap();
    }

    /// A pre-phase-5 run: links and memories exist, but no receipt, no
    /// checkpoint and no index state (false "completed").
    fn legacy_published(store: &Store, id: &str) -> PublicationReceipt {
        let receipt = store
            .publish_atlas_insights(
                id,
                &claims(),
                &[],
                "Legacy brief.",
                &PublishOptions::default(),
            )
            .unwrap();
        store
            .conn
            .execute_batch(
                "DELETE FROM argos_atlas_publications; DELETE FROM argos_memory_index_state;
                 DELETE FROM argos_index_changes; DELETE FROM argos_tasks;",
            )
            .unwrap();
        receipt
    }

    fn disk_store(dir: &tempfile::TempDir) -> Store {
        Store::open(&dir.path().join("argos.db")).unwrap()
    }

    #[cfg(feature = "lancedb")]
    #[test]
    fn repair_requeues_vectors_for_a_false_completed_cycle_without_rewriting_it() {
        let _fake = embed_testing::fake();
        let dir = tempfile::tempdir().unwrap();
        let store = disk_store(&dir);
        finished_run(&store, "old", "2026-10-04T01:00:00Z", &RunStats::default());
        let published = legacy_published(&store, "old");
        let repair = repair_run(&store, "old", None).unwrap();
        assert_eq!(repair.status, RepairStatus::IndexQueued, "{repair:?}");
        assert_eq!(repair.required, 3);
        assert!(repair.unrecoverable.is_empty());
        let conn = rusqlite::Connection::open(dir.path().join("argos.db")).unwrap();
        crate::scheduler::drain_index_once(&conn, &store, "pool", 16).unwrap();
        let receipt = store.atlas_publication_receipt("old").unwrap().unwrap();
        assert_eq!(receipt.revision, LEGACY_REVISION);
        assert!(receipt.reconciles());
        let coverage = store
            .verify_memory_coverage(&published.affected_memory_ids)
            .unwrap();
        assert!(coverage.complete(), "{coverage:?}");
        // Historical state is untouched; repair status is recorded separately.
        let run = store.atlas_latest_run().unwrap().unwrap();
        assert_eq!((run.state.as_str(), run.note.as_str()), ("completed", ""));
        assert_eq!(
            repair_status(&store, "old").unwrap().unwrap().status,
            RepairStatus::IndexQueued
        );
        // A second pass is verified and idempotent.
        assert_eq!(
            repair_run(&store, "old", None).unwrap().status,
            RepairStatus::Verified
        );
    }

    #[test]
    fn repair_restores_orphaned_links_from_the_checkpoint_and_respects_tombstones() {
        let _fake = embed_testing::fake();
        let dir = tempfile::tempdir().unwrap();
        let store = disk_store(&dir);
        finished_run(&store, "cp", "2026-10-04T02:00:00Z", &RunStats::default());
        let x = extraction(claims(), false);
        let cp = save_checkpoint(
            &store,
            "cp",
            CheckpointInput {
                claims: &x.settled.claims,
                relations: &[],
                brief: &x.settled.brief,
                entity_path: "",
                stats: &x.settled.stats,
                partial: false,
                notes: Vec::new(),
                initial_confidences: x.settled.initial_confidences.clone(),
            },
        )
        .unwrap();
        let receipt = publish_checkpoint(&store, &cp, None, false, None).unwrap();
        let (fp_a, mem_a) = receipt.claim_memory_ids[0].clone();
        let (_, mem_b) = receipt.claim_memory_ids[1].clone();
        // Damage: claim A lost its claim row and memory (orphan source link).
        store.conn.execute_batch("PRAGMA foreign_keys=OFF").unwrap();
        store
            .conn
            .execute("DELETE FROM insight_claims WHERE fingerprint=?1", [&fp_a])
            .unwrap();
        store
            .conn
            .execute("DELETE FROM memories WHERE id=?1", [&mem_a])
            .unwrap();
        store.conn.execute_batch("PRAGMA foreign_keys=ON").unwrap();
        // User deliberately deleted claim B.
        store.delete_memory(&mem_b).unwrap();
        let repair = repair_run(&store, "cp", None).unwrap();
        assert_eq!(repair.status, RepairStatus::Repaired, "{repair:?}");
        assert_eq!(repair.restored_memory_ids.len(), 1);
        let texts: Vec<String> = store
            .list_memories()
            .unwrap()
            .into_iter()
            .map(|m| m.text)
            .collect();
        assert!(texts.iter().any(|t| t == "acme is located in lisbon."));
        assert!(
            !texts.iter().any(|t| t == "globex is located in porto."),
            "a user deletion is never resurrected"
        );
        assert!(
            repair.unrecoverable.is_empty(),
            "tombstones are not gaps: {repair:?}"
        );
    }

    #[test]
    fn repair_reports_missing_payload_and_unrecoverable_links_without_fabricating() {
        let _fake = embed_testing::fake();
        let dir = tempfile::tempdir().unwrap();
        let store = disk_store(&dir);
        // (a) Insights counted, but the failed write lost every claim.
        let mut stats = RunStats::default();
        stats.insights.claims = 4;
        finished_run(&store, "lost", "2026-10-04T03:00:00Z", &stats);
        store
            .atlas_upsert_article(&AtlasArticleRow {
                run_id: "lost".into(),
                id: "a".into(),
                title: "t".into(),
                description: String::new(),
                url: "https://x.example/a".into(),
                country: "pt".into(),
                source_name: String::new(),
                source_domain: String::new(),
                published_at: String::new(),
                provider: "newsapi".into(),
                temperature: 1.0,
                category: "economic".into(),
                seen_at: String::new(),
                author: String::new(),
                image_url: String::new(),
            })
            .unwrap();
        let repair = repair_run(&store, "lost", None).unwrap();
        assert_eq!(repair.status, RepairStatus::MissingPayload);
        assert_eq!(repair.detail, MISSING_PAYLOAD_LINE);
        assert!(repair.reextract_available);
        assert!(prepare_reextraction(&store, "lost").unwrap());
        let run = store
            .atlas_list_runs()
            .unwrap()
            .into_iter()
            .find(|r| r.id == "lost")
            .unwrap();
        let cursor: Cursor = serde_json::from_str(&run.cursor_json).unwrap();
        assert_eq!(
            (run.state.as_str(), cursor.phase, cursor.leg.as_str()),
            ("paused", 4, "insights")
        );

        // (b) Links survive but the canonical memory text is gone, no checkpoint.
        finished_run(&store, "gap", "2026-10-04T04:00:00Z", &RunStats::default());
        let published = legacy_published(&store, "gap");
        let (_, gone) = published.claim_memory_ids[0].clone();
        store.conn.execute_batch("PRAGMA foreign_keys=OFF").unwrap();
        store
            .conn
            .execute("DELETE FROM memories WHERE id=?1", [&gone])
            .unwrap();
        store.conn.execute_batch("PRAGMA foreign_keys=ON").unwrap();
        let before = store.list_memories().unwrap().len();
        let repair = repair_run(&store, "gap", None).unwrap();
        assert_eq!(repair.status, RepairStatus::Partial, "{repair:?}");
        assert_eq!(repair.unrecoverable.len(), 1);
        assert!(repair.unrecoverable[0].1.contains("re-extraction required"));
        assert_eq!(
            store.list_memories().unwrap().len(),
            before,
            "nothing fabricated"
        );
    }

    #[test]
    fn repair_job_pages_resumably_and_records_job_progress() {
        let _fake = embed_testing::fake();
        let dir = tempfile::tempdir().unwrap();
        let store = disk_store(&dir);
        for (i, id) in ["r1", "r2", "r3"].iter().enumerate() {
            finished_run(
                &store,
                id,
                &format!("2026-10-04T0{i}:00:00Z"),
                &RunStats::default(),
            );
        }
        legacy_published(&store, "r2");
        let mut progress = RepairProgress {
            job_id: "job-test".into(),
            ..Default::default()
        };
        save_progress(&store, &progress).unwrap();
        let page = repair_page(&store, &mut progress, 2).unwrap();
        assert_eq!(page.len(), 2);
        assert_eq!(progress.after_id, "r2");
        // Interrupted here: the stored cursor resumes at r3.
        let stored = load_progress(&store).unwrap();
        assert_eq!((stored.after_id.as_str(), stored.runs_checked), ("r2", 2));
        let done = run_repair_job(&store, 2, false).unwrap();
        assert!(done.done);
        assert_eq!(
            done.runs_checked, 3,
            "resumed pass covers only r3: {done:?}"
        );
        let (state, title): (String, String) = store
            .conn
            .query_row(
                "SELECT state, title FROM argos_jobs WHERE id=?1",
                [&done.job_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            (state.as_str(), title.as_str()),
            ("completed", "Repair Atlas memories")
        );
        // Owned by this process; its running span is folded into active time.
        let row = store.get_job(&done.job_id).unwrap().unwrap();
        assert_eq!(row.active_since, "");
        assert!(row.active_ms.is_some(), "active time recorded");
        assert_eq!(row.active_now(chrono::Utc::now()), row.active_ms);
        // A repair that dies mid-pass does not stay "running".
        store
            .conn
            .execute(
                "UPDATE argos_jobs SET state='running', finished_at='' WHERE id=?1",
                [&done.job_id],
            )
            .unwrap();
        mark_repair_interrupted(&store, "repair stopped by a panic");
        assert_eq!(
            store.get_job(&done.job_id).unwrap().unwrap().state,
            "failed"
        );
        // User-triggered repair starts a fresh pass.
        let fresh = run_repair_job(&store, 10, true).unwrap();
        assert_ne!(fresh.job_id, done.job_id);
        assert_eq!(fresh.runs_checked, 3);
    }

    #[test]
    fn memory_phase_lines_and_outcomes_are_truthful() {
        let mut phase = MemoryPhase {
            extraction: StepState::Completed,
            publication: StepState::Completed,
            indexing: StepState::Completed,
            created: 18,
            reused: 4,
            accepted: 22,
            indexed: 22,
            required: 22,
            ..Default::default()
        };
        assert_eq!(
            phase.line(),
            "Memories: 18 new claims · 4 reused · 0 brief · 22/22 index ready"
        );
        assert_eq!(phase.outcome(), CycleOutcome::Completed);
        phase.indexing = StepState::Blocked;
        assert_eq!(phase.outcome(), CycleOutcome::Blocked);
        assert!(phase.line().ends_with(INDEXING_DISABLED_LINE));
        phase.publication = StepState::Failed;
        assert_eq!(phase.outcome(), CycleOutcome::Failed);
        assert!(MemoryPhase::default().line().is_empty());
        // Old stats JSON without the field still parses.
        let old: RunStats =
            serde_json::from_str(r#"{"counts":{},"origins":[],"scored":true}"#).unwrap();
        assert!(!old.memories.started());
    }
}
