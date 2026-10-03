//! Atlas collects regional headlines in two phases and stores the run's statistics.
//!
//! Article text stays in the session that streamed it. The database keeps the run,
//! its cursor, the country table, and the daily request counts.

use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url;

use crate::osint::atlas_news::{self, Hit};
use crate::osint::ProviderKeys;
use crate::store::{AtlasRunRow, Store};

const QUERY_LIMIT: usize = atlas_news::NEWSDATA_QUERY_LIMIT;

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
    match provider {
        "gnews" => 100,
        "newsdata" => 200,
        "newsapi" => 100,
        "currents" => 250,
        _ => 0,
    }
}

/// Pack quoted phrases into OR queries. Discovery uses NewsData's 100-character cap so the
/// same chunk is accepted by GNews (200) and NewsData (100).
pub fn pack_queries(phrases: &[&str]) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    for phrase in phrases {
        let piece = format!("\"{phrase}\"");
        let next = if current.is_empty() {
            piece.clone()
        } else {
            format!("{current} OR {piece}")
        };
        if next.len() <= QUERY_LIMIT {
            current = next;
        } else {
            if !current.is_empty() {
                chunks.push(current);
            }
            current = piece;
        }
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

pub fn discovery_queries() -> Vec<String> {
    CLUSTERS
        .iter()
        .flat_map(|(_, phrases)| pack_queries(phrases))
        .collect()
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
}

impl Default for Cursor {
    fn default() -> Self {
        Self {
            phase: 1,
            chunk: 0,
            leg: "gnews".into(),
            country: 0,
            from: String::new(),
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
    pub published_at: String,
    pub provider: String,
    pub temperature: f64,
    pub seen_at: String,
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
    }
}

/// `United States (US)`. Unknown codes keep a readable fallback.
pub fn country_label(code: &str) -> String {
    let code = code.trim().to_ascii_lowercase();
    let name = country_name(&code).unwrap_or("Unknown");
    format!("{name} ({})", code.to_ascii_uppercase())
}

pub fn country_name(code: &str) -> Option<&'static str> {
    COUNTRIES
        .iter()
        .find(|(known, _)| *known == code)
        .map(|(_, name)| *name)
}

const COUNTRIES: &[(&str, &str)] = &[
    ("ae", "United Arab Emirates"),
    ("ar", "Argentina"),
    ("at", "Austria"),
    ("au", "Australia"),
    ("bd", "Bangladesh"),
    ("be", "Belgium"),
    ("bg", "Bulgaria"),
    ("br", "Brazil"),
    ("bw", "Botswana"),
    ("ca", "Canada"),
    ("ch", "Switzerland"),
    ("cl", "Chile"),
    ("cn", "China"),
    ("co", "Colombia"),
    ("cu", "Cuba"),
    ("cz", "Czechia"),
    ("de", "Germany"),
    ("ee", "Estonia"),
    ("eg", "Egypt"),
    ("es", "Spain"),
    ("et", "Ethiopia"),
    ("fi", "Finland"),
    ("fr", "France"),
    ("gb", "United Kingdom"),
    ("gh", "Ghana"),
    ("gr", "Greece"),
    ("hk", "Hong Kong"),
    ("hu", "Hungary"),
    ("id", "Indonesia"),
    ("ie", "Ireland"),
    ("il", "Israel"),
    ("in", "India"),
    ("it", "Italy"),
    ("jp", "Japan"),
    ("ke", "Kenya"),
    ("kr", "South Korea"),
    ("lb", "Lebanon"),
    ("lt", "Lithuania"),
    ("lv", "Latvia"),
    ("ma", "Morocco"),
    ("mx", "Mexico"),
    ("my", "Malaysia"),
    ("na", "Namibia"),
    ("ng", "Nigeria"),
    ("nl", "Netherlands"),
    ("no", "Norway"),
    ("nz", "New Zealand"),
    ("pe", "Peru"),
    ("ph", "Philippines"),
    ("pk", "Pakistan"),
    ("pl", "Poland"),
    ("pt", "Portugal"),
    ("ro", "Romania"),
    ("rs", "Serbia"),
    ("ru", "Russia"),
    ("sa", "Saudi Arabia"),
    ("se", "Sweden"),
    ("sg", "Singapore"),
    ("si", "Slovenia"),
    ("sk", "Slovakia"),
    ("sn", "Senegal"),
    ("th", "Thailand"),
    ("tr", "Turkey"),
    ("tw", "Taiwan"),
    ("tz", "Tanzania"),
    ("ua", "Ukraine"),
    ("ug", "Uganda"),
    ("us", "United States"),
    ("ve", "Venezuela"),
    ("vn", "Vietnam"),
    ("za", "South Africa"),
    ("zw", "Zimbabwe"),
];

pub fn format_run_card(run: &AtlasRunRow) -> String {
    let stats = parse_stats(&run.stats_json);
    let mut lines = vec![
        format!("State: {}", run.state),
        format!("Started: {}", run.started_at),
    ];
    if !run.finished_at.is_empty() {
        lines.push(format!("Finished: {}", run.finished_at));
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
    let source = if hit.source_name.is_empty() {
        domain.clone()
    } else {
        hit.source_name.clone()
    };
    FeedArticle {
        id: format!("art-{}", url_hash(&hit.url)),
        title: hit.title.clone(),
        description: hit.description.clone(),
        url: hit.url.clone(),
        country: hit.country.clone(),
        source_name: source,
        source_domain: domain,
        published_at: hit.published_at.clone(),
        provider: provider.into(),
        temperature,
        seen_at: chrono::Utc::now().to_rfc3339(),
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

fn phase1_call(provider: &str, query: &str, from: &str, keys: &ProviderKeys) -> Result<CallSpec> {
    let key = if provider == "gnews" {
        &keys.gnews
    } else {
        &keys.newsdata
    };
    let prepared = atlas_news::prepare_discovery(provider, query, from, key)?;
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

fn phase2_call(provider: &str, country: &str, keys: &ProviderKeys) -> Result<CallSpec> {
    let key = if provider == "newsapi" {
        &keys.newsapi
    } else {
        &keys.currents
    };
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

fn apply_hits(
    hits: &[Hit],
    provider: &str,
    stats: &mut RunStats,
    seen: &mut Vec<Seen>,
    emit: &mut impl FnMut(AtlasEvent),
) {
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
        return;
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
                emit(AtlasEvent::Article(article));
            }
            Verdict::Replace(id) => {
                remember(seen, &article, Some(&id));
                emit(AtlasEvent::Replaced { id, article });
            }
        }
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
    } = input;
    let store = Store::open(db_path)?;
    let queries = discovery_queries();
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
        require_key(&keys.gnews, "GNews API key")?;
        require_key(&keys.newsdata, "NewsData API key")?;
    }
    let mut seen: Vec<Seen> = feed.iter().map(seen_of).collect();
    let mut last_gnews: Option<std::time::Instant> = None;
    let user_agent = if user_agent.trim().is_empty() {
        crate::osint::DEFAULT_USER_AGENT.to_string()
    } else {
        user_agent.to_string()
    };

    let phase1: Vec<Job> = queries
        .iter()
        .enumerate()
        .flat_map(|(chunk, _)| {
            [
                Job {
                    phase: 1,
                    chunk,
                    leg: "gnews",
                    country: 0,
                    provider: "gnews",
                },
                Job {
                    phase: 1,
                    chunk,
                    leg: "newsdata",
                    country: 0,
                    provider: "newsdata",
                },
            ]
        })
        .collect();

    if cursor.phase <= 1 {
        for job in &phase1 {
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
            let query = &queries[job.chunk];
            let spec = phase1_call(job.provider, query, &cursor.from, keys)?;
            if !quota_open(&store, spec.provider)? {
                emit(AtlasEvent::Note(format!(
                    "{} daily quota is spent",
                    spec.provider
                )));
                continue;
            }
            if pace && spec.provider == "gnews" {
                if let Some(last) = last_gnews {
                    let wait = std::time::Duration::from_secs(1).saturating_sub(last.elapsed());
                    if !wait.is_zero() {
                        tokio::time::sleep(wait).await;
                    }
                }
            }
            emit(AtlasEvent::Status(format!(
                "Discovery {}/{} · {}",
                job.chunk + 1,
                queries.len(),
                spec.provider
            )));
            match dispatch(&mut fetch, spec, &user_agent).await {
                Ok(body) => {
                    if job.provider == "gnews" {
                        last_gnews = Some(std::time::Instant::now());
                    }
                    match atlas_news::discovery_hits(job.provider, &body) {
                        Ok(hits) => {
                            apply_hits(&hits, job.provider, &mut stats, &mut seen, &mut emit)
                        }
                        Err(err) => emit(parse_fault(job.provider, err.to_string(), &body)),
                    }
                }
                Err(fault) => emit(AtlasEvent::Fault(fault)),
            }
            let _ = store.atlas_quota_bump(job.provider, &utc_day())?;
            let next = next_after_phase1(job, &queries);
            save(&store, &run_id, &cursor_at(&next, &cursor.from), &stats)?;
            emit(AtlasEvent::Stats(stats.clone()));
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
        };
        save(&store, &run_id, &cursor, &stats)?;
        emit(AtlasEvent::Stats(stats.clone()));
    }

    require_key(&keys.newsapi, "NewsAPI key")?;
    require_key(&keys.currents, "Currents API key")?;
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
        if job.provider == "newsapi" && !atlas_news::newsapi_country_supported(country) {
            newsapi_skipped.push(country.clone());
            continue;
        }
        if !quota_open(&store, job.provider)? {
            emit(AtlasEvent::Note(format!(
                "{} daily quota is spent",
                job.provider
            )));
            continue;
        }
        let spec = phase2_call(job.provider, country, keys)?;
        emit(AtlasEvent::Status(format!(
            "Headlines {} · {}",
            country.to_ascii_uppercase(),
            spec.provider
        )));
        match dispatch(&mut fetch, spec, &user_agent).await {
            Ok(body) => match atlas_news::headline_hits(job.provider, country, &body) {
                Ok(hits) => apply_hits(&hits, job.provider, &mut stats, &mut seen, &mut emit),
                Err(err) => emit(parse_fault(job.provider, err.to_string(), &body)),
            },
            Err(fault) => emit(AtlasEvent::Fault(fault)),
        }
        let _ = store.atlas_quota_bump(job.provider, &utc_day())?;
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
    store.atlas_set_state(&run_id, "completed", "", true)?;
    emit(AtlasEvent::Status("Pipeline complete".into()));
    emit(AtlasEvent::Stats(stats));
    Ok(Stop::Finished)
}

fn next_after_phase1(job: &Job, queries: &[String]) -> Job {
    if job.leg == "gnews" {
        Job {
            leg: "newsdata",
            ..job.clone()
        }
    } else if job.chunk + 1 < queries.len() {
        Job {
            chunk: job.chunk + 1,
            leg: "gnews",
            ..job.clone()
        }
    } else {
        Job {
            phase: 2,
            chunk: 0,
            leg: "newsapi",
            country: 0,
            provider: "newsapi",
        }
    }
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
    let _ = store.atlas_quota_bump("newsapi", &utc_day());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn chunks_cover_every_phrase_and_stay_within_100_characters() {
        let queries = discovery_queries();
        assert!(
            queries.len() > CLUSTERS.len(),
            "clusters must be split so every phrase still fits"
        );
        for query in &queries {
            assert!(
                query.len() <= QUERY_LIMIT,
                "{} ({} chars)",
                query,
                query.len()
            );
            assert!(query.starts_with('"'), "{query}");
        }
        for (_, phrases) in CLUSTERS {
            for phrase in *phrases {
                assert!(
                    queries
                        .iter()
                        .any(|query| query.contains(&format!("\"{phrase}\""))),
                    "{phrase}"
                );
            }
        }
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
            published_at: "2026-10-01T00:00:00Z".into(),
            provider: "newsapi".into(),
            temperature: 1.0,
            seen_at: String::new(),
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

    #[test]
    fn country_labels_show_the_name_and_the_code() {
        assert_eq!(country_label("us"), "United States (US)");
        assert_eq!(country_label("CN"), "China (CN)");
        assert!(format_run_card(&crate::store::AtlasRunRow {
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
        })
        .contains("Germany (DE)"));
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
            let code = format!("c{}", call.url.len() % 7);
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
}
