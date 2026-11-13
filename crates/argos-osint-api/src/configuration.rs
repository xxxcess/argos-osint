//! Current-format global configuration with one durable optimistic revision.
//!
//! Reads never migrate historical credentials. Portable transfer is explicit,
//! bounded, current-format bytes; normal metadata never contains key material.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

#[cfg(test)]
use argos_osint_core::config_commit;
use argos_osint_core::config_commit::CommitFile;
use argos_osint_core::config_commit::ConfigLock;
use argos_osint_core::provider;
use argos_osint_core::provider::ModelAssignment;
use argos_osint_core::provider::ModelRoute;
use argos_osint_core::provider_capabilities;
use argos_osint_core::request_policy::ReasoningEffort;
use argos_osint_core::secrets::AuthFile;
use argos_osint_core::secrets::ProviderSecret;
use argos_osint_core::storage::configuration_reset;
use axum::body::Bytes;
use axum::extract::Extension;
use axum::extract::State;
use axum::http::header;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::response::Response;
use axum::Json;
use serde::Deserialize;
use serde::Serialize;
use serde_json::json;
use serde_json::Value;

use crate::bad;
use crate::internal;
use crate::request_context::ResolvedRequest;
use crate::ApiError;
use crate::ApiResult;
use crate::ApiState;

const LIMIT: u64 = 2 * 1024 * 1024;
const FORMAT: u16 = 1;
const PORTABLE_SCHEMA: u16 = 4;

pub(crate) type Snapshot = argos_osint_core::current_configuration::Snapshot;

pub(crate) fn workspace() -> Result<PathBuf, ApiError> {
    let root = std::env::var_os("FLEETWORKS_WORKSPACE")
        .map(PathBuf::from)
        .ok_or_else(|| internal("configuration workspace not provisioned"))?;
    if !root.is_absolute() || std::env::var_os("ARGOS_HOME").as_deref() != Some(root.as_os_str()) {
        return Err(internal("configuration workspace not provisioned"));
    }
    let metadata =
        fs::symlink_metadata(&root).map_err(|_| internal("configuration workspace unavailable"))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(internal("configuration workspace unavailable"));
    }
    let canonical = root
        .canonicalize()
        .map_err(|_| internal("configuration workspace unavailable"))?;
    if canonical != root {
        return Err(internal("configuration workspace must be canonical"));
    }
    Ok(root)
}

pub(crate) fn read_locked(root: &Path, lock: &ConfigLock) -> Result<Snapshot, ApiError> {
    argos_osint_core::current_configuration::read_locked(root, lock).map_err(|error| match error {
        argos_osint_core::current_configuration::ReadError::LegacyFormat => ApiError(
            StatusCode::CONFLICT,
            "Current configuration format required; explicit operator reset is pending",
        ),
        _ => internal("Current configuration unavailable"),
    })
}

fn commit(root: &Path, lock: &ConfigLock, snapshot: &Snapshot) -> Result<u64, ApiError> {
    let mut settings = toml::Value::try_from(&snapshot.settings)
        .map_err(|_| internal("configuration encoding failed"))?;
    settings.as_table_mut().ok_or_else(bad)?.insert(
        "configuration_format".into(),
        toml::Value::Integer(i64::from(FORMAT)),
    );
    let mut auth = serde_json::to_value(&snapshot.auth)
        .map_err(|_| internal("configuration encoding failed"))?;
    auth.as_object_mut()
        .ok_or_else(bad)?
        .insert("configuration_format".into(), json!(FORMAT));
    // A distinct current-format slot deliberately bypasses retired legacy
    // pipeline inheritance. All canonical role slots are explicit, even empty.
    lock.commit_files(&[
        CommitFile::new(
            "current_settings",
            root.join("config.toml"),
            toml::to_string_pretty(&settings)
                .map_err(|_| internal("configuration encoding failed"))?,
        ),
        CommitFile::new(
            "auth",
            root.join("auth.json"),
            serde_json::to_string_pretty(&auth)
                .map_err(|_| internal("configuration encoding failed"))?,
        ),
        CommitFile::new(
            "quota",
            root.join("quota.json"),
            serde_json::to_string_pretty(&snapshot.quotas)
                .map_err(|_| internal("configuration encoding failed"))?,
        ),
    ])
    .map_err(|_| internal("atomic configuration commit failed"))
}

fn expected(snapshot: &Snapshot, revision: u64) -> Result<(), ApiError> {
    if snapshot.revision != revision {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "Configuration changed; refresh before saving",
        ));
    }
    Ok(())
}

async fn blocking<T: Send + 'static>(
    run: impl FnOnce() -> Result<T, ApiError> + Send + 'static,
) -> Result<T, ApiError> {
    tokio::task::spawn_blocking(run)
        .await
        .map_err(|_| internal("configuration task failed"))?
}

async fn write_lock(
    state: &ApiState,
    context: Option<Extension<ResolvedRequest>>,
) -> Result<(PathBuf, ConfigLock), ApiError> {
    let Some(Extension(context)) = context else {
        return Err(ApiError(
            StatusCode::UNAUTHORIZED,
            "Current configuration authority required",
        ));
    };
    if !context.permits(fleetworks_contracts::context::Permission::ManageConfiguration) {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "Configuration authority required",
        ));
    }
    let (root, lock) = blocking(|| {
        let root = workspace()?;
        let lock = ConfigLock::acquire_at(&root)
            .map_err(|_| internal("configuration lock unavailable"))?;
        Ok((root, lock))
    })
    .await?;
    configuration_reset::configuration_write_allowed(&state.repo, context.scope_id(), &lock)
        .await
        .map_err(|error| {
            if error == configuration_reset::ResetError::Conflict {
                ApiError(StatusCode::CONFLICT, "Configuration reset is pending")
            } else {
                internal("Configuration reset authority unavailable")
            }
        })?;
    // The returned shared lock remains held through the actual commit, so
    // prepare cannot fence between this check and a first-use/key write.
    Ok((root, lock))
}

pub(crate) async fn profile() -> ApiResult {
    let value = blocking(|| {
        let root = workspace()?;
        let lock = ConfigLock::acquire_at(&root).map_err(|_| internal("configuration lock unavailable"))?;
        let snapshot = read_locked(&root, &lock)?;
        let accounts: Vec<Value> = snapshot.auth.accounts.iter().map(|(id, account)| json!({
            "id": id, "provider": account.kind, "base_url": account.base_url, "model": account.model,
            "has_api_key": account.api_key.as_ref().is_some_and(|key| !key.is_empty()),
            "credential_reference":format!("account:{id}"),
        })).collect();
        let tools: Vec<Value> = provider::KEY_ENV.iter().map(|(id, _)| json!({
            "provider": id, "primary_present": !snapshot.settings.saved_key(id).is_empty(),
            "fallback_present": !snapshot.settings.saved_fallback_key(id).is_empty(),
        })).collect();
        Ok(json!({"configuration_revision":snapshot.revision,"configuration_format":FORMAT,
            "accounts":accounts,"tool_credentials":tools,"setup_required":snapshot.auth.accounts.is_empty(),
            "portable_schema_version":PORTABLE_SCHEMA}))
    }).await?;
    Ok(Json(value))
}

pub(crate) async fn models() -> ApiResult {
    let value = blocking(|| {
        let root = workspace()?;
        let lock = ConfigLock::acquire_at(&root).map_err(|_| internal("configuration lock unavailable"))?;
        let snapshot = read_locked(&root, &lock)?;
        let roles: BTreeMap<&str, &ModelAssignment> = provider::CANONICAL_ROLE_KEYS.iter()
            .filter_map(|role| snapshot.settings.defaults.role(role).map(|assignment| (*role, assignment))).collect();
        let mut catalog = BTreeMap::new();
        for (account, secret) in &snapshot.auth.accounts {
            let kind = provider::effective_kind(secret);
            let mut models = vec![secret.model.clone()];
            for assignment in roles.values() {
                models.extend(assignment.chain().into_iter().filter(|route| {
                    route.provider == kind && (route.account == *account || route.account.is_empty() && *account == kind)
                }).map(|route| route.model));
            }
            for model in models.into_iter().filter(|model| !model.is_empty()) {
                let mut selected = secret.clone();
                selected.model = model.clone();
                let caps = provider_capabilities::for_secret(&selected);
                let mut efforts = caps.reasoning.values.into_iter().filter(|value| {
                    serde_json::from_value::<ReasoningEffort>(json!(value)).is_ok()
                }).collect::<Vec<_>>();
                if caps.reasoning.off.is_some() { efforts.push("off".into()); }
                if caps.reasoning.efficient.is_some() { efforts.push("efficient".into()); }
                efforts.sort(); efforts.dedup();
                catalog.insert(format!("{kind}|{account}|{model}"), json!({"provider":kind,"account":account,"model":model,"efforts":efforts}));
            }
        }
        Ok(json!({"configuration_revision":snapshot.revision,"roles":provider::CANONICAL_ROLE_KEYS,"defaults":roles,
            "catalog":catalog.into_values().collect::<Vec<_>>(), "catalog_source":"configured_routes",
            "model_variants":snapshot.settings.defaults.model_variants}))
    }).await?;
    Ok(Json(value))
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Route {
    provider: String,
    model: String,
    #[serde(default)]
    account: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SetRole {
    expected_revision: u64,
    role: String,
    primary: Option<Route>,
    #[serde(default)]
    fallbacks: Vec<Route>,
    #[serde(default)]
    variant_updates: BTreeMap<String, Option<ReasoningEffort>>,
}

fn checked_route(auth: &AuthFile, route: &Route) -> Result<ModelRoute, ApiError> {
    let kind = provider::normalize_kind(&route.provider);
    if kind.is_empty()
        || kind != route.provider
        || route.model.trim().is_empty()
        || route.model.len() > 256
        || route.model.chars().any(char::is_control)
        || route.account.len() > 128
        || provider::is_removed_kind(&kind)
        || (provider::is_jev_family(&route.model) && kind != "openrouter")
    {
        return Err(bad());
    }
    let account = if route.account.is_empty() {
        kind.as_str()
    } else {
        &route.account
    };
    if auth
        .accounts
        .get(account)
        .is_none_or(|saved| provider::effective_kind(saved) != kind)
    {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "A matching configured provider account is required",
        ));
    }
    let account = account.to_owned();
    Ok(ModelRoute {
        provider: kind,
        model: route.model.clone(),
        account,
    })
}

fn assignment(
    auth: &AuthFile,
    role: &str,
    primary: Option<&Route>,
    fallbacks: &[Route],
) -> Result<ModelAssignment, ApiError> {
    if !provider::CANONICAL_ROLE_KEYS.contains(&role) || fallbacks.len() > 16 {
        return Err(bad());
    }
    let Some(primary) = primary else {
        if !fallbacks.is_empty() {
            return Err(bad());
        }
        return Ok(ModelAssignment::default());
    };
    let route = checked_route(auth, primary)?;
    let mut saved = ModelAssignment {
        provider: route.provider,
        model: route.model,
        account: route.account,
        fallbacks: Vec::new(),
    };
    for route in fallbacks {
        saved
            .add_fallback(checked_route(auth, route)?)
            .map_err(|_| bad())?;
    }
    if matches!(
        role,
        "recon_agent" | "recon_analyst" | "atlas_analyst" | "brain_summary"
    ) && saved
        .chain()
        .iter()
        .all(|route| provider::is_decisions_model(&route.model))
    {
        return Err(bad());
    }
    Ok(saved)
}

#[cfg(test)]
fn set_role(root: &Path, request: SetRole) -> Result<u64, ApiError> {
    let lock =
        ConfigLock::acquire_at(root).map_err(|_| internal("configuration lock unavailable"))?;
    set_role_locked(root, &lock, request)
}

fn set_role_locked(root: &Path, lock: &ConfigLock, request: SetRole) -> Result<u64, ApiError> {
    let mut snapshot = read_locked(root, lock)?;
    expected(&snapshot, request.expected_revision)?;
    let assignment = assignment(
        &snapshot.auth,
        &request.role,
        request.primary.as_ref(),
        &request.fallbacks,
    )?;
    for (key, effort) in request.variant_updates {
        let route = assignment
            .chain()
            .into_iter()
            .find(|route| {
                provider::RoleDefaults::variant_key(&request.role, &route.provider, &route.model)
                    == key
            })
            .ok_or_else(bad)?;
        if let Some(effort) = effort {
            let account = if route.account.is_empty() {
                &route.provider
            } else {
                &route.account
            };
            let mut secret = snapshot
                .auth
                .accounts
                .get(account)
                .cloned()
                .ok_or_else(bad)?;
            secret.model = route.model;
            let caps = provider_capabilities::for_secret(&secret);
            let allowed = effort == ReasoningEffort::Default
                || caps
                    .reasoning
                    .values
                    .iter()
                    .any(|value| value == effort.as_str())
                || effort == ReasoningEffort::Off && caps.reasoning.off.is_some()
                || effort == ReasoningEffort::Efficient && caps.reasoning.efficient.is_some();
            if !allowed {
                return Err(ApiError(
                    StatusCode::BAD_REQUEST,
                    "Reasoning variant is not supported by this model",
                ));
            }
            snapshot
                .settings
                .defaults
                .model_variants
                .insert(key, effort);
        } else {
            snapshot.settings.defaults.model_variants.remove(&key);
        }
    }
    *snapshot
        .settings
        .defaults
        .role_mut(&request.role)
        .ok_or_else(bad)? = assignment;
    commit(root, lock, &snapshot)
}

pub(crate) async fn save_role(
    State(state): State<ApiState>,
    context: Option<Extension<ResolvedRequest>>,
    Json(request): Json<SetRole>,
) -> ApiResult {
    let (root, lock) = write_lock(&state, context).await?;
    let revision = blocking(move || set_role_locked(&root, &lock, request)).await?;
    Ok(Json(
        json!({"saved":true,"configuration_revision":revision}),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AccountInput {
    expected_revision: u64,
    account_id: String,
    provider: String,
    base_url: String,
    #[serde(default)]
    model: Option<String>,
    api_key: Option<String>,
}

fn valid_id(id: &str) -> bool {
    (1..=128).contains(&id.len())
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
}

fn checked_account(
    id: &str,
    kind: &str,
    base: &str,
    model: &str,
    key: Option<String>,
) -> Result<ProviderSecret, ApiError> {
    if !valid_id(id)
        || provider::normalize_kind(kind) != kind
        || provider::is_removed_kind(kind)
        || (provider::preset(kind).is_none()
            && !matches!(kind, "local" | "custom" | "openai-compatible"))
        || model.len() > 256
        || model.chars().any(char::is_control)
        || key
            .as_ref()
            .is_some_and(|key| key.len() > 4096 || key.chars().any(char::is_control))
    {
        return Err(bad());
    }
    let url = url::Url::parse(base).map_err(|_| bad())?;
    let loopback = url.host_str().is_some_and(|host| {
        host == "localhost"
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    });
    if url.host().is_none()
        || !(url.scheme() == "https" || url.scheme() == "http" && loopback)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || base.len() > 2048
    {
        return Err(bad());
    }
    Ok(ProviderSecret {
        kind: kind.to_owned(),
        base_url: base.to_owned(),
        model: model.to_owned(),
        api_key: key.filter(|key| !key.is_empty()),
        stt_model: None,
        device: None,
    })
}

fn patched_account(
    saved: Option<&ProviderSecret>,
    request: &AccountInput,
) -> Result<ProviderSecret, ApiError> {
    let saved =
        saved.filter(|saved| saved.kind == request.provider && saved.base_url == request.base_url);
    let key = request
        .api_key
        .clone()
        .or_else(|| saved.and_then(|saved| saved.api_key.clone()));
    let model = request
        .model
        .as_deref()
        .or_else(|| saved.map(|saved| saved.model.as_str()))
        .unwrap_or("");
    let mut account = checked_account(
        &request.account_id,
        &request.provider,
        &request.base_url,
        model,
        key,
    )?;
    if let Some(saved) = saved {
        account.stt_model = saved.stt_model.clone();
        account.device = saved.device.clone();
    }
    Ok(account)
}

pub(crate) async fn upsert_account(
    State(state): State<ApiState>,
    context: Option<Extension<ResolvedRequest>>,
    Json(request): Json<AccountInput>,
) -> ApiResult {
    let (root, lock) = write_lock(&state, context).await?;
    let revision = blocking(move || {
        let mut snapshot = read_locked(&root, &lock)?;
        expected(&snapshot, request.expected_revision)?;
        if !snapshot.auth.accounts.contains_key(&request.account_id)
            && snapshot.auth.accounts.len() >= 64
        {
            return Err(bad());
        }
        let account = patched_account(snapshot.auth.accounts.get(&request.account_id), &request)?;
        snapshot.auth.accounts.insert(request.account_id, account);
        commit(&root, &lock, &snapshot)
    })
    .await?;
    Ok(Json(
        json!({"saved":true,"configuration_revision":revision}),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DeleteAccount {
    expected_revision: u64,
    account_id: String,
}

pub(crate) async fn delete_account(
    State(state): State<ApiState>,
    context: Option<Extension<ResolvedRequest>>,
    Json(request): Json<DeleteAccount>,
) -> ApiResult {
    let (root, lock) = write_lock(&state, context).await?;
    let revision = blocking(move || {
        let mut snapshot = read_locked(&root, &lock)?;
        expected(&snapshot, request.expected_revision)?;
        if !valid_id(&request.account_id) {
            return Err(bad());
        }
        if provider::CANONICAL_ROLE_KEYS
            .iter()
            .filter_map(|role| snapshot.settings.defaults.role(role))
            .any(|assignment| {
                assignment.chain().iter().any(|route| {
                    route.account == request.account_id
                        || route.account.is_empty() && route.provider == request.account_id
                })
            })
        {
            return Err(ApiError(
                StatusCode::CONFLICT,
                "Clear model assignments referencing this account first",
            ));
        }
        snapshot.auth.accounts.remove(&request.account_id);
        commit(&root, &lock, &snapshot)
    })
    .await?;
    Ok(Json(
        json!({"saved":true,"configuration_revision":revision}),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ToolCredential {
    expected_revision: u64,
    provider: String,
    #[serde(default)]
    primary: Option<String>,
    #[serde(default)]
    fallback: Option<String>,
}

fn apply_tool_credential(settings: &mut provider::SettingsFile, request: &ToolCredential) {
    if let Some(primary) = &request.primary {
        settings.set_saved_key(&request.provider, primary);
    }
    if let Some(fallback) = &request.fallback {
        settings.set_saved_fallback_key(&request.provider, fallback);
    }
}

pub(crate) async fn save_tool_credential(
    State(state): State<ApiState>,
    context: Option<Extension<ResolvedRequest>>,
    Json(request): Json<ToolCredential>,
) -> ApiResult {
    let (root, lock) = write_lock(&state, context).await?;
    let revision = blocking(move || {
        if !provider::KEY_ENV
            .iter()
            .any(|(id, _)| *id == request.provider)
            || [&request.primary, &request.fallback]
                .into_iter()
                .flatten()
                .any(|key| key.len() > 4096 || key.chars().any(char::is_control))
        {
            return Err(bad());
        }
        let mut snapshot = read_locked(&root, &lock)?;
        expected(&snapshot, request.expected_revision)?;
        apply_tool_credential(&mut snapshot.settings, &request);
        commit(&root, &lock, &snapshot)
    })
    .await?;
    Ok(Json(
        json!({"saved":true,"configuration_revision":revision}),
    ))
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PortableAssignment {
    primary: Option<Route>,
    fallbacks: Vec<Route>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PortableAccount {
    provider: String,
    base_url: String,
    model: String,
    api_key: Option<String>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PortableTool {
    primary: String,
    fallback: String,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Portable {
    schema_version: u16,
    roles: BTreeMap<String, PortableAssignment>,
    accounts: BTreeMap<String, PortableAccount>,
    tool_credentials: BTreeMap<String, PortableTool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Export {
    expected_revision: u64,
    include_credentials: bool,
}

pub(crate) async fn export(Json(request): Json<Export>) -> Result<Response, ApiError> {
    let body = blocking(move || {
        if !request.include_credentials {
            return Err(ApiError(
                StatusCode::BAD_REQUEST,
                "Explicit credential export consent required",
            ));
        }
        let root = workspace()?;
        let lock = ConfigLock::acquire_at(&root)
            .map_err(|_| internal("configuration lock unavailable"))?;
        let snapshot = read_locked(&root, &lock)?;
        expected(&snapshot, request.expected_revision)?;
        let route = |route: ModelRoute| Route {
            provider: route.provider,
            model: route.model,
            account: route.account,
        };
        let roles = provider::CANONICAL_ROLE_KEYS
            .iter()
            .map(|role| {
                let saved = snapshot
                    .settings
                    .defaults
                    .role(role)
                    .expect("validated canonical role");
                (
                    (*role).to_owned(),
                    PortableAssignment {
                        primary: (!saved.model.is_empty()).then(|| route(saved.as_route())),
                        fallbacks: saved.fallbacks.iter().cloned().map(route).collect(),
                    },
                )
            })
            .collect();
        let accounts = snapshot
            .auth
            .accounts
            .iter()
            .map(|(id, saved)| {
                (
                    id.clone(),
                    PortableAccount {
                        provider: saved.kind.clone(),
                        base_url: saved.base_url.clone(),
                        model: saved.model.clone(),
                        api_key: saved.api_key.clone(),
                    },
                )
            })
            .collect();
        let tool_credentials = provider::KEY_ENV
            .iter()
            .map(|(id, _)| {
                (
                    (*id).to_owned(),
                    PortableTool {
                        primary: snapshot.settings.saved_key(id).to_owned(),
                        fallback: snapshot.settings.saved_fallback_key(id).to_owned(),
                    },
                )
            })
            .collect();
        serde_json::to_vec_pretty(&Portable {
            schema_version: PORTABLE_SCHEMA,
            roles,
            accounts,
            tool_credentials,
        })
        .map_err(|_| internal("configuration export failed"))
    })
    .await?;
    let mut response = body.into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("application/octet-stream"),
    );
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        header::HeaderValue::from_static("attachment; filename=\"fleetworks-configuration.json\""),
    );
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store, private"),
    );
    Ok(response)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ImportQuery {
    expected_revision: u64,
    replace: bool,
}

pub(crate) async fn import(
    State(state): State<ApiState>,
    context: Option<Extension<ResolvedRequest>>,
    axum::extract::Query(query): axum::extract::Query<ImportQuery>,
    body: Bytes,
) -> ApiResult {
    if body.len() as u64 > LIMIT || !query.replace {
        return Err(bad());
    }
    let (root, lock) = write_lock(&state, context).await?;
    let revision = blocking(move || {
        let document: Portable = serde_json::from_slice(&body).map_err(|_| bad())?;
        if document.schema_version != PORTABLE_SCHEMA
            || document.roles.len() != 6
            || document.accounts.len() > 64
            || document.tool_credentials.len() > provider::KEY_ENV.len()
        {
            return Err(bad());
        }
        let mut snapshot = read_locked(&root, &lock)?;
        expected(&snapshot, query.expected_revision)?;
        let mut auth = AuthFile::default();
        for (id, account) in document.accounts {
            auth.accounts.insert(
                id.clone(),
                checked_account(
                    &id,
                    &account.provider,
                    &account.base_url,
                    &account.model,
                    account.api_key,
                )?,
            );
        }
        for role in provider::CANONICAL_ROLE_KEYS {
            let input = document.roles.get(role).ok_or_else(bad)?;
            *snapshot.settings.defaults.role_mut(role).ok_or_else(bad)? =
                assignment(&auth, role, input.primary.as_ref(), &input.fallbacks)?;
        }
        snapshot.auth = auth;
        for (id, _) in provider::KEY_ENV {
            snapshot.settings.set_saved_key(id, "");
            snapshot.settings.set_saved_fallback_key(id, "");
        }
        for (id, key) in document.tool_credentials {
            if !provider::KEY_ENV.iter().any(|(known, _)| *known == id)
                || [&key.primary, &key.fallback]
                    .into_iter()
                    .any(|key| key.len() > 4096 || key.chars().any(char::is_control))
            {
                return Err(bad());
            }
            snapshot.settings.set_saved_key(&id, &key.primary);
            snapshot.settings.set_saved_fallback_key(&id, &key.fallback);
        }
        commit(&root, &lock, &snapshot)
    })
    .await?;
    Ok(Json(
        json!({"saved":true,"configuration_revision":revision}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn omitted_tool_slots_preserve_and_only_explicit_empty_clears() {
        let mut settings = provider::SettingsFile::default();
        settings.set_saved_key("firecrawl", "synthetic-primary");
        settings.set_saved_fallback_key("firecrawl", "synthetic-fallback");
        let request: ToolCredential =
            serde_json::from_value(json!({"expected_revision":1,"provider":"firecrawl"}))
                .expect("preserve patch");
        let before = serde_json::to_value(&settings).expect("settings");
        apply_tool_credential(&mut settings, &request);
        assert_eq!(serde_json::to_value(&settings).expect("settings"), before);
        let request: ToolCredential = serde_json::from_value(
            json!({"expected_revision":1,"provider":"firecrawl","primary":""}),
        )
        .expect("explicit clear");
        apply_tool_credential(&mut settings, &request);
        assert!(settings.saved_key("firecrawl").is_empty());
        let after = serde_json::to_value(&settings).expect("settings");
        let mut unchanged = provider::SettingsFile::default();
        unchanged.set_saved_fallback_key("firecrawl", "synthetic-fallback");
        assert_eq!(
            after,
            serde_json::to_value(&unchanged).expect("only fallback retained")
        );
        // Explicit replacement is distinct from both preserve and clear.
        let replacement: ToolCredential = serde_json::from_value(
            json!({"expected_revision":1,"provider":"firecrawl","primary":"new-primary"}),
        )
        .expect("replacement");
        apply_tool_credential(&mut settings, &replacement);
        assert_eq!(settings.saved_key("firecrawl"), "new-primary");
    }

    #[test]
    fn key_only_account_patch_preserves_model_and_hidden_fields() {
        let mut saved = checked_account(
            "local",
            "local",
            "http://127.0.0.1:11434/v1",
            "saved-model",
            Some("old-key".into()),
        )
        .map_err(|error| error.0)
        .expect("account");
        saved.stt_model = Some("saved-transcriber".into());
        let request: AccountInput = serde_json::from_value(json!({"expected_revision":1,"account_id":"local","provider":"local","base_url":"http://127.0.0.1:11434/v1","api_key":"new-key"})).expect("key patch");
        let account = patched_account(Some(&saved), &request)
            .map_err(|error| error.0)
            .expect("valid patch");
        assert_eq!(account.model, saved.model);
        assert_eq!(account.stt_model, saved.stt_model);
        assert_eq!(account.api_key.as_deref(), Some("new-key"));
        let mut changed_origin = request;
        changed_origin.base_url = "http://127.0.0.1:11435/v1".into();
        changed_origin.api_key = None;
        let account = patched_account(Some(&saved), &changed_origin)
            .map_err(|error| error.0)
            .expect("new origin");
        assert!(account.api_key.is_none());
        assert!(account.stt_model.is_none());
    }

    #[test]
    fn current_bundle_reopens_with_durable_revision_and_rejects_stale_form() {
        let root = tempfile::tempdir().expect("isolated configuration");
        let lock = ConfigLock::acquire_at(root.path()).expect("lock");
        let mut snapshot =
            read_locked(root.path(), &lock).unwrap_or_else(|_| panic!("fresh empty current setup"));
        assert!(snapshot.auth.accounts.is_empty());
        snapshot.auth.accounts.insert(
            "local".into(),
            checked_account("local", "local", "http://127.0.0.1:11434/v1", "", None)
                .unwrap_or_else(|_| panic!("local account")),
        );
        let revision = commit(root.path(), &lock, &snapshot)
            .unwrap_or_else(|_| panic!("atomic current commit"));
        drop(lock);
        let lock = ConfigLock::acquire_at(root.path()).expect("reopen lock");
        assert_eq!(lock.generation(), revision);
        let snapshot =
            read_locked(root.path(), &lock).unwrap_or_else(|_| panic!("reopen current bundle"));
        assert!(snapshot
            .settings
            .defaults
            .recon_agent
            .as_ref()
            .is_some_and(|role| role.model.is_empty()));
        drop(lock);
        assert!(matches!(
            set_role(
                root.path(),
                SetRole {
                    expected_revision: revision - 1,
                    role: "brain_summary".into(),
                    primary: None,
                    fallbacks: vec![],
                    variant_updates: BTreeMap::new(),
                }
            ),
            Err(ApiError(StatusCode::CONFLICT, _))
        ));
        assert!(set_role(
            root.path(),
            SetRole {
                expected_revision: revision,
                role: "brain_summary".into(),
                primary: Some(Route {
                    provider: "local".into(),
                    model: "model-fixture".into(),
                    account: "local".into()
                }),
                fallbacks: vec![],
                variant_updates: BTreeMap::new(),
            }
        )
        .is_ok());
    }

    #[test]
    fn legacy_configuration_is_preserved_and_never_migrated_by_read() {
        let root = tempfile::tempdir().expect("isolated old configuration");
        let path = root.path().join("config.toml");
        config_commit::write_secure(
            &path,
            "model = 'legacy'\nfirecrawl_api_key = 'fixture-private'\n",
        )
        .expect("old private file");
        let before = fs::read(&path).expect("old bytes");
        let lock = ConfigLock::acquire_at(root.path()).expect("lock");
        assert!(matches!(
            read_locked(root.path(), &lock),
            Err(ApiError(StatusCode::CONFLICT, _))
        ));
        assert_eq!(fs::read(path).expect("preserved bytes"), before);
        assert!(!root.path().join("auth.json").exists());
    }

    #[cfg(unix)]
    #[test]
    fn configuration_read_refuses_symlink_and_public_secret_file() {
        use std::os::unix::fs::symlink;
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().expect("isolated root");
        let outside = tempfile::tempdir().expect("outside");
        fs::write(outside.path().join("secret"), b"fixture").expect("outside fixture");
        symlink(outside.path().join("secret"), root.path().join("auth.json")).expect("symlink");
        assert!(read_locked(
            root.path(),
            &ConfigLock::acquire_at(root.path()).expect("fixture lock")
        )
        .is_err());
        fs::remove_file(root.path().join("auth.json")).expect("remove fixture link");
        fs::write(root.path().join("auth.json"), b"{}").expect("public fixture");
        fs::set_permissions(
            root.path().join("auth.json"),
            fs::Permissions::from_mode(0o644),
        )
        .expect("fixture mode");
        assert!(read_locked(
            root.path(),
            &ConfigLock::acquire_at(root.path()).expect("fixture lock")
        )
        .is_err());
    }
}
