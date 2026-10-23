//! Recon investigations on SurrealDB (migration phase 5, Recon remainder
//! part 1): the thread and message operations the Recon callers use on the
//! SQLite `Store` (`new_thread`, `list_threads`, `rename_thread`,
//! `set_recall_insights`, `save_draft`, `select_thread`/`last_thread`,
//! `list_messages`, `add_message`), the thread entity links
//! (`recon_thread_entities` → `recon_thread_entity`) and the tool
//! preferences (`osint_preferences` → `osint_preference`).
//!
//! Message sequences come from an ordered counter (`counter:[recon_message,
//! <thread>]`) allocated in the message's transaction, so concurrent
//! appends never share a sequence. The counter is first raised to the
//! thread's highest stored sequence, so messages written before it existed
//! (imports, `recon::put_message`) are never overtaken.
//!
//! Record keys: `recon_thread_entity` by `[thread, entity, source_call]`
//! (an empty source call when none, as in SQLite), `osint_preference` by
//! tool id. Deleting a thread deletes its entity links (event).

use anyhow::{anyhow, ensure, Result};
use chrono::Utc;
use surrealdb::types::SurrealValue;

use super::entities::{ensure_recon_entity_in, ReconEntity};
use super::jobs::{rid, run, take};
use super::pipeline_inputs::{app_state_get, app_state_set};
use super::recon::get_thread;
use super::repo::Repository;
use super::tx::{run_tx, RetryPolicy};
use crate::recon::{Message, Thread, PLACEHOLDER_TITLE};

/// Schema (idempotent).
pub const SCHEMA: &str = "DEFINE TABLE IF NOT EXISTS recon_thread_entity SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS thread_id ON recon_thread_entity TYPE string
  ASSERT record::exists(type::record('recon_thread', $value));
DEFINE FIELD IF NOT EXISTS entity_id ON recon_thread_entity TYPE string;
DEFINE FIELD IF NOT EXISTS source_call_id ON recon_thread_entity TYPE string DEFAULT '';
DEFINE INDEX IF NOT EXISTS recon_thread_entity_thread ON recon_thread_entity FIELDS thread_id;
DEFINE INDEX IF NOT EXISTS recon_thread_entity_entity ON recon_thread_entity FIELDS entity_id;

DEFINE TABLE IF NOT EXISTS osint_preference SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS enabled ON osint_preference TYPE bool;

DEFINE EVENT IF NOT EXISTS recon_thread_entity_cascade ON TABLE recon_thread
  WHEN $event = 'DELETE' THEN {
    DELETE recon_thread_entity WHERE thread_id = record::id($before.id) RETURN NONE;
  };";

fn now() -> String {
    Utc::now().to_rfc3339()
}

pub(crate) fn sequence_counter(thread_id: &str) -> String {
    format!("recon_message:{thread_id}")
}

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

impl From<ThreadRow> for Thread {
    fn from(t: ThreadRow) -> Self {
        Thread {
            id: t.id,
            title: t.title,
            created_at: t.created_at,
            updated_at: t.updated_at,
            draft: t.draft,
            scroll: t.scroll,
            recall_insights: t.recall_insights,
        }
    }
}

/// `Store::new_thread`: a blank title stores the placeholder.
pub async fn new_thread(repo: &Repository, title: &str) -> Result<Thread> {
    let time = now();
    let thread = Thread {
        id: crate::recon::id("thread"),
        title: if title.trim().is_empty() {
            PLACEHOLDER_TITLE.into()
        } else {
            title.trim().into()
        },
        created_at: time.clone(),
        updated_at: time,
        draft: String::new(),
        scroll: 0,
        recall_insights: false,
    };
    repo.db()
        .query(
            "CREATE $t SET title = $title, created_at = $c, updated_at = $c, draft = '', \
             scroll = 0, recall_insights = false, deleted = false RETURN NONE",
        )
        .bind(("t", rid("recon_thread", &thread.id)))
        .bind(("title", thread.title.clone()))
        .bind(("c", thread.created_at.clone()))
        .await?
        .check()?;
    Ok(thread)
}

/// `Store::list_threads`: live threads whose title, or one of whose linked
/// entities' canonical name, contains `search` (case-insensitive, as
/// SQLite `LIKE`), newest update first.
pub async fn list_threads(repo: &Repository, search: &str) -> Result<Vec<Thread>> {
    let mut r = repo
        .db()
        .query(
            "LET $n = string::lowercase($s);
             LET $hits = IF $n = '' THEN [] ELSE array::distinct(
               SELECT VALUE thread_id FROM recon_thread_entity
                WHERE string::contains(string::lowercase(
                  type::record('recon_entity', entity_id).canonical ?? ''), $n)) END;
             SELECT meta::id(id) AS id, title, created_at, updated_at, draft, scroll,
                    recall_insights
               FROM recon_thread
              WHERE deleted = false
                AND ($n = '' OR string::contains(string::lowercase(title), $n)
                     OR meta::id(id) INSIDE $hits)
              ORDER BY updated_at DESC;",
        )
        .bind(("s", search.trim().to_string()))
        .await?
        .check()?;
    let rows: Vec<ThreadRow> = r.take(2usize)?;
    Ok(rows.into_iter().map(Thread::from).collect())
}

async fn updated(r: surrealdb::IndexedResults) -> Result<bool> {
    let mut r = r.check()?;
    let ids: Vec<String> = r.take(0usize)?;
    Ok(!ids.is_empty())
}

/// `Store::set_recall_insights`. False when the thread is unknown or deleted.
pub async fn set_recall_insights(repo: &Repository, id: &str, on: bool) -> Result<bool> {
    updated(
        repo.db()
            .query("UPDATE $t SET recall_insights = $on WHERE deleted = false RETURN VALUE meta::id(id)")
            .bind(("t", rid("recon_thread", id)))
            .bind(("on", on))
            .await?,
    )
    .await
}

/// `Store::rename_thread` (a blank title is an error).
pub async fn rename_thread(repo: &Repository, id: &str, title: &str) -> Result<bool> {
    ensure!(!title.trim().is_empty(), "title is empty");
    updated(
        repo.db()
            .query("UPDATE $t SET title = $title, updated_at = $u WHERE deleted = false RETURN VALUE meta::id(id)")
            .bind(("t", rid("recon_thread", id)))
            .bind(("title", title.trim().to_string()))
            .bind(("u", now()))
            .await?,
    )
    .await
}

/// `Store::save_draft` (no-op on an unknown thread).
pub async fn save_draft(repo: &Repository, id: &str, draft: &str, scroll: i64) -> Result<()> {
    updated(
        repo.db()
            .query("UPDATE $t SET draft = $d, scroll = $s WHERE deleted = false RETURN VALUE meta::id(id)")
            .bind(("t", rid("recon_thread", id)))
            .bind(("d", draft.to_string()))
            .bind(("s", scroll))
            .await?,
    )
    .await?;
    Ok(())
}

/// `Store::select_thread`: remember the thread as the last one opened.
pub async fn select_thread(repo: &Repository, id: &str) -> Result<()> {
    ensure!(get_thread(repo, id).await?.is_some(), "thread not found");
    app_state_set(repo, "last_thread", id).await
}

/// `Store::last_thread`: the last selected thread, while it is live.
pub async fn last_thread(repo: &Repository) -> Result<Option<String>> {
    let Some(id) = app_state_get(repo, "last_thread").await? else {
        return Ok(None);
    };
    Ok(get_thread(repo, &id).await?.map(|t| t.id))
}

#[derive(Debug, Clone, SurrealValue)]
struct MessageRow {
    id: String,
    thread_id: String,
    sequence: i64,
    role: String,
    content: String,
    run_id: Option<String>,
    created_at: String,
}

/// `Store::list_messages`, in sequence order.
pub async fn list_messages(repo: &Repository, thread_id: &str) -> Result<Vec<Message>> {
    let mut r = repo
        .db()
        .query(
            "SELECT meta::id(id) AS id, thread_id, sequence, role, content, run_id, created_at
               FROM recon_message WHERE thread_id = $t ORDER BY sequence",
        )
        .bind(("t", thread_id.to_string()))
        .await?
        .check()?;
    let rows: Vec<MessageRow> = r.take(0usize)?;
    Ok(rows
        .into_iter()
        .map(|m| Message {
            id: m.id,
            thread_id: m.thread_id,
            sequence: m.sequence,
            role: m.role,
            content: m.content,
            run_id: m.run_id,
            created_at: m.created_at,
        })
        .collect())
}

/// `Store::add_message`: append to a live thread with the next sequence
/// from the thread's counter, and touch the thread, in one transaction.
pub async fn add_message(
    repo: &Repository,
    thread_id: &str,
    role: &str,
    content: &str,
    run_id: Option<&str>,
) -> Result<Message> {
    let base = Message {
        id: crate::recon::id("msg"),
        thread_id: thread_id.into(),
        sequence: 0,
        role: role.into(),
        content: content.into(),
        run_id: run_id.map(str::to_string),
        created_at: now(),
    };
    let m = base.clone();
    let done = run_tx(repo.db(), RetryPolicy::default(), |tx| {
        let m = m.clone();
        async move {
            let mut r = run(
                &tx,
                tx.query(
                    "IF (SELECT VALUE deleted FROM ONLY type::record('recon_thread', $t)) != false {
                       THROW 'thread not found';
                     };
                     LET $top = math::max(SELECT VALUE sequence FROM recon_message WHERE thread_id = $t) ?? 0;
                     LET $n = (UPSERT $c SET n = math::max([n ?? 0, $top]) + 1 RETURN VALUE n)[0];
                     CREATE $m SET thread_id = $t, sequence = $n, role = $role, content = $body,
                       run_id = $run, created_at = $at RETURN NONE;
                     UPDATE type::record('recon_thread', $t) SET updated_at = $at RETURN NONE;
                     RETURN $n;",
                )
                .bind(("t", m.thread_id.clone()))
                .bind(("c", rid("counter", &sequence_counter(&m.thread_id))))
                .bind(("m", rid("recon_message", &m.id)))
                .bind(("role", m.role.clone()))
                .bind(("body", m.content.clone()))
                .bind(("run", m.run_id.clone()))
                .bind(("at", m.created_at.clone())),
            )
            .await?;
            let last = r.num_statements() - 1;
            take::<Option<i64>>(&mut r, last)
        }
    })
    .await
    .map_err(|e| anyhow!("add message: {e}"))?;
    let sequence = done
        .value
        .ok_or_else(|| anyhow!("add message: no sequence"))?;
    Ok(Message { sequence, ..base })
}

/// `Store::link_entity` (storage part): ensure the `recon_entity`
/// (`kind:canonical`, label = canonical) and link it to the thread once
/// per source call. The Brain actor mention (`record_thread_actor`) moves
/// with the Brain browse write path.
pub async fn link_entity(
    repo: &Repository,
    thread_id: &str,
    kind: &str,
    canonical: &str,
    source_call_id: Option<&str>,
) -> Result<()> {
    let entity = ReconEntity {
        id: format!("{kind}:{canonical}"),
        kind: kind.into(),
        namespace: String::new(),
        canonical: canonical.into(),
        label: canonical.into(),
    };
    let (t, call) = (
        thread_id.to_string(),
        source_call_id.unwrap_or("").to_string(),
    );
    run_tx(repo.db(), RetryPolicy::default(), |tx| {
        let (entity, t, call) = (entity.clone(), t.clone(), call.clone());
        async move {
            ensure_recon_entity_in(&tx, &entity).await?;
            run(
                &tx,
                tx.query(
                    "UPSERT type::record('recon_thread_entity', [$t, $e, $call])
                       SET thread_id = $t, entity_id = $e, source_call_id = $call RETURN NONE",
                )
                .bind(("t", t))
                .bind(("e", entity.id))
                .bind(("call", call)),
            )
            .await?;
            Ok(())
        }
    })
    .await
    .map_err(|e| anyhow!("link entity: {e}"))?;
    Ok(())
}

#[derive(Debug, Clone, SurrealValue)]
struct EntityRow {
    kind: String,
    canonical: String,
}

/// `Store::thread_entities`: distinct `(kind, canonical)`, sorted.
pub async fn thread_entities(repo: &Repository, thread_id: &str) -> Result<Vec<(String, String)>> {
    let mut r = repo
        .db()
        .query(
            "LET $ids = array::distinct(SELECT VALUE entity_id FROM recon_thread_entity WHERE thread_id = $t);
             SELECT kind, canonical FROM recon_entity
              WHERE meta::id(id) INSIDE $ids ORDER BY kind, canonical;",
        )
        .bind(("t", thread_id.to_string()))
        .await?
        .check()?;
    let rows: Vec<EntityRow> = r.take(1usize)?;
    Ok(rows.into_iter().map(|e| (e.kind, e.canonical)).collect())
}

/// `Store::tool_enabled`: the stored preference, else the tool's default.
pub async fn tool_enabled(repo: &Repository, tool_id: &str) -> Result<bool> {
    let mut r = repo
        .db()
        .query("SELECT VALUE enabled FROM ONLY $p")
        .bind(("p", rid("osint_preference", tool_id)))
        .await?
        .check()?;
    let v: Option<bool> = r.take(0usize)?;
    Ok(v.unwrap_or_else(|| crate::osint::default_enabled(tool_id)))
}

/// `Store::set_tool_enabled` (unknown tools are an error).
pub async fn set_tool_enabled(repo: &Repository, tool_id: &str, enabled: bool) -> Result<()> {
    ensure!(crate::osint::definition(tool_id).is_some(), "unknown tool");
    repo.db()
        .query("UPSERT $p SET enabled = $on RETURN NONE")
        .bind(("p", rid("osint_preference", tool_id)))
        .bind(("on", enabled))
        .await?
        .check()?;
    Ok(())
}
