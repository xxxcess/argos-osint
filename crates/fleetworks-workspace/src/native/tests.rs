use fleetworks_contracts::connect::RegistrationEnsureRequest;
use fleetworks_contracts::connect::RegistrationReceipt;

use super::NativeContainer;
use super::RemovalKind;

#[test]
fn unknown_registration_outcome_survives_reopen_and_fences_removal() {
    let temporary = tempfile::tempdir().expect("isolated synthetic home");
    let home = temporary
        .path()
        .canonicalize()
        .expect("canonical fixture home");
    let container = NativeContainer::open(&home).expect("owned container");
    let workspace = container.create("research").expect("owned child");
    let request = RegistrationEnsureRequest::new(
        "synthetic-request-1".into(),
        "192.0.2.42".parse().expect("fixture IP"),
        workspace.name().clone(),
        container.installation_marker().to_owned(),
        workspace.marker().to_owned(),
    );
    let attempt = container
        .begin_registration_attempt(&workspace, "deployment-a", request.clone())
        .expect("journal precedes HTTP");
    assert_eq!(
        container
            .begin_registration_attempt(&workspace, "deployment-a", request.clone())
            .expect("exact retry"),
        attempt
    );
    let mut changed = request.clone();
    changed.workstation_ip = "192.0.2.43".parse().expect("different fixture IP");
    assert!(container
        .begin_registration_attempt(&workspace, "deployment-a", changed)
        .is_err());
    assert!(container
        .begin_removal(&workspace, "remove-1", RemovalKind::Delete)
        .is_err());
    drop(container);
    let container = NativeContainer::open(&home).expect("restart does not forget unknown outcome");
    assert_eq!(
        container
            .pending_registration_attempts()
            .expect("persisted journal"),
        vec![attempt]
    );
    let workspace = container
        .select(Some("research".as_ref()))
        .expect("existing child");
    let receipt = RegistrationReceipt::new(
        "deployment-a".into(),
        "node-a".into(),
        "workspace-a".into(),
        "registration-a".into(),
        "192.0.2.42:research".parse().expect("fixture Connect ID"),
        1,
    );
    let registered = container
        .record_registration(&workspace, &receipt)
        .expect("authoritative receipt reconciles intent");
    assert!(container
        .pending_registration_attempts()
        .expect("journal reconciled")
        .is_empty());
    let removal = container
        .begin_removal(&registered, "remove-2", RemovalKind::Delete)
        .expect("real known binding remains protected by retirement");
    assert_eq!(removal.bindings.len(), 1);
    assert!(container.finish_removal("remove-2").is_err());
    assert!(
        registered.path().exists(),
        "unknown or unretired outcomes never remove bytes"
    );
}

#[test]
fn mismatched_receipt_cannot_clear_attempt_or_attach_another_ip() {
    let temporary = tempfile::tempdir().expect("isolated synthetic home");
    let home = temporary
        .path()
        .canonicalize()
        .expect("canonical fixture home");
    let container = NativeContainer::open(&home).expect("owned container");
    let workspace = container.create("research").expect("owned child");
    let request = RegistrationEnsureRequest::new(
        "synthetic-request-1".into(),
        "192.0.2.42".parse().expect("fixture IP"),
        workspace.name().clone(),
        container.installation_marker().to_owned(),
        workspace.marker().to_owned(),
    );
    container
        .begin_registration_attempt(&workspace, "deployment-a", request)
        .expect("journal intent");
    let wrong = RegistrationReceipt::new(
        "deployment-a".into(),
        "node-a".into(),
        "workspace-a".into(),
        "registration-a".into(),
        "192.0.2.43:research".parse().expect("different fixture IP"),
        1,
    );
    assert!(container.record_registration(&workspace, &wrong).is_err());
    assert_eq!(
        container
            .pending_registration_attempts()
            .expect("unknown original preserved")
            .len(),
        1
    );
    assert!(container
        .begin_removal(&workspace, "remove-1", RemovalKind::RemoveFromApp)
        .is_err());
}
