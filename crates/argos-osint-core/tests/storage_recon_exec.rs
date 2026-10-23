//! The Recon executor's writes on SurrealDB (`storage::recon_exec`):
//! `add_answer`, `recover_runs`, the model operation ledger and the
//! investigation strategies, against the SQLite `Store` behaviour. Both
//! modes via `scripts/test_surreal.py`.
#![cfg(feature = "surreal")]

mod support;

use argos_osint_core::osint::ToolResult;
use argos_osint_core::recon::RunLimits;
use argos_osint_core::storage::recon_exec as rx;
use argos_osint_core::storage::recon_runs::{self as rr, CallFilter};
use argos_osint_core::storage::recon_threads as rt;
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
        "source_url": "https://example.org", "retrieved_at": "2026-10-23T00:00:00Z",
        "observations": {"name": "A"}, "raw": "{}", "error": null,
        "cached": false, "truncated": false
    }))
    .unwrap()
}

#[tokio::test]
async fn answers_cite_evidence_and_memories_and_finalize_the_run() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "recon_exec").await;
    repo.db()
        .query("CREATE memory:m1 SET text = 'one'; CREATE memory:m2 SET text = 'two';")
        .await
        .unwrap()
        .check()
        .unwrap();
    let t = rt::new_thread(&repo, "exec").await.unwrap();
    let run = rr::new_run(&repo, &t.id, "turn", ["r", "p", "s"], LIMITS)
        .await
        .unwrap();
    let inputs = json!({"email": "a@example.org"});
    let call = rr::queue_call(
        &repo,
        TOOL,
        &inputs,
        "recon",
        Some(&run.id),
        Some(&t.id),
        None,
    )
    .await
    .unwrap();
    assert!(rr::finish_call(&repo, &call, &result()).await.unwrap());
    rt::add_message(&repo, &t.id, "user", "who?", None)
        .await
        .unwrap();

    let op = format!("{}-synthesis", run.id);
    rx::begin_operation(&repo, &op, Some(&run.id), None, "synthesis", 1, "dr")
        .await
        .unwrap();
    rx::set_operation_draft(&repo, &op, "draft two")
        .await
        .unwrap();
    rx::begin_operation(&repo, &op, Some(&run.id), None, "synthesis", 2, "")
        .await
        .unwrap();
    let ops = rx::operations(&repo, &run.id, "synthesis").await.unwrap();
    assert_eq!(
        (ops.len(), ops[0].generation, ops[0].draft.as_str()),
        (1, 2, "draft two"),
        "an empty draft keeps the streamed one"
    );
    assert!(!rx::has_final_answer_for_run(&repo, &run.id).await.unwrap());

    // A missing evidence call fails the whole answer.
    assert!(
        rx::add_answer(&repo, &t.id, &run.id, "x", &["nope".into()], &[])
            .await
            .is_err()
    );
    assert_eq!(rt::list_messages(&repo, &t.id).await.unwrap().len(), 1);
    // The run must be running on this thread.
    let other = rt::new_thread(&repo, "other").await.unwrap();
    assert!(rx::add_answer(&repo, &other.id, &run.id, "x", &[], &[])
        .await
        .is_err());

    let memories = vec!["m2".to_string(), "gone".to_string(), "m1".to_string()];
    let answer = rx::add_answer(
        &repo,
        &t.id,
        &run.id,
        "A is A",
        std::slice::from_ref(&call),
        &memories,
    )
    .await
    .unwrap();
    assert_eq!((answer.sequence, answer.role.as_str()), (2, "assistant"));
    let evidence = rr::answer_evidence(&repo, &answer.id).await.unwrap();
    assert_eq!(evidence.len(), 1);
    let mut r = repo
        .db()
        .query(
            "SELECT VALUE memory_id FROM recon_message_memory WHERE message_id = $m ORDER BY ordinal",
        )
        .bind(("m", answer.id.clone()))
        .await
        .unwrap();
    assert_eq!(
        r.take::<Vec<String>>(0usize).unwrap(),
        vec!["m2".to_string(), "m1".to_string()],
        "only stored memories, in citation order"
    );
    assert_eq!(
        rx::extraction_state(&repo, &answer.id).await.unwrap(),
        Some("queued".into())
    );
    let ops = rx::operations(&repo, &run.id, "synthesis").await.unwrap();
    assert_eq!(
        (ops[0].status.as_str(), ops[0].final_message_id.as_deref()),
        ("final", Some(answer.id.as_str()))
    );
    assert!(rx::has_final_answer_for_run(&repo, &run.id).await.unwrap());
    let got = rr::get_run(&repo, &run.id).await.unwrap().unwrap();
    assert_eq!(
        (got.state.as_str(), got.stage.as_str()),
        ("running", "complete")
    );

    // Once the run is no longer running, answers are refused.
    assert!(rr::cancel_run(&repo, &run.id).await.unwrap());
    assert!(rx::add_answer(&repo, &t.id, &run.id, "late", &[], &[])
        .await
        .is_err());

    rx::finish_operation(&repo, &op, "failed", None)
        .await
        .unwrap();
    let ops = rx::operations(&repo, &run.id, "synthesis").await.unwrap();
    assert_eq!(
        (ops[0].status.as_str(), ops[0].final_message_id.as_deref()),
        ("failed", Some(answer.id.as_str())),
        "no final message keeps the stored one"
    );
}

#[tokio::test]
async fn recovery_interrupts_runs_calls_and_requeues_extraction() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "recon_recover").await;
    let t = rt::new_thread(&repo, "recover").await.unwrap();
    let live = rr::new_run(&repo, &t.id, "a", ["r", "p", "s"], LIMITS)
        .await
        .unwrap();
    let done = rr::new_run(&repo, &t.id, "b", ["r", "p", "s"], LIMITS)
        .await
        .unwrap();
    assert!(
        rr::set_run(&repo, &done.id, "completed", "complete", None, None)
            .await
            .unwrap()
    );
    let inputs = json!({"email": "a@example.org"});
    let queued = rr::queue_call(
        &repo,
        TOOL,
        &inputs,
        "recon",
        Some(&live.id),
        Some(&t.id),
        None,
    )
    .await
    .unwrap();
    let finished = rr::queue_call(
        &repo,
        TOOL,
        &inputs,
        "recon",
        Some(&live.id),
        Some(&t.id),
        None,
    )
    .await
    .unwrap();
    assert!(rr::finish_call(&repo, &finished, &result()).await.unwrap());
    let answer = rx::add_answer(&repo, &t.id, &live.id, "ans", &[], &[])
        .await
        .unwrap();
    rx::set_extraction_state(&repo, &answer.id, "running")
        .await
        .unwrap();

    assert_eq!(rx::recover_runs(&repo).await.unwrap(), 1);
    let live = rr::get_run(&repo, &live.id).await.unwrap().unwrap();
    assert_eq!(
        (live.state.as_str(), live.stage.as_str()),
        ("interrupted", "interrupted")
    );
    let done = rr::get_run(&repo, &done.id).await.unwrap().unwrap();
    assert_eq!(done.state, "completed");
    let calls = rr::calls(&repo, CallFilter::Run(&live.id)).await.unwrap();
    let status = |id: &str| {
        calls
            .iter()
            .find(|c| c.id == id)
            .map(|c| c.status.clone())
            .unwrap()
    };
    assert_eq!(status(&queued), "interrupted");
    assert_eq!(status(&finished), "completed");
    assert_eq!(
        rx::extraction_state(&repo, &answer.id).await.unwrap(),
        Some("queued".into())
    );
    assert_eq!(rx::recover_runs(&repo).await.unwrap(), 0, "idempotent");
}

#[tokio::test]
async fn strategies_report_the_latest_kind_per_thread() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "recon_strategy").await;
    let t = rt::new_thread(&repo, "s").await.unwrap();
    assert_eq!(rx::latest_strategy_kind(&repo, &t.id).await.unwrap(), None);
    rx::record_strategy(&repo, &t.id, "run1", "breadth", "start wide", None, None)
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    rx::record_strategy(
        &repo,
        &t.id,
        "run2",
        "depth",
        "found a lead",
        Some("breadth"),
        Some("lead"),
    )
    .await
    .unwrap();
    assert_eq!(
        rx::latest_strategy_kind(&repo, &t.id).await.unwrap(),
        Some("depth".into())
    );
    assert_eq!(
        rx::latest_strategy_kind(&repo, "other").await.unwrap(),
        None
    );
}
