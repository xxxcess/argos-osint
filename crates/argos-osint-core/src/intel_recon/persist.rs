//! Store CRUD for Intel article bodies and report jobs.

use anyhow::Result;
use rusqlite::{params, OptionalExtension};

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

#[derive(Clone, Debug, PartialEq)]
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
}

#[derive(Clone, Debug, PartialEq)]
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
        self.conn.execute(
            "UPDATE article_bodies SET state=?2, updated_at=?3 WHERE id=?1",
            params![id, state, now()],
        )?;
        Ok(())
    }

    /// Clear stored article prose before a Reload / forced re-fetch so the UI does not
    /// keep showing the previous body while retrieval and synthesis run.
    pub fn clear_article_body_content(&self, id: &str) -> Result<()> {
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
        self.conn.execute(
            "UPDATE article_retrieval_attempts SET state=?2, finished_at=?3, reason=?4, remote_job_id=?5
             WHERE id=?1",
            params![id, state, now(), reason, remote_job_id],
        )?;
        Ok(())
    }

    pub fn list_retrieval_attempts(&self, body_id: &str) -> Result<Vec<RetrievalAttemptRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, body_id, tool_id, input_identity, state, started_at, finished_at,
                    reason, remote_job_id, body_version
             FROM article_retrieval_attempts WHERE body_id=?1 ORDER BY started_at",
        )?;
        let rows = stmt.query_map([body_id], |row| {
            Ok(RetrievalAttemptRow {
                id: row.get(0)?,
                body_id: row.get(1)?,
                tool_id: row.get(2)?,
                input_identity: row.get(3)?,
                state: row.get(4)?,
                started_at: row.get(5)?,
                finished_at: row.get(6)?,
                reason: row.get(7)?,
                remote_job_id: row.get(8)?,
                body_version: row.get(9)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn ensure_intel_investigation(
        &self,
        article_id: &str,
        run_id: &str,
        article_url: &str,
        scope_json: &str,
    ) -> Result<IntelInvestigationRow> {
        if let Some(existing) = self.intel_investigation_for_article(article_id)? {
            return Ok(existing);
        }
        let id = new_row_id("iinv");
        let ts = now();
        self.conn.execute(
            "INSERT INTO intel_investigations(
                id, article_id, run_id, article_url, thread_id, scope_json,
                shared_assessment_json, shared_assessment_version, created_at, updated_at
             ) VALUES (?1,?2,?3,?4,NULL,?5,'{}',0,?6,?6)",
            params![id, article_id, run_id, article_url, scope_json, ts],
        )?;
        Ok(self.intel_investigation_by_id(&id)?.expect("just inserted"))
    }

    pub fn intel_investigation_for_article(
        &self,
        article_id: &str,
    ) -> Result<Option<IntelInvestigationRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, article_id, run_id, article_url, thread_id, scope_json,
                    shared_assessment_json, shared_assessment_version, created_at, updated_at
             FROM intel_investigations WHERE article_id=?1",
        )?;
        Ok(stmt
            .query_row([article_id], investigation_from_row)
            .optional()?)
    }

    pub fn intel_investigation_by_id(&self, id: &str) -> Result<Option<IntelInvestigationRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, article_id, run_id, article_url, thread_id, scope_json,
                    shared_assessment_json, shared_assessment_version, created_at, updated_at
             FROM intel_investigations WHERE id=?1",
        )?;
        Ok(stmt.query_row([id], investigation_from_row).optional()?)
    }

    pub fn set_investigation_thread(&self, investigation_id: &str, thread_id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE intel_investigations SET thread_id=?2, updated_at=?3 WHERE id=?1",
            params![investigation_id, thread_id, now()],
        )?;
        Ok(())
    }

    pub fn update_shared_assessment(
        &self,
        investigation_id: &str,
        assessment_json: &str,
    ) -> Result<i64> {
        let ts = now();
        self.conn.execute(
            "UPDATE intel_investigations SET
                shared_assessment_json=?2,
                shared_assessment_version=shared_assessment_version+1,
                updated_at=?3
             WHERE id=?1",
            params![investigation_id, assessment_json, ts],
        )?;
        let version: i64 = self.conn.query_row(
            "SELECT shared_assessment_version FROM intel_investigations WHERE id=?1",
            [investigation_id],
            |row| row.get(0),
        )?;
        Ok(version)
    }

    pub fn insert_report_job(
        &self,
        investigation_id: &str,
        article_id: &str,
        mode: &str,
        revision: i64,
        settings_json: &str,
        budget_json: &str,
        sections_total: i64,
        elements_total: i64,
        tool_calls_allowance: i64,
        parent_job_id: Option<&str>,
    ) -> Result<IntelReportJobRow> {
        let id = new_row_id("ijob");
        let ts = now();
        self.conn.execute(
            "INSERT INTO intel_report_jobs(
                id, investigation_id, article_id, mode, revision, state, stage,
                settings_json, budget_json, parent_job_id,
                sections_done, sections_total, elements_done, elements_total,
                tool_calls_done, tool_calls_allowance, current_tool, warning, error,
                generation, started_at, updated_at, finished_at
             ) VALUES (?1,?2,?3,?4,?5,'queued','planning',?6,?7,?8,0,?9,0,?10,0,?11,'','','',1,?12,?12,'')",
            params![
                id,
                investigation_id,
                article_id,
                mode,
                revision,
                settings_json,
                budget_json,
                parent_job_id,
                sections_total,
                elements_total,
                tool_calls_allowance,
                ts
            ],
        )?;
        Ok(self.intel_report_job(&id)?.expect("just inserted"))
    }

    pub fn intel_report_job(&self, id: &str) -> Result<Option<IntelReportJobRow>> {
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

    pub fn intel_jobs_for_article(&self, article_id: &str) -> Result<Vec<IntelReportJobRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, investigation_id, article_id, mode, revision, state, stage,
                    settings_json, budget_json, parent_job_id,
                    sections_done, sections_total, elements_done, elements_total,
                    tool_calls_done, tool_calls_allowance, current_tool, warning, error,
                    generation, started_at, updated_at, finished_at
             FROM intel_report_jobs WHERE article_id=?1
             ORDER BY updated_at DESC",
        )?;
        let rows = stmt.query_map([article_id], job_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn active_intel_job(
        &self,
        article_id: &str,
        mode: &str,
    ) -> Result<Option<IntelReportJobRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, investigation_id, article_id, mode, revision, state, stage,
                    settings_json, budget_json, parent_job_id,
                    sections_done, sections_total, elements_done, elements_total,
                    tool_calls_done, tool_calls_allowance, current_tool, warning, error,
                    generation, started_at, updated_at, finished_at
             FROM intel_report_jobs
             WHERE article_id=?1 AND mode=?2
               AND state IN ('queued','running','paused','waiting')
             ORDER BY revision DESC LIMIT 1",
        )?;
        Ok(stmt
            .query_row(params![article_id, mode], job_from_row)
            .optional()?)
    }

    pub fn next_job_revision(&self, article_id: &str, mode: &str) -> Result<i64> {
        let max: Option<i64> = self.conn.query_row(
            "SELECT MAX(revision) FROM intel_report_jobs WHERE article_id=?1 AND mode=?2",
            params![article_id, mode],
            |row| row.get(0),
        )?;
        Ok(max.unwrap_or(0) + 1)
    }

    pub fn insert_report_attempt(
        &self,
        job_id: &str,
        generation: i64,
        task_id: &str,
        tool_id: &str,
    ) -> Result<String> {
        let id = new_row_id("atmpt");
        let tx = self.conn.unchecked_transaction()?;
        let reserved = tx.execute(
            "UPDATE intel_report_jobs
             SET tool_calls_done = tool_calls_done + 1, current_tool = ?3, updated_at = ?4
             WHERE id = ?1 AND generation = ?2
               AND state IN ('queued', 'running', 'waiting')
               AND tool_calls_done >= 0
               AND tool_calls_done < tool_calls_allowance",
            rusqlite::params![job_id, generation, tool_id, now()],
        )?;
        anyhow::ensure!(
            reserved == 1,
            "Report tool budget exhausted or job is no longer active"
        );
        tx.execute(
            "INSERT INTO intel_report_attempts (id, job_id, generation, task_id, tool_id, state, started_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 'running', ?6)",
            rusqlite::params![id, job_id, generation, task_id, tool_id, now()],
        )?;
        tx.commit()?;
        Ok(id)
    }

    pub fn finish_report_attempt(&self, job_id: &str, id: &str, state: &str) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        let finished = tx.execute(
            "UPDATE intel_report_attempts SET state=?3, finished_at=?4
             WHERE id=?1 AND job_id=?2 AND state='running'",
            rusqlite::params![id, job_id, state, now()],
        )?;
        if finished == 1 {
            tx.execute(
                "UPDATE intel_report_jobs SET current_tool = '' WHERE id = ?1
                 AND generation = (SELECT generation FROM intel_report_attempts WHERE id=?2)
                 AND NOT EXISTS (SELECT 1 FROM intel_report_attempts
                                 WHERE job_id=?1 AND generation=intel_report_jobs.generation AND state='running')",
                rusqlite::params![job_id, id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn update_report_job(
        &self,
        id: &str,
        state: &str,
        stage: &str,
        sections_done: i64,
        elements_done: i64,
        warning: &str,
        error: &str,
    ) -> Result<()> {
        let finished = if matches!(state, "completed" | "failed" | "cancelled" | "partial") {
            now()
        } else {
            String::new()
        };
        self.conn.execute(
            "UPDATE intel_report_jobs SET
                state=?2, stage=?3, sections_done=?4, elements_done=?5,
                warning=?6, error=?7,
                updated_at=?8,
                finished_at=CASE WHEN length(?9)>0 THEN ?9 ELSE finished_at END
             WHERE id=?1",
            params![
                id,
                state,
                stage,
                sections_done,
                elements_done,
                warning,
                error,
                now(),
                finished
            ],
        )?;
        Ok(())
    }

    pub fn insert_report_section(
        &self,
        job_id: &str,
        section_key: &str,
        title: &str,
        ordinal: i64,
        status: &str,
        waiting_on: &str,
    ) -> Result<IntelReportSectionRow> {
        let id = new_row_id("isec");
        let ts = now();
        self.conn.execute(
            "INSERT INTO intel_report_sections(
                id, job_id, section_key, title, ordinal, markdown, status, revision,
                evidence_ids_json, judgment_json, assessment_version, waiting_on,
                created_at, updated_at
             ) VALUES (?1,?2,?3,?4,?5,'',?6,1,'[]','{}',0,?7,?8,?8)",
            params![
                id,
                job_id,
                section_key,
                title,
                ordinal,
                status,
                waiting_on,
                ts
            ],
        )?;
        Ok(self
            .intel_report_section(job_id, section_key)?
            .expect("just inserted"))
    }

    pub fn intel_report_section(
        &self,
        job_id: &str,
        section_key: &str,
    ) -> Result<Option<IntelReportSectionRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, job_id, section_key, title, ordinal, markdown, status, revision,
                    evidence_ids_json, judgment_json, assessment_version, waiting_on,
                    created_at, updated_at
             FROM intel_report_sections WHERE job_id=?1 AND section_key=?2",
        )?;
        Ok(stmt
            .query_row(params![job_id, section_key], section_from_row)
            .optional()?)
    }

    pub fn intel_report_sections(&self, job_id: &str) -> Result<Vec<IntelReportSectionRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, job_id, section_key, title, ordinal, markdown, status, revision,
                    evidence_ids_json, judgment_json, assessment_version, waiting_on,
                    created_at, updated_at
             FROM intel_report_sections WHERE job_id=?1 ORDER BY ordinal",
        )?;
        let rows = stmt.query_map([job_id], section_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn upsert_report_section_markdown(
        &self,
        job_id: &str,
        section_key: &str,
        markdown: &str,
        status: &str,
        evidence_ids_json: &str,
        judgment_json: &str,
        assessment_version: i64,
        waiting_on: &str,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE intel_report_sections SET
                markdown=?3, status=?4, evidence_ids_json=?5, judgment_json=?6,
                assessment_version=?7, waiting_on=?8, revision=revision+1, updated_at=?9
             WHERE job_id=?1 AND section_key=?2",
            params![
                job_id,
                section_key,
                markdown,
                status,
                evidence_ids_json,
                judgment_json,
                assessment_version,
                waiting_on,
                now()
            ],
        )?;
        Ok(())
    }

    pub fn mark_sections_stale(
        &self,
        job_id: &str,
        keys: &[String],
        waiting_on: &str,
    ) -> Result<()> {
        for key in keys {
            self.conn.execute(
                "UPDATE intel_report_sections SET status='stale', waiting_on=?3, updated_at=?4
                 WHERE job_id=?1 AND section_key=?2 AND status='complete'",
                params![job_id, key, waiting_on, now()],
            )?;
        }
        Ok(())
    }

    pub fn insert_report_task(
        &self,
        job_id: &str,
        task_type: &str,
        section_key: &str,
        depends_on_json: &str,
        status: &str,
    ) -> Result<String> {
        let id = new_row_id("itask");
        let ts = now();
        self.conn.execute(
            "INSERT INTO intel_report_tasks(
                id, job_id, task_type, section_key, depends_on_json, input_hash,
                status, attempts, max_attempts, output_ref, lease_owner, lease_until,
                error, created_at, updated_at
             ) VALUES (?1,?2,?3,?4,?5,'',?6,0,3,'','','','',?7,?7)",
            params![
                id,
                job_id,
                task_type,
                section_key,
                depends_on_json,
                status,
                ts
            ],
        )?;
        Ok(id)
    }

    pub fn intel_report_tasks(&self, job_id: &str) -> Result<Vec<IntelReportTaskRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, job_id, task_type, section_key, depends_on_json, input_hash,
                    status, attempts, max_attempts, output_ref, lease_owner, lease_until,
                    error, created_at, updated_at
             FROM intel_report_tasks WHERE job_id=?1 ORDER BY created_at",
        )?;
        let rows = stmt.query_map([job_id], task_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn claim_report_task(
        &self,
        job_id: &str,
        owner: &str,
        lease_secs: i64,
    ) -> Result<Option<IntelReportTaskRow>> {
        let tasks = self.intel_report_tasks(job_id)?;
        let completed: std::collections::HashSet<String> = tasks
            .iter()
            .filter(|t| t.status == "completed")
            .map(|t| t.id.clone())
            .collect();
        let now_ts = chrono::Utc::now();
        let lease_until = (now_ts + chrono::Duration::seconds(lease_secs)).to_rfc3339();
        for task in &tasks {
            if task.status != "pending" && task.status != "interrupted" {
                if task.status == "running" {
                    if !task.lease_until.is_empty() {
                        if let Ok(until) = chrono::DateTime::parse_from_rfc3339(&task.lease_until) {
                            if until > now_ts {
                                continue;
                            }
                        }
                    } else {
                        continue;
                    }
                } else {
                    continue;
                }
            }
            let deps: Vec<String> = serde_json::from_str(&task.depends_on_json).unwrap_or_default();
            if !deps.iter().all(|d| completed.contains(d)) {
                continue;
            }
            let updated = self.conn.execute(
                "UPDATE intel_report_tasks SET
                    status='running', lease_owner=?2, lease_until=?3,
                    attempts=attempts+1, updated_at=?4
                 WHERE id=?1 AND status IN ('pending','interrupted','running')",
                params![task.id, owner, lease_until, now()],
            )?;
            if updated == 1 {
                return self.intel_report_task(&task.id);
            }
        }
        Ok(None)
    }

    pub fn intel_report_task(&self, id: &str) -> Result<Option<IntelReportTaskRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, job_id, task_type, section_key, depends_on_json, input_hash,
                    status, attempts, max_attempts, output_ref, lease_owner, lease_until,
                    error, created_at, updated_at
             FROM intel_report_tasks WHERE id=?1",
        )?;
        Ok(stmt.query_row([id], task_from_row).optional()?)
    }

    pub fn complete_report_task(&self, id: &str, output_ref: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE intel_report_tasks SET
                status='completed', output_ref=?2, lease_owner='', lease_until='',
                error='', updated_at=?3
             WHERE id=?1",
            params![id, output_ref, now()],
        )?;
        Ok(())
    }

    pub fn fail_report_task(&self, id: &str, error: &str, retryable: bool) -> Result<()> {
        let status = if retryable { "pending" } else { "failed" };
        self.conn.execute(
            "UPDATE intel_report_tasks SET
                status=?2, error=?3, lease_owner='', lease_until='', updated_at=?4
             WHERE id=?1",
            params![id, status, error, now()],
        )?;
        Ok(())
    }

    pub fn interrupt_expired_leases(&self) -> Result<usize> {
        let ts = now();
        let n = self.conn.execute(
            "UPDATE intel_report_tasks SET status='interrupted', lease_owner='', updated_at=?1
             WHERE status='running' AND lease_until<>'' AND lease_until<?1",
            params![ts],
        )?;
        Ok(n)
    }

    pub fn upsert_element(
        &self,
        investigation_id: &str,
        element_key: &str,
        fingerprint: &str,
        element_type: &str,
        original_text: &str,
        provenance: &str,
        status: &str,
    ) -> Result<IntelElementRow> {
        let existing: Option<String> = self
            .conn
            .query_row(
                "SELECT id FROM intel_element_ledger
                 WHERE investigation_id=?1 AND element_key=?2",
                params![investigation_id, element_key],
                |row| row.get(0),
            )
            .optional()?;
        let ts = now();
        if let Some(id) = existing {
            self.conn.execute(
                "UPDATE intel_element_ledger SET
                    fingerprint=?2, element_type=?3, original_text=?4, provenance=?5,
                    status=?6, updated_at=?7
                 WHERE id=?1",
                params![
                    id,
                    fingerprint,
                    element_type,
                    original_text,
                    provenance,
                    status,
                    ts
                ],
            )?;
            return Ok(self.intel_element(&id)?.expect("exists"));
        }
        let id = new_row_id("iel");
        self.conn.execute(
            "INSERT INTO intel_element_ledger(
                id, investigation_id, element_key, fingerprint, element_type, original_text,
                provenance, status, stance, assessment, uncertainty, evidence_ids_json,
                directive_ids_json, section_keys_json, excluded_reason, created_at, updated_at
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,'','','','[]','[]','[]','',?9,?9)",
            params![
                id,
                investigation_id,
                element_key,
                fingerprint,
                element_type,
                original_text,
                provenance,
                status,
                ts
            ],
        )?;
        Ok(self.intel_element(&id)?.expect("just inserted"))
    }

    pub fn intel_element(&self, id: &str) -> Result<Option<IntelElementRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, investigation_id, element_key, fingerprint, element_type, original_text,
                    provenance, status, stance, assessment, uncertainty, evidence_ids_json,
                    directive_ids_json, section_keys_json, excluded_reason, created_at, updated_at
             FROM intel_element_ledger WHERE id=?1",
        )?;
        Ok(stmt.query_row([id], element_from_row).optional()?)
    }

    pub fn intel_elements(&self, investigation_id: &str) -> Result<Vec<IntelElementRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, investigation_id, element_key, fingerprint, element_type, original_text,
                    provenance, status, stance, assessment, uncertainty, evidence_ids_json,
                    directive_ids_json, section_keys_json, excluded_reason, created_at, updated_at
             FROM intel_element_ledger WHERE investigation_id=?1 ORDER BY created_at",
        )?;
        let rows = stmt.query_map([investigation_id], element_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn update_element_assessment(
        &self,
        id: &str,
        status: &str,
        stance: &str,
        assessment: &str,
        uncertainty: &str,
        evidence_ids_json: &str,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE intel_element_ledger SET
                status=?2, stance=?3, assessment=?4, uncertainty=?5,
                evidence_ids_json=?6, updated_at=?7
             WHERE id=?1",
            params![
                id,
                status,
                stance,
                assessment,
                uncertainty,
                evidence_ids_json,
                now()
            ],
        )?;
        Ok(())
    }

    pub fn insert_evidence(
        &self,
        investigation_id: &str,
        source_url: &str,
        source_domain: &str,
        excerpt: &str,
        location: &str,
        claim_ids_json: &str,
        source_date: &str,
        event_date: &str,
        stance: &str,
        origin: &str,
        tool_id: &str,
        call_id: &str,
        limitations: &str,
    ) -> Result<String> {
        let id = new_row_id("iev");
        let ts = now();
        self.conn.execute(
            "INSERT INTO intel_evidence(
                id, investigation_id, source_url, source_domain, excerpt, location,
                claim_ids_json, source_date, event_date, retrieved_at, stance, origin,
                independence, limitations, tool_id, call_id, created_at
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,'',?13,?14,?15,?10)",
            params![
                id,
                investigation_id,
                source_url,
                source_domain,
                excerpt,
                location,
                claim_ids_json,
                source_date,
                event_date,
                ts,
                stance,
                origin,
                limitations,
                tool_id,
                call_id
            ],
        )?;
        Ok(id)
    }

    pub fn intel_evidence_list(&self, investigation_id: &str) -> Result<Vec<IntelEvidenceRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, investigation_id, source_url, source_domain, excerpt, location,
                    claim_ids_json, source_date, event_date, retrieved_at, stance, origin,
                    independence, limitations, tool_id, call_id, created_at
             FROM intel_evidence WHERE investigation_id=?1 ORDER BY created_at",
        )?;
        let rows = stmt.query_map([investigation_id], |row| {
            Ok(IntelEvidenceRow {
                id: row.get(0)?,
                investigation_id: row.get(1)?,
                source_url: row.get(2)?,
                source_domain: row.get(3)?,
                excerpt: row.get(4)?,
                location: row.get(5)?,
                claim_ids_json: row.get(6)?,
                source_date: row.get(7)?,
                event_date: row.get(8)?,
                retrieved_at: row.get(9)?,
                stance: row.get(10)?,
                origin: row.get(11)?,
                independence: row.get(12)?,
                limitations: row.get(13)?,
                tool_id: row.get(14)?,
                call_id: row.get(15)?,
                created_at: row.get(16)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn insert_assessment(
        &self,
        investigation_id: &str,
        element_id: &str,
        origin: &str,
        stance: &str,
        rationale: &str,
        confidence: f64,
        sources_json: &str,
        original_value_json: &str,
        revised_value_json: &str,
    ) -> Result<String> {
        let id = new_row_id("iass");
        let ts = now();
        self.conn.execute(
            "INSERT INTO intel_assessments(
                id, investigation_id, element_id, origin, stance, rationale, confidence,
                sources_json, original_value_json, revised_value_json, created_at, updated_at
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?11)",
            params![
                id,
                investigation_id,
                element_id,
                origin,
                stance,
                rationale,
                confidence,
                sources_json,
                original_value_json,
                revised_value_json,
                ts
            ],
        )?;
        Ok(id)
    }

    /// Articles with retained Intel investigations are not dropped by Atlas prune.
    pub fn article_ids_with_intel_reports(&self) -> Result<std::collections::HashSet<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT DISTINCT article_id FROM intel_investigations")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
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

fn investigation_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<IntelInvestigationRow> {
    Ok(IntelInvestigationRow {
        id: row.get(0)?,
        article_id: row.get(1)?,
        run_id: row.get(2)?,
        article_url: row.get(3)?,
        thread_id: row.get(4)?,
        scope_json: row.get(5)?,
        shared_assessment_json: row.get(6)?,
        shared_assessment_version: row.get(7)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
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

fn section_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<IntelReportSectionRow> {
    Ok(IntelReportSectionRow {
        id: row.get(0)?,
        job_id: row.get(1)?,
        section_key: row.get(2)?,
        title: row.get(3)?,
        ordinal: row.get(4)?,
        markdown: row.get(5)?,
        status: row.get(6)?,
        revision: row.get(7)?,
        evidence_ids_json: row.get(8)?,
        judgment_json: row.get(9)?,
        assessment_version: row.get(10)?,
        waiting_on: row.get(11)?,
        created_at: row.get(12)?,
        updated_at: row.get(13)?,
    })
}

fn task_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<IntelReportTaskRow> {
    Ok(IntelReportTaskRow {
        id: row.get(0)?,
        job_id: row.get(1)?,
        task_type: row.get(2)?,
        section_key: row.get(3)?,
        depends_on_json: row.get(4)?,
        input_hash: row.get(5)?,
        status: row.get(6)?,
        attempts: row.get(7)?,
        max_attempts: row.get(8)?,
        output_ref: row.get(9)?,
        lease_owner: row.get(10)?,
        lease_until: row.get(11)?,
        error: row.get(12)?,
        created_at: row.get(13)?,
        updated_at: row.get(14)?,
    })
}

fn element_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<IntelElementRow> {
    Ok(IntelElementRow {
        id: row.get(0)?,
        investigation_id: row.get(1)?,
        element_key: row.get(2)?,
        fingerprint: row.get(3)?,
        element_type: row.get(4)?,
        original_text: row.get(5)?,
        provenance: row.get(6)?,
        status: row.get(7)?,
        stance: row.get(8)?,
        assessment: row.get(9)?,
        uncertainty: row.get(10)?,
        evidence_ids_json: row.get(11)?,
        directive_ids_json: row.get(12)?,
        section_keys_json: row.get(13)?,
        excluded_reason: row.get(14)?,
        created_at: row.get(15)?,
        updated_at: row.get(16)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_creates_intel_tables_and_body_round_trip() {
        let store = Store::memory().unwrap();
        let body = store
            .ensure_article_body(
                "art-1",
                "run-1",
                "https://example.com/a",
                "example.com",
                "Ex",
            )
            .unwrap();
        assert_eq!(body.article_id, "art-1");
        store
            .commit_article_body(
                &body.id,
                "# Hello\n\nWorld",
                "hash1",
                "complete",
                "ok",
                "https://example.com/a",
                "firecrawl_scrape",
                false,
            )
            .unwrap();
        let loaded = store.article_body_for_article("art-1").unwrap().unwrap();
        assert_eq!(loaded.quality, "complete");
        assert!(loaded.body_markdown.contains("Hello"));
        let inv = store
            .ensure_intel_investigation("art-1", "run-1", "https://example.com/a", "{}")
            .unwrap();
        let job = store
            .insert_report_job(&inv.id, "art-1", "verify", 1, "{}", "{}", 6, 0, 12, None)
            .unwrap();
        store
            .insert_report_section(
                &job.id,
                "bluf",
                "Key Judgments / BLUF",
                0,
                "waiting",
                "evidence",
            )
            .unwrap();
        let sections = store.intel_report_sections(&job.id).unwrap();
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].section_key, "bluf");
    }
}
