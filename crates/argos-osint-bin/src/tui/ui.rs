//! Shared layout and mouse hit areas for the Argos terminal shell.

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, Paragraph, Wrap};
use ratatui::Frame;

use super::app::{App, ButtonId, FieldId, ModuleId, ProviderPage, Target};
use super::theme::{self, panel};

fn split_vertical(area: Rect, constraints: impl IntoIterator<Item = Constraint>) -> Vec<Rect> {
    Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area)
        .to_vec()
}

fn split_horizontal(area: Rect, constraints: impl IntoIterator<Item = Constraint>) -> Vec<Rect> {
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(area)
        .to_vec()
}

fn shell(area: Rect) -> (Rect, Rect, Rect, Rect) {
    let rows = split_vertical(
        area,
        [
            Constraint::Min(8),
            Constraint::Length(3),
            Constraint::Length(1),
        ],
    );
    let cols = split_horizontal(
        rows[0],
        [Constraint::Percentage(26), Constraint::Percentage(74)],
    );
    (cols[0], cols[1], rows[1], rows[2])
}

fn brain_areas(area: Rect) -> Vec<Rect> {
    let outer = split_vertical(area, [Constraint::Length(1), Constraint::Min(0)]);
    let columns = split_horizontal(
        outer[1],
        [Constraint::Percentage(52), Constraint::Percentage(48)],
    );
    let form = split_vertical(
        columns[0],
        [
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(0),
        ],
    );
    let results = split_vertical(columns[1], [Constraint::Min(4), Constraint::Length(5)]);
    vec![
        outer[0], form[0], form[1], form[2], form[3], form[4], form[5], results[0], results[1],
    ]
}

fn provider_areas(area: Rect) -> Vec<Rect> {
    split_vertical(area, [Constraint::Length(3), Constraint::Min(4)])
}

fn model_areas(area: Rect) -> Vec<Rect> {
    split_vertical(
        area,
        [
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(0),
        ],
    )
}

fn auth_areas(area: Rect) -> Vec<Rect> {
    split_vertical(
        area,
        [
            Constraint::Length(3),
            Constraint::Length(4),
            Constraint::Length(3),
            Constraint::Min(0),
        ],
    )
}

fn router_areas(area: Rect) -> Vec<Rect> {
    split_vertical(
        area,
        [
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(0),
        ],
    )
}

fn button_areas(area: Rect, count: usize) -> Vec<Rect> {
    let constraints = (0..count).map(|_| Constraint::Ratio(1, count as u32));
    split_horizontal(area, constraints)
}

fn contains(rect: Rect, x: u16, y: u16) -> bool {
    x >= rect.x
        && x < rect.x.saturating_add(rect.width)
        && y >= rect.y
        && y < rect.y.saturating_add(rect.height)
}

fn visible_start(app: &App, list: Rect) -> usize {
    let room = list.height.saturating_sub(2) as usize / 2;
    app.memory_sel.saturating_sub(room.saturating_sub(1))
}

pub fn focus_order(app: &App) -> Vec<Target> {
    let mut order = ModuleId::ALL
        .iter()
        .enumerate()
        .map(|(index, _)| Target::App(index))
        .collect::<Vec<_>>();
    match app.module {
        Some(ModuleId::Brain) => {
            order.extend(
                [
                    FieldId::BrainApp,
                    FieldId::BrainConversation,
                    FieldId::BrainInsight,
                    FieldId::BrainQuery,
                ]
                .map(Target::Field),
            );
            order.extend(
                [
                    ButtonId::Add,
                    ButtonId::Recall,
                    ButtonId::Pin,
                    ButtonId::Delete,
                ]
                .map(Target::Button),
            );
            if !app.memories.is_empty() {
                order.push(Target::Memory(app.memory_sel));
            }
        }
        Some(ModuleId::Providers) => {
            order.extend(ProviderPage::ALL.map(Target::ProviderTab));
            match app.provider_page {
                ProviderPage::Grok => {
                    order.extend([ButtonId::GrokSignIn, ButtonId::GrokCheck].map(Target::Button))
                }
                ProviderPage::OpenAI => order
                    .extend([ButtonId::OpenAISignIn, ButtonId::OpenAICheck].map(Target::Button)),
                ProviderPage::OpenRouter => {
                    order.push(Target::Field(FieldId::RouterKey));
                    order.extend(
                        [
                            ButtonId::RouterSave,
                            ButtonId::RouterVerify,
                            ButtonId::RouterAdvanced,
                        ]
                        .map(Target::Button),
                    );
                    if app.router_advanced {
                        order.push(Target::Field(FieldId::RouterEndpoint));
                    }
                }
                ProviderPage::Models => order.extend([
                    Target::Field(FieldId::WriterProvider),
                    Target::Field(FieldId::WriterModel),
                    Target::Button(ButtonId::SaveWriter),
                ]),
            }
        }
        Some(ModuleId::System) => order.push(Target::Button(ButtonId::RefreshHardware)),
        None => {}
    }
    order.push(Target::Field(FieldId::Composer));
    order
}

pub fn hit_test(app: &App, x: u16, y: u16) -> Option<Target> {
    let (launcher, canvas, composer, _) = shell(app.screen);
    if contains(composer, x, y) {
        return Some(Target::Field(FieldId::Composer));
    }
    if contains(launcher, x, y) && y > launcher.y {
        let index = (y - launcher.y - 1) as usize / 2;
        if index < ModuleId::ALL.len() {
            return Some(Target::App(index));
        }
    }
    if !contains(canvas, x, y) {
        return None;
    }
    match app.module {
        Some(ModuleId::Brain) => {
            let rows = brain_areas(canvas);
            if contains(rows[1], x, y) {
                return Some(Target::Field(FieldId::BrainApp));
            }
            if contains(rows[2], x, y) {
                return Some(Target::Field(FieldId::BrainConversation));
            }
            if contains(rows[3], x, y) {
                return Some(Target::Field(FieldId::BrainInsight));
            }
            if contains(rows[4], x, y) {
                return Some(Target::Field(FieldId::BrainQuery));
            }
            if contains(rows[5], x, y) {
                let buttons = button_areas(rows[5], 2);
                let ids = [ButtonId::Add, ButtonId::Recall];
                return buttons
                    .iter()
                    .position(|rect| contains(*rect, x, y))
                    .map(|index| Target::Button(ids[index]));
            }
            if contains(rows[6], x, y) {
                let buttons = button_areas(rows[6], 2);
                let ids = [ButtonId::Pin, ButtonId::Delete];
                return buttons
                    .iter()
                    .position(|rect| contains(*rect, x, y))
                    .map(|index| Target::Button(ids[index]));
            }
            if contains(rows[7], x, y)
                && y > rows[7].y
                && y < rows[7].y + rows[7].height.saturating_sub(1)
            {
                let index = visible_start(app, rows[7]) + (y - rows[7].y - 1) as usize / 2;
                if index < app.memories.len() {
                    return Some(Target::Memory(index));
                }
            }
        }
        Some(ModuleId::Providers) => {
            let rows = provider_areas(canvas);
            if contains(rows[0], x, y) {
                let tabs = button_areas(rows[0], 4);
                return tabs
                    .iter()
                    .position(|rect| contains(*rect, x, y))
                    .map(|index| Target::ProviderTab(ProviderPage::ALL[index]));
            }
            match app.provider_page {
                ProviderPage::Grok | ProviderPage::OpenAI => {
                    let auth = auth_areas(rows[1]);
                    if contains(auth[2], x, y) {
                        let buttons = button_areas(auth[2], 2);
                        let ids = if app.provider_page == ProviderPage::Grok {
                            [ButtonId::GrokSignIn, ButtonId::GrokCheck]
                        } else {
                            [ButtonId::OpenAISignIn, ButtonId::OpenAICheck]
                        };
                        return buttons
                            .iter()
                            .position(|rect| contains(*rect, x, y))
                            .map(|index| Target::Button(ids[index]));
                    }
                }
                ProviderPage::OpenRouter => {
                    let router = router_areas(rows[1]);
                    if contains(router[1], x, y) {
                        return Some(Target::Field(FieldId::RouterKey));
                    }
                    if contains(router[2], x, y) {
                        let buttons = button_areas(router[2], 2);
                        return Some(Target::Button(if contains(buttons[0], x, y) {
                            ButtonId::RouterSave
                        } else {
                            ButtonId::RouterVerify
                        }));
                    }
                    if contains(router[3], x, y) {
                        return Some(Target::Button(ButtonId::RouterAdvanced));
                    }
                    if app.router_advanced && contains(router[4], x, y) {
                        return Some(Target::Field(FieldId::RouterEndpoint));
                    }
                }
                ProviderPage::Models => {
                    let models = model_areas(rows[1]);
                    if contains(models[1], x, y) {
                        return Some(Target::Field(FieldId::WriterProvider));
                    }
                    if contains(models[2], x, y) {
                        return Some(Target::Field(FieldId::WriterModel));
                    }
                    if contains(models[3], x, y) {
                        return Some(Target::Button(ButtonId::SaveWriter));
                    }
                }
            }
        }
        Some(ModuleId::System) => {
            let rows = split_vertical(
                canvas,
                [
                    Constraint::Length(9),
                    Constraint::Length(3),
                    Constraint::Min(0),
                ],
            );
            if contains(rows[1], x, y) {
                return Some(Target::Button(ButtonId::RefreshHardware));
            }
        }
        None => {}
    }
    None
}

fn field_rect(app: &App, field: FieldId) -> Option<Rect> {
    let (_, canvas, composer, _) = shell(app.screen);
    match field {
        FieldId::Composer => Some(composer),
        FieldId::BrainApp
        | FieldId::BrainConversation
        | FieldId::BrainInsight
        | FieldId::BrainQuery
            if app.module == Some(ModuleId::Brain) =>
        {
            let rows = brain_areas(canvas);
            match field {
                FieldId::BrainApp => Some(rows[1]),
                FieldId::BrainConversation => Some(rows[2]),
                FieldId::BrainInsight => Some(rows[3]),
                _ => Some(rows[4]),
            }
        }
        FieldId::WriterProvider | FieldId::WriterModel
            if app.module == Some(ModuleId::Providers) =>
        {
            let rows = model_areas(provider_areas(canvas)[1]);
            Some(if field == FieldId::WriterProvider {
                rows[1]
            } else {
                rows[2]
            })
        }
        FieldId::RouterKey | FieldId::RouterEndpoint if app.module == Some(ModuleId::Providers) => {
            let rows = router_areas(provider_areas(canvas)[1]);
            Some(if field == FieldId::RouterKey {
                rows[1]
            } else {
                rows[4]
            })
        }
        _ => None,
    }
}

fn viewport(app: &App, field: FieldId, area: Rect) -> usize {
    let width = area.width.saturating_sub(2) as usize;
    if app.focus == Target::Field(field) {
        app.cursor.saturating_sub(width.saturating_sub(1))
    } else {
        app.field(field).chars().count().saturating_sub(width)
    }
}

pub fn cursor_at(app: &App, field: FieldId, x: u16) -> usize {
    let Some(area) = field_rect(app, field) else {
        return app.field(field).chars().count();
    };
    let offset = x.saturating_sub(area.x.saturating_add(1)) as usize;
    (viewport(app, field, area) + offset).min(app.field(field).chars().count())
}

fn draw_field(frame: &mut Frame, app: &App, field: FieldId, label: &str, area: Rect) {
    let value = app.field(field);
    let secret = field == FieldId::RouterKey;
    let display = if secret {
        "•".repeat(value.chars().count())
    } else {
        value.to_string()
    };
    let scroll = viewport(app, field, area);
    let width = area.width.saturating_sub(2) as usize;
    let visible: String = display.chars().skip(scroll).take(width).collect();
    let focused = app.focus == Target::Field(field);
    let block = if focused {
        panel(label).border_style(theme::selected())
    } else {
        panel(label)
    };
    frame.render_widget(
        Paragraph::new(if display.is_empty() && !focused {
            "Click to edit".into()
        } else {
            visible
        })
        .style(if focused {
            theme::user_message()
        } else {
            theme::text()
        })
        .block(block),
        area,
    );
    if focused && area.width > 2 && area.height > 2 {
        let cursor_x = area.x + 1 + (app.cursor.saturating_sub(scroll) as u16).min(area.width - 2);
        frame.set_cursor_position((cursor_x, area.y + 1));
    }
}

fn draw_button(frame: &mut Frame, app: &App, button: ButtonId, label: &str, area: Rect) {
    let selected = app.focus == Target::Button(button);
    frame.render_widget(
        Paragraph::new(label)
            .alignment(Alignment::Center)
            .style(if selected {
                theme::selected()
            } else {
                theme::text()
            })
            .block(if selected {
                panel(" Action ").border_style(theme::selected())
            } else {
                panel(" Action ")
            }),
        area,
    );
}

pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    frame.render_widget(Paragraph::new("").style(theme::text()), area);
    let (launcher, canvas, composer, footer) = shell(area);
    draw_launcher(frame, app, launcher);
    draw_canvas(frame, app, canvas);
    draw_field(
        frame,
        app,
        FieldId::Composer,
        " Composer · commands and shortcuts ",
        composer,
    );
    frame.render_widget(Paragraph::new(format!(" {}  ·  Click controls  Tab/Shift+Tab focus  Enter activate  F1–F3 apps  Esc dashboard  Ctrl+C quit", app.status)).style(theme::dim()), footer);
}

fn draw_launcher(frame: &mut Frame, app: &App, area: Rect) {
    let items = ModuleId::ALL
        .iter()
        .enumerate()
        .map(|(index, module)| {
            let line = format!(
                "{}  {}\n   {}",
                if app.module == Some(*module) {
                    "●"
                } else {
                    "○"
                },
                module.title(),
                module.blurb()
            );
            ListItem::new(line).style(
                if app.focus == Target::App(index) || index == app.launcher_sel {
                    theme::selected()
                } else {
                    theme::text()
                },
            )
        })
        .collect::<Vec<_>>();
    frame.render_widget(List::new(items).block(panel(" Argos apps ")), area);
}

fn draw_canvas(frame: &mut Frame, app: &App, area: Rect) {
    match app.module {
        None => draw_home(frame, area),
        Some(ModuleId::Brain) => draw_brain(frame, app, area),
        Some(ModuleId::Providers) => draw_providers(frame, app, area),
        Some(ModuleId::System) => draw_system(frame, app, area),
    }
}

fn draw_home(frame: &mut Frame, area: Rect) {
    let text = vec![
        Line::from(Span::styled("ARGOS", theme::accent())),
        Line::from(""),
        Line::from("Memory recall for connected conversations."),
        Line::from(""),
        Line::from("Click an app on the left, or press F1–F3."),
        Line::from(
            "Brain stores insights with their origin. Providers manages accounts and models.",
        ),
    ];
    frame.render_widget(
        Paragraph::new(text)
            .style(theme::text())
            .block(panel(" Welcome "))
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn draw_brain(frame: &mut Frame, app: &App, area: Rect) {
    let rows = brain_areas(area);
    frame.render_widget(
        Paragraph::new(
            " Brain · insight categories: fact, identity, preference, contact, project, goal, task",
        )
        .style(theme::accent()),
        rows[0],
    );
    draw_field(frame, app, FieldId::BrainApp, " Source app * ", rows[1]);
    draw_field(
        frame,
        app,
        FieldId::BrainConversation,
        " Conversation ID * ",
        rows[2],
    );
    draw_field(frame, app, FieldId::BrainInsight, " Insight * ", rows[3]);
    draw_field(
        frame,
        app,
        FieldId::BrainQuery,
        " Recall question ",
        rows[4],
    );
    let buttons = button_areas(rows[5], 2);
    draw_button(frame, app, ButtonId::Add, "Save insight", buttons[0]);
    draw_button(frame, app, ButtonId::Recall, "Recall", buttons[1]);
    let buttons = button_areas(rows[6], 2);
    draw_button(frame, app, ButtonId::Pin, "Pin / unpin", buttons[0]);
    draw_button(frame, app, ButtonId::Delete, "Delete", buttons[1]);
    let start = visible_start(app, rows[7]);
    let room = rows[7].height.saturating_sub(2) as usize / 2;
    let items = app
        .memories
        .iter()
        .enumerate()
        .skip(start)
        .take(room)
        .map(|(index, memory)| {
            let title = format!(
                "{} [{}] {}",
                if memory.pinned { "◆" } else { " " },
                memory.category,
                memory.text
            );
            let source = format!(
                "  {} / {}",
                memory.source.app, memory.source.conversation_id
            );
            ListItem::new(format!("{title}\n{source}")).style(if index == app.memory_sel {
                theme::selected()
            } else {
                theme::text()
            })
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        List::new(items).block(panel(" Saved memories · click to select ")),
        rows[7],
    );
    let recalled = if app.hits.is_empty() {
        "Enter a question and click Recall.".into()
    } else {
        app.hits
            .iter()
            .map(|hit| {
                format!(
                    "{:.2} {} — {} / {}",
                    hit.score,
                    hit.memory.text,
                    hit.memory.source.app,
                    hit.memory.source.conversation_id
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    frame.render_widget(
        Paragraph::new(recalled)
            .style(theme::text())
            .block(panel(" Recall results "))
            .wrap(Wrap { trim: true }),
        rows[8],
    );
}

fn draw_providers(frame: &mut Frame, app: &App, area: Rect) {
    let rows = provider_areas(area);
    let tabs = button_areas(rows[0], ProviderPage::ALL.len());
    for (index, page) in ProviderPage::ALL.into_iter().enumerate() {
        let selected = app.provider_page == page;
        frame.render_widget(
            Paragraph::new(page.title())
                .alignment(Alignment::Center)
                .style(if selected {
                    theme::selected()
                } else {
                    theme::dim()
                })
                .block(panel(" Providers ")),
            tabs[index],
        );
    }
    match app.provider_page {
        ProviderPage::Grok | ProviderPage::OpenAI => {
            let grok = app.provider_page == ProviderPage::Grok;
            let content = auth_areas(rows[1]);
            let intro = if grok {
                "Grok Build subscription sign-in. The account stays with Grok."
            } else {
                "ChatGPT subscription sign-in through Codex CLI for the Writer."
            };
            frame.render_widget(
                Paragraph::new(intro)
                    .style(theme::dim())
                    .block(panel(" Account "))
                    .wrap(Wrap { trim: true }),
                content[0],
            );
            let status = if grok {
                &app.grok_status
            } else {
                &app.openai_status
            };
            frame.render_widget(
                Paragraph::new(status.as_str())
                    .style(theme::accent())
                    .block(panel(" Connection "))
                    .wrap(Wrap { trim: true }),
                content[1],
            );
            let buttons = button_areas(content[2], 2);
            if grok {
                draw_button(
                    frame,
                    app,
                    ButtonId::GrokSignIn,
                    "Sign in with Grok",
                    buttons[0],
                );
                draw_button(
                    frame,
                    app,
                    ButtonId::GrokCheck,
                    "Check existing login",
                    buttons[1],
                );
            } else {
                draw_button(
                    frame,
                    app,
                    ButtonId::OpenAISignIn,
                    "Sign in with ChatGPT",
                    buttons[0],
                );
                draw_button(
                    frame,
                    app,
                    ButtonId::OpenAICheck,
                    "Check existing login",
                    buttons[1],
                );
            }
            let progress = if app.provider_progress_page == Some(app.provider_page)
                && !app.provider_progress.is_empty()
            {
                app.provider_progress.join("\n")
            } else if grok {
                "Sign in opens Grok's browser flow. You can also run grok login --oauth, then check here.".into()
            } else {
                "Sign in shows a device URL and code. You can also run codex login, then check here.".into()
            };
            frame.render_widget(
                Paragraph::new(progress)
                    .style(theme::dim())
                    .block(panel(" Sign-in details "))
                    .wrap(Wrap { trim: true }),
                content[3],
            );
        }
        ProviderPage::OpenRouter => {
            let router = router_areas(rows[1]);
            frame.render_widget(
                Paragraph::new(app.router_status.as_str())
                    .style(theme::accent())
                    .block(panel(" OpenRouter API account "))
                    .wrap(Wrap { trim: true }),
                router[0],
            );
            draw_field(
                frame,
                app,
                FieldId::RouterKey,
                " API key · empty uses OPENROUTER_API_KEY ",
                router[1],
            );
            let buttons = button_areas(router[2], 2);
            draw_button(frame, app, ButtonId::RouterSave, "Save key", buttons[0]);
            draw_button(
                frame,
                app,
                ButtonId::RouterVerify,
                "Verify connection",
                buttons[1],
            );
            draw_button(
                frame,
                app,
                ButtonId::RouterAdvanced,
                if app.router_advanced {
                    "Hide advanced endpoint"
                } else {
                    "Show advanced endpoint"
                },
                router[3],
            );
            if app.router_advanced {
                draw_field(
                    frame,
                    app,
                    FieldId::RouterEndpoint,
                    " HTTPS API endpoint ",
                    router[4],
                );
            }
            frame.render_widget(Paragraph::new("Verify tests the current form without saving it. Save stores only this account; model routing stays separate.").style(theme::dim()).wrap(Wrap { trim: true }), router[5]);
        }
        ProviderPage::Models => {
            let models = model_areas(rows[1]);
            frame.render_widget(
                Paragraph::new("Choose the writer account and model for future chat apps.")
                    .style(theme::dim())
                    .block(panel(" Models ")),
                models[0],
            );
            draw_field(
                frame,
                app,
                FieldId::WriterProvider,
                " Provider · grok / openai-chatgpt / openrouter ",
                models[1],
            );
            draw_field(frame, app, FieldId::WriterModel, " Model ID ", models[2]);
            draw_button(frame, app, ButtonId::SaveWriter, "Save writer", models[3]);
        }
    }
}

fn draw_system(frame: &mut Frame, app: &App, area: Rect) {
    let rows = split_vertical(
        area,
        [
            Constraint::Length(9),
            Constraint::Length(3),
            Constraint::Min(0),
        ],
    );
    let body = format!(
        "Hardware\n{}\nBackend: {}\nLogical cores: {}\n\nConfig: {}\nDatabase: {}",
        app.hardware.one_line(),
        app.hardware.backend,
        app.hardware.logical_cores,
        argos_osint_core::paths::config_path().display(),
        argos_osint_core::paths::db_path().display()
    );
    frame.render_widget(
        Paragraph::new(body)
            .style(Style::default().fg(theme::TEXT).bg(theme::BG))
            .block(panel(" System "))
            .wrap(Wrap { trim: true }),
        rows[0],
    );
    draw_button(
        frame,
        app,
        ButtonId::RefreshHardware,
        "Refresh hardware",
        rows[1],
    );
}
