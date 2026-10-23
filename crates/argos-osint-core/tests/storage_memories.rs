//! Brain memory operations on the async repository (migration phase 4),
//! the CLI's `remember|recall|memories`. Both modes via `scripts/test_surreal.py`.
#![cfg(feature = "surreal")]

mod support;

use argos_osint_core::brain::MemorySource;
use argos_osint_core::storage::{memories, outbox};
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
async fn remember_list_and_recall_through_the_repository() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "memories").await;
    let err = memories::add_memory(&repo, "  ", "fact", false, source()).await;
    assert!(err.is_err());
    let a = memories::add_memory(&repo, " The ship sails at dawn ", "fact", false, source())
        .await
        .unwrap();
    assert_eq!(a.text, "The ship sails at dawn");
    let b = memories::add_memory(&repo, "Harbor closes at noon", "nonsense", true, source())
        .await
        .unwrap();
    assert_eq!(
        b.category,
        argos_osint_core::brain::normalize_category("nonsense")
    );

    let listed = memories::list_memories(&repo).await.unwrap();
    assert_eq!(
        listed.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        vec![b.id.as_str(), a.id.as_str()],
        "pinned first"
    );
    assert_eq!(listed[1], a, "round trip keeps every field");

    // Index work was queued with the write.
    let pending = outbox::pending(repo.db(), 0, 10).await.unwrap();
    assert_eq!(pending.len(), 2);

    let hits = memories::recall(&repo, "when does the ship sail", 8)
        .await
        .unwrap();
    assert_eq!(
        hits.first().map(|h| h.memory.id.clone()),
        Some(a.id.clone())
    );
    assert!(hits.len() <= 5, "recall cap");
    assert!(memories::recall(&repo, " ", 8).await.unwrap().is_empty());
}
