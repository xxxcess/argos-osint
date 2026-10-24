//! One resumable local backfill into the Brain actor/relationship registry
//! (spec §4 "Identity, binding and migration", §5 "Backfill and lifecycle").
//!
//! Sources, in order: legacy `recon_entities` and `investigation_entities`
//! (scoped external refs), insight claim subjects (memory actor refs),
//! `insight_derivation_links` (canonical Link memories),
//! `intel_inference_article_leads` (Related memories) and every retained
//! Atlas article occurrence (Actor + mention receipt + observed
//! `mentioned_in` Link + bounded tentative Related records).
//!
//! Each unit carries a content revision; a receipt with the same revision
//! is skipped, a changed one reindexes only that unit. Units commit in
//! bounded transactions (initially 100) with per-unit savepoints, so a crash
//! resumes at the first unreceipted unit and a failing unit never blocks or
//! falsely completes the rest. No model, network, body or embedding request.

use anyhow::Result;
use serde::Serialize;
use serde_json::Value;

use crate::brain_actors::IDENTITY_INDEX_VERSION;
use crate::store::Store;

pub const BATCH: usize = 100;

pub const SOURCES: &[&str] = &[
    "recon_entities",
    "investigation_entities",
    "recon_thread_entities",
    "insight_claims",
    "insight_derivation_links",
    "intel_inference_article_leads",
    "atlas_articles",
];

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct SourceCoverage {
    pub source: String,
    pub done: usize,
    pub total: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct BackfillReport {
    pub processed: usize,
    pub failed: usize,
    /// Failing units (retried on the next call; never falsely complete).
    pub errors: Vec<String>,
    pub coverage: Vec<SourceCoverage>,
    pub complete: bool,
}

pub(crate) fn h(v: &Value) -> String {
    crate::nlp::cache::hash(&format!(
        "{IDENTITY_INDEX_VERSION}|browse-associations-v2|{}|{v}",
        crate::related_articles::MATCH_VERSION
    ))
}

const ATLAS_RUN_UNITS: &str = "SELECT a.run_id||'/'||a.article_id,
       json_array(a.title,a.description,a.category,
         (SELECT metadata_json FROM atlas_article_metadata m WHERE m.run_id=a.run_id AND m.article_id=a.article_id))
     FROM atlas_articles a WHERE a.run_id=?1 ORDER BY a.article_id";

fn identifiers_of(raw: &str) -> Vec<String> {
    let v: Value = serde_json::from_str(raw).unwrap_or(Value::Null);
    let mut out = Vec::new();
    if let Some(items) = v.as_array() {
        for i in items {
            if let Some(s) = i.as_str() {
                out.push(s.to_string());
            } else if let Some(s) = i.get("value").and_then(Value::as_str) {
                out.push(s.to_string());
            }
        }
    }
    out
}

impl Store {
    /// Run up to `max_units` pending backfill units in transactions of
    /// [`BATCH`]. Safe to call concurrently and repeatedly.
    pub fn brain_backfill(&self, max_units: usize) -> Result<BackfillReport> {
        crate::storage::store_bridge::brain_backfill(max_units)
    }

    /// Per-source historical coverage (receipted units with a current revision).
    pub fn brain_backfill_coverage(&self) -> Result<Vec<SourceCoverage>> {
        crate::storage::store_bridge::brain_backfill_coverage()
    }
}

#[cfg(test)]
#[path = "brain_backfill_tests.rs"]
mod tests;
