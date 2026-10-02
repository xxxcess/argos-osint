//! Recon-path layout for one memory, stacked above its saved summary.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

use argos_osint_core::recon::{recon_path, GraphNodeKind, MemoryGraph};

use super::app::App;
use super::markdown;
use super::theme::{self, panel};

pub fn draw(frame: &mut Frame, app: &App, area: Rect) {
    let rows = split_vertical(
        area,
        [Constraint::Percentage(55), Constraint::Percentage(45)],
    );
    draw_path(frame, app, rows[0]);
    draw_summary(frame, app, rows[1]);
}

fn draw_path(frame: &mut Frame, app: &App, area: Rect) {
    let width = area.width.saturating_sub(2) as usize;
    let lines = if app.brain_graph.is_empty() {
        vec!["This memory has no investigation graph.".to_string()]
    } else {
        path_lines(&app.brain_graph)
    };
    let widest = lines
        .iter()
        .map(|line| line.chars().count())
        .max()
        .unwrap_or(0)
        .min(width);
    let pad = width.saturating_sub(widest) / 2;
    let painted = lines
        .into_iter()
        .map(|text| {
            Line::from(Span::styled(
                format!("{}{text}", " ".repeat(pad)),
                theme::text(),
            ))
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(painted)
            .style(theme::text())
            .block(panel(" recon path "))
            .scroll((app.scrolls.path, 0))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn draw_summary(frame: &mut Frame, app: &App, area: Rect) {
    let text = if app.graph_summary.is_empty() {
        "Open a memory to read its recon path."
    } else {
        app.graph_summary.as_str()
    };
    let width = area.width.saturating_sub(2) as usize;
    frame.render_widget(
        Paragraph::new(summary_lines(text, width))
            .style(theme::text())
            .block(panel(" summary "))
            .scroll((app.scrolls.summary, 0)),
        area,
    );
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

fn path_lines(graph: &MemoryGraph) -> Vec<String> {
    let path = recon_path(graph);
    let mut lines = Vec::new();
    if let Some(investigation) = graph
        .nodes
        .iter()
        .find(|node| node.kind == GraphNodeKind::Investigation)
    {
        lines.push(format!(
            "{}  {}",
            glyph(investigation.kind),
            investigation.label
        ));
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
        lines.push(format!(
            "{branch} {}  {caption}",
            glyph(GraphNodeKind::Directive)
        ));
        let mut children = Vec::new();
        for subject in &band.subjects {
            if let Some(node) = graph.nodes.iter().find(|node| node.label == *subject) {
                children.push(format!("{}  {}", glyph(node.kind), node.label));
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
                .map(|node| format!("{}  {}", glyph(node.kind), node.label)),
        );
        if let Some(label) = &band.finding {
            children.push(format!("{}  {label}", glyph(GraphNodeKind::Finding)));
        }
        for (child_index, child) in children.iter().enumerate() {
            let last = child_index + 1 == children.len();
            let mark = if last { "└─" } else { "├─" };
            lines.push(format!("{pad}{mark} {child}"));
        }
        if children.is_empty() {
            lines.push(format!("{pad}└─ no finding"));
        }
    }
    if lines.is_empty() {
        for node in &graph.nodes {
            lines.push(format!("{}  {}", glyph(node.kind), node.label));
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

fn split_vertical(area: Rect, constraints: impl IntoIterator<Item = Constraint>) -> Vec<Rect> {
    Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area)
        .to_vec()
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
