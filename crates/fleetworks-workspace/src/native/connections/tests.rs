use std::io::Read;
use std::os::unix::fs::PermissionsExt;

use super::*;

fn token(home: &Path, name: &str) -> PathBuf {
    let path = home.join(name);
    std::fs::write(&path, b"synthetic-token-never-in-profile-json").expect("synthetic token bytes");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
        .expect("private synthetic token");
    path
}
fn input(home: &Path, name: &str) -> GatewayProfileInput {
    let reference =
        CredentialReference::capture(&home.join("token"), "https://gateway.example.test")
            .expect("explicit protected reference");
    GatewayProfileInput::new(
        name.into(),
        "https://gateway.example.test".into(),
        reference,
    )
    .expect("validated profile input")
}

#[test]
fn catalog_read_is_nonmutating_and_save_restart_revision_preserves_workspace_journals() {
    let temporary = tempfile::tempdir().expect("isolated fixture home");
    let home = temporary
        .path()
        .canonicalize()
        .expect("real fixture directory");
    token(&home, "token");
    let container = NativeContainer::open(&home).expect("owned container");
    let before = std::fs::read(container.root().join(super::super::state::CATALOG_FILE))
        .expect("workspace identity snapshot");
    assert_eq!(
        container.connection_profiles().expect("empty read"),
        ConnectionProfilesSnapshot::empty()
    );
    assert!(
        !container.root().join(CONNECTIONS_FILE).exists(),
        "a read never creates a saved profile store"
    );
    let saved = container
        .save_connection_profile(0, None, input(&home, "Review gateway"))
        .expect("explicit save");
    assert_eq!(saved.revision, 1);
    assert_eq!(saved.profiles[0].revision, 1);
    assert!(container
        .save_connection_profile(0, None, input(&home, "Stale writer"))
        .is_err());
    let body = std::fs::read_to_string(container.root().join(CONNECTIONS_FILE))
        .expect("private current catalog");
    assert!(!body.contains("synthetic-token-never-in-profile-json"));
    assert!(body.contains("credential") && body.contains("gateway.example.test"));
    assert_eq!(
        std::fs::read(container.root().join(super::super::state::CATALOG_FILE))
            .expect("unchanged identities"),
        before
    );
    drop(container);
    let reopened = NativeContainer::open(&home).expect("restart owned container");
    assert_eq!(
        reopened
            .connection_profiles()
            .expect("reopened saved catalog"),
        saved
    );
}

#[test]
fn origin_binding_and_replaced_or_linked_credentials_fail_closed() {
    let temporary = tempfile::tempdir().expect("isolated fixture home");
    let home = temporary
        .path()
        .canonicalize()
        .expect("real fixture directory");
    let path = token(&home, "token");
    let reference = CredentialReference::capture(&path, "https://one.example.test")
        .expect("exact origin reference");
    assert!(reference.open_verified("https://two.example.test").is_err());
    assert!(GatewayProfileInput::new(
        "Another gateway".into(),
        "https://two.example.test".into(),
        reference.clone()
    )
    .is_err());
    let replacement = token(&home, "replacement");
    std::fs::rename(&replacement, &path).expect("explicit synthetic rotation");
    assert!(
        reference.open_verified("https://one.example.test").is_err(),
        "atomic rotation requires explicit reselection"
    );
    let reference = CredentialReference::capture(&path, "https://one.example.test")
        .expect("explicit renewed reference");
    let mut body = String::new();
    reference
        .open_verified("https://one.example.test")
        .expect("owned exact file")
        .read_to_string(&mut body)
        .expect("bounded fixture read");
    assert_eq!(body, "synthetic-token-never-in-profile-json");
    std::fs::hard_link(&path, home.join("token-hardlink")).expect("synthetic hardlink");
    assert!(CredentialReference::capture(&path, "https://one.example.test").is_err());
    std::os::unix::fs::symlink(&path, home.join("token-symlink")).expect("synthetic symlink");
    assert!(
        CredentialReference::capture(&home.join("token-symlink"), "https://one.example.test")
            .is_err()
    );
}

#[test]
fn unknown_store_is_never_adopted_or_overwritten() {
    let temporary = tempfile::tempdir().expect("isolated fixture home");
    let home = temporary
        .path()
        .canonicalize()
        .expect("real fixture directory");
    token(&home, "token");
    let container = NativeContainer::open(&home).expect("owned container");
    let path = container.root().join(CONNECTIONS_FILE);
    let original = b"{\"old_token\":\"synthetic-legacy\"}";
    std::fs::write(&path, original).expect("unrecognized synthetic document");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
        .expect("private fixture file");
    assert!(container.connection_profiles().is_err());
    assert!(container
        .save_connection_profile(0, None, input(&home, "New profile"))
        .is_err());
    assert_eq!(std::fs::read(&path).expect("preserved original"), original);
}

#[test]
fn edit_selection_and_duplicate_names_obey_catalog_revision() {
    let temporary = tempfile::tempdir().expect("isolated fixture home");
    let home = temporary
        .path()
        .canonicalize()
        .expect("real fixture directory");
    token(&home, "token");
    let container = NativeContainer::open(&home).expect("owned container");
    let first = container
        .save_connection_profile(0, None, input(&home, "Local review"))
        .expect("first profile");
    let id = first.profiles[0].id.clone();
    assert!(container
        .save_connection_profile(1, None, input(&home, "LOCAL REVIEW"))
        .is_err());
    let second = container
        .save_connection_profile(1, None, input(&home, "Remote review"))
        .expect("second profile");
    assert!(container.select_connection_profile(1, &id).is_err());
    let selected = container
        .select_connection_profile(second.revision, &id)
        .expect("explicit selection");
    assert_eq!(selected.selected_profile_id.as_deref(), Some(id.as_str()));
    let edited = container
        .save_connection_profile(selected.revision, Some(&id), input(&home, "Renamed review"))
        .expect("revision-pinned edit");
    assert_eq!(edited.profiles[0].id, id);
    assert_eq!(edited.profiles[0].revision, edited.revision);
    assert_eq!(edited.profiles.len(), 2);
}

#[test]
fn corrupt_binding_permissions_and_record_limits_are_rejected_without_secrets() {
    let temporary = tempfile::tempdir().expect("isolated fixture home");
    let home = temporary
        .path()
        .canonicalize()
        .expect("real fixture directory");
    let path = token(&home, "token");
    let container = NativeContainer::open(&home).expect("owned container");
    assert!(CredentialReference::capture(&path, "http://remote.example.test").is_err());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
        .expect("unsafe synthetic mode");
    assert!(CredentialReference::capture(&path, "http://127.0.0.1:8080").is_err());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
        .expect("restore private fixture mode");
    let mut snapshot = ConnectionProfilesSnapshot::empty();
    for index in 0..MAX_PROFILES {
        snapshot = container
            .save_connection_profile(
                snapshot.revision,
                None,
                input(&home, &format!("Review {index}")),
            )
            .expect("bounded named records");
    }
    assert!(container
        .save_connection_profile(snapshot.revision, None, input(&home, "One too many"))
        .is_err());
    let mut body: serde_json::Value = serde_json::from_slice(
        &std::fs::read(container.root().join(CONNECTIONS_FILE)).expect("catalog bytes"),
    )
    .expect("current metadata");
    body["installation_marker"] = "different-installation".into();
    std::fs::write(
        container.root().join(CONNECTIONS_FILE),
        serde_json::to_vec(&body).expect("synthetic corrupt ownership"),
    )
    .expect("replace test metadata");
    assert!(container.connection_profiles().is_err());
}
