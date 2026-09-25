//! GitHub users and repositories for a handle or an email address.
//! A token raises the rate limit. The public API still answers without one.

use futures_util::future::join_all;
use serde_json::Value;

use super::{clip, get_json_headers, merge_adapter_results, tag, Job, SearchHit};

pub async fn gather(terms: Vec<String>, token: Option<String>) -> Result<Vec<SearchHit>, String> {
    let token = token.filter(|token| !token.trim().is_empty());
    let mut jobs: Vec<Job> = Vec::new();
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
    if jobs.is_empty() {
        return Ok(Vec::new());
    }
    merge_adapter_results(join_all(jobs).await)
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
}
