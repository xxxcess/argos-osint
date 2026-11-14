//! Real disposable owner restart/clear handshake; no production reset gate.

use std::collections::BTreeSet;
use std::fs;
use std::os::unix::fs::PermissionsExt;

use argos_osint_core::config_commit;
use argos_osint_core::provider::ModelAssignment;
use argos_osint_core::provider::SettingsFile;
use argos_osint_core::storage::bootstrap;
use argos_osint_core::storage::connect_registry;
use argos_osint_core::storage::open::Target;
use axum::body::to_bytes;
use axum::body::Body;
use axum::http::Request;
use axum::Router;
use serde_json::json;
use serde_json::Value;
use tower::ServiceExt;

use super::*;
use crate::guard_network;
use crate::ScopedCredential;

#[tokio::test]
async fn restart_during_partial_file_clear_stays_observable_and_requires_actual_clear_before_ack() {
    let temporary = tempfile::tempdir().expect("temporary private owner");
    let root = temporary
        .path()
        .canonicalize()
        .expect("canonical fixture root");
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).expect("private owner");
    config_commit::write_secure(&root.join("config.toml"), "configuration_for")
        .expect("synthetic interrupted prefix");
    let repo = Repository::connect(
        &Target::Embedded {
            path: root.join("store"),
        },
        "fixture",
        "reset_owner",
    )
    .await
    .expect("temporary datastore");
    bootstrap::ensure_network_identity(repo.db())
        .await
        .expect("real bootstrap identity");
    connect_registry::initialize_control(repo.db())
        .await
        .expect("real control bootstrap");
    repo.db().query("UPDATE connect_control:configuration SET configuration_epoch='new-configuration',admission_open=false RETURN NONE")
        .await.expect("synthetic durable reset fence").check().expect("fence statement");
    // Match the coordinator's stable scope key without inventing an authority.
    use sha2::Digest;
    let scope_key = format!("{:x}", sha2::Sha256::digest(b"11:owner-scope5:scope"));
    repo.db().query("CREATE $record CONTENT {scope_id:'owner-scope',active_reset:'fixture-reset',access_epoch:'new-access',first_use_blocked:true,revoked_json:'[]'}")
        .bind(("record",surrealdb::types::RecordId::new("configuration_reset_scope",scope_key)))
        .await.expect("keyed scope").check().expect("keyed scope statement");
    let token = "fixture-real-owner-service-token-0123456789abcdef";
    let credential = ScopedCredential::new(
        "owner-service".into(),
        "owner-scope".into(),
        "old-service-access".into(),
        CredentialPurpose::InternalService,
        BTreeSet::from([Permission::MaintainBackend, Permission::ManageConfiguration]),
        token,
    )
    .expect("real private policy");
    let policy = NetworkPolicy::with_credentials(&["owner.test:8080".into()], vec![credential])
        .expect("loaded policy");
    let owner = Arc::new(
        ResetOwner::new(
            repo,
            policy.clone(),
            "fixture-tools".into(),
            "fixture-process".into(),
            root.clone(),
        )
        .await
        .expect("partial reset remains observable"),
    );
    assert_eq!(
        owner
            .report()
            .await
            .expect("actual unacknowledged owner")
            .cache_epoch,
        "unacknowledged"
    );
    let app = guard_network(
        Router::new()
            .route(OBSERVE_PATH, axum::routing::get(observe))
            .route(CLEAR_PATH, axum::routing::post(clear))
            .layer(Extension(owner)),
        policy,
        Arc::new(Vec::new()),
        8080,
    );
    let reply = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(OBSERVE_PATH)
                .header("host", "owner.test:8080")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .expect("owner observe request"),
        )
        .await
        .expect("actual owner handler");
    assert_eq!(reply.status(), StatusCode::OK);
    let value: Value = serde_json::from_slice(
        &to_bytes(reply.into_body(), LIMIT)
            .await
            .expect("bounded owner report"),
    )
    .expect("typed report");
    assert_eq!(value["cache_epoch"], "unacknowledged");
    assert_eq!(
        value["reader_contract"], 0,
        "unsupported build cannot attest legacy retirement"
    );
    let mut empty = SettingsFile::default();
    empty.defaults.recon_agent = Some(ModelAssignment::default());
    empty.defaults.recon_decisions = Some(ModelAssignment::default());
    empty.defaults.recon_analyst = Some(ModelAssignment::default());
    empty.defaults.atlas_evaluator = Some(ModelAssignment::default());
    empty.defaults.atlas_analyst = Some(ModelAssignment::default());
    empty.defaults.brain_summary = Some(ModelAssignment::default());
    let mut settings = toml::Value::try_from(&empty).expect("empty current format");
    settings
        .as_table_mut()
        .expect("settings table")
        .insert("configuration_format".into(), toml::Value::Integer(1));
    config_commit::write_secure(
        &root.join("config.toml"),
        &toml::to_string(&settings).expect("empty settings bytes"),
    )
    .expect("coordinator's synthetic completed file");
    config_commit::write_secure(
        &root.join("auth.json"),
        "{\"configuration_format\":1,\"accounts\":{}}",
    )
    .expect("empty accounts");
    config_commit::write_secure(&root.join("quota.json"), "{\"settings\":[]}")
        .expect("empty quota");
    let reply = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(CLEAR_PATH)
                .header("host", "owner.test:8080")
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"configuration_epoch":"new-configuration"}).to_string(),
                ))
                .expect("owner clear request"),
        )
        .await
        .expect("actual clear handler");
    assert_eq!(reply.status(), StatusCode::OK);
    let value: Value = serde_json::from_slice(
        &to_bytes(reply.into_body(), LIMIT)
            .await
            .expect("bounded ACK"),
    )
    .expect("actual ACK report");
    assert_eq!(value["configuration_epoch"], "new-configuration");
    assert_eq!(value["cache_epoch"], "new-configuration");
    assert_eq!(
        value["access_epoch"], "old-service-access",
        "clearing files does not pretend a private-token reload"
    );
}
