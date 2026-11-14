use fleetworks_jobs::queue::{
    Delivery, Queue, QueueCommand, QueueRequest, QueueResponse, MAX_PAYLOAD_BYTES,
};

fn publish(id: &str) -> QueueCommand {
    QueueCommand::Publish {
        queue: "atlas".into(),
        id: id.into(),
        payload: format!("synthetic-{id}").into_bytes(),
    }
}

async fn call(queue: &Queue, key: &str, now: i64, command: QueueCommand) -> QueueResponse {
    queue
        .execute(QueueRequest::new(key, now, command))
        .await
        .unwrap()
}

async fn claim(queue: &Queue, key: &str, owner: &str, now: i64) -> Option<Delivery> {
    match call(
        queue,
        key,
        now,
        QueueCommand::Claim {
            queue: "atlas".into(),
            owner: owner.into(),
            visibility_ms: 100,
        },
    )
    .await
    {
        QueueResponse::Delivery(message) => message,
        response => panic!("unexpected queue response: {response:?}"),
    }
}

#[tokio::test]
async fn published_claimed_and_acked_state_survives_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("delivery");
    let queue = Queue::open(&path).await.unwrap();
    assert_eq!(
        call(&queue, "publish", 1000, publish("one")).await,
        QueueResponse::Published(true)
    );
    drop(queue);
    let queue = Queue::open(&path).await.unwrap();
    let first = claim(&queue, "claim-first", "owner-a", 1001).await.unwrap();
    drop(queue);
    let queue = Queue::open(&path).await.unwrap();
    assert!(claim(&queue, "not-expired", "owner-b", 1100)
        .await
        .is_none());
    let second = claim(&queue, "expired", "owner-b", 1101).await.unwrap();
    assert!(second.token.epoch > first.token.epoch);
    assert_eq!(second.delivery_count, 2);
    assert_eq!(first.payload, second.payload);
    assert_eq!(
        call(
            &queue,
            "stale-ack",
            1102,
            QueueCommand::Ack { token: first.token }
        )
        .await,
        QueueResponse::Changed(false)
    );
    assert_eq!(
        call(
            &queue,
            "ack",
            1102,
            QueueCommand::Ack {
                token: second.token
            }
        )
        .await,
        QueueResponse::Changed(true)
    );
    drop(queue);
    let queue = Queue::open(path).await.unwrap();
    assert!(claim(&queue, "after-reopen", "owner-c", 2000)
        .await
        .is_none());
    assert_eq!(
        call(&queue, "republish-same-id", 2001, publish("one")).await,
        QueueResponse::Published(false),
        "ack tombstone must prevent resurrecting duplicate publication"
    );
}

#[tokio::test]
async fn replay_is_stable_across_server_clock_changes_and_rejects_key_collisions() {
    let directory = tempfile::tempdir().unwrap();
    let queue = Queue::open(directory.path().join("delivery"))
        .await
        .unwrap();
    let published = call(&queue, "publish", 1000, publish("one")).await;
    assert_eq!(
        published,
        call(&queue, "publish", 2000, publish("one")).await
    );
    let first = claim(&queue, "claim", "owner-a", 1001).await.unwrap();
    let replay = claim(&queue, "claim", "owner-a", 3000).await.unwrap();
    assert_eq!(
        first, replay,
        "an ambiguous response retry must replay the original lease"
    );
    assert!(queue
        .execute(QueueRequest::new(
            "claim",
            3000,
            QueueCommand::Claim {
                queue: "atlas".into(),
                owner: "other-owner".into(),
                visibility_ms: 100
            }
        ))
        .await
        .is_err());
    assert_eq!(
        call(
            &queue,
            "expired-ack",
            3001,
            QueueCommand::Ack {
                token: replay.token
            }
        )
        .await,
        QueueResponse::Changed(false)
    );
}

#[tokio::test]
async fn renew_requeue_delay_and_expiry_fence_old_tokens() {
    let directory = tempfile::tempdir().unwrap();
    let queue = Queue::open(directory.path().join("delivery"))
        .await
        .unwrap();
    call(&queue, "publish", 1000, publish("one")).await;
    let first = claim(&queue, "claim-a", "owner-a", 1001).await.unwrap();
    assert_eq!(
        call(
            &queue,
            "renew",
            1050,
            QueueCommand::Renew {
                token: first.token.clone(),
                visibility_ms: 200
            }
        )
        .await,
        QueueResponse::Changed(true)
    );
    assert!(claim(&queue, "still-leased", "owner-b", 1101)
        .await
        .is_none());
    assert_eq!(
        call(
            &queue,
            "requeue",
            1102,
            QueueCommand::Requeue {
                token: first.token.clone(),
                delay_ms: 500
            }
        )
        .await,
        QueueResponse::Changed(true)
    );
    assert_eq!(
        call(
            &queue,
            "stale-after-requeue",
            1103,
            QueueCommand::Ack {
                token: first.token.clone()
            }
        )
        .await,
        QueueResponse::Changed(false)
    );
    assert!(claim(&queue, "delay-pending", "owner-b", 1601)
        .await
        .is_none());
    let second = claim(&queue, "delay-elapsed", "owner-b", 1602)
        .await
        .unwrap();
    assert!(second.token.epoch > first.token.epoch);
    assert_eq!(
        call(
            &queue,
            "expired-renew",
            1702,
            QueueCommand::Renew {
                token: second.token.clone(),
                visibility_ms: 100
            }
        )
        .await,
        QueueResponse::Changed(false)
    );
    assert_eq!(
        call(
            &queue,
            "expired-deadletter",
            1702,
            QueueCommand::DeadLetter {
                token: second.token
            }
        )
        .await,
        QueueResponse::Changed(false)
    );
}

#[tokio::test]
async fn concurrent_claims_deliver_each_envelope_once_per_visibility_window() {
    let directory = tempfile::tempdir().unwrap();
    let queue = Queue::open(directory.path().join("delivery"))
        .await
        .unwrap();
    call(&queue, "publish", 1000, publish("one")).await;
    let (left, right) = tokio::join!(
        claim(&queue, "claim-a", "owner-a", 1001),
        claim(&queue, "claim-b", "owner-b", 1001)
    );
    assert_ne!(
        left.is_some(),
        right.is_some(),
        "only one worker may receive the live lease"
    );
}

#[tokio::test]
async fn deadletters_are_retained_across_reopen_and_isolated_by_queue() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("delivery");
    let queue = Queue::open(&path).await.unwrap();
    call(&queue, "publish", 1000, publish("one")).await;
    let first = claim(&queue, "claim", "owner", 1001).await.unwrap();
    assert_eq!(
        call(
            &queue,
            "deadletter",
            1002,
            QueueCommand::DeadLetter { token: first.token }
        )
        .await,
        QueueResponse::Changed(true)
    );
    drop(queue);
    let queue = Queue::open(path).await.unwrap();
    assert!(claim(&queue, "after-deadletter", "owner", 2000)
        .await
        .is_none());
    let QueueResponse::DeadLetters(messages) = call(
        &queue,
        "",
        2000,
        QueueCommand::DeadLetters {
            queue: "atlas".into(),
            limit: 1000,
        },
    )
    .await
    else {
        panic!("expected dead-letter list")
    };
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].payload, b"synthetic-one");
    assert_eq!(
        call(
            &queue,
            "",
            2000,
            QueueCommand::DeadLetters {
                queue: "other".into(),
                limit: 100
            }
        )
        .await,
        QueueResponse::DeadLetters(vec![])
    );
}

#[tokio::test]
async fn validation_rejects_large_payload_collisions_and_bad_durations_without_mutation() {
    let directory = tempfile::tempdir().unwrap();
    let queue = Queue::open(directory.path().join("delivery"))
        .await
        .unwrap();
    assert!(queue
        .execute(QueueRequest::new(
            "large",
            1000,
            QueueCommand::Publish {
                queue: "atlas".into(),
                id: "one".into(),
                payload: vec![0; MAX_PAYLOAD_BYTES + 1]
            }
        ))
        .await
        .is_err());
    assert!(claim(&queue, "empty-after-reject", "owner", 1001)
        .await
        .is_none());
    call(&queue, "publish", 1002, publish("one")).await;
    assert!(queue
        .execute(QueueRequest::new(
            "collision",
            1003,
            QueueCommand::Publish {
                queue: "atlas".into(),
                id: "one".into(),
                payload: b"different".to_vec()
            }
        ))
        .await
        .is_err());
    assert!(queue
        .execute(QueueRequest::new(
            "bad-duration",
            1003,
            QueueCommand::Claim {
                queue: "atlas".into(),
                owner: "owner".into(),
                visibility_ms: 0
            }
        ))
        .await
        .is_err());
    assert!(queue
        .execute(QueueRequest::new(
            "overflow",
            i64::MAX,
            QueueCommand::Claim {
                queue: "atlas".into(),
                owner: "owner".into(),
                visibility_ms: 100
            }
        ))
        .await
        .is_err());
    let first = claim(&queue, "valid-claim", "owner", 1004).await.unwrap();
    assert_eq!(first.payload, b"synthetic-one");
}

// This isolated child deliberately bypasses DB Drop to prove acknowledged WAL
// writes survive process termination without relying on a clean close/flush.
#[test]
fn abrupt_exit_child() {
    let Some(path) = std::env::var_os("FLEETWORKS_QUEUE_ABRUPT_EXIT_TEST_PATH") else {
        return;
    };
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let queue = Queue::open(path).await.unwrap();
        assert_eq!(
            call(&queue, "publish", 1000, publish("one")).await,
            QueueResponse::Published(true)
        );
        std::process::exit(0);
    });
}

#[tokio::test]
async fn synchronous_wal_survives_process_exit_without_destructors() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("delivery");
    let output = tokio::process::Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("abrupt_exit_child")
        .arg("--nocapture")
        .env("FLEETWORKS_QUEUE_ABRUPT_EXIT_TEST_PATH", &path)
        .kill_on_drop(true)
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "isolated queue child failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let queue = Queue::open(path).await.unwrap();
    assert_eq!(
        claim(&queue, "claim", "owner", 1001).await.unwrap().payload,
        b"synthetic-one"
    );
}
