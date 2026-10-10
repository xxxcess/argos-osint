//! Provider secrets. `auth.json` is owner-only on Unix (0600),
//! same convention Grok uses for `~/.grok/auth.json`.
//!
//! Writes route through the configuration commit store (see
//! [`crate::config_commit`]): a unique sibling temporary file is created 0600 at
//! creation, written, flushed, fsynced and atomically renamed into place. This is
//! never a write-then-chmod helper.

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

impl AuthFile {
    pub fn redact(&self, text: &str) -> String {
        let mut result = text.to_string();
        for secret in self
            .accounts
            .values()
            .filter_map(|s| s.api_key.as_deref())
            .chain(self.text.iter().filter_map(|s| s.api_key.as_deref()))
            .chain(self.voice.iter().filter_map(|s| s.api_key.as_deref()))
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
        Self::load_from(&auth_path())
    }

    /// Public staged loader. Reads an auth document from `path`, migrating the
    /// legacy `research` / `gmail` slots away; the migration rewrite itself goes
    /// through the commit store's secure primitive.
    pub fn load_from(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
        if raw.trim().is_empty() {
            return Ok(Self::default());
        }
        let auth: Self = serde_json::from_str(&raw)?;
        let legacy: serde_json::Value = serde_json::from_str(&raw)?;
        if legacy.get("research").is_some() || legacy.get("gmail").is_some() {
            auth.save_to(path)?;
        }
        Ok(auth)
    }

    /// Commits ONLY the `auth` slot, serialised behind the app-wide configuration
    /// write lock. It never writes `config.toml` or `quota.json`; a caller that
    /// needs all three files to move together uses the combined transfer entry
    /// point on `SettingsFile`.
    pub fn save(&self) -> Result<()> {
        ensure_home()?;
        let lock = crate::config_commit::ConfigLock::acquire()?;
        lock.commit_files(&[crate::config_commit::CommitFile::new(
            "auth",
            auth_path(),
            serde_json::to_string_pretty(self)?,
        )])?;
        Ok(())
    }

    /// The secure single-file write used for non-canonical/test paths. It goes
    /// through the commit store's secure primitive (unique sibling temp file
    /// created 0600, written, flushed, fsynced, atomically renamed) and bypasses
    /// the cross-file lock by design.
    pub fn save_to(&self, path: &Path) -> Result<()> {
        write_private(path, &serde_json::to_string_pretty(self)?)
    }
}

/// Writes `path` atomically with owner-only permissions.
///
/// This routes through the commit store's secure primitive: a unique sibling
/// temporary file is created 0600 at creation, written, flushed, fsynced and
/// atomically renamed into place. It is NOT a write-then-chmod helper.
pub fn write_private(path: &Path, body: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    crate::config_commit::write_secure(path, body)
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
        assert!(!serde_json::to_string(&auth)
            .unwrap()
            .contains("existing-mail"));

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
    fn loading_old_auth_removes_gmail_setup_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        std::fs::write(&path, r#"{"gmail":{"email":"ada@gmail.com","app_password":"old-secret"},"accounts":{"openrouter":{"kind":"openrouter","base_url":"https://openrouter.ai/api/v1","model":"router-model","api_key":"router-key"}}}"#).unwrap();
        let auth = AuthFile::load_from(&path).unwrap();
        assert!(auth.account("openrouter").is_some());
        let saved = std::fs::read_to_string(path).unwrap();
        assert!(!saved.contains("gmail"));
        assert!(!saved.contains("old-secret"));
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

    /// The secure primitive creates the destination owner-only and leaves no
    /// staged sibling behind, so a reader never sees a partial file.
    #[test]
    fn saving_through_the_lock_keeps_the_file_owner_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        let mut auth = AuthFile::default();
        let mut secret = crate::provider::account_secret(&auth, "openrouter");
        secret.api_key = Some("router-secret".into());
        auth.set_account(secret);
        auth.save_to(&path).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            serde_json::to_string_pretty(&auth).unwrap()
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        assert_no_staged_files(dir.path());
    }

    /// One slot, one file: a single-slot commit never publishes the other
    /// configuration files, and every commit advances the generation.
    #[test]
    fn auth_save_commits_only_the_auth_slot() {
        let dir = tempfile::tempdir().unwrap();
        let before = crate::config_commit::current_generation();
        let lock = crate::config_commit::ConfigLock::acquire_at(dir.path()).unwrap();
        assert!(lock.generation() >= before);
        let generation = lock
            .commit_files(&[crate::config_commit::CommitFile::new(
                "auth",
                dir.path().join("auth.json"),
                serde_json::to_string_pretty(&AuthFile::default()).unwrap(),
            )])
            .unwrap();
        assert!(generation > before, "commit must bump the generation");
        let after = crate::config_commit::current_generation();
        assert!(
            after >= before,
            "generation must advance monotonically: {before} -> {after}"
        );
        assert!(dir.path().join("auth.json").exists());
        assert!(!dir.path().join("config.toml").exists());
        assert!(!dir.path().join("quota.json").exists());
        assert_no_staged_files(dir.path());
    }

    /// Asserts the directory holds no `*.tmp` or `*.staged` leftovers.
    fn assert_no_staged_files(dir: &std::path::Path) {
        let leftovers: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".tmp") || name.ends_with(".staged"))
            .collect();
        assert!(leftovers.is_empty(), "staged leftovers: {leftovers:?}");
    }
}
