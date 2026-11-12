//! Durable setup metadata, resource admission and origin/pinning transactions.
#![cfg(feature = "surreal")]

mod support;

use argos_osint_core::storage::connect_registry;
use argos_osint_core::storage::connect_registry::RegistryAuthority;
use argos_osint_core::storage::job_dispatch;
use argos_osint_core::storage::repo::Repository;
use argos_osint_core::storage::setup_jobs;
use argos_osint_core::storage::tx;
use fleetworks_contracts::connect::RegistrationEnsureRequest;
use fleetworks_contracts::connect::WorkspaceName;
use fleetworks_contracts::context::JobOrigin;
use fleetworks_contracts::context::MutationContext;
use fleetworks_contracts::context::OriginSource;
use fleetworks_contracts::context::RegisteredContext;
use fleetworks_contracts::readiness::DatasetKind;
use fleetworks_contracts::readiness::DatasetRef;
use fleetworks_contracts::readiness::MaintenanceOperation;
use fleetworks_contracts::readiness::MaintenanceReceipt;
use fleetworks_contracts::readiness::MaintenanceRequest;

use support::Server;

fn authority() -> RegistryAuthority {
    RegistryAuthority::new("fixture-scope", "fixture-native", "fixture-access-epoch")
        .expect("synthetic authority")
}

async fn initialize(repo: &Repository) {
    repo.db()
        .query(
            "UPSERT storage_meta:identity SET instance_id='synthetic-setup-deployment' RETURN NONE",
        )
        .await
        .expect("fixture identity")
        .check()
        .expect("identity statement");
    connect_registry::initialize_control(repo.db())
        .await
        .expect("synthetic bootstrap");
}

async fn context(repo: &Repository, ip: &str, marker: &str) -> RegisteredContext {
    let input = RegistrationEnsureRequest::new(
        format!("ensure-{marker}"),
        ip.parse().expect("synthetic native IP"),
        WorkspaceName::new("research").expect("child name"),
        format!("synthetic-install-{marker}"),
        format!("synthetic-folder-{marker}"),
    );
    let authority = authority();
    let receipt = connect_registry::ensure(repo, &authority, &input)
        .await
        .expect("registered synthetic context")
        .receipt;
    connect_registry::registration_context(
        repo,
        &authority,
        &receipt.registration_id,
        receipt.revision,
    )
    .await
    .expect("current synthetic context")
}

fn reference(id: &str) -> DatasetRef {
    DatasetRef::new(id.into(), DatasetKind::ActorCatalog, "a".repeat(64), 128)
        .expect("synthetic previously validated bytes")
}

fn origin(context: &RegisteredContext, key: &str) -> JobOrigin {
    JobOrigin::new(
        context,
        OriginSource::Native,
        key.into(),
        "2026-01-01T00:00:00Z".into(),
        1,
        Vec::new(),
    )
}

async fn admit(
    repo: &Repository,
    context: &RegisteredContext,
    origin: &JobOrigin,
    request: &MaintenanceRequest,
) -> Result<MaintenanceReceipt, tx::TxError> {
    let authority = authority();
    let committed = tx::run_tx(repo.db(), tx::RetryPolicy::default(), |transaction| {
        let authority = authority.clone();
        async move {
            setup_jobs::admit_in(
                &transaction,
                &authority,
                context,
                origin,
                MaintenanceOperation::SeedActors,
                request,
            )
            .await
        }
    })
    .await?;
    Ok(committed.value)
}

async fn enable(repo: &Repository) {
    job_dispatch::enable_pool(repo.db(), setup_jobs::SEED_POOL)
        .await
        .expect("explicit fixture queue pool");
    setup_jobs::enable_operation(repo.db(), MaintenanceOperation::SeedActors, true)
        .await
        .expect("explicit synthetic consumer enablement");
}

#[tokio::test]
async fn unavailable_dispatch_never_accepts_and_wrong_scope_never_resolves_dataset() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "setup_disabled").await;
    initialize(&repo).await;
    let context = context(&repo, "192.0.2.51", "one").await;
    let dataset = reference("dataset-one");
    setup_jobs::register_dataset(&repo, &authority(), &dataset, setup_jobs::DATASET_TTL)
        .await
        .expect("server validated dataset");
    let request = MaintenanceRequest::for_seed(
        "seed-one".into(),
        MutationContext::new(&context),
        dataset.clone(),
    )
    .expect("typed seed request");
    assert!(
        admit(&repo, &context, &origin(&context, "seed-one"), &request)
            .await
            .is_err()
    );
    let other = RegistryAuthority::new("other-scope", "fixture-other", "fixture-access-epoch")
        .expect("other synthetic authority");
    assert!(setup_jobs::dataset(&repo, &other, &dataset.id)
        .await
        .is_err());
    assert!(
        setup_jobs::dataset_for_job(&repo, "unaccepted-job", &dataset)
            .await
            .is_err()
    );
    let mut rows=repo.db().query("SELECT VALUE payload_json FROM setup_operation; SELECT VALUE job_id FROM setup_dataset_pin").await.expect("fixture rows").check().expect("fixture read statements");
    assert!(rows
        .take::<Vec<String>>(0)
        .expect("operation rows")
        .is_empty());
    assert!(rows.take::<Vec<String>>(1).expect("pins").is_empty());
}

#[tokio::test]
async fn actual_admission_pins_bytes_survives_expiry_and_replays_original_receipt() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "setup_admit").await;
    initialize(&repo).await;
    enable(&repo).await;
    let context = context(&repo, "192.0.2.52", "one").await;
    let dataset = reference("dataset-one");
    setup_jobs::begin_upload(&repo, &authority(), &dataset.id, dataset.kind)
        .await
        .expect("owned upload lease");
    setup_jobs::register_dataset(&repo, &authority(), &dataset, setup_jobs::DATASET_TTL)
        .await
        .expect("validated bytes");
    let request = MaintenanceRequest::for_seed(
        "seed-one".into(),
        MutationContext::new(&context).with_configuration_revision(1),
        dataset.clone(),
    )
    .expect("seed request");
    let original = origin(&context, "seed-one");
    let receipt = admit(&repo, &context, &original, &request)
        .await
        .expect("atomic durable acceptance");
    repo.db().query("UPDATE setup_dataset SET expires_at=time::now()-1s RETURN NONE; UPDATE setup_upload SET expires_at=time::now()-1s RETURN NONE;").await.expect("controlled expiry").check().expect("expiry fixture");
    assert!(setup_jobs::dataset(&repo, &authority(), &dataset.id)
        .await
        .is_err());
    assert_eq!(
        setup_jobs::dataset_for_job(&repo, &receipt.job_id, &dataset)
            .await
            .expect("accepted pin survives upload TTL"),
        dataset
    );
    assert!(
        setup_jobs::claim_expired_uploads(&repo)
            .await
            .expect("bounded expiry claims")
            .is_empty(),
        "pinned bytes cannot be garbage-collected"
    );
    let mut retry_origin = original.clone();
    retry_origin.submitted_at = "2026-01-02T00:00:00Z".into();
    retry_origin.configuration_revision = 2;
    assert_eq!(
        admit(&repo, &context, &retry_origin, &request)
            .await
            .expect("uncertain reply recovery"),
        receipt,
        "first accepted origin/receipt stays immutable after configuration changes"
    );
    let mut stale_request = request.clone();
    stale_request.request_id = "fresh-stale-request".into();
    let mut stale_origin = retry_origin;
    stale_origin.request_id = stale_request.request_id.clone();
    let error = admit(&repo, &context, &stale_origin, &stale_request)
        .await
        .expect_err("new work cannot replay another nonce across configuration drift");
    assert!(
        matches!(error, tx::TxError::Abort(ref code) if code == "setup:configuration_revision_changed"),
        "configuration conflict must precede even the expired upload lookup: {error}"
    );
    let mut rows = repo.db()
        .query("SELECT VALUE <string>id FROM setup_operation; SELECT VALUE job_id FROM setup_dataset_pin")
        .await.expect("durable exact rows").check().expect("row read statements");
    assert_eq!(rows.take::<Vec<String>>(0).expect("operations").len(), 1);
    assert_eq!(rows.take::<Vec<String>>(1).expect("pins"), [receipt.job_id]);
}

#[tokio::test]
async fn different_nodes_share_one_resource_job_with_request_associations_and_busy_conflicts() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "setup_resource").await;
    initialize(&repo).await;
    enable(&repo).await;
    let first = context(&repo, "192.0.2.53", "one").await;
    let second = context(&repo, "192.0.2.54", "two").await;
    let dataset = reference("dataset-one");
    setup_jobs::register_dataset(&repo, &authority(), &dataset, setup_jobs::DATASET_TTL)
        .await
        .expect("validated bytes");
    let one = MaintenanceRequest::for_seed(
        "seed-one".into(),
        MutationContext::new(&first),
        dataset.clone(),
    )
    .expect("first request");
    let one = admit(&repo, &first, &origin(&first, "seed-one"), &one)
        .await
        .expect("first admission");
    let two =
        MaintenanceRequest::for_seed("seed-two".into(), MutationContext::new(&second), dataset)
            .expect("second request");
    let two = admit(&repo, &second, &origin(&second, "seed-two"), &two)
        .await
        .expect("same resource association");
    assert_eq!(one.job_id, two.job_id);
    assert!(two.reused);
    let mut rows=repo.db().query("SELECT VALUE workstation_id FROM job_origin; SELECT VALUE workstation_id FROM job_request").await.expect("durable origin read").check().expect("origin read statements");
    assert_eq!(
        rows.take::<Vec<String>>(0).expect("creator"),
        [first.workstation_id.clone()]
    );
    let associations = rows.take::<Vec<String>>(1).expect("request associations");
    assert_eq!(associations.len(), 2);
    assert!(associations.contains(&second.workstation_id));
    let different = reference("dataset-different");
    setup_jobs::register_dataset(&repo, &authority(), &different, setup_jobs::DATASET_TTL)
        .await
        .expect("second validated input");
    let different = MaintenanceRequest::for_seed(
        "seed-three".into(),
        MutationContext::new(&second),
        different,
    )
    .expect("different dataset request");
    assert!(
        admit(&repo, &second, &origin(&second, "seed-three"), &different)
            .await
            .is_err(),
        "another workspace cannot create a competing global catalog writer"
    );
}

#[tokio::test]
async fn attribution_failure_rolls_back_job_outbox_payload_and_dataset_pin() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "setup_rollback").await;
    initialize(&repo).await;
    enable(&repo).await;
    let context = context(&repo, "192.0.2.55", "one").await;
    let dataset = reference("dataset-one");
    setup_jobs::register_dataset(&repo, &authority(), &dataset, setup_jobs::DATASET_TTL)
        .await
        .expect("validated bytes");
    repo.db().query("DEFINE FIELD OVERWRITE origin_json ON job_origin TYPE string ASSERT $value='reject-fixture'").await.expect("controlled writer failure").check().expect("failure fixture statement");
    let request =
        MaintenanceRequest::for_seed("seed-one".into(), MutationContext::new(&context), dataset)
            .expect("seed request");
    assert!(
        admit(&repo, &context, &origin(&context, "seed-one"), &request)
            .await
            .is_err()
    );
    let mut rows=repo.db().query("SELECT VALUE operation FROM job; SELECT VALUE pool FROM job_dispatch_outbox; SELECT VALUE payload_json FROM setup_operation; SELECT VALUE job_id FROM setup_dataset_pin; SELECT VALUE receipt_json FROM setup_request;").await.expect("rollback reads").check().expect("rollback read statements");
    for index in 0..5 {
        assert!(rows
            .take::<Vec<String>>(index)
            .expect("rolled-back records")
            .is_empty());
    }
}

#[tokio::test]
async fn abandoned_upload_expiry_claims_are_owned_bounded_and_do_not_include_pinned_data() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "setup_expiry").await;
    initialize(&repo).await;
    setup_jobs::begin_upload(
        &repo,
        &authority(),
        "dataset-abandoned",
        DatasetKind::ActorCatalog,
    )
    .await
    .expect("owned partial lease");
    repo.db()
        .query("UPDATE setup_upload SET expires_at=time::now()-1s RETURN NONE")
        .await
        .expect("controlled expiry")
        .check()
        .expect("expiry statement");
    let expired = setup_jobs::claim_expired_uploads(&repo)
        .await
        .expect("expiry lease");
    assert_eq!(expired.len(), 1);
    assert_eq!(expired[0].dataset_id, "dataset-abandoned");
    setup_jobs::finish_upload_cleanup(&repo, "dataset-abandoned")
        .await
        .expect("record completed filesystem cleanup");
    assert!(setup_jobs::claim_expired_uploads(&repo)
        .await
        .expect("idempotent expiry")
        .is_empty());
}
