//! Real local scoring and fenced publication for current-admission fixtures.
use std::path::Path;
use std::sync::Arc;

use argos_osint_core::atlas::{Cursor, RunStats};
use argos_osint_core::atlas_classify::{self, AdmissionContext, ComputationCache, MonotonicClock};
use argos_osint_core::classifier::readiness::{self, ClassifierHome, CycleRequirement};
use argos_osint_core::classifier::WorkControl;
use argos_osint_core::storage::classifier_admission::{
    self as admission, RunStop, StartRunRequest,
};
use argos_osint_core::storage::repo::Repository;
use argos_osint_core::store::AtlasArticleRow;

pub async fn admit(repo: &Repository, model_home: &Path, articles: &[AtlasArticleRow]) {
    publish(
        repo,
        model_home,
        articles,
        &admission::StageProvenance::default(),
        "admitted",
    )
    .await;
}

/// Publishes a real local decision, including explicit provenance rejection.
pub async fn publish(
    repo: &Repository,
    model_home: &Path,
    articles: &[AtlasArticleRow],
    provenance: &admission::StageProvenance,
    expected_state: &str,
) {
    assert!(!articles.is_empty() && articles.len() <= 64);
    let run_id = &articles[0].run_id;
    assert!(articles.iter().all(|article| &article.run_id == run_id));
    let home = ClassifierHome::new(model_home.to_path_buf());
    let owner = admission::AdmissionOwner::new(repo.clone());
    let existing = argos_osint_core::storage::atlas::list_runs(repo.db())
        .await
        .unwrap()
        .iter()
        .any(|run| run.id == *run_id);
    let selected = home.pin(None).await.unwrap();
    let requirement = if existing {
        CycleRequirement::AcquisitionAndAdmission
    } else {
        CycleRequirement::FreshAcquisition
    };
    let permit = readiness::preflight_cycle(&home, requirement, Some(&selected.snapshot))
        .await
        .unwrap();
    drop(selected);
    let pin = permit.pinned().unwrap().clone();
    let model_home = model_home.to_path_buf();
    let context = tokio::task::spawn_blocking(move || AdmissionContext::load(&model_home, pin))
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
    let revision = admission::run_revision(repo, run_id).await.unwrap();
    let request = StartRunRequest::fresh(run_id, "local-fixture", &cursor, &stats, now);
    let request = if existing {
        request.resume(revision.revision, revision.work_generation)
    } else {
        request
    };
    let run = admission::start_run(&owner, &permit, &request)
        .await
        .unwrap();
    for article in articles {
        assert!(admission::stage_article_with_provenance(
            &owner,
            &run,
            article,
            "fixture-source-v1",
            provenance,
            now,
        )
        .await
        .unwrap());
    }
    let token = admission::begin_admission(&owner, &run, &context, false, now)
        .await
        .unwrap();
    let page = admission::claim_page(&owner, &run, &token, now)
        .await
        .unwrap();
    assert_eq!(page.inputs.len(), articles.len());
    let mut cache = ComputationCache::new(Arc::new(MonotonicClock::default()));
    let scored =
        atlas_classify::classify_page(&context, page.inputs, &mut cache, &WorkControl::new())
            .await
            .unwrap();
    admission::commit_outcomes(&owner, &run, &scored.outcomes, now)
        .await
        .unwrap();
    for article in articles {
        let view = admission::classification_view(
            repo,
            &atlas_classify::OccurrenceKey::new(&article.run_id, &article.id),
        )
        .await
        .unwrap();
        assert_eq!(
            view.admission_state, expected_state,
            "{}: {view:?}",
            article.id
        );
    }
    admission::stop_run(
        &owner,
        &run,
        RunStop::Paused,
        &cursor,
        &stats,
        "Fixture published",
        now,
    )
    .await
    .unwrap();
    owner.shutdown().await.unwrap();
    drop(context);
    drop(permit);
    home.shutdown().await.unwrap();
}
