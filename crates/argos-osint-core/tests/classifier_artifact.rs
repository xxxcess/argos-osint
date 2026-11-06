//! Immutable model artifacts generated from real classifier recipes.
#[cfg(feature = "classifier")]
#[path = "fixtures/classifier/support.rs"]
mod support;
use argos_osint_core::classifier::artifact::ModelId;
#[cfg(feature = "classifier")]
use argos_osint_core::classifier::{artifact, ClassifierError};

#[test]
fn model_ids_validate_in_parser_and_deserializer() {
    for value in ["", ".", "..", "/", "a/b", "a\\b", "\0", "non-ascii-Ж"] {
        assert!(ModelId::parse(value).is_none(), "accepted {value:?}");
        assert!(serde_json::from_value::<ModelId>(serde_json::json!(value)).is_err());
    }
    assert!(ModelId::parse(&"a".repeat(65)).is_none());
    for value in ["a.b".into(), "alpha-beta.gamma_9".into(), "a".repeat(64)] {
        let parsed = ModelId::parse(&value).unwrap();
        assert_eq!(
            serde_json::from_value::<ModelId>(serde_json::json!(value)).unwrap(),
            parsed
        );
    }
}

#[cfg(feature = "classifier")]
#[test]
fn artifact_adversarial_matrix_fails_before_activation() {
    let _serial = support::serial();
    let candidate = support::Candidate::from_fixture(support::trained());
    artifact::validate_artifact(
        &candidate.manifest,
        &candidate.payload,
        artifact::compiled_bundle_sha256(),
    )
    .unwrap();
    let mutations: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/classifier/corrupt-models.json")).unwrap();
    for case in mutations["cases"].as_array().unwrap() {
        let name = case["id"].as_str().unwrap();
        let mut payload: serde_json::Value = serde_json::from_str(&candidate.payload).unwrap();
        let model = &mut payload["domains"]["governance"];
        match case["operation"].as_str().unwrap() {
            "missing_present" => {
                model["class_totals"]
                    .as_object_mut()
                    .unwrap()
                    .remove("present");
            }
            "third_label" => {
                model["class_totals"]["unknown"] = 2.into();
            }
            "zero_present" => {
                model["class_totals"]["present"] = 0.into();
            }
            "duplicate_feature" => {
                let feature = model["feature_order"][0].clone();
                model["feature_order"].as_array_mut().unwrap().push(feature);
            }
            "extra_domain" => {
                payload["domains"]["unk"] = serde_json::json!({});
            }
            "missing_domain" => {
                payload["domains"]
                    .as_object_mut()
                    .unwrap()
                    .remove("scientific");
            }
            "forbidden_key" => {
                payload["domains"]["governance"]["__proto__"] = serde_json::json!({});
            }
            "missing_order" => {
                model.as_object_mut().unwrap().remove("feature_order");
            }
            other => panic!("unhandled mutation {other}"),
        }
        let payload = payload.to_string();
        let mut manifest = candidate.manifest.clone();
        manifest.model_sha256 = artifact::sha256_hex(payload.as_bytes());
        let err =
            artifact::validate_artifact(&manifest, &payload, artifact::compiled_bundle_sha256())
                .expect_err(name);
        assert!(
            matches!(err, ClassifierError::InvalidArtifact(_)),
            "{name}: {err:?}"
        );
        assert!(!candidate.store.active_manifest_path().exists());
    }
}

#[cfg(feature = "classifier")]
#[test]
fn compiled_identity_restore_and_data_only_checks_fail_closed() {
    let _serial = support::serial();
    let candidate = support::Candidate::from_fixture(support::trained());
    for field in [
        "bundle_sha256",
        "adapter_sha256",
        "source_manifest_sha256",
        "bridge_sha256",
        "bridge_version",
        "natural_version",
        "apparatus_version",
    ] {
        let mut json = serde_json::to_value(&candidate.manifest).unwrap();
        json[field] = if field.ends_with("sha256") {
            "f".repeat(64)
        } else {
            "unsupported-version".into()
        }
        .into();
        let manifest = serde_json::from_value(json).unwrap();
        let err = artifact::validate_artifact(
            &manifest,
            &candidate.payload,
            artifact::compiled_bundle_sha256(),
        )
        .unwrap_err();
        assert!(
            matches!(err, ClassifierError::Incompatible(_)),
            "{field}: {err:?}"
        );
    }
}

#[cfg(feature = "classifier")]
#[test]
fn synthetic_activation_requires_core_authorization() {
    use argos_osint_core::classifier::calibration::{
        authorize_activation, ActivationAuthorizationDecision,
    };
    use argos_osint_core::classifier::training::SyntheticActivation;
    let _serial = support::serial();
    let candidate = support::Candidate::from_fixture(support::trained());
    let validated = candidate.write();
    let refusal = authorize_activation(
        &candidate.fixture.lease,
        &validated,
        &candidate.report,
        &candidate.request(SyntheticActivation::RetainPrior),
        None,
        None,
    )
    .unwrap();
    assert!(matches!(
        refusal,
        ActivationAuthorizationDecision::RetainPrior(_)
    ));
    assert!(!candidate.store.active_manifest_path().exists());
    let allowed = authorize_activation(
        &candidate.fixture.lease,
        &validated,
        &candidate.report,
        &candidate.request(SyntheticActivation::ExplicitOptIn),
        None,
        None,
    )
    .unwrap();
    let ActivationAuthorizationDecision::Authorized(auth) = allowed else {
        panic!("explicit seed consent denied")
    };
    let active = candidate
        .store
        .activate(&candidate.fixture.model, &auth, &candidate.fixture.lease)
        .unwrap();
    assert_eq!(active.activation_generation, 1);
    let before = std::fs::read(candidate.store.active_manifest_path()).unwrap();
    assert!(matches!(
        candidate
            .store
            .activate(&candidate.fixture.model, &auth, &candidate.fixture.lease),
        Err(ClassifierError::StaleActivation)
    ));
    assert_eq!(
        std::fs::read(candidate.store.active_manifest_path()).unwrap(),
        before
    );
    assert_eq!(
        std::fs::read(
            candidate
                .store
                .model_dir(&candidate.fixture.model)
                .join("model.json")
        )
        .unwrap(),
        candidate.payload.as_bytes()
    );
}
