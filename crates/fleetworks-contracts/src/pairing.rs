//! Bounded native pairing. These messages never grant caller-selected authority.
use std::fmt;

use serde::Deserialize;
use serde::Serialize;

pub const PAIRING_START_PATH: &str = "/api/v1/pairing/start";
pub const PAIRING_POLL_PATH: &str = "/api/v1/pairing/poll";
pub const PAIRING_APPROVE_PATH: &str = "/api/v1/admin/pairing/approve";
pub const PAIRING_DENY_PATH: &str = "/api/v1/admin/pairing/deny";
pub const PAIRING_REVOKE_PATH: &str = "/api/v1/admin/pairing/revoke";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PairingContractError;
impl fmt::Display for PairingContractError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid pairing message")
    }
}
impl std::error::Error for PairingContractError {}

pub fn valid_identity(value: &str) -> bool {
    (16..=128).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}
pub fn valid_secret(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn check(valid: bool) -> Result<(), PairingContractError> {
    valid.then_some(()).ok_or(PairingContractError)
}

/// Persist this exact request before sending. Its request ID is the pairing ID.
/// Deliberately has no Debug implementation: the polling secret is a credential.
#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PairingStartRequest {
    pub request_id: String,
    pub installation_marker: String,
    pub polling_secret: String,
}
impl PairingStartRequest {
    pub fn validate(&self) -> Result<(), PairingContractError> {
        check(
            valid_identity(&self.request_id)
                && valid_identity(&self.installation_marker)
                && valid_secret(&self.polling_secret),
        )
    }
}
#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PairingPollRequest {
    pub pairing_id: String,
    pub polling_secret: String,
}
impl PairingPollRequest {
    pub fn validate(&self) -> Result<(), PairingContractError> {
        check(valid_identity(&self.pairing_id) && valid_secret(&self.polling_secret))
    }
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PairingChallenge {
    pub pairing_id: String,
    pub approval_code: String,
    pub expires_at_ms: u64,
    pub poll_after_ms: u64,
}
impl PairingChallenge {
    pub fn validate(&self) -> Result<(), PairingContractError> {
        check(
            valid_identity(&self.pairing_id)
                && valid_identity(&self.approval_code)
                && self.expires_at_ms > 0
                && (1000..=60_000).contains(&self.poll_after_ms),
        )
    }
}
/// An issued token is secret; neither this response nor its parent has Debug.
#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum PairingStatus {
    Pending {
        poll_after_ms: u64,
    },
    Denied,
    Expired,
    Approved {
        credential_id: String,
        access_token: String,
        scope_id: String,
        access_epoch: String,
    },
}
impl PairingStatus {
    pub fn validate(&self) -> Result<(), PairingContractError> {
        check(match self {
            Self::Pending { poll_after_ms } => (1000..=60_000).contains(poll_after_ms),
            Self::Denied | Self::Expired => true,
            Self::Approved {
                credential_id,
                access_token,
                scope_id,
                access_epoch,
            } => {
                credential_id
                    .strip_prefix("paired-")
                    .is_some_and(|suffix| !suffix.is_empty())
                    && bounded_authority(credential_id)
                    && valid_secret(access_token)
                    && bounded_authority(scope_id)
                    && bounded_authority(access_epoch)
            }
        })
    }
}
fn bounded_authority(value: &str) -> bool {
    (1..=128).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}
/// The operator supplies only the native-displayed challenge, never scope/grants.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PairingDecisionRequest {
    pub pairing_id: String,
    pub approval_code: String,
}
impl PairingDecisionRequest {
    pub fn validate(&self) -> Result<(), PairingContractError> {
        check(valid_identity(&self.pairing_id) && valid_identity(&self.approval_code))
    }
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PairingRevokeRequest {
    pub credential_id: String,
}
impl PairingRevokeRequest {
    pub fn validate(&self) -> Result<(), PairingContractError> {
        check(
            self.credential_id
                .strip_prefix("paired-")
                .is_some_and(|suffix| !suffix.is_empty())
                && bounded_authority(&self.credential_id),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn authority_and_secrets_are_not_client_selectable_or_weak() {
        assert!(serde_json::from_value::<PairingStartRequest>(serde_json::json!({"request_id":"a".repeat(32),"installation_marker":"b".repeat(32),"polling_secret":"c".repeat(64),"scope_id":"other"})).is_err());
        let mut request = PairingStartRequest {
            request_id: "a".repeat(32),
            installation_marker: "b".repeat(32),
            polling_secret: "c".repeat(64),
        };
        assert!(request.validate().is_ok());
        request.polling_secret = "c".repeat(32);
        assert!(request.validate().is_err());
        assert!(serde_json::from_value::<PairingDecisionRequest>(serde_json::json!({"pairing_id":"a".repeat(32),"approval_code":"b".repeat(32),"permissions":["manage_configuration"]})).is_err());
    }
}
