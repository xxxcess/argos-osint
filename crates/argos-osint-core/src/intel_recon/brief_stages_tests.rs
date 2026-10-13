use super::*;
use crate::intel_recon::brief_pipeline::{ensure_chain, stage_views, BriefInputs};
use crate::intel_recon::clean_body::clean_article;
use std::sync::Mutex;

const TITLE: &str = "Ferry operator suspends Baltic route";
const DESC: &str =
    "The operator said storms forced the suspension. Unions denied that staffing was a factor.";

fn story() -> String {
    format!(
        "{TITLE}\n\nBy Zoë Kowalski\n\n{}\n\nThe harbour master confirmed on Monday that the ferry's insurance certificate expired last week, a detail absent from the headline.\n\n{}",
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
    }
}

#[derive(Clone, Debug)]
struct Seen {
    stage: String,
    user: String,
    system: String,
    tokens: u32,
}

#[derive(Default)]
struct Mock {
    body: Option<String>,
    complete: bool,
    prepared: usize,
    seen: Arc<Mutex<Vec<Seen>>>,
    responses: HashMap<String, Vec<String>>,
    fail: HashSet<String>,
    hang: HashSet<String>,
    delay: Option<Duration>,
    related: Vec<AtlasArticleRow>,
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
        _a: &'a AtlasArticleRow,
        _c: &'a Arc<AtomicBool>,
    ) -> BoxFut<'a, Result<CleanArtifact>> {
        Box::pin(async move {
            self.prepared += 1;
            let raw = self.body.clone().ok_or_else(|| anyhow!("story missing"))?;
            let mut artifact =
                clean_article(&raw, TITLE, DESC, "https://example.test/a1", &[], None).await;
            artifact.acquisition_complete = self.complete;
            Ok(artifact)
        })
    }
    fn related(&mut self, _a: &AtlasArticleRow) -> Result<Vec<AtlasArticleRow>> {
        Ok(self.related.clone())
    }
    fn generate<'a>(
        &'a mut self,
        request: StageRequest,
        cancel: &'a Arc<AtomicBool>,
    ) -> BoxFut<'a, Result<String>> {
        Box::pin(async move {
            self.seen.lock().unwrap().push(Seen {
                stage: request.stage.clone(),
                user: request.user.clone(),
                system: request.system.clone(),
                tokens: request.completion_tokens,
            });
            if self.hang.contains(&request.stage) {
                loop {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    let _ = cancel.load(Ordering::Acquire);
                }
            }
            if let Some(delay) = self.delay {
                tokio::time::sleep(delay).await;
            }
            if self.fail.contains(&request.stage) {
                bail!("provider exhausted: server error");
            }
            let queue = self
                .responses
                .get_mut(&request.stage)
                .ok_or_else(|| anyhow!("no response"))?;
            Ok(if queue.len() > 1 {
                queue.remove(0)
            } else {
                queue[0].clone()
            })
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
            "intel_brief_links"
        ]
    );
    assert_eq!(
        seen.iter().map(|s| s.tokens).collect::<Vec<_>>(),
        [2048, 3072, 1536]
    );
    // Claims: title/description only.
    assert!(seen[0].user.contains("Unions denied"));
    assert!(!seen[0].user.contains("harbour master"));
    assert!(!seen[0].user.contains("Baltic ferry insurance lapsed"));
    // Inferences: exact clean artifact sentences + claim anchors; no related metadata.
    let store = Store::open(&db.path).unwrap();
    let body = stage_output(&store, &job, "intel_brief_body").unwrap();
    for s in sentences("b", body["markdown"].as_str().unwrap()) {
        assert!(seen[1]
            .user
            .contains(&serde_json::to_string(&s.text).unwrap()[1..s.text.len().min(40)]));
    }
    assert!(seen[1].user.contains("\"c1\"") && !seen[1].user.contains("regulator says"));
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
    assert!(!seen[2].user.contains("harbour master"));
    assert!(
        seen[2].user.contains("r1")
            && !seen[2].user.contains("r2")
            && !seen[2].user.contains("Syndicated")
    );
    assert!(seen[2].system.starts_with(LINKS_PROMPT));
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
    assert_eq!(after, ["intel_brief_inferences", "intel_brief_links"]);
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

// Stage 1 admission: incomplete acquisition fails and blocks dependents.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn incomplete_acquisition_fails_stage_one_and_blocks_downstream() {
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
    assert!(mock.requests().is_empty());
    assert_eq!(rows(&db.path, &job)[1], "Queued · blocked by Full article");
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
    let canceller = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(400));
        let store = Store::open(&path).unwrap();
        brief_pipeline::cancel_chain(&store.conn, &j, &now()).unwrap();
    });
    let cancel = Arc::new(AtomicBool::new(false));
    let out = tokio::time::timeout(
        Duration::from_secs(10),
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
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
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

// Long bodies: ordered windows inside Stage 3, later windows revise, five total.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn long_body_is_fully_covered_by_ordered_windows() {
    let db = db();
    let job = ensure_chain(&Store::open(&db.path).unwrap().conn, &inputs("r1"), &now()).unwrap();
    let mut mock = Mock::happy();
    let mut long = story();
    for i in 0..500 {
        long.push_str(&format!("\n\nParagraph {i}: inspectors recorded hull measurements at berth {i} and filed their notes with the port authority before noon."));
    }
    long.push_str("\n\nFinal note: the operator later corrected its statement about the storm.");
    mock.body = Some(long);
    let one = r#"{"inferences":[{"id":"i1","text":"Inspection is extensive.","premise_claim_ids":["c1"],"body_evidence_ids":["b1"],"support":"supported","why":"Many berths were inspected.","limitations":[]}]}"#;
    mock.responses
        .insert("intel_brief_inferences".into(), vec![one.into()]);
    let cancel = Arc::new(AtomicBool::new(false));
    drain(&db.path, "w1", &mut mock, &cancel, &cfg())
        .await
        .unwrap();
    let windows: Vec<Seen> = mock
        .requests()
        .into_iter()
        .filter(|s| s.stage == "intel_brief_inferences")
        .collect();
    assert!(windows.len() >= 2);
    assert!(!windows[0].user.contains("PRIOR_WINDOW_PROPOSALS"));
    assert!(windows[1..]
        .iter()
        .all(|w| w.user.contains("PRIOR_WINDOW_PROPOSALS")));
    assert!(windows
        .last()
        .unwrap()
        .user
        .contains("corrected its statement"));
    let store = Store::open(&db.path).unwrap();
    let inf = stage_output(&store, &job, "intel_brief_inferences").unwrap();
    assert_eq!(inf["windows"].as_u64().unwrap() as usize, windows.len());
    assert!(inf["inferences"].as_array().unwrap().len() <= MAX_INFERENCES);
}

// §8.2 validation.
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
