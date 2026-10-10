//! Analytics presets: shared by drawing, focus registration and pointer routing.
use ratatui::layout::Rect;

#[derive(Clone, Debug)]
pub struct AnalyticsLayout {
    pub tabs: Rect,
    pub controls: Rect,
    pub apps: Rect,
    pub content: Rect,
    pub status: Rect,
    pub columns: usize,
    pub card_height: usize,
    pub compact: bool,
}
impl AnalyticsLayout {
    pub fn new(area: Rect) -> Self {
        let row = |n: u16| {
            Rect::new(
                area.x,
                area.y.saturating_add(n),
                area.width,
                u16::from(n < area.height),
            )
        };
        let content = Rect::new(
            area.x,
            area.y.saturating_add(3),
            area.width,
            area.height.saturating_sub(4),
        );
        let card_height = usize::from(content.height.saturating_sub(1) / 2).clamp(16, 24);
        Self {
            tabs: row(0),
            controls: row(1),
            apps: row(2),
            content,
            status: row(area.height.saturating_sub(1)),
            columns: if area.width >= 104 && content.height >= 16 {
                2
            } else {
                1
            },
            card_height,
            compact: content.height < 16,
        }
    }
    pub fn extent(&self, count: usize) -> usize {
        if self.compact {
            count
        } else {
            count.div_ceil(self.columns) * (self.card_height + 1) - usize::from(count > 0)
        }
    }
    pub fn card(&self, index: usize, offset: usize) -> Option<Rect> {
        let stride = if self.compact {
            1
        } else {
            self.card_height + 1
        };
        let start = (index / self.columns) * stride;
        let height = if self.compact { 1 } else { self.card_height };
        // Cards are never partially painted: whole-card reveal preserves stable geometry.
        if start < offset || start + height > offset + usize::from(self.content.height) {
            return None;
        }
        let gutter = if self.columns == 2 { 4 } else { 0 };
        let width = self.content.width.saturating_sub(gutter) / self.columns as u16;
        Some(Rect::new(
            self.content.x + (index % self.columns) as u16 * (width + gutter),
            self.content.y + (start - offset) as u16,
            width,
            height as u16,
        ))
    }
    pub fn focus_start(&self, index: usize) -> usize {
        (index / self.columns)
            * if self.compact {
                1
            } else {
                self.card_height + 1
            }
    }
}

/// Report preset retains exact selected details before allocating a plot.
#[derive(Clone, Debug)]
pub struct ReportLayout {
    pub selected: Rect,
    pub plot: Rect,
    pub details: Rect,
    pub position: Rect,
}
impl ReportLayout {
    pub fn new(area: Rect, selected_height: usize) -> Self {
        let header = (selected_height as u16).min(area.height.saturating_sub(3));
        let selected = Rect::new(area.x, area.y, area.width, header);
        let rest = Rect::new(
            area.x,
            area.y + header,
            area.width,
            area.height.saturating_sub(header + 1),
        );
        let (plot, details) = if area.width >= 120 && rest.height >= 6 {
            let width = area.width / 2;
            (
                Rect::new(rest.x, rest.y, width - 2, rest.height),
                Rect::new(
                    rest.x + width + 2,
                    rest.y,
                    area.width - width - 2,
                    rest.height,
                ),
            )
        } else {
            let height = if rest.height >= 12 {
                rest.height / 2
            } else {
                0
            };
            (
                Rect::new(rest.x, rest.y, rest.width, height),
                Rect::new(rest.x, rest.y + height, rest.width, rest.height - height),
            )
        };
        Self {
            selected,
            plot,
            details,
            position: Rect::new(
                area.x,
                area.bottom().saturating_sub(1),
                area.width,
                u16::from(area.height > 0),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn grid_boundary_and_short_viewports() {
        for (w, h) in [
            (160, 50),
            (120, 40),
            (104, 36),
            (103, 36),
            (100, 36),
            (80, 24),
            (40, 20),
            (100, 24),
        ] {
            let layout = AnalyticsLayout::new(Rect::new(0, 0, w, h));
            assert_eq!(layout.columns, if w >= 104 { 2 } else { 1 });
            for i in 0..8 {
                if let Some(rect) = layout.card(i, 0) {
                    assert!(layout
                        .content
                        .contains((rect.right() - 1, rect.bottom() - 1).into()));
                }
            }
        }
    }
}
