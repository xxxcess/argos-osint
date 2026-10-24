//! Typed Brain browsing: kind filters over the five core kinds, per-kind
//! counts, and typed detail lines for a memory's discriminated payload.
use anyhow::Result;
use rusqlite::params;

use crate::brain::Memory;
use crate::memory_metadata::{EndpointRef, MemoryKind, MemoryPayload};
use crate::store::Store;

/// Kind of a stored memory. Memories without typed metadata are legacy
/// evidence and browse as [`MemoryKind::Claim`].
pub fn memory_kind(store: &Store, memory_id: &str) -> Result<MemoryKind> {
    #[cfg(feature = "surreal")]
    if crate::storage::store_bridge::on() {
        return crate::storage::store_bridge::memory_kind(memory_id);
    }
    use rusqlite::OptionalExtension;
    // A missing metadata row is a legacy untyped memory; a read error is not.
    let raw: Option<String> = store
        .conn
        .query_row(
            "SELECT memory_kind FROM memory_metadata WHERE memory_id=?1",
            [memory_id],
            |r| r.get(0),
        )
        .optional()?;
    Ok(raw
        .as_deref()
        .and_then(MemoryKind::parse)
        .unwrap_or(MemoryKind::Claim))
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
        #[cfg(feature = "surreal")]
        if crate::storage::store_bridge::on() {
            return crate::storage::brain_store::brain_browse_memories(Some(kind), query);
        }
        let spellings = kind.stored_spellings();
        // Legacy untyped memories browse as Claim.
        let legacy = kind == MemoryKind::Claim;
        let needle = format!(
            "%{}%",
            query
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        );
        let sql = format!(
            "SELECT m.id,m.text,m.category,m.pinned,m.created_at,m.source_json FROM memories m
             LEFT JOIN memory_metadata t ON t.memory_id=m.id
             WHERE (t.memory_kind IN ({}) {})
               AND (t.memory_id IS NULL OR t.lifecycle='active')
               AND (?1='' OR m.text LIKE ?2 ESCAPE '\\' OR m.category LIKE ?2 ESCAPE '\\' OR IFNULL(t.payload_json,'') LIKE ?2 ESCAPE '\\')
             ORDER BY m.pinned DESC, m.created_at DESC",
            spellings
                .iter()
                .map(|s| format!("'{s}'"))
                .collect::<Vec<_>>()
                .join(","),
            if legacy { "OR t.memory_id IS NULL" } else { "" }
        );
        // Actors merged into another list once, as their canonical actor.
        let sql = if kind == MemoryKind::Actor {
            sql.replace(
                "ORDER BY m.pinned",
                "AND NOT EXISTS (SELECT 1 FROM brain_actor_redirects r WHERE r.from_actor=m.id) ORDER BY m.pinned",
            )
        } else {
            sql
        };
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![query, needle], crate::store::memory_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
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

fn actor_display_name(store: &Store, id: &str) -> Option<String> {
    #[cfg(feature = "surreal")]
    if crate::storage::store_bridge::on() {
        return crate::storage::store_bridge::actor_display_name(id)
            .ok()
            .flatten();
    }
    crate::brain_actors::actor(&store.conn, id)
        .ok()
        .flatten()
        .map(|a| a.display_name)
}

pub(crate) fn actor_resolve(store: &Store, id: &str) -> Result<String> {
    #[cfg(feature = "surreal")]
    if crate::storage::store_bridge::on() {
        return crate::storage::store_bridge::actor_resolve(id);
    }
    crate::brain_actors::resolve(&store.conn, id)
}

pub(crate) fn actor_mentions(
    store: &Store,
    id: &str,
) -> Result<Vec<(String, String, String, String)>> {
    #[cfg(feature = "surreal")]
    if crate::storage::store_bridge::on() {
        return crate::storage::store_bridge::actor_mentions(id);
    }
    crate::brain_actors::mentions(&store.conn, id)
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

/// Typed detail lines for a memory (kind header first).
pub fn typed_detail(store: &Store, memory_id: &str) -> Result<Vec<String>> {
    let kind = memory_kind(store, memory_id)?;
    let mut lines = vec![format!("Kind: {}", kind_title(kind))];
    match store.memory_payload(memory_id)? {
        Some(MemoryPayload::Actor(p)) => {
            let canonical = actor_resolve(store, memory_id)?;
            push(&mut lines, "Subtype", &snake(&p.subtype));
            push(&mut lines, "Identity", &snake(&p.identity_state));
            push(&mut lines, "Aliases", &p.aliases.join(", "));
            push(&mut lines, "Identifiers", &p.identifiers.join(", "));
            if canonical != memory_id {
                push(&mut lines, "Merged into", &canonical);
            }
            let mentions = actor_mentions(store, memory_id)?;
            let articles = mentions.iter().filter(|m| m.0 == "article").count();
            lines.push(format!(
                "Mentions: {} ({articles} article occurrence{})",
                mentions.len(),
                if articles == 1 { "" } else { "s" }
            ));
        }
        Some(MemoryPayload::Claim(p)) => {
            push(&mut lines, "Predicate", &p.predicate);
            push(&mut lines, "Object", &p.object);
            push(&mut lines, "Modality", &p.modality);
            push(&mut lines, "Assessment", &p.assessment);
            lines.push(format!(
                "Actors: {} · passages {}",
                p.actors.len(),
                p.passages.len()
            ));
        }
        Some(MemoryPayload::Inference(p)) => {
            push(&mut lines, "Reasoning", &p.reasoning);
            push(&mut lines, "Limitations", &p.limitations);
            push(&mut lines, "Support", &p.support_state);
            lines.push(format!("Premise claims: {}", p.premise_claims.len()));
        }
        Some(MemoryPayload::Link(p)) => {
            push(&mut lines, "Relation", &p.relation);
            lines.push(format!("From: {}", endpoint_label(store, &p.source)));
            lines.push(format!("To: {}", endpoint_label(store, &p.target)));
            push(&mut lines, "Rationale", &p.rationale);
            lines.push(format!("Receipts: {} · observed (Link)", p.receipts.len()));
        }
        Some(MemoryPayload::Related(p)) => {
            lines.push(format!("From: {}", endpoint_label(store, &p.source)));
            lines.push(format!("To: {}", endpoint_label(store, &p.target)));
            push(&mut lines, "Basis", &p.basis);
            push(&mut lines, "Interpretation", &p.interpretation);
            push(&mut lines, "Verification gap", &p.verification_gap);
            lines.push(if p.shared_actors.is_empty() {
                "Status: tentative · not evidence".into()
            } else {
                format!("Status: shared actor ({})", p.shared_actors.len())
            });
        }
        None => lines.push("Payload: legacy (untyped)".into()),
    }
    Ok(lines)
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
