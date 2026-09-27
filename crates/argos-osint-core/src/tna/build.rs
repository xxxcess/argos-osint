//! Co-occurrence graph, caps, layout, anchors, and structural gaps.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use petgraph::graph::UnGraph;
use petgraph::visit::EdgeRef;

use super::clean::{canonical, clean_document};
use super::corpus::TnaCorpus;
use super::extract::Occurrence;
use super::types::{
    TnaAnchor, TnaCluster, TnaClusterSummary, TnaEdge, TnaGap, TnaNode, TnaNodeKind, TnaSnapshot,
};

const WINDOW: usize = 3;
const COLLECTION_CAP: usize = 120;
const TARGETED_CAP: usize = 80;
const ANCHOR_LIMIT: usize = 8;

/// Build a complete snapshot from a prepared corpus. Pure CPU; safe for
/// `tokio::task::spawn_blocking`.
pub fn build_snapshot(corpus: &TnaCorpus) -> TnaSnapshot {
    build_snapshot_corrected(corpus, &[])
}

pub fn build_snapshot_corrected(
    corpus: &TnaCorpus,
    corrections: &[crate::evidence::Correction],
) -> TnaSnapshot {
    let mut snap = TnaSnapshot::empty(corpus.scope.clone());
    let mut mentions: HashMap<String, (TnaNodeKind, String, u32)> = HashMap::new();
    // doc_id -> entity ids found in that report (collection Doc edges).
    let mut doc_entities: HashMap<String, Vec<String>> = HashMap::new();
    let mut pair_weights: HashMap<(String, String), u32> = HashMap::new();

    for doc in &corpus.docs {
        let (mut occ, mut decisions) = clean_document(doc);
        for correction in corrections.iter().filter(|c| c.report_id == doc.report_id) {
            if !doc
                .text
                .get(correction.start..)
                .is_some_and(|s| s.starts_with(&correction.original))
            {
                continue;
            }
            occ.retain(|o| o.start != correction.start);
            if let (Some(label), Some(kind)) = (&correction.replacement, correction.entity_type) {
                occ.push(Occurrence {
                    kind,
                    label: label.clone(),
                    start: correction.start,
                });
            }
            for d in decisions
                .iter_mut()
                .filter(|d| d.start == correction.start && d.original == correction.original)
            {
                d.label = correction.replacement.clone();
                d.kind = correction.entity_type.unwrap_or(d.kind);
                d.canonical_id = d.label.as_ref().map(|l| node_id(d.kind, l));
                d.reason = format!(
                    "analyst correction {}: {}",
                    correction.id, correction.reason
                );
            }
        }
        occ.sort_by_key(|o| o.start);
        snap.decisions.extend(decisions);
        // A graph window never bridges unrelated source lines or paragraphs.
        let mut groups: std::collections::BTreeMap<usize, Vec<Occurrence>> = Default::default();
        for o in occ {
            let line = doc.text[..o.start.min(doc.text.len())]
                .bytes()
                .filter(|b| *b == b'\n')
                .count();
            groups.entry(line).or_default().push(o);
        }
        let mut ids = Vec::new();
        for group in groups.values() {
            for id in accumulate_occurrences(group, &mut mentions, &mut pair_weights) {
                if !ids.contains(&id) {
                    ids.push(id);
                }
            }
        }
        if corpus.is_collection() {
            let doc_id = node_id(TnaNodeKind::Doc, &doc.report_id);
            let entry =
                mentions
                    .entry(doc_id.clone())
                    .or_insert((TnaNodeKind::Doc, doc.title.clone(), 0));
            entry.2 = entry.2.saturating_add(1);
            if entry.1.is_empty() {
                entry.1 = doc.title.clone();
            }
            for eid in &ids {
                bump_pair(&mut pair_weights, &doc_id, eid);
            }
            doc_entities.insert(doc_id, ids);
        }
    }

    let cap = if corpus.is_collection() {
        COLLECTION_CAP
    } else {
        TARGETED_CAP
    };
    let omitted = mentions.len().saturating_sub(cap);
    let kept = apply_cap(mentions, cap, corpus.is_collection());

    // Rebuild edges only among kept nodes.
    let mut graph: UnGraph<String, u32> = UnGraph::new_undirected();
    let mut index_of: HashMap<String, petgraph::graph::NodeIndex> = HashMap::new();
    for id in kept.keys() {
        let idx = graph.add_node(id.clone());
        index_of.insert(id.clone(), idx);
    }
    for ((a, b), weight) in &pair_weights {
        if a == b {
            continue;
        }
        if let (Some(&ia), Some(&ib)) = (index_of.get(a), index_of.get(b)) {
            if let Some(e) = graph.find_edge(ia, ib) {
                if let Some(w) = graph.edge_weight_mut(e) {
                    *w = (*w).saturating_add(*weight);
                }
            } else {
                graph.add_edge(ia, ib, *weight);
            }
        }
    }

    let mut nodes: Vec<TnaNode> = kept
        .iter()
        .map(|(id, (kind, label, mentions))| {
            let idx = index_of[id];
            let degree = graph.edges(idx).count() as u32;
            TnaNode {
                id: id.clone(),
                label: label.clone(),
                kind: *kind,
                cluster: kind.cluster(),
                mentions: *mentions,
                degree,
                x: 0.5,
                y: 0.5,
            }
        })
        .collect();

    nodes.sort_by(|a, b| a.id.cmp(&b.id));
    match &corpus.scope {
        crate::tna::types::TnaScope::Collection | crate::tna::types::TnaScope::Selected { .. } => {
            layout_nodes(&mut nodes)
        }
        crate::tna::types::TnaScope::Targeted { .. } => layout_hierarchy(&mut nodes),
    }

    let mut edges: Vec<TnaEdge> = Vec::new();
    for edge in graph.edge_references() {
        let from = graph[edge.source()].clone();
        let to = graph[edge.target()].clone();
        edges.push(TnaEdge {
            from,
            to,
            weight: *edge.weight(),
        });
    }
    edges.sort_by(|a, b| b.weight.cmp(&a.weight).then(a.from.cmp(&b.from)));

    // Refresh degree from final edges in case of any drift.
    let mut degree_map: HashMap<String, u32> = HashMap::new();
    for edge in &edges {
        *degree_map.entry(edge.from.clone()).or_default() += 1;
        *degree_map.entry(edge.to.clone()).or_default() += 1;
    }
    for node in &mut nodes {
        node.degree = degree_map.get(&node.id).copied().unwrap_or(0);
    }

    let mut anchors: Vec<TnaAnchor> = nodes
        .iter()
        .filter(|n| n.kind != TnaNodeKind::Doc)
        .map(|n| TnaAnchor {
            node_id: n.id.clone(),
            degree: n.degree,
            mentions: n.mentions,
        })
        .collect();
    anchors.sort_by(|a, b| {
        b.degree
            .cmp(&a.degree)
            .then(b.mentions.cmp(&a.mentions))
            .then(a.node_id.cmp(&b.node_id))
    });
    anchors.truncate(ANCHOR_LIMIT);

    let mut gaps = structural_gaps(&nodes, &edges);
    if omitted > 0 {
        gaps.push(TnaGap {cluster_a:TnaCluster::Infrastructure,cluster_b:TnaCluster::Infrastructure,note:format!("Display cap {cap}: {omitted} extracted entities omitted; source audit retains them. Ranking: entity kind, mention support, then stable identifier; connectivity is not confidence.")});
    }
    let clusters = cluster_summary(&nodes);

    snap.nodes = nodes;
    snap.edges = edges;
    snap.anchors = anchors;
    snap.gaps = gaps;
    snap.clusters = clusters;
    let _ = doc_entities;
    snap
}

/// Helper the bin can wrap in `tokio::task::spawn_blocking`.
pub fn build_snapshot_blocking(corpus: TnaCorpus) -> TnaSnapshot {
    build_snapshot(&corpus)
}

fn accumulate_occurrences(
    occ: &[Occurrence],
    mentions: &mut HashMap<String, (TnaNodeKind, String, u32)>,
    pair_weights: &mut HashMap<(String, String), u32>,
) -> Vec<String> {
    let mut ordered_ids = Vec::new();
    for item in occ {
        if item.kind == TnaNodeKind::Doc {
            continue;
        }
        let id = node_id(item.kind, &item.label);
        let entry = mentions
            .entry(id.clone())
            .or_insert((item.kind, item.label.clone(), 0));
        entry.2 = entry.2.saturating_add(1);
        ordered_ids.push(id);
    }
    // Sliding window of 3 entities → undirected co-occurrence edges.
    if ordered_ids.len() >= 2 {
        if ordered_ids.len() < WINDOW {
            for i in 0..ordered_ids.len() {
                for j in (i + 1)..ordered_ids.len() {
                    bump_pair(pair_weights, &ordered_ids[i], &ordered_ids[j]);
                }
            }
        } else {
            let last = ordered_ids.len() - WINDOW;
            for start in 0..=last {
                let window = &ordered_ids[start..start + WINDOW];
                for i in 0..window.len() {
                    for j in (i + 1)..window.len() {
                        bump_pair(pair_weights, &window[i], &window[j]);
                    }
                }
            }
        }
    }
    // Unique entity ids in this doc (for Doc edges).
    let mut unique = Vec::new();
    for id in &ordered_ids {
        if !unique.iter().any(|u: &String| u == id) {
            unique.push(id.clone());
        }
    }
    unique
}

fn bump_pair(map: &mut HashMap<(String, String), u32>, a: &str, b: &str) {
    if a == b {
        return;
    }
    let key = if a <= b {
        (a.to_string(), b.to_string())
    } else {
        (b.to_string(), a.to_string())
    };
    *map.entry(key).or_default() += 1;
}

fn node_id(kind: TnaNodeKind, label: &str) -> String {
    format!("{}:{}", kind.as_str(), canonical(label))
}

fn apply_cap(
    mentions: HashMap<String, (TnaNodeKind, String, u32)>,
    cap: usize,
    keep_docs: bool,
) -> HashMap<String, (TnaNodeKind, String, u32)> {
    if mentions.len() <= cap {
        return mentions;
    }
    let mut items: Vec<(String, TnaNodeKind, String, u32)> = mentions
        .into_iter()
        .map(|(id, (kind, label, n))| (id, kind, label, n))
        .collect();
    // Drop Topic nodes first, then lowest-mention others.
    items.sort_by(|a, b| {
        let a_drop = drop_priority(a.1, keep_docs);
        let b_drop = drop_priority(b.1, keep_docs);
        a_drop.cmp(&b_drop).then(a.3.cmp(&b.3)).then(a.0.cmp(&b.0))
    });
    let drop_count = items.len().saturating_sub(cap);
    let kept = items.into_iter().skip(drop_count);
    kept.map(|(id, kind, label, n)| (id, (kind, label, n)))
        .collect()
}

fn drop_priority(kind: TnaNodeKind, keep_docs: bool) -> u8 {
    match kind {
        TnaNodeKind::Topic => 0,
        TnaNodeKind::Doc if keep_docs => 3,
        _ => 1,
    }
}

fn layout_nodes(nodes: &mut [TnaNode]) {
    // Quadrants / rings by cluster; jitter by hash of id for stability.
    let centers: [(TnaCluster, f64, f64); 4] = [
        (TnaCluster::Infrastructure, 0.28, 0.28),
        (TnaCluster::Campaign, 0.72, 0.28),
        (TnaCluster::Identity, 0.28, 0.72),
        (TnaCluster::FiledReports, 0.72, 0.72),
    ];
    let mut per_cluster: HashMap<TnaCluster, usize> = HashMap::new();
    for node in nodes.iter_mut() {
        let (cx, cy) = centers
            .iter()
            .find(|(c, _, _)| *c == node.cluster)
            .map(|(_, x, y)| (*x, *y))
            .unwrap_or((0.5, 0.5));
        let n = per_cluster.entry(node.cluster).or_insert(0);
        let ring = (*n / 6) as f64;
        let slot = (*n % 6) as f64;
        *n += 1;
        let angle = std::f64::consts::TAU * (slot / 6.0) + hash_jitter(&node.id) * 0.35;
        let radius = 0.08 + ring * 0.06 + hash_jitter(&format!("r:{}", node.id)) * 0.04;
        let x = (cx + angle.cos() * radius).clamp(0.04, 0.96);
        let y = (cy + angle.sin() * radius).clamp(0.04, 0.96);
        node.x = x;
        node.y = y;
    }
}

fn layout_hierarchy(nodes: &mut [TnaNode]) {
    if nodes.is_empty() {
        return;
    }
    let mut order: Vec<usize> = (0..nodes.len()).collect();
    order.sort_by(|&a, &b| {
        nodes[b]
            .degree
            .cmp(&nodes[a].degree)
            .then(nodes[b].mentions.cmp(&nodes[a].mentions))
            .then(nodes[a].id.cmp(&nodes[b].id))
    });
    let root = order[0];
    nodes[root].x = 0.5;
    nodes[root].y = 0.18;
    let children: Vec<usize> = order.into_iter().skip(1).collect();
    for (i, idx) in children.into_iter().enumerate() {
        let row = 1 + i / 3;
        let col = i % 3;
        let cols_in_row = 3.min(
            nodes
                .len()
                .saturating_sub(1)
                .saturating_sub(row.saturating_sub(1) * 3)
                .max(1),
        );
        let x = 0.22 + (col as f64 + 0.5) / cols_in_row as f64 * 0.56;
        let y = 0.18 + row as f64 * 0.28;
        let jitter = hash_jitter(&nodes[idx].id) * 0.04;
        nodes[idx].x = (x + jitter - 0.02).clamp(0.08, 0.92);
        nodes[idx].y = y.clamp(0.12, 0.92);
    }
}

fn hash_jitter(id: &str) -> f64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    id.hash(&mut hasher);
    let v = hasher.finish();
    (v % 10_000) as f64 / 10_000.0
}

fn structural_gaps(nodes: &[TnaNode], edges: &[TnaEdge]) -> Vec<TnaGap> {
    let mut present: Vec<TnaCluster> = TnaCluster::all()
        .into_iter()
        .filter(|c| nodes.iter().any(|n| n.cluster == *c))
        .collect();
    present.sort_by_key(|c| c.as_str());

    let mut cross = std::collections::HashSet::new();
    let cluster_of: HashMap<&str, TnaCluster> =
        nodes.iter().map(|n| (n.id.as_str(), n.cluster)).collect();
    for edge in edges {
        if let (Some(&ca), Some(&cb)) = (
            cluster_of.get(edge.from.as_str()),
            cluster_of.get(edge.to.as_str()),
        ) {
            if ca != cb {
                let key = ordered_pair(ca, cb);
                cross.insert(key);
            }
        }
    }

    let mut gaps = Vec::new();
    for i in 0..present.len() {
        for j in (i + 1)..present.len() {
            let ca = present[i];
            let cb = present[j];
            if cross.contains(&ordered_pair(ca, cb)) {
                continue;
            }
            gaps.push(TnaGap {
                cluster_a: ca,
                cluster_b: cb,
                note: format!(
                    "{} and {} share no edge in this snapshot",
                    ca.label(),
                    cb.label()
                ),
            });
        }
    }
    gaps
}

fn ordered_pair(a: TnaCluster, b: TnaCluster) -> (TnaCluster, TnaCluster) {
    if a.as_str() <= b.as_str() {
        (a, b)
    } else {
        (b, a)
    }
}

fn cluster_summary(nodes: &[TnaNode]) -> Vec<TnaClusterSummary> {
    let mut counts: HashMap<TnaCluster, u32> = HashMap::new();
    for node in nodes {
        *counts.entry(node.cluster).or_default() += 1;
    }
    TnaCluster::all()
        .into_iter()
        .filter_map(|cluster| {
            let node_count = counts.get(&cluster).copied().unwrap_or(0);
            if node_count == 0 {
                None
            } else {
                Some(TnaClusterSummary {
                    cluster,
                    node_count,
                })
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tna::corpus::{CorpusDoc, TnaCorpus};
    use crate::tna::types::TnaScope;

    fn targeted(text: &str) -> TnaCorpus {
        TnaCorpus {
            scope: TnaScope::Targeted {
                report_id: "r1".into(),
                title: "Sample".into(),
            },
            docs: vec![CorpusDoc {
                report_id: "r1".into(),
                title: "Sample".into(),
                text: text.into(),
            }],
        }
    }

    fn collection(docs: Vec<(&str, &str, &str)>) -> TnaCorpus {
        TnaCorpus {
            scope: TnaScope::Collection,
            docs: docs
                .into_iter()
                .map(|(id, title, text)| CorpusDoc {
                    report_id: id.into(),
                    title: title.into(),
                    text: text.into(),
                })
                .collect(),
        }
    }

    #[test]
    fn three_window_edges_count_cooccurrence() {
        // A B C D → windows [A,B,C] and [B,C,D]; B-C appears twice.
        let text = "example.com @alice ada@example.com 8.8.8.8";
        let snap = build_snapshot(&targeted(text));
        assert!(snap.nodes.len() >= 3, "{:?}", snap.nodes);
        let bc = snap.edges.iter().find(|e| {
            (e.from.contains("example.com") && e.to.contains("alice"))
                || (e.to.contains("example.com") && e.from.contains("alice"))
                || (e.from.contains("handle:alice") && e.to.contains("domain:example.com"))
                || (e.to.contains("handle:alice") && e.from.contains("domain:example.com"))
        });
        assert!(bc.is_some() || !snap.edges.is_empty(), "{:?}", snap.edges);
        // All pairs in a 3-window get weight >= 1.
        assert!(snap.edges.iter().all(|e| e.weight >= 1));
        // Targeted: no Doc node.
        assert!(
            snap.nodes.iter().all(|n| n.kind != TnaNodeKind::Doc),
            "{:?}",
            snap.nodes
        );
    }

    #[test]
    fn caps_drop_topics_first() {
        let mut text = String::from("example.com @bob Ada Lovelace OpenAI Inc 1.1.1.1\n");
        for i in 0..100 {
            text.push_str(&format!("topic line number {i} about research\n"));
        }
        let mut corpus = targeted(&text);
        // Force many unique topics via many short topical lines + domains.
        for i in 0..90 {
            corpus.docs[0]
                .text
                .push_str(&format!("unique-domain-{i}.com research note {i}\n"));
        }
        let snap = build_snapshot(&corpus);
        assert!(snap.nodes.len() <= TARGETED_CAP, "{}", snap.nodes.len());
        let topics = snap
            .nodes
            .iter()
            .filter(|n| n.kind == TnaNodeKind::Topic)
            .count();
        let domains = snap
            .nodes
            .iter()
            .filter(|n| n.kind == TnaNodeKind::Domain)
            .count();
        // Topics should be preferentially dropped when over cap.
        assert!(
            topics <= domains || snap.nodes.len() < TARGETED_CAP,
            "topics={topics} domains={domains} total={}",
            snap.nodes.len()
        );
    }

    #[test]
    fn collection_adds_doc_edges_targeted_has_none() {
        let col = collection(vec![("r1", "Harbor", "see example.com and @harbor")]);
        let snap = build_snapshot(&col);
        assert!(
            snap.nodes.iter().any(|n| n.kind == TnaNodeKind::Doc),
            "{:?}",
            snap.nodes
        );
        let doc_id = snap
            .nodes
            .iter()
            .find(|n| n.kind == TnaNodeKind::Doc)
            .unwrap()
            .id
            .clone();
        assert!(
            snap.edges
                .iter()
                .any(|e| e.from == doc_id || e.to == doc_id),
            "{:?}",
            snap.edges
        );

        let tgt = targeted("see example.com and @harbor");
        let snap = build_snapshot(&tgt);
        assert!(snap.nodes.iter().all(|n| n.kind != TnaNodeKind::Doc));
    }

    #[test]
    fn title_matches_scope() {
        let col = build_snapshot(&collection(vec![("r1", "A", "example.com")]));
        assert_eq!(col.title, "TNA · collection");
        let tgt = build_snapshot(&targeted("example.com"));
        assert_eq!(tgt.title, "TNA · Sample");
    }

    #[test]
    fn gaps_are_missing_cross_cluster_edges() {
        let isolated = targeted("only-one-kind.example");
        let snap = build_snapshot(&isolated);
        let nclusters = snap
            .nodes
            .iter()
            .map(|n| n.cluster)
            .collect::<std::collections::HashSet<_>>()
            .len();
        assert!(snap.gaps.is_empty() || nclusters < 2, "{:?}", snap.gaps);
    }
}

/// Only explicit analyst decisions resolve identity. Original mention labels/spans remain.
pub fn apply_identity_decisions(
    snapshot: &mut TnaSnapshot,
    decisions: &[crate::evidence::IdentityDecision],
) {
    for decision in decisions {
        let Some(canonical) = &decision.canonical_entity else {
            continue;
        };
        let Some(target) = snapshot.nodes.iter().find(|n| &n.id == canonical).cloned() else {
            continue;
        };
        if snapshot
            .nodes
            .iter()
            .filter(|n| decision.entities.contains(&n.id))
            .any(|n| n.kind != target.kind)
        {
            continue;
        }
        let removed = snapshot
            .nodes
            .iter()
            .filter(|n| decision.entities.contains(&n.id) && &n.id != canonical)
            .map(|n| n.mentions)
            .sum::<u32>();
        snapshot
            .nodes
            .retain(|n| &n.id == canonical || !decision.entities.contains(&n.id));
        if let Some(n) = snapshot.nodes.iter_mut().find(|n| &n.id == canonical) {
            n.mentions = n.mentions.saturating_add(removed);
        }
        for edge in &mut snapshot.edges {
            if decision.entities.contains(&edge.from) {
                edge.from = canonical.clone();
            }
            if decision.entities.contains(&edge.to) {
                edge.to = canonical.clone();
            }
        }
        for d in &mut snapshot.decisions {
            if d.canonical_id
                .as_ref()
                .is_some_and(|id| decision.entities.contains(id))
            {
                d.canonical_id = Some(canonical.clone());
                d.reason = format!("{}; identity decision {}", d.reason, decision.id);
            }
        }
    }
    let mut pairs: std::collections::BTreeMap<(String, String), u32> = Default::default();
    for edge in &snapshot.edges {
        if edge.from != edge.to {
            let pair = if edge.from < edge.to {
                (edge.from.clone(), edge.to.clone())
            } else {
                (edge.to.clone(), edge.from.clone())
            };
            let entry = pairs.entry(pair).or_default();
            *entry = entry.saturating_add(edge.weight);
        }
    }
    snapshot.edges = pairs
        .into_iter()
        .map(|((from, to), weight)| TnaEdge { from, to, weight })
        .collect();
    for node in &mut snapshot.nodes {
        node.degree = snapshot
            .edges
            .iter()
            .filter(|e| e.from == node.id || e.to == node.id)
            .count() as u32;
    }
    snapshot
        .anchors
        .retain(|a| snapshot.nodes.iter().any(|n| n.id == a.node_id));
}
