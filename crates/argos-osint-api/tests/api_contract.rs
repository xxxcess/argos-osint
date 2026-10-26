#![cfg(feature = "embedded")]
use argos_osint_api::{loopback_url, router, ApiState};
use argos_osint_core::{
    storage::{atlas, open::Target, repo::Repository},
    store::AtlasArticleRow,
};
use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;

async fn call(
    app: &Router,
    method: &str,
    path: &str,
    origin: Option<&str>,
    body: Value,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("host", "127.0.0.1:8080");
    if let Some(origin) = origin {
        request = request.header("origin", origin);
    }
    if method == "POST" {
        request = request.header("content-type", "application/json");
    }
    let response = app
        .clone()
        .oneshot(
            request
                .body(if method == "POST" {
                    Body::from(body.to_string())
                } else {
                    Body::empty()
                })
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1_000_000).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
#[test]
fn rejects_remote_and_credentialed_endpoints() {
    for bad in [
        "http://0.0.0.0:8080",
        "http://example.com",
        "https://localhost:8080",
        "http://user:secret@localhost:8080",
        "http://localhost:8080/api",
        "http://localhost:8080?key=secret",
    ] {
        assert!(loopback_url(bad).is_err(), "{bad}");
    }
    assert!(loopback_url("http://127.0.0.1:8080").is_ok());
}
#[tokio::test]
async fn storage_api_contracts() {
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("ARGOS_HOME", dir.path().join("home"));
    let repo = Repository::connect(
        &Target::Embedded {
            path: dir.path().join("db"),
        },
        "argos",
        "api",
    )
    .await
    .unwrap();
    atlas::insert_run(repo.db(), "r1", "{}", "{}")
        .await
        .unwrap();
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    for i in 0..5 {
        atlas::upsert_article(
            repo.db(),
            &AtlasArticleRow {
                run_id: "r1".into(),
                id: format!("a{i}"),
                title: format!("Harbor signal {i}"),
                description: "Shipping news".into(),
                category: "geopolitical".into(),
                published_at: format!("{today}T12:00:00Z"),
                seen_at: format!("{today}T13:00:00Z"),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    }
    let body = argos_osint_core::storage::article_body::ensure(
        &repo,
        "a0",
        "r1",
        "https://example.test",
        "example.test",
        "Fixture Wire",
    )
    .await
    .unwrap();
    argos_osint_core::storage::article_body::commit(
        &repo,
        &body.id,
        "Full retained fixture body",
        "fixture-hash",
        "complete",
        "Offline fixture",
        "https://example.test",
        "fixture",
        false,
    )
    .await
    .unwrap();
    let app = router(ApiState {
        repo: repo.clone(),
        instance_id: "fixture-api".into(),
        port: 8080,
        origins: Arc::new(vec!["http://localhost:3000".into()]),
    });
    let (status, ready) = call(&app, "GET", "/api/v1/health/ready", None, Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ready["storage"], true);
    assert_eq!(ready["instance_id"], "fixture-api");
    let default_port = router(ApiState {
        repo: repo.clone(),
        instance_id: "port-80".into(),
        port: 80,
        origins: Arc::new(Vec::new()),
    });
    let response = default_port
        .oneshot(
            Request::builder()
                .uri("/api/v1/health/ready")
                .header("host", "localhost")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    assert_eq!(
        call(
            &app,
            "GET",
            "/api/v1/days",
            Some("https://evil.test"),
            Value::Null
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/health/ready")
                .header("host", "evil.test:8080")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("OPTIONS")
                .uri("/api/v1/brief/ensure")
                .header("host", "localhost:8080")
                .header("origin", "http://localhost:3000")
                .header("access-control-request-method", "POST")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        response.headers()["access-control-allow-origin"],
        "http://localhost:3000"
    );
    let query = format!("/api/v1/articles?category=geopolitical&day={today}&search=harbor&limit=2");
    let (status, page) = call(&app, "GET", &query, None, Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(page["rows"].as_array().unwrap().len(), 2);
    let cursor = page["next_cursor"].as_str().unwrap();
    let encoded = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("cursor", cursor)
        .finish();
    let (_, second) = call(
        &app,
        "GET",
        &format!("{query}&{encoded}"),
        None,
        Value::Null,
    )
    .await;
    assert_ne!(page["rows"][0]["id"], second["rows"][0]["id"]);
    let (_, again) = call(&app, "GET", &query, None, Value::Null).await;
    assert_eq!(again["revision"], page["revision"]);
    assert_eq!(
        call(
            &app,
            "GET",
            "/api/v1/articles?category=nope&day=bad&limit=999",
            None,
            Value::Null
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let (status, detail) = call(
        &app,
        "GET",
        "/api/v1/articles/detail?run=r1&id=a0",
        None,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert_eq!(detail["article"]["title"], "Harbor signal 0");
    assert_eq!(
        detail["body"]["body_markdown"],
        "Full retained fixture body"
    );
    assert_eq!(
        call(
            &app,
            "GET",
            "/api/v1/articles/detail?run=r1&id=absent",
            None,
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let payload = json!({"run":"r1","id":"a0"});
    let (status, first) = call(&app, "POST", "/api/v1/brief/ensure", None, payload.clone()).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    let (_, second) = call(&app, "POST", "/api/v1/brief/ensure", None, payload).await;
    assert_eq!(first["job_id"], second["job_id"]);
    argos_osint_core::storage::brief::put_index_state(
        &repo,
        "a0",
        &argos_osint_core::intel_recon::brief_canonical::IndexState {
            state: "failed".into(),
            job_id: first["job_id"].as_str().unwrap().into(),
            reason: "Fixture publication failed".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let (_, indexed) = call(
        &app,
        "GET",
        "/api/v1/articles/detail?run=r1&id=a0",
        None,
        Value::Null,
    )
    .await;
    assert_eq!(indexed["indexing"]["state"], "failed");
    let reload = json!({"run":"r1","id":"a0","request_id":"fixture-reload-1"});
    let (status, first_reload) =
        call(&app, "POST", "/api/v1/brief/reload", None, reload.clone()).await;
    assert_eq!(status, StatusCode::OK, "{first_reload}");
    let (_, second_reload) = call(&app, "POST", "/api/v1/brief/reload", None, reload).await;
    assert_eq!(first_reload["job_id"], second_reload["job_id"]);
    assert_ne!(first_reload["job_id"], first["job_id"]);
    let (_, reloading) = call(
        &app,
        "GET",
        "/api/v1/articles/detail?run=r1&id=a0",
        None,
        Value::Null,
    )
    .await;
    assert_eq!(reloading["body_pending"], true);
    assert!(reloading["body"].is_null());

    let (_, newer) = call(
        &app,
        "POST",
        "/api/v1/brief/reload",
        None,
        json!({"run":"r1","id":"a0","request_id":"fixture-reload-2"}),
    )
    .await;
    assert_ne!(newer["job_id"], first_reload["job_id"]);
    let (_, replay) = call(
        &app,
        "POST",
        "/api/v1/brief/reload",
        None,
        json!({"run":"r1","id":"a0","request_id":"fixture-reload-1"}),
    )
    .await;
    assert_eq!(replay["job_id"], first_reload["job_id"]);

    let (status,topology)=call(&app,"POST","/api/v1/topology",None,json!({"seed":{"Article":{"run_id":"r1","article_id":"a0","revision":detail["source_revision"],"originating_memory":null}}})).await;
    assert_eq!(status, StatusCode::OK, "{topology}");
    assert!(!topology["nodes"].as_array().unwrap().is_empty());
    assert!(topology["nodes"].as_array().unwrap().len() <= 100);
    // Readiness must stop succeeding when its schema/service contract disappears.
    repo.db()
        .query("DELETE storage_meta:schema")
        .await
        .unwrap()
        .check()
        .unwrap();
    assert_eq!(
        call(&app, "GET", "/api/v1/health/ready", None, Value::Null)
            .await
            .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
}
