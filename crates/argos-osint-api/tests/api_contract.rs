#![cfg(feature = "embedded")]
#[cfg(feature = "classifier")]
#[path = "../../argos-osint-core/tests/fixtures/classifier/admission-recipe.rs"]
mod classifier_fixture;
#[cfg(feature = "classifier")]
#[path = "support/current_admission.rs"]
mod current_admission;
use argos_osint_api::{loopback_url, network_router, router, ApiState, NetworkPolicy};
use argos_osint_core::{
    storage::{atlas, open::Target, repo::Repository},
    store::AtlasArticleRow,
};
use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use fleetworks_contracts::{ATLAS_CYCLES_PATH, JOB_CANCEL_PATH, JOB_DETAIL_PATH, JOB_RETRY_PATH};
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
    call_as(app, method, path, origin, None, body).await
}

async fn call_as(
    app: &Router,
    method: &str,
    path: &str,
    origin: Option<&str>,
    token: Option<&str>,
    body: Value,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("host", "127.0.0.1:8080");
    if let Some(origin) = origin {
        request = request.header("origin", origin);
    }
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
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
async fn atlas_submission_and_job_controls_use_authenticated_durable_ids() {
    use argos_osint_core::storage::job_dispatch::{self, ATLAS_POOL};

    const TOKEN: &str = "atlas-test-bearer-0123456789abcdefghijkl";
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::connect(
        &Target::Embedded {
            path: dir.path().join("db"),
        },
        "argos",
        "dispatch",
    )
    .await
    .unwrap();
    repo.db()
        .query("UPSERT storage_meta:identity SET instance_id='fixture-generation' RETURN NONE")
        .await
        .unwrap()
        .check()
        .unwrap();
    job_dispatch::enable_pool(repo.db(), ATLAS_POOL)
        .await
        .unwrap();
    let state = ApiState {
        repo,
        instance_id: "dispatch-fixture".into(),
        port: 8080,
        origins: Arc::new(vec![]),
    };
    let app = network_router(
        state.clone(),
        NetworkPolicy::new(&["127.0.0.1:8080".into()], TOKEN).unwrap(),
    );
    let legacy = router(state);
    let request = json!({"request_id":"offline_cycle-1"});
    assert_eq!(
        call(&legacy, "POST", ATLAS_CYCLES_PATH, None, request.clone())
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(&app, "POST", ATLAS_CYCLES_PATH, None, request.clone())
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call_as(
            &app,
            "POST",
            ATLAS_CYCLES_PATH,
            None,
            Some(TOKEN),
            json!({"request_id":"../invalid"})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let (status, receipt) = call_as(
        &app,
        "POST",
        ATLAS_CYCLES_PATH,
        None,
        Some(TOKEN),
        request.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(
        receipt,
        json!({
            "job_id":"fleet-atlas-job-offline_cycle-1",
            "run_id":"fleet-atlas-run-offline_cycle-1"
        })
    );
    assert_eq!(
        call_as(&app, "POST", ATLAS_CYCLES_PATH, None, Some(TOKEN), request).await,
        (StatusCode::ACCEPTED, receipt.clone())
    );
    let job_id = receipt["job_id"].as_str().unwrap();
    let path = format!("{JOB_DETAIL_PATH}?id={job_id}");
    assert_eq!(
        call_as(&app, "GET", &path, None, Some(TOKEN), Value::Null)
            .await
            .1["job"]["state"],
        "queued"
    );
    let cancel = json!({"id":job_id});
    assert_eq!(
        call(&legacy, "POST", JOB_CANCEL_PATH, None, cancel.clone())
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call_as(
            &app,
            "POST",
            JOB_CANCEL_PATH,
            None,
            Some(TOKEN),
            cancel.clone()
        )
        .await,
        (StatusCode::OK, json!({"done":true}))
    );
    assert_eq!(
        call_as(&app, "GET", &path, None, Some(TOKEN), Value::Null)
            .await
            .1["job"]["state"],
        "cancelled"
    );
    assert_eq!(
        call_as(&app, "POST", JOB_RETRY_PATH, None, Some(TOKEN), cancel).await,
        (StatusCode::OK, json!({"requeued":0}))
    );
}
#[tokio::test]
#[cfg(feature = "classifier")]
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
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let articles = (0..5)
        .map(|i| AtlasArticleRow {
            run_id: "r1".into(),
            id: format!("a{i}"),
            title: format!("Harbor signal {i}"),
            description: "Shipping news".into(),
            url: format!("https://example.test/a{i}"),
            source_name: "Fixture Wire".into(),
            source_domain: "example.test".into(),
            provider: "newsapi".into(),
            published_at: format!("{today}T12:00:00Z"),
            seen_at: format!("{today}T13:00:00Z"),
            ..Default::default()
        })
        .collect::<Vec<_>>();
    current_admission::admit(&repo, &articles).await;
    let body = argos_osint_core::storage::article_body::ensure(
        &repo,
        "a0",
        "r1",
        "https://example.test/a0",
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
        "https://example.test/a0",
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
    let (status, tools) = call(&app, "GET", "/api/v1/tools", None, Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert!(tools["tools"]
        .as_array()
        .is_some_and(|tools| !tools.is_empty()));
    let (status, models) = call(&app, "GET", "/api/v1/models/defaults", None, Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert!(models["defaults"].is_object());
    assert!(models.get("auth").is_none());
    let (status, profile) = call(&app, "GET", "/api/v1/profile", None, Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        profile["workspace"],
        dir.path().join("home").to_string_lossy().as_ref()
    );
    assert_eq!(profile["access_mode"], "loopback");
    let (status, logs) = call(&app, "GET", "/api/v1/logs?limit=1", None, Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert!(logs["rows"].as_array().is_some_and(|rows| rows.len() <= 1));
    let (status, _) = call(&app, "GET", "/api/v1/logs?limit=101", None, Value::Null).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
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
    assert_eq!(
        call(
            &app,
            "GET",
            &format!("/api/v1/articles?category=geopolitical&day={today}"),
            None,
            Value::Null
        )
        .await
        .0,
        StatusCode::BAD_REQUEST,
        "legacy topic identifiers are not current domain filters"
    );
    let query = format!("/api/v1/articles?category=governance&day={today}&search=harbor&limit=2");
    for order in ["latest", "hottest"] {
        let (status, _) = call(
            &app,
            "GET",
            &format!("/api/v1/articles?category=governance&day={today}&sort={order}"),
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
        &format!("/api/v1/articles?category=governance&day={today}&sort=unknown"),
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
// The repository comes back so a contract can seed its own corpus.
async fn isolated_api() -> (tempfile::TempDir, Repository, Router) {
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
        repo: repo.clone(),
        instance_id: "contract-instance".into(),
        port: 8080,
        origins: Arc::new(vec!["http://localhost:3000".into()]),
    });
    (dir, repo, app)
}

#[tokio::test]
async fn capabilities_advertise_only_supported_features() {
    let (_dir, _repo, app) = isolated_api().await;
    let (status, body) = call(&app, "GET", "/api/v1/capabilities", None, Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["api_version"], 1);
    assert_eq!(body["auth"], false);
    assert_eq!(
        body["features"],
        json!([
            "bulletin",
            "brief",
            "related",
            "jobs",
            "topology",
            "live",
            "article-search"
        ])
    );
    assert_eq!(
        body["live"],
        json!({"path":"/api/v1/live","channels":["metrics","brief"],"version":1})
    );
}

/// Runtime contract for the discovery search route: the `ArticleSearchPage`
/// shape (matched_field/rank/excerpt/categories/revision/next_cursor) and the
/// paired capabilities feature.
#[tokio::test]
#[cfg(feature = "classifier")]
async fn article_search_serves_the_discovery_page_contract() {
    let (_dir, repo, app) = isolated_api().await;
    let source = |id: &str, title: &str, published: &str| AtlasArticleRow {
        run_id: "r1".into(),
        id: id.into(),
        title: title.into(),
        description: "Fixture description".into(),
        url: format!("https://example.test/{id}"),
        source_name: "Fixture Wire".into(),
        source_domain: "example.test".into(),
        published_at: published.into(),
        provider: "newsapi".into(),
        temperature: 1.0,
        seen_at: published.into(),
        ..Default::default()
    };
    current_admission::admit_with_titles(
        &repo,
        &[
            source(
                "lead-a",
                "Harbor Union approves the merger",
                "2026-01-04T00:00:00Z",
            ),
            source(
                "lead-b",
                "Harbor Union rejects the merger",
                "2026-01-03T00:00:00Z",
            ),
        ],
        ["Ballot patrol", "Harbor Union", "River sensor"],
    )
    .await;
    // New-looking tags and an admission timestamp cannot forge current authority.
    let mut legacy = source(
        "lead-legacy",
        "Harbor Union legacy merger",
        "2026-01-05T00:00:00Z",
    );
    legacy.category = "economic".into();
    legacy.categories = vec!["economic".into(), "human".into()];
    legacy.admitted_at = chrono::Utc::now().to_rfc3339();
    atlas::upsert_article(repo.db(), &legacy).await.unwrap();
    let query = format!(
        "/api/v1/articles/search?query={}&limit=1",
        url::form_urlencoded::byte_serialize("harbor union".as_bytes()).collect::<String>()
    );
    let (status, page) = call(&app, "GET", &query, None, Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    let hits = page["hits"].as_array().expect("hits array");
    assert_eq!(hits.len(), 1, "{page}");
    assert_eq!(page["coverage"]["current_admitted"], 2);
    assert_eq!(page["coverage"]["legacy_unmigrated"], 1);
    let (legacy_status, legacy_detail) = call(
        &app,
        "GET",
        "/api/v1/articles/detail?run=r1&id=lead-legacy",
        None,
        Value::Null,
    )
    .await;
    assert_eq!(legacy_status, StatusCode::OK);
    assert_eq!(legacy_detail["classification"]["legacy"], true);

    for field in ["matched_field", "rank", "excerpt", "categories", "revision"] {
        assert!(
            hits[0].get(field).is_some(),
            "hit misses {field}: {:?}",
            hits[0]
        );
    }
    assert_eq!(hits[0]["matched_field"], "title");
    assert!(hits[0]["rank"].as_f64().is_some(), "rank is a number");
    assert!(hits[0]["excerpt"]
        .as_str()
        .unwrap_or_default()
        .to_lowercase()
        .contains("harbor"));
    assert_eq!(hits[0]["categories"], json!(["economic", "human"]));
    assert_eq!(hits[0]["article_id"], "lead-a", "newest published first");
    assert!(!page["revision"].as_str().unwrap_or_default().is_empty());
    assert_eq!(page["diagnostic"], "");
    assert_eq!(page["next_cursor"].as_str(), Some("1"));
    let (status, next) = call(&app, "GET", &format!("{query}&cursor=1"), None, Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{next}");
    assert_eq!(next["hits"][0]["article_id"], "lead-b");
    assert_eq!(next["next_cursor"], Value::Null, "the corpus is exhausted");
    // A blank query is a validation error, not an empty success.
    assert_eq!(
        call(
            &app,
            "GET",
            "/api/v1/articles/search?query=%20",
            None,
            Value::Null
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    // The route and the capability stay paired.
    let (_, capabilities) = call(&app, "GET", "/api/v1/capabilities", None, Value::Null).await;
    assert!(capabilities["features"]
        .as_array()
        .expect("features")
        .iter()
        .any(|feature| feature == "article-search"));
}

#[tokio::test]
async fn missing_job_returns_public_error_without_internal_details() {
    let (_dir, _repo, app) = isolated_api().await;
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
    let (_dir, _repo, app) = isolated_api().await;
    let (status, body) = call(&app, "GET", "/api/v1/jobs", None, Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({"rows": []}));
}

#[tokio::test]
async fn retrying_missing_job_is_idempotent() {
    let (_dir, _repo, app) = isolated_api().await;
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
    let (_dir, _repo, app) = isolated_api().await;
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
