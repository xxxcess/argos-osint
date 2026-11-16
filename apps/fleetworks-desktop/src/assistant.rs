//! The native contextual Assistant dock.
//!
//! This slice owns the pane, its pinned context and the composer. It sends nothing: the typed chat
//! contract does not exist yet, so the backend advertises no chat capability and sending stays
//! gated with a plain reason. Drafts stay inside one authorized context. Changing context is an
//! explicit action, and a new connection authority clears the draft and the pin.

use rune::El;
use rune::Key;
use rune::KeyEvent;
use rune::TextEdit;
use rune_kit::Kit;

use crate::app::Page;
use crate::icons;
use crate::theme;
use crate::State;

/// Dock width when it shares the window with the center view.
pub(crate) const DOCK_WIDTH: f32 = 380.0;
/// The center view never gets narrower than this because of the dock.
pub(crate) const MIN_CENTER: f32 = 640.0;
/// Input bound from the chat contract: 4 KiB.
pub(crate) const MAX_INPUT_BYTES: usize = 4 * 1024;

pub(crate) const UNAVAILABLE: &str =
    "The Assistant is unavailable: this backend does not advertise chat. Nothing was sent.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Context {
    pub kind: &'static str,
    pub label: String,
    /// Identity used to tell whether the context changed.
    pub key: String,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Send {
    Empty,
    TooLong,
    Unavailable,
    Ready(String),
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum EnterAction {
    Send,
    Newline,
    Ignore,
}

/// Enter sends. Shift+Enter inserts a line. An input method that is composing owns Enter.
pub(crate) fn enter_action(shift: bool, composing: bool) -> EnterAction {
    if composing {
        EnterAction::Ignore
    } else if shift {
        EnterAction::Newline
    } else {
        EnterAction::Send
    }
}

/// What the operator asked for. `Auto` docks the Assistant only when the window has room.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Preference {
    #[default]
    Auto,
    Open,
    Closed,
}

impl Preference {
    /// Where the Assistant shows for a window with `room` points of main area.
    pub(crate) fn placement(self, room: f32) -> Placement {
        let fits = room >= DOCK_WIDTH + MIN_CENTER;
        match (self, fits) {
            (Self::Closed, _) => Placement::Hidden,
            (_, true) => Placement::Docked,
            (Self::Open, false) => Placement::Overlay,
            (Self::Auto, false) => Placement::Hidden,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Placement {
    Docked,
    Overlay,
    Hidden,
}

pub struct Assistant {
    /// Off by default: the chat aux panel stays disabled until the chat contract is in.
    pub(crate) enabled: bool,
    pub(crate) preference: Preference,
    pub(crate) draft: TextEdit,
    pinned: Option<Context>,
    scope: Option<u64>,
    pub(crate) notice: Option<String>,
}

impl Default for Assistant {
    fn default() -> Self {
        Self {
            enabled: false,
            preference: Preference::Auto,
            draft: TextEdit::new(""),
            pinned: None,
            scope: None,
            notice: None,
        }
    }
}

impl Assistant {
    pub(crate) fn pinned(&self) -> Option<&Context> {
        self.pinned.as_ref()
    }

    /// Follow the connection authority. A different one drops the pin, the draft and any notice.
    pub(crate) fn sync_scope(&mut self, scope: u64) -> bool {
        if self.scope == Some(scope) {
            return false;
        }
        let changed = self.scope.is_some();
        self.scope = Some(scope);
        self.pinned = None;
        self.notice = None;
        if changed {
            self.draft = TextEdit::new("");
        }
        changed
    }

    /// Pin the first context seen. Later changes need an explicit switch.
    pub(crate) fn pin_if_empty(&mut self, current: &Context) {
        if self.pinned.is_none() {
            self.pinned = Some(current.clone());
        }
    }

    /// Switch to `current`. The draft belonged to the old context, so it is cleared.
    pub(crate) fn switch_to(&mut self, current: Context) {
        if self.pinned.as_ref() != Some(&current) {
            self.draft = TextEdit::new("");
        }
        self.pinned = Some(current);
        self.notice = None;
    }

    /// Decide what Enter does with the draft. The draft is kept unless it is ready to send.
    pub(crate) fn try_send(&mut self, available: bool) -> Send {
        let text = self.draft.text.trim().to_owned();
        if text.is_empty() {
            return Send::Empty;
        }
        if text.len() > MAX_INPUT_BYTES {
            self.notice = Some(format!(
                "That message is over the {} KiB limit. Shorten it to send.",
                MAX_INPUT_BYTES / 1024
            ));
            return Send::TooLong;
        }
        if !available {
            self.notice = Some(UNAVAILABLE.to_owned());
            return Send::Unavailable;
        }
        self.draft = TextEdit::new("");
        self.notice = None;
        Send::Ready(text)
    }
}

/// Whether the connected backend advertises a typed chat capability. None does yet.
pub(crate) fn chat_available(_state: &State) -> bool {
    false
}

/// What the Assistant is about right now, from the selection the operator already made.
pub(crate) fn current_context(state: &State) -> Context {
    let short = |id: &str| id.chars().take(12).collect::<String>();
    match state.page {
        Page::Jobs => {
            if let Some(id) = &state.selected_job_id {
                return Context {
                    kind: "Job",
                    label: format!("Job {}", short(id)),
                    key: format!("job:{id}"),
                };
            }
        }
        Page::Atlas => {
            if let Some(id) = &state.selected_cycle_id {
                return Context {
                    kind: "Cycle",
                    label: format!("Cycle {}", short(id)),
                    key: format!("cycle:{id}"),
                };
            }
        }
        Page::Workstation => {
            if let Some(id) = &state.selected_readiness {
                return Context {
                    kind: "Readiness",
                    label: format!("Readiness: {id}"),
                    key: format!("readiness:{id}"),
                };
            }
        }
        _ => {}
    }
    Context {
        kind: "Page",
        label: state.page.title().to_owned(),
        key: format!("page:{}", state.page.key()),
    }
}

impl State {
    /// Keep the Assistant bound to the current authority and pin its first context.
    pub(crate) fn sync_assistant(&mut self) -> bool {
        if !self.assistant.enabled {
            return false;
        }
        let changed = self.assistant.sync_scope(self.origin_revision);
        let current = current_context(self);
        self.assistant.pin_if_empty(&current);
        changed
    }

    pub(crate) fn assistant_send(&mut self) {
        let available = chat_available(self);
        // A ready message has nowhere to go until the chat contract lands.
        let _ = self.assistant.try_send(available);
    }
}

/// The toolbar toggle. `visible` is whether the Assistant shows right now.
pub(crate) fn toggle(visible: bool, kit: &Kit) -> El {
    rune_kit::IconButton::new(&icons::ASSISTANT)
        .variant(rune_kit::IconButtonVariant::Ghost)
        .label(if visible {
            "Hide Assistant"
        } else {
            "Show Assistant"
        })
        .on_press(move |state: &mut State| {
            state.assistant.preference = if visible {
                Preference::Closed
            } else {
                Preference::Open
            };
        })
        .build(kit)
        .key("assistant-toggle")
}

/// The dock itself, ~380 pt wide.
pub(crate) fn dock(
    state: &State,
    kit: &Kit,
    preferences: crate::accessibility::ViewPreferences,
) -> El {
    let assistant = &state.assistant;
    let current = current_context(state);
    let mut context = rune::col()
        .key("assistant-context")
        .w_full()
        .gap(4.0)
        .child(
            rune::text("CONTEXT")
                .font(kit.ty.text_xsmall_medium)
                .color(kit.c.text_faint),
        )
        .child(
            rune::text(
                assistant
                    .pinned()
                    .map_or_else(|| current.label.clone(), |pinned| pinned.label.clone()),
            )
            .key("assistant-pinned")
            .font(kit.ty.body_medium)
            .color(theme::hex(theme::TEXT_PRIMARY))
            .wrap(),
        );
    if assistant
        .pinned()
        .is_some_and(|pinned| pinned.key != current.key)
    {
        let target = current.clone();
        context = context
            .child(
                rune::text(format!("You are now viewing {}.", current.label))
                    .font(kit.ty.small)
                    .color(theme::hex(theme::TEXT_MUTED))
                    .wrap(),
            )
            .child(
                rune_kit::Button::new("Switch context")
                    .variant(rune_kit::ButtonVariant::Outline)
                    .on_press(move |state: &mut State| state.assistant.switch_to(target.clone()))
                    .build(kit)
                    .key("assistant-switch-context"),
            );
    }

    let status = rune::text(
        assistant
            .notice
            .clone()
            .unwrap_or_else(|| UNAVAILABLE.to_owned()),
    )
    .key("assistant-status")
    .font(kit.ty.small)
    .color(theme::hex(theme::TEXT_MUTED))
    .wrap();

    let composer = rune_kit::TextArea::new(&assistant.draft)
        .label("Message the Assistant")
        .placeholder("Ask about this context")
        .auto_grow(2, 6)
        .on_edit(|state: &mut State| &mut state.assistant.draft)
        .on_key(|state: &mut State, event: &KeyEvent, _| {
            if event.key != Key::Enter {
                return false;
            }
            match enter_action(event.mods.shift, state.assistant.draft.is_composing()) {
                EnterAction::Send => {
                    state.assistant_send();
                    true
                }
                EnterAction::Newline | EnterAction::Ignore => false,
            }
        })
        .on_submit(|state: &mut State, _| state.assistant_send())
        .build(kit)
        .key("assistant-composer");
    // The composer sits on tinted glass; the history above stays opaque.
    let composer = crate::material::glass(
        rune::col()
            .key("assistant-composer-glass")
            .w_full()
            .padding(6.0)
            .radius(8.0)
            .child(composer),
        crate::material::GlassSurface::Composer,
        preferences,
    );

    let send = rune_kit::Button::new("Send")
        .variant(rune_kit::ButtonVariant::Submit)
        .disabled(!chat_available(state) || assistant.draft.text.trim().is_empty())
        .on_press(|state: &mut State| state.assistant_send())
        .build(kit)
        .key("assistant-send");

    rune::col()
        .key("assistant-dock")
        .w(DOCK_WIDTH)
        .h_full()
        .px(16.0)
        .py(16.0)
        .gap(14.0)
        .fill(theme::hex(theme::SURFACE))
        .border_sides([1.0, 0.0, 0.0, 0.0], theme::hex(theme::BORDER_SUBTLE))
        .child(
            rune::row()
                .w_full()
                .center_y()
                .child(
                    rune::text("Assistant")
                        .font(kit.ty.body_medium)
                        .color(theme::hex(theme::TEXT_PRIMARY)),
                )
                .child(rune::spacer())
                .child(
                    rune_kit::IconButton::new(&icons::ASSISTANT)
                        .variant(rune_kit::IconButtonVariant::Ghost)
                        .label("Collapse Assistant")
                        .on_press(|state: &mut State| {
                            state.assistant.preference = Preference::Closed
                        })
                        .build(kit)
                        .key("assistant-collapse"),
                ),
        )
        .child(context)
        .child(
            rune::col()
                .key("assistant-history")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .child(status),
        )
        .child(composer)
        .child(
            rune::row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(
                    rune::text("Enter sends. Shift+Enter adds a line.")
                        .font(kit.ty.small)
                        .color(theme::hex(theme::TEXT_MUTED)),
                )
                .child(rune::spacer())
                .child(send),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context(key: &str) -> Context {
        Context {
            kind: "Page",
            label: key.to_owned(),
            key: key.to_owned(),
        }
    }

    #[test]
    fn enter_sends_shift_adds_a_line_and_ime_keeps_enter() {
        assert_eq!(enter_action(false, false), EnterAction::Send);
        assert_eq!(enter_action(true, false), EnterAction::Newline);
        assert_eq!(enter_action(false, true), EnterAction::Ignore);
        assert_eq!(enter_action(true, true), EnterAction::Ignore);
    }

    #[test]
    fn sending_is_gated_and_keeps_the_draft() {
        let mut assistant = Assistant::default();
        assert_eq!(assistant.try_send(false), Send::Empty);
        assistant.draft = TextEdit::new("  hello  ");
        assert_eq!(assistant.try_send(false), Send::Unavailable);
        assert_eq!(assistant.draft.text, "  hello  ");
        assert_eq!(assistant.notice.as_deref(), Some(UNAVAILABLE));
        assert_eq!(assistant.try_send(true), Send::Ready("hello".into()));
        assert!(assistant.draft.text.is_empty());
    }

    #[test]
    fn input_is_bounded_to_four_kib() {
        let mut assistant = Assistant {
            draft: TextEdit::new("x".repeat(MAX_INPUT_BYTES)),
            ..Assistant::default()
        };
        assert_eq!(
            assistant.try_send(true),
            Send::Ready("x".repeat(MAX_INPUT_BYTES))
        );
        assistant.draft = TextEdit::new("x".repeat(MAX_INPUT_BYTES + 1));
        assert_eq!(assistant.try_send(true), Send::TooLong);
        assert_eq!(assistant.draft.text.len(), MAX_INPUT_BYTES + 1);
    }

    #[test]
    fn context_pins_once_and_switching_is_explicit_and_clears_the_draft() {
        let mut assistant = Assistant::default();
        assistant.pin_if_empty(&context("page:jobs"));
        assistant.pin_if_empty(&context("page:atlas"));
        assert_eq!(
            assistant.pinned().map(|c| c.key.as_str()),
            Some("page:jobs")
        );
        assistant.draft = TextEdit::new("draft for jobs");
        // Re-pinning the same context keeps the draft.
        assistant.switch_to(context("page:jobs"));
        assert_eq!(assistant.draft.text, "draft for jobs");
        assistant.switch_to(context("page:atlas"));
        assert!(assistant.draft.text.is_empty());
        assert_eq!(
            assistant.pinned().map(|c| c.key.as_str()),
            Some("page:atlas")
        );
    }

    #[test]
    fn a_new_authority_drops_pin_draft_and_notice() {
        let mut assistant = Assistant::default();
        assert!(!assistant.sync_scope(1));
        assistant.pin_if_empty(&context("page:jobs"));
        assistant.draft = TextEdit::new("keep me");
        assert!(!assistant.sync_scope(1));
        assert_eq!(assistant.draft.text, "keep me");
        assert!(assistant.sync_scope(2));
        assert!(assistant.pinned().is_none());
        assert!(assistant.draft.text.is_empty());
    }
}

#[cfg(test)]
mod placement_tests {
    use super::*;

    #[test]
    fn auto_docks_only_when_there_is_room_and_open_floats_when_there_is_not() {
        let fits = DOCK_WIDTH + MIN_CENTER;
        assert_eq!(Preference::Auto.placement(fits), Placement::Docked);
        assert_eq!(Preference::Auto.placement(fits - 1.0), Placement::Hidden);
        assert_eq!(Preference::Open.placement(fits), Placement::Docked);
        assert_eq!(Preference::Open.placement(fits - 1.0), Placement::Overlay);
        assert_eq!(Preference::Closed.placement(5000.0), Placement::Hidden);
        assert_eq!(Preference::Auto.placement(0.0), Placement::Hidden);
    }
}

#[cfg(test)]
mod render_tests {
    use super::*;
    use rune::Env;
    use rune::Runtime;

    fn runtime(width: f32, enabled: bool, preference: Preference) -> Runtime<State> {
        let mut state = crate::tests::registered_fixture();
        state.page = Page::Workstation;
        state.assistant.enabled = enabled;
        state.assistant.preference = preference;
        let mut runtime = Runtime::new(
            state,
            crate::app::view,
            (width, 800.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        runtime.frame(0.0);
        runtime.frame(1.0 / 60.0);
        runtime
    }

    #[test]
    fn the_chat_panel_is_off_by_default_and_the_context_rail_stays() {
        assert!(!Assistant::default().enabled);
        for width in [1440.0, 1180.0] {
            let rt = runtime(width, false, Preference::Open);
            assert!(rt.find("assistant-dock").is_none());
            assert!(rt.find("assistant-overlay").is_none());
            assert!(rt.find("assistant-toggle").is_none());
            assert_eq!(rt.find("context-rail").is_some(), width >= 1440.0);
            // The Inbox is independent of the Assistant.
            assert!(rt.find("inbox-toggle").is_some());
        }
    }

    #[test]
    fn when_enabled_it_docks_with_room_and_floats_only_when_asked() {
        let wide = runtime(1600.0, true, Preference::Auto);
        assert!(wide.find("assistant-dock").is_some());
        assert!(wide.find("context-rail").is_none());
        let narrow = runtime(1180.0, true, Preference::Auto);
        assert!(narrow.find("assistant-dock").is_none());
        assert!(narrow.find("assistant-toggle").is_some());
        let asked = runtime(1180.0, true, Preference::Open);
        assert!(asked.find("assistant-overlay").is_some());
        assert!(asked.find("assistant-dock").is_some());
        let closed = runtime(1600.0, true, Preference::Closed);
        assert!(closed.find("assistant-dock").is_none());
    }

    #[test]
    fn the_composer_and_send_are_present_and_send_stays_gated() {
        let mut rt = runtime(1600.0, true, Preference::Auto);
        assert!(rt.find("assistant-composer").is_some());
        assert!(rt.find("assistant-send").is_some());
        rt.update(|state, _| {
            state.assistant.draft = TextEdit::new("hello");
            state.assistant_send();
        });
        rt.frame(2.0 / 60.0);
        assert_eq!(rt.state.assistant.draft.text, "hello");
        assert_eq!(rt.state.assistant.notice.as_deref(), Some(UNAVAILABLE));
    }
}
