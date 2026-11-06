//! Summary generation is a strict protected action before storage/provider work.
#![cfg(feature = "embedded")]
use std::sync::Arc;

use argos_osint_api::{router, ApiState};
use argos_osint_core::storage::open::Target;
use argos_osint_core::storage::repo::Repository;
use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

#[tokio::test]
async fn summary_action_rejects_unknown_fields_and_untrusted_origin_before_dispatch() {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::connect(
        &Target::Embedded {
            path: dir.path().join("db"),
        },
        "argos",
        "summary_action",
    )
    .await
    .unwrap();
    argos_osint_core::storage::memory::write_then_index(
        repo.publisher(),
        Arc::new(|tx| {
            Box::pin(async move {
                argos_osint_core::storage::memory::put_typed_in(
                    &tx,
                    &argos_osint_core::storage::memory::TypedMemory {
                        id: "current-claim".into(),
                        text: "Retained current assertion".into(),
                        kind: argos_osint_core::memory_metadata::MemoryKind::Claim,
                        payload: Some(argos_osint_core::memory_metadata::MemoryPayload::Claim(
                            Default::default(),
                        )),
                        lifecycle: "active".into(),
                    },
                )
                .await?;
                Ok(vec!["current-claim".into()])
            })
        }),
    )
    .await
    .unwrap();
    let app = router(ApiState {
        repo: repo.clone(),
        instance_id: "summary-fixture".into(),
        port: 8080,
        origins: Arc::new(vec!["http://localhost:3000".into()]),
    });
    for (method, path) in [
        ("GET", "classifier/status"),
        ("POST", "classifier/train"),
        ("GET", "classifier/jobs/detail?id=missing"),
        ("POST", "classifier/cancel"),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(format!("/api/v1/{path}"))
                    .header("host", "localhost:8080")
                    .header("origin", "http://localhost:3000")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::NOT_FOUND,
            "classifier setup belongs to Venture: {path}"
        );
    }
    for (origin, body, expected) in [
        (
            "http://localhost:3000",
            r#"{"seed":"current-claim","basis":"superseded-basis"}"#,
            StatusCode::CONFLICT,
        ),
        (
            "http://localhost:3000",
            r#"{"seed":"missing","basis":"captured-basis","generate":true}"#,
            StatusCode::BAD_REQUEST,
        ),
        (
            "https://evil.invalid",
            r#"{"seed":"missing","basis":"captured-basis"}"#,
            StatusCode::FORBIDDEN,
        ),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/topology/summary/generate")
                    .header("host", "localhost:8080")
                    .header("origin", origin)
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
        let body = to_bytes(response.into_body(), 1024).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(body["error"].is_string());
    }
    let mut response = repo
        .db()
        .query("SELECT VALUE count() FROM task WHERE pool='summary' GROUP ALL")
        .await
        .unwrap()
        .check()
        .unwrap();
    let count: Option<i64> = response.take(0).unwrap();
    assert_eq!(count.unwrap_or(0), 0, "rejected actions never enqueue");
}
