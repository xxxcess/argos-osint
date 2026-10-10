//! Named search-engine SERP adapters via Firecrawl `/v2/scrape` (§9).
//!
//! Per spec `ARGOS_PROVIDER_QUEUE_AND_PROFILE_DASHBOARD_SPEC.md` §9 this module
//! owns the truthful outcome contract for the three named engine tools:
//! `firecrawl_google_search`, `firecrawl_yandex_search`, `firecrawl_mojeek_search`.
//!
//! Rules that are load-bearing here:
//! - Unknown/empty HTML is **never** a verified zero. It is `ParserMismatch`.
//! - No-results is only recognised inside a recognized engine status region,
//!   against a supported phrase list, with zero accepted organic cards.
//! - Challenge/consent is detected structurally (forms, iframes, meta refresh),
//!   never by scanning the whole document's script/snippet text.
//! - The original query/limit/engine/requested URL come from [`SerpRequest`],
//!   never from a redirected URL.

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

/// Boundary marker for DOM-shape changes. Included in the cache identity so a
/// parser upgrade invalidates legacy v1 entries without deleting them.
pub const PARSER_VERSION: &str = "2.0";

/// The fetch contract (formats requested, cache policy) carried in cache identity.
pub const FETCH_CONTRACT_VERSION: &str = "2.0";

/// Named SERP bodies are capped at 8 MiB (spec §9 fix 6).
pub const SERP_MAX_BODY_BYTES: usize = 8 * 1024 * 1024;

/// The three named engine tools, in catalog order.
pub const NAMED_SERP_TOOLS: [&str; 3] = [
    FIRECRAWL_GOOGLE_SEARCH,
    FIRECRAWL_YANDEX_SEARCH,
    FIRECRAWL_MOJEEK_SEARCH,
];

/// Maximum number of distinct rejection reasons kept in a diagnostics snapshot.
const MAX_REJECTION_REASONS: usize = 16;

/// Maximum characters kept for a bounded text field in diagnostics.
const MAX_DIAGNOSTIC_TEXT: usize = 240;

/// Maximum characters kept for a single item title/snippet.
const MAX_ITEM_TEXT: usize = 600;

/// Maximum engine-redirect unwrap depth (spec §9 fix 5).
const MAX_REDIRECT_DEPTH: usize = 3;

/// Engine short names.
const GOOGLE: &str = "google";
const YANDEX: &str = "yandex";
const MOJEEK: &str = "mojeek";

/// True when `tool_id` is one of the three named engine tools.
pub fn is_named_engine_tool(tool_id: &str) -> bool {
    NAMED_SERP_TOOLS.contains(&tool_id)
}

/// Engine short name for a tool id, or `None` when the tool is not a named engine.
fn engine_for_tool(tool_id: &str) -> Option<&'static str> {
    match tool_id {
        FIRECRAWL_GOOGLE_SEARCH => Some(GOOGLE),
        FIRECRAWL_YANDEX_SEARCH => Some(YANDEX),
        FIRECRAWL_MOJEEK_SEARCH => Some(MOJEEK),
        _ => None,
    }
}

/// Stable family of the tool for telemetry grouping. Named engines group under
/// `named_serp`; everything else keeps its own family so a fallback such as
/// `firecrawl_search` is never attributed to an engine.
pub fn engine_family(tool_id: &str) -> &'static str {
    if is_named_engine_tool(tool_id) {
        "named_serp"
    } else {
        "other"
    }
}

/// One typed terminal outcome. Only `Valid` and `VerifiedZero` are successes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SerpOutcome {
    /// At least one accepted organic card.
    Valid,
    /// A recognized engine status region with an explicit supported no-results
    /// phrase and zero accepted cards.
    VerifiedZero,
    /// A real challenge interstitial (captcha/sorry form or widget).
    Challenge,
    /// A real consent interstitial.
    Consent,
    /// Target status 429 or an explicit rate-limit phrase in a status region.
    RateLimited,
    /// Page present but nothing recognized: unknown DOM, links-only, missing HTML.
    ParserMismatch,
    /// Provider/envelope failure: `success == false`, non-2xx/3xx target status,
    /// HTTP 404, or an explicit provider error.
    UpstreamFailure,
    /// Input bytes exceeded [`SERP_MAX_BODY_BYTES`] or the envelope says truncated.
    ResponseTooLarge,
}

impl SerpOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Valid => "valid",
            Self::VerifiedZero => "verified_zero",
            Self::Challenge => "challenge",
            Self::Consent => "consent",
            Self::RateLimited => "rate_limited",
            Self::ParserMismatch => "parser_mismatch",
            Self::UpstreamFailure => "upstream_failure",
            Self::ResponseTooLarge => "response_too_large",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "valid" => Some(Self::Valid),
            "verified_zero" => Some(Self::VerifiedZero),
            "challenge" => Some(Self::Challenge),
            "consent" => Some(Self::Consent),
            "rate_limited" => Some(Self::RateLimited),
            "parser_mismatch" => Some(Self::ParserMismatch),
            "upstream_failure" => Some(Self::UpstreamFailure),
            "response_too_large" => Some(Self::ResponseTooLarge),
            _ => None,
        }
    }

    /// Only `Valid` and `VerifiedZero` are successes.
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Valid | Self::VerifiedZero)
    }

    pub fn is_failure(&self) -> bool {
        !self.is_success()
    }
}

/// Which parser input was used, for provenance.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParserInput {
    /// Non-empty `rawHtml` was used.
    RawHtml,
    /// `rawHtml` was empty; cleaned `html` was used.
    CleanHtml,
    /// Only a `links` array was present, so no ranked result can be produced.
    LinksOnly,
    /// No DOM payload at all.
    #[default]
    Missing,
}

impl ParserInput {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::RawHtml => "raw_html",
            Self::CleanHtml => "clean_html",
            Self::LinksOnly => "links_only",
            Self::Missing => "missing",
        }
    }
}

/// Bounded redacted diagnostics. NEVER contains key material, prompts or full HTML.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SerpDiagnostics {
    pub target_status: Option<u16>,
    pub requested_url: String,
    pub final_url: String,
    pub provider_warning: String,
    pub provider_error: String,
    pub input_bytes: usize,
    pub parser_input: ParserInput,
    pub dom_candidates: usize,
    pub accepted: usize,
    /// `(reason, count)` pairs, bounded by [`MAX_REJECTION_REASONS`].
    pub rejections: Vec<(String, usize)>,
    pub status_region: String,
}

impl SerpDiagnostics {
    /// Records one rejection reason, bounded by [`MAX_REJECTION_REASONS`].
    pub fn record_rejection(&mut self, reason: &str) {
        let reason = reason.trim();
        let reason = if reason.is_empty() { "unknown" } else { reason };
        if let Some((_, count)) = self.rejections.iter_mut().find(|(r, _)| r == reason) {
            *count = count.saturating_add(1);
            return;
        }
        if self.rejections.len() >= MAX_REJECTION_REASONS {
            return;
        }
        self.rejections.push((reason.to_string(), 1));
    }

    /// Count of rejections recorded for `reason`.
    pub fn rejection(&self, reason: &str) -> usize {
        self.rejections
            .iter()
            .find(|(r, _)| r == reason)
            .map_or(0, |(_, count)| *count)
    }
}

/// One extracted result. `title` is the heading text when present.
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
    pub title: String,
    pub snippet: String,
    pub parser_version: String,
    pub outcome: String,
}

/// Request context that must never be reconstructed from a redirected URL.
#[derive(Clone, Debug)]
pub struct SerpRequest {
    pub engine_tool: String,
    pub engine: String,
    pub query: String,
    pub limit: usize,
    pub serp_url: String,
}

impl SerpRequest {
    /// Builds a request for a named engine tool. `query`, `limit`, `engine` and the
    /// requested SERP URL are carried verbatim from the original tool input.
    pub fn for_tool(engine_tool: &str, query: &str, limit: usize) -> Result<Self> {
        let engine = engine_for_tool(engine_tool)
            .ok_or_else(|| anyhow!("unsupported search engine tool: {engine_tool}"))?;
        let serp_url = build_serp_url(engine_tool, query)?;
        Ok(Self {
            engine_tool: engine_tool.to_string(),
            engine: engine.to_string(),
            query: query.to_string(),
            limit,
            serp_url,
        })
    }
}

/// Parse result: items, typed outcome and diagnostics.
#[derive(Clone, Debug)]
pub struct SerpParse {
    pub items: Vec<EngineSearchResult>,
    pub outcome: SerpOutcome,
    pub diagnostics: SerpDiagnostics,
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
///
/// Contract v2 (spec §9 fix 3): request `rawHtml` first with `html` fallback, keep
/// `onlyMainContent: false` so the results region is never stripped, keep `links`
/// for diagnostics only, and set `maxAge: 0` because Argos owns its own cache.
pub fn build_scrape_body(engine_tool: &str, query: &str) -> Result<Value> {
    let serp_url = build_serp_url(engine_tool, query)?;
    Ok(json!({
        "url": serp_url,
        "formats": ["rawHtml", "html", "links"],
        "onlyMainContent": false,
        "maxAge": 0
    }))
}

/// Canonical query for cache identity: trimmed, whitespace-collapsed, lowercased.
fn canonical_query(query: &str) -> String {
    query
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Locale signals carried by the SERP URL (`hl`, `lang`, `setlang`, `lr`, `gl`).
/// Two SERPs differing only in locale are different cache entries.
fn serp_locale(serp_url: &str) -> String {
    let Ok(parsed) = Url::parse(serp_url) else {
        return String::new();
    };
    for key in ["hl", "lang", "setlang", "lr", "gl"] {
        let value = parsed
            .query_pairs()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.into_owned());
        if let Some(value) = value.filter(|v| !v.is_empty()) {
            return format!("{key}={value}");
        }
    }
    String::new()
}

/// Canonical cache identity fragment: parser + fetch contract + canonical
/// query/locale/limit. Legacy v1 entries miss on the version pair.
pub fn cache_fragment(req: &SerpRequest) -> String {
    format!(
        "serp:{}:{}:{}:{}:{}:{}",
        PARSER_VERSION,
        FETCH_CONTRACT_VERSION,
        engine_family(&req.engine_tool),
        canonical_query(&req.query),
        serp_locale(&req.serp_url),
        req.limit
    )
}

/// True when a fresh same-engine fetch retry is allowed for this outcome.
///
/// Bounded recovery (spec §9 fix 6): only `ParserMismatch` from a DOM that was
/// actually present. A `LinksOnly` retry would re-request the same shape.
pub fn retry_allowed(outcome: SerpOutcome, diagnostics: &SerpDiagnostics) -> bool {
    outcome == SerpOutcome::ParserMismatch && diagnostics.parser_input != ParserInput::LinksOnly
}

/// Truncates a bounded diagnostic string.
fn clip_diag(s: &str) -> String {
    s.trim().chars().take(MAX_DIAGNOSTIC_TEXT).collect()
}

/// Collapses whitespace in extracted DOM text.
fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn clip_item_text(s: &str) -> String {
    collapse_ws(s).chars().take(MAX_ITEM_TEXT).collect()
}

/// Joins the text of a selector-matched element.
fn text_of(el: scraper::ElementRef) -> String {
    clip_item_text(&el.text().collect::<Vec<_>>().join(" "))
}

/// Reads a non-empty string field from a JSON object pointer.
fn str_at<'a>(v: &'a Value, path: &str) -> Option<&'a str> {
    v.pointer(path)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// Reads a numeric status from a JSON object pointer.
fn num_at(v: &Value, path: &str) -> Option<u16> {
    v.pointer(path)
        .and_then(Value::as_u64)
        .and_then(|n| u16::try_from(n).ok())
}

/// Supported per-engine phrase lists. Nothing here is matched against the whole page.
struct EnginePolicy {
    zero_phrases: &'static [&'static str],
    rate_limit_phrases: &'static [&'static str],
    /// Recognized engine status regions (CSS selectors).
    status_regions: &'static [&'static str],
    /// Hosts the engine owns, used for internal-link and SERP-host checks.
    hosts: &'static [&'static str],
    /// Organic card selectors.
    cards: &'static [&'static str],
    /// Structural heading-link fallback selectors.
    fallback_cards: &'static [&'static str],
}

const GOOGLE_POLICY: EnginePolicy = EnginePolicy {
    zero_phrases: &[
        "did not match any documents",
        "no results found for",
        "did not match any",
    ],
    rate_limit_phrases: &[
        "unusual traffic from your computer network",
        "unusual traffic",
        "rate limit",
        "too many requests",
    ],
    status_regions: &["#topstuff", "#main"],
    hosts: &["google.com", "www.google.com", "googleusercontent.com"],
    cards: &["div.g", "div.tF2Cxc", "div[data-snc]", "div.MjjYud"],
    fallback_cards: &["h3 a[href]"],
};

const YANDEX_POLICY: EnginePolicy = EnginePolicy {
    zero_phrases: &[
        "did not match any documents",
        "ничего не нашлось",
        "ничего не найдено",
        "no results found",
    ],
    rate_limit_phrases: &["too many requests", "rate limit", "quota exceeded"],
    status_regions: &[".misspell__message", ".serp-list"],
    hosts: &["yandex.com", "yandex.ru", "ya.ru"],
    cards: &["li.serp-item", "div.Organic", "div.serp-item"],
    fallback_cards: &["h2 a[href]"],
};

const MOJEEK_POLICY: EnginePolicy = EnginePolicy {
    zero_phrases: &[
        "no results found",
        "did not match any documents",
        "no results",
        "0 results",
    ],
    rate_limit_phrases: &[
        "too many requests",
        "rate limit",
        "access blocked",
        "temporarily blocked",
    ],
    status_regions: &["#results", ".message", "#results p"],
    hosts: &["mojeek.com", "www.mojeek.com"],
    cards: &[
        "ul.results-standard > li",
        "div.results-standard > li",
        ".results-standard li",
        "li.result",
    ],
    fallback_cards: &["a.title", "a.ob", "h2 a[href]"],
};

fn policy_for(engine: &str) -> &'static EnginePolicy {
    match engine {
        GOOGLE => &GOOGLE_POLICY,
        YANDEX => &YANDEX_POLICY,
        MOJEEK => &MOJEEK_POLICY,
        _ => &GOOGLE_POLICY,
    }
}

/// True when `host` is the engine host or a subdomain of it.
fn host_is(host: &str, base: &str) -> bool {
    if host == base {
        return true;
    }
    let suffix = format!(".{base}");
    host.len() > suffix.len() && host.ends_with(&suffix)
}

fn host_in_any(host: &str, bases: &[&str]) -> bool {
    bases.iter().any(|base| host_is(host, base))
}

/// Lowercased host of a parsed URL.
fn url_host(u: &Url) -> String {
    u.host_str().unwrap_or_default().to_lowercase()
}

/// True when the URL's path looks like a SERP results page for `engine`.
fn is_serp_path(engine: &str, path: &str) -> bool {
    match engine {
        GOOGLE | YANDEX | MOJEEK => path.starts_with("/search"),
        _ => false,
    }
}

/// Structural interstitial detection. Returns `Some(true)` for a challenge wall,
/// `Some(false)` for a consent wall, `None` when no interstitial shape was found.
///
/// Only structural evidence is used: forms whose action points at a wall, iframes
/// or links whose host is a consent/captcha host, a recaptcha widget element, and
/// `noscript` meta refreshes to a consent host.
fn detect_interstitial(doc: &Html, final_url: &str) -> Option<bool> {
    let final_host = Url::parse(final_url)
        .map(|u| url_host(&u))
        .unwrap_or_default();

    // A redirect onto a consent host is a consent wall, whatever the body says.
    if !final_host.is_empty() && host_is_consent(&final_host) {
        return Some(false);
    }

    if let Ok(forms) = Selector::parse("form") {
        for form in doc.select(&forms) {
            let action = form
                .value()
                .attr("action")
                .unwrap_or_default()
                .to_lowercase();
            if action.is_empty() {
                continue;
            }
            if action.contains("captcha") || action.contains("/sorry") {
                return Some(true);
            }
            if action.contains("consent") {
                return Some(false);
            }
        }
    }

    // Iframes or anchors whose host is a consent or captcha host.
    for sel in ["iframe[src]", "a[href]"] {
        if let Ok(selector) = Selector::parse(sel) {
            for el in doc.select(&selector) {
                let raw = if sel.starts_with("iframe") {
                    el.value().attr("src")
                } else {
                    el.value().attr("href")
                }
                .unwrap_or_default();
                if raw.is_empty() {
                    continue;
                }
                let lower = raw.to_lowercase();
                let host = if raw.starts_with("//") {
                    Url::parse(&format!("https:{raw}"))
                        .map(|u| url_host(&u))
                        .unwrap_or_default()
                } else if raw.contains("://") {
                    Url::parse(raw).map(|u| url_host(&u)).unwrap_or_default()
                } else {
                    continue;
                };
                if host.is_empty() {
                    continue;
                }
                if lower.contains("captcha") || host_is_captcha(&host) {
                    return Some(true);
                }
                if host_is_consent(&host) {
                    return Some(false);
                }
            }
        }
    }

    // A recaptcha widget element.
    for sel in [
        ".g-recaptcha",
        "#recaptcha",
        "[class*=\"recaptcha\"]",
        "[class*=\"smartcaptcha\"]",
        "[class*=\"SmartCaptcha\"]",
    ] {
        if let Ok(selector) = Selector::parse(sel) {
            if doc.select(&selector).next().is_some() {
                return Some(true);
            }
        }
    }

    // A meta refresh to a consent host, wherever the tag sits.
    if let Ok(meta) = Selector::parse("meta[http-equiv=\"refresh\"]") {
        for el in doc.select(&meta) {
            let content = el.value().attr("content").unwrap_or_default();
            if meta_refresh_is_consent(content) {
                return Some(false);
            }
        }
    }

    // The European-Google wall wraps that tag in `<noscript>`
    // (`<noscript><meta http-equiv="refresh" content="0;url=https://consent.google.com/save?..."></noscript>`).
    // html5ever parses `<noscript>` as raw text, so the tag inside it never
    // reaches the element tree; reading the text node directly is the only way
    // to see it. It is still structural evidence — a redirect to a consent host
    // — so it stays a consent wall rather than a parser problem.
    if let Ok(noscript) = Selector::parse("noscript") {
        for el in doc.select(&noscript) {
            let raw: String = el.text().collect();
            if meta_refresh_is_consent(&raw) {
                return Some(false);
            }
        }
    }
    None
}

/// True when a meta-refresh `content` value redirects to a consent host.
fn meta_refresh_is_consent(content: &str) -> bool {
    let lower = content.to_ascii_lowercase();
    if !lower.contains("consent") {
        return false;
    }
    let Some(url_part) = lower.split("url=").nth(1) else {
        return false;
    };
    let target = url_part.split_whitespace().next().unwrap_or_default();
    if !target.contains("://") {
        return false;
    }
    match Url::parse(target) {
        Ok(url) => host_is_consent(&url_host(&url)),
        Err(_) => false,
    }
}

/// True for hosts that only host consent walls.
fn host_is_consent(host: &str) -> bool {
    host == "consent.google.com" || host == "consent.youtube.com" || host == "consent.yahoo.com"
}

/// True for hosts that only host challenge walls.
fn host_is_captcha(host: &str) -> bool {
    host == "captcha.google.com" || host == "smartcaptcha.yandex.com"
}

/// True when `phrase` appears in `haystack` as a standalone token sequence.
///
/// The word boundaries are what make count boundaries safe: `0 results` matches a
/// real zero but not `10 results`, `100 results` or `1,000 results`, because the
/// preceding character is a digit.
fn contains_phrase(haystack: &str, phrase: &str) -> bool {
    let hay = haystack.to_lowercase();
    let needle = phrase.to_lowercase();
    if needle.is_empty() {
        return false;
    }
    let mut from = 0usize;
    while let Some(pos) = hay[from..].find(&needle) {
        let start = from + pos;
        let end = start + needle.len();
        let before_ok = start == 0
            || !hay[..start]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric());
        let after_ok = end == hay.len()
            || !hay[end..]
                .chars()
                .next()
                .is_some_and(|c| c.is_alphanumeric());
        if before_ok && after_ok {
            return true;
        }
        from = end;
    }
    false
}

/// Recognizes an explicit no-results/rate-limit phrase inside a status region only.
/// Returns `Some(true)` for rate limited, `Some(false)` for verified zero.
fn classify_status_region(region_text: &str, policy: &EnginePolicy) -> Option<bool> {
    if policy
        .rate_limit_phrases
        .iter()
        .any(|p| contains_phrase(region_text, p))
    {
        return Some(true);
    }
    if policy
        .zero_phrases
        .iter()
        .any(|p| contains_phrase(region_text, p))
    {
        return Some(false);
    }
    None
}

/// Text of every recognized engine status region, plus the region names matched.
fn status_region_text(doc: &Html, policy: &EnginePolicy) -> (Vec<String>, String) {
    let mut names: Vec<String> = Vec::new();
    let mut out = String::new();
    for sel in policy.status_regions {
        if let Ok(selector) = Selector::parse(sel) {
            for el in doc.select(&selector) {
                names.push((*sel).to_string());
                out.push(' ');
                out.push_str(&text_of(el));
            }
        }
    }
    (names, out)
}

/// True when `host` is a literal private/internal address or a non-public hostname.
fn is_private_host(host: &str) -> bool {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return match ip {
            IpAddr::V4(v4) => {
                v4.is_loopback()
                    || v4.is_private()
                    || v4.is_link_local()
                    || v4.is_unspecified()
                    || v4.is_broadcast()
            }
            IpAddr::V6(v6) => {
                v6.is_loopback()
                    || v6.is_unspecified()
                    || (v6.segments()[0] & 0xfe00) == 0xfc00
                    || (v6.segments()[0] & 0xffc0) == 0xfe80
            }
        };
    }
    matches!(host, "localhost" | "local")
        || host.ends_with(".local")
        || host.ends_with(".localhost")
        || host.ends_with(".internal")
        || host.ends_with(".home.arpa")
        || host.is_empty()
        || !host.contains('.')
}

/// Recognized ad hosts, rejected as `ad_host`.
fn is_ad_host(host: &str) -> bool {
    host == "doubleclick.net"
        || host.ends_with(".doubleclick.net")
        || host == "googleadservices.com"
        || host.ends_with(".googleadservices.com")
        || host == "googlesyndication.com"
        || host.ends_with(".googlesyndication.com")
        || host == "adservice.google.com"
        || host.ends_with(".an.yandex.ru")
        || host.ends_with(".ads.mojeek.com")
}

/// Unwraps recognized engine redirect wrappers, bounded to depth 3.
///
/// `query_pairs` performs the percent decoding, so `q`/`url` values are decoded
/// exactly once and destination query parameters survive.
fn unwrap_redirect(url: &Url, engine: &str, base: &Url) -> Url {
    let mut current = url.clone();
    for _ in 0..MAX_REDIRECT_DEPTH {
        let host = url_host(&current);
        let looks_like_wrapper = match engine {
            GOOGLE => {
                host.contains("google")
                    && (current.path().starts_with("/url")
                        || current.path().starts_with("/imgres")
                        || current.path().starts_with("/aclk"))
            }
            YANDEX => {
                host.contains("yandex")
                    && (current.path().starts_with("/clck") || current.path().starts_with("/redir"))
            }
            MOJEEK => host.contains("mojeek") && current.path().starts_with("/redir"),
            _ => false,
        };
        if !looks_like_wrapper {
            return current;
        }
        let target = current
            .query_pairs()
            .find(|(k, _)| k == "q" || k == "url")
            .map(|(_, v)| v.into_owned());
        let Some(target) = target else { return current };
        let next = Url::parse(target.trim())
            .or_else(|_| Url::parse(&format!("https://{}", target.trim())))
            .or_else(|_| base.join(target.trim()));
        match next {
            Ok(u) if u.scheme() == "http" || u.scheme() == "https" => current = u,
            _ => return current,
        }
    }
    current
}

/// Normalizes a destination URL against a base, recording every rejection reason.
///
/// Rejects engine-internal links, ad hosts, private/internal destinations, unsafe
/// schemes, credentials, malformed URLs and duplicates. Preserves legitimate
/// destination query parameters.
pub fn normalize_destination(
    raw: &str,
    engine: &str,
    base: &Url,
    diag: &mut SerpDiagnostics,
) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        diag.record_rejection("malformed");
        return None;
    }
    // Relative and protocol-relative URLs resolve against the parsed SERP URL.
    // Only a real scheme prefix marks an absolute URL: a relative path can carry
    // `://` inside its query (`/url?q=https://target`), and parsing that as
    // absolute would reject a valid wrapper link.
    let is_absolute = trimmed.split_once(':').is_some_and(|(scheme, rest)| {
        !scheme.is_empty()
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
            && rest.starts_with("//")
    });
    let parsed_abs = if is_absolute {
        Url::parse(trimmed).ok()
    } else {
        base.join(trimmed).ok()
    };
    let Some(parsed_abs) = parsed_abs else {
        diag.record_rejection("malformed");
        return None;
    };
    if parsed_abs.scheme() != "http" && parsed_abs.scheme() != "https" {
        diag.record_rejection("unsafe_scheme");
        return None;
    }
    if !parsed_abs.username().is_empty() || parsed_abs.password().is_some() {
        diag.record_rejection("credentials");
        return None;
    }
    let unwrapped = unwrap_redirect(&parsed_abs, engine, base);
    let host = url_host(&unwrapped);
    if is_private_host(&host) {
        diag.record_rejection("private_ip");
        return None;
    }
    if is_ad_host(&host) {
        diag.record_rejection("ad_host");
        return None;
    }
    let policy = policy_for(engine);
    if host_in_any(&host, policy.hosts) {
        diag.record_rejection("engine_internal");
        return None;
    }
    // A host the engine does not own but which is clearly a consent/interstitial host.
    if host_is(&host, "consent.google.com") || host_is(&host, "consent.youtube.com") {
        diag.record_rejection("engine_internal");
        return None;
    }
    let mut clean = unwrapped;
    clean.set_fragment(None);
    Some(clean.to_string())
}

/// Backwards-compatible wrapper over [`normalize_destination`] with the engine's
/// canonical SERP as base.
pub fn normalize_destination_url(raw: &str, engine: &str) -> Option<String> {
    let tool = match engine {
        GOOGLE => FIRECRAWL_GOOGLE_SEARCH,
        YANDEX => FIRECRAWL_YANDEX_SEARCH,
        MOJEEK => FIRECRAWL_MOJEEK_SEARCH,
        _ => return None,
    };
    let Ok(serp) = build_serp_url(tool, "argos") else {
        return None;
    };
    let Ok(base) = Url::parse(&serp) else {
        return None;
    };
    let mut diag = SerpDiagnostics::default();
    normalize_destination(raw, engine, &base, &mut diag)
}

/// Snippet text for a card element: first non-empty supported snippet container.
fn card_snippet(card: &scraper::ElementRef) -> String {
    for sel in [
        "div.VwiC3b",
        "span.aCOpRe",
        "div.OrganicTextContent",
        "p.s",
        "div.s",
        "p",
    ] {
        if let Ok(selector) = Selector::parse(sel) {
            for el in card.select(&selector) {
                let text = collapse_ws(&el.text().collect::<Vec<_>>().join(" "));
                if !text.is_empty() {
                    return clip_item_text(&text);
                }
            }
        }
    }
    String::new()
}

/// Extracted candidate link: href, heading text and snippet.
struct Candidate {
    href: String,
    title: String,
    snippet: String,
}

/// Collects organic candidates from the document, in document order.
///
/// Only card containers are scanned. A conservative structural fallback
/// (heading-link only) runs when the card selectors matched nothing, so DOM
/// drift still produces rankable results without turning every external link
/// on the page into a result.
fn collect_candidates(doc: &Html, policy: &EnginePolicy) -> Vec<Candidate> {
    let mut out: Vec<Candidate> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let Ok(link_sel) = Selector::parse("a[href]") else {
        return out;
    };
    for card_sel in policy.cards {
        let Ok(selector) = Selector::parse(card_sel) else {
            continue;
        };
        for card in doc.select(&selector) {
            // Prefer the first link that carries a heading; otherwise keep the
            // first link in the card so a missing heading is not an automatic loss.
            let mut fallback: Option<String> = None;
            let mut chosen: Option<String> = None;
            for link in card.select(&link_sel) {
                let Some(href) = link.value().attr("href") else {
                    continue;
                };
                if fallback.is_none() {
                    fallback = Some(href.to_string());
                }
                if !collapse_ws(&link.text().collect::<Vec<_>>().join(" ")).is_empty() {
                    chosen = Some(href.to_string());
                    break;
                }
            }
            let Some(href) = chosen.or(fallback) else {
                continue;
            };
            if !seen.insert(href.clone()) {
                continue;
            }
            let title = link_heading(doc, &href, &link_sel);
            let snippet = if title.is_empty() {
                String::new()
            } else {
                card_snippet(&card)
            };
            out.push(Candidate {
                href,
                title: clip_item_text(&title),
                snippet,
            });
        }
    }
    if !out.is_empty() {
        return out;
    }
    for fallback_sel in policy.fallback_cards {
        let Ok(selector) = Selector::parse(fallback_sel) else {
            continue;
        };
        for link in doc.select(&selector) {
            let Some(href) = link.value().attr("href") else {
                continue;
            };
            if !seen.insert(href.to_string()) {
                continue;
            }
            let title = collapse_ws(&link.text().collect::<Vec<_>>().join(" "));
            out.push(Candidate {
                href: href.to_string(),
                title: clip_item_text(&title),
                snippet: String::new(),
            });
        }
    }
    out
}

/// Heading text of the link that owns `href`, looked up in `link_sel` order.
fn link_heading(doc: &Html, href: &str, link_sel: &Selector) -> String {
    for link in doc.select(link_sel) {
        if link.value().attr("href") == Some(href) {
            return link.text().collect::<Vec<_>>().join(" ");
        }
    }
    String::new()
}

/// Counts DOM candidates before filtering, for diagnostics.
fn dom_candidate_count(doc: &Html, policy: &EnginePolicy) -> usize {
    let mut count = 0usize;
    for card_sel in policy.cards {
        if let Ok(selector) = Selector::parse(card_sel) {
            count += doc.select(&selector).count();
        }
    }
    if count == 0 {
        for fallback_sel in policy.fallback_cards {
            if let Ok(selector) = Selector::parse(fallback_sel) {
                count += doc.select(&selector).count();
            }
        }
    }
    count
}

/// Builds the ranked item list from a parsed document.
fn extract_items(
    doc: &Html,
    engine: &str,
    policy: &EnginePolicy,
    req: &SerpRequest,
    base: &Url,
    diag: &mut SerpDiagnostics,
) -> Vec<EngineSearchResult> {
    let mut items: Vec<EngineSearchResult> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let now = Utc::now().to_rfc3339();
    for candidate in collect_candidates(doc, policy) {
        if items.len() >= req.limit {
            break;
        }
        let Some(dest) = normalize_destination(&candidate.href, engine, base, diag) else {
            continue;
        };
        if seen.contains(&dest) {
            diag.record_rejection("duplicate");
            continue;
        }
        seen.insert(dest.clone());
        items.push(EngineSearchResult {
            query: req.query.clone(),
            engine: req.engine.clone(),
            serp_url: req.serp_url.clone(),
            fetched_time: now.clone(),
            rank: items.len() + 1,
            destination: dest.clone(),
            url: dest,
            title: candidate.title.clone(),
            snippet: candidate.snippet.clone(),
            parser_version: PARSER_VERSION.to_string(),
            outcome: SerpOutcome::Valid.as_str().to_string(),
        });
    }
    diag.accepted = items.len();
    items
}

/// Entry point. Never returns an error for a parsable-but-unrecognized page;
/// the caller reads `outcome` instead.
pub fn parse_serp_response(req: &SerpRequest, raw_json: &Value) -> Result<SerpParse> {
    let engine = &req.engine;
    let policy = policy_for(engine);
    let mut diag = SerpDiagnostics {
        parser_input: ParserInput::Missing,
        requested_url: clip_diag(&req.serp_url),
        ..Default::default()
    };

    // Envelope failure: never a zero, never a parser problem.
    if raw_json.get("success").and_then(Value::as_bool) == Some(false) {
        diag.provider_error = clip_diag(
            str_at(raw_json, "/error")
                .or_else(|| str_at(raw_json, "/data/error"))
                .unwrap_or("provider reported success=false"),
        );
        return Ok(SerpParse {
            items: Vec::new(),
            outcome: SerpOutcome::UpstreamFailure,
            diagnostics: diag,
        });
    }
    // HTTP 404 on the provider request is a failure for these tools.
    if matches!(raw_json.get("code").and_then(Value::as_u64), Some(404)) {
        diag.provider_error = "provider returned HTTP 404".to_string();
        return Ok(SerpParse {
            items: Vec::new(),
            outcome: SerpOutcome::UpstreamFailure,
            diagnostics: diag,
        });
    }

    let data = raw_json.get("data").unwrap_or(raw_json);
    if let Some(err) = str_at(data, "/error") {
        diag.provider_error = clip_diag(err);
        return Ok(SerpParse {
            items: Vec::new(),
            outcome: SerpOutcome::UpstreamFailure,
            diagnostics: diag,
        });
    }
    if let Some(warning) = str_at(data, "/warning").or_else(|| str_at(data, "/metadata/warning")) {
        diag.provider_warning = clip_diag(warning);
    }
    if let Some(err) = str_at(data, "/metadata/error") {
        diag.provider_error = clip_diag(err);
        return Ok(SerpParse {
            items: Vec::new(),
            outcome: SerpOutcome::UpstreamFailure,
            diagnostics: diag,
        });
    }

    diag.target_status = num_at(data, "/metadata/statusCode");
    diag.final_url = clip_diag(
        str_at(data, "/metadata/sourceURL")
            .or_else(|| str_at(data, "/metadata/url"))
            .or_else(|| str_at(data, "/metadata/finalUrl"))
            .unwrap_or(&req.serp_url),
    );

    // Input provenance: prefer non-empty rawHtml, fall back to cleaned html.
    let raw_html = str_at(data, "/rawHtml").unwrap_or("");
    let clean_html = str_at(data, "/html").unwrap_or("");
    let (html_str, parser_input) = if !raw_html.is_empty() {
        (raw_html, ParserInput::RawHtml)
    } else if !clean_html.is_empty() {
        (clean_html, ParserInput::CleanHtml)
    } else {
        ("", ParserInput::Missing)
    };
    diag.parser_input = parser_input;
    diag.input_bytes = html_str.len();

    // Target-side HTTP failure behind an outer 200, evaluated before the payload
    // shape: a 403/404/500/429 is an envelope fact, not a parsing problem.
    if let Some(status) = diag.target_status {
        if status == 404 {
            diag.provider_error = format!("target returned HTTP {status}");
            return Ok(SerpParse {
                items: Vec::new(),
                outcome: SerpOutcome::UpstreamFailure,
                diagnostics: diag,
            });
        }
        if status == 429 {
            return Ok(SerpParse {
                items: Vec::new(),
                outcome: SerpOutcome::RateLimited,
                diagnostics: diag,
            });
        }
        if !(200..400).contains(&status) {
            diag.provider_error = format!("target returned HTTP {status}");
            return Ok(SerpParse {
                items: Vec::new(),
                outcome: SerpOutcome::UpstreamFailure,
                diagnostics: diag,
            });
        }
    }

    // Truncated / oversize envelope: never parsed as a valid empty page.
    if data.get("truncated").and_then(Value::as_bool) == Some(true)
        || data.pointer("/metadata/truncated").and_then(Value::as_bool) == Some(true)
        || diag.input_bytes > SERP_MAX_BODY_BYTES
    {
        return Ok(SerpParse {
            items: Vec::new(),
            outcome: SerpOutcome::ResponseTooLarge,
            diagnostics: diag,
        });
    }

    // No DOM at all: links-only or nothing. Neither can rank, so neither is a zero.
    if parser_input == ParserInput::Missing {
        let links_bytes = data
            .get("links")
            .and_then(Value::as_array)
            .map_or(0, |links| {
                links.iter().map(|l| l.as_str().map_or(0, str::len)).sum()
            });
        if links_bytes > 0 {
            diag.parser_input = ParserInput::LinksOnly;
            diag.input_bytes = links_bytes;
        }
        return Ok(SerpParse {
            items: Vec::new(),
            outcome: SerpOutcome::ParserMismatch,
            diagnostics: diag,
        });
    }

    let document = Html::parse_document(html_str);
    diag.dom_candidates = dom_candidate_count(&document, policy);

    // Structural interstitial: real challenge/consent wall.
    if let Some(is_challenge) = detect_interstitial(&document, &diag.final_url) {
        diag.status_region = "interstitial".to_string();
        return Ok(SerpParse {
            items: Vec::new(),
            outcome: if is_challenge {
                SerpOutcome::Challenge
            } else {
                SerpOutcome::Consent
            },
            diagnostics: diag,
        });
    }

    // Validate the final URL against the expected engine SERP host/path.
    let final_url_ok = Url::parse(&diag.final_url)
        .map(|u| host_in_any(&url_host(&u), policy.hosts) && is_serp_path(engine, u.path()))
        .unwrap_or(true);
    let requested_url_ok = Url::parse(&req.serp_url)
        .map(|u| host_in_any(&url_host(&u), policy.hosts) && is_serp_path(engine, u.path()))
        .unwrap_or(true);
    if !requested_url_ok {
        diag.status_region = "requested_url_off_host".to_string();
        return Ok(SerpParse {
            items: Vec::new(),
            outcome: SerpOutcome::UpstreamFailure,
            diagnostics: diag,
        });
    }

    let base = Url::parse(&diag.final_url)
        .or_else(|_| Url::parse(&req.serp_url))
        .unwrap_or_else(|_| Url::parse("https://example.com/").expect("static url"));

    let items = extract_items(&document, engine, policy, req, &base, &mut diag);

    // Only a recognized status region with a supported phrase can be a zero.
    if items.is_empty() {
        let (regions, region) = status_region_text(&document, policy);
        if !region.trim().is_empty() {
            if let Some(is_rate_limited) = classify_status_region(&region, policy) {
                diag.status_region = clip_diag(&regions.join("|"));
                return Ok(SerpParse {
                    items,
                    outcome: if is_rate_limited {
                        SerpOutcome::RateLimited
                    } else {
                        SerpOutcome::VerifiedZero
                    },
                    diagnostics: diag,
                });
            }
            diag.status_region = "status_region_unmatched".to_string();
        }
    }

    if items.is_empty() {
        // Unknown but non-empty DOM is never a zero: it is a parser mismatch.
        diag.status_region = "no_matching_status_region".to_string();
        let outcome = if !final_url_ok {
            SerpOutcome::UpstreamFailure
        } else {
            SerpOutcome::ParserMismatch
        };
        return Ok(SerpParse {
            items,
            outcome,
            diagnostics: diag,
        });
    }

    // Host mismatch on a page that did yield organic cards: keep the items we
    // extracted, but the outcome is not a clean Valid.
    if !final_url_ok {
        diag.status_region = "final_url_off_host".to_string();
        return Ok(SerpParse {
            items,
            outcome: SerpOutcome::UpstreamFailure,
            diagnostics: diag,
        });
    }

    Ok(SerpParse {
        items,
        outcome: SerpOutcome::Valid,
        diagnostics: diag,
    })
}

#[cfg(test)]
mod tests;
