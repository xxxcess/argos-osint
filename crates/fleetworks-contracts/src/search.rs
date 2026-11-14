//! Gateway-only actor discovery DTOs, independent of the Tantivy implementation.
//!
//! Hits contain canonical actor IDs and safe projections, not dossiers or engine
//! document addresses. Scores rank retrieval; they do not express confidence or
//! resolve identity. The gateway checks current canonical eligibility/authority
//! before returning labels and the selected dossier remains a separate read.

use std::fmt;

use serde::Deserialize;
use serde::Serialize;

/// Current gateway actor discovery contract.
pub const ACTOR_SEARCH_CONTRACT_VERSION: u16 = 1;
/// Existing actor search route, extended by explicit discovery mode.
pub const ACTOR_SEARCH_PATH: &str = "/api/v1/actors/search";
/// Authoritative dossier route using the canonical selected actor ID.
pub const ACTOR_DETAIL_PATH: &str = "/api/v1/actors/detail";
/// Authorized durable actor index repair route.
pub const ACTOR_REINDEX_PATH: &str = "/api/v1/search/actors/reindex";
/// Initial public query byte bound, separate from service expansion/time bounds.
pub const MAX_ACTOR_SEARCH_QUERY_BYTES: usize = 256;
/// Initial public query token bound.
pub const MAX_ACTOR_SEARCH_QUERY_TOKENS: usize = 16;
/// Maximum public results per bounded actor discovery request.
pub const MAX_ACTOR_SEARCH_RESULTS: u16 = 25;

/// Explicit new discovery mode; deterministic domain name/alias lookups remain.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ActorSearchMode {
    Discovery,
}

/// Plain bounded query text; not unrestricted engine query syntax.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ActorSearchQuery {
    pub query: String,
    pub mode: ActorSearchMode,
    pub limit: u16,
}

impl ActorSearchQuery {
    /// Create a bounded nonempty discovery query, trimming surrounding whitespace.
    pub fn new(query: String, limit: u16) -> Result<Self, SearchQueryError> {
        if query.len() > MAX_ACTOR_SEARCH_QUERY_BYTES {
            return Err(SearchQueryError::QueryTooLong);
        }
        let request = Self {
            query: query.trim().to_owned(),
            mode: ActorSearchMode::Discovery,
            limit,
        };
        request.validate()?;
        Ok(request)
    }

    /// Revalidate requests decoded from query parameters at the server boundary.
    pub fn validate(&self) -> Result<(), SearchQueryError> {
        if self.query.trim().is_empty() {
            return Err(SearchQueryError::EmptyQuery);
        }
        if self.query.len() > MAX_ACTOR_SEARCH_QUERY_BYTES {
            return Err(SearchQueryError::QueryTooLong);
        }
        if self.query.chars().any(char::is_control) {
            return Err(SearchQueryError::ControlCharacter);
        }
        if self.query.split_whitespace().count() > MAX_ACTOR_SEARCH_QUERY_TOKENS {
            return Err(SearchQueryError::TooManyTokens);
        }
        if !(1..=MAX_ACTOR_SEARCH_RESULTS).contains(&self.limit) {
            return Err(SearchQueryError::InvalidLimit);
        }
        Ok(())
    }
}

/// Bounded query validation failure without echoing query content.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum SearchQueryError {
    EmptyQuery,
    QueryTooLong,
    TooManyTokens,
    ControlCharacter,
    InvalidLimit,
}

impl fmt::Display for SearchQueryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::EmptyQuery => "enter an actor name or alias",
            Self::QueryTooLong => "actor search exceeds the UTF-8 query byte limit",
            Self::TooManyTokens => "actor search exceeds the query token limit",
            Self::ControlCharacter => "actor search contains a control character",
            Self::InvalidLimit => "actor search result limit must be between 1 and 25",
        })
    }
}

impl std::error::Error for SearchQueryError {}

/// Index state is separate from a genuine query with no matching actors.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum IndexState {
    Ready,
    Indexing,
    Paused,
    Throttled,
    Stale,
    Unavailable,
    NotBuilt,
    Degraded,
    Unknown,
}

impl IndexState {
    /// Stable server status spelling for accessible labels and diagnostics.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Indexing => "indexing",
            Self::Paused => "paused",
            Self::Throttled => "throttled",
            Self::Stale => "stale",
            Self::Unavailable => "unavailable",
            Self::NotBuilt => "not_built",
            Self::Degraded => "degraded",
            Self::Unknown => "unknown",
        }
    }
}

impl fmt::Display for IndexState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// A safe serving-index snapshot, with committed versus visible work separated.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct IndexStatus {
    pub state: IndexState,
    pub generation: Option<String>,
    pub source_generation: Option<String>,
    pub schema_version: u16,
    pub analyzer_version: u16,
    pub committed_checkpoint: Option<u64>,
    pub visible_checkpoint: Option<u64>,
    pub pending_work: Option<u64>,
    pub observed_at: String,
    pub reason_code: Option<String>,
}

impl IndexStatus {
    /// Represent missing evidence honestly instead of manufacturing readiness.
    #[must_use]
    pub fn unknown(observed_at: String) -> Self {
        Self {
            state: IndexState::Unknown,
            generation: None,
            source_generation: None,
            schema_version: 0,
            analyzer_version: 0,
            committed_checkpoint: None,
            visible_checkpoint: None,
            pending_work: None,
            observed_at,
            reason_code: None,
        }
    }
}

/// Safe candidate projection after canonical gateway eligibility checks.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[non_exhaustive]
pub struct ActorSearchHit {
    pub actor_id: String,
    pub label: String,
    pub summary: Option<String>,
    pub matched_field: Option<String>,
    pub matched_alias: Option<String>,
    pub actor_type: Option<String>,
    #[serde(default)]
    pub countries: Vec<String>,
    pub projection_revision: u64,
    pub index_generation: String,
    /// Retrieval ranking only; never confidence, resolution or corroboration.
    pub score: f64,
}

impl ActorSearchHit {
    /// Create a candidate for a canonical actor, then fill its safe projection.
    #[must_use]
    pub fn new(
        actor_id: String,
        label: String,
        projection_revision: u64,
        index_generation: String,
    ) -> Self {
        Self {
            actor_id,
            label,
            summary: None,
            matched_field: None,
            matched_alias: None,
            actor_type: None,
            countries: Vec::new(),
            projection_revision,
            index_generation,
            score: 0.0,
        }
    }
}

/// A bounded permission-filtered result page and its serving-index state.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[non_exhaustive]
pub struct ActorSearchResponse {
    pub rows: Vec<ActorSearchHit>,
    pub index: IndexStatus,
    pub truncated: bool,
    /// Opaque continuation bound by the server to query/access/index generation.
    pub continuation: Option<String>,
}

impl ActorSearchResponse {
    /// Construct a bounded page without claiming an exhaustive filtered count.
    #[must_use]
    pub fn new(rows: Vec<ActorSearchHit>, index: IndexStatus) -> Self {
        Self {
            rows,
            index,
            truncated: false,
            continuation: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_query_bounds_prevent_empty_or_unbounded_requests() {
        let query = ActorSearchQuery::new("  研究 alias  ".into(), 20).expect("Unicode query");
        assert_eq!(query.query, "研究 alias");
        assert_eq!(
            serde_json::to_value(query).expect("serialize query")["mode"],
            "discovery"
        );
        assert_eq!(
            ActorSearchQuery::new(" ".into(), 20),
            Err(SearchQueryError::EmptyQuery)
        );
        assert_eq!(
            ActorSearchQuery::new("a".repeat(MAX_ACTOR_SEARCH_QUERY_BYTES + 1), 20),
            Err(SearchQueryError::QueryTooLong)
        );
        assert_eq!(
            ActorSearchQuery::new("alias\u{0}name".into(), 20),
            Err(SearchQueryError::ControlCharacter)
        );
        assert_eq!(
            ActorSearchQuery::new(vec!["x"; MAX_ACTOR_SEARCH_QUERY_TOKENS + 1].join(" "), 20),
            Err(SearchQueryError::TooManyTokens)
        );
        for limit in [0, MAX_ACTOR_SEARCH_RESULTS + 1, u16::MAX] {
            assert_eq!(
                ActorSearchQuery::new("alias".into(), limit),
                Err(SearchQueryError::InvalidLimit)
            );
        }
    }

    #[test]
    fn decoded_requests_must_be_validated_and_cannot_select_private_scope() {
        let unbounded: ActorSearchQuery = serde_json::from_value(
            serde_json::json!({"query":"alias","mode":"discovery","limit":0}),
        )
        .expect("decode before boundary validation");
        assert_eq!(unbounded.validate(), Err(SearchQueryError::InvalidLimit));
        for field in [
            "index_dir",
            "tenant",
            "workspace_id",
            "service_origin",
            "regex",
        ] {
            let mut value = serde_json::json!({"query":"alias","mode":"discovery","limit":20});
            value[field] = serde_json::json!("forged");
            assert!(
                serde_json::from_value::<ActorSearchQuery>(value).is_err(),
                "field {field}"
            );
        }
    }

    #[test]
    fn unavailable_or_unbuilt_index_is_not_an_empty_ready_result() {
        let missing = ActorSearchResponse::new(
            Vec::new(),
            IndexStatus::unknown("2026-01-01T00:00:00Z".into()),
        );
        assert_eq!(missing.index.state, IndexState::Unknown);
        assert!(missing.index.visible_checkpoint.is_none());
        assert!(missing.rows.is_empty());
        let encoded = serde_json::to_value(&missing).expect("serialize state");
        assert_eq!(encoded["index"]["state"], "unknown");
        assert_ne!(encoded["index"]["state"], "ready");
    }
}
