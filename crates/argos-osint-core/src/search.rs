//! Public web search. Research asks DuckDuckGo first. Wikipedia is used only
//! when DuckDuckGo fails or returns nothing. Page fetches refuse loopback,
//! link-local, and private addresses.

use std::net::{IpAddr, ToSocketAddrs};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use url::Url;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchHit {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// A user-configured public source. `url_template` must contain `{query}`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OsintSource {
    pub name: String,
    pub url_template: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_enabled() -> bool {
    true
}

#[derive(Clone, Debug)]
pub struct SourcePlan {
    pub internet: bool,
    pub wikipedia: bool,
    pub searx_url: Option<String>,
    pub extra: Vec<OsintSource>,
}

impl Default for SourcePlan {
    fn default() -> Self {
        Self {
            internet: true,
            wikipedia: true,
            searx_url: None,
            extra: Vec::new(),
        }
    }
}

/// Search every enabled public source. A source that returns no hits is skipped.
/// The case fails only when every enabled source returns no hits.
pub async fn research(query: &str, plan: &SourcePlan) -> Result<Vec<SearchHit>, String> {
    let query = query.trim();
    if query.is_empty() {
        return Err("empty search".into());
    }
    let mut hits = Vec::new();
    let mut errors = Vec::new();
    let mut attempted = 0usize;
    if plan.internet {
        attempted += 1;
        match duckduckgo(query).await {
            Ok(mut rows) if !rows.is_empty() => {
                for hit in &mut rows {
                    hit.title = format!("[web] {}", hit.title);
                }
                hits.extend(rows);
            }
            Ok(_) => errors.push("web: duckduckgo returned no hits".into()),
            Err(err) => errors.push(format!("web: {err}")),
        }
    }
    let duckduckgo_missed = plan.internet && hits.is_empty();
    if plan.wikipedia && (!plan.internet || duckduckgo_missed) {
        attempted += 1;
        match wikipedia(query).await {
            Ok(mut rows) => {
                for hit in &mut rows {
                    hit.title = format!("[wikipedia] {}", hit.title);
                }
                hits.extend(rows);
            }
            Err(err) => errors.push(format!("wikipedia: {err}")),
        }
    }
    for source in plan.extra.iter().filter(|source| source.enabled) {
        attempted += 1;
        match template_source(source, query).await {
            Ok(mut rows) => {
                for hit in &mut rows {
                    hit.title = format!("[{}] {}", source.name, hit.title);
                }
                hits.extend(rows);
            }
            Err(err) => errors.push(format!("{}: {err}", source.name)),
        }
    }
    combine_source_results(attempted, hits, errors)
}

/// Keep hits from the sources that found something. Fail only when none did.
pub fn combine_source_results(
    attempted: usize,
    hits: Vec<SearchHit>,
    errors: Vec<String>,
) -> Result<Vec<SearchHit>, String> {
    if !hits.is_empty() {
        return Ok(hits);
    }
    if attempted == 0 {
        return Err("no OSINT sources are enabled. Turn some on in OSINT Providers.".into());
    }
    if errors.is_empty() {
        Err("no sources returned hits".into())
    } else if errors.len() == attempted {
        Err(errors.join("; "))
    } else {
        Err(format!("no sources returned hits ({})", errors.join("; ")))
    }
}

pub fn parse_wikipedia_opensearch(value: &serde_json::Value) -> Vec<SearchHit> {
    let titles = value.get(1).and_then(|v| v.as_array());
    let snippets = value.get(2).and_then(|v| v.as_array());
    let urls = value.get(3).and_then(|v| v.as_array());
    let Some(titles) = titles else {
        return Vec::new();
    };
    titles
        .iter()
        .enumerate()
        .filter_map(|(i, title)| {
            let title = title.as_str()?.trim();
            if title.is_empty() {
                return None;
            }
            let url = urls
                .and_then(|rows| rows.get(i))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if url.is_empty() {
                return None;
            }
            let snippet = snippets
                .and_then(|rows| rows.get(i))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Some(SearchHit {
                title: title.to_string(),
                url,
                snippet,
            })
        })
        .take(5)
        .collect()
}

async fn wikipedia(query: &str) -> Result<Vec<SearchHit>, String> {
    let url = format!(
        "https://en.wikipedia.org/w/api.php?action=query&list=search&srlimit=5&utf8=1&format=json&srsearch={}",
        urlencoding::encode(query)
    );
    check_public_http(&url)?;
    let client = client()?;
    let value: serde_json::Value = client
        .get(&url)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|err| err.to_string())?
        .error_for_status()
        .map_err(|err| err.to_string())?
        .json()
        .await
        .map_err(|err| err.to_string())?;
    Ok(parse_wikipedia_search(&value))
}

/// Full-text Wikipedia search (`list=search`). OpenSearch only matches titles,
/// so a question such as "who is elon musk" comes back empty.
pub fn parse_wikipedia_search(value: &serde_json::Value) -> Vec<SearchHit> {
    let Some(rows) = value
        .get("query")
        .and_then(|query| query.get("search"))
        .and_then(|search| search.as_array())
    else {
        return Vec::new();
    };
    rows.iter()
        .filter_map(|item| {
            let title = item.get("title").and_then(|v| v.as_str())?.trim();
            if title.is_empty() {
                return None;
            }
            let snippet = strip_tags(item.get("snippet").and_then(|v| v.as_str()).unwrap_or(""));
            Some(SearchHit {
                url: wikipedia_url(title),
                title: title.to_string(),
                snippet,
            })
        })
        .take(5)
        .collect()
}

fn wikipedia_url(title: &str) -> String {
    let slug = title.replace(' ', "_");
    format!(
        "https://en.wikipedia.org/wiki/{}",
        urlencoding::encode(&slug)
    )
}

async fn template_source(source: &OsintSource, query: &str) -> Result<Vec<SearchHit>, String> {
    if !source.url_template.contains("{query}") {
        return Err("url template must contain {query}".into());
    }
    let url = source
        .url_template
        .replace("{query}", &urlencoding::encode(query));
    let page = fetch_page(&url).await?;
    let snippet: String = page.chars().take(500).collect();
    if snippet.trim().is_empty() {
        return Ok(Vec::new());
    }
    Ok(vec![SearchHit {
        title: source.name.clone(),
        url,
        snippet,
    }])
}

pub async fn web_search(query: &str, searx_url: Option<&str>) -> Result<Vec<SearchHit>, String> {
    let query = query.trim();
    if query.is_empty() {
        return Err("empty search".into());
    }
    if let Some(base) = searx_url.map(str::trim).filter(|s| !s.is_empty()) {
        return searx(base, query).await;
    }
    duckduckgo(query).await
}

async fn searx(base: &str, query: &str) -> Result<Vec<SearchHit>, String> {
    let base = base.trim_end_matches('/');
    let url = format!("{base}/search?q={}&format=json", urlencoding::encode(query));
    check_public_http(&url)?;
    let client = client()?;
    let resp = client.get(&url).send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("searx {}", resp.status()));
    }
    let v: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    let mut hits = Vec::new();
    let results = v
        .get("results")
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();
    for item in results.into_iter().take(8) {
        let url = item
            .get("url")
            .and_then(|u| u.as_str())
            .unwrap_or("")
            .to_string();
        if url.is_empty() {
            continue;
        }
        hits.push(SearchHit {
            title: item
                .get("title")
                .and_then(|t| t.as_str())
                .unwrap_or(&url)
                .to_string(),
            url,
            snippet: item
                .get("content")
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .to_string(),
        });
    }
    Ok(hits)
}

async fn duckduckgo(query: &str) -> Result<Vec<SearchHit>, String> {
    let url = format!(
        "https://api.duckduckgo.com/?format=json&no_html=1&skip_disambig=1&q={}",
        urlencoding::encode(query)
    );
    check_public_http(&url)?;
    let client = client()?;
    let value: serde_json::Value = client
        .get(&url)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| format!("duckduckgo {e}"))?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    Ok(parse_ddg_instant(&value))
}

/// DuckDuckGo's public instant-answer JSON. The HTML result pages no longer
/// include `result__a` links for this client.
pub fn parse_ddg_instant(value: &serde_json::Value) -> Vec<SearchHit> {
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

fn collect_ddg_topics(topics: &[serde_json::Value], hits: &mut Vec<SearchHit>) {
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
            title: strip_tags(&decode_entities(&cap[2])),
            url,
            snippet: strip_tags(&decode_entities(&cap[3])),
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

pub async fn fetch_page(raw: &str) -> Result<String, String> {
    let url = check_public_http(raw)?;
    let client = client()?;
    let resp = client
        .get(url.as_str())
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("{} {}", url, resp.status()));
    }
    let ctype = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    if !(ctype.starts_with("text/")
        || ctype.contains("json")
        || ctype.contains("xml")
        || ctype.is_empty())
    {
        return Err(format!("refusing non-text content type {ctype}"));
    }
    let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
    let slice = &bytes[..bytes.len().min(48_000)];
    let text = String::from_utf8_lossy(slice);
    let plain = if ctype.contains("html") || text.contains("<html") {
        strip_tags(&text)
    } else {
        text.to_string()
    };
    let collapsed = collapse_ws(&plain);
    Ok(collapsed.chars().take(8_000).collect())
}

pub fn check_public_http(raw: &str) -> Result<Url, String> {
    let url = Url::parse(raw.trim()).map_err(|e| e.to_string())?;
    if url.scheme() != "http" && url.scheme() != "https" {
        return Err("only http and https".into());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("urls with credentials are refused".into());
    }
    let port = url
        .port()
        .unwrap_or(if url.scheme() == "https" { 443 } else { 80 });
    if port != 80 && port != 443 {
        return Err("only ports 80 and 443".into());
    }
    let host = url.host_str().ok_or("missing host")?.to_string();
    let host_l = host.to_lowercase();
    if host_l == "localhost"
        || host_l.ends_with(".local")
        || host_l.ends_with(".internal")
        || host_l == "metadata.google.internal"
    {
        return Err(format!("refusing host {host}"));
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        if ip_blocked(ip) {
            return Err(format!("refusing address {ip}"));
        }
    }
    let addr = format!("{host}:{port}");
    let resolved = addr
        .to_socket_addrs()
        .map_err(|e| format!("dns {host}: {e}"))?;
    let mut any = false;
    for sock in resolved {
        any = true;
        if ip_blocked(sock.ip()) {
            return Err(format!("refusing {host}: it resolves to {}", sock.ip()));
        }
    }
    if !any {
        return Err(format!("dns {host} returned no addresses"));
    }
    Ok(url)
}

fn ip_blocked(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_unspecified()
                || v4.is_multicast()
                || o[0] == 0
                || (o[0] == 100 && (o[1] & 0xc0) == 64) // 100.64.0.0/10
                || (o[0] == 169 && o[1] == 254)
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return ip_blocked(IpAddr::V4(v4));
            }
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || is_ula(v6)
                || is_link_local_v6(v6)
        }
    }
}

fn is_ula(v6: std::net::Ipv6Addr) -> bool {
    (v6.segments()[0] & 0xfe00) == 0xfc00
}

fn is_link_local_v6(v6: std::net::Ipv6Addr) -> bool {
    (v6.segments()[0] & 0xffc0) == 0xfe80
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(25))
        .redirect(reqwest::redirect::Policy::limited(4))
        .user_agent("argos-osint/0.1 (public research)")
        .build()
        .map_err(|e| e.to_string())
}

pub fn strip_tags(html: &str) -> String {
    let script = regex::Regex::new(r"(?is)<script[^>]*>.*?</script>").unwrap();
    let style = regex::Regex::new(r"(?is)<style[^>]*>.*?</style>").unwrap();
    let without_script = script.replace_all(html, " ");
    let without = style.replace_all(&without_script, " ");
    let tags = regex::Regex::new(r"(?s)<[^>]+>").unwrap();
    collapse_ws(&tags.replace_all(&without, " "))
}

fn decode_entities(text: &str) -> String {
    text.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
}

fn collapse_ws(text: &str) -> String {
    let mut out = String::new();
    let mut space = false;
    for ch in text.chars() {
        if ch.is_whitespace() {
            space = true;
        } else {
            if space && !out.is_empty() {
                out.push(' ');
            }
            space = false;
            out.push(ch);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(title: &str) -> SearchHit {
        SearchHit {
            title: title.into(),
            url: "https://example.com".into(),
            snippet: "a public page".into(),
        }
    }

    #[test]
    fn one_empty_source_does_not_fail_the_report() {
        let kept =
            combine_source_results(2, vec![hit("harbor")], vec!["wikipedia: no hits".into()]);
        assert_eq!(kept.unwrap().len(), 1);
        let none = combine_source_results(2, Vec::new(), Vec::new()).unwrap_err();
        assert_eq!(none, "no sources returned hits");
        let all_failed = combine_source_results(
            2,
            Vec::new(),
            vec!["web: down".into(), "wikipedia: down".into()],
        )
        .unwrap_err();
        assert!(all_failed.contains("web: down"));
        assert!(all_failed.contains("wikipedia: down"));
    }

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
    fn parses_wikipedia_full_text_search() {
        let raw = r#"{"query":{"search":[{"title":"Elon Musk","snippet":"<span class=\"searchmatch\">Elon</span> Musk is a businessman"}]}}"#;
        let value: serde_json::Value = serde_json::from_str(raw).unwrap();
        let hits = parse_wikipedia_search(&value);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "Elon Musk");
        assert_eq!(hits[0].url, "https://en.wikipedia.org/wiki/Elon_Musk");
        assert!(hits[0].snippet.contains("Elon Musk is a businessman"));
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
        let value: serde_json::Value = serde_json::from_str(raw).unwrap();
        let hits = parse_ddg_instant(&value);
        assert_eq!(hits.len(), 3);
        assert_eq!(hits[0].title, "Elon Musk");
        assert!(hits.iter().all(|hit| !hit.url.contains("duckduckgo.com")));
        assert_eq!(hits[2].title, "SpaceX");
    }

    #[test]
    fn parses_wikipedia_opensearch() {
        let raw = r#"["ada",["Ada Lovelace"],["Mathematician"],["https://en.wikipedia.org/wiki/Ada_Lovelace"]]"#;
        let value: serde_json::Value = serde_json::from_str(raw).unwrap();
        let hits = parse_wikipedia_opensearch(&value);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "Ada Lovelace");
        assert!(hits[0].url.contains("Ada_Lovelace"));
    }

    #[test]
    fn blocks_private_and_metadata_targets() {
        assert!(check_public_http("http://127.0.0.1/").is_err());
        assert!(check_public_http("http://169.254.169.254/latest").is_err());
        assert!(check_public_http("https://user:pass@example.com/").is_err());
        assert!(check_public_http("file:///etc/passwd").is_err());
        assert!(check_public_http("http://localhost:11434/").is_err());
    }
}
