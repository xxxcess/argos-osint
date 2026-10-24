//! The Recon job writers on SurrealDB: model attempts and the operation
//! `task_epoch`, Recon coverage, Intel report jobs and tasks, and the job
//! registry rows of Recon investigations. SQLite parity:
//! `recon_model_attempts` → `recon_model_attempt` (keyed by the canonical
//! attempt id), `recon_model_operations.task_epoch` → a field on
//! `recon_model_operation` (the table itself is `storage::recon_exec`),
//! `recon_coverage` → `recon_coverage` (keyed by `coverage_record_id`),
//! `intel_report_jobs` / `intel_report_tasks` → `intel_report_job` /
//! `intel_report_task`, and `argos_jobs` rows of `recon_investigation` →
//! `job` rows (`storage::jobs`, shown by `storage::jobs_view`).
//!
//! [`SCHEMA`] is not in `storage::bootstrap` yet: it must be registered
//! there after `recon_exec` (it adds fields to `recon_model_operation`).
//! Until then callers and tests apply it with [`apply_schema`].

use anyhow::Result;
use serde::Serialize;
use surrealdb::engine::any::Any;
use surrealdb::types::SurrealValue;
use surrealdb::Surreal;

use super::jobs::rid;
use super::recon_runs::now;
use super::repo::Repository;
use crate::intel_recon::{IntelReportJobRow, IntelReportTaskRow};
use crate::profile_stats::{
    CoverageRow, ModelAttemptRow, ModelOperationRow, ReportJobRow, ReportTaskRow,
};
use crate::recon::diversity::{coverage_record_id, CoverageRecord};

/// Schema (idempotent). Register as `("recon_jobs", recon_jobs::SCHEMA)`
/// after `recon_exec` in `storage::bootstrap`.
pub const SCHEMA: &str = "DEFINE FIELD IF NOT EXISTS task_epoch ON recon_model_operation TYPE option<int>;
DEFINE INDEX IF NOT EXISTS recon_model_operation_task_epoch ON recon_model_operation FIELDS task_id, task_epoch;

DEFINE TABLE IF NOT EXISTS recon_model_attempt SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS operation_id ON recon_model_attempt TYPE string;
DEFINE FIELD IF NOT EXISTS generation ON recon_model_attempt TYPE int DEFAULT 1;
DEFINE FIELD IF NOT EXISTS route_index ON recon_model_attempt TYPE int;
DEFINE FIELD IF NOT EXISTS attempt ON recon_model_attempt TYPE int;
DEFINE FIELD IF NOT EXISTS provider ON recon_model_attempt TYPE string;
DEFINE FIELD IF NOT EXISTS account ON recon_model_attempt TYPE string;
DEFINE FIELD IF NOT EXISTS model ON recon_model_attempt TYPE string;
DEFINE FIELD IF NOT EXISTS transport ON recon_model_attempt TYPE string;
DEFINE FIELD IF NOT EXISTS dispatched ON recon_model_attempt TYPE bool DEFAULT true;
DEFINE FIELD IF NOT EXISTS outcome ON recon_model_attempt TYPE string;
DEFINE FIELD IF NOT EXISTS failure_category ON recon_model_attempt TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS http_status ON recon_model_attempt TYPE option<int>;
DEFINE FIELD IF NOT EXISTS request_id ON recon_model_attempt TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS finish_reason ON recon_model_attempt TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS char_count ON recon_model_attempt TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS wait_ms ON recon_model_attempt TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS error_message ON recon_model_attempt TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS queued_at ON recon_model_attempt TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS admitted_at ON recon_model_attempt TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS sent_at ON recon_model_attempt TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS first_response_at ON recon_model_attempt TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS started_at ON recon_model_attempt TYPE string;
DEFINE FIELD IF NOT EXISTS finished_at ON recon_model_attempt TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS elapsed_ms ON recon_model_attempt TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS queue_ms ON recon_model_attempt TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS ttfb_ms ON recon_model_attempt TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS prompt_tokens ON recon_model_attempt TYPE option<int>;
DEFINE FIELD IF NOT EXISTS completion_tokens ON recon_model_attempt TYPE option<int>;
DEFINE FIELD IF NOT EXISTS reasoning_tokens ON recon_model_attempt TYPE option<int>;
DEFINE FIELD IF NOT EXISTS visible_tokens ON recon_model_attempt TYPE option<int>;
DEFINE FIELD IF NOT EXISTS reported_cost_usd ON recon_model_attempt TYPE option<float>;
DEFINE FIELD IF NOT EXISTS response_headers_ms ON recon_model_attempt TYPE option<int>;
DEFINE FIELD IF NOT EXISTS first_reasoning_ms ON recon_model_attempt TYPE option<int>;
DEFINE FIELD IF NOT EXISTS first_output_ms ON recon_model_attempt TYPE option<int>;
DEFINE FIELD IF NOT EXISTS stream_completion_ms ON recon_model_attempt TYPE option<int>;
DEFINE FIELD IF NOT EXISTS stream_completed ON recon_model_attempt TYPE bool DEFAULT false;
DEFINE FIELD IF NOT EXISTS declaration_json ON recon_model_attempt TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS policy_diagnostics_json ON recon_model_attempt TYPE string DEFAULT '[]';
DEFINE INDEX IF NOT EXISTS recon_model_attempt_key ON recon_model_attempt FIELDS operation_id, generation, route_index, attempt UNIQUE;
DEFINE INDEX IF NOT EXISTS recon_model_attempt_started ON recon_model_attempt FIELDS started_at;
DEFINE EVENT IF NOT EXISTS recon_model_attempt_cascade ON TABLE recon_model_operation
  WHEN $event = 'DELETE' THEN {
    DELETE recon_model_attempt WHERE operation_id = record::id($before.id) RETURN NONE;
  };

DEFINE TABLE IF NOT EXISTS recon_coverage SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS scope ON recon_coverage TYPE string;
DEFINE FIELD IF NOT EXISTS generation ON recon_coverage TYPE int DEFAULT 1;
DEFINE FIELD IF NOT EXISTS directive_index ON recon_coverage TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS category ON recon_coverage TYPE string;
DEFINE FIELD IF NOT EXISTS payload_json ON recon_coverage TYPE string DEFAULT '{}';
DEFINE FIELD IF NOT EXISTS updated_at ON recon_coverage TYPE string;
DEFINE INDEX IF NOT EXISTS recon_coverage_scope ON recon_coverage FIELDS scope, generation;

DEFINE TABLE IF NOT EXISTS intel_report_job SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS investigation_id ON intel_report_job TYPE string;
DEFINE FIELD IF NOT EXISTS article_id ON intel_report_job TYPE string;
DEFINE FIELD IF NOT EXISTS mode ON intel_report_job TYPE string;
DEFINE FIELD IF NOT EXISTS revision ON intel_report_job TYPE int DEFAULT 1;
DEFINE FIELD IF NOT EXISTS state ON intel_report_job TYPE string;
DEFINE FIELD IF NOT EXISTS stage ON intel_report_job TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS settings_json ON intel_report_job TYPE string DEFAULT '{}';
DEFINE FIELD IF NOT EXISTS budget_json ON intel_report_job TYPE string DEFAULT '{}';
DEFINE FIELD IF NOT EXISTS parent_job_id ON intel_report_job TYPE option<string>;
DEFINE FIELD IF NOT EXISTS sections_done ON intel_report_job TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS sections_total ON intel_report_job TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS elements_done ON intel_report_job TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS elements_total ON intel_report_job TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS tool_calls_done ON intel_report_job TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS tool_calls_allowance ON intel_report_job TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS current_tool ON intel_report_job TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS warning ON intel_report_job TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS error ON intel_report_job TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS generation ON intel_report_job TYPE int DEFAULT 1;
DEFINE FIELD IF NOT EXISTS started_at ON intel_report_job TYPE string;
DEFINE FIELD IF NOT EXISTS updated_at ON intel_report_job TYPE string;
DEFINE FIELD IF NOT EXISTS finished_at ON intel_report_job TYPE string DEFAULT '';
DEFINE INDEX IF NOT EXISTS intel_report_job_article ON intel_report_job FIELDS article_id, updated_at;
DEFINE INDEX IF NOT EXISTS intel_report_job_investigation ON intel_report_job FIELDS investigation_id, mode, revision;
DEFINE EVENT IF NOT EXISTS intel_report_task_cascade ON TABLE intel_report_job
  WHEN $event = 'DELETE' THEN {
    DELETE intel_report_task WHERE job_id = record::id($before.id) RETURN NONE;
  };

DEFINE TABLE IF NOT EXISTS intel_report_task SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS job_id ON intel_report_task TYPE string;
DEFINE FIELD IF NOT EXISTS task_type ON intel_report_task TYPE string;
DEFINE FIELD IF NOT EXISTS section_key ON intel_report_task TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS depends_on_json ON intel_report_task TYPE string DEFAULT '[]';
DEFINE FIELD IF NOT EXISTS input_hash ON intel_report_task TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS status ON intel_report_task TYPE string;
DEFINE FIELD IF NOT EXISTS attempts ON intel_report_task TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS max_attempts ON intel_report_task TYPE int DEFAULT 3;
DEFINE FIELD IF NOT EXISTS output_ref ON intel_report_task TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS lease_owner ON intel_report_task TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS lease_until ON intel_report_task TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS lease_epoch ON intel_report_task TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS error ON intel_report_task TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS created_at ON intel_report_task TYPE string;
DEFINE FIELD IF NOT EXISTS updated_at ON intel_report_task TYPE string;
DEFINE INDEX IF NOT EXISTS intel_report_task_job ON intel_report_task FIELDS job_id, status;
DEFINE INDEX IF NOT EXISTS intel_report_task_lease ON intel_report_task FIELDS status, lease_until;";

/// Apply [`SCHEMA`] (idempotent). Needed until `storage::bootstrap`
/// registers it.
pub async fn apply_schema(db: &Surreal<Any>) -> Result<()> {
    db.query(SCHEMA).await?.check()?;
    Ok(())
}

fn i64_of(v: u32) -> i64 {
    i64::from(v)
}

// ---------------------------------------------------------------------------
// Model operations (task_epoch) and attempts
// ---------------------------------------------------------------------------

/// The executor's operation upsert (`model_exec::ensure_operation`): create
/// it running or move it back to running at `generation`, keeping its draft
/// unless a new one is given and its `task_epoch` unless a new one is given.
#[allow(clippy::too_many_arguments)]
pub async fn ensure_operation(
    repo: &Repository,
    id: &str,
    run_id: &str,
    task_id: &str,
    role: &str,
    generation: i64,
    draft: &str,
    task_epoch: Option<i64>,
) -> Result<()> {
    repo.db()
        .query(
            "UPSERT $o SET run_id = run_id ?? $run, task_id = task_id ?? $task,
               role = role ?? $role, generation = $g, status = 'running',
               draft = IF $draft != '' { $draft } ELSE { draft ?? '' },
               task_epoch = $epoch ?? task_epoch,
               created_at = created_at ?? $at, updated_at = $at RETURN NONE",
        )
        .bind(("o", rid("recon_model_operation", id)))
        .bind(("run", run_id.to_string()))
        .bind(("task", task_id.to_string()))
        .bind(("role", role.to_string()))
        .bind(("g", generation))
        .bind(("draft", draft.to_string()))
        .bind(("epoch", task_epoch))
        .bind(("at", now()))
        .await?
        .check()?;
    Ok(())
}

/// The operation's `task_epoch` (`None` when unset or missing).
pub async fn operation_task_epoch(repo: &Repository, id: &str) -> Result<Option<i64>> {
    let mut r = repo
        .db()
        .query("RETURN (SELECT VALUE task_epoch FROM ONLY $o)")
        .bind(("o", rid("recon_model_operation", id)))
        .await?
        .check()?;
    Ok(r.take::<Option<i64>>(0usize)?)
}

/// Operations of a `(task, epoch)` pair, oldest first (fencing reads).
pub async fn operations_for_task_epoch(
    repo: &Repository,
    task_id: &str,
    task_epoch: i64,
) -> Result<Vec<String>> {
    let mut r = repo
        .db()
        .query(
            "SELECT VALUE record::id(id) FROM recon_model_operation
             WHERE task_id = $t AND task_epoch = $e ORDER BY created_at, id",
        )
        .bind(("t", task_id.to_string()))
        .bind(("e", task_epoch))
        .await?
        .check()?;
    Ok(r.take(0usize)?)
}

/// The start of one wire attempt (`model_exec::record_attempt_start`).
#[derive(Debug, Clone)]
pub struct AttemptStart<'a> {
    pub id: &'a str,
    pub operation_id: &'a str,
    pub generation: u32,
    pub route_index: u32,
    pub attempt: u32,
    pub provider: &'a str,
    pub account: &'a str,
    pub model: &'a str,
    pub transport: &'a str,
    pub queued_at: &'a str,
    pub declaration_json: &'a str,
}

/// Insert the attempt running, or reset a replayed one to running (same
/// key, same id), like the SQLite `ON CONFLICT` upsert.
pub async fn record_attempt_start(repo: &Repository, a: &AttemptStart<'_>) -> Result<()> {
    repo.db()
        .query(
            "UPSERT $a SET operation_id = $op, generation = $g, route_index = $ri, attempt = $n,
               provider = provider ?? $provider, account = account ?? $account,
               model = model ?? $model, transport = transport ?? $transport, dispatched = true,
               outcome = 'running', queued_at = $queued, admitted_at = $at, started_at = $at,
               finished_at = '', declaration_json = $decl RETURN NONE",
        )
        .bind(("a", rid("recon_model_attempt", a.id)))
        .bind(("op", a.operation_id.to_string()))
        .bind(("g", i64_of(a.generation)))
        .bind(("ri", i64_of(a.route_index)))
        .bind(("n", i64_of(a.attempt)))
        .bind(("provider", a.provider.to_string()))
        .bind(("account", a.account.to_string()))
        .bind(("model", a.model.to_string()))
        .bind(("transport", a.transport.to_string()))
        .bind(("queued", a.queued_at.to_string()))
        .bind(("decl", a.declaration_json.to_string()))
        .bind(("at", now()))
        .await?
        .check()?;
    Ok(())
}

/// The finish of one wire attempt (`model_exec::record_attempt_finish`).
/// `None` timings stay unset.
#[derive(Debug, Clone, Default, Serialize)]
pub struct AttemptFinish {
    pub outcome: String,
    pub failure_category: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_status: Option<i64>,
    pub request_id: String,
    pub finish_reason: String,
    pub char_count: i64,
    pub wait_ms: i64,
    pub error_message: String,
    pub queued_at: String,
    pub admitted_at: String,
    pub sent_at: String,
    pub first_response_at: String,
    pub elapsed_ms: i64,
    pub queue_ms: i64,
    pub ttfb_ms: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completion_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visible_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reported_cost_usd: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_headers_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_reasoning_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_output_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_completion_ms: Option<i64>,
    pub stream_completed: bool,
    pub policy_diagnostics_json: String,
}

/// Close an attempt with its outcome and measured timings. A missing
/// attempt is left missing (SQLite `UPDATE` parity).
pub async fn record_attempt_finish(repo: &Repository, id: &str, f: &AttemptFinish) -> Result<()> {
    let mut patch = serde_json::to_value(f)?;
    if let Some(map) = patch.as_object_mut() {
        map.insert("finished_at".into(), serde_json::Value::String(now()));
    }
    repo.db()
        .query("UPDATE $a MERGE $p RETURN NONE")
        .bind(("a", rid("recon_model_attempt", id)))
        .bind(("p", patch))
        .await?
        .check()?;
    Ok(())
}

/// One stored attempt, as tests and readers see it.
#[derive(Debug, Clone, PartialEq, SurrealValue)]
pub struct Attempt {
    pub id: String,
    pub operation_id: String,
    pub generation: i64,
    pub route_index: i64,
    pub attempt: i64,
    pub provider: String,
    pub model: String,
    pub outcome: String,
    pub failure_category: String,
    pub http_status: Option<i64>,
    pub char_count: i64,
    pub elapsed_ms: i64,
    pub prompt_tokens: Option<i64>,
    pub stream_completed: bool,
    pub started_at: String,
    pub finished_at: String,
}

/// An operation's attempts in `(generation, route_index, attempt)` order.
pub async fn attempts(repo: &Repository, operation_id: &str) -> Result<Vec<Attempt>> {
    let mut r = repo
        .db()
        .query(
            "SELECT record::id(id) AS id, operation_id, generation, route_index, attempt,
               provider, model, outcome, failure_category, http_status, char_count,
               elapsed_ms, prompt_tokens, stream_completed, started_at, finished_at
             FROM recon_model_attempt WHERE operation_id = $op
             ORDER BY generation, route_index, attempt",
        )
        .bind(("op", operation_id.to_string()))
        .await?
        .check()?;
    Ok(r.take(0usize)?)
}

// ---------------------------------------------------------------------------
// Recon coverage
// ---------------------------------------------------------------------------

/// `diversity::save_coverage_record`: upsert by `(scope, generation,
/// directive, category)`.
pub async fn save_coverage_record(repo: &Repository, record: &CoverageRecord) -> Result<()> {
    repo.db()
        .query(
            "UPSERT $c SET scope = $scope, generation = $g, directive_index = $d,
               category = $cat, payload_json = $payload, updated_at = $at RETURN NONE",
        )
        .bind(("c", rid("recon_coverage", &coverage_record_id(record))))
        .bind(("scope", record.scope.clone()))
        .bind(("g", i64_of(record.generation)))
        .bind(("d", i64_of(record.directive_index)))
        .bind(("cat", record.category.clone()))
        .bind(("payload", serde_json::to_string(record)?))
        .bind(("at", now()))
        .await?
        .check()?;
    Ok(())
}

/// `diversity::load_coverage_records`: one scope and generation, by
/// directive then category; unreadable payloads are skipped.
pub async fn load_coverage_records(
    repo: &Repository,
    scope: &str,
    generation: u32,
) -> Result<Vec<CoverageRecord>> {
    let mut r = repo
        .db()
        .query(
            "SELECT VALUE payload_json FROM (SELECT payload_json, directive_index, category
               FROM recon_coverage WHERE scope = $s AND generation = $g
               ORDER BY directive_index, category)",
        )
        .bind(("s", scope.to_string()))
        .bind(("g", i64_of(generation)))
        .await?
        .check()?;
    let rows: Vec<String> = r.take(0usize)?;
    Ok(rows
        .iter()
        .filter_map(|raw| serde_json::from_str(raw).ok())
        .collect())
}

// ---------------------------------------------------------------------------
// Intel report jobs and tasks
// ---------------------------------------------------------------------------

const JOB_FIELDS: &str = "record::id(id) AS id, investigation_id, article_id, mode, revision, \
     state, stage, settings_json, budget_json, parent_job_id, sections_done, sections_total, \
     elements_done, elements_total, tool_calls_done, tool_calls_allowance, current_tool, \
     warning, error, generation, started_at, updated_at, finished_at";

const TASK_FIELDS: &str = "record::id(id) AS id, job_id, task_type, section_key, \
     depends_on_json, input_hash, status, attempts, max_attempts, output_ref, lease_owner, \
     lease_until, error, created_at, updated_at, lease_epoch";

#[derive(Debug, SurrealValue)]
struct JobRec {
    id: String,
    investigation_id: String,
    article_id: String,
    mode: String,
    revision: i64,
    state: String,
    stage: String,
    settings_json: String,
    budget_json: String,
    parent_job_id: Option<String>,
    sections_done: i64,
    sections_total: i64,
    elements_done: i64,
    elements_total: i64,
    tool_calls_done: i64,
    tool_calls_allowance: i64,
    current_tool: String,
    warning: String,
    error: String,
    generation: i64,
    started_at: String,
    updated_at: String,
    finished_at: String,
}

impl From<JobRec> for IntelReportJobRow {
    fn from(j: JobRec) -> Self {
        Self {
            id: j.id,
            investigation_id: j.investigation_id,
            article_id: j.article_id,
            mode: j.mode,
            revision: j.revision,
            state: j.state,
            stage: j.stage,
            settings_json: j.settings_json,
            budget_json: j.budget_json,
            parent_job_id: j.parent_job_id,
            sections_done: j.sections_done,
            sections_total: j.sections_total,
            elements_done: j.elements_done,
            elements_total: j.elements_total,
            tool_calls_done: j.tool_calls_done,
            tool_calls_allowance: j.tool_calls_allowance,
            current_tool: j.current_tool,
            warning: j.warning,
            error: j.error,
            generation: j.generation,
            started_at: j.started_at,
            updated_at: j.updated_at,
            finished_at: j.finished_at,
        }
    }
}

#[derive(Debug, SurrealValue)]
struct TaskRec {
    id: String,
    job_id: String,
    task_type: String,
    section_key: String,
    depends_on_json: String,
    input_hash: String,
    status: String,
    attempts: i64,
    max_attempts: i64,
    output_ref: String,
    lease_owner: String,
    lease_until: String,
    error: String,
    created_at: String,
    updated_at: String,
    lease_epoch: i64,
}

impl From<TaskRec> for IntelReportTaskRow {
    fn from(t: TaskRec) -> Self {
        Self {
            id: t.id,
            job_id: t.job_id,
            task_type: t.task_type,
            section_key: t.section_key,
            depends_on_json: t.depends_on_json,
            input_hash: t.input_hash,
            status: t.status,
            attempts: t.attempts,
            max_attempts: t.max_attempts,
            output_ref: t.output_ref,
            lease_owner: t.lease_owner,
            lease_until: t.lease_until,
            error: t.error,
            created_at: t.created_at,
            updated_at: t.updated_at,
            lease_epoch: t.lease_epoch,
        }
    }
}

/// A new Intel report job (`Store::insert_report_job`).
#[derive(Debug, Clone)]
pub struct NewReportJob<'a> {
    pub id: &'a str,
    pub investigation_id: &'a str,
    pub article_id: &'a str,
    pub mode: &'a str,
    pub revision: i64,
    pub settings_json: &'a str,
    pub budget_json: &'a str,
    pub sections_total: i64,
    pub elements_total: i64,
    pub tool_calls_allowance: i64,
    pub parent_job_id: Option<&'a str>,
}

/// Insert a queued report job in the `planning` stage and return it.
pub async fn insert_report_job(
    repo: &Repository,
    job: &NewReportJob<'_>,
) -> Result<IntelReportJobRow> {
    repo.db()
        .query(
            "CREATE $j SET investigation_id = $inv, article_id = $art, mode = $mode,
               revision = $rev, state = 'queued', stage = 'planning', settings_json = $settings,
               budget_json = $budget, parent_job_id = $parent, sections_total = $st,
               elements_total = $et, tool_calls_allowance = $tca, generation = 1,
               started_at = $at, updated_at = $at, finished_at = '' RETURN NONE",
        )
        .bind(("j", rid("intel_report_job", job.id)))
        .bind(("inv", job.investigation_id.to_string()))
        .bind(("art", job.article_id.to_string()))
        .bind(("mode", job.mode.to_string()))
        .bind(("rev", job.revision))
        .bind(("settings", job.settings_json.to_string()))
        .bind(("budget", job.budget_json.to_string()))
        .bind(("parent", job.parent_job_id.map(str::to_string)))
        .bind(("st", job.sections_total))
        .bind(("et", job.elements_total))
        .bind(("tca", job.tool_calls_allowance))
        .bind(("at", now()))
        .await?
        .check()?;
    report_job(repo, job.id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("report job {} missing after insert", job.id))
}

/// One report job (`Store::intel_report_job`).
pub async fn report_job(repo: &Repository, id: &str) -> Result<Option<IntelReportJobRow>> {
    let mut r = repo
        .db()
        .query(format!("SELECT {JOB_FIELDS} FROM ONLY $j"))
        .bind(("j", rid("intel_report_job", id)))
        .await?
        .check()?;
    let row: Option<JobRec> = r.take(0usize)?;
    Ok(row.map(Into::into))
}

/// An article's report jobs, newest first (`Store::intel_jobs_for_article`
/// without the retained-result rows).
pub async fn report_jobs_for_article(
    repo: &Repository,
    article_id: &str,
) -> Result<Vec<IntelReportJobRow>> {
    let mut r = repo
        .db()
        .query(format!(
            "SELECT {JOB_FIELDS} FROM intel_report_job WHERE article_id = $a ORDER BY updated_at DESC"
        ))
        .bind(("a", article_id.to_string()))
        .await?
        .check()?;
    let rows: Vec<JobRec> = r.take(0usize)?;
    Ok(rows.into_iter().map(Into::into).collect())
}

/// Next report revision for `(article, mode)` (`Store::next_job_revision`).
pub async fn next_job_revision(repo: &Repository, article_id: &str, mode: &str) -> Result<i64> {
    let mut r = repo
        .db()
        .query(
            "SELECT VALUE revision FROM intel_report_job
               WHERE article_id = $a AND mode = $m ORDER BY revision DESC LIMIT 1",
        )
        .bind(("a", article_id.to_string()))
        .bind(("m", mode.to_string()))
        .await?
        .check()?;
    let top: Vec<i64> = r.take(0usize)?;
    Ok(top.first().map_or(1, |rev| rev + 1))
}

fn report_terminal(state: &str) -> bool {
    matches!(state, "completed" | "partial" | "failed" | "cancelled")
}

/// `Store::update_report_job`: state, stage and progress; a terminal state
/// stamps `finished_at`, any other keeps it. Retired jobs are left alone.
#[allow(clippy::too_many_arguments)]
pub async fn update_report_job(
    repo: &Repository,
    id: &str,
    state: &str,
    stage: &str,
    sections_done: i64,
    elements_done: i64,
    warning: &str,
    error: &str,
) -> Result<()> {
    let at = now();
    let finished = if report_terminal(state) {
        at.clone()
    } else {
        String::new()
    };
    repo.db()
        .query(
            "IF !record::exists(type::record('job_retirement', $key)) {
               UPDATE $j SET state = $s, stage = $stage, sections_done = $sd,
                 elements_done = $ed, warning = $w, error = $e, updated_at = $at,
                 finished_at = IF $fin != '' { $fin } ELSE { finished_at } RETURN NONE;
             };",
        )
        .bind(("key", id.to_string()))
        .bind(("j", rid("intel_report_job", id)))
        .bind(("s", state.to_string()))
        .bind(("stage", stage.to_string()))
        .bind(("sd", sections_done))
        .bind(("ed", elements_done))
        .bind(("w", warning.to_string()))
        .bind(("e", error.to_string()))
        .bind(("at", at))
        .bind(("fin", finished))
        .await?
        .check()?;
    Ok(())
}

/// `Store::insert_report_task`.
pub async fn insert_report_task(
    repo: &Repository,
    id: &str,
    job_id: &str,
    task_type: &str,
    section_key: &str,
    depends_on_json: &str,
    status: &str,
) -> Result<()> {
    repo.db()
        .query(
            "CREATE $t SET job_id = $job, task_type = $tt, section_key = $sk,
               depends_on_json = $deps, status = $s, created_at = $at, updated_at = $at
               RETURN NONE",
        )
        .bind(("t", rid("intel_report_task", id)))
        .bind(("job", job_id.to_string()))
        .bind(("tt", task_type.to_string()))
        .bind(("sk", section_key.to_string()))
        .bind(("deps", depends_on_json.to_string()))
        .bind(("s", status.to_string()))
        .bind(("at", now()))
        .await?
        .check()?;
    Ok(())
}

/// A job's tasks in creation order (`Store::intel_report_tasks`).
pub async fn report_tasks(repo: &Repository, job_id: &str) -> Result<Vec<IntelReportTaskRow>> {
    let mut r = repo
        .db()
        .query(format!(
            "SELECT {TASK_FIELDS} FROM intel_report_task WHERE job_id = $j ORDER BY created_at, id"
        ))
        .bind(("j", job_id.to_string()))
        .await?
        .check()?;
    let rows: Vec<TaskRec> = r.take(0usize)?;
    Ok(rows.into_iter().map(Into::into).collect())
}

/// One task (`Store::intel_report_task`).
pub async fn report_task(repo: &Repository, id: &str) -> Result<Option<IntelReportTaskRow>> {
    let mut r = repo
        .db()
        .query(format!("SELECT {TASK_FIELDS} FROM ONLY $t"))
        .bind(("t", rid("intel_report_task", id)))
        .await?
        .check()?;
    let row: Option<TaskRec> = r.take(0usize)?;
    Ok(row.map(Into::into))
}

/// `Store::claim_report_task`: the first pending, interrupted or
/// lease-expired task whose dependencies completed, leased to `owner` for
/// `lease_secs` (epoch and attempts + 1). The job must be live, inside the
/// 72-hour window and not retired. One transaction.
pub async fn claim_report_task(
    repo: &Repository,
    job_id: &str,
    owner: &str,
    lease_secs: i64,
) -> Result<Option<IntelReportTaskRow>> {
    let now_ts = chrono::Utc::now();
    let now_s = now_ts.to_rfc3339();
    let cutoff = (now_ts - chrono::Duration::days(3)).to_rfc3339();
    let mut r = repo
        .db()
        .query(
            "LET $job = (SELECT state, started_at FROM ONLY $j);
             RETURN $job != NONE
               AND $job.state NOTINSIDE ['cancelled', 'paused', 'failed', 'completed']
               AND $job.started_at >= $cutoff
               AND !record::exists(type::record('job_retirement', $key));",
        )
        .bind(("j", rid("intel_report_job", job_id)))
        .bind(("key", job_id.to_string()))
        .bind(("cutoff", cutoff))
        .await?
        .check()?;
    let eligible: Option<bool> = r.take(1usize)?;
    if eligible != Some(true) {
        return Ok(None);
    }
    let tasks = report_tasks(repo, job_id).await?;
    let completed: std::collections::HashSet<&str> = tasks
        .iter()
        .filter(|t| t.status == "completed")
        .map(|t| t.id.as_str())
        .collect();
    let until = (now_ts + chrono::Duration::seconds(lease_secs)).to_rfc3339();
    for task in &tasks {
        let open = match task.status.as_str() {
            "pending" | "interrupted" => true,
            "running" => chrono::DateTime::parse_from_rfc3339(&task.lease_until)
                .is_ok_and(|until| until <= now_ts),
            _ => false,
        };
        if !open {
            continue;
        }
        let deps: Vec<String> = serde_json::from_str(&task.depends_on_json).unwrap_or_default();
        if !deps.iter().all(|d| completed.contains(d.as_str())) {
            continue;
        }
        // Guarded like the SQLite UPDATE: a concurrent claim wins once.
        let mut u = repo
            .db()
            .query(
                "UPDATE $t SET status = 'running', lease_owner = $owner, lease_until = $until,
                   lease_epoch += 1, attempts += 1, updated_at = $now
                 WHERE status IN ['pending', 'interrupted', 'running']
                   AND (lease_until = '' OR lease_until < $now) RETURN VALUE id",
            )
            .bind(("t", rid("intel_report_task", &task.id)))
            .bind(("owner", owner.to_string()))
            .bind(("until", until.clone()))
            .bind(("now", now_s.clone()))
            .await?
            .check()?;
        let ids: Vec<surrealdb::types::RecordId> = u.take(0usize)?;
        if !ids.is_empty() {
            return report_task(repo, &task.id).await;
        }
    }
    Ok(None)
}

/// `Store::complete_report_task`.
pub async fn complete_report_task(repo: &Repository, id: &str, output_ref: &str) -> Result<()> {
    repo.db()
        .query(
            "UPDATE $t SET status = 'completed', output_ref = $o, lease_owner = '',
               lease_until = '', error = '', updated_at = $at RETURN NONE",
        )
        .bind(("t", rid("intel_report_task", id)))
        .bind(("o", output_ref.to_string()))
        .bind(("at", now()))
        .await?
        .check()?;
    Ok(())
}

/// `Store::fail_report_task`: a retryable failure goes back to `pending`.
pub async fn fail_report_task(
    repo: &Repository,
    id: &str,
    error: &str,
    retryable: bool,
) -> Result<()> {
    repo.db()
        .query(
            "UPDATE $t SET status = $s, error = $e, lease_owner = '', lease_until = '',
               updated_at = $at RETURN NONE",
        )
        .bind(("t", rid("intel_report_task", id)))
        .bind(("s", if retryable { "pending" } else { "failed" }))
        .bind(("e", error.to_string()))
        .bind(("at", now()))
        .await?
        .check()?;
    Ok(())
}

/// `Store::interrupt_expired_leases` for report tasks.
pub async fn interrupt_expired_report_leases(repo: &Repository) -> Result<usize> {
    let mut r = repo
        .db()
        .query(
            "UPDATE intel_report_task SET status = 'interrupted', lease_owner = '', updated_at = $at
             WHERE status = 'running' AND lease_until != '' AND lease_until < $at RETURN VALUE id",
        )
        .bind(("at", now()))
        .await?
        .check()?;
    let ids: Vec<surrealdb::types::RecordId> = r.take(0usize)?;
    Ok(ids.len())
}

// ---------------------------------------------------------------------------
// Job registry rows of Recon investigations
// ---------------------------------------------------------------------------

/// Operation of the registry row of one Recon question.
pub const RECON_OPERATION: &str = "recon_investigation";

/// A Recon registry row to begin or resume (`job_registry::JobSpec`).
#[derive(Debug, Clone)]
pub struct ReconJob<'a> {
    /// Reuse this row (a resume); `None` creates a new one.
    pub id: Option<&'a str>,
    pub title: &'a str,
    pub run_ref: &'a str,
    pub resource_ref: &'a str,
}

/// `JobHandle::begin_with` for a Recon question: create the `job` row
/// running (kind `inproc`, app `recon`, cancellable) with one attempt, or
/// move an existing row back to running with attempts + 1 and its error
/// cleared. Returns the job id.
pub async fn begin_recon_job(repo: &Repository, spec: &ReconJob<'_>) -> Result<String> {
    let id = spec
        .id
        .map(str::to_string)
        .unwrap_or_else(|| crate::job_registry::new_job_id(RECON_OPERATION));
    repo.db()
        .query(
            "IF record::exists($j) {
               UPDATE $j SET state = 'running', finished_at = NONE, cancel_requested = false,
                 error_category = '', error_summary = '', attempts_used += 1,
                 heartbeat_at = <datetime>$at, active_since = <datetime>$at,
                 updated_at = <datetime>$at,
                 run_ref = IF $run != '' { $run } ELSE { run_ref } RETURN NONE;
             } ELSE {
               CREATE $j SET kind = 'inproc', operation = $op, root = $j, state = 'running',
                 app = 'recon', title = $title, run_ref = $run, resource_ref = $res,
                 correlation_id = $key, cancellable = true, attempts_used = 1, attempt_cap = 1,
                 worker_owner = $owner, heartbeat_at = <datetime>$at,
                 active_since = <datetime>$at, created_at = <datetime>$at,
                 updated_at = <datetime>$at RETURN NONE;
             };",
        )
        .bind(("j", rid("job", &id)))
        .bind(("key", id.clone()))
        .bind(("op", RECON_OPERATION))
        .bind(("title", spec.title.to_string()))
        .bind(("run", spec.run_ref.to_string()))
        .bind(("res", spec.resource_ref.to_string()))
        .bind(("owner", crate::scheduler::process_owner()))
        .bind(("at", now()))
        .await?
        .check()?;
    Ok(id)
}

/// `JobHandle::phase`: phase and heartbeat. Returns whether Jobs → Cancel
/// asked the job to stop, so the caller can raise its cancel flag.
pub async fn set_recon_phase(repo: &Repository, id: &str, phase: &str) -> Result<bool> {
    let mut r = repo
        .db()
        .query(
            "UPDATE $j SET phase = $p, heartbeat_at = <datetime>$at, updated_at = <datetime>$at
             RETURN VALUE cancel_requested",
        )
        .bind(("j", rid("job", id)))
        .bind(("p", phase.to_string()))
        .bind(("at", now()))
        .await?
        .check()?;
    let flags: Vec<bool> = r.take(0usize)?;
    Ok(flags.first().copied().unwrap_or(false))
}

/// `JobHandle::finish`: `state` is a `job_registry::Finish::state` value;
/// terminal states stamp `finished_at`, `paused`/`waiting`/`blocked` do not.
#[allow(clippy::too_many_arguments)]
pub async fn finish_recon_job(
    repo: &Repository,
    id: &str,
    state: &str,
    run_ref: Option<&str>,
    error_category: &str,
    error_summary: &str,
    result_ref: &str,
    active_ms: i64,
) -> Result<()> {
    let terminal = !matches!(state, "paused" | "waiting" | "blocked");
    let summary: String = crate::events::redact(error_summary)
        .chars()
        .take(500)
        .collect();
    repo.db()
        .query(
            "UPDATE $j SET state = $s, updated_at = <datetime>$at, heartbeat_at = <datetime>$at,
               active_since = NONE, active_ms = $ms,
               finished_at = IF $terminal { <datetime>$at } ELSE { NONE },
               error_category = $cat, error_summary = $sum,
               result_ref = IF $res != '' { $res } ELSE { result_ref },
               run_ref = IF $run != '' { $run } ELSE { run_ref } RETURN NONE",
        )
        .bind(("j", rid("job", id)))
        .bind(("s", state.to_string()))
        .bind(("ms", active_ms))
        .bind(("terminal", terminal))
        .bind(("cat", error_category.to_string()))
        .bind(("sum", summary))
        .bind(("res", result_ref.to_string()))
        .bind(("run", run_ref.unwrap_or("").to_string()))
        .bind(("at", now()))
        .await?
        .check()?;
    Ok(())
}

/// A registry row of another app (Atlas cycle and phase jobs).
#[derive(Debug, Clone)]
pub struct AppJob<'a> {
    pub id: &'a str,
    pub operation: &'a str,
    pub app: &'a str,
    pub title: &'a str,
    pub run_ref: &'a str,
    pub resource_ref: &'a str,
    /// Parent job id for a phase child; `None` for a top-level job.
    pub parent: Option<&'a str>,
}

/// [`begin_recon_job`] for any app: create the row running, or move an
/// existing one back to running with attempts + 1.
pub async fn begin_app_job(repo: &Repository, spec: &AppJob<'_>) -> Result<()> {
    if spec.id.is_empty() {
        anyhow::bail!("job: empty id");
    }
    let parent = spec.parent.filter(|p| !p.is_empty()).map(|p| rid("job", p));
    repo.db()
        .query(
            "IF record::exists($j) {
               UPDATE $j SET state = 'running', finished_at = NONE, cancel_requested = false,
                 error_category = '', error_summary = '', attempts_used += 1,
                 heartbeat_at = <datetime>$at, active_since = <datetime>$at,
                 updated_at = <datetime>$at RETURN NONE;
             } ELSE {
               CREATE $j SET kind = 'inproc', operation = $op, parent = $parent,
                 ancestors = IF $parent != NONE { [$parent] } ELSE { [] },
                 root = IF $parent != NONE { $parent.root ?? $parent } ELSE { $j },
                 state = 'running', app = $app, title = $title, run_ref = $run,
                 resource_ref = $res, correlation_id = $key, cancellable = true,
                 attempts_used = 1, attempt_cap = 1, worker_owner = $owner,
                 heartbeat_at = <datetime>$at, active_since = <datetime>$at,
                 created_at = <datetime>$at, updated_at = <datetime>$at RETURN NONE;
             };",
        )
        .bind(("j", rid("job", spec.id)))
        .bind(("key", spec.id.to_string()))
        .bind(("op", spec.operation.to_string()))
        .bind(("app", spec.app.to_string()))
        .bind(("parent", parent))
        .bind(("title", spec.title.to_string()))
        .bind(("run", spec.run_ref.to_string()))
        .bind(("res", spec.resource_ref.to_string()))
        .bind(("owner", crate::scheduler::process_owner()))
        .bind(("at", now()))
        .await?
        .check()?;
    Ok(())
}

/// `JobHandle::phase` with progress counts. Returns whether Jobs → Cancel
/// asked the job (or its root) to stop.
pub async fn set_job_progress(
    repo: &Repository,
    id: &str,
    phase: &str,
    done: Option<i64>,
    total: Option<i64>,
) -> Result<bool> {
    if id.is_empty() {
        return Ok(false);
    }
    let mut r = repo
        .db()
        .query(
            "UPDATE $j SET phase = $p, progress_done = $done, progress_total = $total,
               heartbeat_at = <datetime>$at, updated_at = <datetime>$at
             RETURN VALUE cancel_requested",
        )
        .bind(("j", rid("job", id)))
        .bind(("p", phase.to_string()))
        .bind(("done", done))
        .bind(("total", total))
        .bind(("at", now()))
        .await?
        .check()?;
    let flags: Vec<bool> = r.take(0usize)?;
    Ok(flags.first().copied().unwrap_or(false))
}

/// `job_registry::job_for_run`: the oldest top-level registry row of
/// `operation` linked to `run_ref`, so a resume reuses one job.
pub async fn job_for_run(
    repo: &Repository,
    operation: &str,
    run_ref: &str,
) -> Result<Option<String>> {
    let mut r = repo
        .db()
        .query(
            "SELECT VALUE record::id(id) FROM job WHERE operation = $op AND run_ref = $run
               AND parent = NONE ORDER BY created_at LIMIT 1",
        )
        .bind(("op", operation.to_string()))
        .bind(("run", run_ref.to_string()))
        .await?
        .check()?;
    let ids: Vec<String> = r.take(0usize)?;
    Ok(ids.into_iter().next())
}

// ---------------------------------------------------------------------------
// Profile read model
// ---------------------------------------------------------------------------

/// The Profile rows these writers own, loaded for
/// [`ProfileTables`](crate::profile_stats::ProfileTables).
#[derive(Debug, Clone, Default)]
pub struct ProfileRows {
    pub recon_run_states: Vec<String>,
    pub report_jobs: Vec<ReportJobRow>,
    pub report_tasks: Vec<ReportTaskRow>,
    pub recon_coverage: Vec<CoverageRow>,
    pub model_operations: Vec<ModelOperationRow>,
    pub model_attempts: Vec<ModelAttemptRow>,
}

#[derive(Debug, SurrealValue)]
struct PJob {
    id: String,
    article_id: String,
    state: String,
    stage: String,
    updated_at: String,
}

#[derive(Debug, SurrealValue)]
struct PTask {
    id: String,
    job_id: String,
    status: String,
    lease_until: String,
}

#[derive(Debug, SurrealValue)]
struct PCoverage {
    scope: String,
    generation: i64,
    category: String,
    payload_json: String,
    updated_at: String,
}

#[derive(Debug, SurrealValue)]
struct POp {
    id: String,
    role: String,
    status: String,
    created_at: String,
    updated_at: String,
}

#[derive(Debug, SurrealValue)]
struct PAttempt {
    id: String,
    operation_id: String,
    provider: String,
    model: String,
    route_index: i64,
    dispatched: bool,
    outcome: String,
    failure_category: String,
    http_status: Option<i64>,
    elapsed_ms: i64,
    queue_ms: i64,
    ttfb_ms: i64,
    started_at: String,
    finished_at: String,
    sent_at: String,
    queued_at: String,
    first_response_at: String,
    transport: String,
}

/// Load the Profile rows (`SqliteSource` parity: attempts without a known
/// operation are dropped, and each attempt takes its operation's role).
pub async fn profile_rows(db: &Surreal<Any>) -> Result<ProfileRows> {
    let mut r = db
        .query(
            "SELECT VALUE state FROM recon_run;
             SELECT record::id(id) AS id, article_id, state, stage, updated_at FROM intel_report_job;
             SELECT record::id(id) AS id, job_id, status, lease_until FROM intel_report_task;
             SELECT scope, generation, category, payload_json, updated_at FROM recon_coverage;
             SELECT record::id(id) AS id, role, status, created_at, updated_at FROM recon_model_operation;
             SELECT record::id(id) AS id, operation_id, provider, model, route_index, dispatched,
               outcome, failure_category, http_status, elapsed_ms, queue_ms, ttfb_ms, started_at,
               finished_at, sent_at, queued_at, first_response_at, transport
               FROM recon_model_attempt ORDER BY started_at, id;",
        )
        .await?
        .check()?;
    let states: Vec<String> = r.take(0usize)?;
    let jobs: Vec<PJob> = r.take(1usize)?;
    let tasks: Vec<PTask> = r.take(2usize)?;
    let coverage: Vec<PCoverage> = r.take(3usize)?;
    let ops: Vec<POp> = r.take(4usize)?;
    let attempts: Vec<PAttempt> = r.take(5usize)?;
    let roles: std::collections::HashMap<&str, &str> = ops
        .iter()
        .map(|o| (o.id.as_str(), o.role.as_str()))
        .collect();
    let model_attempts = attempts
        .iter()
        .filter_map(|a| {
            let role = roles.get(a.operation_id.as_str())?;
            Some(ModelAttemptRow {
                id: a.id.clone(),
                operation_id: a.operation_id.clone(),
                provider: a.provider.clone(),
                model: a.model.clone(),
                route_index: u32::try_from(a.route_index).unwrap_or(0),
                dispatched: a.dispatched,
                outcome: a.outcome.clone(),
                failure_category: a.failure_category.clone(),
                http_status: a.http_status.and_then(|s| u32::try_from(s).ok()),
                elapsed_ms: Some(a.elapsed_ms),
                queue_ms: Some(a.queue_ms),
                ttfb_ms: Some(a.ttfb_ms),
                started_at: a.started_at.clone(),
                finished_at: a.finished_at.clone(),
                sent_at: a.sent_at.clone(),
                queued_at: a.queued_at.clone(),
                first_response_at: a.first_response_at.clone(),
                role: (*role).to_string(),
                transport: a.transport.clone(),
            })
        })
        .collect();
    Ok(ProfileRows {
        recon_run_states: states,
        report_jobs: jobs
            .into_iter()
            .map(|j| ReportJobRow {
                id: j.id,
                article_id: j.article_id,
                state: j.state,
                stage: j.stage,
                updated_at: j.updated_at,
            })
            .collect(),
        report_tasks: tasks
            .into_iter()
            .map(|t| ReportTaskRow {
                id: t.id,
                job_id: t.job_id,
                article_id: String::new(),
                status: t.status,
                lease_until: t.lease_until,
            })
            .collect(),
        recon_coverage: coverage
            .into_iter()
            .map(|c| CoverageRow {
                scope: c.scope,
                generation: c.generation,
                category: c.category,
                payload_json: c.payload_json,
                updated_at: c.updated_at,
            })
            .collect(),
        model_operations: ops
            .into_iter()
            .map(|o| ModelOperationRow {
                id: o.id,
                role: o.role,
                status: o.status,
                created_at: o.created_at,
                updated_at: o.updated_at,
            })
            .collect(),
        model_attempts,
    })
}
