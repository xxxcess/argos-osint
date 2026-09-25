//! Serde-friendly snapshot types for SQLite JSON persistence.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TnaScope {
    Collection,
    Targeted { report_id: String, title: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TnaNodeKind {
    Doc,
    Domain,
    Ip,
    Handle,
    Email,
    Person,
    Org,
    Topic,
}

impl TnaNodeKind {
    pub fn cluster(self) -> TnaCluster {
        match self {
            Self::Domain | Self::Ip => TnaCluster::Infrastructure,
            Self::Topic | Self::Org => TnaCluster::Campaign,
            Self::Person | Self::Handle | Self::Email => TnaCluster::Identity,
            Self::Doc => TnaCluster::FiledReports,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Doc => "doc",
            Self::Domain => "domain",
            Self::Ip => "ip",
            Self::Handle => "handle",
            Self::Email => "email",
            Self::Person => "person",
            Self::Org => "org",
            Self::Topic => "topic",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TnaCluster {
    Infrastructure,
    Campaign,
    Identity,
    FiledReports,
}

impl TnaCluster {
    pub fn all() -> [Self; 4] {
        [
            Self::Infrastructure,
            Self::Campaign,
            Self::Identity,
            Self::FiledReports,
        ]
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Infrastructure => "Infrastructure",
            Self::Campaign => "Campaign",
            Self::Identity => "Identity",
            Self::FiledReports => "Filed reports",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TnaNode {
    pub id: String,
    pub label: String,
    pub kind: TnaNodeKind,
    pub cluster: TnaCluster,
    pub mentions: u32,
    pub degree: u32,
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TnaEdge {
    pub from: String,
    pub to: String,
    pub weight: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TnaAnchor {
    pub node_id: String,
    pub degree: u32,
    pub mentions: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TnaGap {
    pub cluster_a: TnaCluster,
    pub cluster_b: TnaCluster,
    pub note: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TnaClusterSummary {
    pub cluster: TnaCluster,
    pub node_count: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TnaSnapshot {
    pub scope: TnaScope,
    pub title: String,
    pub nodes: Vec<TnaNode>,
    pub edges: Vec<TnaEdge>,
    pub clusters: Vec<TnaClusterSummary>,
    pub anchors: Vec<TnaAnchor>,
    pub gaps: Vec<TnaGap>,
    pub built_at: String,
}

impl TnaSnapshot {
    pub fn empty(scope: TnaScope) -> Self {
        let title = match &scope {
            TnaScope::Collection => "TNA · collection".into(),
            TnaScope::Targeted { title, .. } => format!("TNA · {title}"),
        };
        Self {
            scope,
            title,
            nodes: Vec::new(),
            edges: Vec::new(),
            clusters: Vec::new(),
            anchors: Vec::new(),
            gaps: Vec::new(),
            built_at: chrono::Utc::now()
                .format("%Y-%m-%d %H:%M:%S UTC")
                .to_string(),
        }
    }
}

/// Stable store key for a snapshot.
pub fn desk_key() -> &'static str {
    "desk"
}

pub fn report_key(report_id: &str) -> String {
    format!("report:{report_id}")
}
