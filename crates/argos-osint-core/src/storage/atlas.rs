//! Atlas run storage on SurrealDB (P3): `atlas_run`, `atlas_article`,
//! `atlas_quota`, `atlas_checkpoint`, `atlas_repair`, plus the admission
//! staging table `atlas_admission` and the rejection receipt table
//! `atlas_rejection` (schema v2).
//!
//! Record keys: `atlas_run` by run id, `atlas_article` by
//! `[run_id, article_id]`, `atlas_quota` by `[provider, day]`,
//! `atlas_checkpoint` by `[run_id, revision]`, `atlas_repair` by run id,
//! `atlas_admission` by `[run_id, article_id]`, and `atlas_rejection` by
//! `[article_id, input_hash, taxonomy_version]`.

use surrealdb::engine::any::Any;
use surrealdb::types::SurrealValue;
use surrealdb::Surreal;

use super::codec::encode;
use super::counters;
use super::jobs::{run, take};
use super::tx::{run_tx, RetryPolicy, TxError};
use crate::store::{AtlasArticleRow, AtlasRunRow};

/// Atlas schema (idempotent). The v2 additions are additive `IF NOT EXISTS`
/// definitions: `categories`/`admitted_at` with the membership and full-text
/// indexes, and the admission/rejection tables. No migration ladder. The
/// analyzer is defined separately, in [`ANALYZERS`].
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
DEFINE FIELD IF NOT EXISTS categories ON atlas_article TYPE array DEFAULT [];
DEFINE FIELD IF NOT EXISTS categories.* ON atlas_article TYPE string;
DEFINE FIELD IF NOT EXISTS admitted_at ON atlas_article TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS seen_at ON atlas_article TYPE string;
DEFINE FIELD IF NOT EXISTS author ON atlas_article TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS image_url ON atlas_article TYPE string DEFAULT '';
DEFINE INDEX IF NOT EXISTS atlas_article_run ON atlas_article FIELDS run_id;
DEFINE INDEX IF NOT EXISTS atlas_article_id ON atlas_article FIELDS article_id;
DEFINE INDEX IF NOT EXISTS atlas_article_bulletin ON atlas_article FIELDS category, published_at, seen_at, article_id, run_id;
DEFINE INDEX IF NOT EXISTS atlas_article_bulletin_all ON atlas_article FIELDS published_at, seen_at, article_id, run_id;
DEFINE INDEX IF NOT EXISTS atlas_article_categories ON atlas_article FIELDS categories.*;
DEFINE INDEX IF NOT EXISTS atlas_article_title_search ON atlas_article FIELDS title FULLTEXT ANALYZER atlas_article_text BM25;
DEFINE INDEX IF NOT EXISTS atlas_article_description_search ON atlas_article FIELDS description FULLTEXT ANALYZER atlas_article_text BM25;
DEFINE TABLE IF NOT EXISTS atlas_admission SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS run_id ON atlas_admission TYPE string;
DEFINE FIELD IF NOT EXISTS article_id ON atlas_admission TYPE string;
DEFINE FIELD IF NOT EXISTS payload_json ON atlas_admission TYPE string;
DEFINE FIELD IF NOT EXISTS input_hash ON atlas_admission TYPE string;
DEFINE FIELD IF NOT EXISTS source_revision ON atlas_admission TYPE string;
DEFINE FIELD IF NOT EXISTS state ON atlas_admission TYPE string;
DEFINE FIELD IF NOT EXISTS attempts ON atlas_admission TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS last_error ON atlas_admission TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS taxonomy_version ON atlas_admission TYPE string;
DEFINE FIELD IF NOT EXISTS created_at ON atlas_admission TYPE string;
DEFINE FIELD IF NOT EXISTS updated_at ON atlas_admission TYPE string;
DEFINE INDEX IF NOT EXISTS atlas_admission_state ON atlas_admission FIELDS state, updated_at;
DEFINE INDEX IF NOT EXISTS atlas_admission_article ON atlas_admission FIELDS article_id;
DEFINE INDEX IF NOT EXISTS atlas_admission_hash ON atlas_admission FIELDS input_hash;
DEFINE TABLE IF NOT EXISTS atlas_rejection SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS article_id ON atlas_rejection TYPE string;
DEFINE FIELD IF NOT EXISTS input_hash ON atlas_rejection TYPE string;
DEFINE FIELD IF NOT EXISTS taxonomy_version ON atlas_rejection TYPE string;
DEFINE FIELD IF NOT EXISTS source_revision ON atlas_rejection TYPE string;
DEFINE FIELD IF NOT EXISTS reason ON atlas_rejection TYPE string;
DEFINE FIELD IF NOT EXISTS rejected_at ON atlas_rejection TYPE string;
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

/// The article full-text analyzer. [`super::bootstrap::ensure_schema`]
/// applies analyzer definitions ahead of the guarded schema transaction: a
/// full-text index defined inside that transaction cannot resolve an
/// analyzer defined in the same one when the indexed table already holds
/// rows, which broke opening a schema-version 1 database. [`SCHEMA`]
/// depends on this being in place first, in bootstrap and
/// [`apply_schema`] alike.
pub const ANALYZERS: &str =
    "DEFINE ANALYZER IF NOT EXISTS atlas_article_text TOKENIZERS class FILTERS lowercase, ascii;\n";

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
    db.query(ANALYZERS).await?.check()?;
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
    /// Missing on rows written before schema v2 (legacy scalar only).
    categories: Option<Vec<String>>,
    /// Missing on rows written before schema v2.
    admitted_at: Option<String>,
    seen_at: String,
    author: String,
    image_url: String,
}

impl From<ArticleRow> for AtlasArticleRow {
    fn from(r: ArticleRow) -> Self {
        let categories = r.categories.unwrap_or_default();
        // Legacy rows keep their stored scalar until the resumable
        // migration writes the array; v2 rows project the first tag.
        let category = match categories.first() {
            Some(first) => first.clone(),
            None => r.category,
        };
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
            category,
            seen_at: r.seen_at,
            author: r.author,
            image_url: r.image_url,
            categories,
            admitted_at: r.admitted_at.unwrap_or_default(),
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

/// `atlas::save_native_receipt`: the page receipt (`app_state`) and the
/// run's cursor and stats commit in one transaction, so a crash can neither
/// skip unapplied counts nor replay an applied page.
pub async fn save_with_receipt(
    db: &Surreal<Any>,
    id: &str,
    receipt_key: &str,
    receipt_json: &str,
    cursor_json: &str,
    stats_json: &str,
) -> Result<(), TxError> {
    if id.is_empty() || receipt_key.is_empty() {
        return Err(TxError::Abort("atlas receipt: empty id".into()));
    }
    let phase = serde_json::from_str::<serde_json::Value>(cursor_json)
        .ok()
        .and_then(|v| v.get("phase").and_then(|p| p.as_i64()))
        .unwrap_or(1);
    exec(
        db,
        db.query(
            "BEGIN TRANSACTION;
             UPSERT type::record('app_state', $key) SET value = $receipt RETURN NONE;
             UPDATE type::record('atlas_run', $id) SET phase = $phase, cursor_json = $cursor,
               stats_json = $stats RETURN NONE;
             COMMIT TRANSACTION;",
        )
        .bind(("key", receipt_key.to_string()))
        .bind(("receipt", receipt_json.to_string()))
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

/// `atlas_upsert_article`: insert or replace every column. `category` is
/// stored as the projection of `categories` (first tag), so legacy bulletin
/// readers keep their semantics.
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
                    category: row.projected_category().to_string(),
                    categories: Some(row.categories.clone()),
                    admitted_at: Some(row.admitted_at.clone()),
                    seen_at: row.seen_at.clone(),
                    author: row.author.clone(),
                    image_url: row.image_url.clone(),
                },
            )),
    )
    .await
}

/// `atlas_set_category` (legacy scalar setter; the scalar projects the first
/// tag on v2 rows, so callers pass the projected tag).
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
/// `(run, revision)` wins, as in the legacy store. Returns whether it was new.
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

/// `atlas_articles_for_intel`: one category's admitted articles from runs
/// started on `day` (`YYYY-MM-DD`), optionally filtered by a
/// case-insensitive `query` over title, description, source and URL; newest
/// published first, one row per article id (the newest run's). Filters use
/// ordered-category membership (`categories CONTAINS` semantics in Rust;
/// the `categories.*` index serves it in Phase 4's indexed queries); `all`
/// means every admitted article (non-empty tags), deduplicated by identity.
pub async fn articles_for_intel(
    db: &Surreal<Any>,
    category: &str,
    day: &str,
    query: &str,
) -> Result<Vec<AtlasArticleRow>, TxError> {
    let q = query.trim().to_ascii_lowercase();
    let all = category == "all";
    let mut rows = Vec::new();
    for run in list_runs(db).await? {
        if run.started_at.get(..10) != Some(day) {
            continue;
        }
        for a in list_articles(db, &run.id).await? {
            let member = if all {
                !a.categories.is_empty()
            } else {
                a.categories.iter().any(|tag| tag == category)
            };
            let hit = q.is_empty()
                || [
                    &a.title,
                    &a.description,
                    &a.source_name,
                    &a.source_domain,
                    &a.url,
                ]
                .iter()
                .any(|f| f.to_ascii_lowercase().contains(&q));
            if member && hit {
                rows.push(a);
            }
        }
    }
    rows.sort_by(|a, b| {
        b.published_at
            .cmp(&a.published_at)
            .then_with(|| b.seen_at.cmp(&a.seen_at))
            .then_with(|| a.id.cmp(&b.id))
    });
    let mut seen = std::collections::HashSet::new();
    rows.retain(|r| seen.insert(r.id.clone()));
    Ok(rows)
}

// ---- admission staging (schema v2) -------------------------------------

/// Admission states. `pending`/`evaluating`/`failed` are live staging rows;
/// `admitted` and `discarded` are terminal and the row is deleted as soon as
/// the canonical row or rejection receipt commits, so the staging table
/// stays bounded.
pub const ADMISSION_RETRY_STATES: [&str; 2] = ["pending", "failed"];

/// One bounded staging row. The payload carries the fetched article until
/// admission; the row itself never holds derived indices.
#[derive(Clone, Debug, SurrealValue)]
pub struct AdmissionRow {
    pub run_id: String,
    pub article_id: String,
    pub payload_json: String,
    pub input_hash: String,
    pub source_revision: String,
    pub state: String,
    pub attempts: i64,
    pub last_error: String,
    pub taxonomy_version: String,
    pub created_at: String,
    pub updated_at: String,
}

impl AdmissionRow {
    /// The staged article payload.
    pub fn article(&self) -> Result<AtlasArticleRow, TxError> {
        serde_json::from_str(&self.payload_json)
            .map_err(|e| TxError::Abort(format!("admission payload {}: {e}", self.article_id)))
    }
}

/// Stage one fetched article for admission. The row key is
/// `[run_id, article_id]`; re-staging the same input is idempotent and a
/// changed input hash or taxonomy replaces the row as a fresh `pending`
/// attempt. Returns true when a new row was created.
pub async fn stage_article(
    db: &Surreal<Any>,
    row: &AtlasArticleRow,
    source_revision: &str,
    taxonomy_version: &str,
) -> Result<bool, TxError> {
    let payload = serde_json::to_string(row)
        .map_err(|e| TxError::Abort(format!("admission payload: {e}")))?;
    let hash = crate::atlas_metadata::input_hash(row);
    let mut r = db
        .query(
            "LET $r = type::record('atlas_admission', $key);
             LET $same = ((SELECT VALUE input_hash FROM ONLY $r) ?? '') = $hash
               AND ((SELECT VALUE taxonomy_version FROM ONLY $r) ?? '') = $tax;
             -- Re-staging the same input is a no-op; a changed input hash or
             -- taxonomy replaces the row as a fresh pending attempt (the
             -- record may exist, so UPSERT).
             IF $same { RETURN false } ELSE {
               UPSERT $r CONTENT {
                 run_id: $run, article_id: $article, payload_json: $payload,
                 input_hash: $hash, source_revision: $revision, state: 'pending',
                 attempts: 0, last_error: '', taxonomy_version: $tax,
                 created_at: $now, updated_at: $now
               } RETURN NONE;
               RETURN true
             };",
        )
        .bind(("key", encode(&[&row.run_id, &row.id])))
        .bind(("run", row.run_id.to_string()))
        .bind(("article", row.id.to_string()))
        .bind(("payload", payload))
        .bind(("hash", hash))
        .bind(("revision", source_revision.to_string()))
        .bind(("tax", taxonomy_version.to_string()))
        .bind(("now", now()))
        .await?
        .check()?;
    Ok(r.take::<Option<bool>>(2)?.unwrap_or(false))
}

/// Live staging rows of one run, oldest update first.
pub async fn admission_rows(db: &Surreal<Any>, run_id: &str) -> Result<Vec<AdmissionRow>, TxError> {
    let mut r = db
        .query(
            "SELECT run_id, article_id, payload_json, input_hash, source_revision, state, \
             attempts, last_error, taxonomy_version, created_at, updated_at, record::id(id) AS id \
             FROM atlas_admission WHERE run_id = $run ORDER BY updated_at, article_id",
        )
        .bind(("run", run_id.to_string()))
        .await?
        .check()?;
    Ok(r.take(0)?)
}

/// One staging row.
pub async fn admission_row(
    db: &Surreal<Any>,
    run_id: &str,
    article_id: &str,
) -> Result<Option<AdmissionRow>, TxError> {
    let mut r = db
        .query(
            "SELECT run_id, article_id, payload_json, input_hash, source_revision, state, \
             attempts, last_error, taxonomy_version, created_at, updated_at, record::id(id) AS id \
             FROM ONLY type::record('atlas_admission', $key)",
        )
        .bind(("key", encode(&[run_id, article_id])))
        .await?
        .check()?;
    Ok(r.take::<Option<AdmissionRow>>(0)?)
}

/// Retryable staging work across runs (`pending`/`failed`), oldest update
/// first; pinned to `atlas_admission_state`.
pub const ADMISSION_WORK_SURQL: &str = "SELECT run_id, article_id, payload_json, input_hash, \
     source_revision, state, attempts, last_error, taxonomy_version, created_at, updated_at, \
     record::id(id) AS id FROM atlas_admission WHERE state INSIDE $states \
     ORDER BY updated_at, article_id";

/// Bounded upper bound on work claimed per pass.
pub const ADMISSION_WORK_LIMIT: usize = 512;

pub async fn admission_work(db: &Surreal<Any>) -> Result<Vec<AdmissionRow>, TxError> {
    let mut r = db
        .query(format!("{ADMISSION_WORK_SURQL} LIMIT $limit"))
        .bind(("states", ADMISSION_RETRY_STATES.to_vec()))
        .bind(("limit", ADMISSION_WORK_LIMIT as i64))
        .await?
        .check()?;
    Ok(r.take(0)?)
}

/// Reset rows left in `evaluating` by an interrupted cycle so they retry.
pub async fn admission_reset_stale(db: &Surreal<Any>) -> Result<(), TxError> {
    exec(
        db,
        db.query(
            "UPDATE atlas_admission SET state = 'pending', updated_at = $now \
             WHERE state = 'evaluating' RETURN NONE",
        )
        .bind(("now", now())),
    )
    .await
}

/// Mark one staged row evaluating, guarded by its input hash. False when the
/// row moved on (re-staged with new input or already terminal).
pub async fn admission_begin(
    db: &Surreal<Any>,
    run_id: &str,
    article_id: &str,
    input_hash: &str,
) -> Result<bool, TxError> {
    let mut r = db
        .query(
            "UPDATE type::record('atlas_admission', $key) SET state = 'evaluating', \
             updated_at = $now WHERE input_hash = $hash AND state INSIDE $states \
             RETURN VALUE state",
        )
        .bind(("key", encode(&[run_id, article_id])))
        .bind(("hash", input_hash.to_string()))
        .bind(("states", ADMISSION_RETRY_STATES.to_vec()))
        .bind(("now", now()))
        .await?
        .check()?;
    Ok(r.take::<Vec<String>>(0)?
        .first()
        .is_some_and(|s| s == "evaluating"))
}

/// Record a retryable failure: attempts increment and the row stays staged.
pub async fn admission_failed(
    db: &Surreal<Any>,
    run_id: &str,
    article_id: &str,
    input_hash: &str,
    error: &str,
) -> Result<(), TxError> {
    exec(
        db,
        db.query(
            "UPDATE ONLY type::record('atlas_admission', $key) SET state = 'failed', \
             attempts += 1, last_error = $error, updated_at = $now \
             WHERE input_hash = $hash RETURN NONE",
        )
        .bind(("key", encode(&[run_id, article_id])))
        .bind(("hash", input_hash.to_string()))
        .bind(("error", error.chars().take(300).collect::<String>()))
        .bind(("now", now())),
    )
    .await
}

/// Commit admission: the canonical article row (ordered `categories`, scalar
/// projecting the first tag, `admitted_at` stamped) and the staging row's
/// deletion commit together, so a crash leaves either the staged row or the
/// admitted article, never both and never neither. The staging delete is
/// guarded by the evaluated input hash: a concurrent re-stage survives.
pub async fn admission_admit(
    db: &Surreal<Any>,
    row: &AtlasArticleRow,
    input_hash: &str,
    _taxonomy_version: &str,
) -> Result<(), TxError> {
    // The ordered-tags ceiling is a storage invariant, not just a parser
    // rule: an over-cap payload is a retryable failure, never a write.
    if row.categories.len() > crate::atlas::MAX_ADMISSION_TAGS {
        return Err(TxError::Abort(format!(
            "admission {}: {} categories exceed the {} cap",
            row.id,
            row.categories.len(),
            crate::atlas::MAX_ADMISSION_TAGS
        )));
    }
    if row.categories.len() < crate::atlas::MIN_ADMISSION_TAGS {
        return Err(TxError::Abort(format!(
            "admission {}: {} categories are below the {} minimum",
            row.id,
            row.categories.len(),
            crate::atlas::MIN_ADMISSION_TAGS
        )));
    }
    let canonical = ArticleRow {
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
        category: row.projected_category().to_string(),
        categories: Some(row.categories.clone()),
        admitted_at: Some(now()),
        seen_at: row.seen_at.clone(),
        author: row.author.clone(),
        image_url: row.image_url.clone(),
    };
    exec(
        db,
        db.query(
            "BEGIN TRANSACTION;
               UPSERT type::record('atlas_article', $key) CONTENT $row RETURN NONE;
               DELETE type::record('atlas_admission', $staging)
                 WHERE input_hash = $hash RETURN NONE;
             COMMIT TRANSACTION;",
        )
        .bind(("key", encode(&[&row.run_id, &row.id])))
        .bind(("row", canonical))
        .bind(("staging", encode(&[&row.run_id, &row.id])))
        .bind(("hash", input_hash.to_string())),
    )
    .await
}

/// Commit a discard: the minimal rejection receipt and the staging row's
/// deletion commit together. The canonical row (a legacy occurrence being
/// reclassified) is deleted by the caller through lifecycle cleanup.
pub async fn admission_discard(
    db: &Surreal<Any>,
    run_id: &str,
    article_id: &str,
    input_hash: &str,
    source_revision: &str,
    taxonomy_version: &str,
    reason: &str,
) -> Result<(), TxError> {
    exec(
        db,
        db.query(
            "BEGIN TRANSACTION;
               UPSERT type::record('atlas_rejection', $receipt) SET
                 article_id = $article, input_hash = $hash,
                 taxonomy_version = $tax, source_revision = $revision,
                 reason = $reason, rejected_at = $now RETURN NONE;
               DELETE type::record('atlas_admission', $staging) RETURN NONE;
             COMMIT TRANSACTION;",
        )
        .bind((
            "receipt",
            encode(&[article_id, input_hash, taxonomy_version]),
        ))
        .bind(("article", article_id.to_string()))
        .bind(("hash", input_hash.to_string()))
        .bind(("tax", taxonomy_version.to_string()))
        .bind(("revision", source_revision.to_string()))
        .bind(("reason", reason.chars().take(300).collect::<String>()))
        .bind(("staging", encode(&[run_id, article_id])))
        .bind(("now", now())),
    )
    .await
}

/// The reconsideration predicate: true when the current input hash and
/// taxonomy version carry no rejection receipt, so the payload is evaluated.
pub async fn rejection_exists(
    db: &Surreal<Any>,
    article_id: &str,
    input_hash: &str,
    taxonomy_version: &str,
) -> Result<bool, TxError> {
    let mut r = db
        .query("RETURN record::exists(type::record('atlas_rejection', $key))")
        .bind(("key", encode(&[article_id, input_hash, taxonomy_version])))
        .await?
        .check()?;
    Ok(r.take::<Option<bool>>(0)? == Some(true))
}
