use fleetworks_jobs::{Command, NewJob, NewTask, Request, Response, Store, Task};
use serde_json::json;

fn submit(id: &str, tasks: &[&str]) -> Command {
    Command::Submit {
        job: NewJob {
            id: id.into(),
            operation: "synthetic_fixture".into(),
            ..Default::default()
        },
        tasks: tasks
            .iter()
            .map(|id| NewTask {
                id: (*id).into(),
                operation: "synthetic_task".into(),
                pool: "test".into(),
                ..Default::default()
            })
            .collect(),
    }
}

async fn call(store: &Store, key: &str, now: i64, command: Command) -> Response {
    store
        .execute(Request::new(key, now, command))
        .await
        .unwrap()
}

async fn claim(store: &Store, key: &str, owner: &str, now: i64) -> Option<Task> {
    match call(
        store,
        key,
        now,
        Command::Claim {
            pool: "test".into(),
            owner: owner.into(),
            lease_ms: 100,
            task_id: None,
        },
    )
    .await
    {
        Response::Task(task) => task,
        response => panic!("unexpected claim response: {response:?}"),
    }
}

#[tokio::test]
async fn completed_state_receipt_and_outbox_survive_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("jobs.sqlite3");
    let store = Store::open(&path).await.unwrap();
    let request = Request::new("submit", 1000, submit("job", &["task"]));
    let first = store.execute(request.clone()).await.unwrap();
    assert_eq!(first, store.execute(request).await.unwrap());
    let task = claim(&store, "claim", "worker", 1001).await.unwrap();
    let complete = Request::new(
        "complete",
        1002,
        Command::Complete {
            lease: task.lease().unwrap(),
            output: json!({"synthetic":true}),
        },
    );
    assert_eq!(
        store.execute(complete.clone()).await.unwrap(),
        Response::Changed(true)
    );
    drop(store);
    let store = Store::open(path).await.unwrap();
    assert_eq!(
        store.execute(complete).await.unwrap(),
        Response::Changed(true)
    );
    let Response::Job(Some(job)) = call(
        &store,
        "",
        1003,
        Command::Get {
            job_id: "job".into(),
        },
    )
    .await
    else {
        panic!("job was not retained")
    };
    assert_eq!(job.state, "completed");
    let Response::Outbox(entries) = call(
        &store,
        "",
        1003,
        Command::Outbox {
            kind: Some("publish".into()),
            limit: 100,
        },
    )
    .await
    else {
        panic!("missing publication outbox")
    };
    assert_eq!(
        entries.len(),
        1,
        "replaying completion must not duplicate publication"
    );
    let Response::Attempts(attempts) = call(
        &store,
        "",
        1003,
        Command::Attempts {
            job_id: "job".into(),
        },
    )
    .await
    else {
        panic!("missing attempt history")
    };
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].state, "completed");
    assert_eq!(attempts[0].finished_at, Some(1002));
}

#[tokio::test]
async fn independent_connections_cannot_claim_same_task() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("jobs.sqlite3");
    let first = Store::open(&path).await.unwrap();
    let second = Store::open(path).await.unwrap();
    call(&first, "submit", 1000, submit("job", &["task"])).await;
    let (left, right) = tokio::join!(
        claim(&first, "claim-a", "worker-a", 1001),
        claim(&second, "claim-b", "worker-b", 1001)
    );
    assert_ne!(
        left.is_some(),
        right.is_some(),
        "exactly one concurrent claimant must win"
    );
}

#[tokio::test]
async fn expiry_takeover_and_cancel_fence_stale_workers() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path().join("jobs.sqlite3"))
        .await
        .unwrap();
    call(&store, "submit", 1000, submit("job", &["task"])).await;
    let old = claim(&store, "claim-a", "worker-a", 1001)
        .await
        .unwrap()
        .lease()
        .unwrap();
    assert_eq!(
        call(
            &store,
            "expired-renew",
            1101,
            Command::Renew {
                lease: old.clone(),
                lease_ms: 100
            }
        )
        .await,
        Response::Changed(false)
    );
    assert_eq!(
        call(&store, "reap", 1101, Command::Reap).await,
        Response::Count(1)
    );
    let current = claim(&store, "claim-b", "worker-b", 1102)
        .await
        .unwrap()
        .lease()
        .unwrap();
    assert!(current.epoch > old.epoch);
    assert_eq!(
        call(
            &store,
            "stale-complete",
            1103,
            Command::Complete {
                lease: old,
                output: json!("stale")
            }
        )
        .await,
        Response::Changed(false)
    );
    assert_eq!(
        call(
            &store,
            "cancel",
            1104,
            Command::Cancel {
                job_id: "job".into()
            }
        )
        .await,
        Response::Changed(true)
    );
    assert_eq!(
        call(
            &store,
            "cancelled-complete",
            1105,
            Command::Complete {
                lease: current,
                output: json!("cancelled")
            }
        )
        .await,
        Response::Changed(false)
    );
    assert!(claim(&store, "claim-c", "worker-c", 1106).await.is_none());
    let Response::Outbox(entries) = call(
        &store,
        "",
        1106,
        Command::Outbox {
            kind: Some("publish".into()),
            limit: 100,
        },
    )
    .await
    else {
        panic!("missing outbox response")
    };
    assert!(entries.is_empty());
}

#[tokio::test]
async fn dependency_order_failure_and_manual_retry_preserve_attempt_history() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path().join("jobs.sqlite3"))
        .await
        .unwrap();
    let mut command = submit("job", &["first", "second"]);
    if let Command::Submit { tasks, .. } = &mut command {
        tasks[0].attempt_cap = 1;
        tasks[1].dependencies = vec!["first".into()];
        tasks[1].priority = -10;
    }
    call(&store, "submit", 1000, command).await;
    let first = claim(&store, "claim-first", "worker", 1001).await.unwrap();
    assert_eq!(
        first.id, "first",
        "higher priority dependent cannot claim early"
    );
    assert_eq!(
        call(
            &store,
            "fail",
            1002,
            Command::Fail {
                lease: first.lease().unwrap(),
                error: "synthetic error".into(),
                retry_at: Some(1003)
            }
        )
        .await,
        Response::Changed(true)
    );
    assert!(claim(&store, "blocked", "worker", 1004).await.is_none());
    assert_eq!(
        call(
            &store,
            "retry",
            1005,
            Command::Retry {
                job_id: "job".into()
            }
        )
        .await,
        Response::Count(2)
    );
    let first = claim(&store, "claim-first-again", "worker", 1006)
        .await
        .unwrap();
    assert_eq!(first.id, "first");
    assert_eq!(first.attempts_used, 2);
    call(
        &store,
        "complete-first",
        1007,
        Command::Complete {
            lease: first.lease().unwrap(),
            output: json!(1),
        },
    )
    .await;
    let second = claim(&store, "claim-second", "worker", 1008).await.unwrap();
    assert_eq!(second.id, "second");
}

#[tokio::test]
async fn invalid_submit_rolls_back_and_operation_keys_cannot_be_repurposed() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path().join("jobs.sqlite3"))
        .await
        .unwrap();
    let mut invalid = submit("bad", &["task"]);
    if let Command::Submit { tasks, .. } = &mut invalid {
        tasks[0].dependencies = vec!["missing".into()];
    }
    assert!(store
        .execute(Request::new("invalid", 1000, invalid))
        .await
        .is_err());
    assert_eq!(
        call(
            &store,
            "",
            1001,
            Command::Get {
                job_id: "bad".into()
            }
        )
        .await,
        Response::Job(None)
    );
    call(&store, "stable", 1002, submit("good", &["task"])).await;
    assert!(store
        .execute(Request::new("stable", 1002, submit("different", &[])))
        .await
        .is_err());
}

#[tokio::test]
async fn cancellation_cascades_to_children_and_survives_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("jobs.sqlite3");
    let store = Store::open(&path).await.unwrap();
    call(&store, "parent", 1000, submit("parent", &[])).await;
    let mut child = submit("child", &["task"]);
    if let Command::Submit { job, .. } = &mut child {
        job.parent_id = Some("parent".into());
    }
    call(&store, "child", 1001, child).await;
    call(
        &store,
        "cancel",
        1002,
        Command::Cancel {
            job_id: "parent".into(),
        },
    )
    .await;
    drop(store);
    let store = Store::open(path).await.unwrap();
    let Response::Job(Some(child)) = call(
        &store,
        "",
        1003,
        Command::Get {
            job_id: "child".into(),
        },
    )
    .await
    else {
        panic!("child was lost")
    };
    assert!(child.cancel_requested);
    assert_eq!(child.state, "cancelled");
    assert!(claim(&store, "claim", "worker", 1003).await.is_none());
}

#[tokio::test]
async fn pending_publication_prevents_retention_deletion() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path().join("jobs.sqlite3"))
        .await
        .unwrap();
    call(&store, "submit", 1000, submit("job", &["task"])).await;
    let task = claim(&store, "claim", "worker", 1001).await.unwrap();
    call(
        &store,
        "complete",
        1002,
        Command::Complete {
            lease: task.lease().unwrap(),
            output: json!(true),
        },
    )
    .await;
    assert_eq!(
        call(
            &store,
            "prune-pending",
            2000,
            Command::Prune { before: 1500 }
        )
        .await,
        Response::Count(0)
    );
    let Response::Outbox(entries) = call(
        &store,
        "",
        2000,
        Command::Outbox {
            kind: None,
            limit: 100,
        },
    )
    .await
    else {
        panic!("missing outbox response")
    };
    for entry in entries {
        call(
            &store,
            &format!("ack-{}", entry.seq),
            2001,
            Command::Acknowledge { seq: entry.seq },
        )
        .await;
    }
    assert_eq!(
        call(&store, "prune-acked", 2002, Command::Prune { before: 1500 }).await,
        Response::Count(1)
    );
    assert_eq!(
        call(
            &store,
            "",
            2003,
            Command::Get {
                job_id: "job".into()
            }
        )
        .await,
        Response::Job(None)
    );
}
