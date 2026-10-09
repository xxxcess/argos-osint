//! Native Pinterest email-exists adapter. Protocol from source review, not copied.

use super::{parse_json, CheckSignal, ServiceResult};
use anyhow::Result;
use serde_json::{json, Value};
use url::Url;

pub const ID: &str = "pinterest";
pub const HOST: &str = "www.pinterest.com";
pub const ADAPTER_VERSION: &str = "1.0";

pub fn request_url(email: &str) -> Result<Url> {
    let data = json!({
        "options": {"email": email, "context": {}},
        "context": {}
    });
    let mut url = Url::parse("https://www.pinterest.com/_ngjs/resource/EmailExistsResource/get/")?;
    url.query_pairs_mut()
        .append_pair("source_url", "/")
        .append_pair("data", &data.to_string());
    Ok(url)
}

/// Exact boolean `resource_response.data` → registered/not_registered.
/// An object containing `source_field` is diagnostic, not negative evidence.
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
    let data = value.pointer("/resource_response/data");
    match data {
        Some(Value::Bool(true)) => ServiceResult::ok(CheckSignal::Registered, "data=true"),
        Some(Value::Bool(false)) => ServiceResult::ok(CheckSignal::NotRegistered, "data=false"),
        Some(Value::Object(map)) if map.contains_key("source_field") => {
            ServiceResult::inconclusive("resource_response.data contains source_field")
        }
        Some(_) => ServiceResult::inconclusive("resource_response.data is not a boolean"),
        None => ServiceResult::inconclusive("missing resource_response.data"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positive_negative_and_source_field() {
        assert_eq!(
            parse_body(200, r#"{"resource_response":{"data":true}}"#).signal,
            CheckSignal::Registered
        );
        assert_eq!(
            parse_body(200, r#"{"resource_response":{"data":false}}"#).signal,
            CheckSignal::NotRegistered
        );
        assert_eq!(
            parse_body(
                200,
                r#"{"resource_response":{"data":{"source_field":"email"}}}"#
            )
            .signal,
            CheckSignal::Inconclusive
        );
        assert_eq!(parse_body(429, "{}").signal, CheckSignal::RateLimited);
        assert_eq!(parse_body(403, "{}").signal, CheckSignal::Error);
        let url = request_url("ada@example.org").unwrap();
        assert_eq!(url.host_str(), Some("www.pinterest.com"));
        assert!(url.as_str().contains("EmailExistsResource"));
        assert!(!url.as_str().contains("signup"));
        assert!(!url.as_str().contains("recovery"));
    }
}
