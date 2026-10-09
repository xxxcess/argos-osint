//! Native Twitter email-availability adapter. Protocol from source review, not copied.

use super::{parse_json, CheckSignal, ServiceResult};
use anyhow::Result;
use serde_json::Value;
use url::Url;

pub const ID: &str = "twitter";
pub const HOST: &str = "api.twitter.com";
pub const ADAPTER_VERSION: &str = "1.0";

pub fn request_url(email: &str) -> Result<Url> {
    let mut url = Url::parse("https://api.twitter.com/i/users/email_available.json")?;
    url.query_pairs_mut().append_pair("email", email);
    Ok(url)
}

/// Exact JSON boolean `taken`: true/false → registered/not_registered.
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
    match value.get("taken") {
        Some(Value::Bool(true)) => ServiceResult::ok(CheckSignal::Registered, "taken=true"),
        Some(Value::Bool(false)) => ServiceResult::ok(CheckSignal::NotRegistered, "taken=false"),
        Some(_) => ServiceResult::inconclusive("taken is not a boolean"),
        None => ServiceResult::inconclusive("missing taken"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positive_negative_and_unknown() {
        assert_eq!(
            parse_body(200, r#"{"taken":true}"#).signal,
            CheckSignal::Registered
        );
        assert_eq!(
            parse_body(200, r#"{"taken":false}"#).signal,
            CheckSignal::NotRegistered
        );
        assert_eq!(
            parse_body(200, r#"{"taken":"yes"}"#).signal,
            CheckSignal::Inconclusive
        );
        assert_eq!(parse_body(200, r#"{}"#).signal, CheckSignal::Inconclusive);
        assert_eq!(parse_body(429, "{}").signal, CheckSignal::RateLimited);
        assert_eq!(parse_body(503, "{}").signal, CheckSignal::Error);
        assert_eq!(parse_body(302, "").signal, CheckSignal::Blocked);
        let url = request_url("ada@example.org").unwrap();
        assert_eq!(url.host_str(), Some("api.twitter.com"));
        assert!(url.as_str().contains("email_available.json"));
        assert!(!url.as_str().contains("signup"));
        assert!(!url.as_str().contains("recovery"));
    }
}
