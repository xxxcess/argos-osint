//! Pure dashboard geometry shared by rendering, pointer routing and focus.
use ratatui::layout::Rect;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DashboardPage {
    Summary,
    Recon,
    Atlas,
    Other,
}
#[derive(Clone, Debug)]
pub struct PanelRect {
    pub index: usize,
    pub rect: Rect,
    pub height: usize,
    pub source_offset: usize,
}
#[derive(Clone, Debug)]
pub struct LayoutResult {
    pub tabs: Rect,
    pub apps: Rect,
    pub controls: Rect,
    pub kpis: Vec<Rect>,
    pub content: Rect,
    pub columns: usize,
    pub too_small: bool,
    pub panels: Vec<PanelRect>,
    starts: Vec<(usize, usize, usize)>,
}
impl LayoutResult {
    pub fn new(area: Rect, page: DashboardPage, count: usize, offset: usize) -> Self {
        let tall = area.height >= 38;
        let nav_height = if tall { 2 } else { 1 };
        let kpi_height = if tall {
            6
        } else if area.width >= 80 {
            4
        } else {
            3
        };
        let kpi_columns = if area.width >= 120 { 4 } else { 2 };
        let kpi_rows = 4_usize.div_ceil(kpi_columns);
        let row = |start: u16, height: u16| {
            Rect::new(
                area.x,
                area.y + start.min(area.height),
                area.width,
                height.min(area.height.saturating_sub(start)),
            )
        };
        let tabs = row(0, nav_height);
        let apps = row(nav_height, nav_height);
        let controls = row(nav_height * 2, 1);
        let kpi_start = nav_height * 2 + 1;
        let content_start = kpi_start + kpi_rows as u16 * kpi_height;
        let content = row(content_start, area.height.saturating_sub(content_start));
        let available = area.width.saturating_sub(kpi_columns as u16 - 1);
        let kpis = (0..4)
            .map(|i| {
                let col = i % kpi_columns;
                let x = (available as usize * col / kpi_columns) as u16 + col as u16;
                let end = (available as usize * (col + 1) / kpi_columns) as u16 + col as u16;
                let start = kpi_start + (i / kpi_columns) as u16 * kpi_height;
                Rect::new(
                    area.x + x,
                    area.y + start,
                    end - x,
                    kpi_height.min(area.height.saturating_sub(start)),
                )
            })
            .collect();
        let columns = if area.width >= 120 { 2 } else { 1 };
        let mut starts = Vec::new();
        let mut y = 0;
        let mut i = 0;
        while i < count {
            let full = columns == 1
                || (page == DashboardPage::Atlas && i == 0)
                || (page == DashboardPage::Recon && i == 4);
            let height = match page {
                DashboardPage::Summary => 18,
                DashboardPage::Recon if i < 4 => 11,
                DashboardPage::Recon => 13,
                DashboardPage::Atlas if i == 0 => 15,
                _ => 18,
            };
            starts.push((y, height, 0));
            i += 1;
            if !full && i < count {
                starts.push((y, height, 1));
                i += 1;
            }
            y += height + 1;
        }
        let panels = starts
            .iter()
            .enumerate()
            .filter_map(|(index, &(start, height, col))| {
                let top = start.max(offset);
                let bottom = (start + height).min(offset + content.height as usize);
                if bottom <= top {
                    return None;
                }
                let full = columns == 1
                    || (page == DashboardPage::Atlas && index == 0)
                    || (page == DashboardPage::Recon && index == 4);
                let left_width = area.width.saturating_sub(1) / 2;
                let (x, width) = if full {
                    (area.x, area.width)
                } else if col == 0 {
                    (area.x, left_width)
                } else {
                    (area.x + left_width + 1, area.width - left_width - 1)
                };
                Some(PanelRect {
                    index,
                    rect: Rect::new(
                        x,
                        content.y + (top - offset) as u16,
                        width,
                        (bottom - top) as u16,
                    ),
                    height,
                    source_offset: top - start,
                })
            })
            .collect();
        Self {
            tabs,
            apps,
            controls,
            kpis,
            content,
            columns,
            too_small: area.width < 60 || area.height < 16,
            panels,
            starts,
        }
    }
    pub fn extent(&self) -> usize {
        self.starts.last().map(|(s, h, _)| s + h).unwrap_or(0)
    }
    pub fn focus_start(&self, index: usize) -> usize {
        self.starts.get(index).map(|p| p.0).unwrap_or(0)
    }
    pub fn panel_height(&self, index: usize) -> usize {
        self.starts.get(index).map(|p| p.1).unwrap_or(0)
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
    fn reference_geometry_and_responsive_panels() {
        let l = LayoutResult::new(Rect::new(0, 1, 160, 48), DashboardPage::Summary, 4, 0);
        assert_eq!(l.content, Rect::new(0, 12, 160, 37));
        assert_eq!(l.panels[0].rect, Rect::new(0, 12, 79, 18));
        assert_eq!(l.panels[1].rect, Rect::new(80, 12, 80, 18));
        assert_eq!(l.panels[2].rect.y, 31);
        let l = LayoutResult::new(Rect::new(0, 1, 160, 48), DashboardPage::Recon, 5, 0);
        assert_eq!(l.panels[4].rect, Rect::new(0, 36, 160, 13));
        for (w, h) in [(120, 40), (100, 32), (80, 24), (60, 18), (40, 12)] {
            let l = LayoutResult::new(Rect::new(0, 1, w, h - 2), DashboardPage::Recon, 5, 0);
            assert_eq!(l.columns, if w >= 120 { 2 } else { 1 });
            for p in l.panels {
                assert!(p.rect.right() <= w);
                assert!(p.rect.bottom() < h);
            }
        }
    }
}
