use std::collections::BTreeSet;
use std::time::Duration;
use std::time::Instant;

use axum::body::Body;
use axum::http::Request;
use axum::http::StatusCode;
use fleetworks_contracts::actor_index::MAX_BATCH_BYTES;
use fleetworks_search_index::config::parse_hosts;
use fleetworks_search_index::config::Config;
use fleetworks_search_index::engine::EngineConfig;
use fleetworks_search_index::http;
use fleetworks_search_index::http::Pace;
use sha2::Digest;
use tower::ServiceExt;

fn config() -> Config {
    Config {
        bind: "127.0.0.1:0".parse().expect("loopback"),
        allowed_hosts: BTreeSet::from(["search.test".into()]),
        read_digest: sha2::Sha256::digest(b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").into(),
        write_digest: sha2::Sha256::digest(b"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").into(),
        query_capacity: 4,
        limits: Default::default(),
        engine: EngineConfig {
            directory: "/unopened-fixture-volume".into(),
            deployment_id: "synthetic-deployment".into(),
            source_epoch: "synthetic-source".into(),
            writer_memory_bytes: 64 * 1024 * 1024,
            writer_threads: 1,
            merge_threads: 1,
        },
    }
}

#[tokio::test]
async fn host_origin_and_purpose_credentials_are_checked_before_private_dispatch() {
    let router =
        http::router(config(), None, "synthetic_index_unavailable").expect("valid fixture config");
    let cases = [
        (
            "other.test",
            None,
            false,
            "/internal/v1/actors/status",
            StatusCode::BAD_REQUEST,
        ),
        (
            "search.test",
            None,
            false,
            "/internal/v1/actors/status",
            StatusCode::UNAUTHORIZED,
        ),
        (
            "search.test",
            Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            true,
            "/internal/v1/actors/status",
            StatusCode::FORBIDDEN,
        ),
        (
            "search.test",
            Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            false,
            "/internal/v1/actors/batches",
            StatusCode::FORBIDDEN,
        ),
        (
            "search.test",
            Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
            false,
            "/internal/v1/actors/status",
            StatusCode::OK,
        ),
    ];
    for (host, token, origin, path, expected) in cases {
        let mut builder = Request::builder()
            .method(if path.ends_with("batches") {
                "POST"
            } else {
                "GET"
            })
            .uri(path)
            .header("Host", host);
        if let Some(token) = token {
            builder = builder.header("Authorization", format!("Bearer {token}"));
        }
        if origin {
            builder = builder.header("Origin", "https://browser.example.test");
        }
        let response = router
            .clone()
            .oneshot(builder.body(Body::from("{}")).expect("synthetic request"))
            .await
            .expect("response");
        assert_eq!(response.status(), expected, "host/purpose case");
    }
    let request = Request::builder()
        .method("POST")
        .uri("/internal/v1/actors/batches")
        .header("Host", "search.test")
        .header("Authorization", "Bearer bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
        .body(Body::from(vec![b'x'; MAX_BATCH_BYTES + 1]))
        .expect("oversized body");
    assert_eq!(
        router
            .oneshot(request)
            .await
            .expect("bounded response")
            .status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );
}

#[test]
fn controlled_clock_pacing_has_no_catchup_burst_and_host_allowlist_has_no_wildcards() {
    let start = Instant::now();
    let mut pace = Pace::default();
    assert!(pace.admit(start).is_ok());
    assert_eq!(
        pace.admit(start + Duration::from_millis(499)),
        Err(Duration::from_millis(1))
    );
    assert!(pace.admit(start + Duration::from_millis(500)).is_ok());
    assert!(pace.admit(start + Duration::from_millis(500)).is_err());
    assert!(pace.admit(start + Duration::from_secs(5)).is_ok());
    assert!(pace.admit(start + Duration::from_secs(5)).is_err());
    assert!(parse_hosts("search-index:8087,[::1]:8087").is_ok());
    for value in ["*", "user@host", "https://host", "host/path", ""] {
        assert!(parse_hosts(value).is_err());
    }
}
