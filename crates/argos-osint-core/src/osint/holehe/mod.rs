//! Native Holehe email-registration lookups. No Python, subprocess, or hosted service.
//!
//! Upstream catalog is pinned at 14da70f (123 modules). This increment ships working
//! Twitter, Spotify, and Pinterest adapters. Other services return `unsupported`.

use anyhow::{anyhow, ensure, Result};
use chrono::Utc;
use futures_util::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;
use url::Url;

pub mod catalog;
pub mod pinterest;
pub mod spotify;
pub mod twitter;

pub const TOOL_ID: &str = "holehe_email_lookup";
pub const DEFAULT_MAX_SITES: u32 = 10;
pub const MAX_SITES_CAP: u32 = 50;
pub const GLOBAL_CONCURRENCY: usize = 4;
pub const PER_HOST_CONCURRENCY: usize = 1;
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
pub const MAX_BODY_BYTES: usize = 1_048_576;
pub const CACHE_TTL: Duration = Duration::from_secs(86_400);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceState {
    Enabled,
    Experimental,
    Unsupported,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckSignal {
    Registered,
    NotRegistered,
    Inconclusive,
    RateLimited,
    Blocked,
    Error,
    Unsupported,
}

impl CheckSignal {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Registered => "registered",
            Self::NotRegistered => "not_registered",
            Self::Inconclusive => "inconclusive",
            Self::RateLimited => "rate_limited",
            Self::Blocked => "blocked",
            Self::Error => "error",
            Self::Unsupported => "unsupported",
        }
    }

    fn definitive(self) -> bool {
        matches!(self, Self::Registered | Self::NotRegistered)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ServiceSpec {
    pub id: &'static str,
    pub category: &'static str,
    pub method: &'static str,
    pub hosts: &'static [&'static str],
    pub adapter_version: &'static str,
    pub state: ServiceState,
    pub fixture_validated: bool,
    pub live_validated: bool,
    pub unsupported_reason: &'static str,
}

impl ServiceSpec {
    pub fn implemented(self) -> bool {
        matches!(
            self.state,
            ServiceState::Enabled | ServiceState::Experimental
        ) && matches!(self.id, twitter::ID | spotify::ID | pinterest::ID)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ServiceResult {
    pub service: String,
    pub signal: CheckSignal,
    pub timestamp: String,
    pub adapter_version: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub diagnostic: String,
}

impl ServiceResult {
    pub fn ok(signal: CheckSignal, diagnostic: impl Into<String>) -> Self {
        Self {
            service: String::new(),
            signal,
            timestamp: Utc::now().to_rfc3339(),
            adapter_version: "1.0".into(),
            diagnostic: diagnostic.into(),
        }
    }

    pub fn inconclusive(diagnostic: impl Into<String>) -> Self {
        Self::ok(CheckSignal::Inconclusive, diagnostic)
    }

    pub fn rate_limited(diagnostic: impl Into<String>) -> Self {
        Self::ok(CheckSignal::RateLimited, diagnostic)
    }

    pub fn blocked(diagnostic: impl Into<String>) -> Self {
        Self::ok(CheckSignal::Blocked, diagnostic)
    }

    pub fn error(diagnostic: impl Into<String>) -> Self {
        Self::ok(CheckSignal::Error, diagnostic)
    }

    pub fn unsupported(service: &str, reason: &str) -> Self {
        Self {
            service: service.to_string(),
            signal: CheckSignal::Unsupported,
            timestamp: Utc::now().to_rfc3339(),
            adapter_version: "1.0".into(),
            diagnostic: reason.to_string(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct LookupCounts {
    pub selected: usize,
    pub omitted: usize,
    pub registered: usize,
    pub not_registered: usize,
    pub inconclusive: usize,
    pub rate_limited: usize,
    pub blocked: usize,
    pub error: usize,
    pub unsupported: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LookupObservation {
    pub email_domain: String,
    pub email_hash: String,
    pub partial: bool,
    pub cancelled: bool,
    pub counts: LookupCounts,
    pub results: Vec<ServiceResult>,
    pub catalog_commit: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct LookupInput {
    pub email: String,
    #[serde(default)]
    pub services: Option<Vec<String>>,
    #[serde(default)]
    pub max_sites: Option<u32>,
}

/// Preserve local-part case; IDNA-normalize the domain.
pub fn normalize_email(raw: &str) -> Result<(String, String)> {
    let value = raw.trim();
    ensure!(
        !value.chars().any(char::is_control) && value.len() <= 254,
        "invalid email"
    );
    let (local, host) = value
        .split_once('@')
        .ok_or_else(|| anyhow!("invalid email"))?;
    ensure!(
        !local.is_empty()
            && local.len() <= 64
            && !local.starts_with('.')
            && !local.ends_with('.')
            && !local.contains(".."),
        "invalid email"
    );
    let domain = super::whoxy::normalize_domain(host).or_else(|_| {
        let ascii = host.trim().trim_end_matches('.').to_ascii_lowercase();
        ensure!(
            ascii.contains('.')
                && ascii.split('.').all(|p| !p.is_empty()
                    && p.len() <= 63
                    && p.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')),
            "invalid email"
        );
        Ok(ascii)
    })?;
    Ok((format!("{local}@{domain}"), domain))
}

fn email_hash(email: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(email.as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub fn parse_json(body: &str) -> Result<Value, String> {
    serde_json::from_str(body).map_err(|_| "malformed JSON".to_string())
}

struct CacheEntry {
    result: ServiceResult,
    expires: Instant,
}

fn result_cache() -> &'static Mutex<HashMap<String, CacheEntry>> {
    static CACHE: OnceLock<Mutex<HashMap<String, CacheEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn host_cooldowns() -> &'static Mutex<HashMap<String, Instant>> {
    static COOLDOWNS: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();
    COOLDOWNS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cache_key(email: &str, service: &str, version: &str) -> String {
    format!("{}:{service}:{version}", email_hash(email))
}

fn cached_result(email: &str, service: &str, version: &str) -> Option<ServiceResult> {
    let mut cache = result_cache().lock().ok()?;
    let key = cache_key(email, service, version);
    let entry = cache.get(&key)?;
    if entry.expires <= Instant::now() {
        cache.remove(&key);
        return None;
    }
    Some(entry.result.clone())
}

fn store_cache(email: &str, service: &str, version: &str, result: &ServiceResult) {
    if !result.signal.definitive() {
        return;
    }
    if let Ok(mut cache) = result_cache().lock() {
        cache.insert(
            cache_key(email, service, version),
            CacheEntry {
                result: result.clone(),
                expires: Instant::now() + CACHE_TTL,
            },
        );
    }
}

#[cfg(test)]
pub fn clear_caches() {
    if let Ok(mut cache) = result_cache().lock() {
        cache.clear();
    }
    if let Ok(mut cool) = host_cooldowns().lock() {
        cool.clear();
    }
}

fn host_allowed(host: &str, allow: &[&str]) -> bool {
    allow.iter().any(|allowed| {
        host.eq_ignore_ascii_case(allowed)
            || host.to_ascii_lowercase().ends_with(&format!(".{allowed}"))
    })
}

fn public_host(url: &Url) -> Result<String> {
    ensure!(url.scheme() == "https", "non-https destination");
    let host = url.host_str().ok_or_else(|| anyhow!("missing host"))?;
    if let Ok(ip) = host.parse::<IpAddr>() {
        match ip {
            IpAddr::V4(v4) if v4.is_loopback() || v4.is_private() || v4.is_link_local() => {
                return Err(anyhow!("private host"));
            }
            IpAddr::V6(v6) if v6.is_loopback() => return Err(anyhow!("private host")),
            _ => {}
        }
    }
    Ok(host.to_ascii_lowercase())
}

async fn read_limited(response: reqwest::Response) -> Result<(u16, String)> {
    let status = response.status().as_u16();
    let bytes = response.bytes().await?;
    ensure!(bytes.len() <= MAX_BODY_BYTES, "response too large");
    let body = String::from_utf8_lossy(&bytes).into_owned();
    Ok((status, body))
}

fn classify_http(service: &str, status: u16, body: &str) -> ServiceResult {
    let mut result = match service {
        twitter::ID => twitter::parse_body(status, body),
        spotify::ID => spotify::parse_body(status, body),
        pinterest::ID => pinterest::parse_body(status, body),
        _ => ServiceResult::unsupported(service, "not implemented in this increment"),
    };
    result.service = service.to_string();
    result
}

async fn fetch_once(
    client: &reqwest::Client,
    url: Url,
    allow: &[&str],
    user_agent: &str,
) -> Result<(u16, String)> {
    let host = public_host(&url)?;
    ensure!(host_allowed(&host, allow), "host is not allowed");
    let response = client
        .get(url)
        .header(reqwest::header::USER_AGENT, user_agent)
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await?;
    if response.status().is_redirection() {
        return Ok((response.status().as_u16(), String::new()));
    }
    read_limited(response).await
}

async fn check_implemented(
    client: &reqwest::Client,
    service: &ServiceSpec,
    email: &str,
    user_agent: &str,
    cancel: &AtomicBool,
) -> ServiceResult {
    if cancel.load(Ordering::Relaxed) {
        return ServiceResult {
            service: service.id.to_string(),
            signal: CheckSignal::Inconclusive,
            timestamp: Utc::now().to_rfc3339(),
            adapter_version: service.adapter_version.to_string(),
            diagnostic: "cancelled".into(),
        };
    }
    if let Some(cached) = cached_result(email, service.id, service.adapter_version) {
        return cached;
    }
    let host = service.hosts.first().copied().unwrap_or("");
    if let Ok(cool) = host_cooldowns().lock() {
        if cool.get(host).is_some_and(|until| *until > Instant::now()) {
            return ServiceResult {
                service: service.id.to_string(),
                signal: CheckSignal::RateLimited,
                timestamp: Utc::now().to_rfc3339(),
                adapter_version: service.adapter_version.to_string(),
                diagnostic: "host cooldown".into(),
            };
        }
    }
    let url = match service.id {
        twitter::ID => twitter::request_url(email),
        spotify::ID => spotify::request_url(email),
        pinterest::ID => pinterest::request_url(email),
        _ => {
            return ServiceResult::unsupported(service.id, service.unsupported_reason);
        }
    };
    let Ok(url) = url else {
        return ServiceResult::error("failed to build request");
    };
    let mut last_err = None;
    for attempt in 0..2 {
        if cancel.load(Ordering::Relaxed) {
            return ServiceResult {
                service: service.id.to_string(),
                signal: CheckSignal::Inconclusive,
                timestamp: Utc::now().to_rfc3339(),
                adapter_version: service.adapter_version.to_string(),
                diagnostic: "cancelled".into(),
            };
        }
        match fetch_once(client, url.clone(), service.hosts, user_agent).await {
            Ok((status, body)) => {
                if status == 429 {
                    if let Ok(mut cool) = host_cooldowns().lock() {
                        cool.insert(host.to_string(), Instant::now() + Duration::from_secs(60));
                    }
                }
                let retryable = status >= 500 || status == 0;
                if retryable && attempt == 0 {
                    last_err = Some(format!("HTTP {status}"));
                    continue;
                }
                let mut result = classify_http(service.id, status, &body);
                result.adapter_version = service.adapter_version.to_string();
                store_cache(email, service.id, service.adapter_version, &result);
                return result;
            }
            Err(err) => {
                last_err = Some(err.to_string());
                if attempt == 0 {
                    continue;
                }
            }
        }
    }
    ServiceResult {
        service: service.id.to_string(),
        signal: CheckSignal::Error,
        timestamp: Utc::now().to_rfc3339(),
        adapter_version: service.adapter_version.to_string(),
        diagnostic: last_err.unwrap_or_else(|| "request failed".into()),
    }
}

pub fn select_services(
    requested: Option<&[String]>,
    max_sites: u32,
) -> Result<(Vec<&'static ServiceSpec>, usize)> {
    let max_sites = max_sites.clamp(1, MAX_SITES_CAP) as usize;
    if let Some(ids) = requested {
        let mut selected = Vec::new();
        let mut seen = HashSet::new();
        for id in ids {
            let id = id.trim();
            if id.is_empty() || !seen.insert(id.to_string()) {
                continue;
            }
            let spec = catalog::by_id(id).ok_or_else(|| anyhow!("unknown service {id}"))?;
            selected.push(spec);
        }
        let omitted = selected.len().saturating_sub(max_sites);
        selected.truncate(max_sites);
        return Ok((selected, omitted));
    }
    let mut selected: Vec<_> = catalog::CATALOG
        .iter()
        .filter(|s| s.state == ServiceState::Enabled && s.implemented())
        .collect();
    if selected.is_empty() {
        selected = catalog::CATALOG
            .iter()
            .filter(|s| s.implemented())
            .collect();
    }
    let omitted = selected.len().saturating_sub(max_sites);
    selected.truncate(max_sites);
    Ok((selected, omitted))
}

pub async fn lookup(
    client: &reqwest::Client,
    input: LookupInput,
    user_agent: Option<&str>,
    cancel: &AtomicBool,
) -> Result<LookupObservation> {
    let (email, domain) = normalize_email(&input.email)?;
    let max_sites = input.max_sites.unwrap_or(DEFAULT_MAX_SITES);
    ensure!(max_sites >= 1, "max_sites must be at least 1");
    let (selected, omitted) = select_services(input.services.as_deref(), max_sites)?;
    let ua = user_agent.unwrap_or(super::DEFAULT_USER_AGENT);
    let global = Arc::new(Semaphore::new(GLOBAL_CONCURRENCY));
    let per_host: Arc<Mutex<HashMap<String, Arc<Semaphore>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let cancel = Arc::new(AtomicBool::new(cancel.load(Ordering::Relaxed)));

    let mut jobs = Vec::with_capacity(selected.len());
    for spec in selected {
        let spec = *spec;
        let client = client.clone();
        let email = email.clone();
        let ua = ua.to_string();
        let global = Arc::clone(&global);
        let per_host = Arc::clone(&per_host);
        let cancel = Arc::clone(&cancel);
        jobs.push(async move {
            if spec.implemented() {
                let host = spec.hosts.first().copied().unwrap_or(spec.id);
                let host_sem = {
                    let mut map = per_host.lock().expect("host semaphore map");
                    map.entry(host.to_string())
                        .or_insert_with(|| Arc::new(Semaphore::new(PER_HOST_CONCURRENCY)))
                        .clone()
                };
                let _g = global.acquire().await.ok();
                let _h = host_sem.acquire().await.ok();
                check_implemented(&client, &spec, &email, &ua, &cancel).await
            } else {
                ServiceResult::unsupported(spec.id, spec.unsupported_reason)
            }
        });
    }

    let mut results: Vec<ServiceResult> = stream::iter(jobs)
        .buffer_unordered(GLOBAL_CONCURRENCY)
        .collect()
        .await;
    results.sort_by(|a, b| a.service.cmp(&b.service));

    let mut counts = LookupCounts {
        selected: results.len(),
        omitted,
        ..LookupCounts::default()
    };
    let mut cancelled = false;
    for result in &results {
        match result.signal {
            CheckSignal::Registered => counts.registered += 1,
            CheckSignal::NotRegistered => counts.not_registered += 1,
            CheckSignal::Inconclusive => {
                counts.inconclusive += 1;
                if result.diagnostic == "cancelled" {
                    cancelled = true;
                }
            }
            CheckSignal::RateLimited => counts.rate_limited += 1,
            CheckSignal::Blocked => counts.blocked += 1,
            CheckSignal::Error => counts.error += 1,
            CheckSignal::Unsupported => counts.unsupported += 1,
        }
    }
    let unresolved = counts.inconclusive
        + counts.rate_limited
        + counts.blocked
        + counts.error
        + counts.unsupported;
    let partial = unresolved > 0 || cancelled;
    Ok(LookupObservation {
        email_domain: domain,
        email_hash: email_hash(&email),
        partial,
        cancelled,
        counts,
        results,
        catalog_commit: catalog::UPSTREAM_COMMIT.into(),
    })
}

pub fn observation_status(obs: &LookupObservation) -> &'static str {
    if obs.cancelled {
        return "cancelled";
    }
    let completed = obs.counts.registered + obs.counts.not_registered;
    if completed == obs.counts.selected
        && obs.counts.selected > 0
        && obs.counts.registered == 0
        && !obs.partial
    {
        return "no_results";
    }
    "completed"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_preserves_local_part() {
        let (email, domain) = normalize_email("Ada.Lovelace@Example.ORG").unwrap();
        assert_eq!(email, "Ada.Lovelace@example.org");
        assert_eq!(domain, "example.org");
        assert!(normalize_email("not-an-email").is_err());
    }

    #[test]
    fn unknown_service_is_input_error() {
        let err = select_services(Some(&["not_a_real_site".into()]), 10).unwrap_err();
        assert!(err.to_string().contains("unknown service"));
    }

    #[test]
    fn unsupported_known_ids_are_selected() {
        let (selected, _) =
            select_services(Some(&["instagram".into(), "twitter".into()]), 10).unwrap();
        assert_eq!(selected.len(), 2);
        assert!(!selected[0].implemented());
        assert!(selected[1].implemented());
    }

    #[test]
    fn cap_reports_omitted() {
        let ids: Vec<String> = catalog::CATALOG
            .iter()
            .take(5)
            .map(|s| s.id.into())
            .collect();
        let (selected, omitted) = select_services(Some(&ids), 2).unwrap();
        assert_eq!(selected.len(), 2);
        assert_eq!(omitted, 3);
    }

    #[test]
    fn default_selection_uses_implemented_adapters() {
        let (selected, _) = select_services(None, 10).unwrap();
        assert_eq!(selected.len(), 3);
        let ids: Vec<_> = selected.iter().map(|s| s.id).collect();
        assert!(ids.contains(&"twitter"));
        assert!(ids.contains(&"spotify"));
        assert!(ids.contains(&"pinterest"));
    }

    #[test]
    fn mixed_failure_is_partial_not_absence() {
        let obs = LookupObservation {
            email_domain: "example.org".into(),
            email_hash: "ab".into(),
            partial: true,
            cancelled: false,
            counts: LookupCounts {
                selected: 2,
                not_registered: 1,
                error: 1,
                ..LookupCounts::default()
            },
            results: vec![],
            catalog_commit: catalog::UPSTREAM_COMMIT.into(),
        };
        assert_eq!(observation_status(&obs), "completed");
        assert!(obs.partial);
    }

    #[test]
    fn all_negative_is_no_results() {
        let obs = LookupObservation {
            email_domain: "example.org".into(),
            email_hash: "ab".into(),
            partial: false,
            cancelled: false,
            counts: LookupCounts {
                selected: 2,
                not_registered: 2,
                ..LookupCounts::default()
            },
            results: vec![],
            catalog_commit: catalog::UPSTREAM_COMMIT.into(),
        };
        assert_eq!(observation_status(&obs), "no_results");
    }

    #[test]
    fn cache_stores_only_definitive() {
        clear_caches();
        let email = "user@example.org";
        let registered = ServiceResult {
            service: "twitter".into(),
            signal: CheckSignal::Registered,
            timestamp: Utc::now().to_rfc3339(),
            adapter_version: "1.0".into(),
            diagnostic: "taken=true".into(),
        };
        store_cache(email, "twitter", "1.0", &registered);
        assert_eq!(
            cached_result(email, "twitter", "1.0").unwrap().signal,
            CheckSignal::Registered
        );
        let failed = ServiceResult {
            service: "twitter".into(),
            signal: CheckSignal::Error,
            timestamp: Utc::now().to_rfc3339(),
            adapter_version: "1.0".into(),
            diagnostic: "HTTP 500".into(),
        };
        // Errors must not overwrite a definitive hit as a negative.
        store_cache(email, "spotify", "1.0", &failed);
        assert!(cached_result(email, "spotify", "1.0").is_none());
    }

    #[tokio::test]
    async fn host_cooldown_is_rate_limited() {
        clear_caches();
        {
            let mut cool = host_cooldowns().lock().unwrap();
            cool.insert(
                twitter::HOST.to_string(),
                Instant::now() + Duration::from_secs(60),
            );
        }
        let client = reqwest::Client::new();
        let spec = catalog::by_id("twitter").unwrap();
        let result = check_implemented(
            &client,
            spec,
            "ada@example.org",
            "Argos OSINT/0.1 (test@example.org)",
            &AtomicBool::new(false),
        )
        .await;
        assert_eq!(result.signal, CheckSignal::RateLimited);
        clear_caches();
    }

    #[tokio::test]
    async fn cancel_marks_inconclusive() {
        let client = reqwest::Client::new();
        let spec = catalog::by_id("twitter").unwrap();
        let result = check_implemented(
            &client,
            spec,
            "ada@example.org",
            "Argos OSINT/0.1 (test@example.org)",
            &AtomicBool::new(true),
        )
        .await;
        assert_eq!(result.signal, CheckSignal::Inconclusive);
        assert_eq!(result.diagnostic, "cancelled");
    }
}
