//! Global operational retention. Results and canonical consistency outboxes
//! are durable data; job journals are disposable, fenced execution history.
use anyhow::{ensure, Result};
use chrono::{DateTime, Duration, Utc};
use rusqlite::{params, Connection, OptionalExtension};

pub const RETENTION_HOURS: i64 = 72;
pub const BATCH: usize = 256;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CleanupCounts {
    pub cleared: usize,
    pub stopped: usize,
}

pub fn retired(conn: &Connection, id: &str) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM argos_job_retirements WHERE job_id=?1)",
        [id],
        |r| r.get(0),
    )?)
}

pub fn ensure_admissible(conn: &Connection, id: &str, now: &str) -> Result<()> {
    ensure!(
        !retired(conn, id)?,
        "Job history expired; start a fresh run"
    );
    let expired: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM argos_jobs WHERE id=?1 AND julianday(created_at)<julianday(?2)-3)",params![id,now],|r|r.get(0))?;
    ensure!(!expired, "Job history expired; start a fresh run");
    let owners = conn
        .prepare("SELECT owner_kind,owner_id FROM argos_job_owners WHERE job_id=?1 AND required=1")?
        .query_map([id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (kind, owner) in owners {
        let sql=match kind.as_str() {
            "article"=>"SELECT EXISTS(SELECT 1 FROM atlas_articles WHERE article_id=?1)",
            "atlas_run"=>"SELECT EXISTS(SELECT 1 FROM atlas_runs WHERE id=?1)",
            "thread"=>"SELECT EXISTS(SELECT 1 FROM recon_threads WHERE id=?1 AND deleted=0)",
            "memory"=>"SELECT EXISTS(SELECT 1 FROM memories WHERE id=?1)",
            "recon_run"=>"SELECT EXISTS(SELECT 1 FROM recon_runs r JOIN recon_threads t ON t.id=r.thread_id WHERE r.id=?1 AND t.deleted=0)",
            _=>continue,
        };
        ensure!(
            conn.query_row(sql, [owner], |r| r.get::<_, bool>(0))?,
            "required job owner deleted"
        );
    }
    Ok(())
}

pub fn register_reference(
    conn: &Connection,
    job: &str,
    reference: &str,
    operation: &str,
) -> Result<()> {
    let Some((kind, id)) = reference.split_once(':') else {
        return Ok(());
    };
    if !matches!(
        kind,
        "article" | "atlas_run" | "thread" | "memory" | "recon_run"
    ) || matches!(operation, "index_remove" | "remove")
    {
        return Ok(());
    }
    conn.execute(
        "INSERT OR IGNORE INTO argos_job_owners(job_id,owner_kind,owner_id) VALUES(?1,?2,?3)",
        params![job, kind, id],
    )?;
    Ok(())
}

/// Exact formats only. Index deletion targets are deliberately not owners.
pub fn backfill_owners(conn: &Connection) -> Result<()> {
    conn.execute_batch("UPDATE argos_jobs SET parent_id=COALESCE((SELECT p.id FROM intel_report_jobs child JOIN argos_jobs p ON p.run_ref=child.parent_job_id AND p.operation='intel_recon' WHERE child.id=argos_jobs.run_ref LIMIT 1),'') WHERE operation='intel_recon' AND parent_id='';")?;
    conn.execute_batch("INSERT OR IGNORE INTO argos_job_owners(job_id,owner_kind,owner_id)
        SELECT id,'article',substr(resource_ref,9) FROM argos_jobs WHERE resource_ref LIKE 'article:%' AND operation NOT IN ('index_remove','remove');
        INSERT OR IGNORE INTO argos_job_owners(job_id,owner_kind,owner_id)
        SELECT id,'thread',substr(resource_ref,8) FROM argos_jobs WHERE resource_ref LIKE 'thread:%';
        INSERT OR IGNORE INTO argos_job_owners(job_id,owner_kind,owner_id)
        SELECT id,'memory',substr(resource_ref,8) FROM argos_jobs WHERE resource_ref LIKE 'memory:%' AND operation NOT IN ('index_remove','remove');
        INSERT OR IGNORE INTO argos_job_owners(job_id,owner_kind,owner_id)
        SELECT id,'article',substr(owner_scope,9) FROM argos_jobs WHERE owner_scope LIKE 'article:%';
        INSERT OR IGNORE INTO argos_job_owners(job_id,owner_kind,owner_id)
        SELECT j.id,'article',r.article_id FROM argos_jobs j JOIN intel_report_jobs r ON r.id=j.run_ref WHERE j.operation='intel_recon';
        INSERT OR IGNORE INTO argos_job_owners(job_id,owner_kind,owner_id)
        SELECT id,'atlas_run',run_ref FROM argos_jobs WHERE app='atlas' AND run_ref<>'' AND operation IN ('atlas_cycle','atlas_entities','atlas_publishers','atlas_classify','atlas_collect','atlas_related');
        INSERT OR IGNORE INTO argos_job_owners(job_id,owner_kind,owner_id)
        SELECT id,'recon_run',substr(owner_scope,6) FROM argos_jobs WHERE owner_scope LIKE 'recon:%';")?;
    Ok(())
}

pub const OWNER_MISSING: &str = "EXISTS(SELECT 1 FROM argos_job_owners o WHERE o.job_id=j.id AND o.required=1 AND (
    (o.owner_kind='article' AND NOT EXISTS(SELECT 1 FROM atlas_articles a WHERE a.article_id=o.owner_id)) OR
    (o.owner_kind='atlas_run' AND NOT EXISTS(SELECT 1 FROM atlas_runs a WHERE a.id=o.owner_id)) OR
    (o.owner_kind='thread' AND NOT EXISTS(SELECT 1 FROM recon_threads t WHERE t.id=o.owner_id AND t.deleted=0)) OR
    (o.owner_kind='recon_run' AND NOT EXISTS(SELECT 1 FROM recon_runs r JOIN recon_threads t ON t.id=r.thread_id WHERE r.id=o.owner_id AND t.deleted=0)) OR
    (o.owner_kind='memory' AND NOT EXISTS(SELECT 1 FROM memories m WHERE m.id=o.owner_id))))";

/// View predicate used consistently by lists, counts, details and controls.
pub fn eligible(alias: &str) -> String {
    format!("julianday({alias}.created_at)>=julianday('now')-3 AND NOT EXISTS(SELECT 1 FROM argos_job_retirements x WHERE x.job_id={alias}.id) AND NOT ({})",OWNER_MISSING.replace("j.id",&format!("{alias}.id")))
}

/// One bounded batch; safe both inside deletion transactions and standalone.
pub fn cleanup_batch(conn: &Connection, now: DateTime<Utc>) -> Result<CleanupCounts> {
    let own = conn.is_autocommit();
    if own {
        conn.execute_batch("BEGIN IMMEDIATE")?;
    }
    let result = cleanup_on(conn, now);
    if own {
        if result.is_ok() {
            conn.execute_batch("COMMIT")?;
        } else {
            conn.execute_batch("ROLLBACK")?;
        }
    }
    result
}

/// One-time retirement of article Recon execution. Keep artifacts and owner
/// generations intact; only execution leases and descendants are cancelled.
pub fn retire_article_reports(conn: &Connection) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    conn.execute_batch("SAVEPOINT retire_article_reports")?;
    let result = (|| -> Result<Vec<String>> {
        // Repair only report ancestry. General owner backfill would alter
        // unrelated jobs merely because a store was opened for retirement.
        conn.execute_batch("UPDATE argos_jobs SET parent_id=COALESCE((SELECT p.id FROM intel_report_jobs child JOIN argos_jobs p ON p.run_ref=child.parent_job_id AND p.operation='intel_recon' WHERE child.id=argos_jobs.run_ref LIMIT 1),'') WHERE operation='intel_recon' AND parent_id='';")?;
        let roots = conn.prepare("SELECT id FROM intel_report_jobs WHERE state NOT IN ('completed','done','failed','cancelled','partial')")?.query_map([], |r| r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let mut signalled = Vec::new();
        for root in roots {
            let rows = conn.prepare("WITH RECURSIVE tree(id) AS (SELECT id FROM argos_jobs WHERE operation='intel_recon' AND run_ref=?1 UNION SELECT j.id FROM argos_jobs j JOIN tree t ON j.parent_id=t.id) SELECT id FROM tree")?.query_map([&root], |r| r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            for id in rows.into_iter().chain(std::iter::once(root.clone())) {
                conn.execute("INSERT OR IGNORE INTO argos_job_retirements(job_id,retired_at,reason) VALUES(?1,?2,'Intel article reports retired')",params![id,now])?;
                conn.execute("UPDATE argos_tasks SET state='cancelled',lease_owner='',lease_until='',lease_epoch=lease_epoch+1,updated_at=?2 WHERE job_id=?1 AND state<>'completed'",params![id,now])?;
                conn.execute("UPDATE argos_jobs SET state='cancelled' WHERE id=?1 AND state NOT IN ('completed','failed','cancelled','partial')",[&id])?;
                signalled.push(id);
            }
            conn.execute("UPDATE intel_report_tasks SET status='cancelled',lease_owner='',lease_until='',lease_epoch=lease_epoch+1 WHERE job_id=?1 AND status<>'completed'",[&root])?;
            conn.execute("UPDATE intel_report_jobs SET state='cancelled',stage='Intel article reports retired',updated_at=?2 WHERE id=?1",params![root,now])?;
        }
        let refreshes=conn.prepare("WITH RECURSIVE tree(id) AS (SELECT DISTINCT j.id FROM argos_jobs j JOIN argos_tasks t ON t.job_id=j.id WHERE t.operation='insight_refresh' AND json_valid(t.input_ref) AND json_extract(t.input_ref,'$.surface')='recon_report' AND t.state NOT IN ('completed','cancelled') UNION SELECT j.id FROM argos_jobs j JOIN tree p ON j.parent_id=p.id) SELECT id FROM tree")?.query_map([],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        for id in refreshes {
            conn.execute("INSERT OR IGNORE INTO argos_job_retirements(job_id,retired_at,reason) VALUES(?1,?2,'Intel report extraction retired')",params![id,now])?;
            conn.execute("UPDATE argos_tasks SET state='cancelled',lease_owner='',lease_until='',lease_epoch=lease_epoch+1,updated_at=?2 WHERE job_id=?1 AND state<>'completed'",params![id,now])?;
            conn.execute("UPDATE argos_jobs SET state='cancelled' WHERE id=?1", [&id])?;
            signalled.push(id);
        }
        Ok(signalled)
    })();
    match result {
        Ok(ids) => {
            conn.execute_batch("RELEASE retire_article_reports")?;
            for id in ids {
                crate::job_registry::signal_retirement(&id);
            }
            Ok(())
        }
        Err(e) => {
            conn.execute_batch(
                "ROLLBACK TO retire_article_reports; RELEASE retire_article_reports",
            )?;
            Err(e)
        }
    }
}

fn cleanup_on(conn: &Connection, now: DateTime<Utc>) -> Result<CleanupCounts> {
    backfill_owners(conn)?;
    let ts = now.to_rfc3339();
    let cutoff = (now - Duration::hours(RETENTION_HOURS)).to_rfc3339();
    // A malformed legacy timestamp is repaired once, never from a heartbeat.
    conn.execute("UPDATE argos_jobs SET created_at=COALESCE((SELECT stamp FROM (SELECT created_at AS stamp FROM argos_tasks WHERE job_id=argos_jobs.id UNION ALL SELECT a.started_at AS stamp FROM argos_attempts a JOIN argos_tasks t ON t.id=a.task_id WHERE t.job_id=argos_jobs.id UNION ALL SELECT argos_jobs.started_at UNION ALL SELECT argos_jobs.queued_at) WHERE julianday(stamp) IS NOT NULL ORDER BY julianday(stamp) LIMIT 1),?1) WHERE julianday(created_at) IS NULL",[&ts])?;
    let sql = format!("WITH RECURSIVE retired_tree(id) AS (
        SELECT id FROM (SELECT j.id FROM argos_jobs j WHERE julianday(j.created_at)<julianday(?1) OR ({OWNER_MISSING}) ORDER BY j.created_at,j.id LIMIT {BATCH})
        UNION SELECT j.id FROM argos_jobs j JOIN retired_tree p ON j.parent_id=p.id)
        SELECT j.id,j.state,j.run_ref,j.operation FROM argos_jobs j JOIN retired_tree r ON r.id=j.id");
    let rows = conn
        .prepare(&sql)?
        .query_map([&cutoff], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut counts = retire_rows(conn, rows, &ts, "expired or required owner deleted", false)?;
    // Legacy runtime history uses the same immutable start/creation deadline.
    // Completed bundles have no FK to these journals and remain readable.
    conn.execute("UPDATE intel_report_jobs SET started_at=COALESCE((SELECT stamp FROM (SELECT created_at AS stamp FROM intel_report_tasks WHERE job_id=intel_report_jobs.id UNION ALL SELECT created_at FROM intel_report_sections WHERE job_id=intel_report_jobs.id UNION ALL SELECT intel_report_jobs.finished_at) WHERE julianday(stamp) IS NOT NULL ORDER BY julianday(stamp) LIMIT 1),?1) WHERE julianday(started_at) IS NULL",[&ts])?;
    let legacy = conn.prepare("SELECT id FROM intel_report_jobs WHERE julianday(started_at)<julianday(?1) OR id IN (SELECT job_id FROM argos_job_retirements) OR NOT EXISTS(SELECT 1 FROM atlas_articles a WHERE a.article_id=intel_report_jobs.article_id) ORDER BY started_at LIMIT 256")?.query_map([&cutoff],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
    for id in legacy {
        retain_completed_result(conn, &id)?;
        conn.execute("INSERT OR IGNORE INTO argos_job_retirements(job_id,retired_at,reason) VALUES(?1,?2,'legacy report history expired')",params![id,ts])?;
        conn.execute("DELETE FROM intel_report_attempts WHERE job_id=?1", [&id])?;
        conn.execute("DELETE FROM intel_report_tasks WHERE job_id=?1", [&id])?;
        conn.execute("DELETE FROM intel_report_sections WHERE job_id=?1", [&id])?;
        conn.execute(
            "DELETE FROM intel_report_support_receipts WHERE job_id=?1",
            [&id],
        )?;
        counts.cleared += conn.execute("DELETE FROM intel_report_jobs WHERE id=?1", [&id])?;
    }
    Ok(counts)
}

/// Shared journal removal; canonical results and index consistency survive.
fn retire_rows(
    conn: &Connection,
    rows: Vec<(String, String, String, String)>,
    ts: &str,
    reason: &str,
    manual: bool,
) -> Result<CleanupCounts> {
    let mut counts = CleanupCounts::default();
    for (id, state, run, operation) in rows {
        let refreshes = conn.prepare("SELECT input_ref,role_snapshot FROM argos_tasks WHERE job_id=?1 AND operation='insight_refresh'")?.query_map([&id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        for (input, settings) in refreshes {
            if let (Ok(mut input), Ok(settings)) = (
                serde_json::from_str::<crate::investigation::insights::PipelineInput>(&input),
                serde_json::from_str::<crate::provider::SettingsFile>(&settings),
            ) {
                input.snapshot.generation = 0;
                let key = crate::investigation::insights::input_key(&input, &settings);
                conn.execute("INSERT OR IGNORE INTO article_refresh_retirements(article_id,input_key,retired_at) VALUES(?1,?2,?3)",params![input.snapshot.owner_id,key,ts])?;
            }
        }
        conn.execute("INSERT OR IGNORE INTO argos_job_retirements(job_id,retired_at,reason) VALUES(?1,?2,?3)",params![id,ts,reason])?;
        if matches!(
            state.as_str(),
            "running" | "queued" | "waiting" | "paused" | "blocked" | "retry_scheduled"
        ) {
            counts.stopped += 1;
        }
        conn.execute("UPDATE argos_tasks SET state='cancelled',lease_owner='',lease_until='',lease_epoch=lease_epoch+1,updated_at=?2 WHERE job_id=?1",params![id,ts])?;
        // Keep canonical removals/upserts and rebind them to a fresh service.
        conn.execute("UPDATE argos_index_changes SET task_id='',job_id='',state='pending',updated_at=?2 WHERE job_id=?1 AND state IN ('pending','running','blocked')",params![id,ts])?;
        if manual {
            conn.execute(
                "UPDATE insight_owner_state SET refresh_job_id='' WHERE refresh_job_id=?1",
                [&id],
            )?;
        } else {
            conn.execute("UPDATE insight_owner_state SET desired_generation=desired_generation+1,lifecycle='previous',last_error='analysis request expired',refresh_job_id='' WHERE refresh_job_id=?1",[&id])?;
        }
        if operation == "article_body" {
            conn.execute("UPDATE article_bodies SET state='cancelled' WHERE article_id IN (SELECT owner_id FROM argos_job_owners WHERE job_id=?1 AND owner_kind='article') AND state IN ('queued','running')",[&id])?;
        }
        if operation == "intel_recon" {
            conn.execute("INSERT OR IGNORE INTO argos_job_retirements(job_id,retired_at,reason) VALUES(?1,?2,'report execution retired')",params![run,ts])?;
            conn.execute("UPDATE intel_report_jobs SET state='cancelled',stage='Job history expired',updated_at=?2 WHERE id=?1 AND state NOT IN ('completed','failed','cancelled','partial')",params![run,ts])?;
        }
        if operation == "recon_investigation" {
            conn.execute("UPDATE recon_runs SET state='cancelled',stage='Job history expired' WHERE id=?1 AND state NOT IN ('completed','failed','cancelled')",[&run])?;
        }
        conn.execute("DELETE FROM argos_attempts WHERE task_id IN (SELECT id FROM argos_tasks WHERE job_id=?1)",[&id])?;
        conn.execute("DELETE FROM argos_tasks WHERE job_id=?1", [&id])?;
        conn.execute("DELETE FROM argos_job_owners WHERE job_id=?1", [&id])?;
        conn.execute("DELETE FROM argos_jobs WHERE id=?1", [&id])?;
        counts.cleared += 1;
        if !manual {
            crate::job_registry::signal_retirement(&id);
        }
        let _ = crate::events::record_event(
            conn,
            &crate::events::NewEvent {
                app: "jobs".into(),
                event_type: "job_retired".into(),
                severity: Some(crate::events::Severity::Info),
                message: reason.into(),
                job_id: id.clone(),
                ..Default::default()
            },
        );
    }
    Ok(counts)
}

/// Stop durable work while retaining its journal. Lease epochs fence late
/// completion/publication, including another process's in-flight work.
pub(crate) fn stop_task_job(conn: &Connection, job: &str) -> Result<()> {
    let tx = rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)?;
    let now = Utc::now().to_rfc3339();
    let ids = tx.prepare("WITH RECURSIVE tree(id) AS (SELECT id FROM argos_jobs WHERE id=?1 AND kind<>'service' AND state IN ('running','queued','retry_scheduled') UNION SELECT j.id FROM argos_jobs j JOIN tree p ON j.parent_id=p.id WHERE j.kind<>'service') SELECT id FROM tree")?.query_map([job],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
    for id in &ids {
        tx.execute("UPDATE argos_attempts SET finished_at=?2,outcome='cancelled',duration_ms=MAX(0,CAST((julianday(?2)-julianday(started_at))*86400000 AS INTEGER)) WHERE task_id IN (SELECT id FROM argos_tasks WHERE job_id=?1) AND finished_at=''",params![id,now])?;
        tx.execute("UPDATE argos_tasks SET active_ms=active_ms+CASE WHEN state='running' THEN MAX(0,CAST((julianday(?2)-julianday(COALESCE((SELECT a.started_at FROM argos_attempts a WHERE a.task_id=argos_tasks.id ORDER BY a.attempt_number DESC LIMIT 1),started_at)))*86400000 AS INTEGER)) ELSE 0 END,state='cancelled',lease_epoch=lease_epoch+1,lease_owner='',lease_until='',finished_at=?2,updated_at=?2 WHERE job_id=?1 AND state IN ('running','queued','retry_scheduled','paused','blocked','waiting')",params![id,now])?;
        tx.execute("UPDATE argos_jobs SET state='cancelled',cancel_requested=1,finished_at=?2,updated_at=?2,phase='Stopped by user' WHERE id=?1 AND state NOT IN ('completed','failed','partial','cancelled','superseded')",params![id,now])?;
        tx.execute(
            "UPDATE intel_brief_runs SET state='cancelled',updated_at=?2 WHERE job_id=?1 AND state='active'",
            params![id, now],
        )?;
        tx.execute("UPDATE argos_index_changes SET task_id='',job_id='',state='pending',updated_at=?2 WHERE job_id=?1 AND state IN ('pending','running','blocked')",params![id,now])?;
        crate::tasks::refresh_job_state(&tx, id, &now)?;
        crate::events::record_event(
            &tx,
            &crate::events::NewEvent {
                app: "jobs".into(),
                event_type: "job.cancel_requested".into(),
                message: "Stopped by user".into(),
                job_id: id.clone(),
                ..Default::default()
            },
        )?;
    }
    tx.commit()?;
    for id in ids {
        crate::job_registry::signal_retirement(&id);
    }
    Ok(())
}

fn retain_completed_result(conn: &Connection, job: &str) -> Result<()> {
    let row: Option<(String,i64,String)> = conn.query_row("SELECT article_id,generation,finished_at FROM intel_report_jobs WHERE id=?1 AND state='completed'",[job],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
    let Some((article, generation, finished)) = row else {
        return Ok(());
    };
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM intel_report_results WHERE job_id=?1 AND generation=?2)",
        params![job, generation],
        |r| r.get(0),
    )?;
    if exists {
        return Ok(());
    }
    let job_fields = [
        "id",
        "investigation_id",
        "article_id",
        "mode",
        "revision",
        "state",
        "stage",
        "settings_json",
        "budget_json",
        "parent_job_id",
        "sections_done",
        "sections_total",
        "elements_done",
        "elements_total",
        "tool_calls_done",
        "tool_calls_allowance",
        "current_tool",
        "warning",
        "error",
        "generation",
        "started_at",
        "updated_at",
        "finished_at",
    ];
    let pairs = job_fields
        .iter()
        .map(|name| format!("'{name}',{name}"))
        .collect::<Vec<_>>()
        .join(",");
    let job_json: String = conn.query_row(
        &format!("SELECT json_object({pairs}) FROM intel_report_jobs WHERE id=?1"),
        [job],
        |r| r.get(0),
    )?;
    let job_json: serde_json::Value = serde_json::from_str(&job_json)?;
    let section_fields = [
        "id",
        "job_id",
        "section_key",
        "title",
        "ordinal",
        "markdown",
        "status",
        "revision",
        "evidence_ids_json",
        "judgment_json",
        "assessment_version",
        "waiting_on",
        "created_at",
        "updated_at",
    ];
    let pairs = section_fields
        .iter()
        .map(|name| format!("'{name}',{name}"))
        .collect::<Vec<_>>()
        .join(",");
    let sections=conn.prepare(&format!("SELECT json_object({pairs}) FROM intel_report_sections WHERE job_id=?1 AND status='complete' ORDER BY ordinal"))?.query_map([job],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?.into_iter().map(|json|serde_json::from_str::<serde_json::Value>(&json)).collect::<serde_json::Result<Vec<_>>>()?;
    let text = sections
        .iter()
        .map(|s| {
            format!(
                "## {}\n\n{}",
                s["title"].as_str().unwrap_or_default(),
                s["markdown"].as_str().unwrap_or_default()
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    // Legacy bundles are retained history, not new analysis requests or authority.
    let sequence:i64=conn.query_row("SELECT COALESCE(MIN(completion_sequence),0)-1 FROM intel_report_results WHERE article_id=?1",[&article],|r|r.get(0))?;
    let details=serde_json::json!({"contract":"legacy-report-history-v1","text":text,"sections":sections,"job":job_json,"completed_at":finished}).to_string();
    conn.execute("INSERT INTO intel_report_results(job_id,generation,article_id,completion_sequence,source_hash,details_json,completed_at) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![job,generation,article,sequence,crate::nlp::cache::hash(&details),details,finished])?;
    Ok(())
}

impl crate::store::Store {
    /// Clear one terminal status cohort globally. Never remove a service or a
    /// tree with running descendants; selection and deletion share a write lock.
    pub fn clear_jobs_by_status(
        &self,
        status: crate::jobs_view::JobStatusFilter,
    ) -> Result<CleanupCounts> {
        use crate::jobs_view::JobStatusFilter;
        ensure!(
            matches!(
                status,
                JobStatusFilter::Completed | JobStatusFilter::Stopped | JobStatusFilter::Failed
            ),
            "Choose Completed, Stopped, or Failed history"
        );
        let mut total = CleanupCounts::default();
        loop {
            let tx = rusqlite::Transaction::new_unchecked(
                &self.conn,
                rusqlite::TransactionBehavior::Immediate,
            )?;
            let states = status
                .states()
                .iter()
                .map(|s| format!("'{s}'"))
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!("WITH RECURSIVE roots(id) AS (
                SELECT j.id FROM argos_jobs j WHERE j.parent_id='' AND j.kind<>'service' AND j.state IN ({states})
                AND NOT EXISTS(WITH RECURSIVE descendants(id) AS (SELECT j.id UNION SELECT c.id FROM argos_jobs c JOIN descendants d ON c.parent_id=d.id) SELECT 1 FROM argos_jobs c JOIN descendants d ON c.id=d.id WHERE c.state='running' OR c.kind='service' OR EXISTS(SELECT 1 FROM argos_tasks t WHERE t.job_id=c.id AND t.state='running'))
                ORDER BY j.created_at,j.id LIMIT {BATCH}),
                tree(id) AS (SELECT id FROM roots UNION SELECT c.id FROM argos_jobs c JOIN tree p ON c.parent_id=p.id)
                SELECT j.id,j.state,j.run_ref,j.operation FROM argos_jobs j JOIN tree t ON t.id=j.id");
            let rows = tx
                .prepare(&sql)?
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let ids = rows
                .iter()
                .map(|row: &(String, String, String, String)| row.0.clone())
                .collect::<Vec<_>>();
            let batch = retire_rows(
                &tx,
                rows,
                &Utc::now().to_rfc3339(),
                "Job history cleared by status",
                true,
            )?;
            tx.commit()?;
            for id in ids {
                crate::job_registry::signal_retirement(&id);
            }
            total.cleared += batch.cleared;
            total.stopped += batch.stopped;
            if batch.cleared == 0 {
                break;
            }
        }
        Ok(total)
    }

    pub fn clear_old_jobs(&self) -> Result<CleanupCounts> {
        let now = Utc::now();
        let mut total = CleanupCounts::default();
        loop {
            let batch = cleanup_batch(&self.conn, now)?;
            total.cleared += batch.cleared;
            total.stopped += batch.stopped;
            if batch.cleared == 0 {
                break;
            }
        }
        Ok(total)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;
    use crate::tasks::{self, JobMeta, NewJob, NewTask};
}
