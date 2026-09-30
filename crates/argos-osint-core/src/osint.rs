//! Bounded, public HTTP observations shared by Recon and manual OSINT.
use anyhow::{anyhow, ensure, Result};
use chrono::Utc;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    net::IpAddr,
    sync::{Arc, OnceLock},
    time::Duration,
};
use tokio::sync::{Mutex, Semaphore};
use url::Url;

#[derive(Clone, Debug, Serialize)]
pub struct ToolDefinition {
    pub id: &'static str,
    pub name: &'static str,
    pub category: &'static str,
    pub description: &'static str,
    pub inputs: &'static [&'static str],
    pub documentation: &'static str,
    pub restrictions: &'static str,
    pub timeout_seconds: u64,
    pub cache_seconds: u64,
}
macro_rules! tool { ($id:expr,$name:expr,$category:expr,$description:expr,[$($input:expr),*],$doc:expr,$restriction:expr,$timeout:expr,$cache:expr) => { ToolDefinition {id:$id,name:$name,category:$category,description:$description,inputs:&[$($input),*],documentation:$doc,restrictions:$restriction,timeout_seconds:$timeout,cache_seconds:$cache} }; }
pub fn registry() -> &'static [ToolDefinition] {
    static TOOLS: OnceLock<Vec<ToolDefinition>> = OnceLock::new();
    TOOLS.get_or_init(|| vec![
        tool!("crtsh_certificates","crt.sh certificates","Domains","Discover certificate records and hostnames.",["domain"],"https://crt.sh/","Public service; availability varies.",30,86400),
        tool!("mnemonic_passive_dns","mnemonic Passive DNS","Domains","Historical hostname and IP relationships.",["domain_or_ip"],"https://docs.mnemonic.no/service-integration-guides/passivedns/docs/public/01-public_api.html","Public endpoint; bounded pagination and quotas apply.",30,3600),
        tool!("hackertarget_hostsearch","HackerTarget Host Search","Domains","Indexed subdomains and IPs.",["domain"],"https://hackertarget.com/ip-tools/","Free daily allowance and request rate apply.",20,3600),
        tool!("ripestat_network_info","RIPEstat network info","Networks","Announced prefix and routing ASN for an IP.",["ip"],"https://stat.ripe.net/docs/data_api","Routing origin is not website ownership.",20,3600),
        tool!("arin_rdap","ARIN RDAP","Networks","IP registration and contacts.",["ip"],"https://www.arin.net/resources/registry/whois/rdap/","Regional registry may redirect.",25,86400),
        tool!("apnic_rdap","APNIC RDAP","Networks","Asia-Pacific IP registration records.",["ip"],"https://www.apnic.net/about-apnic/whois_search/about/rdap/","Regional registry may redirect.",25,86400),
        tool!("wayback_availability","Wayback availability","Archives","Locate an available historical snapshot.",["url"],"https://archive.org/help/wayback_api.php","Snapshot availability does not reveal page contents.",30,3600),
        tool!("commoncrawl_urls","Common Crawl URLs","Archives","Crawled URLs and archive references.",["domain"],"https://index.commoncrawl.org/","Indexed crawl coverage is incomplete.",40,86400),
        tool!("arquivo_history","Arquivo.pt history","Archives","Archived versions of a site.",["domain_or_url"],"https://arquivo.pt/api","Archive coverage is incomplete.",40,86400),
        tool!("github_repositories","GitHub repositories","Code","Public repository names and metadata.",["query"],"https://docs.github.com/en/rest/search/search","Anonymous and search-specific quotas; not code search.",20,300),
        tool!("gitlab_projects","GitLab projects","Code","Public project metadata.",["query"],"https://docs.gitlab.com/api/projects/","Public projects only; rate limits apply.",20,300),
        tool!("grepapp_code_search","grep.app code search","Code","Source references to a string.",["query"],"https://grep.app/api","Snippets are untrusted evidence.",20,300),
        tool!("gleif_entities","GLEIF LEI entities","Organizations","Legal entities, LEIs and relationships.",["company_name|lei"],"https://www.gleif.org/en/lei-data/gleif-api","Search matches require identity confirmation.",25,86400),
        tool!("sec_submissions","SEC EDGAR submissions","Organizations","US reporting entity and filing metadata.",["cik|name|ticker"],"https://www.sec.gov/search-filings/edgar-application-programming-interfaces","Identifying User-Agent required; US reporting entities.",30,3600),
        tool!("wikidata_entities","Wikidata entities","Organizations","Find public organization entities and claims.",["name|qid"],"https://www.wikidata.org/wiki/Wikidata:REST_API","Search matches are candidates, not verified identity.",25,3600),
        tool!("keybase_identity","Keybase identity","Identities","Public profile and external identity proofs.",["username|domain"],"https://keybase.io/docs/api/1.0/call/user/lookup","Account does not establish physical identity.",20,3600),
        tool!("stackexchange_users","Stack Exchange users","Identities","Public profiles matching a display name.",["name"],"https://api.stackexchange.com/docs/users","Display names may be ambiguous.",20,3600),
        tool!("wikipedia_users","Wikipedia users","Identities","Account registration and edit metadata.",["username"],"https://www.mediawiki.org/wiki/API:Users","Account does not establish physical identity.",20,3600),
        tool!("nominatim_geocode","Nominatim geocode","Places","Place or address coordinates.",["address_or_place"],"https://operations.osmfoundation.org/policies/nominatim/","OpenStreetMap attribution; one request/second; no autocomplete or bulk queries.",20,86400),
        tool!("census_geocode","US Census geocode","Places","Coordinates for a US street address.",["us_address"],"https://www.census.gov/data/developers/data-sets/Geocoding-services.html","US addresses only.",20,86400),
        tool!("overpass_places","Overpass places","Places","Mapped features around coordinates.",["latitude","longitude","radius_m"],"https://wiki.openstreetmap.org/wiki/Overpass_API","Bounded radius and feature allowlist; OpenStreetMap attribution.",35,3600),
        tool!("blockchain_address","Blockchain.com Bitcoin address","Bitcoin","Bitcoin address balance and activity totals.",["bitcoin_address"],"https://www.blockchain.com/explorer/api/blockchain_api","Bitcoin only; address does not establish owner.",20,120),
        tool!("blockstream_address","Blockstream Bitcoin address","Bitcoin","Bitcoin address activity.",["bitcoin_address"],"https://github.com/Blockstream/esplora/blob/master/API.md","Bitcoin only; address does not establish owner.",20,120),
        tool!("mempool_address","mempool.space Bitcoin address","Bitcoin","Confirmed and unconfirmed address activity.",["bitcoin_address"],"https://mempool.space/docs/api/rest","Bitcoin only; address does not establish owner.",20,120),
        tool!("nvd_cve","NVD CVE","Vulnerabilities","CVE description, severity and affected products.",["cve_id"],"https://nvd.nist.gov/developers/start-here","Advisory does not prove live exploitability.",25,3600),
        tool!("osv_package","OSV package query","Vulnerabilities","Vulnerabilities affecting a package version or commit.",["ecosystem","package_name","version|commit"],"https://google.github.io/osv.dev/api/","Package coordinates and versions must match the target.",25,3600),
        tool!("cve_record","CVE published record","Vulnerabilities","Published CVE record and references.",["cve_id"],"https://cveawg.mitre.org/api-docs/","Advisory does not prove live exploitability.",25,3600),
        tool!("sans_ip_activity","SANS ISC IP activity","Exposure","Reported attack activity for an IP.",["ip"],"https://isc.sans.edu/api/","Reports are historical observations.",25,600),
        tool!("shodan_internetdb","Shodan InternetDB","Exposure","Observed ports, hostnames and vulnerability associations.",["ip"],"https://internetdb.shodan.io/","Free access is noncommercial; observations may be old.",20,600),
        tool!("urlscan_search","urlscan search","Exposure","Search existing website scan records.",["domain|query"],"https://urlscan.io/docs/api/","Search only; no scan submission; historical observations.",20,600),
    ]).as_slice()
}
pub fn definition(id: &str) -> Option<&'static ToolDefinition> {
    registry().iter().find(|t| t.id == id)
}
fn optional_keys(id: &str) -> &'static [&'static str] {
    match id {
        "crtsh_certificates" | "commoncrawl_urls" => &["limit"],
        "mnemonic_passive_dns" => &["limit", "offset"],
        "wayback_availability" => &["timestamp"],
        "arquivo_history" | "nominatim_geocode" => &["limit"],
        "github_repositories" | "gitlab_projects" => &["limit", "page"],
        "stackexchange_users" => &["site"],
        "overpass_places" => &["feature"],
        _ => &[],
    }
}
impl ToolDefinition {
    pub fn schema(&self) -> Value {
        let mut props = serde_json::Map::new();
        let mut required = Vec::new();
        let mut alternatives = Vec::new();
        for group in self.inputs {
            let keys: Vec<_> = group.split('|').collect();
            for key in &keys {
                let typ = if ["latitude", "longitude"].contains(key) {
                    "number"
                } else if ["radius_m", "limit"].contains(key) {
                    "integer"
                } else {
                    "string"
                };
                props.insert((*key).into(), json!({"type":typ}));
            }
            if keys.len() == 1 {
                required.push(keys[0]);
            } else {
                alternatives.push(json!({"anyOf":keys.iter().map(|key|json!({"required":[key]})).collect::<Vec<_>>()}));
            }
        }
        for key in optional_keys(self.id) {
            props.insert(
                (*key).into(),
                json!({"type":if ["limit","offset"].contains(key){"integer"}else{"string"}}),
            );
        }
        json!({"type":"object","properties":props,"required":required,"allOf":alternatives,"additionalProperties":false})
    }
    pub fn example_input(&self) -> Value {
        let mut values = serde_json::Map::new();
        for group in self.inputs {
            let key = group.split('|').next().unwrap_or("");
            let value = match key {
                "domain" | "domain_or_url" => json!("example.org"),
                "domain_or_ip" => json!("example.org"),
                "ip" => json!("8.8.8.8"),
                "url" => json!("https://example.org"),
                "query" => json!("example"),
                "company_name" => json!("Example Inc"),
                "cik" => json!("0000320193"),
                "name" => json!("Example"),
                "username" => json!("ExampleUser"),
                "address_or_place" => json!("New York, NY"),
                "us_address" => json!("1600 Pennsylvania Ave NW, Washington, DC"),
                "latitude" => json!(40.7128),
                "longitude" => json!(-74.006),
                "radius_m" => json!(500),
                "bitcoin_address" => json!("1BoatSLRHtKNngkdXEeobR76b53LETtpyT"),
                "cve_id" => json!("CVE-2021-44228"),
                "ecosystem" => json!("Maven"),
                "package_name" => json!("org.apache.logging.log4j:log4j-core"),
                "version" => json!("2.14.1"),
                _ => json!("example"),
            };
            values.insert(key.into(), value);
        }
        Value::Object(values)
    }
}
pub fn validate(id: &str, inputs: &Value) -> Result<()> {
    let def = definition(id).ok_or_else(|| anyhow!("unknown tool {id}"))?;
    let object = inputs
        .as_object()
        .ok_or_else(|| anyhow!("inputs must be a JSON object"))?;
    let allowed: Vec<_> = def
        .inputs
        .iter()
        .flat_map(|g| g.split('|'))
        .chain(optional_keys(id).iter().copied())
        .collect();
    for key in object.keys() {
        ensure!(allowed.contains(&key.as_str()), "unsupported input {key}");
    }
    for group in def.inputs {
        ensure!(
            group.split('|').any(|key| object.get(key).is_some()),
            "missing {}",
            group.replace('|', " or ")
        );
    }
    if id == "commoncrawl_urls" && inputs.get("index").is_none() {
        let mut copy = inputs.clone();
        copy["index"] = json!("CC-MAIN-0000-00");
        request(id, &copy).map(|_| ())
    } else {
        request(id, inputs).map(|_| ())
    }
}
fn str_arg<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty() && s.len() <= 300)
        .ok_or_else(|| anyhow!("missing or invalid {key}"))
}
fn one_of<'a>(v: &'a Value, keys: &[&'a str]) -> Result<(&'a str, &'a str)> {
    keys.iter()
        .find_map(|k| str_arg(v, k).ok().map(|s| (*k, s)))
        .ok_or_else(|| anyhow!("provide {}", keys.join(" or ")))
}
fn domain(s: &str) -> Result<String> {
    let s = s.trim_end_matches('.').to_ascii_lowercase();
    ensure!(
        s.len() <= 253
            && s.contains('.')
            && s.split('.').all(|p| !p.is_empty()
                && p.len() <= 63
                && p.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                && !p.starts_with('-')
                && !p.ends_with('-')),
        "invalid domain"
    );
    Ok(s)
}
fn ip(s: &str) -> Result<String> {
    Ok(s.parse::<IpAddr>()?.to_string())
}
fn bitcoin(s: &str) -> Result<String> {
    if s.to_ascii_lowercase().starts_with("bc1") {
        ensure!(
            s == s.to_ascii_lowercase() || s == s.to_ascii_uppercase(),
            "mixed case Bech32 address"
        );
        let lower = s.to_ascii_lowercase();
        let chars = "qpzry9x8gf2tvdw0s3jn54khce6mua7l";
        let values: Vec<u8> = lower[3..]
            .chars()
            .map(|c| {
                chars
                    .find(c)
                    .map(|n| n as u8)
                    .ok_or_else(|| anyhow!("invalid Bech32 character"))
            })
            .collect::<Result<_>>()?;
        ensure!(
            values.len() >= 7 && values.len() <= 87,
            "invalid Bech32 length"
        );
        let mut chk: u32 = 1;
        for value in [3u8, 3, 0, 2, 3].into_iter().chain(values.iter().copied()) {
            let top = chk >> 25;
            chk = ((chk & 0x1ffffff) << 5) ^ u32::from(value);
            for (i, g) in [0x3b6a57b2, 0x26508e6d, 0x1ea119fa, 0x3d4233dd, 0x2a1462b3]
                .iter()
                .enumerate()
            {
                if (top >> i) & 1 == 1 {
                    chk ^= g;
                }
            }
        }
        let witness = values[0];
        ensure!(witness <= 16, "invalid witness version");
        ensure!(
            chk == if witness == 0 { 1 } else { 0x2bc830a3 },
            "invalid Bech32 checksum"
        );
        let program = &values[1..values.len() - 6];
        let mut acc = 0u32;
        let mut bits = 0;
        let mut bytes = Vec::new();
        for value in program {
            acc = (acc << 5) | u32::from(*value);
            bits += 5;
            while bits >= 8 {
                bits -= 8;
                bytes.push(((acc >> bits) & 255) as u8);
            }
        }
        ensure!(
            bits < 5 && ((acc << (8 - bits)) & 255) == 0,
            "invalid witness padding"
        );
        ensure!(
            (2..=40).contains(&bytes.len()) && (witness != 0 || [20, 32].contains(&bytes.len())),
            "invalid witness program"
        );
        return Ok(s.into());
    }
    let alphabet = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    ensure!((26..=35).contains(&s.len()), "invalid Base58 length");
    let mut bytes = vec![0u8];
    for ch in s.chars() {
        let digit = alphabet
            .find(ch)
            .ok_or_else(|| anyhow!("invalid Base58 character"))? as u32;
        let mut carry = digit;
        for byte in bytes.iter_mut().rev() {
            carry += u32::from(*byte) * 58;
            *byte = (carry & 255) as u8;
            carry >>= 8;
        }
        while carry > 0 {
            bytes.insert(0, (carry & 255) as u8);
            carry >>= 8;
        }
    }
    let zeros = s.chars().take_while(|c| *c == '1').count();
    let mut decoded = vec![0u8; zeros];
    decoded.extend(bytes.into_iter().skip_while(|b| *b == 0));
    ensure!(
        decoded.len() == 25 && [0u8, 5].contains(&decoded[0]),
        "unsupported Bitcoin network or address type"
    );
    let hash = Sha256::digest(Sha256::digest(&decoded[..21]));
    ensure!(hash[..4] == decoded[21..], "invalid Base58 checksum");
    Ok(s.into())
}
fn url_arg(s: &str) -> Result<String> {
    let u = Url::parse(s)?;
    ensure!(
        (u.scheme() == "http" || u.scheme() == "https") && u.host_str().is_some(),
        "HTTP(S) URL required"
    );
    let host = u.host_str().unwrap_or("");
    ensure!(
        host != "localhost" && !host.ends_with(".localhost"),
        "local targets are unsupported"
    );
    if let Ok(ip) = host.parse::<IpAddr>() {
        let public = match ip {
            IpAddr::V4(ip) => {
                !(ip.is_private() || ip.is_loopback() || ip.is_link_local() || ip.is_unspecified())
            }
            IpAddr::V6(ip) => {
                !(ip.is_loopback()
                    || ip.is_unique_local()
                    || ip.is_unicast_link_local()
                    || ip.is_unspecified())
            }
        };
        ensure!(public, "private targets are unsupported");
    }
    Ok(u.to_string())
}
fn bounded(s: &str) -> Result<String> {
    ensure!(
        s.len() <= 200 && !s.chars().any(char::is_control),
        "input too long or contains controls"
    );
    Ok(s.to_string())
}
fn number_arg(v: &Value, key: &str, default: u64, max: u64) -> Result<String> {
    let n = match v.get(key) {
        Some(value) => value
            .as_u64()
            .ok_or_else(|| anyhow!("{key} must be a nonnegative integer"))?,
        None => default,
    };
    ensure!(
        n <= max && (key == "offset" || n > 0),
        "{key} exceeds the supported bound"
    );
    Ok(n.to_string())
}
fn parse_observations(
    id: &str,
    raw: &str,
    content_type: &str,
    ndjson: bool,
) -> Result<(Value, bool)> {
    ensure!(
        !content_type.contains("html") && !raw.trim_start().starts_with('<'),
        "HTML response instead of API data"
    );
    if id == "hackertarget_hostsearch" {
        ensure!(
            !raw.to_ascii_lowercase().contains("error") && !raw.contains("API count exceeded"),
            "provider quota or error: {}",
            raw.chars().take(200).collect::<String>()
        );
        let rows: Vec<Value> = raw
            .lines()
            .take(100)
            .filter_map(|l| {
                l.split_once(',')
                    .map(|(host, ip)| json!({"hostname":host,"ip":ip}))
            })
            .collect();
        return Ok((json!(rows), raw.lines().count() > 100));
    }
    if ndjson {
        let mut rows = Vec::new();
        for line in raw.lines().take(100) {
            rows.push(serde_json::from_str::<Value>(line)?);
        }
        return Ok((json!(rows), raw.lines().count() > 100));
    }
    let v: Value = serde_json::from_str(raw).map_err(|e| anyhow!("malformed JSON: {e}"))?;
    if let Some(error) = v.get("error").or_else(|| v.get("errors")) {
        if !error.is_null() {
            return Err(anyhow!(
                "provider error: {}",
                error.to_string().chars().take(250).collect::<String>()
            ));
        }
    }
    if id == "crtsh_certificates" {
        let mut hosts = std::collections::BTreeSet::new();
        if let Some(rows) = v.as_array() {
            for row in rows.iter().take(1000) {
                if let Some(names) = row.get("name_value").and_then(Value::as_str) {
                    for name in names.lines() {
                        hosts.insert(name.to_string());
                    }
                }
            }
        }
        return Ok((
            json!({"hostnames":hosts,"certificates":v.as_array().map(|a|a.iter().take(100).cloned().collect::<Vec<_>>()).unwrap_or_default()}),
            v.as_array().is_some_and(|a| a.len() > 100),
        ));
    }
    Ok((v, false))
}
fn no_results(id: &str, value: &Value) -> bool {
    if value.is_null() || value.as_array().is_some_and(Vec::is_empty) {
        return true;
    }
    if id == "wayback_availability" {
        return value
            .get("archived_snapshots")
            .and_then(Value::as_object)
            .is_some_and(|o| o.is_empty());
    }
    if id == "nvd_cve" && value.get("totalResults").and_then(Value::as_u64) == Some(0) {
        return true;
    }
    let pointer = match id {
        "github_repositories" | "stackexchange_users" => "/items",
        "grepapp_code_search" => "/hits/hits",
        "wikidata_entities" => "/search",
        "nvd_cve" => "/vulnerabilities",
        "urlscan_search" => "/results",
        "mnemonic_passive_dns" => "/data",
        "osv_package" => "/vulns",
        _ => return false,
    };
    value
        .pointer(pointer)
        .and_then(Value::as_array)
        .is_some_and(Vec::is_empty)
}
fn url(base: &str, path: &[&str], query: &[(&str, &str)]) -> Result<Url> {
    let mut u = Url::parse(base)?;
    {
        let mut seg = u
            .path_segments_mut()
            .map_err(|_| anyhow!("invalid endpoint"))?;
        for p in path {
            seg.push(p);
        }
    }
    u.query_pairs_mut().extend_pairs(query.iter().copied());
    Ok(u)
}
#[derive(Clone, Debug)]
struct Request {
    url: Url,
    body: Option<Value>,
    form: Option<Vec<(String, String)>>,
    ndjson: bool,
}
fn get(u: Url) -> Request {
    Request {
        url: u,
        body: None,
        form: None,
        ndjson: false,
    }
}
fn request(id: &str, v: &Value) -> Result<Request> {
    let q = |base, path: &[&str], query: &[(&str, &str)]| Ok(get(url(base, path, query)?));
    let domain_arg = || domain(str_arg(v, "domain")?);
    let ip_arg = || ip(str_arg(v, "ip")?);
    let cve = || {
        let s = str_arg(v, "cve_id")?.to_ascii_uppercase();
        ensure!(
            s.starts_with("CVE-")
                && s.len() <= 30
                && s[4..].split('-').count() == 2
                && s[4..].chars().all(|c| c.is_ascii_digit() || c == '-'),
            "invalid CVE ID"
        );
        Ok(s)
    };
    match id {
        "crtsh_certificates" => {
            let d = domain_arg()?;
            q("https://crt.sh/", &[], &[("q", &d), ("output", "json")])
        }
        "mnemonic_passive_dns" => {
            let s = str_arg(v, "domain_or_ip")?;
            let d = ip(s).or_else(|_| domain(s))?;
            let limit = number_arg(v, "limit", 25, 100)?;
            let offset = number_arg(v, "offset", 0, 1000)?;
            q(
                "https://api.mnemonic.no/pdns/v3",
                &[&d],
                &[("limit", &limit), ("offset", &offset)],
            )
        }
        "hackertarget_hostsearch" => {
            let d = domain_arg()?;
            q(
                "https://api.hackertarget.com/hostsearch/",
                &[],
                &[("q", &d)],
            )
        }
        "ripestat_network_info" => {
            let x = ip_arg()?;
            q(
                "https://stat.ripe.net/data/network-info/data.json",
                &[],
                &[("resource", &x)],
            )
        }
        "arin_rdap" => {
            let x = ip_arg()?;
            q("https://rdap.arin.net/registry/ip", &[&x], &[])
        }
        "apnic_rdap" => {
            let x = ip_arg()?;
            q("https://rdap.apnic.net/ip", &[&x], &[])
        }
        "wayback_availability" => {
            let x = url_arg(str_arg(v, "url")?)?;
            let t = v.get("timestamp").and_then(Value::as_str).unwrap_or("");
            q(
                "https://archive.org/wayback/available",
                &[],
                &[("url", &x), ("timestamp", t)],
            )
        }
        "commoncrawl_urls" => {
            let d = domain_arg()?;
            let index = str_arg(v, "index")?;
            ensure!(
                index.starts_with("CC-MAIN-")
                    && index
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-'),
                "invalid Common Crawl index"
            );
            let mut r = q(
                "https://index.commoncrawl.org",
                &[&format!("{index}-index")],
                &[("url", &format!("{d}/*")), ("output", "json")],
            )?;
            r.ndjson = true;
            Ok(r)
        }
        "arquivo_history" => {
            let x = str_arg(v, "domain_or_url")?;
            let x = if x.starts_with("http") {
                url_arg(x)?
            } else {
                domain(x)?
            };
            q(
                "https://arquivo.pt/textsearch",
                &[],
                &[("versionHistory", &x), ("maxItems", "30")],
            )
        }
        "github_repositories" => {
            let x = bounded(str_arg(v, "query")?)?;
            q(
                "https://api.github.com/search/repositories",
                &[],
                &[("q", &x), ("per_page", "30")],
            )
        }
        "gitlab_projects" => {
            let x = bounded(str_arg(v, "query")?)?;
            q(
                "https://gitlab.com/api/v4/projects",
                &[],
                &[("search", &x), ("visibility", "public"), ("per_page", "30")],
            )
        }
        "grepapp_code_search" => {
            let x = bounded(str_arg(v, "query")?)?;
            q("https://grep.app/api/search", &[], &[("q", &x)])
        }
        "gleif_entities" => {
            let (k, x) = one_of(v, &["lei", "company_name"])?;
            let x = bounded(x)?;
            if k == "lei" {
                ensure!(
                    x.len() == 20 && x.bytes().all(|b| b.is_ascii_alphanumeric()),
                    "invalid LEI"
                );
                q("https://api.gleif.org/api/v1/lei-records", &[&x], &[])
            } else {
                q(
                    "https://api.gleif.org/api/v1/lei-records",
                    &[],
                    &[("filter[entity.legalName]", &x), ("page[size]", "30")],
                )
            }
        }
        "sec_submissions" => {
            let cik = str_arg(v, "cik")
                .map(|s| s.to_string())
                .or_else(|_| one_of(v, &["name", "ticker"]).map(|(_, s)| s.to_string()))?;
            if cik.bytes().all(|b| b.is_ascii_digit()) {
                ensure!(cik.len() <= 10, "invalid CIK");
                q(
                    "https://data.sec.gov/submissions",
                    &[&format!("CIK{:0>10}.json", cik)],
                    &[],
                )
            } else {
                q("https://www.sec.gov/files", &["company_tickers.json"], &[])
            }
        }
        "wikidata_entities" => {
            let (k, x) = one_of(v, &["qid", "name"])?;
            let x = bounded(x)?;
            if k == "qid" {
                ensure!(
                    x.starts_with('Q') && x[1..].bytes().all(|b| b.is_ascii_digit()),
                    "invalid QID"
                );
                q(
                    "https://www.wikidata.org/wiki/Special:EntityData",
                    &[&format!("{x}.json")],
                    &[],
                )
            } else {
                q(
                    "https://www.wikidata.org/w/api.php",
                    &[],
                    &[
                        ("action", "wbsearchentities"),
                        ("search", &x),
                        ("language", "en"),
                        ("format", "json"),
                    ],
                )
            }
        }
        "keybase_identity" => {
            let (k, x) = one_of(v, &["username", "domain"])?;
            let x = if k == "domain" {
                domain(x)?
            } else {
                bounded(x)?
            };
            q(
                "https://keybase.io/_/api/1.0/user/lookup.json",
                &[],
                &[(if k == "domain" { "domain" } else { "usernames" }, &x)],
            )
        }
        "stackexchange_users" => {
            let x = bounded(str_arg(v, "name")?)?;
            let site = v
                .get("site")
                .and_then(Value::as_str)
                .unwrap_or("stackoverflow");
            ensure!(
                site.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-'),
                "invalid site"
            );
            q(
                "https://api.stackexchange.com/2.3/users",
                &[],
                &[("site", site), ("inname", &x), ("pagesize", "30")],
            )
        }
        "wikipedia_users" => {
            let x = bounded(str_arg(v, "username")?)?;
            q(
                "https://en.wikipedia.org/w/api.php",
                &[],
                &[
                    ("action", "query"),
                    ("list", "users"),
                    ("ususers", &x),
                    ("usprop", "registration|editcount|groups"),
                    ("format", "json"),
                ],
            )
        }
        "nominatim_geocode" => {
            let x = bounded(str_arg(v, "address_or_place")?)?;
            q(
                "https://nominatim.openstreetmap.org/search",
                &[],
                &[("q", &x), ("format", "jsonv2"), ("limit", "5")],
            )
        }
        "census_geocode" => {
            let x = bounded(str_arg(v, "us_address")?)?;
            q(
                "https://geocoding.geo.census.gov/geocoder/locations/onelineaddress",
                &[],
                &[
                    ("address", &x),
                    ("benchmark", "Public_AR_Current"),
                    ("format", "json"),
                ],
            )
        }
        "overpass_places" => {
            let lat = v
                .get("latitude")
                .and_then(Value::as_f64)
                .ok_or_else(|| anyhow!("latitude required"))?;
            let lon = v
                .get("longitude")
                .and_then(Value::as_f64)
                .ok_or_else(|| anyhow!("longitude required"))?;
            let radius = v.get("radius_m").and_then(Value::as_u64).unwrap_or(1000);
            ensure!(
                (-90.0..=90.0).contains(&lat)
                    && (-180.0..=180.0).contains(&lon)
                    && (50..=3000).contains(&radius),
                "invalid coordinates or radius"
            );
            let feature = v
                .get("feature")
                .and_then(Value::as_str)
                .unwrap_or("amenity");
            ensure!(
                ["amenity", "shop", "tourism", "healthcare", "railway"].contains(&feature),
                "unsupported feature"
            );
            let data = format!(
                "[out:json][timeout:25];nwr(around:{radius},{lat},{lon})[{feature}];out center 50;"
            );
            Ok(Request {
                url: Url::parse("https://overpass-api.de/api/interpreter")?,
                body: None,
                form: Some(vec![("data".into(), data)]),
                ndjson: false,
            })
        }
        "blockchain_address" | "blockstream_address" | "mempool_address" => {
            let x = bitcoin(str_arg(v, "bitcoin_address")?)?;
            match id {
                "blockchain_address" => {
                    q("https://blockchain.info/balance", &[], &[("active", &x)])
                }
                "blockstream_address" => q("https://blockstream.info/api/address", &[&x], &[]),
                _ => q("https://mempool.space/api/address", &[&x], &[]),
            }
        }
        "nvd_cve" => {
            let x = cve()?;
            q(
                "https://services.nvd.nist.gov/rest/json/cves/2.0",
                &[],
                &[("cveId", &x)],
            )
        }
        "osv_package" => {
            let eco = bounded(str_arg(v, "ecosystem")?)?;
            let pkg = bounded(str_arg(v, "package_name")?)?;
            let version = v.get("version").and_then(Value::as_str);
            let commit = v.get("commit").and_then(Value::as_str);
            ensure!(
                version.is_some() ^ commit.is_some(),
                "provide version or commit"
            );
            if eco == "Maven" {
                ensure!(
                    pkg.contains(':') && pkg.split(':').all(|s| !s.is_empty()),
                    "Maven package must be group:artifact"
                );
            }
            let body = if let Some(c) = commit {
                json!({"commit":bounded(c)?})
            } else {
                json!({"package":{"name":pkg,"ecosystem":eco},"version":bounded(version.unwrap())?})
            };
            Ok(Request {
                url: Url::parse("https://api.osv.dev/v1/query")?,
                body: Some(body),
                form: None,
                ndjson: false,
            })
        }
        "cve_record" => {
            let x = cve()?;
            q("https://cveawg.mitre.org/api/cve", &[&x], &[])
        }
        "sans_ip_activity" => {
            let x = ip_arg()?;
            q("https://isc.sans.edu/api/ip", &[&x], &[("json", "")])
        }
        "shodan_internetdb" => {
            let x = ip_arg()?;
            q("https://internetdb.shodan.io", &[&x], &[])
        }
        "urlscan_search" => {
            let x = if let Ok(d) = domain_arg() {
                format!("domain:{d}")
            } else {
                let s = str_arg(v, "query")?;
                ensure!(
                    s.len() <= 120
                        && s.split_whitespace().all(|term| term.starts_with("domain:")
                            || term.starts_with("page.domain:")
                            || term.starts_with("ip:")
                            || term == "AND"),
                    "unsupported search expression"
                );
                s.to_string()
            };
            q(
                "https://urlscan.io/api/v1/search/",
                &[],
                &[("q", &x), ("size", "30")],
            )
        }
        _ => Err(anyhow!("unknown tool {id}")),
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolResult {
    pub tool_id: String,
    pub inputs: Value,
    pub status: String,
    pub source_url: String,
    pub retrieved_at: String,
    pub observations: Value,
    pub raw: String,
    pub error: Option<String>,
    pub cached: bool,
    pub truncated: bool,
}
type HostSchedule = Arc<Mutex<HashMap<String, std::time::Instant>>>;
type SharedHttp = (reqwest::Client, Arc<Semaphore>, HostSchedule);
#[derive(Clone)]
pub struct Executor {
    client: reqwest::Client,
    global: Arc<Semaphore>,
    hosts: HostSchedule,
}
impl Executor {
    pub fn new() -> Result<Self> {
        static SHARED: OnceLock<SharedHttp> = OnceLock::new();
        let shared = SHARED.get_or_init(|| {
            let client = reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .user_agent(
                    "Argos OSINT/0.1 (public research; contact: configure osint_user_agent)",
                )
                .build()
                .expect("HTTP client");
            (
                client,
                Arc::new(Semaphore::new(4)),
                Arc::new(Mutex::new(HashMap::new())),
            )
        });
        Ok(Self {
            client: shared.0.clone(),
            global: shared.1.clone(),
            hosts: shared.2.clone(),
        })
    }
    async fn pace_host(&self, host: &str, interval: Duration) {
        let delay = {
            let mut hosts = self.hosts.lock().await;
            let now = std::time::Instant::now();
            let ready = hosts
                .get(host)
                .map(|last| (*last + interval).max(now))
                .unwrap_or(now);
            hosts.insert(host.to_string(), ready);
            ready.saturating_duration_since(now)
        };
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
    }
    async fn metadata_json(&self, endpoint: &str, user_agent: Option<&str>) -> Result<Value> {
        let url = Url::parse(endpoint)?;
        let host = url
            .host_str()
            .ok_or_else(|| anyhow!("metadata host missing"))?;
        ensure!(
            ["www.sec.gov", "index.commoncrawl.org"].contains(&host),
            "metadata host is not allowed"
        );
        let _permit = self.global.acquire().await?;
        self.pace_host(host, Duration::from_secs(1)).await;
        let mut request = self.client.get(url).timeout(Duration::from_secs(20));
        if let Some(agent) = user_agent {
            request = request.header(reqwest::header::USER_AGENT, agent);
        }
        let response = request.send().await?.error_for_status()?;
        let mut stream = response.bytes_stream();
        let mut bytes = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            ensure!(
                bytes.len() + chunk.len() <= 4_000_000,
                "metadata response too large"
            );
            bytes.extend_from_slice(&chunk);
        }
        Ok(serde_json::from_slice(&bytes)?)
    }
    pub async fn run(
        &self,
        id: &str,
        mut inputs: Value,
        user_agent: Option<&str>,
    ) -> Result<ToolResult> {
        let def = definition(id).ok_or_else(|| anyhow!("unknown tool {id}"))?;
        if ["nominatim_geocode", "sec_submissions"].contains(&id) {
            ensure!(
                user_agent.is_some_and(|s| s.contains('@') || s.contains("http")),
                "configure identifying osint_user_agent for {id}"
            );
        }
        if id == "sec_submissions" && inputs.get("cik").is_none() {
            let (kind, needle) = one_of(&inputs, &["ticker", "name"])?;
            let mapping = self
                .metadata_json("https://www.sec.gov/files/company_tickers.json", user_agent)
                .await?;
            let matches: Vec<Value> = mapping
                .as_object()
                .into_iter()
                .flat_map(|o| o.values())
                .filter(|row| {
                    let field = if kind == "ticker" { "ticker" } else { "title" };
                    row.get(field).and_then(Value::as_str).is_some_and(|s| {
                        if kind == "ticker" {
                            s.eq_ignore_ascii_case(needle)
                        } else {
                            s.to_ascii_lowercase()
                                .contains(&needle.to_ascii_lowercase())
                        }
                    })
                })
                .take(30)
                .cloned()
                .collect();
            if matches.len() != 1 {
                return Ok(ToolResult {
                    tool_id: id.into(),
                    inputs,
                    status: if matches.is_empty() {
                        "no_results"
                    } else {
                        "completed"
                    }
                    .into(),
                    source_url: "https://www.sec.gov/files/company_tickers.json".into(),
                    retrieved_at: Utc::now().to_rfc3339(),
                    observations: json!({"candidate_matches":matches,"ambiguous":matches.len()>1}),
                    raw: String::new(),
                    error: None,
                    cached: false,
                    truncated: matches.len() == 30,
                });
            }
            let cik = matches[0]
                .get("cik_str")
                .and_then(Value::as_u64)
                .ok_or_else(|| anyhow!("SEC mapping lacks CIK"))?;
            inputs["cik"] = json!(cik.to_string());
        }
        if id == "commoncrawl_urls" && inputs.get("index").is_none() {
            let catalogs = self
                .metadata_json("https://index.commoncrawl.org/collinfo.json", user_agent)
                .await?;
            let index = catalogs
                .as_array()
                .and_then(|rows| rows.first())
                .and_then(|row| row.get("id"))
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("Common Crawl index discovery returned no index"))?;
            inputs["index"] = json!(index);
        }
        let req = request(id, &inputs)?;
        let host = req.url.host_str().unwrap_or("").to_string();
        let _permit = self.global.acquire().await?;
        let interval = match id {
            "nominatim_geocode" | "urlscan_search" => Duration::from_secs(1),
            "hackertarget_hostsearch" | "overpass_places" => Duration::from_secs(2),
            "github_repositories" | "nvd_cve" => Duration::from_secs(6),
            _ => Duration::from_millis(250),
        };
        self.pace_host(&host, interval).await;
        let mut url = req.url.clone();
        let mut attempts = 0;
        let mut redirects = 0;
        loop {
            attempts += 1;
            let mut builder = if let Some(body) = &req.body {
                self.client.post(url.clone()).json(body)
            } else if let Some(form) = &req.form {
                self.client.post(url.clone()).form(form)
            } else {
                self.client.get(url.clone())
            };
            builder = builder.timeout(Duration::from_secs(def.timeout_seconds));
            if let Some(ua) = user_agent {
                builder = builder.header(reqwest::header::USER_AGENT, ua);
            }
            let response =
                tokio::time::timeout(Duration::from_secs(def.timeout_seconds), builder.send())
                    .await??;
            if response.status().is_redirection() {
                redirects += 1;
                ensure!(redirects <= 4, "too many redirects");
                let location = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|v| v.to_str().ok())
                    .ok_or_else(|| anyhow!("redirect has no location"))?;
                let next = url.join(location)?;
                let host = next.host_str().unwrap_or("");
                let same_host = host == url.host_str().unwrap_or("");
                let rdap = ["arin_rdap", "apnic_rdap"].contains(&id)
                    && [
                        "rdap.arin.net",
                        "rdap.apnic.net",
                        "rdap.db.ripe.net",
                        "rdap.lacnic.net",
                        "rdap.afrinic.net",
                    ]
                    .contains(&host);
                ensure!(
                    next.scheme() == "https" && (same_host || rdap),
                    "redirect host is not allowed"
                );
                url = next;
                continue;
            }
            if (response.status().as_u16() == 429 || response.status().is_server_error())
                && attempts < 3
            {
                let delay = response
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(1 << attempts)
                    .min(15);
                let jitter = Utc::now().timestamp_subsec_millis() as u64 % 250;
                tokio::time::sleep(Duration::from_secs(delay) + Duration::from_millis(jitter))
                    .await;
                continue;
            }
            let status = response.status();
            let content_type = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();
            let mut stream = response.bytes_stream();
            let mut bytes = Vec::new();
            let mut truncated = false;
            while let Some(chunk) = stream.next().await {
                let chunk = chunk?;
                if bytes.len() + chunk.len() > 1_000_000 {
                    let room = 1_000_000 - bytes.len();
                    bytes.extend_from_slice(&chunk[..room]);
                    truncated = true;
                    break;
                }
                bytes.extend_from_slice(&chunk);
            }
            let raw = String::from_utf8_lossy(&bytes).to_string();
            let mut result = ToolResult {
                tool_id: id.into(),
                inputs: inputs.clone(),
                status: "completed".into(),
                source_url: url.to_string(),
                retrieved_at: Utc::now().to_rfc3339(),
                observations: Value::Null,
                raw,
                error: None,
                cached: false,
                truncated,
            };
            if status.as_u16() == 404 {
                result.status = "no_results".into();
                return Ok(result);
            }
            if !status.is_success() {
                result.status = if status.as_u16() == 429 {
                    "rate_limited"
                } else {
                    "failed"
                }
                .into();
                result.error = Some(format!(
                    "HTTP {status}: {}",
                    result.raw.chars().take(250).collect::<String>()
                ));
                return Ok(result);
            }
            match parse_observations(id, &result.raw, &content_type, req.ndjson) {
                Ok((value, cut)) => {
                    result.observations = value;
                    result.truncated |= cut;
                }
                Err(e) => {
                    result.status = if e.to_string().contains("quota") {
                        "rate_limited"
                    } else {
                        "failed"
                    }
                    .into();
                    result.error = Some(e.to_string());
                    return Ok(result);
                }
            }
            if no_results(id, &result.observations) {
                result.status = "no_results".into();
            }
            return Ok(result);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registry_and_validation() {
        assert_eq!(registry().len(), 30);
        let ids: std::collections::HashSet<_> = registry().iter().map(|t| t.id).collect();
        assert_eq!(ids.len(), 30);
        assert_eq!(
            registry()
                .iter()
                .map(|t| t.category)
                .collect::<std::collections::HashSet<_>>()
                .len(),
            10
        );
        for t in registry() {
            assert!(!t.description.is_empty());
            assert!(!t.documentation.is_empty());
            assert_eq!(t.schema()["type"], "object");
            validate(t.id, &t.example_input()).unwrap_or_else(|e| panic!("{} example: {e}", t.id));
        }
        assert!(request("overpass_places", &json!({"latitude":95,"longitude":0})).is_err());
        assert!(request("shodan_internetdb", &json!({"ip":"127.0.0.1"})).is_ok());
    }
    #[test]
    fn parse_fixtures() {
        let (crt, _) = parse_observations(
            "crtsh_certificates",
            r#"[{"name_value":"a.example.com\n*.example.com"},{"name_value":"a.example.com"}]"#,
            "application/json",
            false,
        )
        .unwrap();
        assert_eq!(crt["hostnames"].as_array().unwrap().len(), 2);
        let (csv, _) = parse_observations(
            "hackertarget_hostsearch",
            "a.example.com,1.2.3.4\n",
            "text/plain",
            false,
        )
        .unwrap();
        assert_eq!(csv[0]["ip"], "1.2.3.4");
        assert!(parse_observations(
            "hackertarget_hostsearch",
            "API count exceeded",
            "text/plain",
            false
        )
        .is_err());
        let (ndjson, _) = parse_observations(
            "commoncrawl_urls",
            "{\"url\":\"https://example.com\"}\n",
            "text/plain",
            true,
        )
        .unwrap();
        assert_eq!(ndjson.as_array().unwrap().len(), 1);
        assert!(parse_observations("commoncrawl_urls", "not-json", "text/plain", true).is_err());
        assert!(parse_observations("nvd_cve", "<html>error</html>", "text/html", false).is_err());
        assert!(parse_observations("nvd_cve", "oops", "application/json", false).is_err());
    }
}
