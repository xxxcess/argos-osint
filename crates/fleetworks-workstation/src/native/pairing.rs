//! Non-secret origin/installation-bound pairing metadata. All methods are
//! synchronous and belong on the caller's blocking worker with Keychain work.
//!
//! Start intent is durable before any HTTP POST. Issuance intent is durable
//! before Keychain writes. A crash between secret storage and metadata promotion
//! is recovered from the exact recorded reference, never a credential scan.

use std::collections::BTreeSet;
use std::fmt;

use fleetworks_contracts::Client;
use rustix::fs::AtFlags;
use serde::Deserialize;
use serde::Serialize;

use super::fs;
use super::fs::ObjectIdentity;
use super::keychain::KeychainError;
use super::keychain::KeychainItem;
use super::keychain::KeychainStore;
use super::keychain::SecretToken;
use super::NativeContainer;

/// Private metadata file; contains references and recovery IDs, never secrets.
pub const PAIRING_FILE: &str = ".native-pairing.json";
const MAX_ENTRIES: usize = 32;
const MAX_BYTES: u64 = 256 * 1024;

/// Current non-secret reference to one credential issued by an exact gateway.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct PairedGateway {
    pub origin: String,
    pub credential_id: String,
    pub deployment_id: String,
}

impl PairedGateway {
    /// Non-secret reference after approval and an authenticated gateway handshake.
    pub fn new(
        origin: &str,
        credential_id: String,
        deployment_id: String,
    ) -> Result<Self, PairingError> {
        let pair = Self {
            origin: canonical_origin(origin)?,
            credential_id,
            deployment_id,
        };
        validate_pair(&pair)?;
        Ok(pair)
    }
}

/// Exact pending start/poll identity; the polling secret exists only in Keychain.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct PendingPairing {
    pub origin: String,
    pub request_id: String,
    /// The server uses the exact client request identity for the challenge.
    pub pairing_id: String,
    /// Informational server hint, not proof of approval or current authority.
    pub expires_at_ms: Option<u64>,
    /// Durable promotion intent permits crash recovery after a Keychain write.
    pub issued_credential: Option<PairedGateway>,
}

impl PendingPairing {
    /// Prepare metadata before the start POST. No service or secret is read here.
    pub fn new(origin: &str, request_id: String) -> Result<Self, PairingError> {
        let origin = canonical_origin(origin)?;
        validate_request_id(&request_id)?;
        Ok(Self {
            origin,
            pairing_id: request_id.clone(),
            request_id,
            expires_at_ms: None,
            issued_credential: None,
        })
    }
}

/// Snapshot revision fences every mutation, including cross-process callers.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PairingCatalogSnapshot {
    pub revision: u64,
    pub selected_origin: Option<String>,
    pub paired_gateways: Vec<PairedGateway>,
    pub pending_pairings: Vec<PendingPairing>,
}

/// Public errors are deliberately redacted, including filesystem failures.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum PairingError {
    Keychain(KeychainError),
    CatalogUnavailable,
    InvalidInput,
    RevisionConflict,
}

impl fmt::Display for PairingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Keychain(error) => fmt::Display::fmt(error, formatter),
            Self::CatalogUnavailable => {
                formatter.write_str("Protected pairing catalog is unavailable or needs repair")
            }
            Self::InvalidInput => {
                formatter.write_str("Pairing reference or gateway origin is invalid")
            }
            Self::RevisionConflict => {
                formatter.write_str("Pairing state changed; reload its exact current reference")
            }
        }
    }
}

impl std::error::Error for PairingError {}

impl From<KeychainError> for PairingError {
    fn from(error: KeychainError) -> Self {
        Self::Keychain(error)
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Catalog {
    version: u16,
    installation_marker: String,
    root_object: ObjectIdentity,
    snapshot: PairingCatalogSnapshot,
}

impl NativeContainer {
    /// Read metadata without creating a pairing file or consulting Keychain.
    pub fn pairing_catalog(&self) -> Result<PairingCatalogSnapshot, PairingError> {
        let _guard = self.locked().map_err(catalog_error)?;
        Ok(self.read_pairing()?.0.snapshot)
    }

    /// Exact-origin lookup, with no fallback to another gateway or token file.
    pub fn paired_gateway(&self, origin: &str) -> Result<Option<PairedGateway>, PairingError> {
        let origin = canonical_origin(origin)?;
        Ok(self
            .pairing_catalog()?
            .paired_gateways
            .into_iter()
            .find(|pair| pair.origin == origin))
    }

    /// Persist explicit reconnect intent; only an existing paired origin is valid.
    pub fn select_paired_gateway(
        &self,
        expected_revision: u64,
        origin: &str,
    ) -> Result<PairingCatalogSnapshot, PairingError> {
        let origin = canonical_origin(origin)?;
        let _guard = self.locked().map_err(catalog_error)?;
        let (mut catalog, object) = self.read_pairing()?;
        check_revision(&catalog, expected_revision)?;
        if !catalog
            .snapshot
            .paired_gateways
            .iter()
            .any(|pair| pair.origin == origin)
        {
            return Err(PairingError::InvalidInput);
        }
        catalog.snapshot.selected_origin = Some(origin);
        self.persist_pairing(&mut catalog, object)?;
        Ok(catalog.snapshot)
    }

    /// Read only the active reference's secret. Missing/denied never falls back.
    pub fn paired_credential(
        &self,
        origin: &str,
        store: &dyn KeychainStore,
    ) -> Result<SecretToken, PairingError> {
        let origin = canonical_origin(origin)?;
        let _guard = self.locked().map_err(catalog_error)?;
        let (catalog, _) = self.read_pairing()?;
        let pair = catalog
            .snapshot
            .paired_gateways
            .iter()
            .find(|pair| pair.origin == origin)
            .ok_or(KeychainError::Missing)?;
        Ok(store.get(&self.credential_item(pair))?)
    }

    /// Persist an exact start identity before storing its generated polling
    /// secret. On failure reload metadata; repeat with the same ID and secret.
    /// Never send start until this method succeeds.
    pub fn begin_pairing(
        &self,
        expected_revision: u64,
        pending: PendingPairing,
        polling_secret: &SecretToken,
        store: &dyn KeychainStore,
    ) -> Result<PairingCatalogSnapshot, PairingError> {
        self.validate_pending(&pending)?;
        if pending.issued_credential.is_some()
            || !fleetworks_contracts::pairing::valid_secret(polling_secret.expose_secret())
        {
            return Err(PairingError::InvalidInput);
        }
        let _guard = self.locked().map_err(catalog_error)?;
        let (mut catalog, object) = self.read_pairing()?;
        check_revision(&catalog, expected_revision)?;
        if let Some(existing) = catalog
            .snapshot
            .pending_pairings
            .iter()
            .find(|row| row.origin == pending.origin)
        {
            if existing != &pending {
                return Err(PairingError::RevisionConflict);
            }
        } else {
            catalog.snapshot.pending_pairings.push(pending.clone());
            self.persist_pairing(&mut catalog, object)?;
        }
        self.store_exact_secret(store, &self.polling_item(&pending), polling_secret)?;
        // Revalidate after platform work; no successful result if root changed.
        self.revalidate_root().map_err(catalog_error)?;
        Ok(catalog.snapshot)
    }

    /// Persist server challenge hints after the start response. Pairing identity
    /// cannot change once recorded; the approval code is deliberately transient.
    pub fn record_pairing_challenge(
        &self,
        expected_revision: u64,
        origin: &str,
        request_id: &str,
        pairing_id: String,
        expires_at_ms: u64,
    ) -> Result<PairingCatalogSnapshot, PairingError> {
        let origin = canonical_origin(origin)?;
        validate_id(&pairing_id)?;
        let _guard = self.locked().map_err(catalog_error)?;
        let (mut catalog, object) = self.read_pairing()?;
        check_revision(&catalog, expected_revision)?;
        let pending = pending_mut(&mut catalog, &origin, request_id)?;
        if pending.pairing_id != pairing_id || pending.issued_credential.is_some() {
            return Err(PairingError::RevisionConflict);
        }
        pending.expires_at_ms = Some(expires_at_ms);
        self.persist_pairing(&mut catalog, object)?;
        Ok(catalog.snapshot)
    }

    /// Read the exact start's polling secret for start replay or a finite poll.
    pub fn pending_pairing_secret(
        &self,
        origin: &str,
        store: &dyn KeychainStore,
    ) -> Result<SecretToken, PairingError> {
        let origin = canonical_origin(origin)?;
        let _guard = self.locked().map_err(catalog_error)?;
        let (catalog, _) = self.read_pairing()?;
        let pending = catalog
            .snapshot
            .pending_pairings
            .iter()
            .find(|row| row.origin == origin)
            .ok_or(KeychainError::Missing)?;
        Ok(store.get(&self.polling_item(pending))?)
    }

    /// Save the approved token after the caller's authenticated handshake.
    /// Persisted issuance intent allows recovery if any later step fails.
    /// Old paired credentials and all transitional file bindings are preserved.
    pub fn complete_pairing(
        &self,
        expected_revision: u64,
        request_id: &str,
        pair: PairedGateway,
        token: &SecretToken,
        store: &dyn KeychainStore,
    ) -> Result<PairingCatalogSnapshot, PairingError> {
        validate_pair(&pair)?;
        let _guard = self.locked().map_err(catalog_error)?;
        let (mut catalog, mut object) = self.read_pairing()?;
        check_revision(&catalog, expected_revision)?;
        let pending = pending_mut(&mut catalog, &pair.origin, request_id)?;
        if let Some(issued) = &pending.issued_credential {
            if issued != &pair {
                return Err(PairingError::RevisionConflict);
            }
        } else {
            pending.issued_credential = Some(pair.clone());
            object = Some(self.persist_pairing(&mut catalog, object)?);
        }
        self.store_exact_secret(store, &self.credential_item(&pair), token)?;
        self.finish_pairing(catalog, object, &pair.origin, store)
    }

    /// Recover only an exact recorded issuance whose secret already exists.
    /// Missing secrets leave pending metadata unchanged for exact start/poll
    /// replay; denied access is an error and never interpreted as an empty pair.
    pub fn recover_pairing(
        &self,
        origin: &str,
        store: &dyn KeychainStore,
    ) -> Result<Option<PairedGateway>, PairingError> {
        let origin = canonical_origin(origin)?;
        let _guard = self.locked().map_err(catalog_error)?;
        let (catalog, object) = self.read_pairing()?;
        let Some(pair) = catalog
            .snapshot
            .pending_pairings
            .iter()
            .find(|pending| pending.origin == origin)
            .and_then(|pending| pending.issued_credential.clone())
        else {
            return Ok(None);
        };
        match store.get(&self.credential_item(&pair)) {
            Ok(_) => (),
            Err(KeychainError::Missing) => return Ok(None),
            Err(error) => return Err(error.into()),
        }
        self.finish_pairing(catalog, object, &origin, store)?;
        Ok(Some(pair))
    }

    /// Explicit local cancellation removes only this pending start's secrets.
    /// An already active credential is preserved; server cancellation is separate.
    pub fn clear_pending_pairing(
        &self,
        expected_revision: u64,
        origin: &str,
        request_id: &str,
        store: &dyn KeychainStore,
    ) -> Result<PairingCatalogSnapshot, PairingError> {
        let origin = canonical_origin(origin)?;
        let _guard = self.locked().map_err(catalog_error)?;
        let (mut catalog, object) = self.read_pairing()?;
        check_revision(&catalog, expected_revision)?;
        let pending = pending_mut(&mut catalog, &origin, request_id)?.clone();
        delete_if_present(store, &self.polling_item(&pending))?;
        if let Some(pair) = &pending.issued_credential {
            if !catalog.snapshot.paired_gateways.contains(pair) {
                delete_if_present(store, &self.credential_item(pair))?;
            }
        }
        catalog
            .snapshot
            .pending_pairings
            .retain(|row| row.origin != origin);
        self.persist_pairing(&mut catalog, object)?;
        Ok(catalog.snapshot)
    }

    /// Explicitly forget one current paired credential. This never revokes a
    /// backend token, deletes a transitional token file, or edits legacy journals.
    pub fn remove_paired_gateway(
        &self,
        expected_revision: u64,
        origin: &str,
        store: &dyn KeychainStore,
    ) -> Result<PairingCatalogSnapshot, PairingError> {
        let origin = canonical_origin(origin)?;
        let _guard = self.locked().map_err(catalog_error)?;
        let (mut catalog, object) = self.read_pairing()?;
        check_revision(&catalog, expected_revision)?;
        let pair = catalog
            .snapshot
            .paired_gateways
            .iter()
            .find(|pair| pair.origin == origin)
            .ok_or(KeychainError::Missing)?;
        if catalog
            .snapshot
            .pending_pairings
            .iter()
            .any(|pending| pending.origin == origin)
        {
            return Err(PairingError::RevisionConflict);
        }
        delete_if_present(store, &self.credential_item(pair))?;
        catalog
            .snapshot
            .paired_gateways
            .retain(|pair| pair.origin != origin);
        if catalog.snapshot.selected_origin.as_ref() == Some(&origin) {
            catalog.snapshot.selected_origin = None;
        }
        self.persist_pairing(&mut catalog, object)?;
        Ok(catalog.snapshot)
    }

    fn credential_item(&self, pair: &PairedGateway) -> KeychainItem {
        KeychainItem::new(
            &self.installation_marker,
            &pair.origin,
            "credential",
            &pair.credential_id,
        )
    }

    fn polling_item(&self, pending: &PendingPairing) -> KeychainItem {
        KeychainItem::new(
            &self.installation_marker,
            &pending.origin,
            "poll",
            &pending.request_id,
        )
    }

    fn store_exact_secret(
        &self,
        store: &dyn KeychainStore,
        item: &KeychainItem,
        token: &SecretToken,
    ) -> Result<(), PairingError> {
        match store.get(item) {
            Ok(existing) if existing == *token => return Ok(()),
            Ok(_) => return Err(PairingError::RevisionConflict),
            Err(KeychainError::Missing) => store.set(item, token)?,
            Err(error) => return Err(error.into()),
        }
        if store.get(item)? != *token {
            return Err(PairingError::Keychain(KeychainError::Unavailable));
        }
        Ok(())
    }

    fn finish_pairing(
        &self,
        mut catalog: Catalog,
        mut object: Option<ObjectIdentity>,
        origin: &str,
        store: &dyn KeychainStore,
    ) -> Result<PairingCatalogSnapshot, PairingError> {
        let pending = catalog
            .snapshot
            .pending_pairings
            .iter()
            .find(|pending| pending.origin == origin)
            .ok_or(PairingError::RevisionConflict)?
            .clone();
        let pair = pending
            .issued_credential
            .as_ref()
            .ok_or(PairingError::RevisionConflict)?;
        if !catalog.snapshot.paired_gateways.contains(pair)
            || catalog.snapshot.selected_origin.as_deref() != Some(origin)
        {
            catalog
                .snapshot
                .paired_gateways
                .retain(|existing| existing.origin != origin);
            catalog.snapshot.paired_gateways.push(pair.clone());
            catalog.snapshot.selected_origin = Some(origin.to_owned());
            object = Some(self.persist_pairing(&mut catalog, object)?);
        }
        // Keep pending metadata until cleanup succeeds, so denied cleanup or a
        // crash is retryable instead of orphaning a polling-secret reference.
        delete_if_present(store, &self.polling_item(&pending))?;
        catalog
            .snapshot
            .pending_pairings
            .retain(|pending| pending.origin != origin);
        self.persist_pairing(&mut catalog, object)?;
        Ok(catalog.snapshot)
    }

    fn read_pairing(&self) -> Result<(Catalog, Option<ObjectIdentity>), PairingError> {
        self.revalidate_root().map_err(catalog_error)?;
        match rustix::fs::statat(&self.root_fd, PAIRING_FILE, AtFlags::SYMLINK_NOFOLLOW) {
            Err(rustix::io::Errno::NOENT) => Ok((
                Catalog {
                    version: 1,
                    installation_marker: self.installation_marker.clone(),
                    root_object: self.root_object,
                    snapshot: PairingCatalogSnapshot::default(),
                },
                None,
            )),
            Ok(_) => {
                let (catalog, object) =
                    fs::read_json_limited::<Catalog>(&self.root_fd, PAIRING_FILE, MAX_BYTES)
                        .map_err(catalog_error)?;
                self.validate_pairing(&catalog)?;
                Ok((catalog, Some(object)))
            }
            Err(_) => Err(PairingError::CatalogUnavailable),
        }
    }

    fn persist_pairing(
        &self,
        catalog: &mut Catalog,
        object: Option<ObjectIdentity>,
    ) -> Result<ObjectIdentity, PairingError> {
        self.revalidate_root().map_err(catalog_error)?;
        catalog.snapshot.revision = catalog
            .snapshot
            .revision
            .checked_add(1)
            .ok_or(PairingError::CatalogUnavailable)?;
        self.validate_pairing(catalog)?;
        let bytes = serde_json::to_vec(catalog).map_err(catalog_error)?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err(PairingError::CatalogUnavailable);
        }
        let object =
            fs::write_json(&self.root_fd, PAIRING_FILE, catalog, object).map_err(catalog_error)?;
        self.revalidate_root().map_err(catalog_error)?;
        Ok(object)
    }

    fn validate_pairing(&self, catalog: &Catalog) -> Result<(), PairingError> {
        if catalog.version != 1
            || catalog.installation_marker != self.installation_marker
            || catalog.root_object != self.root_object
            || catalog.snapshot.paired_gateways.len() > MAX_ENTRIES
            || catalog.snapshot.pending_pairings.len() > MAX_ENTRIES
        {
            return Err(PairingError::CatalogUnavailable);
        }
        let mut origins = BTreeSet::new();
        for pair in &catalog.snapshot.paired_gateways {
            validate_pair(pair).map_err(|_| PairingError::CatalogUnavailable)?;
            if !origins.insert(&pair.origin) {
                return Err(PairingError::CatalogUnavailable);
            }
        }
        if catalog
            .snapshot
            .selected_origin
            .as_ref()
            .is_some_and(|origin| !origins.contains(origin))
        {
            return Err(PairingError::CatalogUnavailable);
        }
        origins.clear();
        for pending in &catalog.snapshot.pending_pairings {
            self.validate_pending(pending)
                .map_err(|_| PairingError::CatalogUnavailable)?;
            if !origins.insert(&pending.origin) {
                return Err(PairingError::CatalogUnavailable);
            }
        }
        Ok(())
    }

    fn validate_pending(&self, pending: &PendingPairing) -> Result<(), PairingError> {
        if canonical_origin(&pending.origin)? != pending.origin {
            return Err(PairingError::InvalidInput);
        }
        validate_request_id(&pending.request_id)?;
        if pending.pairing_id != pending.request_id {
            return Err(PairingError::InvalidInput);
        }
        if let Some(pair) = &pending.issued_credential {
            validate_pair(pair)?;
            if pair.origin != pending.origin {
                return Err(PairingError::InvalidInput);
            }
        }
        Ok(())
    }
}

fn canonical_origin(origin: &str) -> Result<String, PairingError> {
    if origin.len() > 2048 {
        return Err(PairingError::InvalidInput);
    }
    Client::canonical_origin(origin).map_err(|_| PairingError::InvalidInput)
}

fn validate_id(id: &str) -> Result<(), PairingError> {
    if id.is_empty()
        || id.len() > 256
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err(PairingError::InvalidInput);
    }
    Ok(())
}

fn validate_request_id(id: &str) -> Result<(), PairingError> {
    if !fleetworks_contracts::pairing::valid_identity(id) {
        return Err(PairingError::InvalidInput);
    }
    Ok(())
}

fn validate_pair(pair: &PairedGateway) -> Result<(), PairingError> {
    if canonical_origin(&pair.origin)? != pair.origin {
        return Err(PairingError::InvalidInput);
    }
    validate_id(&pair.credential_id)?;
    validate_id(&pair.deployment_id)
}

fn check_revision(catalog: &Catalog, expected: u64) -> Result<(), PairingError> {
    if catalog.snapshot.revision != expected {
        return Err(PairingError::RevisionConflict);
    }
    Ok(())
}

fn pending_mut<'a>(
    catalog: &'a mut Catalog,
    origin: &str,
    request_id: &str,
) -> Result<&'a mut PendingPairing, PairingError> {
    catalog
        .snapshot
        .pending_pairings
        .iter_mut()
        .find(|pending| pending.origin == origin && pending.request_id == request_id)
        .ok_or(PairingError::RevisionConflict)
}

fn delete_if_present(store: &dyn KeychainStore, item: &KeychainItem) -> Result<(), PairingError> {
    match store.delete(item) {
        Ok(()) | Err(KeychainError::Missing) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn catalog_error(_: impl fmt::Display) -> PairingError {
    PairingError::CatalogUnavailable
}

#[cfg(test)]
mod tests;
