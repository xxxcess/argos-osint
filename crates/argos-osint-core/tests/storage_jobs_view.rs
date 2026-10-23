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
    assert_eq!(detail.attempts[0].task_id, "t1");
    assert!(detail.attempts[0].duration_ms.is_some_and(|ms| ms >= 1000));

    assert_eq!(view::retry_failed_tasks(&db, "j1").await.unwrap(), 1);
    assert_eq!(view::retry_failed_tasks(&db, "j1").await.unwrap(), 0);
    let detail = view::job_detail(&db, "j1", 40).await.unwrap().unwrap();
    assert_eq!(detail.job.state, "running");
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
