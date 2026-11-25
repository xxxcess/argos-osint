//! Presentation for the Assistant conversation.
//!
//! These are view parts only: message bubbles, turn status chips, source citations and a
//! bounded history that follows the newest message. They hold no backend types and send nothing.
//! The chat contract, when it lands, fills `Assistant::messages`; nothing here reaches a network,
//! a transport or a provider.

// Variants and helpers the future chat wiring constructs are exercised by tests until then.
#![cfg_attr(not(test), allow(dead_code))]

use rune::El;
use rune::ScrollSpec;
use rune_kit::Kit;

use crate::assistant::DOCK_WIDTH;
use crate::theme;
use crate::theme::StatusTone;

/// Most messages drawn at once. Matches the contract's 50-message page.
pub(crate) const MAX_VISIBLE: usize = 50;
/// A bubble never grows past this share of the dock.
const BUBBLE_MAX: f32 = DOCK_WIDTH * 0.86;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Role {
    Operator,
    Assistant,
}

impl Role {
    fn label(self) -> &'static str {
        match self {
            Self::Operator => "You",
            Self::Assistant => "Assistant",
        }
    }
}

/// The real state of a turn, including provisional V2 generation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TurnState {
    Queued,
    Running,
    Streaming,
    Retrying { attempt: u32 },
    Final,
    Failed { reason: String },
}

impl TurnState {
    /// `None` for a final answer, which needs no chip.
    fn chip(&self) -> Option<(String, StatusTone)> {
        match self {
            Self::Queued => Some(("Queued".to_owned(), StatusTone::Neutral)),
            Self::Running => Some(("Working".to_owned(), StatusTone::Info)),
            Self::Streaming => Some(("Draft · unverified".to_owned(), StatusTone::Info)),
            Self::Retrying { attempt } => {
                Some((format!("Retrying, attempt {attempt}"), StatusTone::Warn))
            }
            Self::Final => None,
            Self::Failed { reason } => Some((format!("Failed: {reason}"), StatusTone::Err)),
        }
    }

    pub(crate) fn is_pending(&self) -> bool {
        matches!(
            self,
            Self::Queued | Self::Running | Self::Streaming | Self::Retrying { .. }
        )
    }
}

/// A source the answer rests on. Only a short label and kind ever reach the view, never raw
/// evidence text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Citation {
    pub kind: &'static str,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ChatMessage {
    pub id: String,
    pub role: Role,
    pub state: TurnState,
    pub text: String,
    pub citations: Vec<Citation>,
}

/// The newest `MAX_VISIBLE` messages and how many older ones are left out.
pub(crate) fn visible(messages: &[ChatMessage]) -> (&[ChatMessage], usize) {
    let hidden = messages.len().saturating_sub(MAX_VISIBLE);
    (&messages[hidden..], hidden)
}

/// Key of the newest message, so the owner can keep it in view as the list changes.
pub(crate) fn newest_key(messages: &[ChatMessage]) -> Option<String> {
    messages.last().map(|m| message_key(&m.id))
}

pub(crate) fn message_key(id: &str) -> String {
    format!("chat-message-{id}")
}

fn chip(label: String, tone: StatusTone, kit: &Kit) -> El {
    rune::row()
        .px(6.0)
        .py(2.0)
        .radius(999.0)
        .fill(tone.bg())
        .border(1.0, tone.border())
        .child(rune::text(label).font(kit.ty.small).color(tone.fg()).wrap())
}

pub(crate) fn citation_rows(list: &[Citation], kit: &Kit) -> El {
    let mut column = rune::col().w_full().gap(3.0);
    for (index, citation) in list.iter().enumerate() {
        column = column.child(
            rune::text(format!("{}: {}", citation.kind, citation.label))
                .key(format!("chat-citation-{index}"))
                .font(kit.ty.small)
                .color(theme::hex(theme::TEXT_SECONDARY))
                .wrap_anywhere(),
        );
    }
    column
}

/// One message. The operator's sit on the right in an accent tint; the Assistant's sit on the
/// left on a raised surface. A pending or failed turn shows its real state beside the text.
pub(crate) fn bubble(message: &ChatMessage, kit: &Kit) -> El {
    let sources = (!message.citations.is_empty()).then(|| {
        rune::col()
            .w_full()
            .gap(3.0)
            .child(
                rune::text("Sources")
                    .font(kit.ty.small_medium)
                    .color(theme::hex(theme::TEXT_MUTED)),
            )
            .child(citation_rows(&message.citations, kit))
    });
    bubble_with_sources(message, sources, kit)
}

pub(crate) fn bubble_with_sources(message: &ChatMessage, sources: Option<El>, kit: &Kit) -> El {
    let state = message
        .state
        .chip()
        .map(|(label, tone)| chip(label, tone, kit).key(format!("chat-state-{}", message.id)));
    message_bubble(
        message_key(&message.id),
        message.role,
        &message.text,
        state,
        sources,
        kit,
    )
}

pub(crate) fn greeting(key: &str, text: &str, kit: &Kit) -> El {
    message_bubble(key.to_owned(), Role::Assistant, text, None, None, kit)
}

fn message_bubble(
    key: String,
    role: Role,
    text: &str,
    state: Option<El>,
    sources: Option<El>,
    kit: &Kit,
) -> El {
    let operator = role == Role::Operator;
    let tone = if operator {
        StatusTone::Accent
    } else {
        StatusTone::Neutral
    };
    let mut body = rune::col().w_full().gap(6.0).child(
        rune::text(role.label())
            .font(kit.ty.small_medium)
            .color(theme::hex(theme::TEXT_MUTED)),
    );
    if !text.is_empty() {
        body = body.child(
            rune::text(text)
                .font(kit.ty.small)
                .color(theme::hex(theme::TEXT_PRIMARY))
                .wrap_anywhere(),
        );
    }
    let surface = rune::col()
        .key(key)
        .label(format!("{} message", role.label()))
        .max_w(BUBBLE_MAX)
        .padding(10.0)
        .radius(10.0)
        .fill(tone.bg())
        .border(1.0, tone.border())
        .child(body.children(state).children(sources));
    let row = rune::row().w_full();
    if operator {
        row.child(rune::spacer()).child(surface)
    } else {
        row.child(surface).child(rune::spacer())
    }
}

/// The history pane. With no messages it shows `empty` (the plain status line). Otherwise it
/// scrolls and sticks to the newest message, with a note when older ones are left out.
pub(crate) fn history(messages: &[ChatMessage], empty: El, kit: &Kit) -> El {
    let pane = rune::col()
        .key("assistant-history")
        .w_full()
        .grow(1.0)
        .min_h(0.0);
    if messages.is_empty() {
        return pane.child(empty);
    }
    let (shown, hidden) = visible(messages);
    let mut list = rune::col().w_full().gap(10.0);
    if hidden > 0 {
        list = list.child(
            rune::text(format!(
                "{hidden} earlier {} not shown.",
                if hidden == 1 { "message" } else { "messages" }
            ))
            .key("chat-earlier")
            .font(kit.ty.small)
            .color(theme::hex(theme::TEXT_MUTED))
            .wrap(),
        );
    }
    for message in shown {
        list = list.child(bubble(message, kit));
    }
    pane.child(
        rune::col()
            .key("assistant-history-scroll")
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .scroll(ScrollSpec { stick_bottom: true })
            .child(list),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(id: usize, role: Role, state: TurnState) -> ChatMessage {
        ChatMessage {
            id: id.to_string(),
            role,
            state,
            text: format!("message {id}"),
            citations: Vec::new(),
        }
    }

    #[test]
    fn history_is_bounded_to_the_newest_messages() {
        let all: Vec<_> = (0..MAX_VISIBLE + 7)
            .map(|i| message(i, Role::Assistant, TurnState::Final))
            .collect();
        let (shown, hidden) = visible(&all);
        assert_eq!(shown.len(), MAX_VISIBLE);
        assert_eq!(hidden, 7);
        assert_eq!(shown[0].id, "7");
        assert_eq!(newest_key(&all).as_deref(), Some("chat-message-56"));
        assert_eq!(newest_key(&[]), None);
        let few = &all[..3];
        assert_eq!(visible(few).1, 0);
    }

    #[test]
    fn each_real_turn_state_has_its_own_label_and_a_final_answer_has_none() {
        assert_eq!(TurnState::Queued.chip().expect("chip").0, "Queued");
        assert_eq!(TurnState::Running.chip().expect("chip").0, "Working");
        assert_eq!(
            TurnState::Retrying { attempt: 2 }.chip().expect("chip").0,
            "Retrying, attempt 2"
        );
        assert_eq!(
            TurnState::Failed {
                reason: "backend restarted".into()
            }
            .chip()
            .expect("chip")
            .0,
            "Failed: backend restarted"
        );
        assert!(TurnState::Final.chip().is_none());
        assert!(TurnState::Queued.is_pending() && TurnState::Running.is_pending());
        assert!(!TurnState::Final.is_pending());
        assert!(!TurnState::Failed {
            reason: String::new()
        }
        .is_pending());
    }
}
