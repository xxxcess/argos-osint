use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

use argos_osint_core::atlas;

use super::theme;
use super::ui::{inset, pane};

pub struct OriginsView<'a> {
    pub area: Rect,
    pub origins: &'a [&'a atlas::OriginStat],
    pub stats: &'a atlas::RunStats,
    pub scroll: usize,
    pub title: &'a str,
    pub empty: &'a str,
    pub prefix: Vec<Line<'static>>,
    pub focused: bool,
}

/// Wrap `text` to `width` columns. Newlines, graphemes, and unbroken tokens
/// (URLs/IDs) stay readable; nothing is clipped.
pub fn wrap_cell(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        if paragraph.is_empty() {
            lines.push(String::new());
            continue;
        }
        let mut current = String::new();
        let mut current_w = 0usize;
        for ch in paragraph.chars() {
            let w = char_width(ch);
            if current_w + w > width && !current.is_empty() {
                lines.push(std::mem::take(&mut current));
                current_w = 0;
            }
            current.push(ch);
            current_w += w;
        }
        if !current.is_empty() || lines.is_empty() {
            lines.push(current);
        }
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn char_width(ch: char) -> usize {
    match ch {
        '\t' => 2,
        c if c.is_control() => 0,
        c if (c as u32) >= 0x1100 && east_asianish(c) => 2,
        _ => 1,
    }
}

fn east_asianish(ch: char) -> bool {
    matches!(ch as u32,
        0x1100..=0x115F
            | 0x2329..=0x232A
            | 0x2E80..=0xA4CF
            | 0xAC00..=0xD7A3
            | 0xF900..=0xFAFF
            | 0xFE10..=0xFE19
            | 0xFE30..=0xFE6F
            | 0xFF00..=0xFF60
            | 0xFFE0..=0xFFE6
            | 0x1F300..=0x1FAFF
    )
}

pub fn display_width(text: &str) -> usize {
    text.chars().map(char_width).sum()
}

/// Allocate `inner_width` across `weights` so explicit columns plus separators
/// consume the full inner width. The last weight is the flexible prose column
/// when `flex_last` is true.
pub fn column_widths(inner_width: u16, min: &[u16], flex_last: bool) -> Vec<u16> {
    let n = min.len();
    if n == 0 {
        return Vec::new();
    }
    let seps = n.saturating_sub(1) as u16;
    let inner = inner_width.saturating_sub(seps);
    let floor: u16 = min.iter().sum();
    if inner <= floor {
        let mut widths = min.to_vec();
        let used: u16 = widths.iter().sum();
        if used < inner {
            if let Some(last) = widths.last_mut() {
                *last = last.saturating_add(inner - used);
            }
        }
        return widths;
    }
    let extra = inner - floor;
    let mut widths = min.to_vec();
    if flex_last {
        if let Some(last) = widths.last_mut() {
            *last = last.saturating_add(extra);
        }
    } else {
        let share = extra / n as u16;
        let rem = extra % n as u16;
        for (i, w) in widths.iter_mut().enumerate() {
            *w = w.saturating_add(share);
            if (i as u16) < rem {
                *w += 1;
            }
        }
    }
    widths
}

pub struct WrappedTable<'a> {
    pub area: Rect,
    pub headers: &'a [&'a str],
    pub rows: &'a [Vec<String>],
    pub min_widths: &'a [u16],
    pub scroll_lines: usize,
    pub focused: bool,
    pub title: &'a str,
    pub prefix: Vec<Line<'static>>,
}

pub fn render_wrapped_table(frame: &mut Frame, table: WrappedTable<'_>) {
    let WrappedTable {
        area,
        headers,
        rows,
        min_widths,
        scroll_lines,
        focused,
        title,
        prefix,
    } = table;
    let title = if focused {
        format!("{title} · focused")
    } else {
        title.to_string()
    };
    frame.render_widget(pane(&title), area);
    let mut inner = inset(area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    if !prefix.is_empty() {
        let prefix_h = (prefix.len() as u16).min(inner.height);
        frame.render_widget(
            Paragraph::new(prefix),
            Rect {
                height: prefix_h,
                ..inner
            },
        );
        inner.y = inner.y.saturating_add(prefix_h);
        inner.height = inner.height.saturating_sub(prefix_h);
        if inner.height == 0 {
            return;
        }
    }
    let widths = column_widths(inner.width, min_widths, true);
    let mut rendered: Vec<Line<'static>> = Vec::new();
    rendered.push(header_line(headers, &widths));
    for row in rows {
        let wrapped_cols: Vec<Vec<String>> = row
            .iter()
            .enumerate()
            .map(|(i, cell)| wrap_cell(cell, widths.get(i).copied().unwrap_or(8) as usize))
            .collect();
        let height = wrapped_cols.iter().map(|c| c.len()).max().unwrap_or(1);
        for line_i in 0..height {
            let mut spans = Vec::new();
            for (i, col) in wrapped_cols.iter().enumerate() {
                let w = widths.get(i).copied().unwrap_or(8) as usize;
                let text = col.get(line_i).map(String::as_str).unwrap_or("");
                let padded = pad_left(text, w);
                spans.push(Span::styled(padded, theme::text()));
                if i + 1 < wrapped_cols.len() {
                    spans.push(Span::raw(" "));
                }
            }
            rendered.push(Line::from(spans));
        }
    }
    let skip = scroll_lines.min(rendered.len().saturating_sub(1));
    let visible: Vec<Line<'static>> = rendered.into_iter().skip(skip).collect();
    frame.render_widget(Paragraph::new(visible).wrap(Wrap { trim: false }), inner);
}

fn header_line(headers: &[&str], widths: &[u16]) -> Line<'static> {
    let mut spans = Vec::new();
    for (i, header) in headers.iter().enumerate() {
        let w = widths.get(i).copied().unwrap_or(8) as usize;
        spans.push(Span::styled(pad_left(header, w), theme::dim()));
        if i + 1 < headers.len() {
            spans.push(Span::raw(" "));
        }
    }
    Line::from(spans)
}

fn pad_left(text: &str, width: usize) -> String {
    let w = display_width(text);
    if w >= width {
        text.to_string()
    } else {
        format!("{text}{}", " ".repeat(width - w))
    }
}

pub fn draw_origins(frame: &mut Frame, view: OriginsView<'_>) {
    let OriginsView {
        area,
        origins,
        stats,
        scroll,
        title,
        empty,
        prefix,
        focused,
    } = view;
    let title = if focused {
        format!("{title} · focused")
    } else {
        title.to_string()
    };
    frame.render_widget(pane(&title), area);
    let mut inner = inset(area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    if !prefix.is_empty() {
        let prefix_h = (prefix.len() as u16).min(inner.height);
        frame.render_widget(
            Paragraph::new(prefix),
            Rect {
                height: prefix_h,
                ..inner
            },
        );
        inner.y = inner.y.saturating_add(prefix_h);
        inner.height = inner.height.saturating_sub(prefix_h);
    }
    if inner.height == 0 {
        return;
    }
    if origins.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(empty.to_string(), theme::dim()))),
            inner,
        );
        return;
    }

    let min = [12_u16, 4, 5, 6, 8, 5];
    let widths = column_widths(inner.width, &min, true);
    let headers = ["Country", "Tier", "Temp", "Volume", "Articles", "Share"];
    let mut rendered: Vec<Line<'static>> = vec![header_line(&headers, &widths)];
    for origin in origins {
        let cells = [
            atlas::country_label(&origin.country),
            if origin.tier == 0 {
                "-".into()
            } else {
                origin.tier.to_string()
            },
            format!("{:.2}", origin.temperature),
            origin.volume.to_string(),
            origin.articles.to_string(),
            format!("{:.0}%", stats.share(origin.articles) * 100.0),
        ];
        let wrapped_cols: Vec<Vec<String>> = cells
            .iter()
            .enumerate()
            .map(|(i, cell)| wrap_cell(cell, widths.get(i).copied().unwrap_or(4) as usize))
            .collect();
        let height = wrapped_cols.iter().map(|c| c.len()).max().unwrap_or(1);
        for line_i in 0..height {
            let mut spans = Vec::new();
            for (i, col) in wrapped_cols.iter().enumerate() {
                let w = widths.get(i).copied().unwrap_or(4) as usize;
                let text = col.get(line_i).map(String::as_str).unwrap_or("");
                spans.push(Span::styled(pad_left(text, w), theme::text()));
                if i + 1 < wrapped_cols.len() {
                    spans.push(Span::raw(" "));
                }
            }
            rendered.push(Line::from(spans));
        }
    }
    let skip = scroll.min(rendered.len().saturating_sub(1));
    let visible: Vec<Line<'static>> = rendered.into_iter().skip(skip).collect();
    frame.render_widget(Paragraph::new(visible).wrap(Wrap { trim: false }), inner);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_keeps_long_urls_and_unicode() {
        let url = "https://example.com/very/long/path/without/spaces/id-12345";
        let lines = wrap_cell(url, 12);
        assert!(lines.len() > 2);
        assert!(lines.concat().contains("id-12345"));
        let uni = wrap_cell("東京ニュース", 4);
        assert!(uni.len() >= 2);
        let multi = wrap_cell("one\ntwo", 8);
        assert_eq!(multi, vec!["one".to_string(), "two".to_string()]);
    }

    #[test]
    fn columns_consume_inner_width() {
        let widths = column_widths(40, &[8, 6, 6], true);
        assert_eq!(widths.iter().sum::<u16>() + 2, 40);
        assert!(widths[2] > 6);
    }
}
