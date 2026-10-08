use ratatui::layout::{Constraint, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Row, Table};
use ratatui::Frame;

use argos_osint_core::atlas;

use super::theme;
use super::ui::{inset, pane};

pub struct OriginsView<'a> {
    pub area: Rect,
    pub origins: &'a [&'a atlas::OriginStat],
    pub stats: &'a atlas::RunStats,
    pub scroll: usize,
    pub title: &'a str,
    pub empty: &'a str,
    pub prefix: Vec<Line<'static>>,
    pub focused: bool,
}

pub fn draw_origins(frame: &mut Frame, view: OriginsView<'_>) {
    let OriginsView {
        area,
        origins,
        stats,
        scroll,
        title,
        empty,
        prefix,
        focused,
    } = view;
    let title = if focused {
        format!("{title} · focused")
    } else {
        title.to_string()
    };
    frame.render_widget(pane(&title), area);
    let mut inner = inset(area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    if !prefix.is_empty() {
        let prefix_h = (prefix.len() as u16).min(inner.height);
        frame.render_widget(
            Paragraph::new(prefix),
            Rect {
                height: prefix_h,
                ..inner
            },
        );
        inner.y = inner.y.saturating_add(prefix_h);
        inner.height = inner.height.saturating_sub(prefix_h);
    }
    if inner.height == 0 {
        return;
    }
    if origins.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(empty.to_string(), theme::dim()))),
            inner,
        );
        return;
    }

    let country_w = inner.width.saturating_sub(39).max(8);
    let widths = [
        Constraint::Length(country_w),
        Constraint::Length(5),
        Constraint::Length(6),
        Constraint::Length(7),
        Constraint::Length(8),
        Constraint::Length(5),
    ];
    let header =
        Row::new(["Country", "Tier", "Temp", "Volume", "Articles", "Share"]).style(theme::dim());
    let visible = inner.height.saturating_sub(1) as usize;
    let rows = origins.iter().skip(scroll).take(visible).map(|origin| {
        Row::new([
            atlas::country_label(&origin.country),
            if origin.tier == 0 {
                "-".into()
            } else {
                origin.tier.to_string()
            },
            format!("{:.2}", origin.temperature),
            origin.volume.to_string(),
            origin.articles.to_string(),
            format!("{:.0}%", stats.share(origin.articles) * 100.0),
        ])
        .style(theme::text())
    });
    frame.render_widget(Table::new(rows, widths).header(header), inner);
}
