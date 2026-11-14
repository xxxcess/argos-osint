//! Explicit Natural candidate activation, independent of training/inference.
//! Hashes identify immutable artifacts; these DTOs grant no server authority.

use serde::Deserialize;
use serde::Serialize;

use crate::context::JobOrigin;
use crate::context::MutationContext;
use crate::context::ResourceScope;
use crate::readiness::TrainableEngine;

pub const CANDIDATE_DETAIL_PATH: &str = "/api/v1/classifier/candidates/detail";
pub const ACTIVATION_PATH: &str = "/api/v1/classifier/activate";
pub const ACTIVATION_RESULT_PATH: &str = "/api/v1/classifier/activation/result";

fn identity(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
}
fn hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

/// Exact candidate produced by the named durable Natural training operation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct CandidateIdentity {
    pub model_id: String,
    pub training_job_id: String,
    pub dataset_sha256: String,
    pub manifest_sha256: String,
    pub model_sha256: String,
    pub report_sha256: String,
}
impl CandidateIdentity {
    pub fn new(
        model_id: String,
        training_job_id: String,
        dataset_sha256: String,
        manifest_sha256: String,
        model_sha256: String,
        report_sha256: String,
    ) -> Result<Self, ActivationInputError> {
        let value = Self {
            model_id,
            training_job_id,
            dataset_sha256,
            manifest_sha256,
            model_sha256,
            report_sha256,
        };
        value.validate()?;
        Ok(value)
    }
    pub fn validate(&self) -> Result<(), ActivationInputError> {
        if !identity(&self.model_id, 64)
            || !identity(&self.training_job_id, 128)
            || [
                &self.dataset_sha256,
                &self.manifest_sha256,
                &self.model_sha256,
                &self.report_sha256,
            ]
            .iter()
            .any(|value| !hash(value))
        {
            return Err(ActivationInputError::InvalidCandidate);
        }
        Ok(())
    }
}

/// An explicit absence or an exact active pointer revision; never a wildcard.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum ActiveModelRevision {
    NeverActivated,
    Active {
        model_id: String,
        generation: u64,
        manifest_sha256: String,
    },
}
impl ActiveModelRevision {
    pub fn active(
        model_id: String,
        generation: u64,
        manifest_sha256: String,
    ) -> Result<Self, ActivationInputError> {
        let value = Self::Active {
            model_id,
            generation,
            manifest_sha256,
        };
        value.validate()?;
        Ok(value)
    }
    pub fn validate(&self) -> Result<(), ActivationInputError> {
        if let Self::Active {
            model_id,
            generation,
            manifest_sha256,
        } = self
        {
            if !identity(model_id, 64) || *generation == 0 || !hash(manifest_sha256) {
                return Err(ActivationInputError::InvalidPrior);
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum SyntheticBaselineConsent {
    #[default]
    NotGranted,
    ActivateSyntheticBaseline,
}

/// The preview exposes metadata only after registered training-job eligibility.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct CandidatePreview {
    pub engine: TrainableEngine,
    pub candidate: CandidateIdentity,
    pub expected_prior: ActiveModelRevision,
    pub synthetic_baseline: bool,
    pub activation_supported: bool,
    pub reason_code: String,
}
impl CandidatePreview {
    pub fn new(
        candidate: CandidateIdentity,
        expected_prior: ActiveModelRevision,
        synthetic_baseline: bool,
        activation_supported: bool,
        reason_code: String,
    ) -> Self {
        Self {
            engine: TrainableEngine::Natural,
            candidate,
            expected_prior,
            synthetic_baseline,
            activation_supported,
            reason_code,
        }
    }
}

/// Separate confirmed mutation. No path, command, owner, promotion evidence or
/// activation-from-training flag may be supplied by a caller.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ActivationRequest {
    pub request_id: String,
    pub context: MutationContext,
    pub engine: TrainableEngine,
    pub candidate: CandidateIdentity,
    pub expected_prior: ActiveModelRevision,
    #[serde(default)]
    pub synthetic_baseline: SyntheticBaselineConsent,
}
impl ActivationRequest {
    pub fn new(
        request_id: String,
        context: MutationContext,
        candidate: CandidateIdentity,
        expected_prior: ActiveModelRevision,
        synthetic_baseline: SyntheticBaselineConsent,
    ) -> Result<Self, ActivationInputError> {
        let value = Self {
            request_id,
            context,
            engine: TrainableEngine::Natural,
            candidate,
            expected_prior,
            synthetic_baseline,
        };
        value.validate()?;
        Ok(value)
    }
    pub fn validate(&self) -> Result<(), ActivationInputError> {
        if !identity(&self.request_id, 64)
            || !self
                .request_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
            || self.engine != TrainableEngine::Natural
        {
            return Err(ActivationInputError::InvalidRequest);
        }
        self.candidate.validate()?;
        self.expected_prior.validate()?;
        if self.context.expected_configuration_revision.is_none() {
            return Err(ActivationInputError::MissingConfigurationRevision);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct ActivationReceipt {
    pub job_id: String,
    pub resource_scope: ResourceScope,
    pub origin: JobOrigin,
    pub candidate: CandidateIdentity,
    pub expected_prior: ActiveModelRevision,
    pub reused: bool,
}
impl ActivationReceipt {
    pub fn new(
        job_id: String,
        origin: JobOrigin,
        request: &ActivationRequest,
        reused: bool,
    ) -> Self {
        Self {
            job_id,
            resource_scope: ResourceScope::Backend,
            origin,
            candidate: request.candidate.clone(),
            expected_prior: request.expected_prior.clone(),
            reused,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ActivationState {
    Queued,
    Running,
    Activated,
    Rejected,
    EffectUncertain,
    Interrupted,
}

/// An effect is attributed only by an operation-owned durable completion proof.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct ActivationResult {
    pub job_id: String,
    pub candidate: CandidateIdentity,
    pub state: ActivationState,
    pub activated_revision: Option<ActiveModelRevision>,
    pub reason_code: String,
}
impl ActivationResult {
    pub fn new(
        job_id: String,
        candidate: CandidateIdentity,
        state: ActivationState,
        activated_revision: Option<ActiveModelRevision>,
        reason_code: String,
    ) -> Self {
        Self {
            job_id,
            candidate,
            state,
            activated_revision,
            reason_code,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ActivationInputError {
    InvalidCandidate,
    InvalidPrior,
    InvalidRequest,
    MissingConfigurationRevision,
}
impl std::fmt::Display for ActivationInputError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Invalid explicit Natural activation preconditions")
    }
}
impl std::error::Error for ActivationInputError {}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn immutable_identity_and_prior_never_accept_paths_or_wildcards() {
        let candidate = CandidateIdentity::new(
            "natural-candidate".into(),
            "setup-training".into(),
            "a".repeat(64),
            "b".repeat(64),
            "c".repeat(64),
            "d".repeat(64),
        )
        .expect("bounded candidate");
        let mut body = serde_json::to_value(&candidate).expect("candidate JSON");
        body["model_id"] = "../active.json".into();
        assert!(serde_json::from_value::<CandidateIdentity>(body)
            .expect("shape")
            .validate()
            .is_err());
        assert!(ActiveModelRevision::active("prior".into(), 0, "a".repeat(64)).is_err());
        assert!(serde_json::from_value::<ActiveModelRevision>(
            serde_json::json!({"kind":"active","model_id":"prior","generation":1})
        )
        .is_err());
        assert_eq!(
            SyntheticBaselineConsent::default(),
            SyntheticBaselineConsent::NotGranted
        );
    }
    #[test]
    fn activation_does_not_accept_caller_authority_or_training_toggle() {
        let base = serde_json::json!({"request_id":"activate-once","context":{"registration_id":"fixture","expected_registration_revision":1,"expected_context_revision":1,"expected_configuration_revision":1},"engine":"natural","candidate":{"model_id":"candidate","training_job_id":"setup-training","dataset_sha256":"a".repeat(64),"manifest_sha256":"b".repeat(64),"model_sha256":"c".repeat(64),"report_sha256":"d".repeat(64)},"expected_prior":{"kind":"never_activated"}});
        let request: ActivationRequest =
            serde_json::from_value(base.clone()).expect("explicit request");
        request.validate().expect("valid input");
        let mut fresh = request.clone();
        fresh.context.expected_configuration_revision = Some(0);
        fresh
            .validate()
            .expect("explicit fresh configuration revision zero");
        fresh.context.expected_configuration_revision = None;
        assert_eq!(
            fresh.validate(),
            Err(ActivationInputError::MissingConfigurationRevision)
        );
        assert_eq!(
            request.synthetic_baseline,
            SyntheticBaselineConsent::NotGranted
        );
        for field in ["activate", "owner", "path", "promotion_evidence", "source"] {
            let mut forged = base.clone();
            forged[field] = serde_json::json!(true);
            assert!(serde_json::from_value::<ActivationRequest>(forged).is_err());
        }
    }
}
