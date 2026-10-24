//! Directive-linked graph for one Brain memory.
//!
//! Nodes are the investigation, its directives, the claim's entity and topic, the
//! finding, cited evidence, and the source that evidence came from. Edges use the
//! relationship verbs the Brain graph draws.

use rusqlite::OptionalExtension;
use std::collections::{HashMap, HashSet};

use crate::store::Store;

use super::{Directive, InsightSource, InsightView, Plan, PlanCall};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphNodeKind {
    Investigation,
    Directive,
    Entity,
    Topic,
    Finding,
    Evidence,
    Source,
    /// One node per canonical Brain actor (merged actors collapse into it).
    Actor,
}

impl GraphNodeKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Investigation => "Investigation",
            Self::Directive => "Directive",
            Self::Entity => "Entity",
            Self::Topic => "Topic",
            Self::Finding => "Finding",
            Self::Evidence => "Evidence",
            Self::Source => "Source",
            Self::Actor => "Actor",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphEdgeKind {
    Investigates,
    Answers,
    Concerns,
    Supports,
    Reports,
    DerivedFrom,
    DiscoveredIn,
    /// Observed Brain Link (relation in `GraphEdge::directive`).
    Link,
    /// Tentative Brain Related lead; never evidence.
    Related,
}

impl GraphEdgeKind {
    pub fn verb(self) -> &'static str {
        match self {
            Self::Investigates => "investigates",
            Self::Answers => "answers",
            Self::Concerns => "concerns",
            Self::Supports => "supports",
            Self::Reports => "reports",
            Self::DerivedFrom => "derived_from",
            Self::DiscoveredIn => "discovered_in",
            Self::Link => "link",
            Self::Related => "related",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphNode {
    pub id: String,
    pub kind: GraphNodeKind,
    pub label: String,
    pub detail: String,
    /// Directive ids this evidence serves. Empty for every other kind.
    pub tags: Vec<String>,
    /// Atlas article this evidence node opens. Empty for every other kind.
    pub article_id: String,
    pub run_id: String,
    pub published_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphEdge {
    pub from: String,
    pub to: String,
    pub kind: GraphEdgeKind,
    /// Directive id; for [`GraphEdgeKind::Link`] the Link relation.
    pub directive: Option<String>,
    pub edge_id: Option<String>,
}

impl GraphEdge {
    pub fn new(from: String, to: String, kind: GraphEdgeKind, directive: Option<String>) -> Self {
        Self {
            from,
            to,
            kind,
            directive,
            edge_id: None,
        }
    }

    pub fn with_id(
        from: String,
        to: String,
        kind: GraphEdgeKind,
        directive: Option<String>,
        edge_id: Option<String>,
    ) -> Self {
        Self {
            from,
            to,
            kind,
            directive,
            edge_id,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MemoryGraph {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

impl MemoryGraph {
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Atlas news claims carry the supporting article on each evidence node.
    pub fn is_claim_path(&self) -> bool {
        self.nodes.iter().any(|node| !node.article_id.is_empty())
    }

    pub fn node(&self, id: &str) -> Option<&GraphNode> {
        self.nodes.iter().find(|node| node.id == id)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForceLink {
    pub from: String,
    pub to: String,
    pub directive: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathBand {
    pub directive_id: String,
    pub directive_label: String,
    pub subjects: Vec<String>,
    pub evidence: Vec<String>,
    /// Finding label when a cited call serves this directive.
    pub finding: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReconPath {
    pub investigation: String,
    pub bands: Vec<PathBand>,
}

impl Store {
    /// The graph for one memory. A hand-saved memory with no claim is empty.
    pub fn graph_for_memory(&self, memory_id: &str) -> anyhow::Result<MemoryGraph> {
        let kind: Option<String> = self
            .conn
            .query_row(
                "SELECT memory_kind FROM memory_metadata WHERE memory_id=?1",
                [memory_id],
                |r| r.get::<_, String>(0),
            )
            .optional()?;

        let Some(kind_str) = kind else {
            return self.legacy_graph_for_memory(memory_id);
        };

        match kind_str.as_str() {
            "claim" | "atomic_claim" => {
                let mut graph = self.legacy_graph_for_memory(memory_id)?;
                if graph.nodes.iter().all(|n| n.kind != GraphNodeKind::Finding) {
                    if let Some(memory) = self.get_memory(memory_id)? {
                        graph.nodes.push(GraphNode {
                            id: memory_id.into(),
                            kind: GraphNodeKind::Finding,
                            label: memory.text.clone(),
                            detail: memory.text,
                            tags: vec![],
                            article_id: String::new(),
                            run_id: String::new(),
                            published_at: String::new(),
                        });
                    }
                }
                let associations = self.claim_associations(memory_id)?;
                for group in associations.actors {
                    if !graph.nodes.iter().any(|n| {
                        n.kind == GraphNodeKind::Entity
                            && crate::brain_actors::normalize_alias(&n.label) == group.name_key
                    }) {
                        graph.nodes.push(GraphNode {
                            id: group.key(),
                            kind: GraphNodeKind::Entity,
                            label: group.display_name.clone(),
                            detail: group.display_name,
                            tags: vec![],
                            article_id: String::new(),
                            run_id: String::new(),
                            published_at: String::new(),
                        });
                    }
                }
                for (id, title) in associations.threads {
                    graph.nodes.push(GraphNode {
                        id: format!("thread:{id}"),
                        kind: GraphNodeKind::Source,
                        label: title.clone(),
                        detail: title,
                        tags: vec![],
                        article_id: String::new(),
                        run_id: String::new(),
                        published_at: String::new(),
                    });
                }
                Ok(graph)
            }
            "cycle_brief" => self.build_brief_graph(memory_id),
            "actor" | "link" | "related" => self.build_actor_graph(memory_id, &kind_str),
            "manual_note" | "user_profile" | "investigation_digest" | "source_summary" => {
                Ok(MemoryGraph::default())
            }
            _ => self.legacy_graph_for_memory(memory_id),
        }
    }

    fn legacy_graph_for_memory(&self, memory_id: &str) -> anyhow::Result<MemoryGraph> {
        let Some(insight) = self.insight_for_memory(memory_id)? else {
            return Ok(MemoryGraph::default());
        };
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT s.run_id FROM insight_claims c              JOIN insight_sources s ON s.fingerprint = c.fingerprint              WHERE c.memory_id = ?1 AND s.run_id IS NOT NULL",
        )?;
        let run_ids: Vec<String> = stmt
            .query_map([memory_id], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        if insight.sources.iter().any(|source| {
            source.answer_id.starts_with("atlas-")
                || (source.thread_id.is_none() && source.run_id.is_some())
        }) {
            let mut articles = Vec::new();
            for source in &insight.sources {
                let Some(run_id) = source.run_id.as_deref() else {
                    continue;
                };
                if let Some(article) = self.atlas_article(run_id, &source.call_id)? {
                    articles.push(article);
                }
            }
            return Ok(build_claim_graph(&insight, &articles));
        }
        let mut plans = Vec::new();
        for run_id in run_ids {
            let Some(run) = self.get_run(&run_id)? else {
                continue;
            };
            let Some(raw) = run.plan_json.as_deref() else {
                continue;
            };
            if let Ok(plan) = serde_json::from_str::<Plan>(raw) {
                plans.push(plan);
            }
        }
        Ok(build_memory_graph(&insight, &plans))
    }

    /// Actor/Link/Related graph: one node per canonical actor, article
    /// occurrences as evidence nodes, observed Link edges and tentative
    /// Related edges kept as distinct edge kinds.
    fn build_actor_graph(&self, memory_id: &str, kind: &str) -> anyhow::Result<MemoryGraph> {
        use crate::memory_metadata::{EndpointRef, MemoryPayload};
        const MAX_ARTICLES: usize = 40;
        let mut graph = GraphBuilder::default();
        let mut articles: Vec<(String, String)> = Vec::new();
        let mut related_memories: Vec<String> = Vec::new();
        match kind {
            "actor" => {
                let canonical = crate::brain_actors::resolve(&self.conn, memory_id)?;
                let actor = self.actor_node(&mut graph, &canonical)?;
                let ids = crate::brain_actors::equivalent_ids(&self.conn, &canonical)?;
                for id in &ids {
                    let mut stmt = self.conn.prepare(
                        "SELECT t.origin,t.ref_id,s.memory_id FROM brain_memory_endpoints s
                         JOIN brain_memory_endpoints t ON t.memory_id=s.memory_id AND t.side='target' AND t.endpoint_kind='article'
                         JOIN memory_metadata mm ON mm.memory_id=s.memory_id AND mm.memory_kind='link' AND mm.lifecycle<>'retired'
                         WHERE s.side='source' AND s.endpoint_kind='memory' AND s.ref_id=?1 ORDER BY t.origin DESC, t.ref_id",
                    )?;
                    let rows = stmt
                        .query_map([id], |r| {
                            Ok((
                                r.get::<_, String>(0)?,
                                r.get::<_, String>(1)?,
                                r.get::<_, String>(2)?,
                            ))
                        })?
                        .collect::<rusqlite::Result<Vec<_>>>()?;
                    for (run, article, link) in rows {
                        if articles.len() >= MAX_ARTICLES {
                            break;
                        }
                        let Some(node) = self.article_node(&mut graph, &run, &article)? else {
                            continue;
                        };
                        let relation = match self.memory_payload(&link)? {
                            Some(MemoryPayload::Link(p)) => p.relation,
                            _ => "link".into(),
                        };
                        graph.edge(GraphEdge {
                            from: actor.clone(),
                            to: node,
                            kind: GraphEdgeKind::Link,
                            directive: Some(relation),
                            edge_id: Some(link),
                        });
                        articles.push((run, article));
                    }
                }
                for id in &ids {
                    let mut stmt = self.conn.prepare(
                        "SELECT m.id, m.text FROM brain_memory_actor_refs r
                         JOIN memories m ON m.id = r.memory_id
                         LEFT JOIN memory_metadata mm ON mm.memory_id = m.id
                         WHERE r.actor_id = ?1 AND (mm.lifecycle IS NULL OR mm.lifecycle = 'active')
                         ORDER BY m.pinned DESC, m.created_at DESC LIMIT 40",
                    )?;
                    let claim_rows = stmt
                        .query_map([id], |r| {
                            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                        })?
                        .collect::<rusqlite::Result<Vec<_>>>()?;
                    for (cid, ctext) in claim_rows {
                        graph.node(GraphNode {
                            id: cid.clone(),
                            kind: GraphNodeKind::Finding,
                            label: ctext.clone(),
                            detail: ctext,
                            tags: vec![],
                            article_id: String::new(),
                            run_id: String::new(),
                            published_at: String::new(),
                        });
                        graph.edge(GraphEdge {
                            from: actor.clone(),
                            to: cid.clone(),
                            kind: GraphEdgeKind::Concerns,
                            directive: Some("concerns".into()),
                            edge_id: Some(format!("edge:{actor}:{cid}")),
                        });
                    }
                }
                for (run, article) in &articles {
                    let mut stmt = self.conn.prepare(
                        "SELECT e.memory_id FROM brain_memory_endpoints e
                         JOIN memory_metadata mm ON mm.memory_id=e.memory_id AND mm.memory_kind='related' AND mm.lifecycle<>'retired'
                         WHERE e.endpoint_kind='article' AND e.origin=?1 AND e.ref_id=?2",
                    )?;
                    related_memories.extend(
                        stmt.query_map([run, article], |r| r.get::<_, String>(0))?
                            .collect::<rusqlite::Result<Vec<_>>>()?,
                    );
                }
            }
            "link" | "related" => {
                let endpoint =
                    |g: &mut GraphBuilder, e: &EndpointRef| -> anyhow::Result<Option<String>> {
                        match e {
                            EndpointRef::Memory { memory_id, .. } => {
                                let mkind: Option<String> = self
                                .conn
                                .query_row(
                                    "SELECT memory_kind FROM memory_metadata WHERE memory_id=?1",
                                    [memory_id],
                                    |r| r.get(0),
                                )
                                .optional()?;
                                match mkind.as_deref() {
                                    Some("actor") => {
                                        let canonical =
                                            crate::brain_actors::resolve(&self.conn, memory_id)?;
                                        Ok(Some(self.actor_node(g, &canonical)?))
                                    }
                                    _ => {
                                        if let Some(mem) = self.get_memory(memory_id)? {
                                            let id = mem.id.clone();
                                            g.node(GraphNode {
                                                id: id.clone(),
                                                kind: GraphNodeKind::Finding,
                                                label: mem.text.clone(),
                                                detail: mem.text,
                                                tags: vec![],
                                                article_id: String::new(),
                                                run_id: String::new(),
                                                published_at: String::new(),
                                            });
                                            Ok(Some(id))
                                        } else {
                                            Ok(None)
                                        }
                                    }
                                }
                            }
                            EndpointRef::Article {
                                run_id, article_id, ..
                            } => self.article_node(g, run_id, article_id),
                            EndpointRef::Source {
                                origin, source_id, ..
                            } => {
                                let id = format!("source:{origin}:{source_id}");
                                g.node(GraphNode {
                                    id: id.clone(),
                                    kind: GraphNodeKind::Source,
                                    label: source_id.clone(),
                                    detail: format!("{origin} {source_id}"),
                                    tags: vec![],
                                    article_id: String::new(),
                                    run_id: String::new(),
                                    published_at: String::new(),
                                });
                                Ok(Some(id))
                            }
                        }
                    };
                match self.memory_payload(memory_id)? {
                    Some(MemoryPayload::Link(p)) => {
                        if let (Some(a), Some(b)) = (
                            endpoint(&mut graph, &p.source)?,
                            endpoint(&mut graph, &p.target)?,
                        ) {
                            graph.edge(GraphEdge {
                                from: a,
                                to: b,
                                kind: GraphEdgeKind::Link,
                                directive: Some(p.relation),
                                edge_id: Some(memory_id.into()),
                            });
                        }
                    }
                    Some(MemoryPayload::Related(_)) => related_memories.push(memory_id.into()),
                    _ => {}
                }
            }
            _ => {}
        }
        related_memories.sort();
        related_memories.dedup();
        // One Related edge per unordered article pair (records exist per side).
        let mut pairs = HashSet::new();
        for related in related_memories {
            if let Some(MemoryPayload::Related(p)) = self.memory_payload(&related)? {
                let (
                    EndpointRef::Article {
                        run_id: ar,
                        article_id: aa,
                        ..
                    },
                    EndpointRef::Article {
                        run_id: br,
                        article_id: ba,
                        ..
                    },
                ) = (&p.source, &p.target)
                else {
                    continue;
                };
                let mut key = [format!("{ar}/{aa}"), format!("{br}/{ba}")];
                key.sort();
                if !pairs.insert(key) {
                    continue;
                }
                if let (Some(a), Some(b)) = (
                    self.article_node(&mut graph, ar, aa)?,
                    self.article_node(&mut graph, br, ba)?,
                ) {
                    graph.edge(GraphEdge {
                        from: a,
                        to: b,
                        kind: GraphEdgeKind::Related,
                        directive: Some(p.basis),
                        edge_id: Some(related),
                    });
                }
            }
        }
        Ok(MemoryGraph {
            nodes: graph.nodes,
            edges: graph.edges,
        })
    }

    fn actor_node(&self, graph: &mut GraphBuilder, canonical: &str) -> anyhow::Result<String> {
        let row = crate::brain_actors::actor(&self.conn, canonical)?;
        let (label, detail) = row
            .map(|a| {
                (
                    a.display_name.clone(),
                    format!("{} · {}", a.display_name, a.subtype),
                )
            })
            .unwrap_or_else(|| (canonical.to_string(), canonical.to_string()));
        Ok(graph.node(GraphNode {
            id: format!("actor:{canonical}"),
            kind: GraphNodeKind::Actor,
            label,
            detail,
            tags: vec![],
            article_id: String::new(),
            run_id: String::new(),
            published_at: String::new(),
        }))
    }

    fn article_node(
        &self,
        graph: &mut GraphBuilder,
        run: &str,
        article: &str,
    ) -> anyhow::Result<Option<String>> {
        let Some(a) = self.atlas_article(run, article)? else {
            return Ok(None);
        };
        Ok(Some(graph.node(GraphNode {
            id: format!("article:{run}/{article}"),
            kind: GraphNodeKind::Evidence,
            label: a.title.clone(),
            detail: format!("{} · {}", a.source_name, a.title),
            tags: vec![],
            article_id: a.id.clone(),
            run_id: a.run_id.clone(),
            published_at: a.published_at.clone(),
        })))
    }

    fn build_brief_graph(&self, memory_id: &str) -> anyhow::Result<MemoryGraph> {
        let mut stmt = self.conn.prepare("SELECT member_id FROM memory_brief_memberships WHERE brief_id=?1 AND member_type IN ('atomic_claim','claim','inference')")?;
        let members: Vec<String> = stmt
            .query_map([memory_id], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<_>>()?;

        let mut graph = GraphBuilder::default();
        let brief_node = graph.node(GraphNode {
            id: format!("brief:{}", memory_id),
            kind: GraphNodeKind::Finding,
            label: "Cycle Brief".into(),
            detail: "Cycle Brief".into(),
            tags: vec![],
            article_id: String::new(),
            run_id: String::new(),
            published_at: String::new(),
        });

        for member_id in members {
            if let Ok(Some(insight)) = self.insight_for_memory(&member_id) {
                let claim_node = graph.node(GraphNode {
                    id: format!("finding:{}", member_id),
                    kind: GraphNodeKind::Finding,
                    label: format!("{} → {}", insight.predicate, insight.object_value),
                    detail: format!(
                        "{} · {} → {}",
                        insight.entity, insight.predicate, insight.object_value
                    ),
                    tags: vec![],
                    article_id: String::new(),
                    run_id: String::new(),
                    published_at: String::new(),
                });
                graph.edge(GraphEdge {
                    from: claim_node.clone(),
                    to: brief_node.clone(),
                    kind: GraphEdgeKind::Supports,
                    directive: None,
                    edge_id: None,
                });
            }
        }

        Ok(MemoryGraph {
            nodes: graph.nodes,
            edges: graph.edges,
        })
    }
}

pub fn build_memory_graph(insight: &InsightView, plans: &[Plan]) -> MemoryGraph {
    let mut objective = String::new();
    let mut directives: Vec<Directive> = Vec::new();
    let mut calls: Vec<PlanCall> = Vec::new();
    let mut seen_directives = HashSet::new();
    let mut seen_calls = HashSet::new();
    for plan in plans {
        if objective.is_empty() {
            let text = plan.objective.trim();
            if !text.is_empty() {
                objective = text.to_string();
            }
        }
        for directive in &plan.directives {
            if directive.id.is_empty() || !seen_directives.insert(directive.id.clone()) {
                continue;
            }
            directives.push(directive.clone());
        }
        for call in &plan.calls {
            let key = if call.call_id.is_empty() {
                call.step_id.clone()
            } else {
                call.call_id.clone()
            };
            if key.is_empty() || !seen_calls.insert(key) {
                continue;
            }
            calls.push(call.clone());
        }
    }

    let mut graph = GraphBuilder::default();
    let investigation = graph.node(GraphNode {
        id: "investigation".into(),
        kind: GraphNodeKind::Investigation,
        label: if objective.is_empty() {
            "Investigation".into()
        } else {
            objective.clone()
        },
        detail: if objective.is_empty() {
            "Investigation".into()
        } else {
            objective
        },
        tags: Vec::new(),
        article_id: String::new(),
        run_id: String::new(),
        published_at: String::new(),
    });

    for directive in &directives {
        let id = graph.node(GraphNode {
            id: format!("directive:{}", directive.id),
            kind: GraphNodeKind::Directive,
            label: directive.id.clone(),
            detail: format!(
                "{}\n{}\n{}",
                directive.goal,
                directive.done_when,
                directive.targets.join(", ")
            ),
            tags: vec![directive.id.clone()],
            article_id: String::new(),
            run_id: String::new(),
            published_at: String::new(),
        });
        for entity in &directive.entities {
            let Some(entity_id) = graph.entity(entity) else {
                continue;
            };
            graph.edge(GraphEdge {
                from: id.clone(),
                to: entity_id,
                kind: GraphEdgeKind::Investigates,
                directive: Some(directive.id.clone()),
                edge_id: None,
            });
        }
    }

    let claim_entity = graph.entity(&insight.entity);
    let topic = insight.topic.trim();
    let topic_id = if topic.is_empty() {
        None
    } else {
        Some(graph.node(GraphNode {
            id: format!("topic:{}", topic.to_lowercase()),
            kind: GraphNodeKind::Topic,
            label: topic.to_string(),
            detail: format!("Topic {topic}"),
            tags: Vec::new(),
            article_id: String::new(),
            run_id: String::new(),
            published_at: String::new(),
        }))
    };
    let finding = graph.node(GraphNode {
        id: format!("finding:{}", insight.memory_id),
        kind: GraphNodeKind::Finding,
        label: format!("{} → {}", insight.predicate, insight.object_value),
        detail: format!(
            "{} · {} → {}\n{}\n{}",
            insight.entity,
            insight.predicate,
            insight.object_value,
            insight.presentation(),
            insight.topic
        ),
        tags: Vec::new(),
        article_id: String::new(),
        run_id: String::new(),
        published_at: String::new(),
    });
    graph.edge(GraphEdge {
        from: finding.clone(),
        to: investigation,
        kind: GraphEdgeKind::DiscoveredIn,
        directive: None,
        edge_id: None,
    });
    if let Some(entity_id) = claim_entity {
        graph.edge(GraphEdge {
            from: finding.clone(),
            to: entity_id,
            kind: GraphEdgeKind::Concerns,
            directive: None,
            edge_id: None,
        });
    }
    if let Some(topic_id) = topic_id.clone() {
        graph.edge(GraphEdge {
            from: finding.clone(),
            to: topic_id,
            kind: GraphEdgeKind::Concerns,
            directive: None,
            edge_id: None,
        });
    }

    let cited: Vec<&InsightSource> = insight
        .sources
        .iter()
        .filter(|source| !source.deleted_origin && !source.call_id.is_empty())
        .collect();
    let mut answered: HashSet<String> = HashSet::new();
    for source in &cited {
        let Some(call) = call_for(&calls, &source.call_id) else {
            continue;
        };
        for directive_id in ordered_serves(call, &directives) {
            answered.insert(directive_id);
        }
    }
    for directive in &directives {
        if !answered.contains(&directive.id) {
            continue;
        }
        graph.edge(GraphEdge {
            from: finding.clone(),
            to: format!("directive:{}", directive.id),
            kind: GraphEdgeKind::Answers,
            directive: Some(directive.id.clone()),
            edge_id: None,
        });
        if let Some(topic_id) = topic_id.clone() {
            graph.edge(GraphEdge {
                from: format!("directive:{}", directive.id),
                to: topic_id,
                kind: GraphEdgeKind::Investigates,
                directive: Some(directive.id.clone()),
                edge_id: None,
            });
        }
    }

    for source in &cited {
        let call = call_for(&calls, &source.call_id);
        let served = call
            .map(|call| ordered_serves(call, &directives))
            .unwrap_or_default();
        let source_label = source
            .source_url
            .clone()
            .filter(|url| !url.trim().is_empty())
            .or_else(|| {
                call.map(|call| call.tool_id.clone())
                    .filter(|tool| !tool.is_empty())
            })
            .unwrap_or_else(|| "source".into());
        let evidence = graph.node(GraphNode {
            id: format!("evidence:{}", source.call_id),
            kind: GraphNodeKind::Evidence,
            label: source.call_id.clone(),
            detail: format!("{} · {source_label}", source.call_id),
            tags: served,
            article_id: String::new(),
            run_id: String::new(),
            published_at: String::new(),
        });
        let domain = crate::atlas::host_of(&source_label);
        let source_id = graph.node(GraphNode {
            id: format!("source:{}", source_label.to_lowercase()),
            kind: GraphNodeKind::Source,
            label: source_label.clone(),
            detail: source_reliability_detail(&source_label, &domain),
            tags: Vec::new(),
            article_id: String::new(),
            run_id: String::new(),
            published_at: String::new(),
        });
        graph.edge(GraphEdge {
            from: evidence.clone(),
            to: finding.clone(),
            kind: if insight.metadata.is_some() {
                GraphEdgeKind::Reports
            } else {
                GraphEdgeKind::Supports
            },
            directive: None,
            edge_id: None,
        });
        graph.edge(GraphEdge {
            from: evidence,
            to: source_id,
            kind: GraphEdgeKind::DerivedFrom,
            directive: None,
            edge_id: None,
        });
    }

    MemoryGraph {
        nodes: graph.nodes,
        edges: graph.edges,
    }
}

/// News-cycle claim: the concluding relation, with each supporting article as hard evidence.
pub fn build_claim_graph(
    insight: &InsightView,
    articles: &[crate::store::AtlasArticleRow],
) -> MemoryGraph {
    let mut graph = GraphBuilder::default();
    let relation = format!(
        "{} {} {}",
        insight.entity, insight.predicate, insight.object_value
    );
    let level = if let Some(metadata) = &insight.metadata {
        format!(
            "Kind: {} · {} · {}. Source assertions remain claims.",
            metadata.insight_kind, metadata.lifecycle, metadata.assessment.relation
        )
    } else {
        format!("Legacy level: {} · unassessed.", insight.classification)
    };
    graph.node(GraphNode {
        id: "investigation".into(),
        kind: GraphNodeKind::Investigation,
        label: relation.clone(),
        detail: format!("{relation}\n{level}"),
        tags: Vec::new(),
        article_id: String::new(),
        run_id: String::new(),
        published_at: String::new(),
    });
    let directive = graph.node(GraphNode {
        id: "directive:articles".into(),
        kind: GraphNodeKind::Directive,
        label: "articles".into(),
        detail: format!(
            "{}\n{}",
            if insight.topic.trim().is_empty() {
                "articles"
            } else {
                insight.topic.trim()
            },
            if insight.metadata.is_some() {
                "Original passages establish source attribution; corroboration remains separate."
            } else {
                "Supporting articles are the hard evidence for this legacy claim."
            }
        ),
        tags: vec!["articles".into()],
        article_id: String::new(),
        run_id: String::new(),
        published_at: String::new(),
    });
    if let Some(entity_id) = graph.entity(&insight.entity) {
        graph.edge(GraphEdge {
            from: directive.clone(),
            to: entity_id,
            kind: GraphEdgeKind::Investigates,
            directive: Some("articles".into()),
            edge_id: None,
        });
    }
    let topic = insight.topic.trim();
    if !topic.is_empty() {
        let topic_id = graph.node(GraphNode {
            id: format!("topic:{}", topic.to_lowercase()),
            kind: GraphNodeKind::Topic,
            label: topic.to_string(),
            detail: format!("Topic {topic}"),
            tags: Vec::new(),
            article_id: String::new(),
            run_id: String::new(),
            published_at: String::new(),
        });
        graph.edge(GraphEdge {
            from: directive.clone(),
            to: topic_id,
            kind: GraphEdgeKind::Investigates,
            directive: Some("articles".into()),
            edge_id: None,
        });
    }
    let finding = graph.node(GraphNode {
        id: format!("finding:{}", insight.memory_id),
        kind: GraphNodeKind::Finding,
        label: format!("{} → {}", insight.predicate, insight.object_value),
        detail: format!(
            "{} · {} → {}\n{}\n{}\n{level}",
            insight.entity,
            insight.predicate,
            insight.object_value,
            insight.presentation(),
            insight.topic
        ),
        tags: Vec::new(),
        article_id: String::new(),
        run_id: String::new(),
        published_at: String::new(),
    });
    graph.edge(GraphEdge {
        from: finding.clone(),
        to: directive.clone(),
        kind: GraphEdgeKind::Answers,
        directive: Some("articles".into()),
        edge_id: None,
    });
    for source in &insight.sources {
        let article = source.run_id.as_deref().and_then(|run_id| {
            articles
                .iter()
                .find(|article| article.run_id == run_id && article.id == source.call_id)
        });
        let title = article
            .map(|article| article.title.trim())
            .filter(|title| !title.is_empty())
            .unwrap_or(source.call_id.as_str());
        let published = article
            .map(|article| article.published_at.as_str())
            .filter(|stamp| !stamp.is_empty())
            .unwrap_or(source.published_at.as_str());
        let when = if published.is_empty() {
            String::new()
        } else {
            crate::atlas::friendly_date(published)
        };
        let label = if when.is_empty() {
            title.to_string()
        } else {
            format!("{title} · {when}")
        };
        let run_id = source.run_id.clone().unwrap_or_default();
        let publisher = article
            .map(|article| article.source_name.trim())
            .filter(|name| !name.is_empty())
            .map(str::to_string)
            .or_else(|| {
                source
                    .source_url
                    .clone()
                    .filter(|url| !url.trim().is_empty())
            })
            .unwrap_or_else(|| "source".into());
        let domain = article
            .map(|article| article.source_domain.as_str())
            .unwrap_or("");
        let source_detail = source_reliability_detail(&publisher, domain);
        let evidence = graph.node(GraphNode {
            id: format!("evidence:{}:{run_id}", source.call_id),
            kind: GraphNodeKind::Evidence,
            label,
            detail: format!(
                "{title}\npublished {published}\n{publisher}\nHard evidence for the concluding claim."
            ),
            tags: vec!["articles".into()],
            article_id: source.call_id.clone(),
            run_id: run_id.clone(),
            published_at: published.to_string(),
        });
        let source_id = graph.node(GraphNode {
            id: format!("source:{}", publisher.to_lowercase()),
            kind: GraphNodeKind::Source,
            label: publisher.clone(),
            detail: source_detail,
            tags: Vec::new(),
            article_id: String::new(),
            run_id: String::new(),
            published_at: String::new(),
        });
        graph.edge(GraphEdge {
            from: evidence.clone(),
            to: finding.clone(),
            kind: if insight.metadata.is_some() {
                GraphEdgeKind::Reports
            } else {
                GraphEdgeKind::Supports
            },
            directive: Some("articles".into()),
            edge_id: None,
        });
        graph.edge(GraphEdge {
            from: evidence,
            to: source_id,
            kind: GraphEdgeKind::DerivedFrom,
            directive: None,
            edge_id: None,
        });
    }
    MemoryGraph {
        nodes: graph.nodes,
        edges: graph.edges,
    }
}

#[derive(Default)]
struct GraphBuilder {
    nodes: Vec<GraphNode>,
    index: HashMap<String, usize>,
    edges: Vec<GraphEdge>,
    edge_keys: HashSet<String>,
    entities: HashMap<String, String>,
}

impl GraphBuilder {
    fn node(&mut self, node: GraphNode) -> String {
        if let Some(existing) = self.index.get(&node.id) {
            return self.nodes[*existing].id.clone();
        }
        let id = node.id.clone();
        self.index.insert(id.clone(), self.nodes.len());
        self.nodes.push(node);
        id
    }

    fn entity(&mut self, name: &str) -> Option<String> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return None;
        }
        let key = trimmed.to_lowercase();
        if let Some(id) = self.entities.get(&key) {
            return Some(id.clone());
        }
        let id = self.node(GraphNode {
            id: format!("entity:{key}"),
            kind: GraphNodeKind::Entity,
            label: trimmed.to_string(),
            detail: format!("Entity {trimmed}"),
            tags: Vec::new(),
            article_id: String::new(),
            run_id: String::new(),
            published_at: String::new(),
        });
        self.entities.insert(key, id.clone());
        Some(id)
    }

    fn edge(&mut self, edge: GraphEdge) {
        let key = format!(
            "{}|{}|{}|{}",
            edge.from,
            edge.to,
            edge.kind.verb(),
            edge.directive.as_deref().unwrap_or("")
        );
        if self.edge_keys.insert(key) {
            self.edges.push(edge);
        }
    }
}

/// Entity, topic, and finding links colored by the directive that joins them.
pub fn force_links(graph: &MemoryGraph) -> Vec<ForceLink> {
    let mut by_directive: HashMap<&str, Vec<&str>> = HashMap::new();
    for edge in &graph.edges {
        if edge.kind != GraphEdgeKind::Investigates {
            continue;
        }
        let Some(directive) = edge.directive.as_deref() else {
            continue;
        };
        let Some(target) = graph.node(&edge.to) else {
            continue;
        };
        if !matches!(
            target.kind,
            GraphNodeKind::Entity | GraphNodeKind::Topic | GraphNodeKind::Finding
        ) {
            continue;
        }
        let targets = by_directive.entry(directive).or_default();
        if !targets.contains(&edge.to.as_str()) {
            targets.push(edge.to.as_str());
        }
    }
    let mut links = Vec::new();
    let mut seen = HashSet::new();
    let push = |links: &mut Vec<ForceLink>,
                seen: &mut HashSet<(String, String, String)>,
                from: &str,
                to: &str,
                directive: &str| {
        if from == to {
            return;
        }
        let (left, right) = if from < to {
            (from.to_string(), to.to_string())
        } else {
            (to.to_string(), from.to_string())
        };
        if seen.insert((left.clone(), right.clone(), directive.to_string())) {
            links.push(ForceLink {
                from: left,
                to: right,
                directive: directive.to_string(),
            });
        }
    };
    for (directive, targets) in &by_directive {
        for i in 0..targets.len() {
            for j in (i + 1)..targets.len() {
                push(&mut links, &mut seen, targets[i], targets[j], directive);
            }
        }
    }
    for edge in &graph.edges {
        if edge.kind != GraphEdgeKind::Answers {
            continue;
        }
        let Some(directive) = edge.directive.as_deref() else {
            continue;
        };
        for concern in &graph.edges {
            if concern.kind == GraphEdgeKind::Concerns && concern.from == edge.from {
                push(&mut links, &mut seen, &edge.from, &concern.to, directive);
            }
        }
    }
    links
}

pub fn recon_path(graph: &MemoryGraph) -> ReconPath {
    let investigation = graph
        .nodes
        .iter()
        .find(|node| node.kind == GraphNodeKind::Investigation)
        .map(|node| node.label.clone())
        .unwrap_or_else(|| "Investigation".into());
    let finding = graph
        .nodes
        .iter()
        .find(|node| node.kind == GraphNodeKind::Finding)
        .map(|node| node.label.clone());
    let chosen = choose_directive(graph);
    let mut bands = Vec::new();
    for directive in graph
        .nodes
        .iter()
        .filter(|node| node.kind == GraphNodeKind::Directive)
    {
        let directive_id = directive
            .id
            .strip_prefix("directive:")
            .unwrap_or(directive.label.as_str())
            .to_string();
        if chosen.as_deref() != Some(directive_id.as_str()) {
            continue;
        }
        let subjects = graph
            .edges
            .iter()
            .filter(|edge| edge.kind == GraphEdgeKind::Investigates && edge.from == directive.id)
            .filter_map(|edge| graph.node(&edge.to).map(|node| node.label.clone()))
            .collect::<Vec<_>>();
        let evidence = graph
            .nodes
            .iter()
            .filter(|node| {
                node.kind == GraphNodeKind::Evidence
                    && node.tags.iter().any(|tag| tag == &directive_id)
            })
            .map(|node| node.detail.clone())
            .collect::<Vec<_>>();
        let answered = graph
            .edges
            .iter()
            .any(|edge| edge.kind == GraphEdgeKind::Answers && edge.to == directive.id);
        bands.push(PathBand {
            directive_id,
            directive_label: directive.label.clone(),
            subjects,
            evidence,
            finding: if answered { finding.clone() } else { None },
        });
    }
    ReconPath {
        investigation,
        bands,
    }
}

/// Plain-text recon path used as the synthesis prompt for a saved graph summary.
pub fn graph_brief(graph: &MemoryGraph) -> String {
    if graph.is_empty() {
        return String::new();
    }
    let path = recon_path(graph);
    let chosen = path.bands.first().map(|band| band.directive_id.as_str());
    let mut lines = vec![format!("Investigation: {}", path.investigation)];
    if let Some(finding) = graph
        .nodes
        .iter()
        .find(|node| node.kind == GraphNodeKind::Finding)
    {
        lines.push(format!(
            "Conclusion: {}",
            finding.detail.replace('\n', " | ")
        ));
    }
    if path.bands.is_empty() {
        for node in &graph.nodes {
            lines.push(format!("{}: {}", node.kind.label(), node.label));
            let detail = node.detail.replace('\n', " | ");
            if !detail.is_empty() && detail != node.label {
                lines.push(format!("  {detail}"));
            }
        }
        lines.push(String::new());
        lines.push("Topology connections".into());
        for edge in &graph.edges {
            let from_node = graph.node(&edge.from);
            let to_node = graph.node(&edge.to);
            let from = from_node
                .map(|node| node.label.as_str())
                .unwrap_or(edge.from.as_str());
            let to = to_node
                .map(|node| node.label.as_str())
                .unwrap_or(edge.to.as_str());
            let verb = edge.directive.as_deref().unwrap_or(edge.kind.verb());
            lines.push(format!("{from} {verb} {to}"));
        }
        return lines.join("\n");
    }
    for node in graph.nodes.iter().filter(|node| keep_node(node, chosen)) {
        lines.push(format!("{}: {}", node.kind.label(), node.label));
        let detail = node.detail.replace('\n', " | ");
        if !detail.is_empty() && detail != node.label {
            lines.push(format!("  {detail}"));
        }
    }
    lines.push(String::new());
    lines.push(if graph.is_claim_path() {
        "Claim path".into()
    } else {
        "Recon path".into()
    });
    for band in &path.bands {
        let goal = graph
            .nodes
            .iter()
            .find(|node| node.kind == GraphNodeKind::Directive && node.label == band.directive_id)
            .and_then(|node| node.detail.lines().next())
            .filter(|line| !line.is_empty())
            .unwrap_or(band.directive_label.as_str());
        lines.push(format!("Directive {}: {goal}", band.directive_id));
        if !band.subjects.is_empty() {
            lines.push(format!("  investigates {}", band.subjects.join(", ")));
        }
        if band.evidence.is_empty() {
            lines.push("  evidence: none".into());
        } else {
            for item in &band.evidence {
                lines.push(format!("  evidence: {}", item.replace('\n', " ")));
            }
        }
        match &band.finding {
            Some(finding) => lines.push(format!("  finding: {finding}")),
            None => lines.push("  finding: none".into()),
        }
    }
    for edge in &graph.edges {
        let from_node = graph.node(&edge.from);
        let to_node = graph.node(&edge.to);
        if from_node.is_some_and(|node| !keep_node(node, chosen))
            || to_node.is_some_and(|node| !keep_node(node, chosen))
        {
            continue;
        }
        let from = from_node
            .map(|node| node.label.as_str())
            .unwrap_or(edge.from.as_str());
        let to = to_node
            .map(|node| node.label.as_str())
            .unwrap_or(edge.to.as_str());
        lines.push(format!("{from} {} {to}", edge.kind.verb()));
    }
    lines.join("\n")
}

fn source_reliability_detail(publisher: &str, domain: &str) -> String {
    use crate::osint::wikipedia_rsp;
    let host = if domain.trim().is_empty() {
        crate::atlas::host_of(publisher)
    } else {
        wikipedia_rsp::normalize_host(domain)
    };
    match wikipedia_rsp::cached_index().and_then(|index| {
        let entry = index.lookup_domain(&host)?;
        Some(entry.clone())
    }) {
        Some(entry) => {
            let letter = entry.status.reliability();
            format!(
                "Source {publisher} · {} {} · RSP {} ({})",
                letter.as_str(),
                letter.label().to_ascii_lowercase(),
                entry.status.label(),
                if entry.last_year.is_empty() {
                    "n.d."
                } else {
                    entry.last_year.as_str()
                }
            )
        }
        None => {
            if host.is_empty() {
                format!("Source {publisher}")
            } else {
                format!("Source {publisher} · F cannot be judged · {host} not in RSP")
            }
        }
    }
}

fn ordered_serves(call: &PlanCall, directives: &[Directive]) -> Vec<String> {
    let known: HashSet<&str> = directives
        .iter()
        .map(|directive| directive.id.as_str())
        .collect();
    directive_ids(&call.reason)
        .into_iter()
        .filter(|id| known.contains(id.as_str()))
        .collect()
}

/// The one directive this insight rests on. Cited evidence counts first: a call that
/// serves only one directive outweighs a call shared across several. The insight's
/// entity, topic, and finding, then the question, break the remaining ties.
fn choose_directive(graph: &MemoryGraph) -> Option<String> {
    let directives: Vec<&GraphNode> = graph
        .nodes
        .iter()
        .filter(|node| node.kind == GraphNodeKind::Directive)
        .collect();
    if directives.is_empty() {
        return None;
    }
    let evidence: Vec<&GraphNode> = graph
        .nodes
        .iter()
        .filter(|node| node.kind == GraphNodeKind::Evidence)
        .collect();
    let tagged: HashSet<&str> = evidence
        .iter()
        .flat_map(|node| node.tags.iter().map(String::as_str))
        .collect();
    let claim = claim_tokens(graph);
    let question = tokens(
        graph
            .nodes
            .iter()
            .find(|node| node.kind == GraphNodeKind::Investigation)
            .map(|node| node.label.as_str())
            .unwrap_or(""),
    );
    let mut best_id = None;
    let mut best_score = i32::MIN;
    for directive in &directives {
        let id = directive
            .id
            .strip_prefix("directive:")
            .unwrap_or(directive.label.as_str());
        if !tagged.is_empty() && !tagged.contains(id) {
            continue;
        }
        let mut score = 0;
        for item in &evidence {
            if item.tags.first().map(String::as_str) == Some(id) {
                score += if item.tags.len() == 1 { 8 } else { 3 };
            } else if item.tags.iter().any(|tag| tag == id) {
                score += 1;
            }
        }
        let directive_tokens = directive_tokens(graph, directive);
        for token in &claim {
            if directive_tokens.contains(token) {
                score += 3;
            }
        }
        for token in &question {
            if directive_tokens.contains(token) && !claim.contains(token) {
                score += 2;
            }
        }
        if graph
            .edges
            .iter()
            .any(|edge| edge.kind == GraphEdgeKind::Answers && edge.to == directive.id)
        {
            score += 1;
        }
        if score > best_score {
            best_score = score;
            best_id = Some(id.to_string());
        }
    }
    best_id.or_else(|| {
        directives.first().map(|directive| {
            directive
                .id
                .strip_prefix("directive:")
                .unwrap_or(directive.label.as_str())
                .to_string()
        })
    })
}

fn keep_node(node: &GraphNode, chosen: Option<&str>) -> bool {
    let Some(chosen) = chosen else {
        return true;
    };
    match node.kind {
        GraphNodeKind::Directive => {
            node.id == format!("directive:{chosen}") || node.label == chosen
        }
        GraphNodeKind::Evidence => {
            node.tags.is_empty() || node.tags.iter().any(|tag| tag == chosen)
        }
        _ => true,
    }
}

fn claim_tokens(graph: &MemoryGraph) -> HashSet<String> {
    let Some(finding) = graph
        .nodes
        .iter()
        .find(|node| node.kind == GraphNodeKind::Finding)
    else {
        return HashSet::new();
    };
    let mut text = finding.label.clone();
    for edge in &graph.edges {
        if edge.kind == GraphEdgeKind::Concerns && edge.from == finding.id {
            if let Some(node) = graph.node(&edge.to) {
                text.push(' ');
                text.push_str(&node.label);
            }
        }
    }
    tokens(&text)
}

fn directive_tokens(graph: &MemoryGraph, directive: &GraphNode) -> HashSet<String> {
    let mut text = directive.detail.clone();
    for edge in &graph.edges {
        if edge.kind != GraphEdgeKind::Investigates || edge.from != directive.id {
            continue;
        }
        let Some(node) = graph.node(&edge.to) else {
            continue;
        };
        if node.kind == GraphNodeKind::Entity {
            text.push(' ');
            text.push_str(&node.label);
        }
    }
    tokens(&text)
}

fn tokens(text: &str) -> HashSet<String> {
    text.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|token| token.len() >= 4 && !STOP.contains(&token.to_ascii_lowercase().as_str()))
        .map(|token| token.to_ascii_lowercase())
        .collect()
}

const STOP: &[&str] = &[
    "about",
    "been",
    "collect",
    "find",
    "found",
    "from",
    "have",
    "identify",
    "into",
    "investigation",
    "locate",
    "official",
    "public",
    "recent",
    "that",
    "their",
    "this",
    "were",
    "what",
    "when",
    "where",
    "which",
    "with",
    "your",
];

fn directive_ids(reason: &str) -> Vec<String> {
    let mut ids = Vec::new();
    for token in reason.split(|c: char| !c.is_ascii_alphanumeric()) {
        if token.len() >= 2
            && token.starts_with('d')
            && token[1..].bytes().all(|byte| byte.is_ascii_digit())
            && !ids.iter().any(|id| id == token)
        {
            ids.push(token.to_string());
        }
    }
    ids
}

fn call_for<'a>(calls: &'a [PlanCall], call_id: &str) -> Option<&'a PlanCall> {
    calls.iter().find(|call| {
        call.call_id == call_id
            || call.step_id == call_id
            || call.evidence_ids.iter().any(|id| id == call_id)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brain::MemorySource;

    fn sample() -> (InsightView, Plan) {
        let insight = InsightView {
            metadata: None,
            memory_id: "m1".into(),
            entity: "elon musk".into(),
            predicate: "employs".into(),
            object_value: "tesla".into(),
            topic: "identity".into(),
            classification: "fact".into(),
            confidence: 0.9,
            sources: vec![InsightSource {
                thread_id: Some("t".into()),
                run_id: Some("run".into()),
                answer_id: "a".into(),
                call_id: "call-w1".into(),
                source_url: None,
                deleted_origin: false,
                published_at: String::new(),
            }],
            related: vec![],
        };
        let plan = Plan {
            objective: "who is Elon Musk?".into(),
            directives: vec![
                Directive {
                    id: "d1".into(),
                    goal: "Establish identity and public roles".into(),
                    entities: vec!["Elon Musk".into()],
                    targets: vec!["person_name".into()],
                    done_when: "roles confirmed".into(),
                    query: String::new(),
                },
                Directive {
                    id: "d2".into(),
                    goal: "Find official accounts".into(),
                    entities: vec!["Elon Musk".into()],
                    ..Directive::default()
                },
                Directive {
                    id: "d3".into(),
                    goal: "Find affiliated orgs".into(),
                    entities: vec!["Tesla".into()],
                    ..Directive::default()
                },
            ],
            calls: vec![
                PlanCall {
                    step_id: "s1".into(),
                    tool_id: "wikidata_entities".into(),
                    reason: "d1".into(),
                    call_id: "call-w1".into(),
                    ..PlanCall::default()
                },
                PlanCall {
                    step_id: "s2".into(),
                    tool_id: "hunter_domain".into(),
                    reason: "d3".into(),
                    call_id: "call-h1".into(),
                    ..PlanCall::default()
                },
            ],
            ..Plan::default()
        };
        (insight, plan)
    }

    #[test]
    fn finding_answers_cited_directive_and_skips_uncited_call() {
        let (insight, plan) = sample();
        let graph = build_memory_graph(&insight, &[plan]);
        let finding = graph
            .nodes
            .iter()
            .find(|node| node.kind == GraphNodeKind::Finding)
            .unwrap();
        assert!(graph.edges.iter().any(|edge| {
            edge.kind == GraphEdgeKind::Answers
                && edge.from == finding.id
                && edge.directive.as_deref() == Some("d1")
        }));
        assert!(!graph.edges.iter().any(|edge| {
            edge.kind == GraphEdgeKind::Answers && edge.directive.as_deref() == Some("d3")
        }));
        assert!(graph.edges.iter().any(|edge| {
            edge.kind == GraphEdgeKind::Concerns
                && edge.from == finding.id
                && graph
                    .node(&edge.to)
                    .is_some_and(|node| node.kind == GraphNodeKind::Entity)
        }));
        assert!(graph.edges.iter().any(|edge| {
            edge.kind == GraphEdgeKind::Concerns
                && edge.from == finding.id
                && graph
                    .node(&edge.to)
                    .is_some_and(|node| node.kind == GraphNodeKind::Topic)
        }));
        assert!(graph.edges.iter().any(|edge| {
            edge.kind == GraphEdgeKind::Supports
                && edge.to == finding.id
                && edge.from == "evidence:call-w1"
        }));
        assert!(graph.node("evidence:call-h1").is_none());
        assert_eq!(
            graph
                .nodes
                .iter()
                .filter(|node| node.kind == GraphNodeKind::Entity && node.label == "Elon Musk")
                .count(),
            1
        );
        let path = recon_path(&graph);
        assert_eq!(path.bands.len(), 1);
        assert_eq!(path.bands[0].directive_id, "d1");
        assert!(path.bands[0].finding.is_some());
        assert!(!path.bands[0].evidence.is_empty());
        let links = force_links(&graph);
        let entity = graph
            .nodes
            .iter()
            .find(|node| node.kind == GraphNodeKind::Entity && node.label == "Elon Musk")
            .unwrap();
        let topic = graph
            .nodes
            .iter()
            .find(|node| node.kind == GraphNodeKind::Topic)
            .unwrap();
        assert!(links.iter().any(|link| {
            link.directive == "d1"
                && ((link.from == entity.id && link.to == topic.id)
                    || (link.from == topic.id && link.to == entity.id))
        }));
        assert!(links.iter().any(|link| {
            link.directive == "d1" && (link.from == finding.id || link.to == finding.id)
        }));
        assert!(!links.iter().any(|link| link.directive == "d3"));
        let brief = graph_brief(&graph);
        assert!(brief.contains("who is Elon Musk?"));
        assert!(brief.contains("Establish identity and public roles"));
        assert!(!brief.contains("Find official accounts"));
        assert!(!brief.contains("Find affiliated orgs"));
        assert!(brief.contains("Recon path"));
    }

    #[test]
    fn path_keeps_the_directive_the_insight_rests_on_when_one_call_serves_several() {
        let (mut insight, mut plan) = sample();
        insight.topic = "accounts".into();
        insight.predicate = "owns".into();
        insight.object_value = "x.com".into();
        plan.calls[0].reason = "d1, d2, d3".into();
        let graph = build_memory_graph(&insight, &[plan]);
        let path = recon_path(&graph);
        assert_eq!(path.bands.len(), 1);
        assert_eq!(path.bands[0].directive_id, "d2");
        assert_eq!(path.bands[0].finding.as_deref(), Some("owns → x.com"));
        assert!(path.bands[0]
            .subjects
            .iter()
            .any(|subject| subject == "Elon Musk"));
    }
}
