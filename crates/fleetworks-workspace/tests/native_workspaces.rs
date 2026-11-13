#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::path::PathBuf;

use fleetworks_workspace::native::legacy_connect::RegistrationReceipt;
use fleetworks_workspace::native::NativeContainer;
use fleetworks_workspace::native::NativeWorkspace;
use fleetworks_workspace::native::OperationPhase;
use fleetworks_workspace::native::RemovalKind;
use fleetworks_workspace::native::RemovalOutcome;
use fleetworks_workspace::native::RetirementReceipt;
use fleetworks_workspace::native::WorkspaceState;

fn home() -> (tempfile::TempDir, PathBuf) {
    let temporary = tempfile::tempdir().expect("owned temporary fixture home");
    let canonical = temporary
        .path()
        .canonicalize()
        .expect("canonical fixture path without /var symlink");
    (temporary, canonical)
}

fn receipt(
    workspace: &NativeWorkspace,
    deployment: &str,
    registration: &str,
) -> RegistrationReceipt {
    let connect = format!("192.0.2.42:{}", workspace.name())
        .parse()
        .expect("synthetic Connect ID");
    RegistrationReceipt::new(
        deployment.into(),
        "node-a".into(),
        "workspace-a".into(),
        registration.into(),
        connect,
        3,
    )
}

#[test]
fn first_use_is_private_and_reuses_root_and_default_identities() {
    let (_temporary, home) = home();
    let container = NativeContainer::open(&home).expect("first-use setup");
    let default = container.default_workspace().expect("default child");
    assert_eq!(container.root(), home.join(".fleetworks"));
    assert_eq!(default.path(), home.join(".fleetworks/default"));
    assert!(default.is_default());
    assert_eq!(default.state(), WorkspaceState::Local);
    assert_eq!(
        fs::metadata(container.root())
            .expect("root mode")
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(default.path())
            .expect("child mode")
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(container.root().join(".native-state.json"))
            .expect("catalog mode")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let marker = default.marker().to_owned();
    let installation = container.installation_marker().to_owned();
    drop(container);
    let reopened = NativeContainer::open(&home).expect("reopen");
    assert_eq!(reopened.installation_marker(), installation);
    assert_eq!(
        reopened.default_workspace().expect("same default").marker(),
        marker
    );
    for legacy in ["config", "secrets", "storage", "models"] {
        assert!(
            !default.path().join(legacy).exists(),
            "native child must not clone operator configuration"
        );
    }
}

#[test]
fn names_are_single_contained_children_with_unicode_and_case_collision_guards() {
    let (_temporary, home) = home();
    let container = NativeContainer::open(&home).expect("container");
    let first = container.create("Cafe\u{301} research").expect("NFC child");
    assert_eq!(first.name().as_str(), "Café research");
    assert!(container.create("CAFÉ RESEARCH").is_err());
    container.create("Maße").expect("Unicode casefold name");
    assert!(container.create("MASSE").is_err());
    for invalid in [
        "",
        ".",
        "..",
        ".internal",
        "default",
        "DEFAULT",
        "a/b",
        "a\\b",
        "/tmp/review",
        " x",
        "x ",
        "a:b",
    ] {
        assert!(container.create(invalid).is_err(), "reject {invalid:?}");
    }
    for selection in [
        container.root().to_path_buf(),
        home.clone(),
        home.join("elsewhere"),
        first.path().join("nested"),
        PathBuf::from("Café research/nested"),
    ] {
        assert!(
            container.select(Some(&selection)).is_err(),
            "reject selection {}",
            selection.display()
        );
    }
    assert_eq!(
        container
            .select(Some(Path::new("Café research")))
            .expect("known basename")
            .marker(),
        first.marker()
    );
    assert_eq!(
        container
            .select(Some(first.path()))
            .expect("known child path")
            .marker(),
        first.marker()
    );
}

#[test]
fn unknown_folders_and_old_operator_root_are_never_adopted() {
    let (_temporary, home) = home();
    let container = NativeContainer::open(&home).expect("container");
    fs::create_dir(container.root().join("unknown")).expect("unmanaged fixture folder");
    assert_eq!(container.list().expect("verified catalog only").len(), 1);
    assert!(container.select(Some(Path::new("unknown"))).is_err());
    assert!(container
        .attach_owned_child("unknown", &"a".repeat(64))
        .is_err());
    assert!(container.create("unknown").is_err());
    let (_legacy_temporary, legacy_home) = self::home();
    let legacy_root = legacy_home.join(".fleetworks");
    fs::create_dir(&legacy_root).expect("legacy root fixture");
    fs::set_permissions(&legacy_root, fs::Permissions::from_mode(0o700))
        .expect("fixture permissions");
    fs::write(
        legacy_root.join("old-config.toml"),
        b"synthetic old configuration",
    )
    .expect("legacy fixture");
    assert!(NativeContainer::open(&legacy_home).is_err());
    assert!(!legacy_root.join("default").exists());
    assert_eq!(
        fs::read(legacy_root.join("old-config.toml")).expect("preserved old data"),
        b"synthetic old configuration"
    );
}

#[test]
fn protected_default_cannot_be_removed_even_with_a_confirmed_snapshot() {
    let (_temporary, home) = home();
    let container = NativeContainer::open(&home).expect("container");
    let default = container.default_workspace().expect("default");
    for kind in [RemovalKind::RemoveFromApp, RemovalKind::Delete] {
        assert!(container
            .begin_removal(&default, "remove-default", kind)
            .is_err());
    }
    assert!(default.path().exists());
    assert!(container.root().exists());
    assert!(container
        .pending_operations()
        .expect("no protected operation")
        .is_empty());
}

#[test]
fn symlink_child_and_parent_substitution_fail_without_touching_external_data() {
    let (_temporary, home) = home();
    let container = NativeContainer::open(&home).expect("container");
    let child = container.create("research").expect("child");
    let external = home.join("external");
    fs::create_dir(&external).expect("owned external fixture");
    fs::write(external.join("keep.txt"), b"outside").expect("external bytes");
    fs::rename(child.path(), home.join("held-child")).expect("simulate child substitution");
    std::os::unix::fs::symlink(&external, child.path()).expect("substituted symlink");
    assert!(container.select(Some(child.path())).is_err());
    assert!(container
        .begin_removal(&child, "remove-child", RemovalKind::Delete)
        .is_err());
    assert_eq!(
        fs::read(external.join("keep.txt")).expect("external preserved"),
        b"outside"
    );
    fs::remove_file(child.path()).expect("remove only fixture symlink");
    fs::rename(home.join("held-child"), child.path()).expect("restore fixture child");
    let held_root = home.join("held-root");
    fs::rename(container.root(), &held_root).expect("simulate parent substitution");
    std::os::unix::fs::symlink(&external, container.root()).expect("substituted root");
    assert!(container.create("must-not-appear").is_err());
    assert!(!external.join("must-not-appear").exists());
    assert!(held_root
        .join("default/.fleetworks-workspace.json")
        .exists());
}

#[test]
fn deletion_requires_every_backend_and_survives_reopen_without_deleting_bytes() {
    let (_temporary, home) = home();
    let container = NativeContainer::open(&home).expect("container");
    let child = container.create("research").expect("child");
    fs::write(child.path().join("keep.txt"), b"workspace content").expect("data fixture");
    let child = container
        .record_registration(&child, &receipt(&child, "deployment-a", "registration-a"))
        .expect("first binding");
    let child = container
        .record_registration(&child, &receipt(&child, "deployment-b", "registration-b"))
        .expect("second binding");
    let operation = container
        .begin_removal(&child, "delete-1", RemovalKind::Delete)
        .expect("confirmed deletion intent");
    assert_eq!(operation.bindings.len(), 2);
    assert!(container.finish_removal("delete-1").is_err());
    assert!(
        container
            .record_registration(&child, &receipt(&child, "deployment-c", "registration-c"))
            .is_err(),
        "new admission fenced locally"
    );
    let mut not_drained = RetirementReceipt::confirmed(
        "deployment-a".into(),
        "registration-a".into(),
        4,
        "retire-a".into(),
    );
    not_drained.active_work_drained = false;
    assert!(container
        .record_retirement("delete-1", not_drained)
        .is_err());
    container
        .record_retirement(
            "delete-1",
            RetirementReceipt::confirmed(
                "deployment-a".into(),
                "registration-a".into(),
                4,
                "retire-a".into(),
            ),
        )
        .expect("first retirement");
    assert!(
        container.finish_removal("delete-1").is_err(),
        "unavailable second backend keeps deletion pending"
    );
    drop(container);
    let reopened = NativeContainer::open(&home).expect("recover journal");
    assert_eq!(
        reopened.pending_operations().expect("pending journal")[0]
            .receipts
            .len(),
        1
    );
    reopened
        .record_retirement(
            "delete-1",
            RetirementReceipt::confirmed(
                "deployment-b".into(),
                "registration-b".into(),
                4,
                "retire-b".into(),
            ),
        )
        .expect("second retirement");
    let outcome = reopened
        .finish_removal("delete-1")
        .expect("retirement finished");
    let RemovalOutcome::TrashPending(intent) = outcome else {
        panic!("physical Trash must remain pending")
    };
    reopened
        .verify_trash_intent(&intent)
        .expect("same exact child");
    assert_eq!(
        fs::read(intent.workspace().path().join("keep.txt")).expect("bytes remain for OS adapter"),
        b"workspace content"
    );
    assert!(reopened
        .record_trash_completed("delete-1", "false-trash-receipt")
        .is_err());
    assert_eq!(
        reopened.pending_operations().expect("cleanup pending")[0].phase,
        OperationPhase::CleanupPending
    );
}

#[test]
fn real_adapter_receipt_allows_new_name_identity_without_erasing_history() {
    let (_temporary, home) = home();
    let container = NativeContainer::open(&home).expect("container");
    let child = container.create("research").expect("child");
    container
        .begin_removal(&child, "delete-1", RemovalKind::Delete)
        .expect("deletion intent");
    assert!(matches!(
        container
            .finish_removal("delete-1")
            .expect("no backend bindings"),
        RemovalOutcome::TrashPending(_)
    ));
    let fixture_trash = home.join("fixture-trash-owned-by-test");
    fs::rename(child.path(), &fixture_trash)
        .expect("simulate external OS adapter within owned fixture only");
    container
        .record_trash_completed("delete-1", "os-receipt-1")
        .expect("actual absence and receipt");
    let replacement = container
        .create("research")
        .expect("new child after completed deletion");
    assert_ne!(replacement.marker(), child.marker());
    assert!(
        fixture_trash.join(".fleetworks-workspace.json").exists(),
        "manager never erased fixture bytes"
    );
    assert!(container
        .default_workspace()
        .expect("default retained")
        .is_default());
}

#[test]
fn remove_from_app_preserves_folder_and_explicit_attachment_reuses_identity() {
    let (_temporary, home) = home();
    let container = NativeContainer::open(&home).expect("container");
    let child = container.create("research").expect("child");
    container
        .begin_removal(&child, "forget-1", RemovalKind::RemoveFromApp)
        .expect("forget intent");
    assert_eq!(
        container.finish_removal("forget-1").expect("no bindings"),
        RemovalOutcome::RemovedFromApp
    );
    assert!(child.path().exists());
    assert_eq!(
        container
            .list()
            .expect("forgotten association hidden")
            .len(),
        1
    );
    assert!(container
        .attach_owned_child("research", "wrong-marker")
        .is_err());
    let attached = container
        .attach_owned_child("research", child.marker())
        .expect("explicit owned attachment");
    assert_eq!(attached.marker(), child.marker());
    assert_eq!(attached.state(), WorkspaceState::Local);
}

#[test]
fn missing_default_and_conflicting_markers_are_repair_states_not_resurrection() {
    let (_temporary, home) = home();
    let container = NativeContainer::open(&home).expect("container");
    let default = container.default_workspace().expect("default");
    fs::rename(default.path(), home.join("moved-default")).expect("simulate missing default");
    assert!(container.create("other").is_err());
    drop(container);
    assert!(NativeContainer::open(&home).is_err());
    assert!(
        !home.join(".fleetworks/default").exists(),
        "do not invent a new protected identity"
    );
    fs::rename(home.join("moved-default"), home.join(".fleetworks/default"))
        .expect("restore fixture default");
    let reopened = NativeContainer::open(&home).expect("identity repaired");
    let child = reopened.create("research").expect("child");
    fs::write(child.path().join(".fleetworks-workspace.json"), b"{}")
        .expect("simulate marker conflict");
    assert!(reopened.select(Some(child.path())).is_err());
    assert!(reopened
        .begin_removal(&child, "delete-conflict", RemovalKind::Delete)
        .is_err());
}

#[test]
fn unsafe_existing_permissions_block_instead_of_silently_chmodding() {
    let (_temporary, home) = home();
    let container = NativeContainer::open(&home).expect("container");
    let child = container.create("research").expect("child");
    fs::set_permissions(child.path(), fs::Permissions::from_mode(0o755))
        .expect("simulate unsafe child mode");
    assert!(container.select(Some(child.path())).is_err());
    assert_eq!(
        fs::metadata(child.path())
            .expect("mode retained for repair")
            .permissions()
            .mode()
            & 0o777,
        0o755
    );
}
