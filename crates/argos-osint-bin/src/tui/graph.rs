//! Claim/Recon detail for one memory: path graph above, Related left, Summary right.

use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

use argos_osint_core::recon::{recon_path, GraphNodeKind, MemoryGraph};

use super::app::{App, Target};
use super::brain_detail;
use super::markdown;
use super::summary_card;
use super::theme::{self, panel};
use super::ui::{center_line, contains};

pub struct PathLine {
    pub text: String,
    pub article_id: String,
    pub run_id: String,
}

/// Graph above, Related (left) and Summary (right) below; stacked when narrow.
pub fn draw(frame: &mut Frame, app: &App, area: Rect) {
    let areas = brain_detail::areas(area, brain_detail::pane_of(app.focus));
    let claim = app.detail_claim();
    brain_detail::draw_nav(frame, app, &areas, claim);
    draw_path(frame, app, areas.path, claim);
    brain_detail::draw_related(frame, app, areas.related);
    draw_summary(frame, app, areas.summary, claim);
}

/// Scrollable path lines, above the anchored legend. `None` when the click misses them.
pub fn path_line_at(app: &App, path: Rect, x: u16, y: u16, scroll: u16) -> Option<usize> {
    let content = path_content(path, legend_height(app, path));
    if !contains(content, x, y) {
        return None;
    }
    Some(scroll as usize + (y - content.y) as usize)
}

fn path_content(path: Rect, legend: u16) -> Rect {
    let inner = inset(path);
    Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width,
        height: inner.height.saturating_sub(legend),
    }
}

/// Legend rows that fit the path pane: one row when it fits, otherwise the
/// entries wrap (at most three rows, never more than half the pane).
fn legend_rows(app: &App, claim: bool, width: usize) -> Vec<String> {
    let parts = legend_parts(app, claim);
    if parts.is_empty() || width == 0 {
        return Vec::new();
    }
    let joined = parts.join("   ");
    if joined.chars().count() <= width {
        return vec![joined];
    }
    let mut rows: Vec<String> = Vec::new();
    for part in parts {
        match rows.last_mut() {
            Some(row) if row.chars().count() + 2 + part.chars().count() <= width => {
                row.push_str("  ");
                row.push_str(&part);
            }
            _ => rows.push(part),
        }
    }
    rows
}

fn legend_height(app: &App, path: Rect) -> u16 {
    let inner = inset(path);
    let rows = legend_rows(app, app.detail_claim(), inner.width as usize).len() as u16;
    rows.min(3).min((inner.height / 2).max(1)).min(inner.height)
}

fn inset(area: Rect) -> Rect {
    Rect {
        x: area.x.saturating_add(1),
        y: area.y.saturating_add(1),
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    }
}

/// Only the current memory's path graph; Related lives in its own section.
fn draw_path(frame: &mut Frame, app: &App, area: Rect, claim: bool) {
    let width = area.width.saturating_sub(2) as usize;
    let lines = if app.brain_graph.is_empty() {
        vec![PathLine {
            text: if claim {
                "This memory has no claim path.".into()
            } else {
                "This memory has no investigation graph.".into()
            },
            article_id: String::new(),
            run_id: String::new(),
        }]
    } else {
        path_lines(&app.brain_graph)
    };
    let widest = lines
        .iter()
        .map(|line| line.text.chars().count())
        .max()
        .unwrap_or(0)
        .min(width);
    let pad = width.saturating_sub(widest) / 2;
    let painted = lines
        .into_iter()
        .map(|line| {
            Line::from(Span::styled(
                format!("{}{}", " ".repeat(pad), line.text),
                theme::text(),
            ))
        })
        .collect::<Vec<_>>();
    let inner = inset(area);
    let legend_h = legend_height(app, area);
    let legend_area = Rect {
        x: inner.x,
        y: inner
            .y
            .saturating_add(inner.height.saturating_sub(legend_h)),
        width: inner.width,
        height: legend_h,
    };
    let path_area = path_content(area, legend_h);
    let mut title = if claim {
        " claim path "
    } else {
        " recon path "
    }
    .to_string();
    if app.focus == Target::DetailPath {
        title.push_str("· focused ");
    }
    app.layout.borrow_mut().register(Target::DetailPath, area);
    for index in (app.scrolls.path as usize)
        ..(app.scrolls.path as usize + path_area.height as usize).min(painted.len())
    {
        app.layout.borrow_mut().register(
            Target::PathLine(index),
            Rect {
                x: path_area.x,
                y: path_area.y + (index - app.scrolls.path as usize) as u16,
                width: path_area.width,
                height: 1,
            },
        );
    }
    frame.render_widget(panel(&title), area);
    frame.render_widget(
        Paragraph::new(painted)
            .style(theme::text())
            .scroll((app.scrolls.path, 0))
            .wrap(Wrap { trim: false }),
        path_area,
    );
    if legend_area.height > 0 {
        frame.render_widget(
            Paragraph::new(
                legend_rows(app, claim, legend_area.width as usize)
                    .iter()
                    .take(legend_h as usize)
                    .map(|row| Line::from(center_line(row, legend_area.width as usize)))
                    .collect::<Vec<_>>(),
            )
            .style(theme::dim()),
            legend_area,
        );
    }
}

fn draw_summary(frame: &mut Frame, app: &App, area: Rect, claim: bool) {
    let text = if app.graph_summary.is_empty() {
        if claim {
            "Open a memory to read its claim path."
        } else {
            "Open a memory to read its recon path."
        }
    } else {
        app.graph_summary.as_str()
    };
    let width = area.width.saturating_sub(2) as usize;
    let card_focused = matches!(app.focus, Target::Button(b) if summary_card::is_card_button(b));
    let title = if app.focus == Target::DetailSummary || card_focused {
        " summary · focused "
    } else {
        " summary "
    };
    app.layout
        .borrow_mut()
        .register(Target::DetailSummary, area);
    let Some(failure) = &app.summary_failure else {
        frame.render_widget(
            Paragraph::new(summary_lines(text, width))
                .style(theme::text())
                .block(panel(title))
                .scroll((app.scrolls.summary, 0)),
            area,
        );
        return;
    };
    frame.render_widget(panel(title), area);
    let inner = inset(area);
    let card_h = summary_card::height(failure, app.summary_details_open, inner);
    summary_card::draw(frame, app, failure, inner);
    let body = Rect {
        y: inner.y + card_h,
        height: inner.height.saturating_sub(card_h),
        ..inner
    };
    let mut lines: Vec<Line> = Vec::new();
    if app.summary_details_open {
        for detail in &failure.details {
            lines.push(Line::from(Span::styled(detail.clone(), theme::dim())));
        }
        lines.push(Line::from(""));
    }
    lines.extend(summary_lines(text, width));
    frame.render_widget(
        Paragraph::new(lines)
            .style(theme::text())
            .wrap(Wrap { trim: false })
            .scroll((app.scrolls.summary, 0)),
        body,
    );
}

/// Inner area of the summary panel (where the failure card sits).
pub fn summary_inner(area: Rect) -> Rect {
    inset(area)
}

fn summary_lines(text: &str, width: usize) -> Vec<Line<'static>> {
    markdown::markdown_lines(text, width.max(1))
        .into_iter()
        .map(|line| {
            let mut spans: Vec<Span> = line
                .pieces
                .into_iter()
                .map(|piece| {
                    let mut style = markdown::style(piece.tone);
                    if line.code {
                        style = style.bg(theme::CODE_BG);
                    }
                    Span::styled(piece.text, style)
                })
                .collect();
            if spans.is_empty() {
                spans.push(Span::styled(String::new(), theme::text()));
            }
            Line::from(spans)
        })
        .collect()
}

fn legend_parts(app: &App, claim: bool) -> Vec<String> {
    let mut parts = Vec::new();
    let present = |kind: GraphNodeKind| app.brain_graph.nodes.iter().any(|node| node.kind == kind);
    let push = |parts: &mut Vec<String>, kind: GraphNodeKind, name: &str| {
        if present(kind) {
            parts.push(format!("{} {name}", glyph(kind)));
        }
    };
    push(
        &mut parts,
        GraphNodeKind::Investigation,
        if claim { "claim" } else { "investigation" },
    );
    push(
        &mut parts,
        GraphNodeKind::Directive,
        if claim { "topic" } else { "directive" },
    );
    push(&mut parts, GraphNodeKind::Entity, "entity");
    push(&mut parts, GraphNodeKind::Topic, "topic");
    push(
        &mut parts,
        GraphNodeKind::Evidence,
        if claim { "article" } else { "evidence" },
    );
    push(&mut parts, GraphNodeKind::Finding, "finding");
    push(&mut parts, GraphNodeKind::Source, "source");
    parts
}

pub(crate) fn path_lines(graph: &MemoryGraph) -> Vec<PathLine> {
    let path = recon_path(graph);
    let mut lines = Vec::new();
    if let Some(investigation) = graph
        .nodes
        .iter()
        .find(|node| node.kind == GraphNodeKind::Investigation)
    {
        lines.push(PathLine {
            text: format!("{}  {}", glyph(investigation.kind), investigation.label),
            article_id: String::new(),
            run_id: String::new(),
        });
    }
    for (band_index, band) in path.bands.iter().enumerate() {
        let last_band = band_index + 1 == path.bands.len();
        let branch = if last_band { "└─" } else { "├─" };
        let pad = if last_band { "  " } else { "│ " };
        let caption = graph
            .nodes
            .iter()
            .find(|node| {
                node.kind == GraphNodeKind::Directive && node.label == band.directive_label
            })
            .and_then(|node| node.detail.lines().next())
            .filter(|line| !line.is_empty())
            .unwrap_or(band.directive_label.as_str());
        lines.push(PathLine {
            text: format!("{branch} {}  {caption}", glyph(GraphNodeKind::Directive)),
            article_id: String::new(),
            run_id: String::new(),
        });
        let mut children = Vec::new();
        for subject in &band.subjects {
            if let Some(node) = graph.nodes.iter().find(|node| node.label == *subject) {
                children.push(PathLine {
                    text: format!("{}  {}", glyph(node.kind), node.label),
                    article_id: String::new(),
                    run_id: String::new(),
                });
            }
        }
        children.extend(
            graph
                .nodes
                .iter()
                .filter(|node| {
                    node.kind == GraphNodeKind::Evidence
                        && node.tags.iter().any(|tag| tag == &band.directive_id)
                })
                .map(|node| PathLine {
                    text: format!("{}  {}", glyph(node.kind), node.label),
                    article_id: node.article_id.clone(),
                    run_id: node.run_id.clone(),
                }),
        );
        if let Some(label) = &band.finding {
            children.push(PathLine {
                text: format!("{}  {label}", glyph(GraphNodeKind::Finding)),
                article_id: String::new(),
                run_id: String::new(),
            });
        }
        for (child_index, child) in children.iter().enumerate() {
            let last = child_index + 1 == children.len();
            let mark = if last { "└─" } else { "├─" };
            lines.push(PathLine {
                text: format!("{pad}{mark} {}", child.text),
                article_id: child.article_id.clone(),
                run_id: child.run_id.clone(),
            });
        }
        if children.is_empty() {
            lines.push(PathLine {
                text: format!("{pad}└─ no finding"),
                article_id: String::new(),
                run_id: String::new(),
            });
        }
    }
    if lines.is_empty() {
        for node in &graph.nodes {
            lines.push(PathLine {
                text: format!("{}  {}", glyph(node.kind), node.label),
                article_id: node.article_id.clone(),
                run_id: node.run_id.clone(),
            });
        }
    }
    lines
}

fn glyph(kind: GraphNodeKind) -> &'static str {
    match kind {
        GraphNodeKind::Finding => "★",
        GraphNodeKind::Entity => "●",
        GraphNodeKind::Topic => "◆",
        GraphNodeKind::Directive => "▸",
        GraphNodeKind::Evidence => "·",
        GraphNodeKind::Source => "○",
        GraphNodeKind::Investigation => "▣",
    }
}

#[cfg(test)]
mod tests {
    use super::summary_lines;

    #[test]
    fn summary_markdown_is_parsed_before_it_is_drawn() {
        let lines = summary_lines(
            "## Elon Musk **launched** the Cybercab\n\nThe news directive supports that relation.",
            72,
        );
        let text = lines
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("Elon Musk launched the Cybercab"), "{text}");
        assert!(text.contains("news directive"), "{text}");
        assert!(!text.contains("##"), "{text}");
        assert!(!text.contains("**"), "{text}");
    }
}
