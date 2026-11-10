//! Private Fleetworks transport for immutable, pinned Rust-BERT inference.

use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use reqwest::header::{HeaderValue, AUTHORIZATION};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tokio::runtime::Handle;
use tokio::sync::OnceCell;
use url::Url;

use super::readiness::PinnedClassifier;
use super::{
    calibration, ClassificationReceipt, ClassifierError, ClassifierService, ClassifierStatus,
    ClassifyInput, GateSet, InferenceEngine, ModelId, ModelSnapshot, Provenance, WorkControl,
    ADMISSION_POLICY_VERSION, PREPROCESSING_VERSION, TAXONOMY_VERSION, ZERO_SHOT_ALGORITHM_VERSION,
};

pub const VERSION: u16 = 1;
pub const READY_PATH: &str = "/api/v1/health/ready";
pub const PIN_PATH: &str = "/api/v1/classifier/pin";
pub const VALIDATE_PATH: &str = "/api/v1/classifier/validate";
pub const INFER_PATH: &str = "/api/v1/classifier/infer";
pub const MAX_REQUEST_BYTES: usize = 1024 * 1024;
pub const MAX_BATCH_ITEMS: usize = 4;
const MAX_RESPONSE_BYTES: usize = 128 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(90);
pub(super) const OPERATION: &str = "fleetworks-classifier-v1";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PinIdentity {
    pub snapshot: ModelSnapshot,
    pub manifest_sha256: String,
}

impl PinIdentity {
    pub fn validate(&self) -> Result<(), ClassifierError> {
        validate_snapshot(&self.snapshot)?;
        if !is_sha256(&self.manifest_sha256)
            || self.snapshot.model_id.strip_prefix("bert-") != self.manifest_sha256.get(..59)
        {
            return Err(protocol_error("invalid_pin_identity"));
        }
        Ok(())
    }

    /// Activation timestamps are preserved in receipts' owning cycle snapshot.
    /// They do not change which worker can serve the immutable bundle.
    pub fn matches_bundle(&self, pin: &PinnedClassifier) -> bool {
        let mut snapshot = pin.snapshot.clone();
        snapshot
            .activated_at
            .clone_from(&self.snapshot.activated_at);
        self.manifest_sha256 == pin.manifest_sha256 && self.snapshot == snapshot
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PinRequest {
    pub version: u16,
    pub requested: Option<ModelSnapshot>,
}

impl PinRequest {
    pub fn validate(&self) -> Result<(), ClassifierError> {
        if self.version != VERSION {
            return Err(protocol_error("unsupported_version"));
        }
        if let Some(snapshot) = &self.requested {
            validate_snapshot(snapshot)?;
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PinResponse {
    pub version: u16,
    pub identity: PinIdentity,
    // Preserve the exact threshold that participates in the manifest hash.
    pub threshold_bits: u64,
}

/// Exact identity proof for derivative-only work that needs no runtime.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValidateRequest {
    pub version: u16,
    pub identity: PinIdentity,
}

impl ValidateRequest {
    pub fn validate(&self) -> Result<(), ClassifierError> {
        if self.version != VERSION {
            return Err(protocol_error("unsupported_version"));
        }
        self.identity.validate()
    }
}

impl PinResponse {
    pub fn from_local(pin: &PinnedClassifier) -> Result<Self, ClassifierError> {
        let (manifest, gates) = pin
            .service
            .zero_shot_policy
            .as_ref()
            .ok_or(ClassifierError::Unavailable)?;
        if manifest != &pin.manifest_sha256 {
            return Err(protocol_error("invalid_local_pin"));
        }
        let response = Self {
            version: VERSION,
            identity: PinIdentity {
                snapshot: pin.snapshot.clone(),
                manifest_sha256: manifest.clone(),
            },
            threshold_bits: gates.domains[0].threshold.to_bits(),
        };
        if &response.validate(None)? != gates {
            return Err(protocol_error("invalid_local_gates"));
        }
        Ok(response)
    }

    fn validate(&self, requested: Option<&ModelSnapshot>) -> Result<GateSet, ClassifierError> {
        self.identity.validate()?;
        let threshold = f64::from_bits(self.threshold_bits);
        if self.version != VERSION
            || requested.is_some_and(|snapshot| snapshot != &self.identity.snapshot)
            || !(0.5..1.0).contains(&threshold)
        {
            return Err(protocol_error("invalid_pin_response"));
        }
        Ok(calibration::zero_shot_gates(threshold))
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InferRequest {
    pub version: u16,
    pub identity: PinIdentity,
    pub inputs: Vec<ClassifyInput>,
}

impl InferRequest {
    pub fn validate(&self) -> Result<(), ClassifierError> {
        if self.version != VERSION || self.inputs.is_empty() || self.inputs.len() > MAX_BATCH_ITEMS
        {
            return Err(protocol_error("invalid_inference_request"));
        }
        self.identity.validate()?;
        for input in &self.inputs {
            if input.occurrence_id.is_empty() {
                return Err(protocol_error("invalid_occurrence_id"));
            }
            super::validate_inference_input(input)?;
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InferResponse {
    pub version: u16,
    pub identity: PinIdentity,
    pub receipts: Vec<ClassificationReceipt>,
}

fn validate_snapshot(snapshot: &ModelSnapshot) -> Result<(), ClassifierError> {
    let valid_model_id = snapshot
        .model_id
        .strip_prefix("bert-")
        .is_some_and(|hash| hash.len() == 59 && is_hex(hash));
    if !valid_model_id
        || !is_sha256(&snapshot.artifact_sha256)
        || snapshot.taxonomy_version != TAXONOMY_VERSION
        || snapshot.algorithm_version != ZERO_SHOT_ALGORITHM_VERSION
        || snapshot.preprocessing_version != PREPROCESSING_VERSION
        || snapshot.policy_version != ADMISSION_POLICY_VERSION
        || snapshot.provenance != Provenance::ZeroShot
        || chrono::DateTime::parse_from_rfc3339(&snapshot.activated_at).is_err()
    {
        return Err(protocol_error("invalid_snapshot"));
    }
    Ok(())
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && is_hex(value)
}

fn is_hex(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn protocol_error(code: &str) -> ClassifierError {
    ClassifierError::BridgeProtocol {
        operation: OPERATION.into(),
        code: code.into(),
    }
}

fn validate_receipt(
    receipt: &ClassificationReceipt,
    input: &ClassifyInput,
    model: &ModelId,
    gates: &GateSet,
) -> Result<(), ClassifierError> {
    receipt.validate()?;
    if receipt.model_id != model.as_str()
        || receipt.input_hash != input.input_hash
        || receipt.occurrence_id != input.occurrence_id
    {
        return Err(protocol_error("invalid_receipt_identity"));
    }
    let raw = receipt
        .raw_diagnostics
        .as_ref()
        .filter(|raw| {
            raw.provider.is_none()
                && raw.support.is_empty()
                && raw
                    .domains
                    .iter()
                    .all(|score| score.known_feature_count == 0 && score.oov_count == 0)
        })
        .ok_or_else(|| protocol_error("invalid_zero_shot_diagnostics"))?;
    let expected = calibration::apply_gates(input, &raw.domains, model, gates)?;
    if receipt.selected_domains != expected.selected_domains
        || receipt.primary_rank_ambiguous != expected.primary_rank_ambiguous
        || receipt.rejection_reason != expected.rejection_reason
        || receipt
            .scores
            .iter()
            .zip(expected.scores)
            .any(|(actual, expected)| {
                actual.passes_gate != expected.passes_gate
                    || (actual.threshold - expected.threshold).abs() > 1e-12
                    || (actual.calibrated_score - expected.calibrated_score).abs() > 1e-12
            })
    {
        return Err(protocol_error("invalid_zero_shot_decision"));
    }
    Ok(())
}

/// Client for the explicitly configured private classifier service.
pub struct Client {
    origin: Url,
    authorization: HeaderValue,
    http: reqwest::Client,
}

impl Client {
    pub fn new(origin: &str, token: &str) -> Result<Self, ClassifierError> {
        let configuration_error = || protocol_error("remote_classifier_configuration");
        let origin = Url::parse(origin).map_err(|_| configuration_error())?;
        if !matches!(origin.scheme(), "http" | "https")
            || origin.host().is_none()
            || !origin.username().is_empty()
            || origin.password().is_some()
            || origin.path() != "/"
            || origin.query().is_some()
            || origin.fragment().is_some()
            || !(32..=256).contains(&token.len())
            || token.trim_end_matches('=').is_empty()
            || !token.trim_end_matches('=').bytes().all(|byte| {
                byte.is_ascii_alphanumeric()
                    || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'+' | b'/')
            })
        {
            return Err(configuration_error());
        }
        let mut authorization =
            HeaderValue::from_str(&format!("Bearer {token}")).map_err(|_| configuration_error())?;
        authorization.set_sensitive(true);
        let http = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|_| configuration_error())?;
        Ok(Self {
            origin,
            authorization,
            http,
        })
    }

    /// Pins a classifier that uses HTTP on the caller's existing blocking
    /// inference thread, keeping Atlas's receipt and inference APIs intact.
    pub async fn pin(
        self: &Arc<Self>,
        requested: Option<&ModelSnapshot>,
    ) -> Result<PinnedClassifier, ClassifierError> {
        let response = self.request_pin(requested).await?;
        let gates = response.validate(requested)?;
        let identity = response.identity;
        let snapshot = identity.snapshot.clone();
        let manifest_sha256 = identity.manifest_sha256.clone();
        let model = ModelId::parse(&snapshot.model_id)
            .ok_or_else(|| protocol_error("invalid_pin_model_id"))?;
        let service = Arc::new(ClassifierService {
            status: ClassifierStatus::Ready,
            unavailable_reason: None,
            pinned: Some(snapshot.clone()),
            engine: Mutex::new(Some(Box::new(Engine {
                runtime: Handle::current(),
                client: self.clone(),
                identity,
                threshold_bits: response.threshold_bits,
                model,
                gates: gates.clone(),
            }))),
            metrics: Mutex::new(Default::default()),
            zero_shot_policy: Some((manifest_sha256.clone(), gates)),
        });
        Ok(PinnedClassifier {
            snapshot,
            manifest_sha256,
            service,
        })
    }

    async fn request_pin(
        &self,
        requested: Option<&ModelSnapshot>,
    ) -> Result<PinResponse, ClassifierError> {
        let request = PinRequest {
            version: VERSION,
            requested: requested.cloned(),
        };
        request.validate()?;
        self.post(PIN_PATH, &request).await
    }

    async fn post<T: DeserializeOwned>(
        &self,
        path: &str,
        body: &impl Serialize,
    ) -> Result<T, ClassifierError> {
        let endpoint = self
            .origin
            .join(path)
            .map_err(|_| protocol_error("invalid_remote_route"))?;
        let mut response = self
            .http
            .post(endpoint)
            .header(AUTHORIZATION, self.authorization.clone())
            .json(body)
            .send()
            .await
            .map_err(transport_error)?;
        match response.status().as_u16() {
            200 => {}
            400 | 413 | 422 => return Err(protocol_error("remote_classifier_invalid_request")),
            401 | 403 => return Err(protocol_error("remote_classifier_authentication")),
            404 => return Err(protocol_error("remote_classifier_route_missing")),
            409 => {
                return Err(ClassifierError::Incompatible(
                    "remote_classifier_pin_mismatch".into(),
                ))
            }
            412 => return Err(protocol_error("remote_classifier_pin_required")),
            424 => return Err(protocol_error("remote_classifier_model_not_configured")),
            429 => return Err(ClassifierError::Busy),
            408 | 504 => return Err(ClassifierError::Timeout),
            _ => return Err(protocol_error("remote_classifier_unavailable")),
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(transport_error)? {
            if chunk.len() > MAX_RESPONSE_BYTES - bytes.len() {
                return Err(protocol_error("remote_classifier_response_too_large"));
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes)
            .map_err(|_| protocol_error("remote_classifier_invalid_response"))
    }
}

fn transport_error(error: reqwest::Error) -> ClassifierError {
    if error.is_timeout() {
        ClassifierError::Timeout
    } else {
        protocol_error("remote_classifier_unreachable")
    }
}

struct Engine {
    runtime: Handle,
    client: Arc<Client>,
    identity: PinIdentity,
    threshold_bits: u64,
    model: ModelId,
    gates: GateSet,
}

impl Engine {
    async fn infer(&self, inputs: &[ClassifyInput]) -> Result<InferResponse, ClassifierError> {
        let request = InferRequest {
            version: VERSION,
            identity: self.identity.clone(),
            inputs: inputs.to_vec(),
        };
        let result = self.client.post(INFER_PATH, &request).await;
        if !matches!(&result, Err(ClassifierError::BridgeProtocol { code, .. }) if code == "remote_classifier_pin_required")
        {
            return result;
        }
        // Recover an evicted pin or a restarted service only by restoring the
        // original exact snapshot, manifest and threshold.
        let pin = self
            .client
            .request_pin(Some(&self.identity.snapshot))
            .await?;
        pin.validate(Some(&self.identity.snapshot))?;
        if pin.identity != self.identity || pin.threshold_bits != self.threshold_bits {
            return Err(ClassifierError::Incompatible(
                "remote_classifier_pin_mismatch".into(),
            ));
        }
        self.client.post(INFER_PATH, &request).await
    }
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
        for batch in inputs.chunks(MAX_BATCH_ITEMS) {
            control.check_active()?;
            let response = self.runtime.block_on(async {
                tokio::select! {
                    biased;
                    _ = cancelled(&control.cancel) => Err(ClassifierError::Cancelled),
                    _ = tokio::time::sleep_until(control.overall_deadline.into()) => Err(ClassifierError::Timeout),
                    result = self.infer(batch) => result,
                }
            })?;
            if response.version != VERSION
                || response.identity != self.identity
                || response.receipts.len() != batch.len()
            {
                return Err(protocol_error("invalid_inference_response"));
            }
            for (receipt, input) in response.receipts.iter().zip(batch) {
                validate_receipt(receipt, input, &self.model, &self.gates)?;
            }
            receipts.extend(response.receipts);
        }
        Ok(receipts)
    }

    fn serialize_model(&mut self) -> Result<String, ClassifierError> {
        Err(ClassifierError::Unavailable)
    }

    fn restore_model(&mut self, _payload: &str) -> Result<(), ClassifierError> {
        Err(ClassifierError::Unavailable)
    }
}

async fn cancelled(cancel: &AtomicBool) {
    while !cancel.load(Ordering::Acquire) {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// Explicit opt-in. Invalid configuration is an error and cannot select local inference.
pub fn is_configured() -> bool {
    std::env::var_os("FLEETWORKS_CLASSIFIER_ORIGIN").is_some()
}

fn client_from_environment() -> Result<Arc<Client>, ClassifierError> {
    let configuration_error = || protocol_error("remote_classifier_configuration");
    let origin =
        std::env::var("FLEETWORKS_CLASSIFIER_ORIGIN").map_err(|_| configuration_error())?;
    let token = if let Some(value) = std::env::var_os("FLEETWORKS_INTERNAL_TOKEN") {
        value.into_string().map_err(|_| configuration_error())?
    } else {
        let path =
            std::env::var_os("FLEETWORKS_INTERNAL_TOKEN_FILE").ok_or_else(configuration_error)?;
        let file = std::fs::File::open(path).map_err(|_| configuration_error())?;
        let mut token = String::new();
        file.take(259)
            .read_to_string(&mut token)
            .map_err(|_| configuration_error())?;
        if token.len() > 258 {
            return Err(configuration_error());
        }
        token.trim_end_matches(['\r', '\n']).to_owned()
    };
    Ok(Arc::new(Client::new(&origin, &token)?))
}

pub(super) async fn pin_configured(
    requested: Option<&ModelSnapshot>,
    cancel: &AtomicBool,
) -> Result<PinnedClassifier, ClassifierError> {
    let client = configured_client().await?;
    tokio::select! {
        biased;
        _ = cancelled(cancel) => Err(ClassifierError::Cancelled),
        result = client.pin(requested) => result,
    }
}

pub(super) async fn validate_configured(
    snapshot: &ModelSnapshot,
    manifest_sha256: &str,
    cancel: &AtomicBool,
) -> Result<(), ClassifierError> {
    let request = ValidateRequest {
        version: VERSION,
        identity: PinIdentity {
            snapshot: snapshot.clone(),
            manifest_sha256: manifest_sha256.to_owned(),
        },
    };
    request.validate()?;
    let client = configured_client().await?;
    let response: ValidateRequest = tokio::select! {
        biased;
        _ = cancelled(cancel) => return Err(ClassifierError::Cancelled),
        result = client.post(VALIDATE_PATH, &request) => result?,
    };
    if response.version != VERSION || response.identity != request.identity {
        return Err(protocol_error("invalid_validation_response"));
    }
    Ok(())
}

async fn configured_client() -> Result<&'static Arc<Client>, ClassifierError> {
    static CLIENT: OnceCell<Arc<Client>> = OnceCell::const_new();
    CLIENT
        .get_or_try_init(|| async {
            tokio::task::spawn_blocking(client_from_environment)
                .await
                .map_err(|_| protocol_error("remote_classifier_configuration"))?
        })
        .await
}
