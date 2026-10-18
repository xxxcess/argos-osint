//! Read-only neighborhood brief for the topology mapper.
//!
//! The brief explains the seed's *loaded* neighborhood (the admitted nodes and
//! edges of one topology snapshot): what stands out, how the records connect,
//! what an existing stored inference says, and what is unresolved. It is
//! assembled once, inside the topology read transaction, from typed inputs:
//! Brain payloads (`MemoryPayload`), typed edge kinds and the snapshot's
//! coverage counters. It never parses labels, details or id strings for
//! meaning, never writes, and never calls a model or provider.
//!
//! Format (120–180 words, fewer for sparse neighborhoods; inapplicable blocks
//! are omitted, with no placeholders): Overview, Connections (at most two),
//! Interpretation (at most one), Limits (one line). Items carry typed basis
//! references (node and edge ids) that the UI opens in Record.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::brain_topology::{
    CycleScope, TopologyEdge, TopologyEdgeKind, TopologyNode, TopologyNodeKind, TopologySeed,
    TopologySnapshot,
};
use crate::memory_metadata::MemoryPayload;
use crate::store::Store;

/// Bump when the rendering or selection rules change (part of `basis_key`).
pub const FORMAT_VERSION: u32 = 1;
/// Upper bound on payload reads while hydrating one brief.
pub const MAX_PAYLOAD_READS: usize = 160;
const WORDS_MAX: usize = 180;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Epistemic {
    /// A stored record or a structural fact computed from admitted records.
    Recorded,
    /// An existing inference record, shown as a stored interpretation.
    StoredInference,
    /// A tentative Related lead: potential context, not corroboration.
    Tentative,
    /// A statement about the loaded neighborhood's coverage.
    Coverage,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageStatus {
    Ready,
    Limited,
    Isolated,
    Missing,
}

impl CoverageStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Ready => "complete within bounds",
            Self::Limited => "limited coverage",
            Self::Isolated => "isolated",
            Self::Missing => "seed not found",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BriefCoverage {
    pub status: CoverageStatus,
    pub depth: usize,
    pub cycle_scope: CycleScope,
    pub admitted_nodes: usize,
    pub admitted_edges: usize,
    pub discovered_nodes: usize,
    pub discovered_edges: usize,
    pub omitted_nodes: usize,
    pub omitted_edges: usize,
    pub unresolved_endpoints: usize,
    pub exploration_exhausted: bool,
    /// Payload reads skipped by the hydration budget.
    pub payload_reads_skipped: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BasisTarget {
    Node(String),
    Edge(String),
}

/// One numbered basis reference. `target` is typed; `label` is display only.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BasisRef {
    pub n: usize,
    pub target: BasisTarget,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BriefItem {
    pub text: String,
    pub epistemic: Epistemic,
    pub node_ids: Vec<String>,
    pub edge_ids: Vec<String>,
    /// Actor-group members or Related shared actors (not graph nodes).
    pub member_ids: Vec<String>,
    pub receipts: Vec<String>,
    /// Numbers into [`NeighborhoodBrief::references`].
    pub basis: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NeighborhoodBrief {
    pub format_version: u32,
    pub basis_key: String,
    pub seed: TopologySeed,
    pub seed_label: String,
    pub overview: Option<BriefItem>,
    pub connections: Vec<BriefItem>,
    pub interpretation: Vec<BriefItem>,
    pub limits: Vec<BriefItem>,
    pub references: Vec<BasisRef>,
    pub coverage: BriefCoverage,
}

impl NeighborhoodBrief {
    pub fn header(&self) -> String {
        format!("{} · Neighborhood summary", self.seed_label)
    }

    pub fn footer(&self) -> String {
        let cycle = match self.coverage.cycle_scope {
            CycleScope::AllRetained => "all retained cycles",
            CycleScope::ThisCycle(_) => "this cycle",
        };
        format!(
            "Within {} hop{} · {} · {}",
            self.coverage.depth,
            if self.coverage.depth == 1 { "" } else { "s" },
            cycle,
            self.coverage.status.label()
        )
    }

    /// Items in reading order with their block name.
    pub fn blocks(&self) -> Vec<(&'static str, Vec<&BriefItem>)> {
        let mut out = Vec::new();
        if let Some(o) = &self.overview {
            out.push(("Overview", vec![o]));
        }
        for (name, items) in [
            ("Connections", &self.connections),
            ("Interpretation", &self.interpretation),
            ("Limits", &self.limits),
        ] {
            if !items.is_empty() {
                out.push((name, items.iter().collect()));
            }
        }
        out
    }

    pub fn items(&self) -> impl Iterator<Item = &BriefItem> {
        self.overview
            .iter()
            .chain(&self.connections)
            .chain(&self.interpretation)
            .chain(&self.limits)
    }

    /// Words of all item text (header, footer and references excluded).
    pub fn word_count(&self) -> usize {
        self.items()
            .map(|i| i.text.split_whitespace().count())
            .sum()
    }

    /// Plain-text rendering (tests, docs and logs; the TUI styles it itself).
    pub fn to_text(&self) -> String {
        let mut out = vec![self.header(), String::new()];
        for (name, items) in self.blocks() {
            out.push(name.to_string());
            for i in items {
                let refs = refs_text(&i.basis);
                let bullet = if name == "Connections" { "- " } else { "" };
                out.push(format!("{bullet}{}{refs}", i.text));
            }
            out.push(String::new());
        }
        out.push(self.footer());
        out.join("\n")
    }

    /// Every item id must resolve in `snapshot`, and every basis number must
    /// exist. Returns the problems found (empty when valid).
    pub fn verify(&self, snapshot: &TopologySnapshot) -> Vec<String> {
        let nodes: BTreeSet<&str> = snapshot.nodes.iter().map(|n| n.id.as_str()).collect();
        let edges: BTreeSet<&str> = snapshot.edges.iter().map(|e| e.id.as_str()).collect();
        let mut problems = Vec::new();
        for item in self.items() {
            problems.extend(
                item.node_ids
                    .iter()
                    .filter(|id| !nodes.contains(id.as_str()))
                    .map(|id| format!("node {id} is not in the snapshot")),
            );
            problems.extend(
                item.edge_ids
                    .iter()
                    .filter(|id| !edges.contains(id.as_str()))
                    .map(|id| format!("edge {id} is not in the snapshot")),
            );
            problems.extend(
                item.basis
                    .iter()
                    .filter(|n| **n == 0 || **n > self.references.len())
                    .map(|n| format!("basis {n} does not exist")),
            );
        }
        problems
    }
}

/// `[1]`, `[1–3]` or `[1, 3]`: compact numbered basis references.
pub fn refs_text(basis: &[usize]) -> String {
    let mut ns = basis.to_vec();
    ns.sort_unstable();
    ns.dedup();
    if ns.is_empty() {
        return String::new();
    }
    let mut parts = Vec::new();
    let mut i = 0;
    while i < ns.len() {
        let mut j = i;
        while j + 1 < ns.len() && ns[j + 1] == ns[j] + 1 {
            j += 1;
        }
        if j - i >= 2 {
            parts.push(format!("{}–{}", ns[i], ns[j]));
        } else {
            parts.extend((i..=j).map(|k| ns[k].to_string()));
        }
        i = j + 1;
    }
    format!(" [{}]", parts.join(", "))
}

// ------------------------------------------------------------ typed input

/// Typed inputs hydrated while the topology read transaction is open.
pub struct BriefInput<'a> {
    pub snapshot: &'a TopologySnapshot,
    pub seed: &'a TopologySeed,
    pub depth: usize,
    pub cycle: &'a CycleScope,
    /// Payloads by memory id (admitted Actor/Claim/Inference nodes) and by
    /// edge id (Link/Related edges, whose id is the memory id).
    pub payloads: BTreeMap<String, MemoryPayload>,
    pub payload_reads_skipped: usize,
}

/// Read the payloads of admitted records only (bounded, read-only).
pub fn hydrate<'a>(
    store: &Store,
    snapshot: &'a TopologySnapshot,
    seed: &'a TopologySeed,
    depth: usize,
    cycle: &'a CycleScope,
) -> BriefInput<'a> {
    let mut payloads = BTreeMap::new();
    let mut skipped = 0;
    let mut reads = 0;
    let mut read = |id: &str, payloads: &mut BTreeMap<String, MemoryPayload>| {
        if reads >= MAX_PAYLOAD_READS {
            skipped += 1;
            return;
        }
        reads += 1;
        if let Ok(Some(p)) = store.memory_payload(id) {
            payloads.insert(id.to_string(), p);
        }
    };
    for n in &snapshot.nodes {
        if matches!(
            n.kind,
            TopologyNodeKind::Entity | TopologyNodeKind::Claim | TopologyNodeKind::Inference
        ) {
            read(&n.id, &mut payloads);
        }
    }
    for e in &snapshot.edges {
        if matches!(e.kind, TopologyEdgeKind::Link | TopologyEdgeKind::Related) {
            read(&e.id, &mut payloads);
        }
    }
    BriefInput {
        snapshot,
        seed,
        depth,
        cycle,
        payloads,
        payload_reads_skipped: skipped,
    }
}

// ---------------------------------------------------------------- builder

fn clip_words(text: &str, max: usize) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    if max == 0 {
        String::new()
    } else if words.len() <= max {
        words.join(" ")
    } else {
        format!("{}…", words[..max].join(" "))
    }
}

fn noun(kind: TopologyNodeKind, n: usize) -> String {
    let (one, many) = match kind {
        TopologyNodeKind::Entity => ("actor", "actors"),
        TopologyNodeKind::ActorGroup => ("actor group", "actor groups"),
        TopologyNodeKind::Claim => ("claim", "claims"),
        TopologyNodeKind::Inference => ("inference", "inferences"),
        TopologyNodeKind::Topic => ("topic", "topics"),
        TopologyNodeKind::Article => ("article", "articles"),
        TopologyNodeKind::Source => ("source", "sources"),
    };
    format!("{n} {}", if n == 1 { one } else { many })
}

fn singular(kind: TopologyNodeKind) -> &'static str {
    match kind {
        TopologyNodeKind::Entity => "actor",
        TopologyNodeKind::ActorGroup => "actor group",
        TopologyNodeKind::Claim => "claim",
        TopologyNodeKind::Inference => "inference",
        TopologyNodeKind::Topic => "topic",
        TopologyNodeKind::Article => "article",
        TopologyNodeKind::Source => "source",
    }
}

#[derive(Clone, Copy)]
struct Tight {
    label: usize,
    reasoning: usize,
    limitation: usize,
    rationale: usize,
}

const LEVELS: [Tight; 3] = [
    Tight {
        label: 9,
        reasoning: 18,
        limitation: 14,
        rationale: 12,
    },
    Tight {
        label: 6,
        reasoning: 10,
        limitation: 8,
        rationale: 6,
    },
    Tight {
        label: 4,
        reasoning: 0,
        limitation: 6,
        rationale: 0,
    },
];

struct Pattern {
    weight: i64,
    pinned: bool,
    key: String,
    lead: String,
    item: BriefItem,
}

fn item(
    text: String,
    epistemic: Epistemic,
    node_ids: Vec<String>,
    edge_ids: Vec<String>,
) -> BriefItem {
    BriefItem {
        text,
        epistemic,
        node_ids,
        edge_ids,
        member_ids: Vec::new(),
        receipts: Vec::new(),
        basis: Vec::new(),
    }
}

fn dedup(mut v: Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    v.retain(|x| seen.insert(x.clone()));
    v
}

/// Build the brief for an accepted snapshot. Pure and deterministic: equal
/// inputs give equal text and `basis_key`; no clock, position or selection.
pub fn build(input: &BriefInput) -> NeighborhoodBrief {
    let mut brief = build_at(input, LEVELS[0]);
    for level in &LEVELS[1..] {
        if brief.word_count() <= WORDS_MAX {
            break;
        }
        brief = build_at(input, *level);
    }
    brief
}

/// Brief for a seed that could not be resolved (nothing to explain).
pub fn missing(
    snapshot: &TopologySnapshot,
    seed: &TopologySeed,
    depth: usize,
    cycle: &CycleScope,
) -> NeighborhoodBrief {
    let input = BriefInput {
        snapshot,
        seed,
        depth,
        cycle,
        payloads: BTreeMap::new(),
        payload_reads_skipped: 0,
    };
    build(&input)
}

fn build_at(input: &BriefInput, t: Tight) -> NeighborhoodBrief {
    let snap = input.snapshot;
    let nodes: BTreeMap<&str, &TopologyNode> =
        snap.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    // Edges in id order so equal content gives equal text whatever the input order.
    let mut sorted_edges: Vec<&TopologyEdge> = snap.edges.iter().collect();
    sorted_edges.sort_by(|a, b| a.id.cmp(&b.id));
    let q = |id: &str| -> String {
        nodes
            .get(id)
            .map(|n| format!("“{}”", clip_words(&n.label, t.label)))
            .unwrap_or_else(|| "a record".into())
    };
    let hops = |id: &str| nodes.get(id).map_or(9, |n| n.min_hops) as i64;
    let roots: Vec<&TopologyNode> = snap.nodes.iter().filter(|n| n.is_root).collect();
    let seed_edge_id = match input.seed {
        TopologySeed::Link(id) | TopologySeed::Related(id) => Some(id.as_str()),
        _ => None,
    };
    let status_missing = roots.is_empty();
    let others: Vec<&TopologyNode> = snap.nodes.iter().filter(|n| !n.is_root).collect();
    let status = if status_missing {
        CoverageStatus::Missing
    } else if snap.omitted_nodes > 0
        || snap.omitted_edges > 0
        || !snap.unresolved_endpoints.is_empty()
        || !snap.exploration_exhausted
        || input.payload_reads_skipped > 0
    {
        CoverageStatus::Limited
    } else if snap.edges.is_empty() {
        CoverageStatus::Isolated
    } else {
        CoverageStatus::Ready
    };
    let coverage = BriefCoverage {
        status,
        depth: input.depth,
        cycle_scope: input.cycle.clone(),
        admitted_nodes: snap.nodes.len(),
        admitted_edges: snap.edges.len(),
        discovered_nodes: snap.discovered_nodes,
        discovered_edges: snap.discovered_edges,
        omitted_nodes: snap.omitted_nodes,
        omitted_edges: snap.omitted_edges,
        unresolved_endpoints: snap.unresolved_endpoints.len(),
        exploration_exhausted: snap.exploration_exhausted,
        payload_reads_skipped: input.payload_reads_skipped,
    };
    let seed_label = match (seed_edge_id, roots.as_slice()) {
        (Some(_), [a, b, ..]) => format!(
            "{} → {}",
            clip_words(&a.label, t.label),
            clip_words(&b.label, t.label)
        ),
        _ => roots
            .first()
            .map(|n| clip_words(&n.label, 12))
            .unwrap_or_else(|| clip_words(&input.seed.display_label(), 12)),
    };

    // Typed edge facts.
    let link_relation = |e: &TopologyEdge| -> String {
        match input.payloads.get(&e.id) {
            Some(MemoryPayload::Link(p)) => p.relation.clone(),
            _ => e.label.clone(),
        }
    };
    let claimish = |id: &str| {
        nodes.get(id).is_some_and(|n| {
            matches!(
                n.kind,
                TopologyNodeKind::Claim | TopologyNodeKind::Inference
            )
        })
    };

    // ---- candidate connection patterns
    let mut patterns: Vec<Pattern> = Vec::new();
    for e in sorted_edges
        .iter()
        .copied()
        .filter(|e| e.kind == TopologyEdgeKind::Link)
    {
        let rel = link_relation(e);
        let (weight, pinned) = match rel.trim().to_lowercase().as_str() {
            "contradicts" => (1000, true),
            "supports" | "derived_from" => (800, false),
            "mentioned_in" | "mentions" => (300, false),
            _ => (600, false),
        };
        let rationale = match input.payloads.get(&e.id) {
            Some(MemoryPayload::Link(p)) => clip_words(&p.rationale, t.rationale),
            _ => String::new(),
        };
        let mut text = format!("Recorded link: {} {} {}", q(&e.from), rel.trim(), q(&e.to));
        if !rationale.is_empty() {
            text.push_str(&format!(" — {rationale}"));
        }
        text.push('.');
        let mut it = item(
            text,
            Epistemic::Recorded,
            vec![e.from.clone(), e.to.clone()],
            vec![e.id.clone()],
        );
        it.receipts = e.receipts.clone();
        let bonus = if seed_edge_id == Some(e.id.as_str()) {
            500
        } else {
            0
        };
        patterns.push(Pattern {
            weight: weight + bonus - 5 * (hops(&e.from) + hops(&e.to)),
            pinned,
            key: format!("link:{}", e.id),
            lead: if pinned {
                "a recorded contradiction".into()
            } else {
                "recorded links".into()
            },
            item: it,
        });
    }
    // Actors shared by two or more claims or inferences.
    let mut by_actor: BTreeMap<&str, Vec<(&str, &str)>> = BTreeMap::new();
    for e in sorted_edges
        .iter()
        .copied()
        .filter(|e| e.kind == TopologyEdgeKind::Concerns)
    {
        let (actor, subject) = if claimish(&e.to) && !claimish(&e.from) {
            (e.from.as_str(), e.to.as_str())
        } else if claimish(&e.from) && !claimish(&e.to) {
            (e.to.as_str(), e.from.as_str())
        } else {
            continue;
        };
        if nodes.get(actor).is_some_and(|a| {
            matches!(
                a.kind,
                TopologyNodeKind::Entity | TopologyNodeKind::ActorGroup
            )
        }) {
            by_actor
                .entry(actor)
                .or_default()
                .push((subject, e.id.as_str()));
        }
    }
    for (actor, links) in &by_actor {
        let subjects: BTreeSet<&str> = links.iter().map(|(s, _)| *s).collect();
        if subjects.len() < 2 {
            continue;
        }
        let a = nodes[actor];
        let list: Vec<&str> = subjects.iter().copied().collect();
        let extra = list.len().saturating_sub(2);
        let mut text = format!("{} and {} share actor {}", q(list[0]), q(list[1]), q(actor));
        if extra > 0 {
            text.push_str(&format!(" (+{extra} more)"));
        }
        let mut member_ids = Vec::new();
        if a.kind == TopologyNodeKind::ActorGroup {
            text.push_str("; a name group, members stay unmerged");
            member_ids = a.member_ids.clone();
        } else if matches!(a.status.as_str(), "provisional" | "ambiguous") {
            text.push_str(&format!("; identity {}", a.status));
        }
        text.push('.');
        let mut it = item(
            text,
            Epistemic::Recorded,
            std::iter::once(actor.to_string())
                .chain(list.iter().take(2).map(|s| s.to_string()))
                .collect(),
            links.iter().take(2).map(|(_, e)| e.to_string()).collect(),
        );
        it.member_ids = member_ids;
        patterns.push(Pattern {
            weight: 700 + 10 * subjects.len() as i64
                - 5 * list.iter().map(|s| hops(s)).sum::<i64>(),
            pinned: false,
            key: format!("actor:{actor}"),
            lead: format!("records sharing actor {}", q(actor)),
            item: it,
        });
    }
    // Premise chains behind inferences.
    let mut premises: BTreeMap<&str, Vec<&TopologyEdge>> = BTreeMap::new();
    for e in sorted_edges
        .iter()
        .copied()
        .filter(|e| e.kind == TopologyEdgeKind::Premise)
    {
        if nodes
            .get(e.to.as_str())
            .is_some_and(|n| n.kind == TopologyNodeKind::Inference)
        {
            premises.entry(e.to.as_str()).or_default().push(e);
        }
    }
    // Source paths shared by claims (duplicate occurrences of one article merge).
    let mut by_source: BTreeMap<String, (Vec<&str>, Vec<&TopologyEdge>, &TopologyNode)> =
        BTreeMap::new();
    for e in sorted_edges
        .iter()
        .copied()
        .filter(|e| e.kind == TopologyEdgeKind::Source)
    {
        let (src, subject) = if claimish(&e.to) {
            (e.from.as_str(), e.to.as_str())
        } else {
            continue;
        };
        let Some(s) = nodes.get(src) else { continue };
        if !matches!(s.kind, TopologyNodeKind::Article | TopologyNodeKind::Source) {
            continue;
        }
        let key = if s.kind == TopologyNodeKind::Article && !s.article_id.is_empty() {
            format!("article:{}/{}", s.run_id, s.article_id)
        } else {
            s.id.clone()
        };
        let entry = by_source.entry(key).or_insert((Vec::new(), Vec::new(), *s));
        if !entry.0.contains(&subject) {
            entry.0.push(subject);
        }
        entry.1.push(e);
    }
    for (key, (subjects, edges, s)) in &by_source {
        if subjects.len() < 2 {
            continue;
        }
        let mut it = item(
            format!(
                "{} trace to retained {} {}.",
                noun(TopologyNodeKind::Claim, subjects.len()),
                singular(s.kind),
                q(&s.id)
            ),
            Epistemic::Recorded,
            std::iter::once(s.id.clone())
                .chain(subjects.iter().take(2).map(|x| x.to_string()))
                .collect(),
            edges.iter().take(2).map(|e| e.id.clone()).collect(),
        );
        it.receipts = dedup(edges.iter().flat_map(|e| e.receipts.clone()).collect());
        patterns.push(Pattern {
            weight: 500 + 10 * subjects.len() as i64 - 5 * hops(&s.id),
            pinned: false,
            key: format!("source:{key}"),
            lead: format!(
                "claims tracing to retained {} {}",
                singular(s.kind),
                q(&s.id)
            ),
            item: it,
        });
    }
    if by_source.len() >= 2 {
        let distinct: BTreeSet<&str> = by_source
            .values()
            .flat_map(|(s, _, _)| s.iter().copied())
            .collect();
        if distinct.len() >= 2 {
            let ids: Vec<String> = by_source.values().map(|(_, _, n)| n.id.clone()).collect();
            let edge_ids: Vec<String> = by_source
                .values()
                .flat_map(|(_, e, _)| e.iter().map(|e| e.id.clone()))
                .collect();
            patterns.push(Pattern {
                weight: 450,
                pinned: false,
                key: "sources".into(),
                lead: "claims traced to several retained records".into(),
                item: item(
                    format!(
                        "{} trace to {} retained records, which are not independent confirmations.",
                        noun(TopologyNodeKind::Claim, distinct.len()),
                        by_source.len()
                    ),
                    Epistemic::Recorded,
                    ids.into_iter().take(3).collect(),
                    edge_ids.into_iter().take(3).collect(),
                ),
            });
        }
    }
    // Stored topics shared by claims (membership is not agreement).
    let mut by_topic: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for e in &snap.edges {
        for (t_id, other) in [(&e.from, &e.to), (&e.to, &e.from)] {
            if nodes
                .get(t_id.as_str())
                .is_some_and(|n| n.kind == TopologyNodeKind::Topic)
                && claimish(other)
            {
                by_topic
                    .entry(t_id.as_str())
                    .or_default()
                    .insert(other.as_str());
            }
        }
    }
    for (topic, subjects) in &by_topic {
        if subjects.len() < 2 {
            continue;
        }
        let list: Vec<&str> = subjects.iter().copied().collect();
        patterns.push(Pattern {
            weight: 200 - 5 * hops(topic),
            pinned: false,
            key: format!("topic:{topic}"),
            lead: format!("claims sharing stored topic {}", q(topic)),
            item: item(
                format!(
                    "{} and {} share stored topic {}; topic membership does not establish agreement or a relationship.",
                    q(list[0]),
                    q(list[1]),
                    q(topic)
                ),
                Epistemic::Recorded,
                vec![topic.to_string(), list[0].to_string(), list[1].to_string()],
                Vec::new(),
            ),
        });
    }

    // ---- interpretation: one existing inference, else one tentative lead
    let mut inf_nodes: Vec<&TopologyNode> = snap
        .nodes
        .iter()
        .filter(|n| n.kind == TopologyNodeKind::Inference)
        .collect();
    inf_nodes.sort_by(|a, b| {
        b.is_root
            .cmp(&a.is_root)
            .then_with(|| {
                premises
                    .get(b.id.as_str())
                    .map_or(0, Vec::len)
                    .cmp(&premises.get(a.id.as_str()).map_or(0, Vec::len))
            })
            .then(a.min_hops.cmp(&b.min_hops))
            .then(a.id.cmp(&b.id))
    });
    let mut interpretation = Vec::new();
    let mut chosen_inference: Option<&TopologyNode> = None;
    if let Some(i) = inf_nodes.first() {
        chosen_inference = Some(i);
        let p = match input.payloads.get(&i.id) {
            Some(MemoryPayload::Inference(p)) => Some(p),
            _ => None,
        };
        let state = p
            .map(|p| p.support_state.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| i.status.clone());
        let mut text = format!(
            "Stored inference {} (support state: {state}; a stored interpretation, not a verified conclusion).",
            q(&i.id)
        );
        if let Some(p) = p {
            let r = clip_words(&p.reasoning, t.reasoning);
            if !r.is_empty() {
                text.push_str(&format!(" Reasoning: {}.", r.trim_end_matches('.')));
            }
            let l = clip_words(&p.limitations, t.limitation);
            if !l.is_empty() {
                text.push_str(&format!(" Limitation: {}.", l.trim_end_matches('.')));
            }
        }
        let pe: Vec<&&TopologyEdge> = premises
            .get(i.id.as_str())
            .map(|v| v.iter().take(2).collect())
            .unwrap_or_default();
        let mut node_ids = vec![i.id.clone()];
        node_ids.extend(pe.iter().map(|e| e.from.clone()));
        let mut it = item(
            text,
            Epistemic::StoredInference,
            node_ids,
            pe.iter().map(|e| e.id.clone()).collect(),
        );
        if let Some(p) = p {
            if !p.coverage_receipt.trim().is_empty() {
                it.receipts.push(p.coverage_receipt.clone());
            }
        }
        interpretation.push(it);
    } else {
        let mut related: Vec<&TopologyEdge> = snap
            .edges
            .iter()
            .filter(|e| e.kind == TopologyEdgeKind::Related)
            .collect();
        related.sort_by(|a, b| {
            (seed_edge_id != Some(a.id.as_str()))
                .cmp(&(seed_edge_id != Some(b.id.as_str())))
                .then(a.id.cmp(&b.id))
        });
        if let Some(e) = related.first() {
            if let Some(MemoryPayload::Related(p)) = input.payloads.get(&e.id) {
                let basis = if p.basis.trim().is_empty() && !p.shared_actors.is_empty() {
                    format!("{} shared actors", p.shared_actors.len())
                } else {
                    clip_words(&p.basis, t.reasoning.max(8))
                };
                let gap = clip_words(&p.verification_gap, t.limitation.max(8));
                if !basis.is_empty() || !gap.is_empty() {
                    let mut text = format!(
                        "Tentative lead: {} and {} may be related",
                        q(&e.from),
                        q(&e.to)
                    );
                    if !basis.is_empty() {
                        text.push_str(&format!(" — basis: {}", basis.trim_end_matches('.')));
                    }
                    text.push('.');
                    if !gap.is_empty() {
                        text.push_str(&format!(
                            " Verification gap: {}.",
                            gap.trim_end_matches('.')
                        ));
                    }
                    text.push_str(" Potential context only, not corroboration.");
                    let mut it = item(
                        text,
                        Epistemic::Tentative,
                        vec![e.from.clone(), e.to.clone()],
                        vec![e.id.clone()],
                    );
                    it.member_ids = p.shared_actors.clone();
                    it.receipts = e.receipts.clone();
                    interpretation.push(it);
                }
            }
        }
    }
    // A premise-chain pattern repeats the chosen inference: drop it there.
    for (inf, edges) in &premises {
        if chosen_inference.is_some_and(|c| c.id == *inf) {
            continue;
        }
        let n = nodes[inf];
        let k = edges.len();
        patterns.push(Pattern {
            weight: 650 + 10 * k as i64 - 5 * hops(inf),
            pinned: false,
            key: format!("premise:{inf}"),
            lead: format!("a premise chain behind inference {}", q(inf)),
            item: item(
                format!(
                    "Inference {} takes {} from this neighborhood as {}.",
                    q(&n.id),
                    noun(TopologyNodeKind::Claim, k),
                    if k == 1 { "its premise" } else { "premises" }
                ),
                Epistemic::Recorded,
                std::iter::once(inf.to_string())
                    .chain(edges.iter().take(2).map(|e| e.from.clone()))
                    .collect(),
                edges.iter().take(2).map(|e| e.id.clone()).collect(),
            ),
        });
    }

    // ---- select connections (two): contradictions pinned, then by weight
    patterns.sort_by(|a, b| {
        b.pinned
            .cmp(&a.pinned)
            .then(b.weight.cmp(&a.weight))
            .then(a.key.cmp(&b.key))
    });
    let mut seen_text = BTreeSet::new();
    patterns.retain(|p| seen_text.insert(p.item.text.clone()));
    let lead = patterns.first().map(|p| p.lead.clone());
    let connections: Vec<BriefItem> = patterns.iter().take(2).map(|p| p.item.clone()).collect();

    // ---- overview
    let overview = if status_missing {
        None
    } else if let (Some(id), [a, b, ..]) = (seed_edge_id, roots.as_slice()) {
        let edge = sorted_edges.iter().copied().find(|e| e.id == id);
        let text = match (matches!(input.seed, TopologySeed::Related(_)), edge) {
            (true, _) => format!(
                "A tentative Related lead joins {} and {}: potential context only.",
                q(&a.id),
                q(&b.id)
            ),
            (false, Some(e)) => format!(
                "The recorded link says {} {} {}.",
                q(&e.from),
                link_relation(e).trim(),
                q(&e.to)
            ),
            (false, None) => format!("Link between {} and {}.", q(&a.id), q(&b.id)),
        };
        Some(item(
            text,
            if matches!(input.seed, TopologySeed::Related(_)) {
                Epistemic::Tentative
            } else {
                Epistemic::Recorded
            },
            vec![a.id.clone(), b.id.clone()],
            vec![id.to_string()],
        ))
    } else {
        let root = roots[0];
        let mut counts: BTreeMap<&str, (TopologyNodeKind, usize)> = BTreeMap::new();
        for n in &others {
            counts.entry(n.kind.label()).or_insert((n.kind, 0)).1 += 1;
        }
        let mut kinds: Vec<(TopologyNodeKind, usize)> = counts.into_values().collect();
        kinds.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.label().cmp(b.0.label())));
        let sing = {
            let s = singular(root.kind);
            let mut c = s.chars();
            c.next()
                .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
                .unwrap_or_default()
        };
        let text = if others.is_empty() {
            format!(
                "{sing} {} has no recorded neighbors in the loaded neighborhood.",
                q(&root.id)
            )
        } else {
            let list = kinds
                .iter()
                .take(3)
                .map(|(k, n)| noun(*k, *n))
                .collect::<Vec<_>>()
                .join(", ");
            let mut s = format!(
                "{sing} {} connects to {list} in the loaded neighborhood.",
                q(&root.id)
            );
            if let Some(l) = &lead {
                s.push_str(&format!(" Most informative: {l}."));
            }
            s
        };
        Some(item(
            text,
            Epistemic::Recorded,
            vec![root.id.clone()],
            Vec::new(),
        ))
    };

    // ---- limits: one line, most important first
    let mut limits: Vec<BriefItem> = Vec::new();
    let in_view: BTreeSet<&str> = connections
        .iter()
        .chain(&interpretation)
        .flat_map(|i| i.node_ids.iter().map(String::as_str))
        .collect();
    let identity = snap
        .nodes
        .iter()
        .filter(|n| {
            matches!(
                n.kind,
                TopologyNodeKind::Entity | TopologyNodeKind::ActorGroup
            )
        })
        .find(|n| {
            (n.kind == TopologyNodeKind::ActorGroup
                && (n.is_root || in_view.contains(n.id.as_str())))
                || (in_view.contains(n.id.as_str())
                    && matches!(n.status.as_str(), "provisional" | "ambiguous"))
        });
    let premise_gap = chosen_inference.and_then(|i| match input.payloads.get(&i.id) {
        Some(MemoryPayload::Inference(p)) if !p.premise_claims.is_empty() => {
            let present = p
                .premise_claims
                .iter()
                .filter(|c| nodes.contains_key(c.as_str()))
                .count();
            (present < p.premise_claims.len()).then_some((i, p.premise_claims.len(), present))
        }
        _ => None,
    });
    if let Some(n) = identity {
        let text = if n.kind == TopologyNodeKind::ActorGroup {
            format!(
                "{} is a display group of separate actor bindings; equal names do not establish identity.",
                q(&n.id)
            )
        } else {
            format!(
                "Identity of actor {} is {}; equal names do not establish identity.",
                q(&n.id),
                n.status
            )
        };
        let mut it = item(text, Epistemic::Recorded, vec![n.id.clone()], Vec::new());
        it.member_ids = n.member_ids.clone();
        limits.push(it);
    } else if let Some((i, total, present)) = premise_gap {
        limits.push(item(
            format!(
                "Inference {} lists {total} premise claims; {} outside the loaded neighborhood, so its grounding here is incomplete.",
                q(&i.id),
                if total - present == 1 {
                    "1 is".to_string()
                } else {
                    format!("{} are", total - present)
                }
            ),
            Epistemic::Recorded,
            vec![i.id.clone()],
            Vec::new(),
        ));
    } else if status == CoverageStatus::Limited || status == CoverageStatus::Missing {
        let mut parts = Vec::new();
        if snap.omitted_nodes > 0 || snap.omitted_edges > 0 {
            parts.push(format!(
                "{} node{} and {} relationship{} omitted",
                snap.omitted_nodes,
                if snap.omitted_nodes == 1 { "" } else { "s" },
                snap.omitted_edges,
                if snap.omitted_edges == 1 { "" } else { "s" }
            ));
        }
        if !snap.unresolved_endpoints.is_empty() {
            parts.push(format!(
                "{} unresolved endpoint{}",
                snap.unresolved_endpoints.len(),
                if snap.unresolved_endpoints.len() == 1 {
                    ""
                } else {
                    "s"
                }
            ));
        }
        if parts.is_empty() && !snap.exploration_exhausted {
            parts.push("exploration was bounded".into());
        }
        if input.payload_reads_skipped > 0 {
            parts.push(format!(
                "{} record details were not read",
                input.payload_reads_skipped
            ));
        }
        let text = if status == CoverageStatus::Missing {
            "The seed record is missing, revised, or has no supported typed endpoint.".to_string()
        } else {
            format!(
                "The loaded neighborhood is incomplete ({}); links not shown here are not evidence of absence.",
                parts.join("; ")
            )
        };
        limits.push(item(text, Epistemic::Coverage, Vec::new(), Vec::new()));
    } else if interpretation.is_empty() && !others.is_empty() {
        limits.push(item(
            "No stored inference covers this neighborhood; the connections are structural, not conclusions.".into(),
            Epistemic::Coverage,
            Vec::new(),
            Vec::new(),
        ));
    }
    limits.truncate(1);

    let mut brief = NeighborhoodBrief {
        format_version: FORMAT_VERSION,
        basis_key: String::new(),
        seed: input.seed.clone(),
        seed_label,
        overview,
        connections,
        interpretation,
        limits,
        references: Vec::new(),
        coverage,
    };
    number_references(&mut brief, &nodes, &snap.edges, &q_label(t));
    brief.basis_key = basis_key(input);
    brief
}

fn q_label(t: Tight) -> usize {
    t.label.min(6)
}

/// Number the typed basis references in reading order (nodes, then edges).
pub(crate) fn number_references(
    brief: &mut NeighborhoodBrief,
    nodes: &BTreeMap<&str, &TopologyNode>,
    edges: &[TopologyEdge],
    label_words: &usize,
) {
    let edge_by_id: BTreeMap<&str, &TopologyEdge> =
        edges.iter().map(|e| (e.id.as_str(), e)).collect();
    let mut refs: Vec<BasisRef> = Vec::new();
    let mut index: BTreeMap<(bool, String), usize> = BTreeMap::new();
    let mut number = |is_edge: bool, id: &str, refs: &mut Vec<BasisRef>| -> Option<usize> {
        if let Some(n) = index.get(&(is_edge, id.to_string())) {
            return Some(*n);
        }
        let (target, label) = if is_edge {
            let e = edge_by_id.get(id)?;
            let from = nodes
                .get(e.from.as_str())
                .map_or("record", |n| n.label.as_str());
            let to = nodes
                .get(e.to.as_str())
                .map_or("record", |n| n.label.as_str());
            (
                BasisTarget::Edge(id.to_string()),
                format!(
                    "{}: {} → {}",
                    clip_words(e.label.trim(), 3),
                    clip_words(from, *label_words),
                    clip_words(to, *label_words)
                ),
            )
        } else {
            let n = nodes.get(id)?;
            (
                BasisTarget::Node(id.to_string()),
                format!(
                    "{}: {}",
                    singular(n.kind),
                    clip_words(&n.label, *label_words)
                ),
            )
        };
        let n = refs.len() + 1;
        refs.push(BasisRef { n, target, label });
        index.insert((is_edge, id.to_string()), n);
        Some(n)
    };
    let fill =
        |it: &mut BriefItem,
         refs: &mut Vec<BasisRef>,
         number: &mut dyn FnMut(bool, &str, &mut Vec<BasisRef>) -> Option<usize>| {
            it.node_ids = dedup(std::mem::take(&mut it.node_ids));
            it.edge_ids = dedup(std::mem::take(&mut it.edge_ids));
            let mut basis = Vec::new();
            it.node_ids.retain(|id| match number(false, id, refs) {
                Some(n) => {
                    basis.push(n);
                    true
                }
                None => false,
            });
            it.edge_ids.retain(|id| match number(true, id, refs) {
                Some(n) => {
                    basis.push(n);
                    true
                }
                None => false,
            });
            it.basis = basis;
        };
    if let Some(o) = &mut brief.overview {
        fill(o, &mut refs, &mut number);
    }
    for it in brief
        .connections
        .iter_mut()
        .chain(brief.interpretation.iter_mut())
        .chain(brief.limits.iter_mut())
    {
        fill(it, &mut refs, &mut number);
    }
    brief.references = refs;
}

/// Hash of everything the brief is computed from. Excludes view position,
/// selection, filters and clock time.
fn basis_key(input: &BriefInput) -> String {
    let snap = input.snapshot;
    let mut rows: Vec<String> = Vec::new();
    rows.push(format!(
        "v{FORMAT_VERSION}|{}|{}|{:?}",
        input.seed.key(),
        input.depth,
        input.cycle
    ));
    rows.push(format!(
        "cov|{}|{}|{}|{}|{}|{}|{}|{}",
        snap.discovered_nodes,
        snap.discovered_edges,
        snap.omitted_nodes,
        snap.omitted_edges,
        snap.unresolved_endpoints.len(),
        snap.exploration_exhausted,
        input.payload_reads_skipped,
        snap.nodes.len() + snap.edges.len()
    ));
    rows.extend(snap.nodes.iter().map(|n| {
        format!(
            "n|{}|{}|{}|{}|{}|{}|{}|{}",
            n.id,
            n.kind.label(),
            n.label,
            n.status,
            n.revision,
            n.evidence_count,
            n.is_root,
            n.member_ids.join(",")
        )
    }));
    rows.extend(snap.edges.iter().map(|e| {
        format!(
            "e|{}|{}|{}|{}|{}|{}|{}",
            e.id,
            e.kind.label(),
            e.from,
            e.to,
            e.label,
            e.status,
            e.receipts.join(",")
        )
    }));
    rows.extend(
        input
            .payloads
            .iter()
            .map(|(id, p)| format!("p|{id}|{}", serde_json::to_string(p).unwrap_or_default())),
    );
    rows.sort();
    crate::evidence::content_hash(&rows.join("\n"))
}

#[cfg(test)]
#[path = "brain_neighborhood_brief_tests.rs"]
pub(crate) mod tests;

// ------------------------------------------------------- model input packet

/// Version of the packet shape and the model output contract.
pub const PACKET_VERSION: u32 = 1;
/// Upper bound on the serialized packet sent to a model.
pub const PACKET_BYTES_MAX: usize = 36_000;
const FIELD_CHARS_MAX: usize = 600;
const RECEIPTS_MAX: usize = 6;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PacketNode {
    /// Model-facing reference (`N1`…).
    pub reference: String,
    pub node: TopologyNode,
    pub payload: Option<serde_json::Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PacketEdge {
    /// Model-facing reference (`E1`…).
    pub reference: String,
    pub edge: TopologyEdge,
    pub payload: Option<serde_json::Value>,
}

/// Frozen, typed packet of the loaded neighborhood. It is the only thing a
/// model sees; the deterministic brief is not part of it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NeighborhoodInput {
    pub packet_version: u32,
    pub basis_key: String,
    pub seed: TopologySeed,
    pub seed_label: String,
    pub depth: usize,
    pub cycle: CycleScope,
    pub coverage: BriefCoverage,
    pub nodes: Vec<PacketNode>,
    pub edges: Vec<PacketEdge>,
    /// Disclosed reductions (shortened fields, dropped record details).
    pub omissions: Vec<String>,
}

fn trim_value(v: &mut serde_json::Value, shortened: &mut usize) {
    match v {
        serde_json::Value::String(s) => {
            if s.chars().count() > FIELD_CHARS_MAX {
                let cut: String = s.chars().take(FIELD_CHARS_MAX).collect();
                *s = format!("{}…", cut.trim_end());
                *shortened += 1;
            }
        }
        serde_json::Value::Array(a) => {
            for x in a.iter_mut() {
                trim_value(x, shortened);
            }
        }
        serde_json::Value::Object(o) => {
            for x in o.values_mut() {
                trim_value(x, shortened);
            }
        }
        _ => {}
    }
}

/// Assemble the packet. Deterministic: nodes are ordered (seed, hops, id) and
/// edges by id; references follow that order.
pub fn build_input(input: &BriefInput, basis_key: &str) -> NeighborhoodInput {
    let snap = input.snapshot;
    let mut shortened = 0usize;
    let mut nodes: Vec<&TopologyNode> = snap.nodes.iter().collect();
    nodes.sort_by(|a, b| (!a.is_root, a.min_hops, &a.id).cmp(&(!b.is_root, b.min_hops, &b.id)));
    let mut edges: Vec<&TopologyEdge> = snap.edges.iter().collect();
    edges.sort_by(|a, b| a.id.cmp(&b.id));
    let payload_of = |id: &str, shortened: &mut usize| {
        input.payloads.get(id).and_then(|p| {
            let mut v = serde_json::to_value(p).ok()?;
            trim_value(&mut v, shortened);
            Some(v)
        })
    };
    let mut pnodes: Vec<PacketNode> = nodes
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let mut lite = (*n).clone();
            lite.detail = String::new();
            lite.passages.clear();
            PacketNode {
                reference: format!("N{}", i + 1),
                payload: payload_of(&n.id, &mut shortened),
                node: lite,
            }
        })
        .collect();
    let mut pedges: Vec<PacketEdge> = edges
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let mut lite = (*e).clone();
            lite.receipts.truncate(RECEIPTS_MAX);
            PacketEdge {
                reference: format!("E{}", i + 1),
                payload: payload_of(&e.id, &mut shortened),
                edge: lite,
            }
        })
        .collect();
    let mut out = NeighborhoodInput {
        packet_version: PACKET_VERSION,
        basis_key: basis_key.to_string(),
        seed: input.seed.clone(),
        seed_label: snap.root_label.clone(),
        depth: input.depth,
        cycle: input.cycle.clone(),
        coverage: coverage_of(input),
        nodes: Vec::new(),
        edges: Vec::new(),
        omissions: Vec::new(),
    };
    let mut omissions = Vec::new();
    if shortened > 0 {
        omissions.push(format!("{shortened} long text fields were shortened"));
    }
    // Drop record details from the farthest records first until the packet fits.
    let mut dropped = 0usize;
    loop {
        out.nodes = pnodes.clone();
        out.edges = pedges.clone();
        if prompt_json(&out).len() <= PACKET_BYTES_MAX {
            break;
        }
        let far_node = pnodes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.payload.is_some() && !n.node.is_root)
            .max_by_key(|(_, n)| (n.node.min_hops, n.node.id.clone()))
            .map(|(i, _)| i);
        if let Some(i) = far_node {
            pnodes[i].payload = None;
            dropped += 1;
            continue;
        }
        if let Some(i) = pedges.iter().rposition(|e| e.payload.is_some()) {
            pedges[i].payload = None;
            dropped += 1;
            continue;
        }
        let Some(i) = pnodes.iter().rposition(|n| !n.node.is_root) else {
            break;
        };
        pnodes.remove(i);
        let keep: BTreeSet<&str> = pnodes.iter().map(|n| n.node.id.as_str()).collect();
        pedges.retain(|e| keep.contains(e.edge.from.as_str()) && keep.contains(e.edge.to.as_str()));
        dropped += 1;
    }
    if dropped > 0 {
        omissions.push(format!(
            "{dropped} record details or records were left out to fit the request budget"
        ));
    }
    out.omissions = omissions;
    out
}

fn coverage_of(input: &BriefInput) -> BriefCoverage {
    let snap = input.snapshot;
    BriefCoverage {
        status: if snap.nodes.len() <= 1 && snap.edges.is_empty() {
            CoverageStatus::Isolated
        } else if snap.omitted_nodes > 0
            || snap.omitted_edges > 0
            || !snap.unresolved_endpoints.is_empty()
            || !snap.exploration_exhausted
            || input.payload_reads_skipped > 0
        {
            CoverageStatus::Limited
        } else {
            CoverageStatus::Ready
        },
        depth: input.depth,
        cycle_scope: input.cycle.clone(),
        admitted_nodes: snap.nodes.len(),
        admitted_edges: snap.edges.len(),
        discovered_nodes: snap.discovered_nodes,
        discovered_edges: snap.discovered_edges,
        omitted_nodes: snap.omitted_nodes,
        omitted_edges: snap.omitted_edges,
        unresolved_endpoints: snap.unresolved_endpoints.len(),
        exploration_exhausted: snap.exploration_exhausted,
        payload_reads_skipped: input.payload_reads_skipped,
    }
}

impl NeighborhoodInput {
    /// The packet as the model sees it (compact JSON, typed records only).
    pub fn prompt_json(&self) -> String {
        prompt_json(self)
    }

    fn node_ref(&self, id: &str) -> &str {
        self.nodes
            .iter()
            .find(|n| n.node.id == id)
            .map_or("", |n| n.reference.as_str())
    }

    /// Whether a model has anything to add beyond the deterministic brief.
    pub fn worth_generating(&self) -> bool {
        !self.edges.is_empty()
            && !matches!(
                self.coverage.status,
                CoverageStatus::Isolated | CoverageStatus::Missing
            )
    }
}

fn prompt_json(input: &NeighborhoodInput) -> String {
    use serde_json::json;
    let nodes: Vec<_> = input
        .nodes
        .iter()
        .map(|n| {
            json!({
                "ref": n.reference,
                "kind": n.node.kind.label(),
                "label": n.node.label,
                "status": n.node.status,
                "subtype": n.node.subtype,
                "hops": n.node.min_hops,
                "seed": n.node.is_root,
                "members": n.node.member_ids.len(),
                "evidence_count": n.node.evidence_count,
                "record": n.payload,
            })
        })
        .collect();
    let edges: Vec<_> = input
        .edges
        .iter()
        .map(|e| {
            json!({
                "ref": e.reference,
                "kind": e.edge.kind.label(),
                "from": input.node_ref(&e.edge.from),
                "to": input.node_ref(&e.edge.to),
                "label": e.edge.label,
                "status": e.edge.status,
                "rationale": e.edge.rationale,
                "receipts": e.edge.receipts,
                "record": e.payload,
            })
        })
        .collect();
    json!({
        "seed": {"kind": seed_kind(&input.seed), "label": input.seed_label},
        "options": {"depth": input.depth, "cycle_scope": cycle_label(&input.cycle)},
        "coverage": {
            "status": input.coverage.status.label(),
            "admitted_nodes": input.coverage.admitted_nodes,
            "admitted_edges": input.coverage.admitted_edges,
            "omitted_nodes": input.coverage.omitted_nodes,
            "omitted_edges": input.coverage.omitted_edges,
            "unresolved_endpoints": input.coverage.unresolved_endpoints,
            "exploration_exhausted": input.coverage.exploration_exhausted,
        },
        "nodes": nodes,
        "edges": edges,
        "omissions": input.omissions,
    })
    .to_string()
}

fn cycle_label(c: &CycleScope) -> &'static str {
    match c {
        CycleScope::AllRetained => "all retained cycles",
        CycleScope::ThisCycle(_) => "this cycle",
    }
}

/// The JSON schema of the model's structured brief.
pub fn output_schema() -> serde_json::Value {
    let item = serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["text", "basis"],
        "properties": {
            "text": {"type": "string"},
            "basis": {"type": "array", "items": {"type": "string"}}
        }
    });
    let list = |max: usize| serde_json::json!({"type": "array", "maxItems": max, "items": item});
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["overview", "connections", "interpretation", "limits"],
        "properties": {
            "overview": list(1),
            "connections": list(2),
            "interpretation": list(1),
            "limits": list(1)
        }
    })
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawItem {
    text: String,
    #[serde(default)]
    basis: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBrief {
    #[serde(default)]
    overview: Vec<RawItem>,
    #[serde(default)]
    connections: Vec<RawItem>,
    #[serde(default)]
    interpretation: Vec<RawItem>,
    #[serde(default)]
    limits: Vec<RawItem>,
}

/// A validated generated brief and the host adjustments applied to it.
#[derive(Clone, Debug, PartialEq)]
pub struct Validated {
    pub brief: NeighborhoodBrief,
    pub host_notes: Vec<String>,
}

const WORDS_TOTAL_MAX: usize = 260;
const ITEM_WORDS: std::ops::RangeInclusive<usize> = 4..=75;
const ASSERTIONS: &[&str] = &[
    "proves",
    "proved",
    "proven",
    "confirms",
    "confirmed",
    "verified",
    "coordinat",
    "orchestrat",
    "conspir",
    "controls",
    "controlled",
    "influenc",
    "independent",
    "intentional",
    "deliberate",
    "responsible",
    "caused",
    "causes",
    "guilty",
    "collud",
    "corroborat",
];
const NEGATORS: &[&str] = &[
    "not", "no", "nor", "without", "cannot", "never", "isn't", "doesn't",
];
const HEDGES: &[&str] = &["potential", "possible", "tentative", "may", "might", "lead"];

fn words_of(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric() && c != '\'')
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase())
        .collect()
}

fn packet_relation(e: &PacketEdge) -> Option<String> {
    e.payload
        .as_ref()?
        .get("payload")?
        .get("relation")
        .and_then(|v| v.as_str())
        .map(|s| s.to_lowercase())
}

fn first_words(label: &str, n: usize) -> String {
    label
        .split_whitespace()
        .take(n)
        .collect::<Vec<_>>()
        .join(" ")
        .trim_end_matches(['…', '.', ','])
        .to_lowercase()
}

/// Validate a model's raw answer against the packet. Nothing is returned
/// unless the whole answer passes; host-owned parts (seed, key, counts,
/// coverage, required caveats and pinned contradictions) come from the
/// deterministic `fallback`.
pub fn validate_generated(
    input: &NeighborhoodInput,
    fallback: &NeighborhoodBrief,
    raw: &str,
) -> Result<Validated, String> {
    let body = raw.trim();
    let body = body
        .strip_prefix("```json")
        .or_else(|| body.strip_prefix("```"))
        .map(|b| b.trim_end().trim_end_matches("```").trim())
        .unwrap_or(body);
    if body.is_empty() {
        return Err("empty output".into());
    }
    let parsed: RawBrief =
        serde_json::from_str(body).map_err(|e| format!("malformed output: {e}"))?;
    if parsed.overview.len() > 1
        || parsed.connections.len() > 2
        || parsed.interpretation.len() > 1
        || parsed.limits.len() > 1
    {
        return Err("too many items in a block".into());
    }
    let corpus = input.prompt_json().to_lowercase();
    let mut allowed_numbers: BTreeSet<String> = corpus
        .split(|c: char| !c.is_ascii_digit())
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect();
    for n in [input.nodes.len(), input.edges.len(), input.depth] {
        allowed_numbers.insert(n.to_string());
    }
    let mut host_notes = Vec::new();
    let mut total_words = 0usize;
    let mut model_items = 0usize;

    let convert = |block: &str,
                   raw_items: &[RawItem],
                   total_words: &mut usize|
     -> Result<Vec<BriefItem>, String> {
        let mut out = Vec::new();
        for r in raw_items {
            let text = r.text.split_whitespace().collect::<Vec<_>>().join(" ");
            if text.is_empty() {
                continue;
            }
            let words = words_of(&text);
            if !ITEM_WORDS.contains(&words.len()) {
                return Err(format!("{block}: item length {} words", words.len()));
            }
            *total_words += words.len();
            let mut node_ids = Vec::new();
            let mut edge_ids = Vec::new();
            let mut nodes_cited: Vec<&PacketNode> = Vec::new();
            let mut edges_cited: Vec<&PacketEdge> = Vec::new();
            for r in &r.basis {
                let key = r.trim();
                if let Some(n) = input.nodes.iter().find(|n| n.reference == key) {
                    node_ids.push(n.node.id.clone());
                    nodes_cited.push(n);
                } else if let Some(e) = input.edges.iter().find(|e| e.reference == key) {
                    edge_ids.push(e.edge.id.clone());
                    edges_cited.push(e);
                } else {
                    return Err(format!("{block}: unknown reference {key:?}"));
                }
            }
            if block != "limits" && nodes_cited.is_empty() && edges_cited.is_empty() {
                return Err(format!("{block}: item without a basis reference"));
            }
            let lower = text.to_lowercase();
            // Epistemic class follows the cited records, never the model's say-so.
            let related = edges_cited
                .iter()
                .any(|e| e.edge.kind == TopologyEdgeKind::Related);
            let inferred = nodes_cited
                .iter()
                .any(|n| n.node.kind == TopologyNodeKind::Inference);
            let epistemic = if related {
                if !words.iter().any(|w| HEDGES.contains(&w.as_str())) {
                    return Err(format!("{block}: a tentative lead is stated as fact"));
                }
                Epistemic::Tentative
            } else if inferred {
                if !(lower.contains("inference") || lower.contains("stored")) {
                    return Err(format!("{block}: a stored inference is not attributed"));
                }
                Epistemic::StoredInference
            } else if node_ids.is_empty() && edge_ids.is_empty() {
                Epistemic::Coverage
            } else {
                Epistemic::Recorded
            };
            if block == "interpretation"
                && !matches!(epistemic, Epistemic::StoredInference | Epistemic::Tentative)
            {
                return Err("interpretation: no stored inference or tentative lead cited".into());
            }
            // Unsupported assertions.
            for (i, w) in words.iter().enumerate() {
                if let Some(stem) = ASSERTIONS.iter().find(|a| w.starts_with(**a)) {
                    let negated = words[i.saturating_sub(4)..i]
                        .iter()
                        .any(|p| NEGATORS.contains(&p.as_str()));
                    let in_packet = corpus.contains(*stem);
                    if !negated && !in_packet {
                        return Err(format!("{block}: unsupported assertion {w:?}"));
                    }
                }
            }
            for tok in text
                .split(|c: char| !c.is_ascii_digit())
                .filter(|t| !t.is_empty())
            {
                if !allowed_numbers.contains(tok) {
                    return Err(format!("{block}: number {tok} is not in the record"));
                }
            }
            for q in quoted(&text) {
                let q = q.trim().trim_end_matches(['…', '.']).to_lowercase();
                if q.len() > 2 && !corpus.contains(&q) {
                    return Err(format!("{block}: quoted text {q:?} is not in the record"));
                }
            }
            // Relation direction.
            if edges_cited.len() == 1 && nodes_cited.is_empty() {
                let e = edges_cited[0];
                let rel = packet_relation(e);
                if e.edge.kind == TopologyEdgeKind::Link
                    && matches!(
                        rel.as_deref(),
                        Some("supports" | "derived_from" | "mentioned_in")
                    )
                {
                    let label = |id: &str| {
                        input
                            .nodes
                            .iter()
                            .find(|n| n.node.id == id)
                            .map(|n| first_words(&n.node.label, 2))
                    };
                    if let (Some(a), Some(b)) = (label(&e.edge.from), label(&e.edge.to)) {
                        if let (Some(pa), Some(pb)) = (lower.find(&a), lower.find(&b)) {
                            if pa > pb && a != b {
                                let between = &lower[pb..pa];
                                if !(between.contains(" by ") || between.contains(" from ")) {
                                    return Err(format!("{block}: relation direction reversed"));
                                }
                            }
                        }
                    }
                }
            }
            out.push(BriefItem {
                text,
                epistemic,
                node_ids,
                edge_ids,
                member_ids: Vec::new(),
                receipts: Vec::new(),
                basis: Vec::new(),
            });
        }
        Ok(out)
    };

    let mut overview = convert("overview", &parsed.overview, &mut total_words)?;
    let mut connections = convert("connections", &parsed.connections, &mut total_words)?;
    let interpretation = convert("interpretation", &parsed.interpretation, &mut total_words)?;
    let mut limits = convert("limits", &parsed.limits, &mut total_words)?;
    model_items += overview.len() + connections.len() + interpretation.len() + limits.len();
    if model_items == 0 || (connections.is_empty() && interpretation.is_empty()) {
        return Err("no meaningful content".into());
    }
    if total_words > WORDS_TOTAL_MAX {
        return Err(format!("too long: {total_words} words"));
    }

    // Host-owned overview when the model gave none.
    if overview.is_empty() {
        if let Some(o) = &fallback.overview {
            overview.push(o.clone());
            host_notes.push("overview kept from the deterministic brief".into());
        }
    }
    // Pinned contradictions must stay visible.
    for e in input
        .edges
        .iter()
        .filter(|e| packet_relation(e).as_deref() == Some("contradicts"))
        .take(2)
    {
        let cited = overview
            .iter()
            .chain(&connections)
            .chain(&interpretation)
            .any(|i| i.edge_ids.contains(&e.edge.id));
        if cited {
            continue;
        }
        if let Some(fb) = fallback
            .connections
            .iter()
            .find(|i| i.edge_ids.contains(&e.edge.id))
        {
            if connections.len() >= 2 {
                connections.pop();
            }
            connections.insert(0, fb.clone());
            host_notes.push("contradiction restored from the deterministic brief".into());
        } else {
            return Err("a recorded contradiction is not preserved".into());
        }
    }
    // Required caveats are host-owned.
    if let Some(fb) = fallback.limits.first() {
        let lower = limits
            .first()
            .map(|l| l.text.to_lowercase())
            .unwrap_or_default();
        let needs: &[&str] = if fb.text.contains("dentity") || fb.text.contains("unmerged") {
            &[
                "identity",
                "unmerged",
                "ambiguous",
                "provisional",
                "namesake",
                "same name",
            ]
        } else if fb.text.contains("premise") {
            &["premise"]
        } else if fb.text.contains("incomplete") || fb.text.contains("seed record") {
            &[
                "incomplete",
                "omitted",
                "bounded",
                "limited",
                "absence",
                "partial",
            ]
        } else {
            &[]
        };
        if !needs.is_empty() && !needs.iter().any(|n| lower.contains(n)) {
            limits = vec![fb.clone()];
            host_notes.push("required limit kept from the deterministic brief".into());
        }
    }
    // A stored inference whose premises are partly missing must say so.
    if limits.is_empty() {
        if let Some(fb) = fallback.limits.first() {
            limits.push(fb.clone());
        }
    }

    let mut brief = NeighborhoodBrief {
        format_version: FORMAT_VERSION,
        basis_key: input.basis_key.clone(),
        seed: input.seed.clone(),
        seed_label: input.seed_label.clone(),
        overview: overview.into_iter().next(),
        connections,
        interpretation,
        limits,
        references: Vec::new(),
        coverage: fallback.coverage.clone(),
    };
    let nodes: BTreeMap<&str, &TopologyNode> = input
        .nodes
        .iter()
        .map(|n| (n.node.id.as_str(), &n.node))
        .collect();
    let edges: Vec<TopologyEdge> = input.edges.iter().map(|e| e.edge.clone()).collect();
    number_references(&mut brief, &nodes, &edges, &4);
    Ok(Validated { brief, host_notes })
}

fn quoted(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (open, close) in [('“', '”'), ('"', '"')] {
        let mut rest = text;
        while let Some(i) = rest.find(open) {
            let after = &rest[i + open.len_utf8()..];
            let Some(j) = after.find(close) else { break };
            out.push(after[..j].to_string());
            rest = &after[j + close.len_utf8()..];
        }
    }
    out
}

/// The task given to the model; the packet follows as the user message.
pub const MODEL_TASK: &str = "Explain what stands out around this seed and why its supplied neighborhood matters. Use only the packet. Return Overview, Connections, Interpretation, and Limits as structured brief items with exact basis references. Preserve attribution, uncertainty, existing inference limitations, tentative leads, and material contradictions. Target 120–180 words. Do not create facts, sources, relationships, or new investigative conclusions.";

fn seed_kind(seed: &TopologySeed) -> String {
    format!("{seed:?}")
        .chars()
        .take_while(|c| c.is_alphanumeric())
        .collect()
}
