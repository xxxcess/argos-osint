//! Atlas collects regional headlines, tags them, and stores the run's statistics.
//!
//! Phase 1 reads the latest headlines. Phase 2 headlines are saved on the run and
//! tagged by the classifier model. The synthesis model then writes a handful of
//! span-checked claims into Brain. The database keeps the run, its cursor, the country table,
//! the articles, and the daily request counts.

use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url;

use crate::atlas_insights::{self, InsightStats};
use crate::osint::atlas_news::{self, Hit};
use crate::osint::ProviderKeys;
use crate::provider::{self, DecisionsResponse};
use crate::secrets::ProviderSecret;
use crate::store::{AtlasArticleRow, AtlasRunRow, Store};

/// Country-coded headlines collected from each phase-1 source.
const PHASE1_TARGET: u32 = 50;
/// Pages per source. Ten articles a page, so five pages cover the target.
const PHASE1_MAX_PAGES: usize = 5;

const CLUSTERS: &[(&str, &[&str])] = &[
    (
        "geopolitical",
        &[
            "joint statement",
            "strategic partnership",
            "memorandum of understanding",
            "bilateral agreement",
            "multilateral pact",
            "diplomatic alignment",
            "voting bloc",
            "treaty ratification",
            "security guarantee",
            "envoy recalled",
            "diplomat expelled",
            "consulate closed",
            "sanctions package",
            "diplomatic retaliation",
            "sovereignty dispute",
            "demarche",
            "official protest",
            "border dispute",
        ],
    ),
    (
        "economic",
        &[
            "critical minerals",
            "semiconductor supply",
            "rare earth elements",
            "lithium production",
            "cobalt sourcing",
            "export restriction",
            "export embargo",
            "export quota",
            "chokepoint vulnerability",
            "de-dollarization",
            "local currency settlement",
            "bilateral swap line",
            "currency decoupling",
            "financial sanctions",
            "SWIFT alternative",
            "asset freeze",
            "central bank reserves",
            "energy security",
            "LNG terminal",
            "pipeline disruption",
            "trade asymmetry",
            "strategic tariff",
            "protectionism",
            "maritime chokepoint",
            "freight route disruption",
        ],
    ),
    (
        "military",
        &[
            "joint military exercise",
            "live-fire drill",
            "troop deployment",
            "combat readiness",
            "troop mobilization",
            "border buildup",
            "naval transit",
            "airspace intrusion",
            "freedom of navigation",
            "defense budget",
            "arms sale",
            "military procurement",
            "hypersonic missile",
            "deep-water port",
            "naval base expansion",
            "air defense system",
            "dual-use infrastructure",
            "satellite reconnaissance",
        ],
    ),
    (
        "information",
        &[
            "foreign interference",
            "disinformation campaign",
            "coordinated inauthentic behavior",
            "state-media narrative",
            "psychological operations",
            "cognitive warfare",
            "information manipulation",
            "bot network",
            "internet censorship",
            "website blocked",
            "content takedown",
            "VPN restriction",
            "information blackout",
            "state-controlled media",
            "media crackdown",
        ],
    ),
    (
        "stability",
        &[
            "civil unrest",
            "mass protest",
            "general strike",
            "curfew declared",
            "anti-government demonstration",
            "food rationing",
            "hyperinflation",
            "energy blackouts",
            "labor shortage",
            "succession plan",
            "political factionalism",
            "cabinet reshuffle",
            "anti-corruption purge",
            "regime fragility",
            "martial law",
            "snap election",
            "constitutional amendment",
        ],
    ),
    (
        "technology",
        &[
            "6G standards",
            "AI governance",
            "biotech protocol",
            "quantum computing regulation",
            "technology decoupling",
            "intellectual property theft",
            "tech controls",
            "cyber reconnaissance",
            "industrial control system",
            "power grid vulnerability",
            "undersea cable",
            "ransomware attack",
            "critical infrastructure breach",
            "firmware exploit",
        ],
    ),
];

pub const CATEGORY_IDS: &[&str] = &[
    "geopolitical",
    "economic",
    "military",
    "information",
    "stability",
    "technology",
    "unk",
];

/// Short tag stored on the article and shown in the news list.
pub fn category_tag(id: &str) -> &'static str {
    match id {
        "geopolitical" => "geopolitical",
        "economic" => "economic",
        "military" => "military",
        "information" => "information",
        "stability" => "stability",
        "technology" => "technology",
        _ => "unk",
    }
}

/// Full OSINT category name for the article card.
pub fn category_name(id: &str) -> &'static str {
    match id {
        "geopolitical" => "Geopolitical & Diplomatic",
        "economic" => "Economic & Resource",
        "military" => "Military Posture",
        "information" => "Information Operations",
        "stability" => "Domestic Stability",
        "technology" => "Technological Vectors",
        _ => "unk",
    }
}

/// One Decisions choice. The criteria are the category phrases. `unk` is the miss.
pub fn classification_request(article: &FeedArticle) -> (serde_json::Value, serde_json::Value) {
    let mut criteria = serde_json::Map::new();
    for (id, phrases) in CLUSTERS {
        criteria.insert(
            (*id).to_string(),
            serde_json::json!(format!(
                "{} Signals include: {}.",
                category_name(id),
                phrases.join(", ")
            )),
        );
    }
    criteria.insert(
        "unk".into(),
        serde_json::json!("The article does not fit any of the other categories."),
    );
    let questions = serde_json::json!({
        "category": {
            "type": "choice",
            "instructions": "Choose the single OSINT category this article belongs to from its title and description. Choose unk when none of the categories fit.",
            "criteria": criteria
        }
    });
    let state = serde_json::json!({
        "title": article.title,
        "description": article.description,
        "source": article.source_name,
        "country": article.country,
    });
    (state, questions)
}

pub fn category_from_decisions(response: &DecisionsResponse) -> String {
    response
        .answers
        .get("category")
        .and_then(|answer| answer.choice.as_deref())
        .map(normalize_category)
        .unwrap_or_else(|| "unk".into())
}

pub fn normalize_category(raw: &str) -> String {
    let id = raw.trim().to_ascii_lowercase();
    let id = id.trim_matches(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_');
    if CATEGORY_IDS.contains(&id) {
        id.to_string()
    } else {
        "unk".into()
    }
}

const DOMAIN_RANK: &[(&str, i32)] = &[
    ("reuters.com", 100),
    ("apnews.com", 96),
    ("afp.com", 94),
    ("bbc.co.uk", 90),
    ("bbc.com", 90),
    ("theguardian.com", 84),
    ("nytimes.com", 82),
    ("washingtonpost.com", 80),
    ("wsj.com", 80),
    ("ft.com", 78),
    ("bloomberg.com", 78),
    ("aljazeera.com", 74),
    ("cnn.com", 70),
    ("economist.com", 70),
    ("nikkei.com", 66),
    ("scmp.com", 64),
];

pub fn daily_cap(provider: &str) -> u32 {
    let provider = provider.split(':').next().unwrap_or(provider);
    match provider {
        "gnews" => 100,
        "newsdata" => 200,
        "newsapi" => 100,
        "currents" => 250,
        _ => 0,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Band {
    pub country: String,
    pub count: u32,
    pub tier: u8,
    pub temperature: f64,
}

/// 20% / 30% / 30% / 20%, rounded so the four sizes sum to `n`. One country is Group 1.
pub fn band_sizes(n: usize) -> (usize, usize, usize, usize) {
    if n == 0 {
        return (0, 0, 0, 0);
    }
    if n == 1 {
        return (1, 0, 0, 0);
    }
    let mut sizes = [
        (n as f64 * 0.20).round() as usize,
        (n as f64 * 0.30).round() as usize,
        (n as f64 * 0.30).round() as usize,
        (n as f64 * 0.20).round() as usize,
    ];
    let sum: usize = sizes.iter().sum();
    if sum > n {
        let mut extra = sum - n;
        for size in sizes.iter_mut().rev() {
            let take = extra.min(*size);
            *size -= take;
            extra -= take;
        }
    } else if sum < n {
        sizes[0] += n - sum;
    }
    if sizes[0] == 0 {
        for index in 1..4 {
            if sizes[index] > 0 {
                sizes[index] -= 1;
                sizes[0] = 1;
                break;
            }
        }
    }
    (sizes[0], sizes[1], sizes[2], sizes[3])
}

fn band_temperature(tier: u8, index: usize, len: usize) -> f64 {
    let (hot, cool) = match tier {
        1 => return 1.0,
        2 => (0.99, 0.70),
        _ => (0.69, 0.10),
    };
    if len <= 1 {
        return hot;
    }
    let step = index as f64 / (len - 1) as f64;
    hot + (cool - hot) * step
}

/// Countries sorted by volume, then code. Group 4 is omitted.
pub fn score_counts(counts: &BTreeMap<String, u32>) -> Vec<Band> {
    let mut ranked: Vec<(String, u32)> = counts
        .iter()
        .filter(|(_, count)| **count > 0)
        .map(|(country, count)| (country.clone(), *count))
        .collect();
    ranked.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    let (g1, g2, g3, _) = band_sizes(ranked.len());
    let mut bands = Vec::new();
    for (offset, (country, count)) in ranked.into_iter().enumerate() {
        let (tier, index, len) = if offset < g1 {
            (1, offset, g1)
        } else if offset < g1 + g2 {
            (2, offset - g1, g2)
        } else if offset < g1 + g2 + g3 {
            (3, offset - g1 - g2, g3)
        } else {
            continue;
        };
        bands.push(Band {
            country,
            count,
            tier,
            temperature: band_temperature(tier, index, len),
        });
    }
    bands
}

pub fn normalize_title(title: &str) -> String {
    title
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(|ch| ch.to_lowercase())
        .collect()
}

pub fn canonical_url(raw: &str) -> String {
    let Ok(mut url) = Url::parse(raw.trim()) else {
        return raw.trim().to_ascii_lowercase();
    };
    if let Some(host) = url.host_str() {
        let host = host.to_ascii_lowercase();
        let _ = url.set_host(Some(&host));
    }
    url.set_fragment(None);
    let mut text = url.to_string();
    if text.ends_with('/') && url.path() == "/" {
        text.pop();
    }
    text
}

pub fn url_hash(raw: &str) -> String {
    let digest = Sha256::digest(canonical_url(raw).as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn host_of(raw: &str) -> String {
    Url::parse(raw)
        .ok()
        .and_then(|url| {
            url.host_str()
                .map(|host| host.trim_start_matches("www.").to_ascii_lowercase())
        })
        .unwrap_or_default()
}

pub fn domain_rank(host: &str) -> i32 {
    let host = host.trim_start_matches("www.");
    DOMAIN_RANK
        .iter()
        .find(|(domain, _)| host == *domain || host.ends_with(&format!(".{domain}")))
        .map(|(_, rank)| *rank)
        .unwrap_or(0)
}

#[derive(Clone, Debug)]
struct Seen {
    url_hash: String,
    title_norm: String,
    domain: String,
    rank: i32,
    published_at: String,
    id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Verdict {
    Keep,
    Drop,
    Replace(String),
}

fn judge(article: &FeedArticle, seen: &[Seen]) -> Verdict {
    let hash = url_hash(&article.url);
    let title = normalize_title(&article.title);
    if title.is_empty() || article.url.is_empty() {
        return Verdict::Drop;
    }
    if seen.iter().any(|item| item.url_hash == hash) {
        return Verdict::Drop;
    }
    if seen.iter().any(|item| item.title_norm == title) {
        return Verdict::Drop;
    }
    let rank = domain_rank(&article.source_domain);
    let mut replace: Option<&Seen> = None;
    for item in seen {
        if item.domain == article.source_domain || item.title_norm.is_empty() {
            continue;
        }
        let score = strsim::normalized_levenshtein(&title, &item.title_norm);
        if score <= 0.85 {
            continue;
        }
        let incoming_wins = rank > item.rank
            || (rank == item.rank
                && article.published_at < item.published_at
                && !article.published_at.is_empty());
        if incoming_wins {
            replace = Some(item);
        } else {
            return Verdict::Drop;
        }
    }
    match replace {
        Some(item) => Verdict::Replace(item.id.clone()),
        None => Verdict::Keep,
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct OriginStat {
    pub country: String,
    pub tier: u8,
    pub temperature: f64,
    pub volume: u32,
    pub articles: u32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct RunStats {
    pub counts: BTreeMap<String, u32>,
    pub origins: Vec<OriginStat>,
    pub scored: bool,
    /// Span-checked claims written for this cycle. Absent on runs saved before insights.
    #[serde(default)]
    pub insights: InsightStats,
}

impl RunStats {
    pub fn share(&self, articles: u32) -> f64 {
        let total: u32 = self.origins.iter().map(|row| row.articles).sum();
        if total == 0 {
            0.0
        } else {
            f64::from(articles) / f64::from(total)
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Cursor {
    pub phase: u8,
    pub chunk: usize,
    pub leg: String,
    pub country: usize,
    pub from: String,
    /// NewsData `nextPage` token for the request this cursor points at.
    #[serde(default)]
    pub page_token: String,
    /// Country-coded hits already kept for the current phase-1 source.
    #[serde(default)]
    pub kept: u32,
}

impl Default for Cursor {
    fn default() -> Self {
        Self {
            phase: 1,
            chunk: 0,
            leg: "gnews".into(),
            country: 0,
            from: String::new(),
            page_token: String::new(),
            kept: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FeedArticle {
    pub id: String,
    pub title: String,
    pub description: String,
    pub url: String,
    pub country: String,
    pub source_name: String,
    pub source_domain: String,
    pub author: String,
    pub image_url: String,
    pub published_at: String,
    pub provider: String,
    pub temperature: f64,
    pub seen_at: String,
    /// OSINT category id, or `unk` until the classifier tags it.
    pub category: String,
}

#[derive(Clone, Debug)]
pub struct HttpCall {
    pub provider: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
}

#[derive(Clone, Debug)]
pub struct HttpReply {
    pub status: u16,
    pub body: String,
}

/// A provider rejection. `summary` is the log line. `body` is the full response.
#[derive(Clone, Debug)]
pub struct ProviderFault {
    pub summary: String,
    pub body: String,
}

#[derive(Clone, Debug)]
pub enum AtlasEvent {
    Status(String),
    Stats(RunStats),
    Article(FeedArticle),
    Replaced { id: String, article: FeedArticle },
    Classified { id: String, category: String },
    Note(String),
    Fault(ProviderFault),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stop {
    Paused,
    Finished,
    Failed(String),
}

#[derive(Clone, Debug)]
struct Job {
    phase: u8,
    chunk: usize,
    leg: &'static str,
    country: usize,
    provider: &'static str,
}

fn leg_order(leg: &str) -> u8 {
    match leg {
        "gnews" | "newsapi" => 0,
        _ => 1,
    }
}

fn job_is_before(job: &Job, cursor: &Cursor) -> bool {
    if job.phase < cursor.phase {
        return true;
    }
    if job.phase > cursor.phase {
        return false;
    }
    if job.phase == 1 {
        (job.chunk, leg_order(job.leg)) < (cursor.chunk, leg_order(&cursor.leg))
    } else {
        (job.country, leg_order(job.leg)) < (cursor.country, leg_order(&cursor.leg))
    }
}

fn cursor_at(job: &Job, from: &str) -> Cursor {
    Cursor {
        phase: job.phase,
        chunk: job.chunk,
        leg: job.leg.into(),
        country: job.country,
        from: from.into(),
        page_token: String::new(),
        kept: 0,
    }
}

/// `United States (US)`. Unknown codes keep a readable fallback.
pub fn country_label(code: &str) -> String {
    let code = code.trim().to_ascii_lowercase();
    let name = country_name(&code).unwrap_or("Unknown");
    format!("{name} ({})", code.to_ascii_uppercase())
}

pub fn country_name(code: &str) -> Option<&'static str> {
    crate::iso3166::name(code)
}

/// Headline card. Category is the OSINT classification. Published is the article time.
pub fn format_article_card(
    title: &str,
    source_name: &str,
    source_domain: &str,
    author: &str,
    country: &str,
    category: &str,
    published_at: &str,
    temperature: f64,
    description: &str,
    image_url: &str,
    url: &str,
) -> String {
    let (publisher, author) = publisher_and_author(source_name, source_domain, author);
    let published = friendly_date(published_at);
    let image = if image_url.trim().is_empty() {
        "none"
    } else {
        image_url.trim()
    };
    let mut lines = vec![
        format!("Title: {title}"),
        format!("Publisher: {publisher}"),
        format!("Author: {author}"),
        format!("Country: {}", country_label(country)),
        format!("Category: {}", category_name(category)),
        format!("Published: {published}"),
        format!("Temperature: {temperature:.2}"),
    ];
    if !description.trim().is_empty() {
        lines.push(String::new());
        lines.push(description.trim().to_string());
    }
    lines.push(String::new());
    lines.push(format!("Image: {image}"));
    lines.push(format!("Article: {url}"));
    lines.join("\n")
}

/// Publisher is the outlet. A personal name, or a semicolon byline left in
/// `source_name` by older Currents rows, is the author. The article host fills
/// the publisher when the outlet name is missing. When only one of the two
/// values is present, both fields use it.
pub fn publisher_and_author(source_name: &str, domain: &str, author: &str) -> (String, String) {
    let source_name = source_name.trim();
    let domain = domain.trim();
    let mut author = author.trim().to_string();
    let mut publisher = source_name.to_string();
    if author.is_empty() && publisher.contains(';') {
        author = publisher.clone();
        publisher = domain.to_string();
    } else if personal_name(&publisher) && !domain.is_empty() {
        if author.is_empty() {
            author = publisher.clone();
        }
        publisher = domain.to_string();
    }
    if publisher.is_empty() {
        publisher = if !domain.is_empty() {
            domain.to_string()
        } else {
            author.clone()
        };
    }
    if author.is_empty() {
        author = publisher.clone();
    }
    if publisher.is_empty() {
        publisher = "unknown".into();
    }
    if author.is_empty() {
        author = publisher.clone();
    }
    (publisher, author)
}

/// Two or three capitalized words, with no outlet token, is a byline.
fn personal_name(value: &str) -> bool {
    let words: Vec<&str> = value.split_whitespace().collect();
    if !(2..=3).contains(&words.len()) {
        return false;
    }
    const STOP: &[&str] = &[
        "the",
        "of",
        "and",
        "for",
        "news",
        "times",
        "post",
        "press",
        "herald",
        "tribune",
        "gazette",
        "journal",
        "daily",
        "review",
        "wire",
        "media",
        "radio",
        "reuters",
        "agency",
        "associated",
        "guardian",
        "bloomberg",
    ];
    words.iter().all(|word| {
        let bare = word.trim_matches(|c: char| matches!(c, ',' | '.' | '\'' | '’'));
        if bare.is_empty() || STOP.contains(&bare.to_ascii_lowercase().as_str()) {
            return false;
        }
        let mut chars = bare.chars();
        let Some(first) = chars.next() else {
            return false;
        };
        first.is_uppercase() && chars.all(|c| c.is_alphabetic() || c == '-' || c == '\'')
    })
}

/// Same clock as `friendly_date`, from a unix timestamp.
pub fn friendly_unix(secs: u64) -> String {
    match chrono::DateTime::from_timestamp(secs as i64, 0) {
        Some(stamp) => friendly_date(&stamp.to_rfc3339()),
        None => "unknown".into(),
    }
}

/// Local calendar day, plus a 12-hour clock when the value has a time.
/// A bare date stays a date. Unparsed text is returned as stored.
pub fn friendly_date(raw: &str) -> String {
    let raw = raw.trim();
    if raw.is_empty() {
        return "unknown".into();
    }
    if !raw.contains(':') {
        if let Ok(date) = chrono::NaiveDate::parse_from_str(raw, "%Y-%m-%d") {
            return loosen_date(&date.format("%b %d, %Y").to_string());
        }
    }
    parse_timestamp(raw)
        .map(|stamp| {
            loosen_date(
                &stamp
                    .with_timezone(&chrono::Local)
                    .format("%b %d, %Y, %I:%M %p")
                    .to_string(),
            )
        })
        .unwrap_or_else(|| raw.to_string())
}

/// Drops the leading zero on the day and on a 12-hour clock.
fn loosen_date(value: &str) -> String {
    value.replace(" 0", " ")
}

fn parse_timestamp(raw: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    if let Ok(stamp) = chrono::DateTime::parse_from_rfc3339(raw) {
        return Some(stamp.with_timezone(&chrono::Utc));
    }
    if let Ok(stamp) = chrono::DateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S %z") {
        return Some(stamp.with_timezone(&chrono::Utc));
    }
    for format in ["%Y-%m-%d %H:%M:%S", "%Y-%m-%dT%H:%M:%S"] {
        if let Ok(stamp) = chrono::NaiveDateTime::parse_from_str(raw, format) {
            return Some(stamp.and_utc());
        }
    }
    chrono::NaiveDate::parse_from_str(raw, "%Y-%m-%d")
        .ok()
        .and_then(|date| date.and_hms_opt(0, 0, 0))
        .map(|stamp| stamp.and_utc())
}

pub fn format_run_card(run: &AtlasRunRow) -> String {
    let stats = parse_stats(&run.stats_json);
    let mut lines = vec![
        format!("State: {}", run.state),
        format!("Started: {}", friendly_date(&run.started_at)),
    ];
    if !run.finished_at.is_empty() {
        lines.push(format!("Finished: {}", friendly_date(&run.finished_at)));
    }
    if !run.note.is_empty() {
        lines.push(run.note.clone());
    }
    lines.push(String::new());
    if stats.origins.is_empty() {
        lines.push("No country statistics for this run.".into());
    } else {
        lines.push("Country                         Tier   Temp   Volume  Articles  Share".into());
        for row in &stats.origins {
            lines.push(format!(
                "{:<32} {:<6} {:<6.2} {:<7} {:<9} {:.0}%",
                country_label(&row.country),
                row.tier,
                row.temperature,
                row.volume,
                row.articles,
                stats.share(row.articles) * 100.0
            ));
        }
    }
    lines.push(String::new());
    lines.extend(atlas_insights::insight_table_lines(&stats.insights));
    lines.join("\n")
}

fn parse_stats(raw: &str) -> RunStats {
    serde_json::from_str(raw).unwrap_or_default()
}

fn parse_cursor(raw: &str) -> Cursor {
    serde_json::from_str(raw).unwrap_or_default()
}

fn lookback_from() -> String {
    (chrono::Utc::now() - chrono::Duration::hours(48))
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn utc_day() -> String {
    chrono::Utc::now().format("%Y-%m-%d").to_string()
}

fn new_run_id() -> String {
    format!(
        "atlas-{}-{}",
        chrono::Utc::now().timestamp_millis(),
        std::process::id()
    )
}

fn article_from(hit: &Hit, provider: &str, temperature: f64) -> FeedArticle {
    let domain = host_of(&hit.url);
    let (publisher, author) = publisher_and_author(&hit.source_name, &domain, &hit.author);
    FeedArticle {
        id: format!("art-{}", url_hash(&hit.url)),
        title: hit.title.clone(),
        description: hit.description.clone(),
        url: hit.url.clone(),
        country: hit.country.clone(),
        source_name: publisher,
        source_domain: domain,
        author,
        image_url: hit.image_url.clone(),
        published_at: hit.published_at.clone(),
        provider: provider.into(),
        temperature,
        seen_at: chrono::Utc::now().to_rfc3339(),
        category: "unk".into(),
    }
}

fn seen_of(article: &FeedArticle) -> Seen {
    Seen {
        url_hash: url_hash(&article.url),
        title_norm: normalize_title(&article.title),
        domain: article.source_domain.clone(),
        rank: domain_rank(&article.source_domain),
        published_at: article.published_at.clone(),
        id: article.id.clone(),
    }
}

fn remember(seen: &mut Vec<Seen>, article: &FeedArticle, replaced: Option<&str>) {
    if let Some(id) = replaced {
        seen.retain(|item| item.id != id);
    }
    seen.push(seen_of(article));
}

type FetchFut = Pin<Box<dyn Future<Output = Result<HttpReply>> + Send>>;

struct CallSpec {
    provider: &'static str,
    url: String,
    headers: Vec<(String, String)>,
}

fn phase1_call(provider: &str, page: usize, page_token: &str, key: &str) -> Result<CallSpec> {
    let prepared = atlas_news::prepare_latest(provider, page, page_token, key)?;
    Ok(CallSpec {
        provider: if provider == "gnews" {
            "gnews"
        } else {
            "newsdata"
        },
        url: prepared.url,
        headers: prepared.headers,
    })
}

fn phase2_call(provider: &str, country: &str, key: &str) -> Result<CallSpec> {
    let prepared = atlas_news::prepare_headlines(provider, country, key)?;
    Ok(CallSpec {
        provider: if provider == "newsapi" {
            "newsapi"
        } else {
            "currents"
        },
        url: prepared.url,
        headers: prepared.headers,
    })
}

/// Primary account while its local daily cap is open and the key is not exhausted.
/// Otherwise the fallback account, when that cap is still open.
fn open_key<'a>(
    store: &Store,
    provider: &str,
    primary: &'a str,
    fallback: &'a str,
) -> Result<Option<&'a str>> {
    let primary = primary.trim();
    let fallback = fallback.trim();
    if !primary.is_empty() && !crate::osint::key_exhausted(primary) && quota_open(store, provider)?
    {
        return Ok(Some(primary));
    }
    if !fallback.is_empty()
        && fallback != primary
        && !crate::osint::key_exhausted(fallback)
        && quota_open(store, &format!("{provider}:fallback"))?
    {
        return Ok(Some(fallback));
    }
    Ok(None)
}

fn quota_bucket(provider: &str, active: &str, primary: &str) -> String {
    if active.trim() == primary.trim() {
        provider.to_string()
    } else {
        format!("{provider}:fallback")
    }
}

fn quota_fault(fault: &ProviderFault) -> bool {
    crate::osint::quota_limited("failed", Some(&fault.summary))
}

struct SpareFetch {
    body: Option<serde_json::Value>,
    fault: Option<ProviderFault>,
    bucket: String,
}

/// One provider call. A rate or quota fault retries once with `spare` when it differs.
async fn fetch_with_spare_key<F>(
    fetch: &mut F,
    user_agent: &str,
    provider: &str,
    primary: &str,
    key: &str,
    spare: &str,
    mut spec_for: impl FnMut(&str) -> Result<CallSpec>,
) -> SpareFetch
where
    F: FnMut(HttpCall) -> FetchFut,
{
    let mut active = key.to_string();
    let mut tried_spare = false;
    loop {
        let spec = match spec_for(&active) {
            Ok(spec) => spec,
            Err(err) => {
                return SpareFetch {
                    body: None,
                    fault: Some(ProviderFault {
                        summary: one_line(&err.to_string()),
                        body: String::new(),
                    }),
                    bucket: quota_bucket(provider, &active, primary),
                };
            }
        };
        match dispatch(fetch, spec, user_agent).await {
            Ok(body) => {
                return SpareFetch {
                    body: Some(body),
                    fault: None,
                    bucket: quota_bucket(provider, &active, primary),
                };
            }
            Err(fault)
                if !tried_spare && quota_fault(&fault) && !spare.is_empty() && spare != active =>
            {
                crate::osint::note_key_exhausted(&active);
                active = spare.to_string();
                tried_spare = true;
                continue;
            }
            Err(fault) => {
                return SpareFetch {
                    body: None,
                    fault: Some(fault),
                    bucket: quota_bucket(provider, &active, primary),
                };
            }
        }
    }
}

fn apply_hits(
    hits: &[Hit],
    provider: &str,
    stats: &mut RunStats,
    seen: &mut Vec<Seen>,
    emit: &mut impl FnMut(AtlasEvent),
    store: &Store,
    run_id: &str,
) -> Result<()> {
    if provider == "gnews" || provider == "newsdata" {
        for hit in hits {
            *stats.counts.entry(hit.country.clone()).or_default() += 1;
        }
        if !stats.scored {
            stats.origins = stats
                .counts
                .iter()
                .map(|(country, volume)| OriginStat {
                    country: country.clone(),
                    tier: 0,
                    temperature: 0.0,
                    volume: *volume,
                    articles: 0,
                })
                .collect();
        }
        return Ok(());
    }
    for hit in hits {
        let temperature = stats
            .origins
            .iter()
            .find(|row| row.country == hit.country)
            .map(|row| row.temperature)
            .unwrap_or(0.0);
        let article = article_from(hit, provider, temperature);
        match judge(&article, seen) {
            Verdict::Drop => {}
            Verdict::Keep => {
                if let Some(row) = stats
                    .origins
                    .iter_mut()
                    .find(|row| row.country == article.country)
                {
                    row.articles += 1;
                }
                remember(seen, &article, None);
                store.atlas_upsert_article(&article_row(run_id, &article))?;
                emit(AtlasEvent::Article(article));
            }
            Verdict::Replace(id) => {
                store.atlas_delete_article(run_id, &id)?;
                remember(seen, &article, Some(&id));
                store.atlas_upsert_article(&article_row(run_id, &article))?;
                emit(AtlasEvent::Replaced { id, article });
            }
        }
    }
    Ok(())
}

fn article_row(run_id: &str, article: &FeedArticle) -> AtlasArticleRow {
    AtlasArticleRow {
        run_id: run_id.into(),
        id: article.id.clone(),
        title: article.title.clone(),
        description: article.description.clone(),
        url: article.url.clone(),
        country: article.country.clone(),
        source_name: article.source_name.clone(),
        source_domain: article.source_domain.clone(),
        author: article.author.clone(),
        image_url: article.image_url.clone(),
        published_at: article.published_at.clone(),
        provider: article.provider.clone(),
        temperature: article.temperature,
        category: category_tag(&article.category).into(),
        seen_at: article.seen_at.clone(),
    }
}

fn article_from_row(row: &AtlasArticleRow) -> FeedArticle {
    FeedArticle {
        id: row.id.clone(),
        title: row.title.clone(),
        description: row.description.clone(),
        url: row.url.clone(),
        country: row.country.clone(),
        source_name: row.source_name.clone(),
        source_domain: row.source_domain.clone(),
        author: row.author.clone(),
        image_url: row.image_url.clone(),
        published_at: row.published_at.clone(),
        provider: row.provider.clone(),
        temperature: row.temperature,
        seen_at: row.seen_at.clone(),
        category: row.category.clone(),
    }
}

fn require_key(value: &str, name: &str) -> Result<()> {
    if value.trim().is_empty() {
        Err(anyhow!(
            "Enter the {name} on its OSINT tool, or set its environment variable"
        ))
    } else {
        Ok(())
    }
}

fn require_either(primary: &str, fallback: &str, name: &str) -> Result<()> {
    if primary.trim().is_empty() && fallback.trim().is_empty() {
        require_key("", name)
    } else {
        Ok(())
    }
}

/// Run or resume Atlas. `resume` continues the newest paused run. Articles already
/// on `feed` are the dedup set for this process; they are not read back from disk.
pub struct RunInput<'a> {
    pub db_path: &'a Path,
    pub pause: &'a AtomicBool,
    pub keys: &'a ProviderKeys,
    pub user_agent: &'a str,
    pub pace: bool,
    pub resume: bool,
    pub feed: &'a [FeedArticle],
    /// Classifier account. Empty means every saved article stays `unk`.
    /// A Jev decisions model tags categories through the decisions endpoint.
    pub classifier: Option<ProviderSecret>,
    /// Synthesis account. Extracts claims. A decisions model is not used here.
    pub synthesizer: Option<ProviderSecret>,
}

pub async fn run_atlas<F>(
    input: RunInput<'_>,
    mut emit: impl FnMut(AtlasEvent) + Send,
    mut fetch: F,
) -> Result<Stop>
where
    F: FnMut(HttpCall) -> FetchFut + Send,
{
    let RunInput {
        db_path,
        pause,
        keys,
        user_agent,
        pace,
        resume,
        feed,
        classifier,
        synthesizer,
    } = input;
    let store = Store::open(db_path)?;
    let (run_id, mut cursor, mut stats) = if resume {
        let Some(run) = store.atlas_latest_run()? else {
            return Ok(Stop::Failed("No Atlas run to resume".into()));
        };
        if run.state != "paused" {
            return Ok(Stop::Failed("The latest Atlas run is not paused".into()));
        }
        store.atlas_set_state(&run.id, "running", &run.note, false)?;
        (
            run.id,
            parse_cursor(&run.cursor_json),
            parse_stats(&run.stats_json),
        )
    } else {
        let id = new_run_id();
        let from = lookback_from();
        let cursor = Cursor {
            from: from.clone(),
            ..Cursor::default()
        };
        store.atlas_insert_run(
            &id,
            &serde_json::to_string(&cursor)?,
            &serde_json::to_string(&RunStats::default())?,
        )?;
        (id, cursor, RunStats::default())
    };
    if cursor.from.is_empty() {
        cursor.from = lookback_from();
    }
    if cursor.phase <= 1 {
        require_either(&keys.gnews, &keys.gnews_fallback, "GNews API key")?;
        require_either(&keys.newsdata, &keys.newsdata_fallback, "NewsData API key")?;
    }
    let mut seen: Vec<Seen> = feed.iter().map(seen_of).collect();
    for row in store.atlas_list_articles(&run_id)? {
        let article = article_from_row(&row);
        if seen.iter().all(|item| item.id != article.id) {
            seen.push(seen_of(&article));
        }
    }
    let mut last_gnews: Option<std::time::Instant> = None;
    let user_agent = if user_agent.trim().is_empty() {
        crate::osint::DEFAULT_USER_AGENT.to_string()
    } else {
        user_agent.to_string()
    };

    if cursor.phase <= 1 {
        let providers = ["gnews", "newsdata"];
        let start = providers
            .iter()
            .position(|provider| *provider == cursor.leg)
            .unwrap_or(0);
        for provider in providers.into_iter().skip(start) {
            let continuing = provider == cursor.leg.as_str();
            let mut page = if continuing { cursor.chunk } else { 0 };
            let mut token = if continuing {
                cursor.page_token.clone()
            } else {
                String::new()
            };
            let mut kept = if continuing { cursor.kept } else { 0 };
            while kept < PHASE1_TARGET && page < PHASE1_MAX_PAGES {
                let here = Cursor {
                    phase: 1,
                    chunk: page,
                    leg: provider.into(),
                    country: 0,
                    from: cursor.from.clone(),
                    page_token: token.clone(),
                    kept,
                };
                if pause.load(Ordering::Relaxed) {
                    return park(&store, &run_id, &here, &stats, &mut emit);
                }
                let (primary, fallback) = keys.pair(provider);
                let Some(key) = open_key(&store, provider, primary, fallback)? else {
                    emit(AtlasEvent::Note(format!("{provider} daily quota is spent")));
                    break;
                };
                let spare = if key == primary.trim() {
                    fallback.trim()
                } else {
                    ""
                };
                if pace && provider == "gnews" {
                    if let Some(last) = last_gnews {
                        let wait = std::time::Duration::from_secs(1).saturating_sub(last.elapsed());
                        if !wait.is_zero() {
                            tokio::time::sleep(wait).await;
                        }
                    }
                }
                emit(AtlasEvent::Status(format!(
                    "Latest {provider} page {} · {kept}/{PHASE1_TARGET}",
                    page + 1
                )));
                let mut stop_provider = false;
                let fetched = fetch_with_spare_key(
                    &mut fetch,
                    &user_agent,
                    provider,
                    primary,
                    key,
                    spare,
                    |active| phase1_call(provider, page, &token, active),
                )
                .await;
                let bucket = fetched.bucket;
                match fetched.body {
                    Some(body) => {
                        if provider == "gnews" {
                            last_gnews = Some(std::time::Instant::now());
                        }
                        let more = atlas_news::next_page(provider, &body);
                        match atlas_news::discovery_hits(provider, &body) {
                            Ok(mut hits) => {
                                hits.retain(|hit| {
                                    published_in_window(&hit.published_at, &cursor.from)
                                });
                                let room = (PHASE1_TARGET - kept) as usize;
                                hits.truncate(room);
                                kept += hits.len() as u32;
                                apply_hits(
                                    &hits, provider, &mut stats, &mut seen, &mut emit, &store,
                                    &run_id,
                                )?;
                                token = if provider == "newsdata" {
                                    more.clone().unwrap_or_default()
                                } else {
                                    String::new()
                                };
                                if more.is_none() {
                                    stop_provider = true;
                                }
                            }
                            Err(err) => {
                                emit(parse_fault(provider, err.to_string(), &body));
                                stop_provider = true;
                            }
                        }
                    }
                    None => {
                        if let Some(fault) = fetched.fault {
                            emit(AtlasEvent::Fault(fault));
                        }
                        stop_provider = true;
                    }
                }
                let _ = store.atlas_quota_bump(&bucket, &utc_day())?;
                page += 1;
                emit(AtlasEvent::Stats(stats.clone()));
                if stop_provider || kept >= PHASE1_TARGET || page >= PHASE1_MAX_PAGES {
                    break;
                }
                let next = Cursor {
                    phase: 1,
                    chunk: page,
                    leg: provider.into(),
                    country: 0,
                    from: cursor.from.clone(),
                    page_token: token.clone(),
                    kept,
                };
                save(&store, &run_id, &next, &stats)?;
                cursor = next;
            }
        }
        let bands = score_counts(&stats.counts);
        let previous: HashMap<String, u32> = stats
            .origins
            .iter()
            .map(|row| (row.country.clone(), row.articles))
            .collect();
        stats.origins = bands
            .iter()
            .map(|band| OriginStat {
                country: band.country.clone(),
                tier: band.tier,
                temperature: band.temperature,
                volume: band.count,
                articles: previous.get(&band.country).copied().unwrap_or(0),
            })
            .collect();
        stats.scored = true;
        cursor = Cursor {
            phase: 2,
            chunk: 0,
            leg: "newsapi".into(),
            country: 0,
            from: cursor.from.clone(),
            page_token: String::new(),
            kept: 0,
        };
        save(&store, &run_id, &cursor, &stats)?;
        emit(AtlasEvent::Stats(stats.clone()));
    }

    if cursor.phase == 2 {
        require_either(&keys.newsapi, &keys.newsapi_fallback, "NewsAPI key")?;
        require_either(&keys.currents, &keys.currents_fallback, "Currents API key")?;
        if stats.origins.is_empty() {
            store.atlas_set_state(
                &run_id,
                "completed",
                "No flashpoints in the 48-hour window",
                true,
            )?;
            emit(AtlasEvent::Status(
                "No flashpoints in the 48-hour window".into(),
            ));
            return Ok(Stop::Finished);
        }
        let countries: Vec<String> = stats
            .origins
            .iter()
            .map(|row| row.country.clone())
            .collect();
        let phase2: Vec<Job> = countries
            .iter()
            .enumerate()
            .flat_map(|(index, _)| {
                [
                    Job {
                        phase: 2,
                        chunk: 0,
                        leg: "newsapi",
                        country: index,
                        provider: "newsapi",
                    },
                    Job {
                        phase: 2,
                        chunk: 0,
                        leg: "currents",
                        country: index,
                        provider: "currents",
                    },
                ]
            })
            .collect();
        let mut newsapi_skipped: Vec<String> = Vec::new();
        let mut currents_skipped: Vec<String> = Vec::new();
        for job in &phase2 {
            if job_is_before(job, &cursor) {
                continue;
            }
            if pause.load(Ordering::Relaxed) {
                return park(
                    &store,
                    &run_id,
                    &cursor_at(job, &cursor.from),
                    &stats,
                    &mut emit,
                );
            }
            let country = &countries[job.country];
            let supported = match job.provider {
                "newsapi" => atlas_news::newsapi_country_supported(country),
                "currents" => atlas_news::currents_country_supported(country),
                _ => true,
            };
            if !supported {
                match job.provider {
                    "newsapi" => newsapi_skipped.push(country.clone()),
                    "currents" => currents_skipped.push(country.clone()),
                    _ => {}
                }
                continue;
            }
            let (primary, fallback) = keys.pair(job.provider);
            let Some(key) = open_key(&store, job.provider, primary, fallback)? else {
                emit(AtlasEvent::Note(format!(
                    "{} daily quota is spent",
                    job.provider
                )));
                continue;
            };
            let spare = if key == primary.trim() {
                fallback.trim()
            } else {
                ""
            };
            emit(AtlasEvent::Status(format!(
                "Headlines {} · {}",
                country.to_ascii_uppercase(),
                job.provider
            )));
            let fetched = fetch_with_spare_key(
                &mut fetch,
                &user_agent,
                job.provider,
                primary,
                key,
                spare,
                |active| phase2_call(job.provider, country, active),
            )
            .await;
            let bucket = fetched.bucket;
            match fetched.body {
                Some(body) => match atlas_news::headline_hits(job.provider, country, &body) {
                    Ok(hits) => apply_hits(
                        &hits,
                        job.provider,
                        &mut stats,
                        &mut seen,
                        &mut emit,
                        &store,
                        &run_id,
                    )?,
                    Err(err) => emit(parse_fault(job.provider, err.to_string(), &body)),
                },
                None => {
                    if let Some(fault) = fetched.fault {
                        emit(AtlasEvent::Fault(fault));
                    }
                }
            }
            let _ = store.atlas_quota_bump(&bucket, &utc_day())?;
            let next = next_after_phase2(job, countries.len());
            save(&store, &run_id, &cursor_at(&next, &cursor.from), &stats)?;
            emit(AtlasEvent::Stats(stats.clone()));
        }
        if !newsapi_skipped.is_empty() {
            let names = newsapi_skipped
                .iter()
                .map(|code| country_label(code))
                .collect::<Vec<_>>()
                .join(", ");
            emit(AtlasEvent::Note(format!(
                "NewsAPI was not queried for {names}. Top headlines has no feed for those countries."
            )));
        }
        if !currents_skipped.is_empty() {
            let names = currents_skipped
                .iter()
                .map(|code| country_label(code))
                .collect::<Vec<_>>()
                .join(", ");
            emit(AtlasEvent::Note(format!(
                "Currents was not queried for {names}. Latest news has no feed for those countries."
            )));
        }
        cursor = Cursor {
            phase: 3,
            chunk: 0,
            leg: "classify".into(),
            country: 0,
            from: cursor.from.clone(),
            page_token: String::new(),
            kept: 0,
        };
        save(&store, &run_id, &cursor, &stats)?;
    }

    if cursor.phase == 3 {
        let articles = store.atlas_list_articles(&run_id)?;
        if !articles.is_empty() && classifier.is_none() {
            emit(AtlasEvent::Note(
                "Classifier is not configured. Articles stay tagged unk.".into(),
            ));
        }
        if let Some(secret) = classifier.as_ref().filter(|_| !articles.is_empty()) {
            let mut noted = false;
            let total = articles.len();
            for (index, row) in articles.iter().enumerate().skip(cursor.country) {
                if pause.load(Ordering::Relaxed) {
                    cursor.country = index;
                    cursor.phase = 3;
                    cursor.leg = "classify".into();
                    return park(&store, &run_id, &cursor, &stats, &mut emit);
                }
                emit(AtlasEvent::Status(format!(
                    "Classifying {}/{total}",
                    index + 1
                )));
                let article = article_from_row(row);
                let category = match tag_article(secret, &article).await {
                    Ok(category) => category,
                    Err(err) => {
                        if !noted {
                            emit(AtlasEvent::Note(format!(
                                "Classifier could not tag an article ({err}). Untagged articles stay unk."
                            )));
                            noted = true;
                        }
                        "unk".into()
                    }
                };
                let category = category_tag(&category).to_string();
                store.atlas_set_category(&run_id, &row.id, &category)?;
                emit(AtlasEvent::Classified {
                    id: row.id.clone(),
                    category,
                });
                cursor.country = index + 1;
                cursor.phase = 3;
                cursor.leg = "classify".into();
                save(&store, &run_id, &cursor, &stats)?;
            }
        }
        cursor = Cursor {
            phase: 4,
            chunk: 0,
            leg: "insights".into(),
            country: 0,
            from: cursor.from.clone(),
            page_token: String::new(),
            kept: 0,
        };
        save(&store, &run_id, &cursor, &stats)?;
    }

    if cursor.phase == 4 && cursor.leg != "insights_done" {
        if pause.load(Ordering::Relaxed) {
            return park(&store, &run_id, &cursor, &stats, &mut emit);
        }
        let articles = store.atlas_list_articles(&run_id)?;
        if let Some(existing) = atlas_insights::resume_stats(
            &store,
            &run_id,
            &articles,
            &stats.origins,
            &stats.insights,
        )? {
            stats.insights = existing;
        } else if let Some(secret) = synthesizer.as_ref() {
            if provider::is_decisions_model(&secret.model) {
                if !articles.is_empty() {
                    emit(AtlasEvent::Note(
                        "Synthesis is a decisions model, so no claims were extracted. Choose a chat model for Synthesis."
                            .into(),
                    ));
                }
            } else {
                emit(AtlasEvent::Status("Extracting insights".into()));
                match atlas_insights::extract(secret, &articles, &stats.origins).await {
                    Ok(extraction) => {
                        if !extraction.settled.claims.is_empty() {
                            if let Err(err) = store.persist_atlas_insights(
                                &run_id,
                                &extraction.settled.claims,
                                &extraction.settled.relations,
                                &extraction.settled.brief,
                                &extraction.settled.entity_path,
                            ) {
                                emit(AtlasEvent::Note(format!(
                                    "Insights were not saved ({err})."
                                )));
                            }
                        }
                        stats.insights = extraction.settled.stats;
                        if let Some(err) = extraction.context_error {
                            emit(AtlasEvent::Note(format!(
                                "Context claims were not extracted ({err})."
                            )));
                        }
                    }
                    Err(err) => {
                        emit(AtlasEvent::Note(format!(
                            "Insights were not extracted ({err})."
                        )));
                    }
                }
            }
        } else if !articles.is_empty() {
            emit(AtlasEvent::Note(
                "Synthesis is not configured. No insights extracted.".into(),
            ));
        }
        cursor.leg = "insights_done".into();
        save(&store, &run_id, &cursor, &stats)?;
    }

    store.atlas_set_state(&run_id, "completed", "", true)?;
    emit(AtlasEvent::Status("Pipeline complete".into()));
    emit(AtlasEvent::Stats(stats));
    Ok(Stop::Finished)
}

fn next_after_phase2(job: &Job, countries: usize) -> Job {
    if job.leg == "newsapi" {
        Job {
            leg: "currents",
            provider: "currents",
            ..job.clone()
        }
    } else if job.country + 1 < countries {
        Job {
            country: job.country + 1,
            leg: "newsapi",
            provider: "newsapi",
            ..job.clone()
        }
    } else {
        job.clone()
    }
}

async fn tag_article(secret: &ProviderSecret, article: &FeedArticle) -> Result<String> {
    if provider::is_decisions_model(&secret.model) {
        let (state, questions) = classification_request(article);
        let response = provider::decide(secret, &state, &questions).await?;
        return Ok(category_from_decisions(&response));
    }
    let messages = [
        provider::ChatMessage {
            role: "system".into(),
            content: "Reply with one category id only: geopolitical, economic, military, information, stability, technology, or unk.".into(),
            tool_call_id: None,
            tool_calls: Vec::new(),
        },
        provider::ChatMessage {
            role: "user".into(),
            content: format!("{}\n\n{}", article.title, article.description),
            tool_call_id: None,
            tool_calls: Vec::new(),
        },
    ];
    let done = provider::complete(secret, &messages, &[], |_| {}).await?;
    Ok(normalize_category(&done.content))
}

fn published_in_window(published: &str, from: &str) -> bool {
    let Ok(from) = chrono::DateTime::parse_from_rfc3339(from) else {
        return true;
    };
    let published = published.trim();
    if published.is_empty() {
        return true;
    }
    if let Ok(stamp) = chrono::DateTime::parse_from_rfc3339(published) {
        return stamp >= from;
    }
    chrono::NaiveDateTime::parse_from_str(published, "%Y-%m-%d %H:%M:%S")
        .map(|naive| naive.and_utc() >= from)
        .unwrap_or(true)
}

fn quota_open(store: &Store, provider: &str) -> Result<bool> {
    let used = store.atlas_quota_used(provider, &utc_day())?;
    Ok(used < daily_cap(provider))
}

fn save(store: &Store, id: &str, cursor: &Cursor, stats: &RunStats) -> Result<()> {
    store.atlas_save(
        id,
        &serde_json::to_string(cursor)?,
        &serde_json::to_string(stats)?,
    )
}

fn park(
    store: &Store,
    id: &str,
    cursor: &Cursor,
    stats: &RunStats,
    emit: &mut impl FnMut(AtlasEvent),
) -> Result<Stop> {
    save(store, id, cursor, stats)?;
    store.atlas_set_state(id, "paused", "Paused", false)?;
    emit(AtlasEvent::Stats(stats.clone()));
    emit(AtlasEvent::Status("Paused".into()));
    Ok(Stop::Paused)
}

fn parse_fault(_provider: &str, summary: String, body: &serde_json::Value) -> AtlasEvent {
    AtlasEvent::Fault(ProviderFault {
        summary: one_line(&summary),
        body: serde_json::to_string_pretty(body).unwrap_or_else(|_| body.to_string()),
    })
}

fn one_line(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    flat.chars().take(180).collect()
}

fn pretty_body(raw: &str) -> String {
    let raw = raw.trim();
    let text = serde_json::from_str::<serde_json::Value>(raw)
        .ok()
        .and_then(|value| serde_json::to_string_pretty(&value).ok())
        .unwrap_or_else(|| raw.to_string());
    const MAX: usize = 16_000;
    if text.chars().count() <= MAX {
        text
    } else {
        let mut cut: String = text.chars().take(MAX).collect();
        cut.push_str("\n…");
        cut
    }
}

fn json_message(raw: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(raw).ok()?;
    let message = value
        .pointer("/results/message")
        .or_else(|| value.get("message"))
        .or_else(|| value.get("errors"))?;
    let text = message
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| message.to_string());
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

fn provider_fault(provider: &str, status: u16, raw: &str) -> ProviderFault {
    let named = atlas_news::http_error(provider, status, raw);
    let summary = match named {
        Some(text) if !text.contains('{') => text,
        _ => match json_message(raw) {
            Some(message) => format!("{provider} HTTP {status}: {}", one_line(&message)),
            None => format!("{provider} HTTP {status}"),
        },
    };
    ProviderFault {
        summary: one_line(&summary),
        body: pretty_body(raw),
    }
}

async fn dispatch<F>(
    fetch: &mut F,
    spec: CallSpec,
    user_agent: &str,
) -> Result<serde_json::Value, ProviderFault>
where
    F: FnMut(HttpCall) -> FetchFut,
{
    let provider = spec.provider;
    let mut headers = spec.headers;
    headers.push(("User-Agent".into(), user_agent.into()));
    headers.push(("Accept".into(), "application/json".into()));
    let reply = fetch(HttpCall {
        provider: provider.into(),
        url: spec.url,
        headers,
    })
    .await
    .map_err(|err| ProviderFault {
        summary: format!("{provider} request failed: {err}"),
        body: String::new(),
    })?;
    if !(200..300).contains(&reply.status) {
        return Err(provider_fault(provider, reply.status, &reply.body));
    }
    serde_json::from_str(&reply.body).map_err(|err| ProviderFault {
        summary: format!("{provider} malformed JSON: {err}"),
        body: pretty_body(&reply.body),
    })
}

/// Live pipeline. Uses the process HTTP client and paces GNews by one second.
pub async fn run_live(
    db_path: &Path,
    pause: &AtomicBool,
    keys: &ProviderKeys,
    user_agent: &str,
    resume: bool,
    feed: &[FeedArticle],
    classifier: Option<ProviderSecret>,
    synthesizer: Option<ProviderSecret>,
    emit: impl FnMut(AtlasEvent) + Send,
) -> Result<Stop> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(25))
        .build()?;
    run_atlas(
        RunInput {
            db_path,
            pause,
            keys,
            user_agent,
            pace: true,
            resume,
            feed,
            classifier,
            synthesizer,
        },
        emit,
        move |call: HttpCall| {
            let client = client.clone();
            Box::pin(async move {
                let mut request = client.get(&call.url);
                for (name, value) in &call.headers {
                    request = request.header(name.as_str(), value.as_str());
                }
                let response = request.send().await?;
                let status = response.status().as_u16();
                let body = response.text().await?;
                Ok(HttpReply { status, body })
            })
        },
    )
    .await
}

pub fn charge_newsapi(store: &Store) {
    charge_quota(store, "newsapi");
}

pub fn charge_quota(store: &Store, bucket: &str) {
    let _ = store.atlas_quota_bump(bucket, &utc_day());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[tokio::test]
    async fn insights_do_not_call_a_decisions_model() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("atlas.db");
        let store = Store::open(&path).unwrap();
        let cursor = Cursor {
            phase: 4,
            leg: "insights".into(),
            from: "2026-09-30T00:00:00Z".into(),
            ..Cursor::default()
        };
        store
            .atlas_insert_run(
                "atlas-jev",
                &serde_json::to_string(&cursor).unwrap(),
                &serde_json::to_string(&RunStats::default()).unwrap(),
            )
            .unwrap();
        store
            .atlas_set_state("atlas-jev", "paused", "", false)
            .unwrap();
        store
            .atlas_upsert_article(&AtlasArticleRow {
                run_id: "atlas-jev".into(),
                id: "art-1".into(),
                title: "Putin sanctioned Acme".into(),
                description: String::new(),
                url: "https://example.com/art-1".into(),
                country: "us".into(),
                source_name: "Desk".into(),
                source_domain: "example.com".into(),
                published_at: String::new(),
                provider: "newsapi".into(),
                temperature: 1.0,
                category: "military".into(),
                seen_at: String::new(),
                author: String::new(),
                image_url: String::new(),
            })
            .unwrap();
        drop(store);
        let notes = Arc::new(Mutex::new(Vec::new()));
        let noted = notes.clone();
        let pause = AtomicBool::new(false);
        let keys = keys();
        let secret = ProviderSecret {
            kind: "openrouter".into(),
            base_url: "https://openrouter.ai/api/v1".into(),
            model: "typesafe/jev-1.3".into(),
            api_key: Some("test".into()),
            stt_model: None,
            device: None,
        };
        let stop = run_atlas(
            RunInput {
                db_path: &path,
                pause: &pause,
                keys: &keys,
                user_agent: "Argos test",
                pace: false,
                resume: true,
                feed: &[],
                classifier: None,
                synthesizer: Some(secret),
            },
            move |event| {
                if let AtlasEvent::Note(text) = event {
                    noted.lock().unwrap().push(text);
                }
            },
            |_call| Box::pin(async { Err(anyhow!("insight phase must not fetch news")) }),
        )
        .await
        .unwrap();
        assert_eq!(stop, Stop::Finished);
        let notes = notes.lock().unwrap();
        assert!(notes
            .iter()
            .any(|note| note.contains("decisions model") && note.contains("Synthesis")));
    }

    #[test]
    fn the_classifier_keeps_a_known_choice_and_maps_anything_else_to_unk() {
        let response = DecisionsResponse {
            answers: [(
                "category".into(),
                provider::DecisionAnswer {
                    choice: Some("military".into()),
                    ..provider::DecisionAnswer::default()
                },
            )]
            .into_iter()
            .collect(),
            ..DecisionsResponse::default()
        };
        assert_eq!(category_from_decisions(&response), "military");
        assert_eq!(normalize_category("  UNK "), "unk");
        assert_eq!(normalize_category("sports"), "unk");
        let article = FeedArticle {
            id: "art".into(),
            title: "Joint military exercise begins".into(),
            description: "A live-fire drill followed the troop deployment.".into(),
            url: "https://example.com/drill".into(),
            country: "us".into(),
            source_name: "Wire".into(),
            source_domain: "example.com".into(),
            author: String::new(),
            image_url: String::new(),
            published_at: String::new(),
            provider: "newsapi".into(),
            temperature: 1.0,
            seen_at: String::new(),
            category: "unk".into(),
        };
        let (state, questions) = classification_request(&article);
        assert_eq!(state["title"], "Joint military exercise begins");
        assert!(questions["category"]["criteria"]["unk"].is_string());
        assert!(questions["category"]["criteria"]["military"]
            .as_str()
            .unwrap()
            .contains("live-fire drill"));
    }

    #[test]
    fn one_country_is_max_heat_and_the_bottom_band_is_dropped() {
        assert_eq!(band_sizes(1), (1, 0, 0, 0));
        assert_eq!(band_sizes(10), (2, 3, 3, 2));
        let mut counts = BTreeMap::new();
        for (index, code) in ["aa", "bb", "cc", "dd", "ee", "ff", "gg", "hh", "ii", "jj"]
            .into_iter()
            .enumerate()
        {
            counts.insert(code.into(), (10 - index) as u32);
        }
        let bands = score_counts(&counts);
        assert_eq!(bands.len(), 8);
        assert_eq!(bands[0].country, "aa");
        assert_eq!(bands[0].temperature, 1.0);
        assert_eq!(bands[1].temperature, 1.0);
        assert!((bands[2].temperature - 0.99).abs() < 0.001);
        assert!((bands[4].temperature - 0.70).abs() < 0.001);
        assert!((bands[5].temperature - 0.69).abs() < 0.001);
        assert!((bands[7].temperature - 0.10).abs() < 0.001);
        assert!(bands
            .iter()
            .all(|band| band.country != "ii" && band.country != "jj"));
    }

    #[test]
    fn dedup_drops_a_url_an_exact_title_and_keeps_the_higher_domain() {
        let reuters = FeedArticle {
            id: "a".into(),
            title: "Export restriction widens".into(),
            description: String::new(),
            url: "https://www.reuters.com/world/story".into(),
            country: "us".into(),
            source_name: "Reuters".into(),
            source_domain: "reuters.com".into(),
            author: String::new(),
            image_url: String::new(),
            published_at: "2026-10-01T00:00:00Z".into(),
            provider: "newsapi".into(),
            temperature: 1.0,
            seen_at: String::new(),
            category: "unk".into(),
        };
        let mut seen = vec![seen_of(&reuters)];
        let copy = FeedArticle {
            url: "https://www.reuters.com/world/story/".into(),
            ..reuters.clone()
        };
        assert_eq!(judge(&copy, &seen), Verdict::Drop);
        let syndicated = FeedArticle {
            id: "b".into(),
            title: "Export restriction widens".into(),
            url: "https://example.com/same".into(),
            source_domain: "example.com".into(),
            source_name: "Example".into(),
            ..reuters.clone()
        };
        assert_eq!(judge(&syndicated, &seen), Verdict::Drop);
        let near = FeedArticle {
            id: "c".into(),
            title: "Export restriction widens now".into(),
            url: "https://example.com/near".into(),
            source_domain: "example.com".into(),
            source_name: "Example".into(),
            published_at: "2026-10-02T00:00:00Z".into(),
            ..reuters.clone()
        };
        assert_eq!(judge(&near, &seen), Verdict::Drop);
        let better = FeedArticle {
            id: "d".into(),
            title: "Export restriction widens nows".into(),
            url: "https://www.reuters.com/world/other".into(),
            source_domain: "reuters.com".into(),
            source_name: "Reuters".into(),
            ..near.clone()
        };
        seen.push(seen_of(&near));
        assert_eq!(judge(&better, &seen), Verdict::Replace("c".into()));
    }

    #[test]
    fn gnews_and_newsdata_payloads_yield_country_codes() {
        let gnews = serde_json::json!({"articles":[
            {"title":"Drill","url":"https://example.com/a","source":{"name":"Times","country":"in"}},
            {"title":"No country","url":"https://example.com/b","source":{"name":"Times"}}
        ]});
        let hits = atlas_news::discovery_hits("gnews", &gnews).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].country, "in");
        let newsdata = serde_json::json!({"status":"success","results":[
            {"title":"Quota","link":"https://example.com/c","country":["us","United States"],"source_name":"Wire"}
        ]});
        let hits = atlas_news::discovery_hits("newsdata", &newsdata).unwrap();
        assert_eq!(hits[0].country, "us");
    }

    #[tokio::test]
    async fn pause_then_resume_does_not_repeat_the_completed_request() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("atlas.db");
        let pause = Arc::new(AtomicBool::new(false));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let flag = pause.clone();
        let log = seen.clone();
        let keys = keys();
        let stop = run_atlas(
            RunInput {
                db_path: &path,
                pause: &pause,
                keys: &keys,
                user_agent: "Argos test",
                pace: false,
                resume: false,
                feed: &[],
                classifier: None,
                synthesizer: None,
            },
            |_| {},
            move |call: HttpCall| {
                let flag = flag.clone();
                let log = log.clone();
                Box::pin(async move {
                    log.lock().unwrap().push(call.url.clone());
                    flag.store(true, Ordering::Relaxed);
                    Ok(reply_for(&call))
                })
            },
        )
        .await
        .unwrap();
        assert_eq!(stop, Stop::Paused);
        assert_eq!(seen.lock().unwrap().len(), 1);
        let store = Store::open(&path).unwrap();
        let run = store.atlas_latest_run().unwrap().unwrap();
        assert_eq!(run.state, "paused");
        let cursor = parse_cursor(&run.cursor_json);
        assert_eq!(cursor.phase, 1);
        assert_eq!(cursor.leg, "newsdata");
        pause.store(false, Ordering::Relaxed);
        let log = seen.clone();
        let before = seen.lock().unwrap().len();
        let _ = run_atlas(
            RunInput {
                db_path: &path,
                pause: &pause,
                keys: &keys,
                user_agent: "Argos test",
                pace: false,
                resume: true,
                feed: &[],
                classifier: None,
                synthesizer: None,
            },
            |_| {},
            move |call: HttpCall| {
                let log = log.clone();
                Box::pin(async move {
                    log.lock().unwrap().push(call.provider.clone());
                    Err(anyhow!("stop"))
                })
            },
        )
        .await
        .unwrap();
        let urls = seen.lock().unwrap();
        assert!(urls.len() > before);
        assert_eq!(urls[before], "newsdata");
        assert!(!urls[before].contains("gnews"));
    }

    #[tokio::test]
    async fn phase2_stops_when_the_provider_quota_is_spent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("atlas.db");
        let store = Store::open(&path).unwrap();
        let stats = RunStats {
            scored: true,
            origins: vec![
                OriginStat {
                    country: "us".into(),
                    tier: 1,
                    temperature: 1.0,
                    volume: 10,
                    articles: 0,
                },
                OriginStat {
                    country: "cn".into(),
                    tier: 2,
                    temperature: 0.8,
                    volume: 4,
                    articles: 0,
                },
            ],
            ..RunStats::default()
        };
        let cursor = Cursor {
            phase: 2,
            leg: "newsapi".into(),
            from: "2026-09-30T00:00:00Z".into(),
            ..Cursor::default()
        };
        store
            .atlas_insert_run(
                "atlas-quota",
                &serde_json::to_string(&cursor).unwrap(),
                &serde_json::to_string(&stats).unwrap(),
            )
            .unwrap();
        store
            .atlas_set_state("atlas-quota", "paused", "", false)
            .unwrap();
        let day = utc_day();
        for _ in 0..daily_cap("newsapi") {
            store.atlas_quota_bump("newsapi", &day).unwrap();
        }
        for _ in 0..daily_cap("currents") - 1 {
            store.atlas_quota_bump("currents", &day).unwrap();
        }
        drop(store);
        let calls = Arc::new(Mutex::new(Vec::new()));
        let log = calls.clone();
        let pause = AtomicBool::new(false);
        let keys = keys();
        let stop = run_atlas(
            RunInput {
                db_path: &path,
                pause: &pause,
                keys: &keys,
                user_agent: "Argos test",
                pace: false,
                resume: true,
                feed: &[],
                classifier: None,
                synthesizer: None,
            },
            |_| {},
            move |call: HttpCall| {
                let log = log.clone();
                Box::pin(async move {
                    log.lock()
                        .unwrap()
                        .push(format!("{} {}", call.provider, call.url));
                    Ok(reply_for(&call))
                })
            },
        )
        .await
        .unwrap();
        assert_eq!(stop, Stop::Finished);
        let calls = calls.lock().unwrap();
        let currents: Vec<_> = calls
            .iter()
            .filter(|line| line.starts_with("currents"))
            .collect();
        let newsapi: Vec<_> = calls
            .iter()
            .filter(|line| line.starts_with("newsapi"))
            .collect();
        assert!(newsapi.is_empty());
        assert_eq!(currents.len(), 1);
        assert!(currents[0].contains("country=US"));
    }

    /// A spent primary daily cap uses the fallback account instead of stopping.
    #[tokio::test]
    async fn a_spent_primary_quota_uses_the_fallback_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("atlas.db");
        let store = Store::open(&path).unwrap();
        let stats = RunStats {
            scored: true,
            origins: vec![OriginStat {
                country: "us".into(),
                tier: 1,
                temperature: 1.0,
                volume: 10,
                articles: 0,
            }],
            ..RunStats::default()
        };
        let cursor = Cursor {
            phase: 2,
            leg: "newsapi".into(),
            from: "2026-09-30T00:00:00Z".into(),
            ..Cursor::default()
        };
        store
            .atlas_insert_run(
                "atlas-fallback",
                &serde_json::to_string(&cursor).unwrap(),
                &serde_json::to_string(&stats).unwrap(),
            )
            .unwrap();
        store
            .atlas_set_state("atlas-fallback", "paused", "", false)
            .unwrap();
        let day = utc_day();
        for _ in 0..daily_cap("newsapi") {
            store.atlas_quota_bump("newsapi", &day).unwrap();
        }
        drop(store);
        let calls = Arc::new(Mutex::new(Vec::new()));
        let log = calls.clone();
        let pause = AtomicBool::new(false);
        let keys = ProviderKeys {
            newsapi: "newsapi-primary-quota".into(),
            newsapi_fallback: "newsapi-spare-quota".into(),
            currents: "currents-key".into(),
            ..ProviderKeys::default()
        };
        run_atlas(
            RunInput {
                db_path: &path,
                pause: &pause,
                keys: &keys,
                user_agent: "Argos test",
                pace: false,
                resume: true,
                feed: &[],
                classifier: None,
                synthesizer: None,
            },
            |_| {},
            move |call: HttpCall| {
                let log = log.clone();
                Box::pin(async move {
                    let key = call
                        .headers
                        .iter()
                        .find(|(name, _)| name.eq_ignore_ascii_case("x-api-key"))
                        .map(|(_, value)| value.clone())
                        .unwrap_or_default();
                    log.lock().unwrap().push(format!("{} {key}", call.provider));
                    Ok(reply_for(&call))
                })
            },
        )
        .await
        .unwrap();
        let calls = calls.lock().unwrap();
        let newsapi: Vec<_> = calls
            .iter()
            .filter(|line| line.starts_with("newsapi "))
            .collect();
        assert!(!newsapi.is_empty(), "{calls:?}");
        assert!(
            newsapi
                .iter()
                .all(|line| line.ends_with("newsapi-spare-quota")),
            "{newsapi:?}"
        );
    }

    #[test]
    fn article_card_labels_title_publisher_author_and_classification() {
        let card = format_article_card(
            "Opinion: International flavour",
            "Joel Schlesinger; Laurie",
            "winnipegfreepress.com",
            "",
            "ca",
            "economic",
            "2026-10-03 07:01:24 +0000",
            0.69,
            "",
            "https://cdn.example/photo.jpg",
            "https://www.winnipegfreepress.com/story",
        );
        assert!(card.contains("Title: Opinion: International flavour"));
        assert!(card.contains("Publisher: winnipegfreepress.com"));
        assert!(card.contains("Author: Joel Schlesinger; Laurie"));
        assert!(card.contains("Category: Economic & Resource"));
        assert!(card.contains(&format!(
            "Published: {}",
            friendly_date("2026-10-03 07:01:24 +0000")
        )));
        assert!(card.contains("Temperature: 0.69"));
        assert!(card.contains("Image: https://cdn.example/photo.jpg"));
        assert!(card.contains("Article: https://www.winnipegfreepress.com/story"));
        assert!(!card.contains("Provider:"));
        let (publisher, author) =
            publisher_and_author("Jane Reporter", "reuters.com", "Jane Reporter");
        assert_eq!(publisher, "reuters.com");
        assert_eq!(author, "Jane Reporter");
        let (publisher, author) = publisher_and_author("Reuters", "reuters.com", "");
        assert_eq!(publisher, "Reuters");
        assert_eq!(author, "Reuters");
        let (publisher, author) = publisher_and_author("", "", "Jane Reporter");
        assert_eq!(publisher, "Jane Reporter");
        assert_eq!(author, "Jane Reporter");
        assert_eq!(friendly_date("2026-10-01"), "Oct 1, 2026");
        let stamp = chrono::DateTime::parse_from_rfc3339("2026-10-01T15:04:00Z").unwrap();
        let expected = loosen_date(
            &stamp
                .with_timezone(&chrono::Local)
                .format("%b %d, %Y, %I:%M %p")
                .to_string(),
        );
        assert_eq!(friendly_date("2026-10-01T15:04:00Z"), expected);
        assert!(expected.contains("AM") || expected.contains("PM"));
    }

    #[test]
    fn country_labels_show_the_name_and_the_code() {
        assert_eq!(country_label("us"), "United States (US)");
        assert_eq!(country_label("CN"), "China (CN)");
        assert_eq!(country_label("qa"), "Qatar (QA)");
        let card = format_run_card(&crate::store::AtlasRunRow {
            id: "atlas-1".into(),
            state: "completed".into(),
            phase: 2,
            cursor_json: String::new(),
            stats_json: serde_json::to_string(&RunStats {
                scored: true,
                origins: vec![OriginStat {
                    country: "de".into(),
                    tier: 1,
                    temperature: 1.0,
                    volume: 3,
                    articles: 2,
                }],
                ..RunStats::default()
            })
            .unwrap(),
            note: String::new(),
            started_at: "2026-10-01T00:00:00Z".into(),
            finished_at: String::new(),
        });
        let country = card.find("Germany (DE)").unwrap();
        let insights = card.find("No insights extracted for this cycle.").unwrap();
        assert!(country < insights);
    }

    #[tokio::test]
    async fn newsapi_is_not_queried_for_spain() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("atlas.db");
        let store = Store::open(&path).unwrap();
        let stats = RunStats {
            scored: true,
            origins: vec![
                OriginStat {
                    country: "us".into(),
                    tier: 1,
                    temperature: 1.0,
                    volume: 4,
                    articles: 0,
                },
                OriginStat {
                    country: "es".into(),
                    tier: 2,
                    temperature: 0.8,
                    volume: 2,
                    articles: 0,
                },
            ],
            ..RunStats::default()
        };
        let cursor = Cursor {
            phase: 2,
            leg: "newsapi".into(),
            from: "2026-09-30T00:00:00Z".into(),
            ..Cursor::default()
        };
        store
            .atlas_insert_run(
                "atlas-es",
                &serde_json::to_string(&cursor).unwrap(),
                &serde_json::to_string(&stats).unwrap(),
            )
            .unwrap();
        store
            .atlas_set_state("atlas-es", "paused", "", false)
            .unwrap();
        drop(store);
        let urls = Arc::new(Mutex::new(Vec::new()));
        let notes = Arc::new(Mutex::new(Vec::new()));
        let log = urls.clone();
        let noted = notes.clone();
        let pause = AtomicBool::new(false);
        let keys = keys();
        run_atlas(
            RunInput {
                db_path: &path,
                pause: &pause,
                keys: &keys,
                user_agent: "Argos test",
                pace: false,
                resume: true,
                feed: &[],
                classifier: None,
                synthesizer: None,
            },
            move |event| {
                if let AtlasEvent::Note(text) = event {
                    noted.lock().unwrap().push(text);
                }
            },
            move |call: HttpCall| {
                let log = log.clone();
                Box::pin(async move {
                    if call.provider == "newsapi" {
                        log.lock().unwrap().push(call.url.clone());
                    }
                    Ok(reply_for(&call))
                })
            },
        )
        .await
        .unwrap();
        let urls = urls.lock().unwrap();
        assert!(urls.iter().any(|url| url.contains("country=us")));
        assert!(urls.iter().all(|url| !url.contains("country=es")));
        let notes = notes.lock().unwrap();
        assert!(notes
            .iter()
            .any(|note| note.contains("Spain (ES)") && note.contains("not queried")));
    }

    #[tokio::test]
    async fn phase2_skips_countries_without_a_provider_feed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("atlas.db");
        let store = Store::open(&path).unwrap();
        let origins = ["us", "es", "pk", "bm", "mv"]
            .into_iter()
            .map(|country| OriginStat {
                country: country.into(),
                tier: 2,
                temperature: 1.0,
                volume: 1,
                articles: 0,
            })
            .collect();
        let stats = RunStats {
            scored: true,
            origins,
            ..RunStats::default()
        };
        let cursor = Cursor {
            phase: 2,
            leg: "newsapi".into(),
            from: "2026-09-30T00:00:00Z".into(),
            ..Cursor::default()
        };
        store
            .atlas_insert_run(
                "atlas-feeds",
                &serde_json::to_string(&cursor).unwrap(),
                &serde_json::to_string(&stats).unwrap(),
            )
            .unwrap();
        store
            .atlas_set_state("atlas-feeds", "paused", "", false)
            .unwrap();
        drop(store);
        let urls = Arc::new(Mutex::new(Vec::new()));
        let notes = Arc::new(Mutex::new(Vec::new()));
        let log = urls.clone();
        let noted = notes.clone();
        let pause = AtomicBool::new(false);
        let keys = keys();
        run_atlas(
            RunInput {
                db_path: &path,
                pause: &pause,
                keys: &keys,
                user_agent: "Argos test",
                pace: false,
                resume: true,
                feed: &[],
                classifier: None,
                synthesizer: None,
            },
            move |event| {
                if let AtlasEvent::Note(text) = event {
                    noted.lock().unwrap().push(text);
                }
            },
            move |call: HttpCall| {
                let log = log.clone();
                Box::pin(async move {
                    if call.provider == "newsapi" || call.provider == "currents" {
                        log.lock().unwrap().push(call.url.clone());
                    }
                    Ok(reply_for(&call))
                })
            },
        )
        .await
        .unwrap();
        let urls = urls.lock().unwrap();
        let newsapi: Vec<_> = urls
            .iter()
            .filter(|url| url.contains("newsapi.org"))
            .collect();
        let currents: Vec<_> = urls
            .iter()
            .filter(|url| url.contains("currentsapi"))
            .collect();
        assert!(newsapi.iter().any(|url| url.contains("country=us")));
        assert!(currents.iter().any(|url| url.contains("country=US")));
        assert!(currents.iter().any(|url| url.contains("country=ES")));
        for country in ["es", "pk", "bm", "mv"] {
            assert!(
                newsapi
                    .iter()
                    .all(|url| !url.contains(&format!("country={country}"))),
                "NewsAPI queried {country}"
            );
        }
        for country in ["PK", "BM", "MV"] {
            assert!(
                currents
                    .iter()
                    .all(|url| !url.contains(&format!("country={country}"))),
                "Currents queried {country}"
            );
        }
        let notes = notes.lock().unwrap();
        let newsapi_note = notes
            .iter()
            .find(|note| note.starts_with("NewsAPI was not queried"))
            .expect("newsapi skip note");
        for name in [
            "Spain (ES)",
            "Pakistan (PK)",
            "Bermuda (BM)",
            "Maldives (MV)",
        ] {
            assert!(newsapi_note.contains(name), "{newsapi_note}");
        }
        let currents_note = notes
            .iter()
            .find(|note| note.starts_with("Currents was not queried"))
            .expect("currents skip note");
        for name in ["Pakistan (PK)", "Bermuda (BM)", "Maldives (MV)"] {
            assert!(currents_note.contains(name), "{currents_note}");
        }
        assert!(!currents_note.contains("Spain"));
    }

    #[tokio::test]
    async fn phase1_reads_latest_headlines_and_phase2_articles_stay_on_the_run() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("atlas.db");
        let pause = AtomicBool::new(false);
        let keys = keys();
        let urls = Arc::new(Mutex::new(Vec::new()));
        let log = urls.clone();
        let stop = run_atlas(
            RunInput {
                db_path: &path,
                pause: &pause,
                keys: &keys,
                user_agent: "Argos test",
                pace: false,
                resume: false,
                feed: &[],
                classifier: None,
                synthesizer: None,
            },
            |_| {},
            move |call: HttpCall| {
                let log = log.clone();
                Box::pin(async move {
                    log.lock().unwrap().push(call.url.clone());
                    Ok(reply_for(&call))
                })
            },
        )
        .await
        .unwrap();
        assert_eq!(stop, Stop::Finished);
        let urls = urls.lock().unwrap();
        assert!(urls
            .iter()
            .any(|url| url.contains("/top-headlines") && !url.contains("q=")));
        assert!(urls.iter().any(|url| {
            url.contains("newsdata.io") && url.contains("/latest") && !url.contains("q=")
        }));
        assert!(
            urls.iter()
                .any(|url| url.contains("newsapi.org") && url.contains("country=")),
            "phase 2 still requests NewsAPI headlines for each scored country"
        );
        assert!(
            urls.iter()
                .any(|url| url.contains("currentsapi") && url.contains("country=")),
            "phase 2 still requests Currents headlines for each scored country"
        );
        drop(urls);
        let store = Store::open(&path).unwrap();
        let run = store.atlas_latest_run().unwrap().unwrap();
        let articles = store.atlas_list_articles(&run.id).unwrap();
        assert!(!articles.is_empty());
        assert!(articles
            .iter()
            .all(|row| row.run_id == run.id && row.category == "unk"));
    }

    #[tokio::test]
    async fn phase2_still_collects_regional_headlines_when_phase1_has_no_code() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("atlas.db");
        let pause = AtomicBool::new(false);
        let keys = keys();
        let urls = Arc::new(Mutex::new(Vec::new()));
        let log = urls.clone();
        let stop = run_atlas(
            RunInput {
                db_path: &path,
                pause: &pause,
                keys: &keys,
                user_agent: "Argos test",
                pace: false,
                resume: false,
                feed: &[],
                classifier: None,
                synthesizer: None,
            },
            |_| {},
            move |call: HttpCall| {
                let log = log.clone();
                Box::pin(async move {
                    log.lock().unwrap().push(call.url.clone());
                    Ok(regional_reply(&call))
                })
            },
        )
        .await
        .unwrap();
        assert_eq!(stop, Stop::Finished);
        let urls = urls.lock().unwrap();
        for country in ["gb", "de"] {
            assert!(
                urls.iter().any(|url| {
                    url.contains("newsapi.org") && url.contains(&format!("country={country}"))
                }),
                "NewsAPI missing {country}: {urls:?}"
            );
            let upper = country.to_ascii_uppercase();
            assert!(
                urls.iter().any(|url| {
                    url.contains("currentsapi") && url.contains(&format!("country={upper}"))
                }),
                "Currents missing {upper}: {urls:?}"
            );
        }
        drop(urls);
        let store = Store::open(&path).unwrap();
        let run = store.atlas_latest_run().unwrap().unwrap();
        let articles = store.atlas_list_articles(&run.id).unwrap();
        let countries: Vec<_> = articles.iter().map(|row| row.country.as_str()).collect();
        assert!(countries.contains(&"gb"), "{countries:?}");
        assert!(countries.contains(&"de"), "{countries:?}");
    }

    fn keys() -> ProviderKeys {
        ProviderKeys {
            gnews: "gnews-key".into(),
            newsdata: "newsdata-key".into(),
            currents: "currents-key".into(),
            newsapi: "newsapi-key".into(),
            ..ProviderKeys::default()
        }
    }

    fn reply_for(call: &HttpCall) -> HttpReply {
        let body = if call.provider == "gnews" || call.provider == "newsdata" {
            let code = ["us", "cn", "de", "fr", "gb", "in", "jp"][call.url.len() % 7];
            if call.provider == "gnews" {
                serde_json::json!({"articles":[{"title":"Story","url":"https://example.com/a","source":{"name":"Wire","country": code}}]}).to_string()
            } else {
                serde_json::json!({"status":"success","results":[{"title":"Story","link":"https://example.com/b","country":[code],"source_name":"Wire"}]}).to_string()
            }
        } else if call.provider == "newsapi" {
            serde_json::json!({"status":"ok","articles":[{"title":"Local","url":"https://www.reuters.com/local","source":{"name":"Reuters"},"publishedAt":"2026-10-01T00:00:00Z"}]}).to_string()
        } else {
            serde_json::json!({"status":"ok","news":[{"title":"Local wire","url":"https://example.com/local","author":"Desk","published":"2026-10-01T01:00:00Z"}]}).to_string()
        };
        HttpReply { status: 200, body }
    }

    /// Top headlines with no `source.country`, and a NewsData row named Germany.
    fn regional_reply(call: &HttpCall) -> HttpReply {
        let body = if call.provider == "gnews" {
            serde_json::json!({"articles":[{
                "title":"Cabinet meets",
                "url":"https://www.reuters.com/world/story",
                "source":{"name":"Reuters","url":"https://www.reuters.com"}
            }]})
            .to_string()
        } else if call.provider == "newsdata" {
            serde_json::json!({"status":"success","results":[{
                "title":"Berlin wire",
                "link":"https://example.com/berlin",
                "country":["Germany"],
                "source_name":"Wire"
            }]})
            .to_string()
        } else {
            let code = url::Url::parse(&call.url)
                .ok()
                .and_then(|url| {
                    url.query_pairs()
                        .find(|(key, _)| key == "country")
                        .map(|(_, value)| value.to_ascii_lowercase())
                })
                .unwrap_or_else(|| "xx".into());
            if call.provider == "newsapi" {
                serde_json::json!({"status":"ok","articles":[{
                    "title": format!("Local {code}"),
                    "url": format!("https://www.reuters.com/{code}"),
                    "source":{"name":"Reuters"},
                    "publishedAt":"2026-10-01T00:00:00Z"
                }]})
                .to_string()
            } else {
                serde_json::json!({"status":"ok","news":[{
                    "title": format!("Desk {code}"),
                    "url": format!("https://example.com/{code}"),
                    "author":"Desk",
                    "published":"2026-10-01T01:00:00Z"
                }]})
                .to_string()
            }
        };
        HttpReply { status: 200, body }
    }
}
