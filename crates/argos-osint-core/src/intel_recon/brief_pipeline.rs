//! Intel brief preparation (`intel-brief-v2`): one durable job per article
//! generation with four operations and explicit dependencies
//! (`body=[]; claims=[]; inferences=[body,claims]; links=[inferences]`).
//!
//! Admission is idempotent and atomic: one active chain per article, all four
//! tasks persisted immediately. Full article and Claims are independent roots
//! and may run together; dependencies live in the claim SQL
//! (`tasks::claim_with`), and stages are routed to the dedicated
//! [`tasks::POOL_INTEL_BRIEF`] pool so unrelated drainers never take them. A
//! stage commit stores its immutable output/hash and, in the same fenced
//! transaction, runs the durable join ([`bind_ready`]): a descendant is bound
//! once, by compare-and-set, only when every prerequisite has committed.

use anyhow::Result;

/// Article-preparation contract version (distinct from the Recon/report
/// extraction contract).
pub const CONTRACT: &str = "intel-brief-v2";
/// The legacy serial contract. Completed v1 history stays intact; active v1
/// chains are fenced at cutover and replaced by a v2 generation.
pub const CONTRACT_V1: &str = "intel-brief-v1";
/// Brain canonical extraction scope. Unchanged by the v2 cutover so Brain
/// Claim/Inference identities stay stable.
pub const CANONICAL_CONTRACT: &str = "intel-brief-v1";

/// Explicit v2 dependencies: (operation, prerequisites).
pub const DEPENDENCIES: [(&str, &[&str]); 4] = [
    ("intel_brief_body", &[]),
    ("intel_brief_claims", &[]),
    (
        "intel_brief_inferences",
        &["intel_brief_body", "intel_brief_claims"],
    ),
    ("intel_brief_links", &["intel_brief_inferences"]),
];

/// Declared prerequisites of an operation.
pub fn prerequisites(operation: &str) -> &'static [&'static str] {
    DEPENDENCIES
        .iter()
        .find(|(op, _)| *op == operation)
        .map(|(_, deps)| *deps)
        .unwrap_or(&[])
}

fn label_of(operation: &str) -> &'static str {
    STAGES
        .iter()
        .find(|(op, _)| *op == operation)
        .map(|(_, l)| *l)
        .unwrap_or("stage")
}

/// Short key used in a bound join input (`{body:{..},claims:{..}}`).
fn short_key(operation: &str) -> &str {
    operation.strip_prefix("intel_brief_").unwrap_or(operation)
}

/// (operation, display label) in dependency order.
pub const STAGES: [(&str, &str); 4] = [
    ("intel_brief_body", "Full article"),
    ("intel_brief_claims", "Claims"),
    ("intel_brief_inferences", "Inferences"),
    ("intel_brief_links", "Links"),
];

/// Stable identity of one preparation run's inputs.
#[derive(Clone, Debug, Default)]
pub struct BriefInputs {
    pub article_id: String,
    /// Article metadata/acquisition revision, related metadata revisions and
    /// captured role chain folded into one fingerprint by the caller.
    pub input_revision: String,
    /// Captured role settings (JSON) for every stage.
    pub role_snapshot: String,
    pub title: String,
    /// Per-node semantic revisions in [`STAGES`] order (spec §6). Each node
    /// is keyed independently: its own source revision, contract/prompt
    /// version and resolved route/policy. Empty disables reuse.
    pub stage_revisions: Vec<String>,
    /// Immutable metadata snapshot (title/description/published/source
    /// revision) persisted in the root task inputs, so both branches read
    /// the same captured values instead of mutable article rows.
    pub metadata: serde_json::Value,
}

/// Task id of one stage in a chain.
pub fn task_id(job_id: &str, operation: &str) -> String {
    format!("{job_id}:{operation}")
}

/// One published lead row for presentation ("Potential corroboration").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublishedLead {
    pub inference_id: String,
    pub inference_text: String,
    pub candidate_article_id: String,
    pub candidate_run_id: String,
    pub interpretation: String,
    pub verification_needed: String,
}

fn checkpoint_key(job_id: &str, stage: &str, item: &str) -> String {
    format!("intel-brief-ckpt:{job_id}:{stage}:{item}")
}

/// One immutable stage row for every surface (brief Jobs panel, section
/// headers, global Jobs, Logs).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StageView {
    pub operation: String,
    pub label: &'static str,
    pub state: String,
    /// Display row, e.g. `Queued · after Claims`, `Completed · 3 items`.
    pub row: String,
    pub active: bool,
    /// Extra lines for a failed stage or one blocked by a failed predecessor:
    /// the failure, a Providers hint for a missing key, and how to retry.
    /// Empty when there is nothing to say.
    pub detail: String,
}

/// Longest failure text shown in a stage pane.
const DETAIL_ERROR_CHARS: usize = 220;

/// First line of a stage error, bounded for display.
fn detail_error(reason: &str) -> String {
    let first = reason.lines().next().unwrap_or("").trim();
    let mut out: String = first.chars().take(DETAIL_ERROR_CHARS).collect();
    if first.chars().count() > DETAIL_ERROR_CHARS {
        out.push('…');
    }
    out
}

/// Pane lines for a failed stage (`own`) or one blocked by `subject`'s failure.
/// Compact by design: a failed stage adds its full error only when the row
/// truncated it, plus one recovery line; a blocked stage adds one line.
fn failure_detail(subject: &str, own: bool, error: &str) -> String {
    let error = detail_error(error);
    if error.is_empty() {
        return String::new();
    }
    let lower = error.to_lowercase();
    let recovery = if lower.contains("api key missing") || lower.contains("all routes blocked") {
        "Configure a provider in Providers, then Retry article retrieval."
    } else {
        "Retry article retrieval resumes the failed stage."
    };
    if !own {
        // The cause sits in the failed stage's own pane; only recovery repeats.
        return format!("{subject} failed · {recovery}");
    }
    if short(&error) != error {
        format!("{error}\n{recovery}")
    } else {
        recovery.to_string()
    }
}

fn short(reason: &str) -> String {
    let first = reason.split(['\n', ';']).next().unwrap_or("").trim();
    let mut out: String = first.chars().take(60).collect();
    if first.chars().count() > 60 {
        out.push('…');
    }
    if out.is_empty() {
        "unknown error".into()
    } else {
        out
    }
}

/// One stage task as the views need it, from either store.
#[derive(Debug, Clone, Default)]
pub struct StageRow {
    pub state: String,
    pub error: String,
    /// Prerequisites `(operation, state)` in priority order.
    pub deps: Vec<(String, String)>,
    /// Committed `(item_count, empty_reason)`.
    pub output: Option<(i64, String)>,
    /// Live activity label of a running stage.
    pub activity: Option<String>,
}

/// [`stage_views`] over rows from `fetch` (by operation; `None` skips it).
pub fn stage_views_from(
    mut fetch: impl FnMut(&str) -> Result<Option<StageRow>>,
) -> Result<Vec<StageView>> {
    let mut out: Vec<StageView> = Vec::new();
    let mut failed_by: std::collections::HashMap<String, (&'static str, String)> =
        Default::default();
    for (operation, label) in STAGES {
        let Some(StageRow {
            state,
            error,
            deps,
            output,
            activity,
        }) = fetch(operation)?
        else {
            continue;
        };
        // A failed prerequisite registers itself in `failed_by` below, so one
        // lookup covers direct and transitive blockers.
        let blocker_full = deps.iter().find_map(|(op, _)| failed_by.get(op).cloned());
        let blocker = blocker_full.as_ref().map(|(label, _)| *label);
        let text = match state.as_str() {
            "completed" => match output {
                Some((0, reason)) if !reason.is_empty() => format!("Completed · {reason}"),
                Some((n, reason)) if reason.contains("warning") => {
                    format!(
                        "Completed with warnings · {n} item{}",
                        if n == 1 { "" } else { "s" }
                    )
                }
                Some((n, _)) => format!("Completed · {n} item{}", if n == 1 { "" } else { "s" }),
                None => "Completed".into(),
            },
            "running" => match activity.filter(|a| !a.is_empty()) {
                Some(a) => format!("Running · {a}"),
                None => "Running".into(),
            },
            "retry_scheduled" => "Running · retry scheduled".into(),
            "failed" => format!("Failed · {}", short(&error)),
            "cancelled" => "Cancelled".into(),
            "superseded" => "Superseded".into(),
            _ => match blocker {
                Some(b) => format!("Queued · blocked by {b}"),
                None => {
                    let pending: Vec<&str> = deps
                        .iter()
                        .filter(|(_, st)| st != "completed")
                        .map(|(op, _)| label_of(op))
                        .collect();
                    if pending.is_empty() {
                        "Queued".into()
                    } else {
                        format!("Queued · after {}", pending.join(", "))
                    }
                }
            },
        };
        let detail = if state == "failed" {
            failure_detail(label, true, &error)
        } else if state == "queued" {
            blocker_full
                .as_ref()
                .map(|(who, err)| failure_detail(who, false, err))
                .unwrap_or_default()
        } else {
            String::new()
        };
        if state == "failed" {
            failed_by.insert(operation.to_string(), (label, error.clone()));
        } else if state != "completed" {
            if let Some(b) = blocker_full {
                failed_by.insert(operation.to_string(), b);
            }
        }
        out.push(StageView {
            detail,
            operation: operation.into(),
            label,
            active: state == "running",
            state,
            row: text,
        });
    }
    Ok(out)
}

/// Truthful chain summary for headers and Jobs: `2 running · 0/4 completed`.
pub fn chain_summary(views: &[StageView]) -> String {
    let running = views.iter().filter(|v| v.state == "running").count();
    let completed = views.iter().filter(|v| v.state == "completed").count();
    let failed = views.iter().filter(|v| v.state == "failed").count();
    let mut parts = Vec::new();
    if running > 0 {
        parts.push(format!("{running} running"));
    }
    if failed > 0 {
        parts.push(format!("{failed} failed"));
    }
    parts.push(format!("{completed}/{} completed", views.len()));
    parts.join(" · ")
}

#[cfg(test)]
#[path = "brief_pipeline_tests.rs"]
mod tests;
