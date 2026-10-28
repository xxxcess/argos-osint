//! The TUI Intel list and brief chain on SurrealDB. Under a storage config
//! (`surreal_brain::on`) the bulletin list, the brief chain admission and
//! its stage views read the repository and never the SQLite store. Reads
//! block on `storage::runtime::blocking`, so callers are worker threads;
//! [`ensure_brief`] hops to its own thread so the UI thread may call it.

use std::collections::HashMap;

use anyhow::Result;
use argos_osint_core::intel_recon::brief_pipeline::StageView;
use argos_osint_core::provider::SettingsFile;
use argos_osint_core::store::{AtlasArticleRow, AtlasRunRow};

/// Stages, stage outputs and lead titles of one brief job.
pub type BriefChain = (
    Vec<StageView>,
    HashMap<String, serde_json::Value>,
    HashMap<String, String>,
);

/// Days, the chosen day and its articles for `(category, day, search)`.
pub(crate) type IntelList = (Vec<String>, String, Vec<AtlasArticleRow>);

#[cfg(feature = "surreal")]
mod imp {
    use super::*;
    use argos_osint_core::storage::{atlas, brief_stages, runtime};

    pub(crate) fn intel_list(category: &str, day: &str, search: &str) -> Result<IntelList> {
        runtime::blocking(|repo| async move {
            let db = repo.db();
            let days = atlas::run_days(db).await?;
            let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
            let day = if days.iter().any(|d| d == day) {
                day.to_string()
            } else if days.contains(&today) {
                today
            } else {
                days.first().cloned().unwrap_or_default()
            };
            let articles = if day.is_empty() {
                Vec::new()
            } else {
                atlas::articles_for_intel(db, category, &day, search).await?
            };
            Ok((days, day, articles))
        })
    }

    pub(crate) fn ensure_brief(
        article: &AtlasArticleRow,
        settings: &SettingsFile,
        on_done: impl FnOnce(Result<String>) + Send + 'static,
    ) {
        let (article, settings) = (article.clone(), settings.clone());
        std::thread::spawn(move || {
            let result = runtime::blocking(|repo| async move {
                brief_stages::ensure_for_article(&repo, &article, &settings).await
            });
            on_done(result);
        });
    }

    /// Stage views, committed outputs and lead titles of a chain.
    pub(crate) fn brief_chain(job: &str) -> Result<BriefChain> {
        runtime::blocking(|repo| async move {
            let stages = brief_stages::stage_views(&repo, job).await?;
            let outputs = brief_stages::stage_outputs(&repo, job).await?;
            let mut titles = HashMap::new();
            if let Some(leads) = outputs
                .get("intel_brief_links")
                .and_then(|v| v["leads"].as_array())
            {
                for lead in leads {
                    if let (Some(id), Some(run)) =
                        (lead["article_id"].as_str(), lead["article_run_id"].as_str())
                    {
                        if let Some(a) = atlas::article(repo.db(), run, id).await? {
                            titles.insert(id.to_string(), a.title);
                        }
                    }
                }
            }
            Ok((stages, outputs, titles))
        })
    }

    pub(crate) fn atlas_runs() -> Result<Vec<AtlasRunRow>> {
        runtime::blocking(|repo| async move { Ok(atlas::list_runs(repo.db()).await?) })
    }

    pub(crate) fn atlas_articles(run_id: &str) -> Result<Vec<AtlasArticleRow>> {
        runtime::blocking(|repo| async move { Ok(atlas::list_articles(repo.db(), run_id).await?) })
    }

    /// `Store::atlas_delete_run` on the repository: the run's memories,
    /// articles and run row in one transaction. True when the run existed.
    pub(crate) fn atlas_delete_run(run_id: &str) -> Result<bool> {
        use argos_osint_core::storage::atlas_delete;
        runtime::blocking(|repo| async move {
            atlas_delete::delete_run(repo.publisher(), run_id)
                .await
                .map_err(|e| anyhow::anyhow!("delete atlas run: {e}"))
        })
    }
}

#[cfg(not(feature = "surreal"))]
mod imp {
    use super::*;

    fn off<T>() -> Result<T> {
        unreachable!("SurrealDB Intel is off without the surreal feature")
    }
    pub(crate) fn intel_list(_: &str, _: &str, _: &str) -> Result<IntelList> {
        off()
    }
    pub(crate) fn ensure_brief(
        _: &AtlasArticleRow,
        _: &SettingsFile,
        on_done: impl FnOnce(Result<String>) + Send + 'static,
    ) {
        on_done(off());
    }
    #[allow(clippy::type_complexity)]
    pub(crate) fn brief_chain(_: &str) -> Result<BriefChain> {
        off()
    }
    pub(crate) fn atlas_runs() -> Result<Vec<AtlasRunRow>> {
        off()
    }
    pub(crate) fn atlas_articles(_: &str) -> Result<Vec<AtlasArticleRow>> {
        off()
    }

    pub(crate) fn atlas_delete_run(_: &str) -> Result<bool> {
        unreachable!("SurrealDB Atlas is off without the surreal feature")
    }
}

pub(crate) use imp::*;
