//! Lifecycle writes for application jobs that run inside a process.

use anyhow::Result;
use chrono::Utc;

use super::jobs::rid;
use super::repo::Repository;

/// `JobHandle::finish`: `state` is a `job_registry::Finish::state` value;
/// terminal states stamp `finished_at`, `paused`/`waiting`/`blocked` do not.
#[allow(clippy::too_many_arguments)]
pub async fn finish_app_job(
    repo: &Repository,
    id: &str,
    state: &str,
    run_ref: Option<&str>,
    error_category: &str,
    error_summary: &str,
    result_ref: &str,
    active_ms: i64,
) -> Result<()> {
    let terminal = !matches!(state, "paused" | "waiting" | "blocked");
    let summary: String = crate::events::redact(error_summary)
        .chars()
        .take(500)
        .collect();
    repo.db()
        .query(
            "UPDATE $j SET state = $s, updated_at = <datetime>$at, heartbeat_at = <datetime>$at,
               active_since = NONE, active_ms = $ms,
               finished_at = IF $terminal { <datetime>$at } ELSE { NONE },
               error_category = $cat, error_summary = $sum,
               result_ref = IF $res != '' { $res } ELSE { result_ref },
               run_ref = IF $run != '' { $run } ELSE { run_ref } RETURN NONE",
        )
        .bind(("j", rid("job", id)))
        .bind(("s", state.to_string()))
        .bind(("ms", active_ms))
        .bind(("terminal", terminal))
        .bind(("cat", error_category.to_string()))
        .bind(("sum", summary))
        .bind(("res", result_ref.to_string()))
        .bind(("run", run_ref.unwrap_or("").to_string()))
        .bind(("at", Utc::now().to_rfc3339()))
        .await?
        .check()?;
    Ok(())
}

/// A registry row of another app (Atlas cycle and phase jobs).
#[derive(Debug, Clone)]
pub struct AppJob<'a> {
    pub id: &'a str,
    pub operation: &'a str,
    pub app: &'a str,
    pub title: &'a str,
    pub run_ref: &'a str,
    pub resource_ref: &'a str,
    /// Parent job id for a phase child; `None` for a top-level job.
    pub parent: Option<&'a str>,
}

/// Create a running application job or restart the existing row and increment its attempts.
pub async fn begin_app_job(repo: &Repository, spec: &AppJob<'_>) -> Result<()> {
    if spec.id.is_empty() {
        anyhow::bail!("job: empty id");
    }
    let parent = spec.parent.filter(|p| !p.is_empty()).map(|p| rid("job", p));
    repo.db()
        .query(
            "IF record::exists($j) {
               UPDATE $j SET state = 'running', finished_at = NONE, cancel_requested = false,
                 error_category = '', error_summary = '', attempts_used += 1,
                 heartbeat_at = <datetime>$at, active_since = <datetime>$at,
                 updated_at = <datetime>$at RETURN NONE;
             } ELSE {
               CREATE $j SET kind = 'inproc', operation = $op, parent = $parent,
                 ancestors = IF $parent != NONE { [$parent] } ELSE { [] },
                 root = IF $parent != NONE { $parent.root ?? $parent } ELSE { $j },
                 state = 'running', app = $app, title = $title, run_ref = $run,
                 resource_ref = $res, correlation_id = $key, cancellable = true,
                 attempts_used = 1, attempt_cap = 1, worker_owner = $owner,
                 heartbeat_at = <datetime>$at, active_since = <datetime>$at,
                 created_at = <datetime>$at, updated_at = <datetime>$at RETURN NONE;
             };",
        )
        .bind(("j", rid("job", spec.id)))
        .bind(("key", spec.id.to_string()))
        .bind(("op", spec.operation.to_string()))
        .bind(("app", spec.app.to_string()))
        .bind(("parent", parent))
        .bind(("title", spec.title.to_string()))
        .bind(("run", spec.run_ref.to_string()))
        .bind(("res", spec.resource_ref.to_string()))
        .bind(("owner", crate::scheduler::process_owner()))
        .bind(("at", Utc::now().to_rfc3339()))
        .await?
        .check()?;
    Ok(())
}

/// `JobHandle::phase` with progress counts. Returns whether Jobs → Cancel
/// asked the job (or its root) to stop.
pub async fn set_job_progress(
    repo: &Repository,
    id: &str,
    phase: &str,
    done: Option<i64>,
    total: Option<i64>,
) -> Result<bool> {
    if id.is_empty() {
        return Ok(false);
    }
    let mut r = repo
        .db()
        .query(
            "UPDATE $j SET phase = $p, progress_done = $done, progress_total = $total,
               heartbeat_at = <datetime>$at, updated_at = <datetime>$at
             RETURN VALUE cancel_requested",
        )
        .bind(("j", rid("job", id)))
        .bind(("p", phase.to_string()))
        .bind(("done", done))
        .bind(("total", total))
        .bind(("at", Utc::now().to_rfc3339()))
        .await?
        .check()?;
    let flags: Vec<bool> = r.take(0usize)?;
    Ok(flags.first().copied().unwrap_or(false))
}
