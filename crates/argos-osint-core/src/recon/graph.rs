//! Directive-linked graph for one Brain memory.
//!
//! Nodes are the investigation, its directives, the claim's entity and topic, the
//! finding, cited evidence, and the source that evidence came from. Edges use the
//! relationship verbs the Brain graph draws.

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
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphEdgeKind {
    Investigates,
    Answers,
    Concerns,
    Supports,
    DerivedFrom,
    DiscoveredIn,
}

impl GraphEdgeKind {
    pub fn verb(self) -> &'static str {
        match self {
            Self::Investigates => "investigates",
            Self::Answers => "answers",
            Self::Concerns => "concerns",
            Self::Supports => "supports",
            Self::DerivedFrom => "derived_from",
            Self::DiscoveredIn => "discovered_in",
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
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphEdge {
    pub from: String,
    pub to: String,
    pub kind: GraphEdgeKind,
    pub directive: Option<String>,
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
        let Some(insight) = self.insight_for_memory(memory_id)? else {
            return Ok(MemoryGraph::default());
        };
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT s.run_id FROM insight_claims c \
             JOIN insight_sources s ON s.fingerprint = c.fingerprint \
             WHERE c.memory_id = ?1 AND s.run_id IS NOT NULL",
        )?;
        let run_ids: Vec<String> = stmt
            .query_map([memory_id], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
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
        }))
    };
    let finding = graph.node(GraphNode {
        id: format!("finding:{}", insight.memory_id),
        kind: GraphNodeKind::Finding,
        label: format!("{} → {}", insight.predicate, insight.object_value),
        detail: format!(
            "{} · {} → {}\n{} · {:.0}%\n{}",
            insight.entity,
            insight.predicate,
            insight.object_value,
            insight.classification,
            insight.confidence * 100.0,
            insight.topic
        ),
        tags: Vec::new(),
    });
    graph.edge(GraphEdge {
        from: finding.clone(),
        to: investigation,
        kind: GraphEdgeKind::DiscoveredIn,
        directive: None,
    });
    if let Some(entity_id) = claim_entity {
        graph.edge(GraphEdge {
            from: finding.clone(),
            to: entity_id,
            kind: GraphEdgeKind::Concerns,
            directive: None,
        });
    }
    if let Some(topic_id) = topic_id.clone() {
        graph.edge(GraphEdge {
            from: finding.clone(),
            to: topic_id,
            kind: GraphEdgeKind::Concerns,
            directive: None,
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
        for directive in &directives {
            if serves_directive(call, &directive.id) {
                answered.insert(directive.id.clone());
            }
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
        });
        if let Some(topic_id) = topic_id.clone() {
            graph.edge(GraphEdge {
                from: format!("directive:{}", directive.id),
                to: topic_id,
                kind: GraphEdgeKind::Investigates,
                directive: Some(directive.id.clone()),
            });
        }
    }

    for source in &cited {
        let call = call_for(&calls, &source.call_id);
        let served = call
            .map(|call| {
                directives
                    .iter()
                    .filter(|directive| serves_directive(call, &directive.id))
                    .map(|directive| directive.id.clone())
                    .collect::<Vec<_>>()
            })
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
        });
        let source_id = graph.node(GraphNode {
            id: format!("source:{}", source_label.to_lowercase()),
            kind: GraphNodeKind::Source,
            label: source_label.clone(),
            detail: format!("Source {source_label}"),
            tags: Vec::new(),
        });
        graph.edge(GraphEdge {
            from: evidence.clone(),
            to: finding.clone(),
            kind: GraphEdgeKind::Supports,
            directive: None,
        });
        graph.edge(GraphEdge {
            from: evidence,
            to: source_id,
            kind: GraphEdgeKind::DerivedFrom,
            directive: None,
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

fn serves_directive(call: &PlanCall, directive_id: &str) -> bool {
    directive_ids(&call.reason)
        .iter()
        .any(|id| id == directive_id)
}

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
            memory_id: "m1".into(),
            entity: "elon musk".into(),
            predicate: "employs".into(),
            object_value: "tesla".into(),
            topic: "identity".into(),
            classification: "fact".into(),
            confidence: 0.9,
            sources: vec![InsightSource {
                thread_id: Some("t".into()),
                answer_id: "a".into(),
                call_id: "call-w1".into(),
                source_url: None,
                deleted_origin: false,
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
        assert_eq!(path.bands.len(), 3);
        assert!(path.bands[0].finding.is_some());
        assert!(path.bands[1].finding.is_none());
        assert!(path.bands[2].finding.is_none());
        assert!(path.bands[2].evidence.is_empty());
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
    }

    #[test]
    fn manual_memory_has_no_graph() {
        let store = Store::memory().unwrap();
        let memory = store
            .add_memory(
                "a note",
                "fact",
                false,
                MemorySource {
                    app: "chat".into(),
                    conversation_id: "c1".into(),
                    message_id: None,
                    reference: None,
                },
            )
            .unwrap();
        let graph = store.graph_for_memory(&memory.id).unwrap();
        assert!(graph.is_empty());
    }
}
