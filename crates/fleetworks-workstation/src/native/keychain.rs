//! Blocking secret storage. Call on an owned blocking worker, never a UI or
//! async runtime callback. Tests inject a fake store and never use the login Keychain.

use std::fmt;

/// Secret bearer or polling token. Debug is redacted; serialization is deliberately absent.
#[derive(Clone, Eq, PartialEq)]
pub struct SecretToken(String);

impl SecretToken {
    /// Accept bounded, printable ASCII token bytes without whitespace.
    pub fn new(value: String) -> Result<Self, KeychainError> {
        if value.is_empty()
            || value.len() > 4096
            || !value.bytes().all(|byte| byte.is_ascii_graphic())
        {
            return Err(KeychainError::Unavailable);
        }
        Ok(Self(value))
    }

    /// Expose only at the authenticated HTTP or injected secret-store boundary.
    #[must_use]
    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretToken(<redacted>)")
    }
}

/// Safe errors never include OS error prose, item values or token bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum KeychainError {
    Missing,
    LockedDenied,
    Unavailable,
}

impl fmt::Display for KeychainError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Missing => "Keychain credential is missing",
            Self::LockedDenied => "Keychain is locked or access was denied",
            Self::Unavailable => "Keychain credential storage is unavailable",
        })
    }
}

impl std::error::Error for KeychainError {}

/// Non-secret exact Keychain identity derived by the protected native catalog.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeychainItem {
    service: String,
    account: String,
}

impl KeychainItem {
    pub(super) fn new(installation: &str, origin: &str, kind: &str, id: &str) -> Self {
        // Length-prefixing makes components unambiguous even when an origin
        // contains colons. No item can fall back to another origin/installation.
        Self {
            service: "dev.fleetworks.native.pairing.v1".into(),
            account: format!(
                "{}:{installation}{}:{origin}{}:{kind}{}:{id}",
                installation.len(),
                origin.len(),
                kind.len(),
                id.len()
            ),
        }
    }

    #[must_use]
    pub fn service(&self) -> &str {
        &self.service
    }

    #[must_use]
    pub fn account(&self) -> &str {
        &self.account
    }
}

/// Synchronous boundary for platform storage and isolated test fakes.
/// Implementations must not log token values or include them in errors.
pub trait KeychainStore: Send + Sync {
    fn get(&self, item: &KeychainItem) -> Result<SecretToken, KeychainError>;
    fn set(&self, item: &KeychainItem, token: &SecretToken) -> Result<(), KeychainError>;
    fn delete(&self, item: &KeychainItem) -> Result<(), KeychainError>;
}

/// macOS login Keychain generic-password adapter; unsupported platforms fail closed.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemKeychain;

#[cfg(target_os = "macos")]
impl KeychainStore for SystemKeychain {
    fn get(&self, item: &KeychainItem) -> Result<SecretToken, KeychainError> {
        let options = security_framework::passwords::PasswordOptions::new_generic_password(
            item.service(),
            item.account(),
        );
        let bytes =
            security_framework::passwords::generic_password(options).map_err(map_platform_error)?;
        let value = String::from_utf8(bytes).map_err(|_| KeychainError::Unavailable)?;
        SecretToken::new(value)
    }

    fn set(&self, item: &KeychainItem, token: &SecretToken) -> Result<(), KeychainError> {
        security_framework::passwords::set_generic_password(
            item.service(),
            item.account(),
            token.expose_secret().as_bytes(),
        )
        .map_err(map_platform_error)
    }

    fn delete(&self, item: &KeychainItem) -> Result<(), KeychainError> {
        security_framework::passwords::delete_generic_password(item.service(), item.account())
            .map_err(map_platform_error)
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn platform_statuses_are_typed_without_accessing_any_keychain() {
        for (status, expected) in [
            (-25300, KeychainError::Missing),
            (-25308, KeychainError::LockedDenied),
            (-25293, KeychainError::LockedDenied),
            (-128, KeychainError::LockedDenied),
            (-25291, KeychainError::Unavailable),
        ] {
            assert_eq!(
                map_platform_error(security_framework::base::Error::from_code(status)),
                expected
            );
        }
    }
}

#[cfg(target_os = "macos")]
fn map_platform_error(error: security_framework::base::Error) -> KeychainError {
    match error.code() {
        -25300 => KeychainError::Missing, // errSecItemNotFound
        -25308 | -25293 | -128 | -25315 => KeychainError::LockedDenied,
        _ => KeychainError::Unavailable,
    }
}

#[cfg(not(target_os = "macos"))]
impl KeychainStore for SystemKeychain {
    fn get(&self, _: &KeychainItem) -> Result<SecretToken, KeychainError> {
        Err(KeychainError::Unavailable)
    }

    fn set(&self, _: &KeychainItem, _: &SecretToken) -> Result<(), KeychainError> {
        Err(KeychainError::Unavailable)
    }

    fn delete(&self, _: &KeychainItem) -> Result<(), KeychainError> {
        Err(KeychainError::Unavailable)
    }
}
