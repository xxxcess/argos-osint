//! Picker references retain admission fences; index work requires canonical actors.
use std::ffi::OsString;
use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex;

use argos_osint_core::storage::bootstrap;
use argos_osint_core::storage::connect_registry;
use argos_osint_core::storage::connect_registry::RegistryAuthority;
use argos_osint_core::storage::job_dispatch;
#[cfg(feature = "embedded")]
use argos_osint_core::storage::repo::Repository;
use argos_osint_core::storage::setup_jobs;
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
use fleetworks_contracts::readiness::DatasetKind;
use fleetworks_contracts::readiness::DatasetRef;
use fleetworks_contracts::readiness::MaintenanceOperation;
use fleetworks_contracts::readiness::MaintenanceRequest;
use tower::ServiceExt;

use crate::request_context::ContextRequirement;
use crate::request_context::ResolvedRequest;
use crate::ApiError;
use crate::ApiState;
use crate::AuthenticatedPrincipal;
use crate::NetworkPolicy;
use crate::ScopedCredential;

struct Environment(Vec<(&'static str, Option<OsString>)>);
impl Environment {
    fn install(root: &Path) -> Self {
        Self(
            ["ARGOS_HOME", "FLEETWORKS_WORKSPACE"]
                .into_iter()
                .map(|key| {
                    let previous = std::env::var_os(key);
                    std::env::set_var(key, root);
                    (key, previous)
                })
                .collect(),
        )
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

#[tokio::test]
async fn picked_datasets_preserve_fences_and_reindex_requires_completed_nonempty_seed() {
    let _environment_gate = crate::CONFIGURATION_TEST_ENVIRONMENT.lock().await;
    let directory = tempfile::tempdir().expect("isolated maintenance owner");
    let root = directory
        .path()
        .canonicalize()
        .expect("canonical private root");
    let _environment = Environment::install(&root);
    #[cfg(feature = "embedded")]
    let repo = Repository::connect(
        &argos_osint_core::storage::open::Target::Embedded {
            path: root.join("store"),
        },
        "maintenance",
        "picker",
    )
    .await
    .expect("isolated embedded repository");
    #[cfg(not(feature = "embedded"))]
    let datastore =
        crate::storage_test_support::Server::start().expect("required isolated pinned server");
    #[cfg(not(feature = "embedded"))]
    let repo = datastore.repo("maintenance", "picker").await;
    bootstrap::ensure_network_identity(repo.db())
        .await
        .expect("identity");
    connect_registry::initialize_control(repo.db())
        .await
        .expect("control");
    let authority = RegistryAuthority::new(
        "maintenance-scope",
        "maintenance-native",
        "maintenance-epoch",
    )
    .expect("authority");
    let receipt = connect_registry::ensure(
        &repo,
        &authority,
        &RegistrationEnsureRequest::new(
            "ensure-maintenance".into(),
            None,
            "synthetic-maintenance-installation".into(),
        ),
    )
    .await
    .expect("registration")
    .receipt;
    // Resolve actual policy and registration, never construct a private context.
    let token = "synthetic-maintenance-native-0123456789abcdef";
    let credential = ScopedCredential::new(
        "maintenance-native".into(),
        "maintenance-scope".into(),
        "maintenance-epoch".into(),
        CredentialPurpose::Native,
        [
            Permission::ReadBackend,
            Permission::RegisterWorkstation,
            Permission::MaintainBackend,
        ]
        .into_iter()
        .collect(),
        token,
    )
    .expect("Native policy");
    let policy =
        NetworkPolicy::with_credentials(&["maintenance.test:8080".into()], vec![credential])
            .expect("policy");
    let (send, receive) = tokio::sync::oneshot::channel::<ResolvedRequest>();
    let send = Arc::new(Mutex::new(Some(send)));
    let gateway = crate::guard_network(
        Router::new().route(
            "/fixture/context",
            get({
                let repo = repo.clone();
                move |Extension(principal): Extension<AuthenticatedPrincipal>,
                      headers: HeaderMap| {
                    let repo = repo.clone();
                    let send = Arc::clone(&send);
                    async move {
                        let resolved = crate::request_context::resolve_gateway(
                            &repo,
                            &principal,
                            &headers,
                            Permission::MaintainBackend,
                            ContextRequirement::Registered,
                        )
                        .await
                        .expect("actual registered permission");
                        send.lock()
                            .expect("sender")
                            .take()
                            .expect("single request")
                            .send(resolved)
                            .expect("receiver");
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
                .uri("/fixture/context")
                .header("host", "maintenance.test:8080")
                .header("authorization", format!("Bearer {token}"))
                .header(
                    "x-fleetworks-registration",
                    receipt.registration_id.as_str(),
                )
                .header(
                    "x-fleetworks-registration-revision",
                    receipt.revision.to_string(),
                )
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    let context = receive.await.expect("resolved context");
    let state = ApiState {
        repo: repo.clone(),
        instance_id: "maintenance-fixture".into(),
        port: 8080,
        origins: Arc::new(Vec::new()),
    };
    let confirmed = MutationContext::new(context.registered().expect("registered context"))
        .with_configuration_revision(0);
    let actors = DatasetRef::new(
        "picked-actors".into(),
        DatasetKind::ActorCatalog,
        "a".repeat(64),
        128,
    )
    .expect("actor reference");
    let training = DatasetRef::new(
        "picked-training".into(),
        DatasetKind::ClassifierTraining,
        "b".repeat(64),
        128,
    )
    .expect("training reference");
    for reference in [&actors, &training] {
        setup_jobs::register_dataset(&repo, &authority, reference, setup_jobs::DATASET_TTL)
            .await
            .expect("previously server-validated upload");
    }
    for operation in [
        MaintenanceOperation::SeedActors,
        MaintenanceOperation::TrainClassifier,
        MaintenanceOperation::ReindexActors,
    ] {
        setup_jobs::enable_operation(repo.db(), operation, true)
            .await
            .expect("configured consumer");
        job_dispatch::enable_pool(repo.db(), setup_jobs::operation_name(operation))
            .await
            .expect("durable dispatch pool");
    }
    // No ReadinessOwners/manifest is supplied: the exact picker reference is
    // checked against server-owned staging metadata by the operation owner.
    let request =
        MaintenanceRequest::for_seed("seed-picked".into(), confirmed.clone(), actors.clone())
            .expect("seed request");
    let (status, Json(seed)) = super::seed(
        State(state.clone()),
        Extension(context.clone()),
        Json(request.clone()),
    )
    .await
    .map_err(|ApiError(status, message)| (status, message))
    .expect("direct uploaded actor reference");
    assert_eq!(status, StatusCode::ACCEPTED);
    let training_request = MaintenanceRequest::for_training(
        "train-picked".into(),
        confirmed.clone(),
        training.clone(),
    )
    .expect("train request");
    assert_eq!(
        super::train(
            State(state.clone()),
            Extension(context.clone()),
            Json(training_request)
        )
        .await
        .map_err(|ApiError(status, message)| (status, message))
        .expect("direct uploaded training reference")
        .0,
        StatusCode::ACCEPTED
    );
    let mut stale = request.clone();
    stale.request_id = "stale-revision".into();
    stale.context.expected_configuration_revision = Some(1);
    assert!(matches!(
        super::seed(
            State(state.clone()),
            Extension(context.clone()),
            Json(stale)
        )
        .await,
        Err(ApiError(StatusCode::CONFLICT, _))
    ));
    let mut altered = request.clone();
    altered.request_id = "altered-hash".into();
    altered.dataset.as_mut().expect("dataset").sha256 = "f".repeat(64);
    assert!(matches!(
        super::seed(
            State(state.clone()),
            Extension(context.clone()),
            Json(altered)
        )
        .await,
        Err(ApiError(StatusCode::CONFLICT, _))
    ));
    let mut wrong_kind = request.clone();
    wrong_kind.request_id = "wrong-kind".into();
    wrong_kind.dataset = Some(training);
    assert!(matches!(
        super::seed(
            State(state.clone()),
            Extension(context.clone()),
            Json(wrong_kind)
        )
        .await,
        Err(ApiError(StatusCode::CONFLICT, _))
    ));
    let foreign = DatasetRef::new(
        "foreign-upload".into(),
        DatasetKind::ActorCatalog,
        "c".repeat(64),
        128,
    )
    .expect("foreign reference");
    setup_jobs::register_dataset(
        &repo,
        &RegistryAuthority::new("other-scope", "other-native", "other-epoch")
            .expect("other authority"),
        &foreign,
        setup_jobs::DATASET_TTL,
    )
    .await
    .expect("other scope upload");
    let mut other = request;
    other.request_id = "other-scope".into();
    other.dataset = Some(foreign);
    assert!(matches!(
        super::seed(
            State(state.clone()),
            Extension(context.clone()),
            Json(other)
        )
        .await,
        Err(ApiError(StatusCode::CONFLICT, _))
    ));
    repo.db().query("UPDATE $job SET state='completed' RETURN NONE; UPDATE task SET state='completed' WHERE job=$job RETURN NONE;").bind(("job",surrealdb::types::RecordId::new("job",seed.job_id.clone()))).await.expect("completed initial seed job fixture").check().expect("initial job and tasks complete");
    let reindex = MaintenanceRequest::for_reindex("build-index".into(), confirmed.clone())
        .expect("index request");
    assert!(
        matches!(
            super::reindex(
                State(state.clone()),
                Extension(context.clone()),
                Json(reindex.clone())
            )
            .await,
            Err(ApiError(StatusCode::CONFLICT, _))
        ),
        "missing completed seed is not indexable"
    );
    repo.db().query("UPSERT brain_actor_seed:catalog SET status='complete',fingerprint='fixture-fingerprint',processed=1,total=1,owner='fixture',lease_until='fixture',updated_at='fixture' RETURN NONE;").await.expect("seed receipt fixture").check().expect("receipt");
    assert!(
        matches!(
            super::reindex(
                State(state.clone()),
                Extension(context.clone()),
                Json(reindex.clone())
            )
            .await,
            Err(ApiError(StatusCode::CONFLICT, _))
        ),
        "receipt without actors is not indexable"
    );
    repo.db().query("CREATE brain_actor:fixture SET display_name='Synthetic actor',scope_key=$scope,created_at='fixture',updated_at='fixture';").bind(("scope",argos_osint_core::actor_catalog::SCOPE)).await.expect("canonical actor fixture").check().expect("actor");
    let reseed_request =
        MaintenanceRequest::for_seed("queued-reseed".into(), confirmed.clone(), actors)
            .expect("explicit reseed request");
    let (_, Json(seed)) = super::seed(
        State(state.clone()),
        Extension(context.clone()),
        Json(reseed_request),
    )
    .await
    .map_err(|ApiError(status, message)| (status, message))
    .expect("new reseed queued while the old catalog remains complete");
    assert!(
        matches!(
            super::reindex(
                State(state.clone()),
                Extension(context.clone()),
                Json(reindex.clone())
            )
            .await,
            Err(ApiError(StatusCode::CONFLICT, _))
        ),
        "accepted queued reseed fences an old complete catalog before worker starts"
    );
    repo.db()
        .query("UPDATE $job SET state='completed' RETURN NONE;")
        .bind((
            "job",
            surrealdb::types::RecordId::new("job", seed.job_id.clone()),
        ))
        .await
        .expect("terminal job with pending seed task")
        .check()
        .expect("job state");
    assert!(
        matches!(
            super::reindex(
                State(state.clone()),
                Extension(context.clone()),
                Json(reindex.clone())
            )
            .await,
            Err(ApiError(StatusCode::CONFLICT, _))
        ),
        "nonterminal seed tasks still fence index admission after job becomes terminal"
    );
    repo.db().query("UPDATE $job SET state='completed' RETURN NONE; UPDATE task SET state='completed' WHERE job=$job RETURN NONE;").bind(("job",surrealdb::types::RecordId::new("job",seed.job_id.clone()))).await.expect("completed seed job fixture").check().expect("completed tasks");
    repo.db()
        .query("UPDATE brain_actor_seed:catalog SET status='running' RETURN NONE;")
        .await
        .expect("running seed")
        .check()
        .expect("running");
    assert!(
        matches!(
            super::reindex(
                State(state.clone()),
                Extension(context.clone()),
                Json(reindex.clone())
            )
            .await,
            Err(ApiError(StatusCode::CONFLICT, _))
        ),
        "running seed with actors is not indexable"
    );
    repo.db()
        .query("UPDATE brain_actor_seed:catalog SET status='complete',processed=0 RETURN NONE;")
        .await
        .expect("incomplete receipt")
        .check()
        .expect("partial");
    assert!(
        matches!(
            super::reindex(
                State(state.clone()),
                Extension(context.clone()),
                Json(reindex.clone())
            )
            .await,
            Err(ApiError(StatusCode::CONFLICT, _))
        ),
        "incomplete progress does not imply seed completion"
    );
    repo.db()
        .query("UPDATE brain_actor_seed:catalog SET processed=1 RETURN NONE;")
        .await
        .expect("completed progress")
        .check()
        .expect("complete");
    let (_, Json(accepted)) = super::reindex(
        State(state.clone()),
        Extension(context.clone()),
        Json(reindex.clone()),
    )
    .await
    .map_err(|ApiError(status, message)| (status, message))
    .expect("complete seed plus actors can build index");
    repo.db().query("DELETE brain_actor:fixture; UPDATE brain_actor_seed:catalog SET status='running' RETURN NONE;").await.expect("later seed change").check().expect("changed");
    let (_, Json(replayed)) = super::reindex(
        State(state.clone()),
        Extension(context.clone()),
        Json(reindex),
    )
    .await
    .map_err(|ApiError(status, message)| (status, message))
    .expect("accepted exact replay keeps original receipt despite later catalog change");
    assert_eq!(replayed, accepted);
    let fresh = MaintenanceRequest::for_reindex("new-index-after-change".into(), confirmed)
        .expect("fresh request");
    assert!(
        matches!(
            super::reindex(State(state), Extension(context), Json(fresh)).await,
            Err(ApiError(StatusCode::CONFLICT, _))
        ),
        "new work still requires the current complete catalog"
    );
}
