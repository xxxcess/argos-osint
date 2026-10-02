//! Layout, chat transcript, and mouse hit areas for the Argos terminal shell.

use std::collections::HashSet;

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, Paragraph, Wrap};
use ratatui::Frame;

use super::markdown::{self, Piece, Tone};

use super::app::{
    is_picker_field, App, ButtonId, ChoiceKind, DefaultsRole, FieldId, ModuleId, Overlay,
    ProviderPage, Target,
};
use super::theme::{self, panel};
use argos_osint_core::osint;
use argos_osint_core::provider;
use argos_osint_core::recon::{self, Plan};

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

struct Chrome {
    header: Rect,
    body: Rect,
    composer: Rect,
    footer: Rect,
    home: Rect,
}

fn composer_height(app: &App) -> u16 {
    if app.module != Some(ModuleId::Recon) || !app.recon_chat {
        return 0;
    }
    let rows = app.input.split('\n').count().max(1) as u16;
    (rows + 2).clamp(3, 6)
}

fn chrome(area: Rect, app: &App) -> Chrome {
    let header_h = u16::from(app.module.is_some());
    let composer_h = composer_height(app);
    let rows = split_vertical(
        area,
        [
            Constraint::Length(header_h),
            Constraint::Min(0),
            Constraint::Length(composer_h),
            Constraint::Length(1),
        ],
    );
    let home = if header_h == 0 || rows[0].width < 8 {
        Rect::default()
    } else {
        Rect {
            x: rows[0].x + rows[0].width.saturating_sub(8),
            y: rows[0].y,
            width: 8.min(rows[0].width),
            height: 1,
        }
    };
    Chrome {
        header: rows[0],
        body: rows[1],
        composer: rows[2],
        footer: rows[3],
        home,
    }
}

fn inset(area: Rect) -> Rect {
    Rect {
        x: area.x.saturating_add(1),
        y: area.y.saturating_add(1),
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    }
}

fn contains(rect: Rect, x: u16, y: u16) -> bool {
    x >= rect.x
        && x < rect.x.saturating_add(rect.width)
        && y >= rect.y
        && y < rect.y.saturating_add(rect.height)
}

fn button_areas(area: Rect, count: usize) -> Vec<Rect> {
    let count = count.max(1);
    split_horizontal(area, (0..count).map(|_| Constraint::Ratio(1, count as u32)))
}

fn composer_parts(area: Rect) -> (Rect, Rect) {
    let send = 12.min(area.width / 4);
    let parts = split_horizontal(area, [Constraint::Min(8), Constraint::Length(send.max(8))]);
    (parts[0], parts[1])
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
    let results = split_vertical(columns[1], [Constraint::Min(4), Constraint::Length(6)]);
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
            Constraint::Length(3),
            Constraint::Min(0),
        ],
    )
}

fn dashboard_areas(area: Rect) -> (Rect, Rect, Rect) {
    let rows = split_vertical(
        area,
        [
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(3),
        ],
    );
    (rows[0], rows[1], rows[2])
}

fn chat_areas(area: Rect) -> (Rect, Rect) {
    let rows = split_vertical(area, [Constraint::Min(0), Constraint::Length(3)]);
    (rows[0], rows[1])
}

struct OsintLayout {
    search: Rect,
    list: Rect,
    detail: Rect,
    key: Rect,
    input: Rect,
    actions: Rect,
}

struct ApiKeySlot {
    field: FieldId,
    button: ButtonId,
}

/// Key row for the selected keyed tool. Firecrawl, SociaVault, Hunter, NewsAPI, and
/// CourtListener tools each share one key per provider.
fn api_key_slot(app: &App) -> Option<ApiKeySlot> {
    let id = osint::registry().get(app.tool_sel)?.id;
    let (field, button) = if id.starts_with("firecrawl_") {
        (FieldId::FirecrawlKey, ButtonId::SaveFirecrawlKey)
    } else if id.starts_with("sociavault_") {
        (FieldId::SociaVaultKey, ButtonId::SaveSociaVaultKey)
    } else if id.starts_with("hunter_") {
        (FieldId::HunterKey, ButtonId::SaveHunterKey)
    } else if id.starts_with("newsapi_") {
        (FieldId::NewsApiKey, ButtonId::SaveNewsApiKey)
    } else if id.starts_with("courtlistener_") {
        (FieldId::CourtListenerKey, ButtonId::SaveCourtListenerKey)
    } else {
        return None;
    };
    Some(ApiKeySlot { field, button })
}

fn osint_areas(area: Rect, with_key: bool) -> OsintLayout {
    let top = split_vertical(area, [Constraint::Length(3), Constraint::Min(0)]);
    let columns = if area.width >= 68 {
        split_horizontal(
            top[1],
            [Constraint::Percentage(38), Constraint::Percentage(62)],
        )
    } else {
        split_vertical(top[1], [Constraint::Length(6), Constraint::Min(0)])
    };
    let right = if with_key {
        split_vertical(
            columns[1],
            [
                Constraint::Min(4),
                Constraint::Length(3),
                Constraint::Length(3),
                Constraint::Length(3),
            ],
        )
    } else {
        split_vertical(
            columns[1],
            [
                Constraint::Min(4),
                Constraint::Length(3),
                Constraint::Length(3),
            ],
        )
    };
    if with_key {
        OsintLayout {
            search: top[0],
            list: columns[0],
            detail: right[0],
            key: right[1],
            input: right[2],
            actions: right[3],
        }
    } else {
        OsintLayout {
            search: top[0],
            list: columns[0],
            detail: right[0],
            key: Rect::default(),
            input: right[1],
            actions: right[2],
        }
    }
}

fn system_areas(area: Rect) -> (Rect, Rect, Rect) {
    let rows = split_vertical(
        area,
        [
            Constraint::Length(8),
            Constraint::Length(3),
            Constraint::Min(0),
        ],
    );
    (rows[0], rows[1], rows[2])
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

fn popup_area(area: Rect) -> Rect {
    let width = area.width.clamp(24, 76).min(area.width);
    let height = area.height.saturating_sub(2).clamp(8, 28).min(area.height);
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

fn home_line(app: &App, x: u16, y: u16) -> Option<usize> {
    let body = chrome(app.screen, app).body;
    if !contains(inset(body), x, y) {
        return None;
    }
    let line = (y - body.y - 1) as usize;
    match line {
        3 => Some(0),
        4 => Some(1),
        7 => Some(2),
        8 => Some(3),
        9 => Some(4),
        _ => None,
    }
}

fn visible_tools(app: &App) -> Vec<(usize, &'static osint::ToolDefinition)> {
    let q = app.osint_search.trim().to_ascii_lowercase();
    osint::registry()
        .iter()
        .enumerate()
        .filter(|(_, tool)| {
            q.is_empty()
                || tool.name.to_ascii_lowercase().contains(&q)
                || tool.category.to_ascii_lowercase().contains(&q)
                || tool.id.contains(&q)
        })
        .collect()
}

#[derive(Clone, Debug)]
pub struct ChatBlock {
    pub key: String,
    pub title: String,
    pub body: String,
    pub collapsible: bool,
    pub message_index: Option<usize>,
    pub has_memory: bool,
}

pub fn chat_blocks(app: &App) -> Vec<ChatBlock> {
    ensure_frame(app);
    app.frame.borrow().blocks.clone()
}

fn build_blocks(app: &App) -> Vec<ChatBlock> {
    let mut blocks = Vec::new();
    let mut used = HashSet::new();
    for (index, message) in app.messages.iter().enumerate() {
        if message.role == "user" {
            blocks.push(ChatBlock {
                key: format!("user:{}", message.id),
                title: "You".into(),
                body: message.content.clone(),
                collapsible: false,
                message_index: Some(index),
                has_memory: false,
            });
            if let Some(run) = app.runs.iter().find(|run| run.turn_id == message.id) {
                blocks.push(plan_block(
                    run,
                    &app.calls,
                    app.expanded.contains(&format!("plan:{}", run.id)),
                ));
                for (call_index, call) in app.calls.iter().enumerate() {
                    if call.run_id.as_deref() == Some(run.id.as_str()) {
                        used.insert(call_index);
                        blocks.push(tool_block(app, call));
                    }
                }
                let answered = app.messages.iter().any(|item| {
                    item.role == "assistant" && item.run_id.as_deref() == Some(run.id.as_str())
                });
                if !answered && app.running_thread(&run.thread_id) {
                    let stage = app.stage_label(&run.thread_id);
                    let deadline = app.deadline_label(&run.thread_id);
                    let title = if deadline.is_empty() {
                        format!("· {stage}")
                    } else {
                        format!("· {stage} · {deadline}")
                    };
                    blocks.push(ChatBlock {
                        key: format!("status:{}", run.id),
                        title,
                        body: String::new(),
                        collapsible: false,
                        message_index: None,
                        has_memory: false,
                    });
                    if let Some((title, body)) = app.live_bubble(&run.thread_id) {
                        blocks.push(ChatBlock {
                            key: format!("stream:{}", run.id),
                            title,
                            body,
                            collapsible: false,
                            message_index: None,
                            has_memory: false,
                        });
                    }
                }
            }
        } else {
            let memories = app
                .answer_memories
                .get(&message.id)
                .map(|items| !items.is_empty())
                .unwrap_or(false);
            blocks.push(ChatBlock {
                key: format!("assistant:{}", message.id),
                title: "Recon".into(),
                body: message.content.clone(),
                collapsible: false,
                message_index: Some(index),
                has_memory: memories,
            });
        }
    }
    for (call_index, call) in app.calls.iter().enumerate() {
        if used.insert(call_index) {
            blocks.push(tool_block(app, call));
        }
    }
    blocks
}

fn plan_block(run: &recon::Run, calls: &[recon::Call], open: bool) -> ChatBlock {
    let plan = run
        .plan_json
        .as_deref()
        .and_then(|raw| serde_json::from_str::<Plan>(raw).ok());
    let title = match &plan {
        Some(plan) if !plan.directives.is_empty() => {
            let transport = if plan.picker_transport.is_empty() {
                "picking"
            } else {
                plan.picker_transport.as_str()
            };
            format!(
                "Recon log · tool picker ({transport}) · {}",
                clip_chars(&plan.directives[0].goal, 64)
            )
        }
        Some(plan) => {
            let label = if plan.strategy.is_empty() {
                "Recon log"
            } else {
                strategy_label(&plan.strategy)
            };
            let rationale = clip_chars(
                if plan.strategy_rationale.is_empty() {
                    if plan.objective.is_empty() {
                        "Recon log"
                    } else {
                        plan.objective.as_str()
                    }
                } else {
                    plan.strategy_rationale.as_str()
                },
                72,
            );
            format!("Recon log · {label} · {rationale}")
        }
        None => "Recon log · waiting for a plan".into(),
    };
    let body = if !open {
        String::new()
    } else {
        match plan {
            Some(plan) if !plan.directives.is_empty() => {
                question_plan_lines(run, &plan, calls).join("\n")
            }
            Some(plan) => {
                let mut lines = Vec::new();
                if !plan.objective.is_empty() {
                    lines.push(format!("Objective: {}", plan.objective));
                }
                if !plan.strategy.is_empty() {
                    lines.push(format!(
                        "Strategy: {} — {}",
                        strategy_label(&plan.strategy),
                        plan.strategy_rationale
                    ));
                }
                if !plan.strategy_change.is_empty() {
                    lines.push(format!("Change: {}", plan.strategy_change));
                }
                if !plan.discovery_note.is_empty() {
                    lines.push(format!("Discovery: {}", plan.discovery_note));
                }
                if !plan.accounts.is_empty() {
                    lines.push(format!("Accounts found: {}", plan.accounts.join("; ")));
                } else if !plan.accounts_note.is_empty() {
                    lines.push("Accounts found: none".into());
                }
                if !plan.accounts_note.is_empty() {
                    lines.push(format!("   {}", plan.accounts_note));
                }
                if !plan.isolated_tools.is_empty() {
                    lines.push("Tool isolation:".into());
                    for tool in &plan.isolated_tools {
                        lines.push(format!("   {tool}"));
                    }
                }
                if plan.question_answered {
                    lines.push("Question answered. No further message.".into());
                } else if !plan.additional_tools.is_empty() {
                    lines.push("Additional context:".into());
                    for tool in &plan.additional_tools {
                        lines.push(format!("   {tool}"));
                    }
                }
                for hypothesis in &plan.hypotheses {
                    lines.push(format!(
                        "Hypothesis: {} ({})",
                        hypothesis.question, hypothesis.status
                    ));
                    for line in &hypothesis.lines {
                        lines.push(format!("   {line}"));
                    }
                }
                for entity in &plan.selected_entities {
                    lines.push(format!(
                        "Entity: {} ({}, {}) {}",
                        entity.name, entity.entity_type, entity.certainty, entity.identifiers
                    ));
                }
                if !plan.stop_condition.is_empty() {
                    lines.push(format!("Stop when: {}", plan.stop_condition));
                }
                for (index, call) in plan.calls.iter().enumerate() {
                    let mut detail = if call.reason.is_empty() {
                        call.step_id.clone()
                    } else {
                        call.reason.clone()
                    };
                    if call.credit_cost > 0 {
                        detail.push_str(&format!(" · {} credits", call.credit_cost));
                    }
                    lines.push(format!("{}. {} — {detail}", index + 1, call.tool_id));
                    if !call.expected.is_empty() {
                        lines.push(format!("   expected: {}", call.expected));
                    }
                    if !call.depends_on.is_empty() {
                        lines.push(format!("   depends on {}", call.depends_on.join(", ")));
                    }
                }
                if !plan.deferred.is_empty() {
                    lines.push(format!("Deferred: {}", plan.deferred.join("; ")));
                }
                if !plan.unresolved_inputs.is_empty() {
                    lines.push(format!("Unresolved: {}", plan.unresolved_inputs.join(", ")));
                }
                if let Some(error) = &run.error {
                    lines.push(format!("Run error: {error}"));
                }
                lines.join("\n")
            }
            None => run
                .error
                .clone()
                .unwrap_or_else(|| "The recon model has not recorded a plan yet.".into()),
        }
    };
    ChatBlock {
        key: format!("plan:{}", run.id),
        title,
        body,
        collapsible: true,
        message_index: None,
        has_memory: false,
    }
}

/// Decision row for a question-driven turn: the three derived questions, the tool
/// picker and its model snapshot, the ordered tools with dependencies and the questions
/// they serve, the inputs bound for each step, and any fallback requests. Pick
/// probabilities stay in `plan_json` (`recon show`); they are not rendered here.
fn question_plan_lines(run: &recon::Run, plan: &Plan, calls: &[recon::Call]) -> Vec<String> {
    let mut lines = Vec::new();
    lines.push(if matches!(plan.directives_mode.as_str(), "directives_fallback" | "questions_fallback") {
        "Directives (fallback set):".to_string()
    } else {
        "Directives:".to_string()
    });
    for directive in &plan.directives {
        let mut row = format!("   {}: {}", directive.id, directive.goal);
        if !directive.entities.is_empty() {
            row.push_str(&format!(" · entities {}", directive.entities.join(", ")));
        }
        if !directive.targets.is_empty() {
            row.push_str(&format!(" · targets {}", directive.targets.join(", ")));
        }
        lines.push(row);
    }
    if !plan.directives_note.is_empty() {
        lines.push(format!("   {}", plan.directives_note));
    }
    if !plan.deadline_note.is_empty() {
        lines.push(plan.deadline_note.clone());
    }
    let model = if run.tool_picker_model.is_empty() {
        plan.picker_model.as_str()
    } else {
        run.tool_picker_model.as_str()
    };
    let transport = if plan.picker_transport.is_empty() {
        "fallback"
    } else {
        plan.picker_transport.as_str()
    };
    let mut picker = format!("Tool picker: {transport}");
    if !model.is_empty() {
        picker.push_str(&format!(" · {model}"));
    }
    if plan.planning_mode == "tool_picker_fallback" {
        picker.push_str(" · deterministic order");
    }
    lines.push(picker);
    if !plan.picker_note.is_empty() {
        lines.push(format!("   {}", plan.picker_note));
    }
    if !plan.calls.is_empty() {
        lines.push("Order:".into());
    }
    for call in &plan.calls {
        let mut row = format!("{}. {}", call.step_id, call.tool_id);
        if !call.reason.is_empty() {
            row.push_str(&format!(" — {}", call.reason));
        }
        if !call.depends_on.is_empty() {
            row.push_str(&format!(" · after {}", call.depends_on.join(", ")));
        }
        if call.pick_reason.starts_with("fallback:") {
            row.push_str(" · fallback");
        }
        if !call.status.is_empty() {
            row.push_str(&format!(" · {}", call.status));
        }
        if call.credit_cost > 0 {
            row.push_str(&format!(" · {} credits", call.credit_cost));
        }
        lines.push(row);
        if let Some(found) = matching_call(calls, call) {
            if let Some(summary) = result_summary(found) {
                lines.push(format!("   result {summary}"));
            }
        }
        for input in &call.filled {
            lines.push(format!("   input {input}"));
        }
        for binding in plan.bindings.iter().filter(|b| b.step_id == call.step_id) {
            let qualifier = if binding.qualifier.is_empty() {
                String::new()
            } else {
                format!(" ({})", binding.qualifier)
            };
            let inferred = if binding.inferred { " · inferred" } else { "" };
            lines.push(format!(
                "   found {} {}{qualifier} · evidence {}{inferred}",
                binding.kind, binding.value, binding.evidence_id
            ));
        }
    }
    let explicit: Vec<_> = plan
        .bindings
        .iter()
        .filter(|binding| binding.step_id.is_empty() && !binding.inferred)
        .map(|binding| {
            let platform = if binding.qualifier.is_empty() { String::new() } else { format!(" ({})", binding.qualifier) };
            if binding.unverified {
                format!("{} {}{platform} · named in {}, unverified", binding.kind, binding.value, binding.evidence_id)
            } else {
                format!("{} {}{platform}", binding.kind, binding.value)
            }
        })
        .collect();
    if !explicit.is_empty() {
        lines.push(format!("From the question: {}", explicit.join("; ")));
    }
    if !plan.binding_notes.is_empty() {
        lines.push("Binding extraction:".into());
        for note in &plan.binding_notes {
            lines.push(format!("   {note}"));
        }
    }
    if !plan.fallback_requests.is_empty() {
        lines.push("Fallback requests:".into());
        for request in &plan.fallback_requests {
            lines.push(format!("   {request}"));
        }
    }
    if !plan.deferred.is_empty() {
        lines.push(format!("Deferred: {}", plan.deferred.join("; ")));
    }
    if !plan.unresolved_inputs.is_empty() {
        lines.push(format!("Unresolved: {}", plan.unresolved_inputs.join(", ")));
    }
    if let Some(error) = &run.error {
        lines.push(format!("Run error: {error}"));
    }
    lines
}

fn matching_call<'a>(calls: &'a [recon::Call], step: &recon::PlanCall) -> Option<&'a recon::Call> {
    if !step.call_id.is_empty() {
        if let Some(found) = calls.iter().find(|call| call.id == step.call_id) {
            return Some(found);
        }
    }
    calls.iter().find(|call| call.tool_id == step.tool_id && call.inputs == step.arguments)
}

fn result_count(observations: &serde_json::Value) -> Option<usize> {
    for key in ["results", "articles"] {
        if let Some(rows) = observations.get(key).and_then(serde_json::Value::as_array) {
            return Some(rows.len());
        }
    }
    None
}

/// Status, cache or live, result count, truncation, and a short error.
fn result_summary(call: &recon::Call) -> Option<String> {
    let result = call.result.as_ref()?;
    let mut parts = vec![result.status.clone()];
    parts.push(if result.cached { "cache".into() } else { "live".into() });
    if let Some(count) = result_count(&result.observations) {
        parts.push(format!(
            "{count} result{}",
            if count == 1 { "" } else { "s" }
        ));
    }
    if result.truncated {
        parts.push("truncated".into());
    }
    if let Some(error) = result.error.as_deref().filter(|text| !text.is_empty()) {
        parts.push(clip_chars(error, 80));
    }
    Some(parts.join(" · "))
}

pub struct ToolLog {
    pub level: &'static str,
    pub summary: String,
    pub detail: String,
}

/// One-line summary and the clipped result body for the System event log.
pub fn tool_result_log(call: &recon::Call) -> Option<ToolLog> {
    let result = call.result.as_ref()?;
    let name = osint::definition(&call.tool_id)
        .map(|tool| tool.name)
        .unwrap_or(call.tool_id.as_str());
    let summary = result_summary(call).unwrap_or_else(|| result.status.clone());
    let level = match result.status.as_str() {
        "failed" => "error",
        "timeout" | "rate_limited" => "warn",
        _ => "info",
    };
    let mut detail = vec![
        format!("Call {}", call.id),
        format!("Status: {} · attempts {}", call.status, call.attempts),
        format!("Result: {summary}"),
        format!(
            "Input: {}",
            serde_json::to_string(&call.inputs).unwrap_or_else(|_| "{}".into())
        ),
    ];
    if !result.source_url.is_empty() {
        detail.push(format!("Source: {}", result.source_url));
    }
    if !result.retrieved_at.is_empty() {
        detail.push(format!("Retrieved: {}", result.retrieved_at));
    }
    if let Some(error) = result.error.as_deref().filter(|text| !text.is_empty()) {
        detail.push(format!("Error: {error}"));
    }
    let observations = serde_json::to_string_pretty(&result.observations).unwrap_or_default();
    if !observations.is_empty() && observations != "null" {
        detail.push(clip_chars(&observations, 4_000));
    }
    Some(ToolLog {
        level,
        summary: format!("{name} {summary}"),
        detail: detail.join("\n"),
    })
}

fn tool_block(app: &App, call: &recon::Call) -> ChatBlock {
    let name = osint::definition(&call.tool_id)
        .map(|tool| tool.name)
        .unwrap_or(call.tool_id.as_str());
    let key = format!("tool:{}", call.id);
    let cache = call
        .result
        .as_ref()
        .map(|result| if result.cached { " · cache" } else { "" })
        .unwrap_or("");
    if !app.expanded.contains(&key) {
        return ChatBlock {
            key,
            title: format!("{name} · {}{cache}", call.status),
            body: String::new(),
            collapsible: true,
            message_index: None,
            has_memory: false,
        };
    }
    let mut lines = vec![
        format!("Call {}", call.id),
        format!("Status: {} · attempts {}", call.status, call.attempts),
    ];
    if let Some(reason) = plan_reason(app, call) {
        lines.insert(0, format!("Reason: {reason}"));
    }
    if let Some(summary) = result_summary(call) {
        lines.push(format!("Result: {summary}"));
    }
    lines.push("Full result is in the System event log.".into());
    ChatBlock {
        key,
        title: format!("{name} · {}", call.status),
        body: lines.join("\n"),
        collapsible: true,
        message_index: None,
        has_memory: false,
    }
}

fn strategy_label(kind: &str) -> &'static str {
    match kind {
        "hypothesis" => "Question and hypothesis testing",
        "adaptive" => "Adaptive expansion by information value",
        "discovery" => "Discovery and selective enrichment",
        _ => "Discovery and selective enrichment",
    }
}

fn plan_reason(app: &App, call: &recon::Call) -> Option<String> {
    let run = app
        .runs
        .iter()
        .find(|run| Some(&run.id) == call.run_id.as_ref())?;
    let plan = serde_json::from_str::<Plan>(run.plan_json.as_deref()?).ok()?;
    plan.calls
        .into_iter()
        .find(|step| step.tool_id == call.tool_id && step.arguments == call.inputs)
        .map(|step| step.reason)
        .filter(|reason| !reason.is_empty())
}

fn clip_chars(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        value.to_string()
    } else {
        let mut clipped: String = value.chars().take(max.saturating_sub(1)).collect();
        clipped.push('…');
        clipped
    }
}

fn fit(value: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    if value.chars().count() <= width {
        value.to_string()
    } else {
        let mut clipped: String = value.chars().take(width.saturating_sub(1)).collect();
        clipped.push('…');
        clipped
    }
}

fn clip_pieces(pieces: &mut Vec<Piece>, width: usize) {
    let width = width.max(1);
    let mut used = 0usize;
    let mut end = pieces.len();
    for (index, piece) in pieces.iter_mut().enumerate() {
        let count = piece.text.chars().count();
        if used >= width {
            end = index;
            break;
        }
        if used + count > width {
            piece.text = fit(&piece.text, width - used);
            end = index + 1;
            break;
        }
        used += count;
    }
    pieces.truncate(end);
}

fn disclosure_pieces(
    open: bool,
    label: &str,
    suffix: Option<(String, Tone)>,
    width: usize,
) -> Vec<Piece> {
    let marker = if open { "▾ " } else { "▸ " };
    let marker_width = marker.chars().count();
    let suffix = suffix.map(|(text, tone)| {
        let budget = width.saturating_sub(marker_width + 4).max(1);
        (fit(&text, budget), tone)
    });
    let suffix_width = suffix
        .as_ref()
        .map(|(text, _)| text.chars().count())
        .unwrap_or(0);
    let label_room = width.saturating_sub(marker_width + suffix_width).max(1);
    let mut pieces = vec![
        Piece {
            text: marker.into(),
            tone: Tone::Dim,
        },
        Piece {
            text: fit(label, label_room),
            tone: Tone::Dim,
        },
    ];
    if let Some((text, tone)) = suffix {
        if !text.is_empty() {
            pieces.push(Piece { text, tone });
        }
    }
    clip_pieces(&mut pieces, width);
    pieces
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RowFace {
    Plain,
    User,
    Code,
}

#[derive(Clone)]
struct ChatRow {
    block: usize,
    header: bool,
    brain: bool,
    collapsible: bool,
    face: RowFace,
    pieces: Vec<Piece>,
}

#[derive(Default, PartialEq, Eq)]
struct FrameStamp {
    width: u16,
    screen_w: u16,
    screen_h: u16,
    recon: bool,
    chat: bool,
    thread: Option<String>,
    stage: String,
    live_shown: usize,
    live_note: String,
    deadline: String,
    messages: u64,
    calls: u64,
    runs: u64,
    expanded: Vec<String>,
}

#[derive(Default)]
pub struct FrameCache {
    stamp: FrameStamp,
    blocks: Vec<ChatBlock>,
    rows: Vec<ChatRow>,
}

fn frame_stamp(app: &App, width: u16) -> FrameStamp {
    let mut expanded: Vec<String> = app.expanded.iter().cloned().collect();
    expanded.sort();
    FrameStamp {
        width,
        screen_w: app.screen.width,
        screen_h: app.screen.height,
        recon: app.module == Some(ModuleId::Recon),
        chat: app.recon_chat,
        thread: app.selected_thread.clone(),
        stage: app.recon_stage.clone(),
        live_shown: app
            .selected_thread
            .as_ref()
            .and_then(|id| app.live_bubble(id))
            .map(|(_, body)| body.len())
            .unwrap_or(0),
        live_note: app
            .selected_thread
            .as_ref()
            .and_then(|id| app.live_bubble(id))
            .map(|(title, _)| title)
            .unwrap_or_default(),
        deadline: app
            .selected_thread
            .as_ref()
            .map(|id| app.deadline_label(id))
            .unwrap_or_default(),
        messages: message_stamp(&app.messages),
        calls: call_stamp(&app.calls),
        runs: run_stamp(&app.runs),
        expanded,
    }
}

fn mix(acc: u64, value: u64) -> u64 {
    acc.wrapping_mul(0x9E37_79B1_85EB_CA87).wrapping_add(value)
}

fn message_stamp(messages: &[recon::Message]) -> u64 {
    let mut acc = messages.len() as u64;
    for message in messages {
        acc = mix(acc, message.content.len() as u64);
        acc = mix(acc, message.role.len() as u64);
        acc = mix(
            acc,
            message
                .run_id
                .as_ref()
                .map(|id| id.len() as u64)
                .unwrap_or(0),
        );
    }
    acc
}

fn call_stamp(calls: &[recon::Call]) -> u64 {
    let mut acc = calls.len() as u64;
    for call in calls {
        acc = mix(acc, call.status.len() as u64);
        acc = mix(acc, call.attempts as u64);
        let result = call
            .result
            .as_ref()
            .map(|result| result.raw.len() as u64 + result.status.len() as u64)
            .unwrap_or(0);
        acc = mix(acc, result);
    }
    acc
}

fn run_stamp(runs: &[recon::Run]) -> u64 {
    let mut acc = runs.len() as u64;
    for run in runs {
        acc = mix(acc, run.stage.len() as u64);
        acc = mix(
            acc,
            run.plan_json
                .as_ref()
                .map(|plan| plan.len() as u64)
                .unwrap_or(0),
        );
        acc = mix(acc, run.state.len() as u64);
    }
    acc
}

fn ensure_frame(app: &App) {
    let width = inset(transcript_rect(app)).width;
    let stamp = frame_stamp(app, width);
    if app.frame.borrow().stamp == stamp {
        return;
    }
    let blocks = if stamp.recon {
        build_blocks(app)
    } else {
        Vec::new()
    };
    let rows = rows_for(app, &blocks, width as usize);
    *app.frame.borrow_mut() = FrameCache {
        stamp,
        blocks,
        rows,
    };
}

fn expanded(app: &App, block: &ChatBlock) -> bool {
    !block.collapsible || app.expanded.contains(&block.key)
}

fn row(
    block: usize,
    header: bool,
    brain: bool,
    collapsible: bool,
    face: RowFace,
    pieces: Vec<Piece>,
) -> ChatRow {
    ChatRow {
        block,
        header,
        brain,
        collapsible,
        face,
        pieces,
    }
}

fn rows_for(app: &App, blocks: &[ChatBlock], width: usize) -> Vec<ChatRow> {
    let width = width.max(1);
    let mut rows = Vec::new();
    for (index, block) in blocks.iter().enumerate() {
        if block.key.starts_with("user:") {
            if index > 0 {
                rows.push(row(index, false, false, false, RowFace::Plain, Vec::new()));
            }
            for (line_index, pieces) in markdown::user_lines(&block.body, width)
                .into_iter()
                .enumerate()
            {
                rows.push(row(
                    index,
                    line_index == 0,
                    false,
                    false,
                    RowFace::User,
                    pieces,
                ));
            }
            continue;
        }
        if block.key.starts_with("assistant:") {
            if index > 0 {
                rows.push(row(index, false, false, false, RowFace::Plain, Vec::new()));
            }
            let lines = markdown::markdown_lines(&block.body, width);
            for (line_index, line) in lines.into_iter().enumerate() {
                let face = if line.code {
                    RowFace::Code
                } else {
                    RowFace::Plain
                };
                rows.push(row(index, line_index == 0, false, false, face, line.pieces));
            }
            if block.has_memory {
                let badge = "◉ brain";
                let pad = width.saturating_sub(badge.chars().count());
                rows.push(row(
                    index,
                    false,
                    true,
                    false,
                    RowFace::Plain,
                    vec![
                        Piece {
                            text: " ".repeat(pad),
                            tone: Tone::Dim,
                        },
                        Piece {
                            text: badge.into(),
                            tone: Tone::Accent,
                        },
                    ],
                ));
            }
            continue;
        }
        if block.key.starts_with("status:") {
            let mut pieces = vec![Piece {
                text: block.title.clone(),
                tone: Tone::Dim,
            }];
            clip_pieces(&mut pieces, width);
            rows.push(row(index, true, false, false, RowFace::Plain, pieces));
            continue;
        }
        if block.key.starts_with("stream:") {
            if index > 0 {
                rows.push(row(index, false, false, false, RowFace::Plain, Vec::new()));
            }
            let mut pieces = vec![Piece {
                text: block.title.clone(),
                tone: Tone::Accent,
            }];
            clip_pieces(&mut pieces, width);
            rows.push(row(index, true, false, false, RowFace::Plain, pieces));
            for line in markdown::markdown_lines(&block.body, width) {
                let face = if line.code { RowFace::Code } else { RowFace::Plain };
                rows.push(row(index, false, false, false, face, line.pieces));
            }
            continue;
        }
        let open = expanded(app, block);
        let (label, status) = block
            .title
            .split_once(" · ")
            .map(|(label, status)| (label.to_string(), Some(status.to_string())))
            .unwrap_or_else(|| (block.title.clone(), None));
        let suffix = status.and_then(|status| {
            let tone = if status.contains("fail") || status.contains("error") {
                Tone::Error
            } else if status == "completed" || status == "no_results" {
                Tone::Dim
            } else {
                Tone::Warn
            };
            if tone == Tone::Dim && !block.key.starts_with("plan:") {
                None
            } else {
                Some((format!(" · {status}"), tone))
            }
        });
        rows.push(row(
            index,
            true,
            false,
            block.collapsible,
            RowFace::Plain,
            disclosure_pieces(open, &label, suffix, width),
        ));
        if open && !block.body.is_empty() {
            for pieces in markdown::plain_lines(&block.body, width, 2) {
                rows.push(row(index, false, false, false, RowFace::Plain, pieces));
            }
        }
    }
    rows
}

fn transcript_rect(app: &App) -> Rect {
    let body = chrome(app.screen, app).body;
    if app.module != Some(ModuleId::Recon) || !app.recon_chat {
        return Rect::default();
    }
    chat_areas(body).0
}

fn chat_view(app: &App) -> (Rect, u16, Vec<ChatRow>) {
    ensure_frame(app);
    let inner = inset(transcript_rect(app));
    let rows = app.frame.borrow().rows.clone();
    let max = rows.len().saturating_sub(inner.height as usize) as u16;
    let scroll = if app.chat_follow {
        max
    } else {
        app.scrolls.chat.min(max)
    };
    (inner, scroll, rows)
}

fn chat_max(app: &App) -> u16 {
    let (inner, _, rows) = chat_view(app);
    rows.len().saturating_sub(inner.height as usize) as u16
}

struct Spot {
    rect: Rect,
    target: Target,
}

fn chat_spots(app: &App) -> Vec<Spot> {
    let (inner, scroll, rows) = chat_view(app);
    if inner.height == 0 || inner.width == 0 {
        return Vec::new();
    }
    let mut spots = Vec::new();
    for (offset, (_index, row)) in rows.iter().enumerate().skip(scroll as usize).enumerate() {
        if offset as u16 >= inner.height {
            break;
        }
        let rect = Rect {
            x: inner.x,
            y: inner.y + offset as u16,
            width: inner.width,
            height: 1,
        };
        if row.brain {
            let badge = 8.min(rect.width);
            spots.push(Spot {
                rect: Rect {
                    x: rect.x + rect.width.saturating_sub(badge),
                    y: rect.y,
                    width: badge,
                    height: 1,
                },
                target: Target::BrainMark(row.block),
            });
            spots.push(Spot {
                rect: Rect {
                    x: rect.x,
                    y: rect.y,
                    width: rect.width.saturating_sub(badge),
                    height: 1,
                },
                target: Target::ChatHeader(row.block),
            });
        } else if row.header {
            spots.push(Spot {
                rect,
                target: Target::ChatHeader(row.block),
            });
        } else {
            spots.push(Spot {
                rect,
                target: Target::ChatBody(row.block),
            });
        }
    }
    spots
}

pub fn normalize(app: &mut App) {
    let count = chat_blocks(app).len();
    if count == 0 {
        app.chat_sel = 0;
    } else if app.chat_follow || app.chat_sel >= count {
        app.chat_sel = count - 1;
    }
    let max = chat_max(app);
    if app.chat_follow || app.scrolls.chat > max {
        app.scrolls.chat = max;
    }
}

pub fn move_chat(app: &mut App, delta: i32) {
    let count = chat_blocks(app).len();
    if count == 0 {
        return;
    }
    app.chat_follow = false;
    let current = app.chat_sel.min(count - 1) as i32;
    app.chat_sel = (current + delta).clamp(0, count as i32 - 1) as usize;
    reveal_chat(app);
    if app.chat_sel + 1 == count && app.scrolls.chat >= chat_max(app) {
        app.chat_follow = true;
    }
}

fn reveal_chat(app: &mut App) {
    let (inner, _, rows) = chat_view(app);
    let Some(line) = rows
        .iter()
        .position(|row| row.block == app.chat_sel && row.header)
    else {
        return;
    };
    let room = inner.height as usize;
    if room == 0 {
        return;
    }
    let start = app.scrolls.chat as usize;
    if line < start {
        app.scrolls.chat = line as u16;
    } else if line >= start + room {
        app.scrolls.chat = (line + 1 - room) as u16;
    }
}

pub fn fold_chat(app: &mut App, expand: bool) {
    let Some(block) = chat_blocks(app).into_iter().nth(app.chat_sel) else {
        return;
    };
    if !block.collapsible {
        return;
    }
    if expand {
        app.expanded.insert(block.key);
    } else {
        app.expanded.remove(&block.key);
    }
}

pub fn toggle_chat(app: &mut App) {
    let Some(block) = chat_blocks(app).into_iter().nth(app.chat_sel) else {
        return;
    };
    if !block.collapsible {
        return;
    }
    if app.expanded.contains(&block.key) {
        app.expanded.remove(&block.key);
    } else {
        app.expanded.insert(block.key);
    }
}

pub fn open_block(app: &mut App, index: usize) {
    let Some(block) = chat_blocks(app).into_iter().nth(index) else {
        return;
    };
    let title = block.title.clone();
    let body = if block.body.is_empty() {
        title.clone()
    } else {
        block.body
    };
    app.scrolls.popup = 0;
    app.overlay = Overlay::Block { title, body };
}

pub fn open_memory(app: &mut App, block_index: usize) {
    let Some(message_index) = chat_blocks(app)
        .into_iter()
        .nth(block_index)
        .and_then(|block| block.message_index)
    else {
        return;
    };
    let Some(message) = app.messages.get(message_index) else {
        return;
    };
    app.scrolls.popup = 0;
    app.overlay = Overlay::Memories {
        message_id: message.id.clone(),
    };
}

pub fn scroll_at(app: &mut App, x: u16, y: u16, delta: i32) {
    if let Overlay::Choice(_) = app.overlay {
        app.move_choice(delta);
        return;
    }
    if app.overlay != Overlay::None {
        let max = popup_max(app);
        nudge(&mut app.scrolls.popup, delta * 3, max);
        return;
    }
    match region_at(app, x, y) {
        Region::Chat => scroll_chat(app, delta * 3),
        Region::Threads => {
            let max = thread_max(app);
            nudge_list(&mut app.scrolls.threads, delta, max);
        }
        Region::Memories => {
            let max = memory_max(app);
            nudge_list(&mut app.scrolls.memories, delta, max);
        }
        Region::Tools => {
            let max = tool_max(app);
            nudge_list(&mut app.scrolls.tools, delta, max);
        }
        Region::Detail => nudge(&mut app.scrolls.detail, delta * 3, 10_000),
        Region::Recall => nudge(&mut app.scrolls.recall, delta * 3, 10_000),
        Region::Log => {
            let max = log_max(app);
            nudge(&mut app.scrolls.log, delta * 3, max);
        }
        Region::None => {}
    }
}

pub fn page(app: &mut App, direction: i32) {
    if let Overlay::Choice(_) = app.overlay {
        let room = choice_list_room(app).max(1) as i32;
        app.move_choice(direction * room);
        return;
    }
    if app.overlay != Overlay::None {
        let room = inset(popup_area(app.screen)).height.max(1) as i32;
        let max = popup_max(app);
        nudge(&mut app.scrolls.popup, direction * room, max);
        return;
    }
    match app.module {
        Some(ModuleId::Recon) if !app.recon_chat => {
            let room = (thread_room(app) / 2).max(1) as i32;
            let max = thread_max(app);
            nudge_list(&mut app.scrolls.threads, direction * room, max);
        }
        Some(ModuleId::Recon) => {
            let room = inset(transcript_rect(app)).height.max(1) as i32;
            scroll_chat(app, direction * room);
        }
        Some(ModuleId::Brain) if matches!(app.focus, Target::Memory(_)) => {
            let room = (memory_room(app) / 2).max(1) as i32;
            let max = memory_max(app);
            nudge_list(&mut app.scrolls.memories, direction * room, max);
        }
        Some(ModuleId::Brain) => nudge(&mut app.scrolls.recall, direction * 6, 10_000),
        Some(ModuleId::Osint) if matches!(app.focus, Target::Tool(_)) => {
            let room = (tool_room(app) / 2).max(1) as i32;
            let max = tool_max(app);
            nudge_list(&mut app.scrolls.tools, direction * room, max);
        }
        Some(ModuleId::Osint) | Some(ModuleId::Providers) => {
            nudge(&mut app.scrolls.detail, direction * 6, 10_000);
        }
        Some(ModuleId::System) => {
            let room = inset(system_areas(chrome(app.screen, app).body).2)
                .height
                .max(1) as i32;
            move_system_log(app, direction * room);
        }
        None => {}
    }
}

fn scroll_chat(app: &mut App, delta: i32) {
    let max = chat_max(app);
    let current = if app.chat_follow {
        max
    } else {
        app.scrolls.chat.min(max)
    };
    let next = (i32::from(current) + delta).clamp(0, i32::from(max)) as u16;
    app.scrolls.chat = next;
    app.chat_follow = next == max;
}

fn nudge(scroll: &mut u16, delta: i32, max: u16) {
    *scroll = (i32::from(*scroll) + delta).clamp(0, i32::from(max)) as u16;
}

fn nudge_list(scroll: &mut u16, delta: i32, max: u16) {
    nudge(scroll, delta, max);
}

enum Region {
    Chat,
    Threads,
    Memories,
    Tools,
    Detail,
    Recall,
    Log,
    None,
}

fn region_at(app: &App, x: u16, y: u16) -> Region {
    let body = chrome(app.screen, app).body;
    match app.module {
        Some(ModuleId::Recon) if app.recon_chat => {
            let (transcript, _) = chat_areas(body);
            if contains(transcript, x, y) {
                Region::Chat
            } else {
                Region::None
            }
        }
        Some(ModuleId::Recon) => {
            let (_, list, _) = dashboard_areas(body);
            if contains(list, x, y) {
                Region::Threads
            } else {
                Region::None
            }
        }
        Some(ModuleId::Brain) => {
            let rows = brain_areas(body);
            if contains(rows[7], x, y) {
                Region::Memories
            } else if contains(rows[8], x, y) {
                Region::Recall
            } else {
                Region::None
            }
        }
        Some(ModuleId::Osint) => {
            let layout = osint_areas(body, api_key_slot(app).is_some());
            let list = layout.list;
            let detail = layout.detail;
            if contains(list, x, y) {
                Region::Tools
            } else if contains(detail, x, y) {
                Region::Detail
            } else {
                Region::None
            }
        }
        Some(ModuleId::Providers) => {
            let rows = provider_areas(body);
            if contains(rows[1], x, y) {
                Region::Detail
            } else {
                Region::None
            }
        }
        Some(ModuleId::System) => {
            let (_, _, log) = system_areas(body);
            if contains(log, x, y) {
                Region::Log
            } else {
                Region::None
            }
        }
        None => Region::None,
    }
}

fn thread_room(app: &App) -> usize {
    if app.recon_chat {
        return 1;
    }
    dashboard_areas(chrome(app.screen, app).body)
        .1
        .height
        .saturating_sub(2) as usize
}

fn memory_room(app: &App) -> usize {
    brain_areas(chrome(app.screen, app).body)[7]
        .height
        .saturating_sub(2) as usize
        / 2
}

fn tool_room(app: &App) -> usize {
    osint_areas(chrome(app.screen, app).body, api_key_slot(app).is_some())
        .list
        .height
        .saturating_sub(2) as usize
}

fn thread_max(app: &App) -> u16 {
    app.threads.len().saturating_sub(thread_room(app).max(1)) as u16
}

fn memory_max(app: &App) -> u16 {
    app.memories.len().saturating_sub(memory_room(app).max(1)) as u16
}

fn tool_max(app: &App) -> u16 {
    visible_tools(app)
        .len()
        .saturating_sub(tool_room(app).max(1)) as u16
}

fn log_max(app: &App) -> u16 {
    let room = inset(system_areas(chrome(app.screen, app).body).2).height;
    log_line_count(app).saturating_sub(room as usize) as u16
}

fn log_line_count(app: &App) -> usize {
    app.log
        .iter()
        .map(|entry| {
            let extra = if app.log_open.contains(&entry.id) && !entry.detail.is_empty() {
                entry.detail.lines().count()
            } else {
                0
            };
            1 + extra
        })
        .sum()
}

fn log_entry_start(app: &App, index: usize) -> usize {
    app.log
        .iter()
        .take(index)
        .map(|entry| {
            let extra = if app.log_open.contains(&entry.id) && !entry.detail.is_empty() {
                entry.detail.lines().count()
            } else {
                0
            };
            1 + extra
        })
        .sum()
}

pub fn move_system_log(app: &mut App, delta: i32) {
    if app.log.is_empty() {
        return;
    }
    app.log_browsing = true;
    let last = app.log.len() as i32 - 1;
    app.log_sel = (app.log_sel as i32 + delta).clamp(0, last) as usize;
    reveal_log(app);
}

pub fn reveal_log(app: &mut App) {
    if app.log.is_empty() {
        app.scrolls.log = 0;
        return;
    }
    if app.log_sel >= app.log.len() {
        app.log_sel = app.log.len() - 1;
    }
    let room = inset(system_areas(chrome(app.screen, app).body).2)
        .height
        .max(1) as usize;
    let start = log_entry_start(app, app.log_sel);
    reveal_index(&mut app.scrolls.log, start, room);
    let max = log_max(app);
    if app.scrolls.log > max {
        app.scrolls.log = max;
    }
}

fn popup_max(app: &App) -> u16 {
    let room = inset(popup_area(app.screen)).height.max(1) as usize;
    popup_text(app).lines().count().saturating_sub(room) as u16
}

pub fn reveal_index(scroll: &mut u16, index: usize, room: usize) {
    if room == 0 {
        return;
    }
    let start = *scroll as usize;
    if index < start {
        *scroll = index as u16;
    } else if index >= start + room {
        *scroll = (index + 1 - room) as u16;
    }
}

pub fn thread_room_for(app: &App) -> usize {
    thread_room(app)
}

pub fn memory_room_for(app: &App) -> usize {
    memory_room(app)
}

pub fn tool_room_for(app: &App) -> usize {
    tool_room(app)
}

pub fn focus_order(app: &App) -> Vec<Target> {
    if app.overlay != Overlay::None {
        return vec![Target::CloseOverlay];
    }
    match app.module {
        None => (0..ModuleId::ALL.len()).map(Target::App).collect(),
        Some(ModuleId::Recon) if app.recon_chat => {
            let mut order = vec![Target::Home, Target::Transcript];
            order.extend(
                [
                    ButtonId::CancelRun,
                    ButtonId::ResumeRun,
                    ButtonId::RetryInsights,
                ]
                .map(Target::Button),
            );
            order.push(Target::Field(FieldId::Composer));
            order.push(Target::Button(ButtonId::Send));
            order
        }
        Some(ModuleId::Recon) => {
            let mut order = vec![
                Target::Home,
                Target::Field(FieldId::ReconSearch),
                Target::Button(ButtonId::NewThread),
                Target::Button(ButtonId::DeleteThread),
            ];
            if !app.threads.is_empty() {
                order.push(Target::Thread(app.thread_sel));
            }
            order
        }
        Some(ModuleId::Brain) => {
            let mut order = vec![Target::Home];
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
                    ButtonId::OpenSource,
                ]
                .map(Target::Button),
            );
            if !app.memories.is_empty() {
                order.push(Target::Memory(app.memory_sel));
            }
            order
        }
        Some(ModuleId::Osint) => {
            let mut order = vec![Target::Home, Target::Field(FieldId::OsintSearch)];
            if !visible_tools(app).is_empty() {
                order.push(Target::Tool(app.tool_sel));
            }
            if let Some(slot) = api_key_slot(app) {
                order.push(Target::Field(slot.field));
                order.push(Target::Button(slot.button));
            }
            order.push(Target::Field(FieldId::OsintInput));
            order.extend(
                [
                    ButtonId::OsintRun,
                    ButtonId::OsintCancel,
                    ButtonId::OsintToggle,
                    ButtonId::OsintRaw,
                    ButtonId::OsintAttach,
                    ButtonId::OsintStartRecon,
                    ButtonId::OsintPrev,
                    ButtonId::OsintNext,
                ]
                .map(Target::Button),
            );
            order
        }
        Some(ModuleId::Providers) => {
            let mut order = vec![Target::Home];
            order.extend(ProviderPage::ALL.map(Target::ProviderTab));
            match app.provider_page {
                ProviderPage::Grok => {
                    order.extend([ButtonId::GrokSignIn, ButtonId::GrokCheck].map(Target::Button));
                }
                ProviderPage::OpenAI => {
                    order.extend(
                        [ButtonId::OpenAISignIn, ButtonId::OpenAICheck].map(Target::Button),
                    );
                }
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
                ProviderPage::Defaults => {
                    order.extend(
                        DefaultsRole::ALL.map(|role| Target::Button(ButtonId::DefaultRole(role))),
                    );
                    let role = app.defaults_role;
                    order.extend([
                        Target::Field(role.provider_field()),
                        Target::Field(role.model_field()),
                        Target::Button(role.save_button()),
                    ]);
                    order.push(Target::Button(ButtonId::RefreshModels));
                }
            }
            order
        }
        Some(ModuleId::System) => vec![
            Target::Home,
            Target::Button(ButtonId::RefreshHardware),
            Target::Button(ButtonId::ClearLog),
        ],
    }
}

pub fn choice_list_room(app: &App) -> usize {
    let inner = inset(popup_area(app.screen));
    let note = usize::from(!app.choice_note.is_empty());
    inner.height.saturating_sub(note as u16) as usize
}

pub fn choice_hits(app: &App) -> Vec<(usize, Rect)> {
    let inner = inset(popup_area(app.screen));
    if inner.width == 0 || inner.height == 0 {
        return Vec::new();
    }
    let mut y = inner.y;
    let mut height = inner.height;
    if !app.choice_note.is_empty() {
        y = y.saturating_add(1);
        height = height.saturating_sub(1);
    }
    let start = app.scrolls.popup as usize;
    app.choice_items
        .iter()
        .enumerate()
        .skip(start)
        .take(height as usize)
        .map(|(index, _)| {
            (
                index,
                Rect {
                    x: inner.x,
                    y: y + (index - start) as u16,
                    width: inner.width,
                    height: 1,
                },
            )
        })
        .collect()
}

pub fn hit_test(app: &App, x: u16, y: u16) -> Option<Target> {
    if app.overlay != Overlay::None {
        let popup = popup_area(app.screen);
        let close = Rect {
            x: popup.x + popup.width.saturating_sub(8),
            y: popup.y,
            width: 8.min(popup.width),
            height: 1,
        };
        if contains(close, x, y) || !contains(popup, x, y) {
            return Some(Target::CloseOverlay);
        }
        if let Overlay::Choice(_) = app.overlay {
            return choice_hits(app)
                .into_iter()
                .find(|(_, rect)| contains(*rect, x, y))
                .map(|(index, _)| Target::Choice(index));
        }
        return None;
    }
    let layout = chrome(app.screen, app);
    if app.module.is_none() {
        return home_line(app, x, y).map(Target::App);
    }
    if contains(layout.home, x, y) {
        return Some(Target::Home);
    }
    if composer_height(app) > 0 && contains(layout.composer, x, y) {
        let (_field, send) = composer_parts(layout.composer);
        return Some(if contains(send, x, y) {
            Target::Button(ButtonId::Send)
        } else {
            Target::Field(FieldId::Composer)
        });
    }
    if !contains(layout.body, x, y) {
        return None;
    }
    match app.module {
        Some(ModuleId::Recon) => recon_hit(app, layout.body, x, y),
        Some(ModuleId::Brain) => brain_hit(app, layout.body, x, y),
        Some(ModuleId::Osint) => osint_hit(app, layout.body, x, y),
        Some(ModuleId::Providers) => provider_hit(app, layout.body, x, y),
        Some(ModuleId::System) => system_hit(app, layout.body, x, y),
        None => None,
    }
}

fn recon_hit(app: &App, body: Rect, x: u16, y: u16) -> Option<Target> {
    if app.recon_chat {
        let (transcript, run_actions) = chat_areas(body);
        if contains(run_actions, x, y) {
            let areas = button_areas(run_actions, 3);
            return Some(Target::Button(if contains(areas[0], x, y) {
                ButtonId::CancelRun
            } else if contains(areas[1], x, y) {
                ButtonId::ResumeRun
            } else {
                ButtonId::RetryInsights
            }));
        }
        if contains(transcript, x, y) {
            return chat_spots(app)
                .into_iter()
                .find(|spot| contains(spot.rect, x, y))
                .map(|spot| spot.target);
        }
        return None;
    }
    let (search, list, actions) = dashboard_areas(body);
    if contains(search, x, y) {
        return Some(Target::Field(FieldId::ReconSearch));
    }
    if contains(list, x, y) && y > list.y && y + 1 < list.y + list.height {
        let index = app.scrolls.threads as usize + (y - list.y - 1) as usize;
        if index < app.threads.len() {
            return Some(Target::Thread(index));
        }
    }
    if contains(actions, x, y) {
        let areas = button_areas(actions, 2);
        return Some(Target::Button(if contains(areas[0], x, y) {
            ButtonId::NewThread
        } else {
            ButtonId::DeleteThread
        }));
    }
    None
}

fn brain_hit(app: &App, body: Rect, x: u16, y: u16) -> Option<Target> {
    let rows = brain_areas(body);
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
        let buttons = button_areas(rows[6], 3);
        let ids = [ButtonId::Pin, ButtonId::Delete, ButtonId::OpenSource];
        return buttons
            .iter()
            .position(|rect| contains(*rect, x, y))
            .map(|index| Target::Button(ids[index]));
    }
    if contains(rows[7], x, y) && y > rows[7].y && y + 1 < rows[7].y + rows[7].height {
        let index = app.scrolls.memories as usize + (y - rows[7].y - 1) as usize / 2;
        if index < app.memories.len() {
            return Some(Target::Memory(index));
        }
    }
    None
}

fn osint_hit(app: &App, body: Rect, x: u16, y: u16) -> Option<Target> {
    let slot = api_key_slot(app);
    let layout = osint_areas(body, slot.is_some());
    let search = layout.search;
    let list = layout.list;
    let input = layout.input;
    let actions = layout.actions;
    if contains(search, x, y) {
        return Some(Target::Field(FieldId::OsintSearch));
    }
    if let Some(slot) = slot {
        if contains(layout.key, x, y) {
            let parts = split_horizontal(layout.key, [Constraint::Min(8), Constraint::Length(16)]);
            return Some(if contains(parts[1], x, y) {
                Target::Button(slot.button)
            } else {
                Target::Field(slot.field)
            });
        }
    }
    if contains(list, x, y) && y > list.y && y + 1 < list.y + list.height {
        let tools = visible_tools(app);
        let index = app.scrolls.tools as usize + (y - list.y - 1) as usize;
        if let Some((id, _)) = tools.get(index) {
            return Some(Target::Tool(*id));
        }
    }
    if contains(input, x, y) {
        return Some(Target::Field(FieldId::OsintInput));
    }
    if contains(actions, x, y) {
        let areas = button_areas(actions, 8);
        let ids = [
            ButtonId::OsintRun,
            ButtonId::OsintCancel,
            ButtonId::OsintToggle,
            ButtonId::OsintRaw,
            ButtonId::OsintAttach,
            ButtonId::OsintStartRecon,
            ButtonId::OsintPrev,
            ButtonId::OsintNext,
        ];
        return areas
            .iter()
            .position(|rect| contains(*rect, x, y))
            .map(|index| Target::Button(ids[index]));
    }
    None
}

fn provider_hit(app: &App, body: Rect, x: u16, y: u16) -> Option<Target> {
    let rows = provider_areas(body);
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
        ProviderPage::Defaults => {
            let models = model_areas(rows[1]);
            for (role, area) in DefaultsRole::ALL
                .into_iter()
                .zip(button_areas(models[0], DefaultsRole::ALL.len()))
            {
                if contains(area, x, y) {
                    return Some(Target::Button(ButtonId::DefaultRole(role)));
                }
            }
            let role = app.defaults_role;
            if contains(models[1], x, y) {
                return Some(Target::Field(role.provider_field()));
            }
            if contains(models[2], x, y) {
                return Some(Target::Field(role.model_field()));
            }
            if contains(models[3], x, y) {
                return Some(Target::Button(role.save_button()));
            }
            if contains(models[4], x, y) {
                return Some(Target::Button(ButtonId::RefreshModels));
            }
        }
    }
    None
}

fn system_hit(app: &App, body: Rect, x: u16, y: u16) -> Option<Target> {
    let (_, actions, _) = system_areas(body);
    if contains(actions, x, y) {
        let areas = button_areas(actions, 2);
        return Some(Target::Button(if contains(areas[0], x, y) {
            ButtonId::RefreshHardware
        } else {
            ButtonId::ClearLog
        }));
    }
    let _ = app;
    None
}

fn field_rect(app: &App, field: FieldId) -> Option<Rect> {
    let layout = chrome(app.screen, app);
    match field {
        FieldId::Composer if composer_height(app) > 0 => Some(composer_parts(layout.composer).0),
        FieldId::ReconSearch if app.module == Some(ModuleId::Recon) && !app.recon_chat => {
            Some(dashboard_areas(layout.body).0)
        }
        FieldId::OsintSearch if app.module == Some(ModuleId::Osint) => {
            Some(osint_areas(layout.body, api_key_slot(app).is_some()).search)
        }
        FieldId::OsintInput if app.module == Some(ModuleId::Osint) => {
            Some(osint_areas(layout.body, api_key_slot(app).is_some()).input)
        }
        FieldId::FirecrawlKey
        | FieldId::HunterKey
        | FieldId::SociaVaultKey
        | FieldId::NewsApiKey
        | FieldId::CourtListenerKey
            if app.module == Some(ModuleId::Osint)
                && api_key_slot(app).is_some_and(|slot| slot.field == field) =>
        {
            let key = osint_areas(layout.body, true).key;
            Some(split_horizontal(key, [Constraint::Min(8), Constraint::Length(16)])[0])
        }
        FieldId::BrainApp
        | FieldId::BrainConversation
        | FieldId::BrainInsight
        | FieldId::BrainQuery
            if app.module == Some(ModuleId::Brain) =>
        {
            let rows = brain_areas(layout.body);
            match field {
                FieldId::BrainApp => Some(rows[1]),
                FieldId::BrainConversation => Some(rows[2]),
                FieldId::BrainInsight => Some(rows[3]),
                _ => Some(rows[4]),
            }
        }
        FieldId::ReconProvider
        | FieldId::ReconModel
        | FieldId::PickerProvider
        | FieldId::PickerModel
        | FieldId::SynthesisProvider
        | FieldId::SynthesisModel
            if app.module == Some(ModuleId::Providers)
                && app.provider_page == ProviderPage::Defaults =>
        {
            let rows = model_areas(provider_areas(layout.body)[1]);
            Some(match field {
                FieldId::ReconProvider | FieldId::PickerProvider | FieldId::SynthesisProvider => {
                    rows[1]
                }
                _ => rows[2],
            })
        }
        FieldId::RouterKey | FieldId::RouterEndpoint
            if app.module == Some(ModuleId::Providers)
                && app.provider_page == ProviderPage::OpenRouter =>
        {
            let rows = router_areas(provider_areas(layout.body)[1]);
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
    let value = app.field(field);
    let cursor = if app.focus == Target::Field(field) {
        app.cursor
    } else {
        value.chars().count()
    };
    let (line, col) = line_col(value, cursor);
    let _ = line;
    col.saturating_sub(width.saturating_sub(1))
}

fn line_col(value: &str, cursor: usize) -> (usize, usize) {
    let mut line = 0usize;
    let mut col = 0usize;
    for (index, ch) in value.chars().enumerate() {
        if index == cursor {
            break;
        }
        if ch == '\n' {
            line += 1;
            col = 0;
        } else {
            col += 1;
        }
    }
    (line, col)
}

pub fn cursor_at(app: &App, field: FieldId, x: u16) -> usize {
    let Some(area) = field_rect(app, field) else {
        return app.field(field).chars().count();
    };
    let offset = x.saturating_sub(area.x.saturating_add(1)) as usize;
    let value = app.field(field);
    if field != FieldId::Composer {
        return (viewport(app, field, area) + offset).min(value.chars().count());
    }
    let width = area.width.saturating_sub(2) as usize;
    let (cursor_line, _) = line_col(value, app.cursor);
    let start = value
        .split('\n')
        .take(cursor_line)
        .map(|line| line.chars().count() + 1)
        .sum::<usize>();
    (start + offset.min(width)).min(value.chars().count())
}

fn draw_field(frame: &mut Frame, app: &App, field: FieldId, label: &str, area: Rect) {
    if area.width < 3 || area.height < 3 {
        return;
    }
    let picker = is_picker_field(field);
    let value = if picker {
        app.field_display(field)
    } else {
        app.field(field).to_string()
    };
    let secret = matches!(
        field,
        FieldId::RouterKey
            | FieldId::FirecrawlKey
            | FieldId::HunterKey
            | FieldId::SociaVaultKey
            | FieldId::NewsApiKey
            | FieldId::CourtListenerKey
    );
    let display = if secret {
        "•".repeat(value.chars().count())
    } else {
        value.replace('\n', "⏎")
    };
    let scroll = if picker {
        0
    } else {
        viewport(app, field, area)
    };
    let width = area.width.saturating_sub(2) as usize;
    let visible: String = display.chars().skip(scroll).take(width).collect();
    let focused = app.focus == Target::Field(field);
    let block = if focused {
        panel(label).border_style(theme::selected())
    } else {
        panel(label)
    };
    frame.render_widget(
        Paragraph::new(if display.is_empty() {
            if picker {
                "Choose".into()
            } else if focused {
                visible
            } else {
                "Click to edit".into()
            }
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
    if focused && !picker && area.width > 2 && area.height > 2 {
        let cursor_x = area.x + 1 + (app.cursor.saturating_sub(scroll) as u16).min(area.width - 2);
        frame.set_cursor_position((cursor_x, area.y + 1));
    }
}

fn draw_button(frame: &mut Frame, app: &App, button: ButtonId, label: &str, area: Rect) {
    if area.width < 3 || area.height < 3 {
        return;
    }
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
                panel("").border_style(theme::selected())
            } else {
                panel("")
            }),
        area,
    );
}

pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    frame.render_widget(Paragraph::new("").style(theme::text()), area);
    let layout = chrome(area, app);
    if app.module.is_some() {
        draw_header(frame, app, &layout);
    }
    match app.module {
        None => draw_home(frame, app, layout.body),
        Some(ModuleId::Recon) => draw_recon(frame, app, layout.body),
        Some(ModuleId::Brain) => draw_brain(frame, app, layout.body),
        Some(ModuleId::Osint) => draw_osint(frame, app, layout.body),
        Some(ModuleId::Providers) => draw_providers(frame, app, layout.body),
        Some(ModuleId::System) => draw_system(frame, app, layout.body),
    }
    if composer_height(app) > 0 {
        let (field, send) = composer_parts(layout.composer);
        draw_field(frame, app, FieldId::Composer, " Ask Recon ", field);
        draw_button(frame, app, ButtonId::Send, "Send", send);
    }
    frame.render_widget(footer_line(app), layout.footer);
    if app.overlay != Overlay::None {
        draw_overlay(frame, app);
    }
}

fn draw_header(frame: &mut Frame, app: &App, layout: &Chrome) {
    let module = app.module.unwrap_or(ModuleId::Recon);
    let detail = match module {
        ModuleId::Recon if !app.recon_chat => "investigations".into(),
        ModuleId::Recon => {
            let title = app
                .threads
                .iter()
                .find(|thread| Some(&thread.id) == app.selected_thread.as_ref())
                .map(|thread| thread.title.as_str())
                .unwrap_or("New investigation");
            format!("{title} · {}", app.recon_stage)
        }
        ModuleId::Brain => "memories and insight provenance".into(),
        ModuleId::Osint => "public lookup tools".into(),
        ModuleId::Providers => "accounts and model defaults".into(),
        ModuleId::System => format!("hardware, paths, event log · {} errors", app.error_count()),
    };
    let room = layout.header.width.saturating_sub(8) as usize;
    let left = fit(&format!(" {} · {detail}", module.title()), room);
    let style = if module == ModuleId::Recon || module == ModuleId::Brain {
        theme::accent()
    } else {
        theme::dim()
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(format!("{left:<width$}", width = room), style),
            Span::styled(
                " Home ",
                if app.focus == Target::Home {
                    theme::selected()
                } else {
                    theme::accent()
                },
            ),
        ]))
        .style(theme::text()),
        layout.header,
    );
}

fn footer_line(app: &App) -> Paragraph<'static> {
    let keys = if let Overlay::Choice(_) = app.overlay {
        "↑↓ choose · Enter select · Esc close · Ctrl+U/Ctrl+D page"
    } else if app.overlay != Overlay::None {
        "Esc close · Ctrl+U/Ctrl+D scroll · click outside closes"
    } else {
        match (app.module, app.focus) {
            (None, _) => "↑↓ select · Enter open · 1–5 apps · ? help · Ctrl+C quit",
            (Some(ModuleId::Recon), Target::Field(FieldId::Composer)) => {
                "Enter send · Shift+Enter newline · Tab transcript · Esc investigations"
            }
            (
                Some(ModuleId::Recon),
                Target::Transcript | Target::ChatHeader(_) | Target::ChatBody(_),
            ) => "↑↓ select · ←→ fold · Enter toggle · Tab prompt · Esc investigations",
            (Some(ModuleId::Recon), _) if !app.recon_chat => {
                "↑↓ investigations · Enter open · Esc home · Ctrl+N new"
            }
            (Some(ModuleId::Recon), _) => {
                "Tab next · ↑↓ move · Enter activate · Esc investigations · Ctrl+N new"
            }
            (Some(ModuleId::System), _) => {
                "Ctrl+U/D log · Enter activate · Esc home · ? help · Ctrl+C quit"
            }
            _ => "Tab next control · ↑↓ move · Ctrl+U/D scroll · Enter activate · Esc home",
        }
    };
    let status = if app.status.is_empty() || app.status == "ready" {
        String::new()
    } else {
        format!("{} · ", clip_chars(&app.status, 42))
    };
    let style = if app.status.contains("fail") || app.status.contains("error") {
        theme::error()
    } else {
        theme::dim()
    };
    Paragraph::new(Line::from(vec![
        Span::styled(status, style),
        Span::styled(keys.to_string(), theme::dim()),
    ]))
}

fn app_label(module: ModuleId, errors: usize) -> String {
    let detail = if module == ModuleId::System && errors > 0 {
        format!("{} · {errors} errors", module.blurb())
    } else {
        module.blurb().to_string()
    };
    format!("{:<10}{detail}", module.title())
}

fn draw_home(frame: &mut Frame, app: &App, area: Rect) {
    let errors = app.error_count();
    let rows = [
        (
            None,
            "Launch an application. Only Recon accepts chat.".to_string(),
        ),
        (None, String::new()),
        (None, "Applications".to_string()),
        (Some(0), app_label(ModuleId::Recon, errors)),
        (Some(1), app_label(ModuleId::Brain, errors)),
        (None, String::new()),
        (None, "System".to_string()),
        (Some(2), app_label(ModuleId::Osint, errors)),
        (Some(3), app_label(ModuleId::Providers, errors)),
        (Some(4), app_label(ModuleId::System, errors)),
    ];
    let mut lines = Vec::new();
    for (index, (target, label)) in rows.into_iter().enumerate() {
        let selected = target.is_some_and(|slot| slot == app.launcher_sel);
        let style = if selected {
            theme::selected()
        } else if target.is_some() {
            theme::text()
        } else if index == 2 || index == 6 {
            theme::accent()
        } else {
            theme::dim()
        };
        let prefix = if selected { "▸ " } else { "  " };
        lines.push(Line::from(Span::styled(format!("{prefix}{label}"), style)));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "System apps change how Argos gathers and stores intelligence. They do not chat.",
        theme::dim(),
    )));
    frame.render_widget(
        Paragraph::new(lines)
            .style(theme::text())
            .block(panel(" Home "))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn draw_recon(frame: &mut Frame, app: &App, area: Rect) {
    if app.recon_chat {
        draw_recon_chat(frame, app, area);
    } else {
        draw_recon_dashboard(frame, app, area);
    }
}

fn draw_recon_dashboard(frame: &mut Frame, app: &App, area: Rect) {
    let (search, list, actions) = dashboard_areas(area);
    draw_field(frame, app, FieldId::ReconSearch, " Find ", search);
    let room = list.height.saturating_sub(2) as usize;
    let width = list.width.saturating_sub(2) as usize;
    let items = app
        .threads
        .iter()
        .enumerate()
        .skip(app.scrolls.threads as usize)
        .take(room)
        .map(|(index, thread)| {
            let running = app.running_thread(&thread.id);
            let mark = if running {
                "·"
            } else if index == app.thread_sel {
                "▸"
            } else {
                " "
            };
            let state = app
                .thread_states
                .get(&thread.id)
                .map(String::as_str)
                .filter(|state| !state.is_empty())
                .unwrap_or("new");
            let when: String = thread.updated_at.chars().take(10).collect();
            let suffix = format!(" · {state} · {when}");
            let title_room = width.saturating_sub(suffix.chars().count() + 2).max(8);
            ListItem::new(format!("{mark} {}{suffix}", fit(&thread.title, title_room))).style(
                if index == app.thread_sel {
                    theme::selected()
                } else {
                    theme::text()
                },
            )
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        List::new(items).block(panel(" Recent investigations ")),
        list,
    );
    let thread_buttons = button_areas(actions, 2);
    draw_button(frame, app, ButtonId::NewThread, "New", thread_buttons[0]);
    draw_button(
        frame,
        app,
        ButtonId::DeleteThread,
        "Delete",
        thread_buttons[1],
    );
}

fn draw_recon_chat(frame: &mut Frame, app: &App, area: Rect) {
    let (transcript, run_actions) = chat_areas(area);
    draw_transcript(frame, app, transcript);
    let run_buttons = button_areas(run_actions, 3);
    draw_button(frame, app, ButtonId::CancelRun, "Cancel", run_buttons[0]);
    draw_button(frame, app, ButtonId::ResumeRun, "Resume", run_buttons[1]);
    draw_button(
        frame,
        app,
        ButtonId::RetryInsights,
        "Insights",
        run_buttons[2],
    );
}

fn draw_transcript(frame: &mut Frame, app: &App, area: Rect) {
    let title = if app
        .selected_thread
        .as_ref()
        .is_some_and(|id| app.running_thread(id))
    {
        format!(" Recon · {} ", app.recon_stage)
    } else {
        " Recon ".into()
    };
    frame.render_widget(Paragraph::new("").block(panel(&title)), area);
    let (inner, scroll, rows) = chat_view(app);
    if rows.is_empty() {
        frame.render_widget(
            Paragraph::new("Ask a question below. Recon plans public lookups, then Synthesis answers from the evidence. A ◉ brain mark means that answer was written with saved memory.")
                .style(theme::dim())
                .wrap(Wrap { trim: true }),
            inner,
        );
        return;
    }
    for (offset, (_index, row)) in rows.iter().enumerate().skip(scroll as usize).enumerate() {
        if offset as u16 >= inner.height {
            break;
        }
        let rect = Rect {
            x: inner.x,
            y: inner.y + offset as u16,
            width: inner.width,
            height: 1,
        };
        let selected = row.block == app.chat_sel
            && matches!(
                app.focus,
                Target::Transcript | Target::ChatHeader(_) | Target::ChatBody(_)
            );
        let line = if selected && row.header && row.collapsible {
            let text: String = row.pieces.iter().map(|piece| piece.text.as_str()).collect();
            Line::from(Span::styled(text, theme::selected()))
        } else {
            paint_pieces(
                &row.pieces,
                row.face,
                inner.width as usize,
                selected && row.header,
            )
        };
        frame.render_widget(Paragraph::new(line).style(theme::text()), rect);
    }
}

fn paint_pieces(pieces: &[Piece], face: RowFace, width: usize, emphasize: bool) -> Line<'static> {
    let mut spans = Vec::new();
    let mut used = 0usize;
    for piece in pieces {
        let mut style = tone_style(piece.tone);
        if emphasize {
            style = style.add_modifier(Modifier::BOLD);
        }
        style = face_background(style, face);
        used += piece.text.chars().count();
        spans.push(Span::styled(piece.text.clone(), style));
    }
    if face != RowFace::Plain {
        let pad = width.saturating_sub(used);
        if pad > 0 {
            spans.push(Span::styled(
                " ".repeat(pad),
                face_background(Style::default(), face),
            ));
        }
    }
    if spans.is_empty() {
        spans.push(Span::styled(String::new(), theme::text()));
    }
    Line::from(spans)
}

fn face_background(style: Style, face: RowFace) -> Style {
    match face {
        RowFace::User => style.bg(theme::USER_BAND),
        RowFace::Code => style.bg(theme::CODE_BG),
        RowFace::Plain => style,
    }
}

fn tone_style(tone: Tone) -> Style {
    match tone {
        Tone::Body => theme::text(),
        Tone::Dim => theme::dim(),
        Tone::Accent | Tone::Heading => theme::accent().add_modifier(Modifier::BOLD),
        Tone::Bold => theme::text().add_modifier(Modifier::BOLD),
        Tone::Italic => theme::text().add_modifier(Modifier::ITALIC),
        Tone::Strike => theme::dim().add_modifier(Modifier::CROSSED_OUT),
        Tone::Code => theme::accent().add_modifier(Modifier::BOLD),
        Tone::Link => theme::accent().add_modifier(Modifier::UNDERLINED),
        Tone::Warn => theme::warn(),
        Tone::Error => theme::error(),
    }
}

fn draw_osint(frame: &mut Frame, app: &App, area: Rect) {
    let slot = api_key_slot(app);
    let layout = osint_areas(area, slot.is_some());
    let search = layout.search;
    let list = layout.list;
    let detail = layout.detail;
    let input = layout.input;
    let actions = layout.actions;
    draw_field(frame, app, FieldId::OsintSearch, " Tools ", search);
    let tools = visible_tools(app);
    let items = tools
        .into_iter()
        .skip(app.scrolls.tools as usize)
        .take(list.height.saturating_sub(2) as usize)
        .map(|(index, tool)| {
            let enabled = app.tool_enabled.get(index).copied().unwrap_or(true);
            ListItem::new(format!(
                "{} {} · {}{}",
                if enabled { "●" } else { "○" },
                tool.category,
                tool.name,
                if app.tool_needs_key(tool.id) { " · needs key" } else { "" }
            ))
            .style(if index == app.tool_sel {
                theme::selected()
            } else {
                theme::text()
            })
        })
        .collect::<Vec<_>>();
    frame.render_widget(List::new(items).block(panel(" Lookup tools ")), list);
    let desc = if let Some(tool) = osint::registry().get(app.tool_sel) {
        let result = app
            .osint_result
            .as_ref()
            .filter(|(_, result)| result.tool_id == tool.id)
            .map(|(id, result)| {
                let body = if app.osint_raw {
                    clip_chars(&result.raw, 12_000)
                } else if result.observations.is_null() {
                    String::new()
                } else {
                    serde_json::to_string_pretty(&result.observations).unwrap_or_default()
                };
                let error = result
                    .error
                    .as_deref()
                    .filter(|text| !text.is_empty())
                    .map(|text| format!("\nError: {text}"))
                    .unwrap_or_default();
                format!(
                    "\n\nManual run {id}: {} · {}\nSource: {}{error}\n{body}",
                    result.status, result.retrieved_at, result.source_url
                )
            })
            .unwrap_or_default();
        format!(
            "{}\n{} · {} · {}\n\nInputs: {}\nExample: {}\n\n{}\n\nPolicy: {}\nTimeout: {}s · Cache: {}s\nDocs: {}{}",
            tool.name,
            tool.id,
            tool.category,
            match (
                app.tool_enabled.get(app.tool_sel).copied().unwrap_or(true),
                app.tool_needs_key(tool.id),
            ) {
                (true, false) => "enabled",
                (true, true) => "enabled · needs key",
                (false, _) => "disabled",
            },
            tool.inputs.join(", "),
            tool.example_input(),
            tool.description,
            tool.restrictions,
            tool.timeout_seconds,
            tool.cache_seconds,
            tool.documentation,
            result
        )
    } else {
        "Select a tool".into()
    };
    frame.render_widget(
        Paragraph::new(desc)
            .style(theme::text())
            .block(panel(" Tool "))
            .scroll((app.scrolls.detail, 0))
            .wrap(Wrap { trim: true }),
        detail,
    );
    if let Some(slot) = slot {
        let parts = split_horizontal(layout.key, [Constraint::Min(8), Constraint::Length(16)]);
        draw_field(frame, app, slot.field, " API key ", parts[0]);
        draw_button(frame, app, slot.button, "Save key", parts[1]);
    }
    draw_field(frame, app, FieldId::OsintInput, " Input JSON ", input);
    let buttons = button_areas(actions, 8);
    let labels = [
        (ButtonId::OsintRun, "Run"),
        (ButtonId::OsintCancel, "Cancel"),
        (ButtonId::OsintToggle, "Enable"),
        (ButtonId::OsintRaw, "Raw"),
        (ButtonId::OsintAttach, "Attach"),
        (ButtonId::OsintStartRecon, "Recon"),
        (ButtonId::OsintPrev, "Prev"),
        (ButtonId::OsintNext, "Next"),
    ];
    for (index, (button, label)) in labels.into_iter().enumerate() {
        draw_button(frame, app, button, label, buttons[index]);
    }
}

fn draw_brain(frame: &mut Frame, app: &App, area: Rect) {
    let rows = brain_areas(area);
    frame.render_widget(
        Paragraph::new(" Save, recall, pin, or delete insights. Categories: fact, identity, preference, contact, project, goal, task").style(theme::accent()),
        rows[0],
    );
    draw_field(frame, app, FieldId::BrainApp, " Source app ", rows[1]);
    draw_field(
        frame,
        app,
        FieldId::BrainConversation,
        " Conversation ",
        rows[2],
    );
    draw_field(frame, app, FieldId::BrainInsight, " Insight ", rows[3]);
    draw_field(frame, app, FieldId::BrainQuery, " Recall ", rows[4]);
    let save_row = button_areas(rows[5], 2);
    draw_button(frame, app, ButtonId::Add, "Save", save_row[0]);
    draw_button(frame, app, ButtonId::Recall, "Recall", save_row[1]);
    let edit_row = button_areas(rows[6], 3);
    draw_button(frame, app, ButtonId::Pin, "Pin", edit_row[0]);
    draw_button(frame, app, ButtonId::Delete, "Delete", edit_row[1]);
    draw_button(frame, app, ButtonId::OpenSource, "Source", edit_row[2]);
    let room = rows[7].height.saturating_sub(2) as usize / 2;
    let items = app
        .memories
        .iter()
        .enumerate()
        .skip(app.scrolls.memories as usize)
        .take(room)
        .map(|(index, memory)| {
            let title = format!(
                "{} [{}] {}",
                if memory.pinned { "◆" } else { "·" },
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
    frame.render_widget(List::new(items).block(panel(" Memories ")), rows[7]);
    let recalled = if let Some(insight) = &app.selected_insight {
        let origins = insight
            .sources
            .iter()
            .map(|source| source.thread_id.as_deref().unwrap_or("deleted origin"))
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "{} · {} → {} · {} · {:.0}%\n{} evidence links · threads: {} · related: {}",
            insight.entity,
            insight.predicate,
            insight.object_value,
            insight.classification,
            insight.confidence * 100.0,
            insight.sources.len(),
            origins,
            insight.related.len()
        )
    } else if app.hits.is_empty() {
        "Recall searches saved memory. Select an investigation insight to read its anchors.".into()
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
            .block(panel(" Recall "))
            .scroll((app.scrolls.recall, 0))
            .wrap(Wrap { trim: true }),
        rows[8],
    );
}

fn draw_providers(frame: &mut Frame, app: &App, area: Rect) {
    let rows = provider_areas(area);
    let tabs = button_areas(rows[0], ProviderPage::ALL.len());
    for (index, page) in ProviderPage::ALL.into_iter().enumerate() {
        let selected = app.provider_page == page || app.focus == Target::ProviderTab(page);
        frame.render_widget(
            Paragraph::new(page.title())
                .alignment(Alignment::Center)
                .style(if app.provider_page == page {
                    theme::selected()
                } else if selected {
                    theme::accent()
                } else {
                    theme::dim()
                })
                .block(panel("")),
            tabs[index],
        );
    }
    match app.provider_page {
        ProviderPage::Grok | ProviderPage::OpenAI => {
            let grok = app.provider_page == ProviderPage::Grok;
            let content = auth_areas(rows[1]);
            let intro = if grok {
                "Grok subscription sign-in. The account stays with Grok."
            } else {
                "ChatGPT subscription sign-in through Codex CLI."
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
                draw_button(frame, app, ButtonId::GrokSignIn, "Sign in", buttons[0]);
                draw_button(frame, app, ButtonId::GrokCheck, "Check", buttons[1]);
            } else {
                draw_button(frame, app, ButtonId::OpenAISignIn, "Sign in", buttons[0]);
                draw_button(frame, app, ButtonId::OpenAICheck, "Check", buttons[1]);
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
                    .block(panel(" Sign-in "))
                    .scroll((app.scrolls.detail, 0))
                    .wrap(Wrap { trim: true }),
                content[3],
            );
        }
        ProviderPage::OpenRouter => {
            let router = router_areas(rows[1]);
            frame.render_widget(
                Paragraph::new(app.router_status.as_str())
                    .style(theme::accent())
                    .block(panel(" OpenRouter "))
                    .wrap(Wrap { trim: true }),
                router[0],
            );
            draw_field(frame, app, FieldId::RouterKey, " API key ", router[1]);
            let buttons = button_areas(router[2], 2);
            draw_button(frame, app, ButtonId::RouterSave, "Save", buttons[0]);
            draw_button(frame, app, ButtonId::RouterVerify, "Verify", buttons[1]);
            draw_button(
                frame,
                app,
                ButtonId::RouterAdvanced,
                if app.router_advanced {
                    "Hide endpoint"
                } else {
                    "Show endpoint"
                },
                router[3],
            );
            if app.router_advanced {
                draw_field(
                    frame,
                    app,
                    FieldId::RouterEndpoint,
                    " HTTPS endpoint ",
                    router[4],
                );
            }
            frame.render_widget(
                Paragraph::new("Verify tests the form without saving it. Save stores this account only. Model routing stays on Defaults.")
                    .style(theme::dim())
                    .wrap(Wrap { trim: true }),
                router[5],
            );
        }
        ProviderPage::Defaults => {
            let models = model_areas(rows[1]);
            for (role, area) in DefaultsRole::ALL
                .into_iter()
                .zip(button_areas(models[0], DefaultsRole::ALL.len()))
            {
                let label = if role == app.defaults_role {
                    format!("● {}", role.label())
                } else {
                    role.label().to_string()
                };
                draw_button(frame, app, ButtonId::DefaultRole(role), &label, area);
            }
            let role = app.defaults_role;
            let provider = role.provider_field();
            let model = role.model_field();
            let save = role.save_button();
            draw_field(frame, app, provider, " Provider ", models[1]);
            draw_field(frame, app, model, " Model ", models[2]);
            draw_button(frame, app, save, "Save default", models[3]);
            draw_button(
                frame,
                app,
                ButtonId::RefreshModels,
                "Refresh models",
                models[4],
            );
            let provider = app.role_provider();
            let label = if provider.is_empty() {
                "this account"
            } else {
                match provider::normalize_kind(&provider).as_str() {
                    "grok" => "Grok",
                    "openai-chatgpt" => "OpenAI",
                    "openrouter" => "OpenRouter",
                    "local" => "Local",
                    _ => "this account",
                }
            };
            let transport = if role == DefaultsRole::ToolPicker && !app.picker_model.is_empty() {
                format!(
                    " Transport: {}.",
                    provider::picker_transport(&app.picker_model)
                )
            } else {
                String::new()
            };
            let note = if provider.is_empty() {
                "Choose a connected account. Open Provider and Model, then Save default.".into()
            } else if app.model_catalog.is_empty() || app.catalog_for != provider {
                format!("Open Model to list what {label} can call. Save default stores this role.")
            } else {
                format!(
                    "{} models this account can call. Open Model to choose, then Save default.",
                    app.model_catalog.len()
                )
            };
            let note = format!("{note}{transport}");
            frame.render_widget(
                Paragraph::new(note)
                    .style(theme::dim())
                    .scroll((app.scrolls.detail, 0))
                    .wrap(Wrap { trim: true }),
                models[5],
            );
        }
    }
}

fn log_rows(app: &App) -> Vec<Line<'static>> {
    let mut rows = Vec::new();
    for (index, entry) in app.log.iter().enumerate() {
        let style = if index == app.log_sel {
            theme::selected()
        } else {
            match entry.level.as_str() {
                "error" => theme::error(),
                "warn" => theme::warn(),
                _ => theme::dim(),
            }
        };
        let marker = if entry.detail.is_empty() {
            String::new()
        } else if app.log_open.contains(&entry.id) {
            "▾ ".into()
        } else {
            "▸ ".into()
        };
        rows.push(Line::from(Span::styled(
            format!("{marker}{} {} {}", entry.at, entry.level, entry.text),
            style,
        )));
        if app.log_open.contains(&entry.id) && !entry.detail.is_empty() {
            for line in entry.detail.lines() {
                rows.push(Line::from(Span::styled(format!("  {line}"), theme::text())));
            }
        }
    }
    rows
}

fn draw_system(frame: &mut Frame, app: &App, area: Rect) {
    let (hardware, actions, log) = system_areas(area);
    let body = format!(
        "{}\nBackend: {}\nLogical cores: {}\nConfig: {}\nDatabase: {}",
        app.hardware.one_line(),
        app.hardware.backend,
        app.hardware.logical_cores,
        argos_osint_core::paths::config_path().display(),
        argos_osint_core::paths::db_path().display()
    );
    frame.render_widget(
        Paragraph::new(body)
            .style(theme::text())
            .block(panel(" Host "))
            .wrap(Wrap { trim: true }),
        hardware,
    );
    let buttons = button_areas(actions, 2);
    draw_button(
        frame,
        app,
        ButtonId::RefreshHardware,
        "Refresh hardware",
        buttons[0],
    );
    draw_button(frame, app, ButtonId::ClearLog, "Clear log", buttons[1]);
    let lines = if app.log.is_empty() {
        vec![Line::from(Span::styled(
            "No events yet. Failures from Recon, Brain, OSINT, and Providers are recorded here.",
            theme::dim(),
        ))]
    } else {
        log_rows(app)
    };
    frame.render_widget(
        Paragraph::new(lines)
            .block(panel(" Event log "))
            .scroll((app.scrolls.log, 0))
            .wrap(Wrap { trim: false }),
        log,
    );
}

fn popup_text(app: &App) -> String {
    match &app.overlay {
        Overlay::Help => help_text(app).to_string(),
        Overlay::Block { title, body } => format!("{title}\n\n{body}"),
        Overlay::Memories { message_id } => memory_popup(app, message_id),
        Overlay::Choice(_) => {
            let mut lines = Vec::new();
            if !app.choice_note.is_empty() {
                lines.push(app.choice_note.clone());
            }
            lines.extend(app.choice_items.iter().map(|item| item.label.clone()));
            lines.join("\n")
        }
        Overlay::None => String::new(),
    }
}

fn memory_popup(app: &App, message_id: &str) -> String {
    let Some(memories) = app.answer_memories.get(message_id) else {
        return "No memory was supplied to this synthesis answer.".into();
    };
    let mut lines = vec![format!(
        "{} saved memor{} informed this synthesis answer.",
        memories.len(),
        if memories.len() == 1 { "y" } else { "ies" }
    )];
    for memory in memories {
        lines.push(String::new());
        lines.push(format!(
            "{} [{}] {}",
            if memory.pinned { "◆" } else { "·" },
            memory.category,
            memory.text
        ));
        lines.push(format!(
            "Source: {} / {}",
            memory.source.app,
            if memory.source.conversation_id.is_empty() {
                "unknown"
            } else {
                memory.source.conversation_id.as_str()
            }
        ));
        if let Some(created) = memory.created_at.chars().next() {
            if created != '\0' && !memory.created_at.is_empty() {
                lines.push(format!("Recorded: {}", memory.created_at));
            }
        }
        if let Some(summary) = app.insight_summary(&memory.id) {
            lines.push(summary);
        }
    }
    lines.join("\n")
}

fn help_text(app: &App) -> &'static str {
    match app.module {
        None => "Home\n\n↑↓ or j/k select an application\nEnter opens it\n1 Recon · 2 Brain · 3 OSINT · 4 Providers · 5 System\n? help · Esc closes this card\nCtrl+C quits when nothing is running · Ctrl+Q quits from anywhere",
        Some(ModuleId::Recon) if !app.recon_chat => "Recon investigations\n\nThe list is the most recent investigations\n↑↓ move · Enter opens the transcript\nNew starts an investigation · Delete removes the selected one\nType to search titles\nEsc returns home · Ctrl+N new investigation",
        Some(ModuleId::Recon) => "Recon chat\n\nEnter sends · Shift+Enter inserts a line\nTab moves between the transcript and the prompt\n↑↓ select a message, recon log, or tool\n←→ or h/l fold the selected recon log or tool\nEnter toggles that fold · f opens the full text\n◉ brain opens the memories Synthesis used\nCtrl+U/Ctrl+D scroll · the wheel scrolls the pane under the pointer\nEsc returns to investigations · Ctrl+C cancels a running turn\nCtrl+N new thread · Alt+←/→ recent threads",
        Some(ModuleId::System) => "System\n\nRefresh hardware re-reads the host profile\nThe event log keeps errors, run stages, and tool results\n↑↓ select a line · Enter folds a tool result\nCtrl+U/Ctrl+D and the wheel scroll the log\nEsc returns home",
        Some(ModuleId::Providers) => "Providers\n\nEach account tab stores that provider only\nDefaults sets Recon and Synthesis separately\nProvider and Model open the accounts and models that connection can use\n↑↓ choose · Enter selects · Esc closes the list\nEsc returns home · ? opens this card",
        _ => "Controls\n\nTab moves between fields and buttons\nEnter activates the focused control\n↑↓ move through lists\nCtrl+U/Ctrl+D and the wheel scroll the pane under the pointer\nTyping works only in a focused field\nEsc returns home · ? opens this card",
    }
}

fn draw_overlay(frame: &mut Frame, app: &App) {
    if let Overlay::Choice(kind) = app.overlay {
        draw_choice(frame, app, kind);
        return;
    }
    let area = popup_area(frame.area());
    let title = match &app.overlay {
        Overlay::Help => " Shortcuts ",
        Overlay::Memories { .. } => " Memory ",
        Overlay::Block { .. } => " Detail ",
        Overlay::Choice(_) | Overlay::None => " ",
    };
    frame.render_widget(
        Paragraph::new(popup_text(app))
            .style(theme::text())
            .block(panel(title))
            .scroll((app.scrolls.popup, 0))
            .wrap(Wrap { trim: false }),
        area,
    );
    let close = Rect {
        x: area.x + area.width.saturating_sub(8),
        y: area.y,
        width: 8.min(area.width),
        height: 1,
    };
    frame.render_widget(Paragraph::new(" close ").style(theme::accent()), close);
}

fn draw_choice(frame: &mut Frame, app: &App, kind: ChoiceKind) {
    let area = popup_area(frame.area());
    let title = match kind {
        ChoiceKind::Provider => format!(" {} provider ", app.defaults_role.label()),
        ChoiceKind::Model => format!(" {} model ", app.defaults_role.label()),
    };
    frame.render_widget(Paragraph::new("").block(panel(&title)), area);
    let inner = inset(area);
    let mut y = inner.y;
    let mut height = inner.height;
    if !app.choice_note.is_empty() && height > 0 {
        frame.render_widget(
            Paragraph::new(fit(
                &app.choice_note.replace('\n', " "),
                inner.width as usize,
            ))
            .style(theme::dim()),
            Rect {
                x: inner.x,
                y,
                width: inner.width,
                height: 1,
            },
        );
        y = y.saturating_add(1);
        height = height.saturating_sub(1);
    }
    if app.choice_items.is_empty() && height > 0 {
        let empty = if matches!(kind, ChoiceKind::Model) {
            "No models for this account yet."
        } else {
            "No connected account yet. Local is always listed."
        };
        frame.render_widget(
            Paragraph::new(empty).style(theme::dim()),
            Rect {
                x: inner.x,
                y,
                width: inner.width,
                height: 1,
            },
        );
    }
    let start = app.scrolls.popup as usize;
    for (index, item) in app
        .choice_items
        .iter()
        .enumerate()
        .skip(start)
        .take(height as usize)
    {
        let selected = index == app.choice_sel;
        let mark = if selected { "▸ " } else { "  " };
        let text = fit(&format!("{mark}{}", item.label), inner.width as usize);
        frame.render_widget(
            Paragraph::new(text).style(if selected {
                theme::selected()
            } else {
                theme::text()
            }),
            Rect {
                x: inner.x,
                y: y + (index - start) as u16,
                width: inner.width,
                height: 1,
            },
        );
    }
    let close = Rect {
        x: area.x + area.width.saturating_sub(8),
        y: area.y,
        width: 8.min(area.width),
        height: 1,
    };
    frame.render_widget(Paragraph::new(" close ").style(theme::accent()), close);
}

#[cfg(test)]
mod tests {
    use super::*;
    use argos_osint_core::osint::ToolResult;
    use argos_osint_core::recon::{Binding, Call, Directive, PickRecord, PlanCall};

    #[test]
    fn decision_row_shows_directives_picker_order_bindings_and_fallbacks() {
        let directive = |id: &str, goal: &str, targets: &[&str]| Directive {
            id: id.into(),
            goal: goal.into(),
            entities: vec!["Jane Roe".into()],
            targets: targets.iter().map(|kind| kind.to_string()).collect(),
            ..Default::default()
        };
        let plan = Plan {
            planning_mode: "tool_picker".into(),
            directives: vec![
                directive("d1", "Establish the subject's identity and public roles", &["person_name", "org_name", "url"]),
                directive("d2", "Find the subject's official online accounts and websites", &["handle", "domain", "url"]),
                directive("d3", "Find organizations affiliated with the subject and their contact domains", &["org_name", "domain", "email"]),
            ],
            directives_mode: "directives_fallback".into(),
            picker_transport: "decisions".into(),
            picker_model: "typesafe/jev-1.13".into(),
            picks: vec![PickRecord {
                position: 1,
                tool_id: "firecrawl_search".into(),
                transport: "decisions".into(),
                outcome: "accepted".into(),
                confidence: Some(0.8731),
                ..Default::default()
            }],
            calls: vec![
                PlanCall {
                    step_id: "s1".into(),
                    tool_id: "firecrawl_search".into(),
                    reason: "d1, d2".into(),
                    status: "completed".into(),
                    call_id: "call-s1".into(),
                    filled: vec!["query=Jane Roe (d1 entity)".into()],
                    confidence: Some(0.8731),
                    ..Default::default()
                },
                PlanCall {
                    step_id: "s2".into(),
                    tool_id: "sociavault_profile".into(),
                    reason: "d2".into(),
                    depends_on: vec!["s1".into()],
                    filled: vec!["handle=janeroe (handle from call-s1)".into()],
                    ..Default::default()
                },
            ],
            bindings: vec![Binding {
                kind: "handle".into(),
                value: "janeroe".into(),
                evidence_id: "call-s1".into(),
                step_id: "s1".into(),
                qualifier: "github".into(),
                inferred: false,
                unverified: false,
                source_tool: String::new(),
            }, Binding {
                kind: "handle".into(),
                value: "janeroe".into(),
                evidence_id: "call-s1".into(),
                step_id: "s1".into(),
                qualifier: "facebook".into(),
                inferred: true,
                unverified: false,
                source_tool: String::new(),
            }, Binding {
                kind: "handle".into(),
                value: "janeroe".into(),
                evidence_id: "d2".into(),
                step_id: String::new(),
                qualifier: "twitter".into(),
                inferred: false,
                unverified: true,
                source_tool: String::new(),
            }],
            binding_notes: vec!["s1 firecrawl_search: rules found 1; Recon model added 0".into()],
            fallback_requests: vec!["hunter_email_finder failed. Recon chose firecrawl_scrape as s3.".into()],
            ..Default::default()
        };
        let run = recon::Run {
            id: "run-1".into(),
            thread_id: "t".into(),
            turn_id: "turn".into(),
            state: "completed".into(),
            stage: String::new(),
            recon_model: "grok / grok-4.6".into(),
            synthesis_model: "grok / grok-4.6".into(),
            tool_picker_model: "openrouter / typesafe/jev-1.13".into(),
            max_rounds: 1,
            max_calls: 8,
            turn_seconds: 120,
            plan_json: Some(serde_json::to_string(&plan).unwrap()),
            error: None,
            created_at: String::new(),
            updated_at: String::new(),
        };
        let calls = vec![Call {
            id: "call-s1".into(),
            tool_id: "firecrawl_search".into(),
            run_id: Some("run-1".into()),
            thread_id: None,
            turn_id: None,
            origin: "recon".into(),
            inputs: serde_json::json!({"query": "Jane Roe"}),
            status: "completed".into(),
            attempts: 1,
            result: Some(ToolResult {
                tool_id: "firecrawl_search".into(),
                inputs: serde_json::json!({"query": "Jane Roe"}),
                status: "completed".into(),
                source_url: "https://example.test/jane".into(),
                retrieved_at: "2026-10-02T00:00:00Z".into(),
                observations: serde_json::json!({"results": [{"title": "Jane Roe role"}, {"title": "Jane Roe site"}]}),
                raw: String::new(),
                error: None,
                cached: false,
                truncated: false,
                credits_charged: 0,
                credits_reported: None,
            }),
            started_at: String::new(),
            completed_at: Some("2026-10-02T00:00:00Z".into()),
        }];
        let block = plan_block(&run, &calls, true);
        assert!(block.title.starts_with("Recon log · tool picker (decisions)"), "{}", block.title);
        for needle in [
            "Directives (fallback set):",
            "d1: Establish the subject's identity and public roles · entities Jane Roe · targets person_name, org_name, url",
            "d2: Find the subject's official online accounts and websites · entities Jane Roe · targets handle, domain, url",
            "d3: Find organizations affiliated with the subject and their contact domains",
            "Tool picker: decisions · openrouter / typesafe/jev-1.13",
            "s1. firecrawl_search — d1, d2 · completed",
            "result completed · live · 2 results",
            "input query=Jane Roe (d1 entity)",
            "s2. sociavault_profile — d2 · after s1",
            "input handle=janeroe (handle from call-s1)",
            "found handle janeroe (github) · evidence call-s1",
            "found handle janeroe (facebook) · evidence call-s1 · inferred",
            "Binding extraction:",
            "From the question: handle janeroe (twitter) · named in d2, unverified",
            "s1 firecrawl_search: rules found 1; Recon model added 0",
            "Fallback requests:",
        ] {
            assert!(block.body.contains(needle), "missing {needle:?} in\n{}", block.body);
        }
        assert!(!block.body.contains("0.87"), "probabilities stay out of the row");
        assert!(!block.body.contains("Jane Roe role"), "raw observations stay out of the decision row");
        let logged = tool_result_log(&calls[0]).unwrap();
        assert_eq!(logged.summary, "Firecrawl search completed · live · 2 results");
        assert!(logged.detail.contains("Jane Roe role"));
        assert!(logged.detail.contains("https://example.test/jane"));
    }
}
