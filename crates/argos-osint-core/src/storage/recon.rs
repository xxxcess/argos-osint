//! Recon threads and messages on SurrealDB (migration phase 4a): the
//! `recon_thread`, `recon_message` and `recon_message_memory` tables, plus
//! the Brain edge tables `brain_memory_endpoint` and `brain_memory_actor_ref`
//! that Brain browse needs. These are the SurrealDB side of the legacy store
//! tables with the same names (plural).
//!
//! the legacy store's foreign keys become field assertions (a child row needs its
//! parent) and its `ON DELETE CASCADE`s become DELETE events, as in
//! `atlas_metadata`:
//!
//! - deleting a thread deletes its messages;
//! - deleting a message deletes its memory links;
//! - deleting a memory deletes its endpoints and actor refs.
//!
//! Record keys: `recon_thread` and `recon_message` by id,
//! `recon_message_memory` by `[message, memory]`, `brain_memory_endpoint`
//! by `[memory, side]`, and `brain_memory_actor_ref` by
//! `[memory, actor, role]`.
//!
//! `recon_runs`, `osint_calls`, `recon_thread_entities` and the
//! investigation tables are not ported yet. `delete_thread` runs the
//! legacy store cascade keeping insights; `delete_thread_with_insights` also
//! removes the insights the thread alone sourced.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Result};
use surrealdb::types::SurrealValue;

use super::jobs::{rid, run};
use super::repo::Repository;
use super::tx::{run_tx, RetryPolicy, Tx, TxError};
use crate::brain::{Memory, MemorySource};
use crate::recon::{Message, Thread};

/// Recon schema (idempotent).
pub const SCHEMA: &str = "DEFINE TABLE IF NOT EXISTS recon_thread SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS title ON recon_thread TYPE string;
DEFINE FIELD IF NOT EXISTS created_at ON recon_thread TYPE string;
DEFINE FIELD IF NOT EXISTS updated_at ON recon_thread TYPE string;
DEFINE FIELD IF NOT EXISTS draft ON recon_thread TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS scroll ON recon_thread TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS deleted ON recon_thread TYPE bool DEFAULT false;
DEFINE FIELD IF NOT EXISTS recall_insights ON recon_thread TYPE bool DEFAULT false;
DEFINE INDEX IF NOT EXISTS recon_thread_updated ON recon_thread FIELDS updated_at;

DEFINE TABLE IF NOT EXISTS recon_message SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS thread_id ON recon_message TYPE string
  ASSERT record::exists(type::record('recon_thread', $value));
DEFINE FIELD IF NOT EXISTS sequence ON recon_message TYPE int;
DEFINE FIELD IF NOT EXISTS role ON recon_message TYPE string;
DEFINE FIELD IF NOT EXISTS content ON recon_message TYPE string;
DEFINE FIELD IF NOT EXISTS run_id ON recon_message TYPE option<string>;
DEFINE FIELD IF NOT EXISTS created_at ON recon_message TYPE string;
DEFINE INDEX IF NOT EXISTS recon_message_sequence ON recon_message FIELDS thread_id, sequence UNIQUE;

DEFINE TABLE IF NOT EXISTS recon_message_memory SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS message_id ON recon_message_memory TYPE string
  ASSERT record::exists(type::record('recon_message', $value));
DEFINE FIELD IF NOT EXISTS memory_id ON recon_message_memory TYPE string;
DEFINE FIELD IF NOT EXISTS ordinal ON recon_message_memory TYPE int;
DEFINE INDEX IF NOT EXISTS recon_message_memory_message ON recon_message_memory FIELDS message_id;

DEFINE TABLE IF NOT EXISTS brain_memory_endpoint SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS memory_id ON brain_memory_endpoint TYPE string
  ASSERT record::exists(type::record('memory', $value));
DEFINE FIELD IF NOT EXISTS side ON brain_memory_endpoint TYPE string
  ASSERT $value INSIDE ['source', 'target'];
DEFINE FIELD IF NOT EXISTS endpoint_kind ON brain_memory_endpoint TYPE string
  ASSERT $value INSIDE ['memory', 'article', 'source'];
DEFINE FIELD IF NOT EXISTS ref_id ON brain_memory_endpoint TYPE string;
DEFINE FIELD IF NOT EXISTS origin ON brain_memory_endpoint TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS revision ON brain_memory_endpoint TYPE string DEFAULT '';
DEFINE INDEX IF NOT EXISTS brain_memory_endpoint_ref ON brain_memory_endpoint FIELDS endpoint_kind, ref_id;
DEFINE INDEX IF NOT EXISTS brain_memory_endpoint_memory ON brain_memory_endpoint FIELDS memory_id;

DEFINE TABLE IF NOT EXISTS brain_memory_actor_ref SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS memory_id ON brain_memory_actor_ref TYPE string
  ASSERT record::exists(type::record('memory', $value));
DEFINE FIELD IF NOT EXISTS actor_id ON brain_memory_actor_ref TYPE string;
DEFINE FIELD IF NOT EXISTS role ON brain_memory_actor_ref TYPE string;
DEFINE FIELD IF NOT EXISTS binding_state ON brain_memory_actor_ref TYPE string DEFAULT 'provisional';
DEFINE FIELD IF NOT EXISTS provenance_json ON brain_memory_actor_ref TYPE string DEFAULT '{}';
DEFINE INDEX IF NOT EXISTS brain_memory_actor_ref_actor ON brain_memory_actor_ref FIELDS actor_id;
DEFINE INDEX IF NOT EXISTS brain_memory_actor_ref_memory ON brain_memory_actor_ref FIELDS memory_id;

DEFINE EVENT IF NOT EXISTS recon_thread_cascade ON TABLE recon_thread
  WHEN $event = 'DELETE' THEN {
    DELETE recon_message WHERE thread_id = record::id($before.id) RETURN NONE;
  };
DEFINE EVENT IF NOT EXISTS recon_message_cascade ON TABLE recon_message
  WHEN $event = 'DELETE' THEN {
    DELETE recon_message_memory WHERE message_id = record::id($before.id) RETURN NONE;
  };
DEFINE EVENT IF NOT EXISTS memory_edge_cascade ON TABLE memory
  WHEN $event = 'DELETE' THEN {
    DELETE brain_memory_endpoint WHERE memory_id = record::id($before.id) RETURN NONE;
    DELETE brain_memory_actor_ref WHERE memory_id = record::id($before.id) RETURN NONE;
  };";

#[derive(Debug, Clone, SurrealValue)]
struct ThreadRow {
    id: String,
    title: String,
    created_at: String,
    updated_at: String,
    draft: String,
    scroll: i64,
    recall_insights: bool,
}

/// Create or replace a thread.
pub async fn put_thread(repo: &Repository, t: &Thread) -> Result<()> {
    repo.db()
        .query(
            "UPSERT $t SET title = $title, created_at = $c, updated_at = $u, draft = $d, \
             scroll = $s, recall_insights = $r, deleted = false RETURN NONE",
        )
        .bind(("t", rid("recon_thread", &t.id)))
        .bind(("title", t.title.clone()))
        .bind(("c", t.created_at.clone()))
        .bind(("u", t.updated_at.clone()))
        .bind(("d", t.draft.clone()))
        .bind(("s", t.scroll))
        .bind(("r", t.recall_insights))
        .await?
        .check()?;
    Ok(())
}

/// A live (not deleted) thread.
pub async fn get_thread(repo: &Repository, id: &str) -> Result<Option<Thread>> {
    get_thread_on(repo.db(), id).await
}

/// [`get_thread`] on the client or inside a caller's transaction.
pub(crate) async fn get_thread_on(db: &impl super::tx::Reads, id: &str) -> Result<Option<Thread>> {
    let mut r = db
        .read(
            "SELECT meta::id(id) AS id, title, created_at, updated_at, draft, scroll, \
             recall_insights FROM ONLY $t WHERE deleted = false",
        )
        .bind(("t", rid("recon_thread", id)))
        .await?
        .check()?;
    let row: Option<ThreadRow> = r.take(0usize)?;
    Ok(row.map(|t| Thread {
        id: t.id,
        title: t.title,
        created_at: t.created_at,
        updated_at: t.updated_at,
        draft: t.draft,
        scroll: t.scroll,
        recall_insights: t.recall_insights,
    }))
}

/// Append a message (the thread must exist; sequences are unique per thread).
pub async fn put_message(repo: &Repository, m: &Message) -> Result<()> {
    repo.db()
        .query(
            "CREATE $m SET thread_id = $t, sequence = $s, role = $role, content = $c, \
             run_id = $run, created_at = $at RETURN NONE",
        )
        .bind(("m", rid("recon_message", &m.id)))
        .bind(("t", m.thread_id.clone()))
        .bind(("s", m.sequence))
        .bind(("role", m.role.clone()))
        .bind(("c", m.content.clone()))
        .bind(("run", m.run_id.clone()))
        .bind(("at", m.created_at.clone()))
        .await?
        .check()?;
    Ok(())
}

/// Replace the memories an answer cited, in order.
pub async fn link_message_memories(
    repo: &Repository,
    message_id: &str,
    memory_ids: &[String],
) -> Result<()> {
    let (message, ids) = (message_id.to_string(), memory_ids.to_vec());
    run_tx(repo.db(), RetryPolicy::default(), |tx| {
        let (message, ids) = (message.clone(), ids.clone());
        async move {
            run(
                &tx,
                tx.query(
                    "DELETE recon_message_memory WHERE message_id = $m RETURN NONE;
                     FOR $i IN array::range(0, array::len($ids)) {
                       CREATE type::record('recon_message_memory', [$m, $ids[$i]])
                         SET message_id = $m, memory_id = $ids[$i], ordinal = $i RETURN NONE;
                     };",
                )
                .bind(("m", message))
                .bind(("ids", ids)),
            )
            .await?;
            Ok(())
        }
    })
    .await
    .map_err(|e| anyhow!("link message memories: {e}"))?;
    Ok(())
}

#[derive(Debug, Clone, SurrealValue)]
struct LinkRow {
    message_id: String,
    memory_id: String,
    text: Option<String>,
    category: Option<String>,
    pinned: Option<bool>,
    created_at: Option<String>,
    source_json: Option<String>,
}

/// Per message of the thread, the memories it cited in order. A memory
/// that is gone reads as "no longer stored", as in the legacy store.
pub async fn answer_memories(
    repo: &Repository,
    thread_id: &str,
) -> Result<HashMap<String, Vec<Memory>>> {
    let mut r = repo
        .db()
        .query(
            "LET $msgs = (SELECT VALUE meta::id(id) FROM recon_message WHERE thread_id = $t);
             SELECT message_id, memory_id, ordinal,
                    type::record('memory', memory_id).text AS text,
                    type::record('memory', memory_id).category AS category,
                    type::record('memory', memory_id).pinned AS pinned,
                    (IF record::exists(type::record('memory', memory_id)) THEN time::format(type::record('memory', memory_id).created_at, '%+') END) AS created_at,
                    type::record('memory', memory_id).source_json AS source_json
               FROM recon_message_memory WHERE message_id INSIDE $msgs
              ORDER BY ordinal;",
        )
        .bind(("t", thread_id.to_string()))
        .await?
        .check()?;
    let rows: Vec<LinkRow> = r.take(1usize)?;
    let mut out: HashMap<String, Vec<Memory>> = HashMap::new();
    for row in rows {
        let memory = match (
            row.text,
            row.category,
            row.pinned,
            row.created_at,
            row.source_json,
        ) {
            (Some(text), Some(category), Some(pinned), Some(created_at), Some(source_json)) => {
                Memory {
                    source: serde_json::from_str(&source_json).unwrap_or(MemorySource {
                        app: "argos".into(),
                        conversation_id: String::new(),
                        message_id: None,
                        reference: None,
                    }),
                    id: row.memory_id,
                    text,
                    category,
                    pinned,
                    created_at,
                }
            }
            _ => Memory {
                id: row.memory_id,
                text: "This memory is no longer stored.".into(),
                category: "missing".into(),
                pinned: false,
                created_at: String::new(),
                source: MemorySource {
                    app: "argos".into(),
                    conversation_id: String::new(),
                    message_id: None,
                    reference: None,
                },
            },
        };
        out.entry(row.message_id).or_default().push(memory);
    }
    Ok(out)
}

/// One typed endpoint of a Link/Related memory.
#[derive(Debug, Clone)]
pub struct Endpoint<'a> {
    pub memory_id: &'a str,
    pub side: &'a str,
    pub endpoint_kind: &'a str,
    pub ref_id: &'a str,
    pub origin: &'a str,
    pub revision: &'a str,
}

pub async fn put_endpoint(repo: &Repository, e: &Endpoint<'_>) -> Result<()> {
    repo.db()
        .query(
            "UPSERT type::record('brain_memory_endpoint', [$m, $side]) SET memory_id = $m, side = $side, endpoint_kind = $kind, ref_id = $ref, \
             origin = $origin, revision = $rev RETURN NONE",
        )
        .bind(("m", e.memory_id.to_string()))
        .bind(("side", e.side.to_string()))
        .bind(("kind", e.endpoint_kind.to_string()))
        .bind(("ref", e.ref_id.to_string()))
        .bind(("origin", e.origin.to_string()))
        .bind(("rev", e.revision.to_string()))
        .await?
        .check()?;
    Ok(())
}

pub async fn put_actor_ref(
    repo: &Repository,
    memory_id: &str,
    actor_id: &str,
    role: &str,
) -> Result<()> {
    repo.db()
        .query("UPSERT type::record('brain_memory_actor_ref', [$m, $a, $role]) SET memory_id = $m, actor_id = $a, role = $role RETURN NONE")
        .bind(("m", memory_id.to_string()))
        .bind(("a", actor_id.to_string()))
        .bind(("role", role.to_string()))
        .await?
        .check()?;
    Ok(())
}

/// `Store::delete_thread` (keeping insights), in one transaction: the
/// thread's actor mentions go; its insight sources are detached and marked
/// `deleted_origin`; in-flight calls are cancelled; its runs' extraction
/// jobs and model operations, its strategies, messages (with their
/// evidence and memory links), runs, entity links, message counter and
/// the `last_thread` pointer are deleted; its calls are detached, and
/// detached Recon calls no insight source or derivation cites are
/// deleted. Manual calls and the tool cache (keyed by tool inputs, not
/// thread) stay, as in the legacy store. Returns false when unknown or already
/// deleted.
pub async fn delete_thread(repo: &Repository, id: &str) -> Result<bool> {
    let id = id.to_string();
    let done = run_tx(repo.db(), RetryPolicy::default(), |tx| {
        let id = id.clone();
        async move { delete_thread_in(&tx, &id).await }
    })
    .await
    .map_err(|e| anyhow!("delete thread: {e}"))?;
    Ok(done.value)
}

/// [`delete_thread`]'s cascade in `tx`.
pub async fn delete_thread_in(tx: &Tx, id: &str) -> Result<bool, TxError> {
    let mut r = run(
        tx,
        tx.query(
            "LET $th = type::record('recon_thread', $id);
             LET $found = record::exists($th) AND (SELECT VALUE deleted FROM ONLY $th) != true;
             IF $found {
               LET $runs = SELECT VALUE meta::id(id) FROM recon_run WHERE thread_id = $id;
               FOR $run IN $runs {
                 UPDATE type::record('insight_owner_state', ['recon', $run])
                   SET desired_generation = desired_generation + 1, lifecycle = 'previous',
                     last_error = 'origin investigation deleted', updated_at = $now RETURN NONE;
               };
               LET $snaps = SELECT VALUE active_snapshot_id FROM insight_owner_state
                 WHERE owner_kind = 'recon' AND last_error = 'origin investigation deleted'
                   AND active_snapshot_id != '';
               FOR $s IN $snaps {
                 UPDATE insight_derivation_link SET status = 'origin_deleted'
                   WHERE snapshot_id = $s RETURN NONE;
               };
               DELETE brain_actor_mention WHERE origin_kind = 'thread' AND origin_id = $id RETURN NONE;
               UPDATE insight_source SET deleted_origin = true, thread_id = NONE, run_id = NONE,
                 answer_id = 'deleted-origin' WHERE thread_id = $id RETURN NONE;
               UPDATE osint_call SET status = 'cancelled'
                 WHERE thread_id = $id AND status INSIDE ['queued', 'running'] RETURN NONE;
               DELETE extraction_job WHERE run_id INSIDE $runs RETURN NONE;
               DELETE recon_model_operation WHERE run_id INSIDE $runs RETURN NONE;
               DELETE investigation_strategy WHERE thread_id = $id RETURN NONE;
               DELETE recon_message WHERE thread_id = $id RETURN NONE;
               DELETE recon_run WHERE thread_id = $id RETURN NONE;
               UPDATE osint_call SET thread_id = NONE, turn_id = NONE, run_id = NONE
                 WHERE thread_id = $id RETURN NONE;
               LET $cited = SELECT VALUE call_id FROM insight_source;
               LET $derived = SELECT VALUE source_id FROM insight_derivation_evidence
                 WHERE string::starts_with(source_id, 'call:');
               FOR $c IN (SELECT VALUE meta::id(id) FROM osint_call
                   WHERE origin = 'recon' AND thread_id = NONE AND run_id = NONE) {
                 LET $key = 'call:' + $c;
                 IF $c NOTINSIDE $cited AND array::len(array::filter($derived,
                     |$s| $s == $key OR string::starts_with($s, $key + '#page'))) == 0 {
                   DELETE type::record('osint_call', $c) RETURN NONE;
                 };
               };
               DELETE $th RETURN NONE;
               DELETE type::record('counter', $counter) RETURN NONE;
               DELETE app_state:last_thread WHERE value = $id RETURN NONE;
             };
             RETURN $found;",
        )
        .bind(("id", id.to_string()))
        .bind(("counter", super::recon_threads::sequence_counter(id)))
        .bind(("now", chrono::Utc::now().to_rfc3339())),
    )
    .await?;
    let last = r.num_statements() - 1;
    let found = super::jobs::take::<Option<bool>>(&mut r, last)? == Some(true);
    if found {
        super::insight::remove_nlp_owner_in(tx, "thread", id).await?;
    }
    Ok(found)
}

/// The memories a thread's deletion with insights removes
/// (`investigation_memory_ids`): claims a source of this thread cites, or
/// memories whose source is this conversation, unless a source of another
/// live thread also cites the claim.
const DOOMED_SURQL: &str = "LET $mine = array::distinct(
       SELECT VALUE fingerprint FROM insight_source WHERE thread_id = $id);
     LET $shared = array::distinct(
       SELECT VALUE fingerprint FROM insight_source
         WHERE thread_id != NONE AND thread_id != $id AND fingerprint INSIDE $mine);
     LET $claimed = array::distinct(SELECT VALUE memory_id FROM insight_claim
         WHERE record::id(id) INSIDE $mine AND record::id(id) NOTINSIDE $shared);
     LET $kept = array::distinct(SELECT VALUE memory_id FROM insight_claim
         WHERE record::id(id) INSIDE $shared);
     LET $conv = SELECT VALUE record::id(id) FROM memory WHERE string::contains(source_json, $conv_key);
     RETURN array::filter(array::distinct(array::concat($claimed, $conv)), |$m| $m NOTINSIDE $kept);";

/// `Store::delete_thread(tid, true)`: [`delete_thread`]'s cascade plus the
/// insights the thread alone sourced, in one publisher transaction. Each
/// doomed memory goes through the user-delete cleanup
/// (`insight::delete_memory_in`: tombstone, provenance downgrade, claims,
/// sources, relations, user edits, graph summary); the thread's remaining
/// sources on shared claims are deleted, derivations whose memory is gone
/// are reconciled, and the removals are queued for the vector index.
pub async fn delete_thread_with_insights(repo: &Repository, id: &str) -> Result<bool> {
    let found = Arc::new(Mutex::new(false));
    let (tid, slot) = (id.to_string(), found.clone());
    let write: super::memory::MemoryWrite = Arc::new(move |tx| {
        let (tid, slot) = (tid.clone(), slot.clone());
        Box::pin(async move {
            let mut r = run(
                &tx,
                tx.query(
                    "LET $th = type::record('recon_thread', $id);
                     RETURN record::exists($th) AND (SELECT VALUE deleted FROM ONLY $th) != true;",
                )
                .bind(("id", tid.clone())),
            )
            .await?;
            if super::jobs::take::<Option<bool>>(&mut r, 1)? != Some(true) {
                return Ok(Vec::new());
            }
            let mut r = run(
                &tx,
                tx.query(DOOMED_SURQL)
                    .bind(("id", tid.clone()))
                    .bind(("conv_key", format!("\"conversation_id\":\"{tid}\""))),
            )
            .await?;
            let last = r.num_statements() - 1;
            let doomed: Vec<String> = super::jobs::take(&mut r, last)?;
            let mut removed = Vec::new();
            for memory in doomed {
                if super::insight::hard_delete_memory_in(&tx, &memory).await? {
                    removed.push(memory);
                }
            }
            run(
                &tx,
                tx.query("DELETE insight_source WHERE thread_id = $id RETURN NONE;")
                    .bind(("id", tid.clone())),
            )
            .await?;
            let gone = delete_thread_in(&tx, &tid).await?;
            super::insight_owner::reconcile_deleted_memories_in(&tx).await?;
            *slot.lock().expect("delete lock") = gone;
            Ok(removed)
        })
    });
    super::memory::write_then_index(repo.publisher(), write)
        .await
        .map_err(|e| anyhow!("delete thread with insights: {e}"))?;
    let gone = *found.lock().expect("delete lock");
    if gone {
        super::jobs::cleanup_batch(repo.db(), chrono::Utc::now())
            .await
            .map_err(|e| anyhow!("job cleanup: {e}"))?;
    }
    Ok(gone)
}
