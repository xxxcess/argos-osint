use super::*;
use std::time::Duration;

fn claimed(id: &str, started: &str) -> ClaimedTask {
    ClaimedTask {
        id: id.into(),
        job_id: "job-t".into(),
        operation: "intel_brief_claims".into(),
        owner: "w".into(),
        epoch: 1,
        attempt: 1,
        max_attempts: 1,
        input_ref: String::new(),
        input_hash: String::new(),
        source_revision: String::new(),
        attempt_id: String::new(),
        started_at: started.into(),
    }
}

#[test]
fn attempts_aggregate_into_admission_backoff_generation_and_routes() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("argos.db")).unwrap();
    let c = &store.conn;
    let started = "2026-10-13T10:00:00+00:00";
    c.execute(
        "INSERT INTO recon_model_operations(id,task_id,role,status,created_at,updated_at) VALUES ('op','task-agg','claim_assessor','final',?1,?1)",
        [started],
    )
    .unwrap();
    // A failed attempt (admission 100, 900 ms), backoff 2000, then success.
    for (i, (provider, outcome, queue, wait, elapsed, ttfb)) in [
        ("google", "fail", 100, 0, 900, 0),
        ("openrouter", "ok", 50, 2000, 1500, 300),
    ]
    .into_iter()
    .enumerate()
    {
        c.execute(
            "INSERT INTO recon_model_attempts(id,operation_id,route_index,attempt,provider,account,model,transport,dispatched,outcome,wait_ms,started_at,queue_ms,elapsed_ms,ttfb_ms) VALUES (?1,'op',?2,?3,?4,'','m','stream',1,?5,?6,?7,?8,?9,?10)",
            rusqlite::params![
                format!("a{i}"),
                i as i64,
                i as i64 + 1,
                provider,
                outcome,
                wait,
                format!("2026-10-13T10:00:0{}+00:00", i + 1),
                queue,
                elapsed,
                ttfb
            ],
        )
        .unwrap();
    }
    let clock = StageClock::default();
    note_on(&clock, Phase::Fidelity, Duration::from_millis(40));
    note_on(&clock, Phase::FirstOutput, Duration::from_millis(700));
    note_on(&clock, Phase::FirstOutput, Duration::from_millis(5));
    let t = collect(
        c,
        &claimed("task-agg", started),
        &clock,
        Duration::from_millis(5000),
        Duration::from_millis(7),
        "committed",
    )
    .unwrap();
    assert_eq!(t.admission_ms, 150);
    assert_eq!(t.backoff_ms, 2000);
    assert_eq!(t.generation_ms, 2400);
    assert_eq!(t.attempts, 2);
    assert_eq!(
        t.headers_ms, 300,
        "header activity of the successful attempt"
    );
    assert_eq!(t.first_output_ms, 700, "first meaningful output only");
    assert_eq!(t.fidelity_ms, 40);
    assert_eq!(t.persistence_ms, 7);
    assert_eq!(t.routes, ["google/m", "openrouter/m"]);
    persist(c, &t, started).unwrap();
    assert_eq!(for_job(c, "job-t").unwrap(), vec![t.clone()]);
    let line = summary(&t);
    assert!(
        line.contains("admission 150") && line.contains("backoff 2000"),
        "{line}"
    );
}

#[test]
fn labels_name_role_and_substep() {
    assert_eq!(role_label("claim_assessor"), "Claim Assessor");
    assert_eq!(role_label("evidence_curator"), "Evidence Curator");
    assert_eq!(substep_label("intel_brief_links"), "selecting links");
}

#[tokio::test]
async fn timed_futures_accumulate_on_their_own_clock_only() {
    let a = StageClock::default();
    let b = StageClock::default();
    let fa = with_clock(a.clone(), async {
        timed(Phase::Model, tokio::time::sleep(Duration::from_millis(30))).await;
    });
    let fb = with_clock(b.clone(), async {
        timed(
            Phase::Fidelity,
            tokio::time::sleep(Duration::from_millis(10)),
        )
        .await;
    });
    tokio::join!(fa, fb);
    assert!(peek(&a, Phase::Model) >= Duration::from_millis(30));
    assert_eq!(peek(&a, Phase::Fidelity), Duration::ZERO);
    assert_eq!(peek(&b, Phase::Model), Duration::ZERO);
    // Outside any clock, notes are dropped rather than misattributed.
    note(Phase::Model, Duration::from_secs(1));
    assert!(peek(&a, Phase::Model) < Duration::from_secs(1));
}

#[tokio::test]
async fn activity_sink_shows_substep_role_model_and_backoff() {
    let clock = StageClock::default();
    with_clock(clock.clone(), sink_scenario()).await;
    assert!(peek(&clock, Phase::FirstOutput) > Duration::ZERO);
}

async fn sink_scenario() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("argos.db");
    let store = Store::open(&path).unwrap();
    let sink = activity_sink(&path, "task-live", "intel_brief_claims", "claim_assessor");
    let label = || -> String {
        store
            .conn
            .query_row(
                "SELECT value FROM app_state WHERE key='intel-brief-activity:task-live'",
                [],
                |r| r.get(0),
            )
            .unwrap_or_default()
    };
    sink(ModelExecEvent::AttemptStart {
        operation_id: "o".into(),
        generation: 1,
        route_index: 0,
        attempt: 1,
        provider: "google".into(),
        account: String::new(),
        model: "gemini-x".into(),
        transport: "stream".into(),
    });
    assert_eq!(
        label(),
        "extracting claims · Claim Assessor · google/gemini-x · awaiting admission"
    );
    sink(ModelExecEvent::Progress {
        operation_id: "o".into(),
        attempt: 1,
        route_index: 0,
        model: "gemini-x".into(),
        progress: crate::provider_attempt::AttemptProgress::Dispatched,
    });
    assert!(label().ends_with("awaiting first output"));
    std::thread::sleep(Duration::from_millis(2));
    sink(ModelExecEvent::Progress {
        operation_id: "o".into(),
        attempt: 1,
        route_index: 0,
        model: "gemini-x".into(),
        progress: crate::provider_attempt::AttemptProgress::FirstModelOutput,
    });
    assert!(label().ends_with("google/gemini-x · generating"));
    sink(ModelExecEvent::RetryStatus {
        operation_id: "o".into(),
        wait: Duration::from_secs(4),
        next_attempt: 2,
        next_route: 1,
    });
    assert_eq!(
        label(),
        "extracting claims · Claim Assessor · backing off 4s"
    );
}

/// Migration 41: a retried task aggregates only its current lease epoch,
/// even when the stale epoch's attempts started after this claim.
#[test]
fn timing_aggregates_the_current_epoch_only() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("argos.db")).unwrap();
    let c = &store.conn;
    let started = "2026-10-13T10:00:00+00:00";
    for (epoch, elapsed, provider) in [(1_i64, 9000, "stale"), (2, 1200, "live")] {
        let op = format!("op-e{epoch}");
        c.execute(
            "INSERT INTO recon_model_operations(id,task_id,role,status,created_at,updated_at,generation,task_epoch) VALUES (?1,'task-ep','summarization','final',?2,?2,?3,?3)",
            rusqlite::params![op, started, epoch],
        )
        .unwrap();
        c.execute(
            "INSERT INTO recon_model_attempts(id,operation_id,generation,route_index,attempt,provider,account,model,transport,dispatched,outcome,started_at,elapsed_ms) VALUES (?1,?2,?3,0,1,?4,'','m','stream',1,'ok','2026-10-13T10:00:05+00:00',?5)",
            rusqlite::params![format!("a{epoch}"), op, epoch, provider, elapsed],
        )
        .unwrap();
    }
    let mut task = claimed("task-ep", started);
    task.epoch = 2;
    let t = collect(
        c,
        &task,
        &StageClock::default(),
        Duration::from_millis(2000),
        Duration::ZERO,
        "committed",
    )
    .unwrap();
    assert_eq!((t.attempts, t.generation_ms), (1, 1200));
    assert_eq!(t.routes, ["live/m"]);
    assert_eq!(t.task_epoch, Some(2));
    persist(c, &t, started).unwrap();
    let epoch: Option<i64> = c
        .query_row(
            "SELECT task_epoch FROM intel_brief_stage_timings WHERE task_id='task-ep'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        epoch,
        Some(2),
        "the timing row records the latest execution's epoch"
    );
}
