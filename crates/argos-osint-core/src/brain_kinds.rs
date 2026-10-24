//! Typed Brain browsing: kind filters over the five core kinds, per-kind
//! counts, and typed detail lines for a memory's discriminated payload.
use anyhow::Result;

use crate::brain::Memory;
use crate::memory_metadata::{EndpointRef, MemoryKind};
use crate::store::Store;

/// Kind of a stored memory. Memories without typed metadata are legacy
/// evidence and browse as [`MemoryKind::Claim`].
pub fn memory_kind(_store: &Store, memory_id: &str) -> Result<MemoryKind> {
    crate::storage::store_bridge::memory_kind(memory_id)
}

/// True when `kind` passes `allowed` (empty = default evidence kinds, i.e.
/// every non-structural kind).
pub fn kind_allowed(allowed: &[MemoryKind], kind: MemoryKind) -> bool {
    if allowed.is_empty() {
        !matches!(
            kind,
            MemoryKind::Actor | MemoryKind::Link | MemoryKind::Related
        )
    } else {
        allowed.contains(&kind)
    }
}

/// Stable identity of a kind set for recall-cache keys ("" = default).
pub fn kinds_key(allowed: &[MemoryKind]) -> String {
    let mut k: Vec<&str> = allowed.iter().map(|k| k.as_str()).collect();
    k.sort_unstable();
    k.dedup();
    k.join(",")
}

impl Store {
    /// Brain list honoring an optional kind filter and Find text. `None`
    /// keeps the default evidence listing (structural kinds excluded).
    pub fn list_memories_by_kind(
        &self,
        kind: Option<MemoryKind>,
        query: &str,
    ) -> Result<Vec<Memory>> {
        let query = query.trim();
        let Some(kind) = kind else {
            return if query.is_empty() {
                self.list_memories()
            } else {
                self.search_memories(query)
            };
        };
        crate::storage::brain_store::brain_browse_memories(Some(kind), query)
    }

    /// Count per core kind (legacy untyped memories count as Claim).
    pub fn memory_kind_counts(&self) -> Result<Vec<(MemoryKind, usize)>> {
        let mut out = Vec::new();
        for kind in MemoryKind::CORE {
            out.push((kind, self.list_memories_by_kind(Some(kind), "")?.len()));
        }
        Ok(out)
    }
}

fn endpoint_label(store: &Store, e: &EndpointRef) -> String {
    match e {
        EndpointRef::Memory { memory_id, kind } => {
            let name = if *kind == MemoryKind::Actor {
                actor_display_name(store, memory_id)
            } else {
                store.get_memory(memory_id).ok().flatten().map(|m| m.text)
            };
            format!("{kind}: {}", name.unwrap_or_else(|| memory_id.clone()))
        }
        EndpointRef::Article {
            run_id, article_id, ..
        } => {
            let title = store
                .atlas_article(run_id, article_id)
                .ok()
                .flatten()
                .map(|a| a.title)
                .unwrap_or_else(|| article_id.clone());
            format!("article: {title} ({run_id})")
        }
        EndpointRef::Source {
            origin, source_id, ..
        } => format!("source: {origin}/{source_id}"),
    }
}

fn actor_display_name(_store: &Store, id: &str) -> Option<String> {
    crate::storage::store_bridge::actor_display_name(id)
        .ok()
        .flatten()
}

pub(crate) fn actor_resolve(_store: &Store, id: &str) -> Result<String> {
    crate::storage::store_bridge::actor_resolve(id)
}

pub(crate) fn actor_mentions(
    _store: &Store,
    id: &str,
) -> Result<Vec<(String, String, String, String)>> {
    crate::storage::store_bridge::actor_mentions(id)
}

fn snake<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

fn push(lines: &mut Vec<String>, label: &str, value: &str) {
    if !value.trim().is_empty() {
        lines.push(format!("{label}: {value}"));
    }
}

pub fn kind_title(kind: MemoryKind) -> &'static str {
    match kind {
        MemoryKind::Actor => "Actor",
        MemoryKind::Claim => "Claim",
        MemoryKind::Inference => "Inference",
        MemoryKind::Link => "Link",
        MemoryKind::Related => "Related",
        _ => "Other",
    }
}

#[cfg(test)]
#[path = "brain_kinds_tests.rs"]
mod tests;
