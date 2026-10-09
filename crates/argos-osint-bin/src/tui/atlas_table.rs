use ratatui::layout::Rect;
use ratatui::style::Style;
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
        let mut widths = min.to_vec();
        let mut allocated = 0u16;
        for (i, w) in widths.iter_mut().enumerate() {
            let add = if floor > 0 {
                ((extra as u32 * min[i] as u32) / floor as u32) as u16
            } else {
                extra / n as u16
            };
            *w = w.saturating_add(add);
            allocated = allocated.saturating_add(add);
        }
        let mut rem = extra.saturating_sub(allocated);
        for w in widths.iter_mut() {
            if rem == 0 {
                break;
            }
            *w += 1;
            rem -= 1;
        }
        return widths;
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

pub fn border_style() -> Style {
    Style::default().fg(theme::BORDER).bg(theme::BG)
}

pub fn clip_to_width(text: &str, width: usize) -> String {
    let mut res = String::new();
    let mut cur = 0;
    for ch in text.chars() {
        let w = char_width(ch);
        if cur + w > width {
            break;
        }
        res.push(ch);
        cur += w;
    }
    res
}

pub fn format_cell(text: &str, width: usize) -> String {
    let w = display_width(text);
    if w > width {
        let clipped = clip_to_width(text, width);
        let cw = display_width(&clipped);
        if cw < width {
            format!("{clipped}{}", " ".repeat(width - cw))
        } else {
            clipped
        }
    } else if width >= w + 2 {
        let trailing = width - w - 1;
        format!(" {text}{}", " ".repeat(trailing))
    } else if width == w + 1 {
        format!(" {text}")
    } else {
        text.to_string()
    }
}

pub fn top_border_line(widths: &[u16]) -> Line<'static> {
    let mut s = String::new();
    s.push('┌');
    for (i, &w) in widths.iter().enumerate() {
        s.push_str(&"─".repeat(w as usize));
        if i + 1 < widths.len() {
            s.push('┬');
        }
    }
    s.push('┐');
    Line::from(Span::styled(s, border_style()))
}

pub fn sep_border_line(widths: &[u16]) -> Line<'static> {
    let mut s = String::new();
    s.push('├');
    for (i, &w) in widths.iter().enumerate() {
        s.push_str(&"─".repeat(w as usize));
        if i + 1 < widths.len() {
            s.push('┼');
        }
    }
    s.push('┤');
    Line::from(Span::styled(s, border_style()))
}

pub fn bottom_border_line(widths: &[u16]) -> Line<'static> {
    let mut s = String::new();
    s.push('└');
    for (i, &w) in widths.iter().enumerate() {
        s.push_str(&"─".repeat(w as usize));
        if i + 1 < widths.len() {
            s.push('┴');
        }
    }
    s.push('┘');
    Line::from(Span::styled(s, border_style()))
}

pub fn header_line(headers: &[&str], widths: &[u16]) -> Line<'static> {
    let mut spans = Vec::new();
    spans.push(Span::styled("│", border_style()));
    for (i, &w) in widths.iter().enumerate() {
        let header = headers.get(i).copied().unwrap_or("");
        spans.push(Span::styled(format_cell(header, w as usize), theme::dim()));
        spans.push(Span::styled("│", border_style()));
    }
    Line::from(spans)
}

pub fn render_table_lines<R: AsRef<[String]>>(
    headers: &[&str],
    rows: &[R],
    widths: &[u16],
    default_w: usize,
) -> Vec<Line<'static>> {
    if widths.is_empty() || headers.is_empty() {
        return Vec::new();
    }
    let mut rendered: Vec<Line<'static>> = Vec::new();
    rendered.push(top_border_line(widths));
    rendered.push(header_line(headers, widths));
    if rows.is_empty() {
        rendered.push(bottom_border_line(widths));
        return rendered;
    }
    rendered.push(sep_border_line(widths));
    for row in rows {
        let cells = row.as_ref();
        let wrapped_cols: Vec<Vec<String>> = cells
            .iter()
            .enumerate()
            .map(|(i, cell)| {
                wrap_cell(
                    cell,
                    widths.get(i).copied().unwrap_or(default_w as u16) as usize,
                )
            })
            .collect();
        let height = wrapped_cols.iter().map(|c| c.len()).max().unwrap_or(1);
        for line_i in 0..height {
            let mut spans = Vec::new();
            spans.push(Span::styled("│", border_style()));
            for (i, &w) in widths.iter().enumerate() {
                let text = wrapped_cols
                    .get(i)
                    .and_then(|col| col.get(line_i))
                    .map(String::as_str)
                    .unwrap_or("");
                spans.push(Span::styled(format_cell(text, w as usize), theme::text()));
                spans.push(Span::styled("│", border_style()));
            }
            rendered.push(Line::from(spans));
        }
    }
    rendered.push(bottom_border_line(widths));
    rendered
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
    if inner.width <= 2 || inner.height == 0 {
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
    let avail = inner.width.saturating_sub(2);
    let widths = column_widths(avail, min_widths, false);
    if widths.is_empty() {
        return;
    }
    let rendered = render_table_lines(headers, rows, &widths, 8);
    let skip = scroll_lines.min(rendered.len().saturating_sub(1));
    let visible: Vec<Line<'static>> = rendered.into_iter().skip(skip).collect();
    frame.render_widget(Paragraph::new(visible).wrap(Wrap { trim: false }), inner);
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
    if inner.width <= 2 || inner.height == 0 {
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
    let avail = inner.width.saturating_sub(2);
    let widths = column_widths(avail, &min, false);
    if widths.is_empty() {
        return;
    }
    let headers = ["Country", "Tier", "Temp", "Volume", "Articles", "Share"];
    let rows: Vec<[String; 6]> = origins
        .iter()
        .map(|origin| {
            [
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
            ]
        })
        .collect();
    let rendered = render_table_lines(&headers, &rows, &widths, 4);
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

    #[test]
    fn columns_distribute_proportionally_when_not_flex_last() {
        let widths = column_widths(40, &[8, 6, 6], false);
        assert_eq!(widths.iter().sum::<u16>() + 2, 40);
        assert!(widths[0] > 8);
        assert!(widths[1] > 6);
        assert!(widths[2] > 6);
    }

    #[test]
    fn format_cell_pads_and_fits() {
        assert_eq!(format_cell("Test", 8), " Test   ");
        assert_eq!(format_cell("Test", 4), "Test");
        assert_eq!(format_cell("Test", 5), " Test");
        assert_eq!(format_cell("VeryLongText", 5), "VeryL");
        assert_eq!(format_cell("", 4), "    ");
    }

    #[test]
    fn borders_and_cells_have_matching_display_widths() {
        let widths = [12_u16, 6, 8];
        let headers = ["Col1", "Col2", "Col3"];
        let rows = [vec![
            "Val1".to_string(),
            "Val2".to_string(),
            "Val3".to_string(),
        ]];
        let lines = render_table_lines(&headers, &rows, &widths, 6);
        let expected_w = 12 + 6 + 8 + 4; // 30
        for line in &lines {
            let actual_w: usize = line.spans.iter().map(|s| display_width(&s.content)).sum();
            assert_eq!(actual_w, expected_w);
        }
        let top = &lines[0].spans[0].content;
        assert!(top.starts_with('┌') && top.ends_with('┐') && top.contains('┬'));
        let sep = &lines[2].spans[0].content;
        assert!(sep.starts_with('├') && sep.ends_with('┤') && sep.contains('┼'));
        let bot = &lines.last().unwrap().spans[0].content;
        assert!(bot.starts_with('└') && bot.ends_with('┘') && bot.contains('┴'));
    }

    #[test]
    fn empty_table_renders_top_header_and_bottom_border() {
        let widths = [10_u16, 10];
        let headers = ["H1", "H2"];
        let rows: [Vec<String>; 0] = [];
        let lines = render_table_lines(&headers, &rows, &widths, 6);
        assert_eq!(lines.len(), 3); // top, header, bottom
        assert!(lines[0].spans[0].content.starts_with('┌'));
        assert_eq!(lines[1].spans[0].content, "│");
        assert!(lines[2].spans[0].content.starts_with('└'));
    }
}
