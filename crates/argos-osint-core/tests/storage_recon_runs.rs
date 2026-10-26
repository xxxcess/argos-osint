//! Recon runs, tool calls, answer evidence and the tool cache on SurrealDB
//! (`storage::recon_runs`), against the SQLite `Store` behaviour. Both
//! modes via `scripts/test_surreal.py`.
#![cfg(feature = "surreal")]

mod support;

use argos_osint_core::osint::ToolResult;
use argos_osint_core::recon::{Plan, RunLimits};
use argos_osint_core::storage::recon_runs::{self as rr, CallFilter};
use argos_osint_core::storage::{recon, recon_threads as rt};
use serde_json::json;
use support::*;

const LIMITS: RunLimits = RunLimits {
    max_rounds: 6,
    max_calls: 12,
    turn_seconds: 300,
};
const TOOL: &str = "hunter_person_enrichment";

fn result(status: &str, retrieved_at: &str) -> ToolResult {
    serde_json::from_value(json!({
        "tool_id": TOOL, "inputs": {"email": "a@example.org"}, "status": status,
        "source_url": "https://example.org", "retrieved_at": retrieved_at,
        "observations": {"name": "A"}, "raw": "{}", "error": null,
        "cached": false, "truncated": false
    }))
    .unwrap()
}

#[tokio::test]
async fn runs_follow_the_sqlite_state_machine() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "recon_runs").await;
    let t = rt::new_thread(&repo, "runs").await.unwrap();
    assert!(rr::new_run(&repo, "nope", "turn", ["r", "p", "s"], LIMITS)
        .await
        .is_err());
    let bad = RunLimits {
        max_rounds: 9,
        ..LIMITS
    };
    assert!(rr::new_run(&repo, &t.id, "turn", ["r", "p", "s"], bad)
        .await
        .is_err());
    let run = rr::new_run(&repo, &t.id, "turn-1", ["recon", "picker", "synth"], LIMITS)
        .await
        .unwrap();
    let got = rr::get_run(&repo, &run.id).await.unwrap().unwrap();
    assert_eq!(
        (
            got.state.as_str(),
            got.tool_picker_model.as_str(),
            got.max_calls
        ),
        ("running", "picker", 12)
    );
    assert_eq!(
        rr::latest_run_state(&repo, &t.id).await.unwrap().as_deref(),
        Some("running")
    );
    assert_eq!(
        rr::latest_run_states(&repo, std::slice::from_ref(&t.id))
            .await
            .unwrap()
            .get(&t.id)
            .map(String::as_str),
        Some("running")
    );
    assert!(rr::latest_resumable_run(&repo, &t.id)
        .await
        .unwrap()
        .is_none());

    let plan = Plan {
        objective: "find A".into(),
        ..Plan::default()
    };
    assert!(rr::set_run(
        &repo,
        &run.id,
        "failed",
        "failed",
        Some(&plan),
        Some("boom")
    )
    .await
    .unwrap());
    assert!(rr::set_run(&repo, &run.id, "failed", "failed", None, None)
        .await
        .unwrap());
    let got = rr::get_run(&repo, &run.id).await.unwrap().unwrap();
    assert!(
        got.plan_json.unwrap().contains("find A"),
        "no plan keeps it"
    );
    assert_eq!(got.error, None);
    assert_eq!(
        rr::latest_resumable_run(&repo, &t.id)
            .await
            .unwrap()
            .map(|r| r.id),
        Some(run.id.clone())
    );
    assert!(
        !rr::cancel_run(&repo, &run.id).await.unwrap(),
        "not running"
    );
    assert!(rr::restart_run(&repo, &run.id).await.unwrap());
    assert!(rr::cancel_run(&repo, &run.id).await.unwrap());
    assert!(
        !rr::set_run(&repo, &run.id, "running", "x", None, None)
            .await
            .unwrap(),
        "cancelled is final"
    );
    let second = rr::new_run(&repo, &t.id, "turn-2", ["r", "", "s"], LIMITS)
        .await
        .unwrap();
    assert_eq!(
        rr::runs_for_thread(&repo, &t.id)
            .await
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect::<Vec<_>>(),
        vec![run.id.clone(), second.id.clone()]
    );
    assert!(recon::delete_thread(&repo, &t.id).await.unwrap());
    assert!(
        rr::get_run(&repo, &run.id).await.unwrap().is_none(),
        "runs cascade"
    );
}

#[tokio::test]
async fn calls_evidence_and_cache_match_the_sqlite_store() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "recon_calls").await;
    let t = rt::new_thread(&repo, "calls").await.unwrap();
    let run = rr::new_run(&repo, &t.id, "turn", ["r", "p", "s"], LIMITS)
        .await
        .unwrap();
    let inputs = json!({"email": "a@example.org"});
    assert!(
        rr::queue_call(&repo, "no_such_tool", &inputs, "recon", None, None, None)
            .await
            .is_err()
    );
    assert!(
        rr::queue_call(
            &repo,
            TOOL,
            &inputs,
            "recon",
            Some(&run.id),
            Some("other"),
            None
        )
        .await
        .is_err(),
        "run on another thread"
    );
    let c1 = rr::queue_call(
        &repo,
        TOOL,
        &inputs,
        "recon",
        Some(&run.id),
        Some(&t.id),
        Some("turn"),
    )
    .await
    .unwrap();
    assert!(rr::inflight_duplicate(&repo, TOOL, &inputs).await.unwrap());
    assert!(!rr::inflight_duplicate(&repo, TOOL, &json!({"email": "b"}))
        .await
        .unwrap());
    assert!(
        rr::finish_call(&repo, &c1, &result("completed", "2026-10-23T00:00:00Z"))
            .await
            .unwrap()
    );
    assert!(
        !rr::finish_call(&repo, &c1, &result("failed", ""))
            .await
            .unwrap(),
        "finished calls stay finished"
    );
    assert!(!rr::inflight_duplicate(&repo, TOOL, &inputs).await.unwrap());
    let c2 = rr::queue_call(
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
    let manual = rr::queue_call(&repo, TOOL, &inputs, "manual", None, None, None)
        .await
        .unwrap();
    rr::finish_call(&repo, &manual, &result("no_results", ""))
        .await
        .unwrap();
    let ids =
        |v: Vec<argos_osint_core::recon::Call>| v.into_iter().map(|c| c.id).collect::<Vec<_>>();
    assert_eq!(
        ids(rr::calls(&repo, CallFilter::Run(&run.id)).await.unwrap()),
        vec![c1.clone(), c2.clone()]
    );
    assert_eq!(
        ids(rr::calls(&repo, CallFilter::ThreadCompleted(&t.id))
            .await
            .unwrap()),
        vec![c1.clone()]
    );
    assert_eq!(
        ids(rr::calls(&repo, CallFilter::Manual).await.unwrap()),
        vec![manual.clone()]
    );
    let first = &rr::calls(&repo, CallFilter::Run(&run.id)).await.unwrap()[0];
    assert_eq!((first.attempts, first.inputs.clone()), (1, inputs.clone()));
    assert_eq!(first.result.as_ref().unwrap().status, "completed");
    assert!(rr::attach_call(&repo, &manual, &t.id).await.unwrap());
    assert!(
        !rr::attach_call(&repo, &c2, &t.id).await.unwrap(),
        "only finished manual calls"
    );
    assert_eq!(
        rr::calls(&repo, CallFilter::Thread(&t.id))
            .await
            .unwrap()
            .len(),
        3
    );

    // Evidence: only existing calls; unfinished calls are not returned.
    let answer = rt::add_message(&repo, &t.id, "assistant", "A is A", Some(&run.id))
        .await
        .unwrap();
    assert!(
        rr::link_evidence(&repo, &answer.id, &["call-missing".into()])
            .await
            .is_err()
    );
    rr::link_evidence(&repo, &answer.id, &[c1.clone(), c2.clone()])
        .await
        .unwrap();
    let ev = rr::answer_evidence(&repo, &answer.id).await.unwrap();
    assert_eq!(
        ev.iter().map(|(id, _)| id.clone()).collect::<Vec<_>>(),
        vec![c1.clone()]
    );

    // Cache: expiry and freshness cutoff.
    rr::cache_put(&repo, "k", &result("completed", "2026-10-20T00:00:00Z"), 60)
        .await
        .unwrap();
    rr::cache_put(&repo, "gone", &result("completed", ""), 0)
        .await
        .unwrap();
    assert!(rr::cache_get_fresh(&repo, "k", None)
        .await
        .unwrap()
        .is_some());
    assert!(
        rr::cache_get_fresh(&repo, "k", Some("2026-10-19T00:00:00Z"))
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        rr::cache_get_fresh(&repo, "k", Some("2026-10-21T00:00:00Z"))
            .await
            .unwrap()
            .is_none()
    );
    assert!(rr::cache_get_fresh(&repo, "gone", None)
        .await
        .unwrap()
        .is_none());
    assert!(rr::cache_get_fresh(&repo, "missing", None)
        .await
        .unwrap()
        .is_none());

    // Deleting the answer drops its evidence rows.
    repo.db()
        .query("DELETE type::record('recon_message', $m)")
        .bind(("m", answer.id.clone()))
        .await
        .unwrap()
        .check()
        .unwrap();
    assert!(rr::answer_evidence(&repo, &answer.id)
        .await
        .unwrap()
        .is_empty());
    let mut r = repo
        .db()
        .query("RETURN count(SELECT * FROM recon_message_evidence)")
        .await
        .unwrap();
    assert_eq!(r.take::<Option<i64>>(0usize).unwrap(), Some(0));
}
