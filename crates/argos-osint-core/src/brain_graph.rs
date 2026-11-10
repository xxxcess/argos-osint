//! Shared Brain memory graph nodes, relationships, and lookup helpers.

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
