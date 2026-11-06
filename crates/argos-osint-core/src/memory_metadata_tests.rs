use super::*;

#[test]
fn kinds_round_trip_and_legacy_atomic_claim_decodes_as_claim() {
    for kind in [
        MemoryKind::Actor,
        MemoryKind::Claim,
        MemoryKind::Inference,
        MemoryKind::Link,
        MemoryKind::Related,
        MemoryKind::CycleBrief,
        MemoryKind::InvestigationDigest,
        MemoryKind::SourceSummary,
        MemoryKind::ManualNote,
        MemoryKind::UserProfile,
    ] {
        let json = serde_json::to_string(&kind).unwrap();
        assert_eq!(serde_json::from_str::<MemoryKind>(&json).unwrap(), kind);
        assert_eq!(MemoryKind::parse(kind.as_str()), Some(kind));
        assert_eq!(kind.to_string(), kind.as_str());
    }
    assert_eq!(
        serde_json::from_str::<MemoryKind>("\"atomic_claim\"").unwrap(),
        MemoryKind::Claim
    );
    assert!(MemoryKind::Claim
        .stored_spellings()
        .contains(&"atomic_claim"));
    assert!(serde_json::from_str::<MemoryKind>("\"fact\"").is_err());
    assert_eq!(MemoryKind::CORE.iter().filter(|k| k.is_core()).count(), 5);
    assert!(!MemoryKind::ManualNote.is_core());
}

#[test]
fn classification_receipts_map_to_claim_or_inference() {
    assert_eq!(
        MemoryKind::from_classification("inference"),
        MemoryKind::Inference
    );
    assert_eq!(
        MemoryKind::from_classification(" Inference "),
        MemoryKind::Inference
    );
    assert_eq!(MemoryKind::from_classification("fact"), MemoryKind::Claim);
    assert_eq!(MemoryKind::from_classification("claim"), MemoryKind::Claim);
}

#[test]
fn typed_payloads_are_discriminated_and_endpoints_typed() {
    let link = MemoryPayload::Link(Box::new(LinkPayload {
        validation: None,
        source: EndpointRef::Memory {
            memory_id: "actor-1".into(),
            kind: MemoryKind::Actor,
        },
        target: EndpointRef::Article {
            run_id: "run-1".into(),
            article_id: "a-1".into(),
            revision: "r1".into(),
        },
        relation: "mentioned_in".into(),
        rationale: String::new(),
        receipts: vec![],
        promoted_from: None,
    }));
    let json = serde_json::to_value(&link).unwrap();
    assert_eq!(json["kind"], "link");
    assert_eq!(json["payload"]["source"]["type"], "memory");
    assert_eq!(json["payload"]["target"]["type"], "article");
    let back: MemoryPayload = serde_json::from_value(json).unwrap();
    assert_eq!(back, link);
    assert_eq!(back.kind(), MemoryKind::Link);
    let MemoryPayload::Link(l) = back else {
        unreachable!()
    };
    assert_eq!(l.target.index_key().0, "article");
    assert_eq!(l.source.index_key().2, "actor");
    // A payload of the wrong shape for its kind is rejected.
    assert!(serde_json::from_value::<MemoryPayload>(
        serde_json::json!({"kind":"related","payload":{"relation":"x"}})
    )
    .is_err());
}

#[test]
fn inference_fidelity_never_becomes_support_and_current_attributes_are_validated() {
    let legacy: InferencePayload = serde_json::from_value(serde_json::json!({"premise_claims":["c1"],"reasoning":"Historical derivation","support_state":"faithful"})).unwrap();
    assert_eq!(
        legacy.support(),
        None,
        "faithful was never a support decision"
    );
    assert!(
        legacy.validate_attributes().is_ok(),
        "legacy remains readable"
    );
    let mut current = InferencePayload {
        inference_kind: Some(crate::intelligence_contracts::InferenceKind::ConditionalOutlook),
        domains: vec![
            crate::atlas_taxonomy::DomainId::Security,
            crate::atlas_taxonomy::DomainId::Economic,
        ],
        support_state: "conditional".into(),
        conditions: vec!["If the closure persists".into()],
        limitations: "No later traffic measurements".into(),
        extraction_fidelity: Some("faithful".into()),
        ..Default::default()
    };
    assert_eq!(current.support(), Some("conditional"));
    assert!(current.validate_attributes().is_ok());
    let serialized = serde_json::to_string(&current).unwrap();
    let restored: InferencePayload = serde_json::from_str(&serialized).unwrap();
    assert_eq!(restored, current);
    current.conditions.clear();
    assert!(current.validate_attributes().is_err());
}
#[test]
fn assertion_revision_ignores_only_publication_locator_and_preserves_material_attributes() {
    use crate::atlas_taxonomy::DomainId;
    use crate::intelligence_contracts::InferenceKind;
    let text = "Shipping traffic may shift if the port closure continues.";
    let payload = InferencePayload {
        inference_kind: Some(InferenceKind::ConditionalOutlook),
        domains: vec![DomainId::Security, DomainId::Economic],
        conditions: vec!["If the port closure continues".into()],
        extraction_fidelity: Some("faithful".into()),
        premise_claims: vec!["claim:closure".into()],
        reasoning: "The retained story states ships are redirected".into(),
        limitations: "No complete duration is known".into(),
        support_state: "conditional".into(),
        coverage_receipt: "snapshot-generation-a".into(),
        ..Default::default()
    };
    let original =
        assertion_semantic_revision(text, &MemoryPayload::Inference(payload.clone())).unwrap();
    let mut republished = payload.clone();
    republished.coverage_receipt = "snapshot-generation-b".into();
    assert_eq!(
        original,
        assertion_semantic_revision(text, &MemoryPayload::Inference(republished)).unwrap()
    );
    assert_ne!(
        original,
        assertion_semantic_revision(
            "Traffic has already shifted.",
            &MemoryPayload::Inference(payload.clone())
        )
        .unwrap()
    );
    for material in 0..7 {
        let mut changed = payload.clone();
        match material {
            0 => changed.conditions = vec!["If a second harbor closes".into()],
            1 => changed.support_state = "partially_supported".into(),
            2 => changed.domains = vec![DomainId::Environmental, DomainId::Economic],
            3 => changed.inference_kind = Some(InferenceKind::Mechanism),
            4 => changed.premise_claims = vec!["claim:revised-source".into()],
            5 => changed.limitations = "Retained sources dispute the extent".into(),
            _ => changed.reasoning = "The retained source was corrected".into(),
        }
        assert_ne!(
            original,
            assertion_semantic_revision(text, &MemoryPayload::Inference(changed)).unwrap(),
            "material field {material} invalidates the endpoint"
        );
    }
}
