//! Article insight-refresh admission on SurrealDB (P3):
//! `insight_persist::schedule_article_refresh` with the refresh queueing
//! (`enqueue_refresh`) and supersession (`retire_replaced`) it needs.
//!
//! SurrealDB `task` rows carry no `input_ref`, `role_snapshot` or
//! `source_revision`, and `job` rows no `owner_scope`, so each refresh task
//! has an `insight_refresh_task` sidecar (keyed by task id) holding them;
//! `insight_supersession` (keyed by task id) is `insight_supersessions`.
//!
//! The admission is one transaction: intent, retirement check, prior-task
//! comparison, generation advance, job/task creation, the owner's
//! `refresh_job_id` and the supersession of older live tasks commit together.
//!
//! Still on the caller: the retained-evidence `PipelineInput`
//! (`insight_persist::article_input` reads SQLite evidence tables) and the
//! `insight.superseded` event (`events`, SQLite). The caller builds the
//! input before admission, so it is built even when no intent exists.

use surrealdb::engine::any::Any;
use surrealdb::types::SurrealValue;
use surrealdb::Surreal;

use super::insight::advance_generation;
use super::jobs::{create_job_in, create_task_in, rid, run, take, NewJob, NewTask};
use super::tx::{run_tx, RetryPolicy, Tx, TxError};
use crate::investigation::insight_persist::{article_details_eligible, supersession_reason};
use crate::investigation::insights::{input_key, PipelineInput};
use crate::provider::SettingsFile;

/// Refresh schema (idempotent). Apply after `jobs`, `insight` and `intel`.
pub const SCHEMA: &str = "DEFINE TABLE IF NOT EXISTS insight_refresh_task SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS job_id ON insight_refresh_task TYPE string;
DEFINE FIELD IF NOT EXISTS owner_scope ON insight_refresh_task TYPE string;
DEFINE FIELD IF NOT EXISTS input_ref ON insight_refresh_task TYPE string;
DEFINE FIELD IF NOT EXISTS role_snapshot ON insight_refresh_task TYPE string;
DEFINE FIELD IF NOT EXISTS input_hash ON insight_refresh_task TYPE string;
DEFINE FIELD IF NOT EXISTS source_revision ON insight_refresh_task TYPE string;
DEFINE FIELD IF NOT EXISTS created_at ON insight_refresh_task TYPE datetime;
DEFINE FIELD IF NOT EXISTS message ON insight_refresh_task TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS blocked_reason ON insight_refresh_task TYPE string DEFAULT '';
DEFINE INDEX IF NOT EXISTS insight_refresh_task_owner ON insight_refresh_task FIELDS owner_scope, created_at;
DEFINE TABLE IF NOT EXISTS insight_supersession SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS job_id ON insight_supersession TYPE string;
DEFINE FIELD IF NOT EXISTS owner_kind ON insight_supersession TYPE string;
DEFINE FIELD IF NOT EXISTS owner_id ON insight_supersession TYPE string;
DEFINE FIELD IF NOT EXISTS generation ON insight_supersession TYPE int;
DEFINE FIELD IF NOT EXISTS replaced_by_job ON insight_supersession TYPE string;
DEFINE FIELD IF NOT EXISTS replaced_by_task ON insight_supersession TYPE string;
DEFINE FIELD IF NOT EXISTS replaced_by_generation ON insight_supersession TYPE int;
DEFINE FIELD IF NOT EXISTS reason ON insight_supersession TYPE string;
DEFINE FIELD IF NOT EXISTS detail_json ON insight_supersession TYPE string;
DEFINE FIELD IF NOT EXISTS at ON insight_supersession TYPE string;";

/// An owner's refresh tasks, newest first; pinned to
/// `insight_refresh_task_owner`.
pub const OWNER_TASKS_SURQL: &str = "SELECT record::id(id) AS id, job_id, input_ref, role_snapshot,
       input_hash, source_revision,
       (SELECT VALUE state FROM ONLY type::record('task', record::id($parent.id))) ?? '' AS state
    FROM insight_refresh_task WHERE owner_scope = $scope ORDER BY created_at DESC";

pub async fn apply_schema(db: &Surreal<Any>) -> Result<(), TxError> {
    db.query(SCHEMA).await?.check()?;
    Ok(())
}

#[derive(Debug, Clone, SurrealValue)]
struct OwnerTask {
    id: String,
    job_id: String,
    input_ref: String,
    role_snapshot: String,
    input_hash: String,
    source_revision: String,
    state: String,
}

fn json_err(e: serde_json::Error) -> TxError {
    TxError::Abort(format!("refresh json: {e}"))
}

async fn owner_tasks(tx: &Tx, scope: &str) -> Result<Vec<OwnerTask>, TxError> {
    let mut r = run(
        tx,
        tx.query(OWNER_TASKS_SURQL)
            .bind(("scope", scope.to_string())),
    )
    .await?;
    take(&mut r, 0)
}

/// `insight_persist::enqueue_refresh` inside the caller's transaction:
/// creates the refresh job and task (unless the identical task exists), its
/// sidecar, and records the job on the owner state of this generation.
pub async fn enqueue_refresh_in(
    tx: &Tx,
    input: &mut PipelineInput,
    settings: &SettingsFile,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<String, TxError> {
    if input.surface == "atlas" || input.snapshot.owner_kind == "atlas" {
        return Err(TxError::Abort(
            "Atlas cycles cannot enqueue insight refreshes".into(),
        ));
    }
    if input.surface == "recon_report" {
        return Err(TxError::Abort(
            "Intel article report extraction is retired".into(),
        ));
    }
    input.snapshot.key = input_key(input, settings);
    let (kind, owner, generation) = (
        input.snapshot.owner_kind.clone(),
        input.snapshot.owner_id.clone(),
        input.snapshot.generation,
    );
    let job_id = format!("insight-refresh:{kind}:{owner}:{generation}");
    let task_id = format!("{job_id}:{}", input.snapshot.key);
    let mut r = run(
        tx,
        tx.query("RETURN [record::exists($j), record::exists($t)]")
            .bind(("j", rid("job", &job_id)))
            .bind(("t", rid("task", &task_id))),
    )
    .await?;
    let exists: Vec<bool> = take(&mut r, 0)?;
    if !exists.first().copied().unwrap_or(false) {
        create_job_in(
            tx,
            &NewJob {
                id: &job_id,
                kind: "insight_refresh",
                operation: "insight_refresh",
                parent: None,
                created_at: now,
            },
        )
        .await?;
    }
    if !exists.get(1).copied().unwrap_or(false) {
        create_task_in(
            tx,
            &NewTask {
                id: &task_id,
                job: &job_id,
                pool: crate::tasks::pool_for_operation("insight_refresh"),
                operation: "insight_refresh",
                priority: 20,
                deps: Vec::new(),
                input_hash: &input.snapshot.key,
                created_at: now,
            },
        )
        .await?;
        // Capture analytical routes/preparation, never provider/tool keys.
        let roles = SettingsFile {
            defaults: settings.defaults.clone(),
            nlp: settings.nlp.clone(),
            ..Default::default()
        };
        run(
            tx,
            tx.query(
                "CREATE type::record('insight_refresh_task', $t) SET job_id = $job,
                   owner_scope = $scope, input_ref = $input, role_snapshot = $roles,
                   input_hash = $hash, source_revision = $rev, created_at = <datetime>$now
                   RETURN NONE;",
            )
            .bind(("t", task_id.clone()))
            .bind(("job", job_id.clone()))
            .bind(("scope", format!("{kind}:{owner}")))
            .bind(("input", serde_json::to_string(input).map_err(json_err)?))
            .bind(("roles", serde_json::to_string(&roles).map_err(json_err)?))
            .bind(("hash", input.snapshot.key.clone()))
            .bind(("rev", generation.to_string()))
            .bind(("now", now.to_rfc3339())),
        )
        .await?;
    }
    run(
        tx,
        tx.query(
            "UPDATE type::record('insight_owner_state', [$k, $o]) SET refresh_job_id = $job
               WHERE desired_generation = $g RETURN NONE;",
        )
        .bind(("k", kind))
        .bind(("o", owner))
        .bind(("g", i64::from(generation)))
        .bind(("job", job_id)),
    )
    .await?;
    Ok(task_id)
}

/// `insight_persist::retire_replaced`: every other live refresh task of the
/// owner gets a supersession record; queued ones are superseded now (and
/// their job settles), running ones retire at their next fence.
#[allow(clippy::too_many_arguments)]
pub async fn retire_replaced_in(
    tx: &Tx,
    kind: &str,
    owner: &str,
    next: &PipelineInput,
    next_task: &str,
    reason: &str,
    detail: &serde_json::Value,
    now: &str,
) -> Result<Vec<String>, TxError> {
    let next_job = owner_tasks(tx, &format!("{kind}:{owner}"))
        .await?
        .into_iter()
        .find(|t| t.id == next_task)
        .map(|t| t.job_id)
        .ok_or_else(|| TxError::Abort(format!("refresh task {next_task} missing")))?;
    let mut retired = Vec::new();
    for t in owner_tasks(tx, &format!("{kind}:{owner}")).await? {
        if t.id == next_task || !matches!(t.state.as_str(), "queued" | "running") {
            continue;
        }
        let message = format!("Superseded · replaced by {next_job} · {reason}");
        run(
            tx,
            tx.query(
                "UPSERT type::record('insight_supersession', $t) SET job_id = $job,
                   owner_kind = $k, owner_id = $o, generation = $g, replaced_by_job = $nj,
                   replaced_by_task = $nt, replaced_by_generation = $ng, reason = $reason,
                   detail_json = $detail, at = $now RETURN NONE;
                 IF $queued {
                   UPDATE type::record('task', $t) SET state = 'superseded',
                     lease_epoch += 1, finished_at = time::now(), updated_at = time::now()
                     WHERE state = 'queued' RETURN NONE;
                   LET $states = (SELECT VALUE state FROM task WHERE job = $jr);
                   IF array::len(array::filter($states, |$s| $s IN ['queued', 'running'])) = 0 {
                     UPDATE $jr SET state = IF 'completed' IN $states THEN
                         (IF 'failed' IN $states THEN 'partial' ELSE 'completed' END)
                       ELSE IF 'failed' IN $states THEN 'failed' ELSE 'cancelled' END,
                       phase = $msg, finished_at = time::now(), updated_at = time::now()
                       RETURN NONE;
                   };
                 };",
            )
            .bind(("t", t.id.clone()))
            .bind(("job", t.job_id.clone()))
            .bind(("jr", rid("job", &t.job_id)))
            .bind(("k", kind.to_string()))
            .bind(("o", owner.to_string()))
            .bind(("g", t.source_revision.parse::<i64>().unwrap_or(0)))
            .bind(("nj", next_job.clone()))
            .bind(("nt", next_task.to_string()))
            .bind(("ng", i64::from(next.snapshot.generation)))
            .bind(("reason", reason.to_string()))
            .bind(("detail", detail.to_string()))
            .bind(("now", now.to_string()))
            .bind(("queued", t.state == "queued"))
            .bind(("msg", message)),
        )
        .await?;
        retired.push(t.id);
    }
    Ok(retired)
}

/// `insight_persist::supersession`: (replacement job, reason).
pub async fn supersession(
    db: &Surreal<Any>,
    task_id: &str,
) -> Result<Option<(String, String)>, TxError> {
    #[derive(SurrealValue)]
    struct Row {
        replaced_by_job: String,
        reason: String,
    }
    let mut r = db
        .query("SELECT replaced_by_job, reason FROM type::record('insight_supersession', $t)")
        .bind(("t", task_id.to_string()))
        .await?
        .check()?;
    let rows: Vec<Row> = r.take(0)?;
    Ok(rows
        .into_iter()
        .next()
        .map(|r| (r.replaced_by_job, r.reason)))
}

/// `insight_persist::schedule_article_refresh` on SurrealDB. `input` is the
/// article's retained-evidence input (generation 0), built by the caller;
/// `None` when it could not be built. Returns the live or new refresh task,
/// or `None` when no intent exists, the input is ineligible or retired.
pub async fn schedule_article_refresh(
    db: &Surreal<Any>,
    article_id: &str,
    input: Option<&PipelineInput>,
    settings: &SettingsFile,
    visit: bool,
) -> Result<Option<String>, TxError> {
    let out = run_tx(db, RetryPolicy::default(), |tx| async move {
        let now = chrono::Utc::now();
        let now_s = now.to_rfc3339();
        let mut r = run(
            &tx,
            tx.query(
                "LET $i = type::record('article_refresh_intent', $a);
                 IF $visit AND !record::exists($i) { CREATE $i SET requested_at = $now RETURN NONE };
                 RETURN record::exists($i);",
            )
            .bind(("a", article_id.to_string()))
            .bind(("visit", visit))
            .bind(("now", now_s.clone())),
        )
        .await?;
        if take::<Option<bool>>(&mut r, 2)? != Some(true) {
            return Ok(None);
        }
        let Some(input) = input.filter(|i| article_details_eligible(i)) else {
            return Ok(None);
        };
        let mut input = input.clone();
        input.snapshot.generation = 0;
        // The generation is excluded only for admission identity.
        let identity = input_key(&input, settings);
        let mut r = run(
            &tx,
            tx.query("RETURN record::exists(type::record('article_refresh_retirement', $key))")
                .bind(("key", super::codec::encode(&[article_id, &identity]))),
        )
        .await?;
        if take::<Option<bool>>(&mut r, 0)? == Some(true) {
            return Ok(None);
        }
        let scope = format!("article:{article_id}");
        let prior = owner_tasks(&tx, &scope).await?.into_iter().next();
        let mut reason = ("initial refresh".to_string(), serde_json::json!({}));
        if let Some(prior) = &prior {
            match serde_json::from_str::<PipelineInput>(&prior.input_ref) {
                Ok(mut p) => {
                    let captured: SettingsFile =
                        serde_json::from_str(&prior.role_snapshot).map_err(json_err)?;
                    let compatible = input_key(&p, &captured) == prior.input_hash;
                    p.snapshot.generation = 0;
                    if compatible && input_key(&p, &captured) == identity {
                        return Ok(Some(prior.id.clone()));
                    }
                    reason = if compatible {
                        supersession_reason(&p, &captured, &input, settings)
                    } else {
                        (
                            "preparation contract changed".into(),
                            serde_json::json!({"contract":"stored fingerprint is from an older contract"}),
                        )
                    };
                }
                Err(_) => {
                    reason = ("preparation contract changed".into(), serde_json::json!({}));
                }
            }
        }
        let generation = advance_generation(&tx, "article", article_id, &now_s).await?;
        input.snapshot.generation = u32::try_from(generation)
            .map_err(|_| TxError::Abort(format!("generation {generation} out of range")))?;
        let task = enqueue_refresh_in(&tx, &mut input, settings, now).await?;
        retire_replaced_in(
            &tx, "article", article_id, &input, &task, &reason.0, &reason.1, &now_s,
        )
        .await?;
        Ok(Some(task))
    })
    .await?;
    Ok(out.value)
}
