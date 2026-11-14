use anyhow::{bail, Result};
use rusqlite::Connection;
use rusqlite::OptionalExtension;
use rusqlite::Row;
use serde::de::DeserializeOwned;

use crate::{Attempt, Command, Event, Job, OutboxEntry, Response, Task};

const JOB_COLUMNS: &str = "id,parent_id,operation,metadata,state,cancel_requested,created_at,updated_at,finished_at,progress_done,progress_total,error";
const TASK_COLUMNS: &str = "id,job_id,operation,pool,input,output,state,priority,next_eligible_at,created_at,updated_at,owner,epoch,lease_until,attempts_used,attempt_cap,error";

fn json<T: DeserializeOwned>(row: &Row<'_>, column: usize) -> rusqlite::Result<T> {
    let value: String = row.get(column)?;
    serde_json::from_str(&value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            column,
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })
}

fn job(row: &Row<'_>) -> rusqlite::Result<Job> {
    Ok(Job {
        id: row.get(0)?,
        parent_id: row.get(1)?,
        operation: row.get(2)?,
        metadata: json(row, 3)?,
        state: row.get(4)?,
        cancel_requested: row.get(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
        finished_at: row.get(8)?,
        progress_done: row.get(9)?,
        progress_total: row.get(10)?,
        error: row.get(11)?,
    })
}

fn task(row: &Row<'_>) -> rusqlite::Result<Task> {
    let output: Option<String> = row.get(5)?;
    let output = output
        .map(|value| {
            serde_json::from_str(&value).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    5,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })
        })
        .transpose()?;
    Ok(Task {
        id: row.get(0)?,
        job_id: row.get(1)?,
        operation: row.get(2)?,
        pool: row.get(3)?,
        input: json(row, 4)?,
        output,
        state: row.get(6)?,
        priority: row.get(7)?,
        next_eligible_at: row.get(8)?,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
        owner: row.get(11)?,
        epoch: row.get(12)?,
        lease_until: row.get(13)?,
        attempts_used: row.get(14)?,
        attempt_cap: row.get(15)?,
        error: row.get(16)?,
    })
}

pub(crate) fn get_job(connection: &Connection, id: &str) -> Result<Option<Job>> {
    Ok(connection
        .query_row(
            &format!("SELECT {JOB_COLUMNS} FROM jobs WHERE id=?1"),
            [id],
            job,
        )
        .optional()?)
}

pub(crate) fn get_task(connection: &Connection, id: &str) -> Result<Option<Task>> {
    Ok(connection
        .query_row(
            &format!("SELECT {TASK_COLUMNS} FROM tasks WHERE id=?1"),
            [id],
            task,
        )
        .optional()?)
}

pub(crate) fn execute(connection: &Connection, command: &Command) -> Result<Response> {
    let response = match command {
        Command::Get { job_id } => Response::Job(get_job(connection, job_id)?),
        Command::List { state, limit } => {
            let mut statement = connection.prepare(&format!("SELECT {JOB_COLUMNS} FROM jobs WHERE (?1 IS NULL OR state=?1) ORDER BY created_at DESC,id LIMIT ?2"))?;
            let rows = statement
                .query_map(rusqlite::params![state, (*limit).min(1000) as i64], job)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Response::Jobs(rows)
        }
        Command::Tasks { job_id } => {
            let mut statement = connection.prepare(&format!(
                "SELECT {TASK_COLUMNS} FROM tasks WHERE job_id=?1 ORDER BY created_at,id"
            ))?;
            let rows = statement
                .query_map([job_id], task)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Response::Tasks(rows)
        }
        Command::Attempts { job_id } => {
            let mut statement = connection.prepare("SELECT a.task_id,a.epoch,a.owner,a.state,a.started_at,a.finished_at,a.error FROM attempts a JOIN tasks t ON t.id=a.task_id WHERE t.job_id=?1 ORDER BY a.started_at,a.task_id,a.epoch")?;
            let rows = statement
                .query_map([job_id], |row| {
                    Ok(Attempt {
                        task_id: row.get(0)?,
                        epoch: row.get(1)?,
                        owner: row.get(2)?,
                        state: row.get(3)?,
                        started_at: row.get(4)?,
                        finished_at: row.get(5)?,
                        error: row.get(6)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Response::Attempts(rows)
        }
        Command::Events {
            job_id,
            after,
            limit,
        } => {
            let mut statement = connection.prepare("SELECT seq,job_id,task_id,kind,body,created_at FROM events WHERE (?1 IS NULL OR job_id=?1) AND seq>?2 ORDER BY seq LIMIT ?3")?;
            let rows = statement
                .query_map(
                    rusqlite::params![job_id, after, (*limit).min(1000) as i64],
                    |row| {
                        Ok(Event {
                            seq: row.get(0)?,
                            job_id: row.get(1)?,
                            task_id: row.get(2)?,
                            kind: row.get(3)?,
                            body: json(row, 4)?,
                            created_at: row.get(5)?,
                        })
                    },
                )?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Response::Events(rows)
        }
        Command::Outbox { kind, limit } => {
            let mut statement = connection.prepare("SELECT seq,job_id,task_id,kind,body,created_at FROM outbox WHERE acknowledged_at IS NULL AND (?1 IS NULL OR kind=?1) ORDER BY seq LIMIT ?2")?;
            let rows = statement
                .query_map(rusqlite::params![kind, (*limit).min(1000) as i64], |row| {
                    Ok(OutboxEntry {
                        seq: row.get(0)?,
                        job_id: row.get(1)?,
                        task_id: row.get(2)?,
                        kind: row.get(3)?,
                        body: json(row, 4)?,
                        created_at: row.get(5)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Response::Outbox(rows)
        }
        _ => bail!("not a Jobs read operation"),
    };
    Ok(response)
}
