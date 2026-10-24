//! Insight pipeline execution state on SurrealDB (`storage::pipeline_exec`):
//! checkpoints, cross-generation reuse, retry operation ids, progress
//! without a running task and the article dispatch fence. Both modes via
//! `scripts/test_surreal.py`.
#![cfg(feature = "surreal")]

mod support;

use argos_osint_core::investigation::insights::{
    snapshot, EvidenceLocation, EvidenceRef, PipelineInput, RetainedEvidence,
};
use argos_osint_core::storage::pipeline_exec::*;
use support::*;

fn input(kind: &str, owner: &str, generation: u32) -> PipelineInput {
    let evidence = vec![RetainedEvidence {
        reference: EvidenceRef {
            source_id: format!("body:{owner}"),
            revision: "r1".into(),
            artifact_hash: "hash".into(),
            acquisition_id: "acq".into(),
            origin_root: String::new(),
            url: "https://example.test/a".into(),
            published_at: None,
            retrieved_at: "2026-10-01".into(),
            location: EvidenceLocation::JsonPointer(String::new()),
        },
        text: "Body.".into(),
        native: None,
        substantive: true,
        relevant: true,
        complete_story: true,
        known_truncation: false,
    }];
    PipelineInput {
        snapshot: snapshot(kind, owner, generation, &evidence),
        evidence,
        objective: "Extract attributed assertions.".into(),
        surface: kind.into(),
        required_tasks_complete: true,
        save_insights: false,
        recall_enabled: false,
    }
}

#[tokio::test]
async fn checkpoints_reuse_and_fence() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "pipeline_exec").await;
    apply_schema(&repo).await.unwrap();
    let first = input("recon", "run-1", 1);

    assert_eq!(checkpoint(&repo, "k1").await.unwrap(), None);
    begin(&repo, "k1", &first, "extraction", "0", false)
        .await
        .unwrap();
    let saved = checkpoint(&repo, "k1").await.unwrap().unwrap();
    assert_eq!(saved.state, "running");
    finish(&repo, "k1", "terminal", None, None, "extraction/0: boom")
        .await
        .unwrap();
    begin(&repo, "k1", &first, "extraction", "0", false)
        .await
        .unwrap();
    assert_eq!(
        checkpoint(&repo, "k1").await.unwrap().unwrap().error,
        "extraction/0: boom",
        "a plain restart keeps the error"
    );
    begin(&repo, "k1", &first, "extraction", "0", true)
        .await
        .unwrap();
    assert_eq!(checkpoint(&repo, "k1").await.unwrap().unwrap().error, "");
    finish(
        &repo,
        "k1",
        "completed",
        Some("[1]".into()),
        Some("{}".into()),
        "",
    )
    .await
    .unwrap();
    let done = checkpoint(&repo, "k1").await.unwrap().unwrap();
    assert_eq!(
        (done.state.as_str(), done.output_json.as_str()),
        ("completed", "[1]")
    );

    // Same generation: no reuse row; a later generation copies it once.
    record_reuse(&repo, "k1", &first).await.unwrap();
    assert_eq!(checkpoint(&repo, "k1:reuse:1").await.unwrap(), None);
    let second = input("recon", "run-1", 2);
    record_reuse(&repo, "k1", &second).await.unwrap();
    record_reuse(&repo, "k1", &second).await.unwrap();
    let reused = checkpoint(&repo, "k1:reuse:2").await.unwrap().unwrap();
    assert_eq!(reused.output_json, "[1]");

    put(
        &repo,
        "m1",
        &first,
        "memory_identity",
        "mem-1",
        "exhausted",
        "false",
        "null",
        "e",
    )
    .await
    .unwrap();
    assert_eq!(
        checkpoint(&repo, "m1").await.unwrap().unwrap().state,
        "exhausted"
    );

    // No running task: empty id, epoch 0, progress and activity are no-ops.
    assert_eq!(task_id(&repo, &first).await.unwrap(), "");
    assert_eq!(operation_id(&repo, "", "k1").await.unwrap(), "k1:retry:0");
    progress(&repo, &first, "extraction", Some(3), None)
        .await
        .unwrap();
    record_activity(&repo, "", "x").await.unwrap();
    assert_eq!(activity(&repo, "nope").await.unwrap(), "");

    // Non-article owners are not fenced; an unknown article is.
    validate_dispatch(&repo, &first).await.unwrap();
    assert!(validate_dispatch(&repo, &input("article", "missing", 1))
        .await
        .is_err());
    assert!(load_memory_evidence(&repo, "no-such-memory")
        .await
        .unwrap()
        .is_none());
}
