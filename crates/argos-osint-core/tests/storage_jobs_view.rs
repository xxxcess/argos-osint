//! Jobs dashboard read model and Retry on SurrealDB (`storage::jobs_view`).
//! Run through `python3 scripts/test_surreal.py --mode both`.
#![cfg(feature = "surreal")]

mod support;

use argos_osint_core::jobs_view::{JobFilter, JobStatusFilter};
use argos_osint_core::storage::jobs::{self, NewJob, NewTask};
use argos_osint_core::storage::jobs_view as view;
use chrono::{Duration, Utc};
use support::*;

fn job<'a>(id: &'a str, parent: Option<&'a str>, hours_ago: i64) -> NewJob<'a> {
    NewJob {
        id,
        kind: "task",
        operation: "summaries",
        parent,
        created_at: Utc::now() - Duration::hours(hours_ago),
    }
}

fn task<'a>(id: &'a str, job: &'a str, operation: &'a str) -> NewTask<'a> {
    NewTask {
        id,
        job,
        pool: "index",
        operation,
        priority: 100,
        deps: vec![],
        input_hash: "h",
        created_at: Utc::now(),
    }
}

#[tokio::test]
async fn running_stack_services_stay_visible_on_a_crowded_jobs_page() {
    use argos_osint_core::{
        events::{EventFilter, NewEvent},
        storage::event_log,
    };
    let Some(server) = Server::start() else {
        return;
    };
    let db = server.root("argos", "crowded_services").await;
    db.query(
        "FOR $i IN 0..300 {
        LET $id = type::record('job', string::concat('work-', <string>$i));
        CREATE $id SET root = $id, kind = 'task', operation = 'summaries',
          state = IF $i < 150 { 'running' } ELSE { 'completed' },
          app = 'intel', title = string::concat('Work ', <string>$i),
          created_at = time::now(), updated_at = time::now();
      };
      CREATE job:api SET root = job:api, kind = 'service', operation = 'api_server',
        state = 'running', app = 'system', title = 'Rust API server',
        created_at = time::now() - 1h, updated_at = time::now() - 1h;
      CREATE job:web SET root = job:web, kind = 'service', operation = 'web_server',
        state = 'running', app = 'system', title = 'Next.js web server',
        created_at = time::now() - 2h, updated_at = time::now() - 2h;
      CREATE job:other SET root = job:other, kind = 'service', operation = 'index_service',
        state = 'running', app = 'system', title = 'Other service',
        created_at = time::now(), updated_at = time::now();",
    )
    .await
    .unwrap()
    .check()
    .unwrap();
    for (job_id, message) in [
        ("api", "GET /api/v1/days · 200 · 1 ms"),
        ("web", "GET / 200 in 4ms"),
    ] {
        event_log::record_event(
            &db,
            &NewEvent {
                job_id: job_id.into(),
                app: "system".into(),
                event_type: "service.request".into(),
                message: message.into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    }
    for limit in [2, 100, 200] {
        let started = std::time::Instant::now();
        let rows = tokio::time::timeout(
            std::time::Duration::from_secs(20),
            view::list_jobs(&db, &JobFilter::default(), limit),
        )
        .await
        .expect("bounded grouped Jobs page exceeded 20s")
        .unwrap();
        eprintln!("crowded Jobs page limit={limit}: {:?}", started.elapsed());
        assert_eq!(rows.len(), limit);
        assert_eq!((&*rows[0].id, &*rows[1].id), ("api", "web"));
        for service in &rows[..2] {
            assert!(service.is_service() && !service.cancellable);
            assert_eq!(service.events, 1);
            let logs = event_log::list_events(
                &db,
                &EventFilter {
                    job_id: service.id.clone(),
                    ..Default::default()
                },
                10,
            )
            .await
            .unwrap();
            assert_eq!(logs.len(), 1);
            assert_eq!(logs[0].job_id, service.id);
        }
    }
    let intel = JobFilter {
        app: "intel".into(),
        ..Default::default()
    };
    assert_eq!(
        view::list_jobs(&db, &intel, 1).await.unwrap()[0].app,
        "intel"
    );
    let web = JobFilter {
        text: "next.js".into(),
        ..Default::default()
    };
    assert_eq!(view::list_jobs(&db, &web, 1).await.unwrap()[0].id, "web");
    let done = JobFilter {
        status: JobStatusFilter::Completed,
        ..Default::default()
    };
    assert_eq!(
        view::list_jobs(&db, &done, 1).await.unwrap()[0].state,
        "completed"
    );
    let missing = JobFilter {
        operation: "missing".into(),
        ..Default::default()
    };
    assert!(view::list_jobs(&db, &missing, 10).await.unwrap().is_empty());
}

#[tokio::test]
async fn grouped_page_metadata_matches_single_job_details() {
    use argos_osint_core::{events::NewEvent, storage::event_log};
    let Some(server) = Server::start() else {
        return;
    };
    let db = server.root("argos", "page_metrics").await;
    jobs::create_job(
        &db,
        &job("root", None, 1),
        &[task("first", "root", "a"), task("second", "root", "b")],
    )
    .await
    .unwrap();
    for (id, parent) in [
        ("child", Some("root")),
        ("grandchild", Some("child")),
        ("sibling", None),
    ] {
        jobs::create_job(&db, &job(id, parent, 1), &[])
            .await
            .unwrap();
    }
    db.query("UPDATE task:first SET started_at = time::now() - 10s;
        UPDATE task:second SET started_at = time::now() - 5s;
        CREATE attempt SET task = task:first, root = job:root, number = 1, epoch = 0, owner = 'worker',
          started_at = time::now() - 8s, finished_at = time::now(), outcome = 'completed';
        CREATE attempt SET task = task:second, root = job:root, number = 1, epoch = 0, owner = 'worker',
          started_at = time::now() - 4s;")
      .await.unwrap().check().unwrap();
    for job_id in ["root", "child", "grandchild", "sibling"] {
        event_log::record_event(
            &db,
            &NewEvent {
                job_id: job_id.into(),
                message: job_id.into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    }
    let rows = view::list_jobs(&db, &JobFilter::default(), 10)
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    for listed in &rows {
        assert_eq!(
            listed,
            &view::get_job(&db, &listed.id).await.unwrap().unwrap()
        );
    }
    let root = rows.iter().find(|r| r.id == "root").unwrap();
    assert_eq!((root.children, root.attempt_rows, root.events), (1, 2, 2));
    assert!(!root.started_at.is_empty() && !root.open_attempt_started.is_empty());
    assert!(root.cancellable);
}

#[tokio::test]
async fn jobs_view_lists_details_and_retries_like_sqlite() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = server.root("argos", "jobs_view").await;
    let index_op = argos_osint_core::tasks::INDEX_OPERATIONS[0];
    jobs::create_job(
        &db,
        &job("j1", None, 1),
        &[
            task("t1", "j1", index_op),
            task("t2", "j1", index_op),
            task("t3", "j1", "other"),
        ],
    )
    .await
    .unwrap();
    jobs::create_job(&db, &job("c1", Some("j1"), 1), &[])
        .await
        .unwrap();
    for (id, hours) in [("j2", 2), ("old", 100), ("gone", 1)] {
        jobs::create_job(&db, &job(id, None, hours), &[])
            .await
            .unwrap();
    }
    db.query(
        "UPDATE job:j1 SET state = 'failed', finished_at = time::now();
         UPDATE job:j2 SET state = 'completed', finished_at = time::now();
         UPDATE task:t1 SET state = 'failed'; UPDATE task:t2 SET state = 'completed';
         UPDATE task:t3 SET state = 'failed';
         CREATE job_retirement:gone SET retired_at = time::now(), reason = 'cleared';
         CREATE attempt SET task = task:t1, root = job:j1, number = 1, epoch = 0, owner = 'w0',
           started_at = time::now() - 2s, finished_at = time::now(), outcome = 'failed';",
    )
    .await
    .unwrap()
    .check()
    .unwrap();
    db.query(
        "UPDATE task:t1 SET error_category = 'timeout', error_message = 'slow', max_attempts = 5;
         CREATE index_change:c1 SET seq = 1, record_kind = 'memory', record_id = 'm1',
           operation = 'index_upsert', state = 'failed', task_id = 't1', error_message = 'x',
           created_at = time::now(), updated_at = time::now();
         CREATE index_change:c2 SET seq = 2, record_kind = 'memory', record_id = 'm2',
           operation = 'index_upsert', state = 'failed', task_id = 't3',
           created_at = time::now(), updated_at = time::now();",
    )
    .await
    .unwrap()
    .check()
    .unwrap();
    let meta = view::JobMeta {
        app: Some("atlas".into()),
        title: Some("Summaries for Rotterdam".into()),
        provider: Some("openai".into()),
        error_summary: Some("1 task failed".into()),
        active_ms: Some(1500),
        attempt_cap: Some(3),
        ..view::JobMeta::default()
    };
    view::set_job_meta(&db, "j1", &meta).await.unwrap();
    view::set_job_meta(
        &db,
        "j2",
        &view::JobMeta {
            app: Some("recon".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(view::job_apps(&db).await.unwrap(), vec!["atlas", "recon"]);
    let atlas = JobFilter {
        app: "atlas".into(),
        ..JobFilter::default()
    };
    let rows = view::list_jobs(&db, &atlas, 50).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        (rows[0].title.as_str(), rows[0].provider.as_str()),
        ("Summaries for Rotterdam", "openai")
    );
    assert_eq!(
        (rows[0].active_ms, rows[0].queue_ms, rows[0].attempt_cap),
        (Some(1500), None, 3)
    );
    let rotterdam = JobFilter {
        text: "rotterdam".into(),
        ..JobFilter::default()
    };
    assert_eq!(
        view::list_jobs(&db, &rotterdam, 50).await.unwrap()[0].id,
        "j1"
    );

    let all = view::list_jobs(&db, &JobFilter::default(), 50)
        .await
        .unwrap();
    let ids: Vec<&str> = all.iter().map(|j| j.id.as_str()).collect();
    assert_eq!(
        ids.len(),
        2,
        "{ids:?}: children, old and retired jobs are hidden"
    );
    assert!(ids.contains(&"j1") && ids.contains(&"j2"));
    let failed = JobFilter {
        status: JobStatusFilter::Failed,
        ..JobFilter::default()
    };
    let rows = view::list_jobs(&db, &failed, 50).await.unwrap();
    assert_eq!(rows.len(), 1);
    let j1 = &rows[0];
    assert_eq!((j1.children, j1.attempt_rows), (1, 1));
    assert!(j1.cancellable && j1.finished_at.ends_with('Z'), "{j1:?}");
    let text = JobFilter {
        text: "J2".into(),
        ..JobFilter::default()
    };
    assert_eq!(view::list_jobs(&db, &text, 50).await.unwrap()[0].id, "j2");
    assert!(view::get_job(&db, "old").await.unwrap().is_none());

    let counts = view::job_counts(&db).await.unwrap();
    assert_eq!(
        (counts.failed, counts.completed_recent, counts.active),
        (1, 1, 0)
    );

    let detail = view::job_detail(&db, "j1", 40).await.unwrap().unwrap();
    assert_eq!(detail.children[0].id, "c1");
    assert_eq!((detail.task_total, detail.retryable), (3, 1));
    assert_eq!(detail.tasks[0].state, "failed", "failed tasks sort first");
    let t1 = detail.tasks.iter().find(|t| t.id == "t1").unwrap();
    assert_eq!(
        (t1.error_category.as_str(), t1.max_attempts),
        ("timeout", 5)
    );
    assert_eq!(detail.job.events, 0);
    assert_eq!(detail.attempts[0].task_id, "t1");
    assert!(detail.attempts[0].duration_ms.is_some_and(|ms| ms >= 1000));

    assert_eq!(view::retry_failed_tasks(&db, "j1").await.unwrap(), 1);
    assert_eq!(view::retry_failed_tasks(&db, "j1").await.unwrap(), 0);
    let detail = view::job_detail(&db, "j1", 40).await.unwrap().unwrap();
    assert_eq!(detail.job.state, "running");
    assert_eq!(detail.job.events, 1, "job.retry recorded");
    let t1 = detail.tasks.iter().find(|t| t.id == "t1").unwrap();
    assert!(t1.error_message.is_empty());
    let mut r = db
        .query("SELECT VALUE state FROM index_change ORDER BY seq; SELECT VALUE event_type FROM job_event")
        .await
        .unwrap();
    let states: Vec<String> = r.take(0).unwrap();
    let events: Vec<String> = r.take(1).unwrap();
    assert_eq!(
        states,
        vec!["pending", "failed"],
        "only the requeued task's change resets"
    );
    assert_eq!(events, vec!["job.retry"]);
    let state = |id: &str| {
        detail
            .tasks
            .iter()
            .find(|t| t.id == id)
            .unwrap()
            .state
            .clone()
    };
    assert_eq!(
        (state("t1"), state("t2"), state("t3")),
        ("queued".into(), "completed".into(), "failed".into())
    );

    // A cancel request blocks Retry.
    db.query("UPDATE task:t1 SET state = 'failed'; UPDATE job:j1 SET cancel_requested = true")
        .await
        .unwrap()
        .check()
        .unwrap();
    assert_eq!(view::retry_failed_tasks(&db, "j1").await.unwrap(), 0);
    assert_eq!(view::retry_failed_tasks(&db, "missing").await.unwrap(), 0);
}

#[tokio::test]
async fn jobs_pane_stop_and_clear_history_on_the_repository() {
    use argos_osint_core::job_registry::CancelRequest;
    let Some(server) = Server::start() else {
        return;
    };
    let db = server.root("argos", "jobs_view_actions").await;
    jobs::create_job(&db, &job("run", None, 1), &[task("rt", "run", "op")])
        .await
        .unwrap();
    jobs::create_job(&db, &job("idle", None, 1), &[])
        .await
        .unwrap();
    jobs::create_job(&db, &job("done", None, 1), &[])
        .await
        .unwrap();
    db.query("UPDATE job:done SET state = 'completed', finished_at = time::now()")
        .await
        .unwrap()
        .check()
        .unwrap();
    assert_eq!(
        view::cancel_job(&db, "missing").await.unwrap(),
        CancelRequest::Missing
    );
    assert_eq!(
        view::cancel_job(&db, "idle").await.unwrap(),
        CancelRequest::NotCancellable
    );
    assert_eq!(
        view::cancel_job(&db, "run").await.unwrap(),
        CancelRequest::Requested
    );
    assert!(
        view::get_job(&db, "run")
            .await
            .unwrap()
            .unwrap()
            .cancel_requested
    );

    assert!(view::clear_jobs(&db, JobStatusFilter::All).await.is_err());
    assert_eq!(
        view::clear_jobs(&db, JobStatusFilter::Completed)
            .await
            .unwrap(),
        1
    );
    assert!(view::get_job(&db, "done").await.unwrap().is_none());
    assert!(view::get_job(&db, "idle").await.unwrap().is_some());

    // Multi-stage completed tree with a failed child, plus succeeded and paused
    // roots, plus a root whose child job is still running (must not be cleared).
    jobs::create_job(&db, &job("multi_done", None, 1), &[])
        .await
        .unwrap();
    jobs::create_job(&db, &job("multi_child_fail", Some("multi_done"), 1), &[])
        .await
        .unwrap();
    jobs::create_job(&db, &job("succeeded_root", None, 1), &[])
        .await
        .unwrap();
    jobs::create_job(&db, &job("paused_root", None, 1), &[])
        .await
        .unwrap();
    jobs::create_job(&db, &job("busy_root", None, 1), &[])
        .await
        .unwrap();
    jobs::create_job(&db, &job("busy_child", Some("busy_root"), 1), &[])
        .await
        .unwrap();
    db.query(
        "UPDATE job:multi_done SET state = 'completed', finished_at = time::now();
         UPDATE job:multi_child_fail SET state = 'failed', finished_at = time::now();
         UPDATE job:succeeded_root SET state = 'succeeded', finished_at = time::now();
         UPDATE job:paused_root SET state = 'paused', finished_at = time::now();
         UPDATE job:busy_root SET state = 'completed', finished_at = time::now();
         UPDATE job:busy_child SET state = 'running';
         CREATE job_event SET job_id = 'multi_done', app = 'atlas', event_type = 'job.completed', severity = 'info', message = 'done', ts = time::now();
         CREATE job_event SET job_id = 'multi_child_fail', app = 'atlas', event_type = 'job.failed', severity = 'error', message = 'child failed', ts = time::now();",
    )
    .await
    .unwrap()
    .check()
    .unwrap();

    // Completed filter clears both `completed` (even with a terminal `failed` child)
    // and `succeeded` roots, removes their `job_event` rows, and skips `busy_root`
    // because `busy_child` is still running.
    assert_eq!(
        view::clear_jobs(&db, JobStatusFilter::Completed)
            .await
            .unwrap(),
        2
    );
    assert!(view::get_job(&db, "multi_done").await.unwrap().is_none());
    assert!(view::get_job(&db, "multi_child_fail")
        .await
        .unwrap()
        .is_none());
    assert!(view::get_job(&db, "succeeded_root")
        .await
        .unwrap()
        .is_none());
    assert!(view::get_job(&db, "busy_root").await.unwrap().is_some());
    assert_eq!(count(&db, "job_event").await, 0);

    // Stopped filter clears `paused` (and `cancelled` / `superseded`) roots.
    assert_eq!(
        view::clear_jobs(&db, JobStatusFilter::Stopped)
            .await
            .unwrap(),
        2
    );
    assert!(view::get_job(&db, "paused_root").await.unwrap().is_none());
    assert!(view::get_job(&db, "run").await.unwrap().is_none());
}

#[tokio::test]
async fn jobs_record_dashboard_metadata_on_create_and_finish() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = server.root("argos", "jobs_view_write").await;
    let now = Utc::now();
    let at = |secs: i64| now + Duration::seconds(secs);
    let mut new = job("w1", None, 0);
    new.operation = "atlas_enrich";
    new.created_at = now;
    let mut t = task("wt", "w1", "op");
    t.pool = "p";
    t.created_at = now;
    jobs::create_job(&db, &new, &[t]).await.unwrap();
    let row = view::get_job(&db, "w1").await.unwrap().unwrap();
    assert_eq!(
        (row.app.as_str(), row.title.as_str()),
        ("atlas", "Atlas enrich")
    );
    assert!(row.cancellable);
    assert_eq!(
        (row.active_ms, row.queue_ms, row.attempts_used),
        (None, None, 0)
    );

    let claimed = jobs::claim_next(&db, "p", "w0", 30, at(2), "claim:wt")
        .await
        .unwrap()
        .value
        .expect("claimed");
    jobs::commit_stage(&db, &claimed, "{}", "h", at(5), "stage:wt")
        .await
        .unwrap();
    jobs::finalize_job(&db, "w1", None, "completed", at(6))
        .await
        .unwrap();
    let detail = view::job_detail(&db, "w1", 10).await.unwrap().unwrap();
    assert_eq!(detail.attempts[0].duration_ms, Some(3000));
    assert_eq!(detail.tasks[0].active_ms, 3000);
    let w1 = detail.job;
    assert_eq!((w1.attempts_used, w1.attempt_cap), (1, 3));
    assert_eq!((w1.active_ms, w1.queue_ms), (Some(3000), Some(2000)));
    assert!(w1.error_summary.is_empty());

    // A stopped job with a failed task records the failure summary.
    jobs::create_job(&db, &job("w2", None, 0), &[task("ft", "w2", "op")])
        .await
        .unwrap();
    db.query("UPDATE task:ft SET state = 'failed', error_category = 'timeout'")
        .await
        .unwrap()
        .check()
        .unwrap();
    assert_eq!(
        view::cancel_job(&db, "w2").await.unwrap(),
        argos_osint_core::job_registry::CancelRequest::Requested
    );
    let w2 = view::get_job(&db, "w2").await.unwrap().unwrap();
    assert_eq!(w2.state, "cancelled");
    assert_eq!(
        (w2.error_summary.as_str(), w2.error_category.as_str()),
        ("1 task(s) failed", "timeout")
    );
    assert_eq!(w2.queue_ms, None, "no task started: Unavailable");
}

#[tokio::test]
async fn retention_sweep_deletes_expired_trees_only() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = server.root("argos", "jobs_view_sweep").await;
    for i in 0..3 {
        let id = format!("old{i}");
        jobs::create_job(&db, &job(&id, None, 100), &[])
            .await
            .unwrap();
    }
    jobs::create_job(&db, &job("oldkid", Some("old0"), 100), &[])
        .await
        .unwrap();
    jobs::create_job(&db, &job("fresh", None, 1), &[])
        .await
        .unwrap();
    assert_eq!(view::sweep_expired(&db, Utc::now(), 64).await.unwrap(), 3);
    assert_eq!(view::sweep_expired(&db, Utc::now(), 64).await.unwrap(), 0);
    assert_eq!(count(&db, "job").await, 1);
    assert!(view::get_job(&db, "fresh").await.unwrap().is_some());
}
