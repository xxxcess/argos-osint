//! Real capture/save/acceptance ordering, with a nonblocking OS-lock witness.

use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex;

use argos_osint_core::config_commit::CommitFile;
use argos_osint_core::config_commit::ConfigLock;
use argos_osint_core::osint::manual;
use argos_osint_core::osint::manual::Tools;
use argos_osint_core::provider;
use argos_osint_core::provider::SettingsFile;
use argos_osint_core::storage::bootstrap;
use argos_osint_core::storage::connect_registry;
use argos_osint_core::storage::connect_registry::RegistryAuthority;
use argos_osint_core::storage::job_context;
use argos_osint_core::storage::job_dispatch;
#[cfg(feature = "embedded")]
use argos_osint_core::storage::repo::Repository;
use axum::body::Body;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::http::Request;
use axum::http::StatusCode;
use axum::routing::get;
use axum::Extension;
use axum::Json;
use axum::Router;
use fleetworks_contracts::connect::RegistrationEnsureRequest;
use fleetworks_contracts::context::CredentialPurpose;
use fleetworks_contracts::context::MutationContext;
use fleetworks_contracts::context::Permission;
use serde_json::json;
use tower::ServiceExt;

use super::capture_checked;
use crate::configuration;
use crate::request_context;
use crate::request_context::ContextRequirement;
use crate::request_context::ResolvedRequest;
use crate::ApiError;
use crate::ApiState;
use crate::AuthenticatedPrincipal;
use crate::NetworkPolicy;
use crate::ScopedCredential;

const TOKEN: &str = "synthetic-admission-native-0123456789abcdefghijklmnop";
const HOST: &str = "admission.test:8080";

struct Environment(Vec<(&'static str, Option<OsString>)>);
impl Environment {
    fn install(root: &Path) -> Self {
        let previous = ["ARGOS_HOME", "FLEETWORKS_WORKSPACE"]
            .into_iter()
            .map(|key| {
                let previous = std::env::var_os(key);
                std::env::set_var(key, root);
                (key, previous)
            })
            .collect();
        Self(previous)
    }
}
impl Drop for Environment {
    fn drop(&mut self) {
        for (key, value) in &self.0 {
            if let Some(value) = value {
                std::env::set_var(key, value);
            } else {
                std::env::remove_var(key);
            }
        }
    }
}

fn initialize_configuration(root: &Path) -> u64 {
    let mut settings = SettingsFile::default();
    for role in provider::CANONICAL_ROLE_KEYS {
        *settings.defaults.role_mut(role).expect("canonical role") = Default::default();
    }
    let mut value = toml::Value::try_from(settings).expect("current settings");
    value
        .as_table_mut()
        .expect("settings table")
        .insert("configuration_format".into(), toml::Value::Integer(1));
    ConfigLock::acquire_at(root)
        .expect("actual configuration writer")
        .commit_files(&[
            CommitFile::new(
                "current_settings",
                root.join("config.toml"),
                toml::to_string(&value).expect("current settings bytes"),
            ),
            CommitFile::new(
                "auth",
                root.join("auth.json"),
                r#"{"configuration_format":1,"accounts":{}}"#.to_owned(),
            ),
            CommitFile::new(
                "quota",
                root.join("quota.json"),
                r#"{"settings":[]}"#.to_owned(),
            ),
        ])
        .expect("actual durable configuration journal commit")
}

async fn assert_configuration_guard_held(root: &Path) {
    let guard = root.join(".argos-config.guard");
    tokio::task::spawn_blocking(move || {
        // A separate open file description probes the actual stable guard. No
        // scheduler delay or lock-acquisition timeout is used as success proof.
        let file = fs::OpenOptions::new()
            .write(true)
            .open(guard)
            .expect("existing guard inode");
        match file.try_lock() {
            Err(fs::TryLockError::WouldBlock) => (),
            Ok(()) => panic!("configuration guard dropped before durable acceptance boundary"),
            Err(fs::TryLockError::Error(error)) => panic!("actual guard probe failed: {error}"),
        }
    })
    .await
    .expect("joined deterministic guard probe");
}

#[tokio::test]
async fn captured_configuration_stays_locked_through_acceptance_then_save_fences_fresh_nonce_and_preserves_replay(
) {
    let _environment_guard = crate::CONFIGURATION_TEST_ENVIRONMENT.lock().await;
    let temporary = tempfile::tempdir().expect("isolated configuration root");
    let root = temporary
        .path()
        .canonicalize()
        .expect("canonical private root");
    let _environment = Environment::install(&root);
    let initial_revision = initialize_configuration(&root);

    #[cfg(not(feature = "embedded"))]
    let datastore = crate::storage_test_support::Server::start()
        .expect("required pinned isolated SurrealDB harness");
    #[cfg(not(feature = "embedded"))]
    let repo = datastore.repo("admission_guard", "capture_save").await;
    #[cfg(feature = "embedded")]
    let repo = Repository::connect(
        &argos_osint_core::storage::open::Target::Embedded {
            path: root.join("store"),
        },
        "admission_guard",
        "capture_save",
    )
    .await
    .expect("actual isolated embedded repository");
    bootstrap::ensure_network_identity(repo.db())
        .await
        .expect("actual deployment identity");
    connect_registry::initialize_control(repo.db())
        .await
        .expect("actual registry bootstrap");
    job_dispatch::ensure_queue_transport(repo.db(), manual::TOOLS_POOL)
        .await
        .expect("real durable transport policy");
    job_dispatch::enable_pool(repo.db(), manual::TOOLS_POOL)
        .await
        .expect("explicit fixture consumer enablement");
    let authority =
        RegistryAuthority::new("admission-scope", "admission-native", "admission-access")
            .expect("synthetic authority");
    let registration = connect_registry::ensure(
        &repo,
        &authority,
        &RegistrationEnsureRequest::new(
            "ensure-admission-guard".into(),
            Some("192.0.2.93".parse().expect("synthetic IP")),
            "synthetic-admission-install".into(),
        ),
    )
    .await
    .expect("actual registered native context")
    .receipt;

    // Obtain the opaque ResolvedRequest through actual purpose authentication
    // and registry resolution. The fixture does not construct private fields.
    let credential = ScopedCredential::new(
        "admission-native".into(),
        "admission-scope".into(),
        "admission-access".into(),
        CredentialPurpose::Native,
        [
            Permission::ReadBackend,
            Permission::ReadWorkstationActivity,
            Permission::RegisterWorkstation,
            Permission::SubmitInteractive,
            Permission::ManageConfiguration,
        ]
        .into_iter()
        .collect(),
        TOKEN,
    )
    .expect("actual Native purpose policy");
    let policy = NetworkPolicy::with_credentials(&[HOST.into()], vec![credential])
        .expect("private fixture ingress");
    let (send, receive) = tokio::sync::oneshot::channel::<ResolvedRequest>();
    let send = Arc::new(Mutex::new(Some(send)));
    let gateway = crate::guard_network(
        Router::new().route(
            "/fixture/registered-proof",
            get({
                let repo = repo.clone();
                move |Extension(principal): Extension<AuthenticatedPrincipal>,
                      headers: HeaderMap| {
                    let repo = repo.clone();
                    let send = Arc::clone(&send);
                    async move {
                        let context = request_context::resolve_gateway(
                            &repo,
                            &principal,
                            &headers,
                            Permission::ManageConfiguration,
                            ContextRequirement::Registered,
                        )
                        .await
                        .expect("actual native proof resolution");
                        send.lock()
                            .expect("fixture sender")
                            .take()
                            .expect("one context")
                            .send(context)
                            .expect("waiting fixture receiver");
                        StatusCode::OK
                    }
                }
            }),
        ),
        policy,
        Arc::new(Vec::new()),
        8080,
    );
    let response = gateway
        .oneshot(
            Request::builder()
                .uri("/fixture/registered-proof")
                .header("host", HOST)
                .header("authorization", format!("Bearer {TOKEN}"))
                .header(
                    request_context::REGISTRATION_HEADER,
                    &registration.registration_id,
                )
                .header(
                    request_context::REGISTRATION_REVISION_HEADER,
                    registration.revision.to_string(),
                )
                .body(Body::empty())
                .expect("registered proof request"),
        )
        .await
        .expect("actual ingress");
    assert_eq!(response.status(), StatusCode::OK);
    let context = receive.await.expect("server-resolved request authority");
    let mutation = MutationContext::new(context.registered().expect("current registration"))
        .with_configuration_revision(initial_revision);
    let admission = capture_checked(
        &repo,
        &context,
        "guard-fresh-nonce",
        manual::TOOLS_OPERATION,
        Some(&mutation),
    )
    .await
    .map_err(|ApiError(status, message)| (status, message))
    .expect("current confirmed capture");
    assert_eq!(admission.origin().configuration_revision, initial_revision);
    assert_configuration_guard_held(&root).await;

    let tools = Tools::default();
    let inputs = json!({"domain":"example.test"});
    let accepted = tools
        .submit_interactive(
            &repo,
            "guard-fresh-nonce",
            "crtsh_certificates",
            inputs.clone(),
            &admission,
        )
        .await
        .expect("actual durable acceptance; no provider execution");
    let job = accepted.job_id.as_deref().expect("durable Job receipt");
    assert_eq!(
        job_context::origin_for_job(&repo, job)
            .await
            .expect("actual immutable origin")
            .expect("stored origin")
            .configuration_revision,
        initial_revision
    );
    assert_configuration_guard_held(&root).await;
    // Cloning captured authority must not release its shared OS guard early.
    let retained = admission.clone();
    drop(admission);
    assert_configuration_guard_held(&root).await;
    drop(retained);

    // Start the real normal writer only after all capture guards are released.
    let state = ApiState {
        repo: repo.clone(),
        instance_id: "admission-guard-fixture".into(),
        port: 8080,
        origins: Arc::new(Vec::new()),
    };
    let request=serde_json::from_value::<configuration::SetRole>(json!({"expected_revision":initial_revision,"role":"brain_summary","primary":null,"fallbacks":[]})).expect("normal typed configuration save");
    let Json(saved) = configuration::save_role(
        State(state),
        Some(Extension(context.clone())),
        Json(request),
    )
    .await
    .map_err(|ApiError(status, message)| (status, message))
    .expect("normal authorized writer after guard release");
    let saved_revision = saved["configuration_revision"]
        .as_u64()
        .expect("actual committed revision");
    assert!(saved_revision > initial_revision);
    let reopened = tokio::task::spawn_blocking({
        let root = root.clone();
        move || {
            argos_osint_core::current_configuration::read(&root)
                .expect("actual post-save snapshot")
                .revision
        }
    })
    .await
    .expect("joined configuration read");
    assert_eq!(reopened, saved_revision);

    let stale = capture_checked(
        &repo,
        &context,
        "guard-stale-new-nonce",
        manual::TOOLS_OPERATION,
        Some(&mutation),
    )
    .await;
    assert!(
        matches!(stale, Err(ApiError(StatusCode::CONFLICT, _))),
        "fresh nonce cannot use the pre-save confirmed revision"
    );
    let replay = capture_checked(
        &repo,
        &context,
        "guard-fresh-nonce",
        manual::TOOLS_OPERATION,
        Some(&mutation),
    )
    .await
    .map_err(|ApiError(status, message)| (status, message))
    .expect("exact accepted replay after save");
    let replayed = tools
        .submit_interactive(
            &repo,
            "guard-fresh-nonce",
            "crtsh_certificates",
            inputs,
            &replay,
        )
        .await
        .expect("actual identical-input replay");
    assert_eq!(replayed.call_id, accepted.call_id);
    assert_eq!(replayed.job_id, accepted.job_id);
    assert_eq!(
        job_context::origin_for_job(&repo, job)
            .await
            .expect("retained immutable origin")
            .expect("stored original")
            .configuration_revision,
        initial_revision
    );
    let mut rows = repo.db()
        .query("SELECT VALUE job_id FROM job_request; SELECT VALUE <string>record::id(id) FROM osint_call;")
        .await.expect("actual acceptance identities").check().expect("identity statements");
    assert_eq!(
        rows.take::<Vec<String>>(0)
            .expect("exact requester association"),
        [job.to_owned()]
    );
    assert_eq!(
        rows.take::<Vec<String>>(1).expect("exact durable call"),
        [accepted.call_id]
    );
    drop(replay);

    // Exercise the actual collection/delete handlers with the native typed body,
    // using this fixture's existing isolated configuration environment.
    let cycle_state = ApiState {
        repo: repo.clone(),
        instance_id: "cycle-config-fixture".into(),
        port: 8080,
        origins: Arc::new(Vec::new()),
    };
    job_dispatch::enable_pool(repo.db(), job_dispatch::ATLAS_POOL)
        .await
        .expect("cycle consumer");
    let current = MutationContext::new(context.registered().expect("registered origin"))
        .with_configuration_revision(saved_revision);
    let collect = fleetworks_contracts::AtlasCycleRequest::new("cycle-config-fixture".into())
        .expect("cycle request")
        .with_context(current.clone());
    let (_, Json(cycle)) = crate::operations::submit_atlas_cycle(
        State(cycle_state.clone()),
        Extension(context.clone()),
        Json(collect),
    )
    .await
    .map_err(|ApiError(status, message)| (status, message))
    .expect("actual collection admission");
    let delete = fleetworks_contracts::cycle_history::DeleteCycleRequest::new(
        "cycle-delete-config-fixture".into(),
        current,
        cycle.job_id.clone(),
        1,
    );
    let mut missing = delete.clone();
    missing.context.expected_configuration_revision = None;
    assert!(matches!(
        crate::cycle_history::delete(
            State(cycle_state.clone()),
            Extension(context.clone()),
            Json(missing),
        )
        .await,
        Err(ApiError(StatusCode::PRECONDITION_REQUIRED, _))
    ));
    let mut stale_delete = delete.clone();
    stale_delete.context.expected_configuration_revision = Some(initial_revision);
    assert!(matches!(
        crate::cycle_history::delete(
            State(cycle_state.clone()),
            Extension(context.clone()),
            Json(stale_delete),
        )
        .await,
        Err(ApiError(StatusCode::CONFLICT, _))
    ));
    let Json(deleted) = crate::cycle_history::delete(
        State(cycle_state.clone()),
        Extension(context.clone()),
        Json(delete.clone()),
    )
    .await
    .map_err(|ApiError(status, message)| (status, message))
    .expect("native revision accepted");
    assert!(deleted.hidden);
    assert_eq!(deleted.job_id, cycle.job_id);
    let role_save = serde_json::from_value::<configuration::SetRole>(json!({"expected_revision":saved_revision,"role":"brain_summary","primary":null,"fallbacks":[]})).expect("later config save");
    let Json(later) = configuration::save_role(
        State(cycle_state.clone()),
        Some(Extension(context.clone())),
        Json(role_save),
    )
    .await
    .map_err(|ApiError(status, message)| (status, message))
    .expect("later configuration saved");
    let later_revision = later["configuration_revision"]
        .as_u64()
        .expect("later revision");
    assert!(later_revision > saved_revision);
    let Json(replayed_delete) = crate::cycle_history::delete(
        State(cycle_state.clone()),
        Extension(context.clone()),
        Json(delete.clone()),
    )
    .await
    .map_err(|ApiError(status, message)| (status, message))
    .expect("exact accepted deletion replay after config change");
    assert_eq!(replayed_delete, deleted);
    let mut altered = delete;
    altered.context.expected_configuration_revision = Some(later_revision);
    assert!(
        matches!(
            crate::cycle_history::delete(State(cycle_state), Extension(context), Json(altered),)
                .await,
            Err(ApiError(StatusCode::CONFLICT, _))
        ),
        "an accepted nonce cannot acquire a refreshed request body"
    );
}
