//! English Wikipedia WP:RSP (Perennial sources) via MediaWiki `action=parse`.
//! Maps community reliability consensus onto Admiralty Source Reliability (A–F).

use super::source_eval::SourceReliability;
use anyhow::{anyhow, Result};
use futures_util::{stream, StreamExt, TryStreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

pub const APP_STATE_KEY: &str = "wikipedia_rsp_index";
pub const CACHE_TTL: Duration = Duration::from_secs(30 * 86_400);
pub const TOOL_ID: &str = "wikipedia_source_reliability";
pub const RSP_PAGES: &[&str] = &[
    "Wikipedia:Reliable_sources/Perennial_sources/1",
    "Wikipedia:Reliable_sources/Perennial_sources/2",
    "Wikipedia:Reliable_sources/Perennial_sources/3",
    "Wikipedia:Reliable_sources/Perennial_sources/4",
    "Wikipedia:Reliable_sources/Perennial_sources/5",
    "Wikipedia:Reliable_sources/Perennial_sources/6",
    "Wikipedia:Reliable_sources/Perennial_sources/7",
    "Wikipedia:Reliable_sources/Perennial_sources/8",
    "Wikipedia:Reliable_sources/Perennial_sources/X",
];
const API: &str = "https://en.wikipedia.org/w/api.php";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RspStatus {
    GenerallyReliable,
    NoConsensus,
    GenerallyUnreliable,
    Deprecated,
    Blacklisted,
}

impl RspStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::GenerallyReliable => "gr",
            Self::NoConsensus => "nc",
            Self::GenerallyUnreliable => "gu",
            Self::Deprecated => "d",
            Self::Blacklisted => "b",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::GenerallyReliable => "generally reliable",
            Self::NoConsensus => "no consensus",
            Self::GenerallyUnreliable => "generally unreliable",
            Self::Deprecated => "deprecated",
            Self::Blacklisted => "blacklisted",
        }
    }

    pub fn reliability(self) -> SourceReliability {
        match self {
            // RSP "generally reliable" is not "completely reliable".
            Self::GenerallyReliable => SourceReliability::B,
            Self::NoConsensus => SourceReliability::C,
            Self::GenerallyUnreliable => SourceReliability::D,
            Self::Deprecated | Self::Blacklisted => SourceReliability::E,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RspEntry {
    pub name: String,
    pub domains: Vec<String>,
    pub status: RspStatus,
    pub blacklisted: bool,
    pub last_year: String,
    pub summary: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RspIndex {
    pub entries: Vec<RspEntry>,
    pub by_domain: HashMap<String, usize>,
    pub fetched_at: String,
}

impl RspIndex {
    pub fn build(entries: Vec<RspEntry>, fetched_at: String) -> Self {
        let mut by_domain = HashMap::new();
        for (index, entry) in entries.iter().enumerate() {
            for domain in &entry.domains {
                by_domain.insert(normalize_host(domain), index);
            }
        }
        Self {
            entries,
            by_domain,
            fetched_at,
        }
    }

    pub fn lookup_domain(&self, host: &str) -> Option<&RspEntry> {
        let host = normalize_host(host);
        if host.is_empty() {
            return None;
        }
        if let Some(index) = self.by_domain.get(&host) {
            return self.entries.get(*index);
        }
        // Prefer the longest matching suffix (bbc.co.uk over co.uk).
        let mut best: Option<(usize, usize)> = None;
        for (domain, index) in &self.by_domain {
            if host == *domain || host.ends_with(&format!(".{domain}")) {
                let rank = domain.len();
                if best.is_none_or(|(prev, _)| rank > prev) {
                    best = Some((rank, *index));
                }
            }
        }
        best.and_then(|(_, index)| self.entries.get(index))
    }

    pub fn lookup_publisher(&self, name: &str) -> Option<&RspEntry> {
        let needle = normalize_name(name);
        if needle.is_empty() {
            return None;
        }
        self.entries.iter().find(|entry| {
            let entry_name = normalize_name(&entry.name);
            entry_name == needle
        })
    }

    pub fn reliability_for_domain(&self, host: &str) -> (SourceReliability, Option<&RspEntry>) {
        match self.lookup_domain(host) {
            Some(entry) => (entry.status.reliability(), Some(entry)),
            None => (SourceReliability::F, None),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SourceReliabilityObservation {
    pub domain: String,
    pub publisher: String,
    pub reliability: String,
    pub reliability_label: String,
    pub rsp_status: String,
    pub rsp_status_label: String,
    pub blacklisted: bool,
    pub last_year: String,
    pub summary: String,
    pub rsp_url: String,
    pub listed: bool,
}

impl SourceReliabilityObservation {
    pub fn from_lookup(domain: &str, publisher: &str, entry: Option<&RspEntry>) -> Self {
        match entry {
            Some(entry) => {
                let reliability = entry.status.reliability();
                Self {
                    domain: domain.to_string(),
                    publisher: if publisher.is_empty() {
                        entry.name.clone()
                    } else {
                        publisher.to_string()
                    },
                    reliability: reliability.as_str().into(),
                    reliability_label: reliability.label().into(),
                    rsp_status: entry.status.as_str().into(),
                    rsp_status_label: entry.status.label().into(),
                    blacklisted: entry.blacklisted,
                    last_year: entry.last_year.clone(),
                    summary: clip_summary(&entry.summary, 280),
                    rsp_url: format!(
                        "https://en.wikipedia.org/wiki/Wikipedia:Reliable_sources/Perennial_sources#{}",
                        urlencoding_fragment(&entry.name)
                    ),
                    listed: true,
                }
            }
            None => Self {
                domain: domain.to_string(),
                publisher: publisher.to_string(),
                reliability: SourceReliability::F.as_str().into(),
                reliability_label: SourceReliability::F.label().into(),
                rsp_status: String::new(),
                rsp_status_label: "not in RSP".into(),
                blacklisted: false,
                last_year: String::new(),
                summary: "Source is not listed on English Wikipedia WP:RSP.".into(),
                rsp_url:
                    "https://en.wikipedia.org/wiki/Wikipedia:Reliable_sources/Perennial_sources"
                        .into(),
                listed: false,
            },
        }
    }
}

struct CachedIndex {
    index: RspIndex,
}

fn cache() -> &'static Mutex<Option<CachedIndex>> {
    static CACHE: OnceLock<Mutex<Option<CachedIndex>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

/// Install an index into the process cache (tests and store warm path).
pub fn install_index(index: RspIndex) {
    if let Ok(mut guard) = cache().lock() {
        *guard = Some(CachedIndex { index });
    }
}

/// Process-cached index when still within TTL.
pub fn cached_index() -> Option<RspIndex> {
    let guard = cache().lock().ok()?;
    let cached = guard.as_ref()?;
    if !index_is_fresh(&cached.index) {
        return None;
    }
    Some(cached.index.clone())
}

pub fn index_to_json(index: &RspIndex) -> Result<String> {
    Ok(serde_json::to_string(index)?)
}

pub fn index_from_json(raw: &str) -> Result<RspIndex> {
    Ok(serde_json::from_str(raw)?)
}

/// Parse one RSP letter-page wikitext into entries.
pub fn parse_rsp_wikitext(wikitext: &str) -> Vec<RspEntry> {
    let mut entries = Vec::new();
    for chunk in wikitext.split("|-").skip(1) {
        let chunk = chunk.trim();
        if !chunk.contains("{{WP:RSPSTATUS") && !chunk.contains("{{WP:RSPSTATUS|") {
            continue;
        }
        let Some(status) = parse_status(chunk) else {
            continue;
        };
        let blacklisted = chunk.contains("|b=y") || chunk.contains("|b = y");
        let status = if blacklisted {
            RspStatus::Blacklisted
        } else {
            status
        };
        let domains = parse_uses(chunk);
        let name = parse_source_name(chunk)
            .unwrap_or_else(|| domains.first().cloned().unwrap_or_else(|| "unknown".into()));
        let last_year = parse_last_year(chunk);
        let summary = parse_summary(chunk);
        if domains.is_empty() && name == "unknown" {
            continue;
        }
        entries.push(RspEntry {
            name,
            domains,
            status,
            blacklisted: matches!(status, RspStatus::Blacklisted) || blacklisted,
            last_year,
            summary,
        });
    }
    entries
}

fn parse_status(chunk: &str) -> Option<RspStatus> {
    let start = chunk.find("{{WP:RSPSTATUS|")?;
    let rest = &chunk[start + "{{WP:RSPSTATUS|".len()..];
    let end = rest.find("}}")?;
    let params = &rest[..end];
    let code = params
        .split('|')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    match code.as_str() {
        "gr" => Some(RspStatus::GenerallyReliable),
        "nc" | "m" => Some(RspStatus::NoConsensus),
        "gu" => Some(RspStatus::GenerallyUnreliable),
        "d" => Some(RspStatus::Deprecated),
        _ => None,
    }
}

fn parse_uses(chunk: &str) -> Vec<String> {
    let Some(start) = chunk.find("{{WP:RSPUSES|") else {
        return Vec::new();
    };
    let rest = &chunk[start + "{{WP:RSPUSES|".len()..];
    let end = rest.find("}}").unwrap_or(rest.len());
    rest[..end]
        .split('|')
        .map(str::trim)
        .filter(|part| !part.is_empty() && *part != "—" && *part != "-")
        .map(|part| {
            normalize_host(
                part.trim_start_matches("https://")
                    .trim_start_matches("http://"),
            )
        })
        .filter(|host| host.contains('.'))
        .collect()
}

fn parse_source_name(chunk: &str) -> Option<String> {
    // First cell after the row marker often looks like: | [[ABC News (United States)|ABC News (USA)]]
    for line in chunk.lines() {
        let line = line.trim();
        if !line.starts_with('|')
            || line.starts_with("|{{")
            || line.starts_with("| ") && line.contains("WP:RSP")
        {
            continue;
        }
        let cell = line.trim_start_matches('|').trim();
        if cell.is_empty() || cell.starts_with('{') || cell.starts_with("class=") {
            continue;
        }
        let name = strip_wiki_link(cell);
        if !name.is_empty() {
            return Some(name);
        }
    }
    None
}

fn strip_wiki_link(raw: &str) -> String {
    let mut text = raw.to_string();
    if let Some(start) = text.find("[[") {
        if let Some(end) = text.find("]]") {
            let inner = &text[start + 2..end];
            text = if let Some((_, label)) = inner.split_once('|') {
                label.to_string()
            } else {
                inner.to_string()
            };
        }
    }
    text = text
        .replace("'''", "")
        .replace("''", "")
        .replace("&nbsp;", " ");
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn parse_last_year(chunk: &str) -> String {
    let Some(start) = chunk.find("{{WP:RSPLAST|") else {
        return String::new();
    };
    let rest = &chunk[start + "{{WP:RSPLAST|".len()..];
    let end = rest.find("}}").unwrap_or(rest.len());
    rest[..end]
        .split('|')
        .next()
        .unwrap_or("")
        .chars()
        .take(4)
        .collect()
}

fn parse_summary(chunk: &str) -> String {
    let mut cells = Vec::new();
    for line in chunk.lines() {
        let line = line.trim();
        if line.starts_with('|') && !line.starts_with("|+") && !line.starts_with("|}") {
            cells.push(line.trim_start_matches('|').trim());
        }
    }
    // Summary is typically the last prose cell before USES.
    for cell in cells.iter().rev() {
        if cell.contains("{{WP:RSPUSES")
            || cell.contains("{{WP:RSPSTATUS")
            || cell.contains("{{WP:RSPLAST")
        {
            continue;
        }
        if cell.contains("{{rsnl") || cell.contains("[[WP:Reliable") {
            continue;
        }
        let text = strip_wiki_link(cell);
        if text.len() > 40 {
            return text;
        }
    }
    String::new()
}

pub fn normalize_host(raw: &str) -> String {
    let raw = raw.trim();
    let candidate = if raw.contains("://") {
        raw.to_owned()
    } else {
        format!("https://{raw}")
    };
    let Ok(url) = reqwest::Url::parse(&candidate) else {
        return String::new();
    };
    if !matches!(url.scheme(), "http" | "https") {
        return String::new();
    }
    url.host_str()
        .unwrap_or("")
        .trim_start_matches("www.")
        .trim_end_matches('.')
        .to_ascii_lowercase()
}

/// Durable freshness uses acquisition time, never process installation time.
pub fn index_is_fresh(index: &RspIndex) -> bool {
    chrono::DateTime::parse_from_rfc3339(&index.fetched_at)
        .ok()
        .is_some_and(|at| {
            let age = chrono::Utc::now().signed_duration_since(at);
            age.num_seconds() >= 0 && age.num_seconds() < CACHE_TTL.as_secs() as i64
        })
}
pub fn last_good_index() -> Option<RspIndex> {
    cache().lock().ok()?.as_ref().map(|c| c.index.clone())
}
static REFRESHING: AtomicBool = AtomicBool::new(false);
fn refresh_in_background(user_agent: &str, _db_path: Option<std::path::PathBuf>) {
    if REFRESHING
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }
    let agent = user_agent.to_owned();
    tokio::spawn(async move {
        if let Ok(index) = fetch_index(&agent).await {
            {
                // SurrealDB: the index lives in the repository's `app_state`;
                // The legacy store is never opened.
                #[cfg(feature = "surreal")]
                if let (Ok(repo), Ok(json)) = (
                    crate::storage::runtime::repository().await,
                    index_to_json(&index),
                ) {
                    let _ =
                        crate::storage::pipeline_inputs::app_state_set(&repo, APP_STATE_KEY, &json)
                            .await;
                }
            }
        }
        REFRESHING.store(false, Ordering::Release);
    });
}

fn normalize_name(raw: &str) -> String {
    raw.chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(|ch| ch.to_lowercase())
        .collect()
}

fn clip_summary(text: &str, max: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max {
        return trimmed.to_string();
    }
    let mut out: String = trimmed.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn urlencoding_fragment(name: &str) -> String {
    name.chars()
        .map(|ch| match ch {
            ' ' => "_".to_string(),
            c if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' => c.to_string(),
            c => format!("%{:02X}", c as u32),
        })
        .collect()
}

/// Resolve domain/url/publisher inputs for the catalog tool.
pub fn resolve_lookup_inputs(inputs: &Value) -> Result<(String, String)> {
    if let Some(domain) = inputs.get("domain").and_then(Value::as_str) {
        let host = normalize_host(domain);
        anyhow::ensure!(!host.is_empty(), "domain required");
        return Ok((host, String::new()));
    }
    if let Some(url) = inputs.get("url").and_then(Value::as_str) {
        let host = normalize_host(url);
        anyhow::ensure!(!host.is_empty(), "url host required");
        return Ok((host, String::new()));
    }
    if let Some(publisher) = inputs.get("publisher").and_then(Value::as_str) {
        let publisher = publisher.trim();
        anyhow::ensure!(!publisher.is_empty(), "publisher required");
        return Ok((String::new(), publisher.to_string()));
    }
    Err(anyhow!("domain, url, or publisher required"))
}

pub fn observation_for(
    index: &RspIndex,
    domain: &str,
    publisher: &str,
) -> SourceReliabilityObservation {
    let entry = if !domain.is_empty() {
        index.lookup_domain(domain)
    } else {
        index.lookup_publisher(publisher)
    };
    let domain = if domain.is_empty() {
        entry
            .and_then(|item| item.domains.first())
            .cloned()
            .unwrap_or_default()
    } else {
        domain.to_string()
    };
    SourceReliabilityObservation::from_lookup(&domain, publisher, entry)
}

/// Fetch all RSP letter pages and build an index.
pub async fn fetch_index(user_agent: &str) -> Result<RspIndex> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .user_agent(user_agent)
        .build()?;
    let work = async {
        let pages: Vec<Vec<RspEntry>> =
            stream::iter(RSP_PAGES.iter().map(|s| s.to_string()).collect::<Vec<_>>())
                .map(|page| {
                    let client = client.clone();
                    async move {
                        let value: Value = client
                            .get(API)
                            .query(&[
                                ("action", "parse"),
                                ("page", page.as_str()),
                                ("prop", "wikitext"),
                                ("formatversion", "2"),
                                ("format", "json"),
                            ])
                            .send()
                            .await?
                            .error_for_status()?
                            .json()
                            .await?;
                        let text = value
                            .pointer("/parse/wikitext")
                            .and_then(Value::as_str)
                            .ok_or_else(|| anyhow!("RSP parse missing wikitext for {page}"))?;
                        Ok::<_, anyhow::Error>(parse_rsp_wikitext(text))
                    }
                })
                .buffer_unordered(3)
                .try_collect()
                .await?;
        let entries: Vec<_> = pages.into_iter().flatten().collect();
        anyhow::ensure!(!entries.is_empty(), "RSP index parsed zero entries");
        Ok::<_, anyhow::Error>(RspIndex::build(entries, chrono::Utc::now().to_rfc3339()))
    };
    let index = tokio::time::timeout(Duration::from_secs(60), work)
        .await
        .map_err(|_| anyhow!("RSP refresh exceeded 60 seconds"))??;
    install_index(index.clone());
    Ok(index)
}

pub async fn ensure_index(user_agent: &str) -> Result<RspIndex> {
    warm_with_store_json(None, user_agent)
        .await
        .map(|(index, _)| index)
}
/// Last complete data remains usable and explicitly stale during a single refresh.
pub async fn warm_with_store_json(
    stored: Option<&str>,
    user_agent: &str,
) -> Result<(RspIndex, bool)> {
    warm_with_store_json_for_db(stored, user_agent, None).await
}
pub async fn warm_with_store_json_for_db(
    stored: Option<&str>,
    user_agent: &str,
    db_path: Option<std::path::PathBuf>,
) -> Result<(RspIndex, bool)> {
    if let Some(index) = cached_index() {
        return Ok((index, false));
    }
    if let Some(index) = stored
        .and_then(|s| index_from_json(s).ok())
        .filter(|i| !i.entries.is_empty())
    {
        install_index(index.clone());
        if !index_is_fresh(&index) {
            refresh_in_background(user_agent, db_path.clone())
        }
        return Ok((index, false));
    }
    if let Some(index) = last_good_index() {
        refresh_in_background(user_agent, db_path.clone());
        return Ok((index, false));
    }
    // Serialize a cold refresh across callers as well as the stale refresh path.
    static COLD: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _guard = COLD.lock().await;
    if let Some(index) = cached_index() {
        return Ok((index, false));
    }
    let index = fetch_index(user_agent).await?;
    Ok((index, true))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durable_ttl_and_host_spoofing_are_checked() {
        let fresh = RspIndex::build(vec![], chrono::Utc::now().to_rfc3339());
        assert!(index_is_fresh(&fresh));
        let expired = RspIndex::build(
            vec![],
            (chrono::Utc::now() - chrono::Duration::days(31)).to_rfc3339(),
        );
        assert!(!index_is_fresh(&expired));
        assert!(!index_is_fresh(&RspIndex::build(vec![], "invalid".into())));
        assert_eq!(
            normalize_host("https://bbc.co.uk@evil.example/story"),
            "evil.example"
        );
        assert_eq!(normalize_host("https://BBC.CO.UK:443/story"), "bbc.co.uk");
        let index = RspIndex::build(
            vec![RspEntry {
                name: "BBC".into(),
                domains: vec!["bbc.co.uk".into()],
                status: RspStatus::GenerallyReliable,
                blacklisted: false,
                last_year: String::new(),
                summary: String::new(),
            }],
            fresh.fetched_at,
        );
        assert!(index.lookup_domain("bbc.co.uk.evil.example").is_none());
        assert!(index.lookup_domain("bbc.co.ukevil.example").is_none());
        assert!(index.lookup_domain("news.bbc.co.uk").is_some());
        assert!(index.lookup_publisher("BBC impersonator").is_none());
    }

    const SAMPLE: &str = r#"
{| class="wikitable"
|- class="s-gu" id="112 Ukraine"
| [[112 Ukraine]]
| {{WP:RSPSTATUS|gu}}
| [[WP:Reliable sources/Noticeboard/Archive 281#x|1]]
| {{WP:RSPLAST|2020|stale=n}}
| 112 Ukraine was deprecated following a 2019 RfC about Russian disinformation.
| {{WP:RSPUSES|112.ua|112.international}}
|- class="s-gr" id="ABC News"
| [[ABC News (United States)|ABC News (USA)]]
| {{WP:RSPSTATUS|gr}}
| [[WP:Reliable sources/Noticeboard/Archive 318#ABC|1]]
| {{WP:RSPLAST|2021}}
| There is consensus that ABC News is generally reliable for news reporting.
| {{WP:RSPUSES|abcnews.com|abcnews.go.com}}
|- class="s-nc" id="Example"
| [[Example News]]
| {{WP:RSPSTATUS|nc}}
| [[WP:x|1]]
| {{WP:RSPLAST|2019}}
| Editors have not reached a clear consensus about Example News reliability overall.
| {{WP:RSPUSES|example-news.test}}
|- class="s-d" id="Bad"
| [[Bad Outlet]]
| {{WP:RSPSTATUS|d}}
| [[WP:x|1]]
| {{WP:RSPLAST|2018}}
| Bad Outlet was deprecated after repeated fabrication findings by independent outlets.
| {{WP:RSPUSES|bad-outlet.test}}
|- class="s-gu" id="Spam"
| [[Spam Site]]
| {{WP:RSPSTATUS|gu|b=y}}
| [[WP:x|1]]
| {{WP:RSPLAST|2017}}
| Spam Site is blacklisted for persistent spam and deceptive publication practices.
| {{WP:RSPUSES|spam-site.test}}
|}
"#;

    #[test]
    fn parses_status_domains_and_maps_reliability() {
        let entries = parse_rsp_wikitext(SAMPLE);
        assert_eq!(entries.len(), 5);
        let index = RspIndex::build(entries, "t".into());
        let abc = index.lookup_domain("abcnews.go.com").unwrap();
        assert_eq!(abc.status, RspStatus::GenerallyReliable);
        assert_eq!(abc.status.reliability(), SourceReliability::B);
        assert_eq!(abc.last_year, "2021");
        let gu = index.lookup_domain("112.ua").unwrap();
        assert_eq!(gu.status.reliability(), SourceReliability::D);
        let nc = index.lookup_domain("example-news.test").unwrap();
        assert_eq!(nc.status.reliability(), SourceReliability::C);
        let dep = index.lookup_domain("bad-outlet.test").unwrap();
        assert_eq!(dep.status.reliability(), SourceReliability::E);
        let banned = index.lookup_domain("spam-site.test").unwrap();
        assert_eq!(banned.status, RspStatus::Blacklisted);
        assert_eq!(banned.status.reliability(), SourceReliability::E);
        assert!(index.lookup_domain("not-listed.example").is_none());
        assert_eq!(
            index.reliability_for_domain("missing.test").0,
            SourceReliability::F
        );
    }

    #[test]
    fn observation_marks_unlisted() {
        let index = RspIndex::build(parse_rsp_wikitext(SAMPLE), "t".into());
        let obs = observation_for(&index, "unknown.example", "");
        assert!(!obs.listed);
        assert_eq!(obs.reliability, "F");
    }
}
