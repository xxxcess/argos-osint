//! Profile read model on SurrealDB (`storage::profile_view`): the same rows
//! in SQLite and SurrealDB give the same snapshot for every period/filter.
//! Run through `python3 scripts/test_surreal.py --mode both`.
#![cfg(feature = "surreal")]

mod support;

use argos_osint_core::profile_stats::{Period, StatFilters};
use argos_osint_core::storage::profile_view as view;
use chrono::{DateTime, Duration, Utc};
use serde_json::{json, Map, Value};
use support::*;

/// One telemetry event as (event_type, minutes ago, dims, metric_ms, payload).
struct Ev {
    kind: &'static str,
    ago_min: i64,
    dims: Value,
    metric_ms: Option<i64>,
    payload: Value,
}

fn ev(kind: &'static str, ago_min: i64, dims: Value, metric_ms: Option<i64>, payload: Value) -> Ev {
    Ev {
        kind,
        ago_min,
        dims,
        metric_ms,
        payload,
    }
}

const DIMS: [&str; 18] = [
    "app",
    "trigger",
    "tool_id",
    "category",
    "provider",
    "role",
    "model",
    "mode",
    "engine",
    "outcome",
    "reason",
    "job_id",
    "run_id",
    "article_id",
    "origin",
    "call_id",
    "canonical_ref",
    "thread_id",
];

fn events() -> Vec<Ev> {
    let mut out = Vec::new();
    for (i, (article, tag, domain)) in [
        ("a1", "politics", "wire.example"),
        ("a2", "economy", "daily.example"),
        ("a3", "politics", "wire.example"),
    ]
    .into_iter()
    .enumerate()
    {
        out.push(ev(
            "intel_article",
            30 + i as i64,
            json!({"app": "intel", "category": tag, "provider": domain, "article_id": article, "origin": "us"}),
            None,
            json!({"initial_confidence": 0.4 + i as f64 / 10.0, "published_at": "2026-10-20T00:00:00Z"}),
        ));
    }
    out.push(ev(
        "intel_report",
        20,
        json!({"app": "intel", "article_id": "a1", "mode": "focused", "reason": "completed"}),
        Some(4_000),
        json!({"wall_ms": 4000, "active_ms": 3000}),
    ));
    out.push(ev(
        "recon_run",
        15,
        json!({"app": "recon", "mode": "live", "run_id": "r1", "outcome": "completed"}),
        Some(60_000),
        json!({"directives": 2, "calls": 5, "categories": 2, "accepted_memories": 1}),
    ));
    out.push(ev(
        "recon_stage",
        15,
        json!({"app": "recon", "mode": "live", "run_id": "r1", "tool_id": "plan"}),
        Some(2_000),
        json!({"wait_ms": 500}),
    ));
    out.push(ev(
        "directive_assessed",
        14,
        json!({"app": "recon", "mode": "live", "run_id": "r1", "outcome": "unresolved", "reason": "no_evidence"}),
        None,
        json!({"label": "d1", "evidence_count": 0, "next_action": "widen"}),
    ));
    out.push(ev(
        "recall_query",
        14,
        json!({"app": "recon", "mode": "live", "run_id": "r1"}),
        None,
        json!({"candidates": 4, "accepted": 1, "rejected": 3}),
    ));
    out.push(ev(
        "atlas_cycle",
        12,
        json!({"app": "atlas", "run_id": "atlas-1", "outcome": "completed"}),
        Some(9_000),
        json!({"queue_ms": 100}),
    ));
    out.push(ev(
        "atlas_stage",
        11,
        json!({"app": "atlas", "run_id": "atlas-1", "tool_id": "classification", "outcome": "blocked", "reason": "quota", "call_id": "unit-1"}),
        None,
        json!({"unit": "article", "error_category": "quota"}),
    ));
    out.push(ev(
        "atlas_origin_snapshot",
        11,
        json!({"app": "atlas", "run_id": "atlas-1", "origin": "us"}),
        None,
        json!({"temperature": 0.7, "tier": "hot", "articles": 2, "eligible": true, "version": 1}),
    ));
    for i in 0..24 {
        let outcome = if i % 6 == 0 { "error" } else { "ok" };
        out.push(ev(
            "tool_invocation",
            40 - i,
            json!({"app": "recon", "tool_id": if i % 2 == 0 { "newsapi" } else { "courtlistener" },
                   "category": "news", "mode": "remote", "trigger": "model", "outcome": outcome,
                   "run_id": "r1", "call_id": format!("c{i}")}),
            Some(100 + i * 37),
            json!({}),
        ));
        out.push(ev(
            "model_attempt",
            40 - i,
            json!({"app": "recon", "provider": "openrouter", "role": "recon", "model": "m1",
                   "outcome": if i % 8 == 0 { "rate_limited" } else { "ok" }}),
            Some(800 + i * 50),
            json!({"queue_ms": 20 + i, "ttfb_ms": 200 + i, "first_content_ms": 300 + i}),
        ));
    }
    out.push(ev(
        "tool_wire_request",
        10,
        json!({"app": "recon", "tool_id": "newsapi", "category": "news", "trigger": "model"}),
        None,
        json!({}),
    ));
    out.push(ev(
        "tool_engine_query",
        9,
        json!({"app": "recon", "engine": "google", "tool_id": "search", "outcome": "challenge", "run_id": "r1"}),
        None,
        json!({"parser_version": "3"}),
    ));
    out.push(ev(
        "evidence_item",
        9,
        json!({"app": "recon", "tool_id": "newsapi", "category": "news", "call_id": "c0", "article_id": "a1", "canonical_ref": "fp-1"}),
        None,
        json!({"cited": true}),
    ));
    out.push(ev(
        "model_operation",
        8,
        json!({"app": "recon", "provider": "openrouter", "role": "recon", "model": "m1", "outcome": "ok"}),
        Some(1_500),
        json!({"attempts": 2, "terminal": true, "recovered": true}),
    ));
    out
}

fn stamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339()
}

/// The stored telemetry rows as flat JSON objects (no nulls).
fn rows(now: DateTime<Utc>) -> Vec<(String, Map<String, Value>)> {
    events()
        .into_iter()
        .enumerate()
        .map(|(i, e)| {
            let mut row = Map::new();
            row.insert("event_type".into(), e.kind.into());
            row.insert(
                "occurred_at".into(),
                stamp(now - Duration::minutes(e.ago_min)).into(),
            );
            for key in DIMS {
                row.insert(
                    key.into(),
                    e.dims.get(key).cloned().unwrap_or_else(|| "".into()),
                );
            }
            if let Some(ms) = e.metric_ms {
                row.insert("metric_ms".into(), ms.into());
            }
            row.insert("count".into(), 1.into());
            row.insert("payload_json".into(), e.payload.to_string().into());
            (format!("ev-{i:03}"), row)
        })
        .collect()
}

async fn seed_surreal(
    db: &surrealdb::Surreal<surrealdb::engine::any::Any>,
    now: DateTime<Utc>,
    old_bucket: &str,
    dims: &str,
) {
    let mut sql = String::new();
    for (id, row) in rows(now) {
        sql.push_str(&format!(
            "CREATE type::record('telemetry_event', '{id}') CONTENT {} RETURN NONE;\n",
            Value::Object(row)
        ));
    }
    let t = stamp(now);
    let rollup = json!({"bucket": old_bucket, "event_type": "tool_invocation", "dim_key": dims,
        "events": 17, "total_ms": 3400, "max_ms": 900, "bins_json": "[0,0,5,12]"});
    sql.push_str(&format!(
        "CREATE telemetry_hourly CONTENT {rollup} RETURN NONE;\n"
    ));
    let capacity = json!({"captured_at": stamp(now - Duration::minutes(1)), "provider": "openrouter",
        "quota_group": "openrouter", "scope": "*", "sends_60s": 12, "rpm_limit": 60,
        "pace_per_min": 12.0, "active": 2, "max_concurrency": 4, "queued": 3,
        "oldest_wait_ms": 320, "cooldown_ms": 1500, "quota_source": "provider_docs"});
    sql.push_str(&format!(
        "CREATE telemetry_provider_capacity CONTENT {capacity} RETURN NONE;\n"
    ));
    for (id, article, body, quality) in [
        ("b1", "a1", "Body", "partial"),
        ("b2", "a2", "  ", "partial"),
        ("b3", "a3", "Body", "unavailable"),
    ] {
        let row = json!({"article_id": article, "run_id": "", "original_url": "https://e",
            "resolved_url": "", "source_domain": "", "source_name": "", "body_markdown": body,
            "content_hash": "", "quality": quality, "quality_rationale": "", "body_version": 1,
            "fetch_tool": "", "fetched_at": t, "retry_after": "", "state": "", "created_at": t,
            "updated_at": t});
        sql.push_str(&format!(
            "CREATE type::record('article_body', '{id}') CONTENT {row} RETURN NONE;\n"
        ));
    }
    for (id, state, stats) in [
        (
            "atlas-1",
            "running",
            "{\"contract_version\":\"atlas-collection-v2\"}",
        ),
        ("atlas-0", "completed", "{}"),
    ] {
        let row = json!({"state": state, "stats_json": stats, "started_at": t});
        sql.push_str(&format!(
            "CREATE type::record('atlas_run', '{id}') CONTENT {row} RETURN NONE;\n"
        ));
    }
    for (run, article, country) in [
        ("atlas-1", "a1", "us"),
        ("atlas-1", "a2", "fr"),
        ("atlas-0", "a1", "us"),
    ] {
        let row = json!({"run_id": run, "article_id": article, "title": "T", "description": "",
            "url": "https://e", "country": country, "source_name": "W", "source_domain": "e",
            "published_at": "", "provider": "fixture", "temperature": 0.0, "category": "unk",
            "seen_at": t});
        sql.push_str(&format!(
            "CREATE type::record('atlas_article', ['{run}', '{article}']) CONTENT {row} RETURN NONE;\n"
        ));
    }
    let meta = json!({"run_id": "atlas-1", "article_id": "a1", "input_hash": "h",
        "entity_state": "blocked", "reliability_state": "completed",
        "classification_status": "running", "metadata_json": "{}"});
    sql.push_str(&format!(
        "CREATE type::record('atlas_article_metadata', ['atlas-1', 'a1']) CONTENT {meta} RETURN NONE;\n"
    ));
    db.query(sql).await.unwrap().check().unwrap();
}

#[tokio::test]
async fn profile_views_cover_every_period_and_filter() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = server.root("argos", "profile").await;
    let now = Utc::now();
    let old = now - Duration::days(100);
    let old_bucket = old.format("%Y-%m-%dT%H").to_string();
    let dims = [
        "recon", "model", "newsapi", "news", "", "", "", "remote", "", "ok", "",
    ]
    .join("\u{1f}");

    // An empty store: everything unavailable.
    let empty_surreal = view::snapshot(&db, &Period::H24, &StatFilters::default(), now)
        .await
        .unwrap();
    assert_eq!(empty_surreal.status.collection_health, "unknown");

    seed_surreal(&db, now, &old_bucket, &dims).await;

    let custom = Period::Custom {
        from: (old - Duration::hours(2)).to_rfc3339(),
        to: now.to_rfc3339(),
    };
    let filtered = |f: fn(&mut StatFilters)| {
        let mut filters = StatFilters::default();
        f(&mut filters);
        filters
    };
    let cases = [
        (Period::H1, StatFilters::default()),
        (Period::H24, StatFilters::default()),
        (Period::H7, StatFilters::default()),
        (Period::H30, StatFilters::default()),
        (custom.clone(), StatFilters::default()),
        (Period::H24, filtered(|f| f.provider = "openrouter".into())),
        (Period::H24, filtered(|f| f.tool = "newsapi".into())),
        (Period::H24, filtered(|f| f.mode = "live".into())),
        (custom, filtered(|f| f.category = "news".into())),
    ];
    for (period, filters) in &cases {
        view::snapshot(&db, period, filters, now).await.unwrap();
    }
    // The TUI's one-load dashboard read matches both separate reads.
    for (period, filters) in &cases {
        let (snapshot, options) = view::dashboard(&db, period, filters, now).await.unwrap();
        assert_eq!(
            snapshot,
            view::snapshot(&db, period, filters, now).await.unwrap(),
            "dashboard {period:?} {filters:?}"
        );
        assert_eq!(options, view::filter_options(&db).await.unwrap());
    }

    // The views are populated, not just equally empty.
    let snap = view::snapshot(&db, &Period::H24, &StatFilters::default(), now)
        .await
        .unwrap();
    assert_eq!(snap.status.active_jobs, 1, "atlas-1 is running");
    assert!(snap.status.provider_capacity_available);
    assert_eq!(snap.status.queued_requests, 3);
    assert_eq!(snap.intel.distinct_new_articles, Some(3));
    assert_eq!(snap.intel.body_available_articles, Some(1));
    assert!(!snap.recon.outcomes.is_empty());
    assert!(!snap.recon.unresolved.is_empty());
    assert!(snap
        .atlas
        .backlog
        .iter()
        .any(|row| row.stage == "entities" && row.blocked == 1));
    assert!(!snap.models.capacity.is_empty());
    assert!(snap.models.latency.iter().any(|b| b.n > 0));
    assert!(!snap.tools.reliability.is_empty());
    assert!(!snap.tools.evidence.is_empty());
    for app in ["Recon", "Atlas", "Tools", "Models"] {
        assert!(
            snap.attention.iter().any(|row| row.app == app),
            "attention lists {app}"
        );
    }
    assert!(!snap.retention.oldest_rollup.is_empty());
}
