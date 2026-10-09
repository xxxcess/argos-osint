//! Native Spotify signup-validation adapter. Protocol from source review, not copied.

use super::{parse_json, CheckSignal, ServiceResult};
use anyhow::Result;
use serde_json::Value;
use url::Url;

pub const ID: &str = "spotify";
pub const HOST: &str = "spclient.wg.spotify.com";
pub const ADAPTER_VERSION: &str = "1.0";

pub fn request_url(email: &str) -> Result<Url> {
    let mut url = Url::parse("https://spclient.wg.spotify.com/signup/public/v1/account")?;
    url.query_pairs_mut()
        .append_pair("validate", "1")
        .append_pair("email", email);
    Ok(url)
}

/// Exact integer status 20 → registered; 1 → not_registered.
pub fn parse_body(status: u16, body: &str) -> ServiceResult {
    if (300..400).contains(&status) {
        return ServiceResult::blocked("challenge redirect");
    }
    if status == 429 {
        return ServiceResult::rate_limited("HTTP 429");
    }
    if !(200..300).contains(&status) {
        return ServiceResult::error(format!("HTTP {status}"));
    }
    let value = match parse_json(body) {
        Ok(v) => v,
        Err(reason) => return ServiceResult::inconclusive(reason),
    };
    let code = match value.get("status") {
        Some(Value::Number(n)) => n.as_i64(),
        Some(Value::String(s)) => s.trim().parse::<i64>().ok(),
        _ => None,
    };
    match code {
        Some(20) => ServiceResult::ok(CheckSignal::Registered, "status=20"),
        Some(1) => ServiceResult::ok(CheckSignal::NotRegistered, "status=1"),
        Some(other) => ServiceResult::inconclusive(format!("unrecognized status {other}")),
        None => ServiceResult::inconclusive("missing integer status"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positive_negative_and_unknown() {
        assert_eq!(
            parse_body(200, r#"{"status":20}"#).signal,
            CheckSignal::Registered
        );
        assert_eq!(
            parse_body(200, r#"{"status":1}"#).signal,
            CheckSignal::NotRegistered
        );
        assert_eq!(
            parse_body(200, r#"{"status":8}"#).signal,
            CheckSignal::Inconclusive
        );
        assert_eq!(
            parse_body(200, r#"{"status":"nope"}"#).signal,
            CheckSignal::Inconclusive
        );
        assert_eq!(parse_body(429, "{}").signal, CheckSignal::RateLimited);
        assert_eq!(parse_body(500, "{}").signal, CheckSignal::Error);
        let url = request_url("ada@example.org").unwrap();
        assert_eq!(url.host_str(), Some("spclient.wg.spotify.com"));
        assert!(url.as_str().contains("validate=1"));
        assert!(!url.as_str().contains("recovery"));
    }
}
