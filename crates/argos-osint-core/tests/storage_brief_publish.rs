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
        categories: Vec::new(),
        admitted_at: String::new(),
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

fn lead(id: &str, candidate: &str) -> brief::Lead {
    brief::Lead {
        id: id.into(),
        inference_id: "i1".into(),
        inference_text: "The strike will disrupt shipping.".into(),
        candidate_article_id: candidate.into(),
        candidate_run_id: "run1".into(),
        candidate_metadata_revision: argos_osint_core::atlas_metadata::input_hash(&article()),
        interpretation: "Same strike, another port.".into(),
        verification_needed: "Confirm the second port joined.".into(),
        citations_json: "[]".into(),
    }
}

async fn memory_rows(
    repo: &argos_osint_core::storage::repo::Repository,
    kind: &str,
) -> Vec<(String, String, String)> {
    let mut r = repo
        .db()
        .query(
            "SELECT meta::id(id) AS id, lifecycle, payload_json FROM memory
               WHERE memory_kind = $k ORDER BY id",
        )
        .bind(("k", kind.to_string()))
        .await
        .unwrap();
    let rows: Vec<serde_json::Value> = r.take(0).unwrap();
    rows.into_iter()
        .map(|v| {
            (
                v["id"].as_str().unwrap_or_default().to_string(),
                v["lifecycle"].as_str().unwrap_or_default().to_string(),
                v["payload_json"].as_str().unwrap_or_default().to_string(),
            )
        })
        .collect()
}

async fn index_work(repo: &argos_osint_core::storage::repo::Repository, id: &str) -> usize {
    let mut r = repo
        .db()
        .query("RETURN count(SELECT 1 FROM index_change WHERE record_id = $id)")
        .bind(("id", id.to_string()))
        .await
        .unwrap();
    r.take::<Option<i64>>(0).unwrap().unwrap_or(0) as usize
}

#[tokio::test]
async fn inferences_then_links_publish_related_leads_and_retire_stale_ones() {
    use argos_osint_core::nlp::cache::hash;
    use argos_osint_core::nlp::Source;
    use argos_osint_core::storage::pipeline_inputs::{app_state_set, put_article_body, register};
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "brief_publish_links").await;
    let a = article();
    atlas::upsert_article(repo.db(), &a).await.unwrap();
    for candidate in ["a2", "a3"] {
        let mut c = article();
        c.id = candidate.into();
        atlas::upsert_article(repo.db(), &c).await.unwrap();
    }
    // A complete retained body acquisition for a1.
    let sentence = "The port strike widens as dockworkers walk out across the coast. ";
    let prose = sentence.repeat(12);
    let prose = prose.trim_end().to_string();
    let raw = format!("<html>{prose}</html>");
    put_article_body(
        &repo,
        &argos_osint_core::intel_recon::ArticleBodyRow {
            id: "b1".into(),
            article_id: "a1".into(),
            run_id: "run1".into(),
            original_url: a.url.clone(),
            resolved_url: a.url.clone(),
            source_domain: a.source_domain.clone(),
            source_name: a.source_name.clone(),
            body_markdown: prose.clone(),
            content_hash: hash(&prose),
            quality: "complete".into(),
            quality_rationale: String::new(),
            body_version: 1,
            fetch_tool: "fetch".into(),
            fetched_at: "2026-10-22T02:00:00Z".into(),
            retry_after: String::new(),
            state: "ready".into(),
            created_at: "2026-10-22T02:00:00Z".into(),
            updated_at: "2026-10-22T02:00:00Z".into(),
        },
    )
    .await
    .unwrap();
    let source = Source {
        source_id: "b1".into(),
        revision: "1".into(),
        attempt_id: "att1".into(),
        raw_hash: Some(hash(&raw)),
        prose_hash: hash(&prose),
        prose: prose.clone(),
        language_hint: None,
        published_at: None,
        retrieved_at: "2026-10-22T02:00:00Z".into(),
    };
    register(
        &repo,
        "article",
        "a1",
        &source,
        Some(&raw),
        "validated-baseline-v1",
    )
    .await
    .unwrap();
    app_state_set(
        &repo,
        "article-acquisition:att1",
        &json!({"artifact_hash": hash(&raw), "complete_story": true, "known_truncation": false})
            .to_string(),
    )
    .await
    .unwrap();

    register(
        &repo,
        "article",
        "a1",
        &source,
        Some(&raw),
        argos_osint_core::intel_recon::clean_body::TRANSFORM,
    )
    .await
    .unwrap();
    // Both payloads bind the current retained clean-body acquisition.
    let body = json!({ "markdown": prose, "content_hash": hash(&prose) });
    let inferences = json!({"body_hash": hash(&prose), "inferences": [{
        "id": "i1", "fidelity": "faithful",
        "text": "The strike will disrupt shipping.",
        "premise_claim_ids": ["t1"], "why": "Ports are closed.",
        "evidence": [{"id": "s1", "start_byte": 0, "end_byte": 22}]
    }]});
    for (op, hash) in [
        ("intel_brief_claims", "hc"),
        ("intel_brief_inferences", "hi"),
    ] {
        let prepared = brief_publish::prepare(&repo, "a1", &body, &claims(), &inferences, op)
            .await
            .unwrap();
        assert!(prepared.is_ok(), "{op}: {prepared:?}");
        let state = brief_publish::publish_stage(&repo, "a1", "job1", op, &prepared, hash)
            .await
            .unwrap();
        assert_eq!(state.state, "indexed", "{op}: {state:?}");
        brief::put_stage_receipt(&repo, "job1", op, &state)
            .await
            .unwrap();
        brief::put_index_state(&repo, "a1", &state).await.unwrap();
    }
    let inference_memories = memory_rows(&repo, "inference").await;
    assert_eq!(inference_memories.len(), 1, "{inference_memories:?}");
    let inference_id = inference_memories[0].0.clone();

    // Links: two leads become tentative Related memories.
    brief::replace_leads(
        &repo,
        "a1",
        "job1",
        1,
        &[lead("l1", "a2"), lead("l2", "a3")],
    )
    .await
    .unwrap();
    let first = links(&repo, &body, &inferences).await;
    assert_eq!(first.state, "indexed", "{first:?}");
    let related = memory_rows(&repo, "related").await;
    assert_eq!(related.len(), 2, "{related:?}");
    for (id, lifecycle, payload) in &related {
        assert_eq!(lifecycle, "active");
        assert!(payload.contains(&inference_id), "{payload}");
        assert!(first.required.iter().any(|r| &r.memory_id == id));
    }
    let a3 = related
        .iter()
        .find(|(_, _, p)| p.contains("\"a3\""))
        .expect("lead to a3")
        .0
        .clone();
    let before = index_work(&repo, &a3).await;
    assert!(before > 0);

    // The a3 lead no longer applies: it is retired with index work queued.
    brief::replace_leads(&repo, "a1", "job1", 1, &[lead("l1", "a2")])
        .await
        .unwrap();
    let second = links(&repo, &body, &inferences).await;
    assert_eq!(second.state, "indexed", "{second:?}");
    let related = memory_rows(&repo, "related").await;
    let lifecycle = |id: &str| related.iter().find(|r| r.0 == id).unwrap().1.clone();
    assert_eq!(lifecycle(&a3), "retired");
    assert_eq!(related.iter().filter(|r| r.1 == "active").count(), 1);
    assert!(index_work(&repo, &a3).await > before);
    assert!(!second.required.iter().any(|r| r.memory_id == a3));

    // A bound indexed Links receipt replays without touching Brain.
    brief::put_stage_receipt(&repo, "job1", "intel_brief_links", &second)
        .await
        .unwrap();
    brief::replace_leads(&repo, "a1", "job1", 1, &[])
        .await
        .unwrap();
    let again = links(&repo, &body, &inferences).await;
    assert_eq!(again, second);
    assert_eq!(
        memory_rows(&repo, "related")
            .await
            .iter()
            .filter(|r| r.1 == "active")
            .count(),
        1
    );
}

async fn links(
    repo: &argos_osint_core::storage::repo::Repository,
    body: &serde_json::Value,
    inferences: &serde_json::Value,
) -> argos_osint_core::intel_recon::brief_canonical::IndexState {
    let prepared =
        brief_publish::prepare(repo, "a1", body, &claims(), inferences, "intel_brief_links")
            .await
            .unwrap();
    brief_publish::publish_stage(repo, "a1", "job1", "intel_brief_links", &prepared, "hl")
        .await
        .unwrap()
}

#[tokio::test]
async fn completed_stage_replays_publication_after_receipt_write_failure() {
    use argos_osint_core::intel_recon::{brief_pipeline::BriefInputs, brief_stages as engine};
    use argos_osint_core::storage::brief_stages;

    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "brief_publication_recovery").await;
    let article = article();
    atlas::upsert_article(repo.db(), &article).await.unwrap();
    let chain = brief_stages::ensure(
        &repo,
        &BriefInputs {
            article_id: article.id.clone(),
            input_revision: "publication-recovery".into(),
            metadata: engine::metadata_snapshot(&article),
            stage_revisions: vec!["body".into(), "claims".into()],
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let config = engine::ExecConfig::default();
    let mut claims_lease = None;
    for _ in 0..2 {
        let (lease, stage) = brief_stages::claim(&repo, "first-worker", &config)
            .await
            .unwrap()
            .unwrap();
        if stage.operation == "intel_brief_claims" {
            claims_lease = Some(lease);
        }
    }
    let lease = claims_lease.expect("the independent Claims root was claimable");
    repo.db().query("DEFINE FIELD OVERWRITE state_json ON intel_brief_stage_receipt TYPE string ASSERT $value = ''")
        .await.unwrap().check().unwrap();
    assert!(brief_stages::commit(&repo, &lease, &claims(), 1, "")
        .await
        .unwrap());
    assert_eq!(count(repo.db(), "stage_output").await, 1);
    assert_eq!(count(repo.db(), "brief_publication_work").await, 1);
    assert_eq!(count(repo.db(), "intel_brief_publication").await, 0);
    assert!(memory_rows(&repo, "claim").await.is_empty());

    repo.db()
        .query("DEFINE FIELD OVERWRITE state_json ON intel_brief_stage_receipt TYPE string")
        .await
        .unwrap()
        .check()
        .unwrap();
    // A fresh worker connection reuses the saved stage output after the
    // failure. A repeated delivery cannot advance the canonical state again.
    let recovered = server.repo("argos", "brief_publication_recovery").await;
    assert_eq!(
        brief_stages::drain_publications(&recovered, 8)
            .await
            .unwrap(),
        1
    );
    assert_eq!(count(repo.db(), "brief_publication_work").await, 0);
    assert_eq!(count(repo.db(), "intel_brief_publication").await, 1);
    let state = brief::retained_canonical_state(&repo, "a1", &chain.job_id, "intel_brief_claims")
        .await
        .unwrap()
        .expect("canonical receipt");
    assert_eq!(state.state, "indexed", "{state:?}");
    let memories = memory_rows(&repo, "claim").await;
    assert_eq!(memories.len(), 1);
    let index_count = index_work(&repo, &memories[0].0).await;
    assert!(index_count > 0);
    assert!(brief_stages::commit(&recovered, &lease, &claims(), 1, "")
        .await
        .unwrap());
    assert_eq!(
        brief_stages::drain_publications(&recovered, 8)
            .await
            .unwrap(),
        0
    );
    assert_eq!(memory_rows(&repo, "claim").await, memories);
    assert_eq!(index_work(&repo, &memories[0].0).await, index_count);
}
