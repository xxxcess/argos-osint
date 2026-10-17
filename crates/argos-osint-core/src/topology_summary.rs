//! Scope summary for the topology mapper: pure, deterministic and offline.
//!
//! The summary explains how the nodes in the *current scope* connect, and which
//! nodes are in scope. "Current scope" is the accepted snapshot after the six
//! kind filters, with depth and cycle scope already applied by the snapshot
//! query. Nothing here touches the database, the network or a model, so it is
//! safe to compute on snapshot accept and is the no-model fallback.
//!
//! Identity of a summary:
//! * [`ScopeSpec::key`]: seed + depth + cycle scope + filters. Recentering
//!   changes the seed, so it is a different scope.
//! * [`graph_version`]: hash of the visible nodes and edges (keys, revisions,
//!   kinds, status). Selection, pan, zoom and layout never enter it.
//! * [`digest`]: scope key + graph version + mode + provider + model + prompt
//!   version. A saved summary is reused only for an identical digest.

use std::collections::{BTreeMap, BTreeSet};

use crate::brain_topology::{
    CycleScope, TopologyEdge, TopologyEdgeKind, TopologyNode, TopologyNodeKind, TopologySnapshot,
};

/// Prompt version of the AI narrative (bump to invalidate saved AI text).
pub const PROMPT_VERSION: &str = "topology-scope-summary-v1";
/// Digest mode of the model-written narrative.
pub const MODE_AI: &str = "ai";

/// The six kind filters of the topology controls, in control order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScopeFilters {
    pub actor: bool,
    pub claim: bool,
    pub inference: bool,
    pub link: bool,
    pub related: bool,
    pub sources: bool,
}

impl Default for ScopeFilters {
    fn default() -> Self {
        Self::from_array([true; 6])
    }
}

impl ScopeFilters {
    /// `[actor, claim, inference, link, related, sources]`.
    pub fn from_array(f: [bool; 6]) -> Self {
        Self {
            actor: f[0],
            claim: f[1],
            inference: f[2],
            link: f[3],
            related: f[4],
            sources: f[5],
        }
    }

    pub fn node_visible(&self, node: &TopologyNode) -> bool {
        node.is_root
            || match node.kind {
                TopologyNodeKind::Entity | TopologyNodeKind::ActorGroup => self.actor,
                TopologyNodeKind::Claim => self.claim,
                TopologyNodeKind::Inference | TopologyNodeKind::Topic => self.inference,
                TopologyNodeKind::Article | TopologyNodeKind::Source => self.sources,
            }
    }

    pub fn edge_kind_visible(&self, kind: TopologyEdgeKind) -> bool {
        match kind {
            TopologyEdgeKind::Related => self.related,
            TopologyEdgeKind::Link => self.link,
            _ => true,
        }
    }

    fn bits(&self) -> String {
        [
            self.actor,
            self.claim,
            self.inference,
            self.link,
            self.related,
            self.sources,
        ]
        .iter()
        .map(|b| if *b { '1' } else { '0' })
        .collect()
    }
}

/// What the user is looking at, apart from the graph content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScopeSpec {
    pub seed_key: String,
    pub depth: usize,
    pub cycle: CycleScope,
    pub filters: ScopeFilters,
}

impl ScopeSpec {
    /// Stable scope key. Recenter, depth, cycle and filter changes all change it.
    pub fn key(&self) -> String {
        let cycle = match &self.cycle {
            CycleScope::AllRetained => "all".to_string(),
            CycleScope::ThisCycle(id) => format!("cycle:{id}"),
        };
        crate::evidence::content_hash(&format!(
            "scope-v1\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}",
            self.seed_key,
            self.depth,
            cycle,
            self.filters.bits()
        ))
    }

    fn cycle_label(&self) -> &'static str {
        match self.cycle {
            CycleScope::AllRetained => "all retained",
            CycleScope::ThisCycle(_) => "current cycle",
        }
    }
}

/// The visible nodes and edges of a snapshot under `filters`.
pub fn visible_scope<'a>(
    snapshot: &'a TopologySnapshot,
    filters: &ScopeFilters,
) -> (Vec<&'a TopologyNode>, Vec<&'a TopologyEdge>) {
    let nodes: Vec<&TopologyNode> = snapshot
        .nodes
        .iter()
        .filter(|n| filters.node_visible(n))
        .collect();
    let ids: BTreeSet<&str> = nodes.iter().map(|n| n.id.as_str()).collect();
    let edges = snapshot
        .edges
        .iter()
        .filter(|e| {
            filters.edge_kind_visible(e.kind)
                && ids.contains(e.from.as_str())
                && ids.contains(e.to.as_str())
        })
        .collect();
    (nodes, edges)
}

/// Hash of the visible graph content. Order independent; excludes selection,
/// pan, zoom and `memory_changed_seq` (that counter is stored separately so
/// unrelated churn can be throttled instead of invalidating).
pub fn graph_version(snapshot: &TopologySnapshot, filters: &ScopeFilters) -> String {
    let (nodes, edges) = visible_scope(snapshot, filters);
    let mut rows: Vec<String> = Vec::with_capacity(nodes.len() + edges.len() + 1);
    rows.push(format!("root|{}", snapshot.root_id));
    rows.extend(nodes.iter().map(|n| {
        format!(
            "n|{}|{}|{}|{}|{}",
            n.id,
            n.kind.label(),
            n.revision,
            n.status,
            n.evidence_count
        )
    }));
    rows.extend(edges.iter().map(|e| {
        format!(
            "e|{}|{}|{}|{}|{}|{}",
            e.id,
            e.kind.label(),
            e.from,
            e.to,
            e.label,
            e.status
        )
    }));
    rows.sort();
    crate::evidence::content_hash(&rows.join("\n"))
}

/// Cache key of one saved summary.
pub fn digest(
    scope_key: &str,
    graph_version: &str,
    mode: &str,
    provider: &str,
    model: &str,
    prompt_version: &str,
) -> String {
    crate::evidence::content_hash(
        &[
            scope_key,
            graph_version,
            mode,
            provider,
            model,
            prompt_version,
        ]
        .join("\u{1f}"),
    )
}

/// Deterministic summary of one scope.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScopeSummary {
    pub node_count: usize,
    pub edge_count: usize,
    /// Which nodes are in scope: counts per kind and a few named anchors.
    pub scope: Vec<String>,
    /// How they connect, grouped by relationship type and path.
    pub connections: Vec<String>,
    pub caveats: Vec<String>,
}

impl ScopeSummary {
    /// Plain text with `##` headings (renders through the markdown view).
    pub fn to_text(&self) -> String {
        let mut out = Vec::new();
        out.push("## In scope".to_string());
        out.extend(self.scope.iter().cloned());
        out.push(String::new());
        out.push("## Connections".to_string());
        out.extend(self.connections.iter().cloned());
        if !self.caveats.is_empty() {
            out.push(String::new());
            out.push("## Caveats".to_string());
            out.extend(self.caveats.iter().cloned());
        }
        out.join("\n")
    }

    /// One-line caveat text (the folded findings strip).
    pub fn caveat_line(&self) -> String {
        self.caveats.join(" · ")
    }
}

fn clip(text: &str, max: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        flat
    } else {
        let mut s: String = flat.chars().take(max.saturating_sub(1)).collect();
        s.push('…');
        s
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn kind_noun(kind: TopologyNodeKind, n: usize) -> String {
    let (one, many) = match kind {
        TopologyNodeKind::Entity => ("actor", "actors"),
        TopologyNodeKind::ActorGroup => ("actor group", "actor groups"),
        TopologyNodeKind::Claim => ("claim", "claims"),
        TopologyNodeKind::Inference => ("inference", "inferences"),
        TopologyNodeKind::Topic => ("topic", "topics"),
        TopologyNodeKind::Article => ("article", "articles"),
        TopologyNodeKind::Source => ("source", "sources"),
    };
    plural(n, one, many)
}

fn group_title(kind: TopologyEdgeKind) -> &'static str {
    match kind {
        TopologyEdgeKind::Source => "Sourced from",
        TopologyEdgeKind::MentionedIn => "Mentioned in",
        TopologyEdgeKind::Contains => "Contained in",
        TopologyEdgeKind::Concerns => "Concerns",
        TopologyEdgeKind::Premise => "Inference premises",
        TopologyEdgeKind::Link => "Links",
        TopologyEdgeKind::Related => "Related (tentative context)",
    }
}

/// Group display order: provenance first, tentative last.
const GROUP_ORDER: [TopologyEdgeKind; 7] = [
    TopologyEdgeKind::Source,
    TopologyEdgeKind::Contains,
    TopologyEdgeKind::MentionedIn,
    TopologyEdgeKind::Concerns,
    TopologyEdgeKind::Premise,
    TopologyEdgeKind::Link,
    TopologyEdgeKind::Related,
];

const EXAMPLES_PER_GROUP: usize = 2;
const ANCHORS: usize = 3;
const LABEL_MAX: usize = 36;

/// Deterministic scope summary. Same input gives identical output.
pub fn deterministic_summary(snapshot: &TopologySnapshot, spec: &ScopeSpec) -> ScopeSummary {
    let (nodes, edges) = visible_scope(snapshot, &spec.filters);
    let by_id: BTreeMap<&str, &TopologyNode> = nodes.iter().map(|n| (n.id.as_str(), *n)).collect();
    let label = |id: &str| {
        by_id
            .get(id)
            .map(|n| clip(&n.label, LABEL_MAX))
            .unwrap_or_else(|| clip(id, LABEL_MAX))
    };
    let mut degree: BTreeMap<&str, usize> = BTreeMap::new();
    for e in &edges {
        *degree.entry(e.from.as_str()).or_default() += 1;
        *degree.entry(e.to.as_str()).or_default() += 1;
    }

    // Scope: what is in it.
    let mut kinds: BTreeMap<&'static str, (TopologyNodeKind, usize)> = BTreeMap::new();
    for n in &nodes {
        kinds.entry(n.kind.label()).or_insert((n.kind, 0)).1 += 1;
    }
    let root = nodes.iter().find(|n| n.is_root).copied();
    let mut scope = Vec::new();
    scope.push(format!(
        "{} around {} · {} · within {} hop{} · {}",
        plural(nodes.len(), "node", "nodes"),
        root.map(|n| format!(
            "{} \"{}\"",
            n.kind.label().to_lowercase(),
            clip(&n.label, LABEL_MAX)
        ))
        .unwrap_or_else(|| clip(&snapshot.root_label, LABEL_MAX)),
        plural(edges.len(), "relationship", "relationships"),
        spec.depth,
        if spec.depth == 1 { "" } else { "s" },
        spec.cycle_label()
    ));
    if !kinds.is_empty() {
        scope.push(
            kinds
                .values()
                .map(|(kind, n)| kind_noun(*kind, *n))
                .collect::<Vec<_>>()
                .join(", "),
        );
    }
    let mut hubs: Vec<(&&TopologyNode, usize)> = nodes
        .iter()
        .filter(|n| !n.is_root)
        .map(|n| (n, degree.get(n.id.as_str()).copied().unwrap_or(0)))
        .filter(|(_, d)| *d >= 2)
        .collect();
    hubs.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.id.cmp(&b.0.id)));
    if !hubs.is_empty() {
        scope.push(format!(
            "Most connected: {}",
            hubs.iter()
                .take(ANCHORS)
                .map(|(n, d)| format!("\"{}\" ({d})", clip(&n.label, LABEL_MAX)))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let mut evidenced: Vec<&&TopologyNode> =
        nodes.iter().filter(|n| n.evidence_count > 0).collect();
    evidenced.sort_by(|a, b| {
        b.evidence_count
            .cmp(&a.evidence_count)
            .then(a.id.cmp(&b.id))
    });
    if let Some(top) = evidenced.first() {
        scope.push(format!(
            "Best evidenced: \"{}\" ({} evidence item{})",
            clip(&top.label, LABEL_MAX),
            top.evidence_count,
            if top.evidence_count == 1 { "" } else { "s" }
        ));
    }

    // Connections: grouped by relationship type.
    let mut connections = Vec::new();
    for kind in GROUP_ORDER {
        let group: Vec<&&TopologyEdge> = edges.iter().filter(|e| e.kind == kind).collect();
        if group.is_empty() {
            continue;
        }
        let mut sorted = group.clone();
        sorted.sort_by(|a, b| a.id.cmp(&b.id));
        let examples = sorted
            .iter()
            .take(EXAMPLES_PER_GROUP)
            .map(|e| {
                let predicate = if e.label.trim().is_empty() || e.label == kind.label() {
                    String::new()
                } else {
                    format!(" [{}]", clip(&e.label, 24))
                };
                format!("\"{}\" → \"{}\"{predicate}", label(&e.from), label(&e.to))
            })
            .collect::<Vec<_>>()
            .join("; ");
        let more = group.len().saturating_sub(EXAMPLES_PER_GROUP);
        connections.push(format!(
            "- {}: {}, e.g. {examples}{}",
            group_title(kind),
            plural(group.len(), "connection", "connections"),
            if more > 0 {
                format!(" (+{more} more)")
            } else {
                String::new()
            }
        ));
    }
    // Strongest explanatory path: the deepest visible node back to the seed.
    if let Some(deepest) = nodes
        .iter()
        .filter(|n| !n.is_root && n.predecessor.is_some())
        .max_by(|a, b| a.min_hops.cmp(&b.min_hops).then(b.id.cmp(&a.id)))
    {
        let mut path = vec![label(&deepest.id)];
        let mut cursor = deepest.predecessor.clone();
        let mut guard = 0;
        while let Some(prev) = cursor {
            if !by_id.contains_key(prev.as_str()) || guard > 8 {
                break;
            }
            path.push(label(&prev));
            cursor = by_id[prev.as_str()].predecessor.clone();
            guard += 1;
        }
        if path.len() > 1 {
            path.reverse();
            connections.push(format!(
                "- Longest path from the seed ({} hops): {}",
                deepest.min_hops,
                path.iter()
                    .map(|p| format!("\"{p}\""))
                    .collect::<Vec<_>>()
                    .join(" → ")
            ));
        }
    }
    let isolated: Vec<&&TopologyNode> = nodes
        .iter()
        .filter(|n| !n.is_root && degree.get(n.id.as_str()).copied().unwrap_or(0) == 0)
        .collect();
    let single: usize = nodes
        .iter()
        .filter(|n| !n.is_root && degree.get(n.id.as_str()).copied().unwrap_or(0) == 1)
        .count();
    if !isolated.is_empty() {
        connections.push(format!(
            "- {} not connected to any other node in this view",
            plural(isolated.len(), "node is", "nodes are")
        ));
    }
    if single > 0 && nodes.len() > 2 {
        connections.push(format!(
            "- {} hang off a single connection (leaf nodes)",
            plural(single, "node", "nodes")
        ));
    }
    if edges.is_empty() {
        connections.push(if nodes.len() <= 1 {
            "No recorded neighbors. The seed stands alone in this scope.".to_string()
        } else {
            "No relationships are visible between these nodes under the current filters."
                .to_string()
        });
    }

    // Caveats (the folded findings strip).
    let mut caveats = Vec::new();
    let hidden = snapshot.nodes.len().saturating_sub(nodes.len());
    if hidden > 0 {
        caveats.push(format!("{hidden} hidden by filters"));
    }
    if snapshot.omitted_nodes > 0 || snapshot.omitted_edges > 0 {
        caveats.push(format!(
            "bounded projection: {} nodes and {} relationships omitted",
            snapshot.omitted_nodes, snapshot.omitted_edges
        ));
    } else if !snapshot.exploration_exhausted {
        caveats.push("exploration was bounded".to_string());
    }
    if !snapshot.unresolved_endpoints.is_empty() {
        caveats.push(format!(
            "{} unresolved endpoint{}",
            snapshot.unresolved_endpoints.len(),
            if snapshot.unresolved_endpoints.len() == 1 {
                ""
            } else {
                "s"
            }
        ));
    }
    if edges.iter().any(|e| e.kind == TopologyEdgeKind::Related) {
        caveats.push("Related links are potential context, not verified".to_string());
    }

    ScopeSummary {
        node_count: nodes.len(),
        edge_count: edges.len(),
        scope,
        connections,
        caveats,
    }
}

#[cfg(test)]
#[path = "topology_summary_tests.rs"]
mod tests;
