//! Shared normalized result adapters and evidence-shape checks.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::ToolResult;

/// Quality classification for a tool's output.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OutputQuality {
    Usable,
    Irrelevant,
    Partial,
    NoResults,
    MissingBinding,
    TransientFailure,
    Blocked,
}

impl OutputQuality {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Usable => "usable",
            Self::Irrelevant => "irrelevant",
            Self::Partial => "partial",
            Self::NoResults => "no_results",
            Self::MissingBinding => "missing_binding",
            Self::TransientFailure => "transient_failure",
            Self::Blocked => "blocked",
        }
    }

    pub fn is_success(&self) -> bool {
        matches!(self, Self::Usable | Self::Partial)
    }
}

/// A normalized search result item from any web/news/code search tool.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct SearchResultItem {
    pub url: String,
    pub title: String,
    pub snippet: String,
    pub published_at: String,
    pub score: Option<f64>,
}

/// Normalized view of a completed tool run.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NormalizedToolResult {
    pub tool_id: String,
    pub quality: OutputQuality,
    pub items: Vec<Value>,
    pub search_results: Vec<SearchResultItem>,
    pub text: Option<String>,
    pub summary: String,
}

/// Extracts search result items from a tool's observations, supporting both
/// object-shaped (`{"results": [...]}`) and array-shaped observations.
pub fn extract_search_results(observations: &Value) -> Vec<SearchResultItem> {
    let items = extract_observation_items(observations);
    let mut out = Vec::new();

    for item in items {
        if !item.is_object() {
            continue;
        }

        let url = item
            .get("url")
            .or_else(|| item.get("link"))
            .or_else(|| item.get("source_url"))
            .or_else(|| item.get("target"))
            .or_else(|| item.get("html_url"))
            .or_else(|| item.get("destination"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();

        let title = item
            .get("title")
            .or_else(|| item.get("headline"))
            .or_else(|| item.get("name"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();

        let snippet = item
            .get("snippet")
            .or_else(|| item.get("description"))
            .or_else(|| item.get("summary"))
            .or_else(|| item.get("excerpt"))
            .or_else(|| item.get("content"))
            .or_else(|| item.get("text"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();

        let published_at = item
            .get("published_at")
            .or_else(|| item.get("publishedAt"))
            .or_else(|| item.get("published"))
            .or_else(|| item.get("date"))
            .or_else(|| item.get("timestamp"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();

        let score = item
            .get("score")
            .or_else(|| item.get("relevance"))
            .and_then(Value::as_f64);

        if !url.is_empty() || !title.is_empty() || !snippet.is_empty() {
            out.push(SearchResultItem {
                url,
                title,
                snippet,
                published_at,
                score,
            });
        }
    }

    out
}

/// Extracts an array of item objects/values from `observations` across all tool formats.
pub fn extract_observation_items(observations: &Value) -> Vec<Value> {
    if let Some(arr) = observations.as_array() {
        return arr.clone();
    }

    let Some(obj) = observations.as_object() else {
        return Vec::new();
    };

    // Check common container keys
    for key in [
        "results",
        "items",
        "articles",
        "data",
        "candidate_matches",
        "vulnerabilities",
        "search",
    ] {
        if let Some(arr) = obj.get(key).and_then(Value::as_array) {
            return arr.clone();
        }
    }

    // grep.app nested hits: {"hits": {"hits": [...]}}
    if let Some(hits) = obj.get("hits") {
        if let Some(arr) = hits.as_array() {
            return arr.clone();
        }
        if let Some(arr) = hits.get("hits").and_then(Value::as_array) {
            return arr.clone();
        }
    }

    // Single entity or record object: return single item if non-empty
    if !obj.is_empty() {
        vec![observations.clone()]
    } else {
        Vec::new()
    }
}

/// Classify the output quality of a completed or failed tool result.
pub fn classify_output_quality(result: &ToolResult) -> OutputQuality {
    if let Some(err) = &result.error {
        let err_lower = err.to_ascii_lowercase();
        if err_lower.contains("401")
            || err_lower.contains("403")
            || err_lower.contains("unauthorized")
            || err_lower.contains("forbidden")
            || err_lower.contains("access denied")
            || err_lower.contains("blocked")
        {
            return OutputQuality::Blocked;
        }
        if err_lower.contains("missing") && err_lower.contains("binding") {
            return OutputQuality::MissingBinding;
        }
        return OutputQuality::TransientFailure;
    }

    if result.status == "failed" || result.status == "error" {
        return OutputQuality::TransientFailure;
    }

    if result.status == "blocked" {
        return OutputQuality::Blocked;
    }

    let items = extract_observation_items(&result.observations);
    let has_text = result
        .observations
        .get("markdown")
        .or_else(|| result.observations.get("text"))
        .or_else(|| result.observations.get("body"))
        .and_then(Value::as_str)
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);

    if items.is_empty() && !has_text {
        return OutputQuality::NoResults;
    }

    if result.truncated {
        return OutputQuality::Partial;
    }

    OutputQuality::Usable
}

/// Normalize a `ToolResult` into a structured, typed `NormalizedToolResult`.
pub fn normalize_tool_result(result: &ToolResult) -> NormalizedToolResult {
    let quality = classify_output_quality(result);
    let items = extract_observation_items(&result.observations);
    let search_results = extract_search_results(&result.observations);

    let text = result
        .observations
        .get("markdown")
        .or_else(|| result.observations.get("text"))
        .or_else(|| result.observations.get("body"))
        .and_then(Value::as_str)
        .map(str::to_string);

    let summary = if !search_results.is_empty() {
        format!("{} results retrieved", search_results.len())
    } else if !items.is_empty() {
        format!("{} items retrieved", items.len())
    } else if text.is_some() {
        "text content retrieved".to_string()
    } else {
        match quality {
            OutputQuality::NoResults => "no records found".to_string(),
            OutputQuality::Blocked => "provider access blocked".to_string(),
            OutputQuality::TransientFailure => result
                .error
                .clone()
                .unwrap_or_else(|| "operation failed".into()),
            OutputQuality::MissingBinding => "missing required input binding".to_string(),
            _ => "completed".to_string(),
        }
    };

    NormalizedToolResult {
        tool_id: result.tool_id.clone(),
        quality,
        items,
        search_results,
        text,
        summary,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extracts_from_object_with_results_array() {
        let obs = json!({
            "results": [
                {
                    "url": "https://example.com/one",
                    "title": "Example One",
                    "description": "First snippet here",
                    "published_at": "2026-10-01"
                },
                {
                    "url": "https://example.com/two",
                    "title": "Example Two",
                    "snippet": "Second snippet here"
                }
            ]
        });

        let results = extract_search_results(&obs);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].url, "https://example.com/one");
        assert_eq!(results[0].snippet, "First snippet here");
        assert_eq!(results[1].url, "https://example.com/two");
        assert_eq!(results[1].snippet, "Second snippet here");
    }

    #[test]
    fn extracts_from_array_shaped_observations() {
        let obs = json!([
            {
                "url": "https://example.com/item",
                "title": "Array Item",
                "snippet": "Array snippet"
            }
        ]);

        let results = extract_search_results(&obs);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Array Item");
    }

    #[test]
    fn classifies_blocked_and_transient_failures() {
        let blocked = ToolResult {
            tool_id: "hunter_domain_search".into(),
            inputs: json!({}),
            status: "error".into(),
            source_url: "".into(),
            retrieved_at: "".into(),
            observations: Value::Null,
            raw: "".into(),
            error: Some("403 Forbidden: Account suspended".into()),
            cached: false,
            truncated: false,
            credits_charged: 0,
            credits_reported: None,
        };
        assert_eq!(classify_output_quality(&blocked), OutputQuality::Blocked);

        let transient = ToolResult {
            tool_id: "firecrawl_search".into(),
            inputs: json!({}),
            status: "error".into(),
            source_url: "".into(),
            retrieved_at: "".into(),
            observations: Value::Null,
            raw: "".into(),
            error: Some("504 Gateway Timeout".into()),
            cached: false,
            truncated: false,
            credits_charged: 0,
            credits_reported: None,
        };
        assert_eq!(
            classify_output_quality(&transient),
            OutputQuality::TransientFailure
        );
    }

    #[test]
    fn empty_results_is_no_results_not_failure() {
        let no_results = ToolResult {
            tool_id: "firecrawl_search".into(),
            inputs: json!({"query": "obscure nonexistent topic"}),
            status: "completed".into(),
            source_url: "".into(),
            retrieved_at: "".into(),
            observations: json!({"results": []}),
            raw: "".into(),
            error: None,
            cached: false,
            truncated: false,
            credits_charged: 2,
            credits_reported: Some(2),
        };
        assert_eq!(
            classify_output_quality(&no_results),
            OutputQuality::NoResults
        );
    }
}
