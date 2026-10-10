//! Fixture-driven tests for the named-SERP outcome contract (spec §9).
//!
//! Every fixture pair under `tests/fixtures/search_engines/` is parsed with
//! `parse_serp_response` and checked against the outcome recorded in its sibling
//! `*.expected.json`. All DOM is invented and sanitized: no captured pages, no
//! real user data, no credentials.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use argos_osint_core::osint::search_engines::{
    parse_serp_response, SerpOutcome, SerpParse, SerpRequest, PARSER_VERSION,
};
use serde_json::{json, Value};

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/search_engines")
}

fn read_json(path: &Path) -> Value {
    let text =
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

fn read_text(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn field<'a>(value: &'a Value, key: &str) -> &'a Value {
    value
        .get(key)
        .unwrap_or_else(|| panic!("missing field `{key}` in {}", value))
}

fn str_field(value: &Value, key: &str) -> String {
    field(value, key)
        .as_str()
        .unwrap_or_else(|| panic!("`{key}` is not a string"))
        .to_string()
}

fn opt_str(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

fn opt_usize(value: &Value, key: &str) -> Option<usize> {
    value.get(key).and_then(Value::as_u64).map(|n| n as usize)
}

/// Every fixture stem with an expected file next to it.
fn fixture_cases() -> Vec<(String, PathBuf, PathBuf)> {
    let dir = fixtures_dir();
    let mut cases = Vec::new();
    let entries = std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display()));
    for entry in entries {
        let path = entry.expect("dir entry").path();
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if !stem.ends_with(".expected") {
            continue;
        }
        let name = stem.trim_end_matches(".expected").to_string();
        let envelope = dir.join(format!("{name}.json"));
        let dom = dir.join(format!("{name}.html"));
        let payload = if envelope.is_file() {
            envelope
        } else if dom.is_file() {
            dom
        } else {
            panic!("fixture `{name}` has no `.json` envelope or `.html` dom");
        };
        cases.push((name, payload, path));
    }
    cases.sort_by(|a, b| a.0.cmp(&b.0));
    assert!(
        !cases.is_empty(),
        "no fixtures found under {}",
        fixtures_dir().display()
    );
    cases
}

/// Builds the request and envelope for one fixture case, then parses it.
fn run_case(expected: &Value, payload: &Path) -> (SerpRequest, SerpParse) {
    let tool = str_field(expected, "tool");
    let query = str_field(expected, "query");
    let limit = opt_usize(expected, "limit").unwrap_or(5);
    let req = SerpRequest::for_tool(&tool, &query, limit)
        .unwrap_or_else(|e| panic!("request for {tool}: {e}"));

    let envelope = if payload.extension().and_then(|e| e.to_str()) == Some("html") {
        let dom = read_text(payload);
        let key = opt_str(expected, "dom_key").unwrap_or_else(|| "rawHtml".to_string());
        let status = opt_usize(expected, "status").unwrap_or(200) as u16;
        json!({
            "success": true,
            "data": {
                key: dom,
                "metadata": {
                    "sourceURL": req.serp_url,
                    "statusCode": status
                }
            }
        })
    } else {
        read_json(payload)
    };

    let parsed =
        parse_serp_response(&req, &envelope).expect("parse never fails on a parsable envelope");
    (req, parsed)
}

#[test]
fn every_fixture_matches_its_recorded_outcome() {
    for (name, payload, expected_path) in fixture_cases() {
        let expected = read_json(&expected_path);
        let (req, parsed) = run_case(&expected, &payload);

        let outcome = str_field(&expected, "outcome");
        let expected_outcome = SerpOutcome::parse(&outcome)
            .unwrap_or_else(|| panic!("fixture {name}: unknown outcome `{outcome}`"));
        assert_eq!(
            parsed.outcome,
            expected_outcome,
            "fixture {name}: expected {outcome}, got {} ({:?})",
            parsed.outcome.as_str(),
            parsed.diagnostics
        );

        if let Some(count) = opt_usize(&expected, "accepted") {
            assert_eq!(
                parsed.diagnostics.accepted, count,
                "fixture {name}: accepted count"
            );
        }
        if let Some(count) = opt_usize(&expected, "items") {
            assert_eq!(parsed.items.len(), count, "fixture {name}: item count");
        }
        if let Some(parser_input) = opt_str(&expected, "parser_input") {
            assert_eq!(
                parsed.diagnostics.parser_input.as_str(),
                parser_input,
                "fixture {name}: parser input"
            );
        }
        if let Some(status) = opt_usize(&expected, "target_status") {
            assert_eq!(
                parsed.diagnostics.target_status,
                Some(status as u16),
                "fixture {name}: target status"
            );
        }
        if let Some(dest) = opt_str(&expected, "first_destination") {
            let item = parsed
                .items
                .first()
                .unwrap_or_else(|| panic!("fixture {name}: no items to check destination"));
            assert_eq!(item.destination, dest, "fixture {name}: first destination");
        }
        if let Some(title) = opt_str(&expected, "first_title") {
            let item = parsed
                .items
                .first()
                .unwrap_or_else(|| panic!("fixture {name}: no items to check title"));
            assert_eq!(item.title, title, "fixture {name}: first title");
        }
        if let Some(query) = opt_str(&expected, "query") {
            assert!(!query.is_empty(), "fixture {name}: expected a query");
            if let Some(item) = parsed.items.first() {
                assert_eq!(item.query, query, "fixture {name}: query provenance");
            }
        }
        if let Some(rejections) = expected.get("rejections") {
            let table: BTreeMap<String, usize> = serde_json::from_value(rejections.clone())
                .unwrap_or_else(|e| panic!("fixture {name}: rejections: {e}"));
            for (reason, count) in table {
                assert_eq!(
                    parsed.diagnostics.rejection(&reason),
                    count,
                    "fixture {name}: rejection `{reason}`"
                );
            }
        }

        // Provenance invariants that must hold whenever an item is produced.
        if let Some(first) = parsed.items.first() {
            assert_eq!(
                first.parser_version.as_str(),
                PARSER_VERSION,
                "fixture {name}: parser version"
            );
        }
        assert!(
            parsed.outcome != SerpOutcome::VerifiedZero || parsed.items.is_empty(),
            "fixture {name}: a verified zero carries no items"
        );
        assert_eq!(
            parsed.diagnostics.requested_url, req.serp_url,
            "fixture {name}: requested url provenance"
        );
    }
}

#[test]
fn the_named_engine_fixture_set_is_complete() {
    let names: Vec<String> = fixture_cases().into_iter().map(|(n, _, _)| n).collect();
    for required in [
        "google_valid_raw_html",
        "yandex_valid_raw_html",
        "mojeek_valid_ten_results",
        "mojeek_valid_hundred_results",
        "google_cleaned_html_fallback",
        "google_unrecognized_dom",
        "google_missing_metadata",
        "google_target_status_403",
        "google_target_status_429",
        "google_target_status_500",
        "mojeek_target_status_404",
        "api_http_404",
        "envelope_success_false",
        "google_consent_redirect",
        "google_consent_noscript",
        "google_sorry_challenge",
        "google_verified_zero",
        "yandex_verified_zero",
        "mojeek_verified_zero",
        "mojeek_truncated_envelope",
        "mojeek_links_only",
        "google_wrapper_links",
        "google_rejected_links",
        "google_duplicate_links",
        "yandex_not_found_snippet",
        "unicode_query",
        "boundary_query_values",
    ] {
        assert!(
            names.iter().any(|n| n == required),
            "missing fixture {required}"
        );
    }
    assert!(names.len() >= 26, "fixture set too small: {names:?}");
}

#[test]
fn no_fixture_claimed_a_verified_zero_without_a_supported_phrase() {
    for (name, payload, expected_path) in fixture_cases() {
        let expected = read_json(&expected_path);
        let (_, parsed) = run_case(&expected, &payload);
        if parsed.outcome != SerpOutcome::VerifiedZero {
            continue;
        }
        assert!(
            parsed.items.is_empty(),
            "fixture {name}: verified zero with items"
        );
        assert!(
            !parsed.diagnostics.status_region.is_empty(),
            "fixture {name}: verified zero must name the status region"
        );
    }
}
