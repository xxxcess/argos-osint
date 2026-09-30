//! Apps column on the left, the open app on the right, prompt at the bottom.
//! The prompt talks to the case desk or the selected case.

use ratatui::layout::{Constraint, Direction, Layout, Margin, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    BorderType, Cell, Clear, Gauge, HighlightSpacing, List, ListItem, ListState, Paragraph, Row,
    Scrollbar, ScrollbarOrientation, Sparkline, Table, Tabs, Wrap,
};
use ratatui::Frame;
use tui_nodes::{Connection, NodeGraph, NodeLayout};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::app::{
    tna_glyph_for_kind, App, CasePage, Focus, ModuleId, ProviderPage, SystemPage, TnaDisplayItem,
    TNA_GRAPH_BOX_BUDGET,
};
use super::theme::{self, panel};
use argos_osint_core::paths::fit_status;
use argos_osint_core::secrets::mask;
use argos_osint_core::tna::TnaCluster;

#[path = "case_view.rs"]
mod case_view;
#[path = "network.rs"]
mod network;

#[path = "providers.rs"]
mod providers;

pub fn draw(frame: &mut Frame, app: &mut App) {
    app.provider_field_hits.clear();
    let area = frame.area();
    frame.render_widget(Paragraph::new("").style(theme::text()), area);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(8),
            Constraint::Length(4),
            Constraint::Length(1),
        ])
        .split(area);
    draw_body(frame, app, chunks[0]);
    if app.chat_report.is_some() {
        let bottom = split_v(
            Rect::new(area.x, chunks[1].y, area.width, 5.min(area.height)),
            &[Constraint::Length(1), Constraint::Length(4)],
        );
        draw_footer(frame, app, bottom[0]);
        draw_prompt(frame, app, bottom[1]);
        network::draw_answer(frame, app, chunks[0]);
    } else {
        draw_prompt(frame, app, chunks[1]);
        draw_footer(frame, app, chunks[2]);
    }
    if app.provider_picker.is_some() {
        providers::draw_picker(frame, app, area);
    }
    if app.modal {
        draw_modal(frame, app, area);
    }
    if app.model_picker {
        draw_model_picker(frame, app, area);
    }
    if app.free_picker {
        draw_free_picker(frame, app, area);
    }
    if app.help {
        if app.chat_report.is_some() {
            network::draw_help(frame, area);
        } else {
            draw_help(frame, area);
        }
    }
    if app.scope.is_some() {
        draw_scope(frame, app, area);
    }
    if app.confirm_query.is_some() {
        draw_case_confirm(frame, app, area);
    }
    if app.confirm_delete_report.is_some() {
        draw_delete_report_confirm(frame, app, area);
    } else if app.confirm_report.is_some() {
        draw_report_confirm(frame, app, area);
    }
    if app.brain_card {
        draw_brain_card(frame, app, area);
    }
}

fn on_case_desk(app: &App) -> bool {
    matches!(app.module, None | Some(ModuleId::Cases))
}

fn draw_body(frame: &mut Frame, app: &mut App, area: Rect) {
    draw_desk_row(frame, app, area);
}

fn draw_main(frame: &mut Frame, app: &mut App, area: Rect) {
    if app.chat_report.is_some() {
        draw_network(frame, app, area);
    } else if on_case_desk(app) {
        let work = under_case_tabs(frame, app, area);
        draw_desk_and_reports(frame, app, work);
    } else {
        app.case_tab_area = Rect::default();
        app.case_tab_hits.clear();
        app.canvas_area = Rect::default();
        draw_widget(frame, app, area);
    }
}

fn draw_desk_row(frame: &mut Frame, app: &mut App, area: Rect) {
    let cols = if app.chat_report.is_some()
        || app.investigation.is_some()
        || app.module == Some(ModuleId::Providers)
    {
        split_h(area, &[Constraint::Length(18), Constraint::Min(0)])
    } else {
        split_h(
            area,
            &[Constraint::Percentage(26), Constraint::Percentage(74)],
        )
    };
    draw_launcher(frame, app, cols[0]);
    draw_main(frame, app, cols[1]);
}

fn draw_desk_and_reports(frame: &mut Frame, app: &mut App, area: Rect) {
    if app.case_page == CasePage::Brain {
        draw_brain(frame, app, area);
        return;
    }
    if app.investigation.is_some() {
        case_view::draw(frame, app, area);
        return;
    }
    if app.case_page == CasePage::Network {
        draw_network(frame, app, area);
        return;
    }
    operations_desk(frame, app, area);
}
fn operations_desk(frame: &mut Frame, app: &mut App, area: Rect) {
    let desk = app.desk_projection();
    if app.cases.is_empty() || app.desk_transcript || desk.cards.is_empty() {
        let rows = split_v(
            area,
            &[
                Constraint::Length(2),
                Constraint::Min(0),
                Constraint::Length(if app.cases.is_empty() { 0 } else { 6 }),
            ],
        );
        frame.render_widget(Paragraph::new("Ask saved evidence · + saves a case without a report.\n\\ transcript · ordinary questions remain retrieval only").style(theme::dim()),rows[0]);
        app.canvas_area = rows[1];
        if app.desk_transcript && area.width >= 80 {
            let cols = split_h(
                rows[1],
                &[Constraint::Percentage(65), Constraint::Percentage(35)],
            );
            app.canvas_area = cols[0];
            draw_canvas(frame, app, cols[0]);
            draw_report_list(frame, app, cols[1]);
        } else {
            draw_canvas(frame, app, rows[1]);
        }
        if !app.cases.is_empty() {
            desk_queue(frame, app, &desk, rows[2]);
        }
        return;
    }
    app.canvas_area = area;
    app.report_area = Rect::default();
    let rows = split_v(
        area,
        &[
            Constraint::Length(9),
            Constraint::Min(0),
            Constraint::Length(1),
        ],
    );
    let mut cards = Vec::new();
    for (i, c) in desk.cards.iter().enumerate() {
        let title = app
            .cases
            .iter()
            .find(|case| case.id == c.case_id)
            .map(|case| case.title.as_str())
            .unwrap_or(&c.case_id);
        let lead = c
            .lead_id
            .as_ref()
            .and_then(|id| {
                app.desk_cases
                    .get(&c.case_id)?
                    .entities
                    .iter()
                    .find(|e| &e.id == id)
            })
            .map(|e| e.label.as_str())
            .unwrap_or("case question");
        cards.push(
            Line::from(format!(
                "{} {} {} · {lead} · {title}",
                if app.desk_pane == 0 && app.desk_row == i {
                    "›"
                } else {
                    " "
                },
                i + 1,
                match c.kind {
                    argos_osint_core::investigation::NextWorkKind::Review => "REVIEW",
                    argos_osint_core::investigation::NextWorkKind::Enrich => "ENRICH",
                    argos_osint_core::investigation::NextWorkKind::Gap => "GAP",
                    argos_osint_core::investigation::NextWorkKind::Product => "PRODUCT",
                }
            ))
            .style(if app.desk_pane == 0 && app.desk_row == i {
                theme::selected()
            } else {
                theme::text()
            }),
        );
        cards.push(Line::from(format!("  {} · Enter opens work", c.reason)).style(theme::dim()));
    }
    frame.render_widget(
        Paragraph::new(cards)
            .wrap(Wrap { trim: false })
            .block(panel(if app.desk_refreshing.is_empty() {
                " Next Work · w focus · j/k · Enter "
            } else {
                " Next Work · refreshing · w focus · Enter "
            })),
        rows[0],
    );
    let lower = if area.width >= 80 {
        split_h(
            rows[1],
            &[Constraint::Percentage(50), Constraint::Percentage(50)],
        )
    } else {
        split_v(
            rows[1],
            &[Constraint::Percentage(50), Constraint::Percentage(50)],
        )
    };
    desk_queue(frame, app, &desk, lower[0]);
    let visible = lower[1].height.saturating_sub(2).max(1) as usize;
    let mut questions = Vec::new();
    for (i, g) in desk
        .questions
        .iter()
        .enumerate()
        .skip(if app.desk_pane == 2 {
            app.desk_row.saturating_sub(visible.saturating_sub(1))
        } else {
            0
        })
        .take(visible)
    {
        questions.push(
            Line::from(format!(
                "{} {} · {} · {}",
                if app.desk_pane == 2 && app.desk_row == i {
                    "›"
                } else {
                    "○"
                },
                g.kind.label(),
                g.input,
                g.reason
            ))
            .style(if app.desk_pane == 2 && app.desk_row == i {
                theme::selected()
            } else {
                theme::dim()
            }),
        );
    }
    if questions.is_empty() {
        questions.push(Line::from(
            "No named gaps · missing links alone are not gaps",
        ));
    }
    frame.render_widget(
        Paragraph::new(questions)
            .wrap(Wrap { trim: false })
            .block(panel(" Open Questions · g focus · Enter ")),
        lower[1],
    );
    frame.render_widget(
        Paragraph::new("\\ transcript · Tab composer · + scope · Selecting a lead never collects.")
            .style(theme::dim()),
        rows[2],
    );
}
fn desk_queue(
    frame: &mut Frame,
    app: &App,
    desk: &argos_osint_core::investigation::DeskProjection,
    area: Rect,
) {
    let mut lines = Vec::new();
    for (i, row) in desk
        .queue
        .iter()
        .enumerate()
        .skip(if app.desk_pane == 1 {
            app.desk_row.saturating_sub(3)
        } else {
            0
        })
        .take(area.height.saturating_sub(2) as usize)
    {
        let title = app
            .cases
            .iter()
            .find(|c| c.id == row.case_id)
            .map(|c| c.title.as_str())
            .unwrap_or(&row.case_id);
        lines.push(
            Line::from(format!(
                "{} {title} · {} review · {} jobs · {} gaps{}",
                if app.desk_pane == 1 && app.desk_row == i {
                    "›"
                } else {
                    " "
                },
                row.pending_review,
                row.running_jobs,
                row.gap_counts.iter().sum::<usize>(),
                if row.draft_ready {
                    " · draft ready"
                } else {
                    ""
                }
            ))
            .style(if app.desk_pane == 1 && app.desk_row == i {
                theme::selected()
            } else {
                theme::text()
            }),
        );
    }
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(panel(" Queue · q focus · cases without reports included ")),
        area,
    );
}

fn draw_brain(frame: &mut Frame, app: &mut App, area: Rect) {
    app.report_area = Rect::default();
    app.report_line_index.clear();
    app.canvas_area = Rect::default();
    draw_brain_list(frame, app, area);
}

fn draw_brain_list(frame: &mut Frame, app: &mut App, area: Rect) {
    app.brain_list_area = area;
    let title = " Memories · j/k move · Enter views a fact ";
    let block = panel(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let shown = app.shown_memories();
    app.sync_brain_list_ui();

    if shown.is_empty() {
        frame.render_widget(
            Paragraph::new("No memories in this view.")
                .style(theme::dim())
                .wrap(Wrap { trim: false }),
            inner,
        );
        return;
    }

    // Copy rows so we do not hold a memories borrow across stateful render.
    let row_text: Vec<(bool, String)> = shown
        .iter()
        .map(|memory| (memory.pinned, memory.text.clone()))
        .collect();
    let items: Vec<ListItem> = row_text
        .into_iter()
        .map(|(pinned, text)| {
            let pin = if pinned { "pin" } else { "   " };
            ListItem::new(Line::from(format!("{pin}  {text}")).style(theme::text()))
        })
        .collect();

    let list = List::new(items)
        .highlight_style(theme::selected())
        .highlight_symbol("> ")
        .highlight_spacing(HighlightSpacing::Always);

    frame.render_stateful_widget(list, inner, &mut app.brain_list_state);

    // Scrollbar synced to selection (ITEM_HEIGHT = 1), same as Table initiative list.
    frame.render_stateful_widget(
        Scrollbar::default()
            .orientation(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None),
        area.inner(Margin {
            vertical: 1,
            horizontal: 0,
        }),
        &mut app.brain_list_scroll,
    );
}

fn draw_brain_card(frame: &mut Frame, app: &App, area: Rect) {
    let modal = centered(
        area,
        area.width.saturating_mul(3) / 4,
        area.height.saturating_mul(3) / 4,
    );
    frame.render_widget(Clear, modal);
    let memory = app
        .memories
        .iter()
        .find(|memory| Some(memory.id.as_str()) == app.brain_edit_id.as_deref());
    let text = memory
        .map(|memory| memory.text.clone())
        .unwrap_or_else(|| "This memory is no longer stored.".into());
    let source = memory
        .and_then(|memory| memory.report_id.as_deref())
        .and_then(|id| app.reports.iter().find(|report| report.id == id))
        .map(|report| format!("From report: {}", report.title))
        .unwrap_or_else(|| "Fact".into());
    let lines = vec![
        Line::from(text).style(theme::text()),
        Line::from(""),
        Line::from(source).style(theme::dim()),
        Line::from(""),
        Line::from("Close").style(theme::selected()),
        Line::from(""),
        Line::from("Enter or Esc closes this card.").style(theme::dim()),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .block(panel(" Memory "))
            .wrap(Wrap { trim: false }),
        modal,
    );
}

fn brain_list_lines(app: &App, height: usize) -> Vec<Line<'static>> {
    let shown = app.shown_memories();
    let mut lines = vec![
        Line::from("j/k move · Enter views a fact").style(theme::dim()),
        Line::from(""),
    ];
    if shown.is_empty() {
        lines.push(Line::from("No memories in this view.".to_string()).style(theme::dim()));
        return tail(lines, height);
    }
    let room = height.saturating_sub(lines.len()).max(1);
    let start = if shown.len() <= room {
        0
    } else {
        app.brain_sel
            .saturating_sub(room / 2)
            .min(shown.len().saturating_sub(room))
    };
    for (offset, memory) in shown.iter().enumerate().skip(start).take(room) {
        let pin = if memory.pinned { "pin" } else { "   " };
        let mark = if offset == app.brain_sel { ">" } else { " " };
        let style = if offset == app.brain_sel {
            theme::selected()
        } else {
            theme::text()
        };
        lines.push(Line::from(format!("{mark} {pin}  {}", memory.text)).style(style));
    }
    tail(lines, height)
}

fn draw_report_list(frame: &mut Frame, app: &mut App, area: Rect) {
    app.report_area = area;
    let (lines, index) = report_lines(app, area.width.saturating_sub(2) as usize);
    app.report_line_index = index;
    let title = if app.focus == Focus::Reports {
        " Cases & reports · focused "
    } else {
        " Cases & reports "
    };
    frame.render_widget(Paragraph::new(lines).block(panel(title)), area);
}

fn report_lines(app: &App, width: usize) -> (Vec<Line<'static>>, Vec<Option<usize>>) {
    let rows = app.report_rows();
    if rows.is_empty() {
        return (
            vec![
                Line::from("No cases or reports yet.").style(theme::dim()),
                Line::from(
                    "+ saves a case and selected first actions. Reports are optional after review.",
                )
                .style(theme::dim()),
            ],
            vec![None, None],
        );
    }
    let mut lines = Vec::new();
    let mut index = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        let selected = i == app.report_sel;
        let status_style = if selected && app.focus == Focus::Reports {
            theme::selected()
        } else if selected {
            theme::accent()
        } else {
            match row.status() {
                "pending" => Style::default().fg(theme::WARN).bg(theme::BG),
                "failed" => Style::default().fg(theme::RED).bg(theme::BG),
                _ => Style::default().fg(theme::GREEN).bg(theme::BG),
            }
        };
        let open = matches!(
            (&row, app.chat_report.as_deref()),
            (super::app::ReportRow::Completed(report), Some(id)) if report.id == id
        );
        let mark = if open { "● " } else { "" };
        let title_line = clip_chars(
            &format!("{mark}{:<10} {}", row.status(), row.title()),
            width,
        );
        lines.push(Line::from(title_line).style(status_style));
        index.push(Some(i));
        if let super::app::ReportRow::Completed(report) = row {
            if let Some((n, hit)) = app
                .recommendations
                .iter()
                .enumerate()
                .find(|(_, h)| h.report_id == report.id)
            {
                lines.push(
                    Line::from(clip_chars(
                        &format!("/cite {} · {} · {}", n + 1, hit.section, hit.reason),
                        width,
                    ))
                    .style(theme::accent()),
                );
                index.push(Some(i));
            }
        }
        if let Some(when) = row.when() {
            lines.push(Line::from(when).style(theme::dim()));
            index.push(Some(i));
        }
    }
    lines.push(
        Line::from(clip_chars(
            "Tab focuses this list · Enter or click asks to open · Esc returns to the desk",
            width,
        ))
        .style(theme::dim()),
    );
    index.push(None);
    (lines, index)
}

fn clip_chars(text: &str, width: usize) -> String {
    text.chars().take(width.max(1)).collect()
}

fn draw_launcher(frame: &mut Frame, app: &mut App, area: Rect) {
    app.launcher_area = area;
    let items: Vec<ListItem> = ModuleId::all()
        .iter()
        .enumerate()
        .map(|(i, module)| {
            let active = app.module == Some(*module)
                || (*module == ModuleId::Cases
                    && matches!(app.module, None | Some(ModuleId::Cases)));
            let mark = if active { ">" } else { " " };
            ListItem::new(Line::from(vec![Span::styled(
                format!(" {mark} {}. {} ", i + 1, module.title()),
                theme::text(),
            )]))
        })
        .collect();
    let mut state = ListState::default();
    state.select(Some(app.launcher_sel));
    let title = if app.focus == Focus::Launcher {
        " Apps "
    } else {
        " Apps "
    };
    let list = List::new(items)
        .block(panel(title))
        .highlight_style(theme::selected())
        .highlight_symbol("");
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_canvas(frame: &mut Frame, app: &App, area: Rect) {
    let focused = app.focus == Focus::Canvas;
    let title = if focused {
        format!(" {} · chat ", app.view_name())
    } else {
        format!(" {} ", app.view_name())
    };
    let lines = canvas_lines(
        app,
        area.width.saturating_sub(2) as usize,
        area.height.saturating_sub(2) as usize,
    );
    let block = if focused {
        panel(&title).border_style(Style::default().fg(theme::ACCENT))
    } else {
        panel(&title)
    };
    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

fn canvas_lines(app: &App, width: usize, height: usize) -> Vec<Line<'static>> {
    let transcript = transcript_lines(app, width);
    let lines = if transcript.is_empty() {
        vec![
            Line::from(
                "Ask saved case evidence and reports. + reviews investigation scope. /case opens saved leads. /clear wipes this chat.",
            )
                .style(theme::dim()),
        ]
    } else {
        transcript
    };
    if app.scroll_back > 0 {
        let mut window = tail(lines, height.saturating_sub(1));
        window.push(
            Line::from("scrolled up · Down, PgDn, or End returns to the latest")
                .style(theme::dim()),
        );
        return window;
    }
    tail(lines, height)
}

fn under_case_tabs(frame: &mut Frame, app: &mut App, area: Rect) -> Rect {
    if app.investigation.is_some() {
        return case_view::under_tabs(frame, app, area);
    }
    if app.module != Some(ModuleId::Cases) || area.height < 7 {
        app.case_tab_area = Rect::default();
        app.case_tab_hits.clear();
        return area;
    }
    let rows = split_v(area, &[Constraint::Length(3), Constraint::Min(4)]);
    draw_case_tabs(frame, app, rows[0]);
    rows[1]
}

fn draw_case_tabs(frame: &mut Frame, app: &mut App, area: Rect) {
    app.case_tab_area = area;
    let titles: Vec<Line> = app
        .case_pages()
        .into_iter()
        .map(|page| Line::from(format!(" {} ", page.title())))
        .collect();
    app.case_tab_hits = tab_hits(area, &titles);
    let selected = app
        .case_pages()
        .iter()
        .position(|page| *page == app.case_page)
        .unwrap_or(0);
    frame.render_widget(
        Tabs::new(titles)
            .block(panel(" Case Desk "))
            .select(selected)
            .highlight_style(theme::selected())
            .divider("")
            .padding(" ", " "),
        area,
    );
}

fn draw_system_tabs(frame: &mut Frame, app: &mut App, area: Rect) {
    app.system_tab_area = area;
    let titles: Vec<Line> = SystemPage::all()
        .into_iter()
        .map(|page| Line::from(format!(" {} ", page.title())))
        .collect();
    app.system_tab_hits = tab_hits(area, &titles);
    let selected = SystemPage::all()
        .iter()
        .position(|page| *page == app.system_page)
        .unwrap_or(0);
    frame.render_widget(
        Tabs::new(titles)
            .block(panel(" System "))
            .select(selected)
            .highlight_style(theme::selected())
            .divider("")
            .padding(" ", " "),
        area,
    );
}

fn draw_provider_tabs(frame: &mut Frame, app: &mut App, area: Rect) {
    app.provider_tab_area = area;
    let titles: Vec<Line> = ProviderPage::all()
        .into_iter()
        .map(|page| Line::from(format!(" {} ", page.title())))
        .collect();
    app.provider_tab_hits = tab_hits(area, &titles);
    let selected = ProviderPage::all()
        .iter()
        .position(|page| *page == app.provider_page)
        .unwrap_or(0);
    frame.render_widget(
        Tabs::new(titles)
            .block(panel(" Providers "))
            .select(selected)
            .highlight_style(theme::selected())
            .divider("")
            .padding(" ", " "),
        area,
    );
}

/// Matches ratatui's left-packed tabs: one space, the label, one space, inside the border.
fn tab_hits(area: Rect, titles: &[Line]) -> Vec<Rect> {
    if area.width < 3 || area.height < 2 {
        return Vec::new();
    }
    let mut x = area.x + 1;
    let right = area.x + area.width - 1;
    let y = area.y + 1;
    let mut hits = Vec::new();
    for title in titles {
        if x >= right {
            break;
        }
        let width = (1 + title.width() as u16 + 1).min(right - x);
        hits.push(Rect {
            x,
            y,
            width,
            height: 1,
        });
        x = x.saturating_add(width);
    }
    hits
}

fn draw_widget(frame: &mut Frame, app: &mut App, area: Rect) {
    let Some(widget) = app.widget() else {
        return;
    };
    frame.render_widget(Clear, area);
    let mut area = area;
    if app.module == Some(ModuleId::Providers) && area.height >= 7 {
        let rows = split_v(area, &[Constraint::Length(3), Constraint::Min(4)]);
        draw_provider_tabs(frame, app, rows[0]);
        area = rows[1];
        app.system_tab_area = Rect::default();
        app.system_tab_hits.clear();
    } else if app.module == Some(ModuleId::System) && area.height >= 7 {
        let rows = split_v(area, &[Constraint::Length(3), Constraint::Min(4)]);
        draw_system_tabs(frame, app, rows[0]);
        area = rows[1];
        app.provider_tab_area = Rect::default();
        app.provider_tab_hits.clear();
    } else {
        app.provider_tab_area = Rect::default();
        app.provider_tab_hits.clear();
        app.system_tab_area = Rect::default();
        app.system_tab_hits.clear();
    }
    if app.module == Some(ModuleId::Providers) && app.provider_page != ProviderPage::Osint {
        providers::draw(frame, app, area);
        return;
    }
    if widget == ModuleId::Hardware && area.width >= 40 && area.height >= 8 {
        draw_gauges(frame, app, area, 2);
        return;
    }
    let title = match app.module {
        Some(ModuleId::Cases) => format!(" {} ", app.case_page.title()),
        Some(ModuleId::Providers) => format!(" {} ", app.provider_page.title()),
        Some(ModuleId::System) => format!(" {} ", app.system_page.title()),
        _ => format!(" {} ", widget.title()),
    };
    let lines = if app.module == Some(ModuleId::Providers) {
        provider_lines(app, area.height.saturating_sub(2) as usize)
    } else if app.module == Some(ModuleId::Cases) {
        case_side_lines(app, area.height.saturating_sub(2) as usize)
    } else if widget == ModuleId::Settings {
        field_lines(app)
    } else {
        widget_lines(app, widget, area.height.saturating_sub(2) as usize)
    };
    frame.render_widget(
        Paragraph::new(lines)
            .block(panel(&title))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn widget_lines(app: &App, widget: ModuleId, height: usize) -> Vec<Line<'static>> {
    let lines = match widget {
        ModuleId::Osint => osint_lines(app, height),
        ModuleId::Hardware => vec![
            Line::from(app.hardware.one_line()),
            Line::from(format!(
                "CPU {cpu:.0}%   RAM {ram}%   Disk {disk}%   {backend}",
                cpu = app.cpu_now,
                ram = app.hardware.ram_pct(),
                disk = app.hardware.disk_pct(),
                backend = app.hardware.backend,
            )),
            Line::from("r rescan".to_string()).style(theme::dim()),
        ],
        ModuleId::Brain => brain_list_lines(app, height),
        ModuleId::Reports => {
            if app.reports.is_empty() {
                vec![Line::from(
                    "No reports yet. A case search writes markdown.".to_string(),
                )]
            } else {
                app.reports
                    .iter()
                    .take(12)
                    .map(|report| Line::from(format!("{}  {}", report.title, report.path)))
                    .collect()
            }
        }
        ModuleId::Log => log_lines(app, height),
        ModuleId::Providers | ModuleId::Gmail | ModuleId::Settings => field_lines(app),
        ModuleId::Cases | ModuleId::System => Vec::new(),
    };
    tail(lines, height)
}

fn log_lines(app: &App, height: usize) -> Vec<Line<'static>> {
    if app.log.is_empty() {
        return tail(
            vec![Line::from(
                "System calls, API calls, failed tasks, and searches from this session land here."
                    .to_string(),
            )
            .style(theme::dim())],
            height,
        );
    }
    let lines = app
        .log
        .iter()
        .map(|entry| {
            let failed = {
                let lower = entry.text.to_lowercase();
                lower.contains("fail") || lower.contains("error")
            };
            let style = if failed {
                Style::default().fg(theme::RED).bg(theme::BG)
            } else {
                theme::dim()
            };
            Line::from(format!("{}  {:<7} {}", entry.at, entry.kind, entry.text)).style(style)
        })
        .collect();
    tail(lines, height)
}

fn field_lines(app: &App) -> Vec<Line<'static>> {
    app.fields
        .iter()
        .enumerate()
        .map(|(i, field)| {
            if field.key.starts_with("__h_") {
                let style = if i == app.field_sel {
                    theme::selected()
                } else {
                    theme::accent()
                };
                return Line::from(field.label.clone()).style(style);
            }
            let shown = if field.secret {
                mask(&field.value)
            } else {
                field.value.clone()
            };
            let cursor = if app.editing && i == app.field_sel {
                "▍"
            } else {
                ""
            };
            let style = if i == app.field_sel {
                theme::selected()
            } else {
                theme::text()
            };
            Line::from(format!(
                "{:<28} {}",
                field.label,
                format!("{shown}{cursor}")
            ))
            .style(style)
        })
        .collect()
}

fn case_side_lines(app: &App, height: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let body = match app.case_page {
        CasePage::Brain => brain_list_lines(app, height.saturating_sub(3)),
        CasePage::Closed => vec![Line::from("Case desk only.".to_string())],
        CasePage::Network => vec![Line::from("Historical report network".to_string())],
        CasePage::Investigation => vec![Line::from("Case investigation".to_string())],
    };
    lines.extend(body);
    tail(lines, height)
}

fn provider_lines(app: &App, height: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let body = match app.provider_page {
        ProviderPage::Osint => osint_lines(app, height.saturating_sub(3)),
        _ => field_lines(app),
    };
    lines.extend(body);
    tail(lines, height)
}

fn osint_lines(app: &App, height: usize) -> Vec<Line<'static>> {
    let mut lines = field_lines(app);
    lines.push(Line::from(""));
    if app.settings.sources.is_empty() {
        lines.push(
            Line::from("No extra sources. Add a public URL template with {query}.".to_string())
                .style(theme::dim()),
        );
    }
    for (i, source) in app.settings.sources.iter().enumerate() {
        let mark = if source.enabled { "on " } else { "off" };
        let style = if i == app.source_sel {
            theme::selected()
        } else {
            theme::text()
        };
        lines.push(
            Line::from(format!("{mark}  {}  {}", source.name, source.url_template)).style(style),
        );
    }
    lines.push(
        Line::from("[ ] move · t toggle · x delete an extra source".to_string())
            .style(theme::dim()),
    );
    tail(lines, height)
}

fn transcript_lines(app: &App, width: usize) -> Vec<Line<'static>> {
    let all = app.transcript();
    let mut lines = Vec::new();
    for (index, message) in all.iter().enumerate() {
        if index > 0 {
            lines.push(Line::from(""));
        }
        lines.extend(render_message(&message.role, &message.body, width));
    }
    let skip = app.scroll_back.min(lines.len().saturating_sub(1));
    let end = lines.len().saturating_sub(skip);
    lines.truncate(end);
    lines
}

fn render_message(role: &str, body: &str, width: usize) -> Vec<Line<'static>> {
    let width = width.max(4);
    match role {
        "user" => {
            let content_width = width.saturating_sub(2).max(1);
            let prefix_style = theme::user_message()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::BOLD);
            markdown_lines(body, content_width, theme::user_message())
                .into_iter()
                .enumerate()
                .map(|(index, mut line)| {
                    let prefix = if index == 0 { "❯ " } else { "  " };
                    let used: usize = line.spans.iter().map(|span| span.content.width()).sum();
                    let pad = content_width.saturating_sub(used);
                    if pad > 0 {
                        line.push_span(Span::styled(" ".repeat(pad), theme::user_message()));
                    }
                    let mut spans = vec![Span::styled(prefix, prefix_style)];
                    spans.extend(line.spans);
                    Line::from(spans)
                })
                .collect()
        }
        "assistant" => markdown_lines(body, width, theme::text()),
        _ => markdown_lines(body, width, theme::dim()),
    }
}

struct Piece {
    text: String,
    style: Style,
}

fn markdown_lines(body: &str, width: usize, base: Style) -> Vec<Line<'static>> {
    let width = width.max(1);
    let mut lines = Vec::new();
    let mut in_fence = false;
    for raw in body.split('\n') {
        let trimmed = raw.trim();
        if trimmed.starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            lines.extend(wrap_pieces(
                &[Piece {
                    text: raw.to_string(),
                    style: code_style(base),
                }],
                width,
            ));
            continue;
        }
        if trimmed.is_empty() {
            lines.push(Line::from(""));
            continue;
        }
        let (prefix, rest, heading) = block_prefix(trimmed);
        let mut pieces = Vec::new();
        if let Some(prefix) = prefix {
            pieces.push(Piece {
                text: prefix,
                style: base.fg(theme::ACCENT).add_modifier(Modifier::BOLD),
            });
        }
        let body_style = if heading {
            base.fg(theme::ACCENT).add_modifier(Modifier::BOLD)
        } else {
            base
        };
        pieces.extend(inline_pieces(rest, body_style));
        lines.extend(wrap_pieces(&pieces, width));
    }
    if lines.is_empty() {
        lines.push(Line::from(""));
    }
    lines
}

fn block_prefix(line: &str) -> (Option<String>, &str, bool) {
    let hashes = line.chars().take_while(|ch| *ch == '#').count();
    if (1..=6).contains(&hashes) && line.chars().nth(hashes) == Some(' ') {
        return (None, line[hashes + 1..].trim(), true);
    }
    if let Some(rest) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
        return (Some("• ".into()), rest, false);
    }
    let mut digits = 0;
    for ch in line.chars() {
        if ch.is_ascii_digit() {
            digits += 1;
        } else {
            break;
        }
    }
    if digits > 0 && line[digits..].starts_with(". ") {
        let marker: String = line[..digits + 2].to_string();
        return (Some(marker), &line[digits + 2..], false);
    }
    (None, line, false)
}

fn inline_pieces(text: &str, base: Style) -> Vec<Piece> {
    let mut pieces = Vec::new();
    let mut buf = String::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    let flush = |buf: &mut String, pieces: &mut Vec<Piece>, style: Style| {
        if !buf.is_empty() {
            pieces.push(Piece {
                text: std::mem::take(buf),
                style,
            });
        }
    };
    while i < chars.len() {
        if chars[i] == '`' {
            if let Some(end) = chars[i + 1..].iter().position(|ch| *ch == '`') {
                flush(&mut buf, &mut pieces, base);
                let inner: String = chars[i + 1..i + 1 + end].iter().collect();
                pieces.push(Piece {
                    text: inner,
                    style: code_style(base),
                });
                i += end + 2;
                continue;
            }
        }
        if starts_with(&chars, i, "**") || starts_with(&chars, i, "__") {
            let marker = &chars[i..i + 2];
            if let Some(end) = find_marker(&chars, i + 2, marker) {
                flush(&mut buf, &mut pieces, base);
                let inner: String = chars[i + 2..end].iter().collect();
                pieces.extend(inline_pieces(&inner, base.add_modifier(Modifier::BOLD)));
                i = end + 2;
                continue;
            }
        }
        if chars[i] == '*' || chars[i] == '_' {
            let marker = [chars[i]];
            if let Some(end) = find_marker(&chars, i + 1, &marker) {
                flush(&mut buf, &mut pieces, base);
                let inner: String = chars[i + 1..end].iter().collect();
                pieces.extend(inline_pieces(&inner, base.add_modifier(Modifier::ITALIC)));
                i = end + 1;
                continue;
            }
        }
        if chars[i] == '[' {
            if let Some((label, url, next)) = parse_link(&chars, i) {
                flush(&mut buf, &mut pieces, base);
                pieces.push(Piece {
                    text: label,
                    style: base.fg(theme::ACCENT).add_modifier(Modifier::BOLD),
                });
                if !url.is_empty() {
                    pieces.push(Piece {
                        text: format!(" ({url})"),
                        style: base.fg(theme::DIM),
                    });
                }
                i = next;
                continue;
            }
        }
        buf.push(chars[i]);
        i += 1;
    }
    flush(&mut buf, &mut pieces, base);
    pieces
}

fn code_style(base: Style) -> Style {
    base.fg(theme::ACCENT)
}

fn starts_with(chars: &[char], index: usize, marker: &str) -> bool {
    chars[index..]
        .iter()
        .take(marker.chars().count())
        .collect::<String>()
        == marker
}

fn find_marker(chars: &[char], from: usize, marker: &[char]) -> Option<usize> {
    if marker.is_empty() || from >= chars.len() {
        return None;
    }
    chars[from..]
        .windows(marker.len())
        .position(|window| window == marker)
        .map(|pos| from + pos)
}

fn parse_link(chars: &[char], start: usize) -> Option<(String, String, usize)> {
    let close = chars[start + 1..].iter().position(|ch| *ch == ']')? + start + 1;
    if chars.get(close + 1) != Some(&'(') {
        return None;
    }
    let end = chars[close + 2..].iter().position(|ch| *ch == ')')? + close + 2;
    let label: String = chars[start + 1..close].iter().collect();
    let url: String = chars[close + 2..end].iter().collect();
    Some((label, url, end + 1))
}

fn wrap_pieces(pieces: &[Piece], width: usize) -> Vec<Line<'static>> {
    let mut lines: Vec<Vec<Piece>> = vec![Vec::new()];
    let mut col = 0usize;
    for piece in pieces {
        let mut rest = piece.text.as_str();
        while !rest.is_empty() {
            if col >= width {
                lines.push(Vec::new());
                col = 0;
            }
            let room = width - col;
            let (head, tail, broken) = take_width(rest, room);
            if head.is_empty() && broken {
                lines.push(Vec::new());
                col = 0;
                continue;
            }
            if !head.is_empty() {
                lines.last_mut().unwrap().push(Piece {
                    text: head,
                    style: piece.style,
                });
                col += UnicodeWidthStr::width(lines.last().unwrap().last().unwrap().text.as_str());
            }
            rest = tail.trim_start();
            if !rest.is_empty() {
                lines.push(Vec::new());
                col = 0;
            }
        }
    }
    if lines.iter().all(|line| line.is_empty()) {
        return vec![Line::from("")];
    }
    lines
        .into_iter()
        .map(|line| {
            Line::from(
                line.into_iter()
                    .map(|piece| Span::styled(piece.text, piece.style))
                    .collect::<Vec<_>>(),
            )
        })
        .collect()
}

fn take_width(text: &str, width: usize) -> (String, &str, bool) {
    if text.width() <= width {
        return (text.to_string(), "", false);
    }
    let mut end = 0usize;
    let mut used = 0usize;
    let mut last_space = None;
    for (index, ch) in text.char_indices() {
        let w = UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + w > width {
            break;
        }
        used += w;
        end = index + ch.len_utf8();
        if ch == ' ' {
            last_space = Some(end);
        }
    }
    if let Some(space) = last_space.filter(|space| *space > 0 && *space < end) {
        return (text[..space].to_string(), &text[space..], true);
    }
    if end == 0 {
        return (String::new(), text, true);
    }
    (text[..end].to_string(), &text[end..], true)
}

fn draw_gauges(frame: &mut Frame, app: &App, area: Rect, cols: u16) {
    let specs: [(&str, u16, String); 9] = [
        (
            "CPU",
            app.cpu_now.round().clamp(0.0, 100.0) as u16,
            format!("{:.0}%", app.cpu_now),
        ),
        (
            "Memory",
            app.hardware.ram_pct(),
            format!("{:.1} GB", app.hardware.total_ram_gb),
        ),
        (
            "GPU",
            if app.hardware.has_gpu { 70 } else { 0 },
            app.hardware
                .gpu_name
                .clone()
                .unwrap_or_else(|| app.hardware.backend.clone()),
        ),
        (
            "Disk",
            app.hardware.disk_pct(),
            format!("{:.0} GB free", app.hardware.disk_available_gb),
        ),
        (
            "Provider",
            if app.auth.text.is_some() { 100 } else { 8 },
            super::app::provider_label(app.auth.text.as_ref()),
        ),
        (
            "Brain",
            (app.memories.len().min(20) as u16) * 5,
            format!("{} memories", app.memories.len()),
        ),
        (
            "Gmail",
            if app.auth.gmail.is_some() { 100 } else { 8 },
            app.auth
                .gmail
                .as_ref()
                .map(|g| g.email.clone())
                .unwrap_or_else(|| "not connected".into()),
        ),
        (
            "Cases",
            if app.cases.is_empty() { 8 } else { 60 },
            format!("{} open", app.cases.len()),
        ),
        (
            "Reports",
            if app.reports.is_empty() { 8 } else { 60 },
            app.reports
                .first()
                .map(|r| r.title.clone())
                .unwrap_or_else(|| "none".into()),
        ),
    ];
    let rows = if cols == 2 { 2 } else { 3 };
    let row_areas = split_v(
        area,
        &vec![Constraint::Percentage(100 / rows); rows as usize],
    );
    let mut idx = 0;
    for row in row_areas.iter().take(rows as usize) {
        let col_areas = split_h(
            *row,
            &vec![Constraint::Percentage(100 / cols); cols as usize],
        );
        for col in col_areas.iter().take(cols as usize) {
            if idx >= specs.len() {
                break;
            }
            let (name, pct, label) = &specs[idx];
            let gauge = Gauge::default()
                .block(panel(&format!(" {name} ")))
                .gauge_style(Style::default().fg(theme::ACCENT).bg(theme::BG))
                .percent(*pct)
                .label(label.clone());
            frame.render_widget(gauge, *col);
            if *name == "CPU" && !app.cpu_hist.is_empty() {
                let spark_area = Rect {
                    x: col.x + 1,
                    y: col.y + col.height.saturating_sub(2),
                    width: col.width.saturating_sub(2),
                    height: 1,
                };
                let data: Vec<u64> = app.cpu_hist.iter().copied().collect();
                frame.render_widget(
                    Sparkline::default().data(&data).style(theme::accent()),
                    spark_area,
                );
            }
            idx += 1;
        }
    }
}

fn draw_prompt(frame: &mut Frame, app: &App, area: Rect) {
    let width = area.width.saturating_sub(2) as usize;
    let title = fit_status(width, &app.mode_label(), &app.db_label);
    let input = prompt_line(app);
    let hint_style = if app.status.contains("error") {
        Style::default().fg(theme::RED).bg(theme::BG)
    } else if app.status == "listening" {
        Style::default().fg(theme::WARN).bg(theme::BG)
    } else {
        theme::dim()
    };
    let hint = if app.chat_report.is_some() && app.status != "ready" && !app.running {
        vec![Line::from(app.status.clone()).style(hint_style)]
    } else if app.chat_report.is_some() {
        vec![Line::from(if app.running {
            "Enter replaces question · Ctrl+C cancels"
        } else {
            "Enter asks about report evidence · /clear dismisses answer"
        })
        .style(hint_style)]
    } else if app.prompt.starts_with('/') && !app.prompt.contains(' ') {
        let card = super::slash_menu(&app.prompt);
        card.options
            .iter()
            .take(3)
            .map(|opt| Line::from(format!("{}  {}", opt.label, opt.detail)).style(hint_style))
            .collect()
    } else if !app.status.is_empty() && app.status != "ready" {
        vec![Line::from(app.status.clone()).style(hint_style)]
    } else if app.settings.modality == "voice" {
        vec![Line::from("voice · Ctrl+R record · Enter send").style(hint_style)]
    } else {
        vec![Line::from("Enter send · + case scope · /help").style(hint_style)]
    };
    let mut lines = vec![input];
    lines.extend(hint);
    let border = if app.focus == Focus::Prompt {
        panel(&format!(" {title} "))
    } else {
        panel(&format!(" {title} "))
    };
    frame.render_widget(Paragraph::new(lines).block(border), area);
}

fn prompt_line(app: &App) -> Line<'static> {
    if app.chat_report.is_some() && app.prompt.is_empty() {
        return Line::from(vec![
            Span::styled("❯ ", theme::accent()),
            Span::styled("ask the open graph…", theme::dim()),
        ]);
    }
    let chars: Vec<char> = app.prompt.chars().collect();
    let cursor = app.cursor.min(chars.len());
    let before: String = chars[..cursor].iter().collect();
    let current: String = chars
        .get(cursor)
        .map(|c| c.to_string())
        .unwrap_or_else(|| " ".into());
    let after: String = chars.get(cursor + 1..).unwrap_or(&[]).iter().collect();
    let caret = if app.focus == Focus::Prompt {
        Style::default().fg(theme::BG).bg(theme::ACCENT)
    } else {
        theme::dim()
    };
    Line::from(vec![
        Span::styled(
            "❯ ",
            Style::default()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(before, theme::text()),
        Span::styled(current, caret),
        Span::styled(after, theme::text()),
    ])
}

fn draw_footer(frame: &mut Frame, app: &App, area: Rect) {
    if app.investigation.is_some() && matches!(app.module, None | Some(ModuleId::Cases)) {
        frame.render_widget(
            Paragraph::new(
                "1 Desk · 2 Work · 3 Graph · 4 Product · e plan · / commands · Esc Desk",
            )
            .style(theme::dim()),
            area,
        );
        return;
    }
    if app.case_page == CasePage::Network && matches!(app.module, None | Some(ModuleId::Cases)) {
        draw_network_footer(frame, app, area);
        return;
    }
    if app.module == Some(ModuleId::Providers) {
        frame.render_widget(
            Paragraph::new(if app.editing {
                "Editing · Ctrl+U clear · Enter finish · Save keeps this account"
            } else {
                "←/→ section · j/k field · Enter select/edit · Tab focus · Esc Desk"
            })
            .style(theme::dim()),
            area,
        );
        return;
    }
    let left = "[Ctrl+P] App Search";
    let right = match app.focus {
        Focus::Prompt => "[Tab] Jump Focus | [Esc] Exit App",
        Focus::Canvas | Focus::Graph => "[Up/Down] Scroll chat  [Wheel] Scroll  [Tab] Jump",
        Focus::TableDetail => "[j/k] Browse graph  [Tab] Jump",
        Focus::Reports => "[j/k] Move  [Enter] Open  [Tab] Jump",
        Focus::Launcher => "[Tab] Jump Focus | [Esc] Exit App",
    };
    let gap = area.width as usize;
    let used = left.chars().count() + right.chars().count();
    let spaces = gap.saturating_sub(used);
    let line = format!("{left}{}{right}", " ".repeat(spaces));
    frame.render_widget(Paragraph::new(line).style(theme::dim()), area);
}

fn draw_modal(frame: &mut Frame, app: &App, area: Rect) {
    let modal = centered(
        area,
        area.width.saturating_mul(2) / 3,
        area.height.saturating_mul(2) / 3,
    );
    frame.render_widget(Clear, modal);
    let modules = app.filtered_modules();
    let mut lines = vec![
        Line::from("Search Applications").style(theme::accent()),
        Line::from(format!(
            "> {}",
            if app.modal_query.is_empty() {
                "type to filter"
            } else {
                app.modal_query.as_str()
            }
        )),
        Line::from(""),
    ];
    for (i, module) in modules.iter().enumerate() {
        let style = if i == app.modal_sel {
            theme::selected()
        } else {
            theme::text()
        };
        lines.push(Line::from(format!("{}  {}", module.title(), module.blurb())).style(style));
    }
    if modules.is_empty() {
        lines.push(Line::from("No matching app").style(theme::dim()));
    }
    lines.push(Line::from(""));
    lines.push(Line::from("[Enter] Select   [Esc] Close").style(theme::dim()));
    frame.render_widget(
        Paragraph::new(lines)
            .block(panel(" Search Applications "))
            .wrap(Wrap { trim: false }),
        modal,
    );
}

fn draw_model_picker(frame: &mut Frame, app: &App, area: Rect) {
    let modal = centered(
        area,
        area.width.saturating_mul(2) / 3,
        area.height.saturating_mul(2) / 3,
    );
    frame.render_widget(Clear, modal);
    let choices = app.filtered_model_choices();
    let current = app.picker_current();
    let router_selected = choices
        .get(app.model_sel)
        .map(|(id, _)| argos_osint_core::provider::is_free_router(id))
        .unwrap_or(false);
    let mut lines = vec![
        Line::from(format!(
            "{} · {}",
            app.picker_title(),
            super::app::provider_name(&argos_osint_core::provider::effective_kind(
                &app.picker_secret()
            ))
        ))
        .style(theme::accent()),
        Line::from(format!(
            "> {}",
            if app.model_query.is_empty() {
                "type to filter"
            } else {
                app.model_query.as_str()
            }
        )),
        Line::from(""),
    ];
    let visible = modal.height.saturating_sub(9).max(1) as usize;
    let offset = app.model_sel.saturating_sub(visible.saturating_sub(1));
    for (i, (id, label)) in choices.iter().enumerate().skip(offset).take(visible) {
        let mark = if *id == current { "●" } else { " " };
        let style = if i == app.model_sel {
            theme::selected()
        } else {
            theme::text()
        };
        let name = if label == id {
            label.clone()
        } else {
            format!("{label}  {id}")
        };
        lines.push(Line::from(format!("{mark} {name}")).style(style));
    }
    if choices.is_empty() {
        lines.push(Line::from("No models match").style(theme::dim()));
    }
    lines.push(Line::from(""));
    let hint = if router_selected {
        "[Enter] List free models   [Esc] Close"
    } else {
        "[Enter] Use model   [F5] Refresh   [Esc] Close"
    };
    lines.push(Line::from(hint).style(theme::dim()));
    frame.render_widget(
        Paragraph::new(lines)
            .block(panel(" Model "))
            .wrap(Wrap { trim: false }),
        modal,
    );
}

fn draw_free_picker(frame: &mut Frame, app: &App, area: Rect) {
    let modal = centered(
        area,
        area.width.saturating_mul(3) / 4,
        area.height.saturating_mul(3) / 4,
    );
    frame.render_widget(Clear, modal);
    let choices = app.filtered_free_models();
    let mut lines = vec![
        Line::from("Free models").style(theme::accent()),
        Line::from("openrouter/free would choose one of these at random.").style(theme::dim()),
        Line::from(format!(
            "> {}",
            if app.free_query.is_empty() {
                "type to filter"
            } else {
                app.free_query.as_str()
            }
        )),
        Line::from(""),
    ];
    if app.free_loading && choices.is_empty() {
        lines.push(Line::from("Loading free models…").style(theme::dim()));
    }
    for (i, (id, label)) in choices.iter().enumerate() {
        let style = if i == app.free_sel {
            theme::selected()
        } else {
            theme::text()
        };
        let name = if label == id {
            id.clone()
        } else {
            format!("{label}  {id}")
        };
        lines.push(Line::from(name).style(style));
    }
    if !app.free_loading && choices.is_empty() {
        lines.push(Line::from("No free models were listed.").style(theme::dim()));
    }
    lines.push(Line::from(""));
    lines.push(Line::from("[Enter] Use this model   [Esc] Back").style(theme::dim()));
    frame.render_widget(
        Paragraph::new(lines)
            .block(panel(" Free models "))
            .wrap(Wrap { trim: false }),
        modal,
    );
}

fn draw_report_confirm(frame: &mut Frame, app: &App, area: Rect) {
    let modal = centered(
        area,
        area.width.saturating_mul(2) / 3,
        area.height.saturating_mul(1) / 2,
    );
    frame.render_widget(Clear, modal);
    let title = app
        .confirm_report
        .as_deref()
        .and_then(|id| app.reports.iter().find(|report| report.id == id))
        .map(|report| report.title.clone())
        .unwrap_or_else(|| "Report".into());
    let style_for = |index: usize| {
        if app.confirm_report_sel == index {
            theme::selected()
        } else {
            theme::text()
        }
    };
    let text = vec![
        Line::from("Open this report?").style(theme::accent()),
        Line::from(""),
        Line::from(title).style(theme::text()),
        Line::from(""),
        Line::from(
            "Opening preserves the historical report and citations. Answers are temporary; /retain-answer explicitly saves a fact. Cases can be investigated before any report exists.",
        )
        .style(theme::dim()),
        Line::from(""),
        Line::from("Open report network").style(style_for(0)),
        Line::from("Delete report").style(style_for(1)),
        Line::from("Cancel").style(style_for(2)),
        Line::from(""),
        Line::from("Enter confirms · Esc cancels · j/k move").style(theme::dim()),
    ];
    frame.render_widget(
        Paragraph::new(text)
            .block(panel(" Report "))
            .wrap(Wrap { trim: false }),
        modal,
    );
}

fn draw_delete_report_confirm(frame: &mut Frame, app: &App, area: Rect) {
    let modal = centered(
        area,
        area.width.saturating_mul(2) / 3,
        area.height.saturating_mul(1) / 2,
    );
    frame.render_widget(Clear, modal);
    let title = app
        .confirm_delete_report
        .as_deref()
        .and_then(|id| app.reports.iter().find(|report| report.id == id))
        .map(|report| report.title.clone())
        .unwrap_or_else(|| "Report".into());
    let delete = if app.confirm_delete_sel == 0 {
        theme::selected()
    } else {
        theme::text()
    };
    let cancel = if app.confirm_delete_sel == 1 {
        theme::selected()
    } else {
        theme::text()
    };
    let text = vec![
        Line::from("Delete this report?").style(theme::accent()),
        Line::from(""),
        Line::from(title).style(theme::text()),
        Line::from(""),
        Line::from("This deletes the markdown file and every fact memory taken from this report.")
            .style(theme::dim()),
        Line::from(""),
        Line::from("Delete report and memories").style(delete),
        Line::from("Cancel").style(cancel),
        Line::from(""),
        Line::from("Enter confirms · Esc cancels · j/k move").style(theme::dim()),
    ];
    frame.render_widget(
        Paragraph::new(text)
            .block(panel(" Delete report "))
            .wrap(Wrap { trim: false }),
        modal,
    );
}

fn draw_scope(frame: &mut Frame, app: &App, area: Rect) {
    let Some(scope) = app.scope.as_ref() else {
        return;
    };
    let modal = centered(
        area,
        area.width.saturating_mul(2) / 3,
        area.height.saturating_mul(3) / 4,
    );
    frame.render_widget(Clear, modal);
    let rows = [
        ("Facts", scope.facts),
        ("Web", scope.web),
        ("News", scope.news),
        ("Domain", scope.domain),
        ("Social", scope.social),
        ("Identity", scope.identity),
    ];
    let mut text = vec![
        Line::from("Research scope").style(theme::accent()),
        Line::from(""),
        Line::from(scope.query.clone()).style(theme::text()),
        Line::from(format!(
            "Case: {} · c selects existing/new",
            scope.existing_case.as_deref().unwrap_or("new case")
        ))
        .style(theme::dim()),
        Line::from(format!(
            "Seeds: {}",
            argos_osint_core::search::TextQuery::extract(&scope.query)
                .domains
                .join(", ")
        ))
        .style(theme::dim()),
        Line::from(format!(
            "Include {} existing passages: {} · e toggles",
            app.recommendations.len(),
            scope.include_evidence
        ))
        .style(theme::dim()),
        Line::from(format!(
            "Allowed: public sources · sensitive {} (s) · active HTTP {} (a)",
            scope.allow_sensitive, scope.allow_active
        ))
        .style(theme::dim()),
        Line::from("Domain lookups run only when the query names a domain.").style(theme::dim()),
        Line::from(""),
    ];
    for (index, (label, on)) in rows.iter().enumerate() {
        let mark = if *on { "[x]" } else { "[ ]" };
        let style = if index == scope.selected {
            theme::selected()
        } else {
            theme::text()
        };
        text.push(Line::from(format!("{mark} {label}")).style(style));
    }
    text.push(Line::from(""));
    text.push(
        Line::from("Space selects first actions · Enter saves case · Esc cancels · j/k move")
            .style(theme::dim()),
    );
    frame.render_widget(
        Paragraph::new(text)
            .block(panel(" Case scope "))
            .wrap(Wrap { trim: false }),
        modal,
    );
}

fn draw_case_confirm(frame: &mut Frame, app: &App, area: Rect) {
    let modal = centered(
        area,
        area.width.saturating_mul(2) / 3,
        area.height.saturating_mul(1) / 2,
    );
    frame.render_widget(Clear, modal);
    let title = app.confirm_query.clone().unwrap_or_default();
    let yes = if app.confirm_sel == 0 {
        theme::selected()
    } else {
        theme::text()
    };
    let no = if app.confirm_sel == 1 {
        theme::selected()
    } else {
        theme::text()
    };
    let text = vec![
        Line::from("Review an investigation scope?").style(theme::accent()),
        Line::from(""),
        Line::from(title).style(theme::text()),
        Line::from(""),
        Line::from("This researches public sources. The report list shows the task as pending.")
            .style(theme::dim()),
        Line::from(""),
        Line::from("Choose case scope").style(yes),
        Line::from("Just answer").style(no),
        Line::from(""),
        Line::from("Enter confirms · Esc cancels · j/k move").style(theme::dim()),
    ];
    frame.render_widget(
        Paragraph::new(text)
            .block(panel(" New case "))
            .wrap(Wrap { trim: false }),
        modal,
    );
}

fn draw_help(frame: &mut Frame, area: Rect) {
    let modal = centered(
        area,
        area.width.saturating_mul(3) / 4,
        area.height.saturating_mul(3) / 4,
    );
    frame.render_widget(Clear, modal);
    let text = vec![
        Line::from("Argos OSINT").style(theme::accent()),
        Line::from("Enter asks saved reviewed evidence and reports. + chooses investigation scope and first actions. /case lists saved cases; /case <id> opens Workbench without a report."),
        Line::from("Ctrl+P app search    Ctrl+M models    Tab cycle focus    Ctrl+C cancel or quit"),
        Line::from("/model lists Grok 4.6 and Grok 4.5. /model grok-4.5 switches. The default is grok-4.6."),
        Line::from("PgUp and PgDn scroll the case desk. End jumps to the latest. The wheel does the same over the chat."),
        Line::from("Modes: 1 Desk · 2 Inbox + Workbench · 3 Graph · 4 Product. Tab cycles inbox, center, inspector, composer. e focuses Plan; Space checks actions; e queues checked actions as separate jobs. g selects holes; p overlays Path; f/t pin endpoints. Product: Space selects accepted IDs; c lead/case; Enter prefills /draft. Desk: w Next Work, q Queue, g Open Questions; \\ toggles transcript. 5 Review, 6 Jobs, 7 Path remain aliases. o source; Esc returns."),
        Line::from("D opens case data controls. /clear-case keeps an empty case; /delete-case removes it. Review the plan, then /confirm-case <exact ID>. /cancel-case-data cancels. Saved reports remain accessible."),
        Line::from("System keeps the log, hardware, and settings. Configuration and task errors stay in that log."),
        Line::from("Providers separates Grok, OpenAI ChatGPT, and OpenRouter accounts. Models assigns Writer and Tools independently."),
        Line::from("Esc or any key closes this card."),
    ];
    frame.render_widget(
        Paragraph::new(text)
            .block(panel(" Help "))
            .wrap(Wrap { trim: false }),
        modal,
    );
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.max(20).min(area.width.saturating_sub(2));
    let height = height.max(8).min(area.height.saturating_sub(2));
    Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    }
}

/// Center a `w`×`h` rect inside `outer` (clamped to outer). No modal min-size.
fn center_rect(outer: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(outer.width);
    let h = h.min(outer.height);
    Rect {
        x: outer.x + (outer.width.saturating_sub(w)) / 2,
        y: outer.y + (outer.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    }
}

fn split_h(area: Rect, constraints: &[Constraint]) -> Vec<Rect> {
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(area)
        .to_vec()
}

fn split_v(area: Rect, constraints: &[Constraint]) -> Vec<Rect> {
    Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area)
        .to_vec()
}

#[cfg(test)]
mod tests {
    use super::{center_rect, markdown_lines, tna_box_connections, tna_ego_box_fit};
    use crate::tui::app::TNA_GRAPH_BOX_BUDGET;
    use crate::tui::theme;
    use ratatui::layout::Rect;
    use ratatui::style::Style;
    use tui_nodes::{NodeGraph, NodeLayout};

    #[test]
    fn chat_markdown_hides_markers() {
        let lines = markdown_lines(
            "1. **who is elon musk?**\n\n`/tmp/report.md`",
            80,
            theme::text(),
        );
        let text: String = lines
            .iter()
            .flat_map(|line| line.spans.iter().map(|span| span.content.to_string()))
            .collect();
        assert!(text.contains("who is elon musk?"));
        assert!(text.contains("/tmp/report.md"));
        assert!(!text.contains("**"));
        assert!(!text.contains('`'));
    }

    #[test]
    fn tna_many_edges_ports_zero_no_panic() {
        // Many duplicate directed edges between 4 boxes → one unordered Connection each, ports 0.
        let mut pairs = Vec::new();
        for _ in 0..6 {
            for i in 0..4usize {
                for j in 0..4usize {
                    if i != j {
                        pairs.push((i, j, Style::default()));
                    }
                }
            }
        }
        assert!(pairs.len() > 4 * 3 / 2);
        let conns = tna_box_connections(pairs);
        assert_eq!(conns.len(), 4 * 3 / 2);
        for c in &conns {
            assert_eq!(c.from_port, 0);
            assert_eq!(c.to_port, 0);
        }
        let nodes: Vec<_> = (0..4).map(|_| NodeLayout::new((16, 4))).collect();
        let mut graph = NodeGraph::new(nodes, conns, 100, 40);
        graph.calculate();
    }

    #[test]
    fn tna_ego_small_area_clamp_dense_edges_no_panic() {
        // Detail ego Rect can be tiny (e.g. 36×12). Unclamped ≤16 boxes of (16,4)
        // stack past height → ConnectionsLayout::block_port OOB. After clamp: safe.
        let w = 36u16;
        let h = 12u16;
        let fit = tna_ego_box_fit(w, h);
        assert!(fit >= 1);
        assert!(fit <= TNA_GRAPH_BOX_BUDGET);
        assert!(fit <= ((h as usize).saturating_sub(2) / 4).max(1));

        let n = fit;
        let nodes: Vec<_> = (0..n).map(|_| NodeLayout::new((16, 4))).collect();
        let mut pairs = Vec::new();
        // Dense star + clique among visible boxes (mirrors Table detail ego).
        for i in 0..n {
            for j in 0..n {
                if i != j {
                    pairs.push((i, j, Style::default()));
                }
            }
        }
        let conns = tna_box_connections(pairs);
        let mut graph = NodeGraph::new(nodes, conns, w as usize, h as usize);
        graph.calculate(); // must not panic
    }

    #[test]
    fn center_rect_centers_in_outer() {
        let outer = Rect::new(10, 20, 100, 40);
        let r = center_rect(outer, 40, 10);
        assert_eq!(r, Rect::new(40, 35, 40, 10));
        // Larger than outer → clamp to outer (no offset).
        assert_eq!(center_rect(outer, 200, 200), outer);
        // Exact fit → same origin.
        assert_eq!(center_rect(outer, 100, 40), outer);
        // Odd slack: floor division.
        let odd = center_rect(Rect::new(0, 0, 11, 9), 4, 2);
        assert_eq!(odd, Rect::new(3, 3, 4, 2));
    }
}

fn tail(mut lines: Vec<Line<'static>>, height: usize) -> Vec<Line<'static>> {
    if height == 0 {
        return Vec::new();
    }
    if lines.len() > height {
        lines.drain(0..lines.len() - height);
    }
    lines
}

fn draw_network(frame: &mut Frame, app: &mut App, area: Rect) {
    network::draw_workspace(frame, app, area);
}

fn draw_tna_ego_boxes(frame: &mut Frame, app: &App, area: Rect) -> bool {
    if area.height < 8 || area.width < 24 {
        let fallback = "Enlarge the terminal to show the graph.";
        frame.render_widget(
            Paragraph::new(fallback)
                .style(theme::dim())
                .wrap(Wrap { trim: false }),
            area,
        );
        return false;
    }
    let Some(snap) = app.tna_snapshot() else {
        frame.render_widget(Paragraph::new("No graph yet.").style(theme::dim()), area);
        return false;
    };
    let fit = tna_ego_box_fit(area.width, area.height);
    let items = app.tna_detail_box_items(fit);
    if items.is_empty() {
        frame.render_widget(Paragraph::new("No ego nodes.").style(theme::dim()), area);
        return false;
    }
    debug_assert!(
        items.len() <= TNA_GRAPH_BOX_BUDGET,
        "graph box budget exceeded: {}",
        items.len()
    );
    debug_assert!(
        items.len() <= fit,
        "area-fit budget exceeded: {} > {}",
        items.len(),
        fit
    );

    let mut titles: Vec<String> = Vec::with_capacity(items.len());
    let mut border_styles: Vec<Style> = Vec::with_capacity(items.len());
    let mut id_to_box: std::collections::HashMap<String, usize> = std::collections::HashMap::new();

    let focus = app.tna_focus_id.as_deref();
    for (box_i, item) in items.iter().enumerate() {
        let (selected, is_focus) = match item {
            TnaDisplayItem::Real { idx } => {
                let id = snap.nodes[*idx].id.as_str();
                let focused = focus == Some(id);
                let selected = focused || app.tna_ledger_neighbor() == Some(id);
                (selected, focused)
            }
        };
        match item {
            TnaDisplayItem::Real { idx } => {
                let node = &snap.nodes[*idx];
                let mut color = cluster_color(node.cluster);
                if selected || is_focus {
                    color = theme::TEXT;
                }
                let mark = if is_focus { "*" } else { "" };
                titles.push(format!("{mark}{}", node.kind.as_str()));
                let mut style = Style::default().fg(color);
                if is_focus {
                    style = style.add_modifier(Modifier::REVERSED | Modifier::BOLD);
                } else if selected {
                    style = style.add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
                }
                border_styles.push(style);
                id_to_box.insert(node.id.clone(), box_i);
            }
        }
    }

    let nodes: Vec<NodeLayout<'_>> = titles
        .iter()
        .zip(border_styles.iter())
        .map(|(title, style)| {
            NodeLayout::new((16, 4))
                .with_title(title.as_str())
                .with_border_type(BorderType::Rounded)
                .with_border_style(*style)
        })
        .collect();

    let mut pairs: Vec<(usize, usize, Style)> = Vec::new();
    for edge in &snap.edges {
        let Some(&a) = id_to_box.get(&edge.from) else {
            continue;
        };
        let Some(&b) = id_to_box.get(&edge.to) else {
            continue;
        };
        let color = snap
            .nodes
            .iter()
            .find(|n| n.id == edge.from)
            .map(|n| cluster_color(n.cluster))
            .unwrap_or(theme::DIM);
        pairs.push((a, b, Style::default().fg(color)));
    }
    let connections = tna_box_connections(pairs);

    // tui-nodes mirrors placements (`pos.x = area.width - pos.right()`), so a
    // full-bleed rect packs boxes to the right. Center a content-sized sub-rect
    // so Graph and Table-detail ego look balanced in the reserved band.
    let (cw, ch) = tna_ego_content_size(items.len(), area);
    let graph_area = center_rect(area, cw, ch);

    let mut graph = NodeGraph::new(
        nodes,
        connections,
        graph_area.width as usize,
        graph_area.height as usize,
    );
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        graph.calculate();
    }))
    .is_err()
    {
        frame.render_widget(
            Paragraph::new("Graph layout unavailable at this terminal size.").style(theme::dim()),
            area,
        );
        return false;
    }
    let zones = graph.split(graph_area);
    for (idx, zone) in zones.into_iter().enumerate() {
        if zone.width == 0 || zone.height == 0 {
            continue;
        }
        let (body, is_focus, zone_selected) = match &items[idx] {
            TnaDisplayItem::Real { idx: ni } => {
                let n = &snap.nodes[*ni];
                let focused = focus.map(|id| n.id == id).unwrap_or(false);
                let selected = focused || app.tna_ledger_neighbor() == Some(n.id.as_str());
                (n.label.clone(), focused, selected)
            }
        };
        let style = if is_focus {
            Style::default()
                .fg(theme::TEXT)
                .add_modifier(Modifier::REVERSED | Modifier::BOLD)
        } else if zone_selected {
            Style::default()
                .fg(theme::TEXT)
                .add_modifier(Modifier::BOLD)
        } else {
            theme::dim()
        };
        frame.render_widget(
            Paragraph::new(body).style(style).wrap(Wrap { trim: false }),
            zone,
        );
    }
    frame.render_stateful_widget(graph, graph_area, &mut ());
    true
}

/// Conservative visual size for `n` ego boxes (16×4, MARGIN 5) inside `area`.
/// Caps horizontal span at ~3 columns (ego layouts rarely fan wider) so the
/// NodeGraph rect can be centered instead of full-bleed / right-hugging.
fn tna_ego_content_size(n: usize, area: Rect) -> (u16, u16) {
    const BOX_W: u16 = 16;
    const BOX_H: u16 = 4;
    const MARGIN: u16 = 5;
    const SLACK: u16 = 4;
    let extra_cols = (n.saturating_sub(1)).min(2) as u16;
    let w = BOX_W
        .saturating_add(extra_cols.saturating_mul(BOX_W.saturating_add(MARGIN)))
        .saturating_add(SLACK);
    let h = (n as u16)
        .saturating_mul(BOX_H)
        .saturating_add(SLACK)
        .max(BOX_H.saturating_add(SLACK));
    (w.min(area.width).max(1), h.min(area.height).max(1))
}

/// Area-fit budget for (16×4) ego boxes inside a NodeGraph of `width`×`height`.
/// Conservative: product of row/col estimates, capped so a worst-case vertical
/// stack (star ego) never drives `ConnectionsLayout::block_port` past the field
/// (`port y = top+port+1`, South indexes `y+1`).
fn tna_ego_box_fit(width: u16, height: u16) -> usize {
    let by_h = (height as usize).saturating_sub(2) / 4;
    let by_w = (width as usize).saturating_sub(2) / (16 + 5);
    let grid = by_h.max(1).saturating_mul(by_w.max(1));
    // Cap to vertical rows — tui-nodes may stack every box in one column.
    let stack_safe = by_h.max(1);
    let horizontal_safe = (width as usize + 5) / (16 + 5);
    TNA_GRAPH_BOX_BUDGET.min(grid.min(stack_safe).min(horizontal_safe.max(1)))
}

/// Truncate display items to `fit`, keeping the focus node when present.
/// One wire per unordered visible box pair; both ports always 0.
/// Compact (16×4) boxes only expose port 0 safely for tui-nodes.
fn tna_box_connections(pairs: impl IntoIterator<Item = (usize, usize, Style)>) -> Vec<Connection> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for (a, b, style) in pairs {
        if a == b {
            continue;
        }
        let (from, to) = if a < b { (a, b) } else { (b, a) };
        if !seen.insert((from, to)) {
            continue;
        }
        out.push(Connection::new(from, 0, to, 0).with_line_style(style));
    }
    out
}

fn cluster_abbr(cluster: TnaCluster) -> &'static str {
    match cluster {
        TnaCluster::Infrastructure => "Infra",
        TnaCluster::Campaign => "Theme",
        TnaCluster::Identity => "Ident",
        TnaCluster::FiledReports => "Filed",
    }
}

fn draw_tna_table_list(frame: &mut Frame, app: &mut App, area: Rect) {
    let list_focused = app.focus == Focus::Graph;
    let title = if list_focused {
        " Entity list · focused "
    } else {
        " Entity list "
    };
    let block = panel(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let visible = app.tna_visible_nodes();
    let show_cluster = inner.width >= 60;
    let constraints = if show_cluster {
        vec![
            Constraint::Length(8),
            Constraint::Min(1),
            Constraint::Length(3),
            Constraint::Length(6),
        ]
    } else {
        vec![
            Constraint::Length(8),
            Constraint::Min(1),
            Constraint::Length(3),
        ]
    };

    let header_cells = if show_cluster {
        vec!["Type", "Label", "Deg", "Group"]
    } else {
        vec!["Type", "Label", "Deg"]
    };
    let header = Row::new(header_cells.into_iter().map(Cell::from))
        .style(theme::accent().add_modifier(Modifier::BOLD))
        .height(1);

    // Copy row data so we do not hold a snapshot borrow across stateful render.
    let row_data: Vec<(String, String, String, String, TnaCluster)> = {
        let Some(snap) = app.tna_snapshot() else {
            return;
        };
        visible
            .iter()
            .filter_map(|&idx| {
                let n = snap.nodes.get(idx)?;
                Some((
                    format!("{} {}", tna_glyph_for_kind(n.kind), n.kind.as_str()),
                    fit_label(
                        &n.label,
                        inner
                            .width
                            .saturating_sub(if show_cluster { 22 } else { 15 })
                            as usize,
                    ),
                    n.degree.to_string(),
                    cluster_abbr(n.cluster).to_string(),
                    n.cluster,
                ))
            })
            .collect()
    };

    let rows: Vec<Row> = if row_data.is_empty() {
        let cols = if show_cluster { 4 } else { 3 };
        let mut cells = vec![Cell::from("No nodes match")];
        while cells.len() < cols {
            cells.push(Cell::from(""));
        }
        vec![Row::new(cells).style(theme::dim()).height(1)]
    } else {
        row_data
            .into_iter()
            .map(|(ty, label, deg, clust, cluster)| {
                let mut cells = vec![Cell::from(ty), Cell::from(label), Cell::from(deg)];
                if show_cluster {
                    cells.push(Cell::from(clust));
                }
                Row::new(cells)
                    .style(Style::default().fg(cluster_color(cluster)))
                    .height(1)
            })
            .collect()
    };

    let selected_style = Style::default()
        .add_modifier(Modifier::REVERSED)
        .fg(theme::ACCENT);

    app.sync_tna_table_ui();

    let table = Table::new(rows, constraints)
        .header(header)
        .row_highlight_style(selected_style)
        .highlight_symbol("▶ ")
        .highlight_spacing(HighlightSpacing::Always);

    frame.render_stateful_widget(table, inner, &mut app.tna_table_state);

    // Scrollbar synced to selection (ITEM_HEIGHT = 1).
    if visible.len() > inner.height.saturating_sub(1) as usize {
        frame.render_stateful_widget(
            Scrollbar::default()
                .orientation(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None),
            area.inner(Margin {
                vertical: 1,
                horizontal: 0,
            }),
            &mut app.tna_table_scroll,
        );
    }
}

fn draw_network_footer(frame: &mut Frame, app: &App, area: Rect) {
    let snap = app.tna_snapshot();
    let (n, e) = snap
        .map(|s| (s.nodes.len(), s.edges.len()))
        .unwrap_or((0, 0));
    let scope = if app.chat_report.is_some() {
        "targeted"
    } else {
        "collection"
    };
    let view = app.tna_layout.title();
    let esc = if app.tna_find.is_some() {
        "Esc clear find"
    } else if app.tna_answer.is_some() {
        "Esc dismiss answer"
    } else {
        "Esc close report"
    };
    let controls = match app.tna_layout {
        super::app::TnaLayout::Cockpit => "j/k select  [/] links",
        super::app::TnaLayout::Clusters => "j/k select  Enter Cockpit",
        super::app::TnaLayout::Path => "f/t pins  Tab paths",
        super::app::TnaLayout::Matrix => "hjkl cursor  Enter Cockpit",
        super::app::TnaLayout::Ribbon => "h/l scrub  d rejected",
    };
    let state = if app.tna_find_editing {
        "Find"
    } else if app.focus == Focus::Prompt {
        "Ask"
    } else {
        "Tab Ask"
    };
    let left = format!("{controls}  ←/→ layout  / find  {state}  {esc}");
    let mid = format!("{view} · {scope} {n}n/{e}e");
    let model = app.active_model();
    let text = if area.width < 110 {
        format!("{view} {n}n/{e}e · {state} · / Find · {esc}")
    } else {
        format!("{mid} · {left}")
    };
    frame.render_widget(
        Paragraph::new(fit_status(area.width as usize, &text, &model)).style(theme::accent()),
        area,
    );
}

fn cluster_color(cluster: TnaCluster) -> ratatui::style::Color {
    match cluster {
        TnaCluster::Infrastructure => theme::ACCENT,
        TnaCluster::Campaign => theme::WARN,
        TnaCluster::Identity => theme::GREEN,
        TnaCluster::FiledReports => theme::DIM,
    }
}

fn fit_label(label: &str, width: usize) -> String {
    if label.width() <= width {
        return label.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for ch in label.chars() {
        let w = ch.width().unwrap_or(0);
        if used + w > width.saturating_sub(1) {
            break;
        }
        out.push(ch);
        used += w;
    }
    out.push('…');
    out
}

fn short_label(label: &str, max: usize) -> String {
    let mut out = String::new();
    for ch in label.chars() {
        if out.chars().count() >= max {
            break;
        }
        out.push(ch);
    }
    out
}
