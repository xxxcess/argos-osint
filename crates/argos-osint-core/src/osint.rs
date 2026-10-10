//! Bounded, public HTTP observations shared by Recon and manual OSINT.
use anyhow::{anyhow, ensure, Result};
use chrono::Utc;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    net::IpAddr,
    sync::{Arc, OnceLock},
    time::Duration,
};
use tokio::sync::{Mutex, Semaphore};
use url::Url;

pub(crate) mod atlas_news;
mod news_legal;
#[cfg(test)]
pub(crate) use news_legal::fixture;
pub mod contracts;
pub mod dataset;
pub mod dork_generator;
pub mod holehe;
mod providers;
pub mod results;
pub mod search_engines;
pub mod source_eval;
pub mod whatsmyname;
pub mod whoxy;
pub mod wikipedia_rsp;
pub use contracts::{
    compact_capability_catalog, picker_candidates, ArgumentBuilderContract, CompactToolCapability,
    IntelligenceCategory, PickerCandidate,
};
pub use news_legal::{
    context_kind, COURTLISTENER_RATE_LIMIT, COURTLISTENER_SPACING, LEGAL_TOOLS, NEWS_TOOLS,
};
pub use providers::{
    batch_urls, map_rank, select_route, sociavault_account_platforms, sociavault_endpoint_hint,
    sociavault_platforms, sociavault_routes, webmail_host, RouteInput, SociaVaultRoute,
    BATCH_SCRAPE_DEFAULT_URLS, BATCH_SCRAPE_MAX_URLS, SOCIAVAULT_ROUTES, SOCIAVAULT_TOOLS,
};
pub use results::{
    classify_output_quality, extract_observation_items, extract_search_results,
    normalize_tool_result, NormalizedToolResult, OutputQuality, SearchResultItem,
};
pub use source_eval::{
    best_credibility, information_credibility, scale_confidence, AdmiraltyCode, CredibilityInputs,
    InformationCredibility, SourceReliability,
};
pub use wikipedia_rsp::{RspEntry, RspIndex, RspStatus, SourceReliabilityObservation};

#[derive(Clone, Debug, Serialize)]
pub struct ToolDefinition {
    pub id: &'static str,
    pub name: &'static str,
    pub category: &'static str,
    pub description: &'static str,
    pub inputs: &'static [&'static str],
    pub documentation: &'static str,
    pub restrictions: &'static str,
    pub timeout_seconds: u64,
    pub cache_seconds: u64,
}

/// How often a provider's pricing plan restores credits. A tool caches a completed
/// result for that long, so the same lookup is not paid for again until the plan would
/// have restored the credits. A provider that does not restore credits caches for a month.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanInterval {
    Daily,
    Weekly,
    Monthly,
    /// Prepaid packs, or no credit plan at all.
    Never,
}

pub const CACHE_DAY_SECONDS: u64 = 86_400;
pub const CACHE_WEEK_SECONDS: u64 = 7 * CACHE_DAY_SECONDS;
/// Thirty days. Expiry is a duration from the call, not a calendar month.
pub const CACHE_MONTH_SECONDS: u64 = 30 * CACHE_DAY_SECONDS;

impl PlanInterval {
    pub const fn cache_seconds(self) -> u64 {
        match self {
            Self::Daily => CACHE_DAY_SECONDS,
            Self::Weekly => CACHE_WEEK_SECONDS,
            Self::Monthly | Self::Never => CACHE_MONTH_SECONDS,
        }
    }
}

/// The pricing-plan interval that sets this tool's cache lifetime.
///
/// Firecrawl and Hunter restore a monthly credit allotment; a yearly bill still resets
/// those credits each month. SociaVault sells prepaid packs that never expire. NewsAPI's
/// developer plan, CourtListener's free tier, and HackerTarget's free allowance each
/// restore daily. Every other catalog tool has no credit plan.
pub fn plan_interval(id: &str) -> PlanInterval {
    let id = canonical_tool_id(id);
    if let Some(provider) = primary_provider(id) {
        return match provider {
            "firecrawl" | "hunter" => PlanInterval::Monthly,
            // SociaVault prepaid packs do not reset. Any other primary provider
            // without a restoring plan is treated the same way.
            _ => PlanInterval::Never,
        };
    }
    if news_legal::provider(id).is_some() || atlas_news::provider(id).is_some() {
        return PlanInterval::Daily;
    }
    match id {
        "hackertarget_hostsearch"
        | "whatsmyname_lookup"
        | "dork_generate"
        | "holehe_email_lookup" => PlanInterval::Daily,
        // WP:RSP changes slowly; keep the built index for a month.
        "wikipedia_source_reliability" => PlanInterval::Monthly,
        _ => PlanInterval::Never,
    }
}

/// Cache lifetime for one catalog tool, from [`plan_interval`].
pub fn cache_seconds(id: &str) -> u64 {
    plan_interval(id).cache_seconds()
}

macro_rules! tool { ($id:expr,$name:expr,$category:expr,$description:expr,[$($input:expr),*],$doc:expr,$restriction:expr,$timeout:expr) => { ToolDefinition {id:$id,name:$name,category:$category,description:$description,inputs:&[$($input),*],documentation:$doc,restrictions:$restriction,timeout_seconds:$timeout,cache_seconds:cache_seconds($id)} }; }
pub fn registry() -> &'static [ToolDefinition] {
    static TOOLS: OnceLock<Vec<ToolDefinition>> = OnceLock::new();
    TOOLS.get_or_init(|| vec![
        tool!("crtsh_certificates","crt.sh certificates","Domains","Discover certificate records and hostnames.",["domain"],"https://crt.sh/","Public service; availability varies.",30),
        tool!("mnemonic_passive_dns","mnemonic Passive DNS","Domains","Historical hostname and IP relationships.",["domain_or_ip"],"https://docs.mnemonic.no/service-integration-guides/passivedns/docs/public/01-public_api.html","Public endpoint; bounded pagination and quotas apply.",30),
        tool!("whoxy_whois_history","Whoxy WHOIS history","Domains","Historical WHOIS snapshots, registrar/contact changes, and nameserver diffs for a domain.",["domain"],"https://www.whoxy.com/whois-history/","GET https://api.whoxy.com/?history=DOMAIN. Enter the Whoxy key on this tool, or set WHOXY_API_KEY; the key is never stored on the input or evidence URL. Prepaid lookups: 1 credit for nonempty history, 0 for a genuine empty result. Observation dates are not registration dates.",30),
        tool!("hackertarget_hostsearch","HackerTarget Host Search","Domains","Indexed subdomains and IPs.",["domain"],"https://hackertarget.com/ip-tools/","Free daily allowance and request rate apply.",20),
        tool!("ripestat_network_info","RIPEstat network info","Networks","Announced prefix and routing ASN for an IP.",["ip"],"https://stat.ripe.net/docs/data_api","Routing origin is not website ownership.",20),
        tool!("arin_rdap","ARIN RDAP","Networks","IP registration and contacts.",["ip"],"https://www.arin.net/resources/registry/whois/rdap/","Regional registry may redirect.",25),
        tool!("apnic_rdap","APNIC RDAP","Networks","Asia-Pacific IP registration records.",["ip"],"https://www.apnic.net/about-apnic/whois_search/about/rdap/","Regional registry may redirect.",25),
        tool!("wayback_availability","Wayback availability","Archives","Locate an available historical snapshot.",["url"],"https://archive.org/help/wayback_api.php","Snapshot availability does not reveal page contents.",30),
        tool!("commoncrawl_urls","Common Crawl URLs","Archives","Crawled URLs and archive references.",["domain"],"https://index.commoncrawl.org/","Indexed crawl coverage is incomplete.",40),
        tool!("arquivo_history","Arquivo.pt history","Archives","Archived versions of a site.",["domain_or_url"],"https://arquivo.pt/api","Archive coverage is incomplete.",40),
        tool!("github_repositories","GitHub repositories","Code","Public repository names and metadata.",["query"],"https://docs.github.com/en/rest/search/search","Anonymous and search-specific quotas; not code search.",20),
        tool!("gitlab_projects","GitLab projects","Code","Public project metadata.",["query"],"https://docs.gitlab.com/api/projects/","Public projects only; rate limits apply.",20),
        tool!("grepapp_code_search","grep.app code search","Code","Source references to a string.",["query"],"https://grep.app/api","Snippets are untrusted evidence.",20),
        tool!("gleif_entities","GLEIF LEI entities","Organizations","Legal entities, LEIs and relationships.",["company_name|lei"],"https://www.gleif.org/en/lei-data/gleif-api","Search matches require identity confirmation.",25),
        tool!("sec_submissions","SEC EDGAR submissions","Organizations","US reporting entity and filing metadata.",["cik|name|ticker"],"https://www.sec.gov/search-filings/edgar-application-programming-interfaces","Identifying User-Agent required; US reporting entities.",30),
        tool!("wikidata_entities","Wikidata entities","Organizations","Find public organization entities and claims.",["name|qid"],"https://www.wikidata.org/wiki/Wikidata:REST_API","Search matches are candidates, not verified identity.",25),
        tool!("keybase_identity","Keybase identity","Identities","Public profile and external identity proofs.",["username|domain"],"https://keybase.io/docs/api/1.0/call/user/lookup","Account does not establish physical identity.",20),
        tool!("stackexchange_users","Stack Exchange users","Identities","Public profiles matching a display name.",["name"],"https://api.stackexchange.com/docs/users","Display names may be ambiguous.",20),
        tool!("wikipedia_users","Wikipedia users","Identities","Account registration and edit metadata.",["username"],"https://www.mediawiki.org/wiki/API:Users","Account does not establish physical identity.",20),
        tool!("nominatim_geocode","Nominatim geocode","Places","Place or address coordinates.",["address_or_place"],"https://operations.osmfoundation.org/policies/nominatim/","OpenStreetMap attribution; one request/second; no autocomplete or bulk queries.",20),
        tool!("census_geocode","US Census geocode","Places","Coordinates for a US street address.",["us_address"],"https://www.census.gov/data/developers/data-sets/Geocoding-services.html","US addresses only.",20),
        tool!("overpass_places","Overpass places","Places","Mapped features around coordinates.",["latitude","longitude","radius_m"],"https://wiki.openstreetmap.org/wiki/Overpass_API","Bounded radius and feature allowlist; OpenStreetMap attribution.",35),
        tool!("blockchain_address","Blockchain.com Bitcoin address","Bitcoin","Bitcoin address balance and activity totals.",["bitcoin_address"],"https://www.blockchain.com/explorer/api/blockchain_api","Bitcoin only; address does not establish owner.",20),
        tool!("blockstream_address","Blockstream Bitcoin address","Bitcoin","Bitcoin address activity.",["bitcoin_address"],"https://github.com/Blockstream/esplora/blob/master/API.md","Bitcoin only; address does not establish owner.",20),
        tool!("mempool_address","mempool.space Bitcoin address","Bitcoin","Confirmed and unconfirmed address activity.",["bitcoin_address"],"https://mempool.space/docs/api/rest","Bitcoin only; address does not establish owner.",20),
        tool!("nvd_cve","NVD CVE","Vulnerabilities","CVE description, severity and affected products.",["cve_id"],"https://nvd.nist.gov/developers/start-here","Advisory does not prove live exploitability.",25),
        tool!("osv_package","OSV package query","Vulnerabilities","Vulnerabilities affecting a package version or commit.",["ecosystem","package_name","version|commit"],"https://google.github.io/osv.dev/api/","Package coordinates and versions must match the target.",25),
        tool!("cve_record","CVE published record","Vulnerabilities","Published CVE record and references.",["cve_id"],"https://cveawg.mitre.org/api-docs/","Advisory does not prove live exploitability.",25),
        tool!("sans_ip_activity","SANS ISC IP activity","Exposure","Reported attack activity for an IP.",["ip"],"https://isc.sans.edu/api/","Reports are historical observations.",25),
        tool!("shodan_internetdb","Shodan InternetDB","Exposure","Observed ports, hostnames and vulnerability associations.",["ip"],"https://internetdb.shodan.io/","Free access is noncommercial; observations may be old.",20),
        tool!("urlscan_search","urlscan search","Exposure","Search existing website scan records.",["domain|query"],"https://urlscan.io/docs/api/","Search only; no scan submission; historical observations.",20),
        tool!("firecrawl_search","Firecrawl search","Web","Web search for titles, links, and descriptions. A new investigation runs two complementary searches before enrichment. Later searches are targeted follow-ups.",["query"],"https://docs.firecrawl.dev/api-reference/endpoint/search","POST https://api.firecrawl.dev/v2/search with query and limit (at most 10). Optional sources (web, news), categories (github, research), tbs time filter, and location. Enter the API key on this tool, or set FIRECRAWL_API_KEY. Results are snippets, not page content. Automatic investigation does not paginate. 2 credits per 10 results.",60),
        tool!("firecrawl_google_search","Google Search (via Firecrawl)","Web","Search Google by scraping its SERP HTML via Firecrawl /v2/scrape.",["query"],"https://docs.firecrawl.dev/api-reference/endpoint/scrape","POST https://api.firecrawl.dev/v2/scrape with Google search URL template. Parses organic result headings, snippets, and clean destination URLs. 1 credit.",60),
        tool!("firecrawl_yandex_search","Yandex Search (via Firecrawl)","Web","Search Yandex by scraping its SERP HTML via Firecrawl /v2/scrape.",["query"],"https://docs.firecrawl.dev/api-reference/endpoint/scrape","POST https://api.firecrawl.dev/v2/scrape with Yandex search URL template. Parses organic result headings, snippets, and clean destination URLs. 1 credit.",60),
        tool!("firecrawl_mojeek_search","Mojeek Search (via Firecrawl)","Web","Search Mojeek by scraping its SERP HTML via Firecrawl /v2/scrape.",["query"],"https://docs.firecrawl.dev/api-reference/endpoint/scrape","POST https://api.firecrawl.dev/v2/scrape with Mojeek search URL template. Parses organic result headings, snippets, and clean destination URLs. 1 credit.",60),
        tool!("firecrawl_scrape","Firecrawl page","Web","Retrieve one public page as markdown when a search snippet is not enough to support a consequential claim.",["url"],"https://docs.firecrawl.dev/api-reference/endpoint/scrape","POST https://api.firecrawl.dev/v2/scrape for a single URL already found in evidence. One page per call; optional formats markdown and links (structured JSON goes through firecrawl_extract). Same Firecrawl API key as search. 1 credit.",60),
        tool!("firecrawl_map","Firecrawl map","Web","List a subject-owned site's pages, contact, about, team, press, and legal pages first.",["domain|url"],"https://docs.firecrawl.dev/api-reference/endpoint/map","POST https://api.firecrawl.dev/v2/map with optional search. Same registrable domain only, at most 100 links, never a social, publisher, or Q&A host. 1 credit per call.",60),
        tool!("firecrawl_batch_scrape","Firecrawl batch scrape","Web","Retrieve up to 10 evidence URLs as markdown in one job, such as the contact and about pages a map found.",["urls"],"https://docs.firecrawl.dev/api-reference/endpoint/batch-scrape","POST https://api.firecrawl.dev/v2/batch/scrape, then GET /v2/batch/scrape/{id} until done. URLs must already be in evidence; default 5, at most 10; markdown only; 1 credit per page. Polling is free; a job still running at the timeout is recorded as partial.",120),
        tool!("firecrawl_crawl","Firecrawl crawl","Web","Small crawl of a subject-owned site: up to 10 pages one link deep. Off by default.",["domain|url"],"https://docs.firecrawl.dev/api-reference/endpoint/crawl-post","POST https://api.firecrawl.dev/v2/crawl, then GET /v2/crawl/{id} until done. limit at most 10, maxDiscoveryDepth 1, same domain, markdown only, 1 credit per page. Disabled until enabled on the OSINT screen.",180),
        tool!("firecrawl_extract","Firecrawl extract","Web","Structured org name, legal name, domain, emails, social profiles, people, and address from one page.",["url"],"https://docs.firecrawl.dev/features/llm-extract","POST https://api.firecrawl.dev/v2/scrape with a JSON format and a fixed Argos schema {org_name, legal_name, domain, emails, social_profiles, people, address}. One page; 5 credits.",90),
        tool!("hunter_domain_finder","Hunter domain finder","Enrichment","Resolve an organization name to its website domain. Free.",["company"],"https://hunter.io/api-documentation/v2#domain-finder","GET https://api.hunter.io/v2/domain-finder. Company name of at least 3 characters; optional limit (1-10) and perfect_match. Free but rate-limited. Matches that are not perfect become inferred bindings. Inputs only from the prompt, Firecrawl, SociaVault, or Hunter.",25),
        tool!("hunter_email_count","Hunter email count","Enrichment","How many addresses Hunter has for a domain or company. Free; zero skips the paid domain search.",["domain|company"],"https://hunter.io/api-documentation/v2#email-count","GET https://api.hunter.io/v2/email-count with optional type (personal, generic). Free. A zero count skips hunter_domain_search; Hunter notes zero can also mean the domain is privacy-suppressed.",20),
        tool!("hunter_domain_search","Hunter domain search","Enrichment","Email addresses, roles, and the email pattern Hunter has for a company domain or name.",["domain|company"],"https://hunter.io/api-documentation/v2#domain-search","GET https://api.hunter.io/v2/domain-search. Enter the API key on a Hunter tool, or set HUNTER_API_KEY. The key is sent as X-API-KEY and is not stored on the tool input. At most 10 addresses per call. Inputs only from the prompt, Firecrawl, SociaVault, or Hunter.",25),
        tool!("hunter_company_enrichment","Hunter company enrichment","Enrichment","Company profile for a domain: name, legal name, industry, size, address, tech stack, social handles, site emails.",["domain"],"https://hunter.io/api-documentation/v2#company-enrichment","GET https://api.hunter.io/v2/companies/find. 1 credit. Replaces hunter_tech_lookup (the old id still resolves here). Same Hunter API key as the other Hunter tools.",25),
        tool!("hunter_email_finder","Hunter email finder","Enrichment","Most likely professional email for a named person at a domain, company, or LinkedIn handle.",["domain|company|linkedin_handle","full_name|first_name|linkedin_handle"],"https://hunter.io/api-documentation/v2#email-finder","GET https://api.hunter.io/v2/email-finder. Requires a domain, company, or LinkedIn handle, plus a full name or a first and last name unless the LinkedIn handle is enough. Same Hunter API key as the other Hunter tools.",25),
        tool!("hunter_email_verifier","Hunter email verifier","Enrichment","Deliverability status and score for one email address, when a claim depends on deliverability.",["email"],"https://hunter.io/api-documentation/v2#email-verifier","GET https://api.hunter.io/v2/email-verifier. The check can take about 20 seconds; a 202 is retried. Same Hunter API key as the other Hunter tools.",40),
        tool!("hunter_email_insight","Hunter email insight","Enrichment","Whether an email is webmail, disposable, or gibberish, plus its MX records. Free.",["email"],"https://hunter.io/api-documentation/v2#email-insight","GET https://api.hunter.io/v2/email-insight. Free. Runs before enrichment: webmail and disposable addresses go to person enrichment only, company addresses to combined enrichment.",20),
        tool!("hunter_person_enrichment","Hunter person enrichment","Enrichment","Person profile for an email or LinkedIn handle: name, employer, location, social handles.",["email|linkedin_handle"],"https://hunter.io/api-documentation/v2#email-enrichment","GET https://api.hunter.io/v2/people/find. 1 credit. 404 means no match. A 451 claimed_email response is stored without the person payload and yields no bindings.",25),
        tool!("hunter_combined_enrichment","Hunter combined enrichment","Enrichment","Person and company profile for one company email address in a single call.",["email"],"https://hunter.io/api-documentation/v2#combined-enrichment","GET https://api.hunter.io/v2/combined/find. Company email addresses only; webmail goes to person enrichment. 1 credit. A 451 claimed_email response is stored without the person payload and yields no bindings.",25),
        tool!("sociavault_profile","SociaVault profile","Social","Public profile stats, biography, outbound links, and account id for one evidence-supported account.",["platform","handle|user_id"],"https://docs.sociavault.com/api-reference/introduction","GET https://api.sociavault.com/v1/scrape/{platform}/profile (YouTube /youtube/channel; LinkedIn /linkedin/profile or /linkedin/company; Instagram /instagram/basic-profile by user_id). Platforms: twitter, instagram, tiktok, youtube, facebook, linkedin, threads, twitch. Optional endpoint. Enter the API key on a SociaVault tool, or set SOCIAVAULT_API_KEY. The key is sent as X-API-Key and is not stored on the tool input. 1 credit.",40),
        tool!("sociavault_search","SociaVault search","Social","Search one platform's posts, videos, or hashtags for the subject's name, organization, or a hashtag.",["platform","query"],"https://docs.sociavault.com/api-reference/introduction","GET https://api.sociavault.com/v1/scrape/... Platforms and endpoints: instagram (hashtag), linkedin (posts), pinterest (search), reddit (search, subreddit), threads (search), tiktok (keyword, hashtag, top), twitter (search), youtube (search, hashtag). Optional endpoint and subreddit. 1 credit. Handles found only here stay unverified.",40),
        tool!("sociavault_search_users","SociaVault account search","Social","Find accounts by name on Instagram, Threads, or TikTok.",["platform","query"],"https://docs.sociavault.com/api-reference/introduction","GET https://api.sociavault.com/v1/scrape/instagram/search, /threads/search-users, or /tiktok/search/users. 1 credit. Accounts found only here stay unverified until a profile call or a Firecrawl page links them to the subject.",40),
        tool!("sociavault_user_content","SociaVault user content","Social","One account's own posts, videos, reels, highlights, playlists, boards, or schedule. No followers or single posts.",["platform","handle|user_id"],"https://docs.sociavault.com/api-reference/introduction","GET https://api.sociavault.com/v1/scrape/... Platforms and endpoints: facebook (posts, reels), instagram (posts, highlights, reels), pinterest (boards), threads (posts), tiktok (videos, live), twitch (videos, schedule), twitter (tweets; tweets_all by user_id), youtube (videos, community_posts, lives, playlists, shorts). Optional endpoint. 1 credit. Runs after a profile call when a numeric id is needed.",40),
        tool!("sociavault_google_search","SociaVault Google search","Web","Google results for the same query when Firecrawl search was weak. Fallback only.",["query"],"https://docs.sociavault.com/api-reference/introduction","GET https://api.sociavault.com/v1/scrape/google/search. Never an opening pick: offered only when Firecrawl search failed, returned fewer than 3 results, returned only filtered hosts, or yielded no binding a later step needs. One page and one call per question; 1 credit.",40),
        tool!("newsapi_search","NewsAPI article search","News","News articles that name the subject (exact phrase). Free-tier articles arrive 24 hours late and search reaches back one month, so this is never breaking news.",["query"],"https://newsapi.org/docs/endpoints/everything","GET https://newsapi.org/v2/everything with q as an exact phrase, pageSize 10 (at most 10 articles), first page only. Optional from and to (YYYY-MM-DD), language, sort_by (relevancy default, publishedAt, popularity), and domains. Enter the NewsAPI key on a News tool, or set NEWSAPI_API_KEY; it is sent as X-Api-Key, never in the URL. Developer plan: 100 requests a day, development use only. At most 2 NewsAPI calls per turn.",20),
        tool!("newsapi_headlines","NewsAPI top headlines","News","Top headlines that name the subject (exact phrase). Free-tier headlines arrive 24 hours late and cover one month at most, so this is never breaking news.",["query"],"https://newsapi.org/docs/endpoints/top-headlines","GET https://newsapi.org/v2/top-headlines with q as an exact phrase, pageSize 10 (at most 10 articles). Optional country (2-letter code) and category (business, entertainment, general, health, science, sports, technology). Same NewsAPI key (NEWSAPI_API_KEY), sent as X-Api-Key, never in the URL. At most 2 NewsAPI calls per turn.",20),
        tool!("wikipedia_source_reliability","Wikipedia source reliability","News","Admiralty Source Reliability (A–F) for a publisher domain from English Wikipedia WP:RSP.",["domain|url|publisher"],"https://www.mediawiki.org/wiki/API:Parsing_wikitext","Maps WP:RSP consensus to A–F. Unlisted sources are F (cannot be judged). Context-dependent; not exhaustive. Uses MediaWiki action=parse on Perennial sources subpages; identifying User-Agent required. Cache 30 days.",45),
        tool!("gnews_search","GNews search","News","Keyword search across recent articles. Atlas uses this for 48-hour flashpoint discovery because the payload includes the source country.",["query"],"https://docs.gnews.io/endpoints/search-endpoint","GET https://gnews.io/api/v4/search with q (at most 200 characters), lang, max 10, and optional from (ISO 8601). Enter the GNews key on a GNews tool, or set GNEWS_API_KEY. The key is sent as X-Api-Key, never in the URL. Free tier: 100 requests a day, 10 articles, about a 12-hour delay. Atlas calls this; Recon does not.",20),
        tool!("newsdata_latest","NewsData latest","News","Latest articles for a keyword query. The endpoint is the past 48 hours. Atlas reads country codes from each result.",["query"],"https://newsdata.io/documentation","GET https://newsdata.io/api/1/latest with q (at most 100 characters), language, and size 10. The latest endpoint is already the past 48 hours. Optional timeframe (1 to 48 hours) is a paid parameter; the free plan returns HTTP 422 if it is sent. A query longer than 100 characters is also HTTP 422. Enter the NewsData key on a NewsData tool, or set NEWSDATA_API_KEY. The key is the apikey query parameter and is redacted from the stored URL. Free tier: 200 credits a day, 10 articles. Atlas calls this; Recon does not.",20),
        tool!("currents_latest","Currents latest news","News","Latest headlines for one country. Atlas uses this in the regional extraction phase.",["country"],"https://currentsapi.services/en/docs/endpoint","GET https://api.currentsapi.services/v1/latest-news with language, country, and page_size 20. Enter the Currents key on a Currents tool, or set CURRENTS_API_KEY. The key is sent as Authorization: Bearer. Free tier: 250 requests a day, 20 articles. Atlas calls this; Recon does not.",20),
        tool!("courtlistener_case_search","CourtListener case law","Legal","Court opinions (case law) that name the subject (exact phrase), with court, filing date, and case name.",["query"],"https://www.courtlistener.com/help/api/rest/search/","GET https://www.courtlistener.com/api/rest/v4/search/?type=o with q as an exact phrase. First page only, at most 20 results, no highlighting, never semantic search. Optional court (court ids separated by spaces), filed_after and filed_before (YYYY-MM-DD). Enter the CourtListener API token on a Legal tool, or set COURTLISTENER_API_TOKEN; it is sent as Authorization: Token. Free tier 5/min, 50/hour, 125/day: at most 3 CourtListener calls per turn, 12 s apart.",30),
        tool!("courtlistener_docket_search","CourtListener federal dockets","Legal","Federal (PACER/RECAP) dockets that name the subject (exact phrase), with court, filing date, and case name.",["query"],"https://www.courtlistener.com/help/api/rest/search/","GET https://www.courtlistener.com/api/rest/v4/search/?type=r with q as an exact phrase. First page only, at most 20 results, no highlighting; no RECAP fetch or paid PACER pulls. Optional court and filed_after (YYYY-MM-DD). Same CourtListener token (COURTLISTENER_API_TOKEN), sent as Authorization: Token. At most 3 CourtListener calls per turn, 12 s apart.",30),
        tool!("courtlistener_judge_search","CourtListener judges","Legal","Judges whose name matches the subject (exact phrase), with court and position.",["query"],"https://www.courtlistener.com/help/api/rest/search/","GET https://www.courtlistener.com/api/rest/v4/search/?type=p with q as an exact phrase. First page only, at most 20 results. Same CourtListener token (COURTLISTENER_API_TOKEN), sent as Authorization: Token. At most 3 CourtListener calls per turn, 12 s apart.",30),
        tool!("whatsmyname_lookup","WhatsMyName lookup","Identities","Enumerate public accounts across websites using the official WhatsMyName dataset.",["username"],"https://github.com/WebBreacher/WhatsMyName","Dataset-driven detection; network access required; zero credit cost.",90),
        tool!("holehe_email_lookup","Holehe email lookup","Email","Check whether an email address is registered on selected public services.",["email"],"https://github.com/megadose/holehe","Native adapters for Twitter, Spotify, and Pinterest; other catalog services return unsupported. No API key. Registration is email association, not identity or account control. Optional services and max_sites (default 10, cap 50).",40),
        tool!("dork_generate","Dork generator","Web","Compose structured search queries from DorkSearch PRO templates for Firecrawl search.",["objective|query|purpose|target"],"https://dorksearch.pro/","Local query synthesis; zero credit cost.",20),
    ]).as_slice()
}
pub fn definition(id: &str) -> Option<&'static ToolDefinition> {
    let id = canonical_tool_id(id);
    registry().iter().find(|t| t.id == id)
}

/// Extra wall time a Firecrawl job may spend polling after its POST. Batch scrape and
/// crawl poll until the tool timeout; every other tool returns zero.
pub fn job_poll_seconds(id: &str) -> u64 {
    match canonical_tool_id(id) {
        "firecrawl_batch_scrape" | "firecrawl_crawl" => {
            definition(id).map(|tool| tool.timeout_seconds).unwrap_or(0)
        }
        _ => 0,
    }
}

/// Old tool ids that still resolve: `hunter_tech_lookup` became `hunter_company_enrichment`.
pub fn canonical_tool_id(id: &str) -> &str {
    match id {
        "hunter_tech_lookup" => "hunter_company_enrichment",
        other => other,
    }
}

/// Canonical cache key identity for a tool and its arguments.
///
/// The three named engine tools use identity v2 (spec §9 fix 7): the parser
/// version, the fetch contract version and the canonical query/locale/limit are
/// part of the key, so a parser upgrade invalidates legacy v1 entries without
/// deleting them and a reworded query never reuses another query's page.
pub fn cache_identity(tool_id: &str, inputs: &serde_json::Value) -> String {
    let canonical = canonical_tool_id(tool_id);
    if canonical == "whoxy_whois_history" {
        if let Some(domain) = inputs.get("domain").and_then(serde_json::Value::as_str) {
            let normalized = whoxy::normalize_domain(domain)
                .unwrap_or_else(|_| domain.trim().to_ascii_lowercase());
            return format!("{canonical}:v1:{{\"domain\":\"{normalized}\"}}");
        }
    }
    if search_engines::is_named_engine_tool(canonical) {
        if let Ok(serp) = serp_request_for(canonical, inputs) {
            return format!("{canonical}:v2:{}", search_engines::cache_fragment(&serp));
        }
    }
    let serialized = serde_json::to_string(inputs).unwrap_or_else(|_| inputs.to_string());
    format!("{canonical}:v1:{serialized}")
}

/// Project a cached base result to the requested view.
pub fn project_cached_result(
    tool_id: &str,
    inputs: &serde_json::Value,
    cached: &ToolResult,
) -> ToolResult {
    let mut projected = cached.clone();
    projected.tool_id = canonical_tool_id(tool_id).to_string();
    projected.inputs = inputs.clone();
    projected.cached = true;
    projected.credits_charged = 0;
    projected.credits_reported = None;
    if canonical_tool_id(tool_id) == whoxy::TOOL_ID {
        if let (Ok(raw), Ok(query)) = (
            serde_json::from_str::<Value>(&cached.raw),
            whoxy::parse_query(inputs),
        ) {
            if let Ok(full) = whoxy::parse_history_envelope(&raw, &query.domain) {
                projected.observations =
                    whoxy::bounded_model_view(&whoxy::project_history(&full, &query));
            }
        }
    }
    projected
}

/// Returns true if the tool is an external search discovery tool.
pub fn is_search_discovery_tool(tool_id: &str) -> bool {
    matches!(
        canonical_tool_id(tool_id),
        "firecrawl_search"
            | "firecrawl_google_search"
            | "firecrawl_yandex_search"
            | "firecrawl_mojeek_search"
    )
}

/// Catalog tools that start disabled. `firecrawl_crawl` spends a credit per page.
pub fn default_enabled(id: &str) -> bool {
    canonical_tool_id(id) != "firecrawl_crawl"
}

/// The three primary providers. Every other catalog tool is a gap filler.
pub fn primary_provider(id: &str) -> Option<&'static str> {
    let id = canonical_tool_id(id);
    if id.starts_with("firecrawl_") {
        Some("firecrawl")
    } else if id.starts_with("sociavault_") {
        Some("sociavault")
    } else if id.starts_with("hunter_") {
        Some("hunter")
    } else {
        None
    }
}

/// Estimated provider credits for one call. Free registry tools return none.
/// Allowances and overrides live with the investigation budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EndpointCost {
    pub provider: &'static str,
    pub credits: u32,
}

pub fn endpoint_cost(id: &str) -> Option<EndpointCost> {
    let cost = |provider, credits| EndpointCost { provider, credits };
    match canonical_tool_id(id) {
        "firecrawl_search" => Some(cost("firecrawl", 2)),
        "firecrawl_scrape"
        | "firecrawl_map"
        | "firecrawl_google_search"
        | "firecrawl_yandex_search"
        | "firecrawl_mojeek_search" => Some(cost("firecrawl", 1)),
        "firecrawl_batch_scrape" => Some(cost("firecrawl", BATCH_SCRAPE_DEFAULT_URLS as u32)),
        "firecrawl_crawl" => Some(cost("firecrawl", providers::CRAWL_MAX_PAGES as u32)),
        "firecrawl_extract" => Some(cost("firecrawl", providers::EXTRACT_CREDITS)),
        // Free Hunter reads: no credits, but the key and the rate limit still apply.
        "hunter_domain_finder" | "hunter_email_count" | "hunter_email_insight" => {
            Some(cost("hunter", 0))
        }
        "hunter_domain_search"
        | "hunter_email_finder"
        | "hunter_email_verifier"
        | "hunter_company_enrichment"
        | "hunter_person_enrichment"
        | "hunter_combined_enrichment" => Some(cost("hunter", 1)),
        id if id.starts_with("sociavault_") && definition(id).is_some() => {
            Some(cost("sociavault", 1))
        }
        // Not credit-metered: the per-turn call caps are their only budget.
        id if news_legal::provider(id).is_some() => {
            news_legal::provider(id).map(|provider| cost(provider, 0))
        }
        id if atlas_news::provider(id).is_some() => {
            atlas_news::provider(id).map(|provider| cost(provider, 0))
        }
        "whoxy_whois_history" => Some(cost("whoxy", 1)),
        _ => None,
    }
}

/// Credits one call with these arguments is expected to cost: batch scrape and crawl
/// charge per page, so the hold follows the URL count or the page limit.
pub fn estimated_cost(id: &str, args: &Value) -> Option<EndpointCost> {
    let base = endpoint_cost(id)?;
    let pages = match canonical_tool_id(id) {
        "firecrawl_batch_scrape" => batch_urls(args)
            .map(|urls| urls.len() as u32)
            .unwrap_or(base.credits),
        "firecrawl_crawl" => providers::crawl_limit(args)
            .map(|limit| limit as u32)
            .unwrap_or(base.credits),
        _ => return Some(base),
    };
    Some(EndpointCost {
        credits: pages.max(1),
        ..base
    })
}

pub fn scarce_provider(id: &str) -> bool {
    endpoint_cost(id).is_some()
}

/// Credits a provider reported on the raw response, when the payload includes them.
pub fn reported_credits(raw: &str) -> Option<u32> {
    let value: Value = serde_json::from_str(raw).ok()?;
    for pointer in [
        "/creditsUsed",
        "/credits_used",
        "/data/creditsUsed",
        "/data/credits_used",
        "/meta/credits_used",
    ] {
        if let Some(credits) = value.pointer(pointer).and_then(Value::as_u64) {
            if credits <= u64::from(u32::MAX) {
                return Some(credits as u32);
            }
        }
    }
    None
}
fn optional_keys(id: &str) -> &'static [&'static str] {
    match canonical_tool_id(id) {
        "crtsh_certificates" | "commoncrawl_urls" => &["limit"],
        "mnemonic_passive_dns" => &["limit", "offset"],
        "wayback_availability" => &["timestamp"],
        "arquivo_history" | "nominatim_geocode" => &["limit"],
        "firecrawl_search" => &["limit", "sources", "categories", "tbs", "location"],
        "firecrawl_google_search" | "firecrawl_yandex_search" | "firecrawl_mojeek_search" => {
            &["limit"]
        }
        "firecrawl_scrape" => &["formats"],
        "firecrawl_map" => &["search", "limit"],
        "firecrawl_crawl" => &["limit"],
        "hunter_domain_search" => &["limit"],
        "hunter_domain_finder" => &["limit", "perfect_match"],
        "hunter_email_count" => &["type"],
        "sociavault_profile" | "sociavault_user_content" | "sociavault_search_users" => {
            &["endpoint"]
        }
        "sociavault_search" => &["endpoint", "subreddit"],
        "hunter_email_finder" => &["last_name"],
        "github_repositories" | "gitlab_projects" => &["limit", "page"],
        "stackexchange_users" => &["site"],
        "overpass_places" => &["feature"],
        "newsapi_search" => &["from", "to", "language", "sort_by", "domains"],
        "newsapi_headlines" => &["country", "category"],
        "gnews_search" => &["from", "lang"],
        "newsdata_latest" => &["timeframe", "language"],
        "currents_latest" => &["language"],
        "courtlistener_case_search" => &["court", "filed_after", "filed_before"],
        "courtlistener_docket_search" => &["court", "filed_after"],
        "whatsmyname_lookup" => &["categories", "sites", "platforms", "max_sites"],
        "whoxy_whois_history" => &["from", "to", "limit"],
        "holehe_email_lookup" => &["services", "max_sites"],
        "dork_generate" => &[
            "purpose",
            "category",
            "target",
            "query",
            "objective",
            "operand_overrides",
            "max_queries",
            "domain",
            "keywords",
            "categories",
            "template_ids",
            "title",
            "url_text",
            "excluded_domain",
            "url",
            "subject",
        ],
        _ => &[],
    }
}
fn key_schema(key: &str) -> Value {
    match key {
        "latitude" | "longitude" => json!({"type": "number"}),
        "radius_m" | "limit" | "offset" | "max_sites" | "max_queries" => {
            json!({"type": "integer"})
        }
        "urls" | "sources" | "categories" | "formats" | "sites" | "template_ids" | "services" => {
            json!({"type": "array", "items": {"type": "string"}})
        }
        "perfect_match" => json!({"type": "boolean"}),
        _ => json!({"type": "string"}),
    }
}
impl ToolDefinition {
    pub fn schema(&self) -> Value {
        let mut props = serde_json::Map::new();
        let mut required = Vec::new();
        let mut alternatives = Vec::new();
        for group in self.inputs {
            let keys: Vec<_> = group.split('|').collect();
            for key in &keys {
                props.insert((*key).into(), key_schema(key));
            }
            if keys.len() == 1 {
                required.push(keys[0]);
            } else {
                alternatives.push(json!({"anyOf":keys.iter().map(|key|json!({"required":[key]})).collect::<Vec<_>>()}));
            }
        }
        for key in optional_keys(self.id) {
            props.insert((*key).into(), key_schema(key));
        }
        json!({"type":"object","properties":props,"required":required,"allOf":alternatives,"additionalProperties":false})
    }
    pub fn example_input(&self) -> Value {
        let mut values = serde_json::Map::new();
        for group in self.inputs {
            let key = group.split('|').next().unwrap_or("");
            let value = match key {
                "domain" | "domain_or_url" => json!("example.org"),
                "domain_or_ip" => json!("example.org"),
                "ip" => json!("8.8.8.8"),
                "url" => json!("https://example.org"),
                "query" => json!("example"),
                "objective" => json!("Find public annual reports for the organization"),
                "company_name" => json!("Example Inc"),
                "cik" => json!("0000320193"),
                "name" => json!("Example"),
                "username" => json!("ExampleUser"),
                "address_or_place" => json!("New York, NY"),
                "us_address" => json!("1600 Pennsylvania Ave NW, Washington, DC"),
                "latitude" => json!(40.7128),
                "longitude" => json!(-74.006),
                "radius_m" => json!(500),
                "bitcoin_address" => json!("1BoatSLRHtKNngkdXEeobR76b53LETtpyT"),
                "cve_id" => json!("CVE-2021-44228"),
                "ecosystem" => json!("Maven"),
                "package_name" => json!("org.apache.logging.log4j:log4j-core"),
                "version" => json!("2.14.1"),
                "email" => json!("ada@example.org"),
                "platform" => {
                    let served = providers::sociavault_platforms(self.id);
                    json!(if served.contains(&"twitter") {
                        "twitter"
                    } else {
                        served.first().copied().unwrap_or("twitter")
                    })
                }
                "handle" => json!("example"),
                "first_name" => json!("Ada"),
                "last_name" => json!("Lovelace"),
                "full_name" => json!("Ada Lovelace"),
                "linkedin_handle" => json!("ada-lovelace"),
                "company" => json!("Example Inc"),
                "country" => json!("us"),
                "urls" => json!(["https://example.org/about"]),
                _ => json!("example"),
            };
            values.insert(key.into(), value);
        }
        Value::Object(values)
    }
}
pub fn validate(id: &str, inputs: &Value) -> Result<()> {
    let id = canonical_tool_id(id);
    let def = definition(id).ok_or_else(|| anyhow!("unknown tool {id}"))?;
    let object = inputs
        .as_object()
        .ok_or_else(|| anyhow!("inputs must be a JSON object"))?;
    let allowed: Vec<_> = def
        .inputs
        .iter()
        .flat_map(|g| g.split('|'))
        .chain(optional_keys(id).iter().copied())
        .collect();
    for key in object.keys() {
        ensure!(allowed.contains(&key.as_str()), "unsupported input {key}");
    }
    for group in def.inputs {
        ensure!(
            group.split('|').any(|key| object.get(key).is_some()),
            "missing {}",
            group.replace('|', " or ")
        );
    }
    if id == "whatsmyname_lookup" {
        str_arg(inputs, "username")?;
        return Ok(());
    }
    if id == "holehe_email_lookup" {
        holehe::normalize_email(str_arg(inputs, "email")?)?;
        if let Some(services) = inputs.get("services") {
            let ids = services
                .as_array()
                .ok_or_else(|| anyhow!("services must be an array of strings"))?
                .iter()
                .map(|v| {
                    v.as_str()
                        .map(str::to_string)
                        .ok_or_else(|| anyhow!("services must be an array of strings"))
                })
                .collect::<Result<Vec<_>>>()?;
            holehe::select_services(Some(&ids), holehe::MAX_SITES_CAP)?;
        }
        return Ok(());
    }
    if id == "whoxy_whois_history" {
        whoxy::parse_query(inputs)?;
        return Ok(());
    }
    if id == "dork_generate" {
        return Ok(());
    }
    if id == "commoncrawl_urls" && inputs.get("index").is_none() {
        let mut copy = inputs.clone();
        copy["index"] = json!("CC-MAIN-0000-00");
        request(id, &copy).map(|_| ())
    } else {
        request(id, inputs).map(|_| ())
    }
}
fn str_arg<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty() && s.len() <= 300)
        .ok_or_else(|| anyhow!("missing or invalid {key}"))
}
fn one_of<'a>(v: &'a Value, keys: &[&'a str]) -> Result<(&'a str, &'a str)> {
    keys.iter()
        .find_map(|k| str_arg(v, k).ok().map(|s| (*k, s)))
        .ok_or_else(|| anyhow!("provide {}", keys.join(" or ")))
}
fn domain(s: &str) -> Result<String> {
    let s = s.trim_end_matches('.').to_ascii_lowercase();
    ensure!(
        s.len() <= 253
            && s.contains('.')
            && s.split('.').all(|p| !p.is_empty()
                && p.len() <= 63
                && p.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                && !p.starts_with('-')
                && !p.ends_with('-')),
        "invalid domain"
    );
    Ok(s)
}
fn ip(s: &str) -> Result<String> {
    Ok(s.parse::<IpAddr>()?.to_string())
}
fn bitcoin(s: &str) -> Result<String> {
    if s.to_ascii_lowercase().starts_with("bc1") {
        ensure!(
            s == s.to_ascii_lowercase() || s == s.to_ascii_uppercase(),
            "mixed case Bech32 address"
        );
        let lower = s.to_ascii_lowercase();
        let chars = "qpzry9x8gf2tvdw0s3jn54khce6mua7l";
        let values: Vec<u8> = lower[3..]
            .chars()
            .map(|c| {
                chars
                    .find(c)
                    .map(|n| n as u8)
                    .ok_or_else(|| anyhow!("invalid Bech32 character"))
            })
            .collect::<Result<_>>()?;
        ensure!(
            values.len() >= 7 && values.len() <= 87,
            "invalid Bech32 length"
        );
        let mut chk: u32 = 1;
        for value in [3u8, 3, 0, 2, 3].into_iter().chain(values.iter().copied()) {
            let top = chk >> 25;
            chk = ((chk & 0x1ffffff) << 5) ^ u32::from(value);
            for (i, g) in [0x3b6a57b2, 0x26508e6d, 0x1ea119fa, 0x3d4233dd, 0x2a1462b3]
                .iter()
                .enumerate()
            {
                if (top >> i) & 1 == 1 {
                    chk ^= g;
                }
            }
        }
        let witness = values[0];
        ensure!(witness <= 16, "invalid witness version");
        ensure!(
            chk == if witness == 0 { 1 } else { 0x2bc830a3 },
            "invalid Bech32 checksum"
        );
        let program = &values[1..values.len() - 6];
        let mut acc = 0u32;
        let mut bits = 0;
        let mut bytes = Vec::new();
        for value in program {
            acc = (acc << 5) | u32::from(*value);
            bits += 5;
            while bits >= 8 {
                bits -= 8;
                bytes.push(((acc >> bits) & 255) as u8);
            }
        }
        ensure!(
            bits < 5 && ((acc << (8 - bits)) & 255) == 0,
            "invalid witness padding"
        );
        ensure!(
            (2..=40).contains(&bytes.len()) && (witness != 0 || [20, 32].contains(&bytes.len())),
            "invalid witness program"
        );
        return Ok(s.into());
    }
    let alphabet = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    ensure!((26..=35).contains(&s.len()), "invalid Base58 length");
    let mut bytes = vec![0u8];
    for ch in s.chars() {
        let digit = alphabet
            .find(ch)
            .ok_or_else(|| anyhow!("invalid Base58 character"))? as u32;
        let mut carry = digit;
        for byte in bytes.iter_mut().rev() {
            carry += u32::from(*byte) * 58;
            *byte = (carry & 255) as u8;
            carry >>= 8;
        }
        while carry > 0 {
            bytes.insert(0, (carry & 255) as u8);
            carry >>= 8;
        }
    }
    let zeros = s.chars().take_while(|c| *c == '1').count();
    let mut decoded = vec![0u8; zeros];
    decoded.extend(bytes.into_iter().skip_while(|b| *b == 0));
    ensure!(
        decoded.len() == 25 && [0u8, 5].contains(&decoded[0]),
        "unsupported Bitcoin network or address type"
    );
    let hash = Sha256::digest(Sha256::digest(&decoded[..21]));
    ensure!(hash[..4] == decoded[21..], "invalid Base58 checksum");
    Ok(s.into())
}
fn url_arg(s: &str) -> Result<String> {
    let u = Url::parse(s)?;
    ensure!(
        (u.scheme() == "http" || u.scheme() == "https") && u.host_str().is_some(),
        "HTTP(S) URL required"
    );
    let host = u.host_str().unwrap_or("");
    ensure!(
        host != "localhost" && !host.ends_with(".localhost"),
        "local targets are unsupported"
    );
    if let Ok(ip) = host.parse::<IpAddr>() {
        let public = match ip {
            IpAddr::V4(ip) => {
                !(ip.is_private() || ip.is_loopback() || ip.is_link_local() || ip.is_unspecified())
            }
            IpAddr::V6(ip) => {
                !(ip.is_loopback()
                    || ip.is_unique_local()
                    || ip.is_unicast_link_local()
                    || ip.is_unspecified())
            }
        };
        ensure!(public, "private targets are unsupported");
    }
    Ok(u.to_string())
}
fn bounded(s: &str) -> Result<String> {
    ensure!(
        s.len() <= 200 && !s.chars().any(char::is_control),
        "input too long or contains controls"
    );
    Ok(s.to_string())
}
fn number_arg(v: &Value, key: &str, default: u64, max: u64) -> Result<String> {
    let n = match v.get(key) {
        Some(value) => value
            .as_u64()
            .ok_or_else(|| anyhow!("{key} must be a nonnegative integer"))?,
        None => default,
    };
    ensure!(
        n <= max && (key == "offset" || n > 0),
        "{key} exceeds the supported bound"
    );
    Ok(n.to_string())
}
/// Main-content markdown kept from one Firecrawl page. Enough for an about,
/// contact, or team page; a longer page keeps its opening and an ellipsis.
const PAGE_MARKDOWN_CHARS: usize = 16_000;

fn clip_page(value: &str) -> String {
    if value.chars().count() <= PAGE_MARKDOWN_CHARS {
        value.to_string()
    } else {
        let mut clipped: String = value.chars().take(PAGE_MARKDOWN_CHARS - 1).collect();
        clipped.push('…');
        clipped
    }
}
fn clip_text(value: &str) -> String {
    let flat = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= 280 {
        flat
    } else {
        let mut clipped: String = flat.chars().take(279).collect();
        clipped.push('…');
        clipped
    }
}
fn email_address(value: &str) -> Result<String> {
    let value = value.trim();
    ensure!(
        !value.chars().any(char::is_control) && value.len() <= 254,
        "invalid email"
    );
    let (local, host) = value
        .split_once('@')
        .ok_or_else(|| anyhow!("invalid email"))?;
    ensure!(
        !local.is_empty()
            && local.len() <= 64
            && !local.starts_with('.')
            && !local.ends_with('.')
            && !local.contains("..")
            && local
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '%' | '+' | '-')),
        "invalid email"
    );
    Ok(format!(
        "{}@{}",
        local.to_ascii_lowercase(),
        domain(&host.to_ascii_lowercase())?
    ))
}
pub(crate) fn social_token(value: &str) -> Result<String> {
    let value = value.trim().trim_start_matches('@');
    ensure!(
        (1..=80).contains(&value.len())
            && value
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')),
        "invalid social handle"
    );
    Ok(value.to_string())
}
fn linkedin_handle(value: &str) -> Result<String> {
    let value = value.trim().trim_start_matches('@');
    ensure!(
        (1..=100).contains(&value.len())
            && value
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')),
        "invalid LinkedIn handle"
    );
    Ok(value.to_string())
}
fn https_on_host(raw: &str, hosts: &[&str]) -> Result<Url> {
    let url = Url::parse(raw.trim())?;
    let host = url.host_str().unwrap_or("");
    ensure!(
        url.scheme() == "https"
            && hosts
                .iter()
                .any(|allowed| { host == *allowed || host.ends_with(&format!(".{allowed}")) }),
        "unsupported profile URL"
    );
    Ok(url)
}
fn hunter_observations(id: &str, value: &Value) -> Value {
    if let Some(card) = providers::hunter_observations(id, value) {
        return card;
    }
    let data = value.get("data").unwrap_or(&Value::Null);
    match id {
        "hunter_domain_search" => {
            let emails = data
                .get("emails")
                .and_then(Value::as_array)
                .map(|rows| {
                    rows.iter()
                        .take(10)
                        .map(|row| {
                            json!({
                                "value": row.get("value"),
                                "type": row.get("type"),
                                "confidence": row.get("confidence"),
                                "first_name": row.get("first_name"),
                                "last_name": row.get("last_name"),
                                "position": row.get("position"),
                                "department": row.get("department"),
                                "seniority": row.get("seniority"),
                                "linkedin": row.get("linkedin"),
                                "twitter": row.get("twitter"),
                                "verification": row.pointer("/verification/status"),
                            })
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            json!({
                "domain": data.get("domain"),
                "organization": data.get("organization"),
                "pattern": data.get("pattern"),
                "accept_all": data.get("accept_all"),
                "webmail": data.get("webmail"),
                "emails": emails,
            })
        }
        "hunter_email_finder" => json!({
            "email": data.get("email"),
            "score": data.get("score"),
            "first_name": data.get("first_name"),
            "last_name": data.get("last_name"),
            "position": data.get("position"),
            "company": data.get("company"),
            "domain": data.get("domain"),
            "linkedin_url": data.get("linkedin_url"),
            "twitter": data.get("twitter"),
            "verification": data.get("verification"),
            "accept_all": data.get("accept_all"),
        }),
        "hunter_email_verifier" => json!({
            "email": data.get("email").or_else(|| value.pointer("/meta/params/email")),
            "status": data.get("status"),
            "result": data.get("result"),
            "score": data.get("score"),
            "regexp": data.get("regexp"),
            "gibberish": data.get("gibberish"),
            "disposable": data.get("disposable"),
            "webmail": data.get("webmail"),
            "mx_records": data.get("mx_records"),
            "smtp_server": data.get("smtp_server"),
            "smtp_check": data.get("smtp_check"),
            "accept_all": data.get("accept_all"),
            "block": data.get("block"),
            "sources": data.get("sources").and_then(Value::as_array).map(Vec::len).unwrap_or(0),
        }),
        _ => value.clone(),
    }
}
fn push_profile_link(links: &mut Vec<String>, raw: &str) {
    push_link(links, raw, 8);
}
/// Adds an http(s) link that is not a media CDN, once, up to `cap` links.
fn push_link(links: &mut Vec<String>, raw: &str, cap: usize) {
    let Ok(url) = Url::parse(raw.trim()) else {
        return;
    };
    let host = url.host_str().unwrap_or("");
    let media = [
        "cdninstagram",
        "fbcdn",
        "twimg",
        "tiktokcdn",
        "googleusercontent",
        "ggpht",
        "licdn.com",
        "jtvnw.net",
        "ytimg.com",
    ]
    .iter()
    .any(|needle| host.contains(needle));
    if media || (url.scheme() != "https" && url.scheme() != "http") {
        return;
    }
    let text = url.to_string();
    if text.chars().count() > 300
        || links.iter().any(|existing| existing == &text)
        || links.len() >= cap
    {
        return;
    }
    links.push(text);
}
fn sociavault_card(value: &Value) -> Value {
    let mut name = String::new();
    let mut handle = String::new();
    let mut biography = String::new();
    let mut stats = serde_json::Map::new();
    let mut links = Vec::new();
    fn walk(
        value: &Value,
        depth: usize,
        name: &mut String,
        handle: &mut String,
        biography: &mut String,
        stats: &mut serde_json::Map<String, Value>,
        links: &mut Vec<String>,
    ) {
        if depth > 5 {
            return;
        }
        let Some(object) = value.as_object() else {
            if let Some(rows) = value.as_array() {
                for row in rows.iter().take(6) {
                    walk(row, depth + 1, name, handle, biography, stats, links);
                }
            }
            return;
        };
        for (key, child) in object {
            let lowered = key.to_ascii_lowercase().replace('-', "_");
            if matches!(
                lowered.as_str(),
                "itemlist"
                    | "item_list"
                    | "posts"
                    | "recentposts"
                    | "recent_posts"
                    | "activity"
                    | "videos"
                    | "allvideos"
                    | "all_videos"
                    | "recentbroadcasts"
                    | "recent_broadcasts"
                    | "similarprofiles"
                    | "similar_profiles"
                    | "similarstreamers"
                    | "edge_owner_to_timeline_media"
                    | "edge_felix_video_timeline"
                    | "articles"
                    | "recommendations"
                    | "experience"
                    | "education"
                    | "publications"
                    | "projects"
            ) {
                continue;
            }
            if biography.is_empty()
                && matches!(
                    lowered.as_str(),
                    "biography"
                        | "bio"
                        | "description"
                        | "about"
                        | "pageintro"
                        | "page_intro"
                        | "signature"
                )
            {
                if let Some(text) = child
                    .as_str()
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                {
                    *biography = clip_text(text);
                }
            }
            if name.is_empty()
                && depth <= 4
                && matches!(
                    lowered.as_str(),
                    "full_name" | "fullname" | "nickname" | "display_name" | "displayname" | "name"
                )
            {
                if let Some(text) = child
                    .as_str()
                    .map(str::trim)
                    .filter(|text| !text.is_empty() && text.chars().count() <= 120)
                {
                    *name = text.to_string();
                }
            }
            if handle.is_empty()
                && depth <= 4
                && matches!(
                    lowered.as_str(),
                    "username" | "handle" | "screen_name" | "screenname" | "uniqueid" | "unique_id"
                )
            {
                if let Some(text) = child.as_str().map(str::trim).filter(|text| {
                    !text.is_empty() && text.chars().count() <= 80 && !text.contains(' ')
                }) {
                    *handle = text.trim_start_matches('@').to_string();
                }
            }
            let numeric = child.is_number()
                || child.as_str().is_some_and(|text| {
                    !text.is_empty() && text.chars().all(|c| c.is_ascii_digit())
                });
            if stats.len() < 12
                && numeric
                && (lowered.contains("follower")
                    || lowered.contains("following")
                    || lowered.contains("subscriber")
                    || lowered.contains("friend")
                    || lowered.contains("heart")
                    || lowered.contains("video_count")
                    || lowered.contains("videocount")
                    || lowered.contains("statuses_count")
                    || lowered.contains("media_count")
                    || lowered.contains("view_count")
                    || lowered.contains("viewcount")
                    || lowered.contains("talking_about")
                    || lowered.contains("talkingabout")
                    || lowered == "connections"
                    || lowered == "likes")
            {
                stats.insert(key.clone(), child.clone());
            }
            if let Some(text) = child.as_str() {
                if text.starts_with("http://") || text.starts_with("https://") {
                    push_profile_link(links, text);
                }
            }
            walk(child, depth + 1, name, handle, biography, stats, links);
        }
    }
    walk(
        value,
        0,
        &mut name,
        &mut handle,
        &mut biography,
        &mut stats,
        &mut links,
    );
    json!({
        "name": name,
        "handle": handle,
        "biography": biography,
        "stats": stats,
        "links": links,
    })
}
// ---------------------------------------------------------------------------
// Named search-engine SERP handling (spec §9)
// ---------------------------------------------------------------------------

/// Requested organic results for one named SERP query: a single page of five.
const NAMED_SERP_LIMIT: usize = 5;

/// Body cap for every tool that is not a named search engine. The named SERP
/// engines read the larger [`search_engines::SERP_MAX_BODY_BYTES`] instead.
const DEFAULT_BODY_BYTES: usize = 1_000_000;

/// Wait before the single allowed SERP recovery fetch (spec §9 fix 6). The sleep
/// is cancelled when the enclosing task is dropped, so a shutdown never spends an
/// extra engine request.
const SERP_RECOVERY_WAIT: Duration = Duration::from_secs(1);

/// The `limit` one named SERP query asks for: the optional `limit` input, bounded
/// to [`NAMED_SERP_LIMIT`], else the default page of five.
fn named_serp_limit(inputs: &Value) -> usize {
    inputs
        .get("limit")
        .and_then(Value::as_u64)
        .and_then(|n| usize::try_from(n).ok())
        .unwrap_or(NAMED_SERP_LIMIT)
        .clamp(1, NAMED_SERP_LIMIT)
}

/// Request context for a named engine query, carried from the ORIGINAL tool input
/// (spec §9 fix 2). The query is read once, before any redirect, so a redirected
/// URL can never rewrite what was asked.
fn serp_request_for(tool_id: &str, inputs: &Value) -> Result<search_engines::SerpRequest> {
    let query = str_arg(inputs, "query")?;
    search_engines::SerpRequest::for_tool(tool_id, query, named_serp_limit(inputs))
}

/// Body cap for one tool: a named SERP engine reads up to 8 MiB, everything else
/// keeps the flat 1 MB limit.
fn body_cap(id: &str) -> usize {
    if search_engines::is_named_engine_tool(id) {
        search_engines::SERP_MAX_BODY_BYTES
    } else {
        DEFAULT_BODY_BYTES
    }
}

/// Terminal status for one typed SERP outcome. Only a verified zero reaches
/// `no_results`: an unrecognised page, a challenge wall, a rate limit, a provider
/// failure and an oversize body are all failures (spec §9 fix 1).
fn serp_status(outcome: search_engines::SerpOutcome) -> &'static str {
    use search_engines::SerpOutcome;
    match outcome {
        SerpOutcome::Valid => "completed",
        SerpOutcome::VerifiedZero => "completed",
        SerpOutcome::Challenge | SerpOutcome::Consent => "blocked",
        SerpOutcome::RateLimited => "rate_limited",
        SerpOutcome::ParserMismatch
        | SerpOutcome::UpstreamFailure
        | SerpOutcome::ResponseTooLarge => "failed",
    }
}

/// Machine reason for one typed SERP failure. Never the words "no results": the
/// outcome says exactly what the engine or the provider did.
fn serp_reason(outcome: search_engines::SerpOutcome) -> &'static str {
    use search_engines::SerpOutcome;
    match outcome {
        SerpOutcome::Challenge => "serp_challenge: engine served a challenge page",
        SerpOutcome::Consent => "serp_consent: engine served a consent page",
        SerpOutcome::RateLimited => "serp_rate_limited: engine rate limited the request",
        SerpOutcome::ParserMismatch => {
            "serp_parser_mismatch: engine page layout not recognised, result count unknown"
        }
        SerpOutcome::UpstreamFailure => "serp_upstream_failure: provider or engine request failed",
        SerpOutcome::ResponseTooLarge => {
            "serp_response_too_large: engine response exceeded the body bound"
        }
        SerpOutcome::Valid | SerpOutcome::VerifiedZero => "serp_ok",
    }
}

/// Observations for a named SERP query whose body never reached the parser: over
/// the body bound, or a provider HTTP failure. The typed outcome is always
/// present, so the payload can never be read as a verified zero.
fn serp_unparsed_observations(
    serp: Option<&search_engines::SerpRequest>,
    outcome: search_engines::SerpOutcome,
    detail: &str,
) -> Value {
    let empty: Vec<search_engines::EngineSearchResult> = Vec::new();
    json!({
        "engine": serp.map(|req| req.engine.clone()).unwrap_or_default(),
        "query": serp.map(|req| req.query.clone()).unwrap_or_default(),
        "serp_url": serp.map(|req| req.serp_url.clone()).unwrap_or_default(),
        "items": empty.clone(),
        "results": empty,
        "outcome": outcome.as_str(),
        "result_count": 0,
        "usable_results": 0,
        "recoverable": false,
        "parser_version": search_engines::PARSER_VERSION,
        "parser_input": search_engines::ParserInput::Missing.as_str(),
        "status_region": "unparsed",
        "provider_error": clip_text(detail),
        "evidence_form": "serp",
    })
}

/// Typed observations for one parsed named SERP page. The outcome is a machine
/// field, never inferred from an empty array: only `VerifiedZero` proves the
/// engine returned nothing, and `ParserMismatch` proves nothing at all.
fn serp_observations(req: &search_engines::SerpRequest, raw: &Value) -> Result<(Value, bool)> {
    let parsed = search_engines::parse_serp_response(req, raw)?;
    let search_engines::SerpParse {
        items,
        outcome,
        diagnostics,
    } = parsed;
    let usable = items.len();
    let truncated = matches!(outcome, search_engines::SerpOutcome::ResponseTooLarge);
    // Bounded recovery is decided against the real diagnostics (spec §9 fix 6):
    // a links-only payload would re-request the same shape, so it is not retried.
    let recoverable = search_engines::retry_allowed(outcome, &diagnostics);
    let rejections = diagnostics
        .rejections
        .iter()
        .map(|(reason, count)| json!({"reason": reason, "count": count}))
        .collect::<Vec<_>>();
    Ok((
        json!({
            "engine": req.engine.clone(),
            "query": req.query.clone(),
            "serp_url": req.serp_url.clone(),
            "items": items.clone(),
            "results": items,
            "outcome": outcome.as_str(),
            "result_count": usable,
            "usable_results": usable,
            "recoverable": recoverable,
            "parser_version": search_engines::PARSER_VERSION,
            "fetch_contract_version": search_engines::FETCH_CONTRACT_VERSION,
            "parser_input": diagnostics.parser_input.as_str(),
            "target_status": diagnostics.target_status,
            "final_url": diagnostics.final_url,
            "requested_url": diagnostics.requested_url,
            "input_bytes": diagnostics.input_bytes,
            "dom_candidates": diagnostics.dom_candidates,
            "status_region": diagnostics.status_region,
            "rejections": rejections,
            "provider_warning": diagnostics.provider_warning,
            "provider_error": diagnostics.provider_error,
            "evidence_form": "serp",
        }),
        truncated,
    ))
}

/// Call sites that carry no request context: every tool except the three named
/// search engines, which pass their original request to [`parse_observations_with`].
#[cfg(test)]
fn parse_observations(
    id: &str,
    raw: &str,
    content_type: &str,
    ndjson: bool,
) -> Result<(Value, bool)> {
    parse_observations_with(id, raw, content_type, ndjson, None)
}

/// Named SERP tools pass the original request so the parser is not rebuilt from a redirect.
fn parse_observations_with(
    id: &str,
    raw: &str,
    content_type: &str,
    ndjson: bool,
    serp: Option<&search_engines::SerpRequest>,
) -> Result<(Value, bool)> {
    let id = canonical_tool_id(id);
    ensure!(
        !content_type.contains("html") && !raw.trim_start().starts_with('<'),
        "HTML response instead of API data"
    );
    if id == "hackertarget_hostsearch" {
        ensure!(
            !raw.to_ascii_lowercase().contains("error") && !raw.contains("API count exceeded"),
            "provider quota or error: {}",
            raw.chars().take(200).collect::<String>()
        );
        let rows: Vec<Value> = raw
            .lines()
            .take(100)
            .filter_map(|l| {
                l.split_once(',')
                    .map(|(host, ip)| json!({"hostname":host,"ip":ip}))
            })
            .collect();
        return Ok((json!(rows), raw.lines().count() > 100));
    }
    if ndjson {
        let mut rows = Vec::new();
        for line in raw.lines().take(100) {
            rows.push(serde_json::from_str::<Value>(line)?);
        }
        return Ok((json!(rows), raw.lines().count() > 100));
    }
    let v: Value = serde_json::from_str(raw).map_err(|e| anyhow!("malformed JSON: {e}"))?;
    if news_legal::provider(id).is_some() {
        return news_legal::observations(id, &v);
    }
    if atlas_news::provider(id).is_some() {
        return atlas_news::observations(id, &v);
    }
    if let Some(error) = v.get("error").or_else(|| v.get("errors")) {
        if !error.is_null() {
            return Err(anyhow!(
                "provider error: {}",
                error.to_string().chars().take(250).collect::<String>()
            ));
        }
    }
    if id == "firecrawl_search" {
        if v.get("success").and_then(Value::as_bool) == Some(false) {
            let message = v
                .get("error")
                .map(|err| err.to_string())
                .unwrap_or_else(|| "search failed".into());
            return Err(anyhow!(
                "Firecrawl search failed: {}",
                message.chars().take(250).collect::<String>()
            ));
        }
        let rows = v
            .pointer("/data/web")
            .and_then(Value::as_array)
            .or_else(|| v.get("data").and_then(Value::as_array));
        let truncated = rows.is_some_and(|rows| rows.len() > 8);
        let results: Vec<Value> = rows
            .into_iter()
            .flatten()
            .take(8)
            .filter_map(|row| {
                let url = row.get("url").and_then(Value::as_str).unwrap_or("");
                if url.is_empty() {
                    return None;
                }
                let snippet = row
                    .get("description")
                    .or_else(|| row.get("snippet"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                Some(json!({
                    "title": clip_text(row.get("title").and_then(Value::as_str).unwrap_or("")),
                    "url": url,
                    "snippet": clip_text(snippet),
                }))
            })
            .collect();
        return Ok((
            json!({"results": results, "infoboxes": [], "evidence_form": "snippet"}),
            truncated,
        ));
    }
    if matches!(
        id,
        "firecrawl_google_search" | "firecrawl_yandex_search" | "firecrawl_mojeek_search"
    ) {
        // The outcome contract is typed: an unrecognised page is a parser
        // mismatch, never a zero (spec §9 fix 1).
        let Some(req) = serp else {
            return Err(anyhow!("named engine request context is missing"));
        };
        return serp_observations(req, &v);
    }
    if id == "firecrawl_scrape" {
        if v.get("success").and_then(Value::as_bool) == Some(false) {
            let message = v
                .get("error")
                .map(|err| err.to_string())
                .unwrap_or_else(|| "page retrieval failed".into());
            return Err(anyhow!(
                "Firecrawl page retrieval failed: {}",
                message.chars().take(250).collect::<String>()
            ));
        }
        let data = v.get("data").unwrap_or(&v);
        let markdown = data
            .get("markdown")
            .or_else(|| data.get("content"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let title = data
            .pointer("/metadata/title")
            .or_else(|| data.get("title"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let page_url = data
            .pointer("/metadata/sourceURL")
            .or_else(|| data.get("url"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let clipped = clip_page(markdown);
        return Ok((
            json!({
                "title": clip_text(title),
                "url": page_url,
                "markdown": clipped,
                "evidence_form": "page",
            }),
            markdown.chars().count() > clipped.chars().count(),
        ));
    }
    if matches!(
        id,
        "firecrawl_map" | "firecrawl_batch_scrape" | "firecrawl_crawl" | "firecrawl_extract"
    ) {
        if v.get("success").and_then(Value::as_bool) == Some(false)
            || v.get("status").and_then(Value::as_str) == Some("failed")
        {
            let message = v
                .get("error")
                .map(|err| err.to_string())
                .unwrap_or_else(|| "job failed".into());
            return Err(anyhow!(
                "Firecrawl {id} failed: {}",
                message.chars().take(250).collect::<String>()
            ));
        }
        return Ok(match id {
            "firecrawl_map" => (providers::map_observations(&v, ""), false),
            "firecrawl_extract" => providers::extract_observations(&v),
            _ => providers::job_observations(&v, false),
        });
    }
    if id.starts_with("hunter_") {
        return Ok((hunter_observations(id, &v), false));
    }
    if id.starts_with("sociavault_") && id != "sociavault_profile" {
        if v.get("success").and_then(Value::as_bool) == Some(false) {
            let message = v
                .get("error")
                .or_else(|| v.get("message"))
                .map(|err| err.to_string())
                .unwrap_or_else(|| "request failed".into());
            return Err(anyhow!(
                "SociaVault {id} failed: {}",
                message.chars().take(250).collect::<String>()
            ));
        }
        return Ok((providers::sociavault_items(id, &v), false));
    }
    if id == "sociavault_profile" {
        if v.get("success").and_then(Value::as_bool) == Some(false) {
            let message = v
                .get("error")
                .map(|err| err.to_string())
                .unwrap_or_else(|| "profile lookup failed".into());
            return Err(anyhow!(
                "SociaVault profile failed: {}",
                message.chars().take(250).collect::<String>()
            ));
        }
        return Ok((sociavault_card(&v), false));
    }
    if id == "whoxy_whois_history" {
        let full = whoxy::parse_history_envelope(&v, "")?;
        return Ok((whoxy::bounded_model_view(&full), false));
    }
    if id == "crtsh_certificates" {
        let mut hosts = std::collections::BTreeSet::new();
        if let Some(rows) = v.as_array() {
            for row in rows.iter().take(1000) {
                if let Some(names) = row.get("name_value").and_then(Value::as_str) {
                    for name in names.lines() {
                        hosts.insert(name.to_string());
                    }
                }
            }
        }
        return Ok((
            json!({"hostnames":hosts,"certificates":v.as_array().map(|a|a.iter().take(100).cloned().collect::<Vec<_>>()).unwrap_or_default()}),
            v.as_array().is_some_and(|a| a.len() > 100),
        ));
    }
    Ok((v, false))
}
/// True when the observations prove the tool genuinely returned nothing.
///
/// A named search engine is deliberately stricter (spec §9 fix 1): only a
/// recognized engine status region with a supported phrase is a verified zero.
/// An empty array never is.
pub(crate) fn no_results(id: &str, value: &Value) -> bool {
    // Spec §9 fix 1: an empty array alone is never proof of zero for a named
    // search engine. Only a recognized engine status region with a supported
    // phrase becomes a verified zero; every other outcome is a failure or a
    // block, classified before the generic rules below.
    if search_engines::is_named_engine_tool(id) {
        return value
            .get("outcome")
            .and_then(Value::as_str)
            .and_then(search_engines::SerpOutcome::parse)
            .is_some_and(|outcome| outcome == search_engines::SerpOutcome::VerifiedZero);
    }
    if value.is_null() || value.as_array().is_some_and(Vec::is_empty) {
        return true;
    }
    if id == "wayback_availability" {
        return value
            .get("archived_snapshots")
            .and_then(Value::as_object)
            .is_some_and(|o| o.is_empty());
    }
    if id == "nvd_cve" && value.get("totalResults").and_then(Value::as_u64) == Some(0) {
        return true;
    }
    let empty = |key: &str| {
        value
            .get(key)
            .and_then(Value::as_array)
            .is_none_or(Vec::is_empty)
    };
    match id {
        "firecrawl_map" => return empty("urls"),
        "firecrawl_batch_scrape" | "firecrawl_crawl" => return empty("pages"),
        "firecrawl_extract" => {
            return ["org_name", "legal_name", "domain", "address"]
                .iter()
                .all(|key| {
                    value
                        .get(*key)
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .is_empty()
                })
                && ["emails", "social_profiles", "people"]
                    .iter()
                    .all(|key| empty(key));
        }
        "hunter_domain_finder" => return empty("companies"),
        "hunter_email_count" => return value.get("total").and_then(Value::as_u64) == Some(0),
        "hunter_person_enrichment" => {
            return value.get("claimed_email").is_some()
                || value.get("full_name").is_none_or(Value::is_null)
                    && value.get("email").is_none_or(Value::is_null)
        }
        "hunter_combined_enrichment" => {
            return value.get("claimed_email").is_some()
                || value.get("person").is_none() && value.get("company").is_none()
        }
        "sociavault_google_search" => return empty("results"),
        id if news_legal::provider(id).is_some() || atlas_news::provider(id).is_some() => {
            return empty("results")
        }
        "sociavault_search" | "sociavault_search_users" | "sociavault_user_content" => {
            return empty("accounts") && empty("links") && empty("texts");
        }
        "whoxy_whois_history" => {
            return value.get("zero_history").and_then(Value::as_bool) == Some(true);
        }
        "holehe_email_lookup" => {
            return value
                .get("counts")
                .and_then(|c| c.get("registered"))
                .and_then(Value::as_u64)
                == Some(0)
                && value.get("partial").and_then(Value::as_bool) != Some(true);
        }
        _ => {}
    }
    if id == "firecrawl_scrape" {
        return value
            .get("markdown")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .is_empty();
    }
    if id == "hunter_email_finder" {
        return value
            .get("email")
            .and_then(Value::as_str)
            .unwrap_or("")
            .is_empty();
    }
    if id == "hunter_domain_search" {
        return value
            .get("emails")
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty)
            && value
                .get("domain")
                .and_then(Value::as_str)
                .unwrap_or("")
                .is_empty();
    }
    if id == "sociavault_profile" {
        return value
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .is_empty()
            && value
                .get("handle")
                .and_then(Value::as_str)
                .unwrap_or("")
                .is_empty()
            && value
                .get("biography")
                .and_then(Value::as_str)
                .unwrap_or("")
                .is_empty();
    }
    let pointer = match id {
        "github_repositories" | "stackexchange_users" => "/items",
        "grepapp_code_search" => "/hits/hits",
        "wikidata_entities" => "/search",
        "nvd_cve" => "/vulnerabilities",
        "urlscan_search" | "firecrawl_search" => "/results",
        "mnemonic_passive_dns" => "/data",
        "osv_package" => "/vulns",
        _ => return false,
    };
    value
        .pointer(pointer)
        .and_then(Value::as_array)
        .is_some_and(Vec::is_empty)
}

// ---------------------------------------------------------------------------
// Best-effort tool telemetry (spec §10)
// ---------------------------------------------------------------------------

/// Writes one telemetry event and forgets any failure. Instrumentation never
/// fails a tool run, and a tool run never creates a state directory as a side
/// effect of trying to observe itself.
fn note_telemetry(event: crate::telemetry::TelemetryEvent) {
    let path = crate::paths::db_path();
    if !path.exists() {
        return;
    }
    let _ = crate::telemetry::record(&path, &event);
}

/// One remote tool request or poll. Wire requests are counted separately from the
/// logical invocation, so a retry or a recovery fetch is never hidden inside a
/// single row.
fn note_wire_request(id: &str, request: u32, status: u16, duration_ms: i64) {
    let event = crate::telemetry::TelemetryEvent::new(crate::telemetry::EventKind::ToolWireRequest)
        .tool(id)
        .provider(primary_provider(id).unwrap_or_default())
        // The executor never sees the prompt that asked, so the trigger is
        // unknown rather than inferred.
        .trigger(crate::telemetry::Trigger::Unknown)
        .outcome(if (200..400).contains(&status) {
            "http_ok"
        } else {
            "http_error"
        })
        .reason(format!("http_{status}"))
        .duration_ms(Some(duration_ms))
        .payload(json!({"request": request, "http_status": status}));
    note_telemetry(event);
}

/// Terminal outcome class for one typed SERP outcome, so the dashboard's
/// completion, block and mismatch groupings cover the named engines too.
fn serp_tool_outcome(outcome: search_engines::SerpOutcome) -> crate::telemetry::ToolOutcome {
    use crate::telemetry::ToolOutcome;
    use search_engines::SerpOutcome;
    match outcome {
        SerpOutcome::Valid => ToolOutcome::CompletedNonEmpty,
        SerpOutcome::VerifiedZero => ToolOutcome::CompletedVerifiedZero,
        SerpOutcome::Challenge | SerpOutcome::Consent => ToolOutcome::Blocked,
        SerpOutcome::RateLimited => ToolOutcome::Blocked,
        SerpOutcome::ParserMismatch => ToolOutcome::ParserMismatch,
        SerpOutcome::UpstreamFailure | SerpOutcome::ResponseTooLarge => ToolOutcome::Failed,
    }
}

/// One named-engine query with its typed outcome, usable result count, parser
/// version and cache-hit flag (spec §10).
fn note_engine_query(
    req: &search_engines::SerpRequest,
    observations: &Value,
    duration_ms: Option<i64>,
    cache_hit: bool,
) {
    let Some(outcome) = observations
        .get("outcome")
        .and_then(Value::as_str)
        .and_then(search_engines::SerpOutcome::parse)
    else {
        return;
    };
    let usable = observations
        .get("usable_results")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let event = crate::telemetry::TelemetryEvent::new(crate::telemetry::EventKind::ToolEngineQuery)
        .tool(&req.engine_tool)
        .provider("firecrawl")
        .engine(&req.engine)
        .trigger(crate::telemetry::Trigger::Unknown)
        .outcome(outcome.as_str())
        // The parser version is a rollup dimension, so a parser upgrade is visible
        // in the dashboard instead of silently rewriting history.
        .reason(search_engines::PARSER_VERSION)
        .duration_ms(duration_ms)
        .payload(json!({
            "parser_version": search_engines::PARSER_VERSION,
            "fetch_contract_version": search_engines::FETCH_CONTRACT_VERSION,
            "usable_results": usable,
            "cache_hit": cache_hit,
            "limit": req.limit,
            "tool_outcome": serp_tool_outcome(outcome).as_str(),
        }));
    note_telemetry(event);
}

fn url(base: &str, path: &[&str], query: &[(&str, &str)]) -> Result<Url> {
    let mut u = Url::parse(base)?;
    {
        let mut seg = u
            .path_segments_mut()
            .map_err(|_| anyhow!("invalid endpoint"))?;
        for p in path {
            seg.push(p);
        }
    }
    u.query_pairs_mut().extend_pairs(query.iter().copied());
    Ok(u)
}
#[derive(Clone, Debug)]
struct Request {
    url: Url,
    body: Option<Value>,
    form: Option<Vec<(String, String)>>,
    ndjson: bool,
    /// Status base for an async job (Firecrawl batch scrape and crawl): the POST returns
    /// an id and the executor polls `GET {poll}/{id}` until the job finishes.
    poll: Option<&'static str>,
}
fn profile_path_token(raw: &str) -> Result<String> {
    let raw = raw.trim();
    if raw.starts_with("http://") || raw.starts_with("https://") {
        let url = Url::parse(raw)?;
        let segment = url
            .path_segments()
            .into_iter()
            .flatten()
            .rfind(|segment| {
                !segment.is_empty()
                    && *segment != "channel"
                    && *segment != "user"
                    && *segment != "c"
            })
            .ok_or_else(|| anyhow!("profile URL has no handle"))?;
        return social_token(segment);
    }
    social_token(raw)
}
fn get(u: Url) -> Request {
    Request {
        url: u,
        body: None,
        form: None,
        ndjson: false,
        poll: None,
    }
}
fn request(id: &str, v: &Value) -> Result<Request> {
    let id = canonical_tool_id(id);
    if id.starts_with("firecrawl_") {
        return providers::firecrawl_request(id, v);
    }
    if id.starts_with("sociavault_") {
        return providers::sociavault_request(id, v);
    }
    if news_legal::provider(id).is_some() {
        return news_legal::request(id, v);
    }
    if atlas_news::is_atlas_tool(id) {
        return atlas_news::request(id, v);
    }
    if matches!(
        id,
        "hunter_domain_finder"
            | "hunter_email_count"
            | "hunter_company_enrichment"
            | "hunter_email_insight"
            | "hunter_person_enrichment"
            | "hunter_combined_enrichment"
    ) {
        return providers::hunter_request(id, v);
    }
    let q = |base, path: &[&str], query: &[(&str, &str)]| Ok(get(url(base, path, query)?));
    let domain_arg = || domain(str_arg(v, "domain")?);
    let ip_arg = || ip(str_arg(v, "ip")?);
    let cve = || {
        let s = str_arg(v, "cve_id")?.to_ascii_uppercase();
        ensure!(
            s.starts_with("CVE-")
                && s.len() <= 30
                && s[4..].split('-').count() == 2
                && s[4..].chars().all(|c| c.is_ascii_digit() || c == '-'),
            "invalid CVE ID"
        );
        Ok(s)
    };
    match id {
        "crtsh_certificates" => {
            let d = domain_arg()?;
            q("https://crt.sh/", &[], &[("q", &d), ("output", "json")])
        }
        "mnemonic_passive_dns" => {
            let s = str_arg(v, "domain_or_ip")?;
            let d = ip(s).or_else(|_| domain(s))?;
            let limit = number_arg(v, "limit", 25, 100)?;
            let offset = number_arg(v, "offset", 0, 1000)?;
            q(
                "https://api.mnemonic.no/pdns/v3",
                &[&d],
                &[("limit", &limit), ("offset", &offset)],
            )
        }
        "hackertarget_hostsearch" => {
            let d = domain_arg()?;
            q(
                "https://api.hackertarget.com/hostsearch/",
                &[],
                &[("q", &d)],
            )
        }
        "ripestat_network_info" => {
            let x = ip_arg()?;
            q(
                "https://stat.ripe.net/data/network-info/data.json",
                &[],
                &[("resource", &x)],
            )
        }
        "arin_rdap" => {
            let x = ip_arg()?;
            q("https://rdap.arin.net/registry/ip", &[&x], &[])
        }
        "apnic_rdap" => {
            let x = ip_arg()?;
            q("https://rdap.apnic.net/ip", &[&x], &[])
        }
        "wayback_availability" => {
            let x = url_arg(str_arg(v, "url")?)?;
            let t = v.get("timestamp").and_then(Value::as_str).unwrap_or("");
            q(
                "https://archive.org/wayback/available",
                &[],
                &[("url", &x), ("timestamp", t)],
            )
        }
        "commoncrawl_urls" => {
            let d = domain_arg()?;
            let index = str_arg(v, "index")?;
            ensure!(
                index.starts_with("CC-MAIN-")
                    && index
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-'),
                "invalid Common Crawl index"
            );
            let mut r = q(
                "https://index.commoncrawl.org",
                &[&format!("{index}-index")],
                &[("url", &format!("{d}/*")), ("output", "json")],
            )?;
            r.ndjson = true;
            Ok(r)
        }
        "arquivo_history" => {
            let x = str_arg(v, "domain_or_url")?;
            let x = if x.starts_with("http") {
                url_arg(x)?
            } else {
                domain(x)?
            };
            q(
                "https://arquivo.pt/textsearch",
                &[],
                &[("versionHistory", &x), ("maxItems", "30")],
            )
        }
        "github_repositories" => {
            let x = bounded(str_arg(v, "query")?)?;
            q(
                "https://api.github.com/search/repositories",
                &[],
                &[("q", &x), ("per_page", "30")],
            )
        }
        "gitlab_projects" => {
            let x = bounded(str_arg(v, "query")?)?;
            q(
                "https://gitlab.com/api/v4/projects",
                &[],
                &[("search", &x), ("visibility", "public"), ("per_page", "30")],
            )
        }
        "grepapp_code_search" => {
            let x = bounded(str_arg(v, "query")?)?;
            q("https://grep.app/api/search", &[], &[("q", &x)])
        }
        "gleif_entities" => {
            let (k, x) = one_of(v, &["lei", "company_name"])?;
            let x = bounded(x)?;
            if k == "lei" {
                ensure!(
                    x.len() == 20 && x.bytes().all(|b| b.is_ascii_alphanumeric()),
                    "invalid LEI"
                );
                q("https://api.gleif.org/api/v1/lei-records", &[&x], &[])
            } else {
                q(
                    "https://api.gleif.org/api/v1/lei-records",
                    &[],
                    &[("filter[entity.legalName]", &x), ("page[size]", "30")],
                )
            }
        }
        "sec_submissions" => {
            let cik = str_arg(v, "cik")
                .map(|s| s.to_string())
                .or_else(|_| one_of(v, &["name", "ticker"]).map(|(_, s)| s.to_string()))?;
            if cik.bytes().all(|b| b.is_ascii_digit()) {
                ensure!(cik.len() <= 10, "invalid CIK");
                q(
                    "https://data.sec.gov/submissions",
                    &[&format!("CIK{:0>10}.json", cik)],
                    &[],
                )
            } else {
                q("https://www.sec.gov/files", &["company_tickers.json"], &[])
            }
        }
        "wikidata_entities" => {
            let (k, x) = one_of(v, &["qid", "name"])?;
            let x = bounded(x)?;
            if k == "qid" {
                ensure!(
                    x.starts_with('Q') && x[1..].bytes().all(|b| b.is_ascii_digit()),
                    "invalid QID"
                );
                q(
                    "https://www.wikidata.org/wiki/Special:EntityData",
                    &[&format!("{x}.json")],
                    &[],
                )
            } else {
                q(
                    "https://www.wikidata.org/w/api.php",
                    &[],
                    &[
                        ("action", "wbsearchentities"),
                        ("search", &x),
                        ("language", "en"),
                        ("format", "json"),
                    ],
                )
            }
        }
        "keybase_identity" => {
            let (k, x) = one_of(v, &["username", "domain"])?;
            let x = if k == "domain" {
                domain(x)?
            } else {
                bounded(x)?
            };
            q(
                "https://keybase.io/_/api/1.0/user/lookup.json",
                &[],
                &[(if k == "domain" { "domain" } else { "usernames" }, &x)],
            )
        }
        "stackexchange_users" => {
            let x = bounded(str_arg(v, "name")?)?;
            let site = v
                .get("site")
                .and_then(Value::as_str)
                .unwrap_or("stackoverflow");
            ensure!(
                site.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-'),
                "invalid site"
            );
            q(
                "https://api.stackexchange.com/2.3/users",
                &[],
                &[("site", site), ("inname", &x), ("pagesize", "30")],
            )
        }
        "wikipedia_users" => {
            let x = bounded(str_arg(v, "username")?)?;
            q(
                "https://en.wikipedia.org/w/api.php",
                &[],
                &[
                    ("action", "query"),
                    ("list", "users"),
                    ("ususers", &x),
                    ("usprop", "registration|editcount|groups"),
                    ("format", "json"),
                ],
            )
        }
        "wikipedia_source_reliability" => {
            let (domain, publisher) = wikipedia_rsp::resolve_lookup_inputs(v)?;
            let page = if !publisher.is_empty() {
                "Wikipedia:Reliable_sources/Perennial_sources"
            } else {
                // Letter page is selected at runtime when the index is built; the
                // request URL documents the MediaWiki parse endpoint for fixtures.
                "Wikipedia:Reliable_sources/Perennial_sources/1"
            };
            let _ = domain;
            q(
                "https://en.wikipedia.org/w/api.php",
                &[],
                &[
                    ("action", "parse"),
                    ("page", page),
                    ("prop", "wikitext"),
                    ("formatversion", "2"),
                    ("format", "json"),
                ],
            )
        }
        "nominatim_geocode" => {
            let x = bounded(str_arg(v, "address_or_place")?)?;
            q(
                "https://nominatim.openstreetmap.org/search",
                &[],
                &[("q", &x), ("format", "jsonv2"), ("limit", "5")],
            )
        }
        "census_geocode" => {
            let x = bounded(str_arg(v, "us_address")?)?;
            q(
                "https://geocoding.geo.census.gov/geocoder/locations/onelineaddress",
                &[],
                &[
                    ("address", &x),
                    ("benchmark", "Public_AR_Current"),
                    ("format", "json"),
                ],
            )
        }
        "overpass_places" => {
            let lat = v
                .get("latitude")
                .and_then(Value::as_f64)
                .ok_or_else(|| anyhow!("latitude required"))?;
            let lon = v
                .get("longitude")
                .and_then(Value::as_f64)
                .ok_or_else(|| anyhow!("longitude required"))?;
            let radius = v.get("radius_m").and_then(Value::as_u64).unwrap_or(1000);
            ensure!(
                (-90.0..=90.0).contains(&lat)
                    && (-180.0..=180.0).contains(&lon)
                    && (50..=3000).contains(&radius),
                "invalid coordinates or radius"
            );
            let feature = v
                .get("feature")
                .and_then(Value::as_str)
                .unwrap_or("amenity");
            ensure!(
                ["amenity", "shop", "tourism", "healthcare", "railway"].contains(&feature),
                "unsupported feature"
            );
            let data = format!(
                "[out:json][timeout:25];nwr(around:{radius},{lat},{lon})[{feature}];out center 50;"
            );
            Ok(Request {
                url: Url::parse("https://overpass-api.de/api/interpreter")?,
                body: None,
                form: Some(vec![("data".into(), data)]),
                ndjson: false,
                poll: None,
            })
        }
        "blockchain_address" | "blockstream_address" | "mempool_address" => {
            let x = bitcoin(str_arg(v, "bitcoin_address")?)?;
            match id {
                "blockchain_address" => {
                    q("https://blockchain.info/balance", &[], &[("active", &x)])
                }
                "blockstream_address" => q("https://blockstream.info/api/address", &[&x], &[]),
                _ => q("https://mempool.space/api/address", &[&x], &[]),
            }
        }
        "nvd_cve" => {
            let x = cve()?;
            q(
                "https://services.nvd.nist.gov/rest/json/cves/2.0",
                &[],
                &[("cveId", &x)],
            )
        }
        "osv_package" => {
            let eco = bounded(str_arg(v, "ecosystem")?)?;
            let pkg = bounded(str_arg(v, "package_name")?)?;
            let version = v.get("version").and_then(Value::as_str);
            let commit = v.get("commit").and_then(Value::as_str);
            ensure!(
                version.is_some() ^ commit.is_some(),
                "provide version or commit"
            );
            if eco == "Maven" {
                ensure!(
                    pkg.contains(':') && pkg.split(':').all(|s| !s.is_empty()),
                    "Maven package must be group:artifact"
                );
            }
            let body = if let Some(c) = commit {
                json!({"commit":bounded(c)?})
            } else {
                json!({"package":{"name":pkg,"ecosystem":eco},"version":bounded(version.unwrap())?})
            };
            Ok(Request {
                url: Url::parse("https://api.osv.dev/v1/query")?,
                body: Some(body),
                form: None,
                ndjson: false,
                poll: None,
            })
        }
        "cve_record" => {
            let x = cve()?;
            q("https://cveawg.mitre.org/api/cve", &[&x], &[])
        }
        "sans_ip_activity" => {
            let x = ip_arg()?;
            q("https://isc.sans.edu/api/ip", &[&x], &[("json", "")])
        }
        "shodan_internetdb" => {
            let x = ip_arg()?;
            q("https://internetdb.shodan.io", &[&x], &[])
        }
        "hunter_domain_search" => {
            let domain_value = str_arg(v, "domain").ok().map(domain).transpose()?;
            let company_value = str_arg(v, "company").ok().map(bounded).transpose()?;
            ensure!(
                domain_value.is_some() || company_value.is_some(),
                "provide domain or company"
            );
            let limit = number_arg(v, "limit", 10, 10)?;
            let mut pairs = vec![("limit", limit.as_str())];
            if let Some(domain_value) = domain_value.as_deref() {
                pairs.push(("domain", domain_value));
            } else if let Some(company_value) = company_value.as_deref() {
                pairs.push(("company", company_value));
            }
            q("https://api.hunter.io/v2/domain-search", &[], &pairs)
        }
        "hunter_email_finder" => {
            let domain_value = str_arg(v, "domain").ok().map(domain).transpose()?;
            let company_value = str_arg(v, "company").ok().map(bounded).transpose()?;
            let linkedin = str_arg(v, "linkedin_handle")
                .ok()
                .map(linkedin_handle)
                .transpose()?;
            ensure!(
                domain_value.is_some() || company_value.is_some() || linkedin.is_some(),
                "provide domain, company, or linkedin_handle"
            );
            let full_name = str_arg(v, "full_name").ok().map(bounded).transpose()?;
            let first_name = str_arg(v, "first_name").ok().map(bounded).transpose()?;
            let last_name = str_arg(v, "last_name").ok().map(bounded).transpose()?;
            if linkedin.is_none() {
                ensure!(
                    full_name.is_some() || (first_name.is_some() && last_name.is_some()),
                    "provide full_name or first_name and last_name"
                );
            }
            let mut pairs = Vec::new();
            if let Some(domain_value) = domain_value.as_deref() {
                pairs.push(("domain", domain_value));
            } else if let Some(company_value) = company_value.as_deref() {
                pairs.push(("company", company_value));
            }
            if let Some(linkedin) = linkedin.as_deref() {
                pairs.push(("linkedin_handle", linkedin));
            }
            if let Some(full_name) = full_name.as_deref() {
                pairs.push(("full_name", full_name));
            } else {
                if let Some(first_name) = first_name.as_deref() {
                    pairs.push(("first_name", first_name));
                }
                if let Some(last_name) = last_name.as_deref() {
                    pairs.push(("last_name", last_name));
                }
            }
            q("https://api.hunter.io/v2/email-finder", &[], &pairs)
        }
        "hunter_email_verifier" => {
            let email = email_address(str_arg(v, "email")?)?;
            q(
                "https://api.hunter.io/v2/email-verifier",
                &[],
                &[("email", &email)],
            )
        }
        "urlscan_search" => {
            let x = if let Ok(d) = domain_arg() {
                format!("domain:{d}")
            } else {
                let s = str_arg(v, "query")?;
                ensure!(
                    s.len() <= 120
                        && s.split_whitespace().all(|term| term.starts_with("domain:")
                            || term.starts_with("page.domain:")
                            || term.starts_with("ip:")
                            || term == "AND"),
                    "unsupported search expression"
                );
                s.to_string()
            };
            q(
                "https://urlscan.io/api/v1/search/",
                &[],
                &[("q", &x), ("size", "30")],
            )
        }
        "whatsmyname_lookup" => {
            let u = str_arg(v, "username")?;
            q(whatsmyname::WMN_UPSTREAM_URL, &[], &[("u", u)])
        }
        "whoxy_whois_history" => {
            let domain = whoxy::parse_query(v)?.domain;
            Ok(get(whoxy::history_request_url(&domain)?))
        }
        "holehe_email_lookup" => {
            holehe::normalize_email(str_arg(v, "email")?)?;
            q(
                "https://api.twitter.com/i/users/email_available.json",
                &[],
                &[],
            )
        }
        "dork_generate" => q("https://dorksearch.pro/", &[], &[]),
        _ => Err(anyhow!("unknown tool {id}")),
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolResult {
    pub tool_id: String,
    pub inputs: Value,
    pub status: String,
    pub source_url: String,
    pub retrieved_at: String,
    pub observations: Value,
    pub raw: String,
    pub error: Option<String>,
    pub cached: bool,
    pub truncated: bool,
    /// Credits reserved for this call. Zero when the result was reused from cache.
    #[serde(default)]
    pub credits_charged: u32,
    /// Provider-reported usage when the response included it.
    #[serde(default)]
    pub credits_reported: Option<u32>,
}
type HostSchedule = Arc<Mutex<HashMap<String, std::time::Instant>>>;
type SharedHttp = (reqwest::Client, Arc<Semaphore>, HostSchedule);
#[derive(Clone, Debug, Default)]
pub struct ProviderKeys {
    pub firecrawl: String,
    pub firecrawl_fallback: String,
    pub hunter: String,
    pub hunter_fallback: String,
    pub sociavault: String,
    pub sociavault_fallback: String,
    pub newsapi: String,
    pub newsapi_fallback: String,
    pub courtlistener: String,
    pub courtlistener_fallback: String,
    pub gnews: String,
    pub gnews_fallback: String,
    pub newsdata: String,
    pub newsdata_fallback: String,
    pub currents: String,
    pub currents_fallback: String,
    pub whoxy: String,
    pub whoxy_fallback: String,
}

impl ProviderKeys {
    /// Primary key and second-account key for a keyed provider.
    pub fn pair(&self, provider: &str) -> (&str, &str) {
        match provider {
            "firecrawl" => (&self.firecrawl, &self.firecrawl_fallback),
            "hunter" => (&self.hunter, &self.hunter_fallback),
            "sociavault" => (&self.sociavault, &self.sociavault_fallback),
            "newsapi" => (&self.newsapi, &self.newsapi_fallback),
            "courtlistener" => (&self.courtlistener, &self.courtlistener_fallback),
            "gnews" => (&self.gnews, &self.gnews_fallback),
            "newsdata" => (&self.newsdata, &self.newsdata_fallback),
            "currents" => (&self.currents, &self.currents_fallback),
            "whoxy" => (&self.whoxy, &self.whoxy_fallback),
            _ => ("", ""),
        }
    }
}

fn exhausted_keys() -> &'static std::sync::Mutex<HashSet<String>> {
    static KEYS: OnceLock<std::sync::Mutex<HashSet<String>>> = OnceLock::new();
    KEYS.get_or_init(|| std::sync::Mutex::new(HashSet::new()))
}

/// Remember that this key hit a rate or quota limit, for the rest of the process.
pub fn note_key_exhausted(key: &str) {
    let key = key.trim();
    if key.is_empty() {
        return;
    }
    if let Ok(mut keys) = exhausted_keys().lock() {
        keys.insert(key.to_string());
    }
}

/// True after [`note_key_exhausted`] for this exact key.
pub fn key_exhausted(key: &str) -> bool {
    exhausted_keys()
        .lock()
        .map(|keys| keys.contains(key.trim()))
        .unwrap_or(false)
}

/// The key to send: the fallback once the primary is empty or exhausted.
fn select_key<'a>(primary: &'a str, fallback: &'a str) -> &'a str {
    let primary = primary.trim();
    let fallback = fallback.trim();
    if !fallback.is_empty() && (primary.is_empty() || key_exhausted(primary)) {
        fallback
    } else {
        primary
    }
}

/// A provider response that means the account's rate limit or quota is spent.
pub fn quota_limited(status: &str, error: Option<&str>) -> bool {
    if status == "rate_limited" {
        return true;
    }
    let Some(error) = error else {
        return false;
    };
    let lower = error.to_ascii_lowercase();
    lower.contains("rate limit")
        || lower.contains("ratelimited")
        || lower.contains("quota")
        || lower.contains("exhausted")
        || lower.contains("too many requests")
        || lower.contains("insufficient credit")
        || lower.contains("payment required")
        || lower.contains("http 402")
        || lower.contains("http 429")
        || lower.contains("apikeyexhausted")
        || lower.contains("apikeydisabled")
}

fn keyed_provider(id: &str) -> Option<&'static str> {
    let id = canonical_tool_id(id);
    primary_provider(id)
        .or_else(|| news_legal::provider(id))
        .or_else(|| atlas_news::provider(id))
        .or_else(|| (id == whoxy::TOOL_ID).then_some("whoxy"))
}

/// Atlas pipeline tools are manual and Atlas-only. Recon's picker never offers them.
pub fn atlas_pipeline_tool(id: &str) -> bool {
    atlas_news::is_atlas_tool(id)
}
fn provider_credential(
    id: &str,
    keys: &ProviderKeys,
) -> Result<Option<(reqwest::header::HeaderName, String)>> {
    let Some(provider) = keyed_provider(id) else {
        return Ok(None);
    };
    let (primary, fallback) = keys.pair(provider);
    header_for(id, select_key(primary, fallback))
}

/// Header for an explicit key. NewsData's value is the raw key; the caller copies it
/// into the `apikey` query and does not send the header.
fn header_for(id: &str, raw: &str) -> Result<Option<(reqwest::header::HeaderName, String)>> {
    let keyed = |missing: &str| -> Result<String> {
        let key = raw.trim();
        ensure!(
            !key.is_empty() && key.len() <= 400 && !key.chars().any(char::is_control),
            "{missing}"
        );
        Ok(key.to_string())
    };
    let id = canonical_tool_id(id);
    if id.starts_with("firecrawl_") {
        let key =
            keyed("Enter the Firecrawl API key on a Firecrawl tool, or set FIRECRAWL_API_KEY")?;
        return Ok(Some((
            reqwest::header::AUTHORIZATION,
            format!("Bearer {key}"),
        )));
    }
    if id.starts_with("hunter_") {
        let key = keyed("Enter the Hunter API key on a Hunter tool, or set HUNTER_API_KEY")?;
        return Ok(Some((
            reqwest::header::HeaderName::from_static("x-api-key"),
            key,
        )));
    }
    match atlas_news::provider(id) {
        Some("gnews") => {
            let key = keyed("Enter the GNews API key on a GNews tool, or set GNEWS_API_KEY")?;
            return Ok(Some((
                reqwest::header::HeaderName::from_static("x-api-key"),
                key,
            )));
        }
        Some("newsdata") => {
            let key =
                keyed("Enter the NewsData API key on a NewsData tool, or set NEWSDATA_API_KEY")?;
            return Ok(Some((
                reqwest::header::HeaderName::from_static("x-api-key"),
                key,
            )));
        }
        Some("currents") => {
            let key =
                keyed("Enter the Currents API key on a Currents tool, or set CURRENTS_API_KEY")?;
            return Ok(Some((
                reqwest::header::AUTHORIZATION,
                format!("Bearer {key}"),
            )));
        }
        _ => {}
    }
    match news_legal::provider(id) {
        Some("newsapi") => {
            let key = keyed("Enter the NewsAPI key on a News tool, or set NEWSAPI_API_KEY")?;
            return Ok(Some((
                reqwest::header::HeaderName::from_static("x-api-key"),
                key,
            )));
        }
        Some(_) => {
            let key = keyed(
                "Enter the CourtListener API token on a Legal tool, or set COURTLISTENER_API_TOKEN",
            )?;
            return Ok(Some((
                reqwest::header::AUTHORIZATION,
                format!("Token {key}"),
            )));
        }
        None => {}
    }
    if id.starts_with("sociavault_") {
        let key =
            keyed("Enter the SociaVault API key on a SociaVault tool, or set SOCIAVAULT_API_KEY")?;
        return Ok(Some((
            reqwest::header::HeaderName::from_static("x-api-key"),
            key,
        )));
    }
    if id == whoxy::TOOL_ID {
        let key = keyed("Enter the Whoxy API key on a Whoxy tool, or set WHOXY_API_KEY")?;
        return Ok(Some((
            reqwest::header::HeaderName::from_static("x-api-key"),
            key,
        )));
    }
    Ok(None)
}

/// URL and headers for one account. NewsData carries the key in `apikey`, not a header.
fn bind_request(
    id: &str,
    req_url: &Url,
    key: &str,
) -> Result<(
    Url,
    Option<(reqwest::header::HeaderName, String)>,
    Option<(reqwest::header::HeaderName, String)>,
)> {
    let credential = header_for(id, key)?;
    let mut url = req_url.clone();
    #[cfg(test)]
    if let Some(base) = test_base(req_url.host_str().unwrap_or("")) {
        url = rebase(&url, &base)?;
    }
    let mut send = credential.clone();
    if atlas_news::provider(id) == Some("newsdata") {
        if let Some((_, value)) = &credential {
            url.query_pairs_mut().append_pair("apikey", value);
        }
        send = None;
    }
    if canonical_tool_id(id) == whoxy::TOOL_ID {
        if let Some((_, value)) = &credential {
            url.query_pairs_mut().append_pair("key", value);
        }
        send = None;
    }
    Ok((url, credential, send))
}
/// A short reason from an error body. An HTML error page (a CDN block page such as
/// Stack Exchange's "This IP address ... has been blocked") becomes its title and the
/// sentence that says why, instead of the first 250 bytes of markup.
fn error_summary(raw: &str) -> String {
    let trimmed = raw.trim_start();
    if !(trimmed.starts_with('<') && trimmed.to_ascii_lowercase().contains("<html")) {
        return raw.chars().take(250).collect();
    }
    let lower = raw.to_ascii_lowercase();
    let title = lower
        .find("<title>")
        .and_then(|start| {
            lower[start..]
                .find("</title>")
                .map(|end| raw[start + 7..start + end].trim().to_string())
        })
        .unwrap_or_default();
    let mut text = String::new();
    let mut in_tag = false;
    let mut skip_until: Option<&str> = None;
    let mut index = 0;
    while index < raw.len() {
        if let Some(close) = skip_until {
            match lower[index..].find(close) {
                Some(at) => index += at + close.len(),
                None => break,
            }
            skip_until = None;
            continue;
        }
        let rest = &lower[index..];
        if rest.starts_with("<style") {
            skip_until = Some("</style>");
            continue;
        }
        if rest.starts_with("<script") {
            skip_until = Some("</script>");
            continue;
        }
        let ch = raw[index..].chars().next().unwrap_or(' ');
        match ch {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                text.push(' ');
            }
            _ if !in_tag => text.push(ch),
            _ => {}
        }
        index += ch.len_utf8();
    }
    let words = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let body = words.strip_prefix(title.as_str()).unwrap_or(&words).trim();
    // Sentences end at ". " so dotted values (an IP address) stay whole.
    let reason = body
        .split(". ")
        .find(|sentence| {
            let sentence = sentence.to_ascii_lowercase();
            [
                "blocked",
                "denied",
                "forbidden",
                "not allowed",
                "rate limit",
            ]
            .iter()
            .any(|word| sentence.contains(word))
        })
        .map(|sentence| sentence.trim().trim_end_matches('.'))
        .unwrap_or("");
    let summary = match (title.is_empty(), reason.is_empty()) {
        (false, false) => format!("{title}: {reason}"),
        (false, true) => title,
        (true, false) => reason.to_string(),
        (true, true) => words,
    };
    summary.chars().take(250).collect()
}

/// Minimum spacing between requests to one host for a tool.
pub fn host_interval(id: &str) -> Duration {
    let id = canonical_tool_id(id);
    match id {
        "nominatim_geocode" | "urlscan_search" => Duration::from_secs(1),
        _ if news_legal::provider(id) == Some("courtlistener") => news_legal::COURTLISTENER_SPACING,
        _ if news_legal::provider(id) == Some("newsapi") || atlas_news::provider(id).is_some() => {
            Duration::from_secs(1)
        }
        // Hunter allows 15 requests per second; Firecrawl and SociaVault keep 1/s.
        _ if id.starts_with("hunter_") => Duration::from_millis(67),
        _ if id.starts_with("firecrawl_") || id.starts_with("sociavault_") => {
            Duration::from_secs(1)
        }
        "hackertarget_hostsearch" | "overpass_places" => Duration::from_secs(2),
        "github_repositories" | "nvd_cve" => Duration::from_secs(6),
        _ => Duration::from_millis(250),
    }
}

/// The bare key a credential header carries (`Bearer …` and `Token …` stripped).
fn credential_key(credential: &Option<(reqwest::header::HeaderName, String)>) -> Option<&str> {
    let (_, value) = credential.as_ref()?;
    let key = value
        .strip_prefix("Bearer ")
        .or_else(|| value.strip_prefix("Token "))
        .unwrap_or(value)
        .trim();
    (key.len() >= 4).then_some(key)
}

/// A provider body that echoes the key never reaches storage with it.
fn redact(raw: String, credential: &Option<(reqwest::header::HeaderName, String)>) -> String {
    match credential_key(credential) {
        Some(key) if raw.contains(key) => raw.replace(key, "[redacted]"),
        _ => raw,
    }
}

/// Stored source URLs never keep an API key query parameter.
fn public_source_url(url: &Url) -> String {
    let mut clean = url.clone();
    let pairs: Vec<(String, String)> = clean
        .query_pairs()
        .filter(|(key, _)| key != "apikey" && key != "apiKey" && key != "key")
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    clean.set_query(None);
    if !pairs.is_empty() {
        clean.query_pairs_mut().extend_pairs(pairs);
    }
    clean.to_string()
}

fn redact_key(result: &mut ToolResult, credential: &Option<(reqwest::header::HeaderName, String)>) {
    result.raw = redact(std::mem::take(&mut result.raw), credential);
    if let Some(error) = result.error.take() {
        result.error = Some(redact(error, credential));
    }
}

/// Test-only base URLs for fixed hosts, so executor tests can reach a local server.
#[cfg(test)]
pub(crate) static TEST_BASES: std::sync::Mutex<Vec<(String, String)>> =
    std::sync::Mutex::new(Vec::new());

#[cfg(test)]
fn test_base(host: &str) -> Option<String> {
    TEST_BASES
        .lock()
        .unwrap()
        .iter()
        .find(|(known, _)| known == host)
        .map(|(_, base)| base.clone())
}

#[cfg(test)]
fn rebase(url: &Url, base: &str) -> Result<Url> {
    let base = Url::parse(base)?;
    let mut next = url.clone();
    next.set_scheme(base.scheme())
        .map_err(|_| anyhow!("scheme"))?;
    next.set_host(base.host_str())
        .map_err(|_| anyhow!("host"))?;
    next.set_port(base.port()).map_err(|_| anyhow!("port"))?;
    Ok(next)
}

/// The User-Agent every OSINT request sends when `osint_user_agent` is unset or blank.
pub const DEFAULT_USER_AGENT: &str =
    "Argos OSINT/0.1 (public research; contact: configure osint_user_agent)";

/// The configured `osint_user_agent`, or None when it is unset, empty, or whitespace. The
/// single place a blank setting is treated as unset, so it never overrides the default.
pub fn custom_user_agent(setting: Option<&str>) -> Option<&str> {
    setting.map(str::trim).filter(|agent| !agent.is_empty())
}

/// The User-Agent a request sends: a non-blank custom value, else [`DEFAULT_USER_AGENT`].
/// Never empty.
pub fn effective_user_agent(setting: Option<&str>) -> &str {
    custom_user_agent(setting).unwrap_or(DEFAULT_USER_AGENT)
}

/// Headers every tool request carries besides its credential: the User-Agent (never
/// empty) and, for CourtListener, `Accept: application/json`.
fn request_headers(
    id: &str,
    user_agent: Option<&str>,
) -> Vec<(reqwest::header::HeaderName, String)> {
    let mut headers = vec![(
        reqwest::header::USER_AGENT,
        effective_user_agent(user_agent).to_string(),
    )];
    if news_legal::provider(id) == Some("courtlistener") {
        headers.push((reqwest::header::ACCEPT, "application/json".to_string()));
    }
    headers
}

#[derive(Clone)]
pub struct Executor {
    client: reqwest::Client,
    global: Arc<Semaphore>,
    hosts: HostSchedule,
}
impl Executor {
    pub fn new() -> Result<Self> {
        static SHARED: OnceLock<SharedHttp> = OnceLock::new();
        let shared = SHARED.get_or_init(|| {
            let client = reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .user_agent(DEFAULT_USER_AGENT)
                .build()
                .expect("HTTP client");
            (
                client,
                Arc::new(Semaphore::new(4)),
                Arc::new(Mutex::new(HashMap::new())),
            )
        });
        Ok(Self {
            client: shared.0.clone(),
            global: shared.1.clone(),
            hosts: shared.2.clone(),
        })
    }
    async fn pace_host(&self, host: &str, interval: Duration) {
        let delay = {
            let mut hosts = self.hosts.lock().await;
            let now = std::time::Instant::now();
            let ready = hosts
                .get(host)
                .map(|last| (*last + interval).max(now))
                .unwrap_or(now);
            hosts.insert(host.to_string(), ready);
            ready.saturating_duration_since(now)
        };
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
    }
    async fn metadata_json(&self, endpoint: &str, user_agent: Option<&str>) -> Result<Value> {
        let url = Url::parse(endpoint)?;
        let host = url
            .host_str()
            .ok_or_else(|| anyhow!("metadata host missing"))?;
        ensure!(
            ["www.sec.gov", "index.commoncrawl.org"].contains(&host),
            "metadata host is not allowed"
        );
        let _permit = self.global.acquire().await?;
        self.pace_host(host, Duration::from_secs(1)).await;
        let request = self
            .client
            .get(url)
            .timeout(Duration::from_secs(20))
            .header(
                reqwest::header::USER_AGENT,
                effective_user_agent(user_agent),
            );
        let response = request.send().await?.error_for_status()?;
        let mut stream = response.bytes_stream();
        let mut bytes = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            ensure!(
                bytes.len() + chunk.len() <= 4_000_000,
                "metadata response too large"
            );
            bytes.extend_from_slice(&chunk);
        }
        Ok(serde_json::from_slice(&bytes)?)
    }
    pub async fn run(
        &self,
        id: &str,
        inputs: Value,
        user_agent: Option<&str>,
    ) -> Result<ToolResult> {
        self.run_configured(id, inputs, user_agent, &ProviderKeys::default())
            .await
    }
    pub async fn run_configured(
        &self,
        id: &str,
        mut inputs: Value,
        user_agent: Option<&str>,
        keys: &ProviderKeys,
    ) -> Result<ToolResult> {
        let id = canonical_tool_id(id);
        let def = definition(id).ok_or_else(|| anyhow!("unknown tool {id}"))?;
        // A blank osint_user_agent is unset: requests fall back to DEFAULT_USER_AGENT.
        let user_agent = custom_user_agent(user_agent);
        if [
            "nominatim_geocode",
            "sec_submissions",
            wikipedia_rsp::TOOL_ID,
        ]
        .contains(&id)
        {
            ensure!(
                user_agent.is_some_and(|s| s.contains('@') || s.contains("http")),
                "configure identifying osint_user_agent for {id}"
            );
        }
        if id == wikipedia_rsp::TOOL_ID {
            let (domain, publisher) = wikipedia_rsp::resolve_lookup_inputs(&inputs)?;
            let agent = effective_user_agent(user_agent);
            let index = wikipedia_rsp::ensure_index(agent).await?;
            let observation = wikipedia_rsp::observation_for(&index, &domain, &publisher);
            let status = if observation.listed {
                "completed"
            } else {
                "completed"
            };
            return Ok(ToolResult {
                tool_id: id.into(),
                inputs,
                status: status.into(),
                source_url: observation.rsp_url.clone(),
                retrieved_at: Utc::now().to_rfc3339(),
                observations: serde_json::to_value(&observation)?,
                raw: String::new(),
                error: None,
                cached: false,
                truncated: false,
                credits_charged: 0,
                credits_reported: None,
            });
        }
        if id == holehe::TOOL_ID {
            let lookup_input: holehe::LookupInput = serde_json::from_value(inputs.clone())?;
            let client = reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(holehe::REQUEST_TIMEOUT)
                .build()?;
            let cancel = std::sync::atomic::AtomicBool::new(false);
            let observation = holehe::lookup(&client, lookup_input, user_agent, &cancel).await?;
            let status = holehe::observation_status(&observation);
            return Ok(ToolResult {
                tool_id: id.into(),
                inputs,
                status: status.into(),
                source_url: format!(
                    "https://github.com/megadose/holehe/tree/{}",
                    holehe::catalog::UPSTREAM_COMMIT
                ),
                retrieved_at: Utc::now().to_rfc3339(),
                observations: serde_json::to_value(&observation)?,
                raw: String::new(),
                error: None,
                cached: false,
                truncated: false,
                credits_charged: 0,
                credits_reported: None,
            });
        }
        if id == "whatsmyname_lookup" {
            let lookup_input: whatsmyname::LookupInput = serde_json::from_value(inputs.clone())?;
            let client = reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(whatsmyname::REQUEST_TIMEOUT)
                .build()?;
            let observation = whatsmyname::lookup(&client, lookup_input, user_agent).await?;
            let status = if observation.coverage.found > 0 {
                "completed"
            } else if observation.coverage.completed > 0 {
                "no_results"
            } else if observation.coverage.error > 0 || observation.coverage.timeout > 0 {
                "failed"
            } else {
                "completed"
            };
            return Ok(ToolResult {
                tool_id: id.into(),
                inputs,
                status: status.into(),
                source_url: whatsmyname::WMN_UPSTREAM_URL.into(),
                retrieved_at: Utc::now().to_rfc3339(),
                observations: serde_json::to_value(&observation)?,
                raw: String::new(),
                error: None,
                cached: false,
                truncated: false,
                credits_charged: 0,
                credits_reported: None,
            });
        }
        if id == "dork_generate" {
            let gen_input: dork_generator::GenerateInput = serde_json::from_value(inputs.clone())?;
            let artifacts = dork_generator::generate(gen_input)?;
            return Ok(ToolResult {
                tool_id: id.into(),
                inputs,
                status: "completed".into(),
                source_url: "https://dorksearch.pro/".into(),
                retrieved_at: Utc::now().to_rfc3339(),
                observations: serde_json::to_value(&artifacts)?,
                raw: String::new(),
                error: None,
                cached: false,
                truncated: false,
                credits_charged: 0,
                credits_reported: None,
            });
        }
        if id == "sec_submissions" && inputs.get("cik").is_none() {
            let (kind, needle) = one_of(&inputs, &["ticker", "name"])?;
            let mapping = self
                .metadata_json("https://www.sec.gov/files/company_tickers.json", user_agent)
                .await?;
            let matches: Vec<Value> = mapping
                .as_object()
                .into_iter()
                .flat_map(|o| o.values())
                .filter(|row| {
                    let field = if kind == "ticker" { "ticker" } else { "title" };
                    row.get(field).and_then(Value::as_str).is_some_and(|s| {
                        if kind == "ticker" {
                            s.eq_ignore_ascii_case(needle)
                        } else {
                            s.to_ascii_lowercase()
                                .contains(&needle.to_ascii_lowercase())
                        }
                    })
                })
                .take(30)
                .cloned()
                .collect();
            if matches.len() != 1 {
                return Ok(ToolResult {
                    tool_id: id.into(),
                    inputs,
                    status: if matches.is_empty() {
                        "no_results"
                    } else {
                        "completed"
                    }
                    .into(),
                    source_url: "https://www.sec.gov/files/company_tickers.json".into(),
                    retrieved_at: Utc::now().to_rfc3339(),
                    observations: json!({"candidate_matches":matches,"ambiguous":matches.len()>1}),
                    raw: String::new(),
                    error: None,
                    cached: false,
                    truncated: matches.len() == 30,
                    credits_charged: 0,
                    credits_reported: None,
                });
            }
            let cik = matches[0]
                .get("cik_str")
                .and_then(Value::as_u64)
                .ok_or_else(|| anyhow!("SEC mapping lacks CIK"))?;
            inputs["cik"] = json!(cik.to_string());
        }
        if id == "commoncrawl_urls" && inputs.get("index").is_none() {
            let catalogs = self
                .metadata_json("https://index.commoncrawl.org/collinfo.json", user_agent)
                .await?;
            let index = catalogs
                .as_array()
                .and_then(|rows| rows.first())
                .and_then(|row| row.get("id"))
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("Common Crawl index discovery returned no index"))?;
            inputs["index"] = json!(index);
        }
        let (primary_key, fallback_key) = match keyed_provider(id) {
            Some(provider) => {
                let (primary, fallback) = keys.pair(provider);
                (primary.trim().to_string(), fallback.trim().to_string())
            }
            None => (String::new(), String::new()),
        };
        // A keyed tool with neither account fails here, before any provider request.
        provider_credential(id, keys)?;
        // Original request context for a named SERP query, read once from the
        // original tool input before any redirect (spec §9 fix 2).
        let serp = if search_engines::is_named_engine_tool(id) {
            Some(serp_request_for(id, &inputs)?)
        } else {
            None
        };
        let req = request(id, &inputs)?;
        let host = req.url.host_str().unwrap_or("").to_string();
        if let Some(locked) = providers::locked_host(id) {
            ensure!(
                host == locked && req.url.scheme() == "https",
                "request host is not allowed for {id}"
            );
        }
        let _permit = self.global.acquire().await?;
        let interval = host_interval(id);
        #[cfg(test)]
        let (url_override, interval) = match test_base(&host) {
            Some(base) => (Some(base), Duration::ZERO),
            None => (None, interval),
        };
        self.pace_host(&host, interval).await;
        let chosen = select_key(&primary_key, &fallback_key);
        let mut using_primary = !primary_key.is_empty() && chosen == primary_key;
        let mut switched = false;
        let (mut url, mut credential, mut send_credential) = if keyed_provider(id).is_some() {
            bind_request(id, &req.url, chosen)?
        } else {
            #[cfg(test)]
            let url = if let Some(base) = url_override.as_ref() {
                rebase(&req.url, base)?
            } else {
                req.url.clone()
            };
            #[cfg(not(test))]
            let url = req.url.clone();
            (url, None, None)
        };
        let mut attempts = 0u32;
        let mut redirects = 0;
        let mut verifying = 0;
        // One fresh same-engine SERP retry is counted here when the parser asks for it.
        let mut serp_recovered = false;
        let headers = request_headers(id, user_agent);
        // NewsAPI and CourtListener 429s are not retried: their daily quotas are tiny.
        let retry_429 = news_legal::provider(id).is_none() && atlas_news::provider(id).is_none();
        // A named SERP engine reads a larger body than every other tool.
        let cap = body_cap(id);
        loop {
            attempts += 1;
            let sent_at = crate::telemetry::Measured::start();
            let mut builder = if let Some(body) = &req.body {
                self.client.post(url.clone()).json(body)
            } else if let Some(form) = &req.form {
                self.client.post(url.clone()).form(form)
            } else {
                self.client.get(url.clone())
            };
            for (name, value) in &headers {
                builder = builder.header(name, value);
            }
            builder = builder.timeout(Duration::from_secs(def.timeout_seconds));
            if let Some((name, value)) = &send_credential {
                builder = builder.header(name, value);
            }
            let response =
                tokio::time::timeout(Duration::from_secs(def.timeout_seconds), builder.send())
                    .await??;
            if response.status().is_redirection() {
                redirects += 1;
                ensure!(redirects <= 4, "too many redirects");
                let location = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|v| v.to_str().ok())
                    .ok_or_else(|| anyhow!("redirect has no location"))?;
                let next = url.join(location)?;
                let host = next.host_str().unwrap_or("");
                let same_host = host == url.host_str().unwrap_or("");
                let rdap = ["arin_rdap", "apnic_rdap"].contains(&id)
                    && [
                        "rdap.arin.net",
                        "rdap.apnic.net",
                        "rdap.db.ripe.net",
                        "rdap.lacnic.net",
                        "rdap.afrinic.net",
                    ]
                    .contains(&host);
                ensure!(
                    next.scheme() == "https" && (same_host || rdap),
                    "redirect host is not allowed"
                );
                ensure!(
                    canonical_tool_id(id) != whoxy::TOOL_ID,
                    "Whoxy does not follow redirects"
                );
                url = next;
                continue;
            }
            if ((response.status().as_u16() == 429 && retry_429)
                || response.status().is_server_error())
                && attempts < 3
            {
                let delay = response
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(1 << attempts)
                    .min(15);
                let jitter = Utc::now().timestamp_subsec_millis() as u64 % 250;
                tokio::time::sleep(Duration::from_secs(delay) + Duration::from_millis(jitter))
                    .await;
                continue;
            }
            // Hunter's verifier answers 202 while the SMTP check is still running.
            if id == "hunter_email_verifier" && response.status().as_u16() == 202 && verifying < 3 {
                verifying += 1;
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
            let status = response.status();
            let content_type = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();
            let (raw, truncated) = read_body(response, cap).await?;
            let raw = redact(raw, &credential);
            let credits_reported = reported_credits(&raw);
            let mut result = ToolResult {
                tool_id: id.into(),
                inputs: inputs.clone(),
                status: "completed".into(),
                source_url: public_source_url(&url),
                retrieved_at: Utc::now().to_rfc3339(),
                observations: Value::Null,
                raw,
                error: None,
                cached: false,
                truncated,
                credits_charged: 0,
                credits_reported,
            };
            // One wire attempt, counted separately from the logical invocation.
            note_wire_request(id, attempts, status.as_u16(), sent_at.elapsed_ms());
            // A truncated SERP body is never parsed as a valid empty page
            // (spec §9 fix 6): the envelope cut mid-payload proves nothing.
            if truncated && serp.is_some() {
                let outcome = search_engines::SerpOutcome::ResponseTooLarge;
                result.observations = serp_unparsed_observations(
                    serp.as_ref(),
                    outcome,
                    &format!(
                        "engine response exceeded the {} byte body bound",
                        search_engines::SERP_MAX_BODY_BYTES
                    ),
                );
                result.status = serp_status(outcome).into();
                result.error = Some(serp_reason(outcome).to_string());
                redact_key(&mut result, &credential);
                if let Some(req) = serp.as_ref() {
                    note_engine_query(req, &result.observations, None, false);
                }
                return Ok(result);
            }
            if status.as_u16() == 451 && id.starts_with("hunter_") {
                claimed_email(&mut result);
                return Ok(result);
            }
            if status.as_u16() == 404 {
                // A provider 404 is a failure for the named engines, never a
                // `no_results` (spec §9 fix 1). Every other tool keeps its
                // 404-is-empty reading.
                if let Some(req) = serp.as_ref() {
                    let outcome = search_engines::SerpOutcome::UpstreamFailure;
                    result.observations = serp_unparsed_observations(
                        Some(req),
                        outcome,
                        &format!("provider returned HTTP {status}"),
                    );
                    result.status = serp_status(outcome).into();
                    result.error = Some(serp_reason(outcome).to_string());
                    redact_key(&mut result, &credential);
                    note_engine_query(req, &result.observations, None, false);
                    return Ok(result);
                }
                result.status = "no_results".into();
                return Ok(result);
            }
            if !status.is_success() {
                result.status = if status.as_u16() == 429 {
                    "rate_limited"
                } else {
                    "failed"
                }
                .into();
                if let Some(req) = serp.as_ref() {
                    // Behind a named engine a provider HTTP error is a failure with
                    // a typed outcome, never an unexplained empty page.
                    let outcome = if status.as_u16() == 429 {
                        search_engines::SerpOutcome::RateLimited
                    } else {
                        search_engines::SerpOutcome::UpstreamFailure
                    };
                    result.observations = serp_unparsed_observations(
                        Some(req),
                        outcome,
                        &format!("provider returned HTTP {status}"),
                    );
                    result.status = serp_status(outcome).into();
                    result.error = Some(serp_reason(outcome).to_string());
                    redact_key(&mut result, &credential);
                    note_engine_query(req, &result.observations, None, false);
                    return Ok(result);
                }
                result.error = Some(
                    news_legal::http_error(id, status.as_u16(), &result.raw)
                        .or_else(|| atlas_news::http_error(id, status.as_u16(), &result.raw))
                        .unwrap_or_else(|| {
                            format!("HTTP {status}: {}", error_summary(&result.raw))
                        }),
                );
                redact_key(&mut result, &credential);
                if using_primary
                    && !switched
                    && !fallback_key.is_empty()
                    && fallback_key != primary_key
                    && quota_limited(&result.status, result.error.as_deref())
                {
                    switched = true;
                    using_primary = false;
                    note_key_exhausted(&primary_key);
                    (url, credential, send_credential) = bind_request(id, &req.url, &fallback_key)?;
                    attempts = 0;
                    continue;
                }
                return Ok(result);
            }
            let mut partial = false;
            if let Some(base) = req.poll {
                match self
                    .poll_job(id, base, &result.raw, &credential, def.timeout_seconds)
                    .await
                {
                    Ok((raw, done)) => {
                        result.raw = raw;
                        result.credits_reported = reported_credits(&result.raw);
                        partial = !done;
                    }
                    Err(e) => {
                        result.status = "failed".into();
                        result.error = Some(e.to_string());
                        return Ok(result);
                    }
                }
            }
            let mut serp_retry = false;
            match parse_observations_with(id, &result.raw, &content_type, req.ndjson, serp.as_ref())
            {
                Ok((value, cut)) => {
                    result.observations = value;
                    result.truncated |= cut;
                    if id == whoxy::TOOL_ID {
                        if let Ok(query) = whoxy::parse_query(&inputs) {
                            if let Ok(raw_json) = serde_json::from_str::<Value>(&result.raw) {
                                if let Ok(full) =
                                    whoxy::parse_history_envelope(&raw_json, &query.domain)
                                {
                                    result.credits_reported =
                                        Some(whoxy::reported_lookup_cost(&full));
                                    result.observations = whoxy::bounded_model_view(
                                        &whoxy::project_history(&full, &query),
                                    );
                                    if full.zero_history {
                                        result.status = "no_results".into();
                                    }
                                }
                            }
                        }
                    }
                    if let Some(req) = serp.as_ref() {
                        // Typed status mapping (spec §9 fix 1). An empty item list
                        // never reaches `no_results` on its own.
                        let outcome = result
                            .observations
                            .get("outcome")
                            .and_then(Value::as_str)
                            .and_then(search_engines::SerpOutcome::parse);
                        if let Some(outcome) = outcome {
                            result.status = serp_status(outcome).into();
                            if outcome.is_failure() {
                                result.error = Some(serp_reason(outcome).to_string());
                                redact_key(&mut result, &credential);
                            }
                        }
                        // Bounded recovery (spec §9 fix 6): exactly one fresh
                        // same-engine fetch, for a real page the parser did not
                        // recognise, still inside this tool's request allowance.
                        // Never for a challenge, a consent wall, a rate limit, a
                        // links-only payload or an oversize body.
                        serp_retry = !serp_recovered
                            && attempts < 3
                            && result
                                .observations
                                .get("recoverable")
                                .and_then(Value::as_bool)
                                .unwrap_or(false);
                        note_engine_query(
                            req,
                            &result.observations,
                            Some(sent_at.elapsed_ms()),
                            false,
                        );
                    }
                }
                Err(e) => {
                    result.status = if e.to_string().contains("quota")
                        || e.to_string().contains("(rateLimited)")
                    {
                        "rate_limited"
                    } else {
                        "failed"
                    }
                    .into();
                    result.error = Some(e.to_string());
                    if let Some(req) = serp.as_ref() {
                        // No parsable envelope is a provider failure for a named
                        // engine, never an empty result set.
                        let outcome = search_engines::SerpOutcome::UpstreamFailure;
                        result.observations =
                            serp_unparsed_observations(Some(req), outcome, &e.to_string());
                        result.status = serp_status(outcome).into();
                        result.error = Some(serp_reason(outcome).to_string());
                        note_engine_query(req, &result.observations, None, false);
                    }
                    redact_key(&mut result, &credential);
                    if using_primary
                        && !switched
                        && !fallback_key.is_empty()
                        && fallback_key != primary_key
                        && quota_limited(&result.status, result.error.as_deref())
                    {
                        switched = true;
                        using_primary = false;
                        note_key_exhausted(&primary_key);
                        (url, credential, send_credential) =
                            bind_request(id, &req.url, &fallback_key)?;
                        attempts = 0;
                        continue;
                    }
                    return Ok(result);
                }
            }
            if serp_retry {
                serp_recovered = true;
                // Bounded, cancellable wait before the one recovery fetch.
                tokio::time::sleep(SERP_RECOVERY_WAIT).await;
                continue;
            }
            annotate(id, &inputs, &mut result);
            if partial {
                result.truncated = true;
                if let Some(object) = result.observations.as_object_mut() {
                    object.insert("status".into(), json!("partial"));
                }
            }
            if no_results(id, &result.observations) {
                result.status = if partial { "timeout" } else { "no_results" }.into();
            }
            return Ok(result);
        }
    }

    /// Polls a Firecrawl job until it finishes or `timeout` passes. Polling costs no
    /// credits. A job still running at the deadline is cancelled (best effort) and the
    /// last status payload is returned with `false`, so the call is recorded as partial.
    async fn poll_job(
        &self,
        id: &str,
        base: &'static str,
        started: &str,
        credential: &Option<(reqwest::header::HeaderName, String)>,
        timeout: u64,
    ) -> Result<(String, bool)> {
        let status_url = job_status_url(base, started)?;
        let deadline = std::time::Instant::now() + Duration::from_secs(timeout);
        let mut last = String::new();
        let mut polls = 0u32;
        while std::time::Instant::now() < deadline {
            polls += 1;
            tokio::time::sleep(Duration::from_secs(2)).await;
            let mut builder = self
                .client
                .get(status_url.clone())
                .timeout(Duration::from_secs(20));
            if let Some((name, value)) = credential {
                builder = builder.header(name, value);
            }
            let sent_at = crate::telemetry::Measured::start();
            let Ok(response) = builder.send().await else {
                continue;
            };
            let code = response.status();
            // Every poll is a wire request, counted separately from the POST.
            note_wire_request(id, polls, code.as_u16(), sent_at.elapsed_ms());
            if code.as_u16() == 429 || code.is_server_error() {
                continue;
            }
            let (text, _) = read_body(response, 4_000_000).await?;
            ensure!(
                code.is_success(),
                "job status HTTP {code}: {}",
                error_summary(&text)
            );
            let state = serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|value| {
                    value
                        .get("status")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .unwrap_or_default();
            last = text;
            if matches!(state.as_str(), "completed" | "failed" | "cancelled") {
                return Ok((last, true));
            }
        }
        let mut cancel = self
            .client
            .delete(status_url)
            .timeout(Duration::from_secs(10));
        if let Some((name, value)) = credential {
            cancel = cancel.header(name, value);
        }
        let _ = cancel.send().await;
        if last.is_empty() {
            last = json!({"status": "partial", "data": []}).to_string();
        }
        Ok((last, false))
    }
}

/// `GET {base}/{id}` for the job a Firecrawl POST started. The id is checked and the host
/// stays api.firecrawl.dev.
fn job_status_url(base: &str, started: &str) -> Result<Url> {
    let value: Value =
        serde_json::from_str(started).map_err(|e| anyhow!("malformed job response: {e}"))?;
    ensure!(
        value.get("success").and_then(Value::as_bool) != Some(false),
        "job was not accepted: {}",
        value
            .get("error")
            .map(Value::to_string)
            .unwrap_or_default()
            .chars()
            .take(200)
            .collect::<String>()
    );
    let job = value
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("job response has no id"))?;
    ensure!(
        (1..=100).contains(&job.len())
            && job.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'),
        "invalid job id"
    );
    let status_url = Url::parse(&format!("{base}/{job}"))?;
    ensure!(
        status_url.host_str() == Some("api.firecrawl.dev"),
        "job host is not allowed"
    );
    Ok(status_url)
}

async fn read_body(response: reqwest::Response, limit: usize) -> Result<(String, bool)> {
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    let mut truncated = false;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if bytes.len() + chunk.len() > limit {
            let room = limit - bytes.len();
            bytes.extend_from_slice(&chunk[..room]);
            truncated = true;
            break;
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok((String::from_utf8_lossy(&bytes).to_string(), truncated))
}

/// Hunter 451 `claimed_email`: the person asked Hunter not to process their data. The
/// call is kept for the audit trail, but without the payload, and it yields no bindings.
fn claimed_email(result: &mut ToolResult) {
    result.status = "no_results".into();
    result.raw = String::new();
    result.truncated = false;
    result.error = None;
    result.observations = json!({
        "claimed_email": true,
        "note": "Hunter returned 451 claimed_email; no person data is stored for this address.",
    });
}

/// Request context the parsers cannot see: the SociaVault platform, route, queried
/// account, and numeric id; the Hunter query that produced a match; the mapped site.
fn annotate(id: &str, inputs: &Value, result: &mut ToolResult) {
    let raw: Value = serde_json::from_str(&result.raw).unwrap_or(Value::Null);
    if id == "firecrawl_map" {
        let site = str_arg(inputs, "domain")
            .ok()
            .map(str::to_string)
            .or_else(|| {
                str_arg(inputs, "url")
                    .ok()
                    .and_then(|page| Url::parse(page).ok()?.host_str().map(str::to_string))
            })
            .unwrap_or_default();
        result.observations = providers::map_observations(&raw, &site);
        return;
    }
    let Some(object) = result.observations.as_object_mut() else {
        return;
    };
    if id.starts_with("sociavault_") {
        let route = select_route(id, inputs).ok();
        let platform = route.map(|route| route.platform).unwrap_or("");
        object.insert("platform".into(), json!(platform));
        if let Some(route) = route {
            object.insert("endpoint".into(), json!(route.endpoint));
        }
        if let Some(handle) = inputs.get("handle") {
            object.insert("queried_handle".into(), handle.clone());
        }
        if id == "sociavault_profile" {
            if let Some(found) = providers::sociavault_platform_id(platform, &raw) {
                object.insert("platform_id".into(), json!(found));
            }
        }
    }
    if id == "hunter_domain_finder" {
        let perfect = inputs
            .get("perfect_match")
            .is_some_and(|flag| flag == &json!(true) || flag == &json!("true"));
        object.insert("perfect_match".into(), json!(perfect));
        object.insert(
            "company".into(),
            inputs.get("company").cloned().unwrap_or(Value::Null),
        );
    }
    if id == "hunter_email_count" {
        for key in ["domain", "company"] {
            if let Some(value) = inputs.get(key) {
                object.insert(key.into(), value.clone());
            }
        }
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn an_html_block_page_becomes_a_short_reason() {
        let page = "<html>\n<head>\n<title>Forbidden - Stack Exchange</title>\n<style type=\"text/css\">body { color: #333; }</style></head><body><h1>Access Denied</h1><p>This IP address 104.28.164.108 has been blocked from access to our services. If you believe this to be in error, please contact us.</p><script>document.getElementById('x');</script></body></html>";
        assert_eq!(
            super::error_summary(page),
            "Forbidden - Stack Exchange: Access Denied This IP address 104.28.164.108 has been blocked from access to our services"
        );
        assert_eq!(
            super::error_summary("{\"error\":\"bad key\"}"),
            "{\"error\":\"bad key\"}"
        );
    }

    use super::search_engines::{ParserInput, SerpOutcome};
    use super::*;
    #[test]
    fn a_claimed_email_keeps_no_person_data() {
        let mut result = ToolResult {
            tool_id: "hunter_person_enrichment".into(),
            inputs: json!({"email": "jane@acmerobotics.com"}),
            status: "failed".into(),
            source_url: String::new(),
            retrieved_at: String::new(),
            observations: json!({"full_name": "Jane Example"}),
            raw: r#"{"errors":[{"id":"claimed_email"}]}"#.into(),
            error: Some("451".into()),
            cached: false,
            truncated: true,
            credits_charged: 0,
            credits_reported: None,
        };
        claimed_email(&mut result);
        assert_eq!(result.status, "no_results");
        assert!(result.raw.is_empty() && result.error.is_none() && !result.truncated);
        assert_eq!(result.observations["claimed_email"], true);
        assert!(result.observations.get("full_name").is_none());
        let bindings = crate::recon::investigation::rule_bindings(
            "Who is jane@acmerobotics.com?",
            "call-s1",
            "hunter_person_enrichment",
            &result.observations,
        );
        assert!(bindings.is_empty(), "{bindings:?}");
    }

    /// No tool request goes out with an empty User-Agent: an unset, empty, or whitespace
    /// `osint_user_agent` falls back to the default, and a non-blank custom value wins.
    #[test]
    fn every_tool_request_sends_a_non_empty_user_agent() {
        let agent = |headers: &[(reqwest::header::HeaderName, String)]| {
            let found: Vec<&String> = headers
                .iter()
                .filter(|(name, _)| name == reqwest::header::USER_AGENT)
                .map(|(_, value)| value)
                .collect();
            assert_eq!(found.len(), 1, "exactly one User-Agent header");
            found[0].clone()
        };
        let mut ids: Vec<&str> = registry().iter().map(|tool| tool.id).collect();
        ids.push("hunter_tech_lookup");
        assert_eq!(ids.len(), 67);
        for id in ids {
            for blank in [None, Some(""), Some("   "), Some(" \t\n ")] {
                let sent = agent(&request_headers(
                    canonical_tool_id(id),
                    custom_user_agent(blank),
                ));
                assert!(!sent.trim().is_empty(), "{id}: {blank:?}");
                assert_eq!(
                    sent, DEFAULT_USER_AGENT,
                    "{id}: {blank:?} falls back to the default"
                );
            }
            assert_eq!(
                agent(&request_headers(
                    id,
                    custom_user_agent(Some("  Argos test@example.com  "))
                )),
                "Argos test@example.com",
                "{id}: a custom value wins"
            );
        }
        assert!(DEFAULT_USER_AGENT.starts_with("Argos OSINT/0.1 ("));
        assert_eq!(effective_user_agent(Some("\t")), DEFAULT_USER_AGENT);
        assert_eq!(custom_user_agent(Some(" ")), None);
    }

    /// Cache lifetime follows the provider's credit reset: a day, a week, or a month.
    /// A provider that does not restore credits caches for a month.
    #[test]
    fn cache_follows_the_provider_plan_interval() {
        assert_eq!(PlanInterval::Daily.cache_seconds(), CACHE_DAY_SECONDS);
        assert_eq!(PlanInterval::Weekly.cache_seconds(), CACHE_WEEK_SECONDS);
        assert_eq!(PlanInterval::Monthly.cache_seconds(), CACHE_MONTH_SECONDS);
        assert_eq!(
            PlanInterval::Never.cache_seconds(),
            PlanInterval::Monthly.cache_seconds()
        );
        for tool in registry() {
            assert_eq!(tool.cache_seconds, cache_seconds(tool.id), "{}", tool.id);
        }
        assert_eq!(plan_interval("firecrawl_search"), PlanInterval::Monthly);
        assert_eq!(plan_interval("hunter_domain_search"), PlanInterval::Monthly);
        assert_eq!(plan_interval("sociavault_profile"), PlanInterval::Never);
        assert_eq!(plan_interval("newsapi_search"), PlanInterval::Daily);
        assert_eq!(
            plan_interval("courtlistener_case_search"),
            PlanInterval::Daily
        );
        assert_eq!(
            plan_interval("hackertarget_hostsearch"),
            PlanInterval::Daily
        );
        assert_eq!(plan_interval("github_repositories"), PlanInterval::Never);
        assert_eq!(plan_interval("blockchain_address"), PlanInterval::Never);
        assert_eq!(plan_interval("whoxy_whois_history"), PlanInterval::Never);
        assert_eq!(plan_interval("holehe_email_lookup"), PlanInterval::Daily);
        assert_eq!(cache_seconds("firecrawl_search"), CACHE_MONTH_SECONDS);
        assert_eq!(cache_seconds("sociavault_profile"), CACHE_MONTH_SECONDS);
        assert_eq!(cache_seconds("newsapi_headlines"), CACHE_DAY_SECONDS);
        assert_eq!(cache_seconds("crtsh_certificates"), CACHE_MONTH_SECONDS);
    }

    #[test]
    fn registry_and_validation() {
        assert_eq!(registry().len(), 66);
        let ids: std::collections::HashSet<_> = registry().iter().map(|t| t.id).collect();
        assert_eq!(ids.len(), 66);
        assert_eq!(
            registry()
                .iter()
                .map(|t| t.category)
                .collect::<std::collections::HashSet<_>>()
                .len(),
            16
        );
        for t in registry() {
            assert!(!t.description.is_empty());
            assert!(!t.documentation.is_empty());
            assert_eq!(t.schema()["type"], "object");
            validate(t.id, &t.example_input()).unwrap_or_else(|e| panic!("{} example: {e}", t.id));
        }
        assert!(request("overpass_places", &json!({"latitude":95,"longitude":0})).is_err());
        assert!(request("shodan_internetdb", &json!({"ip":"127.0.0.1"})).is_ok());
        let rsp = request(
            "wikipedia_source_reliability",
            &json!({"domain": "reuters.com"}),
        )
        .unwrap();
        assert_eq!(rsp.url.host_str(), Some("en.wikipedia.org"));
        assert!(rsp.url.query().unwrap_or("").contains("action=parse"));
        assert!(rsp.url.query().unwrap_or("").contains("prop=wikitext"));
        let search = request("firecrawl_search", &json!({"query":"who is example"})).unwrap();
        assert_eq!(search.url.as_str(), "https://api.firecrawl.dev/v2/search");
        assert_eq!(
            search.body,
            Some(json!({"query":"who is example","limit":3,"sources":["web"]}))
        );
        let page = request(
            "firecrawl_scrape",
            &json!({"url":"https://example.org/about"}),
        )
        .unwrap();
        assert_eq!(page.url.as_str(), "https://api.firecrawl.dev/v2/scrape");
        assert_eq!(page.body.as_ref().unwrap()["formats"][0], "markdown");
        assert!(request(
            "firecrawl_scrape",
            &json!({"url":"http://127.0.0.1/secret"})
        )
        .is_err());
        let contact = format!(
            "Jane Example jane@acmerobotics.com {}",
            "word ".repeat(1_200)
        );
        assert!(contact.chars().count() > 4_000);
        let raw = json!({"success": true, "data": {
            "markdown": contact,
            "metadata": {"title": "Contact", "sourceURL": "https://acmerobotics.com/contact"}
        }})
        .to_string();
        let (page_obs, truncated) =
            parse_observations("firecrawl_scrape", &raw, "application/json", false).unwrap();
        assert!(!truncated);
        assert_eq!(page_obs["markdown"], contact);
        assert_eq!(page_obs["title"], "Contact");
        assert_eq!(page_obs["evidence_form"], "page");
        let huge = "x".repeat(super::PAGE_MARKDOWN_CHARS + 40);
        let raw = json!({"data": {"markdown": huge, "metadata": {"sourceURL": "https://acmerobotics.com/about"}}})
            .to_string();
        let (clipped, truncated) =
            parse_observations("firecrawl_scrape", &raw, "application/json", false).unwrap();
        assert!(truncated);
        assert!(clipped["markdown"].as_str().unwrap().ends_with('…'));
        assert_eq!(
            clipped["markdown"].as_str().unwrap().chars().count(),
            super::PAGE_MARKDOWN_CHARS
        );
        assert_eq!(endpoint_cost("firecrawl_search").unwrap().credits, 2);
        assert_eq!(
            endpoint_cost("firecrawl_scrape").unwrap().provider,
            "firecrawl"
        );
        assert_eq!(
            endpoint_cost("hunter_email_verifier").unwrap().provider,
            "hunter"
        );
        assert_eq!(endpoint_cost("sociavault_profile").unwrap().credits, 1);
        assert!(endpoint_cost("wikidata_entities").is_none());
        assert_eq!(
            reported_credits(r#"{"success":true,"creditsUsed":2,"data":{}}"#),
            Some(2)
        );
        assert!(request(
            "hunter_domain_search",
            &json!({"domain":"example.org","limit":25})
        )
        .is_err());
        let domain_search =
            request("hunter_domain_search", &json!({"domain":"Example.ORG"})).unwrap();
        assert_eq!(
            domain_search.url.as_str(),
            "https://api.hunter.io/v2/domain-search?limit=10&domain=example.org"
        );
        assert!(request(
            "hunter_email_finder",
            &json!({"domain":"example.org","first_name":"Ada"})
        )
        .is_err());
        let finder = request(
            "hunter_email_finder",
            &json!({"domain":"example.org","full_name":"Ada Lovelace"}),
        )
        .unwrap();
        assert!(finder
            .url
            .as_str()
            .starts_with("https://api.hunter.io/v2/email-finder?"));
        assert!(!finder.url.as_str().contains("api_key"));
        let verifier =
            request("hunter_email_verifier", &json!({"email":"Ada@Example.ORG"})).unwrap();
        assert_eq!(
            verifier.url.as_str(),
            "https://api.hunter.io/v2/email-verifier?email=ada%40example.org"
        );
        let tech = request("hunter_tech_lookup", &json!({"domain":"hunter.io"})).unwrap();
        assert_eq!(
            tech.url.as_str(),
            "https://api.hunter.io/v2/companies/find?domain=hunter.io"
        );
        let profile = request(
            "sociavault_profile",
            &json!({"platform":"x","handle":"https://x.com/ExampleUser"}),
        )
        .unwrap();
        assert_eq!(
            profile.url.as_str(),
            "https://api.sociavault.com/v1/scrape/twitter/profile?handle=ExampleUser"
        );
        let linkedin = request(
            "sociavault_profile",
            &json!({"platform":"linkedin","handle":"https://www.linkedin.com/company/hunterio"}),
        )
        .unwrap();
        assert_eq!(
            linkedin.url.as_str(),
            "https://api.sociavault.com/v1/scrape/linkedin/company?url=https%3A%2F%2Fwww.linkedin.com%2Fcompany%2Fhunterio"
        );
        let channel = request(
            "sociavault_profile",
            &json!({"platform":"youtube","handle":"UCxxxxxxxxxxxxxxxxxxxxxx"}),
        )
        .unwrap();
        assert_eq!(
            channel.url.as_str(),
            "https://api.sociavault.com/v1/scrape/youtube/channel?channelId=UCxxxxxxxxxxxxxxxxxxxxxx"
        );
        assert!(request(
            "sociavault_profile",
            &json!({"platform":"reddit","handle":"example"})
        )
        .is_err());
        assert!(provider_credential("hunter_domain_search", &ProviderKeys::default()).is_err());
    }
    #[test]
    fn parse_fixtures() {
        let (crt, _) = parse_observations(
            "crtsh_certificates",
            r#"[{"name_value":"a.example.com\n*.example.com"},{"name_value":"a.example.com"}]"#,
            "application/json",
            false,
        )
        .unwrap();
        assert_eq!(crt["hostnames"].as_array().unwrap().len(), 2);
        let (csv, _) = parse_observations(
            "hackertarget_hostsearch",
            "a.example.com,1.2.3.4\n",
            "text/plain",
            false,
        )
        .unwrap();
        assert_eq!(csv[0]["ip"], "1.2.3.4");
        assert!(parse_observations(
            "hackertarget_hostsearch",
            "API count exceeded",
            "text/plain",
            false
        )
        .is_err());
        let (ndjson, _) = parse_observations(
            "commoncrawl_urls",
            "{\"url\":\"https://example.com\"}\n",
            "text/plain",
            true,
        )
        .unwrap();
        assert_eq!(ndjson.as_array().unwrap().len(), 1);
        assert!(parse_observations("commoncrawl_urls", "not-json", "text/plain", true).is_err());
        assert!(parse_observations("nvd_cve", "<html>error</html>", "text/html", false).is_err());
        assert!(parse_observations("nvd_cve", "oops", "application/json", false).is_err());
        let (web, truncated) = parse_observations(
            "firecrawl_search",
            r#"{"success":true,"data":{"web":[{"title":"Jeff Bezos","description":"American businessman","url":"https://en.wikipedia.org/wiki/Jeff_Bezos"},{"title":"Wikidata","description":"Q312556","url":"https://www.wikidata.org/wiki/Q312556"}]}}"#,
            "application/json",
            false,
        )
        .unwrap();
        assert!(!truncated);
        assert_eq!(web["results"].as_array().unwrap().len(), 2);
        assert_eq!(
            web["results"][0]["url"],
            "https://en.wikipedia.org/wiki/Jeff_Bezos"
        );
        assert!(web["results"][1]["snippet"]
            .as_str()
            .unwrap()
            .contains("Q312556"));
        assert!(parse_observations(
            "firecrawl_search",
            r#"{"success":false,"error":"unauthorized"}"#,
            "application/json",
            false
        )
        .is_err());
        let (company, _) = parse_observations(
            "hunter_tech_lookup",
            r#"{"data":{"name":"Hunter","domain":"hunter.io","description":"Email data","location":"Wilmington, Delaware","category":{"industry":"Internet"},"metrics":{"employees":"11-50"},"tech":["ruby","hsts"],"techCategories":["security"],"site":{"emailAddresses":["support@hunter.io"]},"linkedin":{"handle":"company/hunterio"},"twitter":{"handle":null}}}"#,
            "application/json",
            false,
        )
        .unwrap();
        assert_eq!(company["tech"][0], "ruby");
        assert_eq!(company["social"]["linkedin"]["handle"], "company/hunterio");
        assert_eq!(company["emails"][0], "support@hunter.io");
        let (profile_card, _) = parse_observations(
            "sociavault_profile",
            r#"{"success":true,"data":{"user":{"uniqueId":"example","nickname":"Example","signature":"Builder. https://example.org"},"stats":{"followerCount":10,"followingCount":2},"bio_links":{"0":{"url":"https://www.youtube.com/@example"}}}}"#,
            "application/json",
            false,
        )
        .unwrap();
        assert_eq!(profile_card["handle"], "example");
        assert_eq!(profile_card["name"], "Example");
        assert!(profile_card["biography"]
            .as_str()
            .unwrap()
            .contains("Builder"));
        assert_eq!(profile_card["stats"]["followerCount"], 10);
        assert!(profile_card["links"]
            .as_array()
            .unwrap()
            .iter()
            .any(|link| link.as_str().unwrap().contains("youtube.com")));
    }

    #[test]
    fn whoxy_cache_identity_is_domain_only() {
        let a = cache_identity(
            "whoxy_whois_history",
            &json!({"domain":"Example.ORG.","from":"2015-01-01","limit":2}),
        );
        let b = cache_identity(
            "whoxy_whois_history",
            &json!({"domain":"example.org","to":"2016-01-01"}),
        );
        assert_eq!(a, b);
        assert!(a.contains("example.org"));
        assert!(!a.contains("from"));
    }

    #[test]
    fn whoxy_source_url_drops_the_key() {
        let mut url =
            Url::parse("https://api.whoxy.com/?history=example.org&key=SECRETKEY").unwrap();
        url.query_pairs_mut().append_pair("extra", "1");
        let public = public_source_url(&url);
        assert!(!public.contains("SECRETKEY"));
        assert!(!public.contains("key="));
        assert!(public.contains("history=example.org"));
    }

    #[test]
    fn whoxy_projects_cached_full_history() {
        let raw = json!({
            "status": 1,
            "total_records_found": 2,
            "whois_records": [
                {"query_time": "2014-01-01 00:00:00", "domain_registrar": {"registrar_name": "A"}},
                {"query_time": "2016-01-01 00:00:00", "domain_registrar": {"registrar_name": "B"}}
            ]
        });
        let cached = ToolResult {
            tool_id: "whoxy_whois_history".into(),
            inputs: json!({"domain": "example.org"}),
            status: "completed".into(),
            source_url: "https://api.whoxy.com/?history=example.org".into(),
            retrieved_at: "2020-01-01T00:00:00Z".into(),
            observations: json!({}),
            raw: raw.to_string(),
            error: None,
            cached: false,
            truncated: false,
            credits_charged: 1,
            credits_reported: Some(1),
        };
        let projected = project_cached_result(
            "whoxy_whois_history",
            &json!({"domain": "example.org", "from": "2015-01-01", "limit": 1}),
            &cached,
        );
        assert!(projected.cached);
        assert_eq!(projected.credits_charged, 0);
        assert_eq!(projected.observations["displayed"], 1);
        assert_eq!(projected.observations["omitted"], 0);
        assert_eq!(
            projected.observations["snapshots"][0]["query_time"],
            "2016-01-01 00:00:00"
        );
    }

    // -----------------------------------------------------------------------
    // Named SERP outcome contract (spec §9)
    // -----------------------------------------------------------------------

    /// A Google SERP envelope with `dom` as the fetched page.
    fn google_envelope(dom: &str, serp_url: &str) -> Value {
        json!({
            "success": true,
            "data": {
                "rawHtml": dom,
                "links": ["https://link-only.example.org/"],
                "metadata": {"sourceURL": serp_url, "statusCode": 200}
            }
        })
    }

    fn google_query() -> search_engines::SerpRequest {
        search_engines::SerpRequest::for_tool("firecrawl_google_search", "rust ownership", 5)
            .expect("named engine request")
    }

    fn parse_google(dom: &str) -> Value {
        let req = google_query();
        let raw = google_envelope(dom, &req.serp_url).to_string();
        let (observations, truncated) = parse_observations_with(
            "firecrawl_google_search",
            &raw,
            "application/json",
            false,
            Some(&req),
        )
        .unwrap();
        assert!(!truncated, "a small page is never a truncated one");
        observations
    }

    /// A page nobody recognises proves nothing. It is a parser mismatch, never a
    /// `no_results`, and never a result that may be cached as a zero.
    #[test]
    fn unknown_serp_html_is_not_no_results() {
        let observations = parse_google(
            "<!DOCTYPE html><html><body><div class=\"unknown-drifted-shape\">\
             <a href=\"https://drift.example.org/x\">link only</a></div></body></html>",
        );
        assert_eq!(
            observations["outcome"].as_str(),
            Some("parser_mismatch"),
            "{observations}"
        );
        assert_eq!(observations["usable_results"], 0);
        assert_eq!(observations["parser_input"], "raw_html");
        assert!(
            observations["results"]
                .as_array()
                .is_some_and(|rows| rows.is_empty()),
            "no card is invented from an unrecognised page"
        );
        assert!(
            !no_results("firecrawl_google_search", &observations),
            "an unrecognised page must never be cached or reported as a zero"
        );
        assert_eq!(serp_status(SerpOutcome::ParserMismatch), "failed");
        let reason = serp_reason(SerpOutcome::ParserMismatch);
        assert!(reason.contains("not recognised"), "{reason}");
        assert!(
            !reason.to_ascii_lowercase().contains("no result"),
            "a machine reason, not the words \"no results\": {reason}"
        );
        let result = ToolResult {
            tool_id: "firecrawl_google_search".into(),
            inputs: json!({"query": "rust ownership"}),
            status: "failed".into(),
            source_url: google_query().serp_url,
            retrieved_at: "2026-10-09T00:00:00Z".into(),
            observations: observations.clone(),
            raw: String::new(),
            error: Some(reason.to_string()),
            cached: false,
            truncated: false,
            credits_charged: 1,
            credits_reported: None,
        };
        assert_eq!(
            classify_output_quality(&result),
            OutputQuality::ParserMismatch
        );
    }

    /// Only a recognized engine status region with a supported phrase and zero
    /// accepted cards is a verified zero (spec §9 fix 1).
    #[test]
    fn verified_zero_maps_to_no_results() {
        let observations = parse_google(
            "<!DOCTYPE html><html><body><div id=\"main\">\
             <p>Your search - rust ownership - did not match any documents.</p>\
             </div></body></html>",
        );
        assert_eq!(
            observations["outcome"].as_str(),
            Some("verified_zero"),
            "{observations}"
        );
        assert!(no_results("firecrawl_google_search", &observations));
        assert_eq!(serp_status(SerpOutcome::VerifiedZero), "completed");
        let result = ToolResult {
            tool_id: "firecrawl_google_search".into(),
            inputs: json!({"query": "rust ownership"}),
            status: "completed".into(),
            source_url: google_query().serp_url,
            retrieved_at: "2026-10-09T00:00:00Z".into(),
            observations,
            raw: String::new(),
            error: None,
            cached: false,
            truncated: false,
            credits_charged: 1,
            credits_reported: None,
        };
        assert_eq!(
            classify_output_quality(&result),
            OutputQuality::NoResults,
            "a recognized status region is a zero"
        );
    }

    /// A real SERP with a card is `Valid`, gets a rank and a title, and is not a
    /// zero even though the engine's internal links are dropped.
    #[test]
    fn valid_serp_cards_are_not_a_zero() {
        let observations = parse_google(
            "<!DOCTYPE html><html><body><div class=\"g\">\
             <a href=\"https://alpha.example.org/docs\"><h3>Alpha heading</h3></a>\
             <div class=\"VwiC3b\">Alpha snippet text.</div></div></body></html>",
        );
        assert_eq!(observations["outcome"].as_str(), Some("valid"));
        assert_eq!(observations["usable_results"], 1);
        assert_eq!(observations["results"][0]["title"], "Alpha heading");
        assert_eq!(observations["results"][0]["snippet"], "Alpha snippet text.");
        assert_eq!(observations["results"][0]["rank"], 1);
        assert_eq!(observations["recoverable"], false);
        assert!(!no_results("firecrawl_google_search", &observations));
    }

    /// Cache identity v2 carries the parser and fetch contract versions and the
    /// canonical query, so a parser upgrade or a reworded query can never reuse a
    /// legacy entry (spec §9 fix 7).
    #[test]
    fn named_serp_cache_identity_includes_parser_version() {
        let a = cache_identity(
            "firecrawl_google_search",
            &json!({"query": "Rust  Ownership"}),
        );
        let b = cache_identity(
            "firecrawl_google_search",
            &json!({"query": "async rust traits"}),
        );
        let same_words = cache_identity(
            "firecrawl_google_search",
            &json!({"query": "rust   ownership"}),
        );
        assert_ne!(a, b, "two different queries must not share a cache entry");
        assert!(!a.contains("Rust"), "the canonical query is lowercased");
        assert!(a.contains("rust ownership"), "{a}");
        assert_eq!(
            a, same_words,
            "whitespace collapses into the canonical query"
        );
        assert!(a.contains(":v2:"), "{a}");
        assert!(
            a.contains(search_engines::PARSER_VERSION),
            "{a} must carry the parser version"
        );
        assert!(
            a.contains(search_engines::FETCH_CONTRACT_VERSION),
            "{a} must carry the fetch contract version"
        );
        assert!(a.contains("named_serp"), "{a}");
        assert!(b.contains("async rust traits"), "{b}");
        // The v2 identity is the tool, the version pair and the canonical
        // query/locale/limit fragment, so a parser upgrade invalidates legacy v1
        // entries without deleting them.
        let fragment = search_engines::cache_fragment(
            &serp_request_for(
                "firecrawl_google_search",
                &json!({"query": "rust ownership"}),
            )
            .expect("named engine request"),
        );
        assert_eq!(a, format!("firecrawl_google_search:v2:{fragment}"));
        assert!(
            fragment.starts_with(&format!(
                "serp:{}:{}",
                search_engines::PARSER_VERSION,
                search_engines::FETCH_CONTRACT_VERSION
            )),
            "{fragment}"
        );
        assert!(fragment.ends_with(":5"), "{fragment}: the bounded limit");
        // A legacy v1 identity never matches, so old entries are never reused.
        assert_ne!(
            a,
            format!("firecrawl_google_search:v1:{{\"query\":\"rust ownership\"}}")
        );
        // Every other tool keeps the generic v1 path.
        let generic = cache_identity("firecrawl_search", &json!({"query": "rust ownership"}));
        assert!(generic.contains(":v1:"), "{generic}");
        assert!(!generic.contains("serp:"), "{generic}");
        let whoxy = cache_identity("whoxy_whois_history", &json!({"domain": "Example.ORG."}));
        assert_eq!(
            whoxy,
            cache_identity("whoxy_whois_history", &json!({"domain": "example.org"})),
            "the whoxy special case is unchanged"
        );
        assert!(whoxy.contains(":v1:"), "{whoxy}");
    }

    /// A named SERP body may be 8 MiB; a 9 MiB response is refused instead of
    /// being parsed as a valid empty page (spec §9 fix 6).
    #[test]
    fn named_serp_body_cap_refuses_an_oversize_response() {
        assert_eq!(search_engines::SERP_MAX_BODY_BYTES, 8 * 1024 * 1024);
        assert_eq!(
            body_cap("firecrawl_google_search"),
            search_engines::SERP_MAX_BODY_BYTES
        );
        assert_eq!(
            body_cap("firecrawl_yandex_search"),
            search_engines::SERP_MAX_BODY_BYTES
        );
        assert_eq!(
            body_cap("firecrawl_mojeek_search"),
            search_engines::SERP_MAX_BODY_BYTES
        );
        // Every other tool keeps the flat 1 MB limit.
        assert_eq!(body_cap("firecrawl_search"), 1_000_000);
        assert_eq!(body_cap("firecrawl_scrape"), 1_000_000);
        assert_eq!(body_cap("crtsh_certificates"), 1_000_000);

        let req = google_query();
        let huge = "x".repeat(search_engines::SERP_MAX_BODY_BYTES + 64 * 1024);
        assert!(huge.len() > search_engines::SERP_MAX_BODY_BYTES);
        let raw = google_envelope(&huge, &req.serp_url).to_string();
        let (observations, _) = parse_observations_with(
            "firecrawl_google_search",
            &raw,
            "application/json",
            false,
            Some(&req),
        )
        .unwrap();
        assert_eq!(
            observations["outcome"].as_str(),
            Some("response_too_large"),
            "{:?}",
            observations["outcome"]
        );
        assert!(
            !no_results("firecrawl_google_search", &observations),
            "an oversize body is a failure, never a verified zero"
        );
        assert_eq!(
            serp_status(SerpOutcome::ResponseTooLarge),
            "failed",
            "an oversize body must fail the call"
        );
        // Everything else still parses below the bound.
        let small = "x".repeat(4096);
        let raw = google_envelope(&small, &req.serp_url).to_string();
        let (observations, _) = parse_observations_with(
            "firecrawl_google_search",
            &raw,
            "application/json",
            false,
            Some(&req),
        )
        .unwrap();
        assert_eq!(observations["outcome"].as_str(), Some("parser_mismatch"));
    }

    /// Every typed outcome maps to exactly one status, one machine reason and one
    /// terminal outcome class (spec §9 fix 1). Nothing unexpected may become
    /// `no_results`.
    #[test]
    fn every_typed_outcome_maps_to_one_status_reason_and_class() {
        use crate::telemetry::ToolOutcome;
        // `serp_status`, `serp_reason` and `serp_tool_outcome` each match over
        // every `SerpOutcome` variant, so adding one breaks this build.
        let wanted: [(SerpOutcome, &str, &str, ToolOutcome); 8] = [
            (
                SerpOutcome::Valid,
                "completed",
                "serp_ok",
                ToolOutcome::CompletedNonEmpty,
            ),
            (
                SerpOutcome::VerifiedZero,
                "completed",
                "serp_ok",
                ToolOutcome::CompletedVerifiedZero,
            ),
            (
                SerpOutcome::Challenge,
                "blocked",
                "serp_challenge",
                ToolOutcome::Blocked,
            ),
            (
                SerpOutcome::Consent,
                "blocked",
                "serp_consent",
                ToolOutcome::Blocked,
            ),
            (
                SerpOutcome::RateLimited,
                "rate_limited",
                "serp_rate_limited",
                ToolOutcome::Blocked,
            ),
            (
                SerpOutcome::ParserMismatch,
                "failed",
                "serp_parser_mismatch",
                ToolOutcome::ParserMismatch,
            ),
            (
                SerpOutcome::UpstreamFailure,
                "failed",
                "serp_upstream_failure",
                ToolOutcome::Failed,
            ),
            (
                SerpOutcome::ResponseTooLarge,
                "failed",
                "serp_response_too_large",
                ToolOutcome::Failed,
            ),
        ];
        for (outcome, status, reason, class) in wanted {
            assert_eq!(serp_status(outcome), status, "{outcome:?}");
            let said = serp_reason(outcome);
            // A success is a bare machine token; a failure is `token: detail`,
            // so the reason is always the leading field. It never spells out
            // "no results": the outcome already says what the engine did.
            assert!(
                said == reason || said.starts_with(&format!("{reason}:")),
                "{outcome:?}: {said}"
            );
            assert!(
                !said.to_ascii_lowercase().contains("no result"),
                "{outcome:?}: a machine reason, never \"no results\": {said}"
            );
            assert_eq!(serp_tool_outcome(outcome), class, "{outcome:?}");
        }
    }

    /// A provider 404 and a provider HTTP 500 are failures for the named engines,
    /// never `no_results` (spec §9 fix 1).
    #[test]
    fn provider_http_failures_are_not_a_zero() {
        let req = google_query();
        let raw = json!({"success": true, "code": 404}).to_string();
        let (observations, _) = parse_observations_with(
            "firecrawl_google_search",
            &raw,
            "application/json",
            false,
            Some(&req),
        )
        .unwrap();
        assert_eq!(observations["outcome"].as_str(), Some("upstream_failure"));
        assert!(!no_results("firecrawl_google_search", &observations));
    }

    /// Recovery is offered exactly once, for a real unrecognised page, and never
    /// for a links-only payload, a wall or an oversize body (spec §9 fix 6).
    #[test]
    fn bounded_recovery_is_offered_once_for_an_unrecognised_page_only() {
        let recoverable = |outcome: SerpOutcome, parser_input: ParserInput| {
            let diagnostics = search_engines::SerpDiagnostics {
                target_status: None,
                requested_url: String::new(),
                final_url: String::new(),
                provider_warning: String::new(),
                provider_error: String::new(),
                input_bytes: 0,
                parser_input,
                dom_candidates: 0,
                accepted: 0,
                rejections: Vec::new(),
                status_region: String::new(),
            };
            search_engines::retry_allowed(outcome, &diagnostics)
        };
        assert!(recoverable(
            SerpOutcome::ParserMismatch,
            ParserInput::RawHtml
        ));
        assert!(
            !recoverable(SerpOutcome::ParserMismatch, ParserInput::LinksOnly),
            "a links-only payload would re-request the same shape"
        );
        assert!(recoverable(
            SerpOutcome::ParserMismatch,
            ParserInput::CleanHtml
        ));
        for outcome in [
            SerpOutcome::Challenge,
            SerpOutcome::Consent,
            SerpOutcome::RateLimited,
            SerpOutcome::UpstreamFailure,
            SerpOutcome::ResponseTooLarge,
            SerpOutcome::VerifiedZero,
            SerpOutcome::Valid,
        ] {
            assert!(
                !recoverable(outcome, ParserInput::RawHtml),
                "{outcome:?} must not spend another request"
            );
        }
        // The parsed observations carry the same answer, so the executor can
        // honour the bound without rebuilding the diagnostics.
        let observations = parse_google(
            "<!DOCTYPE html><html><body><div class=\"unknown-drifted-shape\">\
             <a href=\"https://drift.example.org/x\">link only</a></div></body></html>",
        );
        assert_eq!(observations["recoverable"], true);
        // A recovery flag is only ever set on a real page, so the executor's
        // single retry cannot be talked into a loop.
        assert_eq!(serp_status(SerpOutcome::ParserMismatch), "failed");
    }
}
