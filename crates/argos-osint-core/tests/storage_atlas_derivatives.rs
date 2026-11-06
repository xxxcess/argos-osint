//! Independent derivative lease/readiness regression on isolated embedded storage.
#![cfg(all(feature = "embedded", feature = "classifier"))]
#[path = "fixtures/classifier/admission-support.rs"]
mod admission_fixture;
#[allow(dead_code)]
#[path = "fixtures/classifier/admission-publication.rs"]
mod admission_publication;
#[path = "support/actor_catalog.rs"]
mod catalog;
#[path = "support/current_admission.rs"]
mod current_admission;
mod support;
use argos_osint_core::atlas_derivative_worker as worker;
use argos_osint_core::storage::{atlas, repo::Repository};
use argos_osint_core::store::AtlasArticleRow;
use std::sync::{atomic::AtomicBool, Arc};

static DERIVATIVE_TEST_GATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn scalar(repo: &Repository, sql: &str) -> i64 {
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

#[tokio::test]
async fn admitted_occurrence_hands_off_atomically_and_derivatives_ignore_parent_pause() {
    let _serial = DERIVATIVE_TEST_GATE.lock().await;
    let Some(server) = support::Server::start() else {
        eprintln!("SKIP: embedded storage mode not selected");
        return;
    };
    let repo = server.repo("argos", "independent_derivatives").await;
    catalog::seed_actor_catalog(&repo, &["Jane Doe"]).await;
    argos_osint_core::storage::recon_threads::set_tool_enabled(
        &repo,
        argos_osint_core::osint::wikipedia_rsp::TOOL_ID,
        false,
    )
    .await
    .unwrap();
    let mut row = AtlasArticleRow {
        run_id: "collection".into(),
        id: "article-a".into(),
        title: "Jane Doe addresses the assembly".into(),
        url: "https://fixture.test/a".into(),
        provider: "fixture".into(),
        source_name: "Fixture".into(),
        source_domain: "fixture.test".into(),
        published_at: "2026-11-06".into(),
        ..Default::default()
    };
    current_admission::admit(&repo, &mut row, "Ballot patrol common").await;
    assert_eq!(
        scalar(&repo, "RETURN count(SELECT id FROM atlas_derivative_job);").await,
        1,
        "admission and durable handoff must commit together"
    );
    assert_eq!(
        scalar(&repo, "RETURN count(SELECT id FROM atlas_article_index);").await,
        0
    );
    let run = atlas::latest_run(repo.db()).await.unwrap().unwrap();
    assert_eq!(run.state, "paused", "fixture released its collection owner");
    let stats: argos_osint_core::atlas::RunStats = serde_json::from_str(&run.stats_json).unwrap();
    assert_eq!(stats.coverage.actor_readiness.as_ref().unwrap().eligible, 1);
    assert_eq!(
        stats.coverage.actor_readiness.as_ref().unwrap().processed,
        0
    );
    let mut response = repo
        .db()
        .query("SELECT VALUE record::id(id) FROM job WHERE operation='atlas_derivative';")
        .await
        .unwrap()
        .check()
        .unwrap();
    let id = response.take::<Vec<String>>(0).unwrap().remove(0);
    worker::control(&repo, &id, "pause").await.unwrap();
    worker::drain_one(&repo, "derivative-test", Arc::new(AtomicBool::new(false)))
        .await
        .unwrap();
    assert_eq!(
        worker::control(&repo, &id, "status").await.unwrap()["status"]["state"],
        "paused"
    );
    assert_eq!(
        scalar(&repo, "RETURN count(SELECT id FROM atlas_article_index);").await,
        0,
        "discovery does not publish a paused task's output"
    );
    worker::control(&repo, &id, "resume").await.unwrap();
    for _ in 0..64 {
        // Deterministic scheduler clock advance in an isolated fixture. This
        // changes eligibility timing only, never source/output/lease authority.
        repo.db().query("UPDATE task SET next_eligible_at=d'2000-01-01T00:00:00Z' WHERE pool='atlas_derivatives' AND state='queued' RETURN NONE;").await.unwrap().check().unwrap();
        worker::drain_one(&repo, "derivative-test", Arc::new(AtomicBool::new(false)))
            .await
            .unwrap();
        if scalar(&repo,"RETURN count(SELECT id FROM job WHERE operation='atlas_derivative' AND state!='completed');").await==0 && scalar(&repo,"RETURN count(SELECT id FROM atlas_derivative_invalidation);").await==0 {break;}
    }
    let detail = worker::control(&repo, &id, "status").await.unwrap();
    assert_eq!(detail["status"]["state"], "completed", "{detail:#}");
    assert_eq!(
        scalar(&repo, "RETURN count(SELECT id FROM atlas_article_index);").await,
        1
    );
    let refreshed = atlas::latest_run(repo.db()).await.unwrap().unwrap();
    assert_eq!(
        refreshed.state, "paused",
        "derivatives cannot rewrite collection outcome"
    );
    let stats: argos_osint_core::atlas::RunStats =
        serde_json::from_str(&refreshed.stats_json).unwrap();
    assert_eq!(
        stats.coverage.actor_readiness.as_ref().unwrap().state,
        "ready"
    );
    assert_eq!(
        stats
            .coverage
            .reconciliation_readiness
            .as_ref()
            .unwrap()
            .state,
        "ready"
    );
    assert_eq!(
        stats.coverage.actor_readiness.as_ref().unwrap().processed,
        1
    );
    let before = scalar(&repo, "RETURN count(SELECT id FROM atlas_article_index);").await;
    assert_eq!(
        worker::drain_one(&repo, "derivative-test", Arc::new(AtomicBool::new(false)))
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        scalar(&repo, "RETURN count(SELECT id FROM atlas_article_index);").await,
        before,
        "restart must not duplicate committed output"
    );
    let completed = worker::control(&repo, &id, "cancel").await.unwrap();
    assert_eq!(
        completed["status"]["state"], "completed",
        "a late cancellation cannot rewrite a completed job"
    );
}

#[tokio::test]
async fn cancelled_derivative_stays_terminal_and_does_not_dispatch_native_outputs() {
    let _serial = DERIVATIVE_TEST_GATE.lock().await;
    let Some(server) = support::Server::start() else {
        return;
    };
    let repo = server.repo("argos", "cancelled_derivative").await;
    let mut row = AtlasArticleRow {
        run_id: "cancelled".into(),
        id: "cancelled-a".into(),
        title: "Ballot patrol common".into(),
        url: "https://fixture.invalid/cancelled".into(),
        provider: "fixture".into(),
        source_domain: "fixture.invalid".into(),
        ..Default::default()
    };
    current_admission::admit(&repo, &mut row, "Ballot patrol common").await;
    let mut response = repo
        .db()
        .query("SELECT VALUE record::id(id) FROM job WHERE operation='atlas_derivative';")
        .await
        .unwrap()
        .check()
        .unwrap();
    let id = response.take::<Vec<String>>(0).unwrap().remove(0);
    assert_eq!(
        worker::control(&repo, &id, "cancel").await.unwrap()["status"]["state"],
        "cancelled"
    );
    assert!(
        worker::control(&repo, &id, "resume").await.is_err(),
        "resume must not show a cancelled job as running while its root is revoked"
    );
    worker::drain_one(&repo, "cancel-test", Arc::new(AtomicBool::new(false)))
        .await
        .unwrap();
    assert_eq!(
        scalar(&repo, "RETURN count(SELECT id FROM atlas_article_index);").await,
        0
    );
    assert_eq!(
        worker::control(&repo, &id, "status").await.unwrap()["status"]["state"],
        "cancelled"
    );
}

#[tokio::test]
async fn catalog_changes_invalidate_ready_counts_and_requeue_completed_occurrences() {
    let _serial = DERIVATIVE_TEST_GATE.lock().await;
    let Some(server) = support::Server::start() else {
        eprintln!("SKIP: embedded storage mode not selected");
        return;
    };
    let repo = server.repo("argos", "derivative_catalog_revision").await;
    catalog::seed_actor_catalog(&repo, &["Jane Doe"]).await;
    argos_osint_core::storage::recon_threads::set_tool_enabled(
        &repo,
        argos_osint_core::osint::wikipedia_rsp::TOOL_ID,
        false,
    )
    .await
    .unwrap();
    let mut row = AtlasArticleRow {
        run_id: "late-catalog".into(),
        id: "late-a".into(),
        title: "Jane Doe addresses the assembly".into(),
        url: "https://fixture.test/late-a".into(),
        provider: "fixture".into(),
        source_name: "Fixture".into(),
        source_domain: "fixture.test".into(),
        published_at: "2026-11-06".into(),
        ..Default::default()
    };
    current_admission::admit(&repo, &mut row, "Ballot patrol common").await;
    settle(&repo).await;
    let before = scalar(&repo, "RETURN count(SELECT id FROM atlas_derivative_job);").await;
    let ready = atlas::latest_run(repo.db()).await.unwrap().unwrap();
    let stats: argos_osint_core::atlas::RunStats = serde_json::from_str(&ready.stats_json).unwrap();
    assert_eq!(stats.coverage.actor_readiness.unwrap().state, "ready");
    let graph_seed = argos_osint_core::brain_topology::TopologySeed::Article {
        run_id: row.run_id.clone(),
        article_id: row.id.clone(),
        revision: argos_osint_core::atlas_metadata::input_hash(&row),
        originating_memory: None,
    };
    let graph_options = argos_osint_core::brain_topology::TopologyOptions {
        depth: 1,
        ..Default::default()
    };
    let graph =
        argos_osint_core::storage::topology::topology_snapshot(&repo, &graph_seed, &graph_options)
            .await
            .unwrap();
    assert!(
        graph.edges.iter().any(|edge| edge.label == "mentioned_in"),
        "fixture needs a real current native actor relationship: {graph:?}"
    );
    catalog::seed_actor_catalog(&repo, &["Later Person"]).await;
    let pending = atlas::latest_run(repo.db()).await.unwrap().unwrap();
    let stats: argos_osint_core::atlas::RunStats =
        serde_json::from_str(&pending.stats_json).unwrap();
    assert_eq!(
        stats.coverage.actor_readiness.unwrap().state,
        "verification_pending",
        "late inputs cannot retain a current-generation ready claim"
    );
    let stale_graph =
        argos_osint_core::storage::topology::topology_snapshot(&repo, &graph_seed, &graph_options)
            .await
            .unwrap();
    assert!(!stale_graph.edges.iter().any(|edge|edge.label=="mentioned_in"),"an old-generation native link/mention cannot remain current while the source is pending: {stale_graph:?}");
    let mut links=repo.db().query("SELECT VALUE record::id(id) FROM memory WHERE memory_kind='link' AND payload_json CONTAINS 'atlas_article_mentions:' LIMIT 1;").await.unwrap().check().unwrap();
    let link = links.take::<Vec<String>>(0).unwrap().remove(0);
    repo.db()
        .query("UPDATE $memory SET pinned=true RETURN NONE;")
        .bind((
            "memory",
            surrealdb::types::RecordId::new("memory", link.clone()),
        ))
        .await
        .unwrap()
        .check()
        .unwrap();
    let checked_link = link.clone();
    let writable =
        argos_osint_core::storage::tx::run_tx(repo.db(), Default::default(), move |tx| {
            let id = checked_link.clone();
            async move {
                argos_osint_core::storage::lifecycle::ingest_memory_writable_in(&tx, &id).await
            }
        })
        .await
        .unwrap()
        .value;
    assert!(
        !writable,
        "explicitly pinned link must remain outside native ownership"
    );
    let curated =
        argos_osint_core::storage::topology::topology_snapshot(&repo, &graph_seed, &graph_options)
            .await
            .unwrap();
    assert!(
        curated
            .edges
            .iter()
            .any(|edge| edge.label == "mentioned_in"),
        "explicit curated ownership remains independent of native refresh; link={link}; graph={curated:?}"
    );
    settle(&repo).await;
    assert!(scalar(&repo, "RETURN count(SELECT id FROM atlas_derivative_job);").await > before);
    let after = atlas::latest_run(repo.db()).await.unwrap().unwrap();
    assert_eq!(
        after.state, ready.state,
        "derivatives do not rewrite collection completion"
    );
    let stats: argos_osint_core::atlas::RunStats = serde_json::from_str(&after.stats_json).unwrap();
    assert_eq!(stats.coverage.actor_readiness.unwrap().state, "ready");
    assert_eq!(
        scalar(&repo, "RETURN count(SELECT id FROM atlas_article_index);").await,
        1
    );
}
async fn settle(repo: &Repository) {
    for _ in 0..96 {
        repo.db().query("UPDATE task SET next_eligible_at=d'2000-01-01T00:00:00Z' WHERE pool='atlas_derivatives' AND state='queued' RETURN NONE;").await.unwrap().check().unwrap();
        let units = worker::drain_one(repo, "revision-test", Arc::new(AtomicBool::new(false)))
            .await
            .unwrap();
        assert!(units <= worker::MAX_ACTIVATION_UNITS);
        if scalar(repo,"RETURN count(SELECT id FROM atlas_derivative_invalidation);").await==0 && scalar(repo,"RETURN count(SELECT id FROM job WHERE operation='atlas_derivative' AND state!='completed');").await==0 {return;}
    }
    panic!("derivative fixture did not converge within bounded activations");
}

#[tokio::test]
async fn superseded_cancelled_publication_resumes_owed_cleanup_before_new_actor_output() {
    let _serial = DERIVATIVE_TEST_GATE.lock().await;
    let _learner = admission_fixture::serial().await;
    let Some(mut server) = support::Server::start() else {
        eprintln!("SKIP: embedded storage mode not selected");
        return;
    };
    let fixture = tokio::task::spawn_blocking(admission_fixture::install)
        .await
        .expect("fit offline admission fixture");
    let repo = server.repo("argos", "superseded_cleanup").await;
    catalog::seed_actor_catalog(&repo, &["Jane Doe", "John Roe"]).await;
    argos_osint_core::storage::recon_threads::set_tool_enabled(
        &repo,
        argos_osint_core::osint::wikipedia_rsp::TOOL_ID,
        false,
    )
    .await
    .unwrap();
    let mut article = AtlasArticleRow {
        run_id: "cleanup-source".into(),
        id: "changing".into(),
        title: "Jane Doe and John Roe address the assembly".into(),
        description: "Ballot patrol common".into(),
        url: "https://fixture.test/changing".into(),
        provider: "fixture".into(),
        source_name: "Fixture".into(),
        source_domain: "fixture.test".into(),
        published_at: "2026-11-06".into(),
        ..Default::default()
    };
    let shared = AtlasArticleRow {
        id: "shared".into(),
        title: "Jane Doe speaks at another assembly".into(),
        url: "https://fixture.test/shared".into(),
        ..article.clone()
    };
    let ordinary = argos_osint_core::storage::classifier_admission::StageProvenance::default();
    admission_publication::publish(
        &repo,
        fixture.directory.path(),
        &[article.clone(), shared.clone()],
        &ordinary,
        "admitted",
    )
    .await;
    settle(&repo).await;
    let old_link = native_link(&repo, &article, "fixture-Jane Doe").await;
    let curated_link = native_link(&repo, &article, "fixture-John Roe").await;
    let shared_link = native_link(&repo, &shared, "fixture-Jane Doe").await;
    repo.db()
        .query("UPDATE type::record('memory',$id) SET pinned=true RETURN NONE;")
        .bind(("id", curated_link.clone()))
        .await
        .unwrap()
        .check()
        .unwrap();
    let first_batch = publication_batch(&repo, &article).await;

    // Publish correction B through the actual actor processor. Its atomic
    // handoff leaves a real lifecycle cursor; no fabricated readiness/cursor
    // can stand in for the publication whose cleanup must survive.
    article.title = "The assembly deliberates on its policy".into();
    admission_publication::publish(
        &repo,
        fixture.directory.path(),
        std::slice::from_ref(&article),
        &ordinary,
        "admitted",
    )
    .await;
    for _ in 0..64 {
        dispatch(&repo).await;
        if publication_batch(&repo, &article).await != first_batch {
            break;
        }
    }
    let corrected_batch = publication_batch(&repo, &article).await;
    assert_ne!(corrected_batch, first_batch, "correction B must publish");
    let mut response = repo
        .db()
        .query(
            "SELECT VALUE checkpoint_json FROM atlas_classification_derivative
           WHERE run_id=$run AND article_id=$article AND stage='reconcile'
             AND checkpoint_json!='' AND state!='complete' LIMIT 1;",
        )
        .bind(("run", article.run_id.clone()))
        .bind(("article", article.id.clone()))
        .await
        .unwrap()
        .check()
        .unwrap();
    let checkpoints: Vec<String> = response.take(0).unwrap();
    let checkpoint: argos_osint_core::storage::lifecycle::ReconcileCursor = serde_json::from_str(
        checkpoints
            .first()
            .expect("B owes a persisted reconcile page"),
    )
    .unwrap();
    assert_ne!(frontier_phase(&repo, &checkpoint.receipt_key).await, "done");
    let mut response = repo
        .db()
        .query(
            "SELECT VALUE record::id(id) FROM atlas_derivative_job
           WHERE run_id=$run AND article_id=$article
             AND receipt_key=(SELECT VALUE receipt_key FROM ONLY $projection) LIMIT 1;",
        )
        .bind(("run", article.run_id.clone()))
        .bind(("article", article.id.clone()))
        .bind((
            "projection",
            surrealdb::types::RecordId::new(
                "atlas_classification_projection",
                argos_osint_core::storage::codec::encode(&[&article.run_id, &article.id]),
            ),
        ))
        .await
        .unwrap()
        .check()
        .unwrap();
    let old_job = response.take::<Vec<String>>(0).unwrap().remove(0);
    assert_eq!(
        worker::control(&repo, &old_job, "cancel").await.unwrap()["status"]["state"],
        "cancelled"
    );

    // A provenance rejection retains the historical source but cannot erase
    // already committed cleanup. Its replacement job owns its own live lease.
    let unsupported = argos_osint_core::storage::classifier_admission::StageProvenance::new(
        Some("fr".into()),
        Vec::new(),
    )
    .unwrap();
    admission_publication::publish(
        &repo,
        fixture.directory.path(),
        std::slice::from_ref(&article),
        &unsupported,
        "rejected",
    )
    .await;
    assert!(scalar(&repo,"RETURN count(SELECT id FROM job WHERE operation='atlas_derivative' AND state NOTINSIDE ['completed','cancelled']);").await>0,
        "rejection must retain runnable cleanup independently of the cancelled owner");
    repo.publisher().shutdown().await;
    drop(repo);
    server.restart();
    let repo = server.repo("argos", "superseded_cleanup").await;
    assert_ne!(frontier_phase(&repo, &checkpoint.receipt_key).await, "done");

    // An admitted successor C races the owed cleanup at the public scheduler
    // boundary. Its actor inventory must not publish ahead of that frontier.
    article.title = "Jane Doe returns to address the assembly".into();
    admission_publication::publish(
        &repo,
        fixture.directory.path(),
        std::slice::from_ref(&article),
        &ordinary,
        "admitted",
    )
    .await;
    let mut settled = false;
    for _ in 0..96 {
        dispatch(&repo).await;
        if publication_batch(&repo, &article).await != corrected_batch {
            assert_eq!(
                frontier_phase(&repo, &checkpoint.receipt_key).await,
                "done",
                "C cannot publish inventory while B still owes old-generation cleanup"
            );
        }
        if scalar(&repo,"RETURN count(SELECT id FROM atlas_derivative_invalidation);").await==0
            && scalar(&repo,"RETURN count(SELECT id FROM job WHERE operation='atlas_derivative' AND state NOTINSIDE ['completed','cancelled']);").await==0 {
            settled=true;
            break;
        }
    }
    assert!(
        settled,
        "replacement authority must drain cleanup and current work within bounded activations"
    );
    assert_eq!(frontier_phase(&repo, &checkpoint.receipt_key).await, "done");
    assert_ne!(publication_batch(&repo, &article).await, corrected_batch);
    assert_eq!(memory_lifecycle(&repo, &old_link).await, "retired");
    assert_eq!(
        memory_lifecycle(&repo, &curated_link).await,
        "active",
        "curated ownership survives native cleanup"
    );
    assert_eq!(
        memory_lifecycle(&repo, &shared_link).await,
        "active",
        "another retained occurrence keeps its own native evidence"
    );
    let current_link = native_link(&repo, &article, "fixture-Jane Doe").await;
    assert_eq!(
        memory_lifecycle(&repo, &current_link).await,
        "active",
        "old cleanup cannot retire the successor's replacement inventory"
    );
    assert_eq!(
        worker::control(&repo, &old_job, "status").await.unwrap()["status"]["state"],
        "cancelled",
        "replacement cleanup must not borrow or revive the cancelled lease"
    );
    assert_eq!(scalar(&repo,"RETURN count(SELECT id FROM brain_actor_mention WHERE origin_scope='cleanup-source' AND origin_id='shared' AND actor_id='fixture-Jane Doe');").await,1);

    let jobs_before = scalar(&repo, "RETURN count(SELECT id FROM atlas_derivative_job);").await;
    let rejected = AtlasArticleRow {
        id: "fresh-rejected".into(),
        ..article
    };
    admission_publication::publish(
        &repo,
        fixture.directory.path(),
        &[rejected],
        &unsupported,
        "rejected",
    )
    .await;
    assert_eq!(
        scalar(&repo, "RETURN count(SELECT id FROM atlas_derivative_job);").await,
        jobs_before,
        "a fresh rejection without prior publication owes no derivative cleanup"
    );
    repo.publisher().shutdown().await;
    fixture.home.shutdown().await.unwrap();
}

async fn dispatch(repo: &Repository) {
    // Only scheduler eligibility time changes; task/source/epoch authority is
    // still acquired and checked by the real worker in every transaction.
    repo.db().query("UPDATE task SET next_eligible_at=d'2000-01-01T00:00:00Z' WHERE pool='atlas_derivatives' AND state='queued' RETURN NONE;")
        .await.unwrap().check().unwrap();
    let units = worker::drain_one(repo, "cleanup-test", Arc::new(AtomicBool::new(false)))
        .await
        .unwrap();
    assert!(units <= worker::MAX_ACTIVATION_UNITS);
}

async fn publication_batch(repo: &Repository, article: &AtlasArticleRow) -> String {
    let mut response = repo
        .db()
        .query("SELECT VALUE publication_batch FROM ONLY type::record('atlas_article_index',$key);")
        .bind((
            "key",
            argos_osint_core::storage::codec::encode(&[&article.run_id, &article.id]),
        ))
        .await
        .unwrap()
        .check()
        .unwrap();
    response
        .take::<Option<String>>(0)
        .unwrap()
        .unwrap_or_default()
}

async fn frontier_phase(repo: &Repository, receipt: &str) -> String {
    let mut response = repo
        .db()
        .query("SELECT VALUE phase FROM ONLY type::record('lifecycle_reconcile',$receipt);")
        .bind(("receipt", receipt.to_owned()))
        .await
        .unwrap()
        .check()
        .unwrap();
    response
        .take::<Option<String>>(0)
        .unwrap()
        .expect("persisted lifecycle frontier")
}

async fn memory_lifecycle(repo: &Repository, id: &str) -> String {
    let mut response = repo
        .db()
        .query("SELECT VALUE lifecycle FROM ONLY type::record('memory',$id);")
        .bind(("id", id.to_owned()))
        .await
        .unwrap()
        .check()
        .unwrap();
    response
        .take::<Option<String>>(0)
        .unwrap()
        .expect("retained evidence memory")
}

async fn native_link(repo: &Repository, article: &AtlasArticleRow, actor: &str) -> String {
    let receipt = format!(
        "atlas_article_mentions:{}/{}:{}",
        article.run_id,
        article.id,
        argos_osint_core::atlas_metadata::input_hash(article)
    );
    let mut response=repo.db().query("SELECT VALUE record::id(id) FROM memory WHERE memory_kind='link' AND payload_json CONTAINS $receipt AND payload_json CONTAINS $actor LIMIT 2;")
        .bind(("receipt",receipt)).bind(("actor",actor.to_owned())).await.unwrap().check().unwrap();
    let mut ids: Vec<String> = response.take(0).unwrap();
    assert_eq!(
        ids.len(),
        1,
        "one real native actor relationship for {actor}"
    );
    ids.remove(0)
}
