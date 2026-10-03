//! GNews, NewsData.io, and Currents adapters for Atlas and manual OSINT.
//!
//! Phase 1 uses GNews Search, not top headlines: `source.country` is only on the
//! Search payload. NewsData's key is a query parameter; every other key is a header.
use super::{bounded, get, str_arg, url, Request};
use anyhow::{anyhow, ensure, Result};
use serde_json::{json, Value};

pub const GNEWS_HOST: &str = "gnews.io";
pub const NEWSDATA_HOST: &str = "newsdata.io";
pub const CURRENTS_HOST: &str = "api.currentsapi.services";
pub const GNEWS_SEARCH: &str = "https://gnews.io/api/v4/search";
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

pub fn prepare_discovery(provider: &str, query: &str, from: &str, key: &str) -> Result<Prepared> {
    match provider {
        "gnews" => {
            let request = request(
                "gnews_search",
                &json!({"query": query, "from": from, "lang": "en"}),
            )?;
            Ok(prepared(
                request,
                vec![("X-Api-Key".into(), key.trim().into())],
            ))
        }
        "newsdata" => {
            let request = request(
                "newsdata_latest",
                &json!({"query": query, "language": "en"}),
            )?;
            let mut url = request.url;
            url.query_pairs_mut().append_pair("apikey", key.trim());
            Ok(Prepared {
                url: url.to_string(),
                headers: Vec::new(),
            })
        }
        _ => Err(anyhow!("unknown discovery provider {provider}")),
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
    let code = raw.trim().to_ascii_lowercase();
    (code.len() == 2 && code.bytes().all(|byte| byte.is_ascii_lowercase())).then_some(code)
}

fn http_url(raw: &str) -> Option<String> {
    let url = raw.trim();
    (url.starts_with("https://") || url.starts_with("http://")).then(|| url.to_string())
}

/// Phase-1 hits. Country comes from the payload. Rows without a 2-letter code are omitted.
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
        let (url, country, source_name, published_at) = if provider == "gnews" {
            (
                text(row, &["url"]),
                row.pointer("/source/country")
                    .and_then(Value::as_str)
                    .and_then(country_code),
                row.pointer("/source/name")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
                text(row, &["publishedAt"]),
            )
        } else {
            let country = row
                .get("country")
                .and_then(Value::as_array)
                .and_then(|codes| {
                    codes
                        .iter()
                        .find_map(|code| code.as_str().and_then(country_code))
                });
            (
                text(row, &["link", "url"]),
                country,
                text(row, &["source_name", "source_id"]),
                text(row, &["pubDate", "publishedAt"]),
            )
        };
        let Some(url) = http_url(url) else {
            continue;
        };
        let Some(country) = country else {
            continue;
        };
        hits.push(Hit {
            title: clip(text(row, &["title"])),
            description: clip(text(row, &["description", "content"])),
            url,
            country,
            source_name: clip(source_name),
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
            _ => text(row, &["author"]).to_string(),
        };
        hits.push(Hit {
            title: clip(text(row, &["title"])),
            description: clip(text(row, &["description"])),
            url,
            country: country.clone(),
            source_name: clip(source_name.trim()),
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
    fn newsapi_headlines_reject_spain() {
        assert!(!newsapi_country_supported("es"));
        assert!(newsapi_country_headlines("es").is_err());
        assert!(newsapi_country_headlines("us").is_ok());
    }
}
