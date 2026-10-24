//! Recon answer extraction and Recon recall on SurrealDB (step 1 of the
//! Recon switch).
//!
//! [`persist_claims`] is `recon::persist_claims`: in one publisher
//! transaction it checks the answer and its thread still exist, completes
//! the extraction job (without claims when the thread opted out of
//! insights), writes up to five claims (memory, `insight_claim`, subject
//! binding, `conflict_or_revision` relations, `insight_source` rows for the
//! completed evidence they cite), skips claims the user deleted from this
//! run (tombstones), and queues memory-index outbox work for every memory
//! the claims resolve to, new or reused. `memories_changed` is bumped only
//! when a memory was written, as on SQLite.
//!
//! [`recall_for_turn`] is a STOPGAP for `orchestrate::recall_for_turn`: it
//! ranks active memories with SurrealDB full-text search (BM25 over
//! `memory.text`) instead of LanceDB vector recall. It is replaced when
//! vector recall lands (see `acceptance.md`).
//!
//! Nothing calls these yet; the Recon callers switch in a later commit.

use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Result};
use serde_json::Value;
use surrealdb::types::SurrealValue;

use super::codec::encode;
use super::insight::tombstone_runs;
use super::jobs::{rid, run, take};
use super::memory::index_changes_in;
use super::publisher::{Write, Wrote};
use super::recall::{apply_caps, estimate_tokens, RecallHit, RecallSource, DEFAULT_EXCLUDED};
use super::recon_runs::now;
use super::repo::Repository;
use super::tx::{Tx, TxError};
use crate::osint::ToolResult;
use crate::recon::Message;

/// Full-text index for the recall stopgap (idempotent).
pub const SCHEMA: &str = "DEFINE ANALYZER IF NOT EXISTS recon_recall TOKENIZERS class
  FILTERS lowercase, ascii, snowball(english);
DEFINE INDEX IF NOT EXISTS memory_text_search ON memory FIELDS text
  FULLTEXT ANALYZER recon_recall BM25;
DEFINE INDEX IF NOT EXISTS insight_claim_entity_predicate ON insight_claim
  FIELDS entity_id, predicate;";

/// Most claims one answer contributes (as on SQLite).
pub const MAX_CLAIMS: usize = 5;
/// Most memories one Recon turn recalls.
pub const RECALL_LIMIT: usize = 5;
/// Recall token budget per turn.
pub const RECALL_TOKENS: usize = 1_500;
/// Full-text candidates ranked before the caps.
const CANDIDATES: usize = 20;

/// What [`persist_claims`] wrote.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Persisted {
    /// New memories, in claim order.
    pub written: Vec<String>,
    /// Every memory the claims resolve to (new and reused); each got index
    /// work queued unless its current revision is already indexed.
    pub touched: Vec<String>,
    /// Subject actor memories the claims created or changed (index work is
    /// queued for them too; SQLite indexes them outside `touched`).
    pub actors: Vec<String>,
    /// Outbox rows queued (claim memories plus changed actors).
    pub queued: usize,
}

struct Claim {
    fingerprint: String,
    entity: String,
    predicate: String,
    object: String,
    text: String,
    topic: String,
    classification: String,
    confidence: f64,
    evidence: Vec<(String, Option<String>)>,
}

fn field(v: &Value, k: &str) -> String {
    v.get(k)
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string()
}

/// Parse and validate claims as SQLite does: lowercased identity, every
/// part present and at least one cited evidence id that completed.
fn parse(evidence: &[(String, ToolResult)], claims: &[Value]) -> Result<Vec<Claim>> {
    let mut out = Vec::new();
    for c in claims.iter().take(MAX_CLAIMS) {
        let entity = field(c, "entity").to_ascii_lowercase();
        let namespace = field(c, "namespace").to_ascii_lowercase();
        let predicate = field(c, "predicate").to_ascii_lowercase();
        let object = field(c, "object").to_ascii_lowercase();
        let text = field(c, "claim");
        let cited: Vec<(String, Option<String>)> = c
            .get("evidence_ids")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter_map(|eid| {
                evidence
                    .iter()
                    .find(|(id, r)| id == eid && r.status == "completed")
                    .map(|(id, r)| (id.clone(), Some(r.source_url.clone())))
            })
            .collect();
        if entity.is_empty()
            || predicate.is_empty()
            || object.is_empty()
            || text.is_empty()
            || cited.is_empty()
        {
            continue;
        }
        let fingerprint = serde_json::to_string(&(&namespace, &entity, &predicate, &object))?;
        out.push(Claim {
            fingerprint,
            entity,
            predicate,
            object,
            text,
            topic: c.get("topic").and_then(Value::as_str).unwrap_or("").into(),
            classification: c
                .get("classification")
                .and_then(Value::as_str)
                .unwrap_or("fact")
                .into(),
            confidence: c.get("confidence").and_then(Value::as_f64).unwrap_or(0.5),
            evidence: cited,
        });
    }
    Ok(out)
}

#[derive(SurrealValue)]
struct Source {
    live: bool,
    enabled: bool,
}

/// `recon::persist_claims` on SurrealDB. Fails (writing nothing) when the
/// answer or its thread is gone.
pub async fn persist_claims(
    repo: &Repository,
    answer: &Message,
    evidence: &[(String, ToolResult)],
    claims: &[Value],
) -> Result<Persisted> {
    let parsed = Arc::new(parse(evidence, claims)?);
    let answer = Arc::new(answer.clone());
    let result: Arc<Mutex<Persisted>> = Arc::default();
    let out = result.clone();
    let write: Write = Arc::new(move |tx: Tx| {
        let (parsed, answer, out) = (parsed.clone(), answer.clone(), out.clone());
        Box::pin(async move {
            let p = write_claims(&tx, &answer, &parsed).await?;
            let changed = !p.written.is_empty();
            let nonce = format!("extract:{}:{}", answer.id, crate::recon::id("n"));
            let mut index = p.touched.clone();
            index.extend(p.actors.iter().filter(|a| !p.touched.contains(a)).cloned());
            let changes = index_changes_in(&tx, &index, &nonce).await?;
            *out.lock().unwrap() = Persisted {
                queued: changes.len(),
                ..p
            };
            Ok(Wrote { changes, changed })
        })
    });
    repo.publisher()
        .publish(write)
        .await
        .map_err(|e| anyhow!("persist claims: {e}"))?;
    let p = result.lock().unwrap().clone();
    Ok(p)
}

async fn write_claims(tx: &Tx, answer: &Message, claims: &[Claim]) -> Result<Persisted, TxError> {
    let at = now();
    let mut r = run(
        tx,
        tx.query(
            "LET $t = (SELECT deleted, recall_insights FROM ONLY type::record('recon_thread', $tid));
             RETURN {
               live: (SELECT VALUE thread_id FROM ONLY $m) = $tid AND $t.deleted = false,
               enabled: $t.recall_insights = true
             };",
        )
        .bind(("tid", answer.thread_id.clone()))
        .bind(("m", rid("recon_message", &answer.id))),
    )
    .await?;
    let src: Option<Source> = take(&mut r, 1)?;
    let src = src.ok_or_else(|| TxError::Abort("source lookup returned nothing".into()))?;
    if !src.live {
        return Err(TxError::Abort("source answer or thread was deleted".into()));
    }
    let mut p = Persisted::default();
    if src.enabled {
        for c in claims {
            write_claim(tx, answer, c, &at, &mut p).await?;
        }
    }
    run(
        tx,
        tx.query(
            "UPDATE type::record('extraction_job', $m) SET state = 'completed', error = NONE,
               updated_at = $at RETURN NONE;",
        )
        .bind(("m", answer.id.clone()))
        .bind(("at", at)),
    )
    .await?;
    Ok(p)
}

async fn write_claim(
    tx: &Tx,
    answer: &Message,
    c: &Claim,
    at: &str,
    p: &mut Persisted,
) -> Result<(), TxError> {
    let mut r = run(
        tx,
        tx.query("SELECT VALUE memory_id FROM ONLY type::record('insight_claim', $fp);")
            .bind(("fp", c.fingerprint.clone())),
    )
    .await?;
    let existing: Option<String> = take(&mut r, 0)?;
    match &existing {
        Some(memory_id) => {
            if !p.touched.contains(memory_id) {
                p.touched.push(memory_id.clone());
            }
        }
        None => {
            // Never recreate a claim the user deleted from this run.
            if let (Some(runs), Some(run_id)) = (
                tombstone_runs(tx, &c.fingerprint).await?,
                answer.run_id.as_deref(),
            ) {
                if runs.iter().any(|r| r == run_id) {
                    return Ok(());
                }
            }
            let source = crate::brain::MemorySource {
                app: "recon".into(),
                conversation_id: answer.thread_id.clone(),
                message_id: Some(answer.id.clone()),
                reference: None,
            };
            let source =
                serde_json::to_string(&source).map_err(|e| TxError::Abort(e.to_string()))?;
            let memory_id = crate::recon::id("mem");
            run(
                tx,
                tx.query(
                    "CREATE type::record('memory', $mid) SET text = $text, category = 'investigation',
                       pinned = false, lifecycle = 'active', source_json = $src RETURN NONE;
                     CREATE type::record('insight_claim', $fp) SET memory_id = $mid, entity_id = $e,
                       predicate = $pr, object_value = $o, topic = $topic, classification = $class,
                       confidence = $conf, created_at = $at, updated_at = $at RETURN NONE;",
                )
                .bind(("mid", memory_id.clone()))
                .bind(("text", c.text.clone()))
                .bind(("src", source))
                .bind(("fp", c.fingerprint.clone()))
                .bind(("e", c.entity.clone()))
                .bind(("pr", c.predicate.clone()))
                .bind(("o", c.object.clone()))
                .bind(("topic", c.topic.clone()))
                .bind(("class", c.classification.clone()))
                .bind(("conf", c.confidence))
                .bind(("at", at.to_string())),
            )
            .await?;
            p.written.push(memory_id.clone());
            p.touched.push(memory_id);
            for id in super::actor_bind::bind_claim_subject_in(tx, &c.fingerprint, None, at).await?
            {
                if !p.actors.contains(&id) {
                    p.actors.push(id);
                }
            }
            let mut r = run(
                tx,
                tx.query(
                    "SELECT VALUE meta::id(id) FROM insight_claim
                       WHERE entity_id = $e AND predicate = $pr AND meta::id(id) != $fp;",
                )
                .bind(("e", c.entity.clone()))
                .bind(("pr", c.predicate.clone()))
                .bind(("fp", c.fingerprint.clone())),
            )
            .await?;
            let others: Vec<String> = take(&mut r, 0)?;
            for old in others {
                run(
                    tx,
                    tx.query(
                        "UPSERT type::record('insight_relation', $key) SET left_fingerprint = $l,
                           right_fingerprint = $r, relation = 'conflict_or_revision' RETURN NONE;",
                    )
                    .bind((
                        "key",
                        encode(&[&old, &c.fingerprint, "conflict_or_revision"]),
                    ))
                    .bind(("l", old))
                    .bind(("r", c.fingerprint.clone())),
                )
                .await?;
            }
        }
    }
    for (call_id, url) in &c.evidence {
        run(
            tx,
            tx.query(
                "LET $k = type::record('insight_source', $key);
                 IF !record::exists($k) {
                   CREATE $k SET fingerprint = $fp, thread_id = $t, run_id = $run,
                     answer_id = $a, call_id = $call, source_url = $url RETURN NONE;
                 };",
            )
            .bind(("key", encode(&[&c.fingerprint, &answer.id, call_id])))
            .bind(("fp", c.fingerprint.clone()))
            .bind(("t", Some(answer.thread_id.clone())))
            .bind(("run", answer.run_id.clone()))
            .bind(("a", answer.id.clone()))
            .bind(("call", call_id.clone()))
            .bind(("url", url.clone())),
        )
        .await?;
    }
    run(
        tx,
        tx.query("UPDATE type::record('insight_claim', $fp) SET updated_at = $at RETURN NONE;")
            .bind(("fp", c.fingerprint.clone()))
            .bind(("at", at.to_string())),
    )
    .await?;
    Ok(())
}

#[derive(SurrealValue)]
struct Row {
    id: String,
    text: String,
    memory_kind: String,
    score: f64,
}

/// Full-text candidates for one query term: active memories of the default
/// recall kinds, BM25 best first. [`recall_for_turn`] runs it once per
/// distinct term, so a memory matching any term is a candidate (OR
/// semantics). Ranking is by terms matched, then summed BM25 score: BM25
/// idf goes negative for a term in more than half the memories, so the raw
/// score is not a usable relevance cut on its own.
/// Public for the plan test.
pub const RECALL_SURQL: &str = "SELECT record::id(id) AS id, text, memory_kind,
    search::score(1) AS score FROM memory
    WHERE text @1@ $q AND lifecycle = 'active' AND memory_kind NOT IN $excluded
    ORDER BY score DESC LIMIT $n";

/// Most distinct query terms searched per turn.
const MAX_TERMS: usize = 16;

/// Distinct lowercased alphanumeric terms of `question`, in order.
fn terms(question: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in question
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_lowercase)
    {
        if !out.contains(&t) {
            out.push(t);
        }
        if out.len() == MAX_TERMS {
            break;
        }
    }
    out
}

/// `orchestrate::recall_for_turn` on SurrealDB: with an embedder whose
/// model serves the current generation, fenced vector recall (DISKANN,
/// lexical top-up; `recall::recall`) within [`RECALL_LIMIT`] and
/// [`RECALL_TOKENS`]; without one, or when the query cannot be embedded or
/// no vector hit lands, [`recall_for_turn`]. Nothing when the thread opted
/// out of insights.
pub async fn recall_for_turn_with(
    repo: &Repository,
    thread_id: &str,
    question: &str,
    vectors: Option<&super::vectors::Embedder>,
) -> Result<Vec<RecallHit>> {
    recall_candidates_for_turn(
        repo,
        thread_id,
        question,
        vectors,
        RECALL_LIMIT,
        RECALL_TOKENS,
    )
    .await
}

/// [`recall_for_turn_with`] capped at `limit` hits within `tokens`: the
/// candidate pool the Recon admission policy filters before the
/// [`RECALL_LIMIT`] cap, as SQLite's `recall_sqlite` does.
pub async fn recall_candidates_for_turn(
    repo: &Repository,
    thread_id: &str,
    question: &str,
    vectors: Option<&super::vectors::Embedder>,
    limit: usize,
    tokens: usize,
) -> Result<Vec<RecallHit>> {
    let Some(e) = vectors else {
        return lexical_for_turn(repo, thread_id, question, limit, tokens).await;
    };
    if question.trim().is_empty() {
        return Ok(Vec::new());
    }
    let mut r = repo
        .db()
        .query("SELECT VALUE recall_insights FROM ONLY type::record('recon_thread', $t)")
        .bind(("t", thread_id.to_string()))
        .await?
        .check()?;
    if r.take::<Option<bool>>(0usize)? == Some(false) {
        return Ok(Vec::new());
    }
    let (state, embedding) = match super::vectors::query_vector(repo, e, question).await {
        Ok(Some(found)) => found,
        Ok(None) => return lexical_for_turn(repo, thread_id, question, limit, tokens).await,
        Err(err) => {
            tracing::warn!("query embedding failed; using lexical recall: {err:#}");
            return lexical_for_turn(repo, thread_id, question, limit, tokens).await;
        }
    };
    let q = super::recall::RecallQuery {
        text: question.to_string(),
        embedding: Some(embedding),
        model: state.model,
        generation: state.generation,
        max_memories: limit,
        token_budget: tokens,
        ..Default::default()
    };
    let hits = super::recall::recall(repo.db(), &q)
        .await
        .map_err(|e| anyhow!("recall: {e}"))?;
    if hits.iter().any(|hit| hit.source == RecallSource::Vector) {
        return Ok(hits);
    }
    lexical_for_turn(repo, thread_id, question, limit, tokens).await
}

/// STOPGAP for `orchestrate::recall_for_turn`: nothing when the thread
/// opted out of insights; otherwise up to [`RECALL_LIMIT`] memories within
/// [`RECALL_TOKENS`], ranked by SurrealDB full-text search. Replace with
/// vector recall when it lands.
pub async fn recall_for_turn(
    repo: &Repository,
    thread_id: &str,
    question: &str,
) -> Result<Vec<RecallHit>> {
    lexical_for_turn(repo, thread_id, question, RECALL_LIMIT, RECALL_TOKENS).await
}

async fn lexical_for_turn(
    repo: &Repository,
    thread_id: &str,
    question: &str,
    limit: usize,
    tokens: usize,
) -> Result<Vec<RecallHit>> {
    let terms = terms(question);
    if terms.is_empty() {
        return Ok(Vec::new());
    }
    let mut r = repo
        .db()
        .query("SELECT VALUE recall_insights FROM ONLY type::record('recon_thread', $t)")
        .bind(("t", thread_id.to_string()))
        .await?
        .check()?;
    if r.take::<Option<bool>>(0usize)? == Some(false) {
        return Ok(Vec::new());
    }
    let excluded: Vec<String> = DEFAULT_EXCLUDED
        .iter()
        .map(|k| k.as_str().to_string())
        .collect();
    let mut rows: Vec<(usize, Row)> = Vec::new();
    for term in terms {
        let mut r = repo
            .db()
            .query(RECALL_SURQL)
            .bind(("q", term))
            .bind(("excluded", excluded.clone()))
            .bind(("n", CANDIDATES as i64))
            .await?
            .check()?;
        let hits: Vec<Row> = r.take(0usize)?;
        for hit in hits {
            match rows.iter_mut().find(|(_, row)| row.id == hit.id) {
                Some((matched, row)) => {
                    *matched += 1;
                    row.score += hit.score;
                }
                None => rows.push((1, hit)),
            }
        }
    }
    rows.sort_by(|(ma, a), (mb, b)| {
        mb.cmp(ma)
            .then_with(|| b.score.total_cmp(&a.score))
            .then_with(|| a.id.cmp(&b.id))
    });
    let ranked = rows
        .into_iter()
        .map(|(_, row)| RecallHit {
            tokens: estimate_tokens(&row.text),
            id: row.id,
            text: row.text,
            kind: row.memory_kind,
            score: row.score as f32,
            source: RecallSource::Lexical,
        })
        .collect();
    Ok(apply_caps(ranked, limit, tokens))
}
