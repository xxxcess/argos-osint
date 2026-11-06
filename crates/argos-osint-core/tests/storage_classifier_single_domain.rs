//! One-domain admission persists through the same actual model/publication path.
#![cfg(all(feature = "embedded", feature = "classifier"))]
#[path = "support/actor_catalog.rs"]
mod catalog;
#[path = "fixtures/classifier/admission-publication.rs"]
mod publication;
#[allow(dead_code)] // Shared recipe exposes entry points used by other integration targets.
#[path = "fixtures/classifier/admission-recipe.rs"]
mod recipe;
mod support;
use argos_osint_core::storage::{atlas, classifier_admission};
use argos_osint_core::store::AtlasArticleRow;
use argos_osint_core::{atlas_classify::OccurrenceKey, atlas_derivative_worker as worker};
use std::sync::{atomic::AtomicBool, Arc};
#[tokio::test]
async fn supported_primary_round_trips_storage_metadata_and_independent_readiness() {
    let Some(server) = support::Server::start() else {
        eprintln!("SKIP: embedded mode not selected");
        return;
    };
    let repo = server.repo("argos", "single_domain_admission").await;
    catalog::seed_actor_catalog(&repo, &["Jane Doe"]).await;
    let installed = tokio::task::spawn_blocking(recipe::install_primary_only)
        .await
        .unwrap();
    argos_osint_core::storage::recon_threads::set_tool_enabled(
        &repo,
        argos_osint_core::osint::wikipedia_rsp::TOOL_ID,
        false,
    )
    .await
    .unwrap();
    let row = AtlasArticleRow {
        run_id: "single".into(),
        id: "single-a".into(),
        title: "Ballot patrol common".into(),
        url: "https://fixture.invalid/single".into(),
        source_domain: "fixture.invalid".into(),
        source_name: "Fixture".into(),
        provider: "fixture".into(),
        ..Default::default()
    };
    publication::admit(
        &repo,
        installed.directory.path(),
        std::slice::from_ref(&row),
    )
    .await;
    let view =
        classifier_admission::classification_view(&repo, &OccurrenceKey::new("single", "single-a"))
            .await
            .unwrap();
    assert_eq!(view.admission_state, "admitted");
    let receipt = view.receipt.unwrap();
    assert_eq!(
        receipt.receipt.selected_domains,
        vec![argos_osint_core::atlas_taxonomy::DomainId::Governance]
    );
    let encoded = serde_json::to_value(&receipt).unwrap();
    assert_eq!(
        encoded["receipt"]["selected_domains"],
        serde_json::json!(["governance"])
    );
    for _ in 0..96 {
        repo.db().query("UPDATE task SET next_eligible_at=d'2000-01-01T00:00:00Z' WHERE pool='atlas_derivatives' AND state='queued' RETURN NONE;").await.unwrap().check().unwrap();
        worker::drain_one(&repo, "single-domain", Arc::new(AtomicBool::new(false)))
            .await
            .unwrap();
        let mut failures = repo.db().query("SELECT VALUE last_error FROM atlas_derivative_job WHERE last_error!='' AND !string::ends_with(last_error,'continuation');").await.unwrap().check().unwrap();
        let failures: Vec<String> = failures.take(0).unwrap();
        assert!(
            failures.is_empty(),
            "unexpected bounded derivative blocker: {failures:?}"
        );
        let run = atlas::latest_run(repo.db()).await.unwrap().unwrap();
        let stats: argos_osint_core::atlas::RunStats =
            serde_json::from_str(&run.stats_json).unwrap();
        if stats
            .coverage
            .reconciliation_readiness
            .as_ref()
            .is_some_and(|state| state.state == "ready")
        {
            assert_eq!(stats.coverage.actor_readiness.as_ref().unwrap().eligible, 1);
            assert_eq!(
                stats.coverage.actor_readiness.as_ref().unwrap().processed,
                1
            );
            let metadata = argos_osint_core::storage::atlas_metadata::metadata(
                repo.db(),
                "single",
                "single-a",
            )
            .await
            .unwrap()
            .unwrap();
            assert_eq!(
                metadata.classification_receipt.unwrap()["receipt"]["selected_domains"],
                serde_json::json!(["governance"])
            );
            installed.home.shutdown().await.unwrap();
            return;
        }
    }
    panic!("single-domain derivatives did not reach current readiness");
}
