//! Atlas deletes and retention on SurrealDB (P3 step 3): the transactional
//! ports of `Store::atlas_delete_run`, `delete_cycle_memories`,
//! `delete_article_insights`, `atlas_delete_article` and
//! `atlas_prune_expired`, plus `reconcile_deleted_memories` and
//! `reconcile_deleted_articles`.
//!
//! Each public function is one publisher transaction through
//! `write_then_index`: deleted memories queue index removal, retired ones
//! queue reindexing, and `memories_changed` moves only when some memory did.
//! Article placeholder nodes and memory nodes left without edges are
//! removed in the same transaction.
//!
//! Still with the caller (SQLite-only or not yet ported; see
//! `.planning/2026-10-22-surrealdb/acceptance.md`): the NLP cache and
//! `nlp_source_refs`, `brain_backfill_receipts` (step 4), the job retention
//! sweep (`jobs::cleanup_batch`, its own idempotent transaction) and
//! `retire_deleted_article_leads` (`app_state`).

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::{Arc, Mutex};

use surrealdb::types::{RecordId, SurrealValue};

use super::codec::encode;
use super::graph::memory_node;
use super::insight::{atlas_answer_id, atlas_brief_id};
use super::jobs::{rid, run, take};
use super::lifecycle::{reconcile_lifecycle_in, reconcile_sources_in, retire_origin_in};
use super::memory::{write_then_index, MemoryWrite};
use super::publisher::Publisher;
use super::tx::{Tx, TxError};

/// Runs older than this are pruned (`atlas_prune_expired`).
pub const RETENTION_HOURS: i64 = 36;

#[derive(SurrealValue)]
struct Source {
    id: RecordId,
    fingerprint: String,
    answer_id: String,
    call_id: String,
}

async fn atlas_snapshots(tx: &Tx) -> Result<HashSet<String>, TxError> {
    let mut r = run(
        tx,
        tx.query("SELECT VALUE record::id(id) FROM insight_snapshot WHERE owner_kind = 'atlas'"),
    )
    .await?;
    Ok(take::<Vec<String>>(&mut r, 0)?.into_iter().collect())
}

/// Latest other run (seen_at DESC, run_id) holding each article, one round trip.
async fn other_runs(
    tx: &Tx,
    run_id: &str,
    articles: &[String],
) -> Result<HashMap<String, String>, TxError> {
    if articles.is_empty() {
        return Ok(HashMap::new());
    }
    let mut sql = String::new();
    for i in 0..articles.len() {
        sql.push_str(&format!(
            "SELECT VALUE run_id FROM atlas_article WHERE article_id = $a{i} AND run_id != $run \
             ORDER BY seen_at DESC, run_id LIMIT 1;"
        ));
    }
    let mut q = tx.query(sql).bind(("run", run_id.to_string()));
    for (i, a) in articles.iter().enumerate() {
        q = q.bind((format!("a{i}"), a.clone()));
    }
    let mut r = run(tx, q).await?;
    let mut out = HashMap::new();
    for (i, a) in articles.iter().enumerate() {
        if let Some(other) = take::<Vec<String>>(&mut r, i)?.into_iter().next() {
            out.insert(a.clone(), other);
        }
    }
    Ok(out)
}

/// Drop each fingerprint left with no source: its relations, claim, graph
/// summary and memory. Returns the deleted memory ids.
async fn drop_unsourced(tx: &Tx, fingerprints: &BTreeSet<String>) -> Result<Vec<String>, TxError> {
    if fingerprints.is_empty() {
        return Ok(Vec::new());
    }
    let fps: Vec<&String> = fingerprints.iter().collect();
    let mut sql = String::new();
    for i in 0..fps.len() {
        sql.push_str(&format!(
            "RETURN {{ left: count(SELECT id FROM insight_source WHERE fingerprint = $f{i} LIMIT 1), \
             memory: (SELECT VALUE memory_id FROM ONLY $c{i}) }};"
        ));
    }
    let mut q = tx.query(sql);
    for (i, fp) in fps.iter().enumerate() {
        q = q
            .bind((format!("f{i}"), fp.to_string()))
            .bind((format!("c{i}"), rid("insight_claim", fp)));
    }
    let mut r = run(tx, q).await?;
    #[derive(SurrealValue)]
    struct Left {
        left: i64,
        memory: Option<String>,
    }
    let mut dead_fps = Vec::new();
    let mut doomed = Vec::new();
    for (i, fp) in fps.iter().enumerate() {
        let row: Option<Left> = take(&mut r, i)?;
        if let Some(row) = row.filter(|row| row.left == 0) {
            dead_fps.push(fp.to_string());
            doomed.extend(row.memory);
        }
    }
    if dead_fps.is_empty() {
        return Ok(Vec::new());
    }
    run(
        tx,
        tx.query(
            "FOR $fp IN $fps {
               DELETE insight_relation WHERE left_fingerprint = $fp RETURN NONE;
               DELETE insight_relation WHERE right_fingerprint = $fp RETURN NONE;
               DELETE type::record('insight_claim', $fp) RETURN NONE;
             };",
        )
        .bind(("fps", dead_fps)),
    )
    .await?;
    delete_memories_in(tx, &doomed).await?;
    Ok(doomed)
}

/// Delete memories with their graph summary and owned edges; their graph
/// nodes stay, marked `deleted`, until no edge touches them
/// ([`drop_orphan_nodes_in`]).
async fn delete_memories_in(tx: &Tx, ids: &[String]) -> Result<(), TxError> {
    if ids.is_empty() {
        return Ok(());
    }
    run(
        tx,
        tx.query(
            "FOR $m IN $ids {
               DELETE type::record('memory_graph_summary', $m) RETURN NONE;
               DELETE brain_graph_edge WHERE owner = $m RETURN NONE;
               DELETE type::record('memory', $m) RETURN NONE;
             };
             UPDATE $nodes SET lifecycle = 'deleted' RETURN NONE;",
        )
        .bind(("ids", ids.to_vec()))
        .bind((
            "nodes",
            ids.iter()
                .map(|m| rid("brain_graph_node", &memory_node(m)))
                .collect::<Vec<_>>(),
        )),
    )
    .await?;
    Ok(())
}

/// Remove placeholder nodes no edge touches any more: a run's (or one
/// article's) article nodes, and the nodes of `deleted` memories.
async fn drop_orphan_nodes_in(
    tx: &Tx,
    run_id: &str,
    article: Option<&str>,
    deleted: &[String],
) -> Result<(), TxError> {
    run(
        tx,
        tx.query(
            "DELETE brain_graph_node WHERE kind = 'article' AND origin = $run
               AND ($article = NONE OR ref_id = $article)
               AND count(<->brain_graph_edge) = 0 RETURN NONE;
             DELETE $nodes WHERE count(<->brain_graph_edge) = 0 RETURN NONE;",
        )
        .bind(("run", run_id.to_string()))
        .bind(("article", article.map(str::to_string)))
        .bind((
            "nodes",
            deleted
                .iter()
                .map(|m| rid("brain_graph_node", &memory_node(m)))
                .collect::<Vec<_>>(),
        )),
    )
    .await?;
    Ok(())
}

/// `delete_cycle_memories`: re-home the run's sources for articles another
/// retained run also holds, drop the rest, delete memories left with no
/// source and the run's brief, and clear the run's checkpoints,
/// publications and repair status. Returns the deleted memory ids.
pub async fn delete_cycle_memories_in(tx: &Tx, run_id: &str) -> Result<Vec<String>, TxError> {
    let answer = atlas_answer_id(run_id);
    let snaps = atlas_snapshots(tx).await?;
    let mut r = run(
        tx,
        tx.query(
            "SELECT id, fingerprint, answer_id, call_id FROM insight_source WHERE run_id = $run",
        )
        .bind(("run", run_id.to_string())),
    )
    .await?;
    let sources: Vec<Source> = take::<Vec<Source>>(&mut r, 0)?
        .into_iter()
        .filter(|s| s.answer_id == answer || snaps.contains(&s.answer_id))
        .collect();
    let articles: Vec<String> = sources
        .iter()
        .map(|s| s.call_id.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let homes = other_runs(tx, run_id, &articles).await?;
    let mut sql = String::new();
    let mut q_binds: Vec<(String, surrealdb::types::Value)> = Vec::new();
    let mut fingerprints = BTreeSet::new();
    for (n, s) in sources.into_iter().enumerate() {
        // `UPDATE OR IGNORE`: an existing row for the new key wins and the
        // old row goes with the run below.
        if let Some(home) = homes.get(&s.call_id) {
            let new_answer = if s.answer_id == answer {
                atlas_answer_id(home)
            } else {
                s.answer_id.clone()
            };
            let key = encode(&[&s.fingerprint, &new_answer, &s.call_id]);
            sql.push_str(&format!(
                "LET $o{n} = (SELECT * FROM ONLY $s{n});
                 IF !record::exists(type::record('insight_source', $k{n})) {{
                   CREATE type::record('insight_source', $k{n}) CONTENT
                     object::extend(object::remove($o{n}, 'id'), {{ run_id: $r{n}, answer_id: $a{n} }})
                     RETURN NONE;
                 }};"
            ));
            q_binds.push((format!("k{n}"), key.into_value()));
            q_binds.push((format!("r{n}"), home.clone().into_value()));
            q_binds.push((format!("a{n}"), new_answer.into_value()));
        }
        sql.push_str(&format!("DELETE $s{n} RETURN NONE;"));
        q_binds.push((format!("s{n}"), s.id.into_value()));
        fingerprints.insert(s.fingerprint);
    }
    if !sql.is_empty() {
        let mut q = tx.query(sql);
        for b in q_binds {
            q = q.bind(b);
        }
        run(tx, q).await?;
    }
    let mut doomed = drop_unsourced(tx, &fingerprints).await?;
    if let Some(brief) = atlas_brief_id(tx, run_id).await? {
        delete_memories_in(tx, std::slice::from_ref(&brief)).await?;
        doomed.push(brief);
    }
    run(
        tx,
        tx.query(
            "DELETE atlas_checkpoint WHERE run_id = $run RETURN NONE;
             DELETE atlas_publication WHERE run_id = $run RETURN NONE;
             DELETE type::record('atlas_repair', $run) RETURN NONE;",
        )
        .bind(("run", run_id.to_string())),
    )
    .await?;
    reconcile_deleted_memories_in(tx, &doomed).await?;
    Ok(doomed)
}

/// `reconcile_deleted_memories` for the memories this transaction deleted:
/// their derivations become `deleted`, links to those items
/// `deleted_endpoint`, the owning active snapshots `previous`, and evidence
/// loses the memory pointer. One read and one write round trip.
pub async fn reconcile_deleted_memories_in(tx: &Tx, ids: &[String]) -> Result<(), TxError> {
    if ids.is_empty() {
        return Ok(());
    }
    let mut sql = String::new();
    for i in 0..ids.len() {
        sql.push_str(&format!(
            "SELECT snapshot_id, item_json FROM insight_derivation WHERE memory_id = $m{i};"
        ));
    }
    let mut q = tx.query(sql);
    for (i, id) in ids.iter().enumerate() {
        q = q.bind((format!("m{i}"), id.clone()));
    }
    let mut r = run(tx, q).await?;
    #[derive(SurrealValue)]
    struct Derivation {
        snapshot_id: String,
        item_json: String,
    }
    let (mut snaps, mut items) = (BTreeSet::new(), BTreeSet::new());
    for i in 0..ids.len() {
        for d in take::<Vec<Derivation>>(&mut r, i)? {
            snaps.insert(d.snapshot_id);
            if let Some(item) = serde_json::from_str::<serde_json::Value>(&d.item_json)
                .ok()
                .and_then(|v| v.get("id").and_then(|i| i.as_str()).map(str::to_string))
            {
                items.insert(item);
            }
        }
    }
    run(
        tx,
        tx.query(
            "FOR $item IN $items {
               UPDATE insight_derivation_link SET status = 'deleted_endpoint' WHERE left_id = $item RETURN NONE;
               UPDATE insight_derivation_link SET status = 'deleted_endpoint' WHERE right_id = $item RETURN NONE;
             };
             FOR $s IN $snaps {
               UPDATE insight_owner_state SET lifecycle = 'previous', last_error = 'memory endpoint removed'
                 WHERE active_snapshot_id = $s RETURN NONE;
             };
             FOR $m IN $ids {
               UPDATE insight_derivation SET memory_id = NONE, status = 'deleted' WHERE memory_id = $m RETURN NONE;
               UPDATE insight_derivation_evidence SET memory_id = NONE WHERE memory_id = $m RETURN NONE;
             };",
        )
        .bind(("items", items.into_iter().collect::<Vec<_>>()))
        .bind(("snaps", snaps.into_iter().collect::<Vec<_>>()))
        .bind(("ids", ids.to_vec())),
    )
    .await?;
    Ok(())
}

/// `reconcile_deleted_articles`: reconcile sources, then mark article/atlas
/// owners whose article is gone from every run `previous` (bumping their
/// desired generation) and their active links `origin_deleted`. Returns the
/// memory ids the source reconcile retired.
pub async fn reconcile_deleted_articles_in(tx: &Tx) -> Result<Vec<String>, TxError> {
    let changed = reconcile_sources_in(tx).await?;
    #[derive(SurrealValue)]
    struct Owner {
        id: RecordId,
        owner_id: String,
    }
    let mut r = run(
        tx,
        tx.query(
            "SELECT id, owner_id FROM insight_owner_state
               WHERE owner_kind INSIDE ['article', 'atlas'] AND lifecycle != 'previous'",
        ),
    )
    .await?;
    let owners: Vec<Owner> = take(&mut r, 0)?;
    if !owners.is_empty() {
        let mut sql = String::new();
        for i in 0..owners.len() {
            sql.push_str(&format!(
                "RETURN count(SELECT id FROM atlas_article WHERE article_id = $a{i} LIMIT 1);"
            ));
        }
        let mut q = tx.query(sql);
        for (i, o) in owners.iter().enumerate() {
            q = q.bind((format!("a{i}"), o.owner_id.clone()));
        }
        let mut r = run(tx, q).await?;
        let mut gone = Vec::new();
        for (i, o) in owners.into_iter().enumerate() {
            if take::<Option<i64>>(&mut r, i)?.unwrap_or(0) == 0 {
                gone.push(o.id);
            }
        }
        run(
            tx,
            tx.query(
                "UPDATE $gone SET desired_generation += 1, lifecycle = 'previous',
                   last_error = 'origin article removed', updated_at = $now RETURN NONE;",
            )
            .bind(("gone", gone))
            .bind(("now", chrono::Utc::now().to_rfc3339())),
        )
        .await?;
    }
    run(
        tx,
        tx.query(
            "LET $snaps = (SELECT VALUE active_snapshot_id FROM insight_owner_state
               WHERE last_error = 'origin article removed' AND active_snapshot_id != '');
             FOR $s IN $snaps {
               UPDATE insight_derivation_link SET status = 'origin_deleted' WHERE snapshot_id = $s RETURN NONE;
             };",
        ),
    )
    .await?;
    Ok(changed)
}

fn dedup(ids: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    ids.into_iter().filter(|i| seen.insert(i.clone())).collect()
}

/// Run `body` as one memory publish; `T` is the caller's result.
async fn publish<T, F>(publisher: &Publisher, body: F) -> Result<T, TxError>
where
    T: Default + Send + 'static,
    F: Fn(Tx) -> futures_util::future::BoxFuture<'static, Result<(Vec<String>, T), TxError>>
        + Send
        + Sync
        + 'static,
{
    let slot = Arc::new(Mutex::new(T::default()));
    let out = slot.clone();
    let body = Arc::new(body);
    let write: MemoryWrite = Arc::new(move |tx| {
        let (body, slot) = (body.clone(), slot.clone());
        Box::pin(async move {
            let (ids, value) = body(tx).await?;
            *slot.lock().expect("atlas delete lock") = value;
            Ok(dedup(ids))
        })
    });
    write_then_index(publisher, write).await?;
    let value = std::mem::take(&mut *out.lock().expect("atlas delete lock"));
    Ok(value)
}

/// `atlas_delete_run`: the run's memories, origin contributions, articles
/// and run row in one transaction. True when the run existed.
pub async fn delete_run(publisher: &Publisher, run_id: &str) -> Result<bool, TxError> {
    let run_id = run_id.to_string();
    publish(publisher, move |tx| {
        let run_id = run_id.clone();
        Box::pin(async move {
            let mut ids = delete_cycle_memories_in(&tx, &run_id).await?;
            ids.extend(retire_origin_in(&tx, "article", &run_id).await?.1);
            run(
                &tx,
                tx.query("DELETE atlas_article WHERE run_id = $run RETURN NONE")
                    .bind(("run", run_id.clone())),
            )
            .await?;
            ids.extend(reconcile_deleted_articles_in(&tx).await?);
            let mut r = run(
                &tx,
                tx.query("RETURN (DELETE type::record('atlas_run', $run) RETURN BEFORE) != []")
                    .bind(("run", run_id.clone())),
            )
            .await?;
            let existed = take::<Option<bool>>(&mut r, 0)? == Some(true);
            ids.extend(reconcile_lifecycle_in(&tx).await?);
            drop_orphan_nodes_in(&tx, &run_id, None, &ids).await?;
            Ok((ids, existed))
        })
    })
    .await
}

/// `delete_article_insights`: drop one article's sources in one run and
/// delete the memories left with no source. Returns how many were deleted.
pub async fn delete_article_insights(
    publisher: &Publisher,
    run_id: &str,
    article_id: &str,
) -> Result<usize, TxError> {
    let (run_id, article_id) = (run_id.to_string(), article_id.to_string());
    publish(publisher, move |tx| {
        let (run_id, article_id) = (run_id.clone(), article_id.clone());
        Box::pin(async move {
            let answer = atlas_answer_id(&run_id);
            let snaps = atlas_snapshots(&tx).await?;
            let mut r = run(
                &tx,
                tx.query(
                    "SELECT id, fingerprint, answer_id, call_id FROM insight_source WHERE run_id = $run",
                )
                .bind(("run", run_id.clone())),
            )
            .await?;
            let prefixed = format!("article:{article_id}");
            let mine: Vec<Source> = take::<Vec<Source>>(&mut r, 0)?
                .into_iter()
                .filter(|s| s.call_id == article_id || s.call_id == prefixed)
                .filter(|s| s.answer_id == answer || snaps.contains(&s.answer_id))
                .collect();
            if mine.is_empty() {
                return Ok((Vec::new(), 0));
            }
            let fps: BTreeSet<String> = mine.iter().map(|s| s.fingerprint.clone()).collect();
            run(
                &tx,
                tx.query("DELETE $rows RETURN NONE; DELETE intel_link_explanation WHERE article_id = $article RETURN NONE;")
                    .bind(("rows", mine.into_iter().map(|s| s.id).collect::<Vec<_>>()))
                    .bind(("article", article_id.clone())),
            )
            .await?;
            let doomed = drop_unsourced(&tx, &fps).await?;
            // Unlike a run delete, user edits of these memories go too.
            run(
                &tx,
                tx.query("FOR $m IN $ids { DELETE type::record('insight_user_edit', $m) RETURN NONE; };")
                    .bind(("ids", doomed.clone())),
            )
            .await?;
            reconcile_deleted_memories_in(&tx, &doomed).await?;
            drop_orphan_nodes_in(&tx, &run_id, Some(&article_id), &doomed).await?;
            let n = doomed.len();
            Ok((doomed, n))
        })
    })
    .await
}

/// `atlas_delete_article`: one article row of one run, then the article and
/// lifecycle reconciles.
pub async fn delete_article(
    publisher: &Publisher,
    run_id: &str,
    article_id: &str,
) -> Result<(), TxError> {
    let (run_id, article_id) = (run_id.to_string(), article_id.to_string());
    publish(publisher, move |tx| {
        let (run_id, article_id) = (run_id.clone(), article_id.clone());
        Box::pin(async move {
            run(
                &tx,
                tx.query("DELETE type::record('atlas_article', $key) RETURN NONE")
                    .bind(("key", encode(&[&run_id, &article_id]))),
            )
            .await?;
            let mut ids = reconcile_deleted_articles_in(&tx).await?;
            ids.extend(reconcile_lifecycle_in(&tx).await?);
            drop_orphan_nodes_in(&tx, &run_id, Some(&article_id), &[]).await?;
            // `nlp::cache::remove_owner(conn, "article", id)` of the SQLite path.
            super::insight::remove_nlp_owner_in(&tx, "article", &article_id).await?;
            Ok((ids, ()))
        })
    })
    .await
}

/// `atlas_prune_expired`: finished runs started before `now - 36h` lose
/// their memories and articles; articles an Intel investigation references
/// stay, and so does a run that still holds one. Returns the pruned run ids.
pub async fn prune_expired(
    publisher: &Publisher,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<Vec<String>, TxError> {
    let cutoff = (now - chrono::Duration::hours(RETENTION_HOURS)).to_rfc3339();
    publish(publisher, move |tx| {
        let cutoff = cutoff.clone();
        Box::pin(async move {
            let mut r = run(
                &tx,
                tx.query(
                    "SELECT VALUE record::id(id) FROM atlas_run
                       WHERE started_at < $cutoff AND state NOTINSIDE ['running', 'paused'];
                     SELECT VALUE article_id FROM intel_investigation;",
                )
                .bind(("cutoff", cutoff)),
            )
            .await?;
            let runs: Vec<String> = take(&mut r, 0)?;
            let protected: Vec<String> = take(&mut r, 1)?;
            let mut ids = Vec::new();
            for run_id in &runs {
                ids.extend(delete_cycle_memories_in(&tx, run_id).await?);
                run(
                    &tx,
                    tx.query(
                        "DELETE atlas_article WHERE run_id = $run AND article_id NOTINSIDE $keep RETURN NONE;
                         IF count(SELECT id FROM atlas_article WHERE run_id = $run LIMIT 1) = 0 {
                           DELETE type::record('atlas_run', $run) RETURN NONE;
                         };",
                    )
                    .bind(("run", run_id.clone()))
                    .bind(("keep", protected.clone())),
                )
                .await?;
            }
            ids.extend(reconcile_deleted_articles_in(&tx).await?);
            ids.extend(reconcile_lifecycle_in(&tx).await?);
            for run_id in &runs {
                drop_orphan_nodes_in(&tx, run_id, None, &ids).await?;
            }
            Ok((ids, runs))
        })
    })
    .await
}
