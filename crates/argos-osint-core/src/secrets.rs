//! Provider and Gmail secrets. `auth.json` is owner-only on Unix (0600),
//! same convention Grok uses for `~/.grok/auth.json`.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::paths::{auth_path, ensure_home};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AuthFile {
    #[serde(default)]
    pub text: Option<ProviderSecret>,
    #[serde(default)]
    pub voice: Option<ProviderSecret>,
    #[serde(default)]
    pub gmail: Option<GmailSecret>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProviderSecret {
    pub kind: String,
    pub base_url: String,
    pub model: String,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub stt_model: Option<String>,
    #[serde(default)]
    pub device: Option<DeviceEndpoints>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeviceEndpoints {
    pub client_id: String,
    pub device_auth_url: String,
    pub token_url: String,
    #[serde(default = "default_scope")]
    pub scope: String,
}

fn default_scope() -> String {
    "openid profile email".into()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GmailSecret {
    pub email: String,
    pub app_password: String,
}

impl AuthFile {
    pub fn load() -> Result<Self> {
        let path = auth_path();
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        if raw.trim().is_empty() {
            return Ok(Self::default());
        }
        Ok(serde_json::from_str(&raw)?)
    }

    pub fn save(&self) -> Result<()> {
        ensure_home()?;
        let path = auth_path();
        write_private(&path, &serde_json::to_string_pretty(self)?)
    }
}

pub fn write_private(path: &Path, body: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, body)?;
    owner_only(&tmp);
    fs::rename(&tmp, path)?;
    owner_only(path);
    Ok(())
}

fn owner_only(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
}

pub fn mask(secret: &str) -> String {
    if secret.is_empty() {
        return "(empty)".into();
    }
    let n = secret.chars().count();
    if n <= 4 {
        return "••••".into();
    }
    format!(
        "••••{}",
        secret
            .chars()
            .rev()
            .take(4)
            .collect::<String>()
            .chars()
            .rev()
            .collect::<String>()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_keeps_tail() {
        assert_eq!(mask("xai-secret-key1"), "••••key1");
        assert_eq!(mask(""), "(empty)");
    }
}
