//! Recon storage dispatch (migration phase 5, Recon switch step 3).
//!
//! Core Recon callers talk to [`ReconStore`]: SQLite (`Store`) unless the
//! process selected SurrealDB (`storage::runtime::configured`), in which
//! case every call goes to the repository and SQLite is never opened. The
//! two stores never both serve a process.
//!
//! The SQLite store is not `Sync`, so it sits behind a mutex that is only
//! held inside one synchronous call; futures that hold a [`ReconStore`]
//! stay `Send`. Operations with no SurrealDB port fail with
//! "… is not on SurrealDB yet" instead of silently touching SQLite.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use anyhow::{anyhow, Result};
use serde_json::Value;

use super::{Call, CreditHold, Message, Plan, Run, RunLimits, Thread};
use crate::osint::ToolResult;
use crate::provider::ReconLimits;
use crate::store::Store;

/// Extraction contract of SurrealDB answers: claims mined from the stored
/// answer and its evidence (`storage::recon_extract::persist_claims`).
pub const SURREAL_MINED: &str = "surreal-mined-v1";

/// Whether this process keeps Recon on SurrealDB.
pub fn surreal_selected() -> bool {
    #[cfg(feature = "surreal")]
    {
        crate::storage::runtime::configured()
    }
    #[cfg(not(feature = "surreal"))]
    {
        false
    }
}

/// The SQLite path for SQLite-only side stores (telemetry, model attempts,
/// job registry, summarization cache): `None` under SurrealDB, so those
/// writers are skipped rather than reopening SQLite.
pub fn sqlite_path(db_path: &Path) -> Option<&Path> {
    (!surreal_selected()).then_some(db_path)
}

/// The Recon store a caller uses for one unit of work.
pub enum ReconStore {
    Sqlite(Mutex<Store>),
    #[cfg(feature = "surreal")]
    Surreal(crate::storage::repo::Repository),
}

/// Error for an operation that is still SQLite-only.
pub fn not_ported(op: &str) -> anyhow::Error {
    anyhow!("{op} is not on SurrealDB yet")
}

macro_rules! dispatch {
    ($self:ident, $s:ident => $sqlite:expr, $r:ident => $surreal:expr) => {
        match $self {
            ReconStore::Sqlite(m) => {
                let $s = lock(m);
                $sqlite
            }
            #[cfg(feature = "surreal")]
            ReconStore::Surreal($r) => $surreal,
        }
    };
}

fn lock(m: &Mutex<Store>) -> MutexGuard<'_, Store> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl ReconStore {
    /// The selected store: the process repository under SurrealDB,
    /// otherwise `Store::open(db_path)`.
    pub async fn open(db_path: &Path) -> Result<Self> {
        #[cfg(feature = "surreal")]
        if surreal_selected() {
            return Ok(Self::Surreal(crate::storage::runtime::repository().await?));
        }
        Ok(Self::Sqlite(Mutex::new(Store::open(db_path)?)))
    }

    /// Wrap an open SQLite store.
    pub fn from_sqlite(store: Store) -> Self {
        Self::Sqlite(Mutex::new(store))
    }

    /// Wrap a repository (tests, and callers that already hold one).
    #[cfg(feature = "surreal")]
    pub fn from_repository(repo: crate::storage::repo::Repository) -> Self {
        Self::Surreal(repo)
    }

    pub fn is_surreal(&self) -> bool {
        !matches!(self, Self::Sqlite(_))
    }

    /// Run `f` on the SQLite store, or fail with `op` not ported.
    pub fn sqlite<T>(&self, op: &str, f: impl FnOnce(&mut Store) -> Result<T>) -> Result<T> {
        match self {
            Self::Sqlite(m) => {
                let _ = op;
                f(&mut lock(m))
            }
            #[cfg(feature = "surreal")]
            Self::Surreal(_) => Err(not_ported(op)),
        }
    }

    /// The repository under SurrealDB.
    #[cfg(feature = "surreal")]
    pub fn repository(&self) -> Option<&crate::storage::repo::Repository> {
        match self {
            Self::Surreal(r) => Some(r),
            Self::Sqlite(_) => None,
        }
    }

    /// The thread's investigation trace (`investigation_events`).
    pub async fn list_investigation_events(
        &self,
        tid: &str,
    ) -> Result<Vec<crate::investigation::InvestigationEvent>> {
        dispatch!(self, s => s.list_investigation_events(tid),
            r => crate::storage::event_log::list_investigation_events(r.db(), tid).await)
    }
    /// Append to the investigation trace (insert, or update an existing id).
    pub async fn insert_investigation_event(
        &self,
        ev: &crate::investigation::InvestigationEvent,
    ) -> Result<()> {
        dispatch!(self, s => s.insert_investigation_event(ev),
            r => crate::storage::event_log::insert_investigation_event(r.db(), ev).await)
    }
    pub async fn next_event_sequence(&self, tid: &str) -> Result<i64> {
        dispatch!(self, s => s.next_event_sequence(tid),
            r => crate::storage::event_log::next_event_sequence(r.db(), tid).await)
    }
    /// Persist one redacted durable event (`argos_events`).
    pub async fn record_event(&self, event: &crate::events::NewEvent) -> Result<String> {
        dispatch!(self, s => crate::events::record_event(s.connection(), event),
            r => crate::storage::event_log::record_event(r.db(), event).await)
    }
    /// Newest-first page of durable events.
    pub async fn list_events(
        &self,
        filter: &crate::events::EventFilter,
        limit: usize,
    ) -> Result<Vec<crate::events::EventRow>> {
        dispatch!(self, s => crate::events::list_events(s.connection(), filter, limit),
            r => crate::storage::event_log::list_events(r.db(), filter, limit).await)
    }

    pub async fn new_thread(&self, title: &str) -> Result<Thread> {
        dispatch!(self, s => s.new_thread(title),
            r => crate::storage::recon_threads::new_thread(r, title).await)
    }
    pub async fn list_threads(&self, search: &str) -> Result<Vec<Thread>> {
        dispatch!(self, s => s.list_threads(search),
            r => crate::storage::recon_threads::list_threads(r, search).await)
    }
    pub async fn get_thread(&self, tid: &str) -> Result<Option<Thread>> {
        dispatch!(self, s => s.get_thread(tid),
            r => crate::storage::recon::get_thread(r, tid).await)
    }
    pub async fn set_recall_insights(&self, tid: &str, on: bool) -> Result<bool> {
        dispatch!(self, s => s.set_recall_insights(tid, on),
            r => crate::storage::recon_threads::set_recall_insights(r, tid, on).await)
    }
    pub async fn rename_thread(&self, tid: &str, title: &str) -> Result<bool> {
        dispatch!(self, s => s.rename_thread(tid, title),
            r => crate::storage::recon_threads::rename_thread(r, tid, title).await)
    }
    pub async fn list_messages(&self, tid: &str) -> Result<Vec<Message>> {
        dispatch!(self, s => s.list_messages(tid),
            r => crate::storage::recon_threads::list_messages(r, tid).await)
    }
    pub async fn add_message(
        &self,
        tid: &str,
        role: &str,
        content: &str,
        run_id: Option<&str>,
    ) -> Result<Message> {
        dispatch!(self, s => s.add_message(tid, role, content, run_id),
            r => crate::storage::recon_threads::add_message(r, tid, role, content, run_id).await)
    }
    pub async fn add_answer(
        &self,
        tid: &str,
        run_id: &str,
        content: &str,
        evidence_ids: &[String],
        memory_ids: &[String],
    ) -> Result<Message> {
        dispatch!(self, s => { let mut s = s; s.add_answer(tid, run_id, content, evidence_ids, memory_ids) },
            r => crate::storage::recon_exec::add_answer(r, tid, run_id, content, evidence_ids, memory_ids).await)
    }
    pub async fn has_final_answer_for_run(&self, run_id: &str) -> Result<bool> {
        dispatch!(self, s => s.has_final_answer_for_run(run_id),
            r => crate::storage::recon_exec::has_final_answer_for_run(r, run_id).await)
    }
    pub async fn answer_evidence(&self, answer_id: &str) -> Result<Vec<(String, ToolResult)>> {
        dispatch!(self, s => s.answer_evidence(answer_id),
            r => crate::storage::recon_runs::answer_evidence(r, answer_id).await)
    }
    pub async fn answer_memories(
        &self,
        tid: &str,
    ) -> Result<HashMap<String, Vec<crate::brain::Memory>>> {
        dispatch!(self, s => s.answer_memories(tid),
            r => crate::storage::recon::answer_memories(r, tid).await)
    }
    pub async fn new_run_with_models(
        &self,
        tid: &str,
        turn_id: &str,
        models: [&str; 3],
        limits: RunLimits,
    ) -> Result<Run> {
        dispatch!(self, s => s.new_run_with_models(tid, turn_id, models, limits),
            r => crate::storage::recon_runs::new_run(r, tid, turn_id, models, limits).await)
    }
    pub async fn get_run(&self, rid: &str) -> Result<Option<Run>> {
        dispatch!(self, s => s.get_run(rid),
            r => crate::storage::recon_runs::get_run(r, rid).await)
    }
    pub async fn latest_resumable_run(&self, tid: &str) -> Result<Option<Run>> {
        dispatch!(self, s => s.latest_resumable_run(tid),
            r => crate::storage::recon_runs::latest_resumable_run(r, tid).await)
    }
    pub async fn runs_for_thread(&self, tid: &str) -> Result<Vec<Run>> {
        dispatch!(self, s => s.runs_for_thread(tid),
            r => crate::storage::recon_runs::runs_for_thread(r, tid).await)
    }
    pub async fn set_run(
        &self,
        rid: &str,
        state: &str,
        stage: &str,
        plan: Option<&Plan>,
        error: Option<&str>,
    ) -> Result<bool> {
        dispatch!(self, s => s.set_run(rid, state, stage, plan, error),
            r => crate::storage::recon_runs::set_run(r, rid, state, stage, plan, error).await)
    }
    pub async fn cancel_run(&self, rid: &str) -> Result<bool> {
        dispatch!(self, s => s.cancel_run(rid),
            r => crate::storage::recon_runs::cancel_run(r, rid).await)
    }
    pub async fn restart_run(&self, rid: &str) -> Result<bool> {
        dispatch!(self, s => s.restart_run(rid),
            r => crate::storage::recon_runs::restart_run(r, rid).await)
    }
    pub async fn recover_runs(&self) -> Result<usize> {
        dispatch!(self, s => s.recover_runs(),
            r => crate::storage::recon_exec::recover_runs(r).await)
    }
    pub async fn calls_for_run(&self, rid: &str) -> Result<Vec<Call>> {
        dispatch!(self, s => s.calls_for_run(rid),
            r => crate::storage::recon_runs::calls(r, crate::storage::recon_runs::CallFilter::Run(rid)).await)
    }
    #[allow(clippy::too_many_arguments)]
    pub async fn queue_call(
        &self,
        tool_id: &str,
        inputs: &Value,
        origin: &str,
        run_id: Option<&str>,
        thread_id: Option<&str>,
        turn_id: Option<&str>,
    ) -> Result<String> {
        dispatch!(self, s => s.queue_call(tool_id, inputs, origin, run_id, thread_id, turn_id),
            r => crate::storage::recon_runs::queue_call(r, tool_id, inputs, origin, run_id, thread_id, turn_id).await)
    }
    pub async fn finish_call(&self, call_id: &str, result: &ToolResult) -> Result<bool> {
        dispatch!(self, s => s.finish_call(call_id, result),
            r => crate::storage::recon_runs::finish_call(r, call_id, result).await)
    }
    pub async fn attach_call(&self, call_id: &str, tid: &str) -> Result<bool> {
        dispatch!(self, s => s.attach_call(call_id, tid),
            r => crate::storage::recon_runs::attach_call(r, call_id, tid).await)
    }
    pub async fn inflight_duplicate(&self, tool_id: &str, inputs: &Value) -> Result<bool> {
        dispatch!(self, s => s.inflight_duplicate(tool_id, inputs),
            r => crate::storage::recon_runs::inflight_duplicate(r, tool_id, inputs).await)
    }
    /// A run's open calls settled as `cancelled` or `interrupted`.
    pub async fn settle_open_calls(&self, run_id: &str, status: &str) -> Result<usize> {
        dispatch!(self, s => {
            anyhow::ensure!(matches!(status, "cancelled" | "interrupted"), "unexpected call settlement {status}");
            Ok(s.conn.execute(
                "UPDATE osint_calls SET status=?1,completed_at=?2 WHERE run_id=?3 AND status IN ('queued','running')",
                rusqlite::params![status, super::now(), run_id],
            )?)
        }, r => crate::storage::recon_runs::settle_open_calls(r, run_id, status).await)
    }
    pub async fn cache_get(&self, key: &str) -> Result<Option<ToolResult>> {
        self.cache_get_fresh(key, None).await
    }
    pub async fn cache_get_fresh(
        &self,
        key: &str,
        cutoff: Option<&str>,
    ) -> Result<Option<ToolResult>> {
        dispatch!(self, s => s.cache_get_fresh(key, cutoff),
            r => crate::storage::recon_runs::cache_get_fresh(r, key, cutoff).await)
    }
    pub async fn cache_put(&self, key: &str, result: &ToolResult, ttl: u64) -> Result<()> {
        dispatch!(self, s => s.cache_put(key, result, ttl),
            r => crate::storage::recon_runs::cache_put(r, key, result, ttl).await)
    }
    pub async fn record_strategy(
        &self,
        thread_id: &str,
        run_id: &str,
        kind: &str,
        rationale: &str,
        previous_kind: Option<&str>,
        change_reason: Option<&str>,
    ) -> Result<()> {
        dispatch!(self, s => s.record_strategy(thread_id, run_id, kind, rationale, previous_kind, change_reason),
            r => crate::storage::recon_exec::record_strategy(r, thread_id, run_id, kind, rationale, previous_kind, change_reason).await)
    }
    pub async fn latest_strategy_kind(&self, thread_id: &str) -> Result<Option<String>> {
        dispatch!(self, s => s.latest_strategy_kind(thread_id),
            r => crate::storage::recon_exec::latest_strategy_kind(r, thread_id).await)
    }
    pub async fn credits_available(&self, provider: &str, limits: &ReconLimits) -> Result<u32> {
        dispatch!(self, s => s.credits_available(provider, limits),
            r => crate::storage::credits::credits_available(r.db(), provider, limits).await)
    }
    pub async fn reserve_credits(
        &self,
        provider: &str,
        cost: u32,
        limits: &ReconLimits,
    ) -> Result<Option<CreditHold>> {
        dispatch!(self, s => s.reserve_credits(provider, cost, limits),
            r => crate::storage::credits::reserve_credits(r.db(), provider, cost, limits).await)
    }
    pub async fn reconcile_credits(&self, hold: &CreditHold, actual: u32) -> Result<()> {
        dispatch!(self, s => s.reconcile_credits(hold, actual),
            r => crate::storage::credits::reconcile_credits(r.db(), hold, actual).await)
    }
    pub async fn release_credits(&self, hold: &CreditHold) -> Result<()> {
        dispatch!(self, s => s.release_credits(hold),
            r => crate::storage::credits::release_credits(r.db(), hold).await)
    }
    pub async fn tool_enabled(&self, id: &str) -> Result<bool> {
        dispatch!(self, s => s.tool_enabled(id),
            r => crate::storage::recon_threads::tool_enabled(r, id).await)
    }
    pub async fn set_tool_enabled(&self, id: &str, enabled: bool) -> Result<()> {
        dispatch!(self, s => s.set_tool_enabled(id, enabled),
            r => crate::storage::recon_threads::set_tool_enabled(r, id, enabled).await)
    }
    pub async fn link_entity(
        &self,
        tid: &str,
        kind: &str,
        canonical: &str,
        source_call_id: Option<&str>,
    ) -> Result<()> {
        dispatch!(self, s => s.link_entity(tid, kind, canonical, source_call_id),
            r => crate::storage::recon_threads::link_entity(r, tid, kind, canonical, source_call_id).await)
    }
    pub async fn thread_entities(&self, tid: &str) -> Result<Vec<(String, String)>> {
        dispatch!(self, s => s.thread_entities(tid),
            r => crate::storage::recon_threads::thread_entities(r, tid).await)
    }
    /// The newest synthesis operation's generation and draft.
    pub async fn synthesis_operation(&self, run_id: &str) -> Result<Option<(i64, String)>> {
        dispatch!(self, s => {
            use rusqlite::OptionalExtension;
            Ok(s.conn.query_row(
                "SELECT generation, draft FROM recon_model_operations WHERE run_id=?1 AND role='synthesis' ORDER BY generation DESC LIMIT 1",
                [run_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            ).optional()?)
        }, r => Ok(crate::storage::recon_exec::operations(r, run_id, "synthesis").await?
            .into_iter().next().map(|op| (op.generation, op.draft))))
    }
    /// Replace the run's synthesis drafts (a cut-short answer).
    pub async fn set_synthesis_drafts(&self, run_id: &str, draft: &str) -> Result<()> {
        dispatch!(self, s => {
            s.conn.execute(
                "UPDATE recon_model_operations SET draft = ?1, updated_at = ?2 WHERE run_id = ?3 AND role = 'synthesis'",
                rusqlite::params![draft, super::now(), run_id],
            )?;
            Ok(())
        }, r => crate::storage::recon_exec::set_run_drafts(r, run_id, "synthesis", draft).await)
    }
    /// Set an answer's extraction job state (`running`, `completed`,
    /// `skipped`, `failed` with `error`).
    pub async fn set_extraction(
        &self,
        answer_id: &str,
        state: &str,
        error: Option<&str>,
    ) -> Result<()> {
        dispatch!(self, s => {
            s.conn.execute(
                "UPDATE extraction_jobs SET state=?1,error=?2,updated_at=?3 WHERE answer_id=?4",
                rusqlite::params![state, error, super::now(), answer_id],
            )?;
            Ok(())
        }, r => crate::storage::recon_exec::set_extraction_state(r, answer_id, state).await)
    }
    /// Persist mined answer claims as Brain memories and complete the
    /// answer's extraction job.
    pub async fn persist_claims(
        &self,
        answer: &Message,
        evidence: &[(String, ToolResult)],
        claims: &[serde_json::Value],
    ) -> Result<()> {
        dispatch!(self, s => {
            let mut s = s;
            super::persist_claims(&mut s, answer, evidence, claims)
        }, r => crate::storage::recon_extract::persist_claims(r, answer, evidence, claims).await.map(|_| ()))
    }

    /// Complete an answer's extraction under the shared pre-answer refresh
    /// (`insights-v1`); SurrealDB has no contract column, only the state.
    pub async fn complete_extraction_v1(&self, answer_id: &str) -> Result<()> {
        dispatch!(self, s => {
            s.conn.execute(
                "UPDATE extraction_jobs SET state='completed',contract_version='insights-v1',updated_at=?1 WHERE answer_id=?2",
                rusqlite::params![super::now(), answer_id],
            )?;
            Ok(())
        }, r => crate::storage::recon_exec::set_extraction_state(r, answer_id, "completed").await)
    }
    /// Point the run's active Recon snapshot sources at the stored answer.
    pub async fn relink_sources(&self, run_id: &str, answer_id: &str) -> Result<()> {
        dispatch!(self, s => {
            s.conn.execute(
                "UPDATE insight_sources SET answer_id=?1 WHERE run_id=?2 AND answer_id IN (SELECT active_snapshot_id FROM insight_owner_state WHERE owner_kind='recon' AND owner_id=?2)",
                rusqlite::params![answer_id, run_id],
            )?;
            Ok(())
        }, r => crate::storage::recon_exec::relink_sources(r, run_id, answer_id).await)
    }
    /// The thread's completed calls (`Store::calls_for_thread`).
    pub async fn calls_for_thread(&self, tid: &str) -> Result<Vec<Call>> {
        dispatch!(self, s => s.calls_for_thread(tid),
            r => crate::storage::recon_runs::calls(r, crate::storage::recon_runs::CallFilter::ThreadCompleted(tid)).await)
    }
    /// Every call on the thread, any status (`Store::all_calls_for_thread`).
    pub async fn all_calls_for_thread(&self, tid: &str) -> Result<Vec<Call>> {
        dispatch!(self, s => s.all_calls_for_thread(tid),
            r => crate::storage::recon_runs::calls(r, crate::storage::recon_runs::CallFilter::Thread(tid)).await)
    }
    /// Manual (Tools pane) calls (`Store::manual_calls`).
    pub async fn manual_calls(&self) -> Result<Vec<Call>> {
        dispatch!(self, s => s.manual_calls(),
            r => crate::storage::recon_runs::calls(r, crate::storage::recon_runs::CallFilter::Manual).await)
    }
    /// The state of the thread's newest run (`Store::latest_run_state`).
    pub async fn latest_run_state(&self, tid: &str) -> Result<Option<String>> {
        dispatch!(self, s => s.latest_run_state(tid),
            r => crate::storage::recon_runs::latest_run_state(r, tid).await)
    }
    /// Store the thread's composer draft and chat scroll.
    pub async fn save_draft(&self, tid: &str, draft: &str, scroll: i64) -> Result<()> {
        dispatch!(self, s => s.save_draft(tid, draft, scroll),
            r => crate::storage::recon_threads::save_draft(r, tid, draft, scroll).await)
    }
    /// Remember the thread as the last one opened.
    pub async fn select_thread(&self, tid: &str) -> Result<()> {
        dispatch!(self, s => s.select_thread(tid),
            r => crate::storage::recon_threads::select_thread(r, tid).await)
    }
    /// The last selected thread, while it is live.
    pub async fn last_thread(&self) -> Result<Option<String>> {
        dispatch!(self, s => s.last_thread(),
            r => crate::storage::recon_threads::last_thread(r).await)
    }
    /// Delete the thread. `with_insights` (also deleting the insights it
    /// sourced) is SQLite-only: on SurrealDB the insights are kept.
    pub async fn delete_thread(&self, tid: &str, with_insights: bool) -> Result<bool> {
        match self {
            Self::Sqlite(m) => lock(m).delete_thread(tid, with_insights),
            #[cfg(feature = "surreal")]
            Self::Surreal(r) => {
                if with_insights {
                    return Err(not_ported("deleting a thread with its insights"));
                }
                crate::storage::recon::delete_thread(r, tid).await
            }
        }
    }
    /// Start (or restart at `generation`) the run's synthesis operation
    /// `{run}-synthesis`; an existing draft is kept.
    pub async fn begin_synthesis(&self, run_id: &str, generation: i64, draft: &str) -> Result<()> {
        let op_id = format!("{run_id}-synthesis");
        dispatch!(self, s => {
            s.conn.execute(
                "INSERT INTO recon_model_operations(id, run_id, task_id, role, generation, status, draft, created_at, updated_at)
                 VALUES (?1, ?2, '', 'synthesis', ?3, 'running', ?4, ?5, ?5)
                 ON CONFLICT(id) DO UPDATE SET generation=excluded.generation, status='running', updated_at=excluded.updated_at",
                rusqlite::params![op_id, run_id, generation, draft, super::now()],
            )?;
            Ok(())
        }, r => crate::storage::recon_exec::begin_operation(r, &op_id, Some(run_id), None, "synthesis", generation, draft).await)
    }
    /// A retryable (`queued`/`failed`) extraction job: its run and
    /// contract. SurrealDB answers are mined after the answer
    /// ([`SURREAL_MINED`]); the shared pre-answer refresh is SQLite-only.
    pub async fn retryable_extraction(&self, answer_id: &str) -> Result<Option<(String, String)>> {
        dispatch!(self, s => {
            use rusqlite::OptionalExtension;
            Ok(s.conn.query_row(
                "SELECT run_id, contract_version FROM extraction_jobs WHERE answer_id=?1 AND state IN ('queued','failed')",
                [answer_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            ).optional()?)
        }, r => {
            let state = crate::storage::recon_exec::extraction_state(r, answer_id).await?;
            if !matches!(state.as_deref(), Some("queued" | "failed")) {
                return Ok(None);
            }
            let mut q = r
                .db()
                .query("SELECT VALUE run_id FROM ONLY type::record('extraction_job', $a)")
                .bind(("a", answer_id.to_string()))
                .await?
                .check()?;
            let run_id: Option<String> = q.take(0usize)?;
            Ok(run_id.map(|run_id| (run_id, SURREAL_MINED.to_string())))
        })
    }
    /// Retained evidence for the gathered tool results.
    pub async fn evidence(
        &self,
        results: &[(String, ToolResult)],
    ) -> Result<Vec<crate::investigation::insights::RetainedEvidence>> {
        dispatch!(self, s => crate::investigation::insights::evidence_from_tools(&s.conn, results),
            r => crate::storage::pipeline_inputs::recon_evidence(r, results).await)
    }
}
