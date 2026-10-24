//! Read-only Claims/Actors display index. Name grouping never resolves identity.
use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;

use crate::brain::{Memory, MemorySource};
use crate::brain_actors as actors;
use crate::memory_metadata::{MemoryKind, MemoryPayload};
use crate::related_articles::canonical_url;
use crate::store::{AtlasArticleRow, Store};

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ArticleAssociation {
    pub article: AtlasArticleRow,
    pub occurrences: Vec<(String, String, String)>,
}

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Associations {
    pub claims: Vec<Memory>,
    pub actors: Vec<ActorGroup>,
    pub threads: Vec<(String, String)>,
    pub articles: Vec<ArticleAssociation>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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
pub(crate) struct BrowseContext {
    pub(crate) claims: Vec<Memory>,
    pub(crate) typed_actors: BTreeMap<String, BTreeSet<String>>,
    pub(crate) claim_origins: BTreeMap<String, Associations>,
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
        crate::storage::brain_store::add_user_claim(text, category, source)
    }
    pub fn mark_user_claim(&self, memory_id: &str) -> Result<()> {
        crate::storage::brain_store::mark_user_claim(memory_id)
    }
    /// Typed Claims and legacy insight-backed, non-inference claims only.
    pub fn is_browse_claim(&self, id: &str) -> Result<bool> {
        crate::storage::brain_store::is_browse_claim(id)
    }

    pub fn actor_groups(&self) -> Result<Vec<ActorGroup>> {
        crate::storage::brain_store::actor_groups()
    }

    pub fn actor_group(&self, key: &str) -> Result<Option<ActorGroup>> {
        let key = key.strip_prefix("actor-group:").unwrap_or(key);
        Ok(self.actor_groups()?.into_iter().find(|g| g.name_key == key))
    }

    pub fn brain_browse(&self, kind: Option<MemoryKind>, query: &str) -> Result<Vec<BrowseRow>> {
        crate::storage::brain_store::brain_browse(kind, query)
    }

    /// Compatibility snapshot for the TUI list, with stable group keys rather
    /// than an arbitrary member memory. Mutations must reject aggregate keys.
    pub fn brain_browse_memories(
        &self,
        kind: Option<MemoryKind>,
        query: &str,
    ) -> Result<Vec<Memory>> {
        crate::storage::brain_store::brain_browse_memories(kind, query)
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
        crate::storage::brain_store::claim_associations(memory_id)
    }

    pub fn actor_group_associations(&self, group: &ActorGroup) -> Result<Associations> {
        crate::storage::brain_store::actor_group_associations(group)
    }

    pub(crate) fn browse_context(&self) -> Result<BrowseContext> {
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
        })
    }

    /// Read actor groups scoped to an optional cycle run ID.
    pub fn actor_groups_scoped(&self, cycle: Option<&str>) -> Result<Vec<ActorGroup>> {
        crate::storage::brain_store::actor_groups_scoped(cycle)
    }

    /// Read associations for an actor group scoped to an optional cycle run ID.
    pub fn actor_group_associations_scoped(
        &self,
        group: &ActorGroup,
        cycle: Option<&str>,
    ) -> Result<Associations> {
        crate::storage::brain_store::actor_group_associations_scoped(group, cycle)
    }

    pub(crate) fn actor_group_associations_in(
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
            // ActorGroup already contains every redirected member. Expanding
            // each family again makes large merged groups quadratic.
            for (kind, scope, origin, revision) in actors::direct_mentions(&self.conn, id)? {
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
        // A direct mention is sufficient. Related's two-entity overlap rule
        // applies to article suggestions, never to an actor's own sources.
        out.articles = self.association_articles(origins)?;
        Ok(out)
    }

    /// Opt-in detection receipts survive switching Recall off; deletion removes
    /// only that origin. Replacement callers retire their previous detections.
    pub fn record_thread_actor(&self, thread: &str, name: &str, kind: &str) -> Result<()> {
        crate::storage::brain_store::record_thread_actor(thread, name, kind)
    }

    pub fn actor_group_graph(&self, group: &ActorGroup) -> Result<crate::recon::MemoryGraph> {
        crate::storage::brain_store::actor_group_graph(group)
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
            let nid = m.id.clone();
            graph
                .nodes
                .push(node(nid.clone(), GraphNodeKind::Finding, m.text));
            graph.edges.push(GraphEdge {
                from: root.clone(),
                to: nid.clone(),
                kind: GraphEdgeKind::Link,
                directive: Some("concerns".into()),
                edge_id: Some(format!("edge:{root}:{nid}")),
            });
        }
        for (id, title) in a.threads {
            let nid = format!("thread:{id}");
            graph
                .nodes
                .push(node(nid.clone(), GraphNodeKind::Source, title));
            graph.edges.push(GraphEdge {
                from: root.clone(),
                to: nid.clone(),
                kind: GraphEdgeKind::Link,
                directive: Some("source".into()),
                edge_id: Some(format!("edge:{root}:{nid}")),
            });
        }
        for a in a.articles {
            let nid = format!("article:{}/{}", a.article.run_id, a.article.id);
            let mut n = node(nid.clone(), GraphNodeKind::Evidence, a.article.title);
            n.article_id = a.article.id;
            n.run_id = a.article.run_id;
            n.published_at = a.article.published_at;
            graph.nodes.push(n);
            graph.edges.push(GraphEdge {
                from: root.clone(),
                to: nid.clone(),
                kind: GraphEdgeKind::Link,
                directive: Some("mentioned_in".into()),
                edge_id: Some(format!("edge:{root}:{nid}")),
            });
        }
        graph
    }
}
