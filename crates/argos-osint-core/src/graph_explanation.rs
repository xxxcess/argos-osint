//! Brain graph explanation: one durable, budgeted, revision-aware execution.
//!
//! [`explain`] registers one `graph_explanation` job, runs the summary through
//! [`crate::summarization::complete_summary_report`] (≤2 outbound requests,
//! admission waits free), records every attempt as a child job with a
//! structured event, saves the result only for the exact inputs it was written
//! from, and persists the failure event and the diagnostic record *before*
//! returning, so the caller can notify the UI knowing the logs already exist.
//!
//! A failure never touches the memory itself or Atlas indexing state.

use std::path::Path;
use std::time::Duration;

use rusqlite::Connection;
use serde_json::json;

use crate::events::{self, NewEvent, Severity};
use crate::job_registry::{Finish, JobHandle, JobSpec};
use crate::provider::ChatMessage;
use crate::provider_diag::{Category, ProviderFailure, Stage};
use crate::recon::{recon_path, MemoryGraph};
use crate::secrets::ProviderSecret;
use crate::store::{ExplanationRecord, SaveOutcome, Store, SummaryKey};
use crate::summarization::{
    complete_summary_report, AttemptEvent, AttemptLog, ExecOptions, SummarizationMode,
};

pub const OPERATION: &str = "graph_explanation";
pub const PROMPT_VERSION: &str = "graph-explanation-v2";
/// After a failure, reopening the memory does not start another execution
/// for the same inputs until this has passed (explicit Retry bypasses it).
pub const FAILURE_COOLDOWN: Duration = Duration::from_secs(120);
/// Heading of the deterministic text shown when no AI summary is available.
pub const BASIC_HEADING: &str = "Basic graph explanation — AI summary unavailable";

/// Everything one execution needs.
#[derive(Clone, Debug)]
pub struct ExplainRequest {
    pub memory_id: String,
    pub memory_text: String,
    pub focus: String,
    pub claim: bool,
    pub system: String,
    pub graph_brief: String,
    /// Caller-generated id; late completions for older ids are ignored.
    pub request_id: String,
    /// Job id of the failed execution this one retries.
    pub retry_of: Option<String>,
}

impl ExplainRequest {
    pub fn prompt(&self) -> String {
        format!("Memory:\n{}\n\n{}", self.memory_text, self.graph_brief)
    }

    pub fn key(&self, secret: &ProviderSecret) -> SummaryKey {
        SummaryKey {
            memory_revision: crate::store::memory_revision(&self.memory_text),
            graph_revision: crate::evidence::content_hash(&self.graph_brief),
            focus: self.focus.clone(),
            provider: crate::provider::effective_kind(secret),
            model: secret.model.clone(),
            prompt_version: format!(
                "{PROMPT_VERSION}:{}",
                crate::evidence::content_hash(&self.system)
            ),
        }
    }
}

/// How an execution ended.
#[derive(Clone, Debug, PartialEq)]
pub enum ExplainOutcome {
    Saved(String),
    /// The memory changed or was deleted meanwhile; nothing was published.
    Superseded(String),
    Failed(Box<ProviderFailure>),
}

/// Typed execution report.
#[derive(Clone, Debug)]
pub struct ExplainReport {
    pub request_id: String,
    pub memory_id: String,
    pub job_id: String,
    pub cache_key: String,
    pub outcome: ExplainOutcome,
    pub attempts: Vec<AttemptLog>,
    pub admission_wait_ms: u64,
    pub fallback_reason: Option<String>,
    /// Failure event id (selected by Brain → View logs).
    pub event_id: String,
    /// Set when the job/event/diagnostic could not be written.
    pub logging_error: Option<String>,
}

impl ExplainReport {
    pub fn requests(&self) -> usize {
        self.attempts.len()
    }
}

/// What the cache holds for the current inputs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cached {
    Valid(String),
    /// Written from different inputs; shown only as an earlier result.
    Stale(String),
    None,
}

pub fn cached(store: &Store, memory_id: &str, key: &SummaryKey) -> anyhow::Result<Cached> {
    Ok(match store.graph_summary_entry(memory_id)? {
        Some(entry) if entry.cache_key == key.digest() => Cached::Valid(entry.summary),
        Some(entry) => Cached::Stale(entry.summary),
        None => Cached::None,
    })
}

/// Whether opening the memory should start an execution.
#[derive(Clone, Debug, PartialEq)]
pub enum Gate {
    Ready,
    /// An execution for this memory is already running.
    Running(String),
    /// The same inputs failed recently; show the failure instead.
    CoolingDown(Box<ExplanationRecord>),
}

pub fn gate(store: &Store, memory_id: &str, key: &SummaryKey, explicit_retry: bool) -> Gate {
    let Ok(Some(rec)) = store.graph_explanation_record(memory_id) else {
        return Gate::Ready;
    };
    if rec.state == "running" {
        if let Ok(Some(state)) = store.job_state(&rec.job_id) {
            if state == "running" {
                return Gate::Running(rec.job_id);
            }
        }
    }
    if explicit_retry || rec.state != "failed" || rec.cache_key != key.digest() {
        return Gate::Ready;
    }
    let recent = chrono::DateTime::parse_from_rfc3339(&rec.updated_at)
        .map(|t| {
            chrono::Utc::now().signed_duration_since(t.with_timezone(&chrono::Utc))
                < chrono::Duration::from_std(FAILURE_COOLDOWN).unwrap_or_default()
        })
        .unwrap_or(false);
    if recent {
        Gate::CoolingDown(Box::new(rec))
    } else {
        Gate::Ready
    }
}

/// Deterministic explanation built from the path alone.
pub fn basic_explanation(graph: &MemoryGraph, memory_text: &str) -> String {
    let path = recon_path(graph);
    let mut out = vec![format!("## {BASIC_HEADING}"), String::new()];
    if let Some(line) = memory_text.lines().map(str::trim).find(|l| !l.is_empty()) {
        out.push(format!("Conclusion: {line}"));
    }
    if !path.investigation.trim().is_empty() {
        out.push(format!("Investigation: {}", path.investigation.trim()));
    }
    for band in &path.bands {
        let evidence = match band.evidence.len() {
            0 => "no evidence".to_string(),
            1 => "1 evidence item".to_string(),
            n => format!("{n} evidence items"),
        };
        let mut line = format!("- {} · {evidence}", band.directive_label);
        if let Some(finding) = &band.finding {
            line.push_str(&format!(" · finding: {finding}"));
        }
        out.push(line);
    }
    if path.bands.is_empty() {
        out.push(format!("- {} nodes in the graph", graph.nodes.len()));
    }
    out.join("\n")
}

/// Test-only fault injection.
#[derive(Clone, Copy, Debug, Default)]
pub struct Faults {
    /// Make the summary save fail like a storage error.
    pub persistence: bool,
}

fn open(db: &Path) -> anyhow::Result<Connection> {
    let conn = Connection::open(db)?;
    conn.busy_timeout(Duration::from_secs(5))?;
    Ok(conn)
}

#[allow(clippy::too_many_arguments)]
fn event(
    db: &Path,
    severity: Severity,
    event_type: &str,
    message: String,
    job_id: &str,
    correlation: &str,
    memory_id: &str,
    details: serde_json::Value,
) -> anyhow::Result<String> {
    let conn = open(db)?;
    events::record_event(
        &conn,
        &NewEvent {
            severity: Some(severity),
            app: "brain".into(),
            event_type: event_type.into(),
            message,
            details: details.to_string(),
            job_id: job_id.into(),
            resource_ref: format!("memory:{memory_id}"),
            correlation_id: correlation.into(),
            ..Default::default()
        },
    )
}

/// Run one graph explanation end to end.
pub async fn explain(
    db: &Path,
    secret: &ProviderSecret,
    req: &ExplainRequest,
    opts: &ExecOptions,
    faults: Faults,
) -> ExplainReport {
    let key = req.key(secret);
    let digest = key.digest();
    let mut logging: Vec<String> = Vec::new();
    let title = if req.claim {
        "Explain claim path"
    } else {
        "Explain recon path"
    };
    let mut spec = JobSpec::new("brain", OPERATION, title)
        .resource(format!("memory:{}", req.memory_id))
        .run(format!("graph:{}", req.memory_id))
        .model(key.provider.clone(), key.model.clone());
    spec.correlation_id = req.retry_of.clone().unwrap_or_default();
    let job = match JobHandle::begin(db, spec) {
        Ok(job) => Some(job),
        Err(err) => {
            logging.push(format!("job registration failed: {err:#}"));
            None
        }
    };
    let job_id = job.as_ref().map(|j| j.id().to_string()).unwrap_or_default();
    let correlation = job
        .as_ref()
        .map(|j| j.correlation().to_string())
        .unwrap_or_default();
    let mut record = ExplanationRecord {
        memory_id: req.memory_id.clone(),
        job_id: job_id.clone(),
        request_id: req.request_id.clone(),
        cache_key: digest.clone(),
        state: "running".into(),
        updated_at: chrono::Utc::now().to_rfc3339(),
        ..Default::default()
    };
    if let Err(err) = Store::open(db).and_then(|s| s.put_graph_explanation_record(&record)) {
        logging.push(format!("diagnostic record failed: {err:#}"));
    }
    if let Err(err) = event(
        db,
        Severity::Info,
        "graph_explanation.started",
        format!("{title} for memory {}", req.memory_id),
        &job_id,
        &correlation,
        &req.memory_id,
        json!({
            "request_id": req.request_id,
            "retry_of": req.retry_of,
            "cache_key": digest,
            "focus": key.focus,
            "provider": key.provider,
            "model": key.model,
            "prompt_version": key.prompt_version,
            "max_requests": crate::summarization::MAX_REQUESTS,
        }),
    ) {
        logging.push(format!("event write failed: {err:#}"));
    }
    let messages = [
        ChatMessage {
            role: "system".into(),
            content: req.system.clone(),
            tool_call_id: None,
            tool_calls: Vec::new(),
        },
        ChatMessage {
            role: "user".into(),
            content: req.prompt(),
            tool_call_id: None,
            tool_calls: Vec::new(),
        },
    ];
    let mut child: Option<JobHandle> = None;
    let exec = complete_summary_report(
        secret,
        SummarizationMode::GraphExplanation,
        &messages,
        &[],
        opts,
        |ev| match ev {
            AttemptEvent::Started { number, transport } => {
                if let Some(job) = &job {
                    job.phase(
                        &format!("attempt {number} · {}", transport.as_str()),
                        Some(number as i64 - 1),
                        Some(2),
                    );
                    child = job
                        .child(
                            JobSpec::new(
                                "brain",
                                "graph_explanation_attempt",
                                format!("Attempt {number} · {}", transport.as_str()),
                            )
                            .resource(format!("memory:{}", req.memory_id))
                            .model(key.provider.clone(), key.model.clone()),
                        )
                        .ok();
                }
            }
            AttemptEvent::Finished(log) => {
                let child_id = child
                    .as_ref()
                    .map(|c| c.id().to_string())
                    .unwrap_or_default();
                let (severity, message) = match &log.failure {
                    None => (Severity::Info, format!("Attempt {} succeeded", log.number)),
                    Some(f) => (
                        Severity::Warn,
                        format!("Attempt {} failed: {}", log.number, f.summary()),
                    ),
                };
                let _ = event(
                    db,
                    severity,
                    "graph_explanation.attempt",
                    message,
                    if child_id.is_empty() {
                        &job_id
                    } else {
                        &child_id
                    },
                    &correlation,
                    &req.memory_id,
                    json!({ "request_id": req.request_id, "attempt": log }),
                );
                if let Some(c) = child.take() {
                    c.finish(match &log.failure {
                        None => Finish::completed(),
                        Some(f) => Finish::failed(f.category.task_category().as_str(), f.summary()),
                    });
                }
            }
        },
    )
    .await;
    let outcome = match exec.content {
        Some(text) => {
            let saved = if faults.persistence {
                Err(anyhow::anyhow!("disk I/O error (injected)"))
            } else {
                Store::open(db)
                    .and_then(|s| s.save_graph_summary_keyed(&req.memory_id, &text, &key))
            };
            match saved {
                Ok(SaveOutcome::Saved) => ExplainOutcome::Saved(text),
                Ok(SaveOutcome::MemoryGone) => ExplainOutcome::Superseded(
                    "memory was deleted before the summary was saved".into(),
                ),
                Ok(SaveOutcome::MemoryChanged) => ExplainOutcome::Superseded(
                    "memory changed while the summary was written".into(),
                ),
                Err(err) => {
                    let mut f = ProviderFailure::new(
                        Stage::Persistence,
                        Category::Persistence,
                        "summary could not be saved",
                    )
                    .with_context(
                        &key.provider,
                        &key.model,
                        &secret.base_url,
                        "",
                    );
                    f.causes = crate::provider_diag::cause_chain(err.as_ref());
                    f.causes.insert(0, "summary could not be saved".into());
                    ExplainOutcome::Failed(Box::new(f))
                }
            }
        }
        None => ExplainOutcome::Failed(Box::new(exec.failure.clone().unwrap_or_else(|| {
            ProviderFailure::new(Stage::Response, Category::Empty, "no summary produced")
        }))),
    };
    let mut event_id = String::new();
    record.attempts = exec.attempts.len() as i64;
    record.updated_at = chrono::Utc::now().to_rfc3339();
    match &outcome {
        ExplainOutcome::Saved(_) => {
            record.state = "completed".into();
        }
        ExplainOutcome::Superseded(why) => {
            record.state = "superseded".into();
            record.reason = why.clone();
        }
        ExplainOutcome::Failed(f) => {
            record.state = "failed".into();
            record.category = f.category.as_str().into();
            record.reason = f.summary();
            record.guidance = f.guidance().unwrap_or_default();
            record.needs_config = f.needs_configuration();
            record.retryable = f.retryable;
            let details = json!({
                "request_id": req.request_id,
                "retry_of": req.retry_of,
                "failure": f,
                "attempts": exec.attempts,
                "requests": exec.attempts.len(),
                "admission_wait_ms": exec.admission_wait_ms,
                "fallback_reason": exec.fallback_reason,
            });
            record.diagnostic_json = details.to_string();
            match event(
                db,
                Severity::Error,
                "graph_explanation.failed",
                format!("Graph summary failed: {}", f.summary()),
                &job_id,
                &correlation,
                &req.memory_id,
                details,
            ) {
                Ok(id) => event_id = id,
                Err(err) => logging.push(format!("failure event write failed: {err:#}")),
            }
        }
    }
    record.event_id = event_id.clone();
    if let Err(err) = Store::open(db).and_then(|s| s.put_graph_explanation_record(&record)) {
        logging.push(format!("diagnostic record failed: {err:#}"));
    }
    if let Some(job) = job {
        job.finish(match &outcome {
            ExplainOutcome::Saved(_) => Finish::Completed {
                result_ref: format!("memory:{}", req.memory_id),
            },
            ExplainOutcome::Superseded(why) => Finish::Cancelled {
                summary: why.clone(),
            },
            ExplainOutcome::Failed(f) => {
                Finish::failed(f.category.task_category().as_str(), f.summary())
            }
        });
    }
    ExplainReport {
        request_id: req.request_id.clone(),
        memory_id: req.memory_id.clone(),
        job_id,
        cache_key: digest,
        outcome,
        attempts: exec.attempts,
        admission_wait_ms: exec.admission_wait_ms,
        fallback_reason: exec.fallback_reason,
        event_id,
        logging_error: (!logging.is_empty()).then(|| logging.join("; ")),
    }
}

/// "View details" lines for a stored diagnostic record.
pub fn record_detail_lines(rec: &ExplanationRecord) -> Vec<String> {
    let mut lines = Vec::new();
    let v: serde_json::Value = serde_json::from_str(&rec.diagnostic_json).unwrap_or_default();
    if let Ok(f) = serde_json::from_value::<ProviderFailure>(v["failure"].clone()) {
        lines.extend(f.detail_lines());
    } else if !rec.reason.is_empty() {
        lines.push(format!("Reason: {}", rec.reason));
    }
    if let Ok(attempts) = serde_json::from_value::<Vec<AttemptLog>>(v["attempts"].clone()) {
        let wait = v["admission_wait_ms"].as_u64().unwrap_or(0);
        lines.push(format!(
            "Requests: {} of {} · admission wait {} ms (not counted)",
            attempts.len(),
            crate::summarization::MAX_REQUESTS,
            wait
        ));
        lines.extend(attempts.iter().map(AttemptLog::line));
    }
    if let Some(reason) = v["fallback_reason"].as_str() {
        lines.push(format!("Fallback: {reason}"));
    }
    if !rec.job_id.is_empty() {
        lines.push(format!("Job: {}", rec.job_id));
    }
    if !rec.request_id.is_empty() {
        lines.push(format!("Request: {}", rec.request_id));
    }
    lines
}

#[cfg(test)]
mod tests;
