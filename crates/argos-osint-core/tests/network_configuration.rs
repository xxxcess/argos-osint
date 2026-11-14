//! Public network readers must not migrate files or revive environment secrets.

use std::path::PathBuf;
use std::process::Command;

use argos_osint_core::config_commit;
use argos_osint_core::provider;
use argos_osint_core::provider::SettingsFile;
use argos_osint_core::secrets::AuthFile;
use argos_osint_core::secrets::ProviderSecret;

const CHILD: &str = "ARGOS_TEST_CURRENT_NETWORK_CONFIGURATION";

#[test]
fn network_readers_preserve_legacy_bytes_and_use_only_explicit_current_configuration() {
    if let Some(root) = std::env::var_os(CHILD) {
        let root = PathBuf::from(root)
            .canonicalize()
            .expect("parent-created root");
        let temporary = std::env::temp_dir()
            .canonicalize()
            .expect("temporary parent");
        assert!(root.starts_with(temporary));
        assert_eq!(
            std::fs::read(root.join(".network-fixture")).expect("fixture marker"),
            b"synthetic-only"
        );
        let fresh = SettingsFile::load().expect("fresh setup remains readable");
        for role in provider::CANONICAL_ROLE_KEYS {
            let assignment = fresh
                .defaults
                .role(role)
                .expect("explicit empty canonical role");
            assert!(assignment
                .chain()
                .iter()
                .all(|route| route.model.is_empty() && route.provider.is_empty()));
        }
        assert!(fresh.provider_key("firecrawl").is_empty());
        assert!(fresh.provider_fallback_key("firecrawl").is_empty());
        let secret = ProviderSecret {
            kind: "openrouter".into(),
            base_url: "https://openrouter.ai/api/v1".into(),
            model: "synthetic-model".into(),
            api_key: None,
            stt_model: None,
            device: None,
        };
        assert!(
            provider::resolved_key(&secret).is_none(),
            "vendor environment cannot revive a network account"
        );
        assert!(AuthFile::load()
            .expect("fresh empty accounts")
            .accounts
            .is_empty());
        for name in ["config.toml", "auth.json", "quota.json"] {
            assert!(!root.join(name).exists(), "read created {name}");
        }
        let old_settings = "writer_provider = 'openrouter'\nwriter_model = 'historical'\n";
        config_commit::write_secure(&root.join("config.toml"), old_settings)
            .expect("isolated legacy fixture");
        let old_auth = "{\"research\":{\"kind\":\"openrouter\",\"model\":\"historical\",\"base_url\":\"https://example.test\",\"api_key\":\"synthetic-old-secret\"}}";
        config_commit::write_secure(&root.join("auth.json"), old_auth)
            .expect("isolated legacy auth");
        assert!(SettingsFile::load().is_err());
        assert!(AuthFile::load().is_err());
        assert!(AuthFile::load_cached().is_err());
        assert_eq!(
            std::fs::read_to_string(root.join("config.toml")).expect("retained settings"),
            old_settings
        );
        assert_eq!(
            std::fs::read_to_string(root.join("auth.json")).expect("retained auth"),
            old_auth
        );

        let mut current_settings = toml::Value::try_from(&fresh).expect("current settings value");
        current_settings
            .as_table_mut()
            .expect("settings table")
            .insert("configuration_format".into(), toml::Value::Integer(1));
        config_commit::write_secure(
            &root.join("config.toml"),
            &toml::to_string(&current_settings).expect("current settings encoding"),
        )
        .expect("explicit current fixture");
        let mut saved = secret;
        saved.api_key = Some("synthetic-explicit-account-key".into());
        let current_auth =
            serde_json::json!({"configuration_format":1,"accounts":{"synthetic-account":saved}});
        config_commit::write_secure(
            &root.join("auth.json"),
            &serde_json::to_string(&current_auth).expect("current auth encoding"),
        )
        .expect("explicit current auth");
        let settings_before =
            std::fs::read(root.join("config.toml")).expect("current settings bytes");
        let auth_before = std::fs::read(root.join("auth.json")).expect("current auth bytes");
        assert!(SettingsFile::load()
            .expect("current settings")
            .provider_key("firecrawl")
            .is_empty());
        let auth = AuthFile::load().expect("current explicit account");
        assert_eq!(
            provider::resolved_key(&auth.accounts["synthetic-account"]).as_deref(),
            Some("synthetic-explicit-account-key")
        );
        assert_eq!(
            std::fs::read(root.join("config.toml")).expect("unchanged settings"),
            settings_before
        );
        assert_eq!(
            std::fs::read(root.join("auth.json")).expect("unchanged auth"),
            auth_before
        );
        assert!(SettingsFile::load_from(&root.join("alternate.toml")).is_err());
        assert!(AuthFile::load_from(&root.join("alternate.json")).is_err());
        return;
    }
    let fixture = tempfile::tempdir().expect("isolated network workspace");
    let root = fixture.path().canonicalize().expect("canonical fixture");
    std::fs::write(root.join(".network-fixture"), b"synthetic-only")
        .expect("fixture ownership marker");
    let output = Command::new(std::env::current_exe().expect("test executable"))
        .args([
            "--exact",
            "network_readers_preserve_legacy_bytes_and_use_only_explicit_current_configuration",
            "--nocapture",
        ])
        .env(CHILD, &root)
        .env("ARGOS_HOME", &root)
        .env("FLEETWORKS_WORKSPACE", &root)
        .env("FIRECRAWL_API_KEY", "synthetic-unowned-env-key")
        .env(
            "FIRECRAWL_API_KEY_FALLBACK",
            "synthetic-unowned-env-fallback",
        )
        .env("OPENROUTER_API_KEY", "synthetic-unowned-model-key")
        .env_remove("FLEETWORKS_EXECUTION_ORIGIN")
        .output()
        .expect("fresh isolated network process");
    assert!(
        output.status.success(),
        "network reader fixture failed: {} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
