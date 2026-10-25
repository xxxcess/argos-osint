//! The Logs pane and telemetry retention on SurrealDB
//! (`storage::event_log`, `storage::telemetry`) and the Report surface's
//! result lookup (`storage::intel::report_details_by_hash`): the
//! Logs pane counts and apps, the job filter walks child jobs,
//! prune and clear touch only `argos_event`, and the telemetry prune is
//! rate limited through `telemetry_meta`. Both modes via
//! `python3 scripts/test_surreal.py --mode both`.
#![cfg(feature = "surreal")]

mod support;

use argos_osint_core::events::{EventFilter, NewEvent, Severity};
use argos_osint_core::storage::bootstrap::ensure_schema;
use argos_osint_core::storage::intel::{self, Sequence};
use argos_osint_core::storage::{event_log, telemetry as surreal};
use argos_osint_core::telemetry::{EventKind, TelemetryEvent};
use support::*;

fn event(severity: Severity, app: &str, job: &str, message: &str) -> NewEvent {
    NewEvent {
        severity: Some(severity),
        app: app.into(),
        event_type: "session".into(),
        message: message.into(),
        job_id: job.into(),
        ..NewEvent::default()
    }
}

fn sample() -> Vec<NewEvent> {
    vec![
        event(Severity::Info, "recon", "job-root", "root started"),
        event(Severity::Error, "recon", "job-child", "child failed"),
        event(Severity::Warn, "intel", "job-grand", "grandchild slow"),
        event(Severity::Debug, "", "job-other", "other job"),
        event(Severity::Error, "atlas", "", "no job"),
    ]
}

#[tokio::test]
async fn logs_pane_counts_apps_and_walks_the_job_tree() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "logs_pane").await;
    ensure_schema(repo.db()).await.unwrap();
    let db = repo.db();
    // job-root -> job-child -> job-grand; job-other is unrelated.
    db.query(
        "CREATE job:⟨job-root⟩ SET operation = 'test', state = 'running', created_at = time::now(), updated_at = time::now(), root = job:⟨job-root⟩;
         CREATE job:⟨job-child⟩ SET operation = 'test', state = 'running',
           created_at = time::now(), updated_at = time::now(), parent = job:⟨job-root⟩, root = job:⟨job-root⟩;
         CREATE job:⟨job-grand⟩ SET operation = 'test', state = 'running',
           created_at = time::now(), updated_at = time::now(), parent = job:⟨job-child⟩, root = job:⟨job-root⟩;
         CREATE job:⟨job-other⟩ SET operation = 'test', state = 'running', created_at = time::now(), updated_at = time::now(), root = job:⟨job-other⟩;",
    )
    .await
    .unwrap()
    .check()
    .unwrap();

    let mut ids = Vec::new();
    for e in sample() {
        ids.push(event_log::record_event(db, &e).await.unwrap());
    }

    let counts = event_log::event_counts(db).await.unwrap();
    assert_eq!(
        (counts.error, counts.warn, counts.info, counts.debug),
        (2, 1, 1, 1)
    );
    assert_eq!(counts.recent_failures, 2);
    assert_eq!(
        event_log::event_apps(db).await.unwrap(),
        vec!["atlas".to_string(), "intel".into(), "recon".into()]
    );

    let tree = EventFilter {
        job_id: "job-root".into(),
        ..EventFilter::default()
    };
    let rows = event_log::list_events(db, &tree, 10).await.unwrap();
    let mut jobs: Vec<_> = rows.iter().map(|r| r.job_id.clone()).collect();
    jobs.sort();
    assert_eq!(jobs, ["job-child", "job-grand", "job-root"]);
    let child = EventFilter {
        job_id: "job-child".into(),
        ..EventFilter::default()
    };
    assert_eq!(
        event_log::list_events(db, &child, 10).await.unwrap().len(),
        2
    );
    // An unknown job matches only its own id.
    let gone = EventFilter {
        job_id: "job-missing".into(),
        ..EventFilter::default()
    };
    assert!(event_log::list_events(db, &gone, 10)
        .await
        .unwrap()
        .is_empty());

    let one = event_log::get_event(db, &ids[1]).await.unwrap().unwrap();
    assert_eq!(one.message, "child failed");
    assert_eq!(one.severity, "error");
    assert!(event_log::get_event(db, "missing").await.unwrap().is_none());
    assert!(event_log::get_event(db, "").await.unwrap().is_none());
}

#[tokio::test]
async fn prune_and_clear_touch_only_argos_event() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "logs_prune").await;
    ensure_schema(repo.db()).await.unwrap();
    let db = repo.db();
    let old = event_log::record_event(db, &event(Severity::Info, "recon", "", "old"))
        .await
        .unwrap();
    let fresh = event_log::record_event(db, &event(Severity::Info, "recon", "", "fresh"))
        .await
        .unwrap();
    let stale = (chrono::Utc::now() - chrono::Duration::hours(100)).to_rfc3339();
    db.query("UPDATE type::record('argos_event', $id) SET ts = $ts RETURN NONE")
        .bind(("id", old.clone()))
        .bind(("ts", stale))
        .await
        .unwrap()
        .check()
        .unwrap();
    db.query(
        "CREATE job_event SET job_id = 'j', app = 'jobs', event_type = 'job.retry',
           severity = 'info', message = 'kept', ts = time::now() RETURN NONE",
    )
    .await
    .unwrap()
    .check()
    .unwrap();

    assert_eq!(event_log::prune_events(db, 72).await.unwrap(), 1);
    assert!(event_log::get_event(db, &old).await.unwrap().is_none());
    assert!(event_log::get_event(db, &fresh).await.unwrap().is_some());
    assert_eq!(event_log::prune_events(db, 72).await.unwrap(), 0);

    assert_eq!(event_log::clear_events(db).await.unwrap(), 1);
    assert_eq!(count(db, "argos_event").await, 0);
    assert_eq!(count(db, "job_event").await, 1);
    assert_eq!(event_log::clear_events(db).await.unwrap(), 0);
    // Sequence numbering continues from the counter record after a clear:
    // old and fresh were 1 and 2, and `clear_events` only deletes
    // `argos_event` rows (it never resets the counter), so the next event
    // is 3. The old read-max allocator restarted at 1 here.
    event_log::record_event(db, &event(Severity::Warn, "recon", "", "after"))
        .await
        .unwrap();
    let rows = event_log::list_events(db, &EventFilter::default(), 5)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].seq, 3);
    assert_eq!(
        event_log::event_counts(db).await.unwrap().warn,
        1,
        "counts follow the cleared table"
    );
}

#[tokio::test]
async fn telemetry_meta_prune_and_coverage() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "telemetry_meta").await;
    ensure_schema(repo.db()).await.unwrap();
    let db = repo.db();
    // Until bootstrap registers it at merge.
    db.query(surreal::SCHEMA).await.unwrap().check().unwrap();

    assert!(surreal::get_meta(db, "observed_since")
        .await
        .unwrap()
        .is_none());
    let since = surreal::ensure_observed_since(db).await.unwrap();
    assert!(!since.is_empty());
    assert_eq!(surreal::ensure_observed_since(db).await.unwrap(), since);
    surreal::set_meta(db, "k", "v1").await.unwrap();
    surreal::set_meta(db, "k", "v2").await.unwrap();
    assert_eq!(
        surreal::get_meta(db, "k").await.unwrap().as_deref(),
        Some("v2")
    );
    assert!(surreal::get_meta(db, "").await.unwrap().is_none());

    let now = chrono::Utc::now();
    let ancient = (now - chrono::Duration::days(400)).to_rfc3339();
    let recent = (now - chrono::Duration::days(1)).to_rfc3339();
    for (id, at) in [("old", ancient.as_str()), ("new", recent.as_str())] {
        let e = TelemetryEvent::new(EventKind::ReconRun)
            .with_id(id)
            .at(at)
            .app("recon")
            .outcome("completed");
        surreal::record(db, &e).await.unwrap();
    }
    let before = surreal::coverage(db).await.unwrap();
    assert_eq!(before.raw_events, 2);
    assert_eq!(before.rollup_events, 2);
    assert_eq!(before.observed_since, since);

    assert!(surreal::prune(db, now).await.unwrap());
    let after = surreal::coverage(db).await.unwrap();
    assert_eq!(after.raw_events, 1);
    assert_eq!(after.rollup_events, 1);
    assert_eq!(count(db, "telemetry_daily").await, 1);
    assert!(surreal::get_meta(db, "raw_pruned_at")
        .await
        .unwrap()
        .is_some());
    // Rate limited: a second prune inside 15 minutes does nothing.
    assert!(!surreal::prune(db, now + chrono::Duration::minutes(5))
        .await
        .unwrap());
    assert!(surreal::prune(db, now + chrono::Duration::minutes(16))
        .await
        .unwrap());
}

#[tokio::test]
async fn report_details_are_found_by_source_hash() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "report_details").await;
    ensure_schema(repo.db()).await.unwrap();
    let db = repo.db();
    let details = r#"{"support_receipts":[]}"#;
    intel::record_report_result(
        db,
        "job-1",
        1,
        "article-1",
        details,
        "2026-10-24T00:00:00Z",
        Sequence::Next,
    )
    .await
    .unwrap();
    let hash = argos_osint_core::nlp::cache::hash(details);
    assert_eq!(
        intel::report_details_by_hash(db, &hash)
            .await
            .unwrap()
            .as_deref(),
        Some(details)
    );
    assert!(intel::report_details_by_hash(db, "nope")
        .await
        .unwrap()
        .is_none());
    assert!(intel::report_details_by_hash(db, "")
        .await
        .unwrap()
        .is_none());
}
