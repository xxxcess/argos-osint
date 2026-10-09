//! Comprehensive acceptance tests for the Unified Investigation Harness.

use serde_json::json;

use super::contracts::InvestigationSurface;
use super::evidence::{
    assess_claim_against_passages, curate_passages_from_result, ClaimStance, EvidencePassage,
};
use super::patterns::InvestigationPattern;
use super::policy::{check_need_gate, is_observation_fresh, SurfacePolicy};
use super::roles::LogicalRole;
use crate::osint::contracts::IntelligenceCategory;
use crate::osint::{canonical_tool_id, registry, ToolResult};
use crate::provider::{ModelAssignment, RoleDefaults};

#[test]
fn test_all_catalog_tools_mapped_to_categories() {
    let reg = registry();
    assert_eq!(reg.len(), 66, "Catalog must contain exactly 66 tools");

    for tool in reg {
        let cat = IntelligenceCategory::for_tool(tool.id);
        assert!(
            cat.is_some(),
            "Tool '{}' must be mapped to an IntelligenceCategory variant",
            tool.id
        );
    }

    // Verify alias resolution for hunter_tech_lookup -> hunter_company_enrichment
    assert_eq!(
        canonical_tool_id("hunter_tech_lookup"),
        "hunter_company_enrichment"
    );
    assert!(IntelligenceCategory::for_tool("hunter_tech_lookup").is_some());
}

#[test]
fn test_surface_policy_protections() {
    let recon_policy = SurfacePolicy::for_surface(InvestigationSurface::ReconChat);
    let intel_policy = SurfacePolicy::for_surface(InvestigationSurface::IntelBrief);
    let home_policy = SurfacePolicy::for_surface(InvestigationSurface::HomeComposer);

    // Atlas-only tools never permitted in Recon or Intel
    assert!(!recon_policy.is_tool_permitted("atlas_gnews"));
    assert!(!recon_policy.is_tool_permitted("atlas_newsdata"));
    assert!(!recon_policy.is_tool_permitted("atlas_currents"));
    assert!(!recon_policy.is_tool_permitted("atlas_newsapi"));

    assert!(!intel_policy.is_tool_permitted("atlas_gnews"));

    // SociaVault is allowed in Recon and Home, but NOT in Intel
    assert!(recon_policy.is_tool_permitted("sociavault_profile"));
    assert!(home_policy.is_tool_permitted("sociavault_profile"));
    assert!(!intel_policy.is_tool_permitted("sociavault_profile"));
}

#[test]
fn test_scarce_provider_preflight_protections() {
    // 1. Weak Firecrawl alone CANNOT authorize SociaVault Google search spending
    // (Must have both weak Firecrawl AND unmet need)
    let res = check_need_gate(
        InvestigationSurface::ReconChat,
        "sociavault_google_search",
        false, // no unmet need
        true,  // firecrawl was weak
    );
    assert!(
        res.is_err(),
        "Weak Firecrawl alone must not authorize SociaVault Google search spending without unmet need"
    );

    // 2. Unmet need alone cannot authorize SociaVault Google search without weak Firecrawl
    let res = check_need_gate(
        InvestigationSurface::ReconChat,
        "sociavault_google_search",
        true,  // has unmet need
        false, // firecrawl was NOT weak
    );
    assert!(
        res.is_err(),
        "SociaVault Google search requires weak Firecrawl discovery first"
    );

    // 3. Both weak Firecrawl AND unmet need allows it
    let res = check_need_gate(
        InvestigationSurface::ReconChat,
        "sociavault_google_search",
        true, // has unmet need
        true, // firecrawl was weak
    );
    assert!(res.is_ok());

    // 4. Platform-native SociaVault calls require unmet need
    let res = check_need_gate(
        InvestigationSurface::ReconChat,
        "sociavault_profile",
        false, // no unmet need
        false,
    );
    assert!(
        res.is_err(),
        "SociaVault call requires unmet platform-native evidence need"
    );

    let res = check_need_gate(
        InvestigationSurface::ReconChat,
        "sociavault_profile",
        true, // has unmet need
        false,
    );
    assert!(res.is_ok());
}

#[test]
fn test_claim_specific_evidence_isolation() {
    // An unrelated supporting item for entity B must NEVER validate a claim about entity A.
    let passages = vec![
        EvidencePassage {
            id: "pass-1".into(),
            investigation_id: "inv-1".into(),
            task_id: "task-1".into(),
            call_id: "call-1".into(),
            source_url: "https://example.com/acme".into(),
            source_domain: "example.com".into(),
            passage_text: "Acme Corp reported $500M annual revenue in fiscal report.".into(),
            observed_at: "2026-03-01T00:00:00Z".into(),
            published_at: "2026-03-01".into(),
            stance: ClaimStance::Supported,
            relevance_score: 1.0,
            created_at: "2026-03-01T00:00:00Z".into(),
        },
        EvidencePassage {
            id: "pass-2".into(),
            investigation_id: "inv-1".into(),
            task_id: "task-2".into(),
            call_id: "call-2".into(),
            source_url: "https://example.com/beta".into(),
            source_domain: "example.com".into(),
            passage_text: "Beta LLC filed bankruptcy petition and ceased profitable operations."
                .into(),
            observed_at: "2026-03-01T00:00:00Z".into(),
            published_at: "2026-03-01".into(),
            stance: ClaimStance::Disputed,
            relevance_score: 1.0,
            created_at: "2026-03-01T00:00:00Z".into(),
        },
    ];

    // Claim about Acme Corp
    let assessment_acme =
        assess_claim_against_passages("claim-acme", "Acme Corp revenue reached 500M", &passages);
    assert_eq!(assessment_acme.stance, ClaimStance::Supported);
    assert_eq!(assessment_acme.cited_passage_ids, vec!["pass-1"]);

    // Claim about Beta LLC
    let assessment_beta =
        assess_claim_against_passages("claim-beta", "Beta LLC operations profitable", &passages);
    assert_eq!(assessment_beta.stance, ClaimStance::Disputed);
    assert_eq!(assessment_beta.cited_passage_ids, vec!["pass-2"]);

    // Claim about an unrelated entity "Zeta Inc"
    let assessment_zeta = assess_claim_against_passages(
        "claim-zeta",
        "Zeta Inc expanded operations globally",
        &passages,
    );
    // Even though pass-1 is "Supported", it does NOT apply to Zeta Inc!
    assert_eq!(assessment_zeta.stance, ClaimStance::Insufficient);
    assert!(assessment_zeta.cited_passage_ids.is_empty());
}

#[test]
fn test_role_defaults_inheritance_hierarchy() {
    let mut defaults = RoleDefaults {
        recon: ModelAssignment {
            provider: "grok".into(),
            model: "grok-4.6".into(),
            ..Default::default()
        },
        synthesis: ModelAssignment {
            provider: "openai".into(),
            model: "gpt-4.5".into(),
            ..Default::default()
        },
        classifier: ModelAssignment {
            provider: "openrouter".into(),
            model: "typesafe/jev-1.13".into(),
            ..Default::default()
        },
        ..Default::default()
    };

    // Test default inheritance:
    // 1. Evidence Curator inherits Recon
    let (curator, parent) = defaults.resolve_role("evidence_curator");
    assert_eq!(parent, Some("recon"));
    assert_eq!(curator.provider, "grok");
    assert_eq!(curator.model, "grok-4.6");

    // 2. Entity Resolver inherits Classifier
    let (resolver, parent) = defaults.resolve_role("entity_resolver");
    assert_eq!(parent, Some("classifier"));
    assert_eq!(resolver.provider, "openrouter");
    assert_eq!(resolver.model, "typesafe/jev-1.13");

    // 3. Claim Assessor inherits Synthesis
    let (assessor, parent) = defaults.resolve_role("claim_assessor");
    assert_eq!(parent, Some("synthesis"));
    assert_eq!(assessor.provider, "openai");
    assert_eq!(assessor.model, "gpt-4.5");

    // 4. Investigation Controller inherits Recon
    let (controller, parent) = defaults.resolve_role("investigation_controller");
    assert_eq!(parent, Some("recon"));
    assert_eq!(controller.provider, "grok");

    // 5. Summarization inherits Synthesis
    let (summary, parent) = defaults.resolve_role("summarization");
    assert_eq!(parent, Some("synthesis"));
    assert_eq!(summary.provider, "openai");

    // Explicit override overrides inheritance
    defaults.evidence_curator = ModelAssignment {
        provider: "local".into(),
        model: "mistral-7b".into(),
        ..Default::default()
    };
    let (curator_explicit, parent) = defaults.resolve_role("evidence_curator");
    assert_eq!(parent, None);
    assert_eq!(curator_explicit.provider, "local");
    assert_eq!(curator_explicit.model, "mistral-7b");
}

#[test]
fn test_freshness_cutoff_verification() {
    let cutoff = "2026-03-01T12:00:00Z";

    // Observation before cutoff is not fresh
    assert!(!is_observation_fresh("2026-03-01T11:59:59Z", cutoff));
    assert!(!is_observation_fresh("2026-02-28T00:00:00Z", cutoff));

    // Observation at or after cutoff is fresh
    assert!(is_observation_fresh("2026-03-01T12:00:00Z", cutoff));
    assert!(is_observation_fresh("2026-03-01T12:00:01Z", cutoff));

    // Empty cutoff implies fresh
    assert!(is_observation_fresh("2025-01-01T00:00:00Z", ""));
}

#[test]
fn test_curate_passages_from_result() {
    let result = ToolResult {
        tool_id: "firecrawl_search".into(),
        inputs: json!({"query": "Jane Doe"}),
        status: "ok".into(),
        source_url: "https://company.org/about".into(),
        retrieved_at: "2026-03-02T10:00:00Z".into(),
        observations: json!({
            "results": [
                {
                    "url": "https://company.org/about",
                    "title": "About Company",
                    "snippet": "Company was founded in 2020 by Jane Doe."
                }
            ]
        }),
        raw: "{}".into(),
        error: None,
        cached: false,
        truncated: false,
        credits_charged: 1,
        credits_reported: Some(1),
    };

    let passages = curate_passages_from_result("inv-1", "task-1", "call-99", &result);
    assert_eq!(passages.len(), 1);
    assert_eq!(passages[0].source_domain, "company.org");
    assert!(passages[0].passage_text.contains("Jane Doe"));
}

#[test]
fn test_investigation_patterns() {
    let pat =
        InvestigationPattern::detect("Investigate email address alice@example.com", &["email"]);
    assert_eq!(pat, InvestigationPattern::EmailAttribution);

    let pat_btc = InvestigationPattern::detect(
        "Track transactions for bitcoin address",
        &["crypto_address"],
    );
    assert_eq!(pat_btc, InvestigationPattern::Bitcoin);

    let pat_social =
        InvestigationPattern::detect("Who is behind Twitter handle @osint?", &["username"]);
    assert_eq!(pat_social, InvestigationPattern::KnownSocialAccount);

    let tools = pat.preferred_tools();
    assert!(!tools.is_empty());
}

#[test]
fn test_logical_role_metadata() {
    for role in [
        LogicalRole::Planner,
        LogicalRole::ToolPicker,
        LogicalRole::Synthesis,
        LogicalRole::Classifier,
        LogicalRole::Summarization,
        LogicalRole::EvidenceCurator,
        LogicalRole::EntityResolver,
        LogicalRole::ClaimAssessor,
        LogicalRole::Controller,
    ] {
        let purpose = role.purpose();
        assert!(
            !purpose.is_empty(),
            "Role {:?} must have a non-empty purpose",
            role
        );
        assert!(!role.title().is_empty());
        assert!(!role.key().is_empty());
    }
}
