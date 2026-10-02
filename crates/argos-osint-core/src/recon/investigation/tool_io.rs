//! Table-driven tool inputs and outputs.
//!
//! `TOOLS` has one row per catalog tool. The picker reads it for eligibility, input kinds,
//! and declared dependencies; the binder reads it to fill arguments from accepted
//! bindings; the rule extractor reads it to know which binding kinds to parse from that
//! tool's observation. There is no other per-tool input or output logic.

use serde_json::{json, Value};

use super::super::{Binding, Directive};
use super::{
    clip_query, emails_in, names_subject, normalize_platform,
    select_entities, social_or_publisher, subject_of, SearchHit, ACCOUNTS,
};

/// Closed binding vocabulary shared by questions, the picker, and the binder.
pub const BINDING_KINDS: &[&str] = &[
    "domain", "ip", "email", "handle", "platform", "person_name", "org_name", "cve", "package",
    "address", "wallet", "platform_id",
];

/// A numeric account id (Twitter `rest_id`, Instagram user id, YouTube `channelId`) from a
/// SociaVault profile call. Its qualifier is the platform.
pub const PLATFORM_ID_KIND: &str = "platform_id";

/// Kinds deliberately produced only by tools, never by the prompt, with the reason.
#[cfg_attr(not(test), allow(dead_code))]
pub const TOOL_ONLY: &[(&str, &str)] = &[(
    PLATFORM_ID_KIND,
    "numeric account ids come from a SociaVault profile call, so user-content routes run after it",
)];

/// Source of a binding read from the user's prompt.
pub const PROMPT_SOURCE: &str = "prompt";

/// Producers whose observations may feed a consumer's inputs, as tool-id prefixes. Hunter
/// takes inputs only from the prompt and the primary providers (decision D1); Firecrawl
/// map and crawl run only on a domain from the prompt, Firecrawl, or Hunter. `None`: any
/// producer.
pub fn restricted_sources(consumer: &str) -> Option<&'static [&'static str]> {
    let consumer = crate::osint::canonical_tool_id(consumer);
    if consumer.starts_with("hunter_") {
        Some(&["firecrawl_", "sociavault_", "hunter_"])
    } else if matches!(consumer, "firecrawl_map" | "firecrawl_crawl") {
        Some(&["firecrawl_", "hunter_"])
    } else {
        None
    }
}

/// Whether `producer`'s observation may feed `consumer`.
pub fn allowed_producer(consumer: &str, producer: &str) -> bool {
    producer == PROMPT_SOURCE || restricted_sources(consumer).is_none_or(|prefixes| prefixes.iter().any(|prefix| producer.starts_with(prefix)))
}

/// Where a binding came from: its tool, `prompt` for the user's question, or empty for
/// an older binding with no recorded tool (never eligible for a restricted consumer).
pub fn binding_source(binding: &Binding) -> &str {
    if !binding.source_tool.is_empty() {
        &binding.source_tool
    } else if binding.evidence_id == "question" {
        PROMPT_SOURCE
    } else {
        ""
    }
}

/// A binding may fill `consumer`'s input only when its evidence is the prompt or an
/// allowed producer's observation. A gap-filler value becomes eligible once a primary
/// observation also contains it (the merge then records the primary source).
pub fn binding_allowed(consumer: &str, binding: &Binding) -> bool {
    let source = binding_source(binding);
    restricted_sources(consumer).is_none() || (!source.is_empty() && allowed_producer(consumer, source))
}

/// Ordering gates: the first tool runs before the second when both are planned. Email
/// count gates the paid domain search; email insight routes an address to the right
/// enrichment. Neither yields a binding, so they are not producers.
pub const GATES: &[(&str, &str)] = &[
    ("hunter_email_count", "hunter_domain_search"),
    ("hunter_email_insight", "hunter_person_enrichment"),
    ("hunter_email_insight", "hunter_combined_enrichment"),
];

/// Page URLs are an internal binding: `firecrawl_scrape`, `wayback_availability`, and
/// `arquivo_history` take a URL from a previous observation.
pub const URL_KIND: &str = "url";

/// `lat,lon` from a geocoder, for `overpass_places`. Internal, like `url`.
pub const COORDINATES_KIND: &str = "coordinates";

/// A search query, always available: the derived question's text or the subject.
pub const QUERY_KIND: &str = "query";

/// Kinds the prompt extractor (`question_bindings`) can produce from the user's text.
#[cfg_attr(not(test), allow(dead_code))]
pub const PROMPT_KINDS: &[&str] = &[
    "domain", "ip", "email", "url", "cve", "wallet", "handle", "person_name", "org_name",
    "address", "package", "coordinates",
];

/// Binding kinds deliberately produced only by the prompt, with the reason. Empty: every
/// kind a tool input takes is also parsed from at least one tool's observation.
#[cfg_attr(not(test), allow(dead_code))]
pub const PROMPT_ONLY: &[(&str, &str)] = &[];

/// Directive context kinds (#29): a directive may target them, but they are not binding
/// kinds and no tool binds them. News tools serve `news`, Legal tools serve `legal`.
pub const NEWS_KIND: &str = "news";
pub const LEGAL_KIND: &str = "legal";
pub const CONTEXT_KINDS: &[&str] = &[NEWS_KIND, LEGAL_KIND];

pub fn known_kind(kind: &str) -> bool {
    BINDING_KINDS.contains(&kind) || kind == URL_KIND || kind == COORDINATES_KIND
}

/// A kind a directive may target: a binding kind or a context kind.
pub fn target_kind_allowed(kind: &str) -> bool {
    known_kind(kind) || CONTEXT_KINDS.contains(&kind)
}

/// The context kind a tool serves (`news`, `legal`), if it is a context tool.
pub fn context_of(tool_id: &str) -> Option<&'static str> {
    tool_row(tool_id).map(|row| row.context).filter(|kind| !kind.is_empty())
}

const MONTHS: &[&str] = &["january", "february", "march", "april", "may", "june", "july", "august", "september", "october", "november", "december"];

/// A date at the start of `words`: `2026-03-15`, `March 2026`, `March 15, 2026`,
/// `15 March 2026`, or a bare year. `end` picks the last day of a month or year.
fn date_at(words: &[String], end: bool) -> Option<String> {
    let first = words.first()?;
    if let Ok(date) = chrono::NaiveDate::parse_from_str(first, "%Y-%m-%d") {
        return Some(date.format("%Y-%m-%d").to_string());
    }
    let year = |word: &str| word.parse::<i32>().ok().filter(|year| (1900..=2100).contains(year));
    let month = |word: &str| MONTHS.iter().position(|name| *name == word || (word.len() >= 3 && name.starts_with(word) && word.len() <= name.len())).map(|index| index as u32 + 1);
    let day = |word: &str| word.parse::<u32>().ok().filter(|day| (1..=31).contains(day));
    let build = |y: i32, m: u32, d: Option<u32>| {
        let date = match d {
            Some(d) => chrono::NaiveDate::from_ymd_opt(y, m, d)?,
            None if end => {
                let (ny, nm) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
                chrono::NaiveDate::from_ymd_opt(ny, nm, 1)?.pred_opt()?
            }
            None => chrono::NaiveDate::from_ymd_opt(y, m, 1)?,
        };
        Some(date.format("%Y-%m-%d").to_string())
    };
    let second = words.get(1).map(String::as_str).unwrap_or("");
    let third = words.get(2).map(String::as_str).unwrap_or("");
    if let Some(m) = month(first) {
        if let (Some(d), Some(y)) = (day(second), year(third)) {
            return build(y, m, Some(d));
        }
        return year(second).and_then(|y| build(y, m, None));
    }
    if let (Some(d), Some(m), Some(y)) = (day(first), month(second), year(third)) {
        return build(y, m, Some(d));
    }
    year(first).and_then(|y| build(y, if end { 12 } else { 1 }, if end { Some(31) } else { Some(1) }))
}

/// Dates the prompt writes: `(start, end)` from "since/after/from <date>" and
/// "before/until/through <date>". None otherwise; dates are never invented.
pub fn prompt_dates(question: &str) -> (Option<String>, Option<String>) {
    let words: Vec<String> = question
        .to_ascii_lowercase()
        .split(|ch: char| ch.is_whitespace() || ch == ',' || ch == '?' || ch == '!')
        .map(|word| word.trim_matches(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '-')).to_string())
        .filter(|word| !word.is_empty())
        .collect();
    let (mut start, mut end) = (None, None);
    for (index, word) in words.iter().enumerate() {
        let rest = &words[index + 1..];
        match word.as_str() {
            "since" | "after" | "from" if start.is_none() => start = date_at(rest, false),
            "before" if end.is_none() => end = date_at(rest, false),
            "until" | "through" | "till" if end.is_none() => end = date_at(rest, true),
            _ => {}
        }
    }
    (start, end)
}

/// The SociaVault tools whose observation lists accounts found by search: those handles
/// stay `unverified` until a profile call or a Firecrawl page links them to the subject.
fn search_handles(tool_id: &str) -> bool {
    tool_row(tool_id).is_some_and(|row| row.unverified_handles)
}

/// How a binding becomes tool arguments.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum How {
    /// The binding value goes into `input` as is.
    Plain,
    /// A domain becomes `https://<domain>`.
    DomainAsUrl,
    /// A handle on a platform the SociaVault tool serves fills `platform` (its qualifier)
    /// and `handle`.
    SocialPair(&'static str),
    /// A platform id on a platform the SociaVault tool serves fills `platform` and `user_id`.
    PlatformIdPair(&'static str),
    /// A name or handle fills `query`, and `platform` is the first platform the question
    /// names that the SociaVault tool serves, else the tool's default platform.
    PlatformQuery(&'static str),
    /// A handle bound on exactly this platform (no fallback to other platforms).
    PlatformHandle(&'static str),
    /// Up to `BATCH_SCRAPE_DEFAULT_URLS` page URLs, contact and about pages first.
    UrlList,
    /// `r/<name>` named in the question text.
    Subreddit,
    /// An email address on a company (non-webmail) domain.
    CompanyEmail,
    /// An email address on a webmail domain.
    WebmailEmail,
    /// A handle, preferring one on this platform, else the subject's best-supported handle.
    Username(&'static str),
    /// `ecosystem:name@version` fills `ecosystem`, `package_name`, and `version`.
    PackageParts,
    /// `ecosystem:name@version` fills `input` with the package name.
    PackageName,
    /// `lat,lon` fills `latitude` and `longitude` as numbers.
    Coordinates,
    /// The derived question's text, else the subject.
    SearchQuery,
    /// The directive entity alone (News and Legal tools send it as an exact phrase).
    EntityPhrase,
    /// A date written in the prompt: `true` for a start ("since March 2026"), `false` for
    /// an end ("before May 2026"). Never filled otherwise.
    PromptDate(bool),
}

pub struct Fill {
    pub kind: &'static str,
    pub input: &'static str,
    pub how: How,
}

/// One required input group. Fills are alternatives in preference order.
pub struct Slot {
    pub fills: &'static [Fill],
}

/// One catalog tool's input requirements and what the binder parses from its output.
pub struct ToolIo {
    pub tool: &'static str,
    /// Every slot must be filled before the tool runs.
    pub slots: &'static [Slot],
    /// Fixed numeric arguments.
    pub extras: &'static [(&'static str, i64)],
    /// Declared producers: dependency edges the picker may not remove.
    pub after: &'static [&'static str],
    /// Binding kinds the rule extractor parses from this tool's observation.
    pub produces: &'static [&'static str],
    /// Structured JSON keys and the binding kind each yields.
    pub keys: &'static [(&'static str, &'static str)],
    /// Platform for handles read from `keys` when the record names none.
    pub account_platform: &'static str,
    /// The observation describes the account the tool was given (a profile lookup), so
    /// the accounts it links belong to the subject.
    pub profile_of_input: bool,
    /// One call per platform with a handle (SociaVault), within the turn budget.
    pub per_platform: bool,
    /// Optional inputs the binder fills when a binding exists (never required).
    pub optional: &'static [Fill],
    /// Handles this tool finds are search results: `unverified` until confirmed.
    pub unverified_handles: bool,
    /// Directive context kind (`news` or `legal`) a context tool serves. Context tools
    /// produce no bindings, only evidence, and serve only directives targeting the kind.
    pub context: &'static str,
}

const fn fill(kind: &'static str, input: &'static str) -> Fill {
    Fill { kind, input, how: How::Plain }
}

const fn fill_how(kind: &'static str, input: &'static str, how: How) -> Fill {
    Fill { kind, input, how }
}

const fn row(tool: &'static str, slots: &'static [Slot], produces: &'static [&'static str]) -> ToolIo {
    ToolIo {
        tool,
        slots,
        extras: &[],
        after: &[],
        produces,
        keys: &[],
        account_platform: "",
        profile_of_input: false,
        per_platform: false,
        optional: &[],
        unverified_handles: false,
        context: "",
    }
}

const DOMAIN: Slot = Slot { fills: &[fill("domain", "domain")] };
const ENTITY_PHRASE: Slot = Slot { fills: &[fill_how(QUERY_KIND, "query", How::EntityPhrase)] };
const IP: Slot = Slot { fills: &[fill("ip", "ip")] };
const CVE: Slot = Slot { fills: &[fill("cve", "cve_id")] };
const WALLET: Slot = Slot { fills: &[fill("wallet", "bitcoin_address")] };
const CODE_QUERY: Slot = Slot {
    fills: &[
        fill_how("handle", "query", How::Username("github")),
        fill_how("package", "query", How::PackageName),
        fill("org_name", "query"),
        fill("person_name", "query"),
    ],
};
const SEARCH_PRODUCES: &[&str] = &[
    "domain", "url", "handle", "person_name", "org_name", "email", "ip", "cve", "wallet", "package",
];
const PAGE_PRODUCES: &[&str] = &[
    "domain", "url", "handle", "email", "ip", "cve", "wallet", "package", "address",
];
const IP_SEEDS: &[&str] = &["mnemonic_passive_dns", "hackertarget_hostsearch"];
const HANDLE_SEEDS: &[&str] = &["firecrawl_search", "sociavault_profile"];
/// Producers of a domain the subject owns, for Hunter and Firecrawl map and crawl.
const DOMAIN_SEEDS: &[&str] = &["firecrawl_search", "hunter_domain_finder"];
/// Producers of an email for the Hunter email tools.
const EMAIL_SEEDS: &[&str] = &["firecrawl_search", "hunter_domain_search"];
const SOCIAL_SEARCH_QUERY: &[Fill] = &[
    fill_how("person_name", "query", How::PlatformQuery("sociavault_search")),
    fill_how("org_name", "query", How::PlatformQuery("sociavault_search")),
    fill_how("handle", "query", How::PlatformQuery("sociavault_search")),
];
const SOCIAL_USER_QUERY: &[Fill] = &[
    fill_how("person_name", "query", How::PlatformQuery("sociavault_search_users")),
    fill_how("org_name", "query", How::PlatformQuery("sociavault_search_users")),
    fill_how("handle", "query", How::PlatformQuery("sociavault_search_users")),
];
const HUNTER_PERSON_KEYS: &[(&str, &str)] = &[
    ("full_name", "person_name"),
    ("employer", "org_name"),
    ("employer_domain", "domain"),
    ("handle", "handle"),
];
const HUNTER_COMBINED_KEYS: &[(&str, &str)] = &[
    ("full_name", "person_name"),
    ("employer", "org_name"),
    ("employer_domain", "domain"),
    ("handle", "handle"),
    ("name", "org_name"),
    ("legal_name", "org_name"),
    ("domain", "domain"),
    ("address", "address"),
];

/// The tool input table. Coverage is asserted against `osint::registry()` in tests.
pub const TOOLS: &[ToolIo] = &[
    ToolIo { after: &["firecrawl_search"], keys: &[("name_value", "domain"), ("common_name", "domain")], ..row("crtsh_certificates", &[DOMAIN], &["domain"]) },
    ToolIo { after: &["firecrawl_search"], ..row("mnemonic_passive_dns", &[Slot { fills: &[fill("domain", "domain_or_ip"), fill("ip", "domain_or_ip")] }], &["domain", "ip"]) },
    ToolIo { after: &["firecrawl_search"], ..row("hackertarget_hostsearch", &[DOMAIN], &["domain", "ip"]) },
    ToolIo { after: IP_SEEDS, keys: &[("holder", "org_name")], ..row("ripestat_network_info", &[IP], &["org_name"]) },
    ToolIo { after: IP_SEEDS, ..row("arin_rdap", &[IP], &["email", "domain"]) },
    ToolIo { after: IP_SEEDS, ..row("apnic_rdap", &[IP], &["email", "domain"]) },
    row("wayback_availability", &[Slot { fills: &[fill(URL_KIND, "url"), fill_how("domain", "url", How::DomainAsUrl)] }], &[]),
    row("commoncrawl_urls", &[DOMAIN], &["url", "domain"]),
    row("arquivo_history", &[Slot { fills: &[fill("domain", "domain_or_url"), fill(URL_KIND, "domain_or_url")] }], &["url"]),
    ToolIo { keys: &[("login", "handle")], account_platform: "github", ..row("github_repositories", &[CODE_QUERY], &["handle", "url"]) },
    ToolIo { keys: &[("username", "handle")], ..row("gitlab_projects", &[CODE_QUERY], &["url"]) },
    row("grepapp_code_search", &[Slot { fills: &[fill_how("package", "query", How::PackageName), fill("domain", "query"), fill("org_name", "query")] }], &["email", "domain"]),
    ToolIo { keys: &[("legalName", "org_name"), ("legalAddress", "address"), ("headquartersAddress", "address")], ..row("gleif_entities", &[Slot { fills: &[fill("org_name", "company_name")] }], &["org_name", "address"]) },
    ToolIo { keys: &[("business", "address"), ("mailing", "address")], ..row("sec_submissions", &[Slot { fills: &[fill("org_name", "name")] }], &["address"]) },
    row("wikidata_entities", &[Slot { fills: &[fill("org_name", "name"), fill("person_name", "name")] }], &["url"]),
    ToolIo {
        after: HANDLE_SEEDS,
        keys: &[("nametag", "handle"), ("username", "handle")],
        account_platform: "keybase",
        profile_of_input: true,
        ..row("keybase_identity", &[Slot { fills: &[fill_how("handle", "username", How::Username("keybase")), fill("domain", "domain")] }], &["handle", "domain", "url"])
    },
    row("stackexchange_users", &[Slot { fills: &[fill("person_name", "name"), fill_how("handle", "name", How::Username("stackexchange"))] }], &["url", "domain"]),
    ToolIo { after: HANDLE_SEEDS, profile_of_input: true, ..row("wikipedia_users", &[Slot { fills: &[fill_how("handle", "username", How::Username("wikipedia"))] }], &[]) },
    ToolIo { keys: &[("display_name", "address")], ..row("nominatim_geocode", &[Slot { fills: &[fill("address", "address_or_place")] }], &["coordinates", "address"]) },
    ToolIo { keys: &[("matchedAddress", "address")], ..row("census_geocode", &[Slot { fills: &[fill("address", "us_address")] }], &["coordinates", "address"]) },
    ToolIo {
        extras: &[("radius_m", 500)],
        after: &["nominatim_geocode", "census_geocode"],
        ..row("overpass_places", &[Slot { fills: &[fill_how(COORDINATES_KIND, "latitude", How::Coordinates)] }], &[])
    },
    row("blockchain_address", &[WALLET], &[]),
    row("blockstream_address", &[WALLET], &[]),
    row("mempool_address", &[WALLET], &[]),
    row("nvd_cve", &[CVE], &["cve", "url"]),
    row("osv_package", &[Slot { fills: &[fill_how("package", "ecosystem", How::PackageParts)] }], &["cve"]),
    row("cve_record", &[CVE], &["cve", "url"]),
    row("sans_ip_activity", &[IP], &[]),
    row("shodan_internetdb", &[IP], &["domain", "cve"]),
    row("urlscan_search", &[DOMAIN], &["domain", "ip", "url"]),
    ToolIo { extras: &[("limit", 5)], ..row("firecrawl_search", &[Slot { fills: &[fill_how(QUERY_KIND, "query", How::SearchQuery)] }], SEARCH_PRODUCES) },
    ToolIo { after: &["firecrawl_search"], ..row("firecrawl_scrape", &[Slot { fills: &[fill(URL_KIND, "url")] }], PAGE_PRODUCES) },
    ToolIo { after: DOMAIN_SEEDS, ..row("firecrawl_map", &[DOMAIN], &["url"]) },
    ToolIo { after: &["firecrawl_map", "firecrawl_search"], ..row("firecrawl_batch_scrape", &[Slot { fills: &[fill_how(URL_KIND, "urls", How::UrlList)] }], PAGE_PRODUCES) },
    ToolIo { after: DOMAIN_SEEDS, ..row("firecrawl_crawl", &[DOMAIN], PAGE_PRODUCES) },
    ToolIo {
        after: &["firecrawl_map", "firecrawl_search"],
        keys: &[("org_name", "org_name"), ("legal_name", "org_name"), ("domain", "domain"), ("address", "address"), ("name", "person_name")],
        profile_of_input: true,
        ..row("firecrawl_extract", &[Slot { fills: &[fill(URL_KIND, "url")] }], &["org_name", "person_name", "domain", "email", "handle", "address", "url"])
    },
    ToolIo {
        after: &["firecrawl_search"],
        keys: &[("domain", "domain"), ("company_name", "org_name")],
        ..row("hunter_domain_finder", &[Slot { fills: &[fill("org_name", "company")] }], &["domain", "org_name"])
    },
    ToolIo { after: DOMAIN_SEEDS, ..row("hunter_email_count", &[Slot { fills: &[fill("domain", "domain"), fill("org_name", "company")] }], &[]) },
    ToolIo {
        after: DOMAIN_SEEDS,
        keys: &[("organization", "org_name")],
        ..row("hunter_domain_search", &[Slot { fills: &[fill("domain", "domain"), fill("org_name", "company")] }], &["email", "person_name", "domain", "org_name"])
    },
    ToolIo {
        after: DOMAIN_SEEDS,
        keys: &[("name", "org_name"), ("legal_name", "org_name"), ("handle", "handle"), ("domain", "domain"), ("parent_domain", "domain"), ("address", "address")],
        profile_of_input: true,
        ..row("hunter_company_enrichment", &[DOMAIN], &["org_name", "domain", "handle", "email", "address"])
    },
    ToolIo {
        after: &["firecrawl_search", "hunter_domain_search"],
        ..row(
            "hunter_email_finder",
            &[Slot { fills: &[fill("domain", "domain"), fill("org_name", "company")] }, Slot { fills: &[fill("person_name", "full_name")] }],
            &["email"],
        )
    },
    ToolIo { after: &["hunter_email_finder", "hunter_domain_search"], ..row("hunter_email_verifier", &[Slot { fills: &[fill("email", "email")] }], &[]) },
    ToolIo { after: EMAIL_SEEDS, ..row("hunter_email_insight", &[Slot { fills: &[fill("email", "email")] }], &[]) },
    ToolIo {
        after: EMAIL_SEEDS,
        keys: HUNTER_PERSON_KEYS,
        profile_of_input: true,
        ..row(
            "hunter_person_enrichment",
            &[Slot { fills: &[fill_how("email", "email", How::WebmailEmail), fill_how("handle", "linkedin_handle", How::PlatformHandle("linkedin"))] }],
            &["person_name", "org_name", "domain", "handle"],
        )
    },
    ToolIo {
        after: EMAIL_SEEDS,
        keys: HUNTER_COMBINED_KEYS,
        profile_of_input: true,
        ..row("hunter_combined_enrichment", &[Slot { fills: &[fill_how("email", "email", How::CompanyEmail)] }], &["person_name", "org_name", "domain", "handle", "address", "email"])
    },
    ToolIo {
        after: &["firecrawl_search", "hunter_company_enrichment", "sociavault_search_users"],
        keys: &[("platform_id", PLATFORM_ID_KIND)],
        profile_of_input: true,
        per_platform: true,
        ..row(
            "sociavault_profile",
            &[Slot { fills: &[fill_how("handle", "handle", How::SocialPair("sociavault_profile")), fill_how(PLATFORM_ID_KIND, "user_id", How::PlatformIdPair("sociavault_profile"))] }],
            &["handle", "url", "domain", "email", PLATFORM_ID_KIND],
        )
    },
    ToolIo {
        optional: &[fill_how(QUERY_KIND, "subreddit", How::Subreddit)],
        unverified_handles: true,
        ..row("sociavault_search", &[Slot { fills: SOCIAL_SEARCH_QUERY }], &["handle", "url"])
    },
    ToolIo { unverified_handles: true, ..row("sociavault_search_users", &[Slot { fills: SOCIAL_USER_QUERY }], &["handle"]) },
    ToolIo {
        after: &["sociavault_profile"],
        per_platform: true,
        ..row(
            "sociavault_user_content",
            &[Slot { fills: &[fill_how("handle", "handle", How::SocialPair("sociavault_user_content")), fill_how(PLATFORM_ID_KIND, "user_id", How::PlatformIdPair("sociavault_user_content"))] }],
            &["handle", "url", "domain", "email"],
        )
    },
    // Context tools (#29): the entity alone, dates only when the prompt writes one, and no
    // bindings: their URLs are evidence and citations only (D1 unchanged).
    ToolIo { context: NEWS_KIND, optional: &[fill_how(QUERY_KIND, "from", How::PromptDate(true)), fill_how(QUERY_KIND, "to", How::PromptDate(false))], ..row("newsapi_search", &[ENTITY_PHRASE], &[]) },
    ToolIo { context: NEWS_KIND, ..row("newsapi_headlines", &[ENTITY_PHRASE], &[]) },
    ToolIo { context: LEGAL_KIND, optional: &[fill_how(QUERY_KIND, "filed_after", How::PromptDate(true)), fill_how(QUERY_KIND, "filed_before", How::PromptDate(false))], ..row("courtlistener_case_search", &[ENTITY_PHRASE], &[]) },
    ToolIo { context: LEGAL_KIND, optional: &[fill_how(QUERY_KIND, "filed_after", How::PromptDate(true))], ..row("courtlistener_docket_search", &[ENTITY_PHRASE], &[]) },
    ToolIo { context: LEGAL_KIND, ..row("courtlistener_judge_search", &[ENTITY_PHRASE], &[]) },
    // No declared producer: Recon inserts it bound, after a weak Firecrawl search (D3).
    row("sociavault_google_search", &[Slot { fills: &[fill_how(QUERY_KIND, "query", How::SearchQuery)] }], &["domain", "url", "handle", "person_name", "org_name", "email"]),
];

pub fn tool_row(tool_id: &str) -> Option<&'static ToolIo> {
    let tool_id = crate::osint::canonical_tool_id(tool_id);
    TOOLS.iter().find(|row| row.tool == tool_id)
}

/// Tools the picker may be offered: in the table, with at least one input slot.
pub fn pickable(tool_id: &str) -> bool {
    tool_row(tool_id).is_some_and(|row| !row.slots.is_empty())
}

/// Binding kinds a tool can take. A search query takes any kind the prompt can name.
pub fn input_kinds(tool_id: &str) -> Vec<&'static str> {
    let mut kinds = Vec::new();
    for slot in tool_row(tool_id).map(|row| row.slots).unwrap_or(&[]) {
        for fill in slot.fills {
            let extra: &[&'static str] = match fill.kind {
                QUERY_KIND => &["person_name", "org_name", "domain", "handle", "email", "address", "cve", "ip", "wallet", "package"],
                kind if matches!(fill.how, How::SocialPair(_) | How::PlatformIdPair(_)) => {
                    if kind == "handle" { &["platform", "handle"] } else { &["platform", PLATFORM_ID_KIND] }
                }
                _ => std::slice::from_ref(&fill.kind),
            };
            for kind in extra {
                if !kinds.contains(kind) {
                    kinds.push(*kind);
                }
            }
        }
    }
    kinds
}

/// Binding kinds a tool's observation yields (a handle carries its platform).
pub fn output_kinds(tool_id: &str) -> Vec<&'static str> {
    let mut kinds: Vec<&'static str> = tool_row(tool_id).map(|row| row.produces.to_vec()).unwrap_or_default();
    if kinds.contains(&"handle") {
        kinds.push("platform");
    }
    kinds
}

/// Tools whose observation the binder parses for `kind`.
#[cfg_attr(not(test), allow(dead_code))]
pub fn producers_of(kind: &str) -> Vec<&'static str> {
    TOOLS.iter().filter(|row| row.produces.contains(&kind)).map(|row| row.tool).collect()
}

/// Which extractor yields `kind` from an observation, for the audit table and coverage.
#[cfg_attr(not(test), allow(dead_code))]
pub fn extractor(kind: &str) -> &'static str {
    match kind {
        "domain" => "domain scanner (registrable TLD; social, publisher, and file hosts dropped) and entity selection on search hits",
        "ip" => "IPv4/IPv6 scanner",
        "email" => "email scanner",
        "url" => "URL scanner (subject-related or non-social hosts)",
        "cve" => "CVE id scanner",
        "wallet" => "Bitcoin address scanner",
        "package" => "`ecosystem name@version` scanner",
        "handle" => "profile-URL and @mention-near-platform extractor, plus keyed account fields; subject-owned only",
        "platform" => "paired with each handle",
        "person_name" | "org_name" => "entity selection on search hits and keyed name fields",
        "address" => "keyed address fields",
        "coordinates" => "lat/lon fields",
        PLATFORM_ID_KIND => "keyed `platform_id` on a SociaVault profile card (Twitter rest_id, Instagram user id, YouTube channelId)",
        QUERY_KIND => "always available",
        _ => "",
    }
}

// ---------------------------------------------------------------------------
// Binder
// ---------------------------------------------------------------------------

/// Person names are two to four words and never a tool or product description.
pub fn plausible_person_name(value: &str) -> bool {
    const PRODUCT_WORDS: &[&str] = &[
        "scraper", "scrapers", "archive", "archives", "api", "apis", "dataset", "datasets", "tool",
        "tools", "bot", "bots", "crawler", "downloader", "tracker", "generator", "database",
        "extension", "plugin", "script", "sdk", "template", "app", "apps", "actor", "repository",
        "github", "social", "media", "account", "accounts", "profile", "profiles", "life",
        "context", "refer", "recon",
    ];
    let words: Vec<String> = value
        .split_whitespace()
        .map(|word| word.trim_matches(|ch: char| !ch.is_alphanumeric()).to_ascii_lowercase())
        .filter(|word| !word.is_empty())
        .collect();
    (1..=4).contains(&words.len())
        && !words.iter().any(|word| PRODUCT_WORDS.contains(&word.as_str()))
        && !value.contains(['/', '@', ':', '|'])
}

fn usable(binding: &Binding) -> bool {
    match binding.kind.as_str() {
        "domain" => !social_or_publisher(&binding.value) && !crate::osint::webmail_host(&binding.value),
        "person_name" => plausible_person_name(&binding.value),
        _ => true,
    }
}

/// Exact bindings before inferred ones, in the order found.
fn candidates<'a>(bindings: &'a [Binding], kind: &str) -> Vec<&'a Binding> {
    let mut found: Vec<&Binding> = bindings.iter().filter(|binding| binding.kind == kind && usable(binding)).collect();
    found.sort_by_key(|binding| (binding.inferred, binding.unverified));
    found
}

/// The handle with the most support for the subject: observed in evidence before named
/// only in a question, then names the subject, then seen on the most platforms or
/// observations, then first found. Inferred pairings never count.
pub fn best_handle<'a>(bindings: &'a [Binding], subject: &str) -> Option<&'a Binding> {
    let handles: Vec<&Binding> = bindings.iter().filter(|binding| binding.kind == "handle" && !binding.inferred).collect();
    let support = |value: &str| handles.iter().filter(|other| other.value.eq_ignore_ascii_case(value) && !other.unverified).count();
    handles
        .iter()
        .enumerate()
        .max_by_key(|(index, binding)| {
            (!binding.unverified, names_subject(subject, &binding.value), support(&binding.value), usize::MAX - index)
        })
        .map(|(_, binding)| *binding)
}

/// The handle bound for `platform`: an observed one first, then one a question named.
fn platform_handle<'a>(bindings: &'a [Binding], platform: &str) -> Option<&'a Binding> {
    let on = |binding: &&Binding| binding.kind == "handle" && binding.qualifier == platform && !binding.inferred;
    bindings.iter().filter(on).find(|binding| !binding.unverified).or_else(|| bindings.iter().find(on))
}

/// `ecosystem:name@version` as (ecosystem, name, version).
pub fn package_parts(value: &str) -> Option<(String, String, String)> {
    let (ecosystem, rest) = value.split_once(':')?;
    let at = rest.rfind('@').filter(|index| *index > 0)?;
    let (name, version) = (&rest[..at], &rest[at + 1..]);
    (!ecosystem.is_empty() && !name.is_empty() && !version.is_empty())
        .then(|| (ecosystem.to_string(), name.to_string(), version.to_string()))
}

fn coordinate_parts(value: &str) -> Option<(f64, f64)> {
    let (lat, lon) = value.split_once(',')?;
    let (lat, lon) = (lat.trim().parse::<f64>().ok()?, lon.trim().parse::<f64>().ok()?);
    ((-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon)).then_some((lat, lon))
}

struct Chosen {
    args: Vec<(&'static str, Value)>,
    filled: Vec<String>,
    /// Grounding source per argument, in `args` order.
    sources: Vec<String>,
}

/// Grounding source of a binding: the prompt, a directive that named it, or the call
/// whose observation yielded it.
fn ground(binding: &Binding) -> String {
    match binding.evidence_id.as_str() {
        "question" => PROMPT_SOURCE.to_string(),
        id if id.starts_with('d') && id.len() == 2 => format!("{id} directive"),
        id => format!("binding {id}"),
    }
}

/// Fixed tool defaults and named platforms.
pub const FIXED_SOURCE: &str = "fixed";

fn source(binding: &Binding) -> String {
    let via = if binding.source_tool.is_empty() { String::new() } else { format!(" via {}", binding.source_tool) };
    if binding.inferred {
        format!("{} inferred for {} from {}{via}", binding.kind, binding.qualifier, binding.evidence_id)
    } else if binding.unverified {
        format!("{} named in {}, unverified{via}", binding.kind, binding.evidence_id)
    } else {
        format!("{} from {}{via}", binding.kind, binding.evidence_id)
    }
}

/// What `choose` reads besides the bindings.
struct Ctx<'a> {
    subject: &'a str,
    hint: &'a str,
    question: &'a str,
    directive: Option<&'a Directive>,
}

impl Ctx<'_> {
    /// Source label for a value named in the directive goal or the prompt.
    fn named_source(&self, named_in_hint: bool) -> String {
        match (named_in_hint, self.directive) {
            (true, Some(directive)) => format!("{} directive", directive.id),
            _ => PROMPT_SOURCE.to_string(),
        }
    }
}

/// The platform a SociaVault query tool searches: the first one the question or hint
/// names that the tool serves, else the handle's own platform, else the tool's default.
fn query_platform(tool: &str, ctx: &Ctx, binding: &Binding) -> &'static str {
    let served = crate::osint::sociavault_platforms(tool);
    let texts = [(String::new(), format!("{} {}", ctx.hint, ctx.question))];
    let named = question_platforms(&texts).into_iter().find_map(|(platform, _)| served.iter().copied().find(|known| *known == platform));
    let own = served.iter().copied().find(|known| binding.kind == "handle" && *known == binding.qualifier);
    let reddit = served.contains(&"reddit") && (subreddit_in(ctx.hint).is_some() || subreddit_in(ctx.question).is_some());
    reddit.then_some("reddit").or(named).or(own).unwrap_or(if tool == "sociavault_search_users" { "instagram" } else { "twitter" })
}

/// `query_platform` and where the platform came from: the directive goal, the prompt, the
/// handle's own platform, or the tool default.
fn query_platform_sourced(tool: &str, ctx: &Ctx, binding: &Binding) -> (&'static str, String) {
    let platform = query_platform(tool, ctx, binding);
    let served_in = |text: &str| {
        let texts = [(String::new(), text.to_string())];
        question_platforms(&texts).iter().any(|(named, _)| named == platform) || platform == "reddit" && subreddit_in(text).is_some()
    };
    let source = if served_in(ctx.hint) {
        ctx.named_source(true)
    } else if served_in(ctx.question) {
        PROMPT_SOURCE.to_string()
    } else if binding.kind == "handle" && binding.qualifier == platform {
        ground(binding)
    } else {
        FIXED_SOURCE.to_string()
    };
    (platform, source)
}

/// `r/<name>` named in the question or hint.
fn subreddit_in(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    let at = lower.find("r/")?;
    if at > 0 && lower.as_bytes()[at - 1].is_ascii_alphanumeric() {
        return None;
    }
    let name: String = lower[at + 2..].chars().take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_').collect();
    (2..=21).contains(&name.len()).then_some(name)
}

fn choose(fill: &Fill, bindings: &[Binding], ctx: &Ctx) -> Option<Chosen> {
    let (subject, hint) = (ctx.subject, ctx.hint);
    let plain = |input: &'static str, binding: &Binding, value: Value| Chosen {
        filled: vec![format!("{input}={} ({})", value.as_str().map(String::from).unwrap_or_else(|| value.to_string()), source(binding))],
        args: vec![(input, value)],
        sources: vec![ground(binding)],
    };
    match fill.how {
        How::Plain => {
            let binding = if fill.kind == "handle" {
                best_handle(bindings, subject)
            } else {
                candidates(bindings, fill.kind).into_iter().next()
            }?;
            Some(plain(fill.input, binding, json!(binding.value)))
        }
        How::DomainAsUrl => {
            let binding = candidates(bindings, "domain").into_iter().next()?;
            Some(plain(fill.input, binding, json!(format!("https://{}", binding.value))))
        }
        How::SocialPair(tool) | How::PlatformIdPair(tool) => {
            let served = crate::osint::sociavault_account_platforms(tool);
            let binding = candidates(bindings, fill.kind)
                .into_iter()
                .find(|binding| served.contains(&binding.qualifier.as_str()))?;
            let input = if fill.kind == "handle" { "handle" } else { "user_id" };
            Some(Chosen {
                args: vec![("platform", json!(binding.qualifier)), (input, json!(binding.value))],
                filled: vec![
                    format!("platform={} ({})", binding.qualifier, source(binding)),
                    format!("{input}={} ({})", binding.value, source(binding)),
                ],
                sources: vec![ground(binding), ground(binding)],
            })
        }
        How::PlatformQuery(tool) => {
            let binding = candidates(bindings, fill.kind).into_iter().find(|binding| !binding.inferred)?;
            let (platform, platform_source) = query_platform_sourced(tool, ctx, binding);
            Some(Chosen {
                args: vec![("platform", json!(platform)), (fill.input, json!(binding.value))],
                filled: vec![format!("platform={platform} ({platform_source})"), format!("{}={} ({})", fill.input, binding.value, source(binding))],
                sources: vec![platform_source, ground(binding)],
            })
        }
        How::PlatformHandle(platform) => {
            let binding = platform_handle(bindings, platform)?;
            Some(plain(fill.input, binding, json!(binding.value)))
        }
        How::UrlList => {
            let mut urls: Vec<&Binding> = candidates(bindings, URL_KIND);
            urls.sort_by_key(|binding| (binding.inferred, binding.unverified, crate::osint::map_rank(&binding.value)));
            urls.dedup_by(|a, b| a.value == b.value);
            urls.truncate(crate::osint::BATCH_SCRAPE_DEFAULT_URLS);
            let first = *urls.first()?;
            let mut evidence: Vec<String> = Vec::new();
            for binding in &urls {
                let label = ground(binding);
                if !evidence.contains(&label) {
                    evidence.push(label);
                }
            }
            Some(Chosen {
                args: vec![(fill.input, json!(urls.iter().map(|binding| binding.value.clone()).collect::<Vec<_>>()))],
                filled: vec![format!("{}={} URL(s) ({})", fill.input, urls.len(), source(first))],
                sources: vec![evidence.join(", ")],
            })
        }
        How::Subreddit => {
            let in_hint = subreddit_in(ctx.hint);
            let name = in_hint.clone().or_else(|| subreddit_in(ctx.question))?;
            Some(Chosen {
                filled: vec![format!("{}=r/{name} (named in the question)", fill.input)],
                args: vec![(fill.input, json!(name))],
                sources: vec![ctx.named_source(in_hint.is_some())],
            })
        }
        How::CompanyEmail | How::WebmailEmail => {
            let webmail = fill.how == How::WebmailEmail;
            let binding = candidates(bindings, "email").into_iter().find(|binding| {
                binding.value.rsplit_once('@').is_some_and(|(_, host)| crate::osint::webmail_host(&host.to_ascii_lowercase()) == webmail)
            })?;
            Some(plain(fill.input, binding, json!(binding.value)))
        }
        How::Username(platform) => {
            let binding = platform_handle(bindings, platform).or_else(|| best_handle(bindings, subject))?;
            Some(plain(fill.input, binding, json!(binding.value)))
        }
        How::PackageParts => {
            let (binding, (ecosystem, name, version)) = candidates(bindings, "package")
                .into_iter()
                .find_map(|binding| package_parts(&binding.value).map(|parts| (binding, parts)))?;
            Some(Chosen {
                args: vec![("ecosystem", json!(ecosystem)), ("package_name", json!(name)), ("version", json!(version))],
                filled: vec![format!("package={} ({})", binding.value, source(binding))],
                sources: vec![ground(binding); 3],
            })
        }
        How::PackageName => {
            let (binding, (_, name, _)) = candidates(bindings, "package")
                .into_iter()
                .find_map(|binding| package_parts(&binding.value).map(|parts| (binding, parts)))?;
            Some(plain(fill.input, binding, json!(name)))
        }
        How::Coordinates => {
            let (binding, (lat, lon)) = candidates(bindings, COORDINATES_KIND)
                .into_iter()
                .find_map(|binding| coordinate_parts(&binding.value).map(|parts| (binding, parts)))?;
            Some(Chosen {
                args: vec![("latitude", json!(lat)), ("longitude", json!(lon))],
                filled: vec![format!("coordinates={} ({})", binding.value, source(binding))],
                sources: vec![ground(binding); 2],
            })
        }
        How::EntityPhrase => match ctx.directive {
            Some(directive) => {
                let grounded = super::directive_query(directive, false)?;
                Some(Chosen {
                    filled: vec![format!("{}={} ({})", fill.input, grounded.query, grounded.source)],
                    args: vec![(fill.input, json!(grounded.query))],
                    sources: vec![grounded.source],
                })
            }
            None => {
                let hint = clip_query(hint);
                let query = if hint.is_empty() { subject.to_string() } else { hint };
                (!query.is_empty()).then(|| Chosen { args: vec![(fill.input, json!(query))], filled: Vec::new(), sources: vec![PROMPT_SOURCE.into()] })
            }
        },
        How::PromptDate(start) => {
            let (from, to) = prompt_dates(ctx.question);
            let date = if start { from } else { to }?;
            Some(Chosen {
                filled: vec![format!("{}={date} (date in the prompt)", fill.input)],
                args: vec![(fill.input, json!(date))],
                sources: vec![PROMPT_SOURCE.into()],
            })
        }
        How::SearchQuery => match ctx.directive {
            // The directive's entity plus its fixed qualifier, never question text.
            Some(directive) => {
                let grounded = super::directive_query(directive, true)?;
                Some(Chosen {
                    filled: vec![format!("{}={} ({})", fill.input, grounded.query, grounded.source)],
                    args: vec![(fill.input, json!(grounded.query))],
                    sources: vec![grounded.source],
                })
            }
            // Reachability checks only (`unmet_kinds`): a search input is always fillable.
            None => {
                let hint = clip_query(hint);
                let query = if hint.is_empty() { subject.to_string() } else { hint };
                (!query.is_empty()).then(|| Chosen { args: vec![(fill.input, json!(query))], filled: Vec::new(), sources: vec![PROMPT_SOURCE.into()] })
            }
        },
    }
}

/// Tool inputs one fill writes, for labels.
fn fill_names(fill: &Fill) -> &[&'static str] {
    match fill.how {
        How::SocialPair(_) => &["platform", "handle"],
        How::PlatformIdPair(_) => &["platform", "user_id"],
        How::PlatformQuery(_) => &["platform", "query"],
        How::PackageParts => &["ecosystem", "package_name", "version"],
        How::Coordinates => &["latitude", "longitude"],
        _ => std::slice::from_ref(&fill.input),
    }
}

fn slot_label(slot: &Slot) -> String {
    let names = fill_names;
    if slot.fills.len() == 1 {
        return names(&slot.fills[0]).join(", ");
    }
    // Inputs every alternative writes (`platform`), then the alternatives: "platform,
    // handle or user_id"; "domain or company".
    let mut common: Vec<&str> = Vec::new();
    let mut rest: Vec<&str> = Vec::new();
    for fill in slot.fills {
        for name in names(fill) {
            let everywhere = slot.fills.iter().all(|other| names(other).contains(name));
            let list = if everywhere { &mut common } else { &mut rest };
            if !list.contains(name) {
                list.push(name);
            }
        }
    }
    let alternatives = rest.join(" or ");
    if !alternatives.is_empty() {
        common.push(&alternatives);
    }
    common.join(", ")
}

/// One step's bound arguments: the arguments, the fills as `input=value (source)` for the
/// decision row, the inputs still missing, and the grounding of every argument as
/// `(input, value, source)`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Bound {
    pub args: Value,
    pub filled: Vec<String>,
    pub missing: Vec<String>,
    pub grounding: Vec<(String, Value, String)>,
}

/// The binding kind the directive's first entity can fill directly: `person_name` for a
/// person, `org_name` for an organization, or none (a topic or identifier).
pub fn entity_kind(question: &str, entity: &str) -> &'static str {
    let kind = super::target_kind(question);
    let words: Vec<String> = entity.split_whitespace().map(|word| word.trim_matches(|ch: char| !ch.is_alphanumeric()).to_ascii_lowercase()).collect();
    if kind == "organization" || words.iter().any(|word| super::ORG_WORDS.contains(&word.as_str())) {
        "org_name"
    } else if plausible_person_name(entity) && (kind == "person" || super::refers_back(question)) {
        "person_name"
    } else {
        ""
    }
}

/// Subject first: a slot whose input can take the directive's entity directly (a person
/// or organization name, or a search query) gets the entity, unless the slot leads with
/// an identifier (handle, domain, package, …) a binding already fills. Later bindings only
/// fill what the entity cannot.
fn entity_choice(slot: &Slot, eligible: &[Binding], ctx: &Ctx, tool: &str) -> Option<Chosen> {
    let directive = ctx.directive?;
    let entity = directive.entities.first()?;
    let lead = slot.fills.first()?;
    let name_like = |fill: &Fill| matches!(fill.kind, "person_name" | "org_name" | QUERY_KIND);
    if !name_like(lead) && choose(lead, eligible, ctx).is_some() {
        return None;
    }
    let kind = entity_kind(ctx.question, entity);
    let label = format!("{} entity", directive.id);
    for fill in slot.fills.iter().filter(|fill| name_like(fill)) {
        match fill.how {
            How::SearchQuery | How::EntityPhrase => return choose(fill, eligible, ctx),
            How::PlatformQuery(query_tool) => {
                // Platform searches send the entity alone.
                let grounded = super::directive_query(directive, false)?;
                let anchor = Binding { kind: String::new(), ..Binding::default() };
                let (platform, platform_source) = query_platform_sourced(query_tool, ctx, &anchor);
                return Some(Chosen {
                    args: vec![("platform", json!(platform)), (fill.input, json!(grounded.query))],
                    filled: vec![format!("platform={platform} ({platform_source})"), format!("{}={} ({})", fill.input, grounded.query, grounded.source)],
                    sources: vec![platform_source, grounded.source],
                });
            }
            How::Plain if !kind.is_empty() && fill.kind == kind => {
                let _ = tool;
                return Some(Chosen {
                    filled: vec![format!("{}={entity} ({label})", fill.input)],
                    args: vec![(fill.input, json!(entity))],
                    sources: vec![label],
                });
            }
            _ => {}
        }
    }
    None
}

/// Arguments for one step from the directive it serves and accepted bindings. Reads
/// `TOOLS` only; never passes a social or news host where a domain is expected. Without a
/// directive, the fixed directive for the question that the tool serves is used.
pub fn bind_step(tool_id: &str, bindings: &[Binding], question: &str, directive: Option<&Directive>) -> Bound {
    let mut bound = Bound::default();
    let mut args = serde_json::Map::new();
    let Some(row) = tool_row(tool_id) else {
        bound.args = Value::Object(args);
        bound.missing.push(format!("{tool_id} inputs have no binding kind"));
        return bound;
    };
    let fixed;
    let directive = match directive {
        Some(directive) => Some(directive),
        None => {
            fixed = super::fallback_directives(question, &[]);
            let evidence = evidence_kinds(row.tool);
            fixed.iter().find(|item| item.targets.iter().any(|kind| evidence.contains(&kind.as_str()))).or_else(|| fixed.first())
        }
    };
    let subject = subject_of(question);
    let eligible: Vec<Binding> = bindings.iter().filter(|binding| binding_allowed(row.tool, binding)).cloned().collect();
    let hint = directive.map(|item| item.goal.clone()).unwrap_or_default();
    let ctx = Ctx { subject: &subject, hint: &hint, question, directive };
    let mut sources: Vec<(String, String)> = Vec::new();
    let take = |chosen: Chosen, args: &mut serde_json::Map<String, Value>, overwrite: bool, sources: &mut Vec<(String, String)>, filled: &mut Vec<String>| {
        for (index, (input, value)) in chosen.args.into_iter().enumerate() {
            if !overwrite && args.contains_key(input) {
                continue;
            }
            args.insert(input.into(), value);
            sources.retain(|(known, _)| known != input);
            sources.push((input.to_string(), chosen.sources.get(index).cloned().unwrap_or_default()));
        }
        filled.extend(chosen.filled);
    };
    for slot in row.slots {
        match entity_choice(slot, &eligible, &ctx, row.tool).or_else(|| slot.fills.iter().find_map(|fill| choose(fill, &eligible, &ctx))) {
            Some(chosen) => take(chosen, &mut args, true, &mut sources, &mut bound.filled),
            None => bound.missing.push(slot_label(slot)),
        }
    }
    for fill in row.optional {
        if let Some(chosen) = choose(fill, &eligible, &ctx) {
            take(chosen, &mut args, false, &mut sources, &mut bound.filled);
        }
    }
    for (input, value) in row.extras {
        if !args.contains_key(*input) {
            args.insert(input.to_string(), json!(value));
            sources.push((input.to_string(), FIXED_SOURCE.into()));
        }
    }
    if crate::osint::SOCIAVAULT_TOOLS.contains(&row.tool) && bound.missing.is_empty() {
        let platform = args.get("platform").and_then(Value::as_str).unwrap_or("").to_string();
        let mut text = format!("{hint} {question}");
        if args.contains_key("subreddit") {
            text.push_str(" r/");
        }
        let has_user_id = args.contains_key("user_id");
        if let Some(endpoint) = crate::osint::sociavault_endpoint_hint(row.tool, &platform, &text) {
            let route = crate::osint::sociavault_routes(row.tool).find(|route| route.platform == platform && route.endpoint == endpoint);
            // Only an endpoint whose inputs the bound arguments carry.
            let fits = route.is_some_and(|route| route.input.keys().iter().any(|key| args.contains_key(*key) || (*key == "user_id" && has_user_id)));
            if fits {
                let in_hint = crate::osint::sociavault_endpoint_hint(row.tool, &platform, &hint) == Some(endpoint);
                bound.filled.push(format!("endpoint={endpoint} (named in the question)"));
                args.insert("endpoint".into(), json!(endpoint));
                sources.push(("endpoint".into(), ctx.named_source(in_hint)));
            }
        }
    }
    bound.grounding = sources
        .into_iter()
        .filter_map(|(input, source)| args.get(&input).map(|value| (input, value.clone(), source)))
        .collect();
    bound.args = Value::Object(args);
    bound
}

/// Kinds a tool reports evidence about: what it produces, or for a tool that produces no
/// bindings (a verifier, a balance lookup), the kinds it takes.
pub fn evidence_kinds(tool_id: &str) -> Vec<&'static str> {
    if let Some(kind) = context_of(tool_id) {
        return vec![kind];
    }
    let produced = output_kinds(tool_id);
    if produced.is_empty() {
        input_kinds(tool_id)
    } else {
        produced
    }
}

/// `bind_step` as `(arguments, fills, missing)`.
pub fn bind_arguments(tool_id: &str, bindings: &[Binding], question: &str, directive: Option<&Directive>) -> (Value, Vec<String>, Vec<String>) {
    let bound = bind_step(tool_id, bindings, question, directive);
    (bound.args, bound.filled, bound.missing)
}

/// Kinds of each slot the bindings cannot fill yet.
pub fn unmet_kinds(tool_id: &str, bindings: &[Binding]) -> Vec<Vec<&'static str>> {
    let Some(row) = tool_row(tool_id) else {
        return Vec::new();
    };
    let eligible: Vec<Binding> = bindings.iter().filter(|binding| binding_allowed(row.tool, binding)).cloned().collect();
    let ctx = Ctx { subject: "", hint: "-", question: "", directive: None };
    row.slots
        .iter()
        .filter(|slot| !slot.fills.iter().any(|fill| choose(fill, &eligible, &ctx).is_some()))
        .map(|slot| {
            let mut kinds: Vec<&'static str> = Vec::new();
            for fill in slot.fills {
                if !kinds.contains(&fill.kind) {
                    kinds.push(fill.kind);
                }
            }
            kinds
        })
        .collect()
}

/// Declared needs the bindings do not satisfy yet, as `kind or kind`.
pub fn unmet_needs(tool_id: &str, bindings: &[Binding]) -> Vec<String> {
    unmet_kinds(tool_id, bindings).into_iter().map(|kinds| kinds.join(" or ")).collect()
}

/// One declared dependency: the slots' kinds and the producers that must come first.
pub struct Dependency {
    pub tool: &'static str,
    pub needs: Vec<Vec<&'static str>>,
    pub producers: &'static [&'static str],
}

pub fn dependency(tool_id: &str) -> Option<Dependency> {
    let row = tool_row(tool_id).filter(|row| !row.after.is_empty())?;
    Some(Dependency {
        tool: row.tool,
        needs: row
            .slots
            .iter()
            .map(|slot| slot.fills.iter().map(|fill| fill.kind).filter(|kind| *kind != QUERY_KIND).collect())
            .collect(),
        producers: row.after,
    })
}

pub fn dependencies() -> Vec<Dependency> {
    TOOLS.iter().filter_map(|row| dependency(row.tool)).collect()
}

/// Picker catalog inputs: each slot's tool inputs and the binding kinds that fill them.
pub fn catalog_inputs(tool_id: &str) -> Vec<String> {
    tool_row(tool_id)
        .map(|row| {
            row.slots
                .iter()
                .map(|slot| {
                    let kinds: Vec<&str> = slot.fills.iter().map(|fill| fill.kind).collect();
                    format!("{} <- {}", slot_label(slot), kinds.join(" | "))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Tools that consume `kind`, in table order. The deterministic ladder reads this.
pub fn consumers_of(kind: &str) -> Vec<&'static str> {
    TOOLS
        .iter()
        .filter(|row| row.slots.iter().any(|slot| slot.fills.iter().any(|fill| fill.kind == kind)))
        .map(|row| row.tool)
        .collect()
}

// ---------------------------------------------------------------------------
// Rule extraction
// ---------------------------------------------------------------------------

pub(crate) fn strings_in(value: &Value, out: &mut Vec<String>) {
    if out.len() >= 600 {
        return;
    }
    match value {
        Value::String(text) => out.push(text.chars().take(2_000).collect()),
        Value::Array(items) => items.iter().for_each(|item| strings_in(item, out)),
        Value::Object(map) => map.values().for_each(|item| strings_in(item, out)),
        _ => {}
    }
}

pub(crate) fn hits_in(evidence_id: &str, observations: &Value) -> Vec<SearchHit> {
    observations
        .get("results")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|row| SearchHit {
            evidence_id: evidence_id.into(),
            title: row.get("title").and_then(Value::as_str).unwrap_or("").into(),
            url: row.get("url").and_then(Value::as_str).unwrap_or("").into(),
            snippet: row
                .get("snippet")
                .or_else(|| row.get("description"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .into(),
            retrieved_at: String::new(),
            query_role: ACCOUNTS.into(),
        })
        .filter(|hit| !hit.url.is_empty())
        .collect()
}

fn registrable_domain(token: &str) -> bool {
    const FILES: &[&str] = &[
        "png", "jpg", "jpeg", "gif", "svg", "webp", "pdf", "js", "css", "json", "xml", "html", "htm",
        "php", "asp", "aspx", "txt", "csv", "zip", "exe", "md",
    ];
    let lower = token.to_ascii_lowercase();
    let labels: Vec<&str> = lower.split('.').collect();
    labels.len() >= 2
        && labels.iter().all(|label| !label.is_empty() && label.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))
        && labels.last().is_some_and(|tld| tld.len() >= 2 && tld.bytes().all(|b| b.is_ascii_alphabetic()) && !FILES.contains(tld))
}

/// Domains in free text with a registrable TLD.
pub fn domains_in(text: &str) -> Vec<String> {
    super::super::explicit_entities(text)
        .into_iter()
        .filter(|(kind, value)| kind == "domain" && registrable_domain(value))
        .map(|(_, value)| value)
        .collect()
}

fn kind_in(text: &str, wanted: &str) -> Vec<String> {
    super::super::explicit_entities(text)
        .into_iter()
        .filter(|(kind, _)| kind == wanted)
        .map(|(_, value)| value)
        .collect()
}

/// Every Bitcoin address in the text (legacy, P2SH, and bech32 forms).
pub fn bitcoins_in(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|token| token.trim_matches(|ch: char| !ch.is_ascii_alphanumeric()))
        .filter(|token| {
            (26..=62).contains(&token.len())
                && token.bytes().all(|b| b.is_ascii_alphanumeric())
                && (token.starts_with('1') || token.starts_with('3') || token.starts_with("bc1"))
        })
        .map(String::from)
        .collect()
}

const ECOSYSTEMS: &[(&str, &str)] = &[
    ("npm", "npm"), ("node", "npm"), ("pypi", "PyPI"), ("pip", "PyPI"), ("python", "PyPI"),
    ("crates.io", "crates.io"), ("crates", "crates.io"), ("cargo", "crates.io"), ("rust", "crates.io"),
    ("golang", "Go"), ("go", "Go"), ("maven", "Maven"), ("rubygems", "RubyGems"), ("gem", "RubyGems"),
    ("ruby", "RubyGems"), ("nuget", "NuGet"), ("packagist", "Packagist"), ("composer", "Packagist"),
];

fn version_like(token: &str) -> bool {
    let token = token.trim_start_matches('v');
    token.chars().next().is_some_and(|ch| ch.is_ascii_digit())
        && token.contains('.')
        && token.chars().all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '+'))
}

/// Packages as `ecosystem:name@version`, from `name@version` or `name version` next to an
/// ecosystem word ("lodash 4.17.20 on npm", "PyPI requests==2.19.0").
pub fn packages_in(text: &str) -> Vec<String> {
    let tokens: Vec<&str> = text
        .split(|ch: char| ch.is_whitespace() || matches!(ch, ',' | ';' | '(' | ')' | '"' | '\''))
        .map(|token| token.trim_end_matches(['.', '?', '!']))
        .filter(|token| !token.is_empty())
        .collect();
    let ecosystem_at = |index: usize| -> Option<&'static str> {
        let lo = index.saturating_sub(4);
        let hi = (index + 5).min(tokens.len());
        tokens[lo..hi].iter().find_map(|token| {
            let lower = token.to_ascii_lowercase();
            ECOSYSTEMS.iter().find(|(word, _)| lower == *word).map(|(_, name)| *name)
        })
    };
    let mut found = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        let lower = token.to_ascii_lowercase();
        if ECOSYSTEMS.iter().any(|(word, _)| lower == *word) || matches!(lower.as_str(), "version" | "package" | "on" | "in" | "the") {
            continue;
        }
        let pair = if let Some((eco, rest)) = token.split_once(':').filter(|(eco, _)| ECOSYSTEMS.iter().any(|(_, name)| name.eq_ignore_ascii_case(eco))) {
            let at = rest.rfind('@').filter(|at| *at > 0);
            at.map(|at| (Some(ECOSYSTEMS.iter().find(|(_, name)| name.eq_ignore_ascii_case(eco)).map(|(_, name)| *name).unwrap_or("npm")), rest[..at].to_string(), rest[at + 1..].to_string()))
        } else if let Some(at) = token.rfind('@').filter(|at| *at > 0) {
            Some((None, token[..at].to_string(), token[at + 1..].to_string()))
        } else if let Some((name, version)) = token.split_once("==") {
            Some((None, name.to_string(), version.to_string()))
        } else if tokens.get(index + 1).is_some_and(|next| version_like(next)) && !version_like(token) {
            Some((None, token.to_string(), tokens[index + 1].to_string()))
        } else {
            None
        };
        let Some((eco, name, version)) = pair else { continue };
        let version = version.trim_start_matches('v').to_string();
        let name_ok = !name.is_empty()
            && name.len() <= 80
            && name.chars().all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '/' | '@' | ':'))
            && name.chars().any(|ch| ch.is_ascii_alphabetic())
            && !name.contains("://");
        if !name_ok || !version_like(&version) || name.contains('.') && registrable_domain(&name) {
            continue;
        }
        let Some(ecosystem) = eco.or_else(|| ecosystem_at(index)) else { continue };
        let value = format!("{ecosystem}:{name}@{version}");
        if !found.contains(&value) {
            found.push(value);
        }
    }
    found
}

/// `lat,lon` pairs written in text ("40.7128, -74.0060").
pub fn coordinates_in_text(text: &str) -> Vec<String> {
    let tokens: Vec<&str> = text.split(|ch: char| ch.is_whitespace() || ch == ',').filter(|t| !t.is_empty()).collect();
    let mut found = Vec::new();
    for pair in tokens.windows(2) {
        let (a, b) = (pair[0].trim_matches(['(', ')']), pair[1].trim_matches(['(', ')', '.', '?']));
        if a.contains('.') && b.contains('.') && coordinate_parts(&format!("{a},{b}")).is_some() {
            found.push(format!("{a},{b}"));
        }
    }
    found
}

/// Coordinates from geocoder records: `lat`/`lon`, `latitude`/`longitude`, or a
/// `coordinates` object with `x` (longitude) and `y` (latitude).
fn coordinates_in(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            let text = |key: &str| map.get(key).and_then(|value| match value {
                Value::String(text) => Some(text.clone()),
                Value::Number(number) => Some(number.to_string()),
                _ => None,
            });
            let pair = text("lat").zip(text("lon").or_else(|| text("lng")))
                .or_else(|| text("latitude").zip(text("longitude")))
                .or_else(|| map.get("coordinates").and_then(|inner| {
                    let get = |key: &str| inner.get(key).and_then(|v| v.as_f64().map(|n| n.to_string()).or_else(|| v.as_str().map(String::from)));
                    get("y").zip(get("x"))
                }));
            if let Some((lat, lon)) = pair {
                let value = format!("{lat},{lon}");
                if coordinate_parts(&value).is_some() && !out.contains(&value) {
                    out.push(value);
                }
            }
            map.values().for_each(|item| coordinates_in(item, out));
        }
        Value::Array(items) => items.iter().for_each(|item| coordinates_in(item, out)),
        _ => {}
    }
}

fn leaf_strings(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(text) if !text.trim().is_empty() => out.push(text.trim().to_string()),
        Value::Array(items) => items.iter().for_each(|item| leaf_strings(item, out)),
        Value::Object(map) => map
            .iter()
            .filter(|(key, _)| !matches!(key.as_str(), "language" | "geocodes" | "fieldValue" | "type" | "isForeignLocation"))
            .for_each(|(_, item)| leaf_strings(item, out)),
        _ => {}
    }
}

/// Values of the row's structured keys, as (kind, value, platform qualifier).
fn keyed(row: &ToolIo, value: &Value, parent: &str, out: &mut Vec<(&'static str, String, String)>) {
    match value {
        Value::Object(map) => {
            let sibling_platform = ["proof_type", "service", "platform", "network"]
                .iter()
                .find_map(|key| map.get(*key).and_then(Value::as_str))
                .map(normalize_platform)
                .filter(|platform| super::ACCOUNT_PLATFORMS.contains(&platform.as_str()));
            let parent_platform = Some(normalize_platform(parent)).filter(|platform| super::ACCOUNT_PLATFORMS.contains(&platform.as_str()));
            if row.produces.contains(&"person_name") {
                let first = map.get("first_name").and_then(Value::as_str).unwrap_or("").trim();
                let last = map.get("last_name").and_then(Value::as_str).unwrap_or("").trim();
                if !first.is_empty() && !last.is_empty() {
                    out.push(("person_name", format!("{first} {last}"), String::new()));
                }
            }
            for (key, item) in map {
                if let Some((_, kind)) = row.keys.iter().find(|(name, _)| name == key) {
                    let qualifier = if *kind == "handle" || *kind == PLATFORM_ID_KIND {
                        sibling_platform
                            .clone()
                            .or_else(|| parent_platform.clone())
                            .unwrap_or_else(|| row.account_platform.to_string())
                    } else {
                        String::new()
                    };
                    match item {
                        Value::String(text) => out.push((kind, text.trim().to_string(), qualifier)),
                        Value::Object(inner) if *kind != "address" => {
                            if let Some(name) = inner.get("name").and_then(Value::as_str) {
                                out.push((kind, name.trim().to_string(), qualifier));
                            }
                        }
                        Value::Object(_) | Value::Array(_) if *kind == "address" => {
                            let mut parts = Vec::new();
                            leaf_strings(item, &mut parts);
                            if !parts.is_empty() {
                                out.push((kind, parts.join(", "), String::new()));
                            }
                        }
                        Value::Array(items) => {
                            for text in items.iter().filter_map(Value::as_str) {
                                out.push((kind, text.trim().to_string(), qualifier.clone()));
                            }
                        }
                        _ => {}
                    }
                }
                keyed(row, item, key, out);
            }
        }
        Value::Array(items) => items.iter().for_each(|item| keyed(row, item, parent, out)),
        _ => {}
    }
}

/// The subject shares a content word of four or more letters with the value.
fn related(subject: &str, value: &str) -> bool {
    let value = value.to_ascii_lowercase();
    subject
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|word| word.len() >= 4)
        .any(|word| value.contains(&word.to_ascii_lowercase()))
}

/// A handle belongs to the subject when it carries the subject's name, when a result
/// titled with the subject links its profile, or when the observation is the profile
/// of an account the subject already owns.
pub fn owned_handle(subject: &str, handle: &str, hits: &[SearchHit], profile_of_input: bool) -> bool {
    if profile_of_input || names_subject(subject, handle) {
        return true;
    }
    let needle = handle.to_ascii_lowercase();
    hits.iter().any(|hit| {
        names_subject(subject, &hit.title)
            && super::super::extract_social_handles(std::slice::from_ref(&hit.url))
                .iter()
                .any(|found| found.handle.eq_ignore_ascii_case(&needle))
    })
}

/// Keeps only bindings in the vocabulary whose value occurs in the observation text.
/// Composed values (an address from parts, coordinates, a name from first and last
/// name, a package) need every part to occur. Handles lose a leading `@`, and the same
/// handle stays once per platform.
pub fn accept_bindings(candidates: Vec<Binding>, observation: &str) -> Vec<Binding> {
    let haystack = observation.to_ascii_lowercase();
    let mut kept: Vec<Binding> = Vec::new();
    for binding in candidates {
        let mut value = binding.value.trim().to_string();
        if binding.kind == "handle" {
            value = value.trim_start_matches('@').to_string();
        }
        if !known_kind(&binding.kind) || value.is_empty() || value.chars().count() > 300 {
            continue;
        }
        let lower = value.to_ascii_lowercase();
        let occurs = haystack.contains(&lower)
            || match binding.kind.as_str() {
                "address" | COORDINATES_KIND => lower.split(',').map(str::trim).filter(|part| !part.is_empty()).all(|part| haystack.contains(part)),
                "person_name" => lower.split_whitespace().all(|part| haystack.contains(part)),
                "package" => package_parts(&value).is_some_and(|(_, name, version)| haystack.contains(&name.to_ascii_lowercase()) && haystack.contains(&version.to_ascii_lowercase())),
                _ => false,
            };
        if !occurs {
            continue;
        }
        // LinkedIn handles carry their page type (`company/acme`, `in/jane`).
        let linkedin_path = binding.qualifier == "linkedin" && (value.starts_with("company/") || value.starts_with("in/")) && value.matches('/').count() == 1;
        if binding.kind == "handle" && !value.chars().all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '-') || (linkedin_path && ch == '/')) {
            continue;
        }
        if binding.kind == "person_name" && !plausible_person_name(&value) {
            continue;
        }
        if binding.kind == "domain" && !registrable_domain(&value) {
            continue;
        }
        if kept.iter().any(|existing| {
            existing.kind == binding.kind && existing.value.eq_ignore_ascii_case(&value) && existing.qualifier == binding.qualifier
        }) {
            continue;
        }
        kept.push(Binding { value, ..binding });
    }
    kept
}

/// Rule binder for one observation. Parses exactly the kinds `TOOLS` lists for the tool,
/// checks each value against the observation, and keeps only subject-owned handles.
pub fn rule_bindings(question: &str, evidence_id: &str, tool_id: &str, observations: &Value) -> Vec<Binding> {
    let Some(row) = tool_row(tool_id) else {
        return Vec::new();
    };
    let subject = subject_of(question);
    let observation = observations.to_string();
    let mut texts = Vec::new();
    strings_in(observations, &mut texts);
    let hits = hits_in(evidence_id, observations);
    let entities = if hits.is_empty() { Vec::new() } else { select_entities(question, &hits) };
    let mut keyed_values = Vec::new();
    keyed(row, observations, "", &mut keyed_values);
    let mut candidates: Vec<Binding> = Vec::new();
    let mut push = |kind: &str, value: &str, qualifier: &str| {
        candidates.push(Binding {
            kind: kind.into(),
            value: value.into(),
            evidence_id: evidence_id.into(),
            qualifier: qualifier.into(),
            source_tool: row.tool.into(),
            unverified: kind == "handle" && row.unverified_handles,
            ..Binding::default()
        });
    };
    let own_domains: Vec<String> = entities
        .iter()
        .filter(|entity| entity.selected)
        .flat_map(|entity| entity.identifiers.iter().filter(|identifier| identifier.kind == "domain").map(|identifier| identifier.value.clone()))
        .collect();
    for kind in row.produces {
        match *kind {
            "handle" => {
                for text in &texts {
                    for handle in super::super::extract_social_handles(std::slice::from_ref(text)) {
                        if owned_handle(&subject, &handle.handle, &hits, row.profile_of_input) {
                            push("handle", &handle.handle, &handle.platform);
                        }
                    }
                }
                for (kind, value, qualifier) in keyed_values.iter().filter(|(kind, _, _)| *kind == "handle") {
                    if owned_handle(&subject, value, &hits, row.profile_of_input) {
                        push(kind, value, qualifier);
                    }
                }
            }
            "person_name" | "org_name" => {
                let wanted = if *kind == "person_name" { "person" } else { "organization" };
                for entity in entities.iter().filter(|entity| entity.selected && entity.entity_type == wanted) {
                    push(kind, &entity.canonical_name, "");
                }
                for (_, value, _) in keyed_values.iter().filter(|(item, _, _)| item == kind) {
                    if row.profile_of_input || related(&subject, value) || hits.is_empty() && *kind == "person_name" {
                        push(kind, value, "");
                    }
                }
            }
            "domain" => {
                for domain in &own_domains {
                    push("domain", domain, "");
                }
                if hits.is_empty() {
                    for text in &texts {
                        for domain in domains_in(text) {
                            if !social_or_publisher(&domain) {
                                push("domain", &domain, "");
                            }
                        }
                    }
                }
                for (_, value, _) in keyed_values.iter().filter(|(item, _, _)| *item == "domain") {
                    push("domain", value.trim_start_matches("*."), "");
                }
            }
            "url" => {
                let wanted = |raw: &str| {
                    url::Url::parse(raw).ok().and_then(|url| url.host_str().map(str::to_string)).is_some_and(|host| {
                        let host = host.trim_start_matches("www.").to_string();
                        !social_or_publisher(&host)
                            && (hits.is_empty() || related(&subject, &host) || own_domains.iter().any(|own| host == *own || host.ends_with(&format!(".{own}"))))
                    })
                };
                if hits.is_empty() {
                    for text in &texts {
                        for value in kind_in(text, "url") {
                            if wanted(&value) {
                                push(URL_KIND, &value, "");
                            }
                        }
                    }
                } else {
                    for hit in hits.iter().filter(|hit| wanted(&hit.url)).take(5) {
                        push(URL_KIND, &hit.url, "");
                    }
                }
            }
            "email" => texts.iter().flat_map(|text| emails_in(text)).for_each(|email| push("email", &email, "")),
            "ip" => texts.iter().flat_map(|text| kind_in(text, "ip")).for_each(|ip| push("ip", &ip, "")),
            "cve" => texts.iter().flat_map(|text| kind_in(text, "cve")).for_each(|cve| push("cve", &cve, "")),
            "wallet" => texts.iter().flat_map(|text| bitcoins_in(text)).for_each(|wallet| push("wallet", &wallet, "")),
            "package" => texts.iter().flat_map(|text| packages_in(text)).for_each(|package| push("package", &package, "")),
            "address" => {
                for (_, value, _) in keyed_values.iter().filter(|(item, _, _)| *item == "address") {
                    push("address", value, "");
                }
            }
            PLATFORM_ID_KIND => {
                for (_, value, qualifier) in keyed_values.iter().filter(|(item, _, _)| *item == PLATFORM_ID_KIND) {
                    if !qualifier.is_empty() {
                        push(PLATFORM_ID_KIND, value, qualifier);
                    }
                }
            }
            COORDINATES_KIND => {
                let mut found = Vec::new();
                coordinates_in(observations, &mut found);
                found.iter().take(3).for_each(|value| push(COORDINATES_KIND, value, ""));
            }
            _ => {}
        }
    }
    // A domain finder match is a guess unless Hunter called it a perfect match.
    if row.tool == "hunter_domain_finder" && observations.get("perfect_match").and_then(Value::as_bool) != Some(true) {
        for binding in candidates.iter_mut().filter(|binding| binding.kind == "domain") {
            binding.inferred = true;
            binding.qualifier = "company".into();
        }
    }
    let mut per_kind: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    accept_bindings(candidates, &observation)
        .into_iter()
        .filter(|binding| {
            let count = per_kind.entry(binding.kind.clone()).or_default();
            *count += 1;
            *count <= if binding.kind == "handle" { 8 } else { 5 }
        })
        .collect()
}

/// Recon model bindings pass the same gates as rule bindings: the platform is
/// normalized (`X` -> `twitter`, `Truth Social` -> `truthsocial`), a profile URL becomes
/// its handle, and a handle must belong to the subject.
pub fn vet_model_bindings(
    question: &str,
    evidence_id: &str,
    tool_id: &str,
    observations: &Value,
    candidates: Vec<Binding>,
) -> Vec<Binding> {
    let subject = subject_of(question);
    let hits = hits_in(evidence_id, observations);
    let profile = tool_row(tool_id).is_some_and(|row| row.profile_of_input);
    let source_tool = tool_row(tool_id).map(|row| row.tool).unwrap_or("");
    let mut out = Vec::new();
    for mut binding in candidates {
        binding.source_tool = source_tool.into();
        if binding.kind == "handle" && search_handles(tool_id) {
            binding.unverified = true;
        }
        if binding.kind == "handle" {
            let raw = binding.value.trim().to_string();
            if raw.contains('/') {
                match super::super::extract_social_handles(&[raw]).into_iter().next() {
                    Some(found) => {
                        binding.value = found.handle;
                        binding.qualifier = found.platform;
                    }
                    None => continue,
                }
            } else {
                binding.value = raw.trim_start_matches('@').to_string();
                let platform = normalize_platform(&binding.qualifier);
                binding.qualifier = if super::ACCOUNT_PLATFORMS.contains(&platform.as_str()) { platform } else { String::new() };
            }
            if !owned_handle(&subject, &binding.value, &hits, profile) {
                continue;
            }
        }
        out.push(binding);
    }
    accept_bindings(out, &observations.to_string())
}

/// Platforms the derived questions (then the user's question) ask about, in question
/// order, limited to the platforms SociaVault serves.
pub fn question_platforms(texts: &[(String, String)]) -> Vec<(String, String)> {
    let mut found: Vec<(String, String)> = Vec::new();
    for (id, text) in texts {
        let lower = format!(" {} ", text.to_ascii_lowercase());
        let mut hits: Vec<(usize, &str)> = Vec::new();
        for (hint, platform) in [
            ("twitter", "twitter"), (" x ", "twitter"), ("x.com", "twitter"), ("(x)", "twitter"), ("x/twitter", "twitter"),
            ("instagram", "instagram"), ("facebook", "facebook"), ("tiktok", "tiktok"), ("youtube", "youtube"),
            ("linkedin", "linkedin"), ("threads", "threads"), ("twitch", "twitch"),
            ("pinterest", "pinterest"), ("reddit", "reddit"),
        ] {
            if let Some(at) = lower.find(hint) {
                hits.push((at, platform));
            }
        }
        hits.sort();
        for (_, platform) in hits {
            if !found.iter().any(|(known, _)| known == platform) {
                found.push((platform.to_string(), id.clone()));
            }
        }
    }
    found
}

/// Accounts to look up for a per-platform tool: (platform, handle binding). A question's
/// platform without its own handle borrows the subject's best-supported handle, marked
/// `inferred`. Without question platforms, every subject handle on a supported platform.
pub fn per_platform_targets(
    tool_id: &str,
    platforms: &[(String, String)],
    bindings: &[Binding],
    question: &str,
) -> (Vec<(String, Binding, String)>, Vec<String>) {
    let subject = subject_of(question);
    let served = crate::osint::sociavault_account_platforms(tool_id);
    let platforms: Vec<(String, String)> = platforms.iter().filter(|(platform, _)| served.contains(&platform.as_str())).cloned().collect();
    let mut targets = Vec::new();
    let mut unresolved = Vec::new();
    if platforms.is_empty() {
        for binding in bindings.iter().filter(|binding| binding.kind == "handle" && !binding.inferred && served.contains(&binding.qualifier.as_str())) {
            if !targets.iter().any(|(platform, _, _): &(String, Binding, String)| *platform == binding.qualifier) {
                targets.push((binding.qualifier.clone(), binding.clone(), String::new()));
            }
        }
        return (targets, unresolved);
    }
    for (platform, qid) in &platforms {
        let exact = platform_handle(bindings, platform);
        match exact {
            Some(binding) => targets.push((platform.clone(), binding.clone(), qid.clone())),
            None => match best_handle(bindings, &subject) {
                Some(best) => targets.push((
                    platform.clone(),
                    Binding { qualifier: platform.clone(), inferred: true, ..best.clone() },
                    qid.clone(),
                )),
                None => unresolved.push(format!("{platform} ({qid}): no handle found")),
            },
        }
    }
    (targets, unresolved)
}

/// True when this registry tool cannot be reached by any binding producer.
#[cfg_attr(not(test), allow(dead_code))]
pub fn unreachable_reason(tool_id: &str) -> Option<String> {
    let row = tool_row(tool_id)?;
    for slot in row.slots {
        let reachable = slot.fills.iter().any(|fill| {
            fill.kind == QUERY_KIND || PROMPT_KINDS.contains(&fill.kind) || !producers_of(fill.kind).is_empty()
        });
        if !reachable {
            return Some(format!("{} has no producer", slot_label(slot)));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::super::question_bindings;
    use super::*;
    use crate::osint;

    /// Tool inputs a fill writes.
    fn writes(fill: &Fill) -> Vec<&'static str> {
        match fill.how {
            How::SocialPair(_) => vec!["platform", "handle"],
            How::PlatformIdPair(_) => vec!["platform", "user_id"],
            How::PlatformQuery(_) => vec!["platform", fill.input],
            How::PackageParts => vec!["ecosystem", "package_name", "version"],
            How::Coordinates => vec!["latitude", "longitude"],
            _ => vec![fill.input],
        }
    }

    #[test]
    fn every_registry_input_maps_to_a_kind_with_a_producer_and_an_extractor() {
        let registry = osint::registry();
        assert_eq!(TOOLS.len(), registry.len(), "one table row per catalog tool");
        for tool in registry {
            let row = tool_row(tool.id).unwrap_or_else(|| panic!("{} has no TOOLS row", tool.id));
            let written: Vec<&str> = row
                .slots
                .iter()
                .flat_map(|slot| slot.fills.iter().flat_map(writes))
                .chain(row.extras.iter().map(|(input, _)| *input))
                .collect();
            // (a) every required input group has a binding kind that fills it.
            for group in tool.inputs {
                assert!(
                    group.split('|').any(|input| written.contains(&input)),
                    "{}: input {group} has no binding kind",
                    tool.id
                );
            }
            for slot in row.slots {
                for fill in slot.fills {
                    // (b) at least one producer: a tool whose observation the binder parses
                    // for the kind, or an explicit prompt-only entry with its reason.
                    let prompt_only = PROMPT_ONLY.iter().find(|(kind, _)| *kind == fill.kind);
                    assert!(
                        fill.kind == QUERY_KIND || !producers_of(fill.kind).is_empty() || prompt_only.is_some_and(|(_, why)| !why.is_empty()),
                        "{}: {} has no producer and is not marked prompt-only",
                        tool.id,
                        fill.kind
                    );
                    let tool_only = TOOL_ONLY.iter().any(|(kind, why)| *kind == fill.kind && !why.is_empty());
                    assert!(fill.kind == QUERY_KIND || PROMPT_KINDS.contains(&fill.kind) || tool_only, "{}: the prompt never yields {}", tool.id, fill.kind);
                    // (c) a rule extractor exists for the kind.
                    assert!(!extractor(fill.kind).is_empty(), "{}: no extractor for {}", tool.id, fill.kind);
                    for input in writes(fill) {
                        let schema = tool.schema();
                        assert!(schema["properties"].get(input).is_some(), "{}: {input} is not a tool input", tool.id);
                    }
                }
            }
            for fill in row.optional {
                for input in writes(fill) {
                    assert!(tool.schema()["properties"].get(input).is_some(), "{}: optional {input} is not a tool input", tool.id);
                }
            }
            for kind in row.produces {
                assert!(known_kind(kind), "{}: produces unknown kind {kind}", tool.id);
                assert!(!extractor(kind).is_empty(), "{}: no extractor for produced {kind}", tool.id);
            }
            for producer in row.after {
                let produced = tool_row(producer).unwrap_or_else(|| panic!("{}: unknown producer {producer}", tool.id)).produces;
                let fills: Vec<&str> = row.slots.iter().flat_map(|slot| slot.fills.iter().map(|fill| fill.kind)).collect();
                assert!(produced.iter().any(|kind| fills.contains(kind)), "{}: declared producer {producer} yields none of {fills:?}", tool.id);
            }
            assert_eq!(unreachable_reason(tool.id), None, "{} is unreachable", tool.id);
            assert!(pickable(tool.id), "{} is not offered to the picker", tool.id);
        }
        for kind in BINDING_KINDS.iter().filter(|kind| **kind != "platform") {
            assert!(PROMPT_KINDS.contains(kind) || !producers_of(kind).is_empty(), "{kind} has no producer");
        }
    }

    /// Every SociaVault route in the matrix is reachable: the tool's fills write
    /// `platform` and at least one of the route's inputs, and each such fill's kind has a
    /// producer (a tool, the prompt, or the always-available query). Every Firecrawl and
    /// Hunter tool has a row, and Hunter inputs come only from primary providers or the
    /// prompt (D1).
    #[test]
    fn every_primary_provider_route_maps_its_inputs_to_producers() {
        let has_producer = |kind: &str| kind == QUERY_KIND || PROMPT_KINDS.contains(&kind) || !producers_of(kind).is_empty();
        for route in osint::SOCIAVAULT_ROUTES {
            let row = tool_row(route.tool).unwrap_or_else(|| panic!("{} has no row", route.tool));
            let fills: Vec<&Fill> = row.slots.iter().flat_map(|slot| slot.fills.iter()).chain(row.optional.iter()).collect();
            let name = format!("{} {}:{}", route.tool, route.platform, route.endpoint);
            if route.tool != "sociavault_google_search" {
                assert!(fills.iter().any(|fill| writes(fill).contains(&"platform")), "{name}: platform is never written");
            }
            let feeding: Vec<&&Fill> = fills.iter().filter(|fill| writes(fill).iter().any(|input| route.input.keys().contains(input))).collect();
            assert!(!feeding.is_empty(), "{name}: no fill writes {:?}", route.input.keys());
            assert!(feeding.iter().any(|fill| has_producer(fill.kind)), "{name}: no producer for {:?}", route.input.keys());
            // The account platforms a pair fill accepts include this route's platform.
            if route.input.keys().contains(&"handle") || route.input.keys().contains(&"user_id") {
                assert!(osint::sociavault_account_platforms(route.tool).contains(&route.platform), "{name}");
            }
        }
        for tool in osint::registry().iter().filter(|tool| tool.id.starts_with("firecrawl_") || tool.id.starts_with("hunter_")) {
            let row = tool_row(tool.id).unwrap_or_else(|| panic!("{} has no row", tool.id));
            assert!(pickable(tool.id), "{}", tool.id);
            if !tool.id.starts_with("hunter_") {
                continue;
            }
            for producer in row.after {
                assert!(osint::primary_provider(producer).is_some(), "{}: declared producer {producer} is a gap-filler", tool.id);
            }
            for fill in row.slots.iter().flat_map(|slot| slot.fills.iter()) {
                let primary: Vec<&str> = producers_of(fill.kind).into_iter().filter(|producer| allowed_producer(tool.id, producer)).collect();
                assert!(PROMPT_KINDS.contains(&fill.kind) || !primary.is_empty(), "{}: {} has no primary producer", tool.id, fill.kind);
                assert!(primary.iter().all(|producer| osint::primary_provider(producer).is_some()), "{}: {primary:?}", tool.id);
                for gap_filler in producers_of(fill.kind).into_iter().filter(|producer| osint::primary_provider(producer).is_none()) {
                    assert!(!allowed_producer(tool.id, gap_filler), "{}: gap-filler {gap_filler} may feed {}", tool.id, fill.kind);
                }
            }
        }
        assert!(allowed_producer("hunter_domain_search", PROMPT_SOURCE));
        assert!(!allowed_producer("firecrawl_map", "crtsh_certificates"));
        assert!(allowed_producer("urlscan_search", "crtsh_certificates"), "gap-fillers stay unrestricted");
    }

    /// D1: a domain only crt.sh found is never a Hunter input; once a Firecrawl result
    /// also contains it (the binding takes the primary source), it binds.
    #[test]
    fn a_gap_filler_only_domain_waits_for_a_primary_observation_before_hunter() {
        let question = "Who runs Acme Robotics?";
        let crtsh = rule_bindings(question, "call-s1", "crtsh_certificates", &json!({"names": ["acmerobotics.com", "www.acmerobotics.com"]}));
        let domain = crtsh.iter().find(|binding| binding.kind == "domain").expect("crt.sh yields the domain").clone();
        assert_eq!(domain.source_tool, "crtsh_certificates");
        let mut bindings = question_bindings(question);
        bindings.push(domain.clone());
        for hunter in ["hunter_domain_search", "hunter_company_enrichment", "hunter_email_count"] {
            let (args, _, _) = bind_arguments(hunter, &bindings, question, None);
            assert!(args.get("domain").is_none(), "{hunter} took a crt.sh-only domain: {args}");
        }
        assert!(bind_arguments("firecrawl_map", &bindings, question, None).2.len() == 1, "map waits too");
        // Gap-fillers still use it.
        assert_eq!(bind_arguments("urlscan_search", &bindings, question, None).0["domain"], "acmerobotics.com");
        // A Firecrawl observation containing the same value makes it eligible.
        let search = json!({"results": [{"title": "Acme Robotics | Home", "url": "https://acmerobotics.com/", "snippet": "Acme Robotics builds robots."}]});
        let primary = rule_bindings(question, "call-s2", "firecrawl_search", &search);
        let seen = primary.iter().find(|binding| binding.kind == "domain" && binding.value == "acmerobotics.com").expect("Firecrawl yields it");
        assert_eq!(seen.source_tool, "firecrawl_search");
        bindings.retain(|binding| binding.value != "acmerobotics.com");
        bindings.push(Binding { source_tool: seen.source_tool.clone(), evidence_id: seen.evidence_id.clone(), ..domain });
        let (args, filled, missing) = bind_arguments("hunter_domain_search", &bindings, question, None);
        assert!(missing.is_empty(), "{missing:?}");
        assert_eq!(args["domain"], "acmerobotics.com");
        assert!(filled.iter().any(|fill| fill.contains("via firecrawl_search")), "{filled:?}");
        // A domain named in the prompt binds directly.
        let prompt = question_bindings("Who runs acmerobotics.com?");
        assert_eq!(bind_arguments("hunter_company_enrichment", &prompt, "Who runs acmerobotics.com?", None).0["domain"], "acmerobotics.com");
    }

    #[test]
    fn hunter_routes_emails_and_domain_finder_matches() {
        let question = "Who is jane@acmerobotics.com and jane.example@gmail.com?";
        let known = question_bindings(question);
        assert_eq!(bind_arguments("hunter_combined_enrichment", &known, question, None).0["email"], "jane@acmerobotics.com");
        assert_eq!(bind_arguments("hunter_person_enrichment", &known, question, None).0["email"], "jane.example@gmail.com");
        // Webmail domains never become company domains.
        let webmail = vec![Binding { kind: "domain".into(), value: "gmail.com".into(), evidence_id: "question".into(), ..Binding::default() }];
        assert!(!bind_arguments("hunter_domain_search", &webmail, question, None).2.is_empty());
        // Domain finder: perfect matches are exact, others inferred.
        let observed = |perfect: bool| json!({"companies": [{"domain": "acmerobotics.com", "company_name": "Acme Robotics"}], "perfect_match": perfect});
        let exact = rule_bindings("Who runs Acme Robotics?", "call-s1", "hunter_domain_finder", &observed(true));
        assert!(exact.iter().any(|binding| binding.kind == "domain" && !binding.inferred && binding.source_tool == "hunter_domain_finder"));
        let guess = rule_bindings("Who runs Acme Robotics?", "call-s1", "hunter_domain_finder", &observed(false));
        assert!(guess.iter().filter(|binding| binding.kind == "domain").all(|binding| binding.inferred), "{guess:?}");
        let org = "What does Acme Robotics Inc do?";
        let company = bind_arguments("hunter_domain_finder", &question_bindings(org), org, None).0;
        assert!(company["company"].as_str().is_some_and(|name| name.contains("Acme Robotics")), "{company}");
    }

    #[test]
    fn sociavault_query_tools_take_the_named_platform_and_mark_found_handles_unverified() {
        let question = "Who is Jane Example on TikTok?";
        let known = question_bindings(question);
        let (args, _, missing) = bind_arguments("sociavault_search_users", &known, question, None);
        assert!(missing.is_empty(), "{missing:?} from {known:?}");
        assert_eq!(args["platform"], "tiktok", "the named platform");
        assert!(args["query"].as_str().is_some_and(|query| query.contains("Jane Example")), "{args}");
        assert!(osint::validate("sociavault_search_users", &args).is_ok());
        // No platform named: the tool's default.
        let plain = question_bindings("Who is Jane Example?");
        assert_eq!(bind_arguments("sociavault_search", &plain, "Who is Jane Example?", None).0["platform"], "twitter");
        // r/<name> selects the Reddit subreddit route.
        let reddit = "What does r/rust say about Jane Example?";
        let name = vec![Binding { kind: "person_name".into(), value: "Jane Example".into(), evidence_id: "question".into(), ..Binding::default() }];
        let (args, _, _) = bind_arguments("sociavault_search", &name, reddit, None);
        assert_eq!((args["platform"].as_str(), args["subreddit"].as_str(), args["endpoint"].as_str()), (Some("reddit"), Some("rust"), Some("subreddit")), "{args}");
        assert!(osint::validate("sociavault_search", &args).is_ok());
        // Handles a search lists are leads, not the subject's confirmed accounts.
        let found = rule_bindings("Who is Jane Example?", "call-s1", "sociavault_search_users", &json!({"accounts": [{"platform": "tiktok", "handle": "janeexample", "url": "https://www.tiktok.com/@janeexample"}]}));
        assert!(found.iter().filter(|binding| binding.kind == "handle").all(|binding| binding.unverified), "{found:?}");
        // A profile's platform id feeds user-content routes that need it.
        let id = vec![Binding { kind: PLATFORM_ID_KIND.into(), value: "44196397".into(), qualifier: "twitter".into(), evidence_id: "call-s2".into(), source_tool: "sociavault_profile".into(), ..Binding::default() }];
        let (args, _, missing) = bind_arguments("sociavault_user_content", &id, "Show Jane Example's tweets", None);
        assert!(missing.is_empty());
        assert_eq!(args, json!({"platform": "twitter", "user_id": "44196397"}));
        let profile = rule_bindings("Who is Jane Example?", "call-s2", "sociavault_profile", &json!({"platform": "twitter", "handle": "janeexample", "platform_id": "44196397"}));
        assert!(profile.iter().any(|binding| binding.kind == PLATFORM_ID_KIND && binding.qualifier == "twitter" && binding.value == "44196397"), "{profile:?}");
    }

    #[test]
    fn batch_scrape_takes_ranked_urls_and_gates_order_before_hunter() {
        let urls: Vec<Binding> = ["https://acmerobotics.com/blog/a", "https://acmerobotics.com/contact", "https://acmerobotics.com/about", "https://acmerobotics.com/blog/b", "https://acmerobotics.com/blog/c", "https://acmerobotics.com/blog/d"]
            .iter()
            .map(|url| Binding { kind: URL_KIND.into(), value: url.to_string(), evidence_id: "call-s1".into(), source_tool: "firecrawl_map".into(), ..Binding::default() })
            .collect();
        let (args, _, missing) = bind_arguments("firecrawl_batch_scrape", &urls, "Who runs Acme Robotics?", None);
        assert!(missing.is_empty());
        let picked: Vec<&str> = args["urls"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
        assert_eq!(picked.len(), osint::BATCH_SCRAPE_DEFAULT_URLS);
        assert_eq!(&picked[..2], ["https://acmerobotics.com/contact", "https://acmerobotics.com/about"]);
        let order = super::super::dependency_order(&["hunter_domain_search".to_string(), "hunter_email_count".to_string()], &question_bindings("Who runs acmerobotics.com?"), &std::collections::HashMap::new());
        assert_eq!(order, ["hunter_email_count", "hunter_domain_search"], "email count gates the paid domain search");
        let deps = super::super::depends_on(&order, 1, &[], &std::collections::HashMap::new(), &std::collections::HashMap::new());
        assert!(deps.contains(&0));
        // A gap-filler is never a declared or implied producer for Hunter.
        let mixed = ["crtsh_certificates".to_string(), "hunter_company_enrichment".to_string()];
        assert!(super::super::depends_on(&mixed, 1, &[], &std::collections::HashMap::new(), &std::collections::HashMap::new()).is_empty());
    }

    /// Prompt -> known bindings -> the first tool's arguments.
    fn first_args(question: &str, tool: &str) -> Value {
        let known = question_bindings(question);
        let (args, _, missing) = bind_arguments(tool, &known, question, None);
        assert!(missing.is_empty(), "{tool} for {question:?}: missing {missing:?} from {known:?}");
        osint::validate(tool, &args).unwrap_or_else(|err| panic!("{tool} {args}: {err}"));
        args
    }

    /// One producer observation -> bindings -> the dependent tool's arguments.
    fn next_args(question: &str, producer: &str, observation: Value, next: &str) -> Value {
        let mut bindings = question_bindings(question);
        bindings.extend(rule_bindings(question, "call-s1", producer, &observation));
        let (args, _, missing) = bind_arguments(next, &bindings, question, None);
        assert!(missing.is_empty(), "{next} after {producer}: missing {missing:?} from {bindings:?}");
        osint::validate(next, &args).unwrap_or_else(|err| panic!("{next} {args}: {err}"));
        args
    }

    #[test]
    fn person_prompt_feeds_profile_tools_from_a_search() {
        let question = "Who is Ada Lovelace?";
        assert!(question_bindings(question).iter().any(|b| b.kind == "person_name" && b.value == "Ada Lovelace"));
        let search = json!({"results": [
            {"title": "Ada Lovelace (@adalovelace) / X", "url": "https://x.com/adalovelace", "snippet": "Posts by Ada Lovelace. Proofs at keybase.io/adalovelace"},
        ]});
        assert_eq!(next_args(question, "firecrawl_search", search.clone(), "sociavault_profile"), json!({"platform": "twitter", "handle": "adalovelace"}));
        assert_eq!(next_args(question, "firecrawl_search", search, "keybase_identity"), json!({"username": "adalovelace"}));
    }

    #[test]
    fn organization_prompt_finds_its_domain_for_hunter() {
        let question = "What does Acme Robotics Inc do?";
        let search = json!({"results": [
            {"title": "Acme Robotics Inc | Industrial robots", "url": "https://acmerobotics.com/", "snippet": "Acme Robotics Inc builds industrial robots."},
            {"title": "Acme Robotics Inc - About", "url": "https://acmerobotics.com/about", "snippet": "Founded in 2001."},
        ]});
        assert_eq!(next_args(question, "firecrawl_search", search, "hunter_domain_search")["domain"], json!("acmerobotics.com"));
    }

    /// Acceptance 3: a domain prompt runs Hunter first, and its social handles feed
    /// SociaVault in the same turn.
    #[test]
    fn domain_prompt_feeds_hunter_then_sociavault() {
        let question = "Who runs acmerobotics.com?";
        assert_eq!(first_args(question, "hunter_company_enrichment"), json!({"domain": "acmerobotics.com"}));
        assert_eq!(first_args(question, "hunter_email_count"), json!({"domain": "acmerobotics.com"}));
        let card = json!({"name": "Acme Robotics", "domain": "acmerobotics.com", "social": {"twitter": {"handle": "acmerobotics"}, "linkedin": {"handle": "company/acme-robotics"}}});
        let profile = next_args(question, "hunter_company_enrichment", card.clone(), "sociavault_profile");
        assert!(
            profile == json!({"platform": "twitter", "handle": "acmerobotics"}) || profile == json!({"platform": "linkedin", "handle": "company/acme-robotics"}),
            "{profile}"
        );
        let bindings = rule_bindings(question, "call-s1", "hunter_company_enrichment", &card);
        let linkedin = bindings.iter().find(|binding| binding.qualifier == "linkedin").expect("LinkedIn company handle");
        let args = json!({"platform": "linkedin", "handle": linkedin.value});
        assert_eq!(osint::select_route("sociavault_profile", &args).unwrap().endpoint, "company");
    }

    #[test]
    fn domain_prompt_chains_hosts_to_ip_tools() {
        let question = "What is known about example.org?";
        assert_eq!(first_args(question, "crtsh_certificates"), json!({"domain": "example.org"}));
        let hosts = json!({"raw": "api.example.org,93.184.216.34\nwww.example.org,93.184.216.34"});
        assert_eq!(next_args(question, "hackertarget_hostsearch", hosts, "shodan_internetdb"), json!({"ip": "93.184.216.34"}));
    }

    #[test]
    fn ip_prompt_yields_hostnames_and_cves() {
        let question = "Who is behind 8.8.8.8?";
        assert_eq!(first_args(question, "ripestat_network_info"), json!({"ip": "8.8.8.8"}));
        let shodan = json!({"ip": "8.8.8.8", "hostnames": ["dns.google"], "ports": [53, 443], "vulns": ["CVE-2021-44228"]});
        assert_eq!(next_args(question, "shodan_internetdb", shodan.clone(), "crtsh_certificates"), json!({"domain": "dns.google"}));
        assert_eq!(next_args(question, "shodan_internetdb", shodan, "nvd_cve"), json!({"cve_id": "CVE-2021-44228"}));
    }

    #[test]
    fn email_prompt_feeds_verifier_and_domain_search_then_finder() {
        let question = "Is ada@example.org a real address?";
        assert_eq!(first_args(question, "hunter_email_verifier"), json!({"email": "ada@example.org"}));
        assert_eq!(first_args(question, "hunter_domain_search"), json!({"domain": "example.org"}));
        let domain_search = json!({"data": {"domain": "example.org", "organization": "Example Org", "emails": [
            {"value": "bob.stone@example.org", "first_name": "Bob", "last_name": "Stone", "position": "CTO"}
        ]}});
        let args = next_args(question, "hunter_domain_search", domain_search, "hunter_email_finder");
        assert_eq!(args, json!({"domain": "example.org", "full_name": "Bob Stone"}));
        // A webmail address does not become an organization domain.
        assert!(!question_bindings("who uses jane@gmail.com?").iter().any(|b| b.kind == "domain"));
    }

    #[test]
    fn username_prompt_feeds_keybase_whose_proofs_feed_sociavault() {
        let question = "Who is @janedoe99?";
        assert_eq!(first_args(question, "keybase_identity"), json!({"username": "janedoe99"}));
        assert_eq!(first_args(question, "wikipedia_users"), json!({"username": "janedoe99"}));
        assert_eq!(first_args(question, "github_repositories"), json!({"query": "janedoe99"}));
        let keybase = json!({"them": [{"basics": {"username": "janedoe99"}, "proofs_summary": {"all": [
            {"proof_type": "twitter", "nametag": "jdoe_tw"},
            {"proof_type": "github", "nametag": "janedoe99"}
        ]}}]});
        assert_eq!(next_args(question, "keybase_identity", keybase, "sociavault_profile"), json!({"platform": "twitter", "handle": "jdoe_tw"}));
    }

    #[test]
    fn cve_prompt_feeds_both_cve_tools() {
        let question = "Tell me about CVE-2021-44228";
        assert_eq!(first_args(question, "nvd_cve"), json!({"cve_id": "CVE-2021-44228"}));
        assert_eq!(first_args(question, "cve_record"), json!({"cve_id": "CVE-2021-44228"}));
    }

    #[test]
    fn package_prompt_feeds_osv_whose_aliases_feed_nvd() {
        let question = "Is lodash 4.17.20 on npm vulnerable?";
        assert_eq!(first_args(question, "osv_package"), json!({"ecosystem": "npm", "package_name": "lodash", "version": "4.17.20"}));
        assert_eq!(packages_in("pip install requests==2.19.0 from PyPI"), vec!["PyPI:requests@2.19.0".to_string()]);
        let osv = json!({"vulns": [{"id": "GHSA-35jh-r3h4-6jhm", "aliases": ["CVE-2021-23337"]}]});
        assert_eq!(next_args(question, "osv_package", osv, "nvd_cve"), json!({"cve_id": "CVE-2021-23337"}));
    }

    #[test]
    fn wallet_url_and_place_prompts_reach_their_tools() {
        let wallet = "What is the balance of bitcoin address 1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa?";
        assert_eq!(first_args(wallet, "blockstream_address"), json!({"bitcoin_address": "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa"}));
        let page = "What is at https://example.org/about?";
        assert_eq!(first_args(page, "firecrawl_scrape"), json!({"url": "https://example.org/about"}));
        assert_eq!(first_args(page, "hunter_tech_lookup"), json!({"domain": "example.org"}));
        let place = "Where is 1600 Pennsylvania Avenue in Washington?";
        let known = question_bindings(place);
        assert!(known.iter().any(|b| b.kind == "address"), "{known:?}");
        let geocode = json!([{"lat": "38.8977", "lon": "-77.0365", "display_name": "White House, 1600, Pennsylvania Avenue Northwest, Washington, District of Columbia, 20500, United States"}]);
        let args = next_args(place, "nominatim_geocode", geocode, "overpass_places");
        assert_eq!(args, json!({"latitude": 38.8977, "longitude": -77.0365, "radius_m": 500}));
    }

    #[test]
    fn junk_names_and_bad_domains_are_rejected() {
        for junk in ["Trump Truth Social archive scraper", "Truth Social Scraper", "Recon Donald Trumps Social Life. Refer To His Social Accounts For Context"] {
            assert!(!plausible_person_name(junk), "{junk}");
        }
        assert!(plausible_person_name("Donald J. Trump"));
        let candidates = vec![
            Binding { kind: "person_name".into(), value: "Trump Truth Social archive scraper".into(), evidence_id: "c".into(), ..Default::default() },
            Binding { kind: "domain".into(), value: "4.17.20".into(), evidence_id: "c".into(), ..Default::default() },
            Binding { kind: "handle".into(), value: "@realDonaldTrump".into(), qualifier: "truthsocial".into(), evidence_id: "c".into(), ..Default::default() },
            Binding { kind: "handle".into(), value: "realDonaldTrump".into(), qualifier: "twitter".into(), evidence_id: "c".into(), ..Default::default() },
        ];
        let kept = accept_bindings(candidates, "Trump Truth Social archive scraper 4.17.20 @realDonaldTrump");
        let values: Vec<(&str, &str)> = kept.iter().map(|b| (b.value.as_str(), b.qualifier.as_str())).collect();
        assert_eq!(values, [("realDonaldTrump", "truthsocial"), ("realDonaldTrump", "twitter")]);
    }
}
