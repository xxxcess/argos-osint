//! Terminal layouts for one memory's investigation graph.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

use argos_osint_core::recon::{force_links, recon_path, GraphNode, GraphNodeKind, MemoryGraph};

use super::app::{App, GraphView, Target};
use super::theme;

struct Chip {
    node_id: String,
    x: u16,
    y: u16,
    width: u16,
    label: String,
    color: Color,
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
    let mut lines = vec![node.kind.label().to_ascii_uppercase(), node.label.clone()];
    if !node.detail.is_empty() {
        lines.push(String::new());
        lines.push(node.detail.clone());
    }
    let mut relations = Vec::new();
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
        relations.push(format!("{}{directive}  {other}", edge.kind.verb()));
    }
    if !relations.is_empty() {
        lines.push(String::new());
        lines.push("Relations".into());
        lines.extend(relations);
    }
    lines.join("\n")
}

pub fn draw(frame: &mut Frame, app: &App, area: Rect) {
    let chrome = graph_chrome(area);
    draw_view_tabs(frame, app, chrome.tabs);
    if app.brain_graph.is_empty() {
        frame.render_widget(
            Paragraph::new("This memory has no investigation graph.")
                .style(theme::dim())
                .wrap(Wrap { trim: true }),
            chrome.canvas,
        );
        return;
    }
    frame.render_widget(
        Paragraph::new(legend_line()).style(theme::muted()),
        chrome.legend,
    );
    if app.graph_view == GraphView::Path || area.width < 56 {
        draw_tree(frame, app, chrome.canvas);
    } else {
        draw_grid(frame, app, chrome.canvas);
    }
    frame.render_widget(
        Paragraph::new(inspector(app))
            .style(theme::text())
            .wrap(Wrap { trim: true }),
        chrome.inspector,
    );
}

struct GraphChrome {
    tabs: Rect,
    legend: Rect,
    canvas: Rect,
    inspector: Rect,
}

fn graph_chrome(area: Rect) -> GraphChrome {
    let rows = split_vertical(
        area,
        [
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(0),
        ],
    );
    let wide = area.width >= 56;
    let cols = if wide {
        split_horizontal(
            rows[2],
            [Constraint::Percentage(70), Constraint::Percentage(30)],
        )
    } else {
        split_vertical(
            rows[2],
            [Constraint::Percentage(62), Constraint::Percentage(38)],
        )
    };
    GraphChrome {
        tabs: rows[0],
        legend: rows[1],
        canvas: cols[0],
        inspector: cols[1],
    }
}

fn draw_view_tabs(frame: &mut Frame, app: &App, area: Rect) {
    let slots = split_horizontal(
        area,
        [
            Constraint::Ratio(1, 3),
            Constraint::Ratio(1, 3),
            Constraint::Ratio(1, 3),
        ],
    );
    for (view, rect) in GraphView::ALL.into_iter().zip(slots) {
        let active = app.graph_view == view;
        let focused = app.focus == Target::GraphView(view);
        let style = if focused {
            theme::selected()
        } else if active {
            theme::accent().add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
        } else {
            theme::dim()
        };
        frame.render_widget(
            Paragraph::new(view.title())
                .alignment(ratatui::layout::Alignment::Center)
                .style(style),
            rect,
        );
    }
}

fn legend_line() -> String {
    "  ● entity   ◆ topic   ★ finding   · evidence   1 force · 2 directive · 3 path".into()
}

fn draw_tree(frame: &mut Frame, app: &App, area: Rect) {
    let lines = tree_lines(app);
    let selected = app.graph_node;
    let painted = lines
        .into_iter()
        .enumerate()
        .map(|(_index, (node_index, text))| {
            let style = if node_index == Some(selected) {
                theme::selected()
            } else if node_index.is_some() {
                theme::text()
            } else {
                theme::dim()
            };
            Line::from(Span::styled(text, style))
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(painted)
            .style(theme::text())
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn tree_lines(app: &App) -> Vec<(Option<usize>, String)> {
    let graph = &app.brain_graph;
    let nodes = selectable(graph, GraphView::Path);
    let index_of = |id: &str| nodes.iter().position(|node| node.id == id);
    let path = recon_path(graph);
    let mut lines = Vec::new();
    if let Some(investigation) = graph
        .nodes
        .iter()
        .find(|node| node.kind == GraphNodeKind::Investigation)
    {
        lines.push((
            index_of(&investigation.id),
            format!("{}  {}", glyph(investigation.kind), investigation.label),
        ));
    }
    for (band_index, band) in path.bands.iter().enumerate() {
        let last_band = band_index + 1 == path.bands.len();
        let branch = if last_band { "└─" } else { "├─" };
        let pad = if last_band { "  " } else { "│ " };
        if let Some(directive) = graph.nodes.iter().find(|node| {
            node.kind == GraphNodeKind::Directive && node.label == band.directive_label
        }) {
            lines.push((
                index_of(&directive.id),
                format!("{branch} {}  {}", glyph(directive.kind), directive.label),
            ));
        } else {
            lines.push((None, format!("{branch} {}", band.directive_label)));
        }
        let mut children = Vec::new();
        for subject in &band.subjects {
            if let Some(node) = graph.nodes.iter().find(|node| node.label == *subject) {
                children.push(node);
            }
        }
        children.extend(graph.nodes.iter().filter(|node| {
            node.kind == GraphNodeKind::Evidence
                && node.tags.iter().any(|tag| tag == &band.directive_id)
        }));
        if let Some(label) = &band.finding {
            if let Some(node) = graph
                .nodes
                .iter()
                .find(|node| node.kind == GraphNodeKind::Finding && node.label == *label)
            {
                children.push(node);
            }
        }
        for (child_index, node) in children.iter().enumerate() {
            let last = child_index + 1 == children.len();
            let mark = if last { "└─" } else { "├─" };
            lines.push((
                index_of(&node.id),
                format!("{pad}{mark} {}  {}", glyph(node.kind), node.label),
            ));
        }
        if children.is_empty() {
            lines.push((None, format!("{pad}└─ no finding")));
        }
    }
    if lines.is_empty() {
        for (index, node) in nodes.iter().enumerate() {
            lines.push((Some(index), format!("{}  {}", glyph(node.kind), node.label)));
        }
    }
    lines
}

fn draw_grid(frame: &mut Frame, app: &App, area: Rect) {
    if area.width < 4 || area.height < 2 {
        return;
    }
    let width = area.width as usize;
    let height = area.height as usize;
    let mut grid = vec![(' ', theme::MUTED); width * height];
    let chips = match app.graph_view {
        GraphView::Directive => layout_directive(app, width, height),
        _ => layout_force(app, width, height),
    };
    let selected = selectable(&app.brain_graph, app.graph_view)
        .get(app.graph_node)
        .map(|node| node.id.as_str());
    paint_edges(&mut grid, width, height, app, &chips, selected);
    for chip in &chips {
        let focused = selected == Some(chip.node_id.as_str());
        paint_chip(&mut grid, width, height, chip, focused);
    }
    let lines = (0..height)
        .map(|row| {
            let mut spans = Vec::new();
            let mut run = String::new();
            let mut color = theme::MUTED;
            for col in 0..width {
                let (ch, next) = grid[row * width + col];
                if next != color && !run.is_empty() {
                    spans.push(Span::styled(
                        std::mem::take(&mut run),
                        Style::default().fg(color),
                    ));
                    color = next;
                } else if run.is_empty() {
                    color = next;
                }
                run.push(ch);
            }
            if !run.is_empty() {
                spans.push(Span::styled(run, Style::default().fg(color)));
            }
            Line::from(spans)
        })
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(lines).style(theme::text()), area);
}

fn paint_chip(grid: &mut [(char, Color)], width: usize, height: usize, chip: &Chip, focused: bool) {
    let color = if focused { theme::GREEN } else { chip.color };
    let text: Vec<char> = chip.label.chars().take(chip.width as usize).collect();
    for (offset, ch) in text.into_iter().enumerate() {
        let x = chip.x as usize + offset;
        let y = chip.y as usize;
        if x < width && y < height {
            grid[y * width + x] = (ch, color);
        }
    }
}

fn paint_edges(
    grid: &mut [(char, Color)],
    width: usize,
    height: usize,
    app: &App,
    chips: &[Chip],
    selected: Option<&str>,
) {
    let at = |id: &str| {
        chips
            .iter()
            .find(|chip| chip.node_id == id)
            .map(|chip| (chip.x + chip.width.min(2), chip.y))
    };
    for edge in &app.brain_graph.edges {
        let Some((x1, y1)) = at(&edge.from) else {
            continue;
        };
        let Some((x2, y2)) = at(&edge.to) else {
            continue;
        };
        let incident = selected == Some(edge.from.as_str()) || selected == Some(edge.to.as_str());
        let color = if incident {
            edge.directive
                .as_deref()
                .map(theme::directive_color)
                .unwrap_or(theme::ACCENT)
        } else {
            theme::MUTED
        };
        draw_elbow(grid, width, height, x1, y1, x2, y2, color);
    }
}

fn draw_elbow(
    grid: &mut [(char, Color)],
    width: usize,
    height: usize,
    x1: u16,
    y1: u16,
    x2: u16,
    y2: u16,
    color: Color,
) {
    let mid = x1.saturating_add(x2) / 2;
    plot_h(grid, width, height, x1, mid, y1, color);
    plot_v(grid, width, height, mid, y1, y2, color);
    plot_h(grid, width, height, mid, x2, y2, color);
}

fn plot_h(
    grid: &mut [(char, Color)],
    width: usize,
    height: usize,
    x1: u16,
    x2: u16,
    y: u16,
    color: Color,
) {
    if y as usize >= height {
        return;
    }
    let (lo, hi) = if x1 <= x2 { (x1, x2) } else { (x2, x1) };
    for x in lo..=hi {
        plot(grid, width, height, x, y, '─', color);
    }
}

fn plot_v(
    grid: &mut [(char, Color)],
    width: usize,
    height: usize,
    x: u16,
    y1: u16,
    y2: u16,
    color: Color,
) {
    if x as usize >= width {
        return;
    }
    let (lo, hi) = if y1 <= y2 { (y1, y2) } else { (y2, y1) };
    for y in lo..=hi {
        plot(grid, width, height, x, y, '│', color);
    }
}

fn plot(
    grid: &mut [(char, Color)],
    width: usize,
    height: usize,
    x: u16,
    y: u16,
    ch: char,
    color: Color,
) {
    let x = x as usize;
    let y = y as usize;
    if x >= width || y >= height {
        return;
    }
    let i = y * width + x;
    let existing = grid[i].0;
    let next = match (existing, ch) {
        (' ', c) | (c, ' ') => c,
        ('─', '│') | ('│', '─') => '┼',
        (prev, _) if prev != ' ' && !matches!(prev, '─' | '│' | '┼') => prev,
        (_, c) => c,
    };
    if matches!(existing, '●' | '◆' | '★' | '·') {
        return;
    }
    grid[i] = (next, color);
}

fn layout_directive(app: &App, width: usize, height: usize) -> Vec<Chip> {
    let mut columns: [Vec<&GraphNode>; 6] = Default::default();
    for node in &app.brain_graph.nodes {
        columns[column_of(node.kind) as usize].push(node);
    }
    let col_w = (width / 6).max(8);
    let mut chips = Vec::new();
    for (col, nodes) in columns.iter().enumerate() {
        let count = nodes.len().max(1);
        for (row, node) in nodes.iter().enumerate() {
            let x = (col * col_w + 1).min(width.saturating_sub(4));
            let y = ((row + 1) * height / (count + 1)).clamp(0, height.saturating_sub(1));
            chips.push(chip_for(
                node,
                x as u16,
                y as u16,
                (col_w.saturating_sub(2) as u16).max(4),
            ));
        }
    }
    deoverlap(&mut chips, height);
    chips
}

fn layout_force(app: &App, width: usize, height: usize) -> Vec<Chip> {
    let nodes = selectable(&app.brain_graph, GraphView::Force);
    let index_of = |id: &str| nodes.iter().position(|node| node.id == id);
    let pairs: Vec<(usize, usize)> = force_links(&app.brain_graph)
        .into_iter()
        .filter_map(|link| Some((index_of(&link.from)?, index_of(&link.to)?)))
        .collect();
    let positions = force_positions(nodes.len(), &pairs, width as f64 * 2.0, height as f64);
    let mut chips = nodes
        .iter()
        .zip(positions)
        .map(|(node, (x, y))| {
            let x = (x / 2.0).clamp(1.0, (width.saturating_sub(6)) as f64) as u16;
            let y = y.clamp(0.0, (height.saturating_sub(1)) as f64) as u16;
            chip_for(node, x, y, 16)
        })
        .collect::<Vec<_>>();
    deoverlap(&mut chips, height);
    chips
}

fn chip_for(node: &GraphNode, x: u16, y: u16, max_width: u16) -> Chip {
    let label = format!("{} {}", glyph(node.kind), node.label);
    let width = (label.chars().count() as u16 + 1).min(max_width).max(3);
    Chip {
        node_id: node.id.clone(),
        x,
        y,
        width,
        label: truncate(&label, width as usize),
        color: theme::node_color(match node.kind {
            GraphNodeKind::Investigation => "investigation",
            GraphNodeKind::Directive => "directive",
            GraphNodeKind::Entity => "entity",
            GraphNodeKind::Topic => "topic",
            GraphNodeKind::Finding => "finding",
            GraphNodeKind::Evidence => "evidence",
            GraphNodeKind::Source => "source",
        }),
    }
}

fn deoverlap(chips: &mut [Chip], height: usize) {
    chips.sort_by_key(|chip| (chip.y, chip.x));
    let mut used: Vec<(u16, u16, u16)> = Vec::new();
    for chip in chips.iter_mut() {
        let mut y = chip.y;
        while used
            .iter()
            .any(|(ux, uy, uw)| *uy == y && chip.x < ux + uw && chip.x + chip.width > *ux)
        {
            y = y.saturating_add(1);
            if y as usize >= height {
                break;
            }
        }
        chip.y = y.min(height.saturating_sub(1) as u16);
        used.push((chip.x, chip.y, chip.width));
    }
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

pub fn hit(app: &App, area: Rect, x: u16, y: u16) -> Option<Target> {
    let chrome = graph_chrome(area);
    if contains(chrome.tabs, x, y) {
        let tabs = split_horizontal(
            chrome.tabs,
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
    if app.brain_graph.is_empty() || !contains(chrome.canvas, x, y) {
        return None;
    }
    if app.graph_view == GraphView::Path || area.width < 56 {
        let row = y.saturating_sub(chrome.canvas.y) as usize;
        return tree_lines(app)
            .into_iter()
            .nth(row)
            .and_then(|(index, _)| index.map(Target::GraphNode));
    }
    let width = chrome.canvas.width as usize;
    let height = chrome.canvas.height as usize;
    let chips = match app.graph_view {
        GraphView::Directive => layout_directive(app, width, height),
        _ => layout_force(app, width, height),
    };
    let nodes = selectable(&app.brain_graph, app.graph_view);
    let local_x = x.saturating_sub(chrome.canvas.x);
    let local_y = y.saturating_sub(chrome.canvas.y);
    chips.into_iter().find_map(|chip| {
        if local_y == chip.y && local_x >= chip.x && local_x < chip.x + chip.width {
            nodes
                .iter()
                .position(|node| node.id == chip.node_id)
                .map(Target::GraphNode)
        } else {
            None
        }
    })
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
                let dy = (pos[i].1 - pos[j].1) * 2.0;
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
            let dy = (pos[from].1 - pos[to].1) * 2.0;
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
