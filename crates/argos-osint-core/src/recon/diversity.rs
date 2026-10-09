//! Deterministic OSINT tool diversity and coverage ledger (§4).
//!
//! Enforces the 2-distinct-eligible-tools policy per intelligence category,
//! candidate preference ranking by independent upstream dataset,
//! and persists coverage records in SQLite (`recon_coverage`).

use std::collections::HashSet;

use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::osint::contracts::IntelligenceCategory;
use crate::osint::{self, ProviderKeys, ToolResult};
use crate::provider::ReconLimits;
use crate::recon::{Binding, PlanCall, Store};

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
    let id = format!(
        "{}:{}:{}:{}",
        record.scope, record.generation, record.directive_index, record.category
    );
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
    Ok(())
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
}
