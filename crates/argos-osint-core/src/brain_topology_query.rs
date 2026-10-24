//! Bounded, read-only adjacency queries for the shared topology projection.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;

use super::*;
use crate::brain_actors as actors;
use crate::store::Store;

type Adjacency = (TopologyNode, TopologyEdge);

enum ScopedRows<'a> {
    Never,
    Memory(usize),
    Article(usize),
    Mention { kind: usize, run: usize },
    Evidence { source: usize, revision: usize },
    Link(&'a TopologyNode),
}

fn sql_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// Match retained metadata content, URL and publication time, rather than
/// assigning today's article revision to a historical derivation by owner ID.
fn carried_kinds_sql(snapshot: &str) -> String {
    format!("CASE WHEN json_valid({snapshot}.readiness_json) THEN COALESCE(json_extract({snapshot}.readiness_json,'$.carried_kinds'),'[]') ELSE '[]' END")
}

fn source_snapshot_sql(snapshot: &str, kind: &str) -> String {
    let carried = carried_kinds_sql(snapshot);
    format!("COALESCE((SELECT NULLIF(json_extract(topology_carried.value,'$.source_snapshot_id'),'') FROM json_each({carried}) topology_carried WHERE json_extract(topology_carried.value,'$.kind')={kind} LIMIT 1),{snapshot}.id)")
}

fn current_metadata_sql(snapshot: &str, article: &str, kind: &str) -> String {
    let carried = carried_kinds_sql(snapshot);
    let original = format!("CASE WHEN json_valid({snapshot}.inputs_json) THEN json_extract({snapshot}.inputs_json,'$.source_refs') ELSE '[]' END");
    // A scoped publication carries the sibling kind's original retained
    // metadata. Never assign the new kind's source revision to that sibling.
    let refs = format!("COALESCE((SELECT json_extract(topology_carried.value,'$.inputs.source_refs') FROM json_each({carried}) topology_carried WHERE json_extract(topology_carried.value,'$.kind')={kind} LIMIT 1),{original})");
    format!("EXISTS(SELECT 1 FROM json_each({refs}) topology_ref JOIN nlp_artifacts topology_artifact ON topology_artifact.hash=json_extract(topology_ref.value,'$.artifact_hash') WHERE json_extract(topology_ref.value,'$.source_id')='article:'||{article}.article_id AND topology_artifact.text={article}.title||char(10)||{article}.description AND json_extract(topology_ref.value,'$.url')={article}.url AND COALESCE(json_extract(topology_ref.value,'$.published_at'),'')={article}.published_at)")
}

/// New publications write canonical premise IDs in the existing indexed
/// evidence relation. JSON payload/local-ID searches below remain necessary
/// only for historical records without that relational receipt.
pub(super) const INDEXED_PREMISE_INFERENCES_SQL: &str = "SELECT DISTINCT d.memory_id,d.id,e.premise_id FROM insight_derivation_evidence e INDEXED BY insight_evidence_memory JOIN insight_derivations d ON d.id=e.derivation_id WHERE e.memory_id=?1 AND e.premise_id<>'' AND e.premise_id NOT LIKE 'memory:%' AND d.insight_kind='inference' AND d.memory_id IS NOT NULL AND d.status NOT IN ('deleted','retired','rejected') ORDER BY d.memory_id,d.id,e.premise_id";

fn memory_cycle_sql(memory: &str, cycle: &str) -> String {
    let current = current_metadata_sql(
        "topology_snapshot",
        "topology_article",
        "topology_derivation.insight_kind",
    );
    format!("(EXISTS(SELECT 1 FROM insight_derivations topology_derivation JOIN insight_snapshots topology_snapshot ON topology_snapshot.id=topology_derivation.snapshot_id JOIN atlas_articles topology_article ON topology_article.article_id=topology_snapshot.owner_id WHERE topology_derivation.memory_id={memory} AND topology_article.run_id={cycle} AND topology_derivation.status NOT IN ('deleted','retired','rejected') AND {current}) OR EXISTS(SELECT 1 FROM insight_sources topology_source JOIN insight_claims topology_claim ON topology_claim.fingerprint=topology_source.fingerprint WHERE topology_claim.memory_id={memory} AND topology_source.run_id={cycle} AND topology_source.deleted_origin=0) OR EXISTS(SELECT 1 FROM brain_actor_mentions topology_mention WHERE topology_mention.actor_id={memory} AND topology_mention.origin_kind='article' AND topology_mention.origin_scope={cycle}) OR EXISTS(SELECT 1 FROM brain_memory_actor_refs topology_actor_ref JOIN insight_derivations topology_derivation ON topology_derivation.memory_id=topology_actor_ref.memory_id JOIN insight_snapshots topology_snapshot ON topology_snapshot.id=topology_derivation.snapshot_id JOIN atlas_articles topology_article ON topology_article.article_id=topology_snapshot.owner_id WHERE topology_actor_ref.actor_id={memory} AND topology_article.run_id={cycle} AND topology_derivation.status NOT IN ('deleted','retired','rejected') AND {current}) OR EXISTS(SELECT 1 FROM brain_memory_actor_refs topology_actor_ref JOIN insight_claims topology_claim ON topology_claim.memory_id=topology_actor_ref.memory_id JOIN insight_sources topology_source ON topology_source.fingerprint=topology_claim.fingerprint WHERE topology_actor_ref.actor_id={memory} AND topology_source.run_id={cycle} AND topology_source.deleted_origin=0))")
}

pub(crate) fn node(id: String, kind: TopologyNodeKind, label: String) -> TopologyNode {
    TopologyNode {
        id,
        kind,
        detail: label.clone(),
        label,
        subtype: String::new(),
        status: "recorded".into(),
        is_root: false,
        article_id: String::new(),
        run_id: String::new(),
        published_at: String::new(),
        cycle: None,
        member_ids: Vec::new(),
        evidence_count: 0,
        passages: Vec::new(),
        min_hops: 0,
        revision: String::new(),
        predecessor: None,
        topology_seed: None,
    }
}

pub(crate) fn edge(
    id: String,
    from: &str,
    to: &str,
    kind: TopologyEdgeKind,
    receipt: String,
) -> TopologyEdge {
    TopologyEdge {
        id,
        from: from.into(),
        to: to.into(),
        kind,
        label: match kind {
            TopologyEdgeKind::Concerns => "referenced by",
            TopologyEdgeKind::Premise => "premise for",
            TopologyEdgeKind::Source => "source for",
            _ => kind.label(),
        }
        .into(),
        status: "recorded".into(),
        member_ids: Vec::new(),
        receipts: vec![receipt],
        rationale: String::new(),
    }
}

pub(crate) fn edge_priority(e: &TopologyEdge) -> u8 {
    match e.kind {
        TopologyEdgeKind::Source | TopologyEdgeKind::Premise => 0,
        TopologyEdgeKind::Concerns | TopologyEdgeKind::MentionedIn => 1,
        TopologyEdgeKind::Link => 2,
        TopologyEdgeKind::Contains => 3,
        TopologyEdgeKind::Related => 4,
    }
}

pub(crate) fn record_edge(edges: &mut BTreeMap<String, TopologyEdge>, edge: TopologyEdge) {
    match edges.entry(edge.id.clone()) {
        std::collections::btree_map::Entry::Vacant(slot) => {
            slot.insert(edge);
        }
        std::collections::btree_map::Entry::Occupied(mut slot) => {
            let current = slot.get_mut();
            current.receipts.extend(edge.receipts);
            current.receipts.sort();
            current.receipts.dedup();
            current.member_ids.extend(edge.member_ids);
            current.member_ids.sort();
            current.member_ids.dedup();
        }
    }
}

fn meaningful_topic(label: &str) -> bool {
    !matches!(
        label.trim().to_lowercase().as_str(),
        "" | "unk" | "unknown" | "investigation" | "fact" | "claim" | "inference"
    )
}

pub(crate) fn topic_node(label: &str) -> Option<TopologyNode> {
    if !meaningful_topic(label) {
        return None;
    }
    let label = label.trim().to_string();
    let seed = TopologySeed::Topic(label.clone());
    let mut out = node(seed.key(), TopologyNodeKind::Topic, label);
    out.subtype = "derived_topic_v1".into();
    out.status = "recorded attribute".into();
    out.detail = "Read-only projection of recorded topic/category attributes; not a canonical Brain memory or an assessed relationship.".into();
    out.revision = "topic-v1".into();
    out.topology_seed = Some(seed);
    Some(out)
}

pub(crate) fn source_node(origin: &str, source_id: &str, revision: &str) -> TopologyNode {
    let seed = TopologySeed::Source {
        origin: origin.into(),
        source_id: source_id.into(),
        revision: revision.into(),
    };
    let mut out = node(seed.key(), TopologyNodeKind::Source, source_id.into());
    out.subtype = origin.into();
    out.revision = revision.into();
    out.passages.push(source_id.into());
    out.detail = format!("{origin} · {source_id}\nRevision: {revision}");
    out.topology_seed = Some(seed);
    out
}

impl Store {
    /// Read one revision-consistent, source-scoped neighbourhood. All query
    /// branches and exploration have hard bounds; omitted counts are discovered
    /// omissions, not guesses about unvisited database rows.
    pub fn topology_snapshot(
        &self,
        seed: &TopologySeed,
        options: &TopologyOptions,
    ) -> Result<TopologySnapshot> {
        crate::storage::brain_store::topology_snapshot(seed, options)
    }
}

/// Admit discovered nodes with their explanatory paths into `snapshot`
/// under the node/edge budgets, and record findings, evidence counts and
/// the store-independent associations (actors, threads). Shared by the
/// SQLite query and `storage::topology`; callers add claim and article
/// associations from their own store.
pub(crate) fn admit(
    snapshot: &mut TopologySnapshot,
    nodes: BTreeMap<String, TopologyNode>,
    edges: BTreeMap<String, TopologyEdge>,
    options: &TopologyOptions,
    exhausted: bool,
    gaps: BTreeSet<String>,
) {
    snapshot.total_nodes = nodes.len();
    snapshot.total_edges = edges.len();
    snapshot.discovered_nodes = nodes.len();
    snapshot.discovered_edges = edges.len();
    let mut ordered: Vec<_> = nodes.into_values().collect();
    ordered.sort_by(|a, b| {
        b.is_root
            .cmp(&a.is_root)
            .then_with(|| a.min_hops.cmp(&b.min_hops))
            .then_with(|| a.predecessor.cmp(&b.predecessor))
            .then_with(|| a.kind.label().cmp(b.kind.label()))
            .then_with(|| a.id.cmp(&b.id))
    });
    let mut kept = BTreeSet::new();
    let mut chosen_edges = BTreeMap::new();
    // Admit a node only together with its explanatory path. The seed stays
    // visible even when a caller accidentally supplies a zero-node budget.
    for n in ordered {
        if snapshot.nodes.len() >= options.max_nodes.max(1) {
            continue;
        }
        if let Some(parent) = &n.predecessor {
            if !kept.contains(parent) {
                continue;
            }
            let mut paths: Vec<_> = edges
                .values()
                .filter(|e| {
                    (e.from == n.id && e.to == *parent) || (e.to == n.id && e.from == *parent)
                })
                .collect();
            paths.sort_by(|a, b| {
                edge_priority(a)
                    .cmp(&edge_priority(b))
                    .then_with(|| a.id.cmp(&b.id))
            });
            let Some(path) = paths.first() else {
                continue;
            };
            if chosen_edges.len() >= options.max_edges {
                continue;
            }
            chosen_edges.insert(path.id.clone(), (*path).clone());
        }
        kept.insert(n.id.clone());
        snapshot.nodes.push(n);
    }
    let mut extra: Vec<_> = edges
        .into_values()
        .filter(|e| kept.contains(&e.from) && kept.contains(&e.to))
        .collect();
    extra.sort_by(|a, b| {
        edge_priority(a)
            .cmp(&edge_priority(b))
            .then_with(|| a.id.cmp(&b.id))
    });
    for e in extra {
        if chosen_edges.len() >= options.max_edges {
            break;
        }
        chosen_edges.entry(e.id.clone()).or_insert(e);
    }
    snapshot.edges = chosen_edges.into_values().collect();
    snapshot.omitted_nodes = snapshot
        .discovered_nodes
        .saturating_sub(snapshot.nodes.len());
    snapshot.omitted_edges = snapshot
        .discovered_edges
        .saturating_sub(snapshot.edges.len());
    snapshot.exploration_exhausted = exhausted;
    snapshot.unresolved_endpoints = gaps.into_iter().collect();
    snapshot.findings.gaps = snapshot.unresolved_endpoints.clone();
    if snapshot.omitted_nodes > 0 || snapshot.omitted_edges > 0 || !snapshot.exploration_exhausted {
        snapshot.findings.gaps.push("Bounded projection: some relationships are omitted; increase the bounded limit to expand. No continuation cursor is available.".into());
    }
    if snapshot.nodes.len() == 1 && snapshot.edges.is_empty() {
        snapshot
            .findings
            .coverage
            .push("No indexed neighbours in the selected scope.".into());
    }
    snapshot.findings.coverage.push(format!(
        "{} nodes · {} relationships visible · {} discovered candidates{}",
        snapshot.nodes.len(),
        snapshot.edges.len(),
        snapshot.discovered_nodes,
        if snapshot.exploration_exhausted {
            ""
        } else {
            " · exploration bounded"
        }
    ));
    for e in &snapshot.edges {
        if e.kind == TopologyEdgeKind::Link {
            snapshot
                .findings
                .recorded
                .push(format!("{} · {}", e.id, e.label));
        }
    }
    for n in &mut snapshot.nodes {
        n.evidence_count = snapshot
            .edges
            .iter()
            .filter(|e| e.kind == TopologyEdgeKind::Source && (e.from == n.id || e.to == n.id))
            .count();
        if !n.is_root {
            match n.kind {
                TopologyNodeKind::Entity => {
                    snapshot
                        .associations
                        .actors
                        .push(crate::brain_browse::ActorGroup {
                            name_key: actors::normalize_alias(&n.label),
                            display_name: n.label.clone(),
                            actor_ids: n.member_ids.clone(),
                        })
                }
                TopologyNodeKind::Source => {
                    if let Some(TopologySeed::Source {
                        origin, source_id, ..
                    }) = &n.topology_seed
                    {
                        if origin == "thread" {
                            snapshot
                                .associations
                                .threads
                                .push((source_id.clone(), n.label.clone()));
                        }
                    }
                }
                _ => {}
            }
        }
        if n.kind == TopologyNodeKind::Inference {
            snapshot
                .findings
                .active_inferences
                .push(format!("{} · {}", n.id, n.label));
        }
    }
}
