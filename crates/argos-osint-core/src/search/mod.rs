//! Public research. An investigation runs the stages that apply to the
//! question: facts, web, news, and — when the text contains them — domain,
//! social, and identity. One adapter that is down does not fail the case.
//! Page fetches still refuse loopback, link-local, and private addresses.

mod domain;
mod facts;
mod identity;
mod news;
mod query;
mod social;
mod web;

use std::future::Future;
use std::net::{IpAddr, ToSocketAddrs};
use std::pin::Pin;
use std::time::Duration;

use futures_util::future::join_all;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;

pub use facts::{parse_wikipedia_opensearch, parse_wikipedia_search};
pub use query::TextQuery;
pub use web::{parse_ddg_html, parse_ddg_instant};

pub(crate) type Job =
    Pin<Box<dyn Future<Output = (&'static str, Result<Vec<SearchHit>, String>)> + Send>>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchHit {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// How one named adapter finished inside a stage merge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdapterStatus {
    pub name: String,
    pub state: AdapterState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AdapterState {
    Ok { hits: usize },
    Empty,
    Error { message: String },
}

impl AdapterStatus {
    pub fn summary_line(&self) -> String {
        match &self.state {
            AdapterState::Ok { hits } => format!("adapter {}: ok ({hits})", self.name),
            AdapterState::Empty => format!("adapter {}: empty", self.name),
            AdapterState::Error { message } => {
                format!("adapter {}: error ({message})", self.name)
            }
        }
    }
}

/// Hits plus per-adapter outcomes from one stage merge.
#[derive(Clone, Debug, Default)]
pub struct MergeOutcome {
    pub hits: Vec<SearchHit>,
    pub adapters: Vec<AdapterStatus>,
}

/// Full research result: merged hits and every adapter status collected.
#[derive(Clone, Debug, Default)]
pub struct ResearchOutcome {
    pub hits: Vec<SearchHit>,
    pub adapters: Vec<AdapterStatus>,
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

/// Which public stages run for one question. Keys stay empty unless the user
/// set them. A stage flag that is on still skips adapters the question does
/// not support: domain tools need a domain, social needs a topic or handle,
/// and identity needs a handle or email.
#[derive(Clone, Debug)]
pub struct SourcePlan {
    pub facts: bool,
    pub web: bool,
    pub news: bool,
    pub domain: bool,
    pub social: bool,
    pub identity: bool,
    pub searx_url: Option<String>,
    pub brave_key: Option<String>,
    pub tavily_key: Option<String>,
    pub youtube_key: Option<String>,
    pub github_token: Option<String>,
    pub extra: Vec<OsintSource>,
}

impl Default for SourcePlan {
    fn default() -> Self {
        Self {
            facts: true,
            web: true,
            news: true,
            domain: true,
            social: true,
            identity: true,
            searx_url: None,
            brave_key: None,
            tavily_key: None,
            youtube_key: None,
            github_token: None,
            extra: Vec::new(),
        }
    }
}

struct Gates {
    facts: bool,
    web: bool,
    news: bool,
    domain: bool,
    social: bool,
    identity: bool,
}

fn gates(parsed: &TextQuery, plan: &SourcePlan) -> Gates {
    Gates {
        facts: plan.facts,
        web: plan.web,
        news: plan.news,
        domain: plan.domain && !parsed.domains.is_empty(),
        social: plan.social && parsed.has_social(),
        identity: plan.identity && parsed.has_identity(),
    }
}

fn any_enabled(plan: &SourcePlan) -> bool {
    plan.facts
        || plan.web
        || plan.news
        || plan.domain
        || plan.social
        || plan.identity
        || plan.extra.iter().any(|source| source.enabled)
}

/// Adapter names `research` will call for this question and plan.
/// Domain names are absent when the question has no domain, even if that
/// stage is enabled. YouTube, Brave, and Tavily appear only when a key is set.
pub fn scheduled_adapters(query: &str, plan: &SourcePlan) -> Vec<String> {
    let parsed = TextQuery::extract(query);
    let gates = gates(&parsed, plan);
    let mut names = Vec::new();
    if gates.facts {
        names.extend(["wikipedia".into(), "wikidata".into()]);
    }
    if gates.web {
        if searx_base(plan).is_some() {
            names.push("searx".into());
        } else {
            names.push("duckduckgo".into());
        }
        if keyed(&plan.brave_key).is_some() {
            names.push("brave".into());
        }
        if keyed(&plan.tavily_key).is_some() {
            names.push("tavily".into());
        }
    }
    if gates.news {
        if searx_base(plan).is_some() {
            names.push("searx-news".into());
        }
        names.push("gdelt".into());
    }
    if gates.domain {
        names.extend([
            "rdap".into(),
            "crt.sh".into(),
            "doh".into(),
            "wayback".into(),
            "internetdb".into(),
        ]);
    }
    if gates.social {
        names.extend(["bluesky".into(), "hn".into(), "mastodon".into()]);
        if keyed(&plan.youtube_key).is_some() {
            names.push("youtube".into());
        }
    }
    if gates.identity {
        names.push("github".into());
        names.push("leakcheck".into());
    }
    for source in plan.extra.iter().filter(|source| source.enabled) {
        names.push(format!("extra:{}", source.name));
    }
    names
}

/// Search every stage that applies. A dead adapter is recorded and skipped.
/// The case fails only when every stage that actually ran returns nothing.
pub async fn research(query: &str, plan: &SourcePlan) -> Result<ResearchOutcome, String> {
    let query = query.trim();
    if query.is_empty() {
        return Err("empty search".into());
    }
    let parsed = TextQuery::extract(query);
    let gates = gates(&parsed, plan);
    if !gates.facts
        && !gates.web
        && !gates.news
        && !gates.domain
        && !gates.social
        && !gates.identity
        && !plan.extra.iter().any(|source| source.enabled)
    {
        if any_enabled(plan) {
            return Err("no sources applied to this query".into());
        }
        return Err("no OSINT sources are enabled. Turn some on in OSINT Providers.".into());
    }

    type StageFut = Pin<Box<dyn Future<Output = Result<MergeOutcome, String>> + Send>>;
    let mut facts_web: Vec<StageFut> = Vec::new();
    let mut other: Vec<StageFut> = Vec::new();
    if gates.facts {
        let terms = parsed.fact_terms();
        facts_web.push(Box::pin(facts::gather(terms)));
    }
    if gates.web {
        let q = query.to_string();
        let plan = plan.clone();
        facts_web.push(Box::pin(async move { web::gather(&q, &plan).await }));
    }
    if gates.news {
        let q = query.to_string();
        let searx = plan.searx_url.clone();
        other.push(Box::pin(news::gather(q, searx)));
    }
    if gates.domain {
        other.push(Box::pin(domain::gather(parsed.domains.clone())));
    }
    if gates.social {
        let q = parsed.social_query();
        let youtube = keyed(&plan.youtube_key);
        other.push(Box::pin(social::gather(q, youtube)));
    }
    if gates.identity {
        let terms = parsed.identity_terms();
        let token = keyed(&plan.github_token);
        other.push(Box::pin(identity::gather(terms, token)));
    }
    for source in plan.extra.iter().filter(|source| source.enabled).cloned() {
        let q = query.to_string();
        other.push(Box::pin(async move {
            match template_source(&source, &q).await {
                Ok(rows) => {
                    let hits = tag(rows, &source.name);
                    let state = if hits.is_empty() {
                        AdapterState::Empty
                    } else {
                        AdapterState::Ok { hits: hits.len() }
                    };
                    Ok(MergeOutcome {
                        hits,
                        adapters: vec![AdapterStatus {
                            name: format!("extra:{}", source.name),
                            state,
                        }],
                    })
                }
                Err(err) => Err(format!("{}: {err}", source.name)),
            }
        }));
    }

    let mut hits = Vec::new();
    let mut adapters = Vec::new();
    let mut errors = Vec::new();
    let mut attempted = 0usize;
    let mut pivot_pool = Vec::new();

    for result in join_all(facts_web).await {
        attempted += 1;
        match result {
            Ok(outcome) => {
                pivot_pool.extend(outcome.hits.iter().cloned());
                hits.extend(outcome.hits);
                adapters.extend(outcome.adapters);
            }
            Err(err) => errors.push(err),
        }
    }
    for result in join_all(other).await {
        attempted += 1;
        match result {
            Ok(outcome) => {
                hits.extend(outcome.hits);
                adapters.extend(outcome.adapters);
            }
            Err(err) => errors.push(err),
        }
    }

    if plan.domain {
        let pivoted = pivoted_domains(&pivot_pool, &parsed.domains);
        if !pivoted.is_empty() {
            attempted += 1;
            match domain::gather(pivoted).await {
                Ok(outcome) => {
                    hits.extend(outcome.hits);
                    adapters.extend(outcome.adapters);
                }
                Err(err) => errors.push(err),
            }
        }
    }

    combine_source_results(attempted, hits, errors, adapters)
}

pub async fn web_search(query: &str, plan: &SourcePlan) -> Result<Vec<SearchHit>, String> {
    let query = query.trim();
    if query.is_empty() {
        return Err("empty search".into());
    }
    Ok(web::gather(query, plan).await?.hits)
}

pub async fn news_search(query: &str, plan: &SourcePlan) -> Result<Vec<SearchHit>, String> {
    let query = query.trim();
    if query.is_empty() {
        return Err("empty search".into());
    }
    Ok(news::gather(query.to_string(), plan.searx_url.clone()).await?.hits)
}

pub async fn domain_lookup(query: &str, _plan: &SourcePlan) -> Result<Vec<SearchHit>, String> {
    let domains = TextQuery::extract(query).domains;
    if domains.is_empty() {
        return Ok(Vec::new());
    }
    Ok(domain::gather(domains).await?.hits)
}

pub async fn social_search(query: &str, plan: &SourcePlan) -> Result<Vec<SearchHit>, String> {
    let query = query.trim();
    if query.is_empty() {
        return Err("empty search".into());
    }
    Ok(social::gather(query.to_string(), keyed(&plan.youtube_key)).await?.hits)
}

pub async fn identity_lookup(query: &str, plan: &SourcePlan) -> Result<Vec<SearchHit>, String> {
    let parsed = TextQuery::extract(query);
    let mut terms = parsed.identity_terms();
    if terms.is_empty() {
        let query = query.trim();
        if !query.is_empty() {
            terms.push(query.to_string());
        }
    }
    if terms.is_empty() {
        return Ok(Vec::new());
    }
    Ok(identity::gather(terms, keyed(&plan.github_token)).await?.hits)
}

/// Keep hits from the sources that found something. Fail only when none did.
pub fn combine_source_results(
    attempted: usize,
    hits: Vec<SearchHit>,
    errors: Vec<String>,
    adapters: Vec<AdapterStatus>,
) -> Result<ResearchOutcome, String> {
    if !hits.is_empty() {
        return Ok(ResearchOutcome { hits, adapters });
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

pub(crate) fn merge_adapter_results(
    parts: Vec<(&str, Result<Vec<SearchHit>, String>)>,
) -> Result<MergeOutcome, String> {
    let mut hits = Vec::new();
    let mut adapters = Vec::new();
    let mut errors = Vec::new();
    for (name, result) in parts {
        match result {
            Ok(rows) if rows.is_empty() => {
                adapters.push(AdapterStatus {
                    name: name.to_string(),
                    state: AdapterState::Empty,
                });
            }
            Ok(rows) => {
                adapters.push(AdapterStatus {
                    name: name.to_string(),
                    state: AdapterState::Ok { hits: rows.len() },
                });
                hits.extend(rows);
            }
            Err(err) => {
                adapters.push(AdapterStatus {
                    name: name.to_string(),
                    state: AdapterState::Error {
                        message: err.clone(),
                    },
                });
                errors.push(format!("{name}: {err}"));
            }
        }
    }
    if !hits.is_empty() || errors.is_empty() {
        Ok(MergeOutcome { hits, adapters })
    } else {
        Err(errors.join("; "))
    }
}

/// Host from a hit URL when it is a public domain (not loopback/private).
pub fn public_host_from_url(raw: &str) -> Option<String> {
    let url = Url::parse(raw.trim()).ok()?;
    if url.scheme() != "http" && url.scheme() != "https" {
        return None;
    }
    let host = url.host_str()?.to_string();
    let host_l = host.to_ascii_lowercase();
    if host_l == "localhost"
        || host_l.ends_with(".local")
        || host_l.ends_with(".internal")
        || host_l == "metadata.google.internal"
    {
        return None;
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        if ip_blocked(ip) {
            return None;
        }
        return Some(host_l);
    }
    query::normalize_domain(&host)
}

/// Up to three public domains from hit URLs that are not already in the query.
pub(crate) fn pivoted_domains(hits: &[SearchHit], already: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for hit in hits {
        let Some(domain) = public_host_from_url(&hit.url) else {
            continue;
        };
        if already
            .iter()
            .any(|item| item.eq_ignore_ascii_case(&domain))
        {
            continue;
        }
        if out
            .iter()
            .any(|item: &String| item.eq_ignore_ascii_case(&domain))
        {
            continue;
        }
        out.push(domain);
        if out.len() >= 3 {
            break;
        }
    }
    out
}


pub(crate) fn tag(mut hits: Vec<SearchHit>, label: &str) -> Vec<SearchHit> {
    let prefix = format!("[{label}] ");
    for hit in &mut hits {
        if hit.title.trim().is_empty() {
            hit.title = label.to_string();
        }
        if !hit.title.starts_with(&prefix) {
            hit.title = format!("{prefix}{}", hit.title);
        }
    }
    hits
}

pub(crate) fn clip(text: &str, max: usize) -> String {
    let text = collapse_ws(text);
    if text.chars().count() <= max {
        return text;
    }
    let head: String = text.chars().take(max).collect();
    format!("{head}…")
}

pub(crate) fn keyed(value: &Option<String>) -> Option<String> {
    value
        .as_ref()
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
}

pub(crate) fn searx_base(plan: &SourcePlan) -> Option<String> {
    keyed(&plan.searx_url)
}

pub(crate) fn redact(message: &str, secret: &str) -> String {
    let secret = secret.trim();
    if secret.is_empty() {
        message.to_string()
    } else {
        message.replace(secret, "••••")
    }
}

pub(crate) struct RawHttp {
    pub status: u16,
    pub bytes: Vec<u8>,
}

pub(crate) async fn get_json(url: &str) -> Result<Value, String> {
    get_json_headers(url, &[]).await
}

pub(crate) async fn get_json_headers(url: &str, headers: &[(&str, &str)]) -> Result<Value, String> {
    let accept = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("accept"))
        .map(|(_, value)| *value)
        .unwrap_or("application/json");
    let filtered: Vec<(&str, &str)> = headers
        .iter()
        .copied()
        .filter(|(name, _)| !name.eq_ignore_ascii_case("accept"))
        .collect();
    let raw = request(reqwest::Method::GET, url, None, accept, &filtered).await?;
    parse_json_ok(raw)
}

pub(crate) async fn post_json(
    url: &str,
    body: &Value,
    headers: &[(&str, &str)],
) -> Result<Value, String> {
    let raw = request(
        reqwest::Method::POST,
        url,
        Some(body),
        "application/json",
        headers,
    )
    .await?;
    parse_json_ok(raw)
}

pub(crate) fn parse_json_ok(raw: RawHttp) -> Result<Value, String> {
    if !(200..300).contains(&raw.status) {
        return Err(format!("http {}", raw.status));
    }
    serde_json::from_slice(&raw.bytes).map_err(|err| err.to_string())
}

pub(crate) async fn request(
    method: reqwest::Method,
    url: &str,
    body: Option<&Value>,
    accept: &str,
    headers: &[(&str, &str)],
) -> Result<RawHttp, String> {
    let url = check_public_http(url)?;
    let client = client()?;
    let mut req = match method {
        reqwest::Method::POST => client.post(url.as_str()),
        _ => client.get(url.as_str()),
    };
    if let Some(body) = body {
        req = req.json(body);
    }
    req = req.header(reqwest::header::ACCEPT, accept);
    for (name, value) in headers {
        req = req.header(*name, *value);
    }
    let resp = req.send().await.map_err(|err| err.to_string())?;
    let status = resp.status().as_u16();
    let bytes = resp.bytes().await.map_err(|err| err.to_string())?;
    if bytes.len() > 512_000 {
        return Err("response too large".into());
    }
    Ok(RawHttp {
        status,
        bytes: bytes.to_vec(),
    })
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

pub fn ip_blocked(ip: IpAddr) -> bool {
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

pub(crate) fn collapse_ws(text: &str) -> String {
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
        let kept = combine_source_results(
            2,
            vec![hit("harbor")],
            vec!["wikipedia: no hits".into()],
            Vec::new(),
        );
        assert_eq!(kept.unwrap().hits.len(), 1);
        let none = combine_source_results(2, Vec::new(), Vec::new(), Vec::new()).unwrap_err();
        assert_eq!(none, "no sources returned hits");
        let all_failed = combine_source_results(
            2,
            Vec::new(),
            vec!["web: down".into(), "wikipedia: down".into()],
            Vec::new(),
        )
        .unwrap_err();
        assert!(all_failed.contains("web: down"));
        assert!(all_failed.contains("wikipedia: down"));
    }

    #[test]
    fn a_dead_adapter_keeps_the_hits_from_the_others() {
        let kept = merge_adapter_results(vec![
            ("wikipedia", Err("down".into())),
            ("wikidata", Ok(vec![hit("Ada")])),
        ])
        .unwrap();
        assert_eq!(kept.hits.len(), 1);
        assert!(kept.adapters.iter().any(|status| {
            status.name == "wikipedia"
                && matches!(status.state, AdapterState::Error { ref message } if message == "down")
        }));
        assert!(kept.adapters.iter().any(|status| {
            status.name == "wikidata" && matches!(status.state, AdapterState::Ok { hits: 1 })
        }));
        let failed = merge_adapter_results(vec![
            ("rdap", Err("http 500".into())),
            ("crt.sh", Err("timeout".into())),
        ])
        .unwrap_err();
        assert!(failed.contains("rdap"));
        assert!(failed.contains("crt.sh"));
    }

    #[test]
    fn public_host_from_url_extracts_hosts() {
        assert_eq!(
            public_host_from_url("https://www.Example.com/path?q=1"),
            query::normalize_domain("www.example.com")
        );
        assert_eq!(
            public_host_from_url("http://Ada.Lovelace.org/wiki"),
            Some("ada.lovelace.org".into())
        );
        assert!(public_host_from_url("http://127.0.0.1/").is_none());
        assert!(public_host_from_url("http://192.168.1.1/").is_none());
        assert!(public_host_from_url("http://localhost/x").is_none());
        assert!(public_host_from_url("not a url").is_none());
        let hits = vec![
            SearchHit {
                title: "a".into(),
                url: "https://alpha.example/a".into(),
                snippet: "x".into(),
            },
            SearchHit {
                title: "b".into(),
                url: "https://beta.example/b".into(),
                snippet: "y".into(),
            },
            SearchHit {
                title: "c".into(),
                url: "https://gamma.example/c".into(),
                snippet: "z".into(),
            },
            SearchHit {
                title: "d".into(),
                url: "https://delta.example/d".into(),
                snippet: "w".into(),
            },
        ];
        let pivoted = pivoted_domains(&hits, &["beta.example".into()]);
        assert_eq!(
            pivoted,
            vec![
                "alpha.example".to_string(),
                "gamma.example".to_string(),
                "delta.example".to_string()
            ]
        );
    }


    #[test]
    fn domain_stage_skips_rdap_without_a_domain() {
        let mut plan = SourcePlan::default();
        plan.facts = false;
        plan.web = false;
        plan.news = false;
        plan.social = false;
        plan.identity = false;
        plan.domain = true;
        let names = scheduled_adapters("who is Ada Lovelace", &plan);
        assert!(names.is_empty(), "{names:?}");
        let names = scheduled_adapters("example.com", &plan);
        assert!(names.iter().any(|name| name == "rdap"));
        assert!(names.iter().any(|name| name == "crt.sh"));
        assert!(names.iter().any(|name| name == "doh"));
        assert!(names.iter().any(|name| name == "wayback"));
        assert!(names.iter().any(|name| name == "internetdb"));
    }

    #[test]
    fn stages_follow_handles_topics_and_keys() {
        let plan = SourcePlan::default();
        let person = scheduled_adapters("who is Ada Lovelace", &plan);
        assert!(person.iter().any(|name| name == "wikipedia"));
        assert!(person.iter().any(|name| name == "gdelt"));
        assert!(person.iter().any(|name| name == "bluesky"));
        assert!(person.iter().any(|name| name == "duckduckgo"));
        assert!(!person.iter().any(|name| name == "rdap"));
        assert!(!person.iter().any(|name| name == "github"));
        assert!(!person.iter().any(|name| name == "youtube"));

        let handle = scheduled_adapters("@torvalds", &plan);
        assert!(handle.iter().any(|name| name == "github"));
        assert!(handle.iter().any(|name| name == "bluesky"));
        assert!(!handle.iter().any(|name| name == "rdap"));

        let mut keyed = plan.clone();
        keyed.youtube_key = Some("yt".into());
        keyed.brave_key = Some("brave".into());
        keyed.searx_url = Some("https://searx.example".into());
        let names = scheduled_adapters("rust async", &keyed);
        assert!(names.iter().any(|name| name == "youtube"));
        assert!(names.iter().any(|name| name == "brave"));
        assert!(names.iter().any(|name| name == "searx"));
        assert!(names.iter().any(|name| name == "searx-news"));
        assert!(!names.iter().any(|name| name == "duckduckgo"));
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
