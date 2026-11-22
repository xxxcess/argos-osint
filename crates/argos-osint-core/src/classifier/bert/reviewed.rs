//! Operator-owned calibration of an approved BERT checkpoint using reviewed news cycles.

use std::collections::BTreeSet;
use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::candidate::{self, BaselineCheck};
use super::cycle_metrics::{self, ReviewedCycleMetrics, ReviewedPrediction};
use super::{
    active_atlas_bert_revision, artifact, controlled, worker_text, Bundle, Worker, MICROBATCH,
    MICROBATCH_TIMEOUT, STARTUP_TIMEOUT,
};
use crate::classifier::artifact::ArtifactStore;
use crate::classifier::remote::PinIdentity;
use crate::classifier::review::{ReviewDecision, ReviewExchange};
use crate::classifier::training::{FamilyId, LabelMembership, Split};
use crate::classifier::{ClassifierError, ClassifyInput, Provenance, WorkControl};

const RECORD_SCHEMA: &str = "atlas-bert-reviewed-calibration-v1";
const FROZEN_RECORD: &str = "atlas-bert-frozen-review.json";
const TEST_LEDGER: &str = "atlas-bert-reviewed-test-families.json";
const MAX_TEST_FAMILIES: usize = 1024;

/// Result of evaluating a threshold on independent reviewed news-cycle families.
#[derive(Debug, Serialize)]
pub struct ReviewedCalibrationOutcome {
    metrics: ReviewedCycleMetrics,
    /// Exact ranked subject used to score the cohort.
    assessed_revision: String,
    cohort_sha256: String,
    applied_revision: Option<String>,
    recalibration_frozen: bool,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RecordedCalibration {
    schema_version: String,
    frozen_sha256: String,
    evaluation_sha256: String,
    cohort_sha256: String,
    pub(super) metrics: ReviewedCycleMetrics,
}

pub(super) fn record_name(digest: &str) -> String {
    format!("reviewed-calibration-{digest}.json")
}

pub(super) fn load(
    model_dir: &Path,
    digest: &str,
    frozen: &str,
    evaluation: &str,
) -> Result<RecordedCalibration, ClassifierError> {
    let bytes = candidate::read_expected_json(model_dir, &record_name(digest), digest)?;
    let record: RecordedCalibration = serde_json::from_slice(&bytes)
        .map_err(|_| candidate::invalid("invalid reviewed calibration record"))?;
    if record.schema_version != RECORD_SCHEMA
        || record.frozen_sha256 != frozen
        || record.evaluation_sha256 != evaluation
        || !is_sha256(&record.cohort_sha256)
        || !record.metrics.valid()
        || !record.metrics.promotion_eligible()
    {
        return Err(candidate::invalid(
            "reviewed calibration evidence is inconsistent",
        ));
    }
    Ok(record)
}

pub(super) fn frozen_digest(
    model_dir: &Path,
    frozen: &str,
    evaluation: &str,
) -> Result<Option<String>, ClassifierError> {
    let path = model_dir.join(FROZEN_RECORD);
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(super::file_error(&path, error)),
        Ok(_) => (),
    }
    let digest: String = serde_json::from_slice(&candidate::read_json(&path)?)
        .map_err(|_| candidate::invalid("invalid frozen calibration identity"))?;
    if !is_sha256(&digest)
        || !load(model_dir, &digest, frozen, evaluation)?
            .metrics
            .freeze_calibration()
    {
        return Err(candidate::invalid("invalid frozen calibration evidence"));
    }
    Ok(Some(digest))
}

fn is_sha256(digest: &str) -> bool {
    digest.len() == 64
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct TestLedger {
    families: BTreeSet<String>,
}

impl TestLedger {
    fn load(root: &Path) -> Result<Self, ClassifierError> {
        let path = root.join(TEST_LEDGER);
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => return Err(super::file_error(&path, error)),
            Ok(_) => (),
        }
        let ledger: Self = serde_json::from_slice(&candidate::read_json(&path)?)
            .map_err(|_| candidate::invalid("invalid reviewed test-family ledger"))?;
        if ledger.families.len() > MAX_TEST_FAMILIES
            || ledger.families.iter().any(|digest| !is_sha256(digest))
        {
            return Err(candidate::invalid("invalid reviewed test-family identity"));
        }
        Ok(ledger)
    }
}

struct ReviewedInput {
    text: Option<String>,
    labels: [bool; 6],
}

struct Cohort {
    calibration: Vec<ReviewedInput>,
    test: Vec<ReviewedInput>,
    sha256: String,
}

fn prepare(exchange: ReviewExchange, ledger: &mut TestLedger) -> Result<Cohort, ClassifierError> {
    exchange.validate("verified news-cycle review")?;
    let sha256 = exchange.sha256()?;
    let mut calibration = Vec::new();
    let mut test = Vec::new();
    let mut families = BTreeSet::new();
    for item in exchange.items {
        let family = FamilyId::parse(item.family_id.as_str())?;
        if item.decision != ReviewDecision::Complete || !families.insert(family) {
            return Err(candidate::invalid(
                "a cohort requires one complete review per family",
            ));
        }
        let partition = match item.split {
            Split::Calibration => &mut calibration,
            Split::Test => {
                let digest = artifact::sha256_hex(item.family_id.as_str().as_bytes());
                if !ledger.families.insert(digest) {
                    return Err(candidate::invalid(
                        "a reviewed test family was already scored",
                    ));
                }
                &mut test
            }
            _ => {
                return Err(candidate::invalid(
                    "expected reviewed calibration and test families",
                ))
            }
        };
        let labels = item
            .membership()
            .map(|label| label == LabelMembership::Present);
        let observation = item.observation;
        let input = ClassifyInput::new(
            observation.observation_id,
            observation.title,
            observation.description,
            observation.language,
        );
        partition.push(ReviewedInput {
            text: worker_text(&input),
            labels,
        });
    }
    if ledger.families.len() > MAX_TEST_FAMILIES {
        return Err(candidate::invalid(
            "reviewed test-family ledger exceeds its bound",
        ));
    }
    Ok(Cohort {
        calibration,
        test,
        sha256,
    })
}

async fn score(
    worker: &mut Worker,
    rows: Vec<ReviewedInput>,
    control: &WorkControl,
) -> Result<Vec<ReviewedPrediction>, ClassifierError> {
    let mut result = Vec::with_capacity(rows.len());
    for batch in rows.chunks(MICROBATCH) {
        control.check_active()?;
        let texts: Vec<_> = batch.iter().filter_map(|row| row.text.clone()).collect();
        let mut predictions = if texts.is_empty() {
            Vec::new()
        } else {
            controlled(worker.score(&texts), control, MICROBATCH_TIMEOUT).await?
        }
        .into_iter();
        for row in batch {
            let scores = if row.text.is_some() {
                predictions.next().ok_or(ClassifierError::WorkerStopped)?
            } else {
                // Inputs rejected by the classifier receive no domain assignments.
                [0.0; 6]
            };
            result.push(ReviewedPrediction {
                scores,
                labels: row.labels,
            });
        }
    }
    Ok(result)
}

/// Recalibrates an approved checkpoint using a verified operator-owned cohort.
///
/// Threshold selection uses the calibration partition. Independent test families
/// are consumed once, and only a score strictly above the promotion floor can
/// update the pointer. A threshold that meets the stop rule freezes its active
/// model for subsequent recalibration requests.
///
/// # Errors
/// Returns an error for invalid evidence, a stale revision, a frozen model,
/// cancelled work, unavailable inference, or a filesystem failure.
pub async fn recalibrate(
    home: &Path,
    expected_revision: &str,
    review: ReviewExchange,
    control: &WorkControl,
) -> Result<ReviewedCalibrationOutcome, ClassifierError> {
    control.check_active()?;
    let home = home.to_owned();
    let expected = expected_revision.to_owned();
    let lease_home = home.clone();
    let prior = expected.clone();
    let (lease, root, current, ledger, cohort) = tokio::task::spawn_blocking(move || {
        let store = ArtifactStore::new(&lease_home)?;
        let lease = store.try_writer()?;
        let current = Bundle::load(&lease_home, None)?;
        let identity = PinIdentity {
            snapshot: current.snapshot.clone(),
            manifest_sha256: current.manifest_sha256.clone(),
        };
        if identity.activation_revision()? != prior {
            return Err(ClassifierError::StaleActivation);
        }
        let Provenance::FineTunedAtlasBert {
            frozen_sha256,
            evaluation_sha256,
            ..
        } = &current.snapshot.provenance
        else {
            return Err(candidate::invalid(
                "activate a qualified BERT candidate before recalibrating",
            ));
        };
        if current
            .reviewed_calibration
            .as_ref()
            .is_some_and(|r| r.metrics.freeze_calibration())
            || frozen_digest(&current.config.model_dir, frozen_sha256, evaluation_sha256)?.is_some()
        {
            return Err(ClassifierError::ActivationDenied {
                reason: "reviewed BERT recalibration is frozen".into(),
            });
        }
        let root = store.root().to_owned();
        let mut ledger = TestLedger::load(&root)?;
        let cohort = prepare(review, &mut ledger)?;
        Ok::<_, ClassifierError>((lease, root, current, ledger, cohort))
    })
    .await
    .map_err(|_| ClassifierError::WorkerStopped)??;
    let mut worker = controlled(Worker::start(&current.config), control, STARTUP_TIMEOUT).await?;
    let Cohort {
        calibration,
        test,
        sha256,
    } = cohort;
    let calibration = score(&mut worker, calibration, control).await?;
    let test = score(&mut worker, test, control).await?;
    drop(worker);
    let control = control.clone();
    tokio::task::spawn_blocking(move || {
        let _lease = lease;
        let metrics = cycle_metrics::calibrate_reviewed_cycle(&calibration, &test)?;
        let configured_root = artifact::root_for(&home)?;
        if configured_root.canonicalize().ok().as_ref() != Some(&root)
            || active_atlas_bert_revision(&home)? != expected
        {
            return Err(ClassifierError::StaleActivation);
        }
        control.check_active()?;
        let eligible = metrics.promotion_eligible();
        let mut outcome = ReviewedCalibrationOutcome {
            recalibration_frozen: eligible && metrics.freeze_calibration(),
            assessed_revision: expected,
            cohort_sha256: sha256,
            applied_revision: None,
            metrics,
        };
        // Mark the held-out evidence before disclosing its score or changing
        // the active pointer. A partial write can consume evidence but cannot
        // activate an unrecorded threshold.
        artifact::write_atomic(
            &root.join(TEST_LEDGER),
            &artifact::bounded_json_bytes(&ledger)?,
        )?;
        if eligible {
            let Provenance::FineTunedAtlasBert {
                frozen_sha256,
                evaluation_sha256,
                ..
            } = current.snapshot.provenance
            else {
                return Err(candidate::invalid("expected a fine-tuned BERT pin"));
            };
            let record = RecordedCalibration {
                schema_version: RECORD_SCHEMA.into(),
                frozen_sha256: frozen_sha256.clone(),
                evaluation_sha256: evaluation_sha256.clone(),
                cohort_sha256: outcome.cohort_sha256.clone(),
                metrics: outcome.metrics.clone(),
            };
            let bytes = artifact::bounded_json_bytes(&record)?;
            let digest = artifact::sha256_hex(&bytes);
            artifact::write_atomic(&current.config.model_dir.join(record_name(&digest)), &bytes)?;
            let updated = candidate::load(
                &home,
                &frozen_sha256,
                &evaluation_sha256,
                Some(&digest),
                BaselineCheck::PinnedReplay,
            )?;
            let active = PinIdentity {
                snapshot: updated.snapshot,
                manifest_sha256: updated.manifest_sha256,
            };
            let revision = active.activation_revision()?;
            control.check_active()?;
            if outcome.recalibration_frozen {
                // Retain the reviewed result if pointer publication needs recovery
                // or the operator activates this checkpoint again later.
                artifact::write_atomic(
                    &current.config.model_dir.join(FROZEN_RECORD),
                    &artifact::bounded_json_bytes(&digest)?,
                )?;
            }
            artifact::write_atomic(
                &root.join(super::ATLAS_BERT_ACTIVE_FILE),
                &artifact::bounded_json_bytes(&active)?,
            )?;
            outcome.applied_revision = Some(revision);
        }
        Ok(outcome)
    })
    .await
    .map_err(|_| ClassifierError::WorkerStopped)?
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::atlas_taxonomy::{DomainId, DOMAIN_IDS};
    use crate::classifier::review::REVIEW_SCHEMA;

    mod fixture {
        use super::{artifact, DomainId, DOMAIN_IDS};

        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/classifier/atlas_bert_candidate.rs"
        ));
    }

    #[tokio::test]
    async fn reviewed_goal_persists_across_restart_and_keeps_prior_pin_replayable() {
        let installed = fixture::StagedCandidate::new(0.8);
        let home = installed.home.path();
        let prior = candidate::load(
            home,
            &installed.frozen_sha256,
            &installed.evaluation_sha256,
            None,
            BaselineCheck::PinnedReplay,
        )
        .unwrap();
        let mut predictions = vec![
            ReviewedPrediction {
                scores: [0.0; 6],
                labels: [false; 6]
            };
            20
        ];
        for index in 0..DomainId::ALL.len() {
            let mut scores = [0.1; 6];
            scores[index] = 0.95;
            let mut labels = [false; 6];
            labels[index] = true;
            predictions.extend(vec![ReviewedPrediction { scores, labels }; 20]);
        }
        let metrics = cycle_metrics::calibrate_reviewed_cycle(&predictions, &predictions).unwrap();
        assert!(metrics.freeze_calibration());
        let record = RecordedCalibration {
            schema_version: RECORD_SCHEMA.into(),
            frozen_sha256: installed.frozen_sha256.clone(),
            evaluation_sha256: installed.evaluation_sha256.clone(),
            cohort_sha256: "a".repeat(64),
            metrics,
        };
        let bytes = artifact::bounded_json_bytes(&record).unwrap();
        let digest = artifact::sha256_hex(&bytes);
        let record_path = installed.model_dir.join(record_name(&digest));
        fs::write(&record_path, bytes).unwrap();
        fs::write(
            installed.model_dir.join(FROZEN_RECORD),
            artifact::bounded_json_bytes(&digest).unwrap(),
        )
        .unwrap();
        assert_eq!(
            frozen_digest(
                &installed.model_dir,
                &installed.frozen_sha256,
                &installed.evaluation_sha256
            )
            .unwrap(),
            Some(digest.clone())
        );
        let promoted = candidate::load(
            home,
            &installed.frozen_sha256,
            &installed.evaluation_sha256,
            Some(&digest),
            BaselineCheck::PinnedReplay,
        )
        .unwrap();
        let active = PinIdentity {
            snapshot: promoted.snapshot,
            manifest_sha256: promoted.manifest_sha256,
        };
        let root = artifact::root_for(home).unwrap();
        fs::write(
            root.join(super::super::ATLAS_BERT_ACTIVE_FILE),
            artifact::bounded_json_bytes(&active).unwrap(),
        )
        .unwrap();

        let revision = active_atlas_bert_revision(home).unwrap();
        assert_eq!(revision, active.activation_revision().unwrap());
        assert_ne!(active.snapshot, prior.snapshot);
        let replay = Bundle::load(home, Some(prior.snapshot.clone())).unwrap();
        assert_eq!(replay.snapshot, prior.snapshot);
        let exchange = ReviewExchange {
            schema_version: REVIEW_SCHEMA.into(),
            run_id: "new-cycle".into(),
            sampling_bias: String::new(),
            items: vec![],
        };
        assert!(matches!(
            recalibrate(home, &revision, exchange, &WorkControl::new()).await,
            Err(ClassifierError::ActivationDenied { .. })
        ));

        fs::write(record_path, b"{}").unwrap();
        assert!(active_atlas_bert_revision(home).is_err());
    }
}
