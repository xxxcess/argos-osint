//! The SurrealDB index pool (`storage::index_drain`).
#![cfg(feature = "surreal")]

mod support;

use std::sync::Arc;

use argos_osint_core::storage::bootstrap;
use argos_osint_core::storage::index_drain::{drain_index, Drained};
use argos_osint_core::storage::vectors::Embedder;
use support::*;

fn fake() -> Embedder {
    Embedder {
        model: "fake".into(),
        embed: Arc::new(|texts: Vec<String>| Ok(texts.iter().map(|_| vec![0.5f32; 384]).collect())),
    }
}

#[tokio::test]
async fn drains_memory_changes_and_leaves_other_kinds_pending() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "index_drain").await;
    bootstrap::ensure_schema(repo.db()).await.unwrap();
    repo.db()
        .query(
            "CREATE index_change:c1 SET seq = 1, record_kind = 'memory', record_id = 'm1',
               operation = 'upsert', created_at = time::now(), updated_at = time::now();
             CREATE index_change:c2 SET seq = 2, record_kind = 'memory', record_id = 'm2',
               operation = 'remove', created_at = time::now(), updated_at = time::now();
             CREATE index_change:c3 SET seq = 3, record_kind = 'claim', record_id = 'x',
               operation = 'upsert', created_at = time::now(), updated_at = time::now();",
        )
        .await
        .unwrap()
        .check()
        .unwrap();
    let d = drain_index(&repo, &fake(), 10).await.unwrap();
    assert_eq!(
        d,
        Drained {
            completed: 2,
            failed: 0
        }
    );
    let mut r = repo
        .db()
        .query("SELECT VALUE [record::id(id), state, outcome] FROM index_change ORDER BY seq")
        .await
        .unwrap()
        .check()
        .unwrap();
    let rows: Vec<Vec<String>> = r.take(0usize).unwrap();
    assert_eq!(rows[0], ["c1", "completed", "skipped"]);
    assert_eq!(rows[1], ["c2", "completed", "removed"]);
    assert_eq!(rows[2][1], "pending");
    // Nothing left for this consumer.
    assert_eq!(
        drain_index(&repo, &fake(), 10).await.unwrap(),
        Drained::default()
    );
}
