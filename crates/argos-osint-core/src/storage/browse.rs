//! Brain browse and single-memory operations on the async repository, for
//! the TUI's Brain screens: `Store::get_memory`, `delete_memory` and
//! `brain_browse` (Claims plus Actor groups), with the same shapes as SQLite.
//!
//! Every SurrealDB memory has a `memory_kind` (default `claim`), so there is
//! no legacy, untyped insight-backed claim fallback. Actor-group counts and
//! associations (`brain_browse_memories`) are in `browse_assoc`.

use std::collections::BTreeMap;

use anyhow::{anyhow, Result};
use surrealdb::types::SurrealValue;

use super::jobs::{rid, run};
use super::repo::Repository;
use super::tx::{run_tx, RetryPolicy};
use crate::brain::Memory;
use crate::brain_actors::normalize_alias;
use crate::brain_browse::{ActorGroup, BrowseRow};
use crate::memory_metadata::MemoryKind;

#[derive(Debug, Clone, SurrealValue)]
struct Row {
    id: String,
    text: String,
    category: String,
    pinned: bool,
    created_at: String,
    source_json: String,
}

impl Row {
    fn into_memory(self) -> Result<Memory> {
        Ok(Memory {
            source: serde_json::from_str(&self.source_json)
                .map_err(|e| anyhow!("memory {} source: {e}", self.id))?,
            id: self.id,
            text: self.text,
            category: self.category,
            pinned: self.pinned,
            created_at: self.created_at,
        })
    }
}

const ROW: &str = "meta::id(id) AS id, text, category, pinned, \
    time::format(created_at, '%+') AS created_at, source_json";

/// One memory by id, whatever its kind or lifecycle.
pub async fn get_memory(repo: &Repository, id: &str) -> Result<Option<Memory>> {
    let mut r = repo
        .db()
        .query(format!("SELECT {ROW} FROM ONLY $m"))
        .bind(("m", rid("memory", id)))
        .await?
        .check()?;
    let row: Option<Row> = r.take(0usize)?;
    row.map(Row::into_memory).transpose()
}

/// User deletion: tombstone, insight cleanup, the delete and its removal
/// index work in one publication, then the job retention sweep.
pub async fn delete_memory(repo: &Repository, id: &str) -> Result<bool> {
    let deleted = super::insight::delete_memory(repo.publisher(), id)
        .await
        .map_err(|e| anyhow!("delete memory {id}: {e}"))?;
    if deleted {
        // Drop the memory's vector after commit (removal work is also
        // queued). Embedding writes go through the retry runner.
        let key = rid("memory_embeddings", id);
        run_tx(repo.db(), RetryPolicy::default(), |tx| {
            let key = key.clone();
            async move {
                run(&tx, tx.query("DELETE $e RETURN NONE").bind(("e", key))).await?;
                Ok(())
            }
        })
        .await
        .map_err(|e| anyhow!("drop vector {id}: {e}"))?;
        super::jobs::cleanup_batch(repo.db(), chrono::Utc::now())
            .await
            .map_err(|e| anyhow!("job cleanup: {e}"))?;
    }
    Ok(deleted)
}

pub(crate) async fn claims(repo: &Repository) -> Result<Vec<Memory>> {
    let mut r = repo
        .db()
        .query(format!(
            "SELECT {ROW}, created_at AS at FROM memory WHERE lifecycle = 'active' \
             AND memory_kind = 'claim' ORDER BY pinned DESC, at DESC, id"
        ))
        .await?
        .check()?;
    let rows: Vec<Row> = r.take(0usize)?;
    rows.into_iter().map(Row::into_memory).collect()
}

#[derive(Debug, Clone, SurrealValue)]
struct Redirect {
    from: String,
    to: String,
}

#[derive(Debug, Clone, SurrealValue)]
struct Actor {
    id: String,
    display_name: String,
}

/// Live actors grouped by normalized display name, each group with every
/// redirected member. Name grouping never resolves identity.
pub async fn actor_groups(repo: &Repository) -> Result<Vec<ActorGroup>> {
    let mut r = repo
        .db()
        .query(
            "SELECT meta::id(id) AS from, to_actor AS to FROM brain_actor_redirect;
             SELECT meta::id(id) AS id, display_name FROM brain_actor
              WHERE record::exists(type::record('memory', meta::id(id)))
                AND type::record('memory', meta::id(id)).lifecycle = 'active'
                AND !record::exists(type::record('brain_actor_redirect', meta::id(id)))
              ORDER BY id;",
        )
        .await?
        .check()?;
    let redirects: Vec<Redirect> = r.take(0usize)?;
    let actors: Vec<Actor> = r.take(1usize)?;
    let mut into: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for d in redirects {
        into.entry(d.to).or_default().push(d.from);
    }
    let mut groups: BTreeMap<String, ActorGroup> = BTreeMap::new();
    for a in actors {
        let key = normalize_alias(&a.display_name);
        if key.is_empty() {
            continue;
        }
        let group = groups.entry(key.clone()).or_insert_with(|| ActorGroup {
            name_key: key,
            display_name: a.display_name.clone(),
            actor_ids: vec![],
        });
        let mut frontier = vec![a.id.clone()];
        group.actor_ids.push(a.id);
        while let Some(id) = frontier.pop() {
            for from in into.get(&id).into_iter().flatten() {
                if !group.actor_ids.contains(from) {
                    group.actor_ids.push(from.clone());
                    frontier.push(from.clone());
                }
            }
        }
    }
    let mut out: Vec<ActorGroup> = groups.into_values().collect();
    for g in &mut out {
        g.actor_ids.sort();
        g.actor_ids.dedup();
    }
    Ok(out)
}

/// Claims (pinned first, newest first) then Actor groups, filtered by a
/// case-insensitive substring of claim text or group name.
pub async fn brain_browse(
    repo: &Repository,
    kind: Option<MemoryKind>,
    query: &str,
) -> Result<Vec<BrowseRow>> {
    let mut rows = Vec::new();
    if kind.is_none() || kind == Some(MemoryKind::Claim) {
        rows.extend(claims(repo).await?.into_iter().map(BrowseRow::Claim));
    }
    if kind.is_none() || kind == Some(MemoryKind::Actor) {
        rows.extend(
            actor_groups(repo)
                .await?
                .into_iter()
                .map(BrowseRow::ActorGroup),
        );
    }
    let query = query.trim().to_lowercase();
    rows.retain(|row| match row {
        BrowseRow::Claim(m) => m.text.to_lowercase().contains(&query),
        BrowseRow::ActorGroup(g) => g.display_name.to_lowercase().contains(&query),
    });
    Ok(rows)
}
