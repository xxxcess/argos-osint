//! Canonical Connect IDs and the registered-workstation wire contract.
//!
//! An IP address, folder marker or Connect ID is context, never a credential.
//! The gateway derives ownership from its authenticated caller and enforces
//! uniqueness, permissions, protection and revocation in its authoritative store.

use std::fmt;
use std::net::IpAddr;
use std::net::Ipv6Addr;
use std::str::FromStr;

use serde::de;
use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use serde::Serializer;
use unicode_normalization::UnicodeNormalization;

/// Current registry/Connect contract version.
pub const CONNECT_CONTRACT_VERSION: u16 = 1;
/// Native-only authenticated registration creation/reconciliation route.
pub const REGISTRATION_ENSURE_PATH: &str = "/api/v1/connect/registrations/ensure";
/// Purpose-authorized browser session creation route.
pub const WEB_SESSION_CREATE_PATH: &str = "/api/v1/connect/sessions";
/// Maximum UTF-8 bytes accepted for a workspace basename, before and after NFC.
pub const MAX_WORKSPACE_NAME_BYTES: usize = 128;
/// Maximum bytes in an input Connect ID, including IP, brackets and separator.
pub const MAX_CONNECT_ID_BYTES: usize = 176;
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

/// Why a Connect ID cannot identify a native workstation child.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ConnectIdError {
    TooLong,
    InvalidFormat,
    MissingIpv6Brackets,
    InvalidIp,
    InvalidWorkstationIp,
    Workspace(WorkspaceNameError),
}

impl fmt::Display for ConnectIdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLong => formatter.write_str("Connect ID exceeds the byte limit"),
            Self::InvalidFormat => formatter.write_str("expected an IP address and child basename"),
            Self::MissingIpv6Brackets => formatter.write_str("IPv6 Connect IDs require brackets"),
            Self::InvalidIp => formatter.write_str("Connect ID contains an invalid IP address"),
            Self::InvalidWorkstationIp => {
                formatter.write_str("select a non-loopback unicast workstation address")
            }
            Self::Workspace(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ConnectIdError {}

/// A canonical native workstation IP plus a validated child basename.
///
/// IPv4-mapped IPv6 is consistently serialized as IPv4. Ordinary IPv6 uses
/// brackets and the standard compressed lowercase representation. Zone IDs,
/// loopback, unspecified, multicast and IPv4 broadcast are not node identities.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ConnectId {
    ip: IpAddr,
    workspace: WorkspaceName,
}

impl ConnectId {
    /// Build a canonical identifier from a native interface and child basename.
    pub fn new(ip: IpAddr, workspace: WorkspaceName) -> Result<Self, ConnectIdError> {
        let ip = match ip {
            IpAddr::V6(ip) => ip
                .to_ipv4_mapped()
                .map(IpAddr::V4)
                .unwrap_or(IpAddr::V6(ip)),
            IpAddr::V4(ip) => IpAddr::V4(ip),
        };
        if ip.is_unspecified()
            || ip.is_loopback()
            || ip.is_multicast()
            || matches!(ip, IpAddr::V4(value) if value.is_broadcast())
        {
            return Err(ConnectIdError::InvalidWorkstationIp);
        }
        Ok(Self { ip, workspace })
    }

    /// Canonical selected native interface IP; not a peer-IP attestation.
    #[must_use]
    pub fn ip(&self) -> IpAddr {
        self.ip
    }

    /// Registered child basename; never a full local path.
    #[must_use]
    pub fn workspace_name(&self) -> &WorkspaceName {
        &self.workspace
    }
}

impl fmt::Display for ConnectId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.ip {
            IpAddr::V4(ip) => write!(formatter, "{ip}:{}", self.workspace),
            IpAddr::V6(ip) => write!(formatter, "[{ip}]:{}", self.workspace),
        }
    }
}

impl FromStr for ConnectId {
    type Err = ConnectIdError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.len() > MAX_CONNECT_ID_BYTES {
            return Err(ConnectIdError::TooLong);
        }
        let (ip, name) = if let Some(bracketed) = value.strip_prefix('[') {
            let (address, name) = bracketed
                .split_once("]:")
                .ok_or(ConnectIdError::InvalidFormat)?;
            let ip = address
                .parse::<Ipv6Addr>()
                .map_err(|_| ConnectIdError::InvalidIp)?;
            (IpAddr::V6(ip), name)
        } else {
            if value.bytes().filter(|byte| *byte == b':').count() != 1 {
                return Err(ConnectIdError::MissingIpv6Brackets);
            }
            let (address, name) = value.split_once(':').ok_or(ConnectIdError::InvalidFormat)?;
            let ip = address
                .parse::<IpAddr>()
                .map_err(|_| ConnectIdError::InvalidIp)?;
            (ip, name)
        };
        let workspace = WorkspaceName::new(name).map_err(ConnectIdError::Workspace)?;
        Self::new(ip, workspace)
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
    pub workstation_ip: IpAddr,
    pub workspace_name: WorkspaceName,
    pub installation_marker: String,
    pub workspace_marker: String,
}

impl RegistrationEnsureRequest {
    /// Build an ensure input. The server revalidates markers, IDs and authority.
    #[must_use]
    pub fn new(
        request_id: String,
        workstation_ip: IpAddr,
        workspace_name: WorkspaceName,
        installation_marker: String,
        workspace_marker: String,
    ) -> Self {
        Self {
            request_id,
            workstation_ip,
            workspace_name,
            installation_marker,
            workspace_marker,
        }
    }
}

/// Non-secret receipt returned by authoritative registration ensure/reuse.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct RegistrationReceipt {
    pub deployment_id: String,
    pub workstation_id: String,
    pub workstation_ip: IpAddr,
    pub workspace_id: String,
    pub workspace_name: WorkspaceName,
    pub registration_id: String,
    pub connect_id: ConnectId,
    pub status: RegistrationStatus,
    pub revision: u64,
}

impl RegistrationReceipt {
    /// Construct an active receipt from identities resolved by the server.
    #[must_use]
    pub fn new(
        deployment_id: String,
        workstation_id: String,
        workspace_id: String,
        registration_id: String,
        connect_id: ConnectId,
        revision: u64,
    ) -> Self {
        Self {
            deployment_id,
            workstation_id,
            workstation_ip: connect_id.ip(),
            workspace_id,
            workspace_name: connect_id.workspace_name().clone(),
            registration_id,
            connect_id,
            status: RegistrationStatus::Active,
            revision,
        }
    }

    /// Detect inconsistent display snapshots in a decoded receipt.
    ///
    /// Consistency is not proof of authority; the gateway must still verify the
    /// caller and current record before every protected operation.
    #[must_use]
    pub fn is_consistent(&self) -> bool {
        self.workstation_ip == self.connect_id.ip()
            && self.workspace_name == *self.connect_id.workspace_name()
            && !self.deployment_id.is_empty()
            && !self.workstation_id.is_empty()
            && !self.workspace_id.is_empty()
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

    #[derive(Deserialize)]
    struct Fixtures {
        valid: Vec<ValidFixture>,
        invalid: Vec<String>,
    }

    #[derive(Deserialize)]
    struct ValidFixture {
        input: String,
        canonical: String,
        ip: String,
        workspace: String,
    }

    #[test]
    fn shared_connect_id_corpus_is_canonical_and_round_trips() {
        let fixtures: Fixtures =
            serde_json::from_str(include_str!("../tests/fixtures/connect_ids.json"))
                .expect("shared Connect ID fixture must decode");
        for fixture in fixtures.valid {
            let id: ConnectId = fixture.input.parse().expect("valid shared Connect ID");
            assert_eq!(
                id.to_string(),
                fixture.canonical,
                "input {:?}",
                fixture.input
            );
            assert_eq!(id.ip().to_string(), fixture.ip);
            assert_eq!(id.workspace_name().as_str(), fixture.workspace);
            let json = serde_json::to_string(&id).expect("serialize ID");
            assert_eq!(
                serde_json::from_str::<ConnectId>(&json).expect("decode ID"),
                id
            );
            assert_eq!(
                id.to_string().parse::<ConnectId>().expect("canonical ID"),
                id
            );
        }
        for value in fixtures.invalid {
            assert!(
                value.parse::<ConnectId>().is_err(),
                "invalid shared input {value:?}"
            );
            assert!(serde_json::from_value::<ConnectId>(serde_json::json!(value)).is_err());
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
        let id = "192.0.2.42:research";
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
