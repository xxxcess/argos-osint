//! News (NewsAPI) and court-record (CourtListener) adapters (issue #29). Both are keyed,
//! fixed-host GETs: the NewsAPI key goes in `X-Api-Key` and the CourtListener token in
//! `Authorization: Token …`, never in the URL. One page (at most 10 NewsAPI articles,
//! at most 20 CourtListener results), and the query is always the exact-phrase entity.
use super::{bounded, clip_text, domain, get, str_arg, url, Request};
use anyhow::{anyhow, ensure, Result};
use serde_json::{json, Value};
use std::time::Duration;

pub const NEWSAPI_BASE: &str = "https://newsapi.org/v2";
pub const COURTLISTENER_SEARCH: &str = "https://www.courtlistener.com/api/rest/v4/search/";
pub const COURTLISTENER_SITE: &str = "https://www.courtlistener.com";
pub const NEWSAPI_HOST: &str = "newsapi.org";
pub const COURTLISTENER_HOST: &str = "www.courtlistener.com";
/// NewsAPI articles per call (search and headlines); `pageSize` is set to this.
pub const NEWS_MAX_RESULTS: usize = 10;
/// CourtListener results kept per call (one page).
pub const LEGAL_MAX_RESULTS: usize = 20;

/// Results kept per call for a context tool.
pub fn max_results(id: &str) -> usize {
    if provider(id) == Some("newsapi") {
        NEWS_MAX_RESULTS
    } else {
        LEGAL_MAX_RESULTS
    }
}
/// CourtListener allows 5 requests a minute: at least 12 s between requests.
pub const COURTLISTENER_SPACING: Duration = Duration::from_secs(12);
/// Reason recorded on CourtListener steps skipped after a 429.
pub const COURTLISTENER_RATE_LIMIT: &str = "CourtListener rate limit reached";

pub const NEWS_TOOLS: &[&str] = &["newsapi_search", "newsapi_headlines"];
pub const LEGAL_TOOLS: &[&str] = &[
    "courtlistener_case_search",
    "courtlistener_docket_search",
    "courtlistener_judge_search",
];

const LANGUAGES: &[&str] = &[
    "ar", "de", "en", "es", "fr", "he", "it", "nl", "no", "pt", "ru", "sv", "ud", "zh",
];
const SORTS: &[&str] = &["relevancy", "publishedAt", "popularity"];
const CATEGORIES: &[&str] = &[
    "business",
    "entertainment",
    "general",
    "health",
    "science",
    "sports",
    "technology",
];

/// `news` or `legal` for the five context tools (directive context kinds), else none.
pub fn context_kind(id: &str) -> Option<&'static str> {
    let id = super::canonical_tool_id(id);
    if NEWS_TOOLS.contains(&id) {
        Some("news")
    } else if LEGAL_TOOLS.contains(&id) {
        Some("legal")
    } else {
        None
    }
}

/// `newsapi` or `courtlistener` for the context tools.
pub fn provider(id: &str) -> Option<&'static str> {
    match context_kind(id)? {
        "news" => Some("newsapi"),
        _ => Some("courtlistener"),
    }
}

pub fn locked_host(id: &str) -> Option<&'static str> {
    match provider(id)? {
        "newsapi" => Some(NEWSAPI_HOST),
        _ => Some(COURTLISTENER_HOST),
    }
}

/// The entity as an exact phrase: surrounding quotes are dropped, inner quotes refused.
pub fn exact_phrase(raw: &str) -> Result<String> {
    let bare = bounded(raw.trim().trim_matches('"').trim())?;
    ensure!(
        !bare.is_empty() && !bare.contains('"'),
        "query must be a plain name, without quotes"
    );
    Ok(format!("\"{bare}\""))
}

fn date_arg(v: &Value, key: &str) -> Result<Option<String>> {
    let Some(raw) = v.get(key) else {
        return Ok(None);
    };
    let text = raw
        .as_str()
        .map(str::trim)
        .ok_or_else(|| anyhow!("{key} must be a YYYY-MM-DD date"))?;
    ensure!(
        chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d").is_ok(),
        "{key} must be a YYYY-MM-DD date"
    );
    Ok(Some(text.to_string()))
}

fn choice_arg<'a>(v: &'a Value, key: &str, allowed: &[&str]) -> Result<Option<&'a str>> {
    let Some(raw) = v.get(key) else {
        return Ok(None);
    };
    let text = raw.as_str().map(str::trim).unwrap_or("");
    ensure!(
        allowed.contains(&text),
        "{key} must be one of {}",
        allowed.join(", ")
    );
    Ok(Some(text))
}

pub fn request(id: &str, v: &Value) -> Result<Request> {
    let id = super::canonical_tool_id(id);
    let phrase = exact_phrase(str_arg(v, "query")?)?;
    let size = NEWS_MAX_RESULTS.to_string();
    let mut pairs: Vec<(&str, String)> = vec![("q", phrase)];
    let (base, path): (&str, &[&str]) = match id {
        "newsapi_search" => {
            if let Some(from) = date_arg(v, "from")? {
                pairs.push(("from", from));
            }
            if let Some(to) = date_arg(v, "to")? {
                pairs.push(("to", to));
            }
            if let Some(language) = choice_arg(v, "language", LANGUAGES)? {
                pairs.push(("language", language.into()));
            }
            pairs.push((
                "sortBy",
                choice_arg(v, "sort_by", SORTS)?
                    .unwrap_or("relevancy")
                    .into(),
            ));
            if let Some(raw) = v.get("domains") {
                let list = raw
                    .as_str()
                    .ok_or_else(|| anyhow!("domains must be a comma-separated string"))?;
                let domains: Vec<String> = list
                    .split(',')
                    .map(str::trim)
                    .filter(|item| !item.is_empty())
                    .map(domain)
                    .collect::<Result<_>>()?;
                ensure!(
                    !domains.is_empty() && domains.len() <= 10,
                    "domains takes 1 to 10 domains"
                );
                pairs.push(("domains", domains.join(",")));
            }
            pairs.push(("pageSize", size));
            (NEWSAPI_BASE, &["everything"])
        }
        "newsapi_headlines" => {
            if let Some(raw) = v.get("country") {
                let country = raw
                    .as_str()
                    .map(str::trim)
                    .unwrap_or("")
                    .to_ascii_lowercase();
                ensure!(
                    country.len() == 2 && country.bytes().all(|b| b.is_ascii_lowercase()),
                    "country must be a 2-letter code"
                );
                pairs.push(("country", country));
            }
            if let Some(category) = choice_arg(v, "category", CATEGORIES)? {
                pairs.push(("category", category.into()));
            }
            pairs.push(("pageSize", size));
            (NEWSAPI_BASE, &["top-headlines"])
        }
        "courtlistener_case_search"
        | "courtlistener_docket_search"
        | "courtlistener_judge_search" => {
            let kind = match id {
                "courtlistener_case_search" => "o",
                "courtlistener_docket_search" => "r",
                _ => "p",
            };
            pairs.push(("type", kind.into()));
            if kind != "p" {
                if let Some(raw) = v.get("court") {
                    let text = raw.as_str().map(str::trim).unwrap_or("");
                    let courts: Vec<&str> = text.split_whitespace().collect();
                    ensure!(
                        !courts.is_empty()
                            && courts.len() <= 10
                            && courts.iter().all(|court| court.len() <= 20
                                && court
                                    .bytes()
                                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())),
                        "court must be CourtListener court ids separated by spaces"
                    );
                    pairs.push(("court", courts.join(" ")));
                }
                if let Some(after) = date_arg(v, "filed_after")? {
                    pairs.push(("filed_after", after));
                }
                if kind == "o" {
                    if let Some(before) = date_arg(v, "filed_before")? {
                        pairs.push(("filed_before", before));
                    }
                }
            }
            (COURTLISTENER_SEARCH, &[])
        }
        _ => return Err(anyhow!("unknown tool {id}")),
    };
    let borrowed: Vec<(&str, &str)> = pairs
        .iter()
        .map(|(key, value)| (*key, value.as_str()))
        .collect();
    Ok(get(url(base, path, &borrowed)?))
}

fn text<'a>(row: &'a Value, keys: &[&str]) -> &'a str {
    keys.iter()
        .find_map(|key| {
            row.pointer(key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })
        .unwrap_or("")
}

fn first_snippet(row: &Value, list: &str) -> String {
    row.get(list)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| item.get("snippet").and_then(Value::as_str))
        .map(str::trim)
        .find(|snippet| !snippet.is_empty())
        .unwrap_or("")
        .to_string()
}

fn site_url(path: &str) -> String {
    if path.is_empty() {
        String::new()
    } else if path.starts_with("https://") {
        path.to_string()
    } else {
        format!("{COURTLISTENER_SITE}/{}", path.trim_start_matches('/'))
    }
}

/// A readable message for a NewsAPI `status: error` body.
pub fn newsapi_error(value: &Value) -> Option<String> {
    if value.get("status").and_then(Value::as_str) != Some("error") {
        return None;
    }
    let code = value
        .get("code")
        .and_then(Value::as_str)
        .unwrap_or("unexpectedError");
    let message: String = value
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("")
        .chars()
        .take(200)
        .collect();
    Some(match code {
        "apiKeyInvalid" => "NewsAPI rejected the API key (apiKeyInvalid). Check the key on a News tool or NEWSAPI_API_KEY.".into(),
        "apiKeyMissing" => "NewsAPI received no API key (apiKeyMissing). Enter the NewsAPI key on a News tool, or set NEWSAPI_API_KEY.".into(),
        "apiKeyDisabled" | "apiKeyExhausted" => format!("NewsAPI key unavailable ({code}): {message}"),
        "rateLimited" => "NewsAPI rate limit reached (rateLimited): the Developer plan allows 100 requests a day.".into(),
        _ => format!("NewsAPI error {code}: {message}"),
    })
}

/// A readable error for a non-success HTTP status from either provider. `None` for other
/// tools (the executor's generic message is used).
pub fn http_error(id: &str, status: u16, raw: &str) -> Option<String> {
    match provider(id)? {
        "newsapi" => {
            let parsed = serde_json::from_str::<Value>(raw)
                .ok()
                .and_then(|value| newsapi_error(&value));
            Some(parsed.unwrap_or_else(|| match status {
                401 => "NewsAPI rejected the API key (HTTP 401). Check the key on a News tool or NEWSAPI_API_KEY.".into(),
                429 => "NewsAPI rate limit reached (HTTP 429): the Developer plan allows 100 requests a day.".into(),
                _ => format!("NewsAPI HTTP {status}"),
            }))
        }
        _ => {
            let detail: String = serde_json::from_str::<Value>(raw)
                .ok()
                .and_then(|value| {
                    value
                        .get("detail")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .unwrap_or_default()
                .chars()
                .take(200)
                .collect();
            let suffix = if detail.is_empty() {
                String::new()
            } else {
                format!(": {detail}")
            };
            Some(match status {
                401 => format!("CourtListener rejected the API token (HTTP 401{suffix}). Check the token on a Legal tool or COURTLISTENER_API_TOKEN."),
                403 => format!("CourtListener refused the request (HTTP 403{suffix}). The token may lack access to this endpoint."),
                429 => format!("{COURTLISTENER_RATE_LIMIT} (HTTP 429): the free tier allows 5 requests a minute, 50 an hour, and 125 a day."),
                _ => format!("CourtListener HTTP {status}{suffix}"),
            })
        }
    }
}

/// Observations for a context tool: `results` rows with title, date, source (or court),
/// url, and snippet; at most 10 for NewsAPI and 20 for CourtListener. A NewsAPI
/// `status: error` body is an error.
pub fn observations(id: &str, value: &Value) -> Result<(Value, bool)> {
    let id = super::canonical_tool_id(id);
    if let Some(message) = newsapi_error(value) {
        return Err(anyhow!(message));
    }
    let (rows, total) = if provider(id) == Some("newsapi") {
        (
            value.get("articles").and_then(Value::as_array),
            value.get("totalResults").and_then(Value::as_u64),
        )
    } else {
        (
            value.get("results").and_then(Value::as_array),
            value.get("count").and_then(Value::as_u64),
        )
    };
    let rows: Vec<&Value> = rows.into_iter().flatten().collect();
    let limit = max_results(id);
    let truncated =
        rows.len() > limit || total.is_some_and(|total| total as usize > rows.len().min(limit));
    let results: Vec<Value> = rows
        .into_iter()
        .take(limit)
        .filter_map(|row| {
            let item = match id {
                "newsapi_search" | "newsapi_headlines" => json!({
                    "title": clip_text(text(row, &["/title"])),
                    "date": text(row, &["/publishedAt"]),
                    "source": clip_text(text(row, &["/source/name", "/source/id"])),
                    "url": text(row, &["/url"]),
                    "snippet": clip_text(text(row, &["/description", "/content"])),
                }),
                "courtlistener_judge_search" => {
                    let name = text(row, &["/name", "/name_full"]).to_string();
                    let name = if name.is_empty() {
                        [text(row, &["/name_first"]), text(row, &["/name_middle"]), text(row, &["/name_last"])].iter().filter(|part| !part.is_empty()).copied().collect::<Vec<_>>().join(" ")
                    } else {
                        name
                    };
                    json!({
                        "title": clip_text(&name),
                        "date": text(row, &["/positions/0/date_start", "/date_start"]),
                        "court": clip_text(text(row, &["/positions/0/court_full_name", "/positions/0/court", "/court"])),
                        "url": site_url(text(row, &["/absolute_url"])),
                        "snippet": clip_text(text(row, &["/positions/0/position_type", "/political_affiliation", "/snippet"])),
                    })
                }
                _ => {
                    let list = if id == "courtlistener_case_search" { "opinions" } else { "recap_documents" };
                    let snippet = first_snippet(row, list);
                    let snippet = if snippet.is_empty() { text(row, &["/snippet", "/cause", "/suitNature"]).to_string() } else { snippet };
                    json!({
                        "title": clip_text(text(row, &["/caseName", "/case_name", "/caseNameFull"])),
                        "date": text(row, &["/dateFiled", "/date_filed"]),
                        "court": clip_text(text(row, &["/court", "/court_citation_string", "/court_id"])),
                        "docket_number": text(row, &["/docketNumber"]),
                        "url": site_url(text(row, &["/absolute_url"])),
                        "snippet": clip_text(&snippet),
                    })
                }
            };
            let url = item.get("url").and_then(Value::as_str).unwrap_or("");
            (url.starts_with("https://") || url.starts_with("http://")).then_some(item)
        })
        .collect();
    Ok((
        json!({
            "provider": provider(id).unwrap_or(""),
            "context": context_kind(id).unwrap_or(""),
            "total": total,
            "results": results,
            "evidence_form": "snippet",
        }),
        truncated,
    ))
}

/// A local stand-in for newsapi.org and www.courtlistener.com in tests: every request to
/// those hosts goes to one server that answers with `reply(request_line)` and records
/// each raw request (headers and body). Holding the fixture serializes such tests.
#[cfg(test)]
pub(crate) mod fixture {
    use std::sync::{Arc, Mutex};

    pub(crate) static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    pub(crate) type Reply = Arc<dyn Fn(&str) -> (u16, String) + Send + Sync>;

    pub(crate) struct Fixture {
        pub requests: Arc<Mutex<Vec<String>>>,
        _guard: tokio::sync::MutexGuard<'static, ()>,
    }

    impl Fixture {
        pub fn requests(&self) -> Vec<String> {
            self.requests.lock().unwrap().clone()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            crate::osint::TEST_BASES.lock().unwrap().clear();
        }
    }

    pub(crate) async fn serve(reply: Reply) -> Fixture {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let guard = LOCK.lock().await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        {
            let mut bases = crate::osint::TEST_BASES.lock().unwrap();
            bases.clear();
            bases.push((super::NEWSAPI_HOST.into(), base.clone()));
            bases.push((super::COURTLISTENER_HOST.into(), base));
        }
        let requests = Arc::new(Mutex::new(Vec::new()));
        let record = requests.clone();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let mut raw = Vec::new();
                let mut buffer = vec![0u8; 65536];
                while let Ok(n) = socket.read(&mut buffer).await {
                    raw.extend_from_slice(&buffer[..n]);
                    if n == 0 || String::from_utf8_lossy(&raw).contains("\r\n\r\n") {
                        break;
                    }
                }
                let text = String::from_utf8_lossy(&raw).to_string();
                record.lock().unwrap().push(text.clone());
                let line = text.lines().next().unwrap_or("").to_string();
                let (status, body) = reply(&line);
                let reason = match status {
                    200 => "OK",
                    401 => "Unauthorized",
                    403 => "Forbidden",
                    429 => "Too Many Requests",
                    _ => "Error",
                };
                let response = format!(
                    "HTTP/1.1 {status} {reason}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(response.as_bytes()).await;
            }
        });
        Fixture {
            requests,
            _guard: guard,
        }
    }

    /// One NewsAPI article.
    pub(crate) fn article(title: &str, description: &str, url: &str) -> serde_json::Value {
        serde_json::json!({"source": {"id": null, "name": "Reuters"}, "author": "A. Writer", "title": title, "description": description, "url": url, "publishedAt": "2026-09-30T14:00:00Z", "content": description})
    }

    pub(crate) fn articles(rows: &[serde_json::Value]) -> String {
        serde_json::json!({"status": "ok", "totalResults": rows.len(), "articles": rows})
            .to_string()
    }

    /// One CourtListener opinion-search result.
    pub(crate) fn opinion(case: &str, snippet: &str) -> serde_json::Value {
        serde_json::json!({"caseName": case, "absolute_url": "/opinion/123/example/", "court": "District Court, D. Delaware", "court_id": "ded", "dateFiled": "2024-01-30", "docketNumber": "1:22-cv-01234", "opinions": [{"snippet": snippet}]})
    }

    pub(crate) fn search(rows: &[serde_json::Value]) -> String {
        serde_json::json!({"count": rows.len(), "next": null, "previous": null, "results": rows})
            .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::{article, articles, opinion, search, serve};
    use super::*;
    use crate::osint::{
        definition, provider_credential, registry, validate, Executor, ProviderKeys,
        CACHE_DAY_SECONDS,
    };
    use std::sync::Arc;

    const SENTINEL: &str = "sk-test-SENTINEL-29-do-not-leak";

    fn keys() -> ProviderKeys {
        ProviderKeys {
            newsapi: SENTINEL.into(),
            courtlistener: SENTINEL.into(),
            ..ProviderKeys::default()
        }
    }

    fn pairs(req: &Request) -> Vec<(String, String)> {
        req.url
            .query_pairs()
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect()
    }

    /// AC1: 55 tools; the five new ones sit under News and Legal with docs, inputs, and
    /// policy text that names the key, its header, and the limits.
    #[test]
    fn ac1_catalog_has_55_tools_and_the_news_and_legal_entries() {
        assert_eq!(registry().len(), 55);
        let news: Vec<&str> = registry()
            .iter()
            .filter(|tool| tool.category == "News")
            .map(|tool| tool.id)
            .collect();
        let legal: Vec<&str> = registry()
            .iter()
            .filter(|tool| tool.category == "Legal")
            .map(|tool| tool.id)
            .collect();
        assert_eq!(news, NEWS_TOOLS);
        assert_eq!(legal, LEGAL_TOOLS);
        for id in NEWS_TOOLS.iter().chain(LEGAL_TOOLS) {
            let tool = definition(id).unwrap();
            assert_eq!(tool.inputs, ["query"], "{id}");
            assert!(
                tool.documentation.starts_with("https://newsapi.org/docs")
                    || tool
                        .documentation
                        .starts_with("https://www.courtlistener.com/help/api/"),
                "{id}"
            );
            assert!(tool.restrictions.contains("exact phrase"), "{id}");
            let cap = if NEWS_TOOLS.contains(id) {
                "pageSize 10"
            } else {
                "at most 20 results"
            };
            assert!(tool.restrictions.contains(cap), "{id}: {cap}");
            assert!(
                !NEWS_TOOLS.contains(id) || !tool.restrictions.contains("20"),
                "{id}: no 20 in a NewsAPI policy"
            );
            validate(id, &tool.example_input()).unwrap();
            assert_eq!(
                crate::osint::endpoint_cost(id).map(|cost| cost.credits),
                Some(0),
                "{id}: not credit-metered"
            );
        }
        for id in NEWS_TOOLS {
            let tool = definition(id).unwrap();
            assert!(
                tool.description.contains("24 hours") && tool.description.contains("one month"),
                "{id}: delay and window"
            );
            assert!(
                tool.restrictions.contains("NEWSAPI_API_KEY")
                    && tool.restrictions.contains("X-Api-Key"),
                "{id}"
            );
            assert_eq!(tool.cache_seconds, CACHE_DAY_SECONDS);
        }
        for id in LEGAL_TOOLS {
            let tool = definition(id).unwrap();
            assert!(
                tool.restrictions.contains("COURTLISTENER_API_TOKEN")
                    || tool.restrictions.contains("Same CourtListener token"),
                "{id}"
            );
            assert!(tool.restrictions.contains("12 s apart"), "{id}");
            assert_eq!(tool.cache_seconds, CACHE_DAY_SECONDS);
        }
        let optional = |id: &str| {
            definition(id).unwrap().schema()["properties"]
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect::<Vec<_>>()
        };
        for key in ["from", "to", "language", "sort_by", "domains"] {
            assert!(
                optional("newsapi_search").contains(&key.to_string()),
                "{key}"
            );
        }
        assert!(
            optional("newsapi_headlines").contains(&"country".to_string())
                && optional("newsapi_headlines").contains(&"category".to_string())
        );
        assert!(optional("courtlistener_case_search").contains(&"filed_before".to_string()));
        assert!(!optional("courtlistener_docket_search").contains(&"filed_before".to_string()));
    }

    /// AC1: request fixture per tool: URL, params, auth header, and no key in the URL.
    #[test]
    fn ac1_each_request_has_its_url_params_and_auth_header_and_no_key_in_the_url() {
        type Case = (
            &'static str,
            Value,
            &'static str,
            Vec<(&'static str, &'static str)>,
        );
        let cases: [Case; 5] = [
            (
                "newsapi_search",
                json!({"query": "Elon Musk", "from": "2026-09-01", "to": "2026-09-30", "language": "en", "sort_by": "publishedAt", "domains": "reuters.com, apnews.com"}),
                "https://newsapi.org/v2/everything",
                vec![
                    ("q", "\"Elon Musk\""),
                    ("from", "2026-09-01"),
                    ("to", "2026-09-30"),
                    ("language", "en"),
                    ("sortBy", "publishedAt"),
                    ("domains", "reuters.com,apnews.com"),
                    ("pageSize", "10"),
                ],
            ),
            (
                "newsapi_headlines",
                json!({"query": "Elon Musk", "country": "US", "category": "business"}),
                "https://newsapi.org/v2/top-headlines",
                vec![
                    ("q", "\"Elon Musk\""),
                    ("country", "us"),
                    ("category", "business"),
                    ("pageSize", "10"),
                ],
            ),
            (
                "courtlistener_case_search",
                json!({"query": "Elon Musk", "court": "ded cand", "filed_after": "2020-01-01", "filed_before": "2026-01-01"}),
                "https://www.courtlistener.com/api/rest/v4/search/",
                vec![
                    ("q", "\"Elon Musk\""),
                    ("type", "o"),
                    ("court", "ded cand"),
                    ("filed_after", "2020-01-01"),
                    ("filed_before", "2026-01-01"),
                ],
            ),
            (
                "courtlistener_docket_search",
                json!({"query": "Elon Musk", "filed_after": "2025-03-01"}),
                "https://www.courtlistener.com/api/rest/v4/search/",
                vec![
                    ("q", "\"Elon Musk\""),
                    ("type", "r"),
                    ("filed_after", "2025-03-01"),
                ],
            ),
            (
                "courtlistener_judge_search",
                json!({"query": "Elon Musk"}),
                "https://www.courtlistener.com/api/rest/v4/search/",
                vec![("q", "\"Elon Musk\""), ("type", "p")],
            ),
        ];
        for (id, args, endpoint, expected) in cases {
            let req = request(id, &args).unwrap_or_else(|err| panic!("{id}: {err}"));
            let mut bare = req.url.clone();
            bare.set_query(None);
            assert_eq!(bare.as_str(), endpoint, "{id}");
            assert_eq!(req.url.host_str(), locked_host(id), "{id}: host lock");
            assert!(
                req.body.is_none() && req.form.is_none() && req.poll.is_none(),
                "{id}: GET only, never POST"
            );
            let got = pairs(&req);
            let want: Vec<(String, String)> = expected
                .iter()
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .collect();
            assert_eq!(got, want, "{id}");
            assert!(
                !got.iter().any(|(key, _)| matches!(
                    key.as_str(),
                    "apiKey" | "highlight" | "semantic" | "page" | "cursor"
                )),
                "{id}"
            );
            let (name, value) = provider_credential(id, &keys()).unwrap().expect("keyed");
            if id.starts_with("newsapi_") {
                assert_eq!(name.as_str(), "x-api-key", "{id}");
                assert!(value == SENTINEL, "{id}: X-Api-Key carries the key");
            } else {
                assert_eq!(name.as_str(), "authorization", "{id}");
                assert!(
                    value == format!("Token {SENTINEL}"),
                    "{id}: Authorization: Token <key>"
                );
            }
            assert!(
                !req.url.as_str().contains(SENTINEL),
                "{id}: no key in the URL"
            );
        }
        // Bad optional inputs are refused before any request.
        for (id, args) in [
            (
                "newsapi_search",
                json!({"query": "Elon Musk", "from": "March 2026"}),
            ),
            (
                "newsapi_search",
                json!({"query": "Elon Musk", "sort_by": "date"}),
            ),
            (
                "newsapi_headlines",
                json!({"query": "Elon Musk", "category": "politics"}),
            ),
            (
                "courtlistener_case_search",
                json!({"query": "Elon Musk", "court": "ded; drop"}),
            ),
            (
                "courtlistener_judge_search",
                json!({"query": "Elon Musk", "court": "ded"}),
            ),
        ] {
            assert!(validate(id, &args).is_err(), "{id} {args}");
        }
        // Missing keys name the field and the env var.
        let missing = provider_credential("newsapi_search", &ProviderKeys::default())
            .unwrap_err()
            .to_string();
        assert_eq!(
            missing,
            "Enter the NewsAPI key on a News tool, or set NEWSAPI_API_KEY"
        );
        let missing = provider_credential("courtlistener_docket_search", &ProviderKeys::default())
            .unwrap_err()
            .to_string();
        assert!(missing.contains("COURTLISTENER_API_TOKEN"), "{missing}");
    }

    #[test]
    fn newsapi_fixture_rows_keep_title_date_source_url_and_snippet() {
        let mut rows: Vec<Value> = (0..25)
            .map(|n| {
                article(
                    &format!("Elon Musk story {n}"),
                    "Elon Musk said.",
                    &format!("https://www.reuters.com/{n}"),
                )
            })
            .collect();
        rows.push(json!({"title": "No link", "url": ""}));
        let body: Value = serde_json::from_str(&articles(&rows)).unwrap();
        let (value, truncated) = observations("newsapi_search", &body).unwrap();
        let results = value["results"].as_array().unwrap();
        assert_eq!(results.len(), NEWS_MAX_RESULTS);
        assert_eq!(NEWS_MAX_RESULTS, 10);
        assert!(truncated);
        assert_eq!(
            results[0],
            json!({"title": "Elon Musk story 0", "date": "2026-09-30T14:00:00Z", "source": "Reuters", "url": "https://www.reuters.com/0", "snippet": "Elon Musk said."})
        );
        assert_eq!(value["context"], "news");
    }

    #[test]
    fn courtlistener_fixture_rows_keep_case_court_date_url_and_snippet() {
        let body: Value = serde_json::from_str(&search(&[opinion(
            "Tornetta v. Musk",
            "Elon Musk compensation package rescinded.",
        )]))
        .unwrap();
        let (value, _) = observations("courtlistener_case_search", &body).unwrap();
        assert_eq!(
            value["results"][0],
            json!({"title": "Tornetta v. Musk", "date": "2024-01-30", "court": "District Court, D. Delaware", "docket_number": "1:22-cv-01234", "url": "https://www.courtlistener.com/opinion/123/example/", "snippet": "Elon Musk compensation package rescinded."})
        );
        let docket = json!({"results": [{"caseName": "Doe v. Musk", "absolute_url": "/docket/987/doe-v-musk/", "court": "S.D.N.Y.", "dateFiled": "2025-05-01", "recap_documents": [{"snippet": ""}, {"snippet": "Complaint against Elon Musk"}]}]});
        let (value, _) = observations("courtlistener_docket_search", &docket).unwrap();
        assert_eq!(
            value["results"][0]["url"],
            "https://www.courtlistener.com/docket/987/doe-v-musk/"
        );
        assert_eq!(
            value["results"][0]["snippet"],
            "Complaint against Elon Musk"
        );
        let judge = json!({"results": [{"name": "Kathaleen St. Jude McCormick", "absolute_url": "/person/1/kathaleen-mccormick/", "positions": [{"court_full_name": "Delaware Court of Chancery", "date_start": "2011-01-01", "position_type": "Chancellor"}]}]});
        let (value, _) = observations("courtlistener_judge_search", &judge).unwrap();
        assert_eq!(value["results"][0]["title"], "Kathaleen St. Jude McCormick");
        assert_eq!(value["results"][0]["court"], "Delaware Court of Chancery");
        // CourtListener keeps its own cap of 20 rows; NewsAPI's 10 does not apply.
        let many: Vec<Value> = (0..25)
            .map(|n| opinion(&format!("Case {n} v. Musk"), "Elon Musk"))
            .collect();
        let (value, truncated) = observations(
            "courtlistener_case_search",
            &serde_json::from_str(&search(&many)).unwrap(),
        )
        .unwrap();
        assert_eq!(
            (
                value["results"].as_array().unwrap().len(),
                LEGAL_MAX_RESULTS,
                truncated
            ),
            (20, 20, true)
        );
        assert_eq!(
            (
                max_results("newsapi_headlines"),
                max_results("courtlistener_docket_search")
            ),
            (10, 20)
        );
    }

    /// On the wire: both NewsAPI tools send a non-empty User-Agent. With no setting, or a
    /// blank one (Recon passes the saved setting, blank by default), it is the default;
    /// a custom value is sent as configured.
    #[tokio::test]
    async fn newsapi_requests_send_a_user_agent_and_a_blank_setting_falls_back_to_the_default() {
        let fixture = serve(Arc::new(|_: &str| {
            (
                200,
                articles(&[article(
                    "Elon Musk at Tesla",
                    "Elon Musk spoke.",
                    "https://www.reuters.com/a",
                )]),
            )
        }))
        .await;
        let executor = Executor::new().unwrap();
        let agent_of = |raw: &str| {
            raw.lines().find_map(|line| {
                line.to_ascii_lowercase()
                    .starts_with("user-agent:")
                    .then(|| line["user-agent:".len()..].trim().to_string())
            })
        };
        for tool in ["newsapi_search", "newsapi_headlines"] {
            for (setting, want) in [
                (None, crate::osint::DEFAULT_USER_AGENT),
                (Some(""), crate::osint::DEFAULT_USER_AGENT),
                (Some("   "), crate::osint::DEFAULT_USER_AGENT),
                (Some("Argos test@example.com"), "Argos test@example.com"),
            ] {
                let result = executor
                    .run_configured(tool, json!({"query": "Elon Musk"}), setting, &keys())
                    .await
                    .unwrap();
                assert_eq!(
                    result.status, "completed",
                    "{tool} {setting:?}: {:?}",
                    result.error
                );
                let raw = fixture.requests().last().cloned().unwrap();
                let sent = agent_of(&raw).unwrap_or_default();
                assert!(!sent.is_empty(), "{tool} {setting:?}: no empty User-Agent");
                assert_eq!(sent, want, "{tool} {setting:?}");
            }
        }
        assert_eq!(fixture.requests().len(), 8);
    }

    /// On the wire: the key travels only in its header, CourtListener gets Accept JSON,
    /// and a CourtListener 429 is returned once, not retried.
    #[tokio::test]
    async fn fixture_requests_carry_the_key_only_in_headers_and_429_is_not_retried() {
        let fixture = serve(Arc::new(|line: &str| {
            if line.contains("/v2/") {
                (
                    200,
                    articles(&[article(
                        "Elon Musk at Tesla",
                        "Elon Musk spoke.",
                        "https://www.reuters.com/a",
                    )]),
                )
            } else {
                (429, json!({"detail": "Request was throttled."}).to_string())
            }
        }))
        .await;
        let executor = Executor::new().unwrap();
        let news = executor
            .run_configured(
                "newsapi_search",
                json!({"query": "Elon Musk"}),
                None,
                &keys(),
            )
            .await
            .unwrap();
        assert_eq!(news.status, "completed", "{:?}", news.error);
        assert_eq!(
            news.observations["results"][0]["title"],
            "Elon Musk at Tesla"
        );
        let court = executor
            .run_configured(
                "courtlistener_case_search",
                json!({"query": "Elon Musk"}),
                None,
                &keys(),
            )
            .await
            .unwrap();
        assert_eq!(court.status, "rate_limited");
        assert!(
            court
                .error
                .as_deref()
                .unwrap_or("")
                .starts_with(COURTLISTENER_RATE_LIMIT),
            "{:?}",
            court.error
        );
        let requests = fixture.requests();
        assert_eq!(requests.len(), 2, "the 429 is not retried");
        let lower: Vec<String> = requests
            .iter()
            .map(|raw| raw.to_ascii_lowercase())
            .collect();
        let line = |raw: &str| raw.lines().next().unwrap_or("").to_string();
        assert!(
            line(&requests[0]).starts_with("GET /v2/everything?q=%22Elon+Musk%22"),
            "{}",
            line(&requests[0])
        );
        assert!(lower[0].contains("\r\nx-api-key: ") && !lower[0].contains("authorization:"));
        assert!(
            line(&requests[1]).starts_with("GET /api/rest/v4/search/?q=%22Elon+Musk%22&type=o"),
            "{}",
            line(&requests[1])
        );
        assert!(
            lower[1].contains("\r\nauthorization: token ")
                && lower[1].contains("\r\naccept: application/json")
        );
        for raw in &requests {
            assert!(!line(raw).contains(SENTINEL), "no key in the request line");
        }
        assert!(!news.source_url.contains(SENTINEL) && !court.source_url.contains(SENTINEL));
        assert_eq!(
            crate::osint::host_interval("courtlistener_docket_search"),
            COURTLISTENER_SPACING
        );
        assert!(COURTLISTENER_SPACING >= Duration::from_secs(12));
    }

    /// A provider body that echoes the key is stored redacted.
    #[tokio::test]
    async fn an_echoed_key_is_redacted_from_the_stored_body_and_error() {
        let fixture = serve(Arc::new(|_: &str| (401, json!({"status": "error", "code": "apiKeyInvalid", "message": format!("Key {SENTINEL} is invalid")}).to_string()))).await;
        let result = Executor::new()
            .unwrap()
            .run_configured(
                "newsapi_search",
                json!({"query": "Elon Musk"}),
                None,
                &keys(),
            )
            .await
            .unwrap();
        drop(fixture);
        assert_eq!(result.status, "failed");
        assert!(
            !result.raw.contains(SENTINEL) && result.raw.contains("[redacted]"),
            "{}",
            result.raw.len()
        );
        assert!(!result.error.unwrap_or_default().contains(SENTINEL));
    }

    #[test]
    fn newsapi_status_error_codes_read_clearly() {
        let invalid = newsapi_error(&json!({"status": "error", "code": "apiKeyInvalid", "message": "Your API key is invalid."})).unwrap();
        assert!(
            invalid.contains("rejected the API key") && invalid.contains("NEWSAPI_API_KEY"),
            "{invalid}"
        );
        let missing =
            newsapi_error(&json!({"status": "error", "code": "apiKeyMissing", "message": "x"}))
                .unwrap();
        assert!(missing.contains("Enter the NewsAPI key"), "{missing}");
        let limited =
            newsapi_error(&json!({"status": "error", "code": "rateLimited", "message": "x"}))
                .unwrap();
        assert!(
            limited.contains("rate limit") && limited.contains("100 requests"),
            "{limited}"
        );
        let other = newsapi_error(
            &json!({"status": "error", "code": "parameterInvalid", "message": "Bad from date."}),
        )
        .unwrap();
        assert_eq!(other, "NewsAPI error parameterInvalid: Bad from date.");
        assert!(newsapi_error(&json!({"status": "ok", "articles": []})).is_none());
    }

    #[test]
    fn exact_phrase_wraps_the_entity_once() {
        assert_eq!(exact_phrase("Elon Musk").unwrap(), "\"Elon Musk\"");
        assert_eq!(exact_phrase("\"Elon Musk\"").unwrap(), "\"Elon Musk\"");
        assert!(exact_phrase("Elon \"Musk").is_err());
    }

    /// Live smoke check with a real NewsAPI key (`NEWSAPI_API_KEY`). Never prints the key.
    #[tokio::test]
    #[ignore = "needs a real NEWSAPI_API_KEY and network"]
    async fn live_newsapi_smoke() {
        let key = std::env::var("NEWSAPI_API_KEY").unwrap_or_default();
        assert!(!key.trim().is_empty(), "set NEWSAPI_API_KEY");
        let keys = ProviderKeys {
            newsapi: key,
            ..ProviderKeys::default()
        };
        let result = Executor::new()
            .unwrap()
            .run_configured("newsapi_search", json!({"query": "Elon Musk"}), None, &keys)
            .await
            .unwrap();
        assert!(
            matches!(result.status.as_str(), "completed" | "no_results"),
            "{} {:?}",
            result.status,
            result.error
        );
    }

    /// Live smoke check with a real CourtListener token (`COURTLISTENER_API_TOKEN`).
    #[tokio::test]
    #[ignore = "needs a real COURTLISTENER_API_TOKEN and network"]
    async fn live_courtlistener_smoke() {
        let key = std::env::var("COURTLISTENER_API_TOKEN").unwrap_or_default();
        assert!(!key.trim().is_empty(), "set COURTLISTENER_API_TOKEN");
        let keys = ProviderKeys {
            courtlistener: key,
            ..ProviderKeys::default()
        };
        let result = Executor::new()
            .unwrap()
            .run_configured(
                "courtlistener_case_search",
                json!({"query": "Elon Musk"}),
                None,
                &keys,
            )
            .await
            .unwrap();
        assert!(
            matches!(result.status.as_str(), "completed" | "no_results"),
            "{} {:?}",
            result.status,
            result.error
        );
    }
}
