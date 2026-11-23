//! Native Inbox question answering over authenticated HTTP and private IPC.

use std::fmt;

use serde::Deserialize;
use serde::Serialize;

use crate::context::MutationContext;

use super::identity;
use super::request_id;
use super::ChatInputError;
use super::ChatSourceReference;
use super::MAX_EVENT_WAIT_SECS;
use super::MAX_ID_BYTES;
use super::MAX_INPUT_BYTES;

pub const QA_SETTINGS_PATH: &str = "/api/v2/workstation/chat/settings";
pub const QA_TURNS_PATH: &str = "/api/v2/workstation/chat/turns";
pub const QA_LATEST_PATH: &str = "/api/v2/workstation/chat/latest";
pub const QA_EVENTS_PATH: &str = "/api/v2/workstation/chat/events";
pub const QA_CANCEL_PATH: &str = "/api/v2/workstation/chat/cancel";
/// Opt in to typed metric citations on the gateway's V2 Inbox response routes.
pub const QA_EVIDENCE_HEADER: &str = "x-fleetworks-inbox-evidence";
pub const QA_EVIDENCE_VERSION: &str = "1";

/// The Inbox displays at most three independent findings across at most 60 words.
pub const QA_MAX_FINDINGS: usize = 3;
pub const QA_MAX_WORDS: usize = 60;

/// The selected backend's inference lifecycle.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum QaPhase {
    #[default]
    Off,
    Starting,
    Ready,
    Stopping,
    Unavailable,
}

/// Redacted recovery category for an unavailable Embedded inference worker.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum QaAvailabilityReason {
    /// A verified model pack has not been installed for this companion.
    PackMissing,
    /// The configured model pack or its installation cannot be verified.
    PackInvalid,
    /// The host cannot enforce the required model-worker sandbox.
    SandboxUnsupported,
    /// The companion could not start or warm an isolated model worker.
    WorkerStartFailed,
}

/// Inference settings and caller-specific management authority.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct QaSettings {
    pub enabled: bool,
    pub phase: QaPhase,
    pub revision: u64,
    pub generation: u64,
    pub can_manage: bool,
    /// Present only when the selected Embedded companion diagnoses unavailability.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub availability_reason: Option<QaAvailabilityReason>,
}

impl QaSettings {
    /// Construct a backend observation scoped to the caller.
    #[must_use]
    pub fn new(
        enabled: bool,
        phase: QaPhase,
        revision: u64,
        generation: u64,
        can_manage: bool,
    ) -> Self {
        Self {
            enabled,
            phase,
            revision,
            generation,
            can_manage,
            availability_reason: None,
        }
    }
}

/// An explicitly confirmed change to the selected backend's setting.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct QaSettingsUpdate {
    pub context: MutationContext,
    pub expected_revision: u64,
    pub enabled: bool,
}

impl QaSettingsUpdate {
    /// Bind the update to the registered workstation and observed setting revision.
    #[must_use]
    pub fn new(context: MutationContext, expected_revision: u64, enabled: bool) -> Self {
        Self {
            context,
            expected_revision,
            enabled,
        }
    }
}

/// One question. Evidence is always selected by the authorized backend.
#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct QaTurnRequest {
    pub request_id: String,
    pub context: MutationContext,
    pub question: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<ChatSourceReference>,
}

impl fmt::Debug for QaTurnRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("QaTurnRequest(..)")
    }
}

impl QaTurnRequest {
    /// Construct a checked question with an optional typed context reference.
    pub fn new(
        request_id: String,
        context: MutationContext,
        question: String,
        source: Option<ChatSourceReference>,
    ) -> Result<Self, ChatInputError> {
        let request = Self {
            request_id,
            context,
            question,
            source,
        };
        request.validate()?;
        Ok(request)
    }

    /// Validate the shape and bounds before resolving evidence or reserving worker capacity.
    pub fn validate(&self) -> Result<(), ChatInputError> {
        if !request_id(&self.request_id) {
            return Err(ChatInputError::InvalidRequestId);
        }
        if let Some(source) = &self.source {
            source.validate()?;
        }
        if self.question.trim().is_empty() {
            return Err(ChatInputError::EmptyInput);
        }
        if self.question.len() > MAX_INPUT_BYTES {
            return Err(ChatInputError::InputTooLong);
        }
        if self
            .question
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t')
        {
            return Err(ChatInputError::ControlCharacter);
        }
        Ok(())
    }
}

/// Sequenced event query. A cursor is meaningful only for the specified turn.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct QaEventQuery {
    pub turn_id: String,
    #[serde(default)]
    pub after: u64,
    #[serde(default)]
    pub wait_seconds: u32,
}

impl QaEventQuery {
    /// Read forward from a cursor, optionally waiting up to four seconds.
    #[must_use]
    pub fn new(turn_id: String, after: u64, wait_seconds: u32) -> Self {
        Self {
            turn_id,
            after,
            wait_seconds,
        }
    }

    /// Validate the turn ID and polling budget.
    pub fn validate(&self) -> Result<(), ChatInputError> {
        if !identity(&self.turn_id, MAX_ID_BYTES) {
            return Err(ChatInputError::InvalidIdentity);
        }
        if self.wait_seconds > MAX_EVENT_WAIT_SECS {
            return Err(ChatInputError::InvalidWait);
        }
        Ok(())
    }
}

/// Stop the caller's accepted turn while retaining its published findings.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct QaCancelRequest {
    pub context: MutationContext,
    pub turn_id: String,
}

impl QaCancelRequest {
    /// Bind Stop to a registered workstation and an accepted turn.
    #[must_use]
    pub fn new(context: MutationContext, turn_id: String) -> Self {
        Self { context, turn_id }
    }

    /// Validate the turn ID before accessing storage.
    pub fn validate(&self) -> Result<(), ChatInputError> {
        if identity(&self.turn_id, MAX_ID_BYTES) {
            Ok(())
        } else {
            Err(ChatInputError::InvalidIdentity)
        }
    }
}

/// The trusted backend's lifecycle of one accepted question.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum QaTurnState {
    Queued,
    Running,
    Answered,
    InsufficientContext,
    Failed,
    Canceled,
}

impl QaTurnState {
    /// True once the exchange is sealed against further answer parts.
    #[must_use]
    pub fn is_terminal(self) -> bool {
        !matches!(self, Self::Queued | Self::Running)
    }
}

/// A fixed reply without an evidence citation, separate from the turn lifecycle.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum QaTurnNotice {
    Greeting,
    OutOfScope,
    ObservationUnavailable,
}

/// Exact unit of a frozen metric reading. Percentages are stored in tenths of a percent.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum QaMeasurementUnit {
    PercentTenths,
    Jobs,
}

/// The authorized measurement cited by a finding at its original observation time.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct QaMeasurement {
    pub value: u64,
    pub unit: QaMeasurementUnit,
    pub subject: String,
}

impl QaMeasurement {
    #[must_use]
    pub fn new(value: u64, unit: QaMeasurementUnit, subject: String) -> Self {
        Self {
            value,
            unit,
            subject,
        }
    }
}

/// A verified, cited span. The offsets count Unicode scalar values in the source passage.
#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct QaFinding {
    pub source: ChatSourceReference,
    pub label: String,
    pub text: String,
    pub start: u32,
    pub end: u32,
    pub observed_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub measurement: Option<QaMeasurement>,
}

impl fmt::Debug for QaFinding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("QaFinding(..)")
    }
}

impl QaFinding {
    /// Construct an already-validated span with its frozen observation metadata.
    #[must_use]
    pub fn new(
        source: ChatSourceReference,
        label: String,
        text: String,
        start: u32,
        end: u32,
        observed_at: String,
    ) -> Self {
        Self {
            source,
            label,
            text,
            start,
            end,
            observed_at,
            measurement: None,
        }
    }
}

/// The authoritative latest exchange and its highest committed event sequence.
#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct QaTurn {
    pub turn_id: String,
    pub request_id: String,
    pub question: String,
    pub state: QaTurnState,
    pub findings: Vec<QaFinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notice: Option<QaTurnNotice>,
    pub cursor: u64,
}

impl fmt::Debug for QaTurn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("QaTurn")
            .field("state", &self.state)
            .field("cursor", &self.cursor)
            .finish_non_exhaustive()
    }
}

impl QaTurn {
    /// Construct the initial state of an accepted turn.
    #[must_use]
    pub fn queued(turn_id: String, request_id: String, question: String) -> Self {
        Self {
            turn_id,
            request_id,
            question,
            state: QaTurnState::Queued,
            findings: Vec::new(),
            notice: None,
            cursor: 0,
        }
    }
}

/// One persisted status transition or cited finding.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct QaEvent {
    pub sequence: u64,
    pub state: QaTurnState,
    pub finding: Option<QaFinding>,
}

impl QaEvent {
    /// Attach an independently validated part or record a state transition.
    #[must_use]
    pub fn new(sequence: u64, state: QaTurnState, finding: Option<QaFinding>) -> Self {
        Self {
            sequence,
            state,
            finding,
        }
    }
}

/// An ordered event page and the canonical exchange for recovering from a cursor gap.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct QaEventPage {
    pub turn: QaTurn,
    pub events: Vec<QaEvent>,
    pub gap: bool,
}

impl QaEventPage {
    /// Package the canonical turn with its available event slice.
    #[must_use]
    pub fn new(turn: QaTurn, events: Vec<QaEvent>, gap: bool) -> Self {
        Self { turn, events, gap }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_availability_reason_is_optional_and_typed() {
        let mut settings = QaSettings::new(true, QaPhase::Unavailable, 1, 1, false);
        let legacy = serde_json::to_value(settings).unwrap();
        assert!(legacy.get("availability_reason").is_none());
        assert_eq!(
            serde_json::from_value::<QaSettings>(legacy).unwrap(),
            settings
        );
        for (reason, code) in [
            (QaAvailabilityReason::PackMissing, "pack_missing"),
            (QaAvailabilityReason::PackInvalid, "pack_invalid"),
            (
                QaAvailabilityReason::SandboxUnsupported,
                "sandbox_unsupported",
            ),
            (
                QaAvailabilityReason::WorkerStartFailed,
                "worker_start_failed",
            ),
        ] {
            settings.availability_reason = Some(reason);
            let value = serde_json::to_value(settings).unwrap();
            assert_eq!(value["availability_reason"], code);
            assert_eq!(
                serde_json::from_value::<QaSettings>(value).unwrap(),
                settings
            );
        }
    }
}
