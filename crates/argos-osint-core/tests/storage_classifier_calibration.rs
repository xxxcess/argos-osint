//! Post-cycle jobs and provider provenance on isolated embedded storage.
#![cfg(all(feature = "embedded", feature = "classifier"))]

#[path = "fixtures/classifier/admission-support.rs"]
mod fixture;

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use argos_osint_core::atlas::{Cursor, RunStats};
use argos_osint_core::classifier::provider_capture::{self as capture, CaptureContext};
use argos_osint_core::classifier::provider_labels::{self, ProviderFormat};
use argos_osint_core::classifier::readiness::{self, ClassifierHome, CycleRequirement};
use argos_osint_core::storage::classifier_admission::{
    self as admission, RunStop, StartRunRequest,
};
use argos_osint_core::storage::classifier_calibration::{self as calibration, CalibrationReport};
use argos_osint_core::storage::open::Target;
use argos_osint_core::storage::repo::Repository;

fn context(cycle: &str) -> CaptureContext {
    CaptureContext::new(
        cycle,
        "2026-11-05T12:00:00Z",
        format!("fetch:{cycle}"),
        "1".repeat(64),
    )
}

async fn count(repo: &Repository, table: &str) -> i64 {
    let mut response = repo
        .db()
        .query(format!("RETURN count(SELECT id FROM {table})"))
        .await
        .unwrap()
        .check()
        .unwrap();
    response.take::<Option<i64>>(0).unwrap().unwrap()
}

async fn finish(
    repo: &Repository,
    home: &ClassifierHome,
    cycle: &str,
    fresh: bool,
) -> admission::TerminalReceipt {
    let owner = admission::AdmissionOwner::new(repo.clone());
    let requirement = if fresh {
        CycleRequirement::FreshAcquisition
    } else {
        CycleRequirement::PostAdmissionOnly
    };
    let permit = readiness::preflight_cycle(home, requirement, None)
        .await
        .unwrap();
    let stats = RunStats {
        classifier_snapshot: permit.pinned().map(|pin| pin.snapshot.clone()),
        ..Default::default()
    };
    let stats = serde_json::to_string(&stats).unwrap();
    let cursor = serde_json::to_string(&Cursor {
        phase: if fresh { 1 } else { 6 },
        ..Default::default()
    })
    .unwrap();
    if !fresh {
        argos_osint_core::storage::atlas::insert_run(repo.db(), cycle, &cursor, &stats)
            .await
            .unwrap();
    }
    let now = chrono::Utc::now();
    let request = StartRunRequest::fresh(cycle, "cycle-owner", &cursor, &stats, now);
    let request = if fresh { request } else { request.resume(0, 0) };
    let claim = admission::start_run(&owner, &permit, &request)
        .await
        .unwrap();
    let receipt = admission::stop_run_with_hook(
        &owner,
        &claim,
        RunStop::Completed,
        &cursor,
        &stats,
        "fixture completed",
        chrono::Utc::now(),
        Some(calibration::terminal_hook()),
    )
    .await
    .unwrap();
    let replay = admission::stop_run_with_hook(
        &owner,
        &claim,
        RunStop::Completed,
        &cursor,
        &stats,
        "fixture completed",
        chrono::Utc::now(),
        Some(calibration::terminal_hook()),
    )
    .await
    .unwrap();
    assert!(replay.replayed);
    assert_eq!(receipt.calibration_job_id, replay.calibration_job_id);
    owner.shutdown().await.unwrap();
    receipt
}

async fn report(repo: &Repository, job: &str) -> CalibrationReport {
    let mut response = repo
        .db()
        .query("SELECT VALUE report_json FROM ONLY $row")
        .bind((
            "row",
            surrealdb::types::RecordId::new("classifier_calibration_job", job),
        ))
        .await
        .unwrap()
        .check()
        .unwrap();
    serde_json::from_str(&response.take::<Option<String>>(0).unwrap().unwrap()).unwrap()
}

#[tokio::test]
async fn retention_capacity_is_shared_by_new_families_in_one_page() {
    let _serial = fixture::serial().await;
    let installed = tokio::task::spawn_blocking(fixture::install).await.unwrap();
    let repo = Repository::connect(
        &Target::Embedded {
            path: installed.directory.path().join("retention-db"),
        },
        "argos",
        "retention",
    )
    .await
    .unwrap();
    let ctx = context("capacity-cycle");
    let captured = capture::capture_categories(
        ProviderFormat::CurrentsV1,
        "https://api.currentsapi.services/v1/latest-news",
        &serde_json::json!({"news":[
            {"id":"seed","url":"https://seed.invalid/story","language":"en","category":["science"],
             "title":"Quasar pulsar nebula telescope galaxy orbit cosmic spectrum photon vacuum",
             "description":"Astronomical interferometer maps ultraviolet plasma magnetosphere observations."},
            {"id":"new-a","url":"https://water.invalid/story","language":"en","category":["environment"],
             "title":"River water reservoirs flooded coastal wetlands villages upstream downstream soil",
             "description":"Rainfall submerged farmland after embankment overflow drainage failure."},
            {"id":"new-b","url":"https://trade.invalid/story","language":"en","category":["business"],
             "title":"Factory freight currency exports tariffs shipping logistics manufacturing commerce investment",
             "description":"Merchants transport machinery through warehouses amid elevated customs charges."}
        ]}),
        &ctx,
    );
    assert_eq!(captured.observations.len(), 3);
    // Obtain a real retained signature through the public monitoring path.
    let seed_ctx = context("capacity-seed");
    let mut seed = captured.observations[0].clone();
    seed.cycle_ids = vec![seed_ctx.cycle_id.clone()];
    seed.fetched_at = seed_ctx.fetched_at.clone();
    calibration::capture(&repo, &seed_ctx, &[seed])
        .await
        .unwrap();
    finish(&repo, &installed.home, &seed_ctx.cycle_id, true).await;
    assert_eq!(
        calibration::drain_one(
            &repo,
            &installed.home,
            "capacity-seed",
            std::sync::Arc::new(AtomicBool::new(false))
        )
        .await
        .unwrap(),
        1
    );
    // Fill existing capacity with unrelated retained signatures. These rows
    // are outside either new story's indexed family candidate set.
    repo.db()
        .query("LET $seed=(SELECT * FROM classifier_story_family LIMIT 1)[0]; FOR $key IN $keys { CREATE type::record('classifier_story_family',$key) SET split=$seed.split,signature_json=$seed.signature_json,url_sha256=$seed.url_sha256,content_sha256=$seed.content_sha256,bands=$seed.bands,first_capture_seq=0,last_seen=time::now() RETURN NONE; };")
        .bind(("keys", (0..9998).map(|index|format!("reserved-{index:05}")).collect::<Vec<_>>()))
        .await.unwrap().check().unwrap();
    calibration::capture(&repo, &ctx, &captured.observations[1..])
        .await
        .unwrap();
    let terminal = finish(&repo, &installed.home, &ctx.cycle_id, true).await;
    let job = terminal.calibration_job_id.unwrap();
    assert_eq!(
        calibration::drain_one(
            &repo,
            &installed.home,
            "capacity-monitor",
            std::sync::Arc::new(AtomicBool::new(false))
        )
        .await
        .unwrap(),
        2
    );
    let result = report(&repo, &job).await;
    assert_eq!(
        (result.processed, result.monitored, result.quarantined),
        (2, 1, 1)
    );
    assert_eq!(count(&repo, "classifier_story_family").await, 10000);
    let mut response = repo
        .db()
        .query("SELECT VALUE disposition FROM classifier_observation ORDER BY disposition")
        .await
        .unwrap()
        .check()
        .unwrap();
    assert_eq!(
        response.take::<Vec<String>>(0).unwrap(),
        ["monitoring", "monitoring", "retention_capacity"]
    );
    installed.home.shutdown().await.unwrap();
}

#[tokio::test]
async fn provider_capture_survives_country_filtering_replay_restart_and_bounded_monitoring() {
    let _serial = fixture::serial().await;
    let installed = fixture::install();
    let path = installed.directory.path().join("calibration-db");
    let repo = Repository::connect(
        &Target::Embedded { path: path.clone() },
        "argos",
        "calibration",
    )
    .await
    .unwrap();
    let ctx = context("cycle-a");
    let payload = serde_json::json!({"status":"success","results":[{
        "article_id":"unlocated","title":"Cabinet announces coastal water protection",
        "description":"The minister passed a water conservation law.",
        "link":"https://fictional.invalid/unlocated","language":"en",
        "category":["politics","SPORTS","environment"],"ai_tag":["untrusted-model-tag"]
    }]});
    let batch = capture::capture_categories(
        ProviderFormat::NewsDataV1,
        "https://newsdata.io/api/1/latest",
        &payload,
        &ctx,
    );
    assert_eq!(batch.observations.len(), 1);
    assert_eq!(
        batch.observations[0].raw_categories,
        payload["results"][0]["category"]
    );
    let mapped = provider_labels::map_labels(&batch.observations[0], None).unwrap();
    assert!(!mapped.eligible_for_training);
    calibration::capture(&repo, &ctx, &batch.observations)
        .await
        .unwrap();
    calibration::capture(&repo, &ctx, &batch.observations)
        .await
        .unwrap();
    assert_eq!(count(&repo, "classifier_observation").await, 1);
    assert_eq!(count(&repo, "classifier_observation_origin").await, 1);
    let ctx_b = context("cycle-b");
    let mut other = batch.observations.clone();
    other[0].cycle_ids = vec![ctx_b.cycle_id.clone()];
    calibration::capture(&repo, &ctx_b, &other).await.unwrap();
    assert_eq!(count(&repo, "classifier_observation").await, 1);
    assert_eq!(count(&repo, "classifier_observation_origin").await, 2);
    // The same observation ID cannot silently acquire different source fields.
    other[0].title = "Changed source under the same observation identity".into();
    assert!(calibration::capture(&repo, &ctx_b, &other).await.is_err());
    for page in (0..514).collect::<Vec<_>>().chunks(64) {
        let rows=page.iter().map(|index|serde_json::json!({
            "id":format!("story-{index}"),"title":format!("Independent fixture {index} reports freight clinic operations"),
            "description":"Health authorities delivered supplies.","url":format!("https://fictional.invalid/{index}"),
            "language":"en","category":["health","business"]
        })).collect::<Vec<_>>();
        let captured = capture::capture_categories(
            ProviderFormat::CurrentsV1,
            "https://api.currentsapi.services/v1/latest-news",
            &serde_json::json!({"news":rows}),
            &ctx,
        );
        calibration::capture(&repo, &ctx, &captured.observations)
            .await
            .unwrap();
    }
    let active = std::fs::read(installed.directory.path().join("classifier/active.json")).unwrap();
    let receipt = finish(&repo, &installed.home, "cycle-a", true).await;
    assert_eq!(receipt.observation_watermark, 515);
    let job = receipt.calibration_job_id.unwrap();
    assert_eq!(count(&repo, "classifier_calibration_job").await, 1);
    assert_eq!(
        report(&repo, &job).await.processed,
        0,
        "cycle completion cannot run calibration"
    );
    let cancel = Arc::new(AtomicBool::new(false));
    // An independently restarted monitoring pool must reap its own lost task;
    // recovery cannot depend on the brief worker or process registry running.
    let abandoned = argos_osint_core::storage::jobs::claim_next(
        repo.db(),
        calibration::POOL,
        "lost-monitor",
        60,
        chrono::Utc::now(),
        "lost-monitor-claim",
    )
    .await
    .unwrap()
    .value
    .unwrap();
    repo.db()
        .query("UPDATE $task SET lease_until=time::now()-1s RETURN NONE;")
        .bind((
            "task",
            surrealdb::types::RecordId::new("task", abandoned.id.clone()),
        ))
        .await
        .unwrap()
        .check()
        .unwrap();
    assert_eq!(
        calibration::drain_one(&repo, &installed.home, "monitor", cancel.clone())
            .await
            .unwrap(),
        512
    );
    assert_eq!(report(&repo, &job).await.processed, 512);
    let mut recovery = repo
        .db()
        .query("RETURN count(SELECT id FROM attempt WHERE task=$task AND outcome='lease_expired');")
        .bind((
            "task",
            surrealdb::types::RecordId::new("task", abandoned.id),
        ))
        .await
        .unwrap()
        .check()
        .unwrap();
    assert_eq!(recovery.take::<Option<i64>>(0).unwrap(), Some(1));
    drop(repo);
    let repo = Repository::connect(
        &Target::Embedded { path: path.clone() },
        "argos",
        "calibration",
    )
    .await
    .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    assert_eq!(
        calibration::drain_one(&repo, &installed.home, "monitor-restarted", cancel)
            .await
            .unwrap(),
        3
    );
    let result = report(&repo, &job).await;
    assert_eq!(
        (result.processed, result.state.as_str(), result.promoted),
        (515, "completed", false)
    );
    assert_eq!(result.reason, "insufficient_reviewed_evidence");
    assert_eq!(
        std::fs::read(installed.directory.path().join("classifier/active.json")).unwrap(),
        active
    );
    assert_eq!(count(&repo, "classifier_calibration_job").await, 1);
    installed.home.shutdown().await.unwrap();
}

#[tokio::test]
async fn exhausted_monitor_recovery_is_a_durable_failure_without_model_changes() {
    use argos_osint_core::storage::jobs;
    let _serial = fixture::serial().await;
    let installed = fixture::install();
    let repo = Repository::connect(
        &Target::Embedded {
            path: installed.directory.path().join("exhausted-db"),
        },
        "argos",
        "calibration_exhausted",
    )
    .await
    .unwrap();
    let ctx = context("exhausted-cycle");
    let batch = capture::capture_categories(
        ProviderFormat::CurrentsV1,
        "https://api.currentsapi.services/v1/latest-news",
        &serde_json::json!({"news":[{
            "id":"one","title":"Fictional clinic supplies","url":"https://fictional.invalid/one","category":["health"]
        }]}),
        &ctx,
    );
    calibration::capture(&repo, &ctx, &batch.observations)
        .await
        .unwrap();
    let active = std::fs::read(installed.directory.path().join("classifier/active.json")).unwrap();
    let job = finish(&repo, &installed.home, "exhausted-cycle", true)
        .await
        .calibration_job_id
        .unwrap();
    let claimed = jobs::claim_next(
        repo.db(),
        calibration::POOL,
        "lost-monitor",
        60,
        chrono::Utc::now(),
        "exhausted-monitor-claim",
    )
    .await
    .unwrap()
    .value
    .unwrap();
    repo.db()
        .query("UPDATE $task SET lease_until=time::now()-1s,recoveries=$recoveries RETURN NONE;")
        .bind(("task", surrealdb::types::RecordId::new("task", claimed.id)))
        .bind(("recoveries", jobs::MAX_WORKER_RECOVERIES - 1))
        .await
        .unwrap()
        .check()
        .unwrap();
    assert_eq!(
        calibration::drain_one(
            &repo,
            &installed.home,
            "restarted-monitor",
            Arc::new(AtomicBool::new(false))
        )
        .await
        .unwrap(),
        0
    );
    let failure = report(&repo, &job).await;
    assert_eq!(
        (failure.state.as_str(), failure.reason.as_str()),
        ("failed", "worker_recovery_exhausted")
    );
    assert_eq!(failure.processed, 0);
    assert!(!failure.promoted);
    assert_eq!(
        std::fs::read(installed.directory.path().join("classifier/active.json")).unwrap(),
        active
    );
    installed.home.shutdown().await.unwrap();
}

#[tokio::test]
async fn empty_terminal_is_completed_without_model_and_observed_terminal_is_blocked() {
    let directory = tempfile::tempdir().unwrap();
    let repo = Repository::connect(
        &Target::Embedded {
            path: directory.path().join("db"),
        },
        "argos",
        "calibration_empty",
    )
    .await
    .unwrap();
    let home = ClassifierHome::new(directory.path().join("no-model"));
    let empty = finish(&repo, &home, "empty", false).await;
    let result = report(&repo, &empty.calibration_job_id.unwrap()).await;
    assert_eq!(
        (result.state.as_str(), result.reason.as_str()),
        ("completed", "no_new_data")
    );
    let ctx = context("observed");
    let batch = capture::capture_categories(
        ProviderFormat::CurrentsV1,
        "https://api.currentsapi.services/v1/latest-news",
        &serde_json::json!({"news":[{
            "id":"one","title":"Clinic supplies","url":"https://fictional.invalid/one","category":["health"]
        }]}),
        &ctx,
    );
    calibration::capture(&repo, &ctx, &batch.observations)
        .await
        .unwrap();
    let observed = finish(&repo, &home, "observed", false).await;
    let result = report(&repo, &observed.calibration_job_id.unwrap()).await;
    assert_eq!(
        (result.state.as_str(), result.reason.as_str()),
        ("blocked", "missing_model")
    );
    home.shutdown().await.unwrap();
}
