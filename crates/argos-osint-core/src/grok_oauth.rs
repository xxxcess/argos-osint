//! Grok Build's subscription login, reused without API-key fallback.
//!
//! `grok login` writes an OIDC access token to `~/.grok/auth.json`. Argos
//! sends that token as the bearer for `api.x.ai`. A token inside its expiry
//! window is used as-is. An expired one is refreshed against the issuer and
//! written back into the same file so Grok stays signed in.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

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

/// True when a Grok subscription login is stored. An expired token still
/// counts; the next model request refreshes it. An API-key entry does not.
pub fn login_present() -> bool {
    login_present_at(&auth_path())
}

fn login_present_at(path: &Path) -> bool {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return false;
    };
    serde_json::from_str::<Value>(&raw)
        .ok()
        .is_some_and(|value| pick_entry(&value).is_some())
}

/// Bearer token for the Grok provider. `None` when Grok has no login on this
/// machine. Errors are for a login that exists but cannot be refreshed.
pub async fn bearer() -> Result<Option<String>, String> {
    let path = auth_path();
    bearer_from_path(&path).await
}

async fn bearer_from_path(path: &Path) -> Result<Option<String>, String> {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(format!("Cannot read Grok sign-in: {err}")),
    };
    let value: Value = serde_json::from_str(&raw).map_err(|err| err.to_string())?;
    let Some(entry) = pick_entry(&value) else {
        return Ok(None);
    };
    if token_is_fresh(entry.expires_at, Utc::now()) {
        return Ok(Some(entry.access_token));
    }
    let refreshed = refresh(&entry).await?;
    write_back(path, &entry.scope_key, &refreshed)?;
    Ok(Some(refreshed.access_token))
}

pub async fn check_login() -> Result<String> {
    match bearer().await.map_err(anyhow::Error::msg)? {
        Some(_) => Ok("Grok subscription login ready".into()),
        None => Err(anyhow!("Grok subscription sign-in required. Select Sign in with Grok, or run `grok login --oauth` and Check existing login. API-key logins do not count as subscription access.")),
    }
}

fn login_command() -> Command {
    let mut cmd = Command::new("grok");
    cmd.args(["login", "--oauth"])
        .env_remove("XAI_API_KEY")
        .env_remove("GROK_API_KEY")
        .kill_on_drop(true);
    cmd
}

/// Grok Build owns the OAuth flow and credential file. Argos displays only
/// the browser sign-in instructions, then reuses its existing token adapter.
pub async fn login(mut on_progress: impl FnMut(&str)) -> Result<String> {
    let mut child = login_command()
        // Keep stdin open while the browser callback is pending. A closed
        // stdin can end the CLI's optional paste prompt before OAuth returns.
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Install Grok Build CLI on PATH to sign in with your Grok subscription")?;
    let mut stdout = BufReader::new(child.stdout.take().context("Grok stdout")?).lines();
    let mut stderr = BufReader::new(child.stderr.take().context("Grok stderr")?).lines();
    let ansi = regex::Regex::new(r"\x1b\[[0-9;]*m")?;
    tokio::time::timeout(Duration::from_secs(300), async {
        let (mut out_done, mut err_done) = (false, false);
        let mut failure_detail = String::new();
        while !out_done || !err_done {
            let (is_out, line) = tokio::select! {
                line = stdout.next_line(), if !out_done => (true, line?),
                line = stderr.next_line(), if !err_done => (false, line?),
            };
            match line {
                Some(line) if !line.trim().is_empty() => {
                    let line = ansi.replace_all(&line, "");
                    // OAuth URLs can exceed 300 characters. Keep the complete
                    // bounded URL so copying it does not break authentication.
                    on_progress(&line.chars().take(4096).collect::<String>());
                    if !line.contains("https://") && !line.contains("http://") {
                        failure_detail = line.chars().take(300).collect();
                    }
                }
                None if is_out => out_done = true,
                None => err_done = true,
                _ => {}
            }
        }
        if !child.wait().await?.success() {
            return Err(anyhow!("{}", login_failure(&failure_detail)));
        }
        check_login().await
    })
    .await
    .context("Grok sign-in timed out; select Sign in to try again")?
}

fn login_failure(detail: &str) -> String {
    let reason = if detail.trim().is_empty() {
        String::new()
    } else {
        format!(" {detail}")
    };
    format!("Grok browser sign-in did not complete.{reason} If signed in elsewhere, select Check existing login. Otherwise run `grok login --oauth` in a terminal.")
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
    // A saved API key must never be treated as subscription authentication.
    let mode = value.get("auth_mode").and_then(Value::as_str);
    if mode != Some("oidc")
        && !(mode.is_none()
            && text_field(value, "oidc_issuer").is_some()
            && text_field(value, "oidc_client_id").is_some())
    {
        return None;
    }
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
    fn subscription_login_forces_oauth_and_removes_api_environment() {
        let command = login_command();
        let command = command.as_std();
        let args: Vec<_> = command.get_args().map(|a| a.to_string_lossy()).collect();
        assert_eq!(args, ["login", "--oauth"]);
        for name in ["XAI_API_KEY", "GROK_API_KEY"] {
            assert!(command
                .get_envs()
                .any(|(key, value)| key == name && value.is_none()));
        }
    }

    #[test]
    fn failed_login_keeps_the_cli_reason_and_recovery_action() {
        let error = login_failure("OAuth callback timed out");
        assert!(error.contains("OAuth callback timed out"));
        assert!(error.contains("Check existing login"));
    }

    #[tokio::test]
    async fn subscription_tokens_exclude_api_logins_and_surface_file_errors() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join("auth.json");
        assert_eq!(bearer_from_path(&path).await.unwrap(), None);
        assert!(!login_present_at(&path));
        std::fs::write(&path, r#"{"api":{"auth_mode":"api_key","key":"xai-test"}}"#).unwrap();
        assert_eq!(bearer_from_path(&path).await.unwrap(), None);
        assert!(!login_present_at(&path));
        std::fs::write(&path, r#"{
            "api":{"auth_mode":"api_key","key":"xai-test","expires_at":"2100-01-01T00:00:00Z"},
            "oauth":{"auth_mode":"oidc","key":"subscription-test","expires_at":"2099-01-01T00:00:00Z"}
        }"#).unwrap();
        assert_eq!(
            bearer_from_path(&path).await.unwrap().as_deref(),
            Some("subscription-test")
        );
        assert!(login_present_at(&path));
        std::fs::write(&path, "invalid json").unwrap();
        assert!(!login_present_at(&path));
        assert!(bearer_from_path(&path).await.is_err());
        assert!(bearer_from_path(home.path())
            .await
            .unwrap_err()
            .contains("Cannot read"));
    }

    #[test]
    fn legacy_oidc_metadata_is_supported_but_bare_keys_are_rejected() {
        assert!(entry_from("legacy", &json!({"key":"subscription-test", "oidc_issuer":"https://auth.x.ai", "oidc_client_id":"client"})).is_some());
        assert!(entry_from("api", &json!({"key":"xai-test"})).is_none());
        assert!(entry_from("api", &json!({"auth_mode":"api_key", "key":"xai-test", "oidc_issuer":"https://auth.x.ai", "oidc_client_id":"client"})).is_none());
    }

    #[test]
    fn expired_token_is_not_fresh() {
        let when = DateTime::parse_from_rfc3339("2020-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert!(!token_is_fresh(Some(when), Utc::now()));
    }
}
