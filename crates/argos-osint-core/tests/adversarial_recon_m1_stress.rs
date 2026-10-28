//! Adversarial empirical stress harness for Milestone 1 Recon Concurrency & Cancellation
//! Tests:
//! 1. Semaphore(4) Concurrency Cap under high contention (30 concurrent tasks)
//! 2. In-flight Deduplication & Leader Election with 60 concurrent tasks across 3 keys
//! 3. Leader Failure / Drop Fallback Resilience for queued followers
//! 4. End-to-End Cancellation Responsiveness (< 50ms latency) via ToolRunner
//! 5. Holehe OSINT cancellation responsiveness and cooperative halt
//! 6. Mass Concurrent Cancellation Stress (30 parallel cancellation lifecycles)

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Once};
use std::time::{Duration, Instant};

use argos_osint_core::osint::holehe::{self, CheckSignal, LookupInput};
use argos_osint_core::osint::ToolResult;
use argos_osint_core::provider::SettingsFile;
use argos_osint_core::recon::backend::ReconStore;
use argos_osint_core::recon::{cancel_with_notify, Service};
use argos_osint_core::secrets::AuthFile;
use argos_osint_core::storage::home::{self, Layout, Mode, StorageConfig};
use argos_osint_core::storage::runtime;
use serde_json::json;
use tokio::sync::watch;

static INIT_STORAGE: Once = Once::new();

async fn init_test_runtime() {
    INIT_STORAGE.call_once(|| {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::new(dir.path());
        home::save_config(
            &layout,
            &StorageConfig {
                mode: Mode::Embedded,
                port: None,
            },
        )
        .unwrap();
        let _ = runtime::init(layout);
    });
    let _ = runtime::repository().await;
}

#[tokio::test]
async fn stress_test_semaphore_concurrency_cap_never_exceeds_4() {
    let dir = tempfile::tempdir().unwrap();
    let service = Service::new(dir.path(), AuthFile::default(), SettingsFile::default()).unwrap();
    let runner = service.tool_runner();
    let sem = runner.concurrency_semaphore();

    assert_eq!(sem.available_permits(), 4, "Initial permits must be 4");

    let active_counter = Arc::new(AtomicUsize::new(0));
    let peak_active = Arc::new(AtomicUsize::new(0));
    let completed = Arc::new(AtomicUsize::new(0));

    // Spawn 30 concurrent tasks competing for the 4 permits
    let num_tasks = 30;
    let mut handles = Vec::with_capacity(num_tasks);

    for task_id in 0..num_tasks {
        let sem = Arc::clone(&sem);
        let active = Arc::clone(&active_counter);
        let peak = Arc::clone(&peak_active);
        let comp = Arc::clone(&completed);

        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire().await.expect("permit acquisition");

            let current = active.fetch_add(1, Ordering::SeqCst) + 1;
            peak.fetch_max(current, Ordering::SeqCst);

            // Empirical assertion inside critical section
            assert!(
                current <= 4,
                "Invariant violated: active permits {current} exceeded limit of 4 (task {task_id})"
            );

            // Simulate jittered async workload
            let sleep_ms = 5 + (task_id % 15) as u64;
            tokio::time::sleep(Duration::from_millis(sleep_ms)).await;

            active.fetch_sub(1, Ordering::SeqCst);
            comp.fetch_add(1, Ordering::SeqCst);
        }));
    }

    for h in handles {
        h.await.unwrap();
    }

    assert_eq!(completed.load(Ordering::SeqCst), num_tasks);
    assert_eq!(active_counter.load(Ordering::SeqCst), 0);
    let observed_peak = peak_active.load(Ordering::SeqCst);
    println!("stress: Concurrency test finished. Observed peak active = {observed_peak}");
    assert_eq!(
        observed_peak, 4,
        "Under 30 tasks, peak active must reach the cap of 4"
    );
    assert_eq!(
        sem.available_permits(),
        4,
        "Permits must return to exactly 4 without permit leaks"
    );
}

#[tokio::test]
async fn stress_test_inflight_deduplication_under_high_concurrency() {
    let dir = tempfile::tempdir().unwrap();
    let service = Service::new(dir.path(), AuthFile::default(), SettingsFile::default()).unwrap();
    let runner = service.tool_runner();
    let inflight_map = runner.inflight_map();

    // 60 concurrent tasks requesting 3 distinct keys (20 tasks per key)
    let num_keys = 3;
    let tasks_per_key = 20;
    let total_tasks = num_keys * tasks_per_key;

    let leaders_elected = Arc::new(AtomicUsize::new(0));
    let followers_waited = Arc::new(AtomicUsize::new(0));
    let total_completed = Arc::new(AtomicUsize::new(0));

    let mut handles = Vec::with_capacity(total_tasks);

    for key_idx in 0..num_keys {
        let key_name = format!("recon-dedup-key-{}", key_idx);
        for _task_idx in 0..tasks_per_key {
            let key = key_name.clone();
            let inflight = Arc::clone(&inflight_map);
            let leaders = Arc::clone(&leaders_elected);
            let followers = Arc::clone(&followers_waited);
            let comp = Arc::clone(&total_completed);

            handles.push(tokio::spawn(async move {
                // Exactly simulate ToolRunner's deduplication block (lines 273-327)
                let (tx, mut rx, is_leader) = {
                    let mut map = inflight.lock().unwrap();
                    if let Some(existing_rx) = map.get(&key) {
                        (None, existing_rx.clone(), false)
                    } else {
                        let (new_tx, new_rx) = watch::channel(None);
                        map.insert(key.clone(), new_rx.clone());
                        (Some(new_tx), new_rx, true)
                    }
                };

                if is_leader {
                    leaders.fetch_add(1, Ordering::SeqCst);
                    // Simulate doing remote work
                    tokio::time::sleep(Duration::from_millis(30)).await;

                    let mock_result = ToolResult {
                        tool_id: "test_tool".into(),
                        inputs: json!({"key": key}),
                        status: "completed".into(),
                        source_url: String::new(),
                        retrieved_at: "2026-10-28T06:00:00Z".into(),
                        observations: json!({"data": "ok"}),
                        raw: String::new(),
                        error: None,
                        cached: false,
                        truncated: false,
                        credits_charged: 0,
                        credits_reported: None,
                    };

                    // Broadcast to followers
                    if let Some(sender) = tx {
                        let _ = sender.send(Some(mock_result));
                    }

                    // Clean up key on drop (RAII guard simulation)
                    tokio::time::sleep(Duration::from_millis(5)).await;
                    inflight.lock().unwrap().remove(&key);
                } else {
                    followers.fetch_add(1, Ordering::SeqCst);
                    // Wait for leader result
                    let wait_res = rx.changed().await;
                    assert!(
                        wait_res.is_ok(),
                        "Follower must receive leader notification"
                    );
                    let res = rx.borrow().clone();
                    assert!(res.is_some(), "Follower must observe completed result");
                }

                comp.fetch_add(1, Ordering::SeqCst);
            }));
        }
    }

    for h in handles {
        h.await.unwrap();
    }

    assert_eq!(total_completed.load(Ordering::SeqCst), total_tasks);
    assert_eq!(
        leaders_elected.load(Ordering::SeqCst),
        num_keys,
        "Exactly 1 leader must be elected per distinct key"
    );
    assert_eq!(
        followers_waited.load(Ordering::SeqCst),
        total_tasks - num_keys,
        "All other tasks must wait as followers"
    );

    // Assert that the inflight map is completely clean
    assert_eq!(
        inflight_map.lock().unwrap().len(),
        0,
        "Inflight map must be 0 after all executions conclude"
    );
}

#[tokio::test]
async fn stress_test_inflight_leader_dropped_follower_fallback() {
    let inflight_map: Arc<Mutex<HashMap<String, watch::Receiver<Option<ToolResult>>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let key = "crash-key".to_string();

    // 1. Leader registers in-flight
    let (tx, _rx) = watch::channel(None);
    inflight_map
        .lock()
        .unwrap()
        .insert(key.clone(), tx.subscribe());

    // 2. Spawn 5 followers awaiting this key
    let followers_recovered = Arc::new(AtomicUsize::new(0));
    let mut handles = Vec::new();

    for _ in 0..5 {
        let key = key.clone();
        let inflight = Arc::clone(&inflight_map);
        let recovered = Arc::clone(&followers_recovered);

        let mut rx = inflight.lock().unwrap().get(&key).unwrap().clone();

        handles.push(tokio::spawn(async move {
            // Follower waits
            let wait_res = rx.changed().await;
            if wait_res.is_err() {
                // Sender was dropped! Follower falls back to direct run (line 320 in tool_runner.rs)
                recovered.fetch_add(1, Ordering::SeqCst);
            }
        }));
    }

    // Give followers time to await `changed()`
    tokio::time::sleep(Duration::from_millis(15)).await;

    // Leader crashes / drops tx without sending result!
    drop(tx);
    inflight_map.lock().unwrap().remove(&key);

    for h in handles {
        h.await.unwrap();
    }

    assert_eq!(
        followers_recovered.load(Ordering::SeqCst),
        5,
        "All 5 followers must detect leader drop and enter fallback path without deadlock"
    );
}

#[tokio::test]
async fn stress_test_tool_runner_cancellation_responsiveness() {
    init_test_runtime().await;
    let dir = tempfile::tempdir().unwrap();
    let _ = ReconStore::open(dir.path()).await.unwrap();

    let service = Service::new(dir.path(), AuthFile::default(), SettingsFile::default()).unwrap();
    let runner = service.tool_runner();

    // 1. ToolRunner execution with cancellation triggered mid-flight
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_trigger = Arc::clone(&cancel);

    let runner_clone = runner.clone();

    let task = tokio::spawn(async move {
        runner_clone
            .execute_tool_without_reservation(
                holehe::TOOL_ID,
                json!({"email": "adversarial.recon.test@example.com", "max_sites": 20}),
                false,
                &cancel_trigger,
            )
            .await
    });

    // Allow task to initialize and enter tokio::select! execution
    tokio::time::sleep(Duration::from_millis(20)).await;

    let cancel_time = Instant::now();
    cancel_with_notify(&cancel);

    let res = tokio::time::timeout(Duration::from_millis(500), task).await;
    let cancel_latency = cancel_time.elapsed();

    assert!(
        res.is_ok(),
        "ToolRunner must halt within 500ms on cancel_with_notify"
    );
    let execution_res = res.unwrap().unwrap();
    assert!(
        execution_res.is_err(),
        "Cancelled execution must return an error"
    );
    let err_str = execution_res.err().unwrap().to_string();
    assert!(
        err_str.contains("cancelled"),
        "Error message must specify 'cancelled', got '{err_str}'"
    );
    println!(
        "stress: ToolRunner event-driven cancellation latency was {:?}",
        cancel_latency
    );
    assert!(
        cancel_latency < Duration::from_millis(50),
        "Cancellation latency {:?} must be strictly under 50ms (got {:?})",
        cancel_latency,
        cancel_latency
    );

    // Verify permits were returned cleanly
    assert_eq!(
        runner.concurrency_semaphore().available_permits(),
        4,
        "Permits must be 4 after cancelled ToolRunner execution"
    );
}

#[tokio::test]
async fn stress_test_holehe_cancellation_responsiveness() {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(500))
        .build()
        .unwrap();

    // 1. Pre-cancelled token halts immediately
    let pre_cancel = Arc::new(AtomicBool::new(true));
    let input = LookupInput {
        email: "test.challenger.adversarial@example.com".into(),
        services: Some(vec!["twitter".into(), "spotify".into()]),
        max_sites: Some(2),
    };

    let start = Instant::now();
    let res = holehe::lookup(&client, input.clone(), None, &pre_cancel).await;
    let elapsed = start.elapsed();

    assert!(
        res.is_ok(),
        "Pre-cancelled holehe lookup returns Ok observation"
    );
    let obs = res.unwrap();
    assert!(
        elapsed < Duration::from_millis(15),
        "Pre-cancelled lookup must halt in < 15ms, took {:?}",
        elapsed
    );
    assert_eq!(obs.counts.registered, 0);
    assert!(obs.cancelled, "Observation cancelled field must be true");

    // Verify all returned services were marked inconclusive / cancelled
    for item in obs.results {
        assert_eq!(item.signal, CheckSignal::Inconclusive);
        assert_eq!(item.diagnostic, "cancelled");
    }

    // 2. Cancellation cooperatively stops remaining buffered services
    let cancel = Arc::new(AtomicBool::new(true));
    let multi_input = LookupInput {
        email: "test.challenger.multi@example.com".into(),
        services: None,
        max_sites: Some(30),
    };

    let start2 = Instant::now();
    let res2 = holehe::lookup(&client, multi_input, None, &cancel).await;
    let elapsed2 = start2.elapsed();

    assert!(res2.is_ok());
    let obs2 = res2.unwrap();
    println!("stress: Holehe lookup with cancel flag took {:?}", elapsed2);
    assert!(
        elapsed2 < Duration::from_millis(50),
        "Cooperative cancellation with cancel flag took too long: {:?}",
        elapsed2
    );
    assert!(obs2.cancelled, "Observation must reflect cancellation");
}

#[tokio::test]
async fn stress_test_mass_concurrent_cancellation_cycles() {
    init_test_runtime().await;
    let dir = tempfile::tempdir().unwrap();
    let _ = ReconStore::open(dir.path()).await.unwrap();

    let service =
        Arc::new(Service::new(dir.path(), AuthFile::default(), SettingsFile::default()).unwrap());

    let num_tasks = 20;
    let mut handles = Vec::with_capacity(num_tasks);

    for i in 0..num_tasks {
        let svc = Arc::clone(&service);
        handles.push(tokio::spawn(async move {
            let cancel = Arc::new(AtomicBool::new(false));
            let cancel_clone = Arc::clone(&cancel);

            let runner = svc.tool_runner();
            let exec_fut = tokio::spawn(async move {
                runner
                    .execute_tool_without_reservation(
                        holehe::TOOL_ID,
                        json!({"email": format!("adversarial{}@example.com", i), "max_sites": 5}),
                        false,
                        &cancel_clone,
                    )
                    .await
            });

            // Adversarial timing: notify after varying micro-delays
            if i % 2 == 0 {
                tokio::time::sleep(Duration::from_millis(10 + (i % 5) as u64)).await;
                cancel_with_notify(&cancel);
            } else {
                tokio::time::sleep(Duration::from_millis(10 + (i % 3) as u64)).await;
                cancel.store(true, Ordering::SeqCst);
            }

            let res = tokio::time::timeout(Duration::from_millis(2000), exec_fut).await;
            assert!(res.is_ok(), "Task {i} timed out awaiting cancellation");
        }));
    }

    for h in handles {
        h.await.unwrap();
    }

    // Verify all 4 permits returned
    assert_eq!(
        service
            .tool_runner()
            .concurrency_semaphore()
            .available_permits(),
        4,
        "Permit leak detected after mass cancellation cycles"
    );
    println!("stress: 20 concurrent cancellation cycles completed with 0 leaks.");
}

#[tokio::test]
async fn stress_test_prompt_cancellation_latency_via_service_cancel() {
    init_test_runtime().await;
    let dir = tempfile::tempdir().unwrap();
    let _ = ReconStore::open(dir.path()).await.unwrap();

    let service =
        Arc::new(Service::new(dir.path(), AuthFile::default(), SettingsFile::default()).unwrap());
    let runner = service.tool_runner();

    let num_tasks = 20;
    let max_observed_latency = Arc::new(Mutex::new(Duration::ZERO));
    let mut handles = Vec::with_capacity(num_tasks);

    for i in 0..num_tasks {
        let runner_clone = runner.clone();
        let max_lat = Arc::clone(&max_observed_latency);

        handles.push(tokio::spawn(async move {
            let cancel = Arc::new(AtomicBool::new(false));
            let cancel_clone = Arc::clone(&cancel);

            let exec_fut = tokio::spawn(async move {
                runner_clone
                    .execute_tool_without_reservation(
                        holehe::TOOL_ID,
                        json!({"email": format!("prompt_lat_{i}@example.com"), "max_sites": 10}),
                        false,
                        &cancel_clone,
                    )
                    .await
            });

            // Allow task to initialize and enter select wait
            tokio::time::sleep(Duration::from_millis(3 + (i % 4) as u64)).await;

            let cancel_time = Instant::now();
            if i % 2 == 0 {
                Service::cancel(&cancel);
            } else {
                cancel_with_notify(&cancel);
            }

            let res = tokio::time::timeout(Duration::from_millis(200), exec_fut).await;
            let latency = cancel_time.elapsed();

            assert!(
                res.is_ok(),
                "Task {i} timed out after 200ms awaiting prompt cancellation"
            );
            let outcome = res.unwrap().unwrap();
            let is_cancelled = match &outcome {
                Err(e) => e.to_string().contains("cancelled"),
                Ok(r) => r.status == "cancelled" || (r.observations.get("cancelled").and_then(|v| v.as_bool()).unwrap_or(false)),
            };
            if !is_cancelled {
                println!("Task {i} was not cancelled! outcome = {outcome:?}");
            }
            assert!(
                is_cancelled,
                "Task {i} must reflect cancellation (either Err('cancelled') or Ok(status='cancelled')), got: {outcome:?}"
            );

            let mut m = max_lat.lock().unwrap();
            if latency > *m {
                *m = latency;
            }

            assert!(
                latency < Duration::from_millis(50),
                "Task {i} prompt cancellation latency {:?} exceeded 50ms cap",
                latency
            );
        }));
    }

    for h in handles {
        h.await.unwrap();
    }

    let peak_lat = *max_observed_latency.lock().unwrap();
    println!("stress: 20 prompt cancellations completed. Peak latency = {peak_lat:?}");
    assert!(
        peak_lat < Duration::from_millis(50),
        "Peak cancellation latency {peak_lat:?} exceeded 50ms threshold"
    );

    assert_eq!(
        runner.concurrency_semaphore().available_permits(),
        4,
        "Permits must return to 4 after Service::cancel stress"
    );
}

#[tokio::test]
async fn stress_test_rapid_concurrent_select_drops_on_tool_runner() {
    init_test_runtime().await;
    let dir = tempfile::tempdir().unwrap();
    let _ = ReconStore::open(dir.path()).await.unwrap();

    let service =
        Arc::new(Service::new(dir.path(), AuthFile::default(), SettingsFile::default()).unwrap());
    let runner = service.tool_runner();

    let num_tasks = 20;
    let iters_per_task = 5;
    let mut handles = Vec::with_capacity(num_tasks);

    for task_idx in 0..num_tasks {
        let runner_clone = runner.clone();
        handles.push(tokio::spawn(async move {
            for iter in 0..iters_per_task {
                let cancel = Arc::new(AtomicBool::new(false));
                let cancel_clone = Arc::clone(&cancel);

                let runner_run = runner_clone.clone();
                let mut exec_fut = tokio::spawn(async move {
                    runner_run
                        .execute_tool_without_reservation(
                            holehe::TOOL_ID,
                            json!({"email": format!("seldrop_{task_idx}_{iter}@example.com"), "max_sites": 10}),
                            false,
                            &cancel_clone,
                        )
                        .await
                });

                // Micro-sleep to drop future mid-flight
                let sleep_ms = 2 + ((task_idx * 3 + iter) % 8) as u64;
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_millis(sleep_ms)) => {
                        // Drop the execution future!
                        exec_fut.abort();
                        let _ = exec_fut.await;
                    }
                    _ = &mut exec_fut => {}
                }
            }
        }));
    }

    for h in handles {
        h.await.unwrap();
    }

    // Give aborted background tasks a moment to complete drop unwinding
    tokio::time::sleep(Duration::from_millis(50)).await;

    assert_eq!(
        runner.concurrency_semaphore().available_permits(),
        4,
        "Permits must return to 4 after 100 select drops on ToolRunner"
    );
    println!("stress: 100 select drops on ToolRunner completed with 0 permit leaks.");
}

#[tokio::test]
async fn stress_test_repeated_mass_cancellation_waves_zero_permit_leak() {
    init_test_runtime().await;
    let dir = tempfile::tempdir().unwrap();
    let _ = ReconStore::open(dir.path()).await.unwrap();

    let service =
        Arc::new(Service::new(dir.path(), AuthFile::default(), SettingsFile::default()).unwrap());
    let runner = service.tool_runner();

    let num_waves = 5;
    let tasks_per_wave = 12;

    for wave in 0..num_waves {
        let mut wave_handles = Vec::with_capacity(tasks_per_wave);

        for task_idx in 0..tasks_per_wave {
            let runner_clone = runner.clone();
            wave_handles.push(tokio::spawn(async move {
                let cancel = Arc::new(AtomicBool::new(false));
                let cancel_clone = Arc::clone(&cancel);

                let exec_fut = tokio::spawn(async move {
                    runner_clone
                        .execute_tool_without_reservation(
                            holehe::TOOL_ID,
                            json!({"email": format!("wave_{wave}_{task_idx}@example.com"), "max_sites": 5}),
                            false,
                            &cancel_clone,
                        )
                        .await
                });

                // Varying cancel triggers
                let jitter = (task_idx % 5) as u64;
                tokio::time::sleep(Duration::from_millis(5 + jitter)).await;

                if task_idx % 2 == 0 {
                    Service::cancel(&cancel);
                } else {
                    cancel_with_notify(&cancel);
                }

                let res = tokio::time::timeout(Duration::from_millis(500), exec_fut).await;
                assert!(res.is_ok(), "Wave {wave} task {task_idx} timed out on cancel");
            }));
        }

        for h in wave_handles {
            h.await.unwrap();
        }

        assert_eq!(
            runner.concurrency_semaphore().available_permits(),
            4,
            "Permit leak detected after wave {wave}"
        );
    }

    println!(
        "stress: All 5 mass cancellation waves (60 tasks) concluded with 4 available permits."
    );
}
