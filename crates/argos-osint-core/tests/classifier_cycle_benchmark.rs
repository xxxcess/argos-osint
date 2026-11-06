//! Opt-in current whole-cycle replay using the immutable intake provider corpus.
//! Invoke one fresh process per explicitly designated isolated benchmark home.
#![cfg(all(feature = "embedded", feature = "classifier"))]
#[path = "support/actor_catalog.rs"]
mod actors;
#[path = "fixtures/classifier/admission-support.rs"]
mod fixture;
use argos_osint_core as core;
use std::collections::BTreeMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde_json::{json, Value};

fn micros(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_micros()).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "manual immutable whole-cycle replay on an isolated benchmark home"]
async fn offline_fixed_whole_cycle_benchmark() {
    let home = std::path::PathBuf::from(
        std::env::var_os("ARGOS_BENCH_HOME").expect("explicit isolated benchmark home required"),
    );
    assert!(home.is_absolute());
    let mode =
        std::env::var("ARGOS_BENCH_MODE").expect("explicit fresh/reopen/initialize mode required");
    assert!(["fresh", "reopen", "initialize"].contains(&mode.as_str()));
    assert_eq!(
        core::paths::home_dir(),
        home,
        "ARGOS_HOME must match the isolated benchmark home"
    );
    std::fs::create_dir_all(&home).unwrap();
    let _serial = fixture::serial().await;
    let installed = tokio::task::spawn_blocking(fixture::install).await.unwrap();
    let settings = core::provider::SettingsFile {
        data_paths: core::data_paths::DataPaths {
            classifier_dir: installed
                .directory
                .path()
                .join("classifier")
                .display()
                .to_string(),
            ..Default::default()
        },
        ..Default::default()
    };
    settings.save_to(&home.join("config.toml")).unwrap();
    let layout = core::storage::home::Layout::new(&home);
    if mode != "reopen" {
        assert!(
            !layout.config().exists(),
            "fresh benchmark home must not contain prior data"
        );
        core::storage::home::save_config(
            &layout,
            &core::storage::home::StorageConfig {
                mode: core::storage::home::Mode::Embedded,
                port: None,
            },
        )
        .unwrap();
    }
    core::storage::runtime::init(layout).unwrap();
    let opening = Instant::now();
    let repo = core::storage::runtime::repository().await.unwrap();
    let storage_open_us = micros(opening);
    if mode != "reopen" {
        actors::seed_actor_catalog(&repo, &["Unmatched benchmark catalog actor"]).await;
    }
    let store = core::atlas_store::AtlasStore::from_repository(repo.clone());
    assert!(
        store.list_runs().await.unwrap().is_empty(),
        "whole-cycle workload requires an EMPTY database snapshot"
    );
    // Identical retained, fresh publisher fixture in intake and current runs.
    // It matches no outlet, so priors remain unrated without network access.
    let publisher_index = core::osint::wikipedia_rsp::RspIndex::build(
        vec![core::osint::wikipedia_rsp::RspEntry {
            name: "Unmatched offline publisher fixture".into(),
            domains: vec!["unmatched-publisher.invalid".into()],
            status: core::osint::wikipedia_rsp::RspStatus::NoConsensus,
            blacklisted: false,
            last_year: "2026".into(),
            summary: "Synthetic benchmark fixture".into(),
        }],
        "2026-11-05T12:19:30Z".into(),
    );
    let publisher_json = core::osint::wikipedia_rsp::index_to_json(&publisher_index).unwrap();
    store
        .app_state_set(core::osint::wikipedia_rsp::APP_STATE_KEY, &publisher_json)
        .await
        .unwrap();
    core::osint::wikipedia_rsp::install_index(publisher_index);
    if mode == "initialize" {
        core::storage::runtime::drain().await;
        println!(
            "ARGOS_CYCLE_INIT {}",
            json!({"storage_open_us":storage_open_us,"state":"initialized_empty_schema"})
        );
        return;
    }
    let contention = std::env::var("ARGOS_BENCH_CALIBRATION").unwrap_or_else(|_| "off".into());
    assert!(["off", "control", "overlap"].contains(&contention.as_str()));
    let monitoring_job = if contention == "off" {
        None
    } else {
        Some(prepare_monitoring(&repo, &installed.home).await)
    };
    let monitoring_worker = if contention == "overlap" {
        Some(
            start_overlapping_monitor(
                &repo,
                installed.home.clone(),
                monitoring_job.as_ref().unwrap(),
            )
            .await,
        )
    } else {
        None
    };
    let monitoring_before = match &monitoring_job {
        Some(job) => Some(monitoring_report(&repo, job).await),
        None => None,
    };
    let raw = include_bytes!("fixtures/classifier/performance-cycle-responses.json");
    assert_eq!(
        core::classifier::artifact::sha256_hex(raw),
        "c9268ef2c193e0cd831e40a389d5dedcf5d66c53fd98a09e7decb33430c10567"
    );
    let fixture: Value = serde_json::from_slice(raw).unwrap();
    let published =
        chrono::DateTime::parse_from_rfc3339(fixture["frozen_published_at"].as_str().unwrap())
            .unwrap();
    assert!(
        chrono::Utc::now()
            .signed_duration_since(published)
            .num_hours()
            < 48,
        "frozen corpus has aged out; do not alter dates to fabricate comparable admission"
    );
    let responses: BTreeMap<String, Value> =
        serde_json::from_value(fixture["bodies"].clone()).unwrap();
    let counts = Arc::new(Mutex::new(BTreeMap::<String, u64>::new()));
    let fetched = counts.clone();
    let fetch = move |call: core::atlas::HttpCall| {
        let provider = call.provider;
        let body = responses
            .get(&provider)
            .expect("unexpected acquisition provider")
            .to_string();
        *fetched.lock().unwrap().entry(provider).or_default() += 1;
        Box::pin(async move {
            Ok(core::atlas::HttpReply {
                status: 200,
                body,
                location: None,
            })
        })
            as std::pin::Pin<
                Box<
                    dyn std::future::Future<Output = anyhow::Result<core::atlas::HttpReply>> + Send,
                >,
            >
    };
    let pause = AtomicBool::new(false);
    let keys = core::osint::ProviderKeys {
        gnews: "fixture-gnews".into(),
        newsdata: "fixture-newsdata".into(),
        newsapi: "fixture-newsapi".into(),
        currents: "fixture-currents".into(),
        ..Default::default()
    };
    let db = home.join("argos.db");
    let started = Instant::now();
    let stop = core::atlas::run_atlas(
        core::atlas::RunInput {
            db_path: &db,
            pause: &pause,
            keys: &keys,
            user_agent: "ArgosOfflineFixedBenchmark",
            pace: false,
            resume: false,
            feed: &[],
            classifier: None,
            synthesizer: None,
            synthesizer_fallbacks: &[],
            run_id: None,
        },
        |_| {},
        fetch,
    )
    .await
    .unwrap();
    let cycle_us = micros(started);
    assert!(matches!(stop, core::atlas::Stop::Finished));
    let runs = store.list_runs().await.unwrap();
    assert_eq!(runs.len(), if monitoring_job.is_some() { 2 } else { 1 });
    let run = runs
        .iter()
        .find(|run| run.id != "benchmark-monitoring")
        .expect("foreground run");
    assert_eq!(
        run.state, "completed",
        "partial/waiting is not a full-cycle result: {}",
        run.note
    );
    let rows = store.list_articles(&run.id).await.unwrap();
    let mut ids: Vec<_> = rows.iter().map(|row| row.id.clone()).collect();
    ids.sort();
    let mut expected: Vec<String> =
        serde_json::from_value(fixture["expected_url_ids"].clone()).unwrap();
    expected.sort();
    assert_eq!(ids, expected, "fixed admitted occurrence workload changed");
    let acquired = counts.lock().unwrap().clone();
    assert_eq!(
        acquired,
        BTreeMap::from([
            ("currents".into(), 1),
            ("gnews".into(), 1),
            ("newsapi".into(), 1),
            ("newsdata".into(), 1)
        ])
    );
    let stats: core::atlas::RunStats = serde_json::from_str(&run.stats_json).unwrap();
    assert_eq!(stats.coverage.admitted, 4);
    assert_eq!(stats.coverage.actor_index_failed, 0);
    let monitoring_rows = match monitoring_worker {
        Some(worker) => Some(worker.await.unwrap()),
        None => None,
    };
    let monitoring_after = match &monitoring_job {
        Some(job) => Some(monitoring_report(&repo, job).await),
        None => None,
    };
    let drain = Instant::now();
    core::storage::runtime::drain().await;
    let drain_us = micros(drain);
    let stages = core::storage::telemetry::events_of_type(repo.db(), "atlas_stage")
        .await
        .unwrap();
    let stages: Vec<Value> = stages
        .into_iter()
        .filter(|event| event.run_id == run.id)
        .map(|event| json!({"stage":serde_json::from_str::<Value>(&event.payload_json).unwrap(),"wall_ms":event.metric_ms}))
        .collect();
    println!(
        "ARGOS_CYCLE_RESULT {}",
        json!({"schema":"argos-fixed-whole-cycle-v1","implementation":"current-local-classifier","condition":mode,"fresh_process":true,"initialized_empty_snapshot_reopened":mode=="reopen","runtime_prewarms":false,"publisher_index_fixture":"fresh retained unmatched publisher, installed identically in both implementations","global_OS_cache_flushed":false,
        "fixture_sha256":core::nlp::cache::hash(std::str::from_utf8(raw).unwrap()),"frozen_published_at":fixture["frozen_published_at"],"lookback_hours":48,
        "storage_open_us":storage_open_us,"cycle_us":cycle_us,"ordered_storage_drain_us":drain_us,"terminal_state":run.state,"actual_admitted_ids":ids,"actual_pairs":rows.iter().map(|row|(&row.id,&row.categories)).collect::<BTreeMap<_,_>>(),
        "acquisition_calls":acquired,"classification_provider_requests":0,"catalog_fixture":"one unmatched canonical actor; no actor mentions","stats":stats,"actual_stage_events":stages,
        "calibration_condition":contention,"monitoring_before":monitoring_before,"monitoring_after":monitoring_after,"monitoring_activation_rows":monitoring_rows,"rss":null,"rss_reason":"external sampler result is recorded separately","provider_latency_scope":"scripted loopback only; no real-provider latency claim"})
    );
    installed.home.shutdown().await.unwrap();
}

/// Prepares identical pending monitoring work in both control and overlap
/// cases. Capture/setup is outside the measured foreground cycle interval.
async fn prepare_monitoring(
    repo: &core::storage::repo::Repository,
    home: &core::classifier::readiness::ClassifierHome,
) -> String {
    use core::classifier::provider_capture::{self, CaptureContext};
    use core::classifier::provider_labels::ProviderFormat;
    use core::classifier::readiness::{self, CycleRequirement};
    use core::storage::classifier_admission as admission;
    use core::storage::classifier_calibration as calibration;
    let ctx = CaptureContext::new(
        "benchmark-monitoring",
        chrono::Utc::now().to_rfc3339(),
        "benchmark-fixed-fetch",
        "1".repeat(64),
    );
    for indices in (0..512).collect::<Vec<_>>().chunks(64) {
        let rows = indices
            .iter()
            .map(|index| {
                json!({
                    "id":format!("monitor-story-{index:04}"),
                    "title":format!("Independent fictional freight clinic bulletin {index:04}"),
                    "description":"Fictional health authorities delivered clinic supplies.",
                    "url":format!("https://monitoring.invalid/story/{index:04}"),
                    "language":"en","category":["business","health"]
                })
            })
            .collect::<Vec<_>>();
        let captured = provider_capture::capture_categories(
            ProviderFormat::CurrentsV1,
            "https://api.currentsapi.services/v1/latest-news",
            &json!({"news":rows}),
            &ctx,
        );
        assert_eq!(captured.observations.len(), 64);
        calibration::capture(repo, &ctx, &captured.observations)
            .await
            .unwrap();
    }
    let owner = admission::AdmissionOwner::new(repo.clone());
    let permit = readiness::preflight_cycle(home, CycleRequirement::FreshAcquisition, None)
        .await
        .unwrap();
    let stats = core::atlas::RunStats {
        classifier_snapshot: permit.pinned().map(|pin| pin.snapshot.clone()),
        ..Default::default()
    };
    let cursor = serde_json::to_string(&core::atlas::Cursor::default()).unwrap();
    let stats = serde_json::to_string(&stats).unwrap();
    let run = admission::start_run(
        &owner,
        &permit,
        &admission::StartRunRequest::fresh(
            &ctx.cycle_id,
            "monitor-preparation",
            &cursor,
            &stats,
            chrono::Utc::now(),
        ),
    )
    .await
    .unwrap();
    let terminal = admission::stop_run_with_hook(
        &owner,
        &run,
        admission::RunStop::Completed,
        &cursor,
        &stats,
        "fixed background benchmark cohort",
        chrono::Utc::now(),
        Some(calibration::terminal_hook()),
    )
    .await
    .unwrap();
    assert_eq!(terminal.observation_watermark, 512);
    let job = terminal.calibration_job_id.unwrap();
    owner.shutdown().await.unwrap();
    assert_eq!(
        monitoring_report(repo, &job).await.processed,
        0,
        "cycle completion must not process calibration"
    );
    job
}

async fn monitoring_report(
    repo: &core::storage::repo::Repository,
    job: &str,
) -> core::storage::classifier_calibration::CalibrationReport {
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

async fn start_overlapping_monitor(
    repo: &core::storage::repo::Repository,
    home: Arc<core::classifier::readiness::ClassifierHome>,
    job: &str,
) -> tokio::task::JoinHandle<usize> {
    let worker_repo = repo.clone();
    let worker = tokio::spawn(async move {
        core::storage::classifier_calibration::drain_one(
            &worker_repo,
            &home,
            "overlapping-benchmark-monitor",
            Arc::new(AtomicBool::new(false)),
        )
        .await
        .unwrap()
    });
    // A committed first page proves actual background execution rather than
    // measuring a task that has not started. Later pages may overlap admission.
    tokio::time::timeout(std::time::Duration::from_secs(60), async {
        loop {
            let report = monitoring_report(repo, job).await;
            if report.processed >= 64 {
                assert!(
                    report.processed < 512 && !worker.is_finished(),
                    "background cohort ended before the measured foreground cycle"
                );
                break;
            }
            assert!(
                !worker.is_finished(),
                "monitor exited before its first checkpoint"
            );
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("monitor must checkpoint a real page");
    worker
}
