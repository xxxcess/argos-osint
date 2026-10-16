//! Offline reproduction of the Inferences latency report against the user's
//! imported portable profile (`fixtures/brief_user_profile.json`, keys
//! replaced). One request-aware mock plays every provider; the model named
//! in each request body decides what that route does.

use super::super::*;
use super::{cfg, Mock};
use crate::intel_recon::brief_pipeline::stage_views;
use crate::provider_attempt::mock::{self, Reply};
use crate::store::Store;
use serde_json::Value;
use std::sync::atomic::AtomicBool;

const PROFILE: &str = include_str!("fixtures/brief_user_profile.json");
const TITLE: &str = "Ferry operator suspends Baltic route";

/// How the first two routes of every chain (OpenRouter free models, then
/// Gemini) behave; the Nvidia route always answers.
#[derive(Clone, Copy, Debug)]
enum Weather {
    Healthy,
    /// OpenRouter streams an `error` event "A Timeout Occurred"; Gemini
    /// answers 503 "high demand"; Nvidia answers.
    FirstRoutesDown,
}

fn imported(base: &str) -> (crate::secrets::AuthFile, crate::provider::SettingsFile) {
    let plan = crate::config_transfer::parse_document(PROFILE).expect("profile parses");
    let mut snapshot = crate::config_transfer::ConfigurationSnapshot::default();
    snapshot.apply(&plan).expect("profile applies in memory");
    let mut auth = snapshot.auth;
    for secret in auth.accounts.values_mut() {
        secret.base_url = base.to_string();
    }
    (auth, snapshot.settings)
}

fn long_story() -> String {
    let mut text = format!("{TITLE}\n\nBy Zoë Kowalski\n\n");
    for n in 0..36 {
        text.push_str(&format!(
            "Officials said storms forced the suspension of crossing {n}, while unions denied staffing was a factor in the suspension on Monday.\n\n"
        ));
    }
    text.push_str("The harbour master confirmed the insurance certificate expired last week, a detail absent from the headline.\n");
    text
}

fn claims() -> String {
    r#"{"claims":[{"id":"c1","text":"The operator said storms forced the suspension.","attributed_to":"operator","modality":"asserted","polarity":"positive","evidence_ids":["d1"]},{"id":"c2","text":"Unions denied staffing was a factor in the suspension.","attributed_to":"unions","modality":"denied","polarity":"negative","evidence_ids":["d2"]},{"id":"c3","text":"The harbour master confirmed the insurance certificate expired.","attributed_to":"harbour master","modality":"asserted","polarity":"positive","evidence_ids":["d1"]}],"limit_reached":false}"#.into()
}

fn ids_under(value: &Value, key: &str) -> Vec<String> {
    value[key]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v["id"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// What a healthy model answers for the request in `body`.
fn answer(user: &str) -> String {
    let value: Value = serde_json::from_str(user).unwrap_or(Value::Null);
    if value.get("CANDIDATES").is_some() || value.get("candidates").is_some() {
        let candidates = ids_under(&value, "candidates");
        let items: Vec<Value> = candidates
            .iter()
            .map(|id| serde_json::json!({"id": id, "relevance": "support"}))
            .collect();
        return serde_json::json!({"items": items}).to_string();
    }
    if let Some(sentences) = value["CONTEXT_SENTENCES"].as_array() {
        let claim = value["CLAIM"]["id"].as_str().unwrap_or("c1");
        let evidence = sentences
            .first()
            .and_then(|s| s["id"].as_str())
            .unwrap_or("b1");
        return serde_json::json!({"inferences":[{"id":"i1","text":format!("The {claim} account may omit an insurance lapse."),"premise_claim_ids":[claim],"body_evidence_ids":[evidence],"support":"conditional","why":"The cited sentence supports the account only in part.","limitations":["Whether the lapse prevented sailing is unconfirmed"],"counterevidence_ids":[]}]}).to_string();
    }
    if let Some(items) = value["items"].as_array() {
        let items: Vec<Value> = items
            .iter()
            .filter_map(|i| i["id"].as_str())
            .map(|id| serde_json::json!({"id": id, "fidelity": "faithful"}))
            .collect();
        return serde_json::json!({"items": items}).to_string();
    }
    if let Some(candidates) = value["INFERENCE_CANDIDATES"].as_array() {
        let ids: Vec<&str> = candidates.iter().filter_map(|c| c["id"].as_str()).collect();
        return serde_json::json!({"selected_ids": ids}).to_string();
    }
    "{}".into()
}

fn reply(request: &str, weather: Weather) -> Reply {
    let path = request.lines().next().unwrap_or("");
    let body: Value = request
        .split("\r\n\r\n")
        .nth(1)
        .and_then(|b| serde_json::from_str(b).ok())
        .unwrap_or(Value::Null);
    let native = path.contains("decisions");
    let gemini = path.contains("generateContent");
    let model = if gemini {
        path.split("models/")
            .nth(1)
            .and_then(|m| m.split(':').next())
            .map(|m| format!("models/{m}"))
            .unwrap_or_default()
    } else {
        body["model"].as_str().unwrap_or("").to_string()
    };
    let streaming = body["stream"] == true || path.contains("streamGenerateContent");
    let down = matches!(weather, Weather::FirstRoutesDown);
    let openrouter = model.ends_with(":free") || model.contains("jev") || native;
    if down && openrouter {
        return if streaming {
            Reply::Sse(vec![
                mock::delta("{\"inferences\":["),
                "data: {\"error\":{\"message\":\"A Timeout Occurred\",\"code\":524}}\n\n".into(),
            ])
        } else {
            Reply::Json(
                503,
                r#"{"error":{"message":"upstream busy","code":503}}"#.into(),
            )
        };
    }
    if down && gemini {
        return Reply::Json(
            503,
            r#"{"error":{"message":"This model is currently experiencing high demand","code":503}}"#
                .into(),
        );
    }
    if native {
        let answers: serde_json::Map<String, Value> = body["questions"]
            .as_object()
            .map(|q| {
                q.keys()
                    .map(|id| {
                        (id.clone(), serde_json::json!({"type":"choice","choice":"support","confidence":0.9,"probabilities":{"support":0.9,"context":0.05,"counterevidence":0.03,"irrelevant":0.02}}))
                    })
                    .collect()
            })
            .unwrap_or_default();
        return Reply::Json(
            200,
            serde_json::json!({"id":"d","model":model,"answers":answers}).to_string(),
        );
    }
    if gemini {
        let user = body["contents"]
            .as_array()
            .and_then(|c| c.iter().rev().find(|c| c["role"] != "model"))
            .and_then(|c| c["parts"][0]["text"].as_str())
            .unwrap_or("");
        let text = answer(user);
        let part = match body["tools"][0]["functionDeclarations"][0]["name"].as_str() {
            Some(name) => {
                serde_json::json!({"functionCall":{"name":name,"args":serde_json::from_str::<Value>(&text).unwrap_or_default()}})
            }
            None => serde_json::json!({"text": text}),
        };
        let frame = serde_json::json!({"candidates":[{"content":{"role":"model","parts":[part]},"finishReason":"STOP"}]});
        return if streaming {
            Reply::Sse(vec![format!("data: {frame}\n\n")])
        } else {
            Reply::Json(200, frame.to_string())
        };
    }
    let user = body["messages"]
        .as_array()
        .and_then(|m| m.iter().rev().find(|m| m["role"] == "user"))
        .and_then(|m| m["content"].as_str())
        .unwrap_or("");
    let content = answer(user);
    if let Some(name) = body["tools"][0]["function"]["name"].as_str() {
        let call = serde_json::json!({"index":0,"id":"call_1","type":"function","function":{"name":name,"arguments":content}});
        return if streaming {
            Reply::Sse(vec![
                format!(
                    "data: {}\n\n",
                    serde_json::json!({"choices":[{"delta":{"tool_calls":[call]}}]})
                ),
                mock::finish("tool_calls"),
            ])
        } else {
            Reply::Json(200, serde_json::json!({"choices":[{"finish_reason":"tool_calls","message":{"role":"assistant","content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":name,"arguments":content}}]}}]}).to_string())
        };
    }
    if streaming {
        Reply::Sse(vec![mock::delta(&content), mock::finish("stop")])
    } else {
        mock::ok_json(&content, "stop")
    }
}

struct Run {
    requests: usize,
    by_operation: std::collections::BTreeMap<String, usize>,
    by_route: std::collections::BTreeMap<String, usize>,
    stage_state: String,
    stage_error: String,
}

async fn run_inferences(weather: Weather) -> Run {
    let server = mock::serve_by(move |request| reply(request, weather)).await;
    let (auth, settings) = imported(&server.base_url);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("argos.db");
    let store = Store::open(&path).unwrap();
    store.conn.execute("INSERT OR IGNORE INTO atlas_runs(id,started_at,state) VALUES('fixture',?1,'completed')", [now()]).unwrap();
    let row = super::article(
        "fixture",
        "a1",
        TITLE,
        "The operator said storms forced the suspension. Unions denied that staffing was a factor.",
        "https://example.test/a1",
    );
    store.atlas_upsert_article(&row).unwrap();
    let job = ensure_intel_brief_pipeline(&store, &row, &settings).unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    // Body and Claims run on a canned backend; only Inferences is live.
    let mut canned = Mock::happy();
    canned.body = Some(long_story());
    canned
        .responses
        .insert("intel_brief_claims".into(), vec![claims()]);
    let mut roots = Vec::new();
    while let Some(c) = claim_next(&path, "repro", &cfg()).unwrap() {
        roots.push(c);
    }
    for claimed in roots {
        let outcome = run_claimed(&path, claimed, &mut canned, &cancel, &cfg())
            .await
            .unwrap();
        println!("ROOT {outcome:?}");
    }
    let Some(claimed) = claim_next(&path, "repro", &cfg()).unwrap() else {
        let views = stage_views(&store.conn, &job).unwrap();
        panic!(
            "inferences not claimable: {:?}",
            views.iter().map(|v| v.row.clone()).collect::<Vec<_>>()
        );
    };
    assert_eq!(claimed.operation, "intel_brief_inferences");
    let mut backend = RuntimeBackend {
        db_path: path.clone(),
        keys: Default::default(),
        auth,
        settings,
        user_agent: None,
    };
    let _ = run_claimed(&path, claimed, &mut backend, &cancel, &cfg()).await;
    let mut by_operation = std::collections::BTreeMap::new();
    let mut by_route = std::collections::BTreeMap::new();
    let mut stmt = store
        .conn
        .prepare(
            "SELECT operation_id, provider, model FROM recon_model_attempts WHERE dispatched=1",
        )
        .unwrap();
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .unwrap();
    for r in rows {
        let (op, provider, model) = r.unwrap();
        let key = op.split(':').nth(5).unwrap_or("?").to_string();
        if std::env::var("REPRO_DEBUG").is_ok() {
            println!("ATTEMPT {op} {provider} {model}");
        }
        *by_operation.entry(key).or_insert(0) += 1;
        *by_route.entry(format!("{provider} {model}")).or_insert(0) += 1;
    }
    let view = stage_views(&store.conn, &job)
        .unwrap()
        .into_iter()
        .find(|v| v.operation == "intel_brief_inferences")
        .unwrap();
    if std::env::var("REPRO_DEBUG").is_ok() {
        let mut stmt = store.conn.prepare("SELECT route_index, attempt, outcome, failure_category, error_message FROM recon_model_attempts WHERE dispatched=1 ORDER BY started_at").unwrap();
        for r in stmt
            .query_map([], |r| {
                Ok(format!(
                    "{} {} {} {} {}",
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?
                ))
            })
            .unwrap()
        {
            println!("ROW {}", r.unwrap());
        }
    }
    Run {
        requests: server.hits.load(std::sync::atomic::Ordering::SeqCst),
        by_operation,
        by_route,
        stage_state: view.state.clone(),
        stage_error: view.row.clone(),
    }
}

/// Before the stage ledger this scenario cost 80 wire requests (60 of them
/// per-batch classifier operations each restarting at the dead route 0).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dead_first_routes_do_not_multiply_across_operations() {
    let healthy = run_inferences(Weather::Healthy).await;
    assert_eq!(healthy.stage_state, "completed");
    assert!(healthy.requests <= 20, "{:?}", healthy.by_operation);
    let down = run_inferences(Weather::FirstRoutesDown).await;
    assert_eq!(down.stage_state, "completed", "{}", down.stage_error);
    assert!(
        down.requests <= 36,
        "{} requests: {:?} {:?}",
        down.requests,
        down.by_operation,
        down.by_route
    );
    // Only the first concurrent wave (3 slots) can still reach a route that
    // is not yet known to be down.
    assert!(down.by_route["openrouter typesafe/jev-1.13"] <= 8);
    assert!(down.by_route["google models/gemini-3.8-flash"] <= 3);
}
