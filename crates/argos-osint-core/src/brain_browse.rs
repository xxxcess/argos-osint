//! Read-only Claims/Actors display index. Name grouping never resolves identity.
use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;

use crate::brain::{Memory, MemorySource};
use crate::brain_actors as actors;
use crate::memory_metadata::{MemoryKind, MemoryPayload};
use crate::related_articles::canonical_url;
use crate::store::{AtlasArticleRow, Store};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActorGroup {
    pub name_key: String,
    pub display_name: String,
    pub actor_ids: Vec<String>,
}

#[cfg(test)]
#[path = "brain_browse_tests.rs"]
mod tests;

#[derive(Clone, Debug)]
pub enum BrowseRow {
    Claim(Memory),
    ActorGroup(ActorGroup),
}

#[derive(Clone, Debug)]
pub struct ArticleAssociation {
    pub article: AtlasArticleRow,
    pub occurrences: Vec<(String, String, String)>,
}

#[derive(Clone, Debug, Default)]
pub struct Associations {
    pub claims: Vec<Memory>,
    pub actors: Vec<ActorGroup>,
    pub threads: Vec<(String, String)>,
    pub articles: Vec<ArticleAssociation>,
}

struct BrowseContext {
    claims: Vec<Memory>,
    typed_actors: BTreeMap<String, BTreeSet<String>>,
    claim_origins: BTreeMap<String, Associations>,
    ingest: BTreeMap<(String, String), BTreeSet<String>>,
}

impl ActorGroup {
    pub fn key(&self) -> String {
        format!("actor-group:{}", self.name_key)
    }
}

impl Store {
    pub fn add_user_claim(
        &self,
        text: &str,
        category: &str,
        source: MemorySource,
    ) -> Result<Memory> {
        self.write_then_index(|| {
            let memory = self.add_memory(text, category, false, source)?;
            self.mark_user_claim(&memory.id)?;
            let id = memory.id.clone();
            Ok((memory, vec![id]))
        })
    }
    pub fn mark_user_claim(&self, memory_id: &str) -> Result<()> {
        crate::brain_actors::sync_claim_actor_payload(&self.conn, memory_id)
    }
    /// Typed Claims and legacy insight-backed, non-inference claims only.
    pub fn is_browse_claim(&self, id: &str) -> Result<bool> {
        Ok(self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM memories m LEFT JOIN memory_metadata t ON t.memory_id=m.id WHERE m.id=?1 AND (t.lifecycle IS NULL OR t.lifecycle='active') AND (t.memory_kind IN ('claim','atomic_claim') OR (t.memory_kind IS NULL AND EXISTS(SELECT 1 FROM insight_claims c WHERE c.memory_id=m.id AND lower(c.classification)<>'inference'))))",
            [id], |r| r.get(0),
        )?)
    }

    pub fn actor_groups(&self) -> Result<Vec<ActorGroup>> {
        let mut groups: BTreeMap<String, ActorGroup> = BTreeMap::new();
        let mut stmt = self.conn.prepare("SELECT a.actor_id FROM brain_actors a JOIN memories m ON m.id=a.actor_id JOIN memory_metadata t ON t.memory_id=m.id WHERE t.lifecycle IN ('active','redirected') ORDER BY a.actor_id")?;
        let ids = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for id in ids {
            let Some(actor) = actors::actor(&self.conn, &id)? else {
                continue;
            };
            let key = actors::normalize_alias(&actor.display_name);
            if key.is_empty() {
                continue;
            }
            let group = groups.entry(key.clone()).or_insert_with(|| ActorGroup {
                name_key: key,
                display_name: actor.display_name,
                actor_ids: vec![],
            });
            group
                .actor_ids
                .extend(actors::equivalent_ids(&self.conn, &id)?);
        }
        for group in groups.values_mut() {
            group.actor_ids.sort();
            group.actor_ids.dedup();
        }
        Ok(groups.into_values().collect())
    }

    pub fn actor_group(&self, key: &str) -> Result<Option<ActorGroup>> {
        let key = key.strip_prefix("actor-group:").unwrap_or(key);
        Ok(self.actor_groups()?.into_iter().find(|g| g.name_key == key))
    }

    pub fn brain_browse(&self, kind: Option<MemoryKind>, query: &str) -> Result<Vec<BrowseRow>> {
        let mut rows = Vec::new();
        if kind.is_none() || kind == Some(MemoryKind::Claim) {
            let mut stmt = self.conn.prepare("SELECT m.id,m.text,m.category,m.pinned,m.created_at,m.source_json FROM memories m LEFT JOIN memory_metadata t ON t.memory_id=m.id WHERE (t.lifecycle IS NULL OR t.lifecycle='active') AND (t.memory_kind IN ('claim','atomic_claim') OR (t.memory_kind IS NULL AND EXISTS(SELECT 1 FROM insight_claims c WHERE c.memory_id=m.id AND lower(c.classification)<>'inference'))) ORDER BY m.pinned DESC,m.created_at DESC,m.id")?;
            rows.extend(
                stmt.query_map([], crate::store::memory_row)?
                    .collect::<rusqlite::Result<Vec<_>>>()?
                    .into_iter()
                    .map(BrowseRow::Claim),
            );
        }
        if kind.is_none() || kind == Some(MemoryKind::Actor) {
            rows.extend(self.actor_groups()?.into_iter().map(BrowseRow::ActorGroup));
        }
        let query = query.trim().to_lowercase();
        rows.retain(|row| match row {
            BrowseRow::Claim(m) => m.text.to_lowercase().contains(&query),
            BrowseRow::ActorGroup(g) => g.display_name.to_lowercase().contains(&query),
        });
        Ok(rows)
    }

    /// Compatibility snapshot for the TUI list, with stable group keys rather
    /// than an arbitrary member memory. Mutations must reject aggregate keys.
    pub fn brain_browse_memories(
        &self,
        kind: Option<MemoryKind>,
        query: &str,
    ) -> Result<Vec<Memory>> {
        let rows = self.brain_browse(kind, query)?;
        let context = if rows.iter().any(|r| matches!(r, BrowseRow::ActorGroup(_))) {
            Some(self.browse_context()?)
        } else {
            None
        };
        rows.into_iter()
            .map(|row| match row {
                BrowseRow::Claim(m) => Ok(m),
                BrowseRow::ActorGroup(g) => {
                    let a = self.actor_group_associations_in(
                        &g,
                        context.as_ref().expect("actor context"),
                    )?;
                    Ok(Memory {
                        id: g.key(),
                        text: g.display_name,
                        category: format!(
                            "{} claims · {} threads · {} articles",
                            a.claims.len(),
                            a.threads.len(),
                            a.articles.len()
                        ),
                        pinned: false,
                        created_at: String::new(),
                        source: MemorySource {
                            app: "brain-actor-group".into(),
                            conversation_id: g.name_key,
                            message_id: None,
                            reference: None,
                        },
                    })
                }
            })
            .collect()
    }

    fn association_articles(
        &self,
        origins: BTreeSet<(String, String, String)>,
    ) -> Result<Vec<ArticleAssociation>> {
        let mut articles: BTreeMap<String, ArticleAssociation> = BTreeMap::new();
        for (run, id, revision) in origins {
            let Some(article) = self.atlas_article(&run, &id)? else {
                continue;
            };
            let current_revision = crate::atlas_metadata::input_hash(&article);
            if !revision.is_empty() && revision != current_revision {
                continue;
            }
            let revision = current_revision;
            let key = if article.url.is_empty() {
                format!("{run}/{id}")
            } else {
                canonical_url(&article.url)
            };
            let a = articles.entry(key).or_insert_with(|| ArticleAssociation {
                article,
                occurrences: vec![],
            });
            a.occurrences.push((run, id, revision));
        }
        Ok(articles
            .into_values()
            .map(|mut article| {
                article.occurrences.sort();
                article.occurrences.dedup();
                article
            })
            .collect())
    }

    pub fn claim_associations(&self, memory_id: &str) -> Result<Associations> {
        let mut out = Associations::default();
        let mut actor_ids = BTreeSet::new();
        let mut stmt = self
            .conn
            .prepare("SELECT actor_id FROM brain_memory_actor_refs WHERE memory_id=?1")?;
        for id in stmt.query_map([memory_id], |r| r.get::<_, String>(0))? {
            actor_ids.insert(actors::resolve(&self.conn, &id?)?);
        }
        if let Some(MemoryPayload::Claim(p)) = self.memory_payload(memory_id)? {
            for a in p.actors {
                actor_ids.insert(actors::resolve(&self.conn, &a.actor_id)?);
            }
        }
        out.actors = self
            .actor_groups()?
            .into_iter()
            .filter(|g| g.actor_ids.iter().any(|id| actor_ids.contains(id)))
            .collect();
        let mut stmt = self.conn.prepare("SELECT DISTINCT t.id,t.title FROM insight_sources s JOIN insight_claims c ON c.fingerprint=s.fingerprint JOIN recon_threads t ON t.id=s.thread_id WHERE c.memory_id=?1 AND s.deleted_origin=0 AND t.deleted=0 ORDER BY t.title,t.id")?;
        out.threads = stmt
            .query_map([memory_id], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let origins = self
            .article_origins_for_memory(memory_id)?
            .into_iter()
            .map(|o| (o.run_id, o.article_id, String::new()))
            .collect();
        out.articles = self.association_articles(origins)?;
        Ok(out)
    }

    pub fn actor_group_associations(&self, group: &ActorGroup) -> Result<Associations> {
        self.actor_group_associations_in(group, &self.browse_context()?)
    }

    fn browse_context(&self) -> Result<BrowseContext> {
        let claims: Vec<_> = self
            .brain_browse(Some(MemoryKind::Claim), "")?
            .into_iter()
            .filter_map(|r| match r {
                BrowseRow::Claim(m) => Some(m),
                _ => None,
            })
            .collect();
        let mut typed_actors = BTreeMap::new();
        let mut claim_origins = BTreeMap::new();
        for m in &claims {
            if let Some(MemoryPayload::Claim(p)) = self.memory_payload(&m.id)? {
                let mut ids = BTreeSet::new();
                for a in p.actors {
                    ids.insert(actors::resolve(&self.conn, &a.actor_id)?);
                }
                typed_actors.insert(m.id.clone(), ids);
            }
            let mut stmt = self.conn.prepare("SELECT DISTINCT t.id,t.title FROM insight_sources s JOIN insight_claims c ON c.fingerprint=s.fingerprint JOIN recon_threads t ON t.id=s.thread_id WHERE c.memory_id=?1 AND s.deleted_origin=0 AND t.deleted=0 ORDER BY t.title,t.id")?;
            let threads = stmt
                .query_map([&m.id], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?;
            let origins = self
                .article_origins_for_memory(&m.id)?
                .into_iter()
                .map(|o| (o.run_id, o.article_id, String::new()))
                .collect();
            claim_origins.insert(
                m.id.clone(),
                Associations {
                    threads,
                    articles: self.association_articles(origins)?,
                    ..Default::default()
                },
            );
        }
        Ok(BrowseContext {
            claims,
            typed_actors,
            claim_origins,
            ingest: self.ingest_actor_associations()?,
        })
    }

    fn actor_group_associations_in(
        &self,
        group: &ActorGroup,
        context: &BrowseContext,
    ) -> Result<Associations> {
        let mut out = Associations::default();
        let mut claims = BTreeSet::new();
        let mut threads = BTreeMap::new();
        let mut origins = BTreeSet::new();
        for id in &group.actor_ids {
            let mut links=self.conn.prepare("SELECT e.origin,e.ref_id,e.revision FROM brain_memory_endpoints a JOIN brain_memory_endpoints e ON e.memory_id=a.memory_id JOIN memory_metadata m ON m.memory_id=e.memory_id WHERE a.endpoint_kind='memory' AND a.ref_id=?1 AND e.endpoint_kind='article' AND m.memory_kind='link' AND m.lifecycle='active'")?;
            for origin in links.query_map([id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))? {
                origins.insert(origin?);
            }
            let mut stmt = self
                .conn
                .prepare("SELECT memory_id FROM brain_memory_actor_refs WHERE actor_id=?1")?;
            for memory in stmt.query_map([id], |r| r.get::<_, String>(0))? {
                let memory = memory?;
                if context.claim_origins.contains_key(&memory) {
                    claims.insert(memory);
                }
            }
            for (kind, scope, origin, revision) in actors::mentions(&self.conn, id)? {
                if kind == "article" {
                    origins.insert((scope, origin, revision));
                } else if kind == "thread" {
                    if let Some(t) = self.get_thread(&origin)? {
                        threads.insert(t.id, t.title);
                    }
                }
            }
        }
        // Typed references are authoritative even before a legacy index repair.
        for (id, actors) in &context.typed_actors {
            if group.actor_ids.iter().any(|id| actors.contains(id)) {
                claims.insert(id.clone());
            }
        }
        for id in claims {
            let a = &context.claim_origins[&id];
            for (id, title) in &a.threads {
                threads.insert(id.clone(), title.clone());
            }
            for a in &a.articles {
                origins.extend(a.occurrences.clone());
            }
            if let Some(m) = context.claims.iter().find(|m| m.id == id) {
                out.claims.push(m.clone());
            }
        }
        out.threads = threads.into_iter().collect();
        // Qualify exact occurrences before URL deduplication; a stale duplicate
        // must neither hide a live occurrence nor inherit its association.
        let mut eligible = BTreeSet::new();
        for (run, id, revision) in origins {
            let Some(article) = self.atlas_article(&run, &id)? else {
                continue;
            };
            if !revision.is_empty() && revision != crate::atlas_metadata::input_hash(&article) {
                continue;
            }
            if context
                .ingest
                .get(&(run.clone(), id.clone()))
                .is_some_and(|names| names.contains(&group.name_key))
            {
                eligible.insert((run, id, revision));
            }
        }
        out.articles = self.association_articles(eligible)?;
        Ok(out)
    }

    /// Opt-in detection receipts survive switching Recall off; deletion removes
    /// only that origin. Replacement callers retire their previous detections.
    pub fn record_thread_actor(&self, thread: &str, name: &str, kind: &str) -> Result<()> {
        let Some(t) = self.get_thread(thread)?.filter(|t| t.recall_insights) else {
            return Ok(());
        };
        if name.trim().is_empty() {
            return Ok(());
        }
        let now = chrono::Utc::now().to_rfc3339();
        let scope = format!("recon-thread:{}", t.id);
        let (binding, _) = actors::bind(
            &self.conn,
            &actors::BindRequest {
                name,
                subtype: actors::parse_subtype(kind),
                scope: &scope,
                identifiers: &[],
                external: None,
                provenance: serde_json::json!({"thread":thread}),
            },
            &now,
        )?;
        actors::record_mention(
            &self.conn,
            &binding.actor_id,
            "thread",
            "recon",
            thread,
            "thread-associations-v1",
            &serde_json::json!([]),
            &now,
        )?;
        Ok(())
    }

    pub fn actor_group_graph(&self, group: &ActorGroup) -> Result<crate::recon::MemoryGraph> {
        Ok(Self::actor_group_graph_from(
            group,
            self.actor_group_associations(group)?,
        ))
    }

    /// Build the graph from the same immutable associations used by Related.
    pub fn actor_group_graph_from(
        group: &ActorGroup,
        a: Associations,
    ) -> crate::recon::MemoryGraph {
        use crate::recon::{GraphEdge, GraphEdgeKind, GraphNode, GraphNodeKind, MemoryGraph};
        let root = group.key();
        let node = |id: String, kind, label: String| GraphNode {
            id,
            kind,
            detail: label.clone(),
            label,
            tags: vec![],
            article_id: String::new(),
            run_id: String::new(),
            published_at: String::new(),
        };
        let mut graph = MemoryGraph {
            nodes: vec![node(
                root.clone(),
                GraphNodeKind::Entity,
                group.display_name.clone(),
            )],
            edges: vec![],
        };
        for m in a.claims {
            graph.nodes.push(node(m.id, GraphNodeKind::Finding, m.text));
        }
        for (id, title) in a.threads {
            graph
                .nodes
                .push(node(format!("thread:{id}"), GraphNodeKind::Source, title));
        }
        for a in a.articles {
            let mut n = node(
                format!("article:{}/{}", a.article.run_id, a.article.id),
                GraphNodeKind::Evidence,
                a.article.title,
            );
            n.article_id = a.article.id;
            n.run_id = a.article.run_id;
            n.published_at = a.article.published_at;
            graph.nodes.push(n);
        }
        for n in &graph.nodes[1..] {
            graph.edges.push(GraphEdge {
                from: root.clone(),
                to: n.id.clone(),
                kind: GraphEdgeKind::Link,
                directive: Some("associated with".into()),
            });
        }
        graph
    }
}
