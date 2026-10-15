//! Pure geometry and layout computations for Brain Memory Topology (spec §2).
//!
//! Owns panel Rects for desktop (>=132x32), compact (100..131), stacked (<100 or <32),
//! and tiny (<60x18) responsive modes. Pure geometry shared by drawing, focus, and mouse.

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
    pub content: Rect,
    pub left_panel: Rect,
    pub inspector_details: Rect,
    pub inspector_related: Rect,
    pub center_panel: Rect,
    pub graph_title: Rect,
    pub graph_canvas: Rect,
    pub graph_legend: Rect,
    pub scope_findings: Rect,
    pub right_panel: Rect,
    pub nav_scope: Rect,
    pub nav_depth: Rect,
    pub nav_actions: Rect,
    pub nav_zoom: Rect,
    pub nav_filters: Rect,
    pub nav_analysis: Rect,
    pub nav_minimap: Rect,
    pub footer: Rect,
}

impl TopologyLayoutResult {
    pub fn compute(area: Rect) -> Self {
        if area.width < 60 || area.height < 18 {
            return Self::tiny(area);
        }

        let header_height = 1u16;
        let subheader_height = 1u16;
        let tabs_height = 1u16;
        let footer_height = 1u16;

        let header = Rect::new(area.x, area.y, area.width, header_height);
        let subheader = Rect::new(area.x, area.y + header_height, area.width, subheader_height);
        let seed_tabs = Rect::new(
            area.x,
            area.y + header_height + subheader_height,
            area.width,
            tabs_height,
        );

        let top_offset = header_height + subheader_height + tabs_height;
        let bottom_height = footer_height;
        let content_height = area.height.saturating_sub(top_offset + bottom_height);

        let content = Rect::new(area.x, area.y + top_offset, area.width, content_height);
        let footer = Rect::new(
            area.x,
            area.y + area.height.saturating_sub(bottom_height),
            area.width,
            bottom_height,
        );

        if area.width >= 132 && area.height >= 32 {
            Self::desktop(area, header, subheader, seed_tabs, content, footer)
        } else if area.width >= 100 && area.height >= 28 {
            Self::compact(area, header, subheader, seed_tabs, content, footer)
        } else {
            Self::stacked(area, header, subheader, seed_tabs, content, footer)
        }
    }

    fn desktop(
        full: Rect,
        header: Rect,
        subheader: Rect,
        seed_tabs: Rect,
        content: Rect,
        footer: Rect,
    ) -> Self {
        // Desktop: Left 24%, Center 55%, Right 21%
        let left_w = (content.width as f32 * 0.24).round() as u16;
        let right_w = (content.width as f32 * 0.21).round() as u16;
        let center_w = content.width.saturating_sub(left_w + right_w);

        let left_panel = Rect::new(content.x, content.y, left_w, content.height);
        let center_panel = Rect::new(content.x + left_w, content.y, center_w, content.height);
        let right_panel = Rect::new(
            content.x + left_w + center_w,
            content.y,
            right_w,
            content.height,
        );

        // Split Left panel: Inspector details (52%) and Related (48%)
        let insp_details_h = (left_panel.height as f32 * 0.52).round() as u16;
        let insp_rel_h = left_panel.height.saturating_sub(insp_details_h);
        let inspector_details =
            Rect::new(left_panel.x, left_panel.y, left_panel.width, insp_details_h);
        let inspector_related = Rect::new(
            left_panel.x,
            left_panel.y + insp_details_h,
            left_panel.width,
            insp_rel_h,
        );

        // Split Center panel: Title (1), Canvas (flex), Legend (3), Findings (5)
        let findings_h = 5u16.min(center_panel.height.saturating_sub(10));
        let legend_h = 3u16.min(center_panel.height.saturating_sub(findings_h + 5));
        let title_h = 1u16;
        let canvas_h = center_panel
            .height
            .saturating_sub(title_h + legend_h + findings_h);

        let graph_title = Rect::new(center_panel.x, center_panel.y, center_panel.width, title_h);
        let graph_canvas = Rect::new(
            center_panel.x,
            center_panel.y + title_h,
            center_panel.width,
            canvas_h,
        );
        let graph_legend = Rect::new(
            center_panel.x,
            center_panel.y + title_h + canvas_h,
            center_panel.width,
            legend_h,
        );
        let scope_findings = Rect::new(
            center_panel.x,
            center_panel.y + title_h + canvas_h + legend_h,
            center_panel.width,
            findings_h,
        );

        // Split Right panel:
        // Scope (3), Depth (3), Actions (3), Zoom (2), Filters (9), Analysis (3), Minimap (flex/rest)
        let ry = right_panel.y;
        let rw = right_panel.width;
        let nav_scope = Rect::new(right_panel.x, ry, rw, 3);
        let nav_depth = Rect::new(right_panel.x, ry + 3, rw, 3);
        let nav_actions = Rect::new(right_panel.x, ry + 6, rw, 4);
        let nav_zoom = Rect::new(right_panel.x, ry + 10, rw, 2);
        let nav_filters = Rect::new(right_panel.x, ry + 12, rw, 9);
        let nav_analysis = Rect::new(right_panel.x, ry + 21, rw, 3);
        let minimap_y = ry + 24;
        let minimap_h = right_panel.height.saturating_sub(24);
        let nav_minimap = Rect::new(right_panel.x, minimap_y, rw, minimap_h);

        Self {
            mode: TopologyResponsiveMode::Desktop,
            full_area: full,
            header,
            subheader,
            seed_tabs,
            content,
            left_panel,
            inspector_details,
            inspector_related,
            center_panel,
            graph_title,
            graph_canvas,
            graph_legend,
            scope_findings,
            right_panel,
            nav_scope,
            nav_depth,
            nav_actions,
            nav_zoom,
            nav_filters,
            nav_analysis,
            nav_minimap,
            footer,
        }
    }

    fn compact(
        full: Rect,
        header: Rect,
        subheader: Rect,
        seed_tabs: Rect,
        content: Rect,
        footer: Rect,
    ) -> Self {
        // Compact: Left 28%, Center 72%, Right panel drawer
        let left_w = (content.width as f32 * 0.28).round() as u16;
        let center_w = content.width.saturating_sub(left_w);

        let left_panel = Rect::new(content.x, content.y, left_w, content.height);
        let center_panel = Rect::new(content.x + left_w, content.y, center_w, content.height);
        let right_panel = Rect::new(content.x + content.width, content.y, 0, content.height);

        let insp_details_h = (left_panel.height as f32 * 0.50).round() as u16;
        let insp_rel_h = left_panel.height.saturating_sub(insp_details_h);
        let inspector_details =
            Rect::new(left_panel.x, left_panel.y, left_panel.width, insp_details_h);
        let inspector_related = Rect::new(
            left_panel.x,
            left_panel.y + insp_details_h,
            left_panel.width,
            insp_rel_h,
        );

        let findings_h = 4u16;
        let legend_h = 3u16;
        let title_h = 1u16;
        let canvas_h = center_panel
            .height
            .saturating_sub(title_h + legend_h + findings_h);

        let graph_title = Rect::new(center_panel.x, center_panel.y, center_panel.width, title_h);
        let graph_canvas = Rect::new(
            center_panel.x,
            center_panel.y + title_h,
            center_panel.width,
            canvas_h,
        );
        let graph_legend = Rect::new(
            center_panel.x,
            center_panel.y + title_h + canvas_h,
            center_panel.width,
            legend_h,
        );
        let scope_findings = Rect::new(
            center_panel.x,
            center_panel.y + title_h + canvas_h + legend_h,
            center_panel.width,
            findings_h,
        );

        Self {
            mode: TopologyResponsiveMode::Compact,
            full_area: full,
            header,
            subheader,
            seed_tabs,
            content,
            left_panel,
            inspector_details,
            inspector_related,
            center_panel,
            graph_title,
            graph_canvas,
            graph_legend,
            scope_findings,
            right_panel,
            nav_scope: Rect::default(),
            nav_depth: Rect::default(),
            nav_actions: Rect::default(),
            nav_zoom: Rect::default(),
            nav_filters: Rect::default(),
            nav_analysis: Rect::default(),
            nav_minimap: Rect::default(),
            footer,
        }
    }

    fn stacked(
        full: Rect,
        header: Rect,
        subheader: Rect,
        seed_tabs: Rect,
        content: Rect,
        footer: Rect,
    ) -> Self {
        // Stacked: Single active view fills content
        let left_panel = content;
        let center_panel = content;
        let right_panel = content;

        let insp_details_h = content.height / 2;
        let inspector_details = Rect::new(content.x, content.y, content.width, insp_details_h);
        let inspector_related = Rect::new(
            content.x,
            content.y + insp_details_h,
            content.width,
            content.height.saturating_sub(insp_details_h),
        );

        let findings_h = 4u16;
        let legend_h = 2u16;
        let title_h = 1u16;
        let canvas_h = content
            .height
            .saturating_sub(title_h + legend_h + findings_h);

        let graph_title = Rect::new(content.x, content.y, content.width, title_h);
        let graph_canvas = Rect::new(content.x, content.y + title_h, content.width, canvas_h);
        let graph_legend = Rect::new(
            content.x,
            content.y + title_h + canvas_h,
            content.width,
            legend_h,
        );
        let scope_findings = Rect::new(
            content.x,
            content.y + title_h + canvas_h + legend_h,
            content.width,
            findings_h,
        );

        Self {
            mode: TopologyResponsiveMode::Stacked,
            full_area: full,
            header,
            subheader,
            seed_tabs,
            content,
            left_panel,
            inspector_details,
            inspector_related,
            center_panel,
            graph_title,
            graph_canvas,
            graph_legend,
            scope_findings,
            right_panel,
            nav_scope: Rect::new(content.x, content.y, content.width, 3),
            nav_depth: Rect::new(content.x, content.y + 3, content.width, 3),
            nav_actions: Rect::new(content.x, content.y + 6, content.width, 4),
            nav_zoom: Rect::new(content.x, content.y + 10, content.width, 2),
            nav_filters: Rect::new(content.x, content.y + 12, content.width, 9),
            nav_analysis: Rect::new(content.x, content.y + 21, content.width, 3),
            nav_minimap: Rect::default(),
            footer,
        }
    }

    fn tiny(full: Rect) -> Self {
        Self {
            mode: TopologyResponsiveMode::Tiny,
            full_area: full,
            header: Rect::default(),
            subheader: Rect::default(),
            seed_tabs: Rect::default(),
            content: full,
            left_panel: Rect::default(),
            inspector_details: Rect::default(),
            inspector_related: Rect::default(),
            center_panel: full,
            graph_title: Rect::default(),
            graph_canvas: full,
            graph_legend: Rect::default(),
            scope_findings: Rect::default(),
            right_panel: Rect::default(),
            nav_scope: Rect::default(),
            nav_depth: Rect::default(),
            nav_actions: Rect::default(),
            nav_zoom: Rect::default(),
            nav_filters: Rect::default(),
            nav_analysis: Rect::default(),
            nav_minimap: Rect::default(),
            footer: Rect::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_desktop_proportions() {
        let area = Rect::new(0, 0, 160, 50);
        let l = TopologyLayoutResult::compute(area);
        assert_eq!(l.mode, TopologyResponsiveMode::Desktop);
        assert!(l.left_panel.width >= 35 && l.left_panel.width <= 42);
        assert!(l.center_panel.width >= 80 && l.center_panel.width <= 92);
        assert!(l.right_panel.width >= 30 && l.right_panel.width <= 36);
        assert_eq!(
            l.left_panel.width + l.center_panel.width + l.right_panel.width,
            160
        );
    }

    #[test]
    fn layout_compact_collapses_right() {
        let area = Rect::new(0, 0, 120, 40);
        let l = TopologyLayoutResult::compute(area);
        assert_eq!(l.mode, TopologyResponsiveMode::Compact);
        assert_eq!(l.right_panel.width, 0);
        assert_eq!(l.left_panel.width + l.center_panel.width, 120);
    }

    #[test]
    fn layout_tiny_under_threshold() {
        let area = Rect::new(0, 0, 50, 15);
        let l = TopologyLayoutResult::compute(area);
        assert_eq!(l.mode, TopologyResponsiveMode::Tiny);
    }
}
