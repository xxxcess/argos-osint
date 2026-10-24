//! Process registry beats, orphan recovery and NLP feature GC on SurrealDB
//! (`storage::processes`, `storage::nlp_cache::gc_features`), against the
//! embedded RocksDB engine or a disk server (see `support`).
//! Run through `python3 scripts/test_surreal.py --mode embedded`.
#![cfg(feature = "surreal")]

mod support;

use argos_osint_core::storage::jobs::{self, NewJob, NewTask};
use argos_osint_core::storage::{nlp_cache, processes};
use chrono::{Duration, Utc};
use support::*;
use surrealdb::engine::any::Any;
use surrealdb::types::SurrealValue;
use surrealdb::Surreal;

async fn fresh(server: &Server, name: &str) -> Surreal<Any> {
    let db = server.root("argos", name).await;
    jobs::apply_schema(&db).await.expect("jobs schema");
    processes::apply_schema(&db).await.expect("process schema");
    db
}

async fn scalar<T: SurrealValue>(db: &Surreal<Any>, sql: &str) -> T {
    let mut r = db.query(sql).await.unwrap().check().unwrap();
    r.take::<Option<T>>(0usize)
        .unwrap()
        .expect("scalar query returned NONE")
}

#[tokio::test]
async fn process_beat_is_idempotent_and_never_moves_back() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = fresh(&server, "process_beat").await;
    let now = Utc::now();
    processes::beat(&db, "p1", 42, now).await.unwrap();
    processes::beat(&db, "p1", 42, now).await.unwrap();
    // An older replay leaves heartbeat_at where it was.
    processes::beat(&db, "p1", 42, now - Duration::seconds(60))
        .await
        .unwrap();
    assert_eq!(count(&db, "process").await, 1);
    let fresh_now: bool = scalar(
        &db,
        &format!(
            "RETURN (SELECT VALUE heartbeat_at FROM ONLY process:p1) >= <datetime>'{}'",
            now.to_rfc3339()
        ),
    )
    .await;
    assert!(fresh_now, "a stale replay moved the beat back");
    let pid: i64 = scalar(&db, "SELECT VALUE pid FROM ONLY process:p1").await;
    assert_eq!(pid, 42);
}

#[tokio::test]
async fn orphan_recovery_requeues_gone_owners_and_skips_when_self_is_stale() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = fresh(&server, "process_orphans").await;
    let now = Utc::now();
    let job = NewJob {
        id: "j",
        kind: "task",
        operation: "test",
        parent: None,
        created_at: now,
    };
    let task = NewTask {
        id: "t",
        job: "j",
        pool: "p",
        operation: "op",
        priority: 0,
        deps: vec![],
        input_hash: "h",
        created_at: now,
    };
    jobs::create_job(&db, &job, &[task]).await.unwrap();
    let key = format!("claim:w:{}", nanos());
    jobs::claim_next(&db, "p", "w", 30, now, &key)
        .await
        .unwrap()
        .value
        .expect("claimed");
    db.query("UPDATE task:t SET process_owner = 'gone'")
        .await
        .unwrap()
        .check()
        .unwrap();

    // Without a fresh beat of its own the caller recovers nothing.
    let r = processes::recover_orphans(&db, "me", now)
        .await
        .unwrap()
        .value;
    assert!(r.skipped_self_stale);
    assert_eq!(r.tasks, 0);
    let state: String = scalar(&db, "SELECT VALUE state FROM ONLY task:t").await;
    assert_eq!(state, "running");

    // The gone owner beat long ago; the caller is fresh.
    processes::beat(&db, "gone", 1, now - Duration::seconds(120))
        .await
        .unwrap();
    processes::beat(&db, "me", 2, now).await.unwrap();
    let r = processes::recover_orphans(&db, "me", now)
        .await
        .unwrap()
        .value;
    assert!(!r.skipped_self_stale);
    assert_eq!(r.tasks, 1);
    assert_eq!(r.processes, 1);
    let state: String = scalar(&db, "SELECT VALUE state FROM ONLY task:t").await;
    assert_ne!(state, "running");
    let owner: String = scalar(&db, "SELECT VALUE process_owner FROM ONLY task:t").await;
    assert_eq!(owner, "");
    assert_eq!(count(&db, "process").await, 1);

    // Replay recovers nothing more.
    let r = processes::recover_orphans(&db, "me", now)
        .await
        .unwrap()
        .value;
    assert_eq!((r.tasks, r.processes), (0, 0));
}

#[tokio::test]
async fn nlp_feature_gc_drops_features_without_a_source_ref() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "nlp_gc").await;
    let db = repo.db();
    db.query(
        "UPSERT nlp_feature:live SET prose_hash = 'ph-live', version_context_json = '{}',
           annotations_json = '{}', byte_len = 2, last_access = '2026-01-01T00:00:00Z';
         UPSERT nlp_feature:dead SET prose_hash = 'ph-dead', version_context_json = '{}',
           annotations_json = '{}', byte_len = 2, last_access = '2026-01-01T00:00:00Z';
         UPSERT nlp_source_ref:r SET owner_kind = 'answer', owner_id = 'a', source_id = 's',
           revision = '1', attempt_id = 'x', prose_hash = 'ph-live', metadata_json = '{}',
           transform_version = 'v1';",
    )
    .await
    .unwrap()
    .check()
    .unwrap();
    nlp_cache::gc_features(&repo).await.unwrap();
    nlp_cache::gc_features(&repo).await.unwrap();
    let left: Vec<String> = {
        let mut r = db
            .query("SELECT VALUE record::id(id) FROM nlp_feature")
            .await
            .unwrap()
            .check()
            .unwrap();
        r.take(0).unwrap()
    };
    assert_eq!(left, vec!["live".to_string()]);
}
