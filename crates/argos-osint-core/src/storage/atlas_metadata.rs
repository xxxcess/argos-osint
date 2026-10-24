//! Atlas article metadata on SurrealDB (P3 step 4): `atlas_article_metadata`,
//! `atlas_article_mention` and `atlas_related_article`, with the ports of
//! `Store::atlas_metadata`, `atlas_save_metadata` and
//! `atlas_set_classification`.
//!
//! Record keys: metadata by `[run_id, article_id]`, mentions by their SQLite
//! primary key, related pairs by `[run_id, left_id, right_id]`. SQLite's
//! `ON DELETE CASCADE` from `atlas_articles` is the `atlas_article_cascade`
//! event, so every article delete (step 3) drops these rows in its own
//! transaction.
//!
//! Not ported here: `persist_article_rating` (it reads the SQLite confidence
//! inputs); see `.planning/2026-10-22-surrealdb/acceptance.md`.

use surrealdb::engine::any::Any;
use surrealdb::types::SurrealValue;
use surrealdb::Surreal;

use super::atlas::article_in;
use super::codec::encode;
use super::jobs::{run, take};
use super::tx::{run_tx, RetryPolicy, Tx, TxError};
use crate::atlas_metadata::{comparison_key, input_hash, ArticleMetadata};

/// Metadata schema (idempotent). Apply after `atlas::apply_schema`.
pub const SCHEMA: &str = "DEFINE TABLE IF NOT EXISTS atlas_article_metadata SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS run_id ON atlas_article_metadata TYPE string;
DEFINE FIELD IF NOT EXISTS article_id ON atlas_article_metadata TYPE string;
DEFINE FIELD IF NOT EXISTS input_hash ON atlas_article_metadata TYPE string;
DEFINE FIELD IF NOT EXISTS entity_state ON atlas_article_metadata TYPE string;
DEFINE FIELD IF NOT EXISTS reliability_state ON atlas_article_metadata TYPE string;
DEFINE FIELD IF NOT EXISTS classification_status ON atlas_article_metadata TYPE string;
DEFINE FIELD IF NOT EXISTS metadata_json ON atlas_article_metadata TYPE string;
DEFINE TABLE IF NOT EXISTS atlas_article_mention SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS run_id ON atlas_article_mention TYPE string;
DEFINE FIELD IF NOT EXISTS article_id ON atlas_article_mention TYPE string;
DEFINE FIELD IF NOT EXISTS type ON atlas_article_mention TYPE string;
DEFINE FIELD IF NOT EXISTS comparison_key ON atlas_article_mention TYPE string;
DEFINE FIELD IF NOT EXISTS field ON atlas_article_mention TYPE string;
DEFINE FIELD IF NOT EXISTS start_byte ON atlas_article_mention TYPE int;
DEFINE FIELD IF NOT EXISTS end_byte ON atlas_article_mention TYPE int;
DEFINE FIELD IF NOT EXISTS original_text ON atlas_article_mention TYPE string;
DEFINE FIELD IF NOT EXISTS input_hash ON atlas_article_mention TYPE string;
DEFINE INDEX IF NOT EXISTS atlas_mentions_key ON atlas_article_mention FIELDS run_id, type, comparison_key;
DEFINE INDEX IF NOT EXISTS atlas_mentions_article ON atlas_article_mention FIELDS run_id, article_id;
DEFINE TABLE IF NOT EXISTS atlas_related_article SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS run_id ON atlas_related_article TYPE string;
DEFINE FIELD IF NOT EXISTS left_id ON atlas_related_article TYPE string;
DEFINE FIELD IF NOT EXISTS right_id ON atlas_related_article TYPE string;
DEFINE FIELD IF NOT EXISTS left_revision ON atlas_related_article TYPE string;
DEFINE FIELD IF NOT EXISTS right_revision ON atlas_related_article TYPE string;
DEFINE FIELD IF NOT EXISTS shared_json ON atlas_related_article TYPE string;
DEFINE FIELD IF NOT EXISTS match_version ON atlas_related_article TYPE string;
DEFINE FIELD IF NOT EXISTS named_count ON atlas_related_article TYPE int;
DEFINE FIELD IF NOT EXISTS shared_count ON atlas_related_article TYPE int;
DEFINE FIELD IF NOT EXISTS rarity ON atlas_related_article TYPE float;
DEFINE INDEX IF NOT EXISTS atlas_related_left ON atlas_related_article FIELDS run_id, left_id;
DEFINE INDEX IF NOT EXISTS atlas_related_right ON atlas_related_article FIELDS run_id, right_id;
DEFINE EVENT IF NOT EXISTS atlas_article_cascade ON TABLE atlas_article
  WHEN $event = 'DELETE' THEN {
    DELETE type::record('atlas_article_metadata', record::id($before.id)) RETURN NONE;
    DELETE atlas_article_mention WHERE run_id = $before.run_id AND article_id = $before.article_id RETURN NONE;
    DELETE atlas_related_article WHERE run_id = $before.run_id AND left_id = $before.article_id RETURN NONE;
    DELETE atlas_related_article WHERE run_id = $before.run_id AND right_id = $before.article_id RETURN NONE;
  };";

/// One article's mentions; pinned to `atlas_mentions_article`.
pub const ARTICLE_MENTIONS_SURQL: &str =
    "SELECT * FROM atlas_article_mention WHERE run_id = $run AND article_id = $article";

pub async fn apply_schema(db: &Surreal<Any>) -> Result<(), TxError> {
    db.query(SCHEMA).await?.check()?;
    Ok(())
}

fn json_err(e: serde_json::Error) -> TxError {
    TxError::Abort(format!("article metadata json: {e}"))
}

async fn metadata_in(
    tx: &Tx,
    run_id: &str,
    article_id: &str,
) -> Result<Option<ArticleMetadata>, TxError> {
    let mut r = run(
        tx,
        tx.query(
            "SELECT VALUE metadata_json FROM ONLY type::record('atlas_article_metadata', $key)",
        )
        .bind(("key", encode(&[run_id, article_id]))),
    )
    .await?;
    take::<Option<String>>(&mut r, 0)?
        .map(|j| serde_json::from_str(&j).map_err(json_err))
        .transpose()
}

/// `atlas_metadata`.
pub async fn metadata(
    db: &Surreal<Any>,
    run_id: &str,
    article_id: &str,
) -> Result<Option<ArticleMetadata>, TxError> {
    let mut r = db
        .query("SELECT VALUE metadata_json FROM ONLY type::record('atlas_article_metadata', $key)")
        .bind(("key", encode(&[run_id, article_id])))
        .await?
        .check()?;
    r.take::<Option<String>>(0)?
        .map(|j| serde_json::from_str(&j).map_err(json_err))
        .transpose()
}

async fn write_metadata_in(tx: &Tx, m: &ArticleMetadata, json: String) -> Result<(), TxError> {
    run(
        tx,
        tx.query(
            "UPSERT type::record('atlas_article_metadata', $key) CONTENT {
               run_id: $run, article_id: $article, input_hash: $hash, entity_state: $es,
               reliability_state: $rs, classification_status: $cs, metadata_json: $json
             } RETURN NONE;",
        )
        .bind(("key", encode(&[&m.run_id, &m.article_id])))
        .bind(("run", m.run_id.clone()))
        .bind(("article", m.article_id.clone()))
        .bind(("hash", m.input_hash.clone()))
        .bind(("es", m.entity_state.as_str().to_string()))
        .bind(("rs", m.reliability_state.as_str().to_string()))
        .bind(("cs", m.classification_status.clone()))
        .bind(("json", json)),
    )
    .await?;
    Ok(())
}

/// The Related publication of `related_articles::build`: one transaction
/// replaces the run's pairs, keeping only pairs whose two articles are still
/// exactly the rows they were planned from. Returns the stored pair count.
pub async fn replace_related(
    db: &Surreal<Any>,
    run_id: &str,
    articles: &[crate::store::AtlasArticleRow],
    planned: &[crate::related_articles::PlannedPair],
) -> Result<usize, TxError> {
    let (run_id, articles, planned) = (run_id.to_string(), articles.to_vec(), planned.to_vec());
    run_tx(db, RetryPolicy::default(), move |tx| {
        let (run_id, articles, planned) = (run_id.clone(), articles.clone(), planned.clone());
        async move {
            run(
                &tx,
                tx.query("DELETE atlas_related_article WHERE run_id = $run RETURN NONE;")
                    .bind(("run", run_id.clone())),
            )
            .await?;
            let mut count = 0usize;
            for p in &planned {
                let (left, right) = (&articles[p.left], &articles[p.right]);
                if article_in(&tx, &run_id, &left.id).await?.as_ref() != Some(left)
                    || article_in(&tx, &run_id, &right.id).await?.as_ref() != Some(right)
                {
                    continue;
                }
                run(
                    &tx,
                    tx.query(
                        "UPSERT type::record('atlas_related_article', $key) CONTENT {
                           run_id: $run, left_id: $l, right_id: $r, left_revision: $lr,
                           right_revision: $rr, shared_json: $shared, match_version: $mv,
                           named_count: $named, shared_count: $count, rarity: $rarity
                         } RETURN NONE;",
                    )
                    .bind(("key", encode(&[&run_id, &left.id, &right.id])))
                    .bind(("run", run_id.clone()))
                    .bind(("l", left.id.clone()))
                    .bind(("r", right.id.clone()))
                    .bind(("lr", p.left_revision.clone()))
                    .bind(("rr", p.right_revision.clone()))
                    .bind(("shared", p.shared_json.clone()))
                    .bind(("mv", crate::related_articles::MATCH_VERSION.to_string()))
                    .bind(("named", p.named_count))
                    .bind(("count", p.shared_count))
                    .bind(("rarity", p.rarity)),
                )
                .await?;
                count += 1;
            }
            Ok(count)
        }
    })
    .await
    .map(|c| c.value)
}

/// Stored Related pairs of one run.
pub async fn related_count(db: &Surreal<Any>, run_id: &str) -> Result<usize, TxError> {
    let mut r = db
        .query("SELECT VALUE count() FROM atlas_related_article WHERE run_id = $run GROUP ALL")
        .bind(("run", run_id.to_string()))
        .await?
        .check()?;
    Ok(r.take::<Option<i64>>(0)?.unwrap_or(0) as usize)
}

/// `atlas_save_metadata`: false when the article is gone or changed since
/// `metadata` was captured. A same-input save keeps the stored
/// classification; the article's mentions and related pairs are replaced,
/// keeping only mentions whose span still matches the article text.
pub async fn save_metadata(db: &Surreal<Any>, metadata: &ArticleMetadata) -> Result<bool, TxError> {
    let committed = run_tx(db, RetryPolicy::default(), |tx| {
        let metadata = metadata.clone();
        async move {
            let Some(article) = article_in(&tx, &metadata.run_id, &metadata.article_id).await?
            else {
                return Ok(false);
            };
            if input_hash(&article) != metadata.input_hash {
                return Ok(false);
            }
            let mut metadata = metadata;
            if let Some(latest) = metadata_in(&tx, &metadata.run_id, &metadata.article_id).await? {
                if latest.input_hash == metadata.input_hash {
                    metadata.classification_status = latest.classification_status.clone();
                    metadata.classification_receipt = latest.classification_receipt.clone();
                }
                let unchanged = serde_json::to_string(&latest).map_err(json_err)?
                    == serde_json::to_string(&metadata).map_err(json_err)?;
                if unchanged {
                    return Ok(true);
                }
            }
            let json = serde_json::to_string(&metadata).map_err(json_err)?;
            write_metadata_in(&tx, &metadata, json).await?;
            let mut rows = Vec::new();
            for m in &metadata.mentions {
                let text = match m.field.as_str() {
                    "title" => &article.title,
                    "description" => &article.description,
                    _ => continue,
                };
                if text.get(m.start_byte..m.end_byte) != Some(m.original_text.as_str())
                    || comparison_key(&m.original_text) != m.comparison_key
                {
                    continue;
                }
                let (start, end) = (m.start_byte.to_string(), m.end_byte.to_string());
                rows.push(Mention {
                    key: encode(&[
                        &metadata.run_id,
                        &metadata.article_id,
                        &m.type_hint,
                        &m.comparison_key,
                        &m.field,
                        &start,
                        &end,
                    ]),
                    kind: m.type_hint.clone(),
                    comparison_key: m.comparison_key.clone(),
                    field: m.field.clone(),
                    start_byte: m.start_byte as i64,
                    end_byte: m.end_byte as i64,
                    original_text: m.original_text.clone(),
                });
            }
            run(
                &tx,
                tx.query(
                    "DELETE atlas_article_mention WHERE run_id = $run AND article_id = $article RETURN NONE;
                     DELETE atlas_related_article WHERE run_id = $run AND left_id = $article RETURN NONE;
                     DELETE atlas_related_article WHERE run_id = $run AND right_id = $article RETURN NONE;
                     FOR $m IN $rows {
                       -- INSERT OR IGNORE: a repeated primary key keeps the first row.
                       IF !record::exists(type::record('atlas_article_mention', $m.key)) {
                         CREATE type::record('atlas_article_mention', $m.key) CONTENT {
                           run_id: $run, article_id: $article, type: $m.kind,
                           comparison_key: $m.comparison_key, field: $m.field,
                           start_byte: $m.start_byte, end_byte: $m.end_byte,
                           original_text: $m.original_text, input_hash: $hash
                         } RETURN NONE;
                       };
                     };",
                )
                .bind(("run", metadata.run_id.clone()))
                .bind(("article", metadata.article_id.clone()))
                .bind(("hash", metadata.input_hash.clone()))
                .bind(("rows", rows)),
            )
            .await?;
            Ok(true)
        }
    })
    .await?;
    Ok(committed.value)
}

#[derive(SurrealValue)]
struct Mention {
    key: String,
    kind: String,
    comparison_key: String,
    field: String,
    start_byte: i64,
    end_byte: i64,
    original_text: String,
}

/// `atlas_set_classification`: false when the article is gone or changed
/// since `captured_hash`. Sets the classification (and optionally the
/// article category) on the current or a pending metadata row.
pub async fn set_classification(
    db: &Surreal<Any>,
    run_id: &str,
    article_id: &str,
    captured_hash: &str,
    status: &str,
    receipt_json: &str,
    category: Option<&str>,
) -> Result<bool, TxError> {
    let receipt = if receipt_json.is_empty() {
        None
    } else {
        Some(serde_json::from_str::<serde_json::Value>(receipt_json).map_err(json_err)?)
    };
    let committed = run_tx(db, RetryPolicy::default(), |tx| {
        let (run_id, article_id, hash) = (
            run_id.to_string(),
            article_id.to_string(),
            captured_hash.to_string(),
        );
        let (status, receipt, category) = (
            status.to_string(),
            receipt.clone(),
            category.map(str::to_string),
        );
        async move {
            let Some(article) = article_in(&tx, &run_id, &article_id).await? else {
                return Ok(false);
            };
            if input_hash(&article) != hash {
                return Ok(false);
            }
            let mut metadata = metadata_in(&tx, &run_id, &article_id)
                .await?
                .unwrap_or_else(|| ArticleMetadata::pending(&article));
            metadata.classification_status = status;
            metadata.classification_receipt = receipt;
            if let Some(category) = category {
                run(
                    &tx,
                    tx.query(
                        "UPDATE type::record('atlas_article', $key) SET category = $c RETURN NONE",
                    )
                    .bind(("key", encode(&[&run_id, &article_id])))
                    .bind(("c", category)),
                )
                .await?;
            }
            // SQLite stores the captured hash on the row.
            let mut row = metadata.clone();
            row.input_hash = hash;
            let json = serde_json::to_string(&metadata).map_err(json_err)?;
            write_metadata_in(&tx, &row, json).await?;
            Ok(true)
        }
    })
    .await?;
    Ok(committed.value)
}
