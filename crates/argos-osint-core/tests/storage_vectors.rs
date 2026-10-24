//! Brain vectors on the async repository (migration phase 4): reindex into
//! a new generation, inline indexing of new memories, and query-embedded
//! recall. A deterministic fake embedder keeps the test offline. Both modes
//! via `scripts/test_surreal.py`.
#![cfg(feature = "surreal")]

mod support;

use std::sync::Arc;

use argos_osint_core::brain::MemorySource;
use argos_osint_core::storage::recall::RecallSource;
use argos_osint_core::storage::vectors::{self, Embedder};
use argos_osint_core::storage::{memories, recall};
use support::*;

fn source() -> MemorySource {
    MemorySource {
        app: "cli".into(),
        conversation_id: "c1".into(),
        message_id: None,
        reference: None,
    }
}

/// One-hot vectors by first letter, so "zephyr ..." and "Zebra?" agree
/// although they share no lexical token.
fn fake(model: &str) -> Embedder {
    Embedder {
        model: model.into(),
        embed: Arc::new(|texts: Vec<String>| {
            Ok(texts
                .iter()
                .map(|t| {
                    let w = t
                        .trim()
                        .chars()
                        .next()
                        .unwrap_or(' ')
                        .to_ascii_lowercase()
                        .to_string();
                    let slot = w.bytes().fold(7usize, |h, b| h * 31 + b as usize) % 384;
                    let mut v = vec![0.0f32; 384];
                    v[slot] = 1.0;
                    v
                })
                .collect())
        }),
    }
}

async fn count(repo: &argos_osint_core::storage::repo::Repository, sql: &str) -> i64 {
    let mut r = repo.db().query(sql).await.unwrap().check().unwrap();
    let n: Option<i64> = r.take(0usize).unwrap();
    n.unwrap_or(0)
}

#[tokio::test]
async fn reindex_generations_and_query_embedded_recall() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "vectors").await;
    let e = fake("fake-a");
    assert!(vectors::state(&repo).await.unwrap().is_none());
    let a = memories::add_memory(
        &repo,
        "zephyr winds at the pier",
        "fact",
        false,
        source(),
        Some(&e),
    )
    .await
    .unwrap();
    memories::add_memory(
        &repo,
        "quartz mine output rose",
        "fact",
        false,
        source(),
        Some(&e),
    )
    .await
    .unwrap();
    assert_eq!(
        count(&repo, "RETURN count(SELECT id FROM memory_embeddings)").await,
        0,
        "no serving generation yet, so nothing is embedded"
    );

    let report = vectors::reindex(&repo, &e).await.unwrap();
    assert_eq!((report.memories, report.generation), (2, 1));
    assert_eq!(report.fingerprint, "fake-a");
    assert_eq!(
        vectors::state(&repo).await.unwrap(),
        Some(vectors::VectorState {
            generation: 1,
            model: "fake-a".into()
        })
    );
    assert_eq!(
        count(
            &repo,
            "RETURN count(SELECT id FROM memory_embeddings WHERE generation = 1)"
        )
        .await,
        2
    );
    assert_eq!(
        count(&repo, "RETURN count(SELECT id FROM memory_index_state WHERE generation = '1' AND fingerprint = 'fake-a')").await,
        2
    );

    // New memories join the serving generation after commit.
    let c = memories::add_memory(
        &repo,
        "harbor cranes idle",
        "fact",
        false,
        source(),
        Some(&e),
    )
    .await
    .unwrap();
    assert_eq!(
        count(
            &repo,
            "RETURN count(SELECT id FROM memory_embeddings WHERE generation = 1)"
        )
        .await,
        3
    );

    // The query shares no token with `a` but its vector; vector hits rank first.
    let hits = memories::recall(&repo, "Zebra?", 5, Some(&e))
        .await
        .unwrap();
    assert_eq!(
        hits.first().map(|h| h.memory.id.clone()),
        Some(a.id.clone())
    );
    let raw = recall::recall(
        repo.db(),
        &recall::RecallQuery {
            text: "Zebra?".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(
        raw.iter().all(|h| h.source != RecallSource::Vector),
        "no vector without an embedding"
    );
    assert!(
        memories::recall(&repo, "Zebra?", 5, None)
            .await
            .unwrap()
            .iter()
            .all(|h| h.memory.id != a.id),
        "lexical recall alone cannot find it"
    );

    // Another model serves nothing: lexical only, and no inline vectors.
    let other = fake("fake-b");
    assert!(memories::recall(&repo, "Zebra?", 5, Some(&other))
        .await
        .unwrap()
        .iter()
        .all(|h| h.memory.id != a.id));

    // A second reindex (new model) switches generation and drops old rows.
    let report = vectors::reindex(&repo, &other).await.unwrap();
    assert_eq!((report.memories, report.generation), (3, 2));
    assert_eq!(
        count(&repo, "RETURN count(SELECT id FROM memory_embeddings)").await,
        3
    );
    assert_eq!(count(&repo, "RETURN count(SELECT id FROM memory_embeddings WHERE generation = 2 AND model = 'fake-b')").await, 3);
    let hits = memories::recall(&repo, "Hello?", 5, Some(&other))
        .await
        .unwrap();
    assert_eq!(
        hits.first().map(|h| h.memory.id.clone()),
        Some(c.id.clone())
    );

    // An embedding failure falls back to lexical recall.
    let broken = Embedder {
        model: "fake-b".into(),
        embed: Arc::new(|_| anyhow::bail!("injected")),
    };
    let hits = memories::recall(&repo, "harbor cranes", 5, Some(&broken))
        .await
        .unwrap();
    assert_eq!(
        hits.first().map(|h| h.memory.id.clone()),
        Some(c.id.clone())
    );
}
