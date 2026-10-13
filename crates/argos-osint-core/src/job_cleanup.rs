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
        conn.execute("INSERT OR IGNORE INTO argos_job_retirements(job_id,retired_at,reason) VALUES(?1,?2,'expired or required owner deleted')",params![id,ts])?;
        if matches!(
            state.as_str(),
            "running" | "queued" | "waiting" | "paused" | "blocked" | "retry_scheduled"
        ) {
            counts.stopped += 1;
        }
        conn.execute("UPDATE argos_tasks SET state='cancelled',lease_owner='',lease_until='',lease_epoch=lease_epoch+1,updated_at=?2 WHERE job_id=?1",params![id,ts])?;
        // Keep canonical removals/upserts and rebind them to a fresh service.
        conn.execute("UPDATE argos_index_changes SET task_id='',job_id='',state='pending',updated_at=?2 WHERE job_id=?1 AND state IN ('pending','running','blocked')",params![id,ts])?;
        conn.execute("UPDATE insight_owner_state SET desired_generation=desired_generation+1,lifecycle='previous',last_error='analysis request expired',refresh_job_id='' WHERE refresh_job_id=?1",[&id])?;
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
        crate::job_registry::signal_retirement(&id);
        let _ = crate::events::record_event(
            conn,
            &crate::events::NewEvent {
                app: "jobs".into(),
                event_type: "job_retired".into(),
                severity: Some(crate::events::Severity::Info),
                message: "Job history expired".into(),
                job_id: id.clone(),
                ..Default::default()
            },
        );
    }
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
    use crate::tasks::{self, JobMeta, NewJob, NewTask};

    fn job(conn: &Connection, id: &str, created: &str, parent: &str, resource: &str) {
        tasks::enqueue_job_with(
            conn,
            &NewJob {
                id: id.into(),
                kind: "user".into(),
                owner_scope: String::new(),
                input_revision: String::new(),
                deadline_at: String::new(),
            },
            &JobMeta {
                parent_id: parent.into(),
                resource_ref: String::new(),
                ..Default::default()
            },
            created,
        )
        .unwrap();
        conn.execute(
            "UPDATE argos_jobs SET resource_ref=?2 WHERE id=?1",
            params![id, resource],
        )
        .unwrap();
    }
    #[test]
    fn hard_cutoff_ignores_heartbeats_status_and_utc_offsets_and_fences_trees() {
        let store = crate::store::Store::memory().unwrap();
        let now = DateTime::parse_from_rfc3339("2026-10-12T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        for (i, status) in [
            "running",
            "queued",
            "waiting",
            "blocked",
            "paused",
            "completed",
            "failed",
            "cancelled",
            "retry_scheduled",
            "partial",
        ]
        .iter()
        .enumerate()
        {
            let id = format!("old-{i}");
            job(&store.conn, &id, "2026-10-09T07:59:59-04:00", "", "");
            store
                .conn
                .execute(
                    "UPDATE argos_jobs SET state=?2,heartbeat_at=?3,updated_at=?3 WHERE id=?1",
                    params![id, status, now.to_rfc3339()],
                )
                .unwrap();
        }
        job(&store.conn, "exact", "2026-10-09T08:00:00-04:00", "", "");
        job(&store.conn, "younger", "2026-10-09T12:00:01Z", "", "");
        job(&store.conn, "child", "2026-10-12T11:00:00Z", "old-0", "");
        tasks::enqueue_task(
            &store.conn,
            &NewTask {
                id: "child-task".into(),
                job_id: "child".into(),
                operation: "insight_refresh".into(),
                dedupe_key: String::new(),
                priority: 1,
                input_ref: String::new(),
                input_hash: String::new(),
                source_revision: String::new(),
                role_snapshot: String::new(),
                max_attempts: 1,
            },
            "2026-10-12T11:00:00Z",
        )
        .unwrap();
        let lease = tasks::claim_task(
            &store.conn,
            "child-task",
            "worker",
            120,
            "2026-10-12T11:00:00Z",
        )
        .unwrap()
        .unwrap();
        let counts = cleanup_batch(&store.conn, now).unwrap();
        assert_eq!(counts.cleared, 11);
        assert_eq!(
            store
                .conn
                .query_row("SELECT COUNT(*) FROM argos_jobs", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            2
        );
        assert_eq!(
            cleanup_batch(&store.conn, now).unwrap(),
            CleanupCounts::default()
        );
        assert!(ensure_admissible(&store.conn, "old-0", &now.to_rfc3339()).is_err());
        assert!(!tasks::complete_claimed(&store.conn, &lease, "late", &now.to_rfc3339()).unwrap());
        assert!(tasks::enqueue_job(
            &store.conn,
            &NewJob {
                id: "old-0".into(),
                kind: "user".into(),
                owner_scope: String::new(),
                input_revision: String::new(),
                deadline_at: String::new()
            },
            &now.to_rfc3339()
        )
        .is_err());
        assert!(store
            .conn
            .prepare("PRAGMA foreign_key_check")
            .unwrap()
            .query([])
            .unwrap()
            .next()
            .unwrap()
            .is_none());
    }
    #[test]
    fn required_owner_deletion_preserves_index_remove_and_canonical_data() {
        let store = crate::store::Store::memory().unwrap();
        let now = Utc::now().to_rfc3339();
        for state in ["waiting", "blocked", "paused", "running"] {
            job(&store.conn, state, &now, "", "article:deleted");
            store
                .conn
                .execute("UPDATE argos_jobs SET state=?1 WHERE id=?1", [state])
                .unwrap();
        }
        job(&store.conn, "removal", &now, "", "memory:already-deleted");
        store
            .conn
            .execute(
                "UPDATE argos_jobs SET operation='index_remove' WHERE id='removal'",
                [],
            )
            .unwrap();
        let counts = cleanup_batch(&store.conn, Utc::now()).unwrap();
        assert_eq!((counts.cleared, counts.stopped), (4, 4));
        assert!(store.get_job("removal").unwrap().is_some());
    }
    #[test]
    fn service_rotation_rebinds_pending_outbox_and_retains_removals() {
        let store = crate::store::Store::memory().unwrap();
        let now = Utc::now();
        let old = (now - Duration::hours(73)).to_rfc3339();
        tasks::enqueue_index_work(
            &store.conn,
            "memory",
            "gone",
            "r",
            "r",
            "index_remove",
            None,
            &old,
        )
        .unwrap();
        cleanup_batch(&store.conn, now).unwrap();
        assert!(retired(&store.conn, tasks::INDEX_SERVICE_JOB).unwrap());
        assert_eq!(
            tasks::adopt_untracked_index_changes(&store.conn, &now.to_rfc3339()).unwrap(),
            1
        );
        let row: (String, String) = store
            .conn
            .query_row("SELECT job_id,state FROM argos_index_changes", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert!(row.0.starts_with("svc-local-index:g:"));
        assert_eq!(row.1, "pending");
        assert!(store
            .conn
            .prepare("PRAGMA foreign_key_check")
            .unwrap()
            .query([])
            .unwrap()
            .next()
            .unwrap()
            .is_none());
    }

    #[test]
    fn global_cleanup_reaches_every_batch_and_keeps_younger_ownerless_jobs() {
        let store = crate::store::Store::memory().unwrap();
        let old = (Utc::now() - Duration::hours(73)).to_rfc3339();
        let tx = store.conn.unchecked_transaction().unwrap();
        for i in 0..520 {
            tx.execute("INSERT INTO argos_jobs(id,kind,state,created_at,updated_at,app) VALUES(?1,'user',?2,?3,?3,?4)",params![format!("old-{i}"),if i%2==0 {"blocked"} else {"completed"},old,format!("app-{}",i%3)]).unwrap();
        }
        tx.commit().unwrap();
        job(&store.conn, "younger", &Utc::now().to_rfc3339(), "", "");
        let counts = store.clear_old_jobs().unwrap();
        assert_eq!((counts.cleared, counts.stopped), (520, 260));
        assert!(store.get_job("younger").unwrap().is_some());
        assert_eq!(store.clear_old_jobs().unwrap(), CleanupCounts::default());
    }
}
