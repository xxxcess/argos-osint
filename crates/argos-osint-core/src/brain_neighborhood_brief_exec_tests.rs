use super::*;
use crate::brain_neighborhood_brief::tests::{brief_of, input_of, rich};
use crate::provider::{ModelAssignment, SettingsFile};
use crate::provider_attempt::mock;
use crate::secrets::{AuthFile, ProviderSecret};
use crate::store::Store;
use crate::tasks;

fn runtime(base: &str, model: &str) -> RoleRuntime {
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
    settings.defaults.summarization = ModelAssignment {
        provider: "local".into(),
        model: model.into(),
        ..Default::default()
    };
    RoleRuntime::new(auth, settings)
}

fn good() -> String {
    serde_json::json!({
        "overview": [{"text": "This claim concerns a port closure and sits among related claims, an actor and a source.", "basis": ["N1"]}],
        "connections": [{"text": "The Harbor Authority is referenced by both claims, and a recorded link says the second claim contradicts the first.", "basis": ["N2", "E3"]}],
        "interpretation": [{"text": "A stored inference with support state supported reads the two claims together; it is a stored interpretation, not a verified conclusion.", "basis": ["N4"]}],
        "limits": [{"text": "One premise of the stored inference is outside this neighborhood, and links not shown are not evidence of absence.", "basis": []}]
    })
    .to_string()
}

fn cancel() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

#[test]
fn request_identity_follows_basis_options_and_role_configuration() {
    let f = rich();
    let input = input_of(&f);
    let a = build_request(&input, "fp-a").cache_key();
    assert_eq!(a, build_request(&input, "fp-a").cache_key());
    assert_ne!(a, build_request(&input, "fp-b").cache_key());
    let mut changed = rich();
    changed.snapshot.nodes[1].label = "Harbor traffic rerouted again".into();
    assert_ne!(
        a,
        build_request(&input_of(&changed), "fp-a").cache_key(),
        "changed data is a new request"
    );
    let mut rt1 = runtime("http://127.0.0.1:1", "m1");
    let rt2 = runtime("http://127.0.0.1:1", "m2");
    assert_ne!(route_fingerprint(&rt1), route_fingerprint(&rt2));
    rt1.auth.accounts.clear();
    assert!(!route_fingerprint(&rt1).contains("fixture-only"));
}

// ---------------------------------------------------------- scheduler delivery

use crate::scheduler::{drain_summary_flush_with_runtime, DEFAULT_LEASE_SECS};
use std::time::{Duration, Instant};

#[test]
fn the_standard_deadline_is_the_attempt_budget_plus_a_margin() {
    let limits = WatchLimits::standard();
    assert_eq!(limits.deadline, Duration::from_secs(90 * 2 + 60));
    assert!(limits.max_poll_errors >= 2);
}
