use super::theme;
use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
};
pub(crate) const LOGO: [&str; 6] = [
    " █████╗  ██████╗   ██████╗   ██████╗  ███████╗    ██████╗  ███████╗ ██╗ ███╗   ██╗ ████████╗",
    "██╔══██╗ ██╔══██╗ ██╔════╝  ██╔═══██╗ ██╔════╝   ██╔═══██╗ ██╔════╝ ██║ ████╗  ██║ ╚══██╔══╝",
    "███████║ ██████╔╝ ██║  ███╗ ██║   ██║ ███████╗   ██║   ██║ ███████╗ ██║ ██╔██╗ ██║    ██║",
    "██╔══██║ ██╔══██╗ ██║   ██║ ██║   ██║ ╚════██║   ██║   ██║ ╚════██║ ██║ ██║╚██╗██║    ██║",
    "██║  ██║ ██║  ██║ ╚██████╔╝ ╚██████╔╝ ███████║   ╚██████╔╝ ███████║ ██║ ██║ ╚████║    ██║",
    "╚═╝  ╚═╝ ╚═╝  ╚═╝  ╚═════╝   ╚═════╝  ╚══════╝    ╚═════╝  ╚══════╝ ╚═╝ ╚═╝  ╚═══╝    ╚═╝",
];
pub(crate) const LOGO_SPLIT: [usize; 6] = [50, 49, 49, 49, 49, 50];

pub(crate) fn paint_logo(pattern: &str, split: usize, bottom: bool) -> Vec<Span<'static>> {
    pattern
        .chars()
        .enumerate()
        .map(|(index, ch)| {
            let bright = index >= split;
            let (face, shade) = if bright {
                (theme::TEXT, Color::Rgb(78, 108, 120))
            } else {
                (theme::DIM, Color::Rgb(42, 64, 76))
            };
            let shade_stroke = bottom && ch != ' ' || matches!(ch, '╗' | '║' | '╝');
            if ch == ' ' {
                Span::styled(" ", Style::default().bg(theme::BG))
            } else if shade_stroke {
                Span::styled(ch.to_string(), Style::default().fg(shade).bg(theme::BG))
            } else {
                Span::styled(ch.to_string(), Style::default().fg(face).bg(theme::BG))
            }
        })
        .collect()
}

/// The launcher retains the core ARGOS glyphs and stroke palette.
pub(crate) fn venture_rows() -> Vec<Line<'static>> {
    let glyphs = [
        [
            "██╗   ██╗",
            "██║   ██║",
            "██║   ██║",
            "╚██╗ ██╔╝",
            " ╚████╔╝",
            "  ╚═══╝",
        ],
        [
            "███████╗",
            "██╔════╝",
            "█████╗",
            "██╔══╝",
            "███████╗",
            "╚══════╝",
        ],
        [
            "███╗   ██╗",
            "████╗  ██║",
            "██╔██╗ ██║",
            "██║╚██╗██║",
            "██║ ╚████║",
            "╚═╝  ╚═══╝",
        ],
        [
            "████████╗",
            "╚══██╔══╝",
            "   ██║",
            "   ██║",
            "   ██║",
            "   ╚═╝",
        ],
        [
            "██╗   ██╗",
            "██║   ██║",
            "██║   ██║",
            "██║   ██║",
            "╚██████╔╝",
            " ╚═════╝",
        ],
        [
            "██████╗",
            "██╔══██╗",
            "██████╔╝",
            "██╔══██╗",
            "██║  ██║",
            "╚═╝  ╚═╝",
        ],
        [
            "███████╗",
            "██╔════╝",
            "█████╗",
            "██╔══╝",
            "███████╗",
            "╚══════╝",
        ],
    ];
    (0..6)
        .map(|row| {
            let mut pattern: String = LOGO[row].chars().take(LOGO_SPLIT[row]).collect();
            // Core split includes the gutter before the second word.
            while pattern.chars().count() < 50 {
                pattern.push(' ');
            }
            for glyph in &glyphs {
                let width = glyph.iter().map(|s| s.chars().count()).max().unwrap();
                pattern.push_str(glyph[row]);
                pattern.extend(std::iter::repeat_n(
                    ' ',
                    width - glyph[row].chars().count() + 1,
                ));
            }
            Line::from(paint_logo(&pattern, 50, row == 5))
        })
        .collect()
}
