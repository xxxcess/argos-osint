//! Apps column on the left, the open app on the right, prompt at the bottom.
//! The prompt talks to the case desk or the selected case.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Gauge, List, ListItem, ListState, Paragraph, Sparkline, Tabs, Wrap};
use ratatui::Frame;

use super::app::{App, CasePage, Focus, ModuleId, ProviderPage};
use super::theme::{self, panel};
use argos_osint_core::paths::fit_status;
use argos_osint_core::secrets::mask;

pub fn draw(frame: &mut Frame, app: &mut App) {
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
    draw_prompt(frame, app, chunks[1]);
    draw_footer(frame, app, chunks[2]);
    if app.modal {
        draw_modal(frame, app, area);
    }
    if app.model_picker {
        draw_model_picker(frame, app, area);
    }
    if app.help {
        draw_help(frame, area);
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
    if on_case_desk(app) {
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
    let cols = split_h(
        area,
        &[Constraint::Percentage(26), Constraint::Percentage(74)],
    );
    draw_launcher(frame, app, cols[0]);
    draw_main(frame, app, cols[1]);
}

fn draw_desk_and_reports(frame: &mut Frame, app: &mut App, area: Rect) {
    if app.case_page == CasePage::Brain {
        draw_brain(frame, app, area);
        return;
    }
    let cols = split_h(
        area,
        &[Constraint::Percentage(64), Constraint::Percentage(36)],
    );
    app.canvas_area = cols[0];
    draw_canvas(frame, app, cols[0]);
    draw_report_list(frame, app, cols[1]);
}

fn draw_brain(frame: &mut Frame, app: &mut App, area: Rect) {
    app.report_area = Rect::default();
    app.report_line_index.clear();
    app.canvas_area = Rect::default();
    draw_brain_list(frame, app, area);
}

fn draw_brain_list(frame: &mut Frame, app: &App, area: Rect) {
    let title = " Memories ";
    let lines = brain_list_lines(app, area.height.saturating_sub(2) as usize);
    frame.render_widget(
        Paragraph::new(lines)
            .block(panel(&title))
            .wrap(Wrap { trim: false }),
        area,
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
        " Reports · focused "
    } else {
        " Reports "
    };
    frame.render_widget(Paragraph::new(lines).block(panel(title)), area);
}

fn report_lines(app: &App, width: usize) -> (Vec<Line<'static>>, Vec<Option<usize>>) {
    let rows = app.report_rows();
    if rows.is_empty() {
        return (
            vec![
                Line::from("No reports yet.").style(theme::dim()),
                Line::from("A case query stays pending here until the markdown is filed.")
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
        if let Some(when) = row.when() {
            lines.push(Line::from(when).style(theme::dim()));
            index.push(Some(i));
        } else if let Some(detail) = row.detail() {
            lines.push(
                Line::from(clip_chars(detail, width))
                    .style(Style::default().fg(theme::RED).bg(theme::BG)),
            );
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
    let lines = if app.chat_report.is_some() {
        let mut lines = vec![
            Line::from(
                "This report chat replaces the case desk. Answers use only this file. Earlier questions in this report stay here. Esc returns to the desk. /clear wipes this chat.",
            )
            .style(theme::dim()),
            Line::from(""),
        ];
        if transcript.is_empty() {
            lines.push(
                Line::from("Ask a question about this report.".to_string()).style(theme::dim()),
            );
        } else {
            lines.extend(transcript);
        }
        lines
    } else if transcript.is_empty() {
        vec![
            Line::from(
                "Ask about the reports, or type a query and press + to start a case. /clear wipes this chat.",
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
    let titles: Vec<Line> = CasePage::all()
        .into_iter()
        .map(|page| Line::from(format!(" {} ", page.title())))
        .collect();
    app.case_tab_hits = tab_hits(area, &titles);
    let selected = CasePage::all()
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
    } else {
        app.provider_tab_area = Rect::default();
        app.provider_tab_hits.clear();
    }
    if widget == ModuleId::Hardware && area.width >= 40 && area.height >= 8 {
        draw_gauges(frame, app, area, 2);
        return;
    }
    let title = match app.module {
        Some(ModuleId::Cases) => format!(" {} ", app.case_page.title()),
        Some(ModuleId::Providers) => format!(" {} ", app.provider_page.title()),
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
        ModuleId::Log => {
            if app.log.is_empty() {
                vec![Line::from("Tool and search notes land here.").style(theme::dim())]
            } else {
                app.log
                    .iter()
                    .rev()
                    .take(height.max(1))
                    .rev()
                    .cloned()
                    .map(|line| Line::from(line).style(theme::dim()))
                    .collect()
            }
        }
        ModuleId::Providers | ModuleId::Gmail | ModuleId::Settings => field_lines(app),
        ModuleId::Cases => Vec::new(),
    };
    tail(lines, height)
}

fn field_lines(app: &App) -> Vec<Line<'static>> {
    app.fields
        .iter()
        .enumerate()
        .map(|(i, field)| {
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
    };
    lines.extend(body);
    tail(lines, height)
}

fn provider_lines(app: &App, height: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let body = match app.provider_page {
        ProviderPage::Mail => field_lines(app),
        ProviderPage::Osint => osint_lines(app, height.saturating_sub(3)),
        ProviderPage::Llm => field_lines(app),
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
            let wrapped = wrap_text(body, content_width);
            let prefix_style = theme::user_message()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::BOLD);
            wrapped
                .into_iter()
                .enumerate()
                .map(|(index, text)| {
                    let prefix = if index == 0 { "❯ " } else { "  " };
                    let pad = content_width.saturating_sub(text.chars().count());
                    let shown = format!("{text}{}", " ".repeat(pad));
                    Line::from(vec![
                        Span::styled(prefix, prefix_style),
                        Span::styled(shown, theme::user_message()),
                    ])
                })
                .collect()
        }
        "assistant" => wrap_text(body, width)
            .into_iter()
            .map(|text| Line::from(text).style(theme::text()))
            .collect(),
        _ => wrap_text(body, width)
            .into_iter()
            .map(|text| Line::from(text).style(theme::dim()))
            .collect(),
    }
}

fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = Vec::new();
    for raw in text.split('\n') {
        if raw.is_empty() {
            lines.push(String::new());
            continue;
        }
        let mut rest = raw.to_string();
        while rest.chars().count() > width {
            let mut break_at = width;
            let space = rest
                .char_indices()
                .take(width)
                .filter(|(_, ch)| *ch == ' ')
                .map(|(index, _)| index)
                .last();
            if let Some(space) = space {
                if space > 0 {
                    break_at = rest[..space].chars().count();
                }
            }
            let (head, tail) = split_chars(&rest, break_at);
            lines.push(head);
            rest = tail.trim_start().to_string();
        }
        lines.push(rest);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn split_chars(text: &str, count: usize) -> (String, String) {
    let mut end = text.len();
    for (index, (byte, _)) in text.char_indices().enumerate() {
        if index == count {
            end = byte;
            break;
        }
    }
    (text[..end].to_string(), text[end..].to_string())
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
    let hint = if app.prompt.starts_with('/') && !app.prompt.contains(' ') {
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
        vec![Line::from("Enter send · + new case · /help").style(hint_style)]
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
    let left = "[Ctrl+P] App Search";
    let right = match app.focus {
        Focus::Prompt => "[Tab] Jump Focus | [Esc] Exit App",
        Focus::Canvas => "[Up/Down] Scroll chat  [Wheel] Scroll  [Tab] Jump",
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
    let current = app.active_model();
    let mut lines = vec![
        Line::from("Models").style(theme::accent()),
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
    for (i, (id, label)) in choices.iter().enumerate() {
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
    lines.push(Line::from("[Enter] Use model   [Esc] Close   Ctrl+M").style(theme::dim()));
    frame.render_widget(
        Paragraph::new(lines)
            .block(panel(" Model "))
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
            "Opening replaces the case desk chat. Deleting removes the markdown file and the facts taken from this report.",
        )
        .style(theme::dim()),
        Line::from(""),
        Line::from("Open report chat").style(style_for(0)),
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
        Line::from("Start a case worker?").style(theme::accent()),
        Line::from(""),
        Line::from(title).style(theme::text()),
        Line::from(""),
        Line::from("This researches public sources. The report list shows the task as pending.")
            .style(theme::dim()),
        Line::from(""),
        Line::from("Start case worker").style(yes),
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
        Line::from("Enter talks on the case desk. + starts a case worker. Report status stays in the list beside the desk."),
        Line::from("Ctrl+P app search    Ctrl+M models    Tab cycle focus    Ctrl+C cancel or quit"),
        Line::from("/model lists Grok 4.6 and Grok 4.5. /model grok-4.5 switches. The default is grok-4.6."),
        Line::from("PgUp and PgDn scroll the case desk or report chat. End jumps to the latest. The wheel does the same over the chat."),
        Line::from("/clear wipes the case desk chat, or the open report chat. /search /new /use /quit"),
        Line::from("Public search only. Gmail is imap.gmail.com, read-only, app password."),
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

fn tail(mut lines: Vec<Line<'static>>, height: usize) -> Vec<Line<'static>> {
    if height == 0 {
        return Vec::new();
    }
    if lines.len() > height {
        lines.drain(0..lines.len() - height);
    }
    lines
}
