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

pub fn kpi(frame: &mut Frame, area: Rect, metric: &Kpi) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    if area.height < 4 {
        let lines = vec![
            Line::styled(
                components::clip_text(metric.label, area.width as usize),
                theme::dim(),
            ),
            Line::from(vec![
                Span::styled(
                    metric.value.clone(),
                    theme::text().add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!("  {}", metric.coverage), theme::muted()),
            ]),
        ];
        frame.render_widget(Paragraph::new(lines), area);
        return;
    }
    let inner = components::analytics_card(frame, area, &format!(" {} ", metric.label), false);
    let mut lines = components::measured_lines(
        vec![Line::styled(
            metric.value.clone(),
            theme::text().add_modifier(Modifier::BOLD),
        )],
        inner.width,
    );
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
