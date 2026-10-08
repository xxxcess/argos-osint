//! Authoritative tool contracts and 3-level model-facing catalog.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{canonical_tool_id, endpoint_cost, registry};

/// The 14 official intelligence categories from the spec.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum IntelligenceCategory {
    WebDiscovery,
    NewsEvents,
    PublisherContext,
    LegalProceedings,
    Organizations,
    ProfessionalIdentity,
    SocialContent,
    PublicAccountCorroboration,
    DomainNetwork,
    HistoricalWeb,
    SoftwareCode,
    Geography,
    Bitcoin,
    Vulnerabilities,
}

impl IntelligenceCategory {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::WebDiscovery => "Web discovery/acquisition",
            Self::NewsEvents => "News/events",
            Self::PublisherContext => "Publisher context",
            Self::LegalProceedings => "Legal proceedings",
            Self::Organizations => "Organizations",
            Self::ProfessionalIdentity => "Professional identity/contact",
            Self::SocialContent => "Social content/accounts",
            Self::PublicAccountCorroboration => "Public account corroboration",
            Self::DomainNetwork => "Domain/network relationships",
            Self::HistoricalWeb => "Historical web evidence",
            Self::SoftwareCode => "Software/code",
            Self::Geography => "Geography",
            Self::Bitcoin => "Bitcoin activity",
            Self::Vulnerabilities => "Vulnerabilities/exposure",
        }
    }

    pub fn for_tool(tool_id: &str) -> Option<Self> {
        let tool = canonical_tool_id(tool_id);
        match tool {
            // Web discovery (8)
            "firecrawl_search"
            | "firecrawl_scrape"
            | "firecrawl_map"
            | "firecrawl_batch_scrape"
            | "firecrawl_crawl"
            | "firecrawl_extract"
            | "sociavault_google_search"
            | "dork_generate" => Some(Self::WebDiscovery),

            // News/events (5)
            "newsapi_search" | "newsapi_headlines" | "gnews_search" | "newsdata_latest"
            | "currents_latest" => Some(Self::NewsEvents),

            // Publisher context (1)
            "wikipedia_source_reliability" => Some(Self::PublisherContext),

            // Legal proceedings (3)
            "courtlistener_case_search"
            | "courtlistener_docket_search"
            | "courtlistener_judge_search" => Some(Self::LegalProceedings),

            // Organizations (4)
            "gleif_entities"
            | "sec_submissions"
            | "wikidata_entities"
            | "hunter_company_enrichment" => Some(Self::Organizations),

            // Professional identity/contact (8)
            "hunter_domain_finder"
            | "hunter_email_count"
            | "hunter_domain_search"
            | "hunter_email_finder"
            | "hunter_email_verifier"
            | "hunter_email_insight"
            | "hunter_person_enrichment"
            | "hunter_combined_enrichment" => Some(Self::ProfessionalIdentity),

            // Social content/accounts (4)
            "sociavault_search"
            | "sociavault_search_users"
            | "sociavault_profile"
            | "sociavault_user_content" => Some(Self::SocialContent),

            // Public account corroboration (4)
            "keybase_identity"
            | "stackexchange_users"
            | "wikipedia_users"
            | "whatsmyname_lookup" => Some(Self::PublicAccountCorroboration),

            // Domain/network relationships (6)
            "crtsh_certificates"
            | "mnemonic_passive_dns"
            | "hackertarget_hostsearch"
            | "ripestat_network_info"
            | "arin_rdap"
            | "apnic_rdap" => Some(Self::DomainNetwork),

            // Historical web evidence (3)
            "wayback_availability" | "arquivo_history" | "commoncrawl_urls" => {
                Some(Self::HistoricalWeb)
            }

            // Software/code (3)
            "github_repositories" | "gitlab_projects" | "grepapp_code_search" => {
                Some(Self::SoftwareCode)
            }

            // Geography (3)
            "nominatim_geocode" | "census_geocode" | "overpass_places" => Some(Self::Geography),

            // Bitcoin activity (3)
            "blockchain_address" | "blockstream_address" | "mempool_address" => Some(Self::Bitcoin),

            // Vulnerabilities/exposure (6)
            "nvd_cve" | "cve_record" | "osv_package" | "sans_ip_activity" | "shodan_internetdb"
            | "urlscan_search" => Some(Self::Vulnerabilities),

            _ => None,
        }
    }
}

/// Level 1: Compact capability description for Planner and Controller.
#[derive(Clone, Debug, Serialize)]
pub struct CompactToolCapability {
    pub tool_id: &'static str,
    pub name: &'static str,
    pub category: &'static str,
    pub intelligence_category: &'static str,
    pub description: &'static str,
    pub prerequisites: &'static [&'static str],
    pub limitations: &'static str,
    pub cost_credits: u32,
}

/// Level 2: Picker candidate description for a specific task and available bindings.
#[derive(Clone, Debug, Serialize)]
pub struct PickerCandidate {
    pub tool_id: &'static str,
    pub name: &'static str,
    pub eligible: bool,
    pub missing_prerequisites: Vec<String>,
    pub cost_credits: u32,
    pub expected_contribution: String,
}

/// Level 3: Exact route and argument builder contract for the selected tool.
#[derive(Clone, Debug, Serialize)]
pub struct ArgumentBuilderContract {
    pub tool_id: &'static str,
    pub schema: Value,
    pub allowed_routes: Vec<String>,
    pub examples: Vec<Value>,
    pub failure_guidance: &'static str,
}

/// Returns the Level 1 compact capability catalog across all 59 tools.
pub fn compact_capability_catalog() -> Vec<CompactToolCapability> {
    registry()
        .iter()
        .map(|tool| {
            let intel_cat = IntelligenceCategory::for_tool(tool.id)
                .map(|c| c.as_str())
                .unwrap_or(tool.category);
            let cost = endpoint_cost(tool.id).map(|c| c.credits).unwrap_or(0);

            CompactToolCapability {
                tool_id: tool.id,
                name: tool.name,
                category: tool.category,
                intelligence_category: intel_cat,
                description: tool.description,
                prerequisites: tool.inputs,
                limitations: tool.restrictions,
                cost_credits: cost,
            }
        })
        .collect()
}

/// Returns the Level 2 picker candidates for a task with given available bindings.
pub fn picker_candidates(available_bindings: &[&str]) -> Vec<PickerCandidate> {
    registry()
        .iter()
        .map(|tool| {
            let mut missing = Vec::new();
            for req in tool.inputs {
                // Alternating inputs separated by '|'
                let alternatives: Vec<&str> = req.split('|').collect();
                let satisfied = alternatives
                    .iter()
                    .any(|alt| available_bindings.contains(alt));
                if !satisfied {
                    missing.push(req.to_string());
                }
            }

            let eligible = missing.is_empty();
            let cost = endpoint_cost(tool.id).map(|c| c.credits).unwrap_or(0);
            let contribution = format!("Yields {} evidence", tool.category);

            PickerCandidate {
                tool_id: tool.id,
                name: tool.name,
                eligible,
                missing_prerequisites: missing,
                cost_credits: cost,
                expected_contribution: contribution,
            }
        })
        .collect()
}

/// Returns the Level 3 argument builder contract for a specific tool.
pub fn argument_contract(tool_id: &str) -> Option<ArgumentBuilderContract> {
    let def = super::definition(tool_id)?;
    let schema = json!({
        "type": "object",
        "properties": def.inputs.iter().map(|input| {
            (input.to_string(), json!({"type": "string", "description": format!("Input for {input}")}))
        }).collect::<serde_json::Map<String, Value>>(),
        "required": def.inputs,
    });

    Some(ArgumentBuilderContract {
        tool_id: def.id,
        schema,
        allowed_routes: vec![format!("https://api.argos-osint/{}/v1", def.id)],
        examples: vec![json!({"example": true})],
        failure_guidance: def.restrictions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_61_tools_mapped_to_categories() {
        let tools = registry();
        assert_eq!(tools.len(), 61, "catalog has exactly 61 tools");

        for tool in tools {
            let cat = IntelligenceCategory::for_tool(tool.id);
            assert!(
                cat.is_some(),
                "tool {} must be mapped to an intelligence category",
                tool.id
            );
        }
    }

    #[test]
    fn hunter_tech_lookup_is_alias_to_hunter_company_enrichment() {
        assert_eq!(
            canonical_tool_id("hunter_tech_lookup"),
            "hunter_company_enrichment"
        );
    }
}
