//! Surface permissions, scarce-provider restrictions, need-only gates, and freshness admission.

use serde::{Deserialize, Serialize};

use super::contracts::InvestigationSurface;
use crate::osint::canonical_tool_id;

/// Surface eligibility check for a catalog tool.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SurfacePolicy {
    pub surface: InvestigationSurface,
    pub allow_atlas_news: bool,
    pub allow_sociavault: bool,
    pub allow_newsapi: bool,
    pub max_sociavault_calls: u32,
}

impl SurfacePolicy {
    pub fn for_surface(surface: InvestigationSurface) -> Self {
        match surface {
            InvestigationSurface::IntelBrief => Self {
                surface,
                allow_atlas_news: false,
                allow_sociavault: false, // SociaVault strictly restricted
                allow_newsapi: true,
                max_sociavault_calls: 0,
            },
            InvestigationSurface::ReconChat
            | InvestigationSurface::HomeComposer
            | InvestigationSurface::JobsResume => Self {
                surface,
                allow_atlas_news: false,
                allow_sociavault: true,
                allow_newsapi: true,
                max_sociavault_calls: 4,
            },
        }
    }

    /// Whether a tool is permitted to be invoked on this surface.
    pub fn is_tool_permitted(&self, tool_id: &str) -> bool {
        let tool = canonical_tool_id(tool_id);

        // Atlas-only tools are NEVER permitted on Recon, Home, or Intel
        if tool.starts_with("atlas_")
            || matches!(tool, "gnews_search" | "newsdata_latest" | "currents_latest")
        {
            return self.allow_atlas_news;
        }

        // SociaVault tools
        if tool.starts_with("sociavault_") {
            return self.allow_sociavault;
        }

        // NewsAPI
        if tool.starts_with("newsapi_") {
            return self.allow_newsapi;
        }

        true
    }
}

/// Check if a tool call meets scarce-provider need-only conditions.
pub fn check_need_gate(
    surface: InvestigationSurface,
    tool_id: &str,
    has_unmet_need: bool,
    firecrawl_was_weak: bool,
) -> Result<(), &'static str> {
    let tool = canonical_tool_id(tool_id);
    let policy = SurfacePolicy::for_surface(surface);

    if !policy.is_tool_permitted(tool) {
        return Err("Tool not permitted on this investigation surface");
    }

    // SociaVault Google is ONLY allowed as weak-Firecrawl fallback when unmet need exists
    if tool == "sociavault_google_search" {
        if !firecrawl_was_weak {
            return Err("SociaVault Google search requires weak Firecrawl discovery first");
        }
        if !has_unmet_need {
            return Err("SociaVault Google search requires an unmet evidence need");
        }
    }

    // SociaVault platform tools require unmet platform-native need
    if tool.starts_with("sociavault_") && tool != "sociavault_google_search" && !has_unmet_need {
        return Err("SociaVault call requires an unmet platform-native evidence need");
    }

    Ok(())
}
/// Check whether a cached observation satisfies a task's freshness cutoff.
pub fn is_observation_fresh(observed_at: &str, freshness_cutoff: &str) -> bool {
    if freshness_cutoff.is_empty() {
        return true;
    }
    if observed_at.is_empty() {
        return false;
    }
    observed_at >= freshness_cutoff
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atlas_only_tools_blocked_on_all_surfaces() {
        for surface in [
            InvestigationSurface::ReconChat,
            InvestigationSurface::HomeComposer,
            InvestigationSurface::IntelBrief,
            InvestigationSurface::JobsResume,
        ] {
            let policy = SurfacePolicy::for_surface(surface);
            assert!(!policy.is_tool_permitted("gnews_search"));
            assert!(!policy.is_tool_permitted("newsdata_latest"));
            assert!(!policy.is_tool_permitted("currents_latest"));
        }
    }

    #[test]
    fn sociavault_blocked_on_intel_surface() {
        let policy = SurfacePolicy::for_surface(InvestigationSurface::IntelBrief);
        assert!(!policy.is_tool_permitted("sociavault_search"));
        assert!(!policy.is_tool_permitted("sociavault_google_search"));
    }

    #[test]
    fn sociavault_google_requires_weak_firecrawl_and_unmet_need() {
        let surface = InvestigationSurface::ReconChat;
        // Strong firecrawl -> blocked
        assert!(check_need_gate(surface, "sociavault_google_search", true, false).is_err());
        // Weak firecrawl but no unmet need -> blocked
        assert!(check_need_gate(surface, "sociavault_google_search", false, true).is_err());
        // Weak firecrawl and unmet need -> allowed
        assert!(check_need_gate(surface, "sociavault_google_search", true, true).is_ok());
    }

    #[test]
    fn freshness_check_rejects_older_observations() {
        assert!(is_observation_fresh(
            "2026-10-07T10:00:00Z",
            "2026-10-07T08:00:00Z"
        ));
        assert!(!is_observation_fresh(
            "2026-10-06T10:00:00Z",
            "2026-10-07T08:00:00Z"
        ));
        assert!(is_observation_fresh("2026-10-07T10:00:00Z", ""));
    }
}
