//! Primary-provider adapters (issue #27): the SociaVault route matrix, Firecrawl site
//! discovery, multi-page retrieval and structured extraction, and Hunter enrichment reads.
//! Every route here is a fixed-host GET or POST built from validated inputs, like the
//! adapters in `osint.rs`.
use super::{
    bounded, clip_text, domain, email_address, get, https_on_host, linkedin_handle,
    number_arg, profile_path_token, push_link, social_token, str_arg, url, url_arg,
    Request,
};
use anyhow::{anyhow, ensure, Result};
use serde_json::{json, Value};
use url::Url;

pub const SOCIAVAULT_BASE: &str = "https://api.sociavault.com/v1/scrape";
pub const FIRECRAWL_BASE: &str = "https://api.firecrawl.dev/v2";
pub const HUNTER_BASE: &str = "https://api.hunter.io/v2";

/// Default and ceiling for the URLs one `firecrawl_batch_scrape` call takes.
pub const BATCH_SCRAPE_DEFAULT_URLS: usize = 5;
pub const BATCH_SCRAPE_MAX_URLS: usize = 10;
/// Ceiling for `firecrawl_crawl` pages and `firecrawl_map` links.
pub const CRAWL_MAX_PAGES: u64 = 10;
pub const MAP_MAX_LINKS: u64 = 100;
/// Credits `firecrawl_extract` costs: one page plus the JSON format.
pub const EXTRACT_CREDITS: u32 = 5;

/// Webmail hosts: an address there says nothing about the owner's organization.
const FREE_MAIL: &[&str] = &[
    "gmail.com", "googlemail.com", "yahoo.com", "outlook.com", "hotmail.com", "live.com",
    "icloud.com", "me.com", "aol.com", "proton.me", "protonmail.com", "gmx.com", "mail.com",
    "yandex.com", "zoho.com",
];

pub fn webmail_host(host: &str) -> bool {
    FREE_MAIL.contains(&host.trim().trim_start_matches("www.").to_ascii_lowercase().as_str())
}

// ---------------------------------------------------------------------------
// SociaVault route matrix
// ---------------------------------------------------------------------------

/// How one SociaVault route reads the tool inputs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RouteInput {
    /// `?handle=` from `handle` (a bare handle or a profile URL).
    Handle,
    /// `?<param>=` from `user_id` (a numeric platform id from a profile call).
    UserId,
    /// `?handle=` from `handle`, else `?user_id=` from `user_id`.
    HandleOrUserId,
    /// `?url=` with a Facebook page URL built from `handle`.
    FacebookUrl,
    /// `?url=` with a LinkedIn profile URL built from `handle`.
    LinkedinProfileUrl,
    /// `?url=` with a LinkedIn company URL built from `handle`.
    LinkedinCompanyUrl,
    /// `?channelId=` from `user_id`, else `channelId`, `handle`, or `url` from `handle`.
    YoutubeChannel,
    /// `?query=` from `query`.
    Query,
    /// `?hashtag=` from `query` (one token, `#` dropped).
    Hashtag,
    /// `?subreddit=` from `subreddit`, plus `?query=` from `query` when given.
    SubredditQuery,
}

impl RouteInput {
    /// Tool inputs the route reads. Any one of them is enough.
    pub fn keys(self) -> &'static [&'static str] {
        match self {
            RouteInput::Handle
            | RouteInput::FacebookUrl
            | RouteInput::LinkedinProfileUrl
            | RouteInput::LinkedinCompanyUrl => &["handle"],
            RouteInput::UserId => &["user_id"],
            RouteInput::HandleOrUserId | RouteInput::YoutubeChannel => &["handle", "user_id"],
            RouteInput::Query | RouteInput::Hashtag => &["query"],
            RouteInput::SubredditQuery => &["subreddit"],
        }
    }
}

/// One SociaVault route: the catalog tool and platform it serves, the `endpoint` value
/// that selects it, its path under `/v1/scrape`, and how it reads the inputs. The first
/// route of a (tool, platform) pair is the default.
#[derive(Clone, Copy, Debug)]
pub struct SociaVaultRoute {
    pub tool: &'static str,
    pub platform: &'static str,
    pub endpoint: &'static str,
    pub path: &'static str,
    pub input: RouteInput,
    /// The query parameter for `UserId` routes.
    pub param: &'static str,
}

const fn sv(
    tool: &'static str,
    platform: &'static str,
    endpoint: &'static str,
    path: &'static str,
    input: RouteInput,
) -> SociaVaultRoute {
    SociaVaultRoute { tool, platform, endpoint, path, input, param: "" }
}

const fn sv_id(
    tool: &'static str,
    platform: &'static str,
    endpoint: &'static str,
    path: &'static str,
    param: &'static str,
) -> SociaVaultRoute {
    SociaVaultRoute { tool, platform, endpoint, path, input: RouteInput::UserId, param }
}

use RouteInput::*;

/// Every SociaVault route Argos can call: 1-credit search, profile, and user-content
/// routes. Followers, following, and single-post routes (post, tweet, video, comments,
/// transcripts) are deliberately absent.
pub const SOCIAVAULT_ROUTES: &[SociaVaultRoute] = &[
    // Google search: a fallback when Firecrawl search was weak.
    sv("sociavault_google_search", "google", "search", "google/search", Query),
    // Profiles.
    sv("sociavault_profile", "facebook", "profile", "facebook/profile", FacebookUrl),
    sv("sociavault_profile", "instagram", "profile", "instagram/profile", Handle),
    sv_id("sociavault_profile", "instagram", "basic_profile", "instagram/basic-profile", "userId"),
    sv("sociavault_profile", "linkedin", "profile", "linkedin/profile", LinkedinProfileUrl),
    sv("sociavault_profile", "linkedin", "company", "linkedin/company", LinkedinCompanyUrl),
    sv("sociavault_profile", "threads", "profile", "threads/profile", Handle),
    sv("sociavault_profile", "tiktok", "profile", "tiktok/profile", Handle),
    sv("sociavault_profile", "twitch", "profile", "twitch/profile", Handle),
    sv("sociavault_profile", "twitter", "profile", "twitter/profile", Handle),
    sv("sociavault_profile", "youtube", "channel", "youtube/channel", YoutubeChannel),
    // Content search.
    sv("sociavault_search", "instagram", "hashtag", "instagram/search/hashtag", Hashtag),
    sv("sociavault_search", "linkedin", "posts", "linkedin/search/posts", Query),
    sv("sociavault_search", "pinterest", "search", "pinterest/search", Query),
    sv("sociavault_search", "reddit", "search", "reddit/search", Query),
    sv("sociavault_search", "reddit", "subreddit", "reddit/subreddit/search", SubredditQuery),
    sv("sociavault_search", "threads", "search", "threads/search", Query),
    sv("sociavault_search", "tiktok", "keyword", "tiktok/search/keyword", Query),
    sv("sociavault_search", "tiktok", "hashtag", "tiktok/search/hashtag", Hashtag),
    sv("sociavault_search", "tiktok", "top", "tiktok/search/top", Query),
    sv("sociavault_search", "twitter", "search", "twitter/search", Query),
    sv("sociavault_search", "youtube", "search", "youtube/search", Query),
    sv("sociavault_search", "youtube", "hashtag", "youtube/search/hashtag", Hashtag),
    // Account search.
    sv("sociavault_search_users", "instagram", "users", "instagram/search", Query),
    sv("sociavault_search_users", "threads", "users", "threads/search-users", Query),
    sv("sociavault_search_users", "tiktok", "users", "tiktok/search/users", Query),
    // One account's own content.
    sv("sociavault_user_content", "facebook", "posts", "facebook/profile/posts", FacebookUrl),
    sv("sociavault_user_content", "facebook", "reels", "facebook/profile/reels", FacebookUrl),
    sv("sociavault_user_content", "instagram", "posts", "instagram/posts", Handle),
    sv("sociavault_user_content", "instagram", "highlights", "instagram/highlights", HandleOrUserId),
    sv("sociavault_user_content", "instagram", "reels", "instagram/reels", HandleOrUserId),
    sv("sociavault_user_content", "pinterest", "boards", "pinterest/user/boards", Handle),
    sv("sociavault_user_content", "threads", "posts", "threads/user-posts", Handle),
    sv("sociavault_user_content", "tiktok", "videos", "tiktok/videos", Handle),
    sv("sociavault_user_content", "tiktok", "live", "tiktok/live", Handle),
    sv("sociavault_user_content", "twitch", "videos", "twitch/user/videos", Handle),
    sv("sociavault_user_content", "twitch", "schedule", "twitch/user/schedule", Handle),
    sv("sociavault_user_content", "twitter", "tweets", "twitter/user-tweets", Handle),
    sv_id("sociavault_user_content", "twitter", "tweets_all", "twitter/user-tweets-all", "user_id"),
    sv("sociavault_user_content", "youtube", "videos", "youtube/channel-videos", YoutubeChannel),
    sv("sociavault_user_content", "youtube", "community_posts", "youtube/channel/community-posts", YoutubeChannel),
    sv("sociavault_user_content", "youtube", "lives", "youtube/channel/lives", YoutubeChannel),
    sv("sociavault_user_content", "youtube", "playlists", "youtube/channel/playlists", YoutubeChannel),
    sv("sociavault_user_content", "youtube", "shorts", "youtube/channel/shorts", YoutubeChannel),
];

/// The five SociaVault catalog tools, in registry order.
pub const SOCIAVAULT_TOOLS: &[&str] = &[
    "sociavault_profile",
    "sociavault_search",
    "sociavault_search_users",
    "sociavault_user_content",
    "sociavault_google_search",
];

pub fn sociavault_routes(tool: &str) -> impl Iterator<Item = &'static SociaVaultRoute> + '_ {
    SOCIAVAULT_ROUTES.iter().filter(move |route| route.tool == tool)
}

/// Platforms a SociaVault tool serves, in route order.
pub fn sociavault_platforms(tool: &str) -> Vec<&'static str> {
    let mut platforms = Vec::new();
    for route in sociavault_routes(tool) {
        if !platforms.contains(&route.platform) {
            platforms.push(route.platform);
        }
    }
    platforms
}

/// Platforms where a tool takes an account (handle or platform id) rather than a query.
pub fn sociavault_account_platforms(tool: &str) -> Vec<&'static str> {
    let mut platforms = Vec::new();
    for route in sociavault_routes(tool) {
        let account = route.input.keys().iter().any(|key| *key == "handle" || *key == "user_id");
        if account && !platforms.contains(&route.platform) {
            platforms.push(route.platform);
        }
    }
    platforms
}

fn normalize_platform(raw: &str) -> String {
    let lower = raw.trim().to_ascii_lowercase();
    match lower.as_str() {
        "x" | "x.com" | "twitter.com" => "twitter".into(),
        "ig" => "instagram".into(),
        "fb" => "facebook".into(),
        "yt" => "youtube".into(),
        _ => lower,
    }
}

/// The route a call selects: the named `endpoint`, else the first route of the platform
/// whose inputs are present (a LinkedIn `/company/` URL selects the company page).
pub fn select_route(tool: &str, v: &Value) -> Result<&'static SociaVaultRoute> {
    let platform = if tool == "sociavault_google_search" {
        "google".to_string()
    } else {
        normalize_platform(str_arg(v, "platform")?)
    };
    let routes: Vec<&SociaVaultRoute> =
        sociavault_routes(tool).filter(|route| route.platform == platform).collect();
    ensure!(!routes.is_empty(), "unsupported SociaVault platform for {tool}");
    if let Ok(endpoint) = str_arg(v, "endpoint") {
        let wanted = endpoint.to_ascii_lowercase().replace(['-', ' '], "_");
        return routes.iter().copied().find(|route| route.endpoint == wanted).ok_or_else(|| {
            anyhow!(
                "unsupported endpoint {wanted} for {platform}; use {}",
                routes.iter().map(|route| route.endpoint).collect::<Vec<_>>().join(", ")
            )
        });
    }
    let present = |key: &str| str_arg(v, key).is_ok();
    if platform == "linkedin" && str_arg(v, "handle").is_ok_and(|handle| handle.contains("/company/") || handle.starts_with("company/")) {
        if let Some(route) = routes.iter().find(|route| route.endpoint == "company") {
            return Ok(route);
        }
    }
    routes
        .iter()
        .copied()
        .find(|route| route.input.keys().iter().any(|key| present(key)))
        .ok_or_else(|| anyhow!("provide {}", routes[0].input.keys().join(" or ")))
}

fn platform_id(raw: &str) -> Result<String> {
    let value = raw.trim();
    ensure!(
        (1..=64).contains(&value.len())
            && value.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-')),
        "invalid platform id"
    );
    Ok(value.to_string())
}

fn hashtag(raw: &str) -> Result<String> {
    let value: String = raw.trim().trim_start_matches('#').chars().filter(|c| !c.is_whitespace()).collect();
    ensure!(
        (1..=100).contains(&value.chars().count()) && value.chars().all(|c| c.is_alphanumeric() || c == '_'),
        "invalid hashtag"
    );
    Ok(value)
}

fn subreddit(raw: &str) -> Result<String> {
    let value = raw.trim().trim_start_matches('/').trim_start_matches("r/").trim_end_matches('/');
    ensure!(
        (2..=21).contains(&value.len()) && value.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
        "invalid subreddit"
    );
    Ok(value.to_string())
}

fn facebook_url(raw: &str) -> Result<String> {
    if raw.starts_with("https://") || raw.starts_with("http://") {
        Ok(https_on_host(raw, &["facebook.com", "fb.com"])?.to_string())
    } else {
        Ok(format!("https://www.facebook.com/{}", social_token(raw)?))
    }
}

fn linkedin_url(raw: &str, company: bool) -> Result<String> {
    if raw.starts_with("https://") || raw.starts_with("http://") {
        let parsed = https_on_host(raw, &["linkedin.com"])?;
        ensure!(
            parsed.path().contains("/company/") == company,
            "{} URL required",
            if company { "LinkedIn company" } else { "LinkedIn profile" }
        );
        Ok(parsed.to_string())
    } else {
        // Hunter reports LinkedIn handles with their page type: `company/acme`, `in/jane`.
        let (kind, bare) = match (raw.strip_prefix("company/"), raw.strip_prefix("in/")) {
            (Some(rest), _) => (Some(true), rest),
            (_, Some(rest)) => (Some(false), rest),
            _ => (None, raw),
        };
        ensure!(kind.is_none_or(|is_company| is_company == company), "{} handle required", if company { "LinkedIn company" } else { "LinkedIn profile" });
        let segment = if company { "company" } else { "in" };
        Ok(format!("https://www.linkedin.com/{segment}/{}", linkedin_handle(bare)?))
    }
}

fn youtube_pair(v: &Value) -> Result<(&'static str, String)> {
    if let Ok(id) = str_arg(v, "user_id") {
        return Ok(("channelId", platform_id(id)?));
    }
    let raw = str_arg(v, "handle")?;
    if raw.starts_with("https://") || raw.starts_with("http://") {
        let parsed = https_on_host(raw, &["youtube.com", "youtu.be"])?;
        let segments: Vec<_> =
            parsed.path_segments().into_iter().flatten().filter(|segment| !segment.is_empty()).collect();
        if segments.first().copied() == Some("channel") {
            return Ok(("channelId", social_token(segments.get(1).copied().unwrap_or(""))?));
        }
        return Ok(("handle", profile_path_token(raw)?));
    }
    let token = social_token(raw)?;
    let name = if token.starts_with("UC") && token.len() >= 20 { "channelId" } else { "handle" };
    Ok((name, token))
}

/// Query pairs for a route.
pub fn route_query(route: &SociaVaultRoute, v: &Value) -> Result<Vec<(&'static str, String)>> {
    let handle = || str_arg(v, "handle");
    Ok(match route.input {
        Handle => vec![("handle", profile_path_token(handle()?)?)],
        UserId => vec![(route.param, platform_id(str_arg(v, "user_id")?)?)],
        HandleOrUserId => match handle() {
            Ok(raw) => vec![("handle", profile_path_token(raw)?)],
            Err(_) => vec![("user_id", platform_id(str_arg(v, "user_id")?)?)],
        },
        FacebookUrl => vec![("url", facebook_url(handle()?)?)],
        LinkedinProfileUrl => vec![("url", linkedin_url(handle()?, false)?)],
        LinkedinCompanyUrl => vec![("url", linkedin_url(handle()?, true)?)],
        YoutubeChannel => vec![youtube_pair(v)?],
        Query => vec![("query", bounded(str_arg(v, "query")?)?)],
        Hashtag => vec![("hashtag", hashtag(str_arg(v, "query")?)?)],
        SubredditQuery => {
            let mut pairs = vec![("subreddit", subreddit(str_arg(v, "subreddit")?)?)];
            if let Ok(query) = str_arg(v, "query") {
                pairs.push(("query", bounded(query)?));
            }
            pairs
        }
    })
}

pub fn sociavault_request(tool: &str, v: &Value) -> Result<Request> {
    let route = select_route(tool, v)?;
    let pairs = route_query(route, v)?;
    let query: Vec<(&str, &str)> = pairs.iter().map(|(key, value)| (*key, value.as_str())).collect();
    let segments: Vec<&str> = route.path.split('/').collect();
    Ok(get(url(SOCIAVAULT_BASE, &segments, &query)?))
}

/// A non-default endpoint the question text asks for ("their reels", "community posts",
/// "#launch", "r/rust"), for the binder to pass as `endpoint`.
pub fn sociavault_endpoint_hint(tool: &str, platform: &str, text: &str) -> Option<&'static str> {
    let lower = text.to_ascii_lowercase();
    let routes: Vec<&SociaVaultRoute> =
        sociavault_routes(tool).filter(|route| route.platform == platform).collect();
    routes.iter().skip(1).find_map(|route| {
        let words: &[&str] = match route.endpoint {
            "hashtag" => &["#", "hashtag"],
            "subreddit" => &["r/", "subreddit"],
            "community_posts" => &["community post"],
            "lives" | "live" => &["live"],
            "top" => &["top "],
            "reels" => &["reels"],
            "highlights" => &["highlights"],
            "playlists" => &["playlists"],
            "shorts" => &["shorts"],
            "schedule" => &["schedule"],
            "boards" => &["boards"],
            "videos" => &["videos"],
            "posts" => &["posts"],
            // Needs a numeric id or a company URL; never inferred from text.
            _ => &[],
        };
        words.iter().any(|word| lower.contains(word)).then_some(route.endpoint)
    })
}

/// Numeric account id from a SociaVault profile payload, for user-content routes.
pub fn sociavault_platform_id(platform: &str, value: &Value) -> Option<String> {
    let pointers: &[&str] = match platform {
        "twitter" => &["/data/rest_id", "/data/user/rest_id", "/rest_id"],
        "instagram" => &["/data/data/user/id", "/data/user/id", "/data/user/pk", "/data/id"],
        "youtube" => &["/data/channelId", "/channelId"],
        _ => &[],
    };
    pointers.iter().find_map(|pointer| {
        let found = value.pointer(pointer)?;
        let text = found.as_str().map(str::to_string).or_else(|| found.as_u64().map(|n| n.to_string()))?;
        platform_id(&text).ok()
    })
}

/// Compact SociaVault search, account-search, and user-content payload: account rows,
/// outbound links, and short texts. Google search becomes `results` like Firecrawl search.
pub fn sociavault_items(tool: &str, value: &Value) -> Value {
    if tool == "sociavault_google_search" {
        let rows = value
            .pointer("/data/results")
            .or_else(|| value.get("results"))
            .map(|rows| match rows {
                Value::Array(items) => items.clone(),
                Value::Object(map) => map.values().cloned().collect(),
                _ => Vec::new(),
            })
            .unwrap_or_default();
        let results: Vec<Value> = rows
            .iter()
            .filter_map(|row| {
                let link = row.get("url").or_else(|| row.get("link")).and_then(Value::as_str)?;
                Some(json!({
                    "title": clip_text(row.get("title").and_then(Value::as_str).unwrap_or("")),
                    "url": link,
                    "snippet": clip_text(row.get("description").or_else(|| row.get("snippet")).and_then(Value::as_str).unwrap_or("")),
                }))
            })
            .take(10)
            .collect();
        return json!({"results": results, "evidence_form": "snippet"});
    }
    let mut accounts: Vec<Value> = Vec::new();
    let mut links: Vec<String> = Vec::new();
    let mut texts: Vec<String> = Vec::new();
    fn walk(value: &Value, depth: usize, accounts: &mut Vec<Value>, links: &mut Vec<String>, texts: &mut Vec<String>) {
        if depth > 9 {
            return;
        }
        match value {
            Value::Array(items) => items.iter().take(40).for_each(|item| walk(item, depth + 1, accounts, links, texts)),
            Value::Object(map) => {
                let field = |keys: &[&str]| {
                    keys.iter().find_map(|key| map.get(*key).and_then(Value::as_str)).map(str::trim).filter(|text| !text.is_empty())
                };
                if let Some(handle) = field(&["username", "screen_name", "uniqueId", "unique_id", "handle", "channelHandle"])
                    .map(|handle| handle.trim_start_matches('@'))
                    .filter(|handle| social_token(handle).is_ok())
                {
                    let known = accounts.iter().any(|row| row["handle"].as_str().is_some_and(|other| other.eq_ignore_ascii_case(handle)));
                    if !known && accounts.len() < 15 {
                        accounts.push(json!({
                            "handle": handle,
                            "name": field(&["full_name", "fullName", "nickname", "name", "title"]).unwrap_or(""),
                        }));
                    }
                }
                for (key, child) in map {
                    if let Some(text) = child.as_str() {
                        if text.starts_with("http://") || text.starts_with("https://") {
                            push_link(links, text, 20);
                        } else if matches!(key.as_str(), "text" | "full_text" | "caption" | "description" | "title" | "content" | "snippet" | "desc")
                            && texts.len() < 10
                        {
                            let clipped = clip_text(text);
                            if !clipped.is_empty() && !texts.contains(&clipped) {
                                texts.push(clipped);
                            }
                        }
                    }
                    walk(child, depth + 1, accounts, links, texts);
                }
            }
            _ => {}
        }
    }
    walk(value.get("data").unwrap_or(value), 0, &mut accounts, &mut links, &mut texts);
    json!({"accounts": accounts, "links": links, "texts": texts})
}

// ---------------------------------------------------------------------------
// Firecrawl
// ---------------------------------------------------------------------------

/// `firecrawl_search` body: query and limit, plus the optional sources, categories, time
/// filter, and location. `github` maps to Firecrawl's current `developer` category.
pub fn firecrawl_search_body(v: &Value) -> Result<Value> {
    let query = bounded(str_arg(v, "query")?)?;
    let limit: u64 = number_arg(v, "limit", 3, 10)?.parse().unwrap_or(3);
    let list = |key: &str, allowed: &[&str]| -> Result<Option<Vec<String>>> {
        let Some(raw) = v.get(key) else { return Ok(None) };
        let items: Vec<String> = match raw {
            Value::String(text) => vec![text.trim().to_ascii_lowercase()],
            Value::Array(items) => items
                .iter()
                .map(|item| item.as_str().map(|text| text.trim().to_ascii_lowercase()).ok_or_else(|| anyhow!("{key} must be strings")))
                .collect::<Result<_>>()?,
            _ => return Err(anyhow!("{key} must be a list")),
        };
        ensure!(!items.is_empty() && items.len() <= allowed.len(), "invalid {key}");
        for item in &items {
            ensure!(allowed.contains(&item.as_str()), "unsupported {key} {item}; use {}", allowed.join(", "));
        }
        Ok(Some(items))
    };
    let mut body = json!({"query": query, "limit": limit, "sources": ["web"]});
    if let Some(sources) = list("sources", &["web", "news"])? {
        body["sources"] = json!(sources);
    }
    if let Some(categories) = list("categories", &["github", "research"])? {
        let mapped: Vec<Value> = categories
            .iter()
            .map(|item| json!({"type": if item == "github" { "developer" } else { item.as_str() }}))
            .collect();
        body["categories"] = json!(mapped);
    }
    if let Ok(tbs) = str_arg(v, "tbs") {
        let ok = tbs.split(',').all(|part| {
            matches!(part, "qdr:h" | "qdr:d" | "qdr:w" | "qdr:m" | "qdr:y" | "sbd:1" | "cdr:1")
                || part.strip_prefix("cd_min:").or_else(|| part.strip_prefix("cd_max:")).is_some_and(|date| {
                    date.len() == 10 && date.chars().all(|c| c.is_ascii_digit() || c == '/')
                })
        });
        ensure!(ok, "unsupported tbs time filter");
        body["tbs"] = json!(tbs);
    }
    if let Ok(location) = str_arg(v, "location") {
        ensure!(location.len() <= 100, "location too long");
        body["location"] = json!(bounded(location)?);
    }
    Ok(body)
}

pub fn firecrawl_scrape_body(v: &Value) -> Result<Value> {
    let page = url_arg(str_arg(v, "url")?)?;
    let mut formats = vec!["markdown".to_string()];
    if let Some(raw) = v.get("formats") {
        let items: Vec<&str> = match raw {
            Value::String(text) => vec![text.as_str()],
            Value::Array(items) => items.iter().filter_map(Value::as_str).collect(),
            _ => Vec::new(),
        };
        ensure!(!items.is_empty(), "formats must name markdown or links");
        formats.clear();
        for item in items {
            let item = item.trim().to_ascii_lowercase();
            ensure!(
                matches!(item.as_str(), "markdown" | "links"),
                "unsupported format {item}; structured JSON goes through firecrawl_extract"
            );
            if !formats.contains(&item) {
                formats.push(item);
            }
        }
    }
    Ok(json!({"url": page, "formats": formats, "onlyMainContent": true}))
}

/// Site root for map and crawl: a bare domain or an http(s) URL on a public host.
fn site_url(v: &Value) -> Result<(String, String)> {
    let raw = str_arg(v, "domain").or_else(|_| str_arg(v, "url"))?;
    let page = if raw.starts_with("http://") || raw.starts_with("https://") {
        url_arg(raw)?
    } else {
        format!("https://{}", domain(raw)?)
    };
    let host = Url::parse(&page)?.host_str().unwrap_or("").trim_start_matches("www.").to_ascii_lowercase();
    domain(&host)?;
    ensure!(
        !crate::recon::investigation::social_or_publisher(&host),
        "{host} is a social, publisher, or Q&A host; map and crawl run only on a site the subject owns"
    );
    Ok((page, host))
}

pub fn firecrawl_map_body(v: &Value) -> Result<Value> {
    let (page, _) = site_url(v)?;
    let limit: u64 = number_arg(v, "limit", 50, MAP_MAX_LINKS)?.parse().unwrap_or(50);
    let mut body = json!({"url": page, "limit": limit, "includeSubdomains": false, "sitemap": "include"});
    if let Ok(search) = str_arg(v, "search") {
        ensure!(search.len() <= 80, "search term too long");
        body["search"] = json!(bounded(search)?);
    }
    Ok(body)
}

pub fn batch_urls(v: &Value) -> Result<Vec<String>> {
    let items: Vec<&str> = match v.get("urls") {
        Some(Value::Array(items)) => items.iter().map(|item| item.as_str().ok_or_else(|| anyhow!("urls must be strings"))).collect::<Result<_>>()?,
        Some(Value::String(text)) => text.split_whitespace().collect(),
        _ => return Err(anyhow!("missing or invalid urls")),
    };
    ensure!(!items.is_empty(), "urls must name at least one page");
    ensure!(items.len() <= BATCH_SCRAPE_MAX_URLS, "at most {BATCH_SCRAPE_MAX_URLS} URLs per batch");
    let mut pages = Vec::new();
    for item in items {
        let page = url_arg(item.trim())?;
        if !pages.contains(&page) {
            pages.push(page);
        }
    }
    Ok(pages)
}

pub fn firecrawl_batch_body(v: &Value) -> Result<Value> {
    let pages = batch_urls(v)?;
    Ok(json!({"urls": pages, "formats": ["markdown"], "onlyMainContent": true, "ignoreInvalidURLs": true}))
}

pub fn crawl_limit(v: &Value) -> Result<u64> {
    Ok(number_arg(v, "limit", CRAWL_MAX_PAGES, CRAWL_MAX_PAGES)?.parse().unwrap_or(CRAWL_MAX_PAGES))
}

pub fn firecrawl_crawl_body(v: &Value) -> Result<Value> {
    let (page, _) = site_url(v)?;
    Ok(json!({
        "url": page,
        "limit": crawl_limit(v)?,
        "maxDiscoveryDepth": 1,
        "crawlEntireDomain": false,
        "allowSubdomains": false,
        "allowExternalLinks": false,
        "scrapeOptions": {"formats": ["markdown"], "onlyMainContent": true},
    }))
}

/// The fixed extraction schema. Never model-authored, so cost and shape stay predictable.
pub fn extract_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "org_name": {"type": "string"},
            "legal_name": {"type": "string"},
            "domain": {"type": "string"},
            "emails": {"type": "array", "items": {"type": "string"}},
            "social_profiles": {"type": "array", "items": {"type": "string"}},
            "people": {"type": "array", "items": {"type": "object", "properties": {"name": {"type": "string"}, "title": {"type": "string"}}}},
            "address": {"type": "string"}
        }
    })
}

pub fn firecrawl_extract_body(v: &Value) -> Result<Value> {
    let page = url_arg(str_arg(v, "url")?)?;
    Ok(json!({
        "url": page,
        "formats": [{"type": "json", "schema": extract_schema(), "prompt": "Extract the organization or person this page belongs to: names, legal name, domain, contact emails, social profile URLs, named people with titles, and postal address. Leave a field empty when the page does not state it."}],
        "onlyMainContent": false,
    }))
}

/// Firecrawl request for the tools added in #27. `poll` is the status base for jobs.
pub fn firecrawl_request(id: &str, v: &Value) -> Result<Request> {
    let post = |path: &str, body: Value, poll: Option<&'static str>| -> Result<Request> {
        Ok(Request { url: Url::parse(&format!("{FIRECRAWL_BASE}/{path}"))?, body: Some(body), form: None, ndjson: false, poll })
    };
    match id {
        "firecrawl_search" => post("search", firecrawl_search_body(v)?, None),
        "firecrawl_scrape" => post("scrape", firecrawl_scrape_body(v)?, None),
        "firecrawl_map" => post("map", firecrawl_map_body(v)?, None),
        "firecrawl_batch_scrape" => post("batch/scrape", firecrawl_batch_body(v)?, Some("https://api.firecrawl.dev/v2/batch/scrape")),
        "firecrawl_crawl" => post("crawl", firecrawl_crawl_body(v)?, Some("https://api.firecrawl.dev/v2/crawl")),
        "firecrawl_extract" => post("scrape", firecrawl_extract_body(v)?, None),
        _ => Err(anyhow!("unknown tool {id}")),
    }
}

/// The only host a primary-provider request may reach. The executor refuses any other.
pub fn locked_host(id: &str) -> Option<&'static str> {
    let id = super::canonical_tool_id(id);
    if id.starts_with("firecrawl_") {
        Some("api.firecrawl.dev")
    } else if id.starts_with("sociavault_") {
        Some("api.sociavault.com")
    } else if id.starts_with("hunter_") {
        Some("api.hunter.io")
    } else {
        super::news_legal::locked_host(id)
    }
}

/// Map ranking: contact, about, team, leadership, press, legal, imprint pages first.
pub fn map_rank(link: &str) -> (usize, usize) {
    const PREFERRED: &[&str] = &["contact", "about", "team", "leadership", "press", "legal", "imprint"];
    let path = Url::parse(link).map(|url| url.path().to_ascii_lowercase()).unwrap_or_default();
    let rank = PREFERRED.iter().position(|word| path.contains(word)).unwrap_or(PREFERRED.len());
    (rank, path.len())
}

/// `firecrawl_map` links on the mapped site only, ranked, at most 25.
pub fn map_observations(value: &Value, site: &str) -> Value {
    let site = site.trim_start_matches("www.").to_ascii_lowercase();
    let mut links: Vec<String> = value
        .get("links")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|row| row.as_str().or_else(|| row.get("url").and_then(Value::as_str)))
        .filter(|link| {
            Url::parse(link).ok().and_then(|url| url.host_str().map(|host| host.trim_start_matches("www.").to_ascii_lowercase())).is_some_and(|host| {
                site.is_empty() || host == site || host.ends_with(&format!(".{site}"))
            })
        })
        .map(str::to_string)
        .collect();
    let mut seen = std::collections::HashSet::new();
    links.retain(|link| seen.insert(link.clone()));
    links.sort_by_key(|link| map_rank(link));
    let total = links.len();
    links.truncate(25);
    json!({"site": site, "urls": links, "total": total, "evidence_form": "map"})
}

/// Pages from a batch-scrape or crawl status payload.
pub fn job_observations(value: &Value, partial: bool) -> (Value, bool) {
    let mut clipped_any = false;
    let pages: Vec<Value> = value
        .get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(BATCH_SCRAPE_MAX_URLS)
        .map(|page| {
            let markdown = page.get("markdown").and_then(Value::as_str).unwrap_or("");
            let clipped: String = markdown.chars().take(2500).collect();
            clipped_any |= clipped.chars().count() < markdown.chars().count();
            json!({
                "url": page.pointer("/metadata/sourceURL").or_else(|| page.pointer("/metadata/url")).and_then(Value::as_str).unwrap_or(""),
                "title": clip_text(page.pointer("/metadata/title").and_then(Value::as_str).unwrap_or("")),
                "markdown": clipped,
            })
        })
        .collect();
    (
        json!({
            "status": if partial { "partial" } else { value.get("status").and_then(Value::as_str).unwrap_or("") },
            "completed": value.get("completed"),
            "total": value.get("total"),
            "pages": pages,
            "evidence_form": "page",
        }),
        clipped_any || partial,
    )
}

pub fn extract_observations(value: &Value) -> Value {
    let data = value.get("data").unwrap_or(value);
    let found = data.get("json").or_else(|| data.get("extract")).cloned().unwrap_or(Value::Null);
    let text = |key: &str| found.get(key).and_then(Value::as_str).map(clip_text).unwrap_or_default();
    let list = |key: &str, limit: usize| -> Vec<Value> {
        found.get(key).and_then(Value::as_array).map(|items| items.iter().take(limit).cloned().collect()).unwrap_or_default()
    };
    json!({
        "url": data.pointer("/metadata/sourceURL").and_then(Value::as_str).unwrap_or(""),
        "org_name": text("org_name"),
        "legal_name": text("legal_name"),
        "domain": text("domain"),
        "emails": list("emails", 10),
        "social_profiles": list("social_profiles", 10),
        "people": list("people", 10),
        "address": text("address"),
        "evidence_form": "extract",
    })
}

// ---------------------------------------------------------------------------
// Hunter
// ---------------------------------------------------------------------------

pub fn hunter_request(id: &str, v: &Value) -> Result<Request> {
    let q = |path: &str, pairs: &[(&str, &str)]| Ok(get(url(HUNTER_BASE, &[path], pairs)?));
    let domain_or_company = || -> Result<(&'static str, String)> {
        if let Ok(raw) = str_arg(v, "domain") {
            Ok(("domain", domain(raw)?))
        } else {
            Ok(("company", bounded(str_arg(v, "company").map_err(|_| anyhow!("provide domain or company"))?)?))
        }
    };
    match id {
        "hunter_domain_finder" => {
            let company = bounded(str_arg(v, "company")?)?;
            ensure!(company.chars().count() >= 3, "company must be at least 3 characters");
            let limit = number_arg(v, "limit", 5, 10)?;
            let mut pairs = vec![("company", company.as_str()), ("limit", limit.as_str())];
            let perfect = match v.get("perfect_match") {
                None => None,
                Some(Value::Bool(flag)) => Some(*flag),
                Some(Value::String(text)) if matches!(text.as_str(), "true" | "false") => Some(text == "true"),
                Some(_) => return Err(anyhow!("perfect_match must be true or false")),
            };
            if let Some(flag) = perfect {
                pairs.push(("perfect_match", if flag { "true" } else { "false" }));
            }
            q("domain-finder", &pairs)
        }
        "hunter_email_count" => {
            let (key, value) = domain_or_company()?;
            let mut pairs = vec![(key, value.as_str())];
            let kind = str_arg(v, "type").ok().map(str::to_ascii_lowercase);
            if let Some(kind) = kind.as_deref() {
                ensure!(matches!(kind, "personal" | "generic"), "type must be personal or generic");
                pairs.push(("type", kind));
            }
            q("email-count", &pairs)
        }
        "hunter_company_enrichment" => {
            let d = domain(str_arg(v, "domain")?)?;
            Ok(get(url(HUNTER_BASE, &["companies", "find"], &[("domain", &d)])?))
        }
        "hunter_email_insight" => {
            let email = email_address(str_arg(v, "email")?)?;
            q("email-insight", &[("email", &email)])
        }
        "hunter_person_enrichment" => {
            if let Ok(raw) = str_arg(v, "email") {
                let email = email_address(raw)?;
                Ok(get(url(HUNTER_BASE, &["people", "find"], &[("email", &email)])?))
            } else {
                let handle = linkedin_handle(str_arg(v, "linkedin_handle").map_err(|_| anyhow!("provide email or linkedin_handle"))?)?;
                Ok(get(url(HUNTER_BASE, &["people", "find"], &[("linkedin_handle", &handle)])?))
            }
        }
        "hunter_combined_enrichment" => {
            let email = email_address(str_arg(v, "email")?)?;
            let host = email.rsplit('@').next().unwrap_or("");
            ensure!(!webmail_host(host), "combined enrichment needs a company email; use hunter_person_enrichment for webmail");
            Ok(get(url(HUNTER_BASE, &["combined", "find"], &[("email", &email)])?))
        }
        _ => Err(anyhow!("unknown tool {id}")),
    }
}

fn social_handles(data: &Value) -> Value {
    let mut social = serde_json::Map::new();
    for platform in ["twitter", "facebook", "instagram", "github", "youtube", "linkedin"] {
        // Only LinkedIn handles carry a path (`company/hunterio`, `in/jane`).
        let fits = |handle: &&str| !handle.is_empty() && (platform == "linkedin" || !handle.contains('/'));
        if let Some(handle) = data.pointer(&format!("/{platform}/handle")).and_then(Value::as_str).filter(fits) {
            social.insert(platform.into(), json!({"handle": handle}));
        }
    }
    Value::Object(social)
}

fn company_card(data: &Value) -> Value {
    let listed = |key: &str, limit: usize| -> Vec<String> {
        data.get(key)
            .and_then(Value::as_array)
            .map(|rows| rows.iter().filter_map(Value::as_str).filter(|item| !item.is_empty()).take(limit).map(str::to_string).collect())
            .unwrap_or_default()
    };
    let address = ["city", "state", "country"]
        .iter()
        .filter_map(|key| data.pointer(&format!("/geo/{key}")).and_then(Value::as_str))
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(", ");
    json!({
        "name": data.get("name"),
        "legal_name": data.get("legalName"),
        "domain": data.get("domain"),
        "parent_domain": data.pointer("/parent/domain"),
        "description": data.get("description").and_then(Value::as_str).map(clip_text),
        "location": data.get("location"),
        "address": if address.is_empty() { Value::Null } else { json!(address) },
        "industry": data.pointer("/category/industry"),
        "employees": data.pointer("/metrics/employees"),
        "tech": listed("tech", 40),
        "tech_categories": listed("techCategories", 20),
        "emails": data.pointer("/site/emailAddresses").and_then(Value::as_array).map(|rows| {
            rows.iter().filter_map(Value::as_str).take(8).map(str::to_string).collect::<Vec<_>>()
        }).unwrap_or_default(),
        "social": social_handles(data),
    })
}

fn person_card(data: &Value) -> Value {
    json!({
        "full_name": data.pointer("/name/fullName"),
        "email": data.get("email"),
        "location": data.get("location"),
        "employer": data.pointer("/employment/name"),
        "employer_domain": data.pointer("/employment/domain"),
        "title": data.pointer("/employment/title"),
        "linkedin_handle": data.pointer("/linkedin/handle"),
        "social": social_handles(data),
    })
}

/// Observations for the Hunter tools added in #27 and the renamed company enrichment.
pub fn hunter_observations(id: &str, value: &Value) -> Option<Value> {
    let data = value.get("data").unwrap_or(&Value::Null);
    Some(match id {
        "hunter_domain_finder" => {
            let rows: Vec<Value> = match data {
                Value::Array(items) => items.clone(),
                Value::Object(_) => vec![data.clone()],
                _ => Vec::new(),
            };
            let companies: Vec<Value> = rows
                .iter()
                .take(10)
                .map(|row| json!({"domain": row.get("domain"), "company_name": row.get("company_name").or_else(|| row.get("organization")), "email_count": row.get("email_count")}))
                .collect();
            json!({"companies": companies})
        }
        "hunter_email_count" => json!({
            "total": data.get("total"),
            "personal_emails": data.get("personal_emails"),
            "generic_emails": data.get("generic_emails"),
        }),
        "hunter_company_enrichment" => company_card(data),
        "hunter_email_insight" => json!({
            "email": data.get("email"),
            "gibberish": data.get("gibberish"),
            "pattern": data.get("pattern"),
            "mx_records": data.get("mx_records"),
            "disposable": data.get("disposable"),
            "webmail": data.get("webmail"),
        }),
        "hunter_person_enrichment" => person_card(data),
        "hunter_combined_enrichment" => {
            let person = person_card(data.get("person").unwrap_or(&Value::Null));
            let company = company_card(data.get("company").unwrap_or(&Value::Null));
            json!({"person": person, "company": company})
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::super::{canonical_tool_id, provider_credential, request, ProviderKeys};
    use super::*;

    const TEST_KEY: &str = "test-key-not-a-secret";

    fn keys() -> ProviderKeys {
        ProviderKeys { firecrawl: TEST_KEY.into(), hunter: TEST_KEY.into(), sociavault: TEST_KEY.into(), newsapi: TEST_KEY.into(), courtlistener: TEST_KEY.into() }
    }

    fn query(req: &Request) -> Vec<(String, String)> {
        req.url.query_pairs().map(|(key, value)| (key.into_owned(), value.into_owned())).collect()
    }

    /// The credential header a tool sends, checked without printing the key.
    fn header(id: &str) -> (String, bool) {
        let (name, value) = provider_credential(id, &keys()).unwrap().expect("keyed tool");
        let carries_key = value == TEST_KEY || value == format!("Bearer {TEST_KEY}");
        (name.as_str().to_string(), carries_key && (name.as_str() != "authorization" || value.starts_with("Bearer ")))
    }

    fn assert_host_locked(id: &str, req: &Request) {
        assert_eq!(req.url.scheme(), "https", "{id}");
        assert_eq!(req.url.host_str(), locked_host(id), "{id}: host lock");
        assert!(req.url.username().is_empty() && req.url.port().is_none(), "{id}");
    }

    /// Arguments that select `route` and the query pairs it must send.
    fn fixture(route: &SociaVaultRoute) -> (Value, Vec<(&'static str, &'static str)>) {
        let mut args = json!({"platform": route.platform, "endpoint": route.endpoint});
        let expected: Vec<(&str, &str)> = match route.input {
            Handle => {
                args["handle"] = json!("example");
                vec![("handle", "example")]
            }
            UserId => {
                args["user_id"] = json!("44196397");
                vec![(route.param, "44196397")]
            }
            HandleOrUserId => {
                args["handle"] = json!("example");
                vec![("handle", "example")]
            }
            FacebookUrl => {
                args["handle"] = json!("examplepage");
                vec![("url", "https://www.facebook.com/examplepage")]
            }
            LinkedinProfileUrl => {
                args["handle"] = json!("jane-example");
                vec![("url", "https://www.linkedin.com/in/jane-example")]
            }
            LinkedinCompanyUrl => {
                args["handle"] = json!("acme-robotics");
                vec![("url", "https://www.linkedin.com/company/acme-robotics")]
            }
            YoutubeChannel => {
                args["handle"] = json!("example");
                vec![("handle", "example")]
            }
            Query => {
                args["query"] = json!("jane example");
                vec![("query", "jane example")]
            }
            Hashtag => {
                args["query"] = json!("#Open Source");
                vec![("hashtag", "OpenSource")]
            }
            SubredditQuery => {
                args["subreddit"] = json!("r/rust");
                args["query"] = json!("argos");
                vec![("subreddit", "rust"), ("query", "argos")]
            }
        };
        if route.tool == "sociavault_google_search" {
            args.as_object_mut().unwrap().remove("platform");
            args.as_object_mut().unwrap().remove("endpoint");
        }
        (args, expected)
    }

    #[test]
    fn every_sociavault_route_builds_a_host_locked_get_with_its_params_and_key_header() {
        for route in SOCIAVAULT_ROUTES {
            let (args, expected) = fixture(route);
            let name = format!("{} {}:{}", route.tool, route.platform, route.endpoint);
            super::super::validate(route.tool, &args).unwrap_or_else(|err| panic!("{name}: {err}"));
            assert_eq!(select_route(route.tool, &args).unwrap().path, route.path, "{name}");
            let req = request(route.tool, &args).unwrap_or_else(|err| panic!("{name}: {err}"));
            assert_host_locked(route.tool, &req);
            assert_eq!(req.url.path(), format!("/v1/scrape/{}", route.path), "{name}");
            assert!(req.body.is_none() && req.form.is_none() && req.poll.is_none(), "{name}: GET only");
            let sent = query(&req);
            let wanted: Vec<(String, String)> = expected.iter().map(|(key, value)| (key.to_string(), value.to_string())).collect();
            assert_eq!(sent, wanted, "{name}");
            assert_eq!(header(route.tool), ("x-api-key".to_string(), true), "{name}");
        }
    }

    #[test]
    fn sociavault_alternate_inputs_pick_the_right_param() {
        let ig = request("sociavault_user_content", &json!({"platform": "instagram", "endpoint": "reels", "user_id": "25025320"})).unwrap();
        assert_eq!(query(&ig), vec![("user_id".to_string(), "25025320".to_string())]);
        let yt = request("sociavault_user_content", &json!({"platform": "youtube", "user_id": "UCX6OQ3DkcsbYNE6H8uQQuVA"})).unwrap();
        assert_eq!(yt.url.path(), "/v1/scrape/youtube/channel-videos");
        assert_eq!(query(&yt), vec![("channelId".to_string(), "UCX6OQ3DkcsbYNE6H8uQQuVA".to_string())]);
        // Without an endpoint, a LinkedIn company URL selects the company route.
        let company = request("sociavault_profile", &json!({"platform": "linkedin", "handle": "https://www.linkedin.com/company/acme-robotics/"})).unwrap();
        assert_eq!(company.url.path(), "/v1/scrape/linkedin/company");
        let hunter_style = request("sociavault_profile", &json!({"platform": "linkedin", "handle": "company/acme-robotics"})).unwrap();
        assert_eq!(hunter_style.url.path(), "/v1/scrape/linkedin/company");
        assert_eq!(query(&hunter_style), vec![("url".to_string(), "https://www.linkedin.com/company/acme-robotics".to_string())]);
        assert!(request("sociavault_profile", &json!({"platform": "linkedin", "endpoint": "profile", "handle": "company/acme-robotics"})).is_err());
        // Platform aliases normalize; a platform the tool does not serve is rejected.
        assert_eq!(request("sociavault_profile", &json!({"platform": "X", "handle": "@example"})).unwrap().url.path(), "/v1/scrape/twitter/profile");
        assert!(request("sociavault_profile", &json!({"platform": "reddit", "handle": "example"})).is_err());
        assert!(request("sociavault_search_users", &json!({"platform": "twitter", "query": "jane"})).is_err());
        // A handle cannot steer the request off the provider host.
        for hostile in ["https://evil.example/x", "../../admin", "example?api_key=1"] {
            if let Ok(req) = request("sociavault_profile", &json!({"platform": "twitter", "handle": hostile})) {
                assert_host_locked("sociavault_profile", &req);
                assert_eq!(req.url.path(), "/v1/scrape/twitter/profile");
            }
        }
    }

    #[test]
    fn the_route_matrix_is_44_routes_without_followers_or_single_posts() {
        assert_eq!(SOCIAVAULT_ROUTES.len(), 44);
        let count = |tool: &str| sociavault_routes(tool).count();
        assert_eq!(
            [count("sociavault_google_search"), count("sociavault_profile"), count("sociavault_search"), count("sociavault_search_users"), count("sociavault_user_content")],
            [1, 10, 12, 3, 18]
        );
        assert!(SOCIAVAULT_ROUTES.iter().all(|route| SOCIAVAULT_TOOLS.contains(&route.tool)));
        let mut seen = std::collections::HashSet::new();
        for route in SOCIAVAULT_ROUTES {
            assert!(seen.insert((route.tool, route.platform, route.endpoint)), "duplicate {route:?}");
            assert!(seen.insert(("path", route.path, "")), "duplicate path {}", route.path);
            for segment in route.path.split('/') {
                for banned in ["follow", "comment", "transcript", "repost", "retweet", "like"] {
                    assert!(!segment.contains(banned), "{} is a follower or single-post route", route.path);
                }
                // Single-item routes (one post, tweet, video, reel, pin) are excluded.
                assert!(!["post", "tweet", "video", "reel", "pin", "media", "details", "info"].contains(&segment), "{} is a single-item route", route.path);
            }
            assert_eq!(route.input == UserId, !route.param.is_empty(), "{}", route.path);
        }
        for endpoint in ["followers", "following", "comments", "post", "tweet", "transcript"] {
            assert!(select_route("sociavault_user_content", &json!({"platform": "twitter", "handle": "example", "endpoint": endpoint})).is_err(), "{endpoint}");
        }
        assert!(select_route("sociavault_profile", &json!({"platform": "instagram", "handle": "example", "endpoint": "post_details"})).is_err());
    }

    #[test]
    fn endpoint_hints_come_from_the_question_text() {
        assert_eq!(sociavault_endpoint_hint("sociavault_user_content", "instagram", "Show her latest reels"), Some("reels"));
        assert_eq!(sociavault_endpoint_hint("sociavault_user_content", "youtube", "any community posts?"), Some("community_posts"));
        assert_eq!(sociavault_endpoint_hint("sociavault_search", "tiktok", "videos tagged #launch"), Some("hashtag"));
        assert_eq!(sociavault_endpoint_hint("sociavault_search", "reddit", "what does r/rust say"), Some("subreddit"));
        assert_eq!(sociavault_endpoint_hint("sociavault_user_content", "twitter", "recent tweets"), None, "the default route needs no hint");
        // Routes that need a numeric id are never inferred from text.
        assert_eq!(sociavault_endpoint_hint("sociavault_profile", "instagram", "basic profile"), None);
    }

    #[test]
    fn sociavault_parsers_read_ids_and_google_results() {
        assert_eq!(sociavault_platform_id("twitter", &json!({"data": {"rest_id": "44196397"}})).as_deref(), Some("44196397"));
        assert_eq!(sociavault_platform_id("instagram", &json!({"data": {"data": {"user": {"id": "25025320"}}}})).as_deref(), Some("25025320"));
        assert_eq!(sociavault_platform_id("youtube", &json!({"data": {"channelId": "UCX6OQ3DkcsbYNE6H8uQQuVA"}})).as_deref(), Some("UCX6OQ3DkcsbYNE6H8uQQuVA"));
        let google = sociavault_items("sociavault_google_search", &json!({"data": {"results": {
            "0": {"title": "Acme Robotics", "url": "https://acmerobotics.com/", "description": "Industrial robots"},
            "1": {"title": "Acme on X", "url": "https://x.com/acmerobotics", "description": "Posts"}
        }}}));
        let rows = google["results"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["url"], "https://acmerobotics.com/");
    }

    fn post(id: &str, args: Value) -> Request {
        let req = request(id, &args).unwrap_or_else(|err| panic!("{id}: {err}"));
        assert_host_locked(id, &req);
        assert!(req.body.is_some() && req.form.is_none(), "{id}: POST with a JSON body");
        assert_eq!(header(id), ("authorization".to_string(), true), "{id}: Bearer key");
        req
    }

    #[test]
    fn every_firecrawl_tool_builds_a_host_locked_post() {
        let search = post("firecrawl_search", json!({"query": "acme robotics", "sources": ["web", "news"], "categories": ["github"], "tbs": "qdr:m", "limit": 10}));
        assert_eq!(search.url.path(), "/v2/search");
        let body = search.body.unwrap();
        assert_eq!(body["limit"], 10);
        assert!(request("firecrawl_search", &json!({"query": "x", "limit": 11})).is_err(), "limit is bounded");
        assert_eq!(body["sources"], json!(["web", "news"]));
        assert_eq!(body["categories"], json!([{"type": "developer"}]), "github maps to Firecrawl's developer category");
        assert_eq!(body["tbs"], "qdr:m");
        assert!(request("firecrawl_search", &json!({"query": "x", "categories": ["images"]})).is_err());

        let scrape = post("firecrawl_scrape", json!({"url": "https://acmerobotics.com/about", "formats": ["markdown", "links"]}));
        assert_eq!(scrape.url.path(), "/v2/scrape");
        assert_eq!(scrape.body.unwrap()["formats"], json!(["markdown", "links"]));
        assert!(request("firecrawl_scrape", &json!({"url": "https://acmerobotics.com", "formats": ["json"]})).unwrap_err().to_string().contains("firecrawl_extract"));

        let map = post("firecrawl_map", json!({"domain": "acmerobotics.com", "search": "contact"}));
        assert_eq!(map.url.path(), "/v2/map");
        assert_eq!(map.body.unwrap(), json!({"url": "https://acmerobotics.com", "limit": 50, "includeSubdomains": false, "sitemap": "include", "search": "contact"}));
        assert!(request("firecrawl_map", &json!({"domain": "x.com"})).is_err(), "never maps a social host");

        let batch = post("firecrawl_batch_scrape", json!({"urls": ["https://acmerobotics.com/about", "https://acmerobotics.com/contact", "https://acmerobotics.com/about"]}));
        assert_eq!(batch.url.path(), "/v2/batch/scrape");
        assert_eq!(batch.poll, Some("https://api.firecrawl.dev/v2/batch/scrape"));
        assert_eq!(batch.body.unwrap()["urls"].as_array().unwrap().len(), 2, "duplicates dropped");
        let eleven: Vec<String> = (0..11).map(|n| format!("https://acmerobotics.com/p{n}")).collect();
        assert!(request("firecrawl_batch_scrape", &json!({"urls": eleven})).is_err());

        assert!(request("firecrawl_crawl", &json!({"domain": "acmerobotics.com", "limit": 50})).is_err(), "small crawls only");
        let crawl = post("firecrawl_crawl", json!({"domain": "acmerobotics.com"}));
        assert_eq!(crawl.url.path(), "/v2/crawl");
        assert_eq!(crawl.poll, Some("https://api.firecrawl.dev/v2/crawl"));
        let body = crawl.body.unwrap();
        assert_eq!(body["limit"], CRAWL_MAX_PAGES);
        assert_eq!(body["maxDiscoveryDepth"], 1);
        assert_eq!(body["crawlEntireDomain"], false);

        let extract = post("firecrawl_extract", json!({"url": "https://acmerobotics.com/about"}));
        assert_eq!(extract.url.path(), "/v2/scrape");
        let body = extract.body.unwrap();
        assert_eq!(body["formats"][0]["type"], "json");
        assert_eq!(body["formats"][0]["schema"], extract_schema(), "the schema is fixed, never model-authored");
    }

    fn hunter(id: &str, args: Value, path: &str, pairs: &[(&str, &str)]) {
        let req = request(id, &args).unwrap_or_else(|err| panic!("{id}: {err}"));
        assert_host_locked(id, &req);
        assert!(req.body.is_none() && req.form.is_none(), "{id}: GET");
        assert_eq!(req.url.path(), path, "{id}");
        let mut wanted: Vec<(String, String)> = pairs.iter().map(|(key, value)| (key.to_string(), value.to_string())).collect();
        let mut sent = query(&req);
        wanted.sort();
        sent.sort();
        assert_eq!(sent, wanted, "{id}");
        assert!(!req.url.as_str().contains(TEST_KEY), "{id}: the key never rides in the URL");
        assert_eq!(header(id), ("x-api-key".to_string(), true), "{id}");
    }

    #[test]
    fn every_hunter_tool_builds_a_host_locked_get() {
        hunter("hunter_domain_finder", json!({"company": "Acme Robotics", "perfect_match": true}), "/v2/domain-finder", &[("company", "Acme Robotics"), ("limit", "5"), ("perfect_match", "true")]);
        hunter("hunter_email_count", json!({"domain": "acmerobotics.com", "type": "personal"}), "/v2/email-count", &[("domain", "acmerobotics.com"), ("type", "personal")]);
        hunter("hunter_email_count", json!({"company": "Acme Robotics"}), "/v2/email-count", &[("company", "Acme Robotics")]);
        hunter("hunter_domain_search", json!({"domain": "acmerobotics.com"}), "/v2/domain-search", &[("domain", "acmerobotics.com"), ("limit", "10")]);
        hunter("hunter_email_finder", json!({"domain": "acmerobotics.com", "full_name": "Jane Example"}), "/v2/email-finder", &[("domain", "acmerobotics.com"), ("full_name", "Jane Example")]);
        hunter("hunter_email_verifier", json!({"email": "jane@acmerobotics.com"}), "/v2/email-verifier", &[("email", "jane@acmerobotics.com")]);
        hunter("hunter_company_enrichment", json!({"domain": "acmerobotics.com"}), "/v2/companies/find", &[("domain", "acmerobotics.com")]);
        hunter("hunter_tech_lookup", json!({"domain": "acmerobotics.com"}), "/v2/companies/find", &[("domain", "acmerobotics.com")]);
        hunter("hunter_email_insight", json!({"email": "jane@acmerobotics.com"}), "/v2/email-insight", &[("email", "jane@acmerobotics.com")]);
        hunter("hunter_person_enrichment", json!({"email": "jane.example@gmail.com"}), "/v2/people/find", &[("email", "jane.example@gmail.com")]);
        hunter("hunter_person_enrichment", json!({"linkedin_handle": "jane-example"}), "/v2/people/find", &[("linkedin_handle", "jane-example")]);
        hunter("hunter_combined_enrichment", json!({"email": "jane@acmerobotics.com"}), "/v2/combined/find", &[("email", "jane@acmerobotics.com")]);
        assert!(request("hunter_combined_enrichment", &json!({"email": "jane@gmail.com"})).is_err(), "webmail goes to person enrichment");
        assert!(request("hunter_domain_finder", &json!({"company": "Ac"})).is_err(), "company needs three characters");
        assert_eq!(canonical_tool_id("hunter_tech_lookup"), "hunter_company_enrichment");
    }

    #[test]
    fn map_and_job_parsers_rank_filter_and_mark_partial_jobs() {
        let map = map_observations(&json!({"success": true, "links": [
            {"url": "https://acmerobotics.com/blog/robots"},
            {"url": "https://acmerobotics.com/contact"},
            {"url": "https://x.com/acmerobotics"},
            "https://www.acmerobotics.com/about-us",
            {"url": "https://acmerobotics.com/contact"}
        ]}), "acmerobotics.com");
        let urls: Vec<&str> = map["urls"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
        assert_eq!(urls.len(), 3, "off-site and duplicate links dropped: {urls:?}");
        assert!(urls[..2].iter().all(|url| url.contains("contact") || url.contains("about")), "{urls:?}");
        assert!(map_rank("https://acmerobotics.com/contact") < map_rank("https://acmerobotics.com/blog/robots"));
        let (job, truncated) = job_observations(&json!({"status": "scraping", "total": 3, "completed": 1, "data": [
            {"markdown": "# About Acme", "metadata": {"sourceURL": "https://acmerobotics.com/about", "title": "About"}}
        ]}), true);
        assert_eq!(job["status"], "partial");
        assert_eq!(job["pages"][0]["url"], "https://acmerobotics.com/about");
        assert!(truncated);
        let (done, _) = job_observations(&json!({"status": "completed", "total": 1, "completed": 1, "data": []}), false);
        assert_eq!(done["status"], "completed");
    }

    #[test]
    fn hunter_parsers_shape_cards_and_domain_matches() {
        let found = hunter_observations("hunter_domain_finder", &json!({"data": {"domain": "acmerobotics.com", "company_name": "Acme Robotics", "email_count": 12}})).unwrap();
        assert_eq!(found["companies"][0]["domain"], "acmerobotics.com");
        let count = hunter_observations("hunter_email_count", &json!({"data": {"total": 0, "personal_emails": 0, "generic_emails": 0}})).unwrap();
        assert_eq!(count["total"], 0);
        let person = hunter_observations("hunter_person_enrichment", &json!({"data": {"name": {"fullName": "Jane Example"}, "employment": {"name": "Acme Robotics", "domain": "acmerobotics.com"}, "twitter": {"handle": "janeexample"}}})).unwrap();
        assert_eq!(person["full_name"], "Jane Example");
        assert_eq!(person["social"]["twitter"]["handle"], "janeexample");
    }

    #[test]
    fn crawl_is_off_by_default_and_costs_scale_with_pages() {
        assert!(!super::super::default_enabled("firecrawl_crawl"));
        assert!(super::super::default_enabled("firecrawl_map"));
        let urls: Vec<String> = (0..3).map(|n| format!("https://acmerobotics.com/p{n}")).collect();
        assert_eq!(super::super::estimated_cost("firecrawl_batch_scrape", &json!({"urls": urls})).unwrap().credits, 3);
        assert_eq!(super::super::estimated_cost("firecrawl_crawl", &json!({"domain": "acmerobotics.com", "limit": 4})).unwrap().credits, 4);
        for id in SOCIAVAULT_TOOLS {
            assert_eq!(super::super::endpoint_cost(id).unwrap().credits, 1, "{id}: 1-credit routes only");
        }
        for id in ["hunter_domain_finder", "hunter_email_count", "hunter_email_insight"] {
            assert_eq!(super::super::endpoint_cost(id).unwrap().credits, 0, "{id} is free");
        }
    }
}
