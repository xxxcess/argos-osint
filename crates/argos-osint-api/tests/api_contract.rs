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
    repo.db().query("CREATE memory:routeactor SET text = 'Route actor', memory_kind = 'actor'; CREATE brain_actor:routeactor SET display_name = 'Route actor', created_at = '', updated_at = ''; CREATE memory:routeclaim SET text = 'Route claim', memory_kind = 'claim'; CREATE memory:routeinference SET text = 'Route inference', memory_kind = 'inference';").await.unwrap().check().unwrap();
    actor_dossier_contracts(&app, &repo).await;
    for (id, kind) in [
        ("routeactor", "entity"),
        ("routeclaim", "claim"),
        ("routeinference", "inference"),
    ] {
        let (status, graph) = call(
            &app,
            "GET",
            &format!("/api/v1/topology?seed={id}"),
            None,
            Value::Null,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{graph}");
        assert_eq!(graph["root_id"], id);
        assert_eq!(graph["root_kind"], kind);
    }
    let (status, resolved) = call(
        &app,
        "POST",
        "/api/v1/topology/resolve",
        None,
        json!({"seed":{"Inference":"routeinference"}}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(resolved, json!({"seed":"routeinference"}));
    assert_eq!(
        call(
            &app,
            "GET",
            "/api/v1/topology?seed=missing",
            None,
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(
            &app,
            "GET",
            "/api/v1/topology?seed=routeinference&kind=claim",
            None,
            Value::Null
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/v1/topology/resolve",
            None,
            json!({"seed":{"Topic":"No fabricated memory"}})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let (status, metrics) = call(&app, "GET", "/api/v1/host/metrics", None, Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(metrics["cpu_percent"], Value::Null, "CPU needs two samples");
    for field in ["platform", "chipset"] {
        let value = metrics.get(field).expect("Host identity field is present");
        assert!(
            value.is_null()
                || value.as_str().is_some_and(|label| {
                    !label.is_empty()
                        && label.chars().count() <= 64
                        && !label.chars().any(char::is_control)
                }),
            "Invalid host {field}: {value}"
        );
    }
    assert_eq!(
        metrics["network_kbps"],
        Value::Null,
        "Network needs a delta window"
    );
    for field in ["memory_percent", "disk_percent"] {
        assert!(metrics.get(field).is_some(), "Missing {field}");
        if let Some(value) = metrics[field].as_f64() {
            assert!((0.0..=100.0).contains(&value), "Invalid {field}: {value}");
        }
    }
    let (status, _) = call(
        &app,
        "GET",
        "/api/v1/host/metrics",
        Some("http://untrusted.test"),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
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
    for order in ["latest", "hottest"] {
        let (status, _) = call(
            &app,
            "GET",
            &format!("/api/v1/articles?category=geopolitical&day={today}&sort={order}"),
            None,
            Value::Null,
        )
        .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "sort={order} must reach the current query contract"
        );
    }
    let (status, _) = call(
        &app,
        "GET",
        &format!("/api/v1/articles?category=geopolitical&day={today}&sort=unknown"),
        None,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
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

// Each contract owns its datastore; no process-wide ARGOS_HOME mutation.
async fn isolated_api() -> (tempfile::TempDir, Router) {
    let dir = tempfile::tempdir().expect("isolated API directory");
    let repo = Repository::connect(
        &Target::Embedded {
            path: dir.path().join("db"),
        },
        "argos",
        "contracts",
    )
    .await
    .expect("embedded contract repository");
    let app = router(ApiState {
        repo,
        instance_id: "contract-instance".into(),
        port: 8080,
        origins: Arc::new(vec!["http://localhost:3000".into()]),
    });
    (dir, app)
}

#[tokio::test]
async fn capabilities_advertise_only_supported_features() {
    let (_dir, app) = isolated_api().await;
    let (status, body) = call(&app, "GET", "/api/v1/capabilities", None, Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["api_version"], 1);
    assert_eq!(body["auth"], false);
    assert_eq!(
        body["features"],
        json!(["bulletin", "brief", "related", "jobs", "topology", "live"])
    );
    assert_eq!(
        body["live"],
        json!({"path":"/api/v1/live","channels":["metrics","brief"],"version":1})
    );
}

#[tokio::test]
async fn missing_job_returns_public_error_without_internal_details() {
    let (_dir, app) = isolated_api().await;
    let (status, body) = call(
        &app,
        "GET",
        "/api/v1/jobs/detail?id=missing-job",
        None,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body, json!({"error": "Resource not found"}));
}

#[tokio::test]
async fn fresh_repository_has_verified_empty_jobs() {
    let (_dir, app) = isolated_api().await;
    let (status, body) = call(&app, "GET", "/api/v1/jobs", None, Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({"rows": []}));
}

#[tokio::test]
async fn retrying_missing_job_is_idempotent() {
    let (_dir, app) = isolated_api().await;
    for _ in 0..2 {
        let (status, body) = call(
            &app,
            "POST",
            "/api/v1/jobs/retry",
            Some("http://localhost:3000"),
            json!({"id": "missing-job"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, json!({"requeued": 0}));
    }
    let (_, body) = call(&app, "GET", "/api/v1/jobs", None, Value::Null).await;
    assert_eq!(body, json!({"rows": []}));
}

#[tokio::test]
async fn untrusted_origin_cannot_retry_jobs() {
    let (_dir, app) = isolated_api().await;
    let (status, body) = call(
        &app,
        "POST",
        "/api/v1/jobs/retry",
        Some("https://evil.test"),
        json!({"id": "missing-job"}),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(body["error"].is_string());
    let (_, body) = call(&app, "GET", "/api/v1/jobs", None, Value::Null).await;
    assert_eq!(body, json!({"rows": []}));
}

// The caller owns the temporary embedded repository; these reads must be inert.
async fn actor_dossier_contracts(app: &Router, repo: &Repository) {
    for (id, expected) in [
        ("", StatusCode::BAD_REQUEST),
        ("missing", StatusCode::NOT_FOUND),
        ("routeclaim", StatusCode::NOT_FOUND),
    ] {
        let (status, _) = call(
            app,
            "GET",
            &format!("/api/v1/actors/detail?id={id}"),
            None,
            Value::Null,
        )
        .await;
        assert_eq!(status, expected);
    }
    let (status, _) = call(
        app,
        "GET",
        &format!("/api/v1/actors/detail?id={}", "x".repeat(257)),
        None,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    repo.db().query("CREATE memory:otheractor SET text = 'Route actor', memory_kind = 'actor'; CREATE brain_actor:otheractor SET display_name = 'Route actor', created_at = '', updated_at = ''; CREATE brain_actor_redirect:oldactor SET to_actor = 'routeactor', created_at = ''; UPDATE memory:routeactor SET payload_json = $payload; UPDATE brain_actor:routeactor SET catalog_id = 'fixture:1', catalog_json = $catalog;")
        .bind(("payload", json!({"kind":"actor","payload":{"subtype":"organization","display_name":"Route actor","aliases":["Legacy alias"],"identity_state":"provisional"}}).to_string()))
        .bind(("catalog", json!({"id":"routeactor","source_id":"fixture:1","name":"Route actor","subtype":"organization","aliases":["Catalog alias"],"metadata":{"actor_type":"government","description":"Attributed description","country_of_origin":"[\"Example\"]","countries_operated_in":"broken"}}).to_string()))
        .await.unwrap().check().unwrap();
    let counts = || async {
        let mut r = repo
            .db()
            .query("RETURN count(SELECT id FROM memory); RETURN count(SELECT id FROM job);")
            .await
            .unwrap()
            .check()
            .unwrap();
        (
            r.take::<Option<i64>>(0).unwrap(),
            r.take::<Option<i64>>(1).unwrap(),
        )
    };
    let before = counts().await;
    let (status, dossier) = call(
        app,
        "GET",
        "/api/v1/actors/detail?id=oldactor",
        None,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{dossier}");
    assert_eq!(dossier["id"], "routeactor");
    assert_eq!(dossier["catalog_type"], "government");
    assert_eq!(dossier["description"], "Attributed description");
    assert!(dossier["aliases"]
        .as_array()
        .unwrap()
        .contains(&json!("Legacy alias")));
    assert_eq!(dossier["loaded_claims"], 0);
    assert_eq!(dossier["loaded_briefs"], 0);
    assert!(!dossier["truncated"].as_bool().unwrap());
    let (_, other) = call(
        app,
        "GET",
        "/api/v1/actors/detail?id=otheractor",
        None,
        Value::Null,
    )
    .await;
    assert_eq!(other["id"], "otheractor");
    assert!(other["description"].is_null());
    assert!(other["catalog_source_id"].is_null());
    // Sharing an article or a display name never supplies a claim reference.
    repo.db().query("CREATE brain_memory_actor_ref:recorded SET memory_id = 'routeclaim', actor_id = 'routeactor', role = 'subject', binding_state = 'confirmed', provenance_json = 'offline receipt'; CREATE brain_memory_actor_ref:inferred SET memory_id = 'routeinference', actor_id = 'routeactor', role = 'mentioned'; CREATE brain_actor_mention:current SET actor_id = 'routeactor', origin_kind = 'article', origin_scope = 'r1', origin_id = 'a0', origin_revision = $revision, created_at = ''; CREATE brain_actor_mention:stale SET actor_id = 'routeactor', origin_kind = 'article', origin_scope = 'r1', origin_id = 'a1', origin_revision = 'stale', created_at = '';")
        .bind(("revision", argos_osint_core::atlas_metadata::input_hash(&atlas::article(repo.db(), "r1", "a0").await.unwrap().unwrap())))
        .await.unwrap().check().unwrap();
    let (_, associated) = call(
        app,
        "GET",
        "/api/v1/actors/detail?id=routeactor",
        None,
        Value::Null,
    )
    .await;
    assert_eq!(associated["loaded_claims"], 1, "{associated}");
    assert_eq!(associated["claims"][0]["node"]["id"], "routeclaim");
    assert_eq!(
        associated["claims"][0]["relationship"]["actor_reference"]["role"],
        "subject"
    );
    assert_eq!(
        associated["claims"][0]["relationship"]["actor_reference"]["binding_state"],
        "confirmed"
    );
    assert_eq!(associated["loaded_inferences"], 1);
    assert_eq!(associated["inferences"][0]["node"]["id"], "routeinference");
    assert_eq!(
        associated["loaded_claims"], 1,
        "inferences remain separate from claims"
    );
    assert_eq!(associated["loaded_briefs"], 1);
    assert_eq!(associated["briefs"][0]["article"]["run_id"], "r1");
    assert_eq!(associated["briefs"][0]["article"]["id"], "a0");
    assert!(
        !associated["gaps"].as_array().unwrap().is_empty(),
        "stale revision must remain visible as a gap"
    );
    let (_, isolated) = call(
        app,
        "GET",
        "/api/v1/actors/detail?id=otheractor",
        None,
        Value::Null,
    )
    .await;
    assert_eq!(isolated["loaded_claims"], 0);
    // Exact read side-effect invariant before adding the deliberate bound fixture.
    assert_eq!(
        before,
        counts().await,
        "dossier reads must not create memories or jobs"
    );
    for i in 0..105 {
        repo.db().query("CREATE type::record('memory', $id) SET text = $id, memory_kind = 'claim'; CREATE type::record('brain_memory_actor_ref', $id) SET memory_id = $id, actor_id = 'routeactor', role = 'subject';")
            .bind(("id", format!("bounded-{i}"))).await.unwrap().check().unwrap();
    }
    let (_, bounded) = call(
        app,
        "GET",
        "/api/v1/actors/detail?id=routeactor",
        None,
        Value::Null,
    )
    .await;
    assert!(bounded["loaded_claims"].as_u64().unwrap() <= 99);
    assert_eq!(bounded["truncated"], true);
    repo.db()
        .query("UPDATE memory:otheractor SET lifecycle = 'retired'")
        .await
        .unwrap()
        .check()
        .unwrap();
    let (status, _) = call(
        app,
        "GET",
        "/api/v1/actors/detail?id=otheractor",
        None,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
