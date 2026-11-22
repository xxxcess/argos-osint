//! Local Rust-BERT worker and immutable Atlas checkpoint identity.

mod candidate;
pub(crate) mod cycle_metrics;
pub(super) mod reviewed;
use std::fs::File;
use std::future::Future;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::runtime::Handle;

pub(super) use self::candidate::ATLAS_BERT_ACTIVE_FILE;
pub use self::candidate::{verify_atlas_bert_candidate, AtlasBertActive};

use super::{
    artifact, calibration, ClassificationReceipt, ClassifierError, ClassifierService,
    ClassifierStatus, ClassifyInput, DomainId, GateSet, InferenceEngine, ModelId, ModelSnapshot,
    Provenance, RawDomainScore, WorkControl,
};

const PROTOCOL: &str = "argos-atlas-bert-v1";
const MODEL_FILES: [&str; 3] = ["config.json", "vocab.txt", "rust_model.ot"];
const MICROBATCH: usize = 4;
const MAX_REPLY_BYTES: u64 = 8192;
pub(crate) const STARTUP_TIMEOUT: Duration = Duration::from_secs(60);
const MICROBATCH_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    worker_path: PathBuf,
    model_dir: PathBuf,
    #[serde(default = "default_threshold")]
    threshold: f64,
}

fn default_threshold() -> f64 {
    0.8
}

impl Config {
    fn load(home: &Path) -> Result<Self, ClassifierError> {
        let path = artifact::root_for(home)?.join("zero-shot.json");
        let file = File::open(&path).map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                ClassifierError::MissingZeroShotConfig(path.clone())
            } else {
                file_error(&path, error)
            }
        })?;
        let mut bytes = Vec::new();
        file.take(16385)
            .read_to_end(&mut bytes)
            .map_err(|error| file_error(&path, error))?;
        if bytes.len() > 16384 {
            return Err(ClassifierError::InvalidArtifact(
                "zero-shot config exceeds 16 KiB".into(),
            ));
        }
        let config: Self = serde_json::from_slice(&bytes).map_err(|error| {
            ClassifierError::InvalidArtifact(format!("zero-shot config: {error}"))
        })?;
        if !config.worker_path.is_absolute()
            || !config.model_dir.is_absolute()
            || !(0.5..1.0).contains(&config.threshold)
        {
            return Err(ClassifierError::InvalidArtifact(
                "zero-shot config requires absolute paths and a threshold in [0.5, 1)".into(),
            ));
        }
        Ok(config)
    }
}

/// Check the worker executable and model inputs without initializing inference.
pub(super) fn check_configured_assets(
    home: &Path,
) -> Result<Option<AtlasBertActive>, ClassifierError> {
    let mut config = Config::load(home)?;
    let active = candidate::read_active(home)?;
    if let Some(active) = &active {
        if let Provenance::FineTunedAtlasBert { frozen_sha256, .. } = &active.snapshot.provenance {
            config.model_dir = candidate::directory(home, frozen_sha256)?;
        } else {
            return Err(candidate::invalid(
                "active pointer requires a fine-tuned checkpoint",
            ));
        }
    }
    let worker = asset_metadata(&config.worker_path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if worker.permissions().mode() & 0o111 == 0 {
            return Err(ClassifierError::InvalidArtifact(
                "configured Atlas worker is not executable".into(),
            ));
        }
    }
    #[cfg(not(unix))]
    let _ = worker;
    for file in MODEL_FILES {
        asset_metadata(&config.model_dir.join(file))?;
    }
    Ok(active)
}

pub(super) struct Bundle {
    config: Config,
    pub(super) snapshot: ModelSnapshot,
    pub(super) manifest_sha256: String,
    reviewed_calibration: Option<reviewed::RecordedCalibration>,
}

impl Bundle {
    pub(super) fn load(
        home: &Path,
        requested: Option<ModelSnapshot>,
    ) -> Result<Self, ClassifierError> {
        match requested {
            Some(snapshot) => Self::load_exact(home, snapshot),
            None => match candidate::read_active(home)? {
                Some(active) => Self::load_active(home, active),
                None => Self::load_baseline(Config::load(home)?),
            },
        }
    }

    fn load_active(home: &Path, active: AtlasBertActive) -> Result<Self, ClassifierError> {
        if !matches!(
            active.snapshot.provenance,
            Provenance::FineTunedAtlasBert { .. }
        ) {
            return Err(candidate::invalid(
                "active pointer requires a fine-tuned checkpoint",
            ));
        }
        let bundle = Self::load_exact(home, active.snapshot)?;
        if bundle.manifest_sha256 != active.manifest_sha256 {
            return Err(candidate::invalid(
                "active checkpoint manifest hash mismatch",
            ));
        }
        Ok(bundle)
    }

    fn load_baseline(config: Config) -> Result<Self, ClassifierError> {
        let (artifact_sha256, manifest_sha256) = bundle_hashes(&config)?;
        Ok(Self::new(
            config,
            artifact_sha256,
            manifest_sha256,
            Provenance::ZeroShot,
        ))
    }

    fn load_exact(home: &Path, requested: ModelSnapshot) -> Result<Self, ClassifierError> {
        let mut bundle = match &requested.provenance {
            Provenance::ZeroShot => Self::load_baseline(Config::load(home)?)?,
            Provenance::FineTunedAtlasBert {
                frozen_sha256,
                evaluation_sha256,
                reviewed_calibration_sha256,
                ..
            } => candidate::load(
                home,
                frozen_sha256,
                evaluation_sha256,
                reviewed_calibration_sha256.as_deref(),
                candidate::BaselineCheck::PinnedReplay,
            )?,
            _ => {
                return Err(ClassifierError::Incompatible(
                    "expected an Atlas BERT checkpoint".into(),
                ))
            }
        };
        bundle
            .snapshot
            .activated_at
            .clone_from(&requested.activated_at);
        if bundle.snapshot != requested
            || chrono::DateTime::parse_from_rfc3339(&requested.activated_at).is_err()
        {
            return Err(ClassifierError::Incompatible(
                "saved Atlas BERT pin requires the original worker, model files and threshold"
                    .into(),
            ));
        }
        Ok(bundle)
    }

    fn new(
        config: Config,
        artifact_sha256: String,
        manifest_sha256: String,
        provenance: Provenance,
    ) -> Self {
        let mut snapshot = ModelSnapshot::new(
            format!("bert-{}", &manifest_sha256[..59]),
            artifact_sha256,
            provenance,
        );
        snapshot.algorithm_version = super::ZERO_SHOT_ALGORITHM_VERSION.into();
        Self {
            config,
            snapshot,
            manifest_sha256,
            reviewed_calibration: None,
        }
    }

    pub(super) fn start(
        self,
        runtime: Handle,
        control: &WorkControl,
    ) -> Result<ClassifierService, ClassifierError> {
        let id = ModelId::parse(&self.snapshot.model_id)
            .ok_or_else(|| ClassifierError::InvalidArtifact("invalid zero-shot model ID".into()))?;
        let gates = calibration::zero_shot_gates(self.config.threshold);
        let manifest_sha256 = self.manifest_sha256.clone();
        let snapshot = self.snapshot.clone();
        let process = runtime.block_on(controlled(
            Worker::start(&self.config),
            control,
            STARTUP_TIMEOUT,
        ))?;
        Ok(ClassifierService {
            status: ClassifierStatus::Ready,
            unavailable_reason: None,
            pinned: Some(snapshot),
            engine: std::sync::Mutex::new(Some(Box::new(Engine {
                runtime,
                bundle: self,
                worker: Some(process),
                id,
                gates: gates.clone(),
            }))),
            metrics: std::sync::Mutex::new(Default::default()),
            zero_shot_policy: Some((manifest_sha256, gates)),
        })
    }
}

pub(super) fn active_atlas_bert_revision(home: &Path) -> Result<String, ClassifierError> {
    let active = Bundle::load(home, None)?;
    AtlasBertActive {
        snapshot: active.snapshot,
        manifest_sha256: active.manifest_sha256,
    }
    .activation_revision()
}

pub(super) fn validate_atlas_bert_identity(
    home: &Path,
    identity: &AtlasBertActive,
) -> Result<(), ClassifierError> {
    identity.validate()?;
    let bundle = Bundle::load(home, Some(identity.snapshot.clone()))?;
    if bundle.manifest_sha256 != identity.manifest_sha256 {
        return Err(candidate::invalid(
            "staged checkpoint manifest hash mismatch",
        ));
    }
    Ok(())
}

fn file_error(path: &Path, error: io::Error) -> ClassifierError {
    ClassifierError::Io(format!("{}: {error}", path.display()))
}

const MAX_ASSET_BYTES: u64 = 1024 * 1024 * 1024;

fn asset_metadata(path: &Path) -> Result<std::fs::Metadata, ClassifierError> {
    let metadata = std::fs::metadata(path).map_err(|error| file_error(path, error))?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_ASSET_BYTES {
        return Err(ClassifierError::InvalidArtifact(format!(
            "zero-shot asset must be a nonempty file at most 1 GiB: {}",
            path.display()
        )));
    }
    Ok(metadata)
}

fn file_sha256(path: &Path) -> Result<String, ClassifierError> {
    let metadata = asset_metadata(path)?;
    let mut file = File::open(path)
        .map_err(|error| file_error(path, error))?
        .take(MAX_ASSET_BYTES + 1);
    let mut hash = Sha256::new();
    let bytes = io::copy(&mut file, &mut hash).map_err(|error| file_error(path, error))?;
    if bytes != metadata.len() {
        return Err(ClassifierError::InvalidArtifact(
            "zero-shot asset size changed".into(),
        ));
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn bundle_hashes(config: &Config) -> Result<(String, String), ClassifierError> {
    let mut assets = String::new();
    for file in MODEL_FILES {
        assets.push_str(&file_sha256(&config.model_dir.join(file))?);
    }
    let artifact_sha256 = artifact::sha256_hex(assets.as_bytes());
    let manifest_sha256 = bundle_manifest_hash(config, &artifact_sha256)?;
    Ok((artifact_sha256, manifest_sha256))
}

fn bundle_manifest_hash(config: &Config, artifact_sha256: &str) -> Result<String, ClassifierError> {
    Ok(artifact::sha256_hex(
        format!(
            "{PROTOCOL}\n{}\n{}\n{}\n{}\n{artifact_sha256}\n{}\n{:016x}",
            super::ZERO_SHOT_ALGORITHM_VERSION,
            super::TAXONOMY_VERSION,
            super::PREPROCESSING_VERSION,
            super::ADMISSION_POLICY_VERSION,
            file_sha256(&config.worker_path)?,
            config.threshold.to_bits()
        )
        .as_bytes(),
    ))
}

struct Worker {
    _child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Ready {
    protocol: String,
}

#[derive(Serialize)]
struct Request<'a> {
    texts: &'a [String],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Response {
    scores: Vec<[f64; 6]>,
}

fn protocol_error(code: &str) -> ClassifierError {
    ClassifierError::BridgeProtocol {
        operation: PROTOCOL.into(),
        code: code.into(),
    }
}

impl Worker {
    async fn start(config: &Config) -> Result<Self, ClassifierError> {
        let mut command = Command::new(&config.worker_path);
        // The score worker only needs executable and dynamic library discovery.
        // Gateway and service credentials stay in the supervising process.
        command.env_clear();
        for name in ["PATH", "LD_LIBRARY_PATH", "DYLD_LIBRARY_PATH"] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        let mut child = command
            .arg("serve")
            .arg(&config.model_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| file_error(&config.worker_path, error))?;
        let mut worker = Self {
            input: child
                .stdin
                .take()
                .ok_or_else(|| protocol_error("stdin_missing"))?,
            output: BufReader::new(
                child
                    .stdout
                    .take()
                    .ok_or_else(|| protocol_error("stdout_missing"))?,
            ),
            _child: child,
        };
        let ready: Ready = worker.read().await?;
        if ready.protocol != PROTOCOL {
            return Err(ClassifierError::Incompatible(
                "zero-shot worker protocol mismatch".into(),
            ));
        }
        Ok(worker)
    }

    async fn read<T: serde::de::DeserializeOwned>(&mut self) -> Result<T, ClassifierError> {
        let mut line = Vec::new();
        (&mut self.output)
            .take(MAX_REPLY_BYTES)
            .read_until(b'\n', &mut line)
            .await
            .map_err(|_| ClassifierError::WorkerStopped)?;
        if line.is_empty() {
            return Err(ClassifierError::WorkerStopped);
        }
        if line.last() != Some(&b'\n') {
            return Err(protocol_error("incomplete_or_oversized_reply"));
        }
        serde_json::from_slice(&line).map_err(|_| protocol_error("invalid_json_reply"))
    }

    async fn score(&mut self, texts: &[String]) -> Result<Vec<[f64; 6]>, ClassifierError> {
        let mut line = serde_json::to_vec(&Request { texts })
            .map_err(|_| protocol_error("invalid_request"))?;
        line.push(b'\n');
        self.input
            .write_all(&line)
            .await
            .map_err(|_| ClassifierError::WorkerStopped)?;
        let response: Response = self.read().await?;
        if response.scores.len() != texts.len()
            || response
                .scores
                .iter()
                .flatten()
                .any(|score| !(0.0..=1.0).contains(score))
        {
            return Err(protocol_error("invalid_score_batch"));
        }
        Ok(response.scores)
    }
}

async fn controlled<T>(
    operation: impl Future<Output = Result<T, ClassifierError>>,
    control: &WorkControl,
    budget: Duration,
) -> Result<T, ClassifierError> {
    control.check_active()?;
    let deadline = control
        .overall_deadline
        .min(std::time::Instant::now() + budget);
    tokio::pin!(operation);
    loop {
        tokio::select! {
            result = &mut operation => return result,
            () = tokio::time::sleep_until(deadline.into()) => return Err(ClassifierError::Timeout),
            () = tokio::time::sleep(Duration::from_millis(25)) => control.check_active()?,
        }
    }
}

fn raw_scores(scores: [f64; 6]) -> [RawDomainScore; 6] {
    std::array::from_fn(|index| {
        // Keep receipt log odds finite at exact softmax endpoints.
        let score = scores[index].clamp(1e-12, 1.0 - 1e-12);
        let present = score.ln();
        let absent = (-score).ln_1p();
        RawDomainScore {
            domain: DomainId::ALL[index],
            present_log_score: present,
            absent_log_score: absent,
            raw_log_odds: present - absent,
            known_feature_count: 0,
            oov_count: 0,
        }
    })
}

struct Engine {
    runtime: Handle,
    bundle: Bundle,
    worker: Option<Worker>,
    id: ModelId,
    gates: GateSet,
}

impl Engine {
    fn score(
        &mut self,
        texts: &[String],
        control: &WorkControl,
    ) -> Result<Vec<[f64; 6]>, ClassifierError> {
        if self.worker.is_none() {
            let (artifact, manifest) = bundle_hashes(&self.bundle.config)?;
            let manifest = match &self.bundle.snapshot.provenance {
                Provenance::FineTunedAtlasBert {
                    frozen_sha256,
                    evaluation_sha256,
                    reviewed_calibration_sha256,
                    ..
                } => {
                    candidate::validate_evidence_hashes(
                        &self.bundle.config.model_dir,
                        frozen_sha256,
                        evaluation_sha256,
                        reviewed_calibration_sha256.as_deref(),
                    )?;
                    candidate::manifest_hash(
                        &manifest,
                        frozen_sha256,
                        evaluation_sha256,
                        reviewed_calibration_sha256.as_deref(),
                    )
                }
                _ => manifest,
            };
            if artifact != self.bundle.snapshot.artifact_sha256
                || manifest != self.bundle.manifest_sha256
            {
                return Err(ClassifierError::Incompatible(
                    "zero-shot bundle changed while pinned".into(),
                ));
            }
            self.worker = Some(self.runtime.block_on(controlled(
                Worker::start(&self.bundle.config),
                control,
                STARTUP_TIMEOUT,
            ))?);
        }
        let worker = self.worker.as_mut().ok_or(ClassifierError::WorkerStopped)?;
        let result =
            self.runtime
                .block_on(controlled(worker.score(texts), control, MICROBATCH_TIMEOUT));
        if result.is_err() {
            // The stream can contain an unread reply after cancellation. Close
            // the child and establish a fresh stream before any retry.
            self.worker.take();
        }
        result
    }
}

fn worker_text(input: &ClassifyInput) -> Option<String> {
    calibration::input_rejection(input)
        .is_none()
        .then(|| format!("{}\n{}", input.title, input.description))
}

impl InferenceEngine for Engine {
    fn classify_batch(
        &mut self,
        inputs: &[ClassifyInput],
    ) -> Result<Vec<ClassificationReceipt>, ClassifierError> {
        self.classify_batch_with_control(inputs, &WorkControl::new())
    }

    fn classify_batch_with_control(
        &mut self,
        inputs: &[ClassifyInput],
        control: &WorkControl,
    ) -> Result<Vec<ClassificationReceipt>, ClassifierError> {
        let mut receipts = Vec::with_capacity(inputs.len());
        for batch in inputs.chunks(MICROBATCH) {
            control.check_active()?;
            let mut texts = Vec::new();
            let mut score_input = Vec::with_capacity(batch.len());
            for input in batch {
                let text = worker_text(input);
                score_input.push(text.is_some());
                texts.extend(text);
            }
            let scores = if texts.is_empty() {
                Vec::new()
            } else {
                self.score(&texts, control)?
            };
            let mut scores = scores.into_iter();
            for (input, usable) in batch.iter().zip(score_input) {
                let scores = if usable {
                    scores
                        .next()
                        .ok_or_else(|| protocol_error("missing_input_scores"))?
                } else {
                    [0.5; 6]
                };
                receipts.push(calibration::apply_gates(
                    input,
                    &raw_scores(scores),
                    &self.id,
                    &self.gates,
                )?);
            }
        }
        Ok(receipts)
    }

    fn serialize_model(&mut self) -> Result<String, ClassifierError> {
        Err(ClassifierError::Unavailable)
    }

    fn restore_model(&mut self, _: &str) -> Result<(), ClassifierError> {
        Err(ClassifierError::Unavailable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classifier::RejectionReason;

    #[test]
    fn zero_shot_applies_threshold_without_lexical_support() {
        let model_id = ModelId::parse(&format!("bert-{}", "a".repeat(59))).unwrap();
        let input =
            ClassifyInput::new("item", "A report", "Article description", Some("en".into()));
        let gates = calibration::zero_shot_gates(0.8);
        let raw = raw_scores([0.95, 0.82, 0.81, 0.79, 0.0, 0.2]);
        let receipt = calibration::apply_gates(&input, &raw, &model_id, &gates).unwrap();
        receipt.validate().unwrap();
        assert_eq!(receipt.selected_domains, &DomainId::ALL[..2]);
        assert!(receipt
            .scores
            .iter()
            .all(|score| score.informative_features == 0));

        let raw = raw_scores([0.95, 0.79, 0.75, 0.6, 0.0, 0.2]);
        let receipt = calibration::apply_gates(&input, &raw, &model_id, &gates).unwrap();
        assert_eq!(receipt.selected_domains, &DomainId::ALL[..1]);

        let unsupported =
            ClassifyInput::new("item", "A report", "Article description", Some("fr".into()));
        let receipt = calibration::apply_gates(&unsupported, &raw, &model_id, &gates).unwrap();
        receipt.validate().unwrap();
        assert_eq!(
            receipt.rejection_reason,
            Some(RejectionReason::UnsupportedLanguage)
        );

        let raw = raw_scores([0.79; 6]);
        let receipt = calibration::apply_gates(&input, &raw, &model_id, &gates).unwrap();
        receipt.validate().unwrap();
        assert_eq!(receipt.rejection_reason, Some(RejectionReason::OutOfScope));
    }
}
