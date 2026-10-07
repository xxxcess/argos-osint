//! Investigation-pattern routes and preferred retrieval fallbacks.

use serde::{Deserialize, Serialize};

/// High-level prompt investigation pattern.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum InvestigationPattern {
    GeneralSubject,
    ArticleVerification,
    BreakingNews,
    HistoricalTimeline,
    OrganizationOwnership,
    ProfessionalContact,
    EmailAttribution,
    KnownSocialAccount,
    UnknownAccount,
    Litigation,
    HistoricalWebsite,
    DomainIp,
    SoftwareCve,
    Place,
    Bitcoin,
    FollowUp,
}

impl InvestigationPattern {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::GeneralSubject => "general_subject",
            Self::ArticleVerification => "article_verification",
            Self::BreakingNews => "breaking_news",
            Self::HistoricalTimeline => "historical_timeline",
            Self::OrganizationOwnership => "organization_ownership",
            Self::ProfessionalContact => "professional_contact",
            Self::EmailAttribution => "email_attribution",
            Self::KnownSocialAccount => "known_social_account",
            Self::UnknownAccount => "unknown_account",
            Self::Litigation => "litigation",
            Self::HistoricalWebsite => "historical_website",
            Self::DomainIp => "domain_ip",
            Self::SoftwareCve => "software_cve",
            Self::Place => "place",
            Self::Bitcoin => "bitcoin",
            Self::FollowUp => "follow_up",
        }
    }

    /// Primary tools preferred for this pattern in order.
    pub fn preferred_tools(&self) -> &'static [&'static str] {
        match self {
            Self::GeneralSubject => &["firecrawl_search", "firecrawl_scrape"],
            Self::ArticleVerification => &[
                "firecrawl_scrape",
                "firecrawl_search",
                "wikipedia_source_reliability",
            ],
            Self::BreakingNews => &["firecrawl_search", "newsapi_search", "newsapi_headlines"],
            Self::HistoricalTimeline => {
                &["newsapi_search", "firecrawl_search", "wayback_availability"]
            }
            Self::OrganizationOwnership => &[
                "hunter_domain_finder",
                "gleif_entities",
                "sec_submissions",
                "hunter_company_enrichment",
            ],
            Self::ProfessionalContact => &[
                "hunter_email_count",
                "hunter_domain_search",
                "hunter_email_finder",
            ],
            Self::EmailAttribution => &[
                "hunter_email_insight",
                "hunter_combined_enrichment",
                "hunter_person_enrichment",
            ],
            Self::KnownSocialAccount => &["sociavault_profile", "sociavault_user_content"],
            Self::UnknownAccount => &[
                "firecrawl_search",
                "sociavault_search_users",
                "sociavault_profile",
            ],
            Self::Litigation => &[
                "courtlistener_case_search",
                "courtlistener_docket_search",
                "courtlistener_judge_search",
            ],
            Self::HistoricalWebsite => &[
                "wayback_availability",
                "arquivo_history",
                "commoncrawl_urls",
            ],
            Self::DomainIp => &[
                "crtsh_certificates",
                "mnemonic_passive_dns",
                "ripestat_network_info",
                "arin_rdap",
                "shodan_internetdb",
            ],
            Self::SoftwareCve => &[
                "cve_record",
                "nvd_cve",
                "osv_package",
                "github_repositories",
                "grepapp_code_search",
            ],
            Self::Place => &["nominatim_geocode", "census_geocode", "overpass_places"],
            Self::Bitcoin => &[
                "blockstream_address",
                "mempool_address",
                "blockchain_address",
            ],
            Self::FollowUp => &["firecrawl_search", "firecrawl_scrape"],
        }
    }

    /// Detect the most likely pattern from the user's question and initial bindings.
    pub fn detect(question: &str, bindings: &[&str]) -> Self {
        let q = question.to_ascii_lowercase();

        if q.contains("cve-") || q.contains("vulnerability") || q.contains("exploit") {
            return Self::SoftwareCve;
        }

        if bindings.contains(&"bitcoin_address")
            || q.contains("bitcoin")
            || q.contains("btc")
            || q.contains("wallet")
        {
            return Self::Bitcoin;
        }

        if q.contains("court")
            || q.contains("lawsuit")
            || q.contains("judge")
            || q.contains("docket")
            || q.contains("litigation")
        {
            return Self::Litigation;
        }

        if q.contains("archive") || q.contains("wayback") || q.contains("historical site") {
            return Self::HistoricalWebsite;
        }

        if q.contains("twitter")
            || q.contains("instagram")
            || q.contains("github")
            || q.contains("reddit")
            || q.contains("social")
            || (q.contains("handle") && (q.contains("@") || bindings.contains(&"username")))
        {
            if bindings.contains(&"username") || q.contains("handle") || q.contains("profile") {
                return Self::KnownSocialAccount;
            }
            return Self::UnknownAccount;
        }

        if q.contains("investigate email")
            || q.contains("email attribution")
            || (q.contains("email")
                && (q.contains("who owns")
                    || q.contains("who is behind")
                    || q.contains("investigate")))
            || (bindings.contains(&"email") && !q.contains("find email") && !q.contains("contact"))
        {
            return Self::EmailAttribution;
        }

        if q.contains("@") || q.contains("email") || q.contains("contact") {
            return Self::ProfessionalContact;
        }

        if q.contains("ip address")
            || q.contains("subnet")
            || q.contains("dns")
            || q.contains("nameserver")
        {
            return Self::DomainIp;
        }

        if q.contains("where is")
            || q.contains("coordinates")
            || q.contains("located")
            || q.contains("address")
        {
            return Self::Place;
        }

        if q.contains("http://") || q.contains("https://") {
            return Self::ArticleVerification;
        }

        if q.contains("breaking") || q.contains("latest news") || q.contains("just now") {
            return Self::BreakingNews;
        }

        if q.contains("timeline") || q.contains("history of") {
            return Self::HistoricalTimeline;
        }

        if q.contains("company")
            || q.contains("inc.")
            || q.contains("corp")
            || q.contains("sec")
            || q.contains("lei")
        {
            return Self::OrganizationOwnership;
        }

        Self::GeneralSubject
    }
}

/// Fallback retrieval chain for fetching full article bodies.
/// Firecrawl scrape → direct HTTP → Wayback discovery and retrieval → Arquivo retrieval.
pub fn article_body_chain() -> &'static [&'static str] {
    &[
        "firecrawl_scrape",
        "wayback_availability",
        "arquivo_history",
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_patterns_reliably() {
        assert_eq!(
            InvestigationPattern::detect("Check CVE-2024-1234", &[]),
            InvestigationPattern::SoftwareCve
        );
        assert_eq!(
            InvestigationPattern::detect(
                "Lookup 1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa",
                &["bitcoin_address"]
            ),
            InvestigationPattern::Bitcoin
        );
        assert_eq!(
            InvestigationPattern::detect("Search CourtListener docket for Acme", &[]),
            InvestigationPattern::Litigation
        );
        assert_eq!(
            InvestigationPattern::detect("Verify article https://example.com/news", &[]),
            InvestigationPattern::ArticleVerification
        );
    }
}
