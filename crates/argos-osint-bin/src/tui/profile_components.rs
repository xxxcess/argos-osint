//! Profile surfaces: measured cells, immutable data and clipped virtual panels.
use super::{components, profile_layout::PanelRect, theme};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::Modifier,
    text::{Line, Span},
    widgets::{Paragraph, Widget},
    Frame,
};

pub struct Kpi {
    pub label: &'static str,
    pub value: String,
    pub coverage: String,
}

/// Glanceable shape behind a KPI tile. Ratios get a bullet-style gauge
/// coloured by direction; counts and latencies get a per-bucket sparkline of
/// the same filtered period the headline value summarises.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum KpiVisual {
    #[default]
    None,
    /// Share in 0..=1. `higher_is_better` picks the colour ramp.
    Ratio { value: f64, higher_is_better: bool },
    /// Oldest bucket first; `None` marks a bucket without coverage.
    Trend(Vec<Option<u64>>),
}

const SPARK: [&str; 8] = ["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];

/// One-row sparkline. Uncovered buckets render as a dim `·`, never as zero;
/// a long series keeps the newest buckets so the tile shows recent shape.
pub fn sparkline(values: &[Option<u64>], width: usize) -> Vec<Span<'static>> {
    if width == 0 {
        return Vec::new();
    }
    let visible = &values[values.len().saturating_sub(width)..];
    let peak = visible.iter().flatten().copied().max().unwrap_or(0);
    visible
        .iter()
        .map(|value| match value {
            None => Span::styled("·", theme::dim()),
            Some(0) => Span::styled("▁", theme::dim()),
            Some(v) => {
                let level = (*v * 8).div_ceil(peak.max(1)) as usize - 1;
                Span::styled(SPARK[level.min(7)], theme::accent())
            }
        })
        .collect()
}

/// Single-row gauge: filled `█` share, dim `·` remainder and a colour that
/// encodes good/neutral/bad bands so direction reads without the legend.
pub fn gauge(value: f64, higher_is_better: bool, width: usize) -> Vec<Span<'static>> {
    if width == 0 {
        return Vec::new();
    }
    let value = if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let good = if higher_is_better { value } else { 1.0 - value };
    let colour = if good >= 0.8 {
        theme::GREEN
    } else if good >= 0.5 {
        theme::WARN
    } else {
        theme::RED
    };
    let filled = ((value * width as f64).round() as usize).min(width);
    let mut spans = Vec::new();
    if filled > 0 {
        spans.push(Span::styled(
            "█".repeat(filled),
            ratatui::style::Style::default().fg(colour).bg(theme::BG),
        ));
    }
    if filled < width {
        spans.push(Span::styled("·".repeat(width - filled), theme::dim()));
    }
    spans
}

pub fn visual_spans(visual: &KpiVisual, width: usize) -> Vec<Span<'static>> {
    match visual {
        KpiVisual::None => Vec::new(),
        KpiVisual::Ratio {
            value,
            higher_is_better,
        } => gauge(*value, *higher_is_better, width),
        KpiVisual::Trend(values) if values.is_empty() => Vec::new(),
        KpiVisual::Trend(values) => sparkline(values, width),
    }
}

pub fn kpi_with(frame: &mut Frame, area: Rect, metric: &Kpi, visual: &KpiVisual) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    if area.height < 4 {
        let value_width = components::text_width(&metric.value);
        let room = (area.width as usize).saturating_sub(value_width + 2);
        let inline = visual_spans(visual, room.min(12));
        let mut row = vec![Span::styled(
            metric.value.clone(),
            theme::text().add_modifier(Modifier::BOLD),
        )];
        if !inline.is_empty() {
            row.push(Span::raw(" "));
            row.extend(inline);
        }
        row.push(Span::styled(
            format!("  {}", metric.coverage),
            theme::muted(),
        ));
        let lines = vec![
            Line::styled(
                components::clip_text(metric.label, area.width as usize),
                theme::dim(),
            ),
            Line::from(row),
        ];
        frame.render_widget(Paragraph::new(lines), area);
        return;
    }
    let inner = components::analytics_card(frame, area, &format!(" {} ", metric.label), false);
    let value = Span::styled(
        metric.value.clone(),
        theme::text().add_modifier(Modifier::BOLD),
    );
    let mut lines = if inner.height >= 3 {
        // Tall tile: value, then a full-width visual row, then coverage.
        let mut lines = components::measured_lines(vec![Line::from(value)], inner.width);
        let spans = visual_spans(visual, inner.width as usize);
        if !spans.is_empty() {
            lines.push(Line::from(spans));
        }
        lines
    } else {
        // Short tile: the visual rides on the value row when it fits.
        let room = (inner.width as usize)
            .saturating_sub(components::text_width(&metric.value) + 2)
            .min(24);
        let mut row = vec![value];
        let spans = if room >= 4 {
            visual_spans(visual, room)
        } else {
            Vec::new()
        };
        if !spans.is_empty() {
            row.push(Span::raw("  "));
            row.extend(spans);
        }
        vec![Line::from(row)]
    };
    lines.push(Line::styled(
        components::clip_text(&metric.coverage, inner.width as usize),
        theme::dim(),
    ));
    frame.render_widget(Paragraph::new(lines), inner);
}

/// Paint a minimum-height virtual panel and copy only visible rows. Resizing and
/// page scrolling never squeeze its plot or reflow numeric table rows.
pub fn panel(
    frame: &mut Frame,
    geometry: &PanelRect,
    title: &str,
    focused: bool,
    lines: Vec<Line<'static>>,
) {
    let area = Rect::new(0, 0, geometry.rect.width, geometry.height as u16);
    let mut buffer = Buffer::empty(area);
    let title = format!(" {title}{} ", if focused { " • focused" } else { "" });
    let block = theme::panel(&title).border_style(if focused {
        theme::accent()
    } else {
        ratatui::style::Style::default().fg(theme::BORDER)
    });
    let inner = block.inner(area);
    block.render(area, &mut buffer);
    let inner = Rect::new(
        inner.x + 1,
        inner.y,
        inner.width.saturating_sub(2),
        inner.height,
    );
    Paragraph::new(lines).render(inner, &mut buffer);
    for y in 0..geometry.rect.height {
        for x in 0..geometry.rect.width {
            frame.buffer_mut()[(geometry.rect.x + x, geometry.rect.y + y)] =
                buffer[(x, y + geometry.source_offset as u16)].clone();
        }
    }
}

/// Column proportions are shared by headings, data and row hit targets. Every
/// cell is ellipsized independently; full prose belongs in expanded details.
pub fn table_row(
    cells: &[String],
    proportions: &[usize],
    width: usize,
    selected: bool,
) -> Line<'static> {
    let available = width.saturating_sub(cells.len().saturating_sub(1));
    let total: usize = proportions.iter().sum();
    let mut spans = Vec::new();
    let mut cumulative = 0;
    for (i, cell) in cells.iter().enumerate() {
        let start = available * cumulative / total.max(1);
        cumulative += proportions.get(i).copied().unwrap_or(1);
        let length = available * cumulative / total.max(1) - start;
        let text = components::clip_text(cell, length);
        let text = format!(
            "{text}{}{}",
            " ".repeat(length.saturating_sub(components::text_width(&text))),
            if i + 1 < cells.len() { " " } else { "" }
        );
        spans.push(Span::styled(
            text,
            if selected {
                theme::selected()
            } else {
                theme::text()
            },
        ));
    }
    Line::from(spans)
}

pub fn clipped(lines: Vec<Line<'static>>, width: usize) -> Vec<Line<'static>> {
    lines
        .into_iter()
        .map(|line| {
            let mut remaining = width;
            let mut spans = Vec::new();
            for span in line.spans {
                if remaining == 0 {
                    break;
                }
                let text = components::clip_text(&span.content, remaining);
                remaining = remaining.saturating_sub(components::text_width(&text));
                spans.push(Span::styled(text, span.style));
            }
            Line::from(spans)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(spans: &[Span<'_>]) -> String {
        spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn sparkline_scales_to_peak_and_marks_missing_coverage() {
        let line = text(&sparkline(&[Some(0), Some(4), None, Some(8)], 10));
        assert_eq!(line, "▁▄·█");
    }

    #[test]
    fn sparkline_keeps_the_newest_buckets_when_narrow() {
        let line = text(&sparkline(&[Some(8), Some(1), Some(1), Some(1)], 2));
        assert_eq!(line.chars().count(), 2);
        assert_eq!(line, "██");
    }

    #[test]
    fn gauge_fills_exactly_its_width() {
        for value in [0.0, 0.33, 0.5, 1.0, 7.0, f64::NAN] {
            let line = text(&gauge(value, true, 10));
            assert_eq!(line.chars().count(), 10, "{value}");
        }
        assert_eq!(text(&gauge(0.5, true, 10)), "█████·····");
    }

    #[test]
    fn gauge_colour_follows_direction() {
        let good = gauge(0.9, true, 4)[0].style.fg;
        let bad = gauge(0.9, false, 4)[0].style.fg;
        assert_eq!(good, Some(theme::GREEN));
        assert_eq!(bad, Some(theme::RED));
    }
}
