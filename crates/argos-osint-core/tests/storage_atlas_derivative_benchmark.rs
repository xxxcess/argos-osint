//! Fixed offline scheduling comparison. It measures convergence, not providers.
#![cfg(all(feature = "embedded", feature = "classifier"))]
#[path = "support/actor_catalog.rs"]
mod catalog;
#[path = "fixtures/classifier/admission-support.rs"]
mod fixture;
#[path = "support/derivative_measure.rs"]
mod measure;
#[path = "fixtures/classifier/admission-publication.rs"]
mod publication;
mod support;
use argos_osint_core::storage::{atlas, repo::Repository};
use argos_osint_core::store::AtlasArticleRow;
use argos_osint_core::{atlas_actor_index, atlas_derivative_worker as worker};
use serde_json::{json, Value};
use std::sync::{atomic::AtomicBool, Arc};
use std::time::Instant;
use surrealdb::types::SurrealValue;
async fn count(repo: &Repository, sql: &str) -> i64 {
    repo.db()
        .query(sql)
        .await
        .unwrap()
        .check()
        .unwrap()
        .take::<Option<i64>>(0)
        .unwrap()
        .unwrap_or_default()
}
async fn tick(repo: &Repository) {
    repo.db().query("UPDATE task SET next_eligible_at=d'2000-01-01T00:00:00Z' WHERE pool='atlas_derivatives' AND state='queued' RETURN NONE;").await.unwrap().check().unwrap();
    let units = worker::drain_one(repo, "fixed-cohort", Arc::new(AtomicBool::new(false)))
        .await
        .unwrap();
    assert!(units <= worker::MAX_ACTIVATION_UNITS);
}
async fn settle(repo: &Repository) -> Value {
    let start = Instant::now();
    let mut first_actor = None;
    let mut actor_ready = None;
    for activation in 0..256 {
        tick(repo).await;
        if first_actor.is_none()
            && count(repo, "RETURN count(SELECT id FROM atlas_article_index);").await > 0
        {
            first_actor = Some(start.elapsed().as_millis());
        }
        let run = atlas::latest_run(repo.db()).await.unwrap().unwrap();
        let stats: argos_osint_core::atlas::RunStats =
            serde_json::from_str(&run.stats_json).unwrap();
        if actor_ready.is_none()
            && stats
                .coverage
                .actor_readiness
                .as_ref()
                .is_some_and(|state| state.state == "ready")
        {
            actor_ready = Some(start.elapsed().as_millis());
        }
        if count(repo,"RETURN count(SELECT id FROM atlas_derivative_invalidation);").await==0 && count(repo,"RETURN count(SELECT id FROM job WHERE operation='atlas_derivative' AND state!='completed');").await==0 {
            assert_eq!(stats.coverage.actor_readiness.unwrap().state,"ready");
            assert_eq!(stats.coverage.reconciliation_readiness.unwrap().state,"ready");
            return json!({"first_actor_ms":first_actor,"actor_ready_ms":actor_ready,"reconcile_ready_ms":start.elapsed().as_millis(),"activations":activation+1});
        }
    }
    panic!("fixed cohort failed to converge");
}
async fn prepare(repo: &Repository, model_home: &std::path::Path) -> (Vec<AtlasArticleRow>, u128) {
    catalog::seed_actor_catalog(repo, &["Jane Doe", "Alex Roe"]).await;
    argos_osint_core::storage::recon_threads::set_tool_enabled(
        repo,
        argos_osint_core::osint::wikipedia_rsp::TOOL_ID,
        false,
    )
    .await
    .unwrap();
    // Initialize/coalesce startup policy/catalog events before either cohort.
    // Otherwise duplicate startup jobs would make the schedules incomparable.
    for _ in 0..16 {
        tick(repo).await;
        if count(
            repo,
            "RETURN count(SELECT id FROM atlas_derivative_invalidation);",
        )
        .await
            == 0
        {
            break;
        }
    }
    assert_eq!(
        count(
            repo,
            "RETURN count(SELECT id FROM atlas_derivative_invalidation);"
        )
        .await,
        0
    );
    let start = Instant::now();
    let mut rows = Vec::new();
    for index in 0..3 {
        let row = AtlasArticleRow {
            run_id: "fixed-cohort".into(),
            id: format!("article-{index}"),
            title: if index == 2 {
                "Ballot patrol common".into()
            } else {
                "Jane Doe and Alex Roe address the assembly".into()
            },
            url: format!("https://fixture.invalid/story-{index}"),
            description: "Ballot patrol common".into(),
            provider: "fixture".into(),
            source_name: "Fixture".into(),
            source_domain: "fixture.invalid".into(),
            published_at: "2026-11-06".into(),
            ..Default::default()
        };
        publication::admit(repo, model_home, std::slice::from_ref(&row)).await;
        rows.push(row);
    }
    let collection_handoff_ms = start.elapsed().as_millis();
    assert_eq!(
        count(repo, "RETURN count(SELECT id FROM atlas_article_index);").await,
        0
    );
    prerequisites(repo, 3).await;
    assert_eq!(
        count(repo, "RETURN count(SELECT id FROM atlas_article_index);").await,
        0,
        "prerequisite phase cannot publish actors"
    );
    (rows, collection_handoff_ms)
}
async fn prerequisites(repo: &Repository, expected: i64) {
    // Stage prerequisites are produced by the actual worker, never forged.
    // Pause each job once Related is settled so both scheduling paths start
    // with the same real metadata/Related prerequisites and no actor output.
    for _ in 0..96 {
        tick(repo).await;
        let mut reply=repo.db().query("SELECT VALUE record::id(id) FROM job WHERE operation='atlas_derivative' AND phase='actors pending' AND stopped=false;").await.unwrap().check().unwrap();
        let ready: Vec<String> = reply.take(0).unwrap();
        for id in ready {
            worker::control(repo, &id, "pause").await.unwrap();
        }
        if count(repo,"RETURN count(SELECT id FROM job WHERE operation='atlas_derivative' AND state='paused');").await==expected {break;}
    }
    let mut states=repo.db().query("SELECT VALUE phase+'|'+state+'|'+error_summary FROM job WHERE operation='atlas_derivative';").await.unwrap().check().unwrap();
    let states: Vec<String> = states.take(0).unwrap();
    assert_eq!(count(repo,"RETURN count(SELECT id FROM job WHERE operation='atlas_derivative' AND state='paused');").await,expected,"{states:?}");
}
async fn resume(repo: &Repository) {
    let mut reply=repo.db().query("SELECT VALUE record::id(id) FROM job WHERE operation='atlas_derivative' AND state='paused';").await.unwrap().check().unwrap();
    for id in reply.take::<Vec<String>>(0).unwrap() {
        worker::control(repo, &id, "resume").await.unwrap();
    }
}
async fn inventory(repo: &Repository) -> Value {
    #[derive(SurrealValue)]
    struct Memory {
        memory_kind: String,
        lifecycle: String,
        payload_json: String,
    }
    let mut reply=repo.db().query("SELECT memory_kind,lifecycle,payload_json FROM memory WHERE memory_kind INSIDE ['actor','link','related']; SELECT VALUE actor_id+'|'+origin_kind+'|'+origin_scope+'|'+origin_id+'|'+origin_revision+'|'+spans_json FROM brain_actor_mention ORDER BY actor_id,origin_kind,origin_scope,origin_id; SELECT VALUE left_id+'|'+right_id+'|'+shared_json FROM atlas_related_article ORDER BY left_id,right_id;").await.unwrap().check().unwrap();
    let mut memories: Vec<_> = reply
        .take::<Vec<Memory>>(0)
        .unwrap()
        .into_iter()
        .map(|row| {
            let mut payload: Value = serde_json::from_str(&row.payload_json).unwrap();
            if let Some(revisions) = payload
                .pointer_mut("/payload/source_revisions")
                .and_then(Value::as_array_mut)
            {
                revisions.retain(|revision| {
                    !revision.as_str().is_some_and(|text| {
                        text.starts_with("ingest:")
                            || argos_osint_core::storage::codec::decode(text).is_some_and(|parts| {
                                parts
                                    .first()
                                    .is_some_and(|part| part == "native-related-v1")
                            })
                    })
                });
            }
            json!({"kind":row.memory_kind,"lifecycle":row.lifecycle,"payload":payload})
        })
        .collect();
    memories.sort_by_key(Value::to_string);
    json!({"memories":memories,"mentions":reply.take::<Vec<String>>(1).unwrap(),"related_pairs":reply.take::<Vec<String>>(2).unwrap()})
}
#[tokio::test]
#[ignore = "explicit required-embedded fixed-cohort scheduling/inventory comparison"]
async fn independent_and_synchronous_schedules_converge_after_shared_source_correction() {
    let measure = measure::Measure::default();
    tracing::subscriber::set_global_default(measure.clone()).unwrap();
    let Some(server) = support::Server::start() else {
        eprintln!("SKIP: embedded mode not selected");
        return;
    };
    let _runtime_serial = fixture::serial().await;
    let installed = tokio::task::spawn_blocking(fixture::install).await.unwrap();
    let model = installed.home.pin_atlas(None).await.unwrap();
    let model_id = model.snapshot.model_id.clone();
    let model_sha256 = model.snapshot.artifact_sha256.clone();
    let model_manifest_sha256 = model.manifest_sha256.clone();
    drop(model);
    installed.home.shutdown().await.unwrap();
    let sync = server.repo("argos", "synchronous_cohort").await;
    tracing::info!(target:"argos::atlas_derivatives",phase="synchronous_prerequisites","benchmark_marker");
    let (mut sync_rows, sync_collection) = prepare(&sync, installed.directory.path()).await;
    let cohort_sha256 = argos_osint_core::classifier::artifact::sha256_hex(
        &serde_json::to_vec(&sync_rows).unwrap(),
    );
    tracing::info!(target:"argos::atlas_derivatives",phase="synchronous_post_prerequisites","benchmark_marker");
    let start = Instant::now();
    let report = atlas_actor_index::surreal::index_run(&sync, "fixed-cohort", &|| false)
        .await
        .unwrap();
    assert!(report.complete(), "{report:?}");
    let synchronous_publish_reconcile_ms = start.elapsed().as_millis();
    assert_eq!(report.zero_actor, 1);
    resume(&sync).await;
    settle(&sync).await;
    let related = argos_osint_core::related_articles::related_articles_on(
        &argos_osint_core::atlas_store::AtlasStore::from_repository(sync.clone()),
        "fixed-cohort",
        "article-0",
        0,
        10,
    )
    .await
    .unwrap();
    assert!(
        !related.rows.is_empty(),
        "the comparison must include real current Related article pairs"
    );
    let before = inventory(&sync).await;
    let async_server = support::Server::start().unwrap();
    let asynchronous = async_server.repo("argos", "asynchronous_cohort").await;
    tracing::info!(target:"argos::atlas_derivatives",phase="asynchronous_prerequisites","benchmark_marker");
    let (mut async_rows, async_collection) =
        prepare(&asynchronous, installed.directory.path()).await;
    assert_eq!(
        cohort_sha256,
        argos_osint_core::classifier::artifact::sha256_hex(
            &serde_json::to_vec(&async_rows).unwrap()
        ),
        "same immutable input bytes for both schedules"
    );
    resume(&asynchronous).await;
    tracing::info!(target:"argos::atlas_derivatives",phase="asynchronous_post_prerequisites","benchmark_marker");
    let asynchronous_timing = settle(&asynchronous).await;
    assert_eq!(
        inventory(&asynchronous).await,
        before,
        "same initial native inventories and relationships"
    );
    for (repo, rows) in [(&sync, &mut sync_rows), (&asynchronous, &mut async_rows)] {
        tracing::info!(target:"argos::atlas_derivatives",phase=if std::ptr::eq(repo,&sync) {"synchronous_correction_prerequisites"}else{"asynchronous_correction_prerequisites"},"benchmark_marker");
        rows[0].title = "Ballot patrol common".into();
        publication::admit(
            repo,
            installed.directory.path(),
            std::slice::from_ref(&rows[0]),
        )
        .await;
        prerequisites(repo, 1).await;
    }
    // Compatibility refresh drains the actual same actor/reconcile processors.
    tracing::info!(target:"argos::atlas_derivatives",phase="synchronous_correction_publication","benchmark_marker");
    let report = atlas_actor_index::surreal::index_run(&sync, "fixed-cohort", &|| false)
        .await
        .unwrap();
    assert!(report.complete(), "{report:?}");
    resume(&sync).await;
    resume(&asynchronous).await;
    settle(&sync).await;
    tracing::info!(target:"argos::atlas_derivatives",phase="asynchronous_correction_publication","benchmark_marker");
    let correction = settle(&asynchronous).await;
    assert!(
        count(
            &sync,
            "RETURN count(SELECT id FROM memory WHERE memory_kind='link' AND lifecycle!='active');"
        )
        .await
            > 0,
        "a source correction must retire its old native mention relationships"
    );
    assert_eq!(
        count(
            &sync,
            "RETURN count(SELECT id FROM memory WHERE memory_kind='actor' AND lifecycle='active');"
        )
        .await,
        2,
        "the other source retains both shared actors"
    );
    let related = argos_osint_core::related_articles::related_articles_on(
        &argos_osint_core::atlas_store::AtlasStore::from_repository(sync.clone()),
        "fixed-cohort",
        "article-0",
        0,
        10,
    )
    .await
    .unwrap();
    assert!(
        related.rows.is_empty(),
        "the corrected source no longer shares the two mentions"
    );
    assert_eq!(
        inventory(&asynchronous).await,
        inventory(&sync).await,
        "correction, retirement and shared support must converge"
    );
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let events = measure.events.lock().unwrap().clone();
    let dropped = measure.dropped.load(std::sync::atomic::Ordering::Relaxed);
    std::fs::write(root.join(".agent-scratch/classifier-v2-derivative-cohort-metrics.json"),serde_json::to_vec_pretty(&json!({"schema_version":"atlas-derivative-cohort-metrics-v1","events":events,"dropped":dropped})).unwrap()).unwrap();
    assert_eq!(
        dropped, 0,
        "bounded telemetry must disclose incomplete capture"
    );
    println!(
        "ARGOS_DERIVATIVE_COHORT {}",
        json!({"schema_version":"atlas-derivative-cohort-v1","cohort_sha256":cohort_sha256,"model_id":model_id,"model_bundle_sha256":model_sha256,"model_manifest_sha256":model_manifest_sha256,"scorer":"offline_worker_protocol_fixture","occurrences":3,"shared_actor_occurrences":2,"zero_actor_occurrences":1,"baseline":"current synchronous compatibility processors; shared metadata/Related prerequisites","synchronous_collection_handoff_ms":sync_collection,"asynchronous_collection_handoff_ms":async_collection,"synchronous_post_prerequisites_ready_ms":synchronous_publish_reconcile_ms,"asynchronous_post_prerequisites":asynchronous_timing,"correction":correction,"note":"Scheduler eligibility clock advanced equally for deterministic tests; not a production end-to-end speedup or loaded publisher-contention measurement"})
    );
}
