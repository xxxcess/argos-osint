use std::ffi::OsStr;
use std::fs;
use std::path::Path;

use crate::{persist_new, Workspace};

#[test]
fn workspace_selection_is_absolute_and_does_not_create_directories() {
    let temp = tempfile::tempdir().unwrap();
    let cwd = fs::canonicalize(temp.path()).unwrap();
    let workspace = Workspace::resolve_from(
        Some(Path::new("next/../chosen")),
        Some(OsStr::new("ignored")),
        &cwd,
    )
    .unwrap();
    assert_eq!(workspace.root(), cwd.join("chosen"));
    assert!(!workspace.root().exists());

    let configured = Workspace::resolve_from(None, Some(OsStr::new("configured")), &cwd).unwrap();
    assert_eq!(configured.root(), cwd.join("configured"));
    let default = Workspace::resolve_from(None, None, &cwd).unwrap();
    assert_eq!(default.root(), cwd.join(".fleetworks"));
}

#[test]
fn workspace_initialization_preserves_the_token_and_anchors_datasets() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = Workspace::resolve(Some(&temp.path().join("workspace"))).unwrap();
    workspace.initialize_api_token().unwrap();
    let original = workspace
        .read_api_token_file(Path::new("secrets/api-token"))
        .unwrap();
    assert_eq!(original.len(), 64);
    workspace.initialize_api_token().unwrap();
    assert_eq!(
        workspace
            .read_api_token_file(Path::new("secrets/api-token"))
            .unwrap(),
        original
    );

    fs::write(
        workspace.root().join("config/workspace.toml"),
        "[datasets]\narticles = \"datasets/examples.csv\"\n",
    )
    .unwrap();
    assert_eq!(
        workspace.dataset("articles").unwrap().unwrap(),
        workspace.root().join("datasets/examples.csv")
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(workspace.root().join("secrets/api-token"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}

#[test]
fn private_environment_defaults_take_priority_and_errors_are_redacted() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = Workspace::resolve(Some(temp.path())).unwrap();
    workspace.initialize().unwrap();
    let setting = workspace.root().join("config/workspace.toml");
    fs::write(
        &setting,
        "[environment]\nFLEETWORKS_FIXTURE_SETTING = \"public\"\n",
    )
    .unwrap();
    persist_new(
        &workspace.root().join("secrets/environment.toml"),
        b"[environment]\nFLEETWORKS_FIXTURE_SETTING = \"private\"\n",
    )
    .unwrap();
    let env = workspace.environment().unwrap();
    assert_eq!(
        env.get("FLEETWORKS_FIXTURE_SETTING").as_deref(),
        Some(OsStr::new("private"))
    );
    assert_eq!(env.get("PATH"), std::env::var_os("PATH"));

    fs::write(&setting, "invalid: unprinted-secret").unwrap();
    let error = workspace.environment().err().unwrap();
    assert!(!format!("{error:#}").contains("unprinted-secret"));
}

#[test]
fn legacy_import_projects_only_supported_values_and_replays() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("legacy");
    fs::create_dir(&source).unwrap();
    let config = r#"
newsdata_api_key = "fixture-news-key"
osint_user_agent = "unrelated-setting"
[defaults.atlas_analyst]
provider = "openrouter"
model = "fixture-model"
unexpected = "omit-unknown-field"
[[defaults.atlas_analyst.fallbacks]]
provider = "local"
model = "fallback"
[defaults.operation_overrides."intel_brief_claims.generate"]
route_role = "atlas_analyst"
"#;
    let auth = r#"{"accounts":{"openrouter":{"kind":"openrouter","base_url":"https://example.test","model":"fixture-model","api_key":"fixture-api-key","device":{"client_id":"omit-device"}},"other":{"kind":"other","base_url":"https://example.test","model":"ignored"}}}"#;
    fs::write(source.join("config.toml"), config).unwrap();
    fs::write(source.join("auth.json"), auth).unwrap();

    let workspace = Workspace::resolve(Some(&temp.path().join("workspace"))).unwrap();
    let summary = workspace.import_legacy(&source).unwrap();
    assert_eq!(summary.api_keys, 2);
    assert_eq!(summary.model_defaults, 2);
    assert_eq!(workspace.import_legacy(&source).unwrap(), summary);
    assert_eq!(
        fs::read_to_string(source.join("config.toml")).unwrap(),
        config
    );
    assert_eq!(fs::read_to_string(source.join("auth.json")).unwrap(), auth);

    let imported = fs::read_to_string(workspace.root().join("config.toml")).unwrap();
    assert!(imported.contains("fixture-news-key"));
    assert!(imported.contains("route_role"));
    assert!(imported.contains("fallback"));
    assert!(!imported.contains("unrelated-setting"));
    assert!(!imported.contains("omit-unknown-field"));
    let accounts = fs::read_to_string(workspace.root().join("auth.json")).unwrap();
    assert!(accounts.contains("fixture-api-key"));
    assert!(!accounts.contains("omit-device"));
    assert!(!accounts.contains("ignored"));
}

#[test]
fn legacy_import_checks_all_conflicts_before_writing() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("legacy");
    fs::create_dir(&source).unwrap();
    fs::write(
        source.join("config.toml"),
        "newsdata_api_key = \"incoming-key\"",
    )
    .unwrap();
    fs::write(
        source.join("auth.json"),
        r#"{"text":{"kind":"local","base_url":"https://example.test","model":"fixture","api_key":"incoming-account"}}"#,
    )
    .unwrap();

    let workspace = Workspace::resolve(Some(&temp.path().join("workspace"))).unwrap();
    workspace.initialize().unwrap();
    persist_new(&workspace.root().join("config.toml"), b"existing-key").unwrap();
    let error = workspace.import_legacy(&source).unwrap_err();
    assert!(!format!("{error:#}").contains("incoming-key"));
    assert!(!workspace.root().join("auth.json").exists());
    assert_eq!(
        fs::read(workspace.root().join("config.toml")).unwrap(),
        b"existing-key"
    );
    let nested = Workspace::resolve(Some(&source.join("nested"))).unwrap();
    assert!(nested.import_legacy(&source).is_err());
    assert!(!nested.root().exists());
}
