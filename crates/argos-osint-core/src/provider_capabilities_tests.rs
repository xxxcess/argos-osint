use super::*;
use serde_json::json;

#[test]
fn digest_ignores_freshness_and_provenance() {
    let ev = CatalogEvidence {
        supported_parameters: vec!["response_format".into(), "tools".into()],
        ..CatalogEvidence::default()
    };
    let a = resolve_with(
        AdapterId::OpenRouter,
        "https://openrouter.ai/api/v1",
        "x/y",
        Some((&ev, 100)),
    );
    let b = resolve_with(
        AdapterId::OpenRouter,
        "https://openrouter.ai/api/v1",
        "x/y",
        Some((&ev, 999_999)),
    );
    assert_eq!(a.digest(), b.digest());
    assert_ne!(a.provenance, b.provenance);
    let mut ev2 = ev.clone();
    ev2.supported_parameters.push("structured_outputs".into());
    let c = resolve_with(
        AdapterId::OpenRouter,
        "https://openrouter.ai/api/v1",
        "x/y",
        Some((&ev2, 100)),
    );
    assert_ne!(
        a.digest(),
        c.digest(),
        "a semantic change changes the digest"
    );
}

#[test]
fn openrouter_endpoint_metadata_intersects_eligible_free_endpoints() {
    let v = json!({"data":{"endpoints":[
        {"provider_name":"A","pricing":{"prompt":"0","completion":"0"},"supported_parameters":["tools","tool_choice","max_tokens"],"max_completion_tokens":8192},
        {"provider_name":"B","pricing":{"prompt":"0","completion":"0"},"supported_parameters":["tools","max_tokens","response_format"],"max_completion_tokens":4096},
        {"provider_name":"Paid","pricing":{"prompt":"0.1","completion":"0.2"},"supported_parameters":["response_format","structured_outputs"]}
    ]}});
    let ev = parse_openrouter_endpoints("m/x:free", &v).unwrap();
    assert_eq!(ev.supported_parameters, vec!["max_tokens", "tools"]);
    assert_eq!(ev.output_token_limit, Some(4096));
    let caps = resolve_with(
        AdapterId::OpenRouter,
        "https://openrouter.ai/api/v1",
        "m/x:free",
        Some((&ev, 0)),
    );
    assert_eq!(caps.json_mode, Support::Unsupported);
    assert_eq!(caps.tools, Support::Supported);
    assert_eq!(caps.forced_tool, Support::Unsupported);
    assert_eq!(caps.max_output_tokens, Some(4096));
}

#[test]
fn google_catalog_methods_gate_generation() {
    let v = json!({"name":"models/embed-x","supportedGenerationMethods":["embedContent"],"outputTokenLimit":1});
    let ev = parse_google_model(&v).unwrap();
    let caps = resolve_with(
        AdapterId::Google,
        "https://generativelanguage.googleapis.com/v1beta/openai",
        "models/embed-x",
        Some((&ev, 0)),
    );
    assert_eq!(caps.text_generation, Support::Unsupported);
    assert!(
        crate::provider_adapters::compile(
            &crate::secrets::ProviderSecret {
                kind: "google".into(),
                base_url: "https://generativelanguage.googleapis.com/v1beta/openai".into(),
                model: "models/embed-x".into(),
                api_key: None,
                stt_model: None,
                device: None,
            },
            &crate::provider_adapters::OperationRequest {
                messages: &[],
                tools: &[],
                contract: &crate::provider_adapters::ResponseContract::Prose,
                policy: &Default::default(),
                stream: false,
            },
            &caps,
        )
        .is_err(),
        "non-text target is a clear capability skip"
    );
}

#[test]
fn observed_rejection_beats_profile_claim_and_is_value_scoped() {
    let base = "https://nv-observed.test/v1";
    let model = "nvidia/nemotron-3-ultra-550b-a55b";
    learn_rejection(AdapterId::Nvidia, base, model, "reasoning_effort", "high");
    let caps = resolve(AdapterId::Nvidia, base, model);
    assert!(caps.is_rejected("reasoning_effort", "high"));
    assert!(!caps.is_rejected("reasoning_effort", "medium"));
    assert!(caps.provenance.sources.contains(&"observed".to_string()));
}

#[test]
fn nvidia_profiles_are_exact_not_family_inferred() {
    let base = "https://integrate.api.nvidia.com/v1";
    let known = resolve(AdapterId::Nvidia, base, "nvidia/nemotron-3-super-120b-a12b");
    assert_eq!(known.reasoning.values, vec!["none", "low", "high"]);
    let sibling = resolve(
        AdapterId::Nvidia,
        base,
        "nvidia/nemotron-3-super-120b-a12b-v2",
    );
    assert_eq!(sibling.reasoning.control, ReasoningControl::None);
}

#[test]
fn cache_round_trips_through_app_state_and_coalesces_refreshes() {
    let dir = tempfile::tempdir().unwrap();
    let store = crate::store::Store::open(&dir.path().join("argos.db")).unwrap();
    let key = cache_key(
        AdapterId::OpenRouter,
        "https://openrouter.ai/api/v1/",
        ApiDialect::Chat,
        "a/b",
        None,
    );
    assert_eq!(
        key,
        cache_key(
            AdapterId::OpenRouter,
            "https://OPENROUTER.ai/api/v1",
            ApiDialect::Chat,
            "a/b",
            None
        )
    );
    let entry = CachedEvidence {
        evidence: CatalogEvidence {
            supported_parameters: vec!["tools".into()],
            ..CatalogEvidence::default()
        },
        fetched_at: 1_000,
    };
    store_cached(Some(&store), &key, &entry).unwrap();
    forget_memory_cache();
    assert_eq!(cached(Some(&store), &key), Some(entry.clone()));
    assert!(entry.is_fresh(1_000 + FRESHNESS_SECS - 1));
    assert!(!entry.is_fresh(1_000 + FRESHNESS_SECS));
    assert!(begin_refresh(&key));
    assert!(!begin_refresh(&key), "second request coalesces");
    end_refresh(&key);
    forget_memory_cache();
}

#[test]
fn admission_snapshot_is_immune_to_a_later_refresh_but_takes_observed_rejections() {
    let base = "https://snapshot.example/api/v1";
    let secret = crate::secrets::ProviderSecret {
        kind: "openrouter".into(),
        model: "snap/model:free".into(),
        base_url: base.into(),
        api_key: Some("fixture-only".into()),
        stt_model: None,
        device: None,
    };
    let snapshot = admit(&secret, None);
    let before = snapshot.digest();
    // A background refresh lands mid-operation.
    let key = cache_key(
        AdapterId::OpenRouter,
        base,
        ApiDialect::Chat,
        "snap/model:free",
        None,
    );
    store_cached(
        None,
        &key,
        &CachedEvidence {
            evidence: CatalogEvidence {
                supported_parameters: vec!["response_format".into(), "structured_outputs".into()],
                ..CatalogEvidence::default()
            },
            fetched_at: 2_000,
        },
    )
    .unwrap();
    assert_eq!(snapshot.digest(), before, "in-flight snapshot unchanged");
    assert_ne!(
        admit(&secret, None).digest(),
        before,
        "next admission sees it"
    );
    // Observed rejections still narrow the snapshot for recovery.
    learn_rejection(
        AdapterId::OpenRouter,
        base,
        "snap/model:free",
        "response_format",
        "json_object",
    );
    let narrowed = with_observed(snapshot, base, "snap/model:free");
    assert!(narrowed.is_rejected("response_format", "json_object"));
}
