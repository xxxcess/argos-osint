//! Opt-in immutable offline Natural inference measurements for the legacy engine.
//! Run explicitly with --ignored --nocapture on an otherwise idle host.
#![cfg(feature = "classifier")]

#[path = "fixtures/classifier/admission-support.rs"]
mod fixture;

use std::collections::BTreeMap;
use std::time::Instant;

use argos_osint_core::atlas_classify::AdmissionContext;
use argos_osint_core::classifier::artifact::sha256_hex;
use argos_osint_core::classifier::{ClassifyInput, WorkControl};
use serde_json::{json, Value};

fn micros(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_micros()).unwrap()
}
fn distribution(samples: &[u64]) -> Value {
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let rank = |percent: usize| sorted[(sorted.len() * percent).div_ceil(100) - 1];
    json!({"samples":samples,"count":samples.len(),"p50_us":rank(50),"p95_us":rank(95)})
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "manual immutable offline benchmark; source hashes and fixture required"]
async fn offline_fixed_current_classifier_benchmark() {
    let bytes = include_bytes!("fixtures/classifier/performance-corpus.json");
    assert_eq!(
        sha256_hex(bytes),
        "da672f8412f16aa4db673ff2caa26b75cd9f3529d793e5d79fdf573392565515"
    );
    let rows: Vec<argos_osint_core::store::AtlasArticleRow> =
        serde_json::from_slice(bytes).unwrap();
    assert_eq!(rows.len(), 64);
    let fitting_started = Instant::now();
    let _serial = fixture::serial().await;
    let installed = tokio::task::spawn_blocking(fixture::install).await.unwrap();
    let fixture_fit_us = micros(fitting_started);
    let pin_started = Instant::now();
    let pin = installed.home.pin(None).await.unwrap();
    let pin_startup_us = micros(pin_started);
    let context = AdmissionContext::load(installed.directory.path(), pin.clone()).unwrap();
    let inputs: Vec<_> = rows
        .iter()
        .map(|row| ClassifyInput::new(&row.id, &row.title, &row.description, None))
        .collect();
    let manifest_sha = context.manifest_sha256.clone();
    let corpus_sha = sha256_hex(bytes);
    let service = pin.service.clone();
    let result = tokio::task::spawn_blocking(move || {
        let mut singleton = Vec::new();
        let mut batch64 = Vec::new();
        let mut first_singleton_us = None;
        let mut first64_us = None;
        let mut actual_pairs = BTreeMap::new();
        for iteration in 0..33 {
            for size in [1, 64] {
                let started = Instant::now();
                let receipts = service.classify_batch_with_control(inputs[..size].to_vec(), &WorkControl::default()).unwrap();
                let elapsed = micros(started);
                assert_eq!(receipts.len(), size);
                for receipt in receipts {
                    assert!(receipt.rejection_reason.is_none(), "fixed admitted workload changed");
                    let pair = receipt.selected_domains.clone();
                    assert_ne!(pair[0],pair[1]);
                    actual_pairs.insert(receipt.occurrence_id, pair);
                }
                if iteration == 0 {
                    if size == 1 {first_singleton_us=Some(elapsed);} else {first64_us=Some(elapsed);}
                } else if size == 1 {singleton.push(elapsed);} else {batch64.push(elapsed);}
            }
        }
        json!({
            "schema":"argos-fixed-workload-benchmark-v1","implementation":"legacy-natural-classifier",
            "corpus_sha256":corpus_sha,"admitted_ids":inputs.iter().map(|input|&input.occurrence_id).collect::<Vec<_>>(),
            "actual_returned_pairs":actual_pairs,"actual_model_snapshot":service.pinned_model(),"manifest_sha256":manifest_sha,
            "fixture_fit_us":fixture_fit_us,"pin_startup_us":pin_startup_us,"first_singleton_us":first_singleton_us,"first64_us":first64_us,
            "warm_singleton":distribution(&singleton),"warm_batch64":distribution(&batch64),"actual_worker_performance":service.performance().unwrap(),
            "aggregate_metrics":service.metrics().unwrap(),"classification_provider_requests":0,
            "cold_distribution":null,"cold_distribution_reason":"one fresh process observation; repeat process for cold distribution",
            "rss":null,"rss_reason":"external process sampler not attached to this initial engine experiment",
            "whole_cycle":null,"whole_cycle_reason":"separate actual cycle experiment required",
            "storage":null,"storage_reason":"direct engine phase; actual DAO experiment measured separately",
            "contention":null,"contention_reason":"direct inference experiment; separate actual drainer contention experiment required"
        })
    }).await.unwrap();
    println!("ARGOS_BENCH_RESULT {result}");
    drop(context);
    drop(pin);
    installed.home.shutdown().await.unwrap();
}
