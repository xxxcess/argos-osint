//! Public web search. A configured SearXNG JSON API is preferred. Without
//! one, Argos reads DuckDuckGo's HTML results. Page fetches refuse loopback,
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
    let url = "https://html.duckduckgo.com/html/";
    let client = client()?;
    let resp = client
        .post(url)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(format!("q={}", urlencoding::encode(query)))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("duckduckgo {}", resp.status()));
    }
    let html = resp.text().await.map_err(|e| e.to_string())?;
    let hits = parse_ddg_html(&html);
    if hits.is_empty() {
        Err("no public results (DuckDuckGo returned an empty page; set searx_url in ~/.argos/config.toml)".into())
    } else {
        Ok(hits)
    }
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
    fn blocks_private_and_metadata_targets() {
        assert!(check_public_http("http://127.0.0.1/").is_err());
        assert!(check_public_http("http://169.254.169.254/latest").is_err());
        assert!(check_public_http("https://user:pass@example.com/").is_err());
        assert!(check_public_http("file:///etc/passwd").is_err());
        assert!(check_public_http("http://localhost:11434/").is_err());
    }
}
