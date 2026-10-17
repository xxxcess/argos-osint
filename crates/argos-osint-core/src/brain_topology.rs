//! Shared immutable Brain memory topology queries and projections.
//!
//! Provides `Store::topology_snapshot` across Claims, Actors, Articles, Links,
//! and Inferences with bounded depth and continuation.

use serde::{Deserialize, Serialize};

use crate::brain_browse::Associations;

#[path = "brain_topology_query.rs"]
mod query;

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
    Topic(String),
    Source {
        origin: String,
        source_id: String,
        revision: String,
    },
    /// Immutable Intel display record; resolved without writing on navigation.
    RetainedClaim {
        run_id: String,
        article_id: String,
        revision: String,
        local_id: String,
        text: String,
        job_id: String,
    },
    RetainedInference {
        run_id: String,
        article_id: String,
        revision: String,
        local_id: String,
        text: String,
        job_id: String,
    },
    RetainedActor {
        run_id: String,
        article_id: String,
        revision: String,
        actor_ref: String,
        name: String,
    },
}

impl TopologySeed {
    pub fn key(&self) -> String {
        match self {
            Self::Claim(id) => format!("claim:{id}"),
            Self::Actor(id) => format!("actor:{id}"),
            Self::ActorGroup {
                name_key,
                member_ids,
            } => {
                let mut ids = member_ids.clone();
                ids.sort();
                ids.dedup();
                format!(
                    "actor-group:{name_key}:{}",
                    crate::nlp::cache::hash(&ids.join("\u{1f}"))
                )
            }
            Self::Article {
                run_id,
                article_id,
                revision,
                ..
            } => format!("article:{run_id}/{article_id}@{revision}"),
            Self::Inference(id) => format!("inference:{id}"),
            Self::Link(id) => format!("link:{id}"),
            Self::Related(id) => format!("related:{id}"),
            Self::Topic(label) => format!(
                "topic:v1:{}",
                crate::nlp::cache::hash(&label.trim().to_lowercase())
            ),
            Self::Source {
                origin,
                source_id,
                revision,
            } => format!("source:{origin}:{source_id}@{revision}"),
            Self::RetainedClaim {
                run_id,
                article_id,
                revision,
                local_id,
                job_id,
                ..
            } => format!("retained-claim:{run_id}/{article_id}@{revision}:{job_id}:{local_id}"),
            Self::RetainedInference {
                run_id,
                article_id,
                revision,
                local_id,
                job_id,
                ..
            } => format!("retained-inference:{run_id}/{article_id}@{revision}:{job_id}:{local_id}"),
            Self::RetainedActor {
                run_id,
                article_id,
                revision,
                actor_ref,
                ..
            } => format!("retained-actor:{run_id}/{article_id}@{revision}:{actor_ref}"),
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
            Self::Topic(label) => format!("Topic {label}"),
            Self::Source { source_id, .. } => format!("Source {source_id}"),
            Self::RetainedClaim { text, .. } => text.clone(),
            Self::RetainedInference { text, .. } => text.clone(),
            Self::RetainedActor { name, .. } => name.clone(),
        }
    }
}

/// Presentation kinds in the retained relationship projection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TopologyNodeKind {
    Entity,
    ActorGroup,
    Claim,
    Inference,
    Topic,
    Article,
    Source,
}

impl TopologyNodeKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Entity => "Actor",
            Self::ActorGroup => "Actor group",
            Self::Claim => "Claim",
            Self::Inference => "Inference",
            Self::Topic => "Topic",
            Self::Article => "Article",
            Self::Source => "Source",
        }
    }

    pub fn glyph(self) -> &'static str {
        match self {
            Self::Entity | Self::ActorGroup => "●",
            Self::Claim => "★",
            Self::Inference => "◆",
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
    #[serde(default)]
    pub min_hops: usize,
    #[serde(default)]
    pub revision: String,
    /// Previous node on a discovered shortest explanatory path.
    #[serde(default)]
    pub predecessor: Option<String>,
    #[serde(default)]
    pub topology_seed: Option<TopologySeed>,
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
            depth: 2,
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
    #[serde(default)]
    pub discovered_nodes: usize,
    #[serde(default)]
    pub discovered_edges: usize,
    #[serde(default)]
    pub omitted_nodes: usize,
    #[serde(default)]
    pub omitted_edges: usize,
    #[serde(default)]
    pub unresolved_endpoints: Vec<String>,
    #[serde(default)]
    pub exploration_exhausted: bool,
}
