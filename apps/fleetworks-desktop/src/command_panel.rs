//! The native command panel: a typed list of known navigation and utility actions.
//!
//! Cmd/Ctrl+K or the toolbar button opens it. It filters a fixed list, never runs arbitrary text,
//! and never starts work: an action that could accept work (seed, index, train) only navigates to
//! the page whose existing review step owns it. Only this panel owns the overlay while it is open.

use rune::El;
use rune::Key;
use rune::KeyEvent;
use rune::TextEdit;
use rune_kit::Kit;

use crate::app::Page;
use crate::icons;
use crate::theme;
use crate::State;

/// Rows shown at once. The list is fixed and small, so this is a safety bound.
const MAX_ROWS: usize = 24;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Command {
    Go(Page),
    SetupGateway,
    OpenSettings,
    OpenInbox,
    ToggleAssistant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Entry {
    pub command: Command,
    pub group: &'static str,
    pub label: String,
}

/// Every known action for the current connection. Nothing here takes free-form input.
pub(crate) fn entries(connected: bool, assistant_enabled: bool) -> Vec<Entry> {
    let mut out = vec![Entry {
        command: Command::Go(Page::Workstation),
        group: "Go to",
        label: Page::Workstation.title().to_owned(),
    }];
    if connected {
        for page in Page::ALL {
            out.push(Entry {
                command: Command::Go(page),
                group: "Go to",
                label: page.title().to_owned(),
            });
        }
    }
    out.push(Entry {
        command: Command::SetupGateway,
        group: "Connection",
        label: "Setup Gateway".to_owned(),
    });
    out.push(Entry {
        command: Command::OpenSettings,
        group: "App",
        label: "Settings".to_owned(),
    });
    out.push(Entry {
        command: Command::OpenInbox,
        group: "App",
        label: "Open Inbox".to_owned(),
    });
    if assistant_enabled && connected {
        out.push(Entry {
            command: Command::ToggleAssistant,
            group: "App",
            label: "Show or hide Assistant".to_owned(),
        });
    }
    out
}

/// Entries whose label or group contains every typed word, in list order.
pub(crate) fn filter(all: Vec<Entry>, query: &str) -> Vec<Entry> {
    let words: Vec<String> = query
        .split_whitespace()
        .take(8)
        .map(str::to_lowercase)
        .collect();
    all.into_iter()
        .filter(|entry| {
            let haystack = format!("{} {}", entry.label, entry.group).to_lowercase();
            words.iter().all(|word| haystack.contains(word))
        })
        .take(MAX_ROWS)
        .collect()
}

#[derive(Default)]
pub struct CommandPanel {
    pub(crate) open: bool,
    pub(crate) query: TextEdit,
    pub(crate) active: usize,
    /// Set when the panel opens; the panel's first layout moves keyboard focus into the filter
    /// and clears it, so typing works without a click and resizing never steals focus again.
    pub(crate) focus_pending: bool,
}

impl CommandPanel {
    pub(crate) fn close(&mut self) {
        self.open = false;
        self.focus_pending = false;
        self.query = TextEdit::new("");
        self.active = 0;
    }

    /// Move the highlight, wrapping at both ends. An empty list keeps it at 0.
    pub(crate) fn step(&mut self, delta: isize, len: usize) {
        if len == 0 {
            self.active = 0;
            return;
        }
        let len = len as isize;
        self.active = (self.active as isize + delta).rem_euclid(len) as usize;
    }
}

impl State {
    fn command_entries(&self) -> Vec<Entry> {
        filter(
            entries(self.connected(), self.assistant.enabled),
            &self.commands.query.text,
        )
    }

    /// Open from Cmd/Ctrl+K or the toolbar. This panel becomes the only overlay.
    pub(crate) fn open_command_panel(&mut self) {
        if self.startup.active() {
            return;
        }
        self.settings_open = false;
        self.inbox.open = false;
        self.inbox.selected = None;
        self.commands.query = TextEdit::new("");
        self.commands.active = 0;
        self.commands.open = true;
        self.commands.focus_pending = true;
    }

    pub(crate) fn run_command(&mut self, command: Command) {
        self.commands.close();
        match command {
            Command::Go(page) => self.navigate(page),
            Command::SetupGateway => {
                self.navigate(Page::Workstation);
                self.setup_gateway = true;
            }
            Command::OpenSettings => self.settings_open = true,
            Command::OpenInbox => self.inbox.open = true,
            Command::ToggleAssistant => {
                use crate::assistant::Preference;
                self.assistant.preference = if self.assistant.preference == Preference::Closed {
                    Preference::Open
                } else {
                    Preference::Closed
                };
            }
        }
    }

    fn run_active_command(&mut self) {
        let entries = self.command_entries();
        if let Some(entry) = entries.get(self.commands.active) {
            self.run_command(entry.command);
        }
    }
}

/// Keys inside the search field: arrows move, Enter runs the highlighted row.
fn on_field_key(state: &mut State, event: &KeyEvent) -> bool {
    let len = state.command_entries().len();
    match event.key {
        Key::Down => state.commands.step(1, len),
        Key::Up => state.commands.step(-1, len),
        Key::Enter => state.run_active_command(),
        _ => return false,
    }
    true
}

/// The toolbar button.
pub(crate) fn toolbar_button(kit: &Kit) -> El {
    rune_kit::IconButton::new(&icons::SEARCH)
        .variant(rune_kit::IconButtonVariant::Ghost)
        .label("Command panel (Ctrl+K or ⌘K)")
        .on_press(|state: &mut State| state.open_command_panel())
        .build(kit)
        .key("command-open")
}

/// The command overlay, pushed to `layers` while it is open. It keeps the modal contract of a
/// dialog (focus trap, inert background, Escape and outside press close it) but draws its own
/// panel so it can sit on glass.
pub(crate) fn view(
    state: &State,
    kit: &Kit,
    preferences: crate::accessibility::ViewPreferences,
    layers: &mut Vec<El>,
) {
    if !state.commands.open {
        return;
    }
    let entries = state.command_entries();
    let search = rune_kit::TextInput::new(&state.commands.query)
        .label("Search commands")
        .placeholder("Type to filter")
        .on_edit(|state: &mut State| &mut state.commands.query)
        .on_change(|state: &mut State, _| state.commands.active = 0)
        .on_key(|state: &mut State, event: &KeyEvent, _| on_field_key(state, event))
        .build(kit)
        .key("command-search")
        .w_full();
    let mut list = rune::col().key("command-list").w_full().gap(2.0);
    let mut group = "";
    for (index, entry) in entries.iter().enumerate() {
        if entry.group != group {
            group = entry.group;
            list = list.child(
                rune::text(group.to_uppercase())
                    .font(kit.ty.text_xsmall_medium)
                    .color(kit.c.text_faint)
                    .px(8.0)
                    .pt(8.0),
            );
        }
        let command = entry.command;
        let active = index == state.commands.active;
        list = list.child(
            rune::row()
                .key(format!("command-{index}"))
                .w_full()
                .px(8.0)
                .py(7.0)
                .radius(6.0)
                .fill(if active {
                    theme::hex(theme::SELECTION_SURFACE)
                } else {
                    theme::hex(theme::SURFACE)
                })
                .on_press(move |state: &mut State| state.run_command(command))
                .child(
                    rune::text(entry.label.clone())
                        .font(kit.ty.small_medium)
                        .color(theme::hex(theme::TEXT_PRIMARY)),
                ),
        );
    }
    if entries.is_empty() {
        list = list.child(
            rune::text("No matching command.")
                .key("command-empty")
                .font(kit.ty.small)
                .color(theme::hex(theme::TEXT_MUTED))
                .padding(8.0),
        );
    }
    let close = rune_kit::IconButton::new(kit.glyph.close)
        .variant(rune_kit::IconButtonVariant::Ghost)
        .label("Close")
        .on_press(|state: &mut State| state.commands.close())
        .build(kit)
        .key("close")
        .size(20.0, 20.0)
        .mt(-2.0)
        .mr(-2.0);
    let header = rune::row()
        .key("header")
        .w_full()
        .padding(16.0)
        .items_start()
        .child(
            rune::col().grow(1.0).child(
                rune::text("Commands")
                    .font(kit.ty.ui_medium.lh(16.0))
                    .color(kit.c.text_base),
            ),
        )
        .child(close);
    let body = rune::col()
        .key("body")
        .w_full()
        .px(16.0)
        .pb(16.0)
        .gap(10.0)
        .child(search)
        .child(rune_kit::popover::fit_scroll(list));
    let mut panel = rune::col()
        .key("command-panel-dialog")
        .role("dialog")
        .label("Commands")
        .w_full()
        .max_w(460.0)
        .radius(10.0)
        .child(header)
        .child(body)
        .token_motion(kit.motion.overlay)
        // Keyboard focus goes to the filter as soon as the panel is laid out, once per opening.
        .on_layout_tx(|state: &mut State, _rect: [f32; 4], tx| {
            if state.commands.focus_pending {
                state.commands.focus_pending = false;
                tx.focus("command-search/input", true);
            } else {
                tx.live_only();
            }
        });
    for shadow in kit.shadow_layers(&kit.elevation.overlay) {
        panel = panel.box_shadow(shadow);
    }
    let panel = crate::material::glass(panel, crate::material::GlassSurface::Overlay, preferences)
        .modal(true, |state: &mut State| state.commands.close())
        .modal_click_outside(true);
    layers.push(
        rune::el()
            .key("command-panel-scrim")
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .fill(kit.c.scrim)
            .token_motion(kit.motion.overlay)
            .a11y_hidden(),
    );
    layers.push(
        rune::el()
            .key("command-panel-layer")
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .padding(16.0)
            .center()
            .child(panel),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(entries: &[Entry]) -> Vec<&str> {
        entries.iter().map(|entry| entry.label.as_str()).collect()
    }

    #[test]
    fn disconnected_lists_only_what_can_open() {
        let all = entries(false, false);
        assert!(all
            .iter()
            .all(|entry| !matches!(entry.command, Command::Go(page) if page != Page::Workstation)));
        assert!(labels(&all).contains(&"Setup Gateway"));
        assert!(!labels(&all).contains(&"Show or hide Assistant"));
    }

    #[test]
    fn connected_lists_every_destination_and_assistant_only_when_enabled() {
        let all = entries(true, false);
        for page in Page::ALL {
            assert!(all.iter().any(|entry| entry.command == Command::Go(page)));
        }
        assert!(!labels(&all).contains(&"Show or hide Assistant"));
        assert!(labels(&entries(true, true)).contains(&"Show or hide Assistant"));
        assert!(!labels(&entries(false, true)).contains(&"Show or hide Assistant"));
    }

    #[test]
    fn filtering_matches_every_word_over_label_and_group_and_never_runs_text() {
        let all = entries(true, true);
        let jobs = filter(all.clone(), "job");
        assert!(!jobs.is_empty());
        assert!(jobs
            .iter()
            .all(|entry| entry.label.to_lowercase().contains("job")));
        // Free-form text is only a filter. It never matches an action or runs anything.
        assert!(filter(all.clone(), "run any shell command").is_empty());
        assert_eq!(filter(all.clone(), "").len(), all.len());
        assert!(!filter(all.clone(), "go to jobs").is_empty());
        assert!(filter(all, "go to nonsense").is_empty());
    }

    #[test]
    fn arrows_wrap_and_an_empty_list_stays_put() {
        let mut panel = CommandPanel::default();
        panel.step(1, 3);
        panel.step(1, 3);
        assert_eq!(panel.active, 2);
        panel.step(1, 3);
        assert_eq!(panel.active, 0);
        panel.step(-1, 3);
        assert_eq!(panel.active, 2);
        panel.step(1, 0);
        assert_eq!(panel.active, 0);
    }

    #[test]
    fn running_a_command_closes_the_panel_and_navigates_without_starting_work() {
        let mut state = crate::tests::registered_fixture();
        state.open_command_panel();
        assert!(state.commands.open);
        state.run_command(Command::Go(Page::Seed));
        assert!(!state.commands.open);
        assert_eq!(state.page, Page::Seed);
        assert!(state.maintenance.confirmation.is_none());
        state.open_command_panel();
        state.run_command(Command::SetupGateway);
        assert!(state.setup_gateway);
    }

    #[test]
    fn opening_takes_the_overlay_from_settings_and_the_inbox() {
        let mut state = crate::tests::registered_fixture();
        state.settings_open = true;
        state.inbox.open = true;
        state.open_command_panel();
        assert!(!state.settings_open);
        assert!(!state.inbox.open);
        assert!(state.commands.open);
    }

    #[test]
    fn real_view_opens_filters_and_runs_from_the_keyboard_and_pointer() {
        let mut state = crate::tests::registered_fixture();
        state.page = Page::Workstation;
        let mut runtime = rune::Runtime::new(
            state,
            crate::app::view,
            (1280.0, 800.0),
            rune::Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        runtime.frame(0.0);
        assert!(runtime.find("command-open").is_some());
        assert!(runtime.find("command-list").is_none());

        runtime.update(|state, _| state.open_command_panel());
        runtime.frame(1.0 / 60.0);
        runtime.frame(2.0 / 60.0);
        assert!(runtime.find("command-search").is_some());
        assert!(runtime.find("command-panel-dialog").is_some());
        assert!(runtime.find("command-panel-scrim").is_some());
        assert!(runtime.find("close").is_some());
        assert!(runtime.find("command-0").is_some());

        let (x, y) = runtime.find("command-search").expect("search field");
        runtime.pointer_down(x, y);
        runtime.pointer_up(x, y);
        runtime.update(|state, _| {
            state.commands.query = TextEdit::new("jobs");
            state.commands.active = 0;
        });
        runtime.frame(3.0 / 60.0);
        assert!(runtime.find("command-1").is_none());
        let (x, y) = runtime.find("command-0").expect("filtered row");
        runtime.pointer_down(x, y);
        runtime.pointer_up(x, y);
        runtime.frame(4.0 / 60.0);
        assert!(!runtime.state.commands.open);
        assert_eq!(runtime.state.page, Page::Jobs);
    }

    #[test]
    fn opening_the_panel_puts_keyboard_focus_in_the_filter_without_a_click() {
        let mut runtime = rune::Runtime::new(
            crate::tests::registered_fixture(),
            crate::app::view,
            (1280.0, 800.0),
            rune::Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        let focused_filter = |runtime: &rune::Runtime<State>| {
            runtime.elements().iter().any(|element| {
                element.focused
                    && element.key.as_deref() == Some("input")
                    && element.path.contains("command-search")
            })
        };
        runtime.frame(0.0);
        assert!(!focused_filter(&runtime));
        for opening in 0..2 {
            runtime.update(|state, _| state.open_command_panel());
            for step in 1..5 {
                runtime.frame(f64::from(opening * 10 + step) / 60.0);
            }
            assert!(
                focused_filter(&runtime),
                "the filter has focus after opening {opening}"
            );
            assert!(!runtime.state.commands.focus_pending, "requested once");
            runtime.update(|state, _| state.commands.close());
            runtime.frame(f64::from(opening * 10 + 6) / 60.0);
        }
    }
}
