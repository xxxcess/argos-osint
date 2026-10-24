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

/// Evidence memories (Brain Actor/Link/Related records are structural).
const EVIDENCE_MEMORIES: &str = "SELECT COUNT(*) FROM memories m WHERE NOT EXISTS (SELECT 1 FROM memory_metadata t WHERE t.memory_id=m.id AND t.memory_kind IN ('actor','link','related'))";

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
        let slice = insight_persist::run_refresh_slice(path, id, auth, settings, &cancel).await;
        // Flaky on loaded hosts ("article refresh lease lost before dispatch"):
        // show the lease state that caused it.
        let slice = slice.unwrap_or_else(|e| {
            panic!(
                "refresh slice for {id} failed: {e:#}{}",
                crate::test_diag::dump_task_state(path)
            )
        });
        if let Some(output) = slice {
            return output;
        }
    }
    panic!("refresh did not settle in bounded slices")
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
