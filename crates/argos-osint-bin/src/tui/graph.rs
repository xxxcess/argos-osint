//! Terminal layouts for one memory's directive graph.
//!
//! Positions are computed once per draw from a seeded layout. The canvas is not a
//! physics simulation.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::canvas::{Canvas, Line as CanvasLine};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

use argos_osint_core::recon::{force_links, recon_path, GraphNode, GraphNodeKind, MemoryGraph};

use super::app::{App, GraphView, Target};
use super::theme::{self, panel};

struct Spot {
    node_id: String,
    x: f64,
    y: f64,
    label: String,
}

struct Stroke {
    x1: f64,
    y1: f64,
    x2: f64,
    y2: f64,
    color: ratatui::style::Color,
    label: String,
}

pub fn selectable<'a>(graph: &'a MemoryGraph, view: GraphView) -> Vec<&'a GraphNode> {
    match view {
        GraphView::Force => graph
            .nodes
            .iter()
            .filter(|node| {
                matches!(
                    node.kind,
                    GraphNodeKind::Entity | GraphNodeKind::Topic | GraphNodeKind::Finding
                )
            })
            .collect(),
        GraphView::Directive | GraphView::Path => graph.nodes.iter().collect(),
    }
}

pub fn inspector(app: &App) -> String {
    let nodes = selectable(&app.brain_graph, app.graph_view);
    let Some(node) = nodes.get(app.graph_node).copied() else {
        if app.brain_graph.is_empty() {
            return "This memory has no investigation graph.\n\nSave a memory from Create, or select an investigation insight."
                .into();
        }
        return "Select a node.".into();
    };
    let mut lines = vec![
        format!("{}  {}", node.kind.label(), node.label),
        node.detail.clone(),
        String::new(),
    ];
    for edge in &app.brain_graph.edges {
        if edge.from != node.id && edge.to != node.id {
            continue;
        }
        let other_id = if edge.from == node.id {
            edge.to.as_str()
        } else {
            edge.from.as_str()
        };
        let other = app
            .brain_graph
            .node(other_id)
            .map(|item| item.label.as_str())
            .unwrap_or(other_id);
        let directive = edge
            .directive
            .as_deref()
            .map(|id| format!(" {id}"))
            .unwrap_or_default();
        lines.push(format!("{}{directive}  {other}", edge.kind.verb()));
    }
    lines.join("\n")
}

pub fn draw(frame: &mut Frame, app: &App, area: Rect) {
    let rows = split_vertical(area, [Constraint::Length(3), Constraint::Min(0)]);
    let tabs = split_horizontal(
        rows[0],
        [
            Constraint::Ratio(1, 3),
            Constraint::Ratio(1, 3),
            Constraint::Ratio(1, 3),
        ],
    );
    for (view, rect) in GraphView::ALL.into_iter().zip(tabs) {
        let selected = app.graph_view == view || app.focus == Target::GraphView(view);
        let style = if app.graph_view == view {
            theme::selected()
        } else if selected {
            theme::accent()
        } else {
            theme::dim()
        };
        frame.render_widget(
            Paragraph::new(view.title())
                .alignment(ratatui::layout::Alignment::Center)
                .style(style)
                .block(panel("")),
            rect,
        );
    }
    if app.brain_graph.is_empty() {
        frame.render_widget(
            Paragraph::new("This memory has no investigation graph.")
                .style(theme::text())
                .block(panel(" Graph "))
                .wrap(Wrap { trim: true }),
            rows[1],
        );
        return;
    }
    let cols = split_horizontal(
        rows[1],
        [Constraint::Percentage(68), Constraint::Percentage(32)],
    );
    let inner = inset(cols[0]);
    let (spots, strokes) = layout(app, inner.width.max(1) as f64, inner.height.max(1) as f64);
    let width = inner.width.max(1) as f64;
    let height = inner.height.max(1) as f64;
    frame.render_widget(
        Canvas::default()
            .block(panel(match app.graph_view {
                GraphView::Force => " Force ",
                GraphView::Directive => " Directive ",
                GraphView::Path => " Path ",
            }))
            .x_bounds([0.0, width])
            .y_bounds([0.0, height])
            .paint(move |ctx| {
                for stroke in &strokes {
                    ctx.draw(&CanvasLine {
                        x1: stroke.x1,
                        y1: height - stroke.y1,
                        x2: stroke.x2,
                        y2: height - stroke.y2,
                        color: stroke.color,
                    });
                    if !stroke.label.is_empty() {
                        let x = (stroke.x1 + stroke.x2) / 2.0;
                        let y = (stroke.y1 + stroke.y2) / 2.0;
                        ctx.print(x, height - y, stroke.label.clone());
                    }
                }
                for spot in &spots {
                    let style = if spot_selected(app, &spot.node_id) {
                        Style::default()
                            .fg(theme::GREEN)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(theme::TEXT)
                    };
                    ctx.print(
                        spot.x,
                        height - spot.y,
                        Line::from(Span::styled(spot.label.clone(), style)),
                    );
                }
            }),
        cols[0],
    );
    frame.render_widget(
        Paragraph::new(inspector(app))
            .style(theme::text())
            .block(panel(" Inspector "))
            .wrap(Wrap { trim: true }),
        cols[1],
    );
}

fn spot_selected(app: &App, node_id: &str) -> bool {
    selectable(&app.brain_graph, app.graph_view)
        .get(app.graph_node)
        .is_some_and(|node| node.id == node_id)
}

pub fn hit(app: &App, area: Rect, x: u16, y: u16) -> Option<Target> {
    let rows = split_vertical(area, [Constraint::Length(3), Constraint::Min(0)]);
    if contains(rows[0], x, y) {
        let tabs = split_horizontal(
            rows[0],
            [
                Constraint::Ratio(1, 3),
                Constraint::Ratio(1, 3),
                Constraint::Ratio(1, 3),
            ],
        );
        return tabs
            .into_iter()
            .zip(GraphView::ALL)
            .find_map(|(rect, view)| contains(rect, x, y).then_some(Target::GraphView(view)));
    }
    if app.brain_graph.is_empty() || !contains(rows[1], x, y) {
        return None;
    }
    let cols = split_horizontal(
        rows[1],
        [Constraint::Percentage(68), Constraint::Percentage(32)],
    );
    let inner = inset(cols[0]);
    if !contains(inner, x, y) {
        return None;
    }
    let (spots, _) = layout(app, inner.width.max(1) as f64, inner.height.max(1) as f64);
    let nodes = selectable(&app.brain_graph, app.graph_view);
    let mut best: Option<(usize, f64)> = None;
    for spot in &spots {
        let Some(index) = nodes.iter().position(|node| node.id == spot.node_id) else {
            continue;
        };
        let dx = f64::from(x) - (f64::from(inner.x) + spot.x);
        let dy = f64::from(y) - (f64::from(inner.y) + spot.y);
        let width = spot.label.chars().count() as f64;
        if dx < -1.0 || dx > width + 1.0 || dy.abs() > 1.0 {
            continue;
        }
        let distance = dx * dx + dy * dy;
        if best.is_none_or(|(_, nearest)| distance < nearest) {
            best = Some((index, distance));
        }
    }
    best.map(|(index, _)| Target::GraphNode(index))
}

fn layout(app: &App, width: f64, height: f64) -> (Vec<Spot>, Vec<Stroke>) {
    match app.graph_view {
        GraphView::Force => layout_force(&app.brain_graph, app.graph_node, width, height),
        GraphView::Directive => layout_directive(&app.brain_graph, width, height),
        GraphView::Path => layout_path(&app.brain_graph, width, height),
    }
}

fn layout_force(
    graph: &MemoryGraph,
    selected: usize,
    width: f64,
    height: f64,
) -> (Vec<Spot>, Vec<Stroke>) {
    let nodes = selectable(graph, GraphView::Force);
    let index_of = |id: &str| nodes.iter().position(|node| node.id == id);
    let pairs = force_links(graph)
        .into_iter()
        .filter_map(|link| Some((index_of(&link.from)?, index_of(&link.to)?, link.directive)))
        .collect::<Vec<_>>();
    let bare: Vec<(usize, usize)> = pairs.iter().map(|(from, to, _)| (*from, *to)).collect();
    let positions = force_positions(nodes.len(), &bare, width, height);
    let selected_id = nodes.get(selected).map(|node| node.id.as_str());
    let spots = nodes
        .iter()
        .zip(positions)
        .map(|(node, (x, y))| Spot {
            node_id: node.id.clone(),
            x,
            y,
            label: truncate(&node_label(node), 18),
        })
        .collect::<Vec<_>>();
    let strokes = pairs
        .into_iter()
        .filter_map(|(from, to, directive)| {
            let left = spots.get(from)?;
            let right = spots.get(to)?;
            let incident = selected_id == Some(left.node_id.as_str())
                || selected_id == Some(right.node_id.as_str());
            Some(Stroke {
                x1: left.x,
                y1: left.y,
                x2: right.x,
                y2: right.y,
                color: theme::directive_color(&directive),
                label: if incident { directive } else { String::new() },
            })
        })
        .collect();
    (spots, strokes)
}

fn layout_directive(graph: &MemoryGraph, width: f64, height: f64) -> (Vec<Spot>, Vec<Stroke>) {
    let mut columns: [Vec<&GraphNode>; 6] = [
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
    ];
    for node in &graph.nodes {
        columns[column_of(node.kind) as usize].push(node);
    }
    let mut spots = Vec::new();
    let mut at: std::collections::HashMap<String, (f64, f64)> = std::collections::HashMap::new();
    for (col, nodes) in columns.iter().enumerate() {
        let count = nodes.len().max(1);
        for (row, node) in nodes.iter().enumerate() {
            let x = (col as f64 + 0.08) * width / 6.0;
            let y = (row as f64 + 1.0) * height / (count as f64 + 1.0);
            let x = x.clamp(1.0, (width - 8.0).max(1.0));
            let y = y.clamp(1.0, (height - 1.0).max(1.0));
            at.insert(node.id.clone(), (x, y));
            spots.push(Spot {
                node_id: node.id.clone(),
                x,
                y,
                label: truncate(&node_label(node), 14),
            });
        }
    }
    let mut strokes = Vec::new();
    for edge in &graph.edges {
        let Some((x1, y1)) = at.get(&edge.from).copied() else {
            continue;
        };
        let Some((x2, y2)) = at.get(&edge.to).copied() else {
            continue;
        };
        let color = edge
            .directive
            .as_deref()
            .map(theme::directive_color)
            .unwrap_or(theme::DIM);
        push_elbow(&mut strokes, x1, y1, x2, y2, color, edge.kind.verb());
    }
    (spots, strokes)
}

fn layout_path(graph: &MemoryGraph, width: f64, height: f64) -> (Vec<Spot>, Vec<Stroke>) {
    let path = recon_path(graph);
    if path.bands.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let band_h = height / path.bands.len() as f64;
    let x_of = |col: f64| ((col + 0.08) * width / 5.0).clamp(1.0, (width - 10.0).max(1.0));
    let mut spots = Vec::new();
    let mut strokes = Vec::new();
    let investigation = graph
        .nodes
        .iter()
        .find(|node| node.kind == GraphNodeKind::Investigation);
    let inv_y = (height / 2.0).clamp(1.0, (height - 1.0).max(1.0));
    let inv_x = x_of(0.0);
    if let Some(node) = investigation {
        spots.push(Spot {
            node_id: node.id.clone(),
            x: inv_x,
            y: inv_y,
            label: truncate(&node.label, 16),
        });
    }
    for (band_index, band) in path.bands.iter().enumerate() {
        let y = (band_h * band_index as f64 + band_h / 2.0).clamp(1.0, (height - 1.0).max(1.0));
        let directive = graph.nodes.iter().find(|node| {
            node.kind == GraphNodeKind::Directive && node.label == band.directive_label
        });
        let dir_x = x_of(1.0);
        if let Some(node) = directive {
            spots.push(Spot {
                node_id: node.id.clone(),
                x: dir_x,
                y,
                label: truncate(&node.label, 8),
            });
            strokes.push(Stroke {
                x1: inv_x,
                y1: inv_y,
                x2: dir_x,
                y2: y,
                color: theme::directive_color(&band.directive_id),
                label: String::new(),
            });
        }
        let subject_nodes = graph
            .nodes
            .iter()
            .filter(|node| band.subjects.iter().any(|label| label == &node.label))
            .collect::<Vec<_>>();
        let subject_x = x_of(2.0);
        for (offset, node) in subject_nodes.iter().enumerate() {
            let subject_y = (y + offset as f64).min((height - 1.0).max(1.0));
            spots.push(Spot {
                node_id: node.id.clone(),
                x: subject_x,
                y: subject_y,
                label: truncate(&node.label, 14),
            });
            strokes.push(Stroke {
                x1: dir_x,
                y1: y,
                x2: subject_x,
                y2: subject_y,
                color: theme::directive_color(&band.directive_id),
                label: String::new(),
            });
        }
        let evidence_nodes = graph
            .nodes
            .iter()
            .filter(|node| {
                node.kind == GraphNodeKind::Evidence
                    && node.tags.iter().any(|tag| tag == &band.directive_id)
            })
            .collect::<Vec<_>>();
        let evidence_x = x_of(3.0);
        let from_x = if subject_nodes.is_empty() {
            dir_x
        } else {
            subject_x
        };
        let from_y = y;
        for (offset, node) in evidence_nodes.iter().enumerate() {
            let evidence_y = (y + offset as f64).min((height - 1.0).max(1.0));
            spots.push(Spot {
                node_id: node.id.clone(),
                x: evidence_x,
                y: evidence_y,
                label: truncate(&node.label, 12),
            });
            strokes.push(Stroke {
                x1: from_x,
                y1: from_y,
                x2: evidence_x,
                y2: evidence_y,
                color: theme::directive_color(&band.directive_id),
                label: String::new(),
            });
        }
        let find_x = x_of(4.0);
        let link_x = if evidence_nodes.is_empty() {
            from_x
        } else {
            evidence_x
        };
        if let Some(label) = &band.finding {
            if let Some(node) = graph
                .nodes
                .iter()
                .find(|node| node.kind == GraphNodeKind::Finding && node.label == *label)
            {
                spots.push(Spot {
                    node_id: node.id.clone(),
                    x: find_x,
                    y,
                    label: truncate(label, 16),
                });
                strokes.push(Stroke {
                    x1: link_x,
                    y1: y,
                    x2: find_x,
                    y2: y,
                    color: theme::directive_color(&band.directive_id),
                    label: String::new(),
                });
            }
        } else {
            spots.push(Spot {
                node_id: String::new(),
                x: find_x,
                y,
                label: "no finding".into(),
            });
            strokes.push(Stroke {
                x1: link_x,
                y1: y,
                x2: find_x,
                y2: y,
                color: theme::DIM,
                label: String::new(),
            });
        }
    }
    (spots, strokes)
}

fn node_label(node: &GraphNode) -> String {
    match node.kind {
        GraphNodeKind::Finding => format!("★ {}", node.label),
        GraphNodeKind::Entity => format!("● {}", node.label),
        GraphNodeKind::Topic => format!("◆ {}", node.label),
        GraphNodeKind::Directive => node.label.clone(),
        _ => node.label.clone(),
    }
}

fn column_of(kind: GraphNodeKind) -> u8 {
    match kind {
        GraphNodeKind::Investigation => 0,
        GraphNodeKind::Directive => 1,
        GraphNodeKind::Entity | GraphNodeKind::Topic => 2,
        GraphNodeKind::Finding => 3,
        GraphNodeKind::Evidence => 4,
        GraphNodeKind::Source => 5,
    }
}

fn push_elbow(
    strokes: &mut Vec<Stroke>,
    x1: f64,
    y1: f64,
    x2: f64,
    y2: f64,
    color: ratatui::style::Color,
    label: &str,
) {
    let mid = (x1 + x2) / 2.0;
    strokes.push(Stroke {
        x1,
        y1,
        x2: mid,
        y2: y1,
        color,
        label: String::new(),
    });
    strokes.push(Stroke {
        x1: mid,
        y1,
        x2: mid,
        y2,
        color,
        label: label.to_string(),
    });
    strokes.push(Stroke {
        x1: mid,
        y1: y2,
        x2,
        y2,
        color,
        label: String::new(),
    });
}

pub fn force_positions(
    count: usize,
    edges: &[(usize, usize)],
    width: f64,
    height: f64,
) -> Vec<(f64, f64)> {
    let width = width.max(1.0);
    let height = height.max(1.0);
    if count == 0 {
        return Vec::new();
    }
    let max_x = (width - 8.0).max(1.0);
    let max_y = (height - 1.0).max(1.0);
    if count == 1 {
        return vec![(width / 2.0, height / 2.0)];
    }
    let mut pos = (0..count)
        .map(|index| {
            let angle = index as f64 / count as f64 * std::f64::consts::TAU;
            (
                width / 2.0 + angle.cos() * width * 0.25,
                height / 2.0 + angle.sin() * height * 0.25,
            )
        })
        .collect::<Vec<_>>();
    let k = (width * height / count as f64).sqrt().max(0.01);
    let mut temperature = width.min(height) / 10.0;
    let cool = temperature / 40.0;
    for _ in 0..40 {
        let mut disp = vec![(0.0, 0.0); count];
        for i in 0..count {
            for j in (i + 1)..count {
                let dx = pos[i].0 - pos[j].0;
                let dy = pos[i].1 - pos[j].1;
                let dist = (dx * dx + dy * dy).sqrt().max(0.01);
                let force = k * k / dist;
                disp[i].0 += dx / dist * force;
                disp[i].1 += dy / dist * force;
                disp[j].0 -= dx / dist * force;
                disp[j].1 -= dy / dist * force;
            }
        }
        for &(from, to) in edges {
            if from >= count || to >= count || from == to {
                continue;
            }
            let dx = pos[from].0 - pos[to].0;
            let dy = pos[from].1 - pos[to].1;
            let dist = (dx * dx + dy * dy).sqrt().max(0.01);
            let force = dist * dist / k;
            disp[from].0 -= dx / dist * force;
            disp[from].1 -= dy / dist * force;
            disp[to].0 += dx / dist * force;
            disp[to].1 += dy / dist * force;
        }
        for i in 0..count {
            let length = (disp[i].0 * disp[i].0 + disp[i].1 * disp[i].1)
                .sqrt()
                .max(0.01);
            let step = temperature.min(length);
            pos[i].0 = (pos[i].0 + disp[i].0 / length * step).clamp(1.0, max_x);
            pos[i].1 = (pos[i].1 + disp[i].1 / length * step).clamp(1.0, max_y);
        }
        temperature = (temperature - cool).max(0.0);
    }
    pos
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn split_vertical(area: Rect, constraints: impl IntoIterator<Item = Constraint>) -> Vec<Rect> {
    Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area)
        .to_vec()
}

fn split_horizontal(area: Rect, constraints: impl IntoIterator<Item = Constraint>) -> Vec<Rect> {
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(area)
        .to_vec()
}

fn inset(area: Rect) -> Rect {
    Rect {
        x: area.x.saturating_add(1),
        y: area.y.saturating_add(1),
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    }
}

fn contains(area: Rect, x: u16, y: u16) -> bool {
    x >= area.x
        && y >= area.y
        && x < area.x.saturating_add(area.width)
        && y < area.y.saturating_add(area.height)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn force_layout_is_deterministic_and_in_bounds() {
        let edges = [(0usize, 1usize), (1, 2)];
        let first = force_positions(3, &edges, 80.0, 24.0);
        let second = force_positions(3, &edges, 80.0, 24.0);
        assert_eq!(first, second);
        assert_eq!(first.len(), 3);
        for (x, y) in first {
            assert!((0.0..=80.0).contains(&x), "{x}");
            assert!((0.0..=24.0).contains(&y), "{y}");
        }
    }

    #[test]
    fn directive_columns_follow_kind() {
        assert_eq!(column_of(GraphNodeKind::Investigation), 0);
        assert_eq!(column_of(GraphNodeKind::Directive), 1);
        assert_eq!(column_of(GraphNodeKind::Entity), 2);
        assert_eq!(column_of(GraphNodeKind::Topic), 2);
        assert_eq!(column_of(GraphNodeKind::Finding), 3);
        assert_eq!(column_of(GraphNodeKind::Evidence), 4);
        assert_eq!(column_of(GraphNodeKind::Source), 5);
    }
}
