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
    let link = MemoryPayload::Link(LinkPayload {
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
    });
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
