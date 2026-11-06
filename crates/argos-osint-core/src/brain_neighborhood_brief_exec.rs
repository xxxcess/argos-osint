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
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::brain_neighborhood_brief::{
    validate_generated, NeighborhoodBrief, NeighborhoodInput, PACKET_BYTES_MAX,
};
use crate::provider::ChatMessage;
use crate::provider_adapters::ResponseContract;
use crate::recon::model_exec::{
    execute_chat, resolve_operation_routes, ModelOperationError, OperationScope, RoleRuntime,
};
use crate::summarization::{
    sha_hex, CoverageMeta, SummarizationMode, SummaryRequest, SummaryResult,
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
    let started = std::time::Instant::now();
    let text = serde_json::to_string(input).unwrap_or_default();
    crate::intel_recon::brief_timing::note(
        crate::intel_recon::brief_timing::Phase::Packet,
        started.elapsed(),
    );
    crate::intel_recon::brief_timing::emit_observation(
        "neighborhood_request",
        serde_json::json!({"elapsed_us":started.elapsed().as_micros() as u64,"packet_bytes":text.len()}),
    );
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
        prompt_version: format!(
            "{}:{}",
            SummarizationMode::TopologyNeighborhood.prompt_version(),
            crate::provider::structured_text::VERSION
        ),
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
        "brain_summary",
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
        coverage: CoverageMeta {
            partial: matches!(
                fallback.coverage.status,
                crate::brain_neighborhood_brief::CoverageStatus::Limited
                    | crate::brain_neighborhood_brief::CoverageStatus::Missing
            ),
            omitted: vec![format!(
                "{} nodes and {} relationships omitted; {} payload reads skipped",
                fallback.coverage.omitted_nodes,
                fallback.coverage.omitted_edges,
                fallback.coverage.payload_reads_skipped
            )],
            notes: fallback.footer(),
        },
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

/// Deliver only an upgrade bound to this exact frozen packet and contract.
/// A retained cache row cannot authorize a different basis or foreign refs.
pub(crate) fn delivered_for(req: &SummaryRequest, result: &SummaryResult) -> Option<Generated> {
    if result.prompt_version != req.prompt_version
        || result.source_hashes
            != req
                .sources
                .iter()
                .map(|source| source.hash.clone())
                .collect::<Vec<_>>()
    {
        return None;
    }
    let input: NeighborhoodInput = serde_json::from_str(&req.sources.first()?.text).ok()?;
    let generated = delivered(result)?;
    if generated.brief.basis_key != input.basis_key || generated.brief.seed != input.seed {
        return None;
    }
    // An explicit empty model answer retains the exact host fallback. Its
    // references can cover records omitted only from the smaller model packet.
    if generated.route == "deterministic"
        && decode_request(req)
            .ok()
            .is_some_and(|(_, fallback)| fallback == generated.brief)
    {
        return Some(generated);
    }
    let nodes: std::collections::BTreeSet<_> = input
        .nodes
        .iter()
        .map(|node| node.node.id.as_str())
        .collect();
    let edges: std::collections::BTreeSet<_> = input
        .edges
        .iter()
        .map(|edge| edge.edge.id.as_str())
        .collect();
    if generated
        .brief
        .references
        .iter()
        .any(|reference| match &reference.target {
            crate::brain_neighborhood_brief::BasisTarget::Node(id) => !nodes.contains(id.as_str()),
            crate::brain_neighborhood_brief::BasisTarget::Edge(id) => !edges.contains(id.as_str()),
        })
        || generated.brief.items().any(|item| {
            item.node_ids.iter().any(|id| !nodes.contains(id.as_str()))
                || item.edge_ids.iter().any(|id| !edges.contains(id.as_str()))
                || item
                    .basis
                    .iter()
                    .any(|index| *index == 0 || *index > generated.brief.references.len())
        })
    {
        return None;
    }
    Some(generated)
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

/// The classified answer of one summary generation.
enum SummaryAnswer {
    /// Explicit `Result: none` with its reason: a valid empty result, never
    /// a rejection. The deterministic brief stays the served content.
    Empty(String),
    /// Section blocks converted to the cached Brief DTO.
    Sections(Value),
}

fn summary_answer(raw: &str) -> anyhow::Result<SummaryAnswer> {
    let document =
        crate::provider::structured_text::parse(raw).map_err(|error| anyhow::anyhow!("{error}"))?;
    if document.blocks.is_empty() {
        return Ok(SummaryAnswer::Empty(
            document.empty_reason.unwrap_or_default(),
        ));
    }
    Ok(SummaryAnswer::Sections(
        crate::provider::structured_text::summary_value(raw)?,
    ))
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
    let measured_at = std::time::Instant::now();
    let clock = crate::intel_recon::brief_timing::StageClock::default();
    let result = crate::intel_recon::brief_timing::with_clock(
        clock.clone(),
        generate_measured(runtime, db_path, operation_id, task, req, cancel),
    )
    .await;
    let (outcome, requests) = match &result {
        Ok(value) => ("generated", Some(value.requests)),
        Err(_) => ("failed_or_cancelled", None),
    };
    crate::intel_recon::brief_timing::emit(&clock, measured_at.elapsed(), outcome, 0, OPERATION);
    crate::intel_recon::brief_timing::emit_observation(
        "neighborhood_provider",
        serde_json::json!({"provider_calls":requests,"outcome":outcome}),
    );
    result
}

async fn generate_measured(
    runtime: &RoleRuntime,
    db_path: Option<&Path>,
    operation_id: &str,
    task: Option<(&str, i64)>,
    req: &SummaryRequest,
    cancel: &Arc<AtomicBool>,
) -> Result<Generated, GenerationFailure> {
    if !req
        .prompt_version
        .ends_with(crate::provider::structured_text::VERSION)
    {
        return Err(GenerationFailure::new(FailureKind::Rejected,
            "legacy summary generation superseded; request a current text-contract generation (retained evidence and fallback are unchanged)"));
    }
    let (input, fallback) = decode_request(req)
        .map_err(|e| GenerationFailure::new(FailureKind::Storage, e.to_string()))?;
    let (routes, _) = resolve_operation_routes(
        &runtime.auth,
        &runtime.settings,
        "brain_summary",
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
                "{} Use readable sections Current State, Significant Developments, Actor Relationships, Cross-Domain Implications, Emerging Signals, Conditional Outlook, Evidence, and Limitations. Omit unsupported sections; each supported section is one or two complete text blocks. Text is prose and Basis lists packet references such as N1 or E2, never record IDs. Use the same fields for each section.\n{}",
                crate::summarization::system_prompt(SummarizationMode::TopologyNeighborhood),
                crate::provider::structured_text::instructions("current state")
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
    let mut scope = OperationScope::new(operation_id, "brain_summary")
        .with_operation(OPERATION)
        .with_input_revision(&input.basis_key)
        .for_summarization(ResponseContract::Prose)
        .with_structured_text();
    if let Some((id, epoch)) = task {
        scope = scope.with_task(id).with_task_epoch(epoch);
    }
    let (raw, report) = crate::intel_recon::brief_timing::timed(
        crate::intel_recon::brief_timing::Phase::Model,
        execute_chat(
            &runtime.auth,
            &runtime.settings,
            &scope,
            &messages,
            |raw| match summary_answer(raw) {
                Ok(SummaryAnswer::Empty(_)) => Ok(()),
                Ok(SummaryAnswer::Sections(value)) => {
                    validate_generated(&input, &fallback, &value.to_string())
                        .map(|_| ())
                        .map_err(|e| format!("brief rejected: {e}"))
                }
                Err(error) => Err(format!("brief rejected: {error}")),
            },
            cancel,
            None,
            None,
            db_path,
            false,
        ),
    )
    .await
    .map_err(|e| classify(&e))?;
    let converted = match summary_answer(&raw) {
        // An explicit `Result: none` summary is a valid empty result, never a
        // rejection: the deterministic brief remains the cached content and
        // the audit note records why no generated sections exist.
        Ok(SummaryAnswer::Empty(reason)) => {
            return Ok(Generated {
                brief: fallback,
                route: "deterministic".into(),
                requests: report.requests,
                host_notes: vec![format!("explicit empty summary: {reason}")],
            });
        }
        Ok(SummaryAnswer::Sections(converted)) => converted,
        Err(error) => {
            return Err(GenerationFailure::new(
                FailureKind::Rejected,
                error.to_string(),
            ))
        }
    };
    let validation_at = std::time::Instant::now();
    let validated = validate_generated(&input, &fallback, &converted.to_string()).map_err(|e| {
        GenerationFailure::new(FailureKind::Rejected, format!("brief rejected: {e}"))
    })?;
    crate::intel_recon::brief_timing::note(
        crate::intel_recon::brief_timing::Phase::Validation,
        validation_at.elapsed(),
    );
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

#[cfg(test)]
mod summary_answer_tests {
    use super::*;

    #[test]
    fn explicit_empty_summary_is_a_valid_empty_result() {
        let raw = "argos-text-v1\nResult: none\nReason: The neighborhood has no recorded connections yet.";
        match summary_answer(raw).expect("explicit empty parses") {
            SummaryAnswer::Empty(reason) => {
                assert!(reason.contains("no recorded connections"));
            }
            SummaryAnswer::Sections(_) => panic!("empty result misread as sections"),
        }
    }

    #[test]
    fn summary_sections_convert_and_malformed_output_is_rejected() {
        let raw = "argos-text-v1\n## Overview\nText: Two actors connect here.\nBasis: N1; E2\nEnd";
        match summary_answer(raw).expect("sections parse") {
            SummaryAnswer::Sections(value) => {
                assert_eq!(value["overview"][0]["basis"][0], "N1");
            }
            SummaryAnswer::Empty(_) => panic!("sections misread as empty"),
        }
        // A parse failure is never an empty success.
        assert!(summary_answer("argos-text-v1\n## Overview\nText: no basis\nEnd").is_err());
        assert!(summary_answer("not the text contract").is_err());
        assert!(summary_answer("argos-text-v1").is_err());
    }
}
