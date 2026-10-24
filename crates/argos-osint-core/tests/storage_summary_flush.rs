//! The summary flush pool on SurrealDB (`storage::summary_flush`): a
//! deterministic publish caches the local result and queues one task per
//! cache key; a drain without a secret completes it with the cached value,
//! and a model result is never replaced by a fallback.
#![cfg(feature = "surreal")]

mod support;

use argos_osint_core::storage::{bootstrap, summary_flush};
use argos_osint_core::summarization::{deterministic_follow_up, flush_request, SummarizationMode};
use support::*;

#[tokio::test]
async fn publish_queues_once_and_drain_completes_without_a_secret() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "summary_flush").await;
    bootstrap::ensure_schema(repo.db()).await.unwrap();
    let text = "Observed output of a tool call with several lines of text.";
    let mut req = flush_request(
        SummarizationMode::ToolObservation,
        "call-1",
        "call-1",
        text,
        "tool",
        400,
    );
    req.provider = "openai".into();
    req.model = "m".into();
    let det = deterministic_follow_up(text, 400);
    let first = summary_flush::publish(&repo, &req, det.clone())
        .await
        .unwrap();
    assert!(first.fallback);
    summary_flush::publish(&repo, &req, det.clone())
        .await
        .unwrap();
    let task = format!("sum-task-{}", req.cache_key());
    assert_eq!(
        summary_flush::task_state(&repo, &task)
            .await
            .unwrap()
            .as_deref(),
        Some("queued")
    );
    let mut r = repo
        .db()
        .query("RETURN count(SELECT 1 FROM task WHERE pool = 'summary')")
        .await
        .unwrap()
        .check()
        .unwrap();
    let n: Option<i64> = r.take(0).unwrap();
    assert_eq!(n, Some(1));
    let hit = summary_flush::cache_get(&repo, &req)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(hit.content, det.content);

    let out = summary_flush::drain(&repo, "w1", || None, 60)
        .await
        .unwrap();
    assert_eq!(out.len(), 1, "{out:?}");
    assert_eq!(out[0].tag, "cached_deterministic_no_secret");
    assert_eq!(
        summary_flush::task_state(&repo, &task)
            .await
            .unwrap()
            .as_deref(),
        Some("completed")
    );
    assert!(summary_flush::drain(&repo, "w1", || None, 60)
        .await
        .unwrap()
        .is_empty());

    let mut model = det.clone();
    model.fallback = false;
    model.content = "model summary".into();
    summary_flush::cache_put(&repo, &req, &model).await.unwrap();
    summary_flush::cache_put(&repo, &req, &det).await.unwrap();
    let hit = summary_flush::cache_get(&repo, &req)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(hit.content, "model summary");
    let again = summary_flush::publish(&repo, &req, det).await.unwrap();
    assert_eq!(again.content, "model summary");
}
