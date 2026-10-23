//! Insight pipeline inputs on SurrealDB (migration phase 4c): article
//! bodies, NLP artifacts and source refs, acquisition app state, and the
//! article and Recon `PipelineInput` builders. Both modes via
//! `scripts/test_surreal.py`.
#![cfg(feature = "surreal")]

mod support;

use argos_osint_core::intel_recon::ArticleBodyRow;
use argos_osint_core::nlp::cache::hash;
use argos_osint_core::nlp::Source;
use argos_osint_core::recon::Thread;
use argos_osint_core::storage::pipeline_inputs::*;
use argos_osint_core::store::AtlasArticleRow;
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

fn body(version: i64, text: &str) -> ArticleBodyRow {
    ArticleBodyRow {
        id: format!("b{version}"),
        article_id: "a1".into(),
        run_id: "run1".into(),
        original_url: "https://example.test/a1".into(),
        resolved_url: "https://example.test/a1".into(),
        source_domain: "example.test".into(),
        source_name: "Example".into(),
        body_markdown: text.into(),
        content_hash: hash(text),
        quality: "complete".into(),
        quality_rationale: String::new(),
        body_version: version,
        fetch_tool: "fetch".into(),
        fetched_at: "2026-10-22T02:00:00Z".into(),
        retry_after: String::new(),
        state: "ready".into(),
        created_at: "2026-10-22T02:00:00Z".into(),
        updated_at: "2026-10-22T02:00:00Z".into(),
    }
}

#[tokio::test]
async fn article_input_reads_retained_body_and_metadata() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "pipeline_article").await;
    let a = article();
    // No body yet: only the metadata passage.
    let input = article_input(&repo, &a, 1, "obj").await.unwrap();
    assert_eq!(input.evidence.len(), 1);
    assert_eq!(input.evidence[0].reference.source_id, "article:a1");
    assert!(input.required_tasks_complete && input.save_insights);

    put_article_body(&repo, &body(1, "old")).await.unwrap();
    let prose = "Dockworkers walked out on Tuesday across every major port.";
    put_article_body(&repo, &body(2, prose)).await.unwrap();
    assert_eq!(
        article_body_for_article(&repo, "a1")
            .await
            .unwrap()
            .unwrap()
            .body_version,
        2
    );
    let raw = "<html>raw</html>";
    let source = Source {
        source_id: "b2".into(),
        revision: "2".into(),
        attempt_id: "att1".into(),
        raw_hash: Some(hash(raw)),
        prose_hash: hash(prose),
        prose: prose.into(),
        language_hint: None,
        published_at: None,
        retrieved_at: "2026-10-22T02:00:00Z".into(),
    };
    register(
        &repo,
        "article",
        "a1",
        &source,
        Some(raw),
        "validated-baseline-v1",
    )
    .await
    .unwrap();
    // Idempotent.
    register(
        &repo,
        "article",
        "a1",
        &source,
        Some(raw),
        "validated-baseline-v1",
    )
    .await
    .unwrap();
    assert_eq!(count(repo.db(), "nlp_source_ref").await, 1);
    // A prose hash that does not match is refused.
    let mut bad = source.clone();
    bad.prose_hash = "x".into();
    assert!(register(&repo, "article", "a1", &bad, Some(raw), "t")
        .await
        .is_err());

    let input = article_input(&repo, &a, 2, "obj").await.unwrap();
    assert_eq!(input.evidence.len(), 2);
    let b = &input.evidence[0];
    assert_eq!(b.reference.source_id, "body:b2");
    assert_eq!(b.text, prose);
    assert_eq!(
        b.reference.origin_root,
        format!("acquisition:{}", hash(raw))
    );
    // No acquisition coverage recorded: not a complete story.
    assert!(!b.complete_story && b.known_truncation);

    app_state_set(&repo, "k", "v1").await.unwrap();
    app_state_set(&repo, "k", "v2").await.unwrap();
    assert_eq!(
        app_state_get(&repo, "k").await.unwrap().as_deref(),
        Some("v2")
    );
    assert_eq!(app_state_get(&repo, "missing").await.unwrap(), None);
}

#[tokio::test]
async fn recon_input_follows_thread_opt_in_and_advances_generation() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "pipeline_recon").await;
    let off = recon_input(&repo, "r1", "t-missing", "q", Vec::new(), true)
        .await
        .unwrap();
    assert!(!off.save_insights && !off.recall_enabled);
    assert_eq!(off.snapshot.generation, 1);
    let again = recon_input(&repo, "r1", "t-missing", "q", Vec::new(), true)
        .await
        .unwrap();
    assert_eq!(again.snapshot.generation, 1);

    argos_osint_core::storage::recon::put_thread(
        &repo,
        &Thread {
            id: "t1".into(),
            title: "T".into(),
            created_at: "2026-10-23T00:00:00Z".into(),
            updated_at: "2026-10-23T00:00:00Z".into(),
            draft: String::new(),
            scroll: 0,
            recall_insights: true,
        },
    )
    .await
    .unwrap();
    let on = recon_input(&repo, "r2", "t1", "q", Vec::new(), false)
        .await
        .unwrap();
    assert!(on.save_insights && on.recall_enabled && !on.required_tasks_complete);
    assert_eq!(on.surface, "recon");
}
