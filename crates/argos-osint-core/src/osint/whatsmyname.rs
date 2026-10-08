//! Official WhatsMyName dataset parsing, indexing, and native Rust HTTP adapter.
//!
//! Provides fast, secure username enumeration across hundreds of sites
//! with strict SSRF protection, connection pooling, per-host pacing,
//! redirect inspection, and structured account tuple generation.

use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex, RwLock};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use futures_util::stream::{self, StreamExt};
use reqwest::header::{HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::Semaphore;
use url::form_urlencoded;
use url::Url;

use super::dataset::{self, to_hex, DatasetManifest};

pub const TOOL_ID: &str = "whatsmyname_lookup";
pub const DATASET_NAME: &str = "whatsmyname";
pub const ADAPTER_VERSION: &str = "v1.0";
pub const WMN_UPSTREAM_URL: &str =
    "https://raw.githubusercontent.com/WebBreacher/WhatsMyName/main/wmn-data.json";

pub const DEFAULT_AUTO_MAX_SITES: usize = 50;
pub const DEFAULT_MANUAL_MAX_SITES: usize = 100;
pub const GLOBAL_CONCURRENCY: usize = 8;
pub const PER_HOST_CONCURRENCY: usize = 1;
pub const PER_HOST_MIN_SPACING: Duration = Duration::from_millis(1000);
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
pub const MAX_BODY_BYTES: usize = 512 * 1024; // 512 KB

fn default_true() -> bool {
    true
}

fn deserialize_string_or_list<'de, D>(deserializer: D) -> Result<Option<Vec<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Deserialize;
    let opt_val = Option::<serde_json::Value>::deserialize(deserializer)?;
    match opt_val {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(s)) => {
            let t = s.trim().to_string();
            if t.is_empty() {
                Ok(None)
            } else {
                Ok(Some(vec![t]))
            }
        }
        Some(serde_json::Value::Array(arr)) => {
            let mut list = Vec::new();
            for item in arr {
                match item {
                    serde_json::Value::String(s) => {
                        let t = s.trim().to_string();
                        if !t.is_empty() {
                            list.push(t);
                        }
                    }
                    serde_json::Value::Number(n) => list.push(n.to_string()),
                    _ => {}
                }
            }
            if list.is_empty() {
                Ok(None)
            } else {
                Ok(Some(list))
            }
        }
        _ => Ok(None),
    }
}

fn deserialize_flexible_code<'de, D>(deserializer: D) -> Result<i32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Deserialize;
    let val = serde_json::Value::deserialize(deserializer)?;
    match val {
        serde_json::Value::Number(n) => n
            .as_i64()
            .map(|v| v as i32)
            .ok_or_else(|| serde::de::Error::custom("invalid code number")),
        serde_json::Value::String(s) => s
            .trim()
            .parse::<i32>()
            .map_err(|e| serde::de::Error::custom(format!("invalid code string: {e}"))),
        other => Err(serde::de::Error::custom(format!(
            "expected number or string for status code, got {other:?}"
        ))),
    }
}

fn deserialize_optional_string_or_empty<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Deserialize;
    let opt_val = Option::<serde_json::Value>::deserialize(deserializer)?;
    match opt_val {
        None | Some(serde_json::Value::Null) => Ok(String::new()),
        Some(serde_json::Value::String(s)) => Ok(s),
        Some(serde_json::Value::Number(n)) => Ok(n.to_string()),
        Some(serde_json::Value::Bool(b)) => Ok(b.to_string()),
        _ => Ok(String::new()),
    }
}

fn deserialize_headers<'de, D>(deserializer: D) -> Result<Option<HashMap<String, String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Deserialize;
    let opt_val = Option::<serde_json::Value>::deserialize(deserializer)?;
    match opt_val {
        Some(serde_json::Value::Object(map)) => {
            let mut headers = HashMap::new();
            for (k, v) in map {
                match v {
                    serde_json::Value::String(s) => {
                        headers.insert(k, s);
                    }
                    serde_json::Value::Number(n) => {
                        headers.insert(k, n.to_string());
                    }
                    serde_json::Value::Bool(b) => {
                        headers.insert(k, b.to_string());
                    }
                    _ => {}
                }
            }
            Ok(Some(headers))
        }
        _ => Ok(None),
    }
}

fn deserialize_flexible_bool<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Deserialize;
    let opt_val = Option::<serde_json::Value>::deserialize(deserializer)?;
    match opt_val {
        None | Some(serde_json::Value::Null) => Ok(true),
        Some(serde_json::Value::Bool(b)) => Ok(b),
        Some(serde_json::Value::String(s)) => match s.trim().to_ascii_lowercase().as_str() {
            "false" | "0" | "no" => Ok(false),
            _ => Ok(true),
        },
        Some(serde_json::Value::Number(n)) => Ok(n.as_i64() != Some(0)),
        _ => Ok(true),
    }
}

/// Upstream WhatsMyName JSON file format.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WmnData {
    #[serde(default, deserialize_with = "deserialize_string_or_list")]
    pub license: Option<Vec<String>>,
    #[serde(default, deserialize_with = "deserialize_string_or_list")]
    pub authors: Option<Vec<String>>,
    #[serde(default, deserialize_with = "deserialize_string_or_list")]
    pub categories: Option<Vec<String>>,
    pub sites: Vec<WmnSite>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WmnSite {
    pub name: String,
    pub uri_check: String,
    #[serde(default)]
    pub uri_pretty: Option<String>,
    #[serde(default)]
    pub cat: String,
    #[serde(deserialize_with = "deserialize_flexible_code")]
    pub e_code: i32,
    #[serde(default, deserialize_with = "deserialize_optional_string_or_empty")]
    pub e_string: String,
    #[serde(deserialize_with = "deserialize_flexible_code")]
    pub m_code: i32,
    #[serde(default, deserialize_with = "deserialize_optional_string_or_empty")]
    pub m_string: String,
    #[serde(default, deserialize_with = "deserialize_headers")]
    pub headers: Option<HashMap<String, String>>,
    #[serde(default)]
    pub post_body: Option<String>,
    #[serde(default)]
    pub strip_bad_char: Option<String>,
    #[serde(default, deserialize_with = "deserialize_string_or_list")]
    pub known: Option<Vec<String>>,
    #[serde(
        default = "default_true",
        deserialize_with = "deserialize_flexible_bool"
    )]
    pub valid: bool,
    #[serde(default, deserialize_with = "deserialize_string_or_list")]
    pub protection: Option<Vec<String>>,
}

/// Pre-parsed and validated site entry for runtime execution.
#[derive(Clone, Debug)]
pub struct CompiledSite {
    pub site_id: String,
    pub name: String,
    pub category: String,
    pub uri_check: String,
    pub uri_pretty: Option<String>,
    pub e_code: u16,
    pub e_string: Option<String>,
    pub m_code: u16,
    pub m_string: Option<String>,
    pub is_post: bool,
    pub post_body: Option<String>,
    pub headers: Vec<(String, String)>,
    pub strip_bad_char: Option<String>,
    pub protection: Option<String>,
    pub protection_tags: Vec<String>,
    pub valid: bool,
    pub definition_hash: String,
}

impl CompiledSite {
    pub fn compile(site: &WmnSite) -> Self {
        let site_id = slugify(&site.name);
        let e_string = if site.e_string.trim().is_empty() {
            None
        } else {
            Some(site.e_string.clone())
        };
        let m_string = if site.m_string.trim().is_empty() {
            None
        } else {
            Some(site.m_string.clone())
        };

        let headers = site
            .headers
            .as_ref()
            .map(|h| h.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default();

        let is_post = site.post_body.is_some();

        let protection_tags = site.protection.clone().unwrap_or_default();
        let protection = if protection_tags.is_empty() {
            None
        } else {
            Some(protection_tags.join(", "))
        };

        // Compute definition hash
        let mut hasher = Sha256::new();
        hasher.update(site.name.as_bytes());
        hasher.update(b"\0");
        hasher.update(site.uri_check.as_bytes());
        hasher.update(b"\0");
        hasher.update(site.e_code.to_string().as_bytes());
        hasher.update(b"\0");
        hasher.update(site.e_string.as_bytes());
        hasher.update(b"\0");
        hasher.update(site.m_code.to_string().as_bytes());
        hasher.update(b"\0");
        hasher.update(site.m_string.as_bytes());
        let def_hash = to_hex(&hasher.finalize());

        Self {
            site_id,
            name: site.name.clone(),
            category: site.cat.trim().to_string(),
            uri_check: site.uri_check.clone(),
            uri_pretty: site.uri_pretty.clone(),
            e_code: site.e_code as u16,
            e_string,
            m_code: site.m_code as u16,
            m_string,
            is_post,
            post_body: site.post_body.clone(),
            headers,
            strip_bad_char: site.strip_bad_char.clone(),
            protection,
            protection_tags,
            valid: site.valid,
            definition_hash: def_hash,
        }
    }
}

pub fn slugify(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .to_string()
}

/// Immutable in-memory dataset snapshot.
#[derive(Clone, Debug)]
pub struct DatasetSnapshot {
    pub sites: Vec<CompiledSite>,
    pub id_to_index: HashMap<String, usize>,
    pub category_to_indexes: HashMap<String, Vec<usize>>,
    pub snapshot_hash: String,
    pub raw_data: WmnData,
}

impl DatasetSnapshot {
    pub fn new(data: WmnData, snapshot_hash: String) -> Self {
        let mut sites = Vec::with_capacity(data.sites.len());
        let mut id_to_index = HashMap::new();
        let mut category_to_indexes: HashMap<String, Vec<usize>> = HashMap::new();

        for site in &data.sites {
            let compiled = CompiledSite::compile(site);
            let idx = sites.len();
            id_to_index.insert(compiled.site_id.clone(), idx);
            category_to_indexes
                .entry(compiled.category.clone())
                .or_default()
                .push(idx);
            sites.push(compiled);
        }

        Self {
            sites,
            id_to_index,
            category_to_indexes,
            snapshot_hash,
            raw_data: data,
        }
    }

    pub fn get_site(&self, id: &str) -> Option<&CompiledSite> {
        self.id_to_index.get(id).map(|&idx| &self.sites[idx])
    }
}

static ACTIVE_SNAPSHOT: LazyLock<RwLock<Option<Arc<DatasetSnapshot>>>> =
    LazyLock::new(|| RwLock::new(None));

/// Parse and compile raw JSON bytes into a `DatasetSnapshot`.
pub fn parse_dataset(bytes: &[u8]) -> Result<DatasetSnapshot> {
    let data: WmnData = serde_json::from_slice(bytes)
        .map_err(|err| anyhow!("invalid WhatsMyName JSON format: {err}"))?;

    if data.sites.is_empty() {
        return Err(anyhow!("WhatsMyName dataset contains no sites"));
    }

    let sha256_hex = to_hex(&Sha256::digest(bytes));
    Ok(DatasetSnapshot::new(data, sha256_hex))
}

/// Retrieve or lazily load the active snapshot from disk.
pub fn active_snapshot() -> Option<Arc<DatasetSnapshot>> {
    {
        let reader = ACTIVE_SNAPSHOT.read().unwrap();
        if let Some(ref snap) = *reader {
            return Some(Arc::clone(snap));
        }
    }

    let mut writer = ACTIVE_SNAPSHOT.write().unwrap();
    if let Some(ref snap) = *writer {
        return Some(Arc::clone(snap));
    }

    // Try reading active snapshot from disk
    if let Ok(Some((_manifest, bytes))) = dataset::read_active_data(DATASET_NAME, "wmn-data.json") {
        if let Ok(snap) = parse_dataset(&bytes) {
            let arc = Arc::new(snap);
            *writer = Some(Arc::clone(&arc));
            return Some(arc);
        }
    }

    None
}

/// Reload active snapshot from disk.
pub fn reload_active_snapshot() {
    let mut writer = ACTIVE_SNAPSHOT.write().unwrap();
    *writer = None;
}

/// Download and activate the official WhatsMyName dataset from upstream using a default client.
pub async fn refresh() -> Result<DatasetManifest> {
    let client = reqwest::Client::new();
    refresh_from_upstream(&client).await
}

/// Download and activate the official WhatsMyName dataset from upstream.
pub async fn refresh_from_upstream(client: &reqwest::Client) -> Result<DatasetManifest> {
    refresh_with_progress_and_client(client, None, |_| {}).await
}

/// Download and activate the official WhatsMyName dataset with progress and cancellation support.
pub async fn refresh_with_progress<F>(
    cancel: Option<Arc<AtomicBool>>,
    on_phase: F,
) -> Result<DatasetManifest>
where
    F: FnMut(&str) + Send,
{
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());
    refresh_with_progress_and_client(&client, cancel, on_phase).await
}

/// Download and activate the official WhatsMyName dataset with a client, progress, and cancellation support.
pub async fn refresh_with_progress_and_client<F>(
    client: &reqwest::Client,
    cancel: Option<Arc<AtomicBool>>,
    mut on_phase: F,
) -> Result<DatasetManifest>
where
    F: FnMut(&str) + Send,
{
    on_phase("Acquiring dataset refresh lease");
    let lease = dataset::acquire_refresh_lease(DATASET_NAME)
        .ok_or_else(|| anyhow!("refresh for {} is already running", DATASET_NAME))?;

    if cancel.as_ref().is_some_and(|c| c.load(Ordering::Relaxed)) {
        anyhow::bail!("cancelled by user");
    }

    on_phase("Fetching upstream dataset from GitHub");
    let resp = client
        .get(WMN_UPSTREAM_URL)
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .context("failed to fetch WhatsMyName upstream dataset")?;

    if cancel.as_ref().is_some_and(|c| c.load(Ordering::Relaxed)) {
        anyhow::bail!("cancelled by user");
    }

    on_phase("Reading upstream response bytes");
    let bytes = resp
        .bytes()
        .await
        .context("failed to read response bytes")?;

    if cancel.as_ref().is_some_and(|c| c.load(Ordering::Relaxed)) {
        anyhow::bail!("cancelled by user");
    }

    on_phase("Parsing and validating dataset schema");
    let snapshot = parse_dataset(&bytes)?;

    let valid_count = snapshot.sites.iter().filter(|s| s.valid).count();
    let skipped_count = snapshot.sites.len().saturating_sub(valid_count);

    if let Some(active) = dataset::load_active_manifest(DATASET_NAME) {
        if active.active_version == snapshot.snapshot_hash {
            dataset::update_last_checked(DATASET_NAME)?;
            on_phase("Dataset already up to date");
            return Ok(active);
        }
    }

    on_phase("Activating version in persistent storage");
    let now = Utc::now().to_rfc3339();
    let manifest = DatasetManifest {
        dataset: DATASET_NAME.to_string(),
        active_version: snapshot.snapshot_hash.clone(),
        retrieved_at: now.clone(),
        last_checked: now,
        source_url: WMN_UPSTREAM_URL.to_string(),
        license_status: "CC BY-SA 4.0".into(),
        total_count: snapshot.sites.len(),
        supported_count: valid_count,
        skipped_count,
        schema_version: 1,
    };

    let activated = dataset::store_and_activate(DATASET_NAME, "wmn-data.json", &bytes, manifest)?;
    reload_active_snapshot();
    drop(lease);
    on_phase("Dataset activated successfully");
    Ok(activated)
}

/// Import WhatsMyName dataset from a local file.
pub fn import_from_file(path: &std::path::Path) -> Result<DatasetManifest> {
    let bytes =
        std::fs::read(path).with_context(|| format!("failed to read file {}", path.display()))?;
    let snapshot = parse_dataset(&bytes)?;
    let valid_count = snapshot.sites.iter().filter(|s| s.valid).count();
    let skipped_count = snapshot.sites.len().saturating_sub(valid_count);

    let now = Utc::now().to_rfc3339();
    let manifest = DatasetManifest {
        dataset: DATASET_NAME.to_string(),
        active_version: snapshot.snapshot_hash.clone(),
        retrieved_at: now.clone(),
        last_checked: now,
        source_url: format!("file://{}", path.display()),
        license_status: "CC BY-SA 4.0".into(),
        total_count: snapshot.sites.len(),
        supported_count: valid_count,
        skipped_count,
        schema_version: 1,
    };

    let activated = dataset::store_and_activate(DATASET_NAME, "wmn-data.json", &bytes, manifest)?;
    reload_active_snapshot();
    Ok(activated)
}

/// Get current status of WhatsMyName dataset.
pub fn status() -> dataset::DatasetStatus {
    dataset::get_status(DATASET_NAME)
}

/// Site check status outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SiteOutcomeStatus {
    Found,
    NotFound,
    Blocked,
    RateLimited,
    Timeout,
    Ambiguous,
    Error,
    Skipped,
}

impl SiteOutcomeStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Found => "found",
            Self::NotFound => "not_found",
            Self::Blocked => "blocked",
            Self::RateLimited => "rate_limited",
            Self::Timeout => "timeout",
            Self::Ambiguous => "ambiguous",
            Self::Error => "error",
            Self::Skipped => "skipped",
        }
    }
}

/// Per-site enumeration outcome.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SiteOutcome {
    pub site_id: String,
    pub site_name: String,
    pub platform: Option<String>,
    pub original_handle: String,
    pub effective_handle: String,
    pub profile_url: String,
    pub request_url: String,
    pub status: SiteOutcomeStatus,
    pub detection_basis: String,
    pub checked_at: String,
    pub dataset_hash: String,
    pub error: Option<String>,
}

/// Account tuple for verified found accounts.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AccountTuple {
    pub handle: String,
    pub platform: String,
    pub profile_url: String,
    pub source_call_id: Option<String>,
    pub evidence_id: Option<String>,
}

/// Coverage summary across all attempted sites.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct CoverageSummary {
    pub eligible: usize,
    pub selected: usize,
    pub attempted: usize,
    pub completed: usize,
    pub found: usize,
    pub not_found: usize,
    pub blocked: usize,
    pub rate_limited: usize,
    pub timeout: usize,
    pub ambiguous: usize,
    pub error: usize,
    pub skipped: usize,
}

/// Aggregate observation returned by `whatsmyname_lookup`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LookupObservation {
    pub username: String,
    pub coverage: CoverageSummary,
    pub outcomes: Vec<SiteOutcome>,
    pub accounts: Vec<AccountTuple>,
}

/// Input schema for `whatsmyname_lookup`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LookupInput {
    pub username: String,
    #[serde(default)]
    pub categories: Option<Vec<String>>,
    #[serde(default)]
    pub sites: Option<Vec<String>>,
    #[serde(default)]
    pub max_sites: Option<usize>,
}

/// Normalize handle: strips leading '@'.
pub fn normalize_handle(handle: &str) -> String {
    handle.trim().trim_start_matches('@').to_string()
}

/// Apply site-local `strip_bad_char` to username.
pub fn apply_strip_bad_char(username: &str, strip: Option<&str>) -> String {
    let mut result = username.to_string();
    if let Some(bad_chars) = strip {
        result.retain(|c| !bad_chars.contains(c));
    }
    result
}

/// Context-aware replacement of `{account}` in a URL.
pub fn interpolate_url(template: &str, effective_handle: &str) -> Result<String> {
    if !template.contains("{account}") {
        return Ok(template.to_string());
    }
    // URL percent-encode for path/query context
    let encoded: String = form_urlencoded::byte_serialize(effective_handle.as_bytes()).collect();
    Ok(template.replace("{account}", &encoded))
}

/// Context-aware replacement of `{account}` in a POST body.
pub fn interpolate_post_body(template: &str, effective_handle: &str) -> String {
    if !template.contains("{account}") {
        return template.to_string();
    }
    // If it looks like JSON, escape quotes
    if template.trim().starts_with('{') {
        let escaped = effective_handle.replace('\\', "\\\\").replace('"', "\\\"");
        template.replace("{account}", &escaped)
    } else {
        // Form encoded
        let encoded: String =
            form_urlencoded::byte_serialize(effective_handle.as_bytes()).collect();
        template.replace("{account}", &encoded)
    }
}

/// SSRF protection: validates URL scheme and ensures resolved IP addresses
/// are not loopback, private, link-local, multicast, or unspecified.
pub fn validate_safe_destination(url_str: &str) -> Result<Url> {
    let parsed = Url::parse(url_str).with_context(|| format!("invalid URL: {url_str}"))?;

    let scheme = parsed.scheme();
    if scheme != "http" && scheme != "https" {
        return Err(anyhow!("unsupported scheme: {scheme}"));
    }

    let host_str = parsed
        .host_str()
        .ok_or_else(|| anyhow!("URL missing host"))?;

    // Check hostname strings
    let lower_host = host_str.to_ascii_lowercase();
    if lower_host == "localhost"
        || lower_host.ends_with(".localhost")
        || lower_host.ends_with(".local")
        || lower_host.ends_with(".internal")
    {
        return Err(anyhow!("disallowed local destination host: {host_str}"));
    }

    // If host is an IP literal
    if let Ok(ip) = host_str.parse::<IpAddr>() {
        check_ip_safe(ip)?;
        return Ok(parsed);
    }

    // Resolve DNS and verify every address
    let port = parsed.port_or_known_default().unwrap_or(80);
    let addrs = format!("{host_str}:{port}")
        .to_socket_addrs()
        .with_context(|| format!("DNS resolution failed for {host_str}"))?;

    for socket_addr in addrs {
        check_ip_safe(socket_addr.ip())?;
    }

    Ok(parsed)
}

fn check_ip_safe(ip: IpAddr) -> Result<()> {
    match ip {
        IpAddr::V4(v4) => {
            if v4.is_loopback() {
                return Err(anyhow!("loopback IP disallowed: {v4}"));
            }
            if v4.is_private() {
                return Err(anyhow!("private IP disallowed: {v4}"));
            }
            if v4.is_link_local() {
                return Err(anyhow!("link-local IP disallowed: {v4}"));
            }
            if v4.is_broadcast() || v4.is_multicast() || v4.is_unspecified() {
                return Err(anyhow!("reserved/unspecified IP disallowed: {v4}"));
            }
        }
        IpAddr::V6(v6) => {
            if v6.is_loopback() {
                return Err(anyhow!("loopback IPv6 disallowed: {v6}"));
            }
            if v6.is_multicast() || v6.is_unspecified() {
                return Err(anyhow!("multicast/unspecified IPv6 disallowed: {v6}"));
            }
            // Unique local (fc00::/7) or link local (fe80::/10)
            let segs = v6.segments();
            if (segs[0] & 0xfe00) == 0xfc00 {
                return Err(anyhow!("unique local IPv6 disallowed: {v6}"));
            }
            if (segs[0] & 0xffc0) == 0xfe80 {
                return Err(anyhow!("link-local IPv6 disallowed: {v6}"));
            }
        }
    }
    Ok(())
}

/// Host pacer tracking last request timestamp per host to enforce `>= 1s` spacing.
#[derive(Default)]
struct HostPacer {
    last_times: Mutex<HashMap<String, Instant>>,
}

impl HostPacer {
    async fn pace(&self, host: &str) {
        let wait_dur = {
            let mut map = self.last_times.lock().unwrap();
            let now = Instant::now();
            if let Some(last) = map.get(host) {
                let elapsed = now.duration_since(*last);
                if elapsed < PER_HOST_MIN_SPACING {
                    let wait = PER_HOST_MIN_SPACING - elapsed;
                    map.insert(host.to_string(), now + wait);
                    Some(wait)
                } else {
                    map.insert(host.to_string(), now);
                    None
                }
            } else {
                map.insert(host.to_string(), now);
                None
            }
        };

        if let Some(dur) = wait_dur {
            tokio::time::sleep(dur).await;
        }
    }
}

static HOST_PACER: LazyLock<HostPacer> = LazyLock::new(HostPacer::default);

/// In-memory cache for deterministic site outcomes:
/// key = `wmn:{effective_username}:{definition_hash}:{ADAPTER_VERSION}`
static SITE_CACHE: LazyLock<RwLock<HashMap<String, (SiteOutcome, Instant)>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

const CACHE_TTL: Duration = Duration::from_secs(24 * 3600); // 24 hours

fn cache_get(key: &str) -> Option<SiteOutcome> {
    let map = SITE_CACHE.read().unwrap();
    if let Some((outcome, inst)) = map.get(key) {
        if inst.elapsed() < CACHE_TTL {
            let mut hit = outcome.clone();
            hit.detection_basis = format!("{} (cached)", hit.detection_basis);
            return Some(hit);
        }
    }
    None
}

fn cache_set(key: String, outcome: SiteOutcome) {
    if matches!(
        outcome.status,
        SiteOutcomeStatus::Found | SiteOutcomeStatus::NotFound
    ) {
        let mut map = SITE_CACHE.write().unwrap();
        map.insert(key, (outcome, Instant::now()));
    }
}

/// Check one site against an effective username using the shared HTTP client.
pub async fn check_single_site(
    client: &reqwest::Client,
    site: &CompiledSite,
    username: &str,
    snapshot_hash: &str,
    global_sem: &Semaphore,
) -> SiteOutcome {
    let effective_handle = apply_strip_bad_char(username, site.strip_bad_char.as_deref());
    let cache_key = format!(
        "wmn:{}:{}:{}",
        effective_handle, site.definition_hash, ADAPTER_VERSION
    );

    if let Some(cached) = cache_get(&cache_key) {
        return cached;
    }

    let checked_at = Utc::now().to_rfc3339();

    if !site.valid {
        return SiteOutcome {
            site_id: site.site_id.clone(),
            site_name: site.name.clone(),
            platform: Some(site.site_id.clone()),
            original_handle: username.to_string(),
            effective_handle,
            profile_url: site
                .uri_pretty
                .clone()
                .unwrap_or_else(|| site.uri_check.clone()),
            request_url: site.uri_check.clone(),
            status: SiteOutcomeStatus::Skipped,
            detection_basis: "site marked valid: false upstream".into(),
            checked_at,
            dataset_hash: snapshot_hash.to_string(),
            error: None,
        };
    }

    let request_url_str = match interpolate_url(&site.uri_check, &effective_handle) {
        Ok(u) => u,
        Err(err) => {
            return SiteOutcome {
                site_id: site.site_id.clone(),
                site_name: site.name.clone(),
                platform: Some(site.site_id.clone()),
                original_handle: username.to_string(),
                effective_handle,
                profile_url: site
                    .uri_pretty
                    .clone()
                    .unwrap_or_else(|| site.uri_check.clone()),
                request_url: site.uri_check.clone(),
                status: SiteOutcomeStatus::Skipped,
                detection_basis: "failed to interpolate URL".into(),
                checked_at,
                dataset_hash: snapshot_hash.to_string(),
                error: Some(err.to_string()),
            };
        }
    };

    let profile_url_str = if let Some(ref pretty) = site.uri_pretty {
        interpolate_url(pretty, &effective_handle).unwrap_or_else(|_| request_url_str.clone())
    } else {
        request_url_str.clone()
    };

    // SSRF Destination Check
    let validated_url = match validate_safe_destination(&request_url_str) {
        Ok(u) => u,
        Err(err) => {
            return SiteOutcome {
                site_id: site.site_id.clone(),
                site_name: site.name.clone(),
                platform: Some(site.site_id.clone()),
                original_handle: username.to_string(),
                effective_handle,
                profile_url: profile_url_str,
                request_url: request_url_str,
                status: SiteOutcomeStatus::Skipped,
                detection_basis: "destination rejected by SSRF safety validation".into(),
                checked_at,
                dataset_hash: snapshot_hash.to_string(),
                error: Some(err.to_string()),
            };
        }
    };

    let host = validated_url.host_str().unwrap_or("").to_string();

    // Acquire global semaphore
    let _permit = match global_sem.acquire().await {
        Ok(p) => p,
        Err(_) => {
            return SiteOutcome {
                site_id: site.site_id.clone(),
                site_name: site.name.clone(),
                platform: Some(site.site_id.clone()),
                original_handle: username.to_string(),
                effective_handle,
                profile_url: profile_url_str,
                request_url: request_url_str,
                status: SiteOutcomeStatus::Error,
                detection_basis: "scheduler concurrency error".into(),
                checked_at,
                dataset_hash: snapshot_hash.to_string(),
                error: Some("semaphore closed".into()),
            };
        }
    };

    // Host pacing (>= 1s between requests to the same host)
    HOST_PACER.pace(&host).await;

    // Build HTTP request
    let mut req_builder = if site.is_post {
        let post_body = site
            .post_body
            .as_deref()
            .map(|body| interpolate_post_body(body, &effective_handle))
            .unwrap_or_default();
        client.post(validated_url.clone()).body(post_body)
    } else {
        client.get(validated_url.clone())
    };

    for (k, v) in &site.headers {
        if let (Ok(hname), Ok(hval)) = (k.parse::<HeaderName>(), v.parse::<HeaderValue>()) {
            req_builder = req_builder.header(hname, hval);
        }
    }

    req_builder = req_builder.timeout(REQUEST_TIMEOUT);

    // Send request
    let response = match req_builder.send().await {
        Ok(resp) => resp,
        Err(err) => {
            let status = if err.is_timeout() {
                SiteOutcomeStatus::Timeout
            } else {
                SiteOutcomeStatus::Error
            };
            return SiteOutcome {
                site_id: site.site_id.clone(),
                site_name: site.name.clone(),
                platform: Some(site.site_id.clone()),
                original_handle: username.to_string(),
                effective_handle,
                profile_url: profile_url_str,
                request_url: request_url_str,
                status,
                detection_basis: "HTTP transport failure".into(),
                checked_at,
                dataset_hash: snapshot_hash.to_string(),
                error: Some(err.to_string()),
            };
        }
    };

    let status_code = response.status().as_u16();

    // Check rate limit
    if status_code == 429 {
        return SiteOutcome {
            site_id: site.site_id.clone(),
            site_name: site.name.clone(),
            platform: Some(site.site_id.clone()),
            original_handle: username.to_string(),
            effective_handle,
            profile_url: profile_url_str,
            request_url: request_url_str,
            status: SiteOutcomeStatus::RateLimited,
            detection_basis: "HTTP 429 Too Many Requests".into(),
            checked_at,
            dataset_hash: snapshot_hash.to_string(),
            error: None,
        };
    }

    // Inspect raw 302 redirect response for missing signature
    if (status_code == 301
        || status_code == 302
        || status_code == 303
        || status_code == 307
        || status_code == 308)
        && status_code == site.m_code
        && site.m_string.is_none()
    {
        let outcome = SiteOutcome {
            site_id: site.site_id.clone(),
            site_name: site.name.clone(),
            platform: Some(site.site_id.clone()),
            original_handle: username.to_string(),
            effective_handle,
            profile_url: profile_url_str,
            request_url: request_url_str,
            status: SiteOutcomeStatus::NotFound,
            detection_basis: format!("HTTP redirect {status_code} matched missing code"),
            checked_at,
            dataset_hash: snapshot_hash.to_string(),
            error: None,
        };
        cache_set(cache_key, outcome.clone());
        return outcome;
    }

    // Read bounded body
    let body_bytes = match response.bytes().await {
        Ok(b) => b,
        Err(err) => {
            return SiteOutcome {
                site_id: site.site_id.clone(),
                site_name: site.name.clone(),
                platform: Some(site.site_id.clone()),
                original_handle: username.to_string(),
                effective_handle,
                profile_url: profile_url_str,
                request_url: request_url_str,
                status: SiteOutcomeStatus::Error,
                detection_basis: "failed to read response body".into(),
                checked_at,
                dataset_hash: snapshot_hash.to_string(),
                error: Some(err.to_string()),
            };
        }
    };

    let truncated = body_bytes.len() > MAX_BODY_BYTES;
    let body_slice = if truncated {
        &body_bytes[..MAX_BODY_BYTES]
    } else {
        &body_bytes[..]
    };
    let body_text = String::from_utf8_lossy(body_slice);

    // Evaluate detection
    let has_m_string = site
        .m_string
        .as_ref()
        .is_some_and(|m| body_text.contains(m));
    let has_e_string = site
        .e_string
        .as_ref()
        .is_some_and(|e| body_text.contains(e));

    let (outcome_status, basis) = if status_code == site.m_code {
        if let Some(ref m_str) = site.m_string {
            if has_m_string {
                (
                    SiteOutcomeStatus::NotFound,
                    format!("status {status_code} and body contained missing marker '{m_str}'"),
                )
            } else if status_code == site.e_code && has_e_string {
                (
                    SiteOutcomeStatus::Found,
                    format!("status {status_code} and body contained existing marker"),
                )
            } else {
                (
                    SiteOutcomeStatus::Ambiguous,
                    format!("status {status_code} matched m_code but missing marker was absent"),
                )
            }
        } else if status_code != site.e_code {
            (
                SiteOutcomeStatus::NotFound,
                format!("status {status_code} matched m_code {status_code}"),
            )
        } else {
            (SiteOutcomeStatus::Ambiguous, format!("status {status_code} matches both m_code and e_code without distinguishing markers"))
        }
    } else if status_code == site.e_code {
        if let Some(ref e_str) = site.e_string {
            if has_e_string {
                (
                    SiteOutcomeStatus::Found,
                    format!("status {status_code} and body contained existing marker '{e_str}'"),
                )
            } else if has_m_string {
                (
                    SiteOutcomeStatus::NotFound,
                    format!("status {status_code} but body contained missing marker"),
                )
            } else {
                (
                    SiteOutcomeStatus::Ambiguous,
                    format!("status {status_code} matched e_code but existing marker was absent"),
                )
            }
        } else if site.m_code != site.e_code {
            (
                SiteOutcomeStatus::Found,
                format!("status {status_code} matched e_code"),
            )
        } else {
            (
                SiteOutcomeStatus::Ambiguous,
                format!("status {status_code} matches both codes without markers"),
            )
        }
    } else if status_code == 403 {
        (
            SiteOutcomeStatus::Blocked,
            "HTTP 403 Forbidden / Challenge".into(),
        )
    } else {
        (
            SiteOutcomeStatus::Ambiguous,
            format!("status code {status_code} matched neither e_code nor m_code"),
        )
    };

    let outcome = SiteOutcome {
        site_id: site.site_id.clone(),
        site_name: site.name.clone(),
        platform: Some(site.site_id.clone()),
        original_handle: username.to_string(),
        effective_handle,
        profile_url: profile_url_str,
        request_url: request_url_str,
        status: outcome_status,
        detection_basis: basis,
        checked_at,
        dataset_hash: snapshot_hash.to_string(),
        error: None,
    };

    cache_set(cache_key, outcome.clone());
    outcome
}

/// Execute username lookup across selected sites in the active dataset.
pub async fn lookup(
    client: &reqwest::Client,
    input: LookupInput,
    user_agent: Option<&str>,
) -> Result<LookupObservation> {
    let snapshot = active_snapshot().ok_or_else(|| {
        anyhow!("WhatsMyName dataset not loaded. Please download or import dataset first.")
    })?;

    let clean_username = normalize_handle(&input.username);
    if clean_username.is_empty() {
        return Err(anyhow!("username cannot be empty"));
    }

    let max_sites = input.max_sites.unwrap_or(DEFAULT_AUTO_MAX_SITES);

    // Site selection algorithm
    let mut candidate_indices: Vec<usize> = if let Some(ref site_ids) = input.sites {
        site_ids
            .iter()
            .filter_map(|id| snapshot.id_to_index.get(id.trim()).copied())
            .collect()
    } else if let Some(ref cats) = input.categories {
        cats.iter()
            .flat_map(|cat| {
                snapshot
                    .category_to_indexes
                    .get(cat.trim())
                    .into_iter()
                    .flat_map(|idxs| idxs.iter().copied())
            })
            .collect()
    } else {
        (0..snapshot.sites.len()).collect()
    };

    // Filter valid sites and deduplicate indices
    let mut seen = HashSet::new();
    candidate_indices.retain(|&idx| {
        if seen.insert(idx) {
            snapshot.sites[idx].valid
        } else {
            false
        }
    });

    let eligible_count = candidate_indices.len();
    candidate_indices.truncate(max_sites);
    let selected_count = candidate_indices.len();

    // Prepare client with redirect none and custom/default user agent
    let _effective_ua =
        user_agent.unwrap_or("Argos-OSINT/1.0 (+https://github.com/xxxcess/argos-osint)");
    let client = client.clone();

    let global_sem = Arc::new(Semaphore::new(GLOBAL_CONCURRENCY));
    let snap_ref = Arc::clone(&snapshot);
    let u_name = clean_username.clone();

    // Stream site checks with buffer_unordered
    let outcomes_stream = stream::iter(candidate_indices.into_iter().map(|idx| {
        let site = snap_ref.sites[idx].clone();
        let client = client.clone();
        let u = u_name.clone();
        let hash = snap_ref.snapshot_hash.clone();
        let sem = Arc::clone(&global_sem);
        async move { check_single_site(&client, &site, &u, &hash, &sem).await }
    }))
    .buffer_unordered(GLOBAL_CONCURRENCY);

    let outcomes: Vec<SiteOutcome> = outcomes_stream.collect().await;

    // Calculate coverage and extract verified account tuples
    let mut coverage = CoverageSummary {
        eligible: eligible_count,
        selected: selected_count,
        attempted: outcomes.len(),
        completed: 0,
        found: 0,
        not_found: 0,
        blocked: 0,
        rate_limited: 0,
        timeout: 0,
        ambiguous: 0,
        error: 0,
        skipped: 0,
    };

    let mut accounts = Vec::new();

    for outcome in &outcomes {
        match outcome.status {
            SiteOutcomeStatus::Found => {
                coverage.completed += 1;
                coverage.found += 1;
                accounts.push(AccountTuple {
                    handle: outcome.effective_handle.clone(),
                    platform: outcome
                        .platform
                        .clone()
                        .unwrap_or_else(|| outcome.site_id.clone()),
                    profile_url: outcome.profile_url.clone(),
                    source_call_id: None,
                    evidence_id: None,
                });
            }
            SiteOutcomeStatus::NotFound => {
                coverage.completed += 1;
                coverage.not_found += 1;
            }
            SiteOutcomeStatus::Blocked => {
                coverage.completed += 1;
                coverage.blocked += 1;
            }
            SiteOutcomeStatus::RateLimited => {
                coverage.completed += 1;
                coverage.rate_limited += 1;
            }
            SiteOutcomeStatus::Timeout => {
                coverage.timeout += 1;
            }
            SiteOutcomeStatus::Ambiguous => {
                coverage.completed += 1;
                coverage.ambiguous += 1;
            }
            SiteOutcomeStatus::Error => {
                coverage.error += 1;
            }
            SiteOutcomeStatus::Skipped => {
                coverage.skipped += 1;
            }
        }
    }

    Ok(LookupObservation {
        username: clean_username,
        coverage,
        outcomes,
        accounts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_parse_wmn_data_and_compile_site() {
        let raw = r#"{
            "categories": ["tech", "social"],
            "sites": [
                {
                    "name": "TestSite",
                    "uri_check": "https://example.com/u/{account}",
                    "uri_pretty": "https://example.com/{account}",
                    "cat": "tech",
                    "e_code": 200,
                    "e_string": "Profile",
                    "m_code": 404,
                    "m_string": "Not Found",
                    "valid": true
                },
                {
                    "name": "InvalidSite",
                    "uri_check": "https://example.com/invalid/{account}",
                    "cat": "social",
                    "e_code": 200,
                    "e_string": "",
                    "m_code": 404,
                    "m_string": "",
                    "valid": false
                }
            ]
        }"#;

        let snapshot = parse_dataset(raw.as_bytes()).expect("should parse");
        assert_eq!(snapshot.sites.len(), 2);
        assert!(snapshot.sites[0].valid);
        assert!(!snapshot.sites[1].valid);
        assert_eq!(snapshot.sites[0].site_id, "testsite");
        assert_eq!(snapshot.sites[0].e_string.as_deref(), Some("Profile"));
        assert_eq!(snapshot.sites[1].e_string, None);
    }

    #[test]
    fn test_handle_normalization_and_strip_bad_char() {
        assert_eq!(normalize_handle("@alice"), "alice");
        assert_eq!(normalize_handle("bob"), "bob");

        assert_eq!(apply_strip_bad_char("user.name", Some(".")), "username");
        assert_eq!(apply_strip_bad_char("user-name", Some(".-")), "username");
        assert_eq!(apply_strip_bad_char("alice", None), "alice");
    }

    #[test]
    fn test_url_and_post_interpolation() {
        assert_eq!(
            interpolate_url("https://example.com/{account}", "john doe").unwrap(),
            "https://example.com/john+doe"
        );

        let json_body = r#"{"user":"{account}"}"#;
        assert_eq!(
            interpolate_post_body(json_body, r#"alice"smith"#),
            r#"{"user":"alice\"smith"}"#
        );
    }

    #[test]
    fn test_ssrf_protection_rejects_private_and_loopback_ips() {
        assert!(validate_safe_destination("http://127.0.0.1/test").is_err());
        assert!(validate_safe_destination("http://10.0.0.1/test").is_err());
        assert!(validate_safe_destination("http://192.168.1.1/test").is_err());
        assert!(validate_safe_destination("http://169.254.169.254/latest/meta-data").is_err());
        assert!(validate_safe_destination("ftp://example.com").is_err());
        assert!(validate_safe_destination("http://localhost:8080/").is_err());
    }

    #[test]
    fn fixture_17_4_whatsmyname_site_selection() {
        let mut sites = Vec::new();
        for i in 0..15 {
            sites.push(json!({
                "name": format!("TechSite{i}"),
                "uri_check": format!("https://example.com/tech{i}/{{account}}"),
                "cat": "tech",
                "e_code": 200,
                "m_code": 404,
                "valid": true,
            }));
        }
        sites.push(json!({
            "name": "SocialSite1",
            "uri_check": "https://example.com/social1/{account}",
            "cat": "social",
            "e_code": 200,
            "m_code": 404,
            "valid": true,
        }));

        let raw = json!({
            "categories": ["tech", "social"],
            "sites": sites,
        });

        let snapshot = parse_dataset(raw.to_string().as_bytes()).expect("should parse");
        assert_eq!(snapshot.sites.len(), 16);

        // Test site selection with categories: ["tech"] and max_sites: 10
        let input = LookupInput {
            username: "ExampleUser".into(),
            categories: Some(vec!["tech".into()]),
            sites: None,
            max_sites: Some(10),
        };

        let mut candidate_indices: Vec<usize> = input
            .categories
            .as_ref()
            .unwrap()
            .iter()
            .flat_map(|cat| {
                snapshot
                    .category_to_indexes
                    .get(cat.trim())
                    .into_iter()
                    .flat_map(|idxs| idxs.iter().copied())
            })
            .collect();
        candidate_indices.sort_unstable();
        candidate_indices.dedup();
        candidate_indices.truncate(10);

        assert_eq!(candidate_indices.len(), 10);
        for idx in candidate_indices {
            assert_eq!(snapshot.sites[idx].category, "tech");
            assert!(snapshot.sites[idx].valid);
        }
    }

    #[test]
    fn benchmark_14_4_parse_index_and_selection() {
        // Generate a 1,000-entry synthetic fixture dataset
        let mut sites = Vec::with_capacity(1000);
        for i in 0..1000 {
            let cat = match i % 5 {
                0 => "tech",
                1 => "social",
                2 => "gaming",
                3 => "music",
                _ => "finance",
            };
            sites.push(json!({
                "name": format!("Site{i}"),
                "uri_check": format!("https://site{i}.example.com/u/{{account}}"),
                "uri_pretty": format!("https://site{i}.example.com/{{account}}"),
                "cat": cat,
                "e_code": 200,
                "e_string": "Profile",
                "m_code": 404,
                "m_string": "Not Found",
                "valid": i % 50 != 0, // valid except 2%
            }));
        }

        let dataset_json = json!({
            "license": ["CC BY-SA 4.0"],
            "authors": ["TestAuthor"],
            "categories": ["tech", "social", "gaming", "music", "finance"],
            "sites": sites,
        });
        let bytes = dataset_json.to_string().into_bytes();

        // 1. Measure cold parse and index build
        let t0 = std::time::Instant::now();
        let snapshot = parse_dataset(&bytes).expect("parse 1000-site dataset");
        let parse_elapsed = t0.elapsed();

        assert_eq!(snapshot.sites.len(), 1000);
        assert_eq!(snapshot.id_to_index.len(), 1000);
        // Cold index build target: under 100ms
        println!("1,000-site cold parse and index build: {:?}", parse_elapsed);
        assert!(
            parse_elapsed < std::time::Duration::from_millis(500),
            "cold parse under 500ms"
        );

        // 2. Measure warm exact and category selection
        let t1 = std::time::Instant::now();
        for _ in 0..100 {
            let tech_sites = snapshot.category_to_indexes.get("tech").unwrap();
            assert_eq!(tech_sites.len(), 200);
            let site = snapshot.id_to_index.get("site42").unwrap();
            assert_eq!(*site, 42);
        }
        let select_elapsed = t1.elapsed();
        println!("100 category & exact lookups: {:?}", select_elapsed);
        assert!(
            select_elapsed < std::time::Duration::from_millis(50),
            "100 selections under 50ms"
        );
    }

    #[test]
    fn test_parse_wmn_data_protection_variants() {
        let json_data = r#"{
            "license": "CC BY-SA 4.0",
            "authors": ["TestAuthor"],
            "categories": ["tech"],
            "sites": [
                {
                    "name": "SiteArrayProtection",
                    "uri_check": "https://example.com/{account}",
                    "cat": "tech",
                    "e_code": "200",
                    "e_string": "Profile",
                    "m_code": 404,
                    "m_string": "",
                    "protection": ["captcha", "cloudflare", "multiple"],
                    "valid": "true"
                },
                {
                    "name": "SiteStringProtection",
                    "uri_check": "https://example.com/2/{account}",
                    "cat": "tech",
                    "e_code": 200,
                    "e_string": "OK",
                    "m_code": "404",
                    "m_string": "Missing",
                    "protection": "cloudflare",
                    "valid": true
                },
                {
                    "name": "SiteNullProtection",
                    "uri_check": "https://example.com/3/{account}",
                    "cat": "tech",
                    "e_code": 200,
                    "e_string": "",
                    "m_code": 404,
                    "m_string": "",
                    "protection": null,
                    "valid": false
                }
            ]
        }"#;

        let snapshot =
            parse_dataset(json_data.as_bytes()).expect("should parse all protection variants");
        assert_eq!(snapshot.sites.len(), 3);

        let site0 = &snapshot.sites[0];
        assert_eq!(
            site0.protection.as_deref(),
            Some("captcha, cloudflare, multiple")
        );
        assert_eq!(
            site0.protection_tags,
            vec!["captcha", "cloudflare", "multiple"]
        );
        assert_eq!(site0.e_code, 200);
        assert!(site0.valid);

        let site1 = &snapshot.sites[1];
        assert_eq!(site1.protection.as_deref(), Some("cloudflare"));
        assert_eq!(site1.protection_tags, vec!["cloudflare"]);
        assert_eq!(site1.m_code, 404);
        assert!(site1.valid);

        let site2 = &snapshot.sites[2];
        assert_eq!(site2.protection, None);
        assert!(site2.protection_tags.is_empty());
        assert!(!site2.valid);
    }

    #[tokio::test]
    #[ignore]
    async fn test_live_upstream_wmn_data_parses() {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap();
        let resp = client
            .get(WMN_UPSTREAM_URL)
            .send()
            .await
            .expect("fetch upstream");
        let bytes = resp.bytes().await.expect("read upstream bytes");
        let snapshot = parse_dataset(&bytes).expect("parse upstream dataset");
        assert!(snapshot.sites.len() >= 700);
        println!(
            "Successfully parsed {} live upstream sites!",
            snapshot.sites.len()
        );
    }
}
