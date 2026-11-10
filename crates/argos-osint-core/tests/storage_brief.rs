//! Intel brief publications and the retained Intel display seeds on
//! SurrealDB (`storage::brief`, `storage::topology`), against the legacy store
//! `brief_pipeline::publish` upsert and the `brain_topology_query` retained
//! seed fence. Both modes via `scripts/test_surreal.py`.
#![cfg(feature = "surreal")]

mod support;

use argos_osint_core::brain_topology::{
    CycleScope, TopologyEdgeKind, TopologyNodeKind, TopologyOptions, TopologySeed,
};
use argos_osint_core::intel_recon::brief_pipeline::BriefInputs;
use argos_osint_core::intel_recon::brief_stages::{ExecConfig, StageStore};
use argos_osint_core::storage::brief::{self, Publication, RetainedKind, Stage};
use argos_osint_core::storage::{atlas, brief_stages, topology};
use argos_osint_core::store::AtlasArticleRow;
use serde_json::json;
use support::*;

fn publication(job: &str, generation: i64, stage: Stage, claim: &str) -> Publication {
    Publication {
        article_id: "a1".into(),
        job_id: job.into(),
        generation,
        contract: "fixture".into(),
        body_hash: String::new(),
        snapshot_hash: format!("snap-{job}"),
        claims: json!({"claims": [{"id": "c1", "text": claim}]}),
        inferences: json!({"inferences": [{"id": "i1", "text": "The route could reopen"}]}),
        leads: json!([]),
        stage,
    }
}

#[tokio::test]
async fn brief_admission_commits_snapshots_and_fence_with_tasks() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "brief_admission").await;
    let db = repo.db();
    atlas::insert_run(db, "r1", "{}", "{}").await.unwrap();
    atlas::upsert_article(
        db,
        &AtlasArticleRow {
            run_id: "r1".into(),
            id: "a1".into(),
            title: "Ferry schedule".into(),
            url: "https://example.test/a1".into(),
            seen_at: "2026-10-23T00:00:00Z".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let inputs = BriefInputs {
        article_id: "a1".into(),
        input_revision: "metadata-r1".into(),
        role_snapshot: r#"{"fixture":"route-r1"}"#.into(),
        metadata: json!({"title":"Captured ferry schedule"}),
        stage_revisions: vec!["body-r1".into(), "claims-r1".into()],
        ..Default::default()
    };
    // Force a descendant snapshot write to fail after the roots and the
    // retained-seed fence have already been written in this attempt.
    db.query(
        "DEFINE FIELD OVERWRITE input_json ON brief_stage_input TYPE string ASSERT $value != ''",
    )
    .await
    .unwrap()
    .check()
    .unwrap();
    let err = brief_stages::ensure(&repo, &inputs).await.unwrap_err();
    assert!(err.to_string().contains("input_json"), "{err:#}");
    let config = ExecConfig::default();
    assert!(brief_stages::claim(&repo, "brief-worker", &config)
        .await
        .unwrap()
        .is_none());
    let mut rolled_back = db
        .query("SELECT VALUE generation FROM brief_run; SELECT VALUE generation FROM intel_brief_run; SELECT VALUE input_json FROM brief_stage_input")
        .await
        .unwrap()
        .check()
        .unwrap();
    assert!(rolled_back.take::<Vec<i64>>(0).unwrap().is_empty());
    assert!(rolled_back.take::<Vec<i64>>(1).unwrap().is_empty());
    assert!(rolled_back.take::<Vec<String>>(2).unwrap().is_empty());

    db.query("DEFINE FIELD OVERWRITE input_json ON brief_stage_input TYPE string")
        .await
        .unwrap()
        .check()
        .unwrap();
    let chain = brief_stages::ensure(&repo, &inputs).await.unwrap();
    assert!(chain.created);
    assert_eq!(
        chain.generation, 1,
        "a rejected admission spends no generation"
    );
    let (leased, body) = brief_stages::claim(&repo, "brief-worker", &config)
        .await
        .unwrap()
        .unwrap();
    let root: serde_json::Value = serde_json::from_str(&body.input_ref).unwrap();
    assert_eq!(body.job_id, chain.job_id);
    assert_eq!(root["node_revision"], "body-r1");
    assert_eq!(root["force_body"], false);
    let (context, _) = brief_stages::SurrealStages::new(&repo)
        .load(&body)
        .await
        .unwrap();
    assert_eq!(context.metadata_snapshot, inputs.metadata);
    assert_eq!(context.route_snapshot, inputs.role_snapshot);

    let reload = BriefInputs {
        input_revision: "metadata-r1:reload:explicit".into(),
        ..inputs.clone()
    };
    let next = brief_stages::ensure(&repo, &reload).await.unwrap();
    assert!(next.created);
    assert_eq!(next.generation, 2);
    assert!(!brief_stages::commit(&repo, &leased, &json!({}), 0, "")
        .await
        .unwrap());
    for request in [&inputs, &reload] {
        let replay = brief_stages::ensure(&repo, request).await.unwrap();
        assert_eq!(replay.job_id, next.job_id);
        assert!(!replay.created);
    }
    let (_, body) = brief_stages::claim(&repo, "brief-worker", &config)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(body.job_id, next.job_id);
    let root: serde_json::Value = serde_json::from_str(&body.input_ref).unwrap();
    assert_eq!(root["force_body"], true);
}

#[tokio::test]
async fn brief_completion_rolls_back_with_its_retained_run_fence() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "brief_completion").await;
    let chain = brief_stages::ensure(
        &repo,
        &BriefInputs {
            article_id: "article".into(),
            input_revision: "source-revision".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let config = ExecConfig::default();
    let output = json!({});
    let final_stage = loop {
        let (leased, _) = brief_stages::claim(&repo, "brief-worker", &config)
            .await
            .unwrap()
            .unwrap();
        if leased.operation == "intel_brief_links" {
            break leased;
        }
        assert!(brief_stages::commit(&repo, &leased, &output, 0, "")
            .await
            .unwrap());
    };
    let db = repo.db();
    db.query(
        "DEFINE FIELD OVERWRITE state ON intel_brief_run TYPE string ASSERT $value != 'completed'",
    )
    .await
    .unwrap()
    .check()
    .unwrap();
    let error = brief_stages::commit(&repo, &final_stage, &output, 1, "warning: retry item")
        .await
        .unwrap_err();
    assert!(error.to_string().contains("state"), "{error:#}");
    assert!(brief_stages::stage_meta(&repo, &final_stage.id)
        .await
        .unwrap()
        .is_none());
    let outputs = brief_stages::stage_outputs(&repo, &chain.job_id)
        .await
        .unwrap();
    assert!(!outputs.contains_key("intel_brief_links"));

    db.query("DEFINE FIELD OVERWRITE state ON intel_brief_run TYPE string")
        .await
        .unwrap()
        .check()
        .unwrap();
    assert!(
        brief_stages::commit(&repo, &final_stage, &output, 1, "warning: retry item")
            .await
            .unwrap()
    );
    assert!(
        brief_stages::commit(&repo, &final_stage, &output, 0, "different retry metadata")
            .await
            .unwrap()
    );
    assert_eq!(
        brief_stages::stage_meta(&repo, &final_stage.id)
            .await
            .unwrap(),
        Some((1, "warning: retry item".into())),
    );
    let mut state = db
        .query("SELECT VALUE state FROM ONLY type::record('intel_brief_run', $job)")
        .bind(("job", chain.job_id))
        .await
        .unwrap()
        .check()
        .unwrap();
    assert_eq!(
        state.take::<Option<String>>(0).unwrap().as_deref(),
        Some("completed")
    );
}

#[tokio::test]
async fn publications_keep_the_newest_generation_and_other_stages() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "brief_pub").await;
    assert!(brief::published_brief(&repo, "a1").await.unwrap().is_none());
    assert!(
        brief::publish(&repo, &publication("j1", 1, Stage::Claims, "First"))
            .await
            .unwrap()
    );
    // Generation 2, inferences stage: claims stay from generation 1.
    let mut p = publication("j2", 2, Stage::Inferences, "Ignored");
    p.inferences = json!({"inferences": [{"id": "i2", "text": "New"}]});
    assert!(brief::publish(&repo, &p).await.unwrap());
    let (g, claims, inferences) = brief::published_brief(&repo, "a1").await.unwrap().unwrap();
    assert_eq!(g, 2);
    assert_eq!(claims["claims"][0]["text"], "First");
    assert_eq!(inferences["inferences"][0]["id"], "i2");
    // An older generation never overwrites.
    assert!(
        !brief::publish(&repo, &publication("j0", 1, Stage::Claims, "Stale"))
            .await
            .unwrap()
    );
    let (g, claims, _) = brief::published_brief(&repo, "a1").await.unwrap().unwrap();
    assert_eq!(
        (g, claims["claims"][0]["text"].as_str()),
        (2, Some("First"))
    );
}

#[tokio::test]
async fn retained_seeds_are_fenced_by_job_run_and_item() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "brief_seed").await;
    let db = repo.db().clone();
    atlas::insert_run(&db, "r1", "{}", "{}").await.unwrap();
    atlas::upsert_article(
        &db,
        &AtlasArticleRow {
            run_id: "r1".into(),
            id: "a1".into(),
            title: "Ferry".into(),
            url: "https://ex.com/a1".into(),
            category: "unk".into(),
            seen_at: "2026-10-23T00:00:00Z".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    brief::publish(
        &repo,
        &publication("j1", 1, Stage::Claims, "Ferry suspended"),
    )
    .await
    .unwrap();
    // No run row yet: not live.
    let check = |job: &'static str, kind, id: &'static str, text: &'static str| {
        let repo = repo.clone();
        async move {
            brief::retained_item(&repo, "a1", job, kind, id, text)
                .await
                .unwrap()
        }
    };
    assert!(!check("j1", RetainedKind::Claim, "c1", "Ferry suspended").await);
    brief::record_run(&repo, "j1", "a1", 1, "completed")
        .await
        .unwrap();
    assert!(check("j1", RetainedKind::Claim, "c1", "Ferry suspended").await);
    assert!(
        check(
            "j1",
            RetainedKind::Inference,
            "i1",
            "The route could reopen"
        )
        .await
    );
    // Wrong text, wrong id, wrong job.
    assert!(!check("j1", RetainedKind::Claim, "c1", "Ferry resumed").await);
    assert!(!check("j1", RetainedKind::Claim, "c9", "Ferry suspended").await);
    assert!(!check("j9", RetainedKind::Claim, "c1", "Ferry suspended").await);

    let seed = TopologySeed::RetainedClaim {
        run_id: "r1".into(),
        article_id: "a1".into(),
        revision: String::new(),
        local_id: "c1".into(),
        text: "Ferry suspended".into(),
        job_id: "j1".into(),
    };
    let options = TopologyOptions {
        depth: 1,
        cycle_scope: CycleScope::AllRetained,
        ..Default::default()
    };
    let snap = topology::topology_snapshot(&repo, &seed, &options)
        .await
        .unwrap();
    let root = snap.nodes.iter().find(|n| n.is_root).unwrap();
    assert_eq!(root.kind, TopologyNodeKind::Claim);
    assert_eq!(root.status, "retained source · not indexed");
    assert!(snap
        .edges
        .iter()
        .any(|e| e.kind == TopologyEdgeKind::Source && e.to == root.id));

    // A newer run for the article retires the retained job.
    brief::record_run(&repo, "j2", "a1", 2, "active")
        .await
        .unwrap();
    assert!(!check("j1", RetainedKind::Claim, "c1", "Ferry suspended").await);
    let snap = topology::topology_snapshot(&repo, &seed, &options)
        .await
        .unwrap();
    let root = snap.nodes.iter().find(|n| n.is_root).unwrap();
    assert_eq!(
        root.status,
        "retained source · canonical revision unavailable"
    );

    // The retained actor seed is provisional.
    let actor = TopologySeed::RetainedActor {
        run_id: "r1".into(),
        article_id: "a1".into(),
        revision: String::new(),
        actor_ref: "x".into(),
        name: "Ferry Board".into(),
    };
    let snap = topology::topology_snapshot(&repo, &actor, &options)
        .await
        .unwrap();
    let root = snap.nodes.iter().find(|n| n.is_root).unwrap();
    assert_eq!(root.kind, TopologyNodeKind::Entity);
    assert_eq!(root.status, "retained source · provisional");
}

fn indexed(
    job: &str,
    snapshot: &str,
) -> argos_osint_core::intel_recon::brief_canonical::IndexState {
    argos_osint_core::intel_recon::brief_canonical::IndexState {
        state: "indexed".into(),
        snapshot_id: snapshot.into(),
        job_id: job.into(),
        ..Default::default()
    }
}

#[tokio::test]
async fn canonical_state_prefers_article_then_stage_receipt() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "brief_receipts").await;

    assert!(brief::retained_canonical_state(&repo, "a1", "j1", "claim")
        .await
        .unwrap()
        .is_none());
    // A stage receipt of the exact job binds while the article state does not.
    brief::put_stage_receipt(
        &repo,
        "j1",
        "intel_brief_claims",
        &indexed("j1", "s-claims"),
    )
    .await
    .unwrap();
    let got = brief::retained_canonical_state(&repo, "a1", "j1", "claim")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got.snapshot_id, "s-claims");
    assert!(
        brief::retained_canonical_state(&repo, "a1", "j1", "inference")
            .await
            .unwrap()
            .is_none()
    );
    assert!(brief::retained_canonical_state(&repo, "a1", "j2", "claim")
        .await
        .unwrap()
        .is_none());
    // The article's indexed state for the job wins.
    brief::put_index_state(&repo, "a1", &indexed("j1", "s-all"))
        .await
        .unwrap();
    let got = brief::retained_canonical_state(&repo, "a1", "j1", "inference")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got.snapshot_id, "s-all");
}

#[tokio::test]
async fn leads_are_replaced_per_article() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "brief_leads").await;
    let lead = |id: &str| brief::Lead {
        id: id.into(),
        inference_id: "i1".into(),
        inference_text: "text".into(),
        candidate_article_id: "a9".into(),
        candidate_run_id: "r9".into(),
        candidate_metadata_revision: "rev".into(),
        interpretation: "related".into(),
        verification_needed: String::new(),
        citations_json: "[]".into(),
    };
    brief::replace_leads(&repo, "a1", "j1", 1, &[lead("l1"), lead("l2")])
        .await
        .unwrap();
    brief::replace_leads(&repo, "a2", "j9", 1, &[lead("l9")])
        .await
        .unwrap();
    assert_eq!(brief::leads(&repo, "a1").await.unwrap().len(), 2);
    brief::replace_leads(&repo, "a1", "j2", 2, &[lead("l3")])
        .await
        .unwrap();
    assert_eq!(brief::leads(&repo, "a1").await.unwrap(), vec![lead("l3")]);
    assert_eq!(brief::leads(&repo, "a2").await.unwrap(), vec![lead("l9")]);
}
