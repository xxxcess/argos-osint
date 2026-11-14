//! Shared OSINT host exclusions for account platforms and citation publishers.

/// Sites that host accounts. Their domains are never organizations in a result set.
pub(crate) fn account_platform_host(host: &str) -> bool {
    const HOSTS: &[&str] = &[
        "x.com",
        "twitter.com",
        "truthsocial.com",
        "instagram.com",
        "facebook.com",
        "fb.com",
        "youtube.com",
        "youtu.be",
        "tiktok.com",
        "threads.net",
        "linkedin.com",
        "twitch.tv",
        "github.com",
        "gitlab.com",
        "keybase.io",
        "reddit.com",
        "pinterest.com",
        "medium.com",
        "substack.com",
        "rumble.com",
        "gettr.com",
        "parler.com",
        "bsky.app",
        "mastodon.social",
        "linktr.ee",
    ];
    HOSTS
        .iter()
        .any(|known| host == *known || host.ends_with(&format!(".{known}")))
}

/// News and reference publishers whose pages are citations, never entities.
pub(crate) fn publisher_host(host: &str) -> bool {
    const OUTLETS: &[&str] = &[
        "apnews",
        "reuters",
        "cnn",
        "foxnews",
        "nbcnews",
        "cbsnews",
        "abcnews",
        "npr",
        "pbs",
        "politico",
        "axios",
        "thehill",
        "rollcall",
        "usatoday",
        "wsj",
        "washingtonpost",
        "nytimes",
        "bbc",
        "theguardian",
        "bloomberg",
        "forbes",
        "newsweek",
        "time",
        "latimes",
        "cnbc",
        "msnbc",
        "aljazeera",
        "theatlantic",
        "vox",
        "huffpost",
        "businessinsider",
        "yahoo",
        "msn",
        "britannica",
        "factcheck",
        "snopes",
        "politifact",
        "ballotpedia",
        "c-span",
        "cspan",
        "vanityfair",
        "newyorker",
        "slate",
        "salon",
        "independent",
        "telegraph",
        "economist",
        "ft",
        "nypost",
        "dailymail",
        "mediaite",
        "semafor",
    ];
    const NEWS_WORDS: &[&str] = &[
        "times",
        "post",
        "tribune",
        "herald",
        "gazette",
        "journal",
        "daily",
        "press",
        "chronicle",
        "reporter",
        "magazine",
        "radio",
        "broadcast",
        "courier",
        "observer",
    ];
    let labels: Vec<&str> = host.split('.').collect();
    let mut index = labels.len().saturating_sub(2);
    if labels.len() >= 3 && matches!(labels[index], "co" | "com" | "org" | "net" | "ac" | "gov") {
        index -= 1;
    }
    let site = labels.get(index).copied().unwrap_or(host);
    // US broadcast call signs such as kark.com or wfaa.com are local news stations.
    let call_sign = site.starts_with(['k', 'w'])
        && (site.len() == 4 || (site.len() == 6 && site.ends_with("tv")))
        && site.chars().all(|ch| ch.is_ascii_lowercase());
    // Q&A, wiki, aggregator, scraper-marketplace, and other user-generated sites: a page
    // there is a citation about the subject, never the subject's own site or name.
    const CITATION_SITES: &[&str] = &[
        "quora",
        "reddit",
        "medium",
        "fandom",
        "wikia",
        "answers",
        "ask",
        "wikipedia",
        "wikiwand",
        "wikimili",
        "wikimedia",
        "wikidata",
        "everybodywiki",
        "dbpedia",
        "famousbirthdays",
        "celebritynetworth",
        "imdb",
        "pinterest",
        "tumblr",
        "scribd",
        "slideshare",
        "socialblade",
        "stackexchange",
        "stackoverflow",
        "apify",
        "rapidapi",
        "brainly",
        "chegg",
        "crunchbase",
        "ranker",
        "statista",
        "similarweb",
        "trustpilot",
        "glassdoor",
        "zoominfo",
        "rocketreach",
        "linktree",
        "substack",
        "blogspot",
        "wordpress",
        "weebly",
        "wix",
        "tiktokcounter",
        "livecounts",
        "socialcounts",
        "hypeauditor",
        "noxinfluencer",
        "influencermarketinghub",
    ];
    OUTLETS.contains(&site)
        || CITATION_SITES.contains(&site)
        || labels.iter().any(|label| {
            matches!(
                *label,
                "quora" | "fandom" | "wikia" | "wikipedia" | "answers"
            )
        })
        || call_sign
        || site.contains("news")
        || NEWS_WORDS
            .iter()
            .any(|word| site.ends_with(word) || (site.starts_with(word) && *word != "post"))
}

pub(crate) fn social_or_publisher(domain: &str) -> bool {
    let host = domain.trim_start_matches("www.").to_ascii_lowercase();
    account_platform_host(&host) || publisher_host(&host)
}

/// News/reference publisher host (citations), used when classifying Brain resource URLs.
pub(crate) fn is_publisher_host(host: &str) -> bool {
    let host = host.trim_start_matches("www.").to_ascii_lowercase();
    publisher_host(&host)
}
