//! Public ingress, durable issuance and real workstation authorization boundaries.
use std::collections::BTreeSet;
use std::sync::Arc;

use argos_osint_core::storage::bootstrap;
use argos_osint_core::storage::connect_registry;
use argos_osint_core::storage::connect_registry::RegistryAuthority;
#[cfg(feature = "embedded")]
use argos_osint_core::storage::repo::Repository;
use axum::body::Body;
use axum::http::Request;
use axum::http::StatusCode;
use axum::Router;
use fleetworks_contracts::connect::RegistrationEnsureRequest;
use fleetworks_contracts::context::CredentialPurpose;
use fleetworks_contracts::context::Permission;
use fleetworks_contracts::pairing::PairingStartRequest;
use serde_json::json;
use serde_json::Value;
use tower::ServiceExt;

use crate::ApiState;
use crate::NetworkPolicy;
use crate::ScopedCredential;

const HOST: &str = "pairing.test:8080";
const OPERATOR: &str = "synthetic-pairing-operator-0123456789abcdef";
const NATIVE: &str = "synthetic-static-native-0123456789abcdef";
const OTHER: &str = "synthetic-other-operator-0123456789abcdef";
fn policy(reduced: bool) -> NetworkPolicy {
    let grants = BTreeSet::from([
        Permission::ReadBackend,
        Permission::RegisterWorkstation,
        Permission::ManageConfiguration,
        Permission::ReadWorkstationActivity,
        Permission::SubmitInteractive,
    ]);
    NetworkPolicy::with_credentials(
        &[HOST.into()],
        vec![
            ScopedCredential::new(
                "operator".into(),
                "scope-a".into(),
                "epoch-a".into(),
                CredentialPurpose::Operator,
                if reduced {
                    BTreeSet::from([Permission::ReadBackend])
                } else {
                    grants.clone()
                },
                OPERATOR,
            )
            .expect("operator"),
            ScopedCredential::new(
                "native".into(),
                "scope-a".into(),
                "epoch-a".into(),
                CredentialPurpose::Native,
                grants.clone(),
                NATIVE,
            )
            .expect("native"),
            ScopedCredential::new(
                "other".into(),
                "scope-other".into(),
                "epoch-other".into(),
                CredentialPurpose::Operator,
                grants,
                OTHER,
            )
            .expect("other scope operator"),
        ],
    )
    .expect("current explicit policy")
}
async fn request(
    app: &Router,
    path: &str,
    token: Option<&str>,
    body: Value,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method("POST")
        .uri(path)
        .header("host", HOST)
        .header("content-type", "application/json");
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    let response = app
        .clone()
        .oneshot(
            request
                .body(Body::from(
                    serde_json::to_vec(&body).expect("synthetic body"),
                ))
                .expect("request"),
        )
        .await
        .expect("response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 16 * 1024)
        .await
        .expect("bounded response");
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
fn start_body(id: char) -> Value {
    json!({"request_id":id.to_string().repeat(32),"installation_marker":format!("installation-{}",id.to_string().repeat(32)),"polling_secret":id.to_string().repeat(64)})
}
fn decision(challenge: &Value) -> Value {
    json!({"pairing_id":challenge["pairing_id"],"approval_code":challenge["approval_code"]})
}
fn poll_body(id: char) -> Value {
    json!({"pairing_id":id.to_string().repeat(32),"polling_secret":id.to_string().repeat(64)})
}

struct ConfigurationEnvironment(Vec<(&'static str, Option<std::ffi::OsString>)>);
impl ConfigurationEnvironment {
    fn install(root: &std::path::Path) -> Self {
        Self(
            ["ARGOS_HOME", "FLEETWORKS_WORKSPACE"]
                .into_iter()
                .map(|key| {
                    let old = std::env::var_os(key);
                    std::env::set_var(key, root);
                    (key, old)
                })
                .collect(),
        )
    }
}
impl Drop for ConfigurationEnvironment {
    fn drop(&mut self) {
        for (key, old) in &self.0 {
            if let Some(old) = old {
                std::env::set_var(key, old);
            } else {
                std::env::remove_var(key);
            }
        }
    }
}

#[tokio::test]
async fn public_pairing_is_durable_scoped_single_issuance_and_installation_bound() {
    let _environment_guard = crate::CONFIGURATION_TEST_ENVIRONMENT.lock().await;
    let config_directory = tempfile::tempdir().expect("isolated configuration owner");
    let config_root = config_directory
        .path()
        .canonicalize()
        .expect("canonical owner");
    let _environment = ConfigurationEnvironment::install(&config_root);
    #[cfg(feature = "embedded")]
    let directory = tempfile::tempdir().expect("isolated durable store");
    #[cfg(feature = "embedded")]
    let repo = Repository::connect(
        &argos_osint_core::storage::open::Target::Embedded {
            path: directory.path().join("store"),
        },
        "pairing",
        "lifecycle",
    )
    .await
    .expect("isolated repository");
    #[cfg(not(feature = "embedded"))]
    let datastore =
        crate::storage_test_support::Server::start().expect("required isolated pinned server");
    #[cfg(not(feature = "embedded"))]
    let repo = datastore.repo("pairing", "lifecycle").await;
    bootstrap::ensure_network_identity(repo.db())
        .await
        .expect("identity");
    connect_registry::initialize_control(repo.db())
        .await
        .expect("control");
    let state = ApiState {
        repo: repo.clone(),
        instance_id: "pairing-instance".into(),
        port: 8080,
        origins: Arc::new(vec!["https://native.example".into()]),
    };
    let app = crate::network_router(state.clone(), policy(false));
    let start = "/api/v1/pairing/start";
    let poll = "/api/v1/pairing/poll";
    let approve = "/api/v1/admin/pairing/approve";
    let ensure = "/api/v1/connect/registrations/ensure";
    // Public exemptions are exact POST paths after ingress checks, never a
    // prefix exception, query escape, or access to another registered API.
    for (path, method, host, origin, expected) in [
        (start, "POST", "evil.example", None, StatusCode::FORBIDDEN),
        (
            start,
            "POST",
            HOST,
            Some("https://evil.example"),
            StatusCode::FORBIDDEN,
        ),
        (
            "/api/v1/pairing/start/extra",
            "POST",
            HOST,
            None,
            StatusCode::UNAUTHORIZED,
        ),
        (
            "/api/v1/pairing/start?bypass=1",
            "POST",
            HOST,
            None,
            StatusCode::UNAUTHORIZED,
        ),
        (start, "GET", HOST, None, StatusCode::UNAUTHORIZED),
        (approve, "POST", HOST, None, StatusCode::UNAUTHORIZED),
        (ensure, "POST", HOST, None, StatusCode::UNAUTHORIZED),
    ] {
        let mut req = Request::builder()
            .method(method)
            .uri(path)
            .header("host", host)
            .header("content-type", "application/json");
        if let Some(origin) = origin {
            req = req.header("origin", origin);
        }
        let response = app
            .clone()
            .oneshot(
                req.body(Body::from(start_body('a').to_string()))
                    .expect("ingress fixture"),
            )
            .await
            .expect("ingress response");
        assert_eq!(
            response.status(),
            expected,
            "{method} {path} must keep ingress/authority fences"
        );
    }
    let (status, challenge) = request(&app, start, None, start_body('a')).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(challenge["pairing_id"], "a".repeat(32));
    let (status, replay) = request(&app, start, None, start_body('a')).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(replay, challenge);
    let mut changed = start_body('a');
    changed["installation_marker"] = json!("other-installation-marker");
    assert_eq!(
        request(&app, start, None, changed).await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        request(&app, poll, None, poll_body('a')).await.1["status"],
        "pending"
    );
    let mut wrong = poll_body('a');
    wrong["polling_secret"] = json!("f".repeat(64));
    assert_eq!(
        request(&app, poll, None, wrong).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(&app, approve, Some(NATIVE), decision(&challenge))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let mut escalated = decision(&challenge);
    escalated["scope_id"] = json!("scope-other");
    assert_eq!(
        request(&app, approve, Some(OPERATOR), escalated).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        request(&app, approve, Some(OPERATOR), decision(&challenge))
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        request(&app, approve, Some(OPERATOR), decision(&challenge))
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        request(&app, approve, Some(OTHER), decision(&challenge))
            .await
            .0,
        StatusCode::CONFLICT
    );
    let (status, issued) = request(&app, poll, None, poll_body('a')).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(issued["status"], "approved");
    assert_eq!(issued["scope_id"], "scope-a");
    assert_eq!(issued["access_epoch"], "epoch-a");
    let token = issued["access_token"]
        .as_str()
        .expect("secret returned only to proof holder");
    let credential = issued["credential_id"]
        .as_str()
        .expect("non-secret credential ID");
    // Reconstructing the router/policy has no in-memory issuance cache.
    let restarted = crate::network_router(state.clone(), policy(false));
    assert_eq!(
        request(&restarted, poll, None, poll_body('a')).await.1,
        issued
    );
    let ensure_body = json!({"request_id":"ensure-paired-installation","installation_marker":start_body('a')["installation_marker"]});
    let (status, registration) =
        request(&restarted, ensure, Some(token), ensure_body.clone()).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(
        request(&restarted, ensure, Some(token), ensure_body)
            .await
            .1,
        registration
    );
    let registration_id = registration["registration_id"]
        .as_str()
        .expect("real registration");
    assert_eq!(
        request(
            &restarted,
            "/api/v1/connect/registrations/context",
            Some(token),
            json!({"registration_id":registration_id,"expected_revision":registration["revision"]})
        )
        .await
        .0,
        StatusCode::OK
    );
    let authority =
        RegistryAuthority::new("scope-a", credential, "epoch-a").expect("paired authority");
    let context = connect_registry::registration_context(
        &repo,
        &authority,
        registration_id,
        registration["revision"].as_u64().expect("revision"),
    )
    .await
    .expect("normal registered API path");
    assert_eq!(
        context.workstation_id,
        registration["workstation_id"]
            .as_str()
            .expect("stable identity")
    );
    let other = connect_registry::ensure(
        &repo,
        &RegistryAuthority::new("scope-a", "native", "epoch-a").expect("static authority"),
        &RegistrationEnsureRequest::new(
            "other-ensure".into(),
            None,
            "other-installation-marker".into(),
        ),
    )
    .await
    .expect("different installation")
    .receipt;
    assert!(
        connect_registry::registration_context(
            &repo,
            &authority,
            &other.registration_id,
            other.revision
        )
        .await
        .is_err(),
        "same-scope paired access cannot claim another installation's registration"
    );
    assert_eq!(request(&restarted,ensure,Some(token),json!({"request_id":"evil-ensure-installation","installation_marker":"other-installation-marker"})).await.0,StatusCode::UNAUTHORIZED);
    // Copying an issued Native secret into a manifest must retain its original
    // purpose/binding, and must not turn it into an Operator credential.
    let relabelled_policy = NetworkPolicy::with_credentials(
        &[HOST.into()],
        vec![
            ScopedCredential::new(
                "operator".into(),
                "scope-a".into(),
                "epoch-a".into(),
                CredentialPurpose::Operator,
                BTreeSet::from([
                    Permission::ReadBackend,
                    Permission::RegisterWorkstation,
                    Permission::ManageConfiguration,
                    Permission::ReadWorkstationActivity,
                    Permission::SubmitInteractive,
                ]),
                OPERATOR,
            )
            .expect("issuer"),
            ScopedCredential::new(
                "relabeled".into(),
                "scope-a".into(),
                "epoch-a".into(),
                CredentialPurpose::Operator,
                BTreeSet::from([
                    Permission::ReadBackend,
                    Permission::RegisterWorkstation,
                    Permission::ManageConfiguration,
                ]),
                token,
            )
            .expect("copied token fixture"),
        ],
    )
    .expect("distinct tokens");
    let relabelled = crate::network_router(state.clone(), relabelled_policy);
    assert_eq!(
        request(&relabelled, approve, Some(token), decision(&challenge))
            .await
            .0,
        StatusCode::FORBIDDEN,
        "issued Native token cannot be relabelled Operator"
    );
    assert!(
        argos_osint_core::storage::native_pairing::validate_authority(
            &repo,
            "scope-a",
            credential,
            "epoch-a",
            &BTreeSet::from([Permission::ResolveWebSession])
        )
        .await
        .is_err(),
        "private delegation cannot add grants absent from the issued Native authority"
    );
    let reduced = crate::network_router(state.clone(), policy(true));
    assert_eq!(request(&reduced,ensure,Some(token),json!({"request_id":"reduced-ensure","installation_marker":start_body('a')["installation_marker"]})).await.0,StatusCode::UNAUTHORIZED);
    assert_eq!(
        request(&reduced, poll, None, poll_body('a')).await.0,
        StatusCode::FORBIDDEN
    );
    let paired_principal = policy(false)
        .native_principal(&repo, token)
        .await
        .expect("current dynamic policy")
        .expect("active paired principal");
    let stale_global = crate::request_context::resolve_gateway(
        &repo,
        &paired_principal,
        &axum::http::HeaderMap::new(),
        Permission::ManageConfiguration,
        crate::request_context::ContextRequirement::GlobalRead,
    )
    .await
    .expect("global configuration authority without registration proof");
    assert!(matches!(
        stale_global.proof(),
        crate::request_context::AdmissionProof::None
    ));
    stale_global
        .revalidate(&repo)
        .await
        .expect("active global authority");
    let initial_configuration = {
        let lock = argos_osint_core::config_commit::ConfigLock::acquire_at(&config_root)
            .expect("real configuration barrier");
        crate::configuration::read_locked(&config_root, &lock)
            .map_err(|error| error.0)
            .expect("fresh current configuration")
            .revision
    };
    let revoke = "/api/v1/admin/pairing/revoke";
    assert_eq!(
        request(
            &restarted,
            revoke,
            Some(OTHER),
            json!({"credential_id":credential})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let barrier = argos_osint_core::config_commit::ConfigLock::acquire_at(&config_root)
        .expect("hold real configuration lock before revoke");
    let mut revoking = Box::pin(request(
        &restarted,
        revoke,
        Some(OPERATOR),
        json!({"credential_id":credential}),
    ));
    assert!(
        futures_util::poll!(revoking.as_mut()).is_pending(),
        "revocation cannot complete while the configuration owner is held"
    );
    stale_global
        .revalidate(&repo)
        .await
        .expect("revocation waiting behind lock has not fenced yet");
    drop(barrier);
    assert_eq!(revoking.await.0, StatusCode::OK);
    assert!(
        stale_global.revalidate(&repo).await.is_err(),
        "proof-None global authority must revalidate dynamic revocation"
    );
    let stale_role = serde_json::from_value::<crate::configuration::SetRole>(json!({"expected_revision":initial_configuration,"role":"chat","primary":{"provider":"local","model":"synthetic-model","account":"local"},"fallbacks":[]})).expect("configuration request DTO");
    assert!(
        matches!(
            crate::configuration::save_role(
                axum::extract::State(state.clone()),
                Some(axum::Extension(stale_global)),
                axum::Json(stale_role)
            )
            .await,
            Err(crate::ApiError(StatusCode::UNAUTHORIZED, _))
        ),
        "a previously resolved writer must recheck after acquiring ConfigLock"
    );
    let lock = argos_osint_core::config_commit::ConfigLock::acquire_at(&config_root)
        .expect("inspect unchanged configuration");
    assert_eq!(
        crate::configuration::read_locked(&config_root, &lock)
            .map_err(|error| error.0)
            .expect("unchanged configuration")
            .revision,
        initial_configuration
    );
    assert!(
        !config_root.join("config.toml").exists(),
        "revoked save cannot create configuration bytes"
    );
    drop(lock);
    assert_eq!(
        request(
            &restarted,
            revoke,
            Some(OPERATOR),
            json!({"credential_id":credential})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert!(
        connect_registry::registration_context(
            &repo,
            &authority,
            registration_id,
            registration["revision"].as_u64().expect("revision")
        )
        .await
        .is_err(),
        "core revalidation must observe revoke without trusting middleware cache"
    );
    assert_eq!(
        request(&restarted, poll, None, poll_body('a')).await.1["status"],
        "denied"
    );
    assert!(
        argos_osint_core::storage::native_pairing::validate_authority(
            &repo,
            "scope-a",
            credential,
            "epoch-a",
            &BTreeSet::from([Permission::ReadBackend])
        )
        .await
        .is_err(),
        "read-only global revalidation must observe credential revocation"
    );
    assert_eq!(request(&restarted,ensure,Some(token),json!({"request_id":"revoked-ensure","installation_marker":start_body('a')["installation_marker"]})).await.0,StatusCode::UNAUTHORIZED);
    assert_eq!(request(&relabelled, ensure, Some(token), json!({"request_id":"relabeled-revoked-ensure","installation_marker":start_body('a')["installation_marker"]})).await.0, StatusCode::UNAUTHORIZED, "retained token digest prevents relabelling revoked access");
    let (_, denied) = request(&restarted, start, None, start_body('b')).await;
    assert_eq!(
        request(
            &restarted,
            "/api/v1/admin/pairing/deny",
            Some(OPERATOR),
            decision(&denied)
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        request(&restarted, approve, Some(OPERATOR), decision(&denied))
            .await
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        request(&restarted, poll, None, poll_body('b')).await.1["status"],
        "denied"
    );
    let (_, expired) = request(&restarted, start, None, start_body('c')).await;
    repo.db()
        .query("UPDATE native_pairing SET expires_at_ms=0 WHERE pairing_id=$id")
        .bind(("id", "c".repeat(32)))
        .await
        .expect("controlled expiry")
        .check()
        .expect("expiry mutation");
    assert_eq!(
        request(&restarted, poll, None, poll_body('c')).await.1["status"],
        "expired"
    );
    assert_eq!(
        request(&restarted, approve, Some(OPERATOR), decision(&expired))
            .await
            .0,
        StatusCode::GONE
    );
    // Persisted state can contain public metadata/hashes, never either bearer.
    let mut rows = repo
        .db()
        .query("SELECT * FROM native_pairing")
        .await
        .expect("audit rows")
        .check()
        .expect("audit query");
    let rows: Vec<Value> = rows.take(0).expect("audit JSON rows");
    assert!(
        rows.iter()
            .any(|row| row["credential_id"].as_str() == Some(credential)),
        "secret-storage audit must include the issued credential"
    );
    let text = serde_json::to_string(&rows).expect("serialized audit rows");
    assert!(!text.contains(token));
    assert!(!text.contains(&"a".repeat(64)));
    assert!(!text.contains(OPERATOR));
    // Global pending/rate checks are durable and bounded; exact small fixtures
    // prove refusal without flooding requests or depending on sleeps.
    repo.db()
        .query("UPSERT native_pairing_rate:start SET started_at_ms=$now,attempts=20")
        .bind(("now", chrono::Utc::now().timestamp_millis()))
        .await
        .expect("controlled rate")
        .check()
        .expect("rate mutation");
    assert_eq!(
        request(&restarted, start, None, start_body('d')).await.0,
        StatusCode::TOO_MANY_REQUESTS
    );
    // Reset only the isolated rate fixture so the pending cap must independently
    // reject an otherwise valid start. These rows have no authority or tokens.
    repo.db().query("UPDATE native_pairing_rate:start SET attempts=0; FOR $n IN 0..100 { CREATE native_pairing SET pairing_id='synthetic-pending',installation_hash='fixture',proof_hash='fixture',approval_code='fixture',expires_at_ms=$now+600000,state='pending',credential_id='',issuance_nonce='',issuer_id='',scope_id='',access_epoch='',permissions_json='[]',token_hash=''; };").bind(("now",chrono::Utc::now().timestamp_millis())).await.expect("controlled bounded pending fixtures").check().expect("pending mutation");
    assert_eq!(
        request(&restarted, start, None, start_body('e')).await.0,
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[test]
fn public_requests_reject_authority_fields_before_storage() {
    let mut body = start_body('e');
    body["permissions"] = json!(["manage_configuration"]);
    assert!(serde_json::from_value::<PairingStartRequest>(body).is_err());
}

#[tokio::test]
async fn issued_pairing_reopens_from_disk_without_secret_storage() {
    // This is the same mode-selecting persisted harness as storage contracts.
    // The required embedded gate sets REQUIRED=1; ordinary unset runs may skip.
    let Some(mut datastore) = crate::storage_test_support::Server::start() else {
        return;
    };
    let repo = datastore.repo("pairing", "restart").await;
    let input = PairingStartRequest {
        request_id: "restart-pairing-request".into(),
        installation_marker: "restart-installation-marker".into(),
        polling_secret: "e".repeat(64),
    };
    let challenge = argos_osint_core::storage::native_pairing::start(&repo, &input)
        .await
        .expect("durable start");
    let authority = argos_osint_core::storage::native_pairing::OperatorAuthority {
        credential_id: "operator".into(),
        scope_id: "scope-a".into(),
        access_epoch: "epoch-a".into(),
        permissions: BTreeSet::from([
            Permission::ReadBackend,
            Permission::RegisterWorkstation,
            Permission::ManageConfiguration,
        ]),
    };
    argos_osint_core::storage::native_pairing::decide(
        &repo,
        &authority,
        &fleetworks_contracts::pairing::PairingDecisionRequest {
            pairing_id: challenge.pairing_id,
            approval_code: challenge.approval_code,
        },
        true,
    )
    .await
    .expect("approval persisted before restart");
    let poll = fleetworks_contracts::pairing::PairingPollRequest {
        pairing_id: input.request_id,
        polling_secret: input.polling_secret,
    };
    let original = argos_osint_core::storage::native_pairing::poll(&repo, &poll)
        .await
        .expect("first issuance");
    drop(repo);
    datastore.restart();
    let reopened = datastore.repo("pairing", "restart").await;
    let recovered = argos_osint_core::storage::native_pairing::poll(&reopened, &poll)
        .await
        .expect("exact issuance recovery after persisted restart");
    assert!(
        original == recovered,
        "restart must return the same single issuance, not mint a replacement"
    );
    let fleetworks_contracts::pairing::PairingStatus::Approved {
        access_token,
        credential_id,
        ..
    } = recovered
    else {
        panic!("approved credential must survive reopen");
    };
    assert!(
        argos_osint_core::storage::native_pairing::authenticate(&reopened, &access_token)
            .await
            .expect("current issued authority")
            .is_some()
    );
    // Model the existing durable reset scope fence with controlled metadata;
    // dynamic credentials must obey it even though they are not manifest rows.
    let scope_key = {
        use sha2::Digest;
        format!("{:x}", sha2::Sha256::digest(b"7:scope-a5:scope"))
    };
    reopened.db().query("UPSERT $row SET scope_id='scope-a',active_reset='',access_epoch='epoch-next',first_use_blocked=false,revoked_json='[]'").bind(("row",surrealdb::types::RecordId::new("configuration_reset_scope",scope_key.clone()))).await.expect("controlled epoch change").check().expect("scope fence mutation");
    assert!(
        argos_osint_core::storage::native_pairing::authenticate(&reopened, &access_token)
            .await
            .expect("epoch fenced authentication")
            .is_none()
    );
    reopened
        .db()
        .query("DELETE $row")
        .bind((
            "row",
            surrealdb::types::RecordId::new("configuration_reset_scope", scope_key),
        ))
        .await
        .expect("restore isolated scope")
        .check()
        .expect("scope cleanup");
    let config_directory = tempfile::tempdir().expect("isolated restart configuration owner");
    let lock = argos_osint_core::config_commit::ConfigLock::acquire_at(config_directory.path())
        .expect("configuration lock precedes registry guard");
    argos_osint_core::storage::native_pairing::revoke(&reopened, &authority, &credential_id, &lock)
        .await
        .expect("durable revoke");
    drop(reopened);
    datastore.restart();
    let reopened = datastore.repo("pairing", "restart").await;
    assert!(
        argos_osint_core::storage::native_pairing::authenticate(&reopened, &access_token)
            .await
            .expect("reopened revocation")
            .is_none()
    );
    assert!(
        argos_osint_core::storage::native_pairing::is_issued_token(&reopened, &access_token)
            .await
            .expect("retained revocation digest")
    );
}
