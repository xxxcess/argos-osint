//! Layouts follow the reference sheet: launcher, canvas, splits, modal,
//! floating actions, widget grid, and zen. The prompt stays at the bottom
//! and is bound to whatever the canvas is showing.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Gauge, List, ListItem, ListState, Paragraph, Sparkline, Wrap};
use ratatui::Frame;

use super::app::{App, Focus, LayoutMode, ModuleId};
use super::theme::{self, panel};
use argos_osint_core::paths::fit_status;
use argos_osint_core::secrets::mask;

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    frame.render_widget(Paragraph::new("").style(theme::text()), area);
    let prompt_h = if app.layout == LayoutMode::Zen { 6 } else { 4 };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(8),
            Constraint::Length(prompt_h),
            Constraint::Length(1),
        ])
        .split(area);
    draw_body(frame, app, chunks[0]);
    draw_prompt(frame, app, chunks[1]);
    draw_footer(frame, app, chunks[2]);
    if app.modal || app.layout == LayoutMode::Modal {
        draw_modal(frame, app, area);
    }
    if app.help {
        draw_help(frame, area);
    }
}

fn draw_body(frame: &mut Frame, app: &mut App, area: Rect) {
    match app.layout {
        LayoutMode::Classic | LayoutMode::Modal => {
            let cols = split_h(
                area,
                &[Constraint::Percentage(28), Constraint::Percentage(72)],
            );
            draw_launcher(frame, app, cols[0]);
            draw_canvas(frame, app, cols[1]);
        }
        LayoutMode::Dashboard => {
            let cols = split_h(
                area,
                &[Constraint::Percentage(30), Constraint::Percentage(70)],
            );
            draw_launcher(frame, app, cols[0]);
            draw_gauges(frame, app, cols[1], 2);
        }
        LayoutMode::Tabs => {
            let rows = split_v(area, &[Constraint::Length(3), Constraint::Min(6)]);
            draw_tabs(frame, app, rows[0]);
            let cols = split_h(
                rows[1],
                &[Constraint::Percentage(62), Constraint::Percentage(38)],
            );
            draw_canvas(frame, app, cols[0]);
            draw_log(frame, app, cols[1]);
        }
        LayoutMode::Vertical => {
            let cols = split_h(
                area,
                &[Constraint::Percentage(26), Constraint::Percentage(74)],
            );
            draw_launcher(frame, app, cols[0]);
            let rows = split_v(
                cols[1],
                &[Constraint::Percentage(62), Constraint::Percentage(38)],
            );
            draw_canvas(frame, app, rows[0]);
            draw_log(frame, app, rows[1]);
        }
        LayoutMode::Horizontal => {
            let rows = split_v(
                area,
                &[Constraint::Percentage(64), Constraint::Percentage(36)],
            );
            draw_canvas(frame, app, rows[0]);
            draw_log(frame, app, rows[1]);
        }
        LayoutMode::Three => {
            let cols = split_h(
                area,
                &[
                    Constraint::Percentage(22),
                    Constraint::Percentage(54),
                    Constraint::Percentage(24),
                ],
            );
            draw_launcher(frame, app, cols[0]);
            draw_canvas(frame, app, cols[1]);
            draw_context(frame, app, cols[2]);
        }
        LayoutMode::Float => {
            draw_canvas(frame, app, area);
            let panel = Rect {
                x: area.x + area.width.saturating_sub(36),
                y: area.y + 1,
                width: 34.min(area.width),
                height: area.height.saturating_sub(2).min(16),
            };
            draw_quick(frame, app, panel);
        }
        LayoutMode::Grid => draw_gauges(frame, app, area, 3),
        LayoutMode::Zen => draw_zen(frame, app, area),
    }
}

fn draw_launcher(frame: &mut Frame, app: &mut App, area: Rect) {
    app.launcher_area = area;
    let items: Vec<ListItem> = ModuleId::all()
        .iter()
        .enumerate()
        .map(|(i, module)| {
            let mark = if app.module == Some(*module) {
                ">"
            } else {
                " "
            };
            ListItem::new(Line::from(vec![Span::styled(
                format!(" {mark} {}. {} ", i + 1, module.title()),
                theme::text(),
            )]))
        })
        .collect();
    let mut state = ListState::default();
    state.select(Some(app.launcher_sel));
    let title = if app.focus == Focus::Launcher {
        " APPLICATION LAUNCHER "
    } else {
        " APPLICATION LAUNCHER "
    };
    let list = List::new(items)
        .block(panel(title))
        .highlight_style(theme::selected())
        .highlight_symbol("");
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_canvas(frame: &mut Frame, app: &App, area: Rect) {
    let title = format!(" {} ", app.view_name());
    let lines = canvas_lines(app, area.height.saturating_sub(2) as usize);
    let paragraph = Paragraph::new(lines)
        .block(panel(&title))
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

fn canvas_lines(app: &App, height: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    match app.module {
        Some(ModuleId::Hardware) => {
            lines.push(Line::from(app.hardware.one_line()));
            lines.push(Line::from(format!(
                "CPU {cpu:.0}%   RAM {ram}%   Disk {disk}%   backend {backend}",
                cpu = app.cpu_now,
                ram = app.hardware.ram_pct(),
                disk = app.hardware.disk_pct(),
                backend = app.hardware.backend,
            )));
            lines.push(Line::from("r rescan".to_string()).style(theme::dim()));
        }
        Some(ModuleId::Cases) => {
            if app.cases.is_empty() {
                lines.push(Line::from(
                    "No cases yet. /new <title> opens one.".to_string(),
                ));
            } else {
                for (i, case) in app.cases.iter().take(6).enumerate() {
                    let style = if i == app.case_sel {
                        theme::selected()
                    } else {
                        theme::text()
                    };
                    lines.push(Line::from(format!("{}  {}", case.id, case.title)).style(style));
                }
                lines.push(
                    Line::from(
                        "J/K switch case. The prompt talks to the highlighted case.".to_string(),
                    )
                    .style(theme::dim()),
                );
            }
        }
        Some(ModuleId::Brain) => {
            if app.memories.is_empty() {
                lines.push(Line::from(
                    "No memories. /brain <fact> or say \"remember …\".".to_string(),
                ));
            }
            for (i, mem) in app.memories.iter().take(8).enumerate() {
                let style = if i == app.brain_sel {
                    theme::accent()
                } else {
                    theme::text()
                };
                lines.push(Line::from(mem.text.clone()).style(style));
            }
        }
        Some(ModuleId::Reports) => {
            if app.reports.is_empty() {
                lines.push(Line::from(
                    "No reports yet. /search or a case turn writes markdown.".to_string(),
                ));
            }
            for report in app.reports.iter().take(10) {
                lines.push(Line::from(format!("{}  {}", report.title, report.path)));
            }
        }
        Some(ModuleId::Log) => {
            for line in app.log.iter().rev().take(height.max(1)).rev() {
                lines.push(Line::from(line.clone()).style(theme::dim()));
            }
        }
        Some(ModuleId::Providers) | Some(ModuleId::Gmail) | Some(ModuleId::Settings) => {
            lines.extend(field_lines(app));
        }
        None => {
            lines.push(Line::from(app.layout.label().to_string()).style(theme::accent()));
            lines.push(Line::from(
                "Ctrl+P searches apps. The prompt on this screen talks to the desk.".to_string(),
            ));
            lines.push(Line::from(app.hardware.one_line()).style(theme::dim()));
        }
    }
    if !matches!(app.module, Some(ModuleId::Log)) {
        lines.push(Line::from(""));
        let transcript = visible_transcript(app, height.saturating_sub(lines.len()));
        lines.extend(transcript);
    }
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

fn visible_transcript(app: &App, rows: usize) -> Vec<Line<'static>> {
    let all = app.transcript();
    if all.is_empty() {
        return vec![Line::from("The stream for this view is empty.").style(theme::dim())];
    }
    let skip = app.scroll_back.min(all.len().saturating_sub(1));
    let end = all.len().saturating_sub(skip);
    let start = end.saturating_sub(rows.max(1));
    all[start..end]
        .iter()
        .map(|line| {
            let style = if line.role == "user" {
                theme::accent()
            } else if line.role == "assistant" {
                theme::text()
            } else {
                theme::dim()
            };
            let body = line.body.replace('\n', " ");
            let clipped: String = body.chars().take(220).collect();
            Line::from(format!("{}  {}", role_mark(&line.role), clipped)).style(style)
        })
        .collect()
}

fn role_mark(role: &str) -> &'static str {
    match role {
        "user" => "you",
        "assistant" => "argos",
        _ => "note",
    }
}

fn draw_log(frame: &mut Frame, app: &App, area: Rect) {
    let lines: Vec<Line> = if app.log.is_empty() {
        vec![Line::from("Tool and search notes land here.").style(theme::dim())]
    } else {
        app.log
            .iter()
            .rev()
            .take(area.height.saturating_sub(2) as usize)
            .rev()
            .cloned()
            .map(Line::from)
            .collect()
    };
    frame.render_widget(
        Paragraph::new(lines)
            .block(panel(" Recent Events "))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn draw_context(frame: &mut Frame, app: &App, area: Rect) {
    let gmail = app
        .auth
        .gmail
        .as_ref()
        .map(|g| g.email.as_str())
        .unwrap_or("not connected");
    let lines = vec![
        Line::from("Context").style(theme::accent()),
        Line::from(format!("View   {}", app.view_name())),
        Line::from(format!(
            "Text   {}",
            super::app::provider_label(app.auth.text.as_ref())
        )),
        Line::from(format!(
            "Voice  {}",
            super::app::provider_label(app.auth.voice.as_ref())
        )),
        Line::from(format!("Brain  {}", app.memories.len())),
        Line::from(format!("Gmail  {gmail}")),
        Line::from(format!("Reports {}", app.reports.len())),
        Line::from(""),
        Line::from(app.hardware.one_line()).style(theme::dim()),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .block(panel(" Context "))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn draw_tabs(frame: &mut Frame, app: &App, area: Rect) {
    let mut spans = Vec::new();
    for (i, module) in ModuleId::all().iter().enumerate() {
        let style = if i == app.tab_sel {
            theme::selected()
        } else {
            theme::dim()
        };
        spans.push(Span::styled(format!(" {} ", module.title()), style));
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)).block(panel(" Modules ")),
        area,
    );
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

fn draw_quick(frame: &mut Frame, app: &App, area: Rect) {
    frame.render_widget(Clear, area);
    let mut lines = vec![Line::from("Quick Actions").style(theme::accent())];
    for (i, module) in ModuleId::all().iter().enumerate() {
        let style = if i == app.quick_sel {
            theme::selected()
        } else {
            theme::text()
        };
        lines.push(Line::from(format!("{}. {}", i + 1, module.title())).style(style));
    }
    lines.push(Line::from(""));
    lines.push(Line::from("r  rescan hardware").style(theme::dim()));
    lines.push(Line::from("/new  open a case").style(theme::dim()));
    frame.render_widget(Paragraph::new(lines).block(panel(" Quick Actions ")), area);
}

fn draw_zen(frame: &mut Frame, app: &App, area: Rect) {
    let rows = split_v(area, &[Constraint::Length(5), Constraint::Min(3)]);
    let summary = vec![
        Line::from(format!(" {} ", app.view_name())).style(theme::accent()),
        Line::from(format!(
            "CPU {:.0}%    RAM {}%    {}",
            app.cpu_now,
            app.hardware.ram_pct(),
            app.hardware.cpu_name
        )),
        Line::from(app.hardware.one_line()).style(theme::dim()),
    ];
    frame.render_widget(Paragraph::new(summary).block(panel(" Zen ")), rows[0]);
    let data: Vec<u64> = if app.cpu_hist.is_empty() {
        vec![1, 2, 3, 2, 4]
    } else {
        app.cpu_hist.iter().copied().collect()
    };
    frame.render_widget(
        Sparkline::default()
            .block(panel(" CPU "))
            .data(&data)
            .style(theme::accent()),
        rows[1],
    );
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
    } else if app.settings.modality == "voice" {
        vec![Line::from("voice · Ctrl+R record · Enter sends").style(hint_style)]
    } else {
        vec![Line::from("text · Enter sends · /help").style(hint_style)]
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
        Focus::Canvas => "[Tab] Jump Focus | [Esc] Exit App",
        Focus::Launcher => "[Tab] Jump Focus | [Esc] Exit App",
    };
    let gap = area.width as usize;
    let used = left.chars().count() + right.chars().count();
    let spaces = gap.saturating_sub(used);
    let line = format!("{left}{}{right}", " ".repeat(spaces));
    let _ = app;
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

fn draw_help(frame: &mut Frame, area: Rect) {
    let modal = centered(
        area,
        area.width.saturating_mul(3) / 4,
        area.height.saturating_mul(3) / 4,
    );
    frame.render_widget(Clear, modal);
    let text = vec![
        Line::from("Argos OSINT").style(theme::accent()),
        Line::from("Enter sends to the view on the canvas. Esc leaves that app for the dashboard."),
        Line::from("Ctrl+P app search    Tab cycle focus    Ctrl+L next layout    Ctrl+C cancel or quit"),
        Line::from("Ctrl+R record voice  Ctrl+U clear prompt    ? help when the prompt is not focused"),
        Line::from("/search /new /use /report /hardware /provider /brain /gmail /voice /text /layout /quit"),
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
