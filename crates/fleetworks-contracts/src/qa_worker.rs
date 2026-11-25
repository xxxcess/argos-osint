//! Private protocol between the trusted Inbox QA executor and the model process.
//!
//! This module is also compiled into the isolated worker, without the application's other
//! dependencies. Only the question and frozen evidence cross this boundary.

use std::collections::BTreeSet;
use std::io;

use serde::Deserialize;
use serde::Serialize;

pub const QA_WORKER_PROTOCOL: u16 = 1;
pub const QA_CONTEXT_WINDOW_TOKENS: u16 = 256;
pub const QA_CONTEXT_USAGE_ENV: &str = "FLEETWORKS_QA_CONTEXT_USAGE";
pub const QA_STREAMING_ENV: &str = "FLEETWORKS_QA_STREAMING";
pub const QA_MODEL_PINS: &str = include_str!("qa_models.json");
pub const QA_MODEL_TARGET_ENV: &str = "FLEETWORKS_QA_MODEL_TARGET";
pub const QA_DIALOGUE_ENV: &str = "FLEETWORKS_QA_DIALOGUE";
pub const QA_MODEL_REVISION: &str = "564e9b582944a57a3e586bbb98fd6f0a4118db7f";
pub const QA_CHAT_MODEL_REVISION: &str = "0fc9ddf78a1e988dac52e2dac162b0ede4fd74ab";
pub const QA_MODEL_FILES: [(&str, &str); 3] = [
    (
        "config.json",
        "0b5cb15ec08645604ef7085acfaf9c4131158ac22207a76634574cf2771b1515",
    ),
    (
        "vocab.txt",
        "eeaa9875b23b04b4c54ef759d03db9d1ba1554838f8fb26c5d96fa551df93d02",
    ),
    (
        "rust_model.ot",
        "8a9f9b2f153ac9ff230aca4548fa3286be9d2f9ea4eb7e9169665b1a8e983f44",
    ),
];
pub const QA_GENERATOR_FILES: [(&str, &str); 3] = [
    (
        "config.json",
        "439aa0fecf5a5546a1def68b1fc45e538e2c94528ce805378daf091e2bf6e4de",
    ),
    (
        "spiece.model",
        "d60acb128cf7b7f2536e8f38a5b18a05535c9e14c7a355904270e15b0945ea86",
    ),
    (
        "model.safetensors",
        "495fa51e204676f1a857a9fc13c4c89f3f5ba9f480b898cebca02add25e6d749",
    ),
];

/// Pinned generation checkpoints understood by the companion and model process.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum QaModelTarget {
    #[default]
    #[serde(rename = "flan-t5-small")]
    FlanT5Small,
    #[serde(rename = "flan-t5-base")]
    FlanT5Base,
}

impl QaModelTarget {
    pub const ALL: [Self; 2] = [Self::FlanT5Base, Self::FlanT5Small];

    pub const fn slug(self) -> &'static str {
        match self {
            Self::FlanT5Small => "flan-t5-small",
            Self::FlanT5Base => "flan-t5-base",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::FlanT5Small => "FLAN-T5 Small",
            Self::FlanT5Base => "FLAN-T5 Base",
        }
    }

    pub fn from_slug(slug: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|target| target.slug() == slug)
    }

    pub const fn revision(self) -> &'static str {
        match self {
            Self::FlanT5Small => QA_CHAT_MODEL_REVISION,
            Self::FlanT5Base => "7bcac572ce56db69c1ea7c8af255c5d7c9672fc2",
        }
    }

    pub const fn weight_bytes(self) -> u64 {
        match self {
            Self::FlanT5Small => 307_867_048,
            Self::FlanT5Base => 990_345_061,
        }
    }

    pub const fn input_tokens(self) -> u16 {
        512
    }

    pub const fn output_tokens(self) -> u16 {
        128
    }
}

pub const QA_MAX_DRAFT_BYTES: usize = 4 * 1024;
pub const QA_MAX_DRAFT_EVENTS: u32 = 128;

pub fn valid_draft_delta(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= QA_MAX_DRAFT_BYTES
        && !text
            .chars()
            .any(|character| character.is_control() && character != '\n' && character != '\t')
}

/// Room for bounded evidence, JSON escaping, and the envelope.
pub const QA_MAX_FRAME_BYTES: usize = 512 * 1024;
pub const QA_MAX_QUESTION_BYTES: usize = 4 * 1024;
pub const QA_MAX_EVIDENCE_BYTES: usize = 64 * 1024;
pub const QA_MAX_PASSAGES: usize = 16;
pub const QA_MAX_FINDINGS: usize = 3;
pub const QA_MAX_VISIBLE_WORDS: usize = 60;
pub const QA_MAX_DIALOGUE_HISTORY: usize = 2;
pub const QA_MAX_DIALOGUE_MESSAGE_BYTES: usize = 1_024;
const MAX_ANSWER_BYTES: usize = 32 * 1024;
const MAX_OPAQUE_ID_BYTES: usize = 128;
const MAX_WINDOWS_PER_PASSAGE: usize = 3;

/// Model input usage for one turn. Peak excludes batch padding.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct QaContextUsage {
    pub window_tokens: u16,
    pub peak_input_tokens: u16,
    pub model_windows: u16,
}

impl QaContextUsage {
    pub const fn new(window_tokens: u16, peak_input_tokens: u16, model_windows: u16) -> Self {
        Self {
            window_tokens,
            peak_input_tokens,
            model_windows,
        }
    }

    fn is_valid_for(self, request: &QaWorkerRequest) -> bool {
        let max_windows = if request.dialogue.is_some() && request.passages.is_empty() {
            1
        } else {
            request.passages.len() * MAX_WINDOWS_PER_PASSAGE
        };
        (self.window_tokens == QA_CONTEXT_WINDOW_TOKENS
            || (request.dialogue.is_some()
                && QaModelTarget::ALL
                    .iter()
                    .any(|target| self.window_tokens == target.input_tokens())))
            && self.peak_input_tokens <= self.window_tokens
            && (self.model_windows == 0) == (self.peak_input_tokens == 0)
            && usize::from(self.model_windows) <= max_windows
    }
}

/// One immutable authorized passage. IDs identify distinct facts within a snapshot.
#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct QaPassage {
    pub source_id: String,
    pub text: String,
}

impl QaPassage {
    pub fn new(source_id: String, text: String) -> Self {
        Self { source_id, text }
    }
}

/// Input prepared by the trusted executor. This is never a client request body.
#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct QaWorkerRequest {
    pub turn_id: String,
    pub snapshot_id: String,
    pub question: String,
    pub passages: Vec<QaPassage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dialogue: Option<QaWorkerDialogue>,
}

impl QaWorkerRequest {
    pub fn new(
        turn_id: String,
        snapshot_id: String,
        question: String,
        passages: Vec<QaPassage>,
    ) -> Self {
        Self {
            turn_id,
            snapshot_id,
            question,
            passages,
            dialogue: None,
        }
    }

    /// Validate identifiers and size limits before work can enter the model.
    pub fn validate(&self) -> io::Result<()> {
        if !opaque_id(&self.turn_id)
            || !opaque_id(&self.snapshot_id)
            || !bounded_text(&self.question, QA_MAX_QUESTION_BYTES)
            || self.passages.len() > QA_MAX_PASSAGES
            || self.dialogue.as_ref().is_some_and(|dialogue| {
                (!self.passages.is_empty() && dialogue.kind != QaDialogueKind::Operational)
                    || (self.passages.is_empty() && dialogue.kind == QaDialogueKind::Operational)
                    || dialogue.history.len() > QA_MAX_DIALOGUE_HISTORY
                    || dialogue.history.iter().any(|exchange| {
                        !bounded_text(&exchange.question, QA_MAX_DIALOGUE_MESSAGE_BYTES)
                            || !bounded_text(&exchange.reply, QA_MAX_DIALOGUE_MESSAGE_BYTES)
                    })
            })
        {
            return Err(invalid("invalid QA worker request"));
        }
        let mut ids = BTreeSet::new();
        let mut bytes = 0;
        for passage in &self.passages {
            if !opaque_id(&passage.source_id)
                || !ids.insert(passage.source_id.as_str())
                || !bounded_text(&passage.text, QA_MAX_EVIDENCE_BYTES)
            {
                return Err(invalid("invalid QA evidence passage"));
            }
            bytes += passage.text.len();
            if bytes > QA_MAX_EVIDENCE_BYTES {
                return Err(invalid("QA evidence exceeds its size limit"));
            }
        }
        Ok(())
    }
}

/// A conversational request and at most two earlier exchanges in chronological order.
#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct QaWorkerDialogue {
    pub kind: QaDialogueKind,
    pub history: Vec<QaDialogueExchange>,
}

#[derive(Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QaDialogueKind {
    Greeting,
    Acknowledgement,
    Help,
    Recall,
    Operational,
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct QaDialogueExchange {
    pub question: String,
    pub reply: String,
}

pub fn valid_dialogue_reply(reply: &str) -> bool {
    bounded_text(reply, QA_MAX_DIALOGUE_MESSAGE_BYTES)
        && reply.split_whitespace().count() <= QA_MAX_VISIBLE_WORDS
        && !reply.chars().any(char::is_control)
        && !contains_prompt_marker(reply)
}

fn contains_prompt_marker(text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    [
        "earlier user",
        "earlier assistant",
        "current user",
        "approved observations:",
        "reply:",
    ]
    .iter()
    .any(|marker| text.contains(marker))
}

/// Admit only complete, quoted operational facts and harmless introductory wording.
pub fn verified_operational_sources<'a>(
    request: &'a QaWorkerRequest,
    reply: &str,
) -> Option<Vec<&'a QaPassage>> {
    if !valid_dialogue_reply(reply) {
        return None;
    }
    let mut selected = Vec::new();
    let mut first = true;
    for sentence in reply.split(". ").map(|part| part.trim_end_matches('.')) {
        if first
            && matches!(
                sentence,
                "Here are the current checks" | "Here is the current status"
            )
        {
            first = false;
            continue;
        }
        first = false;
        let passage = request.passages.iter().find(|passage| {
            passage
                .text
                .split(". ")
                .any(|fact| fact.trim_end_matches('.') == sentence)
        })?;
        if !selected.contains(&passage) {
            selected.push(passage);
        }
        if selected.len() > QA_MAX_FINDINGS {
            return None;
        }
    }
    (!selected.is_empty()).then_some(selected)
}

/// The last event a worker emits for one request.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum QaWorkerOutcome {
    Answered,
    InsufficientContext,
    Failed,
    Cancelled,
}

/// An extracted answer or terminal event. Offsets are half-open Unicode scalar positions.
#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum QaWorkerEvent {
    Delta {
        turn_id: String,
        sequence: u32,
        text: String,
    },
    Finding {
        turn_id: String,
        sequence: u32,
        source_id: String,
        start: u32,
        end: u32,
        text: String,
    },
    Complete {
        turn_id: String,
        sequence: u32,
        outcome: QaWorkerOutcome,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        context: Option<QaContextUsage>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reply: Option<String>,
    },
}

/// Commands only accepted over an executor-owned pipe or socket.
#[derive(Deserialize, Serialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum QaWorkerCommand {
    Open,
    OpenWithContext,
    OpenWithDialogue,
    OpenWithStreaming,
    Ping,
    Ask { request: QaWorkerRequest },
    Shutdown,
}

/// Private framing for handshake, answer events and a confirmed process unload.
#[derive(Deserialize, Serialize)]
#[serde(tag = "reply", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum QaWorkerReply {
    Ready {
        protocol: u16,
        model_revision: String,
    },
    ReadyWithDialogue {
        protocol: u16,
        model_revision: String,
    },
    ReadyWithStreaming {
        protocol: u16,
        model_revision: String,
    },
    StartupFailed {
        source: QaWorkerPanicSource,
        line: u32,
        cause: QaWorkerPanicCause,
    },
    Event {
        event: QaWorkerEvent,
    },
    Stopped,
}

/// Fixed diagnostic categories. Source paths and panic messages stay in the model process.
#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QaWorkerPanicSource {
    TorchUtils,
    TorchTensor,
    TorchOther,
    RustBert,
    Worker,
    Other,
}

impl QaWorkerPanicSource {
    pub fn code(self) -> &'static str {
        match self {
            Self::TorchUtils => "torch_utils",
            Self::TorchTensor => "torch_tensor",
            Self::TorchOther => "torch_other",
            Self::RustBert => "rust_bert",
            Self::Worker => "worker",
            Self::Other => "other",
        }
    }
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QaWorkerPanicCause {
    Memory,
    Permission,
    Threads,
    Other,
}

impl QaWorkerPanicCause {
    pub fn code(self) -> &'static str {
        match self {
            Self::Memory => "memory",
            Self::Permission => "permission",
            Self::Threads => "threads",
            Self::Other => "other",
        }
    }
}

/// A shared check used before any worker event can enter the durable event log.
#[derive(Default)]
pub struct QaWorkerSequence {
    delta_count: u32,
    draft: String,
    sources: BTreeSet<String>,
    bytes: usize,
    words: usize,
    terminal: bool,
}

impl QaWorkerSequence {
    pub fn next_sequence(&self) -> u32 {
        self.delta_count + self.sources.len() as u32 + 1
    }

    /// Require exactly one ordered stream for the given turn and validate every extracted span.
    pub fn accept(&mut self, request: &QaWorkerRequest, event: &QaWorkerEvent) -> io::Result<bool> {
        if self.terminal {
            return Err(invalid("QA worker sent an event after its terminal result"));
        }
        let expected = self.next_sequence();
        match event {
            QaWorkerEvent::Delta {
                turn_id,
                sequence,
                text,
            } => {
                if turn_id != &request.turn_id
                    || *sequence != expected
                    || request.dialogue.is_none()
                    || !valid_draft_delta(text)
                    || self.delta_count >= QA_MAX_DRAFT_EVENTS
                    || self.draft.len() + text.len() > QA_MAX_DRAFT_BYTES
                {
                    return Err(invalid("QA worker returned an invalid draft delta"));
                }
                let previous_length = self.draft.len();
                self.draft.push_str(text);
                if contains_prompt_marker(&self.draft)
                    || (request
                        .dialogue
                        .as_ref()
                        .is_some_and(|dialogue| dialogue.kind == QaDialogueKind::Operational)
                        && verified_operational_sources(request, &self.draft).is_none())
                {
                    self.draft.truncate(previous_length);
                    return Err(invalid("QA worker returned an invalid draft"));
                }
                self.delta_count += 1;
            }
            QaWorkerEvent::Finding {
                turn_id,
                sequence,
                source_id,
                start,
                end,
                text,
            } => {
                let passage = request.passages.iter().find(|p| &p.source_id == source_id);
                let words = self.words + text.split_whitespace().count();
                let bytes = self.bytes + text.len();
                if turn_id != &request.turn_id
                    || request
                        .dialogue
                        .as_ref()
                        .is_some_and(|dialogue| dialogue.kind != QaDialogueKind::Operational)
                    || *sequence != expected
                    || self.sources.len() == QA_MAX_FINDINGS
                    || self.sources.contains(source_id)
                    || words > QA_MAX_VISIBLE_WORDS
                    || bytes > MAX_ANSWER_BYTES
                    || !passage.is_some_and(|p| exact_span(&p.text, *start, *end, text))
                {
                    return Err(invalid("QA worker returned an invalid evidence span"));
                }
                self.sources.insert(source_id.clone());
                self.bytes = bytes;
                self.words = words;
            }
            QaWorkerEvent::Complete {
                turn_id,
                sequence,
                outcome,
                context,
                reply,
            } => {
                if turn_id != &request.turn_id
                    || *sequence != expected
                    || context.is_some_and(|usage| !usage.is_valid_for(request))
                    || reply.as_deref().is_some_and(|text| {
                        let Some(dialogue) = &request.dialogue else {
                            return true;
                        };
                        *outcome != QaWorkerOutcome::Answered
                            || if dialogue.kind == QaDialogueKind::Operational {
                                !verified_operational_sources(request, text).is_some_and(
                                    |passages| {
                                        passages.len() == self.sources.len()
                                            && passages.iter().all(|passage| {
                                                self.sources.contains(&passage.source_id)
                                            })
                                    },
                                )
                            } else {
                                !valid_dialogue_reply(text)
                            }
                    })
                    || (*outcome == QaWorkerOutcome::Answered
                        && self.sources.is_empty()
                        && reply.is_none())
                    || (*outcome == QaWorkerOutcome::InsufficientContext
                        && !self.sources.is_empty())
                {
                    return Err(invalid("QA worker returned an invalid terminal result"));
                }
                self.terminal = true;
            }
        }
        Ok(self.terminal)
    }
}

pub fn exact_span(passage: &str, start: u32, end: u32, text: &str) -> bool {
    let Some(length) = end.checked_sub(start).filter(|length| *length > 0) else {
        return false;
    };
    !text.trim().is_empty()
        && text.chars().count() == length as usize
        && passage
            .chars()
            .skip(start as usize)
            .take(length as usize)
            .eq(text.chars())
}

/// Reject overlarge frames before allocating their payload.
pub fn qa_frame_len(header: [u8; 4]) -> io::Result<usize> {
    let len = u32::from_be_bytes(header) as usize;
    if !(1..=QA_MAX_FRAME_BYTES).contains(&len) {
        return Err(invalid("invalid QA worker frame size"));
    }
    Ok(len)
}

fn opaque_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_OPAQUE_ID_BYTES
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}

fn bounded_text(text: &str, max: usize) -> bool {
    text.len() <= max
        && !text.trim().is_empty()
        && !text
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\t' | '\n' | '\r'))
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_prompt_roles_in_final_and_streamed_conversation() {
        assert!(valid_dialogue_reply("Hi! How can I help?"));
        for reply in [
            "Earlier user: summarize the workstation",
            "Hi! EARLIER ASSISTANT: all checks passed.",
            "Current user: Hi!",
            "Approved observations: Ready.",
            "Reply: Hello there.",
        ] {
            assert!(!valid_dialogue_reply(reply), "{reply}");
        }

        let mut request =
            QaWorkerRequest::new("turn".into(), "snapshot".into(), "Hi!".into(), vec![]);
        request.dialogue = Some(QaWorkerDialogue {
            kind: QaDialogueKind::Greeting,
            history: vec![],
        });
        let mut sequence = QaWorkerSequence::default();
        let delta = |sequence, text: &str| QaWorkerEvent::Delta {
            turn_id: request.turn_id.clone(),
            sequence,
            text: text.into(),
        };
        assert!(sequence.accept(&request, &delta(1, "Hi! Earlier")).is_ok());
        assert!(sequence
            .accept(&request, &delta(2, " user: summarize the workstation"))
            .is_err());
    }

    #[test]
    fn unicode_spans_are_exact_and_half_open() {
        let passage = "État du café: prêt ☕";
        assert!(exact_span(passage, 14, 18, "prêt"));
        assert!(!exact_span(passage, 15, 19, "prêt"));
        assert!(!exact_span(passage, 14, 80, "prêt"));
        assert!(!exact_span("ab", 1, 3, "b"));
    }

    #[test]
    fn rejects_foreign_replayed_and_nonterminal_event_sequences() {
        let request = QaWorkerRequest::new(
            "turn-a".into(),
            "snapshot-a".into(),
            "What is the state of actor search?".into(),
            vec![QaPassage::new(
                "check-a".into(),
                "Actor search is ready.".into(),
            )],
        );
        let event = QaWorkerEvent::Finding {
            turn_id: "turn-a".into(),
            sequence: 1,
            source_id: "check-a".into(),
            start: 16,
            end: 21,
            text: "ready".into(),
        };
        let mut sequence = QaWorkerSequence::default();
        assert!(!sequence.accept(&request, &event).unwrap());
        assert!(sequence.accept(&request, &event).is_err());
        let foreign = QaWorkerEvent::Complete {
            turn_id: "turn-b".into(),
            sequence: 2,
            outcome: QaWorkerOutcome::Answered,
            context: None,
            reply: None,
        };
        assert!(sequence.accept(&request, &foreign).is_err());
        let oversized_context = QaWorkerEvent::Complete {
            turn_id: request.turn_id.clone(),
            sequence: 2,
            outcome: QaWorkerOutcome::Answered,
            context: Some(QaContextUsage::new(256, 257, 1)),
            reply: None,
        };
        assert!(sequence.accept(&request, &oversized_context).is_err());
        let terminal = QaWorkerEvent::Complete {
            turn_id: request.turn_id.clone(),
            sequence: 2,
            outcome: QaWorkerOutcome::Answered,
            context: Some(QaContextUsage::new(256, 18, 1)),
            reply: None,
        };
        assert!(sequence.accept(&request, &terminal).unwrap());
        assert!(sequence.accept(&request, &terminal).is_err());
    }

    #[test]
    fn only_dialogue_requests_accept_a_bounded_generated_reply() {
        let mut request =
            QaWorkerRequest::new("turn".into(), "snapshot".into(), "Hi".into(), vec![]);
        let event = QaWorkerEvent::Complete {
            turn_id: request.turn_id.clone(),
            sequence: 1,
            outcome: QaWorkerOutcome::Answered,
            context: Some(QaContextUsage::new(QA_CONTEXT_WINDOW_TOKENS, 80, 1)),
            reply: Some("Hello! How can I help?".into()),
        };
        assert!(QaWorkerSequence::default()
            .accept(&request, &event)
            .is_err());
        let earlier = QaDialogueExchange {
            question: "What can you check?".into(),
            reply: "Workstation status.".into(),
        };
        request.dialogue = Some(QaWorkerDialogue {
            kind: QaDialogueKind::Greeting,
            history: vec![earlier.clone(); QA_MAX_DIALOGUE_HISTORY],
        });
        assert!(request.validate().is_ok());
        assert!(QaWorkerSequence::default()
            .accept(&request, &event)
            .unwrap());
        request.dialogue.as_mut().unwrap().history.push(earlier);
        assert!(request.validate().is_err());
    }

    #[test]
    fn operational_replies_require_complete_observed_facts() {
        let request = QaWorkerRequest::new(
            "turn".into(),
            "snapshot".into(),
            "Summarize status".into(),
            vec![QaPassage::new("cpu".into(), "CPU usage is 25.5%.".into())],
        );
        let sources = verified_operational_sources(
            &request,
            "Here is the current status. CPU usage is 25.5%.",
        )
        .unwrap();
        assert_eq!(sources[0].source_id, "cpu");
        for reply in [
            "CPU usage is 26.5%.",
            "CPU usage is 25.5% and all jobs completed.",
            "CPU usage is 25.5%. All jobs completed.",
            "CPU usage is 25.5%. Indexing failed because the disk was full.",
            "CPU usage is 25.5%. Open https://example.invalid for the job.",
            "Here is the current status.",
        ] {
            assert!(verified_operational_sources(&request, reply).is_none());
        }
    }

    #[test]
    fn bounds_frames_and_total_evidence_before_use() {
        assert!(qa_frame_len(u32::MAX.to_be_bytes()).is_err());
        assert!(qa_frame_len(0u32.to_be_bytes()).is_err());
        let request = QaWorkerRequest::new(
            "turn-a".into(),
            "snapshot-a".into(),
            "What changed?".into(),
            vec![
                QaPassage::new("check-a".into(), "a".repeat(QA_MAX_EVIDENCE_BYTES)),
                QaPassage::new("check-b".into(), "b".into()),
            ],
        );
        assert!(request.validate().is_err());
    }
}
