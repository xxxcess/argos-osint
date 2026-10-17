use std::sync::atomic::AtomicBool;
use tempfile::tempdir;

use super::*;
use crate::brain::MemorySource;
use crate::brain_actors::{self as actors, BindRequest};
use crate::store::Store;

fn test_store() -> (tempfile::TempDir, Store) {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test.db");
    let store = Store::open(&path).unwrap();
    (dir, store)
}

#[tokio::test]
async fn enqueue_and_drain_scope_analysis_creates_inferences_and_links() {
    let (dir, store) = test_store();
    let db_path = dir.path().join("test.db");
    // Claims only lease jobs inside a 3-day window, so a fixed date rots.
    let now = &chrono::Utc::now().to_rfc3339();

    // Migrate tasks tables
    tasks::migrate_tables(&store.conn).unwrap();

    // 1. Add 2 claims
    let c1 = store
        .add_user_claim(
            "Acme Corp expanded European logistics",
            "fact",
            MemorySource {
                app: "test".into(),
                conversation_id: "c1".into(),
                message_id: None,
                reference: None,
            },
        )
        .unwrap();

    let c2 = store
        .add_user_claim(
            "Acme Corp acquired terminal in Rotterdam",
            "fact",
            MemorySource {
                app: "test".into(),
                conversation_id: "c2".into(),
                message_id: None,
                reference: None,
            },
        )
        .unwrap();

    // 2. Add an actor bound to c1
    let (binding, _) = actors::bind(
        &store.conn,
        &BindRequest {
            name: "Acme Corp",
            subtype: actors::parse_subtype("organization"),
            scope: "global",
            identifiers: &[],
            external: None,
            provenance: serde_json::json!({}),
        },
        now,
    )
    .unwrap();

    store
        .conn
        .execute(
            "INSERT INTO brain_memory_actor_refs(memory_id, actor_id, role, binding_state) VALUES(?1, ?2, 'subject', 'resolved')",
            rusqlite::params![c1.id, binding.actor_id],
        )
        .unwrap();

    store
        .conn
        .execute(
            "INSERT INTO brain_memory_actor_refs(memory_id, actor_id, role, binding_state) VALUES(?1, ?2, 'subject', 'resolved')",
            rusqlite::params![c2.id, binding.actor_id],
        )
        .unwrap();

    // 3. Enqueue scope analysis
    let payload = ScopeAnalysisPayload {
        seed: TopologySeed::Claim(c1.id.clone()),
        cycle_scope: CycleScope::AllRetained,
        depth: 2,
        options: TopologyOptions::default(),
    };

    let job_id = enqueue_scope_analysis(&store.conn, &payload, now).unwrap();
    assert!(!job_id.is_empty());

    // 4. Drain scope tasks
    let stop = AtomicBool::new(false);
    let drained = drain_scope_tasks(&db_path, "test_owner", &stop)
        .await
        .unwrap();
    assert_eq!(drained, 1);

    // 5. Verify task is completed
    let state: String = store
        .conn
        .query_row(
            "SELECT state FROM argos_tasks WHERE job_id=?1",
            [&job_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "completed");

    // 6. Verify an inference was created
    let count: i64 = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM memory_metadata WHERE memory_kind='inference'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(count >= 1);
}
