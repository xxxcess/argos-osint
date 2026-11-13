//! Real isolated registry/session persistence; no client marker grants authority.
#![cfg(feature = "surreal")]

mod support;

use argos_osint_core::storage::connect_registry;
use argos_osint_core::storage::connect_registry::RegistryAuthority;
use argos_osint_core::storage::connect_registry::RegistryError;
use argos_osint_core::storage::repo::Repository;
use fleetworks_contracts::connect::RegistrationEnsureRequest;

use support::Server;

fn authority(scope: &str, credential: &str) -> RegistryAuthority {
    RegistryAuthority::new(scope, credential, "synthetic-access-epoch")
        .expect("synthetic authority must be valid")
}

fn request(
    key: &str,
    ip: &str,
    _name: &str,
    install: &str,
    _folder: &str,
) -> RegistrationEnsureRequest {
    RegistrationEnsureRequest::new(
        key.into(),
        Some(ip.parse().expect("synthetic unicast IP")),
        install.into(),
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
async fn stable_identity_ignores_ip_and_isolates_installations_and_scopes() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "connect_ensure_v2").await;
    initialize(&repo, "synthetic-deployment-1").await;
    let owner = authority("scope-a", "native-a");
    let first = RegistrationEnsureRequest::new(
        "ensure-a".into(),
        Some("192.0.2.42".parse().expect("IP")),
        "synthetic-install-a".into(),
    );
    let mut concurrent = first.clone();
    concurrent.request_id = "ensure-b".into();
    let (one, two) = tokio::join!(
        connect_registry::ensure(&repo, &owner, &first),
        connect_registry::ensure(&repo, &owner, &concurrent)
    );
    let one = one.expect("first ensure");
    let two = two.expect("concurrent ensure");
    assert_eq!(one.receipt, two.receipt);
    assert_ne!(one.created, two.created);
    assert_eq!(one.receipt.connect_id.as_str(), one.receipt.workstation_id);
    let relay = authority("scope-a", "relay-a");
    let session = connect_registry::create_session(&repo, &relay, &one.receipt.connect_id)
        .await
        .expect("session");
    for (key, ip) in [
        ("changed-ip", Some("192.0.2.99".parse().expect("IP"))),
        ("no-ip", None),
    ] {
        let changed = connect_registry::ensure(
            &repo,
            &owner,
            &RegistrationEnsureRequest::new(key.into(), ip, first.installation_marker.clone()),
        )
        .await
        .expect("metadata ensure");
        assert_eq!(changed.receipt.workstation_id, one.receipt.workstation_id);
        assert_eq!(changed.receipt.registration_id, one.receipt.registration_id);
        assert_eq!(changed.receipt.workstation_ip, ip);
        connect_registry::validate_session(&repo, &relay, session.token.expose_secret(), None)
            .await
            .expect("IP metadata does not revoke identity");
    }
    let mut other = first.clone();
    other.request_id = "other-install".into();
    other.installation_marker = "synthetic-install-b".into();
    let other = connect_registry::ensure(&repo, &owner, &other)
        .await
        .expect("same IP separate installation");
    assert_ne!(other.receipt.workstation_id, one.receipt.workstation_id);
    let scoped = connect_registry::ensure(&repo, &authority("scope-b", "native-b"), &first)
        .await
        .expect("separate scope");
    assert_ne!(scoped.receipt.workstation_id, one.receipt.workstation_id);
    let mut conflicting = first.clone();
    conflicting.workstation_ip = None;
    assert!(matches!(
        connect_registry::ensure(&repo, &owner, &conflicting).await,
        Err(RegistryError::Collision)
    ));
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
    let unknown = "unregistered-node".parse().expect("synthetic Connect ID");
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
async fn forward_migration_preserves_legacy_mapping_and_rejects_legacy_writers() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "connect_migration").await;
    initialize(&repo, "migration-deployment").await;
    // Reconstruct version-one schema at its actual authority boundary.
    repo.db().query("DEFINE FIELD OVERWRITE connect_id ON connect_registration TYPE string; DEFINE FIELD OVERWRITE contract_version ON connect_registration TYPE option<int>;
        CREATE connect_workstation:legacy_node SET workstation_id='legacy_node',scope_id='scope-a',installation_hash='retained-installation-hash',workstation_ip='192.0.2.42',state='active',revision=7,last_seen_at=time::now();
        CREATE connect_registration:old_child SET registration_id='old_child',scope_id='scope-a',workstation_id='legacy_node',workspace_id='old_workspace',workspace_name='research',workstation_ip='192.0.2.42',connect_id='192.0.2.42:research',state='active',revision=3,created_at=time::now();
        UPDATE connect_control:configuration SET contract_version=1 RETURN NONE;")
        .await.expect("legacy fixture").check().expect("legacy fixture statements");
    argos_osint_core::storage::bootstrap::ensure_schema(repo.db())
        .await
        .expect("forward migration");
    let mut response=repo.db().query("SELECT VALUE state FROM ONLY connect_registration:old_child; SELECT VALUE workstation_id FROM ONLY connect_registration:old_child; SELECT VALUE installation_hash FROM ONLY connect_workstation:legacy_node;").await.expect("history read").check().expect("history statements");
    assert_eq!(
        response
            .take::<Option<String>>(0)
            .expect("state")
            .as_deref(),
        Some("legacy_read_only")
    );
    assert_eq!(
        response.take::<Option<String>>(1).expect("node").as_deref(),
        Some("legacy_node")
    );
    assert_eq!(
        response
            .take::<Option<String>>(2)
            .expect("marker")
            .as_deref(),
        Some("retained-installation-hash")
    );
    let relay = authority("scope-a", "relay-a");
    let id = "legacy_node".parse().expect("opaque ID");
    let session = connect_registry::create_session(&repo, &relay, &id)
        .await
        .expect("migrated session");
    assert_eq!(session.context.workstation_id, "legacy_node");
    argos_osint_core::storage::bootstrap::ensure_schema(repo.db())
        .await
        .expect("idempotent restart");
    connect_registry::validate_session(&repo, &relay, session.token.expose_secret(), None)
        .await
        .expect("checkpoint does not revoke v2 session");
    assert!(connect_registry::registration_context(
        &repo,
        &authority("scope-a", "native-a"),
        "old_child",
        3
    )
    .await
    .is_err());
    let old_writer=repo.db().query("CREATE connect_registration:old_writer SET registration_id='old_writer',scope_id='scope-a',workstation_id='legacy_node',workspace_id='other',workspace_name='other',workstation_ip='192.0.2.42',connect_id='192.0.2.42:other',state='active',revision=1,created_at=time::now() RETURN NONE").await.expect("old writer transport").check();
    assert!(
        old_writer.is_err(),
        "schema fences incompatible active writers"
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
    let unknown = "unregistered-rate-limit-node"
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

#[tokio::test]
async fn migration_preserves_inactive_admission_without_revoking_accepted_causal_work() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "connect_inactive_migration").await;
    initialize(&repo, "inactive-migration-deployment").await;
    for (node, state, installation) in [
        ("revoked_node", "revoked", "revoked-installation-marker"),
        ("retiring_node", "retiring", "retiring-installation-marker"),
    ] {
        use sha2::Digest;
        let mut digest = sha2::Sha256::new();
        digest.update((installation.len() as u64).to_be_bytes());
        digest.update(installation.as_bytes());
        let installation_hash = format!("{:x}", digest.finalize());
        repo.db().query("DEFINE FIELD OVERWRITE connect_id ON connect_registration TYPE string; DEFINE FIELD OVERWRITE contract_version ON connect_registration TYPE option<int>;
            CREATE $node_record SET workstation_id=$node,scope_id='scope-a',installation_hash=$installation,workstation_ip='192.0.2.42',state='active',revision=3,last_seen_at=time::now();
            CREATE $registration SET registration_id=$registration_id,scope_id='scope-a',workstation_id=$node,workspace_id='old-child',workspace_name='research',workstation_ip='192.0.2.42',connect_id='192.0.2.42:research',state=$state,revision=4,created_at=time::now();
            UPDATE connect_control:configuration SET contract_version=1 RETURN NONE;")
            .bind(("node_record",surrealdb::types::RecordId::new("connect_workstation",node)))
            .bind(("node",node)).bind(("installation",installation_hash))
            .bind(("registration",surrealdb::types::RecordId::new("connect_registration",format!("old-{node}"))))
            .bind(("registration_id",format!("old-{node}"))).bind(("state",state))
            .await.expect("old inactive records").check().expect("old schema statements");
    }
    // This root was genuinely accepted before its old registration was revoked.
    argos_osint_core::storage::jobs::create_job(
        repo.db(),
        &argos_osint_core::storage::jobs::NewJob {
            id: "inactive-accepted-root",
            kind: "task",
            operation: "fixture_operation",
            parent: None,
            created_at: chrono::Utc::now(),
        },
        &[],
    )
    .await
    .expect("historical root");
    let old_origin = serde_json::json!({"deployment_id":"inactive-migration-deployment","registration_id":"old-revoked_node","workstation_id":"revoked_node","workspace_id":"old-child","connect_id_at_submission":"192.0.2.42:research","workstation_ip_at_submission":"192.0.2.42","workspace_name_at_submission":"research","source":"native","request_id":"old-accepted-request","submitted_at":"2026-11-01T00:00:00Z","configuration_epoch":"old-configuration","configuration_revision":1,"credential_references":[]});
    let old_json = serde_json::to_string(&old_origin).expect("legacy JSON");
    repo.db().query("CREATE $record SET job_id='inactive-accepted-root',root_id='inactive-accepted-root',scope_id='scope-a',deployment_id='inactive-migration-deployment',workstation_id='revoked_node',workspace_id='old-child',origin_json=$origin RETURN NONE")
        .bind(("record",surrealdb::types::RecordId::new("job_origin","inactive-accepted-root"))).bind(("origin",old_json.clone()))
        .await.expect("immutable accepted origin").check().expect("origin statement");
    // A second genuinely accepted historical tree belongs to the retiring
    // installation, with durable requester membership used by retirement drain.
    argos_osint_core::storage::jobs::create_job(
        repo.db(),
        &argos_osint_core::storage::jobs::NewJob {
            id: "retiring-accepted-root",
            kind: "task",
            operation: "fixture_operation",
            parent: None,
            created_at: chrono::Utc::now(),
        },
        &[],
    )
    .await
    .expect("retiring installation's historical root");
    let mut retiring_origin = old_origin.clone();
    retiring_origin["registration_id"] = "old-retiring_node".into();
    retiring_origin["workstation_id"] = "retiring_node".into();
    retiring_origin["request_id"] = "retiring-accepted-request".into();
    repo.db().query("CREATE $record SET job_id='retiring-accepted-root',root_id='retiring-accepted-root',scope_id='scope-a',deployment_id='inactive-migration-deployment',workstation_id='retiring_node',workspace_id='old-child',origin_json=$origin RETURN NONE;
        CREATE job_request:retiring_accepted SET request_key='retiring-accepted-request',operation='fixture_operation',job_id='retiring-accepted-root',scope_id='scope-a',deployment_id='inactive-migration-deployment',registration_id='old-retiring_node',workstation_id='retiring_node',workspace_id='old-child',requested_at='2026-11-01T00:00:00Z',origin_json=$origin RETURN NONE;")
        .bind(("record",surrealdb::types::RecordId::new("job_origin","retiring-accepted-root")))
        .bind(("origin",serde_json::to_string(&retiring_origin).expect("historical retiring origin")))
        .await.expect("retiring origin and requester association").check().expect("retiring attribution statements");
    argos_osint_core::storage::bootstrap::ensure_schema(repo.db())
        .await
        .expect("migration");
    for (node, state, installation) in [
        ("revoked_node", "revoked", "revoked-installation-marker"),
        ("retiring_node", "retiring", "retiring-installation-marker"),
    ] {
        let registration = format!("workstation-{node}");
        let mut result = repo
            .db()
            .query(
                "SELECT VALUE state FROM ONLY $registration; SELECT VALUE state FROM ONLY $node;",
            )
            .bind((
                "registration",
                surrealdb::types::RecordId::new("connect_registration", registration.clone()),
            ))
            .bind((
                "node",
                surrealdb::types::RecordId::new("connect_workstation", node),
            ))
            .await
            .expect("state read")
            .check()
            .expect("state statements");
        assert_eq!(
            result
                .take::<Option<String>>(0)
                .expect("registration state")
                .as_deref(),
            Some(state)
        );
        assert_eq!(
            result
                .take::<Option<String>>(1)
                .expect("node state")
                .as_deref(),
            Some("active")
        );
        assert!(
            connect_registry::ensure(
                &repo,
                &authority("scope-a", "native-a"),
                &RegistrationEnsureRequest::new(format!("retry-{node}"), None, installation.into())
            )
            .await
            .is_err(),
            "ensure must not revive {state} registration"
        );
        assert!(connect_registry::registration_context(
            &repo,
            &authority("scope-a", "native-a"),
            &registration,
            1
        )
        .await
        .is_err());
        assert!(connect_registry::create_session(
            &repo,
            &authority("scope-a", "relay-a"),
            &node.parse().expect("opaque ID")
        )
        .await
        .is_err());
    }
    argos_osint_core::storage::jobs::create_job(
        repo.db(),
        &argos_osint_core::storage::jobs::NewJob {
            id: "inactive-accepted-child",
            kind: "task",
            operation: "fixture_child",
            parent: Some("inactive-accepted-root"),
            created_at: chrono::Utc::now(),
        },
        &[],
    )
    .await
    .expect("accepted historical chain continues under active node fence");
    let child =
        argos_osint_core::storage::job_context::origin_for_job(&repo, "inactive-accepted-child")
            .await
            .expect("child origin read")
            .expect("retained origin");
    assert_eq!(
        serde_json::to_value(child).expect("child snapshot"),
        old_origin
    );
    let mut unchanged = repo
        .db()
        .query("SELECT VALUE origin_json FROM ONLY $record")
        .bind((
            "record",
            surrealdb::types::RecordId::new("job_origin", "inactive-accepted-root"),
        ))
        .await
        .expect("old bytes read")
        .check()
        .expect("old bytes statement");
    assert_eq!(
        unchanged.take::<Option<String>>(0).expect("old bytes"),
        Some(old_json)
    );

    // A genuine v1 retirement migrates to an inactive v2 registration while
    // its workstation stays active for accepted causal work. Resume that fence
    // through public retirement operations rather than editing v2 state.
    let registration = "workstation-retiring_node";
    let native = authority("scope-a", "native-a");
    assert!(
        connect_registry::begin_retirement(&repo, &native, registration, 2)
            .await
            .is_err(),
        "stale registration revision cannot fence the migrated owner"
    );
    let mut owner = repo
        .db()
        .query("SELECT VALUE state FROM ONLY connect_workstation:retiring_node; SELECT VALUE revision FROM ONLY connect_workstation:retiring_node;")
        .await
        .expect("owner after rejected stale revision")
        .check()
        .expect("owner snapshot statements");
    assert_eq!(
        owner.take::<Option<String>>(0).expect("state").as_deref(),
        Some("active")
    );
    assert_eq!(owner.take::<Option<i64>>(1).expect("revision"), Some(3));

    let pending = connect_registry::begin_retirement(&repo, &native, registration, 1)
        .await
        .expect("resume migrated retirement with its actual revision");
    assert_eq!(pending.revision, 1);
    let mut owner = repo
        .db()
        .query("SELECT VALUE state FROM ONLY connect_workstation:retiring_node; SELECT VALUE revision FROM ONLY connect_workstation:retiring_node;")
        .await
        .expect("synchronized workstation fence")
        .check()
        .expect("workstation fence statements");
    assert_eq!(
        owner.take::<Option<String>>(0).expect("state").as_deref(),
        Some("retiring")
    );
    assert_eq!(owner.take::<Option<i64>>(1).expect("revision"), Some(4));
    argos_osint_core::storage::jobs::create_job(
        repo.db(),
        &argos_osint_core::storage::jobs::NewJob {
            id: "retiring-accepted-child",
            kind: "task",
            operation: "fixture_child",
            parent: Some("retiring-accepted-root"),
            created_at: chrono::Utc::now(),
        },
        &[],
    )
    .await
    .expect("same installation's accepted causal work continues after retirement begins");
    let child =
        argos_osint_core::storage::job_context::origin_for_job(&repo, "retiring-accepted-child")
            .await
            .expect("retiring child origin read")
            .expect("retiring child retains its accepted context");
    assert_eq!(
        serde_json::to_value(child).expect("retiring child snapshot"),
        retiring_origin
    );
    assert!(matches!(
        connect_registry::finish_retirement(&repo, &native, registration, pending.revision)
            .await
            .expect("same installation's accepted work keeps retirement pending"),
        connect_registry::RetirementResult::Pending(_)
    ));
    assert!(
        argos_osint_core::storage::jobs::stop_job(
            repo.db(),
            "retiring-accepted-root",
            chrono::Utc::now(),
            argos_osint_core::storage::jobs::STOP_BATCH,
            None,
        )
        .await
        .expect("settle the actual accepted root and causal child")
        .done
    );
    let completed =
        connect_registry::finish_retirement(&repo, &native, registration, pending.revision)
            .await
            .expect("migrated retirement completes with no associated active work");
    assert!(matches!(
        &completed,
        connect_registry::RetirementResult::Complete(_)
    ));
    let replay = connect_registry::begin_retirement(&repo, &native, registration, 1)
        .await
        .expect("terminal migrated retirement replay");
    assert_eq!(
        connect_registry::finish_retirement(&repo, &native, registration, replay.revision)
            .await
            .expect("same terminal retirement receipt"),
        completed
    );
}
