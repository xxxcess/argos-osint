//! Captured learning inputs preserve reviewed evidence and permanent holdouts.
use argos_osint_core::atlas_taxonomy::DomainId;
use argos_osint_core::classifier::{
    dataset, learning, provider_labels, review, training, Provenance,
};
fn seed() -> training::TrainingCorpus {
    let mut request = dataset::DatasetRequest::default();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    request.cwd = root.clone();
    request.home = root.join(".agent-scratch/unselected-learning-fixture");
    request.executable = root.join("argos");
    request.explicit_file = Some(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../datasets/argos_atlas_strategic_domains_v2.csv"),
    );
    dataset::parse_csv(&dataset::resolve_dataset(&request).unwrap())
        .unwrap()
        .corpus
}
fn approval() -> provider_labels::MappingApproval {
    provider_labels::MappingApproval {
        mapping_version: provider_labels::MAPPING_VERSION.into(),
        predicates_sha256: provider_labels::mapping_predicates_sha256(),
        review_reference: "isolated mapping fixture".into(),
        reviewed_corpus_sha256: "a".repeat(64),
        domains: DomainId::ALL.map(|domain| provider_labels::RuleValidation {
            domain,
            true_positive_families: 20,
            false_positive_families: 0,
        }),
    }
}
fn reviewed(id: &str, split: training::Split) -> learning::ReviewedCapture {
    let observation = provider_labels::ProviderObservation {
        observation_id: id.into(),
        provider: provider_labels::ProviderFormat::NewsDataV1,
        endpoint: "fixture-v1".into(),
        article_id: Some(id.into()),
        input_hash: "0".repeat(64),
        cycle_ids: vec!["fixture".into()],
        url: format!("https://fixture.invalid/{id}"),
        publisher: Some("Fixture".into()),
        published_at: Some("2026-11-06".into()),
        fetched_at: "2026-11-06T12:00:00Z".into(),
        source_revision: "source-1".into(),
        title: format!("Parliament votes on legislation {id}"),
        description: "Lawmakers ratify the statute after a parliamentary vote.".into(),
        language: Some("en".into()),
        raw_categories: serde_json::json!(["politics"]),
        raw_ai_tags: None,
        raw_metadata: serde_json::json!({}),
    };
    let payload_sha256 = argos_osint_core::classifier::artifact::sha256_hex(
        &serde_json::to_vec(&observation).unwrap(),
    );
    learning::ReviewedCapture {
        item: review::ReviewItem {
            observation,
            payload_sha256,
            family_id: training::FamilyId::parse(id).unwrap(),
            split,
            revision: 1,
            decision: review::ReviewDecision::Complete,
            domains: vec![DomainId::Governance],
            review_note: String::new(),
        },
        review_reference: "actual isolated reviewer fixture".into(),
        corpus_sha256: "b".repeat(64),
    }
}
#[test]
fn capture_excludes_test_and_preserves_actual_review_sources_and_seed_identity() {
    let seed = seed();
    let mut reviews: Vec<_> = (0..100)
        .map(|i| reviewed(&format!("reviewed-{i}"), training::Split::Train))
        .collect();
    reviews.push(reviewed("final-real-holdout", training::Split::Test));
    let captured = learning::capture(&seed, &reviews, &[], &approval()).unwrap();
    assert_eq!(captured.eligible_family_ids.len(), 100);
    assert!(captured
        .corpus
        .rows
        .iter()
        .all(|row| row.split != training::Split::Test));
    assert_eq!(
        captured.reviews.len(),
        100,
        "Test source text stays outside repeated learning captures"
    );
    assert!(captured
        .corpus
        .rows
        .iter()
        .filter(|row| row.sample_id.starts_with("review-"))
        .all(|row| matches!(row.provenance, Provenance::ReviewedReal { .. })));
    assert!(
        matches!(captured.provenance,Provenance::MixedTraining {seed_dataset_sha256,..} if seed_dataset_sha256==seed.dataset_sha256)
    );
    assert!(!captured.reviewed_thresholds);
}
#[test]
fn reviewed_holdout_is_never_reintroduced_as_weak_training_and_conflicts_abstain() {
    let seed = seed();
    let review = reviewed("held-out", training::Split::Development);
    let weak = learning::WeakCapture {
        observation: review.item.observation.clone(),
        family_id: review.item.family_id.clone(),
        split: training::Split::Train,
    };
    let captured =
        learning::capture(&seed, std::slice::from_ref(&review), &[weak], &approval()).unwrap();
    assert!(captured.eligible_family_ids.is_empty());
    assert_eq!(
        captured
            .corpus
            .rows
            .iter()
            .filter(|row| row.split == training::Split::Development)
            .count(),
        1
    );
    let mut contradiction = review.clone();
    contradiction.item.domains = vec![DomainId::Security];
    let captured = learning::capture(&seed, &[review, contradiction], &[], &approval()).unwrap();
    assert!(!captured
        .corpus
        .rows
        .iter()
        .any(|row| row.family_id.as_str() == "held-out"));
}
#[test]
fn changed_review_source_is_rejected_instead_of_minting_reviewed_provenance() {
    let mut review = reviewed("changed", training::Split::Train);
    review.item.observation.title.push_str(" correction");
    assert!(learning::capture(&seed(), &[review], &[], &approval()).is_err());
}
