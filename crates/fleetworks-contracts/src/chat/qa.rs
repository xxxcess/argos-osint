//! Native Inbox question answering over authenticated HTTP and private IPC.

use std::collections::BTreeSet;
use std::fmt;

use serde::Deserialize;
use serde::Serialize;

use crate::context::MutationContext;
use crate::qa_worker::QaContextUsage;
pub use crate::qa_worker::QaModelTarget;
use crate::readiness::ReadinessCheckId;

use super::identity;
use super::request_id;
use super::ChatInputError;
use super::ChatSourceKind;
use super::ChatSourceReference;
use super::MAX_EVENT_WAIT_SECS;
use super::MAX_ID_BYTES;
use super::MAX_INPUT_BYTES;

pub const QA_SETTINGS_PATH: &str = "/api/v2/workstation/chat/settings";
pub const QA_TURNS_PATH: &str = "/api/v2/workstation/chat/turns";
pub const QA_LATEST_PATH: &str = "/api/v2/workstation/chat/latest";
/// Read retained terminal exchanges over private Embedded IPC.
pub const QA_HISTORY_PATH: &str = "/api/v2/workstation/chat/history";
pub const QA_EVENTS_PATH: &str = "/api/v2/workstation/chat/events";
pub const QA_CANCEL_PATH: &str = "/api/v2/workstation/chat/cancel";
/// Embedded capability for direct Assistant answers and inline triage.
pub const QA_TRIAGE_FEATURE: &str = "inbox-qa-triage-v1";
/// Embedded capability for independently revisioned generator and decision preferences.
pub const QA_MODEL_SETTINGS_FEATURE: &str = "inbox-qa-model-settings-v1";
pub const QA_GPT2_GENERATORS_FEATURE: &str = "inbox-qa-gpt2-v1";
/// Embedded capability for browsing a registration's retained Inbox exchanges.
pub const QA_HISTORY_FEATURE: &str = "inbox-qa-history-v1";
/// Embedded capability for accepting a bounded summary of native Inbox notices.
pub const QA_INBOX_NOTIFICATIONS_FEATURE: &str = "inbox-qa-notifications-v1";
/// Read-only related activity lookup over private Embedded IPC.
pub const QA_RELATED_ACTIVITY_PATH: &str = "/api/v2/workstation/chat/related-activity";

/// Opt in to typed metric citations on the gateway's V2 Inbox response routes.
pub const QA_EVIDENCE_HEADER: &str = "x-fleetworks-inbox-evidence";
pub const QA_EVIDENCE_VERSION: &str = "1";

/// The Inbox displays at most three independent findings across at most 60 words.
pub const QA_MAX_FINDINGS: usize = 3;
pub const QA_MAX_WORDS: usize = 60;
/// Maximum number of exchanges retained per registration.
pub const QA_MAX_RETAINED_TURNS: usize = 16;
/// Number of native Inbox notification topics that may accompany a question.
pub const QA_MAX_INBOX_NOTIFICATION_GROUPS: usize = 3;
/// Maximum notices retained in the native Inbox.
pub const QA_MAX_INBOX_NOTIFICATION_ITEMS: u8 = 50;

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

/// The work currently preparing the optional Embedded inference model.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum QaStartupStage {
    DownloadingPack,
    VerifyingPack,
    LoadingModel,
    Activating,
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
    /// Available workstation memory cannot safely accommodate the selected model.
    InsufficientResources,
    /// The selected model assets are unavailable or could not be verified.
    ModelProvisionFailed,
}

/// Saved generator policy, independent of the currently running model.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum QaGeneratorPreference {
    #[default]
    Automatic,
    FlanT5Small,
    FlanT5Base,
    Gpt2Small,
    Gpt2Medium,
}

/// Saved decision policy, as well as the effective rules or local scorer.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum QaDecisionPreference {
    #[default]
    RulesOnly,
    Julia1Q8,
    VllmD3EdgeQ8,
}

/// Redacted reason a selected decision scorer is unavailable.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum QaDecisionAvailabilityReason {
    AwaitingQualification,
    InsufficientResources,
    PackMissing,
    WorkerStartFailed,
}

/// The saved generator policy and the verified worker, if ready.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct QaGeneratorSettings {
    pub preference: QaGeneratorPreference,
    pub effective: Option<QaModelTarget>,
    pub phase: QaPhase,
    pub reason: Option<QaAvailabilityReason>,
}

impl QaGeneratorSettings {
    /// Describe the saved policy and observed generator lifecycle.
    #[must_use]
    pub const fn new(
        preference: QaGeneratorPreference,
        effective: Option<QaModelTarget>,
        phase: QaPhase,
        reason: Option<QaAvailabilityReason>,
    ) -> Self {
        Self {
            preference,
            effective,
            phase,
            reason,
        }
    }
}

/// The saved decision policy and the currently usable engine.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct QaDecisionSettings {
    pub preference: QaDecisionPreference,
    pub effective: QaDecisionPreference,
    pub phase: QaPhase,
    pub reason: Option<QaDecisionAvailabilityReason>,
}

impl QaDecisionSettings {
    /// Describe the saved policy and observed decision lifecycle.
    #[must_use]
    pub const fn new(
        preference: QaDecisionPreference,
        effective: QaDecisionPreference,
        phase: QaPhase,
        reason: Option<QaDecisionAvailabilityReason>,
    ) -> Self {
        Self {
            preference,
            effective,
            phase,
            reason,
        }
    }
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
    #[serde(default)]
    pub model_target: QaModelTarget,
    /// Present only when the selected Embedded companion diagnoses unavailability.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub availability_reason: Option<QaAvailabilityReason>,
    /// Optional progress from the private Embedded installer or current worker startup.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub startup_stage: Option<QaStartupStage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generator: Option<QaGeneratorSettings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<QaDecisionSettings>,
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
            model_target: QaModelTarget::default(),
            availability_reason: None,
            startup_stage: None,
            generator: None,
            decision: None,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generator_preference: Option<QaGeneratorPreference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision_preference: Option<QaDecisionPreference>,
}

impl QaSettingsUpdate {
    /// Bind the update to the registered workstation and observed setting revision.
    #[must_use]
    pub fn new(context: MutationContext, expected_revision: u64, enabled: bool) -> Self {
        Self {
            enabled: Some(enabled),
            ..Self::preferences(context, expected_revision)
        }
    }

    /// Start a partial update without modifying the backend inference switch.
    #[must_use]
    pub fn preferences(context: MutationContext, expected_revision: u64) -> Self {
        Self {
            context,
            expected_revision,
            enabled: None,
            generator_preference: None,
            decision_preference: None,
        }
    }

    /// Set the generator preference on a peer that advertises model settings.
    #[must_use]
    pub const fn with_generator_preference(mut self, preference: QaGeneratorPreference) -> Self {
        self.generator_preference = Some(preference);
        self
    }

    /// Set the decision preference on a peer that advertises model settings.
    #[must_use]
    pub const fn with_decision_preference(mut self, preference: QaDecisionPreference) -> Self {
        self.decision_preference = Some(preference);
        self
    }
}

/// One explicitly selected job or readiness check for read-only triage.
#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct QaRelatedActivityRequest {
    pub source: ChatSourceReference,
}

impl fmt::Debug for QaRelatedActivityRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("QaRelatedActivityRequest(..)")
    }
}

impl QaRelatedActivityRequest {
    pub fn new(source: ChatSourceReference) -> Result<Self, ChatInputError> {
        let request = Self { source };
        request.validate()?;
        Ok(request)
    }

    /// Limit triage to supported typed sources.
    pub fn validate(&self) -> Result<(), ChatInputError> {
        self.source.validate()?;
        match self.source.kind {
            ChatSourceKind::Job | ChatSourceKind::Readiness => Ok(()),
            _ => Err(ChatInputError::InvalidIdentity),
        }
    }
}

/// A backend-verified reason for offering navigation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum QaRelationshipReason {
    CurrentStatus,
    StageCheck,
    SharedOperation,
}

/// Canonical outcome of a bounded Assistant lookup.
#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum QaRelatedActivity {
    Found {
        target: ChatSourceReference,
        reason: QaRelationshipReason,
    },
    NoRelatedActivity,
    TriageUnavailable,
    Clarify,
}

impl fmt::Debug for QaRelatedActivity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Found { .. } => "QaRelatedActivity::Found(..)",
            Self::NoRelatedActivity => "QaRelatedActivity::NoRelatedActivity",
            Self::TriageUnavailable => "QaRelatedActivity::TriageUnavailable",
            Self::Clarify => "QaRelatedActivity::Clarify",
        })
    }
}

/// Fixed categories of native Inbox observations. Jobs are aggregated without an identifier.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum QaInboxNotificationTopic {
    Connection,
    Jobs,
    Readiness(ReadinessCheckId),
}

/// Highest visual severity among the unread notices in a group, or among its read notices.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum QaInboxNotificationSeverity {
    Informational,
    Warning,
    Critical,
}

/// Counts observed by the submitting desktop. This value confers no authority.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct QaInboxNotificationGroup {
    pub topic: QaInboxNotificationTopic,
    pub severity: QaInboxNotificationSeverity,
    pub unread: u8,
    pub total: u8,
}

impl QaInboxNotificationGroup {
    #[must_use]
    pub const fn new(
        topic: QaInboxNotificationTopic,
        severity: QaInboxNotificationSeverity,
        unread: u8,
        total: u8,
    ) -> Self {
        Self {
            topic,
            severity,
            unread,
            total,
        }
    }
}

impl fmt::Display for QaInboxNotificationGroup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let topic = match self.topic {
            QaInboxNotificationTopic::Connection => "connection",
            QaInboxNotificationTopic::Jobs => "job activity",
            QaInboxNotificationTopic::Readiness(check) => check.as_str(),
        };
        let severity = match self.severity {
            QaInboxNotificationSeverity::Informational => "informational",
            QaInboxNotificationSeverity::Warning => "warning",
            QaInboxNotificationSeverity::Critical => "critical",
        };
        write!(
            f,
            "{}, {severity} ({} unread of {})",
            topic.replace('_', " "),
            self.unread,
            self.total
        )
    }
}

/// Whether the question asks about the operator's visible Inbox notices.
#[must_use]
pub fn is_inbox_notification_question(question: &str) -> bool {
    let mut names_inbox = false;
    let mut refers_to_local_items = false;
    for word in question.split(|ch: char| !ch.is_ascii_alphabetic()) {
        names_inbox |= ["inbox", "notification", "notifications"]
            .iter()
            .any(|candidate| word.eq_ignore_ascii_case(candidate));
        refers_to_local_items |= [
            "my", "our", "these", "unread", "current", "recent", "latest", "today", "here",
        ]
        .iter()
        .any(|candidate| word.eq_ignore_ascii_case(candidate));
        if names_inbox && refers_to_local_items {
            return true;
        }
    }
    false
}

/// One question. The backend selects evidence and may receive a native Inbox observation.
#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct QaTurnRequest {
    pub request_id: String,
    pub context: MutationContext,
    pub question: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<ChatSourceReference>,
    /// A summary of the submitting desktop's Inbox for an explicit notification question.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inbox_notifications: Option<Vec<QaInboxNotificationGroup>>,
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
            inbox_notifications: None,
        };
        request.validate()?;
        Ok(request)
    }

    /// Attach a bounded Inbox summary to a question for a compatible Embedded companion.
    pub fn with_inbox_notifications(
        mut self,
        groups: Vec<QaInboxNotificationGroup>,
    ) -> Result<Self, ChatInputError> {
        self.inbox_notifications = Some(groups);
        self.validate()?;
        Ok(self)
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
        if let Some(groups) = &self.inbox_notifications {
            if self.source.is_some()
                || !is_inbox_notification_question(&self.question)
                || groups.len() > QA_MAX_INBOX_NOTIFICATION_GROUPS
            {
                return Err(ChatInputError::InvalidInboxNotifications);
            }
            let mut topics = BTreeSet::new();
            let mut total = 0_u16;
            for group in groups {
                total += u16::from(group.total);
                if group.total == 0
                    || group.unread > group.total
                    || total > u16::from(QA_MAX_INBOX_NOTIFICATION_ITEMS)
                    || !topics.insert(group.topic)
                {
                    return Err(ChatInputError::InvalidInboxNotifications);
                }
            }
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

/// Why a negotiated worker could not answer this turn.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum QaTurnLimitation {
    CapabilityUnavailable,
    QuestionTooLong,
    #[serde(other)]
    Unknown,
}

/// A conversational reply that asks for input or explains supported assistance.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum QaConversationReply {
    Acknowledgement,
    Help,
    ContextNeeded,
    FollowUpNeedsTopic,
    ChooseSubject,
    RecallNeedsHistory,
    #[serde(other)]
    Unknown,
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

/// The age and origin of a cited observation. Missing metadata in older
/// responses retains the original timestamp and label.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum QaEvidenceProvenance {
    Current,
    PreviousObservation,
    Historical {
        sequence: u64,
        coverage_start_ms: i64,
        has_gaps: bool,
    },
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
    pub provenance: Option<QaEvidenceProvenance>,
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
            provenance: None,
            measurement: None,
        }
    }
}

/// Actual executor work within a running turn, independent of model readiness.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum QaTurnStage {
    Working,
    Thinking,
    Writing,
    #[serde(other)]
    Unknown,
}

/// The authoritative exchange and its highest committed event sequence.
#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct QaTurn {
    pub turn_id: String,
    pub request_id: String,
    pub question: String,
    /// The backend admission time in UTC Unix seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepted_at: Option<i64>,
    /// The terminal transition time in UTC Unix seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<i64>,
    /// Elapsed milliseconds from backend request receipt through the terminal transition.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_time_ms: Option<u64>,
    /// The backend resolved this turn using a previous exchange.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub used_previous_exchange: bool,
    pub state: QaTurnState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<QaTurnStage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limitation: Option<QaTurnLimitation>,
    pub findings: Vec<QaFinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notice: Option<QaTurnNotice>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation: Option<QaConversationReply>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<QaContextUsage>,
    /// The verified generator that produced a nonzero context usage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_target: Option<QaModelTarget>,
    /// The generation under which that worker handled this turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_generation: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provisional_reply: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub related_activity: Option<QaRelatedActivity>,
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
            accepted_at: None,
            completed_at: None,
            response_time_ms: None,
            used_previous_exchange: false,
            state: QaTurnState::Queued,
            stage: None,
            limitation: None,
            findings: Vec::new(),
            notice: None,
            conversation: None,
            context: None,
            model_target: None,
            model_generation: None,
            reply: None,
            provisional_reply: None,
            related_activity: None,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<QaTurnStage>,
    pub finding: Option<QaFinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provisional_delta: Option<String>,
}

impl QaEvent {
    /// Attach an independently validated part or record a state transition.
    #[must_use]
    pub fn new(sequence: u64, state: QaTurnState, finding: Option<QaFinding>) -> Self {
        Self {
            sequence,
            state,
            stage: None,
            finding,
            provisional_delta: None,
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
    fn assistant_preferences_are_independent_of_legacy_toggles() {
        let legacy = serde_json::json!({
            "context": {
                "registration_id": "registration-1",
                "expected_registration_revision": 1,
                "expected_context_revision": 1,
            },
            "expected_revision": 5,
            "enabled": false,
        });
        let toggle: QaSettingsUpdate = serde_json::from_value(legacy.clone()).unwrap();
        assert_eq!(toggle.enabled, Some(false));
        assert_eq!(toggle.generator_preference, None);
        assert_eq!(toggle.decision_preference, None);
        assert_eq!(serde_json::to_value(&toggle).unwrap(), legacy);
        let patch = QaSettingsUpdate::preferences(toggle.context, 5)
            .with_generator_preference(QaGeneratorPreference::FlanT5Small)
            .with_decision_preference(QaDecisionPreference::VllmD3EdgeQ8);
        let encoded = serde_json::to_value(&patch).unwrap();
        assert!(encoded.get("enabled").is_none());
        assert_eq!(encoded["generator_preference"], "flan_t5_small");
        assert_eq!(encoded["decision_preference"], "vllm_d3_edge_q8");
        assert_eq!(
            serde_json::from_value::<QaSettingsUpdate>(encoded).unwrap(),
            patch
        );
    }

    #[test]
    fn related_activity_rejects_untyped_inputs_and_keeps_targets_internal() {
        let request = serde_json::json!({"source": {"kind": "job", "id": "job-1"}});
        assert!(
            serde_json::from_value::<QaRelatedActivityRequest>(request.clone())
                .unwrap()
                .validate()
                .is_ok()
        );
        let mut supplied_evidence = request;
        supplied_evidence["candidate"] = serde_json::json!({"kind": "job", "id": "job-2"});
        assert!(serde_json::from_value::<QaRelatedActivityRequest>(supplied_evidence).is_err());
        let unsupported =
            ChatSourceReference::new(ChatSourceKind::Actor, "actor-1".into()).unwrap();
        assert!(QaRelatedActivityRequest::new(unsupported).is_err());

        let result = serde_json::json!({
            "status": "found",
            "target": {"kind": "readiness", "id": "gateway"},
            "reason": "current_status",
            "url": "https://example.invalid"
        });
        assert!(serde_json::from_value::<QaRelatedActivity>(result).is_err());
    }

    #[test]
    fn turn_context_cannot_be_supplied_by_the_frontend() {
        let valid = serde_json::json!({
            "request_id": "request-1",
            "context": {
                "registration_id": "registration-1",
                "expected_registration_revision": 1,
                "expected_context_revision": 1,
            },
            "question": "Which checks passed?",
        });
        assert!(serde_json::from_value::<QaTurnRequest>(valid.clone()).is_ok());
        for field in ["evidence", "history"] {
            let mut supplied = valid.clone();
            supplied[field] = serde_json::json!([]);
            assert!(serde_json::from_value::<QaTurnRequest>(supplied).is_err());
        }
    }

    #[test]
    fn inbox_notification_context_accepts_only_bounded_typed_local_observations() {
        let legacy = serde_json::json!({
            "request_id": "request-1",
            "context": {
                "registration_id": "registration-1",
                "expected_registration_revision": 1,
                "expected_context_revision": 1,
            },
            "question": "what are my most important notifications?",
        });
        let request = serde_json::from_value::<QaTurnRequest>(legacy.clone()).unwrap();
        assert_eq!(serde_json::to_value(&request).unwrap(), legacy);

        let group = QaInboxNotificationGroup::new(
            QaInboxNotificationTopic::Readiness(ReadinessCheckId::Registration),
            QaInboxNotificationSeverity::Warning,
            1,
            2,
        );
        let request = request.with_inbox_notifications(vec![group]).unwrap();
        assert_eq!(group.to_string(), "registration, warning (1 unread of 2)");
        let valid = serde_json::to_value(&request).unwrap();
        assert_eq!(
            serde_json::from_value::<QaTurnRequest>(valid.clone()).unwrap(),
            request
        );

        let invalid_groups = [
            vec![group, group],
            vec![QaInboxNotificationGroup::new(
                group.topic,
                group.severity,
                3,
                2,
            )],
            vec![
                QaInboxNotificationGroup::new(group.topic, group.severity, 1, 50),
                QaInboxNotificationGroup::new(
                    QaInboxNotificationTopic::Connection,
                    group.severity,
                    1,
                    1,
                ),
            ],
        ];
        for groups in invalid_groups {
            assert_eq!(
                request.clone().with_inbox_notifications(groups),
                Err(ChatInputError::InvalidInboxNotifications)
            );
        }

        let mut unrelated = request;
        unrelated.question = "How do browser notifications work?".into();
        assert_eq!(
            unrelated.validate(),
            Err(ChatInputError::InvalidInboxNotifications)
        );
        let mut injected = valid;
        injected["inbox_notifications"][0]["title"] = serde_json::json!("untrusted text");
        assert!(serde_json::from_value::<QaTurnRequest>(injected).is_err());
    }

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

    #[test]
    fn turn_stage_and_limitations_decode_from_optional_transport_fields() {
        let mut turn = QaTurn::queued("turn".into(), "request".into(), "Current status?".into());
        let legacy = serde_json::to_value(&turn).unwrap();
        let decoded: QaTurn = serde_json::from_value(legacy.clone()).unwrap();
        assert!(legacy.get("stage").is_none());
        assert!(legacy.get("limitation").is_none());
        assert_eq!(decoded, turn);

        turn.state = QaTurnState::Running;
        turn.stage = Some(QaTurnStage::Thinking);
        let reading = QaEventPage::new(turn, Vec::new(), true);
        let serialized = serde_json::to_value(&reading).unwrap();
        assert_eq!(serialized["turn"]["stage"], "thinking");
        let decoded: QaEventPage = serde_json::from_value(serialized).unwrap();
        assert_eq!(decoded.turn.stage, Some(QaTurnStage::Thinking));
        assert!(decoded.gap);
    }

    #[test]
    fn embedded_startup_stage_is_optional_and_typed() {
        let mut settings = QaSettings::new(false, QaPhase::Off, 1, 1, true);
        let old = serde_json::to_value(settings).unwrap();
        assert!(old.get("startup_stage").is_none());
        assert_eq!(serde_json::from_value::<QaSettings>(old).unwrap(), settings);

        settings.startup_stage = Some(QaStartupStage::DownloadingPack);
        let current = serde_json::to_value(settings).unwrap();
        assert_eq!(current["startup_stage"], "downloading_pack");
        assert_eq!(current["phase"], "off");
        assert_eq!(
            serde_json::from_value::<QaSettings>(current).unwrap(),
            settings
        );
    }

    #[test]
    fn conversational_reply_preserves_the_legacy_notice() {
        let mut turn = QaTurn::queued("turn".into(), "request".into(), "Can you help?".into());
        turn.state = QaTurnState::InsufficientContext;
        turn.notice = Some(QaTurnNotice::Greeting);
        let old = serde_json::to_value(&turn).unwrap();
        assert!(old.get("conversation").is_none());
        assert_eq!(serde_json::from_value::<QaTurn>(old).unwrap(), turn);

        turn.conversation = Some(QaConversationReply::Help);
        let mut payload = serde_json::to_value(&turn).unwrap();
        assert_eq!(payload["notice"], "greeting");
        assert_eq!(payload["conversation"], "help");
        payload["conversation"] = "future_reply".into();
        let decoded = serde_json::from_value::<QaTurn>(payload).unwrap();
        assert_eq!(decoded.conversation, Some(QaConversationReply::Unknown));
        assert_eq!(decoded.notice, turn.notice);
    }
}
