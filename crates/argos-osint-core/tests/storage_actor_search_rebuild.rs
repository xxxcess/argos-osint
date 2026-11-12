//! Canonical DB shadow scan/catch-up/restart fences; engine disk semantics have
//! separate real Tantivy tests. Receipts here model that authenticated boundary.
#![cfg(feature = "surreal")]

mod support;

use std::collections::BTreeMap;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use argos_osint_core::actor_catalog::Actor;
use argos_osint_core::storage::actor_catalog;
use argos_osint_core::storage::actor_search_outbox as index;
use argos_osint_core::storage::connect_registry;
use argos_osint_core::storage::connect_registry::RegistryAuthority;
use argos_osint_core::storage::job_dispatch;
use argos_osint_core::storage::jobs::ClaimedTask;
use argos_osint_core::storage::memory;
use argos_osint_core::storage::repo::Repository;
use argos_osint_core::storage::setup_jobs;
use argos_osint_core::storage::tx;
use chrono::Duration;
use chrono::Utc;
use fleetworks_contracts::actor_index::ActorIndexBatch;
use fleetworks_contracts::actor_index::ActorIndexGeneration;
use fleetworks_contracts::actor_index::ActorIndexPressure;
use fleetworks_contracts::actor_index::ActorIndexReceipt;
use fleetworks_contracts::actor_index::ActorIndexStatus;
use fleetworks_contracts::connect::RegistrationEnsureRequest;
use fleetworks_contracts::connect::WorkspaceName;
use fleetworks_contracts::context::JobOrigin;
use fleetworks_contracts::context::MutationContext;
use fleetworks_contracts::context::OriginSource;
use fleetworks_contracts::readiness::MaintenanceOperation;
use fleetworks_contracts::readiness::MaintenanceRequest;

async fn fixture(repo: &Repository) -> (ClaimedTask, ActorIndexStatus) {
    argos_osint_core::storage::bootstrap::ensure_network_identity(repo.db())
        .await
        .expect("actual deployment bootstrap");
    connect_registry::initialize_control(repo.db())
        .await
        .expect("registry bootstrap");
    job_dispatch::ensure_queue_transport(repo.db(), setup_jobs::REINDEX_POOL)
        .await
        .expect("real transport policy");
    job_dispatch::enable_pool(repo.db(), setup_jobs::REINDEX_POOL)
        .await
        .expect("configured fixture consumer");
    setup_jobs::enable_operation(repo.db(), MaintenanceOperation::ReindexActors, true)
        .await
        .expect("explicit operation");
    let authority =
        RegistryAuthority::new("synthetic-scope", "synthetic-native", "synthetic-epoch")
            .expect("bounded authority");
    let registration = connect_registry::ensure(
        repo,
        &authority,
        &RegistrationEnsureRequest::new(
            "ensure-shadow".into(),
            "192.0.2.87".parse().expect("synthetic address"),
            WorkspaceName::new("shadow-fixture").expect("name"),
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
    let request =
        MaintenanceRequest::for_reindex("rebuild-fixture".into(), MutationContext::new(&context))
            .expect("canonical DB rebuild request");
    let origin = JobOrigin::new(
        &context,
        OriginSource::Native,
        "rebuild-fixture".into(),
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
                MaintenanceOperation::ReindexActors,
                request,
            )
            .await
        }
    })
    .await
    .expect("actual provenance-bearing admission");
    let relay = job_dispatch::claim_ready(
        repo.db(),
        setup_jobs::REINDEX_POOL,
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
    let mut response = repo
        .db()
        .query("SELECT VALUE instance_id FROM ONLY storage_meta:identity")
        .await
        .expect("identity")
        .check()
        .expect("identity statement");
    let deployment: Option<String> = response.take(0).expect("identity decode");
    (
        claimed,
        ActorIndexStatus {
            protocol: 1,
            deployment_id: deployment.expect("production identity"),
            source_epoch: "synthetic-disk-source".into(),
            schema_version: 1,
            analyzer_version: 1,
            tantivy_version: "0.27.0".into(),
            index_format_version: 7,
            active: Some(ActorIndexGeneration {
                generation: "g1".into(),
                committed_checkpoint: 0,
                visible_checkpoint: 0,
                source_complete: false,
                writer_epoch: 0,
            }),
            rebuilding: None,
            retained_generations: 0,
            retained_generation_ids: Vec::new(),
            manifest_checksum: Some("a".repeat(64)),
            recovery_available: false,
            limits: Default::default(),
            pressure: Some(ActorIndexPressure {
                resident_bytes: Some(64 * 1024 * 1024),
                disk_free_bytes: Some(1024 * 1024 * 1024),
                query_ewma_ms: 1,
                commit_ewma_ms: 1,
                paused: false,
                reason_code: None,
                observed_at_unix_ms: Utc::now().timestamp_millis() as u64,
            }),
            state: "not_built".into(),
            reason_code: None,
        },
    )
}

fn actors(count: usize) -> Vec<Actor> {
    (0..count)
        .map(|number| Actor {
            id: format!("shadow-actor-{number:04}"),
            source_id: format!("fixture:{number}"),
            name: format!("港 Harbor {number}"),
            subtype: "organization".into(),
            aliases: vec![format!("研究 {number}")],
            metadata: BTreeMap::new(),
        })
        .collect()
}

fn receipt(batch: &ActorIndexBatch) -> ActorIndexReceipt {
    ActorIndexReceipt {
        batch_id: batch.batch_id.clone(),
        checksum: batch.content_checksum().expect("canonical checksum"),
        index_generation: batch.index_generation.clone(),
        committed_checkpoint: batch.through_checkpoint,
        visible_checkpoint: batch.through_checkpoint,
        writer_epoch: batch.writer_epoch,
        replayed: false,
    }
}

#[tokio::test]
async fn canonical_shadow_restarts_exact_pending_then_promotes_only_verified_visible_barrier() {
    let Some(mut server) = support::Server::start() else {
        return;
    };
    let repo = server.repo("search_shadow", "restart").await;
    actor_catalog::seed_actors_pinned(
        &repo,
        actors(129),
        "synthetic-catalog",
        &AtomicBool::new(false),
        |_| {},
    )
    .await
    .expect("canonical seed");
    let (claimed, mut status) = fixture(&repo).await;
    index::request_rebuild(&repo, &claimed)
        .await
        .expect("real consumer submits durable rebuild");
    let mut at = Utc::now();
    let start = index::next_rebuild(&repo, "pump-before-restart", &status, at)
        .await
        .expect("start intent")
        .expect("rebuild owns pump");
    let index::RebuildAction::Start { lease, request } = start else {
        panic!("expected start")
    };
    assert_eq!(request.source_checkpoint, 0);
    let generation = ActorIndexGeneration {
        generation: "g2".into(),
        committed_checkpoint: 0,
        visible_checkpoint: 0,
        writer_epoch: lease.epoch,
        source_complete: false,
    };
    index::rebuild_started(&repo, &lease, &generation, at)
        .await
        .expect("verified start");
    status.rebuilding = Some(generation);
    at += Duration::seconds(1);
    let index::RebuildAction::Deliver {
        lease: old_lease,
        batch: pending,
    } = index::next_rebuild(&repo, "pump-before-restart", &status, at)
        .await
        .expect("first scan")
        .expect("scan")
    else {
        panic!("expected scan")
    };
    assert_eq!(pending.mutations.len(), 128);
    assert_eq!(pending.from_checkpoint, pending.through_checkpoint);
    drop(repo);
    server.restart();
    let repo = server.repo("search_shadow", "restart").await;
    at += Duration::seconds(46);
    let index::RebuildAction::Deliver {
        lease,
        batch: replay,
    } = index::next_rebuild(&repo, "pump-after-restart", &status, at)
        .await
        .expect("replay")
        .expect("pending scan")
    else {
        panic!("expected exact scan replay")
    };
    assert_eq!(
        pending.content_checksum().expect("pending checksum"),
        replay.content_checksum().expect("replay checksum")
    );
    assert!(lease.epoch > old_lease.epoch);
    assert!(
        index::rebuild_acknowledge(&repo, &old_lease, &receipt(&pending), at)
            .await
            .is_err()
    );
    let mut invalid = receipt(&replay);
    invalid.visible_checkpoint += 1;
    assert!(index::rebuild_acknowledge(&repo, &lease, &invalid, at)
        .await
        .is_err());
    index::rebuild_acknowledge(&repo, &lease, &receipt(&replay), at)
        .await
        .expect("visible scan receipt");
    status.rebuilding.as_mut().expect("shadow").writer_epoch = lease.epoch;
    // A concurrent lifecycle producer must become a revisioned catch-up
    // tombstone rather than resurrecting the earlier scan's active actor.
    tx::run_tx(
        repo.db(),
        tx::RetryPolicy::default(),
        |transaction| async move {
            memory::set_lifecycle_in(&transaction, &["shadow-actor-0000".into()], "retired")
                .await?;
            Ok(())
        },
    )
    .await
    .expect("canonical concurrent producer");
    let mut saw_tombstone = false;
    let mut promoted = false;
    for _ in 0..12 {
        at += Duration::seconds(1);
        match index::next_rebuild(&repo, "pump-after-restart", &status, at)
            .await
            .expect("bounded turn")
            .expect("active rebuild")
        {
            index::RebuildAction::Deliver { lease, batch } => {
                saw_tombstone |= batch
                    .mutations
                    .iter()
                    .any(|row| row.actor_id == "shadow-actor-0000" && row.projection.is_none());
                let receipt = receipt(&batch);
                index::rebuild_acknowledge(&repo, &lease, &receipt, at)
                    .await
                    .expect("visible catch-up");
                let shadow = status.rebuilding.as_mut().expect("shadow");
                shadow.committed_checkpoint = receipt.committed_checkpoint;
                shadow.visible_checkpoint = receipt.visible_checkpoint;
                shadow.writer_epoch = receipt.writer_epoch;
                shadow.source_complete = batch.source_complete;
                assert_eq!(
                    index::observe(&repo)
                        .await
                        .expect("serving observation")
                        .expect("source")
                        .acknowledged_sequence,
                    0,
                    "shadow acknowledgement cannot advance serving cursor"
                );
            }
            index::RebuildAction::Promote { lease, request } => {
                let mut accepted = status.rebuilding.clone().expect("shadow");
                accepted.writer_epoch = lease.epoch;
                let mut wrong = accepted.clone();
                wrong.source_complete = false;
                assert!(index::rebuild_promoted(&repo, &lease, &wrong, at)
                    .await
                    .is_err());
                assert_eq!(request.catchup_checkpoint, 130);
                index::rebuild_promoted(&repo, &lease, &accepted, at)
                    .await
                    .expect("reader switch receipt");
                promoted = true;
                break;
            }
            _ => panic!("unexpected shadow transition"),
        }
    }
    assert!(saw_tombstone);
    assert!(promoted);
    let source = index::observe(&repo)
        .await
        .expect("source receipt")
        .expect("source");
    assert_eq!(source.index_generation, "g2");
    assert_eq!(source.acknowledged_sequence, 130);
    assert_eq!(
        index::observe_rebuild(&repo, &claimed.job_id)
            .await
            .expect("job progress")
            .expect("request")
            .phase,
        "completed"
    );
    let mut origin=repo.db().query("SELECT VALUE workstation_id FROM job_origin;SELECT VALUE workstation_id FROM job_request").await.expect("provenance").check().expect("provenance statements");
    assert_eq!(origin.take::<Vec<String>>(0).expect("origin").len(), 1);
    assert_eq!(
        origin
            .take::<Vec<String>>(1)
            .expect("request association")
            .len(),
        1
    );
}

#[tokio::test]
async fn pressure_observation_is_fenced_and_source_pause_cancels_without_losing_intent() {
    let Some(server) = support::Server::start() else {
        return;
    };
    let repo = server.repo("search_shadow", "pressure").await;
    let (_, mut status) = fixture(&repo).await;
    let now = Utc::now();
    let lease = index::claim(&repo, "elected", now)
        .await
        .expect("lease")
        .expect("coordinator");
    status.pressure.as_mut().expect("measurement").paused = true;
    status.pressure.as_mut().expect("measurement").reason_code = Some("disk_low".into());
    index::observe_resources(&repo, &lease, &status, now)
        .await
        .expect("real measured resource receipt");
    let mut stale = lease.clone();
    stale.epoch += 1;
    assert!(index::observe_resources(&repo, &stale, &status, now)
        .await
        .is_err());
    let cancel = AtomicBool::new(true);
    assert!(
        index::await_bulk_capacity(&repo, "no-source-owner", &cancel)
            .await
            .is_err()
    );
    assert_eq!(support::count(repo.db(), "actor_search_event").await, 0);
    let cancel = Arc::new(AtomicBool::new(false));
    let pending = tokio::spawn({
        let repo = repo.clone();
        let cancel = Arc::clone(&cancel);
        async move {
            Box::pin(actor_catalog::seed_actors_pinned(
                &repo,
                actors(1),
                "synthetic-catalog",
                &cancel,
                |_| {},
            ))
            .await
        }
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let mut response = repo
                .db()
                .query("SELECT VALUE index_pause_reason FROM ONLY brain_actor_seed:catalog")
                .await
                .expect("observable pause")
                .check()
                .expect("pause statement");
            if response
                .take::<Option<String>>(0)
                .expect("pause reason")
                .as_deref()
                == Some("disk_low")
            {
                break;
            }
            assert!(
                !pending.is_finished(),
                "seed must not commit while measured pressure pauses bulk work"
            );
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("source entered actual pressure pause");
    assert_eq!(support::count(repo.db(), "actor_search_event").await, 0);
    cancel.store(true, Ordering::Release);
    assert!(pending.await.expect("joined source operation").is_err());
    status.pressure.as_mut().expect("measurement").paused = false;
    status.pressure.as_mut().expect("measurement").reason_code = None;
    index::observe_resources(&repo, &lease, &status, now)
        .await
        .expect("low watermark receipt");
    actor_catalog::seed_actors_pinned(
        &repo,
        actors(1),
        "synthetic-catalog",
        &AtomicBool::new(false),
        |_| {},
    )
    .await
    .expect("bulk resumes under measured capacity");
    assert_eq!(
        support::count(repo.db(), "actor_search_event").await,
        1,
        "source commit retains derived intent"
    );
}

#[tokio::test]
async fn direct_typed_actor_visibility_write_advances_tombstone_fence_in_same_transaction() {
    let Some(server) = support::Server::start() else {
        return;
    };
    let repo = server.repo("search_shadow", "typed_visibility").await;
    actor_catalog::seed_actors_pinned(
        &repo,
        actors(1),
        "synthetic-catalog",
        &AtomicBool::new(false),
        |_| {},
    )
    .await
    .expect("seed");
    let ids = vec!["shadow-actor-0000".to_owned()];
    let before = index::canonical_many(&repo, &ids)
        .await
        .expect("active projection");
    assert!(before[0].projection.is_some());
    tx::run_tx(
        repo.db(),
        tx::RetryPolicy::default(),
        |transaction| async move {
            memory::put_typed_in(
                &transaction,
                &memory::TypedMemory {
                    id: "shadow-actor-0000".into(),
                    text: "Synthetic retired actor".into(),
                    kind: argos_osint_core::memory_metadata::MemoryKind::Actor,
                    payload: None,
                    lifecycle: "retired".into(),
                },
            )
            .await?;
            Ok(())
        },
    )
    .await
    .expect("real generic actor writer");
    let after = index::canonical_many(&repo, &ids)
        .await
        .expect("current tombstone");
    assert!(after[0].projection.is_none());
    assert!(after[0].revision > before[0].revision);
    assert_eq!(support::count(repo.db(), "actor_search_event").await, 2);
}

#[tokio::test]
async fn oversized_alias_identity_stays_unresolved_and_keeps_source_intent() {
    let Some(server) = support::Server::start() else {
        return;
    };
    let repo = server.repo("search_shadow", "identity_bound").await;
    let mut data = actors(1);
    data[0].aliases = (0..32)
        .map(|number| format!("Distinct alias {number}"))
        .collect();
    actor_catalog::seed_actors_pinned(
        &repo,
        data,
        "synthetic-catalog",
        &AtomicBool::new(false),
        |_| {},
    )
    .await
    .expect("canonical source commit");
    assert!(
        index::canonical_many(&repo, &["shadow-actor-0000".into()])
            .await
            .is_err(),
        "33 names including canonical name cannot be silently truncated to 32"
    );
    assert_eq!(
        support::count(repo.db(), "actor_search_event").await,
        1,
        "unresolved identity retains durable intent"
    );
}
