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
    pub research: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub text: Option<ProviderSecret>,
    #[serde(default)]
    pub voice: Option<ProviderSecret>,
    #[serde(default)]
    pub gmail: Option<GmailSecret>,
    /// Independent provider accounts. Legacy text/voice slots still round-trip.
    #[serde(default)]
    pub accounts: std::collections::BTreeMap<String, ProviderSecret>,
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
    pub fn redact(&self, text: &str) -> String {
        let mut result = text.to_string();
        for secret in self
            .research
            .values()
            .map(String::as_str)
            .chain(self.accounts.values().filter_map(|s| s.api_key.as_deref()))
            .chain(self.text.iter().filter_map(|s| s.api_key.as_deref()))
            .chain(self.voice.iter().filter_map(|s| s.api_key.as_deref()))
            .chain(self.gmail.iter().map(|s| s.app_password.as_str()))
        {
            if !secret.is_empty() {
                result = result.replace(secret, "[redacted]");
            }
        }
        result
    }
    /// Resolve an account without copying another vendor's credentials.
    pub fn account(&self, kind: &str) -> Option<ProviderSecret> {
        let kind = crate::provider::normalize_kind(kind);
        self.accounts.get(&kind).cloned().or_else(|| {
            self.text
                .as_ref()
                .filter(|secret| crate::provider::effective_kind(secret) == kind)
                .cloned()
        })
    }

    pub fn set_account(&mut self, secret: ProviderSecret) {
        let kind = crate::provider::effective_kind(&secret);
        if let Some(legacy) = &self.text {
            self.accounts
                .entry(crate::provider::effective_kind(legacy))
                .or_insert_with(|| legacy.clone());
        }
        // Preserve the existing default connection for CLI and legacy configs.
        if self
            .text
            .as_ref()
            .is_some_and(|old| crate::provider::effective_kind(old) == kind)
        {
            self.text = Some(secret.clone());
        }
        self.accounts.insert(kind, secret);
    }

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
        self.save_to(&auth_path())
    }

    pub fn save_to(&self, path: &Path) -> Result<()> {
        write_private(path, &serde_json::to_string_pretty(self)?)
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

    #[test]
    fn legacy_connection_migrates_without_overwriting_other_accounts() {
        let legacy = r#"{"text":{"kind":"grok","base_url":"https://api.x.ai/v1","model":"grok-custom","api_key":"xai-existing"},"voice":null,"gmail":{"email":"test@example.com","app_password":"existing-mail"}}"#;
        let mut auth: AuthFile = serde_json::from_str(legacy).unwrap();
        let mut router = crate::provider::account_secret(&auth, "openrouter");
        assert!(router.api_key.is_none());
        router.api_key = Some("router-existing".into());
        auth.set_account(router);
        assert_eq!(
            auth.account("grok").unwrap().api_key.as_deref(),
            Some("xai-existing")
        );
        assert_eq!(
            auth.account("openrouter").unwrap().api_key.as_deref(),
            Some("router-existing")
        );
        assert!(auth.account("openai").is_none());
        assert!(auth.account("openai-chatgpt").is_none());
        assert_eq!(auth.gmail.as_ref().unwrap().app_password, "existing-mail");

        // A legacy CLI login may change text, but cannot discard its previous account.
        auth.text = auth.account("openrouter");
        let restored: AuthFile =
            serde_json::from_str(&serde_json::to_string(&auth).unwrap()).unwrap();
        assert_eq!(restored.account("grok").unwrap().model, "grok-custom");
        assert_eq!(
            restored.account("openrouter").unwrap().api_key.as_deref(),
            Some("router-existing")
        );
    }

    #[test]
    fn accounts_persist_with_owner_only_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        let mut auth = AuthFile::default();
        let mut grok = crate::provider::account_secret(&auth, "grok");
        grok.api_key = Some("xai-existing".into());
        auth.set_account(grok);
        auth.save_to(&path).unwrap();
        let restored: AuthFile =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            restored.account("grok").unwrap().api_key.as_deref(),
            Some("xai-existing")
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
}
