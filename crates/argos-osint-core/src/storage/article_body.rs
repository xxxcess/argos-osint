//! Article body acquisition state on the repository: the `article_body`
//! row's lifecycle (`Store::ensure_article_body`, state changes, commit,
//! failure) and the retrieval-attempt ledger
//! (`article_retrieval_attempts`). `intel_recon::body` drives them.

use anyhow::Result;
use chrono::Utc;
use surrealdb::types::SurrealValue;

use super::jobs::rid;
use super::pipeline_inputs::{article_body_for_article, put_article_body};
use super::repo::Repository;
use crate::intel_recon::ArticleBodyRow;

pub const SCHEMA: &str = "
DEFINE TABLE IF NOT EXISTS article_retrieval_attempt SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS body_id ON article_retrieval_attempt TYPE string;
DEFINE FIELD IF NOT EXISTS tool_id ON article_retrieval_attempt TYPE string;
DEFINE FIELD IF NOT EXISTS input_identity ON article_retrieval_attempt TYPE string;
DEFINE FIELD IF NOT EXISTS state ON article_retrieval_attempt TYPE string;
DEFINE FIELD IF NOT EXISTS started_at ON article_retrieval_attempt TYPE string;
DEFINE FIELD IF NOT EXISTS finished_at ON article_retrieval_attempt TYPE string;
DEFINE FIELD IF NOT EXISTS reason ON article_retrieval_attempt TYPE string;
DEFINE FIELD IF NOT EXISTS remote_job_id ON article_retrieval_attempt TYPE string;
DEFINE FIELD IF NOT EXISTS body_version ON article_retrieval_attempt TYPE int;
DEFINE INDEX IF NOT EXISTS article_retrieval_attempt_body ON article_retrieval_attempt FIELDS body_id;
";

fn now() -> String {
    Utc::now().to_rfc3339()
}

fn new_id(prefix: &str) -> String {
    format!(
        "{prefix}-{}",
        crate::store::new_id().trim_start_matches("mem-")
    )
}

#[derive(SurrealValue)]
struct Row {
    id: String,
    article_id: String,
    run_id: String,
    original_url: String,
    resolved_url: String,
    source_domain: String,
    source_name: String,
    body_markdown: String,
    content_hash: String,
    quality: String,
    quality_rationale: String,
    body_version: i64,
    fetch_tool: String,
    fetched_at: String,
    retry_after: String,
    state: String,
    created_at: String,
    updated_at: String,
}

/// `Store::article_body_by_id`.
pub async fn by_id(repo: &Repository, id: &str) -> Result<Option<ArticleBodyRow>> {
    let mut r = repo
        .db()
        .query("SELECT *, meta::id(id) AS id FROM ONLY $b")
        .bind(("b", rid("article_body", id)))
        .await?
        .check()?;
    let row: Option<Row> = r.take(0)?;
    Ok(row.map(|b| ArticleBodyRow {
        id: b.id,
        article_id: b.article_id,
        run_id: b.run_id,
        original_url: b.original_url,
        resolved_url: b.resolved_url,
        source_domain: b.source_domain,
        source_name: b.source_name,
        body_markdown: b.body_markdown,
        content_hash: b.content_hash,
        quality: b.quality,
        quality_rationale: b.quality_rationale,
        body_version: b.body_version,
        fetch_tool: b.fetch_tool,
        fetched_at: b.fetched_at,
        retry_after: b.retry_after,
        state: b.state,
        created_at: b.created_at,
        updated_at: b.updated_at,
    }))
}

/// `Store::ensure_article_body`: the article's newest body, else a new idle
/// unavailable row at version 1.
pub async fn ensure(
    repo: &Repository,
    article_id: &str,
    run_id: &str,
    url: &str,
    source_domain: &str,
    source_name: &str,
) -> Result<ArticleBodyRow> {
    if let Some(existing) = article_body_for_article(repo, article_id).await? {
        return Ok(existing);
    }
    let ts = now();
    let row = ArticleBodyRow {
        id: new_id("abody"),
        article_id: article_id.into(),
        run_id: run_id.into(),
        original_url: url.into(),
        resolved_url: String::new(),
        source_domain: source_domain.into(),
        source_name: source_name.into(),
        body_markdown: String::new(),
        content_hash: String::new(),
        quality: "unavailable".into(),
        quality_rationale: String::new(),
        body_version: 1,
        fetch_tool: String::new(),
        fetched_at: String::new(),
        retry_after: String::new(),
        state: "idle".into(),
        created_at: ts.clone(),
        updated_at: ts,
    };
    put_article_body(repo, &row).await?;
    Ok(row)
}

async fn exec(
    repo: &Repository,
    sql: &'static str,
    id: &str,
    binds: Vec<(&'static str, serde_json::Value)>,
) -> Result<()> {
    let mut q = repo
        .db()
        .query(sql)
        .bind(("b", rid("article_body", id)))
        .bind(("now", now()));
    for (k, v) in binds {
        q = q.bind((k, v));
    }
    q.await?.check()?;
    Ok(())
}

/// `Store::update_article_body_state`.
pub async fn set_state(repo: &Repository, id: &str, state: &str) -> Result<()> {
    exec(
        repo,
        "UPDATE $b SET state = $st, updated_at = $now RETURN NONE",
        id,
        vec![("st", state.into())],
    )
    .await
}

/// `Store::clear_article_body_content`.
pub async fn clear_content(repo: &Repository, id: &str) -> Result<()> {
    exec(
        repo,
        "UPDATE $b SET body_markdown = '', content_hash = '', quality = 'unavailable',
           quality_rationale = '', state = 'running', retry_after = '', updated_at = $now
         RETURN NONE",
        id,
        vec![],
    )
    .await
}

/// `Store::commit_article_body` (the NLP source registration is the
/// caller's).
#[allow(clippy::too_many_arguments)]
pub async fn commit(
    repo: &Repository,
    id: &str,
    markdown: &str,
    content_hash: &str,
    quality: &str,
    rationale: &str,
    resolved_url: &str,
    fetch_tool: &str,
    bump_version: bool,
) -> Result<()> {
    exec(
        repo,
        "UPDATE $b SET body_markdown = $md, content_hash = $ch, quality = $q,
           quality_rationale = $qr, resolved_url = $ru, fetch_tool = $ft, fetched_at = $now,
           retry_after = '', state = 'ready',
           body_version = IF $bump THEN body_version + 1 ELSE body_version END,
           updated_at = $now RETURN NONE",
        id,
        vec![
            ("md", markdown.into()),
            ("ch", content_hash.into()),
            ("q", quality.into()),
            ("qr", rationale.into()),
            ("ru", resolved_url.into()),
            ("ft", fetch_tool.into()),
            ("bump", bump_version.into()),
        ],
    )
    .await
}

/// `Store::mark_article_body_failed`.
pub async fn mark_failed(
    repo: &Repository,
    id: &str,
    rationale: &str,
    retry_after: &str,
    partial_markdown: &str,
    quality: &str,
) -> Result<()> {
    exec(
        repo,
        "UPDATE $b SET quality = $q, quality_rationale = $qr, retry_after = $ra,
           body_markdown = IF string::len($md) > 0 THEN $md ELSE body_markdown END,
           state = 'failed', updated_at = $now RETURN NONE",
        id,
        vec![
            ("q", quality.into()),
            ("qr", rationale.into()),
            ("ra", retry_after.into()),
            ("md", partial_markdown.into()),
        ],
    )
    .await
}

/// `Store::insert_retrieval_attempt`; returns the attempt id.
pub async fn insert_attempt(
    repo: &Repository,
    body_id: &str,
    tool_id: &str,
    input_identity: &str,
    state: &str,
    reason: &str,
    body_version: i64,
) -> Result<String> {
    let id = new_id("attempt");
    let ts = now();
    let finished = if state == "running" {
        String::new()
    } else {
        ts.clone()
    };
    repo.db()
        .query(
            "CREATE $a SET body_id = $body, tool_id = $tool, input_identity = $input,
               state = $st, started_at = $ts, finished_at = $fin, reason = $reason,
               remote_job_id = '', body_version = $v RETURN NONE",
        )
        .bind(("a", rid("article_retrieval_attempt", &id)))
        .bind(("body", body_id.to_string()))
        .bind(("tool", tool_id.to_string()))
        .bind(("input", input_identity.to_string()))
        .bind(("st", state.to_string()))
        .bind(("ts", ts))
        .bind(("fin", finished))
        .bind(("reason", reason.to_string()))
        .bind(("v", body_version))
        .await?
        .check()?;
    Ok(id)
}

/// `Store::finish_retrieval_attempt`.
pub async fn finish_attempt(repo: &Repository, id: &str, state: &str, reason: &str) -> Result<()> {
    repo.db()
        .query("UPDATE $a SET state = $st, finished_at = $now, reason = $reason RETURN NONE")
        .bind(("a", rid("article_retrieval_attempt", id)))
        .bind(("st", state.to_string()))
        .bind(("now", now()))
        .bind(("reason", reason.to_string()))
        .await?
        .check()?;
    Ok(())
}

/// One stored retrieval attempt (tests and diagnostics).
#[derive(Debug, Clone, PartialEq, Eq, SurrealValue)]
pub struct Attempt {
    pub tool_id: String,
    pub state: String,
    pub reason: String,
}

/// A body's attempts in start order.
pub async fn attempts(repo: &Repository, body_id: &str) -> Result<Vec<Attempt>> {
    let mut r = repo
        .db()
        .query(
            "SELECT tool_id, state, reason, started_at FROM article_retrieval_attempt
               WHERE body_id = $b ORDER BY started_at",
        )
        .bind(("b", body_id.to_string()))
        .await?
        .check()?;
    Ok(r.take(0)?)
}
