//! Reads of retained investigation threads and messages.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use surrealdb::types::SurrealValue;

use super::jobs::rid;
use super::repo::Repository;
use super::tx::Reads;

/// A persisted investigation thread.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Thread {
    pub id: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
    pub draft: String,
    pub scroll: i64,
    /// When set, synthesis writes Brain claims for answers produced from then on.
    #[serde(default)]
    pub recall_insights: bool,
}

/// One message in a persisted investigation thread.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub thread_id: String,
    pub sequence: i64,
    pub role: String,
    pub content: String,
    pub run_id: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, SurrealValue)]
pub(crate) struct ThreadRow {
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

/// A live (not deleted) thread.
pub async fn get_thread(repo: &Repository, id: &str) -> Result<Option<Thread>> {
    get_thread_on(repo.db(), id).await
}

/// [`get_thread`] on the client or inside a caller's transaction.
pub(crate) async fn get_thread_on(db: &impl Reads, id: &str) -> Result<Option<Thread>> {
    let mut r = db
        .read(
            "SELECT meta::id(id) AS id, title, created_at, updated_at, draft, scroll, \
             recall_insights FROM ONLY $t WHERE deleted = false",
        )
        .bind(("t", rid("recon_thread", id)))
        .await?
        .check()?;
    let row: Option<ThreadRow> = r.take(0usize)?;
    Ok(row.map(Thread::from))
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
