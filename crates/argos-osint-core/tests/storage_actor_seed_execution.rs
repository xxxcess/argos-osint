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
use fleetworks_contracts::context::JobOrigin;
use fleetworks_contracts::context::MutationContext;
use fleetworks_contracts::context::OriginSource;
use fleetworks_contracts::readiness::DatasetKind;
use fleetworks_contracts::readiness::DatasetRef;
use fleetworks_contracts::readiness::MaintenanceOperation;
use fleetworks_contracts::readiness::MaintenanceRequest;
use surrealdb::types::RecordId;

async fn claim_seed(repo: &Repository) -> ClaimedTask {
    claim_seed_dataset(repo, "fixture", 'b').await.task
}

async fn claim_seed_dataset(
    repo: &Repository,
    label: &str,
    hash: char,
) -> job_dispatch::DeliveryLease {
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
            Some("192.0.2.87".parse().expect("synthetic address")),
            "synthetic-install".into(),
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
        format!("dataset-seed-execution-{label}"),
        DatasetKind::ActorCatalog,
        hash.to_string().repeat(64),
        128,
    )
    .expect("synthetic previously validated reference");
    setup_jobs::register_dataset(repo, &authority, &dataset, setup_jobs::DATASET_TTL)
        .await
        .expect("actual trusted dataset registration");
    let request = MaintenanceRequest::for_seed(
        format!("seed-execution-{label}"),
        MutationContext::new(&context),
        dataset,
    )
    .expect("pinned maintenance request");
    let origin = JobOrigin::new(
        &context,
        OriginSource::Native,
        format!("seed-execution-{label}"),
        Utc::now().to_rfc3339(),
        1,
        Vec::new(),
    );
    let admitted = tx::run_tx(repo.db(), tx::RetryPolicy::default(), |transaction| {
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
    .expect("actual provenance-bearing admission")
    .value;
    let replay = tx::run_tx(repo.db(), tx::RetryPolicy::default(), |transaction| {
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
    .expect("exact admitted request replay")
    .value;
    assert_eq!(
        replay.job_id, admitted.job_id,
        "same request never creates another job"
    );
    assert_eq!(
        replay, admitted,
        "exact replay retains the original receipt"
    );
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
        &format!("fixture-claim-{label}"),
    )
    .await
    .expect("real task lease")
    {
        job_dispatch::DeliveryClaim::Claimed(lease) => *lease,
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

#[tokio::test]
async fn streamed_claimed_imports_merge_selected_identities_and_keep_other_actors() {
    let Some(server) = support::Server::start() else {
        return;
    };
    let repo = server.repo("seed_execution", "stream_merge").await;
    let cancel = AtomicBool::new(false);
    let first = claim_seed_dataset(&repo, "first", 'a').await;
    let execution = actor_catalog::SeedExecution::from_claimed(&repo, &first.task, &cancel)
        .await
        .expect("first actual execution");
    let mut initial = actors(2);
    initial[0].aliases = (0..258)
        .map(|index| format!("Historical Harbor Alias {index}"))
        .collect();
    initial[0].metadata = (0..40)
        .map(|index| (format!("field-{index}"), "retained source value".repeat(4)))
        .collect();
    let original = initial[0].clone();
    let first_progress = actor_catalog::seed_actors_stream_claimed(
        &repo,
        2,
        futures_util::stream::iter([Ok(initial)]),
        &execution,
        &cancel,
        |_| {},
    )
    .await
    .expect("first streamed import");
    assert!(first_progress.complete);
    assert_eq!(
        support::count(repo.db(), "brain_actor_alias").await,
        260,
        "all 258 explicit aliases and both canonical names persist"
    );
    assert!(matches!(
        job_dispatch::finish_delivery(
            repo.db(),
            &first,
            job_dispatch::DeliveryOutcome::Complete {
                result_ref: "synthetic-first-seed"
            },
            Utc::now()
        )
        .await
        .expect("first task completes"),
        job_dispatch::DeliveryFinish::Completed
    ));
    let second = claim_seed_dataset(&repo, "second", 'c').await;
    let second_execution = actor_catalog::SeedExecution::from_claimed(&repo, &second.task, &cancel)
        .await
        .expect("new exact claimed dataset");
    let mut selected = actors(3);
    selected[0].name = "Updated Harbor Union".into();
    selected[0]
        .metadata
        .insert("updated".into(), "new selected source".into());
    let updated = selected[0].clone();
    let added = selected.pop().expect("third actor");
    actor_catalog::seed_actors_stream_claimed(
        &repo,
        2,
        futures_util::stream::iter([Ok(vec![updated.clone(), added.clone()])]),
        &second_execution,
        &cancel,
        |_| {},
    )
    .await
    .expect("new CSV merges selected IDs");
    assert_eq!(
        support::count(repo.db(), "brain_actor").await,
        3,
        "absent identities are retained"
    );
    let mut rows = repo.db().query("SELECT VALUE catalog_json FROM brain_actor ORDER BY id; RETURN (SELECT VALUE fingerprint FROM ONLY brain_actor_seed:catalog);")
        .await.expect("merged catalog").check().expect("catalog statements");
    let catalog: Vec<String> = rows.take(0).expect("catalog JSON");
    let catalog: Vec<Actor> = catalog
        .iter()
        .map(|value| serde_json::from_str(value).expect("typed actor"))
        .collect();
    assert_eq!(
        catalog.iter().find(|actor| actor.id == updated.id),
        Some(&updated)
    );
    assert_eq!(
        catalog.iter().find(|actor| actor.id == added.id),
        Some(&added)
    );
    assert_eq!(
        catalog.iter().find(|actor| actor.id == "seed-fence-0001"),
        Some(&actors(2)[1])
    );
    assert_eq!(
        rows.take::<Option<String>>(1)
            .expect("latest import fingerprint"),
        Some("c".repeat(64))
    );
    assert_eq!(original.aliases.len(), 258);
    assert_eq!(
        support::count(repo.db(), "brain_actor_alias").await,
        262,
        "additive import preserves existing aliases and adds current names"
    );
    job_dispatch::finish_delivery(
        repo.db(),
        &second,
        job_dispatch::DeliveryOutcome::Complete {
            result_ref: "synthetic-second-seed",
        },
        Utc::now(),
    )
    .await
    .expect("second task completes");
    let events = support::count(repo.db(), "actor_search_event").await;
    assert!(
        actor_catalog::seed_actors_stream_claimed(
            &repo,
            1,
            futures_util::stream::iter([Ok(vec![updated])]),
            &second_execution,
            &cancel,
            |_| {}
        )
        .await
        .is_err(),
        "completed task cannot replay source writes"
    );
    assert_eq!(
        support::count(repo.db(), "actor_search_event").await,
        events
    );
}

#[tokio::test]
async fn streamed_source_error_preserves_only_committed_batches_and_refuses_completion() {
    let Some(server) = support::Server::start() else {
        return;
    };
    let repo = server.repo("seed_execution", "stream_error").await;
    let task = claim_seed(&repo).await;
    let cancel = AtomicBool::new(false);
    let execution = actor_catalog::SeedExecution::from_claimed(&repo, &task, &cancel)
        .await
        .expect("actual execution");
    let batches = futures_util::stream::iter([
        Ok(actors(1)),
        Err(anyhow::anyhow!("synthetic producer failure")),
    ]);
    assert!(actor_catalog::seed_actors_stream_claimed(
        &repo,
        2,
        batches,
        &execution,
        &cancel,
        |_| {}
    )
    .await
    .is_err());
    assert_eq!(support::count(repo.db(), "brain_actor").await, 1);
    let mut response = repo
        .db()
        .query("SELECT status,processed,total FROM ONLY brain_actor_seed:catalog")
        .await
        .expect("source observation")
        .check()
        .expect("source query");
    let source: Option<setup_jobs::SeedObservation> = response.take(0).expect("typed receipt");
    let source = source.expect("source exists");
    assert_eq!(
        (source.status.as_str(), source.processed, source.total),
        ("failed", 1, 2)
    );
}
