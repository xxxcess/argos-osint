//! Synchronous `Store` Brain entry points on SurrealDB (brain track).
//!
//! When the process selected SurrealDB ([`on`]), the `Store` methods in
//! `brain_browse`, `brain_topology_query` and `brain_actors` dispatch here
//! instead of reading or writing the SQLite connection. Every function blocks
//! on [`runtime::blocking`], so callers must be plain threads, never async
//! code (the same rule as the TUI's `surreal_brain`). The reads reuse
//! `browse`, `browse_assoc` and `topology`; the writes reuse `actor_bind`,
//! `actor_dedup`, `entities`, `lifecycle` and `memories`.

use std::sync::{Arc, Mutex};

use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};

use super::jobs::{rid, run};
use super::memory::{set_lifecycle_in, write_then_index, MemoryWrite};
use super::runtime;
use super::tx::TxError;
use crate::brain::{Memory, MemorySource};
use crate::brain_actors::BindRequest;
use crate::brain_browse::{ActorGroup, Associations, BrowseRow};
use crate::brain_topology::{TopologyOptions, TopologySeed, TopologySnapshot};
use crate::memory_metadata::{MemoryKind, MemoryPayload};

/// Whether the `Store` Brain methods run on SurrealDB.
pub fn on() -> bool {
    crate::recon::backend::surreal_selected()
}

fn tx_err(e: TxError) -> anyhow::Error {
    match e {
        TxError::Abort(m) => anyhow!(m),
        e => e.into(),
    }
}

pub fn actor_groups() -> Result<Vec<ActorGroup>> {
    runtime::blocking(|repo| async move { super::browse::actor_groups(&repo).await })
}

pub fn brain_browse(kind: Option<MemoryKind>, query: &str) -> Result<Vec<BrowseRow>> {
    let query = query.to_string();
    runtime::blocking(|repo| async move { super::browse::brain_browse(&repo, kind, &query).await })
}

pub fn brain_browse_memories(kind: Option<MemoryKind>, query: &str) -> Result<Vec<Memory>> {
    let query = query.to_string();
    runtime::blocking(|repo| async move {
        super::browse_assoc::brain_browse_memories(&repo, kind, &query).await
    })
}

pub fn claim_associations(memory_id: &str) -> Result<Associations> {
    let id = memory_id.to_string();
    runtime::blocking(
        |repo| async move { super::browse_assoc::claim_associations(&repo, &id).await },
    )
}

pub fn actor_group_associations(group: &ActorGroup) -> Result<Associations> {
    let group = group.clone();
    runtime::blocking(|repo| async move {
        super::browse_assoc::actor_group_associations(&repo, &group).await
    })
}

pub fn actor_groups_scoped(cycle: Option<&str>) -> Result<Vec<ActorGroup>> {
    let cycle = cycle.map(str::to_string);
    runtime::blocking(|repo| async move {
        super::browse_assoc::actor_groups_scoped(&repo, cycle.as_deref()).await
    })
}

pub fn actor_group_associations_scoped(
    group: &ActorGroup,
    cycle: Option<&str>,
) -> Result<Associations> {
    let (group, cycle) = (group.clone(), cycle.map(str::to_string));
    runtime::blocking(|repo| async move {
        super::browse_assoc::actor_group_associations_scoped(&repo, &group, cycle.as_deref()).await
    })
}

pub fn actor_group_graph(group: &ActorGroup) -> Result<crate::recon::MemoryGraph> {
    let group = group.clone();
    runtime::blocking(
        |repo| async move { super::browse_assoc::actor_group_graph(&repo, &group).await },
    )
}

pub fn topology_snapshot(
    seed: &TopologySeed,
    options: &TopologyOptions,
) -> Result<TopologySnapshot> {
    let (seed, options) = (seed.clone(), options.clone());
    runtime::blocking(|repo| async move {
        super::topology::topology_snapshot(&repo, &seed, &options).await
    })
}

/// `Store::is_browse_claim`: an active claim or atomic claim.
pub fn is_browse_claim(id: &str) -> Result<bool> {
    let id = id.to_string();
    runtime::blocking(|repo| async move { super::brain_scope::is_claim(&repo, &id).await })
}

/// `Store::memory_payload`: the typed payload, `None` when the memory is
/// missing or untyped (empty `payload_json`).
pub fn memory_payload(memory_id: &str) -> Result<Option<MemoryPayload>> {
    let id = memory_id.to_string();
    runtime::blocking(|repo| async move {
        let mut r = repo
            .db()
            .query("SELECT VALUE payload_json ?? '' FROM ONLY $m")
            .bind(("m", rid("memory", &id)))
            .await?
            .check()?;
        let raw: Option<String> = r.take(0usize)?;
        match raw.filter(|s| !s.trim().is_empty()) {
            Some(s) => Ok(Some(serde_json::from_str(&s)?)),
            None => Ok(None),
        }
    })
}

/// `Store::mark_user_claim`: keep the typed claim payload's actors equal to
/// its refs.
pub fn mark_user_claim(memory_id: &str) -> Result<()> {
    let id = memory_id.to_string();
    runtime::blocking(|repo| async move {
        let write: MemoryWrite = Arc::new(move |tx| {
            let id = id.clone();
            Box::pin(async move {
                super::actor_bind::sync_claim_actor_payload_in(&tx, &id).await?;
                Ok(vec![])
            })
        });
        write_then_index(repo.publisher(), write)
            .await
            .map_err(tx_err)?;
        Ok(())
    })
}

/// `Store::add_user_claim`: save the memory, then mark it a user claim. The
/// vector is left to `argos memories reindex` (no embedder here).
pub fn add_user_claim(text: &str, category: &str, source: MemorySource) -> Result<Memory> {
    let (text, category) = (text.to_string(), category.to_string());
    let memory = runtime::blocking(|repo| async move {
        super::memories::add_memory(&repo, &text, &category, false, source, None).await
    })?;
    mark_user_claim(&memory.id)?;
    Ok(memory)
}

/// `Store::record_thread_actor`: bind the actor scoped to an opt-in thread
/// and record its thread mention.
pub fn record_thread_actor(thread: &str, name: &str, kind: &str) -> Result<()> {
    let (thread, name, kind) = (thread.to_string(), name.to_string(), kind.to_string());
    runtime::blocking(|repo| async move {
        let Some(t) = super::recon::get_thread(&repo, &thread)
            .await?
            .filter(|t| t.recall_insights)
        else {
            return Ok(());
        };
        if name.trim().is_empty() {
            return Ok(());
        }
        let write: MemoryWrite = Arc::new(move |tx| {
            let (thread, name, kind, tid) =
                (thread.clone(), name.clone(), kind.clone(), t.id.clone());
            Box::pin(async move {
                let now = chrono::Utc::now().to_rfc3339();
                let scope = format!("recon-thread:{tid}");
                let (bound, mut changed) = super::actor_bind::bind_in(
                    &tx,
                    &BindRequest {
                        name: &name,
                        subtype: crate::brain_actors::parse_subtype(&kind),
                        scope: &scope,
                        identifiers: &[],
                        external: None,
                        provenance: json!({ "thread": thread }),
                    },
                    &now,
                )
                .await?;
                changed.extend(
                    super::lifecycle::record_mention_in(
                        &tx,
                        &bound.actor_id,
                        "thread",
                        "recon",
                        &thread,
                        "thread-associations-v1",
                        "[]",
                        &now,
                    )
                    .await?,
                );
                Ok(changed)
            })
        });
        write_then_index(repo.publisher(), write)
            .await
            .map_err(tx_err)?;
        Ok(())
    })
}

/// `Store::bind_actor` through `actor_bind::bind_in`. `created`, `basis` and
/// `candidates` are not reported by the SurrealDB bind: `created` is false,
/// `basis` is `scoped` and `candidates` comes from `name_candidates_in`.
pub fn bind_actor(req: &BindRequest<'_>) -> Result<crate::brain_actors::Binding> {
    let name = req.name.to_string();
    let scope = req.scope.to_string();
    let identifiers = req.identifiers.to_vec();
    let external = req.external.map(|(a, b)| (a.to_string(), b.to_string()));
    let (subtype, provenance) = (req.subtype, req.provenance.clone());
    let out = Arc::new(Mutex::new(None));
    let slot = out.clone();
    runtime::blocking(|repo| async move {
        let write: MemoryWrite = Arc::new(move |tx| {
            let (name, scope, identifiers, external, provenance, slot) = (
                name.clone(),
                scope.clone(),
                identifiers.clone(),
                external.clone(),
                provenance.clone(),
                slot.clone(),
            );
            Box::pin(async move {
                let now = chrono::Utc::now().to_rfc3339();
                let req = BindRequest {
                    name: &name,
                    subtype,
                    scope: &scope,
                    identifiers: &identifiers,
                    external: external.as_ref().map(|(a, b)| (a.as_str(), b.as_str())),
                    provenance,
                };
                let (bound, changed) = super::actor_bind::bind_in(&tx, &req, &now).await?;
                let candidates = super::lifecycle::name_candidates_in(
                    &tx,
                    &name,
                    crate::brain_actors::subtype_str(subtype),
                    &bound.actor_id,
                )
                .await?;
                *slot.lock().expect("bind slot") = Some(crate::brain_actors::Binding {
                    actor_id: bound.actor_id,
                    created: false,
                    state: bound.state,
                    basis: "scoped",
                    candidates,
                });
                Ok(changed)
            })
        });
        write_then_index(repo.publisher(), write)
            .await
            .map_err(tx_err)?;
        Ok(())
    })?;
    let binding = out.lock().expect("bind slot").take();
    binding.ok_or_else(|| anyhow!("actor bind returned no binding"))
}

/// `Store::merge_actors` through `actor_dedup::merge_in`.
pub fn merge_actors(from: &str, to: &str, receipt: &Value) -> Result<()> {
    let (from, to, receipt) = (from.to_string(), to.to_string(), receipt.clone());
    runtime::blocking(|repo| async move {
        let write: MemoryWrite = Arc::new(move |tx| {
            let (from, to, receipt) = (from.clone(), to.clone(), receipt.clone());
            Box::pin(async move {
                let now = chrono::Utc::now().to_rfc3339();
                super::actor_dedup::merge_in(&tx, &from, &to, &receipt, &now).await
            })
        });
        write_then_index(repo.publisher(), write)
            .await
            .map_err(tx_err)?;
        Ok(())
    })
}

/// `Store::split_actor` (`brain_actors::split`): drop `from`'s redirect,
/// make it active, bump both revisions and record the explicit separation.
pub fn split_actor(from: &str) -> Result<()> {
    let from = from.to_string();
    let ok = runtime::blocking(|repo| async move {
        let mut r = repo
            .db()
            .query("SELECT VALUE to_actor FROM ONLY $r")
            .bind(("r", rid("brain_actor_redirect", &from)))
            .await?
            .check()?;
        let to: Option<String> = r.take(0usize)?;
        let Some(to) = to else {
            return Ok(Some(from));
        };
        let write: MemoryWrite = Arc::new(move |tx| {
            let (from, to) = (from.clone(), to.clone());
            Box::pin(async move {
                let now = chrono::Utc::now().to_rfc3339();
                let to = super::actor_bind::resolve_in(&tx, &to).await?;
                run(
                    &tx,
                    tx.query(
                        "DELETE $r RETURN NONE;
                         UPDATE $a, $b SET revision += 1, updated_at = $now RETURN NONE;",
                    )
                    .bind(("r", rid("brain_actor_redirect", &from)))
                    .bind(("a", rid("brain_actor", &from)))
                    .bind(("b", rid("brain_actor", &to)))
                    .bind(("now", now.clone())),
                )
                .await?;
                let mut changed =
                    set_lifecycle_in(&tx, std::slice::from_ref(&from), "active").await?;
                super::entities::mark_separate_in(
                    &tx,
                    &from,
                    &to,
                    &json!({ "basis": "explicit split" }).to_string(),
                    &now,
                )
                .await?;
                // Both revisions changed: re-index both, as SQLite bumps
                // `memories_changed`.
                changed.push(from);
                changed.push(to);
                changed.sort();
                changed.dedup();
                Ok(changed)
            })
        });
        write_then_index(repo.publisher(), write)
            .await
            .map_err(tx_err)?;
        Ok(None)
    })?;
    if let Some(from) = ok {
        bail!("{from} is not merged");
    }
    Ok(())
}

// ---- Memory CRUD, recall and vectors behind the frozen `Store` surface ----

/// `Store::list_memories`.
pub fn list_memories() -> Result<Vec<Memory>> {
    runtime::blocking(|repo| async move { super::memories::list_memories(&repo).await })
}

/// `Store::add_memory`: publish, then index with the local embedder.
pub fn add_memory(
    text: &str,
    category: &str,
    pinned: bool,
    source: MemorySource,
) -> Result<Memory> {
    let (text, category) = (text.to_string(), category.to_string());
    runtime::blocking(|repo| async move {
        let e = super::vectors::Embedder::local();
        super::memories::add_memory(&repo, &text, &category, pinned, source, e.as_ref()).await
    })
}

/// `Store::recall`: vector-first, lexical top-up (SurrealDB HNSW search
/// replaces the Lance index).
pub fn recall(query: &str, top_k: usize) -> Result<Vec<crate::brain::ScoredMemory>> {
    let query = query.to_string();
    let hits = runtime::blocking(|repo| async move {
        let e = super::vectors::Embedder::local();
        super::memories::recall(&repo, &query, top_k, e.as_ref()).await
    })?;
    Ok(hits
        .into_iter()
        .map(|h| crate::brain::ScoredMemory {
            memory: h.memory,
            score: h.score,
        })
        .collect())
}

/// `Store::find_similar_memory`: the best vector hit at or above `threshold`.
pub fn find_similar_memory(text: &str, threshold: f32) -> Option<(String, f32)> {
    let text = text.to_string();
    runtime::blocking(|repo| async move {
        let Some(e) = super::vectors::Embedder::local() else {
            return Ok(None);
        };
        let hits = super::memories::recall(&repo, &text, 1, Some(&e)).await?;
        Ok(hits
            .into_iter()
            .find(|h| h.score >= threshold)
            .map(|h| (h.memory.id, h.score)))
    })
    .unwrap_or_else(|err| {
        tracing::warn!("find_similar_memory: {err:#}");
        None
    })
}

/// `Store::get_memory`.
pub fn get_memory(id: &str) -> Result<Option<Memory>> {
    let id = id.to_string();
    runtime::blocking(|repo| async move { super::browse::get_memory(&repo, &id).await })
}

/// `Store::delete_memory`.
pub fn delete_memory(id: &str) -> Result<bool> {
    let id = id.to_string();
    runtime::blocking(|repo| async move { super::browse::delete_memory(&repo, &id).await })
}

/// `Store::update_memory`: rewrite text/category/pin and queue reindexing.
pub fn update_memory(id: &str, text: &str, category: &str, pinned: bool) -> Result<bool> {
    let text = text.trim().to_string();
    anyhow::ensure!(!text.is_empty(), "memory text is empty");
    let (id, category) = (
        id.to_string(),
        crate::brain::normalize_category(category).to_string(),
    );
    runtime::blocking(|repo| async move {
        if super::browse::get_memory(&repo, &id).await?.is_none() {
            return Ok(false);
        }
        let write: MemoryWrite = Arc::new(move |tx| {
            let (id, text, category) = (id.clone(), text.clone(), category.clone());
            Box::pin(async move {
                run(
                    &tx,
                    tx.query(
                        "UPDATE $m SET text = $text, category = $category, \
                         pinned = $pinned RETURN NONE",
                    )
                    .bind(("m", rid("memory", &id)))
                    .bind(("text", text))
                    .bind(("category", category))
                    .bind(("pinned", pinned)),
                )
                .await?;
                Ok(vec![id])
            })
        });
        write_then_index(repo.publisher(), write)
            .await
            .map_err(|e| anyhow!("update memory: {e}"))?;
        Ok(true)
    })
}

/// `Store::search_memories`: active memories whose text contains `query`.
pub fn search_memories(query: &str) -> Result<Vec<Memory>> {
    let needle = query.trim().to_lowercase();
    let all = list_memories()?;
    if needle.is_empty() {
        return Ok(all);
    }
    Ok(all
        .into_iter()
        .filter(|m| m.text.to_lowercase().contains(&needle))
        .collect())
}

/// `Store::memory_count`.
pub fn memory_count() -> Result<usize> {
    runtime::blocking(|repo| async move {
        let mut r = repo
            .db()
            .query("RETURN count(SELECT VALUE id FROM memory)")
            .await?
            .check()?;
        let n: Option<i64> = r.take(0usize)?;
        Ok(n.unwrap_or(0).max(0) as usize)
    })
}

/// `Store::graph_summary`.
pub fn graph_summary(memory_id: &str) -> Result<Option<crate::store::GraphSummary>> {
    let id = memory_id.to_string();
    runtime::blocking(|repo| async move { super::summaries::graph_summary(&repo, &id).await })
}

/// `Store::save_graph_summary`.
pub fn save_graph_summary(memory_id: &str, summary: &str, focus: &str) -> Result<bool> {
    let (id, summary, focus) = (
        memory_id.to_string(),
        summary.to_string(),
        focus.to_string(),
    );
    runtime::blocking(|repo| async move {
        super::summaries::save_graph_summary(&repo, &id, &summary, &focus).await
    })
}
