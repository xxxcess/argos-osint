//! Indexed cross-cycle discovery uses real current admission, never legacy labels.
#![cfg(all(feature = "embedded", feature = "classifier"))]
#[path = "fixtures/classifier/admission-support.rs"]
mod fixture;

use argos_osint_core::atlas::{Cursor, RunStats};
use argos_osint_core::atlas_classify::{self, AdmissionContext, ComputationCache, MonotonicClock};
use argos_osint_core::classifier::readiness::{self, CycleRequirement};
use argos_osint_core::classifier::WorkControl;
use argos_osint_core::intel_recon::brief_stages::{self, StageContext};
use argos_osint_core::storage::open::Target;
use argos_osint_core::storage::repo::Repository;
use argos_osint_core::storage::{atlas, brief_stages as stages, classifier_admission as admission};
use argos_osint_core::store::AtlasArticleRow;
use chrono::{DateTime, Utc};
use serde_json::json;
use std::sync::Arc;

#[tokio::test]
async fn current_cross_cycle_indexes_bound_raw_windows_and_preserve_unicode_occurrences() {
    let _serial = fixture::serial().await;
    let fixture = fixture::install();
    let repo = Repository::connect(
        &Target::Embedded {
            path: fixture.directory.path().join("storage"),
        },
        "argos",
        "indexed_links",
    )
    .await
    .unwrap();
    let owner = admission::AdmissionOwner::new(repo.clone());
    let permit =
        readiness::preflight_cycle(&fixture.home, CycleRequirement::FreshAcquisition, None)
            .await
            .unwrap();
    let pin = permit.pinned().unwrap().clone();
    let path = fixture.home.home().to_path_buf();
    let context = tokio::task::spawn_blocking(move || AdmissionContext::load(&path, pin))
        .await
        .unwrap()
        .unwrap();
    let stats = RunStats {
        classifier_snapshot: Some(context.pin.snapshot.clone()),
        ..Default::default()
    };
    let cursor = serde_json::to_string(&Cursor::default()).unwrap();
    let now: DateTime<Utc> = "2026-11-01T00:00:00Z".parse().unwrap();
    let run_id = "a-当前|retained";
    let stats_json = serde_json::to_string(&stats).unwrap();
    let run = admission::start_run(
        &owner,
        &permit,
        &admission::StartRunRequest::fresh(run_id, "indexed-owner", &cursor, &stats_json, now),
    )
    .await
    .unwrap();
    let mut rows = Vec::new();
    for index in 0..64 {
        let row = AtlasArticleRow {
            run_id: run_id.into(),
            id: format!("报告|{index:03}"),
            title: if index < 62 {
                format!("Ballot patrol common district {index}")
            } else {
                "Banana pastry marzipan".into()
            },
            description: if index < 62 {
                format!("Fictional independent training example district {index}.")
            } else {
                "Saffron frosting recipes".into()
            },
            url: format!("https://archive.invalid/distinct/{index}"),
            provider: "fixture".into(),
            source_name: "Fictional archive".into(),
            source_domain: "archive.invalid".into(),
            published_at: "2026-10-30T00:00:00Z".into(),
            seen_at: now.to_rfc3339(),
            ..Default::default()
        };
        if index >= 62 {
            atlas::upsert_article(repo.db(), &row).await.unwrap();
        }
        assert!(
            admission::stage_article(&owner, &run, &row, "source-v1", now)
                .await
                .unwrap()
        );
        rows.push(row);
    }
    let token = admission::begin_admission(&owner, &run, &context, false, now)
        .await
        .unwrap();
    let page = admission::claim_page(&owner, &run, &token, now)
        .await
        .unwrap();
    assert_eq!(page.inputs.len(), 64);
    let mut cache = ComputationCache::new(Arc::new(MonotonicClock::default()));
    let scored =
        atlas_classify::classify_page(&context, page.inputs, &mut cache, &WorkControl::new())
            .await
            .unwrap();
    admission::commit_outcomes(&owner, &run, &scored.outcomes, now)
        .await
        .unwrap();
    for row in &rows {
        let view = admission::classification_view(
            &repo,
            &atlas_classify::OccurrenceKey::new(&row.run_id, &row.id),
        )
        .await
        .unwrap();
        assert_eq!(
            view.admission_state,
            if row.title.starts_with("Ballot") {
                "admitted"
            } else {
                "rejected"
            }
        );
    }
    // More historical mentions than the old raw limit sort before every current
    // occurrence. None can displace a current hit or masquerade as a new domain.
    for index in 0..60 {
        let row = AtlasArticleRow {
            run_id: "z-legacy".into(),
            id: format!("legacy-{index:03}"),
            title: format!("Ballot patrol common old district {index}"),
            description: "Retained historical category only".into(),
            category: "economic".into(),
            categories: vec!["economic".into()],
            ..Default::default()
        };
        atlas::upsert_article(repo.db(), &row).await.unwrap();
        rows.push(row);
    }
    for (index, row) in rows.iter().enumerate() {
        repo.db().query("CREATE type::record('brain_actor_mention',$id) SET actor_id='fixture-actor',origin_kind='article',origin_scope=$run,origin_id=$article,origin_revision='metadata-v1',created_at=$now RETURN NONE")
            .bind(("id",format!("mention-{index}"))).bind(("run",row.run_id.clone())).bind(("article",row.id.clone())).bind(("now",now.to_rfc3339())).await.unwrap().check().unwrap();
    }
    let seed = AtlasArticleRow {
        id: "never-repeats".into(),
        run_id: "new-cycle".into(),
        title: "Ballot patrol common district".into(),
        url: "https://new.invalid/unique-seed".into(),
        ..Default::default()
    };
    let context = StageContext {
        metadata_snapshot: json!({"actors":[{"id":"fixture-actor"}]}),
        ..Default::default()
    };
    let first = stages::discover_link_candidates(&repo, &context, &seed)
        .await
        .unwrap();
    assert_eq!(first.coverage["actor_fetched"], 50);
    assert!(
        first.coverage["actor_fetched"].as_u64().unwrap()
            + first.coverage["text_fetched"].as_u64().unwrap()
            <= 100
    );
    assert!(first
        .rows
        .iter()
        .all(|row| row.run_id == run_id && row.title.starts_with("Ballot")));
    assert!(!first.rows.is_empty());
    let candidates = brief_stages::shortlist(&seed, first.rows.clone());
    assert_eq!(candidates.len(), 12);
    assert!(candidates
        .iter()
        .all(|candidate| candidate.article.url != seed.url));
    assert!(first.coverage["truncated"].as_bool().unwrap());
    let next = first.coverage["next_cursor"]
        .as_str()
        .expect("explicit bounded continuation");
    let second = stages::discover_link_candidates_after(&repo, &context, &seed, Some(next))
        .await
        .unwrap();
    assert!(second.coverage["actor_fetched"].as_u64().unwrap() <= 50);
    assert!(
        second.coverage["actor_fetched"].as_u64().unwrap()
            + second.coverage["text_fetched"].as_u64().unwrap()
            <= 100
    );
    let mut discovered = std::collections::BTreeSet::new();
    for row in first.rows.iter().chain(&second.rows) {
        discovered.insert((row.run_id.clone(), row.id.clone()));
    }
    assert_eq!(
        discovered.len(),
        62,
        "continuation reaches every current occurrence behind historical matches"
    );
    let mut changed = seed.clone();
    changed.title.push_str(" changed source");
    assert!(
        stages::discover_link_candidates_after(&repo, &context, &changed, Some(next))
            .await
            .is_err()
    );
    owner.shutdown().await.unwrap();
    fixture.home.shutdown().await.unwrap();
}
