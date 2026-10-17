use super::*;

#[path = "brief_configured_live_tests.rs"]
mod configured_live;
#[path = "brief_user_chain_tests.rs"]
mod user_chain;
use crate::intel_recon::brief_pipeline::{ensure_chain, stage_views, BriefInputs};
use crate::intel_recon::clean_body::clean_article;
use std::sync::atomic::AtomicUsize;
use std::sync::Mutex;

const TITLE: &str = "Ferry operator suspends Baltic route";
const DESC: &str =
    "The operator said storms forced the suspension. Unions denied that staffing was a factor.";

fn story() -> String {
    format!(
        "{TITLE}\n\nBy Zoë Kowalski\n\n{}\n\nFollowing the suspension, the operator's harbour master confirmed on Monday that the ferry's insurance certificate expired last week, a detail absent from the headline.\n\n{}",
        "Officials said the crossing would stay closed while engineers inspected the vessel, and passengers were offered refunds or alternative sailings later in the month. ".repeat(4),
        "The company declined to say when the route would reopen, and regulators said an inquiry remained possible if the inspection found defects. ".repeat(4)
    )
}

fn article(run: &str, id: &str, title: &str, desc: &str, url: &str) -> AtlasArticleRow {
    AtlasArticleRow {
        run_id: run.into(),
        id: id.into(),
        title: title.into(),
        description: desc.into(),
        url: url.into(),
        country: String::new(),
        source_name: "Wire".into(),
        source_domain: "example.test".into(),
        published_at: "2026-10-12T08:00:00Z".into(),
        provider: "fixture".into(),
        temperature: 0.0,
        category: String::new(),
        seen_at: chrono::Utc::now().to_rfc3339(),
        author: String::new(),
        image_url: String::new(),
    }
}

struct Db {
    _dir: tempfile::TempDir,
    path: PathBuf,
}

fn db() -> Db {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("argos.db");
    let store = Store::open(&path).unwrap();
    store.conn.execute("INSERT OR IGNORE INTO atlas_runs(id,started_at,state) VALUES('fixture',?1,'completed')", [now()]).unwrap();
    store
        .atlas_upsert_article(&article(
            "fixture",
            "a1",
            TITLE,
            DESC,
            "https://example.test/a1",
        ))
        .unwrap();
    Db { _dir: dir, path }
}

fn inputs(rev: &str) -> BriefInputs {
    BriefInputs {
        article_id: "a1".into(),
        input_revision: rev.into(),
        role_snapshot: "{}".into(),
        title: TITLE.into(),
        stage_revisions: vec![],
        metadata: Default::default(),
    }
}

#[derive(Clone, Debug)]
struct Seen {
    stage: String,
    user: String,
    system: String,
    tokens: u32,
    reasoning: crate::request_policy::ReasoningEffort,
    budget: crate::request_policy::StageBudget,
    contract: Option<crate::provider_adapters::ResponseContract>,
}

#[derive(Clone, Default)]
struct Mock {
    /// Both roots wait here: only a truly parallel drainer gets past it.
    barrier: Option<Arc<tokio::sync::Barrier>>,
    inference_barrier: Option<Arc<tokio::sync::Barrier>>,
    inference_started: Arc<AtomicUsize>,
    inference_active: Arc<AtomicUsize>,
    inference_peak: Arc<AtomicUsize>,
    filter_inference_evidence: bool,
    body: Option<String>,
    complete: bool,
    prepared: usize,
    seen: Arc<Mutex<Vec<Seen>>>,
    responses: HashMap<String, Vec<String>>,
    fail: HashSet<String>,
    /// Fail with a typed executor timeout (what a real exhausted chain carries).
    fail_timeout: HashSet<String>,
    /// Claim ids whose Inferences request fails with a typed timeout.
    fail_claim: HashSet<String>,
    hang: HashSet<String>,
    delay: Option<Duration>,
    related: Vec<AtlasArticleRow>,
    assessed: Arc<Mutex<usize>>,
    fidelity_items: Arc<Mutex<Vec<crate::intel_recon::brief_fidelity::FidelityItem>>>,
    reject_denied_inference: bool,
    classified: Arc<Mutex<Vec<crate::intel_recon::brief_context::ContextBatch>>>,
    context_started: Arc<AtomicUsize>,
    reject_context: bool,
    invalid_context: bool,
    fail_context_claim: HashSet<String>,
    fail_context_sentence: HashSet<String>,
    forced: bool,
}

fn claims_ok() -> String {
    r#"{"claims":[{"id":"c1","text":"The operator said storms forced the suspension.","attributed_to":"operator","modality":"asserted","polarity":"positive","evidence_ids":["d1"]},{"id":"c2","text":"Unions denied staffing was a factor.","attributed_to":"unions","modality":"denied","polarity":"negative","evidence_ids":["d2"]}],"limit_reached":false}"#.into()
}

fn inferences_ok(body_id: &str) -> String {
    format!(
        r#"{{"inferences":[{{"id":"i1","text":"The suspension may also reflect an insurance lapse, not only weather.","premise_claim_ids":["c1"],"body_evidence_ids":["{body_id}"],"support":"conditional","why":"The harbour master confirmed the insurance certificate expired.","limitations":["Whether the lapse prevented sailing is unconfirmed"],"counterevidence_ids":[]}}]}}"#
    )
}

fn leads_ok() -> String {
    r#"{"leads":[{"article_id":"r1","inference_id":"i1","metadata_evidence_ids":["r1.t1"],"why":"The candidate title reports the same ferry's insurance lapse; the causal link to the suspension still needs verification.","verification_needed":"Confirm the lapse date."}]}"#.into()
}

fn insurance_sentence(markdown: &str) -> String {
    sentences("b", markdown)
        .into_iter()
        .find(|s| s.text.contains("insurance certificate"))
        .unwrap()
        .id
}

impl Mock {
    fn happy() -> Self {
        let body = story();
        let clean_md = validate_article_body_md(&body);
        let mut responses = HashMap::new();
        responses.insert("intel_brief_claims".into(), vec![claims_ok()]);
        responses.insert(
            "intel_brief_inferences".into(),
            vec![inferences_ok(&insurance_sentence(&clean_md))],
        );
        responses.insert("intel_brief_links".into(), vec![leads_ok()]);
        Mock {
            filter_inference_evidence: true,
            body: Some(body),
            complete: true,
            responses,
            related: vec![
                article(
                    "fixture",
                    "a2",
                    "Baltic ferry insurance lapsed last week, regulator says",
                    "The certificate expired days before the route closed.",
                    "https://other.test/x",
                ),
                article(
                    "fixture",
                    "a3",
                    "Baltic ferry insurance lapsed last week, regulator says",
                    "Syndicated copy.",
                    "https://syndicate.test/y",
                ),
                article("fixture", "a1", TITLE, DESC, "https://example.test/a1"),
            ],
            ..Default::default()
        }
    }
    fn requests(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

fn validate_article_body_md(raw: &str) -> String {
    crate::intel_recon::validate_article_body(raw, TITLE, "https://example.test/a1")
        .cleaned_markdown
}

impl StageBackend for Mock {
    fn prepare_body<'a>(
        &'a mut self,
        _ctx: &'a StageContext,
        _a: &'a AtlasArticleRow,
        force: bool,
        _c: &'a Arc<AtomicBool>,
    ) -> BoxFut<'a, Result<CleanArtifact>> {
        Box::pin(async move {
            if let Some(b) = &self.barrier {
                b.wait().await;
            }
            self.prepared += 1;
            self.forced |= force;
            let raw = self.body.clone().ok_or_else(|| anyhow!("story missing"))?;
            let mut artifact =
                clean_article(&raw, TITLE, DESC, "https://example.test/a1", &[], None).await;
            artifact.acquisition_complete = self.complete;
            Ok(artifact)
        })
    }
    fn assess<'a>(
        &'a mut self,
        _ctx: &'a StageContext,
        items: Vec<crate::intel_recon::brief_fidelity::FidelityItem>,
        _c: &'a Arc<AtomicBool>,
    ) -> BoxFut<'a, Result<HashMap<String, String>>> {
        Box::pin(async move {
            *self.assessed.lock().unwrap() += items.len();
            self.fidelity_items.lock().unwrap().extend(items.clone());
            assert!(items.len() <= crate::intel_recon::brief_fidelity::BATCH);
            Ok(items
                .into_iter()
                .map(|i| {
                    let label = if i.passages.is_empty()
                        || i.text.contains("UNFAITHFUL")
                        || (self.reject_denied_inference
                            && i.kind == "inference"
                            && i.passages.iter().any(|p| {
                                p.contains("Counterevidence:")
                                    && p.contains("denied the insurance certificate")
                            })) {
                        "insufficient"
                    } else {
                        "faithful"
                    };
                    (i.id, label.to_string())
                })
                .collect())
        })
    }
    fn related(&mut self, _a: &AtlasArticleRow) -> Result<Vec<AtlasArticleRow>> {
        Ok(self.related.clone())
    }
    fn classify_context<'a>(
        &mut self,
        _ctx: &'a StageContext,
        batch: crate::intel_recon::brief_context::ContextBatch,
        cancel: &'a Arc<AtomicBool>,
    ) -> BoxFut<'a, Result<HashMap<String, String>>> {
        let mock = self.clone();
        Box::pin(async move {
            ensure!(!cancel.load(Ordering::Acquire), "cancelled");
            mock.context_started.fetch_add(1, Ordering::AcqRel);
            mock.classified.lock().unwrap().push(batch.clone());
            if mock.hang.contains("intel_brief_context") {
                loop {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            }
            if mock.invalid_context {
                return Ok(HashMap::from([(
                    "invented-sentence".into(),
                    "support".into(),
                )]));
            }
            if batch.claim["id"]
                .as_str()
                .is_some_and(|id| mock.fail_context_claim.contains(id))
                && (mock.fail_context_sentence.is_empty()
                    || batch
                        .candidates
                        .iter()
                        .any(|s| mock.fail_context_sentence.contains(&s.id)))
            {
                return Err(crate::recon::model_exec::ModelOperationError {
                    category: Some(crate::provider_diag::Category::Timeout),
                    requests: 3,
                    history: vec![],
                    message: "context classifier routes exhausted".into(),
                }
                .into());
            }
            Ok(batch
                .candidates
                .into_iter()
                .map(|s| {
                    let relation = if mock.reject_context || s.text.contains("unrelated") {
                        "irrelevant"
                    } else if s.text.contains("corrected") || s.text.contains("denied") {
                        "counterevidence"
                    } else if s.text.contains("insurance certificate") {
                        "support"
                    } else {
                        "context"
                    };
                    (s.id, relation.to_string())
                })
                .collect())
        })
    }
    fn generate<'a>(
        &mut self,
        request: StageRequest,
        cancel: &'a Arc<AtomicBool>,
    ) -> BoxFut<'a, Result<String>> {
        let selection = request.user.contains("\"INFERENCE_CANDIDATES\"");
        let response_key = if selection {
            "inference_selection"
        } else {
            &request.stage
        };
        let response = self.responses.get_mut(response_key).map(|queue| {
            if queue.len() > 1 {
                queue.remove(0)
            } else {
                queue[0].clone()
            }
        });
        let mock = self.clone();
        Box::pin(async move {
            mock.seen.lock().unwrap().push(Seen {
                stage: request.stage.clone(),
                user: request.user.clone(),
                system: request.system.clone(),
                tokens: request.completion_tokens,
                reasoning: request.reasoning,
                budget: request.budget,
                contract: request.contract.clone(),
            });
            if request.stage == "intel_brief_claims" {
                if let Some(b) = &mock.barrier {
                    b.wait().await;
                }
            }
            struct Active(Arc<AtomicUsize>);
            impl Drop for Active {
                fn drop(&mut self) {
                    self.0.fetch_sub(1, Ordering::AcqRel);
                }
            }
            let _active = if request.stage == "intel_brief_inferences" && !selection {
                let n = mock.inference_started.fetch_add(1, Ordering::AcqRel);
                let active = mock.inference_active.fetch_add(1, Ordering::AcqRel) + 1;
                mock.inference_peak.fetch_max(active, Ordering::AcqRel);
                let guard = Active(mock.inference_active.clone());
                if n < INFERENCE_CONCURRENCY {
                    if let Some(b) = &mock.inference_barrier {
                        b.wait().await;
                    }
                }
                Some(guard)
            } else {
                None
            };
            if mock.hang.contains(&request.stage) {
                loop {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    let _ = cancel.load(Ordering::Acquire);
                }
            }
            if let Some(delay) = mock.delay {
                tokio::time::sleep(delay).await;
            }
            if request.stage == "intel_brief_inferences" && !selection {
                let claim = serde_json::from_str::<Value>(&request.user)
                    .ok()
                    .and_then(|u| u["CLAIM"]["id"].as_str().map(str::to_string));
                if claim.is_some_and(|id| mock.fail_claim.contains(&id)) {
                    return Err(crate::recon::model_exec::ModelOperationError {
                        category: Some(crate::provider_diag::Category::Timeout),
                        requests: 3,
                        history: vec![],
                        message: "operation wall-clock cap reached".into(),
                    }
                    .into());
                }
            }
            if mock.fail_timeout.contains(&request.stage) {
                return Err(crate::recon::model_exec::ModelOperationError {
                    category: Some(crate::provider_diag::Category::Timeout),
                    requests: 4,
                    history: vec![],
                    message: "model routes exhausted: first response/timeout: no response headers within 60s".into(),
                }
                .into());
            }
            if mock.fail.contains(&request.stage) {
                bail!("provider exhausted: server error");
            }
            let response = response.ok_or_else(|| anyhow!("no response"))?;
            if selection {
                return Ok(response);
            }
            if mock.filter_inference_evidence && request.stage == "intel_brief_inferences" {
                let user: Value = serde_json::from_str(&request.user)?;
                let ids: HashSet<&str> = user["CONTEXT_SENTENCES"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter_map(|s| s["id"].as_str())
                    .collect();
                let mut out: Value = serde_json::from_str(&response)?;
                out["inferences"].as_array_mut().unwrap().retain(|i| {
                    i["premise_claim_ids"] == json!([user["CLAIM"]["id"]])
                        && i["body_evidence_ids"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .all(|id| ids.contains(id.as_str().unwrap()))
                });
                return Ok(out.to_string());
            }
            Ok(response)
        })
    }
}

fn cfg() -> ExecConfig {
    ExecConfig {
        lease_secs: 120,
        heartbeat: Duration::from_millis(100),
    }
}

fn rows(path: &Path, job: &str) -> Vec<String> {
    let store = Store::open(path).unwrap();
    stage_views(&store.conn, job)
        .unwrap()
        .into_iter()
        .map(|v| v.row)
        .collect()
}

// §8.1: one chain, ordered dispatch, scoped inputs, zero requests when cached.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn one_chain_ordered_dispatch_scoped_inputs_and_no_repeat_requests() {
    let db = db();
    let job = {
        let store = Store::open(&db.path).unwrap();
        let a = ensure_chain(&store.conn, &inputs("r1"), &now()).unwrap();
        let b = ensure_chain(&store.conn, &inputs("r1"), &now()).unwrap();
        assert_eq!(a, b);
        a
    };
    let mut mock = Mock::happy();
    let cancel = Arc::new(AtomicBool::new(false));
    let outcomes = drain(&db.path, "w1", &mut mock, &cancel, &cfg())
        .await
        .unwrap();
    let ops: Vec<String> = outcomes
        .iter()
        .map(|o| match o {
            StageOutcome::Committed { operation, .. } => operation.clone(),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(
        ops,
        [
            "intel_brief_body",
            "intel_brief_claims",
            "intel_brief_inferences",
            "intel_brief_links"
        ]
    );
    let seen = mock.requests();
    assert_eq!(
        seen.iter().map(|s| s.stage.as_str()).collect::<Vec<_>>(),
        [
            "intel_brief_claims",
            "intel_brief_inferences",
            "intel_brief_inferences",
            "intel_brief_links"
        ]
    );
    assert_eq!(
        seen.iter().map(|s| s.tokens).collect::<Vec<_>>(),
        [2048, 3072, 3072, 1536]
    );
    // §5 stage budget table: Claims/Links thinking off, Inferences efficient.
    {
        use crate::request_policy::ReasoningEffort::{Efficient, Off};
        assert_eq!(
            seen.iter().map(|s| s.reasoning).collect::<Vec<_>>(),
            [Off, Efficient, Efficient, Off]
        );
        assert_eq!(
            seen.iter()
                .map(|s| (s.budget.answer, s.budget.thinking))
                .collect::<Vec<_>>(),
            [(2048, 4096), (3072, 6144), (3072, 6144), (1536, 3584)]
        );
        assert_eq!(seen[1].budget.reasoning_budget, Some(1024));
        assert_eq!(seen[1].budget.budgeted_ceiling, Some(4096));
    }
    // Every generation stage carries its authoritative schema, and the
    // compact limit text rides in the prompt.
    for (s, name) in seen.iter().zip([
        "brief_claims",
        "brief_inferences",
        "brief_inferences",
        "brief_leads",
    ]) {
        match &s.contract {
            Some(crate::provider_adapters::ResponseContract::GeneratedJson {
                name: n,
                schema: Some(_),
            }) => assert_eq!(n, name),
            other => panic!("{name}: missing schema contract {other:?}"),
        }
        assert!(s.system.contains(" Limits: "), "{name} limits text");
    }
    // Claims: title/description only.
    assert!(seen[0].user.contains("Unions denied"));
    assert!(!seen[0].user.contains("harbour master"));
    assert!(!seen[0].user.contains("Baltic ferry insurance lapsed"));
    // Each claim is analyzed separately with selected original body sentences.
    let store = Store::open(&db.path).unwrap();
    let body = stage_output(&store, &job, "intel_brief_body").unwrap();
    assert!(seen[1].user.contains("insurance certificate"));
    assert!(seen[1].user.contains("\"c1\"") && !seen[1].user.contains("\"c2\""));
    assert!(seen[2].user.contains("\"c2\"") && !seen[2].user.contains("\"c1\""));
    assert!(seen[1..3]
        .iter()
        .all(|s| !s.user.contains("CLEAN_ARTICLE_BODY") && !s.user.contains("regulator says")));
    // A body detail absent from the headline supports an inference, not a claim.
    let claims = stage_output(&store, &job, "intel_brief_claims").unwrap();
    assert!(!claims.to_string().contains("insurance"));
    let inf = stage_output(&store, &job, "intel_brief_inferences").unwrap();
    let span = &inf["inferences"][0]["evidence"][0];
    let md = body["markdown"].as_str().unwrap();
    assert!(md[span["start_byte"].as_u64().unwrap() as usize
        ..span["end_byte"].as_u64().unwrap() as usize]
        .contains("insurance certificate"));
    // Links: related metadata only, self + syndicated copy excluded.
    assert!(!seen[3].user.contains("harbour master"));
    assert!(
        seen[3].user.contains("r1")
            && !seen[3].user.contains("r2")
            && !seen[3].user.contains("Syndicated")
    );
    assert!(seen[3].system.starts_with(LINKS_PROMPT));
    let links = stage_output(&store, &job, "intel_brief_links").unwrap();
    assert_eq!(links["leads"][0]["article_id"], "a2");
    // Reopen: attaches to the completed chain and issues no requests.
    assert_eq!(
        ensure_chain(&store.conn, &inputs("r1"), &now()).unwrap(),
        job
    );
    drop(store);
    let mut again = Mock::happy();
    assert!(drain(&db.path, "w2", &mut again, &cancel, &cfg())
        .await
        .unwrap()
        .is_empty());
    assert!(again.requests().is_empty());
    assert_eq!(again.prepared, 0);
}

// §8.4: Stage 3 failure isolates; Retry resumes Stage 3 only.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stage_three_failure_keeps_predecessors_and_retry_runs_only_stage_three() {
    let db = db();
    let job = ensure_chain(&Store::open(&db.path).unwrap().conn, &inputs("r1"), &now()).unwrap();
    let mut mock = Mock::happy();
    mock.fail.insert("intel_brief_inferences".into());
    let cancel = Arc::new(AtomicBool::new(false));
    let outcomes = drain(&db.path, "w1", &mut mock, &cancel, &cfg())
        .await
        .unwrap();
    assert!(
        matches!(outcomes.last(), Some(StageOutcome::Failed { operation, .. }) if operation == "intel_brief_inferences")
    );
    let r = rows(&db.path, &job);
    assert!(r[0].starts_with("Completed"), "{r:?}");
    assert!(r[1].starts_with("Completed · 2 items"), "{r:?}");
    assert!(r[2].starts_with("Failed"), "{r:?}");
    assert_eq!(r[3], "Queued · blocked by Inferences");
    // Blocked dependents hold no lease and never dispatch.
    assert!(drain(&db.path, "w1", &mut mock, &cancel, &cfg())
        .await
        .unwrap()
        .is_empty());
    let before = mock.requests().len();
    {
        let store = Store::open(&db.path).unwrap();
        brief_pipeline::retry_stage(&store.conn, &job, &now()).unwrap();
    }
    mock.fail.clear();
    drain(&db.path, "w1", &mut mock, &cancel, &cfg())
        .await
        .unwrap();
    let after: Vec<String> = mock.requests()[before..]
        .iter()
        .map(|s| s.stage.clone())
        .collect();
    assert_eq!(
        after,
        [
            "intel_brief_inferences",
            "intel_brief_inferences",
            "intel_brief_links"
        ]
    );
    assert_eq!(mock.prepared, 1);
    assert!(rows(&db.path, &job)
        .iter()
        .all(|r| r.starts_with("Completed")));
}

// §8.4: empty arrays complete; no-inference Stage 4 uses no model.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn empty_inferences_complete_and_links_finish_locally() {
    let db = db();
    let job = ensure_chain(&Store::open(&db.path).unwrap().conn, &inputs("r1"), &now()).unwrap();
    let mut mock = Mock::happy();
    mock.responses.insert(
        "intel_brief_inferences".into(),
        vec![r#"{"inferences":[]}"#.into()],
    );
    let cancel = Arc::new(AtomicBool::new(false));
    drain(&db.path, "w1", &mut mock, &cancel, &cfg())
        .await
        .unwrap();
    assert!(!mock
        .requests()
        .iter()
        .any(|s| s.stage == "intel_brief_links"));
    let r = rows(&db.path, &job);
    assert_eq!(r[2], "Completed · no supported inferences");
    assert_eq!(r[3], "Completed · no supported inferences");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn empty_claims_skip_context_and_inference_requests() {
    let db = db();
    let job = ensure_chain(&Store::open(&db.path).unwrap().conn, &inputs("r1"), &now()).unwrap();
    let mut mock = Mock::happy();
    mock.responses
        .insert("intel_brief_claims".into(), vec![r#"{"claims":[]}"#.into()]);
    drain(
        &db.path,
        "w1",
        &mut mock,
        &Arc::new(AtomicBool::new(false)),
        &cfg(),
    )
    .await
    .unwrap();
    assert_eq!(mock.requests().len(), 1);
    assert!(mock.classified.lock().unwrap().is_empty());
    let output = stage_output(
        &Store::open(&db.path).unwrap(),
        &job,
        "intel_brief_inferences",
    )
    .unwrap();
    assert_eq!(output["analysis_requests"], json!(0));
    assert_eq!(output["inferences"], json!([]));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_relevant_context_still_analyzes_every_claim_without_body_fallback() {
    let db = db();
    let job = ensure_chain(&Store::open(&db.path).unwrap().conn, &inputs("r1"), &now()).unwrap();
    let mut mock = Mock::happy();
    mock.reject_context = true;
    drain(
        &db.path,
        "w1",
        &mut mock,
        &Arc::new(AtomicBool::new(false)),
        &cfg(),
    )
    .await
    .unwrap();
    assert!(!mock.classified.lock().unwrap().is_empty());
    let requests: Vec<_> = mock
        .requests()
        .into_iter()
        .filter(|r| r.stage == "intel_brief_inferences")
        .collect();
    assert_eq!(requests.len(), 2);
    for request in requests {
        let user: Value = serde_json::from_str(&request.user).unwrap();
        assert_eq!(user["CONTEXT_SENTENCES"], json!([]));
        assert!(request
            .system
            .contains("Empty selected context cannot support a conclusion"));
        assert!(!request.user.contains("insurance certificate"));
        assert!(!request.user.contains("CLEAN_ARTICLE_BODY"));
    }
    let output = stage_output(
        &Store::open(&db.path).unwrap(),
        &job,
        "intel_brief_inferences",
    )
    .unwrap();
    assert_eq!(output["analysis_requests"], json!(2));
    assert_eq!(output["inferences"], json!([]));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn invalid_classifier_decisions_fail_closed_before_synthesis() {
    let db = db();
    let job = ensure_chain(&Store::open(&db.path).unwrap().conn, &inputs("r1"), &now()).unwrap();
    let mut mock = Mock::happy();
    // Both claims need real classifier work. A claim with no candidates is a
    // valid independent empty-evidence analysis, not a classifier failure.
    mock.body = Some(format!(
        "{}\n\nUnions denied that staffing was a factor in the suspension, saying the crew schedule was unchanged.",
        story()
    ));
    mock.invalid_context = true;
    let outcomes = drain(
        &db.path,
        "w1",
        &mut mock,
        &Arc::new(AtomicBool::new(false)),
        &cfg(),
    )
    .await
    .unwrap();
    assert!(
        matches!(outcomes.last(), Some(StageOutcome::Failed { operation, .. }) if operation == "intel_brief_inferences")
    );
    assert!(mock
        .requests()
        .iter()
        .all(|r| r.stage == "intel_brief_claims"));
    let classified: HashSet<String> = mock
        .classified
        .lock()
        .unwrap()
        .iter()
        .filter_map(|b| b.claim["id"].as_str().map(str::to_string))
        .collect();
    assert_eq!(classified, HashSet::from(["c1".into(), "c2".into()]));
    assert!(stage_output(
        &Store::open(&db.path).unwrap(),
        &job,
        "intel_brief_inferences"
    )
    .is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inference_assessor_receives_selected_denial_even_when_synthesis_omits_it() {
    let db = db();
    let job = ensure_chain(
        &Store::open(&db.path).unwrap().conn,
        &inputs("counterevidence"),
        &now(),
    )
    .unwrap();
    let mut mock = Mock::happy();
    mock.body = Some(format!("{}\n\nThe operator denied the insurance certificate had expired, correcting the harbour master's earlier account of the suspension.", story()));
    mock.reject_denied_inference = true;
    let proposal: Value =
        serde_json::from_str(&mock.responses["intel_brief_inferences"][0]).unwrap();
    assert_eq!(proposal["inferences"][0]["counterevidence_ids"], json!([]));
    let outcomes = drain(
        &db.path,
        "w1",
        &mut mock,
        &Arc::new(AtomicBool::new(false)),
        &cfg(),
    )
    .await
    .unwrap();
    assert!(
        outcomes
            .iter()
            .all(|o| matches!(o, StageOutcome::Committed { .. })),
        "{outcomes:?}"
    );
    let assessed = mock.fidelity_items.lock().unwrap();
    let inference = assessed
        .iter()
        .find(|item| item.kind == "inference")
        .unwrap();
    assert!(inference
        .passages
        .iter()
        .any(|p| p.contains("confirmed") && p.contains("insurance certificate")));
    assert!(
        inference
            .passages
            .iter()
            .any(|p| p.starts_with("Counterevidence:")
                && p.contains("denied the insurance certificate")),
        "the assessor must see the source correction despite the missing model citation"
    );
    let mut without_denial = inference.clone();
    without_denial
        .passages
        .retain(|p| !p.starts_with("Counterevidence:"));
    assert_ne!(
        inference.receipt_key(),
        without_denial.receipt_key(),
        "support-only fidelity receipts cannot certify corrected evidence"
    );
    let output = stage_output(
        &Store::open(&db.path).unwrap(),
        &job,
        "intel_brief_inferences",
    )
    .unwrap();
    assert_eq!(output["inferences"], json!([]));
    assert_eq!(output["withheld"], json!(1));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancellation_drops_the_context_classifier_without_committing_inferences() {
    let db = db();
    let job = ensure_chain(&Store::open(&db.path).unwrap().conn, &inputs("r1"), &now()).unwrap();
    let mut mock = Mock::happy();
    mock.hang.insert("intel_brief_context".into());
    let started = mock.context_started.clone();
    let cancel = Arc::new(AtomicBool::new(false));
    let stop = cancel.clone();
    let path = db.path.clone();
    let runner = tokio::spawn(async move {
        drain(&path, "w1", &mut mock, &cancel, &cfg())
            .await
            .unwrap()
    });
    tokio::time::timeout(Duration::from_secs(30), async {
        while started.load(Ordering::Acquire) == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    stop.store(true, Ordering::Release);
    let outcomes = tokio::time::timeout(Duration::from_secs(5), runner)
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(outcomes.last(), Some(StageOutcome::Stale { operation }) if operation == "intel_brief_inferences")
    );
    assert!(stage_output(
        &Store::open(&db.path).unwrap(),
        &job,
        "intel_brief_inferences"
    )
    .is_err());
}

// Stage 1 admission: incomplete acquisition fails and blocks dependents.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn incomplete_acquisition_fails_body_and_blocks_only_its_descendants() {
    let db = db();
    let job = ensure_chain(&Store::open(&db.path).unwrap().conn, &inputs("r1"), &now()).unwrap();
    let mut mock = Mock::happy();
    mock.complete = false;
    let cancel = Arc::new(AtomicBool::new(false));
    let out = drain(&db.path, "w1", &mut mock, &cancel, &cfg())
        .await
        .unwrap();
    assert!(
        matches!(&out[0], StageOutcome::Failed { reason, .. } if reason.contains("acquisition"))
    );
    // A failed root blocks only its descendants: the Claims sibling still
    // runs from the metadata snapshot, Inferences and Links never dispatch.
    assert!(mock
        .requests()
        .iter()
        .all(|r| r.stage == "intel_brief_claims"));
    let r = rows(&db.path, &job);
    assert!(r[1].starts_with("Completed"), "{r:?}");
    assert_eq!(r[2], "Queued · blocked by Full article");
    assert_eq!(r[3], "Queued · blocked by Full article");
}

// §8.5: cancellation drops the in-flight request and writes nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancellation_drops_in_flight_request_and_prevents_commit() {
    let db = db();
    let job = ensure_chain(&Store::open(&db.path).unwrap().conn, &inputs("r1"), &now()).unwrap();
    let mut mock = Mock::happy();
    mock.hang.insert("intel_brief_claims".into());
    let path = db.path.clone();
    let j = job.clone();
    // Cancel once Claims is actually in flight (not after a fixed sleep, which
    // races Stage 1 under a loaded test run).
    let claims_task = brief_pipeline::task_id(&job, "intel_brief_claims");
    let canceller = std::thread::spawn(move || {
        let store = Store::open(&path).unwrap();
        for _ in 0..400 {
            let running: bool = store
                .conn
                .query_row(
                    "SELECT state='running' FROM argos_tasks WHERE id=?1",
                    [&claims_task],
                    |r| r.get(0),
                )
                .unwrap();
            if running {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        std::thread::sleep(Duration::from_millis(100));
        brief_pipeline::cancel_chain(&store.conn, &j, &now()).unwrap();
    });
    let cancel = Arc::new(AtomicBool::new(false));
    let out = tokio::time::timeout(
        Duration::from_secs(20),
        drain(&db.path, "w1", &mut mock, &cancel, &cfg()),
    )
    .await
    .expect("in-flight request was not cancelled")
    .unwrap();
    canceller.join().unwrap();
    assert!(
        matches!(out.last(), Some(StageOutcome::Stale { operation }) if operation == "intel_brief_claims")
    );
    let store = Store::open(&db.path).unwrap();
    assert!(stage_output(&store, &job, "intel_brief_claims").is_err());
}

// §8.5: a parent cancel also cancels the in-flight request.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn parent_cancel_drops_in_flight_request() {
    let db = db();
    ensure_chain(&Store::open(&db.path).unwrap().conn, &inputs("r1"), &now()).unwrap();
    let mut mock = Mock::happy();
    mock.hang.insert("intel_brief_claims".into());
    let cancel = Arc::new(AtomicBool::new(false));
    let c = cancel.clone();
    // Cancel once the Claims request is actually in flight (not on a timer:
    // with parallel roots the Body may finish first or second).
    let seen = mock.seen.clone();
    std::thread::spawn(move || {
        while !seen
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.stage == "intel_brief_claims")
        {
            std::thread::yield_now();
        }
        c.store(true, Ordering::Release);
    });
    let out = tokio::time::timeout(
        Duration::from_secs(10),
        drain(&db.path, "w1", &mut mock, &cancel, &cfg()),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(matches!(out.last(), Some(StageOutcome::Stale { .. })));
}

// §8.5: processing longer than the lease survives because the heartbeat
// renews through commit; concurrent reapers never reap it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn heartbeat_covers_processing_beyond_lease_and_reapers_do_not_reap() {
    let db = db();
    let job = ensure_chain(&Store::open(&db.path).unwrap().conn, &inputs("r1"), &now()).unwrap();
    let mut mock = Mock::happy();
    mock.delay = Some(Duration::from_millis(2500));
    let path = db.path.clone();
    let stop = Arc::new(AtomicBool::new(false));
    let s = stop.clone();
    let reaper = std::thread::spawn(move || {
        let mut reaped = 0;
        while !s.load(Ordering::Acquire) {
            let store = Store::open_light(&path).unwrap();
            reaped += tasks::interrupt_expired_leases(&store.conn, &now()).unwrap_or(0);
            std::thread::sleep(Duration::from_millis(100));
        }
        reaped
    });
    let cancel = Arc::new(AtomicBool::new(false));
    let config = ExecConfig {
        lease_secs: 1,
        heartbeat: Duration::from_millis(250),
    };
    let out = drain(&db.path, "w1", &mut mock, &cancel, &config)
        .await
        .unwrap();
    stop.store(true, Ordering::Release);
    assert_eq!(reaper.join().unwrap(), 0);
    assert_eq!(out.len(), 4, "{out:?}");
    assert!(out
        .iter()
        .all(|o| matches!(o, StageOutcome::Committed { .. })));
    let store = Store::open(&db.path).unwrap();
    let (attempts, recoveries): (i64, i64) = store
        .conn
        .query_row(
            "SELECT attempts,recoveries FROM argos_tasks WHERE id=?1",
            [brief_pipeline::task_id(&job, "intel_brief_claims")],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((attempts, recoveries), (1, 0));
}

// §8.5: stale late output after a reclaim is fenced; provider attempts survive.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stale_worker_output_is_fenced_after_epoch_change() {
    let db = db();
    let job = ensure_chain(&Store::open(&db.path).unwrap().conn, &inputs("r1"), &now()).unwrap();
    let mut mock = Mock::happy();
    mock.delay = Some(Duration::from_millis(600));
    let path = db.path.clone();
    let task = brief_pipeline::task_id(&job, "intel_brief_claims");
    let t = task.clone();
    std::thread::spawn(move || {
        // Simulate a crash/reclaim: another worker's recovery advanced the epoch.
        loop {
            std::thread::sleep(Duration::from_millis(50));
            let store = Store::open_light(&path).unwrap();
            if store.conn.execute("UPDATE argos_tasks SET lease_epoch=lease_epoch+1 WHERE id=?1 AND state='running'", [&t]).unwrap() > 0 {
                break;
            }
        }
    });
    let cancel = Arc::new(AtomicBool::new(false));
    let out = drain(
        &db.path,
        "w1",
        &mut mock,
        &cancel,
        &ExecConfig {
            lease_secs: 120,
            heartbeat: Duration::from_millis(100),
        },
    )
    .await
    .unwrap();
    assert!(
        matches!(out.last(), Some(StageOutcome::Stale { operation }) if operation == "intel_brief_claims")
    );
    let store = Store::open(&db.path).unwrap();
    assert!(stage_output(&store, &job, "intel_brief_claims").is_err());
    let note = store
        .app_state_get(&format!("intel-brief-heartbeat:{task}"))
        .unwrap();
    assert!(note.is_some_and(|n| n.contains("renewal failed")));
}

// Article length does not multiply analysis requests or send unrelated prose.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn long_body_uses_bounded_selected_context_and_one_request_per_claim() {
    let db = db();
    let job = ensure_chain(&Store::open(&db.path).unwrap().conn, &inputs("r1"), &now()).unwrap();
    let mut mock = Mock::happy();
    let mut long = story();
    for i in 0..500 {
        long.push_str(&format!("\n\nParagraph {i}: inspectors recorded hull measurements at berth {i} and filed their notes with the port authority before noon."));
    }
    long.push_str("\n\nFinal note: the operator later corrected its statement about the storm.");
    long.push_str("\n\nThe unrelated auditor denied a budget variance in a separate dispute.");
    mock.body = Some(long);
    mock.inference_barrier = Some(Arc::new(tokio::sync::Barrier::new(2)));
    mock.delay = Some(Duration::from_millis(20));
    let cancel = Arc::new(AtomicBool::new(false));
    tokio::time::timeout(
        Duration::from_secs(120),
        drain(&db.path, "w1", &mut mock, &cancel, &cfg()),
    )
    .await
    .unwrap()
    .unwrap();
    let analyses: Vec<Seen> = mock
        .requests()
        .into_iter()
        .filter(|s| s.stage == "intel_brief_inferences")
        .collect();
    assert_eq!(analyses.len(), 2);
    assert!(analyses
        .iter()
        .all(|w| !w.user.contains("PRIOR_WINDOW_PROPOSALS")
            && !w.user.contains("CLEAN_ARTICLE_BODY")
            && !w.user.contains("Paragraph 499")
            && !w.user.contains("unrelated auditor")
            && w.system.len() + w.user.len() <= CLAIM_INFERENCE_REQUEST_BYTES));
    assert_eq!(mock.inference_peak.load(Ordering::Acquire), 2);
    assert_eq!(mock.inference_active.load(Ordering::Acquire), 0);
    assert!(analyses
        .iter()
        .any(|w| w.user.contains("corrected its statement")));
    let classified = mock.classified.lock().unwrap();
    assert!(classified.iter().any(|batch| batch
        .candidates
        .iter()
        .any(|s| s.text.contains("unrelated auditor"))));
    assert!(classified.iter().all(
        |batch| batch.payload_bytes() <= crate::intel_recon::brief_context::CONTEXT_BATCH_BYTES
    ));
    let store = Store::open(&db.path).unwrap();
    let inf = stage_output(&store, &job, "intel_brief_inferences").unwrap();
    let body = stage_output(&store, &job, "intel_brief_body").unwrap();
    let (original, _) = crate::intel_recon::sentence_index::body_sentences(&body);
    let sent: HashSet<String> = analyses
        .iter()
        .flat_map(|w| {
            let user: Value = serde_json::from_str(&w.user).unwrap();
            let body = user["CONTEXT_SENTENCES"].as_array().unwrap();
            body.iter()
                .map(|s| s["id"].as_str().unwrap().to_string())
                .collect::<Vec<_>>()
        })
        .collect();
    assert!(
        sent.len() < original.len() / 4,
        "unrelated article prose stays local"
    );
    assert_eq!(inf["analysis_requests"], json!(2));
    assert_eq!(inf["path"], json!("claim_context"));
    let receipts: HashSet<String> = inf["analysis_sentence_ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|id| id.as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        sent, receipts,
        "publication knows exactly which sentences Synthesis saw"
    );
    assert!(inf["inferences"].as_array().unwrap().len() <= MAX_INFERENCES);
}

// §8.2 validation.
#[tokio::test]
async fn semantic_dedup_selects_existing_objects_and_preserves_distinctions() {
    let db = db();
    let mut mock = Mock::default();
    mock.responses.insert(
        "inference_selection".into(),
        vec![r#"{"selected_ids":["i1","i3"]}"#.into()],
    );
    let item = |id: &str, text: &str| {
        json!({"id":id,"text":text,
        "why":"The cited body supports this conclusion.","support":"conditional",
        "limitations":["Cause remains unconfirmed"],"body_evidence_ids":["b17"],
        "evidence":[{"start_byte":123,"end_byte":456}],"fidelity":"faithful"})
    };
    let original = item("old1", "The insurance lapse may have delayed reopening.");
    let mut out = json!({"inferences":[original.clone(),
        item("old2", "The insurance lapse may have delayed reopening!"),
        item("old3", "Reopening could have been held up by expired insurance."),
        item("old4", "Inspections may delay reopening even after insurance is renewed.") ]});
    dedup_inferences(
        &db.path,
        &mut mock,
        &StageContext::default(),
        &Arc::new(AtomicBool::new(false)),
        &mut out,
    )
    .await
    .unwrap();
    let kept = out["inferences"].as_array().unwrap();
    assert_eq!(kept.len(), 2);
    assert_eq!(kept[0]["text"], original["text"]);
    assert_eq!(kept[0]["evidence"], original["evidence"]);
    assert!(kept[1]["text"]
        .as_str()
        .unwrap()
        .contains("even after insurance"));
    assert_eq!(kept[1]["id"], "i2");
    let seen = mock.requests();
    assert_eq!(seen.len(), 1);
    assert!(seen[0].system.contains("same underlying meaning"));
    assert!(seen[0]
        .system
        .contains("contradictory conclusions are not duplicates"));
    assert!(!seen[0].user.contains("CLEAN_ARTICLE_BODY"));
    let offered = HashSet::from(["i1".into(), "i2".into()]);
    assert!(validate_inference_selection(r#"{"selected_ids":["invented"]}"#, &offered).is_err());
    assert!(validate_inference_selection(r#"{"selected_ids":["i1","i1"]}"#, &offered).is_err());
    assert!(validate_inference_selection(r#"{"selected_ids":[]}"#, &offered).is_err());
}

#[tokio::test]
async fn semantic_dedup_is_bounded_and_visits_later_candidates() {
    let db = db();
    let mut mock = Mock::default();
    mock.responses.insert(
        "inference_selection".into(),
        vec![
            r#"{"selected_ids":["i12"]}"#.into(),
            r#"{"selected_ids":["i12","i15"]}"#.into(),
        ],
    );
    let mut out = json!({"inferences":(1..=15).map(|n|json!({"id":format!("old{n}"),
        "text":format!("Distinct conclusion number {n}."),"why":"Original reason.","evidence":[{"id":format!("b{n}")}]})).collect::<Vec<_>>()});
    dedup_inferences(
        &db.path,
        &mut mock,
        &StageContext::default(),
        &Arc::new(AtomicBool::new(false)),
        &mut out,
    )
    .await
    .unwrap();
    assert_eq!(
        out["inferences"][1]["text"],
        "Distinct conclusion number 15."
    );
    for seen in mock.requests() {
        let user: Value = serde_json::from_str(&seen.user).unwrap();
        assert!(user["INFERENCE_CANDIDATES"].as_array().unwrap().len() <= INFERENCE_DEDUP_BATCH);
    }
}

#[test]
fn exact_dedup_preserves_different_conditions_for_semantic_selection() {
    let mut out = json!({"inferences":[
        {"text":"The lapse delayed reopening.","support":"supported","limitations":[],"counterevidence_ids":[]},
        {"text":"The lapse delayed reopening!","support":"supported","limitations":[],"counterevidence_ids":[]},
        {"text":"The lapse delayed reopening.","support":"conditional","limitations":["Timing is disputed"],"counterevidence_ids":["b19"]}
    ]});
    exact_dedup_inferences(&mut out);
    assert_eq!(out["inferences"].as_array().unwrap().len(), 2);
    assert_eq!(out["inferences"][1]["counterevidence_ids"], json!(["b19"]));
}

#[test]
fn claim_scoped_inferences_require_the_assigned_premise_and_offered_sentences() {
    let source = "Zoë confirmed an insurance certificate lapse. Unrelated context.";
    let body = sentences("b", source);
    let good = inferences_ok("b1");
    let out = validate_claim_inferences(&good, "c1", &body[..1]).unwrap();
    let span = &out["inferences"][0]["evidence"][0];
    assert_eq!(
        span,
        &json!({"id":"b1","start_byte":body[0].start_byte,"end_byte":body[0].end_byte})
    );
    assert_eq!(
        &source[span["start_byte"].as_u64().unwrap() as usize
            ..span["end_byte"].as_u64().unwrap() as usize],
        body[0].text
    );
    assert!(validate_claim_inferences(&good, "c2", &body[..1]).is_err());
    assert!(validate_claim_inferences(&good.replace("[\"c1\"]", "[]"), "c1", &body[..1]).is_err());
    assert!(validate_claim_inferences(
        &good.replace("[\"c1\"]", "[\"c1\",\"c1\"]"),
        "c1",
        &body[..1]
    )
    .is_err());
    assert!(validate_claim_inferences(&inferences_ok("b2"), "c1", &body[..1]).is_err());
    assert!(validate_claim_inferences(&good, "c1", &[]).is_err());
    assert!(validate_claim_inferences(
        r#"{"inferences":[],"empty_reason":"No relevant evidence was selected."}"#,
        "c1",
        &[]
    )
    .is_ok());
}

#[test]
fn server_owned_counterevidence_keeps_exact_spans_when_the_model_omits_citations() {
    let source = "The certificate expired. The operator denied the certificate had expired.";
    let body = sentences("b", source);
    let mut output = validate_claim_inferences(&inferences_ok("b1"), "c1", &body).unwrap();
    retain_inference_counterevidence(&mut output, &body, &body[1..]).unwrap();
    assert_eq!(output["inferences"][0]["body_evidence_ids"], json!(["b1"]));
    assert_eq!(
        output["inferences"][0]["counterevidence_ids"],
        json!(["b2"])
    );
    let span = &output["inferences"][0]["counterevidence"][0];
    assert_eq!(span["id"], json!("b2"));
    assert_eq!(span["start_byte"], json!(body[1].start_byte));
    assert_eq!(span["end_byte"], json!(body[1].end_byte));
    assert_eq!(
        &source[span["start_byte"].as_u64().unwrap() as usize
            ..span["end_byte"].as_u64().unwrap() as usize],
        body[1].text
    );
}

#[test]
fn bounded_analysis_prioritizes_late_counterevidence_and_records_omissions() {
    let mut clean = (0..80)
        .map(|n| {
            format!(
                "The operator said the suspension required review number {n} with {}. ",
                "documented inspection detail ".repeat(12)
            )
        })
        .collect::<String>();
    clean.push_str("The operator corrected its suspension account after the regulator denied the original explanation.");
    let index = crate::intel_recon::sentence_index::SentenceIndex::local(&clean, "test");
    let claim = json!({"id":"c1","text":"The operator said storms forced the suspension."});
    let mut context =
        crate::intel_recon::brief_context::claim_candidates(&index, &clean, &claim, &[]).unwrap();
    assert!(
        context.candidates.len() > 24,
        "candidate selection does not discard later matches"
    );
    let labels: HashMap<String, String> = context
        .candidates
        .iter()
        .map(|s| {
            (
                s.id.clone(),
                if s.text.contains("corrected") {
                    "counterevidence"
                } else {
                    "context"
                }
                .into(),
            )
        })
        .collect();
    let selected = context.candidates.iter().map(Sentence::from).collect();
    let selected = bound_claim_context(&mut context, selected, &labels, INFERENCES_PROMPT).unwrap();
    assert!(selected.iter().any(|s| s.text.contains("corrected")));
    assert!(selected.len() < context.candidates.len());
    assert!(context
        .omitted
        .iter()
        .any(|s| s.reason == "analysis_context_budget"));
    assert!(
        claim_inferences_user(&context, &selected, &labels).len() + INFERENCES_PROMPT.len()
            <= CLAIM_INFERENCE_REQUEST_BYTES
    );
    assert!(selected
        .windows(2)
        .all(|pair| pair[0].start_byte < pair[1].start_byte));
}

#[test]
fn analysis_context_dedup_preserves_currency_signs_case_and_relation() {
    let source = [
        "The operator denied losing $5 million.",
        "The operator denied losing €5 million.",
        "The operator denied losing +5 million.",
        "The operator denied losing -5 million.",
        "The operator denied losing $5 million.",
        "the operator denied losing $5 million.",
        "The operator denied losing $5 million.",
    ]
    .join("\n\n");
    let index = crate::intel_recon::sentence_index::SentenceIndex::local(&source, "test");
    let mut context = crate::intel_recon::brief_context::claim_candidates(
        &index,
        &source,
        &json!({"id":"c1","text":"The operator denied a loss."}),
        &[],
    )
    .unwrap();
    let selected = index.wire_sentences(&source);
    let mut labels: HashMap<String, String> = selected
        .iter()
        .map(|sentence| (sentence.id.clone(), "counterevidence".into()))
        .collect();
    labels.insert("b7".into(), "support".into());
    let retained = bound_claim_context(&mut context, selected, &labels, INFERENCES_PROMPT).unwrap();
    assert_eq!(
        retained
            .iter()
            .map(|sentence| sentence.id.as_str())
            .collect::<Vec<_>>(),
        ["b1", "b2", "b3", "b4", "b6", "b7"]
    );
    assert_eq!(
        context.omitted,
        [crate::intel_recon::brief_context::OmittedSentence {
            id: "b5".into(),
            reason: "duplicate_selected_sentence".into()
        }],
        "only identical text with the same relation is deduplicated"
    );
    for sentence in retained {
        assert_eq!(
            &source[sentence.start_byte..sentence.end_byte],
            sentence.text
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_claim_failure_or_cancellation_drops_siblings_without_partial_commit() {
    for cancel_stage in [false, true] {
        let db = db();
        let job =
            ensure_chain(&Store::open(&db.path).unwrap().conn, &inputs("r1"), &now()).unwrap();
        let mut mock = Mock::happy();
        mock.inference_barrier = Some(Arc::new(tokio::sync::Barrier::new(2)));
        if cancel_stage {
            mock.hang.insert("intel_brief_inferences".into());
        } else {
            mock.fail.insert("intel_brief_inferences".into());
        }
        let active = mock.inference_active.clone();
        let started = mock.inference_started.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = cancel.clone();
        let path = db.path.clone();
        let runner = tokio::spawn(async move {
            let outcomes = drain(&path, "w1", &mut mock, &cancel, &cfg())
                .await
                .unwrap();
            (outcomes, mock)
        });
        tokio::time::timeout(Duration::from_secs(30), async {
            while started.load(Ordering::Acquire) < 2 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        if cancel_stage {
            stop.store(true, Ordering::Release);
        }
        let (outcomes, mut mock) = tokio::time::timeout(Duration::from_secs(5), runner)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            active.load(Ordering::Acquire),
            0,
            "all in-flight claim analyses dropped"
        );
        let store = Store::open(&db.path).unwrap();
        assert!(stage_output(&store, &job, "intel_brief_inferences").is_err());
        assert!(stage_output(&store, &job, "intel_brief_links").is_err());
        if !cancel_stage {
            assert!(
                matches!(outcomes.last(),Some(StageOutcome::Failed{operation,..}) if operation=="intel_brief_inferences")
            );
            assert_eq!(
                started.load(Ordering::Acquire),
                2,
                "no further claim analyses after failure"
            );
            brief_pipeline::retry_stage(&store.conn, &job, &now()).unwrap();
            mock.fail.clear();
            // The two-party barrier synchronizes only the failed epoch.
            // Retry's cumulative dispatch counter must not re-enter it.
            mock.inference_barrier = None;
            tokio::time::timeout(
                Duration::from_secs(30),
                drain(&db.path, "w2", &mut mock, &stop, &cfg()),
            )
            .await
            .expect("claim inference retry must finish without first-epoch barriers")
            .unwrap();
            assert_eq!(
                started.load(Ordering::Acquire),
                4,
                "two failed and two successful claim analyses"
            );
            assert_eq!(active.load(Ordering::Acquire), 0);
            assert!(stage_output(&store, &job, "intel_brief_inferences").is_ok());
            assert_eq!(mock.prepared, 1, "roots reused on retry");
        }
    }
}

#[test]
fn validators_reject_unknown_ids_overlong_duplicates_fences_links_and_sixth_inference() {
    let meta = metadata_sentences(&article("r", "a", TITLE, DESC, "u"));
    assert!(validate_claims(&claims_ok(), &meta).is_ok());
    assert!(validate_claims(&format!("```json\n{}\n```", claims_ok()), &meta).is_err());
    assert!(validate_claims(r#"{"claims":[{"text":"x","evidence_ids":["b9"]}]}"#, &meta).is_err());
    assert!(validate_claims(
        &format!(
            r#"{{"claims":[{{"text":"{}","evidence_ids":["t1"]}}]}}"#,
            "word ".repeat(36)
        ),
        &meta
    )
    .is_err());
    assert!(validate_claims(r#"{"claims":[{"text":"Same.","evidence_ids":["t1"]},{"text":"same","evidence_ids":["d1"]}]}"#, &meta).is_err());
    let empty = validate_claims(r#"{"claims":[]}"#, &meta).unwrap();
    assert!(!empty["empty_reason"].as_str().unwrap().is_empty());

    let body = sentences("b", "Alpha happened. Beta followed. Gamma ended.");
    let claims: HashSet<String> = ["c1".to_string()].into();
    let item = |n: usize| {
        format!(
            r#"{{"id":"i{n}","text":"Conclusion {n}.","premise_claim_ids":["c1"],"body_evidence_ids":["b1"],"support":"supported","why":"Because.","limitations":[]}}"#
        )
    };
    let five = (1..=5).map(item).collect::<Vec<_>>().join(",");
    assert!(validate_inferences(
        &format!(r#"{{"inferences":[{five}]}}"#),
        &claims,
        &body,
        &[]
    )
    .is_ok());
    let six = (1..=6).map(item).collect::<Vec<_>>().join(",");
    assert!(
        validate_inferences(&format!(r#"{{"inferences":[{six}]}}"#), &claims, &body, &[]).is_err()
    );
    assert!(validate_inferences(r#"{"inferences":[{"text":"x","premise_claim_ids":["c9"],"body_evidence_ids":["b1"],"support":"supported","why":"y"}]}"#, &claims, &body, &[]).is_err());
    assert!(validate_inferences(
        r#"{"inferences":[{"text":"x","body_evidence_ids":[],"support":"supported","why":"y"}]}"#,
        &claims,
        &body,
        &[]
    )
    .is_err());
    assert!(validate_inferences(r#"{"inferences":[{"text":"x","body_evidence_ids":["b1"],"support":"conditional","why":"y","limitations":[]}]}"#, &claims, &body, &[]).is_err());
    assert!(validate_inferences(r#"{"inferences":[],"links":[]}"#, &claims, &body, &[]).is_err());

    let cands = shortlist(
        &article("r", "a", TITLE, DESC, "https://example.test/a1"),
        vec![article(
            "r",
            "b",
            "Other ferry news",
            "Detail.",
            "https://o.test/b",
        )],
    );
    let iids: HashSet<String> = ["i1".to_string()].into();
    assert!(validate_leads(r#"{"leads":[{"article_id":"r1","inference_id":"i1","metadata_evidence_ids":["r1.t1"],"why":"Overlap."}]}"#, &iids, &cands).is_ok());
    assert!(validate_leads(r#"{"leads":[{"article_id":"r7","inference_id":"i1","metadata_evidence_ids":["r7.t1"],"why":"Invented."}]}"#, &iids, &cands).is_err());
    assert!(validate_leads(r#"{"leads":[{"article_id":"r1","inference_id":"i1","metadata_evidence_ids":["b1"],"why":"Body evidence."}]}"#, &iids, &cands).is_err());
}

#[test]
fn sentence_spans_are_valid_utf8_byte_ranges() {
    let text = "Zoë said «non». Müller denied it! 東京で会議が開かれた。 End";
    for s in sentences("b", text) {
        assert_eq!(&text[s.start_byte..s.end_byte], s.text);
    }
}

#[test]
fn shortlist_excludes_source_dedupes_syndication_and_caps_at_twelve() {
    let source = article("r", "a1", TITLE, DESC, "https://example.test/a1");
    let mut related = vec![
        source.clone(),
        article(
            "r",
            "dup",
            TITLE,
            "copy",
            "https://www.example.test/a1/?utm=x",
        ),
    ];
    for i in 0..20 {
        related.push(article(
            "r",
            &format!("x{i}"),
            &format!("Story {i}"),
            "d",
            &format!("https://o.test/{i}"),
        ));
        related.push(article(
            "r",
            &format!("y{i}"),
            &format!("Story {i}"),
            "syndicated",
            &format!("https://s.test/{i}"),
        ));
    }
    let list = shortlist(&source, related);
    assert_eq!(list.len(), MAX_CANDIDATES);
    assert!(list.iter().all(|c| c.article.id.starts_with('x')));
}

// Every successful analytical stage publishes independently; newer inputs win.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stage_four_publishes_snapshot_and_leads_and_new_inputs_replace_them() {
    let db = db();
    let job = ensure_chain(&Store::open(&db.path).unwrap().conn, &inputs("r1"), &now()).unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let mut partial = Mock::happy();
    partial.fail.insert("intel_brief_inferences".into());
    partial.fail.insert("intel_brief_links".into());
    drain(&db.path, "w1", &mut partial, &cancel, &cfg())
        .await
        .unwrap();
    {
        let store = Store::open(&db.path).unwrap();
        let (_, claims, inferences) = brief_pipeline::published_brief(&store.conn, "a1")
            .unwrap()
            .unwrap();
        assert_eq!(claims["claims"].as_array().unwrap().len(), 2);
        assert_eq!(inferences["inferences"].as_array().map_or(0, Vec::len), 0);
        brief_pipeline::retry_stage(&store.conn, &job, &now()).unwrap();
    }
    partial.fail.remove("intel_brief_inferences");
    drain(&db.path, "w1", &mut partial, &cancel, &cfg())
        .await
        .unwrap();
    {
        let store = Store::open(&db.path).unwrap();
        let (_, _, inferences) = brief_pipeline::published_brief(&store.conn, "a1")
            .unwrap()
            .unwrap();
        assert_eq!(inferences["inferences"].as_array().unwrap().len(), 1);
        assert!(brief_pipeline::published_leads(&store.conn, "a1")
            .unwrap()
            .is_empty());
        brief_pipeline::retry_stage(&store.conn, &job, &now()).unwrap();
    }
    let mut mock = Mock::happy();
    drain(&db.path, "w1", &mut mock, &cancel, &cfg())
        .await
        .unwrap();
    let store = Store::open(&db.path).unwrap();
    let (generation, claims, inferences) = brief_pipeline::published_brief(&store.conn, "a1")
        .unwrap()
        .unwrap();
    assert_eq!(generation, 1);
    assert_eq!(claims["claims"].as_array().unwrap().len(), 2);
    assert_eq!(inferences["inferences"].as_array().unwrap().len(), 1);
    let leads = brief_pipeline::published_leads(&store.conn, "a1").unwrap();
    assert_eq!(leads.len(), 1);
    assert_eq!(leads[0].candidate_article_id, "a2");
    assert!(leads[0].interpretation.contains("still needs verification"));
    assert!(leads[0].inference_text.contains("insurance lapse"));
    // Changed inputs: a new generation replaces the publication and its leads.
    ensure_chain(&store.conn, &inputs("r2"), &now()).unwrap();
    drop(store);
    let mut next = Mock::happy();
    next.related.clear();
    drain(&db.path, "w1", &mut next, &cancel, &cfg())
        .await
        .unwrap();
    let store = Store::open(&db.path).unwrap();
    assert_eq!(
        brief_pipeline::published_brief(&store.conn, "a1")
            .unwrap()
            .unwrap()
            .0,
        2
    );
    assert!(brief_pipeline::published_leads(&store.conn, "a1")
        .unwrap()
        .is_empty());
}

#[test]
fn input_revision_tracks_metadata_and_roles() {
    let a = article("r", "a1", TITLE, DESC, "u");
    let s = crate::provider::SettingsFile::default();
    let base = input_revision(&a, &s);
    assert_eq!(base, input_revision(&a, &s));
    let mut b = a.clone();
    b.description.push_str(" Update.");
    assert_ne!(base, input_revision(&b, &s));
}

#[test]
fn canonical_preparation_rejects_a_url_changed_after_the_captured_snapshot() {
    let db = db();
    let store = Store::open(&db.path).unwrap();
    let original = article_for(&store, "a1").unwrap();
    let mut input = inputs("source-fence");
    input.metadata = metadata_snapshot(&original);
    ensure_chain(&store.conn, &input, &now()).unwrap();
    let _body = claim_next(&db.path, "w1", &cfg()).unwrap().unwrap();
    let claims = claim_next(&db.path, "w2", &cfg()).unwrap().unwrap();
    assert_eq!(claims.operation, "intel_brief_claims");
    let mut changed = original;
    changed.url = "https://different.test/replaced-source".into();
    store.atlas_upsert_article(&changed).unwrap();
    let output: Value = serde_json::from_str(&claims_ok()).unwrap();
    let error = prepare_canonical(&store, &claims, &output).unwrap_err();
    assert!(error.contains("source revision changed"), "{error}");
}

// Fidelity: batched, scoped, cached; non-faithful items are withheld.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fidelity_batches_withhold_unfaithful_items_and_reuse_receipts() {
    let db = db();
    let job = ensure_chain(&Store::open(&db.path).unwrap().conn, &inputs("r1"), &now()).unwrap();
    let mut mock = Mock::happy();
    mock.responses.insert("intel_brief_claims".into(), vec![r#"{"claims":[{"id":"c1","text":"The operator said storms forced the suspension.","evidence_ids":["d1"]},{"id":"c2","text":"UNFAITHFUL unions admitted staffing caused it.","evidence_ids":["d2"]}]}"#.into()]);
    let cancel = Arc::new(AtomicBool::new(false));
    drain(&db.path, "w1", &mut mock, &cancel, &cfg())
        .await
        .unwrap();
    let store = Store::open(&db.path).unwrap();
    let claims = stage_output(&store, &job, "intel_brief_claims").unwrap();
    assert_eq!(claims["claims"].as_array().unwrap().len(), 1);
    assert_eq!(claims["withheld"], 1);
    assert_eq!(claims["claims"][0]["fidelity"], "faithful");
    let first = *mock.assessed.lock().unwrap();
    assert_eq!(first, 3, "2 claims + 1 inference");
    // Same items again: cached receipts, no new decisions.
    let items = vec![crate::intel_recon::brief_fidelity::FidelityItem {
        id: "c1".into(),
        kind: "claim",
        text: "The operator said storms forced the suspension.".into(),
        passages: vec!["The operator said storms forced the suspension.".into()],
    }];
    drop(store);
    let ctx = StageContext::default();
    let route = ctx.route_key(None, "evidence_curator");
    let labels = assess_cached(&db.path, &mut mock, &ctx, &route, items.clone(), &cancel)
        .await
        .unwrap();
    assert_eq!(labels["c1"], "faithful");
    assert_eq!(*mock.assessed.lock().unwrap(), first);
    // The receipt names the resolved route: another assessor re-decides.
    assess_cached(&db.path, &mut mock, &ctx, "other-route", items, &cancel)
        .await
        .unwrap();
    assert_eq!(*mock.assessed.lock().unwrap(), first + 1);
}

#[test]
fn fidelity_adapter_rejects_missing_extra_and_invalid_labels() {
    use crate::intel_recon::brief_fidelity::{FidelityDecisions, FidelityItem};
    use crate::recon::model_exec::DecisionsAdapter;
    let items = vec![FidelityItem {
        id: "c1".into(),
        kind: "claim",
        text: "x".into(),
        passages: vec!["x".into()],
    }];
    let a = FidelityDecisions { items: &items };
    assert!(a
        .parse_chat(r#"{"items":[{"id":"c1","fidelity":"faithful"}]}"#)
        .is_ok());
    assert!(a.parse_chat(r#"{"items":[]}"#).is_err());
    assert!(a
        .parse_chat(
            r#"{"items":[{"id":"c1","fidelity":"faithful"},{"id":"c9","fidelity":"faithful"}]}"#
        )
        .is_err());
    assert!(a
        .parse_chat(r#"{"items":[{"id":"c1","fidelity":"probably"}]}"#)
        .is_err());
}

// Reload goes through the chain: forced re-acquisition, later visits attach.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reload_supersedes_with_forced_body_and_visits_attach() {
    let db = db();
    let settings = crate::provider::SettingsFile::default();
    let store = Store::open(&db.path).unwrap();
    let a = store.atlas_article("fixture", "a1").unwrap().unwrap();
    let first = ensure_intel_brief_pipeline(&store, &a, &settings).unwrap();
    let reload = reload_intel_brief_pipeline(&store, &a, &settings).unwrap();
    assert_ne!(first, reload);
    assert_eq!(
        ensure_intel_brief_pipeline(&store, &a, &settings).unwrap(),
        reload
    );
    drop(store);
    let mut mock = Mock::happy();
    let cancel = Arc::new(AtomicBool::new(false));
    drain(&db.path, "w1", &mut mock, &cancel, &cfg())
        .await
        .unwrap();
    assert!(mock.forced);
    assert_eq!(mock.prepared, 1, "superseded chain never ran");
}

// §8.5: SQLite contention and publication delay. A competing writer holds the
// write lock repeatedly while stages run and commit; heartbeat renewal retries
// within its bound, no stage is reaped, and every stage commits exactly once.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sqlite_contention_and_publication_delay_keep_leases_and_commit_once() {
    let db = db();
    let job = ensure_chain(&Store::open(&db.path).unwrap().conn, &inputs("r1"), &now()).unwrap();
    let mut mock = Mock::happy();
    mock.delay = Some(Duration::from_millis(1200));
    let path = db.path.clone();
    let stop = Arc::new(AtomicBool::new(false));
    let s = stop.clone();
    let writer = std::thread::spawn(move || {
        let store = Store::open(&path).unwrap();
        let mut held = 0;
        while !s.load(Ordering::Acquire) {
            store.conn.execute_batch("BEGIN IMMEDIATE").unwrap();
            std::thread::sleep(Duration::from_millis(900));
            store.conn.execute_batch("COMMIT").unwrap();
            held += 1;
            std::thread::sleep(Duration::from_millis(150));
        }
        held
    });
    let cancel = Arc::new(AtomicBool::new(false));
    let config = ExecConfig {
        lease_secs: 6,
        heartbeat: Duration::from_millis(250),
    };
    let out = drain(&db.path, "w1", &mut mock, &cancel, &config)
        .await
        .unwrap();
    stop.store(true, Ordering::Release);
    assert!(writer.join().unwrap() >= 2, "contention actually happened");
    assert_eq!(out.len(), 4, "{out:?}");
    assert!(out
        .iter()
        .all(|o| matches!(o, StageOutcome::Committed { .. })));
    assert_eq!(
        mock.requests().len(),
        4,
        "no repeated successful stage calls"
    );
    let store = Store::open(&db.path).unwrap();
    let recoveries: i64 = store
        .conn
        .query_row(
            "SELECT COALESCE(SUM(recoveries),0) FROM argos_tasks WHERE job_id=?1",
            [&job],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(recoveries, 0);
    assert!(rows(&db.path, &job)
        .iter()
        .all(|r| r.starts_with("Completed")));
}

// §8.5: two real drainers race over one chain; each stage runs exactly once.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn overlapping_live_drainers_execute_each_stage_once() {
    let db = db();
    let job = ensure_chain(&Store::open(&db.path).unwrap().conn, &inputs("r1"), &now()).unwrap();
    let mut a = Mock::happy();
    let mut b = Mock::happy();
    a.delay = Some(Duration::from_millis(150));
    b.delay = Some(Duration::from_millis(150));
    let cancel = Arc::new(AtomicBool::new(false));
    let config = cfg();
    let mut committed = Vec::new();
    // Keep both drainers polling until the chain completes (an idle drainer
    // returns while the other holds the single active stage).
    for _ in 0..40 {
        let (x, y) = tokio::join!(
            drain(&db.path, "wa", &mut a, &cancel, &config),
            drain(&db.path, "wb", &mut b, &cancel, &config)
        );
        for o in x.unwrap().into_iter().chain(y.unwrap()) {
            if let StageOutcome::Committed { operation, .. } = o {
                committed.push(operation);
            }
        }
        if committed.len() == 4 {
            break;
        }
    }
    committed.sort();
    assert_eq!(
        committed,
        [
            "intel_brief_body",
            "intel_brief_claims",
            "intel_brief_inferences",
            "intel_brief_links"
        ]
    );
    assert_eq!(a.requests().len() + b.requests().len(), 4);
    assert_eq!(a.prepared + b.prepared, 1);
    assert!(rows(&db.path, &job)
        .iter()
        .all(|r| r.starts_with("Completed")));
}

// §8.7: a lead must share an inference-specific term with its cited metadata
// and name a verification gap; shared country alone yields no lead.
#[test]
fn shared_country_alone_yields_no_lead_and_explanations_name_the_gap() {
    let source = article("r", "a1", TITLE, DESC, "https://example.test/a1");
    let cands = shortlist(
        &source,
        vec![
            article(
                "r",
                "geo",
                "Lithuania weather update",
                "Rain across Lithuania.",
                "https://g.test/1",
            ),
            article(
                "r",
                "ins",
                "Ferry insurance lapsed, regulator says",
                "Certificate expired.",
                "https://i.test/2",
            ),
        ],
    );
    let texts = HashMap::from([(
        "i1".to_string(),
        "The Lithuania suspension may reflect an insurance lapse.".to_string(),
    )]);
    let iids: HashSet<String> = texts.keys().cloned().collect();
    let raw = r#"{"leads":[{"article_id":"r1","inference_id":"i1","metadata_evidence_ids":["r1.t1"],"why":"Both concern Lithuania.","verification_needed":"Check relevance."},{"article_id":"r2","inference_id":"i1","metadata_evidence_ids":["r2.t1"],"why":"Reports the same insurance lapse; causal link unverified.","verification_needed":"Confirm the lapse date."}]}"#;
    let mut out = validate_leads(raw, &iids, &cands).unwrap();
    filter_leads(&mut out, &texts, &cands);
    let kept = out["leads"].as_array().unwrap();
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0]["article_id"], "ins");
    assert_eq!(out["withheld"], 1);
    let raw2 = r#"{"leads":[{"article_id":"r1","inference_id":"i1","metadata_evidence_ids":["r1.t1"],"why":"Both concern Lithuania.","verification_needed":"Check relevance."},{"article_id":"r2","inference_id":"i1","metadata_evidence_ids":["r2.t1"],"why":"Reports the same insurance lapse; causal link unverified.","verification_needed":""}]}"#;
    let mut gapless = validate_leads(raw2, &iids, &cands).unwrap();
    filter_leads(&mut gapless, &texts, &cands);
    assert!(
        gapless["leads"].as_array().unwrap().is_empty(),
        "country-only and gapless leads dropped"
    );
    assert_eq!(gapless["withheld"], 2);
    assert_eq!(gapless["empty_reason"], "no inference-specific overlap");
}

// §2/§7 timing: every stage records a counts-only timing row and Logs event.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_stage_records_timing_aggregates_without_text() {
    let db = db();
    let job = {
        let store = Store::open(&db.path).unwrap();
        ensure_chain(&store.conn, &inputs("r1"), &now()).unwrap()
    };
    let mut mock = Mock::happy();
    mock.delay = Some(Duration::from_millis(60));
    let cancel = Arc::new(AtomicBool::new(false));
    drain(&db.path, "w1", &mut mock, &cancel, &cfg())
        .await
        .unwrap();
    let store = Store::open(&db.path).unwrap();
    let timings = crate::intel_recon::brief_timing::for_job(&store.conn, &job).unwrap();
    let ops: Vec<&str> = timings.iter().map(|t| t.operation.as_str()).collect();
    assert_eq!(
        ops,
        [
            "intel_brief_body",
            "intel_brief_claims",
            "intel_brief_inferences",
            "intel_brief_links"
        ]
    );
    for t in &timings {
        assert_eq!(t.outcome, "committed");
        assert!(t.total_ms >= t.local_ms + t.fidelity_ms, "{t:?}");
        if t.operation != "intel_brief_body" {
            assert!(t.generation_ms >= 60, "model wall time measured: {t:?}");
            assert!(t.total_ms >= t.generation_ms, "{t:?}");
        }
        assert!(t.queue_ms >= 0 && t.persistence_ms >= 0);
    }
    let messages: Vec<String> = store
        .conn
        .prepare(
            "SELECT message FROM argos_events WHERE event_type='intel_brief.timing' AND job_id=?1",
        )
        .unwrap()
        .query_map([&job], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(messages.len(), 4);
    for m in &messages {
        assert!(m.contains("total ") && m.contains("generation "), "{m}");
        for secret_or_text in ["Unions denied", "insurance", "harbour", TITLE] {
            assert!(!m.contains(secret_or_text), "timing log leaked text: {m}");
        }
    }
}

// §7: a Links-only route change reuses body, Claims and Inferences checkpoints.
#[test]
fn route_changes_move_only_their_own_node_revision() {
    let a = article("r", "a1", TITLE, DESC, "u");
    let mut s = crate::provider::SettingsFile::default();
    let before = stage_revisions(&a, &s);
    s.defaults.operation_overrides.insert(
        crate::provider::OP_LINKS_GENERATE.into(),
        crate::provider::ModelAssignment {
            provider: "openrouter".into(),
            model: "openai/gpt-4o-mini".into(),
            ..Default::default()
        }
        .into(),
    );
    let after = stage_revisions(&a, &s);
    assert_eq!(before[..3], after[..3]);
    assert_ne!(before[3], after[3]);
    // A Claims change moves every later stage.
    s.defaults.operation_overrides.insert(
        crate::provider::OP_CLAIMS_GENERATE.into(),
        crate::provider::ModelAssignment {
            provider: "openrouter".into(),
            model: "openai/gpt-4o".into(),
            ..Default::default()
        }
        .into(),
    );
    let claims = stage_revisions(&a, &s);
    // Node revisions are independent: descendants pick up a Claims change
    // through the prerequisite output hash bound at the join, not here.
    assert_eq!(claims[0], after[0]);
    assert_ne!(claims[1], after[1]);
    assert_eq!(claims[2..], after[2..]);

    // Context classification is routed independently from inference Synthesis.
    s.defaults.classifier.model = "context-classifier-fixture".into();
    let classifier = stage_revisions(&a, &s);
    assert_ne!(
        classifier[0], claims[0],
        "body cleanup already uses Classifier"
    );
    assert_eq!(
        classifier[1], claims[1],
        "claim extraction routing is stable"
    );
    assert_ne!(
        classifier[2], claims[2],
        "context decisions invalidate Inferences"
    );
    assert_eq!(classifier[3], claims[3]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn links_only_change_reuses_checkpoints_and_reruns_links() {
    let db = db();
    let mut first = inputs("r1");
    first.stage_revisions = vec!["b".into(), "c".into(), "i".into(), "l1".into()];
    ensure_chain(&Store::open(&db.path).unwrap().conn, &first, &now()).unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let mut mock = Mock::happy();
    drain(&db.path, "w1", &mut mock, &cancel, &cfg())
        .await
        .unwrap();
    let first_calls = mock.seen.lock().unwrap().len();
    assert!(first_calls >= 3);
    let mut second = inputs("r2");
    second.stage_revisions = vec!["b".into(), "c".into(), "i".into(), "l2".into()];
    let job = ensure_chain(&Store::open(&db.path).unwrap().conn, &second, &now()).unwrap();
    let mut next = Mock::happy();
    drain(&db.path, "w1", &mut next, &cancel, &cfg())
        .await
        .unwrap();
    let stages: Vec<String> = next
        .seen
        .lock()
        .unwrap()
        .iter()
        .map(|s| s.stage.clone())
        .collect();
    assert_eq!(stages, ["intel_brief_links"], "only Links dispatches");
    assert_eq!(next.prepared, 0, "body checkpoint reused");
    let store = Store::open(&db.path).unwrap();
    assert!(stage_output(&store, &job, "intel_brief_claims").is_ok());
    assert_eq!(
        brief_pipeline::published_brief(&store.conn, "a1")
            .unwrap()
            .unwrap()
            .0,
        2
    );
}

#[test]
fn request_windows_fit_the_serialized_request_budget() {
    let body: Vec<Sentence> = (0..400)
        .map(|i| Sentence {
            id: format!("b{i}"),
            text: format!(
                "Sentence {i} says “quoted” things about the ministry and its budget in detail."
            ),
            start_byte: 0,
            end_byte: 0,
        })
        .collect();
    let system = format!("{INFERENCES_PROMPT}{STRICT_JSON}");
    let claims = json!({"claims":[{"id":"c1","text":"A claim."}]});
    let budget = 12_000;
    let windows = request_windows(&body, &system, &claims, budget);
    assert!(windows.len() > 1);
    let flat: Vec<String> = windows.concat().into_iter().map(|s| s.id).collect();
    assert_eq!(
        flat,
        body.iter().map(|s| s.id.clone()).collect::<Vec<_>>(),
        "ordered, complete"
    );
    for w in &windows {
        let compiled = system.len() + inferences_user(&claims, w, &json!([])).len();
        assert!(compiled + CARRIED_RESERVE_BYTES <= budget, "{compiled}");
    }
}

#[test]
fn fidelity_modes_route_extraction_to_evidence_curator_and_support_to_assessor() {
    use crate::intel_recon::brief_fidelity::FidelityMode;
    let e = FidelityMode::for_kind("claim");
    let s = FidelityMode::for_kind("inference");
    assert_eq!(e.role(), "evidence_curator");
    assert_eq!(s.role(), "claim_assessor");
    assert_eq!(e.operation_key(), None);
    assert_eq!(
        s.operation_key(),
        Some(crate::provider::OP_INFERENCES_ASSESS)
    );
    let labels: Vec<String> = e.labels().into_iter().map(|(l, _)| l).collect();
    assert!(labels.iter().any(|l| l == "faithful"));
    assert_ne!(e.version(), s.version());
}

// Phase 4: the two-slot drainer runs both roots together. Each root parks on
// a two-party barrier, so a serial drainer would time out here.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn parallel_drainer_runs_both_roots_together_then_the_join() {
    let db = db();
    let job = ensure_chain(&Store::open(&db.path).unwrap().conn, &inputs("r1"), &now()).unwrap();
    let mut mock = Mock::happy();
    mock.barrier = Some(Arc::new(tokio::sync::Barrier::new(2)));
    let template = mock.clone();
    let cancel = Arc::new(AtomicBool::new(false));
    let out = tokio::time::timeout(
        Duration::from_secs(30),
        drain_parallel(
            &db.path,
            "w1",
            move || Box::new(template.clone()) as Box<dyn StageBackend + Send>,
            &cancel,
            &cfg(),
            STAGE_SLOTS,
        ),
    )
    .await
    .expect("roots must run concurrently")
    .unwrap();
    assert_eq!(out.len(), 4, "{out:?}");
    let stages: Vec<String> = mock.requests().into_iter().map(|r| r.stage).collect();
    let pos = |s: &str| stages.iter().position(|x| x == s).unwrap();
    assert!(pos("intel_brief_claims") < pos("intel_brief_inferences"));
    assert!(pos("intel_brief_inferences") < pos("intel_brief_links"));
    assert!(rows(&db.path, &job)
        .iter()
        .all(|r| r.starts_with("Completed")));
}

// A claim whose lease epoch was superseded cannot publish anything.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stale_epoch_worker_is_fenced_and_writes_nothing() {
    let db = db();
    let job = ensure_chain(&Store::open(&db.path).unwrap().conn, &inputs("r1"), &now()).unwrap();
    let claimed = claim_next(&db.path, "w1", &cfg()).unwrap().unwrap();
    let operation = claimed.operation.clone();
    Store::open(&db.path)
        .unwrap()
        .conn
        .execute(
            "UPDATE argos_tasks SET lease_epoch=lease_epoch+1,lease_owner='w2' WHERE id=?1",
            [&claimed.id],
        )
        .unwrap();
    let mut mock = Mock::happy();
    let cancel = Arc::new(AtomicBool::new(false));
    let out = run_claimed(&db.path, claimed, &mut mock, &cancel, &cfg())
        .await
        .unwrap();
    assert!(matches!(out, StageOutcome::Stale { .. }), "{out:?}");
    assert!(stage_output(&Store::open(&db.path).unwrap(), &job, &operation).is_err());
}

// Per-node reuse: a body-only reload re-runs the body and makes zero Claims
// requests; an unchanged visit after completion makes zero requests at all.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn body_reload_reuses_claims_and_unchanged_visit_makes_no_requests() {
    let db = db();
    let settings = crate::provider::SettingsFile::default();
    let a = article("fixture", "a1", TITLE, DESC, "https://example.test/a1");
    let store = Store::open(&db.path).unwrap();
    let first = ensure_intel_brief_pipeline(&store, &a, &settings).unwrap();
    let mut mock = Mock::happy();
    let cancel = Arc::new(AtomicBool::new(false));
    drain(&db.path, "w1", &mut mock, &cancel, &cfg())
        .await
        .unwrap();
    let before = mock.requests().len();
    assert!(before >= 3, "{:?}", rows(&db.path, &first));
    // Unchanged visit: same chain, nothing to do.
    assert_eq!(
        ensure_intel_brief_pipeline(&store, &a, &settings).unwrap(),
        first
    );
    drain(&db.path, "w1", &mut mock, &cancel, &cfg())
        .await
        .unwrap();
    assert_eq!(
        mock.requests().len(),
        before,
        "unchanged visit: zero requests"
    );
    // Body-only reload: a new generation, Claims adopted from the prior run.
    let reloaded = reload_intel_brief_pipeline(&store, &a, &settings).unwrap();
    assert_ne!(reloaded, first);
    drain(&db.path, "w1", &mut mock, &cancel, &cfg())
        .await
        .unwrap();
    let after: Vec<String> = mock.requests()[before..]
        .iter()
        .map(|r| r.stage.clone())
        .collect();
    assert!(
        !after.iter().any(|s| s == "intel_brief_claims"),
        "{after:?}"
    );
    assert!(mock.forced, "the reload forces a fresh body");
    assert!(rows(&db.path, &reloaded)
        .iter()
        .all(|r| r.starts_with("Completed")));
}

/// Spec §9 item 7: bounded live held-out comparison of the Claims stage under
/// the shipped routing versus the Brief fast preset. Opt-in only:
/// `ARGOS_HELDOUT_LIVE=1 OPENROUTER_API_KEY=… ARGOS_HELDOUT_OUT=…/results.json
///  cargo test … heldout_fast_preset_comparison -- --ignored`.
/// Both configurations use the same OpenRouter free models for fidelity, so
/// only the Claims generation route differs.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn heldout_fast_preset_comparison() {
    if std::env::var("ARGOS_HELDOUT_LIVE").ok().as_deref() != Some("1") {
        return;
    }
    let key = std::env::var("OPENROUTER_API_KEY").expect("OPENROUTER_API_KEY");
    let fixtures: Value =
        serde_json::from_str(include_str!("fixtures/brief_heldout.json")).unwrap();
    let mut auth = crate::secrets::AuthFile::default();
    auth.set_account(crate::secrets::ProviderSecret {
        kind: "openrouter".into(),
        model: "nvidia/nemotron-3-super-120b-a12b:free".into(),
        base_url: "https://openrouter.ai/api/v1".into(),
        api_key: Some(key),
        stt_model: None,
        device: None,
    });
    let assign = |model: &str| crate::provider::ModelAssignment {
        provider: "openrouter".into(),
        model: model.into(),
        ..Default::default()
    };
    let base = || {
        let mut s = crate::provider::SettingsFile::default();
        // Free-model stand-ins for the screenshot chains (J is not free).
        s.defaults.claim_assessor = assign("nvidia/nemotron-3-ultra-550b-a55b:free");
        s.defaults.synthesis = assign("nvidia/nemotron-3-ultra-550b-a55b:free");
        s.defaults.summarization = assign("nvidia/nemotron-3-super-120b-a12b:free");
        s.defaults.evidence_curator = assign("google/gemma-4-31b-it:free");
        s
    };
    let mut preset = base();
    preset.defaults.apply_fast_preset();
    let mut report = Vec::new();
    for (name, settings) in [("shipped", base()), ("fast_preset", preset)] {
        let mut rows = Vec::new();
        for a in fixtures["articles"].as_array().unwrap() {
            let id = a["id"].as_str().unwrap();
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("argos.db");
            let store = Store::open(&path).unwrap();
            store.conn.execute("INSERT OR IGNORE INTO atlas_runs(id,started_at,state) VALUES('fixture',?1,'completed')", [now()]).unwrap();
            let row = article(
                "fixture",
                id,
                a["title"].as_str().unwrap(),
                a["description"].as_str().unwrap(),
                &format!("https://heldout.test/{id}"),
            );
            store.atlas_upsert_article(&row).unwrap();
            let job = ensure_intel_brief_pipeline(&store, &row, &settings).unwrap();
            // Claim both roots; run Claims only (no body retrieval offline).
            let mut claimed = Vec::new();
            while let Some(c) = claim_next(&path, "heldout", &cfg()).unwrap() {
                claimed.push(c);
            }
            let claims = claimed
                .into_iter()
                .find(|c| c.operation == "intel_brief_claims")
                .expect("claims root claimable");
            let mut backend = RuntimeBackend {
                db_path: path.clone(),
                keys: Default::default(),
                auth: auth.clone(),
                settings: settings.clone(),
                user_agent: None,
            };
            let cancel = Arc::new(AtomicBool::new(false));
            let started = std::time::Instant::now();
            let outcome = run_claimed(&path, claims, &mut backend, &cancel, &cfg()).await;
            let ms = started.elapsed().as_millis() as i64;
            let (requests, prompt, completion): (i64, i64, i64) = store
                .conn
                .query_row(
                    "SELECT COUNT(*),COALESCE(SUM(prompt_tokens),0),COALESCE(SUM(completion_tokens),0) FROM recon_model_attempts WHERE dispatched=1",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .unwrap();
            let out = stage_output(&store, &job, "intel_brief_claims").ok();
            let kept: Vec<String> = out
                .as_ref()
                .and_then(|o| o["claims"].as_array().cloned())
                .unwrap_or_default()
                .iter()
                .map(|c| c["text"].as_str().unwrap_or("").to_lowercase())
                .collect();
            let withheld = out
                .as_ref()
                .and_then(|o| o["withheld"].as_i64())
                .unwrap_or(0);
            let joined = kept.join(" | ");
            let n = kept.len() as i64;
            let pass = out.is_some()
                && n >= a["min_claims"].as_i64().unwrap()
                && n <= a["max_claims"].as_i64().unwrap()
                && a["must_mention"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|m| joined.contains(m.as_str().unwrap()));
            rows.push(json!({
                "id": id, "kind": a["kind"], "ok": matches!(outcome, Ok(StageOutcome::Committed { .. })),
                "error": match &outcome { Ok(StageOutcome::Failed { reason, .. }) => reason.clone(), Err(e) => e.to_string(), _ => String::new() },
                "pass": pass, "kept": n, "withheld": withheld, "ms": ms,
                "requests": requests, "prompt_tokens": prompt, "completion_tokens": completion,
                "claims": kept,
            }));
            // Warm: the unchanged visit attaches and issues zero requests.
            assert_eq!(
                ensure_intel_brief_pipeline(&store, &row, &settings).unwrap(),
                job
            );
        }
        let mut ms: Vec<i64> = rows.iter().map(|r| r["ms"].as_i64().unwrap()).collect();
        ms.sort();
        let pct = |p: f64| ms[((ms.len() as f64 - 1.0) * p).round() as usize];
        let sum = |k: &str| rows.iter().map(|r| r[k].as_i64().unwrap()).sum::<i64>();
        report.push(json!({
            "config": name,
            "claims_route": settings.defaults.resolve_operation(Some(crate::provider::OP_CLAIMS_GENERATE), "claim_assessor").0.model,
            "pass": rows.iter().filter(|r| r["pass"] == true).count(),
            "committed": rows.iter().filter(|r| r["ok"] == true).count(),
            "articles": rows.len(),
            "p50_ms": pct(0.5), "p95_ms": pct(0.95),
            "requests": sum("requests"), "prompt_tokens": sum("prompt_tokens"),
            "completion_tokens": sum("completion_tokens"),
            "kept": sum("kept"), "withheld": sum("withheld"),
            "rows": rows,
        }));
    }
    let text = serde_json::to_string_pretty(&report).unwrap();
    if let Ok(out) = std::env::var("ARGOS_HELDOUT_OUT") {
        std::fs::write(out, &text).unwrap();
    }
    println!("{text}");
}

// Phase A: the stage's error category is the executor's real cause, not a
// blanket `invalid_result`; Jobs names the task of every attempt row.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn timeout_failure_keeps_its_category_and_attempt_rows_name_their_task() {
    use crate::provider_diag::Category;
    use crate::tasks::ErrorCategory as E;
    assert_eq!(stage_error_category(&anyhow!("bad json")), E::InvalidResult);
    for (c, want) in [
        (Category::Timeout, E::Timeout),
        (Category::Server, E::TemporaryNetwork),
        (Category::RateLimited, E::RateLimit),
        (Category::Auth, E::AuthOrQuota),
        (Category::TokenLimit, E::ContextLimit),
        (Category::InvalidResult, E::InvalidResult),
    ] {
        let err: anyhow::Error = crate::recon::model_exec::ModelOperationError {
            category: Some(c),
            requests: 1,
            history: vec![],
            message: "x".into(),
        }
        .into();
        assert_eq!(stage_error_category(&err.context("stage")), want);
    }
    let db = db();
    let job = ensure_chain(&Store::open(&db.path).unwrap().conn, &inputs("r1"), &now()).unwrap();
    let mut mock = Mock::happy();
    mock.fail_timeout.insert("intel_brief_inferences".into());
    let cancel = Arc::new(AtomicBool::new(false));
    drain(&db.path, "w1", &mut mock, &cancel, &cfg())
        .await
        .unwrap();
    let store = Store::open(&db.path).unwrap();
    let (cat, msg): (String, String) = store
        .conn
        .query_row(
            "SELECT error_category,error_message FROM argos_tasks WHERE id=?1",
            [brief_pipeline::task_id(&job, "intel_brief_inferences")],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(cat, "timeout", "{msg}");
    let detail = crate::jobs_view::job_detail(&store.conn, &job, 10)
        .unwrap()
        .unwrap();
    assert_eq!(detail.job.error_category, "timeout");
    let labels: HashSet<&str> = detail
        .attempts
        .iter()
        .map(|a| a.task_label.as_str())
        .collect();
    assert_eq!(
        labels,
        HashSet::from(["Full article", "Claims", "Inferences"])
    );
    let failed = detail
        .attempts
        .iter()
        .find(|a| a.task_label == "Inferences")
        .unwrap();
    assert_eq!(
        (failed.outcome.as_str(), failed.error_category.as_str()),
        ("failed", "timeout")
    );
    assert!(
        detail.attempts.iter().all(|a| a.attempt_number == 1),
        "same #1, now disambiguated"
    );
}

fn inference_requests(mock: &Mock) -> Vec<Value> {
    mock.requests()
        .into_iter()
        .filter(|s| s.stage == "intel_brief_inferences" && !s.user.contains("INFERENCE_CANDIDATES"))
        .map(|s| serde_json::from_str::<Value>(&s.user).unwrap()["CLAIM"]["id"].clone())
        .collect()
}

// Phase D: one failing claim no longer fails Inferences. The analyzed claims
// publish, the stage completes with warnings, Links runs on them, and Retry
// re-asks only the failed claim before re-running Links.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn one_failed_claim_completes_with_warnings_and_retry_asks_only_that_claim() {
    let db = db();
    let job = ensure_chain(&Store::open(&db.path).unwrap().conn, &inputs("r1"), &now()).unwrap();
    let mut mock = Mock::happy();
    mock.filter_inference_evidence = true;
    mock.fail_claim.insert("c2".into());
    let cancel = Arc::new(AtomicBool::new(false));
    drain(&db.path, "w1", &mut mock, &cancel, &cfg())
        .await
        .unwrap();
    let r = rows(&db.path, &job);
    assert_eq!(r[2], "Completed with warnings · 1 item", "{r:?}");
    assert!(
        r[3].starts_with("Completed"),
        "Links degrades, not blocks: {r:?}"
    );
    let store = Store::open(&db.path).unwrap();
    let out = stage_output(&store, &job, "intel_brief_inferences").unwrap();
    assert_eq!(out["claim_failures"][0]["claim_id"], "c2");
    assert_eq!(out["claim_failures"][0]["category"], "timeout");
    assert_eq!(
        out["claim_analysis"][1]["analysis_failed"]["category"],
        "timeout"
    );
    assert_eq!(out["inferences"].as_array().unwrap().len(), 1);
    assert_eq!(inference_requests(&mock).len(), 2);
    let leads_before = mock
        .requests()
        .iter()
        .filter(|s| s.stage == "intel_brief_links")
        .count();
    assert_eq!(leads_before, 1);
    let ckpt: i64 = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM app_state WHERE key LIKE 'intel-brief-ckpt:%' AND key NOT LIKE '%:context:%'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(ckpt, 1, "the analyzed claim keeps its checkpoint");

    // Retry: only c2 is asked; Links reruns on the complete set.
    mock.fail_claim.clear();
    mock.seen.lock().unwrap().clear();
    mock.classified.lock().unwrap().clear();
    assert!(
        crate::intel_recon::brief_pipeline::retry_stage(&store.conn, &job, &now())
            .unwrap()
            .is_some()
    );
    let r = rows(&db.path, &job);
    assert!(r[2].starts_with("Queued"), "{r:?}");
    assert!(r[3].starts_with("Queued"), "{r:?}");
    drain(&db.path, "w1", &mut mock, &cancel, &cfg())
        .await
        .unwrap();
    assert_eq!(inference_requests(&mock), [json!("c2")]);
    assert!(
        mock.classified.lock().unwrap().is_empty(),
        "Retry reuses validated context batches as well as generated claims"
    );
    let retried = stage_output(&store, &job, "intel_brief_inferences").unwrap();
    assert_eq!(retried["analysis_requests"], 1);
    assert_eq!(retried["claim_analysis"][0]["analysis_reused"], true);
    assert_eq!(retried["claim_analysis"][0]["analysis_request_sent"], false);
    let r = rows(&db.path, &job);
    assert!(!r[2].contains("warning"), "{r:?}");
    assert!(r[3].starts_with("Completed"), "{r:?}");
    assert_eq!(
        mock.requests()
            .iter()
            .filter(|s| s.stage == "intel_brief_links")
            .count(),
        1
    );
    let ckpt: i64 = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM app_state WHERE key LIKE 'intel-brief-ckpt:%'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(ckpt, 0, "a clean commit clears the checkpoints");
}

// Every claim failing is still a failed stage and keeps the provider category.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_claim_failing_fails_inferences_with_the_provider_category() {
    let db = db();
    let job = ensure_chain(&Store::open(&db.path).unwrap().conn, &inputs("r1"), &now()).unwrap();
    let mut mock = Mock::happy();
    mock.fail_claim = ["c1", "c2"].map(String::from).into();
    let cancel = Arc::new(AtomicBool::new(false));
    drain(&db.path, "w1", &mut mock, &cancel, &cfg())
        .await
        .unwrap();
    let r = rows(&db.path, &job);
    assert!(r[2].starts_with("Failed"), "{r:?}");
    assert_eq!(r[3], "Queued · blocked by Inferences");
    let store = Store::open(&db.path).unwrap();
    let category: String = store
        .conn
        .query_row(
            "SELECT error_category FROM argos_tasks WHERE id=?1",
            [crate::intel_recon::brief_pipeline::task_id(
                &job,
                "intel_brief_inferences",
            )],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(category, "timeout");
}

// The Jobs panel's retry goes through the same resume path as the brief's
// Retry button, for failed branches and for completed-with-warnings stages.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn jobs_retry_resumes_failed_and_warning_brief_stages() {
    let db = db();
    let job = ensure_chain(&Store::open(&db.path).unwrap().conn, &inputs("r1"), &now()).unwrap();
    let mut mock = Mock::happy();
    mock.filter_inference_evidence = true;
    mock.fail_claim = ["c1", "c2"].map(String::from).into();
    let cancel = Arc::new(AtomicBool::new(false));
    drain(&db.path, "w1", &mut mock, &cancel, &cfg())
        .await
        .unwrap();
    let store = Store::open(&db.path).unwrap();
    let detail = crate::jobs_view::job_detail(&store.conn, &job, 10)
        .unwrap()
        .unwrap();
    assert_eq!(detail.retryable, 1, "the failed Inferences branch");
    mock.fail_claim = ["c2"].map(String::from).into();
    assert_eq!(store.retry_failed_tasks(&job).unwrap(), 1);
    drain(&db.path, "w1", &mut mock, &cancel, &cfg())
        .await
        .unwrap();
    assert!(rows(&db.path, &job)[2].starts_with("Completed with warnings"));
    let detail = crate::jobs_view::job_detail(&store.conn, &job, 10)
        .unwrap()
        .unwrap();
    assert_eq!(detail.retryable, 1, "the warning stage is retryable");
    assert_eq!(detail.warnings.len(), 1);
    mock.fail_claim.clear();
    mock.seen.lock().unwrap().clear();
    assert_eq!(store.retry_failed_tasks(&job).unwrap(), 1);
    drain(&db.path, "w1", &mut mock, &cancel, &cfg())
        .await
        .unwrap();
    assert_eq!(inference_requests(&mock), [json!("c2")]);
    let detail = crate::jobs_view::job_detail(&store.conn, &job, 10)
        .unwrap()
        .unwrap();
    assert_eq!((detail.retryable, detail.warnings.len()), (0, 0));
    assert_eq!(store.retry_failed_tasks(&job).unwrap(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_context_batch_is_isolated_and_retry_only_classifies_that_batch() {
    let db = db();
    let job = ensure_chain(&Store::open(&db.path).unwrap().conn, &inputs("r1"), &now()).unwrap();
    let mut mock = Mock::happy();
    let extra = (0..24)
        .map(|n| format!("Unions denied that staffing was a factor in the suspension of service number {n}.\n\n"))
        .collect::<String>();
    mock.body = Some(format!("{}\n\n{extra}", story()));
    let cancel = Arc::new(AtomicBool::new(false));
    // Commit the two independent roots before choosing one classifier batch.
    for _ in 0..2 {
        let outcome = run_next_stage(&db.path, "w1", &mut mock, &cancel, &cfg())
            .await
            .unwrap()
            .unwrap();
        assert!(
            matches!(outcome, StageOutcome::Committed { .. }),
            "{outcome:?}"
        );
    }
    let store = Store::open(&db.path).unwrap();
    let body = stage_output(&store, &job, "intel_brief_body").unwrap();
    let claims = stage_output(&store, &job, "intel_brief_claims").unwrap();
    let (_, index) = crate::intel_recon::sentence_index::body_sentences(&body);
    let context = crate::intel_recon::brief_context::claim_candidates(
        &index,
        body["markdown"].as_str().unwrap(),
        &claims["claims"][1],
        &[],
    )
    .unwrap();
    let batches = crate::intel_recon::brief_context::classifier_batches(&context).unwrap();
    assert!(
        batches.len() >= 2,
        "fixture exercises partial classification"
    );
    let failed_id = batches.last().unwrap().candidates[0].id.clone();
    mock.fail_context_claim.insert("c2".into());
    mock.fail_context_sentence.insert(failed_id.clone());
    drain(&db.path, "w1", &mut mock, &cancel, &cfg())
        .await
        .unwrap();
    let output = stage_output(&store, &job, "intel_brief_inferences").unwrap();
    assert_eq!(output["claim_failures"][0]["claim_id"], "c2");
    assert_eq!(output["claim_failures"][0]["phase"], "context");
    assert_eq!(output["claim_failures"][0]["category"], "timeout");
    assert_eq!(output["claim_analysis"][1]["analysis_request_sent"], false);
    assert_eq!(inference_requests(&mock), [json!("c1")]);
    assert!(rows(&db.path, &job)[3].starts_with("Completed"));

    mock.fail_context_claim.clear();
    mock.classified.lock().unwrap().clear();
    mock.seen.lock().unwrap().clear();
    brief_pipeline::retry_stage(&store.conn, &job, &now())
        .unwrap()
        .unwrap();
    drain(&db.path, "w1", &mut mock, &cancel, &cfg())
        .await
        .unwrap();
    let classified = mock.classified.lock().unwrap();
    assert_eq!(classified.len(), 1, "successful batches are durable");
    assert_eq!(classified[0].claim["id"], "c2");
    assert!(classified[0].candidates.iter().any(|s| s.id == failed_id));
    assert_eq!(inference_requests(&mock), [json!("c2")]);
    assert!(!rows(&db.path, &job)[2].contains("warning"));
    assert!(rows(&db.path, &job)[3].starts_with("Completed"));
}
