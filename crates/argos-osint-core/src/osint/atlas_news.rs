//! GNews, NewsData.io, and Currents adapters for Atlas and manual OSINT.
//!
//! Atlas phase 1 asks GNews for top headlines and NewsData for latest news, then
//! reads `source.country` the same way search results expose it. NewsData's key
//! is a query parameter; every other key is a header.
use super::{bounded, get, str_arg, url, Request};
use anyhow::{anyhow, ensure, Result};
use serde_json::{json, Value};

pub const GNEWS_HOST: &str = "gnews.io";
pub const NEWSDATA_HOST: &str = "newsdata.io";
pub const CURRENTS_HOST: &str = "api.currentsapi.services";
pub const GNEWS_SEARCH: &str = "https://gnews.io/api/v4/search";
pub const GNEWS_TOP: &str = "https://gnews.io/api/v4/top-headlines";
pub const NEWSDATA_LATEST: &str = "https://newsdata.io/api/1/latest";
pub const CURRENTS_LATEST: &str = "https://api.currentsapi.services/v1/latest-news";

pub const GNEWS_MAX: usize = 10;
pub const NEWSDATA_SIZE: usize = 10;
pub const PHASE2_PAGE: usize = 20;
pub const QUERY_LIMIT: usize = 200;
/// NewsData rejects `q` longer than this with HTTP 422 `UnsupportedQueryLength`.
pub const NEWSDATA_QUERY_LIMIT: usize = 100;

pub const ATLAS_TOOLS: &[&str] = &["gnews_search", "newsdata_latest", "currents_latest"];

/// NewsAPI top-headlines countries. A code outside this list is not requested.
pub const NEWSAPI_COUNTRIES: &[&str] = &[
    "ae", "ar", "at", "au", "be", "bg", "br", "ca", "ch", "cn", "co", "cu", "cz", "de", "eg", "fr",
    "gb", "gr", "hk", "hu", "id", "ie", "il", "in", "it", "jp", "kr", "lt", "lv", "ma", "mx", "my",
    "ng", "nl", "no", "nz", "ph", "pl", "pt", "ro", "rs", "ru", "sa", "se", "sg", "si", "sk", "th",
    "tr", "tw", "ua", "us", "ve", "za",
];

pub fn is_atlas_tool(id: &str) -> bool {
    ATLAS_TOOLS.contains(&super::canonical_tool_id(id))
}

pub fn provider(id: &str) -> Option<&'static str> {
    match super::canonical_tool_id(id) {
        "gnews_search" => Some("gnews"),
        "newsdata_latest" => Some("newsdata"),
        "currents_latest" => Some("currents"),
        _ => None,
    }
}

pub fn locked_host(id: &str) -> Option<&'static str> {
    match provider(id)? {
        "gnews" => Some(GNEWS_HOST),
        "newsdata" => Some(NEWSDATA_HOST),
        "currents" => Some(CURRENTS_HOST),
        _ => None,
    }
}

pub fn newsapi_country_supported(country: &str) -> bool {
    NEWSAPI_COUNTRIES.contains(&country)
}

fn query_arg(v: &Value, limit: usize) -> Result<String> {
    let query = str_arg(v, "query")?;
    ensure!(
        query.len() <= limit && !query.chars().any(char::is_control),
        "query must be at most {limit} characters"
    );
    Ok(query.to_string())
}

fn country_arg(v: &Value) -> Result<String> {
    let country = str_arg(v, "country")?.to_ascii_lowercase();
    ensure!(
        country.len() == 2 && country.bytes().all(|byte| byte.is_ascii_lowercase()),
        "country must be a 2-letter code"
    );
    Ok(country)
}

fn optional_text(v: &Value, key: &str) -> Result<Option<String>> {
    let Some(raw) = v.get(key) else {
        return Ok(None);
    };
    let text = raw
        .as_str()
        .map(str::trim)
        .ok_or_else(|| anyhow!("{key} must be a string"))?;
    ensure!(!text.is_empty() && text.len() <= 40, "{key} is invalid");
    Ok(Some(text.to_string()))
}

/// A pipeline request: URL plus the headers that carry the key.
pub struct Prepared {
    pub url: String,
    pub headers: Vec<(String, String)>,
}

fn prepared(request: Request, headers: Vec<(String, String)>) -> Prepared {
    Prepared {
        url: request.url.to_string(),
        headers,
    }
}

/// Latest headlines for Atlas phase 1. No keyword query and no `from`.
/// The 48-hour window is applied to the payload. `page` is zero-based.
/// NewsData sends `page_token` as `page` when the previous response had `nextPage`.
pub fn prepare_latest(
    provider: &str,
    page: usize,
    page_token: &str,
    key: &str,
) -> Result<Prepared> {
    match provider {
        "gnews" => {
            let page_no = (page + 1).to_string();
            let max = GNEWS_MAX.to_string();
            let request = get(url(
                GNEWS_TOP,
                &[],
                &[("lang", "en"), ("max", &max), ("page", &page_no)],
            )?);
            Ok(prepared(
                request,
                vec![("X-Api-Key".into(), key.trim().into())],
            ))
        }
        "newsdata" => {
            let size = NEWSDATA_SIZE.to_string();
            let mut pairs = vec![
                ("language".to_string(), "en".to_string()),
                ("size".to_string(), size),
            ];
            if !page_token.trim().is_empty() {
                pairs.push(("page".into(), page_token.trim().to_string()));
            }
            let refs: Vec<(&str, &str)> = pairs
                .iter()
                .map(|(name, value)| (name.as_str(), value.as_str()))
                .collect();
            let mut built = url(NEWSDATA_LATEST, &[], &refs)?;
            built.query_pairs_mut().append_pair("apikey", key.trim());
            Ok(Prepared {
                url: built.to_string(),
                headers: Vec::new(),
            })
        }
        _ => Err(anyhow!("unknown discovery provider {provider}")),
    }
}

/// Token for the following page, when this payload did not exhaust the feed.
/// GNews has no token; a full page of `GNEWS_MAX` means another numeric page exists.
pub fn next_page(provider: &str, value: &Value) -> Option<String> {
    match provider {
        "newsdata" => value
            .get("nextPage")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|token| !token.is_empty())
            .map(str::to_string),
        "gnews" => {
            let count = value
                .get("articles")
                .and_then(Value::as_array)
                .map(Vec::len)
                .unwrap_or(0);
            (count >= GNEWS_MAX).then(|| "page".into())
        }
        _ => None,
    }
}

pub fn prepare_headlines(provider: &str, country: &str, key: &str) -> Result<Prepared> {
    match provider {
        "newsapi" => {
            let request = newsapi_country_headlines(country)?;
            Ok(prepared(
                request,
                vec![("X-Api-Key".into(), key.trim().into())],
            ))
        }
        "currents" => {
            let request = request(
                "currents_latest",
                &json!({"country": country, "language": "en"}),
            )?;
            Ok(prepared(
                request,
                vec![("Authorization".into(), format!("Bearer {}", key.trim()))],
            ))
        }
        _ => Err(anyhow!("unknown headline provider {provider}")),
    }
}

/// Manual OSINT request. The NewsData key is appended by the executor, not here.
pub(super) fn request(id: &str, v: &Value) -> Result<Request> {
    let id = super::canonical_tool_id(id);
    match id {
        "gnews_search" => {
            let query = query_arg(v, QUERY_LIMIT)?;
            let mut pairs = vec![
                ("q", query),
                (
                    "lang",
                    optional_text(v, "lang")?.unwrap_or_else(|| "en".into()),
                ),
                ("max", GNEWS_MAX.to_string()),
            ];
            if let Some(from) = optional_text(v, "from")? {
                ensure!(
                    chrono::DateTime::parse_from_rfc3339(&from).is_ok(),
                    "from must be an ISO 8601 timestamp"
                );
                pairs.push(("from", from));
            }
            let refs: Vec<(&str, &str)> = pairs.iter().map(|(k, val)| (*k, val.as_str())).collect();
            Ok(get(url(GNEWS_SEARCH, &[], &refs)?))
        }
        "newsdata_latest" => {
            let query = query_arg(v, NEWSDATA_QUERY_LIMIT)?;
            let language = optional_text(v, "language")?.unwrap_or_else(|| "en".into());
            let size = NEWSDATA_SIZE.to_string();
            let mut pairs = vec![
                ("q".to_string(), query),
                ("language".to_string(), language),
                ("size".to_string(), size),
            ];
            // The latest endpoint is already the past 48 hours. The free plan rejects
            // `timeframe` with HTTP 422, so it is sent only when the caller sets it.
            if let Some(timeframe) = optional_text(v, "timeframe")? {
                ensure!(
                    timeframe.chars().all(|ch| ch.is_ascii_digit())
                        && timeframe
                            .parse::<u16>()
                            .is_ok_and(|hours| (1..=48).contains(&hours)),
                    "timeframe must be 1 to 48 hours"
                );
                pairs.push(("timeframe".into(), timeframe));
            }
            let refs: Vec<(&str, &str)> = pairs
                .iter()
                .map(|(key, value)| (key.as_str(), value.as_str()))
                .collect();
            Ok(get(url(NEWSDATA_LATEST, &[], &refs)?))
        }
        "currents_latest" => {
            let country = country_arg(v)?.to_ascii_uppercase();
            let language = optional_text(v, "language")?.unwrap_or_else(|| "en".into());
            let page = PHASE2_PAGE.to_string();
            Ok(get(url(
                CURRENTS_LATEST,
                &[],
                &[
                    ("language", &language),
                    ("country", &country),
                    ("page_size", &page),
                ],
            )?))
        }
        _ => Err(anyhow!("unknown atlas tool {id}")),
    }
}

/// Country-only NewsAPI headlines. `pageSize` is 20 and there is no keyword query.
pub(super) fn newsapi_country_headlines(country: &str) -> Result<Request> {
    let country = bounded(country.trim())?.to_ascii_lowercase();
    ensure!(
        newsapi_country_supported(&country),
        "NewsAPI has no headline feed for {country}"
    );
    let page = PHASE2_PAGE.to_string();
    Ok(get(url(
        super::news_legal::NEWSAPI_BASE,
        &["top-headlines"],
        &[("country", &country), ("pageSize", &page)],
    )?))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub title: String,
    pub description: String,
    pub url: String,
    pub country: String,
    pub source_name: String,
    pub author: String,
    pub image_url: String,
    pub published_at: String,
}

fn clip(value: &str) -> String {
    value.chars().take(500).collect()
}

fn text<'a>(value: &'a Value, keys: &[&str]) -> &'a str {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .unwrap_or("")
        .trim()
}

fn country_code(raw: &str) -> Option<String> {
    let code = raw.trim();
    crate::iso3166::name(code).map(|_| code.to_ascii_lowercase())
}

/// Ranked outlets whose top-headline payloads often omit `source.country`.
const OUTLET_COUNTRY: &[(&str, &str)] = &[
    ("reuters.com", "gb"),
    ("apnews.com", "us"),
    ("afp.com", "fr"),
    ("bbc.co.uk", "gb"),
    ("bbc.com", "gb"),
    ("theguardian.com", "gb"),
    ("nytimes.com", "us"),
    ("washingtonpost.com", "us"),
    ("wsj.com", "us"),
    ("ft.com", "gb"),
    ("bloomberg.com", "us"),
    ("aljazeera.com", "qa"),
    ("cnn.com", "us"),
    ("economist.com", "gb"),
    ("nikkei.com", "jp"),
    ("scmp.com", "hk"),
];

fn place_code(raw: &str) -> Option<String> {
    if let Some(code) = country_code(raw) {
        return Some(code);
    }
    crate::iso3166::code_for_name(raw).map(str::to_string)
}

fn host_country(raw_url: &str) -> Option<String> {
    let host = url::Url::parse(raw_url)
        .ok()?
        .host_str()?
        .to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host);
    OUTLET_COUNTRY
        .iter()
        .find(|(domain, _)| {
            host == *domain
                || host
                    .strip_suffix(domain)
                    .is_some_and(|rest| rest.ends_with('.'))
        })
        .map(|(_, code)| (*code).to_string())
}

fn newsdata_country(row: &Value) -> Option<String> {
    match row.get("country") {
        Some(Value::Array(items)) => items
            .iter()
            .find_map(|item| item.as_str().and_then(place_code)),
        Some(Value::String(text)) => place_code(text),
        _ => None,
    }
}

fn http_url(raw: &str) -> Option<String> {
    let url = raw.trim();
    if url.is_empty() || url.eq_ignore_ascii_case("none") || url.eq_ignore_ascii_case("null") {
        return None;
    }
    (url.starts_with("https://") || url.starts_with("http://")).then(|| url.to_string())
}

fn image_url(row: &Value) -> String {
    ["urlToImage", "image", "image_url"]
        .iter()
        .find_map(|key| row.get(*key).and_then(Value::as_str).and_then(http_url))
        .unwrap_or_default()
}

fn authors(row: &Value) -> String {
    if let Some(text) = row.get("author").and_then(Value::as_str) {
        let text = text.trim();
        if !text.is_empty() && !text.eq_ignore_ascii_case("none") {
            return clip(text);
        }
    }
    let Some(items) = row.get("creator").and_then(Value::as_array) else {
        return String::new();
    };
    clip(
        &items
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .collect::<Vec<_>>()
            .join("; "),
    )
}

/// Phase-1 hits. A 2-letter code wins. Otherwise a country name, then a known
/// publisher host. Rows that still have no country are omitted.
pub fn discovery_hits(provider: &str, value: &Value) -> Result<Vec<Hit>> {
    if let Some(message) = api_error(provider, value) {
        return Err(anyhow!(message));
    }
    let rows = match provider {
        "gnews" => value.get("articles").and_then(Value::as_array),
        "newsdata" => value.get("results").and_then(Value::as_array),
        _ => None,
    };
    let mut hits = Vec::new();
    for row in rows.into_iter().flatten() {
        let (url, mut country, source_name, published_at, source_url) = if provider == "gnews" {
            (
                text(row, &["url"]),
                row.pointer("/source/country")
                    .and_then(Value::as_str)
                    .and_then(country_code),
                row.pointer("/source/name")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
                text(row, &["publishedAt"]),
                row.pointer("/source/url")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
            )
        } else {
            (
                text(row, &["link", "url"]),
                newsdata_country(row),
                text(row, &["source_name", "source_id"]),
                text(row, &["pubDate", "publishedAt"]),
                "",
            )
        };
        let Some(url) = http_url(url) else {
            continue;
        };
        if country.is_none() {
            country = host_country(source_url).or_else(|| host_country(&url));
        }
        let Some(country) = country else {
            continue;
        };
        hits.push(Hit {
            title: clip(text(row, &["title"])),
            description: clip(text(row, &["description", "content"])),
            url,
            country,
            source_name: clip(source_name),
            author: authors(row),
            image_url: image_url(row),
            published_at: published_at.to_string(),
        });
    }
    Ok(hits)
}

/// Phase-2 headlines. The country is the one that was queried.
pub fn headline_hits(provider: &str, country: &str, value: &Value) -> Result<Vec<Hit>> {
    if let Some(message) = api_error(provider, value) {
        return Err(anyhow!(message));
    }
    let country = country_code(country).unwrap_or_default();
    let rows = match provider {
        "newsapi" => value.get("articles").and_then(Value::as_array),
        "currents" => value.get("news").and_then(Value::as_array),
        _ => None,
    };
    let mut hits = Vec::new();
    for row in rows.into_iter().flatten().take(PHASE2_PAGE) {
        let url = http_url(text(row, &["url"]));
        let Some(url) = url else { continue };
        let source_name = match provider {
            "newsapi" => row
                .pointer("/source/name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            "currents" => String::new(),
            _ => text(row, &["source_name", "source_id"]).to_string(),
        };
        hits.push(Hit {
            title: clip(text(row, &["title"])),
            description: clip(text(row, &["description"])),
            url,
            country: country.clone(),
            source_name: clip(source_name.trim()),
            author: authors(row),
            image_url: image_url(row),
            published_at: text(row, &["publishedAt", "published"]).to_string(),
        });
    }
    Ok(hits)
}

fn api_error(provider: &str, value: &Value) -> Option<String> {
    match provider {
        "newsapi" => super::news_legal::newsapi_error(value),
        "gnews" => value.get("errors").map(|errors| {
            format!(
                "GNews rejected the request: {}",
                errors.to_string().chars().take(200).collect::<String>()
            )
        }),
        "newsdata" => {
            let status = value.get("status").and_then(Value::as_str).unwrap_or("");
            (status.eq_ignore_ascii_case("error") || status.eq_ignore_ascii_case("failed")).then(
                || {
                    let message = value
                        .pointer("/results/message")
                        .or_else(|| value.get("message"))
                        .map(|item| item.to_string())
                        .unwrap_or_else(|| "request failed".into());
                    format!(
                        "NewsData rejected the request: {}",
                        message.chars().take(200).collect::<String>()
                    )
                },
            )
        }
        "currents" => {
            let status = value.get("status").and_then(Value::as_str).unwrap_or("ok");
            (status != "ok").then(|| {
                format!(
                    "Currents rejected the request ({status}): {}",
                    value
                        .get("message")
                        .map(|item| item.to_string())
                        .unwrap_or_default()
                        .chars()
                        .take(200)
                        .collect::<String>()
                )
            })
        }
        _ => None,
    }
}

pub fn observations(id: &str, value: &Value) -> Result<(Value, bool)> {
    let id = super::canonical_tool_id(id);
    let provider = provider(id).unwrap_or("");
    let hits = if id == "currents_latest" {
        headline_hits(provider, "", value)?
    } else {
        discovery_hits(provider, value)?
    };
    let results: Vec<Value> = hits
        .iter()
        .map(|hit| {
            json!({
                "title": hit.title,
                "description": hit.description,
                "url": hit.url,
                "country": hit.country,
                "source": hit.source_name,
                "date": hit.published_at,
            })
        })
        .collect();
    Ok((
        json!({
            "provider": provider,
            "results": results,
            "evidence_form": "snippet",
        }),
        false,
    ))
}

pub fn http_error(id: &str, status: u16, body: &str) -> Option<String> {
    let provider = provider(id).or(match id {
        "gnews" | "newsdata" | "currents" => Some(id),
        _ => None,
    })?;
    let detail = body.chars().take(180).collect::<String>();
    Some(match (provider, status) {
        ("gnews", 401 | 403) => {
            "GNews rejected the API key. Check the key on a GNews tool or GNEWS_API_KEY.".into()
        }
        ("gnews", 429) => {
            "GNews rate limit reached (HTTP 429). The free tier allows 100 requests a day.".into()
        }
        ("newsdata", 401 | 403) => {
            "NewsData rejected the API key. Check the key on a NewsData tool or NEWSDATA_API_KEY."
                .into()
        }
        ("newsdata", 429) => {
            "NewsData rate limit reached (HTTP 429). The free tier allows 200 credits a day.".into()
        }
        ("currents", 401 | 403) => {
            "Currents rejected the API key. Check the key on a Currents tool or CURRENTS_API_KEY."
                .into()
        }
        ("currents", 429) => {
            "Currents rate limit reached (HTTP 429). The free tier allows 250 requests a day."
                .into()
        }
        _ => format!("{provider} HTTP {status}: {detail}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn latest_omits_timeframe_unless_the_caller_sets_one() {
        let plain = request("newsdata_latest", &json!({"query": "export restriction"})).unwrap();
        assert!(plain.url.query_pairs().all(|(key, _)| key != "timeframe"));
        let paid = request(
            "newsdata_latest",
            &json!({"query": "export restriction", "timeframe": "6"}),
        )
        .unwrap();
        assert!(paid
            .url
            .query_pairs()
            .any(|(key, value)| key == "timeframe" && value == "6"));
    }

    #[test]
    fn latest_requests_have_no_keyword_and_can_page() {
        let gnews = prepare_latest("gnews", 1, "", "gnews-key").unwrap();
        assert!(gnews.url.contains("/top-headlines"));
        assert!(gnews.url.contains("page=2"));
        let gnews_url = url::Url::parse(&gnews.url).unwrap();
        assert!(gnews_url
            .query_pairs()
            .all(|(key, _)| key != "q" && key != "from"));
        assert_eq!(gnews.headers[0].0, "X-Api-Key");
        let first = prepare_latest("newsdata", 0, "", "nd-key").unwrap();
        assert!(first.url.contains("/latest"));
        assert!(first.url.contains("apikey=nd-key"));
        let first_url = url::Url::parse(&first.url).unwrap();
        assert!(first_url
            .query_pairs()
            .all(|(key, _)| key != "q" && key != "page"));
        let next = prepare_latest("newsdata", 1, "tok-9", "nd-key").unwrap();
        let next_url = url::Url::parse(&next.url).unwrap();
        assert!(next_url
            .query_pairs()
            .any(|(key, value)| key == "page" && value == "tok-9"));
        let full = json!({"articles": (0..10).map(|n| json!({"title": n})).collect::<Vec<_>>()});
        assert!(next_page("gnews", &full).is_some());
        assert!(next_page("gnews", &json!({"articles":[{"title":"one"}]})).is_none());
        assert_eq!(
            next_page("newsdata", &json!({"nextPage": "abc"})).as_deref(),
            Some("abc")
        );
    }

    #[test]
    fn discovery_reads_a_publisher_host_and_a_country_name() {
        let gnews = json!({"articles":[{
            "title":"Cabinet meets",
            "url":"https://www.reuters.com/world/story",
            "source":{"name":"Reuters","url":"https://www.reuters.com"}
        }]});
        let hits = discovery_hits("gnews", &gnews).unwrap();
        assert_eq!(hits[0].country, "gb");
        let newsdata = json!({"status":"success","results":[{
            "title":"Berlin wire",
            "link":"https://example.com/berlin",
            "country":["Germany"],
            "source_name":"Wire"
        }]});
        let hits = discovery_hits("newsdata", &newsdata).unwrap();
        assert_eq!(hits[0].country, "de");
    }

    #[test]
    fn newsapi_headlines_reject_spain() {
        assert!(!newsapi_country_supported("es"));
        assert!(newsapi_country_headlines("es").is_err());
        assert!(newsapi_country_headlines("us").is_ok());
    }
}
