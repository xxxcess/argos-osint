use std::collections::BTreeMap;
use std::fs;
use std::fs::File;
use std::io::ErrorKind;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::{
    artifact, bundle_manifest_hash, controlled, file_error, file_sha256, Bundle, Config, Worker,
    MICROBATCH, MICROBATCH_TIMEOUT, MODEL_FILES, PROTOCOL, STARTUP_TIMEOUT,
};
use crate::atlas_taxonomy::{DomainId, DOMAIN_IDS};
use crate::classifier::remote::PinIdentity;
use crate::classifier::{ClassifierError, Provenance, WorkControl};

const CANDIDATE_DIRECTORY: &str = "atlas-bert-candidates";
const FROZEN_FILE: &str = "atlas-bert-frozen.json";
const EVALUATION_FILE: &str = "atlas-bert-evaluation.json";
const FROZEN_SCHEMA: &str = "atlas-bert-frozen-v1";
const DATASET_SHA256: &str = "625f10150ba413f2fcae49ef425f9829004daaf9e69e49eecf66f7598de80d93";
const REGRESSION_POLICY: &str = "atlas-bert-synthetic-noninferiority-v1";
const EVALUATION_SCHEMA: &str = "atlas-bert-evaluation-v1";
const BASE_CHECKPOINT_REVISION: &str = "cfa538a0fddbbd978fefe8966c1aeff7ad409c90";
const MAX_JSON_BYTES: u64 = 128 * 1024;
const SPLIT_COUNTS: [(&str, usize); 4] = [
    ("train", 440),
    ("development", 165),
    ("calibration", 230),
    ("test", 185),
];

pub(crate) const ATLAS_BERT_ACTIVE_FILE: &str = "atlas-bert-active.json";

/// The exact verified Atlas BERT model and manifest staged for operator activation.
pub type AtlasBertActive = PinIdentity;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FrozenCandidate {
    schema_version: String,
    dataset_sha256: String,
    base_checkpoint_revision: String,
    split_counts: BTreeMap<String, usize>,
    threshold: f64,
    assets: BTreeMap<String, String>,
    provenance: CandidateProvenance,
}

#[derive(Deserialize)]
struct CandidateProvenance {
    kind: String,
    source_revision: String,
    calibration_worker_sha256: String,
    baseline_assets: BTreeMap<String, String>,
    runtime_contract: RuntimeContract,
}

#[derive(Deserialize)]
struct RuntimeContract {
    protocol: String,
    domains: [String; 6],
    hypotheses: [String; 6],
    max_tokens: usize,
    max_batch: usize,
}

#[derive(Deserialize)]
struct Evaluation {
    schema_version: String,
    frozen_sha256: String,
    dataset_sha256: String,
    held_out_rows: usize,
    worker_sha256: String,
    baseline_threshold: f64,
    baseline_base_checkpoint_revision: String,
    baseline_assets: BTreeMap<String, String>,
    regression_policy: String,
    baseline: Metrics,
    candidate: Metrics,
    passed: bool,
}

#[derive(Deserialize)]
struct Metrics {
    per_domain: BTreeMap<DomainId, DomainMetrics>,
    top_two: TopTwoMetrics,
    macro_f1: f64,
    micro_f1: f64,
    false_out_of_scope_admissions: usize,
    out_of_scope_rows: usize,
}

#[derive(Deserialize)]
struct DomainMetrics {
    tp: usize,
    fp: usize,
    #[serde(rename = "fn")]
    false_negatives: usize,
    f1: f64,
}

#[derive(Deserialize)]
struct TopTwoMetrics {
    pair_rows: usize,
    exact_pair_rows: usize,
    triad_rows: usize,
    valid_triad_pairs: usize,
    single_domain_rows: usize,
    excess_single_domain_admissions: usize,
    published_pairs: usize,
    invalid_published_pairs: usize,
}

impl RuntimeContract {
    fn valid(&self) -> bool {
        self.protocol == PROTOCOL
            && self.max_tokens == 192
            && self.max_batch == MICROBATCH
            && self.domains.iter().map(String::as_str).eq(DOMAIN_IDS)
            && self
                .hypotheses
                .iter()
                .zip(DomainId::ALL)
                .all(|(text, domain)| {
                    text == &format!(
                        "This article substantially discusses the following topic: {}",
                        domain.definition()
                    )
                })
    }
}

impl FrozenCandidate {
    fn validate(&self) -> Result<(), ClassifierError> {
        // Fitting, candidate selection, calibration and final evaluation have
        // distinct partitions. Their sizes and the checkpoint revision are part
        // of the frozen synthetic regression contract.
        if self.schema_version != FROZEN_SCHEMA
            || self.dataset_sha256 != DATASET_SHA256
            || self.base_checkpoint_revision != BASE_CHECKPOINT_REVISION
            || self.split_counts.len() != SPLIT_COUNTS.len()
            || SPLIT_COUNTS
                .iter()
                .any(|(split, count)| self.split_counts.get(*split) != Some(count))
            || !(0.5..1.0).contains(&self.threshold)
            || !valid_asset_hashes(&self.assets)
            || !valid_asset_hashes(&self.provenance.baseline_assets)
            || self.assets.get("config.json") != self.provenance.baseline_assets.get("config.json")
            || self.assets.get("vocab.txt") != self.provenance.baseline_assets.get("vocab.txt")
            || self.provenance.kind != "synthetic"
            || !is_digest(&self.provenance.source_revision, 40)
            || !is_sha256(&self.provenance.calibration_worker_sha256)
            || !self.provenance.runtime_contract.valid()
        {
            return Err(invalid("invalid frozen checkpoint manifest"));
        }
        Ok(())
    }
}

impl Evaluation {
    fn validate(
        &self,
        frozen: &FrozenCandidate,
        frozen_sha256: &str,
    ) -> Result<(), ClassifierError> {
        if self.schema_version != EVALUATION_SCHEMA
            || self.frozen_sha256 != frozen_sha256
            || self.dataset_sha256 != frozen.dataset_sha256
            || self.held_out_rows != 185
            || self.worker_sha256 != frozen.provenance.calibration_worker_sha256
            || !(0.5..1.0).contains(&self.baseline_threshold)
            || self.regression_policy != REGRESSION_POLICY
            || self.baseline_base_checkpoint_revision != BASE_CHECKPOINT_REVISION
            || self.baseline_assets != frozen.provenance.baseline_assets
            || !self.baseline.valid(self.held_out_rows)
            || !self.candidate.valid(self.held_out_rows)
            || self.candidate.out_of_scope_rows != self.baseline.out_of_scope_rows
            || self.candidate.macro_f1 < self.baseline.macro_f1
            || self.candidate.micro_f1 < self.baseline.micro_f1
            || self.candidate.false_out_of_scope_admissions
                > self.baseline.false_out_of_scope_admissions
            || !self.passed
        {
            return Err(invalid("held-out Rust worker evaluation did not pass"));
        }
        Ok(())
    }
}

impl Metrics {
    fn valid(&self, held_out_rows: usize) -> bool {
        if self.out_of_scope_rows != 15
            || self.false_out_of_scope_admissions > self.out_of_scope_rows
            || !self.top_two.valid(held_out_rows)
            || self.per_domain.len() != DOMAIN_IDS.len()
        {
            return false;
        }
        let (mut tp, mut fp, mut false_negatives) = (0, 0, 0);
        let mut sum_f1 = 0.0;
        for counts in self.per_domain.values() {
            if [counts.tp, counts.fp, counts.false_negatives]
                .into_iter()
                .any(|count| count > held_out_rows)
                || counts.tp + counts.fp + counts.false_negatives > held_out_rows
                || !f1_matches(counts.f1, f1(counts.tp, counts.fp, counts.false_negatives))
            {
                return false;
            }
            tp += counts.tp;
            fp += counts.fp;
            false_negatives += counts.false_negatives;
            sum_f1 += counts.f1;
        }
        f1_matches(self.micro_f1, f1(tp, fp, false_negatives))
            && f1_matches(self.macro_f1, sum_f1 / DOMAIN_IDS.len() as f64)
    }
}

impl TopTwoMetrics {
    fn valid(&self, held_out_rows: usize) -> bool {
        self.pair_rows == 30
            && self.triad_rows == 20
            && self.single_domain_rows == 120
            && self.exact_pair_rows <= self.pair_rows
            && self.valid_triad_pairs <= self.triad_rows
            && self.excess_single_domain_admissions <= self.single_domain_rows
            && self.published_pairs <= held_out_rows
            && self.invalid_published_pairs <= self.published_pairs
    }
}

fn f1(tp: usize, fp: usize, false_negatives: usize) -> f64 {
    let denominator = (2 * tp + fp + false_negatives).max(1);
    (2 * tp) as f64 / denominator as f64
}

fn f1_matches(reported: f64, expected: f64) -> bool {
    (0.0..=1.0).contains(&reported) && (reported - expected).abs() <= 1e-12
}

fn valid_asset_hashes(assets: &BTreeMap<String, String>) -> bool {
    assets.len() == MODEL_FILES.len()
        && MODEL_FILES
            .iter()
            .all(|name| assets.get(*name).is_some_and(|digest| is_sha256(digest)))
}

fn is_sha256(value: &str) -> bool {
    is_digest(value, 64)
}

fn is_digest(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(super) fn invalid(detail: &str) -> ClassifierError {
    ClassifierError::InvalidArtifact(format!("Atlas BERT candidate: {detail}"))
}

pub(super) fn directory(home: &Path, frozen_sha256: &str) -> Result<PathBuf, ClassifierError> {
    if !is_sha256(frozen_sha256) {
        return Err(invalid("invalid frozen manifest hash"));
    }
    let root = artifact::root_for(home)?.join(CANDIDATE_DIRECTORY);
    let dir = root.join(frozen_sha256);
    for path in [&root, &dir] {
        let metadata = fs::symlink_metadata(path).map_err(|error| file_error(path, error))?;
        if !metadata.is_dir() {
            return Err(invalid("candidate location must be a real directory"));
        }
    }
    Ok(dir)
}

fn read_json(path: &Path) -> Result<Vec<u8>, ClassifierError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| file_error(path, error))?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_JSON_BYTES {
        return Err(invalid("expected a bounded regular JSON file"));
    }

    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|error| file_error(path, error))?
        .take(MAX_JSON_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| file_error(path, error))?;
    if bytes.len() as u64 != metadata.len() {
        return Err(invalid("JSON file changed while reading"));
    }
    Ok(bytes)
}

pub(super) fn read_active(home: &Path) -> Result<Option<AtlasBertActive>, ClassifierError> {
    let path = artifact::root_for(home)?.join(ATLAS_BERT_ACTIVE_FILE);
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(file_error(&path, error)),
        Ok(_) => (),
    }
    let pointer: AtlasBertActive = serde_json::from_slice(&read_json(&path)?)
        .map_err(|_| invalid("invalid active pointer"))?;
    pointer.validate()?;
    Ok(Some(pointer))
}

fn read_expected_json(
    model_dir: &Path,
    name: &str,
    expected: &str,
) -> Result<Vec<u8>, ClassifierError> {
    if !is_sha256(expected) {
        return Err(invalid("invalid evidence hash"));
    }
    let bytes = read_json(&model_dir.join(name))?;
    if artifact::sha256_hex(&bytes) != expected {
        return Err(invalid("evidence hash mismatch"));
    }
    Ok(bytes)
}

pub(super) fn validate_evidence_hashes(
    model_dir: &Path,
    frozen_sha256: &str,
    evaluation_sha256: &str,
) -> Result<(), ClassifierError> {
    read_expected_json(model_dir, FROZEN_FILE, frozen_sha256)?;
    read_expected_json(model_dir, EVALUATION_FILE, evaluation_sha256)?;
    Ok(())
}

pub(super) fn manifest_hash(base: &str, frozen: &str, evaluation: &str) -> String {
    artifact::sha256_hex(
        format!("atlas-bert-candidate-manifest-v1\n{base}\n{frozen}\n{evaluation}").as_bytes(),
    )
}

fn check_checkpoint(
    directory: &Path,
    hashes: &BTreeMap<String, String>,
) -> Result<String, ClassifierError> {
    let mut digests = String::with_capacity(MODEL_FILES.len() * 64);
    for name in MODEL_FILES {
        let path = directory.join(name);
        let metadata = fs::symlink_metadata(&path).map_err(|error| file_error(&path, error))?;
        if !metadata.is_file() {
            return Err(invalid("expected a regular checkpoint asset"));
        }
        let digest = file_sha256(&path)?;
        if hashes.get(name) != Some(&digest) {
            return Err(invalid("checkpoint asset hash mismatch"));
        }
        digests.push_str(&digest);
    }
    Ok(artifact::sha256_hex(digests.as_bytes()))
}

#[derive(PartialEq, Eq)]
pub(super) enum BaselineCheck {
    Activation,
    PinnedReplay,
}

pub(super) fn load(
    home: &Path,
    frozen_sha256: &str,
    evaluation_sha256: &str,
    baseline_check: BaselineCheck,
) -> Result<Bundle, ClassifierError> {
    let model_dir = self::directory(home, frozen_sha256)?;
    let frozen: FrozenCandidate =
        serde_json::from_slice(&read_expected_json(&model_dir, FROZEN_FILE, frozen_sha256)?)
            .map_err(|_| invalid("invalid frozen JSON"))?;
    frozen.validate()?;
    let evaluation: Evaluation = serde_json::from_slice(&read_expected_json(
        &model_dir,
        EVALUATION_FILE,
        evaluation_sha256,
    )?)
    .map_err(|_| invalid("invalid evaluation JSON"))?;
    evaluation.validate(&frozen, frozen_sha256)?;
    let mut config = Config::load(home)?;
    if baseline_check == BaselineCheck::Activation {
        // Initial activation needs evidence for the exact configured baseline.
        // After activation, exact candidate replay depends only on its own
        // immutable files and configured worker.
        if config.threshold.to_bits() != evaluation.baseline_threshold.to_bits() {
            return Err(invalid(
                "evaluation baseline threshold differs from deployment",
            ));
        }
        check_checkpoint(&config.model_dir, &evaluation.baseline_assets)?;
    }
    let artifact_sha256 = check_checkpoint(&model_dir, &frozen.assets)?;
    config.model_dir = model_dir;
    config.threshold = frozen.threshold;
    let base = bundle_manifest_hash(&config, &artifact_sha256)?;
    Ok(Bundle::new(
        config,
        artifact_sha256,
        manifest_hash(&base, frozen_sha256, evaluation_sha256),
        Provenance::FineTunedAtlasBert {
            dataset_sha256: frozen.dataset_sha256,
            frozen_sha256: frozen_sha256.to_owned(),
            evaluation_sha256: evaluation_sha256.to_owned(),
        },
    ))
}

/// Checks an operator-staged candidate's frozen inputs and held-out evidence,
/// then loads and scores that checkpoint with the configured Rust worker.
/// Publication of the active pointer is the caller's separate fenced operation.
///
/// # Errors
/// Returns a classifier error when candidate or baseline integrity fails,
/// the candidate regresses on the held-out corpus, or the worker cannot score.
pub async fn verify_atlas_bert_candidate(
    home: &Path,
    frozen_sha256: &str,
    evaluation_sha256: &str,
    control: &WorkControl,
) -> Result<AtlasBertActive, ClassifierError> {
    control.check_active()?;
    let home = home.to_path_buf();
    let frozen_sha256 = frozen_sha256.to_owned();
    let evaluation_sha256 = evaluation_sha256.to_owned();
    let bundle = tokio::task::spawn_blocking(move || {
        load(
            &home,
            &frozen_sha256,
            &evaluation_sha256,
            BaselineCheck::Activation,
        )
    })
    .await
    .map_err(|_| ClassifierError::WorkerStopped)??;
    let mut worker = controlled(Worker::start(&bundle.config), control, STARTUP_TIMEOUT).await?;
    let inputs = [
        "Local election authorities released the official municipal vote count.".to_owned(),
        "The municipal water board reported unusually low river flow.".to_owned(),
    ];
    controlled(worker.score(&inputs), control, MICROBATCH_TIMEOUT).await?;
    control.check_active()?;
    Ok(PinIdentity {
        snapshot: bundle.snapshot,
        manifest_sha256: bundle.manifest_sha256,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classifier::ModelSnapshot;

    mod fixture {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/classifier/atlas_bert_candidate.rs"
        ));
    }

    use self::fixture::StagedCandidate;

    impl StagedCandidate {
        fn validate(&self) -> Result<Bundle, ClassifierError> {
            load(
                self.home.path(),
                &self.frozen_sha256,
                &self.evaluation_sha256,
                BaselineCheck::Activation,
            )
        }

        fn publish_pointer(&self, snapshot: ModelSnapshot, manifest_sha256: String) {
            let root = artifact::root_for(self.home.path()).expect("load test artifact root");
            let pointer = serde_json::to_vec(&PinIdentity {
                snapshot,
                manifest_sha256,
            })
            .expect("encode fixture pointer");
            fs::write(root.join(ATLAS_BERT_ACTIVE_FILE), pointer).expect("write fixture pointer");
        }
    }

    #[test]
    fn fresh_and_saved_pins_select_their_exact_checkpoints() {
        let fixture = StagedCandidate::new(0.8);
        let baseline = Bundle::load(fixture.home.path(), None).expect("load the original default");
        assert_eq!(baseline.snapshot.provenance, Provenance::ZeroShot);
        let candidate = fixture.validate().expect("validate frozen candidate");
        assert_eq!(candidate.config.threshold, 0.75);
        assert_ne!(candidate.snapshot.model_id, baseline.snapshot.model_id);
        fixture.publish_pointer(
            candidate.snapshot.clone(),
            candidate.manifest_sha256.clone(),
        );

        let fresh = Bundle::load(fixture.home.path(), None).expect("load the activated model");
        assert_eq!(fresh.snapshot, candidate.snapshot);
        assert_eq!(fresh.manifest_sha256, candidate.manifest_sha256);
        let saved = Bundle::load(fixture.home.path(), Some(baseline.snapshot.clone()))
            .expect("restore saved baseline pin after activating a candidate");
        assert_eq!(saved.snapshot, baseline.snapshot);
        assert_eq!(saved.manifest_sha256, baseline.manifest_sha256);

        fs::write(
            fixture.model_dir.join("rust_model.ot"),
            b"substituted weights",
        )
        .expect("alter fixture candidate");
        assert!(
            Bundle::load(fixture.home.path(), None).is_err(),
            "a broken active pin must never fall back to the baseline"
        );
        assert!(
            Bundle::load(fixture.home.path(), Some(candidate.snapshot)).is_err(),
            "an exact saved pin must reject substituted weights"
        );
    }

    #[test]
    fn candidate_requires_bound_evaluation_and_a_nonregressing_score() {
        assert!(
            StagedCandidate::new(0.6).validate().is_err(),
            "a positive report flag cannot override measured regression"
        );
        let fixture = StagedCandidate::new(0.8);
        fs::write(fixture.model_dir.join(EVALUATION_FILE), b"{}").expect("alter held-out report");
        assert!(
            fixture.validate().is_err(),
            "changed held-out evidence must invalidate the candidate"
        );
    }
}
