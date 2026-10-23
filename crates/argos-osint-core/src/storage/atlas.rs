//! Atlas run storage on SurrealDB (P3): `atlas_run`, `atlas_article`,
//! `atlas_quota`, `atlas_checkpoint` and `atlas_repair`, the SurrealDB side
//! of `atlas_runs`, `atlas_articles`, `atlas_quota`,
//! `argos_atlas_checkpoints` and `argos_atlas_repairs`.
//!
//! Record keys: `atlas_run` by run id, `atlas_article` by
//! `[run_id, article_id]`, `atlas_quota` by `[provider, day]`,
//! `atlas_checkpoint` by `[run_id, revision]`, `atlas_repair` by run id.

use surrealdb::engine::any::Any;
use surrealdb::types::SurrealValue;
use surrealdb::Surreal;

use super::codec::encode;
use super::counters;
use super::jobs::{run, take};
use super::tx::{run_tx, RetryPolicy, TxError};
use crate::store::{AtlasArticleRow, AtlasRunRow};

/// Atlas schema (idempotent).
pub const SCHEMA: &str = "DEFINE TABLE IF NOT EXISTS atlas_run SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS state ON atlas_run TYPE string;
DEFINE FIELD IF NOT EXISTS phase ON atlas_run TYPE int DEFAULT 1;
DEFINE FIELD IF NOT EXISTS cursor_json ON atlas_run TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS stats_json ON atlas_run TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS note ON atlas_run TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS started_at ON atlas_run TYPE string;
DEFINE FIELD IF NOT EXISTS finished_at ON atlas_run TYPE string DEFAULT '';
DEFINE INDEX IF NOT EXISTS atlas_run_started ON atlas_run FIELDS started_at;
DEFINE INDEX IF NOT EXISTS atlas_run_state ON atlas_run FIELDS state;
DEFINE TABLE IF NOT EXISTS atlas_article SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS run_id ON atlas_article TYPE string;
DEFINE FIELD IF NOT EXISTS article_id ON atlas_article TYPE string;
DEFINE FIELD IF NOT EXISTS title ON atlas_article TYPE string;
DEFINE FIELD IF NOT EXISTS description ON atlas_article TYPE string;
DEFINE FIELD IF NOT EXISTS url ON atlas_article TYPE string;
DEFINE FIELD IF NOT EXISTS country ON atlas_article TYPE string;
DEFINE FIELD IF NOT EXISTS source_name ON atlas_article TYPE string;
DEFINE FIELD IF NOT EXISTS source_domain ON atlas_article TYPE string;
DEFINE FIELD IF NOT EXISTS published_at ON atlas_article TYPE string;
DEFINE FIELD IF NOT EXISTS provider ON atlas_article TYPE string;
DEFINE FIELD IF NOT EXISTS temperature ON atlas_article TYPE float;
DEFINE FIELD IF NOT EXISTS category ON atlas_article TYPE string DEFAULT 'unk';
DEFINE FIELD IF NOT EXISTS seen_at ON atlas_article TYPE string;
DEFINE FIELD IF NOT EXISTS author ON atlas_article TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS image_url ON atlas_article TYPE string DEFAULT '';
DEFINE INDEX IF NOT EXISTS atlas_article_run ON atlas_article FIELDS run_id;
DEFINE INDEX IF NOT EXISTS atlas_article_id ON atlas_article FIELDS article_id;
DEFINE TABLE IF NOT EXISTS atlas_quota SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS used ON atlas_quota TYPE int DEFAULT 0;
DEFINE TABLE IF NOT EXISTS atlas_checkpoint SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS run_id ON atlas_checkpoint TYPE string;
DEFINE FIELD IF NOT EXISTS revision ON atlas_checkpoint TYPE string;
DEFINE FIELD IF NOT EXISTS payload_json ON atlas_checkpoint TYPE string;
DEFINE FIELD IF NOT EXISTS accepted ON atlas_checkpoint TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS rejected ON atlas_checkpoint TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS created_at ON atlas_checkpoint TYPE string;
DEFINE INDEX IF NOT EXISTS atlas_checkpoint_run ON atlas_checkpoint FIELDS run_id;
DEFINE TABLE IF NOT EXISTS atlas_repair SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS status ON atlas_repair TYPE string;
DEFINE FIELD IF NOT EXISTS detail_json ON atlas_repair TYPE string DEFAULT '{}';
DEFINE FIELD IF NOT EXISTS job_id ON atlas_repair TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS updated_at ON atlas_repair TYPE string;";

/// Runs newest first (`atlas_list_runs`); pinned to `atlas_run_started`.
pub const LIST_RUNS_SURQL: &str = "SELECT record::id(id) AS id, state, phase, cursor_json, \
     stats_json, note, started_at, finished_at FROM atlas_run ORDER BY started_at DESC";
/// One run's articles (`atlas_list_articles`); pinned to `atlas_article_run`.
pub const RUN_ARTICLES_SURQL: &str = "SELECT *, record::id(id) AS id FROM atlas_article \
     WHERE run_id = $run ORDER BY seen_at, article_id";
/// Latest occurrence of one article; pinned to `atlas_article_id`.
pub const LATEST_ARTICLE_SURQL: &str = "SELECT *, record::id(id) AS id FROM atlas_article \
     WHERE article_id = $article ORDER BY seen_at DESC LIMIT 1";

pub async fn apply_schema(db: &Surreal<Any>) -> Result<(), TxError> {
    db.query(SCHEMA).await?.check()?;
    Ok(())
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

#[derive(SurrealValue)]
struct RunRow {
    id: String,
    state: String,
    phase: i64,
    cursor_json: String,
    stats_json: String,
    note: String,
    started_at: String,
    finished_at: String,
}

impl From<RunRow> for AtlasRunRow {
    fn from(r: RunRow) -> Self {
        AtlasRunRow {
            id: r.id,
            state: r.state,
            phase: r.phase,
            cursor_json: r.cursor_json,
            stats_json: r.stats_json,
            note: r.note,
            started_at: r.started_at,
            finished_at: r.finished_at,
        }
    }
}

#[derive(SurrealValue)]
pub(crate) struct ArticleRow {
    run_id: String,
    article_id: String,
    title: String,
    description: String,
    url: String,
    country: String,
    source_name: String,
    source_domain: String,
    published_at: String,
    provider: String,
    temperature: f64,
    category: String,
    seen_at: String,
    author: String,
    image_url: String,
}

impl From<ArticleRow> for AtlasArticleRow {
    fn from(r: ArticleRow) -> Self {
        AtlasArticleRow {
            run_id: r.run_id,
            id: r.article_id,
            title: r.title,
            description: r.description,
            url: r.url,
            country: r.country,
            source_name: r.source_name,
            source_domain: r.source_domain,
            published_at: r.published_at,
            provider: r.provider,
            temperature: r.temperature,
            category: r.category,
            seen_at: r.seen_at,
            author: r.author,
            image_url: r.image_url,
        }
    }
}

async fn exec(db: &Surreal<Any>, q: surrealdb::method::Query<'_, Any>) -> Result<(), TxError> {
    let _ = db;
    q.await?.check()?;
    Ok(())
}

/// `atlas_insert_run`: a new running run at phase 1. Fails if it exists.
pub async fn insert_run(
    db: &Surreal<Any>,
    id: &str,
    cursor_json: &str,
    stats_json: &str,
) -> Result<(), TxError> {
    exec(
        db,
        db.query(
            "CREATE type::record('atlas_run', $id) SET state = 'running', phase = 1, \
             cursor_json = $cursor, stats_json = $stats, note = '', started_at = $now, \
             finished_at = '' RETURN NONE",
        )
        .bind(("id", id.to_string()))
        .bind(("cursor", cursor_json.to_string()))
        .bind(("stats", stats_json.to_string()))
        .bind(("now", now())),
    )
    .await
}

/// `atlas_save`: cursor, stats and the phase read from the cursor.
pub async fn save(
    db: &Surreal<Any>,
    id: &str,
    cursor_json: &str,
    stats_json: &str,
) -> Result<(), TxError> {
    let phase = serde_json::from_str::<serde_json::Value>(cursor_json)
        .ok()
        .and_then(|v| v.get("phase").and_then(|p| p.as_i64()))
        .unwrap_or(1);
    exec(
        db,
        db.query(
            "UPDATE type::record('atlas_run', $id) SET phase = $phase, cursor_json = $cursor, \
             stats_json = $stats RETURN NONE",
        )
        .bind(("id", id.to_string()))
        .bind(("phase", phase))
        .bind(("cursor", cursor_json.to_string()))
        .bind(("stats", stats_json.to_string())),
    )
    .await
}

/// `atlas_set_state`; `finished` stamps `finished_at`, otherwise it is kept.
pub async fn set_state(
    db: &Surreal<Any>,
    id: &str,
    state: &str,
    note: &str,
    finished: bool,
) -> Result<(), TxError> {
    let finished_at = if finished { now() } else { String::new() };
    exec(
        db,
        db.query(
            "UPDATE type::record('atlas_run', $id) SET state = $state, note = $note, \
             finished_at = IF $fin = '' THEN finished_at ELSE $fin END RETURN NONE",
        )
        .bind(("id", id.to_string()))
        .bind(("state", state.to_string()))
        .bind(("note", note.to_string()))
        .bind(("fin", finished_at)),
    )
    .await
}

/// `atlas_park_running`: every running run becomes paused.
pub async fn park_running(db: &Surreal<Any>) -> Result<(), TxError> {
    exec(
        db,
        db.query("UPDATE atlas_run SET state = 'paused', note = 'Paused' WHERE state = 'running' RETURN NONE"),
    )
    .await
}

/// `atlas_list_runs`, newest first.
pub async fn list_runs(db: &Surreal<Any>) -> Result<Vec<AtlasRunRow>, TxError> {
    let mut r = db.query(LIST_RUNS_SURQL).await?.check()?;
    let rows: Vec<RunRow> = r.take(0)?;
    Ok(rows.into_iter().map(Into::into).collect())
}

/// `atlas_latest_run`.
pub async fn latest_run(db: &Surreal<Any>) -> Result<Option<AtlasRunRow>, TxError> {
    let mut r = db
        .query(format!("{LIST_RUNS_SURQL} LIMIT 1"))
        .await?
        .check()?;
    let rows: Vec<RunRow> = r.take(0)?;
    Ok(rows.into_iter().next().map(Into::into))
}

/// `atlas_run_days`: distinct `YYYY-MM-DD` run days, newest first.
pub async fn run_days(db: &Surreal<Any>) -> Result<Vec<String>, TxError> {
    let mut r = db
        .query("SELECT VALUE started_at FROM atlas_run")
        .await?
        .check()?;
    let started: Vec<String> = r.take(0)?;
    let mut days: Vec<String> = started
        .iter()
        .filter_map(|s| s.get(..10).map(str::to_string))
        .collect();
    days.sort_unstable_by(|a, b| b.cmp(a));
    days.dedup();
    Ok(days)
}

/// `atlas_upsert_article`: insert or replace every column.
pub async fn upsert_article(db: &Surreal<Any>, row: &AtlasArticleRow) -> Result<(), TxError> {
    exec(
        db,
        db.query("UPSERT type::record('atlas_article', $key) CONTENT $row RETURN NONE")
            .bind(("key", encode(&[&row.run_id, &row.id])))
            .bind((
                "row",
                ArticleRow {
                    run_id: row.run_id.clone(),
                    article_id: row.id.clone(),
                    title: row.title.clone(),
                    description: row.description.clone(),
                    url: row.url.clone(),
                    country: row.country.clone(),
                    source_name: row.source_name.clone(),
                    source_domain: row.source_domain.clone(),
                    published_at: row.published_at.clone(),
                    provider: row.provider.clone(),
                    temperature: row.temperature,
                    category: row.category.clone(),
                    seen_at: row.seen_at.clone(),
                    author: row.author.clone(),
                    image_url: row.image_url.clone(),
                },
            )),
    )
    .await
}

/// `atlas_set_category`.
pub async fn set_category(
    db: &Surreal<Any>,
    run_id: &str,
    article_id: &str,
    category: &str,
) -> Result<(), TxError> {
    exec(
        db,
        db.query("UPDATE type::record('atlas_article', $key) SET category = $c RETURN NONE")
            .bind(("key", encode(&[run_id, article_id])))
            .bind(("c", category.to_string())),
    )
    .await
}

/// `atlas_list_articles`: one run's articles by `seen_at, article_id`.
pub async fn list_articles(
    db: &Surreal<Any>,
    run_id: &str,
) -> Result<Vec<AtlasArticleRow>, TxError> {
    let mut r = db
        .query(RUN_ARTICLES_SURQL)
        .bind(("run", run_id.to_string()))
        .await?
        .check()?;
    let rows: Vec<ArticleRow> = r.take(0)?;
    Ok(rows.into_iter().map(Into::into).collect())
}

/// `atlas_recent_articles`: every retained article, newest first.
pub async fn recent_articles(db: &Surreal<Any>) -> Result<Vec<AtlasArticleRow>, TxError> {
    let mut r = db
        .query(
            "SELECT *, record::id(id) AS id FROM atlas_article ORDER BY seen_at DESC, article_id",
        )
        .await?
        .check()?;
    let rows: Vec<ArticleRow> = r.take(0)?;
    Ok(rows.into_iter().map(Into::into).collect())
}

/// `atlas_article`.
pub async fn article(
    db: &Surreal<Any>,
    run_id: &str,
    article_id: &str,
) -> Result<Option<AtlasArticleRow>, TxError> {
    let mut r = db
        .query("SELECT * FROM ONLY type::record('atlas_article', $key)")
        .bind(("key", encode(&[run_id, article_id])))
        .await?
        .check()?;
    Ok(r.take::<Option<ArticleRow>>(0)?.map(Into::into))
}

/// `atlas_article` inside the caller's transaction.
pub(crate) async fn article_in(
    tx: &super::tx::Tx,
    run_id: &str,
    article_id: &str,
) -> Result<Option<AtlasArticleRow>, TxError> {
    let mut r = run(
        tx,
        tx.query("SELECT * FROM ONLY type::record('atlas_article', $key)")
            .bind(("key", encode(&[run_id, article_id]))),
    )
    .await?;
    Ok(take::<Option<ArticleRow>>(&mut r, 0)?.map(Into::into))
}

/// `atlas_latest_article_by_id`.
pub async fn latest_article_by_id(
    db: &Surreal<Any>,
    article_id: &str,
) -> Result<Option<AtlasArticleRow>, TxError> {
    let mut r = db
        .query(LATEST_ARTICLE_SURQL)
        .bind(("article", article_id.to_string()))
        .await?
        .check()?;
    let rows: Vec<ArticleRow> = r.take(0)?;
    Ok(rows.into_iter().next().map(Into::into))
}

/// `atlas_quota_used`.
pub async fn quota_used(db: &Surreal<Any>, provider: &str, day: &str) -> Result<u32, TxError> {
    let mut r = db
        .query("RETURN (SELECT VALUE used FROM ONLY type::record('atlas_quota', $key)) ?? 0")
        .bind(("key", encode(&[provider, day])))
        .await?
        .check()?;
    Ok(r.take::<Option<i64>>(0)?.unwrap_or(0).max(0) as u32)
}

/// `atlas_quota_bump`: one atomic increment, returning the new count.
pub async fn quota_bump(db: &Surreal<Any>, provider: &str, day: &str) -> Result<u32, TxError> {
    let key = encode(&[provider, day]);
    let policy: RetryPolicy = counters::HOT_RECORD_POLICY;
    let c = run_tx(db, policy, |tx| {
        let key = key.clone();
        async move {
            let mut r = run(
                &tx,
                tx.query("UPSERT type::record('atlas_quota', $key) SET used = (used ?? 0) + 1 RETURN VALUE used")
                    .bind(("key", key)),
            )
            .await?;
            let used: Vec<i64> = take(&mut r, 0)?;
            used.first()
                .copied()
                .ok_or_else(|| TxError::Abort("atlas_quota returned no value".into()))
        }
    })
    .await?;
    Ok(c.value.max(0) as u32)
}

/// `argos_atlas_checkpoints` insert-or-keep: the first payload for a
/// `(run, revision)` wins, as in SQLite. Returns whether it was new.
pub async fn save_checkpoint(
    db: &Surreal<Any>,
    run_id: &str,
    revision: &str,
    payload_json: &str,
    accepted: i64,
    rejected: i64,
) -> Result<bool, TxError> {
    let mut r = db
        .query(
            "LET $r = type::record('atlas_checkpoint', $key);
             IF record::exists($r) { RETURN false } ELSE {
               CREATE $r SET run_id = $run, revision = $rev, payload_json = $payload,
                 accepted = $acc, rejected = $rej, created_at = $now RETURN NONE;
               RETURN true
             };",
        )
        .bind(("key", encode(&[run_id, revision])))
        .bind(("run", run_id.to_string()))
        .bind(("rev", revision.to_string()))
        .bind(("payload", payload_json.to_string()))
        .bind(("acc", accepted))
        .bind(("rej", rejected))
        .bind(("now", now()))
        .await?
        .check()?;
    Ok(r.take::<Option<bool>>(1)? == Some(true))
}

/// A checkpoint's payload.
pub async fn checkpoint(
    db: &Surreal<Any>,
    run_id: &str,
    revision: &str,
) -> Result<Option<String>, TxError> {
    let mut r = db
        .query("SELECT VALUE payload_json FROM ONLY type::record('atlas_checkpoint', $key)")
        .bind(("key", encode(&[run_id, revision])))
        .await?
        .check()?;
    Ok(r.take::<Option<String>>(0)?)
}

/// `argos_atlas_repairs` upsert.
pub async fn set_repair(
    db: &Surreal<Any>,
    run_id: &str,
    status: &str,
    detail_json: &str,
    job_id: &str,
) -> Result<(), TxError> {
    exec(
        db,
        db.query(
            "UPSERT type::record('atlas_repair', $run) SET status = $status, \
             detail_json = $detail, job_id = $job, updated_at = $now RETURN NONE",
        )
        .bind(("run", run_id.to_string()))
        .bind(("status", status.to_string()))
        .bind(("detail", detail_json.to_string()))
        .bind(("job", job_id.to_string()))
        .bind(("now", now())),
    )
    .await
}

/// A run's repair status.
pub async fn repair_status(db: &Surreal<Any>, run_id: &str) -> Result<Option<String>, TxError> {
    let mut r = db
        .query("SELECT VALUE status FROM ONLY type::record('atlas_repair', $run)")
        .bind(("run", run_id.to_string()))
        .await?
        .check()?;
    Ok(r.take::<Option<String>>(0)?)
}
