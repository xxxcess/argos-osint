//! Ingress policy shared by the HTTP routes and the WebSocket upgrade.

use std::collections::BTreeSet;
use std::fs;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::sync::Arc;

use anyhow::ensure;
use argos_osint_core::storage::configuration_reset::CredentialFingerprint;
use argos_osint_core::storage::configuration_reset::LoadedUserCredential;
use argos_osint_core::storage::configuration_reset::OwnerObservation;
use argos_osint_core::storage::configuration_reset::ResetAuthority;
use argos_osint_core::storage::configuration_reset::ResetError;
use argos_osint_core::storage::configuration_reset::RuntimeObservation;
use axum::extract::{Request, State};
use axum::http::header;
use axum::http::uri::Authority;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use fleetworks_contracts::context::CredentialPurpose;
use fleetworks_contracts::context::Permission;
use serde::Deserialize;
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

const MIN_TOKEN_LEN: usize = 32;
const MAX_TOKEN_LEN: usize = 256;

/// Explicit opt-in ingress policy for the Fleetworks API.
#[derive(Clone)]
pub struct NetworkPolicy {
    hosts: Vec<Authority>,
    credentials: Vec<ScopedCredential>,
}

/// Server-issued authority associated with one validated credential.
///
/// Scope identifies the deployment's existing authorization boundary; it does
/// not imply personal identity or device attestation. Request bodies and
/// forwarded browser headers cannot construct this authority.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthenticatedPrincipal {
    credential_id: String,
    scope_id: String,
    access_epoch: String,
    purpose: CredentialPurpose,
    permissions: BTreeSet<Permission>,
}

impl AuthenticatedPrincipal {
    /// Non-secret server-assigned credential identifier.
    pub fn credential_id(&self) -> &str {
        &self.credential_id
    }

    /// Existing authorized deployment scope, not a body-supplied owner.
    pub fn scope_id(&self) -> &str {
        &self.scope_id
    }

    /// Credential/reset epoch used to isolate obsolete access and caches.
    pub fn access_epoch(&self) -> &str {
        &self.access_epoch
    }

    /// The operator-provisioned credential purpose.
    pub fn purpose(&self) -> CredentialPurpose {
        self.purpose
    }

    /// Whether the server granted this operation to the credential.
    pub fn permits(&self, permission: Permission) -> bool {
        self.permissions.contains(&permission)
    }

    /// Effective server-issued grants for safe capability presentation.
    pub fn permissions(&self) -> impl Iterator<Item = Permission> + '_ {
        self.permissions.iter().copied()
    }
}

/// A purpose-scoped bearer credential. Only a digest is retained in memory.
///
/// Deliberately does not implement Debug/Serialize: credential digests and
/// policy internals are not diagnostic payloads.
#[derive(Clone)]
pub struct ScopedCredential {
    token_digest: [u8; 32],
    principal: Option<AuthenticatedPrincipal>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CredentialManifest {
    version: u16,
    scope_id: String,
    access_epoch: String,
    credentials: Vec<CredentialEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CredentialEntry {
    id: String,
    purpose: CredentialPurpose,
    permissions: BTreeSet<Permission>,
    token_file: std::path::PathBuf,
}

fn protected_bytes(path: &Path, limit: u64) -> anyhow::Result<Vec<u8>> {
    let before =
        fs::symlink_metadata(path).map_err(|_| anyhow::anyhow!("credential file unavailable"))?;
    ensure!(
        before.is_file() && !before.file_type().is_symlink(),
        "credential input must be a regular file"
    );
    let file = File::open(path).map_err(|_| anyhow::anyhow!("credential file unavailable"))?;
    let metadata = file.metadata()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        ensure!(
            metadata.mode() & 0o077 == 0 && metadata.nlink() == 1,
            "credential input must be private and singly linked"
        );
        ensure!(
            metadata.dev() == before.dev() && metadata.ino() == before.ino(),
            "credential input changed while opening"
        );
    }
    ensure!(
        metadata.is_file() && metadata.len() <= limit,
        "credential input exceeds its bound"
    );
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= limit,
        "credential input exceeds its bound"
    );
    Ok(bytes)
}

impl ScopedCredential {
    /// Create explicit server authority from externally provisioned credentials.
    ///
    /// The operator must provision a distinct token for each purpose. Merely
    /// knowing a Connect ID never grants these permissions.
    pub fn new(
        credential_id: String,
        scope_id: String,
        access_epoch: String,
        purpose: CredentialPurpose,
        permissions: BTreeSet<Permission>,
        token: &str,
    ) -> anyhow::Result<Self> {
        ensure!(valid_token(token.as_bytes()), "invalid scoped API bearer");
        for identity in [&credential_id, &scope_id, &access_epoch] {
            ensure!(
                (1..=128).contains(&identity.len())
                    && identity
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric()
                            || matches!(byte, b'-' | b'_' | b'.')),
                "invalid API credential identity"
            );
        }
        ensure!(
            !permissions.is_empty(),
            "scoped API credential requires permissions"
        );
        ensure!(
            purpose != CredentialPurpose::WebRelay
                || (!permissions.contains(&Permission::RegisterWorkstation)
                    && !permissions.contains(&Permission::RebindWorkstation)
                    && !permissions.contains(&Permission::MaintainBackend)
                    && !permissions.contains(&Permission::ManageConfiguration)),
            "web relay cannot receive native registration or maintenance authority"
        );
        Ok(Self {
            token_digest: Sha256::digest(token).into(),
            principal: Some(AuthenticatedPrincipal {
                credential_id,
                scope_id,
                access_epoch,
                purpose,
                permissions,
            }),
        })
    }
}

impl NetworkPolicy {
    /// Observe this exact loaded policy, never the potentially replaced token
    /// files. A server restart/reprovision must replace both ingress and owner.
    pub(crate) fn reset_observation(
        &self,
        authority: &ResetAuthority,
        deployment_id: &str,
        owners: Vec<OwnerObservation>,
    ) -> Result<RuntimeObservation, ResetError> {
        if self
            .credentials
            .iter()
            .any(|entry| entry.principal.is_none())
        {
            return Err(ResetError::UnsupportedBuild);
        }
        let operator = self
            .credentials
            .iter()
            .find(|entry| {
                entry.principal.as_ref().is_some_and(|principal| {
                    principal.credential_id() == authority.credential_id()
                        && principal.scope_id() == authority.scope_id()
                        && principal.purpose() == CredentialPurpose::Operator
                        && principal.permits(Permission::ManageConfiguration)
                        && principal.permits(Permission::MaintainBackend)
                })
            })
            .ok_or(ResetError::Forbidden)?;
        let users = self
            .credentials
            .iter()
            .filter_map(|entry| {
                let principal = entry.principal.as_ref()?;
                matches!(
                    principal.purpose(),
                    CredentialPurpose::Native | CredentialPurpose::WebRelay
                )
                .then(|| LoadedUserCredential {
                    scope_id: principal.scope_id().to_owned(),
                    purpose: principal.purpose(),
                    access_epoch: principal.access_epoch().to_owned(),
                    fingerprint: CredentialFingerprint::from_validated_policy_digest(
                        entry.token_digest,
                    ),
                })
            })
            .collect();
        Ok(RuntimeObservation {
            scope_id: authority.scope_id().to_owned(),
            deployment_id: deployment_id.to_owned(),
            independent_operator_id: authority.credential_id().to_owned(),
            independent_operator_fingerprint: CredentialFingerprint::from_validated_policy_digest(
                operator.token_digest,
            ),
            users,
            owners,
        })
    }

    pub(crate) fn owner_epoch(&self, scope: &str) -> Result<String, ResetError> {
        let mut epochs = BTreeSet::new();
        for entry in &self.credentials {
            let principal = entry
                .principal
                .as_ref()
                .ok_or(ResetError::UnsupportedBuild)?;
            if principal.purpose() != CredentialPurpose::InternalService
                || principal.scope_id() != scope
            {
                return Err(ResetError::Forbidden);
            }
            epochs.insert(principal.access_epoch().to_owned());
        }
        if epochs.len() != 1 {
            return Err(ResetError::Conflict);
        }
        epochs.into_iter().next().ok_or(ResetError::Unavailable)
    }

    pub(crate) fn owner_scope(&self) -> Result<String, ResetError> {
        let principal = self
            .credentials
            .first()
            .and_then(|entry| entry.principal.as_ref())
            .ok_or(ResetError::UnsupportedBuild)?;
        let scope = principal.scope_id().to_owned();
        self.owner_epoch(&scope)?;
        Ok(scope)
    }
    /// Load the current purpose-credential format during operator startup.
    ///
    /// Call on a blocking worker. The manifest contains only private token-file
    /// references, never inline bearer bytes. An explicitly selected invalid
    /// manifest is an error; callers must not fall back to an old shared token.
    pub fn from_credentials_file(hosts: &[String], path: &Path) -> anyhow::Result<Self> {
        ensure!(
            path.is_absolute(),
            "credential manifest must be an absolute path"
        );
        let manifest: CredentialManifest =
            serde_json::from_slice(&protected_bytes(path, 64 * 1024)?)
                .map_err(|_| anyhow::anyhow!("invalid purpose credential manifest"))?;
        ensure!(
            manifest.version == 1 && (1..=64).contains(&manifest.credentials.len()),
            "unsupported purpose credential manifest"
        );
        let mut identities = BTreeSet::new();
        let mut credentials = Vec::with_capacity(manifest.credentials.len());
        for entry in manifest.credentials {
            ensure!(
                identities.insert(entry.id.clone()),
                "duplicate API credential identity"
            );
            let token_path = if entry.token_file.is_absolute() {
                entry.token_file
            } else {
                path.parent()
                    .expect("absolute manifest has a parent")
                    .join(entry.token_file)
            };
            let token = String::from_utf8(protected_bytes(&token_path, 4096)?)
                .map_err(|_| anyhow::anyhow!("invalid scoped API bearer"))?;
            credentials.push(ScopedCredential::new(
                entry.id,
                manifest.scope_id.clone(),
                manifest.access_epoch.clone(),
                entry.purpose,
                entry.permissions,
                token.trim_end_matches(['\r', '\n']),
            )?);
        }
        Self::with_credentials(hosts, credentials)
    }

    /// Accepts exact Host authorities and a 32–256 byte bearer token.
    ///
    /// Include the published port in each authority when clients send it.
    pub fn new(hosts: &[String], token: &str) -> anyhow::Result<Self> {
        ensure!(
            valid_token(token.as_bytes()),
            "API bearer token must be 32–256 bytes in bearer token syntax"
        );
        // Existing operator/private-service consumers retain their current
        // routes, but this compatibility constructor grants no new purpose
        // permissions. Registry/maintenance routes require an explicit principal.
        Self::with_credentials(
            hosts,
            vec![ScopedCredential {
                token_digest: Sha256::digest(token).into(),
                principal: None,
            }],
        )
    }

    /// Configure distinct externally provisioned credentials and permissions.
    pub fn with_credentials(
        hosts: &[String],
        credentials: Vec<ScopedCredential>,
    ) -> anyhow::Result<Self> {
        ensure!(!hosts.is_empty(), "at least one API host is required");
        ensure!(
            !credentials.is_empty() && credentials.len() <= 64,
            "API requires 1–64 credentials"
        );
        for (index, credential) in credentials.iter().enumerate() {
            ensure!(
                !credentials[..index]
                    .iter()
                    .any(|prior| bool::from(prior.token_digest.ct_eq(&credential.token_digest))),
                "API credential tokens must be distinct"
            );
        }
        let hosts = hosts
            .iter()
            .map(|host| {
                let authority: Authority = host
                    .parse()
                    .map_err(|_| anyhow::anyhow!("invalid API host authority"))?;
                let port_suffix = &authority.as_str()[authority.host().len()..];
                let valid_port = port_suffix.is_empty()
                    || port_suffix
                        .strip_prefix(':')
                        .is_some_and(|port| port.parse::<u16>().is_ok_and(|number| number != 0));
                ensure!(
                    !authority.host().is_empty() && !host.contains('@') && valid_port,
                    "invalid API host authority"
                );
                Ok(authority)
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        Ok(Self { hosts, credentials })
    }

    fn allows_host(&self, host: &str) -> bool {
        self.hosts
            .iter()
            .any(|allowed| allowed.as_str().eq_ignore_ascii_case(host))
    }

    fn credential(&self, headers: &HeaderMap) -> Option<&ScopedCredential> {
        let values = headers.get_all(header::AUTHORIZATION);
        let mut values = values.iter();
        let value = values.next()?;
        let bytes = value.as_bytes();
        if values.next().is_some()
            || bytes.len() < 7
            || !bytes[..7].eq_ignore_ascii_case(b"Bearer ")
        {
            return None;
        }
        let token = &bytes[7..];
        if !valid_token(token) {
            return None;
        }
        let digest: [u8; 32] = Sha256::digest(token).into();
        let mut found = None;
        for credential in &self.credentials {
            if bool::from(credential.token_digest.ct_eq(&digest)) {
                found = Some(credential);
            }
        }
        found
    }
}

pub(crate) fn valid_token(token: &[u8]) -> bool {
    if !(MIN_TOKEN_LEN..=MAX_TOKEN_LEN).contains(&token.len()) {
        return false;
    }
    let value = token.trim_ascii_end();
    let end = value.iter().rposition(|byte| *byte != b'=');
    end.is_some_and(|end| {
        value.len() == token.len()
            && value[..=end].iter().all(|byte| {
                byte.is_ascii_alphanumeric()
                    || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'+' | b'/')
            })
    })
}

/// Validates and canonicalizes an exact HTTP(S) browser origin.
pub fn network_origin(raw: &str) -> anyhow::Result<String> {
    Ok(validate_origin(raw)?.origin().ascii_serialization())
}

pub(crate) fn validate_origin(raw: &str) -> anyhow::Result<url::Url> {
    let origin = url::Url::parse(raw).map_err(|_| anyhow::anyhow!("invalid API origin"))?;
    ensure!(
        matches!(origin.scheme(), "http" | "https")
            && origin.host_str().is_some()
            && origin.username().is_empty()
            && origin.password().is_none()
            && origin.path() == "/"
            && origin.query().is_none()
            && origin.fragment().is_none(),
        "invalid API origin"
    );
    Ok(origin)
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AccessMode {
    Loopback,
    Bearer,
}

#[derive(Clone)]
pub(crate) struct AccessPolicy {
    pub(crate) port: u16,
    pub(crate) origins: Arc<Vec<String>>,
    pub(crate) network: Option<NetworkPolicy>,
}

pub(crate) async fn protect(
    State(policy): State<AccessPolicy>,
    mut request: Request,
    next: Next,
) -> Response {
    let headers = request.headers();
    let host = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    let allowed_host = match &policy.network {
        Some(network) => {
            headers.get_all(header::HOST).iter().count() == 1 && network.allows_host(host)
        }
        None => {
            host == format!("127.0.0.1:{}", policy.port)
                || host == format!("localhost:{}", policy.port)
                || (policy.port == 80 && matches!(host, "127.0.0.1" | "localhost"))
        }
    };
    if !allowed_host {
        return refused(StatusCode::FORBIDDEN, "Host refused");
    }

    let origin = headers.get(header::ORIGIN).cloned();
    if let Some(value) = &origin {
        if (policy.network.is_some() && headers.get_all(header::ORIGIN).iter().count() != 1)
            || !policy
                .origins
                .iter()
                .any(|allowed| value.as_bytes() == allowed.as_bytes())
        {
            return refused(StatusCode::FORBIDDEN, "Origin refused");
        }
    }
    if headers
        .get("sec-fetch-site")
        .is_some_and(|value| value == "cross-site")
        && origin.is_none()
    {
        return refused(StatusCode::FORBIDDEN, "Origin required");
    }

    let credential = policy
        .network
        .as_ref()
        .and_then(|network| network.credential(headers));
    let authenticated = policy.network.is_none() || credential.is_some();
    let principal = credential.and_then(|credential| credential.principal.clone());
    let fingerprint = credential.map(|credential| {
        CredentialFingerprint::from_validated_policy_digest(credential.token_digest)
    });
    let mut response = if request.method() == Method::OPTIONS {
        StatusCode::NO_CONTENT.into_response()
    } else if !authenticated {
        let mut response = refused(StatusCode::UNAUTHORIZED, "API authentication required");
        response
            .headers_mut()
            .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        response
    } else {
        if let Some(principal) = principal {
            request.extensions_mut().insert(principal);
        }
        if let Some(fingerprint) = fingerprint {
            request.extensions_mut().insert(fingerprint);
        }
        next.run(request).await
    };
    let headers = response.headers_mut();
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store, private"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(header::VARY, HeaderValue::from_static("Origin"));
    if let Some(value) = origin {
        headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, value);
        headers.insert(
            header::ACCESS_CONTROL_ALLOW_METHODS,
            HeaderValue::from_static("GET, POST, OPTIONS"),
        );
        let allowed = if policy.network.is_some() {
            "authorization, content-type"
        } else {
            "content-type"
        };
        headers.insert(
            header::ACCESS_CONTROL_ALLOW_HEADERS,
            HeaderValue::from_static(allowed),
        );
    }
    response
}

fn refused(status: StatusCode, reason: &'static str) -> Response {
    (status, Json(json!({"error": reason}))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use axum::middleware;
    use axum::routing::get;
    use axum::Router;
    use tower::ServiceExt;

    const TOKEN: &str = "0123456789abcdefghijklmnopqrstuvwxyz";

    fn private_write(path: &Path, bytes: &[u8]) {
        use std::io::Write;
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(path).expect("private fixture file");
        file.write_all(bytes).expect("fixture bytes");
    }

    fn credential_manifest() -> serde_json::Value {
        serde_json::json!({
            "version":1,"scope_id":"fixture-scope","access_epoch":"fixture-epoch",
            "credentials":[
                {"id":"native","purpose":"native","permissions":["read_backend","register_workstation"],"token_file":"native-token"},
                {"id":"web","purpose":"web_relay","permissions":["resolve_web_session"],"token_file":"web-token"}
            ]
        })
    }

    #[test]
    fn private_manifest_resolves_distinct_relative_tokens_without_escalating_grants() {
        let root = tempfile::tempdir().expect("isolated credential directory");
        private_write(
            &root.path().join("native-token"),
            format!("{TOKEN}\n").as_bytes(),
        );
        let web_token = "syntheticwebrelay0123456789abcdefgh";
        private_write(&root.path().join("web-token"), web_token.as_bytes());
        let path = root.path().join("credentials.json");
        private_write(
            &path,
            &serde_json::to_vec(&credential_manifest()).expect("manifest"),
        );
        let policy = NetworkPolicy::from_credentials_file(&["api:8080".into()], &path)
            .expect("current private manifest");
        for (token, purpose, granted, denied) in [
            (
                TOKEN,
                CredentialPurpose::Native,
                Permission::RegisterWorkstation,
                Permission::MaintainBackend,
            ),
            (
                web_token,
                CredentialPurpose::WebRelay,
                Permission::ResolveWebSession,
                Permission::RegisterWorkstation,
            ),
        ] {
            let mut headers = HeaderMap::new();
            headers.insert(
                header::AUTHORIZATION,
                format!("Bearer {token}").parse().expect("fixture bearer"),
            );
            let principal = policy
                .credential(&headers)
                .and_then(|credential| credential.principal.as_ref())
                .expect("explicit principal");
            assert_eq!(principal.purpose(), purpose);
            assert_eq!(principal.scope_id(), "fixture-scope");
            assert_eq!(principal.access_epoch(), "fixture-epoch");
            assert!(principal.permits(granted) && !principal.permits(denied));
        }
    }

    #[test]
    fn malformed_manifest_duplicate_tokens_and_inline_secrets_are_rejected() {
        for variant in [
            "version",
            "identity",
            "permissions",
            "duplicate_token",
            "inline_secret",
            "missing_token",
        ] {
            let root = tempfile::tempdir().expect("isolated credential directory");
            private_write(&root.path().join("native-token"), TOKEN.as_bytes());
            let web = if variant == "duplicate_token" {
                TOKEN
            } else {
                "syntheticwebrelay0123456789abcdefgh"
            };
            private_write(&root.path().join("web-token"), web.as_bytes());
            let mut manifest = credential_manifest();
            match variant {
                "version" => manifest["version"] = 0.into(),
                "identity" => manifest["credentials"][1]["id"] = "native".into(),
                "permissions" => {
                    manifest["credentials"][1]["permissions"] =
                        serde_json::json!(["manage_configuration"])
                }
                "inline_secret" => manifest["credentials"][0]["token"] = TOKEN.into(),
                "missing_token" => manifest["credentials"][0]["token_file"] = "absent".into(),
                _ => {}
            }
            let path = root.path().join("credentials.json");
            private_write(&path, &serde_json::to_vec(&manifest).expect("manifest"));
            assert!(
                NetworkPolicy::from_credentials_file(&["api:8080".into()], &path).is_err(),
                "accepted invalid {variant} manifest"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn credential_files_reject_public_permissions_links_and_oversized_inputs() {
        use std::os::unix::fs::symlink;
        use std::os::unix::fs::PermissionsExt;
        for variant in [
            "public_manifest",
            "public_token",
            "symlink",
            "hardlink",
            "oversized",
        ] {
            let root = tempfile::tempdir().expect("isolated credential directory");
            let token = root.path().join("native-token");
            private_write(&token, TOKEN.as_bytes());
            private_write(
                &root.path().join("web-token"),
                b"syntheticwebrelay0123456789abcdefgh",
            );
            let mut manifest = credential_manifest();
            match variant {
                "public_token" => fs::set_permissions(&token, fs::Permissions::from_mode(0o644))
                    .expect("fixture permissions"),
                "symlink" => {
                    symlink(&token, root.path().join("linked")).expect("fixture symlink");
                    manifest["credentials"][0]["token_file"] = "linked".into();
                }
                "hardlink" => {
                    fs::hard_link(&token, root.path().join("linked")).expect("fixture hardlink");
                }
                "oversized" => {
                    fs::write(&token, vec![b'a'; 4097]).expect("oversized token");
                }
                _ => {}
            }
            let path = root.path().join("credentials.json");
            private_write(&path, &serde_json::to_vec(&manifest).expect("manifest"));
            if variant == "public_manifest" {
                fs::set_permissions(&path, fs::Permissions::from_mode(0o644))
                    .expect("fixture permissions");
            }
            assert!(
                NetworkPolicy::from_credentials_file(&["api:8080".into()], &path).is_err(),
                "accepted unsafe {variant} input"
            );
        }
    }

    fn scoped(purpose: CredentialPurpose, permissions: &[Permission]) -> ScopedCredential {
        ScopedCredential::new(
            "review-native".into(),
            "review-deployment".into(),
            "fresh-config-v1".into(),
            purpose,
            permissions.iter().copied().collect(),
            TOKEN,
        )
        .unwrap()
    }

    #[test]
    fn web_credentials_cannot_acquire_registration_or_maintenance_authority() {
        for permission in [
            Permission::RegisterWorkstation,
            Permission::RebindWorkstation,
            Permission::MaintainBackend,
            Permission::ManageConfiguration,
        ] {
            assert!(ScopedCredential::new(
                "web".into(),
                "deployment".into(),
                "epoch-1".into(),
                CredentialPurpose::WebRelay,
                BTreeSet::from([permission]),
                TOKEN,
            )
            .is_err());
        }
        let web = scoped(
            CredentialPurpose::WebRelay,
            &[Permission::ResolveWebSession],
        );
        assert!(
            NetworkPolicy::with_credentials(&["api:8080".into()], vec![web.clone(), web]).is_err()
        );
    }

    #[test]
    fn old_bearer_authentication_does_not_grant_new_purpose_permissions() {
        let policy = NetworkPolicy::new(&["api:8080".into()], TOKEN).unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {TOKEN}").parse().unwrap(),
        );
        assert!(policy.credential(&headers).unwrap().principal.is_none());
        headers.insert(
            "x-fleetworks-permissions",
            "register_workstation".parse().unwrap(),
        );
        assert!(policy.credential(&headers).unwrap().principal.is_none());
    }

    #[tokio::test]
    async fn ingress_injects_only_server_granted_principal_after_authentication() {
        use axum::Extension;

        async fn registration(principal: Option<Extension<AuthenticatedPrincipal>>) -> StatusCode {
            if principal.is_some_and(|Extension(principal)| {
                principal.permits(Permission::RegisterWorkstation)
            }) {
                StatusCode::OK
            } else {
                StatusCode::FORBIDDEN
            }
        }

        let policies = [
            (
                NetworkPolicy::new(&["api:8080".into()], TOKEN).unwrap(),
                StatusCode::FORBIDDEN,
            ),
            (
                NetworkPolicy::with_credentials(
                    &["api:8080".into()],
                    vec![scoped(
                        CredentialPurpose::Native,
                        &[Permission::RegisterWorkstation],
                    )],
                )
                .unwrap(),
                StatusCode::OK,
            ),
            (
                NetworkPolicy::with_credentials(
                    &["api:8080".into()],
                    vec![scoped(
                        CredentialPurpose::WebRelay,
                        &[Permission::ResolveWebSession],
                    )],
                )
                .unwrap(),
                StatusCode::FORBIDDEN,
            ),
        ];
        for (network, expected) in policies {
            let access = AccessPolicy {
                port: 8080,
                origins: Arc::new(Vec::new()),
                network: Some(network),
            };
            let app = Router::new()
                .route("/registration", get(registration))
                .layer(middleware::from_fn_with_state(access, protect));
            let request = Request::builder()
                .uri("/registration")
                .header(header::HOST, "api:8080")
                .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
                .header("x-fleetworks-scope-id", "forged-owner")
                .header("x-fleetworks-permissions", "register_workstation")
                .body(Body::empty())
                .unwrap();
            assert_eq!(app.oneshot(request).await.unwrap().status(), expected);
        }
    }

    #[test]
    fn rejects_invalid_config_without_reflecting_it() {
        for host in ["", "https://api:8080", "name@api:8080", "api:70000"] {
            let result = NetworkPolicy::new(&[host.into()], TOKEN);
            assert!(result.is_err(), "{host}");
        }
        assert!(NetworkPolicy::new(&["api:8080".into()], "secret").is_err());
        assert_eq!(
            network_origin("https://intel.example.test:443/").unwrap(),
            "https://intel.example.test"
        );
        for origin in [
            "null",
            "ftp://intel.example.test",
            "https://secret@intel.example.test",
            "https://intel.example.test/app",
            "https://intel.example.test?token=secret",
        ] {
            let error = network_origin(origin).unwrap_err();
            assert_eq!(error.to_string(), "invalid API origin");
        }
    }

    #[tokio::test]
    async fn ingress_rejects_wrong_hosts_origins_and_bearers_before_dispatch() {
        let access = AccessPolicy {
            port: 8080,
            origins: Arc::new(vec!["http://localhost:3000".into()]),
            network: Some(NetworkPolicy::new(&["api:8080".into()], TOKEN).unwrap()),
        };
        let app = Router::new()
            .route("/api/v1/live", get(|| async { StatusCode::OK }))
            .layer(middleware::from_fn_with_state(access, protect));
        for (method, host, origin, token, status) in [
            (
                "GET",
                "other:8080",
                None,
                Some(TOKEN),
                StatusCode::FORBIDDEN,
            ),
            (
                "GET",
                "api:8080",
                Some("http://other:3000"),
                Some(TOKEN),
                StatusCode::FORBIDDEN,
            ),
            ("GET", "api:8080", None, None, StatusCode::UNAUTHORIZED),
            (
                "GET",
                "api:8080",
                None,
                Some("0123456789abcdefghijklmnopqrstuvwx00"),
                StatusCode::UNAUTHORIZED,
            ),
            ("GET", "API:8080", None, Some(TOKEN), StatusCode::OK),
            (
                "OPTIONS",
                "api:8080",
                Some("http://localhost:3000"),
                None,
                StatusCode::NO_CONTENT,
            ),
        ] {
            let mut request = Request::builder()
                .method(method)
                .uri("/api/v1/live")
                .header(header::HOST, host);
            if let Some(origin) = origin {
                request = request.header(header::ORIGIN, origin);
            }
            if let Some(token) = token {
                request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
            }
            let response = app
                .clone()
                .oneshot(request.body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), status, "{method} {host} {origin:?}");
            if status == StatusCode::UNAUTHORIZED {
                assert_eq!(response.headers()[header::WWW_AUTHENTICATE], "Bearer");
            }
            if status == StatusCode::NO_CONTENT {
                assert_eq!(
                    response.headers()[header::ACCESS_CONTROL_ALLOW_HEADERS],
                    "authorization, content-type"
                );
            }
        }
    }
}
