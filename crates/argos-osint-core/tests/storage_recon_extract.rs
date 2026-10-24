//! Recon answer extraction and the full-text recall stopgap on SurrealDB
//! (`storage::recon_extract`), against the SQLite `recon::persist_claims`
//! behaviour. Both modes via `scripts/test_surreal.py`.
#![cfg(feature = "surreal")]

mod support;

use argos_osint_core::osint::ToolResult;
use argos_osint_core::recon::{Message, RunLimits};
use argos_osint_core::storage::counters;
use argos_osint_core::storage::recon_exec as rx;
use argos_osint_core::storage::recon_extract as re;
use argos_osint_core::storage::recon_runs as rr;
use argos_osint_core::storage::recon_threads as rt;
use argos_osint_core::storage::repo::Repository;
use serde_json::{json, Value};
use support::*;

const LIMITS: RunLimits = RunLimits {
    max_rounds: 6,
    max_calls: 12,
    turn_seconds: 300,
};
const TOOL: &str = "hunter_person_enrichment";

fn result(status: &str) -> ToolResult {
    serde_json::from_value(json!({
        "tool_id": TOOL, "inputs": {"email": "a@example.org"}, "status": status,
        "source_url": "https://example.org/a", "retrieved_at": "2026-10-23T00:00:00Z",
        "observations": {"name": "A"}, "raw": "{}", "error": null,
        "cached": false, "truncated": false
    }))
    .unwrap()
}

/// A thread (insights on or off) with one answer citing one completed call.
async fn answer(repo: &Repository, insights: bool) -> (Message, String) {
    let t = rt::new_thread(repo, "extract").await.unwrap();
    assert!(rt::set_recall_insights(repo, &t.id, insights)
        .await
        .unwrap());
    let run = rr::new_run(repo, &t.id, "turn", ["r", "p", "s"], LIMITS)
        .await
        .unwrap();
    let inputs = json!({"email": "a@example.org"});
    let call = rr::queue_call(
        repo,
        TOOL,
        &inputs,
        "recon",
        Some(&run.id),
        Some(&t.id),
        None,
    )
    .await
    .unwrap();
    assert!(rr::finish_call(repo, &call, &result("completed"))
        .await
        .unwrap());
    let a = rx::add_answer(
        repo,
        &t.id,
        &run.id,
        "A works at Acme",
        std::slice::from_ref(&call),
        &[],
    )
    .await
    .unwrap();
    (a, call)
}

fn claim(object: &str, eids: &[&str]) -> Value {
    json!({"entity": " Alice ", "namespace": "Person", "predicate": "Works_At",
           "object": object, "claim": format!("Alice works at {object}"),
           "topic": "employment", "classification": "fact", "confidence": 0.9,
           "evidence_ids": eids})
}

async fn strings(repo: &Repository, sql: &str) -> Vec<String> {
    repo.db()
        .query(sql)
        .await
        .unwrap()
        .check()
        .unwrap()
        .take(0usize)
        .unwrap()
}

#[tokio::test]
async fn extraction_writes_claims_sources_relations_and_index_work_in_one_transaction() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "recon_extract").await;
    let (a, call) = answer(&repo, true).await;
    let evidence = vec![
        (call.clone(), result("completed")),
        ("failed".into(), result("failed")),
    ];
    let changed0 = counters::current(repo.db(), "memories_changed")
        .await
        .unwrap();
    let claims = vec![
        claim("Acme", &[&call]),
        claim("Acme", &[&call]), // same fingerprint: one claim
        claim("Globex", &[&call, "failed"]),
        claim("Initech", &["failed"]), // no completed evidence
        json!({"entity": "x", "predicate": "", "object": "y", "claim": "z", "evidence_ids": [call]}),
    ];
    let p = re::persist_claims(&repo, &a, &evidence, &claims)
        .await
        .unwrap();
    assert_eq!((p.written.len(), p.touched.len()), (2, 2));
    assert_eq!(p.actors.len(), 1, "Alice's subject actor is bound once");
    assert!(!p.touched.contains(&p.actors[0]));
    assert_eq!(p.queued, 3, "index work for each new memory and the actor");
    assert_eq!(count(repo.db(), "insight_claim").await, 2);
    assert_eq!(
        count(repo.db(), "insight_source").await,
        2,
        "failed evidence is not cited"
    );
    let fp: String = serde_json::to_string(&("person", "alice", "works_at", "acme")).unwrap();
    let mut r = repo
        .db()
        .query("SELECT VALUE memory_id FROM ONLY type::record('insight_claim', $fp)")
        .bind(("fp", fp.clone()))
        .await
        .unwrap();
    let mid: Option<String> = r.take(0usize).unwrap();
    assert_eq!(mid.as_deref(), Some(p.written[0].as_str()));
    let rel = strings(&repo, "SELECT VALUE relation FROM insight_relation").await;
    assert_eq!(
        rel,
        vec!["conflict_or_revision".to_string()],
        "Globex revises Acme"
    );
    let text = strings(
        &repo,
        "SELECT VALUE text FROM memory WHERE category = 'investigation' ORDER BY text",
    )
    .await;
    assert_eq!(text, vec!["Alice works at Acme", "Alice works at Globex"]);
    assert_eq!(
        rx::extraction_state(&repo, &a.id).await.unwrap(),
        Some("completed".into())
    );
    assert_eq!(count(repo.db(), "index_change").await, 3);
    assert_eq!(
        counters::current(repo.db(), "memories_changed")
            .await
            .unwrap(),
        changed0 + 1
    );

    // Rerun: claims are reused, nothing new written, no bump, sources kept once.
    let again = re::persist_claims(&repo, &a, &evidence, &claims[..1])
        .await
        .unwrap();
    assert!(again.written.is_empty());
    assert_eq!(again.touched, vec![p.written[0].clone()]);
    assert_eq!(again.queued, 1, "not yet indexed, so verified again");
    assert_eq!(count(repo.db(), "insight_claim").await, 2);
    assert_eq!(count(repo.db(), "insight_source").await, 2);
    assert_eq!(
        counters::current(repo.db(), "memories_changed")
            .await
            .unwrap(),
        changed0 + 1
    );
}

#[tokio::test]
async fn extraction_honours_opt_out_tombstones_and_deleted_sources() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "recon_extract_guard").await;
    // Opted out: the job completes, nothing is written.
    let (off, call) = answer(&repo, false).await;
    let evidence = vec![(call.clone(), result("completed"))];
    let p = re::persist_claims(&repo, &off, &evidence, &[claim("Acme", &[&call])])
        .await
        .unwrap();
    assert_eq!(p, re::Persisted::default());
    assert_eq!(
        rx::extraction_state(&repo, &off.id).await.unwrap(),
        Some("completed".into())
    );
    assert_eq!(count(repo.db(), "insight_claim").await, 0);

    // A claim the user deleted from this run is never recreated.
    let (on, call) = answer(&repo, true).await;
    let evidence = vec![(call.clone(), result("completed"))];
    let fp = serde_json::to_string(&("person", "alice", "works_at", "acme")).unwrap();
    repo.db()
        .query("CREATE memory_tombstone SET memory_id = 'gone', fingerprint = $fp, run_ids = [$run], created_at = 'now'")
        .bind(("fp", fp))
        .bind(("run", on.run_id.clone().unwrap()))
        .await
        .unwrap()
        .check()
        .unwrap();
    let p = re::persist_claims(
        &repo,
        &on,
        &evidence,
        &[claim("Acme", &[&call]), claim("Globex", &[&call])],
    )
    .await
    .unwrap();
    assert_eq!(p.written.len(), 1);
    assert_eq!(count(repo.db(), "insight_claim").await, 1);

    // A deleted thread fails the extraction and writes nothing.
    let (gone, call) = answer(&repo, true).await;
    assert!(
        argos_osint_core::storage::recon::delete_thread(&repo, &gone.thread_id)
            .await
            .unwrap()
    );
    let before = count(repo.db(), "memory").await;
    assert!(re::persist_claims(
        &repo,
        &gone,
        &[(call.clone(), result("completed"))],
        &[claim("Umbrella", &[&call])]
    )
    .await
    .is_err());
    assert_eq!(count(repo.db(), "memory").await, before);
}

/// Fixed corpus for the recall stopgap. The expected order is locked in so
/// the later swap to vector recall can be checked against it.
const CORPUS: [(&str, &str, &str, &str); 7] = [
    (
        "m1",
        "Northwind ferry timetable changed in March",
        "claim",
        "active",
    ),
    (
        "m2",
        "The Northwind ferry operator is Blue Line Shipping",
        "claim",
        "active",
    ),
    (
        "m3",
        "Ferry crossings to the island were cancelled by storms",
        "claim",
        "active",
    ),
    (
        "m4",
        "Acme Corp moved its headquarters to Denver",
        "claim",
        "active",
    ),
    (
        "m5",
        "Northwind ferry timetable archived",
        "claim",
        "archived",
    ),
    ("m6", "Northwind ferry timetable actor", "actor", "active"),
    ("m7", "Northwind harbour opens at dawn", "claim", "active"),
];

#[tokio::test]
async fn full_text_recall_stopgap_locks_in_results_for_a_fixed_corpus() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "recon_recall").await;
    for (id, text, kind, life) in CORPUS {
        repo.db()
            .query("CREATE type::record('memory', $id) SET text = $t, memory_kind = $k, lifecycle = $l")
            .bind(("id", id))
            .bind(("t", text))
            .bind(("k", kind))
            .bind(("l", life))
            .await
            .unwrap()
            .check()
            .unwrap();
    }
    let ids = |hits: Vec<argos_osint_core::storage::recall::RecallHit>| {
        hits.into_iter().map(|h| h.id).collect::<Vec<_>>()
    };
    let got = ids(
        re::recall_for_turn(&repo, "no-thread", "northwind ferry timetable")
            .await
            .unwrap(),
    );
    eprintln!("recall order: {got:?}");
    assert_eq!(
        got,
        vec!["m1", "m2", "m3", "m7"],
        "archived and actor memories are left out"
    );
    let got = ids(re::recall_for_turn(&repo, "no-thread", "Acme headquarters")
        .await
        .unwrap());
    assert_eq!(got, vec!["m4"]);
    assert!(re::recall_for_turn(&repo, "no-thread", "zeppelin")
        .await
        .unwrap()
        .is_empty());
    assert!(re::recall_for_turn(&repo, "no-thread", "  ")
        .await
        .unwrap()
        .is_empty());
    // A thread that opted out of insights recalls nothing.
    let t = rt::new_thread(&repo, "quiet").await.unwrap();
    assert!(re::recall_for_turn(&repo, &t.id, "northwind ferry")
        .await
        .unwrap()
        .is_empty());
    assert!(rt::set_recall_insights(&repo, &t.id, true).await.unwrap());
    assert_eq!(
        ids(re::recall_for_turn(&repo, &t.id, "Acme headquarters")
            .await
            .unwrap()),
        vec!["m4"]
    );
}
