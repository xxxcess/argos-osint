//! Legacy child-registration snapshots retained for local recovery only.
//! New workstation admissions use fleetworks_contracts::connect instead.
pub use fleetworks_contracts::connect::{WorkspaceName, WorkspaceNameError};
use serde::{de, Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::net::{IpAddr, Ipv6Addr};
use std::str::FromStr;
const MAX_CONNECT_ID_BYTES: usize = 176;
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
