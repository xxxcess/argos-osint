//! Claim/Recon detail for one memory: path graph above, Related left, Summary right.

use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

use argos_osint_core::recon::{GraphEdgeKind, GraphNodeKind, MemoryGraph};

use super::app::{App, Target};
use super::brain_detail;
use super::markdown;
use super::summary_card;
use super::theme::{self, panel};
use super::ui::center_line;

pub struct PathLine {
    pub text: String,
}

/// Graph above, Related (left) and Summary (right) below; stacked when narrow.
pub fn draw(frame: &mut Frame, app: &App, area: Rect) {
    let areas = brain_detail::areas_for(app, area);
    let claim = app.detail_claim();
    brain_detail::draw_nav(frame, app, &areas, claim);
    draw_path(frame, app, areas.path, claim);
    brain_detail::draw_related(frame, app, areas.related);
    draw_summary(frame, app, areas.summary, claim);
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
    let legend_h = legend_height(app, area);
    let path_area = path_content(area, legend_h);
    let lines = if app.brain_graph.is_empty() {
        vec![PathLine {
            text: if claim {
                "This memory has no claim path.".into()
            } else {
                "This memory has no investigation graph.".into()
            },
        }]
    } else {
        path_lines(&app.brain_graph)
    };
    let lines = with_typed_for_height(&app.brain_typed, lines, path_area.height as usize);
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
    let legend_area = Rect {
        x: inner.x,
        y: inner
            .y
            .saturating_add(inner.height.saturating_sub(legend_h)),
        width: inner.width,
        height: legend_h,
    };
    let mut title = if let Some(kind) = super::app::structural_title(app) {
        kind
    } else if claim {
        " claim path "
    } else {
        " recon path "
    }
    .to_string();
    if let Some(metadata) = app
        .selected_insight
        .as_ref()
        .and_then(|insight| insight.metadata.as_ref())
    {
        title = format!(" {} path · {} ", metadata.insight_kind, metadata.lifecycle);
    }
    if app.focus == Target::DetailPath {
        title.push_str("· focused ");
    }
    title.push_str("· Enter maps ");
    app.layout.borrow_mut().register(Target::DetailPath, area);
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

pub(super) fn summary_lines(text: &str, width: usize) -> Vec<Line<'static>> {
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

fn legend_parts(app: &App, _claim: bool) -> Vec<String> {
    let mut parts = Vec::new();
    for (kind, label) in [
        (GraphNodeKind::Finding, "Claim"),
        (GraphNodeKind::Topic, "Topic"),
        (GraphNodeKind::Entity, "Entity"),
        (GraphNodeKind::Evidence, "Article"),
        (GraphNodeKind::Source, "Source"),
    ] {
        if app.brain_graph.nodes.iter().any(|n| {
            n.kind == kind || (kind == GraphNodeKind::Entity && n.kind == GraphNodeKind::Actor)
        }) {
            parts.push(format!("{} {label}", glyph(kind)));
        }
    }
    parts
}

/// Typed payload header (kind, typed fields) above the path lines.
pub(crate) fn with_typed(typed: &[String], lines: Vec<PathLine>) -> Vec<PathLine> {
    if typed.is_empty() {
        return lines;
    }
    let blank = |text: String| PathLine { text };
    let mut out: Vec<PathLine> = typed.iter().cloned().map(blank).collect();
    out.push(blank(String::new()));
    out.extend(lines);
    out
}

fn with_typed_for_height(
    typed: &[String],
    mut lines: Vec<PathLine>,
    visible_rows: usize,
) -> Vec<PathLine> {
    if !typed.is_empty() && visible_rows <= typed.len().saturating_add(1) {
        lines.push(PathLine {
            text: String::new(),
        });
        lines.extend(typed.iter().cloned().map(|text| PathLine { text }));
        lines
    } else {
        with_typed(typed, lines)
    }
}

pub(crate) fn path_lines(graph: &MemoryGraph) -> Vec<PathLine> {
    let visible = |n: &&argos_osint_core::recon::GraphNode| {
        !matches!(
            n.kind,
            GraphNodeKind::Investigation | GraphNodeKind::Directive
        ) && (n.kind != GraphNodeKind::Evidence || !n.article_id.is_empty())
    };
    let mut nodes: Vec<_> = graph.nodes.iter().filter(visible).collect();
    let actor_root = graph
        .nodes
        .first()
        .filter(|n| n.id.starts_with("actor-group:"));
    nodes.sort_by_key(|n| {
        if actor_root.is_some_and(|root| root.id == n.id) {
            0
        } else {
            match n.kind {
                GraphNodeKind::Topic => 0,
                GraphNodeKind::Finding => 1,
                GraphNodeKind::Entity | GraphNodeKind::Actor => 2,
                GraphNodeKind::Evidence => 3,
                _ => 4,
            }
        }
    });
    let mut lines = Vec::new();
    let total = nodes.len();
    for (index, n) in nodes.into_iter().enumerate() {
        let kind = if n.kind == GraphNodeKind::Actor {
            GraphNodeKind::Entity
        } else {
            n.kind
        };
        let branch = if index == 0 {
            ""
        } else if index + 1 == total {
            "└─ "
        } else {
            "├─ "
        };
        let mut text = format!(
            "{branch}{} {}  {}",
            glyph(kind),
            match kind {
                GraphNodeKind::Finding => "Claim",
                GraphNodeKind::Entity => "Entity",
                GraphNodeKind::Topic => "Topic",
                GraphNodeKind::Evidence => "Article",
                _ => "Source",
            },
            n.label
        );
        for edge in graph
            .edges
            .iter()
            .filter(|e| e.to == n.id && e.kind != GraphEdgeKind::Related)
        {
            if let Some(from) = graph.node(&edge.from).filter(|n| {
                !matches!(
                    n.kind,
                    GraphNodeKind::Investigation | GraphNodeKind::Directive
                )
            }) {
                let relation = if from.kind == GraphNodeKind::Evidence
                    && n.kind == GraphNodeKind::Finding
                    && matches!(edge.kind, GraphEdgeKind::Supports | GraphEdgeKind::Reports)
                {
                    "reported by"
                } else {
                    edge.directive
                        .as_deref()
                        .filter(|_| edge.kind == GraphEdgeKind::Link)
                        .unwrap_or(edge.kind.verb())
                };
                text.push_str(&format!(" · {relation}: {}", from.label));
            }
        }
        lines.push(PathLine { text });
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
        GraphNodeKind::Actor => "◉",
    }
}

#[cfg(test)]
mod tests {
    use super::{summary_lines, with_typed_for_height, PathLine};

    #[test]
    fn compact_path_shows_graph_before_headers_that_fill_the_viewport() {
        let typed = vec![
            "Kind: Claim".into(),
            "Assessment: unassessed".into(),
            "Actors: 1".into(),
        ];
        for height in [0, 3, 4] {
            let lines = with_typed_for_height(
                &typed,
                vec![PathLine {
                    text: "Claim seed".into(),
                }],
                height,
            );
            assert_eq!(lines[0].text, "Claim seed");
            assert_eq!(
                lines[2..].iter().map(|line| &line.text).collect::<Vec<_>>(),
                typed.iter().collect::<Vec<_>>()
            );
        }
        let roomy = with_typed_for_height(
            &typed,
            vec![PathLine {
                text: "Claim seed".into(),
            }],
            5,
        );
        assert_eq!(roomy[0].text, "Kind: Claim");
        assert_eq!(roomy[4].text, "Claim seed");
    }

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
