//! Actual purpose/context/privacy gateway plus genuine disposable Natural fit.
#![cfg(all(feature = "embedded", feature = "classifier"))]

#[path = "../../argos-osint-core/tests/fixtures/classifier/activation_training.rs"]
mod training;

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use argos_osint_api::classifier_activation::ActivationArtifacts;
use argos_osint_api::network_router;
use argos_osint_api::ApiState;
use argos_osint_api::NetworkPolicy;
use argos_osint_api::ScopedCredential;
use argos_osint_core::config_commit::CommitFile;
use argos_osint_core::config_commit::ConfigLock;
use argos_osint_core::storage::bootstrap;
use argos_osint_core::storage::classifier_activation;
use argos_osint_core::storage::classifier_activation::ActivationWork;
use argos_osint_core::storage::connect_registry;
use argos_osint_core::storage::connect_registry::RegistryAuthority;
use argos_osint_core::storage::job_dispatch;
use argos_osint_core::storage::job_dispatch::DeliveryClaim;
use argos_osint_core::storage::job_dispatch::DeliveryLease;
use argos_osint_core::storage::job_dispatch::DeliveryOutcome;
use argos_osint_core::storage::open::Target;
use argos_osint_core::storage::repo::Repository;
use argos_osint_core::storage::setup_jobs;
use argos_osint_core::storage::tx;
use axum::body::to_bytes;
use axum::body::Body;
use axum::http::Request;
use axum::http::StatusCode;
use axum::Extension;
use axum::Router;
use chrono::Utc;
use fleetworks_contracts::classifier_activation::ActivationRequest;
use fleetworks_contracts::classifier_activation::CandidatePreview;
use fleetworks_contracts::classifier_activation::SyntheticBaselineConsent;
use fleetworks_contracts::classifier_activation::ACTIVATION_PATH;
use fleetworks_contracts::classifier_activation::ACTIVATION_RESULT_PATH;
use fleetworks_contracts::classifier_activation::CANDIDATE_DETAIL_PATH;
use fleetworks_contracts::connect::RegistrationEnsureRequest;
use fleetworks_contracts::context::CredentialPurpose;
use fleetworks_contracts::context::JobOrigin;
use fleetworks_contracts::context::MutationContext;
use fleetworks_contracts::context::OriginSource;
use fleetworks_contracts::context::Permission;
use fleetworks_contracts::context::RegisteredContext;
use fleetworks_contracts::readiness::MaintenanceOperation;
use fleetworks_contracts::readiness::MaintenanceRequest;
use serde_json::json;
use serde_json::Value;
use tower::ServiceExt;

const NATIVE: &str = "fixture-native-activation-token-0123456789abcdef";
const READONLY: &str = "fixture-readonly-activation-token-0123456789abcdef";
struct Environment(Vec<(&'static str, Option<OsString>)>);
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
fn authority() -> RegistryAuthority {
    RegistryAuthority::new("activation-scope", "native", "activation-access")
        .expect("fixture policy authority")
}
async fn registration(repo: &Repository, node: &str) -> RegisteredContext {
    let registration = connect_registry::ensure(
        repo,
        &authority(),
        &RegistrationEnsureRequest::new(
            format!("ensure-{node}"),
            Some(
                if node == "a" {
                    "192.0.2.42"
                } else {
                    "198.51.100.42"
                }
                .parse()
                .expect("synthetic IP"),
            ),
            format!("fixture-installation-{node}"),
        ),
    )
    .await
    .expect("actual registration")
    .receipt;
    connect_registry::registration_context(
        repo,
        &authority(),
        &registration.registration_id,
        registration.revision,
    )
    .await
    .expect("actual registered context")
}
async fn claim(repo: &Repository, pool: &str) -> Box<DeliveryLease> {
    let outbox = job_dispatch::claim_ready(repo.db(), pool, "fixture-relay", Utc::now())
        .await
        .expect("outbox")
        .expect("accepted outbox record");
    job_dispatch::mark_published(repo.db(), &outbox, Utc::now())
        .await
        .expect("persistent publish receipt");
    match job_dispatch::claim_delivery(
        repo.db(),
        &outbox.envelope,
        "fixture-worker",
        300,
        Utc::now(),
        &argos_osint_core::store::new_id(),
    )
    .await
    .expect("actual delivery claim")
    {
        DeliveryClaim::Claimed(lease) => lease,
        _ => panic!("expected claimed delivery"),
    }
}
async fn call(
    app: &Router,
    method: &str,
    path: &str,
    token: &str,
    context: Option<&RegisteredContext>,
    body: Value,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("host", "127.0.0.1:8080")
        .header("authorization", format!("Bearer {token}"));
    if let Some(context) = context {
        request = request
            .header("x-fleetworks-registration", &context.registration_id)
            .header(
                "x-fleetworks-registration-revision",
                context.registration_revision.to_string(),
            );
    }
    let request = if method == "POST" {
        request
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
    } else {
        request.body(Body::empty())
    }
    .expect("bounded fixture request");
    let response = app
        .clone()
        .oneshot(request)
        .await
        .expect("actual gateway response");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("bounded response");
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or_else(|_| json!({})),
    )
}
#[tokio::test]
async fn explicit_activation_enforces_permissions_source_privacy_revision_and_exact_accepted_replay(
) {
    let temporary = tempfile::tempdir().expect("disposable backend");
    let root = temporary
        .path()
        .canonicalize()
        .expect("canonical temp home");
    let _environment = Environment(
        ["FLEETWORKS_WORKSPACE", "ARGOS_HOME"]
            .into_iter()
            .map(|key| {
                let previous = std::env::var_os(key);
                std::env::set_var(key, &root);
                (key, previous)
            })
            .collect(),
    );
    let lock = ConfigLock::acquire_at(&root).expect("configuration lock");
    let snapshot = argos_osint_core::current_configuration::read_locked(&root, &lock)
        .expect("fresh current empty snapshot");
    let mut settings = toml::Value::try_from(&snapshot.settings).expect("settings");
    settings
        .as_table_mut()
        .expect("table")
        .insert("configuration_format".into(), toml::Value::Integer(1));
    let revision = lock
        .commit_files(&[
            CommitFile::new(
                "current_settings",
                root.join("config.toml"),
                toml::to_string(&settings).expect("settings JSON"),
            ),
            CommitFile::new(
                "auth",
                root.join("auth.json"),
                "{\"configuration_format\":1,\"accounts\":{}}".into(),
            ),
            CommitFile::new("quota", root.join("quota.json"), "{\"settings\":[]}".into()),
        ])
        .expect("actual initial configuration commit");
    drop(lock);
    let repo = Repository::connect(
        &Target::Embedded {
            path: root.join("store"),
        },
        "fixture",
        "activation_api",
    )
    .await
    .expect("isolated store");
    bootstrap::ensure_network_identity(repo.db())
        .await
        .expect("real identity bootstrap");
    connect_registry::initialize_control(repo.db())
        .await
        .expect("real control bootstrap");
    repo.db()
        .query(classifier_activation::SCHEMA)
        .await
        .expect("activation schema")
        .check()
        .expect("schema statements");
    let creator = registration(&repo, "a").await;
    let other = registration(&repo, "b").await;
    let (parsed, dataset) = training::prepare(&root);
    setup_jobs::register_dataset(&repo, &authority(), &dataset, setup_jobs::DATASET_TTL)
        .await
        .expect("actual parsed immutable bytes");
    job_dispatch::ensure_queue_transport(repo.db(), setup_jobs::TRAIN_POOL)
        .await
        .expect("training transport");
    job_dispatch::enable_pool(repo.db(), setup_jobs::TRAIN_POOL)
        .await
        .expect("consumer");
    setup_jobs::enable_operation(repo.db(), MaintenanceOperation::TrainClassifier, true)
        .await
        .expect("explicit train operation");
    let training_request = MaintenanceRequest::for_training(
        "train-once".into(),
        MutationContext::new(&creator).with_configuration_revision(revision),
        dataset.clone(),
    )
    .expect("training does not activate");
    let accepted = tx::run_tx(repo.db(), tx::RetryPolicy::default(), |transaction| {
        let request = &training_request;
        let context = &creator;
        async move {
            let origin = JobOrigin::new(
                context,
                OriginSource::Native,
                "train-once".into(),
                Utc::now().to_rfc3339(),
                revision,
                Vec::new(),
            );
            setup_jobs::admit_in(
                &transaction,
                &authority(),
                context,
                &origin,
                MaintenanceOperation::TrainClassifier,
                request,
            )
            .await
        }
    })
    .await
    .expect("real attributed training admission")
    .value;
    let training_lease = claim(&repo, setup_jobs::TRAIN_POOL).await;
    let source_job = accepted.job_id.clone();
    let home = root.clone();
    let trained = tokio::task::spawn_blocking(move || training::train(&home, &parsed, &source_job))
        .await
        .expect("genuine Natural fit");
    setup_jobs::record_result(&repo,&training_lease.task,&json!({"operation":"classifier_train","engine":"natural","dataset_sha256":dataset.sha256,"candidate_model_id":trained.candidate.model_id,"activation_requested":false})).await.expect("real live training result");
    job_dispatch::finish_delivery(
        repo.db(),
        &training_lease,
        DeliveryOutcome::Complete {
            result_ref: "fixture-candidate",
        },
        Utc::now(),
    )
    .await
    .expect("training completion");
    job_dispatch::ensure_queue_transport(repo.db(), classifier_activation::POOL)
        .await
        .expect("activation transport");
    classifier_activation::configure_consumer(&repo, true)
        .await
        .expect("actual activation consumer");
    let hosts = vec!["127.0.0.1:8080".into()];
    let credential = |id: &str, token: &str, permissions: BTreeSet<Permission>| {
        ScopedCredential::new(
            id.into(),
            "activation-scope".into(),
            "activation-access".into(),
            CredentialPurpose::Native,
            permissions,
            token,
        )
        .expect("actual scoped credential")
    };
    let full = BTreeSet::from([
        Permission::ReadBackend,
        Permission::RegisterWorkstation,
        Permission::MaintainBackend,
        Permission::ManageConfiguration,
        Permission::ReadWorkstationActivity,
        Permission::SubmitInteractive,
    ]);
    let policy = NetworkPolicy::with_credentials(
        &hosts,
        vec![
            credential("native", NATIVE, full),
            credential(
                "reader",
                READONLY,
                BTreeSet::from([
                    Permission::ReadBackend,
                    Permission::RegisterWorkstation,
                    Permission::ReadWorkstationActivity,
                ]),
            ),
        ],
    )
    .expect("distinct purpose policy");
    let state = ApiState {
        repo: repo.clone(),
        instance_id: "fixture-api".into(),
        port: 8080,
        origins: Arc::new(Vec::new()),
    };
    let app = network_router(state, policy).layer(Extension(Arc::new(
        ActivationArtifacts::new(root.clone()).expect("real server-owned artifact home"),
    )));
    let preview_path = format!(
        "{CANDIDATE_DETAIL_PATH}?training_job_id={}&model_id={}",
        accepted.job_id, trained.candidate.model_id
    );
    assert_eq!(
        call(
            &app,
            "GET",
            &preview_path,
            READONLY,
            Some(&creator),
            json!({})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(&app, "GET", &preview_path, NATIVE, Some(&other), json!({}))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let (status, body) = call(
        &app,
        "GET",
        &preview_path,
        NATIVE,
        Some(&creator),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let preview: CandidatePreview =
        serde_json::from_value(body).expect("actual scoped hash/prior preview");
    assert_eq!(preview.candidate, trained.candidate);
    let request = ActivationRequest::new(
        "activate-once".into(),
        MutationContext::new(&creator).with_configuration_revision(revision),
        preview.candidate.clone(),
        preview.expected_prior.clone(),
        SyntheticBaselineConsent::ActivateSyntheticBaseline,
    )
    .expect("separate confirmation");
    let (status, receipt) = call(
        &app,
        "POST",
        ACTIVATION_PATH,
        NATIVE,
        Some(&creator),
        serde_json::to_value(&request).expect("typed request"),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let job_id = receipt["job_id"]
        .as_str()
        .expect("durable activation job")
        .to_owned();
    let (status,_)=call(&app,"POST","/api/v1/profile/accounts/upsert",NATIVE,Some(&creator),json!({"expected_revision":revision,"account_id":"local","provider":"local","base_url":"http://127.0.0.1:11434/v1","model":"","api_key":null})).await;
    assert_eq!(status, StatusCode::OK);
    let (status, repeated) = call(
        &app,
        "POST",
        ACTIVATION_PATH,
        NATIVE,
        Some(&creator),
        serde_json::to_value(&request).expect("same frozen request"),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(repeated["job_id"], job_id);
    assert_eq!(repeated["origin"], receipt["origin"]);
    let mut stale = request.clone();
    stale.request_id = "new-stale-request".into();
    assert_eq!(
        call(
            &app,
            "POST",
            ACTIVATION_PATH,
            NATIVE,
            Some(&creator),
            serde_json::to_value(stale).expect("stale new request")
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let lease = claim(&repo, classifier_activation::POOL).await;
    let result = classifier_activation::execute_claimed(
        &repo,
        root,
        &lease.task,
        Arc::new(AtomicBool::new(false)),
    )
    .await
    .expect("actual claimed activation");
    let ActivationWork::Complete { result_ref } = result else {
        panic!("genuine activation must complete");
    };
    job_dispatch::finish_delivery(
        repo.db(),
        &lease,
        DeliveryOutcome::Complete {
            result_ref: &result_ref,
        },
        Utc::now(),
    )
    .await
    .expect("actual terminal activation");
    let result_path = format!("{ACTIVATION_RESULT_PATH}?job_id={job_id}");
    assert_eq!(
        call(&app, "GET", &result_path, NATIVE, Some(&other), json!({}))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let (status, result) = call(&app, "GET", &result_path, NATIVE, Some(&creator), json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(result["state"], "activated");
}
