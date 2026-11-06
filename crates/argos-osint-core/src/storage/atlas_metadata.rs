//! Atlas article metadata on SurrealDB (P3 step 4): `atlas_article_metadata`,
//! `atlas_article_mention` and `atlas_related_article`, with the ports of
//! `Store::atlas_metadata`, `atlas_save_metadata` and
//! `atlas_set_classification`.
//!
//! Record keys: metadata by `[run_id, article_id]`, mentions by their legacy store
//! primary key, related pairs by `[run_id, left_id, right_id]`. The legacy store's
//! `ON DELETE CASCADE` from `atlas_articles` is the `atlas_article_cascade`
//! event, so every article delete (step 3) drops these rows in its own
//! transaction.
//!
//! Not ported here: `persist_article_rating` (it reads the legacy store confidence
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
DEFINE FIELD IF NOT EXISTS left_native_revision ON atlas_related_article TYPE option<string>;
DEFINE FIELD IF NOT EXISTS right_native_revision ON atlas_related_article TYPE option<string>;
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

/// One stored Related pair (`atlas_related_article`).
#[derive(Debug, Clone, SurrealValue)]
pub struct RelatedPairRow {
    pub left_id: String,
    pub right_id: String,
    pub left_revision: String,
    pub right_revision: String,
    pub shared_json: String,
    pub named_count: i64,
    pub shared_count: i64,
    pub rarity: f64,
}

/// The stored Related pairs of one run that include `article_id` on either
/// side (`Store::related_articles`' pair read; the caller orders them).
pub async fn related_pairs(
    db: &Surreal<Any>,
    run_id: &str,
    article_id: &str,
) -> Result<Vec<RelatedPairRow>, TxError> {
    let mut r = db
        .query(
            "SELECT left_id, right_id, left_revision, right_revision, shared_json,
                    named_count, shared_count, rarity
             FROM atlas_related_article
             WHERE run_id = $run AND (left_id = $article OR right_id = $article)",
        )
        .bind(("run", run_id.to_string()))
        .bind(("article", article_id.to_string()))
        .await?
        .check()?;
    Ok(r.take(0)?)
}

/// `atlas_save_metadata`: false when the article is gone or changed since
/// `metadata` was captured. A same-input save keeps the stored
/// classification; the article's mentions and related pairs are replaced,
/// keeping only mentions whose span still matches the article text.
pub async fn save_metadata(db: &Surreal<Any>, metadata: &ArticleMetadata) -> Result<bool, TxError> {
    let metadata = metadata.clone();
    Ok(run_tx(db, RetryPolicy::default(), move |tx| {
        let metadata = metadata.clone();
        async move { save_metadata_in(&tx, &metadata).await }
    })
    .await?
    .value)
}

/// Native metadata and mentions share the classifier source guard, preventing
/// a pre-classification read from overwriting a newer receipt or source.
pub(crate) async fn save_metadata_in(tx: &Tx, metadata: &ArticleMetadata) -> Result<bool, TxError> {
    super::jobs::run(tx,tx.query("UPSERT type::record('atlas_classifier_guard',$key) SET updated_at=time::now() RETURN NONE;").bind(("key",encode(&["source",&metadata.run_id,&metadata.article_id])))).await?;
    let metadata = metadata.clone();
    let Some(article) = article_in(tx, &metadata.run_id, &metadata.article_id).await? else {
        return Ok(false);
    };
    if input_hash(&article) != metadata.input_hash {
        return Ok(false);
    }
    let mut metadata = metadata;
    if let Some(latest) = metadata_in(tx, &metadata.run_id, &metadata.article_id).await? {
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
    write_metadata_in(tx, &metadata, json).await?;
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
    tx,
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
            // the legacy store stores the captured hash on the row.
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

// Appended to storage/atlas_metadata.rs in the next repository checkpoint.
#[derive(SurrealValue)]
struct MetadataSize {
    key: String,
    encoded_bytes: i64,
}
/// Exact encoded wire bounds without fetching metadata text. Used before
/// acquiring a claim batch so large valid metadata is processed in smaller
/// groups instead of repeatedly failing an oversized batch.
pub(crate) async fn metadata_wire_bounds(
    db: &Surreal<Any>,
    keys: &[crate::atlas_classify::OccurrenceKey],
) -> Result<
    std::collections::BTreeMap<crate::atlas_classify::OccurrenceKey, usize>,
    crate::storage::classifier_admission::AdmissionError,
> {
    use super::classifier_admission::{AdmissionError, MAX_RESPONSE_BYTES};
    if keys.len() > 512 {
        return Err(AdmissionError::InvalidPage {
            code: "metadata_key_limit".into(),
        });
    }
    let mut result = std::collections::BTreeMap::new();
    for keys in keys.chunks(64) {
        let identities: std::collections::BTreeMap<_, _> = keys
            .iter()
            .map(|key| (key.record_id(), key.clone()))
            .collect();
        let records: Vec<_> = identities
            .keys()
            .map(|key| super::jobs::rid("atlas_article_metadata", key))
            .collect();
        let mut response=super::classifier_admission::db_query(db,"SELECT record::id(id) AS key,string::len(encoding::base64::encode(type::bytes(metadata_json),true)) AS encoded_bytes FROM atlas_article_metadata WHERE id INSIDE $records").bind(("records",records)).await.map_err(TxError::from)?.check().map_err(TxError::from)?;
        let rows: Vec<MetadataSize> = response.take(0).map_err(TxError::from)?;
        for row in rows {
            let key = identities
                .get(&row.key)
                .ok_or_else(|| AdmissionError::InvalidReceipt {
                    code: "metadata_header_correlation".into(),
                })?
                .clone();
            let bytes = usize::try_from(row.encoded_bytes)
                .ok()
                .and_then(|size| size.checked_add(1024))
                .ok_or_else(|| AdmissionError::PayloadTooLarge {
                    kind: "metadata_response".into(),
                    limit: MAX_RESPONSE_BYTES,
                })?;
            if bytes > MAX_RESPONSE_BYTES {
                return Err(AdmissionError::PayloadTooLarge {
                    kind: "metadata_response".into(),
                    limit: MAX_RESPONSE_BYTES,
                });
            }
            if result.insert(key, bytes).is_some() {
                return Err(AdmissionError::InvalidReceipt {
                    code: "duplicate_metadata_header".into(),
                });
            }
        }
    }
    Ok(result)
}

/// Bulk metadata inputs, partitioned by encoded wire bytes before payload fetch.
pub async fn metadata_many(
    db: &Surreal<Any>,
    keys: &[crate::atlas_classify::OccurrenceKey],
) -> Result<
    std::collections::BTreeMap<crate::atlas_classify::OccurrenceKey, ArticleMetadata>,
    crate::storage::classifier_admission::AdmissionError,
> {
    metadata_many_read(MetadataReader::Db(db), keys).await
}
pub(crate) async fn metadata_many_in(
    tx: &Tx,
    keys: &[crate::atlas_classify::OccurrenceKey],
) -> Result<
    std::collections::BTreeMap<crate::atlas_classify::OccurrenceKey, ArticleMetadata>,
    TxError,
> {
    metadata_many_read(MetadataReader::Tx(tx), keys)
        .await
        .map_err(|error| match error {
            crate::storage::classifier_admission::AdmissionError::Storage(error) => error,
            _ => TxError::Abort("invalid_or_oversized_bulk_metadata".into()),
        })
}
enum MetadataReader<'a> {
    Db(&'a Surreal<Any>),
    Tx(&'a Tx),
}
impl MetadataReader<'_> {
    fn query(&self, sql: &str) -> super::classifier_admission::MeasuredQuery<'_> {
        match self {
            Self::Db(db) => super::classifier_admission::db_query(db, sql.to_owned()),
            Self::Tx(tx) => super::classifier_admission::tx_query(tx, sql.to_owned()),
        }
    }
}
async fn metadata_many_read(
    reader: MetadataReader<'_>,
    keys: &[crate::atlas_classify::OccurrenceKey],
) -> Result<
    std::collections::BTreeMap<crate::atlas_classify::OccurrenceKey, ArticleMetadata>,
    crate::storage::classifier_admission::AdmissionError,
> {
    use crate::storage::classifier_admission::{self, AdmissionError, MAX_RESPONSE_BYTES};
    if keys.len() > 512 {
        return Err(AdmissionError::InvalidPage {
            code: "metadata_key_limit".into(),
        });
    }
    let mut result = std::collections::BTreeMap::new();
    let mut total_bound = 0usize;
    for keys in keys.chunks(64) {
        let identities: std::collections::BTreeMap<_, _> = keys
            .iter()
            .map(|key| (encode(&[&key.run_id, &key.article_id]), key.clone()))
            .collect();
        let records: Vec<_> = identities
            .keys()
            .map(|key| super::jobs::rid("atlas_article_metadata", key))
            .collect();
        // Casting the string to bytes measures UTF-8 bytes exactly. Base64 is
        // ASCII without JSON escaping, so its length bounds the wire response.
        let mut response=reader.query("SELECT record::id(id) AS key,string::len(encoding::base64::encode(type::bytes(metadata_json),true)) AS encoded_bytes FROM atlas_article_metadata WHERE id INSIDE $records")
            .bind(("records",records)).await.map_err(TxError::from)?.check().map_err(TxError::from)?;
        let sizes: Vec<MetadataSize> = response.take(0).map_err(TxError::from)?;
        let mut groups = Vec::new();
        let mut group = Vec::new();
        let mut bytes = 0usize;
        let mut seen = std::collections::BTreeSet::new();
        for size in sizes {
            let key = identities
                .get(&size.key)
                .ok_or_else(|| AdmissionError::InvalidReceipt {
                    code: "bulk_metadata_correlation".into(),
                })?
                .clone();
            if !seen.insert(size.key) {
                return Err(AdmissionError::InvalidReceipt {
                    code: "bulk_metadata_duplicate".into(),
                });
            }
            let encoded = usize::try_from(size.encoded_bytes).map_err(|_| {
                AdmissionError::InvalidReceipt {
                    code: "metadata_size_invalid".into(),
                }
            })?;
            let bound =
                encoded
                    .checked_add(1024)
                    .ok_or_else(|| AdmissionError::PayloadTooLarge {
                        kind: "metadata_response".into(),
                        limit: MAX_RESPONSE_BYTES,
                    })?;
            total_bound =
                total_bound
                    .checked_add(bound)
                    .ok_or_else(|| AdmissionError::PayloadTooLarge {
                        kind: "metadata_response".into(),
                        limit: MAX_RESPONSE_BYTES,
                    })?;
            if total_bound > MAX_RESPONSE_BYTES {
                return Err(AdmissionError::PayloadTooLarge {
                    kind: "metadata_response".into(),
                    limit: MAX_RESPONSE_BYTES,
                });
            }
            if bytes + bound > MAX_RESPONSE_BYTES && !group.is_empty() {
                groups.push(group);
                group = Vec::new();
                bytes = 0;
            }
            bytes += bound;
            group.push((key, encoded));
        }
        if !group.is_empty() {
            groups.push(group);
        }
        for group in groups {
            // Every field is guarded by its observed header bound. Concurrent
            // growth returns a retry diagnostic rather than allocating it.
            // The statements are issued together as one bounded driver batch.
            let mut sql = String::new();
            for index in 0..group.len() {
                sql.push_str(&format!("SELECT VALUE IF string::len(encoding::base64::encode(type::bytes(metadata_json),true)) <= $cap{index} {{encoding::base64::encode(type::bytes(metadata_json),true)}} ELSE {{NONE}} FROM ONLY $record{index};"));
            }
            let mut query = reader.query(&sql);
            for (index, (key, cap)) in group.iter().enumerate() {
                query = query
                    .bind((
                        format!("record{index}"),
                        super::jobs::rid("atlas_article_metadata", &key.record_id()),
                    ))
                    .bind((format!("cap{index}"), *cap as i64));
            }
            let mut response = query
                .await
                .map_err(TxError::from)?
                .check()
                .map_err(TxError::from)?;
            for (index, (key, cap)) in group.into_iter().enumerate() {
                let encoded = response
                    .take::<Option<String>>(index)
                    .map_err(TxError::from)?
                    .ok_or_else(|| AdmissionError::InvalidReceipt {
                        code: "bulk_metadata_changed".into(),
                    })?;
                if encoded.len() > cap {
                    return Err(AdmissionError::InvalidReceipt {
                        code: "bulk_metadata_bound".into(),
                    });
                }
                let bytes = classifier_admission::decode_wire_json(&encoded, MAX_RESPONSE_BYTES)?;
                let metadata: ArticleMetadata =
                    serde_json::from_slice(&bytes).map_err(|_| AdmissionError::InvalidReceipt {
                        code: "bulk_metadata_json".into(),
                    })?;
                if metadata.run_id != key.run_id
                    || metadata.article_id != key.article_id
                    || result.insert(key, metadata).is_some()
                {
                    return Err(AdmissionError::InvalidReceipt {
                        code: "bulk_metadata_identity".into(),
                    });
                }
            }
        }
    }
    Ok(result)
}

/// Bounded original-metadata capture for historical requests. Diagnostics are
/// item dispositions; malformed or oversized input never poisons valid siblings.
#[derive(Debug, Clone)]
pub enum OriginalMetadata {
    Missing,
    Present(Box<ArticleMetadata>),
    Oversized,
    Invalid,
}
pub async fn metadata_originals_many_in(
    tx: &Tx,
    keys: &[crate::atlas_classify::OccurrenceKey],
) -> Result<
    std::collections::BTreeMap<crate::atlas_classify::OccurrenceKey, OriginalMetadata>,
    TxError,
> {
    if keys.len() > 64 {
        return Err(TxError::Abort("original_metadata_batch_limit".into()));
    }
    #[derive(SurrealValue)]
    struct Wire {
        bytes: i64,
        metadata64: Option<String>,
    }
    let mut sql = String::new();
    for index in 0..keys.len() {
        sql.push_str(&format!("SELECT bytes::len(type::bytes(metadata_json)) AS bytes,IF bytes::len(type::bytes(metadata_json))<=131072 {{encoding::base64::encode(type::bytes(metadata_json),true)}} ELSE {{NONE}} AS metadata64 FROM ONLY $key{index};"));
    }
    if keys.is_empty() {
        return Ok(std::collections::BTreeMap::new());
    }
    let mut query = super::classifier_admission::tx_query(tx, sql);
    for (index, key) in keys.iter().enumerate() {
        query = query.bind((
            format!("key{index}"),
            super::jobs::rid("atlas_article_metadata", &key.record_id()),
        ));
    }
    let mut response = query.await?.check()?;
    let mut out = std::collections::BTreeMap::new();
    for (index, key) in keys.iter().enumerate() {
        let value = match response.take::<Option<Wire>>(index)? {
            None => OriginalMetadata::Missing,
            Some(wire) if wire.bytes > 131072 => OriginalMetadata::Oversized,
            Some(wire) => match wire
                .metadata64
                .and_then(|wire| super::classifier_admission::decode_wire_json(&wire, 131072).ok())
                .and_then(|bytes| serde_json::from_slice::<ArticleMetadata>(&bytes).ok())
            {
                Some(metadata)
                    if metadata.run_id == key.run_id && metadata.article_id == key.article_id =>
                {
                    OriginalMetadata::Present(Box::new(metadata))
                }
                _ => OriginalMetadata::Invalid,
            },
        };
        out.insert(key.clone(), value);
    }
    Ok(out)
}

/// Header-only wire costs. An oversized/invalid item is represented separately
/// so valid siblings can still be hydrated and committed in source order.
pub(crate) async fn metadata_wire_costs(
    db: &Surreal<Any>,
    keys: &[crate::atlas_classify::OccurrenceKey],
) -> Result<std::collections::BTreeMap<crate::atlas_classify::OccurrenceKey, Option<usize>>, TxError>
{
    if keys.len() > 512 {
        return Err(TxError::Abort("metadata_cost_key_limit".into()));
    }
    let mut out = std::collections::BTreeMap::new();
    for keys in keys.chunks(64) {
        let identities: std::collections::BTreeMap<_, _> = keys
            .iter()
            .map(|key| (key.record_id(), key.clone()))
            .collect();
        let mut response=super::classifier_admission::db_query(db,"SELECT record::id(id) AS key,string::len(encoding::base64::encode(type::bytes(metadata_json),true)) AS encoded_bytes FROM $records").bind(("records",identities.keys().map(|key|super::jobs::rid("atlas_article_metadata",key)).collect::<Vec<_>>())).await?.check()?;
        let sizes: Vec<MetadataSize> = response.take(0)?;
        for size in sizes {
            let key = identities
                .get(&size.key)
                .ok_or_else(|| TxError::Abort("metadata_cost_correlation".into()))?;
            let cost = usize::try_from(size.encoded_bytes)
                .ok()
                .and_then(|bytes| bytes.checked_add(1024))
                .filter(|bytes| *bytes <= super::classifier_admission::MAX_RESPONSE_BYTES);
            if out.insert(key.clone(), cost).is_some() {
                return Err(TxError::Abort("duplicate_metadata_cost".into()));
            }
        }
    }
    Ok(out)
}
