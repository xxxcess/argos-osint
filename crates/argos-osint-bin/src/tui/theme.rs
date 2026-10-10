use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, BorderType, Borders};

// Semantic colors shared by every app. The aliases keep existing widgets on one
// palette while their layouts are migrated to the quieter shell.
pub const BG: Color = Color::Rgb(20, 20, 20);
pub const SURFACE: Color = Color::Rgb(30, 30, 30);
pub const BORDER: Color = Color::Rgb(83, 83, 83);
pub const ACCENT: Color = Color::Rgb(135, 191, 255);
pub const GREEN: Color = Color::Rgb(152, 195, 121);
pub const TEXT: Color = Color::Rgb(232, 232, 232);
pub const DIM: Color = Color::Rgb(181, 181, 181);
pub const MUTED: Color = Color::Rgb(145, 145, 145);
pub const SELECT: Color = Color::Rgb(48, 48, 48);
pub const WARN: Color = Color::Rgb(229, 192, 123);
pub const RED: Color = Color::Rgb(240, 138, 138);
/// Raised band behind a user prompt, the same role as Grok Build's `bg_light` prompt band.
pub const USER_BAND: Color = SURFACE;
/// Fenced code background, quieter than the user band.
pub const CODE_BG: Color = Color::Rgb(37, 37, 37);

pub fn panel(title: &str) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(Style::default().fg(BORDER))
        .title(title.to_string())
        .title_style(Style::default().fg(ACCENT))
        .style(Style::default().bg(BG).fg(TEXT))
}

/// Solid raised card. Background is part of the style so a popup covers the view under it.
pub fn card(title: &str) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(Style::default().fg(ACCENT).bg(SURFACE))
        .title(title.to_string())
        .title_style(
            Style::default()
                .fg(ACCENT)
                .bg(SURFACE)
                .add_modifier(Modifier::BOLD),
        )
        .style(Style::default().bg(SURFACE).fg(TEXT))
}

pub fn text() -> Style {
    Style::default().fg(TEXT).bg(BG)
}

pub fn dim() -> Style {
    Style::default().fg(DIM).bg(BG)
}

pub fn muted() -> Style {
    Style::default().fg(MUTED).bg(BG)
}

pub fn accent() -> Style {
    Style::default().fg(ACCENT).bg(BG)
}

/// User turn, close to the Grok Build prompt block.
pub fn user_message() -> Style {
    Style::default().fg(TEXT).bg(USER_BAND)
}

pub fn selected() -> Style {
    Style::default()
        .fg(TEXT)
        .bg(SELECT)
        .add_modifier(Modifier::BOLD)
}

pub fn warn() -> Style {
    Style::default().fg(WARN).bg(BG)
}

pub fn error() -> Style {
    Style::default().fg(RED).bg(BG)
}

pub fn card_text() -> Style {
    Style::default().fg(TEXT).bg(SURFACE)
}

pub fn card_dim() -> Style {
    Style::default().fg(DIM).bg(SURFACE)
}

pub fn card_accent() -> Style {
    Style::default().fg(ACCENT).bg(SURFACE)
}

/// Categorical series palette for the Profile dashboard charts. Eight distinct
/// hues ordered around the colour wheel, so adjacent series stay separable on a
/// 256-colour and on a true-colour terminal. Entries 1, 3, 4 and 6 reuse the
/// semantic ACCENT, GREEN, WARN and RED hues, so a chart, its legend and the
/// rest of the shell agree.
pub const SERIES: [Color; 8] = [
    ACCENT,                    // sky
    Color::Rgb(80, 200, 200),  // teal
    GREEN,                     // green
    WARN,                      // amber
    Color::Rgb(232, 152, 72),  // orange
    RED,                       // red
    Color::Rgb(198, 120, 221), // violet
    Color::Rgb(245, 169, 197), // pink
];

/// Series colour by index, wrapping for more than eight series.
pub fn series(index: usize) -> Color {
    SERIES[index % SERIES.len()]
}

/// Style for one series segment: the series hue on the app background.
pub fn series_style(index: usize) -> Style {
    Style::default().fg(series(index)).bg(BG)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn series_palette_is_eight_distinct_hues() {
        assert_eq!(SERIES.len(), 8);
        for (a, first) in SERIES.iter().enumerate() {
            for (b, second) in SERIES.iter().enumerate().skip(a + 1) {
                assert_ne!(first, second, "SERIES[{a}] repeats SERIES[{b}]");
            }
        }
    }

    #[test]
    fn series_index_wraps_after_eight() {
        assert_eq!(series(0), SERIES[0]);
        assert_eq!(series(7), SERIES[7]);
        assert_eq!(series(8), series(0));
        assert_eq!(series(9), series(1));
        assert_eq!(series_style(8).fg, Some(SERIES[0]));
        assert_eq!(series_style(8).bg, Some(BG));
    }
}
