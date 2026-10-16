//! Seeded, immutable topology presentation in the Graph first preset.
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use argos_osint_core::brain_topology::{
    CycleScope, TopologyEdgeKind, TopologyNode, TopologyNodeKind, TopologySeed, TopologySnapshot,
};

use super::app::{App, ButtonId, Target};
use super::components::{self, ListGeometry, ScrollPane};
use super::theme::{self, panel, ACCENT, BG, BORDER, DIM, GREEN, MUTED, SERIES, WARN};
use super::topology_layout::{graph_geometry, TopologyLayoutResult, TopologyResponsiveMode};
use super::ui::{draw_button, draw_button_state};

pub const TOPOLOGY_HISTORY_CAP: usize = 32;

#[derive(Clone, Debug)]
pub enum TopologyFocus {
    Node(String),
    Edge(String),
    Relationship(String),
    Control(Target),
}

pub fn stable_focus(state: &TopologyState, target: Target) -> Option<TopologyFocus> {
    match target {
        Target::TopologyNode(index) => state
            .snapshot
            .as_ref()?
            .nodes
            .get(index)
            .map(|node| TopologyFocus::Node(node.id.clone())),
        Target::TopologyEdge(index) => state
            .snapshot
            .as_ref()?
            .edges
            .get(index)
            .map(|edge| TopologyFocus::Edge(edge.id.clone())),
        Target::TopologyRelated(index) => {
            let edge_index = *related_edge_indices(state).get(index)?;
            state
                .snapshot
                .as_ref()?
                .edges
                .get(edge_index)
                .map(|edge| TopologyFocus::Relationship(edge.id.clone()))
        }
        target => Some(TopologyFocus::Control(target)),
    }
}

pub fn resolve_focus(state: &TopologyState, focus: TopologyFocus) -> Option<Target> {
    match focus {
        TopologyFocus::Node(id) => state
            .snapshot
            .as_ref()?
            .nodes
            .iter()
            .position(|node| node.id == id && node_visible(state, node))
            .map(Target::TopologyNode),
        TopologyFocus::Edge(id) => state
            .snapshot
            .as_ref()?
            .edges
            .iter()
            .position(|edge| edge.id == id && edge_visible(state, edge.kind))
            .map(Target::TopologyEdge),
        TopologyFocus::Relationship(id) => {
            let snapshot = state.snapshot.as_ref()?;
            related_edge_indices(state)
                .iter()
                .position(|index| snapshot.edges[*index].id == id)
                .map(Target::TopologyRelated)
        }
        TopologyFocus::Control(target) => Some(target),
    }
}

#[derive(Clone, Debug)]
pub struct TopologyHistoryFrame {
    pub seed: TopologySeed,
    pub selected_node_id: Option<String>,
    pub selected_edge_id: Option<String>,
    pub depth: usize,
    pub cycle_scope: CycleScope,
    pub zoom_percent: usize,
    pub pan: (i16, i16),
    pub viewport_center_id: Option<String>,
    pub active_panel_tab: usize,
    pub inspector_tab: usize,
    pub inspector_scroll: usize,
    pub related_sel: usize,
    pub related_scroll: usize,
    pub filters: [bool; 6],
    pub focus: Option<TopologyFocus>,
}

#[derive(Clone, Debug)]
pub struct TopologyState {
    pub seed: Option<TopologySeed>,
    pub snapshot: Option<TopologySnapshot>,
    pub selected_node_id: Option<String>,
    pub selected_edge_id: Option<String>,
    pub depth: usize,
    pub cycle_scope: CycleScope,
    pub filter_actor: bool,
    pub filter_claim: bool,
    pub filter_inference: bool,
    pub filter_link: bool,
    pub filter_related: bool,
    pub filter_sources: bool,
    pub zoom_percent: usize,
    pub pan: (i16, i16),
    pub viewport_center_id: Option<String>,
    pub active_panel_tab: usize,
    pub inspector_tab: usize,
    pub inspector_scroll: usize,
    pub related_sel: usize,
    /// Logical relationship record offset; detail prose has its own row offset.
    pub related_scroll: usize,
    pub analyzing_scope: bool,
    pub scope_status: String,
    pub history: Vec<TopologyHistoryFrame>,
    pub pending_focus: Option<TopologyFocus>,
}

impl Default for TopologyState {
    fn default() -> Self {
        Self {
            seed: None,
            snapshot: None,
            selected_node_id: None,
            selected_edge_id: None,
            depth: 2,
            cycle_scope: CycleScope::AllRetained,
            filter_actor: true,
            filter_claim: true,
            filter_inference: true,
            filter_link: true,
            filter_related: true,
            filter_sources: true,
            zoom_percent: 100,
            pan: (0, 0),
            viewport_center_id: None,
            active_panel_tab: 0,
            inspector_tab: 0,
            inspector_scroll: 0,
            related_sel: 0,
            related_scroll: 0,
            analyzing_scope: false,
            scope_status: "On demand · idle".into(),
            history: Vec::new(),
            pending_focus: None,
        }
    }
}

impl TopologyState {
    pub fn filters(&self) -> [bool; 6] {
        [
            self.filter_actor,
            self.filter_claim,
            self.filter_inference,
            self.filter_link,
            self.filter_related,
            self.filter_sources,
        ]
    }

    pub fn set_filters(&mut self, filters: [bool; 6]) {
        [
            self.filter_actor,
            self.filter_claim,
            self.filter_inference,
            self.filter_link,
            self.filter_related,
            self.filter_sources,
        ] = filters;
    }

    pub fn push_history(&mut self) {
        if let Some(seed) = &self.seed {
            if self.history.len() >= TOPOLOGY_HISTORY_CAP {
                self.history.remove(0);
            }
            self.history.push(TopologyHistoryFrame {
                seed: seed.clone(),
                selected_node_id: self.selected_node_id.clone(),
                selected_edge_id: self.selected_edge_id.clone(),
                depth: self.depth,
                cycle_scope: self.cycle_scope.clone(),
                zoom_percent: self.zoom_percent,
                pan: self.pan,
                viewport_center_id: self.viewport_center_id.clone(),
                active_panel_tab: self.active_panel_tab,
                inspector_tab: self.inspector_tab,
                inspector_scroll: self.inspector_scroll,
                related_sel: self.related_sel,
                related_scroll: self.related_scroll,
                filters: self.filters(),
                focus: None,
            });
        }
    }

    pub fn pop_history(&mut self) -> Option<TopologyHistoryFrame> {
        self.history.pop()
    }

    pub fn restore_history(&mut self, frame: TopologyHistoryFrame) {
        self.seed = Some(frame.seed);
        self.selected_node_id = frame.selected_node_id;
        self.selected_edge_id = frame.selected_edge_id;
        self.depth = frame.depth;
        self.cycle_scope = frame.cycle_scope;
        self.zoom_percent = frame.zoom_percent;
        self.pan = frame.pan;
        self.viewport_center_id = frame.viewport_center_id;
        self.active_panel_tab = frame.active_panel_tab;
        self.inspector_tab = frame.inspector_tab;
        self.inspector_scroll = frame.inspector_scroll;
        self.related_sel = frame.related_sel;
        self.related_scroll = frame.related_scroll;
        self.set_filters(frame.filters);
        self.pending_focus = frame.focus;
    }

    pub fn current_cycle_id(&self) -> Option<String> {
        let snapshot = self.snapshot.as_ref()?;
        self.selected_node_id
            .as_ref()
            .and_then(|id| snapshot.nodes.iter().find(|node| &node.id == id))
            .and_then(|node| node.cycle.clone())
            .or_else(|| {
                snapshot
                    .nodes
                    .iter()
                    .find(|node| node.is_root)
                    .and_then(|node| node.cycle.clone())
            })
            .or_else(|| match self.seed.as_ref()? {
                TopologySeed::Article { run_id, .. }
                | TopologySeed::RetainedClaim { run_id, .. }
                | TopologySeed::RetainedInference { run_id, .. }
                | TopologySeed::RetainedActor { run_id, .. } => {
                    (!run_id.is_empty()).then(|| run_id.clone())
                }
                _ => None,
            })
    }

    pub fn center_selected(&mut self) {
        self.pan = (0, 0);
        self.viewport_center_id = self.selected_node_id.clone();
    }
}

pub fn node_visible(state: &TopologyState, node: &TopologyNode) -> bool {
    node.is_root
        || match node.kind {
            TopologyNodeKind::Entity | TopologyNodeKind::ActorGroup => state.filter_actor,
            TopologyNodeKind::Claim => state.filter_claim,
            TopologyNodeKind::Inference | TopologyNodeKind::Topic => state.filter_inference,
            TopologyNodeKind::Article | TopologyNodeKind::Source => state.filter_sources,
        }
}

fn edge_visible(state: &TopologyState, kind: TopologyEdgeKind) -> bool {
    match kind {
        TopologyEdgeKind::Related => state.filter_related,
        TopologyEdgeKind::Link => state.filter_link,
        _ => true,
    }
}

fn visible_endpoints(
    state: &TopologyState,
    snapshot: &TopologySnapshot,
    from: &str,
    to: &str,
) -> bool {
    [from, to].into_iter().all(|id| {
        snapshot
            .nodes
            .iter()
            .find(|node| node.id == id)
            .is_some_and(|node| node_visible(state, node))
    })
}

/// Ordered actual snapshot edge indices for the selected node's relationship list.
pub fn related_edge_indices(state: &TopologyState) -> Vec<usize> {
    let Some(snapshot) = &state.snapshot else {
        return Vec::new();
    };
    let anchor = state
        .selected_node_id
        .as_deref()
        .unwrap_or(&snapshot.root_id);
    snapshot
        .edges
        .iter()
        .enumerate()
        .filter(|(_, edge)| {
            (edge.from == anchor || edge.to == anchor)
                && edge_visible(state, edge.kind)
                && visible_endpoints(state, snapshot, &edge.from, &edge.to)
                && (state.inspector_tab == 0
                    || snapshot.nodes.iter().any(|node| {
                        node.id
                            == (if edge.from == anchor {
                                &edge.to
                            } else {
                                &edge.from
                            })
                            .as_str()
                            && matches!(
                                node.kind,
                                TopologyNodeKind::Article | TopologyNodeKind::Source
                            )
                    }))
        })
        .map(|(index, _)| index)
        .collect()
}

/// Scroll exactly the immutable content measured by the most recent draw.
pub fn scroll_topology(app: &mut App, target: Target, delta: isize) -> bool {
    match target {
        Target::TopologyGraph | Target::TopologyNode(_) | Target::TopologyEdge(_) => {
            let delta = delta.clamp(i16::MIN as isize, i16::MAX as isize) as i16;
            app.topology_state.pan.1 = app.topology_state.pan.1.saturating_sub(delta);
            true
        }
        Target::TopologyInspector => {
            let area = app
                .layout
                .borrow()
                .entries
                .iter()
                .find(|entry| entry.target == Target::TopologyInspector)
                .map(|entry| entry.rect);
            if let (Some(area), Some(snapshot)) = (area, app.topology_state.snapshot.as_ref()) {
                let lines = components::measured_lines(
                    detail_lines(&app.topology_state, snapshot),
                    area.width,
                );
                let mut pane = ScrollPane {
                    offset: app.topology_state.inspector_scroll,
                };
                pane.scroll(delta, lines.len(), area.height as usize);
                app.topology_state.inspector_scroll = pane.offset;
            }
            true
        }
        Target::TopologyRelated(_) | Target::TopologyRelationships => {
            app.topology_state.related_scroll = app
                .topology_state
                .related_scroll
                .saturating_add_signed(delta)
                .min(
                    related_edge_indices(&app.topology_state)
                        .len()
                        .saturating_sub(1),
                );
            true
        }
        _ => false,
    }
}

pub fn draw_topology(frame: &mut Frame, app: &App, area: Rect) {
    let layout = TopologyLayoutResult::compute(area);
    if layout.mode == TopologyResponsiveMode::Tiny {
        draw_button(
            frame,
            app,
            ButtonId::TopologyBack,
            "[← Back]",
            Rect::new(area.x, area.y, area.width.min(9), area.height.min(1)),
        );
        let notice = Rect::new(
            area.x,
            area.y.saturating_add(1),
            area.width,
            area.height.saturating_sub(1),
        );
        frame.render_widget(
            Paragraph::new(components::measured_lines(
                vec![
                    Line::styled("Topology mapper", theme::accent()),
                    Line::raw(
                    "Needs 60×18 content cells (60×20 terminal). Resize to inspect the neighborhood.",
                    ),
                ],
                notice.width.saturating_sub(2),
            ))
            .block(panel(" Topology ")),
            notice,
        );
        return;
    }
    draw_top_bar(frame, app, &layout);
    let split = matches!(
        layout.mode,
        TopologyResponsiveMode::Desktop | TopologyResponsiveMode::Compact
    );
    if app.topology_state.active_panel_tab == 2 {
        draw_controls_page(frame, app, layout.controls_page);
    } else if split {
        draw_graph_panel(frame, app, &layout);
        draw_inspector(frame, app, &layout);
    } else if app.topology_state.active_panel_tab == 1 {
        draw_inspector(frame, app, &layout);
    } else {
        draw_graph_panel(frame, app, &layout);
    }
    frame.render_widget(
        Paragraph::new(components::clip_text(
            "Tab focus · Enter inspect · r refresh · Esc back",
            layout.footer.width as usize,
        ))
        .style(theme::muted()),
        layout.footer,
    );
}

fn draw_top_bar(frame: &mut Frame, app: &App, layout: &TopologyLayoutResult) {
    let labels = ["[← Back] ", "Topology mapper"];
    let rects = components::action_rects(layout.header, &labels);
    draw_button(frame, app, ButtonId::TopologyBack, labels[0], rects[0]);
    frame.render_widget(
        Paragraph::new(labels[1]).style(theme::text().add_modifier(Modifier::BOLD)),
        rects[1],
    );
    let state = &app.topology_state;
    let seed = state
        .snapshot
        .as_ref()
        .map(|snapshot| snapshot.root_label.clone())
        .or_else(|| state.seed.as_ref().map(TopologySeed::display_label))
        .unwrap_or_else(|| "No seed selected".into());
    let origin = if app.topology_origin_label.is_empty() {
        "Brain"
    } else {
        app.topology_origin_label.as_str()
    };
    let context = format!("Origin: {origin} · Seed: {seed}");
    frame.render_widget(
        Paragraph::new(components::clip_text(
            &context,
            layout.subheader.width as usize,
        ))
        .style(theme::muted()),
        layout.subheader,
    );
    let tabs = ["Graph", "Details", "Controls"];
    let reveal = match app.focus {
        Target::TopologyPanel(index) => index,
        _ => state.active_panel_tab,
    };
    for (index, rect) in components::tab_rects(layout.seed_tabs, &tabs, reveal)
        .into_iter()
        .enumerate()
    {
        if rect.width == 0 {
            continue;
        }
        let target = Target::TopologyPanel(index);
        components::tab_button(
            frame,
            rect,
            tabs[index],
            state.active_panel_tab == index,
            app.focus == target,
        );
        app.layout.borrow_mut().register(target, rect);
    }
    draw_scope_controls(frame, app, layout.controls);
}

fn draw_scope_controls(frame: &mut Frame, app: &App, area: Rect) {
    let state = &app.topology_state;
    let labels: Vec<String> = (1..=3)
        .map(|depth| {
            format!(
                "[{} hop{}{}] ",
                depth,
                if depth == 1 { "" } else { "s" },
                if state.depth == depth { " ●" } else { "" }
            )
        })
        .chain([
            format!(
                "[Retained{}] ",
                if state.cycle_scope == CycleScope::AllRetained {
                    " ●"
                } else {
                    ""
                }
            ),
            format!(
                "[Cycle{}]",
                if matches!(state.cycle_scope, CycleScope::ThisCycle(_)) {
                    " ●"
                } else {
                    ""
                }
            ),
        ])
        .collect();
    let refs: Vec<_> = labels.iter().map(String::as_str).collect();
    let buttons = [
        ButtonId::TopologyDepth1,
        ButtonId::TopologyDepth2,
        ButtonId::TopologyDepth3,
        ButtonId::TopologyScopeAll,
        ButtonId::TopologyScopeThis,
    ];
    for ((button, label), rect) in buttons
        .into_iter()
        .zip(&labels)
        .zip(components::action_rects(area, &refs))
    {
        if rect.width > 0 {
            draw_button(frame, app, button, label, rect);
        }
    }
}

fn selected_node<'a>(
    state: &TopologyState,
    snapshot: &'a TopologySnapshot,
) -> Option<&'a TopologyNode> {
    state
        .selected_node_id
        .as_deref()
        .and_then(|id| snapshot.nodes.iter().find(|node| node.id == id))
        .or_else(|| snapshot.nodes.iter().find(|node| node.is_root))
}

fn node_style(kind: TopologyNodeKind) -> (Color, &'static str) {
    match kind {
        TopologyNodeKind::Entity | TopologyNodeKind::ActorGroup => (ACCENT, "●"),
        TopologyNodeKind::Claim => (GREEN, "■"),
        TopologyNodeKind::Topic => (SERIES[6], "◆"),
        TopologyNodeKind::Inference => (SERIES[6], "◇"),
        TopologyNodeKind::Article => (DIM, "□"),
        TopologyNodeKind::Source => (MUTED, "○"),
    }
}

fn draw_graph_panel(frame: &mut Frame, app: &App, layout: &TopologyLayoutResult) {
    let state = &app.topology_state;
    let counts = state
        .snapshot
        .as_ref()
        .map(|snapshot| {
            let node_count = snapshot
                .nodes
                .iter()
                .filter(|node| node_visible(state, node))
                .count();
            format!(
                "Neighborhood · {node_count} nodes · {} relationships",
                snapshot
                    .edges
                    .iter()
                    .filter(|edge| edge_visible(state, edge.kind)
                        && visible_endpoints(state, snapshot, &edge.from, &edge.to))
                    .count()
            )
        })
        .unwrap_or_else(|| "Seeded neighborhood".into());
    frame.render_widget(
        Paragraph::new(components::clip_text(
            &counts,
            layout.graph_title.width as usize,
        ))
        .style(theme::text().add_modifier(Modifier::BOLD)),
        layout.graph_title,
    );
    let graph_block = panel(" Graph ").border_style(
        if matches!(app.focus, Target::TopologyNode(_) | Target::TopologyEdge(_)) {
            theme::accent()
        } else {
            Style::default().fg(BORDER)
        },
    );
    let inner = graph_block.inner(layout.graph_canvas);
    frame.render_widget(graph_block, layout.graph_canvas);
    app.layout
        .borrow_mut()
        .register(Target::TopologyGraph, inner);
    let Some(snapshot) = &state.snapshot else {
        let status = app
            .topology_error
            .as_deref()
            .unwrap_or(if app.topology_loading {
                "Loading seeded neighborhood…"
            } else {
                "No topology snapshot. Refresh to retry."
            });
        frame.render_widget(
            Paragraph::new(components::measured_lines(
                vec![Line::styled(status.to_owned(), theme::muted())],
                inner.width,
            )),
            inner,
        );
        draw_graph_annotations(frame, app, layout, 0, 0);
        return;
    };
    let indices: Vec<_> = snapshot
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| node_visible(state, node))
        .map(|(index, _)| index)
        .collect();
    let geometry = graph_geometry(
        inner,
        snapshot,
        &indices,
        state.zoom_percent,
        state.pan,
        state.viewport_center_id.as_deref(),
    );
    for placement in &geometry.edges {
        let edge = &snapshot.edges[placement.index];
        if !edge_visible(state, edge.kind) {
            continue;
        }
        let selected = state.selected_edge_id.as_deref() == Some(&edge.id);
        let adjacent = state.selected_edge_id.is_none()
            && state
                .selected_node_id
                .as_ref()
                .is_some_and(|id| edge.from == *id || edge.to == *id);
        let tentative = edge.kind == TopologyEdgeKind::Related;
        let color = if selected || adjacent {
            if tentative {
                WARN
            } else {
                ACCENT
            }
        } else {
            BORDER
        };
        for &(x, y, glyph) in &placement.cells {
            let glyph = if tentative && glyph == "─" {
                "┄"
            } else if tentative && glyph == "│" {
                "┊"
            } else {
                glyph
            };
            if let Some(cell) = frame.buffer_mut().cell_mut((x, y)) {
                cell.set_symbol(glyph).set_fg(color);
            }
        }
        if placement.target.width > 0 {
            app.layout
                .borrow_mut()
                .register(Target::TopologyEdge(placement.index), placement.target);
        }
    }
    for placement in &geometry.nodes {
        let node = &snapshot.nodes[placement.index];
        let selected =
            state.selected_node_id.as_deref() == Some(&node.id) && state.selected_edge_id.is_none();
        let target = Target::TopologyNode(placement.index);
        let focused = app.focus == target;
        let (color, glyph) = node_style(node.kind);
        let style = if focused {
            theme::selected()
        } else {
            theme::text()
        };
        let block = Block::default()
            .borders(if placement.rect.height >= 4 {
                Borders::ALL
            } else {
                Borders::LEFT | Borders::RIGHT
            })
            .style(Style::default().bg(BG))
            .border_style(Style::default().fg(if selected || focused { ACCENT } else { BORDER }));
        let content = if placement.rect.height >= 2 {
            let content = block.inner(placement.rect);
            frame.render_widget(block, placement.rect);
            content
        } else {
            frame.render_widget(
                Paragraph::new("").style(Style::default().bg(BG)),
                placement.rect,
            );
            placement.rect
        };
        let title = format!(
            "{glyph} {}{}",
            if node.is_root && placement.rect.height == 1 {
                "SEED "
            } else if node.is_root {
                "• "
            } else {
                ""
            },
            node.label
        );
        let kind = format!(
            "{}{} · {}h",
            if node.is_root { "SEED · " } else { "" },
            node.kind.label(),
            node.min_hops
        );
        let lines = vec![
            Line::styled(
                components::clip_text(&title, content.width as usize),
                if selected && !focused {
                    style.fg(ACCENT).add_modifier(Modifier::BOLD)
                } else {
                    style.add_modifier(Modifier::BOLD)
                },
            ),
            Line::styled(
                components::clip_text(&kind, content.width as usize),
                Style::default().fg(color),
            ),
        ];
        frame.render_widget(Paragraph::new(lines), content);
        app.layout.borrow_mut().register(target, placement.visible);
    }
    if snapshot.nodes.is_empty() {
        frame.render_widget(
            Paragraph::new("Seed record is unavailable. Return to the source or refresh.")
                .style(theme::muted()),
            inner,
        );
    }
    draw_graph_annotations(
        frame,
        app,
        layout,
        geometry.outside_viewport,
        snapshot.nodes.len().saturating_sub(indices.len()),
    );
}

fn draw_graph_annotations(
    frame: &mut Frame,
    app: &App,
    layout: &TopologyLayoutResult,
    outside: usize,
    filtered: usize,
) {
    let legend = vec![
        Line::from(vec![
            Span::styled("● Actor  ", Style::default().fg(ACCENT)),
            Span::styled("■ Claim  ", Style::default().fg(GREEN)),
            Span::styled("◆ Topic  ◇ Inference", Style::default().fg(SERIES[6])),
        ]),
        Line::from(vec![
            Span::styled("□ Article  ○ Source  ", theme::muted()),
            Span::styled(
                "→ recorded  ┄→ potential context",
                Style::default().fg(WARN),
            ),
        ]),
    ];
    frame.render_widget(Paragraph::new(legend), layout.graph_legend);
    let state = &app.topology_state;
    let scope = match &state.cycle_scope {
        CycleScope::AllRetained => "all retained".to_owned(),
        CycleScope::ThisCycle(id) => format!("cycle {id}"),
    };
    let mut lines = vec![Line::styled(
        format!(
            "Within {} hops · {scope} · {}%",
            state.depth, state.zoom_percent
        ),
        theme::muted(),
    )];
    if let Some(error) = &app.topology_error {
        lines.push(Line::styled(
            format!(
                "{} · {error}",
                if state.snapshot.is_some() {
                    "Retained snapshot"
                } else {
                    "Snapshot unavailable"
                }
            ),
            Style::default().fg(WARN),
        ));
    } else if app.topology_loading {
        lines.push(Line::styled(
            if state.snapshot.is_some() {
                "Refreshing retained snapshot…"
            } else {
                "Loading seeded neighborhood…"
            },
            theme::muted(),
        ));
    } else if let Some(snapshot) = &state.snapshot {
        let message = if outside > 0 || filtered > 0 {
            format!("{outside} outside viewport · {filtered} filtered · Shift+arrows to pan")
        } else if snapshot.omitted_nodes > 0
            || snapshot.omitted_edges > 0
            || !snapshot.exploration_exhausted
        {
            format!(
                "Bounded projection · {} nodes / {} relationships omitted",
                snapshot.omitted_nodes, snapshot.omitted_edges
            )
        } else if snapshot.nodes.len() == 1 {
            "No recorded neighbors. The seed remains visible.".into()
        } else {
            "● Seed remains fixed · selection inspects · Recenter changes scope".into()
        };
        lines.push(Line::styled(message, theme::muted()));
    }
    frame.render_widget(Paragraph::new(lines), layout.scope_findings);
}

fn detail_lines(state: &TopologyState, snapshot: &TopologySnapshot) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    if let Some(edge) = state
        .selected_edge_id
        .as_ref()
        .and_then(|id| snapshot.edges.iter().find(|edge| &edge.id == id))
    {
        let label = |id: &str| {
            snapshot
                .nodes
                .iter()
                .find(|node| node.id == id)
                .map(|node| node.label.as_str())
                .unwrap_or(id)
                .to_owned()
        };
        lines.push(Line::styled(
            format!(
                "{} → {} → {}",
                label(&edge.from),
                edge.label,
                label(&edge.to)
            ),
            theme::text().add_modifier(Modifier::BOLD),
        ));
        lines.push(Line::styled(
            format!("{} · {}", edge.kind.label(), edge.status),
            if edge.kind == TopologyEdgeKind::Related {
                Style::default().fg(WARN)
            } else {
                theme::muted()
            },
        ));
        lines.push(Line::raw(format!("Record: {}", edge.id)));
        if edge.kind == TopologyEdgeKind::Related {
            lines.push(Line::styled(
                "Potential context · not verified corroboration",
                Style::default().fg(WARN),
            ));
        }
        if !edge.rationale.is_empty() {
            lines.push(Line::raw(""));
            lines.push(Line::raw(edge.rationale.clone()));
        }
        lines.push(Line::raw(""));
        lines.push(Line::styled("Recorded basis / receipts", theme::muted()));
        if edge.receipts.is_empty() {
            lines.push(Line::raw("No retained receipt available."));
        } else {
            lines.extend(edge.receipts.iter().cloned().map(Line::raw));
        }
        if !edge.member_ids.is_empty() {
            lines.push(Line::raw(format!(
                "Members: {}",
                edge.member_ids.join(", ")
            )));
        }
    } else if let Some(node) = selected_node(state, snapshot) {
        let (color, glyph) = node_style(node.kind);
        lines.push(Line::styled(
            format!("{glyph} {}", node.label),
            theme::text().add_modifier(Modifier::BOLD),
        ));
        lines.push(Line::styled(
            format!(
                "{} · {} · {}",
                node.kind.label(),
                if node.is_root {
                    "seed".to_owned()
                } else {
                    format!("{} hops from seed", node.min_hops)
                },
                node.status
            ),
            Style::default().fg(color),
        ));
        if !node.detail.is_empty() {
            lines.push(Line::raw(""));
            lines.push(Line::raw(node.detail.clone()));
        }
        lines.push(Line::raw(""));
        lines.push(Line::styled("Record / projection", theme::muted()));
        lines.push(Line::raw(node.id.clone()));
        if !node.revision.is_empty() {
            lines.push(Line::raw(format!("Revision: {}", node.revision)));
        }
        if !node.article_id.is_empty() {
            lines.push(Line::raw(format!(
                "Article: {} / {}",
                node.run_id, node.article_id
            )));
        }
        if !node.subtype.is_empty() {
            lines.push(Line::raw(format!("Type: {}", node.subtype)));
        }
        if !node.member_ids.is_empty() {
            lines.push(Line::raw(format!(
                "Retained members: {}",
                node.member_ids.join(", ")
            )));
        }
        if !node.published_at.is_empty() {
            lines.push(Line::raw(format!("Published: {}", node.published_at)));
        }
        if let Some(predecessor) = &node.predecessor {
            lines.push(Line::raw(format!("Path via: {predecessor}")));
        }
        for passage in &node.passages {
            lines.push(Line::raw(""));
            lines.push(Line::raw(passage.clone()));
        }
    } else {
        lines.push(Line::styled(
            "Selection unavailable in this snapshot.",
            theme::muted(),
        ));
    }
    lines
}

fn draw_inspector(frame: &mut Frame, app: &App, layout: &TopologyLayoutResult) {
    let state = &app.topology_state;
    let title = if state.snapshot.is_none() {
        " Inspector "
    } else if state.selected_edge_id.is_some() {
        " Relationship · selected "
    } else {
        " Inspector · selected "
    };
    let block = panel(title).border_style(if app.focus == Target::TopologyInspector {
        theme::accent()
    } else {
        Style::default().fg(BORDER)
    });
    let inner = block.inner(layout.inspector_details);
    frame.render_widget(block, layout.inspector_details);
    if let Some(snapshot) = &state.snapshot {
        let action_labels = if state.selected_edge_id.is_some() {
            vec!["Recenter source", "Recenter target"]
        } else {
            vec!["Open record", "Recenter", "Articles"]
        };
        let action_width = inner.width.saturating_sub(2);
        let action_height =
            components::wrapped_action_rects(Rect::new(0, 0, action_width, 3), &action_labels, 1)
                .iter()
                .map(|rect| rect.bottom())
                .max()
                .unwrap_or(1)
                .min(inner.height);
        let actions = Rect::new(
            inner.x + 1,
            inner.bottom().saturating_sub(action_height),
            inner.width.saturating_sub(2),
            action_height,
        );
        let content = Rect::new(
            inner.x + 1,
            inner.y,
            inner.width.saturating_sub(2),
            inner.height.saturating_sub(action_height + 1),
        );
        let lines = components::measured_lines(detail_lines(state, snapshot), content.width);
        app.layout
            .borrow_mut()
            .register(Target::TopologyInspector, content);
        let pane = ScrollPane {
            offset: state
                .inspector_scroll
                .min(lines.len().saturating_sub(content.height as usize)),
        };
        frame.render_widget(
            Paragraph::new(pane.visible(&lines, content.height as usize).to_vec()),
            content,
        );
        for (index, rect) in components::wrapped_action_rects(actions, &action_labels, 1)
            .into_iter()
            .enumerate()
        {
            if rect.height == 0 || rect.width == 0 {
                continue;
            }
            if state.selected_edge_id.is_some() {
                let target = Target::TopologyEndpoint(index);
                frame.render_widget(
                    Paragraph::new(format!("[{}]", action_labels[index])).style(
                        if app.focus == target {
                            theme::selected()
                        } else {
                            theme::accent()
                        },
                    ),
                    rect,
                );
                app.layout.borrow_mut().register(target, rect);
            } else {
                let button = [
                    ButtonId::TopologyOpenMemory,
                    ButtonId::TopologyRecenter,
                    ButtonId::TopologyViewEvidence,
                ][index];
                draw_button(frame, app, button, action_labels[index], rect);
            }
        }
    } else {
        frame.render_widget(
            Paragraph::new(components::measured_lines(
                vec![Line::styled(
                    if app.topology_error.is_some() {
                        "Snapshot unavailable. Press r to retry."
                    } else {
                        "Waiting for the seeded snapshot…"
                    },
                    theme::muted(),
                )],
                inner.width,
            )),
            inner,
        );
    }
    draw_relationships(frame, app, layout.inspector_related);
}

fn draw_relationships(frame: &mut Frame, app: &App, area: Rect) {
    let block = panel(" Relationships ").border_style(
        if matches!(
            app.focus,
            Target::TopologyRelationships | Target::TopologyRelated(_)
        ) {
            theme::accent()
        } else {
            Style::default().fg(BORDER)
        },
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let tabs = ["All", "Articles"];
    for (index, rect) in components::tab_rects(
        Rect::new(inner.x, inner.y, inner.width, 1),
        &tabs,
        app.topology_state.inspector_tab,
    )
    .into_iter()
    .enumerate()
    {
        if rect.width == 0 {
            continue;
        }
        let target = Target::TopologyInspectorTab(index);
        components::tab_button(
            frame,
            rect,
            tabs[index],
            app.topology_state.inspector_tab == index,
            app.focus == target,
        );
        app.layout.borrow_mut().register(target, rect);
    }
    let content = Rect::new(
        inner.x + 1,
        inner.y + 2,
        inner.width.saturating_sub(2),
        inner.height.saturating_sub(2),
    );
    app.layout
        .borrow_mut()
        .register(Target::TopologyRelationships, content);
    let Some(snapshot) = &app.topology_state.snapshot else {
        frame.render_widget(
            Paragraph::new("Snapshot unavailable.").style(theme::muted()),
            content,
        );
        return;
    };
    let indices = related_edge_indices(&app.topology_state);
    if indices.is_empty() {
        frame.render_widget(
            Paragraph::new("No recorded relationships.").style(theme::muted()),
            content,
        );
        return;
    }
    let anchor = app
        .topology_state
        .selected_node_id
        .as_deref()
        .unwrap_or(&snapshot.root_id);
    let records: Vec<_> = indices
        .iter()
        .map(|&index| {
            let edge = &snapshot.edges[index];
            let outgoing = edge.from == anchor;
            let other_id = if outgoing { &edge.to } else { &edge.from };
            let other = snapshot
                .nodes
                .iter()
                .find(|node| &node.id == other_id)
                .map(|node| node.label.as_str())
                .unwrap_or(other_id);
            let title = if outgoing {
                format!("{} → {other}", edge.label)
            } else {
                format!("← {} · {other}", edge.label)
            };
            components::measured_lines(
                vec![
                    Line::raw(title),
                    Line::styled(
                        if edge.kind == TopologyEdgeKind::Related {
                            format!("Potential context · {}", edge.status)
                        } else {
                            edge.status.clone()
                        },
                        theme::muted(),
                    ),
                ],
                content.width,
            )
        })
        .collect();
    let geometry = ListGeometry::new(records.iter().map(Vec::len));
    let logical_offset = app
        .topology_state
        .related_scroll
        .min(records.len().saturating_sub(1));
    let offset = geometry
        .items
        .get(logical_offset)
        .map(|range| range.start)
        .unwrap_or(0);
    for (logical_index, lines) in records.iter().enumerate().skip(logical_offset) {
        let range = &geometry.items[logical_index];
        let y = range.start.saturating_sub(offset);
        if y >= content.height as usize {
            break;
        }
        let height = lines.len().min(content.height as usize - y);
        let rect = Rect::new(
            content.x,
            content.y + y as u16,
            content.width,
            height as u16,
        );
        let selected = app.topology_state.selected_edge_id.as_deref()
            == Some(&snapshot.edges[indices[logical_index]].id);
        let focused = app.focus == Target::TopologyRelated(logical_index);
        let mut visible = lines[..height].to_vec();
        if selected || focused {
            for line in &mut visible {
                *line = line.clone().style(if focused {
                    theme::selected()
                } else {
                    theme::accent()
                });
            }
        }
        frame.render_widget(Paragraph::new(visible), rect);
        app.layout
            .borrow_mut()
            .register(Target::TopologyRelated(logical_index), rect);
    }
}

fn draw_controls_page(frame: &mut Frame, app: &App, area: Rect) {
    let block = panel(" Topology controls ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let inner = Rect::new(
        inner.x + 1,
        inner.y,
        inner.width.saturating_sub(2),
        inner.height,
    );
    let mut y = inner.y;
    frame.render_widget(
        Paragraph::new("Viewport · pan with Shift+arrows; zoom changes actual positions")
            .style(theme::muted()),
        Rect::new(inner.x, y, inner.width, 1),
    );
    y += 1;
    let labels = ["Center selected", "Reset view", "− Zoom", "+ Zoom"];
    let buttons = [
        ButtonId::TopologyCenterSelected,
        ButtonId::TopologyFitGraph,
        ButtonId::TopologyZoomOut,
        ButtonId::TopologyZoomIn,
    ];
    let action_area = Rect::new(
        inner.x,
        y,
        inner.width,
        2.min(inner.bottom().saturating_sub(y)),
    );
    let rects = components::wrapped_action_rects(action_area, &labels, 1);
    for ((button, label), rect) in buttons.into_iter().zip(labels).zip(&rects) {
        if rect.height > 0 {
            draw_button(frame, app, button, label, *rect);
        }
    }
    y = rects.iter().map(|rect| rect.bottom()).max().unwrap_or(y) + 1;
    if y >= inner.bottom() {
        return;
    }
    frame.render_widget(
        Paragraph::new("Visible recorded kinds · seed always retained").style(theme::muted()),
        Rect::new(inner.x, y, inner.width, 1),
    );
    y += 1;
    let filter_names = [
        "Actors",
        "Claims",
        "Topics/inferences",
        "Links",
        "Related",
        "Articles/sources",
    ];
    let filters = app.topology_state.filters();
    let labels: Vec<_> = filter_names
        .iter()
        .zip(filters)
        .map(|(name, enabled)| format!("[{}] {name}", if enabled { "x" } else { " " }))
        .collect();
    let refs: Vec<_> = labels.iter().map(String::as_str).collect();
    let rects = components::wrapped_action_rects(
        Rect::new(inner.x, y, inner.width, inner.bottom().saturating_sub(y)),
        &refs,
        1,
    );
    for (index, rect) in rects.iter().enumerate() {
        if rect.height == 0 || rect.width == 0 {
            continue;
        }
        let target = Target::TopologyFilter(index);
        frame.render_widget(
            Paragraph::new(labels[index].clone()).style(if app.focus == target {
                theme::selected()
            } else {
                theme::accent()
            }),
            *rect,
        );
        app.layout.borrow_mut().register(target, *rect);
    }
    y = rects
        .iter()
        .filter(|rect| rect.height > 0)
        .map(|rect| rect.bottom())
        .max()
        .unwrap_or(y)
        + 1;
    if y < inner.bottom() {
        let rect = Rect::new(inner.x, y, inner.width.min(20), 1);
        draw_button_state(
            frame,
            app,
            ButtonId::TopologyAnalyzeScope,
            "Analyze scope",
            rect,
            app.topology_state.analyzing_scope,
        );
        y += 1;
    }
    if y < inner.bottom() {
        frame.render_widget(
            Paragraph::new(app.topology_state.scope_status.clone()).style(theme::muted()),
            Rect::new(inner.x, y, inner.width, 1),
        );
        y += 2;
    }
    if y < inner.bottom() {
        let mut lines = vec![Line::styled("Projection coverage", theme::accent())];
        if let Some(snapshot) = &app.topology_state.snapshot {
            lines.push(Line::raw(format!(
                "{} displayed nodes · {} displayed relationships",
                snapshot.nodes.len(),
                snapshot.edges.len()
            )));
            lines.push(Line::raw(format!(
                "{} discovered nodes · {} discovered relationships (bounded)",
                snapshot.discovered_nodes, snapshot.discovered_edges
            )));
            lines.push(Line::raw(format!(
                "{} omitted nodes · {} omitted relationships · {} unresolved endpoints",
                snapshot.omitted_nodes,
                snapshot.omitted_edges,
                snapshot.unresolved_endpoints.len()
            )));
            lines.extend(
                snapshot
                    .findings
                    .gaps
                    .iter()
                    .cloned()
                    .map(|text| Line::styled(text, Style::default().fg(WARN))),
            );
        } else {
            lines.push(Line::raw("Snapshot unavailable."));
        }
        let rows = components::measured_lines(lines, inner.width);
        frame.render_widget(
            Paragraph::new(rows),
            Rect::new(inner.x, y, inner.width, inner.bottom().saturating_sub(y)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retained_origin_supplies_cycle_when_canonical_root_has_no_cycle() {
        let state = TopologyState {
            seed: Some(TopologySeed::RetainedActor {
                run_id: "cycle-42".into(),
                article_id: "article-7".into(),
                revision: "revision-3".into(),
                actor_ref: "local-a1".into(),
                name: "Retained source actor".into(),
            }),
            snapshot: Some(TopologySnapshot::default()),
            ..Default::default()
        };
        assert_eq!(state.current_cycle_id().as_deref(), Some("cycle-42"));
    }

    #[test]
    fn topology_history_preserves_view_and_filters() {
        let mut state = TopologyState {
            seed: Some(TopologySeed::Claim("c1".into())),
            active_panel_tab: 2,
            inspector_scroll: 8,
            related_scroll: 3,
            filter_related: false,
            ..Default::default()
        };
        state.center_selected();
        state.push_history();
        let saved = state.pop_history().unwrap();
        assert_eq!(saved.seed, TopologySeed::Claim("c1".into()));
        assert_eq!(saved.active_panel_tab, 2);
        assert_eq!(saved.inspector_scroll, 8);
        assert_eq!(saved.related_scroll, 3);
        assert!(!saved.filters[4]);
    }
}
