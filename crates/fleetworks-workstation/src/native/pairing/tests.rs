use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::Mutex;

use super::*;

const ORIGIN: &str = "https://gateway.example.test/";
const POLL: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const ACCESS: &str = "synthetic-issued-credential-never-in-files";

#[derive(Default)]
struct FakeState {
    items: BTreeMap<(String, String), SecretToken>,
    get_error: Option<KeychainError>,
    set_error: Option<KeychainError>,
    delete_error: Option<KeychainError>,
    fail_catalog_after_set: Option<PathBuf>,
    calls: usize,
}

#[derive(Default)]
struct FakeKeychain(Mutex<FakeState>);

impl KeychainStore for FakeKeychain {
    fn get(&self, item: &KeychainItem) -> Result<SecretToken, KeychainError> {
        let mut state = self.0.lock().expect("isolated fake store");
        state.calls += 1;
        if let Some(error) = state.get_error {
            return Err(error);
        }
        state
            .items
            .get(&(item.service().into(), item.account().into()))
            .cloned()
            .ok_or(KeychainError::Missing)
    }

    fn set(&self, item: &KeychainItem, token: &SecretToken) -> Result<(), KeychainError> {
        let mut state = self.0.lock().expect("isolated fake store");
        state.calls += 1;
        if let Some(error) = state.set_error {
            return Err(error);
        }
        state.items.insert(
            (item.service().into(), item.account().into()),
            token.clone(),
        );
        if let Some(path) = state.fail_catalog_after_set.take() {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o644))
                .expect("inject isolated metadata failure after secret write");
        }
        Ok(())
    }

    fn delete(&self, item: &KeychainItem) -> Result<(), KeychainError> {
        let mut state = self.0.lock().expect("isolated fake store");
        state.calls += 1;
        if let Some(error) = state.delete_error {
            return Err(error);
        }
        state
            .items
            .remove(&(item.service().into(), item.account().into()))
            .map(|_| ())
            .ok_or(KeychainError::Missing)
    }
}

fn home() -> (tempfile::TempDir, PathBuf) {
    let temporary = tempfile::tempdir().expect("isolated home");
    let path = temporary.path().canonicalize().expect("real home path");
    (temporary, path)
}

fn secret(value: &str) -> SecretToken {
    SecretToken::new(value.into()).expect("synthetic secret")
}

fn pending(origin: &str, id: &str) -> PendingPairing {
    PendingPairing::new(origin, id.into()).expect("exact synthetic request")
}

fn pair(origin: &str, id: &str) -> PairedGateway {
    PairedGateway::new(origin, id.into(), "synthetic-deployment".into())
        .expect("authenticated synthetic gateway")
}

fn approve(
    container: &NativeContainer,
    origin: &str,
    id: &str,
    store: &FakeKeychain,
) -> PairingCatalogSnapshot {
    let current = container.pairing_catalog().expect("current revision");
    let begun = container
        .begin_pairing(current.revision, pending(origin, id), &secret(POLL), store)
        .expect("durable start");
    container
        .complete_pairing(begun.revision, id, pair(origin, id), &secret(ACCESS), store)
        .expect("approved durable pair")
}

#[cfg(target_os = "macos")]
#[test]
fn apfs_device_renumbering_retains_saved_folders_and_pairing() {
    use crate::native::state;
    use crate::native::volume;
    use crate::native::CONNECTIONS_FILE;

    let (_temporary, home) = home();
    let container = NativeContainer::open(&home).expect("profile with a retained legacy child");
    let marker = container.installation_marker().to_owned();
    let default = container
        .ensure_local_backend_state_dir("embedded")
        .expect("default Embedded directory");
    let embedded_data = default.join("existing-data");
    std::fs::write(&embedded_data, b"retained").expect("synthetic Embedded data");
    let folder = home.join("saved-embedded-folder");
    let profiles = container
        .add_local_backend_folder(0, folder.clone())
        .expect("saved Embedded directory");
    let store = FakeKeychain::default();
    let pairing = approve(&container, ORIGIN, "synthetic-request-a", &store);

    // Simulate a v1 installation whose volume had a different device number at creation.
    let stale_device = container.root_object.device ^ 1;
    std::fs::remove_file(container.root().join(volume::FILE)).expect("remove new volume binding");
    for file in [state::CATALOG_FILE, CONNECTIONS_FILE, PAIRING_FILE] {
        let (mut catalog, object) = fs::read_json::<serde_json::Value>(&container.root_fd, file)
            .expect("existing v1 catalog");
        catalog["root_object"]["device"] = stale_device.into();
        if file == state::CATALOG_FILE {
            catalog["workspaces"][0]["object"]["device"] = stale_device.into();
        }
        fs::write_json(&container.root_fd, file, &catalog, Some(object))
            .expect("record the older mount device");
    }
    drop(container);

    let error = NativeContainer::open_settings(&home)
        .err()
        .expect("first binding of a renumbered device needs an explicit retry");
    assert!(
        error.to_string().contains("Retry folder access"),
        "{error:#}"
    );
    let repaired = NativeContainer::retry_settings_access(&home).expect("reopen owned APFS root");
    assert_eq!(repaired.installation_marker(), marker);
    assert_eq!(
        repaired.connection_profiles().expect("saved profiles"),
        profiles
    );
    assert_eq!(repaired.pairing_catalog().expect("saved pairing"), pairing);
    assert_eq!(
        repaired.paired_credential(ORIGIN, &store).unwrap(),
        secret(ACCESS)
    );
    assert_eq!(
        repaired.ensure_local_backend_state_dir("embedded").unwrap(),
        default
    );
    assert_eq!(
        repaired.validate_local_backend_state_dir(&folder).unwrap(),
        folder
    );
    assert_eq!(std::fs::read(&embedded_data).unwrap(), b"retained");
    assert!(repaired.default_workspace().is_err());
    drop(repaired);

    let reopened = NativeContainer::open_settings(&home).expect("reopen using the volume UUID");
    assert_eq!(reopened.installation_marker(), marker);
    assert_eq!(
        reopened.pairing_catalog().expect("reopened pairing"),
        pairing
    );
    assert!(
        NativeContainer::open(&home).is_err(),
        "legacy root checks still require an exact device"
    );
}

#[test]
fn nonsecret_catalog_reopens_selected_pair_without_changing_legacy_state_or_token_files() {
    let (_temporary, path) = home();
    let container = NativeContainer::open_settings(&path).expect("protected installation");
    let store = FakeKeychain::default();
    let legacy_file = container.root().join(super::super::state::CATALOG_FILE);
    let legacy_before = std::fs::read(&legacy_file).expect("existing identity catalog");
    let token_file = path.join("operator-token");
    std::fs::write(&token_file, "synthetic-transitional-file-token")
        .expect("untouched operator input");
    assert_eq!(
        container.pairing_catalog().expect("empty metadata"),
        PairingCatalogSnapshot::default()
    );
    assert!(
        !container.root().join(PAIRING_FILE).exists(),
        "reads do not create metadata"
    );
    let snapshot = approve(
        &container,
        "https://GATEWAY.example.test:443",
        "synthetic-request-a",
        &store,
    );
    assert_eq!(snapshot.selected_origin.as_deref(), Some(ORIGIN));
    assert!(snapshot.pending_pairings.is_empty());
    assert_eq!(
        container
            .paired_credential(ORIGIN, &store)
            .expect("exact secret"),
        secret(ACCESS)
    );
    let body = std::fs::read_to_string(container.root().join(PAIRING_FILE))
        .expect("nonsecret private metadata");
    for value in [POLL, ACCESS, "synthetic-transitional-file-token"] {
        assert!(!body.contains(value));
    }
    assert_eq!(
        std::fs::read(&legacy_file).expect("legacy identity"),
        legacy_before
    );
    assert_eq!(
        std::fs::read_to_string(&token_file).expect("operator token preserved"),
        "synthetic-transitional-file-token"
    );
    assert_eq!(
        std::fs::metadata(container.root().join(PAIRING_FILE))
            .expect("private metadata")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    drop(container);
    let reopened = NativeContainer::open_settings(&path).expect("restart");
    assert_eq!(reopened.pairing_catalog().expect("reopened pair"), snapshot);
    assert_eq!(
        reopened
            .paired_credential(ORIGIN, &store)
            .expect("reopened exact secret"),
        secret(ACCESS)
    );
    assert!(
        store
            .0
            .lock()
            .expect("fake state")
            .items
            .values()
            .all(|value| value != &secret(POLL)),
        "completed polling secret is explicitly cleaned"
    );
}

#[test]
fn start_failure_is_recoverable_but_stale_or_changed_requests_never_replace_a_secret() {
    let (_temporary, path) = home();
    let container = NativeContainer::open_settings(&path).expect("installation");
    let store = FakeKeychain::default();
    assert_eq!(
        container.begin_pairing(
            0,
            pending(ORIGIN, "synthetic-request-a"),
            &secret("invalid-short-polling-secret"),
            &store,
        ),
        Err(PairingError::InvalidInput)
    );
    assert!(!container.root().join(PAIRING_FILE).exists());
    assert_eq!(store.0.lock().expect("fake").calls, 0);
    store.0.lock().expect("fake").set_error = Some(KeychainError::LockedDenied);
    let request = pending(ORIGIN, "synthetic-request-a");
    assert_eq!(
        container.begin_pairing(0, request.clone(), &secret(POLL), &store),
        Err(PairingError::Keychain(KeychainError::LockedDenied))
    );
    let persisted = container.pairing_catalog().expect("durable pre-POST start");
    assert_eq!(
        persisted.pending_pairings.as_slice(),
        std::slice::from_ref(&request)
    );
    assert_eq!(
        container.pending_pairing_secret(ORIGIN, &store),
        Err(PairingError::Keychain(KeychainError::Missing))
    );
    store.0.lock().expect("fake").set_error = None;
    let calls = store.0.lock().expect("fake").calls;
    assert_eq!(
        container.begin_pairing(0, request.clone(), &secret(POLL), &store),
        Err(PairingError::RevisionConflict)
    );
    assert_eq!(
        store.0.lock().expect("fake").calls,
        calls,
        "stale revisions are fenced before Keychain access"
    );
    let saved = container
        .begin_pairing(persisted.revision, request.clone(), &secret(POLL), &store)
        .expect("exact start retry");
    assert_eq!(
        container.begin_pairing(
            saved.revision,
            request,
            &secret("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
            &store
        ),
        Err(PairingError::RevisionConflict)
    );
    assert_eq!(
        container.begin_pairing(
            saved.revision,
            pending(ORIGIN, "synthetic-replacement-request"),
            &secret(POLL),
            &store
        ),
        Err(PairingError::RevisionConflict)
    );
    assert_eq!(
        container
            .pending_pairing_secret(ORIGIN, &store)
            .expect("original exact poll"),
        secret(POLL)
    );
    let observed = container
        .record_pairing_challenge(
            saved.revision,
            ORIGIN,
            "synthetic-request-a",
            "synthetic-request-a".into(),
            123_456,
        )
        .expect("server expiry hint");
    assert_eq!(observed.pending_pairings[0].expires_at_ms, Some(123_456));
    assert_eq!(
        observed.pending_pairings[0].pairing_id,
        "synthetic-request-a"
    );
    assert_eq!(
        container.record_pairing_challenge(
            observed.revision,
            ORIGIN,
            "synthetic-request-a",
            "synthetic-other-challenge".into(),
            123_456
        ),
        Err(PairingError::RevisionConflict)
    );
}

#[test]
fn origin_installation_and_explicit_selection_isolate_credentials_without_fallback() {
    let (_first, first_path) = home();
    let (_second, second_path) = home();
    let first = NativeContainer::open_settings(&first_path).expect("first installation");
    let second = NativeContainer::open_settings(&second_path).expect("second installation");
    let store = FakeKeychain::default();
    let first_pair = approve(&first, ORIGIN, "synthetic-same-request", &store);
    assert_eq!(
        second.paired_credential(ORIGIN, &store),
        Err(PairingError::Keychain(KeychainError::Missing))
    );
    let second_pending = second
        .begin_pairing(
            0,
            pending(ORIGIN, "synthetic-same-request"),
            &secret("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
            &store,
        )
        .expect("separate installation account");
    assert_eq!(
        second
            .pending_pairing_secret(ORIGIN, &store)
            .expect("second poll"),
        secret("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
    );
    assert_eq!(
        first.paired_credential("https://other.example.test", &store),
        Err(PairingError::Keychain(KeychainError::Missing))
    );
    let other = approve(
        &first,
        "https://other.example.test",
        "synthetic-other-request",
        &store,
    );
    assert_eq!(
        other.selected_origin.as_deref(),
        Some("https://other.example.test/")
    );
    assert_eq!(other.paired_gateways.len(), 2);
    let selected = first
        .select_paired_gateway(other.revision, ORIGIN)
        .expect("explicit selection");
    assert_eq!(selected.selected_origin.as_deref(), Some(ORIGIN));
    assert_eq!(
        first.select_paired_gateway(first_pair.revision, ORIGIN),
        Err(PairingError::RevisionConflict)
    );
    assert_eq!(
        first.select_paired_gateway(selected.revision, "https://unpaired.example.test"),
        Err(PairingError::InvalidInput)
    );
    let forgotten = first
        .remove_paired_gateway(selected.revision, ORIGIN, &store)
        .expect("explicit forget");
    assert_eq!(
        forgotten.selected_origin, None,
        "forget never guesses another reconnect destination"
    );
    assert_eq!(
        first.paired_credential(ORIGIN, &store),
        Err(PairingError::Keychain(KeychainError::Missing))
    );
    assert_eq!(
        first
            .paired_credential("https://other.example.test", &store)
            .expect("other exact origin retained"),
        secret(ACCESS)
    );
    second
        .clear_pending_pairing(
            second_pending.revision,
            ORIGIN,
            "synthetic-same-request",
            &store,
        )
        .expect("explicit exact cancellation");
}

#[test]
fn issued_secret_survives_catalog_failure_and_exact_recovery_after_restart() {
    let (_temporary, path) = home();
    let container = NativeContainer::open_settings(&path).expect("installation");
    let store = FakeKeychain::default();
    let started = container
        .begin_pairing(
            0,
            pending(ORIGIN, "synthetic-request-a"),
            &secret(POLL),
            &store,
        )
        .expect("durable start");
    let metadata = container.root().join(PAIRING_FILE);
    store.0.lock().expect("fake").fail_catalog_after_set = Some(metadata.clone());
    assert_eq!(
        container.complete_pairing(
            started.revision,
            "synthetic-request-a",
            pair(ORIGIN, "issued-a"),
            &secret(ACCESS),
            &store
        ),
        Err(PairingError::CatalogUnavailable)
    );
    assert_eq!(
        container.pairing_catalog(),
        Err(PairingError::CatalogUnavailable),
        "unsafe metadata blocks all use"
    );
    std::fs::set_permissions(&metadata, std::fs::Permissions::from_mode(0o600))
        .expect("repair only isolated injected permission fault");
    drop(container);
    let reopened = NativeContainer::open_settings(&path).expect("restart");
    let interrupted = reopened.pairing_catalog().expect("durable issuance intent");
    assert!(interrupted.paired_gateways.is_empty());
    assert_eq!(
        interrupted.pending_pairings[0].issued_credential,
        Some(pair(ORIGIN, "issued-a"))
    );
    assert_eq!(
        reopened
            .recover_pairing(ORIGIN, &store)
            .expect("recover exact Keychain reference"),
        Some(pair(ORIGIN, "issued-a"))
    );
    let recovered = reopened.pairing_catalog().expect("committed recovery");
    assert!(recovered.pending_pairings.is_empty());
    assert_eq!(recovered.selected_origin.as_deref(), Some(ORIGIN));
    assert_eq!(
        reopened
            .paired_credential(ORIGIN, &store)
            .expect("preserved issued secret"),
        secret(ACCESS)
    );
}

#[test]
fn denied_cleanup_retains_reference_and_recovery_never_claims_a_missing_secret() {
    let (_temporary, path) = home();
    let container = NativeContainer::open_settings(&path).expect("installation");
    let store = FakeKeychain::default();
    let first = approve(&container, ORIGIN, "synthetic-old-pair", &store);
    let started = container
        .begin_pairing(
            first.revision,
            pending(ORIGIN, "synthetic-new-pair"),
            &secret(POLL),
            &store,
        )
        .expect("new exact start");
    store.0.lock().expect("fake").set_error = Some(KeychainError::LockedDenied);
    assert_eq!(
        container.complete_pairing(
            started.revision,
            "synthetic-new-pair",
            pair(ORIGIN, "issued-new"),
            &secret("new-issued-secret"),
            &store
        ),
        Err(PairingError::Keychain(KeychainError::LockedDenied))
    );
    assert_eq!(
        container
            .recover_pairing(ORIGIN, &store)
            .expect("no fabricated secret"),
        None
    );
    assert_eq!(
        container
            .paired_credential(ORIGIN, &store)
            .expect("previous pair retained on failure"),
        secret(ACCESS)
    );
    let interrupted = container.pairing_catalog().expect("exact pending state");
    {
        let mut fake = store.0.lock().expect("fake");
        fake.set_error = None;
        fake.delete_error = Some(KeychainError::LockedDenied);
    }
    assert_eq!(
        container.complete_pairing(
            interrupted.revision,
            "synthetic-new-pair",
            pair(ORIGIN, "issued-new"),
            &secret("new-issued-secret"),
            &store
        ),
        Err(PairingError::Keychain(KeychainError::LockedDenied))
    );
    let cleanup_pending = container
        .pairing_catalog()
        .expect("committed pair with retained cleanup reference");
    assert_eq!(
        cleanup_pending.paired_gateways[0].credential_id,
        "issued-new"
    );
    assert_eq!(cleanup_pending.pending_pairings.len(), 1);
    store.0.lock().expect("fake").delete_error = None;
    container
        .recover_pairing(ORIGIN, &store)
        .expect("repeat exact cleanup");
    assert!(container
        .pairing_catalog()
        .expect("clean pair")
        .pending_pairings
        .is_empty());
    assert!(
        store.0.lock().expect("fake").items.contains_key(&(
            container
                .credential_item(&pair(ORIGIN, "synthetic-old-pair"))
                .service()
                .into(),
            container
                .credential_item(&pair(ORIGIN, "synthetic-old-pair"))
                .account()
                .into()
        )),
        "re-pairing does not silently delete a prior credential"
    );
    store.0.lock().expect("fake").get_error = Some(KeychainError::LockedDenied);
    assert_eq!(
        container.paired_credential(ORIGIN, &store),
        Err(PairingError::Keychain(KeychainError::LockedDenied))
    );
}

#[test]
fn hostile_metadata_origins_and_secret_diagnostics_fail_closed() {
    for origin in [
        "http://remote.example.test",
        "https://gateway.example.test/path",
        "https://user:password@gateway.example.test",
        "https://gateway.example.test/?secret=value",
    ] {
        assert_eq!(
            PendingPairing::new(origin, "synthetic-request-a".into()),
            Err(PairingError::InvalidInput)
        );
    }
    for value in ["", "contains whitespace", "line\nbreak", "non-ascii-秘密"] {
        assert!(SecretToken::new(value.into()).is_err());
    }
    assert!(!format!("{:?}", secret(ACCESS)).contains(ACCESS));
    for error in [
        KeychainError::Missing,
        KeychainError::LockedDenied,
        KeychainError::Unavailable,
    ] {
        assert!(!format!("{error:?} {error}").contains(ACCESS));
    }
    let (_temporary, path) = home();
    let container = NativeContainer::open_settings(&path).expect("installation");
    let store = FakeKeychain::default();
    let begun = container
        .begin_pairing(
            0,
            pending(ORIGIN, "synthetic-request-a"),
            &secret(POLL),
            &store,
        )
        .expect("durable exact start");
    let outside = path.join("outside.json");
    std::fs::write(&outside, "untrusted metadata").expect("isolated target");
    std::fs::remove_file(container.root().join(PAIRING_FILE)).expect("replace fixture metadata");
    std::os::unix::fs::symlink(&outside, container.root().join(PAIRING_FILE))
        .expect("hostile metadata link");
    let calls = store.0.lock().expect("fake").calls;
    assert_eq!(
        container.clear_pending_pairing(begun.revision, ORIGIN, "synthetic-request-a", &store),
        Err(PairingError::CatalogUnavailable)
    );
    assert_eq!(
        store.0.lock().expect("fake").calls,
        calls,
        "unsafe metadata cannot authorize any Keychain access"
    );
    assert_eq!(
        std::fs::read_to_string(&outside).expect("outside retained"),
        "untrusted metadata"
    );
}

#[test]
fn session_only_marker_round_trips_and_rejects_unknown_modes_without_dropping_prior_durable_pair() {
    let (_temporary, path) = home();
    let container = NativeContainer::open_settings(&path).expect("installation");
    let durable_store = FakeKeychain::default();
    let session_store = FakeKeychain::default();

    // First pair durably.
    let begun = container
        .begin_pairing(
            0,
            pending(ORIGIN, "synthetic-request-durable-1"),
            &secret(POLL),
            &durable_store,
        )
        .expect("begin durable");
    let durable_pair =
        PairedGateway::new(ORIGIN, "cred-durable".into(), "dep-1".into()).expect("durable pair");
    let paired = container
        .complete_pairing(
            begun.revision,
            "synthetic-request-durable-1",
            durable_pair.clone(),
            &secret(ACCESS),
            &durable_store,
        )
        .expect("complete durable");
    assert_eq!(paired.paired_gateways, vec![durable_pair.clone()]);

    // Start a session-only replacement and abandon before completing -> durable pair preserved.
    let started_replacement = container
        .begin_pairing(
            paired.revision,
            pending(ORIGIN, "synthetic-request-session-1"),
            &secret(&"e".repeat(64)),
            &session_store,
        )
        .expect("begin replacement");
    let aborted = container
        .clear_pending_pairing(
            started_replacement.revision,
            ORIGIN,
            "synthetic-request-session-1",
            &session_store,
        )
        .expect("abandon replacement");
    assert_eq!(aborted.paired_gateways, vec![durable_pair]);

    // Now finish a session-only pairing: updates marker to session_only without writing secret.
    let begun_session = container
        .begin_pairing(
            aborted.revision,
            pending(ORIGIN, "synthetic-request-session-2"),
            &secret(&"f".repeat(64)),
            &session_store,
        )
        .expect("begin session");
    let session_pair =
        PairedGateway::new_session_only(ORIGIN, "cred-session".into(), "dep-1".into())
            .expect("session pair");
    let finished = container
        .complete_pairing(
            begun_session.revision,
            "synthetic-request-session-2",
            session_pair.clone(),
            &secret(&"1".repeat(64)),
            &session_store,
        )
        .expect("complete session");
    assert!(!session_pair.is_durable());
    assert_eq!(finished.paired_gateways, vec![session_pair]);

    let metadata = std::fs::read_to_string(container.root().join(PAIRING_FILE)).expect("json");
    assert!(metadata.contains("\"credential_storage\":\"session_only\""));
    assert!(!metadata.contains(&"1".repeat(64)));

    // Reject unknown credential_storage values closed.
    let corrupted = metadata.replace("\"session_only\"", "\"plaintext_file\"");
    std::fs::write(container.root().join(PAIRING_FILE), corrupted).expect("write tampered");
    assert_eq!(
        container.pairing_catalog(),
        Err(PairingError::CatalogUnavailable)
    );
}
