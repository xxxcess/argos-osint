//! The Recon storage dispatch (`recon::backend::ReconStore`): the same
//! sequence of Recon writes and reads against SQLite and SurrealDB gives
//! the same observable results. Both modes via `scripts/test_surreal.py`.
#![cfg(feature = "surreal")]

mod support;

use argos_osint_core::osint::ToolResult;
use argos_osint_core::recon::backend::{surreal_selected, ReconStore};
use argos_osint_core::recon::RunLimits;
use argos_osint_core::store::Store;
use serde_json::json;
use support::*;

const LIMITS: RunLimits = RunLimits {
    max_rounds: 6,
    max_calls: 12,
    turn_seconds: 300,
};
const TOOL: &str = "hunter_person_enrichment";

fn result() -> ToolResult {
    serde_json::from_value(json!({
        "tool_id": TOOL, "inputs": {"email": "a@example.org"}, "status": "completed",
        "source_url": "https://example.org/a", "retrieved_at": "2026-10-23T00:00:00Z",
        "observations": {"name": "A"}, "raw": "{}", "error": null,
        "cached": false, "truncated": false
    }))
    .unwrap()
}

/// What one backend observed, compared across backends.
#[derive(Debug, PartialEq)]
struct Seen {
    titles: Vec<String>,
    roles: Vec<String>,
    statuses: Vec<String>,
    stage: String,
    answer_cites: usize,
    evidence: Vec<String>,
    tool_on: bool,
    tool_off: bool,
    entities: Vec<(String, String)>,
    cached: bool,
    duplicate_after: bool,
}

async fn scenario(store: &ReconStore) -> Seen {
    let thread = store.new_thread("Who is A").await.unwrap();
    store
        .add_message(&thread.id, "user", "who is a@example.org", None)
        .await
        .unwrap();
    let run = store
        .new_run_with_models(&thread.id, "turn-1", ["m1", "m2", "m3"], LIMITS)
        .await
        .unwrap();
    let inputs = json!({"email": "a@example.org"});
    let done = store
        .queue_call(
            TOOL,
            &inputs,
            "recon",
            Some(&run.id),
            Some(&thread.id),
            Some("turn-1"),
        )
        .await
        .unwrap();
    store.finish_call(&done, &result()).await.unwrap();
    store
        .queue_call(
            TOOL,
            &json!({"email": "b@example.org"}),
            "recon",
            Some(&run.id),
            Some(&thread.id),
            Some("turn-1"),
        )
        .await
        .unwrap();
    assert_eq!(
        store
            .settle_open_calls(&run.id, "interrupted")
            .await
            .unwrap(),
        1
    );
    assert!(store.settle_open_calls(&run.id, "done").await.is_err());
    store
        .set_run(&run.id, "running", "synthesizing", None, None)
        .await
        .unwrap();
    assert_eq!(store.synthesis_operation(&run.id).await.unwrap(), None);
    let answer = store
        .add_answer(
            &thread.id,
            &run.id,
            &format!("A is A [{done}]"),
            &[done.clone()],
            &[],
        )
        .await
        .unwrap();
    store
        .set_extraction(&answer.id, "completed", None)
        .await
        .unwrap();
    store.relink_sources(&run.id, &answer.id).await.unwrap();
    let evidence = store
        .evidence(&[(done.clone(), result())])
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.reference.source_id)
        .collect();
    let tool_on = store.tool_enabled(TOOL).await.unwrap();
    store.set_tool_enabled(TOOL, false).await.unwrap();
    let tool_off = store.tool_enabled(TOOL).await.unwrap();
    store
        .link_entity(&thread.id, "email", "a@example.org", Some(&done))
        .await
        .unwrap();
    store.cache_put("k1", &result(), 3600).await.unwrap();
    Seen {
        titles: store
            .list_threads("")
            .await
            .unwrap()
            .into_iter()
            .map(|t| t.title)
            .collect(),
        roles: store
            .list_messages(&thread.id)
            .await
            .unwrap()
            .into_iter()
            .map(|m| m.role)
            .collect(),
        statuses: {
            let mut s: Vec<String> = store
                .calls_for_run(&run.id)
                .await
                .unwrap()
                .into_iter()
                .map(|c| c.status)
                .collect();
            s.sort();
            s
        },
        stage: store.get_run(&run.id).await.unwrap().unwrap().stage,
        answer_cites: store.answer_evidence(&answer.id).await.unwrap().len(),
        evidence,
        tool_on,
        tool_off,
        entities: store.thread_entities(&thread.id).await.unwrap(),
        cached: store.cache_get("k1").await.unwrap().is_some(),
        duplicate_after: store.inflight_duplicate(TOOL, &inputs).await.unwrap(),
    }
}

#[tokio::test]
async fn sqlite_and_surreal_dispatch_agree() {
    // Core tests never initialize the storage runtime: SQLite stays selected.
    assert!(!surreal_selected());
    let Some(server) = Server::start() else {
        return;
    };
    let dir = std::env::temp_dir().join(format!("argos-dispatch-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let sqlite = ReconStore::from_sqlite(Store::open(&dir.join("argos.db")).unwrap());
    assert!(!sqlite.is_surreal());
    let surreal = ReconStore::from_repository(server.repo("argos", "recon_dispatch").await);
    assert!(surreal.is_surreal());
    let a = scenario(&sqlite).await;
    let b = scenario(&surreal).await;
    assert_eq!(a, b);
    assert_eq!(a.roles, ["user", "assistant"]);
    assert_eq!(a.statuses, ["completed", "interrupted"]);
    assert!(
        a.evidence.iter().all(|s| s.starts_with("call:")),
        "{:?}",
        a.evidence
    );
    assert!(sqlite.sqlite("probe", |_| Ok(())).is_ok());
    let err = surreal.sqlite("probe", |_| Ok(())).unwrap_err();
    assert_eq!(err.to_string(), "probe is not on SurrealDB yet");
    let _ = std::fs::remove_dir_all(&dir);
}
