//! Actual Jobs owner/epoch fencing of attributed canonical seed transactions.
//! Dataset byte/parser fixtures are separate; this trusted Core fixture registers
//! a synthetic validated DatasetRef through the normal storage boundary.
#![cfg(feature = "surreal")]

mod support;

use std::collections::BTreeMap;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use argos_osint_core::actor_catalog::Actor;
use argos_osint_core::storage::actor_catalog;
use argos_osint_core::storage::connect_registry;
use argos_osint_core::storage::connect_registry::RegistryAuthority;
use argos_osint_core::storage::job_dispatch;
use argos_osint_core::storage::jobs::ClaimedTask;
use argos_osint_core::storage::repo::Repository;
use argos_osint_core::storage::setup_jobs;
use argos_osint_core::storage::tx;
use chrono::Utc;
use fleetworks_contracts::connect::RegistrationEnsureRequest;
use fleetworks_contracts::connect::WorkspaceName;
use fleetworks_contracts::context::JobOrigin;
use fleetworks_contracts::context::MutationContext;
use fleetworks_contracts::context::OriginSource;
use fleetworks_contracts::readiness::DatasetKind;
use fleetworks_contracts::readiness::DatasetRef;
use fleetworks_contracts::readiness::MaintenanceOperation;
use fleetworks_contracts::readiness::MaintenanceRequest;
use surrealdb::types::RecordId;

async fn claim_seed(repo: &Repository) -> ClaimedTask {
    argos_osint_core::storage::bootstrap::ensure_network_identity(repo.db())
        .await
        .expect("actual deployment bootstrap");
    connect_registry::initialize_control(repo.db())
        .await
        .expect("registry bootstrap");
    job_dispatch::ensure_queue_transport(repo.db(), setup_jobs::SEED_POOL)
        .await
        .expect("real transport policy");
    job_dispatch::enable_pool(repo.db(), setup_jobs::SEED_POOL)
        .await
        .expect("configured fixture consumer");
    setup_jobs::enable_operation(repo.db(), MaintenanceOperation::SeedActors, true)
        .await
        .expect("explicit operation");
    let authority =
        RegistryAuthority::new("synthetic-scope", "synthetic-native", "synthetic-epoch")
            .expect("bounded authority");
    let registration = connect_registry::ensure(
        repo,
        &authority,
        &RegistrationEnsureRequest::new(
            "ensure-seed-execution".into(),
            "192.0.2.87".parse().expect("synthetic address"),
            WorkspaceName::new("seed-fence").expect("name"),
            "synthetic-install".into(),
            "synthetic-folder".into(),
        ),
    )
    .await
    .expect("actual registration")
    .receipt;
    let context = connect_registry::registration_context(
        repo,
        &authority,
        &registration.registration_id,
        registration.revision,
    )
    .await
    .expect("current proof");
    let dataset = DatasetRef::new(
        "dataset-seed-execution".into(),
        DatasetKind::ActorCatalog,
        "b".repeat(64),
        128,
    )
    .expect("synthetic previously validated reference");
    setup_jobs::register_dataset(repo, &authority, &dataset, setup_jobs::DATASET_TTL)
        .await
        .expect("actual trusted dataset registration");
    let request = MaintenanceRequest::for_seed(
        "seed-execution-fixture".into(),
        MutationContext::new(&context),
        dataset,
    )
    .expect("pinned maintenance request");
    let origin = JobOrigin::new(
        &context,
        OriginSource::Native,
        "seed-execution-fixture".into(),
        Utc::now().to_rfc3339(),
        1,
        Vec::new(),
    );
    tx::run_tx(repo.db(), tx::RetryPolicy::default(), |transaction| {
        let (authority, context, origin, request) = (&authority, &context, &origin, &request);
        async move {
            setup_jobs::admit_in(
                &transaction,
                authority,
                context,
                origin,
                MaintenanceOperation::SeedActors,
                request,
            )
            .await
        }
    })
    .await
    .expect("actual provenance-bearing admission");
    let relay = job_dispatch::claim_ready(
        repo.db(),
        setup_jobs::SEED_POOL,
        "fixture-relay",
        Utc::now(),
    )
    .await
    .expect("relay")
    .expect("durable outbox");
    job_dispatch::mark_published(repo.db(), &relay, Utc::now())
        .await
        .expect("broker handoff");
    let claimed = match job_dispatch::claim_delivery(
        repo.db(),
        &relay.envelope,
        "fixture-consumer",
        300,
        Utc::now(),
        "fixture-claim",
    )
    .await
    .expect("real task lease")
    {
        job_dispatch::DeliveryClaim::Claimed(lease) => lease.task,
        other => panic!("expected actual claim, got {other:?}"),
    };
    claimed
}

fn actors(count: usize) -> Vec<Actor> {
    (0..count)
        .map(|index| Actor {
            id: format!("seed-fence-{index:04}"),
            source_id: format!("synthetic:{index}"),
            name: format!("港 Harbor Union {index}"),
            subtype: "organization".into(),
            aliases: Vec::new(),
            metadata: BTreeMap::new(),
        })
        .collect()
}

#[tokio::test]
async fn captured_seed_execution_rejects_changed_owner_epoch_before_any_source_intent() {
    let Some(server) = support::Server::start() else {
        return;
    };
    let repo = server.repo("seed_execution", "stale").await;
    let task = claim_seed(&repo).await;
    let cancel = AtomicBool::new(false);
    let execution = actor_catalog::SeedExecution::from_claimed(&repo, &task, &cancel)
        .await
        .expect("actual current execution");
    repo.db()
        .query("UPDATE $task SET lease_epoch+=1,lease_owner='synthetic-next-owner' RETURN NONE")
        .bind(("task", RecordId::new("task", task.id.clone())))
        .await
        .expect("adversarial lease takeover")
        .check()
        .expect("takeover statement");
    assert!(actor_catalog::seed_actors_pinned_claimed(
        &repo,
        actors(1),
        &execution,
        &cancel,
        |_| {}
    )
    .await
    .is_err());
    assert_eq!(support::count(repo.db(), "brain_actor").await, 0);
    assert_eq!(support::count(repo.db(), "actor_search_event").await, 0);
    assert_eq!(
        support::count(repo.db(), "brain_actor_seed").await,
        0,
        "stale execution cannot acquire source ownership"
    );
}

#[tokio::test]
async fn genuine_shutdown_after_one_chunk_keeps_only_committed_partial_source_and_outbox() {
    let Some(server) = support::Server::start() else {
        return;
    };
    let repo = server.repo("seed_execution", "cancel_partial").await;
    let task = claim_seed(&repo).await;
    let cancel = AtomicBool::new(false);
    let execution = actor_catalog::SeedExecution::from_claimed(&repo, &task, &cancel)
        .await
        .expect("actual execution");
    assert!(actor_catalog::seed_actors_pinned_claimed(
        &repo,
        actors(129),
        &execution,
        &cancel,
        |progress| {
            if progress.processed == 128 {
                cancel.store(true, Ordering::Release);
            }
        }
    )
    .await
    .is_err());
    assert_eq!(support::count(repo.db(), "brain_actor").await, 128);
    assert_eq!(support::count(repo.db(), "actor_search_event").await, 128);
    let mut response=repo.db().query("SELECT status,processed,total FROM ONLY brain_actor_seed:catalog;SELECT VALUE closed FROM actor_search_cause;SELECT VALUE partial_source FROM actor_search_cause;").await.expect("partial receipts").check().expect("receipt statements");
    let source: Option<setup_jobs::SeedObservation> = response.take(0).expect("source receipt");
    let source = source.expect("committed source receipt");
    assert_eq!(source.status, "failed");
    assert_eq!((source.processed, source.total), (128, 129));
    assert_eq!(
        response.take::<Vec<bool>>(1).expect("closed causal range"),
        [true]
    );
    assert_eq!(
        response
            .take::<Vec<bool>>(2)
            .expect("partial causal receipt"),
        [true]
    );
    assert!(actor_catalog::seed_actors_pinned_claimed(
        &repo,
        actors(1),
        &execution,
        &cancel,
        |_| {}
    )
    .await
    .is_err());
    assert_eq!(
        support::count(repo.db(), "actor_search_event").await,
        128,
        "shutdown cannot publish a following chunk"
    );
}

#[tokio::test]
async fn database_cancellation_and_expired_claim_cannot_publish_even_with_unset_local_flag() {
    let Some(server) = support::Server::start() else {
        return;
    };
    for (name, statement) in [
        (
            "cancelled",
            "UPDATE $job SET cancel_requested=true RETURN NONE",
        ),
        (
            "expired",
            "UPDATE $task SET lease_until=time::now()-1s RETURN NONE",
        ),
    ] {
        let repo = server.repo("seed_execution", name).await;
        let task = claim_seed(&repo).await;
        let cancel = AtomicBool::new(false);
        let execution = actor_catalog::SeedExecution::from_claimed(&repo, &task, &cancel)
            .await
            .expect("original actual lease");
        repo.db()
            .query(statement)
            .bind(("job", RecordId::new("job", task.job_id.clone())))
            .bind(("task", RecordId::new("task", task.id.clone())))
            .await
            .expect("revoked fixture authority")
            .check()
            .expect("revocation statement");
        assert!(actor_catalog::seed_actors_pinned_claimed(
            &repo,
            actors(1),
            &execution,
            &cancel,
            |_| {}
        )
        .await
        .is_err());
        assert_eq!(support::count(repo.db(), "brain_actor").await, 0);
        assert_eq!(support::count(repo.db(), "actor_search_event").await, 0);
    }
}
