//! Atlas publication receipts and record-derived graph edges on SurrealDB
//! (P3 step 4 remainder).
//!
//! [`legacy_receipt`] ports `atlas_memory::legacy_receipt`: a receipt for a
//! run published before checkpoints existed, rebuilt from its stored
//! claim/source links. Unrecoverable links are rejected with a reason and
//! nothing is fabricated from a fingerprint. Unlike SQLite (reads outside,
//! writes in a later `BEGIN IMMEDIATE`), the reads, the index work and the
//! `atlas_publication` row share one publisher transaction.
//!
//! The record-derived edge meanings (`source`, `mentioned_in`, `contains`)
//! are not stored as `brain_graph_edge` rows: as in
//! `brain_topology_query`, they are read from the canonical Atlas, insight
//! and actor records at query time, so they can never drift from them.
//! [`memory_record_edges`] and [`actor_mention_edges`] return them.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use surrealdb::engine::any::Any;
use surrealdb::types::SurrealValue;
use surrealdb::Surreal;

use super::codec::encode;
use super::insight::{atlas_answer_id, atlas_brief_id};
use super::jobs::{run, take};
use super::publisher::{Publisher, Wrote};
use super::tx::{Tx, TxError};
use crate::store::{PublicationReceipt, RejectedClaim};

/// Revision recorded for reconstructed receipts (`atlas_memory::LEGACY_REVISION`).
pub const LEGACY_REVISION: &str = "legacy";

/// Derivation statuses that no longer support provenance edges.
const DEAD: [&str; 3] = ["deleted", "retired", "rejected"];

/// A run's distinct source links, pinned to `insight_source_call`'s
/// `run_id` prefix. SQLite orders by first rowid; SurrealDB has no insertion
/// order, so links are ordered by fingerprint.
pub const RUN_LINKS_SURQL: &str = "SELECT fingerprint, call_id FROM insight_source
    WHERE run_id = $run AND answer_id = $answer ORDER BY fingerprint, call_id";

#[derive(Debug, Clone, SurrealValue)]
struct Link {
    fingerprint: String,
    call_id: String,
}

async fn legacy_in(tx: &Tx, run_id: &str) -> Result<PublicationReceipt, TxError> {
    let mut r = run(
        tx,
        tx.query(RUN_LINKS_SURQL)
            .bind(("run", run_id.to_string()))
            .bind(("answer", atlas_answer_id(run_id))),
    )
    .await?;
    let rows: Vec<Link> = take(&mut r, 0)?;
    // GROUP BY fingerprint with MIN(call_id).
    let mut links: Vec<Link> = Vec::new();
    for row in rows {
        if links.last().map(|l| l.fingerprint.as_str()) != Some(row.fingerprint.as_str()) {
            links.push(row);
        }
    }
    let mut receipt = PublicationReceipt {
        run_id: run_id.into(),
        revision: LEGACY_REVISION.into(),
        input_claims: links.len(),
        ..Default::default()
    };
    for (index, link) in links.into_iter().enumerate() {
        let mut r = run(
            tx,
            tx.query(
                "LET $mem = (SELECT VALUE memory_id FROM ONLY type::record('insight_claim', $fp));
                 RETURN $mem;
                 RETURN $mem != NONE AND record::exists(type::record('memory', $mem));
                 RETURN count(SELECT id FROM memory_tombstone WHERE fingerprint = $fp AND fingerprint != '')
                   + (IF $mem = NONE THEN 0 ELSE count(SELECT id FROM memory_tombstone WHERE memory_id = $mem) END);",
            )
            .bind(("fp", link.fingerprint.clone())),
        )
        .await?;
        let memory_id: Option<String> = take(&mut r, 1)?;
        let exists: Option<bool> = take(&mut r, 2)?;
        let tombstoned: Option<i64> = take(&mut r, 3)?;
        let reason = match (&memory_id, exists == Some(true)) {
            (Some(_), true) => None,
            _ if tombstoned.unwrap_or(0) > 0 => Some("deleted by user (tombstoned); not recreated"),
            (None, _) => Some("source link has no claim; claim text lost — re-extraction required"),
            (Some(_), false) => {
                Some("canonical memory row missing; claim text lost — re-extraction required")
            }
        };
        match reason {
            None => {
                let id = memory_id.unwrap_or_default();
                receipt.reused += 1;
                if !receipt.reused_memory_ids.contains(&id) {
                    receipt.reused_memory_ids.push(id.clone());
                }
                receipt.accepted_claims += 1;
                receipt.claim_memory_ids.push((link.fingerprint, id));
            }
            Some(reason) => receipt.rejected.push(RejectedClaim {
                index,
                article_id: link.call_id,
                reason: reason.into(),
            }),
        }
    }
    receipt.brief_memory_id = atlas_brief_id(tx, run_id).await?;
    let mut affected: Vec<String> = Vec::new();
    for id in receipt
        .claim_memory_ids
        .iter()
        .map(|(_, id)| id)
        .chain(receipt.brief_memory_id.iter())
    {
        if !affected.contains(id) {
            affected.push(id.clone());
        }
    }
    receipt.affected_memory_ids = affected;
    Ok(receipt)
}

/// `atlas_memory::legacy_receipt` on SurrealDB. Index work is queued for
/// every recoverable memory whose revision is missing or stale, and the
/// receipt is stored as `atlas_publication:[run, 'legacy']` with state
/// `reconstructed`. `queued_index_changes` and `index_tasks` stay empty, as
/// in `insight::publish_atlas_insights` (index work is keyed by memory id).
/// Nothing a user sees changes, so `memories_changed` is not bumped.
pub async fn legacy_receipt(
    publisher: &Publisher,
    run_id: &str,
    parent_job: Option<&str>,
) -> Result<PublicationReceipt, TxError> {
    let slot: Arc<Mutex<PublicationReceipt>> = Arc::default();
    let out = slot.clone();
    let args = Arc::new((
        run_id.to_string(),
        parent_job.unwrap_or_default().to_string(),
        format!("legacy-{run_id}-{}", chrono::Utc::now().timestamp_micros()),
    ));
    let w: super::publisher::Write = Arc::new(move |tx: Tx| {
        let (args, out) = (args.clone(), out.clone());
        Box::pin(async move {
            let (run_id, job, nonce) = &*args;
            let mut receipt = legacy_in(&tx, run_id).await?;
            let changes =
                super::memory::index_changes_in(&tx, &receipt.affected_memory_ids, nonce).await?;
            receipt.already_indexed = receipt
                .affected_memory_ids
                .len()
                .saturating_sub(changes.len());
            let json = serde_json::to_string(&receipt)
                .map_err(|e| TxError::Abort(format!("receipt serialization: {e}")))?;
            run(
                &tx,
                tx.query(
                    "UPSERT type::record('atlas_publication', [$run, $rev]) SET run_id = $run,
                       revision = $rev, state = 'reconstructed', receipt_json = $json,
                       job_id = $job, created_at = created_at ?? $now, updated_at = $now RETURN NONE;",
                )
                .bind(("run", run_id.clone()))
                .bind(("rev", LEGACY_REVISION.to_string()))
                .bind(("json", json))
                .bind(("job", job.clone()))
                .bind(("now", chrono::Utc::now().to_rfc3339())),
            )
            .await?;
            *out.lock().expect("receipt lock") = receipt;
            Ok(Wrote {
                changes,
                changed: false,
            })
        })
    });
    publisher.publish(w).await?;
    let receipt = slot.lock().expect("receipt lock").clone();
    Ok(receipt)
}

// ---------------------------------------------------------------------------
// Record-derived edges

/// One record-derived edge. `from`/`to` use the topology node keys of
/// `brain_topology_query`: `memory id`, `article:<run>/<article>`,
/// `source:<kind>:<id>@<revision>` and `topic:<label>`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct RecordEdge {
    pub key: String,
    pub meaning: &'static str,
    pub from: String,
    pub to: String,
    /// The canonical record(s) the edge was read from.
    pub receipt: String,
    pub status: String,
}

/// Article node key.
pub fn article_key(run_id: &str, article_id: &str) -> String {
    format!("article:{run_id}/{article_id}")
}

/// Non-article source node key.
pub fn source_key(kind: &str, id: &str, revision: &str) -> String {
    format!("source:{kind}:{id}@{revision}")
}

/// `brain_topology_query::meaningful_topic`.
pub fn meaningful_topic(label: &str) -> bool {
    !matches!(
        label.trim().to_lowercase().as_str(),
        "" | "unk" | "unknown" | "investigation" | "fact" | "claim" | "inference"
    )
}

#[derive(Debug, Clone, SurrealValue)]
struct Derivation {
    id: String,
    snapshot_id: String,
    insight_kind: String,
}

#[derive(Debug, Clone, SurrealValue)]
struct Owner {
    owner_kind: String,
    owner_id: String,
    inputs_json: String,
    readiness_json: String,
}

#[derive(Debug, Clone, SurrealValue)]
struct Evidence {
    derivation_id: String,
    source_id: String,
    source_revision: String,
    evidence_json: String,
}

#[derive(Debug, Clone, SurrealValue)]
struct ClaimRow {
    fingerprint: String,
    topic: String,
}

#[derive(Debug, Clone, SurrealValue)]
struct SourceRow {
    fingerprint: String,
    answer_id: String,
    call_id: String,
    run_id: Option<String>,
    thread_id: Option<String>,
}

#[derive(Debug, Clone, SurrealValue)]
pub(crate) struct ArticleRef {
    pub(crate) run_id: String,
    pub(crate) article_id: String,
    pub(crate) title: String,
    pub(crate) description: String,
    pub(crate) url: String,
    pub(crate) published_at: String,
}

pub(crate) async fn articles(
    db: &impl super::tx::Reads,
    article_id: &str,
) -> Result<Vec<ArticleRef>, TxError> {
    let mut r = db
        .read(
            "SELECT run_id, article_id, title, description, url, published_at FROM atlas_article
             WHERE article_id = $a ORDER BY run_id",
        )
        .bind(("a", article_id.to_string()))
        .await?
        .check()?;
    Ok(r.take(0)?)
}

/// `brain_topology_query::current_metadata_sql`: a snapshot's retained
/// source refs (`inputs_json.source_refs`) still describe the article as
/// stored: a ref for `article:<id>` with the same URL and published time
/// whose artifact text is the current title and description. An article
/// edge that fails it is shown as a historical origin. `refs` comes from
/// [`retained_refs`].
/// The source refs a derivation of `kind` was made from: a scoped
/// publication carries the sibling kind's original refs in its readiness
/// receipt (`carried_kinds[].inputs.source_refs`), which win over the
/// snapshot's own `inputs_json.source_refs`, so the new kind's revision is
/// never assigned to the sibling (`current_metadata_sql`).
pub(crate) fn retained_refs(
    inputs_json: &str,
    readiness_json: &str,
    kind: &str,
) -> serde_json::Value {
    let carried = serde_json::from_str::<serde_json::Value>(readiness_json)
        .ok()
        .and_then(|r| {
            r["carried_kinds"].as_array().and_then(|kinds| {
                kinds
                    .iter()
                    .find(|c| c["kind"].as_str() == Some(kind))
                    .map(|c| c["inputs"]["source_refs"].clone())
            })
        })
        .filter(|refs| !refs.is_null());
    carried.unwrap_or_else(|| {
        serde_json::from_str::<serde_json::Value>(inputs_json)
            .ok()
            .map(|i| i["source_refs"].clone())
            .unwrap_or(serde_json::Value::Null)
    })
}

pub(crate) async fn current_metadata(
    db: &impl super::tx::Reads,
    refs: &serde_json::Value,
    a: &ArticleRef,
) -> Result<bool, TxError> {
    let source = format!("article:{}", a.article_id);
    let field =
        |r: &serde_json::Value, k: &str| r.get(k).and_then(|v| v.as_str()).map(str::to_string);
    let hashes: Vec<String> = refs
        .as_array()
        .into_iter()
        .flatten()
        .filter(|r| field(r, "source_id").as_deref() == Some(source.as_str()))
        .filter(|r| field(r, "url").as_deref() == Some(a.url.as_str()))
        .filter(|r| field(r, "published_at").unwrap_or_default() == a.published_at)
        .filter_map(|r| field(r, "artifact_hash"))
        .collect();
    if hashes.is_empty() {
        return Ok(false);
    }
    let text = format!("{}\n{}", a.title, a.description);
    let mut r = db
        .read(
            "RETURN array::len(array::filter($hashes,
               |$h| (SELECT VALUE text FROM ONLY type::record('nlp_artifact', $h)) == $text)) > 0",
        )
        .bind(("hashes", hashes))
        .bind(("text", text))
        .await?
        .check()?;
    Ok(r.take::<Option<bool>>(0)? == Some(true))
}

/// Source and Contains edges of one memory, read from live derivations
/// (with their evidence), insight claims and sources, and retained Atlas
/// articles. Sorted and deduplicated by key.
pub async fn memory_record_edges(
    db: &Surreal<Any>,
    memory_id: &str,
) -> Result<Vec<RecordEdge>, TxError> {
    let mut r = db
        .query(
            "SELECT record::id(id) AS id, snapshot_id, insight_kind FROM insight_derivation
               WHERE memory_id = $m AND status NOTINSIDE $dead ORDER BY id;
             SELECT derivation_id, source_id, source_revision, evidence_json
               FROM insight_derivation_evidence WHERE memory_id = $m;
             SELECT record::id(id) AS fingerprint, topic FROM insight_claim WHERE memory_id = $m;",
        )
        .bind(("m", memory_id.to_string()))
        .bind((
            "dead",
            DEAD.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        ))
        .await?
        .check()?;
    let derivations: Vec<Derivation> = r.take(0)?;
    let evidence: Vec<Evidence> = r.take(1)?;
    let claims: Vec<ClaimRow> = r.take(2)?;
    let live: BTreeSet<&str> = derivations.iter().map(|d| d.id.as_str()).collect();
    let mut out: BTreeSet<RecordEdge> = BTreeSet::new();
    let article_edge = |run: &str, id: &str, receipt: String, historical: bool| {
        let from = article_key(run, id);
        RecordEdge {
            key: format!("article-evidence:{from}:{memory_id}"),
            meaning: "source",
            to: memory_id.to_string(),
            receipt,
            status: if historical {
                "historical origin · revision unavailable".into()
            } else {
                String::new()
            },
            from,
        }
    };
    // Article provenance from live derivations: the snapshot owner, or an
    // `article:<id>` evidence source.
    for d in &derivations {
        let mut owners: BTreeSet<String> = BTreeSet::new();
        let (mut inputs, mut readiness) = (String::new(), String::new());
        let mut r = db
            .query(
                "SELECT owner_kind, owner_id, inputs_json, readiness_json
                   FROM type::record('insight_snapshot', $s)",
            )
            .bind(("s", d.snapshot_id.clone()))
            .await?
            .check()?;
        for o in r.take::<Vec<Owner>>(0)? {
            inputs = o.inputs_json.clone();
            readiness = o.readiness_json.clone();
            if matches!(o.owner_kind.as_str(), "article" | "atlas") {
                owners.insert(o.owner_id);
            }
        }
        for e in evidence.iter().filter(|e| e.derivation_id == d.id) {
            if let Some(a) = e.source_id.strip_prefix("article:") {
                owners.insert(a.to_string());
            }
        }
        for owner in owners {
            for a in articles(db, &owner).await? {
                let refs = retained_refs(&inputs, &readiness, &d.insight_kind);
                let current = current_metadata(db, &refs, &a).await?;
                out.insert(article_edge(
                    &a.run_id,
                    &a.article_id,
                    format!("insight_derivations:{}", d.id),
                    !current,
                ));
            }
        }
    }
    // Evidence sources of live derivations.
    for e in evidence
        .iter()
        .filter(|e| live.contains(e.derivation_id.as_str()))
    {
        let from = source_key("evidence", &e.source_id, &e.source_revision);
        out.insert(RecordEdge {
            key: format!("evidence:{from}:{memory_id}"),
            meaning: "source",
            from,
            to: memory_id.to_string(),
            receipt: e.evidence_json.clone(),
            status: String::new(),
        });
    }
    for c in &claims {
        // Historical origins from insight sources (not deleted).
        let mut r = db
            .query(
                "SELECT fingerprint, answer_id, call_id, run_id, thread_id FROM insight_source
                 WHERE fingerprint = $fp AND deleted_origin = false ORDER BY answer_id, call_id",
            )
            .bind(("fp", c.fingerprint.clone()))
            .await?
            .check()?;
        let sources: Vec<SourceRow> = r.take(0)?;
        for s in &sources {
            let article = s.call_id.strip_prefix("article:").unwrap_or(&s.call_id);
            for a in articles(db, article).await? {
                let run_ok = match &s.run_id {
                    None => true,
                    Some(run) => *run == a.run_id || *run == a.article_id,
                };
                if run_ok {
                    out.insert(article_edge(
                        &a.run_id,
                        &a.article_id,
                        format!(
                            "insight_sources:{}:{}:{}",
                            s.fingerprint, s.answer_id, s.call_id
                        ),
                        true,
                    ));
                }
            }
            if let Some(thread) = s.thread_id.as_deref().filter(|t| !t.is_empty()) {
                // Only live Recon threads (`recon_threads.deleted = 0`).
                let mut r = db
                    .query("SELECT VALUE deleted FROM ONLY type::record('recon_thread', $t)")
                    .bind(("t", thread.to_string()))
                    .await?
                    .check()?;
                let deleted: Option<bool> = r.take(0)?;
                if deleted != Some(false) {
                    continue;
                }
                let from = source_key("thread", thread, "");
                out.insert(RecordEdge {
                    key: format!("thread-evidence:{thread}:{memory_id}"),
                    meaning: "source",
                    from,
                    to: memory_id.to_string(),
                    receipt: format!("recon-thread:{thread}"),
                    status: String::new(),
                });
            }
        }
        if meaningful_topic(&c.topic) {
            let topic = format!("topic:{}", c.topic.trim());
            out.insert(RecordEdge {
                key: format!("topic:{topic}:{memory_id}"),
                meaning: "contains",
                from: memory_id.to_string(),
                to: topic,
                receipt: format!("insight_claims:{}:topic={}", c.fingerprint, c.topic),
                status: String::new(),
            });
        }
    }
    // An article edge from a live derivation wins over the historical one.
    let mut edges: Vec<RecordEdge> = Vec::new();
    for e in out {
        match edges.iter_mut().find(|x| x.key == e.key) {
            Some(prior) if prior.status.is_empty() => {}
            Some(prior) if e.status.is_empty() => *prior = e,
            Some(_) => {}
            None => edges.push(e),
        }
    }
    Ok(edges)
}

#[derive(Debug, Clone, SurrealValue)]
struct MentionRow {
    origin_kind: String,
    origin_scope: String,
    origin_id: String,
    origin_revision: String,
    spans_json: String,
}

/// MentionedIn edges of one actor, from `brain_actor_mention` (pinned to
/// `brain_actor_mention_actor`). Article mentions resolve to the retained
/// article of their run (`origin_scope`) and are dropped when it is gone.
pub async fn actor_mention_edges(
    db: &Surreal<Any>,
    actor_id: &str,
) -> Result<Vec<RecordEdge>, TxError> {
    let mut r = db
        .query(ACTOR_MENTIONS_SURQL)
        .bind(("a", actor_id.to_string()))
        .await?
        .check()?;
    let rows: Vec<MentionRow> = r.take(0)?;
    let mut out = Vec::new();
    for m in rows {
        let to = if m.origin_kind == "article" {
            let mut r = db
                .query("RETURN record::exists(type::record('atlas_article', $key))")
                .bind(("key", encode(&[&m.origin_scope, &m.origin_id])))
                .await?
                .check()?;
            if r.take::<Option<bool>>(0)? != Some(true) {
                continue;
            }
            article_key(&m.origin_scope, &m.origin_id)
        } else {
            source_key(&m.origin_kind, &m.origin_id, &m.origin_revision)
        };
        out.push(RecordEdge {
            key: format!(
                "mention:{actor_id}:{}:{}:{}:{}",
                m.origin_kind, m.origin_scope, m.origin_id, m.origin_revision
            ),
            meaning: "mentioned_in",
            from: actor_id.to_string(),
            to,
            receipt: format!(
                "brain_actor_mentions:{actor_id}:{}:{}:{}@{}\n{}",
                m.origin_kind, m.origin_scope, m.origin_id, m.origin_revision, m.spans_json
            ),
            status: String::new(),
        });
    }
    Ok(out)
}

/// One actor's mentions; pinned to `brain_actor_mention_actor`.
pub const ACTOR_MENTIONS_SURQL: &str =
    "SELECT origin_kind, origin_scope, origin_id, origin_revision, spans_json
    FROM brain_actor_mention WHERE actor_id = $a
    ORDER BY origin_kind, origin_scope, origin_id, origin_revision";

// ---------------------------------------------------------------------------
// refresh_cached_publication inputs

/// Stages `brief_canonical::refresh_cached_publication` republishes, in order.
pub const REPUBLISHED_STAGES: [&str; 3] = [
    "intel_brief_claims",
    "intel_brief_inferences",
    "intel_brief_links",
];

#[derive(Debug, Clone, SurrealValue)]
struct StageRow {
    stage: String,
    output_json: String,
}

/// `brief_pipeline::stage_outputs` on SurrealDB: a chain's committed stage
/// outputs by stage, through `stage_output_root`. Unparseable outputs are
/// skipped, as in SQLite.
pub async fn stage_outputs(
    db: &Surreal<Any>,
    job_id: &str,
) -> Result<std::collections::HashMap<String, serde_json::Value>, TxError> {
    let mut r = db
        .query(STAGE_OUTPUTS_SURQL)
        .bind(("root", super::jobs::rid("job", job_id)))
        .await?
        .check()?;
    let rows: Vec<StageRow> = r.take(0)?;
    Ok(rows
        .into_iter()
        .filter_map(|row| Some((row.stage, serde_json::from_str(&row.output_json).ok()?)))
        .collect())
}

/// One chain's stage outputs; pinned to `stage_output_root`.
pub const STAGE_OUTPUTS_SURQL: &str =
    "SELECT stage, output_json FROM stage_output WHERE root = $root";

/// The stages of `outputs` that `refresh_cached_publication` republishes.
pub fn republished_stages(
    outputs: &std::collections::HashMap<String, serde_json::Value>,
) -> Vec<&'static str> {
    REPUBLISHED_STAGES
        .into_iter()
        .filter(|s| outputs.contains_key(*s))
        .collect()
}

#[cfg(test)]
mod retained_refs_tests {
    use super::retained_refs;
    use serde_json::json;

    #[test]
    fn a_scoped_publication_uses_the_sibling_kinds_carried_refs() {
        let inputs =
            json!({"source_refs": [{"source_id": "article:a", "revision": "new"}]}).to_string();
        let readiness = json!({"carried_kinds": [
            {"kind": "claim", "source_snapshot_id": "s0",
             "inputs": {"source_refs": [{"source_id": "article:a", "revision": "old"}]}}
        ]})
        .to_string();
        assert_eq!(
            retained_refs(&inputs, &readiness, "claim")[0]["revision"],
            "old"
        );
        assert_eq!(
            retained_refs(&inputs, &readiness, "inference")[0]["revision"],
            "new"
        );
        assert_eq!(retained_refs(&inputs, "", "claim")[0]["revision"], "new");
        assert!(retained_refs("", "", "claim").is_null());
    }
}
