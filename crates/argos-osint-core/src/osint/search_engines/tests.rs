//! Inline tests for the named-SERP outcome contract (spec §9).

use super::*;
use serde_json::json;

const GOOGLE_CARD: &str = r#"<div class="g"><a href="https://alpha.example.org/docs"><h3>Alpha heading</h3></a><div class="VwiC3b">Alpha snippet text.</div></div>"#;

fn request(tool: &str, query: &str, limit: usize) -> SerpRequest {
    SerpRequest::for_tool(tool, query, limit).expect("named engine request")
}

/// Builds a Firecrawl `/v2/scrape` envelope with the contract v2 formats.
fn envelope(dom_key: &str, dom: &str, source_url: &str, status: u16) -> Value {
    json!({
        "success": true,
        "data": {
            dom_key: dom,
            "links": ["https://link-one.example.org/"],
            "metadata": {
                "sourceURL": source_url,
                "statusCode": status
            }
        }
    })
}

fn google_dom(cards: &str) -> String {
    format!("<!DOCTYPE html><html><body><div id=\"search\">{cards}</div></body></html>")
}

fn parse_with(
    tool: &str,
    query: &str,
    limit: usize,
    dom_key: &str,
    dom: &str,
    source_url: &str,
    status: u16,
) -> SerpParse {
    let value = envelope(dom_key, dom, source_url, status);
    parse_serp_response(&request(tool, query, limit), &value).expect("parse never errors on a page")
}

fn parse_google(cards: &str, limit: usize) -> SerpParse {
    let url = build_serp_url(FIRECRAWL_GOOGLE_SEARCH, "rust ownership").expect("serp url");
    parse_with(
        FIRECRAWL_GOOGLE_SEARCH,
        "rust ownership",
        limit,
        "rawHtml",
        &google_dom(cards),
        &url,
        200,
    )
}

// ---------------------------------------------------------------------------
// URL / request construction
// ---------------------------------------------------------------------------

#[test]
fn serp_urls_are_engine_specific() {
    let g = build_serp_url(FIRECRAWL_GOOGLE_SEARCH, "elon musk tesla").expect("google");
    assert_eq!(g, "https://www.google.com/search?q=elon+musk+tesla");

    let y = build_serp_url(FIRECRAWL_YANDEX_SEARCH, "spacex launch").expect("yandex");
    assert_eq!(y, "https://yandex.com/search/?text=spacex+launch");

    let m = build_serp_url(FIRECRAWL_MOJEEK_SEARCH, "open source intelligence").expect("mojeek");
    assert_eq!(
        m,
        "https://www.mojeek.com/search?q=open+source+intelligence"
    );

    assert!(build_serp_url("firecrawl_search", "x").is_err());
}

#[test]
fn scrape_body_requests_raw_html_and_html_and_keeps_the_results_region() {
    let body = build_scrape_body(FIRECRAWL_GOOGLE_SEARCH, "rust").expect("body");
    let formats = body
        .get("formats")
        .and_then(Value::as_array)
        .expect("formats array");
    let names: Vec<&str> = formats.iter().filter_map(Value::as_str).collect();
    assert!(names.contains(&"rawHtml"), "{names:?}");
    assert!(names.contains(&"html"), "{names:?}");
    assert!(names.contains(&"links"), "{names:?}");
    assert_eq!(body.get("onlyMainContent"), Some(&Value::Bool(false)));
    assert_eq!(body.get("maxAge"), Some(&json!(0)));
    assert_eq!(
        body.get("url").and_then(Value::as_str),
        Some("https://www.google.com/search?q=rust")
    );
}

#[test]
fn named_engine_helpers_cover_the_three_tools() {
    assert!(is_named_engine_tool(FIRECRAWL_GOOGLE_SEARCH));
    assert!(is_named_engine_tool(FIRECRAWL_YANDEX_SEARCH));
    assert!(is_named_engine_tool(FIRECRAWL_MOJEEK_SEARCH));
    assert!(!is_named_engine_tool("firecrawl_search"));
    assert_eq!(NAMED_SERP_TOOLS.len(), 3);
    assert_eq!(engine_family(FIRECRAWL_GOOGLE_SEARCH), "named_serp");
    assert_eq!(engine_family(FIRECRAWL_YANDEX_SEARCH), "named_serp");
    // A fallback tool is never grouped as an engine.
    assert_ne!(engine_family("firecrawl_search"), "named_serp");
}

#[test]
fn request_carries_query_limit_engine_and_requested_url() {
    // Everything the parser records comes from the tool input: the engine, the
    // bounded limit, the verbatim query and the engine's own requested URL. A
    // redirected final URL never rewrites any of them (spec §9 fix 2).
    let query = "  mojeek   catalogue  ";
    let req = request(FIRECRAWL_MOJEEK_SEARCH, query, 7);
    assert_eq!(req.engine, "mojeek");
    assert_eq!(req.engine_tool, FIRECRAWL_MOJEEK_SEARCH);
    assert_eq!(req.query, query, "the original query is verbatim");
    assert_eq!(req.limit, 7, "the caller picks the bound");
    assert_eq!(
        req.serp_url,
        build_serp_url(FIRECRAWL_MOJEEK_SEARCH, query).expect("mojeek serp url"),
        "the requested URL is the engine's own, built from the tool input"
    );
    let requested = Url::parse(&req.serp_url).expect("requested url");
    assert_eq!(requested.host_str(), Some("www.mojeek.com"));
    assert_eq!(requested.path(), "/search");
    let (key, value) = requested.query_pairs().next().expect("q parameter");
    assert_eq!(key, "q");
    assert_eq!(value, query, "the URL carries the same query, not a copy");
    assert!(SerpRequest::for_tool("firecrawl_search", "x", 5).is_err());
}

#[test]
fn outcome_strings_round_trip() {
    for outcome in [
        SerpOutcome::Valid,
        SerpOutcome::VerifiedZero,
        SerpOutcome::Challenge,
        SerpOutcome::Consent,
        SerpOutcome::RateLimited,
        SerpOutcome::ParserMismatch,
        SerpOutcome::UpstreamFailure,
        SerpOutcome::ResponseTooLarge,
    ] {
        assert_eq!(SerpOutcome::parse(outcome.as_str()), Some(outcome));
    }
    // Legacy and unknown strings never round-trip.
    assert_eq!(SerpOutcome::parse("zero_results"), None);
    assert_eq!(SerpOutcome::parse("unknown_html"), None);
    assert!(SerpOutcome::Valid.is_success());
    assert!(SerpOutcome::VerifiedZero.is_success());
    assert!(SerpOutcome::Challenge.is_failure());
    assert!(SerpOutcome::ParserMismatch.is_failure());
    assert!(SerpOutcome::ResponseTooLarge.is_failure());
}

// ---------------------------------------------------------------------------
// Valid cards
// ---------------------------------------------------------------------------

#[test]
fn google_valid_cards_are_ranked_and_bounded_by_the_requested_limit() {
    let cards = format!(
        "{GOOGLE_CARD}<div class=\"g\"><a href=\"https://beta.example.org/docs\"><h3>Beta heading</h3></a><div class=\"VwiC3b\">Beta snippet text.</div></div><div class=\"g\"><a href=\"https://gamma.example.org/docs\"><h3>Gamma heading</h3></a><div class=\"VwiC3b\">Gamma snippet text.</div></div>"
    );
    let parsed = parse_google(&cards, 2);
    assert_eq!(parsed.outcome, SerpOutcome::Valid);
    assert_eq!(parsed.items.len(), 2);
    assert_eq!(
        parsed.items[0].destination,
        "https://alpha.example.org/docs"
    );
    assert_eq!(parsed.items[0].title, "Alpha heading");
    assert_eq!(parsed.items[0].snippet, "Alpha snippet text.");
    assert_eq!(parsed.items[0].rank, 1);
    assert_eq!(parsed.items[1].rank, 2);
    assert_eq!(parsed.items[1].destination, "https://beta.example.org/docs");
    assert_eq!(parsed.diagnostics.accepted, 2);
    assert_eq!(parsed.diagnostics.parser_input, ParserInput::RawHtml);
    assert_eq!(parsed.items[0].parser_version, PARSER_VERSION);
}

#[test]
fn yandex_valid_cards_keep_title_and_snippet() {
    let dom = r#"<!DOCTYPE html><html><body><ul class="serp-list"><li class="serp-item"><div class="Organic"><h2><a class="OrganicTitle-Link" href="https://target-yandex.example.org/doc">Target Document</a></h2><div class="OrganicTextContent">Yandex snippet describing the document.</div></div></li></ul></body></html>"#;
    let url = build_serp_url(FIRECRAWL_YANDEX_SEARCH, "yandex query").expect("url");
    let parsed = parse_with(
        FIRECRAWL_YANDEX_SEARCH,
        "yandex query",
        5,
        "rawHtml",
        dom,
        &url,
        200,
    );
    assert_eq!(parsed.outcome, SerpOutcome::Valid);
    assert_eq!(parsed.items.len(), 1);
    assert_eq!(parsed.items[0].engine, "yandex");
    assert_eq!(parsed.items[0].title, "Target Document");
    assert_eq!(
        parsed.items[0].snippet,
        "Yandex snippet describing the document."
    );
}

#[test]
fn mojeek_valid_cards_keep_title_and_snippet() {
    let dom = r#"<!DOCTYPE html><html><body><div id="results"><ul class="results-standard"><li><h2><a class="title" href="https://mojeek-hit.example.org/info">Mojeek Result</a></h2><p class="s">Mojeek independent index snippet.</p></li></ul></div></body></html>"#;
    let url = build_serp_url(FIRECRAWL_MOJEEK_SEARCH, "mojeek query").expect("url");
    let parsed = parse_with(
        FIRECRAWL_MOJEEK_SEARCH,
        "mojeek query",
        5,
        "rawHtml",
        dom,
        &url,
        200,
    );
    assert_eq!(parsed.outcome, SerpOutcome::Valid);
    assert_eq!(parsed.items.len(), 1);
    assert_eq!(
        parsed.items[0].destination,
        "https://mojeek-hit.example.org/info"
    );
    assert_eq!(parsed.items[0].title, "Mojeek Result");
    assert_eq!(parsed.items[0].snippet, "Mojeek independent index snippet.");
}

#[test]
fn raw_html_is_preferred_over_cleaned_html() {
    let raw = r#"<div class="g"><a href="https://raw-wins.example.org/"><h3>Raw heading</h3></a><div class="VwiC3b">Raw snippet.</div></div>"#;
    let value = json!({
        "success": true,
        "data": {
            "rawHtml": raw,
            "html": r#"<div class="g"><a href="https://clean-loses.example.org/"><h3>Clean heading</h3></a></div>"#,
            "metadata": { "sourceURL": "https://www.google.com/search?q=raw", "statusCode": 200 }
        }
    });
    let parsed =
        parse_serp_response(&request(FIRECRAWL_GOOGLE_SEARCH, "raw", 5), &value).expect("parse");
    assert_eq!(parsed.outcome, SerpOutcome::Valid);
    assert_eq!(parsed.diagnostics.parser_input, ParserInput::RawHtml);
    assert_eq!(parsed.items[0].destination, "https://raw-wins.example.org/");
    assert_eq!(parsed.items[0].title, "Raw heading");
}

#[test]
fn cleaned_html_is_used_when_raw_html_is_absent_or_empty() {
    let clean = r#"<div class="g"><a href="https://clean-fallback.example.org/"><h3>Clean heading</h3></a><div class="VwiC3b">Clean snippet.</div></div>"#;
    for value in [
        json!({
            "success": true,
            "data": {
                "html": clean,
                "metadata": { "sourceURL": "https://www.google.com/search?q=clean", "statusCode": 200 }
            }
        }),
        json!({
            "success": true,
            "data": {
                "rawHtml": "   ",
                "html": clean,
                "metadata": { "sourceURL": "https://www.google.com/search?q=clean", "statusCode": 200 }
            }
        }),
    ] {
        let parsed = parse_serp_response(&request(FIRECRAWL_GOOGLE_SEARCH, "clean", 5), &value)
            .expect("parse");
        assert_eq!(parsed.outcome, SerpOutcome::Valid);
        assert_eq!(parsed.diagnostics.parser_input, ParserInput::CleanHtml);
        assert_eq!(
            parsed.items[0].destination,
            "https://clean-fallback.example.org/"
        );
    }
}

#[test]
fn headings_and_links_survive_missing_card_classes() {
    // Google's `div.g` is absent: the conservative `h3 a` fallback keeps the result.
    let dom = r#"<!DOCTYPE html><html><body><div class="unknown-drifted-shape"><h3><a href="https://fallback.example.org/doc">Fallback heading</a></h3></div></body></html>"#;
    let url = build_serp_url(FIRECRAWL_GOOGLE_SEARCH, "drift").expect("url");
    let parsed = parse_with(
        FIRECRAWL_GOOGLE_SEARCH,
        "drift",
        5,
        "rawHtml",
        dom,
        &url,
        200,
    );
    assert_eq!(parsed.outcome, SerpOutcome::Valid);
    assert_eq!(parsed.items.len(), 1);
    assert_eq!(parsed.items[0].title, "Fallback heading");
    assert!(parsed.items[0].snippet.is_empty());
}

// ---------------------------------------------------------------------------
// Truthful outcomes: unknown DOM, zeros, boundaries
// ---------------------------------------------------------------------------

#[test]
fn malformed_and_missing_html_is_a_parser_mismatch_never_a_zero() {
    let url = build_serp_url(FIRECRAWL_GOOGLE_SEARCH, "broken").expect("url");
    for value in [
        json!({ "success": true, "data": { "metadata": { "sourceURL": url, "statusCode": 200 } } }),
        json!({ "success": true, "data": { "rawHtml": "   ", "html": "", "metadata": { "sourceURL": url, "statusCode": 200 } } }),
        json!({ "success": true, "data": {} }),
    ] {
        let parsed = parse_serp_response(&request(FIRECRAWL_GOOGLE_SEARCH, "broken", 5), &value)
            .expect("parse");
        assert_eq!(parsed.outcome, SerpOutcome::ParserMismatch);
        assert!(parsed.items.is_empty());
        assert_ne!(parsed.outcome, SerpOutcome::VerifiedZero);
    }
}

#[test]
fn unrecognized_but_non_empty_dom_is_a_parser_mismatch() {
    // No card selectors match, and no recognized engine status region exists.
    let dom = r#"<!DOCTYPE html><html><body><div id="app"><p>We scanned everything and found no results for you today.</p><script>var zero = 0;</script></div></body></html>"#;
    let parsed = parse_google(dom, 5);
    assert_eq!(parsed.outcome, SerpOutcome::ParserMismatch);
    assert!(parsed.items.is_empty());
    assert_eq!(parsed.diagnostics.dom_candidates, 0);
}

#[test]
fn an_empty_dom_without_any_payload_is_a_parser_mismatch() {
    let url = build_serp_url(FIRECRAWL_GOOGLE_SEARCH, "empty").expect("url");
    let value = json!({
        "success": true,
        "data": { "rawHtml": "", "html": "", "metadata": { "sourceURL": url, "statusCode": 200 } }
    });
    let parsed =
        parse_serp_response(&request(FIRECRAWL_GOOGLE_SEARCH, "empty", 5), &value).expect("parse");
    assert_eq!(parsed.outcome, SerpOutcome::ParserMismatch);
    assert_eq!(parsed.diagnostics.parser_input, ParserInput::Missing);
    assert_ne!(parsed.outcome, SerpOutcome::VerifiedZero);
}

#[test]
fn links_only_payload_is_a_parser_mismatch_and_never_a_zero() {
    let value = json!({
        "success": true,
        "data": {
            "links": ["https://a.example.org/", "https://b.example.org/"],
            "metadata": { "sourceURL": "https://www.mojeek.com/search?q=links", "statusCode": 200 }
        }
    });
    let parsed =
        parse_serp_response(&request(FIRECRAWL_MOJEEK_SEARCH, "links", 5), &value).expect("parse");
    assert_eq!(parsed.outcome, SerpOutcome::ParserMismatch);
    assert_eq!(parsed.diagnostics.parser_input, ParserInput::LinksOnly);
    assert!(parsed.items.is_empty());
    assert!(parsed.diagnostics.input_bytes > 0);
    // A links-only shape must not trigger a fresh retry.
    assert!(!retry_allowed(parsed.outcome, &parsed.diagnostics));
}

#[test]
fn mojeek_pages_labelled_ten_and_one_hundred_results_are_valid() {
    for label in ["10 results", "100 results", "1,000 results"] {
        let dom = format!(
            r#"<!DOCTYPE html><html><body><div id="results"><h1>{label} for mojeek catalogue</h1><ul class="results-standard"><li><h2><a class="title" href="https://count.example.org/">Counted Result</a></h2><p class="s">Counted snippet.</p></li></ul></div></body></html>"#
        );
        let url = build_serp_url(FIRECRAWL_MOJEEK_SEARCH, "mojeek catalogue").expect("url");
        let parsed = parse_with(
            FIRECRAWL_MOJEEK_SEARCH,
            "mojeek catalogue",
            5,
            "rawHtml",
            &dom,
            &url,
            200,
        );
        assert_eq!(parsed.outcome, SerpOutcome::Valid, "label {label}");
        assert_eq!(parsed.items.len(), 1, "label {label}");
        assert_ne!(parsed.outcome, SerpOutcome::VerifiedZero);
    }
}

#[test]
fn a_standalone_zero_count_in_a_status_region_is_a_verified_zero() {
    let dom = r#"<!DOCTYPE html><html><body><div id="results"><h1>0 results for mojeek catalogue</h1><ul class="results-standard"></ul></div></body></html>"#;
    let url = build_serp_url(FIRECRAWL_MOJEEK_SEARCH, "mojeek catalogue").expect("url");
    let parsed = parse_with(
        FIRECRAWL_MOJEEK_SEARCH,
        "mojeek catalogue",
        5,
        "rawHtml",
        dom,
        &url,
        200,
    );
    assert_eq!(parsed.outcome, SerpOutcome::VerifiedZero);
    assert!(parsed.items.is_empty());
}

#[test]
fn count_boundaries_are_respected_when_reading_a_zero() {
    for label in ["10 results", "100 results", "1,000 results", "1 result"] {
        let dom = format!(
            r#"<!DOCTYPE html><html><body><div id="results"><h1>{label} for mojeek catalogue</h1><ul class="results-standard"></ul></div></body></html>"#
        );
        let url = build_serp_url(FIRECRAWL_MOJEEK_SEARCH, "mojeek catalogue").expect("url");
        let parsed = parse_with(
            FIRECRAWL_MOJEEK_SEARCH,
            "mojeek catalogue",
            5,
            "rawHtml",
            &dom,
            &url,
            200,
        );
        assert_eq!(parsed.outcome, SerpOutcome::ParserMismatch, "label {label}");
        assert_ne!(parsed.outcome, SerpOutcome::VerifiedZero);
    }
}

#[test]
fn yandex_snippet_mentioning_not_found_is_valid() {
    let dom = r#"<!DOCTYPE html><html><body><ul class="serp-list"><li class="serp-item"><div class="Organic"><h2><a class="OrganicTitle-Link" href="https://httpguides.example.org/errors/">File not found: a field guide</a></h2><div class="OrganicTextContent">Why a server answers that a file is not found, and what the code means.</div></div></li></ul></body></html>"#;
    let url = build_serp_url(FIRECRAWL_YANDEX_SEARCH, "file not found").expect("url");
    let parsed = parse_with(
        FIRECRAWL_YANDEX_SEARCH,
        "file not found",
        5,
        "rawHtml",
        dom,
        &url,
        200,
    );
    assert_eq!(parsed.outcome, SerpOutcome::Valid);
    assert_eq!(parsed.items.len(), 1);
}

#[test]
fn snippets_discussing_captcha_do_not_look_like_a_challenge() {
    // Each trigger word lives only inside this result card's heading and
    // snippet, and the link host is an ordinary page host. A challenge or
    // consent wall is a *structural* shape — a form, an interstitial host, an
    // iframe, a meta refresh — never a substring scan over the page's own text
    // (spec §9 fix 4), so this page stays `Valid` with its card extracted.
    let dom = r#"<!DOCTYPE html><html><body><ul class="serp-list"><li class="serp-item"><div class="Organic"><h2><a class="OrganicTitle-Link" href="https://security.example.org/anti-bot-forms">How captcha forms work</a></h2><div class="OrganicTextContent">A field guide explaining a captcha form, why a server answers 404 not found, and what "0 results" really proves.</div></div></li></ul></body></html>"#;
    let url = build_serp_url(FIRECRAWL_YANDEX_SEARCH, "captcha").expect("url");
    let parsed = parse_with(
        FIRECRAWL_YANDEX_SEARCH,
        "captcha",
        5,
        "rawHtml",
        dom,
        &url,
        200,
    );
    assert_eq!(parsed.outcome, SerpOutcome::Valid);
    assert_eq!(parsed.items.len(), 1);
    assert_eq!(parsed.items[0].title, "How captcha forms work");
    let snippet = &parsed.items[0].snippet;
    assert!(snippet.contains("captcha"), "{snippet}");
    assert_eq!(parsed.items[0].rank, 1);
    assert_eq!(parsed.diagnostics.accepted, 1);
    // No structural interstitial was found, so no status region and no wall.
    assert!(parsed.diagnostics.status_region.is_empty());
    assert_ne!(parsed.outcome, SerpOutcome::Challenge);
    assert_ne!(parsed.outcome, SerpOutcome::Consent);
    assert_ne!(parsed.outcome, SerpOutcome::VerifiedZero);
    // Nothing about the phrases may reach a zero, a block or a retry.
    assert!(!retry_allowed(parsed.outcome, &parsed.diagnostics));
}

#[test]
fn google_no_results_status_region_is_a_verified_zero() {
    let dom = r#"<!DOCTYPE html><html><body><div id="topstuff"><p>Your search - <b>kajshdkjashdkjahskd</b> - did not match any documents.</p></div><div id="main"></div></body></html>"#;
    let url = build_serp_url(FIRECRAWL_GOOGLE_SEARCH, "kajshdkjashdkjahskd").expect("url");
    let parsed = parse_with(
        FIRECRAWL_GOOGLE_SEARCH,
        "kajshdkjashdkjahskd",
        5,
        "rawHtml",
        dom,
        &url,
        200,
    );
    assert_eq!(parsed.outcome, SerpOutcome::VerifiedZero);
    assert!(parsed.items.is_empty());
    assert!(parsed.diagnostics.status_region.contains("#topstuff"));
    assert!(parsed.outcome.is_success());
}

#[test]
fn yandex_and_mojeek_no_results_regions_are_verified_zeros() {
    let yandex = r#"<!DOCTYPE html><html><body><div class="misspell__message">Ничего не нашлось по этому запросу</div></body></html>"#;
    let yurl = build_serp_url(FIRECRAWL_YANDEX_SEARCH, "несуществующий запрос").expect("url");
    let parsed = parse_with(
        FIRECRAWL_YANDEX_SEARCH,
        "несуществующий запрос",
        5,
        "rawHtml",
        yandex,
        &yurl,
        200,
    );
    assert_eq!(parsed.outcome, SerpOutcome::VerifiedZero);

    let mojeek = r#"<!DOCTYPE html><html><body><div id="results"><p>No results found for this query.</p></div></body></html>"#;
    let murl = build_serp_url(FIRECRAWL_MOJEEK_SEARCH, "zzzznope").expect("url");
    let parsed = parse_with(
        FIRECRAWL_MOJEEK_SEARCH,
        "zzzznope",
        5,
        "rawHtml",
        mojeek,
        &murl,
        200,
    );
    assert_eq!(parsed.outcome, SerpOutcome::VerifiedZero);
    assert!(parsed.diagnostics.status_region.contains("#results"));
}

#[test]
fn a_zero_phrase_outside_a_status_region_is_not_a_verified_zero() {
    let dom = r#"<!DOCTYPE html><html><body><main><p>No results found for your query.</p></main></body></html>"#;
    let parsed = parse_google(dom, 5);
    assert_eq!(parsed.outcome, SerpOutcome::ParserMismatch);
}

// ---------------------------------------------------------------------------
// Challenge / consent: structural evidence only
// ---------------------------------------------------------------------------

#[test]
fn google_sorry_form_is_a_challenge() {
    let dom = r#"<!DOCTYPE html><html><body><form action="/sorry/index?continue=https://www.google.com/search%3Fq%3Dblocked" method="GET"><h1>Our systems have detected unusual traffic</h1><input name="q" type="text"></form></body></html>"#;
    let url = build_serp_url(FIRECRAWL_GOOGLE_SEARCH, "blocked").expect("url");
    let parsed = parse_with(
        FIRECRAWL_GOOGLE_SEARCH,
        "blocked",
        5,
        "rawHtml",
        dom,
        &url,
        200,
    );
    assert_eq!(parsed.outcome, SerpOutcome::Challenge);
    assert!(parsed.items.is_empty());
    assert_eq!(parsed.diagnostics.status_region, "interstitial");
    assert!(parsed.outcome.is_failure());
}

#[test]
fn recaptcha_widget_element_is_a_challenge() {
    let dom = r#"<!DOCTYPE html><html><body><div class="g-recaptcha" data-sitekey="sanitized-fixture-key"></div><noscript>Enable JavaScript to continue.</noscript></body></html>"#;
    let url = build_serp_url(FIRECRAWL_GOOGLE_SEARCH, "captcha").expect("url");
    let parsed = parse_with(
        FIRECRAWL_GOOGLE_SEARCH,
        "captcha",
        5,
        "rawHtml",
        dom,
        &url,
        200,
    );
    assert_eq!(parsed.outcome, SerpOutcome::Challenge);
}

#[test]
fn yandex_smartcaptcha_form_is_a_challenge() {
    let dom = r#"<!DOCTYPE html><html><body><form action="https://yandex.com/search/?captcha=smart" method="GET"><div class="SmartCaptcha-Domains"></div></form></body></html>"#;
    let url = build_serp_url(FIRECRAWL_YANDEX_SEARCH, "blocked").expect("url");
    let parsed = parse_with(
        FIRECRAWL_YANDEX_SEARCH,
        "blocked",
        5,
        "rawHtml",
        dom,
        &url,
        200,
    );
    assert_eq!(parsed.outcome, SerpOutcome::Challenge);
}

#[test]
fn consent_interstitial_is_consent_not_a_challenge() {
    let dom = r#"<!DOCTYPE html><html><body><h1>Before you continue to Google</h1><form action="https://consent.google.com/save" method="POST"><p>We use cookies to deliver services.</p><button type="submit">Accept all</button></form></body></html>"#;
    let url = build_serp_url(FIRECRAWL_GOOGLE_SEARCH, "consent").expect("url");
    let parsed = parse_with(
        FIRECRAWL_GOOGLE_SEARCH,
        "consent",
        5,
        "rawHtml",
        dom,
        &url,
        200,
    );
    assert_eq!(parsed.outcome, SerpOutcome::Consent);
    assert_ne!(parsed.outcome, SerpOutcome::Challenge);
}

#[test]
fn consent_iframe_is_consent() {
    let dom = r#"<!DOCTYPE html><html><body><iframe src="https://consent.youtube.com/ml?continue=https%3A%2F%2Fwww.google.com"></iframe></body></html>"#;
    let url = build_serp_url(FIRECRAWL_YANDEX_SEARCH, "consent").expect("url");
    let parsed = parse_with(
        FIRECRAWL_YANDEX_SEARCH,
        "consent",
        5,
        "rawHtml",
        dom,
        &url,
        200,
    );
    assert_eq!(parsed.outcome, SerpOutcome::Consent);
}

#[test]
fn meta_refresh_to_a_consent_host_is_consent() {
    // A structural meta-refresh interstitial (spec §9 fix 4): the redirect
    // target is a consent host, so the page is a wall — never a parser problem
    // and never a zero.
    let dom = r#"<!DOCTYPE html><html><head><meta http-equiv="refresh" content="0;url=https://consent.google.com/save?continue=https%3A%2F%2Fwww.google.com"></head><body></body></html>"#;
    let url = build_serp_url(FIRECRAWL_GOOGLE_SEARCH, "consent").expect("url");
    let parsed = parse_with(
        FIRECRAWL_GOOGLE_SEARCH,
        "consent",
        5,
        "rawHtml",
        dom,
        &url,
        200,
    );
    assert_eq!(parsed.outcome, SerpOutcome::Consent);
    assert_ne!(parsed.outcome, SerpOutcome::Challenge);
    assert!(parsed.items.is_empty());
    assert!(!parsed.outcome.is_success());
    assert!(!retry_allowed(parsed.outcome, &parsed.diagnostics));
}

#[test]
fn a_noscript_wrapped_consent_meta_refresh_is_consent() {
    // The real European-Google wall wraps the tag in `<noscript>`. html5ever runs
    // with scripting enabled, so `<noscript>` becomes one raw-text node and the
    // meta element never reaches the tree: `detect_interstitial` reads that text
    // node as well as the element selector, or this variant would fall through to
    // an unknown layout.
    let dom = r#"<!DOCTYPE html><html><head><noscript><meta http-equiv="refresh" content="0;url=https://consent.google.com/save?continue=https%3A%2F%2Fwww.google.com"></noscript></head><body><h1>Before you continue to Google</h1></body></html>"#;
    let url = build_serp_url(FIRECRAWL_GOOGLE_SEARCH, "noscript").expect("url");
    let parsed = parse_with(
        FIRECRAWL_GOOGLE_SEARCH,
        "noscript",
        5,
        "rawHtml",
        dom,
        &url,
        200,
    );
    assert_eq!(parsed.outcome, SerpOutcome::Consent);
    assert_ne!(parsed.outcome, SerpOutcome::Challenge);
    assert!(parsed.items.is_empty());
    assert!(!parsed.outcome.is_success());
    assert!(!retry_allowed(parsed.outcome, &parsed.diagnostics));

    // A noscript block that only preserves a redirect (no consent host) is not
    // evidence: the layout stays unknown rather than becoming a wall.
    let unrelated = r#"<!DOCTYPE html><html><head><noscript><meta http-equiv="refresh" content="0;url=https://www.google.com/sorry"></noscript></head><body></body></html>"#;
    let unrelated = parse_with(
        FIRECRAWL_GOOGLE_SEARCH,
        "noscript",
        5,
        "rawHtml",
        unrelated,
        &url,
        200,
    );
    assert_eq!(unrelated.outcome, SerpOutcome::ParserMismatch);
}

#[test]
fn redirect_to_a_consent_host_is_consent() {
    let consent_url = "https://consent.google.com/save?continue=https%3A%2F%2Fwww.google.com%2Fsearch%3Fq%3Dredirected&gl=GB&hl=en";
    let dom = r#"<!DOCTYPE html><html><body><h1>Before you continue</h1><form action="/save" method="POST"></form></body></html>"#;
    let _url = build_serp_url(FIRECRAWL_GOOGLE_SEARCH, "redirected").expect("url");
    let parsed = parse_with(
        FIRECRAWL_GOOGLE_SEARCH,
        "redirected",
        5,
        "rawHtml",
        dom,
        consent_url,
        200,
    );
    assert_eq!(parsed.outcome, SerpOutcome::Consent);
    assert!(parsed
        .diagnostics
        .final_url
        .starts_with("https://consent.google.com/"));
    // The original query is preserved even though the page redirected.
    assert!(parsed.diagnostics.requested_url.contains("q=redirected"));
}

// ---------------------------------------------------------------------------
// Envelope and target-status failures
// ---------------------------------------------------------------------------

#[test]
fn envelope_success_false_is_an_upstream_failure_not_a_zero() {
    let value = json!({ "success": false, "error": "Unauthorized: invalid API key" });
    let parsed =
        parse_serp_response(&request(FIRECRAWL_GOOGLE_SEARCH, "q", 5), &value).expect("parse");
    assert_eq!(parsed.outcome, SerpOutcome::UpstreamFailure);
    assert!(parsed.items.is_empty());
    assert!(parsed.diagnostics.provider_error.contains("Unauthorized"));
}

#[test]
fn api_http_404_is_an_upstream_failure() {
    let value = json!({
        "success": false,
        "code": 404,
        "error": "Page not found"
    });
    let parsed =
        parse_serp_response(&request(FIRECRAWL_MOJEEK_SEARCH, "q", 5), &value).expect("parse");
    assert_eq!(parsed.outcome, SerpOutcome::UpstreamFailure);
    assert!(parsed.items.is_empty());
}

#[test]
fn api_http_404_with_a_success_envelope_is_an_upstream_failure() {
    let value = json!({
        "success": true,
        "data": { "metadata": { "sourceURL": "https://www.mojeek.com/search?q=q", "statusCode": 404 } }
    });
    let parsed =
        parse_serp_response(&request(FIRECRAWL_MOJEEK_SEARCH, "q", 5), &value).expect("parse");
    assert_eq!(parsed.outcome, SerpOutcome::UpstreamFailure);
    assert!(parsed.diagnostics.provider_error.contains("404"));
}

#[test]
fn explicit_provider_error_is_an_upstream_failure() {
    let value = json!({
        "success": true,
        "data": {
            "metadata": { "sourceURL": "https://www.google.com/search?q=boom", "statusCode": 200, "error": "Scrape failed: rendering timeout" }
        }
    });
    let parsed =
        parse_serp_response(&request(FIRECRAWL_GOOGLE_SEARCH, "boom", 5), &value).expect("parse");
    assert_eq!(parsed.outcome, SerpOutcome::UpstreamFailure);
    assert!(parsed
        .diagnostics
        .provider_error
        .contains("rendering timeout"));
}

#[test]
fn target_status_failures_behind_an_outer_http_200_are_truthful() {
    let url = build_serp_url(FIRECRAWL_GOOGLE_SEARCH, "status").expect("url");
    let cases = [
        (403_u16, SerpOutcome::UpstreamFailure),
        (500_u16, SerpOutcome::UpstreamFailure),
        (503_u16, SerpOutcome::UpstreamFailure),
        (429_u16, SerpOutcome::RateLimited),
        (200_u16, SerpOutcome::Valid),
    ];
    for (status, expected) in cases {
        let parsed = parse_with(
            FIRECRAWL_GOOGLE_SEARCH,
            "status",
            5,
            "rawHtml",
            r#"<div class="g"><a href="https://status.example.org/"><h3>Status heading</h3></a></div>"#,
            &url,
            status,
        );
        assert_eq!(parsed.outcome, expected, "target status {status}");
        assert_eq!(parsed.diagnostics.target_status, Some(status));
    }
}

#[test]
fn provider_warnings_are_recorded_without_leaking_the_page() {
    let warning = "w".repeat(400);
    let value = json!({
        "success": true,
        "data": {
            "rawHtml": "<div></div>",
            "warning": warning,
            "metadata": { "sourceURL": "https://www.google.com/search?q=warn", "statusCode": 200 }
        }
    });
    let parsed =
        parse_serp_response(&request(FIRECRAWL_GOOGLE_SEARCH, "warn", 5), &value).expect("parse");
    assert_eq!(
        parsed.diagnostics.provider_warning.chars().count(),
        MAX_DIAGNOSTIC_TEXT
    );
    assert!(!parsed.diagnostics.provider_warning.contains('<'));
}

#[test]
fn truncated_envelope_is_a_response_too_large_failure() {
    let value = json!({
        "success": true,
        "data": {
            "rawHtml": r#"<div class="g"><a href="https://trunc.example.org/"><h3>Trunc</h3></a></div>"#,
            "truncated": true,
            "metadata": { "sourceURL": "https://www.google.com/search?q=trunc", "statusCode": 200 }
        }
    });
    let parsed =
        parse_serp_response(&request(FIRECRAWL_GOOGLE_SEARCH, "trunc", 5), &value).expect("parse");
    assert_eq!(parsed.outcome, SerpOutcome::ResponseTooLarge);
    assert!(parsed.items.is_empty());
    // A truncated envelope must never be parsed as a valid empty page.
    assert_ne!(parsed.outcome, SerpOutcome::VerifiedZero);
    assert_ne!(parsed.outcome, SerpOutcome::Valid);
}

#[test]
fn oversize_input_is_a_response_too_large_failure() {
    let url = build_serp_url(FIRECRAWL_GOOGLE_SEARCH, "big").expect("url");
    let big = "a".repeat(SERP_MAX_BODY_BYTES + 1);
    let parsed = parse_with(
        FIRECRAWL_GOOGLE_SEARCH,
        "big",
        5,
        "rawHtml",
        &big,
        &url,
        200,
    );
    assert_eq!(parsed.outcome, SerpOutcome::ResponseTooLarge);
    assert!(parsed.diagnostics.input_bytes > SERP_MAX_BODY_BYTES);
}

// ---------------------------------------------------------------------------
// Request context, final-URL validation, missing metadata
// ---------------------------------------------------------------------------

#[test]
fn missing_metadata_keeps_the_original_query_and_engine() {
    let value = json!({
        "success": true,
        "data": { "rawHtml": GOOGLE_CARD }
    });
    let original = "  quoted   \"query\"  ";
    let parsed =
        parse_serp_response(&request(FIRECRAWL_GOOGLE_SEARCH, original, 5), &value).expect("parse");
    assert_eq!(parsed.outcome, SerpOutcome::Valid);
    assert_eq!(parsed.items.len(), 1);
    // The recorded query is the request context, never a URL reconstruction.
    assert_eq!(parsed.items[0].query, original);
    assert_eq!(parsed.diagnostics.requested_url, req_serp_url_for(original));
    assert!(parsed.items[0].serp_url.contains("q="));
}

fn req_serp_url_for(query: &str) -> String {
    build_serp_url(FIRECRAWL_GOOGLE_SEARCH, query).expect("serp url")
}

#[test]
fn a_redirected_final_url_is_not_used_to_rebuild_the_query() {
    let value = json!({
        "success": true,
        "data": {
            "rawHtml": GOOGLE_CARD,
            "metadata": {
                "sourceURL": "https://www.google.com/search?q=totally+different+redirected+query&sa=U",
                "statusCode": 200
            }
        }
    });
    let parsed = parse_serp_response(
        &request(FIRECRAWL_GOOGLE_SEARCH, "rust ownership", 5),
        &value,
    )
    .expect("parse");
    assert_eq!(parsed.outcome, SerpOutcome::Valid);
    assert_eq!(parsed.items[0].query, "rust ownership");
    assert_ne!(parsed.items[0].query, "totally different redirected query");
}

#[test]
fn an_off_host_final_url_with_cards_keeps_the_items_but_is_not_valid() {
    let parsed = parse_with(
        FIRECRAWL_GOOGLE_SEARCH,
        "mirror",
        5,
        "rawHtml",
        &google_dom(GOOGLE_CARD),
        "https://mirror.example.org/search?q=mirror",
        200,
    );
    assert_eq!(parsed.outcome, SerpOutcome::UpstreamFailure);
    assert_eq!(parsed.items.len(), 1);
    assert!(parsed.diagnostics.status_region.contains("off_host"));
    assert_eq!(parsed.items[0].engine, "google");
    assert_eq!(parsed.items[0].query, "mirror");
}

#[test]
fn an_off_host_final_url_without_cards_is_never_a_zero() {
    let parsed = parse_with(
        FIRECRAWL_GOOGLE_SEARCH,
        "mirror",
        5,
        "rawHtml",
        &google_dom("<div class=\"unknown\"><p>Nothing recognized here.</p></div>"),
        "https://mirror.example.org/search?q=mirror",
        200,
    );
    assert_eq!(parsed.outcome, SerpOutcome::UpstreamFailure);
    assert_ne!(parsed.outcome, SerpOutcome::VerifiedZero);
}

#[test]
fn a_non_serp_path_on_the_engine_host_is_not_a_serp_page() {
    let _url = build_serp_url(FIRECRAWL_GOOGLE_SEARCH, "settings").expect("url");
    let parsed = parse_with(
        FIRECRAWL_GOOGLE_SEARCH,
        "settings",
        5,
        "rawHtml",
        &google_dom(GOOGLE_CARD),
        "https://www.google.com/preferences",
        200,
    );
    assert_eq!(parsed.outcome, SerpOutcome::UpstreamFailure);
}

// ---------------------------------------------------------------------------
// URL normalization
// ---------------------------------------------------------------------------

#[test]
fn absolute_relative_and_protocol_relative_links_resolve_against_the_serp() {
    let base =
        Url::parse("https://results.example.org/search?q=relative&page=2").expect("base url");
    let mut diag = SerpDiagnostics::default();
    assert_eq!(
        normalize_destination(
            "https://absolute.example.org/a/b",
            "google",
            &base,
            &mut diag
        )
        .as_deref(),
        Some("https://absolute.example.org/a/b")
    );
    assert_eq!(
        normalize_destination("/relative/path", "google", &base, &mut diag).as_deref(),
        Some("https://results.example.org/relative/path")
    );
    assert_eq!(
        normalize_destination("//protocol.example.org/x", "google", &base, &mut diag).as_deref(),
        Some("https://protocol.example.org/x")
    );
    assert_eq!(
        normalize_destination("../up/path", "google", &base, &mut diag).as_deref(),
        Some("https://results.example.org/up/path")
    );
}

#[test]
fn destination_query_parameters_and_fragments_are_handled() {
    let base = Url::parse("https://results.example.org/search?q=keep").expect("base url");
    let mut diag = SerpDiagnostics::default();
    assert_eq!(
        normalize_destination(
            "https://keep.example.org/path?a=1&b=two+words&empty=#section",
            "google",
            &base,
            &mut diag
        )
        .as_deref(),
        Some("https://keep.example.org/path?a=1&b=two+words&empty=")
    );
}

#[test]
fn google_url_query_wrapper_is_unwrapped_once() {
    let wrapper = "https://www.google.com/url?q=https%3A%2F%2Fwrapped.example.org%2Fpage%3Fsig%3D42%26x%3D1&sa=U&ved=abc";
    let dom = format!(
        r#"<div class="g"><a href="{wrapper}"><h3>Wrapped heading</h3></a><div class="VwiC3b">Wrapped snippet.</div></div>"#
    );
    let url = build_serp_url(FIRECRAWL_GOOGLE_SEARCH, "wrapper").expect("url");
    let parsed = parse_with(
        FIRECRAWL_GOOGLE_SEARCH,
        "wrapper",
        5,
        "rawHtml",
        &google_dom(&dom),
        &url,
        200,
    );
    assert_eq!(parsed.outcome, SerpOutcome::Valid);
    assert_eq!(parsed.items.len(), 1);
    assert_eq!(
        parsed.items[0].destination,
        "https://wrapped.example.org/page?sig=42&x=1"
    );
    // The wrapper itself is never reported as a destination.
    assert!(!parsed.items[0].destination.contains("google.com/url"));
}

#[test]
fn yandex_clck_wrapper_and_mojeek_redir_wrapper_are_unwrapped() {
    let yandex = r#"<li class="serp-item"><div class="Organic"><h2><a class="OrganicTitle-Link" href="https://yandex.com/clck/redir/*?url=https%3A%2F%2Fclicked.example.org%2Fyandex&sig=zz">Clicked</a></h2></div></li>"#;
    let yurl = build_serp_url(FIRECRAWL_YANDEX_SEARCH, "wrapper").expect("url");
    let parsed = parse_with(
        FIRECRAWL_YANDEX_SEARCH,
        "wrapper",
        5,
        "rawHtml",
        &format!("<html><body><ul class=\"serp-list\">{yandex}</ul></body></html>"),
        &yurl,
        200,
    );
    assert_eq!(parsed.outcome, SerpOutcome::Valid);
    assert_eq!(
        parsed.items[0].destination,
        "https://clicked.example.org/yandex"
    );

    let mojeek = r#"<li><h2><a class="title" href="https://www.mojeek.com/redir?q=https%3A%2F%2Fclicked.example.org%2Fmojeek">Clicked</a></h2></li>"#;
    let murl = build_serp_url(FIRECRAWL_MOJEEK_SEARCH, "wrapper").expect("url");
    let parsed = parse_with(
        FIRECRAWL_MOJEEK_SEARCH,
        "wrapper",
        5,
        "rawHtml",
        &format!(
            "<html><body><div id=\"results\"><ul class=\"results-standard\">{mojeek}</ul></div></body></html>"
        ),
        &murl,
        200,
    );
    assert_eq!(parsed.outcome, SerpOutcome::Valid);
    assert_eq!(
        parsed.items[0].destination,
        "https://clicked.example.org/mojeek"
    );
}

#[test]
fn engine_wrappers_without_a_target_are_rejected_not_chased() {
    // No `q`/`url` parameter: the wrapper cannot be unwrapped, so the link is
    // rejected as engine-internal instead of being navigated.
    let dom = r#"<div class="g"><a href="https://www.google.com/url?sa=U&ved=abc"><h3>No target</h3></a></div>"#;
    let url = build_serp_url(FIRECRAWL_GOOGLE_SEARCH, "wrapper").expect("url");
    let parsed = parse_with(
        FIRECRAWL_GOOGLE_SEARCH,
        "wrapper",
        5,
        "rawHtml",
        &google_dom(dom),
        &url,
        200,
    );
    assert_eq!(parsed.outcome, SerpOutcome::ParserMismatch);
    assert!(parsed.items.is_empty());
    assert!(parsed.diagnostics.rejection("engine_internal") > 0);
}

#[test]
fn unwrap_depth_is_bounded() {
    assert_eq!(MAX_REDIRECT_DEPTH, 3);
    fn pct(s: &str) -> String {
        url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
    }
    let base = Url::parse("https://www.google.com/search?q=depth").expect("base");
    let mut wrapped = "https://final.example.org/page".to_string();
    for _ in 0..4 {
        wrapped = format!("https://www.google.com/url?q={}", pct(&wrapped));
    }
    let parsed = Url::parse(&wrapped).expect("wrapped url");
    let unwrapped = unwrap_redirect(&parsed, GOOGLE, &base);
    // Three unwraps land on the innermost remaining wrapper, which is still an
    // engine host; the wrapper is never navigated further.
    assert!(url_host(&unwrapped).contains("google"));
    assert_eq!(unwrapped.path(), "/url");
}

#[test]
fn private_internal_ad_and_unsafe_links_are_rejected_with_reasons() {
    let dom = r#"<div class="g"><a href="https://127.0.0.1:8080/admin"><h3>Loopback admin</h3></a></div><div class="g"><a href="https://10.0.0.7/router"><h3>Private router</h3></a></div><div class="g"><a href="https://192.168.1.24/nas"><h3>Private NAS</h3></a></div><div class="g"><a href="https://localhost:3000/dev"><h3>Localhost dev</h3></a></div><div class="g"><a href="https://internal.corp.local/api"><h3>Internal API</h3></a></div><div class="g"><a href="https://ads.doubleclick.net/page"><h3>Ad host</h3></a></div><div class="g"><a href="https://googleads.g.doubleclick.net/purchase"><h3>Ad subdomain</h3></a></div><div class="g"><a href="https://www.google.com/search?q=internal"><h3>Engine internal</h3></a></div><div class="g"><a href="https://news.google.com/rss/articles"><h3>Engine news</h3></a></div><div class="g"><a href="javascript:alert(1)"><h3>Unsafe scheme</h3></a></div><div class="g"><a href="https://user:hunter2@creds.example.org/private"><h3>Credentials</h3></a></div><div class="g"><a href="ftp://files.example.org/thing"><h3>FTP</h3></a></div>"#;
    let url = build_serp_url(FIRECRAWL_GOOGLE_SEARCH, "reject").expect("url");
    let parsed = parse_with(
        FIRECRAWL_GOOGLE_SEARCH,
        "reject",
        10,
        "rawHtml",
        &google_dom(dom),
        &url,
        200,
    );
    assert_eq!(parsed.outcome, SerpOutcome::ParserMismatch);
    assert!(parsed.items.is_empty());
    assert!(
        parsed.diagnostics.rejection("private_ip") >= 4,
        "{:?}",
        parsed.diagnostics.rejections
    );
    assert!(
        parsed.diagnostics.rejection("ad_host") >= 2,
        "{:?}",
        parsed.diagnostics.rejections
    );
    assert!(
        parsed.diagnostics.rejection("engine_internal") >= 2,
        "{:?}",
        parsed.diagnostics.rejections
    );
    assert!(
        parsed.diagnostics.rejection("unsafe_scheme") >= 1,
        "{:?}",
        parsed.diagnostics.rejections
    );
    assert!(
        parsed.diagnostics.rejection("credentials") >= 1,
        "{:?}",
        parsed.diagnostics.rejections
    );
    assert!(
        parsed.diagnostics.dom_candidates >= 11,
        "{:?}",
        parsed.diagnostics.rejections
    );
    // A page whose only candidates were rejected is not evidence of zero results.
    assert_ne!(parsed.outcome, SerpOutcome::VerifiedZero);
}

#[test]
fn malformed_hrefs_are_rejected_as_malformed() {
    let base = Url::parse("https://results.example.org/search?q=x").expect("base url");
    let mut diag = SerpDiagnostics::default();
    assert_eq!(
        normalize_destination("   ", "google", &base, &mut diag),
        None
    );
    assert_eq!(
        normalize_destination("http://", "google", &base, &mut diag),
        None
    );
    assert_eq!(
        normalize_destination("https://[broken", "google", &base, &mut diag),
        None
    );
    assert!(diag.rejection("malformed") >= 3);
}

#[test]
fn legacy_normalize_destination_url_wrapper_still_works() {
    assert_eq!(
        normalize_destination_url("https://legacy.example.org/page", "google").as_deref(),
        Some("https://legacy.example.org/page")
    );
    assert_eq!(
        normalize_destination_url("https://www.google.com/search?q=x", "google"),
        None
    );
    assert_eq!(
        normalize_destination_url("https://legacy.example.org/page?a=1", "yandex").as_deref(),
        Some("https://legacy.example.org/page?a=1")
    );
    assert_eq!(
        normalize_destination_url("https://legacy.example.org/page", "mojeek").as_deref(),
        Some("https://legacy.example.org/page")
    );
    // Unknown engines have no canonical SERP base to resolve against.
    assert_eq!(
        normalize_destination_url("https://legacy.example.org/page", "bing"),
        None
    );
}

// ---------------------------------------------------------------------------
// Unicode, duplicates, provenance, bounds, cache identity
// ---------------------------------------------------------------------------

#[test]
fn unicode_queries_and_urls_round_trip() {
    let query = "результаты поиска — 検索";
    let dom = r#"<div class="g"><a href="https://unicode.example.org/搜索结果?q=результаты"><h3>Заголовок результата</h3></a><div class="VwiC3b">Пояснение к результату с юникодом.</div></div>"#;
    let url = build_serp_url(FIRECRAWL_GOOGLE_SEARCH, query).expect("serp url");
    let parsed = parse_with(
        FIRECRAWL_GOOGLE_SEARCH,
        query,
        5,
        "rawHtml",
        &google_dom(dom),
        &url,
        200,
    );
    assert_eq!(parsed.outcome, SerpOutcome::Valid);
    assert_eq!(parsed.items[0].query, query);
    assert_eq!(parsed.items[0].title, "Заголовок результата");
    assert_eq!(
        parsed.items[0].snippet,
        "Пояснение к результату с юникодом."
    );
    assert!(parsed.items[0].destination.contains("unicode.example.org"));
    assert!(!parsed.items[0].destination.contains(' '));
}

#[test]
fn duplicate_destinations_are_rejected_and_counted() {
    let dom = r#"<div class="g"><a href="https://one.example.org/hop"><h3>First copy</h3></a></div><div class="g"><a href="https://one.example.org/hop#section"><h3>Second copy</h3></a></div><div class="g"><a href="https://two.example.org/hop"><h3>Unique</h3></a></div>"#;
    let parsed = parse_google(dom, 10);
    assert_eq!(parsed.outcome, SerpOutcome::Valid);
    assert_eq!(parsed.items.len(), 2);
    assert_eq!(parsed.diagnostics.rejection("duplicate"), 1);
    assert_eq!(parsed.items[0].destination, "https://one.example.org/hop");
    assert_eq!(parsed.items[1].destination, "https://two.example.org/hop");
    assert_eq!(parsed.items[0].rank, 1);
    assert_eq!(parsed.items[1].rank, 2);
}

#[test]
fn items_carry_parser_version_outcome_and_provenance() {
    let parsed = parse_google(
        &format!(
            "{GOOGLE_CARD}{}",
            GOOGLE_CARD.replace("alpha.example.org", "second.example.org")
        ),
        5,
    );
    assert_eq!(parsed.items.len(), 2);
    for (idx, item) in parsed.items.iter().enumerate() {
        assert_eq!(item.parser_version, "2.0");
        assert_eq!(item.outcome, "valid");
        assert_eq!(item.engine, "google");
        assert_eq!(item.rank, idx + 1);
        assert_eq!(item.url, item.destination);
        assert!(!item.fetched_time.is_empty());
    }
    let payload = serde_json::to_value(&parsed.items[0]).expect("serialize");
    assert_eq!(
        payload.get("parser_version").and_then(Value::as_str),
        Some("2.0")
    );
    assert_eq!(
        payload.get("title").and_then(Value::as_str),
        Some("Alpha heading")
    );
}

#[test]
fn a_single_retry_is_allowed_only_for_a_parser_mismatch_with_a_dom() {
    let mismatched = parse_google(
        r#"<div class="unknown"><p>Unrecognized layout.</p></div>"#,
        5,
    );
    assert!(retry_allowed(mismatched.outcome, &mismatched.diagnostics));
    assert_eq!(mismatched.diagnostics.parser_input, ParserInput::RawHtml);

    let challenge = parse_with(
        FIRECRAWL_GOOGLE_SEARCH,
        "blocked",
        5,
        "rawHtml",
        r#"<form action="/sorry/index"></form>"#,
        &build_serp_url(FIRECRAWL_GOOGLE_SEARCH, "blocked").expect("url"),
        200,
    );
    assert!(!retry_allowed(challenge.outcome, &challenge.diagnostics));

    let rate_limited = parse_with(
        FIRECRAWL_GOOGLE_SEARCH,
        "limited",
        5,
        "rawHtml",
        r#"<div id="topstuff"><p>Rate limit.</p></div>"#,
        &build_serp_url(FIRECRAWL_GOOGLE_SEARCH, "limited").expect("url"),
        200,
    );
    assert!(!retry_allowed(
        rate_limited.outcome,
        &rate_limited.diagnostics
    ));

    let valid = parse_google(GOOGLE_CARD, 5);
    assert!(!retry_allowed(valid.outcome, &valid.diagnostics));

    let links = json!({ "success": true, "data": { "links": ["https://a.example.org/"] } });
    let links =
        parse_serp_response(&request(FIRECRAWL_GOOGLE_SEARCH, "links", 5), &links).expect("parse");
    assert!(!retry_allowed(links.outcome, &links.diagnostics));
}

#[test]
fn cache_fragment_carries_both_versions_and_changes_with_query_and_limit() {
    let a = request(FIRECRAWL_GOOGLE_SEARCH, "rust ownership", 5);
    let b = request(FIRECRAWL_GOOGLE_SEARCH, "yandex   ownership", 5);
    let c = request(FIRECRAWL_GOOGLE_SEARCH, "rust ownership", 13);

    let fa = cache_fragment(&a);
    let fb = cache_fragment(&b);
    let fc = cache_fragment(&c);
    assert_eq!(fa, cache_fragment(&a));
    assert_ne!(fa, fb, "query must change identity");
    assert_ne!(fa, fc, "limit must change identity");
    assert_eq!(fb, cache_fragment(&b), "whitespace collapses to one space");
    assert!(fa.contains(PARSER_VERSION), "{fa}");
    assert!(fa.contains(FETCH_CONTRACT_VERSION), "{fa}");
    // Locale is part of the identity.
    let mut located = a.clone();
    located.serp_url = "https://www.google.com/search?q=rust+ownership&hl=fr".to_string();
    assert_ne!(fa, cache_fragment(&located));
}

#[test]
fn rejection_reasons_are_bounded() {
    let mut diag = SerpDiagnostics::default();
    for idx in 0..40 {
        diag.record_rejection(&format!("reason_{idx}"));
    }
    assert_eq!(diag.rejections.len(), MAX_REJECTION_REASONS);
    assert_eq!(diag.rejection("reason_0"), 1);
    assert_eq!(diag.rejection("reason_39"), 0);
    diag.record_rejection("  reason_0  ");
    assert_eq!(diag.rejection("reason_0"), 2);
    assert_eq!(diag.rejection("never_recorded"), 0);
    let empty = SerpDiagnostics::default();
    assert_eq!(empty.rejection("anything"), 0);
}
