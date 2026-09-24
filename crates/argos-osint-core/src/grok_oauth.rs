//! Grok Build's login, reused when Argos has no API key of its own.
//!
//! `grok login` writes an OIDC access token to `~/.grok/auth.json`. Argos
//! sends that token as the bearer for `api.x.ai`. A token inside its expiry
//! window is used as-is. An expired one is refreshed against the issuer and
//! written back into the same file so Grok stays signed in.

use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_json::{json, Value};

#[derive(Clone, Debug)]
struct Entry {
    scope_key: String,
    access_token: String,
    refresh_token: Option<String>,
    client_id: Option<String>,
    issuer: Option<String>,
    principal_type: Option<String>,
    principal_id: Option<String>,
    expires_at: Option<DateTime<Utc>>,
}

pub fn auth_path() -> PathBuf {
    if let Ok(path) = std::env::var("ARGOS_GROK_AUTH") {
        if !path.trim().is_empty() {
            return PathBuf::from(path);
        }
    }
    if let Ok(home) = std::env::var("GROK_HOME") {
        if !home.trim().is_empty() {
            return PathBuf::from(home).join("auth.json");
        }
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".grok")
        .join("auth.json")
}

/// Bearer token for the Grok provider. `None` when Grok has no login on this
/// machine. Errors are for a login that exists but cannot be refreshed.
pub async fn bearer() -> Result<Option<String>, String> {
    let path = auth_path();
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(_) => return Ok(None),
    };
    let value: Value = serde_json::from_str(&raw).map_err(|err| err.to_string())?;
    let Some(entry) = pick_entry(&value) else {
        return Ok(None);
    };
    if token_is_fresh(entry.expires_at, Utc::now()) {
        return Ok(Some(entry.access_token));
    }
    let refreshed = refresh(&entry).await?;
    write_back(&path, &entry.scope_key, &refreshed)?;
    Ok(Some(refreshed.access_token))
}

fn token_is_fresh(expires_at: Option<DateTime<Utc>>, now: DateTime<Utc>) -> bool {
    match expires_at {
        None => true,
        Some(when) => when > now + chrono::Duration::seconds(90),
    }
}

fn pick_entry(root: &Value) -> Option<Entry> {
    let obj = root.as_object()?;
    let mut best: Option<Entry> = None;
    for (scope_key, value) in obj {
        let Some(entry) = entry_from(scope_key, value) else {
            continue;
        };
        let better = match &best {
            None => true,
            Some(current) => {
                entry.expires_at.unwrap_or(DateTime::<Utc>::MIN_UTC)
                    > current.expires_at.unwrap_or(DateTime::<Utc>::MIN_UTC)
            }
        };
        if better {
            best = Some(entry);
        }
    }
    best
}

fn entry_from(scope_key: &str, value: &Value) -> Option<Entry> {
    let access_token = value
        .get("key")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if access_token.is_empty() {
        return None;
    }
    Some(Entry {
        scope_key: scope_key.to_string(),
        access_token: access_token.to_string(),
        refresh_token: text_field(value, "refresh_token"),
        client_id: text_field(value, "oidc_client_id"),
        issuer: text_field(value, "oidc_issuer"),
        principal_type: text_field(value, "principal_type"),
        principal_id: text_field(value, "principal_id"),
        expires_at: value
            .get("expires_at")
            .and_then(|v| v.as_str())
            .and_then(|text| DateTime::parse_from_rfc3339(text).ok())
            .map(|dt| dt.with_timezone(&Utc)),
    })
}

fn text_field(value: &Value, name: &str) -> Option<String> {
    value
        .get(name)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

struct Refreshed {
    access_token: String,
    refresh_token: Option<String>,
    expires_at: Option<DateTime<Utc>>,
}

async fn refresh(entry: &Entry) -> Result<Refreshed, String> {
    let refresh_token = entry
        .refresh_token
        .clone()
        .ok_or_else(|| "Grok login has expired. Run `grok login` again.".to_string())?;
    let issuer = entry
        .issuer
        .clone()
        .unwrap_or_else(|| "https://auth.x.ai".into());
    let client_id = entry.client_id.clone().ok_or_else(|| {
        "Grok login is missing its client id. Run `grok login` again.".to_string()
    })?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|err| err.to_string())?;
    let discovery_url = format!(
        "{}/.well-known/openid-configuration",
        issuer.trim_end_matches('/')
    );
    let discovery: Value = client
        .get(&discovery_url)
        .send()
        .await
        .map_err(|err| err.to_string())?
        .error_for_status()
        .map_err(|err| err.to_string())?
        .json()
        .await
        .map_err(|err| err.to_string())?;
    let token_endpoint = discovery
        .get("token_endpoint")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Grok issuer did not publish a token endpoint".to_string())?
        .to_string();
    let mut params = vec![
        ("grant_type".to_string(), "refresh_token".to_string()),
        ("refresh_token".to_string(), refresh_token),
        ("client_id".to_string(), client_id),
    ];
    if let Some(value) = &entry.principal_type {
        params.push(("principal_type".into(), value.clone()));
    }
    if let Some(value) = &entry.principal_id {
        params.push(("principal_id".into(), value.clone()));
    }
    let response = client
        .post(&token_endpoint)
        .form(&params)
        .send()
        .await
        .map_err(|err| err.to_string())?;
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "Grok login refresh failed ({status}). Run `grok login` again."
        ));
    }
    let tokens: Value = serde_json::from_str(&body).map_err(|err| err.to_string())?;
    let access_token = tokens
        .get("access_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Grok refresh returned no access token".to_string())?
        .to_string();
    let expires_at = tokens
        .get("expires_in")
        .and_then(|v| v.as_u64())
        .map(|seconds| Utc::now() + chrono::Duration::seconds(seconds as i64));
    Ok(Refreshed {
        access_token,
        refresh_token: tokens
            .get("refresh_token")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        expires_at,
    })
}

fn write_back(path: &Path, scope_key: &str, refreshed: &Refreshed) -> Result<(), String> {
    let raw = std::fs::read_to_string(path).unwrap_or_else(|_| "{}".into());
    let mut root: Value = serde_json::from_str(&raw).unwrap_or_else(|_| json!({}));
    let entry = root
        .as_object_mut()
        .ok_or_else(|| "Grok auth.json is not an object".to_string())?
        .entry(scope_key.to_string())
        .or_insert_with(|| json!({}));
    let obj = entry
        .as_object_mut()
        .ok_or_else(|| "Grok auth entry is not an object".to_string())?;
    obj.insert("key".into(), json!(refreshed.access_token));
    if let Some(refresh) = &refreshed.refresh_token {
        obj.insert("refresh_token".into(), json!(refresh));
    }
    if let Some(expires) = refreshed.expires_at {
        obj.insert(
            "expires_at".into(),
            json!(expires.to_rfc3339_opts(chrono::SecondsFormat::Micros, true)),
        );
    }
    let body = serde_json::to_string_pretty(&root).map_err(|err| err.to_string())?;
    crate::secrets::write_private(path, &body).map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_the_unexpired_grok_access_token() {
        let raw = r#"{
            "https://auth.x.ai::client": {
                "auth_mode": "oidc",
                "key": "eyJ.access",
                "refresh_token": "rt",
                "oidc_issuer": "https://auth.x.ai",
                "oidc_client_id": "client",
                "expires_at": "2099-01-01T00:00:00Z"
            }
        }"#;
        let value: Value = serde_json::from_str(raw).unwrap();
        let entry = pick_entry(&value).unwrap();
        assert_eq!(entry.access_token, "eyJ.access");
        assert!(token_is_fresh(entry.expires_at, Utc::now()));
    }

    #[test]
    fn expired_token_is_not_fresh() {
        let when = DateTime::parse_from_rfc3339("2020-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert!(!token_is_fresh(Some(when), Utc::now()));
    }
}
