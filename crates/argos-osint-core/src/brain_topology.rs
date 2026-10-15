//! Shared immutable Brain memory topology queries and projections.
//!
//! Provides `Store::topology_snapshot` across Claims, Actors, Articles, Links,
//! and Inferences with bounded depth and continuation.

use std::collections::{BTreeMap, HashSet, VecDeque};

use anyhow::Result;
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};

use crate::brain_actors as actors;
use crate::brain_browse::{ActorGroup, Associations};
use crate::memory_metadata::{EndpointRef, MemoryPayload};
use crate::store::Store;

#[cfg(test)]
#[path = "brain_topology_tests.rs"]
mod tests;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TopologySeed {
    Claim(String),
    Actor(String),
    ActorGroup {
        name_key: String,
        member_ids: Vec<String>,
    },
    Article {
        run_id: String,
        article_id: String,
        revision: String,
        originating_memory: Option<String>,
    },
    Inference(String),
    Link(String),
    Related(String),
}

impl TopologySeed {
    pub fn key(&self) -> String {
        match self {
            Self::Claim(id) => format!("claim:{id}"),
            Self::Actor(id) => format!("actor:{id}"),
            Self::ActorGroup { name_key, .. } => format!("actor-group:{name_key}"),
            Self::Article {
                run_id, article_id, ..
            } => format!("article:{run_id}/{article_id}"),
            Self::Inference(id) => format!("inference:{id}"),
            Self::Link(id) => format!("link:{id}"),
            Self::Related(id) => format!("related:{id}"),
        }
    }

    pub fn display_label(&self) -> String {
        match self {
            Self::Claim(id) => format!("Claim {id}"),
            Self::Actor(id) => format!("Actor {id}"),
            Self::ActorGroup { name_key, .. } => format!("Actor Group {name_key}"),
            Self::Article {
                run_id, article_id, ..
            } => format!("Article {run_id}/{article_id}"),
            Self::Inference(id) => format!("Inference {id}"),
            Self::Link(id) => format!("Link {id}"),
            Self::Related(id) => format!("Related {id}"),
        }
    }
}

/// Render only Claim, Topic, Entity, Article, Source node kinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TopologyNodeKind {
    Entity,
    Claim,
    Topic,
    Article,
    Source,
}

impl TopologyNodeKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Entity => "Entity",
            Self::Claim => "Claim",
            Self::Topic => "Topic",
            Self::Article => "Article",
            Self::Source => "Source",
        }
    }

    pub fn glyph(self) -> &'static str {
        match self {
            Self::Entity => "●",
            Self::Claim => "★",
            Self::Topic => "◆",
            Self::Article => "•",
            Self::Source => "○",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TopologyEdgeKind {
    Link,
    Related,
    Concerns,
    Premise,
    Source,
    MentionedIn,
    Contains,
}

impl TopologyEdgeKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Link => "link",
            Self::Related => "related",
            Self::Concerns => "concerns",
            Self::Premise => "premise",
            Self::Source => "source",
            Self::MentionedIn => "mentioned_in",
            Self::Contains => "contains",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TopologyNode {
    pub id: String,
    pub kind: TopologyNodeKind,
    pub label: String,
    pub detail: String,
    pub subtype: String,
    pub status: String,
    pub is_root: bool,
    pub article_id: String,
    pub run_id: String,
    pub published_at: String,
    pub cycle: Option<String>,
    pub member_ids: Vec<String>,
    pub evidence_count: usize,
    pub passages: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TopologyEdge {
    pub id: String,
    pub from: String,
    pub to: String,
    pub kind: TopologyEdgeKind,
    pub label: String,
    pub status: String,
    pub member_ids: Vec<String>,
    pub receipts: Vec<String>,
    pub rationale: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CycleScope {
    #[default]
    AllRetained,
    ThisCycle(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TopologyOptions {
    pub depth: usize,
    pub cycle_scope: CycleScope,
    pub max_nodes: usize,
    pub max_edges: usize,
    pub continuation_token: Option<String>,
}

impl Default for TopologyOptions {
    fn default() -> Self {
        Self {
            depth: 1,
            cycle_scope: CycleScope::AllRetained,
            max_nodes: 100,
            max_edges: 200,
            continuation_token: None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopeFindings {
    pub recorded: Vec<String>,
    pub coverage: Vec<String>,
    pub gaps: Vec<String>,
    pub active_inferences: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TopologySnapshot {
    pub seed: Option<TopologySeed>,
    pub root_id: String,
    pub root_label: String,
    pub root_kind: Option<TopologyNodeKind>,
    pub nodes: Vec<TopologyNode>,
    pub edges: Vec<TopologyEdge>,
    pub findings: ScopeFindings,
    pub associations: Associations,
    pub total_nodes: usize,
    pub total_edges: usize,
    pub has_continuation: bool,
    pub continuation_token: Option<String>,
    pub memory_changed_seq: u64,
}

impl Store {
    /// Read-only snapshot of the Brain memory topology starting at `seed`.
    pub fn topology_snapshot(
        &self,
        seed: &TopologySeed,
        options: &TopologyOptions,
    ) -> Result<TopologySnapshot> {
        let browse_context = self.browse_context()?;
        let memory_changed_seq: u64 = self
            .conn
            .query_row(
                "SELECT seq FROM sqlite_sequence WHERE name='memories'",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0);

        let mut snapshot = TopologySnapshot {
            seed: Some(seed.clone()),
            memory_changed_seq,
            ..Default::default()
        };

        let mut nodes_map: BTreeMap<String, TopologyNode> = BTreeMap::new();
        let mut edges_set: Vec<TopologyEdge> = Vec::new();
        let mut visited_frontier: HashSet<String> = HashSet::new();
        let mut frontier: VecDeque<(String, usize)> = VecDeque::new();

        let depth_cap = options.depth.clamp(1, 3);

        // 1. Resolve root and immediate seed content
        match seed {
            TopologySeed::Claim(claim_id) => {
                let Some(mem) = self.get_memory(claim_id)? else {
                    return Ok(snapshot);
                };
                let root_node = TopologyNode {
                    id: claim_id.clone(),
                    kind: TopologyNodeKind::Claim,
                    label: mem.text.clone(),
                    detail: mem.text.clone(),
                    subtype: mem.category.clone(),
                    status: "assessed".into(),
                    is_root: true,
                    article_id: String::new(),
                    run_id: String::new(),
                    published_at: mem.created_at.clone(),
                    cycle: None,
                    member_ids: Vec::new(),
                    evidence_count: 0,
                    passages: Vec::new(),
                };
                snapshot.root_id = claim_id.clone();
                snapshot.root_label = mem.text.clone();
                snapshot.root_kind = Some(TopologyNodeKind::Claim);
                snapshot.associations = self.claim_associations(claim_id)?;

                nodes_map.insert(claim_id.clone(), root_node);
                visited_frontier.insert(claim_id.clone());
                frontier.push_back((claim_id.clone(), 0));

                // Direct Claim Passages from ClaimPayload
                if let Ok(Some(MemoryPayload::Claim(p))) = self.memory_payload(claim_id) {
                    if let Some(rn) = nodes_map.get_mut(claim_id) {
                        rn.passages = p.passages;
                    }
                }
            }
            TopologySeed::Actor(actor_id) => {
                let canonical = actors::resolve(&self.conn, actor_id)?;
                let Some(actor) = actors::actor(&self.conn, &canonical)? else {
                    return Ok(snapshot);
                };
                let key = actors::normalize_alias(&actor.display_name);
                let group = self.actor_group(&key)?.unwrap_or_else(|| ActorGroup {
                    name_key: key.clone(),
                    display_name: actor.display_name.clone(),
                    actor_ids: vec![canonical.clone()],
                });
                let node_id = group.key();
                let root_node = TopologyNode {
                    id: node_id.clone(),
                    kind: TopologyNodeKind::Entity,
                    label: group.display_name.clone(),
                    detail: format!("canonical: {canonical}"),
                    subtype: actor.subtype.clone(),
                    status: actor.identity_state.as_str().to_string(),
                    is_root: true,
                    article_id: String::new(),
                    run_id: String::new(),
                    published_at: String::new(),
                    cycle: None,
                    member_ids: group.actor_ids.clone(),
                    evidence_count: 0,
                    passages: Vec::new(),
                };
                snapshot.root_id = node_id.clone();
                snapshot.root_label = group.display_name.clone();
                snapshot.root_kind = Some(TopologyNodeKind::Entity);
                snapshot.associations =
                    self.actor_group_associations_in(&group, &browse_context)?;

                nodes_map.insert(node_id.clone(), root_node);
                visited_frontier.insert(node_id.clone());
                frontier.push_back((node_id.clone(), 0));
            }
            TopologySeed::ActorGroup {
                name_key,
                member_ids: _,
            } => {
                let Some(group) = self.actor_group(name_key)? else {
                    return Ok(snapshot);
                };
                let node_id = group.key();
                let root_node = TopologyNode {
                    id: node_id.clone(),
                    kind: TopologyNodeKind::Entity,
                    label: group.display_name.clone(),
                    detail: format!("{} member actors", group.actor_ids.len()),
                    subtype: "entity_group".into(),
                    status: "resolved".into(),
                    is_root: true,
                    article_id: String::new(),
                    run_id: String::new(),
                    published_at: String::new(),
                    cycle: None,
                    member_ids: group.actor_ids.clone(),
                    evidence_count: 0,
                    passages: Vec::new(),
                };
                snapshot.root_id = node_id.clone();
                snapshot.root_label = group.display_name.clone();
                snapshot.root_kind = Some(TopologyNodeKind::Entity);
                snapshot.associations =
                    self.actor_group_associations_in(&group, &browse_context)?;

                nodes_map.insert(node_id.clone(), root_node);
                visited_frontier.insert(node_id.clone());
                frontier.push_back((node_id.clone(), 0));
            }
            TopologySeed::Article {
                run_id,
                article_id,
                revision: _,
                originating_memory: _,
            } => {
                let Some(article) = self.atlas_article(run_id, article_id)? else {
                    return Ok(snapshot);
                };
                let node_id = format!("article:{}/{}", run_id, article_id);
                let root_node = TopologyNode {
                    id: node_id.clone(),
                    kind: TopologyNodeKind::Article,
                    label: article.title.clone(),
                    detail: article.description.clone(),
                    subtype: article.category.clone(),
                    status: "retained".into(),
                    is_root: true,
                    article_id: article.id.clone(),
                    run_id: article.run_id.clone(),
                    published_at: article.published_at.clone(),
                    cycle: Some(run_id.clone()),
                    member_ids: Vec::new(),
                    evidence_count: 0,
                    passages: Vec::new(),
                };
                snapshot.root_id = node_id.clone();
                snapshot.root_label = article.title.clone();
                snapshot.root_kind = Some(TopologyNodeKind::Article);

                nodes_map.insert(node_id.clone(), root_node);
                visited_frontier.insert(node_id.clone());
                frontier.push_back((node_id.clone(), 0));
            }
            TopologySeed::Inference(inf_id) => {
                // Inferences highlight their premise Claims; hidden intermediaries stay in findings
                let mut premise_ids = Vec::new();
                let mut rationale = String::new();
                if let Some(MemoryPayload::Inference(p)) = self.memory_payload(inf_id)? {
                    premise_ids = p.premise_claims;
                    rationale = p.reasoning;
                } else {
                    // Try insight_derivations
                    let mut stmt = self.conn.prepare(
                        "SELECT item_json FROM insight_derivations WHERE memory_id=?1 OR id=?1",
                    )?;
                    if let Ok(item) = stmt.query_row([inf_id], |r| r.get::<_, String>(0)) {
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&item) {
                            if let Some(arr) = v.get("premise_ids").and_then(|a| a.as_array()) {
                                premise_ids = arr
                                    .iter()
                                    .filter_map(|s| s.as_str().map(String::from))
                                    .collect();
                            }
                            if let Some(r) = v.get("reasoning").and_then(|s| s.as_str()) {
                                rationale = r.to_string();
                            }
                        }
                    }
                }

                snapshot.root_id = format!("inference:{inf_id}");
                snapshot.root_label = format!("Inference {inf_id}");
                snapshot.findings.active_inferences.push(format!(
                    "I-{} · {}",
                    inf_id.strip_prefix("inf-").unwrap_or(inf_id),
                    rationale
                ));

                // Add premise claims as root components
                for pid in &premise_ids {
                    if let Ok(Some(mem)) = self.get_memory(pid) {
                        let node = TopologyNode {
                            id: pid.clone(),
                            kind: TopologyNodeKind::Claim,
                            label: mem.text.clone(),
                            detail: mem.text.clone(),
                            subtype: "premise_claim".into(),
                            status: "assessed".into(),
                            is_root: true,
                            article_id: String::new(),
                            run_id: String::new(),
                            published_at: mem.created_at.clone(),
                            cycle: None,
                            member_ids: Vec::new(),
                            evidence_count: 0,
                            passages: Vec::new(),
                        };
                        nodes_map.insert(pid.clone(), node);
                        visited_frontier.insert(pid.clone());
                        frontier.push_back((pid.clone(), 0));
                    }
                }
            }
            TopologySeed::Link(link_id) | TopologySeed::Related(link_id) => {
                let payload = self.memory_payload(link_id)?;
                snapshot.root_id = format!("link:{link_id}");
                snapshot.root_label = format!("Link {link_id}");

                if let Some(MemoryPayload::Link(p)) = payload {
                    let ep_source = self.resolve_endpoint_node(&p.source)?;
                    let ep_target = self.resolve_endpoint_node(&p.target)?;
                    if let (Some(s), Some(t)) = (ep_source, ep_target) {
                        let sid = s.id.clone();
                        let tid = t.id.clone();
                        nodes_map.insert(sid.clone(), s);
                        nodes_map.insert(tid.clone(), t);
                        visited_frontier.insert(sid.clone());
                        visited_frontier.insert(tid.clone());
                        frontier.push_back((sid.clone(), 0));
                        frontier.push_back((tid.clone(), 0));

                        edges_set.push(TopologyEdge {
                            id: link_id.clone(),
                            from: sid,
                            to: tid,
                            kind: TopologyEdgeKind::Link,
                            label: p.relation.clone(),
                            status: "assessed".into(),
                            member_ids: Vec::new(),
                            receipts: vec![link_id.clone()],
                            rationale: p.rationale.clone(),
                        });
                        snapshot
                            .findings
                            .recorded
                            .push(format!("{} {}", link_id, p.relation));
                    }
                } else if let Some(MemoryPayload::Related(p)) = payload {
                    let ep_source = self.resolve_endpoint_node(&p.source)?;
                    let ep_target = self.resolve_endpoint_node(&p.target)?;
                    if let (Some(s), Some(t)) = (ep_source, ep_target) {
                        let sid = s.id.clone();
                        let tid = t.id.clone();
                        nodes_map.insert(sid.clone(), s);
                        nodes_map.insert(tid.clone(), t);
                        visited_frontier.insert(sid.clone());
                        visited_frontier.insert(tid.clone());
                        frontier.push_back((sid.clone(), 0));
                        frontier.push_back((tid.clone(), 0));

                        edges_set.push(TopologyEdge {
                            id: link_id.clone(),
                            from: sid,
                            to: tid,
                            kind: TopologyEdgeKind::Related,
                            label: p.basis.clone(),
                            status: "tentative".into(),
                            member_ids: Vec::new(),
                            receipts: vec![link_id.clone()],
                            rationale: p.basis,
                        });
                        snapshot
                            .findings
                            .gaps
                            .push(format!("{} still needs evidence review", link_id));
                    }
                }
            }
        }

        // 2. Expand topology up to depth_cap
        while let Some((curr_id, curr_depth)) = frontier.pop_front() {
            if curr_depth >= depth_cap {
                continue;
            }

            // A. Expand from Entity / ActorGroup
            if curr_id.starts_with("actor-group:") {
                let name_key = curr_id.trim_start_matches("actor-group:");
                if let Some(group) = self.actor_group(name_key)? {
                    let assoc = self.actor_group_associations_in(&group, &browse_context)?;
                    // Connect Claims
                    for c in assoc.claims {
                        if !self.matches_cycle_filter(c.created_at.as_str(), &options.cycle_scope) {
                            continue;
                        }
                        let cid = c.id.clone();
                        if !nodes_map.contains_key(&cid) {
                            nodes_map.insert(
                                cid.clone(),
                                TopologyNode {
                                    id: cid.clone(),
                                    kind: TopologyNodeKind::Claim,
                                    label: c.text.clone(),
                                    detail: c.text.clone(),
                                    subtype: c.category.clone(),
                                    status: "assessed".into(),
                                    is_root: false,
                                    article_id: String::new(),
                                    run_id: String::new(),
                                    published_at: c.created_at.clone(),
                                    cycle: None,
                                    member_ids: Vec::new(),
                                    evidence_count: 0,
                                    passages: Vec::new(),
                                },
                            );
                        }
                        edges_set.push(TopologyEdge {
                            id: format!("edge:{curr_id}:{cid}"),
                            from: curr_id.clone(),
                            to: cid.clone(),
                            kind: TopologyEdgeKind::Concerns,
                            label: "concerns".into(),
                            status: "assessed".into(),
                            member_ids: group.actor_ids.clone(),
                            receipts: Vec::new(),
                            rationale: String::new(),
                        });
                        if visited_frontier.insert(cid.clone()) {
                            frontier.push_back((cid, curr_depth + 1));
                        }
                    }

                    // Connect Threads
                    for (tid, ttitle) in assoc.threads {
                        let sid = format!("thread:{tid}");
                        if !nodes_map.contains_key(&sid) {
                            nodes_map.insert(
                                sid.clone(),
                                TopologyNode {
                                    id: sid.clone(),
                                    kind: TopologyNodeKind::Source,
                                    label: ttitle.clone(),
                                    detail: format!("Recon Thread {tid}"),
                                    subtype: "thread".into(),
                                    status: "recorded".into(),
                                    is_root: false,
                                    article_id: String::new(),
                                    run_id: String::new(),
                                    published_at: String::new(),
                                    cycle: None,
                                    member_ids: Vec::new(),
                                    evidence_count: 0,
                                    passages: Vec::new(),
                                },
                            );
                        }
                        edges_set.push(TopologyEdge {
                            id: format!("edge:{curr_id}:{sid}"),
                            from: curr_id.clone(),
                            to: sid.clone(),
                            kind: TopologyEdgeKind::Source,
                            label: "source".into(),
                            status: "recorded".into(),
                            member_ids: group.actor_ids.clone(),
                            receipts: vec![tid.clone()],
                            rationale: String::new(),
                        });
                    }

                    // Connect Articles (ingest-eligible)
                    for a in assoc.articles {
                        if !self.matches_cycle_run(&a.article.run_id, &options.cycle_scope) {
                            continue;
                        }
                        let aid = format!("article:{}/{}", a.article.run_id, a.article.id);
                        if !nodes_map.contains_key(&aid) {
                            nodes_map.insert(
                                aid.clone(),
                                TopologyNode {
                                    id: aid.clone(),
                                    kind: TopologyNodeKind::Article,
                                    label: a.article.title.clone(),
                                    detail: a.article.description.clone(),
                                    subtype: a.article.category.clone(),
                                    status: "retained".into(),
                                    is_root: false,
                                    article_id: a.article.id.clone(),
                                    run_id: a.article.run_id.clone(),
                                    published_at: a.article.published_at.clone(),
                                    cycle: Some(a.article.run_id.clone()),
                                    member_ids: Vec::new(),
                                    evidence_count: a.occurrences.len(),
                                    passages: Vec::new(),
                                },
                            );
                        }
                        edges_set.push(TopologyEdge {
                            id: format!("edge:{curr_id}:{aid}"),
                            from: curr_id.clone(),
                            to: aid.clone(),
                            kind: TopologyEdgeKind::MentionedIn,
                            label: "mentioned_in".into(),
                            status: "retained".into(),
                            member_ids: group.actor_ids.clone(),
                            receipts: a
                                .occurrences
                                .iter()
                                .map(|(r, i, _)| format!("{r}/{i}"))
                                .collect(),
                            rationale: String::new(),
                        });
                        if visited_frontier.insert(aid.clone()) {
                            frontier.push_back((aid, curr_depth + 1));
                        }
                    }
                }
            } else if nodes_map
                .get(&curr_id)
                .is_some_and(|n| n.kind == TopologyNodeKind::Claim)
            {
                // B. Expand from Claim
                let assoc = self.claim_associations(&curr_id)?;
                for g in assoc.actors {
                    let gid = g.key();
                    if !nodes_map.contains_key(&gid) {
                        nodes_map.insert(
                            gid.clone(),
                            TopologyNode {
                                id: gid.clone(),
                                kind: TopologyNodeKind::Entity,
                                label: g.display_name.clone(),
                                detail: format!("{} member actors", g.actor_ids.len()),
                                subtype: "entity".into(),
                                status: "resolved".into(),
                                is_root: false,
                                article_id: String::new(),
                                run_id: String::new(),
                                published_at: String::new(),
                                cycle: None,
                                member_ids: g.actor_ids.clone(),
                                evidence_count: 0,
                                passages: Vec::new(),
                            },
                        );
                    }
                    edges_set.push(TopologyEdge {
                        id: format!("edge:{gid}:{curr_id}"),
                        from: gid.clone(),
                        to: curr_id.clone(),
                        kind: TopologyEdgeKind::Concerns,
                        label: "concerns".into(),
                        status: "assessed".into(),
                        member_ids: g.actor_ids.clone(),
                        receipts: Vec::new(),
                        rationale: String::new(),
                    });
                    if visited_frontier.insert(gid.clone()) {
                        frontier.push_back((gid, curr_depth + 1));
                    }
                }

                // Sources: Threads
                for (tid, ttitle) in assoc.threads {
                    let sid = format!("thread:{tid}");
                    if !nodes_map.contains_key(&sid) {
                        nodes_map.insert(
                            sid.clone(),
                            TopologyNode {
                                id: sid.clone(),
                                kind: TopologyNodeKind::Source,
                                label: ttitle.clone(),
                                detail: format!("Thread {tid}"),
                                subtype: "thread".into(),
                                status: "source".into(),
                                is_root: false,
                                article_id: String::new(),
                                run_id: String::new(),
                                published_at: String::new(),
                                cycle: None,
                                member_ids: Vec::new(),
                                evidence_count: 0,
                                passages: Vec::new(),
                            },
                        );
                    }
                    edges_set.push(TopologyEdge {
                        id: format!("edge:{sid}:{curr_id}"),
                        from: sid,
                        to: curr_id.clone(),
                        kind: TopologyEdgeKind::Source,
                        label: "source".into(),
                        status: "source".into(),
                        member_ids: Vec::new(),
                        receipts: vec![tid],
                        rationale: String::new(),
                    });
                }

                // Sources: Articles
                for a in assoc.articles {
                    if !self.matches_cycle_run(&a.article.run_id, &options.cycle_scope) {
                        continue;
                    }
                    let aid = format!("article:{}/{}", a.article.run_id, a.article.id);
                    if !nodes_map.contains_key(&aid) {
                        nodes_map.insert(
                            aid.clone(),
                            TopologyNode {
                                id: aid.clone(),
                                kind: TopologyNodeKind::Article,
                                label: a.article.title.clone(),
                                detail: a.article.description.clone(),
                                subtype: a.article.category.clone(),
                                status: "source".into(),
                                is_root: false,
                                article_id: a.article.id.clone(),
                                run_id: a.article.run_id.clone(),
                                published_at: a.article.published_at.clone(),
                                cycle: Some(a.article.run_id.clone()),
                                member_ids: Vec::new(),
                                evidence_count: a.occurrences.len(),
                                passages: Vec::new(),
                            },
                        );
                    }
                    edges_set.push(TopologyEdge {
                        id: format!("edge:{aid}:{curr_id}"),
                        from: aid.clone(),
                        to: curr_id.clone(),
                        kind: TopologyEdgeKind::Source,
                        label: "source".into(),
                        status: "source".into(),
                        member_ids: Vec::new(),
                        receipts: a
                            .occurrences
                            .iter()
                            .map(|(r, i, _)| format!("{r}/{i}"))
                            .collect(),
                        rationale: String::new(),
                    });
                    if visited_frontier.insert(aid.clone()) {
                        frontier.push_back((aid, curr_depth + 1));
                    }
                }

                // Query assessed Links involving this claim
                let mut stmt = self.conn.prepare(
                    "SELECT mm.memory_id, s.ref_id, t.ref_id, s.endpoint_kind, t.endpoint_kind
                     FROM brain_memory_endpoints s
                     JOIN brain_memory_endpoints t ON t.memory_id=s.memory_id AND t.side='target'
                     JOIN memory_metadata mm ON mm.memory_id=s.memory_id AND mm.memory_kind='link' AND mm.lifecycle='active'
                     WHERE s.side='source' AND ((s.endpoint_kind='memory' AND s.ref_id=?1) OR (t.endpoint_kind='memory' AND t.ref_id=?1))",
                )?;
                let links = stmt
                    .query_map([&curr_id], |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, String>(2)?,
                            r.get::<_, String>(3)?,
                            r.get::<_, String>(4)?,
                        ))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;

                for (link_id, s_ref, t_ref, s_kind, t_kind) in links {
                    let relation = match self.memory_payload(&link_id)? {
                        Some(MemoryPayload::Link(p)) => p.relation,
                        _ => "supports".into(),
                    };
                    let from_id = if s_kind == "memory" {
                        s_ref.clone()
                    } else {
                        format!("{s_kind}:{s_ref}")
                    };
                    let to_id = if t_kind == "memory" {
                        t_ref.clone()
                    } else {
                        format!("{t_kind}:{t_ref}")
                    };

                    let other_id = if from_id == curr_id { &to_id } else { &from_id };
                    if !nodes_map.contains_key(other_id) {
                        if let Ok(Some(omem)) = self.get_memory(other_id) {
                            nodes_map.insert(
                                other_id.clone(),
                                TopologyNode {
                                    id: other_id.clone(),
                                    kind: TopologyNodeKind::Claim,
                                    label: omem.text.clone(),
                                    detail: omem.text.clone(),
                                    subtype: omem.category.clone(),
                                    status: "assessed".into(),
                                    is_root: false,
                                    article_id: String::new(),
                                    run_id: String::new(),
                                    published_at: omem.created_at.clone(),
                                    cycle: None,
                                    member_ids: Vec::new(),
                                    evidence_count: 0,
                                    passages: Vec::new(),
                                },
                            );
                            if visited_frontier.insert(other_id.clone()) {
                                frontier.push_back((other_id.clone(), curr_depth + 1));
                            }
                        }
                    }

                    edges_set.push(TopologyEdge {
                        id: link_id.clone(),
                        from: from_id,
                        to: to_id,
                        kind: TopologyEdgeKind::Link,
                        label: relation.clone(),
                        status: "assessed".into(),
                        member_ids: Vec::new(),
                        receipts: vec![link_id.clone()],
                        rationale: String::new(),
                    });
                    snapshot
                        .findings
                        .recorded
                        .push(format!("{} {}", link_id, relation));
                }
            } else if nodes_map
                .get(&curr_id)
                .is_some_and(|n| n.kind == TopologyNodeKind::Article)
            {
                // C. Expand from Article
                let parts: Vec<&str> = curr_id
                    .strip_prefix("article:")
                    .unwrap_or(&curr_id)
                    .split('/')
                    .collect();
                if parts.len() == 2 {
                    let run = parts[0];
                    let id = parts[1];
                    // Mentions -> Actor groups
                    if let Some(names) = browse_context.ingest.get(&(run.into(), id.into())) {
                        for name in names {
                            if let Some(group) = self.actor_group(name)? {
                                let gid = group.key();
                                if !nodes_map.contains_key(&gid) {
                                    nodes_map.insert(
                                        gid.clone(),
                                        TopologyNode {
                                            id: gid.clone(),
                                            kind: TopologyNodeKind::Entity,
                                            label: group.display_name.clone(),
                                            detail: format!(
                                                "{} member actors",
                                                group.actor_ids.len()
                                            ),
                                            subtype: "entity".into(),
                                            status: "resolved".into(),
                                            is_root: false,
                                            article_id: String::new(),
                                            run_id: String::new(),
                                            published_at: String::new(),
                                            cycle: None,
                                            member_ids: group.actor_ids.clone(),
                                            evidence_count: 0,
                                            passages: Vec::new(),
                                        },
                                    );
                                }
                                edges_set.push(TopologyEdge {
                                    id: format!("edge:{gid}:{curr_id}"),
                                    from: gid.clone(),
                                    to: curr_id.clone(),
                                    kind: TopologyEdgeKind::MentionedIn,
                                    label: "mentioned_in".into(),
                                    status: "retained".into(),
                                    member_ids: group.actor_ids.clone(),
                                    receipts: vec![format!("{run}/{id}")],
                                    rationale: String::new(),
                                });
                                if visited_frontier.insert(gid.clone()) {
                                    frontier.push_back((gid, curr_depth + 1));
                                }
                            }
                        }
                    }

                    // Tentative Related articles from atlas_related_articles
                    let mut stmt = self.conn.prepare(
                        "SELECT right_id, right_revision, shared_json FROM atlas_related_articles
                         WHERE run_id=?1 AND left_id=?2
                         UNION
                         SELECT left_id, left_revision, shared_json FROM atlas_related_articles
                         WHERE run_id=?1 AND right_id=?2
                         LIMIT 20",
                    )?;
                    let related_rows = stmt
                        .query_map([run, id], |r| {
                            Ok((
                                r.get::<_, String>(0)?,
                                r.get::<_, String>(1)?,
                                r.get::<_, String>(2)?,
                            ))
                        })?
                        .collect::<rusqlite::Result<Vec<_>>>()?;

                    for (other_id, _rev, shared_json) in related_rows {
                        let other_key = format!("article:{run}/{other_id}");
                        if !nodes_map.contains_key(&other_key) {
                            if let Some(art) = self.atlas_article(run, &other_id)? {
                                nodes_map.insert(
                                    other_key.clone(),
                                    TopologyNode {
                                        id: other_key.clone(),
                                        kind: TopologyNodeKind::Article,
                                        label: art.title.clone(),
                                        detail: art.description.clone(),
                                        subtype: art.category.clone(),
                                        status: "related_lead".into(),
                                        is_root: false,
                                        article_id: art.id.clone(),
                                        run_id: art.run_id.clone(),
                                        published_at: art.published_at.clone(),
                                        cycle: Some(art.run_id.clone()),
                                        member_ids: Vec::new(),
                                        evidence_count: 0,
                                        passages: Vec::new(),
                                    },
                                );
                            }
                        }
                        edges_set.push(TopologyEdge {
                            id: format!("related:{curr_id}:{other_key}"),
                            from: curr_id.clone(),
                            to: other_key,
                            kind: TopologyEdgeKind::Related,
                            label: "possible context".into(),
                            status: "tentative".into(),
                            member_ids: Vec::new(),
                            receipts: vec![shared_json],
                            rationale: "shared entities".into(),
                        });
                    }
                }
            }
        }

        // Deduplicate edges and assign stable sort
        edges_set.sort_by(|a, b| a.id.cmp(&b.id));
        edges_set.dedup_by(|a, b| a.id == b.id);

        let total_nodes = nodes_map.len();
        let total_edges = edges_set.len();
        snapshot.total_nodes = total_nodes;
        snapshot.total_edges = total_edges;

        // Bounded output
        let mut final_nodes: Vec<TopologyNode> = nodes_map.into_values().collect();
        // Keep root first, then sort by kind and label
        final_nodes.sort_by(|a, b| {
            b.is_root
                .cmp(&a.is_root)
                .then_with(|| a.kind.label().cmp(b.kind.label()))
                .then_with(|| a.label.cmp(&b.label))
        });

        if final_nodes.len() > options.max_nodes {
            final_nodes.truncate(options.max_nodes);
            snapshot.has_continuation = true;
            snapshot.continuation_token = Some(format!("nodes:{}", options.max_nodes));
        }
        if edges_set.len() > options.max_edges {
            edges_set.truncate(options.max_edges);
            snapshot.has_continuation = true;
        }

        snapshot.nodes = final_nodes;
        snapshot.edges = edges_set;

        // Populate scope coverage summary
        let source_count = snapshot
            .nodes
            .iter()
            .filter(|n| matches!(n.kind, TopologyNodeKind::Article | TopologyNodeKind::Source))
            .count();
        snapshot.findings.coverage.push(format!(
            "original sources: {} references across retained cycles",
            source_count
        ));

        // Deduplicate recorded and gaps
        snapshot.findings.recorded.sort();
        snapshot.findings.recorded.dedup();
        snapshot.findings.gaps.sort();
        snapshot.findings.gaps.dedup();

        Ok(snapshot)
    }

    fn resolve_endpoint_node(&self, ep: &EndpointRef) -> Result<Option<TopologyNode>> {
        match ep {
            EndpointRef::Memory { memory_id, .. } => {
                let kind: Option<String> = self
                    .conn
                    .query_row(
                        "SELECT memory_kind FROM memory_metadata WHERE memory_id=?1",
                        [memory_id],
                        |r| r.get(0),
                    )
                    .optional()?;
                match kind.as_deref() {
                    Some("actor") => {
                        let canonical = actors::resolve(&self.conn, memory_id)?;
                        let actor = actors::actor(&self.conn, &canonical)?;
                        if let Some(actor) = actor {
                            let key = actors::normalize_alias(&actor.display_name);
                            let group = self.actor_group(&key)?.unwrap_or_else(|| ActorGroup {
                                name_key: key,
                                display_name: actor.display_name.clone(),
                                actor_ids: vec![canonical.clone()],
                            });
                            Ok(Some(TopologyNode {
                                id: group.key(),
                                kind: TopologyNodeKind::Entity,
                                label: group.display_name,
                                detail: format!("canonical: {canonical}"),
                                subtype: actor.subtype,
                                status: actor.identity_state.as_str().to_string(),
                                is_root: false,
                                article_id: String::new(),
                                run_id: String::new(),
                                published_at: String::new(),
                                cycle: None,
                                member_ids: group.actor_ids,
                                evidence_count: 0,
                                passages: Vec::new(),
                            }))
                        } else {
                            Ok(None)
                        }
                    }
                    _ => {
                        if let Some(mem) = self.get_memory(memory_id)? {
                            Ok(Some(TopologyNode {
                                id: mem.id.clone(),
                                kind: TopologyNodeKind::Claim,
                                label: mem.text.clone(),
                                detail: mem.text.clone(),
                                subtype: mem.category.clone(),
                                status: "assessed".into(),
                                is_root: false,
                                article_id: String::new(),
                                run_id: String::new(),
                                published_at: mem.created_at,
                                cycle: None,
                                member_ids: Vec::new(),
                                evidence_count: 0,
                                passages: Vec::new(),
                            }))
                        } else {
                            Ok(None)
                        }
                    }
                }
            }
            EndpointRef::Article {
                run_id, article_id, ..
            } => {
                if let Some(art) = self.atlas_article(run_id, article_id)? {
                    Ok(Some(TopologyNode {
                        id: format!("article:{}/{}", run_id, article_id),
                        kind: TopologyNodeKind::Article,
                        label: art.title.clone(),
                        detail: art.description.clone(),
                        subtype: art.category.clone(),
                        status: "source".into(),
                        is_root: false,
                        article_id: art.id.clone(),
                        run_id: art.run_id.clone(),
                        published_at: art.published_at.clone(),
                        cycle: Some(run_id.clone()),
                        member_ids: Vec::new(),
                        evidence_count: 0,
                        passages: Vec::new(),
                    }))
                } else {
                    Ok(None)
                }
            }
            EndpointRef::Source {
                origin, source_id, ..
            } => Ok(Some(TopologyNode {
                id: format!("source:{origin}:{source_id}"),
                kind: TopologyNodeKind::Source,
                label: source_id.clone(),
                detail: format!("{origin} {source_id}"),
                subtype: origin.clone(),
                status: "source".into(),
                is_root: false,
                article_id: String::new(),
                run_id: String::new(),
                published_at: String::new(),
                cycle: None,
                member_ids: Vec::new(),
                evidence_count: 0,
                passages: Vec::new(),
            })),
        }
    }

    fn matches_cycle_filter(&self, created_at: &str, scope: &CycleScope) -> bool {
        match scope {
            CycleScope::AllRetained => true,
            CycleScope::ThisCycle(c) => created_at.contains(c),
        }
    }

    fn matches_cycle_run(&self, run_id: &str, scope: &CycleScope) -> bool {
        match scope {
            CycleScope::AllRetained => true,
            CycleScope::ThisCycle(c) => run_id == c,
        }
    }
}
