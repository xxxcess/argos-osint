//! GitHub users and repositories for a handle or an email address.
//! A token raises the rate limit. The public API still answers without one.
//! LeakCheck Public adds breach-source hits for the same identity terms.

use std::time::Duration;

use futures_util::future::join_all;
use serde_json::Value;

use super::{
    clip, get_json, get_json_headers, merge_adapter_results, tag, Job, MergeOutcome, SearchHit,
};

pub async fn gather(terms: Vec<String>, token: Option<String>) -> Result<MergeOutcome, String> {
    let token = token.filter(|token| !token.trim().is_empty());
    let mut jobs: Vec<Job> = Vec::new();
    let leak_terms: Vec<String> = terms
        .iter()
        .filter(|term| is_leakcheck_term(term))
        .cloned()
        .collect();
    for term in terms.into_iter().take(2) {
        let user_term = term.clone();
        let user_token = token.clone();
        jobs.push(Box::pin(async move {
            (
                "github",
                github_search("users", &user_term, user_token.as_deref()).await,
            )
        }));
        let repo_token = token.clone();
        jobs.push(Box::pin(async move {
            (
                "github",
                github_search("repositories", &term, repo_token.as_deref()).await,
            )
        }));
    }
    if !leak_terms.is_empty() {
        jobs.push(Box::pin(async move {
            ("leakcheck", leakcheck_terms(leak_terms).await)
        }));
    }
    if jobs.is_empty() {
        return Ok(MergeOutcome::default());
    }
    merge_adapter_results(join_all(jobs).await)
}

fn is_leakcheck_term(term: &str) -> bool {
    let term = term.trim();
    if term.contains('@') {
        return !term.is_empty();
    }
    // Usernames need at least three characters.
    term.chars().count() >= 3
}

async fn leakcheck_terms(terms: Vec<String>) -> Result<Vec<SearchHit>, String> {
    let mut hits = Vec::new();
    let mut last_err: Option<String> = None;
    for (index, term) in terms.into_iter().enumerate() {
        if index > 0 {
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        match leakcheck_one(&term).await {
            Ok(rows) => hits.extend(rows),
            Err(err) => last_err = Some(err),
        }
    }
    if hits.is_empty() {
        if let Some(err) = last_err {
            return Err(err);
        }
    }
    Ok(hits)
}

async fn leakcheck_one(query: &str) -> Result<Vec<SearchHit>, String> {
    let url = format!(
        "https://leakcheck.io/api/public?check={}",
        urlencoding::encode(query)
    );
    let value = get_json(&url).await?;
    Ok(tag(parse_leakcheck(&value, query), "leakcheck"))
}

/// Parse LeakCheck Public JSON. Empty `found` or missing sources yield no hits.
pub(crate) fn parse_leakcheck(value: &Value, query: &str) -> Vec<SearchHit> {
    let found = value.get("found").and_then(|v| v.as_u64()).unwrap_or(0);
    if found == 0 {
        return Vec::new();
    }
    let Some(sources) = value.get("sources").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    if sources.is_empty() {
        return Vec::new();
    }
    let fields: Vec<&str> = value
        .get("fields")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|item| item.as_str()).collect())
        .unwrap_or_default();
    let field_note = if fields.is_empty() {
        String::new()
    } else {
        format!(" · exposed fields: {}", fields.join(", "))
    };
    sources
        .iter()
        .filter_map(|source| {
            let name = source
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            if name.is_empty() {
                return None;
            }
            let date = source
                .get("date")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            let mut snippet = String::new();
            if !date.is_empty() {
                snippet.push_str("date: ");
                snippet.push_str(date);
            }
            snippet.push_str(&field_note);
            if snippet.is_empty() {
                snippet = format!("listed source for {query}");
            }
            Some(SearchHit {
                title: name.to_string(),
                url: "https://leakcheck.io".to_string(),
                snippet: clip(&snippet, 360),
            })
        })
        .collect()
}

async fn github_search(
    kind: &str,
    query: &str,
    token: Option<&str>,
) -> Result<Vec<SearchHit>, String> {
    let url = format!(
        "https://api.github.com/search/{kind}?q={}&per_page=5",
        urlencoding::encode(query)
    );
    let auth = token.map(|token| format!("Bearer {token}"));
    let mut headers = vec![("Accept", "application/vnd.github+json")];
    if let Some(auth) = auth.as_deref() {
        headers.push(("Authorization", auth));
    }
    let value = get_json_headers(&url, &headers)
        .await
        .map_err(|err| match token {
            Some(token) => super::redact(&err, token),
            None => err,
        })?;
    let hits = if kind == "users" {
        parse_github_users(&value)
    } else {
        parse_github_repos(&value)
    };
    Ok(tag(hits, "github"))
}

pub(crate) fn parse_github_users(value: &Value) -> Vec<SearchHit> {
    let Some(items) = value.get("items").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let login = item
                .get("login")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            let url = item
                .get("html_url")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            if login.is_empty() || !url.starts_with("http") {
                return None;
            }
            let kind = item.get("type").and_then(|v| v.as_str()).unwrap_or("User");
            Some(SearchHit {
                title: login.to_string(),
                url: url.to_string(),
                snippet: kind.to_string(),
            })
        })
        .take(5)
        .collect()
}

pub(crate) fn parse_github_repos(value: &Value) -> Vec<SearchHit> {
    let Some(items) = value.get("items").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let name = item
                .get("full_name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            let url = item
                .get("html_url")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            if name.is_empty() || !url.starts_with("http") {
                return None;
            }
            let description = item
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let stars = item
                .get("stargazers_count")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            Some(SearchHit {
                title: name.to_string(),
                url: url.to_string(),
                snippet: clip(&format!("{stars} stars · {description}"), 360),
            })
        })
        .take(5)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_github_users_and_repositories() {
        let users: Value = serde_json::from_str(
            r#"{"items":[{"login":"octocat","html_url":"https://github.com/octocat","type":"User"}]}"#,
        )
        .unwrap();
        let hit = &parse_github_users(&users)[0];
        assert_eq!(hit.title, "octocat");
        assert_eq!(hit.snippet, "User");

        let repos: Value = serde_json::from_str(
            r#"{"items":[{"full_name":"octocat/hello","html_url":"https://github.com/octocat/hello","description":"demo","stargazers_count":12}]}"#,
        )
        .unwrap();
        let hit = &parse_github_repos(&repos)[0];
        assert!(hit.snippet.contains("12 stars"));
        assert!(hit.snippet.contains("demo"));
    }

    #[test]
    fn parses_leakcheck_sources_and_empty_found() {
        let found: Value = serde_json::from_str(
            r#"{"success":true,"found":3,"fields":["username","email"],"sources":[{"name":"Evony.com","date":"2016-07"}]}"#,
        )
        .unwrap();
        let hits = parse_leakcheck(&found, "ada@example.com");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "Evony.com");
        assert!(hits[0].snippet.contains("2016-07"));
        assert!(hits[0].snippet.contains("username"));
        assert!(hits[0].snippet.contains("email"));

        let empty: Value =
            serde_json::from_str(r#"{"success":true,"found":0,"sources":[]}"#).unwrap();
        assert!(parse_leakcheck(&empty, "nobody@example.com").is_empty());

        let no_sources: Value =
            serde_json::from_str(r#"{"success":true,"found":2}"#).unwrap();
        assert!(parse_leakcheck(&no_sources, "ghost").is_empty());
    }
}
