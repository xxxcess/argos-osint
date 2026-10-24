//! NLP annotation cache on the repository: the SurrealDB side of SQLite's
//! `nlp_features` (`nlp::cache::{get, put}`), plus the artifact reads and
//! the source-ref selection update `nlp::record_view` and
//! `nlp::expanded_observations` need. Artifacts and source refs themselves
//! live in `pipeline_inputs` (`nlp_artifact`, `nlp_source_ref`).
//!
//! Record keys: `nlp_feature` by feature key. The cache is bounded like
//! SQLite's: entries past [`MAX_BYTES`] are evicted oldest `last_access`
//! first.

use anyhow::Result;
use surrealdb::types::SurrealValue;

use super::jobs::rid;
use super::repo::Repository;
use crate::nlp::cache::{feature_key, hash};
use crate::nlp::{Annotation, ParseContext, Source, Status};

/// Byte cap of the feature cache (as SQLite: 256 MiB).
pub const MAX_BYTES: i64 = 256 * 1024 * 1024;

/// Feature cache schema (idempotent). Register as `("nlp_cache", SCHEMA)`
/// after `pipeline_inputs`.
pub const SCHEMA: &str = "DEFINE TABLE IF NOT EXISTS nlp_feature SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS prose_hash ON nlp_feature TYPE string;
DEFINE FIELD IF NOT EXISTS version_context_json ON nlp_feature TYPE string;
DEFINE FIELD IF NOT EXISTS annotations_json ON nlp_feature TYPE string;
DEFINE FIELD IF NOT EXISTS byte_len ON nlp_feature TYPE int;
DEFINE FIELD IF NOT EXISTS last_access ON nlp_feature TYPE string;
DEFINE INDEX IF NOT EXISTS nlp_feature_lru ON nlp_feature FIELDS last_access;
DEFINE INDEX IF NOT EXISTS nlp_feature_prose ON nlp_feature FIELDS prose_hash;";

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// `nlp::cache::get`: a complete, valid cached annotation for `source`
/// under `context`, touching its `last_access`.
pub async fn get(
    repo: &Repository,
    source: &Source,
    context: &ParseContext,
) -> Result<Option<Annotation>> {
    let key = feature_key(source, context);
    let mut r = repo
        .db()
        .query("SELECT VALUE annotations_json FROM ONLY $f")
        .bind(("f", rid("nlp_feature", &key)))
        .await?
        .check()?;
    let Some(json) = r.take::<Option<String>>(0)? else {
        return Ok(None);
    };
    let a: Annotation = serde_json::from_str(&json)?;
    if a.status != Status::Complete || a.feature_key != key || !a.validate(source) {
        return Ok(None);
    }
    repo.db()
        .query("UPDATE $f SET last_access = $now RETURN NONE")
        .bind(("f", rid("nlp_feature", &key)))
        .bind(("now", now()))
        .await?
        .check()?;
    Ok(Some(a))
}

/// `nlp::cache::put`: store a complete annotation, then evict least
/// recently used entries until the cache is within [`MAX_BYTES`].
pub async fn put(
    repo: &Repository,
    source: &Source,
    context: &ParseContext,
    a: &Annotation,
) -> Result<()> {
    anyhow::ensure!(
        a.status == Status::Complete
            && a.validate(source)
            && a.feature_key == feature_key(source, context),
        "invalid NLP cache entry"
    );
    let json = serde_json::to_string(a)?;
    let version = serde_json::json!({"context":context,"schema":crate::nlp::SCHEMA,
        "bridge_hash":hash(include_str!("../nlp/bridge.js")),
        "policy":"paragraph-4096-8192-overlap-v1"})
    .to_string();
    repo.db()
        .query(
            "UPSERT $f SET prose_hash = $ph, version_context_json = $vc,
               annotations_json = $aj, byte_len = $len, last_access = $now RETURN NONE",
        )
        .bind(("f", rid("nlp_feature", &a.feature_key)))
        .bind(("ph", a.prose_hash.clone()))
        .bind(("vc", version))
        .bind(("len", json.len() as i64))
        .bind(("aj", json))
        .bind(("now", now()))
        .await?
        .check()?;
    evict(repo).await
}

#[derive(SurrealValue)]
struct LruRow {
    id: String,
    byte_len: i64,
}

/// Drop least recently used features until the total is within the cap.
async fn evict(repo: &Repository) -> Result<()> {
    let mut r = repo
        .db()
        .query("RETURN math::sum((SELECT VALUE byte_len FROM nlp_feature))")
        .await?
        .check()?;
    let mut total = r.take::<Option<i64>>(0)?.unwrap_or(0);
    if total <= MAX_BYTES {
        return Ok(());
    }
    let mut r = repo
        .db()
        .query(
            "SELECT meta::id(id) AS id, byte_len FROM nlp_feature
             ORDER BY last_access, id",
        )
        .await?
        .check()?;
    let rows: Vec<LruRow> = r.take(0)?;
    let mut doomed = Vec::new();
    for row in rows {
        if total <= MAX_BYTES {
            break;
        }
        total -= row.byte_len;
        doomed.push(rid("nlp_feature", &row.id));
    }
    repo.db()
        .query("FOR $f IN $doomed { DELETE $f RETURN NONE; };")
        .bind(("doomed", doomed))
        .await?
        .check()?;
    Ok(())
}

/// `nlp::cache::gc` for features: drop cached annotations whose prose no
/// source ref references any more. Artifact collection is
/// `insight::remove_nlp_owner_in`'s.
pub async fn gc_features(repo: &Repository) -> Result<()> {
    repo.db()
        .query(
            "LET $live = array::distinct((SELECT VALUE prose_hash FROM nlp_source_ref));
             DELETE nlp_feature WHERE prose_hash NOTINSIDE $live RETURN NONE;",
        )
        .await?
        .check()?;
    Ok(())
}

/// The retained text of artifact `hash` when it is at most `max_len`
/// bytes and still hashes to `hash` (`nlp::expanded_observations`).
pub async fn artifact_text(repo: &Repository, h: &str, max_len: usize) -> Result<Option<String>> {
    let mut r = repo
        .db()
        .query("SELECT VALUE text FROM ONLY $a WHERE byte_len <= $max")
        .bind(("a", rid("nlp_artifact", h)))
        .bind(("max", max_len as i64))
        .await?
        .check()?;
    let text: Option<String> = r.take(0)?;
    Ok(text.filter(|t| hash(t) == h))
}

/// `nlp::record_view`: register the historical derived prose for an owner
/// and record the prepared selection on its source ref (SQLite's
/// `json_set(metadata_json,'$.selection',…)`).
pub async fn record_view(
    repo: &Repository,
    owner_kind: &str,
    owner_id: &str,
    source: &Source,
    selection: serde_json::Value,
) -> Result<()> {
    const TRANSFORM: &str = "historical-prose-v1";
    super::pipeline_inputs::register(repo, owner_kind, owner_id, source, None, TRANSFORM).await?;
    #[derive(SurrealValue)]
    struct RefRow {
        id: String,
        metadata_json: String,
    }
    let mut r = repo
        .db()
        .query(
            "SELECT meta::id(id) AS id, metadata_json FROM nlp_source_ref
             WHERE owner_kind = $ok AND owner_id = $oi AND prose_hash = $ph
               AND transform_version = $tr",
        )
        .bind(("ok", owner_kind.to_string()))
        .bind(("oi", owner_id.to_string()))
        .bind(("ph", source.prose_hash.clone()))
        .bind(("tr", TRANSFORM.to_string()))
        .await?
        .check()?;
    let rows: Vec<RefRow> = r.take(0)?;
    for row in rows {
        let mut meta: serde_json::Value =
            serde_json::from_str(&row.metadata_json).unwrap_or_else(|_| serde_json::json!({}));
        if let Some(obj) = meta.as_object_mut() {
            obj.insert("selection".into(), selection.clone());
        }
        repo.db()
            .query("UPDATE $r SET metadata_json = $m RETURN NONE")
            .bind(("r", rid("nlp_source_ref", &row.id)))
            .bind(("m", meta.to_string()))
            .await?
            .check()?;
    }
    Ok(())
}

/// `nlp::capture_page`: retain a fetched page's text as an artifact.
pub async fn capture(repo: &Repository, text: &str) -> Result<String> {
    let text = text.to_string();
    let h = super::tx::run_tx(repo.db(), super::tx::RetryPolicy::default(), |tx| {
        let text = text.clone();
        async move { super::pipeline_inputs::artifact_in(&tx, &text).await }
    })
    .await?;
    Ok(h.value)
}
