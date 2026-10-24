//! The store the insight pipeline (`insights::run_pipeline`,
//! `insight_assessment::assess`) reads and writes between model calls:
//! stage checkpoints, refresh execution progress, the dispatch fence and
//! memory evidence for recall admission. On a SurrealDB run it is
//! `storage::pipeline_exec` and the SQLite file is never opened; otherwise
//! the SQLite tables. One store per run, never both.
//!
//! On SurrealDB, stage and activity events go to the repository's event log
//! (`argos_event`, `storage::pipeline_exec::record_task_event`).

use std::path::Path;

use anyhow::Result;
use rusqlite::{params, OptionalExtension};

use super::insights::PipelineInput;

pub(crate) enum ExecStore {
    Sqlite(crate::store::Store),
    #[cfg(feature = "surreal")]
    Surreal(crate::storage::repo::Repository),
}

/// A stored stage checkpoint: (state, output_json, error).
pub(crate) type Saved = (String, String, String);

impl ExecStore {
    /// SurrealDB when selected, else the SQLite file at `db_path`.
    pub(crate) async fn open(db_path: &Path) -> Result<Self> {
        #[cfg(feature = "surreal")]
        if crate::recon::backend::surreal_selected() {
            return Ok(Self::Surreal(crate::storage::runtime::repository().await?));
        }
        Ok(Self::Sqlite(crate::store::Store::open(db_path)?))
    }

    /// The SQLite path model execution and decisions write telemetry to;
    /// `None` on SurrealDB.
    pub(crate) fn telemetry_path<'a>(&self, db_path: &'a Path) -> Option<&'a Path> {
        match self {
            Self::Sqlite(_) => Some(db_path),
            #[cfg(feature = "surreal")]
            Self::Surreal(_) => None,
        }
    }

    /// The SQLite store, `None` on SurrealDB.
    pub(crate) fn sqlite(&self) -> Option<&crate::store::Store> {
        match self {
            Self::Sqlite(store) => Some(store),
            #[cfg(feature = "surreal")]
            Self::Surreal(_) => None,
        }
    }

    pub(crate) async fn validate_dispatch(&self, input: &PipelineInput) -> Result<()> {
        match self {
            Self::Sqlite(s) => super::insight_persist::validate_dispatch(&s.conn, input),
            #[cfg(feature = "surreal")]
            Self::Surreal(r) => crate::storage::pipeline_exec::validate_dispatch(r, input).await,
        }
    }

    pub(crate) async fn checkpoint(&self, key: &str) -> Result<Option<Saved>> {
        match self {
            Self::Sqlite(s) => Ok(s
                .conn
                .query_row(
                    "SELECT state,output_json,error FROM insight_stage_checkpoints WHERE operation_key=?1",
                    [key],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()?),
            #[cfg(feature = "surreal")]
            Self::Surreal(r) => Ok(crate::storage::pipeline_exec::checkpoint(r, key)
                .await?
                .map(|c| (c.state, c.output_json, c.error))),
        }
    }

    pub(crate) async fn record_reuse(&self, key: &str, input: &PipelineInput) -> Result<()> {
        match self {
            Self::Sqlite(s) => super::insight_execution::record_reuse(&s.conn, key, input),
            #[cfg(feature = "surreal")]
            Self::Surreal(r) => crate::storage::pipeline_exec::record_reuse(r, key, input).await,
        }
    }

    /// Mark `key` running; `clear_error` resets its error.
    pub(crate) async fn begin(
        &self,
        key: &str,
        input: &PipelineInput,
        stage: &str,
        window_id: &str,
        clear_error: bool,
    ) -> Result<()> {
        match self {
            Self::Sqlite(s) => {
                let update = if clear_error {
                    "state='running',error='',updated_at=excluded.updated_at"
                } else {
                    "state='running',updated_at=excluded.updated_at"
                };
                s.conn.execute(&format!("INSERT INTO insight_stage_checkpoints(operation_key,owner_kind,owner_id,generation,stage,window_id,state,updated_at) VALUES(?1,?2,?3,?4,?5,?6,'running',?7) ON CONFLICT(operation_key) DO UPDATE SET {update}"), params![key,input.snapshot.owner_kind,input.snapshot.owner_id,input.snapshot.generation,stage,window_id,chrono::Utc::now().to_rfc3339()])?;
                Ok(())
            }
            #[cfg(feature = "surreal")]
            Self::Surreal(r) => {
                crate::storage::pipeline_exec::begin(r, key, input, stage, window_id, clear_error)
                    .await
            }
        }
    }

    /// Settle `key`; `None` leaves output or route unchanged.
    pub(crate) async fn finish(
        &self,
        key: &str,
        state: &str,
        output: Option<String>,
        route: Option<String>,
        error: &str,
    ) -> Result<()> {
        match self {
            Self::Sqlite(s) => {
                s.conn.execute("UPDATE insight_stage_checkpoints SET state=?2,output_json=COALESCE(?3,output_json),route_json=COALESCE(?4,route_json),error=?5,updated_at=?6 WHERE operation_key=?1",params![key,state,output,route,error,chrono::Utc::now().to_rfc3339()])?;
                Ok(())
            }
            #[cfg(feature = "surreal")]
            Self::Surreal(r) => {
                crate::storage::pipeline_exec::finish(r, key, state, output, route, error).await
            }
        }
    }

    /// Write a whole checkpoint.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn put(
        &self,
        key: &str,
        input: &PipelineInput,
        stage: &str,
        window_id: &str,
        state: &str,
        output: &str,
        route: &str,
        error: &str,
    ) -> Result<()> {
        match self {
            Self::Sqlite(s) => {
                s.conn.execute("INSERT INTO insight_stage_checkpoints(operation_key,owner_kind,owner_id,generation,stage,window_id,state,output_json,route_json,error,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11) ON CONFLICT(operation_key) DO UPDATE SET state=excluded.state,output_json=excluded.output_json,route_json=excluded.route_json,error=excluded.error,updated_at=excluded.updated_at",params![key,input.snapshot.owner_kind,input.snapshot.owner_id,input.snapshot.generation,stage,window_id,state,output,route,error,chrono::Utc::now().to_rfc3339()])?;
                Ok(())
            }
            #[cfg(feature = "surreal")]
            Self::Surreal(r) => {
                crate::storage::pipeline_exec::put(
                    r, key, input, stage, window_id, state, output, route, error,
                )
                .await
            }
        }
    }

    pub(crate) async fn task_id(&self, input: &PipelineInput) -> Result<String> {
        match self {
            Self::Sqlite(s) => super::insight_assessment::task_id(&s.conn, input),
            #[cfg(feature = "surreal")]
            Self::Surreal(r) => crate::storage::pipeline_exec::task_id(r, input).await,
        }
    }

    pub(crate) async fn operation_id(&self, task_id: &str, key: &str) -> Result<String> {
        match self {
            Self::Sqlite(s) => super::insight_execution::operation_id(&s.conn, task_id, key),
            #[cfg(feature = "surreal")]
            Self::Surreal(r) => crate::storage::pipeline_exec::operation_id(r, task_id, key).await,
        }
    }

    pub(crate) async fn progress(
        &self,
        input: &PipelineInput,
        stage: &str,
        windows: Option<usize>,
        withheld: Option<usize>,
    ) -> Result<()> {
        match self {
            Self::Sqlite(s) => {
                super::insight_execution::progress(&s.conn, input, stage, windows, withheld)
            }
            #[cfg(feature = "surreal")]
            Self::Surreal(r) => {
                crate::storage::pipeline_exec::progress(r, input, stage, windows, withheld).await
            }
        }
    }

    /// `insight_execution::stage_event`; on SurrealDB queued to `argos_event`.
    pub(crate) fn stage_event(
        &self,
        input: &PipelineInput,
        task_id: &str,
        stage: &str,
        outcome: &str,
        message: &str,
        details: serde_json::Value,
    ) -> Result<()> {
        match self {
            Self::Sqlite(s) => super::insight_execution::stage_event(
                &s.conn, input, task_id, stage, outcome, message, details,
            ),
            #[cfg(feature = "surreal")]
            Self::Surreal(_) => {
                let event = super::insight_execution::stage_new_event(
                    input,
                    task_id,
                    String::new(),
                    stage,
                    outcome,
                    message,
                    details,
                );
                let task_id = task_id.to_string();
                crate::storage::runtime::spawn_ordered(async move {
                    if let Ok(repo) = crate::storage::runtime::repository().await {
                        let _ = crate::storage::pipeline_exec::record_task_event(
                            &repo, &task_id, false, event,
                        )
                        .await;
                    }
                });
                Ok(())
            }
        }
    }

    /// Recall candidates with their memory evidence, best first, deduped.
    pub(crate) async fn recall_evidence(
        &self,
        input: &PipelineInput,
        query: &str,
        limit: usize,
        take: usize,
    ) -> Result<
        Vec<(
            crate::brain::ScoredMemory,
            crate::brain_query::MemoryEvidence,
        )>,
    > {
        let mut hits = match self {
            Self::Sqlite(s) => crate::brain_query::recall_candidates_with_limit(
                s,
                &input.snapshot.owner_kind,
                &input.snapshot.owner_id,
                query,
                limit,
            )?,
            #[cfg(feature = "surreal")]
            Self::Surreal(r) => {
                crate::storage::memories::recall(r, query, limit.clamp(10, 20), None)
                    .await?
                    .into_iter()
                    .map(|h| crate::brain::ScoredMemory {
                        score: h.score,
                        memory: h.memory,
                    })
                    .collect()
            }
        };
        hits.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then_with(|| a.memory.id.cmp(&b.memory.id))
        });
        let mut seen = std::collections::BTreeSet::new();
        hits.retain(|h| seen.insert(h.memory.id.clone()));
        hits.truncate(take);
        let mut out = Vec::new();
        for hit in hits {
            let evidence = match self {
                Self::Sqlite(s) => {
                    crate::brain_query::load_memory_evidence(&s.conn, &hit.memory.id)
                }
                #[cfg(feature = "surreal")]
                Self::Surreal(r) => {
                    crate::storage::pipeline_exec::load_memory_evidence(r, &hit.memory.id).await
                }
            };
            if let Ok(Some(e)) = evidence {
                out.push((hit, e));
            }
        }
        Ok(out)
    }
}

/// Record a model-execution activity label (and its log message on
/// SQLite) for `task_id`, best effort; never blocks the caller.
pub(crate) fn record_activity(db_path: &Path, task_id: &str, label: &str, message: Option<String>) {
    #[cfg(feature = "surreal")]
    if crate::recon::backend::surreal_selected() {
        let (task_id, label) = (task_id.to_string(), label.to_string());
        crate::storage::runtime::spawn_ordered(async move {
            if let Ok(repo) = crate::storage::runtime::repository().await {
                let _ =
                    crate::storage::pipeline_exec::record_activity(&repo, &task_id, &label).await;
                if let Some(message) = message {
                    let event = super::insight_execution::activity_new_event(
                        &task_id,
                        String::new(),
                        String::new(),
                        &message,
                    );
                    let _ = crate::storage::pipeline_exec::record_task_event(
                        &repo, &task_id, true, event,
                    )
                    .await;
                }
            }
        });
        return;
    }
    if let Ok(store) = crate::store::Store::open(db_path) {
        let _ = super::insight_execution::record_activity(&store.conn, task_id, label);
        if let Some(message) = message {
            let _ = super::insight_execution::activity_event(&store.conn, task_id, &message);
        }
    }
}
