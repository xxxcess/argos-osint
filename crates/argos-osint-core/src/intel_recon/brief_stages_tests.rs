use super::*;

#[cfg(feature = "embedded")]
#[path = "brief_stages_surreal_tests.rs"]
mod surreal;
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

// §8.1: one chain, ordered dispatch, scoped inputs, zero requests when cached.

// §8.4: Stage 3 failure isolates; Retry resumes Stage 3 only.

// §8.4: empty arrays complete; no-inference Stage 4 uses no model.

// Stage 1 admission: incomplete acquisition fails and blocks dependents.

// §8.5: cancellation drops the in-flight request and writes nothing.

// §8.5: a parent cancel also cancels the in-flight request.

// §8.5: processing longer than the lease survives because the heartbeat
// renews through commit; concurrent reapers never reap it.

// §8.5: stale late output after a reclaim is fenced; provider attempts survive.

// Article length does not multiply analysis requests or send unrelated prose.

// §8.2 validation.

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

// Fidelity: batched, scoped, cached; non-faithful items are withheld.

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

// §8.5: SQLite contention and publication delay. A competing writer holds the
// write lock repeatedly while stages run and commit; heartbeat renewal retries
// within its bound, no stage is reaped, and every stage commits exactly once.

// §8.5: two real drainers race over one chain; each stage runs exactly once.

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

// A claim whose lease epoch was superseded cannot publish anything.

// Per-node reuse: a body-only reload re-runs the body and makes zero Claims
// requests; an unchanged visit after completion makes zero requests at all.

// Phase A: the stage's error category is the executor's real cause, not a
// blanket `invalid_result`; Jobs names the task of every attempt row.

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

// Every claim failing is still a failed stage and keeps the provider category.

// The Jobs panel's retry goes through the same resume path as the brief's
// Retry button, for failed branches and for completed-with-warnings stages.

// A chain whose Body and Claims failed leaves Inferences queued with a bound
// input at the head of the pool's order. A fresh chain whose predecessors
// committed must still claim its own Inferences stage behind that head.

// The failure and its recovery reach the stage rows: a failed stage and every
// stage it blocks name the error and Retry; a missing key adds the Providers
// hint; healthy queued stages and unrelated failures carry nothing extra.

// Through the REAL drainer loop with an injected backend, the whole chain
// completes without the test driving any stage by hand.

// End to end on a COPY of a database seeded with the stale rows a long-lived
// install accumulates (job past the 3-day window, cancel requested, retired,
// failed predecessors): the real drainer loop must still run a fresh chain.
