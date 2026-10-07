//! Investigation task, handoff, and result schemas.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Originating surface for an investigation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InvestigationSurface {
    ReconChat,
    HomeComposer,
    IntelBrief,
    JobsResume,
}

impl InvestigationSurface {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ReconChat => "recon_chat",
            Self::HomeComposer => "home_composer",
            Self::IntelBrief => "intel_brief",
            Self::JobsResume => "jobs_resume",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "home" | "home_composer" => Self::HomeComposer,
            "intel" | "intel_brief" | "intel_briefing" => Self::IntelBrief,
            "jobs" | "jobs_resume" => Self::JobsResume,
            _ => Self::ReconChat,
        }
    }
}

/// Lifecycle states of a task in the investigation harness.
/// `planned → ready → running → validating → completed | partial | unresolved | deferred | failed | cancelled`
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskStatus {
    Planned,
    Ready,
    Running,
    Validating,
    Completed,
    Partial,
    Unresolved,
    Deferred,
    Failed,
    Cancelled,
}

impl TaskStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Planned => "planned",
            Self::Ready => "ready",
            Self::Running => "running",
            Self::Validating => "validating",
            Self::Completed => "completed",
            Self::Partial => "partial",
            Self::Unresolved => "unresolved",
            Self::Deferred => "deferred",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "ready" => Self::Ready,
            "running" => Self::Running,
            "validating" => Self::Validating,
            "completed" => Self::Completed,
            "partial" => Self::Partial,
            "unresolved" => Self::Unresolved,
            "deferred" => Self::Deferred,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            _ => Self::Planned,
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed
                | Self::Partial
                | Self::Unresolved
                | Self::Deferred
                | Self::Failed
                | Self::Cancelled
        )
    }
}

/// One durable task in an investigation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskRecord {
    pub id: String,
    pub investigation_id: String,
    pub run_id: String,
    pub directive_id: String,
    pub task_id: String,
    pub revision: i64,
    pub surface: InvestigationSurface,
    pub objective: String,
    pub report_mode: String,
    pub strategy: String,
    pub bindings: Value,
    pub required_evidence: Value,
    pub freshness_cutoff: String,
    pub allowed_capabilities: Value,
    pub policy: Value,
    pub budget_ceiling: Value,
    pub output_schema: Value,
    pub completion_criteria: String,
    pub attempts: u32,
    pub max_attempts: u32,
    pub status: TaskStatus,
    pub output_ref: String,
    pub unmet_needs: Value,
    pub terminal_reason: String,
    pub superseded_revision: Option<i64>,
    pub created_at: String,
    pub updated_at: String,
}

impl TaskRecord {
    pub fn new(
        investigation_id: impl Into<String>,
        run_id: impl Into<String>,
        directive_id: impl Into<String>,
        task_id: impl Into<String>,
        surface: InvestigationSurface,
        objective: impl Into<String>,
        report_mode: impl Into<String>,
        strategy: impl Into<String>,
    ) -> Self {
        let ts = Utc::now().to_rfc3339();
        let inv_id = investigation_id.into();
        let t_id = task_id.into();
        let id = format!("{}:{}:1", inv_id, t_id);

        Self {
            id,
            investigation_id: inv_id,
            run_id: run_id.into(),
            directive_id: directive_id.into(),
            task_id: t_id,
            revision: 1,
            surface,
            objective: objective.into(),
            report_mode: report_mode.into(),
            strategy: strategy.into(),
            bindings: serde_json::json!([]),
            required_evidence: serde_json::json!([]),
            freshness_cutoff: String::new(),
            allowed_capabilities: serde_json::json!([]),
            policy: serde_json::json!({}),
            budget_ceiling: serde_json::json!({}),
            output_schema: serde_json::json!({}),
            completion_criteria: String::new(),
            attempts: 0,
            max_attempts: 3,
            status: TaskStatus::Planned,
            output_ref: String::new(),
            unmet_needs: serde_json::json!([]),
            terminal_reason: String::new(),
            superseded_revision: None,
            created_at: ts.clone(),
            updated_at: ts,
        }
    }
}

/// Structured role output handoff contract.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HandoffRecord {
    pub task_id: String,
    pub revision: i64,
    pub role: String,
    pub structured_findings: Value,
    pub evidence_references: Vec<String>,
    pub unresolved_issues: Vec<String>,
    pub decision_summary: String,
}

/// Call proposal generated by the Tool Picker or Planner.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CallProposal {
    pub task_id: String,
    pub directive_id: String,
    pub task_revision: i64,
    pub tool_id: String,
    pub contract_version: String,
    pub arguments: Value,
    pub purpose: String,
    pub expected_evidence: String,
    pub satisfaction_test: String,
}
