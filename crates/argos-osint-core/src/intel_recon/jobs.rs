//! Create and control Intel report jobs.

use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

use crate::osint::ProviderKeys;
use crate::provider::SettingsFile;
use crate::secrets::ProviderSecret;
use crate::store::{AtlasArticleRow, Store};

use super::ledger::{body_assertion_candidates, seed_element_ledger};
use super::modes::{section_plan, ReportMode};
use super::persist::IntelReportJobRow;
use super::worker::{self, JobRuntime};

#[derive(Clone, Debug)]
pub enum IntelReportEvent {
    JobCreated {
        article_id: String,
        job_id: String,
        mode: String,
        revision: i64,
        generation: i64,
    },
    Stage {
        article_id: String,
        job_id: String,
        stage: String,
        generation: i64,
    },
    Section {
        article_id: String,
        job_id: String,
        section_key: String,
        status: String,
        generation: i64,
    },
    InsightsUpdated {
        article_id: String,
        job_id: String,
        generation: i64,
    },
    JobDone {
        article_id: String,
        job_id: String,
        state: String,
        generation: i64,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReportScope {
    pub timeframe: String,
    pub depth: String,
    pub outlook_horizon_days: u32,
    /// Enabled section keys for this report. Empty means every section in the mode plan.
    #[serde(default)]
    pub sections: Vec<String>,
}

impl Default for ReportScope {
    fn default() -> Self {
        Self {
            timeframe: "current".into(),
            depth: "standard".into(),
            outlook_horizon_days: 30,
            sections: Vec::new(),
        }
    }
}

/// Mode section plan filtered by `scope.sections` (empty scope keeps the full plan).
pub fn scoped_section_plan(mode: ReportMode, scope: &ReportScope) -> Vec<super::modes::SectionPlan> {
    let plan = section_plan(mode);
    if scope.sections.is_empty() {
        return plan;
    }
    let enabled: std::collections::HashSet<&str> =
        scope.sections.iter().map(|s| s.as_str()).collect();
    plan.into_iter()
        .filter(|section| enabled.contains(section.key))
        .collect()
}

pub fn active_job_for_mode(
    store: &Store,
    article_id: &str,
    mode: ReportMode,
) -> Result<Option<IntelReportJobRow>> {
    store.active_intel_job(article_id, mode.as_str())
}

/// Create or focus a report job with all section placeholders and the task graph.
pub fn create_report_job(
    store: &Store,
    article: &AtlasArticleRow,
    mode: ReportMode,
    scope: &ReportScope,
    force_new_revision: bool,
) -> Result<IntelReportJobRow> {
    if !force_new_revision {
        if let Some(active) = active_job_for_mode(store, &article.id, mode)? {
            return Ok(active);
        }
    }

    let scope_json = serde_json::to_string(scope)?;
    let inv = store.ensure_intel_investigation(
        &article.id,
        &article.run_id,
        &article.url,
        &scope_json,
    )?;

    // Seed / refresh element ledger.
    let claims = store
        .atlas_claims_for_article(&article.run_id, &article.id)
        .unwrap_or_default();
    let body = store.article_body_for_article(&article.id)?;
    let body_md = body.as_ref().map(|b| b.body_markdown.as_str()).unwrap_or("");
    let candidates = if claims.is_empty() {
        body_assertion_candidates(body_md, 12)
    } else {
        // Still pick up consequential body sentences not already claimed.
        body_assertion_candidates(body_md, 6)
    };
    let elements = seed_element_ledger(store, &inv.id, &claims, &candidates)?;

    let plan = scoped_section_plan(mode, scope);
    anyhow::ensure!(
        !plan.is_empty(),
        "enable at least one report section before starting Recon"
    );
    let revision = store.next_job_revision(&article.id, mode.as_str())?;
    let budget = serde_json::json!({
        "max_collection_concurrent": 2,
        "max_synthesis_concurrent": 1,
        "tool_calls_allowance": 24,
        "outlook_horizon_days": scope.outlook_horizon_days,
    });
    let job = store.insert_report_job(
        &inv.id,
        &article.id,
        mode.as_str(),
        revision,
        &scope_json,
        &budget.to_string(),
        plan.len() as i64,
        elements.len() as i64,
        24,
        None,
    )?;

    // Sections appear immediately with waiting status.
    for (ordinal, section) in plan.iter().enumerate() {
        let waiting = if section.key == "bluf" {
            "findings"
        } else if ordinal + 1 == plan.len() {
            "body sections"
        } else {
            "evidence"
        };
        store.insert_report_section(
            &job.id,
            section.key,
            section.title,
            ordinal as i64,
            "waiting",
            waiting,
        )?;
    }

    // Task DAG.
    let acquire = store.insert_report_task(&job.id, "acquire_body", "", "[]", "pending")?;
    let inventory =
        store.insert_report_task(&job.id, "inventory", "", &serde_json::json!([acquire]).to_string(), "pending")?;
    let collect = store.insert_report_task(
        &job.id,
        "collect",
        "",
        &serde_json::json!([inventory]).to_string(),
        "pending",
    )?;
    let assess = store.insert_report_task(
        &job.id,
        "assess",
        "",
        &serde_json::json!([collect]).to_string(),
        "pending",
    )?;

    let include_bluf = plan.iter().any(|section| section.key == "bluf");
    let mut section_task_ids = Vec::new();
    for section in &plan {
        if section.key == "bluf" {
            continue; // BLUF after body sections when present
        }
        let tid = store.insert_report_task(
            &job.id,
            "synthesize",
            section.key,
            &serde_json::json!([assess]).to_string(),
            "pending",
        )?;
        section_task_ids.push(tid);
    }
    let mut finalize_deps = section_task_ids.clone();
    if include_bluf {
        let bluf_deps = if section_task_ids.is_empty() {
            serde_json::json!([assess])
        } else {
            serde_json::json!(section_task_ids.clone())
        };
        let bluf_tid = store.insert_report_task(
            &job.id,
            "synthesize",
            "bluf",
            &bluf_deps.to_string(),
            "pending",
        )?;
        finalize_deps = vec![bluf_tid];
    } else if finalize_deps.is_empty() {
        finalize_deps = vec![assess];
    }
    store.insert_report_task(
        &job.id,
        "finalize",
        "",
        &serde_json::json!(finalize_deps).to_string(),
        "pending",
    )?;

    // Seed article body as evidence when available.
    if let Some(body) = body {
        if !body.body_markdown.trim().is_empty() {
            let excerpt: String = body.body_markdown.chars().take(2000).collect();
            let _ = store.insert_evidence(
                &inv.id,
                &article.url,
                &article.source_domain,
                &excerpt,
                "article_body",
                "[]",
                &article.published_at,
                "",
                "support",
                "article",
                &body.fetch_tool,
                &article.id,
                if body.quality == "complete" {
                    ""
                } else {
                    "partial or uncertain article body"
                },
            );
        }
    }

    store.update_report_job(
        &job.id,
        "queued",
        "planned",
        0,
        0,
        0,
        "",
        "",
        "",
    )?;
    Ok(store.intel_report_job(&job.id)?.expect("job exists"))
}

pub fn pause_job(store: &Store, job_id: &str) -> Result<()> {
    let job = store
        .intel_report_job(job_id)?
        .ok_or_else(|| anyhow!("job not found"))?;
    anyhow::ensure!(
        matches!(job.state.as_str(), "queued" | "running" | "waiting"),
        "job cannot be paused from {}",
        job.state
    );
    store.update_report_job(
        job_id,
        "paused",
        &job.stage,
        job.sections_done,
        job.elements_done,
        job.tool_calls_done,
        &job.current_tool,
        &job.warning,
        &job.error,
    )
}

pub fn resume_job(store: &Store, job_id: &str) -> Result<()> {
    let job = store
        .intel_report_job(job_id)?
        .ok_or_else(|| anyhow!("job not found"))?;
    anyhow::ensure!(job.state == "paused", "job is not paused");
    store.update_report_job(
        job_id,
        "queued",
        &job.stage,
        job.sections_done,
        job.elements_done,
        job.tool_calls_done,
        &job.current_tool,
        &job.warning,
        &job.error,
    )
}

pub fn cancel_job(store: &Store, job_id: &str) -> Result<()> {
    let job = store
        .intel_report_job(job_id)?
        .ok_or_else(|| anyhow!("job not found"))?;
    store.update_report_job(
        job_id,
        "cancelled",
        "cancelled",
        job.sections_done,
        job.elements_done,
        job.tool_calls_done,
        "",
        &job.warning,
        "cancelled by user",
    )?;
    // Pending tasks will not be claimed once job is cancelled.
    Ok(())
}

pub fn retry_failed_tasks(store: &Store, job_id: &str) -> Result<usize> {
    let tasks = store.intel_report_tasks(job_id)?;
    let mut n = 0;
    for task in tasks {
        if task.status == "failed" {
            store.conn.execute(
                "UPDATE intel_report_tasks SET status='pending', error='', updated_at=?2 WHERE id=?1",
                rusqlite::params![task.id, chrono::Utc::now().to_rfc3339()],
            )?;
            n += 1;
        }
    }
    if n > 0 {
        let job = store.intel_report_job(job_id)?.expect("job");
        store.update_report_job(
            job_id,
            "queued",
            "retrying",
            job.sections_done,
            job.elements_done,
            job.tool_calls_done,
            "",
            &job.warning,
            "",
        )?;
    }
    Ok(n)
}

/// Spawn the background worker that drains the job's task graph.
pub fn start_report_worker(
    db_path: &Path,
    job_id: String,
    article_title: String,
    article_url: String,
    article_preview: String,
    article_published: String,
    article_domain: String,
    run_id: String,
    keys: ProviderKeys,
    synthesis_secret: Option<ProviderSecret>,
    classifier_secret: Option<ProviderSecret>,
    settings: SettingsFile,
    cancel: Arc<AtomicBool>,
    on_event: impl FnMut(IntelReportEvent) + Send + 'static,
) {
    let db_path = db_path.to_path_buf();
    let runtime = JobRuntime {
        db_path,
        job_id,
        article_title,
        article_url,
        article_preview,
        article_published,
        article_domain,
        run_id,
        keys,
        synthesis_secret,
        classifier_secret,
        settings,
        cancel,
    };
    tokio::spawn(async move {
        let _ = worker::run_job_to_completion(runtime, on_event).await;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_job_inserts_all_verify_sections() {
        let store = Store::memory().unwrap();
        let article = AtlasArticleRow {
            run_id: "r1".into(),
            id: "a1".into(),
            title: "Test".into(),
            description: "Desc".into(),
            url: "https://example.com/a".into(),
            country: "us".into(),
            source_name: "Ex".into(),
            source_domain: "example.com".into(),
            published_at: "2026-10-01T00:00:00Z".into(),
            provider: "newsapi".into(),
            temperature: 0.5,
            category: "geopolitical".into(),
            seen_at: "2026-10-01T00:00:00Z".into(),
            author: String::new(),
            image_url: String::new(),
        };
        store.atlas_upsert_article(&article).unwrap();
        let job = create_report_job(&store, &article, ReportMode::Verify, &ReportScope::default(), false)
            .unwrap();
        assert_eq!(job.mode, "verify");
        assert_eq!(job.sections_total, 6);
        let sections = store.intel_report_sections(&job.id).unwrap();
        assert_eq!(sections.len(), 6);
        assert_eq!(sections[0].section_key, "bluf");
        let again =
            create_report_job(&store, &article, ReportMode::Verify, &ReportScope::default(), false)
                .unwrap();
        assert_eq!(again.id, job.id, "active job is reused");
    }
}
