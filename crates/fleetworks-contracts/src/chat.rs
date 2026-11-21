//! Typed contract for the contextual Assistant chat.
//!
//! V1 serves only `POST turns`, over HTTP and IPC. It checks the caller and the request, then
//! answers `ChatTurnAck` ("ok"). It stores nothing and never replies with an assistant message.
//! The thread, message and event reads below are typed for later and are not served yet.
//! A caller names a typed source reference. It never supplies evidence: the server resolves
//! evidence from its own authorized data. Replies are a status, then one final answer. There is
//! no partial text and no tool execution.
//!
//! Permissions (the server rechecks them): reads need `ReadWorkstationActivity` and a
//! registration; submit needs `SubmitInteractive` and a registration.

mod qa;

pub use qa::*;

use std::fmt;

use serde::Deserialize;
use serde::Serialize;

use crate::context::MutationContext;

/// Submit one turn (POST). Reads below are GET.
pub const CHAT_TURNS_PATH: &str = "/api/v1/workstation/chat/turns";
pub const CHAT_THREADS_PATH: &str = "/api/v1/workstation/chat/threads";
pub const CHAT_MESSAGES_PATH: &str = "/api/v1/workstation/chat/messages";
pub const CHAT_EVENTS_PATH: &str = "/api/v1/workstation/chat/events";

/// Version a backend advertises in `ChatCapabilities`.
pub const CHAT_VERSION: u16 = 1;
/// Operator input, in UTF-8 bytes.
pub const MAX_INPUT_BYTES: usize = 4 * 1024;
/// Evidence the server captures for one turn, in UTF-8 bytes. Never client supplied.
pub const MAX_EVIDENCE_BYTES: usize = 64 * 1024;
/// A final answer, in UTF-8 bytes.
pub const MAX_ANSWER_BYTES: usize = 32 * 1024;
pub const MAX_MESSAGES_PER_PAGE: usize = 50;
pub const MAX_EVENTS_PER_PAGE: usize = 100;
/// Longest an events read may wait for news.
pub const MAX_EVENT_WAIT_SECS: u32 = 4;
/// Serialized size of one page.
pub const MAX_PAGE_BYTES: usize = 256 * 1024;
pub const MAX_CITATIONS: usize = 16;
const MAX_ID_BYTES: usize = 128;
const MAX_REQUEST_ID_BYTES: usize = 64;
const MAX_LABEL_BYTES: usize = 120;
const MAX_REASON_BYTES: usize = 200;

/// Why a value is not acceptable. The text never echoes the value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ChatInputError {
    InvalidRequestId,
    InvalidIdentity,
    EmptyInput,
    InputTooLong,
    ControlCharacter,
    RevisionWithoutThread,
    InvalidLabel,
    TooManyCitations,
    AnswerTooLong,
    PageTooLarge,
    InvalidLimit,
    InvalidWait,
}

impl fmt::Display for ChatInputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidRequestId => "Invalid chat request ID",
            Self::InvalidIdentity => "Invalid chat identity",
            Self::EmptyInput => "Chat input is empty",
            Self::InputTooLong => "Chat input is over the 4 KiB limit",
            Self::ControlCharacter => "Chat input holds a control character",
            Self::RevisionWithoutThread => "A thread revision needs a thread ID",
            Self::InvalidLabel => "Invalid chat source label",
            Self::TooManyCitations => "Too many chat sources",
            Self::AnswerTooLong => "Chat answer is over the 32 KiB limit",
            Self::PageTooLarge => "Chat page is over its size limit",
            Self::InvalidLimit => "Invalid chat page limit",
            Self::InvalidWait => "Invalid chat wait",
        })
    }
}

impl std::error::Error for ChatInputError {}

fn identity(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
}

fn request_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_REQUEST_ID_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
}

fn label(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}

/// Whether a backend offers chat. Unsupported and unavailable differ from a permission or a
/// configuration problem, and each says why sending is off.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ChatAvailability {
    /// The backend implements the contract and can take turns now.
    Supported,
    /// This backend does not implement chat at all.
    Unsupported,
    /// It implements chat but cannot take turns right now.
    Unavailable,
}

/// Not `deny_unknown_fields`: a newer backend may add fields without breaking capability decode.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct ChatCapabilities {
    pub version: u16,
    pub availability: ChatAvailability,
}

impl ChatCapabilities {
    #[must_use]
    pub fn new(availability: ChatAvailability) -> Self {
        Self {
            version: CHAT_VERSION,
            availability,
        }
    }

    /// A client may send only on an exact version match that is `Supported`.
    #[must_use]
    pub fn can_send(&self) -> bool {
        self.version == CHAT_VERSION && self.availability == ChatAvailability::Supported
    }
}

/// What a turn is about. The server loads evidence for it; the client never sends any.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ChatSourceKind {
    Actor,
    Job,
    Cycle,
    Readiness,
    /// A sample from the selected backend's host metrics feed.
    HostMetric,
    /// A count from the registered workstation's activity scope.
    JobCount,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ChatSourceReference {
    pub kind: ChatSourceKind,
    pub id: String,
}

impl ChatSourceReference {
    pub fn new(kind: ChatSourceKind, id: String) -> Result<Self, ChatInputError> {
        let value = Self { kind, id };
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), ChatInputError> {
        if identity(&self.id, MAX_ID_BYTES) {
            Ok(())
        } else {
            Err(ChatInputError::InvalidIdentity)
        }
    }
}

/// One operator turn. The durable `request_id` makes a replay reuse the accepted turn.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ChatTurnRequest {
    pub request_id: String,
    pub context: MutationContext,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_thread_revision: Option<u64>,
    pub source: ChatSourceReference,
    pub input: String,
}

impl ChatTurnRequest {
    pub fn new(
        request_id: String,
        context: MutationContext,
        thread_id: Option<String>,
        expected_thread_revision: Option<u64>,
        source: ChatSourceReference,
        input: String,
    ) -> Result<Self, ChatInputError> {
        let value = Self {
            request_id,
            context,
            thread_id,
            expected_thread_revision,
            source,
            input,
        };
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), ChatInputError> {
        if !request_id(&self.request_id) {
            return Err(ChatInputError::InvalidRequestId);
        }
        if let Some(thread) = &self.thread_id {
            if !identity(thread, MAX_ID_BYTES) {
                return Err(ChatInputError::InvalidIdentity);
            }
        }
        if self.expected_thread_revision.is_some() && self.thread_id.is_none() {
            return Err(ChatInputError::RevisionWithoutThread);
        }
        self.source.validate()?;
        if self.input.trim().is_empty() {
            return Err(ChatInputError::EmptyInput);
        }
        if self.input.len() > MAX_INPUT_BYTES {
            return Err(ChatInputError::InputTooLong);
        }
        if self
            .input
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t')
        {
            return Err(ChatInputError::ControlCharacter);
        }
        Ok(())
    }
}

/// The only reply V1 gives to a turn: the endpoint was reached and the request was valid.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ChatTurnAck {
    pub status: String,
}

impl ChatTurnAck {
    #[must_use]
    pub fn ok() -> Self {
        Self {
            status: "ok".to_owned(),
        }
    }

    #[must_use]
    pub fn is_ok(&self) -> bool {
        self.status == "ok"
    }
}

/// The real state of a turn, as the server holds it.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ChatTurnState {
    Queued,
    Running,
    Retrying,
    Final,
    Failed,
}

/// Returned once a turn, its message, job and outbox entry are durably admitted. Acceptance is
/// not an answer.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ChatTurnReceipt {
    pub request_id: String,
    pub thread_id: String,
    pub turn_id: String,
    pub message_id: String,
    pub job_id: String,
    /// Thread revision after this turn.
    pub revision: u64,
    pub state: ChatTurnState,
    /// True when this request ID was already accepted and nothing new was created.
    pub reused: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ChatRole {
    Operator,
    Assistant,
}

/// A source an answer rests on: a kind, an ID and a short label. Never evidence text.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ChatCitation {
    pub kind: ChatSourceKind,
    pub id: String,
    pub label: String,
}

impl ChatCitation {
    pub fn validate(&self) -> Result<(), ChatInputError> {
        if !identity(&self.id, MAX_ID_BYTES) {
            return Err(ChatInputError::InvalidIdentity);
        }
        if !label(&self.label, MAX_LABEL_BYTES) {
            return Err(ChatInputError::InvalidLabel);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ChatMessage {
    pub message_id: String,
    pub thread_id: String,
    pub turn_id: String,
    pub role: ChatRole,
    pub state: ChatTurnState,
    /// Empty until the turn is final or failed.
    pub text: String,
    #[serde(default)]
    pub citations: Vec<ChatCitation>,
}

impl ChatMessage {
    pub fn validate(&self) -> Result<(), ChatInputError> {
        for id in [&self.message_id, &self.thread_id, &self.turn_id] {
            if !identity(id, MAX_ID_BYTES) {
                return Err(ChatInputError::InvalidIdentity);
            }
        }
        let limit = match self.role {
            ChatRole::Operator => MAX_INPUT_BYTES,
            ChatRole::Assistant => MAX_ANSWER_BYTES,
        };
        if self.text.len() > limit {
            return Err(match self.role {
                ChatRole::Operator => ChatInputError::InputTooLong,
                ChatRole::Assistant => ChatInputError::AnswerTooLong,
            });
        }
        if self.citations.len() > MAX_CITATIONS {
            return Err(ChatInputError::TooManyCitations);
        }
        self.citations.iter().try_for_each(ChatCitation::validate)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ChatThread {
    pub thread_id: String,
    pub revision: u64,
    pub source: ChatSourceReference,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_state: Option<ChatTurnState>,
}

/// One step in a turn's life. Events are ordered by `sequence` and resumable from it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ChatEvent {
    pub sequence: u64,
    pub thread_id: String,
    pub turn_id: String,
    pub state: ChatTurnState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt: Option<u32>,
    /// A short, safe reason for a failure. Never a provider message or a path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl ChatEvent {
    pub fn validate(&self) -> Result<(), ChatInputError> {
        for id in [&self.thread_id, &self.turn_id] {
            if !identity(id, MAX_ID_BYTES) {
                return Err(ChatInputError::InvalidIdentity);
            }
        }
        match &self.reason {
            Some(reason) if !label(reason, MAX_REASON_BYTES) => Err(ChatInputError::InvalidLabel),
            _ => Ok(()),
        }
    }
}

fn within_page_bytes<T: Serialize>(page: &T) -> Result<(), ChatInputError> {
    match serde_json::to_vec(page) {
        Ok(bytes) if bytes.len() <= MAX_PAGE_BYTES => Ok(()),
        _ => Err(ChatInputError::PageTooLarge),
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ChatThreadsPage {
    pub threads: Vec<ChatThread>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ChatMessagesPage {
    pub messages: Vec<ChatMessage>,
    pub revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

impl ChatMessagesPage {
    pub fn validate(&self) -> Result<(), ChatInputError> {
        if self.messages.len() > MAX_MESSAGES_PER_PAGE {
            return Err(ChatInputError::InvalidLimit);
        }
        self.messages.iter().try_for_each(ChatMessage::validate)?;
        within_page_bytes(self)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ChatEventsPage {
    pub events: Vec<ChatEvent>,
    /// Pass this as `after` to continue. It does not move back.
    pub next_sequence: u64,
}

impl ChatEventsPage {
    pub fn validate(&self) -> Result<(), ChatInputError> {
        if self.events.len() > MAX_EVENTS_PER_PAGE {
            return Err(ChatInputError::InvalidLimit);
        }
        self.events.iter().try_for_each(ChatEvent::validate)?;
        within_page_bytes(self)
    }
}

/// Query for one thread's messages.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
#[non_exhaustive]
pub struct ChatMessagesQuery {
    pub thread_id: String,
    pub limit: Option<usize>,
    pub cursor: Option<String>,
}

impl ChatMessagesQuery {
    pub fn validate(&self) -> Result<(), ChatInputError> {
        if !identity(&self.thread_id, MAX_ID_BYTES) {
            return Err(ChatInputError::InvalidIdentity);
        }
        if !(1..=MAX_MESSAGES_PER_PAGE).contains(&self.limit.unwrap_or(MAX_MESSAGES_PER_PAGE)) {
            return Err(ChatInputError::InvalidLimit);
        }
        Ok(())
    }
}

/// Query for one thread's events after a sequence, optionally waiting for news.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
#[non_exhaustive]
pub struct ChatEventsQuery {
    pub thread_id: String,
    pub after: u64,
    pub limit: Option<usize>,
    pub wait_secs: Option<u32>,
}

impl ChatEventsQuery {
    pub fn validate(&self) -> Result<(), ChatInputError> {
        if !identity(&self.thread_id, MAX_ID_BYTES) {
            return Err(ChatInputError::InvalidIdentity);
        }
        if !(1..=MAX_EVENTS_PER_PAGE).contains(&self.limit.unwrap_or(MAX_EVENTS_PER_PAGE)) {
            return Err(ChatInputError::InvalidLimit);
        }
        if self.wait_secs.unwrap_or(0) > MAX_EVENT_WAIT_SECS {
            return Err(ChatInputError::InvalidWait);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> MutationContext {
        serde_json::from_value(serde_json::json!({
            "registration_id": "registration-a",
            "expected_registration_revision": 1,
            "expected_context_revision": 1
        }))
        .expect("context")
    }

    fn source() -> ChatSourceReference {
        ChatSourceReference::new(ChatSourceKind::Job, "job-1".into()).expect("source")
    }

    fn turn(input: &str) -> Result<ChatTurnRequest, ChatInputError> {
        ChatTurnRequest::new(
            "turn-1".into(),
            context(),
            None,
            None,
            source(),
            input.into(),
        )
    }

    #[test]
    fn a_turn_is_bounded_and_never_carries_evidence() {
        assert!(turn("Why is this job slow?").is_ok());
        assert!(turn("two\nlines\tand a tab").is_ok());
        assert_eq!(turn("   ").unwrap_err(), ChatInputError::EmptyInput);
        assert_eq!(turn(&"x".repeat(MAX_INPUT_BYTES)).map(|_| ()), Ok(()));
        assert_eq!(
            turn(&"x".repeat(MAX_INPUT_BYTES + 1)).unwrap_err(),
            ChatInputError::InputTooLong
        );
        assert_eq!(
            turn("bad\u{0}byte").unwrap_err(),
            ChatInputError::ControlCharacter
        );
        let extra = serde_json::json!({
            "request_id": "turn-1",
            "context": {"registration_id": "registration-a", "expected_registration_revision": 1, "expected_context_revision": 1},
            "source": {"kind": "job", "id": "job-1"},
            "input": "hi",
            "evidence": "client supplied"
        });
        assert!(serde_json::from_value::<ChatTurnRequest>(extra).is_err());
    }

    #[test]
    fn identities_and_the_revision_precondition_are_checked() {
        let bad_request = ChatTurnRequest::new(
            "bad id".into(),
            context(),
            None,
            None,
            source(),
            "hi".into(),
        );
        assert_eq!(bad_request.unwrap_err(), ChatInputError::InvalidRequestId);
        let revision_alone =
            ChatTurnRequest::new("t".into(), context(), None, Some(3), source(), "hi".into());
        assert_eq!(
            revision_alone.unwrap_err(),
            ChatInputError::RevisionWithoutThread
        );
        let traversal = ChatTurnRequest::new(
            "t".into(),
            context(),
            Some("../x".into()),
            Some(1),
            source(),
            "hi".into(),
        );
        assert_eq!(traversal.unwrap_err(), ChatInputError::InvalidIdentity);
        assert!(ChatSourceReference::new(ChatSourceKind::Actor, "a/b".into()).is_err());
    }

    #[test]
    fn only_an_exact_supported_version_may_send() {
        assert!(ChatCapabilities::new(ChatAvailability::Supported).can_send());
        assert!(!ChatCapabilities::new(ChatAvailability::Unsupported).can_send());
        assert!(!ChatCapabilities::new(ChatAvailability::Unavailable).can_send());
        let mut future = ChatCapabilities::new(ChatAvailability::Supported);
        future.version = CHAT_VERSION + 1;
        assert!(!future.can_send());
    }

    fn message(role: ChatRole, text: String) -> ChatMessage {
        ChatMessage {
            message_id: "m-1".into(),
            thread_id: "th-1".into(),
            turn_id: "tn-1".into(),
            role,
            state: ChatTurnState::Final,
            text,
            citations: Vec::new(),
        }
    }

    #[test]
    fn answers_citations_and_pages_stay_inside_their_bounds() {
        assert!(message(ChatRole::Assistant, "x".repeat(MAX_ANSWER_BYTES))
            .validate()
            .is_ok());
        assert_eq!(
            message(ChatRole::Assistant, "x".repeat(MAX_ANSWER_BYTES + 1))
                .validate()
                .unwrap_err(),
            ChatInputError::AnswerTooLong
        );
        assert_eq!(
            message(ChatRole::Operator, "x".repeat(MAX_INPUT_BYTES + 1))
                .validate()
                .unwrap_err(),
            ChatInputError::InputTooLong
        );
        let citation = |id: &str, text: &str| ChatCitation {
            kind: ChatSourceKind::Job,
            id: id.into(),
            label: text.into(),
        };
        let mut with_sources = message(ChatRole::Assistant, "ok".into());
        with_sources.citations = vec![citation("job-1", "Job one")];
        assert!(with_sources.validate().is_ok());
        with_sources.citations = vec![citation("job-1", "line\nbreak")];
        assert_eq!(
            with_sources.validate().unwrap_err(),
            ChatInputError::InvalidLabel
        );
        with_sources.citations = (0..=MAX_CITATIONS).map(|_| citation("j", "L")).collect();
        assert_eq!(
            with_sources.validate().unwrap_err(),
            ChatInputError::TooManyCitations
        );

        let full = ChatMessagesPage {
            messages: (0..=MAX_MESSAGES_PER_PAGE)
                .map(|_| message(ChatRole::Assistant, String::new()))
                .collect(),
            revision: 1,
            next_cursor: None,
        };
        assert_eq!(full.validate().unwrap_err(), ChatInputError::InvalidLimit);
        let heavy = ChatMessagesPage {
            messages: (0..MAX_MESSAGES_PER_PAGE)
                .map(|_| message(ChatRole::Assistant, "x".repeat(MAX_ANSWER_BYTES)))
                .collect(),
            revision: 1,
            next_cursor: None,
        };
        assert_eq!(heavy.validate().unwrap_err(), ChatInputError::PageTooLarge);
    }

    #[test]
    fn event_and_message_queries_bound_limits_and_waiting() {
        let events = |limit, wait| ChatEventsQuery {
            thread_id: "th-1".into(),
            after: 0,
            limit,
            wait_secs: wait,
        };
        assert!(events(None, None).validate().is_ok());
        assert!(events(Some(MAX_EVENTS_PER_PAGE), Some(MAX_EVENT_WAIT_SECS))
            .validate()
            .is_ok());
        assert_eq!(
            events(Some(0), None).validate().unwrap_err(),
            ChatInputError::InvalidLimit
        );
        assert_eq!(
            events(Some(MAX_EVENTS_PER_PAGE + 1), None)
                .validate()
                .unwrap_err(),
            ChatInputError::InvalidLimit
        );
        assert_eq!(
            events(None, Some(MAX_EVENT_WAIT_SECS + 1))
                .validate()
                .unwrap_err(),
            ChatInputError::InvalidWait
        );
        let messages = ChatMessagesQuery {
            thread_id: "th-1".into(),
            limit: Some(MAX_MESSAGES_PER_PAGE + 1),
            cursor: None,
        };
        assert_eq!(
            messages.validate().unwrap_err(),
            ChatInputError::InvalidLimit
        );
        let event = ChatEvent {
            sequence: 1,
            thread_id: "th-1".into(),
            turn_id: "tn-1".into(),
            state: ChatTurnState::Failed,
            attempt: Some(2),
            reason: Some("backend restarted".into()),
        };
        assert!(event.validate().is_ok());
        let mut bad = event;
        bad.reason = Some("a\nb".into());
        assert_eq!(bad.validate().unwrap_err(), ChatInputError::InvalidLabel);
    }

    #[test]
    fn the_acknowledgement_is_a_plain_ok() {
        let ack = ChatTurnAck::ok();
        assert!(ack.is_ok());
        assert_eq!(
            serde_json::to_value(&ack).expect("json"),
            serde_json::json!({"status": "ok"})
        );
    }

    #[test]
    fn states_use_snake_case_on_the_wire() {
        let receipt: ChatTurnReceipt = serde_json::from_value(serde_json::json!({
            "request_id": "turn-1", "thread_id": "th-1", "turn_id": "tn-1",
            "message_id": "m-1", "job_id": "j-1", "revision": 2,
            "state": "queued", "reused": false
        }))
        .expect("receipt decodes");
        assert_eq!(receipt.state, ChatTurnState::Queued);
        assert_eq!(
            serde_json::to_value(ChatAvailability::Unsupported).expect("json"),
            serde_json::json!("unsupported")
        );
    }
}
