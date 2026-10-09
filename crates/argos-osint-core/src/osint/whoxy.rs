//! Whoxy WHOIS history adapter (`whoxy_whois_history`).
//!
//! GET `https://api.whoxy.com/?key=SECRET&history=DOMAIN`. The key is bound at
//! request time and never stored on the tool input or evidence `source_url`.

use anyhow::{anyhow, ensure, Result};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::net::IpAddr;
use url::Url;

pub const TOOL_ID: &str = "whoxy_whois_history";
pub const HOST: &str = "api.whoxy.com";
pub const ENDPOINT: &str = "https://api.whoxy.com/";
pub const DEFAULT_LIMIT: u32 = 25;
pub const MAX_LIMIT: u32 = 100;
pub const PARSER_VERSION: &str = "1.0";

/// Normalize an IDNA domain. Rejects URLs, IPs, paths, and invalid labels.
pub fn normalize_domain(raw: &str) -> Result<String> {
    let s = raw.trim();
    ensure!(!s.is_empty(), "invalid domain");
    ensure!(
        !s.contains("://")
            && !s.contains('/')
            && !s.contains('?')
            && !s.contains('#')
            && !s.contains('@')
            && !s.contains('\\')
            && !s.contains(':'),
        "invalid domain"
    );
    let host_only = s.trim_end_matches('.');
    if host_only.parse::<IpAddr>().is_ok() {
        return Err(anyhow!("invalid domain"));
    }
    let parsed =
        Url::parse(&format!("https://{host_only}/")).map_err(|_| anyhow!("invalid domain"))?;
    ensure!(
        parsed.host_str().is_some() && parsed.path() == "/" && parsed.query().is_none(),
        "invalid domain"
    );
    let ascii = parsed
        .host_str()
        .ok_or_else(|| anyhow!("invalid domain"))?
        .trim_end_matches('.')
        .to_ascii_lowercase();
    ensure!(
        ascii.parse::<IpAddr>().is_err()
            && ascii.len() <= 253
            && ascii.contains('.')
            && ascii.split('.').all(|label| {
                !label.is_empty()
                    && label.len() <= 63
                    && label
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-')
                    && !label.starts_with('-')
                    && !label.ends_with('-')
            }),
        "invalid domain"
    );
    Ok(ascii)
}

fn parse_iso_date(raw: &str, field: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(raw.trim(), "%Y-%m-%d")
        .map_err(|_| anyhow!("invalid {field}: expected YYYY-MM-DD"))
}

fn query_date(query_time: &str) -> Option<NaiveDate> {
    let trimmed = query_time.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(date) = NaiveDate::parse_from_str(trimmed, "%Y-%m-%d") {
        return Some(date);
    }
    let date_part = trimmed.split(' ').next().unwrap_or(trimmed);
    NaiveDate::parse_from_str(date_part, "%Y-%m-%d").ok()
}

/// Inputs after validation. `from`/`to`/`limit` are view filters on a full-history fetch.
#[derive(Clone, Debug)]
pub struct HistoryQuery {
    pub domain: String,
    pub from: Option<NaiveDate>,
    pub to: Option<NaiveDate>,
    pub limit: u32,
}

pub fn parse_query(inputs: &Value) -> Result<HistoryQuery> {
    let domain = inputs
        .get("domain")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("missing or invalid domain"))?;
    let domain = normalize_domain(domain)?;
    let from = match inputs.get("from").and_then(Value::as_str) {
        Some(raw) if !raw.trim().is_empty() => Some(parse_iso_date(raw, "from")?),
        _ => None,
    };
    let to = match inputs.get("to").and_then(Value::as_str) {
        Some(raw) if !raw.trim().is_empty() => Some(parse_iso_date(raw, "to")?),
        _ => None,
    };
    if let (Some(from), Some(to)) = (from, to) {
        ensure!(from <= to, "from must be on or before to");
    }
    let limit = match inputs.get("limit") {
        None | Some(Value::Null) => DEFAULT_LIMIT,
        Some(Value::Number(n)) => {
            let n = n.as_u64().ok_or_else(|| anyhow!("invalid limit"))?;
            ensure!(
                n >= 1 && n <= u64::from(MAX_LIMIT),
                "limit must be 1–{MAX_LIMIT}"
            );
            n as u32
        }
        _ => return Err(anyhow!("invalid limit")),
    };
    Ok(HistoryQuery {
        domain,
        from,
        to,
        limit,
    })
}

/// History request without the API key. The executor binds `key` at send time.
pub fn history_request_url(domain: &str) -> Result<Url> {
    let domain = normalize_domain(domain)?;
    let mut url = Url::parse(ENDPOINT)?;
    url.query_pairs_mut().append_pair("history", &domain);
    Ok(url)
}

/// Account-balance request without the API key.
pub fn balance_request_url() -> Result<Url> {
    let mut url = Url::parse(ENDPOINT)?;
    url.query_pairs_mut().append_pair("account", "balance");
    Ok(url)
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContactCard {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub full_name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub company_name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub email_address: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub country_name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct WhoisSnapshot {
    pub query_time: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query_date: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub domain_name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub create_date: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub update_date: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub expiry_date: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub registrar: String,
    #[serde(default)]
    pub nameservers: Vec<String>,
    #[serde(default)]
    pub status: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registrant: Option<ContactCard>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub administrative: Option<ContactCard>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub technical: Option<ContactCard>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdjacentChange {
    pub from_query_time: String,
    pub to_query_time: String,
    pub fields: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct HistoryObservation {
    pub domain: String,
    pub parser_version: String,
    pub total_records_found: u64,
    pub displayed: usize,
    pub omitted: usize,
    pub undated: usize,
    pub zero_history: bool,
    pub snapshots: Vec<WhoisSnapshot>,
    pub changes: Vec<AdjacentChange>,
}

fn text_field(value: &Value, keys: &[&str]) -> String {
    for key in keys {
        if let Some(text) = value.get(*key).and_then(Value::as_str) {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
    }
    String::new()
}

fn string_list(value: &Value, key: &str) -> Vec<String> {
    match value.get(key) {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string)
            .collect(),
        Some(Value::String(s)) => s
            .split([',', ' ', '\n'])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

fn contact(value: &Value, key: &str) -> Option<ContactCard> {
    let object = value.get(key)?.as_object()?;
    let card = ContactCard {
        full_name: text_field(&Value::Object(object.clone()), &["full_name", "name"]),
        company_name: text_field(
            &Value::Object(object.clone()),
            &["company_name", "organization", "org"],
        ),
        email_address: text_field(&Value::Object(object.clone()), &["email_address", "email"]),
        country_name: text_field(&Value::Object(object.clone()), &["country_name", "country"]),
    };
    if card.full_name.is_empty()
        && card.company_name.is_empty()
        && card.email_address.is_empty()
        && card.country_name.is_empty()
    {
        None
    } else {
        Some(card)
    }
}

fn snapshot_from_record(record: &Value) -> WhoisSnapshot {
    let query_time = text_field(record, &["query_time"]);
    let query_date = query_date(&query_time).map(|d| d.format("%Y-%m-%d").to_string());
    let registrar = record
        .get("domain_registrar")
        .map(|reg| text_field(reg, &["registrar_name", "name"]))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| text_field(record, &["registrar"]));
    WhoisSnapshot {
        query_time,
        query_date,
        domain_name: text_field(record, &["domain_name"]),
        create_date: text_field(record, &["create_date"]),
        update_date: text_field(record, &["update_date"]),
        expiry_date: text_field(record, &["expiry_date"]),
        registrar,
        nameservers: string_list(record, "name_servers"),
        status: string_list(record, "domain_status"),
        registrant: contact(record, "registrant_contact"),
        administrative: contact(record, "administrative_contact"),
        technical: contact(record, "technical_contact"),
    }
}

fn fingerprint(snapshot: &WhoisSnapshot) -> String {
    format!(
        "{}|{}|{}|{}|{}|{}|{}",
        snapshot.registrar,
        snapshot.create_date,
        snapshot.update_date,
        snapshot.expiry_date,
        snapshot.nameservers.join(","),
        snapshot.status.join(","),
        snapshot
            .registrant
            .as_ref()
            .map(|c| format!("{}|{}|{}", c.full_name, c.company_name, c.email_address))
            .unwrap_or_default()
    )
}

fn adjacent_changes(snapshots: &[WhoisSnapshot]) -> Vec<AdjacentChange> {
    let mut changes = Vec::new();
    for pair in snapshots.windows(2) {
        let older = &pair[0];
        let newer = &pair[1];
        let mut fields = Vec::new();
        if older.registrar != newer.registrar {
            fields.push("registrar".into());
        }
        if older.create_date != newer.create_date {
            fields.push("create_date".into());
        }
        if older.update_date != newer.update_date {
            fields.push("update_date".into());
        }
        if older.expiry_date != newer.expiry_date {
            fields.push("expiry_date".into());
        }
        if older.nameservers != newer.nameservers {
            fields.push("nameservers".into());
        }
        if older.status != newer.status {
            fields.push("status".into());
        }
        if older.registrant != newer.registrant {
            fields.push("registrant".into());
        }
        if older.administrative != newer.administrative {
            fields.push("administrative".into());
        }
        if older.technical != newer.technical {
            fields.push("technical".into());
        }
        if fingerprint(older) != fingerprint(newer) && fields.is_empty() {
            fields.push("record".into());
        }
        if !fields.is_empty() {
            changes.push(AdjacentChange {
                from_query_time: older.query_time.clone(),
                to_query_time: newer.query_time.clone(),
                fields,
            });
        }
    }
    changes
}

fn json_status(value: &Value) -> Result<i64> {
    match value.get("status") {
        Some(Value::Number(n)) => n
            .as_i64()
            .ok_or_else(|| anyhow!("malformed Whoxy envelope: status")),
        Some(Value::String(s)) => s
            .trim()
            .parse::<i64>()
            .map_err(|_| anyhow!("malformed Whoxy envelope: status")),
        _ => Err(anyhow!("malformed Whoxy envelope: missing status")),
    }
}

/// Parse a Whoxy JSON envelope. Incomplete or error envelopes fail; they must
/// not become an empty-history cache hit.
pub fn parse_history_envelope(raw: &Value, domain: &str) -> Result<HistoryObservation> {
    let status = json_status(raw)?;
    if status == 0 {
        let message = raw
            .get("status_reason")
            .or_else(|| raw.get("error"))
            .or_else(|| raw.get("message"))
            .and_then(Value::as_str)
            .unwrap_or("Whoxy error");
        let lower = message.to_ascii_lowercase();
        if lower.contains("quota")
            || lower.contains("limit")
            || lower.contains("insufficient")
            || lower.contains("balance")
        {
            return Err(anyhow!("Whoxy quota: {message}"));
        }
        return Err(anyhow!("Whoxy error: {message}"));
    }
    ensure!(status == 1, "malformed Whoxy envelope: status {status}");
    let records = raw
        .get("whois_records")
        .cloned()
        .unwrap_or(Value::Array(Vec::new()));
    let records = records
        .as_array()
        .ok_or_else(|| anyhow!("malformed Whoxy envelope: whois_records"))?;
    let total = raw
        .get("total_records_found")
        .and_then(Value::as_u64)
        .unwrap_or(records.len() as u64);
    let mut snapshots: Vec<WhoisSnapshot> = records.iter().map(snapshot_from_record).collect();
    snapshots.sort_by(
        |a, b| match (query_date(&a.query_time), query_date(&b.query_time)) {
            (Some(left), Some(right)) => left.cmp(&right).then(a.query_time.cmp(&b.query_time)),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.query_time.cmp(&b.query_time),
        },
    );
    let undated = snapshots
        .iter()
        .filter(|s| query_date(&s.query_time).is_none())
        .count();
    let changes = adjacent_changes(&snapshots);
    Ok(HistoryObservation {
        domain: domain.to_string(),
        parser_version: PARSER_VERSION.into(),
        total_records_found: total,
        displayed: snapshots.len(),
        omitted: 0,
        undated,
        zero_history: snapshots.is_empty() && total == 0,
        snapshots,
        changes,
    })
}

/// Project a cached full-history observation through from/to/limit filters.
pub fn project_history(full: &HistoryObservation, query: &HistoryQuery) -> HistoryObservation {
    let mut dated = Vec::new();
    let mut undated = Vec::new();
    for snapshot in &full.snapshots {
        match query_date(&snapshot.query_time) {
            Some(date) => {
                if query.from.is_some_and(|from| date < from) {
                    continue;
                }
                if query.to.is_some_and(|to| date > to) {
                    continue;
                }
                dated.push(snapshot.clone());
            }
            None => undated.push(snapshot.clone()),
        }
    }
    let undated_count = undated.len();
    let mut selected = dated;
    let limit = query.limit as usize;
    let omitted = selected.len().saturating_sub(limit);
    if selected.len() > limit {
        selected = selected[selected.len() - limit..].to_vec();
    }
    let changes = adjacent_changes(&selected);
    HistoryObservation {
        domain: query.domain.clone(),
        parser_version: PARSER_VERSION.into(),
        total_records_found: full.total_records_found,
        displayed: selected.len(),
        omitted,
        undated: undated_count,
        zero_history: full.zero_history && selected.is_empty(),
        snapshots: selected,
        changes,
    }
}

pub fn bounded_model_view(observation: &HistoryObservation) -> Value {
    json!({
        "domain": observation.domain,
        "parser_version": observation.parser_version,
        "total_records_found": observation.total_records_found,
        "displayed": observation.displayed,
        "omitted": observation.omitted,
        "undated": observation.undated,
        "zero_history": observation.zero_history,
        "snapshots": observation.snapshots,
        "changes": observation.changes,
        "note": "query_time is the observation date, not a registration or transfer date. A registrar change or redaction is not proof of ownership change.",
    })
}

/// Prepaid lookup cost: 1 on nonempty history, 0 on a recognized empty success.
pub fn reported_lookup_cost(observation: &HistoryObservation) -> u32 {
    if observation.zero_history || observation.total_records_found == 0 {
        0
    } else {
        1
    }
}

pub fn parse_balance(raw: &Value) -> Result<u32> {
    let status = json_status(raw)?;
    if status == 0 {
        let message = raw
            .get("status_reason")
            .or_else(|| raw.get("error"))
            .and_then(Value::as_str)
            .unwrap_or("Whoxy error");
        let lower = message.to_ascii_lowercase();
        if lower.contains("auth") || lower.contains("invalid") || lower.contains("key") {
            return Err(anyhow!("Whoxy authentication failed: {message}"));
        }
        return Err(anyhow!("Whoxy error: {message}"));
    }
    ensure!(status == 1, "malformed Whoxy balance envelope");
    let live = raw
        .get("live_whois_balance")
        .or_else(|| raw.get("whois_balance"))
        .or_else(|| raw.get("balance"));
    let history = raw.get("whois_history_balance").or(live);
    let value = history
        .and_then(Value::as_u64)
        .or_else(|| {
            history
                .and_then(Value::as_str)
                .and_then(|s| s.trim().parse::<u64>().ok())
        })
        .ok_or_else(|| anyhow!("malformed Whoxy balance envelope"))?;
    u32::try_from(value).map_err(|_| anyhow!("Whoxy balance out of range"))
}

/// Live account=balance check. Distinguishes authentication failure from a zero balance.
pub async fn check_balance(key: &str, user_agent: &str) -> Result<u32> {
    let key = key.trim();
    ensure!(!key.is_empty(), "Enter a Whoxy API key");
    let mut url = balance_request_url()?;
    url.query_pairs_mut().append_pair("key", key);
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let response = client
        .get(url)
        .header(reqwest::header::USER_AGENT, user_agent)
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await?;
    if response.status().is_redirection() {
        return Err(anyhow!("Whoxy refused a redirect"));
    }
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    let body = if body.contains(key) {
        body.replace(key, "[redacted]")
    } else {
        body
    };
    if !status.is_success() {
        if status.as_u16() == 401 || status.as_u16() == 403 {
            return Err(anyhow!("Whoxy authentication failed"));
        }
        return Err(anyhow!("Whoxy HTTP {status}"));
    }
    let value: Value =
        serde_json::from_str(&body).map_err(|_| anyhow!("malformed Whoxy balance envelope"))?;
    parse_balance(&value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const SAMPLE: &str = r#"{
  "status": 1,
  "api_query": "whois_history",
  "total_records_found": 4,
  "whois_records": [
    {"num":1,"domain_name":"koyz.com","query_time":"2014-09-23 13:45:05","create_date":"2006-03-31","update_date":"2014-07-01","expiry_date":"2015-03-31","domain_registrar":{"registrar_name":"GoDaddy.com, LLC"},"registrant_contact":{"full_name":"Senthil Kumar","company_name":"This domain is for sale !!","email_address":"domain@sdexperts.com"},"name_servers":["ns01.cashparking.com","ns02.cashparking.com"],"domain_status":["clientDeleteProhibited"]},
    {"num":2,"domain_name":"koyz.com","query_time":"2015-04-01 00:05:04","create_date":"2006-03-31","update_date":"2015-01-14","expiry_date":"2015-03-31","domain_registrar":{"registrar_name":"GoDaddy.com, LLC"},"name_servers":["buy.internettraffic.com","sell.internettraffic.com"]},
    {"num":3,"domain_name":"koyz.com","query_time":"2015-07-07 20:30:05","create_date":"2006-03-31","update_date":"2015-04-02","expiry_date":"2016-03-31","domain_registrar":{"registrar_name":"GoDaddy.com, LLC"},"name_servers":["buy.internettraffic.com","sell.internettraffic.com"]},
    {"num":4,"domain_name":"koyz.com","query_time":"2015-07-19 06:59:22","create_date":"2006-03-31","update_date":"2015-07-15","expiry_date":"2016-03-31","domain_registrar":{"registrar_name":"GoDaddy.com, LLC"},"name_servers":["ns1.bodis.com","ns2.bodis.com"]}
  ]
}"#;

    #[test]
    fn idna_and_rejects() {
        assert_eq!(normalize_domain("Example.ORG.").unwrap(), "example.org");
        assert_eq!(normalize_domain("münchen.de").unwrap(), "xn--mnchen-3ya.de");
        assert!(normalize_domain("https://example.org").is_err());
        assert!(normalize_domain("example.org/path").is_err());
        assert!(normalize_domain("8.8.8.8").is_err());
        assert!(normalize_domain("localhost").is_err());
        assert!(normalize_domain("").is_err());
    }

    #[test]
    fn envelope_success_empty_and_error() {
        let empty = json!({"status":1,"total_records_found":0,"whois_records":[]});
        let obs = parse_history_envelope(&empty, "example.org").unwrap();
        assert!(obs.zero_history);
        assert_eq!(reported_lookup_cost(&obs), 0);

        let err = json!({"status":0,"status_reason":"Invalid API key"});
        assert!(parse_history_envelope(&err, "example.org")
            .unwrap_err()
            .to_string()
            .contains("Whoxy error"));

        let quota = json!({"status":0,"status_reason":"Insufficient query balance"});
        assert!(parse_history_envelope(&quota, "example.org")
            .unwrap_err()
            .to_string()
            .contains("quota"));

        assert!(parse_history_envelope(&json!({"whois_records":[]}), "example.org").is_err());
    }

    #[test]
    fn sample_diffs_and_local_filters() {
        let raw: Value = serde_json::from_str(SAMPLE).unwrap();
        let full = parse_history_envelope(&raw, "koyz.com").unwrap();
        assert_eq!(full.total_records_found, 4);
        assert_eq!(full.snapshots.len(), 4);
        assert!(full.snapshots[0].query_time < full.snapshots[3].query_time);
        assert!(!full.changes.is_empty());
        assert_eq!(reported_lookup_cost(&full), 1);

        let query = HistoryQuery {
            domain: "koyz.com".into(),
            from: Some(NaiveDate::from_ymd_opt(2015, 1, 1).unwrap()),
            to: Some(NaiveDate::from_ymd_opt(2015, 12, 31).unwrap()),
            limit: 2,
        };
        let view = project_history(&full, &query);
        assert_eq!(view.displayed, 2);
        assert_eq!(view.omitted, 1);
        assert!(view
            .snapshots
            .iter()
            .all(|s| s.query_time.starts_with("2015-")));
    }

    #[test]
    fn undated_records_are_reported() {
        let raw = json!({
            "status": 1,
            "total_records_found": 2,
            "whois_records": [
                {"query_time": "2015-01-01 00:00:00", "domain_registrar": {"registrar_name": "A"}},
                {"domain_name": "example.org"}
            ]
        });
        let obs = parse_history_envelope(&raw, "example.org").unwrap();
        assert_eq!(obs.undated, 1);
        let query = parse_query(&json!({"domain":"example.org","limit":1})).unwrap();
        let view = project_history(&obs, &query);
        assert_eq!(view.displayed, 1);
        assert_eq!(view.undated, 1);
    }

    #[test]
    fn date_and_limit_validation() {
        assert!(parse_query(&json!({"domain":"example.org","from":"nope"})).is_err());
        assert!(parse_query(
            &json!({"domain":"example.org","from":"2020-02-02","to":"2020-01-01"})
        )
        .is_err());
        assert!(parse_query(&json!({"domain":"example.org","limit":0})).is_err());
        assert!(parse_query(&json!({"domain":"example.org","limit":101})).is_err());
        let q = parse_query(&json!({"domain":"Example.ORG"})).unwrap();
        assert_eq!(q.domain, "example.org");
        assert_eq!(q.limit, DEFAULT_LIMIT);
    }

    #[test]
    fn balance_auth_vs_zero() {
        let zero = json!({"status":1,"live_whois_balance":0,"whois_history_balance":0});
        assert_eq!(parse_balance(&zero).unwrap(), 0);
        let bad = json!({"status":0,"status_reason":"Invalid API Key"});
        assert!(parse_balance(&bad)
            .unwrap_err()
            .to_string()
            .contains("authentication"));
    }
}
