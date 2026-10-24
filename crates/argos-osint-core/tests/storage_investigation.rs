//! Investigation harness storage on SurrealDB (`storage::investigation`,
//! `investigation::engine` through `ReconStore`): the same writes give the
//! same rows as SQLite. Run through `python3 scripts/test_surreal.py --mode both`.
#![cfg(feature = "surreal")]

mod support;

use argos_osint_core::investigation::evidence::{ClaimAssessment, ClaimStance, EvidencePassage};
use argos_osint_core::investigation::{InvestigationSurface, TaskRecord, TaskStatus};
use argos_osint_core::recon::backend::ReconStore;
use argos_osint_core::storage::investigation;
use argos_osint_core::store::Store;
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

/// Comparable view of a task: `updated_at` is stamped at write time.
fn view(t: &TaskRecord) -> serde_json::Value {
    let mut v = serde_json::to_value(t).unwrap();
    v.as_object_mut().unwrap().remove("updated_at");
    v
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

#[tokio::test]
async fn investigation_tasks_match_sqlite() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = server.root("argos", "investigation_tasks").await;
    investigation::apply_schema(&db).await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("argos.db")).unwrap();

    let mut b = task("inv-1", "t2");
    let a = task("inv-1", "t1");
    let other = task("inv-2", "t1");
    for t in [&b, &a, &other] {
        store.insert_investigation_task(t).unwrap();
        investigation::insert_task(&db, t).await.unwrap();
    }
    // A rewrite only moves status, attempts, output_ref, unmet needs and
    // terminal reason; the objective stays.
    b.status = TaskStatus::Completed;
    b.attempts = 2;
    b.output_ref = "passages:3".into();
    b.objective = "ignored".into();
    store.insert_investigation_task(&b).unwrap();
    investigation::insert_task(&db, &b).await.unwrap();

    let sqlite: Vec<_> = store.list_investigation_tasks("inv-1").unwrap();
    let surreal = investigation::list_tasks(&db, "inv-1").await.unwrap();
    assert_eq!(
        surreal.iter().map(view).collect::<Vec<_>>(),
        sqlite.iter().map(view).collect::<Vec<_>>()
    );
    assert_eq!(surreal.len(), 2);
    assert_eq!(surreal[1].status, TaskStatus::Completed);
    assert_eq!(surreal[1].attempts, 2);
    assert_eq!(surreal[1].objective, "objective t2");

    let one = investigation::get_task(&db, &a.id).await.unwrap().unwrap();
    assert_eq!(
        view(&one),
        view(&store.get_investigation_task(&a.id).unwrap().unwrap())
    );
    // Missing and empty ids are `None`, never a query on an empty record id.
    assert!(investigation::get_task(&db, "nope")
        .await
        .unwrap()
        .is_none());
    assert!(investigation::get_task(&db, "").await.unwrap().is_none());
    let mut blank = task("inv-1", "t9");
    blank.id = String::new();
    assert!(investigation::insert_task(&db, &blank).await.is_err());

    investigation::update_task_state(&db, &a.id, TaskStatus::Failed, "", "boom")
        .await
        .unwrap();
    store
        .update_task_state(&a.id, TaskStatus::Failed, "", "boom")
        .unwrap();
    let a2 = investigation::get_task(&db, &a.id).await.unwrap().unwrap();
    assert_eq!(a2.status, TaskStatus::Failed);
    assert_eq!(a2.terminal_reason, "boom");
    investigation::update_task_state(&db, "missing", TaskStatus::Failed, "", "x")
        .await
        .unwrap();
    assert!(investigation::get_task(&db, "missing")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn investigation_dependencies_passages_assessments_match_sqlite() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = server.root("argos", "investigation_evidence").await;
    investigation::apply_schema(&db).await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("argos.db")).unwrap();

    // SQLite's dependency rows reference existing tasks.
    for id in ["t0", "t1", "t2", "t3"] {
        let mut t = task("inv-1", id);
        t.id = id.into();
        store.insert_investigation_task(&t).unwrap();
    }
    for (t, d) in [("t2", "t1"), ("t2", "t0"), ("t2", "t1"), ("t3", "t2")] {
        store.add_task_dependency(t, d).unwrap();
        investigation::add_dependency(&db, t, d).await.unwrap();
    }
    let mut sqlite = store.get_task_dependencies("t2").unwrap();
    sqlite.sort();
    assert_eq!(
        investigation::dependencies(&db, "t2").await.unwrap(),
        sqlite
    );
    assert_eq!(sqlite, vec!["t0".to_string(), "t1".to_string()]);
    assert!(investigation::dependencies(&db, "t1")
        .await
        .unwrap()
        .is_empty());
    assert!(investigation::add_dependency(&db, "", "t1").await.is_err());

    let p2 = passage("p2", "2026-10-24T02:00:00+00:00", ClaimStance::Mention);
    let p1 = passage("p1", "2026-10-24T01:00:00+00:00", ClaimStance::Mention);
    for p in [&p2, &p1] {
        store.insert_evidence_passage(p).unwrap();
        investigation::insert_passage(&db, p).await.unwrap();
    }
    let mut rewrite = p1.clone();
    rewrite.stance = ClaimStance::Supported;
    rewrite.relevance_score = 0.9;
    rewrite.passage_text = "ignored".into();
    store.insert_evidence_passage(&rewrite).unwrap();
    investigation::insert_passage(&db, &rewrite).await.unwrap();
    let surreal = investigation::list_passages(&db, "inv-1").await.unwrap();
    assert_eq!(surreal, store.list_evidence_passages("inv-1").unwrap());
    assert_eq!(surreal[0].stance, ClaimStance::Supported);
    assert_eq!(surreal[0].passage_text, "passage p1");

    let ass = ClaimAssessment {
        id: "ass:t1:1".into(),
        investigation_id: "inv-1".into(),
        claim_id: "t1".into(),
        evidence_id: "p1".into(),
        stance: ClaimStance::Supported,
        rationale: "cited".into(),
        created_at: "2026-10-24T03:00:00+00:00".into(),
    };
    let mut again = ass.clone();
    again.stance = ClaimStance::Disputed;
    again.rationale = "revised".into();
    again.evidence_id = "ignored".into();
    for a in [&ass, &again] {
        store.insert_claim_assessment(a).unwrap();
        investigation::insert_assessment(&db, a).await.unwrap();
    }
    let surreal = investigation::list_assessments(&db, "inv-1").await.unwrap();
    assert_eq!(surreal, store.list_claim_assessments("inv-1").unwrap());
    assert_eq!(surreal.len(), 1);
    assert_eq!(surreal[0].evidence_id, "p1");
    assert_eq!(surreal[0].rationale, "revised");
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
