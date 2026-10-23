//! Jobs dashboard read model and Retry on SurrealDB (`crate::jobs_view`
//! parity). Returns the same `JobRow` / `JobDetail` types as SQLite.
//!
//! The SurrealDB `job` / `task` / `attempt` records carry less than
//! `argos_jobs`: fields with no source (app, title, provider, model, tool,
//! run/resource/result refs, error text, durable timing) are `""` / `None`
//! ("Unavailable"), never zero-filled. Visibility follows
//! `job_cleanup::eligible`: created inside the 72-hour execution window and
//! not retired.

use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use surrealdb::engine::any::Any;
use surrealdb::types::SurrealValue;
use surrealdb::Surreal;

use super::jobs::{bump, rid, run, take, ts, EXECUTION_WINDOW_HOURS};
use super::tx::{run_tx, RetryPolicy, TxError};
use crate::jobs_view::{task_label, AttemptRow, JobCounts, JobDetail, JobFilter, JobRow, TaskRow};

/// Visible jobs (alias-free; `$cutoff` bound by the caller).
const ELIGIBLE: &str = "created_at >= <datetime>$cutoff \
    AND !record::exists(type::record('job_retirement', record::id(id)))";

const ROW: &str = "SELECT record::id(id) AS key,
    IF parent != NONE { record::id(parent) } ELSE { '' } AS parent_id,
    kind, operation, state, phase, cancel_requested,
    <string>created_at AS created_at,
    IF finished_at != NONE { <string>finished_at } ELSE { '' } AS finished_at,
    updated_at,
    IF state IN ['running', 'queued', 'retry_scheduled'] AND kind != 'service' { 0 }
      ELSE IF kind = 'service' { 2 } ELSE { 1 } AS rank,
    count((SELECT VALUE id FROM job WHERE parent = $parent.id)) AS children,
    count((SELECT VALUE id FROM task WHERE job = $parent.id)) AS tasks,
    count((SELECT VALUE id FROM attempt WHERE task.job = $parent.id)) AS attempt_rows,
    (SELECT VALUE <string>started_at FROM attempt
       WHERE task.job = $parent.id AND finished_at = NONE ORDER BY started_at LIMIT 1)[0]
      ?? '' AS open_attempt_started,
    (SELECT VALUE <string>started_at FROM task
       WHERE job = $parent.id AND started_at != NONE ORDER BY started_at LIMIT 1)[0]
      ?? '' AS started_at
  FROM job";

#[derive(SurrealValue)]
struct Raw {
    key: String,
    parent_id: String,
    kind: String,
    operation: String,
    state: String,
    phase: String,
    cancel_requested: bool,
    created_at: String,
    finished_at: String,
    children: i64,
    tasks: i64,
    attempt_rows: i64,
    open_attempt_started: String,
    started_at: String,
}

fn row(r: Raw) -> JobRow {
    JobRow {
        cancellable: r.kind != "service" && r.tasks > 0,
        id: r.key,
        parent_id: r.parent_id,
        kind: r.kind,
        operation: r.operation,
        state: r.state,
        phase: r.phase,
        queued_at: r.created_at.clone(),
        created_at: r.created_at,
        started_at: r.started_at,
        finished_at: r.finished_at,
        children: r.children,
        attempt_rows: r.attempt_rows,
        open_attempt_started: r.open_attempt_started,
        cancel_requested: r.cancel_requested,
        ..JobRow::default()
    }
}

fn cutoff(now: DateTime<Utc>) -> String {
    ts(now - Duration::hours(EXECUTION_WINDOW_HOURS))
}

fn retryable_ops() -> Vec<String> {
    crate::tasks::INDEX_OPERATIONS
        .iter()
        .chain(crate::tasks::SUMMARY_OPERATIONS.iter())
        .map(|s| s.to_string())
        .collect()
}

/// `list_jobs`: top-level visible jobs, active first, then most recently
/// updated, services last. Bounded by `limit`. `app` filters match nothing
/// but `""` (SurrealDB jobs carry no app yet).
pub async fn list_jobs(db: &Surreal<Any>, filter: &JobFilter, limit: usize) -> Result<Vec<JobRow>> {
    if !filter.app.is_empty() {
        return Ok(Vec::new());
    }
    let mut sql = format!("{ROW} WHERE parent = NONE AND {ELIGIBLE}");
    let states: Vec<String> = filter
        .status
        .states()
        .iter()
        .map(|s| s.to_string())
        .collect();
    if !states.is_empty() {
        sql.push_str(" AND state IN $states");
    }
    if !filter.operation.is_empty() {
        sql.push_str(" AND operation = $op");
    }
    let text = filter.text.trim().to_lowercase();
    if !text.is_empty() {
        sql.push_str(
            " AND (string::contains(string::lowercase(operation), $text) \
             OR string::contains(string::lowercase(<string>record::id(id)), $text))",
        );
    }
    if !filter.since.is_empty() {
        sql.push_str(" AND created_at >= <datetime>$since");
    }
    sql.push_str(" ORDER BY rank, updated_at DESC, key LIMIT $n");
    let rows: Vec<Raw> = db
        .query(sql)
        .bind(("cutoff", cutoff(Utc::now())))
        .bind(("states", states))
        .bind(("op", filter.operation.clone()))
        .bind(("text", text))
        .bind(("since", filter.since.clone()))
        .bind(("n", limit.max(1) as i64))
        .await?
        .check()?
        .take(0)?;
    Ok(rows.into_iter().map(row).collect())
}

/// `get_job`: one visible job by id, at any depth.
pub async fn get_job(db: &Surreal<Any>, id: &str) -> Result<Option<JobRow>> {
    let rows: Vec<Raw> = db
        .query(format!("{ROW} WHERE id = $j AND {ELIGIBLE}"))
        .bind(("j", rid("job", id)))
        .bind(("cutoff", cutoff(Utc::now())))
        .await?
        .check()?
        .take(0)?;
    Ok(rows.into_iter().next().map(row))
}

#[derive(SurrealValue)]
struct StateCount {
    state: String,
    recent: bool,
}

/// `job_counts`: header counts over visible top-level non-service jobs.
pub async fn job_counts(db: &Surreal<Any>) -> Result<JobCounts> {
    let now = Utc::now();
    let rows: Vec<StateCount> = db
        .query(format!(
            "SELECT state, (finished_at ?? d'1970-01-01T00:00:00Z') >= <datetime>$day AS recent \
             FROM job WHERE parent = NONE AND kind != 'service' AND {ELIGIBLE}"
        ))
        .bind(("cutoff", cutoff(now)))
        .bind(("day", ts(now - Duration::hours(24))))
        .await?
        .check()?
        .take(0)?;
    let mut c = JobCounts::default();
    for r in rows {
        match r.state.as_str() {
            "running" => c.active += 1,
            "queued" => c.queued += 1,
            "retry_scheduled" => c.retrying += 1,
            "failed" | "partial" => c.failed += 1,
            "completed" if r.recent => c.completed_recent += 1,
            _ => {}
        }
    }
    Ok(c)
}

/// `job_apps`: SurrealDB jobs carry no app yet, so the filter list is empty.
pub async fn job_apps(_db: &Surreal<Any>) -> Result<Vec<String>> {
    Ok(Vec::new())
}

#[derive(SurrealValue)]
struct Head {
    root: String,
    cancel_requested: bool,
}

#[derive(SurrealValue)]
struct RawTask {
    id: String,
    operation: String,
    pool: String,
    state: String,
    attempts: i64,
    next_eligible_at: String,
    updated_at: String,
}

#[derive(SurrealValue)]
struct RawAttempt {
    id: String,
    task_id: String,
    number: i64,
    owner: String,
    started_at: String,
    finished_at: String,
    outcome: String,
    operation: String,
}

fn ms_between(a: &str, b: &str) -> Option<i64> {
    let a = DateTime::parse_from_rfc3339(a).ok()?;
    let b = DateTime::parse_from_rfc3339(b).ok()?;
    Some((b - a).num_milliseconds().max(0))
}

/// `job_detail`: children (50), the bounded task list (open and failed
/// first), the 20 newest attempts and the retryable count. Brief stage
/// labels, wire requests, warnings and insight progress stay empty (their
/// tables are not on SurrealDB).
pub async fn job_detail(
    db: &Surreal<Any>,
    id: &str,
    task_limit: usize,
) -> Result<Option<JobDetail>> {
    let Some(job) = get_job(db, id).await? else {
        return Ok(None);
    };
    let mut r = db
        .query(format!(
            "{ROW} WHERE parent = $j ORDER BY created_at, id LIMIT 50;
             SELECT record::id(id) AS id, operation, pool, state, attempts,
               <string>next_eligible_at AS next_eligible_at, <string>updated_at AS updated_at,
               IF state IN ['running', 'queued', 'retry_scheduled', 'failed'] {{ 0 }} ELSE {{ 1 }} AS rank,
               updated_at AS u
             FROM task WHERE job = $j ORDER BY rank, u DESC, id LIMIT $n;
             RETURN count((SELECT VALUE id FROM task WHERE job = $j));
             SELECT record::id(id) AS id, record::id(task) AS task_id, number, owner,
               <string>started_at AS started_at,
               IF finished_at != NONE {{ <string>finished_at }} ELSE {{ '' }} AS finished_at,
               outcome, task.operation AS operation, started_at AS s
             FROM attempt WHERE task.job = $j ORDER BY s DESC, id LIMIT 20;
             RETURN count((SELECT VALUE id FROM task WHERE job = $j AND state = 'failed'
               AND operation IN $ops));"
        ))
        .bind(("j", rid("job", id)))
        .bind(("n", task_limit.max(1) as i64))
        .bind(("ops", retryable_ops()))
        .await?
        .check()?;
    let children: Vec<Raw> = r.take(0)?;
    let tasks: Vec<RawTask> = r.take(1)?;
    let task_total: Option<i64> = r.take(2)?;
    let attempts: Vec<RawAttempt> = r.take(3)?;
    let retryable: Option<i64> = r.take(4)?;
    Ok(Some(JobDetail {
        job,
        children: children.into_iter().map(row).collect(),
        tasks: tasks
            .into_iter()
            .map(|t| TaskRow {
                id: t.id,
                operation: t.operation,
                pool: t.pool,
                state: t.state,
                attempts: t.attempts,
                next_eligible_at: t.next_eligible_at,
                updated_at: t.updated_at,
                ..TaskRow::default()
            })
            .collect(),
        task_total: task_total.unwrap_or(0),
        attempts: attempts
            .into_iter()
            .map(|a| AttemptRow {
                duration_ms: ms_between(&a.started_at, &a.finished_at),
                task_label: task_label(&a.operation),
                id: a.id,
                task_id: a.task_id,
                attempt_number: a.number,
                owner: a.owner,
                started_at: a.started_at,
                finished_at: a.finished_at,
                outcome: a.outcome,
                ..AttemptRow::default()
            })
            .collect(),
        retryable: retryable.unwrap_or(0),
        ..JobDetail::default()
    }))
}

/// `retry_failed_tasks`: requeue a job's failed tasks that a durable pool
/// owns (index and summary work), under the tree's `job_guard`, and reopen a
/// failed/partial job. Completed tasks are untouched. A job with a cancel
/// request, a retired or missing job retries nothing.
pub async fn retry_failed_tasks(db: &Surreal<Any>, id: &str) -> Result<usize> {
    if get_job(db, id).await?.is_none() {
        return Ok(0);
    }
    let now = ts(Utc::now());
    let id = id.to_string();
    let n = run_tx(db, RetryPolicy::default(), |tx| {
        let (id, now) = (id.clone(), now.clone());
        async move {
            let mut r = run(
                &tx,
                tx.query("SELECT record::id(root) AS root, cancel_requested FROM ONLY $j")
                    .bind(("j", rid("job", &id))),
            )
            .await?;
            let Some(head): Option<Head> = take(&mut r, 0)? else {
                return Ok(0usize);
            };
            if head.cancel_requested {
                return Ok(0);
            }
            let root = head.root;
            bump(&tx, "job_guard", &root).await?;
            let mut r = run(
                &tx,
                tx.query(
                    "UPDATE task SET state = 'queued', attempts = 0, next_eligible_at = <datetime>$now,
                       lease_owner = '', process_owner = '', lease_until = NONE, finished_at = NONE,
                       updated_at = <datetime>$now
                     WHERE job = $j AND state = 'failed' AND operation IN $ops RETURN VALUE id;",
                )
                .bind(("j", rid("job", &id)))
                .bind(("ops", retryable_ops()))
                .bind(("now", now.clone())),
            )
            .await?;
            let ids: Vec<surrealdb::types::RecordId> = take(&mut r, 0)?;
            if !ids.is_empty() {
                run(
                    &tx,
                    tx.query(
                        "UPDATE $j SET state = 'running', finished_at = NONE, updated_at = <datetime>$now
                         WHERE state IN ['failed', 'partial'] RETURN NONE",
                    )
                    .bind(("j", rid("job", &id)))
                    .bind(("now", now)),
                )
                .await?;
            }
            Ok::<usize, TxError>(ids.len())
        }
    })
    .await?
    .value;
    Ok(n)
}
