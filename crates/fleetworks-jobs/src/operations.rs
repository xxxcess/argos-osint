use std::collections::HashSet;

use anyhow::{bail, ensure, Result};
use rusqlite::Connection;
use rusqlite::OptionalExtension;
use serde_json::Value;

use crate::read;
use crate::{Command, Lease, NewJob, NewTask, Response};

pub(crate) fn execute(
    connection: &Connection,
    command: &Command,
    now: i64,
) -> Result<(Response, Option<String>)> {
    let (response, job_id) = match command {
        Command::Submit { job, tasks } => {
            submit(connection, job, tasks, now)?;
            (
                Response::Job(read::get_job(connection, &job.id)?),
                Some(job.id.clone()),
            )
        }
        Command::Claim {
            pool,
            owner,
            lease_ms,
            task_id,
        } => {
            ensure!(
                !pool.is_empty() && !owner.is_empty(),
                "claim requires pool and owner"
            );
            let until = lease_until(now, *lease_ms)?;
            let id: Option<String> = connection.query_row(
                "SELECT t.id FROM tasks t JOIN jobs j ON j.id=t.job_id
                 WHERE t.pool=?1 AND t.state='queued' AND t.next_eligible_at<=?2
                 AND t.attempts_used<t.attempt_cap AND j.cancel_requested=0
                 AND j.state IN ('queued','running') AND (?3 IS NULL OR t.id=?3)
                 AND NOT EXISTS(SELECT 1 FROM dependencies d JOIN tasks dep ON dep.id=d.dependency_id
                                WHERE d.task_id=t.id AND dep.state!='completed')
                 ORDER BY t.priority,t.created_at,t.id LIMIT 1",
                rusqlite::params![pool, now, task_id], |row| row.get(0),
            ).optional()?;
            if let Some(id) = id {
                connection.execute("UPDATE tasks SET state='running',owner=?2,epoch=epoch+1,lease_until=?3,attempts_used=attempts_used+1,updated_at=?4,error='' WHERE id=?1",
                    rusqlite::params![id, owner, until, now])?;
                connection.execute("INSERT INTO attempts(task_id,epoch,owner,state,started_at) SELECT id,epoch,owner,'running',?2 FROM tasks WHERE id=?1", rusqlite::params![id,now])?;
                let task =
                    read::get_task(connection, &id)?.expect("claimed task exists in transaction");
                connection.execute(
                    "UPDATE jobs SET state='running',updated_at=?2 WHERE id=?1",
                    rusqlite::params![task.job_id, now],
                )?;
                event(
                    connection,
                    Some(&task.job_id),
                    Some(&id),
                    "claimed",
                    &serde_json::json!({"owner":owner,"epoch":task.epoch}),
                    now,
                )?;
                let job_id = task.job_id.clone();
                (Response::Task(Some(task)), Some(job_id))
            } else {
                (Response::Task(None), None)
            }
        }
        Command::Renew { lease, lease_ms } => {
            let until = lease_until(now, *lease_ms)?;
            let id = valid_lease(connection, lease, now)?;
            if id.is_some() {
                connection.execute(
                    "UPDATE tasks SET lease_until=MAX(lease_until,?2),updated_at=?3 WHERE id=?1",
                    rusqlite::params![lease.task_id, until, now],
                )?;
            }
            (Response::Changed(id.is_some()), id)
        }
        Command::Complete { lease, output } => {
            let id = valid_lease(connection, lease, now)?;
            if let Some(job_id) = &id {
                connection.execute("UPDATE tasks SET state='completed',output=?2,owner=NULL,lease_until=NULL,updated_at=?3 WHERE id=?1",
                    rusqlite::params![lease.task_id,serde_json::to_string(output)?,now])?;
                finish_attempt(connection, lease, "completed", "", now)?;
                event(
                    connection,
                    Some(job_id),
                    Some(&lease.task_id),
                    "completed",
                    output,
                    now,
                )?;
                // A publication consumer must still obtain a domain-owned permit.
                outbox(
                    connection,
                    job_id,
                    &lease.task_id,
                    "publish",
                    &serde_json::json!({"lease":lease,"output":output}),
                    now,
                )?;
                settle(connection, job_id, now)?;
            }
            (Response::Changed(id.is_some()), id)
        }
        Command::Fail {
            lease,
            error,
            retry_at,
        } => {
            let id = valid_lease(connection, lease, now)?;
            if let Some(job_id) = &id {
                let task = read::get_task(connection, &lease.task_id)?.expect("leased task exists");
                let retry = retry_at.is_some() && task.attempts_used < task.attempt_cap;
                let state = if retry { "queued" } else { "failed" };
                let eligible = retry_at.unwrap_or(now).max(now);
                connection.execute("UPDATE tasks SET state=?2,error=?3,next_eligible_at=?4,owner=NULL,lease_until=NULL,updated_at=?5 WHERE id=?1",
                    rusqlite::params![lease.task_id,state,error,eligible,now])?;
                finish_attempt(connection, lease, "failed", error, now)?;
                event(
                    connection,
                    Some(job_id),
                    Some(&lease.task_id),
                    if retry { "retry_scheduled" } else { "failed" },
                    &serde_json::json!({"error":error,"retry_at":retry_at}),
                    now,
                )?;
                if retry {
                    dispatch(connection, job_id, &lease.task_id, now)?;
                }
                settle(connection, job_id, now)?;
            }
            (Response::Changed(id.is_some()), id)
        }
        Command::Cancel { job_id } => {
            let exists = read::get_job(connection, job_id)?.is_some();
            if exists {
                cancel(connection, job_id, now)?;
            }
            (Response::Changed(exists), exists.then(|| job_id.clone()))
        }
        Command::Retry { job_id } => {
            let job = read::get_job(connection, job_id)?;
            let count = if job.as_ref().is_some_and(|job| !job.cancel_requested) {
                let mut statement = connection
                    .prepare("SELECT id FROM tasks WHERE job_id=?1 AND state='failed'")?;
                let ids = statement
                    .query_map([job_id], |row| row.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                for id in &ids {
                    connection.execute("UPDATE tasks SET state='queued',attempt_cap=attempt_cap+1,next_eligible_at=?2,error='',updated_at=?2 WHERE id=?1", rusqlite::params![id,now])?;
                    dispatch(connection, job_id, id, now)?;
                }
                if !ids.is_empty() {
                    connection.execute("UPDATE jobs SET state='queued',finished_at=NULL,error='',updated_at=?2 WHERE id=?1", rusqlite::params![job_id,now])?;
                }
                ids.len()
            } else {
                0
            };
            (Response::Count(count), job.map(|job| job.id))
        }
        Command::Reap => (Response::Count(reap(connection, now)?), None),
        Command::Progress {
            job_id,
            state,
            done,
            total,
            error,
        } => {
            ensure!(
                matches!(
                    state.as_str(),
                    "queued" | "running" | "completed" | "failed" | "partial" | "cancelled"
                ),
                "invalid progress state"
            );
            ensure!(
                done.is_none_or(|n| n >= 0) && total.is_none_or(|n| n >= 0),
                "progress cannot be negative"
            );
            ensure!(
                done.zip(*total).is_none_or(|(done, total)| done <= total),
                "progress exceeds total"
            );
            let job = read::get_job(connection, job_id)?;
            let tasks: i64 = connection.query_row(
                "SELECT COUNT(*) FROM tasks WHERE job_id=?1",
                [job_id],
                |row| row.get(0),
            )?;
            ensure!(
                tasks == 0,
                "task-backed job state is derived from its tasks"
            );
            let changed = if job
                .as_ref()
                .is_some_and(|job| !job.cancel_requested && job.finished_at.is_none())
            {
                connection.execute("UPDATE jobs SET state=?2,progress_done=?3,progress_total=?4,error=?5,updated_at=?6,finished_at=CASE WHEN ?2 IN ('completed','failed','partial','cancelled') THEN ?6 ELSE NULL END WHERE id=?1",
                    rusqlite::params![job_id,state,done,total,error,now])? > 0
            } else {
                false
            };
            (Response::Changed(changed), job.map(|job| job.id))
        }
        Command::AppendEvent {
            job_id,
            task_id,
            kind,
            body,
        } => {
            ensure!(!kind.is_empty(), "event kind is required");
            if let Some(task_id) = task_id {
                let task = read::get_task(connection, task_id)?
                    .ok_or_else(|| anyhow::anyhow!("event task does not exist"))?;
                ensure!(
                    job_id.as_ref() == Some(&task.job_id),
                    "event task does not belong to job"
                );
            }
            (
                Response::Event(event(
                    connection,
                    job_id.as_deref(),
                    task_id.as_deref(),
                    kind,
                    body,
                    now,
                )?),
                job_id.clone(),
            )
        }
        Command::Acknowledge { seq } => {
            let job_id: Option<String> = connection
                .query_row("SELECT job_id FROM outbox WHERE seq=?1", [seq], |row| {
                    row.get(0)
                })
                .optional()?;
            let changed = connection.execute(
                "UPDATE outbox SET acknowledged_at=?2 WHERE seq=?1 AND acknowledged_at IS NULL",
                rusqlite::params![seq, now],
            )? > 0;
            (Response::Changed(changed), job_id)
        }
        Command::Prune { before } => {
            // Never remove active trees, pending handoffs, or their ancestors.
            let removed = connection.execute("DELETE FROM jobs WHERE finished_at<?1 AND state IN ('completed','failed','cancelled','partial')
                AND NOT EXISTS(SELECT 1 FROM jobs child WHERE child.parent_id=jobs.id)
                AND NOT EXISTS(SELECT 1 FROM outbox o WHERE o.job_id=jobs.id AND o.acknowledged_at IS NULL)", [before])?;
            (Response::Count(removed), None)
        }
        _ => bail!("not a Jobs mutation operation"),
    };
    Ok((response, job_id))
}

fn lease_until(now: i64, duration: i64) -> Result<i64> {
    ensure!(
        duration > 0 && duration <= 24 * 60 * 60 * 1000,
        "lease duration must be between 1ms and 24h"
    );
    now.checked_add(duration)
        .ok_or_else(|| anyhow::anyhow!("lease timestamp overflow"))
}

fn valid_lease(connection: &Connection, lease: &Lease, now: i64) -> Result<Option<String>> {
    Ok(connection.query_row("SELECT t.job_id FROM tasks t JOIN jobs j ON j.id=t.job_id WHERE t.id=?1 AND t.state='running' AND t.owner=?2 AND t.epoch=?3 AND t.lease_until>?4 AND j.cancel_requested=0",
        rusqlite::params![lease.task_id,lease.owner,lease.epoch,now],|row|row.get(0)).optional()?)
}

fn finish_attempt(
    connection: &Connection,
    lease: &Lease,
    state: &str,
    error: &str,
    now: i64,
) -> Result<()> {
    connection.execute("UPDATE attempts SET state=?3,error=?4,finished_at=?5 WHERE task_id=?1 AND epoch=?2 AND finished_at IS NULL",rusqlite::params![lease.task_id,lease.epoch,state,error,now])?;
    Ok(())
}

fn submit(connection: &Connection, job: &NewJob, tasks: &[NewTask], now: i64) -> Result<()> {
    ensure!(
        !job.id.is_empty() && !job.operation.is_empty(),
        "job id and operation are required"
    );
    ensure!(tasks.len() <= 10000, "job exceeds 10000 tasks");
    ensure!(
        read::get_job(connection, &job.id)?.is_none(),
        "job already exists; retry with its original operation key"
    );
    if let Some(parent) = &job.parent_id {
        let parent = read::get_job(connection, parent)?
            .ok_or_else(|| anyhow::anyhow!("parent job does not exist"))?;
        ensure!(
            !parent.cancel_requested && parent.finished_at.is_none(),
            "parent job is no longer active"
        );
    }
    let ids: HashSet<&str> = tasks.iter().map(|task| task.id.as_str()).collect();
    ensure!(ids.len() == tasks.len(), "duplicate task id");
    for task in tasks {
        ensure!(
            !task.id.is_empty() && !task.operation.is_empty() && !task.pool.is_empty(),
            "task id, operation and pool are required"
        );
        ensure!(task.attempt_cap > 0, "task attempt cap must be positive");
        ensure!(
            task.dependencies
                .iter()
                .all(|id| ids.contains(id.as_str()) && id != &task.id),
            "task dependency must reference another task in this job"
        );
    }
    // Topological admission excludes permanently blocked cyclic jobs.
    let mut admitted = HashSet::new();
    while admitted.len() < tasks.len() {
        let before = admitted.len();
        for task in tasks {
            if task
                .dependencies
                .iter()
                .all(|id| admitted.contains(id.as_str()))
            {
                admitted.insert(task.id.as_str());
            }
        }
        ensure!(admitted.len() > before, "task dependency cycle");
    }
    connection.execute("INSERT INTO jobs(id,parent_id,operation,metadata,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?5)",
        rusqlite::params![job.id,job.parent_id,job.operation,serde_json::to_string(&job.metadata)?,now])?;
    for task in tasks {
        connection.execute("INSERT INTO tasks(id,job_id,operation,pool,input,priority,next_eligible_at,attempt_cap,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?9)",
            rusqlite::params![task.id,job.id,task.operation,task.pool,serde_json::to_string(&task.input)?,task.priority,task.next_eligible_at,task.attempt_cap,now])?;
    }
    for task in tasks {
        for dependency in &task.dependencies {
            connection.execute(
                "INSERT INTO dependencies(task_id,dependency_id) VALUES(?1,?2)",
                rusqlite::params![task.id, dependency],
            )?;
        }
        dispatch(connection, &job.id, &task.id, now)?;
    }
    event(
        connection,
        Some(&job.id),
        None,
        "submitted",
        &serde_json::json!({"tasks":tasks.len()}),
        now,
    )?;
    Ok(())
}

fn event(
    connection: &Connection,
    job: Option<&str>,
    task: Option<&str>,
    kind: &str,
    body: &Value,
    now: i64,
) -> Result<i64> {
    connection.execute(
        "INSERT INTO events(job_id,task_id,kind,body,created_at) VALUES(?1,?2,?3,?4,?5)",
        rusqlite::params![job, task, kind, serde_json::to_string(body)?, now],
    )?;
    Ok(connection.last_insert_rowid())
}

fn outbox(
    connection: &Connection,
    job: &str,
    task: &str,
    kind: &str,
    body: &Value,
    now: i64,
) -> Result<()> {
    connection.execute(
        "INSERT INTO outbox(job_id,task_id,kind,body,created_at) VALUES(?1,?2,?3,?4,?5)",
        rusqlite::params![job, task, kind, serde_json::to_string(body)?, now],
    )?;
    Ok(())
}

fn dispatch(connection: &Connection, job: &str, task: &str, now: i64) -> Result<()> {
    let row = read::get_task(connection, task)?.expect("dispatch task exists");
    outbox(
        connection,
        job,
        task,
        "dispatch",
        &serde_json::json!({"task_id":task,"job_id":job,"pool":row.pool,"next_eligible_at":row.next_eligible_at}),
        now,
    )
}

fn settle(connection: &Connection, job: &str, now: i64) -> Result<()> {
    loop {
        let blocked = connection.execute("UPDATE tasks SET state='failed',error='dependency did not complete',updated_at=?2
            WHERE job_id=?1 AND state='queued' AND EXISTS(SELECT 1 FROM dependencies d JOIN tasks dep ON dep.id=d.dependency_id WHERE d.task_id=tasks.id AND dep.state IN ('failed','cancelled'))",rusqlite::params![job,now])?;
        if blocked == 0 {
            break;
        }
    }
    connection.execute("UPDATE jobs SET state=CASE
        WHEN EXISTS(SELECT 1 FROM tasks WHERE job_id=?1 AND state IN ('queued','running')) THEN 'running'
        WHEN EXISTS(SELECT 1 FROM tasks WHERE job_id=?1 AND state='failed') THEN 'failed'
        WHEN EXISTS(SELECT 1 FROM tasks WHERE job_id=?1 AND state='cancelled') THEN 'cancelled'
        ELSE 'completed' END,
        finished_at=CASE WHEN EXISTS(SELECT 1 FROM tasks WHERE job_id=?1 AND state IN ('queued','running')) THEN NULL ELSE ?2 END,updated_at=?2
        WHERE id=?1 AND cancel_requested=0",rusqlite::params![job,now])?;
    Ok(())
}

fn cancel(connection: &Connection, job: &str, now: i64) -> Result<()> {
    let mut statement = connection.prepare("WITH RECURSIVE tree(id) AS (SELECT id FROM jobs WHERE id=?1 UNION ALL SELECT child.id FROM jobs child JOIN tree parent ON child.parent_id=parent.id) SELECT id FROM tree")?;
    let ids = statement
        .query_map([job], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for id in ids {
        connection.execute("UPDATE attempts SET state='cancelled',finished_at=?2,error='job cancelled' WHERE finished_at IS NULL AND task_id IN (SELECT id FROM tasks WHERE job_id=?1)",rusqlite::params![id,now])?;
        connection.execute("UPDATE tasks SET state='cancelled',epoch=epoch+1,owner=NULL,lease_until=NULL,updated_at=?2 WHERE job_id=?1 AND state IN ('queued','running')",rusqlite::params![id,now])?;
        connection.execute("UPDATE jobs SET cancel_requested=1,state=CASE WHEN finished_at IS NULL THEN 'cancelled' ELSE state END,finished_at=COALESCE(finished_at,?2),updated_at=?2 WHERE id=?1",rusqlite::params![id,now])?;
        event(
            connection,
            Some(&id),
            None,
            "cancel_requested",
            &Value::Null,
            now,
        )?;
    }
    Ok(())
}

fn reap(connection: &Connection, now: i64) -> Result<usize> {
    let mut statement = connection.prepare("SELECT id FROM tasks WHERE state='running' AND lease_until<=?1 ORDER BY lease_until LIMIT 1000")?;
    let ids = statement
        .query_map([now], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for id in &ids {
        let task = read::get_task(connection, id)?.expect("expired task exists");
        let retry = task.attempts_used < task.attempt_cap;
        let state = if retry { "queued" } else { "failed" };
        if let Some(lease) = task.lease() {
            finish_attempt(connection, &lease, "interrupted", "lease expired", now)?;
        }
        connection.execute("UPDATE tasks SET state=?2,owner=NULL,lease_until=NULL,next_eligible_at=?3,updated_at=?3,error='lease expired' WHERE id=?1",rusqlite::params![id,state,now])?;
        event(
            connection,
            Some(&task.job_id),
            Some(id),
            "lease_expired",
            &serde_json::json!({"epoch":task.epoch}),
            now,
        )?;
        if retry {
            dispatch(connection, &task.job_id, id, now)?;
        }
        settle(connection, &task.job_id, now)?;
    }
    Ok(ids.len())
}
