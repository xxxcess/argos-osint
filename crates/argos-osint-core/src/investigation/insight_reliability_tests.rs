//! Offline wire/executor/publication fixtures for insights-v3.
use super::{insight_execution, insight_persist, insights::*};
use crate::{
    nlp::Span,
    provider::{ModelAssignment, SettingsFile},
    provider_attempt::mock,
    secrets::{AuthFile, ProviderSecret},
    store::Store,
};
use rusqlite::params;
use serde_json::{json, Value};
use std::sync::{atomic::AtomicBool, Arc};

fn print_attempt_timings(store: &Store, task_id: &str) {
    let mut statement = store
        .conn
        .prepare(
            "SELECT COALESCE(c.stage,'unknown'),a.queue_ms,a.elapsed_ms FROM recon_model_attempts a
             JOIN recon_model_operations o ON o.id=a.operation_id
             LEFT JOIN insight_stage_checkpoints c ON a.operation_id LIKE c.operation_key || ':retry:%'
             WHERE o.task_id=?1 ORDER BY a.started_at,a.id",
        )
        .unwrap();
    for row in statement
        .query_map([task_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })
        .unwrap()
    {
        let (stage, queue_ms, elapsed_ms) = row.unwrap();
        eprintln!("fixture attempt {stage}: queue_ms={queue_ms} request_ms={elapsed_ms}");
    }
}

fn routes(base: &str) -> (AuthFile, SettingsFile) {
    let mut auth = AuthFile::default();
    auth.set_account(ProviderSecret {
        kind: "local".into(),
        model: "fixture".into(),
        base_url: base.into(),
        api_key: Some("fixture-only".into()),
        stt_model: None,
        device: None,
    });
    let mut settings = SettingsFile::default();
    settings.nlp.mode = crate::nlp::Mode::Off;
    settings.defaults.synthesis = ModelAssignment {
        provider: "local".into(),
        model: "fixture".into(),
        ..Default::default()
    };
    settings.defaults.claim_assessor = settings.defaults.synthesis.clone();
    (auth, settings)
}

fn seed_report(store: &Store, owner: &str, text: &str) -> PipelineInput {
    let now = chrono::Utc::now().to_rfc3339();
    store.conn.execute("INSERT OR IGNORE INTO atlas_runs(id,started_at,state) VALUES('fixture',?1,'completed')",[&now]).unwrap();
    store.conn.execute("INSERT INTO atlas_articles(run_id,article_id,title,description,url,country,source_name,source_domain,published_at,provider,temperature,seen_at) VALUES('fixture',?1,'Acme report','','https://example.test/report','','','','','fixture',0,?2)",params![owner,now]).unwrap();
    let bundle = json!({"text":text,"completed_at":now,"support_receipts":[]}).to_string();
    store.conn.execute("INSERT INTO intel_report_results(job_id,generation,article_id,completion_sequence,source_hash,details_json,completed_at) VALUES(?1,1,?2,1,?3,?4,?5)",params![format!("report-{owner}"),owner,crate::nlp::cache::hash(&bundle),bundle,now]).unwrap();
    let generation =
        insight_persist::advance_generation(&store.conn, "article", owner, &now).unwrap();
    insight_persist::report_input(&store.conn, owner, generation)
        .unwrap()
        .unwrap()
}

fn claims(input: &PipelineInput, n: usize) -> (EntityCandidate, Vec<SourceClaim>) {
    let evidence = &input.evidence[0];
    let entity = EntityCandidate {
        id: format!("{}:entity:acme", evidence.reference.source_id),
        type_hint: "org".into(),
        comparison_key: "acme".into(),
        original_forms: vec!["Acme".into()],
        mention_refs: vec![Span {
            start_byte: 0,
            end_byte: 4,
        }],
    };
    let claims = (0..n)
        .map(|index| SourceClaim {
            id: format!("c{index}"),
            subject_ref: entity.id.clone(),
            predicate: "announced".into(),
            object: format!("project {index}"),
            text: format!("Acme announced project {index}."),
            polarity: "positive".into(),
            modality: "asserted".into(),
            attributed_to: "Acme".into(),
            temporal_scope: "2026".into(),
            evidence_refs: vec![evidence.reference.clone()],
            extraction_scope: "complete source window".into(),
            assessment: Assessment::default(),
            support_receipts: vec![],
        })
        .collect();
    (entity, claims)
}

fn answers(n: usize, claim: bool, label: &str) -> String {
    let mut answers = serde_json::Map::new();
    for index in 0..n {
        answers.insert(format!("q{index}:publication"), json!({"label":label}));
        if claim {
            answers.insert(
                format!("q{index}:claim_relation"),
                json!({"label":"mentions_only"}),
            );
        }
    }
    json!({"answers":answers}).to_string()
}

async fn finish(
    path: &std::path::Path,
    id: &str,
    auth: &AuthFile,
    settings: &SettingsFile,
) -> PipelineOutput {
    let cancel = Arc::new(AtomicBool::new(false));
    for _ in 0..64 {
        if let Some(output) = insight_persist::run_refresh_slice(path, id, auth, settings, &cancel)
            .await
            .unwrap()
        {
            return output;
        }
    }
    panic!("refresh did not settle in bounded slices")
}

#[tokio::test]
async fn overlapping_windows_dedup_before_assessment_preserve_late_denial_and_bound_old_oversized_merge(
) {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("windows.db");
    let store = Store::open(&path).unwrap();
    let text = format!(
        "{}\n\nCorrection: Acme denied project 8.",
        "Acme announced projects 0 through 7; announcements do not establish completion.\n\n"
            .repeat(820)
    );
    let mut input = seed_report(&store, "windows", &text);
    let spans = evidence_windows(&text, WINDOW_BYTES / 4).unwrap();
    assert!(spans.len() > 2);
    let (entity, mut claims) = claims(&input, 8);
    let first_ref = EvidenceRef {
        location: EvidenceLocation::Range(spans[0]),
        ..input.evidence[0].reference.clone()
    };
    for claim in &mut claims {
        claim.evidence_refs = vec![first_ref.clone()];
    }
    let mut correction = claims[0].clone();
    correction.id = "late-correction".into();
    correction.object = "project 8".into();
    correction.text = "Acme denied project 8.".into();
    correction.polarity = "negative".into();
    correction.evidence_refs = vec![EvidenceRef {
        location: EvidenceLocation::Range(*spans.last().unwrap()),
        ..input.evidence[0].reference.clone()
    }];
    let premise_id = format!("claim:{}", claim_fingerprint(&claims[1]));
    let proposal = json!({"id":"proposal","text":"Acme announced an intention, not completion.","entity_refs":[entity.id],"premise_claim_ids":[premise_id],"evidence_refs":[first_ref],"rationale":"Original announcement only.","limitations":["Later correction denies project 8."]});
    let synthesis = json!({"inferences":[proposal],"links":[]}).to_string();
    let mut script = Vec::new();
    for (index, span) in spans.iter().enumerate() {
        let mut extracted = if index == 0 { claims.clone() } else { vec![] };
        if index == 1 {
            let mut duplicate = claims[1].clone();
            duplicate.evidence_refs = vec![EvidenceRef {
                location: EvidenceLocation::Range(*span),
                ..input.evidence[0].reference.clone()
            }];
            extracted.push(duplicate);
        }
        if index == spans.len() - 1 {
            extracted.push(correction.clone());
        }
        script.push(mock::ok_json(&json!({"entities":if index==0 {vec![entity.clone()]} else {vec![]},"claims":extracted,"limit_reached":false,"partial_enumeration":false}).to_string(),"stop"));
    }
    script.push(mock::ok_json(&answers(8, true, "faithful"), "stop"));
    for _ in 0..spans.len() * 2 {
        script.push(mock::ok_json(&synthesis, "stop"));
    }
    script.push(mock::ok_json(&answers(1, false, "faithful"), "stop"));
    let server = mock::serve(script).await;
    let (auth, settings) = routes(&server.base_url);
    let id = insight_persist::enqueue_refresh(
        &store.conn,
        &mut input,
        &settings,
        &chrono::Utc::now().to_rfc3339(),
    )
    .unwrap();
    let output = finish(&path, &id, &auth, &settings).await;
    assert_eq!(output.claims.len(), 8);
    assert!(output.limit_reached && output.partial_enumeration);
    assert!(output
        .claims
        .iter()
        .any(|c| c.polarity == "negative" && c.text.contains("denied project 8")));
    assert_eq!(output.inferences.len(), 1);
    assert!(output.readiness.publication_eligible);
    let snapshot: String = store
        .conn
        .query_row(
            "SELECT inputs_json FROM insight_snapshots ORDER BY created_at DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let snapshot: EvidenceSnapshot = serde_json::from_str(&snapshot).unwrap();
    assert!(snapshot
        .coverage_by_stage
        .iter()
        .any(|c| c.stage == "claim_assessor" && c.covers(&input.evidence[0])));
    assert!(snapshot
        .coverage_by_stage
        .iter()
        .any(|c| c.stage == "synthesis" && c.covers(&input.evidence[0])));
    let requests = server.requests.lock().unwrap();
    let bytes: Vec<_> = requests
        .iter()
        .map(|r| r.split_once("\r\n\r\n").unwrap().1.len())
        .collect();
    assert!(bytes.iter().all(|b| *b <= REQUEST_BYTES));
    eprintln!(
        "multi-window: windows={} requests={} wire_bytes={bytes:?}",
        spans.len(),
        server.hits()
    );
    print_attempt_timings(&store, &id);
}

#[tokio::test]
async fn report_eight_five_eight_is_five_requests_atomic_publication_and_zero_request_replay() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("fixture.db");
    let store = Store::open(&path).unwrap();
    let text = (0..8)
        .map(|i| format!("Acme announced project {i}. "))
        .collect::<String>();
    let mut input = seed_report(&store, "owner", &text);
    let (entity, claims) = claims(&input, 8);
    let premise_ids: Vec<_> = claims
        .iter()
        .map(|c| format!("claim:{}", claim_fingerprint(c)))
        .collect();
    let inferences:Vec<_>=(0..5).map(|i|json!({"id":format!("i{i}"),"text":format!("Project {i} was announced, not completed."),"entity_refs":[entity.id],"premise_claim_ids":[premise_ids[i]],"evidence_refs":[input.evidence[0].reference],"rationale":"An announcement establishes stated intent only.","limitations":["Completion is unknown."]})).collect();
    let links:Vec<_>=(0..8).map(|i|json!({"id":format!("l{i}"),"typed_endpoints":[{"kind":"entity","id":entity.id},{"kind":"claim","id":premise_ids[i]}],"relation":"entity_connection","evidence_refs":[input.evidence[0].reference],"rationale":format!("Acme is the attributed speaker for project {i}.")})).collect();
    let server=mock::serve(vec![mock::ok_json(&json!({"entities":[entity],"claims":claims,"limit_reached":false,"partial_enumeration":false}).to_string(),"stop"),mock::ok_json(&answers(8,true,"faithful"),"stop"),mock::ok_json(&json!({"inferences":inferences,"links":links}).to_string(),"stop"),mock::ok_json(&answers(8,false,"faithful"),"stop"),mock::ok_json(&answers(5,false,"faithful"),"stop")]).await;
    let (auth, settings) = routes(&server.base_url);
    let id = insight_persist::enqueue_refresh(
        &store.conn,
        &mut input,
        &settings,
        &chrono::Utc::now().to_rfc3339(),
    )
    .unwrap();
    let replay = input.clone();
    let started = std::time::Instant::now();
    let output = finish(&path, &id, &auth, &settings).await;
    assert_eq!(
        (
            output.claims.len(),
            output.inferences.len(),
            output.links.len()
        ),
        (8, 5, 8)
    );
    assert!(
        output.claims.iter().all(|c| c.support_receipts.is_empty()),
        "report cannot corroborate itself"
    );
    assert_eq!(server.hits(), 5);
    assert_eq!(
        store
            .conn
            .query_row("SELECT COUNT(*) FROM memories", [], |r| r
                .get::<_, usize>(0))
            .unwrap(),
        13
    );
    assert_eq!(
        store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM insight_snapshots WHERE owner_id='owner'",
                [],
                |r| r.get::<_, usize>(0)
            )
            .unwrap(),
        1
    );
    assert_eq!(
        store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM argos_index_changes WHERE record_kind='memory'",
                [],
                |r| r.get::<_, usize>(0)
            )
            .unwrap(),
        13
    );
    let progress = insight_execution::task_progress(&store.conn, &id)
        .unwrap()
        .unwrap();
    assert_eq!(progress.stage, "published");
    assert!(progress.index_status.contains("pending"));
    let bytes: Vec<_> = server
        .requests
        .lock()
        .unwrap()
        .iter()
        .map(|request| request.split_once("\r\n\r\n").unwrap().1.len())
        .collect();
    assert!(bytes.iter().all(|b| *b <= REQUEST_BYTES));
    eprintln!(
        "report 8/5/8 cold: requests=5 wire_bytes={bytes:?} elapsed_ms={}",
        started.elapsed().as_millis()
    );
    print_attempt_timings(&store, &id);
    let mut replay = replay;
    let started = std::time::Instant::now();
    let warm = run_pipeline(
        &mut replay,
        &auth,
        &settings,
        &Arc::new(AtomicBool::new(false)),
        &path,
    )
    .await
    .unwrap_err();
    // Production dispatch requires a live task; warm checkpoints are reviewed via
    // a newly leased continuation without allowing any provider dispatch.
    assert!(warm.to_string().contains("lease lost"));
    store
        .conn
        .execute(
            "UPDATE argos_tasks SET state='queued',max_attempts=attempts+1 WHERE id=?1",
            [&id],
        )
        .unwrap();
    let warm = finish(&path, &id, &auth, &settings).await;
    assert_eq!(warm.links.len(), 8);
    assert_eq!(server.hits(), 5);
    eprintln!(
        "report warm: requests=0 elapsed_ms={}",
        started.elapsed().as_millis()
    );
    let count = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM recon_model_operations WHERE task_id=?1",
            [&id],
            |r| r.get::<_, usize>(0),
        )
        .unwrap();
    assert_eq!(count, 5, "all model operations linked to owning task");
}

#[tokio::test]
async fn invalid_output_repairs_once_then_uses_declared_fallback_and_legacy_validation_stays_terminal(
) {
    use crate::recon::model_exec::{execute_chat, ModelOperationError, OperationScope};
    let server = mock::serve(vec![
        mock::ok_json("not json", "stop"),
        mock::ok_json("still not json", "stop"),
        mock::ok_json("{}", "stop"),
    ])
    .await;
    let (auth, mut settings) = routes(&server.base_url);
    settings
        .defaults
        .synthesis
        .fallbacks
        .push(crate::provider::ModelRoute {
            provider: "local".into(),
            model: "fallback".into(),
            ..Default::default()
        });
    let messages = vec![crate::provider::ChatMessage {
        role: "user".into(),
        content: "Return {}".into(),
        tool_call_id: None,
        tool_calls: vec![],
    }];
    let cancel = Arc::new(AtomicBool::new(false));
    let (_, report) = execute_chat(
        &auth,
        &settings,
        &OperationScope::new("insight-fixture", "synthesis").with_insight_contract(),
        &messages,
        |raw| {
            serde_json::from_str::<Value>(raw)
                .map(|_| ())
                .map_err(|e| e.to_string())
        },
        &cancel,
        None,
        None,
        None,
        false,
    )
    .await
    .unwrap();
    assert_eq!(report.requests, 3);
    assert_eq!(report.effective_route, Some(1));
    assert!(
        server.requests.lock().unwrap()[1].contains("Repair the previous output contract failure")
    );
    let error = execute_chat(
        &auth,
        &settings,
        &OperationScope::new("ordinary-fixture", "synthesis"),
        &messages,
        |_| Err("bad local contract".into()),
        &cancel,
        None,
        None,
        None,
        false,
    )
    .await
    .unwrap_err();
    assert_eq!(
        error
            .downcast_ref::<ModelOperationError>()
            .unwrap()
            .requests,
        1
    );
    assert!(error.to_string().contains("stopped"));
}

#[test]
fn compact_evidence_rejects_unknown_ids_bad_utf8_and_out_of_window_spans() {
    let store = Store::memory().unwrap();
    let input = seed_report(&store, "wire", "Acme café.");
    let reference = &input.evidence[0].reference;
    let mut wire = json!({"evidence_refs":[reference]});
    let mut dictionary = std::collections::BTreeMap::new();
    super::insight_wire::compact(&mut wire, &mut dictionary).unwrap();
    let mut bad = wire.clone();
    bad["evidence_refs"][0]["evidence_id"] = json!("unknown");
    assert!(super::insight_wire::hydrate(bad, &dictionary, &input).is_err());
    let mut bad = wire.clone();
    bad["evidence_refs"][0]["location"] = serde_json::to_value(EvidenceLocation::Range(Span {
        start_byte: 0,
        end_byte: 9,
    }))
    .unwrap();
    assert!(super::insight_wire::hydrate(bad, &dictionary, &input).is_err());
    let restored = super::insight_wire::hydrate(wire, &dictionary, &input).unwrap();
    assert_eq!(restored["evidence_refs"][0], json!(reference));
}

#[tokio::test]
async fn partial_assessment_keeps_qualified_items_and_explicit_epoch_preserves_completed_work() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("partial.db");
    let store = Store::open(&path).unwrap();
    let mut input = seed_report(
        &store,
        "partial",
        "Acme announced project 0. Acme announced project 1.",
    );
    let (entity, claims) = claims(&input, 2);
    let id0 = format!("claim:{}", claim_fingerprint(&claims[0]));
    let inference = json!({"id":"i","text":"Acme expressed an intention.","entity_refs":[entity.id],"premise_claim_ids":[id0],"evidence_refs":[input.evidence[0].reference],"rationale":"Announced intent is not completion.","limitations":[]});
    let server=mock::serve(vec![mock::ok_json(&json!({"entities":[entity],"claims":claims}).to_string(),"stop"),mock::ok_json(&json!({"answers":{"q0:publication":{"label":"faithful"},"q0:claim_relation":{"label":"mentions_only"},"q1:publication":{"label":"overstated"},"q1:claim_relation":{"label":"supports"}}}).to_string(),"stop"),mock::ok_json(&json!({"inferences":[inference],"links":[]}).to_string(),"stop"),mock::Reply::Json(503,"{}".into()),mock::Reply::Json(503,"{}".into()),mock::ok_json(&answers(1,false,"faithful"),"stop")]).await;
    let (auth, settings) = routes(&server.base_url);
    let id = insight_persist::enqueue_refresh(
        &store.conn,
        &mut input,
        &settings,
        &chrono::Utc::now().to_rfc3339(),
    )
    .unwrap();
    let output = finish(&path, &id, &auth, &settings).await;
    assert_eq!(output.withheld_count, 1);
    assert!(output.inferences.is_empty());
    assert_eq!(
        store
            .conn
            .query_row("SELECT COUNT(*) FROM memories", [], |r| r
                .get::<_, usize>(0))
            .unwrap(),
        1
    );
    assert!(
        !output.claims[1].assessment.faithful(),
        "Pass cannot authorize overstated label"
    );
    assert_eq!(server.hits(), 5);
    assert!(insight_execution::retry_extraction(&store.conn, &id).unwrap());
    assert!(
        !insight_execution::retry_extraction(&store.conn, &id).unwrap(),
        "queued epoch was admitted twice"
    );
    drop(store);
    let output = finish(&path, &id, &auth, &settings).await;
    assert_eq!(
        server.hits(),
        6,
        "retry repeated completed extraction or synthesis"
    );
    assert_eq!(output.inferences.len(), 1);
    let store = Store::open(&path).unwrap();
    assert_eq!(
        insight_execution::task_progress(&store.conn, &id)
            .unwrap()
            .unwrap()
            .retry_epoch,
        1
    );
    assert!(!insight_execution::retry_extraction(&store.conn, &id).unwrap());
}

#[test]
fn captured_routes_and_contract_admit_replacement_without_revisit_retry_loops() {
    let store = Store::memory().unwrap();
    let mut input = seed_report(&store, "routes", "Acme announced project 0.");
    let (_, mut settings) = routes("http://127.0.0.1:1");
    let id = insight_persist::enqueue_refresh(
        &store.conn,
        &mut input,
        &settings,
        &chrono::Utc::now().to_rfc3339(),
    )
    .unwrap();
    store
        .conn
        .execute("UPDATE argos_tasks SET state='failed' WHERE id=?1", [&id])
        .unwrap();
    let article = store.atlas_article("fixture", "routes").unwrap().unwrap();
    assert_eq!(
        insight_persist::schedule_article_refresh(&store, &article, &settings, true).unwrap(),
        Some(id.clone())
    );
    settings.defaults.recon.model = "unrelated-model".into();
    assert_eq!(
        insight_persist::schedule_article_refresh(&store, &article, &settings, true).unwrap(),
        Some(id.clone()),
        "unrelated route admitted work"
    );
    settings.defaults.synthesis.model = "replacement-model".into();
    let replacement = insight_persist::schedule_article_refresh(&store, &article, &settings, true)
        .unwrap()
        .unwrap();
    assert_ne!(
        replacement, id,
        "prior routes recomputed using today's settings"
    );
    assert!(
        !insight_execution::retry_extraction(&store.conn, &id).is_ok(),
        "superseded owner retried"
    );
    assert_eq!(
        insight_persist::schedule_article_refresh(&store, &article, &settings, true).unwrap(),
        Some(replacement.clone())
    );
    // A legacy input/contract fingerprint must not be recomputed as current.
    store
        .conn
        .execute(
            "UPDATE argos_tasks SET input_hash='legacy-contract-fingerprint' WHERE id=?1",
            [&replacement],
        )
        .unwrap();
    let upgraded = insight_persist::schedule_article_refresh(&store, &article, &settings, false)
        .unwrap()
        .unwrap();
    assert_ne!(upgraded, replacement);
}

#[test]
fn schema_32_upgrade_preserves_legacy_checkpoints_and_unrelated_tables() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("upgrade.db");
    let store = Store::open(&path).unwrap();
    store.conn.execute_batch("CREATE TABLE unrelated_probe(value TEXT); INSERT INTO unrelated_probe VALUES('kept'); INSERT INTO insight_stage_checkpoints(operation_key,owner_kind,owner_id,generation,stage,window_id,state,output_json,updated_at) VALUES('legacy','article','old',1,'extraction','0','completed','{}','old'); DROP TABLE insight_refresh_execution; PRAGMA user_version=31;").unwrap();
    drop(store);
    let store = Store::open(&path).unwrap();
    assert_eq!(
        store
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        crate::store::SCHEMA_VERSION
    );
    assert_eq!(
        store
            .conn
            .query_row("SELECT value FROM unrelated_probe", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        "kept"
    );
    assert_eq!(
        store
            .conn
            .query_row(
                "SELECT state FROM insight_stage_checkpoints WHERE operation_key='legacy'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "completed"
    );
}

#[tokio::test]
async fn recovery_continues_after_one_owner_fails_and_restart_does_not_revive_it() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("recovery.db");
    let store = Store::open(&path).unwrap();
    let mut failed = seed_report(&store, "failed", "Acme announced project 0.");
    let mut good = seed_report(&store, "good", "Acme announced project 0.");
    let (entity, claims) = claims(&good, 1);
    let server = mock::serve(vec![
        mock::ok_json("invalid", "stop"),
        mock::ok_json("invalid again", "stop"),
        mock::ok_json(
            &json!({"entities":[entity],"claims":claims}).to_string(),
            "stop",
        ),
        mock::ok_json(&answers(1, true, "faithful"), "stop"),
        mock::ok_json(r#"{"inferences":[],"links":[]}"#, "stop"),
    ])
    .await;
    let (auth, settings) = routes(&server.base_url);
    let bad_id = insight_persist::enqueue_refresh(
        &store.conn,
        &mut failed,
        &settings,
        &chrono::Utc::now().to_rfc3339(),
    )
    .unwrap();
    let good_id = insight_persist::enqueue_refresh(
        &store.conn,
        &mut good,
        &settings,
        &chrono::Utc::now().to_rfc3339(),
    )
    .unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    for _ in 0..3 {
        insight_persist::recover_refreshes(&path, &auth, &settings, &cancel)
            .await
            .unwrap();
    }
    assert_eq!(server.hits(), 5);
    assert_eq!(
        store
            .conn
            .query_row(
                "SELECT state FROM argos_tasks WHERE id=?1",
                [&bad_id],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "failed"
    );
    assert_eq!(
        store
            .conn
            .query_row(
                "SELECT state FROM argos_tasks WHERE id=?1",
                [&good_id],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "completed"
    );
    drop(store);
    insight_persist::recover_refreshes(&path, &auth, &settings, &cancel)
        .await
        .unwrap();
    assert_eq!(server.hits(), 5);
}

#[test]
fn batch_answers_require_exact_ids_and_original_threshold_profiles() {
    use super::decisions::{
        parse_general_model_response, parse_native_response, templates::template_insight_batch,
        DecisionAdapterKind, DecisionPolicy, ThresholdProfile,
    };
    let mut policy = DecisionPolicy::default().with_question_profile(
        "publication",
        ThresholdProfile {
            min_probability: Some(0.9),
            ..Default::default()
        },
    );
    let contract = template_insight_batch(&[("q0".into(), "claim".into())], &mut policy);
    let identity = template_insight_batch(&[("q0".into(), "identity".into())], &mut policy);
    assert_eq!(identity.role_id, "entity_resolver");
    assert_eq!(
        policy.question_profiles["q0:publication"].min_probability,
        Some(0.9)
    );
    for raw in [
        r#"{"answers":{"q0:publication":"faithful"}}"#,
        r#"{"answers":{"q0:publication":"faithful","q0:publication":"faithful","q0:claim_relation":"supports"}}"#,
        r#"{"answers":{"q0:publication":"faithful","q0:claim_relation":"supports","invented":"faithful"}}"#,
    ] {
        assert!(parse_general_model_response(
            raw,
            &contract,
            "fixture",
            DecisionAdapterKind::ValidatedText
        )
        .is_err());
    }
    let native=crate::provider::parse_decisions(r#"{"model":"typesafe/jev-fixture","answers":{"q0:publication":{"choice":"faithful","probabilities":{"faithful":0.6,"overstated":0.4}},"q0:claim_relation":{"choice":"supports","probabilities":{"supports":0.99,"insufficient":0.01}}}}"#).unwrap();
    let normalized = parse_native_response(&native, &contract).unwrap();
    assert!(!policy
        .evaluate("q0:publication", &normalized.answers["q0:publication"])
        .is_approved());
    assert!(policy
        .evaluate(
            "q0:claim_relation",
            &normalized.answers["q0:claim_relation"]
        )
        .is_approved());
}

#[test]
fn entity_resolver_routes_are_relevant_to_article_and_recon_but_not_report_only_work() {
    let store = Store::memory().unwrap();
    let mut input = seed_report(&store, "resolver", "Acme announced project 0.");
    let mut settings = SettingsFile::default();
    let report = input_key(&input, &settings);
    input.surface = "article".into();
    let article = input_key(&input, &settings);
    input.surface = "recon".into();
    let recon = input_key(&input, &settings);
    settings.defaults.entity_resolver = ModelAssignment {
        provider: "local".into(),
        model: "identity-route".into(),
        ..Default::default()
    };
    assert_ne!(recon, input_key(&input, &settings));
    input.surface = "article".into();
    assert_ne!(article, input_key(&input, &settings));
    input.surface = "recon_report".into();
    assert_eq!(report, input_key(&input, &settings));
}

#[test]
fn status_steps_follow_the_streamed_sequence_and_never_claim_activity_from_headers() {
    use insight_execution::status_step;
    assert_eq!(status_step("queued", "queued", "", None, false), "Queued");
    assert_eq!(
        status_step("running", "extraction", "", Some(5), false),
        "Waiting for output"
    );
    assert_eq!(
        status_step(
            "running",
            "extraction",
            "Waiting for output · attempt 1 · super",
            Some(3),
            false
        ),
        "Waiting for output"
    );
    assert_eq!(
        status_step(
            "running",
            "extraction",
            "Receiving output · attempt 1 · 812 chars",
            Some(1),
            false
        ),
        "Receiving output"
    );
    assert_eq!(
        status_step("running", "extraction", "Validating", Some(400), false),
        "Validating"
    );
    assert_eq!(
        status_step("running", "assessment", "", None, false),
        "Waiting for decision"
    );
    assert!(status_step(
        "running",
        "extraction",
        "Retry wait 12s · next attempt 2",
        Some(2),
        false
    )
    .starts_with("Retry wait 12s"));
    assert_eq!(
        status_step(
            "running",
            "extraction",
            "Receiving output · attempt 1",
            Some(95),
            false
        ),
        "Stalled · no model activity for 95s"
    );
    assert_eq!(
        status_step("running", "extraction", "", Some(150), false),
        "Waiting for output",
        "first output may take up to 180s"
    );
    assert_eq!(
        status_step("completed", "published", "", None, false),
        "Published"
    );
    assert_eq!(
        status_step("completed", "published", "", None, true),
        "Indexed"
    );
    assert_eq!(
        status_step("failed", "assessment", "", None, false),
        "Failed"
    );

    // Headers alone are "Waiting for output", never activity.
    use crate::provider_attempt::AttemptProgress as P;
    use crate::recon::model_exec::ModelExecEvent as E;
    let headers = E::Progress {
        operation_id: "op".into(),
        attempt: 1,
        route_index: 0,
        model: "super".into(),
        progress: P::HeadersReceived { status: 200 },
    };
    assert!(insight_execution::activity_label(&headers)
        .unwrap()
        .starts_with("Waiting for output"));
    assert!(insight_execution::activity_label(&E::Progress {
        operation_id: "op".into(),
        attempt: 1,
        route_index: 0,
        model: "super".into(),
        progress: P::Heartbeat,
    })
    .is_none());
}

#[test]
fn progress_reaches_jobs_and_logs_throttled_and_resets_per_attempt_and_stage() {
    use crate::provider_attempt::AttemptProgress as P;
    use crate::recon::model_exec::ModelExecEvent as E;
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("progress.db");
    let store = Store::open(&path).unwrap();
    let now = chrono::Utc::now().to_rfc3339();
    store.conn.execute("INSERT INTO argos_jobs(id,kind,owner_scope,state,created_at,updated_at) VALUES('job-p','insight_refresh','article:a1','running',?1,?1)",[&now]).unwrap();
    store.conn.execute("INSERT INTO argos_tasks(id,job_id,operation,state,created_at,updated_at) VALUES('task-p','job-p','insight_refresh','running',?1,?1)",[&now]).unwrap();
    store.conn.execute("INSERT INTO insight_refresh_execution(task_id,stage,updated_at) VALUES('task-p','extraction',?1)",[&now]).unwrap();
    let sink = activity_sink(&path, "task-p");
    let progress = |attempt, progress| E::Progress {
        operation_id: "op".into(),
        attempt,
        route_index: 2,
        model: "super".into(),
        progress,
    };
    let activity = || insight_execution::activity(&store.conn, "task-p");
    sink(E::AttemptStart {
        operation_id: "op".into(),
        generation: 1,
        route_index: 2,
        attempt: 1,
        provider: "local".into(),
        account: "local".into(),
        model: "super".into(),
        transport: "stream".into(),
    });
    assert!(activity().starts_with("Waiting for output · attempt 1"));
    sink(progress(1, P::HeadersReceived { status: 200 }));
    assert!(activity().starts_with("Waiting for output"));
    sink(progress(1, P::FirstModelOutput));
    sink(progress(
        1,
        P::Receiving {
            answer_chars: 40,
            reasoning_chars: 0,
        },
    ));
    // Within the 1s throttle the durable label is not rewritten per update.
    sink(progress(
        1,
        P::Receiving {
            answer_chars: 41,
            reasoning_chars: 0,
        },
    ));
    assert_eq!(activity(), "Receiving output · attempt 1 · super");
    sink(progress(1, P::Heartbeat));
    assert_eq!(activity(), "Receiving output · attempt 1 · super");
    // A new attempt resets provisional counters.
    sink(E::AttemptReset {
        operation_id: "op".into(),
        attempt: 2,
    });
    assert_eq!(activity(), "Waiting for output · attempt 2");
    sink(progress(
        2,
        P::StreamComplete {
            finish_reason: Some("stop".into()),
        },
    ));
    assert_eq!(activity(), "Validating");

    let events = crate::events::list_events(
        &store.conn,
        &crate::events::EventFilter {
            job_id: "job-p".into(),
            ..Default::default()
        },
        50,
    )
    .unwrap();
    let messages: Vec<_> = events.iter().map(|e| e.message.clone()).collect();
    assert!(
        messages.iter().any(|m| m.contains("attempt 1 dispatched")),
        "{messages:?}"
    );
    assert!(messages
        .iter()
        .any(|m| m.contains("receiving model output")));
    assert!(messages.iter().any(|m| m.contains("stream complete")));
    assert!(messages.len() <= 4, "no per-delta log rows: {messages:?}");

    // Jobs detail picks up the live step through task progress; a stage
    // change resets activity ("Waiting for decision" for assessment).
    store.conn.execute("UPDATE insight_refresh_execution SET stage='assessment',activity='Waiting for decision' WHERE task_id='task-p'",[]).unwrap();
    assert_eq!(
        insight_execution::status_step("running", "assessment", &activity(), Some(1), false),
        "Waiting for decision"
    );
}

#[test]
fn source_and_route_changes_retire_old_task_with_linked_reason_and_polling_never_duplicates() {
    let store = Store::memory().unwrap();
    let mut input = seed_report(&store, "supersede", "Acme announced project 0.");
    let (_, mut settings) = routes("http://127.0.0.1:1");
    let first = insight_persist::enqueue_refresh(
        &store.conn,
        &mut input,
        &settings,
        &chrono::Utc::now().to_rfc3339(),
    )
    .unwrap();
    let article = store
        .atlas_article("fixture", "supersede")
        .unwrap()
        .unwrap();
    // Repeated unchanged visits/polling: same task, no new generation.
    let generation =
        insight_persist::desired_generation(&store.conn, "article", "supersede").unwrap();
    for _ in 0..3 {
        assert_eq!(
            insight_persist::schedule_article_refresh(&store, &article, &settings, true).unwrap(),
            Some(first.clone())
        );
    }
    assert_eq!(
        insight_persist::desired_generation(&store.conn, "article", "supersede").unwrap(),
        generation
    );
    // Report body changes while the first task is still queued.
    let bundle = json!({"text":"Acme announced project 1 and later denied project 0.","completed_at":chrono::Utc::now().to_rfc3339(),"support_receipts":[]}).to_string();
    store.conn.execute("INSERT INTO intel_report_results(job_id,generation,article_id,completion_sequence,source_hash,details_json,completed_at) VALUES('report-supersede-2',1,'supersede',2,?1,?2,?3)",params![crate::nlp::cache::hash(&bundle),bundle,chrono::Utc::now().to_rfc3339()]).unwrap();
    let second = insight_persist::schedule_article_refresh(&store, &article, &settings, true)
        .unwrap()
        .unwrap();
    assert_ne!(second, first);
    let (state, message): (String, String) = store
        .conn
        .query_row(
            "SELECT state,error_message FROM argos_tasks WHERE id=?1",
            [&first],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(state, "superseded");
    let second_job: String = store
        .conn
        .query_row(
            "SELECT job_id FROM argos_tasks WHERE id=?1",
            [&second],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        message,
        format!("Superseded · replaced by {second_job} · source changed")
    );
    assert_eq!(
        insight_persist::supersession(&store.conn, &first),
        Some((second_job.clone(), "source changed".into()))
    );
    // Only the current generation may publish; the old one is fenced.
    assert!(insight_persist::is_superseded(&store.conn, &input));
    // The latest live generation stays queued (actionable), never failed.
    let live: String = store
        .conn
        .query_row(
            "SELECT state FROM argos_tasks WHERE id=?1",
            [&second],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(live, "queued");
    // Superseded progress reads as retirement, not Unknown/Timeout/exhaustion.
    let progress = insight_execution::task_progress(&store.conn, &first)
        .unwrap()
        .unwrap();
    let lines = progress.lines().join("\n");
    assert!(lines.starts_with("Superseded · replaced by"), "{lines}");
    assert!(!lines.contains("Extraction error"));
    // Route change: reason names the changed role.
    settings.defaults.synthesis.model = "replacement-model".into();
    let third = insight_persist::schedule_article_refresh(&store, &article, &settings, true)
        .unwrap()
        .unwrap();
    assert_ne!(third, second);
    assert_eq!(
        insight_persist::supersession(&store.conn, &second).map(|(_, r)| r),
        Some("model routes changed".into())
    );
    let events = crate::events::list_events(
        &store.conn,
        &crate::events::EventFilter {
            text: "Superseded".into(),
            ..Default::default()
        },
        10,
    )
    .unwrap();
    assert_eq!(events.len(), 2);
}

#[test]
fn completed_report_replacement_is_named_as_such() {
    let store = Store::memory().unwrap();
    let report = seed_report(&store, "rep", "Acme announced project 0.");
    let (_, settings) = routes("http://127.0.0.1:1");
    let mut article = report.clone();
    article.surface = "article".into();
    assert_eq!(
        insight_persist::supersession_reason(&article, &settings, &report, &settings).0,
        "completed report replaces article details"
    );
    assert_eq!(
        insight_persist::supersession_reason(&report, &settings, &report, &settings).0,
        "preparation contract changed"
    );
}

#[tokio::test]
async fn generation_advanced_mid_flight_retires_task_as_superseded_with_zero_requests() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("race.db");
    let store = Store::open(&path).unwrap();
    let mut input = seed_report(&store, "race", "Acme announced project 0.");
    let server = mock::serve(vec![]).await;
    let (auth, settings) = routes(&server.base_url);
    let id = insight_persist::enqueue_refresh(
        &store.conn,
        &mut input,
        &settings,
        &chrono::Utc::now().to_rfc3339(),
    )
    .unwrap();
    // A replacement generation lands between enqueue and dispatch.
    insight_persist::advance_generation(
        &store.conn,
        "article",
        "race",
        &chrono::Utc::now().to_rfc3339(),
    )
    .unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let out = insight_persist::run_refresh_slice(&path, &id, &auth, &settings, &cancel)
        .await
        .unwrap();
    assert!(out.is_none(), "late work never publishes");
    assert_eq!(server.hits(), 0, "obsolete work sends no requests");
    let (state, category, message): (String, String, String) = store
        .conn
        .query_row(
            "SELECT state,error_category,error_message FROM argos_tasks WHERE id=?1",
            [&id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(state, "superseded");
    assert_eq!(category, "", "not Unknown/Timeout");
    assert!(message.starts_with("Superseded"), "{message}");
    let lifecycle_error: String = store
        .conn
        .query_row(
            "SELECT last_error FROM insight_owner_state WHERE owner_kind='article' AND owner_id='race'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        lifecycle_error.is_empty(),
        "retirement is not an owner error"
    );
}

#[tokio::test]
async fn index_only_repair_after_insight_publication_makes_zero_model_calls() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("fixture.db");
    let store = Store::open(&path).unwrap();
    let text = (0..8)
        .map(|i| format!("Acme announced project {i}. "))
        .collect::<String>();
    let mut input = seed_report(&store, "owner", &text);
    let (entity, claims) = claims(&input, 8);
    let premise_ids: Vec<_> = claims
        .iter()
        .map(|c| format!("claim:{}", claim_fingerprint(c)))
        .collect();
    let inferences:Vec<_>=(0..5).map(|i|json!({"id":format!("i{i}"),"text":format!("Project {i} was announced, not completed."),"entity_refs":[entity.id],"premise_claim_ids":[premise_ids[i]],"evidence_refs":[input.evidence[0].reference],"rationale":"An announcement establishes stated intent only.","limitations":["Completion is unknown."]})).collect();
    let links:Vec<_>=(0..8).map(|i|json!({"id":format!("l{i}"),"typed_endpoints":[{"kind":"entity","id":entity.id},{"kind":"claim","id":premise_ids[i]}],"relation":"entity_connection","evidence_refs":[input.evidence[0].reference],"rationale":format!("Acme is the attributed speaker for project {i}.")})).collect();
    let server=mock::serve(vec![mock::ok_json(&json!({"entities":[entity],"claims":claims,"limit_reached":false,"partial_enumeration":false}).to_string(),"stop"),mock::ok_json(&answers(8,true,"faithful"),"stop"),mock::ok_json(&json!({"inferences":inferences,"links":links}).to_string(),"stop"),mock::ok_json(&answers(8,false,"faithful"),"stop"),mock::ok_json(&answers(5,false,"faithful"),"stop")]).await;
    let (auth, settings) = routes(&server.base_url);
    let id = insight_persist::enqueue_refresh(
        &store.conn,
        &mut input,
        &settings,
        &chrono::Utc::now().to_rfc3339(),
    )
    .unwrap();
    finish(&path, &id, &auth, &settings).await;
    assert_eq!(server.hits(), 5);
    let count = |sql: &str| {
        store
            .conn
            .query_row(sql, [], |r| r.get::<_, usize>(0))
            .unwrap()
    };
    let memories = count("SELECT COUNT(*) FROM memories");
    let generation = insight_persist::desired_generation(&store.conn, "article", "owner").unwrap();
    // Crash after publication: the index outbox is the only unfinished work.
    // Disabled vector index (ARGOS_EMBED=0 store) blocks honestly.
    let drained = crate::scheduler::drain_index_once(&store.conn, &store, "w", 64).unwrap();
    assert!(drained > 0);
    assert_eq!(
        count("SELECT COUNT(*) FROM argos_index_changes WHERE record_kind='memory' AND state<>'blocked'"),
        0
    );
    // Index-only repair: requeue the outbox and drain again; recovery and
    // revisits must not re-run extraction/assessment/synthesis.
    store
        .conn
        .execute(
            "UPDATE argos_index_changes SET state='pending' WHERE record_kind='memory'",
            [],
        )
        .unwrap();
    crate::scheduler::drain_index_once(&store.conn, &store, "w", 64).unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    insight_persist::recover_refreshes(&path, &auth, &settings, &cancel)
        .await
        .unwrap();
    let article = store.atlas_article("fixture", "owner").unwrap().unwrap();
    for _ in 0..2 {
        assert_eq!(
            insight_persist::schedule_article_refresh(&store, &article, &settings, true).unwrap(),
            Some(id.clone())
        );
    }
    assert_eq!(server.hits(), 5, "index-only repair sent model requests");
    assert_eq!(count("SELECT COUNT(*) FROM memories"), memories);
    assert_eq!(
        count("SELECT COUNT(*) FROM insight_snapshots WHERE owner_id='owner'"),
        1
    );
    assert_eq!(
        insight_persist::desired_generation(&store.conn, "article", "owner").unwrap(),
        generation
    );
    let state: String = store
        .conn
        .query_row("SELECT state FROM argos_tasks WHERE id=?1", [&id], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(state, "completed");
}
