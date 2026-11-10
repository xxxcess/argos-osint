//! Authenticated writes to the service workspace configuration.

use std::env;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use argos_osint_core::config_commit::ConfigLock;
use argos_osint_core::config_transfer;
use argos_osint_core::config_transfer::QuotaSettingsFile;
use argos_osint_core::provider;
use argos_osint_core::provider::ModelAssignment;
use argos_osint_core::provider::ModelRoute;
use argos_osint_core::provider::SettingsFile;
use argos_osint_core::secrets::AuthFile;
use axum::http::StatusCode;
use axum::routing::post;
use axum::Json;
use axum::Router;
use serde::Deserialize;
use serde_json::json;
use serde_json::Value;

use crate::{bad, internal, ApiError, ApiResult, ApiState};

pub(crate) fn routes() -> Router<ApiState> {
    Router::new()
        .route("/api/v1/profile/export", post(export))
        .route("/api/v1/models/defaults/set", post(set_default))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Export {
    name: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetDefault {
    role: String,
    primary: Primary,
    fallbacks: Option<Vec<Route>>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Primary {
    Route(Route),
    Clear(()),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Route {
    provider: String,
    model: String,
    #[serde(default)]
    account: String,
}

async fn export(Json(request): Json<Export>) -> ApiResult {
    let result = tokio::task::spawn_blocking(move || export_config(&workspace()?, &request.name))
        .await
        .map_err(|_| internal("profile export task failed"))??;
    Ok(Json(result))
}

async fn set_default(Json(request): Json<SetDefault>) -> ApiResult {
    tokio::task::spawn_blocking(move || update_role(&workspace()?, request))
        .await
        .map_err(|_| internal("model settings task failed"))??;
    Ok(Json(json!({"saved": true})))
}

fn workspace() -> Result<PathBuf, ApiError> {
    let unavailable = || internal("workspace controls require an initialized Fleetworks workspace");
    let root = env::var_os("FLEETWORKS_WORKSPACE").ok_or_else(unavailable)?;
    if !Path::new(&root).is_absolute() || env::var_os("ARGOS_HOME").as_ref() != Some(&root) {
        return Err(unavailable());
    }
    fs::canonicalize(root).map_err(|_| unavailable())
}

fn valid_export_name(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn export_config(root: &Path, name: &str) -> Result<Value, ApiError> {
    if !valid_export_name(name) {
        return Err(bad());
    }
    let exports = root.join("exports");
    if !fs::symlink_metadata(&exports).is_ok_and(|metadata| metadata.is_dir()) {
        return Err(internal("workspace exports directory is unavailable"));
    }
    let filename = format!("{name}.json");
    let (body, schema_version) = {
        let _lock =
            ConfigLock::acquire_at(root).map_err(|_| internal("configuration lock failed"))?;
        let settings = read_settings(root)?;
        let auth = read_auth(root)?;
        let quotas = QuotaSettingsFile::load_from(&configuration_path(root, "quota.json")?)
            .map_err(|_| internal("could not read workspace quota settings"))?;
        let document = config_transfer::export_document(&settings, &auth, &quotas);
        let body = config_transfer::serialize_document(&document)
            .map_err(|_| internal("could not encode portable configuration"))?;
        (body, document.schema_version)
    };
    write_export(&exports, &filename, &body)?;
    Ok(json!({"path":format!("exports/{filename}"),"schema_version":schema_version}))
}

#[cfg(unix)]
fn write_export(exports: &Path, filename: &str, body: &str) -> Result<(), ApiError> {
    use std::io::ErrorKind;
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);
    let unavailable = || internal("could not publish private portable configuration");
    let (stage, mut file) = loop {
        let seq = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = exports.join(format!(".profile-export-{}-{seq}", std::process::id()));
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
        {
            Ok(file) => break (StagedExport(path), file),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(_) => return Err(unavailable()),
        }
    };
    file.write_all(body.as_bytes()).map_err(|_| unavailable())?;
    file.sync_all().map_err(|_| unavailable())?;
    drop(file);
    fs::hard_link(&stage.0, exports.join(filename)).map_err(|error| {
        if error.kind() == ErrorKind::AlreadyExists {
            ApiError(StatusCode::CONFLICT, "Export already exists")
        } else {
            unavailable()
        }
    })?;
    drop(stage);
    // Match the configuration store's best-effort directory fsync.
    let _ = fs::File::open(exports).and_then(|dir| dir.sync_all());
    Ok(())
}

#[cfg(unix)]
struct StagedExport(PathBuf);

#[cfg(unix)]
impl Drop for StagedExport {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

#[cfg(not(unix))]
fn write_export(_: &Path, _: &str, _: &str) -> Result<(), ApiError> {
    Err(internal(
        "owner-only workspace exports are unavailable on this platform",
    ))
}

fn read_settings(root: &Path) -> Result<SettingsFile, ApiError> {
    SettingsFile::load_from(&configuration_path(root, "config.toml")?)
        .map_err(|_| internal("could not read workspace settings"))
}

fn read_auth(root: &Path) -> Result<AuthFile, ApiError> {
    AuthFile::load_from(&configuration_path(root, "auth.json")?)
        .map_err(|_| internal("could not read workspace accounts"))
}

fn configuration_path(root: &Path, name: &str) -> Result<PathBuf, ApiError> {
    let path = root.join(name);
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.is_file() => Ok(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(path),
        _ => Err(internal("workspace configuration file is unavailable")),
    }
}

fn update_role(root: &Path, request: SetDefault) -> Result<(), ApiError> {
    if !provider::CANONICAL_ROLE_KEYS.contains(&request.role.as_str()) {
        return Err(bad());
    }
    // Hold the same cross-process write lock used by settings and imports for
    // the whole read-modify-write. save_to already performs a secure atomic
    // replacement without reacquiring that lock.
    let _lock = ConfigLock::acquire_at(root).map_err(|_| internal("configuration lock failed"))?;
    let mut settings = read_settings(root)?;
    let auth = read_auth(root)?;
    let saved = settings.defaults.role_mut(&request.role).ok_or_else(bad)?;
    *saved = assignment(&auth, request, saved)?;
    settings
        .save_to(&configuration_path(root, "config.toml")?)
        .map_err(|_| internal("could not save workspace model defaults"))
}

fn assignment(
    auth: &AuthFile,
    request: SetDefault,
    saved: &ModelAssignment,
) -> Result<ModelAssignment, ApiError> {
    let Primary::Route(primary) = request.primary else {
        if request.fallbacks.is_some_and(|routes| !routes.is_empty()) {
            return Err(bad());
        }
        return Ok(ModelAssignment::default());
    };
    let ModelRoute {
        provider,
        model,
        account,
    } = checked_route(auth, primary)?;
    let mut assignment = ModelAssignment {
        provider,
        model,
        account,
        fallbacks: Vec::new(),
    };
    let fallbacks = match request.fallbacks {
        Some(routes) => routes
            .into_iter()
            .map(|route| checked_route(auth, route))
            .collect::<Result<Vec<_>, _>>()?,
        None => saved.fallbacks.clone(),
    };
    if fallbacks.len() > 16 {
        return Err(bad());
    }
    for fallback in fallbacks {
        assignment.add_fallback(fallback).map_err(|_| bad())?;
    }
    if matches!(
        request.role.as_str(),
        "recon_agent" | "recon_analyst" | "atlas_analyst" | "brain_summary"
    ) && assignment
        .chain()
        .iter()
        .all(|route| provider::is_decisions_model(&route.model))
    {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "Role needs a chat-capable route",
        ));
    }
    Ok(assignment)
}

fn checked_route(auth: &AuthFile, route: Route) -> Result<ModelRoute, ApiError> {
    let provider = provider::normalize_kind(&route.provider);
    let model = route.model.trim();
    let account = provider::normalize_kind(&route.account);
    let valid = |value: &str, maximum: usize| {
        value.len() <= maximum && !value.chars().any(char::is_control)
    };
    if provider.is_empty()
        || model.is_empty()
        || !valid(&route.provider, 128)
        || !valid(&route.model, 256)
        || !valid(&route.account, 128)
        || provider::is_removed_kind(&provider)
        || (provider::is_jev_family(model) && provider != "openrouter")
    {
        return Err(bad());
    }
    let configured_for_provider = |key: &str| {
        auth.account(key)
            .is_some_and(|secret| provider::effective_kind(&secret) == provider)
    };
    let compatible = if account.is_empty() {
        provider::preset(&provider).is_some()
            || provider == "openai-chatgpt"
            || configured_for_provider(&provider)
    } else {
        configured_for_provider(&account)
    };
    if !compatible {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "Provider and saved account do not match",
        ));
    }
    Ok(ModelRoute {
        provider,
        model: model.to_owned(),
        account,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn portable_export_is_private_and_cannot_overwrite_or_follow_a_symlink() {
        use std::os::unix::fs::{symlink, PermissionsExt};

        let root = tempfile::tempdir().unwrap();
        let exports = root.path().join("exports");
        fs::create_dir(&exports).unwrap();
        let mut settings = SettingsFile::default();
        settings.set_saved_key("firecrawl", "fixture-credential-only-on-disk");
        settings.save_to(&root.path().join("config.toml")).unwrap();

        let receipt = export_config(root.path(), "portable")
            .unwrap_or_else(|_| panic!("portable export failed"));
        assert_eq!(receipt["path"], "exports/portable.json");
        assert!(!receipt.to_string().contains("fixture-credential"));
        let path = exports.join("portable.json");
        let saved = fs::read_to_string(&path).unwrap();
        assert!(saved.contains("fixture-credential-only-on-disk"));
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(matches!(
            export_config(root.path(), "portable"),
            Err(ApiError(StatusCode::CONFLICT, _))
        ));
        assert_eq!(fs::read_to_string(&path).unwrap(), saved);
        assert!(matches!(
            export_config(root.path(), "../escape"),
            Err(ApiError(StatusCode::BAD_REQUEST, _))
        ));

        fs::remove_file(&path).unwrap();
        fs::remove_dir(&exports).unwrap();
        let outside = tempfile::tempdir().unwrap();
        symlink(outside.path(), &exports).unwrap();
        assert!(export_config(root.path(), "outside").is_err());
        assert_eq!(outside.path().read_dir().unwrap().count(), 0);
    }

    #[test]
    fn role_updates_serialize_and_reject_incompatible_assignments() {
        let root = tempfile::tempdir().unwrap();
        let request = |role: &str, model: &str| {
            serde_json::from_value(json!({
                "role": role,
                "primary": {"provider":"local","model": model},
            }))
            .unwrap()
        };
        std::thread::scope(|scope| {
            let path = root.path();
            let first = scope.spawn(|| update_role(path, request("atlas_analyst", "first")));
            let second = scope.spawn(|| update_role(path, request("brain_summary", "second")));
            assert!(first.join().unwrap().is_ok());
            assert!(second.join().unwrap().is_ok());
        });
        let saved = SettingsFile::load_from(&root.path().join("config.toml")).unwrap();
        assert_eq!(saved.defaults.role("atlas_analyst").unwrap().model, "first");
        assert_eq!(
            saved.defaults.role("brain_summary").unwrap().model,
            "second"
        );

        let bad: SetDefault = serde_json::from_value(json!({
            "role":"atlas_analyst",
            "primary":{"provider":"openrouter","model":"typesafe/jev"},
        }))
        .unwrap();
        assert!(matches!(
            update_role(root.path(), bad),
            Err(ApiError(StatusCode::BAD_REQUEST, _))
        ));
        let unknown_account: SetDefault = serde_json::from_value(json!({
            "role":"atlas_analyst",
            "primary":{"provider":"local","model":"text-model","account":"unsaved"},
        }))
        .unwrap();
        assert!(matches!(
            update_role(root.path(), unknown_account),
            Err(ApiError(StatusCode::BAD_REQUEST, _))
        ));
        assert!(serde_json::from_value::<SetDefault>(json!({"role":"atlas_analyst"})).is_err());
        let clear: SetDefault =
            serde_json::from_value(json!({"role":"atlas_analyst","primary":null})).unwrap();
        assert!(update_role(root.path(), clear).is_ok());
        let saved = SettingsFile::load_from(&root.path().join("config.toml")).unwrap();
        assert!(saved
            .defaults
            .role("atlas_analyst")
            .unwrap()
            .model
            .is_empty());
        assert_eq!(
            saved.defaults.role("brain_summary").unwrap().model,
            "second"
        );
    }
}
