//! Jobs dashboard read model and Retry on SurrealDB (`crate::jobs_view`
//! parity). Returns the same `JobRow` / `JobDetail` types as SQLite.
//!
//! Dashboard metadata (app, title, provider, model, tool, refs, error text,
//! durable timing, attempt caps) lives on `job` / `task` / `attempt` and is
//! written with [`set_job_meta`]; unknown timing stays `None`
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
    kind, operation, state, phase, cancel_requested, cancellable AS cancellable_flag,
    app, title, provider, model, tool, run_ref, resource_ref, result_ref, error_category,
    error_summary, correlation_id, stage_coverage_json, active_ms, queue_ms, retry_wait_ms,
    attempts_used, attempt_cap, worker_owner, progress_done, progress_total,
    IF heartbeat_at != NONE { <string>heartbeat_at } ELSE { '' } AS heartbeat_at,
    IF active_since != NONE { <string>active_since } ELSE { '' } AS active_since,
    count((SELECT VALUE id FROM job_event WHERE job_id = record::id($parent.id)
      OR job_id IN (SELECT VALUE record::id(id) FROM job WHERE parent = $parent.id))) AS events,
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
    cancellable_flag: bool,
    app: String,
    title: String,
    provider: String,
    model: String,
    tool: String,
    run_ref: String,
    resource_ref: String,
    result_ref: String,
    error_category: String,
    error_summary: String,
    correlation_id: String,
    stage_coverage_json: String,
    active_ms: Option<i64>,
    queue_ms: Option<i64>,
    retry_wait_ms: Option<i64>,
    attempts_used: i64,
    attempt_cap: i64,
    worker_owner: String,
    progress_done: Option<i64>,
    progress_total: Option<i64>,
    heartbeat_at: String,
    active_since: String,
    events: i64,
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
        cancellable: r.kind != "service" && (r.cancellable_flag || r.tasks > 0),
        app: r.app,
        title: r.title,
        provider: r.provider,
        model: r.model,
        tool: r.tool,
        run_ref: r.run_ref,
        resource_ref: r.resource_ref,
        result_ref: r.result_ref,
        error_category: r.error_category,
        error_summary: r.error_summary,
        correlation_id: r.correlation_id,
        stage_coverage_json: r.stage_coverage_json,
        active_ms: r.active_ms,
        queue_ms: r.queue_ms,
        retry_wait_ms: r.retry_wait_ms,
        attempts_used: r.attempts_used,
        attempt_cap: r.attempt_cap,
        worker_owner: r.worker_owner,
        progress_done: r.progress_done,
        progress_total: r.progress_total,
        heartbeat_at: r.heartbeat_at,
        active_since: r.active_since,
        events: r.events,
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
/// updated, services last. Bounded by `limit`.
pub async fn list_jobs(db: &Surreal<Any>, filter: &JobFilter, limit: usize) -> Result<Vec<JobRow>> {
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
    if !filter.app.is_empty() {
        sql.push_str(" AND app = $app");
    }
    if !filter.operation.is_empty() {
        sql.push_str(" AND operation = $op");
    }
    let text = filter.text.trim().to_lowercase();
    if !text.is_empty() {
        sql.push_str(
            " AND (string::contains(string::lowercase(operation), $text) \
             OR string::contains(string::lowercase(title), $text) \
             OR string::contains(string::lowercase(error_summary), $text) \
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
        .bind(("app", filter.app.clone()))
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

/// `job_apps`: distinct apps of visible top-level jobs, at most 32.
pub async fn job_apps(db: &Surreal<Any>) -> Result<Vec<String>> {
    let apps: Vec<String> = db
        .query(format!(
            "SELECT VALUE app FROM job WHERE parent = NONE AND app != '' AND {ELIGIBLE}"
        ))
        .bind(("cutoff", cutoff(Utc::now())))
        .await?
        .check()?
        .take(0)?;
    let mut apps: Vec<String> = apps
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    apps.truncate(32);
    Ok(apps)
}

/// Dashboard metadata for a job (`argos_jobs` columns with no executor
/// role). `None` leaves a field unchanged.
#[derive(Clone, Debug, Default)]
pub struct JobMeta {
    pub app: Option<String>,
    pub title: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub tool: Option<String>,
    pub run_ref: Option<String>,
    pub resource_ref: Option<String>,
    pub result_ref: Option<String>,
    pub error_category: Option<String>,
    pub error_summary: Option<String>,
    pub correlation_id: Option<String>,
    pub worker_owner: Option<String>,
    pub progress_done: Option<i64>,
    pub progress_total: Option<i64>,
    pub active_ms: Option<i64>,
    pub queue_ms: Option<i64>,
    pub retry_wait_ms: Option<i64>,
    pub attempts_used: Option<i64>,
    pub attempt_cap: Option<i64>,
    pub cancellable: Option<bool>,
}

/// Write [`JobMeta`] onto `job:id` (merge; absent fields untouched).
pub async fn set_job_meta(db: &Surreal<Any>, id: &str, meta: &JobMeta) -> Result<()> {
    let mut patch = serde_json::Map::new();
    macro_rules! put {
        ($($f:ident),*) => {$(
            if let Some(v) = &meta.$f {
                patch.insert(stringify!($f).into(), serde_json::json!(v));
            }
        )*};
    }
    put!(
        app,
        title,
        provider,
        model,
        tool,
        run_ref,
        resource_ref,
        result_ref,
        error_category,
        error_summary,
        correlation_id,
        active_ms,
        queue_ms,
        retry_wait_ms,
        attempts_used,
        attempt_cap,
        cancellable
    );
    db.query("UPDATE $j MERGE $patch RETURN NONE")
        .bind(("j", rid("job", id)))
        .bind(("patch", serde_json::Value::Object(patch)))
        .await?
        .check()?;
    Ok(())
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
    max_attempts: i64,
    active_ms: i64,
    queue_ms: Option<i64>,
    retry_wait_ms: i64,
    error_category: String,
    error_message: String,
    result_ref: String,
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
    duration_ms: Option<i64>,
    error_category: String,
    error_message: String,
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
             SELECT record::id(id) AS id, operation, pool, state, attempts, max_attempts, active_ms,
               queue_ms, retry_wait_ms, error_category, error_message, result_ref,
               <string>next_eligible_at AS next_eligible_at, <string>updated_at AS updated_at,
               IF state IN ['running', 'queued', 'retry_scheduled', 'failed'] {{ 0 }} ELSE {{ 1 }} AS rank,
               updated_at AS u
             FROM task WHERE job = $j ORDER BY rank, u DESC, id LIMIT $n;
             RETURN count((SELECT VALUE id FROM task WHERE job = $j));
             SELECT record::id(id) AS id, record::id(task) AS task_id, number, owner,
               <string>started_at AS started_at,
               IF finished_at != NONE {{ <string>finished_at }} ELSE {{ '' }} AS finished_at,
               outcome, duration_ms, error_category, error_message,
               task.operation AS operation, started_at AS s
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
                max_attempts: t.max_attempts,
                active_ms: t.active_ms,
                queue_ms: t.queue_ms,
                retry_wait_ms: t.retry_wait_ms,
                error_category: t.error_category,
                error_message: t.error_message,
                result_ref: t.result_ref,
                next_eligible_at: t.next_eligible_at,
                updated_at: t.updated_at,
                stage_label: String::new(),
            })
            .collect(),
        task_total: task_total.unwrap_or(0),
        attempts: attempts
            .into_iter()
            .map(|a| AttemptRow {
                duration_ms: a
                    .duration_ms
                    .or_else(|| ms_between(&a.started_at, &a.finished_at)),
                error_category: a.error_category,
                error_message: a.error_message,
                task_label: task_label(&a.operation),
                id: a.id,
                task_id: a.task_id,
                attempt_number: a.number,
                owner: a.owner,
                started_at: a.started_at,
                finished_at: a.finished_at,
                outcome: a.outcome,
            })
            .collect(),
        retryable: retryable.unwrap_or(0),
        ..JobDetail::default()
    }))
}

/// `retry_failed_tasks`: requeue a job's failed tasks that a durable pool
/// owns (index and summary work), under the tree's `job_guard`, and reopen a
/// failed/partial job, resets the requeued tasks' failed index changes and
/// records a `job.retry` event in the same transaction. Brief-chain and
/// insight-extraction retries wait for those pipelines. Completed tasks are untouched. A job with a cancel
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
                       error_category = '', error_message = '',
                       updated_at = <datetime>$now
                     WHERE job = $j AND state = 'failed' AND operation IN $ops RETURN VALUE record::id(id);",
                )
                .bind(("j", rid("job", &id)))
                .bind(("ops", retryable_ops()))
                .bind(("now", now.clone())),
            )
            .await?;
            let ids: Vec<String> = take(&mut r, 0)?;
            if !ids.is_empty() {
                run(
                    &tx,
                    tx.query(
                        "UPDATE $j SET state = 'running', finished_at = NONE, updated_at = <datetime>$now
                         WHERE state IN ['failed', 'partial'] RETURN NONE;
                         UPDATE index_change SET state = 'pending', error_category = '',
                           error_message = '', updated_at = <datetime>$now
                         WHERE state = 'failed' AND task_id IN $tasks RETURN NONE;
                         CREATE job_event SET job_id = $key, app = 'jobs', event_type = 'job.retry',
                           severity = 'info', message = $msg, ts = <datetime>$now RETURN NONE;",
                    )
                    .bind(("j", rid("job", &id)))
                    .bind(("key", id.clone()))
                    .bind(("tasks", ids.clone()))
                    .bind((
                        "msg",
                        format!("Retry requested for {} failed task(s)", ids.len()),
                    ))
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
