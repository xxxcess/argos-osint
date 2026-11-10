use super::{Envelope, MAX_MESSAGE_BYTES};

#[test]
fn delivery_wire_rejects_unversioned_or_non_identity_payloads() {
    let valid = br#"{"version":1,"workspace_generation":"store-1","job_id":"atlas-job","task_id":"atlas-task","pool":"atlas_cycle","revision":0,"correlation_id":"request-1"}"#;
    let envelope = Envelope::decode(valid).unwrap();
    assert_eq!(envelope.job_id, "atlas-job");
    for invalid in [
        String::from_utf8_lossy(valid).replace("\"version\":1", "\"version\":2"),
        String::from_utf8_lossy(valid).replace("\"revision\":0", "\"revision\":-1"),
        String::from_utf8_lossy(valid)
            .replace("\"pool\":\"atlas_cycle\"", "\"pool\":\"wrong pool\""),
        String::from_utf8_lossy(valid).replace("\"job_id\"", "\"secret\""),
    ] {
        assert!(Envelope::decode(invalid.as_bytes()).is_err());
    }
    assert!(Envelope::decode(&vec![b' '; MAX_MESSAGE_BYTES + 1]).is_err());
}

#[cfg(feature = "embedded")]
#[tokio::test]
async fn published_deliveries_recover_and_fence_stale_workers() {
    use chrono::{Duration, Utc};

    use super::{
        claim_delivery, claim_ready, configure_pool, enable_pool, finish_delivery, mark_published,
        recover_expired, renew_delivery, retry_failed_atlas, submit, DeliveryClaim, DeliveryFinish,
        DeliveryOutcome, ATLAS_POOL,
    };
    use crate::storage::bootstrap;
    use crate::storage::jobs::{NewJob, NewTask};
    use crate::storage::open::Target;
    use crate::storage::repo::Repository;

    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::connect(
        &Target::Embedded {
            path: dir.path().join("db"),
        },
        "fixture",
        "fixture",
    )
    .await
    .unwrap();
    let db = repo.db();
    bootstrap::ensure_network_identity(db).await.unwrap();
    let now = Utc::now();
    let job = NewJob {
        id: "atlas-job",
        kind: "task",
        operation: "atlas_cycle",
        parent: None,
        created_at: now,
    };
    let task = NewTask {
        id: "atlas-task",
        job: job.id,
        pool: ATLAS_POOL,
        operation: job.operation,
        priority: 100,
        deps: vec![],
        input_hash: "run-revision",
        created_at: now,
    };
    assert!(submit(db, &job, &task).await.is_err());
    enable_pool(db, ATLAS_POOL).await.unwrap();
    assert!(submit(db, &job, &task).await.unwrap());
    assert!(!submit(db, &job, &task).await.unwrap());
    let conflicting = NewTask {
        input_hash: "different-revision",
        ..task.clone()
    };
    assert!(submit(db, &job, &conflicting).await.is_err());

    let first = claim_ready(db, ATLAS_POOL, "relay-1", now)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first.envelope.revision, 0);
    assert!(claim_ready(db, ATLAS_POOL, "relay-2", now)
        .await
        .unwrap()
        .is_none());

    let after_relay_expiry = now + Duration::seconds(31);
    let relay = claim_ready(db, ATLAS_POOL, "relay-2", after_relay_expiry)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(relay.envelope, first.envelope);
    assert!(!mark_published(db, &first, after_relay_expiry)
        .await
        .unwrap());
    assert!(mark_published(db, &relay, after_relay_expiry)
        .await
        .unwrap());

    let mut stale_generation = relay.envelope.clone();
    stale_generation.workspace_generation = "old-store".into();
    assert!(matches!(
        claim_delivery(
            db,
            &stale_generation,
            "worker-1",
            5,
            after_relay_expiry,
            "old-claim"
        )
        .await
        .unwrap(),
        DeliveryClaim::Stale
    ));
    let DeliveryClaim::Claimed(old) = claim_delivery(
        db,
        &relay.envelope,
        "worker-1",
        5,
        after_relay_expiry,
        "claim-1",
    )
    .await
    .unwrap() else {
        panic!("expected the published task to be claimable");
    };
    assert!(matches!(
        claim_delivery(
            db,
            &relay.envelope,
            "worker-2",
            5,
            after_relay_expiry,
            "duplicate"
        )
        .await
        .unwrap(),
        DeliveryClaim::Stale
    ));
    let expired = after_relay_expiry + Duration::seconds(6);
    assert!(!renew_delivery(db, &old, 5, expired).await.unwrap());
    configure_pool(db, ATLAS_POOL, false).await.unwrap();
    assert_eq!(recover_expired(db, ATLAS_POOL, expired).await.unwrap(), 1);
    assert!(claim_ready(db, ATLAS_POOL, "relay-2", expired)
        .await
        .unwrap()
        .is_none());
    enable_pool(db, ATLAS_POOL).await.unwrap();
    let recovered = claim_ready(db, ATLAS_POOL, "relay-2", expired)
        .await
        .unwrap()
        .unwrap();
    assert!(recovered.envelope.revision > first.envelope.revision);
    let DeliveryClaim::Claimed(active) =
        claim_delivery(db, &recovered.envelope, "worker-2", 5, expired, "claim-2")
            .await
            .unwrap()
    else {
        panic!("expected the recovered revision to be claimable before publisher confirm");
    };
    assert_eq!(active.task.attempts, old.task.attempts + 1);
    let result = DeliveryOutcome::Complete {
        result_ref: "atlas-run",
    };
    assert_eq!(
        finish_delivery(db, &old, result, expired).await.unwrap(),
        DeliveryFinish::Stale
    );
    assert_eq!(
        finish_delivery(
            db,
            &active,
            DeliveryOutcome::Failed {
                reason: "offline_fixture",
            },
            expired,
        )
        .await
        .unwrap(),
        DeliveryFinish::Failed
    );
    configure_pool(db, ATLAS_POOL, false).await.unwrap();
    assert!(!retry_failed_atlas(db, job.id, task.id, expired)
        .await
        .unwrap());
    enable_pool(db, ATLAS_POOL).await.unwrap();
    assert!(retry_failed_atlas(db, job.id, task.id, expired)
        .await
        .unwrap());
    assert!(!retry_failed_atlas(db, job.id, task.id, expired)
        .await
        .unwrap());
    let retry = claim_ready(db, ATLAS_POOL, "relay-2", expired)
        .await
        .unwrap()
        .unwrap();
    assert!(retry.envelope.revision > recovered.envelope.revision);
    let DeliveryClaim::Claimed(retried) =
        claim_delivery(db, &retry.envelope, "worker-3", 5, expired, "claim-3")
            .await
            .unwrap()
    else {
        panic!("expected the manually retried revision to be claimable");
    };
    assert_eq!(retried.task.attempts, active.task.attempts + 1);
    assert_eq!(
        finish_delivery(db, &retried, result, expired)
            .await
            .unwrap(),
        DeliveryFinish::Completed
    );
    assert!(matches!(
        claim_delivery(db, &retry.envelope, "worker-3", 5, expired, "after-commit")
            .await
            .unwrap(),
        DeliveryClaim::Stale
    ));
    let row = crate::storage::jobs_view::get_job(db, job.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.state, "completed");
    assert_eq!(row.result_ref, "atlas-run");
}
