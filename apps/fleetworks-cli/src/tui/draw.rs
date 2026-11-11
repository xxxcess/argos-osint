//! Rendering for the shared shell. Reads view snapshots; performs no I/O.

use ratatui::layout::Alignment;
use ratatui::layout::Constraint;
use ratatui::layout::Layout;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Block;
use ratatui::widgets::BorderType;
use ratatui::widgets::Borders;
use ratatui::widgets::Clear;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Wrap;
use ratatui::Frame;
use unicode_width::UnicodeWidthChar;
use unicode_width::UnicodeWidthStr;

use super::shell::sanitize;
use super::shell::Pane;
use super::shell::PromptKind;
use super::shell::Shell;
use super::View;

// Semantic tokens shared with the Argos terminal theme.
const BG: Color = Color::Rgb(20, 20, 20);
const SURFACE: Color = Color::Rgb(30, 30, 30);
const BORDER: Color = Color::Rgb(83, 83, 83);
const ACCENT: Color = Color::Rgb(135, 191, 255);
const TEXT: Color = Color::Rgb(232, 232, 232);
const DIM: Color = Color::Rgb(181, 181, 181);
const MUTED: Color = Color::Rgb(145, 145, 145);
const SELECT: Color = Color::Rgb(48, 48, 48);
const WARN: Color = Color::Rgb(229, 192, 123);
const RED: Color = Color::Rgb(240, 138, 138);

pub(super) const MIN_WIDTH: u16 = 60;
pub(super) const MIN_HEIGHT: u16 = 18;
/// Side-by-side panes from this width; below it one pane shows at a time.
pub(super) const WIDE: u16 = 100;

fn base() -> Style {
    Style::default().fg(TEXT).bg(BG)
}

fn panel(title: String, focused: bool) -> Block<'static> {
    let (border, title) = if focused {
        (ACCENT, format!(" {title} • focused "))
    } else {
        (BORDER, format!(" {title} "))
    };
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(Style::default().fg(border).bg(BG))
        .title(title)
        .title_style(Style::default().fg(ACCENT).bg(BG))
        .style(base())
}

pub(super) fn draw(frame: &mut Frame, shell: &mut Shell, view: &dyn View) {
    let area = frame.area();
    frame.render_widget(Block::default().style(base()), area);
    shell.list_area = Rect::default();
    shell.detail_area = Rect::default();
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        let notice = format!(
            "Terminal too small: {}x{}. Resize to at least {MIN_WIDTH}x{MIN_HEIGHT}.",
            area.width, area.height
        );
        frame.render_widget(
            Paragraph::new(notice)
                .style(Style::default().fg(WARN).bg(BG))
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true }),
            area,
        );
        return;
    }
    let [header, body, status, app_hints, hints] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(area);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                " Fleetworks ",
                Style::default()
                    .fg(ACCENT)
                    .bg(BG)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(sanitize(&view.title()), Style::default().fg(DIM).bg(BG)),
        ])),
        header,
    );
    shell.compact = area.width < WIDE;
    let (list, detail) = if shell.compact {
        match shell.focus {
            Pane::List => (Some(body), None),
            Pane::Detail => (None, Some(body)),
        }
    } else {
        let [list, detail] =
            Layout::horizontal([Constraint::Percentage(40), Constraint::Percentage(60)])
                .areas(body);
        (Some(list), Some(detail))
    };
    if let Some(list) = list {
        draw_list(frame, shell, view, list);
    }
    if let Some(detail) = detail {
        draw_detail(frame, shell, view, detail);
    }
    let style = if shell.status.starts_with("Error") {
        Style::default().fg(RED).bg(BG)
    } else {
        Style::default().fg(DIM).bg(BG)
    };
    frame.render_widget(
        Paragraph::new(format!(" {}", clip(&shell.status, area.width.into()))).style(style),
        status,
    );
    let shared = if shell.compact {
        "Tab pane · ↑↓ · Enter · Esc back · r refresh · q quit"
    } else {
        "Tab pane · ↑↓ select · Enter open · PgUp/PgDn scroll · r refresh · q quit"
    };
    frame.render_widget(
        Paragraph::new(clip(&format!(" {}", view.hints()), area.width.into()))
            .style(Style::default().fg(DIM).bg(BG)),
        app_hints,
    );
    let line = format!(" {shared}");
    frame.render_widget(
        Paragraph::new(clip(&line, area.width.into())).style(Style::default().fg(MUTED).bg(BG)),
        hints,
    );
    if shell.prompt.is_some() {
        draw_prompt(frame, shell, area);
    }
}

fn draw_list(frame: &mut Frame, shell: &mut Shell, view: &dyn View, area: Rect) {
    let block = panel(
        view.list_title().to_owned(),
        shell.focus == Pane::List || shell.compact,
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    shell.list_area = area;
    let rows = view.rows();
    if rows.is_empty() {
        frame.render_widget(
            Paragraph::new(" Nothing to show. Press r to refresh.")
                .style(Style::default().fg(MUTED).bg(BG)),
            inner,
        );
        return;
    }
    let stride = if view.dense() { 1 } else { 2 };
    shell.stride = stride;
    let capacity = usize::from(inner.height).div_ceil(stride).max(1);
    if shell.selected < shell.list_offset {
        shell.list_offset = shell.selected;
    } else if shell.selected >= shell.list_offset + capacity {
        shell.list_offset = shell.selected + 1 - capacity;
    }
    let width = usize::from(inner.width);
    let lines: Vec<Line> = rows
        .iter()
        .enumerate()
        .skip(shell.list_offset)
        .take(capacity)
        .flat_map(|(index, row)| {
            let selected = index == shell.selected;
            let style = if selected {
                Style::default()
                    .fg(TEXT)
                    .bg(SELECT)
                    .add_modifier(Modifier::BOLD)
            } else {
                base()
            };
            let text = format!(" {}", clip(&sanitize(row), width.saturating_sub(2)));
            let mut lines = vec![Line::styled(pad(&text, width), style)];
            if stride > 1 {
                lines.push(Line::raw(""));
            }
            lines
        })
        .collect();
    frame.render_widget(Paragraph::new(lines).style(base()), inner);
}

fn draw_detail(frame: &mut Frame, shell: &mut Shell, view: &dyn View, area: Rect) {
    let source = view.detail();
    let mut title = view.detail_title();
    let focused = shell.focus == Pane::Detail || shell.compact;
    let block = panel(String::new(), focused);
    let inner = block.inner(area);
    shell.detail_area = area;
    let width = usize::from(inner.width).saturating_sub(2).max(1);
    let wrapped: Vec<String> = source
        .iter()
        .flat_map(|line| wrap(&sanitize(line), width))
        .collect();
    let height = usize::from(inner.height);
    shell.detail_max = wrapped.len().saturating_sub(height);
    shell.scroll = shell.scroll.min(shell.detail_max);
    if wrapped.len() > height {
        title = format!(
            "{title} {}-{}/{}",
            shell.scroll + 1,
            (shell.scroll + height).min(wrapped.len()),
            wrapped.len()
        );
    }
    frame.render_widget(panel(title, focused), area);
    if wrapped.is_empty() {
        frame.render_widget(
            Paragraph::new(" Select an item and press Enter.")
                .style(Style::default().fg(MUTED).bg(BG)),
            inner,
        );
        return;
    }
    let lines: Vec<Line> = wrapped
        .into_iter()
        .skip(shell.scroll)
        .take(height)
        .map(|line| Line::raw(format!(" {line}")))
        .collect();
    frame.render_widget(Paragraph::new(lines).style(base()), inner);
}

fn draw_prompt(frame: &mut Frame, shell: &Shell, area: Rect) {
    let Some(active) = &shell.prompt else {
        return;
    };
    let width = area.width.saturating_sub(4).min(72);
    let height = 7.min(area.height);
    let popup = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    frame.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(Style::default().fg(ACCENT).bg(SURFACE))
        .title(" Confirm ")
        .title_style(
            Style::default()
                .fg(ACCENT)
                .bg(SURFACE)
                .add_modifier(Modifier::BOLD),
        )
        .style(Style::default().bg(SURFACE).fg(TEXT));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    let (body, hint) = match active.prompt.kind {
        PromptKind::Input => (
            format!(
                "{}\n\n> {}▏",
                sanitize(&active.prompt.label),
                clip_tail(
                    &sanitize(&active.buffer),
                    usize::from(inner.width).saturating_sub(4)
                )
            ),
            "Enter submit · Esc cancel",
        ),
        PromptKind::Confirm => (
            sanitize(&active.prompt.label),
            "y confirm · n or Esc cancel",
        ),
    };
    let [text, hints] = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(inner);
    frame.render_widget(
        Paragraph::new(body)
            .style(Style::default().fg(TEXT).bg(SURFACE))
            .wrap(Wrap { trim: false }),
        text,
    );
    frame.render_widget(
        Paragraph::new(hint).style(Style::default().fg(DIM).bg(SURFACE)),
        hints,
    );
}

/// Truncate to `width` terminal cells, ending with an ellipsis.
pub(super) fn clip(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_owned();
    }
    let mut out = String::new();
    let mut used = 0;
    for character in text.chars() {
        let cells = character.width().unwrap_or(0);
        if used + cells + 1 > width {
            break;
        }
        out.push(character);
        used += cells;
    }
    out.push('…');
    out
}

fn clip_tail(text: &str, width: usize) -> String {
    let mut kept: Vec<char> = Vec::new();
    let mut used = 0;
    for character in text.chars().rev() {
        let cells = character.width().unwrap_or(0);
        if used + cells > width {
            break;
        }
        kept.push(character);
        used += cells;
    }
    kept.into_iter().rev().collect()
}

fn pad(text: &str, width: usize) -> String {
    let used = text.width();
    format!("{text}{}", " ".repeat(width.saturating_sub(used)))
}

/// Wrap on terminal cells, preferring spaces; a blank line stays blank.
pub(super) fn wrap(line: &str, width: usize) -> Vec<String> {
    if line.width() <= width {
        return vec![line.to_owned()];
    }
    let mut rows = Vec::new();
    let mut current = String::new();
    let mut used = 0;
    for character in line.chars() {
        let cells = character.width().unwrap_or(0);
        if used + cells > width {
            match current.rfind(' ') {
                Some(space) if space > 0 && character != ' ' => {
                    let rest = current[space + 1..].to_owned();
                    current.truncate(space);
                    rows.push(std::mem::take(&mut current));
                    used = rest.width();
                    current = rest;
                }
                _ => {
                    rows.push(std::mem::take(&mut current));
                    used = 0;
                }
            }
        }
        if used == 0 && character == ' ' && !rows.is_empty() {
            continue;
        }
        current.push(character);
        used += cells;
    }
    if !current.is_empty() {
        rows.push(current);
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::super::shell::Prompt;
    use super::super::shell::Step;
    use super::super::testing::render;
    use super::super::testing::StaticView;
    use super::*;

    fn text(rows: &[String]) -> String {
        rows.join("\n")
    }

    #[test]
    fn wide_layout_shows_both_panes_with_focus_marker() {
        let view = StaticView::new(3);
        let mut shell = Shell::new();
        let screen = text(&render(&mut shell, &view, 120, 40));
        assert!(screen.contains("Items • focused"), "{screen}");
        assert!(screen.contains("row 1"));
        assert!(screen.contains("detail line 1"));
        assert!(screen.contains("Fleetworks"));
        assert!(!shell.compact);
    }

    #[test]
    fn compact_layout_shows_one_pane_at_a_time() {
        let view = StaticView::new(3);
        let mut shell = Shell::new();
        let screen = text(&render(&mut shell, &view, 80, 24));
        assert!(screen.contains("row 1") && !screen.contains("detail line 1"));
        shell.focus = Pane::Detail;
        let screen = text(&render(&mut shell, &view, 80, 24));
        assert!(screen.contains("detail line 1") && !screen.contains("row 1"));
    }

    #[test]
    fn minimum_size_and_below() {
        let view = StaticView::new(3);
        let mut shell = Shell::new();
        assert!(text(&render(&mut shell, &view, 60, 18)).contains("row 1"));
        let small = text(&render(&mut shell, &view, 59, 18));
        assert!(small.contains("Terminal too small"), "{small}");
    }

    #[test]
    fn spaced_rows_scroll_to_keep_the_selection_visible() {
        let view = StaticView::new(30);
        let mut shell = Shell::new();
        shell.selected = 25;
        let screen = text(&render(&mut shell, &view, 80, 24));
        assert!(screen.contains("row 26"), "{screen}");
        assert!(!screen.contains("row 1 "));
        assert!(shell.list_offset > 0);
    }

    #[test]
    fn dense_rows_use_every_line() {
        let mut view = StaticView::new(30);
        view.dense = true;
        let mut shell = Shell::new();
        let screen = render(&mut shell, &view, 80, 24);
        assert!(screen[2].contains("row 1") && screen[3].contains("row 2"));
    }

    #[test]
    fn detail_scrolls_and_reports_its_range() {
        let view = StaticView::new(1);
        let mut shell = Shell::new();
        render(&mut shell, &view, 120, 24);
        assert!(shell.detail_max > 0);
        shell.scroll = shell.detail_max;
        let screen = text(&render(&mut shell, &view, 120, 24));
        assert!(screen.contains("detail line 40"), "{screen}");
        assert!(screen.contains("/40"));
    }

    #[test]
    fn modal_covers_the_view_and_shows_input() {
        let view = StaticView::new(3);
        let mut shell = Shell::new();
        shell.apply(
            Ok(Step::prompt(Prompt::input("find", "Find an actor", "ab"))),
            3,
        );
        let screen = text(&render(&mut shell, &view, 100, 30));
        assert!(
            screen.contains("Find an actor") && screen.contains("> ab"),
            "{screen}"
        );
        assert!(screen.contains("Enter submit"));
    }

    #[test]
    fn clicks_map_through_the_drawn_row_geometry() {
        use crossterm::event::Event;
        use crossterm::event::KeyModifiers;
        use crossterm::event::MouseEvent;
        use crossterm::event::MouseEventKind;

        use super::super::shell::Action;
        let view = StaticView::new(10);
        let mut shell = Shell::new();
        render(&mut shell, &view, 120, 40);
        let column = shell.list_area.x + 2;
        let click = |row| {
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(crossterm::event::MouseButton::Left),
                column,
                row,
                modifiers: KeyModifiers::NONE,
            })
        };
        // List border is at list_area.y; records start one row below, two rows apart.
        let top = shell.list_area.y;
        let third = click(top + 1 + 4);
        assert_eq!(shell.event(third, 10), Some(Action::Select(2)));
        let gap = click(top + 1 + 1);
        assert_eq!(shell.event(gap, 10), None);
        let border = click(top);
        assert_eq!(shell.event(border, 10), None);
    }

    #[test]
    fn wrapping_counts_cells_and_prefers_spaces() {
        assert_eq!(wrap("alpha beta gamma", 10), ["alpha beta", "gamma"]);
        assert_eq!(wrap("", 10), [""]);
        assert_eq!(wrap("世界世界世界", 6), ["世界世", "界世界"]);
        assert_eq!(clip("abcdef", 4), "abc…");
        assert_eq!(clip("世界世界", 5), "世界…");
    }
}
