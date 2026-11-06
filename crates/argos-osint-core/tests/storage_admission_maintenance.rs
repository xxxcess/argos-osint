//! Public historical jobs preserve original material and exact model identity.
#![cfg(all(feature = "embedded", feature = "classifier"))]
#[path = "fixtures/classifier/admission-support.rs"]
mod fixture;
use argos_osint_core::atlas::{Cursor, RunStats};
use argos_osint_core::atlas_admission_maintenance::{
    self as maintenance, MaintenanceScope, MaintenanceState, ReclassificationRequest,
};
use argos_osint_core::classifier::readiness::ClassifierHome;
use argos_osint_core::storage::{
    atlas, classifier_admission as admission, open::Target, repo::Repository,
};
use argos_osint_core::store::AtlasArticleRow;
use std::sync::{atomic::AtomicBool, Arc};
use surrealdb::types::SurrealValue;

fn row(run: &str, index: usize) -> AtlasArticleRow {
    AtlasArticleRow {
        run_id: run.into(),
        id: format!("historical-{index:04}"),
        title: "Banana pastry marzipan".into(),
        description: "Saffron frosting recipes".into(),
        category: "economic".into(),
        categories: vec!["economic".into()],
        url: format!("https://fictional.invalid/{run}/{index}"),
        source_name: "Fictional retained bulletin".into(),
        source_domain: "fictional.invalid".into(),
        provider: "offline-fixture".into(),
        published_at: "2026-01-01T00:00:00Z".into(),
        seen_at: "2026-01-01T00:00:00Z".into(),
        ..Default::default()
    }
}
async fn connect(path: &std::path::Path, database: &str) -> Repository {
    Repository::connect(&Target::Embedded { path: path.into() }, "argos", database)
        .await
        .unwrap()
}
async fn seed(repo: &Repository, run: &str, count: usize) {
    atlas::insert_run(
        repo.db(),
        run,
        &serde_json::to_string(&Cursor::default()).unwrap(),
        &serde_json::to_string(&RunStats::default()).unwrap(),
    )
    .await
    .unwrap();
    for index in 0..count {
        atlas::upsert_article(repo.db(), &row(run, index))
            .await
            .unwrap();
    }
}
async fn count(repo: &Repository, table: &str) -> i64 {
    let mut response = repo
        .db()
        .query(format!("RETURN count(SELECT id FROM {table});"))
        .await
        .unwrap()
        .check()
        .unwrap();
    response.take::<Option<i64>>(0).unwrap().unwrap()
}

#[tokio::test]
async fn restarted_pool_recovers_only_its_expired_task_and_preserves_the_checkpoint() {
    use argos_osint_core::storage::jobs::{self, NewJob, NewTask};
    let _serial = fixture::serial().await;
    let installed = fixture::install();
    let path = installed.directory.path().join("expired-maintenance-db");
    let repo = connect(&path, "expired_maintenance").await;
    seed(&repo, "retained", 1).await;
    let job = maintenance::start(
        &repo,
        &installed.home,
        ReclassificationRequest::new("recover-expired", MaintenanceScope::AllRetained),
    )
    .await
    .unwrap();
    let now = chrono::Utc::now();
    jobs::create_job(
        repo.db(),
        &NewJob {
            id: "foreign-job",
            kind: "task",
            operation: "unrelated",
            parent: None,
            created_at: now,
        },
        &[NewTask {
            id: "foreign-task",
            job: "foreign-job",
            pool: "foreign-pool",
            operation: "unrelated",
            priority: 20,
            deps: vec![],
            input_hash: "foreign",
            created_at: now,
        }],
    )
    .await
    .unwrap();
    let foreign = jobs::claim_next(
        repo.db(),
        "foreign-pool",
        "other-owner",
        30,
        now,
        "foreign-claim",
    )
    .await
    .unwrap()
    .value
    .unwrap();
    let abandoned = jobs::claim_next(
        repo.db(),
        maintenance::POOL,
        "lost-owner",
        30,
        now,
        "abandoned-claim",
    )
    .await
    .unwrap()
    .value
    .unwrap();
    repo.db().query("UPDATE task SET lease_until=time::now()-1s WHERE lease_owner INSIDE ['lost-owner','other-owner'] RETURN NONE;").await.unwrap().check().unwrap();
    let before = maintenance::result(&repo, &job).await.unwrap();
    drop(repo);
    let repo = connect(&path, "expired_maintenance").await;
    let stop = Arc::new(AtomicBool::new(false));
    for _ in 0..5 {
        if maintenance::result(&repo, &job).await.unwrap().state == MaintenanceState::Completed {
            break;
        }
        assert!(
            maintenance::drain_one(&repo, &installed.home, "restarted-owner", stop.clone())
                .await
                .unwrap()
                .is_some()
        );
    }
    let after = maintenance::result(&repo, &job).await.unwrap();
    assert_eq!(after.state, MaintenanceState::Completed);
    assert_eq!(after.rows_reclassified, 1);
    assert_eq!(before.model_id, after.model_id);
    let mut response = repo.db().query("SELECT VALUE lease_epoch FROM ONLY $ours; SELECT VALUE state FROM ONLY $foreign; SELECT VALUE lease_epoch FROM ONLY $foreign; RETURN count(SELECT id FROM attempt WHERE task=$ours AND outcome='lease_expired');")
        .bind(("ours",surrealdb::types::RecordId::new("task",abandoned.id)))
        .bind(("foreign",surrealdb::types::RecordId::new("task",foreign.id)))
        .await.unwrap().check().unwrap();
    assert_eq!(response.take::<Option<i64>>(0).unwrap(), Some(3));
    assert_eq!(
        response.take::<Option<String>>(1).unwrap().as_deref(),
        Some("running")
    );
    assert_eq!(response.take::<Option<i64>>(2).unwrap(), Some(1));
    assert_eq!(response.take::<Option<i64>>(3).unwrap(), Some(1));

    let exhausted = maintenance::start(
        &repo,
        &installed.home,
        ReclassificationRequest::new("exhausted-recovery", MaintenanceScope::AllRetained),
    )
    .await
    .unwrap();
    let lost = jobs::claim_next(
        repo.db(),
        maintenance::POOL,
        "lost-again",
        30,
        chrono::Utc::now(),
        "exhausted-claim",
    )
    .await
    .unwrap()
    .value
    .unwrap();
    repo.db()
        .query("UPDATE $task SET lease_until=time::now()-1s,recoveries=$recoveries RETURN NONE;")
        .bind(("task", surrealdb::types::RecordId::new("task", lost.id)))
        .bind(("recoveries", jobs::MAX_WORKER_RECOVERIES - 1))
        .await
        .unwrap()
        .check()
        .unwrap();
    assert!(
        maintenance::drain_one(&repo, &installed.home, "last-recovery", stop)
            .await
            .unwrap()
            .is_none()
    );
    let failure = maintenance::result(&repo, &exhausted).await.unwrap();
    assert_eq!(failure.state, MaintenanceState::Failed);
    assert_eq!(failure.reason_codes, ["worker_recovery_exhausted"]);
    installed.home.shutdown().await.unwrap();
}

#[tokio::test]
async fn historical_job_reopens_with_64_row_activations_originals_and_policy_derivatives() {
    let _serial = fixture::serial().await;
    let installed = fixture::install();
    let db_path = installed.directory.path().join("maintenance-db");
    let repo = connect(&db_path, "maintenance_reopen").await;
    seed(&repo, "selected", 130).await;
    seed(&repo, "unselected", 2).await;
    let original = atlas::list_runs(repo.db()).await.unwrap();
    let request = ReclassificationRequest::new(
        "bounded-history",
        MaintenanceScope::Run {
            run_id: "selected".into(),
        },
    );
    let job = maintenance::start(&repo, &installed.home, request.clone())
        .await
        .unwrap();
    assert_eq!(
        maintenance::start(&repo, &installed.home, request)
            .await
            .unwrap(),
        job,
        "request replay is durable and does not repin"
    );
    let stop = Arc::new(AtomicBool::new(false));
    assert!(
        maintenance::drain_one(&repo, &installed.home, "maintenance-owner", stop.clone())
            .await
            .unwrap()
            .is_some()
    );
    let first = maintenance::result(&repo, &job).await.unwrap();
    assert_eq!(
        (
            first.rows_scanned,
            first.rows_reclassified,
            first.rows_rejected
        ),
        (64, 64, 64)
    );
    assert_ne!(
        first.state,
        MaintenanceState::Completed,
        "actual derivative receipts must precede completion"
    );
    let first_receipts = count(&repo, "atlas_classification_receipt").await;
    assert_eq!(first_receipts, 64);
    installed.home.shutdown().await.unwrap();
    drop(repo);
    let home = ClassifierHome::new(installed.directory.path().to_path_buf());
    let repo = connect(&db_path, "maintenance_reopen").await;
    let mut previous = first;
    for _ in 0..16 {
        assert!(
            maintenance::drain_one(&repo, &home, "reopened-owner", stop.clone())
                .await
                .unwrap()
                .is_some()
        );
        let report = maintenance::result(&repo, &job).await.unwrap();
        assert!(
            report.rows_scanned - previous.rows_scanned <= 64,
            "scan activation exceeded occurrence budget"
        );
        assert!(
            report.rows_reclassified - previous.rows_reclassified <= 64,
            "inference activation exceeded occurrence budget"
        );
        previous = report;
        if previous.state == MaintenanceState::Completed {
            break;
        }
    }
    assert_eq!(
        previous.state,
        MaintenanceState::Completed,
        "a finite retained scope must finish without live providers"
    );
    assert_eq!(
        (
            previous.rows_scanned,
            previous.rows_reclassified,
            previous.rows_rejected,
            previous.rows_skipped_unmigrated,
            previous.derivatives_pending
        ),
        (130, 130, 130, 0, 0)
    );
    assert_eq!(count(&repo, "atlas_classification_receipt").await, 130);
    assert_eq!(count(&repo, "atlas_reclassification_origin").await, 130);
    let mut telemetry=repo.db().query("SELECT payload_json FROM telemetry_event WHERE job_id=$job AND category='maintenance' ORDER BY occurred_at;").bind(("job",job.clone())).await.unwrap().check().unwrap();
    #[derive(surrealdb::types::SurrealValue)]
    struct Metric {
        payload_json: String,
    }
    let observations: Vec<Metric> = telemetry.take(0).unwrap();
    assert_eq!(
        observations.len(),
        6,
        "one idempotent event per actual classification/derivative activation"
    );
    let mut actual_scoring = 0;
    for observation in observations {
        assert!(
            !observation.payload_json.contains("Banana")
                && !observation.payload_json.contains("Saffron")
                && !observation.payload_json.contains("fictional.invalid"),
            "private source prose/URLs cannot enter numeric metrics"
        );
        let payload: serde_json::Value = serde_json::from_str(&observation.payload_json).unwrap();
        assert!(payload["atlas.rows.read"].as_u64().unwrap() <= 64);
        assert!(payload["atlas.db"]["begin"].as_u64().unwrap() > 0);
        assert!(
            payload["atlas.db"]["round_trips"].as_u64().unwrap()
                >= payload["atlas.db"]["begin"].as_u64().unwrap()
                    + payload["atlas.db"]["commit"].as_u64().unwrap()
                    + payload["atlas.db"]["rollback"].as_u64().unwrap()
        );
        assert!(payload["atlas.host.rss_bytes"].is_null());
        assert!(payload["atlas.host.rss_reason"].is_string());
        if payload["stage"] == "classification" {
            actual_scoring += 1;
            assert_eq!(
                payload["atlas.classifier.worker"]["atlas.classifier.latency_unit"],
                "batch"
            );
            assert!(
                payload["atlas.classifier.worker"]["atlas.classifier.samples"]
                    .as_u64()
                    .unwrap()
                    > 0
            );
            assert!(
                payload["atlas.classifier.page"]["computations"]
                    .as_u64()
                    .unwrap()
                    <= 64
            );
        }
    }
    assert_eq!(actual_scoring, 3);

    let mut retained=repo.db().query("SELECT original_row_json FROM atlas_reclassification_item WHERE job_id=$job AND disposition='rejected' ORDER BY article_id LIMIT 1; RETURN count(SELECT id FROM atlas_classification_derivative WHERE state='complete');").bind(("job",job.clone())).await.unwrap().check().unwrap();
    #[derive(surrealdb::types::SurrealValue)]
    struct Original {
        original_row_json: String,
    }
    let original_rows: Vec<Original> = retained.take(0).unwrap();
    let saved: AtlasArticleRow = serde_json::from_str(&original_rows[0].original_row_json).unwrap();
    assert_eq!(
        saved,
        row("selected", 0),
        "original title/description/provenance/tags survive rejection"
    );
    assert_eq!(retained.take::<Option<i64>>(1).unwrap(), Some(520));
    let current = admission::classification_view(
        &repo,
        &argos_osint_core::atlas_classify::OccurrenceKey::new("selected", "historical-0000"),
    )
    .await
    .unwrap();
    assert_eq!(current.admission_state, "rejected");
    let untouched = admission::classification_view(
        &repo,
        &argos_osint_core::atlas_classify::OccurrenceKey::new("unselected", "historical-0000"),
    )
    .await
    .unwrap();
    assert!(untouched.legacy && untouched.receipt.is_none());
    let after = atlas::list_runs(repo.db()).await.unwrap();
    assert_eq!(original.len(), after.len());
    for (before, after) in original.iter().zip(&after) {
        assert_eq!(
            (
                &before.id,
                &before.stats_json,
                &before.cursor_json,
                &before.state
            ),
            (
                &after.id,
                &after.stats_json,
                &after.cursor_json,
                &after.state
            ),
            "historical work must not rewrite original cycle state/stats/pin"
        );
    }
    assert!(
        maintenance::drain_one(&repo, &home, "reopened-owner", stop.clone())
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        maintenance::result(&repo, &job)
            .await
            .unwrap()
            .rows_reclassified,
        130
    );
    home.shutdown().await.unwrap();
}

#[tokio::test]
async fn blocked_exact_pin_requires_a_new_explicit_resume_after_restoration() {
    let _serial = fixture::serial().await;
    let installed = fixture::install();
    let repo = connect(
        &installed.directory.path().join("resume-db"),
        "maintenance_resume",
    )
    .await;
    seed(&repo, "selected", 1).await;
    let job = maintenance::start(
        &repo,
        &installed.home,
        ReclassificationRequest::new("exact-resume", MaintenanceScope::AllRetained),
    )
    .await
    .unwrap();
    maintenance::pause(&repo, &job).await.unwrap();
    assert_eq!(
        maintenance::result(&repo, &job).await.unwrap().state,
        MaintenanceState::Paused
    );
    let store =
        argos_osint_core::classifier::artifact::ArtifactStore::new(installed.home.home()).unwrap();
    let manifest = store.read_active().unwrap().unwrap();
    let saved_snapshot = installed.home.pin(None).await.unwrap().snapshot;
    let model = store.model_dir(&manifest.model_id);
    let missing = model.with_extension("held-fixture");
    std::fs::rename(&model, &missing).unwrap();
    maintenance::resume(&repo, &job).await.unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    assert!(
        maintenance::drain_one(&repo, &installed.home, "resume-owner", stop.clone())
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(
        maintenance::result(&repo, &job).await.unwrap().state,
        MaintenanceState::Blocked
    );
    maintenance::resume(&repo, &job).await.unwrap();
    assert_eq!(
        maintenance::result(&repo, &job).await.unwrap().state,
        MaintenanceState::Blocked,
        "resume reports validation pending, never queued with an unverified pin"
    );
    assert!(
        maintenance::drain_one(&repo, &installed.home, "resume-owner", stop.clone())
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(
        maintenance::result(&repo, &job).await.unwrap().state,
        MaintenanceState::Blocked
    );
    std::fs::rename(&missing, &model).unwrap();
    assert!(
        maintenance::drain_one(&repo, &installed.home, "resume-owner", stop.clone())
            .await
            .unwrap()
            .is_none(),
        "file restoration alone cannot auto-resume a blocked request"
    );
    maintenance::resume(&repo, &job).await.unwrap();
    assert!(
        maintenance::drain_one(&repo, &installed.home, "resume-owner", stop.clone())
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(
        maintenance::result(&repo, &job).await.unwrap().state,
        MaintenanceState::Queued
    );
    assert_eq!(
        count(&repo, "atlas_classification_receipt").await,
        0,
        "pin validation does not classify before the next activation"
    );
    for _ in 0..4 {
        if maintenance::result(&repo, &job).await.unwrap().state == MaintenanceState::Completed {
            break;
        }
        assert!(
            maintenance::drain_one(&repo, &installed.home, "resume-owner", stop.clone())
                .await
                .unwrap()
                .is_some()
        );
    }
    assert_eq!(
        maintenance::result(&repo, &job).await.unwrap().state,
        MaintenanceState::Completed
    );
    let mut changed = saved_snapshot;
    changed.artifact_sha256 = "f".repeat(64);
    assert!(matches!(
        maintenance::start(
            &repo,
            &installed.home,
            ReclassificationRequest::new("exact-resume", MaintenanceScope::AllRetained)
                .with_model(changed)
        )
        .await,
        Err(maintenance::MaintenanceError::RequestConflict)
    ));
    let corrupt = maintenance::start(
        &repo,
        &installed.home,
        ReclassificationRequest::new("corrupt-saved-pin", MaintenanceScope::AllRetained),
    )
    .await
    .unwrap();
    let payload = model.join(argos_osint_core::classifier::artifact::MODEL_PAYLOAD_FILE);
    let original_bytes = std::fs::read(&payload).unwrap();
    std::fs::write(&payload, b"{").unwrap();
    assert!(
        maintenance::drain_one(&repo, &installed.home, "corrupt-pin-owner", stop.clone())
            .await
            .unwrap()
            .is_some()
    );
    let blocked = maintenance::result(&repo, &corrupt).await.unwrap();
    assert_eq!(blocked.state, MaintenanceState::Blocked);
    assert_eq!(
        blocked.rows_reclassified, 0,
        "corrupt immutable pin cannot reuse a receipt as unchecked current"
    );
    assert!(blocked
        .reason_codes
        .iter()
        .any(|reason| reason == "pinned_model_missing"));
    std::fs::write(&payload, original_bytes).unwrap();
    installed.home.shutdown().await.unwrap();
}

/// The process-global repository runtime is exercised in a child test process,
/// so this pool proof cannot select another test's state root or user home.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn default_pool_waits_for_initial_model_then_accepts_one_legacy_request_without_csv() {
    const CHILD: &str = "ARGOS_TEST_ADMISSION_POOL_CHILD";
    const NAME: &str =
        "default_pool_waits_for_initial_model_then_accepts_one_legacy_request_without_csv";
    if std::env::var(CHILD).as_deref() != Ok("1") {
        let binary = std::env::current_exe().unwrap();
        let status = tokio::task::spawn_blocking(move || {
            std::process::Command::new(binary)
                .args(["--exact", NAME, "--nocapture"])
                .env(CHILD, "1")
                .status()
                .unwrap()
        })
        .await
        .unwrap();
        assert!(
            status.success(),
            "isolated actual worker-pool proof failed: {status}"
        );
        return;
    }
    let _serial = fixture::serial().await;
    let installed = fixture::install();
    let layout = argos_osint_core::storage::home::Layout::new(installed.directory.path());
    argos_osint_core::storage::home::save_config(
        &layout,
        &argos_osint_core::storage::home::StorageConfig {
            mode: argos_osint_core::storage::home::Mode::Embedded,
            port: None,
        },
    )
    .unwrap();
    argos_osint_core::storage::runtime::init(layout).unwrap();
    let repo = argos_osint_core::storage::runtime::repository()
        .await
        .unwrap();
    seed(&repo, "initial", 1).await;
    let classifier = installed.directory.path().join("classifier");
    let held = installed
        .directory
        .path()
        .join("classifier-held-by-fixture");
    std::fs::rename(&classifier, &held).unwrap();
    let pool = argos_osint_core::scheduler::WorkerPool::spawn_admission_maintenance_drainer(
        installed.directory.path().join("argos.db"),
        "actual-initial-pool".into(),
        installed.home.clone(),
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        let mut response=repo.db().query("RETURN count(SELECT id FROM argos_event WHERE event_type='drainer.error' AND string::contains(message,'admission-maintenance-startup') AND string::contains(message,'blocked_initial_classifier'));").await.unwrap().check().unwrap();
        if response.take::<Option<i64>>(0).unwrap().unwrap() > 0 {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "missing initial classifier did not produce an actual blocked startup diagnostic"
        );
        tokio::task::yield_now().await;
    }
    assert_eq!(
        count(&repo, "atlas_reclassification_job").await,
        0,
        "no pin means no accepted/fabricated default request"
    );
    let legacy = admission::classification_view(
        &repo,
        &argos_osint_core::atlas_classify::OccurrenceKey::new("initial", "historical-0000"),
    )
    .await
    .unwrap();
    assert!(legacy.legacy && legacy.receipt.is_none());
    assert_eq!(
        atlas::article(repo.db(), "initial", "historical-0000")
            .await
            .unwrap()
            .unwrap()
            .title,
        "Banana pastry marzipan"
    );
    assert!(!installed.directory.path().join("dataset").exists());
    assert!(!installed.directory.path().join("datasets").exists());
    std::fs::rename(&held, &classifier).unwrap();
    let mut job = String::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        let mut response=repo.db().query("SELECT VALUE record::id(id) FROM atlas_reclassification_job WHERE request_id='legacy-v3-initial';").await.unwrap().check().unwrap();
        let jobs: Vec<String> = response.take(0).unwrap();
        assert!(jobs.len() <= 1);
        if let Some(id) = jobs.first() {
            job = id.clone();
            let report = maintenance::result(&repo, id).await.unwrap();
            if report.state == MaintenanceState::Completed {
                assert_eq!((report.rows_reclassified, report.rows_rejected), (1, 1));
                break;
            }
        }
        assert!(
            std::time::Instant::now() < deadline,
            "compatible initial model did not progress the single default job: {job}"
        );
        tokio::task::yield_now().await;
    }
    tokio::task::spawn_blocking(move || drop(pool))
        .await
        .unwrap();
    assert_eq!(count(&repo, "atlas_reclassification_job").await, 1);
    assert_eq!(
        maintenance::start(
            &repo,
            &installed.home,
            ReclassificationRequest::new("legacy-v3-initial", MaintenanceScope::AllRetained)
        )
        .await
        .unwrap(),
        job
    );
    assert!(!installed.directory.path().join("dataset").exists());
    assert!(!installed.directory.path().join("datasets").exists());
    installed.home.shutdown().await.unwrap();
    argos_osint_core::storage::runtime::drain().await;
}

#[tokio::test]
async fn already_current_and_derivative_only_activations_need_no_new_n1_runtime() {
    let _serial = fixture::serial().await;
    let installed = fixture::install();
    let repo = connect(
        &installed.directory.path().join("no_runtime_db"),
        "maintenance_no_runtime",
    )
    .await;
    seed(&repo, "retained", 1).await;
    let stop = Arc::new(AtomicBool::new(false));
    let original = maintenance::start(
        &repo,
        &installed.home,
        ReclassificationRequest::new("first-current", MaintenanceScope::AllRetained),
    )
    .await
    .unwrap();
    for _ in 0..4 {
        if maintenance::result(&repo, &original).await.unwrap().state == MaintenanceState::Completed
        {
            break;
        }
        assert!(maintenance::drain_one(
            &repo,
            &installed.home,
            "initial-score-owner",
            stop.clone()
        )
        .await
        .unwrap()
        .is_some());
    }
    assert_eq!(
        maintenance::result(&repo, &original).await.unwrap().state,
        MaintenanceState::Completed
    );
    let next = maintenance::start(
        &repo,
        &installed.home,
        ReclassificationRequest::new("already-current", MaintenanceScope::AllRetained),
    )
    .await
    .unwrap();
    // Simulate a restarted maintenance host: release the prior inference cache
    // before occupying every runtime slot with three independent candidates.
    installed.home.shutdown().await.unwrap();
    let readonly_home = ClassifierHome::new(installed.directory.path().to_path_buf());
    // Three actual candidate runtimes exhaust the same process ledger. The
    // target inference service is no longer retained by the completed job.
    let mut held = Vec::new();
    for index in 0..3 {
        let root = tempfile::tempdir().unwrap();
        let store =
            argos_osint_core::classifier::artifact::ArtifactStore::new(root.path()).unwrap();
        let lease = Arc::new(store.try_writer().unwrap());
        let model =
            argos_osint_core::classifier::ModelId::parse(&format!("held-n1-{index}")).unwrap();
        let worker = argos_osint_core::classifier::start_candidate_worker(
            &model,
            lease.clone(),
            &argos_osint_core::classifier::WorkControl::new(),
        )
        .unwrap();
        held.push((worker, lease, root));
    }
    let extra_root = tempfile::tempdir().unwrap();
    let extra_store =
        argos_osint_core::classifier::artifact::ArtifactStore::new(extra_root.path()).unwrap();
    let extra_lease = Arc::new(extra_store.try_writer().unwrap());
    let extra_id = argos_osint_core::classifier::ModelId::parse("must-not-fit-n1").unwrap();
    assert!(
        matches!(
            argos_osint_core::classifier::start_candidate_worker(
                &extra_id,
                extra_lease,
                &argos_osint_core::classifier::WorkControl::new()
            ),
            Err(argos_osint_core::classifier::ClassifierError::RuntimeBudgetExceeded { .. })
        ),
        "the real N1 ledger must be full for this proof"
    );
    for _ in 0..3 {
        if maintenance::result(&repo, &next).await.unwrap().state == MaintenanceState::Completed {
            break;
        }
        assert!(maintenance::drain_one(
            &repo,
            &readonly_home,
            "already-current-owner",
            stop.clone()
        )
        .await
        .unwrap()
        .is_some());
    }
    let report = maintenance::result(&repo, &next).await.unwrap();
    assert_eq!(
        report.state,
        MaintenanceState::Completed,
        "matching receipt/derivative audit cannot require a fourth runtime"
    );
    assert_eq!(
        (
            report.rows_scanned,
            report.rows_already_current,
            report.rows_reclassified,
            report.derivatives_pending
        ),
        (1, 1, 0, 0)
    );
    assert_eq!(count(&repo, "atlas_classification_receipt").await, 1);
    drop(held);
    installed.home.shutdown().await.unwrap();
}
