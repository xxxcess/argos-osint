//! Shared cell-measured presentation primitives. Components never read storage.
use ratatui::{layout::Rect, text::Line};

pub const LIST_ITEM_GAP_ROWS: usize = 1;

/// Display geometry of complete logical records. Gaps never own a pointer target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListGeometry {
    pub items: Vec<std::ops::Range<usize>>,
    pub extent: usize,
}
impl ListGeometry {
    pub fn new(heights: impl IntoIterator<Item = usize>) -> Self {
        Self::with_gap(heights, LIST_ITEM_GAP_ROWS)
    }
    pub fn with_gap(heights: impl IntoIterator<Item = usize>, gap: usize) -> Self {
        let mut items = Vec::new();
        let mut extent: usize = 0;
        for height in heights {
            if !items.is_empty() {
                extent = extent.saturating_add(gap);
            }
            let end = extent.saturating_add(height.max(1));
            items.push(extent..end);
            extent = end;
        }
        Self { items, extent }
    }
    pub fn fixed(count: usize, height: usize) -> Self {
        Self::new(std::iter::repeat_n(height, count))
    }
    pub fn fixed_with_gap(count: usize, height: usize, gap: usize) -> Self {
        Self::with_gap(std::iter::repeat_n(height, count), gap)
    }
    pub fn capacity(rows: usize, height: usize) -> usize {
        Self::capacity_with_gap(rows, height, LIST_ITEM_GAP_ROWS)
    }
    pub fn capacity_with_gap(rows: usize, height: usize, gap: usize) -> usize {
        rows.saturating_add(gap) / height.max(1).saturating_add(gap)
    }
    pub fn index_at(&self, row: usize) -> Option<usize> {
        let index = self.items.partition_point(|item| item.end <= row);
        self.items
            .get(index)
            .filter(|item| item.contains(&row))
            .map(|_| index)
    }
    pub fn reveal(&self, index: usize, offset: usize, viewport: usize) -> usize {
        let Some(item) = self.items.get(index) else {
            return 0;
        };
        let mut pane = ScrollPane { offset };
        pane.reveal(item.start, item.len(), viewport);
        pane.offset.min(self.extent.saturating_sub(viewport))
    }
}

/// Insert ordinary-background gaps between complete already-measured records.
pub fn spaced<T>(items: impl IntoIterator<Item = T>, mut gap: impl FnMut() -> T) -> Vec<T> {
    let mut out = Vec::new();
    for item in items {
        if !out.is_empty() {
            out.push(gap());
        }
        out.push(item);
    }
    out
}

/// TabBar separates the selected page from keyboard focus at every height.
pub fn tab_button(
    frame: &mut ratatui::Frame,
    area: Rect,
    label: &str,
    active: bool,
    focused: bool,
) {
    use ratatui::{
        layout::Alignment,
        style::Modifier,
        widgets::{Block, Borders, Paragraph},
    };
    let theme = super::theme::text();
    let style = if focused {
        super::theme::selected()
    } else if active {
        super::theme::accent().add_modifier(Modifier::BOLD)
    } else {
        super::theme::dim()
    };
    let label = clip_text(label, area.width.saturating_sub(2) as usize);
    if area.height >= 3 {
        frame.render_widget(
            Paragraph::new(label)
                .alignment(Alignment::Center)
                .style(style)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .style(theme)
                        .border_style(if active {
                            super::theme::accent()
                        } else {
                            super::theme::dim()
                        }),
                ),
            area,
        );
    } else {
        frame.render_widget(
            Paragraph::new(format!("[{label}]"))
                .alignment(Alignment::Center)
                .style(style),
            area,
        );
    }
}

/// Cell-measured tab geometry reveals the chosen button when a row overflows.
/// Empty rectangles are deliberately omitted from the focus registry.
pub fn tab_rects(area: Rect, labels: &[&str], reveal: usize) -> Vec<Rect> {
    let widths: Vec<_> = labels
        .iter()
        .map(|label| text_width(label).saturating_add(3))
        .collect();
    let total: usize = widths.iter().sum();
    if total <= area.width as usize && area.height < 3 {
        let measured: Vec<_> = labels.iter().map(|label| format!("[{label}] ")).collect();
        let labels: Vec<_> = measured.iter().map(String::as_str).collect();
        return action_rects(area, &labels)
            .into_iter()
            .map(|mut rect| {
                rect.width = rect.width.saturating_sub(1);
                rect
            })
            .collect();
    }
    if total <= area.width as usize && area.height >= 3 {
        return (0..labels.len())
            .map(|index| {
                let start = area.width as usize * index / labels.len().max(1);
                let end = area.width as usize * (index + 1) / labels.len().max(1);
                Rect::new(
                    area.x + start as u16,
                    area.y,
                    (end - start) as u16,
                    area.height,
                )
            })
            .collect();
    }
    let end: usize = widths.iter().take(reveal.saturating_add(1)).sum();
    let offset = end.saturating_sub(area.width as usize);
    let mut start = 0;
    widths
        .into_iter()
        .map(|width| {
            let top = start.max(offset);
            let bottom = (start + width).min(offset + area.width as usize);
            start += width;
            if bottom <= top {
                Rect::default()
            } else {
                Rect::new(
                    area.x + (top - offset) as u16,
                    area.y,
                    (bottom - top).saturating_sub(1) as u16,
                    area.height,
                )
            }
        })
        .collect()
}

/// AnalyticsCard owns the plain border and one cell of inner padding.
pub fn analytics_card(frame: &mut ratatui::Frame, area: Rect, title: &str, focused: bool) -> Rect {
    let block = super::theme::panel(title).border_style(if focused {
        super::theme::accent()
    } else {
        ratatui::style::Style::default().fg(super::theme::BORDER)
    });
    let inner = block.inner(area);
    frame.render_widget(block, area);
    Rect::new(
        inner.x + 1,
        inner.y,
        inner.width.saturating_sub(2),
        inner.height,
    )
}

pub fn text_width(text: &str) -> usize {
    use unicode_width::UnicodeWidthStr;
    text.width()
}

pub fn clip_text(text: &str, width: usize) -> String {
    use unicode_segmentation::UnicodeSegmentation;
    if text_width(text) <= width {
        return text.to_owned();
    }
    if width == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for grapheme in text.graphemes(true) {
        let cells = text_width(grapheme);
        if used + cells > width - 1 {
            break;
        }
        out.push_str(grapheme);
        used += cells;
    }
    out.push('…');
    out
}

/// MeasuredEditor uses the same wrapping as its painted input.
pub fn editor_height(text: &str, width: u16, minimum: u16, maximum: u16) -> u16 {
    (measured_lines(vec![Line::raw(text.to_owned())], width)
        .len()
        .saturating_add(1) as u16)
        .clamp(minimum, maximum.max(minimum))
}

/// Typed, revision-cached transcript content. Completed Markdown survives
/// streaming changes in neighbouring blocks and is invalidated by its own text.
#[derive(Clone, Debug)]
pub struct TranscriptBlock<T> {
    pub revision: u64,
    pub width: usize,
    pub content: T,
}

/// Independent logical scrolling; offsets never pass through Paragraph's u16 API.
#[derive(Clone, Debug, Default)]
pub struct ScrollPane {
    pub offset: usize,
}
impl ScrollPane {
    pub fn scroll(&mut self, delta: isize, extent: usize, viewport: usize) {
        self.offset = self
            .offset
            .saturating_add_signed(delta)
            .min(extent.saturating_sub(viewport));
    }
    pub fn reveal(&mut self, start: usize, height: usize, viewport: usize) {
        if height >= viewport {
            self.offset = start;
            return;
        }
        if start < self.offset {
            self.offset = start;
        }
        if start.saturating_add(height) > self.offset.saturating_add(viewport) {
            self.offset = start.saturating_add(height).saturating_sub(viewport);
        }
    }
    pub fn visible<'a>(&self, lines: &'a [Line<'static>], height: usize) -> &'a [Line<'static>] {
        let start = self.offset.min(lines.len());
        &lines[start..start.saturating_add(height).min(lines.len())]
    }
}

/// ActionBar measures the same rectangles its callers draw and register.
pub fn action_rects(area: Rect, labels: &[&str]) -> Vec<Rect> {
    let mut x = area.x;
    labels
        .iter()
        .map(|label| {
            let width = (Line::raw(*label).width() as u16).min(area.right().saturating_sub(x));
            let rect = Rect::new(x, area.y, width, area.height.min(1));
            x = x.saturating_add(width);
            rect
        })
        .collect()
}

/// Measured bordered ActionBar buttons wrap as a unit. Paint, focus and pointer
/// registration consume these same rectangles, including compact layouts.
pub fn wrapped_action_rects(area: Rect, labels: &[&str], row_height: u16) -> Vec<Rect> {
    let row_height = row_height.max(1);
    let mut x = area.x;
    let mut y = area.y;
    labels
        .iter()
        .map(|label| {
            let width = (Line::raw(*label).width() as u16)
                .saturating_add(4)
                .min(area.width);
            if x > area.x && x.saturating_add(width) > area.right() {
                x = area.x;
                y = y.saturating_add(row_height);
            }
            let rect = Rect::new(x, y, width, row_height.min(area.bottom().saturating_sub(y)));
            x = x.saturating_add(width);
            rect
        })
        .collect()
}

/// DetailTable fallback preserves every value in measured labelled records.
/// Ratatui's text wrapper operates on graphemes and display cells.
pub fn detail_records(value: &serde_json::Value, width: u16) -> Vec<Line<'static>> {
    let mut records = Vec::new();
    fn append(value: &serde_json::Value, prefix: &str, out: &mut Vec<Line<'static>>) {
        match value {
            serde_json::Value::Object(fields) => {
                for (key, value) in fields {
                    let label = if prefix.is_empty() {
                        key.clone()
                    } else {
                        format!("{prefix}.{key}")
                    };
                    append(value, &label, out);
                }
            }
            serde_json::Value::Array(rows) => {
                for (index, row) in rows.iter().enumerate() {
                    if index > 0 {
                        out.push(Line::raw(""));
                    }
                    if let Some(tuple) = row.as_array().filter(|tuple| {
                        tuple.len() == 2
                            && tuple[0].is_string()
                            && !tuple[1].is_array()
                            && !tuple[1].is_object()
                    }) {
                        append(
                            &tuple[1],
                            &format!("{prefix}.{}", tuple[0].as_str().unwrap_or("unknown")),
                            out,
                        );
                    } else if row.is_object() {
                        out.push(Line::raw(format!("{prefix} record {}", index + 1)));
                        append(row, prefix, out);
                    } else {
                        append(row, &format!("{prefix}[{}]", index + 1), out);
                    }
                }
            }
            value => out.push(Line::raw(format!(
                "{prefix}: {}",
                match value {
                    serde_json::Value::Null => "N/A".to_owned(),
                    serde_json::Value::String(text) => text.clone(),
                    value if prefix.ends_with(".n") || prefix == "n" => {
                        let n = value.as_u64().unwrap_or(0);
                        if n > 0 && n < argos_osint_core::profile_stats::SMALL_SAMPLE_MIN {
                            format!("{n} (small sample)")
                        } else {
                            value.to_string()
                        }
                    }
                    value => value.to_string(),
                }
            ))),
        }
    }
    append(value, "", &mut records);
    measured_lines(records, width)
}

/// DetailTable keeps numeric columns intact; switches to labelled records if
/// any complete value or header cannot fit. Nested data stays in records.
pub fn detail_table(value: &serde_json::Value, columns: &[&str], width: u16) -> Vec<Line<'static>> {
    let Some(rows) = value.as_array().filter(|rows| !rows.is_empty()) else {
        return detail_records(value, width);
    };
    if columns.is_empty()
        || rows.iter().any(|row| {
            columns
                .iter()
                .any(|key| row[*key].is_array() || row[*key].is_object())
        })
    {
        return detail_records(value, width);
    }
    let cell = |value: &serde_json::Value| match value {
        serde_json::Value::Null => "N/A".to_owned(),
        serde_json::Value::String(text) => text.clone(),
        value => value.to_string(),
    };
    let widths: Vec<usize> = columns
        .iter()
        .map(|key| {
            rows.iter()
                .map(|row| Line::raw(cell(&row[*key])).width())
                .max()
                .unwrap_or(0)
                .max(Line::raw(*key).width())
        })
        .collect();
    if widths.iter().sum::<usize>() + columns.len().saturating_sub(1) * 2 > usize::from(width) {
        return detail_records(value, width);
    }
    let mut out = vec![Line::raw(
        columns
            .iter()
            .zip(&widths)
            .map(|(key, width)| format!("{key:width$}"))
            .collect::<Vec<_>>()
            .join("  "),
    )];
    for row in rows {
        if out.len() > 1 {
            out.push(Line::raw(""));
        }
        let spans = columns
            .iter()
            .zip(&widths)
            .enumerate()
            .flat_map(|(index, (key, width))| {
                let value = &row[*key];
                let text = cell(value);
                let pad = width.saturating_sub(Line::raw(text.as_str()).width());
                let text = if value.is_number() {
                    format!(
                        "{}{text}{}",
                        " ".repeat(pad),
                        if index + 1 < columns.len() { "  " } else { "" }
                    )
                } else {
                    format!(
                        "{text}{}{}",
                        " ".repeat(pad),
                        if index + 1 < columns.len() { "  " } else { "" }
                    )
                };
                [ratatui::text::Span::raw(text)]
            })
            .collect::<Vec<_>>();
        out.push(Line::from(spans));
    }
    out
}

/// Freeze wrapped lines before scrolling so no authoritative trailing value clips.
pub fn measured_lines(lines: Vec<Line<'static>>, width: u16) -> Vec<Line<'static>> {
    use ratatui::text::Span;
    use unicode_segmentation::UnicodeSegmentation;
    use unicode_width::UnicodeWidthStr;
    let width = usize::from(width.max(1));
    let mut out = Vec::new();
    for line in lines {
        let mut spans = Vec::new();
        let mut used = 0;
        for span in line.spans {
            for token in span.content.split_inclusive(char::is_whitespace) {
                let cells = token.trim_end_matches('\n').width();
                if used > 0 && cells <= width && used + cells > width {
                    out.push(Line::from(std::mem::take(&mut spans)));
                    used = 0;
                }
                for grapheme in token.graphemes(true) {
                    let cells = grapheme.width();
                    if grapheme == "\n" || used + cells > width {
                        out.push(Line::from(std::mem::take(&mut spans)));
                        used = 0;
                    }
                    if grapheme != "\n" {
                        spans.push(Span::styled(grapheme.to_owned(), span.style));
                        used += cells;
                    }
                }
            }
        }
        out.push(Line::from(spans));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dense_geometry_keeps_complete_records_and_spaced_defaults() {
        let dense = ListGeometry::with_gap([1, 3, 2], 0);
        assert_eq!(dense.items, vec![0..1, 1..4, 4..6]);
        assert_eq!(dense.extent, 6);
        assert_eq!(dense.index_at(3), Some(1));
        assert_eq!(dense.index_at(4), Some(2));
        assert_eq!(dense.index_at(6), None);
        assert_eq!(dense.reveal(1, 0, 3), 1);
        assert_eq!(dense.reveal(2, 0, 3), 3);
        assert_eq!(ListGeometry::fixed_with_gap(0, 1, 0).extent, 0);
        assert_eq!(ListGeometry::capacity_with_gap(5, 2, 0), 2);
        assert_eq!(ListGeometry::capacity_with_gap(0, 1, 0), 0);
        assert_eq!(ListGeometry::capacity_with_gap(4, 0, 0), 4);
        assert_eq!(ListGeometry::new([1, 3, 2]).items, vec![0..1, 2..5, 6..8]);
        assert_eq!(ListGeometry::fixed(3, 1).extent, 5);
        assert_eq!(ListGeometry::capacity(5, 1), 3);
    }
    #[test]
    fn list_geometry_maps_only_content_and_reveals_titles() {
        let geometry = ListGeometry::new([1, 2, 8]);
        assert_eq!(geometry.items, vec![0..1, 2..4, 5..13]);
        assert_eq!(geometry.extent, 13);
        assert_eq!(geometry.index_at(1), None);
        assert_eq!(geometry.index_at(4), None);
        assert_eq!(geometry.index_at(12), Some(2));
        assert_eq!(geometry.index_at(13), None);
        assert_eq!(geometry.reveal(2, 0, 3), 5);
        assert_eq!(geometry.reveal(1, 5, 3), 2);
        assert_eq!(ListGeometry::fixed(0, 1).extent, 0);
        assert_eq!(ListGeometry::capacity(5, 2), 2);
        assert_eq!(spaced([1, 2], || 0), vec![1, 0, 2]);
    }
    #[test]
    fn scrolls_beyond_u16_and_reveals_whole_items() {
        let mut pane = ScrollPane::default();
        pane.scroll(70_000, 100_000, 20);
        assert_eq!(pane.offset, 70_000);
        pane.reveal(99_980, 20, 20);
        assert_eq!(pane.offset, 99_980);
    }
    #[test]
    fn compact_records_keep_trailing_numeric_values() {
        let lines = detail_records(&serde_json::json!({"long_named_metric":123456789}), 8);
        let text: String = lines.iter().map(ToString::to_string).collect();
        assert!(text.contains("123456789"));
        assert!(lines.iter().all(|line| line.width() <= 8));
    }
}
