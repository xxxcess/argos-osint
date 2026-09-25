//! Public web search. SearXNG when it is configured, DuckDuckGo otherwise.
//! Brave and Tavily run in addition when a key is set.

use futures_util::future::join_all;
use serde_json::{json, Value};
use url::Url;

use super::{
    clip, get_json, keyed, merge_adapter_results, post_json, searx_base, tag, Job, MergeOutcome, SearchHit,
    SourcePlan,
};

pub async fn gather(query: &str, plan: &SourcePlan) -> Result<MergeOutcome, String> {
    let query = query.to_string();
    let mut jobs: Vec<Job> = Vec::new();
    let primary_query = query.clone();
    let primary_plan = plan.clone();
    jobs.push(Box::pin(async move {
        ("web", primary(&primary_query, &primary_plan).await)
    }));
    if let Some(key) = keyed(&plan.brave_key) {
        let brave_query = query.clone();
        jobs.push(Box::pin(async move {
            ("brave", brave(&brave_query, &key).await)
        }));
    }
    if let Some(key) = keyed(&plan.tavily_key) {
        let tavily_query = query.clone();
        jobs.push(Box::pin(async move {
            ("tavily", tavily(&tavily_query, &key).await)
        }));
    }
    merge_adapter_results(join_all(jobs).await)
}

async fn primary(query: &str, plan: &SourcePlan) -> Result<Vec<SearchHit>, String> {
    if let Some(base) = searx_base(plan) {
        match searx(&base, query, None).await {
            Ok(hits) if !hits.is_empty() => return Ok(tag(hits, "web")),
            Ok(_) => {}
            Err(err) => {
                return match duckduckgo(query).await {
                    Ok(hits) if !hits.is_empty() => Ok(tag(hits, "web")),
                    Ok(_) => Err(err),
                    Err(ddg) => Err(format!("{err}; {ddg}")),
                };
            }
        }
    }
    duckduckgo(query).await.map(|hits| tag(hits, "web"))
}

pub(crate) async fn searx(
    base: &str,
    query: &str,
    category: Option<&str>,
) -> Result<Vec<SearchHit>, String> {
    let base = base.trim_end_matches('/');
    let mut url = format!("{base}/search?q={}&format=json", urlencoding::encode(query));
    if let Some(category) = category {
        url.push_str("&categories=");
        url.push_str(category);
    }
    let value = get_json(&url).await?;
    Ok(parse_searx(&value))
}

pub(crate) fn parse_searx(value: &Value) -> Vec<SearchHit> {
    let results = value
        .get("results")
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();
    let mut hits = Vec::new();
    for item in results.into_iter().take(8) {
        let url = item
            .get("url")
            .and_then(|u| u.as_str())
            .unwrap_or("")
            .trim();
        if !url.starts_with("http") {
            continue;
        }
        hits.push(SearchHit {
            title: item
                .get("title")
                .and_then(|t| t.as_str())
                .filter(|title| !title.trim().is_empty())
                .unwrap_or(url)
                .trim()
                .to_string(),
            url: url.to_string(),
            snippet: clip(
                item.get("content").and_then(|t| t.as_str()).unwrap_or(""),
                360,
            ),
        });
    }
    hits
}

async fn brave(query: &str, key: &str) -> Result<Vec<SearchHit>, String> {
    let url = format!(
        "https://api.search.brave.com/res/v1/web/search?q={}&count=8",
        urlencoding::encode(query)
    );
    let value = super::get_json_headers(&url, &[("X-Subscription-Token", key)])
        .await
        .map_err(|err| super::redact(&err, key))?;
    Ok(tag(parse_brave(&value), "brave"))
}

pub(crate) fn parse_brave(value: &Value) -> Vec<SearchHit> {
    let Some(rows) = value.pointer("/web/results").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    rows.iter()
        .filter_map(|item| {
            let url = item
                .get("url")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            if !url.starts_with("http") {
                return None;
            }
            let title = item
                .get("title")
                .and_then(|v| v.as_str())
                .filter(|title| !title.trim().is_empty())
                .unwrap_or(url);
            Some(SearchHit {
                title: title.trim().to_string(),
                url: url.to_string(),
                snippet: clip(
                    item.get("description")
                        .and_then(|v| v.as_str())
                        .unwrap_or(""),
                    360,
                ),
            })
        })
        .take(8)
        .collect()
}

async fn tavily(query: &str, key: &str) -> Result<Vec<SearchHit>, String> {
    let value = post_json(
        "https://api.tavily.com/search",
        &json!({
            "api_key": key,
            "query": query,
            "max_results": 8,
            "search_depth": "basic"
        }),
        &[],
    )
    .await
    .map_err(|err| super::redact(&err, key))?;
    Ok(tag(parse_tavily(&value), "tavily"))
}

pub(crate) fn parse_tavily(value: &Value) -> Vec<SearchHit> {
    let Some(rows) = value.get("results").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    rows.iter()
        .filter_map(|item| {
            let url = item
                .get("url")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            if !url.starts_with("http") {
                return None;
            }
            let title = item
                .get("title")
                .and_then(|v| v.as_str())
                .filter(|title| !title.trim().is_empty())
                .unwrap_or(url);
            Some(SearchHit {
                title: title.trim().to_string(),
                url: url.to_string(),
                snippet: clip(
                    item.get("content").and_then(|v| v.as_str()).unwrap_or(""),
                    360,
                ),
            })
        })
        .take(8)
        .collect()
}

async fn duckduckgo(query: &str) -> Result<Vec<SearchHit>, String> {
    let url = format!(
        "https://api.duckduckgo.com/?format=json&no_html=1&skip_disambig=1&q={}",
        urlencoding::encode(query)
    );
    let value = get_json(&url)
        .await
        .map_err(|err| format!("duckduckgo {err}"))?;
    Ok(parse_ddg_instant(&value))
}

/// DuckDuckGo's public instant-answer JSON. The HTML result pages no longer
/// include `result__a` links for this client.
pub fn parse_ddg_instant(value: &Value) -> Vec<SearchHit> {
    let mut hits = Vec::new();
    let abstract_url = value
        .get("AbstractURL")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let abstract_text = value
        .get("AbstractText")
        .and_then(|v| v.as_str())
        .or_else(|| value.get("Abstract").and_then(|v| v.as_str()))
        .unwrap_or("")
        .trim();
    if !abstract_url.is_empty() && !abstract_text.is_empty() {
        let title = value
            .get("Heading")
            .and_then(|v| v.as_str())
            .filter(|text| !text.trim().is_empty())
            .unwrap_or(abstract_url);
        hits.push(SearchHit {
            title: title.to_string(),
            url: abstract_url.to_string(),
            snippet: abstract_text.to_string(),
        });
    }
    if let Some(topics) = value.get("RelatedTopics").and_then(|v| v.as_array()) {
        collect_ddg_topics(topics, &mut hits);
    }
    hits.truncate(8);
    hits
}

fn collect_ddg_topics(topics: &[Value], hits: &mut Vec<SearchHit>) {
    for topic in topics {
        if hits.len() >= 8 {
            return;
        }
        if let Some(nested) = topic.get("Topics").and_then(|v| v.as_array()) {
            collect_ddg_topics(nested, hits);
            continue;
        }
        let url = topic
            .get("FirstURL")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        let text = topic
            .get("Text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if url.is_empty() || text.is_empty() || !is_public_web_result(url) {
            continue;
        }
        if hits.iter().any(|hit| hit.url == url) {
            continue;
        }
        let title = text.split(" - ").next().unwrap_or(text).trim();
        hits.push(SearchHit {
            title: title.to_string(),
            url: url.to_string(),
            snippet: text.to_string(),
        });
    }
}

fn is_public_web_result(url: &str) -> bool {
    let Ok(parsed) = Url::parse(url) else {
        return false;
    };
    let host = parsed.host_str().unwrap_or("").to_lowercase();
    if host.is_empty() || host.ends_with("duckduckgo.com") {
        return false;
    }
    parsed.scheme() == "http" || parsed.scheme() == "https"
}

pub fn parse_ddg_html(html: &str) -> Vec<SearchHit> {
    let re = regex::Regex::new(
        r#"(?s)<a[^>]*class="result__a"[^>]*href="([^"]+)"[^>]*>(.*?)</a>.*?<a[^>]*class="result__snippet"[^>]*>(.*?)</a>"#,
    )
    .expect("ddg regex");
    let mut hits = Vec::new();
    for cap in re.captures_iter(html) {
        let href = decode_entities(&cap[1]);
        let url = unwrap_ddg_redirect(&href);
        if url.is_empty() {
            continue;
        }
        hits.push(SearchHit {
            title: super::strip_tags(&decode_entities(&cap[2])),
            url,
            snippet: super::strip_tags(&decode_entities(&cap[3])),
        });
        if hits.len() == 8 {
            break;
        }
    }
    hits
}

fn unwrap_ddg_redirect(href: &str) -> String {
    if let Ok(url) = Url::parse(href) {
        if let Some((_, uddg)) = url.query_pairs().find(|(k, _)| k == "uddg") {
            return uddg.to_string();
        }
    }
    if let Some(idx) = href.find("uddg=") {
        let rest = &href[idx + 5..];
        let end = rest.find('&').unwrap_or(rest.len());
        return urlencoding::decode(&rest[..end])
            .unwrap_or(std::borrow::Cow::Borrowed(&rest[..end]))
            .to_string();
    }
    href.to_string()
}

fn decode_entities(text: &str) -> String {
    text.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ddg_and_unwraps_redirect() {
        let html = r#"
        <a class="result__a" href="https://duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fa&amp;rut=1">Example <b>A</b></a>
        <a class="result__snippet">A snippet &amp; more</a>
        "#;
        let hits = parse_ddg_html(html);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].url, "https://example.com/a");
        assert_eq!(hits[0].title, "Example A");
        assert!(hits[0].snippet.contains("snippet & more"));
    }

    #[test]
    fn parses_duckduckgo_instant_answer() {
        let raw = r#"{
            "Heading":"Elon Musk",
            "AbstractText":"Elon Musk leads Tesla and SpaceX.",
            "AbstractURL":"https://en.wikipedia.org/wiki/Elon_Musk",
            "RelatedTopics":[
                {"Text":"Elon Musk Category","FirstURL":"https://duckduckgo.com/c/Elon_Musk"},
                {"Text":"Tesla, Inc. - car company","FirstURL":"https://en.wikipedia.org/wiki/Tesla,_Inc."},
                {"Name":"More","Topics":[{"Text":"SpaceX - launch company","FirstURL":"https://en.wikipedia.org/wiki/SpaceX"}]}
            ]
        }"#;
        let value: Value = serde_json::from_str(raw).unwrap();
        let hits = parse_ddg_instant(&value);
        assert_eq!(hits.len(), 3);
        assert_eq!(hits[0].title, "Elon Musk");
        assert!(hits.iter().all(|hit| !hit.url.contains("duckduckgo.com")));
        assert_eq!(hits[2].title, "SpaceX");
    }

    #[test]
    fn parses_searx_brave_and_tavily() {
        let searx: Value = serde_json::from_str(
            r#"{"results":[{"title":"Harbor","url":"https://example.com/a","content":"A page"}]}"#,
        )
        .unwrap();
        assert_eq!(parse_searx(&searx)[0].title, "Harbor");
        let brave: Value = serde_json::from_str(
            r#"{"web":{"results":[{"title":"Brave hit","url":"https://example.com/b","description":"desc"}]}}"#,
        )
        .unwrap();
        assert_eq!(parse_brave(&brave)[0].snippet, "desc");
        let tavily: Value = serde_json::from_str(
            r#"{"results":[{"title":"Tavily hit","url":"https://example.com/c","content":"body"}]}"#,
        )
        .unwrap();
        assert_eq!(parse_tavily(&tavily)[0].title, "Tavily hit");
    }
}
