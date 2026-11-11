//! Shell state: focus, selection, scrolling, modal input and event decoding.

use anyhow::Result;
use crossterm::event::Event;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyEventKind;
use crossterm::event::KeyModifiers;
use crossterm::event::MouseButton;
use crossterm::event::MouseEvent;
use crossterm::event::MouseEventKind;
use ratatui::layout::Position;
use ratatui::layout::Rect;

/// Which pane receives arrows and the wheel.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Pane {
    List,
    Detail,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PromptKind {
    /// Free text edited in the modal.
    Input,
    /// Explicit `y` confirms; `n` and Esc cancel. Enter never confirms.
    Confirm,
}

/// A modal raised by a view; its answer returns through `View::submit`.
#[derive(Clone, Debug)]
pub(crate) struct Prompt {
    pub tag: &'static str,
    pub label: String,
    pub kind: PromptKind,
    pub initial: String,
}

impl Prompt {
    pub fn input(tag: &'static str, label: impl Into<String>, initial: impl Into<String>) -> Self {
        Self {
            tag,
            label: label.into(),
            kind: PromptKind::Input,
            initial: initial.into(),
        }
    }

    pub fn confirm(tag: &'static str, label: impl Into<String>) -> Self {
        Self {
            tag,
            label: label.into(),
            kind: PromptKind::Confirm,
            initial: String::new(),
        }
    }
}

/// Result of a view action: an optional status and an optional follow-up modal.
#[derive(Debug, Default)]
pub(crate) struct Step {
    pub status: Option<String>,
    pub prompt: Option<Prompt>,
}

impl Step {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn status(status: impl Into<String>) -> Self {
        Self {
            status: Some(status.into()),
            prompt: None,
        }
    }

    pub fn prompt(prompt: Prompt) -> Self {
        Self {
            status: None,
            prompt: Some(prompt),
        }
    }
}

/// Work the shell hands to the view; it may block on the network.
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum Action {
    Refresh,
    Select(usize),
    Activate(usize),
    Key(char, usize),
    Submit(&'static str, String),
}

pub(crate) struct Active {
    pub prompt: Prompt,
    pub buffer: String,
}

pub(crate) struct Shell {
    pub(super) selected: usize,
    pub(super) scroll: usize,
    pub(super) focus: Pane,
    pub(super) status: String,
    pub(super) prompt: Option<Active>,
    quit: bool,
    // Geometry recorded by the last draw. Hit testing reuses the drawn rectangles.
    pub(super) list_area: Rect,
    pub(super) detail_area: Rect,
    pub(super) list_offset: usize,
    pub(super) stride: usize,
    pub(super) detail_max: usize,
    pub(super) compact: bool,
}

impl Shell {
    pub fn new() -> Self {
        Self {
            selected: 0,
            scroll: 0,
            focus: Pane::List,
            status: String::new(),
            prompt: None,
            quit: false,
            list_area: Rect::default(),
            detail_area: Rect::default(),
            list_offset: 0,
            stride: 2,
            detail_max: 0,
            compact: false,
        }
    }

    pub fn quit(&self) -> bool {
        self.quit
    }

    pub fn status(&mut self, status: impl AsRef<str>) {
        self.status = sanitize(status.as_ref());
    }

    /// Apply a view result and keep the selection inside `rows`.
    pub fn apply(&mut self, result: Result<Step>, rows: usize) {
        match result {
            Ok(step) => {
                self.status = step.status.map(|text| sanitize(&text)).unwrap_or_default();
                if let Some(prompt) = step.prompt {
                    self.prompt = Some(Active {
                        buffer: prompt.initial.clone(),
                        prompt,
                    });
                }
            }
            Err(error) => self.status = format!("Error: {}", sanitize(&format!("{error:#}"))),
        }
        self.selected = self.selected.min(rows.saturating_sub(1));
    }

    /// Decode one terminal event into view work.
    pub fn event(&mut self, event: Event, rows: usize) -> Option<Action> {
        match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => self.key(key, rows),
            Event::Mouse(mouse) => self.mouse(mouse, rows),
            _ => None,
        }
    }

    fn key(&mut self, key: KeyEvent, rows: usize) -> Option<Action> {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.quit = true;
            return None;
        }
        if self.prompt.is_some() {
            return self.prompt_key(key);
        }
        let page = self.detail_area.height.saturating_sub(2).max(1) as usize;
        match key.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Esc if self.compact && self.focus == Pane::Detail => self.focus = Pane::List,
            KeyCode::Esc => self.quit = true,
            KeyCode::Tab | KeyCode::BackTab => {
                self.focus = match self.focus {
                    Pane::List => Pane::Detail,
                    Pane::Detail => Pane::List,
                }
            }
            KeyCode::Up | KeyCode::Char('k') => return self.step(-1, rows),
            KeyCode::Down | KeyCode::Char('j') => return self.step(1, rows),
            KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(page),
            KeyCode::PageDown => self.scroll = (self.scroll + page).min(self.detail_max),
            KeyCode::Home => return self.jump(0, rows),
            KeyCode::End => return self.jump(rows.saturating_sub(1), rows),
            KeyCode::Enter => {
                self.focus = Pane::Detail;
                self.scroll = 0;
                return (rows > 0).then_some(Action::Activate(self.selected));
            }
            KeyCode::Char('r') => return Some(Action::Refresh),
            KeyCode::Char(character) => return Some(Action::Key(character, self.selected)),
            _ => (),
        }
        None
    }

    fn prompt_key(&mut self, key: KeyEvent) -> Option<Action> {
        let active = self.prompt.as_mut()?;
        match (active.prompt.kind, key.code) {
            (_, KeyCode::Esc) => {
                self.prompt = None;
                self.status = "No action sent.".into();
            }
            (PromptKind::Confirm, KeyCode::Char('y' | 'Y')) => {
                let tag = active.prompt.tag;
                self.prompt = None;
                return Some(Action::Submit(tag, "yes".into()));
            }
            (PromptKind::Confirm, KeyCode::Char('n' | 'N')) => {
                self.prompt = None;
                self.status = "No action sent.".into();
            }
            (PromptKind::Input, KeyCode::Enter) => {
                let value = active.buffer.trim().to_owned();
                let tag = active.prompt.tag;
                self.prompt = None;
                if value.is_empty() {
                    self.status = "No action sent.".into();
                } else {
                    return Some(Action::Submit(tag, value));
                }
            }
            (PromptKind::Input, KeyCode::Backspace) => {
                active.buffer.pop();
            }
            (PromptKind::Input, KeyCode::Char(character))
                if !key.modifiers.contains(KeyModifiers::CONTROL) && !character.is_control() =>
            {
                active.buffer.push(character);
            }
            _ => (),
        }
        None
    }

    fn step(&mut self, delta: isize, rows: usize) -> Option<Action> {
        if self.focus == Pane::Detail {
            self.scroll = if delta < 0 {
                self.scroll.saturating_sub(1)
            } else {
                (self.scroll + 1).min(self.detail_max)
            };
            return None;
        }
        self.jump(
            self.selected
                .saturating_add_signed(delta)
                .min(rows.saturating_sub(1)),
            rows,
        )
    }

    fn jump(&mut self, index: usize, rows: usize) -> Option<Action> {
        if rows == 0 || index == self.selected {
            return None;
        }
        self.selected = index.min(rows - 1);
        self.scroll = 0;
        Some(Action::Select(self.selected))
    }

    fn mouse(&mut self, mouse: MouseEvent, rows: usize) -> Option<Action> {
        if self.prompt.is_some() {
            return None;
        }
        let position = Position::new(mouse.column, mouse.row);
        let in_list = self.list_area.contains(position);
        match mouse.kind {
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                let up = mouse.kind == MouseEventKind::ScrollUp;
                if in_list {
                    return self.jump(
                        self.selected
                            .saturating_add_signed(if up { -1 } else { 1 })
                            .min(rows.saturating_sub(1)),
                        rows,
                    );
                }
                if self.detail_area.contains(position) {
                    self.scroll = if up {
                        self.scroll.saturating_sub(1)
                    } else {
                        (self.scroll + 1).min(self.detail_max)
                    };
                }
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if in_list {
                    self.focus = Pane::List;
                    let inner = usize::from(mouse.row.checked_sub(self.list_area.y + 1)?);
                    let stride = self.stride.max(1);
                    let row = inner / stride + self.list_offset;
                    if row < rows && inner % stride == 0 {
                        self.selected = row;
                        self.scroll = 0;
                        return Some(Action::Select(row));
                    }
                } else if self.detail_area.contains(position) {
                    self.focus = Pane::Detail;
                }
            }
            _ => (),
        }
        None
    }
}

/// Replace control characters so service text cannot move the cursor.
pub(crate) fn sanitize(text: &str) -> String {
    text.chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use crossterm::event::KeyEventState;

    use super::*;

    fn press(code: KeyCode) -> Event {
        Event::Key(KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        })
    }

    #[test]
    fn arrows_move_selection_and_request_a_detail_load() {
        let mut shell = Shell::new();
        assert_eq!(
            shell.event(press(KeyCode::Down), 3),
            Some(Action::Select(1))
        );
        assert_eq!(shell.event(press(KeyCode::Up), 3), Some(Action::Select(0)));
        assert_eq!(shell.event(press(KeyCode::Up), 3), None);
        assert_eq!(shell.event(press(KeyCode::End), 3), Some(Action::Select(2)));
    }

    #[test]
    fn enter_activates_and_focuses_detail() {
        let mut shell = Shell::new();
        assert_eq!(
            shell.event(press(KeyCode::Enter), 2),
            Some(Action::Activate(0))
        );
        assert_eq!(shell.focus, Pane::Detail);
        assert_eq!(shell.event(press(KeyCode::Enter), 0), None);
    }

    #[test]
    fn confirmation_requires_an_explicit_y() {
        let mut shell = Shell::new();
        shell.apply(Ok(Step::prompt(Prompt::confirm("retry", "Retry?"))), 1);
        assert_eq!(shell.event(press(KeyCode::Enter), 1), None);
        assert!(shell.prompt.is_some());
        assert_eq!(
            shell.event(press(KeyCode::Char('y')), 1),
            Some(Action::Submit("retry", "yes".into()))
        );
        shell.apply(Ok(Step::prompt(Prompt::confirm("retry", "Retry?"))), 1);
        assert_eq!(shell.event(press(KeyCode::Esc), 1), None);
        assert!(shell.prompt.is_none());
        assert_eq!(shell.status, "No action sent.");
    }

    #[test]
    fn input_prompt_edits_and_submits_trimmed_text() {
        let mut shell = Shell::new();
        shell.apply(Ok(Step::prompt(Prompt::input("find", "Find", "ab"))), 0);
        shell.event(press(KeyCode::Char('c')), 0);
        shell.event(press(KeyCode::Backspace), 0);
        shell.event(press(KeyCode::Char('d')), 0);
        assert_eq!(
            shell.event(press(KeyCode::Enter), 0),
            Some(Action::Submit("find", "abd".into()))
        );
    }

    #[test]
    fn hotkeys_and_quit_keys() {
        let mut shell = Shell::new();
        assert_eq!(
            shell.event(press(KeyCode::Char('t')), 2),
            Some(Action::Key('t', 0))
        );
        assert_eq!(
            shell.event(press(KeyCode::Char('r')), 2),
            Some(Action::Refresh)
        );
        assert!(!shell.quit());
        shell.event(press(KeyCode::Char('q')), 2);
        assert!(shell.quit());
    }

    #[test]
    fn errors_become_a_sanitized_status() {
        let mut shell = Shell::new();
        shell.apply(Err(anyhow::anyhow!("bad\u{1b}[2J")), 0);
        assert_eq!(shell.status, "Error: bad [2J");
    }
}
