//! Indexed cross-cycle discovery uses real current admission, never legacy labels.
#![cfg(all(feature = "embedded", feature = "classifier"))]
#[path = "support/actor_catalog.rs"]
mod catalog;
#[path = "fixtures/classifier/admission-support.rs"]
mod fixture;

use argos_osint_core::atlas::{Cursor, RunStats};
use argos_osint_core::atlas_classify::{self, AdmissionContext, ComputationCache, MonotonicClock};
use argos_osint_core::atlas_derivative_worker;
use argos_osint_core::classifier::readiness::{self, CycleRequirement};
use argos_osint_core::classifier::WorkControl;
use argos_osint_core::intel_recon::brief_stages::{self, StageContext};
use argos_osint_core::storage::open::Target;
use argos_osint_core::storage::repo::Repository;
use argos_osint_core::storage::{atlas, brief_stages as stages, classifier_admission as admission};
use argos_osint_core::store::AtlasArticleRow;
use chrono::{DateTime, Utc};
use serde_json::json;
use std::sync::{atomic::AtomicBool, Arc};

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
    catalog::seed_actor_catalog(&repo, &["Jane Doe"]).await;
    argos_osint_core::storage::recon_threads::set_tool_enabled(
        &repo,
        argos_osint_core::osint::wikipedia_rsp::TOOL_ID,
        false,
    )
    .await
    .unwrap();
    // Flush startup policy/catalog discovery before creating the cohort.
    for _ in 0..8 {
        atlas_derivative_worker::drain_one(
            &repo,
            "indexed-startup",
            Arc::new(AtomicBool::new(false)),
        )
        .await
        .unwrap();
    }
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
                format!("Ballot patrol common Jane Doe district {index}")
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
    // Admission alone cannot certify native actor mentions. Build real current
    // metadata, required Related inputs and actor output through independent jobs.
    let mut current_actor = None;
    for _ in 0..768 {
        repo.db().query("UPDATE task SET next_eligible_at=d'2000-01-01T00:00:00Z' WHERE pool='atlas_derivatives' AND state='queued' RETURN NONE;").await.unwrap().check().unwrap();
        atlas_derivative_worker::drain_one(
            &repo,
            "indexed-native",
            Arc::new(AtomicBool::new(false)),
        )
        .await
        .unwrap();
        let mut response = repo.db().query("RETURN count(SELECT id FROM brain_actor_mention WHERE origin_scope=$run AND native_input_revision!='' AND publication_batch!=''); SELECT VALUE actor_id FROM brain_actor_mention WHERE origin_scope=$run AND native_input_revision!='' AND publication_batch!='' LIMIT 1; SELECT VALUE last_error FROM atlas_derivative_job WHERE run_id=$run AND last_error!='' AND !string::ends_with(last_error,'continuation') LIMIT 1;").bind(("run",run_id.to_owned())).await.unwrap().check().unwrap();
        let count = response.take::<Option<i64>>(0).unwrap().unwrap_or_default();
        let actors: Vec<String> = response.take(1).unwrap();
        let failures: Vec<String> = response.take(2).unwrap();
        assert!(failures.is_empty(), "native fixture blocker: {failures:?}");
        if count == 62 {
            current_actor = actors.into_iter().next();
            break;
        }
    }
    let current_actor = current_actor.expect("62 real current native actor publications");
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
        repo.db().query("CREATE type::record('brain_actor_mention',$id) SET actor_id=$actor,origin_kind='article',origin_scope=$run,origin_id=$article,origin_revision='metadata-v1',created_at=$now RETURN NONE")
            .bind(("id",format!("mention-{index}"))).bind(("actor",current_actor.clone())).bind(("run",row.run_id.clone())).bind(("article",row.id.clone())).bind(("now",now.to_rfc3339())).await.unwrap().check().unwrap();
    }
    let seed = AtlasArticleRow {
        id: "never-repeats".into(),
        run_id: "new-cycle".into(),
        title: "Ballot patrol common district".into(),
        url: "https://new.invalid/unique-seed".into(),
        ..Default::default()
    };
    let context = StageContext {
        metadata_snapshot: json!({"actors":[{"id":current_actor}]}),
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
    // A real later body revision passes the cheap stored publication stamp,
    // then fails the authoritative current-input audit. Keep scanning after it.
    let revised = &rows[0];
    let body = argos_osint_core::storage::article_body::ensure(
        &repo,
        &revised.id,
        &revised.run_id,
        &revised.url,
        &revised.source_domain,
        &revised.source_name,
    )
    .await
    .unwrap();
    let content = "A later retained fixture body changes the native input revision.";
    let content_hash = argos_osint_core::classifier::artifact::sha256_hex(content.as_bytes());
    argos_osint_core::storage::article_body::commit(
        &repo,
        &body.id,
        content,
        &content_hash,
        "complete",
        "Offline body revision",
        &revised.url,
        "fixture",
        true,
    )
    .await
    .unwrap();
    let retained_body = argos_osint_core::storage::article_body::by_id(&repo, &body.id)
        .await
        .unwrap()
        .unwrap();
    let mut source = argos_osint_core::nlp::Source::derived(
        &body.id,
        &retained_body.body_version.to_string(),
        content,
    );
    source.attempt_id = "indexed-body-change".into();
    source.raw_hash = Some(content_hash);
    for transform in [
        "validated-baseline-v1",
        argos_osint_core::intel_recon::clean_body::TRANSFORM,
    ] {
        argos_osint_core::storage::pipeline_inputs::register(
            &repo,
            "article",
            &revised.id,
            &source,
            Some(content),
            transform,
        )
        .await
        .unwrap();
    }
    let after_body = stages::discover_link_candidates(&repo, &context, &seed)
        .await
        .unwrap();
    assert_eq!(after_body.coverage["actor_fetched"], 50);
    assert!(
        after_body.coverage["excluded_or_missing_actor_candidates"]
            .as_u64()
            .unwrap()
            >= 1
    );
    let continuation: serde_json::Value =
        serde_json::from_str(after_body.coverage["next_cursor"].as_str().unwrap()).unwrap();
    assert_eq!(continuation["actor_done"], false);
    owner.shutdown().await.unwrap();
    fixture.home.shutdown().await.unwrap();
}
