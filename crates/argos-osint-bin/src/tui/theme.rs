use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, BorderType, Borders};

pub const BG: Color = Color::Rgb(6, 14, 22);
pub const BORDER: Color = Color::Rgb(28, 110, 140);
pub const ACCENT: Color = Color::Rgb(72, 214, 196);
pub const GREEN: Color = Color::Rgb(64, 210, 130);
pub const TEXT: Color = Color::Rgb(214, 232, 236);
pub const DIM: Color = Color::Rgb(120, 150, 162);
pub const SELECT: Color = Color::Rgb(14, 92, 72);
pub const WARN: Color = Color::Rgb(230, 186, 72);
pub const RED: Color = Color::Rgb(220, 90, 90);

pub fn panel(title: &str) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(BORDER))
        .title(title.to_string())
        .title_style(Style::default().fg(ACCENT).add_modifier(Modifier::BOLD))
        .style(Style::default().bg(BG).fg(TEXT))
}

pub fn text() -> Style {
    Style::default().fg(TEXT).bg(BG)
}

pub fn dim() -> Style {
    Style::default().fg(DIM).bg(BG)
}

pub fn accent() -> Style {
    Style::default().fg(ACCENT).bg(BG)
}

/// User turn, close to the Grok Build prompt block.
pub fn user_message() -> Style {
    Style::default().fg(TEXT).bg(Color::Rgb(12, 36, 44))
}

pub fn selected() -> Style {
    Style::default()
        .fg(GREEN)
        .bg(SELECT)
        .add_modifier(Modifier::BOLD)
}
