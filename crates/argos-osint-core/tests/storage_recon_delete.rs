//! The full Recon `delete_thread` cascade on SurrealDB
//! (`storage::recon::delete_thread`) over the thread, messages, runs,
//! calls, evidence, executor rows and cache, against the legacy store
//! behaviour (insights kept). Both modes via `scripts/test_surreal.py`.
#![cfg(feature = "surreal")]

mod support;

use argos_osint_core::osint::ToolResult;
use argos_osint_core::recon::RunLimits;
use argos_osint_core::storage::recon_exec as rx;
use argos_osint_core::storage::recon_runs::{self as rr, CallFilter};
use argos_osint_core::storage::{recon, recon_threads as rt};
use serde_json::json;
use support::*;
use surrealdb::types::SurrealValue;

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

async fn n(repo: &argos_osint_core::storage::repo::Repository, sql: &str) -> i64 {
    let mut r = repo.db().query(sql).await.unwrap().check().unwrap();
    r.take::<Option<i64>>(0usize).unwrap().unwrap_or(-1)
}

#[tokio::test]
async fn delete_thread_cascades_over_the_recon_tables() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "recon_delete").await;
    let t = rt::new_thread(&repo, "doomed").await.unwrap();
    let keep = rt::new_thread(&repo, "kept").await.unwrap();
    let run = rr::new_run(&repo, &t.id, "turn", ["r", "p", "s"], LIMITS)
        .await
        .unwrap();
    let kept_run = rr::new_run(&repo, &keep.id, "turn", ["r", "p", "s"], LIMITS)
        .await
        .unwrap();
    let inputs = json!({"email": "a@example.org"});
    let call = |run: String, thread: String| {
        let (repo, inputs) = (&repo, &inputs);
        async move {
            rr::queue_call(repo, TOOL, inputs, "recon", Some(&run), Some(&thread), None)
                .await
                .unwrap()
        }
    };
    let plain = call(run.id.clone(), t.id.clone()).await;
    let cited = call(run.id.clone(), t.id.clone()).await;
    let derived = call(run.id.clone(), t.id.clone()).await;
    let inflight = call(run.id.clone(), t.id.clone()).await;
    let other = call(kept_run.id.clone(), keep.id.clone()).await;
    for c in [&plain, &cited, &derived] {
        assert!(rr::finish_call(&repo, c, &result()).await.unwrap());
    }
    let manual = rr::queue_call(&repo, TOOL, &inputs, "manual", None, None, None)
        .await
        .unwrap();
    assert!(rr::finish_call(&repo, &manual, &result()).await.unwrap());
    assert!(rr::attach_call(&repo, &manual, &t.id).await.unwrap());
    rr::cache_put(&repo, "cache-key", &result(), 3600)
        .await
        .unwrap();

    let op = format!("{}-synthesis", run.id);
    rx::begin_operation(&repo, &op, Some(&run.id), None, "synthesis", 1, "")
        .await
        .unwrap();
    rx::record_strategy(&repo, &t.id, &run.id, "breadth", "why", None, None)
        .await
        .unwrap();
    rx::record_strategy(&repo, &keep.id, &kept_run.id, "depth", "why", None, None)
        .await
        .unwrap();
    let answer = rx::add_answer(
        &repo,
        &t.id,
        &run.id,
        "ans",
        std::slice::from_ref(&plain),
        &[],
    )
    .await
    .unwrap();
    rt::select_thread(&repo, &t.id).await.unwrap();
    repo.db()
        .query(
            "CREATE insight_source SET fingerprint = 'fp', thread_id = $t, run_id = $r,
               answer_id = $a, call_id = $c;
             CREATE insight_derivation_evidence SET derivation_id = 'd1', source_id = $d, source_revision = '', evidence_json = '{}';
             CREATE brain_actor_mention SET actor_id = 'actor-1', origin_kind = 'thread',
               origin_id = $t, created_at = '' RETURN NONE;",
        )
        .bind(("t", t.id.clone()))
        .bind(("r", run.id.clone()))
        .bind(("a", answer.id.clone()))
        .bind(("c", cited.clone()))
        .bind(("d", format!("call:{derived}#page2")))
        .await
        .unwrap()
        .check()
        .unwrap();

    assert!(recon::delete_thread(&repo, &t.id).await.unwrap());
    assert!(
        !recon::delete_thread(&repo, &t.id).await.unwrap(),
        "already gone"
    );
    assert!(!recon::delete_thread(&repo, "nope").await.unwrap());

    assert!(recon::get_thread(&repo, &t.id).await.unwrap().is_none());
    assert!(rr::get_run(&repo, &run.id).await.unwrap().is_none());
    assert_eq!(rt::last_thread(&repo).await.unwrap(), None);
    let tid = &t.id;
    for (what, sql, want) in [
        ("messages", format!("RETURN count(SELECT id FROM recon_message WHERE thread_id = '{tid}')"), 0),
        ("evidence", format!("RETURN count(SELECT id FROM recon_message_evidence WHERE message_id = '{}')", answer.id), 0),
        ("extraction", "RETURN count(SELECT id FROM extraction_job)".to_string(), 0),
        ("operations", "RETURN count(SELECT id FROM recon_model_operation)".to_string(), 0),
        ("strategies", "RETURN count(SELECT id FROM investigation_strategy)".to_string(), 1),
        ("mentions", format!("RETURN count(SELECT id FROM brain_actor_mention WHERE origin_id = '{tid}')"), 0),
        ("counter", format!("RETURN count(SELECT id FROM counter:`recon_message:{tid}`)"), 0),
        ("cache", "RETURN count(SELECT id FROM osint_cache)".to_string(), 1),
        ("detached sources", "RETURN count(SELECT id FROM insight_source WHERE deleted_origin = true AND thread_id = NONE AND run_id = NONE AND answer_id = 'deleted-origin')".to_string(), 1),
    ] {
        assert_eq!(n(&repo, &sql).await, want, "{what}");
    }
    let all: Vec<_> = [&plain, &cited, &derived, &inflight, &other, &manual]
        .iter()
        .map(|id| id.to_string())
        .collect();
    let mut r = repo
        .db()
        .query(
            "SELECT meta::id(id) AS id, status, thread_id, run_id FROM osint_call
             WHERE meta::id(id) INSIDE $ids",
        )
        .bind(("ids", all))
        .await
        .unwrap();
    #[derive(Debug, SurrealValue)]
    struct Row {
        id: String,
        status: String,
        thread_id: Option<String>,
        run_id: Option<String>,
    }
    let rows: Vec<Row> = r.take(0usize).unwrap();
    let get = |id: &str| rows.iter().find(|r| r.id == id);
    assert!(get(&plain).is_none(), "uncited recon calls go");
    assert!(get(&inflight).is_none(), "cancelled in-flight calls go too");
    for kept in [&cited, &derived] {
        let row = get(kept).expect("cited calls stay");
        assert_eq!(
            (row.thread_id.as_deref(), row.run_id.as_deref()),
            (None, None)
        );
    }
    assert_eq!(
        get(&manual).unwrap().thread_id,
        None,
        "manual calls are detached"
    );
    assert_eq!(
        get(&other).unwrap().thread_id.as_deref(),
        Some(keep.id.as_str())
    );
    assert_eq!(get(&other).unwrap().status, "queued");
    assert_eq!(
        rr::calls(&repo, CallFilter::Run(&kept_run.id))
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(rr::get_run(&repo, &kept_run.id).await.unwrap().is_some());
}
