//! The Recon storage dispatch (`recon::backend::ReconStore`): the same
//! sequence of Recon writes and reads against SQLite and SurrealDB gives
//! the same observable results. Both modes via `scripts/test_surreal.py`.
#![cfg(feature = "surreal")]

mod support;

use argos_osint_core::osint::ToolResult;
use argos_osint_core::recon::backend::ReconStore;
use argos_osint_core::recon::RunLimits;
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
            std::slice::from_ref(&done),
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
        // Call ids are random per backend: compare the reference shape.
        .map(|e| e.reference.source_id.replace(&done, "<done>"))
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

/// The TUI/CLI calls (Recon switch step 4): drafts, selection, run state,
/// call listings and delete agree across backends.
type UiSeen = (
    Option<String>,
    String,
    i64,
    usize,
    Option<String>,
    bool,
    bool,
);

async fn ui_scenario(store: &ReconStore) -> UiSeen {
    let thread = store.new_thread("UI thread").await.unwrap();
    store.save_draft(&thread.id, "half typed", 7).await.unwrap();
    store.select_thread(&thread.id).await.unwrap();
    let last = store
        .last_thread()
        .await
        .unwrap()
        .map(|id| (id == thread.id).to_string());
    let t = store.get_thread(&thread.id).await.unwrap().unwrap();
    let run = store
        .new_run_with_models(&thread.id, "turn-1", ["m1", "m2", "m3"], LIMITS)
        .await
        .unwrap();
    store
        .queue_call(
            TOOL,
            &json!({"email": "ui@example.org"}),
            "recon",
            Some(&run.id),
            Some(&thread.id),
            Some("turn-1"),
        )
        .await
        .unwrap();
    let calls = store.all_calls_for_thread(&thread.id).await.unwrap().len();
    let state = store.latest_run_state(&thread.id).await.unwrap();
    let deleted = store.delete_thread(&thread.id, false).await.unwrap();
    let gone = store.get_thread(&thread.id).await.unwrap().is_none()
        && store.last_thread().await.unwrap().is_none();
    (last, t.draft, t.scroll, calls, state, deleted, gone)
}

#[tokio::test]
async fn surreal_dispatch_round_trips() {
    let Some(server) = Server::start() else {
        return;
    };
    let surreal = ReconStore::from_repository(server.repo("argos", "recon_dispatch").await);
    let a = scenario(&surreal).await;
    let ua = ui_scenario(&surreal).await;
    assert_eq!(ua.0.as_deref(), Some("true"));
    assert_eq!((ua.1.as_str(), ua.2, ua.3), ("half typed", 7, 1));
    assert!(ua.5 && ua.6);
    // Deleting with insights is ported (18f6b74): a missing thread is false.
    assert!(!surreal.delete_thread("missing", true).await.unwrap());
    assert_eq!(a.roles, ["user", "assistant"]);
    assert_eq!(a.statuses, ["completed", "interrupted"]);
    assert!(a.evidence == ["call:<done>"], "{:?}", a.evidence);
}
