//! Persistence transactions for tasks, dependencies, passages, assessments, events, and stream parts.

use anyhow::Result;
use chrono::Utc;
use rusqlite::params;

use super::contracts::{InvestigationSurface, TaskRecord, TaskStatus};
use super::evidence::{ClaimAssessment, ClaimStance, EvidencePassage};
use super::trace::InvestigationEvent;
use crate::store::Store;

impl Store {
    // -------------------------------------------------------------------------
    // Tasks
    // -------------------------------------------------------------------------

    pub fn insert_investigation_task(&self, task: &TaskRecord) -> Result<()> {
        let ts = Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO investigation_tasks(
                id, investigation_id, run_id, directive_id, task_id, revision,
                surface, objective, report_mode, strategy, bindings_json,
                required_evidence_json, freshness_cutoff, allowed_capabilities_json,
                policy_json, budget_ceiling_json, output_schema_json,
                completion_criteria, attempts, max_attempts, status, output_ref,
                unmet_needs_json, terminal_reason, superseded_revision,
                created_at, updated_at
            ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25,?26,?27)
            ON CONFLICT(id) DO UPDATE SET
                status=excluded.status,
                attempts=excluded.attempts,
                output_ref=excluded.output_ref,
                unmet_needs_json=excluded.unmet_needs_json,
                terminal_reason=excluded.terminal_reason,
                updated_at=excluded.updated_at",
            params![
                task.id,
                task.investigation_id,
                task.run_id,
                task.directive_id,
                task.task_id,
                task.revision,
                task.surface.as_str(),
                task.objective,
                task.report_mode,
                task.strategy,
                serde_json::to_string(&task.bindings)?,
                serde_json::to_string(&task.required_evidence)?,
                task.freshness_cutoff,
                serde_json::to_string(&task.allowed_capabilities)?,
                serde_json::to_string(&task.policy)?,
                serde_json::to_string(&task.budget_ceiling)?,
                serde_json::to_string(&task.output_schema)?,
                task.completion_criteria,
                task.attempts,
                task.max_attempts,
                task.status.as_str(),
                task.output_ref,
                serde_json::to_string(&task.unmet_needs)?,
                task.terminal_reason,
                task.superseded_revision,
                task.created_at,
                ts,
            ],
        )?;
        Ok(())
    }

    pub fn get_investigation_task(&self, id: &str) -> Result<Option<TaskRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, investigation_id, run_id, directive_id, task_id, revision,
                    surface, objective, report_mode, strategy, bindings_json,
                    required_evidence_json, freshness_cutoff, allowed_capabilities_json,
                    policy_json, budget_ceiling_json, output_schema_json,
                    completion_criteria, attempts, max_attempts, status, output_ref,
                    unmet_needs_json, terminal_reason, superseded_revision,
                    created_at, updated_at
             FROM investigation_tasks WHERE id=?1",
        )?;
        let mut rows = stmt.query([id])?;
        if let Some(row) = rows.next()? {
            let surface_str: String = row.get(6)?;
            let status_str: String = row.get(20)?;
            let bindings_str: String = row.get(10)?;
            let req_ev_str: String = row.get(11)?;
            let caps_str: String = row.get(13)?;
            let policy_str: String = row.get(14)?;
            let budget_str: String = row.get(15)?;
            let schema_str: String = row.get(16)?;
            let unmet_str: String = row.get(22)?;

            Ok(Some(TaskRecord {
                id: row.get(0)?,
                investigation_id: row.get(1)?,
                run_id: row.get(2)?,
                directive_id: row.get(3)?,
                task_id: row.get(4)?,
                revision: row.get(5)?,
                surface: InvestigationSurface::parse(&surface_str),
                objective: row.get(7)?,
                report_mode: row.get(8)?,
                strategy: row.get(9)?,
                bindings: serde_json::from_str(&bindings_str).unwrap_or_default(),
                required_evidence: serde_json::from_str(&req_ev_str).unwrap_or_default(),
                freshness_cutoff: row.get(12)?,
                allowed_capabilities: serde_json::from_str(&caps_str).unwrap_or_default(),
                policy: serde_json::from_str(&policy_str).unwrap_or_default(),
                budget_ceiling: serde_json::from_str(&budget_str).unwrap_or_default(),
                output_schema: serde_json::from_str(&schema_str).unwrap_or_default(),
                completion_criteria: row.get(17)?,
                attempts: row.get(18)?,
                max_attempts: row.get(19)?,
                status: TaskStatus::parse(&status_str),
                output_ref: row.get(21)?,
                unmet_needs: serde_json::from_str(&unmet_str).unwrap_or_default(),
                terminal_reason: row.get(23)?,
                superseded_revision: row.get(24)?,
                created_at: row.get(25)?,
                updated_at: row.get(26)?,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn list_investigation_tasks(&self, investigation_id: &str) -> Result<Vec<TaskRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, investigation_id, run_id, directive_id, task_id, revision,
                    surface, objective, report_mode, strategy, bindings_json,
                    required_evidence_json, freshness_cutoff, allowed_capabilities_json,
                    policy_json, budget_ceiling_json, output_schema_json,
                    completion_criteria, attempts, max_attempts, status, output_ref,
                    unmet_needs_json, terminal_reason, superseded_revision,
                    created_at, updated_at
             FROM investigation_tasks WHERE investigation_id=?1 ORDER BY revision ASC, task_id ASC",
        )?;
        let mut rows = stmt.query([investigation_id])?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            let surface_str: String = row.get(6)?;
            let status_str: String = row.get(20)?;
            let bindings_str: String = row.get(10)?;
            let req_ev_str: String = row.get(11)?;
            let caps_str: String = row.get(13)?;
            let policy_str: String = row.get(14)?;
            let budget_str: String = row.get(15)?;
            let schema_str: String = row.get(16)?;
            let unmet_str: String = row.get(22)?;

            out.push(TaskRecord {
                id: row.get(0)?,
                investigation_id: row.get(1)?,
                run_id: row.get(2)?,
                directive_id: row.get(3)?,
                task_id: row.get(4)?,
                revision: row.get(5)?,
                surface: InvestigationSurface::parse(&surface_str),
                objective: row.get(7)?,
                report_mode: row.get(8)?,
                strategy: row.get(9)?,
                bindings: serde_json::from_str(&bindings_str).unwrap_or_default(),
                required_evidence: serde_json::from_str(&req_ev_str).unwrap_or_default(),
                freshness_cutoff: row.get(12)?,
                allowed_capabilities: serde_json::from_str(&caps_str).unwrap_or_default(),
                policy: serde_json::from_str(&policy_str).unwrap_or_default(),
                budget_ceiling: serde_json::from_str(&budget_str).unwrap_or_default(),
                output_schema: serde_json::from_str(&schema_str).unwrap_or_default(),
                completion_criteria: row.get(17)?,
                attempts: row.get(18)?,
                max_attempts: row.get(19)?,
                status: TaskStatus::parse(&status_str),
                output_ref: row.get(21)?,
                unmet_needs: serde_json::from_str(&unmet_str).unwrap_or_default(),
                terminal_reason: row.get(23)?,
                superseded_revision: row.get(24)?,
                created_at: row.get(25)?,
                updated_at: row.get(26)?,
            });
        }
        Ok(out)
    }

    pub fn update_task_state(
        &self,
        task_id: &str,
        status: TaskStatus,
        output_ref: &str,
        terminal_reason: &str,
    ) -> Result<()> {
        let ts = Utc::now().to_rfc3339();
        self.conn.execute(
            "UPDATE investigation_tasks SET status=?2, output_ref=?3, terminal_reason=?4, updated_at=?5 WHERE id=?1",
            params![task_id, status.as_str(), output_ref, terminal_reason, ts],
        )?;
        Ok(())
    }

    // -------------------------------------------------------------------------
    // Dependencies
    // -------------------------------------------------------------------------

    pub fn add_task_dependency(&self, task_id: &str, depends_on_task_id: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO investigation_task_dependencies(task_id, depends_on_task_id) VALUES (?1,?2)
             ON CONFLICT(task_id, depends_on_task_id) DO NOTHING",
            params![task_id, depends_on_task_id],
        )?;
        Ok(())
    }

    pub fn get_task_dependencies(&self, task_id: &str) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT depends_on_task_id FROM investigation_task_dependencies WHERE task_id=?1",
        )?;
        let rows = stmt.query_map([task_id], |r| r.get(0))?;
        let mut deps = Vec::new();
        for dep in rows {
            deps.push(dep?);
        }
        Ok(deps)
    }

    // -------------------------------------------------------------------------
    // Evidence Passages
    // -------------------------------------------------------------------------

    pub fn insert_evidence_passage(&self, passage: &EvidencePassage) -> Result<()> {
        self.conn.execute(
            "INSERT INTO investigation_evidence_passages(
                id, investigation_id, task_id, call_id, source_url, source_domain,
                passage_text, observed_at, published_at, stance, relevance_score, created_at
            ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)
            ON CONFLICT(id) DO UPDATE SET
                stance=excluded.stance,
                relevance_score=excluded.relevance_score",
            params![
                passage.id,
                passage.investigation_id,
                passage.task_id,
                passage.call_id,
                passage.source_url,
                passage.source_domain,
                passage.passage_text,
                passage.observed_at,
                passage.published_at,
                passage.stance.as_str(),
                passage.relevance_score,
                passage.created_at,
            ],
        )?;
        Ok(())
    }

    pub fn list_evidence_passages(&self, investigation_id: &str) -> Result<Vec<EvidencePassage>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, investigation_id, task_id, call_id, source_url, source_domain,
                    passage_text, observed_at, published_at, stance, relevance_score, created_at
             FROM investigation_evidence_passages WHERE investigation_id=?1 ORDER BY created_at ASC",
        )?;
        let mut rows = stmt.query([investigation_id])?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            let stance_str: String = row.get(9)?;
            out.push(EvidencePassage {
                id: row.get(0)?,
                investigation_id: row.get(1)?,
                task_id: row.get(2)?,
                call_id: row.get(3)?,
                source_url: row.get(4)?,
                source_domain: row.get(5)?,
                passage_text: row.get(6)?,
                observed_at: row.get(7)?,
                published_at: row.get(8)?,
                stance: ClaimStance::parse(&stance_str),
                relevance_score: row.get(10)?,
                created_at: row.get(11)?,
            });
        }
        Ok(out)
    }

    // -------------------------------------------------------------------------
    // Claim Assessments
    // -------------------------------------------------------------------------

    pub fn insert_claim_assessment(&self, assessment: &ClaimAssessment) -> Result<()> {
        self.conn.execute(
            "INSERT INTO investigation_claim_assessments(
                id, investigation_id, claim_id, evidence_id, stance, rationale, created_at
            ) VALUES (?1,?2,?3,?4,?5,?6,?7)
            ON CONFLICT(id) DO UPDATE SET
                stance=excluded.stance,
                rationale=excluded.rationale",
            params![
                assessment.id,
                assessment.investigation_id,
                assessment.claim_id,
                assessment.evidence_id,
                assessment.stance.as_str(),
                assessment.rationale,
                assessment.created_at,
            ],
        )?;
        Ok(())
    }

    pub fn list_claim_assessments(&self, investigation_id: &str) -> Result<Vec<ClaimAssessment>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, investigation_id, claim_id, evidence_id, stance, rationale, created_at
             FROM investigation_claim_assessments WHERE investigation_id=?1 ORDER BY created_at ASC",
        )?;
        let mut rows = stmt.query([investigation_id])?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            let stance_str: String = row.get(4)?;
            out.push(ClaimAssessment {
                id: row.get(0)?,
                investigation_id: row.get(1)?,
                claim_id: row.get(2)?,
                evidence_id: row.get(3)?,
                stance: ClaimStance::parse(&stance_str),
                rationale: row.get(5)?,
                created_at: row.get(6)?,
            });
        }
        Ok(out)
    }

    // -------------------------------------------------------------------------
    // Durable Events & Stream Parts
    // -------------------------------------------------------------------------

    pub fn insert_investigation_event(&self, ev: &InvestigationEvent) -> Result<()> {
        self.conn.execute(
            "INSERT INTO investigation_events(
                id, investigation_id, sequence, occurrence_time, recording_time,
                surface, run_id, directive_id, task_id, revision, parent_event_id,
                role, model, provider, attempt_id, event_type, status, summary,
                payload_json, evidence_refs_json, superseded_event_id
            ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21)
            ON CONFLICT(id) DO UPDATE SET
                status=excluded.status,
                summary=excluded.summary,
                payload_json=excluded.payload_json",
            params![
                ev.id,
                ev.investigation_id,
                ev.sequence,
                ev.occurrence_time,
                ev.recording_time,
                ev.surface.as_str(),
                ev.run_id,
                ev.directive_id,
                ev.task_id,
                ev.revision,
                ev.parent_event_id,
                ev.role,
                ev.model,
                ev.provider,
                ev.attempt_id,
                ev.event_type,
                ev.status,
                ev.summary,
                serde_json::to_string(&ev.payload)?,
                serde_json::to_string(&ev.evidence_refs)?,
                ev.superseded_event_id,
            ],
        )?;
        Ok(())
    }

    pub fn next_event_sequence(&self, investigation_id: &str) -> Result<i64> {
        let max_seq: Option<i64> = self
            .conn
            .query_row(
                "SELECT MAX(sequence) FROM investigation_events WHERE investigation_id=?1",
                [investigation_id],
                |r| r.get(0),
            )
            .unwrap_or(None);
        Ok(max_seq.unwrap_or(0) + 1)
    }

    pub fn list_investigation_events(
        &self,
        investigation_id: &str,
    ) -> Result<Vec<InvestigationEvent>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, investigation_id, sequence, occurrence_time, recording_time,
                    surface, run_id, directive_id, task_id, revision, parent_event_id,
                    role, model, provider, attempt_id, event_type, status, summary,
                    payload_json, evidence_refs_json, superseded_event_id
             FROM investigation_events WHERE investigation_id=?1 ORDER BY sequence ASC",
        )?;
        let mut rows = stmt.query([investigation_id])?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            let surface_str: String = row.get(5)?;
            let payload_str: String = row.get(18)?;
            let refs_str: String = row.get(19)?;

            out.push(InvestigationEvent {
                id: row.get(0)?,
                investigation_id: row.get(1)?,
                sequence: row.get(2)?,
                occurrence_time: row.get(3)?,
                recording_time: row.get(4)?,
                surface: InvestigationSurface::parse(&surface_str),
                run_id: row.get(6)?,
                directive_id: row.get(7)?,
                task_id: row.get(8)?,
                revision: row.get(9)?,
                parent_event_id: row.get(10)?,
                role: row.get(11)?,
                model: row.get(12)?,
                provider: row.get(13)?,
                attempt_id: row.get(14)?,
                event_type: row.get(15)?,
                status: row.get(16)?,
                summary: row.get(17)?,
                payload: serde_json::from_str(&payload_str).unwrap_or_default(),
                evidence_refs: serde_json::from_str(&refs_str).unwrap_or_default(),
                superseded_event_id: row.get(20)?,
            });
        }
        Ok(out)
    }

    pub fn insert_stream_part(
        &self,
        id: &str,
        event_id: &str,
        kind: &str,
        chunk_offset: i64,
        content: &str,
    ) -> Result<()> {
        let ts = Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO investigation_stream_parts(id, event_id, stream_kind, chunk_offset, content, created_at)
             VALUES (?1,?2,?3,?4,?5,?6)
             ON CONFLICT(id) DO NOTHING",
            params![id, event_id, kind, chunk_offset, content, ts],
        )?;
        Ok(())
    }

    pub fn list_stream_parts(&self, event_id: &str) -> Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT stream_kind, content FROM investigation_stream_parts WHERE event_id=?1 ORDER BY chunk_offset ASC",
        )?;
        let rows = stmt.query_map([event_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
        let mut out = Vec::new();
        for item in rows {
            out.push(item?);
        }
        Ok(out)
    }
}
