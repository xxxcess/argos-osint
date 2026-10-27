//! The launcher uses the same semantic palette as the investigation TUI.
use super::{configure, Prompts, Selection};
use crate::tui::theme;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Layout, Rect},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
    Frame, Terminal,
};
use std::io::{self, Stdout};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

struct Restore;
impl Drop for Restore {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            io::stdout(),
            event::DisableMouseCapture,
            event::DisableBracketedPaste,
            LeaveAlternateScreen,
            crossterm::cursor::Show
        );
    }
}

pub(super) fn run(existing: Option<&str>, notice: Option<&str>) -> anyhow::Result<Option<Selection>> {
    enable_raw_mode()?;
    let _restore = Restore;
    execute!(
        io::stdout(),
        EnterAlternateScreen,
        event::EnableMouseCapture,
        event::EnableBracketedPaste
    )?;
    let terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    let initial_notice = notice.unwrap_or_default().to_string();
    let mut prompts = Screen {
        terminal,
        history: vec![format!(
            "Home: {}",
            argos_osint_core::paths::ensure_home()?.display()
        )],
        notice: initial_notice.clone(),
        navigation: Navigation::default(),
    };
    loop {
        prompts.navigation.begin();
        prompts.history.truncate(1);
        if prompts.notice.is_empty() {
            prompts.notice = initial_notice.clone();
        }
        let selection = configure(&mut prompts, existing)?;
        if !prompts.navigation.back_requested {
            return Ok(selection);
        }
    }
}

struct Screen {
    terminal: Terminal<CrosstermBackend<Stdout>>,
    history: Vec<String>,
    notice: String,
    navigation: Navigation,
}

#[derive(Default)]
struct Navigation {
    answers: Vec<(String, String)>,
    next: usize,
    replay_until: usize,
    back_requested: bool,
}
impl Navigation {
    fn begin(&mut self) {
        self.next = 0;
        self.back_requested = false;
    }
    fn remembered(
        &self,
        index: usize,
        label: &str,
        valid: &dyn Fn(&str) -> bool,
    ) -> Option<String> {
        self.answers
            .get(index)
            .filter(|(saved_label, value)| saved_label == label && valid(value))
            .map(|(_, value)| value.clone())
    }
    fn remember(&mut self, index: usize, label: &str, value: &str) {
        let answer = (label.to_owned(), value.to_owned());
        if self.answers.get(index) != Some(&answer) {
            self.answers.truncate(index);
            self.answers.push(answer);
        }
    }
    fn previous(&mut self, index: usize) {
        if index > 0 {
            self.replay_until = index - 1;
            self.back_requested = true;
        }
    }
}

impl Prompts for Screen {
    fn message(&mut self, message: &str) -> io::Result<()> {
        if !message.contains("Welcome to Argos")
            && !message.contains("1. TUI")
            && !message.starts_with("Launch:")
            && !message.starts_with("Storage setup:")
        {
            self.notice = message.to_string();
        }
        Ok(())
    }

    fn ask(
        &mut self,
        label: &str,
        default: &str,
        valid: &dyn Fn(&str) -> bool,
    ) -> io::Result<Option<String>> {
        let index = self.navigation.next;
        self.navigation.next += 1;
        let saved = self.navigation.remembered(index, label, valid);
        if index < self.navigation.replay_until {
            if let Some(value) = &saved {
                let question = Question::new(label, value, valid);
                self.history
                    .push(format!("{}: {}", question.title(), question.shown_value()));
                return Ok(Some(value.clone()));
            }
            self.navigation.replay_until = index;
        }
        let mut question = Question::new(label, saved.as_deref().unwrap_or(default), valid);
        question.can_back = index > 0;
        loop {
            self.terminal
                .draw(|frame| question.draw(frame, &self.history, &self.notice))?;
            let event = event::read()?;
            if matches!(event, Event::Key(key) if key.kind == KeyEventKind::Press && key.code == KeyCode::Esc)
            {
                self.navigation.remember(index, label, question.raw_value());
                self.navigation.previous(index);
                return Ok(None);
            }
            if let Some(answer) = question.event(event, valid) {
                if let Some(value) = &answer {
                    self.navigation.remember(index, label, value);
                    self.history
                        .push(format!("{}: {}", question.title(), question.shown_value()));
                }
                return Ok(answer);
            }
        }
    }
}

struct Question {
    label: String,
    value: String,
    cursor: usize,
    choices: Vec<(String, String)>,
    selected: usize,
    error: String,
    rows: Vec<Rect>,
    pressed: Option<(usize, u16, u16)>,
    history_scroll: usize,
    too_small: bool,
    can_back: bool,
}

impl Question {
    fn new(label: &str, default: &str, valid: &dyn Fn(&str) -> bool) -> Self {
        let candidates: Vec<(&str, &str)> = if label.starts_with("Choose a service") {
            vec![("1", "TUI"), ("2", "API Server"), ("3", "Web")]
        } else if label.starts_with("Web profile") {
            vec![("dev", "Development"), ("installed", "Built static app")]
        } else if label.starts_with("API [") {
            vec![
                ("start", "Start a local API"),
                ("connect", "Connect to an existing API"),
            ]
        } else if label.starts_with("Storage mode") {
            vec![
                ("keep", "Keep current storage"),
                ("embedded", "Embedded"),
                ("server", "SurrealDB server"),
            ]
        } else if label.starts_with("Terminal display") {
            vec![
                ("tui", "TUI · Jobs / Logs available"),
                ("logs", "Server logs"),
                ("cancel", "Cancel"),
            ]
        } else {
            Vec::new()
        };
        let choices: Vec<_> = candidates
            .into_iter()
            .filter(|(key, _)| valid(key))
            .map(|(key, name)| (key.into(), name.into()))
            .collect();
        let selected = choices
            .iter()
            .position(|(key, _)| key == default)
            .unwrap_or(0);
        Self {
            label: label.into(),
            value: default.into(),
            cursor: default.len(),
            choices,
            selected,
            error: String::new(),
            rows: Vec::new(),
            pressed: None,
            history_scroll: 0,
            too_small: false,
            can_back: false,
        }
    }

    fn raw_value(&self) -> &str {
        self.choices
            .get(self.selected)
            .map_or(self.value.as_str(), |(key, _)| key)
    }

    fn shown_value(&self) -> &str {
        self.choices
            .get(self.selected)
            .map_or(self.value.as_str(), |(_, name)| name)
    }

    fn title(&self) -> &str {
        self.label.split('[').next().unwrap_or(&self.label).trim()
    }

    fn draw(&mut self, frame: &mut Frame, history: &[String], notice: &str) {
        let area = frame.area();
        frame.render_widget(Paragraph::new("").style(theme::text()), area);
        self.too_small = area.width < 40 || area.height < 14;
        self.rows.clear();
        if self.too_small {
            frame.render_widget(
                Paragraph::new(if self.can_back {
                    "ARGOS VENTURE\nService launcher\n\nResize to at least 40 × 14.\nEsc goes back."
                } else {
                    "ARGOS VENTURE\nService launcher\n\nResize to at least 40 × 14.\nEsc quits."
                })
                .style(theme::accent())
                .centered(),
                Rect::new(
                    area.x,
                    area.y + area.height.saturating_sub(5) / 2,
                    area.width,
                    area.height.min(5),
                ),
            );
            return;
        }
        let logo = crate::tui::wordmark::venture_rows();
        let large = area.width >= logo.iter().map(Line::width).max().unwrap_or(0) as u16 + 4
            && area.height >= 38;
        let item_height = if area.height >= 30 { 2 } else { 1 };
        let card_height = if self.choices.is_empty() {
            6
        } else {
            self.choices.len() as u16 * item_height + 4
        };
        let brand_height = if large { 8 } else { 3 };
        let summary_height = (history.len() + usize::from(!notice.is_empty())).min(8) as u16;
        let content_height = brand_height + card_height + summary_height + 4;
        let height = content_height.min(area.height.saturating_sub(2));
        let width = area.width.saturating_sub(4);
        let outer = Rect::new(
            area.x + (area.width - width) / 2,
            area.y + (area.height - height) / 2,
            width,
            height,
        );
        let parts = Layout::vertical([
            Constraint::Length(brand_height),
            Constraint::Length(card_height),
            Constraint::Min(1),
            Constraint::Length(2),
        ])
        .split(outer);
        let mut brand: Vec<Line<'_>> = if large {
            logo
        } else {
            vec![Line::styled("ARGOS VENTURE", theme::accent())]
        };
        brand.push(Line::styled(
            if area.width < 60 {
                "Welcome · choose your service"
            } else {
                "Welcome · choose your service and launch settings"
            },
            theme::dim(),
        ));
        frame.render_widget(Paragraph::new(brand).centered(), parts[0]);
        let card_width = parts[1].width.min(76);
        let card = Rect::new(
            parts[1].x + (parts[1].width - card_width) / 2,
            parts[1].y,
            card_width,
            parts[1].height,
        );
        frame.render_widget(
            theme::panel(self.title())
                .border_style(theme::accent())
                .title_alignment(ratatui::layout::Alignment::Center),
            card,
        );
        let inner = card.inner(ratatui::layout::Margin::new(2, 1));
        if self.choices.is_empty() {
            let width = (self.value.width().saturating_add(1) as u16)
                .min(inner.width)
                .max(1);
            let field = Rect::new(inner.x + (inner.width - width) / 2, inner.y, width, 1);
            let max = field.width.saturating_sub(1) as usize;
            let before = &self.value[..self.cursor];
            let mut offset = self.cursor;
            let mut used = 0;
            for (index, c) in before.char_indices().rev() {
                let width = c.width().unwrap_or(0);
                if used + width > max {
                    break;
                }
                offset = index;
                used += width;
            }
            frame.render_widget(
                Paragraph::new(&self.value[offset..]).style(theme::selected()),
                field,
            );
            let column = self.value[offset..self.cursor].width().min(max) as u16;
            frame.set_cursor_position((field.x + column, field.y));
            frame.render_widget(
                Paragraph::new(if area.width < 60 {
                    "Ctrl+U clear · Enter next"
                } else {
                    "Type a value · Ctrl+U clears · Enter continues"
                })
                .style(theme::muted())
                .centered(),
                Rect::new(inner.x, inner.y + 2, inner.width, 1),
            );
        } else {
            for (index, (_, name)) in self.choices.iter().enumerate() {
                let row = Rect::new(
                    inner.x,
                    inner.y + index as u16 * item_height,
                    inner.width,
                    item_height,
                )
                .intersection(inner);
                self.rows.push(row);
                let style = if index == self.selected {
                    theme::selected().fg(theme::ACCENT)
                } else {
                    theme::text()
                };
                frame.render_widget(
                    Paragraph::new(format!(
                        "{} {}  {name}",
                        if index == self.selected { "›" } else { " " },
                        index + 1
                    ))
                    .style(style)
                    .centered(),
                    row,
                );
            }
        }
        if !self.error.is_empty() {
            frame.render_widget(
                Paragraph::new(&*self.error)
                    .style(theme::error())
                    .centered(),
                Rect::new(inner.x, parts[1].bottom().saturating_sub(2), inner.width, 1),
            );
        }
        let summary = parts[2].inner(ratatui::layout::Margin::new(1, 1));
        let room = summary.height as usize;
        let mut lines: Vec<Line<'_>> = history
            .iter()
            .map(|value| Line::from(vec![Span::styled(value, theme::dim())]))
            .collect();
        if !notice.is_empty() {
            let style = if notice.to_ascii_lowercase().contains("fail")
                || notice.to_ascii_lowercase().contains("error")
            {
                theme::error()
            } else {
                theme::muted()
            };
            lines.push(Line::styled(notice, style));
        }
        let start = lines.len().saturating_sub(room + self.history_scroll);
        frame.render_widget(
            Paragraph::new(lines.into_iter().skip(start).take(room).collect::<Vec<_>>()).centered(),
            summary,
        );
        let escape = if self.can_back {
            "Esc back"
        } else {
            "Esc quit"
        };
        let help = if area.width < 60 {
            format!("↑↓ select · Enter next · {escape}\nCtrl+Q quit · PgUp/Dn settings")
        } else {
            format!("↑↓ / Tab select · Enter continue · {escape}\nCtrl+Q quit · PgUp/PgDn settings · mouse selects")
        };
        frame.render_widget(
            Paragraph::new(help)
                .style(theme::muted())
                .centered()
                .wrap(Wrap { trim: false }),
            parts[3],
        );
    }

    fn accept(&mut self, valid: &dyn Fn(&str) -> bool) -> Option<Option<String>> {
        let value = self
            .choices
            .get(self.selected)
            .map_or(self.value.trim(), |(key, _)| key);
        if matches!(value, "q" | "quit" | "exit" | "cancel") {
            return Some(None);
        }
        if valid(value) {
            return Some(Some(value.into()));
        }
        self.error = "Enter a valid port or loopback URL.".into();
        None
    }

    fn event(&mut self, event: Event, valid: &dyn Fn(&str) -> bool) -> Option<Option<String>> {
        if let Event::Key(key) = event {
            if key.kind != KeyEventKind::Press {
                return None;
            }
            if key.code == KeyCode::Esc
                || (key.modifiers.contains(KeyModifiers::CONTROL)
                    && matches!(key.code, KeyCode::Char('c' | 'q')))
            {
                return Some(None);
            }
            if self.too_small {
                return None;
            }
            match key.code {
                KeyCode::Enter => return self.accept(valid),
                KeyCode::PageUp => self.history_scroll = self.history_scroll.saturating_add(3),
                KeyCode::PageDown => self.history_scroll = self.history_scroll.saturating_sub(3),
                KeyCode::Char('q') if !self.choices.is_empty() => return Some(None),
                KeyCode::Up | KeyCode::BackTab if !self.choices.is_empty() => {
                    self.selected = (self.selected + self.choices.len() - 1) % self.choices.len()
                }
                KeyCode::Down | KeyCode::Tab if !self.choices.is_empty() => {
                    self.selected = (self.selected + 1) % self.choices.len()
                }
                KeyCode::Char(c) if !self.choices.is_empty() => {
                    if let Some(index) = c
                        .to_digit(10)
                        .and_then(|n| n.checked_sub(1))
                        .filter(|n| (*n as usize) < self.choices.len())
                    {
                        self.selected = index as usize;
                    }
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.value.clear();
                    self.cursor = 0;
                }
                KeyCode::Home => self.cursor = 0,
                KeyCode::End => self.cursor = self.value.len(),
                KeyCode::Left => {
                    self.cursor = self.value[..self.cursor]
                        .char_indices()
                        .last()
                        .map_or(0, |(index, _)| index)
                }
                KeyCode::Right => {
                    if let Some(c) = self.value[self.cursor..].chars().next() {
                        self.cursor += c.len_utf8();
                    }
                }
                KeyCode::Backspace => {
                    if let Some((index, _)) = self.value[..self.cursor].char_indices().last() {
                        self.value.replace_range(index..self.cursor, "");
                        self.cursor = index;
                    }
                }
                KeyCode::Delete => {
                    if let Some(c) = self.value[self.cursor..].chars().next() {
                        self.value
                            .replace_range(self.cursor..self.cursor + c.len_utf8(), "");
                    }
                }
                KeyCode::Char(c)
                    if !c.is_control()
                        && !key.modifiers.contains(KeyModifiers::CONTROL)
                        && self.value.len() + c.len_utf8() <= 512 =>
                {
                    self.value.insert(self.cursor, c);
                    self.cursor += c.len_utf8();
                }
                _ => {}
            }
            self.error.clear();
        } else if let Event::Mouse(mouse) = event {
            let hit = self
                .rows
                .iter()
                .position(|row| row.contains((mouse.column, mouse.row).into()));
            match mouse.kind {
                MouseEventKind::Down(MouseButton::Left) => {
                    self.pressed = hit.map(|index| (index, mouse.column, mouse.row));
                }
                MouseEventKind::Up(MouseButton::Left) => {
                    if let (Some(index), Some((pressed, x, y))) = (hit, self.pressed.take()) {
                        if index == pressed
                            && x.abs_diff(mouse.column) <= 1
                            && y.abs_diff(mouse.row) <= 1
                        {
                            self.selected = index;
                            return self.accept(valid);
                        }
                    }
                }
                _ => {}
            }
        } else if let Event::Paste(text) = event {
            if self.choices.is_empty() {
                let mut remaining = 512usize.saturating_sub(self.value.len());
                let clean: String = text
                    .chars()
                    .filter(|c| !c.is_control())
                    .take_while(|c| {
                        if c.len_utf8() > remaining {
                            false
                        } else {
                            remaining -= c.len_utf8();
                            true
                        }
                    })
                    .collect();
                self.value.insert_str(self.cursor, &clean);
                self.cursor += clean.len();
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(code: KeyCode) -> Event {
        Event::Key(event::KeyEvent::new(code, KeyModifiers::NONE))
    }

    #[test]
    fn venture_back_retains_values_and_revalidates_changed_branches() {
        let mut navigation = Navigation::default();
        navigation.remember(0, "Service", "web");
        navigation.remember(1, "Web port", "4010");
        navigation.remember(2, "Profile", "dev");
        navigation.previous(2);
        assert!(navigation.back_requested);
        assert_eq!(navigation.replay_until, 1);
        navigation.begin();
        assert!(!navigation.back_requested);
        assert_eq!(
            navigation
                .remembered(1, "Web port", &super::super::port)
                .as_deref(),
            Some("4010")
        );
        navigation.remember(1, "Web port", "4010");
        assert_eq!(
            navigation.answers.len(),
            3,
            "unchanged answers keep valid later drafts"
        );
        navigation.remember(0, "Service", "tui");
        assert_eq!(
            navigation.answers.len(),
            1,
            "changed branch discards later answers"
        );
        assert!(navigation.remembered(1, "Storage", &|_| true).is_none());
        navigation.previous(0);
        assert!(!navigation.back_requested, "Escape at root leaves Venture");
    }

    #[test]
    fn keyboard_edits_validate_and_terminal_choice_is_final() {
        let mut field = Question::new("Web port", "3000", &|v| {
            v.parse::<u16>().is_ok_and(|p| p > 0)
        });
        field.value = "0".into();
        field.cursor = 1;
        assert!(field
            .event(key(KeyCode::Enter), &super::super::port)
            .is_none());
        assert!(!field.error.is_empty());
        field.event(key(KeyCode::Backspace), &super::super::port);
        for c in "9000".chars() {
            field.event(key(KeyCode::Char(c)), &super::super::port);
        }
        assert_eq!(
            field.event(key(KeyCode::Enter), &super::super::port),
            Some(Some("9000".into()))
        );
        let valid = |v: &str| matches!(v, "tui" | "logs" | "cancel");
        let mut terminal = Question::new("Terminal display", "logs", &valid);
        terminal.event(key(KeyCode::Up), &valid);
        assert_eq!(
            terminal.event(key(KeyCode::Enter), &valid),
            Some(Some("tui".into()))
        );
        assert_eq!(terminal.event(key(KeyCode::Esc), &valid), Some(None));
    }

    #[test]
    fn launcher_cells_and_mouse_targets() {
        use ratatui::{backend::TestBackend, style::Modifier};
        for (width, height) in [(160, 50), (120, 40), (80, 24), (60, 18), (40, 20), (30, 10)] {
            for (name, label, default, valid) in [
                (
                    "service",
                    "Choose a service",
                    "",
                    (|v: &str| matches!(v, "1" | "2" | "3")) as fn(&str) -> bool,
                ),
                (
                    "terminal",
                    "Terminal display",
                    "logs",
                    (|v: &str| matches!(v, "tui" | "logs" | "cancel")) as fn(&str) -> bool,
                ),
                (
                    "port",
                    "API port",
                    "8080",
                    super::super::port as fn(&str) -> bool,
                ),
                (
                    "invalid-port",
                    "API port",
                    "0",
                    super::super::port as fn(&str) -> bool,
                ),
            ] {
                let mut question = Question::new(label, default, &valid);
                question.can_back = !label.starts_with("Choose a service");
                if name == "invalid-port" {
                    question.accept(&valid);
                }
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                let mut history = vec!["Home: /Users/demo/.argos".into()];
                if name != "service" {
                    history.push("Service: Web".into());
                }
                if name == "terminal" {
                    history.extend(
                        [
                            "Web port: 3000",
                            "Web profile: Development",
                            "API: Start locally",
                            "API port: 8080",
                            "Storage: Embedded",
                        ]
                        .map(String::from),
                    );
                }
                terminal
                    .draw(|frame| question.draw(frame, &history, ""))
                    .unwrap();
                for row in &question.rows {
                    assert!(row.right() <= width && row.bottom() <= height);
                    assert!(
                        (i32::from(row.x) * 2 + i32::from(row.width) - i32::from(width)).abs() <= 1
                    );
                }
                if width >= 40 && name == "service" {
                    let row = question.rows[1];
                    for kind in [
                        MouseEventKind::Down(MouseButton::Left),
                        MouseEventKind::Up(MouseButton::Left),
                    ] {
                        let result = question.event(
                            Event::Mouse(event::MouseEvent {
                                kind,
                                column: row.x,
                                row: row.y,
                                modifiers: KeyModifiers::NONE,
                            }),
                            &valid,
                        );
                        if matches!(kind, MouseEventKind::Up(_)) {
                            assert_eq!(result, Some(Some("2".into())));
                        }
                    }
                }
                if let Ok(folder) = std::env::var("ARGOS_SCREEN_DIR") {
                    let buffer = terminal.backend().buffer();
                    let cells: Vec<Vec<_>> = (0..height).map(|y| (0..width).map(|x| { let c = &buffer[(x,y)]; serde_json::json!({"s":c.symbol(),"fg":format!("{:?}",c.fg),"bg":format!("{:?}",c.bg),"b":c.modifier.contains(Modifier::BOLD),"u":c.modifier.contains(Modifier::UNDERLINED)}) }).collect()).collect();
                    std::fs::create_dir_all(&folder).unwrap();
                    std::fs::write(
                        std::path::Path::new(&folder)
                            .join(format!("launcher-{name}-{width}x{height}.json")),
                        serde_json::to_vec(
                            &serde_json::json!({"width":width,"height":height,"cells":cells}),
                        )
                        .unwrap(),
                    )
                    .unwrap();
                }
            }
        }
    }

    #[test]
    fn unicode_field_cursor_uses_cell_width_and_edits_whole_characters() {
        use ratatui::backend::TestBackend;
        let mut question = Question::new("API endpoint", "好abc", &|_| true);
        question.event(key(KeyCode::Home), &|_| true);
        question.event(key(KeyCode::Right), &|_| true);
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|frame| question.draw(frame, &[], ""))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let (x, y) = (0..24)
            .flat_map(|y| (0..80).map(move |x| (x, y)))
            .find(|(x, y)| buffer[(*x, *y)].symbol() == "好")
            .unwrap();
        let cursor = terminal.get_cursor_position().unwrap();
        assert_eq!((cursor.x, cursor.y), (x + 2, y));
        question.event(key(KeyCode::Backspace), &|_| true);
        assert_eq!(question.value, "abc");
    }

    #[test]
    fn error_notice_is_rendered_in_summary_area() {
        use ratatui::backend::TestBackend;
        let mut question = Question::new("Choose a service", "1", &|_| true);
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|frame| {
                question.draw(
                    frame,
                    &["Home: /Users/demo/.argos".into()],
                    "Launch failed: port in use",
                )
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let rendered: String = (0..24)
            .flat_map(|y| {
                let mut line: String = (0..80).map(|x| buffer[(x, y)].symbol()).collect();
                line.push('\n');
                line.into_bytes()
            })
            .map(|b| b as char)
            .collect();
        assert!(rendered.contains("Launch failed: port in use"));
    }
}
