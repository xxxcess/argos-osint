//! Confirmed interactive requests and server-verified opaque acceptance IDs.
//!
//! No native request derives a job ID or claims to be its creator. A reported
//! receipt is resolved through registered workstation Jobs before details or
//! private tool results are displayed. This module contains no provider client.

use std::io::Write;

use fleetworks_contracts::context::JobRequestAssociation;
use fleetworks_contracts::context::MutationContext;
use fleetworks_contracts::context::OriginSource;
use fleetworks_contracts::context::Permission;
use fleetworks_contracts::context::RegisteredContext;
use fleetworks_contracts::AtlasCycleReceipt;
use fleetworks_contracts::AtlasCycleRequest;
use fleetworks_contracts::Client;
use fleetworks_contracts::ClientError;
use fleetworks_contracts::ToolRunRequest;
use fleetworks_contracts::ATLAS_CYCLES_PATH;
use fleetworks_contracts::ATLAS_CYCLE_OPERATION;
use fleetworks_contracts::JOB_DETAIL_PATH;
use fleetworks_contracts::TOOL_RESULT_PATH;
use fleetworks_contracts::TOOL_RUN_PATH;
use serde::Deserialize;
use serde_json::Value;

use crate::app::Intent;
use crate::app::Load;
use crate::app::State;
use crate::backend::JobDetail;
use crate::connection::GatewayEffects;
use crate::connection::RequestTask;
use crate::platform;

const MAX_INPUT_BYTES: usize = 12 * 1024;
const MAX_RESULT_TEXT: usize = 16 * 1024;
const TOOLS_OPERATION: &str = "manual_tool";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SubmissionKind {
    Atlas,
    Tool,
}

impl SubmissionKind {
    pub fn feature(self) -> &'static str {
        match self {
            Self::Atlas => "atlas-cycles",
            Self::Tool => "tool-execution",
        }
    }
    pub fn operation(self) -> &'static str {
        match self {
            Self::Atlas => ATLAS_CYCLE_OPERATION,
            Self::Tool => TOOLS_OPERATION,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Atlas => "Atlas collection cycle",
            Self::Tool => "Tool execution",
        }
    }
}

/// Tool inputs can contain private investigation data. Never print a snapshot
/// or put it into fixture intents, traces or diagnostics.
#[derive(Clone, PartialEq)]
pub enum SubmissionInput {
    Atlas,
    Tool { tool_id: String, inputs: Value },
}

impl SubmissionInput {
    pub fn kind(&self) -> SubmissionKind {
        match self {
            Self::Atlas => SubmissionKind::Atlas,
            Self::Tool { .. } => SubmissionKind::Tool,
        }
    }
    pub fn tool_id(&self) -> Option<&str> {
        match self {
            Self::Tool { tool_id, .. } => Some(tool_id),
            Self::Atlas => None,
        }
    }
}

#[derive(Clone)]
pub struct SubmissionSnapshot {
    pub context: RegisteredContext,
    pub configuration_revision: u64,
    pub origin_revision: u64,
    pub input: SubmissionInput,
}

/// Idempotency identity and exact confirmed body, retained after uncertain
/// delivery. Reusing it never reads a changed tool draft or assigns new origin.
#[derive(Clone)]
pub struct SubmissionAttempt {
    pub request_id: String,
    pub snapshot: SubmissionSnapshot,
}

impl SubmissionAttempt {
    pub fn new(request_id: String, snapshot: SubmissionSnapshot) -> Result<Self, String> {
        if !(1..=64).contains(&request_id.len())
            || !request_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err("Interactive request identity is invalid".into());
        }
        Ok(Self {
            request_id,
            snapshot,
        })
    }
    pub fn mutation_context(&self) -> MutationContext {
        MutationContext::new(&self.snapshot.context)
            .with_configuration_revision(self.snapshot.configuration_revision)
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct ToolReceipt {
    pub call_id: String,
    #[serde(default)]
    pub job_id: Option<String>,
}

#[derive(Clone, Debug)]
pub enum AcceptedReceipt {
    Atlas(AtlasCycleReceipt),
    Tool(ToolReceipt),
}

impl AcceptedReceipt {
    pub fn kind(&self) -> SubmissionKind {
        match self {
            Self::Atlas(_) => SubmissionKind::Atlas,
            Self::Tool(_) => SubmissionKind::Tool,
        }
    }
    /// A missing optional Tools job reference is not a job claim. Its reported
    /// call identity is only a candidate for an exact authorized Jobs lookup.
    pub fn job_candidate(&self) -> &str {
        match self {
            Self::Atlas(receipt) => &receipt.job_id,
            Self::Tool(receipt) => receipt.job_id.as_deref().unwrap_or(&receipt.call_id),
        }
    }
    fn validate(&self) -> Result<(), String> {
        let opaque =
            |id: &str| !id.is_empty() && id.len() <= 512 && !id.chars().any(char::is_control);
        match self {
            Self::Atlas(receipt) if opaque(&receipt.job_id) && opaque(&receipt.run_id) => Ok(()),
            Self::Tool(receipt)
                if opaque(&receipt.call_id) && receipt.job_id.as_deref().is_none_or(opaque) =>
            {
                Ok(())
            }
            _ => Err("Gateway returned an invalid acceptance identity".into()),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SubmissionPhase {
    #[default]
    Idle,
    Preparing,
    Posting,
    Reported,
    Verified,
    Rejected,
    OutcomeUnknown,
}

#[derive(Clone, Debug)]
pub enum SubmissionFailure {
    Rejected(String),
    Uncertain(String),
}

impl SubmissionFailure {
    fn from_client(error: ClientError) -> Self {
        match error {
            ClientError::Status(code) if (400..500).contains(&code) && code != 408 => {
                Self::Rejected(error.to_string())
            }
            _ => Self::Uncertain(error.to_string()),
        }
    }
    fn message(&self) -> &str {
        match self {
            Self::Rejected(message) | Self::Uncertain(message) => message,
        }
    }
}

/// Only safe structured fields are retained. `raw` provider data and echoed
/// request inputs are deliberately absent from this wire projection.
#[derive(Deserialize)]
struct ToolStatusWire {
    call_id: String,
    #[serde(default)]
    job_id: Option<String>,
    tool_id: String,
    status: String,
    result: Option<ToolResultWire>,
}

#[derive(Deserialize)]
struct ToolResultWire {
    tool_id: String,
    status: String,
    source_url: String,
    retrieved_at: String,
    observations: Value,
    error: Option<String>,
    cached: bool,
    truncated: bool,
}

pub struct ToolResultView {
    pub tool_id: String,
    pub status: String,
    pub observed_at: String,
    pub observations: String,
    pub error: Option<String>,
    pub cached: bool,
    pub truncated: bool,
    pub source: Option<String>,
}

pub struct ToolStatusView {
    pub status: String,
    pub result: Option<ToolResultView>,
}

pub struct SubmissionLane {
    pub confirmation: Option<SubmissionSnapshot>,
    pub phase: SubmissionPhase,
    pub attempt: Option<SubmissionAttempt>,
    pub receipt: Option<AcceptedReceipt>,
    pub verification: Load<JobDetail>,
    pub resolution: Load<JobRequestAssociation>,
    pub result: Load<ToolStatusView>,
    pub message: Option<String>,
    pub delivery_uncertain: bool,
    pub(crate) revision: u64,
    pub(crate) request: Option<RequestTask>,
    pub(crate) verification_request: Option<RequestTask>,
    pub(crate) result_request: Option<RequestTask>,
    pub(crate) resolution_request: Option<RequestTask>,
}

impl Default for SubmissionLane {
    fn default() -> Self {
        Self {
            confirmation: None,
            phase: SubmissionPhase::Idle,
            attempt: None,
            receipt: None,
            verification: Load::Idle,
            resolution: Load::Idle,
            result: Load::Idle,
            message: None,
            delivery_uncertain: false,
            revision: 0,
            request: None,
            verification_request: None,
            result_request: None,
            resolution_request: None,
        }
    }
}

impl SubmissionLane {
    pub fn job_candidate(&self) -> Option<&str> {
        self.receipt
            .as_ref()
            .map(AcceptedReceipt::job_candidate)
            .or(match &self.resolution {
                Load::Ready(association) => Some(association.job_id.as_str()),
                _ => None,
            })
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn busy(&self) -> bool {
        matches!(
            self.phase,
            SubmissionPhase::Preparing | SubmissionPhase::Posting
        )
    }
    pub fn private_result_available(&self) -> bool {
        matches!(&self.verification, Load::Ready(detail) if detail.private_details_available && detail.job.private_details_available && !detail.job.status_only)
    }
    /// A prior uncertain outcome remains uncertain even if a later replay is
    /// rejected; losing permission or a revision cannot prove no earlier job.
    pub fn observe_post(
        &mut self,
        outcome: Result<AcceptedReceipt, SubmissionFailure>,
    ) -> Result<(), String> {
        let Some(attempt) = &self.attempt else {
            return Err("No confirmed interactive request is active".into());
        };
        match outcome {
            Ok(receipt) => {
                receipt.validate()?;
                if receipt.kind() != attempt.snapshot.input.kind() {
                    return Err("Acceptance kind differs from the confirmed request".into());
                }
                self.receipt = Some(receipt);
                self.phase = SubmissionPhase::Reported;
                self.verification = Load::Idle;
                self.resolution = Load::Idle;
                self.result = Load::Idle;
                self.delivery_uncertain = false;
                self.message = Some("Gateway reported acceptance. Registered workstation scope is being checked; completion is still pending.".into());
            }
            Err(failure) => {
                self.delivery_uncertain |= matches!(failure, SubmissionFailure::Uncertain(_));
                self.phase = if self.delivery_uncertain {
                    SubmissionPhase::OutcomeUnknown
                } else {
                    SubmissionPhase::Rejected
                };
                self.message = Some(if self.delivery_uncertain {
                    format!(
                        "Acceptance outcome remains unknown: {}. Retry this exact request or inspect workstation activity; no cancellation was sent.",
                        failure.message()
                    )
                } else {
                    format!("Request was rejected: {}", failure.message())
                });
            }
        }
        Ok(())
    }

    pub fn observe_verification(&mut self, result: Result<JobDetail, String>) {
        let expected = self.job_candidate().map(str::to_owned);
        let kind = self
            .attempt
            .as_ref()
            .map(|attempt| attempt.snapshot.input.kind());
        self.verification = match result {
            Ok(detail)
                if expected.as_deref() == Some(detail.job.id.as_str())
                    && kind.is_some_and(|kind| detail.job.operation == kind.operation()) =>
            {
                self.phase = SubmissionPhase::Verified;
                self.message = Some("Reported job verified in this registered workstation's activity. The backend owns its lifecycle.".into());
                Load::Ready(detail)
            }
            Ok(_) => Load::Failed(
                "Reported receipt did not resolve to the expected authorized job".into(),
            ),
            Err(error) => Load::Failed(error),
        };
        if !self.private_result_available() {
            self.result = Load::Idle;
            self.result_request = None;
        }
    }

    pub(crate) fn cancel_requests(&mut self) {
        self.request = None;
        self.verification_request = None;
        self.result_request = None;
        self.resolution_request = None;
    }

    fn invalidate_origin(&mut self) {
        let revision = self.revision.wrapping_add(1);
        let was_posting = self.phase == SubmissionPhase::Posting;
        self.cancel_requests();
        self.confirmation = None;
        self.verification = Load::Idle;
        self.result = Load::Idle;
        if matches!(self.resolution, Load::Loading | Load::Pending) {
            self.resolution = Load::Idle;
        }
        if self.job_candidate().is_some() {
            self.phase = SubmissionPhase::Reported;
            self.message = Some("The previous reported job needs explicit verification under the current registered workstation before details are shown.".into());
        } else if self.attempt.is_some() && (self.delivery_uncertain || was_posting) {
            self.delivery_uncertain = true;
            self.phase = SubmissionPhase::OutcomeUnknown;
            self.message = Some("Origin changed while the previous request outcome was unresolved. Its identity is retained, but this new origin cannot replay it; use an explicit scoped lookup.".into());
        } else {
            *self = Self::default();
        }
        self.revision = revision;
    }
}

#[derive(Default)]
pub struct Submissions {
    pub atlas: SubmissionLane,
    pub tools: SubmissionLane,
}

impl Submissions {
    pub fn lane(&self, kind: SubmissionKind) -> &SubmissionLane {
        match kind {
            SubmissionKind::Atlas => &self.atlas,
            SubmissionKind::Tool => &self.tools,
        }
    }
    pub fn lane_mut(&mut self, kind: SubmissionKind) -> &mut SubmissionLane {
        match kind {
            SubmissionKind::Atlas => &mut self.atlas,
            SubmissionKind::Tool => &mut self.tools,
        }
    }
    pub(crate) fn cancel_requests(&mut self) {
        self.atlas.cancel_requests();
        self.tools.cancel_requests();
    }
    pub(crate) fn invalidate_origin(&mut self) {
        self.atlas.invalidate_origin();
        self.tools.invalidate_origin();
    }
    pub(crate) fn invalidate_confirmations(&mut self) {
        self.atlas.confirmation = None;
        self.tools.confirmation = None;
    }
}

impl State {
    pub fn interactive_draft_edited(&mut self, kind: SubmissionKind) {
        self.submissions.lane_mut(kind).confirmation = None;
    }
    pub fn submission_disabled_reason(&self, kind: SubmissionKind) -> Option<String> {
        if !self.permission(Permission::SubmitInteractive) {
            return Some("Interactive submission permission required".into());
        }
        if self.registration.context.is_none() {
            return Some("Register the workstation before submitting work".into());
        }
        if self.configuration_revision().is_none() {
            return Some(
                "Backend configuration is unavailable; check the gateway connection".into(),
            );
        }
        if !self
            .connection
            .capabilities
            .as_ref()
            .is_some_and(|capabilities| {
                capabilities
                    .features
                    .iter()
                    .any(|feature| feature == kind.feature())
            })
        {
            return Some("This gateway has not enabled the required durable worker".into());
        }
        let lane = self.submissions.lane(kind);
        if lane.busy() {
            return Some("The current request is awaiting its acceptance response".into());
        }
        if lane.delivery_uncertain {
            return Some("Resolve the current uncertain outcome or retry its exact request before starting another".into());
        }
        if kind == SubmissionKind::Tool {
            let Load::Ready(catalog) = &self.backend.tools else {
                return Some("Refresh the tool catalog before reviewing a run".into());
            };
            if self
                .backend
                .selected_tool
                .as_ref()
                .is_none_or(|selected| !catalog.tools.iter().any(|tool| &tool.id == selected))
            {
                return Some("Select a catalog tool before reviewing a run".into());
            }
        }
        None
    }

    pub fn preview_atlas_cycle(&mut self) {
        self.preview_submission(SubmissionInput::Atlas);
    }

    pub fn preview_tool_run(&mut self) {
        let Load::Ready(catalog) = &self.backend.tools else {
            self.submissions.tools.message = Some("Refresh the tool catalog first".into());
            return;
        };
        let Some(tool_id) = &self.backend.selected_tool else {
            return;
        };
        if !catalog.tools.iter().any(|tool| &tool.id == tool_id) {
            return;
        }
        if self.backend.tool_inputs.text.len() > MAX_INPUT_BYTES {
            self.submissions.tools.message = Some("Native tool input JSON exceeds 12 KiB".into());
            return;
        }
        let inputs: Value = match serde_json::from_str(&self.backend.tool_inputs.text) {
            Ok(Value::Object(inputs)) => Value::Object(inputs),
            _ => {
                self.submissions.tools.message = Some("Enter tool inputs as a JSON object".into());
                return;
            }
        };
        self.preview_submission(SubmissionInput::Tool {
            tool_id: tool_id.clone(),
            inputs,
        });
    }

    fn preview_submission(&mut self, input: SubmissionInput) {
        let kind = input.kind();
        if let Some(reason) = self.submission_disabled_reason(kind) {
            self.submissions.lane_mut(kind).message = Some(reason);
            return;
        }
        let (Some(context), Some(configuration_revision)) =
            (&self.registration.context, self.configuration_revision())
        else {
            return;
        };
        self.submissions.lane_mut(kind).confirmation = Some(SubmissionSnapshot {
            context: context.clone(),
            configuration_revision,
            origin_revision: self.origin_revision,
            input,
        });
        self.reveal_control(match kind {
            SubmissionKind::Atlas => "atlas-submit-confirmation",
            SubmissionKind::Tool => "tool-submit-confirmation",
        });
    }

    fn submission_snapshot_current(&self, snapshot: &SubmissionSnapshot) -> bool {
        self.submission_context_current(snapshot)
            && self.configuration_revision() == Some(snapshot.configuration_revision)
    }

    fn submission_context_current(&self, snapshot: &SubmissionSnapshot) -> bool {
        self.origin_revision == snapshot.origin_revision
            && self.registration.context.as_ref() == Some(&snapshot.context)
    }

    pub fn confirm_submission(&mut self, kind: SubmissionKind) {
        let Some(snapshot) = self.submissions.lane_mut(kind).confirmation.take() else {
            return;
        };
        if !self.submission_snapshot_current(&snapshot) {
            self.submissions.lane_mut(kind).message =
                Some("Origin or backend configuration changed; review the request again".into());
            return;
        }
        if let Some(reason) = self.submission_disabled_reason(kind) {
            self.submissions.lane_mut(kind).message = Some(reason);
            return;
        }
        if !self.fixture && self.services.is_none() {
            let lane = self.submissions.lane_mut(kind);
            lane.phase = SubmissionPhase::Rejected;
            lane.message = Some("Submission preparation service is unavailable".into());
            return;
        }
        let lane = self.submissions.lane_mut(kind);
        lane.cancel_requests();
        lane.phase = SubmissionPhase::Preparing;
        lane.revision = lane.revision.wrapping_add(1);
        let revision = lane.revision;
        lane.attempt = None;
        lane.receipt = None;
        lane.resolution = Load::Idle;
        lane.verification = Load::Idle;
        lane.result = Load::Idle;
        lane.message = None;
        // Fixtures receive this exact confirmed snapshot before injecting the
        // preparation/HTTP response. It never appears in an intent or trace.
        lane.confirmation = Some(snapshot.clone());
        self.record(Intent::SubmitInteractive { kind });
        if self.fixture {
            return;
        }
        if let Some(services) = &self.services {
            self.submissions.lane_mut(kind).request =
                services.prepare_interactive(snapshot, self.connection.epoch, revision);
        }
    }

    pub fn complete_submission_preparation(
        &mut self,
        kind: SubmissionKind,
        revision: u64,
        prepared: Result<SubmissionAttempt, String>,
    ) {
        if self.submissions.lane(kind).revision != revision {
            return;
        }
        self.submissions.lane_mut(kind).request = None;
        let attempt = match prepared {
            Ok(attempt) if self.submission_snapshot_current(&attempt.snapshot) => attempt,
            Ok(_) => {
                let lane = self.submissions.lane_mut(kind);
                lane.confirmation = None;
                lane.phase = SubmissionPhase::Rejected;
                lane.message = Some("Origin or configuration changed before submission".into());
                return;
            }
            Err(error) => {
                let lane = self.submissions.lane_mut(kind);
                lane.phase = SubmissionPhase::Rejected;
                lane.message = Some(error);
                return;
            }
        };
        let lane = self.submissions.lane_mut(kind);
        lane.confirmation = None;
        lane.attempt = Some(attempt);
        lane.phase = SubmissionPhase::Idle;
        self.send_interactive_attempt(kind);
    }

    pub fn retry_submission(&mut self, kind: SubmissionKind) {
        if self.submissions.lane(kind).job_candidate().is_some() {
            self.verify_submission(kind);
            return;
        }
        self.send_interactive_attempt(kind);
    }

    /// A read-only lookup of the original request identity. Current scope is
    /// always supplied by the registered client; original registration is only
    /// a reference, never an owner/workstation selector or new admission.
    pub fn resolve_submission(&mut self, kind: SubmissionKind) {
        let lane = self.submissions.lane(kind);
        if lane.busy() || matches!(lane.resolution, Load::Loading) {
            return;
        }
        let Some(attempt) = lane.attempt.clone() else {
            return;
        };
        let Some(context) = self.registration.context.clone() else {
            self.submissions.lane_mut(kind).message =
                Some("Register the original workstation to resolve its prior request".into());
            return;
        };
        if !self.permission(Permission::ReadWorkstationActivity) {
            return;
        }
        let lane = self.submissions.lane_mut(kind);
        lane.cancel_requests();
        lane.resolution = Load::Loading;
        lane.verification = Load::Idle;
        lane.result = Load::Idle;
        lane.revision = lane.revision.wrapping_add(1);
        let revision = lane.revision;
        self.record(Intent::ResolveInteractive { kind });
        if self.fixture {
            return;
        }
        let (Some(services), Some(client)) = (self.services.clone(), self.client.clone()) else {
            self.complete_submission_resolution(
                kind,
                revision,
                Err("Gateway resolution services are unavailable".into()),
            );
            return;
        };
        let client = match client.with_registered_context(&context) {
            Ok(client) => client,
            Err(error) => {
                self.complete_submission_resolution(kind, revision, Err(error.to_string()));
                return;
            }
        };
        self.submissions.lane_mut(kind).resolution_request = services.resolve_interactive(
            client,
            attempt,
            self.connection.epoch,
            self.origin_revision,
            revision,
        );
    }

    pub fn complete_submission_resolution(
        &mut self,
        kind: SubmissionKind,
        revision: u64,
        result: Result<JobRequestAssociation, String>,
    ) {
        if self.submissions.lane(kind).revision != revision {
            return;
        }
        let lane = self.submissions.lane_mut(kind);
        lane.resolution_request = None;
        lane.resolution = match result {
            Ok(association)
                if lane
                    .attempt
                    .as_ref()
                    .is_some_and(|attempt| association_matches(attempt, &association))
                    && lane
                        .receipt
                        .as_ref()
                        .is_none_or(|receipt| receipt.job_candidate() == association.job_id) =>
            {
                lane.delivery_uncertain = false;
                lane.phase = SubmissionPhase::Reported;
                lane.message = Some("Server verified the exact request association. Its returned job is being checked through current registered Jobs.".into());
                Load::Ready(association)
            }
            Ok(_) => Load::Failed(
                "Request lookup returned a different immutable request association".into(),
            ),
            Err(error) => Load::Failed(error),
        };
        if matches!(self.submissions.lane(kind).resolution, Load::Ready(_)) {
            self.verify_submission(kind);
        }
    }

    fn send_interactive_attempt(&mut self, kind: SubmissionKind) {
        let Some(attempt) = self.submissions.lane(kind).attempt.clone() else {
            return;
        };
        if self.submissions.lane(kind).busy() {
            return;
        }
        let config_current =
            self.configuration_revision() == Some(attempt.snapshot.configuration_revision);
        let reconcile_only = self.submissions.lane(kind).delivery_uncertain;
        if !self.submission_context_current(&attempt.snapshot)
            || (!config_current && !reconcile_only)
            || !self.permission(Permission::SubmitInteractive)
        {
            self.submissions.lane_mut(kind).message = Some("This exact request belongs to an earlier origin or configuration; inspect its workstation activity before any new submission".into());
            return;
        }
        let lane = self.submissions.lane_mut(kind);
        lane.phase = SubmissionPhase::Posting;
        lane.revision = lane.revision.wrapping_add(1);
        let revision = lane.revision;
        self.record(Intent::PostInteractive { kind });
        if self.fixture {
            return;
        }
        let (Some(services), Some(client)) = (self.services.clone(), self.client.clone()) else {
            self.complete_submission_post(
                kind,
                revision,
                Err(SubmissionFailure::Rejected(
                    "Gateway submission services are unavailable".into(),
                )),
            );
            return;
        };
        let client = match client.with_registered_context(&attempt.snapshot.context) {
            Ok(client) => client,
            Err(error) => {
                self.complete_submission_post(
                    kind,
                    revision,
                    Err(SubmissionFailure::Rejected(error.to_string())),
                );
                return;
            }
        };
        self.submissions.lane_mut(kind).request =
            services.post_interactive(client, attempt, self.connection.epoch, revision);
    }

    pub fn complete_submission_post(
        &mut self,
        kind: SubmissionKind,
        revision: u64,
        result: Result<AcceptedReceipt, SubmissionFailure>,
    ) {
        if self.submissions.lane(kind).revision != revision {
            return;
        }
        let lane = self.submissions.lane_mut(kind);
        lane.request = None;
        if let Err(error) = lane.observe_post(result) {
            lane.phase = SubmissionPhase::OutcomeUnknown;
            lane.delivery_uncertain = true;
            lane.message = Some(error);
        }
        if self.submissions.lane(kind).receipt.is_some() {
            if !self.fixture && kind == SubmissionKind::Atlas {
                self.refresh_cycles();
            }
            self.verify_submission(kind);
        }
    }

    pub fn verify_submission(&mut self, kind: SubmissionKind) {
        let lane = self.submissions.lane(kind);
        if matches!(lane.verification, Load::Loading) {
            return;
        }
        let Some(candidate) = lane.job_candidate() else {
            return;
        };
        if self.registration.context.is_none()
            || !self.permission(Permission::ReadWorkstationActivity)
        {
            let lane = self.submissions.lane_mut(kind);
            lane.result = Load::Idle;
            lane.result_request = None;
            lane.verification = Load::Failed("Acceptance was reported, but current registered activity permission/context cannot verify the job".into());
            return;
        }
        let id = candidate.to_owned();
        let Some(context) = self.registration.context.clone() else {
            return;
        };
        let lane = self.submissions.lane_mut(kind);
        lane.verification_request = None;
        lane.result_request = None;
        lane.verification = Load::Loading;
        lane.result = Load::Idle;
        lane.revision = lane.revision.wrapping_add(1);
        let revision = lane.revision;
        self.record(Intent::VerifyInteractive { kind });
        if self.fixture {
            return;
        }
        let (Some(services), Some(client)) = (self.services.clone(), self.client.clone()) else {
            self.complete_submission_verification(
                kind,
                revision,
                Err("Gateway verification services are unavailable".into()),
            );
            return;
        };
        let client = match client.with_registered_context(&context) {
            Ok(client) => client,
            Err(error) => {
                self.submissions
                    .lane_mut(kind)
                    .observe_verification(Err(error.to_string()));
                return;
            }
        };
        self.submissions.lane_mut(kind).verification_request = services.verify_interactive(
            client,
            id,
            kind,
            self.connection.epoch,
            self.origin_revision,
            revision,
        );
    }

    pub fn complete_submission_verification(
        &mut self,
        kind: SubmissionKind,
        revision: u64,
        result: Result<JobDetail, String>,
    ) {
        if self.submissions.lane(kind).revision != revision {
            return;
        }
        let lane = self.submissions.lane_mut(kind);
        lane.verification_request = None;
        lane.observe_verification(result);
    }

    pub fn refresh_tool_result(&mut self) {
        if !self.permission(Permission::ReadWorkstationActivity)
            || !self.submissions.tools.private_result_available()
            || matches!(self.submissions.tools.result, Load::Loading)
        {
            return;
        }
        let lane = &self.submissions.tools;
        let Some(attempt) = &lane.attempt else {
            return;
        };
        let Some(context) = self.registration.context.clone() else {
            return;
        };
        let tool_id = attempt.snapshot.input.tool_id().map(str::to_owned);
        let (call_id, job_id) = match &lane.receipt {
            Some(AcceptedReceipt::Tool(receipt)) => {
                (receipt.call_id.clone(), receipt.job_id.clone())
            }
            _ => match &lane.resolution {
                Load::Ready(association) => {
                    (association.job_id.clone(), Some(association.job_id.clone()))
                }
                _ => return,
            },
        };
        let lane = &mut self.submissions.tools;
        lane.result_request = None;
        lane.result = Load::Loading;
        lane.revision = lane.revision.wrapping_add(1);
        let revision = lane.revision;
        self.record(Intent::InteractiveToolResult);
        if self.fixture {
            return;
        }
        let (Some(services), Some(client)) = (self.services.clone(), self.client.clone()) else {
            self.submissions.tools.result =
                Load::Failed("Gateway tool result services are unavailable".into());
            return;
        };
        let client = match client.with_registered_context(&context) {
            Ok(client) => client,
            Err(error) => {
                self.submissions.tools.result = Load::Failed(error.to_string());
                return;
            }
        };
        self.submissions.tools.result_request = services.interactive_tool_result(
            client,
            call_id,
            job_id,
            tool_id,
            self.connection.epoch,
            self.origin_revision,
            revision,
        );
    }

    pub fn complete_interactive_tool_result(
        &mut self,
        revision: u64,
        result: Result<ToolStatusView, String>,
    ) {
        if self.submissions.tools.revision != revision {
            return;
        }
        let lane = &mut self.submissions.tools;
        lane.result_request = None;
        lane.result = match result {
            Ok(result) => Load::Ready(result),
            Err(error) => {
                lane.verification =
                    Load::Failed("Private result eligibility must be verified again".into());
                Load::Failed(error)
            }
        };
    }
}

impl GatewayEffects {
    fn resolve_interactive(
        &self,
        client: Client,
        attempt: SubmissionAttempt,
        epoch: u64,
        origin_revision: u64,
        revision: u64,
    ) -> Option<RequestTask> {
        let ui = self.ui.get()?.clone();
        let kind = attempt.snapshot.input.kind();
        let task = self.runtime.spawn(async move {
            #[derive(serde::Serialize)]
            struct Lookup<'a> {
                request_id: &'a str,
                operation: &'a str,
                original_registration_id: &'a str,
            }
            let result = client
                .get_with_query(
                    "/api/v1/workstation/requests/detail",
                    &Lookup {
                        request_id: &attempt.request_id,
                        operation: kind.operation(),
                        original_registration_id: &attempt.snapshot.context.registration_id,
                    },
                )
                .await
                .map_err(|error| error.to_string());
            ui.update(move |state, _| {
                if state.connection.epoch == epoch && state.origin_revision == origin_revision {
                    state.complete_submission_resolution(kind, revision, result);
                }
            });
        });
        Some(RequestTask(task.abort_handle()))
    }
    fn prepare_interactive(
        &self,
        snapshot: SubmissionSnapshot,
        epoch: u64,
        revision: u64,
    ) -> Option<RequestTask> {
        let ui = self.ui.get()?.clone();
        let kind = snapshot.input.kind();
        let origin_revision = snapshot.origin_revision;
        let task = self.runtime.spawn(async move {
            let prepared = tokio::task::spawn_blocking(move || {
                platform::request_id()
                    .map_err(|error| error.to_string())
                    .and_then(|id| SubmissionAttempt::new(id, snapshot))
            })
            .await
            .map_err(|_| "Request identity worker stopped".to_owned())
            .and_then(|result| result);
            ui.update(move |state, _| {
                if state.connection.epoch == epoch && state.origin_revision == origin_revision {
                    state.complete_submission_preparation(kind, revision, prepared);
                }
            });
        });
        Some(RequestTask(task.abort_handle()))
    }

    fn post_interactive(
        &self,
        client: Client,
        attempt: SubmissionAttempt,
        epoch: u64,
        revision: u64,
    ) -> Option<RequestTask> {
        let ui = self.ui.get()?.clone();
        let kind = attempt.snapshot.input.kind();
        let origin_revision = attempt.snapshot.origin_revision;
        let task = self.runtime.spawn(async move {
            let result = post(&client, &attempt).await;
            ui.update(move |state, _| {
                if state.connection.epoch == epoch && state.origin_revision == origin_revision {
                    state.complete_submission_post(kind, revision, result);
                }
            });
        });
        Some(RequestTask(task.abort_handle()))
    }

    fn verify_interactive(
        &self,
        client: Client,
        id: String,
        kind: SubmissionKind,
        epoch: u64,
        origin_revision: u64,
        revision: u64,
    ) -> Option<RequestTask> {
        let ui = self.ui.get()?.clone();
        let task = self.runtime.spawn(async move {
            let result = client
                .get_with_query(JOB_DETAIL_PATH, &[("id", id)])
                .await
                .map_err(|error| error.to_string());
            ui.update(move |state, _| {
                if state.connection.epoch == epoch && state.origin_revision == origin_revision {
                    state.complete_submission_verification(kind, revision, result);
                }
            });
        });
        Some(RequestTask(task.abort_handle()))
    }

    #[allow(clippy::too_many_arguments)]
    fn interactive_tool_result(
        &self,
        client: Client,
        call_id: String,
        job_id: Option<String>,
        tool_id: Option<String>,
        epoch: u64,
        origin_revision: u64,
        revision: u64,
    ) -> Option<RequestTask> {
        let ui = self.ui.get()?.clone();
        let task = self.runtime.spawn(async move {
            let result = async {
                let wire: ToolStatusWire = client
                    .get_with_query(TOOL_RESULT_PATH, &[("id", call_id.clone())])
                    .await
                    .map_err(|error| error.to_string())?;
                if wire.call_id != call_id
                    || Some(wire.tool_id.as_str()) != tool_id.as_deref()
                    || job_id
                        .as_ref()
                        .is_some_and(|expected| wire.job_id.as_ref() != Some(expected))
                {
                    return Err("Gateway returned a result for another tool request".to_owned());
                }
                tokio::task::spawn_blocking(move || format_tool_status(wire))
                    .await
                    .map_err(|_| "Tool result formatter stopped".to_owned())?
            }
            .await;
            ui.update(move |state, _| {
                if state.connection.epoch != epoch
                    || state.origin_revision != origin_revision
                    || state.submissions.tools.revision != revision
                {
                    return;
                }
                state.complete_interactive_tool_result(revision, result);
            });
        });
        Some(RequestTask(task.abort_handle()))
    }
}

async fn post(
    client: &Client,
    attempt: &SubmissionAttempt,
) -> Result<AcceptedReceipt, SubmissionFailure> {
    // The gateway must enforce these client preconditions while capturing the
    // actual configuration, before admission. They never assign server origin.
    match &attempt.snapshot.input {
        SubmissionInput::Atlas => {
            let request = AtlasCycleRequest::new(attempt.request_id.clone())
                .ok_or_else(|| SubmissionFailure::Rejected("Invalid request identity".into()))?;
            client
                .post(
                    ATLAS_CYCLES_PATH,
                    &request.with_context(attempt.mutation_context()),
                )
                .await
                .map(AcceptedReceipt::Atlas)
                .map_err(SubmissionFailure::from_client)
        }
        SubmissionInput::Tool { tool_id, inputs } => {
            let request =
                ToolRunRequest::new(attempt.request_id.clone(), tool_id.clone(), inputs.clone());
            client
                .post(
                    TOOL_RUN_PATH,
                    &request.with_context(attempt.mutation_context()),
                )
                .await
                .map(AcceptedReceipt::Tool)
                .map_err(SubmissionFailure::from_client)
        }
    }
}

fn format_tool_status(wire: ToolStatusWire) -> Result<ToolStatusView, String> {
    let result = wire
        .result
        .map(|result| {
            if result.tool_id != wire.tool_id {
                return Err("Tool result identity is inconsistent".to_owned());
            }
            let (observations, bounded) = bounded_json(&result.observations);
            let source = url::Url::parse(&result.source_url)
                .ok()
                .filter(|url| {
                    matches!(url.scheme(), "http" | "https")
                        && url.host().is_some()
                        && url.username().is_empty()
                        && url.password().is_none()
                })
                .map(|mut url| {
                    url.set_query(None);
                    url.set_fragment(None);
                    url.to_string()
                });
            Ok(ToolResultView {
                tool_id: result.tool_id,
                status: result.status,
                observed_at: result.retrieved_at,
                observations,
                error: result.error,
                cached: result.cached,
                truncated: result.truncated || bounded,
                source,
            })
        })
        .transpose()?;
    Ok(ToolStatusView {
        status: wire.status,
        result,
    })
}

fn bounded_json(value: &Value) -> (String, bool) {
    struct Bounded {
        bytes: Vec<u8>,
        truncated: bool,
    }
    impl Write for Bounded {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            let take = bytes
                .len()
                .min(MAX_RESULT_TEXT.saturating_sub(self.bytes.len()));
            self.bytes.extend_from_slice(&bytes[..take]);
            if take < bytes.len() {
                self.truncated = true;
                return Err(std::io::Error::other("display bound reached"));
            }
            Ok(take)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut output = Bounded {
        bytes: Vec::new(),
        truncated: false,
    };
    let _ = serde_json::to_writer_pretty(&mut output, value);
    (
        String::from_utf8_lossy(&output.bytes).into_owned(),
        output.truncated,
    )
}

fn association_matches(attempt: &SubmissionAttempt, association: &JobRequestAssociation) -> bool {
    let origin = &association.origin;
    let context = &attempt.snapshot.context;
    !association.job_id.is_empty()
        && association.job_id.len() <= 512
        && !association.job_id.chars().any(char::is_control)
        && !association.associated_at.is_empty()
        && origin.source == OriginSource::Native
        && origin.request_id == attempt.request_id
        && origin.registration_id == context.registration_id
        && origin.deployment_id == context.deployment_id
        && origin.workstation_id == context.workstation_id
        && origin.workspace_id.is_none()
        && origin.connect_id_at_submission == context.connect_id.as_str()
        && origin.workstation_ip_at_submission == context.workstation_ip
        && origin.workspace_name_at_submission.is_none()
        && origin.configuration_epoch == context.configuration_epoch
        && origin.configuration_revision == attempt.snapshot.configuration_revision
}

#[cfg(test)]
mod tests;
