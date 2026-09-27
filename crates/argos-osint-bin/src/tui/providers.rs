//! Account setup and model routing are deliberately separate workspaces.

use super::*;
use crate::tui::app::{provider_name, ProviderPage};
use argos_osint_core::provider;

pub fn draw(frame: &mut Frame, app: &mut App, area: Rect) {
    if app.provider_page == ProviderPage::Research {
        draw_research(frame, app, area);
        return;
    }
    app.provider_field_hits.clear();
    app.canvas_area = area;
    let rows = split_v(area, &[Constraint::Length(3), Constraint::Min(0)]);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(if app.provider_page == ProviderPage::Models {
                "Choose who writes. Choose who researches."
            } else {
                "Connect once. Keep each account separate."
            })
            .style(theme::accent()),
            Line::from(
                "Grok / OpenAI / OpenRouter keep their own access. Models assigns the roles.",
            )
            .style(theme::dim()),
        ])
        .wrap(Wrap { trim: false }),
        rows[0],
    );
    if app.provider_page == ProviderPage::Models {
        draw_roles(frame, app, rows[1]);
    } else {
        draw_account(frame, app, rows[1]);
    }
}

fn field_row(frame: &mut Frame, app: &mut App, index: usize, area: Rect) {
    let Some(field) = app.fields.get(index) else {
        return;
    };
    let active = index == app.field_sel && app.focus == Focus::Canvas;
    let style = if active {
        theme::selected()
    } else {
        theme::text()
    };
    let shown = if field.secret {
        mask(&field.value)
    } else {
        field.value.clone()
    };
    let cursor = if active && app.editing { "▍" } else { "" };
    let label = field.label.clone();
    let value = if field.key.starts_with("__") {
        format!("{} {label}  ›  {shown}", if active { "›" } else { " " })
    } else {
        format!("{} {shown}{cursor}", if active { "›" } else { " " })
    };
    if area.height < 3 {
        frame.render_widget(Paragraph::new(value).style(style), area);
        app.provider_field_hits.push((index, area));
        return;
    }
    let role_field = matches!(
        field.key.as_str(),
        "__writer_provider" | "__tool_provider" | "__role_writer" | "__role_tool"
    );
    let value = if role_field {
        format!("{} {shown}{cursor}", if active { "›" } else { " " })
    } else {
        value
    };
    let block = panel(if field.key.starts_with("__") && !role_field {
        ""
    } else {
        &label
    });
    frame.render_widget(Paragraph::new(value).block(block).style(style), area);
    app.provider_field_hits.push((index, area));
}

fn draw_account(frame: &mut Frame, app: &mut App, area: Rect) {
    let kind = app.provider_page.account().unwrap_or("grok");
    let grok = kind == "grok";
    let subscription = grok || kind == "openai-chatgpt";
    let wide = area.width >= 92;
    let cols = if wide {
        split_h(
            area,
            &[Constraint::Percentage(63), Constraint::Percentage(37)],
        )
    } else {
        vec![area]
    };
    let title = format!(
        " {} · {} ",
        app.provider_page.title(),
        if grok {
            "Subscription"
        } else if subscription {
            "ChatGPT subscription"
        } else {
            "API account"
        }
    );
    let block = panel(&title);
    let inner = block.inner(cols[0]);
    frame.render_widget(block, cols[0]);
    let status = if grok {
        app.grok_subscription_status.clone()
    } else if subscription {
        app.subscription_status.clone()
    } else {
        app.provider_draft_checks
            .get(kind)
            .cloned()
            .unwrap_or_else(|| app.account_status(kind))
    };
    let intro = if grok {
        "Use your Grok account through Grok Build subscription sign-in."
    } else if subscription {
        "Use your ChatGPT plan through Codex CLI for the Writer."
    } else {
        "Your OpenRouter key is used only for OpenRouter models."
    };
    let header_height = if subscription {
        inner.height.saturating_sub(app.fields.len() as u16).min(4)
    } else {
        4
    };
    let header = split_v(
        inner,
        &[Constraint::Length(header_height), Constraint::Min(0)],
    );
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(status.clone()).style(if grok && status.contains("blocked") {
                Style::default().fg(theme::WARN)
            } else {
                theme::accent()
            }),
            Line::from(intro).style(theme::dim()),
        ])
        .wrap(Wrap { trim: false }),
        header[0],
    );

    let pending = if grok {
        app.grok_subscription_pending
    } else {
        app.subscription_pending
    };
    let instructions = if grok {
        &app.grok_subscription_instructions
    } else {
        &app.subscription_instructions
    };
    if subscription && pending {
        let lines: Vec<Line> = if instructions.is_empty() {
            vec![Line::from(if grok {
                "Opening Grok browser sign-in or checking existing login…"
            } else {
                "Starting Codex device sign-in…"
            })
            .style(theme::accent())]
        } else {
            instructions
                .iter()
                .map(|line| Line::from(line.clone()).style(theme::accent()))
                .collect()
        };
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), header[1]);
        if wide {
            draw_account_summary(frame, app, cols[1]);
        }
        return;
    }

    let body = header[1];
    // Keep sign-in, checking an existing login, and model routing visible
    // together on short terminals. Use rounded rows when they fit.
    let field_height = if subscription && body.height < app.fields.len() as u16 * 3 {
        1u16
    } else {
        3u16
    };
    let visible = (body.height / field_height).max(1) as usize;
    // Scroll to the active field instead of hiding the Save action on short screens.
    let offset = app.field_sel.saturating_sub(visible.saturating_sub(1));
    let field_count = app.fields.len();
    for index in offset..field_count.min(offset + visible) {
        let y = body.y + (index - offset) as u16 * field_height;
        let height = field_height.min(body.bottom().saturating_sub(y));
        field_row(frame, app, index, Rect::new(body.x, y, body.width, height));
    }
    let used =
        ((field_count.saturating_sub(offset)).min(visible) as u16 * field_height).min(body.height);
    let note = Rect::new(
        body.x,
        body.y + used,
        body.width,
        body.height.saturating_sub(used),
    );
    let lines: Vec<Line> = if grok {
        vec![
            Line::from("Requires Grok Build CLI on PATH.").style(theme::dim()),
            Line::from("Sign in opens your browser. Complete the Grok account sign-in there.")
                .style(theme::dim()),
            Line::from("Or run grok login --oauth, then Check existing login.").style(theme::dim()),
            Line::from("Grok uses subscription access for Writer and Tools.").style(theme::dim()),
            Line::from("Available models depend on your Grok account's access.")
                .style(theme::dim()),
        ]
    } else if subscription {
        if app.subscription_pending && !app.subscription_instructions.is_empty() {
            app.subscription_instructions
                .iter()
                .map(|line| Line::from(line.clone()).style(theme::accent()))
                .collect()
        } else {
            vec![
                Line::from("Requires a recent Codex CLI on PATH.").style(theme::dim()),
                Line::from("Sign in opens a device flow: visit the URL and enter the code here.")
                    .style(theme::dim()),
                Line::from("Or run codex login, then Check existing login.").style(theme::dim()),
                Line::from("Subscription is Writer-only. Choose Grok or OpenRouter for Tools.")
                    .style(theme::dim()),
            ]
        }
    } else {
        let preset = provider::preset(kind).expect("account preset");
        let secret = provider::account_secret(&app.auth, kind);
        vec![
            Line::from(format!(
                "Empty key uses {}.",
                preset.env_key.unwrap_or("environment")
            ))
            .style(theme::dim()),
            Line::from(format!("Endpoint: {}", secret.base_url)).style(theme::dim()),
            Line::from("Verify checks this form; Save stores it. Neither changes model roles.")
                .style(theme::dim()),
            Line::from("Use Models to select this account for Writer or Tools.")
                .style(theme::dim()),
        ]
    };
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), note);
    if wide {
        draw_account_summary(frame, app, cols[1]);
    }
}

fn draw_account_summary(frame: &mut Frame, app: &App, area: Rect) {
    let block = panel(" Accounts & routing ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let mut lines = Vec::new();
    for (kind, name) in [
        ("grok", "Grok"),
        ("openai-chatgpt", "OpenAI · ChatGPT"),
        ("openrouter", "OpenRouter"),
    ] {
        lines.push(Line::from(name).style(theme::accent()));
        lines.push(
            Line::from(if kind == "openai-chatgpt" {
                app.subscription_status.clone()
            } else {
                app.account_status(kind)
            })
            .style(theme::dim()),
        );
        lines.push(Line::from(""));
    }
    lines.extend([
        Line::from("WRITER").style(theme::accent()),
        Line::from(app.role_label(true)),
        Line::from(""),
        Line::from("TOOLS").style(theme::accent()),
        Line::from(app.role_label(false)),
        Line::from(""),
        Line::from("Change assignments in Models.").style(theme::dim()),
    ]);
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn draw_roles(frame: &mut Frame, app: &mut App, area: Rect) {
    let rows = split_v(
        area,
        &[
            Constraint::Min(0),
            Constraint::Length(if area.height >= 20 { 3 } else { 1 }),
        ],
    );
    let cards = if area.width >= 76 {
        split_h(
            rows[0],
            &[Constraint::Percentage(50), Constraint::Percentage(50)],
        )
    } else {
        split_v(
            rows[0],
            &[Constraint::Percentage(50), Constraint::Percentage(50)],
        )
    };
    for (writer, card) in [(true, cards[0]), (false, cards[1])] {
        let block = panel(if writer { " Writer " } else { " Tools " });
        let inner = block.inner(card);
        frame.render_widget(block, card);
        let compact = inner.height < 12;
        let short = inner.height < 7;
        let parts = split_v(
            inner,
            &[
                Constraint::Length(if short {
                    0
                } else if compact {
                    1
                } else {
                    3
                }),
                Constraint::Length(if short { 1 } else { 3 }),
                Constraint::Length(if short { 1 } else { 3 }),
                Constraint::Min(0),
            ],
        );
        frame.render_widget(
            Paragraph::new(if writer {
                "Answers questions and writes reports."
            } else {
                "Calls research tools and gathers evidence."
            })
            .style(theme::dim())
            .wrap(Wrap { trim: false }),
            parts[0],
        );
        let start = if writer { 0 } else { 2 };
        field_row(frame, app, start, parts[1]);
        field_row(frame, app, start + 1, parts[2]);
        let secret = app.role_secret(writer);
        let kind = provider::effective_kind(&secret);
        let status = if kind == "openai-chatgpt" {
            app.subscription_status.clone()
        } else {
            app.account_status(&kind)
        };
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(status).style(theme::accent()),
                Line::from(if writer {
                    "Grok / OpenAI ChatGPT / OpenRouter"
                } else {
                    "Grok subscription / OpenRouter"
                })
                .style(theme::dim()),
                Line::from("Choose a provider, then select or enter a model ID.")
                    .style(theme::dim()),
            ])
            .wrap(Wrap { trim: false }),
            parts[3],
        );
    }
    frame.render_widget(Paragraph::new("Selections save automatically. Both roles can share one account or use different accounts. Switching models keeps every saved key.").style(theme::dim()).wrap(Wrap { trim: false }), rows[1]);
}

pub fn draw_picker(frame: &mut Frame, app: &App, area: Rect) {
    let Some(writer) = app.provider_picker else {
        return;
    };
    let choices = app.role_provider_choices(writer);
    let modal = centered(area, 66, 16);
    frame.render_widget(Clear, modal);
    let mut lines = vec![
        Line::from(if writer {
            "Choose the Writer account"
        } else {
            "Choose the Tools account"
        })
        .style(theme::accent()),
        Line::from("Each account retains its own credentials.").style(theme::dim()),
        Line::from(""),
    ];
    for (index, kind) in choices.iter().enumerate() {
        let style = if index == app.provider_choice {
            theme::selected()
        } else {
            theme::text()
        };
        lines.push(
            Line::from(format!(
                "{} {}",
                if index == app.provider_choice {
                    "›"
                } else {
                    " "
                },
                provider_name(kind)
            ))
            .style(style),
        );
        lines.push(
            Line::from(if *kind == "openai-chatgpt" {
                app.subscription_status.clone()
            } else {
                app.account_status(kind)
            })
            .style(theme::dim()),
        );
    }
    lines.push(Line::from(""));
    lines.push(Line::from("↑/↓ choose · Enter select · Esc cancel").style(theme::dim()));
    frame.render_widget(
        Paragraph::new(lines)
            .block(panel(if writer {
                " Writer provider "
            } else {
                " Tools provider "
            }))
            .wrap(Wrap { trim: false }),
        modal,
    );
}

fn draw_research(frame: &mut Frame, app: &mut App, area: Rect) {
    app.provider_field_hits.clear();
    app.canvas_area = area;
    let name = app.research_name();
    let c = app
        .settings
        .research
        .get(&name)
        .cloned()
        .unwrap_or_default();
    let mut intro = vec![
        Line::from(format!(
            "{name} · {:?} · enabled {}",
            c.readiness, c.enabled
        ))
        .style(theme::accent()),
        Line::from(argos_osint_core::research::capabilities(&name)).style(theme::dim()),
        Line::from(format!(
            "Supported {} · detected {} · profile {} · capabilities {} · quota {}",
            c.supported_version,
            if c.detected_version.is_empty() {
                "unverified"
            } else {
                &c.detected_version
            },
            c.scan_profile,
            c.account_capabilities,
            c.quota
        ))
        .style(theme::dim()),
    ];
    if let Some((_, _, plan)) = &app.tool_plan {
        intro.push(
            Line::from(format!(
                "PLAN: {} {} from {} to {}",
                plan.tool,
                plan.version,
                plan.source,
                plan.destination.display()
            ))
            .style(theme::accent()),
        );
        intro.push(
            Line::from(format!(
                "Prerequisites: {}. Official SHA-256 verified before activation. Apply to start.",
                plan.prerequisites.join(", ")
            ))
            .style(theme::dim()),
        );
    }
    let rows = split_v(
        area,
        &[
            Constraint::Length(if app.tool_plan.is_some() { 8 } else { 5 }),
            Constraint::Min(0),
        ],
    );
    frame.render_widget(Paragraph::new(intro).wrap(Wrap { trim: false }), rows[0]);
    let visible = rows[1].height.saturating_sub(2) as usize;
    let first = app.field_sel.saturating_sub(visible.saturating_sub(1));
    let block =
        panel(" Research settings · Enter edit/action · j/k navigate · keyboard remains active ");
    let inner = block.inner(rows[1]);
    frame.render_widget(block, rows[1]);
    for (position, index) in (first..app.fields.len()).take(visible).enumerate() {
        let field = &app.fields[index];
        let active = index == app.field_sel;
        let value = if field.secret {
            mask(&field.value)
        } else {
            field.value.clone()
        };
        let cursor = if active && app.editing { "▍" } else { "" };
        let line = format!(
            "{} {}: {}{}",
            if active { "›" } else { " " },
            field.label,
            value,
            cursor
        );
        let rect = Rect::new(inner.x, inner.y + position as u16, inner.width, 1);
        frame.render_widget(
            Paragraph::new(line).style(if active {
                theme::selected()
            } else {
                theme::text()
            }),
            rect,
        );
        app.provider_field_hits.push((index, rect));
    }
}
