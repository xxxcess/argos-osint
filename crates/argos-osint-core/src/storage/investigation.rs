//! Investigation harness storage on SurrealDB: `investigation_task`,
//! `investigation_task_dep`, `investigation_passage` and
//! `investigation_assessment`, the SurrealDB side of `investigation_tasks`,
//! `investigation_task_dependencies`, `investigation_evidence_passages` and
//! `investigation_claim_assessments` (`investigation::persist`). The trace
//! itself (`investigation_events`) lives in `storage::event_log`.
//!
//! Record keys: tasks, passages and assessments by their id; dependencies
//! by `codec::encode([task_id, depends_on_task_id])`. Rewrites of an existing id update
//! only the columns SQLite's `ON CONFLICT(id) DO UPDATE` updates. An empty
//! id is rejected before any record id is built.

use anyhow::{bail, Result};
use surrealdb::engine::any::Any;
use surrealdb::types::{RecordId, SurrealValue};
use surrealdb::Surreal;

use super::codec::encode;
use super::jobs::rid;
use crate::investigation::evidence::{ClaimAssessment, ClaimStance, EvidencePassage};
use crate::investigation::{InvestigationSurface, TaskRecord, TaskStatus};

/// Investigation harness schema (idempotent). Register in bootstrap as
/// `("investigation", investigation::SCHEMA)`.
pub const SCHEMA: &str = "DEFINE TABLE IF NOT EXISTS investigation_task SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS investigation_id ON investigation_task TYPE string;
DEFINE FIELD IF NOT EXISTS run_id ON investigation_task TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS directive_id ON investigation_task TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS task_id ON investigation_task TYPE string;
DEFINE FIELD IF NOT EXISTS revision ON investigation_task TYPE int DEFAULT 1;
DEFINE FIELD IF NOT EXISTS surface ON investigation_task TYPE string;
DEFINE FIELD IF NOT EXISTS objective ON investigation_task TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS report_mode ON investigation_task TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS strategy ON investigation_task TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS bindings_json ON investigation_task TYPE string DEFAULT 'null';
DEFINE FIELD IF NOT EXISTS required_evidence_json ON investigation_task TYPE string DEFAULT 'null';
DEFINE FIELD IF NOT EXISTS freshness_cutoff ON investigation_task TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS allowed_capabilities_json ON investigation_task TYPE string DEFAULT 'null';
DEFINE FIELD IF NOT EXISTS policy_json ON investigation_task TYPE string DEFAULT 'null';
DEFINE FIELD IF NOT EXISTS budget_ceiling_json ON investigation_task TYPE string DEFAULT 'null';
DEFINE FIELD IF NOT EXISTS output_schema_json ON investigation_task TYPE string DEFAULT 'null';
DEFINE FIELD IF NOT EXISTS completion_criteria ON investigation_task TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS attempts ON investigation_task TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS max_attempts ON investigation_task TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS status ON investigation_task TYPE string;
DEFINE FIELD IF NOT EXISTS output_ref ON investigation_task TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS unmet_needs_json ON investigation_task TYPE string DEFAULT 'null';
DEFINE FIELD IF NOT EXISTS terminal_reason ON investigation_task TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS superseded_revision ON investigation_task TYPE option<int>;
DEFINE FIELD IF NOT EXISTS created_at ON investigation_task TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS updated_at ON investigation_task TYPE string DEFAULT '';
DEFINE INDEX IF NOT EXISTS investigation_task_inv ON investigation_task FIELDS investigation_id, revision, task_id;
DEFINE TABLE IF NOT EXISTS investigation_task_dep SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS task_id ON investigation_task_dep TYPE string;
DEFINE FIELD IF NOT EXISTS depends_on ON investigation_task_dep TYPE string;
DEFINE INDEX IF NOT EXISTS investigation_task_dep_task ON investigation_task_dep FIELDS task_id;
DEFINE TABLE IF NOT EXISTS investigation_passage SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS investigation_id ON investigation_passage TYPE string;
DEFINE FIELD IF NOT EXISTS task_id ON investigation_passage TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS call_id ON investigation_passage TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS source_url ON investigation_passage TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS source_domain ON investigation_passage TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS passage_text ON investigation_passage TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS observed_at ON investigation_passage TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS published_at ON investigation_passage TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS stance ON investigation_passage TYPE string;
DEFINE FIELD IF NOT EXISTS relevance_score ON investigation_passage TYPE float DEFAULT 0.0;
DEFINE FIELD IF NOT EXISTS created_at ON investigation_passage TYPE string DEFAULT '';
DEFINE INDEX IF NOT EXISTS investigation_passage_inv ON investigation_passage FIELDS investigation_id, created_at;
DEFINE TABLE IF NOT EXISTS investigation_assessment SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS investigation_id ON investigation_assessment TYPE string;
DEFINE FIELD IF NOT EXISTS claim_id ON investigation_assessment TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS evidence_id ON investigation_assessment TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS stance ON investigation_assessment TYPE string;
DEFINE FIELD IF NOT EXISTS rationale ON investigation_assessment TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS created_at ON investigation_assessment TYPE string DEFAULT '';
DEFINE INDEX IF NOT EXISTS investigation_assessment_inv ON investigation_assessment FIELDS investigation_id, created_at;";

pub async fn apply_schema(db: &Surreal<Any>) -> Result<()> {
    db.query(SCHEMA).await?.check()?;
    Ok(())
}

fn key(table: &str, id: &str) -> Result<RecordId> {
    if id.is_empty() {
        bail!("{table}: empty id");
    }
    Ok(rid(table, id))
}

fn dep_key(task_id: &str, depends_on: &str) -> Result<RecordId> {
    if task_id.is_empty() || depends_on.is_empty() {
        bail!("investigation_task_dep: empty id");
    }
    Ok(rid(
        "investigation_task_dep",
        &encode(&[task_id, depends_on]),
    ))
}

#[derive(SurrealValue, Clone)]
struct TaskContent {
    investigation_id: String,
    run_id: String,
    directive_id: String,
    task_id: String,
    revision: i64,
    surface: String,
    objective: String,
    report_mode: String,
    strategy: String,
    bindings_json: String,
    required_evidence_json: String,
    freshness_cutoff: String,
    allowed_capabilities_json: String,
    policy_json: String,
    budget_ceiling_json: String,
    output_schema_json: String,
    completion_criteria: String,
    attempts: i64,
    max_attempts: i64,
    status: String,
    output_ref: String,
    unmet_needs_json: String,
    terminal_reason: String,
    superseded_revision: Option<i64>,
    created_at: String,
    updated_at: String,
}

#[derive(SurrealValue)]
struct StoredTask {
    key: String,
    investigation_id: String,
    run_id: String,
    directive_id: String,
    task_id: String,
    revision: i64,
    surface: String,
    objective: String,
    report_mode: String,
    strategy: String,
    bindings_json: String,
    required_evidence_json: String,
    freshness_cutoff: String,
    allowed_capabilities_json: String,
    policy_json: String,
    budget_ceiling_json: String,
    output_schema_json: String,
    completion_criteria: String,
    attempts: i64,
    max_attempts: i64,
    status: String,
    output_ref: String,
    unmet_needs_json: String,
    terminal_reason: String,
    superseded_revision: Option<i64>,
    created_at: String,
    updated_at: String,
}

const TASK_COLS: &str = "record::id(id) AS key, investigation_id, run_id, directive_id,
    task_id, revision, surface, objective, report_mode, strategy, bindings_json,
    required_evidence_json, freshness_cutoff, allowed_capabilities_json, policy_json,
    budget_ceiling_json, output_schema_json, completion_criteria, attempts, max_attempts,
    status, output_ref, unmet_needs_json, terminal_reason, superseded_revision,
    created_at, updated_at";

fn json(v: &serde_json::Value) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "null".into())
}

fn parse_json(s: &str) -> serde_json::Value {
    serde_json::from_str(s).unwrap_or_default()
}

impl From<StoredTask> for TaskRecord {
    fn from(t: StoredTask) -> Self {
        TaskRecord {
            id: t.key,
            investigation_id: t.investigation_id,
            run_id: t.run_id,
            directive_id: t.directive_id,
            task_id: t.task_id,
            revision: t.revision,
            surface: InvestigationSurface::parse(&t.surface),
            objective: t.objective,
            report_mode: t.report_mode,
            strategy: t.strategy,
            bindings: parse_json(&t.bindings_json),
            required_evidence: parse_json(&t.required_evidence_json),
            freshness_cutoff: t.freshness_cutoff,
            allowed_capabilities: parse_json(&t.allowed_capabilities_json),
            policy: parse_json(&t.policy_json),
            budget_ceiling: parse_json(&t.budget_ceiling_json),
            output_schema: parse_json(&t.output_schema_json),
            completion_criteria: t.completion_criteria,
            attempts: t.attempts.max(0) as u32,
            max_attempts: t.max_attempts.max(0) as u32,
            status: TaskStatus::parse(&t.status),
            output_ref: t.output_ref,
            unmet_needs: parse_json(&t.unmet_needs_json),
            terminal_reason: t.terminal_reason,
            superseded_revision: t.superseded_revision,
            created_at: t.created_at,
            updated_at: t.updated_at,
        }
    }
}

/// `Store::insert_investigation_task`: create, or on an existing id update
/// status, attempts, output_ref, unmet_needs, terminal_reason, updated_at.
pub async fn insert_task(db: &Surreal<Any>, task: &TaskRecord) -> Result<()> {
    let id = key("investigation_task", &task.id)?;
    let row = TaskContent {
        investigation_id: task.investigation_id.clone(),
        run_id: task.run_id.clone(),
        directive_id: task.directive_id.clone(),
        task_id: task.task_id.clone(),
        revision: task.revision,
        surface: task.surface.as_str().to_string(),
        objective: task.objective.clone(),
        report_mode: task.report_mode.clone(),
        strategy: task.strategy.clone(),
        bindings_json: json(&task.bindings),
        required_evidence_json: json(&task.required_evidence),
        freshness_cutoff: task.freshness_cutoff.clone(),
        allowed_capabilities_json: json(&task.allowed_capabilities),
        policy_json: json(&task.policy),
        budget_ceiling_json: json(&task.budget_ceiling),
        output_schema_json: json(&task.output_schema),
        completion_criteria: task.completion_criteria.clone(),
        attempts: i64::from(task.attempts),
        max_attempts: i64::from(task.max_attempts),
        status: task.status.as_str().to_string(),
        output_ref: task.output_ref.clone(),
        unmet_needs_json: json(&task.unmet_needs),
        terminal_reason: task.terminal_reason.clone(),
        superseded_revision: task.superseded_revision,
        created_at: task.created_at.clone(),
        updated_at: chrono::Utc::now().to_rfc3339(),
    };
    db.query(
        "IF record::exists($t) {
           UPDATE $t SET status = $row.status, attempts = $row.attempts,
             output_ref = $row.output_ref, unmet_needs_json = $row.unmet_needs_json,
             terminal_reason = $row.terminal_reason, updated_at = $row.updated_at
             RETURN NONE;
         } ELSE {
           CREATE $t CONTENT $row RETURN NONE;
         };",
    )
    .bind(("t", id))
    .bind(("row", row))
    .await?
    .check()?;
    Ok(())
}

/// `Store::get_investigation_task`. An empty id is `None`, never a query.
pub async fn get_task(db: &Surreal<Any>, id: &str) -> Result<Option<TaskRecord>> {
    if id.is_empty() {
        return Ok(None);
    }
    let mut r = db
        .query(format!("SELECT {TASK_COLS} FROM ONLY $t"))
        .bind(("t", rid("investigation_task", id)))
        .await?
        .check()?;
    let row: Option<StoredTask> = r.take(0)?;
    Ok(row.map(Into::into))
}

/// `Store::list_investigation_tasks`: revision then task id order.
pub async fn list_tasks(db: &Surreal<Any>, investigation_id: &str) -> Result<Vec<TaskRecord>> {
    let mut r = db
        .query(format!(
            "SELECT {TASK_COLS} FROM investigation_task WHERE investigation_id = $i \
             ORDER BY revision ASC, task_id ASC"
        ))
        .bind(("i", investigation_id.to_string()))
        .await?
        .check()?;
    let rows: Vec<StoredTask> = r.take(0)?;
    Ok(rows.into_iter().map(Into::into).collect())
}

/// `Store::update_task_state`. A missing task is left alone, like SQLite.
pub async fn update_task_state(
    db: &Surreal<Any>,
    id: &str,
    status: TaskStatus,
    output_ref: &str,
    terminal_reason: &str,
) -> Result<()> {
    if id.is_empty() {
        return Ok(());
    }
    db.query(
        "IF record::exists($t) {
           UPDATE $t SET status = $s, output_ref = $o, terminal_reason = $r,
             updated_at = $now RETURN NONE;
         };",
    )
    .bind(("t", rid("investigation_task", id)))
    .bind(("s", status.as_str().to_string()))
    .bind(("o", output_ref.to_string()))
    .bind(("r", terminal_reason.to_string()))
    .bind(("now", chrono::Utc::now().to_rfc3339()))
    .await?
    .check()?;
    Ok(())
}

/// `Store::add_task_dependency` (idempotent).
pub async fn add_dependency(db: &Surreal<Any>, task_id: &str, depends_on: &str) -> Result<()> {
    db.query("UPSERT $d SET task_id = $t, depends_on = $o RETURN NONE")
        .bind(("d", dep_key(task_id, depends_on)?))
        .bind(("t", task_id.to_string()))
        .bind(("o", depends_on.to_string()))
        .await?
        .check()?;
    Ok(())
}

/// `Store::get_task_dependencies`.
pub async fn dependencies(db: &Surreal<Any>, task_id: &str) -> Result<Vec<String>> {
    let mut r = db
        .query(
            "SELECT VALUE depends_on FROM investigation_task_dep WHERE task_id = $t \
             ORDER BY depends_on",
        )
        .bind(("t", task_id.to_string()))
        .await?
        .check()?;
    Ok(r.take(0)?)
}

#[derive(SurrealValue, Clone)]
struct PassageContent {
    investigation_id: String,
    task_id: String,
    call_id: String,
    source_url: String,
    source_domain: String,
    passage_text: String,
    observed_at: String,
    published_at: String,
    stance: String,
    relevance_score: f64,
    created_at: String,
}

#[derive(SurrealValue)]
struct StoredPassage {
    key: String,
    investigation_id: String,
    task_id: String,
    call_id: String,
    source_url: String,
    source_domain: String,
    passage_text: String,
    observed_at: String,
    published_at: String,
    stance: String,
    relevance_score: f64,
    created_at: String,
}

/// `Store::insert_evidence_passage`: a rewrite updates stance and score.
pub async fn insert_passage(db: &Surreal<Any>, p: &EvidencePassage) -> Result<()> {
    let id = key("investigation_passage", &p.id)?;
    let row = PassageContent {
        investigation_id: p.investigation_id.clone(),
        task_id: p.task_id.clone(),
        call_id: p.call_id.clone(),
        source_url: p.source_url.clone(),
        source_domain: p.source_domain.clone(),
        passage_text: p.passage_text.clone(),
        observed_at: p.observed_at.clone(),
        published_at: p.published_at.clone(),
        stance: p.stance.as_str().to_string(),
        relevance_score: p.relevance_score,
        created_at: p.created_at.clone(),
    };
    db.query(
        "IF record::exists($p) {
           UPDATE $p SET stance = $row.stance, relevance_score = $row.relevance_score
             RETURN NONE;
         } ELSE {
           CREATE $p CONTENT $row RETURN NONE;
         };",
    )
    .bind(("p", id))
    .bind(("row", row))
    .await?
    .check()?;
    Ok(())
}

/// `Store::list_evidence_passages`, oldest first.
pub async fn list_passages(
    db: &Surreal<Any>,
    investigation_id: &str,
) -> Result<Vec<EvidencePassage>> {
    let mut r = db
        .query(
            "SELECT record::id(id) AS key, investigation_id, task_id, call_id, source_url,
                source_domain, passage_text, observed_at, published_at, stance,
                relevance_score, created_at
             FROM investigation_passage WHERE investigation_id = $i
             ORDER BY created_at ASC, key ASC",
        )
        .bind(("i", investigation_id.to_string()))
        .await?
        .check()?;
    let rows: Vec<StoredPassage> = r.take(0)?;
    Ok(rows
        .into_iter()
        .map(|p| EvidencePassage {
            id: p.key,
            investigation_id: p.investigation_id,
            task_id: p.task_id,
            call_id: p.call_id,
            source_url: p.source_url,
            source_domain: p.source_domain,
            passage_text: p.passage_text,
            observed_at: p.observed_at,
            published_at: p.published_at,
            stance: ClaimStance::parse(&p.stance),
            relevance_score: p.relevance_score,
            created_at: p.created_at,
        })
        .collect())
}

#[derive(SurrealValue, Clone)]
struct AssessmentContent {
    investigation_id: String,
    claim_id: String,
    evidence_id: String,
    stance: String,
    rationale: String,
    created_at: String,
}

#[derive(SurrealValue)]
struct StoredAssessment {
    key: String,
    investigation_id: String,
    claim_id: String,
    evidence_id: String,
    stance: String,
    rationale: String,
    created_at: String,
}

/// `Store::insert_claim_assessment`: a rewrite updates stance and rationale.
pub async fn insert_assessment(db: &Surreal<Any>, a: &ClaimAssessment) -> Result<()> {
    let id = key("investigation_assessment", &a.id)?;
    let row = AssessmentContent {
        investigation_id: a.investigation_id.clone(),
        claim_id: a.claim_id.clone(),
        evidence_id: a.evidence_id.clone(),
        stance: a.stance.as_str().to_string(),
        rationale: a.rationale.clone(),
        created_at: a.created_at.clone(),
    };
    db.query(
        "IF record::exists($a) {
           UPDATE $a SET stance = $row.stance, rationale = $row.rationale RETURN NONE;
         } ELSE {
           CREATE $a CONTENT $row RETURN NONE;
         };",
    )
    .bind(("a", id))
    .bind(("row", row))
    .await?
    .check()?;
    Ok(())
}

/// `Store::list_claim_assessments`, oldest first.
pub async fn list_assessments(
    db: &Surreal<Any>,
    investigation_id: &str,
) -> Result<Vec<ClaimAssessment>> {
    let mut r = db
        .query(
            "SELECT record::id(id) AS key, investigation_id, claim_id, evidence_id, stance,
                rationale, created_at
             FROM investigation_assessment WHERE investigation_id = $i
             ORDER BY created_at ASC, key ASC",
        )
        .bind(("i", investigation_id.to_string()))
        .await?
        .check()?;
    let rows: Vec<StoredAssessment> = r.take(0)?;
    Ok(rows
        .into_iter()
        .map(|a| ClaimAssessment {
            id: a.key,
            investigation_id: a.investigation_id,
            claim_id: a.claim_id,
            evidence_id: a.evidence_id,
            stance: ClaimStance::parse(&a.stance),
            rationale: a.rationale,
            created_at: a.created_at,
        })
        .collect())
}
