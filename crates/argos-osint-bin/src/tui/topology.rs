//! Brain Memory Topology interactive view (spec §2 & mockup).
//!
//! Three-panel reactive topology mapper: Left inspector & related records,
//! Central graph canvas with anchored legend & scope findings, Right navigation
//! & controls with minimap.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};
use ratatui::Frame;

use argos_osint_core::brain_topology::{
    CycleScope, TopologyNodeKind, TopologySeed, TopologySnapshot,
};

use super::app::{App, ButtonId, Target};
use super::theme::{self, panel, ACCENT, BG, BORDER, DIM, GREEN, MUTED, SERIES, TEXT, WARN};
use super::topology_layout::{TopologyLayoutResult, TopologyResponsiveMode};
use super::ui::draw_button;

pub const TOPOLOGY_HISTORY_CAP: usize = 32;

#[derive(Clone, Debug)]
pub struct TopologyHistoryFrame {
    pub seed: TopologySeed,
    pub selected_node_id: Option<String>,
    pub selected_edge_id: Option<String>,
    pub depth: usize,
    pub cycle_scope: CycleScope,
    pub zoom_percent: usize,
    pub pan: (i16, i16),
    pub inspector_tab: usize,
    pub related_sel: usize,
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
    pub active_panel_tab: usize,
    pub inspector_tab: usize,
    pub related_sel: usize,
    pub related_scroll: usize,
    pub analyzing_scope: bool,
    pub scope_status: String,
    pub history: Vec<TopologyHistoryFrame>,
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
            active_panel_tab: 0,
            inspector_tab: 0,
            related_sel: 0,
            related_scroll: 0,
            analyzing_scope: false,
            scope_status: "On demand • idle".into(),
            history: Vec::new(),
        }
    }
}

impl TopologyState {
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
                inspector_tab: self.inspector_tab,
                related_sel: self.related_sel,
            });
        }
    }

    pub fn pop_history(&mut self) -> Option<TopologyHistoryFrame> {
        self.history.pop()
    }

    pub fn current_cycle_id(&self) -> Option<String> {
        if let Some(snap) = &self.snapshot {
            if let Some(sel_id) = &self.selected_node_id {
                if let Some(node) = snap.nodes.iter().find(|n| &n.id == sel_id) {
                    if let Some(c) = &node.cycle {
                        return Some(c.clone());
                    }
                }
            }
            for node in &snap.nodes {
                if let Some(c) = &node.cycle {
                    return Some(c.clone());
                }
            }
        }
        None
    }

    pub fn center_selected(&mut self) {
        self.pan = (0, 0);
    }
}

pub fn draw_topology(frame: &mut Frame, app: &App, area: Rect) {
    let layout = TopologyLayoutResult::compute(area);
    if layout.mode == TopologyResponsiveMode::Tiny {
        draw_tiny_guidance(frame, area);
        return;
    }

    // 1. Top breadcrumbs & subheader
    draw_top_bar(frame, app, &layout);

    // 2. Main content panels
    match layout.mode {
        TopologyResponsiveMode::Desktop => {
            draw_inspector(frame, app, &layout);
            draw_graph_panel(frame, app, &layout);
            draw_navigation(frame, app, &layout);
        }
        TopologyResponsiveMode::Compact => {
            draw_inspector(frame, app, &layout);
            draw_graph_panel(frame, app, &layout);
        }
        TopologyResponsiveMode::Stacked => match app.topology_state.active_panel_tab {
            0 => draw_graph_panel(frame, app, &layout),
            1 => draw_inspector(frame, app, &layout),
            _ => draw_navigation(frame, app, &layout),
        },
        TopologyResponsiveMode::Tiny => {}
    }

    // 3. Footer help line
    draw_footer(frame, &layout);
}

fn draw_tiny_guidance(frame: &mut Frame, area: Rect) {
    let msg = vec![
        Line::from(Span::styled(
            "Topology View",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            "Terminal too small. Minimum viewport: 60×18 cells.",
            Style::default().fg(MUTED),
        )),
        Line::from(Span::styled(
            "Resize window to view interactive graph.",
            Style::default().fg(DIM),
        )),
    ];
    frame.render_widget(Paragraph::new(msg).block(panel(" Topology ")), area);
}

fn draw_top_bar(frame: &mut Frame, app: &App, layout: &TopologyLayoutResult) {
    // Row 1: Breadcrumbs
    let root_label = app
        .topology_state
        .snapshot
        .as_ref()
        .map(|s| s.root_label.as_str())
        .unwrap_or("Brain");
    let crumbs = Line::from(vec![
        Span::styled(
            "ARGOS OSINT",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ),
        Span::styled("  │  ", Style::default().fg(BORDER)),
        Span::styled(
            format!("Brain / {root_label} / Topology"),
            Style::default().fg(DIM),
        ),
    ]);
    frame.render_widget(Paragraph::new(crumbs), layout.header);

    // Row 2: Subheader with Back, Title, Root indicator, and Search
    let back_btn_rect = Rect::new(layout.subheader.x, layout.subheader.y, 10, 1);
    app.layout
        .borrow_mut()
        .register(Target::Button(ButtonId::TopologyBack), back_btn_rect);
    let back_style = if app.focus == Target::Button(ButtonId::TopologyBack) {
        theme::selected()
    } else {
        Style::default().fg(ACCENT)
    };
    frame.render_widget(
        Paragraph::new(Span::styled("[← Back]", back_style)),
        back_btn_rect,
    );

    let title_span = Span::styled(
        " │ Brain Memory Topology",
        Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
    );
    let title_rect = Rect::new(layout.subheader.x + 10, layout.subheader.y, 25, 1);
    frame.render_widget(Paragraph::new(title_span), title_rect);

    let root_id = app
        .topology_state
        .snapshot
        .as_ref()
        .map(|s| s.root_id.as_str())
        .unwrap_or("A-001");
    let root_info = format!("Root: {} {}  │  [ ⚲ Find memory... ]", root_id, root_label);
    let info_span = Span::styled(root_info, Style::default().fg(MUTED));
    let info_w = 45u16.min(layout.subheader.width.saturating_sub(35));
    let info_rect = Rect::new(
        layout.subheader.x + layout.subheader.width.saturating_sub(info_w),
        layout.subheader.y,
        info_w,
        1,
    );
    frame.render_widget(Paragraph::new(info_span), info_rect);

    // Row 3: Seed kind tabs and/or panel tabs in stacked mode
    let mut tab_spans = Vec::new();
    if layout.mode == TopologyResponsiveMode::Stacked {
        let panel_tabs = ["[Graph]", "[Details]", "[Navigation]"];
        for (i, label) in panel_tabs.iter().enumerate() {
            if i > 0 {
                tab_spans.push(Span::raw("  "));
            }
            let is_active = app.topology_state.active_panel_tab == i;
            let style = if is_active {
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(MUTED)
            };
            let text = if is_active {
                format!("[{} ●]", &label[1..label.len() - 1])
            } else {
                (*label).to_string()
            };
            tab_spans.push(Span::styled(text, style));
        }
        tab_spans.push(Span::styled("   │   ", Style::default().fg(BORDER)));
    }

    let seed_kinds = ["[Actor ●]", "[Claim]", "[Inference]", "[Link]"];
    for (i, label) in seed_kinds.iter().enumerate() {
        if i > 0 || layout.mode == TopologyResponsiveMode::Stacked {
            tab_spans.push(Span::raw("  "));
        }
        let style = if i == 0 {
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(MUTED)
        };
        tab_spans.push(Span::styled(*label, style));
    }
    frame.render_widget(Paragraph::new(Line::from(tab_spans)), layout.seed_tabs);
}

fn draw_inspector(frame: &mut Frame, app: &App, layout: &TopologyLayoutResult) {
    if layout.left_panel.width == 0 {
        return;
    }

    // Top: Memory / Edge details
    let block_title = if app.topology_state.selected_edge_id.is_some() {
        "Memory details • Link"
    } else {
        "Memory details • Entity"
    };

    let mut details_lines = Vec::new();
    if let Some(edge_id) = &app.topology_state.selected_edge_id {
        let edge = app
            .topology_state
            .snapshot
            .as_ref()
            .and_then(|s| s.edges.iter().find(|e| &e.id == edge_id));
        let label = edge.map(|e| e.label.as_str()).unwrap_or("supports");
        details_lines.push(Line::from(vec![
            Span::styled("➔ ", Style::default().fg(ACCENT)),
            Span::styled(
                format!("{edge_id} • {label}"),
                Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
            ),
        ]));
        details_lines.push(Line::raw(""));
        details_lines.push(Line::from(vec![
            Span::styled("Kind:         ", Style::default().fg(MUTED)),
            Span::styled("Link", Style::default().fg(TEXT)),
        ]));
        details_lines.push(Line::from(vec![
            Span::styled("From:         ", Style::default().fg(MUTED)),
            Span::styled(
                edge.map(|e| e.from.as_str()).unwrap_or("C-112"),
                Style::default().fg(TEXT),
            ),
        ]));
        details_lines.push(Line::from(vec![
            Span::styled("To:           ", Style::default().fg(MUTED)),
            Span::styled(
                edge.map(|e| e.to.as_str()).unwrap_or("C-104"),
                Style::default().fg(TEXT),
            ),
        ]));
        details_lines.push(Line::from(vec![
            Span::styled("Status:       ", Style::default().fg(MUTED)),
            Span::styled("assessed", Style::default().fg(GREEN)),
        ]));
        details_lines.push(Line::from(vec![
            Span::styled("Actor:        ", Style::default().fg(MUTED)),
            Span::styled("A-001 Northstar Labs", Style::default().fg(ACCENT)),
        ]));
        details_lines.push(Line::from(vec![
            Span::styled("Evidence:     ", Style::default().fg(MUTED)),
            Span::styled("2 original sources", Style::default().fg(TEXT)),
        ]));
        details_lines.push(Line::from(vec![
            Span::styled("Source roots: ", Style::default().fg(MUTED)),
            Span::styled("2", Style::default().fg(TEXT)),
        ]));
        details_lines.push(Line::raw(""));
        details_lines.push(Line::styled("Source passage", Style::default().fg(MUTED)));
        let rationale = edge
            .map(|e| e.rationale.as_str())
            .filter(|r| !r.is_empty())
            .unwrap_or("\"The board approved the pilot.\"");
        details_lines.push(Line::styled(
            format!("\"{rationale}\""),
            Style::default().fg(DIM),
        ));
        details_lines.push(Line::styled(
            "Supply Review • 13 Oct 2026",
            Style::default().fg(MUTED),
        ));
    } else {
        // Node details
        let node_id = app
            .topology_state
            .selected_node_id
            .as_deref()
            .unwrap_or("A-001");
        let node = app
            .topology_state
            .snapshot
            .as_ref()
            .and_then(|s| s.nodes.iter().find(|n| n.id == node_id));

        let label = node.map(|n| n.label.as_str()).unwrap_or("Northstar Labs");
        let kind = node.map(|n| n.kind.label()).unwrap_or("Entity");
        let subtype = node.map(|n| n.subtype.as_str()).unwrap_or("organization");
        let status = node.map(|n| n.status.as_str()).unwrap_or("resolved");

        details_lines.push(Line::from(vec![
            Span::styled("● ", Style::default().fg(ACCENT)),
            Span::styled(
                format!("{node_id} • {label}"),
                Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
            ),
        ]));
        details_lines.push(Line::raw(""));
        details_lines.push(Line::from(vec![
            Span::styled("Kind:         ", Style::default().fg(MUTED)),
            Span::styled(kind, Style::default().fg(TEXT)),
        ]));
        details_lines.push(Line::from(vec![
            Span::styled("Subtype:      ", Style::default().fg(MUTED)),
            Span::styled(subtype, Style::default().fg(TEXT)),
        ]));
        details_lines.push(Line::from(vec![
            Span::styled("Status:       ", Style::default().fg(MUTED)),
            Span::styled(status, Style::default().fg(GREEN)),
        ]));
        details_lines.push(Line::from(vec![
            Span::styled("Evidence:     ", Style::default().fg(MUTED)),
            Span::styled("4 linked claims", Style::default().fg(TEXT)),
        ]));
        details_lines.push(Line::raw(""));
        if let Some(n) = node {
            if !n.passages.is_empty() {
                details_lines.push(Line::styled("Passage", Style::default().fg(MUTED)));
                details_lines.push(Line::styled(
                    format!("\"{}\"", n.passages[0]),
                    Style::default().fg(DIM),
                ));
            }
        }
    }

    let p = Paragraph::new(details_lines).block(panel(block_title));
    frame.render_widget(p, layout.inspector_details);

    // Buttons at bottom of details
    let btn_y = layout.inspector_details.y + layout.inspector_details.height.saturating_sub(2);
    let btn1 = Rect::new(layout.inspector_details.x + 2, btn_y, 15, 1);
    let btn2 = Rect::new(layout.inspector_details.x + 19, btn_y, 16, 1);
    draw_button(
        frame,
        app,
        ButtonId::TopologyOpenMemory,
        "[Open memory]",
        btn1,
    );
    draw_button(
        frame,
        app,
        ButtonId::TopologyViewEvidence,
        "[View evidence]",
        btn2,
    );

    // Bottom: Related to selection
    draw_related_to_selection(frame, app, layout.inspector_related);
}

fn draw_related_to_selection(frame: &mut Frame, app: &App, area: Rect) {
    let mut lines = Vec::new();

    // Tabs: [Memories ●]   [Articles]
    let t1_style = if app.topology_state.inspector_tab == 0 {
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(MUTED)
    };
    let t2_style = if app.topology_state.inspector_tab == 1 {
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(MUTED)
    };
    lines.push(Line::from(vec![
        Span::styled("[Memories ●]", t1_style),
        Span::raw("   "),
        Span::styled("[Articles]", t2_style),
    ]));
    lines.push(Line::raw(""));

    // Spaced items list with LIST_ITEM_GAP_ROWS = 1
    let items = [
        ("■", GREEN, "C-104 • Pilot approved", "Claim"),
        ("■", GREEN, "C-112 • Approval confirmed", "Claim"),
        ("◆", SERIES[6], "I-027 • Supply-chain risk", "Inference"),
        (
            "☼",
            WARN,
            "RL-008 • Vendor article lead",
            "Related • unverified",
        ),
    ];

    let skip = app.topology_state.related_scroll;
    let start_y = area.y.saturating_add(3);
    for (i, (glyph, color, title, subtitle)) in items.iter().skip(skip).enumerate() {
        let actual_idx = i + skip;
        let is_sel = app.topology_state.related_sel == actual_idx;
        let title_style = if is_sel {
            theme::selected()
        } else {
            Style::default().fg(TEXT).add_modifier(Modifier::BOLD)
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{glyph} "), Style::default().fg(*color)),
            Span::styled(*title, title_style),
            Span::styled("  >", Style::default().fg(MUTED)),
        ]));
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(*subtitle, Style::default().fg(MUTED)),
        ]));
        // LIST_ITEM_GAP_ROWS = 1
        lines.push(Line::raw(""));

        let row_y = start_y + (i as u16) * 3;
        if row_y + 1 < area.y + area.height {
            let row_rect = Rect::new(area.x + 1, row_y, area.width.saturating_sub(2), 2);
            app.layout
                .borrow_mut()
                .register(Target::TopologyRelated(actual_idx), row_rect);
        }
    }

    lines.push(Line::styled("See more ↓", Style::default().fg(ACCENT)));

    let p = Paragraph::new(lines).block(panel("Related to selection"));
    frame.render_widget(p, area);
}

fn draw_graph_panel(frame: &mut Frame, app: &App, layout: &TopologyLayoutResult) {
    // 1. Graph Title
    let title_line = Line::from(vec![
        Span::styled(
            "Scoped relationships",
            Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" • ", Style::default().fg(MUTED)),
        Span::styled("focused", Style::default().fg(ACCENT)),
    ]);
    frame.render_widget(Paragraph::new(title_line), layout.graph_title);

    // 2. Graph Canvas with Nodes and Edges
    draw_graph_canvas(frame, app, layout.graph_canvas);

    // 3. Anchored Legend
    draw_legend(frame, layout.graph_legend);

    // 4. Scope Findings
    draw_scope_findings(frame, app, layout.scope_findings);
}

fn draw_graph_canvas(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(Style::default().fg(BORDER))
        .style(Style::default().bg(BG));
    frame.render_widget(block, area);
    app.layout
        .borrow_mut()
        .register(Target::TopologyGraph, area);

    let inner = Rect::new(
        area.x + 1,
        area.y + 1,
        area.width.saturating_sub(2),
        area.height.saturating_sub(2),
    );
    if inner.width < 20 || inner.height < 6 {
        return;
    }

    // Positions for canonical nodes (proportional coordinates across canvas)
    let cx = inner.x + inner.width / 2;
    let cy = inner.y + inner.height / 2;

    let v_scale = (inner.height as f32 / 36.0).clamp(0.25, 1.0);
    let scale_y = |offset: i16| -> u16 {
        let scaled = (offset as f32 * v_scale).round() as i16;
        let y = cy as i16 + scaled;
        y.clamp(
            inner.y as i16,
            (inner.y + inner.height.saturating_sub(3)) as i16,
        ) as u16
    };

    let h_scale = (inner.width as f32 / 80.0).clamp(0.35, 1.0);
    let scale_x = |offset: i16| -> u16 {
        let scaled = (offset as f32 * h_scale).round() as i16;
        let x = cx as i16 + scaled;
        x.clamp(
            inner.x as i16,
            (inner.x + inner.width.saturating_sub(20)) as i16,
        ) as u16
    };

    // Node layout offsets relative to center
    // Root: A-001 (center)
    let n_root = (scale_x(-4), scale_y(0));
    // A-002 Maya Chen (top left)
    let n_a002 = (scale_x(-25), scale_y(-10));
    // A-003 Harbor Systems (top right)
    let n_a003 = (scale_x(18), scale_y(-10));
    // C-111 Leads pilot (mid left-top)
    let n_c111 = (scale_x(-23), scale_y(-4));
    // C-112 Approval confirmed (mid left)
    let n_c112 = (scale_x(-23), scale_y(2));
    // C-104 Pilot approved (lower left)
    let n_c104 = (scale_x(-23), scale_y(8));
    // C-109 Vendor selected (mid right)
    let n_c109 = (scale_x(18), scale_y(-3));
    // I-027 Supply-chain risk (lower center-right)
    let n_i027 = (scale_x(18), scale_y(6));
    // ART-023 (far left top)
    let n_art023 = (inner.x + 2, scale_y(-8));
    // ART-029 (far left mid)
    let n_art029 = (inner.x + 2, scale_y(-1));
    // ART-018 (far left lower)
    let n_art018 = (inner.x + 2, scale_y(7));
    // ART-021 (far right top)
    let n_art021 = (inner.x + inner.width.saturating_sub(18), scale_y(-6));
    // ART-028 (far right lower)
    let n_art028 = (inner.x + inner.width.saturating_sub(18), scale_y(9));
    // REP-012 (lower mid-right)
    let n_rep012 = (scale_x(12), scale_y(12));

    // Draw Edges (connecting arrows / pills)
    draw_edge_line(frame, app, 0, n_a002, n_c111, "concerns", ACCENT, false);
    draw_edge_line(frame, app, 1, n_a002, n_art023, "mentioned_in", MUTED, true);
    draw_edge_line(frame, app, 2, n_c111, n_root, "concerns", ACCENT, false);
    draw_edge_line(frame, app, 3, n_c112, n_root, "concerns", ACCENT, false);
    draw_edge_line(
        frame,
        app,
        4,
        n_c112,
        n_c104,
        "L-014 • supports",
        ACCENT,
        false,
    );
    draw_edge_line(frame, app, 5, n_root, n_c109, "concerns", ACCENT, false);
    draw_edge_line(frame, app, 6, n_a003, n_c109, "concerns", ACCENT, false);
    draw_edge_line(frame, app, 7, n_a003, n_art021, "mentioned_in", MUTED, true);
    draw_edge_line(frame, app, 8, n_c109, n_i027, "premise", SERIES[6], true);
    draw_edge_line(frame, app, 9, n_c104, n_i027, "premise", SERIES[6], true);
    draw_edge_line(
        frame,
        app,
        10,
        n_i027,
        n_art028,
        "RL-008 • possible context",
        WARN,
        true,
    );
    draw_edge_line(frame, app, 11, n_i027, n_rep012, "contains", MUTED, true);
    draw_edge_line(frame, app, 12, n_art023, n_c111, "source", MUTED, false);
    draw_edge_line(frame, app, 13, n_art029, n_c112, "source", MUTED, false);
    draw_edge_line(frame, app, 14, n_art018, n_c104, "source", MUTED, false);
    draw_edge_line(frame, app, 15, n_c109, n_art021, "source", MUTED, false);

    // Draw Nodes
    draw_node(
        frame,
        app,
        0,
        n_root,
        "A-001",
        "Northstar Labs",
        "organization • resolved",
        TopologyNodeKind::Entity,
        true,
    );
    draw_node(
        frame,
        app,
        1,
        n_a002,
        "A-002",
        "Maya Chen",
        "person • resolved",
        TopologyNodeKind::Entity,
        false,
    );
    draw_node(
        frame,
        app,
        2,
        n_a003,
        "A-003",
        "Harbor Systems",
        "organization • resolved",
        TopologyNodeKind::Entity,
        false,
    );
    draw_node(
        frame,
        app,
        3,
        n_c111,
        "C-111",
        "Leads pilot",
        "claim • assessed",
        TopologyNodeKind::Claim,
        false,
    );
    draw_node(
        frame,
        app,
        4,
        n_c112,
        "C-112",
        "Approval confirmed",
        "claim • assessed",
        TopologyNodeKind::Claim,
        false,
    );
    draw_node(
        frame,
        app,
        5,
        n_c104,
        "C-104",
        "Pilot approved",
        "claim • assessed",
        TopologyNodeKind::Claim,
        false,
    );
    draw_node(
        frame,
        app,
        6,
        n_c109,
        "C-109",
        "Vendor selected",
        "claim • assessed",
        TopologyNodeKind::Claim,
        false,
    );
    draw_node(
        frame,
        app,
        7,
        n_i027,
        "I-027",
        "Supply-chain risk",
        "inference • assessed",
        TopologyNodeKind::Topic,
        false,
    );
    draw_node(
        frame,
        app,
        8,
        n_art023,
        "ART-023",
        "Leadership",
        "12 Oct • cycle 12",
        TopologyNodeKind::Article,
        false,
    );
    draw_node(
        frame,
        app,
        9,
        n_art029,
        "ART-029",
        "Pilot follow-up",
        "13 Oct • cycle 13",
        TopologyNodeKind::Article,
        false,
    );
    draw_node(
        frame,
        app,
        10,
        n_art018,
        "ART-018",
        "Port pilot",
        "12 Oct • cycle 12",
        TopologyNodeKind::Article,
        false,
    );
    draw_node(
        frame,
        app,
        11,
        n_art021,
        "ART-021",
        "Vendor report",
        "11 Oct • cycle 11",
        TopologyNodeKind::Article,
        false,
    );
    draw_node(
        frame,
        app,
        12,
        n_art028,
        "ART-028",
        "Vendor background",
        "01 Oct • cycle 1",
        TopologyNodeKind::Article,
        false,
    );
    draw_node(
        frame,
        app,
        13,
        n_rep012,
        "REP-012",
        "Supply review",
        "13 Oct • cycle 13",
        TopologyNodeKind::Article,
        false,
    );
}

#[allow(clippy::too_many_arguments)]
fn draw_node(
    frame: &mut Frame,
    app: &App,
    node_idx: usize,
    pos: (u16, u16),
    id: &str,
    label: &str,
    subtitle: &str,
    kind: TopologyNodeKind,
    is_root: bool,
) {
    let (color, glyph) = match kind {
        TopologyNodeKind::Entity => (ACCENT, "●"),
        TopologyNodeKind::Claim => (GREEN, "■"),
        TopologyNodeKind::Topic => (SERIES[6], "◆"),
        TopologyNodeKind::Article => (DIM, "□"),
        TopologyNodeKind::Source => (MUTED, "○"),
    };

    let is_sel = app.topology_state.selected_node_id.as_deref() == Some(id);
    let title_style = if is_sel {
        theme::selected()
    } else {
        Style::default().fg(TEXT).add_modifier(Modifier::BOLD)
    };

    let title_line = Line::from(vec![
        Span::styled(format!("{glyph} "), Style::default().fg(color)),
        Span::styled(id, Style::default().fg(MUTED)),
        Span::raw(" "),
        Span::styled(label, title_style),
    ]);

    let sub_line = Line::from(vec![
        Span::raw("  "),
        Span::styled(subtitle, Style::default().fg(color)),
    ]);

    let w = 22u16;
    let h = if is_root { 3u16 } else { 2u16 };
    let rect = Rect::new(pos.0, pos.1, w, h);
    app.layout
        .borrow_mut()
        .register(Target::TopologyNode(node_idx), rect);

    let mut lines = vec![title_line, sub_line];
    if is_root {
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                "[root]",
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
        ]));
    }
    frame.render_widget(Paragraph::new(lines), rect);
}

#[allow(clippy::too_many_arguments)]
fn draw_edge_line(
    frame: &mut Frame,
    app: &App,
    edge_idx: usize,
    from: (u16, u16),
    to: (u16, u16),
    label: &str,
    color: Color,
    dashed: bool,
) {
    let mid_x = (from.0 + to.0) / 2;
    let mid_y = (from.1 + to.1) / 2;

    let pill_text = if dashed {
        format!("┄ {label} ┄▶")
    } else {
        format!("─ {label} ─▶")
    };
    let pill_w = pill_text.chars().count() as u16;
    let pill_rect = Rect::new(mid_x.saturating_sub(pill_w / 2), mid_y, pill_w, 1);
    app.layout
        .borrow_mut()
        .register(Target::TopologyEdge(edge_idx), pill_rect);
    frame.render_widget(
        Paragraph::new(Span::styled(pill_text, Style::default().fg(color))),
        pill_rect,
    );
}

fn draw_legend(frame: &mut Frame, area: Rect) {
    let line1 = Line::from(vec![
        Span::styled("● ", Style::default().fg(ACCENT)),
        Span::styled("Actor   ", Style::default().fg(TEXT)),
        Span::styled("■ ", Style::default().fg(GREEN)),
        Span::styled("Claim   ", Style::default().fg(TEXT)),
        Span::styled("◆ ", Style::default().fg(SERIES[6])),
        Span::styled("Inference   ", Style::default().fg(TEXT)),
        Span::styled("➔ ", Style::default().fg(ACCENT)),
        Span::styled("Link (recorded)   ", Style::default().fg(TEXT)),
        Span::styled("┄┄➔ ", Style::default().fg(WARN)),
        Span::styled("Related (tentative)", Style::default().fg(TEXT)),
    ]);

    let line2 = Line::from(vec![
        Span::styled("□ ", Style::default().fg(DIM)),
        Span::styled(
            "Article / Report = source reference",
            Style::default().fg(MUTED),
        ),
    ]);

    let p = Paragraph::new(vec![line1, line2]).block(
        Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(BORDER)),
    );
    frame.render_widget(p, area);
}

fn draw_scope_findings(frame: &mut Frame, app: &App, area: Rect) {
    let mut lines = Vec::new();
    if let Some(snap) = &app.topology_state.snapshot {
        if !snap.findings.recorded.is_empty() {
            lines.push(Line::from(vec![
                Span::styled("Recorded:    ", Style::default().fg(MUTED)),
                Span::styled(&snap.findings.recorded[0], Style::default().fg(TEXT)),
            ]));
        }
        if !snap.findings.coverage.is_empty() {
            lines.push(Line::from(vec![
                Span::styled("Coverage:    ", Style::default().fg(MUTED)),
                Span::styled(&snap.findings.coverage[0], Style::default().fg(TEXT)),
            ]));
        }
        if !snap.findings.gaps.is_empty() {
            lines.push(Line::from(vec![
                Span::styled("Gap:         ", Style::default().fg(MUTED)),
                Span::styled(&snap.findings.gaps[0], Style::default().fg(WARN)),
            ]));
        }
    }
    if lines.is_empty() {
        lines = vec![
            Line::from(vec![
                Span::styled("Recorded:    ", Style::default().fg(MUTED)),
                Span::styled("L-014 supports C-104", Style::default().fg(TEXT)),
            ]),
            Line::from(vec![
                Span::styled("Coverage:    ", Style::default().fg(MUTED)),
                Span::styled("original sources across cycles", Style::default().fg(TEXT)),
            ]),
            Line::from(vec![
                Span::styled("Gap:         ", Style::default().fg(MUTED)),
                Span::styled(
                    "RL-008 still needs evidence review",
                    Style::default().fg(WARN),
                ),
            ]),
        ];
    }
    let p = Paragraph::new(lines).block(panel("Scope findings"));
    frame.render_widget(p, area);
}

fn draw_navigation(frame: &mut Frame, app: &App, layout: &TopologyLayoutResult) {
    if layout.right_panel.width == 0 {
        return;
    }

    let p_block = panel("Graph navigation");
    frame.render_widget(p_block, layout.right_panel);

    let inner = Rect::new(
        layout.right_panel.x + 1,
        layout.right_panel.y + 1,
        layout.right_panel.width.saturating_sub(2),
        layout.right_panel.height.saturating_sub(2),
    );

    let mut curr_y = inner.y;

    // Scope
    let scope_label = Line::styled("Scope", Style::default().fg(MUTED));
    frame.render_widget(
        Paragraph::new(scope_label),
        Rect::new(inner.x, curr_y, inner.width, 1),
    );
    curr_y += 1;
    let b_scope_all = Rect::new(inner.x, curr_y, 16, 1);
    let b_scope_cycle = Rect::new(inner.x + 17, curr_y, 14, 1);
    draw_button(
        frame,
        app,
        ButtonId::TopologyScopeAll,
        "[All retained ●]",
        b_scope_all,
    );
    draw_button(
        frame,
        app,
        ButtonId::TopologyScopeThis,
        "[This cycle]",
        b_scope_cycle,
    );
    curr_y += 2;

    // Depth
    let depth_label = Line::styled("Depth", Style::default().fg(MUTED));
    frame.render_widget(
        Paragraph::new(depth_label),
        Rect::new(inner.x, curr_y, inner.width, 1),
    );
    curr_y += 1;
    let b_d1 = Rect::new(inner.x, curr_y, 5, 1);
    let b_d2 = Rect::new(inner.x + 7, curr_y, 7, 1);
    let b_d3 = Rect::new(inner.x + 16, curr_y, 5, 1);
    draw_button(frame, app, ButtonId::TopologyDepth1, "[1]", b_d1);
    draw_button(frame, app, ButtonId::TopologyDepth2, "[2 ●]", b_d2);
    draw_button(frame, app, ButtonId::TopologyDepth3, "[3]", b_d3);
    curr_y += 2;

    // Action buttons
    let b_center = Rect::new(inner.x, curr_y, inner.width, 1);
    draw_button(
        frame,
        app,
        ButtonId::TopologyCenterSelected,
        "[Center selected]",
        b_center,
    );
    curr_y += 1;
    let b_expand = Rect::new(inner.x, curr_y, inner.width, 1);
    draw_button(
        frame,
        app,
        ButtonId::TopologyExpandNeighbors,
        "[Expand neighbors]",
        b_expand,
    );
    curr_y += 1;
    let b_fit = Rect::new(inner.x, curr_y, inner.width, 1);
    draw_button(frame, app, ButtonId::TopologyFitGraph, "[Fit graph]", b_fit);
    curr_y += 2;

    // Zoom
    let zoom_line = Line::from(vec![
        Span::styled("Zoom   ", Style::default().fg(MUTED)),
        Span::styled("[-]   ", Style::default().fg(ACCENT)),
        Span::styled(
            format!("{}%   ", app.topology_state.zoom_percent),
            Style::default().fg(TEXT),
        ),
        Span::styled("[+]", Style::default().fg(ACCENT)),
    ]);
    frame.render_widget(
        Paragraph::new(zoom_line),
        Rect::new(inner.x, curr_y, inner.width, 1),
    );
    curr_y += 2;

    // Filter checkboxes
    let filters_title = Line::styled("Memory Kinds", Style::default().fg(MUTED));
    frame.render_widget(
        Paragraph::new(filters_title),
        Rect::new(inner.x, curr_y, inner.width, 1),
    );
    curr_y += 1;

    let kinds = [
        ("Actor", app.topology_state.filter_actor),
        ("Claim", app.topology_state.filter_claim),
        ("Inference", app.topology_state.filter_inference),
        ("Link", app.topology_state.filter_link),
        ("Related", app.topology_state.filter_related),
    ];
    for (k, active) in kinds {
        let chk = if active { "[x]" } else { "[ ]" };
        let line = Line::from(vec![
            Span::styled(chk, Style::default().fg(ACCENT)),
            Span::raw(" "),
            Span::styled(k, Style::default().fg(TEXT)),
        ]);
        frame.render_widget(
            Paragraph::new(line),
            Rect::new(inner.x, curr_y, inner.width, 1),
        );
        curr_y += 1;
    }

    curr_y += 1;
    let src_title = Line::styled("Sources", Style::default().fg(MUTED));
    frame.render_widget(
        Paragraph::new(src_title),
        Rect::new(inner.x, curr_y, inner.width, 1),
    );
    curr_y += 1;
    let chk = if app.topology_state.filter_sources {
        "[x]"
    } else {
        "[ ]"
    };
    let src_line = Line::from(vec![
        Span::styled(chk, Style::default().fg(ACCENT)),
        Span::raw(" "),
        Span::styled("Articles / reports", Style::default().fg(TEXT)),
    ]);
    frame.render_widget(
        Paragraph::new(src_line),
        Rect::new(inner.x, curr_y, inner.width, 1),
    );
    curr_y += 2;

    // Analysis
    let analysis_title = Line::styled("Analysis", Style::default().fg(MUTED));
    frame.render_widget(
        Paragraph::new(analysis_title),
        Rect::new(inner.x, curr_y, inner.width, 1),
    );
    curr_y += 1;

    let b_analyze = Rect::new(inner.x, curr_y, 16, 1);
    draw_button(
        frame,
        app,
        ButtonId::TopologyAnalyzeScope,
        "[Analyze scope]",
        b_analyze,
    );
    let status_span = Span::styled(
        format!(" {}", app.topology_state.scope_status),
        Style::default().fg(MUTED),
    );
    frame.render_widget(
        Paragraph::new(status_span),
        Rect::new(inner.x + 16, curr_y, inner.width.saturating_sub(16), 1),
    );
    curr_y += 2;

    // Minimap / Overview
    if inner.height.saturating_sub(curr_y - inner.y) >= 6 {
        draw_minimap(
            frame,
            Rect::new(
                inner.x,
                curr_y,
                inner.width,
                inner.height.saturating_sub(curr_y - inner.y),
            ),
        );
    }
}

fn draw_minimap(frame: &mut Frame, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(Style::default().fg(BORDER))
        .title("Graph overview")
        .title_style(Style::default().fg(MUTED));
    frame.render_widget(block, area);

    let inner = Rect::new(
        area.x + 1,
        area.y + 1,
        area.width.saturating_sub(2),
        area.height.saturating_sub(2),
    );
    if inner.width < 10 || inner.height < 4 {
        return;
    }

    // Draw miniature nodes and dashed viewport rectangle
    let mut mini_lines = Vec::new();
    let pad = inner.width.saturating_sub(18) / 2;
    let pad_str = " ".repeat(pad as usize);

    mini_lines.push(Line::raw(""));
    mini_lines.push(Line::from(vec![
        Span::raw(&pad_str),
        Span::styled("┌───", Style::default().fg(ACCENT)),
        Span::styled("●", Style::default().fg(ACCENT)),
        Span::styled("─────", Style::default().fg(ACCENT)),
        Span::styled("●", Style::default().fg(ACCENT)),
        Span::styled("───┐", Style::default().fg(ACCENT)),
        Span::styled("┄□", Style::default().fg(DIM)),
    ]));
    mini_lines.push(Line::from(vec![
        Span::raw(&pad_str),
        Span::styled("│   ", Style::default().fg(ACCENT)),
        Span::styled("■", Style::default().fg(GREEN)),
        Span::styled("       ", Style::default().fg(ACCENT)),
        Span::styled("│", Style::default().fg(ACCENT)),
    ]));
    mini_lines.push(Line::from(vec![
        Span::raw(&pad_str),
        Span::styled("│   ", Style::default().fg(ACCENT)),
        Span::styled("■", Style::default().fg(GREEN)),
        Span::styled("──▶", Style::default().fg(ACCENT)),
        Span::styled("◆", Style::default().fg(SERIES[6])),
        Span::styled("   │", Style::default().fg(ACCENT)),
        Span::styled("┄□", Style::default().fg(DIM)),
    ]));
    mini_lines.push(Line::from(vec![
        Span::raw(&pad_str),
        Span::styled("└───", Style::default().fg(ACCENT)),
        Span::styled("■", Style::default().fg(GREEN)),
        Span::styled("───────┘", Style::default().fg(ACCENT)),
    ]));
    mini_lines.push(Line::raw(""));
    mini_lines.push(Line::styled(
        "Identity: Brain canonical IDs",
        Style::default().fg(MUTED),
    ));

    frame.render_widget(Paragraph::new(mini_lines), inner);
}

fn draw_footer(frame: &mut Frame, layout: &TopologyLayoutResult) {
    let help = Line::from(vec![
        Span::styled("Tab", Style::default().fg(ACCENT)),
        Span::styled(" panels   ", Style::default().fg(MUTED)),
        Span::styled("↑↓", Style::default().fg(ACCENT)),
        Span::styled(" select   ", Style::default().fg(MUTED)),
        Span::styled("Enter", Style::default().fg(ACCENT)),
        Span::styled(" inspect   ", Style::default().fg(MUTED)),
        Span::styled("+/-", Style::default().fg(ACCENT)),
        Span::styled(" zoom   ", Style::default().fg(MUTED)),
        Span::styled("Esc", Style::default().fg(ACCENT)),
        Span::styled(" back", Style::default().fg(MUTED)),
    ]);
    frame.render_widget(Paragraph::new(help), layout.footer);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topology_state_history_push_and_pop() {
        let mut state = TopologyState {
            seed: Some(TopologySeed::Claim("c1".into())),
            ..Default::default()
        };
        state.push_history();
        assert_eq!(state.history.len(), 1);

        state.seed = Some(TopologySeed::Claim("c2".into()));
        state.push_history();
        assert_eq!(state.history.len(), 2);

        let popped = state.pop_history().unwrap();
        assert_eq!(popped.seed, TopologySeed::Claim("c2".into()));
        assert_eq!(state.history.len(), 1);
    }
}
