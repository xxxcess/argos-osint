//! Store reads and writes for Intel article bodies, plus the one report-job
//! read the TUI still uses.
//!
//! Each method runs on SurrealDB (`storage::article_body`,
//! `storage::recon_jobs`) when the process selected it, and on the legacy
//! SQLite connection otherwise. The retired report-job worker paths
//! (`report_execution_enabled()` was hard-coded false) are gone.

use anyhow::Result;

#[cfg(feature = "surreal")]
use crate::storage::article_body as sb;
use crate::store::Store;

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn new_row_id(prefix: &str) -> String {
    format!(
        "{prefix}-{}-{}",
        chrono::Utc::now().timestamp_millis(),
        uuid_lite()
    )
}

fn uuid_lite() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(1);
    N.fetch_add(1, Ordering::Relaxed)
}

#[derive(Clone, Debug, PartialEq)]
pub struct ArticleBodyRow {
    pub id: String,
    pub article_id: String,
    pub run_id: String,
    pub original_url: String,
    pub resolved_url: String,
    pub source_domain: String,
    pub source_name: String,
    pub body_markdown: String,
    pub content_hash: String,
    pub quality: String,
    pub quality_rationale: String,
    pub body_version: i64,
    pub fetch_tool: String,
    pub fetched_at: String,
    pub retry_after: String,
    pub state: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RetrievalAttemptRow {
    pub id: String,
    pub body_id: String,
    pub tool_id: String,
    pub input_identity: String,
    pub state: String,
    pub started_at: String,
    pub finished_at: String,
    pub reason: String,
    pub remote_job_id: String,
    pub body_version: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct IntelInvestigationRow {
    pub id: String,
    pub article_id: String,
    pub run_id: String,
    pub article_url: String,
    pub thread_id: Option<String>,
    pub scope_json: String,
    pub shared_assessment_json: String,
    pub shared_assessment_version: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct IntelReportJobRow {
    pub id: String,
    pub investigation_id: String,
    pub article_id: String,
    pub mode: String,
    pub revision: i64,
    pub state: String,
    pub stage: String,
    pub settings_json: String,
    pub budget_json: String,
    pub parent_job_id: Option<String>,
    pub sections_done: i64,
    pub sections_total: i64,
    pub elements_done: i64,
    pub elements_total: i64,
    pub tool_calls_done: i64,
    pub tool_calls_allowance: i64,
    pub current_tool: String,
    pub warning: String,
    pub error: String,
    pub generation: i64,
    pub started_at: String,
    pub updated_at: String,
    pub finished_at: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct IntelReportTaskRow {
    pub id: String,
    pub job_id: String,
    pub task_type: String,
    pub section_key: String,
    pub depends_on_json: String,
    pub input_hash: String,
    pub status: String,
    pub attempts: i64,
    pub max_attempts: i64,
    pub output_ref: String,
    pub lease_owner: String,
    pub lease_until: String,
    pub error: String,
    pub created_at: String,
    pub updated_at: String,
    pub lease_epoch: i64,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct IntelReportSectionRow {
    pub id: String,
    pub job_id: String,
    pub section_key: String,
    pub title: String,
    pub ordinal: i64,
    pub markdown: String,
    pub status: String,
    pub revision: i64,
    pub evidence_ids_json: String,
    pub judgment_json: String,
    pub assessment_version: i64,
    pub waiting_on: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct IntelElementRow {
    pub id: String,
    pub investigation_id: String,
    pub element_key: String,
    pub fingerprint: String,
    pub element_type: String,
    pub original_text: String,
    pub provenance: String,
    pub status: String,
    pub stance: String,
    pub assessment: String,
    pub uncertainty: String,
    pub evidence_ids_json: String,
    pub directive_ids_json: String,
    pub section_keys_json: String,
    pub excluded_reason: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct IntelEvidenceRow {
    pub id: String,
    pub investigation_id: String,
    pub source_url: String,
    pub source_domain: String,
    pub excerpt: String,
    pub location: String,
    pub claim_ids_json: String,
    pub source_date: String,
    pub event_date: String,
    pub retrieved_at: String,
    pub stance: String,
    pub origin: String,
    pub independence: String,
    pub limitations: String,
    pub tool_id: String,
    pub call_id: String,
    pub created_at: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct IntelAssessmentRow {
    pub id: String,
    pub investigation_id: String,
    pub element_id: String,
    pub origin: String,
    pub stance: String,
    pub rationale: String,
    pub confidence: f64,
    pub sources_json: String,
    pub original_value_json: String,
    pub revised_value_json: String,
    pub created_at: String,
    pub updated_at: String,
}

impl Store {
    pub fn article_body_for_article(&self, article_id: &str) -> Result<Option<ArticleBodyRow>> {
        let a = article_id.to_string();
        crate::storage::runtime::blocking(|repo| async move {
            crate::storage::pipeline_inputs::article_body_for_article(&repo, &a).await
        })
    }

    pub fn article_body_by_id(&self, id: &str) -> Result<Option<ArticleBodyRow>> {
        let id = id.to_string();
        crate::storage::runtime::blocking(|repo| async move { sb::by_id(&repo, &id).await })
    }

    pub fn ensure_article_body(
        &self,
        article_id: &str,
        run_id: &str,
        url: &str,
        source_domain: &str,
        source_name: &str,
    ) -> Result<ArticleBodyRow> {
        let (a, r, u, d, n) = (
            article_id.to_string(),
            run_id.to_string(),
            url.to_string(),
            source_domain.to_string(),
            source_name.to_string(),
        );
        crate::storage::runtime::blocking(|repo| async move {
            sb::ensure(&repo, &a, &r, &u, &d, &n).await
        })
    }

    pub fn update_article_body_state(&self, id: &str, state: &str) -> Result<()> {
        let (id, st) = (id.to_string(), state.to_string());
        crate::storage::runtime::blocking(
            |repo| async move { sb::set_state(&repo, &id, &st).await },
        )
    }

    /// Clear stored article prose before a Reload / forced re-fetch so the UI does not
    /// keep showing the previous body while retrieval and synthesis run.
    pub fn clear_article_body_content(&self, id: &str) -> Result<()> {
        let id = id.to_string();
        crate::storage::runtime::blocking(|repo| async move { sb::clear_content(&repo, &id).await })
    }

    pub fn commit_article_body(
        &self,
        id: &str,
        markdown: &str,
        content_hash: &str,
        quality: &str,
        rationale: &str,
        resolved_url: &str,
        fetch_tool: &str,
        bump_version: bool,
    ) -> Result<()> {
        let (id, md, h, q, ra, u, t) = (
            id.to_string(),
            markdown.to_string(),
            content_hash.to_string(),
            quality.to_string(),
            rationale.to_string(),
            resolved_url.to_string(),
            fetch_tool.to_string(),
        );
        crate::storage::runtime::blocking(|repo| async move {
            sb::commit(&repo, &id, &md, &h, &q, &ra, &u, &t, bump_version).await
        })
    }

    pub fn mark_article_body_failed(
        &self,
        id: &str,
        rationale: &str,
        retry_after: &str,
        partial_markdown: &str,
        quality: &str,
    ) -> Result<()> {
        let (id, ra, ry, md, q) = (
            id.to_string(),
            rationale.to_string(),
            retry_after.to_string(),
            partial_markdown.to_string(),
            quality.to_string(),
        );
        crate::storage::runtime::blocking(|repo| async move {
            sb::mark_failed(&repo, &id, &ra, &ry, &md, &q).await
        })
    }

    pub fn insert_retrieval_attempt(
        &self,
        body_id: &str,
        tool_id: &str,
        input_identity: &str,
        state: &str,
        reason: &str,
        remote_job_id: &str,
        body_version: i64,
    ) -> Result<String> {
        let _ = remote_job_id;
        let (b, t, i, st, r) = (
            body_id.to_string(),
            tool_id.to_string(),
            input_identity.to_string(),
            state.to_string(),
            reason.to_string(),
        );
        crate::storage::runtime::blocking(|repo| async move {
            sb::insert_attempt(&repo, &b, &t, &i, &st, &r, body_version).await
        })
    }

    pub fn finish_retrieval_attempt(
        &self,
        id: &str,
        state: &str,
        reason: &str,
        remote_job_id: &str,
    ) -> Result<()> {
        let _ = remote_job_id;
        let (id, st, r) = (id.to_string(), state.to_string(), reason.to_string());
        crate::storage::runtime::blocking(|repo| async move {
            sb::finish_attempt(&repo, &id, &st, &r).await
        })
    }

    pub fn intel_report_job(&self, id: &str) -> Result<Option<IntelReportJobRow>> {
        let id = id.to_string();
        crate::storage::runtime::blocking(|repo| async move {
            crate::storage::recon_jobs::report_job(&repo, &id).await
        })
    }
}
