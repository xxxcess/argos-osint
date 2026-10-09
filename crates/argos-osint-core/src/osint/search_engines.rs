//! Explicit search-engine SERP scraping adapters via Firecrawl `/v2/scrape` (§5).
//!
//! Provides canonical tools:
//! - `firecrawl_google_search`
//! - `firecrawl_yandex_search`
//! - `firecrawl_mojeek_search`
//!
//! Uses DOM parsing via `scraper` to parse HTML SERP responses, extract grounded
//! public destination URLs and snippets, normalize wrappers, and detect genuine
//! zero-results or challenges/consent walls.

use std::collections::HashSet;
use std::net::IpAddr;

use anyhow::{anyhow, Result};
use chrono::Utc;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use url::Url;

pub const FIRECRAWL_GOOGLE_SEARCH: &str = "firecrawl_google_search";
pub const FIRECRAWL_YANDEX_SEARCH: &str = "firecrawl_yandex_search";
pub const FIRECRAWL_MOJEEK_SEARCH: &str = "firecrawl_mojeek_search";

pub const PARSER_VERSION: &str = "1.0";

/// A single extracted search result item.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct EngineSearchResult {
    pub query: String,
    pub engine: String,
    pub serp_url: String,
    pub fetched_time: String,
    pub rank: usize,
    pub destination: String,
    #[serde(default)]
    pub url: String,
    pub snippet: String,
    pub parser_version: String,
    pub outcome: String,
}

/// Constructs the SERP URL for an engine and query.
pub fn build_serp_url(engine_tool: &str, query: &str) -> Result<String> {
    match engine_tool {
        FIRECRAWL_GOOGLE_SEARCH => {
            let mut u = Url::parse("https://www.google.com/search")?;
            u.query_pairs_mut().append_pair("q", query);
            Ok(u.to_string())
        }
        FIRECRAWL_YANDEX_SEARCH => {
            let mut u = Url::parse("https://yandex.com/search/")?;
            u.query_pairs_mut().append_pair("text", query);
            Ok(u.to_string())
        }
        FIRECRAWL_MOJEEK_SEARCH => {
            let mut u = Url::parse("https://www.mojeek.com/search")?;
            u.query_pairs_mut().append_pair("q", query);
            Ok(u.to_string())
        }
        _ => Err(anyhow!("unsupported search engine tool: {engine_tool}")),
    }
}

/// Constructs the Firecrawl `/v2/scrape` request body for a SERP.
pub fn build_scrape_body(engine_tool: &str, query: &str) -> Result<Value> {
    let serp_url = build_serp_url(engine_tool, query)?;
    Ok(json!({
        "url": serp_url,
        "formats": ["html", "links"],
        "onlyMainContent": false
    }))
}

/// Normalizes and cleans a destination URL, rejecting search-engine internal links,
/// private networks, ads, and malformed wrappers.
pub fn normalize_destination_url(raw: &str, engine: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Handle Google redirect wrappers: /url?q=<target>&... or /url?esrc=...&q=<target>
    let dest_str = if engine == "google" && trimmed.contains("/url?") {
        if let Ok(u) = Url::parse(&format!("https://www.google.com{trimmed}")) {
            u.query_pairs()
                .find(|(k, _)| k == "q")
                .map(|(_, v)| v.into_owned())
                .unwrap_or_else(|| trimmed.to_string())
        } else if let Ok(u) = Url::parse(trimmed) {
            u.query_pairs()
                .find(|(k, _)| k == "q")
                .map(|(_, v)| v.into_owned())
                .unwrap_or_else(|| trimmed.to_string())
        } else {
            trimmed.to_string()
        }
    } else {
        trimmed.to_string()
    };

    let parsed = Url::parse(&dest_str).ok()?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return None;
    }

    let host_str = parsed.host_str()?.to_lowercase();
    if host_str.is_empty() || host_str == "localhost" {
        return None;
    }

    // Reject private IP destinations
    if let Ok(ip) = host_str.parse::<IpAddr>() {
        match ip {
            IpAddr::V4(v4) => {
                if v4.is_loopback() || v4.is_private() || v4.is_link_local() {
                    return None;
                }
            }
            IpAddr::V6(v6) => {
                if v6.is_loopback() {
                    return None;
                }
            }
        }
    }

    let is_engine_domain = |base: &str| -> bool {
        host_str == base
            || host_str.starts_with(&format!("{base}."))
            || host_str.ends_with(&format!(".{base}"))
            || host_str.contains(&format!(".{base}."))
    };

    // Reject search engine self-links, ads, and consent walls
    match engine {
        "google" => {
            if is_engine_domain("google")
                || is_engine_domain("googleadservices")
                || (is_engine_domain("youtube") && parsed.path() == "/redirect")
            {
                return None;
            }
        }
        "yandex" => {
            if is_engine_domain("yandex")
                || is_engine_domain("ya")
                || (is_engine_domain("kinopoisk") && parsed.path().contains("/clck/"))
            {
                return None;
            }
        }
        "mojeek" => {
            if is_engine_domain("mojeek") {
                return None;
            }
        }
        _ => {}
    }

    // Clean fragment
    let mut clean = parsed;
    clean.set_fragment(None);
    Some(clean.to_string())
}

/// Parses Google SERP HTML.
pub fn parse_google_serp(
    html_str: &str,
    query: &str,
    serp_url: &str,
    limit: usize,
) -> (Vec<EngineSearchResult>, String) {
    let lower = html_str.to_lowercase();
    if lower.contains("unusual traffic from your computer network")
        || lower.contains("consent.google.com")
        || lower.contains("before you continue to google")
        || lower.contains("recaptcha")
    {
        return (Vec::new(), "challenge".into());
    }

    if lower.contains("did not match any documents")
        || lower.contains("no results found for")
        || lower.contains("your search -") && lower.contains("- did not match")
    {
        return (Vec::new(), "zero_results".into());
    }

    let document = Html::parse_document(html_str);
    let mut results = Vec::new();
    let mut seen = HashSet::new();

    // Select container elements or search headings
    let item_selector = Selector::parse("div.g, div.tF2Cxc, div[data-snc]").unwrap();
    let a_selector = Selector::parse("a[href]").unwrap();
    let h3_selector = Selector::parse("h3").unwrap();
    let snippet_selector = Selector::parse("div.VwiC3b, span.aCOpRe, div.yXK7lf").unwrap();

    let now_str = Utc::now().to_rfc3339();

    for el in document.select(&item_selector) {
        if results.len() >= limit {
            break;
        }

        let link_el = match el
            .select(&a_selector)
            .find(|a| a.select(&h3_selector).next().is_some())
            .or_else(|| el.select(&a_selector).next())
        {
            Some(a) => a,
            None => continue,
        };

        let raw_href = match link_el.value().attr("href") {
            Some(h) => h,
            None => continue,
        };

        let dest = match normalize_destination_url(raw_href, "google") {
            Some(d) => d,
            None => continue,
        };

        if seen.contains(&dest) {
            continue;
        }
        seen.insert(dest.clone());

        let snippet = el
            .select(&snippet_selector)
            .next()
            .map(|s| s.text().collect::<Vec<_>>().join(" "))
            .unwrap_or_default()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");

        results.push(EngineSearchResult {
            query: query.to_string(),
            engine: "google".into(),
            serp_url: serp_url.to_string(),
            fetched_time: now_str.clone(),
            rank: results.len() + 1,
            destination: dest.clone(),
            url: dest,
            snippet,
            parser_version: PARSER_VERSION.into(),
            outcome: "valid".into(),
        });
    }

    // Fallback parser if structural classes were obfuscated by Google
    if results.is_empty() {
        for a_el in document.select(&a_selector) {
            if results.len() >= limit {
                break;
            }
            if a_el.select(&h3_selector).next().is_none() {
                continue;
            }
            let href = match a_el.value().attr("href") {
                Some(h) => h,
                None => continue,
            };
            if let Some(dest) = normalize_destination_url(href, "google") {
                if !seen.contains(&dest) {
                    seen.insert(dest.clone());
                    results.push(EngineSearchResult {
                        query: query.to_string(),
                        engine: "google".into(),
                        serp_url: serp_url.to_string(),
                        fetched_time: now_str.clone(),
                        rank: results.len() + 1,
                        destination: dest.clone(),
                        url: dest,
                        snippet: String::new(),
                        parser_version: PARSER_VERSION.into(),
                        outcome: "valid".into(),
                    });
                }
            }
        }
    }

    let outcome = if !results.is_empty() {
        "valid".into()
    } else {
        "unknown_html".into()
    };

    (results, outcome)
}

/// Parses Yandex SERP HTML.
pub fn parse_yandex_serp(
    html_str: &str,
    query: &str,
    serp_url: &str,
    limit: usize,
) -> (Vec<EngineSearchResult>, String) {
    let lower = html_str.to_lowercase();
    if lower.contains("smartcaptcha")
        || lower.contains("show that you're not a robot")
        || lower.contains("checkbox-captcha")
    {
        return (Vec::new(), "challenge".into());
    }

    if lower.contains("ничего не нашлось")
        || lower.contains("ничего не найдено")
        || lower.contains("not found")
    {
        return (Vec::new(), "zero_results".into());
    }

    let document = Html::parse_document(html_str);
    let mut results = Vec::new();
    let mut seen = HashSet::new();

    let item_selector = Selector::parse("li.serp-item, div.Organic, div.serp-item").unwrap();
    let link_selector = Selector::parse("a.OrganicTitle-Link, a.link_theme_outer, h2 a").unwrap();
    let snippet_selector =
        Selector::parse("div.OrganicTextContent, div.organic__text, div.Organic-Content").unwrap();
    let now_str = Utc::now().to_rfc3339();

    for el in document.select(&item_selector) {
        if results.len() >= limit {
            break;
        }

        let link_el = match el.select(&link_selector).next() {
            Some(l) => l,
            None => continue,
        };

        let raw_href = match link_el.value().attr("href") {
            Some(h) => h,
            None => continue,
        };

        let dest = match normalize_destination_url(raw_href, "yandex") {
            Some(d) => d,
            None => continue,
        };

        if seen.contains(&dest) {
            continue;
        }
        seen.insert(dest.clone());

        let snippet = el
            .select(&snippet_selector)
            .next()
            .map(|s| s.text().collect::<Vec<_>>().join(" "))
            .unwrap_or_default()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");

        results.push(EngineSearchResult {
            query: query.to_string(),
            engine: "yandex".into(),
            serp_url: serp_url.to_string(),
            fetched_time: now_str.clone(),
            rank: results.len() + 1,
            destination: dest.clone(),
            url: dest,
            snippet,
            parser_version: PARSER_VERSION.into(),
            outcome: "valid".into(),
        });
    }

    let outcome = if !results.is_empty() {
        "valid".into()
    } else {
        "unknown_html".into()
    };

    (results, outcome)
}

/// Parses Mojeek SERP HTML.
pub fn parse_mojeek_serp(
    html_str: &str,
    query: &str,
    serp_url: &str,
    limit: usize,
) -> (Vec<EngineSearchResult>, String) {
    let lower = html_str.to_lowercase();
    if lower.contains("too many requests")
        || lower.contains("access blocked")
        || lower.contains("captcha")
    {
        return (Vec::new(), "challenge".into());
    }

    if lower.contains("no results found")
        || lower.contains("did not match any documents")
        || lower.contains("0 results")
    {
        return (Vec::new(), "zero_results".into());
    }

    let document = Html::parse_document(html_str);
    let mut results = Vec::new();
    let mut seen = HashSet::new();

    let item_selector = Selector::parse(
        "ul.results-standard > li, div.results-standard > li, .results-standard li, li.result",
    )
    .unwrap();
    let link_selector = Selector::parse("a.title, a.ob, h2 a").unwrap();
    let snippet_selector = Selector::parse("p.s, p.snippet, .s").unwrap();
    let now_str = Utc::now().to_rfc3339();

    for el in document.select(&item_selector) {
        if results.len() >= limit {
            break;
        }

        let link_el = match el.select(&link_selector).next() {
            Some(l) => l,
            None => continue,
        };

        let raw_href = match link_el.value().attr("href") {
            Some(h) => h,
            None => continue,
        };

        let dest = match normalize_destination_url(raw_href, "mojeek") {
            Some(d) => d,
            None => continue,
        };

        if seen.contains(&dest) {
            continue;
        }
        seen.insert(dest.clone());

        let snippet = el
            .select(&snippet_selector)
            .next()
            .map(|s| s.text().collect::<Vec<_>>().join(" "))
            .unwrap_or_default()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");

        results.push(EngineSearchResult {
            query: query.to_string(),
            engine: "mojeek".into(),
            serp_url: serp_url.to_string(),
            fetched_time: now_str.clone(),
            rank: results.len() + 1,
            destination: dest.clone(),
            url: dest,
            snippet,
            parser_version: PARSER_VERSION.into(),
            outcome: "valid".into(),
        });
    }

    let outcome = if !results.is_empty() {
        "valid".into()
    } else {
        "unknown_html".into()
    };

    (results, outcome)
}

/// Parses the outer Firecrawl `/v2/scrape` JSON envelope and then parses the embedded SERP HTML.
pub fn parse_serp_response(
    engine_tool: &str,
    raw_json: &Value,
    query: &str,
    limit: usize,
) -> Result<(Value, bool)> {
    if raw_json.get("success").and_then(Value::as_bool) == Some(false) {
        let msg = raw_json
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("Firecrawl SERP scrape failed");
        return Err(anyhow!("{msg}"));
    }

    let data = raw_json.get("data").unwrap_or(raw_json);
    let html_content = data
        .get("html")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("Firecrawl response missing HTML format for SERP scrape"))?;

    let serp_url = data
        .get("metadata")
        .and_then(|m| m.get("sourceURL").or_else(|| m.get("url")))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .or_else(|| {
            if !query.is_empty() {
                build_serp_url(engine_tool, query).ok()
            } else {
                None
            }
        })
        .unwrap_or_else(|| build_serp_url(engine_tool, query).unwrap_or_default());

    let effective_query = if query.is_empty() {
        Url::parse(&serp_url)
            .ok()
            .and_then(|u| {
                u.query_pairs()
                    .find(|(k, _)| k == "q" || k == "text")
                    .map(|(_, v)| v.into_owned())
            })
            .unwrap_or_default()
    } else {
        query.to_string()
    };

    let (items, outcome) = match engine_tool {
        FIRECRAWL_GOOGLE_SEARCH => {
            parse_google_serp(html_content, &effective_query, &serp_url, limit)
        }
        FIRECRAWL_YANDEX_SEARCH => {
            parse_yandex_serp(html_content, &effective_query, &serp_url, limit)
        }
        FIRECRAWL_MOJEEK_SEARCH => {
            parse_mojeek_serp(html_content, &effective_query, &serp_url, limit)
        }
        _ => return Err(anyhow!("unsupported search engine tool: {engine_tool}")),
    };

    if outcome == "challenge" {
        return Err(anyhow!(
            "search engine challenge/consent encountered on {engine_tool}"
        ));
    }

    let truncated = items.len() >= limit;
    let engine_name = match engine_tool {
        FIRECRAWL_GOOGLE_SEARCH => "google",
        FIRECRAWL_YANDEX_SEARCH => "yandex",
        FIRECRAWL_MOJEEK_SEARCH => "mojeek",
        _ => "unknown",
    };

    let items_val = serde_json::to_value(&items)?;
    let obs = json!({
        "items": items_val,
        "results": items_val, // Compatible with generic extract_search_results
        "engine": engine_name,
        "query": effective_query,
        "serp_url": serp_url,
        "outcome": outcome,
        "parser_version": PARSER_VERSION,
    });

    Ok((obs, truncated))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_serp_url_construction() {
        let g = build_serp_url(FIRECRAWL_GOOGLE_SEARCH, "elon musk tesla").unwrap();
        assert_eq!(g, "https://www.google.com/search?q=elon+musk+tesla");

        let y = build_serp_url(FIRECRAWL_YANDEX_SEARCH, "spacex launch").unwrap();
        assert_eq!(y, "https://yandex.com/search/?text=spacex+launch");

        let m = build_serp_url(FIRECRAWL_MOJEEK_SEARCH, "open source intelligence").unwrap();
        assert_eq!(
            m,
            "https://www.mojeek.com/search?q=open+source+intelligence"
        );
    }

    #[test]
    fn test_google_serp_parser_valid_html() {
        let html = r#"
        <!DOCTYPE html>
        <html>
        <body>
            <div class="g">
                <a href="https://example.com/target-page">
                    <h3>Example Target Title</h3>
                </a>
                <div class="VwiC3b">This is the search snippet for example target.</div>
            </div>
            <div class="g">
                <a href="/url?q=https://secondary.com/page&amp;sa=U">
                    <h3>Secondary Page</h3>
                </a>
                <div class="VwiC3b">Snippet for secondary page.</div>
            </div>
            <!-- Internal link that should be filtered -->
            <div class="g">
                <a href="https://www.google.com/search?q=more">
                    <h3>More results</h3>
                </a>
            </div>
        </body>
        </html>
        "#;

        let (results, outcome) = parse_google_serp(
            html,
            "test query",
            "https://www.google.com/search?q=test",
            5,
        );
        assert_eq!(outcome, "valid");
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].destination, "https://example.com/target-page");
        assert_eq!(
            results[0].snippet,
            "This is the search snippet for example target."
        );
        assert_eq!(results[0].rank, 1);
        assert_eq!(results[1].destination, "https://secondary.com/page");
        assert_eq!(results[1].rank, 2);
    }

    #[test]
    fn test_google_serp_parser_zero_results() {
        let html = r#"
        <!DOCTYPE html>
        <html>
        <body>
            <div id="topstuff">
                <p>Your search - <b>kajshdkjashdkjahskd</b> - did not match any documents.</p>
            </div>
        </body>
        </html>
        "#;

        let (results, outcome) = parse_google_serp(
            html,
            "nonsense",
            "https://www.google.com/search?q=nonsense",
            5,
        );
        assert_eq!(outcome, "zero_results");
        assert!(results.is_empty());
    }

    #[test]
    fn test_google_serp_parser_challenge() {
        let html = r#"
        <!DOCTYPE html>
        <html>
        <body>
            <h1>Before you continue to Google</h1>
            <p>We use cookies and data to deliver and maintain Google services...</p>
        </body>
        </html>
        "#;

        let (results, outcome) =
            parse_google_serp(html, "query", "https://www.google.com/search?q=query", 5);
        assert_eq!(outcome, "challenge");
        assert!(results.is_empty());
    }

    #[test]
    fn test_yandex_serp_parser_valid_html() {
        let html = r#"
        <!DOCTYPE html>
        <html>
        <body>
            <li class="serp-item">
                <div class="Organic">
                    <h2><a class="OrganicTitle-Link" href="https://target-yandex.org/doc">Target Document</a></h2>
                    <div class="OrganicTextContent">Yandex snippet describing document.</div>
                </div>
            </li>
            <li class="serp-item">
                <div class="Organic">
                    <h2><a class="OrganicTitle-Link" href="https://yandex.ru/adv">Yandex Ad Link</a></h2>
                </div>
            </li>
        </body>
        </html>
        "#;

        let (results, outcome) = parse_yandex_serp(
            html,
            "yandex query",
            "https://yandex.com/search/?text=yandex",
            5,
        );
        assert_eq!(outcome, "valid");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].destination, "https://target-yandex.org/doc");
        assert_eq!(results[0].snippet, "Yandex snippet describing document.");
    }

    #[test]
    fn test_mojeek_serp_parser_valid_html() {
        let html = r#"
        <!DOCTYPE html>
        <html>
        <body>
            <ul class="results-standard">
                <li>
                    <h2><a class="title" href="https://mojeek-hit.com/info">Mojeek Result</a></h2>
                    <p class="s">Mojeek independent index snippet.</p>
                </li>
            </ul>
        </body>
        </html>
        "#;

        let (results, outcome) = parse_mojeek_serp(
            html,
            "mojeek query",
            "https://www.mojeek.com/search?q=mojeek",
            5,
        );
        assert_eq!(outcome, "valid");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].destination, "https://mojeek-hit.com/info");
        assert_eq!(results[0].snippet, "Mojeek independent index snippet.");
    }

    #[test]
    fn test_parse_serp_response_envelope() {
        let raw = json!({
            "success": true,
            "data": {
                "html": "<ul class=\"results-standard\"><li><a class=\"title\" href=\"https://target.com\">Title</a><p class=\"s\">Snippet</p></li></ul>"
            }
        });

        let (obs, truncated) =
            parse_serp_response(FIRECRAWL_MOJEEK_SEARCH, &raw, "target", 5).unwrap();
        assert!(!truncated);
        assert_eq!(obs["engine"], "mojeek");
        assert_eq!(obs["outcome"], "valid");
        assert_eq!(obs["items"][0]["destination"], "https://target.com/");
    }
}
