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

use super::insights::PipelineInput;

pub(crate) enum ExecStore {
    #[cfg(feature = "surreal")]
    Surreal(crate::storage::repo::Repository),
}

pub(crate) type Saved = (String, String, String);

impl ExecStore {
    /// SurrealDB when selected, else the SQLite file at `db_path`.
    pub(crate) async fn open(_db_path: &Path) -> Result<Self> {
        Ok(Self::Surreal(crate::storage::runtime::repository().await?))
    }

    pub(crate) async fn checkpoint(&self, key: &str) -> Result<Option<Saved>> {
        match self {
            #[cfg(feature = "surreal")]
            Self::Surreal(r) => Ok(crate::storage::pipeline_exec::checkpoint(r, key)
                .await?
                .map(|c| (c.state, c.output_json, c.error))),
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
            #[cfg(feature = "surreal")]
            Self::Surreal(r) => {
                crate::storage::pipeline_exec::finish(r, key, state, output, route, error).await
            }
        }
    }

    /// The Report surface's `intel_report_results.details_json` for a
    /// source hash: `intel_report_result`. A missing row is an error.
    pub(crate) async fn report_details(&self, source_hash: &str) -> Result<String> {
        match self {
            #[cfg(feature = "surreal")]
            Self::Surreal(r) => crate::storage::intel::report_details_by_hash(r.db(), source_hash)
                .await?
                .ok_or_else(|| anyhow::anyhow!("no Intel report result for {source_hash}")),
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
            #[cfg(feature = "surreal")]
            Self::Surreal(r) => {
                crate::storage::pipeline_exec::begin(r, key, input, stage, window_id, clear_error)
                    .await
            }
        }
    }

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
            #[cfg(feature = "surreal")]
            Self::Surreal(r) => {
                crate::storage::pipeline_exec::put(
                    r, key, input, stage, window_id, state, output, route, error,
                )
                .await
            }
        }
    }

    pub(crate) async fn validate_dispatch(&self, input: &PipelineInput) -> Result<()> {
        match self {
            #[cfg(feature = "surreal")]
            Self::Surreal(r) => crate::storage::pipeline_exec::validate_dispatch(r, input).await,
        }
    }

    pub(crate) async fn record_reuse(&self, key: &str, input: &PipelineInput) -> Result<()> {
        match self {
            #[cfg(feature = "surreal")]
            Self::Surreal(r) => crate::storage::pipeline_exec::record_reuse(r, key, input).await,
        }
    }

    pub(crate) async fn task_id(&self, input: &PipelineInput) -> Result<String> {
        match self {
            #[cfg(feature = "surreal")]
            Self::Surreal(r) => crate::storage::pipeline_exec::task_id(r, input).await,
        }
    }

    pub(crate) async fn operation_id(&self, task_id: &str, key: &str) -> Result<String> {
        match self {
            #[cfg(feature = "surreal")]
            Self::Surreal(r) => crate::storage::pipeline_exec::operation_id(r, task_id, key).await,
        }
    }

    pub(crate) fn progress<'a>(
        &self,
        input: &'a PipelineInput,
        stage: &'a str,
        windows: Option<usize>,
        withheld: Option<usize>,
    ) -> impl std::future::Future<Output = Result<()>> + Send + 'a {
        let sqlite = match self {
            #[cfg(feature = "surreal")]
            Self::Surreal(_) => None,
        };
        #[cfg(feature = "surreal")]
        let repo = match self {
            Self::Surreal(r) => Some(r.clone()),
        };
        async move {
            if let Some(done) = sqlite {
                return done;
            }
            #[cfg(feature = "surreal")]
            if let Some(r) = repo {
                return crate::storage::pipeline_exec::progress(
                    &r, input, stage, windows, withheld,
                )
                .await;
            }
            Ok(())
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
        _input: &PipelineInput,
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
            #[cfg(feature = "surreal")]
            Self::Surreal(r) => {
                crate::storage::memories::recall(r, query, limit.clamp(10, 20), None)
                    .await?
                    .into_iter()
                    .map(|h| crate::brain::ScoredMemory {
                        score: h.score,
                        memory: h.memory,
                    })
                    .collect::<Vec<_>>()
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
pub(crate) fn record_activity(
    _db_path: &Path,
    task_id: &str,
    label: &str,
    message: Option<String>,
) {
    let (task_id, label) = (task_id.to_string(), label.to_string());
    crate::storage::runtime::spawn_ordered(async move {
        if let Ok(repo) = crate::storage::runtime::repository().await {
            let _ = crate::storage::pipeline_exec::record_activity(&repo, &task_id, &label).await;
            if let Some(message) = message {
                let event = super::insight_execution::activity_new_event(
                    &task_id,
                    String::new(),
                    String::new(),
                    &message,
                );
                let _ =
                    crate::storage::pipeline_exec::record_task_event(&repo, &task_id, true, event)
                        .await;
            }
        }
    });
}
