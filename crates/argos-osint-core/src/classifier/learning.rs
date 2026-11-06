//! Immutable inputs for post-cycle learning. Source kinds and permanent family
//! partitions survive blending; held-out examples never become weak training.
use super::artifact::sha256_hex;
use super::provider_labels::{self, MappingApproval, ProviderObservation};
use super::review::{ReviewDecision, ReviewItem};
use super::training::{self, DatasetRow, FamilyId, LabelMembership, Split, TrainingCorpus};
use super::{ClassifierError, Provenance};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewedCapture {
    pub item: ReviewItem,
    pub review_reference: String,
    pub corpus_sha256: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WeakCapture {
    pub observation: ProviderObservation,
    pub family_id: FamilyId,
    pub split: Split,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningInput {
    pub schema_version: String,
    #[serde(default)]
    pub pipeline_sha256: String,
    pub corpus: TrainingCorpus,
    pub provenance: Provenance,
    pub eligible_family_ids: Vec<FamilyId>,
    pub reviews: Vec<ReviewedCapture>,
    pub approval: MappingApproval,
    pub reviewed_thresholds: bool,
}
fn invalid(code: &str) -> ClassifierError {
    ClassifierError::InvalidTrainingData {
        row_id: None,
        reason: code.into(),
    }
}

/// The durable input identity binds data to this executable feature/training
/// policy. An old candidate/job cannot silently replay under a different bundle.
pub(crate) fn pipeline_sha256() -> String {
    sha256_hex(
        format!(
            "classifier-learning-policy-v2\0{}\0{}\0{}\0{}",
            super::training::TRAINING_REPORT_SCHEMA_VERSION,
            super::calibration::CALIBRATION_VERSION,
            super::ADMISSION_POLICY_VERSION,
            sha256_hex(include_str!("../../assets/classifier/source-manifest.json").as_bytes())
        )
        .as_bytes(),
    )
}
pub(crate) fn validate_pipeline(input: &LearningInput) -> Result<(), ClassifierError> {
    if input.schema_version != "classifier-learning-input-v2"
        || input.pipeline_sha256 != pipeline_sha256()
    {
        return Err(invalid("learning_pipeline_incompatible"));
    }
    Ok(())
}

/// Constructs one bounded, frozen candidate input from actual stored evidence.
/// Reviewed holdouts replace synthetic holdouts when available; missing real
/// calibration/development remains explicit and cannot authorize promotion.
pub fn capture(
    seed: &TrainingCorpus,
    reviews: &[ReviewedCapture],
    weak: &[WeakCapture],
    approval: &MappingApproval,
) -> Result<LearningInput, ClassifierError> {
    training::validate_corpus(seed)?;
    provider_labels::validate_approval(approval)?;
    if reviews.len() > 1024 || weak.len() > 2048 {
        return Err(invalid("learning_capture_bound"));
    }
    let mut reviews: Vec<_> = reviews
        .iter()
        .filter(|review| review.item.split != Split::Test)
        .cloned()
        .collect();
    reviews.sort_by(|a, b| {
        a.item
            .family_id
            .cmp(&b.item.family_id)
            .then(
                a.item
                    .observation
                    .observation_id
                    .cmp(&b.item.observation.observation_id),
            )
            .then(a.item.revision.cmp(&b.item.revision))
    });
    let review_bytes = serde_json::to_vec(&reviews).map_err(|_| invalid("review_capture_json"))?;
    if review_bytes.len() > 16 * 1024 * 1024 {
        return Err(invalid("review_capture_byte_bound"));
    }
    let reviewed_hash = sha256_hex(&review_bytes);
    let reference = format!("classifier-review-capture:{reviewed_hash}");
    let approval_hash =
        sha256_hex(&serde_json::to_vec(approval).map_err(|_| invalid("approval_json"))?);
    let mut by_family: BTreeMap<FamilyId, &ReviewedCapture> = BTreeMap::new();
    let mut conflicts = BTreeSet::new();
    for review in &reviews {
        if review.review_reference.trim().is_empty()
            || review.review_reference.len() > 1024
            || review.review_reference.chars().any(char::is_control)
            || review.corpus_sha256.len() != 64
            || !review
                .corpus_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(invalid("invalid_review_capture"));
        }
        let item = &review.item;
        super::review::ReviewExchange {
            schema_version: super::review::REVIEW_SCHEMA.into(),
            run_id: "learning-capture".into(),
            sampling_bias: String::new(),
            items: vec![item.clone()],
        }
        .validate(&review.review_reference)?;
        if item.payload_sha256
            != sha256_hex(
                &serde_json::to_vec(&item.observation)
                    .map_err(|_| invalid("review_source_json"))?,
            )
        {
            return Err(invalid("review_source_changed"));
        }
        provider_labels::validate_observation(&item.observation)?;
        if let Some(prior) = by_family.insert(item.family_id.clone(), review) {
            if prior.item.split != item.split || prior.item.membership() != item.membership() {
                conflicts.insert(item.family_id.clone());
            }
        }
    }
    let usable = |review: &ReviewedCapture| {
        review
            .item
            .observation
            .language
            .as_deref()
            .is_some_and(|language| {
                let language = language.to_ascii_lowercase();
                language == "en" || language.starts_with("en-")
            })
    };
    let reviewed_thresholds = by_family.values().filter(|review| usable(review)).any(|r| {
        r.item.split == Split::Calibration
            && r.item.decision == ReviewDecision::Complete
            && !conflicts.contains(&r.item.family_id)
    });
    let reviewed_development = by_family.values().filter(|review| usable(review)).any(|r| {
        r.item.split == Split::Development
            && r.item.decision == ReviewDecision::Complete
            && !conflicts.contains(&r.item.family_id)
    });
    let mut rows: Vec<_> = seed
        .rows
        .iter()
        .filter(|row| {
            row.split == Split::Train
                || (row.split == Split::Calibration && !reviewed_thresholds)
                || (row.split == Split::Development && !reviewed_development)
        })
        .cloned()
        .collect();
    // Test is outside repeated learning/selection. Its existing identity is
    // captured separately by the retained seed; no Test text enters this job.
    let mut eligible = BTreeSet::new();
    for (family, review) in &by_family {
        let item = &review.item;
        if conflicts.contains(family)
            || item.decision != ReviewDecision::Complete
            || item.split == Split::Test
            || !item
                .observation
                .language
                .as_deref()
                .is_some_and(|language| {
                    let language = language.to_ascii_lowercase();
                    language == "en" || language.starts_with("en-")
                })
        {
            continue;
        }
        if item.split == Split::Train {
            eligible.insert(family.clone());
        }
        rows.push(DatasetRow {
            sample_id: format!(
                "review-{}",
                sha256_hex(item.observation.observation_id.as_bytes())
            ),
            family_id: family.clone(),
            split: item.split,
            title: item.observation.title.clone(),
            description: item.observation.description.clone(),
            language: "en".into(),
            membership: item.membership(),
            labels_complete: true,
            provenance: Provenance::reviewed_real("review-capture-v1", &reviewed_hash, &reference),
        });
    }
    let mut positive = [0u64; 6];
    let mut seed_seen = BTreeSet::new();
    for row in &rows {
        if row.split == Split::Train && seed_seen.insert(&row.family_id) {
            for (index, label) in row.membership.iter().enumerate() {
                positive[index] += u64::from(*label == LabelMembership::Present);
            }
        }
    }
    let (mut selected, mut providers, mut publishers) = (
        [0u64; 6],
        BTreeMap::<String, u64>::new(),
        BTreeMap::<String, u64>::new(),
    );
    let mut seen = BTreeSet::new();
    let mut weak: Vec<_> = weak.iter().collect();
    weak.sort_by(|a, b| {
        a.family_id.cmp(&b.family_id).then(
            a.observation
                .observation_id
                .cmp(&b.observation.observation_id),
        )
    });
    for capture in weak {
        if capture.split != Split::Train || by_family.contains_key(&capture.family_id) {
            continue;
        }
        let mapping = provider_labels::map_labels(&capture.observation, Some(approval))?;
        if seen.contains(&capture.family_id) {
            continue;
        }
        if !mapping.eligible_for_training {
            continue;
        }
        let provider = format!("{:?}", capture.observation.provider);
        let Some(publisher) = capture
            .observation
            .publisher
            .as_ref()
            .filter(|name| !name.trim().is_empty())
        else {
            continue;
        };
        if providers.get(&provider).copied().unwrap_or(0) >= 256
            || publishers.get(publisher).copied().unwrap_or(0) >= 32
            || mapping
                .training_membership
                .iter()
                .enumerate()
                .any(|(index, label)| {
                    *label == LabelMembership::Present && selected[index] >= positive[index]
                })
        {
            continue;
        }
        seen.insert(capture.family_id.clone());
        for (index, label) in mapping.training_membership.iter().enumerate() {
            selected[index] += u64::from(*label == LabelMembership::Present);
        }
        *providers.entry(provider).or_default() += 1;
        *publishers.entry(publisher.clone()).or_default() += 1;
        eligible.insert(capture.family_id.clone());
        rows.push(DatasetRow {
            sample_id: format!(
                "weak-{}",
                sha256_hex(capture.observation.observation_id.as_bytes())
            ),
            family_id: capture.family_id.clone(),
            split: Split::Train,
            title: capture.observation.title.clone(),
            description: capture.observation.description.clone(),
            language: "en".into(),
            membership: mapping.training_membership,
            labels_complete: !mapping
                .training_membership
                .contains(&LabelMembership::Unknown),
            provenance: Provenance::ApprovedProvider {
                dataset_version: provider_labels::MAPPING_VERSION.into(),
                dataset_sha256: sha256_hex(
                    &serde_json::to_vec(&capture.observation)
                        .map_err(|_| invalid("weak_source_json"))?,
                ),
                mapping_approval_sha256: approval_hash.clone(),
            },
        });
    }
    rows.sort_by(|a, b| a.sample_id.cmp(&b.sample_id));
    let hashes = training::corpus_hashes(&rows)?;
    let dataset_hash = sha256_hex(
        &serde_json::to_vec(&(
            &seed.dataset_sha256,
            &hashes,
            &approval_hash,
            &reviewed_hash,
        ))
        .map_err(|_| invalid("learning_hash_json"))?,
    );
    let corpus = TrainingCorpus {
        rows,
        dataset_sha256: dataset_hash.clone(),
        train_split_sha256: hashes.train_split_sha256,
        calibration_split_sha256: hashes.calibration_split_sha256,
        development_split_sha256: hashes.development_split_sha256,
        test_split_sha256: hashes.test_split_sha256,
        family_assignment_sha256: hashes.family_assignment_sha256,
    };
    training::validate_corpus(&corpus)?;
    let provenance = Provenance::MixedTraining {
        dataset_version: "learning-capture-v1".into(),
        dataset_sha256: dataset_hash,
        seed_dataset_sha256: seed.dataset_sha256.clone(),
        reviewed_corpus_sha256: reviewed_hash,
        review_reference: reference,
        mapping_approval_sha256: approval_hash,
    };
    Ok(LearningInput {
        schema_version: "classifier-learning-input-v2".into(),
        pipeline_sha256: pipeline_sha256(),
        corpus,
        provenance,
        eligible_family_ids: eligible.into_iter().collect(),
        reviews: reviews.to_vec(),
        approval: approval.clone(),
        reviewed_thresholds,
    })
}
