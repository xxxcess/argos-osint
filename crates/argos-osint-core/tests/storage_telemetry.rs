//! Telemetry and event-log writes on SurrealDB (`storage::telemetry`,
//! `storage::event_log`): the same writes give the same rows as SQLite.
//! Run through `python3 scripts/test_surreal.py --mode both`.
#![cfg(feature = "surreal")]

mod support;

use argos_osint_core::events::{EventFilter, NewEvent, Severity};
use argos_osint_core::storage::event_log;
use support::*;

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
