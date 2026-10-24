//! Telemetry and event-log writes on SurrealDB (`storage::telemetry`,
//! `storage::event_log`): the same writes give the same rows as SQLite.
//! Run through `python3 scripts/test_surreal.py --mode both`.
#![cfg(feature = "surreal")]

mod support;

use argos_osint_core::events::{EventFilter, NewEvent, Severity};
use argos_osint_core::investigation::{InvestigationEvent, InvestigationSurface};
use argos_osint_core::profile_stats::RollupRow;
use argos_osint_core::storage::{event_log, profile_view, telemetry as surreal};
use argos_osint_core::store::Store;
use argos_osint_core::telemetry::{self, EventKind, TelemetryEvent, Trigger};
use serde_json::json;
use support::*;

fn events() -> Vec<TelemetryEvent> {
    let at = "2026-10-23T10:15:00+00:00";
    vec![
        TelemetryEvent::new(EventKind::ReconStage)
            .with_id("run-1:plan:1")
            .canonical("run-1:plan")
            .at(at)
            .app("recon")
            .trigger(Trigger::ReconPrompt)
            .category("plan")
            .mode("deep")
            .outcome("completed")
            .run("run-1")
            .duration_ms(Some(1_200))
            .payload(json!({"stage": "plan", "attempt": 1})),
        TelemetryEvent::new(EventKind::ReconStage)
            .with_id("run-1:plan:2")
            .canonical("run-1:plan")
            .at("2026-10-23T10:40:00+00:00")
            .app("recon")
            .trigger(Trigger::ReconPrompt)
            .category("plan")
            .mode("deep")
            .outcome("completed")
            .run("run-1")
            .duration_ms(Some(300)),
        TelemetryEvent::new(EventKind::DirectiveAssessed)
            .with_id("run-1:d1")
            .at(at)
            .app("recon")
            .mode("deep")
            .outcome("answered")
            .run("run-1")
            .turn("turn-1"),
        TelemetryEvent::new(EventKind::ReconRun)
            .with_id("run-1")
            .at("2026-10-24T01:00:00+00:00")
            .app("recon")
            .outcome("completed_with_evidence")
            .run("run-1")
            .thread("thread-1")
            .payload(json!({"directives": 1})),
        // A replay of the run row: rewritten, never counted twice.
        TelemetryEvent::new(EventKind::ReconRun)
            .with_id("run-1")
            .at("2026-10-24T01:00:00+00:00")
            .app("recon")
            .outcome("partial")
            .run("run-1")
            .thread("thread-1"),
        // No id: a fresh row every time.
        TelemetryEvent::new(EventKind::ToolInvocation)
            .at(at)
            .app("recon")
            .tool("newsapi")
            .provider("newsapi")
            .outcome("ok")
            .duration_ms(Some(90_000)),
    ]
}

fn sqlite_rollups(conn: &rusqlite::Connection, table: &str) -> Vec<RollupRow> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT bucket, event_type, dim_key, events, total_ms, max_ms, bins_json FROM {table}"
        ))
        .unwrap();
    let mut rows: Vec<RollupRow> = stmt
        .query_map([], |r| {
            Ok(RollupRow {
                bucket: r.get(0)?,
                event_type: r.get(1)?,
                dim_key: r.get(2)?,
                events: r.get(3)?,
                total_ms: r.get(4)?,
                max_ms: r.get(5)?,
                bins_json: r.get(6)?,
            })
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    rows.sort_by(|a, b| {
        (&a.bucket, &a.event_type, &a.dim_key).cmp(&(&b.bucket, &b.event_type, &b.dim_key))
    });
    rows
}

fn sorted(mut rows: Vec<RollupRow>) -> Vec<RollupRow> {
    rows.sort_by(|a, b| {
        (&a.bucket, &a.event_type, &a.dim_key).cmp(&(&b.bucket, &b.event_type, &b.dim_key))
    });
    rows
}

#[tokio::test]
async fn telemetry_events_and_rollups_match_sqlite() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = server.root("argos", "telemetry").await;
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("argos.db")).unwrap();
    for event in events() {
        telemetry::insert(store.connection(), &event).unwrap();
        surreal::record(&db, &event).await.unwrap();
    }

    let tables = profile_view::load_tables(&db).await.unwrap();
    assert_eq!(
        sorted(tables.telemetry_hourly.clone().unwrap()),
        sqlite_rollups(store.connection(), "telemetry_hourly")
    );
    assert_eq!(
        sorted(tables.telemetry_daily.clone().unwrap()),
        sqlite_rollups(store.connection(), "telemetry_daily")
    );
    let sqlite_count: i64 = store
        .connection()
        .query_row("SELECT COUNT(*) FROM telemetry_events", [], |r| r.get(0))
        .unwrap();
    assert_eq!(tables.telemetry_events.unwrap().len() as i64, sqlite_count);
    assert_eq!(sqlite_count, 5);

    // The replayed run row carries the last write, with its thread link.
    let runs = surreal::events_of_type(&db, "recon_run").await.unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].outcome, "partial");
    assert_eq!(runs[0].thread_id, "thread-1");
    let directives = surreal::events_of_type(&db, "directive_assessed")
        .await
        .unwrap();
    assert_eq!(directives[0].turn_id, "turn-1");

    // Stage attempts count the rows already written for that run and stage.
    assert_eq!(
        surreal::stage_attempt(&db, "run-1", "plan").await.unwrap(),
        3
    );
    assert_eq!(
        surreal::stage_attempt(&db, "run-1", "search")
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        surreal::stage_attempt(&db, "run-2", "plan").await.unwrap(),
        1
    );
}

fn trace(thread: &str, sequence: i64, kind: &str, summary: &str) -> InvestigationEvent {
    let mut ev = InvestigationEvent::new(
        thread,
        sequence,
        InvestigationSurface::ReconChat,
        kind,
        summary,
    );
    ev.id = format!("ev:{thread}:{sequence}");
    ev.run_id = "run-1".into();
    ev.payload = json!({"n": sequence});
    ev.evidence_refs = vec![format!("ref-{sequence}")];
    ev
}

#[tokio::test]
async fn investigation_events_match_sqlite() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = server.root("argos", "investigation_events").await;
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("argos.db")).unwrap();

    assert_eq!(event_log::next_event_sequence(&db, "t1").await.unwrap(), 1);
    let first = trace("t1", 1, "task_started", "Started");
    let second = trace("t1", 2, "task_completed", "Completed");
    let other = trace("t2", 1, "task_started", "Other thread");
    for ev in [&second, &first, &other] {
        store.insert_investigation_event(ev).unwrap();
        event_log::insert_investigation_event(&db, ev)
            .await
            .unwrap();
    }
    // A rewrite of an existing id updates status, summary and payload only.
    let mut rewrite = trace("t1", 9, "ignored", "Completed again");
    rewrite.id = second.id.clone();
    rewrite.status = "superseded".into();
    store.insert_investigation_event(&rewrite).unwrap();
    event_log::insert_investigation_event(&db, &rewrite)
        .await
        .unwrap();

    let sqlite = store.list_investigation_events("t1").unwrap();
    let surreal = event_log::list_investigation_events(&db, "t1")
        .await
        .unwrap();
    assert_eq!(surreal, sqlite);
    assert_eq!(surreal.len(), 2);
    assert_eq!(surreal[1].sequence, 2);
    assert_eq!(surreal[1].status, "superseded");
    assert_eq!(surreal[1].event_type, "task_completed");
    assert_eq!(
        event_log::next_event_sequence(&db, "t1").await.unwrap(),
        store.next_event_sequence("t1").unwrap()
    );
}

#[tokio::test]
async fn event_log_records_redacted_ordered_events() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = server.root("argos", "event_log").await;
    let call = |severity, app: &str, message: &str| NewEvent {
        severity: Some(severity),
        app: app.into(),
        event_type: "session".into(),
        message: message.into(),
        details: "Authorization: Bearer abcdefghijklmnop".into(),
        job_id: "job-1".into(),
        ..NewEvent::default()
    };
    let a = event_log::record_event(&db, &call(Severity::Info, "recon", "tool ok"))
        .await
        .unwrap();
    let b = event_log::record_event(&db, &call(Severity::Error, "recon", "tool failed"))
        .await
        .unwrap();
    let c = event_log::record_event(&db, &call(Severity::Warn, "intel", "slow"))
        .await
        .unwrap();

    let all = event_log::list_events(&db, &EventFilter::default(), 10)
        .await
        .unwrap();
    assert_eq!(
        all.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
        [c.as_str(), b.as_str(), a.as_str()]
    );
    assert_eq!(all.iter().map(|e| e.seq).collect::<Vec<_>>(), [3, 2, 1]);
    assert!(all.iter().all(|e| !e.details.contains("abcdefghijklmnop")));

    let warn_up = EventFilter {
        min_severity: Some(Severity::Warn),
        ..EventFilter::default()
    };
    let rows = event_log::list_events(&db, &warn_up, 10).await.unwrap();
    assert_eq!(rows.len(), 2);
    let recon = EventFilter {
        app: "recon".into(),
        text: "failed".into(),
        ..EventFilter::default()
    };
    let rows = event_log::list_events(&db, &recon, 10).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].severity, "error");
    let page = EventFilter {
        before_seq: Some(3),
        job_id: "job-1".into(),
        ..EventFilter::default()
    };
    let rows = event_log::list_events(&db, &page, 1).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].seq, 2);
}
