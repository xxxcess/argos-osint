//! Insight pipeline execution state on SurrealDB: the stage checkpoints
//! (`insight_stage_checkpoints`), the refresh execution row
//! (`insight_refresh_execution`: stage, window and withheld counters,
//! activity, retry epoch) and the dispatch fence (`validate_dispatch`) that
//! `insights::run_pipeline` reads and writes between model calls. With
//! these, a SurrealDB run of the pipeline never opens the legacy store file
//! (`investigation::exec_store`).
//!
//! Record keys: `insight_stage_checkpoint` by operation key,
//! `insight_refresh_execution` by task id.

use anyhow::{anyhow, ensure, Result};
use surrealdb::types::SurrealValue;

use super::repo::Repository;
use super::tx::{run_tx, RetryPolicy};
use crate::investigation::insights::PipelineInput;

pub const SCHEMA: &str = "DEFINE TABLE IF NOT EXISTS insight_stage_checkpoint SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS owner_kind ON insight_stage_checkpoint TYPE string;
DEFINE FIELD IF NOT EXISTS owner_id ON insight_stage_checkpoint TYPE string;
DEFINE FIELD IF NOT EXISTS generation ON insight_stage_checkpoint TYPE int;
DEFINE FIELD IF NOT EXISTS stage ON insight_stage_checkpoint TYPE string;
DEFINE FIELD IF NOT EXISTS window_id ON insight_stage_checkpoint TYPE string;
DEFINE FIELD IF NOT EXISTS state ON insight_stage_checkpoint TYPE string;
DEFINE FIELD IF NOT EXISTS output_json ON insight_stage_checkpoint TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS route_json ON insight_stage_checkpoint TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS error ON insight_stage_checkpoint TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS updated_at ON insight_stage_checkpoint TYPE string;
DEFINE INDEX IF NOT EXISTS insight_stage_checkpoint_owner ON insight_stage_checkpoint FIELDS owner_kind, owner_id, generation;
DEFINE TABLE IF NOT EXISTS insight_refresh_execution SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS stage ON insight_refresh_execution TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS windows_total ON insight_refresh_execution TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS withheld_count ON insight_refresh_execution TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS retry_epoch ON insight_refresh_execution TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS activity ON insight_refresh_execution TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS activity_at ON insight_refresh_execution TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS updated_at ON insight_refresh_execution TYPE string DEFAULT '';";

/// Apply [`SCHEMA`] (tests, until bootstrap registers it).
pub async fn apply_schema(repo: &Repository) -> Result<()> {
    repo.db().query(SCHEMA).await?.check()?;
    Ok(())
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// A stage checkpoint: state, output and error.
#[derive(Debug, Clone, PartialEq, Eq, SurrealValue)]
pub struct Checkpoint {
    pub state: String,
    pub output_json: String,
    pub error: String,
}

pub async fn checkpoint(repo: &Repository, key: &str) -> Result<Option<Checkpoint>> {
    let mut r = repo
        .db()
        .query(
            "SELECT state, output_json, error
               FROM ONLY type::record('insight_stage_checkpoint', $k)",
        )
        .bind(("k", key.to_string()))
        .await?
        .check()?;
    Ok(r.take(0usize)?)
}

/// `insight_execution::record_reuse`: a completed checkpoint from another
/// generation is copied under `<key>:reuse:<generation>`, once.
pub async fn record_reuse(repo: &Repository, key: &str, input: &PipelineInput) -> Result<()> {
    repo.db()
        .query(
            "LET $src = SELECT * FROM ONLY type::record('insight_stage_checkpoint', $k);
             IF $src != NONE AND $src.generation != $g AND $src.state = 'completed'
                AND !record::exists(type::record('insight_stage_checkpoint', $reuse)) {
               CREATE type::record('insight_stage_checkpoint', $reuse) SET
                 owner_kind = $src.owner_kind, owner_id = $src.owner_id, generation = $g,
                 stage = $src.stage, window_id = $src.window_id, state = $src.state,
                 output_json = $src.output_json, route_json = $src.route_json,
                 error = $src.error, updated_at = $now RETURN NONE;
             };",
        )
        .bind(("k", key.to_string()))
        .bind((
            "reuse",
            format!("{key}:reuse:{}", input.snapshot.generation),
        ))
        .bind(("g", i64::from(input.snapshot.generation)))
        .bind(("now", now()))
        .await?
        .check()?;
    Ok(())
}

/// Mark a checkpoint `running` (creating it); `clear_error` resets its
/// error, as the assessment stage does.
pub async fn begin(
    repo: &Repository,
    key: &str,
    input: &PipelineInput,
    stage: &str,
    window_id: &str,
    clear_error: bool,
) -> Result<()> {
    repo.db()
        .query(
            "UPSERT type::record('insight_stage_checkpoint', $k) SET
               owner_kind = owner_kind ?? $kind, owner_id = owner_id ?? $owner,
               generation = generation ?? $g, stage = stage ?? $stage,
               window_id = window_id ?? $window, state = 'running',
               error = IF $clear THEN '' ELSE error ?? '' END, updated_at = $now
             RETURN NONE;",
        )
        .bind(("k", key.to_string()))
        .bind(("kind", input.snapshot.owner_kind.clone()))
        .bind(("owner", input.snapshot.owner_id.clone()))
        .bind(("g", i64::from(input.snapshot.generation)))
        .bind(("stage", stage.to_string()))
        .bind(("window", window_id.to_string()))
        .bind(("clear", clear_error))
        .bind(("now", now()))
        .await?
        .check()?;
    Ok(())
}

/// Settle a checkpoint; `None` leaves output or route unchanged.
pub async fn finish(
    repo: &Repository,
    key: &str,
    state: &str,
    output: Option<String>,
    route: Option<String>,
    error: &str,
) -> Result<()> {
    repo.db()
        .query(
            "UPDATE type::record('insight_stage_checkpoint', $k) SET state = $state,
               output_json = $out ?? output_json, route_json = $route ?? route_json,
               error = $error, updated_at = $now RETURN NONE;",
        )
        .bind(("k", key.to_string()))
        .bind(("state", state.to_string()))
        .bind(("out", output))
        .bind(("route", route))
        .bind(("error", error.to_string()))
        .bind(("now", now()))
        .await?
        .check()?;
    Ok(())
}

/// Write a whole checkpoint (the memory-identity decision cache).
#[allow(clippy::too_many_arguments)]
pub async fn put(
    repo: &Repository,
    key: &str,
    input: &PipelineInput,
    stage: &str,
    window_id: &str,
    state: &str,
    output: &str,
    route: &str,
    error: &str,
) -> Result<()> {
    repo.db()
        .query(
            "UPSERT type::record('insight_stage_checkpoint', $k) SET owner_kind = $kind,
               owner_id = $owner, generation = $g, stage = $stage, window_id = $window,
               state = $state, output_json = $out, route_json = $route, error = $error,
               updated_at = $now RETURN NONE;",
        )
        .bind(("k", key.to_string()))
        .bind(("kind", input.snapshot.owner_kind.clone()))
        .bind(("owner", input.snapshot.owner_id.clone()))
        .bind(("g", i64::from(input.snapshot.generation)))
        .bind(("stage", stage.to_string()))
        .bind(("window", window_id.to_string()))
        .bind(("state", state.to_string()))
        .bind(("out", output.to_string()))
        .bind(("route", route.to_string()))
        .bind(("error", error.to_string()))
        .bind(("now", now()))
        .await?
        .check()?;
    Ok(())
}

/// `insight_assessment::task_id`: the owner's running refresh task for the
/// input's generation, empty when none.
pub async fn task_id(repo: &Repository, input: &PipelineInput) -> Result<String> {
    let mut r = repo
        .db()
        .query(
            "SELECT VALUE record::id(id) FROM insight_refresh_task
               WHERE owner_scope = $scope AND source_revision = $rev
                 AND type::record('task', record::id(id)).state = 'running'
               ORDER BY created_at DESC LIMIT 1",
        )
        .bind((
            "scope",
            format!("{}:{}", input.snapshot.owner_kind, input.snapshot.owner_id),
        ))
        .bind(("rev", input.snapshot.generation.to_string()))
        .await?
        .check()?;
    let ids: Vec<String> = r.take(0usize)?;
    Ok(ids.into_iter().next().unwrap_or_default())
}

/// `insight_execution::operation_id`: `<key>:retry:<epoch>`.
pub async fn operation_id(repo: &Repository, task_id: &str, key: &str) -> Result<String> {
    // An empty record id is invalid on a server: no task means epoch 0.
    if task_id.is_empty() {
        return Ok(format!("{key}:retry:0"));
    }
    let mut r = repo
        .db()
        .query("SELECT VALUE retry_epoch FROM ONLY type::record('insight_refresh_execution', $t)")
        .bind(("t", task_id.to_string()))
        .await?
        .check()?;
    let epoch: Option<i64> = r.take(0usize)?;
    Ok(format!("{key}:retry:{}", epoch.unwrap_or(0)))
}

/// `insight_execution::progress`: the running task's stage and counters; a
/// new stage resets the provisional activity.
pub async fn progress(
    repo: &Repository,
    input: &PipelineInput,
    stage: &str,
    windows: Option<usize>,
    withheld: Option<usize>,
) -> Result<()> {
    let id = task_id(repo, input).await?;
    if id.is_empty() {
        return Ok(());
    }
    let activity = if stage == "assessment" {
        "Waiting for decision"
    } else {
        ""
    };
    repo.db()
        .query(
            "UPSERT type::record('insight_refresh_execution', $t) SET stage = $stage,
               windows_total = $windows ?? windows_total ?? 0,
               withheld_count = $withheld ?? withheld_count ?? 0,
               updated_at = $now, activity = $activity, activity_at = $now RETURN NONE;",
        )
        .bind(("t", id))
        .bind(("stage", stage.to_string()))
        .bind(("windows", windows.map(|n| n as i64)))
        .bind(("withheld", withheld.map(|n| n as i64)))
        .bind(("activity", activity.to_string()))
        .bind(("now", now()))
        .await?
        .check()?;
    Ok(())
}

/// `insight_execution::record_activity`.
pub async fn record_activity(repo: &Repository, task_id: &str, label: &str) -> Result<()> {
    if task_id.is_empty() {
        return Ok(());
    }
    repo.db()
        .query(
            "UPDATE type::record('insight_refresh_execution', $t) SET activity = $label,
               activity_at = $now RETURN NONE;",
        )
        .bind(("t", task_id.to_string()))
        .bind(("label", label.to_string()))
        .bind(("now", now()))
        .await?
        .check()?;
    Ok(())
}

/// `insight_execution::activity`: empty when idle or unknown.
pub async fn activity(repo: &Repository, task_id: &str) -> Result<String> {
    if task_id.is_empty() {
        return Ok(String::new());
    }
    let mut r = repo
        .db()
        .query("SELECT VALUE activity FROM ONLY type::record('insight_refresh_execution', $t)")
        .bind(("t", task_id.to_string()))
        .await?
        .check()?;
    Ok(r.take::<Option<String>>(0usize)?.unwrap_or_default())
}

/// `insight_persist::validate_dispatch`: an article refresh may dispatch a
/// model call only while its evidence is live, its generation is the
/// desired one and its refresh task still holds an unexpired lease.
pub async fn validate_dispatch(repo: &Repository, input: &PipelineInput) -> Result<()> {
    if input.snapshot.owner_kind != "article" {
        return Ok(());
    }
    let snapshot = input.snapshot.clone();
    run_tx(repo.db(), RetryPolicy::default(), |tx| {
        let snapshot = snapshot.clone();
        async move { super::live_evidence::validate_in(&tx, &snapshot).await }
    })
    .await
    .map_err(|e| anyhow!("{e}"))?;
    let mut r = repo
        .db()
        .query(
            "LET $state = SELECT desired_generation, refresh_job_id
               FROM ONLY type::record('insight_owner_state', ['article', $o]);
             LET $job = IF $state.refresh_job_id THEN type::record('job', $state.refresh_job_id) ELSE NONE END;
             RETURN {
               generation: $state.desired_generation ?? -1,
               leased: array::len(SELECT id FROM task WHERE $job != NONE AND job = $job
                 AND operation = 'insight_refresh' AND state = 'running'
                 AND lease_until != NONE AND lease_until > time::now()) > 0
             };",
        )
        .bind(("o", input.snapshot.owner_id.clone()))
        .await?
        .check()?;
    #[derive(SurrealValue)]
    struct Fence {
        generation: i64,
        leased: bool,
    }
    let fence: Option<Fence> = r.take(1usize)?;
    let fence = fence.ok_or_else(|| anyhow!("article refresh superseded before dispatch"))?;
    ensure!(
        fence.generation == i64::from(input.snapshot.generation),
        "article refresh superseded before dispatch"
    );
    ensure!(fence.leased, "article refresh lease lost before dispatch");
    Ok(())
}

#[derive(SurrealValue)]
struct EvidenceRow {
    text: Option<String>,
    derivation: Option<String>,
    kind: Option<String>,
    assessment: Option<String>,
    owner_kind: Option<String>,
    owner_id: Option<String>,
    entities_json: Option<String>,
    evidence: Vec<String>,
}

/// `brain_query::load_memory_evidence`: the memory's newest live
/// derivation with its snapshot owner, entities and the retained evidence
/// whose artifact text still validates. `None` without a live derivation
/// (SurrealDB has no pre-derivation legacy memories to fall back to).
pub async fn load_memory_evidence(
    repo: &Repository,
    memory_id: &str,
) -> Result<Option<crate::brain_query::MemoryEvidence>> {
    use crate::investigation::insights::{EvidenceLocation, EvidenceRef, RetainedEvidence};
    let mut r = repo
        .db()
        .query(
            "LET $d = (SELECT record::id(id) AS id, insight_kind, assessment_json, snapshot_id,
                 type::record('insight_snapshot', snapshot_id).generation AS generation
               FROM insight_derivation WHERE memory_id = $m
                 AND status NOT IN ['deleted', 'retired']
               ORDER BY generation DESC LIMIT 1)[0];
             LET $s = IF $d != NONE THEN type::record('insight_snapshot', $d.snapshot_id) ELSE NONE END;
             RETURN {
               text: type::record('memory', $m).text,
               derivation: $d.id, kind: $d.insight_kind, assessment: $d.assessment_json,
               owner_kind: $s.owner_kind, owner_id: $s.owner_id, entities_json: $s.entities_json,
               evidence: IF $d != NONE THEN array::distinct(SELECT VALUE evidence_json
                 FROM insight_derivation_evidence WHERE derivation_id = $d.id
                 ORDER BY source_id, source_revision) ELSE [] END
             };",
        )
        .bind(("m", memory_id.to_string()))
        .await?
        .check()?;
    let row: Option<EvidenceRow> = r.take(2usize)?;
    let Some(row) = row else { return Ok(None) };
    let (Some(text), Some(_), Some(kind), Some(assessment), Some(owner_kind), Some(owner_id)) = (
        row.text,
        row.derivation,
        row.kind,
        row.assessment,
        row.owner_kind,
        row.owner_id,
    ) else {
        return Ok(None);
    };
    let entities = serde_json::from_str(row.entities_json.as_deref().unwrap_or("[]"))?;
    let mut evidence = Vec::new();
    for json in row.evidence {
        let reference: EvidenceRef = serde_json::from_str(&json)?;
        let mut a = repo
            .db()
            .query("SELECT VALUE text FROM ONLY type::record('nlp_artifact', $h)")
            .bind(("h", reference.artifact_hash.clone()))
            .await?
            .check()?;
        let Some(text) = a.take::<Option<String>>(0usize)? else {
            continue;
        };
        let native = if matches!(reference.location, EvidenceLocation::JsonPointer(_)) {
            serde_json::from_str(&text).ok()
        } else {
            None
        };
        let retained = RetainedEvidence {
            reference,
            text,
            native,
            substantive: true,
            relevant: true,
            complete_story: false,
            known_truncation: false,
        };
        if retained.validate_ref(&retained.reference) {
            evidence.push(retained);
        }
    }
    Ok(Some(crate::brain_query::MemoryEvidence {
        memory_id: memory_id.into(),
        revision: super::memory::memory_revision(&text),
        kind,
        assessment,
        owner_kind,
        owner_id,
        entities,
        evidence,
    }))
}

/// The job id and owner scope (`resource_ref`) of `task_id`, for its
/// stage and activity events; `None` when the task is unknown.
pub async fn task_event_scope(
    repo: &Repository,
    task_id: &str,
) -> Result<Option<(String, String)>> {
    if task_id.is_empty() {
        return Ok(None);
    }
    let mut r = repo
        .db()
        .query(
            "SELECT VALUE [record::id(job), job.resource_ref ?? '']
             FROM type::record('task', $t)",
        )
        .bind(("t", task_id.to_string()))
        .await?
        .check()?;
    let rows: Vec<Vec<String>> = r.take(0usize)?;
    Ok(rows
        .into_iter()
        .next()
        .and_then(|row| Some((row.first()?.clone(), row.get(1)?.clone()))))
}

/// Record one stage or activity event of `task_id` in the event log
/// (`argos_event`), filling its job (and, with `owner_scope`, the owner
/// scope and app) from the task. A task-less event is not recorded.
pub async fn record_task_event(
    repo: &Repository,
    task_id: &str,
    owner_scope: bool,
    mut event: crate::events::NewEvent,
) -> Result<()> {
    if task_id.is_empty() {
        return Ok(());
    }
    let Some((job, owner)) = task_event_scope(repo, task_id).await? else {
        return Ok(());
    };
    event.job_id = job;
    if owner_scope {
        event.app = if owner.starts_with("recon:") {
            "recon"
        } else {
            "intel"
        }
        .into();
        event.resource_ref = owner;
    }
    super::event_log::record_event(repo.db(), &event).await?;
    Ok(())
}
