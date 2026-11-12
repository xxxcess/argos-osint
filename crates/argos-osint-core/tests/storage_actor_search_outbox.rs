//! Durable actor search intent, source rollback, lease fencing and visible ack.
#![cfg(feature = "surreal")]

mod support;

use std::collections::BTreeMap;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use argos_osint_core::actor_catalog::Actor;
use argos_osint_core::storage::actor_catalog;
use argos_osint_core::storage::actor_search_outbox;
use argos_osint_core::storage::actor_search_outbox::Preparation;
use argos_osint_core::storage::jobs;
use argos_osint_core::storage::memory;
use argos_osint_core::storage::repo::Repository;
use argos_osint_core::storage::tx::run_tx;
use argos_osint_core::storage::tx::RetryPolicy;
use argos_osint_core::storage::tx::TxError;
use chrono::DateTime;
use chrono::Duration;
use chrono::Utc;
use fleetworks_contracts::actor_index::ActorIndexGeneration;
use fleetworks_contracts::actor_index::ActorIndexReceipt;
use fleetworks_contracts::actor_index::ActorIndexStatus;

fn now() -> DateTime<Utc> {
    "2026-11-12T12:00:00Z".parse().expect("fixture time")
}

fn actors(count: usize) -> Vec<Actor> {
    (0..count)
        .map(|index| Actor {
            id: format!("search-fixture-{index:04}"),
            source_id: format!("fixture:{index}"),
            name: format!("研究 Harbor Union {index}"),
            subtype: "organization".into(),
            aliases: vec![format!("港 Alias {index}")],
            metadata: BTreeMap::from([(
                "description".into(),
                "Synthetic offshore freight research".into(),
            )]),
        })
        .collect()
}

async fn status(repo: &Repository) -> ActorIndexStatus {
    // The harness's legacy Server/embedded target applies schema only. Use the
    // actual production initializer before observing deployment identity.
    argos_osint_core::storage::bootstrap::ensure_network_identity(repo.db())
        .await
        .expect("production deployment identity initializer");
    let mut response = repo
        .db()
        .query("SELECT VALUE instance_id FROM ONLY storage_meta:identity")
        .await
        .expect("identity")
        .check()
        .expect("identity statement");
    let deployment: Option<String> = response.take(0).expect("deployment");
    ActorIndexStatus {
        protocol: 1,
        deployment_id: deployment.expect("bootstrapped deployment"),
        source_epoch: "fixture-source".into(),
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
        manifest_checksum: None,
        recovery_available: false,
        limits: Default::default(),
        pressure: None,
        state: "not_built".into(),
        reason_code: None,
    }
}

fn receipt(batch: &fleetworks_contracts::actor_index::ActorIndexBatch) -> ActorIndexReceipt {
    ActorIndexReceipt {
        batch_id: batch.batch_id.clone(),
        checksum: batch.content_checksum().expect("digest"),
        index_generation: batch.index_generation.clone(),
        committed_checkpoint: batch.through_checkpoint,
        visible_checkpoint: batch.through_checkpoint,
        writer_epoch: batch.writer_epoch,
        replayed: false,
    }
}

async fn batch(
    repo: &Repository,
    owner: &str,
    status: &ActorIndexStatus,
    at: DateTime<Utc>,
) -> (
    actor_search_outbox::Lease,
    actor_search_outbox::PreparedBatch,
) {
    assert!(actor_search_outbox::claim(repo, owner, at)
        .await
        .expect("claim")
        .is_some());
    match actor_search_outbox::prepare(repo, owner, status, at)
        .await
        .expect("prepare")
    {
        Preparation::Batch { lease, prepared } => (lease, *prepared),
        Preparation::Waiting => panic!("fixture expected eligible delivery"),
    }
}

#[tokio::test]
async fn source_rollback_leaves_no_projection_intent() {
    let Some(server) = support::Server::start() else {
        return;
    };
    let repo = server.repo("actor_search", "rollback").await;
    let result = run_tx(repo.db(), RetryPolicy::default(), |tx| async move {
        tx.query("CREATE brain_actor:rolled_back SET display_name='Synthetic' RETURN NONE")
            .await?
            .check()?;
        actor_search_outbox::enqueue_actors_in(&tx, &["rolled_back".into()], None, now()).await?;
        Err::<(), _>(TxError::Abort("fixture rollback".into()))
    })
    .await;
    assert!(result.is_err());
    assert_eq!(support::count(repo.db(), "brain_actor").await, 0);
    assert_eq!(support::count(repo.db(), "actor_search_event").await, 0);
    assert_eq!(support::count(repo.db(), "actor_search_head").await, 0);
    assert!(actor_search_outbox::observe(&repo)
        .await
        .expect("read-only status")
        .is_none());
}

#[tokio::test]
async fn attributed_seed_waits_for_exact_reader_visible_ack_and_fences_replicas() {
    let Some(server) = support::Server::start() else {
        return;
    };
    let repo = server.repo("actor_search", "visible").await;
    jobs::create_job(
        repo.db(),
        &jobs::NewJob {
            id: "setup-fixture",
            kind: "maintenance",
            operation: "actor_seed",
            parent: None,
            created_at: now(),
        },
        &[],
    )
    .await
    .expect("parent job");
    actor_catalog::seed_actors_pinned_attributed(
        &repo,
        actors(2),
        "fixture-sha",
        "setup-fixture",
        &AtomicBool::new(false),
        |_| {},
    )
    .await
    .expect("seed");
    let mut status = status(&repo).await;
    let (lease, prepared) = batch(&repo, "leader-a", &status, now()).await;
    assert_eq!(
        prepared.batch.mutations.len(),
        2,
        "source plus reconciliation coalesces canonical actors"
    );
    assert!(prepared.batch.source_complete);
    assert!(actor_search_outbox::claim(&repo, "leader-b", now())
        .await
        .expect("contender")
        .is_none());
    assert!(matches!(
        actor_search_outbox::prepare(
            &repo,
            "leader-a",
            &status,
            now() + Duration::milliseconds(499)
        )
        .await
        .expect("paced"),
        Preparation::Waiting
    ));
    let mut invalid = receipt(&prepared.batch);
    invalid.visible_checkpoint -= 1;
    assert!(
        actor_search_outbox::acknowledge(&repo, &lease, &invalid, now())
            .await
            .is_err()
    );
    assert!(support::count(repo.db(), "actor_search_event").await > 0);
    let mut jobs = repo
        .db()
        .query("SELECT VALUE state FROM job WHERE operation='index_actor_catalog'")
        .await
        .expect("child")
        .check()
        .expect("child statement");
    assert_eq!(
        jobs.take::<Vec<String>>(0).expect("child state"),
        vec!["running"]
    );
    let (replacement, replay) =
        batch(&repo, "leader-b", &status, now() + Duration::seconds(46)).await;
    assert_eq!(
        replay.batch.content_checksum().expect("digest"),
        prepared.batch.content_checksum().expect("digest")
    );
    assert!(replacement.epoch > lease.epoch);
    assert!(actor_search_outbox::acknowledge(
        &repo,
        &lease,
        &receipt(&prepared.batch),
        now() + Duration::seconds(46)
    )
    .await
    .is_err());
    actor_search_outbox::acknowledge(
        &repo,
        &replacement,
        &receipt(&replay.batch),
        now() + Duration::seconds(46),
    )
    .await
    .expect("visible acknowledgment");
    assert_eq!(support::count(repo.db(), "actor_search_event").await, 0);
    let mut jobs = repo
        .db()
        .query("SELECT VALUE state FROM job WHERE operation='index_actor_catalog'")
        .await
        .expect("child")
        .check()
        .expect("child statement");
    assert_eq!(
        jobs.take::<Vec<String>>(0).expect("completed child"),
        vec!["completed"]
    );
    assert_eq!(
        support::count(repo.db(), "job_origin").await,
        0,
        "unattributed parent must not gain a fabricated workstation"
    );
    status.active.as_mut().expect("active").visible_checkpoint = replay.batch.through_checkpoint;
    status.active.as_mut().expect("active").committed_checkpoint = replay.batch.through_checkpoint;
    status.active.as_mut().expect("active").writer_epoch = replacement.epoch;
    status.active.as_mut().expect("active").source_complete = true;
    assert!(matches!(
        actor_search_outbox::prepare(&repo, "leader-b", &status, now() + Duration::seconds(47))
            .await
            .expect("caught up"),
        Preparation::Waiting
    ));
}

#[tokio::test]
async fn partial_seed_noop_retry_and_lifecycle_have_monotonic_fences() {
    let Some(server) = support::Server::start() else {
        return;
    };
    let repo = server.repo("actor_search", "partial").await;
    let cancel = AtomicBool::new(false);
    assert!(actor_catalog::seed_actors_pinned(
        &repo,
        actors(129),
        "fixture-sha",
        &cancel,
        |progress| {
            if progress.processed == 128 {
                cancel.store(true, Ordering::Release);
            }
        }
    )
    .await
    .is_err());
    let (_, partial) = batch(&repo, "leader", &status(&repo).await, now()).await;
    assert!(
        !partial.batch.source_complete,
        "committed partial catalog is never claimed complete"
    );
    cancel.store(false, Ordering::Release);
    actor_catalog::seed_actors_pinned(&repo, actors(129), "fixture-sha", &cancel, |_| {})
        .await
        .expect("source retry");
    let ids = vec!["search-fixture-0000".to_owned()];
    let before = actor_search_outbox::canonical_many(&repo, &ids)
        .await
        .expect("canonical");
    assert!(
        before[0].revision >= 2,
        "no-op existing source actors still reconcile indexing intent"
    );
    run_tx(repo.db(), RetryPolicy::default(), |tx| {
        let ids = ids.clone();
        async move {
            memory::set_lifecycle_in(&tx, &ids, "retired").await?;
            Ok(())
        }
    })
    .await
    .expect("retire");
    let after = actor_search_outbox::canonical_many(&repo, &ids)
        .await
        .expect("canonical retired");
    assert!(after[0].revision > before[0].revision);
    assert!(
        after[0].projection.is_none(),
        "retirement has a persisted revisioned tombstone"
    );
}

#[tokio::test]
async fn pending_body_survives_disk_reopen_and_long_retry_after_pauses_without_shortening() {
    let Some(mut server) = support::Server::start() else {
        return;
    };
    let repo = server.repo("actor_search", "restart").await;
    actor_catalog::seed_actors_pinned(
        &repo,
        actors(1),
        "fixture-sha",
        &AtomicBool::new(false),
        |_| {},
    )
    .await
    .expect("seed");
    let status = status(&repo).await;
    let (lease, prepared) = batch(&repo, "before-restart", &status, now()).await;
    drop(repo);
    server.restart();
    let repo = server.repo("actor_search", "restart").await;
    let (lease2, replay) = batch(
        &repo,
        "after-restart",
        &status,
        now() + Duration::seconds(46),
    )
    .await;
    assert_eq!(
        prepared.batch.content_checksum().expect("digest"),
        replay.batch.content_checksum().expect("replay digest")
    );
    assert!(lease2.epoch > lease.epoch);
    actor_search_outbox::defer(
        &repo,
        &lease2,
        Some(std::time::Duration::from_secs(86_400)),
        false,
        now() + Duration::seconds(46),
    )
    .await
    .expect("durable Retry-After");
    assert!(
        actor_search_outbox::claim(&repo, "another", now() + Duration::hours(23))
            .await
            .expect("delay")
            .is_none()
    );
    assert!(
        actor_search_outbox::claim(&repo, "another", now() + Duration::hours(25))
            .await
            .expect("after delay")
            .is_some()
    );
}
