//! Intel storage on SurrealDB (P3): `intel_investigation`,
//! `intel_link_explanation`, `intel_report_result`, `article_refresh_intent`
//! and `article_refresh_retirement`, the SurrealDB side of the legacy store tables
//! of the same names (plural).
//!
//! Record keys: `intel_investigation` by id (unique `article_id` index),
//! `intel_link_explanation` by `[article, left, right]`,
//! `intel_report_result` by `[job_id, generation]` (unique
//! `[article_id, completion_sequence]`), `article_refresh_intent` by article,
//! `article_refresh_retirement` by `[article, input_key]`.

use std::collections::HashMap;

use surrealdb::engine::any::Any;
use surrealdb::types::SurrealValue;
use surrealdb::Surreal;

use super::codec::encode;
use super::jobs::{run, take};
use super::tx::{run_tx, RetryPolicy, TxError};
use crate::intel_recon::IntelInvestigationRow;

/// Intel schema (idempotent).
pub const SCHEMA: &str = "DEFINE TABLE IF NOT EXISTS intel_investigation SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS article_id ON intel_investigation TYPE string;
DEFINE FIELD IF NOT EXISTS run_id ON intel_investigation TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS article_url ON intel_investigation TYPE string;
DEFINE FIELD IF NOT EXISTS thread_id ON intel_investigation TYPE option<string>;
DEFINE FIELD IF NOT EXISTS scope_json ON intel_investigation TYPE string DEFAULT '{}';
DEFINE FIELD IF NOT EXISTS shared_assessment_json ON intel_investigation TYPE string DEFAULT '{}';
DEFINE FIELD IF NOT EXISTS shared_assessment_version ON intel_investigation TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS created_at ON intel_investigation TYPE string;
DEFINE FIELD IF NOT EXISTS updated_at ON intel_investigation TYPE string;
DEFINE INDEX IF NOT EXISTS intel_investigation_article ON intel_investigation FIELDS article_id UNIQUE;
DEFINE TABLE IF NOT EXISTS intel_link_explanation SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS article_id ON intel_link_explanation TYPE string;
DEFINE FIELD IF NOT EXISTS left_id ON intel_link_explanation TYPE string;
DEFINE FIELD IF NOT EXISTS right_id ON intel_link_explanation TYPE string;
DEFINE FIELD IF NOT EXISTS explanation ON intel_link_explanation TYPE string;
DEFINE FIELD IF NOT EXISTS updated_at ON intel_link_explanation TYPE string;
DEFINE INDEX IF NOT EXISTS intel_link_explanation_article ON intel_link_explanation FIELDS article_id;
DEFINE TABLE IF NOT EXISTS intel_report_result SCHEMAFULL
  PERMISSIONS FOR update NONE;
DEFINE FIELD IF NOT EXISTS job_id ON intel_report_result TYPE string READONLY;
DEFINE FIELD IF NOT EXISTS generation ON intel_report_result TYPE int READONLY;
DEFINE FIELD IF NOT EXISTS article_id ON intel_report_result TYPE string READONLY;
DEFINE FIELD IF NOT EXISTS completion_sequence ON intel_report_result TYPE int READONLY;
DEFINE FIELD IF NOT EXISTS source_hash ON intel_report_result TYPE string READONLY;
DEFINE FIELD IF NOT EXISTS details_json ON intel_report_result TYPE string READONLY;
DEFINE FIELD IF NOT EXISTS completed_at ON intel_report_result TYPE string READONLY;
DEFINE INDEX IF NOT EXISTS intel_report_result_sequence ON intel_report_result
  FIELDS article_id, completion_sequence UNIQUE;
DEFINE INDEX IF NOT EXISTS intel_report_result_job ON intel_report_result FIELDS job_id;
DEFINE TABLE IF NOT EXISTS article_refresh_intent SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS requested_at ON article_refresh_intent TYPE string;
DEFINE TABLE IF NOT EXISTS article_refresh_retirement SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS article_id ON article_refresh_retirement TYPE string;
DEFINE FIELD IF NOT EXISTS input_key ON article_refresh_retirement TYPE string;
DEFINE FIELD IF NOT EXISTS retired_at ON article_refresh_retirement TYPE string;
DEFINE INDEX IF NOT EXISTS article_refresh_retirement_article ON article_refresh_retirement FIELDS article_id;";

/// Investigation by article; pinned to `intel_investigation_article`.
pub const INVESTIGATION_BY_ARTICLE_SURQL: &str =
    "SELECT *, record::id(id) AS id FROM intel_investigation WHERE article_id = $article";
/// An article's explanations; pinned to `intel_link_explanation_article`.
pub const EXPLANATIONS_SURQL: &str =
    "SELECT left_id, right_id, explanation FROM intel_link_explanation WHERE article_id = $article";
/// An article's newest positive result; pinned to `intel_report_result_sequence`.
pub const LATEST_RESULT_SURQL: &str = "SELECT job_id, source_hash, details_json, \
     completion_sequence FROM intel_report_result WHERE article_id = $article \
     AND completion_sequence > 0 ORDER BY completion_sequence DESC LIMIT 1";

pub async fn apply_schema(db: &Surreal<Any>) -> Result<(), TxError> {
    db.query(SCHEMA).await?.check()?;
    Ok(())
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// Retries for the read-then-create sites (a unique-index race retries).
const POLICY: RetryPolicy = super::counters::HOT_RECORD_POLICY;

#[derive(SurrealValue)]
struct InvRow {
    id: String,
    article_id: String,
    run_id: String,
    article_url: String,
    thread_id: Option<String>,
    scope_json: String,
    shared_assessment_json: String,
    shared_assessment_version: i64,
    created_at: String,
    updated_at: String,
}

impl From<InvRow> for IntelInvestigationRow {
    fn from(r: InvRow) -> Self {
        IntelInvestigationRow {
            id: r.id,
            article_id: r.article_id,
            run_id: r.run_id,
            article_url: r.article_url,
            thread_id: r.thread_id,
            scope_json: r.scope_json,
            shared_assessment_json: r.shared_assessment_json,
            shared_assessment_version: r.shared_assessment_version,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

fn new_row_id(prefix: &str) -> String {
    format!(
        "{prefix}-{}",
        crate::store::new_id().trim_start_matches("mem-")
    )
}

/// `ensure_intel_investigation`: the article's investigation, created once.
/// Two racing creators converge on one row (unique article index + retry).
pub async fn ensure_investigation(
    db: &Surreal<Any>,
    article_id: &str,
    run_id: &str,
    article_url: &str,
    scope_json: &str,
) -> Result<IntelInvestigationRow, TxError> {
    let id = new_row_id("iinv");
    let c = run_tx(db, POLICY, |tx| {
        let (id, article, runid, url, scope) = (
            id.clone(),
            article_id.to_string(),
            run_id.to_string(),
            article_url.to_string(),
            scope_json.to_string(),
        );
        async move {
            let mut r = run(
                &tx,
                tx.query(INVESTIGATION_BY_ARTICLE_SURQL)
                    .bind(("article", article.clone())),
            )
            .await?;
            let found: Vec<InvRow> = take(&mut r, 0)?;
            if let Some(row) = found.into_iter().next() {
                return Ok(row);
            }
            let ts = now();
            let mut r = run(
                &tx,
                tx.query(
                    "CREATE type::record('intel_investigation', $id) SET article_id = $article, \
                     run_id = $run, article_url = $url, thread_id = NONE, scope_json = $scope, \
                     shared_assessment_json = '{}', shared_assessment_version = 0, \
                     created_at = $ts, updated_at = $ts; \
                     SELECT *, record::id(id) AS id FROM ONLY type::record('intel_investigation', $id);",
                )
                .bind(("id", id))
                .bind(("article", article))
                .bind(("run", runid))
                .bind(("url", url))
                .bind(("scope", scope))
                .bind(("ts", ts)),
            )
            .await?;
            take::<Option<InvRow>>(&mut r, 1)?
                .ok_or_else(|| TxError::Abort("investigation not created".into()))
        }
    })
    .await?;
    Ok(c.value.into())
}

pub async fn investigation_for_article(
    db: &Surreal<Any>,
    article_id: &str,
) -> Result<Option<IntelInvestigationRow>, TxError> {
    let mut r = db
        .query(INVESTIGATION_BY_ARTICLE_SURQL)
        .bind(("article", article_id.to_string()))
        .await?
        .check()?;
    let rows: Vec<InvRow> = r.take(0)?;
    Ok(rows.into_iter().next().map(Into::into))
}

pub async fn investigation_by_id(
    db: &Surreal<Any>,
    id: &str,
) -> Result<Option<IntelInvestigationRow>, TxError> {
    let mut r = db
        .query("SELECT *, record::id(id) AS id FROM ONLY type::record('intel_investigation', $id)")
        .bind(("id", id.to_string()))
        .await?
        .check()?;
    Ok(r.take::<Option<InvRow>>(0)?.map(Into::into))
}

pub async fn set_investigation_thread(
    db: &Surreal<Any>,
    id: &str,
    thread_id: &str,
) -> Result<(), TxError> {
    db.query(
        "UPDATE type::record('intel_investigation', $id) SET thread_id = $t, updated_at = $ts RETURN NONE",
    )
    .bind(("id", id.to_string()))
    .bind(("t", thread_id.to_string()))
    .bind(("ts", now()))
    .await?
    .check()?;
    Ok(())
}

/// `update_shared_assessment`: one atomic increment, returning the version.
pub async fn update_shared_assessment(
    db: &Surreal<Any>,
    id: &str,
    assessment_json: &str,
) -> Result<i64, TxError> {
    let mut r = db
        .query(
            "UPDATE ONLY type::record('intel_investigation', $id) SET shared_assessment_json = $a, \
             shared_assessment_version += 1, updated_at = $ts RETURN VALUE shared_assessment_version",
        )
        .bind(("id", id.to_string()))
        .bind(("a", assessment_json.to_string()))
        .bind(("ts", now()))
        .await?
        .check()?;
    r.take::<Option<i64>>(0)?
        .ok_or_else(|| TxError::Abort(format!("investigation {id} not found")))
}

/// Distinct articles with an investigation.
pub async fn investigated_articles(db: &Surreal<Any>) -> Result<Vec<String>, TxError> {
    let mut r = db
        .query("SELECT VALUE article_id FROM intel_investigation")
        .await?
        .check()?;
    Ok(r.take(0)?)
}

/// `save_intel_link_explanations`: upserts in one statement batch.
pub async fn save_link_explanations(
    db: &Surreal<Any>,
    article_id: &str,
    explanations: &HashMap<(String, String), String>,
) -> Result<(), TxError> {
    if explanations.is_empty() {
        return Ok(());
    }
    let rows: Vec<(String, String, String, String)> = explanations
        .iter()
        .map(|((l, r), e)| (encode(&[article_id, l, r]), l.clone(), r.clone(), e.clone()))
        .collect();
    db.query(
        "FOR $e IN $rows {
           UPSERT type::record('intel_link_explanation', $e[0]) SET article_id = $article,
             left_id = $e[1], right_id = $e[2], explanation = $e[3], updated_at = $ts RETURN NONE;
         };",
    )
    .bind(("rows", rows))
    .bind(("article", article_id.to_string()))
    .bind(("ts", now()))
    .await?
    .check()?;
    Ok(())
}

#[derive(SurrealValue)]
struct LinkRow {
    left_id: String,
    right_id: String,
    relation: String,
    rationale: String,
    status: String,
}

#[derive(SurrealValue)]
struct ExplRow {
    left_id: String,
    right_id: String,
    explanation: String,
}

/// `get_intel_link_explanations`: the article's active snapshot links when
/// it has one, else the saved explanations.
pub async fn link_explanations(
    db: &Surreal<Any>,
    article_id: &str,
) -> Result<HashMap<(String, String), String>, TxError> {
    let mut r = db
        .query(
            "LET $active = (SELECT VALUE active_snapshot_id FROM ONLY \
               type::record('insight_owner_state', ['article', $article]));
             RETURN $active;
             IF $active != NONE AND $active != '' {
               RETURN (SELECT left_id, right_id, relation, rationale, status, id
                 FROM insight_derivation_link WHERE snapshot_id = $active ORDER BY id)
             } ELSE { RETURN [] };",
        )
        .bind(("article", article_id.to_string()))
        .await?
        .check()?;
    let active: Option<String> = r.take(1)?;
    if active.as_deref().is_some_and(|a| !a.is_empty()) {
        let links: Vec<LinkRow> = r.take(2)?;
        return Ok(links
            .into_iter()
            .map(|l| {
                (
                    (l.left_id, l.right_id),
                    format!("[{} · {}] {}", l.relation, l.status, l.rationale),
                )
            })
            .collect());
    }
    let mut r = db
        .query(EXPLANATIONS_SURQL)
        .bind(("article", article_id.to_string()))
        .await?
        .check()?;
    let rows: Vec<ExplRow> = r.take(0)?;
    Ok(rows
        .into_iter()
        .map(|e| ((e.left_id, e.right_id), e.explanation))
        .collect())
}

pub async fn delete_link_explanations(db: &Surreal<Any>, article_id: &str) -> Result<(), TxError> {
    db.query("DELETE intel_link_explanation WHERE article_id = $article RETURN NONE")
        .bind(("article", article_id.to_string()))
        .await?
        .check()?;
    Ok(())
}

/// One completed report result.
#[derive(Clone, Debug, PartialEq, Eq, SurrealValue)]
pub struct ReportResult {
    pub job_id: String,
    pub source_hash: String,
    pub details_json: String,
    pub completion_sequence: i64,
}

/// Where a new result's sequence goes: after the newest (a completion) or
/// before the oldest (a result retained by job cleanup).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sequence {
    Next,
    Retained,
}

/// Record a completed result once per `(job, generation)`; results are
/// immutable. Returns the sequence, or `None` when it already existed.
pub async fn record_report_result(
    db: &Surreal<Any>,
    job_id: &str,
    generation: i64,
    article_id: &str,
    details_json: &str,
    completed_at: &str,
    sequence: Sequence,
) -> Result<Option<i64>, TxError> {
    let hash = crate::nlp::cache::hash(details_json);
    let pick = match sequence {
        Sequence::Next => "array::max(array::append($seqs, 0)) + 1",
        Sequence::Retained => "array::min(array::append($seqs, 0)) - 1",
    };
    let sql = format!(
        "LET $r = type::record('intel_report_result', [$job, $gen]);
         IF record::exists($r) {{ RETURN NONE }} ELSE {{
           LET $seqs = (SELECT VALUE completion_sequence FROM intel_report_result WHERE article_id = $article);
           LET $seq = {pick};
           CREATE $r SET job_id = $job, generation = $gen, article_id = $article,
             completion_sequence = $seq, source_hash = $hash, details_json = $details,
             completed_at = $at RETURN NONE;
           RETURN $seq
         }};"
    );
    let c = run_tx(db, POLICY, |tx| {
        let (sql, job, article, hash, details, at) = (
            sql.clone(),
            job_id.to_string(),
            article_id.to_string(),
            hash.clone(),
            details_json.to_string(),
            completed_at.to_string(),
        );
        async move {
            let q = tx
                .query(sql)
                .bind(("job", job))
                .bind(("gen", generation))
                .bind(("article", article))
                .bind(("hash", hash))
                .bind(("details", details))
                .bind(("at", at));
            let mut r = run(&tx, q).await?;
            take::<Option<i64>>(&mut r, 1)
        }
    })
    .await?;
    Ok(c.value)
}

pub async fn report_result_exists(
    db: &Surreal<Any>,
    job_id: &str,
    generation: i64,
) -> Result<bool, TxError> {
    let mut r = db
        .query("RETURN record::exists(type::record('intel_report_result', [$job, $gen]))")
        .bind(("job", job_id.to_string()))
        .bind(("gen", generation))
        .await?
        .check()?;
    Ok(r.take::<Option<bool>>(0)? == Some(true))
}

/// The article's newest completed (positive-sequence) result.
pub async fn latest_report_result(
    db: &Surreal<Any>,
    article_id: &str,
) -> Result<Option<ReportResult>, TxError> {
    let mut r = db
        .query(LATEST_RESULT_SURQL)
        .bind(("article", article_id.to_string()))
        .await?
        .check()?;
    let rows: Vec<ReportResult> = r.take(0)?;
    Ok(rows.into_iter().next())
}

/// A job's newest result details.
pub async fn job_report_details(
    db: &Surreal<Any>,
    job_id: &str,
) -> Result<Option<String>, TxError> {
    let mut r = db
        .query(
            "SELECT VALUE details_json FROM intel_report_result WHERE job_id = $job \
             ORDER BY generation DESC LIMIT 1",
        )
        .bind(("job", job_id.to_string()))
        .await?
        .check()?;
    let rows: Vec<String> = r.take(0)?;
    Ok(rows.into_iter().next())
}

/// The Report surface's result bundle for one source hash (the
/// `acquisition_id` of a report claim's evidence), newest completion first.
/// `None` when no retained result has that hash.
pub async fn report_details_by_hash(
    db: &Surreal<Any>,
    source_hash: &str,
) -> Result<Option<String>, TxError> {
    if source_hash.is_empty() {
        return Ok(None);
    }
    let mut r = db
        .query(
            "SELECT VALUE details_json FROM intel_report_result WHERE source_hash = $hash \
             ORDER BY completion_sequence DESC LIMIT 1",
        )
        .bind(("hash", source_hash.to_string()))
        .await?
        .check()?;
    let rows: Vec<String> = r.take(0)?;
    Ok(rows.into_iter().next())
}

/// `INSERT OR IGNORE` an article refresh intent; true when new.
pub async fn add_refresh_intent(db: &Surreal<Any>, article_id: &str) -> Result<bool, TxError> {
    let mut r = db
        .query(
            "LET $r = type::record('article_refresh_intent', $article);
             IF record::exists($r) { RETURN false } ELSE {
               CREATE $r SET requested_at = $ts RETURN NONE; RETURN true };",
        )
        .bind(("article", article_id.to_string()))
        .bind(("ts", now()))
        .await?
        .check()?;
    Ok(r.take::<Option<bool>>(1)? == Some(true))
}

pub async fn has_refresh_intent(db: &Surreal<Any>, article_id: &str) -> Result<bool, TxError> {
    let mut r = db
        .query("RETURN record::exists(type::record('article_refresh_intent', $article))")
        .bind(("article", article_id.to_string()))
        .await?
        .check()?;
    Ok(r.take::<Option<bool>>(0)? == Some(true))
}

pub async fn refresh_intents(db: &Surreal<Any>) -> Result<Vec<String>, TxError> {
    let mut r = db
        .query("SELECT VALUE record::id(id) FROM article_refresh_intent")
        .await?
        .check()?;
    Ok(r.take(0)?)
}

pub async fn clear_refresh_intent(db: &Surreal<Any>, article_id: &str) -> Result<(), TxError> {
    db.query("DELETE type::record('article_refresh_intent', $article) RETURN NONE")
        .bind(("article", article_id.to_string()))
        .await?
        .check()?;
    Ok(())
}

/// Mark an article refresh input as retired (`INSERT OR IGNORE`).
pub async fn retire_refresh_input(
    db: &Surreal<Any>,
    article_id: &str,
    input_key: &str,
) -> Result<(), TxError> {
    db.query(
        "LET $r = type::record('article_refresh_retirement', $key);
         IF !record::exists($r) {
           CREATE $r SET article_id = $article, input_key = $input, retired_at = $ts RETURN NONE
         };",
    )
    .bind(("key", encode(&[article_id, input_key])))
    .bind(("article", article_id.to_string()))
    .bind(("input", input_key.to_string()))
    .bind(("ts", now()))
    .await?
    .check()?;
    Ok(())
}

pub async fn refresh_input_retired(
    db: &Surreal<Any>,
    article_id: &str,
    input_key: &str,
) -> Result<bool, TxError> {
    let mut r = db
        .query("RETURN record::exists(type::record('article_refresh_retirement', $key))")
        .bind(("key", encode(&[article_id, input_key])))
        .await?
        .check()?;
    Ok(r.take::<Option<bool>>(0)? == Some(true))
}
