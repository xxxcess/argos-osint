//! Purpose-authorized request context for public and authenticated private hops.
//!
//! Public context headers are proof references, never caller-assigned origin.
//! Private delegation is accepted only under an InternalService credential with
//! matching scope/epoch and a superset of the delegated permissions. Every
//! registered proof is resolved against current authoritative registry state.

use std::collections::BTreeSet;
use std::fmt;

use argos_osint_core::storage::connect_registry;
use argos_osint_core::storage::connect_registry::RegistryAuthority;
use argos_osint_core::storage::connect_registry::RegistryError;
use argos_osint_core::storage::repo::Repository;
use axum::http::HeaderMap;
use axum::http::HeaderValue;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::response::Response;
use axum::Json;
use fleetworks_contracts::context::CredentialPurpose;
use fleetworks_contracts::context::OriginSource;
use fleetworks_contracts::context::Permission;
use fleetworks_contracts::context::RegisteredContext;
use serde::Deserialize;
use serde::Serialize;
use serde_json::json;

use crate::AuthenticatedPrincipal;

pub(crate) const SESSION_HEADER: &str = "x-fleetworks-session";
pub(crate) const CONTEXT_REVISION_HEADER: &str = "x-fleetworks-context-revision";
pub(crate) const REGISTRATION_HEADER: &str = "x-fleetworks-registration";
pub(crate) const REGISTRATION_REVISION_HEADER: &str = "x-fleetworks-registration-revision";
pub(crate) const DELEGATION_HEADER: &str = "x-fleetworks-delegation";
const MAX_DELEGATION_BYTES: usize = 8192;
const PERMISSIONS: [Permission; 10] = [
    Permission::ReadBackend,
    Permission::RegisterWorkstation,
    Permission::RebindWorkstation,
    Permission::ResolveWebSession,
    Permission::ManageConfiguration,
    Permission::MaintainBackend,
    Permission::SubmitInteractive,
    Permission::ReadWorkstationActivity,
    Permission::RetryJob,
    Permission::CancelJob,
];

/// Global resource reads do not require selecting an arbitrary native folder.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ContextRequirement {
    GlobalRead,
    Registered,
}

/// Session proof cannot accidentally be printed or serialized into job payloads.
#[derive(Clone, Eq, PartialEq)]
pub(crate) struct SessionProofToken(String);

impl SessionProofToken {
    pub(crate) fn expose_secret(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SessionProofToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SessionProofToken([redacted])")
    }
}

/// The actual admission transaction must revalidate this proof, not just a DTO.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum AdmissionProof {
    None,
    Native {
        registration_id: String,
        expected_registration_revision: u64,
    },
    Web {
        session_token: SessionProofToken,
        expected_context_revision: u64,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RequestAuthority {
    credential_id: String,
    scope_id: String,
    access_epoch: String,
    purpose: CredentialPurpose,
    permissions: BTreeSet<Permission>,
}

impl RequestAuthority {
    fn from_principal(principal: &AuthenticatedPrincipal) -> Self {
        Self {
            credential_id: principal.credential_id().to_owned(),
            scope_id: principal.scope_id().to_owned(),
            access_epoch: principal.access_epoch().to_owned(),
            purpose: principal.purpose(),
            permissions: PERMISSIONS
                .into_iter()
                .filter(|permission| principal.permits(*permission))
                .collect(),
        }
    }

    fn registry(&self) -> Result<RegistryAuthority, ContextFailure> {
        RegistryAuthority::new(&self.scope_id, &self.credential_id, &self.access_epoch)
            .map_err(|_| ContextFailure::forbidden())
    }
}

/// Authority and immutable context resolved by a trusted server boundary.
///
/// No Deserialize implementation or public caller-supplied constructor exists.
#[derive(Clone, Debug)]
pub(crate) struct ResolvedRequest {
    authority: RequestAuthority,
    registry: RegistryAuthority,
    context: Option<RegisteredContext>,
    proof: AdmissionProof,
    required_permission: Permission,
}

impl ResolvedRequest {
    pub(crate) fn authority(&self) -> &RegistryAuthority {
        &self.registry
    }
    pub(crate) fn context(&self) -> Option<&RegisteredContext> {
        self.context.as_ref()
    }
    pub(crate) fn proof(&self) -> &AdmissionProof {
        &self.proof
    }
    pub(crate) fn scope_id(&self) -> &str {
        &self.authority.scope_id
    }
    pub(crate) fn access_epoch(&self) -> &str {
        &self.authority.access_epoch
    }
    pub(crate) fn credential_id(&self) -> &str {
        &self.authority.credential_id
    }
    pub(crate) fn purpose(&self) -> CredentialPurpose {
        self.authority.purpose
    }
    pub(crate) fn permits(&self, permission: Permission) -> bool {
        self.authority.permissions.contains(&permission)
    }
    pub(crate) fn registered(&self) -> Result<&RegisteredContext, ContextFailure> {
        self.context
            .as_ref()
            .ok_or_else(ContextFailure::registration_required)
    }
    pub(crate) fn source(&self) -> Option<OriginSource> {
        match &self.proof {
            AdmissionProof::Native { .. } => Some(OriginSource::Native),
            AdmissionProof::Web { .. } => Some(OriginSource::IntelWeb),
            AdmissionProof::None => None,
        }
    }

    /// Bound revocation/expiry checks for long-lived reads and subscriptions.
    pub(crate) async fn revalidate(&self, repo: &Repository) -> Result<(), ContextFailure> {
        let current = resolve_proof(repo, &self.registry, &self.proof).await?;
        match (&self.context, current) {
            (None, None) => Ok(()),
            (Some(previous), Some(current)) if previous.same_request_context(&current) => Ok(()),
            _ => Err(ContextFailure::changed()),
        }
    }

    /// One bounded private header, created from server-resolved authority only.
    pub(crate) fn delegation_header(&self) -> Result<HeaderValue, ContextFailure> {
        let proof = match &self.proof {
            AdmissionProof::None => ProofWire::None,
            AdmissionProof::Native {
                registration_id,
                expected_registration_revision,
            } => ProofWire::Native {
                registration_id: registration_id.clone(),
                revision: *expected_registration_revision,
            },
            AdmissionProof::Web {
                session_token,
                expected_context_revision,
            } => ProofWire::Web {
                session_token: session_token.expose_secret().to_owned(),
                revision: *expected_context_revision,
            },
        };
        let wire = DelegationWire {
            version: 1,
            credential_id: self.authority.credential_id.clone(),
            scope_id: self.authority.scope_id.clone(),
            access_epoch: self.authority.access_epoch.clone(),
            purpose: self.authority.purpose,
            // The internal hop receives only this operation's authority. A
            // Native credential's registration/maintenance grants must not
            // silently become a private Brain or Jobs credential requirement.
            permissions: delegation_permissions(self.required_permission, &self.authority),
            proof,
        };
        let bytes = serde_json::to_vec(&wire).map_err(|_| ContextFailure::unavailable())?;
        if bytes.len() > MAX_DELEGATION_BYTES / 2 {
            return Err(ContextFailure::unavailable());
        }
        let mut value = HeaderValue::from_str(&hex_encode(&bytes))
            .map_err(|_| ContextFailure::unavailable())?;
        value.set_sensitive(true);
        Ok(value)
    }
}

/// Public failures never echo IDs, proof tokens, database errors or headers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ContextFailure {
    status: StatusCode,
    message: &'static str,
}

impl ContextFailure {
    pub(crate) fn status(&self) -> StatusCode {
        self.status
    }
    pub(crate) fn registration_required() -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            message: "Current registered request context required",
        }
    }
    pub(crate) fn forbidden() -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            message: "Request is not permitted in this authority scope",
        }
    }
    pub(crate) fn changed() -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message: "Request context changed; reconnect before retrying",
        }
    }
    pub(crate) fn unavailable() -> Self {
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            message: "Request context authority unavailable",
        }
    }
    pub(crate) fn missing() -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: "Attributed job is unavailable",
        }
    }
    pub(crate) fn invalid() -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: "Invalid bounded request context",
        }
    }
}

impl IntoResponse for ContextFailure {
    fn into_response(self) -> Response {
        let mut response = (self.status, Json(json!({"error":self.message}))).into_response();
        response.headers_mut().insert(
            axum::http::header::CACHE_CONTROL,
            HeaderValue::from_static("no-store, private"),
        );
        response
    }
}

impl From<ContextFailure> for crate::ApiError {
    fn from(error: ContextFailure) -> Self {
        Self(error.status(), error.message)
    }
}

fn registry_failure(error: RegistryError) -> ContextFailure {
    match error {
        RegistryError::RevisionMismatch => ContextFailure::changed(),
        RegistryError::InvalidInput => ContextFailure::invalid(),
        RegistryError::Storage(_)
        | RegistryError::Unconfigured
        | RegistryError::AdmissionPaused => ContextFailure::unavailable(),
        RegistryError::NotFound
        | RegistryError::Revoked
        | RegistryError::Collision
        | RegistryError::ProtectedDefault => ContextFailure::registration_required(),
        _ => ContextFailure::unavailable(),
    }
}

fn one_header<'a>(headers: &'a HeaderMap, name: &str) -> Result<Option<&'a str>, ContextFailure> {
    let mut values = headers.get_all(name).iter();
    let first = values.next();
    if values.next().is_some() {
        return Err(ContextFailure::invalid());
    }
    first
        .map(|value| value.to_str().map_err(|_| ContextFailure::invalid()))
        .transpose()
}

fn revision(value: Option<&str>) -> Result<u64, ContextFailure> {
    let value = value.ok_or_else(ContextFailure::registration_required)?;
    if value.is_empty()
        || value.len() > 20
        || value.starts_with('0')
        || !value.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(ContextFailure::invalid());
    }
    value.parse().map_err(|_| ContextFailure::invalid())
}

fn identity(value: &str) -> bool {
    (1..=128).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn session_token(value: &str) -> Result<SessionProofToken, ContextFailure> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(ContextFailure::registration_required());
    }
    Ok(SessionProofToken(value.to_owned()))
}

fn public_proof(
    authority: &RequestAuthority,
    headers: &HeaderMap,
) -> Result<AdmissionProof, ContextFailure> {
    let session = one_header(headers, SESSION_HEADER)?;
    let context_revision = one_header(headers, CONTEXT_REVISION_HEADER)?;
    let registration = one_header(headers, REGISTRATION_HEADER)?;
    let registration_revision = one_header(headers, REGISTRATION_REVISION_HEADER)?;
    match authority.purpose {
        CredentialPurpose::WebRelay => {
            if !authority
                .permissions
                .contains(&Permission::ResolveWebSession)
                || registration.is_some()
                || registration_revision.is_some()
            {
                return Err(ContextFailure::forbidden());
            }
            Ok(AdmissionProof::Web {
                session_token: session_token(
                    session.ok_or_else(ContextFailure::registration_required)?,
                )?,
                expected_context_revision: revision(context_revision)?,
            })
        }
        CredentialPurpose::Native | CredentialPurpose::Operator => {
            if session.is_some() || context_revision.is_some() {
                return Err(ContextFailure::forbidden());
            }
            match (registration, registration_revision) {
                (None, None) => Ok(AdmissionProof::None),
                (Some(id), Some(expected)) if identity(id) => Ok(AdmissionProof::Native {
                    registration_id: id.to_owned(),
                    expected_registration_revision: revision(Some(expected))?,
                }),
                _ => Err(ContextFailure::registration_required()),
            }
        }
        _ => Err(ContextFailure::forbidden()),
    }
}

async fn resolve_proof(
    repo: &Repository,
    authority: &RegistryAuthority,
    proof: &AdmissionProof,
) -> Result<Option<RegisteredContext>, ContextFailure> {
    match proof {
        AdmissionProof::None => Ok(None),
        AdmissionProof::Native {
            registration_id,
            expected_registration_revision,
        } => connect_registry::registration_context(
            repo,
            authority,
            registration_id,
            *expected_registration_revision,
        )
        .await
        .map(Some)
        .map_err(registry_failure),
        AdmissionProof::Web {
            session_token,
            expected_context_revision,
        } => connect_registry::validate_session(
            repo,
            authority,
            session_token.expose_secret(),
            Some(*expected_context_revision),
        )
        .await
        .map(|session| Some(session.context))
        .map_err(registry_failure),
    }
}

async fn resolve_authority(
    repo: &Repository,
    authority: RequestAuthority,
    proof: AdmissionProof,
    permission: Permission,
    requirement: ContextRequirement,
) -> Result<ResolvedRequest, ContextFailure> {
    if !authority.permissions.contains(&permission) {
        return Err(ContextFailure::forbidden());
    }
    if requirement == ContextRequirement::Registered && proof == AdmissionProof::None {
        return Err(ContextFailure::registration_required());
    }
    let registry = authority.registry()?;
    let context = resolve_proof(repo, &registry, &proof).await?;
    Ok(ResolvedRequest {
        authority,
        registry,
        context,
        proof,
        required_permission: permission,
    })
}

fn delegation_permissions(required: Permission, authority: &RequestAuthority) -> Vec<Permission> {
    let mut permissions = BTreeSet::from([required]);
    if authority.purpose == CredentialPurpose::WebRelay {
        permissions.insert(Permission::ResolveWebSession);
    }
    // Action membership is checked independently from the operation grant.
    if matches!(required, Permission::RetryJob | Permission::CancelJob)
        && authority
            .permissions
            .contains(&Permission::ReadWorkstationActivity)
    {
        permissions.insert(Permission::ReadWorkstationActivity);
    }
    permissions.into_iter().collect()
}

/// Resolve public native/web proof under actual purpose-scoped ingress authority.
pub(crate) async fn resolve_gateway(
    repo: &Repository,
    principal: &AuthenticatedPrincipal,
    headers: &HeaderMap,
    permission: Permission,
    requirement: ContextRequirement,
) -> Result<ResolvedRequest, ContextFailure> {
    let authority = RequestAuthority::from_principal(principal);
    let proof = public_proof(&authority, headers)?;
    resolve_authority(repo, authority, proof, permission, requirement).await
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct DelegationWire {
    version: u16,
    credential_id: String,
    scope_id: String,
    access_epoch: String,
    purpose: CredentialPurpose,
    permissions: Vec<Permission>,
    proof: ProofWire,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum ProofWire {
    None,
    Native {
        registration_id: String,
        revision: u64,
    },
    Web {
        session_token: String,
        revision: u64,
    },
}

/// Private routers accept a delegated request only from their scoped internal caller.
pub(crate) async fn resolve_private(
    repo: &Repository,
    internal: &AuthenticatedPrincipal,
    headers: &HeaderMap,
    permission: Permission,
    requirement: ContextRequirement,
) -> Result<ResolvedRequest, ContextFailure> {
    let internal = RequestAuthority::from_principal(internal);
    if !internal.permissions.contains(&permission) {
        return Err(ContextFailure::forbidden());
    }
    let (authority, proof) = delegated_proof(&internal, headers)?;
    resolve_authority(repo, authority, proof, permission, requirement).await
}

fn delegated_proof(
    internal: &RequestAuthority,
    headers: &HeaderMap,
) -> Result<(RequestAuthority, AdmissionProof), ContextFailure> {
    if internal.purpose != CredentialPurpose::InternalService {
        return Err(ContextFailure::forbidden());
    }
    let raw = one_header(headers, DELEGATION_HEADER)?
        .ok_or_else(ContextFailure::registration_required)?;
    let bytes = hex_decode(raw)?;
    let wire: DelegationWire =
        serde_json::from_slice(&bytes).map_err(|_| ContextFailure::forbidden())?;
    if wire.version != 1
        || wire.scope_id != internal.scope_id
        || wire.access_epoch != internal.access_epoch
        || !identity(&wire.credential_id)
        || wire.permissions.is_empty()
        || wire.permissions.len() > PERMISSIONS.len()
        || wire
            .permissions
            .iter()
            .any(|permission| !internal.permissions.contains(permission))
        || !matches!(
            wire.purpose,
            CredentialPurpose::Native | CredentialPurpose::WebRelay | CredentialPurpose::Operator
        )
    {
        return Err(ContextFailure::forbidden());
    }
    let permissions: BTreeSet<_> = wire.permissions.iter().copied().collect();
    if permissions.len() != wire.permissions.len() {
        return Err(ContextFailure::forbidden());
    }
    let proof = match wire.proof {
        ProofWire::None if wire.purpose != CredentialPurpose::WebRelay => AdmissionProof::None,
        ProofWire::Native {
            registration_id,
            revision,
        } if wire.purpose != CredentialPurpose::WebRelay
            && identity(&registration_id)
            && revision > 0 =>
        {
            AdmissionProof::Native {
                registration_id,
                expected_registration_revision: revision,
            }
        }
        ProofWire::Web {
            session_token: token,
            revision,
        } if wire.purpose == CredentialPurpose::WebRelay
            && permissions.contains(&Permission::ResolveWebSession)
            && revision > 0 =>
        {
            AdmissionProof::Web {
                session_token: session_token(&token)?,
                expected_context_revision: revision,
            }
        }
        _ => return Err(ContextFailure::forbidden()),
    };
    let authority = RequestAuthority {
        credential_id: wire.credential_id,
        scope_id: wire.scope_id,
        access_epoch: wire.access_epoch,
        purpose: wire.purpose,
        permissions,
    };
    Ok((authority, proof))
}

/// Remove caller-supplied delegation/origin claims before resolving public proof.
pub(crate) fn strip_untrusted_delegation(headers: &mut HeaderMap) {
    let remove: Vec<_> = headers
        .keys()
        .filter(|name| {
            name.as_str() == DELEGATION_HEADER
                || name.as_str().starts_with("x-fleetworks-delegated-")
                || name.as_str().starts_with("x-fleetworks-origin")
                || matches!(
                    name.as_str(),
                    "x-fleetworks-workstation-id"
                        | "x-fleetworks-workstation-ip"
                        | "x-fleetworks-workspace-id"
                        | "x-fleetworks-workspace-name"
                        | "x-fleetworks-deployment-id"
                        | "x-fleetworks-source"
                        | "x-fleetworks-authority"
                        | "x-fleetworks-access-epoch"
                        | "x-fleetworks-configuration-epoch"
                        | "x-fleetworks-connect-id"
                )
        })
        .cloned()
        .collect();
    for name in remove {
        headers.remove(name);
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push(HEX[(byte >> 4) as usize] as char);
        text.push(HEX[(byte & 15) as usize] as char);
    }
    text
}

fn hex_decode(value: &str) -> Result<Vec<u8>, ContextFailure> {
    if value.is_empty()
        || value.len() > MAX_DELEGATION_BYTES
        || !value.len().is_multiple_of(2)
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(ContextFailure::forbidden());
    }
    fn nibble(byte: u8) -> u8 {
        if byte <= b'9' {
            byte - b'0'
        } else {
            byte - b'a' + 10
        }
    }
    Ok(value
        .as_bytes()
        .chunks_exact(2)
        .map(|bytes| (nibble(bytes[0]) << 4) | nibble(bytes[1]))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn authority(purpose: CredentialPurpose) -> RequestAuthority {
        RequestAuthority {
            credential_id: "fixture-credential".into(),
            scope_id: "fixture-scope".into(),
            access_epoch: "fixture-epoch".into(),
            purpose,
            permissions: PERMISSIONS.into_iter().collect(),
        }
    }

    #[test]
    fn purpose_does_not_turn_public_claims_into_registered_authority() {
        let mut headers = HeaderMap::new();
        assert_eq!(
            public_proof(&authority(CredentialPurpose::Native), &headers)
                .expect("native global read"),
            AdmissionProof::None
        );
        assert!(public_proof(&authority(CredentialPurpose::WebRelay), &headers).is_err());
        headers.insert(
            REGISTRATION_HEADER,
            HeaderValue::from_static("registration-a"),
        );
        headers.insert(REGISTRATION_REVISION_HEADER, HeaderValue::from_static("1"));
        assert!(matches!(
            public_proof(&authority(CredentialPurpose::Native), &headers),
            Ok(AdmissionProof::Native { .. })
        ));
        assert!(public_proof(&authority(CredentialPurpose::WebRelay), &headers).is_err());
        headers.insert(
            SESSION_HEADER,
            HeaderValue::from_str(&"a".repeat(64)).expect("synthetic token"),
        );
        assert!(public_proof(&authority(CredentialPurpose::Native), &headers).is_err());
        assert!(public_proof(
            &authority(CredentialPurpose::InternalService),
            &HeaderMap::new()
        )
        .is_err());
    }

    #[test]
    fn duplicate_headers_zero_overflow_and_secret_debug_fail_safely() {
        for raw in [
            None,
            Some(""),
            Some("0"),
            Some("01"),
            Some("-1"),
            Some("18446744073709551616"),
            Some("1,2"),
        ] {
            assert!(revision(raw).is_err());
        }
        assert_eq!(revision(Some("1")).expect("positive revision"), 1);
        let mut headers = HeaderMap::new();
        headers.append(SESSION_HEADER, HeaderValue::from_static("first"));
        headers.append(SESSION_HEADER, HeaderValue::from_static("second"));
        assert!(one_header(&headers, SESSION_HEADER).is_err());
        let secret = "a".repeat(64);
        let proof = AdmissionProof::Web {
            session_token: session_token(&secret).expect("synthetic proof"),
            expected_context_revision: 1,
        };
        assert!(!format!("{proof:?}").contains(&secret));
    }

    #[test]
    fn private_envelope_is_bounded_and_claim_headers_are_stripped() {
        let value = br#"{"version":1}"#;
        assert_eq!(hex_decode(&hex_encode(value)).expect("hex fixture"), value);
        for invalid in ["", "0", "gg", "FF"] {
            assert!(hex_decode(invalid).is_err());
        }
        assert!(hex_decode(&"a".repeat(MAX_DELEGATION_BYTES + 2)).is_err());
        let mut headers = HeaderMap::new();
        for name in [
            DELEGATION_HEADER,
            "x-fleetworks-origin-workstation",
            "x-fleetworks-workspace-id",
        ] {
            headers.insert(name, HeaderValue::from_static("forged"));
        }
        headers.insert(
            REGISTRATION_HEADER,
            HeaderValue::from_static("registration-a"),
        );
        strip_untrusted_delegation(&mut headers);
        assert_eq!(headers.len(), 1);
        assert!(headers.contains_key(REGISTRATION_HEADER));
    }

    #[test]
    fn private_delegation_cannot_cross_scope_epoch_purpose_or_permission_ceiling() {
        fn headers(wire: &DelegationWire) -> HeaderMap {
            let mut headers = HeaderMap::new();
            headers.insert(
                DELEGATION_HEADER,
                HeaderValue::from_str(&hex_encode(
                    &serde_json::to_vec(wire).expect("fixture wire"),
                ))
                .expect("fixture header"),
            );
            headers
        }
        let internal = authority(CredentialPurpose::InternalService);
        let mut wire = DelegationWire {
            version: 1,
            credential_id: "native-fixture".into(),
            scope_id: internal.scope_id.clone(),
            access_epoch: internal.access_epoch.clone(),
            purpose: CredentialPurpose::Native,
            permissions: vec![Permission::ReadBackend],
            proof: ProofWire::None,
        };
        let (delegated, proof) =
            delegated_proof(&internal, &headers(&wire)).expect("bounded internal delegation");
        assert_eq!(delegated.credential_id, "native-fixture");
        assert_eq!(proof, AdmissionProof::None);
        assert!(delegated_proof(&authority(CredentialPurpose::Native), &headers(&wire)).is_err());
        wire.scope_id = "another-scope".into();
        assert!(delegated_proof(&internal, &headers(&wire)).is_err());
        wire.scope_id = internal.scope_id.clone();
        wire.access_epoch = "obsolete-epoch".into();
        assert!(delegated_proof(&internal, &headers(&wire)).is_err());
        wire.access_epoch = internal.access_epoch.clone();
        wire.permissions.push(Permission::ReadBackend);
        assert!(delegated_proof(&internal, &headers(&wire)).is_err());
        wire.permissions = vec![Permission::CancelJob];
        let mut limited = internal.clone();
        limited.permissions = [Permission::ReadBackend].into_iter().collect();
        assert!(delegated_proof(&limited, &headers(&wire)).is_err());
        wire.purpose = CredentialPurpose::WebRelay;
        assert!(delegated_proof(&internal, &headers(&wire)).is_err());
        wire.purpose = CredentialPurpose::InternalService;
        assert!(delegated_proof(&internal, &headers(&wire)).is_err());
    }

    #[test]
    fn a_private_read_does_not_delegate_native_registration_or_maintenance() {
        assert_eq!(
            delegation_permissions(
                Permission::ReadBackend,
                &authority(CredentialPurpose::Native)
            ),
            vec![Permission::ReadBackend]
        );
        let web = delegation_permissions(
            Permission::SubmitInteractive,
            &authority(CredentialPurpose::WebRelay),
        );
        assert_eq!(
            web.into_iter().collect::<BTreeSet<_>>(),
            [Permission::SubmitInteractive, Permission::ResolveWebSession]
                .into_iter()
                .collect()
        );
        let mut native = authority(CredentialPurpose::Native);
        native
            .permissions
            .remove(&Permission::ReadWorkstationActivity);
        assert_eq!(
            delegation_permissions(Permission::CancelJob, &native),
            vec![Permission::CancelJob]
        );
    }
}
