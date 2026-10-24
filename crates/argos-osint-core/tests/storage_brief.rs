//! Intel brief publications and the retained Intel display seeds on
//! SurrealDB (`storage::brief`, `storage::topology`), against the SQLite
//! `brief_pipeline::publish` upsert and the `brain_topology_query` retained
//! seed fence. Both modes via `scripts/test_surreal.py`.
#![cfg(feature = "surreal")]

mod support;

use argos_osint_core::brain_topology::{
    CycleScope, TopologyEdgeKind, TopologyNodeKind, TopologyOptions, TopologySeed,
};
use argos_osint_core::storage::brief::{self, Publication, RetainedKind, Stage};
use argos_osint_core::storage::{atlas, topology};
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
