//! Bounded checkpoint slices and explicit retry epochs, independent of immutable work identity.
use super::insights::PipelineInput;
use anyhow::{ensure, Result};

tokio::task_local! { static SLICE_REMAINING: std::cell::Cell<usize>; }

#[derive(Debug)]
pub(crate) struct SliceYield;
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

pub(crate) async fn slice<T>(future: impl std::future::Future<Output = T>) -> T {
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

/// The `insight.activity` event of `task_id` (job and owner scope looked up
/// by the store).
pub(crate) fn activity_new_event(
    task_id: &str,
    job: String,
    owner: String,
    message: &str,
) -> crate::events::NewEvent {
    let app = if owner.starts_with("recon:") {
        "recon"
    } else {
        "intel"
    };
    crate::events::NewEvent {
        severity: Some(crate::events::Severity::Info),
        app: app.into(),
        event_type: "insight.activity".into(),
        message: message.into(),
        job_id: job,
        task_id: task_id.into(),
        resource_ref: owner,
        ..Default::default()
    }
}

/// The `insight.stage.{outcome}` event of one pipeline stage.
pub(crate) fn stage_new_event(
    input: &PipelineInput,
    task_id: &str,
    job: String,
    stage: &str,
    outcome: &str,
    message: &str,
    details: serde_json::Value,
) -> crate::events::NewEvent {
    {
        crate::events::NewEvent {
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
        }
    }
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
