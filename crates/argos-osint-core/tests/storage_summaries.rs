//! The graph summary cache and the NewsAPI quota on SurrealDB
//! (`storage::summaries`, `storage::atlas::quota_*`), against the SQLite
//! `Store::save_graph_summary` / `atlas_quota_bump` behaviour.
//! Both modes via `scripts/test_surreal.py`.
#![cfg(feature = "surreal")]

mod support;

use argos_osint_core::brain::MemorySource;
use argos_osint_core::storage::{atlas, browse, memories, summaries};
use support::*;

fn source() -> MemorySource {
    MemorySource {
        app: "cli".into(),
        conversation_id: "c1".into(),
        message_id: None,
        reference: None,
    }
}

#[tokio::test]
async fn graph_summary_is_cached_per_memory_and_dies_with_it() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "summaries").await;
    let m = memories::add_memory(
        &repo,
        "Harbor closes at noon",
        "fact",
        false,
        source(),
        None,
    )
    .await
    .unwrap();
    assert!(summaries::graph_summary(&repo, &m.id)
        .await
        .unwrap()
        .is_none());
    assert!(summaries::save_graph_summary(&repo, &m.id, "  ", "d1")
        .await
        .is_err());
    assert!(
        summaries::save_graph_summary(&repo, &m.id, " The path established it. ", "d1")
            .await
            .unwrap()
    );
    let got = summaries::graph_summary(&repo, &m.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (got.summary.as_str(), got.focus.as_str()),
        ("The path established it.", "d1")
    );
    // A rewrite replaces the row.
    assert!(summaries::save_graph_summary(&repo, &m.id, "Second.", "")
        .await
        .unwrap());
    let got = summaries::graph_summary(&repo, &m.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!((got.summary.as_str(), got.focus.as_str()), ("Second.", ""));
    // Missing memories are refused, and a delete drops the cached row.
    assert!(!summaries::save_graph_summary(&repo, "nope", "x", "")
        .await
        .unwrap());
    assert!(browse::delete_memory(&repo, &m.id).await.unwrap());
    assert!(summaries::graph_summary(&repo, &m.id)
        .await
        .unwrap()
        .is_none());
    assert!(!summaries::save_graph_summary(&repo, &m.id, "late", "")
        .await
        .unwrap());
}

#[tokio::test]
async fn newsapi_quota_counts_per_bucket_and_day() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = server.root("argos", "quota").await;
    assert_eq!(
        atlas::quota_used(&db, "newsapi", "2000-01-01")
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        atlas::quota_bump(&db, "newsapi", "2000-01-01")
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        atlas::quota_bump(&db, "newsapi", "2000-01-01")
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        atlas::quota_bump(&db, "newsapi:fallback", "2000-01-01")
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        atlas::quota_used(&db, "newsapi", "2000-01-02")
            .await
            .unwrap(),
        0
    );
    // Concurrent charges never lose a count.
    let mut handles = Vec::new();
    for _ in 0..8 {
        let db = db.clone();
        handles.push(tokio::spawn(async move {
            atlas::quota_bump(&db, "newsapi", "2000-01-03")
                .await
                .unwrap()
        }));
    }
    for h in handles {
        h.await.unwrap();
    }
    assert_eq!(
        atlas::quota_used(&db, "newsapi", "2000-01-03")
            .await
            .unwrap(),
        8
    );
}
