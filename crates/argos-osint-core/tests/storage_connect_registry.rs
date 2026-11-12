//! Real isolated registry/session persistence; no client marker grants authority.
#![cfg(feature = "surreal")]

mod support;

use argos_osint_core::storage::connect_registry;
use argos_osint_core::storage::connect_registry::RebindInput;
use argos_osint_core::storage::connect_registry::RegistryAuthority;
use argos_osint_core::storage::connect_registry::RegistryError;
use argos_osint_core::storage::repo::Repository;
use argos_osint_core::storage::tx;
use fleetworks_contracts::connect::RegistrationEnsureRequest;
use fleetworks_contracts::connect::WorkspaceName;

use support::Server;

fn authority(scope: &str, credential: &str) -> RegistryAuthority {
    RegistryAuthority::new(scope, credential, "synthetic-access-epoch")
        .expect("synthetic authority must be valid")
}

fn request(
    key: &str,
    ip: &str,
    name: &str,
    install: &str,
    folder: &str,
) -> RegistrationEnsureRequest {
    RegistrationEnsureRequest::new(
        key.into(),
        ip.parse().expect("synthetic unicast IP"),
        WorkspaceName::new(name).expect("synthetic child name"),
        install.into(),
        folder.into(),
    )
}

async fn initialize(repo: &Repository, deployment: &str) {
    repo.db()
        .query("UPSERT storage_meta:identity SET instance_id=$identity RETURN NONE")
        .bind(("identity", deployment.to_owned()))
        .await
        .expect("fixture identity write")
        .check()
        .expect("fixture identity statement");
    connect_registry::initialize_control(repo.db())
        .await
        .expect("explicit synthetic bootstrap");
}

#[tokio::test]
async fn ensure_reuses_concurrent_owned_identity_and_rejects_ip_marker_and_request_collisions() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "connect_ensure").await;
    initialize(&repo, "synthetic-deployment-1").await;
    let owner = authority("scope-a", "native-a");
    let first = request(
        "ensure-a",
        "192.0.2.42",
        "research",
        "synthetic-install-a",
        "synthetic-folder-a",
    );
    let mut retry = first.clone();
    retry.request_id = "ensure-b".into();
    let (one, two) = tokio::join!(
        connect_registry::ensure(&repo, &owner, &first),
        connect_registry::ensure(&repo, &owner, &retry)
    );
    let one = one.expect("first concurrent ensure");
    let two = two.expect("second concurrent ensure");
    assert_eq!(one.receipt, two.receipt);
    assert_ne!(
        one.created, two.created,
        "exactly one request creates the binding"
    );
    assert_eq!(one.receipt.connect_id.to_string(), "192.0.2.42:research");
    let replay = connect_registry::ensure(&repo, &owner, &first)
        .await
        .expect("idempotent replay");
    assert_eq!(replay.receipt, one.receipt);
    assert!(!replay.created);
    let mut observed = first.clone();
    observed.request_id = "ensure-observed".into();
    let observed = connect_registry::ensure_observed(
        &repo,
        &owner,
        &observed,
        Some("198.51.100.10".parse().expect("synthetic gateway peer")),
    )
    .await
    .expect("separate peer audit");
    assert_eq!(
        observed.receipt.workstation_ip.to_string(),
        "192.0.2.42",
        "gateway-observed peer cannot replace native identity"
    );
    let mut audit = repo
        .db()
        .query("SELECT VALUE observed_peer_ip FROM connect_workstation WHERE workstation_id=$id")
        .bind(("id", one.receipt.workstation_id.clone()))
        .await
        .expect("peer audit read")
        .check()
        .expect("peer audit statement");
    assert_eq!(
        audit.take::<Vec<String>>(0).expect("peer audit values"),
        ["198.51.100.10"]
    );
    for other in [
        request(
            "collision-ip",
            "192.0.2.42",
            "other",
            "synthetic-install-b",
            "synthetic-folder-b",
        ),
        request(
            "collision-folder",
            "192.0.2.42",
            "research",
            "synthetic-install-a",
            "synthetic-folder-c",
        ),
        request(
            "ensure-a",
            "192.0.2.42",
            "renamed",
            "synthetic-install-a",
            "synthetic-folder-a",
        ),
    ] {
        assert!(matches!(
            connect_registry::ensure(&repo, &owner, &other).await,
            Err(RegistryError::Collision)
        ));
    }
    let other_scope = authority("scope-b", "native-b");
    assert!(matches!(
        connect_registry::ensure(&repo, &other_scope, &first).await,
        Err(RegistryError::Collision)
    ));
    let duplicate_claim = repo.db().query("CREATE connect_ip_claim:forged_duplicate SET workstation_ip='192.0.2.42', workstation_id='forged' RETURN NONE").await.expect("constraint query transport").check();
    assert!(
        duplicate_claim.is_err(),
        "database uniqueness must reject a duplicate even outside application preflight"
    );
}

#[tokio::test]
async fn session_is_hashed_revocable_revision_bound_and_does_not_register_unknown_ids() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "connect_sessions").await;
    initialize(&repo, "synthetic-deployment-2").await;
    let native = authority("scope-a", "native-a");
    let relay = authority("scope-a", "web-a");
    let input = request(
        "ensure-a",
        "192.0.2.43",
        "research",
        "synthetic-install-a",
        "synthetic-folder-a",
    );
    let registration = connect_registry::ensure(&repo, &native, &input)
        .await
        .expect("native registration")
        .receipt;
    let unknown = "192.0.2.44:unregistered"
        .parse()
        .expect("synthetic Connect ID");
    assert!(matches!(
        connect_registry::create_session(&repo, &relay, &unknown).await,
        Err(RegistryError::NotFound)
    ));
    let session = connect_registry::create_session(&repo, &relay, &registration.connect_id)
        .await
        .expect("known session");
    let token = session.token.expose_secret().to_owned();
    assert_eq!(token.len(), 64);
    assert!(!format!("{:?}", session.token).contains(&token));
    let mut rows = repo.db().query("SELECT VALUE <string>id FROM connect_session; SELECT VALUE registration_id FROM connect_registration").await.expect("session fixture reads").check().expect("session fixture read statements");
    let keys: Vec<String> = rows.take(0).expect("hashed session keys");
    assert!(
        keys.iter().all(|key| !key.contains(&token)),
        "plaintext session must not be persisted"
    );
    assert_eq!(
        rows.take::<Vec<String>>(1).expect("registration IDs").len(),
        1,
        "unknown browser IDs never register themselves"
    );
    let validated = connect_registry::validate_session(
        &repo,
        &relay,
        &token,
        Some(session.context.context_revision),
    )
    .await
    .expect("current proof");
    assert_eq!(validated.context, session.context);
    assert!(matches!(
        connect_registry::validate_session(
            &repo,
            &relay,
            &token,
            Some(session.context.context_revision + 1)
        )
        .await,
        Err(RegistryError::RevisionMismatch)
    ));
    assert!(connect_registry::validate_session(
        &repo,
        &authority("scope-b", "web-b"),
        &token,
        None
    )
    .await
    .is_err());
    assert!(connect_registry::validate_session(
        &repo,
        &authority("scope-a", "different-relay"),
        &token,
        None
    )
    .await
    .is_err());
    connect_registry::revoke_session(&repo, &relay, &token)
        .await
        .expect("logout");
    connect_registry::revoke_session(&repo, &relay, &token)
        .await
        .expect("idempotent logout");
    assert!(matches!(
        connect_registry::validate_session(&repo, &relay, &token, None).await,
        Err(RegistryError::Revoked)
    ));
}

#[tokio::test]
async fn expiry_and_configuration_epoch_invalidate_old_proofs_without_domain_reset() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "connect_epoch").await;
    initialize(&repo, "synthetic-deployment-3").await;
    let native = authority("scope-a", "native-a");
    let relay = authority("scope-a", "web-a");
    let input = request(
        "ensure-a",
        "192.0.2.45",
        "research",
        "synthetic-install-a",
        "synthetic-folder-a",
    );
    let registration = connect_registry::ensure(&repo, &native, &input)
        .await
        .expect("native registration")
        .receipt;
    let session = connect_registry::create_session(&repo, &relay, &registration.connect_id)
        .await
        .expect("first session");
    repo.db()
        .query("UPDATE connect_session SET expires_at=time::now()-1s RETURN NONE")
        .await
        .expect("controlled expiry fixture")
        .check()
        .expect("expiry statement");
    assert!(matches!(
        connect_registry::validate_session(&repo, &relay, session.token.expose_secret(), None)
            .await,
        Err(RegistryError::Revoked)
    ));
    let session = connect_registry::create_session(&repo, &relay, &registration.connect_id)
        .await
        .expect("second session");
    repo.db().query("UPDATE connect_control:configuration SET configuration_epoch='synthetic-new-config-epoch' RETURN NONE").await.expect("explicit synthetic epoch rotation").check().expect("epoch statement");
    assert!(matches!(
        connect_registry::validate_session(&repo, &relay, session.token.expose_secret(), None)
            .await,
        Err(RegistryError::Revoked)
    ));
    let state = connect_registry::initialize_control(repo.db())
        .await
        .expect("repeated explicit bootstrap");
    assert_eq!(
        state.configuration_epoch, "synthetic-new-config-epoch",
        "bootstrap cannot resurrect an old epoch"
    );
    assert_eq!(
        connect_registry::ensure(&repo, &native, &input)
            .await
            .expect("domain registry retained")
            .receipt,
        registration
    );
}

#[tokio::test]
async fn rebind_keeps_node_and_folder_ids_revokes_old_sessions_and_retirement_stays_pending() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "connect_rebind").await;
    initialize(&repo, "synthetic-deployment-4").await;
    let native = authority("scope-a", "native-a");
    let relay = authority("scope-a", "web-a");
    let default = request(
        "default-a",
        "192.0.2.46",
        "default",
        "synthetic-install-a",
        "synthetic-default-a",
    );
    let child = request(
        "child-a",
        "192.0.2.46",
        "research",
        "synthetic-install-a",
        "synthetic-folder-a",
    );
    let default = connect_registry::ensure(&repo, &native, &default)
        .await
        .expect("protected default")
        .receipt;
    let child = connect_registry::ensure(&repo, &native, &child)
        .await
        .expect("ordinary child")
        .receipt;
    assert!(matches!(
        connect_registry::begin_retirement(
            &repo,
            &native,
            &default.registration_id,
            default.revision
        )
        .await,
        Err(RegistryError::ProtectedDefault)
    ));
    let old = connect_registry::create_session(&repo, &relay, &child.connect_id)
        .await
        .expect("old session");
    let changed = request(
        "rebind-a",
        "192.0.2.47",
        "renamed",
        "synthetic-install-a",
        "synthetic-folder-a",
    );
    let input = RebindInput::new(changed, child.registration_id.clone(), child.revision);
    let rebound = connect_registry::rebind(&repo, &native, &input)
        .await
        .expect("confirmed rebind")
        .receipt;
    assert_eq!(rebound.workstation_id, child.workstation_id);
    assert_eq!(rebound.workspace_id, child.workspace_id);
    assert_ne!(rebound.registration_id, child.registration_id);
    assert_eq!(rebound.connect_id.to_string(), "192.0.2.47:renamed");
    assert_eq!(
        child.connect_id.to_string(),
        "192.0.2.46:research",
        "past snapshots stay unchanged"
    );
    assert!(
        connect_registry::validate_session(&repo, &relay, old.token.expose_secret(), None)
            .await
            .is_err()
    );
    assert_eq!(
        connect_registry::rebind(&repo, &native, &input)
            .await
            .expect("rebind replay")
            .receipt,
        rebound
    );
    let default_new = "192.0.2.47:default".parse().expect("new default binding");
    assert_eq!(
        connect_registry::create_session(&repo, &relay, &default_new)
            .await
            .expect("default preserved across IP rebind")
            .context
            .workspace_id,
        default.workspace_id
    );
    let pending = connect_registry::begin_retirement(
        &repo,
        &native,
        &rebound.registration_id,
        rebound.revision,
    )
    .await
    .expect("retirement fence");
    assert_eq!(pending.workspace_id, child.workspace_id);
    let admission = tx::run_tx(repo.db(), tx::RetryPolicy::default(), |transaction| {
        let native = native.clone();
        let id = rebound.registration_id.clone();
        async move {
            connect_registry::validate_registration_in(&transaction, &native, &id, rebound.revision)
                .await
        }
    })
    .await;
    assert!(
        admission.is_err(),
        "retirement fences actual admission validation"
    );
    assert!(
        connect_registry::create_session(&repo, &relay, &rebound.connect_id)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn failed_resolution_is_durably_rate_limited_and_never_creates_a_registration() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "connect_rate").await;
    initialize(&repo, "synthetic-deployment-5").await;
    let relay = authority("scope-a", "web-a");
    let unknown = "192.0.2.48:unregistered"
        .parse()
        .expect("synthetic Connect ID");
    for _ in 0..20 {
        assert!(matches!(
            connect_registry::create_session(&repo, &relay, &unknown).await,
            Err(RegistryError::NotFound)
        ));
    }
    assert!(matches!(
        connect_registry::create_session(&repo, &relay, &unknown).await,
        Err(RegistryError::RateLimited)
    ));
    let mut window = repo
        .db()
        .query("SELECT VALUE attempts FROM connect_rate_window")
        .await
        .expect("rate window transport")
        .check()
        .expect("rate window statement");
    assert_eq!(
        window.take::<Vec<i64>>(0).expect("persisted attempts"),
        vec![20],
        "failed lookups consume the same durable bounded window"
    );
    repo.db()
        .query("UPDATE connect_rate_window SET started_at=time::now()-2m RETURN NONE")
        .await
        .expect("expire synthetic rate window")
        .check()
        .expect("expiration statement");
    assert!(matches!(
        connect_registry::create_session(&repo, &relay, &unknown).await,
        Err(RegistryError::NotFound)
    ));
    let mut window = repo
        .db()
        .query("SELECT VALUE attempts FROM connect_rate_window")
        .await
        .expect("renewed window transport")
        .check()
        .expect("renewed window statement");
    assert_eq!(
        window.take::<Vec<i64>>(0).expect("renewed attempts"),
        vec![1],
        "an expired window starts once rather than keeping the old count"
    );
    let mut count = repo
        .db()
        .query("SELECT VALUE registration_id FROM connect_registration")
        .await
        .expect("registration read")
        .check()
        .expect("read statement");
    assert!(count
        .take::<Vec<String>>(0)
        .expect("registration IDs")
        .is_empty());
}
