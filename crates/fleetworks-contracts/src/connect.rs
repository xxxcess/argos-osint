//! Canonical Connect IDs and the registered-workstation wire contract.
//!
//! An IP address, folder marker or Connect ID is context, never a credential.
//! The gateway derives ownership from its authenticated caller and enforces
//! uniqueness, permissions, protection and revocation in its authoritative store.

use std::fmt;
use std::net::IpAddr;
use std::str::FromStr;

use serde::de;
use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use serde::Serializer;
use unicode_normalization::UnicodeNormalization;

/// Current registry/Connect contract version.
pub const CONNECT_CONTRACT_VERSION: u16 = 2;
/// Native-only authenticated registration creation/reconciliation route.
pub const REGISTRATION_ENSURE_PATH: &str = "/api/v1/connect/registrations/ensure";
/// Purpose-authorized browser session creation route.
pub const WEB_SESSION_CREATE_PATH: &str = "/api/v1/connect/sessions";
/// Maximum UTF-8 bytes accepted for a workspace basename, before and after NFC.
pub const MAX_WORKSPACE_NAME_BYTES: usize = 128;
/// Maximum bytes in an input Connect ID, including IP, brackets and separator.
pub const MAX_CONNECT_ID_BYTES: usize = 128;
/// Protected default child name. The parent container has no Connect ID.
pub const DEFAULT_WORKSPACE_NAME: &str = "default";

/// Why a folder basename cannot be used as a managed child name.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum WorkspaceNameError {
    Empty,
    TooLong,
    Whitespace,
    HiddenOrTraversal,
    Separator,
    Control,
    ProtectedDefault,
}

impl fmt::Display for WorkspaceNameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "workspace name is empty",
            Self::TooLong => "workspace name exceeds the UTF-8 byte limit",
            Self::Whitespace => "workspace name has leading or trailing whitespace",
            Self::HiddenOrTraversal => "hidden and traversal workspace names are reserved",
            Self::Separator => "workspace name contains a path or Connect ID separator",
            Self::Control => "workspace name contains a control character",
            Self::ProtectedDefault => "the default workspace is reserved and protected",
        })
    }
}

impl std::error::Error for WorkspaceNameError {}

/// A validated NFC basename, preserving case and internal spaces.
///
/// This type validates text only. It does not prove that a directory is owned,
/// contained, unique on the local filesystem, safe to adopt, or safe to delete.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct WorkspaceName(String);

impl WorkspaceName {
    /// Validate a basename for registration or an existing child.
    pub fn new(value: &str) -> Result<Self, WorkspaceNameError> {
        if value.is_empty() {
            return Err(WorkspaceNameError::Empty);
        }
        if value.len() > MAX_WORKSPACE_NAME_BYTES {
            return Err(WorkspaceNameError::TooLong);
        }
        if value.trim() != value {
            return Err(WorkspaceNameError::Whitespace);
        }
        if value.starts_with('.') {
            return Err(WorkspaceNameError::HiddenOrTraversal);
        }
        if value
            .chars()
            .any(|character| matches!(character, '/' | '\\' | ':'))
        {
            return Err(WorkspaceNameError::Separator);
        }
        if value.chars().any(char::is_control) {
            return Err(WorkspaceNameError::Control);
        }
        let normalized: String = value.nfc().collect();
        if normalized.len() > MAX_WORKSPACE_NAME_BYTES {
            return Err(WorkspaceNameError::TooLong);
        }
        Ok(Self(normalized))
    }

    /// Validate a user-created non-default child, reserving default case aliases.
    pub fn new_child(value: &str) -> Result<Self, WorkspaceNameError> {
        let name = Self::new(value)?;
        if name.as_str().eq_ignore_ascii_case(DEFAULT_WORKSPACE_NAME) {
            return Err(WorkspaceNameError::ProtectedDefault);
        }
        Ok(name)
    }

    /// Return the normalized basename, without a parent path.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether this is the exact reserved default basename.
    ///
    /// Server-side persisted protection and local object identity remain
    /// authoritative; this convenience method cannot authorize deletion.
    #[must_use]
    pub fn is_default(&self) -> bool {
        self.0 == DEFAULT_WORKSPACE_NAME
    }
}

impl fmt::Display for WorkspaceName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl AsRef<str> for WorkspaceName {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl FromStr for WorkspaceName {
    type Err = WorkspaceNameError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl TryFrom<String> for WorkspaceName {
    type Error = WorkspaceNameError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(&value)
    }
}

impl Serialize for WorkspaceName {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for WorkspaceName {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::new(&value).map_err(de::Error::custom)
    }
}

/// A malformed opaque workstation Connect ID. Legacy IP:workspace IDs are rejected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConnectIdError;

impl fmt::Display for ConnectIdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("expected an opaque workstation ID (1–128 ASCII letters, digits, dash, underscore or dot)")
    }
}

impl std::error::Error for ConnectIdError {}

/// Stable backend workstation identity; IP is optional mutable metadata, never identity.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ConnectId(String);

impl ConnectId {
    /// Validate a backend-issued workstation identifier, rejecting legacy ID shapes.
    pub fn new(value: &str) -> Result<Self, ConnectIdError> {
        if !(1..=128).contains(&value.len())
            || !value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        {
            return Err(ConnectIdError);
        }
        Ok(Self(value.into()))
    }

    /// Return the exact, case-preserving workstation ID.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ConnectId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl FromStr for ConnectId {
    type Err = ConnectIdError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl Serialize for ConnectId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for ConnectId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(de::Error::custom)
    }
}

/// State of an existing binding. Ensure must not implicitly reactivate revocation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RegistrationStatus {
    Active,
    Retiring,
    Retired,
    Revoked,
}

/// Native-only registration input. Caller/owner/deployment authority is absent.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct RegistrationEnsureRequest {
    pub request_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workstation_ip: Option<IpAddr>,
    pub installation_marker: String,
}

impl RegistrationEnsureRequest {
    /// Build a workstation ensure input without any folder identity.
    #[must_use]
    pub fn new(
        request_id: String,
        workstation_ip: Option<IpAddr>,
        installation_marker: String,
    ) -> Self {
        Self {
            request_id,
            workstation_ip,
            installation_marker,
        }
    }
}

/// Non-secret receipt returned by authoritative workstation registration ensure/reuse.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct RegistrationReceipt {
    pub deployment_id: String,
    pub workstation_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workstation_ip: Option<IpAddr>,
    pub registration_id: String,
    pub connect_id: ConnectId,
    pub status: RegistrationStatus,
    pub revision: u64,
}

impl RegistrationReceipt {
    /// Construct a current receipt from server-resolved opaque identities.
    #[must_use]
    pub fn new(
        deployment_id: String,
        workstation_id: String,
        registration_id: String,
        workstation_ip: Option<IpAddr>,
        revision: u64,
    ) -> Self {
        let connect_id = ConnectId(workstation_id.clone());
        Self {
            deployment_id,
            workstation_id,
            workstation_ip,
            registration_id,
            connect_id,
            status: RegistrationStatus::Active,
            revision,
        }
    }

    /// Snapshot consistency is not proof of authority; every boundary revalidates it.
    #[must_use]
    pub fn is_consistent(&self) -> bool {
        ConnectId::new(&self.workstation_id).is_ok()
            && self.connect_id.as_str() == self.workstation_id
            && !self.deployment_id.is_empty()
            && !self.registration_id.is_empty()
            && self.revision > 0
    }
}

/// Browser resolution selects an existing registration; it never registers one.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct WebSessionCreateRequest {
    pub connect_id: ConnectId,
}

impl WebSessionCreateRequest {
    /// Select a previously registered Connect ID within configured web authority.
    #[must_use]
    pub fn new(connect_id: ConnectId) -> Self {
        Self { connect_id }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workstation_ids_reject_legacy_and_ambiguous_inputs() {
        let fixtures: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/connect_ids.json"))
                .expect("corpus");
        for fixture in fixtures["valid"].as_array().expect("valid corpus") {
            let input = fixture["input"].as_str().expect("input");
            let id: ConnectId = input.parse().expect("valid workstation ID");
            assert_eq!(
                id.as_str(),
                fixture["canonical"].as_str().expect("canonical")
            );
            assert_eq!(
                serde_json::from_str::<ConnectId>(&serde_json::to_string(&id).expect("serialize"))
                    .expect("deserialize"),
                id
            );
        }
        for input in fixtures["invalid"].as_array().expect("invalid corpus") {
            assert!(input.as_str().expect("input").parse::<ConnectId>().is_err());
        }
    }

    #[test]
    fn workspace_identity_is_not_a_registration_input() {
        let valid =
            serde_json::json!({"request_id":"request-a", "installation_marker":"installation-a"});
        assert!(serde_json::from_value::<RegistrationEnsureRequest>(valid.clone()).is_ok());
        for field in [
            "workspace_name",
            "workspace_marker",
            "workspace_id",
            "owner",
            "workstation_id",
        ] {
            let mut forged = valid.clone();
            forged[field] = serde_json::json!("forged");
            assert!(
                serde_json::from_value::<RegistrationEnsureRequest>(forged).is_err(),
                "{field}"
            );
        }
    }
    #[test]
    fn name_bounds_are_utf8_based_and_default_creation_is_protected() {
        assert!(WorkspaceName::new(&"a".repeat(MAX_WORKSPACE_NAME_BYTES)).is_ok());
        assert_eq!(
            WorkspaceName::new(&"a".repeat(MAX_WORKSPACE_NAME_BYTES + 1)),
            Err(WorkspaceNameError::TooLong)
        );
        assert!(WorkspaceName::new(&"é".repeat(64)).is_ok());
        assert_eq!(
            WorkspaceName::new(&"é".repeat(65)),
            Err(WorkspaceNameError::TooLong)
        );
        for value in ["default", "DEFAULT", "Default"] {
            assert_eq!(
                WorkspaceName::new_child(value),
                Err(WorkspaceNameError::ProtectedDefault)
            );
        }
        assert!(WorkspaceName::new("default")
            .expect("reserved default can register")
            .is_default());
        assert_eq!(
            WorkspaceName::new_child("Café research")
                .expect("ordinary child")
                .as_str(),
            "Café research"
        );
    }

    #[test]
    fn browser_session_input_cannot_supply_registration_or_gateway_authority() {
        let id = "node-a";
        assert!(serde_json::from_value::<WebSessionCreateRequest>(
            serde_json::json!({"connect_id":id})
        )
        .is_ok());
        for field in [
            "gateway",
            "owner",
            "workstation_id",
            "workspace_id",
            "bearer",
        ] {
            let mut value = serde_json::json!({"connect_id":id});
            value[field] = serde_json::json!("forged");
            assert!(
                serde_json::from_value::<WebSessionCreateRequest>(value).is_err(),
                "field {field}"
            );
        }
    }
}
