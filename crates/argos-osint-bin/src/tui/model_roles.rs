//! Reusable editor state and presentation for the 9 logical model roles.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;

use super::app::{App, DefaultsRole};
use super::theme;
use argos_osint_core::provider;

/// Metadata and purpose for each logical role in the investigation harness.
#[derive(Clone, Copy, Debug)]
pub struct RoleMetadata {
    pub role: DefaultsRole,
    pub name: &'static str,
    pub purpose: &'static str,
    #[allow(dead_code)]
    pub default_inheritance: Option<&'static str>,
}

pub const ROLE_METADATA: [RoleMetadata; 9] = [
    RoleMetadata {
        role: DefaultsRole::Recon,
        name: "Recon (Planner)",
        purpose: "Formulates directives, query decomposition, and task dependencies.",
        default_inheritance: None,
    },
    RoleMetadata {
        role: DefaultsRole::ToolPicker,
        name: "Tool picker",
        purpose: "Selects next eligible OSINT tool and argument bindings.",
        default_inheritance: Some("OpenRouter Jev"),
    },
    RoleMetadata {
        role: DefaultsRole::Synthesis,
        name: "Synthesis",
        purpose: "Generates structured assessments, final briefings, and executive summaries.",
        default_inheritance: None,
    },
    RoleMetadata {
        role: DefaultsRole::Classifier,
        name: "Classifier",
        purpose: "OSINT taxonomy classification and tag assignment.",
        default_inheritance: Some("OpenRouter Jev"),
    },
    RoleMetadata {
        role: DefaultsRole::Summarization,
        name: "Summarization",
        purpose: "Evidence compression and source-grounded views.",
        default_inheritance: Some("Synthesis"),
    },
    RoleMetadata {
        role: DefaultsRole::EvidenceCurator,
        name: "Evidence curator",
        purpose: "Extracts source-linked passages, observations, and temporal dates.",
        default_inheritance: Some("Recon"),
    },
    RoleMetadata {
        role: DefaultsRole::EntityResolver,
        name: "Entity resolver",
        purpose: "Correlates cross-platform identifiers and resolves entity conflicts.",
        default_inheritance: Some("Classifier"),
    },
    RoleMetadata {
        role: DefaultsRole::ClaimAssessor,
        name: "Claim assessor",
        purpose: "Evaluates per-claim verification, stances, and citations.",
        default_inheritance: Some("Synthesis"),
    },
    RoleMetadata {
        role: DefaultsRole::InvestigationController,
        name: "Investigation controller",
        purpose: "Verifies checkpoints, stopping conditions, and next-task dispatch.",
        default_inheritance: Some("Recon"),
    },
];

/// Account readiness status.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccountHealth {
    Ready,
    Unverified,
    Cooldown,
    Overloaded,
    AuthRequired,
    Restricted,
    CreditsExhausted,
}

impl AccountHealth {
    pub fn badge(self) -> (&'static str, ratatui::style::Color) {
        match self {
            AccountHealth::Ready => ("Ready", ratatui::style::Color::Green),
            AccountHealth::Unverified => ("Unverified", ratatui::style::Color::DarkGray),
            AccountHealth::Cooldown => ("Cooldown", ratatui::style::Color::Yellow),
            AccountHealth::Overloaded => ("Overloaded", ratatui::style::Color::LightRed),
            AccountHealth::AuthRequired => ("Auth req", ratatui::style::Color::Red),
            AccountHealth::Restricted => ("Restricted", ratatui::style::Color::Magenta),
            AccountHealth::CreditsExhausted => ("Exhausted", ratatui::style::Color::Red),
        }
    }
}

/// Computes the presentation info for a given role from the app's settings.
pub fn role_presentation(app: &App, role: DefaultsRole) -> RolePresentationInfo {
    let meta = ROLE_METADATA
        .iter()
        .find(|m| m.role == role)
        .copied()
        .unwrap_or(ROLE_METADATA[0]);

    let (assignment, inherited_from) = app.settings.defaults.resolve_role(role.settings_key());
    let explicit_provider = app.field(role.provider_field()).trim().to_string();
    let explicit_model = app.field(role.model_field()).trim().to_string();
    let is_explicit = !explicit_provider.is_empty() && !explicit_model.is_empty();

    let provider_name = if is_explicit {
        &explicit_provider
    } else {
        &assignment.provider
    };
    let model_name = if is_explicit {
        &explicit_model
    } else {
        &assignment.model
    };

    let health = if provider_name.is_empty() {
        AccountHealth::Unverified
    } else {
        AccountHealth::Ready
    };

    RolePresentationInfo {
        meta,
        is_explicit,
        inherited_from: inherited_from.map(|s| s.to_string()),
        resolved_provider: provider_name.clone(),
        resolved_model: model_name.clone(),
        health,
    }
}

pub struct RolePresentationInfo {
    pub meta: RoleMetadata,
    pub is_explicit: bool,
    pub inherited_from: Option<String>,
    pub resolved_provider: String,
    pub resolved_model: String,
    pub health: AccountHealth,
}

/// Draws the scrollable role list and current role editor in Providers -> Defaults.
pub fn draw_model_roles_view(frame: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(42), Constraint::Percentage(58)])
        .split(area);

    let list_area = chunks[0];
    let editor_area = chunks[1];

    // Left pane: 9 roles scrollable list
    let items: Vec<ListItem> = DefaultsRole::ALL
        .iter()
        .map(|&role| {
            let info = role_presentation(app, role);
            let selected = role == app.defaults_role;

            let prefix = if selected { "▶ " } else { "  " };
            let (h_text, h_color) = info.health.badge();

            let role_span = Span::styled(
                format!("{}{:<18}", prefix, info.meta.role.label()),
                if selected {
                    theme::accent().add_modifier(Modifier::BOLD)
                } else {
                    theme::text()
                },
            );

            let inherit_label = if let Some(parent) = info.inherited_from {
                format!("(via {parent}) ")
            } else {
                String::new()
            };

            let target_span = Span::styled(
                format!(
                    "{inherit_label}{} / {}",
                    info.resolved_provider, info.resolved_model
                ),
                theme::dim(),
            );

            let badge_span = Span::styled(
                format!(" [{h_text}]"),
                ratatui::style::Style::default().fg(h_color),
            );

            let line = Line::from(vec![role_span, target_span, badge_span]);
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Decision & Analytical Roles (9 Profiles) ")
            .border_style(ratatui::style::Style::default().fg(theme::BORDER)),
    );
    frame.render_widget(list, list_area);

    // Right pane: details & purpose for currently selected role
    let sel_info = role_presentation(app, app.defaults_role);
    let mut details_lines = Vec::new();
    details_lines.push(Line::from(vec![
        Span::styled(
            sel_info.meta.name,
            theme::accent().add_modifier(Modifier::BOLD),
        ),
        Span::raw("  —  "),
        Span::styled(sel_info.meta.purpose, theme::dim()),
    ]));
    details_lines.push(Line::raw(""));

    let inherit_note = if let Some(parent) = &sel_info.inherited_from {
        format!("Assignment: Inherited from `{parent}` (set explicitly below to override)")
    } else if sel_info.is_explicit {
        "Assignment: Explicitly configured".to_string()
    } else {
        "Assignment: Root default".to_string()
    };
    details_lines.push(Line::from(Span::styled(inherit_note, theme::text())));

    let adapter = argos_osint_core::investigation::decisions::resolve_adapter(
        &sel_info.resolved_model,
        &sel_info.resolved_provider,
    );
    details_lines.push(Line::from(Span::styled(
        format!("Adapter: {} (strict output contract)", adapter.as_str()),
        theme::accent(),
    )));

    if let Some(transport) = if sel_info.meta.role == DefaultsRole::ToolPicker {
        Some(provider::picker_transport(&sel_info.resolved_model))
    } else {
        None
    } {
        details_lines.push(Line::from(Span::styled(
            format!("Transport: {transport} (Jev alpha decisions API supported)"),
            theme::dim(),
        )));
    }

    let p = Paragraph::new(details_lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" {} Details ", sel_info.meta.role.label()))
                .border_style(ratatui::style::Style::default().fg(theme::BORDER)),
        )
        .wrap(Wrap { trim: true });
    frame.render_widget(p, editor_area);
}
