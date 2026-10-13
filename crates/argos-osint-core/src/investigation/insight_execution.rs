//! Bounded checkpoint slices and explicit retry epochs, independent of immutable work identity.
use super::insights::PipelineInput;
use anyhow::{ensure, Result};
use rusqlite::{params, Connection, OptionalExtension};

tokio::task_local! { static SLICE_REMAINING: std::cell::Cell<usize>; }

#[derive(Debug)]
pub(super) struct SliceYield;
impl std::fmt::Display for SliceYield {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("insight slice complete; queued continuation")
    }
}
impl std::error::Error for SliceYield {}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Progress {
    pub task_id: String,
    pub job_id: String,
    pub retry_epoch: u32,
    pub stage: String,
    pub state: String,
    pub windows_done: usize,
    pub windows_total: usize,
    pub batches_done: usize,
    pub batches_total: usize,
    pub withheld_count: usize,
    pub route: String,
    pub error: String,
    pub retryable: bool,
    pub index_status: String,
    /// Live throttled activity label for the current attempt (no model text).
    pub activity: String,
    /// Seconds since the last activity update, when known.
    pub activity_age_secs: Option<u64>,
    /// Per-artifact progression (Claims, Insights, Links) for this task.
    #[serde(default)]
    pub artifacts: Vec<ArtifactProgress>,
}

/// One analytical artifact's own progression inside an insight refresh.
///
/// Claims come from the extraction stage plus claim-kind assessment batches.
/// Insights (inferences) and Links are produced by the *same* synthesis model
/// call (one `InferenceWindow` per evidence window), then assessed in separate
/// role-partitioned batches. Publication is atomic for all three, so
/// Published/Indexed are shared.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ArtifactProgress {
    /// "Claims", "Insights" or "Links".
    pub artifact: String,
    /// Canonical step (see [`status_step`]) or a waiting/validated note.
    pub step: String,
    /// True while this artifact owns the in-flight model request.
    pub live: bool,
    /// Live activity label (empty unless `live`).
    pub activity: String,
}

/// Checkpoint facts needed to attribute the current stage to an artifact.
#[derive(Clone, Debug, Default)]
pub struct ArtifactFacts {
    /// Any synthesis (inference/reconciliation) checkpoint exists.
    pub synthesis_started: bool,
    /// Assessment checkpoints: (artifact, state).
    pub assessments: Vec<(&'static str, String)>,
}

fn assessment_artifact(window_id: &str) -> Option<&'static str> {
    match window_id.split(',').next()?.split(':').next()? {
        "claim" => Some("Claims"),
        "inference" => Some("Insights"),
        "link" => Some("Links"),
        _ => None,
    }
}

/// Pure attribution of a task's progress to Claims / Insights / Links.
pub fn artifact_steps(p: &Progress, facts: &ArtifactFacts) -> Vec<ArtifactProgress> {
    let names = ["Claims", "Insights", "Links"];
    let shared = status_step(
        &p.state,
        &p.stage,
        &p.activity,
        p.activity_age_secs,
        p.index_status.starts_with("Indexed"),
    );
    let terminal = matches!(
        p.state.as_str(),
        "queued" | "retry_scheduled" | "failed" | "cancelled" | "superseded" | "completed"
    ) || p.stage == "published";
    // An artifact whose own checkpoints all completed keeps that state when
    // the parent refresh later fails or is cancelled; only the artifacts that
    // did not finish inherit the parent terminal state.
    let validated = |name: &str| {
        let mine: Vec<&String> = facts
            .assessments
            .iter()
            .filter(|(a, _)| *a == name)
            .map(|(_, s)| s)
            .collect();
        !mine.is_empty() && mine.iter().all(|s| *s == "completed")
    };
    if matches!(p.state.as_str(), "failed" | "cancelled") && p.stage != "published" {
        return names
            .iter()
            .map(|n| ArtifactProgress {
                artifact: (*n).into(),
                step: if validated(n) {
                    format!("Validated · not published ({})", p.state)
                } else {
                    shared.clone()
                },
                ..Default::default()
            })
            .collect();
    }
    if terminal || p.stage == "queued" {
        return names
            .iter()
            .map(|n| ArtifactProgress {
                artifact: (*n).into(),
                step: shared.clone(),
                ..Default::default()
            })
            .collect();
    }
    let running = facts
        .assessments
        .iter()
        .rev()
        .find(|(_, state)| state == "running")
        .map(|(a, _)| *a);
    let live: Vec<&str> = match p.stage.as_str() {
        "extraction" => vec!["Claims"],
        "assessment" => vec![running.unwrap_or(if facts.synthesis_started {
            "Insights"
        } else {
            "Claims"
        })],
        // One synthesis call yields both Insights and Links.
        _ => vec!["Insights", "Links"],
    };
    names
        .iter()
        .map(|name| {
            if live.contains(name) {
                return ArtifactProgress {
                    artifact: (*name).into(),
                    step: shared.clone(),
                    live: true,
                    activity: p.activity.clone(),
                };
            }
            let mine: Vec<&String> = facts
                .assessments
                .iter()
                .filter(|(a, _)| a == name)
                .map(|(_, s)| s)
                .collect();
            let step = if *name == "Claims" {
                if facts.synthesis_started
                    || mine.iter().all(|s| *s == "completed") && !mine.is_empty()
                {
                    "Validated · awaiting publication"
                } else {
                    "Waiting for decision"
                }
            } else if !facts.synthesis_started {
                "Queued · after claims"
            } else if !mine.is_empty() && mine.iter().all(|s| *s == "completed") {
                "Validated · awaiting publication"
            } else if p.stage == "assessment" {
                "Queued · awaiting assessment"
            } else {
                "Waiting for output"
            };
            ArtifactProgress {
                artifact: (*name).into(),
                step: step.into(),
                ..Default::default()
            }
        })
        .collect()
}

fn artifact_facts(conn: &Connection, input: &PipelineInput) -> Result<ArtifactFacts> {
    let mut facts = ArtifactFacts::default();
    let mut stmt = conn.prepare("SELECT stage,window_id,state FROM insight_stage_checkpoints WHERE owner_kind=?1 AND owner_id=?2 AND generation=?3 ORDER BY updated_at,rowid")?;
    for row in stmt.query_map(
        params![
            input.snapshot.owner_kind,
            input.snapshot.owner_id,
            input.snapshot.generation
        ],
        |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        },
    )? {
        let (stage, window, state) = row?;
        match stage.as_str() {
            "extraction" => {}
            "assessment" => {
                if let Some(a) = assessment_artifact(&window) {
                    facts.assessments.push((a, state));
                }
            }
            _ => facts.synthesis_started = true,
        }
    }
    Ok(facts)
}

/// Canonical Intel/Jobs status step for insight work:
/// Queued → Waiting for output → Receiving output → Validating → Published → Indexed.
/// Decision work shows "Waiting for decision"; stalls, retry waits and failures
/// are explicit. Headers alone never count as model activity.
pub fn status_step(
    task_state: &str,
    stage: &str,
    activity: &str,
    activity_age_secs: Option<u64>,
    indexed: bool,
) -> String {
    match task_state {
        "queued" | "retry_scheduled" => return "Queued".into(),
        "failed" => return "Failed".into(),
        "cancelled" => return "Cancelled".into(),
        "superseded" => return "Superseded".into(),
        _ => {}
    }
    if stage == "published" || task_state == "completed" {
        return if indexed { "Indexed" } else { "Published" }.into();
    }
    let head = activity.split(" · ").next().unwrap_or("").trim();
    let base = match head {
        "" if stage == "assessment" => "Waiting for decision".to_string(),
        "" => "Waiting for output".to_string(),
        "Attempt failed" => activity.to_string(),
        h if h.starts_with("Retry wait") => activity.to_string(),
        h => h.to_string(),
    };
    // Explicit stall: no activity update for longer than the streamed
    // first-output (180s) / inactivity (90s) policy allows.
    let limit = if base == "Receiving output" { 90 } else { 180 };
    match activity_age_secs {
        Some(age) if age > limit && base != "Validating" && !base.starts_with("Retry") => {
            format!("Stalled · no model activity for {age}s")
        }
        _ => base,
    }
}

impl Progress {
    pub fn lines(&self) -> Vec<String> {
        let step = status_step(
            &self.state,
            &self.stage,
            &self.activity,
            self.activity_age_secs,
            self.index_status.starts_with("Indexed"),
        );
        // Intentional retirement reads as such, never as an extraction error.
        let step = if self.state == "superseded" && self.error.starts_with("Superseded") {
            self.error.clone()
        } else {
            step
        };
        let mut lines = vec![
            format!("{step} · {} · epoch {}", self.state, self.retry_epoch),
            format!(
                "Stage: {} · windows {}/{} · batches {}/{}",
                self.stage,
                self.windows_done,
                self.windows_total,
                self.batches_done,
                self.batches_total
            ),
            format!("Withheld: {} · {}", self.withheld_count, self.index_status),
        ];
        if !self.activity.is_empty() && self.state == "running" {
            lines.push(match self.activity_age_secs {
                Some(age) => format!("Activity: {} · {age}s ago", self.activity),
                None => format!("Activity: {}", self.activity),
            });
        }
        if !self.route.is_empty() {
            lines.push(self.route.clone());
        }
        if !self.error.is_empty() && self.state != "superseded" {
            lines.push(format!("Extraction error: {}", self.error));
        }
        lines
    }
}

pub fn task_progress(conn: &Connection, task_id: &str) -> Result<Option<Progress>> {
    let row:Option<(String,String,String,String,u32,usize,usize,String,String)>=conn.query_row("SELECT t.job_id,t.state,t.input_ref,COALESCE(e.stage,'queued'),COALESCE(e.retry_epoch,0),COALESCE(e.windows_total,0),COALESCE(e.withheld_count,0),COALESCE(e.activity,''),COALESCE(e.activity_at,'') FROM argos_tasks t LEFT JOIN insight_refresh_execution e ON e.task_id=t.id WHERE t.id=?1 AND t.operation='insight_refresh'",[task_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?))).optional()?;
    let Some((job, state, json, stage, epoch, total, withheld, activity, activity_at)) = row else {
        return Ok(None);
    };
    let input: PipelineInput = serde_json::from_str(&json)?;
    let (done,batches_done,batches_total,exhausted):(usize,usize,usize,usize)=conn.query_row("SELECT COALESCE(SUM(stage='extraction' AND state='completed'),0),COALESCE(SUM(stage='assessment' AND state='completed'),0),COALESCE(SUM(stage='assessment'),0),COALESCE(SUM(state='exhausted'),0) FROM insight_stage_checkpoints WHERE owner_kind=?1 AND owner_id=?2 AND generation=?3",params![input.snapshot.owner_kind,input.snapshot.owner_id,input.snapshot.generation],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
    let error:String=conn.query_row("SELECT COALESCE((SELECT error FROM insight_stage_checkpoints WHERE owner_kind=?1 AND owner_id=?2 AND generation=?3 AND state='exhausted' ORDER BY updated_at DESC LIMIT 1),(SELECT error_message FROM argos_tasks WHERE id=?4),'')",params![input.snapshot.owner_kind,input.snapshot.owner_id,input.snapshot.generation,task_id],|r|r.get(0))?;
    let route:Option<(String,String,u32,String)>=conn.query_row("SELECT a.provider,a.model,a.attempt,a.outcome FROM recon_model_attempts a JOIN recon_model_operations o ON o.id=a.operation_id WHERE o.task_id=?1 ORDER BY a.started_at DESC,a.rowid DESC LIMIT 1",[task_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
    let mut pending = 0;
    let mut count = 0;
    let mut stmt=conn.prepare("SELECT m.text,s.revision FROM insight_derivations d JOIN insight_owner_state h ON h.active_snapshot_id=d.snapshot_id JOIN memories m ON m.id=d.memory_id LEFT JOIN argos_memory_index_state s ON s.memory_id=m.id WHERE h.owner_kind=?1 AND h.owner_id=?2")?;
    for row in stmt.query_map(
        params![input.snapshot.owner_kind, input.snapshot.owner_id],
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)),
    )? {
        let (text, revision) = row?;
        count += 1;
        if revision.as_deref() != Some(crate::nlp::cache::hash(&text).as_str()) {
            pending += 1;
        }
    }
    let index_status = if count == 0 {
        "Index: no published memories".into()
    } else if pending == 0 {
        format!("Indexed: {count} current revisions acknowledged")
    } else {
        format!("Index pending: {pending}/{count}")
    };
    Ok(Some(Progress {
        task_id: task_id.into(),
        job_id: job,
        retry_epoch: epoch,
        stage,
        state: state.clone(),
        windows_done: done,
        windows_total: total,
        batches_done,
        batches_total,
        withheld_count: withheld,
        route: route
            .map(|(p, m, a, o)| format!("Route: {p}/{m} · attempt {a} · {o}"))
            .unwrap_or_else(|| {
                if state == "queued" {
                    "Queued · awaiting provider admission".into()
                } else {
                    "Provider wait or route not dispatched".into()
                }
            }),
        error,
        retryable: (state == "failed" || state == "completed")
            && exhausted > 0
            && validate_retry_admission(conn, &input, task_id).is_ok(),
        index_status,
        activity_age_secs: chrono::DateTime::parse_from_rfc3339(&activity_at)
            .ok()
            .map(|at| {
                (chrono::Utc::now() - at.with_timezone(&chrono::Utc))
                    .num_seconds()
                    .max(0) as u64
            }),
        activity,
        artifacts: Vec::new(),
    })
    .map(|mut p| {
        let facts = artifact_facts(conn, &input).unwrap_or_default();
        p.artifacts = artifact_steps(&p, &facts);
        p
    }))
}

fn validate_retry_admission(conn: &Connection, input: &PipelineInput, task_id: &str) -> Result<()> {
    let job: String = conn.query_row(
        "SELECT job_id FROM argos_tasks WHERE id=?1",
        [task_id],
        |r| r.get(0),
    )?;
    crate::job_cleanup::ensure_admissible(conn, &job, &chrono::Utc::now().to_rfc3339())?;
    super::insight_persist::validate_retry(conn, input)
}

pub fn owner_progress(conn: &Connection, kind: &str, owner: &str) -> Result<Option<Progress>> {
    let id:Option<String>=conn.query_row("SELECT t.id FROM insight_owner_state s JOIN argos_tasks t ON t.job_id=s.refresh_job_id WHERE s.owner_kind=?1 AND s.owner_id=?2 AND t.operation='insight_refresh' ORDER BY t.created_at DESC LIMIT 1",params![kind,owner],|r|r.get(0)).optional()?;
    id.map(|id| task_progress(conn, &id))
        .transpose()
        .map(Option::flatten)
}

pub(super) async fn slice<T>(future: impl std::future::Future<Output = T>) -> T {
    SLICE_REMAINING.scope(std::cell::Cell::new(2), future).await
}

/// Called only after a checkpoint miss, never on warm replay.
pub(super) fn consume() -> Result<()> {
    SLICE_REMAINING
        .try_with(|remaining| {
            ensure!(remaining.get() > 0, SliceYield);
            remaining.set(remaining.get() - 1);
            Ok(())
        })
        .unwrap_or(Ok(()))
}

pub(super) fn operation_id(conn: &Connection, task_id: &str, key: &str) -> Result<String> {
    let epoch: Option<u32> = conn
        .query_row(
            "SELECT retry_epoch FROM insight_refresh_execution WHERE task_id=?1",
            [task_id],
            |r| r.get(0),
        )
        .optional()?;
    Ok(format!("{key}:retry:{}", epoch.unwrap_or(0)))
}

pub(super) fn record_reuse(conn: &Connection, key: &str, input: &PipelineInput) -> Result<()> {
    conn.execute("INSERT OR IGNORE INTO insight_stage_checkpoints(operation_key,owner_kind,owner_id,generation,stage,window_id,state,output_json,route_json,error,updated_at) SELECT ?1,owner_kind,owner_id,?2,stage,window_id,state,output_json,route_json,error,?3 FROM insight_stage_checkpoints WHERE operation_key=?4 AND generation<>?2 AND state='completed'",params![format!("{key}:reuse:{}",input.snapshot.generation),input.snapshot.generation,chrono::Utc::now().to_rfc3339(),key])?;
    Ok(())
}

pub(super) fn progress(
    conn: &Connection,
    input: &PipelineInput,
    stage: &str,
    windows: Option<usize>,
    withheld: Option<usize>,
) -> Result<()> {
    let id = super::insight_assessment::task_id(conn, input)?;
    if id.is_empty() {
        return Ok(());
    }
    // A new stage resets the provisional activity (counters never carry over).
    let activity = if stage == "assessment" {
        "Waiting for decision"
    } else {
        ""
    };
    conn.execute("INSERT INTO insight_refresh_execution(task_id,stage,windows_total,withheld_count,updated_at,activity,activity_at) VALUES(?1,?2,COALESCE(?3,0),COALESCE(?4,0),?5,?6,?5) ON CONFLICT(task_id) DO UPDATE SET stage=excluded.stage,windows_total=COALESCE(?3,windows_total),withheld_count=COALESCE(?4,withheld_count),updated_at=excluded.updated_at,activity=excluded.activity,activity_at=excluded.activity_at",
        params![id,stage,windows.map(|n|n as i64),withheld.map(|n|n as i64),chrono::Utc::now().to_rfc3339(),activity])?;
    Ok(())
}

/// Status label for one typed model-execution event, or `None` when the event
/// must not touch durable state (heartbeats, provisional text).
pub fn activity_label(event: &crate::recon::model_exec::ModelExecEvent) -> Option<String> {
    use crate::provider_attempt::AttemptProgress as P;
    use crate::recon::model_exec::ModelExecEvent as E;
    Some(match event {
        E::AttemptStart { attempt, model, .. } => {
            format!("Waiting for output · attempt {attempt} · {model}")
        }
        E::AttemptReset { attempt, .. } => format!("Waiting for output · attempt {attempt}"),
        E::Progress {
            progress,
            attempt,
            model,
            ..
        } => {
            match progress {
                P::Dispatched | P::HeadersReceived { .. } => {
                    format!("Waiting for output · attempt {attempt} · {model}")
                }
                P::FirstModelOutput => format!("Receiving output · attempt {attempt} · {model}"),
                P::Receiving {
                    answer_chars,
                    reasoning_chars,
                } => {
                    if *answer_chars == 0 {
                        format!("Receiving output · attempt {attempt} · thinking ({reasoning_chars} chars)")
                    } else {
                        format!("Receiving output · attempt {attempt} · {answer_chars} chars")
                    }
                }
                P::StreamComplete { .. } => "Validating".into(),
                P::Heartbeat => return None,
            }
        }
        E::AttemptFinish {
            outcome, category, ..
        } => {
            if outcome == "ok" {
                "Validating".into()
            } else {
                format!(
                    "Attempt failed · {}",
                    category.as_deref().unwrap_or(outcome)
                )
            }
        }
        E::RetryStatus {
            wait, next_attempt, ..
        } => {
            format!(
                "Retry wait {}s · next attempt {next_attempt}",
                wait.as_secs()
            )
        }
        E::ProvisionalDelta { .. } | E::FinalReplacement { .. } => return None,
    })
}

/// Logs message for activity transitions worth a durable event (attempt
/// start, first model output, completion, failure, retry). Never per delta.
pub fn activity_log_message(event: &crate::recon::model_exec::ModelExecEvent) -> Option<String> {
    use crate::provider_attempt::AttemptProgress as P;
    use crate::recon::model_exec::ModelExecEvent as E;
    match event {
        E::AttemptStart {
            attempt,
            provider,
            model,
            transport,
            ..
        } => Some(format!(
            "attempt {attempt} dispatched · {provider}/{model} · {transport} · waiting for output"
        )),
        E::Progress {
            progress: P::FirstModelOutput,
            attempt,
            ..
        } => Some(format!("attempt {attempt} receiving model output")),
        E::Progress {
            progress: P::StreamComplete { finish_reason },
            attempt,
            ..
        } => Some(format!(
            "attempt {attempt} stream complete ({}) · validating",
            finish_reason.as_deref().unwrap_or("done")
        )),
        E::AttemptFinish {
            attempt,
            outcome,
            category,
            chars,
            ..
        } => Some(format!(
            "attempt {attempt} {outcome}{} · {chars} chars",
            category
                .as_deref()
                .map(|c| format!(" ({c})"))
                .unwrap_or_default()
        )),
        E::RetryStatus {
            wait, next_attempt, ..
        } => Some(format!(
            "retry wait {}s before attempt {next_attempt}",
            wait.as_secs()
        )),
        _ => None,
    }
}

/// Durable Logs event for one activity transition of an insight task.
pub fn activity_event(conn: &Connection, task_id: &str, message: &str) -> Result<()> {
    if task_id.is_empty() {
        return Ok(());
    }
    let (job, owner): (String, String) = conn.query_row(
        "SELECT t.job_id,COALESCE(j.owner_scope,'') FROM argos_tasks t LEFT JOIN argos_jobs j ON j.id=t.job_id WHERE t.id=?1",
        [task_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let app = if owner.starts_with("recon:") {
        "recon"
    } else {
        "intel"
    };
    crate::events::record_event(
        conn,
        &crate::events::NewEvent {
            severity: Some(crate::events::Severity::Info),
            app: app.into(),
            event_type: "insight.activity".into(),
            message: message.into(),
            job_id: job,
            task_id: task_id.into(),
            resource_ref: owner,
            ..Default::default()
        },
    )?;
    Ok(())
}

/// Durable, throttled activity write (one row update, never per token).
pub fn record_activity(conn: &Connection, task_id: &str, label: &str) -> Result<()> {
    if task_id.is_empty() {
        return Ok(());
    }
    conn.execute(
        "UPDATE insight_refresh_execution SET activity=?2,activity_at=?3 WHERE task_id=?1",
        params![task_id, label, chrono::Utc::now().to_rfc3339()],
    )?;
    Ok(())
}

/// Current activity label for a task, empty when idle/unknown.
pub fn activity(conn: &Connection, task_id: &str) -> String {
    conn.query_row(
        "SELECT activity FROM insight_refresh_execution WHERE task_id=?1",
        [task_id],
        |r| r.get(0),
    )
    .unwrap_or_default()
}

/// Admits exactly one bounded epoch for failed/partially published recoverable work.
pub(super) fn stage_event(
    conn: &Connection,
    input: &PipelineInput,
    task_id: &str,
    stage: &str,
    outcome: &str,
    message: &str,
    details: serde_json::Value,
) -> Result<()> {
    if task_id.is_empty() {
        return Ok(());
    }
    let job: String = conn.query_row(
        "SELECT job_id FROM argos_tasks WHERE id=?1",
        [task_id],
        |r| r.get(0),
    )?;
    crate::events::record_event(
        conn,
        &crate::events::NewEvent {
            severity: Some(match outcome {
                "failed" => crate::events::Severity::Error,
                "withheld" => crate::events::Severity::Warn,
                _ => crate::events::Severity::Info,
            }),
            app: if input.snapshot.owner_kind == "article" {
                "intel"
            } else {
                &input.snapshot.owner_kind
            }
            .into(),
            event_type: format!("insight.stage.{outcome}"),
            message: format!("{stage}: {message}"),
            details: details.to_string(),
            job_id: job,
            task_id: task_id.into(),
            run_id: if input.snapshot.owner_kind == "recon" {
                input.snapshot.owner_id.clone()
            } else {
                String::new()
            },
            resource_ref: input.snapshot.owner_id.clone(),
            ..Default::default()
        },
    )?;
    Ok(())
}

/// Admits exactly one bounded epoch for failed/partially published recoverable work.
/// Reopening and startup recovery never call this action.
pub fn retry_extraction(conn: &Connection, task_id: &str) -> Result<bool> {
    let now = chrono::Utc::now().to_rfc3339();
    let row:Option<(String,String,String,String)>=conn.query_row("SELECT job_id,state,input_ref,role_snapshot FROM argos_tasks WHERE id=?1 AND operation='insight_refresh'",[task_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
    let Some((job, state, json, captured)) = row else {
        return Ok(false);
    };
    crate::job_cleanup::ensure_admissible(conn, &job, &now)?;
    let input: PipelineInput = serde_json::from_str(&json)?;
    super::insight_persist::validate_retry(conn, &input)?;
    let exhausted:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM insight_stage_checkpoints WHERE owner_kind=?1 AND owner_id=?2 AND generation=?3 AND state='exhausted')",params![input.snapshot.owner_kind,input.snapshot.owner_id,input.snapshot.generation],|r|r.get(0))?;
    if !exhausted || !matches!(state.as_str(), "failed" | "completed") {
        return Ok(false);
    }
    let captured: crate::provider::SettingsFile = serde_json::from_str(&captured)?;
    // Route changes replace admission through the normal owner scheduling path.
    // Explicit retry itself operates on the captured immutable contract.
    let live_key = super::insights::input_key(&input, &captured);
    let tx = conn.unchecked_transaction()?;
    let n=tx.execute("UPDATE argos_tasks SET state='queued',max_attempts=attempts+1,next_eligible_at='',lease_owner='',lease_until='',error_category='',error_message='',blocked_reason='',finished_at='',updated_at=?2 WHERE id=?1 AND state=?3",params![task_id,now,state])?;
    if n == 0 {
        return Ok(false);
    }
    tx.execute("UPDATE insight_stage_checkpoints SET state='retry_admitted',updated_at=?4 WHERE owner_kind=?1 AND owner_id=?2 AND generation=?3 AND state IN ('exhausted','cancelled')",params![input.snapshot.owner_kind,input.snapshot.owner_id,input.snapshot.generation,now])?;
    tx.execute("INSERT INTO insight_refresh_execution(task_id,retry_epoch,stage,updated_at) VALUES(?1,1,'queued',?2) ON CONFLICT(task_id) DO UPDATE SET retry_epoch=retry_epoch+1,stage='queued',updated_at=excluded.updated_at",params![task_id,now])?;
    tx.execute("UPDATE insight_owner_state SET lifecycle='pending',last_error='' WHERE owner_kind=?1 AND owner_id=?2 AND desired_generation=?3",params![input.snapshot.owner_kind,input.snapshot.owner_id,input.snapshot.generation])?;
    crate::tasks::refresh_job_state(&tx, &job, &now)?;
    let _ = crate::events::record_event(
        &tx,
        &crate::events::NewEvent {
            app: input.snapshot.owner_kind,
            event_type: "insight.retry".into(),
            message: format!(
                "Retry extraction admitted for {live_key}; completed checkpoints retained"
            ),
            job_id: job,
            task_id: task_id.into(),
            ..Default::default()
        },
    );
    tx.commit()?;
    Ok(true)
}

#[cfg(test)]
mod artifact_step_tests {
    use super::*;

    #[test]
    fn parent_failure_is_not_copied_to_validated_artifacts() {
        let p = Progress {
            state: "failed".into(),
            stage: "assessment".into(),
            ..Default::default()
        };
        let facts = ArtifactFacts {
            synthesis_started: true,
            assessments: vec![
                ("Claims", "completed".into()),
                ("Insights", "failed".into()),
            ],
        };
        let steps = artifact_steps(&p, &facts);
        assert_eq!(steps[0].step, "Validated · not published (failed)");
        assert!(steps[1].step.starts_with("Failed"), "{:?}", steps[1]);
        assert!(steps[2].step.starts_with("Failed"), "{:?}", steps[2]);
    }
}
