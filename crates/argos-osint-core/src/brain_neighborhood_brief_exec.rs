//! Generation, cache and delivery coordinator for the neighborhood brief.
//!
//! The deterministic brief is shown immediately. This module publishes it as
//! the fallback in the shared summarization cache, enqueues one background
//! upgrade (deduplicated on the request identity), generates a structured
//! brief with the configured Summarization role from the frozen typed packet,
//! validates the *complete* answer, and stores it. Anything short of a valid
//! complete answer leaves the fallback in place.
//!
//! Reuses `summarization.rs` (`argos_derived_summaries`, flush requests,
//! `argos_tasks`); there is no new table.

use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::brain_neighborhood_brief::{
    output_schema, validate_generated, NeighborhoodBrief, NeighborhoodInput, PACKET_BYTES_MAX,
};
use crate::provider::ChatMessage;
use crate::provider_adapters::ResponseContract;
use crate::recon::model_exec::{
    execute_chat, resolve_operation_routes, ModelOperationError, OperationScope, RoleRuntime,
};
use crate::summarization::{
    cache_get, cache_put, enqueue_summary_flush, load_flush_request,
    publish_deterministic_and_enqueue, sha_hex, CoverageMeta, SummarizationMode, SummaryRequest,
    SummaryResult,
};

/// Operation key used for route overrides, task rows and telemetry.
pub const OPERATION: &str = "topology_neighborhood";
const FAILED_TAG: &str = "topology_failed:";

/// Why a generation ended without a usable brief.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    /// No usable Summarization route (missing key/model, Decisions-only).
    NotConfigured,
    /// Every route was blocked (auth, quota, policy).
    Blocked,
    Cancelled,
    /// The model answered but the answer failed validation.
    Rejected,
    /// Provider/transport failure after the chain's own retries.
    Provider,
    /// Cache or queue storage failed.
    Storage,
    /// No worker settled the request before the deadline.
    Timeout,
}

impl FailureKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotConfigured => "not_configured",
            Self::Blocked => "blocked",
            Self::Cancelled => "cancelled",
            Self::Rejected => "rejected",
            Self::Provider => "provider",
            Self::Storage => "storage",
            Self::Timeout => "timeout",
        }
    }

    fn parse(raw: &str) -> Self {
        match raw {
            "not_configured" => Self::NotConfigured,
            "blocked" => Self::Blocked,
            "cancelled" => Self::Cancelled,
            "rejected" => Self::Rejected,
            "storage" => Self::Storage,
            "timeout" => Self::Timeout,
            _ => Self::Provider,
        }
    }

    /// Whether the Models settings can fix it.
    pub fn needs_models(self) -> bool {
        matches!(self, Self::NotConfigured | Self::Blocked)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenerationFailure {
    pub kind: FailureKind,
    pub message: String,
}

impl GenerationFailure {
    pub(crate) fn new(kind: FailureKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into().chars().take(300).collect(),
        }
    }

    /// Terminal task tag. The queue completes the task with it, so an
    /// exhausted chain is never replayed.
    pub fn tag(&self) -> String {
        format!("{FAILED_TAG}{}:{}", self.kind.as_str(), self.message)
    }

    pub fn from_tag(tag: &str) -> Option<Self> {
        let rest = tag.strip_prefix(FAILED_TAG)?;
        let (kind, message) = rest.split_once(':')?;
        Some(Self {
            kind: FailureKind::parse(kind),
            message: message.to_string(),
        })
    }
}

/// A validated generated brief and where it came from.
#[derive(Clone, Debug, PartialEq)]
pub struct Generated {
    pub brief: NeighborhoodBrief,
    /// `provider/model` that produced it (never a secret).
    pub route: String,
    pub requests: u32,
    pub host_notes: Vec<String>,
}

/// What a reader finds for one request.
#[derive(Clone, Debug, PartialEq)]
pub enum Delivery {
    /// A complete, validated generated brief.
    Generated(Box<Generated>),
    /// Generation is queued or running; keep the fallback.
    Pending,
    /// Generation ended without a usable brief; keep the fallback.
    Failed(GenerationFailure),
    /// Nothing to generate (isolated or missing seed).
    Idle,
}

/// Request identity. `fingerprint` summarizes the effective role
/// configuration and route chain (no secrets).
pub fn build_request(input: &NeighborhoodInput, fingerprint: &str) -> SummaryRequest {
    let text = serde_json::to_string(input).unwrap_or_default();
    SummaryRequest {
        mode: SummarizationMode::TopologyNeighborhood,
        sources: vec![crate::summarization::SummarySource {
            id: format!("topology:{}", input.seed.key()),
            revision: input.basis_key.clone(),
            hash: sha_hex(&text),
            text,
            meta: serde_json::json!({
                "packet": input.packet_version,
                "depth": input.depth,
            }),
        }],
        focus: input.seed.key(),
        budget_chars: PACKET_BYTES_MAX,
        required_fields: Vec::new(),
        model: fingerprint.to_string(),
        provider: "role-chain".into(),
        prompt_version: SummarizationMode::TopologyNeighborhood
            .prompt_version()
            .into(),
        preprocessing: serde_json::json!({"output": "structured_brief"}),
        deterministic: None,
    }
}

/// Fingerprint of the Summarization role and its route chain: provider,
/// account, model, skip state and endpoint host per route, in order.
pub fn route_fingerprint(runtime: &RoleRuntime) -> String {
    let (routes, secrets) = resolve_operation_routes(
        &runtime.auth,
        &runtime.settings,
        "summarization",
        Some(OPERATION),
    );
    let rows: Vec<String> = routes
        .iter()
        .zip(&secrets)
        .map(|(r, s)| {
            let host = url::Url::parse(&s.base_url)
                .ok()
                .and_then(|u| u.host_str().map(str::to_string))
                .unwrap_or_default();
            format!(
                "{}|{}|{}|{}|{}",
                r.provider,
                r.account,
                s.model,
                host,
                r.skip_reason.is_some()
            )
        })
        .collect();
    sha_hex(&rows.join("\n"))[..16].to_string()
}

pub(crate) fn fallback_result(req: &SummaryRequest, fallback: &NeighborhoodBrief) -> SummaryResult {
    SummaryResult {
        content: serde_json::to_string(fallback).unwrap_or_default(),
        source_refs: vec![req.focus.clone()],
        source_hash: req
            .sources
            .first()
            .map(|s| s.hash.clone())
            .unwrap_or_default(),
        model: "deterministic".into(),
        prompt_version: req.prompt_version.clone(),
        coverage: CoverageMeta::default(),
        fallback: true,
        source_hashes: Vec::new(),
        provider: "local".into(),
        preprocessing: serde_json::Value::Null,
    }
}

pub(crate) fn delivered(result: &SummaryResult) -> Option<Generated> {
    if result.fallback {
        return None;
    }
    let brief: NeighborhoodBrief = serde_json::from_str(&result.content).ok()?;
    let meta = &result.preprocessing;
    Some(Generated {
        brief,
        route: meta
            .get("route")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        requests: meta.get("requests").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
        host_notes: meta
            .get("host_notes")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default(),
    })
}

/// Publish the fallback and enqueue one background upgrade, unless an exact
/// generated result already exists. Deduplicated on the request identity.
pub fn publish(
    conn: &Connection,
    input: &NeighborhoodInput,
    fallback: &NeighborhoodBrief,
    fingerprint: &str,
) -> Result<(SummaryRequest, Delivery)> {
    let req = build_request(input, fingerprint);
    if !input.worth_generating() {
        return Ok((req, Delivery::Idle));
    }
    if let Some(hit) = cache_get(conn, &req)? {
        if let Some(g) = delivered(&hit) {
            return Ok((req, Delivery::Generated(Box::new(g))));
        }
    }
    publish_deterministic_and_enqueue(conn, &req, fallback_result(&req, fallback))?;
    let delivery = poll(conn, &req)?;
    Ok((req, delivery))
}

/// Read-only status of one request: generated result, terminal failure, or
/// still pending. Safe for a background watcher; never runs a model.
pub fn poll(conn: &Connection, req: &SummaryRequest) -> Result<Delivery> {
    if let Some(hit) = cache_get(conn, req)? {
        if let Some(g) = delivered(&hit) {
            return Ok(Delivery::Generated(Box::new(g)));
        }
    }
    let row: Option<(String, String, String)> = conn
        .query_row(
            "SELECT state, result_ref, error_message FROM argos_tasks WHERE id=?1",
            [format!("sum-task-{}", req.cache_key())],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    Ok(match row {
        None => Delivery::Idle,
        Some((state, result_ref, error)) => match state.as_str() {
            "queued" | "running" | "retry_scheduled" | "paused" => Delivery::Pending,
            "completed" => match GenerationFailure::from_tag(&result_ref) {
                Some(f) => Delivery::Failed(f),
                None => Delivery::Failed(GenerationFailure::new(
                    FailureKind::Provider,
                    "generation finished without a stored result",
                )),
            },
            _ => Delivery::Failed(GenerationFailure::new(
                FailureKind::Provider,
                if error.is_empty() { state } else { error },
            )),
        },
    })
}

/// Explicit Retry: enqueue a fresh upgrade for the same request identity.
/// Returns false when one is already queued or running (no duplicate work).
pub fn retry(conn: &Connection, req: &SummaryRequest) -> Result<bool> {
    if load_flush_request(conn, &req.cache_key())?.is_none() {
        return Ok(false);
    }
    let key = req.cache_key();
    let now = chrono::Utc::now().to_rfc3339();
    // A finished (completed/failed) task of the same identity is reset in
    // place so Retry is one explicit new run; queued/running work is left
    // alone, which is what prevents duplicate in-flight generation.
    let reset = conn.execute(
        "UPDATE argos_tasks SET state='queued', attempts=0, result_ref='', error_category='',
            error_message='', lease_owner='', lease_until='', next_eligible_at='',
            updated_at=?1, finished_at=''
         WHERE id=?2 AND state IN ('completed','failed','cancelled')",
        rusqlite::params![now, format!("sum-task-{key}")],
    )?;
    if reset > 0 {
        conn.execute(
            "UPDATE argos_jobs SET created_at=?1, cancel_requested=0 WHERE id=?2",
            rusqlite::params![now, format!("sum-job-{key}")],
        )?;
        return Ok(true);
    }
    enqueue_summary_flush(conn, req)
}

/// Decode what a worker needs from a stored flush request.
pub fn decode_request(req: &SummaryRequest) -> Result<(NeighborhoodInput, NeighborhoodBrief)> {
    let text = req
        .sources
        .first()
        .map(|s| s.text.as_str())
        .ok_or_else(|| anyhow::anyhow!("topology request has no packet"))?;
    let input: NeighborhoodInput = serde_json::from_str(text)?;
    let fallback: NeighborhoodBrief = serde_json::from_str(
        &req.deterministic
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("topology request has no fallback"))?
            .content,
    )?;
    Ok((input, fallback))
}

fn classify(error: &anyhow::Error) -> GenerationFailure {
    let message = error.to_string();
    let lower = message.to_lowercase();
    match error.downcast_ref::<ModelOperationError>() {
        None => GenerationFailure::new(FailureKind::NotConfigured, message),
        Some(_) if lower.contains("cancelled") => {
            GenerationFailure::new(FailureKind::Cancelled, message)
        }
        Some(_) if lower.contains("brief rejected") => {
            GenerationFailure::new(FailureKind::Rejected, message)
        }
        Some(_) if lower.contains("blocked") => {
            GenerationFailure::new(FailureKind::Blocked, message)
        }
        Some(_) => GenerationFailure::new(FailureKind::Provider, message),
    }
}

/// Run one generation with the full Summarization role chain. The chain owns
/// provider retries (Summarization attempt cap, summarization timeouts);
/// nothing here retries. Output is buffered and validated whole.
pub async fn generate(
    runtime: &RoleRuntime,
    db_path: Option<&Path>,
    operation_id: &str,
    task: Option<(&str, i64)>,
    req: &SummaryRequest,
    cancel: &Arc<AtomicBool>,
) -> Result<Generated, GenerationFailure> {
    let (input, fallback) = decode_request(req)
        .map_err(|e| GenerationFailure::new(FailureKind::Storage, e.to_string()))?;
    let (routes, _) = resolve_operation_routes(
        &runtime.auth,
        &runtime.settings,
        "summarization",
        Some(OPERATION),
    );
    if routes.iter().all(|r| r.skip_reason.is_some()) {
        return Err(GenerationFailure::new(
            FailureKind::NotConfigured,
            "no usable Summarization model is configured",
        ));
    }
    let messages = vec![
        ChatMessage {
            role: "system".into(),
            content: format!(
                "{} Reply with one JSON object with arrays overview, connections, interpretation and limits; each item is {{\"text\": string, \"basis\": [packet refs such as N1 or E2 only; never record ids]}}.",
                crate::summarization::system_prompt(SummarizationMode::TopologyNeighborhood)
            ),
            tool_call_id: None,
            tool_calls: vec![],
        },
        ChatMessage {
            role: "user".into(),
            content: format!("Neighborhood packet:\n{}", input.prompt_json()),
            tool_call_id: None,
            tool_calls: vec![],
        },
    ];
    let mut scope = OperationScope::new(operation_id, "summarization")
        .with_operation(OPERATION)
        .with_input_revision(&input.basis_key)
        .for_summarization(ResponseContract::GeneratedJson {
            name: "topology_neighborhood_brief".into(),
            schema: Some(output_schema(&input)),
        });
    if let Some((id, epoch)) = task {
        scope = scope.with_task(id).with_task_epoch(epoch);
    }
    let (raw, report) = execute_chat(
        &runtime.auth,
        &runtime.settings,
        &scope,
        &messages,
        |raw| {
            validate_generated(&input, &fallback, raw)
                .map(|_| ())
                .map_err(|e| format!("brief rejected: {e}"))
        },
        cancel,
        None,
        None,
        db_path,
        false,
    )
    .await
    .map_err(|e| classify(&e))?;
    let validated = validate_generated(&input, &fallback, &raw).map_err(|e| {
        GenerationFailure::new(FailureKind::Rejected, format!("brief rejected: {e}"))
    })?;
    let route = report
        .effective_route
        .and_then(|i| routes.get(i as usize))
        .map(|r| format!("{}/{}", r.provider, report.effective_model))
        .unwrap_or_else(|| report.effective_model.clone());
    Ok(Generated {
        brief: validated.brief,
        route,
        requests: report.requests,
        host_notes: validated.host_notes,
    })
}

/// Atomically publish a validated result under the request identity.
pub fn store_generated(conn: &Connection, req: &SummaryRequest, g: &Generated) -> Result<()> {
    cache_put(conn, req, &generated_result(req, g)?)
}

/// The cached form of a validated generated brief.
pub(crate) fn generated_result(req: &SummaryRequest, g: &Generated) -> Result<SummaryResult> {
    Ok(SummaryResult {
        content: serde_json::to_string(&g.brief)?,
        source_refs: g
            .brief
            .references
            .iter()
            .map(|r| format!("{:?}", r.target))
            .collect(),
        source_hash: req
            .sources
            .first()
            .map(|s| s.hash.clone())
            .unwrap_or_default(),
        model: g.route.rsplit('/').next().unwrap_or_default().to_string(),
        prompt_version: req.prompt_version.clone(),
        coverage: CoverageMeta::default(),
        fallback: false,
        source_hashes: req.sources.iter().map(|s| s.hash.clone()).collect(),
        provider: g.route.split('/').next().unwrap_or_default().to_string(),
        preprocessing: serde_json::json!({
            "route": g.route,
            "requests": g.requests,
            "host_notes": g.host_notes,
        }),
    })
}

/// Terminal result of one claimed topology task.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// Queue tag recorded on the task.
    pub tag: String,
    /// A complete, validated generated brief is now cached. A fallback, a
    /// rejection or a failure is never success.
    pub generated: bool,
    /// The lease was lost; nothing was published or completed.
    pub stale: bool,
}

fn lease_held(conn: &Connection, c: &crate::tasks::ClaimedTask) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT 1 FROM argos_tasks WHERE id=?1 AND lease_owner=?2 AND lease_epoch=?3 AND state='running'",
        rusqlite::params![c.id, c.owner, c.epoch],
        |_| Ok(()),
    )
    .optional()
    .map(|r| r.is_some())
}

/// Renew the lease on its own connection until `stop`. A refused renewal
/// means the lease is lost: set `lost` and cancel the model operation.
fn heartbeat(
    db_path: &Path,
    c: &crate::tasks::ClaimedTask,
    lease_secs: i64,
    stop: Arc<AtomicBool>,
    lost: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
) -> std::thread::JoinHandle<()> {
    use std::sync::atomic::Ordering;
    let path = db_path.to_path_buf();
    let (id, owner, epoch) = (c.id.clone(), c.owner.clone(), c.epoch);
    let every = std::time::Duration::from_millis((lease_secs.max(1) as u64 * 1000 / 3).max(50));
    std::thread::spawn(move || {
        // Real elapsed time, not a count of sleeps: a descheduled thread must
        // not stretch the renewal interval.
        let mut last_attempt = std::time::Instant::now();
        let tick = std::time::Duration::from_millis(25);
        // The lease was last extended at `renewed_at`; it truly lapses
        // `lease_secs` later. A late or failed renewal under load is retried
        // (shorter interval) and is only fatal when the lease really passed,
        // two renewals in a row failed, or another worker owns the task.
        let mut renewed_at = std::time::Instant::now();
        let mut failures = 0u32;
        let mut interval = every;
        while !stop.load(Ordering::SeqCst) {
            std::thread::sleep(tick);
            if last_attempt.elapsed() < interval {
                continue;
            }
            last_attempt = std::time::Instant::now();
            // Ok(Some(true)) renewed; Ok(Some(false)) refused but still ours;
            // Ok(None) the task is gone/taken; Err transient failure.
            let attempt = Connection::open(&path).map_err(|_| ()).and_then(|conn| {
                let _ = conn.busy_timeout(std::time::Duration::from_secs(5));
                let ok = crate::tasks::renew_lease(
                    &conn,
                    &id,
                    &owner,
                    epoch,
                    lease_secs,
                    &chrono::Utc::now().to_rfc3339(),
                )
                .map_err(|_| ())?;
                if ok {
                    return Ok(Some(true));
                }
                // Refused: is the task still ours (a transient refusal) or
                // genuinely taken/finished/cancelled?
                let ours: bool = conn
                    .query_row(
                        "SELECT 1 FROM argos_tasks WHERE id=?1 AND lease_owner=?2 AND lease_epoch=?3 AND state='running'",
                        rusqlite::params![id, owner, epoch],
                        |_| Ok(()),
                    )
                    .optional()
                    .map_err(|_| ())?
                    .is_some();
                Ok(if ours { Some(false) } else { None })
            });
            match attempt {
                Ok(Some(true)) => {
                    renewed_at = std::time::Instant::now();
                    failures = 0;
                    interval = every;
                }
                other => {
                    failures += 1;
                    let gone = matches!(other, Ok(None));
                    let passed = renewed_at.elapsed()
                        >= std::time::Duration::from_secs(lease_secs.max(1) as u64);
                    if gone || failures >= 2 || passed {
                        lost.store(true, Ordering::SeqCst);
                        cancel.store(true, Ordering::SeqCst);
                        break;
                    }
                    interval = std::time::Duration::from_millis(100);
                }
            }
        }
    })
}

/// Run one claimed `topology_neighborhood` task with the full role runtime.
///
/// The lease is renewed while the model runs. Publication and completion
/// happen in one immediate transaction fenced by `(owner, epoch)`, so a
/// stale worker can neither publish nor complete. Provider retries belong to
/// the model chain; every chain-level failure completes the task with a
/// typed tag, so the queue never replays an exhausted chain. Only storage
/// failures are returned as `Err` (the queue's own retry policy applies).
pub fn run_claimed(
    conn: &Connection,
    claimed: &crate::tasks::ClaimedTask,
    runtime: Option<&RoleRuntime>,
    db_path: Option<&Path>,
    lease_secs: i64,
) -> Result<Outcome, crate::tasks::TaskError> {
    use crate::tasks::{ErrorCategory, TaskError};
    use std::sync::atomic::Ordering;
    let storage = |e: &dyn std::fmt::Display| TaskError::new(ErrorCategory::Unknown, e.to_string());
    let req = load_flush_request(conn, &claimed.input_hash)
        .map_err(|e| storage(&format!("{e:#}")))?
        .ok_or_else(|| storage(&format!("flush request missing for {}", claimed.input_hash)))?;
    let finish = |tag: String, generated: bool| -> Result<Outcome, TaskError> {
        let now = chrono::Utc::now().to_rfc3339();
        let done = crate::tasks::complete_claimed(conn, claimed, &tag, &now)
            .map_err(|e| storage(&format!("{e:#}")))?;
        Ok(Outcome {
            tag,
            generated: generated && done,
            stale: !done,
        })
    };
    if let Some(hit) = cache_get(conn, &req).map_err(|e| storage(&format!("{e:#}")))? {
        if delivered(&hit).is_some() {
            return finish("already_upgraded".into(), true);
        }
    }
    let Some(runtime) = runtime else {
        let f = GenerationFailure::new(
            FailureKind::NotConfigured,
            "no Summarization model runtime is available",
        );
        return finish(f.tag(), false);
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let lost = Arc::new(AtomicBool::new(false));
    let stop = Arc::new(AtomicBool::new(false));
    let beat = db_path.map(|p| {
        heartbeat(
            p,
            claimed,
            lease_secs,
            stop.clone(),
            lost.clone(),
            cancel.clone(),
        )
    });
    let operation_id = format!("{}:e{}:topology", claimed.id, claimed.epoch);
    let result = crate::brain_lance::block_on(generate(
        runtime,
        db_path,
        &operation_id,
        Some((&claimed.id, claimed.epoch)),
        &req,
        &cancel,
    ));
    stop.store(true, Ordering::SeqCst);
    if let Some(h) = beat {
        let _ = h.join();
    }
    if lost.load(Ordering::SeqCst) {
        return Ok(Outcome {
            tag: "lease_lost".into(),
            generated: false,
            stale: true,
        });
    }
    // Fenced publication: lease check, cache write and completion are one
    // immediate transaction.
    let tx = rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| storage(&e))?;
    if !lease_held(&tx, claimed).map_err(|e| storage(&e))? {
        return Ok(Outcome {
            tag: "lease_lost".into(),
            generated: false,
            stale: true,
        });
    }
    let (tag, generated) = match &result {
        Ok(g) => {
            store_generated(&tx, &req, g).map_err(|e| storage(&format!("{e:#}")))?;
            (format!("generated:{}", g.route), true)
        }
        Err(f) => (f.tag(), false),
    };
    let now = chrono::Utc::now().to_rfc3339();
    let done = crate::tasks::complete_claimed(&tx, claimed, &tag, &now)
        .map_err(|e| storage(&format!("{e:#}")))?;
    if !done {
        return Ok(Outcome {
            tag: "lease_lost".into(),
            generated: false,
            stale: true,
        });
    }
    tx.commit().map_err(|e| storage(&e))?;
    Ok(Outcome {
        tag,
        generated,
        stale: false,
    })
}

/// Bounds for the background watcher: it never waits forever.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WatchLimits {
    /// Give up after this many consecutive failed polls.
    pub max_poll_errors: u32,
    /// Overall deadline from the first poll.
    pub deadline: std::time::Duration,
}

impl WatchLimits {
    /// Summarization attempt timeout x attempt cap, plus a margin for the
    /// queue and worker start.
    pub fn standard() -> Self {
        let per_attempt = crate::provider_request::TimeoutProfile::SUMMARIZATION
            .attempt_timeout()
            .as_secs();
        let cap = u64::from(crate::tasks::OperationKind::Summarization.attempt_cap());
        Self {
            max_poll_errors: 5,
            deadline: std::time::Duration::from_secs(per_attempt * cap + 60),
        }
    }
}

/// Background watcher body: poll the cache and task state every `every`
/// (one read in flight, never a model request) until the request settles or
/// `stop` is set. Returns `None` when detached.
pub fn watch_until_settled(
    db_path: &Path,
    req: &SummaryRequest,
    every: std::time::Duration,
    stop: &AtomicBool,
) -> Option<Delivery> {
    watch_with_limits(db_path, req, every, stop, WatchLimits::standard())
}

/// As [`watch_until_settled`] with explicit bounds. Repeated poll errors, a
/// vanished database and the deadline all end in a typed `Delivery::Failed`
/// so the UI can stop showing "generating".
pub fn watch_with_limits(
    db_path: &Path,
    req: &SummaryRequest,
    every: std::time::Duration,
    stop: &AtomicBool,
    limits: WatchLimits,
) -> Option<Delivery> {
    use std::sync::atomic::Ordering;
    let begun = std::time::Instant::now();
    let fail =
        |kind, message: String| Some(Delivery::Failed(GenerationFailure::new(kind, message)));
    let open = || -> Option<Connection> {
        let conn = Connection::open(db_path).ok()?;
        let _ = conn.busy_timeout(std::time::Duration::from_secs(5));
        Some(conn)
    };
    let Some(mut conn) = open() else {
        return fail(
            FailureKind::Storage,
            "the database could not be opened".into(),
        );
    };
    let mut errors = 0u32;
    loop {
        if stop.load(Ordering::SeqCst) {
            return None;
        }
        match poll(&conn, req) {
            Ok(Delivery::Pending) => errors = 0,
            Ok(settled) => return Some(settled),
            Err(error) => {
                errors += 1;
                if errors >= limits.max_poll_errors {
                    return fail(
                        FailureKind::Storage,
                        format!("could not read the summary state: {error:#}"),
                    );
                }
                if let Some(fresh) = open() {
                    conn = fresh;
                }
            }
        }
        if begun.elapsed() >= limits.deadline {
            return fail(
                FailureKind::Timeout,
                "waiting for a worker: no summary result before the deadline".into(),
            );
        }
        let mut slept = std::time::Duration::ZERO;
        while slept < every {
            if stop.load(Ordering::SeqCst) {
                return None;
            }
            let step = std::time::Duration::from_millis(25).min(every - slept);
            std::thread::sleep(step);
            slept += step;
        }
    }
}

#[cfg(test)]
#[path = "brain_neighborhood_brief_exec_tests.rs"]
mod tests;
