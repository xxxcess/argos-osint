//! Deterministic OSINT tool diversity and coverage ledger (§4).
//!
//! Enforces the 2-distinct-eligible-tools policy per intelligence category,
//! candidate preference ranking by independent upstream dataset,
//! and persists coverage records in SQLite (`recon_coverage`).

use std::collections::{BTreeMap, HashSet};

use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::osint::contracts::IntelligenceCategory;
use crate::osint::{self, ProviderKeys, ToolResult};
use crate::provider::ReconLimits;
use crate::recon::{Binding, PlanCall, Store};
use crate::telemetry::{EventKind, TelemetryEvent};

/// Authorship and verification state for an intelligence category within a turn.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct CoverageRecord {
    pub scope: String,
    pub generation: u32,
    pub directive_index: u32,
    pub category: String,
    pub candidates: Vec<CoverageCandidate>,
    pub planned_tools: Vec<String>,
    pub attempted_tools: Vec<String>,
    pub successful_tools: Vec<String>,
    pub provider_datasets: Vec<String>,
    pub evidence_ids: Vec<String>,
    pub independent_source_groups: Vec<String>,
    pub engine_query_states: Vec<EngineQueryState>,
    pub coverage_gap: Option<String>,
    pub two_tools_attempted: bool,
    pub two_tools_successful: bool,
    pub independent_claim_support: bool,
}

/// A candidate tool considered during category planning with its eligibility status.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CoverageCandidate {
    pub tool_id: String,
    pub canonical_id: String,
    pub provider: String,
    pub dataset_id: String,
    pub eligible: bool,
    pub rejection_reason: Option<String>,
    pub cost_credits: u32,
}

/// Search engine query dispatch and progress state.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct EngineQueryState {
    pub engine: String,
    pub query: String,
    pub state: String,
}

/// Determine whether a tool has prerequisite bindings satisfied.
pub fn prerequisites_satisfied(tool_id: &str, bindings: &[Binding]) -> (bool, Vec<String>) {
    let def = match osint::definition(tool_id) {
        Some(d) => d,
        None => return (false, vec!["unknown tool".into()]),
    };
    let mut missing = Vec::new();
    for req in def.inputs {
        let alternatives: Vec<&str> = req.split('|').collect();
        let satisfied = alternatives.iter().any(|alt| {
            bindings
                .iter()
                .any(|b| b.kind == *alt && !b.value.trim().is_empty())
        });
        if !satisfied {
            missing.push(req.to_string());
        }
    }
    (missing.is_empty(), missing)
}

/// Check if credentials exist for a tool.
pub fn credentials_available(tool_id: &str, keys: &ProviderKeys) -> bool {
    let canonical = osint::canonical_tool_id(tool_id);
    if canonical.starts_with("firecrawl_") {
        let (p, f) = keys.pair("firecrawl");
        !p.is_empty() || !f.is_empty()
    } else if canonical.starts_with("sociavault_") {
        let (p, f) = keys.pair("sociavault");
        !p.is_empty() || !f.is_empty()
    } else if canonical.starts_with("hunter_") {
        let (p, f) = keys.pair("hunter");
        !p.is_empty() || !f.is_empty()
    } else if canonical.starts_with("newsapi_") {
        let (p, f) = keys.pair("newsapi");
        !p.is_empty() || !f.is_empty()
    } else if canonical == "gnews_search" {
        !keys.gnews.trim().is_empty()
    } else if canonical == "newsdata_latest" {
        !keys.newsdata.trim().is_empty()
    } else if canonical == "currents_latest" {
        !keys.currents.trim().is_empty()
    } else if canonical.starts_with("courtlistener_") {
        !keys.courtlistener.trim().is_empty()
    } else if canonical == "whoxy_whois_history" {
        let (p, f) = keys.pair("whoxy");
        !p.is_empty() || !f.is_empty()
    } else {
        // Free / public catalog tools
        true
    }
}

/// Plan category diversity for a single directive and intelligence category.
pub fn plan_category_diversity(
    scope: &str,
    generation: u32,
    directive_index: u32,
    category: IntelligenceCategory,
    bindings: &[Binding],
    keys: &ProviderKeys,
    limits: &ReconLimits,
    store: &Store,
) -> CoverageRecord {
    let ranked = IntelligenceCategory::ranked_candidates_for(category);
    let mut candidates = Vec::new();
    let mut planned = Vec::new();
    let mut chosen_datasets = HashSet::new();

    for &tool_id in ranked {
        let canonical = osint::canonical_tool_id(tool_id);
        let dataset = IntelligenceCategory::dataset_for_tool(canonical);
        let enabled = store.tool_enabled(canonical).unwrap_or(true);
        let has_creds = credentials_available(canonical, keys);
        let (has_bindings, missing_reqs) = prerequisites_satisfied(canonical, bindings);
        let cost = osint::endpoint_cost(canonical)
            .map(|c| c.credits)
            .unwrap_or(0);
        let provider = osint::primary_provider(canonical).unwrap_or("public");

        let mut rejection_reason = None;
        if !enabled {
            rejection_reason = Some("disabled in catalog".into());
        } else if !has_creds {
            rejection_reason = Some(format!("missing API key for {provider}"));
        } else if !has_bindings {
            rejection_reason = Some(format!("missing bindings: {}", missing_reqs.join(", ")));
        } else if let Some((prov, req_cost)) =
            limits.configured_cost_for(canonical, &serde_json::Value::Null)
        {
            if req_cost > 0 && store.credits_available(prov, limits).unwrap_or(0) < req_cost {
                rejection_reason = Some(format!("credit allowance exhausted for {prov}"));
            }
        }

        let eligible = rejection_reason.is_none();
        candidates.push(CoverageCandidate {
            tool_id: tool_id.to_string(),
            canonical_id: canonical.to_string(),
            provider: provider.to_string(),
            dataset_id: dataset.to_string(),
            eligible,
            rejection_reason: rejection_reason.clone(),
            cost_credits: cost,
        });

        if eligible && planned.len() < 2 {
            // Prefer distinct upstream datasets
            if planned.is_empty() || !chosen_datasets.contains(dataset) {
                planned.push(canonical.to_string());
                chosen_datasets.insert(dataset.to_string());
            } else if planned.len() == 1 && chosen_datasets.contains(dataset) {
                // If only same dataset is available as second candidate, record it if no distinct exists
                planned.push(canonical.to_string());
            }
        }
    }

    let gap = if planned.len() < 2 {
        if category == IntelligenceCategory::PublisherContext
            || category == IntelligenceCategory::EmailRegistration
        {
            None // Single tool categories legitimately have only one tool
        } else if planned.is_empty() {
            Some(format!(
                "no eligible tool for {}: {}",
                category.as_str(),
                candidates
                    .iter()
                    .filter_map(|c| c.rejection_reason.as_deref())
                    .collect::<Vec<_>>()
                    .join("; ")
            ))
        } else {
            Some(format!(
                "second distinct tool unavailable for {}: single tool {} planned",
                category.as_str(),
                planned[0]
            ))
        }
    } else {
        None
    };

    CoverageRecord {
        scope: scope.to_string(),
        generation,
        directive_index,
        category: format!("{:?}", category),
        candidates,
        planned_tools: planned,
        attempted_tools: Vec::new(),
        successful_tools: Vec::new(),
        provider_datasets: chosen_datasets.into_iter().collect(),
        evidence_ids: Vec::new(),
        independent_source_groups: Vec::new(),
        engine_query_states: Vec::new(),
        coverage_gap: gap,
        two_tools_attempted: false,
        two_tools_successful: false,
        independent_claim_support: false,
    }
}

/// Update coverage record with executed calls and results.
pub fn update_coverage_with_results(
    record: &mut CoverageRecord,
    calls: &[PlanCall],
    results: &[(String, ToolResult)],
) {
    let mut attempted = HashSet::new();
    let mut successful = HashSet::new();
    let mut ev_ids = Vec::new();
    let mut source_groups = HashSet::new();

    for call in calls {
        let canonical = osint::canonical_tool_id(&call.tool_id);
        if let Some(cat) = IntelligenceCategory::for_tool(canonical) {
            if format!("{cat:?}") == record.category {
                attempted.insert(canonical.to_string());
            }
        }
    }

    for (id, res) in results {
        let canonical = osint::canonical_tool_id(&res.tool_id);
        if let Some(cat) = IntelligenceCategory::for_tool(canonical) {
            if format!("{cat:?}") == record.category {
                attempted.insert(canonical.to_string());
                if res.status == "completed" {
                    successful.insert(canonical.to_string());
                    ev_ids.push(id.clone());
                    let dataset = IntelligenceCategory::dataset_for_tool(canonical);
                    source_groups.insert(dataset.to_string());
                }
            }
        }
    }

    record.attempted_tools = attempted.into_iter().collect();
    record.successful_tools = successful.into_iter().collect();
    record.evidence_ids = ev_ids;
    record.independent_source_groups = source_groups.into_iter().collect();
    record.two_tools_attempted = record.attempted_tools.len() >= 2;
    record.two_tools_successful = record.successful_tools.len() >= 2;
    record.independent_claim_support = record.independent_source_groups.len() >= 2;
}

/// Save a coverage record to the `recon_coverage` table.
pub fn save_coverage_record(store: &Store, record: &CoverageRecord) -> Result<()> {
    let payload = serde_json::to_string(record)?;
    let now = Utc::now().to_rfc3339();
    let id = coverage_record_id(record);
    store.conn.execute(
        "INSERT INTO recon_coverage(id, scope, generation, directive_index, category, payload_json, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(scope, generation, directive_index, category) DO UPDATE SET
           payload_json = excluded.payload_json,
           updated_at = excluded.updated_at",
        rusqlite::params![
            id,
            record.scope,
            record.generation,
            record.directive_index,
            record.category,
            payload,
            now,
        ],
    )?;
    note_coverage(store, record);
    Ok(())
}

/// Durable id of one coverage record; the same key the table's unique index uses.
pub fn coverage_record_id(record: &CoverageRecord) -> String {
    format!(
        "{}:{}:{}:{}",
        record.scope, record.generation, record.directive_index, record.category
    )
}

/// Aggregate-friendly view of one coverage record, for the Profile dashboard.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CoverageStats {
    /// Eligible candidate tools considered for this category.
    pub eligible_tools: usize,
    /// Tools planned for execution (the two-distinct-tools target).
    pub planned_tools: usize,
    /// Tools that actually ran.
    pub attempted_tools: usize,
    /// Tools that completed with usable output.
    pub successful_tools: usize,
    /// Independent upstream source groups that produced evidence.
    pub independent_source_groups: usize,
    /// Two distinct eligible tools were attempted.
    pub two_tools_attempted: bool,
    /// Two distinct eligible tools completed.
    pub two_tools_successful: bool,
    /// Two independent upstream datasets supported the claim.
    pub independent_claim_support: bool,
    /// The scope carried the two-eligible-tools target, so it may contribute a
    /// coverage rate. Scopes with fewer eligible tools are marked ineligible and
    /// must not be averaged into one.
    pub eligible_for_rate: bool,
    /// Bounded label of the most frequent shortfall, when a target was missed.
    pub shortfall_reason: Option<&'static str>,
}

/// Coverage counts and the most frequent shortfall reason for one record.
/// Provider diversity and independent upstream sources are counted separately.
pub fn coverage_stats(record: &CoverageRecord) -> CoverageStats {
    let eligible_tools = record.candidates.iter().filter(|c| c.eligible).count();
    let stats = CoverageStats {
        eligible_tools,
        planned_tools: record.planned_tools.len(),
        attempted_tools: record.attempted_tools.len(),
        successful_tools: record.successful_tools.len(),
        independent_source_groups: record.independent_source_groups.len(),
        two_tools_attempted: record.two_tools_attempted,
        two_tools_successful: record.two_tools_successful,
        independent_claim_support: record.independent_claim_support,
        eligible_for_rate: eligible_tools >= 2,
        shortfall_reason: None,
    };
    CoverageStats {
        shortfall_reason: top_shortfall_reason(record, &stats),
        ..stats
    }
}

/// A scope met every recorded target: two tools planned, two attempted, two
/// successful and two independent upstream source groups.
fn targets_met(stats: &CoverageStats) -> bool {
    stats.planned_tools >= 2
        && stats.two_tools_attempted
        && stats.two_tools_successful
        && stats.independent_source_groups >= 2
}

/// The most frequent shortfall reason for one record, as a bounded label.
///
/// A record that met every target has no shortfall at all. Otherwise the
/// record's own `coverage_gap` is authoritative: it is written while the category
/// is planned, which is the only point at which a gap recorded before any tool
/// ran can be seen — counts re-derived after the fact cannot recover it. When no
/// gap was persisted the candidate rejection reasons are counted instead, the
/// most frequent wins, and a lexicographic tie-break keeps aggregation over
/// records deterministic.
fn top_shortfall_reason(record: &CoverageRecord, stats: &CoverageStats) -> Option<&'static str> {
    if targets_met(stats) {
        return None;
    }
    if let Some(label) = record.coverage_gap.as_deref().and_then(shortfall_label) {
        return Some(label);
    }
    let mut counts: BTreeMap<&'static str, usize> = BTreeMap::new();
    for candidate in record.candidates.iter().filter(|c| !c.eligible) {
        if let Some(reason) = candidate.rejection_reason.as_deref() {
            if let Some(label) = shortfall_label(reason) {
                *counts.entry(label).or_default() += 1;
            }
        }
    }
    if !counts.is_empty() {
        // Most frequent first, then lexicographic, so the winner never depends
        // on candidate order.
        let mut ranked: Vec<(&'static str, usize)> = counts.into_iter().collect();
        ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
        return ranked.first().map(|(label, _)| *label);
    }
    Some(derived_shortfall_reason(stats))
}

/// Re-derived reason for a record that persisted no gap: the same
/// most-specific-first order the dashboard label set is built from.
fn derived_shortfall_reason(stats: &CoverageStats) -> &'static str {
    if stats.planned_tools == 0 {
        "no_eligible_tool"
    } else if stats.planned_tools < 2 {
        "second_distinct_tool_unavailable"
    } else if !stats.two_tools_attempted {
        "single_tool_attempted"
    } else if !stats.two_tools_successful {
        "second_tool_unsuccessful"
    } else {
        "single_source_group"
    }
}

/// Bounded label for a recorded coverage gap or candidate rejection reason.
///
/// `plan_category_diversity` keeps prose so a human can read why a category
/// fell short; aggregation needs a flat label set, so the leading clause is
/// mapped onto a label. Prose this does not recognise keeps a stable `other`
/// label instead of leaking unbounded text into analytics.
fn shortfall_label(reason: &str) -> Option<&'static str> {
    let head = reason
        .split([':', ';'])
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if head.is_empty() {
        return None;
    }
    if head.contains("no eligible tool") {
        Some("no_eligible_tool")
    } else if head.contains("second distinct tool unavailable") {
        Some("second_distinct_tool_unavailable")
    } else if head.contains("credit allowance") {
        Some("credit_allowance_exhausted")
    } else if head.contains("missing api key") {
        Some("missing_api_key")
    } else if head.contains("missing bindings") {
        Some("missing_bindings")
    } else if head.contains("disabled in catalog") {
        Some("disabled_in_catalog")
    } else {
        Some("other")
    }
}

/// One `recon_stage`-adjacent row per saved coverage record. The row is keyed by
/// the durable record id, so a save and its later update cannot both count, and
/// it carries counts rather than prose.
fn note_coverage(store: &Store, record: &CoverageRecord) {
    let stats = coverage_stats(record);
    let id = format!("coverage-{}", coverage_record_id(record));
    let event = TelemetryEvent::new(EventKind::ReconStage)
        .with_id(id)
        .canonical(coverage_record_id(record))
        .run(&record.scope)
        .category(&record.category)
        .mode("coverage")
        .outcome(if stats.shortfall_reason.is_some() {
            "coverage_gap"
        } else {
            "covered"
        })
        .reason(stats.shortfall_reason.unwrap_or_default())
        .count(1)
        .payload(serde_json::json!({
            "scope": record.scope,
            "generation": record.generation,
            "directive_index": record.directive_index,
            "eligible_tools": stats.eligible_tools,
            "planned_tools": stats.planned_tools,
            "attempted_tools": stats.attempted_tools,
            "successful_tools": stats.successful_tools,
            "independent_source_groups": stats.independent_source_groups,
            "two_tools_attempted": stats.two_tools_attempted,
            "two_tools_successful": stats.two_tools_successful,
            "independent_claim_support": stats.independent_claim_support,
            "eligible_for_rate": stats.eligible_for_rate,
            "provider_datasets": record.provider_datasets.len(),
            "shortfall_reason": stats.shortfall_reason,
        }));
    // Best effort: a telemetry failure never fails coverage persistence.
    let _ = crate::telemetry::insert(&store.conn, &event);
}

/// Load all coverage records for a scope and generation.
pub fn load_coverage_records(
    store: &Store,
    scope: &str,
    generation: u32,
) -> Result<Vec<CoverageRecord>> {
    let mut stmt = store.conn.prepare(
        "SELECT payload_json FROM recon_coverage WHERE scope=?1 AND generation=?2 ORDER BY directive_index, category",
    )?;
    let rows = stmt.query_map([scope, &generation.to_string()], |row| {
        let json_str: String = row.get(0)?;
        Ok(json_str)
    })?;
    let mut records = Vec::new();
    for row in rows {
        let json_str = row?;
        if let Ok(record) = serde_json::from_str::<CoverageRecord>(&json_str) {
            records.push(record);
        }
    }
    Ok(records)
}

/// Summarize active coverage gaps for synthesis prompts or UI display.
pub fn coverage_gaps_summary(records: &[CoverageRecord]) -> Option<String> {
    let gaps: Vec<&str> = records
        .iter()
        .filter_map(|r| r.coverage_gap.as_deref())
        .collect();
    if gaps.is_empty() {
        None
    } else {
        Some(gaps.join("; "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_prerequisites_satisfied() {
        let bindings = vec![Binding {
            kind: "domain".into(),
            value: "example.com".into(),
            evidence_id: "e1".into(),
            source_tool: String::new(),
            ..Binding::default()
        }];
        let (sat, missing) = prerequisites_satisfied("crtsh_certificates", &bindings);
        assert!(sat);
        assert!(missing.is_empty());

        let (sat, missing) = prerequisites_satisfied("blockchain_address", &bindings);
        assert!(!sat);
        assert!(!missing.is_empty());
    }

    #[test]
    fn test_plan_category_diversity_and_persistence() {
        let store = Store::memory().unwrap();
        let keys = ProviderKeys {
            firecrawl: "key".into(),
            ..ProviderKeys::default()
        };
        let limits = ReconLimits::default();

        let bindings = vec![Binding {
            kind: "domain".into(),
            value: "example.com".into(),
            evidence_id: "e1".into(),
            source_tool: String::new(),
            ..Binding::default()
        }];

        let record = plan_category_diversity(
            "run-test",
            1,
            0,
            IntelligenceCategory::DomainNetwork,
            &bindings,
            &keys,
            &limits,
            &store,
        );

        assert_eq!(record.scope, "run-test");
        assert_eq!(record.generation, 1);
        assert_eq!(record.directive_index, 0);
        // Public domain tools like crtsh_certificates and mnemonic_passive_dns should be planned
        assert!(record.planned_tools.len() >= 2);

        save_coverage_record(&store, &record).unwrap();
        let loaded = load_coverage_records(&store, "run-test", 1).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].scope, "run-test");
        assert_eq!(loaded[0].planned_tools, record.planned_tools);
    }

    #[test]
    fn test_update_coverage_with_results() {
        let mut record = CoverageRecord {
            scope: "test".into(),
            generation: 1,
            directive_index: 0,
            category: "DomainNetwork".into(),
            ..CoverageRecord::default()
        };

        let calls = vec![
            PlanCall {
                step_id: "s1".into(),
                tool_id: "crtsh_certificates".into(),
                arguments: json!({"domain": "example.com"}),
                ..PlanCall::default()
            },
            PlanCall {
                step_id: "s2".into(),
                tool_id: "mnemonic_passive_dns".into(),
                arguments: json!({"query": "example.com"}),
                ..PlanCall::default()
            },
        ];

        let results = vec![
            (
                "call-1".into(),
                ToolResult {
                    tool_id: "crtsh_certificates".into(),
                    inputs: json!({"domain": "example.com"}),
                    status: "completed".into(),
                    source_url: "https://crt.sh".into(),
                    retrieved_at: Utc::now().to_rfc3339(),
                    observations: json!({}),
                    raw: "{}".into(),
                    error: None,
                    cached: false,
                    truncated: false,
                    credits_charged: 0,
                    credits_reported: None,
                },
            ),
            (
                "call-2".into(),
                ToolResult {
                    tool_id: "mnemonic_passive_dns".into(),
                    inputs: json!({"query": "example.com"}),
                    status: "completed".into(),
                    source_url: "https://api.mnemonic.no".into(),
                    retrieved_at: Utc::now().to_rfc3339(),
                    observations: json!({}),
                    raw: "{}".into(),
                    error: None,
                    cached: false,
                    truncated: false,
                    credits_charged: 0,
                    credits_reported: None,
                },
            ),
        ];

        update_coverage_with_results(&mut record, &calls, &results);
        assert!(record.two_tools_attempted);
        assert!(record.two_tools_successful);
        assert!(record.independent_claim_support);
        assert_eq!(record.evidence_ids.len(), 2);
    }

    fn covered_record() -> CoverageRecord {
        let mut record = CoverageRecord {
            scope: "run-stats".into(),
            generation: 1,
            directive_index: 0,
            category: "DomainNetwork".into(),
            candidates: vec![
                CoverageCandidate {
                    tool_id: "crtsh_certificates".into(),
                    canonical_id: "crtsh_certificates".into(),
                    provider: "public".into(),
                    dataset_id: "crtsh".into(),
                    eligible: true,
                    rejection_reason: None,
                    cost_credits: 0,
                },
                CoverageCandidate {
                    tool_id: "mnemonic_passive_dns".into(),
                    canonical_id: "mnemonic_passive_dns".into(),
                    provider: "public".into(),
                    dataset_id: "mnemonic".into(),
                    eligible: false,
                    rejection_reason: Some("missing API key".into()),
                    cost_credits: 0,
                },
            ],
            planned_tools: vec!["crtsh_certificates".into()],
            attempted_tools: vec!["crtsh_certificates".into()],
            successful_tools: vec!["crtsh_certificates".into()],
            independent_source_groups: vec!["crtsh".into()],
            coverage_gap: Some("second distinct tool unavailable".into()),
            ..CoverageRecord::default()
        };
        record.two_tools_attempted = record.attempted_tools.len() >= 2;
        record.two_tools_successful = record.successful_tools.len() >= 2;
        record.independent_claim_support = record.independent_source_groups.len() >= 2;
        record
    }

    #[test]
    fn test_coverage_stats_reports_the_top_shortfall_reason() {
        let stats = coverage_stats(&covered_record());
        assert_eq!(stats.eligible_tools, 1);
        assert_eq!(stats.planned_tools, 1);
        assert_eq!(stats.attempted_tools, 1);
        assert_eq!(stats.successful_tools, 1);
        assert_eq!(stats.independent_source_groups, 1);
        assert!(!stats.two_tools_attempted);
        assert_eq!(
            stats.shortfall_reason,
            Some("second_distinct_tool_unavailable")
        );

        // A record that met every target has no shortfall at all.
        let mut full = covered_record();
        full.planned_tools = vec!["crtsh_certificates".into(), "mnemonic_passive_dns".into()];
        full.attempted_tools = full.planned_tools.clone();
        full.successful_tools = full.planned_tools.clone();
        full.independent_source_groups = vec!["crtsh".into(), "mnemonic".into()];
        full.two_tools_attempted = true;
        full.two_tools_successful = true;
        full.independent_claim_support = true;
        let stats = coverage_stats(&full);
        assert!(stats.two_tools_attempted && stats.two_tools_successful);
        assert_eq!(stats.independent_source_groups, 2);
        assert_eq!(stats.shortfall_reason, None);

        // Nothing eligible is reported before any tool runs.
        let empty = CoverageRecord {
            coverage_gap: Some("no eligible tool".into()),
            ..covered_record()
        };
        assert_eq!(
            coverage_stats(&empty).shortfall_reason,
            Some("no_eligible_tool")
        );
    }

    #[test]
    fn test_save_coverage_record_writes_one_telemetry_row() {
        let store = Store::memory().unwrap();
        let record = covered_record();
        save_coverage_record(&store, &record).unwrap();
        // A later update of the same record must not double count.
        let mut updated = record.clone();
        updated.successful_tools = vec!["crtsh_certificates".into()];
        save_coverage_record(&store, &updated).unwrap();

        let id = coverage_record_id(&record);
        let rows: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM telemetry_events WHERE id = ?1",
                [format!("coverage-{id}")],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(rows, 1, "one authoritative row per coverage record");
        let category: String = store
            .conn
            .query_row(
                "SELECT category FROM telemetry_events WHERE id = ?1",
                [format!("coverage-{id}")],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(category, "DomainNetwork");
    }
}
