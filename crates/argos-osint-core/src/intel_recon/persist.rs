//! Store reads and writes for Intel article bodies, plus the one report-job
//! read the TUI still uses.
//!
//! Each method runs on SurrealDB (`storage::article_body`,
//! `storage::recon_jobs`) when the process selected it, and on the legacy
//! SQLite connection otherwise. The retired report-job worker paths
//! (`report_execution_enabled()` was hard-coded false) are gone.

use anyhow::Result;
use rusqlite::{params, OptionalExtension};

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
        #[cfg(feature = "surreal")]
        if crate::storage::brain_store::on() {
            let a = article_id.to_string();
            return crate::storage::runtime::blocking(|repo| async move {
                crate::storage::pipeline_inputs::article_body_for_article(&repo, &a).await
            });
        }
        let mut stmt = self.conn.prepare(
            "SELECT id, article_id, run_id, original_url, resolved_url, source_domain, source_name,
                    body_markdown, content_hash, quality, quality_rationale, body_version,
                    fetch_tool, fetched_at, retry_after, state, created_at, updated_at
             FROM article_bodies WHERE article_id=?1
             ORDER BY body_version DESC, updated_at DESC LIMIT 1",
        )?;
        Ok(stmt
            .query_row([article_id], article_body_from_row)
            .optional()?)
    }

    pub fn article_body_by_id(&self, id: &str) -> Result<Option<ArticleBodyRow>> {
        #[cfg(feature = "surreal")]
        if crate::storage::brain_store::on() {
            let id = id.to_string();
            return crate::storage::runtime::blocking(
                |repo| async move { sb::by_id(&repo, &id).await },
            );
        }
        let mut stmt = self.conn.prepare(
            "SELECT id, article_id, run_id, original_url, resolved_url, source_domain, source_name,
                    body_markdown, content_hash, quality, quality_rationale, body_version,
                    fetch_tool, fetched_at, retry_after, state, created_at, updated_at
             FROM article_bodies WHERE id=?1",
        )?;
        Ok(stmt.query_row([id], article_body_from_row).optional()?)
    }

    pub fn ensure_article_body(
        &self,
        article_id: &str,
        run_id: &str,
        url: &str,
        source_domain: &str,
        source_name: &str,
    ) -> Result<ArticleBodyRow> {
        #[cfg(feature = "surreal")]
        if crate::storage::brain_store::on() {
            let (a, r, u, d, n) = (
                article_id.to_string(),
                run_id.to_string(),
                url.to_string(),
                source_domain.to_string(),
                source_name.to_string(),
            );
            return crate::storage::runtime::blocking(|repo| async move {
                sb::ensure(&repo, &a, &r, &u, &d, &n).await
            });
        }
        if let Some(existing) = self.article_body_for_article(article_id)? {
            return Ok(existing);
        }
        let ts = now();
        let id = new_row_id("abody");
        self.conn.execute(
            "INSERT INTO article_bodies(
                id, article_id, run_id, original_url, resolved_url, source_domain, source_name,
                body_markdown, content_hash, quality, quality_rationale, body_version,
                fetch_tool, fetched_at, retry_after, state, created_at, updated_at
             ) VALUES (?1,?2,?3,?4,'',?5,?6,'','','unavailable','',1,'','','','idle',?7,?7)",
            params![id, article_id, run_id, url, source_domain, source_name, ts],
        )?;
        Ok(self.article_body_by_id(&id)?.expect("just inserted"))
    }

    pub fn update_article_body_state(&self, id: &str, state: &str) -> Result<()> {
        #[cfg(feature = "surreal")]
        if crate::storage::brain_store::on() {
            let (id, st) = (id.to_string(), state.to_string());
            return crate::storage::runtime::blocking(|repo| async move {
                sb::set_state(&repo, &id, &st).await
            });
        }
        self.conn.execute(
            "UPDATE article_bodies SET state=?2, updated_at=?3 WHERE id=?1",
            params![id, state, now()],
        )?;
        Ok(())
    }

    /// Clear stored article prose before a Reload / forced re-fetch so the UI does not
    /// keep showing the previous body while retrieval and synthesis run.
    pub fn clear_article_body_content(&self, id: &str) -> Result<()> {
        #[cfg(feature = "surreal")]
        if crate::storage::brain_store::on() {
            let id = id.to_string();
            return crate::storage::runtime::blocking(|repo| async move {
                sb::clear_content(&repo, &id).await
            });
        }
        self.conn.execute(
            "UPDATE article_bodies SET
                body_markdown='', content_hash='', quality='unavailable',
                quality_rationale='', state='running', retry_after='', updated_at=?2
             WHERE id=?1",
            params![id, now()],
        )?;
        Ok(())
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
        #[cfg(feature = "surreal")]
        if crate::storage::brain_store::on() {
            let (id, md, h, q, ra, u, t) = (
                id.to_string(),
                markdown.to_string(),
                content_hash.to_string(),
                quality.to_string(),
                rationale.to_string(),
                resolved_url.to_string(),
                fetch_tool.to_string(),
            );
            return crate::storage::runtime::blocking(|repo| async move {
                sb::commit(&repo, &id, &md, &h, &q, &ra, &u, &t, bump_version).await
            });
        }
        let ts = now();
        if bump_version {
            self.conn.execute(
                "UPDATE article_bodies SET
                    body_markdown=?2, content_hash=?3, quality=?4, quality_rationale=?5,
                    resolved_url=?6, fetch_tool=?7, fetched_at=?8, retry_after='',
                    state='ready', body_version=body_version+1, updated_at=?8
                 WHERE id=?1",
                params![
                    id,
                    markdown,
                    content_hash,
                    quality,
                    rationale,
                    resolved_url,
                    fetch_tool,
                    ts
                ],
            )?;
        } else {
            self.conn.execute(
                "UPDATE article_bodies SET
                    body_markdown=?2, content_hash=?3, quality=?4, quality_rationale=?5,
                    resolved_url=?6, fetch_tool=?7, fetched_at=?8, retry_after='',
                    state='ready', updated_at=?8
                 WHERE id=?1",
                params![
                    id,
                    markdown,
                    content_hash,
                    quality,
                    rationale,
                    resolved_url,
                    fetch_tool,
                    ts
                ],
            )?;
        }
        if let Some(body) = self.article_body_by_id(id)? {
            let source = crate::nlp::Source::derived(id, &body.body_version.to_string(), markdown);
            crate::nlp::cache::register(
                &self.conn,
                "article",
                &body.article_id,
                &source,
                None,
                "historical-body-v1",
            )?;
        }
        Ok(())
    }

    pub fn mark_article_body_failed(
        &self,
        id: &str,
        rationale: &str,
        retry_after: &str,
        partial_markdown: &str,
        quality: &str,
    ) -> Result<()> {
        #[cfg(feature = "surreal")]
        if crate::storage::brain_store::on() {
            let (id, ra, ry, md, q) = (
                id.to_string(),
                rationale.to_string(),
                retry_after.to_string(),
                partial_markdown.to_string(),
                quality.to_string(),
            );
            return crate::storage::runtime::blocking(|repo| async move {
                sb::mark_failed(&repo, &id, &ra, &ry, &md, &q).await
            });
        }
        let ts = now();
        self.conn.execute(
            "UPDATE article_bodies SET
                quality=?2, quality_rationale=?3, retry_after=?4,
                body_markdown=CASE WHEN length(?5)>0 THEN ?5 ELSE body_markdown END,
                state='failed', updated_at=?6
             WHERE id=?1",
            params![id, quality, rationale, retry_after, partial_markdown, ts],
        )?;
        Ok(())
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
        #[cfg(feature = "surreal")]
        if crate::storage::brain_store::on() {
            let _ = remote_job_id;
            let (b, t, i, st, r) = (
                body_id.to_string(),
                tool_id.to_string(),
                input_identity.to_string(),
                state.to_string(),
                reason.to_string(),
            );
            return crate::storage::runtime::blocking(|repo| async move {
                sb::insert_attempt(&repo, &b, &t, &i, &st, &r, body_version).await
            });
        }
        let id = new_row_id("attempt");
        let ts = now();
        let finished = if state == "running" {
            String::new()
        } else {
            ts.clone()
        };
        self.conn.execute(
            "INSERT INTO article_retrieval_attempts(
                id, body_id, tool_id, input_identity, state, started_at, finished_at,
                reason, remote_job_id, body_version
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![
                id,
                body_id,
                tool_id,
                input_identity,
                state,
                ts,
                finished,
                reason,
                remote_job_id,
                body_version
            ],
        )?;
        Ok(id)
    }

    pub fn finish_retrieval_attempt(
        &self,
        id: &str,
        state: &str,
        reason: &str,
        remote_job_id: &str,
    ) -> Result<()> {
        #[cfg(feature = "surreal")]
        if crate::storage::brain_store::on() {
            let _ = remote_job_id;
            let (id, st, r) = (id.to_string(), state.to_string(), reason.to_string());
            return crate::storage::runtime::blocking(|repo| async move {
                sb::finish_attempt(&repo, &id, &st, &r).await
            });
        }
        self.conn.execute(
            "UPDATE article_retrieval_attempts SET state=?2, finished_at=?3, reason=?4, remote_job_id=?5
             WHERE id=?1",
            params![id, state, now(), reason, remote_job_id],
        )?;
        Ok(())
    }

    pub fn intel_report_job(&self, id: &str) -> Result<Option<IntelReportJobRow>> {
        #[cfg(feature = "surreal")]
        if crate::storage::brain_store::on() {
            let id = id.to_string();
            return crate::storage::runtime::blocking(|repo| async move {
                crate::storage::recon_jobs::report_job(&repo, &id).await
            });
        }
        let mut stmt = self.conn.prepare(
            "SELECT id, investigation_id, article_id, mode, revision, state, stage,
                    settings_json, budget_json, parent_job_id,
                    sections_done, sections_total, elements_done, elements_total,
                    tool_calls_done, tool_calls_allowance, current_tool, warning, error,
                    generation, started_at, updated_at, finished_at
             FROM intel_report_jobs WHERE id=?1",
        )?;
        Ok(stmt.query_row([id], job_from_row).optional()?)
    }
}

fn article_body_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ArticleBodyRow> {
    Ok(ArticleBodyRow {
        id: row.get(0)?,
        article_id: row.get(1)?,
        run_id: row.get(2)?,
        original_url: row.get(3)?,
        resolved_url: row.get(4)?,
        source_domain: row.get(5)?,
        source_name: row.get(6)?,
        body_markdown: row.get(7)?,
        content_hash: row.get(8)?,
        quality: row.get(9)?,
        quality_rationale: row.get(10)?,
        body_version: row.get(11)?,
        fetch_tool: row.get(12)?,
        fetched_at: row.get(13)?,
        retry_after: row.get(14)?,
        state: row.get(15)?,
        created_at: row.get(16)?,
        updated_at: row.get(17)?,
    })
}

fn job_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<IntelReportJobRow> {
    Ok(IntelReportJobRow {
        id: row.get(0)?,
        investigation_id: row.get(1)?,
        article_id: row.get(2)?,
        mode: row.get(3)?,
        revision: row.get(4)?,
        state: row.get(5)?,
        stage: row.get(6)?,
        settings_json: row.get(7)?,
        budget_json: row.get(8)?,
        parent_job_id: row.get(9)?,
        sections_done: row.get(10)?,
        sections_total: row.get(11)?,
        elements_done: row.get(12)?,
        elements_total: row.get(13)?,
        tool_calls_done: row.get(14)?,
        tool_calls_allowance: row.get(15)?,
        current_tool: row.get(16)?,
        warning: row.get(17)?,
        error: row.get(18)?,
        generation: row.get(19)?,
        started_at: row.get(20)?,
        updated_at: row.get(21)?,
        finished_at: row.get(22)?,
    })
}
