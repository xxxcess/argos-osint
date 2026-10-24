//! Investigation harness storage on SurrealDB (`storage::investigation`,
//! `investigation::engine` through `ReconStore`): the same writes give the
//! same rows as SQLite. Run through `python3 scripts/test_surreal.py --mode both`.
#![cfg(feature = "surreal")]

mod support;

use argos_osint_core::investigation::evidence::{ClaimStance, EvidencePassage};
use argos_osint_core::investigation::{InvestigationSurface, TaskRecord};
use argos_osint_core::recon::backend::ReconStore;
use argos_osint_core::storage::investigation;
use support::*;

fn task(inv: &str, task_id: &str) -> TaskRecord {
    TaskRecord::new(
        inv,
        "run-1",
        "d1",
        task_id,
        InvestigationSurface::parse("recon"),
        format!("objective {task_id}"),
        "deep",
        "breadth",
    )
}

fn passage(id: &str, at: &str, stance: ClaimStance) -> EvidencePassage {
    EvidencePassage {
        id: id.into(),
        investigation_id: "inv-1".into(),
        task_id: "t1".into(),
        call_id: "call_harness".into(),
        source_url: format!("https://example.com/{id}"),
        source_domain: "example.com".into(),
        passage_text: format!("passage {id}"),
        observed_at: at.into(),
        published_at: String::new(),
        stance,
        relevance_score: 0.5,
        created_at: at.into(),
    }
}

/// The harness's store calls dispatch to SurrealDB through `ReconStore`.
#[tokio::test]
async fn harness_store_dispatches_to_surreal() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "investigation_dispatch").await;
    investigation::apply_schema(repo.db()).await.unwrap();
    let store = ReconStore::from_repository(repo.clone());

    let t = task("inv-9", "t1");
    store.insert_investigation_task(&t).await.unwrap();
    store.add_task_dependency(&t.id, "dep").await.unwrap();
    let listed = store.list_investigation_tasks("inv-9").await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, t.id);
    assert_eq!(
        store.get_task_dependencies(&t.id).await.unwrap(),
        vec!["dep".to_string()]
    );
    assert!(store.get_investigation_task("dep").await.unwrap().is_none());
    assert_eq!(
        investigation::list_tasks(repo.db(), "inv-9")
            .await
            .unwrap()
            .len(),
        1
    );
}

/// The TUI's `/evidence` count reads passages through `ReconStore`.
#[tokio::test]
async fn evidence_passages_dispatch_to_surreal() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server
        .repo("argos", "investigation_passages_dispatch")
        .await;
    investigation::apply_schema(repo.db()).await.unwrap();
    let store = ReconStore::from_repository(repo);
    assert!(store
        .list_evidence_passages("inv-1")
        .await
        .unwrap()
        .is_empty());
    store
        .insert_evidence_passage(&passage("p1", "2026-01-01T00:00:00Z", ClaimStance::Mention))
        .await
        .unwrap();
    let listed = store.list_evidence_passages("inv-1").await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, "p1");
    assert!(store.list_evidence_passages("").await.unwrap().is_empty());
}
