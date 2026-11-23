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
pub const QA_MODEL_REVISION: &str = "564e9b582944a57a3e586bbb98fd6f0a4118db7f";
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

/// Room for bounded evidence, JSON escaping, and the envelope.
pub const QA_MAX_FRAME_BYTES: usize = 512 * 1024;
pub const QA_MAX_QUESTION_BYTES: usize = 4 * 1024;
pub const QA_MAX_EVIDENCE_BYTES: usize = 64 * 1024;
pub const QA_MAX_PASSAGES: usize = 16;
pub const QA_MAX_FINDINGS: usize = 3;
pub const QA_MAX_VISIBLE_WORDS: usize = 60;
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
        self.window_tokens == QA_CONTEXT_WINDOW_TOKENS
            && self.peak_input_tokens <= self.window_tokens
            && (self.model_windows == 0) == (self.peak_input_tokens == 0)
            && usize::from(self.model_windows) <= request.passages.len() * MAX_WINDOWS_PER_PASSAGE
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
        }
    }

    /// Validate identifiers and size limits before work can enter the model.
    pub fn validate(&self) -> io::Result<()> {
        if !opaque_id(&self.turn_id)
            || !opaque_id(&self.snapshot_id)
            || !bounded_text(&self.question, QA_MAX_QUESTION_BYTES)
            || self.passages.len() > QA_MAX_PASSAGES
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
    },
}

/// Commands only accepted over an executor-owned pipe or socket.
#[derive(Deserialize, Serialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum QaWorkerCommand {
    Open,
    OpenWithContext,
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
    sources: BTreeSet<String>,
    bytes: usize,
    words: usize,
    terminal: bool,
}

impl QaWorkerSequence {
    /// Require exactly one ordered stream for the given turn and validate every extracted span.
    pub fn accept(&mut self, request: &QaWorkerRequest, event: &QaWorkerEvent) -> io::Result<bool> {
        if self.terminal {
            return Err(invalid("QA worker sent an event after its terminal result"));
        }
        let expected = self.sources.len() as u32 + 1;
        match event {
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
            } => {
                if turn_id != &request.turn_id
                    || *sequence != expected
                    || context.is_some_and(|usage| !usage.is_valid_for(request))
                    || (*outcome == QaWorkerOutcome::Answered && self.sources.is_empty())
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
        };
        assert!(sequence.accept(&request, &foreign).is_err());
        let oversized_context = QaWorkerEvent::Complete {
            turn_id: request.turn_id.clone(),
            sequence: 2,
            outcome: QaWorkerOutcome::Answered,
            context: Some(QaContextUsage::new(256, 257, 1)),
        };
        assert!(sequence.accept(&request, &oversized_context).is_err());
        let terminal = QaWorkerEvent::Complete {
            turn_id: request.turn_id.clone(),
            sequence: 2,
            outcome: QaWorkerOutcome::Answered,
            context: Some(QaContextUsage::new(256, 18, 1)),
        };
        assert!(sequence.accept(&request, &terminal).unwrap());
        assert!(sequence.accept(&request, &terminal).is_err());
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
