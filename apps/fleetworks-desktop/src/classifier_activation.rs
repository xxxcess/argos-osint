//! Exact Natural candidate activation. Training, durable acceptance, proven
//! activation and real inference readiness remain separate states.

use fleetworks_contracts::classifier_activation::ActivationReceipt;
use fleetworks_contracts::classifier_activation::ActivationRequest;
use fleetworks_contracts::classifier_activation::ActivationResult;
use fleetworks_contracts::classifier_activation::ActivationState;
use fleetworks_contracts::classifier_activation::ActiveModelRevision;
use fleetworks_contracts::classifier_activation::CandidatePreview;
use fleetworks_contracts::classifier_activation::SyntheticBaselineConsent;
use fleetworks_contracts::classifier_activation::ACTIVATION_PATH;
use fleetworks_contracts::classifier_activation::ACTIVATION_RESULT_PATH;
use fleetworks_contracts::classifier_activation::CANDIDATE_DETAIL_PATH;
use fleetworks_contracts::context::JobOrigin;
use fleetworks_contracts::context::JobRequestAssociation;
use fleetworks_contracts::context::MutationContext;
use fleetworks_contracts::context::OriginSource;
use fleetworks_contracts::context::Permission;
use fleetworks_contracts::context::RegisteredContext;
use fleetworks_contracts::context::ResourceScope;
use fleetworks_contracts::readiness::ReadinessActionId;
use fleetworks_contracts::readiness::TrainableEngine;
use fleetworks_contracts::Client;
use fleetworks_contracts::ClientError;
use fleetworks_contracts::JOB_DETAIL_PATH;
use rune::TextEdit;
use serde::Serialize;

use crate::app::Intent;
use crate::app::Load;
use crate::app::Page;
use crate::app::State;
use crate::backend::JobDetail;
use crate::connection::GatewayEffects;
use crate::connection::RequestTask;
use crate::platform;

const OPERATION: &str = "classifier_activate";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActivationAction {
    Preview,
    Prepare,
    Post,
    Resolve,
    Verify,
    Result,
}

#[derive(Clone)]
pub struct CandidateSnapshot {
    pub preview: CandidatePreview,
    pub context: RegisteredContext,
    pub origin_revision: u64,
}

#[derive(Clone)]
pub struct ActivationConfirmation {
    pub candidate: CandidateSnapshot,
    pub configuration_revision: u64,
    pub consent: SyntheticBaselineConsent,
}

#[derive(Clone)]
pub struct ActivationAttempt {
    pub request: ActivationRequest,
    pub context: RegisteredContext,
    pub origin_revision: u64,
}

#[derive(Default)]
pub struct ClassifierActivation {
    pub training_job: TextEdit,
    pub candidate_model: TextEdit,
    pub preview: Load<CandidateSnapshot>,
    pub confirmation: Option<ActivationConfirmation>,
    pub consent: bool,
    pub attempt: Option<ActivationAttempt>,
    pub receipt: Load<ActivationReceipt>,
    pub association: Load<JobRequestAssociation>,
    pub job: Load<JobDetail>,
    pub result: Load<ActivationResult>,
    pub message: Option<String>,
    pub uncertain: bool,
    pub busy: bool,
    known_job: Option<String>,
    pub(crate) request: Option<RequestTask>,
    revision: u64,
    active: Option<ActivationAction>,
}

impl ClassifierActivation {
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub(crate) fn cancel_requests(&mut self) {
        self.request = None;
        self.revision = self.revision.wrapping_add(1);
        self.busy = false;
        self.active = None;
    }
    pub(crate) fn invalidate_origin(&mut self) {
        self.request = None;
        self.revision = self.revision.wrapping_add(1);
        self.busy = false;
        self.active = None;
        self.preview = Load::Idle;
        self.confirmation = None;
        self.consent = false;
        self.receipt = Load::Idle;
        self.association = Load::Idle;
        self.job = Load::Idle;
        self.result = Load::Idle;
    }
    pub(crate) fn invalidate_confirmation(&mut self) {
        self.confirmation = None;
    }
}

#[derive(Clone)]
pub(crate) enum ActivationEffect {
    Preview {
        training_job_id: String,
        model_id: String,
    },
    Prepare(ActivationConfirmation),
    Post(ActivationAttempt),
    Resolve(ActivationAttempt),
    Verify(String),
    Result(String),
}

impl ActivationEffect {
    fn action(&self) -> ActivationAction {
        match self {
            Self::Preview { .. } => ActivationAction::Preview,
            Self::Prepare(_) => ActivationAction::Prepare,
            Self::Post(_) => ActivationAction::Post,
            Self::Resolve(_) => ActivationAction::Resolve,
            Self::Verify(_) => ActivationAction::Verify,
            Self::Result(_) => ActivationAction::Result,
        }
    }
}

pub enum ActivationResponse {
    Preview(CandidatePreview),
    Prepared(ActivationAttempt),
    Receipt(ActivationReceipt),
    Association(JobRequestAssociation),
    Job(JobDetail),
    Result(ActivationResult),
}

pub struct ActivationFailure {
    pub message: String,
    pub uncertain: bool,
}

impl State {
    fn activation_read_context(&self) -> Result<RegisteredContext, String> {
        if !self.permission(Permission::MaintainBackend)
            || !self.permission(Permission::ReadWorkstationActivity)
        {
            return Err(
                "Backend maintenance and workstation activity permissions are required".into(),
            );
        }
        self.registration
            .context
            .clone()
            .ok_or_else(|| "Register the workstation first".into())
    }

    pub fn activation_disabled_reason(&self) -> Option<String> {
        let context = match self.activation_read_context() {
            Ok(context) => context,
            Err(reason) => return Some(reason),
        };
        if self.classifier_activation.busy {
            return Some("An activation request is already being checked".into());
        }
        if self.classifier_activation.uncertain {
            return Some("Resolve the earlier activation request before preparing another".into());
        }
        let Load::Ready(readiness) = &self.readiness else {
            return Some("Refresh backend checks before activation".into());
        };
        if readiness.registration.as_ref().is_none_or(|receipt| {
            receipt.registration_id != context.registration_id
                || receipt.revision != context.registration_revision
        }) {
            return Some("Refresh checks for the current registered workstation".into());
        }
        let Some((check, action)) = readiness.checks.iter().find_map(|check| {
            check
                .actions
                .iter()
                .find(|action| action.id == ReadinessActionId::ActivateClassifier)
                .map(|action| (check, action))
        }) else {
            return Some("This backend has not advertised candidate activation".into());
        };
        if !self.readiness_check_fresh(check) {
            return Some("Activation availability is stale; refresh checks".into());
        }
        if !action.allowed {
            return Some(
                action
                    .disabled_reason
                    .clone()
                    .unwrap_or_else(|| "Candidate activation is unavailable".into()),
            );
        }
        if self.configuration_revision().is_none() {
            return Some(
                "Refresh Profile or Models to observe the backend configuration revision".into(),
            );
        }
        None
    }

    pub fn inspect_training_candidate(&mut self, training_job: &str, candidate: &str) {
        self.classifier_activation.training_job = TextEdit::new(training_job);
        self.classifier_activation.candidate_model = TextEdit::new(candidate);
        self.navigate(Page::Workstation);
        self.load_activation_candidate();
    }

    pub fn load_activation_candidate(&mut self) {
        if self.classifier_activation.busy {
            return;
        }
        if let Err(reason) = self.activation_read_context() {
            self.classifier_activation.message = Some(reason);
            return;
        }
        if self.classifier_activation.uncertain {
            self.classifier_activation.message =
                Some("Resolve the earlier activation request before preparing another".into());
            return;
        }
        let training_job_id = self.classifier_activation.training_job.text.clone();
        let model_id = self.classifier_activation.candidate_model.text.clone();
        if !identifier(&training_job_id, 128) || !identifier(&model_id, 64) {
            self.classifier_activation.message =
                Some("Enter the completed training job and candidate model identifiers".into());
            return;
        }
        self.classifier_activation.preview = Load::Loading;
        self.classifier_activation.confirmation = None;
        self.classifier_activation.consent = false;
        self.activation_effect(ActivationEffect::Preview {
            training_job_id,
            model_id,
        });
    }

    pub fn preview_candidate_activation(&mut self) {
        if let Some(reason) = self.activation_disabled_reason() {
            self.classifier_activation.message = Some(reason);
            return;
        }
        let Load::Ready(candidate) = &self.classifier_activation.preview else {
            return;
        };
        if self.registration.context.as_ref() != Some(&candidate.context)
            || self.origin_revision != candidate.origin_revision
        {
            self.classifier_activation.message =
                Some("The registered workstation changed; inspect the candidate again".into());
            return;
        }
        if !candidate.preview.activation_supported {
            self.classifier_activation.message = Some(candidate.preview.reason_code.clone());
            return;
        }
        if candidate.preview.synthetic_baseline && !self.classifier_activation.consent {
            self.classifier_activation.message =
                Some("Explicit consent is required to activate this synthetic baseline".into());
            return;
        }
        let Some(configuration_revision) = self.configuration_revision() else {
            return;
        };
        self.classifier_activation.confirmation = Some(ActivationConfirmation {
            candidate: candidate.clone(),
            configuration_revision,
            consent: if candidate.preview.synthetic_baseline {
                SyntheticBaselineConsent::ActivateSyntheticBaseline
            } else {
                SyntheticBaselineConsent::NotGranted
            },
        });
        self.reveal_control("classifier-activation-confirmation");
    }

    pub fn confirm_candidate_activation(&mut self) {
        let Some(confirmation) = self.classifier_activation.confirmation.take() else {
            return;
        };
        if self.activation_disabled_reason().is_some()
            || self.registration.context.as_ref() != Some(&confirmation.candidate.context)
            || self.origin_revision != confirmation.candidate.origin_revision
            || self.configuration_revision() != Some(confirmation.configuration_revision)
            || confirmation.candidate.preview.synthetic_baseline
                && !self.classifier_activation.consent
        {
            self.classifier_activation.message = Some(
                "Activation context, configuration or consent changed; review it again".into(),
            );
            return;
        }
        self.classifier_activation.receipt = Load::Idle;
        self.classifier_activation.association = Load::Idle;
        self.classifier_activation.job = Load::Idle;
        self.classifier_activation.result = Load::Idle;
        self.activation_effect(ActivationEffect::Prepare(confirmation));
    }

    pub fn retry_exact_activation(&mut self) {
        let Some(attempt) = self.classifier_activation.attempt.clone() else {
            return;
        };
        if !self.classifier_activation.uncertain
            || self.registration.context.as_ref() != Some(&attempt.context)
            || self.origin_revision != attempt.origin_revision
        {
            self.classifier_activation.message = Some(
                "Use the read-only lookup for an activation from an earlier registration context"
                    .into(),
            );
            return;
        }
        self.activation_effect(ActivationEffect::Post(attempt));
    }

    pub fn resolve_activation_request(&mut self) {
        let Some(attempt) = self.classifier_activation.attempt.clone() else {
            return;
        };
        let Ok(current) = self.activation_read_context() else {
            return;
        };
        if !same_workstation(&current, &attempt.context) {
            self.classifier_activation.message =
                Some("Select the original workstation before looking up this request".into());
            return;
        }
        self.classifier_activation.association = Load::Loading;
        self.activation_effect(ActivationEffect::Resolve(attempt));
    }

    pub fn refresh_activation_result(&mut self) {
        let Some(job) = self.classifier_activation.known_job.clone() else {
            return;
        };
        self.classifier_activation.job = Load::Loading;
        self.classifier_activation.result = Load::Idle;
        self.activation_effect(ActivationEffect::Verify(job));
    }

    fn activation_effect(&mut self, effect: ActivationEffect) {
        if self.classifier_activation.busy {
            return;
        }
        let context = match self.activation_read_context() {
            Ok(context) => context,
            Err(reason) => {
                self.fail_activation_effect(&effect, reason);
                return;
            }
        };
        self.classifier_activation.request = None;
        self.classifier_activation.revision = self.classifier_activation.revision.wrapping_add(1);
        self.classifier_activation.busy = true;
        self.classifier_activation.message = None;
        let revision = self.classifier_activation.revision;
        let action = effect.action();
        self.classifier_activation.active = Some(action);
        self.record(Intent::ClassifierActivation { action });
        if self.fixture {
            return;
        }
        if let Some(services) = &self.services {
            if let Some(client) = &self.client {
                if let Ok(client) = client.with_registered_context(&context) {
                    self.classifier_activation.request = services.classifier_activation(
                        client,
                        context,
                        effect,
                        self.connection.epoch,
                        self.origin_revision,
                        revision,
                    );
                    return;
                }
            }
        }
        self.fail_activation_effect(
            &effect,
            "Gateway activation client or registered context is unavailable".into(),
        );
    }

    fn fail_activation_effect(&mut self, effect: &ActivationEffect, reason: String) {
        self.classifier_activation.busy = false;
        self.classifier_activation.active = None;
        self.classifier_activation.message = Some(reason.clone());
        match effect {
            ActivationEffect::Preview { .. } => {
                self.classifier_activation.preview = Load::Failed(reason);
            }
            ActivationEffect::Resolve(_) => {
                self.classifier_activation.association = Load::Failed(reason);
            }
            ActivationEffect::Verify(_) => {
                self.classifier_activation.job = Load::Failed(reason);
            }
            ActivationEffect::Prepare(_)
            | ActivationEffect::Post(_)
            | ActivationEffect::Result(_) => {}
        }
    }

    /// Production and fixtures share all response identity/privacy checks.
    pub fn complete_classifier_activation(
        &mut self,
        revision: u64,
        result: Result<ActivationResponse, ActivationFailure>,
    ) {
        if self.classifier_activation.revision != revision {
            return;
        }
        self.classifier_activation.request = None;
        self.classifier_activation.busy = false;
        let action = self.classifier_activation.active.take();
        let response = match result {
            Ok(response) => response,
            Err(error) => {
                self.classifier_activation.uncertain |= error.uncertain;
                match action {
                    Some(ActivationAction::Preview) => {
                        self.classifier_activation.preview = Load::Failed(error.message.clone())
                    }
                    Some(ActivationAction::Post) => {
                        self.classifier_activation.receipt = Load::Failed(error.message.clone())
                    }
                    Some(ActivationAction::Resolve) => {
                        self.classifier_activation.association = Load::Failed(error.message.clone())
                    }
                    Some(ActivationAction::Verify) => {
                        self.classifier_activation.job = Load::Failed(error.message.clone())
                    }
                    Some(ActivationAction::Result) => {
                        self.classifier_activation.result = Load::Failed(error.message.clone())
                    }
                    _ => {}
                }
                self.classifier_activation.message = Some(error.message);
                return;
            }
        };
        match response {
            ActivationResponse::Preview(preview) => {
                if preview.engine != TrainableEngine::Natural
                    || preview.candidate.validate().is_err()
                    || preview.expected_prior.validate().is_err()
                    || preview.candidate.training_job_id
                        != self.classifier_activation.training_job.text
                    || preview.candidate.model_id != self.classifier_activation.candidate_model.text
                {
                    self.classifier_activation.preview = Load::Failed(
                        "Candidate preview changed its requested identity or hashes".into(),
                    );
                    return;
                }
                let Some(context) = self.registration.context.clone() else {
                    return;
                };
                self.classifier_activation.preview = Load::Ready(CandidateSnapshot {
                    preview,
                    context,
                    origin_revision: self.origin_revision,
                });
            }
            ActivationResponse::Prepared(attempt) => {
                if attempt.request.validate().is_err()
                    || !matches!(&self.classifier_activation.preview, Load::Ready(snapshot)
                        if snapshot.context == attempt.context && snapshot.origin_revision == attempt.origin_revision
                            && snapshot.preview.candidate == attempt.request.candidate && snapshot.preview.expected_prior == attempt.request.expected_prior
                            && (!snapshot.preview.synthetic_baseline || attempt.request.synthetic_baseline == SyntheticBaselineConsent::ActivateSyntheticBaseline && self.classifier_activation.consent))
                    || self.registration.context.as_ref() != Some(&attempt.context)
                    || self.origin_revision != attempt.origin_revision
                    || self.configuration_revision()
                        != attempt.request.context.expected_configuration_revision
                {
                    self.classifier_activation.message =
                        Some("Activation changed before its request was prepared".into());
                    return;
                }
                self.classifier_activation.attempt = Some(attempt.clone());
                self.classifier_activation.uncertain = false;
                self.activation_effect(ActivationEffect::Post(attempt));
            }
            ActivationResponse::Receipt(receipt) => {
                let valid = self
                    .classifier_activation
                    .attempt
                    .as_ref()
                    .is_some_and(|attempt| {
                        receipt.resource_scope == ResourceScope::Backend
                            && opaque(&receipt.job_id)
                            && receipt.candidate == attempt.request.candidate
                            && receipt.expected_prior == attempt.request.expected_prior
                            && origin_matches(attempt, &receipt.origin)
                    });
                if !valid {
                    self.classifier_activation.uncertain = true;
                    self.classifier_activation.message = Some(
                        "Activation receipt does not match the confirmed immutable request".into(),
                    );
                    return;
                }
                self.classifier_activation.uncertain = false;
                self.classifier_activation.known_job = Some(receipt.job_id.clone());
                self.classifier_activation.receipt = Load::Ready(receipt);
                self.refresh_activation_result();
            }
            ActivationResponse::Association(association) => {
                if !opaque(&association.job_id)
                    || association.associated_at.is_empty()
                    || !self
                        .classifier_activation
                        .attempt
                        .as_ref()
                        .is_some_and(|attempt| origin_matches(attempt, &association.origin))
                    || self
                        .classifier_activation
                        .known_job
                        .as_ref()
                        .is_some_and(|known| known != &association.job_id)
                {
                    self.classifier_activation.association =
                        Load::Failed("Lookup returned a different activation request".into());
                    return;
                }
                self.classifier_activation.uncertain = false;
                self.classifier_activation.known_job = Some(association.job_id.clone());
                self.classifier_activation.association = Load::Ready(association);
                self.refresh_activation_result();
            }
            ActivationResponse::Job(detail) => {
                if Some(&detail.job.id) != self.classifier_activation.known_job.as_ref()
                    || detail.job.operation != OPERATION
                {
                    self.classifier_activation.job =
                        Load::Failed("Jobs returned a different activation operation".into());
                    return;
                }
                let private = detail.private_details_available
                    && detail.job.private_details_available
                    && !detail.job.status_only;
                let id = detail.job.id.clone();
                self.classifier_activation.job = Load::Ready(detail);
                if private {
                    self.classifier_activation.result = Load::Loading;
                    self.activation_effect(ActivationEffect::Result(id));
                } else {
                    self.classifier_activation.result =
                        Load::Failed("Activation result is unavailable for this shared job".into());
                }
            }
            ActivationResponse::Result(result) => {
                let valid = Some(&result.job_id) == self.classifier_activation.known_job.as_ref()
                    && self
                        .classifier_activation
                        .attempt
                        .as_ref()
                        .is_some_and(|attempt| result.candidate == attempt.request.candidate)
                    && matches!(&self.classifier_activation.job, Load::Ready(detail) if detail.private_details_available && detail.job.private_details_available && !detail.job.status_only);
                if !valid
                    || !self
                        .classifier_activation
                        .attempt
                        .as_ref()
                        .is_some_and(|attempt| {
                            result_matches(&result, &attempt.request.expected_prior)
                        })
                {
                    self.classifier_activation.result = Load::Failed(
                        "Activation result lacks matching authorized effect proof".into(),
                    );
                    return;
                }
                self.classifier_activation.result = Load::Ready(result);
            }
        }
    }
}

impl GatewayEffects {
    #[allow(clippy::too_many_arguments)]
    fn classifier_activation(
        &self,
        client: Client,
        context: RegisteredContext,
        effect: ActivationEffect,
        epoch: u64,
        origin: u64,
        revision: u64,
    ) -> Option<RequestTask> {
        let ui = self.ui.get()?.clone();
        let task = self.runtime.spawn(async move {
            let result = run_effect(&client, effect).await;
            ui.update(move |state, _| {
                if state.connection.epoch == epoch
                    && state.origin_revision == origin
                    && state.registration.context.as_ref() == Some(&context)
                {
                    state.complete_classifier_activation(revision, result);
                }
            });
        });
        Some(RequestTask(task.abort_handle()))
    }
}

async fn run_effect(
    client: &Client,
    effect: ActivationEffect,
) -> Result<ActivationResponse, ActivationFailure> {
    #[derive(Serialize)]
    struct CandidateQuery {
        training_job_id: String,
        model_id: String,
    }
    #[derive(Serialize)]
    struct JobQuery {
        id: String,
    }
    #[derive(Serialize)]
    struct ResultQuery {
        job_id: String,
    }
    #[derive(Serialize)]
    struct RequestQuery {
        request_id: String,
        operation: &'static str,
        original_registration_id: String,
    }
    let action = effect.action();
    let result = match effect {
        ActivationEffect::Preview {
            training_job_id,
            model_id,
        } => client
            .get_with_query(
                CANDIDATE_DETAIL_PATH,
                &CandidateQuery {
                    training_job_id,
                    model_id,
                },
            )
            .await
            .map(ActivationResponse::Preview),
        ActivationEffect::Prepare(confirmation) => {
            return tokio::task::spawn_blocking(move || {
                let request = ActivationRequest::new(
                    platform::request_id().map_err(|error| error.to_string())?,
                    MutationContext::new(&confirmation.candidate.context)
                        .with_configuration_revision(confirmation.configuration_revision),
                    confirmation.candidate.preview.candidate,
                    confirmation.candidate.preview.expected_prior,
                    confirmation.consent,
                )
                .map_err(|error| error.to_string())?;
                Ok(ActivationResponse::Prepared(ActivationAttempt {
                    request,
                    context: confirmation.candidate.context,
                    origin_revision: confirmation.candidate.origin_revision,
                }))
            })
            .await
            .map_err(|_| ActivationFailure {
                message: "Activation preparation worker stopped".into(),
                uncertain: false,
            })?
            .map_err(|message| ActivationFailure {
                message,
                uncertain: false,
            });
        }
        ActivationEffect::Post(attempt) => client
            .post(ACTIVATION_PATH, &attempt.request)
            .await
            .map(ActivationResponse::Receipt),
        ActivationEffect::Resolve(attempt) => client
            .get_with_query(
                "/api/v1/workstation/requests/detail",
                &RequestQuery {
                    request_id: attempt.request.request_id,
                    operation: OPERATION,
                    original_registration_id: attempt.context.registration_id,
                },
            )
            .await
            .map(ActivationResponse::Association),
        ActivationEffect::Verify(id) => client
            .get_with_query(JOB_DETAIL_PATH, &JobQuery { id })
            .await
            .map(ActivationResponse::Job),
        ActivationEffect::Result(job_id) => client
            .get_with_query(ACTIVATION_RESULT_PATH, &ResultQuery { job_id })
            .await
            .map(ActivationResponse::Result),
    };
    result.map_err(|error| { let uncertain = action == ActivationAction::Post && !matches!(error, ClientError::Status(code) if (400..500).contains(&code) && code != 408);
        ActivationFailure { message:error.to_string(),uncertain } })
}

fn identifier(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
}
fn opaque(value: &str) -> bool {
    !value.is_empty() && value.len() <= 512 && !value.chars().any(char::is_control)
}
fn same_workstation(left: &RegisteredContext, right: &RegisteredContext) -> bool {
    left.deployment_id == right.deployment_id && left.workstation_id == right.workstation_id
}
fn origin_matches(attempt: &ActivationAttempt, origin: &JobOrigin) -> bool {
    let context = &attempt.context;
    origin.source == OriginSource::Native
        && origin.request_id == attempt.request.request_id
        && origin.registration_id == context.registration_id
        && origin.deployment_id == context.deployment_id
        && origin.workstation_id == context.workstation_id
        && origin.workspace_id.is_none()
        && origin.connect_id_at_submission == context.connect_id.as_str()
        && origin.workstation_ip_at_submission == context.workstation_ip
        && origin.workspace_name_at_submission.is_none()
        && origin.configuration_epoch == context.configuration_epoch
        && Some(origin.configuration_revision)
            == attempt.request.context.expected_configuration_revision
}
fn result_matches(result: &ActivationResult, expected_prior: &ActiveModelRevision) -> bool {
    if result.state != ActivationState::Activated {
        return true;
    }
    let previous_generation = match expected_prior {
        ActiveModelRevision::NeverActivated => 0,
        ActiveModelRevision::Active { generation, .. } => *generation,
        _ => return false,
    };
    matches!(&result.activated_revision, Some(ActiveModelRevision::Active { model_id,generation,manifest_sha256 })
        if model_id == &result.candidate.model_id && *generation > previous_generation && manifest_sha256 == &result.candidate.manifest_sha256)
}

#[cfg(test)]
mod tests;
