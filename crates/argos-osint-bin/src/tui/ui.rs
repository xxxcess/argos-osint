//! Apps column on the left, the open app on the right, prompt at the bottom.
//! The prompt talks to the case desk or the selected case.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{BorderType, Clear, Gauge, List, ListItem, ListState, Paragraph, Sparkline, Tabs, Wrap};
use ratatui::Frame;
use tui_nodes::{Connection, NodeGraph, NodeLayout};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::app::{
    App, CasePage, Focus, ModuleId, ProviderPage, SystemPage, TnaDisplayItem, TnaView,
    TNA_GRAPH_BOX_BUDGET, tna_glyph_for_cluster, tna_glyph_for_kind,
};
use super::theme::{self, panel};
use argos_osint_core::paths::fit_status;
use argos_osint_core::tna::TnaCluster;
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
    if app.free_picker {
        draw_free_picker(frame, app, area);
    }
    if app.help {
        draw_help(frame, area);
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
    if app.case_page == CasePage::Network {
        draw_network(frame, app, area);
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
        CasePage::Network => vec![Line::from("Network graph".to_string())],
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
    if app.case_page == CasePage::Network && matches!(app.module, None | Some(ModuleId::Cases)) {
        draw_network_footer(frame, app, area);
        return;
    }
    let left = "[Ctrl+P] App Search";
    let right = match app.focus {
        Focus::Prompt => "[Tab] Jump Focus | [Esc] Exit App",
        Focus::Canvas | Focus::Graph => "[Up/Down] Scroll chat  [Wheel] Scroll  [Tab] Jump",
        Focus::Reports => "[j/k] Move  [Enter] Open  [Tab] Jump",
        Focus::Launcher | Focus::TnaSide => "[Tab] Jump Focus | [Esc] Exit App",
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
        Line::from(app.picker_title()).style(theme::accent()),
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
    let hint = if router_selected {
        "[Enter] List free models   [Esc] Close"
    } else {
        "[Enter] Use model   [Esc] Close   Ctrl+M"
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
        Line::from("This run only. Defaults come from OSINT settings.").style(theme::dim()),
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
        Line::from("Space toggles · Enter starts · Esc cancels · j/k move").style(theme::dim()),
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
        Line::from("Enter talks on the case desk. + opens the source scope, then starts a case worker. Report status stays in the list beside the desk."),
        Line::from("Ctrl+P app search    Ctrl+M models    Tab cycle focus    Ctrl+C cancel or quit"),
        Line::from("/model lists Grok 4.6 and Grok 4.5. /model grok-4.5 switches. The default is grok-4.6."),
        Line::from("PgUp and PgDn scroll the case desk or report chat. End jumps to the latest. The wheel does the same over the chat."),
        Line::from("/clear wipes the case desk chat, or the open report chat. /search /new /use /system /log /quit"),
        Line::from("System keeps the log, hardware, and settings. Configuration and task errors stay in that log."),
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

#[cfg(test)]
mod tests {
    use super::markdown_lines;
    use crate::tui::theme;

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
    app.report_area = Rect::default();
    app.report_line_index.clear();
    let mut work = area;
    if app.chat_report.is_some() && work.height > 2 {
        let rows = split_v(work, &[Constraint::Length(1), Constraint::Min(4)]);
        let title = app
            .open_report()
            .map(|r| r.title.as_str())
            .unwrap_or("report");
        frame.render_widget(
            Paragraph::new(format!(
                "Report chat · {title} · Esc returns to desk"
            ))
            .style(theme::dim()),
            rows[0],
        );
        work = rows[1];
    }
    let cols = split_h(
        work,
        &[Constraint::Percentage(68), Constraint::Percentage(32)],
    );
    app.canvas_area = cols[0];
    draw_tna_canvas(frame, app, cols[0]);
    draw_tna_sidebar(frame, app, cols[1]);
}

fn draw_tna_canvas(frame: &mut Frame, app: &App, area: Rect) {
    match app.tna_view {
        TnaView::Graph => draw_tna_graph(frame, app, area),
        TnaView::Outline => draw_tna_outline(frame, app, area),
        TnaView::Table => draw_tna_table(frame, app, area),
    }
}

fn draw_tna_graph(frame: &mut Frame, app: &App, area: Rect) {
    let snap = app.tna_snapshot();
    let title = snap.map(|s| s.title.as_str()).unwrap_or("TNA");
    let focused = app.focus == Focus::Graph;
    let border = if focused {
        format!(" {title} · graph · focused ")
    } else {
        format!(" {title} · graph ")
    };
    let find = app
        .tna_find
        .as_ref()
        .map(|q| format!(" /{q}"))
        .unwrap_or_default();
    let status = if app.tna_rebuilding {
        " building…".to_string()
    } else {
        find
    };
    let block = panel(&format!("{border}{status}"));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(snap) = snap else {
        frame.render_widget(
            Paragraph::new("No graph yet. File a report to build the collection.")
                .style(theme::dim()),
            inner,
        );
        return;
    };
    if snap.nodes.is_empty() {
        frame.render_widget(
            Paragraph::new("Empty graph for this scope.").style(theme::dim()),
            inner,
        );
        return;
    }

    let items = app.tna_display_nodes();
    if items.is_empty() {
        frame.render_widget(
            Paragraph::new("No nodes in ego view (try / find or Esc).").style(theme::dim()),
            inner,
        );
        return;
    }
    debug_assert!(
        items.len() <= TNA_GRAPH_BOX_BUDGET,
        "graph box budget exceeded: {}",
        items.len()
    );

    // Owned titles so NodeLayout can borrow &'a str for the frame.
    let mut titles: Vec<String> = Vec::with_capacity(items.len());
    let mut border_styles: Vec<Style> = Vec::with_capacity(items.len());
    let mut id_to_box: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();

    for (box_i, item) in items.iter().enumerate() {
        let selected = box_i == app.tna_sel;
        match item {
            TnaDisplayItem::Real { idx } => {
                let node = &snap.nodes[*idx];
                let mut color = cluster_color(node.cluster);
                if selected {
                    color = theme::TEXT;
                }
                titles.push(short_label(&node.label, 12));
                let mut style = Style::default().fg(color);
                if selected {
                    style = style.add_modifier(Modifier::BOLD);
                }
                border_styles.push(style);
                id_to_box.insert(node.id.clone(), box_i);
            }
            TnaDisplayItem::Super {
                hub_id, count, ..
            } => {
                let hub = snap.nodes.iter().find(|n| n.id == *hub_id);
                let mut color = hub
                    .map(|n| cluster_color(n.cluster))
                    .unwrap_or(theme::DIM);
                if selected {
                    color = theme::TEXT;
                }
                titles.push(format!("▣×{count}"));
                let mut style = Style::default().fg(color);
                if selected {
                    style = style.add_modifier(Modifier::BOLD);
                }
                border_styles.push(style);
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

    // Invented ports: unique per endpoint so wire layout does not clobber.
    let mut from_port_count: Vec<usize> = vec![0; items.len()];
    let mut to_port_count: Vec<usize> = vec![0; items.len()];
    let mut connections: Vec<Connection> = Vec::new();
    for edge in &snap.edges {
        let Some(&a) = id_to_box.get(&edge.from) else {
            continue;
        };
        let Some(&b) = id_to_box.get(&edge.to) else {
            continue;
        };
        if a == b {
            continue;
        }
        // Directed for layout: lower box index → higher.
        let (from, to) = if a < b { (a, b) } else { (b, a) };
        let fp = from_port_count[from];
        let tp = to_port_count[to];
        from_port_count[from] += 1;
        to_port_count[to] += 1;
        let color = snap
            .nodes
            .iter()
            .find(|n| n.id == edge.from)
            .map(|n| cluster_color(n.cluster))
            .unwrap_or(theme::DIM);
        connections.push(
            Connection::new(from, fp, to, tp).with_line_style(Style::default().fg(color)),
        );
    }

    let mut graph = NodeGraph::new(
        nodes,
        connections,
        inner.width as usize,
        inner.height as usize,
    );
    graph.calculate();
    let zones = graph.split(inner);
    for (idx, zone) in zones.into_iter().enumerate() {
        if zone.width == 0 || zone.height == 0 {
            continue;
        }
        let body = match &items[idx] {
            TnaDisplayItem::Real { idx: ni } => {
                let n = &snap.nodes[*ni];
                format!("{} {}", tna_glyph_for_kind(n.kind), n.kind.as_str())
            }
            TnaDisplayItem::Super { count, .. } => format!("hub · {count}"),
        };
        let style = if idx == app.tna_sel {
            Style::default().fg(theme::TEXT).add_modifier(Modifier::BOLD)
        } else {
            theme::dim()
        };
        frame.render_widget(Paragraph::new(body).style(style), zone);
    }
    frame.render_stateful_widget(graph, inner, &mut ());
}

fn draw_tna_outline(frame: &mut Frame, app: &App, area: Rect) {
    let snap = app.tna_snapshot();
    let title = snap.map(|s| s.title.as_str()).unwrap_or("TNA");
    let focused = app.focus == Focus::Graph;
    let border = if focused {
        format!(" {title} · outline · focused ")
    } else {
        format!(" {title} · outline ")
    };
    let find = app
        .tna_find
        .as_ref()
        .map(|q| format!(" /{q}"))
        .unwrap_or_default();
    let block = panel(&format!("{border}{find}"));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(snap) = snap else {
        frame.render_widget(
            Paragraph::new("No graph yet.").style(theme::dim()),
            inner,
        );
        return;
    };

    let visible = app.tna_visible_nodes();
    let selected_idx = visible.get(app.tna_sel).copied();
    let collection = matches!(
        snap.scope,
        argos_osint_core::tna::TnaScope::Collection
    );
    let anchor_ids: std::collections::HashSet<&str> =
        snap.anchors.iter().map(|a| a.node_id.as_str()).collect();

    let mut lines: Vec<Line<'static>> = Vec::new();
    for cluster in TnaCluster::all() {
        if !collection && cluster == TnaCluster::FiledReports {
            continue;
        }
        let cluster_nodes: Vec<usize> = visible
            .iter()
            .copied()
            .filter(|&i| snap.nodes[i].cluster == cluster)
            .collect();
        if cluster_nodes.is_empty() {
            continue;
        }
        lines.push(
            Line::from(format!(
                "{} {}",
                tna_glyph_for_cluster(cluster),
                cluster.as_str()
            ))
            .style(Style::default().fg(cluster_color(cluster)).add_modifier(Modifier::BOLD)),
        );
        let anchors: Vec<usize> = cluster_nodes
            .iter()
            .copied()
            .filter(|&i| anchor_ids.contains(snap.nodes[i].id.as_str()))
            .collect();
        if !anchors.is_empty() {
            lines.push(Line::from("  Anchors").style(theme::dim()));
            for i in &anchors {
                let n = &snap.nodes[*i];
                let sel = selected_idx == Some(*i);
                let prefix = if sel { "▶" } else { " " };
                let style = if sel {
                    Style::default().fg(theme::TEXT).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(cluster_color(n.cluster))
                };
                lines.push(
                    Line::from(format!(
                        "  {prefix} {} {}",
                        tna_glyph_for_kind(n.kind),
                        n.label
                    ))
                    .style(style),
                );
            }
        }
        lines.push(Line::from("  Nodes").style(theme::dim()));
        for i in &cluster_nodes {
            if anchor_ids.contains(snap.nodes[*i].id.as_str()) {
                continue;
            }
            let n = &snap.nodes[*i];
            let sel = selected_idx == Some(*i);
            let prefix = if sel { "▶" } else { " " };
            let style = if sel {
                Style::default().fg(theme::TEXT).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(cluster_color(n.cluster))
            };
            lines.push(
                Line::from(format!(
                    "  {prefix} {} {}",
                    tna_glyph_for_kind(n.kind),
                    n.label
                ))
                .style(style),
            );
        }
        lines.push(Line::from(""));
    }
    if lines.is_empty() {
        lines.push(Line::from("No nodes match.").style(theme::dim()));
    }
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }),
        inner,
    );
}

fn draw_tna_table(frame: &mut Frame, app: &App, area: Rect) {
    let snap = app.tna_snapshot();
    let title = snap.map(|s| s.title.as_str()).unwrap_or("TNA");
    let focused = app.focus == Focus::Graph;
    let border = if focused {
        format!(" {title} · table · focused ")
    } else {
        format!(" {title} · table ")
    };
    let find = app
        .tna_find
        .as_ref()
        .map(|q| format!(" /{q}"))
        .unwrap_or_default();
    let block = panel(&format!("{border}{find}"));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(snap) = snap else {
        frame.render_widget(
            Paragraph::new("No graph yet.").style(theme::dim()),
            inner,
        );
        return;
    };

    let visible = app.tna_visible_nodes();
    let mut lines: Vec<Line<'static>> = Vec::new();
    lines.push(
        Line::from(format!(
            "  {:<6} {:<22} {:>6} {}",
            "Type", "Label", "Degree", "Cluster"
        ))
        .style(theme::accent()),
    );
    for (row, &idx) in visible.iter().enumerate() {
        let n = &snap.nodes[idx];
        let sel = row == app.tna_sel;
        let prefix = if sel { "▶" } else { " " };
        let style = if sel {
            Style::default().fg(theme::TEXT).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(cluster_color(n.cluster))
        };
        lines.push(
            Line::from(format!(
                "{prefix} {} {:<4} {:<22} {:>6} {}",
                tna_glyph_for_kind(n.kind),
                n.kind.as_str(),
                short_label(&n.label, 22),
                n.degree,
                n.cluster.as_str()
            ))
            .style(style),
        );
    }
    if visible.is_empty() {
        lines.push(Line::from("  (empty)").style(theme::dim()));
    }
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }),
        inner,
    );
}

fn draw_tna_sidebar(frame: &mut Frame, app: &App, area: Rect) {
    let focused = app.focus == Focus::TnaSide;
    let title = if focused {
        " Network · side "
    } else {
        " Network "
    };
    let lines = tna_sidebar_lines(app, area.height.saturating_sub(2) as usize);
    frame.render_widget(
        Paragraph::new(lines)
            .block(panel(title))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn tna_sidebar_lines(app: &App, height: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let Some(snap) = app.tna_snapshot() else {
        lines.push(Line::from("No snapshot.".to_string()).style(theme::dim()));
        return tail(lines, height);
    };
    let collection = matches!(
        snap.scope,
        argos_osint_core::tna::TnaScope::Collection
    );

    // Selected line at top (FR-4).
    match app.tna_selected_item() {
        Some(TnaDisplayItem::Real { idx }) => {
            if let Some(n) = snap.nodes.get(idx) {
                lines.push(
                    Line::from(format!(
                        "Selected: {} {} ({})",
                        tna_glyph_for_kind(n.kind),
                        n.label,
                        n.kind.as_str()
                    ))
                    .style(Style::default().fg(theme::TEXT).add_modifier(Modifier::BOLD)),
                );
            }
        }
        Some(TnaDisplayItem::Super {
            hub_id, count, ..
        }) => {
            let hub_label = snap
                .nodes
                .iter()
                .find(|n| n.id == hub_id)
                .map(|n| n.label.as_str())
                .unwrap_or(hub_id.as_str());
            lines.push(
                Line::from(format!("Selected: ▣×{count} (hub {hub_label})"))
                    .style(Style::default().fg(theme::TEXT).add_modifier(Modifier::BOLD)),
            );
        }
        None => {
            lines.push(Line::from("Selected: —").style(theme::dim()));
        }
    }
    lines.push(Line::from(""));

    lines.push(Line::from("Topical Clusters").style(theme::accent()));
    for summary in &snap.clusters {
        if !collection && summary.cluster == TnaCluster::FiledReports {
            continue;
        }
        let style = Style::default().fg(cluster_color(summary.cluster)).bg(theme::BG);
        lines.push(
            Line::from(format!(
                "  {} {:<14} {:>3}",
                tna_glyph_for_cluster(summary.cluster),
                summary.cluster.as_str(),
                summary.node_count
            ))
            .style(style),
        );
    }
    lines.push(Line::from(""));
    lines.push(Line::from("Strategic Anchors").style(theme::accent()));
    let max_score = snap
        .anchors
        .iter()
        .map(|a| a.degree.saturating_add(a.mentions))
        .max()
        .unwrap_or(1)
        .max(1) as f64;
    for (i, anchor) in snap.anchors.iter().enumerate().take(6) {
        let node = snap.nodes.iter().find(|n| n.id == anchor.node_id);
        let label = node.map(|n| n.label.as_str()).unwrap_or(anchor.node_id.as_str());
        let glyph = node
            .map(|n| tna_glyph_for_kind(n.kind))
            .unwrap_or("·");
        let score = (anchor.degree as f64 + anchor.mentions as f64) / max_score;
        let color = node
            .map(|n| cluster_color(n.cluster))
            .unwrap_or(theme::TEXT);
        lines.push(
            Line::from(format!(
                "  {}. {} {:<16} {:>4.2}",
                i + 1,
                glyph,
                short_label(label, 16),
                score
            ))
            .style(Style::default().fg(color).bg(theme::BG)),
        );
    }
    lines.push(Line::from(""));
    lines.push(Line::from("Structural Gaps").style(theme::accent()));
    if snap.gaps.is_empty() {
        lines.push(Line::from("  none".to_string()).style(theme::dim()));
    } else {
        for gap in snap.gaps.iter().take(5) {
            lines.push(
                Line::from(format!(
                    "  ! {} ↔ {}",
                    gap.cluster_a.as_str(),
                    gap.cluster_b.as_str()
                ))
                .style(Style::default().fg(theme::RED).bg(theme::BG)),
            );
            lines.push(
                Line::from("    proximity, no edge".to_string()).style(theme::dim()),
            );
        }
    }
    if app.tna_side_scroll > 0 && lines.len() > height {
        let skip = app.tna_side_scroll.min(lines.len().saturating_sub(height));
        lines = lines.split_off(skip);
    }
    tail(lines, height)
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
    let view = app.tna_view.as_str();
    let esc = if app.tna_find.is_some() {
        "Esc clear find"
    } else if app.chat_report.is_some() {
        "Esc close report"
    } else {
        "Esc desk"
    };
    let left = format!("hjkl  / find  v views  {esc}");
    let mid = format!("{view} · {scope} {n}n/{e}e");
    let model = app.active_model();
    let gap = area.width as usize;
    let used = left.chars().count() + mid.chars().count() + model.chars().count() + 4;
    let spaces = gap.saturating_sub(used).max(1);
    let pad_left = spaces / 2;
    let pad_right = spaces - pad_left;
    let line = Line::from(vec![
        Span::styled(left, theme::accent()),
        Span::raw(" ".repeat(pad_left)),
        Span::styled(mid, Style::default().fg(theme::WARN).bg(theme::BG)),
        Span::raw(" ".repeat(pad_right)),
        Span::styled(model, theme::dim()),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn cluster_color(cluster: TnaCluster) -> ratatui::style::Color {
    match cluster {
        TnaCluster::Infrastructure => theme::ACCENT,
        TnaCluster::Campaign => theme::WARN,
        TnaCluster::Identity => theme::GREEN,
        TnaCluster::FiledReports => theme::DIM,
    }
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
