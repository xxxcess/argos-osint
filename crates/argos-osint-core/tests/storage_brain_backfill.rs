//! The Brain backfill pool on SurrealDB (`storage::brain_backfill`): Atlas
//! occurrences without a current index receipt are indexed in bounded
//! passes, and a rerun finds nothing left.
#![cfg(feature = "surreal")]

#[path = "support/actor_catalog.rs"]
mod actor_catalog_fixture;
#[cfg(feature = "classifier")]
#[path = "fixtures/classifier/admission-support.rs"]
mod classifier_fixture;
mod support;

use argos_osint_core::atlas_metadata::{ArticleMention, ArticleMetadata, MetadataState};
use argos_osint_core::storage::{bootstrap, brain_backfill, lifecycle, repo::Repository};
use argos_osint_core::store::AtlasArticleRow;
use support::*;

async fn seed(repo: &Repository, run: &str, id: &str, title: &str, names: &[&str]) {
    let a = AtlasArticleRow {
        run_id: run.into(),
        id: id.into(),
        title: title.into(),
        url: format!("https://x.test/{id}"),
        source_name: "X".into(),
        source_domain: "x.test".into(),
        published_at: "2026-10-12".into(),
        provider: "fixture".into(),
        category: "unk".into(),
        ..Default::default()
    };
    argos_osint_core::storage::atlas::upsert_article(repo.db(), &a)
        .await
        .unwrap();
    let mut m = ArticleMetadata::pending(&a);
    m.entity_state = MetadataState::Complete;
    for n in names {
        let start = title.find(n).unwrap();
        m.mentions.push(ArticleMention {
            type_hint: "person".into(),
            comparison_key: n.to_lowercase(),
            field: "title".into(),
            start_byte: start,
            end_byte: start + n.len(),
            original_text: (*n).into(),
        });
    }
    argos_osint_core::storage::atlas_metadata::save_metadata(repo.db(), &m)
        .await
        .unwrap();
}

#[cfg(feature = "classifier")]
#[tokio::test]
async fn backfill_indexes_unreceipted_occurrences_within_the_budget() {
    let Some(server) = Server::start() else {
        return;
    };
    let _serial = classifier_fixture::serial().await;
    let model = classifier_fixture::install();
    let config_path = model.directory.path().join("classifier/zero-shot.json");
    let config_before = std::fs::read(&config_path).unwrap();
    let repo = server.repo("argos", "brain_backfill").await;
    actor_catalog_fixture::seed_actor_catalog(&repo, &["Jane Doe"]).await;
    bootstrap::ensure_schema(repo.db()).await.unwrap();
    lifecycle::apply_article_index_schema(repo.db())
        .await
        .unwrap();
    for i in 0..3 {
        seed(
            &repo,
            "r",
            &format!("a{i}"),
            "Ballot patrol common Jane Doe speaks",
            &["Jane Doe"],
        )
        .await;
    }
    seed(
        &repo,
        "r",
        "rejected",
        "Banana pastry marzipan Jane Doe",
        &["Jane Doe"],
    )
    .await;
    seed(
        &repo,
        "r",
        "legacy",
        "Ballot patrol common Jane Doe remains legacy",
        &["Jane Doe"],
    )
    .await;
    admit(&repo, &model.home, "r", &["a0", "a1", "a2", "rejected"]).await;
    let rejected = argos_osint_core::storage::classifier_admission::classification_view(
        &repo,
        &argos_osint_core::atlas_classify::OccurrenceKey::new("r", "rejected"),
    )
    .await
    .unwrap();
    assert_eq!(rejected.admission_state, "rejected");
    let first = brain_backfill::backfill(&repo, 2).await.unwrap();
    let mut diagnostics = repo
        .db()
        .query(
            "SELECT stage,state,last_error,disposition_json FROM atlas_classification_derivative",
        )
        .await
        .unwrap()
        .check()
        .unwrap();
    let diagnostics: Vec<serde_json::Value> = diagnostics.take(0).unwrap();
    assert_eq!(
        (first.processed, first.failed),
        (2, 0),
        "{first:?}; native stage diagnostics: {diagnostics:?}"
    );
    assert!(!first.complete);
    let second = brain_backfill::backfill(&repo, 100).await.unwrap();
    assert_eq!(second.processed, 1, "{second:?}");
    assert!(second.complete);
    assert_eq!(second.coverage[0].done, 3);
    let mut r = repo
        .db()
        .query("RETURN count(SELECT 1 FROM brain_actor_mention)")
        .await
        .unwrap()
        .check()
        .unwrap();
    let mentions: Option<i64> = r.take(0).unwrap();
    assert_eq!(mentions, Some(3));
    let third = brain_backfill::backfill(&repo, 100).await.unwrap();
    assert_eq!(third.processed, 0);
    assert!(third.complete);
    let legacy = argos_osint_core::storage::atlas::article(repo.db(), "r", "legacy")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(legacy.title, "Ballot patrol common Jane Doe remains legacy");
    assert!(
        argos_osint_core::storage::atlas::article(repo.db(), "r", "rejected")
            .await
            .unwrap()
            .is_some(),
        "historical rejected row remains readable"
    );
    assert_eq!(
        std::fs::read(&config_path).unwrap(),
        config_before,
        "backfill cannot change the active classifier"
    );
    model.home.shutdown().await.unwrap();
}

#[cfg(feature = "classifier")]
async fn admit(
    repo: &Repository,
    home: &argos_osint_core::classifier::readiness::ClassifierHome,
    run_id: &str,
    ids: &[&str],
) {
    use argos_osint_core::atlas::{Cursor, RunStats};
    use argos_osint_core::atlas_classify::{self, AdmissionContext, ComputationCache};
    use argos_osint_core::classifier::readiness::{self, CycleRequirement};
    use argos_osint_core::storage::classifier_admission as admission;
    let owner = admission::AdmissionOwner::new(repo.clone());
    let permit = readiness::preflight_cycle(home, CycleRequirement::FreshAcquisition, None)
        .await
        .unwrap();
    let pin = permit.pinned().unwrap().clone();
    let root = home.home().to_path_buf();
    let context = tokio::task::spawn_blocking(move || AdmissionContext::load(&root, pin))
        .await
        .unwrap()
        .unwrap();
    let stats = RunStats {
        classifier_snapshot: Some(context.pin.snapshot.clone()),
        ..Default::default()
    };
    let cursor = serde_json::to_string(&Cursor::default()).unwrap();
    let stats = serde_json::to_string(&stats).unwrap();
    let now = chrono::Utc::now();
    let run = admission::start_run(
        &owner,
        &permit,
        &admission::StartRunRequest::fresh(run_id, "backfill-fixture-owner", &cursor, &stats, now),
    )
    .await
    .unwrap();
    for id in ids {
        let row = argos_osint_core::storage::atlas::article(repo.db(), run_id, id)
            .await
            .unwrap()
            .unwrap();
        assert!(
            admission::stage_article(&owner, &run, &row, "backfill-source-v1", now)
                .await
                .unwrap()
        );
    }
    let token = admission::begin_admission(&owner, &run, &context, false, now)
        .await
        .unwrap();
    let page = admission::claim_page(&owner, &run, &token, now)
        .await
        .unwrap();
    assert_eq!(page.inputs.len(), ids.len());
    let scored = atlas_classify::classify_page(
        &context,
        page.inputs,
        &mut ComputationCache::default(),
        &argos_osint_core::classifier::WorkControl::new(),
    )
    .await
    .unwrap();
    let committed = admission::commit_outcomes(&owner, &run, &scored.outcomes, now)
        .await
        .unwrap();
    assert!(committed
        .items
        .iter()
        .all(|item| matches!(item, admission::ItemCommit::Committed { .. })));
    admission::stop_run(
        &owner,
        &run,
        admission::RunStop::Paused,
        &cursor,
        &stats,
        "fixture-paused-after-real-admission",
        chrono::Utc::now(),
    )
    .await
    .unwrap();
    owner.shutdown().await.unwrap();
}

#[cfg(not(feature = "classifier"))]
#[tokio::test]
async fn legacy_backfill_cannot_invent_current_admission_when_classifier_is_disabled() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "legacy_backfill_feature_off").await;
    bootstrap::ensure_schema(repo.db()).await.unwrap();
    seed(
        &repo,
        "retained",
        "legacy",
        "Jane Doe remains legacy",
        &["Jane Doe"],
    )
    .await;
    let report = brain_backfill::backfill(&repo, 64).await.unwrap();
    assert_eq!((report.processed, report.failed), (0, 0));
    assert_eq!((report.coverage[0].done, report.coverage[0].total), (0, 0));
    assert!(report.complete);
    let row = argos_osint_core::storage::atlas::article(repo.db(), "retained", "legacy")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.title, "Jane Doe remains legacy");
    let mut response = repo
        .db()
        .query("RETURN count(SELECT id FROM brain_actor_mention);")
        .await
        .unwrap()
        .check()
        .unwrap();
    assert_eq!(response.take::<Option<i64>>(0).unwrap(), Some(0));
}
