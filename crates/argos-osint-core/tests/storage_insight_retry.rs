//! Insight retry admission on SurrealDB (`storage::insight_retry`), the
//! counterpart of `insight_persist::validate_retry`.
//! Run through `python3 scripts/test_surreal.py --mode both`.
#![cfg(feature = "surreal")]

mod support;

use argos_osint_core::investigation::insights::PipelineInput;
use argos_osint_core::storage::{bootstrap, insight_retry};
use support::*;

fn input(kind: &str, owner: &str, generation: u32) -> PipelineInput {
    serde_json::from_value(serde_json::json!({
        "snapshot": {
            "owner_kind": kind, "owner_id": owner, "generation": generation,
            "source_refs": [], "entities": [], "tool_refs": [],
            "coverage_by_stage": [], "assessed_claim_refs": [],
            "memory_refs": [], "key": "k"
        },
        "evidence": [], "objective": "", "surface": "",
        "required_tasks_complete": true, "save_insights": true
    }))
    .expect("pipeline input")
}

#[tokio::test]
async fn validate_retry_refuses_a_missing_owner_and_an_unknown_kind() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "insight_retry_owner").await;
    bootstrap::ensure_schema(repo.db()).await.unwrap();
    let missing = insight_retry::validate_retry(repo.db(), &input("article", "gone", 1))
        .await
        .unwrap_err();
    assert!(missing
        .to_string()
        .contains("article owner no longer exists"));
    let unknown = insight_retry::validate_retry(repo.db(), &input("nope", "x", 1))
        .await
        .unwrap_err();
    assert!(unknown
        .to_string()
        .contains("unsupported publication owner"));
}

#[tokio::test]
async fn exhausted_stage_is_scoped_to_owner_and_generation() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "insight_retry_exhausted").await;
    bootstrap::ensure_schema(repo.db()).await.unwrap();
    repo.db()
        .query(
            "CREATE insight_stage_checkpoint SET owner_kind = 'article', owner_id = 'a1',
               generation = 2, stage = 'claims', window_id = '', state = 'exhausted',
               updated_at = '2026-10-24T00:00:00Z';",
        )
        .await
        .unwrap()
        .check()
        .unwrap();
    assert!(
        insight_retry::has_exhausted_stage(repo.db(), "article", "a1", 2)
            .await
            .unwrap()
    );
    assert!(
        !insight_retry::has_exhausted_stage(repo.db(), "article", "a1", 1)
            .await
            .unwrap()
    );
    assert!(
        !insight_retry::has_exhausted_stage(repo.db(), "article", "a2", 2)
            .await
            .unwrap()
    );
    // Not failed/completed: never retryable, whatever the checkpoints say.
    assert!(
        !insight_retry::retryable(repo.db(), "running", &input("article", "a1", 2))
            .await
            .unwrap()
    );
}
