//! Pure geometry for the seeded topology Graph first preset.
//!
//! Painting and target registration consume these same clipped rectangles. The
//! virtual hop columns remain collision free when a crowded graph needs panning.
use std::collections::BTreeMap;

use argos_osint_core::brain_topology::TopologySnapshot;
use ratatui::layout::Rect;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TopologyResponsiveMode {
    Desktop,
    Compact,
    Stacked,
    Tiny,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TopologyLayoutResult {
    pub mode: TopologyResponsiveMode,
    pub full_area: Rect,
    pub header: Rect,
    pub subheader: Rect,
    pub seed_tabs: Rect,
    pub controls: Rect,
    pub content: Rect,
    pub inspector_details: Rect,
    pub inspector_related: Rect,
    pub graph_title: Rect,
    pub graph_canvas: Rect,
    pub graph_legend: Rect,
    /// Bordered Scope summary panel (Desktop/Compact) or the one-line
    /// headline under the graph (Stacked).
    pub scope_summary: Rect,
    /// Full-page Scope summary (Stacked Summary tab).
    pub summary_page: Rect,
    pub controls_page: Rect,
    pub footer: Rect,
}

impl TopologyLayoutResult {
    pub fn compute(area: Rect) -> Self {
        let mode = if area.width < 60 || area.height < 18 {
            TopologyResponsiveMode::Tiny
        } else if area.width >= 132 && area.height >= 32 {
            TopologyResponsiveMode::Desktop
        } else if area.width >= 100 && area.height >= 32 {
            TopologyResponsiveMode::Compact
        } else {
            TopologyResponsiveMode::Stacked
        };
        let row = |offset| Rect::new(area.x, area.y + offset, area.width, 1);
        let header = row(0);
        let subheader = row(1);
        let seed_tabs = row(2);
        let controls = row(3);
        let content = Rect::new(
            area.x,
            area.y + 4,
            area.width,
            area.height.saturating_sub(5),
        );
        let footer = Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1);
        let split = matches!(
            mode,
            TopologyResponsiveMode::Desktop | TopologyResponsiveMode::Compact
        );
        let graph_width = if split {
            (area.width as u32
                * if mode == TopologyResponsiveMode::Desktop {
                    65
                } else {
                    70
                }
                / 100) as u16
        } else {
            area.width
        };
        let graph = Rect::new(content.x, content.y, graph_width, content.height);
        let inspector = if split {
            Rect::new(
                graph.right(),
                content.y,
                content.width.saturating_sub(graph_width),
                content.height,
            )
        } else {
            content
        };
        let detail_height = (inspector.height * 55 / 100)
            .max(7)
            .min(inspector.height.saturating_sub(5));
        let inspector_details = Rect::new(inspector.x, inspector.y, inspector.width, detail_height);
        let inspector_related = Rect::new(
            inspector.x,
            inspector.y + detail_height,
            inspector.width,
            inspector.height.saturating_sub(detail_height),
        );
        let legend_height = 2;
        // The old two-row findings strip is folded into the summary's caveat
        // line. Wide layouts give the summary a bordered panel; stacked
        // layouts keep a one-line headline and a separate Summary page.
        let scope_height = if split {
            ((graph.height as u32 * 22 / 100) as u16).clamp(6, 10)
        } else {
            1
        };
        let canvas_height = graph
            .height
            .saturating_sub(1 + legend_height + scope_height);
        Self {
            mode,
            full_area: area,
            header,
            subheader,
            seed_tabs,
            controls,
            content,
            inspector_details,
            inspector_related,
            graph_title: Rect::new(graph.x, graph.y, graph.width, 1),
            graph_canvas: Rect::new(graph.x, graph.y + 1, graph.width, canvas_height),
            graph_legend: Rect::new(
                graph.x,
                graph.y + 1 + canvas_height,
                graph.width,
                legend_height,
            ),
            summary_page: content,
            scope_summary: Rect::new(
                graph.x,
                graph.y + 1 + canvas_height + legend_height,
                graph.width,
                scope_height,
            ),
            controls_page: content,
            footer,
        }
    }
}

#[derive(Clone, Debug)]
pub struct GraphNodePlacement {
    pub index: usize,
    /// Unclipped node bounds, retained to avoid painting a false border on clipping.
    pub rect: Rect,
    pub visible: Rect,
}

#[derive(Clone, Debug)]
pub struct GraphEdgePlacement {
    pub index: usize,
    pub cells: Vec<(u16, u16, &'static str)>,
    pub target: Rect,
}

#[derive(Clone, Debug, Default)]
pub struct GraphGeometry {
    pub nodes: Vec<GraphNodePlacement>,
    pub edges: Vec<GraphEdgePlacement>,
    pub outside_viewport: usize,
}

/// Snapshot-only hop placement; vector indices always belong to this snapshot.
pub fn graph_geometry(
    area: Rect,
    snapshot: &TopologySnapshot,
    indices: &[usize],
    zoom_percent: usize,
    pan: (i16, i16),
    center_id: Option<&str>,
) -> GraphGeometry {
    let mut result = GraphGeometry::default();
    if area.width < 4 || area.height < 3 {
        result.outside_viewport = indices.len();
        return result;
    }
    let mut columns: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for &index in indices {
        if let Some(node) = snapshot.nodes.get(index) {
            columns.entry(node.min_hops).or_default().push(index);
        }
    }
    for column in columns.values_mut() {
        column.sort_by(|a, b| snapshot.nodes[*a].id.cmp(&snapshot.nodes[*b].id));
    }
    let column_count = columns.keys().next_back().copied().unwrap_or(0) + 1;
    let base_node_width = ((area.width as usize).saturating_sub(2) / column_count.max(1))
        .saturating_sub(4)
        .clamp(10, 25)
        .min(area.width as usize);
    let zoom = zoom_percent.clamp(50, 200) as i32;
    let busiest = columns.values().map(Vec::len).max().unwrap_or(1);
    // A modest neighborhood should fit before demanding pan. Keep two text
    // rows on medium screens, then use the existing glyph/label on short ones.
    // Truly crowded columns retain virtual extent instead of crushing records.
    let (base_node_height, gap) = if busiest <= 10 {
        if busiest * 4 + busiest.saturating_sub(1) <= area.height as usize {
            (4, 1)
        } else if busiest * 2 + busiest.saturating_sub(1) <= area.height as usize {
            (2, 1)
        } else if busiest + busiest.saturating_sub(1) <= area.height as usize {
            (1, 1)
        } else {
            (1, 0)
        }
    } else {
        (4, 1)
    };
    let node_width = (base_node_width as i32 * zoom / 100)
        .max(5)
        .min(base_node_width as i32) as usize;
    let node_height = (base_node_height * zoom / 100).clamp(1, base_node_height);
    let vertical_zoom = if base_node_height == 1 {
        zoom.max(100)
    } else {
        zoom
    };
    let virtual_width = (area.width as i32).max(column_count as i32 * (base_node_width as i32 + 4));
    let mut positions: BTreeMap<String, (usize, i32, i32)> = BTreeMap::new();
    for (hop, column) in columns {
        let base_x = if column_count == 1 {
            (area.width as i32 - base_node_width as i32) / 2
        } else {
            1 + (virtual_width - base_node_width as i32 - 2) * hop as i32
                / (column_count as i32 - 1)
        };
        let stack_height =
            column.len() as i32 * base_node_height + column.len().saturating_sub(1) as i32 * gap;
        let virtual_height = (area.height as i32).max(stack_height);
        let margin = (virtual_height - stack_height) / 2;
        for (slot, index) in column.iter().enumerate() {
            let base_y = margin + slot as i32 * (base_node_height + gap)
                - (virtual_height - area.height as i32) / 2;
            let x = (base_x + base_node_width as i32 / 2 - area.width as i32 / 2) * zoom / 100
                + area.width as i32 / 2
                - node_width as i32 / 2;
            let y = (base_y + base_node_height / 2 - area.height as i32 / 2) * vertical_zoom / 100
                + area.height as i32 / 2
                - node_height / 2;
            positions.insert(snapshot.nodes[*index].id.clone(), (*index, x, y));
        }
    }
    let (center_x, center_y) = center_id
        .and_then(|id| positions.get(id))
        .map(|(_, x, y)| {
            (
                area.width as i32 / 2 - x - node_width as i32 / 2,
                area.height as i32 / 2 - y - node_height / 2,
            )
        })
        .unwrap_or((0, 0));
    for (_, x, y) in positions.values_mut() {
        *x += area.x as i32 + pan.0 as i32 + center_x;
        *y += area.y as i32 + pan.1 as i32 + center_y;
    }
    for &(index, x, y) in positions.values() {
        // Negative coordinates cannot be represented by Rect. Partially clipped
        // nodes at the top/left are omitted, so borders and targets stay honest.
        if x < area.x as i32
            || y < area.y as i32
            || x + node_width as i32 > area.right() as i32
            || y + node_height > area.bottom() as i32
        {
            result.outside_viewport += 1;
            continue;
        }
        let rect = Rect::new(x as u16, y as u16, node_width as u16, node_height as u16);
        result.nodes.push(GraphNodePlacement {
            index,
            rect,
            visible: rect.intersection(area),
        });
    }
    for (index, edge) in snapshot.edges.iter().enumerate() {
        let endpoint_visible = |id: &str| {
            result
                .nodes
                .iter()
                .any(|placement| snapshot.nodes[placement.index].id == id)
        };
        if !endpoint_visible(&edge.from) || !endpoint_visible(&edge.to) {
            continue;
        }
        let (Some(&(_, ax, ay)), Some(&(_, bx, by))) =
            (positions.get(&edge.from), positions.get(&edge.to))
        else {
            continue;
        };
        let (start_x, end_x, direction) = if ax < bx {
            (ax + node_width as i32, bx - 1, "▶")
        } else if ax > bx {
            (ax - 1, bx + node_width as i32, "◀")
        } else if ax + node_width as i32 + 2 < area.right() as i32 {
            (ax + node_width as i32, bx + node_width as i32, "◀")
        } else {
            (ax - 1, bx - 1, "▶")
        };
        let start_y = ay + node_height / 2;
        let end_y = by + node_height / 2;
        let middle_x = if ax == bx {
            start_x + if direction == "◀" { 2 } else { -2 }
        } else {
            (start_x + end_x) / 2
        };
        let mut cells = Vec::new();
        let mut add = |x: i32, y: i32, glyph| {
            if x >= area.x as i32
                && x < area.right() as i32
                && y >= area.y as i32
                && y < area.bottom() as i32
            {
                cells.push((x as u16, y as u16, glyph));
            }
        };
        for x in start_x.min(middle_x)..=start_x.max(middle_x) {
            add(x, start_y, "─");
        }
        for y in start_y.min(end_y)..=start_y.max(end_y) {
            add(middle_x, y, "│");
        }
        for x in middle_x.min(end_x)..=middle_x.max(end_x) {
            add(x, end_y, "─");
        }
        add(end_x, end_y, direction);
        let target = if end_x >= area.x as i32
            && end_x < area.right() as i32
            && end_y >= area.y as i32
            && end_y < area.bottom() as i32
        {
            Rect::new(end_x as u16, end_y as u16, 1, 1)
        } else {
            Rect::default()
        };
        result.edges.push(GraphEdgePlacement {
            index,
            cells,
            target,
        });
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use argos_osint_core::brain_topology::{
        TopologyEdge, TopologyEdgeKind, TopologyNode, TopologyNodeKind,
    };

    fn node(id: &str, hop: usize) -> TopologyNode {
        TopologyNode {
            id: id.into(),
            kind: TopologyNodeKind::Claim,
            label: format!("Long multilingual record {id} 東京"),
            detail: String::new(),
            subtype: String::new(),
            status: "retained".into(),
            is_root: hop == 0,
            article_id: String::new(),
            run_id: String::new(),
            published_at: String::new(),
            cycle: None,
            member_ids: Vec::new(),
            evidence_count: 0,
            passages: Vec::new(),
            min_hops: hop,
            revision: String::new(),
            predecessor: None,
            topology_seed: None,
        }
    }

    #[test]
    fn graph_first_breakpoints_and_ratio() {
        for (width, height, expected) in [
            (160, 50, TopologyResponsiveMode::Desktop),
            (132, 32, TopologyResponsiveMode::Desktop),
            (131, 32, TopologyResponsiveMode::Compact),
            (100, 32, TopologyResponsiveMode::Compact),
            (100, 31, TopologyResponsiveMode::Stacked),
            (80, 24, TopologyResponsiveMode::Stacked),
            (60, 18, TopologyResponsiveMode::Stacked),
            (59, 18, TopologyResponsiveMode::Tiny),
        ] {
            let layout = TopologyLayoutResult::compute(Rect::new(3, 4, width, height));
            assert_eq!(layout.mode, expected);
            if matches!(
                expected,
                TopologyResponsiveMode::Desktop | TopologyResponsiveMode::Compact
            ) {
                assert_eq!(layout.graph_canvas.right(), layout.inspector_details.x);
                assert_eq!(layout.inspector_details.right(), layout.content.right());
                assert!(layout.graph_canvas.width >= width * 65 / 100);
            }
        }
    }

    #[test]
    fn modest_neighborhood_adapts_without_hiding_nodes_or_colliding() {
        let mut snapshot = TopologySnapshot::default();
        snapshot.nodes.push(node("root", 0));
        for slot in 0..6 {
            snapshot.nodes.push(node(&format!("neighbor-{slot}"), 1));
        }
        snapshot
            .nodes
            .extend([node("second-a", 2), node("second-b", 2)]);
        let indices: Vec<_> = (0..snapshot.nodes.len()).collect();
        for (width, terminal_height) in [
            (160, 50),
            (132, 40),
            (131, 40),
            (120, 40),
            (100, 40),
            (100, 32),
            (80, 24),
            (60, 20),
        ] {
            let layout = TopologyLayoutResult::compute(Rect::new(0, 1, width, terminal_height - 2));
            let canvas = layout.graph_canvas;
            let area = Rect::new(
                canvas.x + 1,
                canvas.y + 1,
                canvas.width - 2,
                canvas.height - 2,
            );
            for zoom in [50, 75, 100] {
                let geometry = graph_geometry(area, &snapshot, &indices, zoom, (0, 0), None);
                assert_eq!(
                    geometry.outside_viewport, 0,
                    "{width}x{terminal_height} at{zoom}%"
                );
                for (index, placement) in geometry.nodes.iter().enumerate() {
                    for other in geometry.nodes.iter().skip(index + 1) {
                        assert!(placement.rect.intersection(other.rect).is_empty());
                    }
                }
            }
        }
    }

    #[test]
    fn offscreen_endpoints_do_not_leave_dangling_connectors() {
        let mut snapshot = TopologySnapshot {
            nodes: vec![node("root", 0)],
            ..Default::default()
        };
        for slot in 0..20 {
            let id = format!("neighbor-{slot:02}");
            snapshot.nodes.push(node(&id, 1));
            snapshot.edges.push(TopologyEdge {
                id: format!("edge-{slot}"),
                from: "root".into(),
                to: id,
                kind: TopologyEdgeKind::Concerns,
                label: "concerns".into(),
                status: "recorded".into(),
                member_ids: Vec::new(),
                receipts: Vec::new(),
                rationale: String::new(),
            });
        }
        let indices: Vec<_> = (0..snapshot.nodes.len()).collect();
        let geometry = graph_geometry(
            Rect::new(0, 0, 78, 10),
            &snapshot,
            &indices,
            100,
            (0, 0),
            None,
        );
        assert!(geometry.outside_viewport > 0);
        assert_eq!(geometry.edges.len(), geometry.nodes.len() - 1);
        for edge in geometry.edges {
            let actual = &snapshot.edges[edge.index];
            for id in [&actual.from, &actual.to] {
                assert!(geometry
                    .nodes
                    .iter()
                    .any(|placement| &snapshot.nodes[placement.index].id == id));
            }
        }
    }

    #[test]
    fn crowded_columns_never_collide_and_keep_indices_coherent() {
        let mut snapshot = TopologySnapshot::default();
        snapshot.nodes.push(node("root", 0));
        for hop in 1..=3 {
            for slot in 0..20 {
                snapshot.nodes.push(node(&format!("{hop}-{slot:02}"), hop));
            }
        }
        let indices: Vec<_> = (0..snapshot.nodes.len()).collect();
        for zoom in [50, 75, 100, 150, 200] {
            let area = Rect::new(2, 5, 58, 20);
            let geometry = graph_geometry(area, &snapshot, &indices, zoom, (0, 0), None);
            assert_eq!(
                geometry.nodes.len() + geometry.outside_viewport,
                indices.len()
            );
            for (index, placement) in geometry.nodes.iter().enumerate() {
                assert!(placement.index < snapshot.nodes.len());
                assert_eq!(placement.rect.intersection(area), placement.rect);
                for other in geometry.nodes.iter().skip(index + 1) {
                    assert!(placement.rect.intersection(other.rect).is_empty());
                }
            }
        }
        let geometry = graph_geometry(
            Rect::new(2, 5, 58, 20),
            &snapshot,
            &indices,
            100,
            (0, 0),
            None,
        );
        assert!(geometry
            .nodes
            .iter()
            .any(|placement| snapshot.nodes[placement.index].is_root));
        assert!(geometry.outside_viewport > 0);
    }

    #[test]
    fn placement_is_stable_under_snapshot_reorder_and_transforms_are_real() {
        let mut snapshot = TopologySnapshot {
            nodes: vec![node("root", 0), node("b", 1), node("a", 1)],
            ..Default::default()
        };
        let area = Rect::new(10, 4, 80, 30);
        let indices = [0, 1, 2];
        let positions = |snapshot: &TopologySnapshot| {
            graph_geometry(area, snapshot, &indices, 100, (0, 0), None)
                .nodes
                .into_iter()
                .map(|placement| (snapshot.nodes[placement.index].id.clone(), placement.rect))
                .collect::<BTreeMap<_, _>>()
        };
        let before = positions(&snapshot);
        snapshot.nodes.reverse();
        assert_eq!(before, positions(&snapshot));
        let normal = graph_geometry(area, &snapshot, &indices, 100, (0, 0), None);
        let panned = graph_geometry(area, &snapshot, &indices, 100, (3, 2), None);
        let normal_root = normal
            .nodes
            .iter()
            .find(|p| snapshot.nodes[p.index].is_root)
            .unwrap();
        let panned_root = panned
            .nodes
            .iter()
            .find(|p| snapshot.nodes[p.index].is_root)
            .unwrap();
        assert_eq!(panned_root.rect.x, normal_root.rect.x + 3);
        assert_eq!(panned_root.rect.y, normal_root.rect.y + 2);
        let centered = graph_geometry(area, &snapshot, &indices, 150, (0, 0), Some("a"));
        let selected = centered
            .nodes
            .iter()
            .find(|p| snapshot.nodes[p.index].id == "a")
            .unwrap();
        assert_eq!(
            selected.rect.x + selected.rect.width / 2,
            area.x + area.width / 2
        );
        assert_eq!(
            selected.rect.y + selected.rect.height / 2,
            area.y + area.height / 2
        );
        assert_ne!(
            normal_root.rect,
            graph_geometry(area, &snapshot, &indices, 50, (0, 0), None)
                .nodes
                .iter()
                .find(|p| snapshot.nodes[p.index].is_root)
                .unwrap()
                .rect
        );
    }
}
