//! Insight pipeline inputs behind the repository (migration phase 4c):
//! article bodies, the content-addressed NLP artifact store and source refs
//! (`nlp_artifacts`, `nlp_source_refs`), the acquisition-coverage app state,
//! and the article and Recon `PipelineInput` builders that read them.
//!
//! Record keys: `article_body` by body id, `nlp_artifact` by hash,
//! `nlp_source_ref` by ref key, `app_state` by key.
//!
//! Gaps: `article_input` has no tool evidence (`osint_calls` and
//! `intel_evidence` are not on SurrealDB), and `recon_input` takes evidence
//! the caller built from tool results (`evidence_from_tools` still reads the
//! legacy store NLP cache); its texts are retained here as artifacts. The legacy store's
//! "newest rowid" source ref is the newest `registered_at`.

use anyhow::{ensure, Result};
use surrealdb::types::SurrealValue;

use super::jobs::{rid, run};
use super::repo::Repository;
use super::tx::{run_tx, RetryPolicy, Tx, TxError};
use crate::intel_recon::ArticleBodyRow;
use crate::investigation::insights::{
    snapshot, EvidenceLocation, EvidenceRef, PipelineInput, RetainedEvidence,
};
use crate::nlp::cache::{hash, key};
use crate::nlp::{Source, Span};
use crate::store::AtlasArticleRow;

/// Pipeline input schema (idempotent).
pub const SCHEMA: &str = "DEFINE TABLE IF NOT EXISTS article_body SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS article_id ON article_body TYPE string;
DEFINE FIELD IF NOT EXISTS run_id ON article_body TYPE string;
DEFINE FIELD IF NOT EXISTS original_url ON article_body TYPE string;
DEFINE FIELD IF NOT EXISTS resolved_url ON article_body TYPE string;
DEFINE FIELD IF NOT EXISTS source_domain ON article_body TYPE string;
DEFINE FIELD IF NOT EXISTS source_name ON article_body TYPE string;
DEFINE FIELD IF NOT EXISTS body_markdown ON article_body TYPE string;
DEFINE FIELD IF NOT EXISTS content_hash ON article_body TYPE string;
DEFINE FIELD IF NOT EXISTS quality ON article_body TYPE string;
DEFINE FIELD IF NOT EXISTS quality_rationale ON article_body TYPE string;
DEFINE FIELD IF NOT EXISTS body_version ON article_body TYPE int;
DEFINE FIELD IF NOT EXISTS fetch_tool ON article_body TYPE string;
DEFINE FIELD IF NOT EXISTS fetched_at ON article_body TYPE string;
DEFINE FIELD IF NOT EXISTS retry_after ON article_body TYPE string;
DEFINE FIELD IF NOT EXISTS state ON article_body TYPE string;
DEFINE FIELD IF NOT EXISTS created_at ON article_body TYPE string;
DEFINE FIELD IF NOT EXISTS updated_at ON article_body TYPE string;
DEFINE INDEX IF NOT EXISTS article_body_article ON article_body FIELDS article_id;
DEFINE TABLE IF NOT EXISTS nlp_artifact SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS text ON nlp_artifact TYPE string;
DEFINE FIELD IF NOT EXISTS byte_len ON nlp_artifact TYPE int;
DEFINE FIELD IF NOT EXISTS created_at ON nlp_artifact TYPE string;
DEFINE TABLE IF NOT EXISTS nlp_source_ref SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS owner_kind ON nlp_source_ref TYPE string;
DEFINE FIELD IF NOT EXISTS owner_id ON nlp_source_ref TYPE string;
DEFINE FIELD IF NOT EXISTS source_id ON nlp_source_ref TYPE string;
DEFINE FIELD IF NOT EXISTS revision ON nlp_source_ref TYPE string;
DEFINE FIELD IF NOT EXISTS attempt_id ON nlp_source_ref TYPE string;
DEFINE FIELD IF NOT EXISTS raw_hash ON nlp_source_ref TYPE option<string>;
DEFINE FIELD IF NOT EXISTS prose_hash ON nlp_source_ref TYPE string;
DEFINE FIELD IF NOT EXISTS metadata_json ON nlp_source_ref TYPE string;
DEFINE FIELD IF NOT EXISTS transform_version ON nlp_source_ref TYPE string;
DEFINE FIELD IF NOT EXISTS registered_at ON nlp_source_ref TYPE datetime DEFAULT time::now();
DEFINE INDEX IF NOT EXISTS nlp_ref_owner ON nlp_source_ref FIELDS owner_kind, owner_id;
DEFINE TABLE IF NOT EXISTS app_state SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS value ON app_state TYPE string;";

/// Retain `text` in the artifact store (idempotent); returns its hash.
pub async fn artifact_in(tx: &Tx, text: &str) -> Result<String, TxError> {
    let h = hash(text);
    run(
        tx,
        tx.query(
            "IF !record::exists($a) {
               CREATE $a SET text = $text, byte_len = $len, created_at = $now RETURN NONE;
             };",
        )
        .bind(("a", rid("nlp_artifact", &h)))
        .bind(("text", text.to_string()))
        .bind(("len", text.len() as i64))
        .bind(("now", chrono::Utc::now().to_rfc3339())),
    )
    .await?;
    Ok(h)
}

/// Register a source's prose (and raw acquisition) for an owner, as
/// `nlp::cache::register` does.
pub async fn register_in(
    tx: &Tx,
    owner_kind: &str,
    owner_id: &str,
    source: &Source,
    raw: Option<&str>,
    transform: &str,
) -> Result<(), TxError> {
    if hash(&source.prose) != source.prose_hash {
        return Err(TxError::Abort("prose hash mismatch".into()));
    }
    let raw_hash = match raw {
        Some(raw) => Some(artifact_in(tx, raw).await?),
        None => None,
    };
    if raw_hash != source.raw_hash {
        return Err(TxError::Abort("raw provenance mismatch".into()));
    }
    artifact_in(tx, &source.prose).await?;
    let ref_key = key(&serde_json::json!([
        owner_kind,
        owner_id,
        source.source_id,
        source.revision,
        source.attempt_id,
        raw_hash,
        source.prose_hash,
        transform
    ]));
    let metadata = serde_json::json!({"language":source.language_hint,"published_at":source.published_at,
        "retrieved_at":source.retrieved_at,"historical_derived":raw.is_none()});
    run(
        tx,
        tx.query(
            "IF !record::exists($r) {
               CREATE $r SET owner_kind = $ok, owner_id = $oi, source_id = $sid, revision = $rev,
                 attempt_id = $att, raw_hash = $raw, prose_hash = $prose, metadata_json = $meta,
                 transform_version = $tr RETURN NONE;
             };",
        )
        .bind(("r", rid("nlp_source_ref", &ref_key)))
        .bind(("ok", owner_kind.to_string()))
        .bind(("oi", owner_id.to_string()))
        .bind(("sid", source.source_id.clone()))
        .bind(("rev", source.revision.clone()))
        .bind(("att", source.attempt_id.clone()))
        .bind(("raw", raw_hash))
        .bind(("prose", source.prose_hash.clone()))
        .bind(("meta", metadata.to_string()))
        .bind(("tr", transform.to_string())),
    )
    .await?;
    Ok(())
}

pub async fn register(
    repo: &Repository,
    owner_kind: &str,
    owner_id: &str,
    source: &Source,
    raw: Option<&str>,
    transform: &str,
) -> Result<()> {
    let (ok, oi, tr) = (
        owner_kind.to_string(),
        owner_id.to_string(),
        transform.to_string(),
    );
    let raw = raw.map(str::to_string);
    run_tx(repo.db(), RetryPolicy::default(), |tx| {
        let (ok, oi, tr, raw, source) = (
            ok.clone(),
            oi.clone(),
            tr.clone(),
            raw.clone(),
            source.clone(),
        );
        async move { register_in(&tx, &ok, &oi, &source, raw.as_deref(), &tr).await }
    })
    .await?;
    Ok(())
}

pub async fn app_state_set(repo: &Repository, key: &str, value: &str) -> Result<()> {
    repo.db()
        .query("UPSERT $k SET value = $v RETURN NONE")
        .bind(("k", rid("app_state", key)))
        .bind(("v", value.to_string()))
        .await?
        .check()?;
    Ok(())
}

/// `DELETE FROM app_state WHERE key=?1`.
pub async fn app_state_delete(repo: &Repository, key: &str) -> Result<()> {
    if key.is_empty() {
        return Ok(());
    }
    repo.db()
        .query("DELETE $k RETURN NONE")
        .bind(("k", rid("app_state", key)))
        .await?
        .check()?;
    Ok(())
}

pub async fn app_state_get(repo: &Repository, key: &str) -> Result<Option<String>> {
    let mut r = repo
        .db()
        .query("SELECT VALUE value FROM ONLY $k")
        .bind(("k", rid("app_state", key)))
        .await?
        .check()?;
    Ok(r.take::<Option<String>>(0)?)
}

#[derive(SurrealValue)]
struct BodyRow {
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

pub async fn put_article_body(repo: &Repository, b: &ArticleBodyRow) -> Result<()> {
    repo.db()
        .query(
            "UPSERT $b SET article_id = $a, run_id = $run, original_url = $ou, resolved_url = $ru,
               source_domain = $sd, source_name = $sn, body_markdown = $md, content_hash = $ch,
               quality = $q, quality_rationale = $qr, body_version = $v, fetch_tool = $ft,
               fetched_at = $fa, retry_after = $ra, state = $st, created_at = $c,
               updated_at = $u RETURN NONE",
        )
        .bind(("b", rid("article_body", &b.id)))
        .bind(("a", b.article_id.clone()))
        .bind(("run", b.run_id.clone()))
        .bind(("ou", b.original_url.clone()))
        .bind(("ru", b.resolved_url.clone()))
        .bind(("sd", b.source_domain.clone()))
        .bind(("sn", b.source_name.clone()))
        .bind(("md", b.body_markdown.clone()))
        .bind(("ch", b.content_hash.clone()))
        .bind(("q", b.quality.clone()))
        .bind(("qr", b.quality_rationale.clone()))
        .bind(("v", b.body_version))
        .bind(("ft", b.fetch_tool.clone()))
        .bind(("fa", b.fetched_at.clone()))
        .bind(("ra", b.retry_after.clone()))
        .bind(("st", b.state.clone()))
        .bind(("c", b.created_at.clone()))
        .bind(("u", b.updated_at.clone()))
        .await?
        .check()?;
    Ok(())
}

/// The article's newest body (highest version, then latest update).
pub async fn article_body_for_article(
    repo: &Repository,
    article_id: &str,
) -> Result<Option<ArticleBodyRow>> {
    let mut r = repo
        .db()
        .query(
            "SELECT *, meta::id(id) AS id FROM article_body WHERE article_id = $a
               ORDER BY body_version DESC, updated_at DESC LIMIT 1",
        )
        .bind(("a", article_id.to_string()))
        .await?
        .check()?;
    let rows: Vec<BodyRow> = r.take(0)?;
    Ok(rows.into_iter().next().map(|b| ArticleBodyRow {
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

#[derive(SurrealValue)]
struct Acquisition {
    raw_hash: String,
    prose_hash: String,
    attempt_id: String,
    revision: String,
}

/// Port of `insight_persist::article_input` (without tool evidence): the
/// retained body acquisition (clean artifact, else validated baseline) and
/// the title/description passage, with metadata actors as snapshot
/// entities.
pub async fn article_input(
    repo: &Repository,
    article: &AtlasArticleRow,
    generation: u32,
    objective: &str,
) -> Result<PipelineInput> {
    let db = repo.db();
    let mut evidence = Vec::new();
    if let Some(body) = article_body_for_article(repo, &article.id).await? {
        let mut r = db
            .query(
                "SELECT raw_hash, prose_hash, attempt_id, revision FROM nlp_source_ref
                   WHERE owner_kind = 'article' AND owner_id = $a AND source_id = $s
                     AND transform_version = 'validated-baseline-v1' AND revision = $rev
                     AND raw_hash != NONE ORDER BY registered_at DESC LIMIT 1",
            )
            .bind(("a", article.id.clone()))
            .bind(("s", body.id.clone()))
            .bind(("rev", body.body_version.to_string()))
            .await?
            .check()?;
        let acquisition: Option<Acquisition> = r.take::<Vec<Acquisition>>(0)?.into_iter().next();
        if let Some(acq) = acquisition {
            let mut r = db
                .query(
                    "SELECT VALUE prose_hash FROM nlp_source_ref WHERE owner_kind = 'article'
                       AND owner_id = $a AND source_id = $s AND transform_version = $t
                       AND revision = $rev AND attempt_id = $att AND raw_hash = $raw
                       ORDER BY registered_at DESC LIMIT 1",
                )
                .bind(("a", article.id.clone()))
                .bind(("s", body.id.clone()))
                .bind(("t", crate::intel_recon::clean_body::TRANSFORM))
                .bind(("rev", acq.revision.clone()))
                .bind(("att", acq.attempt_id.clone()))
                .bind(("raw", acq.raw_hash.clone()))
                .await?
                .check()?;
            let prose_hash = r
                .take::<Vec<String>>(0)?
                .into_iter()
                .next()
                .unwrap_or(acq.prose_hash);
            let mut r = db
                .query("SELECT VALUE text FROM ONLY $a")
                .bind(("a", rid("nlp_artifact", &prose_hash)))
                .await?
                .check()?;
            if let Some(text) = r.take::<Option<String>>(0)? {
                let quality =
                    crate::intel_recon::validate_article_body(&text, &article.title, &article.url);
                let coverage =
                    app_state_get(repo, &format!("article-acquisition:{}", acq.attempt_id))
                        .await?
                        .and_then(|json| serde_json::from_str::<serde_json::Value>(&json).ok());
                let complete = quality.quality == crate::intel_recon::BodyQuality::Complete
                    && coverage.as_ref().is_some_and(|c| {
                        c["artifact_hash"].as_str() == Some(acq.raw_hash.as_str())
                            && c["complete_story"].as_bool() == Some(true)
                            && c["known_truncation"].as_bool() == Some(false)
                    });
                evidence.push(RetainedEvidence {
                    reference: EvidenceRef {
                        source_id: format!("body:{}", body.id),
                        revision: acq.revision,
                        artifact_hash: prose_hash,
                        acquisition_id: acq.attempt_id,
                        origin_root: format!("acquisition:{}", acq.raw_hash),
                        url: article.url.clone(),
                        published_at: Some(article.published_at.clone()),
                        retrieved_at: body.fetched_at.clone(),
                        location: EvidenceLocation::Range(Span {
                            start_byte: 0,
                            end_byte: text.len(),
                        }),
                    },
                    text,
                    native: None,
                    substantive: true,
                    relevant: true,
                    complete_story: complete,
                    known_truncation: !complete,
                });
            }
        }
    }
    // Metadata is an independent retained passage with its own offsets.
    let text = format!("{}\n{}", article.title, article.description);
    let artifact = {
        let text = text.clone();
        run_tx(db, RetryPolicy::default(), |tx| {
            let text = text.clone();
            async move { artifact_in(&tx, &text).await }
        })
        .await?
        .value
    };
    evidence.push(RetainedEvidence {
        reference: EvidenceRef {
            source_id: format!("article:{}", article.id),
            revision: artifact.clone(),
            artifact_hash: artifact,
            acquisition_id: article.id.clone(),
            origin_root: String::new(),
            url: article.url.clone(),
            published_at: Some(article.published_at.clone()),
            retrieved_at: article.seen_at.clone(),
            location: EvidenceLocation::Range(Span {
                start_byte: 0,
                end_byte: text.len(),
            }),
        },
        text,
        native: None,
        substantive: false,
        relevant: true,
        complete_story: false,
        known_truncation: true,
    });
    let mut input = PipelineInput {
        snapshot: snapshot("article", &article.id, generation, &evidence),
        evidence,
        objective: objective.into(),
        surface: "article".into(),
        required_tasks_complete: true,
        save_insights: true,
        recall_enabled: false,
    };
    if let Some(metadata) =
        super::atlas_metadata::metadata(db, &article.run_id, &article.id).await?
    {
        for actor in crate::atlas_metadata::article_actors(article, &metadata) {
            input.snapshot.entities.push(crate::nlp::EntityCandidate {
                id: actor.id,
                type_hint: actor.type_hint,
                comparison_key: actor.display_name.to_lowercase(),
                original_forms: actor.spellings,
                mention_refs: actor
                    .mentions
                    .iter()
                    .map(|m| {
                        let offset = if m.field == "description" {
                            article.title.len() + 1
                        } else {
                            0
                        };
                        Span {
                            start_byte: offset + m.start_byte,
                            end_byte: offset + m.end_byte,
                        }
                    })
                    .collect(),
            });
        }
    }
    input.required_tasks_complete =
        !article.title.trim().is_empty() && !article.description.trim().is_empty();
    Ok(input)
}

/// The Recon run's input: caller-built tool evidence (retained here as
/// artifacts), the owner's desired generation (advanced from 0), and the
/// thread's live insight opt-in for saving and recall.
pub async fn recon_input(
    repo: &Repository,
    run_id: &str,
    thread_id: &str,
    question: &str,
    evidence: Vec<RetainedEvidence>,
    required_tasks_complete: bool,
) -> Result<PipelineInput> {
    let thread = super::recon::get_thread(repo, thread_id).await?;
    let opted_in = thread.is_some_and(|t| t.recall_insights);
    let texts: Vec<String> = evidence.iter().map(|e| e.text.clone()).collect();
    let run = run_id.to_string();
    let generation = run_tx(repo.db(), RetryPolicy::default(), |tx| {
        let (texts, run) = (texts.clone(), run.clone());
        async move {
            for text in &texts {
                artifact_in(&tx, text).await?;
            }
            let g = super::insight::desired_generation(&tx, "recon", &run).await?;
            if g == 0 {
                let now = chrono::Utc::now().to_rfc3339();
                super::insight::advance_generation(&tx, "recon", &run, &now).await
            } else {
                Ok(g)
            }
        }
    })
    .await?
    .value;
    ensure!(generation > 0, "recon generation was not advanced");
    Ok(PipelineInput {
        snapshot: snapshot("recon", run_id, generation as u32, &evidence),
        evidence,
        objective: question.into(),
        surface: "recon".into(),
        required_tasks_complete,
        save_insights: opted_in,
        recall_enabled: opted_in,
    })
}

/// `insights::evidence_from_tools` on SurrealDB: the calls' persisted
/// results and the referenced artifact texts are read up front, then the
/// shared builder runs over them. Retained texts are hashed here; the
/// artifacts themselves are written by [`recon_input`].
pub async fn recon_evidence(
    repo: &Repository,
    results: &[(String, crate::osint::ToolResult)],
) -> Result<Vec<RetainedEvidence>> {
    use std::collections::HashMap;
    let ids: Vec<String> = results
        .iter()
        .map(|(id, _)| id.split('#').next().unwrap_or(id).to_string())
        .collect();
    let hashes: Vec<String> = results
        .iter()
        .filter_map(|(_, r)| {
            r.observations
                .pointer("/nlp_source/artifact_hash")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
        .collect();
    #[derive(SurrealValue)]
    struct Persisted {
        id: String,
        result_json: Option<String>,
    }
    #[derive(SurrealValue)]
    struct Text {
        id: String,
        text: String,
    }
    let mut r = repo
        .db()
        .query(
            "SELECT meta::id(id) AS id, result_json FROM osint_call
              WHERE meta::id(id) INSIDE $ids;
             SELECT meta::id(id) AS id, text FROM nlp_artifact
              WHERE meta::id(id) INSIDE $hs;",
        )
        .bind(("ids", ids))
        .bind(("hs", hashes))
        .await?
        .check()?;
    let persisted: Vec<Persisted> = r.take(0usize)?;
    let texts: Vec<Text> = r.take(1usize)?;
    let mut lookup = Prefetched {
        results: persisted
            .into_iter()
            .filter_map(|p| p.result_json.map(|j| (p.id, j)))
            .collect::<HashMap<_, _>>(),
        texts: texts.into_iter().map(|t| (t.id, t.text)).collect(),
    };
    crate::investigation::insights::evidence_with(&mut lookup, results)
}

struct Prefetched {
    results: std::collections::HashMap<String, String>,
    texts: std::collections::HashMap<String, String>,
}

impl crate::investigation::insights::EvidenceLookup for Prefetched {
    fn persisted_result(&mut self, call_id: &str) -> Result<Option<String>> {
        Ok(self.results.get(call_id).cloned())
    }
    fn artifact_text(&mut self, hash: &str) -> Option<String> {
        self.texts.get(hash).cloned()
    }
    fn retain_artifact(&mut self, text: &str) -> Result<String> {
        Ok(hash(text))
    }
}
