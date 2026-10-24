//! Topology gaps on SurrealDB: the TUI memory check that seeds the
//! topology mapper (`topology::memory_seed_kind`, the repository side of
//! `Store::get_memory` plus `brain_kinds::memory_kind`) and the live-thread
//! filter on memory provenance edges. Both modes via `scripts/test_surreal.py`.
#![cfg(feature = "surreal")]

mod support;

use argos_osint_core::brain::MemorySource;
use argos_osint_core::memory_metadata::MemoryKind;
use argos_osint_core::recon::Thread;
use argos_osint_core::storage::{memories, provenance, recon, topology};
use support::*;

fn source() -> MemorySource {
    MemorySource {
        app: "cli".into(),
        conversation_id: "c1".into(),
        message_id: None,
        reference: None,
    }
}

fn thread(id: &str) -> Thread {
    Thread {
        id: id.into(),
        title: format!("Thread {id}"),
        created_at: "2026-10-23T00:00:00Z".into(),
        updated_at: "2026-10-23T00:00:00Z".into(),
        draft: String::new(),
        scroll: 0,
        recall_insights: true,
    }
}

#[tokio::test]
async fn memory_seed_kind_matches_the_sqlite_memory_check() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "topology_memory_check").await;
    let db = repo.db().clone();
    let claim = memories::add_memory(&repo, "Acme bought Widgets", "fact", false, source(), None)
        .await
        .unwrap();
    let mut ids = Vec::new();
    for (kind, text) in [
        ("inference", "Acme is expanding"),
        ("link", "Acme links Widgets"),
        ("related", "Acme relates to Gadgets"),
        ("actor", "Acme Corp"),
        ("bogus", "Unknown kind"),
    ] {
        let m = memories::add_memory(&repo, text, "fact", false, source(), None)
            .await
            .unwrap();
        db.query("UPDATE type::record('memory', $m) SET memory_kind = $k RETURN NONE")
            .bind(("m", m.id.clone()))
            .bind(("k", kind.to_string()))
            .await
            .unwrap()
            .check()
            .unwrap();
        ids.push(m.id);
    }

    let kind = |id: String| {
        let repo = repo.clone();
        async move { topology::memory_seed_kind(&repo, &id).await.unwrap() }
    };
    assert_eq!(kind(claim.id.clone()).await, Some(MemoryKind::Claim));
    assert_eq!(kind(ids[0].clone()).await, Some(MemoryKind::Inference));
    assert_eq!(kind(ids[1].clone()).await, Some(MemoryKind::Link));
    assert_eq!(kind(ids[2].clone()).await, Some(MemoryKind::Related));
    assert_eq!(kind(ids[3].clone()).await, Some(MemoryKind::Actor));
    // An unknown kind reads as a Claim, like SQLite's untyped rows.
    assert_eq!(kind(ids[4].clone()).await, Some(MemoryKind::Claim));
    // A missing memory is "no longer exists", not an error.
    assert_eq!(kind("missing".into()).await, None);
}

#[tokio::test]
async fn memory_record_edges_skip_deleted_and_unknown_threads() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "topology_thread_edges").await;
    let db = repo.db().clone();
    for t in ["live", "gone"] {
        recon::put_thread(&repo, &thread(t)).await.unwrap();
    }
    let mem = memories::add_memory(&repo, "Acme sued Gadgets", "fact", false, source(), None)
        .await
        .unwrap()
        .id;
    db.query(
        "CREATE insight_claim:fp1 SET memory_id = $m, entity_id = 'e', predicate = 'p',
           object_value = 'o', topic = '', classification = 'fact', confidence = 0.5,
           created_at = '', updated_at = '';
         CREATE insight_source SET fingerprint = 'fp1', answer_id = 'a1', call_id = 'c1',
           thread_id = 'live';
         CREATE insight_source SET fingerprint = 'fp1', answer_id = 'a2', call_id = 'c2',
           thread_id = 'gone';
         CREATE insight_source SET fingerprint = 'fp1', answer_id = 'a3', call_id = 'c3',
           thread_id = 'unknown';
         UPDATE recon_thread:gone SET deleted = true RETURN NONE;",
    )
    .bind(("m", mem.clone()))
    .await
    .unwrap()
    .check()
    .unwrap();
    let edges = provenance::memory_record_edges(&db, &mem).await.unwrap();
    let threads: Vec<&str> = edges
        .iter()
        .filter(|e| e.key.starts_with("thread-evidence:"))
        .map(|e| e.key.as_str())
        .collect();
    let live = format!("thread-evidence:live:{mem}");
    assert_eq!(threads, vec![live.as_str()], "{edges:#?}");
}
