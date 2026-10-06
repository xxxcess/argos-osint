//! Related memories for the Brain claim/recon detail view.
//!
//! Every row is an existing canonical memory, identified by its memory id.
//! Explicit relationships come from stored rows (claim relations, shared
//! source articles or tool results, the same entity, the same investigation).
//! Similarity-only suggestions come from recall and are labelled as such;
//! similarity is never presented as evidence. Graph evidence nodes that have
//! no canonical memory never become rows.

use std::collections::{HashMap, HashSet};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::brain::Memory;
use crate::store::Store;

/// How a related memory is connected to the open one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RelationKind {
    /// A stored relationship (claim relation, shared source, entity, investigation).
    Explicit,
    /// Recall similarity only. Not evidence.
    Similar,
}

impl RelationKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Explicit => "linked",
            Self::Similar => "similar",
        }
    }
}

/// One navigable related memory.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RelatedMemory {
    pub memory_id: String,
    /// Readable claim / memory text (first non-empty line).
    pub title: String,
    /// Where the memory came from, e.g. "atlas · 2026-10-05".
    pub provenance: String,
    /// Short reason for the relationship.
    pub reason: String,
    pub kind: RelationKind,
    /// Explicit rank (lower is stronger) or similarity score.
    pub score: f32,
}

/// Row limits for [`Store::related_memories`].
#[derive(Clone, Copy, Debug)]
pub struct RelatedLimits {
    pub explicit: usize,
    pub similar: usize,
}

impl Default for RelatedLimits {
    fn default() -> Self {
        Self {
            explicit: 12,
            similar: 5,
        }
    }
}

/// Explicit relationship strength; lower ranks sort first.
const RANK_RELATION: u8 = 0;
const RANK_SOURCE: u8 = 1;
const RANK_ENTITY: u8 = 2;
const RANK_THREAD: u8 = 3;

struct Candidate {
    rank: u8,
    reason: String,
}

impl Store {
    /// Total saved memories (used to explain an active filter).
    pub fn memory_count(&self) -> Result<usize> {
        let n: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM memories", [], |r| r.get(0))?;
        Ok(n.max(0) as usize)
    }

    /// Unique, existing memories related to `memory_id`, excluding itself.
    /// Explicit relationships first (by strength, then newest), then
    /// similarity-only suggestions. Errors when the memory does not exist.
    pub fn related_memories(
        &self,
        memory_id: &str,
        limits: RelatedLimits,
    ) -> Result<Vec<RelatedMemory>> {
        let memory = self
            .get_memory(memory_id)?
            .ok_or_else(|| anyhow::anyhow!("memory {memory_id} no longer exists"))?;
        let mut best: HashMap<String, Candidate> = HashMap::new();
        let mut offer = |id: String, rank: u8, reason: String| {
            if id == memory_id {
                return;
            }
            match best.get(&id) {
                Some(existing) if existing.rank <= rank => {}
                _ => {
                    best.insert(id, Candidate { rank, reason });
                }
            }
        };

        // Stored claim relations (e.g. conflict or revision).
        let mut stmt = self.conn.prepare(
            "SELECT c2.memory_id, r.relation
             FROM insight_claims c1
             JOIN insight_relations r
               ON r.left_fingerprint = c1.fingerprint OR r.right_fingerprint = c1.fingerprint
             JOIN insight_claims c2
               ON c2.fingerprint = CASE WHEN r.left_fingerprint = c1.fingerprint
                                        THEN r.right_fingerprint ELSE r.left_fingerprint END
             WHERE c1.memory_id = ?1",
        )?;
        let rows = stmt
            .query_map([memory_id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (id, relation) in rows {
            offer(
                id,
                RANK_RELATION,
                format!("claim relation: {}", relation.replace('_', " ")),
            );
        }

        // The same source article (Atlas) or cited tool result (Recon).
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT c2.memory_id, IFNULL(s1.thread_id, ''), IFNULL(s1.run_id, ''), s1.call_id
             FROM insight_claims c1
             JOIN insight_sources s1 ON s1.fingerprint = c1.fingerprint AND s1.deleted_origin = 0
             JOIN insight_sources s2 ON s2.call_id = s1.call_id
                  AND IFNULL(s2.run_id, '') = IFNULL(s1.run_id, '') AND s2.deleted_origin = 0
             JOIN insight_claims c2 ON c2.fingerprint = s2.fingerprint
             WHERE c1.memory_id = ?1 AND s1.call_id <> ''",
        )?;
        let rows = stmt
            .query_map([memory_id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut article_titles: HashMap<(String, String), Option<String>> = HashMap::new();
        for (id, thread, run, call) in rows {
            let reason = if thread.is_empty() && !run.is_empty() {
                let title = article_titles
                    .entry((run.clone(), call.clone()))
                    .or_insert_with(|| {
                        self.atlas_article(&run, &call)
                            .ok()
                            .flatten()
                            .map(|article| article.title)
                    })
                    .clone();
                match title {
                    Some(title) if !title.trim().is_empty() => {
                        format!("same source article: {}", title.trim())
                    }
                    _ => "same source article".to_string(),
                }
            } else {
                "cites the same tool result".to_string()
            };
            offer(id, RANK_SOURCE, reason);
        }

        // The same entity.
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT c2.memory_id, c1.entity_id
             FROM insight_claims c1
             JOIN insight_claims c2 ON lower(c2.entity_id) = lower(c1.entity_id)
             WHERE c1.memory_id = ?1 AND c1.entity_id <> ''",
        )?;
        let rows = stmt
            .query_map([memory_id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (id, entity) in rows {
            offer(id, RANK_ENTITY, format!("same entity: {entity}"));
        }

        // The same investigation thread.
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT c2.memory_id
             FROM insight_claims c1
             JOIN insight_sources s1 ON s1.fingerprint = c1.fingerprint
             JOIN insight_sources s2 ON s2.thread_id = s1.thread_id
             JOIN insight_claims c2 ON c2.fingerprint = s2.fingerprint
             WHERE c1.memory_id = ?1 AND IFNULL(s1.thread_id, '') <> ''",
        )?;
        let rows = stmt
            .query_map([memory_id], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for id in rows {
            offer(id, RANK_THREAD, "same investigation".to_string());
        }

        // Resolve to existing canonical memories; drop anything missing.
        let mut explicit = Vec::new();
        for (id, candidate) in best {
            if let Some(target) = self.get_memory(&id)? {
                explicit.push((candidate, target));
            }
        }
        explicit.sort_by(|(a, ma), (b, mb)| {
            a.rank
                .cmp(&b.rank)
                .then_with(|| mb.created_at.cmp(&ma.created_at))
                .then_with(|| ma.id.cmp(&mb.id))
        });
        explicit.truncate(limits.explicit);
        let mut seen: HashSet<String> = explicit.iter().map(|(_, m)| m.id.clone()).collect();
        seen.insert(memory.id.clone());
        let mut out: Vec<RelatedMemory> = explicit
            .into_iter()
            .map(|(candidate, target)| {
                row(
                    &target,
                    candidate.reason,
                    RelationKind::Explicit,
                    candidate.rank as f32,
                )
            })
            .collect();

        if limits.similar > 0 {
            let reason = if self.vectors_enabled() {
                "semantic similarity · not evidence"
            } else {
                "similar wording · not evidence"
            };
            let hits = self.recall(&memory.text, limits.similar + seen.len() + 2)?;
            let mut similar = Vec::new();
            for hit in hits {
                if hit.score <= 0.0 || !seen.insert(hit.memory.id.clone()) {
                    continue;
                }
                similar.push(row(
                    &hit.memory,
                    reason.to_string(),
                    RelationKind::Similar,
                    hit.score,
                ));
                if similar.len() >= limits.similar {
                    break;
                }
            }
            out.extend(similar);
        }
        Ok(out)
    }
}

fn row(memory: &Memory, reason: String, kind: RelationKind, score: f32) -> RelatedMemory {
    let title = memory
        .text
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("")
        .trim()
        .to_string();
    let date = memory.created_at.get(..10).unwrap_or(&memory.created_at);
    let app = if memory.source.app.trim().is_empty() {
        "manual"
    } else {
        memory.source.app.trim()
    };
    RelatedMemory {
        memory_id: memory.id.clone(),
        title,
        provenance: if date.is_empty() {
            app.to_string()
        } else {
            format!("{app} · {date}")
        },
        reason,
        kind,
        score,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brain::MemorySource;

    fn memory(store: &Store, text: &str, app: &str) -> String {
        store
            .add_memory(
                text,
                "fact",
                false,
                MemorySource {
                    app: app.into(),
                    conversation_id: "c".into(),
                    message_id: None,
                    reference: None,
                },
            )
            .unwrap()
            .id
    }

    fn claim(store: &Store, fingerprint: &str, memory_id: &str, entity: &str) {
        store
            .conn
            .execute(
                "INSERT INTO insight_claims(fingerprint, memory_id, entity_id, predicate, object_value,
                    topic, classification, confidence, created_at, updated_at)
                 VALUES (?1, ?2, ?3, 'p', 'o', 't', 'news', 0.9, '2026-10-01', '2026-10-01')",
                rusqlite::params![fingerprint, memory_id, entity],
            )
            .unwrap();
    }

    fn source(store: &Store, fingerprint: &str, run: &str, call: &str) {
        store
            .conn
            .execute(
                "INSERT INTO insight_sources(fingerprint, thread_id, run_id, answer_id, call_id)
                 VALUES (?1, NULL, ?2, ?3, ?4)",
                rusqlite::params![fingerprint, run, format!("atlas-{fingerprint}"), call],
            )
            .unwrap();
    }

    #[test]
    fn related_rows_are_unique_existing_memories_ranked_explicit_before_similar() {
        let store = Store::memory().unwrap();
        let open = memory(
            &store,
            "Harbor tanker manifests list cargo bound for Odesa",
            "atlas",
        );
        let shared = memory(
            &store,
            "Port authority confirms tanker berth schedule",
            "atlas",
        );
        let entity = memory(&store, "Harbor authority opens a second terminal", "atlas");
        let revised = memory(&store, "Tanker manifests were revised on Friday", "atlas");
        let similar = memory(
            &store,
            "Tanker manifests list cargo for another port",
            "manual",
        );
        claim(&store, "f-open", &open, "Harbor Authority");
        claim(&store, "f-shared", &shared, "Port authority");
        claim(&store, "f-entity", &entity, "harbor authority");
        claim(&store, "f-revised", &revised, "Shipping");
        source(&store, "f-open", "run-1", "article-7");
        source(&store, "f-shared", "run-1", "article-7");
        store
            .conn
            .execute(
                "INSERT INTO insight_relations(left_fingerprint, right_fingerprint, relation)
                 VALUES ('f-revised', 'f-open', 'conflict_or_revision')",
                [],
            )
            .unwrap();
        // Evidence whose claim has no canonical memory row must not appear.
        let _ = store.conn.execute_batch("PRAGMA foreign_keys = OFF");
        claim(&store, "f-ghost", "missing-memory", "Ghost");
        source(&store, "f-ghost", "run-1", "article-7");

        let rows = store
            .related_memories(&open, RelatedLimits::default())
            .unwrap();
        let ids: Vec<&str> = rows.iter().map(|r| r.memory_id.as_str()).collect();
        assert!(!ids.contains(&open.as_str()), "self excluded: {ids:?}");
        assert!(
            !ids.contains(&"missing-memory"),
            "no fabricated rows: {ids:?}"
        );
        let unique: HashSet<&str> = ids.iter().copied().collect();
        assert_eq!(unique.len(), ids.len(), "deduplicated by memory id");
        assert_eq!(ids[0], revised, "claim relation ranks first");
        assert_eq!(ids[1], shared, "shared article next");
        assert_eq!(ids[2], entity, "same entity next");
        assert_eq!(rows[0].reason, "claim relation: conflict or revision");
        assert_eq!(rows[1].reason, "same source article");
        assert_eq!(rows[2].reason, "same entity: Harbor Authority");
        assert!(rows[..3].iter().all(|r| r.kind == RelationKind::Explicit));
        let similar_row = rows
            .iter()
            .find(|r| r.memory_id == similar)
            .expect("similar suggestion present");
        assert_eq!(similar_row.kind, RelationKind::Similar);
        assert!(similar_row.reason.contains("not evidence"));
        assert_eq!(similar_row.provenance.split(" · ").next(), Some("manual"));
        let first_similar = rows
            .iter()
            .position(|r| r.kind == RelationKind::Similar)
            .unwrap();
        assert!(rows[first_similar..]
            .iter()
            .all(|r| r.kind == RelationKind::Similar));

        // A deleted target disappears; a missing memory is an error, not empty.
        store.delete_memory(&shared).unwrap();
        let rows = store
            .related_memories(&open, RelatedLimits::default())
            .unwrap();
        assert!(rows.iter().all(|r| r.memory_id != shared));
        assert!(store
            .related_memories("nope", RelatedLimits::default())
            .is_err());
        assert_eq!(store.memory_count().unwrap(), 4);
    }
}
