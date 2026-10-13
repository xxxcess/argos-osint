//! Fixed-corpus comparison of the full-body and focused oversized paths
//! (spec §2/§7). Offline and deterministic: it measures request count,
//! request bytes, local packet cost (cold/warm p50/p95) and retention of the
//! gold conclusion / attribution / pronoun / alias / late-denial sentences.
//! Model quality (missed conclusions in generated output) needs a live
//! held-out run and is not claimed here.
use super::*;
use crate::intel_recon::brief_stages::{inferences_user, request_windows, REQUEST_WINDOW_BYTES};
use serde_json::json;
use std::time::Instant;

struct Story {
    name: &'static str,
    text: String,
    actors: Vec<String>,
    claims: serde_json::Value,
    /// (category, exact gold sentence)
    gold: Vec<(&'static str, String)>,
}

fn filler(seed: usize, paragraphs: usize) -> String {
    let topics = [
        "Regional weather stayed mild through the week across the coast",
        "Local markets reported steady foot traffic in the old town",
        "The museum extended its opening hours for the autumn season",
        "Commuter rail services ran to timetable on most routes",
        "A community choir rehearsed for the winter concert series",
    ];
    (0..paragraphs)
        .map(|p| {
            let t = topics[(seed + p) % topics.len()];
            format!(
                "{t} on day {p}. Observers noted nothing unusual in block {p}. Officials described conditions as routine for paragraph {p}."
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn corpus() -> Vec<Story> {
    let mut out = Vec::new();
    for (k, (name, actor, alias, predicate, object)) in [
        (
            "port-talks",
            "Ana Ruiz",
            "Ruiz",
            "signed",
            "the Gdansk ferry accord",
        ),
        (
            "ferry-halt",
            "Northwind Ferries",
            "Northwind",
            "suspended",
            "Baltic crossings",
        ),
        (
            "budget",
            "Jan Kowal",
            "Kowal",
            "approved",
            "a 12% transport budget",
        ),
        (
            "grid",
            "Baltic Grid",
            "the operator",
            "delayed",
            "the 400 MW interconnector",
        ),
        (
            "mayor",
            "Ewa Nowak",
            "Nowak",
            "announced",
            "a harbour inquiry",
        ),
        ("audit", "Lena Berg", "Berg", "published", "the 2025 audit"),
    ]
    .into_iter()
    .enumerate()
    {
        let conclusion = format!("{actor} {predicate} {object} on 12 October.");
        let attribution = format!("\"This changes the timetable,\" {actor} told reporters.");
        let pronoun = "They expect the change to take effect within weeks.".to_string();
        let alias_s = format!("{alias} has previously opposed similar measures.");
        let denial = format!("A ministry spokesperson later denied that {object} was final.");
        let text = [
            filler(k, 160),
            format!("{conclusion} {attribution} {pronoun}"),
            filler(k + 1, 160),
            alias_s.clone(),
            filler(k + 2, 160),
            denial.clone(),
        ]
        .join("\n\n");
        out.push(Story {
            name,
            text,
            actors: vec![actor.to_string(), alias.to_string()],
            claims: json!({"claims":[{"id":"c1","text":conclusion,"predicate":predicate,"object":object,"attributed_to":actor,"time":"12 October"}]}),
            gold: vec![
                ("conclusion", conclusion),
                ("attribution", attribution),
                ("pronoun", pronoun),
                ("alias", alias_s),
                ("late_denial", denial),
            ],
        });
    }
    out
}

fn bytes(
    windows: &[Vec<super::super::brief_stages::Sentence>],
    system: &str,
    claims: &serde_json::Value,
) -> usize {
    windows
        .iter()
        .map(|w| system.len() + inferences_user(claims, w, &json!([])).len())
        .sum()
}

fn pct(mut v: Vec<f64>, p: f64) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[((v.len() as f64 - 1.0) * p).round() as usize]
}

struct Row {
    name: &'static str,
    full_windows: usize,
    focused_windows: usize,
    full_bytes: usize,
    focused_bytes: usize,
    retained: Vec<(&'static str, bool)>,
    coverage: String,
}

fn run() -> (Vec<Row>, [f64; 4]) {
    let system = "x".repeat(4_000);
    let mut rows = Vec::new();
    let (mut cold, mut warm) = (Vec::new(), Vec::new());
    for story in corpus() {
        for rep in 0..30 {
            let t = Instant::now();
            let index = SentenceIndex::local(&story.text, "fixture");
            let q = claims_query(&story.actors, &story.claims);
            let _ = focused_sentences(&index, &story.text, &q);
            cold.push(t.elapsed().as_secs_f64() * 1e3);
            let t = Instant::now();
            let _ = focused_sentences(&index, &story.text, &q);
            warm.push(t.elapsed().as_secs_f64() * 1e3);
            if rep > 0 {
                continue;
            }
            let all = index.wire_sentences(&story.text);
            let full = request_windows(&all, &system, &story.claims, REQUEST_WINDOW_BYTES);
            let (sel, set) = focused_sentences(&index, &story.text, &q).expect("matches");
            let focused = request_windows(&sel, &system, &story.claims, REQUEST_WINDOW_BYTES);
            let texts: Vec<&str> = sel.iter().map(|s| s.text.trim()).collect();
            rows.push(Row {
                name: story.name,
                full_windows: full.len(),
                focused_windows: focused.len(),
                full_bytes: bytes(&full, &system, &story.claims),
                focused_bytes: bytes(&focused, &system, &story.claims),
                retained: story
                    .gold
                    .iter()
                    .map(|(c, g)| (*c, texts.iter().any(|t| t.contains(g.trim()))))
                    .collect(),
                coverage: set.coverage.clone(),
            });
        }
    }
    let stats = [
        pct(cold.clone(), 0.5),
        pct(cold, 0.95),
        pct(warm.clone(), 0.5),
        pct(warm, 0.95),
    ];
    (rows, stats)
}

#[test]
fn focused_path_on_fixed_corpus_reduces_requests_and_keeps_gold_context() {
    let (rows, _) = run();
    assert_eq!(rows.len(), 6);
    for r in &rows {
        assert!(r.full_windows > 1, "{}: corpus must be oversized", r.name);
        assert_eq!(r.focused_windows, 1, "{}", r.name);
        assert!(r.focused_bytes * 5 < r.full_bytes, "{}", r.name);
        for (cat, kept) in &r.retained {
            assert!(kept, "{}: lost {cat} ({})", r.name, r.coverage);
        }
    }
}

/// `cargo test -p argos-osint-core --lib focused_compare_report -- --ignored --nocapture`
#[test]
#[ignore]
fn focused_compare_report() {
    let (rows, [c50, c95, w50, w95]) = run();
    println!("| story | full req | focused req | full bytes | focused bytes | packet coverage | gold kept |");
    println!("| --- | --- | --- | --- | --- | --- | --- |");
    for r in &rows {
        let kept = r.retained.iter().filter(|(_, k)| *k).count();
        println!(
            "| {} | {} | {} | {} | {} | {} | {kept}/{} |",
            r.name,
            r.full_windows,
            r.focused_windows,
            r.full_bytes,
            r.focused_bytes,
            r.coverage,
            r.retained.len()
        );
    }
    println!("local packet build: cold p50 {c50:.2} ms p95 {c95:.2} ms · warm p50 {w50:.2} ms p95 {w95:.2} ms");
}
