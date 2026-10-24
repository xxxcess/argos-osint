//! Canonical Brain publication of completed intel brief kinds on SurrealDB
//! (`storage::brief_publish`, the port of `brief_canonical::publish_stage_on`).
#![cfg(feature = "surreal")]

mod support;

use argos_osint_core::storage::{atlas, brief, brief_publish};
use argos_osint_core::store::AtlasArticleRow;
use serde_json::json;
use support::*;

fn article() -> AtlasArticleRow {
    AtlasArticleRow {
        run_id: "run1".into(),
        id: "a1".into(),
        title: "Port strike widens".into(),
        description: "Dockworkers walk out across the coast.".into(),
        url: "https://example.test/a1".into(),
        country: "US".into(),
        source_name: "Example".into(),
        source_domain: "example.test".into(),
        published_at: "2026-10-22T00:00:00Z".into(),
        provider: "p".into(),
        temperature: 0.0,
        category: "labor".into(),
        seen_at: "2026-10-22T01:00:00Z".into(),
        author: String::new(),
        image_url: String::new(),
    }
}

fn claims() -> serde_json::Value {
    json!({"claims": [{
        "id": "t1", "fidelity": "faithful", "subject_id": "e1",
        "predicate": "widens", "object": "port strike",
        "text": "The port strike widens.",
        "evidence": [{"id": "t1", "start_byte": 0, "end_byte": 18}]
    }]})
}

#[tokio::test]
async fn claims_stage_publishes_a_canonical_snapshot_and_replays() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "brief_publish_claims").await;
    atlas::upsert_article(repo.db(), &article()).await.unwrap();
    let prepared = brief_publish::prepare(
        &repo,
        "a1",
        &json!({}),
        &claims(),
        &json!({}),
        "intel_brief_claims",
    )
    .await
    .unwrap();
    assert!(prepared.is_ok(), "{prepared:?}");
    let state =
        brief_publish::publish_stage(&repo, "a1", "job1", "intel_brief_claims", &prepared, "h1")
            .await
            .unwrap();
    assert_eq!(state.state, "indexed", "{state:?}");
    assert!(state.snapshot_id.starts_with("isnap:article:a1:"));
    assert!(!state.required.is_empty(), "the claim memory is required");
    let mut r = repo
        .db()
        .query("SELECT VALUE memory_kind FROM memory WHERE meta::id(id) IN $ids")
        .bind((
            "ids",
            state
                .required
                .iter()
                .map(|r| r.memory_id.clone())
                .collect::<Vec<_>>(),
        ))
        .await
        .unwrap();
    let kinds: Vec<String> = r.take(0).unwrap();
    assert!(kinds.iter().any(|k| k == "claim"), "{kinds:?}");

    // A bound indexed receipt replays without a second snapshot.
    brief::put_stage_receipt(&repo, "job1", "intel_brief_claims", &state)
        .await
        .unwrap();
    let again =
        brief_publish::publish_stage(&repo, "a1", "job1", "intel_brief_claims", &prepared, "h1")
            .await
            .unwrap();
    assert_eq!(again, state);
}

#[tokio::test]
async fn unpublishable_briefs_report_not_indexed_or_failed() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "brief_publish_gaps").await;
    // No Atlas article: inputs cannot be prepared.
    let prepared = brief_publish::prepare(
        &repo,
        "missing",
        &json!({}),
        &claims(),
        &json!({}),
        "intel_brief_claims",
    )
    .await
    .unwrap();
    let state = brief_publish::publish_stage(
        &repo,
        "missing",
        "job1",
        "intel_brief_claims",
        &prepared,
        "h1",
    )
    .await
    .unwrap();
    assert_eq!(state.state, "not_indexed");
    assert!(!state.reason.is_empty());

    // Inferences need the retained body.
    atlas::upsert_article(repo.db(), &article()).await.unwrap();
    let prepared = brief_publish::prepare(
        &repo,
        "a1",
        &json!({"markdown": "body"}),
        &claims(),
        &json!({"inferences": []}),
        "intel_brief_inferences",
    )
    .await
    .unwrap();
    assert_eq!(
        prepared.as_ref().unwrap_err(),
        "retained article body unavailable"
    );

    // Links need an indexed inference publication for the same job.
    let claims_only = brief_publish::prepare(
        &repo,
        "a1",
        &json!({}),
        &claims(),
        &json!({}),
        "intel_brief_claims",
    )
    .await
    .unwrap();
    let state =
        brief_publish::publish_stage(&repo, "a1", "job2", "intel_brief_links", &claims_only, "h2")
            .await
            .unwrap();
    assert_eq!(state.state, "failed");
    assert_eq!(state.reason, "canonical inference publication unavailable");
}
